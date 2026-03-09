use std::future::Future;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bincode::config::Configuration;
use bincode::serde::Compat;
use bincode::{config, encode_to_vec, Decode, Encode};
use blake3::{Hash, Hasher};
use bytes::Bytes;
use nockvm::noun::NounAllocator;
use nockvm_macros::tas;
use tempfile::{Builder, TempPath};
use thiserror::Error;
use tokio::fs::create_dir_all;
use tokio::sync::oneshot;
use tokio::task::spawn_blocking;
use tracing::{debug, error, info, trace, warn};

use crate::metrics::NockAppMetrics;
use crate::noun::slab::{Jammer, NockJammer, NounSlab};
use crate::JammedNoun;

pub const JAM_MAGIC_BYTES: u64 = tas!(b"CHKJAM");
const SNAPSHOT_VERSION_0: u32 = 0;
const SNAPSHOT_VERSION_1: u32 = 1;
const SNAPSHOT_VERSION_2: u32 = 2;
pub const LATEST_SNAPSHOT_VERSION: u32 = SNAPSHOT_VERSION_2;

pub enum WhichSnapshot {
    Snapshot0,
    Snapshot1,
}

impl WhichSnapshot {
    pub fn next(&self) -> Self {
        match self {
            WhichSnapshot::Snapshot0 => WhichSnapshot::Snapshot1,
            WhichSnapshot::Snapshot1 => WhichSnapshot::Snapshot0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StreamingCheckpointRequest {
    pub state_path: PathBuf,
    pub cold_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct StreamingCheckpointMeta {
    pub ker_hash: Hash,
    pub event_num: u64,
    pub state_bytes: usize,
    pub cold_bytes: usize,
}

#[derive(Debug)]
pub struct StreamingCheckpoint {
    pub meta: StreamingCheckpointMeta,
    pub state_path: TempPath,
    pub cold_path: TempPath,
}

impl StreamingCheckpoint {
    pub fn new(meta: StreamingCheckpointMeta, state_path: TempPath, cold_path: TempPath) -> Self {
        Self {
            meta,
            state_path,
            cold_path,
        }
    }

    fn state_path(&self) -> &Path {
        self.state_path.as_ref()
    }

    fn cold_path(&self) -> &Path {
        self.cold_path.as_ref()
    }
}

/// State object which handles all NockApp saves and loads
pub struct Saver<J = NockJammer> {
    path_0: PathBuf,
    path_1: PathBuf,
    save_to_next: WhichSnapshot,
    waiters: Vec<(u64, oneshot::Sender<()>)>,
    last_event_num: u64,
    _phantom: std::marker::PhantomData<J>,
}

#[derive(Clone, Debug)]
pub(crate) struct CheckpointSummary {
    pub path: PathBuf,
    pub event_num: u64,
    pub checksum: Hash,
}

struct SaverState {
    save_to_next: WhichSnapshot,
    last_event_num: u64,
}

impl<J> Saver<J> {
    pub(crate) fn new_empty(path: &PathBuf) -> Self {
        Self::from_state(
            path,
            SaverState {
                save_to_next: WhichSnapshot::Snapshot0,
                last_event_num: 0,
            },
        )
    }

    pub(crate) fn from_boot_event_num(path: &PathBuf, event_num: u64) -> Self {
        Self::from_state(
            path,
            SaverState {
                save_to_next: WhichSnapshot::Snapshot0,
                last_event_num: event_num,
            },
        )
    }

    fn from_state(path: &PathBuf, state: SaverState) -> Self {
        let path_0 = path.join("0.chkjam");
        let path_1 = path.join("1.chkjam");
        Self {
            path_0,
            path_1,
            save_to_next: state.save_to_next,
            waiters: Vec::new(),
            last_event_num: state.last_event_num,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn last_path(&self) -> PathBuf {
        match self.save_to_next {
            WhichSnapshot::Snapshot1 => self.path_0.clone(),
            WhichSnapshot::Snapshot0 => self.path_1.clone(),
        }
    }

    pub fn next_path(&self) -> PathBuf {
        match self.save_to_next {
            WhichSnapshot::Snapshot1 => self.path_1.clone(),
            WhichSnapshot::Snapshot0 => self.path_0.clone(),
        }
    }

    pub(crate) async fn resume(
        path: &PathBuf,
    ) -> Result<(Self, Option<CheckpointSummary>), CheckpointError> {
        let (state, _, summary) = inspect_latest(path).await?;
        Ok((Self::from_state(path, state), summary))
    }

    /// The future from this function should not be awaited before any mutex
    /// around the 'Saver' is released, or a deadlock will result.
    #[tracing::instrument(skip(self))]
    #[allow(clippy::async_yields_async)]
    pub async fn wait_for_snapshot<'a>(
        &'a mut self,
        wait_for_event_num: u64,
    ) -> impl Future<Output = Result<(), oneshot::error::RecvError>> {
        if self.last_event_num >= wait_for_event_num {
            return futures::future::Either::Left(std::future::ready(Ok(())));
        }

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.waiters.push((wait_for_event_num, tx));
        futures::future::Either::Right(rx)
    }

    /// Check if we need to save
    pub fn save_needed(&self, event_num: u64) -> bool {
        self.last_event_num < event_num
    }

    async fn save_jammed_checkpoint(
        &mut self,
        checkpoint: JammedCheckpoint,
    ) -> Result<(), CheckpointError> {
        let event_num = checkpoint.event_num;
        trace!("Saving checkpoint at event_num {}", event_num);
        let path = self.next_path();
        checkpoint.save_to_file(&path).await?;
        self.save_to_next = self.save_to_next.next();
        debug!(
            "Saved checkpoint to file: {}",
            &path.as_os_str().to_str().unwrap_or("<invalid-path>")
        );

        let mut still_waiting = Vec::new();
        for (waiting_event_num, waiter) in self.waiters.drain(..) {
            if waiting_event_num <= event_num {
                let _ = waiter.send(());
            } else {
                still_waiting.push((waiting_event_num, waiter));
            }
        }

        self.last_event_num = event_num;
        self.waiters = still_waiting;
        Ok(())
    }

    pub async fn save_jammed(
        &mut self,
        checkpoint: JammedCheckpoint,
    ) -> Result<(), CheckpointError> {
        self.save_jammed_checkpoint(checkpoint).await
    }

    pub async fn save_streaming(
        &mut self,
        checkpoint: StreamingCheckpoint,
    ) -> Result<(), CheckpointError> {
        let event_num = checkpoint.meta.event_num;
        trace!("Saving streaming checkpoint at event_num {}", event_num);
        let path = self.next_path();
        let path_for_log = path.clone();
        let write_result =
            spawn_blocking(move || write_streaming_checkpoint(&path, checkpoint)).await;
        match write_result {
            Ok(result) => result?,
            Err(err) => {
                return Err(CheckpointError::IOError(io::Error::new(
                    io::ErrorKind::Other,
                    err,
                )))
            }
        }
        self.save_to_next = self.save_to_next.next();
        debug!(
            "Saved streaming checkpoint to file: {}",
            &path_for_log
                .as_os_str()
                .to_str()
                .unwrap_or("<invalid-path>")
        );

        let mut still_waiting = Vec::new();
        for (waiting_event_num, waiter) in self.waiters.drain(..) {
            if waiting_event_num <= event_num {
                let _ = waiter.send(());
            } else {
                still_waiting.push((waiting_event_num, waiter));
            }
        }

        self.last_event_num = event_num;
        self.waiters = still_waiting;
        Ok(())
    }
}

impl<J: Jammer> Saver<J> {
    pub(crate) async fn load_latest<C: Checkpoint>(
        path: &PathBuf,
        metrics: Option<Arc<NockAppMetrics>>,
    ) -> Result<Option<C>, CheckpointError> {
        let (_, loaded_checkpoint, _) = inspect_latest(path).await?;
        loaded_checkpoint
            .map(|checkpoint| {
                let saveable = checkpoint.into_saveable::<J>(metrics)?;
                C::from_saveable(saveable)
            })
            .transpose()
    }

    pub async fn try_load<C: Checkpoint>(
        path: &PathBuf,
        metrics: Option<Arc<NockAppMetrics>>,
    ) -> Result<(Self, Option<C>), CheckpointError> {
        let (state, loaded_checkpoint, _) = inspect_latest(path).await?;
        let saver = Self::from_state(path, state);
        let checkpoint = loaded_checkpoint
            .map(|loaded| {
                let saveable = loaded.into_saveable::<J>(metrics.clone())?;
                trace!("After from_jammed_checkpoint");
                C::from_saveable(saveable)
            })
            .transpose()?;
        Ok((saver, checkpoint))
    }

    #[tracing::instrument(skip_all)]
    pub async fn save<C: Checkpoint>(
        &mut self,
        checkpoint: C,
        metrics: Arc<NockAppMetrics>,
    ) -> Result<(), CheckpointError> {
        let event_num = checkpoint.event_num();
        trace!("Saving checkpoint at event_num {}", event_num);
        let saveable = checkpoint.to_saveable();
        trace!("Converted checkpoint to saveable");
        let jammed = saveable.to_jammed_checkpoint::<J>(metrics);
        trace!("Converted saveable to jammed");
        self.save_jammed_checkpoint(jammed).await?;

        Ok(())
    }
}

/// This trait decouples the serf's capture of the current kernel state from the
/// snapshotting process.
pub trait Checkpoint: Sized {
    fn to_saveable(self) -> SaveableCheckpoint;
    fn event_num(&self) -> u64;
    fn from_saveable(saveable: SaveableCheckpoint) -> Result<Self, CheckpointError>;
}

#[derive(Debug, Clone)]
pub struct SaveableCheckpoint {
    pub ker_hash: Hash,
    pub event_num: u64,
    pub state: NounSlab,
    pub cold: NounSlab,
}

impl SaveableCheckpoint {
    #[tracing::instrument(skip(self, metrics))]
    fn to_jammed_checkpoint<J: Jammer>(self, metrics: Arc<NockAppMetrics>) -> JammedCheckpointV2 {
        let SaveableCheckpoint {
            ker_hash,
            event_num,
            state,
            cold,
        } = self;

        let jam_start = Instant::now();
        let state_jam = JammedNoun::new(state.coerce_jammer::<J>().jam());
        let cold_jam = JammedNoun::new(cold.coerce_jammer::<J>().jam());
        metrics.save_jam_time.add_timing(&jam_start.elapsed());

        JammedCheckpointV2::new(ker_hash, event_num, cold_jam, state_jam)
    }

    fn from_jammed_checkpoint_v1<'a, J: Jammer>(
        jammed: JammedCheckpointV1,
        metrics: Option<Arc<NockAppMetrics>>,
    ) -> Result<Self, CheckpointError> {
        let mut slab: NounSlab<J> = NounSlab::new();
        let cue_start = Instant::now();
        let root = slab.cue_into(jammed.jam.0)?;
        metrics.map(|m| m.load_cue_time.add_timing(&cue_start.elapsed()));
        slab.set_root(root);
        let space = slab.noun_space();
        let cell = root
            .in_space(&space)
            .as_cell()
            .expect("legacy checkpoint root should be a cell");

        let mut state_slab: NounSlab = NounSlab::new();
        let state_copy = state_slab.copy_into(cell.head().noun(), &space);
        state_slab.set_root(state_copy);

        let mut cold_slab: NounSlab = NounSlab::new();
        let cold_copy = cold_slab.copy_into(cell.tail().noun(), &space);
        cold_slab.set_root(cold_copy);

        Ok(Self {
            ker_hash: jammed.ker_hash,
            event_num: jammed.event_num,
            state: state_slab,
            cold: cold_slab,
        })
    }

    fn from_jammed_checkpoint_v2<'a, J: Jammer>(
        jammed: JammedCheckpointV2,
        metrics: Option<Arc<NockAppMetrics>>,
    ) -> Result<Self, CheckpointError> {
        let mut durations = std::time::Duration::ZERO;

        let mut state_slab: NounSlab<J> = NounSlab::new();
        let state_start = Instant::now();
        let state_root = state_slab.cue_into(jammed.state_jam.0.clone())?;
        durations += state_start.elapsed();
        state_slab.set_root(state_root);
        let state_slab = state_slab.coerce_jammer::<NockJammer>();

        let mut cold_slab: NounSlab<J> = NounSlab::new();
        let cold_start = Instant::now();
        let cold_root = cold_slab.cue_into(jammed.cold_jam.0.clone())?;
        durations += cold_start.elapsed();
        cold_slab.set_root(cold_root);
        let cold_slab = cold_slab.coerce_jammer::<NockJammer>();

        if let Some(metrics) = metrics {
            metrics.load_cue_time.add_timing(&durations);
        }

        Ok(Self {
            ker_hash: jammed.ker_hash,
            event_num: jammed.event_num,
            state: state_slab,
            cold: cold_slab,
        })
    }
}

impl Checkpoint for SaveableCheckpoint {
    fn to_saveable(self) -> SaveableCheckpoint {
        self
    }

    fn from_saveable(saveable: SaveableCheckpoint) -> Result<Self, CheckpointError> {
        Ok(saveable)
    }

    fn event_num(&self) -> u64 {
        self.event_num
    }
}

#[derive(Error, Debug)]
pub enum CheckpointError {
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("Bincode decoding error: {0}")]
    DecodeError(#[from] bincode::error::DecodeError),
    #[error("Bincode encoding error: {0}")]
    EncodeError(#[from] bincode::error::EncodeError),
    #[error("Invalid checksum at {0}")]
    InvalidChecksum(PathBuf),
    #[error("Invalid version at {0}")]
    InvalidVersion(PathBuf),
    #[error("Sword noun error: {0}")]
    SwordNounError(#[from] nockvm::noun::Error),
    #[error("Sword cold error: {0}")]
    FromNounError(#[from] nockvm::jets::cold::FromNounError),
    #[error("Both checkpoints failed: {0}, {1}")]
    BothCheckpointsFailed(Box<CheckpointError>, Box<CheckpointError>),
    #[error("Sword interpreter error")]
    SwordInterpreterError,
    #[error("Cue error: {0}")]
    CueError(#[from] crate::noun::slab::CueError),
    #[error("Loading at version 1 failed: {v1}\\nLoading at version 0 failed: {v0}")]
    VersionsFailed {
        v1: Box<CheckpointError>,
        v0: Box<CheckpointError>,
    },
    #[error(
        "Loading at version 2 failed: {v2}\\nLoading at version 1 failed: {v1}\\nLoading at version 0 failed: {v0}"
    )]
    VersionsFailedV2 {
        v2: Box<CheckpointError>,
        v1: Box<CheckpointError>,
        v0: Box<CheckpointError>,
    },
}

pub type JammedCheckpoint = JammedCheckpointV2;

#[derive(Clone, Encode, Decode, PartialEq, Debug)]
pub struct JammedCheckpointV1 {
    /// Magic bytes to identify checkpoint format
    pub magic_bytes: u64,
    /// Version of checkpoint
    pub version: u32,
    /// Hash of the boot kernel
    #[bincode(with_serde)]
    pub ker_hash: Hash,
    /// Checksum derived from event_num and jam (the entries below)
    #[bincode(with_serde)]
    pub checksum: Hash,
    /// Checksum derived from event_num and jam (the entries below)
    #[bincode(with_serde)]
    /// Event number
    pub event_num: u64,
    /// Event number
    pub jam: JammedNoun,
}

impl JammedCheckpointV1 {
    pub fn new(ker_hash: Hash, event_num: u64, jam: JammedNoun) -> Self {
        let checksum = Self::checksum(event_num, &jam.0);
        Self {
            magic_bytes: JAM_MAGIC_BYTES,
            version: SNAPSHOT_VERSION_1,
            ker_hash,
            checksum,
            event_num,
            jam,
        }
    }

    pub fn validate(&self, path: &PathBuf) -> Result<(), CheckpointError> {
        if self.version != SNAPSHOT_VERSION_1 {
            Err(CheckpointError::InvalidVersion(path.clone()))
        } else if self.checksum != Self::checksum(self.event_num, &self.jam.0) {
            Err(CheckpointError::InvalidChecksum(path.clone()))
        } else {
            Ok(())
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        // TODO: Make this zero copy in the future
        encode_to_vec(self, config::standard())
    }

    fn checksum(event_num: u64, jam: &Bytes) -> Hash {
        let jam_len = jam.len();
        let mut hasher = Hasher::new();
        hasher.update(&event_num.to_le_bytes());
        hasher.update(&jam_len.to_le_bytes());
        hasher.update(jam);
        hasher.finalize()
    }

    #[tracing::instrument(skip_all)]
    async fn load_from_file(path: &PathBuf) -> Result<Self, CheckpointError> {
        debug!(
            "Loading jammed checkpoint from file: {}",
            path.as_os_str().to_str().unwrap()
        );
        let bytes = tokio::fs::read(path).await?;
        let config = bincode::config::standard();
        let (checkpoint, _) = bincode::decode_from_slice::<Self, Configuration>(&bytes, config)?;
        checkpoint.validate(path)?;
        Ok(checkpoint)
    }

    #[allow(dead_code)]
    #[tracing::instrument(skip(self))]
    async fn save_to_file(&self, path: &PathBuf) -> Result<(), CheckpointError> {
        let bytes = self.encode()?;
        trace!("Saving jammed checkpoint to file: {}", path.display());
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }
}

#[derive(Clone, Encode, Decode, PartialEq, Debug)]
pub struct JammedCheckpointV2 {
    /// Hash of the boot kernel
    #[bincode(with_serde)]
    pub ker_hash: Hash,
    /// Checksum derived from event_num and jam (the entries below)
    #[bincode(with_serde)]
    pub checksum: Hash,
    /// Event number
    pub event_num: u64,
    pub cold_jam: JammedNoun,
    pub state_jam: JammedNoun,
}

#[derive(Clone, Encode, Decode, PartialEq, Debug)]
struct JammedCheckpointV2Envelope {
    /// Magic bytes to identify checkpoint format
    pub magic_bytes: u64,
    pub version: u32,
    pub payload: Vec<u8>,
}

impl JammedCheckpointV2 {
    pub fn new(
        ker_hash: Hash,
        event_num: u64,
        cold_jam: JammedNoun,
        state_jam: JammedNoun,
    ) -> Self {
        let checksum = Self::checksum(event_num, &cold_jam.0, &state_jam.0);
        Self {
            ker_hash,
            checksum,
            event_num,
            cold_jam,
            state_jam,
        }
    }

    pub fn validate(&self, path: &PathBuf) -> Result<(), CheckpointError> {
        if self.checksum != Self::checksum(self.event_num, &self.cold_jam.0, &self.state_jam.0) {
            Err(CheckpointError::InvalidChecksum(path.clone()))
        } else {
            Ok(())
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        // TODO: Make this zero copy in the future
        let payload = encode_to_vec(self, config::standard())?;
        let envelope = JammedCheckpointV2Envelope {
            magic_bytes: JAM_MAGIC_BYTES,
            version: SNAPSHOT_VERSION_2,
            payload,
        };
        encode_to_vec(envelope, config::standard())
    }

    fn checksum(event_num: u64, cold_jam: &Bytes, state_jam: &Bytes) -> Hash {
        let cold_jam_len = cold_jam.len();
        let state_jam_len = state_jam.len();
        let mut hasher = Hasher::new();
        hasher.update(&event_num.to_le_bytes());
        hasher.update(&cold_jam_len.to_le_bytes());
        hasher.update(cold_jam);
        hasher.update(&state_jam_len.to_le_bytes());
        hasher.update(state_jam);
        hasher.finalize()
    }

    #[tracing::instrument(skip_all)]
    async fn load_from_file(path: &PathBuf) -> Result<Self, CheckpointError> {
        debug!(
            "Loading jammed checkpoint from file: {}",
            path.as_os_str().to_str().unwrap()
        );
        let bytes = tokio::fs::read(path).await?;
        let config = bincode::config::standard();
        let (envelope, _) = bincode::decode_from_slice::<JammedCheckpointV2Envelope, Configuration>(
            &bytes, config,
        )?;
        let checkpoint = Self::from_envelope(envelope, Some(path))?;
        checkpoint.validate(path)?;
        Ok(checkpoint)
    }

    #[tracing::instrument(skip(self))]
    async fn save_to_file(&self, path: &PathBuf) -> Result<(), CheckpointError> {
        let bytes = self.encode()?;
        trace!("Saving jammed checkpoint to file: {}", path.display());
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    fn from_envelope(
        envelope: JammedCheckpointV2Envelope,
        path: Option<&PathBuf>,
    ) -> Result<Self, CheckpointError> {
        if envelope.magic_bytes != JAM_MAGIC_BYTES {
            return Err(CheckpointError::InvalidVersion(path_or_memory(path)));
        }
        if envelope.version != LATEST_SNAPSHOT_VERSION {
            return Err(CheckpointError::InvalidVersion(path_or_memory(path)));
        }

        let config = bincode::config::standard();
        let (checkpoint, _) =
            bincode::decode_from_slice::<Self, Configuration>(&envelope.payload, config)?;

        Ok(checkpoint)
    }

    pub fn decode_from_bytes(bytes: &[u8]) -> Result<Self, CheckpointError> {
        let config = bincode::config::standard();
        let (envelope, _) =
            bincode::decode_from_slice::<JammedCheckpointV2Envelope, Configuration>(bytes, config)?;
        let checkpoint = Self::from_envelope(envelope, None)?;
        let fake_path = path_or_memory(None);
        checkpoint.validate(&fake_path)?;
        Ok(checkpoint)
    }
}

fn path_or_memory(path: Option<&PathBuf>) -> PathBuf {
    path.cloned().unwrap_or_else(|| PathBuf::from("<memory>"))
}

fn write_streaming_checkpoint(
    path: &Path,
    checkpoint: StreamingCheckpoint,
) -> Result<(), CheckpointError> {
    let start = Instant::now();
    let dir = path.parent().ok_or_else(|| {
        CheckpointError::IOError(io::Error::new(
            io::ErrorKind::Other,
            "checkpoint path missing parent directory",
        ))
    })?;
    std::fs::create_dir_all(dir)?;

    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream save start event_num={} path={}",
        checkpoint.meta.event_num,
        path.display()
    );

    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream checksum start event_num={}",
        checkpoint.meta.event_num
    );
    let checksum_start = Instant::now();
    let checksum = streaming_checksum(
        checkpoint.meta.event_num,
        checkpoint.meta.cold_bytes,
        checkpoint.cold_path(),
        checkpoint.meta.state_bytes,
        checkpoint.state_path(),
    )?;
    let checksum_elapsed = checksum_start.elapsed();
    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream checksum done event_num={} ms={}",
        checkpoint.meta.event_num,
        checksum_elapsed.as_millis()
    );

    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream payload start event_num={}",
        checkpoint.meta.event_num
    );
    let payload_start = Instant::now();
    let mut payload_tmp = Builder::new().prefix("chkjam-payload-").tempfile_in(dir)?;
    {
        let mut writer = BufWriter::new(payload_tmp.as_file_mut());
        write_streaming_payload(
            &mut writer,
            &checkpoint.meta,
            checksum,
            checkpoint.cold_path(),
            checkpoint.state_path(),
        )?;
        writer.flush()?;
    }
    let payload_len = payload_tmp.as_file().metadata()?.len();
    let payload_path = payload_tmp.into_temp_path();
    let payload_elapsed = payload_start.elapsed();
    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream payload done event_num={} bytes={} ms={}",
        checkpoint.meta.event_num,
        payload_len,
        payload_elapsed.as_millis()
    );

    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream final write start event_num={}",
        checkpoint.meta.event_num
    );
    let final_start = Instant::now();
    let mut final_tmp = Builder::new().prefix("chkjam-final-").tempfile_in(dir)?;
    {
        let mut writer = BufWriter::new(final_tmp.as_file_mut());
        write_streaming_envelope(&mut writer, payload_len, payload_path.as_ref())?;
        writer.flush()?;
    }
    final_tmp
        .persist(path)
        .map_err(|err| CheckpointError::IOError(err.error))?;
    let final_elapsed = final_start.elapsed();
    info!(
        target: "nockapp::checkpoint::stream",
        "checkpoint stream save done event_num={} ms={} total_ms={}",
        checkpoint.meta.event_num,
        final_elapsed.as_millis(),
        start.elapsed().as_millis()
    );

    Ok(())
}

fn write_streaming_payload<W: Write>(
    writer: &mut W,
    meta: &StreamingCheckpointMeta,
    checksum: Hash,
    cold_path: &Path,
    state_path: &Path,
) -> Result<(), CheckpointError> {
    let config = config::standard();
    bincode::encode_into_std_write(Compat(&meta.ker_hash), writer, config)?;
    bincode::encode_into_std_write(Compat(&checksum), writer, config)?;
    bincode::encode_into_std_write(meta.event_num, writer, config)?;
    bincode::encode_into_std_write(meta.cold_bytes as u64, writer, config)?;
    copy_file_to_writer(cold_path, writer)?;
    bincode::encode_into_std_write(meta.state_bytes as u64, writer, config)?;
    copy_file_to_writer(state_path, writer)?;
    Ok(())
}

fn write_streaming_envelope<W: Write>(
    writer: &mut W,
    payload_len: u64,
    payload_path: &Path,
) -> Result<(), CheckpointError> {
    let config = config::standard();
    bincode::encode_into_std_write(JAM_MAGIC_BYTES, writer, config)?;
    bincode::encode_into_std_write(SNAPSHOT_VERSION_2, writer, config)?;
    bincode::encode_into_std_write(payload_len, writer, config)?;
    copy_file_to_writer(payload_path, writer)?;
    Ok(())
}

fn copy_file_to_writer<W: Write>(path: &Path, writer: &mut W) -> Result<(), CheckpointError> {
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buf[..read])?;
    }
    Ok(())
}

fn streaming_checksum(
    event_num: u64,
    cold_bytes: usize,
    cold_path: &Path,
    state_bytes: usize,
    state_path: &Path,
) -> Result<Hash, CheckpointError> {
    let mut hasher = Hasher::new();
    hasher.update(&event_num.to_le_bytes());
    hasher.update(&cold_bytes.to_le_bytes());
    update_hasher_from_file(&mut hasher, cold_path)?;
    hasher.update(&state_bytes.to_le_bytes());
    update_hasher_from_file(&mut hasher, state_path)?;
    Ok(hasher.finalize())
}

fn update_hasher_from_file(hasher: &mut Hasher, path: &Path) -> Result<(), CheckpointError> {
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(())
}

#[derive(Clone, Debug)]
enum LoadedCheckpoint {
    V2(JammedCheckpointV2),
    V1(JammedCheckpointV1),
    V0(JammedCheckpointV0),
}

impl LoadedCheckpoint {
    fn event_num(&self) -> u64 {
        match self {
            LoadedCheckpoint::V2(cp) => cp.event_num,
            LoadedCheckpoint::V1(cp) => cp.event_num,
            LoadedCheckpoint::V0(cp) => cp.event_num,
        }
    }

    fn checksum(&self) -> Hash {
        match self {
            LoadedCheckpoint::V2(cp) => cp.checksum,
            LoadedCheckpoint::V1(cp) => cp.checksum,
            LoadedCheckpoint::V0(cp) => cp.checksum,
        }
    }

    fn into_saveable<J: Jammer>(
        self,
        metrics: Option<Arc<NockAppMetrics>>,
    ) -> Result<SaveableCheckpoint, CheckpointError> {
        match self {
            LoadedCheckpoint::V2(cp) => {
                SaveableCheckpoint::from_jammed_checkpoint_v2::<J>(cp, metrics)
            }
            LoadedCheckpoint::V1(cp) => {
                SaveableCheckpoint::from_jammed_checkpoint_v1::<J>(cp, metrics)
            }
            LoadedCheckpoint::V0(cp) => SaveableCheckpoint::from_jammed_checkpoint_v2::<J>(
                JammedCheckpoint::from(cp),
                metrics,
            ),
        }
    }
}

async fn inspect_latest(
    path: &PathBuf,
) -> Result<
    (
        SaverState,
        Option<LoadedCheckpoint>,
        Option<CheckpointSummary>,
    ),
    CheckpointError,
> {
    let path_0 = path.join("0.chkjam");
    let path_1 = path.join("1.chkjam");

    if !path_0.exists() && !path_1.exists() {
        create_dir_all(path).await?;
        return Ok((
            SaverState {
                save_to_next: WhichSnapshot::Snapshot0,
                last_event_num: 0,
            },
            None,
            None,
        ));
    }

    let checkpoint_0 = load_checkpoint_file(&path_0).await;
    let checkpoint_1 = load_checkpoint_file(&path_1).await;

    let (loaded_checkpoint, save_to_next, selected_path) = match (checkpoint_0, checkpoint_1) {
        (Ok(c0), Ok(c1)) => {
            if c0.event_num() > c1.event_num() {
                debug!(
                    "Loading checkpoint at: {}, checksum: {}",
                    path_0.display(),
                    c0.checksum()
                );
                (c0, WhichSnapshot::Snapshot1, path_0)
            } else {
                debug!(
                    "Loading checkpoint at: {}, checksum: {}",
                    path_1.display(),
                    c1.checksum()
                );
                (c1, WhichSnapshot::Snapshot0, path_1)
            }
        }
        (Ok(c0), Err(e1)) => {
            warn!("checkpoint at {} failed to load: {}", path_1.display(), e1);
            debug!(
                "Loading checkpoint at: {}, checksum: {}",
                path_0.display(),
                c0.checksum()
            );
            (c0, WhichSnapshot::Snapshot1, path_0)
        }
        (Err(e0), Ok(c1)) => {
            warn!("checkpoint at {} failed to load: {}", path_0.display(), e0);
            debug!(
                "Loading checkpoint at: {}, checksum: {}",
                path_1.display(),
                c1.checksum()
            );
            (c1, WhichSnapshot::Snapshot0, path_1)
        }
        (Err(e0), Err(e1)) => {
            error!("checkpoint at {} failed to load: {}", path_0.display(), e0);
            error!("checkpoint at {} failed to load: {}", path_1.display(), e1);
            return Err(CheckpointError::BothCheckpointsFailed(
                Box::new(e0),
                Box::new(e1),
            ));
        }
    };
    let summary = CheckpointSummary {
        path: selected_path,
        event_num: loaded_checkpoint.event_num(),
        checksum: loaded_checkpoint.checksum(),
    };
    Ok((
        SaverState {
            save_to_next,
            last_event_num: summary.event_num,
        },
        Some(loaded_checkpoint),
        Some(summary),
    ))
}

async fn load_checkpoint_file(path: &PathBuf) -> Result<LoadedCheckpoint, CheckpointError> {
    match JammedCheckpointV2::load_from_file(path).await {
        Ok(cp) => Ok(LoadedCheckpoint::V2(cp)),
        Err(e_v2) => match JammedCheckpointV1::load_from_file(path).await {
            Ok(cp) => Ok(LoadedCheckpoint::V1(cp)),
            Err(e_v1) => match JammedCheckpointV0::load_from_file(path).await {
                Ok(cp0) => Ok(LoadedCheckpoint::V0(cp0)),
                Err(e_v0) => Err(CheckpointError::VersionsFailedV2 {
                    v2: Box::new(e_v2),
                    v1: Box::new(e_v1),
                    v0: Box::new(e_v0),
                }),
            },
        },
    }
}

impl From<JammedCheckpointV0> for JammedCheckpoint {
    fn from(v0: JammedCheckpointV0) -> Self {
        let v1 = JammedCheckpointV1 {
            magic_bytes: v0.magic_bytes,
            version: SNAPSHOT_VERSION_1,
            ker_hash: v0.ker_hash,
            checksum: v0.checksum,
            event_num: v0.event_num,
            jam: v0.jam,
        };

        let mut slab: NounSlab = NounSlab::new();
        let root = slab
            .cue_into(v1.jam.0.clone())
            .expect("legacy checkpoint jam should cue");
        slab.set_root(root);
        let space = slab.noun_space();
        let cell = root
            .in_space(&space)
            .as_cell()
            .expect("legacy checkpoint root should be a cell");

        let mut state_slab: NounSlab = NounSlab::new();
        let state_copy = state_slab.copy_into(cell.head().noun(), &space);
        state_slab.set_root(state_copy);
        let state_jam = JammedNoun::new(state_slab.jam());

        let mut cold_slab: NounSlab = NounSlab::new();
        let cold_copy = cold_slab.copy_into(cell.tail().noun(), &space);
        cold_slab.set_root(cold_copy);
        let cold_jam = JammedNoun::new(cold_slab.jam());

        JammedCheckpointV2::new(v1.ker_hash, v1.event_num, cold_jam, state_jam)
    }
}

#[derive(Clone, Encode, Decode, PartialEq, Debug)]
pub struct JammedCheckpointV0 {
    /// Magic bytes to identify checkpoint format
    pub magic_bytes: u64,
    /// Version of checkpoint
    pub version: u32,
    /// The buffer this checkpoint was saved to, either 0 or 1
    pub buff_index: bool,
    /// Hash of the boot kernel
    #[bincode(with_serde)]
    pub ker_hash: Hash,
    /// Checksum derived from event_num and jam (the entries below)
    #[bincode(with_serde)]
    pub checksum: Hash,
    /// Event number
    pub event_num: u64,
    /// Jammed noun of [kernel_state cold_state]
    pub jam: JammedNoun,
}

impl JammedCheckpointV0 {
    pub fn new(buff_index: bool, ker_hash: Hash, event_num: u64, jam: JammedNoun) -> Self {
        let checksum = Self::checksum(event_num, &jam.0);
        Self {
            magic_bytes: JAM_MAGIC_BYTES,
            version: SNAPSHOT_VERSION_0,
            buff_index,
            ker_hash,
            checksum,
            event_num,
            jam,
        }
    }

    pub fn validate(&self, path: &PathBuf) -> Result<(), CheckpointError> {
        if self.version != SNAPSHOT_VERSION_0 {
            Err(CheckpointError::InvalidVersion(path.clone()))
        } else if self.checksum != Self::checksum(self.event_num, &self.jam.0) {
            Err(CheckpointError::InvalidChecksum(path.clone()))
        } else {
            Ok(())
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        // TODO: Make this zero copy in the future
        encode_to_vec(self, config::standard())
    }

    fn checksum(event_num: u64, jam: &Bytes) -> Hash {
        let jam_len = jam.len();
        let mut hasher = Hasher::new();
        hasher.update(&event_num.to_le_bytes());
        hasher.update(&jam_len.to_le_bytes());
        hasher.update(jam);
        hasher.finalize()
    }

    #[tracing::instrument(skip_all)]
    async fn load_from_file(path: &PathBuf) -> Result<Self, CheckpointError> {
        debug!(
            "Loading jammed checkpoint from file: {}",
            path.as_os_str().to_str().unwrap()
        );
        let bytes = tokio::fs::read(path).await?;
        let config = bincode::config::standard();
        let (checkpoint, _) = bincode::decode_from_slice::<Self, Configuration>(&bytes, config)?;
        checkpoint.validate(path)?;
        Ok(checkpoint)
    }

    #[tracing::instrument(skip(self))]
    #[allow(dead_code)] // Preserving this for posterity
    async fn save_to_file(&self, path: &PathBuf) -> Result<(), CheckpointError> {
        let bytes = self.encode()?;
        trace!("Saving jammed checkpoint to file: {}", path.display());
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }
}

#[cfg(test)]
mod version_tests {
    use blake3::hash;
    use nockvm::noun::{Noun, NounSpace, D, T};
    use tempfile::TempDir;

    use super::*;
    use crate::test_support::TestArena;

    fn legacy_pair_jam(state_value: u64, cold_value: u64) -> JammedNoun {
        let mut slab = NounSlab::<NockJammer>::new();
        let space = NounSpace::empty();
        let state = slab.copy_into(D(state_value), &space);
        let cold = slab.copy_into(D(cold_value), &space);
        let root = T(&mut slab, &[state, cold]);
        slab.set_root(root);
        JammedNoun::new(slab.coerce_jammer::<NockJammer>().jam())
    }

    fn atom_value(noun: Noun, space: &NounSpace) -> u64 {
        noun.in_space(space)
            .as_atom()
            .expect("expected atom")
            .as_u64()
            .expect("expected atom to fit in u64")
    }

    #[tokio::test(flavor = "current_thread")]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    async fn loads_v1_checkpoint_via_saver() {
        let temp = TempDir::new().expect("create temp dir");
        let _test_arena = TestArena::default();
        let state_value = 5;
        let cold_value = 9;
        let legacy_jam = legacy_pair_jam(state_value, cold_value);
        let ker_hash = hash(b"legacy-v1");
        let checkpoint = JammedCheckpointV1::new(ker_hash, 7, legacy_jam.clone());
        let bytes = checkpoint.encode().expect("encode v1 checkpoint");
        std::fs::write(temp.path().join("0.chkjam"), bytes).expect("write checkpoint");

        let (_, maybe_saveable) =
            Saver::<NockJammer>::try_load::<SaveableCheckpoint>(&temp.path().to_path_buf(), None)
                .await
                .expect("load checkpoint");

        let saveable = maybe_saveable.expect("expected a checkpoint");
        assert_eq!(saveable.ker_hash, ker_hash);
        assert_eq!(saveable.event_num, 7);

        let state_space = saveable.state.noun_space();
        let cold_space = saveable.cold.noun_space();
        let loaded_state = atom_value(unsafe { *saveable.state.root() }, &state_space);
        let loaded_cold = atom_value(unsafe { *saveable.cold.root() }, &cold_space);
        assert_eq!(loaded_state, state_value);
        assert_eq!(loaded_cold, cold_value);
    }

    #[tokio::test(flavor = "current_thread")]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    async fn loads_v0_checkpoint_via_saver() {
        let temp = TempDir::new().expect("create temp dir");
        let _test_arena = TestArena::default();
        let state_value = 11;
        let cold_value = 22;
        let legacy_jam = legacy_pair_jam(state_value, cold_value);
        let ker_hash = hash(b"legacy-v0");
        let checkpoint = JammedCheckpointV0::new(false, ker_hash, 3, legacy_jam.clone());
        let bytes = checkpoint.encode().expect("encode v0 checkpoint");
        std::fs::write(temp.path().join("0.chkjam"), bytes).expect("write checkpoint");

        let (_, maybe_saveable) =
            Saver::<NockJammer>::try_load::<SaveableCheckpoint>(&temp.path().to_path_buf(), None)
                .await
                .expect("load checkpoint");

        let saveable = maybe_saveable.expect("expected a checkpoint");
        assert_eq!(saveable.ker_hash, ker_hash);
        assert_eq!(saveable.event_num, 3);

        let state_space = saveable.state.noun_space();
        let cold_space = saveable.cold.noun_space();
        let loaded_state = atom_value(unsafe { *saveable.state.root() }, &state_space);
        let loaded_cold = atom_value(unsafe { *saveable.cold.root() }, &cold_space);
        assert_eq!(loaded_state, state_value);
        assert_eq!(loaded_cold, cold_value);
    }
}

/*
// We need to figure out how to do this with quickcheck instead of a golden master jam
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_jammed_checkpoint_header() {
        let chk_header = std::path::PathBuf::from("../../../jammed_checkpoint_header.jam");
        let mut chk_header_bytes = std::fs::read(chk_header).unwrap();
        let result: (JammedCheckpoint, usize) =
            bincode::decode_from_slice(&mut chk_header_bytes, bincode::config::standard()).unwrap();
        let jammed_checkpoint = result.0;
        println!("jammed_checkpoint: {:?}", jammed_checkpoint);
        assert_eq!(jammed_checkpoint.magic_bytes, JAM_MAGIC_BYTES);
        assert_eq!(jammed_checkpoint.version, SNAPSHOT_VERSION);
        assert_eq!(jammed_checkpoint.buff_index, true);
    }
}
*/
