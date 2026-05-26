//! NockApp-compatible mining driver.
//!
//! `create_driver(cfg) -> IODriverFn`. Drop-in replacement for the
//! current Equix-based `crates/nockchain/src/mining.rs::
//! create_mining_driver` at the driver-registration site.
//!
//! v1 takes mining jobs through an in-memory `tokio::sync::mpsc`
//! channel (`DriverConfig::jobs_rx`) rather than decoding nouns
//! from `handle.effect_receiver`. This sidesteps the chain-Hoon
//! candidate-effect format for the first cut — the kernel-side
//! noun shape is captured in the spec, and a follow-up commit
//! wires the effect-channel listener that translates the Hoon
//! `[%candidate ...]` payload into an `OwnedMiningJob`. The
//! driver still uses [`NockAppHandle`] for the outbound
//! `[%mined ...]` poke so the integration surface with the kernel
//! is real.

use std::sync::Arc;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use nockapp::nockapp::driver::{make_driver, IODriverFn};
use nockapp::nockapp::wire::{Wire, WireRepr};
use nockapp::noun::slab::NounSlab;
use nockvm::noun::{Noun, D, T};
use nockvm_macros::tas;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use crate::mining;
use crate::{
    DifficultyTarget, MineOptions, MinedSolution, MiningCancel, MiningError, MiningJob,
    NonceAnchors,
};

// ───────────────────────────── wire vocabulary ─────────────────────────────

/// Wire enum mirroring the existing `MiningWire` shape in
/// `crates/nockchain/src/mining.rs` so the kernel-side Hoon can
/// re-use the established poke vocabulary.
pub enum AiPowMinerWire {
    /// Kernel → driver: enable/disable mining.
    Enable,
    /// Kernel → driver: new puzzle. (V1 takes jobs via in-memory
    /// channel; this variant is the eventual home for the noun
    /// candidate the kernel will emit.)
    Candidate,
    /// Driver → kernel: a solved tile. Payload (v1):
    /// `[%mined nonce-as-atom found-idx-as-atom]`.
    Mined,
    /// Driver → kernel: mining errored or terminated without a
    /// solution. Payload: `[%mining-error message-as-atom]`.
    MiningError,
}

impl AiPowMinerWire {
    pub fn label(&self) -> &'static str {
        match self {
            AiPowMinerWire::Enable => "enable",
            AiPowMinerWire::Candidate => "candidate",
            AiPowMinerWire::Mined => "mined",
            AiPowMinerWire::MiningError => "mining-error",
        }
    }
}

impl Wire for AiPowMinerWire {
    const VERSION: u64 = 1;
    const SOURCE: &'static str = "ai-pow-miner";

    fn to_wire(&self) -> WireRepr {
        let tags = vec![self.label().into()];
        WireRepr::new(AiPowMinerWire::SOURCE, AiPowMinerWire::VERSION, tags)
    }
}

// ─────────────────────────── driver config + jobs ──────────────────────────

/// Owning form of [`MiningJob`] — the driver receives it across
/// channels, so the matrices need owned storage.
pub struct OwnedMiningJob {
    pub puzzle_id: Vec<u8>,
    pub anchors: NonceAnchors,
    pub params: MatmulParams,
    pub target: DifficultyTarget,
    pub a: Arc<Vec<i8>>,
    pub b: Arc<Vec<i8>>,
    pub opts: MineOptions,
}

/// Events the driver emits during its lifetime. Useful for tests
/// + observability dashboards.
#[derive(Debug)]
pub enum DriverEvent {
    /// Driver entered its select loop.
    Started,
    /// A job was received and the spawn-blocking mining task started.
    JobStarted,
    /// A solution was produced + poked to the kernel.
    Mined {
        nonce: [u8; crate::NCMN_NONCE_LEN],
        found_idx: u32,
        extranonces_tried: u64,
        elapsed: Duration,
    },
    /// Mining terminated without a solution.
    MiningError(String),
    /// Driver loop exited (jobs channel closed).
    Stopped,
}

/// Driver wiring + behavior.
pub struct DriverConfig {
    /// Mining gate. `false` ⇒ driver still starts but ignores any
    /// incoming jobs (mirrors the existing `mine: bool` toggle).
    pub mining_enabled: bool,
    /// Inbound jobs. v1 source. Wrapped in `Mutex` so the closure
    /// satisfies `IODriverFn`'s `FnOnce + Send + 'static`.
    pub jobs_rx: Arc<Mutex<mpsc::Receiver<OwnedMiningJob>>>,
    /// Optional driver-event tap (test bins + dashboards subscribe).
    pub events_tx: Option<mpsc::Sender<DriverEvent>>,
    /// Optional one-shot signal that the driver has finished
    /// startup (mirrors the existing `init_complete_tx` pattern).
    pub init_complete_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

// ──────────────────────────────── driver entry ─────────────────────────────

/// Construct the [`IODriverFn`] to register with a [`nockapp::NockApp`].
pub fn create_driver(cfg: DriverConfig) -> IODriverFn {
    make_driver(move |handle| async move {
        let DriverConfig {
            mining_enabled,
            jobs_rx,
            events_tx,
            init_complete_tx,
        } = cfg;

        if let Some(tx) = init_complete_tx {
            let _ = tx.send(());
        }
        emit(&events_tx, DriverEvent::Started).await;

        if !mining_enabled {
            // Mining disabled: drain jobs but don't act on them.
            // Still exits cleanly when the channel closes.
            let mut rx = jobs_rx.lock().await;
            while rx.recv().await.is_some() { /* drop */ }
            emit(&events_tx, DriverEvent::Stopped).await;
            return Ok(());
        }

        let mut rx = jobs_rx.lock().await;
        loop {
            tokio::select! {
                maybe_job = rx.recv() => {
                    let Some(job) = maybe_job else {
                        emit(&events_tx, DriverEvent::Stopped).await;
                        return Ok(());
                    };
                    emit(&events_tx, DriverEvent::JobStarted).await;
                    let cancel = MiningCancel::new();
                    // mining::run is sync (heavy CPU + BLAKE3); run on
                    // the blocking pool so it can't stall the tokio
                    // reactor.
                    let result = tokio::task::spawn_blocking(move || run_owned(job, cancel))
                        .await
                        .map_err(|e| nockapp::nockapp::NockAppError::OtherError(
                            format!("ai-pow-miner: blocking task join error: {e}")
                        ))?;
                    match result {
                        Ok(sol) => {
                            let elapsed = sol.stats.elapsed;
                            let attempts = sol.stats.extranonces_tried;
                            let nonce = sol.nonce;
                            let found_idx = sol.found_idx;
                            poke_mined(&handle, &nonce, found_idx).await?;
                            emit(&events_tx, DriverEvent::Mined {
                                nonce, found_idx, extranonces_tried: attempts, elapsed,
                            }).await;
                        }
                        Err(e) => {
                            let msg = format!("{e}");
                            poke_mining_error(&handle, &msg).await?;
                            emit(&events_tx, DriverEvent::MiningError(msg)).await;
                        }
                    }
                }
            }
        }
    })
}

/// Helper to run mining::run on an owned job.
fn run_owned(job: OwnedMiningJob, cancel: MiningCancel) -> Result<MinedSolution, MiningError> {
    let borrow = MiningJob {
        puzzle_id: &job.puzzle_id,
        anchors: job.anchors,
        params: &job.params,
        target: job.target,
        a: &job.a,
        b: &job.b,
    };
    mining::run(&borrow, &job.opts, cancel)
}

async fn emit(tx: &Option<mpsc::Sender<DriverEvent>>, ev: DriverEvent) {
    if let Some(t) = tx {
        let _ = t.send(ev).await;
    }
}

// ──────────────────────── outbound noun construction ───────────────────────

async fn poke_mined(
    handle: &nockapp::nockapp::driver::NockAppHandle,
    nonce: &[u8; crate::NCMN_NONCE_LEN],
    found_idx: u32,
) -> Result<(), nockapp::nockapp::NockAppError> {
    let mut slab = NounSlab::new();
    let nonce_atom = bytes_to_atom(&mut slab, nonce);
    let head = D(tas!(b"mined"));
    let payload: Noun = T(&mut slab, &[head, nonce_atom, D(found_idx as u64)]);
    slab.set_root(payload);
    handle.poke(AiPowMinerWire::Mined.to_wire(), slab).await?;
    Ok(())
}

async fn poke_mining_error(
    handle: &nockapp::nockapp::driver::NockAppHandle,
    message: &str,
) -> Result<(), nockapp::nockapp::NockAppError> {
    let mut slab = NounSlab::new();
    let msg_atom = bytes_to_atom(&mut slab, message.as_bytes());
    // `tas!()` packs to a u64 (≤ 8 chars); "mining-error" is 12,
    // so build the head as an indirect atom from the byte string.
    let head = bytes_to_atom(&mut slab, b"mining-error");
    let payload: Noun = T(&mut slab, &[head, msg_atom]);
    slab.set_root(payload);
    handle.poke(AiPowMinerWire::MiningError.to_wire(), slab).await?;
    Ok(())
}

fn bytes_to_atom(slab: &mut NounSlab, bytes: &[u8]) -> Noun {
    use nockvm::noun::IndirectAtom;
    let atom = <IndirectAtom as nockapp::IndirectAtomExt>::from_bytes(slab, bytes);
    atom.as_noun()
}
