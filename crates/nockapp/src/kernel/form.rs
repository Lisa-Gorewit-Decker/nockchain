#![allow(dead_code)]
use std::any::Any;
use std::fs;
use std::future::Future;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use bincode::{config, Decode, Encode};
use blake3::{Hash, Hasher};
use byteorder::{LittleEndian, WriteBytesExt};
use chaff::stream::{jam_noun_to_writer, jam_pma_to_writer};
use nockvm::hamt::Hamt;
use nockvm::interpreter::{self, interpret, Error, Mote, NockCancelToken};
use nockvm::jets::cold::{Cold, Nounable};
use nockvm::jets::hot::{HotEntry, URBIT_HOT_STATE};
use nockvm::jets::nock::util::mook;
use nockvm::mem::NockStack;
use nockvm::mug::met3_usize;
use nockvm::noun::{Atom, Cell, DirectAtom, IndirectAtom, Noun, NounSpace, D, T};
use nockvm::pma::{Pma, PmaCopy, PmaCopyFrom, PmaDirectJamConfig, PmaDirectReader};
use nockvm::trace::{path_to_cord, write_serf_trace_safe};
use nockvm_macros::tas;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Duration;
use tracing::{debug, info, warn};

use crate::kernel::boot::TraceOpts;
use crate::metrics::NockAppMetrics;
use crate::nockapp::wire::{wire_to_noun, WireRepr};
use crate::noun::slab::NounSlab;
use crate::noun::slam;
use crate::save::{SaveableCheckpoint, StreamingCheckpointMeta, StreamingCheckpointRequest};
use crate::utils::{
    create_context, current_da, NOCK_STACK_SIZE, NOCK_STACK_SIZE_HUGE, NOCK_STACK_SIZE_LARGE,
    NOCK_STACK_SIZE_MEDIUM, NOCK_STACK_SIZE_SMALL, NOCK_STACK_SIZE_TINY,
};
use crate::{AtomExt, CrownError, IndirectAtomExt, NounExt, Result, ToBytesExt};

pub(crate) const STATE_AXIS: u64 = 6;
const LOAD_AXIS: u64 = 4;
const PEEK_AXIS: u64 = 22;
const POKE_AXIS: u64 = 23;

const SERF_FINISHED_INTERVAL: Duration = Duration::from_millis(100);
const SERF_THREAD_STACK_SIZE: usize = 256 * 1024 * 1024; // 8MB

pub struct LoadState {
    pub ker_hash: Hash,
    pub event_num: u64,
    pub kernel_state: NounSlab,
}

#[derive(Clone, Debug)]
pub struct PmaConfig {
    pub path_0: PathBuf,
    pub path_1: PathBuf,
    pub words: usize,
    pub open_existing: bool,
    pub gc_interval: Option<Duration>,
}

#[derive(Clone, Copy, Debug)]
enum PmaSlab {
    Slab0,
    Slab1,
}

impl PmaSlab {
    fn next(self) -> Self {
        match self {
            PmaSlab::Slab0 => PmaSlab::Slab1,
            PmaSlab::Slab1 => PmaSlab::Slab0,
        }
    }
}

#[derive(Clone, Debug)]
struct PmaSlabPaths {
    path_0: PathBuf,
    path_1: PathBuf,
}

impl PmaSlabPaths {
    fn path(&self, slab: PmaSlab) -> &PathBuf {
        match slab {
            PmaSlab::Slab0 => &self.path_0,
            PmaSlab::Slab1 => &self.path_1,
        }
    }
}

const PMA_PERSIST_MAGIC: u64 = u64::from_le_bytes(*b"PMAPERS1");
const PMA_PERSIST_VERSION: u32 = 2;

#[derive(Clone, Encode, Decode, Debug)]
struct PmaPersistMetadata {
    magic: u64,
    version: u32,
    #[bincode(with_serde)]
    ker_hash: Hash,
    event_num: u64,
    kernel_state_raw: u64,
    cold_offset: u32,
    pma_base: u64,
    #[bincode(with_serde)]
    checksum: Hash,
}

impl PmaPersistMetadata {
    fn new(
        ker_hash: Hash,
        event_num: u64,
        kernel_state_raw: u64,
        cold_offset: u32,
        pma_base: u64,
    ) -> Self {
        let checksum = Self::checksum(ker_hash, event_num, kernel_state_raw, cold_offset, pma_base);
        Self {
            magic: PMA_PERSIST_MAGIC,
            version: PMA_PERSIST_VERSION,
            ker_hash,
            event_num,
            kernel_state_raw,
            cold_offset,
            pma_base,
            checksum,
        }
    }

    fn checksum(
        ker_hash: Hash,
        event_num: u64,
        kernel_state_raw: u64,
        cold_offset: u32,
        pma_base: u64,
    ) -> Hash {
        let mut hasher = Hasher::new();
        hasher.update(ker_hash.as_bytes());
        hasher.update(&event_num.to_le_bytes());
        hasher.update(&kernel_state_raw.to_le_bytes());
        hasher.update(&cold_offset.to_le_bytes());
        hasher.update(&pma_base.to_le_bytes());
        hasher.finalize()
    }

    fn validate(&self) -> bool {
        if self.magic != PMA_PERSIST_MAGIC || self.version != PMA_PERSIST_VERSION {
            return false;
        }
        self.checksum
            == Self::checksum(
                self.ker_hash, self.event_num, self.kernel_state_raw, self.cold_offset,
                self.pma_base,
            )
    }

    fn load_from_path(path: &PathBuf) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        let (meta, _) =
            bincode::decode_from_slice::<Self, config::Configuration>(&bytes, config::standard())
                .ok()?;
        meta.validate().then_some(meta)
    }

    fn save_to_path(&self, path: &PathBuf) -> std::io::Result<()> {
        let bytes = bincode::encode_to_vec(self, config::standard())
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, bytes)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }
}

fn pma_meta_path(path: &PathBuf) -> PathBuf {
    path.with_extension("meta")
}

fn pma_meta_status(path: &PathBuf, ker_hash: Hash) -> Option<(u64, SystemTime)> {
    let meta_path = pma_meta_path(path);
    let meta = PmaPersistMetadata::load_from_path(&meta_path)?;
    if meta.ker_hash != ker_hash {
        return None;
    }
    let modified = std::fs::metadata(&meta_path)
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Some((meta.event_num, modified))
}

fn select_active_pma_slab(paths: &PmaSlabPaths, ker_hash: Hash) -> PmaSlab {
    let status_0 = pma_meta_status(&paths.path_0, ker_hash);
    let status_1 = pma_meta_status(&paths.path_1, ker_hash);
    match (status_0, status_1) {
        (Some((event_0, mod_0)), Some((event_1, mod_1))) => {
            if event_0 > event_1 {
                PmaSlab::Slab0
            } else if event_1 > event_0 {
                PmaSlab::Slab1
            } else if mod_1 > mod_0 {
                PmaSlab::Slab1
            } else {
                PmaSlab::Slab0
            }
        }
        (Some(_), None) => PmaSlab::Slab0,
        (None, Some(_)) => PmaSlab::Slab1,
        (None, None) => PmaSlab::Slab0,
    }
}

struct PmaGcState {
    paths: PmaSlabPaths,
    active: PmaSlab,
    interval: Duration,
    last_gc: Instant,
    words: usize,
}

impl PmaGcState {
    fn new(paths: PmaSlabPaths, active: PmaSlab, interval: Duration, words: usize) -> Self {
        Self {
            paths,
            active,
            interval,
            last_gc: Instant::now(),
            words,
        }
    }

    fn active_path(&self) -> &PathBuf {
        self.paths.path(self.active)
    }

    fn inactive_path(&self) -> &PathBuf {
        self.paths.path(self.active.next())
    }

    fn mark_gc_completed(&mut self) {
        self.active = self.active.next();
        self.last_gc = Instant::now();
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PmaCopySegment {
    pub elapsed: Duration,
    pub alloc_words: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PmaCopyDetail {
    pub warm: PmaCopySegment,
    pub test_jets: PmaCopySegment,
    pub hot: PmaCopySegment,
    pub cache: PmaCopySegment,
    pub cold: PmaCopySegment,
    pub arvo: PmaCopySegment,
}

#[derive(Clone, Copy, Debug)]
pub struct PmaTimingSample {
    pub event: Duration,
    pub pma_copy: Duration,
    pub detail: Option<PmaCopyDetail>,
}

#[derive(Default)]
pub(crate) struct PmaTiming {
    samples: Mutex<Vec<PmaTimingSample>>,
}

impl PmaTiming {
    pub(crate) fn record(
        &self,
        event: Duration,
        pma_copy: Duration,
        detail: Option<PmaCopyDetail>,
    ) {
        let mut samples = self.samples.lock().unwrap_or_else(|err| err.into_inner());
        samples.push(PmaTimingSample {
            event,
            pma_copy,
            detail,
        });
    }

    pub(crate) fn take_samples(&self) -> Vec<PmaTimingSample> {
        let mut samples = self.samples.lock().unwrap_or_else(|err| err.into_inner());
        std::mem::take(&mut *samples)
    }
}

// Actions to request of the serf thread
pub enum SerfAction<C> {
    // Make a CheckPoint
    Checkpoint {
        result: oneshot::Sender<C>,
    },
    // Make a streaming checkpoint directly from the PMA slab
    CheckpointStreaming {
        request: StreamingCheckpointRequest,
        result: oneshot::Sender<Result<StreamingCheckpointMeta>>,
    },
    Import {
        state: LoadState,
        result: oneshot::Sender<Result<()>>,
    },
    Export {
        result: oneshot::Sender<Result<LoadState>>,
    },
    // Get the state noun of the kernel as a slab
    GetKernelStateSlab {
        result: oneshot::Sender<Result<NounSlab>>,
    },
    // Get the cold state as a NounSlab
    GetColdStateSlab {
        result: oneshot::Sender<NounSlab>,
    },
    // Run a peek
    Peek {
        ovo: NounSlab,
        result: oneshot::Sender<Result<NounSlab>>,
    },
    // Run a poke
    //
    // TODO: send back the event number after each poke
    Poke {
        wire: WireRepr,
        cause: NounSlab,
        result: oneshot::Sender<Result<NounSlab>>,
        result_ack: oneshot::Receiver<()>,
    },
    // Provide metrics
    ProvideMetrics {
        metrics: Arc<NockAppMetrics>,
        result: oneshot::Sender<()>,
    },
    // Stop the loop
    Stop,
}

pub struct SerfThread<C> {
    handle: Option<std::thread::JoinHandle<()>>,
    action_sender: mpsc::Sender<SerfAction<C>>,
    pub cancel_token: NockCancelToken,
    inhibit: Arc<AtomicBool>,
    pub event_number: Arc<AtomicU64>,
    pub(crate) pma_timing: Option<Arc<PmaTiming>>,
}

impl<C: SerfCheckpoint + Send + 'static> SerfThread<C> {
    pub async fn new(
        kernel_bytes: Vec<u8>,
        checkpoint: Option<C>,
        constant_hot_state: Vec<HotEntry>,
        nock_stack_size: usize,
        pma: Option<PmaConfig>,
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
    ) -> Result<Self> {
        let (action_sender, action_receiver) = mpsc::channel(1);
        let pma_timing = std::env::var_os("NOCK_PMA_TIMING")
            .is_some()
            .then(|| Arc::new(PmaTiming::default()));
        let (init_sender, init_receiver) = oneshot::channel();
        let inhibit = Arc::new(AtomicBool::new(false));
        let inhibit_clone = inhibit.clone();
        let pma_timing_thread = pma_timing.clone();
        let handle = std::thread::Builder::new()
            .name("serf".to_string())
            .stack_size(SERF_THREAD_STACK_SIZE)
            .spawn(move || {
                let ker_hash = {
                    let mut hasher = Hasher::new();
                    hasher.update(&kernel_bytes);
                    hasher.finalize()
                };
                let mut pma_meta_load = false;
                let (pma, pma_meta_path, pma_gc_state) = match pma {
                    Some(config) => {
                        let PmaConfig {
                            path_0,
                            path_1,
                            words,
                            open_existing,
                            gc_interval,
                        } = config;
                        let paths = PmaSlabPaths { path_0, path_1 };
                        let active = if open_existing {
                            select_active_pma_slab(&paths, ker_hash)
                        } else {
                            PmaSlab::Slab0
                        };
                        let active_path = paths.path(active).clone();
                        let pma_meta_path = pma_meta_path(&active_path);
                        let pma_meta_base = if open_existing {
                            PmaPersistMetadata::load_from_path(&pma_meta_path)
                                .map(|meta| meta.pma_base)
                        } else {
                            None
                        };
                        let pma_result = if open_existing && active_path.exists() {
                            pma_meta_load = true;
                            if let Some(base) = pma_meta_base {
                                match Pma::open_with_base(active_path.clone(), base) {
                                    Ok(pma) => Ok(pma),
                                    Err(err) => {
                                        let _ = init_sender.send(Err(CrownError::Unknown(
                                            format!(
                                                "PMA map failed at saved base {:#x}: {err}. To reset, pass --new or delete {} and {}",
                                                base,
                                                active_path.display(),
                                                pma_meta_path.display()
                                            ),
                                        )));
                                        return;
                                    }
                                }
                            } else {
                                Pma::open(active_path.clone())
                            }
                        } else {
                            pma_meta_load = false;
                            Pma::new(words, active_path.clone())
                        };
                        match pma_result {
                            Ok(pma) => (
                                Some(pma),
                                Some(pma_meta_path),
                                gc_interval.map(|interval| {
                                    PmaGcState::new(paths, active, interval, words)
                                }),
                            ),
                            Err(err) => {
                                let _ =
                                    init_sender.send(Err(CrownError::Unknown(err.to_string())));
                                return;
                            }
                        }
                    }
                    None => (None, None, None),
                };
                let stack = NockStack::new(nock_stack_size, 0);
                let serf = Serf::new(
                    stack,
                    pma,
                    pma_meta_path,
                    pma_meta_load,
                    pma_gc_state,
                    checkpoint,
                    &kernel_bytes,
                    &constant_hot_state,
                    test_jets,
                    trace,
                );
                let _ = init_sender.send(Ok((serf.event_num.clone(), serf.context.cancel_token())));
                serf_loop(serf, action_receiver, inhibit_clone, pma_timing_thread);
            })?;
        let (event_number, cancel_token) = init_receiver
            .await
            .map_err(|err| CrownError::Unknown(err.to_string()))??;
        Ok(SerfThread {
            inhibit,
            handle: Some(handle),
            action_sender,
            event_number,
            cancel_token,
            pma_timing,
        })
    }
}

impl<C> SerfThread<C> {
    pub(crate) fn provide_metrics(
        &mut self,
        metrics: Arc<NockAppMetrics>,
    ) -> impl Future<Output = Result<()>> {
        let action_sender = self.action_sender.clone();
        let (result, result_recv) = oneshot::channel();
        async move {
            action_sender
                .send(SerfAction::ProvideMetrics { metrics, result })
                .await?;
            Ok(result_recv.await?)
        }
    }

    pub(crate) fn stop(&mut self) -> impl Future<Output = Result<()>> {
        let action_sender = self.action_sender.clone();
        let cancel_token = self.cancel_token.clone();
        let join_handle = self.handle.take().expect("Serf join handle already taken.");
        let tokio_join_handle = tokio::task::spawn_blocking(move || join_handle.join());
        self.inhibit.store(true, Ordering::SeqCst);
        async move {
            cancel_token.cancel();
            action_sender
                .send(SerfAction::Stop)
                .await
                .expect("Failed to send stop action");
            match tokio_join_handle.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(CrownError::Unknown(format!("Serf thread panicked: {e:?}"))),
                Err(e) => Err(CrownError::JoinError(e)),
            }
        }
    }

    pub(crate) fn join(&mut self) -> Result<(), Box<dyn Any + Send + 'static>> {
        self.handle
            .take()
            .expect("Serf thread already joined")
            .join()
    }

    pub(crate) async fn get_kernel_state_slab(&self) -> Result<NounSlab> {
        let (result, result_fut) = oneshot::channel();
        self.action_sender
            .send(SerfAction::GetKernelStateSlab { result })
            .await?;
        result_fut.await?
    }

    pub(crate) async fn get_cold_state_slab(&self) -> Result<NounSlab> {
        let (result, result_fut) = oneshot::channel();
        self.action_sender
            .send(SerfAction::GetColdStateSlab { result })
            .await?;
        Ok(result_fut.await?)
    }

    pub(crate) fn peek(&self, ovo: NounSlab) -> impl Future<Output = Result<NounSlab>> {
        let (result, result_fut) = oneshot::channel();
        let action_sender = self.action_sender.clone();
        async move {
            action_sender.send(SerfAction::Peek { ovo, result }).await?;
            result_fut.await?
        }
    }

    // We are very carefully ensuring that the future does not contain the &self reference, to allow spawning a task without lifetime issues
    pub fn poke(&self, wire: WireRepr, cause: NounSlab) -> impl Future<Output = Result<NounSlab>> {
        let (result, result_fut) = oneshot::channel();
        let (result_ack_sender, result_ack) = oneshot::channel();
        let action_sender = self.action_sender.clone();
        async move {
            action_sender
                .send(SerfAction::Poke {
                    wire,
                    cause,
                    result,
                    result_ack,
                })
                .await?;
            let res = result_fut.await?;
            let _ = result_ack_sender.send(());
            res
        }
    }

    pub fn poke_timeout(
        &self,
        wire: WireRepr,
        cause: NounSlab,
        timeout: Duration,
    ) -> impl Future<Output = Result<NounSlab>> {
        let (result, result_fut) = oneshot::channel();
        let (result_ack_sender, result_ack) = oneshot::channel();
        let action_sender = self.action_sender.clone();
        let cancel = self.cancel_token.clone();
        let timer = tokio::time::sleep(timeout);
        let cancel_task = tokio::spawn(async move {
            timer.await;
            cancel.cancel();
        });
        async move {
            action_sender
                .send(SerfAction::Poke {
                    wire,
                    cause,
                    result,
                    result_ack,
                })
                .await?;
            let res = result_fut.await?;
            cancel_task.abort();
            let _ = cancel_task.await;
            let _ = result_ack_sender.send(());
            res
        }
    }

    pub(crate) fn poke_sync(&self, wire: WireRepr, cause: NounSlab) -> Result<NounSlab> {
        let (result, result_fut) = oneshot::channel();
        let (result_ack_sender, result_ack) = oneshot::channel();
        self.action_sender.blocking_send(SerfAction::Poke {
            wire,
            cause,
            result,
            result_ack,
        })?;
        let res = result_fut.blocking_recv()?;
        let _ = result_ack_sender.send(());
        res
    }

    pub(crate) fn peek_sync(&self, ovo: NounSlab) -> Result<NounSlab> {
        let (result, result_fut) = oneshot::channel();
        self.action_sender
            .blocking_send(SerfAction::Peek { ovo, result })?;
        result_fut.blocking_recv()?
    }

    pub(crate) fn checkpoint(&self) -> impl Future<Output = Result<C>> {
        let (result, result_fut) = oneshot::channel();
        let action_sender = self.action_sender.clone();
        async move {
            action_sender
                .send(SerfAction::Checkpoint { result })
                .await?;
            Ok(result_fut.await?)
        }
    }

    pub(crate) fn checkpoint_streaming(
        &self,
        request: StreamingCheckpointRequest,
    ) -> impl Future<Output = Result<StreamingCheckpointMeta>> {
        let (result, result_fut) = oneshot::channel();
        let action_sender = self.action_sender.clone();
        async move {
            action_sender
                .send(SerfAction::CheckpointStreaming { request, result })
                .await?;
            result_fut.await?
        }
    }

    pub fn import(&self, state: LoadState) -> impl Future<Output = Result<()>> {
        let (result, result_fut) = oneshot::channel();
        let action_sender = self.action_sender.clone();
        async move {
            action_sender
                .send(SerfAction::Import { state, result })
                .await?;
            result_fut.await?
        }
    }

    pub fn export(&self) -> impl Future<Output = Result<LoadState>> {
        let (result, result_fut) = oneshot::channel();
        let action_sender = self.action_sender.clone();
        async move {
            action_sender.send(SerfAction::Export { result }).await?;
            result_fut.await?
        }
    }
}

fn serf_loop<C: SerfCheckpoint>(
    mut serf: Serf,
    mut action_receiver: mpsc::Receiver<SerfAction<C>>,
    inhibit: Arc<AtomicBool>,
    pma_timing: Option<Arc<PmaTiming>>,
) {
    loop {
        let start = std::time::Instant::now();
        let Some(action) = action_receiver.blocking_recv() else {
            break;
        };
        let recv_elapsed = start.elapsed();
        if let Some(nockapp_metrics) = &serf.metrics {
            nockapp_metrics
                .serf_loop_blocking_recv
                .add_timing(&recv_elapsed);
        };
        let action_start = std::time::Instant::now();
        match action {
            SerfAction::Stop => {
                break;
            }
            SerfAction::Export { result } => {
                let space = serf.context.stack.noun_space();
                let kernel_state_noun = serf
                    .arvo
                    .in_space(&space)
                    .slot(STATE_AXIS)
                    .map(|handle| handle.noun());
                let kernel_state = kernel_state_noun.map_or_else(
                    |err| Err(CrownError::from(err)),
                    |noun| {
                        let mut slab = NounSlab::new();
                        slab.copy_into(noun, &space);
                        Ok(slab)
                    },
                );
                let load_state = kernel_state.map(|kernel_state| LoadState {
                    kernel_state,
                    ker_hash: serf.ker_hash,
                    event_num: serf.event_num.load(Ordering::SeqCst),
                });
                let _ = result.send(load_state).inspect_err(|_err| {
                    debug!("Failed to send to dropped channel");
                });
            }
            SerfAction::Import { state, result } => {
                let state_noun = state.kernel_state.copy_to_stack(serf.stack());
                let arvo = serf.load(state_noun);
                match arvo {
                    Err(e) => {
                        let _ = result.send(Err(e)).map_err(|err| {
                            debug!("Tried to send to dropped channel: {:?}", err);
                        });
                    }
                    Ok(arvo) => {
                        if serf.ker_hash != state.ker_hash {
                            debug!(
                                "Importing state from kernel hash {} into kernel hash {}",
                                state.ker_hash, serf.ker_hash
                            );
                        }
                        unsafe {
                            serf.event_update(state.event_num, arvo);
                            let _ = serf.preserve_event_update_leftovers();
                        }
                        let _ = result.send(Ok(())).map_err(|err| {
                            debug!("Tried to send to dropped channel: {:?}", err);
                        });
                    }
                }
            }
            SerfAction::GetKernelStateSlab { result } => {
                let space = serf.context.stack.noun_space();
                let kernel_state_noun = serf
                    .arvo
                    .in_space(&space)
                    .slot(STATE_AXIS)
                    .map(|handle| handle.noun());
                let kernel_state_slab = kernel_state_noun.map_or_else(
                    |err| Err(CrownError::from(err)),
                    |noun| {
                        let mut slab = NounSlab::new();
                        slab.copy_into(noun, &space);
                        Ok(slab)
                    },
                );
                let _ = result.send(kernel_state_slab).inspect_err(|_e| {
                    debug!("Tried to send to dropped result channel");
                });
                let action_elapsed = action_start.elapsed();
                if let Some(nockapp_metrics) = &serf.metrics {
                    nockapp_metrics
                        .serf_loop_get_kernel_state_slab
                        .add_timing(&action_elapsed);
                };
            }
            SerfAction::GetColdStateSlab { result } => {
                let cold_state_noun = serf.context.cold.into_noun(serf.stack());
                let cold_state_slab = {
                    let mut slab = NounSlab::new();
                    let space = serf.context.stack.noun_space();
                    slab.copy_into(cold_state_noun, &space);
                    slab
                };
                let _ = result.send(cold_state_slab).inspect_err(|_e| {
                    debug!("Could not send cold state to dropped channel.");
                });
                let action_elapsed = action_start.elapsed();
                if let Some(nockapp_metrics) = &serf.metrics {
                    nockapp_metrics
                        .serf_loop_get_cold_state_slab
                        .add_timing(&action_elapsed);
                };
            }
            SerfAction::Checkpoint { result } => {
                let metrics_checkpoint = serf.metrics.clone();
                let checkpoint = create_checkpoint(&mut serf, &metrics_checkpoint);
                //result.send(checkpoint).expect("Could not send checkpoint");
                if result.send(checkpoint).is_err() {
                    debug!(
                        "Checkpoint receiver dropped before receiving result - likely timed out"
                    );
                };
                let action_elapsed = action_start.elapsed();
                if let Some(nockapp_metrics) = &serf.metrics {
                    nockapp_metrics
                        .serf_loop_checkpoint
                        .add_timing(&action_elapsed);
                };
            }
            SerfAction::CheckpointStreaming { request, result } => {
                let metrics_checkpoint = serf.metrics.clone();
                let checkpoint =
                    create_streaming_checkpoint(&mut serf, request, &metrics_checkpoint);
                if result.send(checkpoint).is_err() {
                    debug!(
                        "Streaming checkpoint receiver dropped before receiving result - likely timed out"
                    );
                };
                let action_elapsed = action_start.elapsed();
                if let Some(nockapp_metrics) = &serf.metrics {
                    nockapp_metrics
                        .serf_loop_checkpoint
                        .add_timing(&action_elapsed);
                };
            }
            SerfAction::Peek { ovo, result } => {
                if inhibit.load(Ordering::SeqCst) {
                    let _ = result
                        .send(Err(CrownError::Unknown("Serf stopping".to_string())))
                        .inspect_err(|_e| {
                            debug!("Tried to send inhibited peek state to dropped channel");
                        });
                } else {
                    let ovo_noun = ovo.copy_to_stack(serf.stack());
                    let noun_res = serf.peek(ovo_noun);
                    let space = serf.context.stack.noun_space();
                    let noun_slab_res = noun_res.map(|noun| {
                        let mut slab = NounSlab::new();
                        slab.copy_into(noun, &space);
                        slab
                    });
                    let _ = result.send(noun_slab_res).inspect_err(|_e| {
                        debug!("Tried to send peek state to dropped channel");
                    });
                };
                let action_elapsed = action_start.elapsed();
                if let Some(nockapp_metrics) = &serf.metrics {
                    nockapp_metrics.serf_loop_peek.add_timing(&action_elapsed);
                };
            }
            SerfAction::Poke {
                wire,
                cause,
                result,
                result_ack,
            } => {
                if inhibit.load(Ordering::SeqCst) {
                    let _ = result
                        .send(Err(CrownError::Unknown("Serf stopping".to_string())))
                        .inspect_err(|_e| {
                            debug!("Failed to send inihibited poke result from serf thread");
                        });
                } else {
                    let cause_noun = cause.copy_to_stack(serf.stack());
                    let event_start = Instant::now();
                    let noun_res = serf.poke(wire, cause_noun);
                    let event_elapsed = event_start.elapsed();
                    let space = serf.context.stack.noun_space();
                    let (noun_slab_res, did_update) = match noun_res {
                        Ok(noun) => {
                            let mut slab = NounSlab::new();
                            slab.copy_into(noun, &space);
                            (Ok(slab), true)
                        }
                        Err(err) => (Err(err), false),
                    };
                    let _ = result.send(noun_slab_res).inspect_err(|_e| {
                        debug!("Failed to send poke result from serf thread");
                    });
                    let mut pma_elapsed = None;
                    let mut pma_detail = None;
                    if did_update {
                        let pma_start = Instant::now();
                        unsafe {
                            pma_detail = serf.preserve_event_update_leftovers();
                        }
                        pma_elapsed = Some(pma_start.elapsed());
                    }
                    if let Some(timing) = &pma_timing {
                        let pma_elapsed = pma_elapsed.unwrap_or_else(|| Duration::from_millis(0));
                        timing.record(event_elapsed, pma_elapsed, pma_detail);
                    }
                    if std::env::var_os("NOCK_PMA_TIMING").is_some() {
                        let event_ms = event_elapsed.as_secs_f64() * 1000.0;
                        let pma_ms = pma_elapsed
                            .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
                            .unwrap_or(0.0);
                        let total_ms = event_ms + pma_ms;
                        let total_alloc_words = pma_detail.map_or(0usize, |detail| {
                            detail.warm.alloc_words
                                + detail.test_jets.alloc_words
                                + detail.hot.alloc_words
                                + detail.cache.alloc_words
                                + detail.cold.alloc_words
                                + detail.arvo.alloc_words
                        });
                        let total_alloc_mib = (total_alloc_words as f64 * 8.0) / (1024.0 * 1024.0);
                        let event_num = serf.event_num.load(Ordering::SeqCst);
                        info!(
                            "pma-timing: event_ms={:.3} pma_copy_ms={:.3} total_ms={:.3} alloc_words={} alloc_mib={:.3} event_num={}",
                            event_ms,
                            pma_ms,
                            total_ms,
                            total_alloc_words,
                            total_alloc_mib,
                            event_num
                        );
                    }
                };
                let _ = result_ack.blocking_recv().inspect_err(|_e| {
                    debug!("Failed to receive result ack in serf thread");
                });
                let action_elapsed = action_start.elapsed();
                if let Some(nockapp_metrics) = &serf.metrics {
                    nockapp_metrics.serf_loop_poke.add_timing(&action_elapsed);
                };
            }
            SerfAction::ProvideMetrics { metrics, result } => {
                serf.metrics = Some(metrics);
                let _ = result.send(()).inspect_err(|_e| {
                    debug!("Failed to send metric-provision result from serf thread");
                });
                let action_elapsed = action_start.elapsed();
                if let Some(nockapp_metrics) = &serf.metrics {
                    nockapp_metrics
                        .serf_loop_provide_metrics
                        .add_timing(&action_elapsed);
                };
            }
        };
        let elapsed = start.elapsed();
        if let Some(nockapp_metrics) = &serf.metrics {
            nockapp_metrics.serf_loop_all.add_timing(&elapsed);
        };
    }
}

fn create_checkpoint<C: SerfCheckpoint>(
    serf: &mut Serf,
    metrics: &Option<Arc<NockAppMetrics>>,
) -> C {
    let ker_hash = serf.ker_hash;
    let event_num = serf.event_num.load(Ordering::SeqCst);
    let space = serf.context.stack.noun_space();
    let ker_state = serf
        .arvo
        .in_space(&space)
        .slot(STATE_AXIS)
        .map(|handle| handle.noun())
        .unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
    let cold_state = serf.context.cold;

    let checkpoint = C::new(
        serf.stack(),
        ker_hash,
        event_num,
        ker_state,
        cold_state,
        metrics,
    );

    if let Some(pma) = serf.pma.as_ref() {
        if let Err(err) = pma.sync_all() {
            warn!("Failed to msync PMA after checkpoint save: {err}");
        }
        if let Err(err) = pma.advise_drop_allocated_prefix(4, 5) {
            warn!("Failed to madvise PMA prefix after checkpoint save: {err}");
        }
    }

    checkpoint
}

fn create_streaming_checkpoint(
    serf: &mut Serf,
    request: StreamingCheckpointRequest,
    metrics: &Option<Arc<NockAppMetrics>>,
) -> Result<StreamingCheckpointMeta> {
    let ker_hash = serf.ker_hash;
    let event_num = serf.event_num.load(Ordering::SeqCst);
    let kernel_state_raw = {
        let space = serf.context.stack.noun_space();
        let kernel_state = serf
            .arvo
            .in_space(&space)
            .slot(STATE_AXIS)
            .map(|handle| handle.noun())?;
        unsafe { kernel_state.as_raw() }
    };

    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream start event_num={}",
        event_num
    );

    let (state_stats, state_elapsed) = {
        let pma = serf.pma.as_ref().ok_or_else(|| {
            CrownError::SaveError("streaming checkpoint requires PMA persistence".to_string())
        })?;

        info!(
            target: "nockapp::checkpoint::stream",
            "checkpoint stream msync start event_num={}",
            event_num
        );
        let sync_start = Instant::now();
        if let Err(err) = pma.sync_all() {
            warn!("Failed to msync PMA before streaming checkpoint: {err}");
        }
        let sync_elapsed = sync_start.elapsed();
        info!(
            target: "nockapp::checkpoint::stream",
            "checkpoint stream msync done event_num={} ms={}",
            event_num,
            sync_elapsed.as_millis()
        );

        let state_start = Instant::now();
        info!(
            target: "nockapp::checkpoint::stream",
            "checkpoint stream jam state start event_num={} path={}",
            event_num,
            request.state_path.display()
        );
        let state_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&request.state_path)
            .map_err(|err| CrownError::SaveError(format!("streaming state open failed: {err}")))?;
        let mut state_writer = BufWriter::new(state_file);
        let mut reader = PmaDirectReader::new(pma, PmaDirectJamConfig::default())
            .map_err(|err| CrownError::SaveError(format!("streaming reader init failed: {err}")))?;
        let state_stats = jam_pma_to_writer(&mut reader, kernel_state_raw, &mut state_writer)
            .map_err(|err| CrownError::SaveError(format!("streaming state jam failed: {err}")))?;
        state_writer
            .flush()
            .map_err(|err| CrownError::SaveError(format!("streaming state flush failed: {err}")))?;
        let state_elapsed = state_start.elapsed();
        info!(
            target: "nockapp::checkpoint::stream",
            "checkpoint stream jam state done event_num={} bytes={} ms={}",
            event_num,
            state_stats.byte_len,
            state_elapsed.as_millis()
        );
        (state_stats, state_elapsed)
    };

    let cold_start = Instant::now();
    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream jam cold start event_num={} path={}",
        event_num,
        request.cold_path.display()
    );
    let cold_noun = serf.context.cold.into_noun(serf.stack());
    let cold_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&request.cold_path)
        .map_err(|err| CrownError::SaveError(format!("streaming cold open failed: {err}")))?;
    let mut cold_writer = BufWriter::new(cold_file);
    let cold_stats = {
        let space = serf.context.stack.noun_space();
        jam_noun_to_writer(cold_noun, &space, &mut cold_writer)
            .map_err(|err| CrownError::SaveError(format!("streaming cold jam failed: {err}")))?
    };
    cold_writer
        .flush()
        .map_err(|err| CrownError::SaveError(format!("streaming cold flush failed: {err}")))?;
    let cold_elapsed = cold_start.elapsed();
    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream jam cold done event_num={} bytes={} ms={}",
        event_num,
        cold_stats.byte_len,
        cold_elapsed.as_millis()
    );

    if let Some(metrics) = metrics {
        let total = state_elapsed + cold_elapsed;
        metrics.save_jam_time.add_timing(&total);
    }

    if let Some(pma) = serf.pma.as_ref() {
        if let Err(err) = pma.advise_drop_allocated_prefix(4, 5) {
            warn!("Failed to madvise PMA prefix after streaming checkpoint: {err}");
        }
    }

    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream jam complete event_num={} state_bytes={} cold_bytes={}",
        event_num,
        state_stats.byte_len,
        cold_stats.byte_len
    );

    Ok(StreamingCheckpointMeta {
        ker_hash,
        event_num,
        state_bytes: state_stats.byte_len,
        cold_bytes: cold_stats.byte_len,
    })
}

/// Represents a Sword kernel, containing a Serf and snapshot location.
pub struct Kernel<C> {
    /// The Serf managing the interface to the Sword.
    pub(crate) serf: SerfThread<C>,
}

impl<C: SerfCheckpoint + 'static> Kernel<C> {
    /// Loads a kernel with a custom hot state.
    ///
    /// # Arguments
    ///
    /// * `snap_dir` - Directory for storing snapshots.
    /// * `kernel` - Byte slice containing the kernel as a jammed noun.
    /// * `hot_state` - Custom hot state entries.
    /// * `trace` - Whether to enable tracing.
    ///
    /// # Returns
    ///
    /// A new `Kernel` instance.
    pub async fn load_with_hot_state(
        kernel: &[u8],
        checkpoint: Option<C>,
        hot_state: &[HotEntry],
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
        pma: Option<PmaConfig>,
    ) -> Result<Self> {
        let kernel_vec = Vec::from(kernel);
        let hot_state_vec = Vec::from(hot_state);
        let serf = SerfThread::new(
            kernel_vec, checkpoint, hot_state_vec, NOCK_STACK_SIZE, pma, test_jets, trace,
        )
        .await?;
        Ok(Self { serf })
    }

    pub async fn load_with_hot_state_tiny(
        kernel: &[u8],
        checkpoint: Option<C>,
        hot_state: &[HotEntry],
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
        pma: Option<PmaConfig>,
    ) -> Result<Self> {
        let kernel_vec = Vec::from(kernel);
        let hot_state_vec = Vec::from(hot_state);
        let serf = SerfThread::new(
            kernel_vec, checkpoint, hot_state_vec, NOCK_STACK_SIZE_TINY, pma, test_jets, trace,
        )
        .await?;
        Ok(Self { serf })
    }

    pub async fn load_with_hot_state_small(
        kernel: &[u8],
        checkpoint: Option<C>,
        hot_state: &[HotEntry],
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
        pma: Option<PmaConfig>,
    ) -> Result<Self> {
        let kernel_vec = Vec::from(kernel);
        let hot_state_vec = Vec::from(hot_state);
        let serf = SerfThread::new(
            kernel_vec, checkpoint, hot_state_vec, NOCK_STACK_SIZE_SMALL, pma, test_jets, trace,
        )
        .await?;
        Ok(Self { serf })
    }

    pub async fn load_with_hot_state_medium(
        kernel: &[u8],
        checkpoint: Option<C>,
        hot_state: &[HotEntry],
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
        pma: Option<PmaConfig>,
    ) -> Result<Self> {
        let kernel_vec = Vec::from(kernel);
        let hot_state_vec = Vec::from(hot_state);
        let serf = SerfThread::new(
            kernel_vec, checkpoint, hot_state_vec, NOCK_STACK_SIZE_MEDIUM, pma, test_jets, trace,
        )
        .await?;
        Ok(Self { serf })
    }

    pub async fn load_with_hot_state_large(
        kernel: &[u8],
        checkpoint: Option<C>,
        hot_state: &[HotEntry],
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
        pma: Option<PmaConfig>,
    ) -> Result<Self> {
        let kernel_vec = Vec::from(kernel);
        let hot_state_vec = Vec::from(hot_state);
        let serf = SerfThread::new(
            kernel_vec, checkpoint, hot_state_vec, NOCK_STACK_SIZE_LARGE, pma, test_jets, trace,
        )
        .await?;
        Ok(Self { serf })
    }

    pub fn take_pma_timing_samples(&self) -> Option<Vec<(Duration, Duration)>> {
        self.serf.pma_timing.as_ref().map(|timing| {
            timing
                .take_samples()
                .into_iter()
                .map(|sample| (sample.event, sample.pma_copy))
                .collect()
        })
    }

    pub fn take_pma_timing_samples_detailed(&self) -> Option<Vec<PmaTimingSample>> {
        self.serf
            .pma_timing
            .as_ref()
            .map(|timing| timing.take_samples())
    }

    pub async fn load_with_hot_state_huge(
        kernel: &[u8],
        checkpoint: Option<C>,
        hot_state: &[HotEntry],
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
        pma: Option<PmaConfig>,
    ) -> Result<Self> {
        let kernel_vec = Vec::from(kernel);
        let hot_state_vec = Vec::from(hot_state);
        let serf = SerfThread::new(
            kernel_vec, checkpoint, hot_state_vec, NOCK_STACK_SIZE_HUGE, pma, test_jets, trace,
        )
        .await?;
        Ok(Self { serf })
    }

    /// Loads a kernel with default hot state.
    ///
    /// # Arguments
    ///
    /// * `snap_dir` - Directory for storing snapshots.
    /// * `kernel` - Byte slice containing the kernel code.
    /// * `trace` - Whether to enable tracing.
    ///
    /// # Returns
    ///
    /// A new `Kernel` instance.
    pub async fn load(
        kernel: &[u8],
        checkpoint: Option<C>,
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
        pma: Option<PmaConfig>,
    ) -> Result<Self> {
        Self::load_with_hot_state(kernel, checkpoint, &Vec::new(), test_jets, trace, pma).await
    }

    /// Produces a checkpoint of the kernel state.
    pub fn checkpoint(&self) -> impl Future<Output = Result<C>> {
        self.serf.checkpoint()
    }

    /// Produces a streaming checkpoint directly from the PMA slab.
    pub fn checkpoint_streaming(
        &self,
        request: StreamingCheckpointRequest,
    ) -> impl Future<Output = Result<StreamingCheckpointMeta>> {
        self.serf.checkpoint_streaming(request)
    }
}

impl<C> Kernel<C> {
    // We are very carefully ensuring the future does not contain the "self" reference to ensure no lifetime issues when spawning tasks
    pub fn poke(&self, wire: WireRepr, cause: NounSlab) -> impl Future<Output = Result<NounSlab>> {
        self.serf.poke(wire, cause)
    }

    pub fn poke_sync(&self, wire: WireRepr, cause: NounSlab) -> Result<NounSlab> {
        self.serf.poke_sync(wire, cause)
    }

    pub fn peek_sync(&self, ovo: NounSlab) -> Result<NounSlab> {
        self.serf.peek_sync(ovo)
    }

    pub fn poke_timeout(
        &self,
        wire: WireRepr,
        cause: NounSlab,
        timeout: Duration,
    ) -> impl Future<Output = Result<NounSlab>> {
        self.serf.poke_timeout(wire, cause, timeout)
    }

    // We are very carefully ensuring the future does not contain the "self" reference to ensure no lifetime issues when spawning tasks
    #[tracing::instrument(name = "crown::Kernel::peek", skip_all)]
    pub(crate) fn peek(&self, ovo: NounSlab) -> impl Future<Output = Result<NounSlab>> {
        self.serf.peek(ovo)
    }

    pub fn import(&self, state: LoadState) -> impl Future<Output = Result<()>> {
        self.serf.import(state)
    }

    pub fn export(&self) -> impl Future<Output = Result<LoadState>> {
        self.serf.export()
    }

    pub(crate) fn provide_metrics(
        &mut self,
        metrics: Arc<NockAppMetrics>,
    ) -> impl Future<Output = Result<()>> {
        self.serf.provide_metrics(metrics)
    }
}

/// Represents the Serf, which maintains context and provides an interface to
/// the Sword.
pub struct Serf {
    /// Hash of boot kernel
    pub ker_hash: Hash,
    /// The current Arvo state.
    pub arvo: Noun,
    /// The interpreter context.
    pub context: interpreter::Context,
    /// Persistent memory arena for long-lived state.
    pub pma: Option<Pma>,
    /// Optional metadata path for PMA persistence.
    pub pma_meta_path: Option<PathBuf>,
    /// Optional GC configuration for PMA slab compaction.
    pma_gc_state: Option<PmaGcState>,
    /// Cancellation
    pub cancel_token: NockCancelToken,
    /// The current event number.
    pub event_num: Arc<AtomicU64>,
    /// A metrics
    pub metrics: Option<Arc<NockAppMetrics>>,
}

impl Serf {
    /// Creates a new Serf instance.
    ///
    /// # Arguments
    ///
    /// * `stack` - The Nock stack.
    /// * `checkpoint` - Optional checkpoint to restore from.
    /// * `kernel_bytes` - Byte slice containing the kernel code.
    /// * `constant_hot_state` - Custom hot state entries.
    /// * `trace_info` - Optional nockvm tracing implementation.
    ///
    /// # Returns
    ///
    /// A new `Serf` instance.
    fn new<C: SerfCheckpoint>(
        mut stack: NockStack,
        mut pma: Option<Pma>,
        pma_meta_path: Option<PathBuf>,
        pma_meta_load: bool,
        pma_gc_state: Option<PmaGcState>,
        checkpoint: Option<C>,
        kernel_bytes: &[u8],
        constant_hot_state: &[HotEntry],
        test_jets: Vec<NounSlab>,
        trace: TraceOpts,
    ) -> Self {
        let hot_state = [URBIT_HOT_STATE, constant_hot_state].concat();

        if let Some(ref pma) = pma {
            stack.install_pma_arena(Arc::clone(pma.arena()));
        }

        let mut hasher = Hasher::new();
        hasher.update(kernel_bytes);
        let ker_hash = hasher.finalize();

        let mut reset_pma = false;
        let pma_state = if pma_meta_load && checkpoint.is_none() {
            if let (Some(pma), Some(meta_path)) = (pma.as_ref(), pma_meta_path.as_ref()) {
                if let Some(meta) = PmaPersistMetadata::load_from_path(meta_path) {
                    let pma_base = pma.arena().base_ptr() as u64;
                    if meta.pma_base != pma_base {
                        warn!(
                            "PMA metadata base mismatch (metadata: {:#x}, current: {:#x}); ignoring",
                            meta.pma_base, pma_base
                        );
                        reset_pma = true;
                        None
                    } else if meta.ker_hash == ker_hash {
                        let kernel_state = unsafe { Noun::from_raw(meta.kernel_state_raw) };
                        let cold = unsafe { Cold::from_pma_offset(pma, meta.cold_offset) };
                        Some((kernel_state, cold, meta.event_num))
                    } else {
                        warn!(
                            "PMA metadata kernel hash mismatch (metadata: {}, kernel: {}); ignoring",
                            meta.ker_hash, ker_hash
                        );
                        reset_pma = true;
                        None
                    }
                } else {
                    if meta_path.exists() {
                        warn!(
                            "Failed to load PMA metadata at {}; starting fresh",
                            meta_path.display()
                        );
                    }
                    reset_pma = true;
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if !pma_meta_load {
            if let Some(meta_path) = pma_meta_path.as_ref() {
                let _ = fs::remove_file(meta_path);
            }
        }

        if reset_pma {
            if let Some(pma) = pma.as_mut() {
                pma.reset();
            }
            if let Some(meta_path) = pma_meta_path.as_ref() {
                let _ = fs::remove_file(meta_path);
            }
        }

        let (maybe_state, cold, event_num_raw) = if let Some(c) = checkpoint {
            let saveable = c.load();

            let ker_state = saveable.state.copy_to_stack(&mut stack);
            let cold_noun = saveable.cold.copy_to_stack(&mut stack);
            let space = stack.noun_space();
            let cold_vecs = Cold::from_noun(&mut stack, &cold_noun, &space)
                .expect("Could not load cold state from snapshot");
            let cold = Cold::from_vecs(&mut stack, cold_vecs.0, cold_vecs.1, cold_vecs.2);
            if saveable.ker_hash != ker_hash {
                debug!(
                    "Loading snapshot from kernel {} into kernel {}",
                    saveable.ker_hash, ker_hash
                );
            }
            (Some(ker_state), cold, saveable.event_num)
        } else if let Some((ker_state, cold, event_num)) = pma_state {
            info!("Loaded PMA state at event_num {}", event_num);
            (Some(ker_state), cold, event_num)
        } else {
            (None, Cold::new(&mut stack), 0)
        };

        let event_num = Arc::new(AtomicU64::new(event_num_raw));

        let mut context = create_context(stack, &hot_state, cold, trace.into(), test_jets);
        let cancel_token = context.cancel_token();

        let mut arvo = {
            let kernel_trap = Noun::cue_bytes_slice(&mut context.stack, kernel_bytes)
                .expect("invalid kernel jam");
            let fol = T(&mut context.stack, &[D(9), D(2), D(0), D(1)]);

            if context.trace_info.is_some() {
                let start = Instant::now();
                let arvo = interpret(&mut context, kernel_trap, fol).unwrap_or_else(|err| {
                    panic!(
                        "Panicked with {err:?} at {}:{} (git sha: {:?})",
                        file!(),
                        line!(),
                        option_env!("GIT_SHA")
                    )
                });
                write_serf_trace_safe(&mut context, "boot", start);
                arvo
            } else {
                interpret(&mut context, kernel_trap, fol).unwrap_or_else(|err| {
                    panic!(
                        "Panicked with {err:?} at {}:{} (git sha: {:?})",
                        file!(),
                        line!(),
                        option_env!("GIT_SHA")
                    )
                })
            }
        };

        let mut serf = Self {
            ker_hash,
            arvo,
            context,
            pma,
            pma_meta_path,
            pma_gc_state,
            event_num,
            cancel_token,
            metrics: None,
        };

        if let Some(kernel_state) = maybe_state {
            arvo = serf.load(kernel_state).expect("serf: load failed");
        }

        unsafe {
            serf.event_update(event_num_raw, arvo);
            let _ = serf.preserve_event_update_leftovers();
        }
        serf
    }

    /// Performs a peek operation on the Arvo state.
    ///
    /// # Arguments
    ///
    /// * `ovo` - The peek request noun.
    ///
    /// # Returns
    ///
    /// Result containing the peeked data or an error.
    #[tracing::instrument(skip_all)]
    pub fn peek(&mut self, ovo: Noun) -> Result<Noun> {
        if self.context.trace_info.is_some() {
            let trace_name = "peek";
            let start = Instant::now();
            let slam_res = self.slam(PEEK_AXIS, ovo);
            write_serf_trace_safe(&mut self.context, trace_name, start);

            slam_res
        } else {
            self.slam(PEEK_AXIS, ovo)
        }
    }

    /// Generates a goof (error) noun.
    ///
    /// # Arguments
    ///
    /// * `mote` - The error mote.
    /// * `traces` - Trace information.
    ///
    /// # Returns
    ///
    /// A noun representing the error.
    pub fn goof(&mut self, mote: Mote, traces: Noun) -> Noun {
        let tone = Cell::new(&mut self.context.stack, D(2), traces);
        let space = self.context.stack.noun_space();
        let tang = mook(&mut self.context, tone, false)
            .expect("serf: goof: +mook crashed on bail")
            .in_space(&space)
            .tail()
            .noun();
        T(&mut self.context.stack, &[D(mote as u64), tang])
    }

    /// Performs a load operation on the Arvo state.
    ///
    /// # Arguments
    ///
    /// * `old` - The state to load.
    ///
    /// # Returns
    ///
    /// Result containing the loaded kernel or an error.
    pub fn load(&mut self, old: Noun) -> Result<Noun> {
        match self.soft(old, LOAD_AXIS, Some("load".to_string())) {
            Ok(res) => Ok(res),
            Err(goof) => {
                self.print_goof(goof);
                Err(CrownError::SerfLoadError)
            }
        }
    }

    pub fn print_goof(&mut self, goof: Noun) {
        let space = self.context.stack.noun_space();
        let tang = goof
            .in_space(&space)
            .as_cell()
            .expect("print goof: expected goof to be a cell")
            .tail()
            .noun();
        tang.in_space(&space).list_iter().for_each(|tank| {
            //  TODO: Slogger should be emitting Results in case of failure
            self.context
                .slogger
                .slog(&mut self.context.stack, 1, tank.noun());
        });
    }

    /// Performs a poke operation on the Arvo state.
    ///
    /// # Arguments
    ///
    /// * `job` - The poke job noun.
    ///
    /// # Returns
    ///
    /// Result containing the poke response or an error.
    #[tracing::instrument(level = "info", skip_all)]
    pub fn do_poke(&mut self, job: Noun) -> Result<Noun> {
        let space = self.context.stack.noun_space();
        match self.soft(job, POKE_AXIS, Some("poke".to_string())) {
            Ok(res) => {
                let cell = res
                    .in_space(&space)
                    .as_cell()
                    .expect("serf: poke: +slam returned atom");
                let fec = cell.head().noun();
                let eve = self.event_num.load(Ordering::SeqCst);

                unsafe {
                    self.event_update(eve + 1, cell.tail().noun());
                }
                Ok(fec)
            }
            Err(goof) => self.poke_swap(job, goof),
        }
    }

    /// Slams (applies) a gate at a specific axis of Arvo.
    ///
    /// # Arguments
    ///
    /// * `axis` - The axis to slam.
    /// * `ovo` - The sample noun.
    ///
    /// # Returns
    ///
    /// Result containing the slammed result or an error.
    pub fn slam(&mut self, axis: u64, ovo: Noun) -> Result<Noun> {
        let arvo = self.arvo;
        slam(&mut self.context, arvo, axis, ovo, self.metrics.clone())
    }

    /// Performs a "soft" computation, handling errors gracefully.
    ///
    /// # Arguments
    ///
    /// * `ovo` - The input noun.
    /// * `axis` - The axis to slam.
    /// * `trace_name` - Optional name for tracing.
    ///
    /// # Returns
    ///
    /// Result containing the computed noun or an error noun.
    fn soft(&mut self, ovo: Noun, axis: u64, trace_name: Option<String>) -> Result<Noun, Noun> {
        let slam_res = if self.context.trace_info.is_some() {
            let start = Instant::now();
            let slam_res = self.slam(axis, ovo);
            write_serf_trace_safe(
                &mut self.context,
                trace_name.as_ref().unwrap_or_else(|| {
                    panic!(
                        "Panicked at {}:{} (git sha: {:?})",
                        file!(),
                        line!(),
                        option_env!("GIT_SHA")
                    )
                }),
                start,
            );

            slam_res
        } else {
            self.slam(axis, ovo)
        };

        match slam_res {
            Ok(res) => Ok(res),
            Err(error) => match error {
                CrownError::InterpreterError(e) => {
                    let (mote, traces) = match e.0 {
                        Error::Deterministic(mote, traces)
                        | Error::NonDeterministic(mote, traces) => (mote, traces),
                        Error::ScryBlocked(_) | Error::ScryCrashed(_) => {
                            panic!("serf: soft: .^ invalid outside of virtual Nock")
                        }
                    };

                    Err(self.goof(mote, traces))
                }
                _ => Err(D(0)),
            },
        }
    }

    /// Plays a list of events.
    ///
    /// # Arguments
    ///
    /// * `lit` - The list of events to play.
    ///
    /// # Returns
    ///
    /// Result containing the final Arvo state or an error.
    fn play_list(&mut self, mut lit: Noun) -> Result<Noun> {
        let space = self.context.stack.noun_space();
        let mut eve = self.event_num.load(Ordering::SeqCst);
        while let Ok(cell) = lit.in_space(&space).as_cell() {
            let ovo = cell.head().noun();
            lit = cell.tail().noun();
            let trace_name = if self.context.trace_info.is_some() {
                Some(format!("play [{}]", eve))
            } else {
                None
            };

            match self.soft(ovo, POKE_AXIS, trace_name) {
                Ok(res) => {
                    let arvo = res.in_space(&space).as_cell()?.tail().noun();
                    eve += 1;

                    unsafe {
                        self.event_update(eve, arvo);
                    }
                }
                Err(goof) => {
                    return Err(CrownError::KernelError(Some(goof)));
                }
            }
        }
        Ok(self.arvo)
    }

    /// Handles a poke error by swapping in a new event.
    ///
    /// # Arguments
    ///
    /// * `job` - The original poke job.
    /// * `goof` - The error noun.
    ///
    /// # Returns
    ///
    /// Result containing the new event or an error.
    fn poke_swap(&mut self, job: Noun, goof: Noun) -> Result<Noun> {
        let stack = &mut self.context.stack;
        let space = stack.noun_space();
        self.context.cache = Hamt::<Noun>::new(stack);
        let job_cell = job
            .in_space(&space)
            .as_cell()
            .expect("serf: poke: job not a cell");
        // job data is job without event_num
        let job_data = job_cell
            .tail()
            .as_cell()
            .expect("serf: poke: data not a cell");
        //  job input is job without event_num or wire
        let job_input = job_data.tail().noun();
        let wire = T(stack, &[D(0), D(tas!(b"arvo")), D(0)]);
        let crud = DirectAtom::new_panic(tas!(b"crud"));
        let event_num = D(self.event_num.load(Ordering::SeqCst) + 1);

        let ovo = T(stack, &[event_num, wire, goof, job_input]);
        let trace_name = if self.context.trace_info.is_some() {
            Some(Self::poke_trace_name(
                &mut self.context.stack,
                wire,
                crud.as_atom(),
            ))
        } else {
            None
        };

        match self.soft(ovo, POKE_AXIS, trace_name) {
            Ok(res) => {
                let cell = res
                    .in_space(&space)
                    .as_cell()
                    .expect("serf: poke: crud +slam returned atom");
                let fec = cell.head().noun();
                let eve = self.event_num.load(Ordering::SeqCst);

                unsafe {
                    self.event_update(eve + 1, cell.tail().noun());
                }
                Ok(fec)
            }
            Err(goof_crud) => Err(CrownError::KernelError(Some(goof_crud))),
        }
    }

    /// Generates a trace name for a poke operation.
    ///
    /// # Arguments
    ///
    /// * `stack` - The Nock stack.
    /// * `wire` - The wire noun.
    /// * `vent` - The vent atom.
    ///
    /// # Returns
    ///
    /// A string representing the trace name.
    fn poke_trace_name(stack: &mut NockStack, wire: Noun, vent: Atom) -> String {
        let wpc = path_to_cord(stack, wire);
        let space = stack.noun_space();
        let wpc_len = met3_usize(wpc, &space);
        let wpc_handle = wpc.in_space(&space);
        let wpc_bytes = &wpc_handle.as_ne_bytes()[0..wpc_len];
        let wpc_str = match std::str::from_utf8(wpc_bytes) {
            Ok(valid) => valid,
            Err(error) => {
                let (valid, _) = wpc_bytes.split_at(error.valid_up_to());
                unsafe { std::str::from_utf8_unchecked(valid) }
            }
        };

        let vc_len = met3_usize(vent, &space);
        let vent_handle = vent.in_space(&space);
        let vc_bytes = &vent_handle.as_ne_bytes()[0..vc_len];
        let vc_str = match std::str::from_utf8(vc_bytes) {
            Ok(valid) => valid,
            Err(error) => {
                let (valid, _) = vc_bytes.split_at(error.valid_up_to());
                unsafe { std::str::from_utf8_unchecked(valid) }
            }
        };

        format!("poke [{} {}]", wpc_str, vc_str)
    }

    /// Performs a poke operation with a given cause.
    ///
    /// # Arguments
    ///
    /// * `wire` - The wire noun.
    /// * `cause` - The cause noun.
    ///
    /// # Returns
    ///
    /// Result containing the poke response or an error.
    #[tracing::instrument(level = "info", skip_all, fields(
        src = wire.source
    ))]
    pub fn poke(&mut self, wire: WireRepr, cause: Noun) -> Result<Noun> {
        let random_bytes = rand::random::<u64>();
        let bytes = random_bytes.as_bytes()?;
        let eny: Atom = Atom::from_bytes(&mut self.context.stack, &bytes);
        let our = <nockvm::noun::Atom as AtomExt>::from_value(&mut self.context.stack, 0)?; // Using 0 as default value
        let mut t_vec: Vec<u8> = vec![];
        t_vec.write_u128::<LittleEndian>(current_da().0)?;
        let now: Atom = <IndirectAtom as IndirectAtomExt>::from_bytes(
            &mut self.context.stack,
            t_vec.as_slice(),
        );

        let event_num = D(self.event_num.load(Ordering::SeqCst) + 1);
        let base_wire_noun = wire_to_noun(&mut self.context.stack, &wire);
        let wire = T(&mut self.context.stack, &[D(tas!(b"poke")), base_wire_noun]);
        let poke = T(
            &mut self.context.stack,
            &[event_num, wire, eny.as_noun(), our.as_noun(), now.as_noun(), cause],
        );

        self.do_poke(poke)
    }

    /// Updates the Serf's state after an event.
    ///
    /// # Arguments
    ///
    /// * `new_event_num` - The new event number.
    /// * `new_arvo` - The new Arvo state.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it modifies the Serf's state directly.
    #[tracing::instrument(level = "info", skip_all)]
    pub unsafe fn event_update(&mut self, new_event_num: u64, new_arvo: Noun) {
        self.arvo = new_arvo;
        self.event_num.store(new_event_num, Ordering::SeqCst);

        self.context.cache = Hamt::new(&mut self.context.stack);
        self.context.scry_stack = D(0);
    }

    unsafe fn copy_segment<T: PmaCopy>(
        label: &str,
        value: &mut T,
        stack: &NockStack,
        pma: &mut Pma,
        trace_pma: bool,
        segment: Option<&mut PmaCopySegment>,
    ) {
        if trace_pma {
            info!("pma-copy: {label}");
        }
        if let Some(segment) = segment {
            let before = pma.alloc_offset();
            let start = Instant::now();
            value.copy_to_pma(stack, pma);
            segment.elapsed = start.elapsed();
            segment.alloc_words = pma.alloc_offset().saturating_sub(before);
        } else {
            value.copy_to_pma(stack, pma);
        }
    }

    unsafe fn copy_persistent_state_to_pma(
        &mut self,
        pma: &mut Pma,
        trace_pma: bool,
        detail: Option<&mut PmaCopyDetail>,
    ) {
        let stack = &mut self.context.stack;
        if let Some(detail) = detail {
            Self::copy_segment(
                "warm",
                &mut self.context.warm,
                stack,
                pma,
                trace_pma,
                Some(&mut detail.warm),
            );
            Self::copy_segment(
                "test_jets",
                &mut self.context.test_jets,
                stack,
                pma,
                trace_pma,
                Some(&mut detail.test_jets),
            );
            Self::copy_segment(
                "hot",
                &mut self.context.hot,
                stack,
                pma,
                trace_pma,
                Some(&mut detail.hot),
            );
            Self::copy_segment(
                "cache",
                &mut self.context.cache,
                stack,
                pma,
                trace_pma,
                Some(&mut detail.cache),
            );
            Self::copy_segment(
                "cold",
                &mut self.context.cold,
                stack,
                pma,
                trace_pma,
                Some(&mut detail.cold),
            );
            Self::copy_segment(
                "arvo",
                &mut self.arvo,
                stack,
                pma,
                trace_pma,
                Some(&mut detail.arvo),
            );
        } else {
            Self::copy_segment("warm", &mut self.context.warm, stack, pma, trace_pma, None);
            Self::copy_segment(
                "test_jets", &mut self.context.test_jets, stack, pma, trace_pma, None,
            );
            Self::copy_segment("hot", &mut self.context.hot, stack, pma, trace_pma, None);
            Self::copy_segment(
                "cache", &mut self.context.cache, stack, pma, trace_pma, None,
            );
            Self::copy_segment("cold", &mut self.context.cold, stack, pma, trace_pma, None);
            Self::copy_segment("arvo", &mut self.arvo, stack, pma, trace_pma, None);
        }
    }

    #[cfg(feature = "pma-assert")]
    fn assert_persistent_state_in_pma(&self, pma: &Pma) {
        self.context.warm.assert_in_pma(pma);
        self.context.test_jets.assert_in_pma(pma);
        self.context.hot.assert_in_pma(pma);
        self.context.cache.assert_in_pma(pma);
        self.context.cold.assert_in_pma(pma);
        self.arvo.assert_in_pma(pma);
    }

    unsafe fn preserve_persistent_state_in_stack(&mut self) {
        let stack = &mut self.context.stack;
        stack.preserve(&mut self.context.warm);
        stack.preserve(&mut self.context.test_jets);
        stack.preserve(&mut self.context.hot);
        stack.preserve(&mut self.context.cache);
        stack.preserve(&mut self.context.cold);
        stack.preserve(&mut self.arvo);
    }

    fn persist_pma_metadata(&self, pma: &Pma) {
        let Some(meta_path) = self.pma_meta_path.as_ref() else {
            return;
        };
        let space = self.context.stack.noun_space();
        let kernel_state = self
            .arvo
            .in_space(&space)
            .slot(STATE_AXIS)
            .map(|handle| handle.noun())
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        let kernel_state_raw = unsafe { kernel_state.as_raw() };
        let Some(cold_offset) = self.context.cold.pma_offset(pma) else {
            warn!("PMA metadata update skipped: cold state not in PMA");
            return;
        };
        let event_num = self.event_num.load(Ordering::SeqCst);
        let pma_base = pma.arena().base_ptr() as u64;
        let meta = PmaPersistMetadata::new(
            self.ker_hash, event_num, kernel_state_raw, cold_offset, pma_base,
        );
        if let Err(err) = meta.save_to_path(meta_path) {
            warn!(
                "Failed to persist PMA metadata to {}: {err}",
                meta_path.display()
            );
        }
    }

    fn maybe_pma_gc(&mut self, mut pma: Pma) -> Pma {
        let Some(gc_state) = self.pma_gc_state.as_ref() else {
            return pma;
        };
        if gc_state.last_gc.elapsed() < gc_state.interval {
            return pma;
        }

        let from_path = gc_state.active_path().clone();
        let to_path = gc_state.inactive_path().clone();
        let gc_words = gc_state.words;
        let event_num = self.event_num.load(Ordering::SeqCst);
        let gc_start = Instant::now();
        let from_alloc = pma.alloc_offset();
        info!(
            "pma-gc: start: event_num={} from={} to={} from_alloc_words={}",
            event_num,
            from_path.display(),
            to_path.display(),
            from_alloc
        );

        let create_start = Instant::now();
        let mut to_pma = match Pma::new(gc_words, to_path.clone()) {
            Ok(pma) => pma,
            Err(err) => {
                warn!(
                    "pma-gc: failed to create new PMA slab at {}: {err}",
                    to_path.display()
                );
                return pma;
            }
        };
        let create_elapsed = create_start.elapsed();

        let warm_start = Instant::now();
        unsafe {
            self.context.warm.copy_from_pma(&pma, &mut to_pma);
        }
        let warm_elapsed = warm_start.elapsed();

        let test_jets_start = Instant::now();
        unsafe {
            self.context.test_jets.copy_from_pma(&pma, &mut to_pma);
        }
        let test_jets_elapsed = test_jets_start.elapsed();

        let hot_start = Instant::now();
        unsafe {
            self.context.hot.copy_from_pma(&pma, &mut to_pma);
        }
        let hot_elapsed = hot_start.elapsed();

        let cache_start = Instant::now();
        unsafe {
            self.context.cache.copy_from_pma(&pma, &mut to_pma);
        }
        let cache_elapsed = cache_start.elapsed();

        let cold_start = Instant::now();
        unsafe {
            self.context.cold.copy_from_pma(&pma, &mut to_pma);
        }
        let cold_elapsed = cold_start.elapsed();

        let arvo_start = Instant::now();
        unsafe {
            self.arvo.copy_from_pma(&pma, &mut to_pma);
        }
        let arvo_elapsed = arvo_start.elapsed();

        info!(
            "pma-gc: copy timings: warm_ms={} test_jets_ms={} hot_ms={} cache_ms={} cold_ms={} arvo_ms={}",
            warm_elapsed.as_millis(),
            test_jets_elapsed.as_millis(),
            hot_elapsed.as_millis(),
            cache_elapsed.as_millis(),
            cold_elapsed.as_millis(),
            arvo_elapsed.as_millis()
        );

        self.context
            .stack
            .install_pma_arena(Arc::clone(to_pma.arena()));
        self.pma_meta_path = Some(pma_meta_path(&to_path));
        to_pma.persist_metadata();
        self.persist_pma_metadata(&to_pma);

        let to_alloc = to_pma.alloc_offset();
        if let Some(gc_state) = self.pma_gc_state.as_mut() {
            gc_state.mark_gc_completed();
        }
        info!(
            "pma-gc: done: total_ms={} create_ms={} to_alloc_words={}",
            gc_start.elapsed().as_millis(),
            create_elapsed.as_millis(),
            to_alloc
        );

        pma = to_pma;
        pma
    }

    /// Preserves leftovers after an event update.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it modifies the Serf's state directly.
    #[tracing::instrument(level = "info", skip_all)]
    pub unsafe fn preserve_event_update_leftovers(&mut self) -> Option<PmaCopyDetail> {
        assert!(
            self.context.scry_stack.is_direct(),
            "scry_stack must be cleared before resetting the NockStack"
        );
        if std::env::var_os("NOCK_STACK_TIMING_DETAIL").is_some() {
            let total_words = self.context.stack.arena().words() as u64;
            let least_space = self.context.stack.least_space() as u64;
            let used_words = total_words.saturating_sub(least_space);
            let used_mib = (used_words as f64 * 8.0) / (1024.0 * 1024.0);
            let event_num = self.event_num.load(Ordering::SeqCst);
            info!(
                "stack-usage: used_words={} used_mib={:.3} least_space_words={} total_words={} event_num={}",
                used_words,
                used_mib,
                least_space,
                total_words,
                event_num
            );
        }
        if self.pma.is_some() {
            let trace_pma = std::env::var_os("NOCK_PMA_TRACE").is_some();
            let detail_enabled = std::env::var_os("NOCK_PMA_TIMING_DETAIL").is_some();
            let mut pma = self.pma.take().expect("checked is_some");
            let mut detail = if detail_enabled {
                Some(PmaCopyDetail::default())
            } else {
                None
            };
            self.copy_persistent_state_to_pma(&mut pma, trace_pma, detail.as_mut());
            #[cfg(feature = "pma-assert")]
            {
                // Enforce: PMA data must not reference the NockStack.
                self.assert_persistent_state_in_pma(&pma);
            }

            pma.persist_metadata();
            self.persist_pma_metadata(&pma);

            pma = self.maybe_pma_gc(pma);
            self.context.stack.reset(0);
            self.pma = Some(pma);
            detail
        } else {
            self.preserve_persistent_state_in_stack();
            self.context.stack.flip_top_frame(0);
            None
        }
    }

    /// Returns a mutable reference to the Nock stack.
    ///
    /// # Returns
    ///
    /// A mutable reference to the `NockStack`.
    pub fn stack(&mut self) -> &mut NockStack {
        &mut self.context.stack
    }

    /// Creates a poke swap noun.
    ///
    /// # Arguments
    ///
    /// * `eve` - The event number.
    /// * `mug` - The mug value.
    /// * `ovo` - The original noun.
    /// * `fec` - The effect noun.
    ///
    /// # Returns
    ///
    /// A noun representing the poke swap.
    pub fn poke_bail(&mut self, eve: u64, mug: u64, ovo: Noun, fec: Noun) -> Noun {
        T(
            self.stack(),
            &[D(tas!(b"poke")), D(tas!(b"swap")), D(eve), D(mug), ovo, fec],
        )
    }

    /// Creates a poke bail noun.
    ///
    /// # Arguments
    ///
    /// * `lud` - The lud noun.
    ///
    /// # Returns
    ///
    /// A noun representing the poke bail.
    pub fn poke_bail_noun(&mut self, lud: Noun) -> Noun {
        T(self.stack(), &[D(tas!(b"poke")), D(tas!(b"bail")), lud])
    }
}

fn slot(noun: Noun, axis: u64, space: &NounSpace) -> Result<Noun> {
    Ok(noun.in_space(space).slot(axis)?.noun())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use nockvm::jets::cold::Cold;
    use nockvm::jets::hot::HotEntry;

    use super::*;

    fn dummy_serf() -> Serf {
        let mut stack = NockStack::new(1 << 18, 0);
        let cold = Cold::new(&mut stack);
        let hot_state: [HotEntry; 0] = [];
        let context = create_context(stack, &hot_state, cold, None, vec![]);
        let cancel_token = context.cancel_token();
        Serf {
            ker_hash: Hash::from([0; 32]),
            arvo: D(0),
            context,
            pma: None,
            pma_meta_path: None,
            pma_gc_state: None,
            cancel_token,
            event_num: Arc::new(AtomicU64::new(0)),
            metrics: None,
        }
    }

    async fn setup_kernel(jam: &str) -> Kernel<SaveableCheckpoint> {
        let jam_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("assets")
            .join(jam);
        let jam_bytes =
            fs::read(jam_path).unwrap_or_else(|_| panic!("Failed to read {} file", jam));
        Kernel::load(&jam_bytes, None, vec![], TraceOpts::default(), None)
            .await
            .expect("Could not load kernel")
    }

    // Convert this to an integration test and feed it the kernel.jam from Choo in CI/CD
    // https://www.youtube.com/watch?v=4m1EFMoRFvY
    // #[test]
    // #[cfg_attr(miri, ignore)]
    // fn test_kernel_boot() {
    //     let _ = setup_kernel("dumb.jam");
    // }

    // To test your own kernel, place a `kernel.jam` file in the `assets` directory
    // and uncomment the following test:
    //
    // #[test]
    // fn test_custom_kernel() {
    //     let (kernel, _temp_dir) = setup_kernel("kernel.jam");
    //     // Add your custom assertions here to test the kernel's behavior
    // }

    // preserve_event_update_leftovers no longer retags survivors into offsets.
}

pub trait SerfCheckpoint: Send {
    fn new(
        stack: &mut NockStack,
        ker_hash: Hash,
        event_num: u64,
        kernel_state: Noun,
        cold_state: Cold,
        metrics: &Option<Arc<NockAppMetrics>>,
    ) -> Self;

    fn load(self) -> SaveableCheckpoint;
}

impl SerfCheckpoint for SaveableCheckpoint {
    fn new(
        stack: &mut NockStack,
        ker_hash: Hash,
        event_num: u64,
        kernel_state: Noun,
        cold_state: Cold,
        metrics: &Option<Arc<NockAppMetrics>>,
    ) -> Self {
        let cold_noun_start = Instant::now();
        // Cold state has nouns in it which are *not* copied in into_noun
        // TODO: FIX THIS FOOTGUN
        let cold_stack_noun = cold_state.into_noun(stack);
        let space = stack.noun_space();
        let mut cold_slab: NounSlab = NounSlab::new();
        let cold_copy = cold_slab.copy_into(cold_stack_noun, &space);
        cold_slab.set_root(cold_copy);
        let cold_noun_elapsed = cold_noun_start.elapsed();

        let state_copy_start = Instant::now();
        let mut state_slab: NounSlab = NounSlab::new();
        let state_copy = state_slab.copy_into(kernel_state, &space);
        state_slab.set_root(state_copy);
        let state_copy_elapsed = state_copy_start.elapsed();

        if let Some(metrics) = metrics {
            metrics
                .serf_loop_noun_encode_cold_state
                .add_timing(&cold_noun_elapsed);
            metrics
                .serf_loop_copy_state_noun
                .add_timing(&state_copy_elapsed);
        }
        Self {
            ker_hash,
            event_num,
            state: state_slab,
            cold: cold_slab,
        }
    }

    fn load(self) -> SaveableCheckpoint {
        self
    }
}
