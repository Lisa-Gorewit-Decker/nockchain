//! Out-of-process node-connecting run loop for the AI-PoW miner.
//!
//! Mirrors `zk-pow-miner::run` in shape: the miner runs as a separate
//! OS process and talks to a `nockchain` node over the node's private
//! [`nockapp_grpc`] `NockAppService`. The substrate (connect /
//! `set-mining-key` / `enable-mining` / `WatchEffects` / `submit`)
//! is shared via [`nockchain_mining_common::NodeClient`]; only the
//! puzzle-specific bits (the AI-PoW matmul prover, the NCMN nonce
//! shape, the `AiPowMinerWire` submission wire) live here.
//!
//! ## Lifecycle
//! 1. Build the [`MinerConfig`] with the AI-puzzle parameters
//!    (puzzle id, matmul shape, matrices) baked in.
//! 2. (re)connect to the node with backoff.
//! 3. `set_mining_key` → `watch_candidates` → `enable_mining(true)`
//!    (subscribe before enable to avoid the candidate-emit race).
//! 4. Inner loop (single worker for v1):
//!    - shutdown → cancel current attempt + best-effort
//!      `enable_mining(false)` + exit.
//!    - new candidate → cancel any in-flight attempt, derive the
//!      mining job, spawn-blocking [`crate::mining::run`].
//!    - worker result:
//!      - success → build the canonical recursive certificate via the
//!        configured certificate builder once the proof is sound for
//!        production, and poke the node with
//!        `[%command %pow %ai-pow nonce cert]` on [`AiPowMinerWire::Mined`],
//!        then idle until the next candidate.
//!      - error → log + idle.
//! 5. Stream drop → outer loop reconnects.
//!
//! ## Note on submission
//! The canonical payload shape is the recursive AI-PoW certificate noun. The
//! plain `MatmulProof`, nonce, and tile index are mining internals; they are
//! not submitted to the kernel as the block proof. The current certificate
//! support gate fails preflight for multi-tile shapes until the recursive
//! statement binds a full-matrix aggregate, and the kernel remains fail-closed
//! until recursive certificate verification is wired. Pearl merge-mined
//! submissions use the separate `%ai-pmp` helper in this module, but the run
//! loop remains native `%ai-pow` until the Pearl ticket prover and consensus
//! verifier are wired end to end.

use std::sync::Arc;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use ai_pow::pearl_compat::PearlMergeTicketAttempt;
use ai_pow::prover::ProverOptions;
use futures::StreamExt;
use nockapp::nockapp::wire::Wire;
use nockapp::noun::slab::NounSlab;
use nockchain_mining_common::{MiningCandidate, MiningKeyConfig, MiningPkhConfig, NodeClient};
use nockvm::noun::{NounAllocator, D, T};
use nockvm_macros::tas;
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::certificate_noun::{
    build_ai_pow_pearl_merge_artifact_noun_from_ticket,
    build_ai_pow_pearl_merge_artifact_noun_from_ticket_node,
    decode_ai_pow_pearl_merge_artifact_slab, AiProofNode, CertificateNounError,
    CertificateNounLimits,
};
use crate::wire::AiPowMinerWire;
use crate::{
    mining, parse_ncmn_nonce, DifficultyTarget, MineOptions, MinedSolution, MiningCancel,
    MiningError as PuzzleMiningError, MiningJob, NcmnNonce, NonceAnchors, NonceFormatError,
};

const MAX_CHAIN_TARGET_U32_LIMBS: usize = 10;

pub type AiPowCertificateBuilder =
    dyn Fn(&MinedSolution) -> Result<NounSlab, AiPowCertificateBuildError> + Send + Sync + 'static;

/// Which canonical block proof arm this miner configuration is allowed to
/// submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPowSubmissionMode {
    /// Native Nockchain AI-PoW: mine NCMN nonce-bound attempts and submit
    /// `[%command %pow %ai-pow nonce cert]`.
    NativeNcmn,
    /// Pearl-compatible merge-mined AI-PoW: mine Pearl ticket attempts and
    /// submit `[%command %pow %ai-pmp artifact]`.
    ///
    /// The public helper APIs for constructing `%ai-pmp` artifacts exist, but
    /// the main node run loop remains fail-closed in this mode until it has a
    /// Pearl candidate/job source and recursive Pearl statement prover.
    PearlMerge,
}

#[derive(Debug, Error)]
#[error("AI-PoW recursive certificate build failed: {0}")]
pub struct AiPowCertificateBuildError(pub String);

#[derive(Debug, Error)]
pub enum AiPowCertificatePokeError {
    #[error("NCMN nonce: {0}")]
    Nonce(#[from] NonceFormatError),
    #[error("NCMN external commitment is reserved and must be absent")]
    NonceExternalCommitmentPresent,
    #[error("Pearl merge AI-PoW artifact: {0}")]
    PearlMergeArtifact(#[from] CertificateNounError),
}

impl From<String> for AiPowCertificateBuildError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AiPowCertificateBuildError {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Default v0 mining key — pass-through for the kernel's v0 pubkey
/// infrastructure (matches `zk-pow-miner`'s default).
pub const DEFAULT_V0_PUBKEY: &str = "2cPnE4Z9RevhTv9is9Hmc1amFubEFbUxzCV2Fxb9GxevJstV5VG92oYt6Sai3d3NjLFcsuVXSLx9hikMbD1agv9M267TVw3hV9MCpMfEnGo5LYtjJ7jPyHg8SERPjJRCWTgZ";

/// Build the default v0 `MiningKeyConfig` list (single
/// `[share=1 m=1 keys=[DEFAULT_V0_PUBKEY]]` entry).
pub fn default_v0_configs() -> Vec<MiningKeyConfig> {
    vec![MiningKeyConfig {
        share: 1,
        m: 1,
        keys: vec![DEFAULT_V0_PUBKEY.to_string()],
    }]
}

/// AI puzzle inputs — the local-state portion of the mining config
/// that isn't carried in the chain's `%mine` effect (the chain only
/// supplies header + target + pow-len).
///
/// These come from operator config (CLI / config file). In a future
/// chain-AI integration these may be derived from chain state (e.g.
/// layer/epoch); the substrate is structured so that a follow-up can
/// swap the derivation in without changing the run loop.
#[derive(Clone)]
pub struct AiPuzzleInputs {
    /// Stable puzzle identity bound into `κ` (`ai_pow::BlockContext`'s
    /// `block_commitment` argument). Convention:
    /// `BLAKE3("ai-pow-puzzle-id-v1" ‖ layer_id ‖ epoch_id ‖ params_tag)`.
    pub puzzle_id: Vec<u8>,
    pub params: MatmulParams,
    /// Reference matmul inputs. `Arc` so the spawn-blocking worker can
    /// hold a cheap clone without copying the bytes.
    pub a: Arc<Vec<i8>>,
    pub b: Arc<Vec<i8>>,
    /// Forwarded to the per-attempt prover (mostly defaults).
    pub prover_opts: ProverOptions,
    /// Production handoff that turns a winning mining attempt into a
    /// structured recursive certificate noun. If absent, or if the current
    /// recursive proof lacks the full production binding, the production node
    /// miner fails preflight before enabling mining.
    pub certificate_builder: Option<Arc<AiPowCertificateBuilder>>,
    /// The block proof arm this run-loop configuration may submit.
    pub submission_mode: AiPowSubmissionMode,
}

impl AiPuzzleInputs {
    /// Production node-mining preflight: do not spend matmul work unless the
    /// configured puzzle can be converted into the canonical recursive
    /// certificate accepted at the block boundary.
    pub fn validate_canonical_submission_ready(&self) -> Result<(), MinerError> {
        if self.submission_mode == AiPowSubmissionMode::PearlMerge {
            return Err(MinerError::CanonicalCertificateUnavailable(
                "Pearl merge-mining mode is explicit but the node run loop is not wired to mine Pearl ticket attempts yet; use the %ai-pmp ticket-derived submission helpers only after a Pearl ticket/prover path succeeds".to_string(),
            ));
        }
        if self.certificate_builder.is_none() {
            return Err(MinerError::CanonicalCertificateUnavailable(
                "no recursive certificate builder configured".to_string(),
            ));
        }
        ai_pow::zk_bridge::validate_canonical_recursive_certificate_params(&self.params).map_err(
            |e| {
                MinerError::CanonicalCertificateUnavailable(format!(
                    "configured AI-PoW params cannot produce a canonical full-matmul recursive certificate: {e}"
                ))
            },
        )
    }
}

#[derive(Clone)]
pub struct MinerConfig {
    /// Node's private gRPC URL, e.g. `http://127.0.0.1:5555`.
    pub node_addr: String,
    /// v0 (pubkey) reward configs. Default: hard-coded pass-through key.
    pub mining_configs: Vec<MiningKeyConfig>,
    /// v1 (pubkey-hash) reward configs.
    pub mining_pkh_configs: Vec<MiningPkhConfig>,
    /// AI puzzle local-state inputs (matrices, params, puzzle id).
    pub puzzle: AiPuzzleInputs,
    /// Per-attempt mining-loop tuning (extranonce_start, deadline,
    /// max_extranonces, progress interval). The `prover` field is
    /// derived from `puzzle.prover_opts` so callers don't have to
    /// duplicate it.
    pub mine_opts: MineOptions,
    pub reconnect_backoff_initial: Duration,
    pub reconnect_backoff_max: Duration,
    pub reconnect_max_attempts: u32,
}

impl MinerConfig {
    /// Convenience builder for the common case: single mining-pkh,
    /// default v0 key, default backoff (1s→30s, 5 retries).
    pub fn new(
        node_addr: String,
        mining_pkh_configs: Vec<MiningPkhConfig>,
        puzzle: AiPuzzleInputs,
    ) -> Self {
        let mut mine_opts = MineOptions::default();
        mine_opts.prover = puzzle.prover_opts;
        Self {
            node_addr,
            mining_configs: default_v0_configs(),
            mining_pkh_configs,
            puzzle,
            mine_opts,
            reconnect_backoff_initial: Duration::from_secs(1),
            reconnect_backoff_max: Duration::from_secs(30),
            reconnect_max_attempts: 5,
        }
    }
}

#[derive(Debug, Error)]
pub enum MinerError {
    #[error("kernel configuration failed: {0}")]
    Configure(String),
    #[error("gave up after {count} consecutive connect attempts")]
    TooManyReconnects { count: u32 },
    #[error("candidate decode failed: {0}")]
    CandidateDecode(String),
    #[error("worker join failed: {0}")]
    WorkerJoin(String),
    #[error("{0}")]
    CertificateBuild(String),
    #[error("{0}")]
    CanonicalCertificateUnavailable(String),
}

/// Production entry point. Returns `Ok(())` on clean shutdown, `Err` on
/// unrecoverable failure.
pub async fn run(cfg: MinerConfig, shutdown: CancellationToken) -> Result<(), MinerError> {
    cfg.puzzle.validate_canonical_submission_ready()?;
    info!(
        node = %cfg.node_addr,
        puzzle_id_len = cfg.puzzle.puzzle_id.len(),
        params = ?cfg.puzzle.params,
        submission_mode = ?cfg.puzzle.submission_mode,
        "ai-pow-miner: entering main loop"
    );

    let mut consecutive_failures: u32 = 0;
    let mut backoff = cfg.reconnect_backoff_initial;

    loop {
        if shutdown.is_cancelled() {
            break;
        }

        // ── (re)connect ──
        let mut client = match NodeClient::connect(&cfg.node_addr).await {
            Ok(c) => {
                consecutive_failures = 0;
                backoff = cfg.reconnect_backoff_initial;
                c
            }
            Err(e) => {
                consecutive_failures += 1;
                warn!(
                    attempt = consecutive_failures,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %e,
                    "connect failed; backing off"
                );
                if consecutive_failures >= cfg.reconnect_max_attempts {
                    return Err(MinerError::TooManyReconnects {
                        count: consecutive_failures,
                    });
                }
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(cfg.reconnect_backoff_max);
                continue;
            }
        };

        // ── configure ──
        // Order matters: subscribe BEFORE enable_mining so the initial
        // candidate (which the post-poke update-candidate-block emits)
        // lands on a live stream.
        if let Err(e) = client
            .set_mining_key(
                AiPowMinerWire::SetPubKey.to_wire(),
                cfg.mining_configs.clone(),
                cfg.mining_pkh_configs.clone(),
            )
            .await
        {
            return Err(MinerError::Configure(format!("set_mining_key: {e}")));
        }
        let mut candidates = match client.watch_candidates(vec![b"mine-ai".to_vec()]).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "watch_candidates failed; reconnect");
                consecutive_failures += 1;
                continue;
            }
        };
        if let Err(e) = client
            .enable_mining(AiPowMinerWire::Enable.to_wire(), true)
            .await
        {
            return Err(MinerError::Configure(format!("enable_mining(true): {e}")));
        }
        info!("ai-pow-miner: subscribed + mining enabled; awaiting candidates");

        // ── inner loop ──
        // `worker` is the currently-running spawn-blocking task (if
        // any). `cancel` is the AI-PoW MiningCancel handle for it.
        // On a new candidate we cancel the existing attempt + spawn
        // a fresh one. On shutdown we cancel + drain.
        let mut worker: Option<(
            JoinHandle<Result<MinedSolution, PuzzleMiningError>>,
            MiningCancel,
        )> = None;
        let inner_result: InnerOutcome = loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break InnerOutcome::Shutdown,
                maybe_c = candidates.next() => {
                    let Some(c_res) = maybe_c else {
                        warn!("watch_candidates stream ended; will reconnect");
                        break InnerOutcome::StreamLost;
                    };
                    let candidate = match c_res {
                        Ok(c) => c,
                        Err(e) => break InnerOutcome::Fatal(MinerError::CandidateDecode(format!("{e}"))),
                    };
                    // Cancel any in-flight attempt; await its join so we
                    // don't accumulate handles. Drop the result — we're
                    // moving on.
                    if let Some((h, c)) = worker.take() {
                        c.cancel();
                        match h.await {
                            Ok(_) => {}
                            Err(e) => debug!(error = %e, "prior worker join error (ignored)"),
                        }
                    }
                    let (target, anchors) = match derive_job_inputs(&candidate) {
                        Ok(x) => x,
                        Err(e) => {
                            warn!(error = %e, "could not derive job inputs from candidate; skipping");
                            continue;
                        }
                    };
                    info!(pow_len = candidate.pow_len, "new candidate; dispatching ai-pow attempt");
                    let cancel = MiningCancel::new();
                    let h = spawn_attempt(&cfg, anchors, target, cancel.clone());
                    worker = Some((h, cancel));
                }
                done = await_worker(&mut worker) => {
                    // worker is now None.
                    match done {
                        WorkerOutcome::None => {
                            // No worker was running; await_worker yielded
                            // immediately. Park ourselves until the next
                            // candidate / shutdown.
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        WorkerOutcome::Joined(Ok(Ok(sol))) => {
                            info!(
                                matmul_attempts = sol.stats.matmul_attempts_tried,
                                elapsed_s = sol.stats.elapsed.as_secs_f64(),
                                matmul_attempt_rate = sol.stats.matmul_attempt_rate_per_sec(),
                                "ai-pow-miner: solution found; submitting"
                            );
                            let Some(build_certificate) = cfg.puzzle.certificate_builder.as_ref() else {
                                warn!(
                                    "ai-pow-miner: solution found but no recursive certificate builder configured; refusing to submit legacy nonce/tile artifact"
                                );
                                continue;
                            };
                            let cert = match build_certificate(&sol) {
                                Ok(cert) => cert,
                                Err(e) => {
                                    warn!(error = %e, "canonical AI-PoW certificate build failed");
                                    break InnerOutcome::Fatal(MinerError::CertificateBuild(e.to_string()));
                                }
                            };
                            let poke = match build_ai_pow_certificate_poke(&sol.nonce, &cert) {
                                Ok(poke) => poke,
                                Err(e) => {
                                    warn!(error = %e, "canonical AI-PoW certificate poke build failed");
                                    break InnerOutcome::Fatal(MinerError::CertificateBuild(e.to_string()));
                                }
                            };
                            if let Err(e) = client
                                .poke_wire(AiPowMinerWire::Mined.to_wire(), poke)
                                .await
                            {
                                warn!(error = %e, "submit canonical ai-pow certificate poke failed (likely stale candidate)");
                            }
                        }
                        WorkerOutcome::Joined(Ok(Err(PuzzleMiningError::Cancelled))) => {
                            debug!("worker cancelled (expected on candidate supersede / shutdown)");
                        }
                        WorkerOutcome::Joined(Ok(Err(e))) => {
                            warn!(error = %e, "ai-pow attempt terminated without solution");
                        }
                        WorkerOutcome::Joined(Err(e)) => {
                            break InnerOutcome::Fatal(MinerError::WorkerJoin(format!("{e}")));
                        }
                    }
                }
            }
        };

        // ── cleanup before reconnect or exit ──
        if let Some((h, c)) = worker.take() {
            c.cancel();
            let _ = h.await;
        }
        let _ = client
            .enable_mining(AiPowMinerWire::Enable.to_wire(), false)
            .await;

        match inner_result {
            InnerOutcome::Shutdown => return Ok(()),
            InnerOutcome::StreamLost => {
                consecutive_failures += 1;
                if consecutive_failures >= cfg.reconnect_max_attempts {
                    return Err(MinerError::TooManyReconnects {
                        count: consecutive_failures,
                    });
                }
                tokio::select! {
                    _ = shutdown.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(cfg.reconnect_backoff_max);
            }
            InnerOutcome::Fatal(e) => return Err(e),
        }
    }

    Ok(())
}

enum InnerOutcome {
    Shutdown,
    StreamLost,
    Fatal(MinerError),
}

enum WorkerOutcome {
    /// No worker was running; the future returned immediately.
    None,
    /// Worker joined: outer Result = tokio JoinError, inner = mining result.
    Joined(Result<Result<MinedSolution, PuzzleMiningError>, tokio::task::JoinError>),
}

/// Helper to make `tokio::select!` work over an `Option<JoinHandle>`.
/// If the slot is empty, returns `WorkerOutcome::None` immediately
/// (caller pauses to avoid a busy-loop). If the slot has a handle,
/// awaits it (drops it on join). Mutates `worker` in place so the
/// caller doesn't need to thread the take/put.
async fn await_worker(
    worker: &mut Option<(
        JoinHandle<Result<MinedSolution, PuzzleMiningError>>,
        MiningCancel,
    )>,
) -> WorkerOutcome {
    if let Some((handle, _)) = worker.as_mut() {
        let outcome = handle.await;
        *worker = None;
        WorkerOutcome::Joined(outcome)
    } else {
        WorkerOutcome::None
    }
}

/// Derive the per-candidate job inputs the AI-PoW prover needs:
/// the 32-byte chain difficulty target and the NCMN nonce anchors.
///
/// **`nck_commitment`** is `BLAKE3(jam(candidate.block_header))` — a
/// deterministic, collision-resistant 32-byte commitment to the
/// chain's per-block header noun. The exact derivation is local to
/// this miner crate (the chain-AI integration will swap this in for
/// a chain-prescribed binding when the kernel-side puzzle lands).
///
/// **`target`** is decoded from the kernel-side bignum noun
/// `[%bn limbs]`, where `limbs` are little-endian u32 chunks. The
/// ai-pow primitive compares BLAKE3 attempt hashes as 256-bit
/// little-endian integers, so bignum values above `2^256 - 1`
/// saturate to `FF..FF`.
fn derive_job_inputs(
    candidate: &MiningCandidate,
) -> Result<(DifficultyTarget, NonceAnchors), String> {
    // Hash the jammed block_header to a 32-byte commitment.
    let header_bytes = candidate.block_header.jam();
    let nck = *blake3::hash(&header_bytes).as_bytes();
    let target = decode_chain_target_bignum(&candidate.target)?;
    Ok((target, NonceAnchors::nck_only(nck)))
}

fn decode_chain_target_bignum(target: &NounSlab) -> Result<DifficultyTarget, String> {
    let space = target.noun_space();
    let root = unsafe { *target.root() };
    let target_cell = root
        .in_space(&space)
        .as_cell()
        .map_err(|_| "target must be a Hoon bignum cell [%bn limbs]".to_string())?;
    if !target_cell.head().eq_bytes("bn") {
        return Err("target must have %bn bignum tag".to_string());
    }

    let mut out = [0u8; 32];
    let mut list = target_cell.tail().noun();
    let mut limb_index = 0usize;
    let mut saturate = false;

    loop {
        if noun_is_zero_atom(list, &space)? {
            break;
        }
        if limb_index >= MAX_CHAIN_TARGET_U32_LIMBS {
            return Err(format!(
                "target bignum exceeds {MAX_CHAIN_TARGET_U32_LIMBS} u32 limbs"
            ));
        }

        let limb_cell = list
            .in_space(&space)
            .as_cell()
            .map_err(|_| "target bignum limbs must be a proper list".to_string())?;
        let limb = limb_cell
            .head()
            .as_atom()
            .map_err(|_| "target bignum limb must be an atom".to_string())?
            .as_u64()
            .map_err(|_| "target bignum limb does not fit in u64".to_string())?;
        let limb =
            u32::try_from(limb).map_err(|_| "target bignum limb must fit in u32".to_string())?;

        if limb_index < 8 {
            let offset = limb_index * 4;
            out[offset..offset + 4].copy_from_slice(&limb.to_le_bytes());
        } else if limb != 0 {
            saturate = true;
        }

        list = limb_cell.tail().noun();
        limb_index += 1;
    }

    Ok(if saturate { [0xFF; 32] } else { out })
}

fn noun_is_zero_atom(
    noun: nockvm::noun::Noun,
    space: &nockvm::noun::NounSpace,
) -> Result<bool, String> {
    match noun.in_space(space).as_atom() {
        Ok(atom) => atom
            .as_u64()
            .map(|value| value == 0)
            .map_err(|_| "target bignum list terminator does not fit in u64".to_string()),
        Err(_) => Ok(false),
    }
}

/// Spawn the per-attempt blocking worker. Holds owned clones of the
/// matrices and params so the spawned task is `'static`.
fn spawn_attempt(
    cfg: &MinerConfig,
    anchors: NonceAnchors,
    target: DifficultyTarget,
    cancel: MiningCancel,
) -> JoinHandle<Result<MinedSolution, PuzzleMiningError>> {
    let puzzle_id = cfg.puzzle.puzzle_id.clone();
    let params = cfg.puzzle.params;
    let a = cfg.puzzle.a.clone();
    let b = cfg.puzzle.b.clone();
    let opts = cfg.mine_opts.clone();
    tokio::task::spawn_blocking(move || {
        let job = MiningJob {
            puzzle_id: &puzzle_id,
            anchors,
            params: &params,
            target,
            a: &a,
            b: &b,
        };
        mining::run(&job, &opts, cancel)
    })
}

/// Build the production consensus poke shape:
///
/// ```hoon
/// [%command %pow %ai-pow nonce cert]
/// ```
///
/// `cert` must already be the structured Hoon-compatible
/// `ai-pow-certificate` noun. The NCMN nonce is carried alongside it because
/// the production verifier must check that the attempt is anchored to the
/// candidate block before accepting the recursive proof. This helper
/// deliberately does not accept the plain `MatmulProof` or tile index because
/// those are not canonical block proof artifacts.
pub fn build_ai_pow_certificate_poke(
    nonce: &NcmnNonce,
    certificate: &NounSlab,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    use nockapp::IndirectAtomExt;
    use nockvm::noun::{IndirectAtom, NounAllocator};

    let (anchors, _) = parse_ncmn_nonce(nonce)?;
    if anchors.external_commitment.is_some() {
        return Err(AiPowCertificatePokeError::NonceExternalCommitmentPresent);
    }

    let cert_space = certificate.noun_space();
    let mut slab = NounSlab::new();
    let nonce = <IndirectAtom as IndirectAtomExt>::from_bytes(&mut slab, nonce).as_noun();
    let cert = slab.copy_into(unsafe { *certificate.root() }, &cert_space);
    let payload = T(
        &mut slab,
        &[D(tas!(b"command")), D(tas!(b"pow")), D(tas!(b"ai-pow")), nonce, cert],
    );
    slab.set_root(payload);
    Ok(slab)
}

/// Build the production Pearl merge-mined consensus poke shape:
///
/// ```hoon
/// [%command %pow %ai-pmp artifact]
/// ```
///
/// `artifact` must already be the structured Hoon-compatible `%ai-pmp`
/// artifact:
///
/// ```hoon
/// [%ai-pmp statement=pearl-merge-public-statement cert=ai-pow-certificate]
/// ```
///
/// The helper decodes the artifact shape before wrapping it so miner-side code
/// cannot accidentally submit a native `%ai-pow` certificate, a Pearl ZKP, or a
/// raw `MatmulProof` on the Pearl merge-mining arm.
pub fn build_ai_pow_pearl_merge_certificate_poke(
    artifact: &NounSlab,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    decode_ai_pow_pearl_merge_artifact_slab(artifact, CertificateNounLimits::default())?;

    let artifact_space = artifact.noun_space();
    let mut slab = NounSlab::new();
    let artifact = slab.copy_into(unsafe { *artifact.root() }, &artifact_space);
    let payload = T(
        &mut slab,
        &[D(tas!(b"command")), D(tas!(b"pow")), D(tas!(b"ai-pmp")), artifact],
    );
    slab.set_root(payload);
    Ok(slab)
}

/// Build the production Pearl merge-mined consensus poke directly from a
/// successful Pearl-compatible ticket and an already-serialized recursive
/// proof node.
///
/// This is the miner-facing safe path for `%ai-pmp`: it recomputes the public
/// Pearl work against trusted matrices before deriving recursive metadata, then
/// wraps only the resulting structured `%ai-pmp` artifact.
pub fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(
    attempt: &PearlMergeTicketAttempt,
    a_row_major: &[i8],
    b_col_major: &[i8],
    max_pattern_len: usize,
    certificate: &AiProofNode,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    let artifact = build_ai_pow_pearl_merge_artifact_noun_from_ticket_node(
        attempt, a_row_major, b_col_major, max_pattern_len, certificate,
    )?;
    build_ai_pow_pearl_merge_certificate_poke(&artifact)
}

/// Build the production Pearl merge-mined consensus poke directly from a
/// successful Pearl-compatible ticket and recursive certificate object.
pub fn build_ai_pow_pearl_merge_certificate_poke_from_ticket<C: serde::Serialize>(
    attempt: &PearlMergeTicketAttempt,
    a_row_major: &[i8],
    b_col_major: &[i8],
    max_pattern_len: usize,
    recursive_certificate: &C,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    let artifact = build_ai_pow_pearl_merge_artifact_noun_from_ticket(
        attempt, a_row_major, b_col_major, max_pattern_len, recursive_certificate,
    )?;
    build_ai_pow_pearl_merge_certificate_poke(&artifact)
}

// ──────────────────────────── tests ────────────────────────────

#[cfg(test)]
mod tests {
    //! Integration tests for the AI-PoW miner run loop.
    //!
    //! Strategy: stand up a private `NockAppService` gRPC server on an
    //! ephemeral port (same fixture pattern as `zk-pow-miner`'s
    //! run-loop tests), drive [`run`] against it, push a synthetic
    //! `%mine` effect, and assert the miner pokes an
    //! `AiPowMinerWire::Mined` slab back at the server within a
    //! generous timeout. Uses `MatmulParams::TEST_SMALL` + trivial
    //! `FF..FF` target so the real ai-pow prover wins on extranonce 0.

    use std::net::{SocketAddr, TcpListener};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use ai_pow::params::MatmulParams;
    use ai_pow::pearl_compat::{
        evaluate_pearl_merge_ticket_attempt, PearlIncompleteBlockHeader, PearlMiningConfig,
        PearlNockchainAux, PearlPeriodicPattern, PEARL_MINING_CONFIG_RESERVED_SIZE,
        PEARL_MMA_INT7XINT7_TO_INT32,
    };
    use ai_pow::synth::synth_matrices;
    use ai_pow::zk_bridge::ZkPublicCommitments;
    use ai_pow_zk::{CompositePublicInputs, ZkParams};
    use nockapp::driver::{IOAction, NockAppHandle};
    use nockapp::noun::slab::NounSlab;
    use nockapp::NockAppExit;
    use nockapp_grpc::services::private_nockapp::server::PrivateNockAppGrpcServer;
    use nockchain_mining_common::MiningPkhConfig;
    use nockvm::noun::{D, T};
    use nockvm_macros::tas;
    use once_cell::sync::Lazy;
    use tokio::sync::{broadcast, mpsc, Mutex as TMutex};

    use super::*;
    use crate::certificate_noun::{
        build_ai_pow_certificate_noun_from_node, build_ai_pow_pearl_merge_artifact_noun_from_node,
        decode_ai_pow_pearl_merge_artifact_noun, AiProofNode, PearlMergePublicStatementShape,
    };
    use crate::pearl_mining::{
        self, PearlMergeMineOptions, PearlMergeMiningError, PearlMergeMiningJob,
    };
    use crate::wire::AiPowMinerWire;

    // Shared NockAppMetrics — gnort rejects double-registration.
    static METRICS: Lazy<Arc<nockapp::nockapp::metrics::NockAppMetrics>> = Lazy::new(|| {
        Arc::new(
            nockapp::nockapp::metrics::NockAppMetrics::register(gnort::global_metrics_registry())
                .expect("register NockAppMetrics"),
        )
    });

    struct MockNode {
        addr: SocketAddr,
        effect_tx: Arc<broadcast::Sender<NounSlab>>,
        pokes_observed: Arc<AtomicU64>,
        mined_pokes: Arc<TMutex<Vec<NounSlab>>>,
        server_task: tokio::task::JoinHandle<nockapp_grpc::error::Result<()>>,
        action_drainer: tokio::task::JoinHandle<()>,
    }

    impl MockNode {
        async fn spawn() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("local_addr");
            drop(listener);
            let (action_tx, mut action_rx) = mpsc::channel::<IOAction>(64);
            let (effect_tx, _seed_rx) = broadcast::channel::<NounSlab>(64);
            let effect_tx = Arc::new(effect_tx);
            let effect_rx_for_handle = effect_tx.subscribe();
            let (exit, _exit_rx) = NockAppExit::new();
            let handle = NockAppHandle {
                io_sender: action_tx,
                effect_sender: effect_tx.clone(),
                effect_receiver: TMutex::new(effect_rx_for_handle),
                metrics: METRICS.clone(),
                exit,
            };
            let pokes_observed = Arc::new(AtomicU64::new(0));
            let mined_pokes: Arc<TMutex<Vec<NounSlab>>> = Arc::new(TMutex::new(Vec::new()));
            let pokes_clone = pokes_observed.clone();
            let mined_clone = mined_pokes.clone();
            let action_drainer = tokio::spawn(async move {
                while let Some(action) = action_rx.recv().await {
                    match action {
                        IOAction::Poke {
                            wire,
                            ack_channel,
                            poke,
                            ..
                        } => {
                            pokes_clone.fetch_add(1, Ordering::SeqCst);
                            if wire.source == <AiPowMinerWire as nockapp::wire::Wire>::SOURCE
                                && wire.tags.iter().any(|t| match t {
                                    nockapp::wire::WireTag::String(s) => s == "mined",
                                    _ => false,
                                })
                            {
                                mined_clone.lock().await.push(poke);
                            }
                            use nockapp::driver::PokeResult;
                            let _ = ack_channel.send(PokeResult::Ack);
                        }
                        IOAction::Peek { .. } => {}
                    }
                }
            });
            let server = PrivateNockAppGrpcServer::new(handle);
            let server_task = tokio::spawn(async move { server.serve(addr).await });
            tokio::time::sleep(Duration::from_millis(100)).await;
            MockNode {
                addr,
                effect_tx,
                pokes_observed,
                mined_pokes,
                server_task,
                action_drainer,
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.addr)
        }

        // Publish a synthetic %mine-ai effect (the AI miner subscribes
        // to mine-ai post Stage 4; the mock fixture publishes accordingly).
        fn publish_synth_mine_effect(&self, header_seed: u64, target_seed: u64, pow_len: u64) {
            let mut slab = NounSlab::new();
            let head = D(tas!(b"mine-ai"));
            let version = D(0);
            let commit = T(
                &mut slab,
                &[
                    D(header_seed),
                    D(header_seed + 1),
                    D(header_seed + 2),
                    D(header_seed + 3),
                    D(header_seed + 4),
                ],
            );
            let target_list = T(&mut slab, &[D(target_seed), D(0)]);
            let target = T(&mut slab, &[D(tas!(b"bn")), target_list]);
            let plen = D(pow_len);
            let effect = T(&mut slab, &[head, version, commit, target, plen]);
            slab.set_root(effect);
            self.effect_tx.send(slab).expect("publish %mine effect");
        }

        async fn shutdown(self) {
            self.server_task.abort();
            self.action_drainer.abort();
            let _ = self.server_task.await;
            let _ = self.action_drainer.await;
        }
    }

    fn single_tile_prod_params() -> MatmulParams {
        MatmulParams {
            m: 8,
            k: 512,
            n: 8,
            noise_rank: 32,
            tile: 8,
            spot_checks: 1,
            difficulty_bits: 0,
        }
    }

    fn multi_tile_prod_params() -> MatmulParams {
        MatmulParams {
            m: 16,
            k: 512,
            n: 16,
            noise_rank: 32,
            tile: 8,
            spot_checks: 4,
            difficulty_bits: 0,
        }
    }

    fn pearl_test_pattern(length: u32) -> PearlPeriodicPattern {
        PearlPeriodicPattern {
            shape: [(1, length), (length, 1), (length, 1)],
        }
    }

    fn pearl_test_header() -> PearlIncompleteBlockHeader {
        PearlIncompleteBlockHeader {
            version: 0x0102_0304,
            prev_block: [0x11; 32],
            merkle_root: [0x22; 32],
            timestamp: 0x6677_8899,
            nbits: 0x207f_ffff,
        }
    }

    fn pearl_test_config() -> PearlMiningConfig {
        PearlMiningConfig {
            common_dim: 1024,
            rank: 64,
            mma_type: PEARL_MMA_INT7XINT7_TO_INT32,
            rows_pattern: pearl_test_pattern(8),
            cols_pattern: pearl_test_pattern(8),
            reserved: [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
        }
    }

    fn pearl_test_params() -> MatmulParams {
        MatmulParams {
            m: 128,
            k: 1024,
            n: 128,
            noise_rank: 64,
            tile: 8,
            spot_checks: 1,
            difficulty_bits: 0,
        }
    }

    fn pearl_test_aux() -> PearlNockchainAux {
        PearlNockchainAux {
            nockchain_chain_id: b"nockchain-mainnet".to_vec(),
            nock_block_commitment: [0x42; 32],
            nockchain_target_epoch_or_height: 123_456,
            extra_domain_data: b"ai-pow-target-window".to_vec(),
        }
    }

    fn test_cfg(node_addr: String) -> MinerConfig {
        let params = single_tile_prod_params();
        let (a, b) = synth_matrices(b"ai-pow-node-run-test", &params);
        let puzzle = AiPuzzleInputs {
            puzzle_id: b"ai-pow-node-run-test-pid".to_vec(),
            params,
            a: Arc::new(a),
            b: Arc::new(b),
            prover_opts: ProverOptions::default(),
            certificate_builder: Some(Arc::new(|sol: &MinedSolution| {
                let params = ZkParams {
                    m: 8,
                    k: 512,
                    n: 8,
                    noise_rank: 32,
                    tile: 8,
                    difficulty_bits: 0,
                };
                let commitments = ZkPublicCommitments {
                    h_a_chunk: [12; 32],
                    h_b_chunk: [13; 32],
                };
                let mut pis = CompositePublicInputs::zero();
                pis.jackpot = core::array::from_fn(|i| i as u32);
                let certificate = AiProofNode::Seq(vec![AiProofNode::U64(sol.found_idx as u64)]);
                Ok(build_ai_pow_certificate_noun_from_node(
                    &params, sol.found_idx, 8_192, &commitments, &pis, &certificate,
                ))
            })),
            submission_mode: AiPowSubmissionMode::NativeNcmn,
        };
        MinerConfig {
            node_addr,
            mining_configs: default_v0_configs(),
            mining_pkh_configs: vec![MiningPkhConfig {
                share: 1,
                pkh: "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV".to_string(),
            }],
            puzzle,
            mine_opts: MineOptions {
                max_extranonces: Some(8),
                ..MineOptions::default()
            },
            reconnect_backoff_initial: Duration::from_millis(50),
            reconnect_backoff_max: Duration::from_millis(200),
            reconnect_max_attempts: 3,
        }
    }

    fn bignum_target_slab(limbs: &[u64]) -> NounSlab {
        let mut slab = NounSlab::new();
        let mut list = D(0);
        for limb in limbs.iter().rev() {
            list = T(&mut slab, &[D(*limb), list]);
        }
        let target = T(&mut slab, &[D(tas!(b"bn")), list]);
        slab.set_root(target);
        slab
    }

    fn candidate_for_target(target: NounSlab) -> MiningCandidate {
        let mut version = NounSlab::new();
        version.set_root(D(0));
        let mut block_header = NounSlab::new();
        block_header.set_root(D(0xCAFE));
        MiningCandidate {
            version,
            block_header,
            target,
            pow_len: 64,
        }
    }

    #[test]
    fn derive_job_inputs_decodes_bignum_target_little_endian() {
        let candidate =
            candidate_for_target(bignum_target_slab(&[0x0403_0201, 0x0807_0605, 0x0c0b_0a09]));

        let (target, _) = derive_job_inputs(&candidate).expect("derive job inputs");

        assert_eq!(&target[0..12], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert!(target[12..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn derive_job_inputs_saturates_targets_above_u256() {
        let mut limbs = vec![0u64; 10];
        limbs[9] = 0x8;
        let candidate = candidate_for_target(bignum_target_slab(&limbs));

        let (target, _) = derive_job_inputs(&candidate).expect("derive job inputs");

        assert_eq!(target, [0xFF; 32]);
    }

    #[test]
    fn derive_job_inputs_rejects_malformed_target_nouns() {
        let mut atom_target = NounSlab::new();
        atom_target.set_root(D(0xFFFF));
        let err = derive_job_inputs(&candidate_for_target(atom_target))
            .expect_err("target atom is not a bignum");
        assert!(err.contains("bignum cell"), "unexpected error: {err}");

        let err = derive_job_inputs(&candidate_for_target(bignum_target_slab(&[u64::from(
            u32::MAX,
        ) + 1])))
        .expect_err("u64 limb exceeds u32");
        assert!(err.contains("u32"), "unexpected error: {err}");

        let err = derive_job_inputs(&candidate_for_target(bignum_target_slab(&[0; 11])))
            .expect_err("oversized limb list is rejected");
        assert!(err.contains("exceeds"), "unexpected error: {err}");
    }

    #[test]
    fn production_preflight_accepts_single_tile_chunk_seed_binding() {
        let cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle
            .validate_canonical_submission_ready()
            .expect("single-tile recursive certificate binds canonical chunk commitments");
    }

    #[test]
    fn production_preflight_rejects_missing_certificate_builder() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.certificate_builder = None;
        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("node miner must not mine without canonical certificate builder");
        assert!(
            err.to_string()
                .contains("no recursive certificate builder configured"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_preflight_rejects_multi_tile_selected_tile_gap_before_mining() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        let params = multi_tile_prod_params();
        assert_eq!(params.num_tiles(), 4);
        params
            .validate_prod_envelope()
            .expect("fixture should isolate the full-matmul gap, not params validity");
        let (a, b) = synth_matrices(b"ai-pow-node-run-multi-tile-preflight", &params);
        cfg.puzzle.params = params;
        cfg.puzzle.a = Arc::new(a);
        cfg.puzzle.b = Arc::new(b);

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("multi-tile selected-tile certificate must fail before mining");
        assert!(
            err.to_string()
                .contains("recursive certificate proves one selected tile"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_preflight_rejects_pearl_merge_mode_until_ticket_loop_is_wired() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.submission_mode = AiPowSubmissionMode::PearlMerge;

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("run loop must not silently submit native ai-pow in Pearl mode");
        assert!(
            err.to_string()
                .contains("Pearl merge-mining mode is explicit"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn build_ai_pow_certificate_poke_has_kernel_command_shape() {
        use nockvm::jets::bits::util::met;
        use nockvm::noun::NounAllocator;

        let mut cert = NounSlab::new();
        cert.set_root(D(999));
        let nonce = crate::build_ncmn_nonce(&crate::NonceAnchors::nck_only([0xA5; 32]), 7);

        let poke = build_ai_pow_certificate_poke(&nonce, &cert).expect("build poke");
        let space = poke.noun_space();
        let root = unsafe { *poke.root() };

        let command_cell = root.in_space(&space).as_cell().expect("poke cell");
        assert!(command_cell.head().eq_bytes("command"));

        let pow_cell = command_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("pow cell");
        assert!(pow_cell.head().eq_bytes("pow"));

        let ai_pow_cell = pow_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("ai-pow cell");
        assert!(ai_pow_cell.head().eq_bytes("ai-pow"));

        let nonce_cell = ai_pow_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("nonce/certificate cell");
        let nonce_atom = nonce_cell.head().as_atom().expect("nonce atom");
        assert_eq!(
            met(3, nonce_atom.atom(), &space),
            ai_pow::ncmn::NCMN_NONCE_LEN
        );

        let cert_atom = nonce_cell
            .tail()
            .as_atom()
            .expect("certificate atom")
            .as_u64()
            .expect("certificate atom fits u64");
        assert_eq!(cert_atom, 999);
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_has_kernel_command_shape() {
        let statement = PearlMergePublicStatementShape {
            block_header: [0x10; ai_pow::pearl_compat::PEARL_INCOMPLETE_BLOCK_HEADER_SIZE],
            public_data: [0x20; ai_pow::pearl_compat::PEARL_PUBLIC_PROOF_PARAMS_SIZE],
            expected_aux_commitment: [0x30; 32],
            aux: PearlNockchainAux {
                nockchain_chain_id: b"nockchain-mainnet".to_vec(),
                nock_block_commitment: [0x42; 32],
                nockchain_target_epoch_or_height: 123_456,
                extra_domain_data: b"ai-pow-target-window".to_vec(),
            },
        };
        let params = ZkParams {
            m: 8,
            k: 512,
            n: 8,
            noise_rank: 32,
            tile: 8,
            difficulty_bits: 0,
        };
        let commitments = ZkPublicCommitments {
            h_a_chunk: [12; 32],
            h_b_chunk: [13; 32],
        };
        let pis = CompositePublicInputs::zero();
        let artifact = build_ai_pow_pearl_merge_artifact_noun_from_node(
            &statement,
            &params,
            0,
            8_192,
            &commitments,
            &pis,
            &AiProofNode::Unit,
        );

        let poke =
            build_ai_pow_pearl_merge_certificate_poke(&artifact).expect("build pearl merge poke");
        let space = poke.noun_space();
        let root = unsafe { *poke.root() };

        let command_cell = root.in_space(&space).as_cell().expect("poke cell");
        assert!(command_cell.head().eq_bytes("command"));

        let pow_cell = command_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("pow cell");
        assert!(pow_cell.head().eq_bytes("pow"));

        let ai_pmp_cell = pow_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("ai-pmp cell");
        assert!(ai_pmp_cell.head().eq_bytes("ai-pmp"));

        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pmp_cell.tail().noun(),
            &space,
            CertificateNounLimits::default(),
        )
        .expect("decode wrapped pearl merge artifact");
        assert_eq!(decoded.statement, statement);
        assert_eq!(decoded.certificate.zk_params, params);
        assert_eq!(decoded.certificate.commitments, commitments);
        assert_eq!(decoded.certificate.public_inputs, pis);
        assert_eq!(decoded.certificate.certificate, AiProofNode::Unit);
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_derives_artifact() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-ticket-poke", &params);
        let attempt = evaluate_pearl_merge_ticket_attempt(
            &pearl_test_header(),
            &pearl_test_config(),
            &params,
            0,
            0,
            &a,
            &b,
            &[0xff; 32],
            16,
            pearl_test_aux(),
        )
        .expect("evaluate Pearl ticket");

        let poke = build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(
            &attempt,
            &a,
            &b,
            16,
            &AiProofNode::Unit,
        )
        .expect("build pearl merge poke from ticket");
        let space = poke.noun_space();
        let root = unsafe { *poke.root() };
        let command_cell = root.in_space(&space).as_cell().expect("poke cell");
        assert!(command_cell.head().eq_bytes("command"));
        let pow_cell = command_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("pow cell");
        assert!(pow_cell.head().eq_bytes("pow"));
        let ai_pmp_cell = pow_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("ai-pmp cell");
        assert!(ai_pmp_cell.head().eq_bytes("ai-pmp"));

        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pmp_cell.tail().noun(),
            &space,
            CertificateNounLimits::default(),
        )
        .expect("decode ticket-derived pearl merge artifact");
        assert_eq!(
            decoded.statement,
            PearlMergePublicStatementShape::from_wire_statement(&attempt.statement)
                .expect("statement shape")
        );
        assert_eq!(decoded.certificate.found_idx, 0);
        assert_eq!(decoded.certificate.certificate, AiProofNode::Unit);
    }

    #[test]
    fn pearl_ticket_loop_output_builds_canonical_ai_pmp_poke() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-loop-to-poke", &params);
        let header = pearl_test_header();
        let config = pearl_test_config();
        let aux = pearl_test_aux();
        let job = PearlMergeMiningJob {
            header: &header,
            config: &config,
            params: &params,
            nockchain_target: [0xff; 32],
            a: &a,
            b: &b,
            max_pattern_len: 16,
            aux,
        };
        let mined = pearl_mining::run(
            &job,
            &PearlMergeMineOptions {
                max_attempts: Some(1),
                ..PearlMergeMineOptions::default()
            },
            MiningCancel::new(),
        )
        .expect("Pearl ticket loop should mine first trivial-target ticket");

        let poke = build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(
            &mined.attempt,
            &a,
            &b,
            job.max_pattern_len,
            &AiProofNode::Unit,
        )
        .expect("mined Pearl ticket should build ai-pmp poke");
        let space = poke.noun_space();
        let root = unsafe { *poke.root() };
        let command_cell = root.in_space(&space).as_cell().expect("poke cell");
        assert!(command_cell.head().eq_bytes("command"));
        let pow_cell = command_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("pow cell");
        assert!(pow_cell.head().eq_bytes("pow"));
        let ai_pmp_cell = pow_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("ai-pmp cell");
        assert!(ai_pmp_cell.head().eq_bytes("ai-pmp"));

        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pmp_cell.tail().noun(),
            &space,
            CertificateNounLimits::default(),
        )
        .expect("decode mined-ticket ai-pmp artifact");
        assert_eq!(
            decoded.certificate.found_idx,
            mined.attempt.public_params.t_rows
        );
        assert_eq!(
            decoded.statement,
            PearlMergePublicStatementShape::from_wire_statement(&mined.attempt.statement)
                .expect("statement shape")
        );
    }

    #[test]
    fn pearl_ticket_loop_miss_cannot_build_ai_pmp_poke() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-loop-miss-no-poke", &params);
        let header = pearl_test_header();
        let config = pearl_test_config();
        let job = PearlMergeMiningJob {
            header: &header,
            config: &config,
            params: &params,
            nockchain_target: [0; 32],
            a: &a,
            b: &b,
            max_pattern_len: 16,
            aux: pearl_test_aux(),
        };

        assert!(matches!(
            pearl_mining::run(
                &job,
                &PearlMergeMineOptions {
                    max_attempts: Some(1),
                    ..PearlMergeMineOptions::default()
                },
                MiningCancel::new(),
            ),
            Err(PearlMergeMiningError::BudgetExhausted { max: 1 })
        ));
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_rejects_wrong_matrices() {
        let params = pearl_test_params();
        let (mut a, b) = synth_matrices(b"pearl-run-ticket-poke-wrong-matrices", &params);
        let attempt = evaluate_pearl_merge_ticket_attempt(
            &pearl_test_header(),
            &pearl_test_config(),
            &params,
            0,
            0,
            &a,
            &b,
            &[0xff; 32],
            16,
            pearl_test_aux(),
        )
        .expect("evaluate Pearl ticket");
        a[0] ^= 1;

        assert!(matches!(
            build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(
                &attempt,
                &a,
                &b,
                16,
                &AiProofNode::Unit,
            ),
            Err(AiPowCertificatePokeError::PearlMergeArtifact(
                CertificateNounError::PearlMergeStatement(
                    ai_pow::pearl_compat::PearlCompatError::PublicCommitmentMismatch
                )
            ))
        ));
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_rejects_wrong_artifact_arm() {
        let mut wrong = NounSlab::new();
        wrong.set_root(D(999));

        assert!(matches!(
            build_ai_pow_pearl_merge_certificate_poke(&wrong),
            Err(AiPowCertificatePokeError::PearlMergeArtifact(_))
        ));
    }

    #[test]
    fn build_ai_pow_certificate_poke_rejects_non_canonical_nonce() {
        let mut cert = NounSlab::new();
        cert.set_root(D(999));

        let bad_magic = [0u8; ai_pow::ncmn::NCMN_NONCE_LEN];
        assert!(matches!(
            build_ai_pow_certificate_poke(&bad_magic, &cert),
            Err(AiPowCertificatePokeError::Nonce(_))
        ));

        let external_nonce = crate::build_ncmn_nonce(
            &crate::NonceAnchors {
                nck_commitment: [0x11; 32],
                external_commitment: Some([0x22; 32]),
            },
            7,
        );
        assert!(matches!(
            build_ai_pow_certificate_poke(&external_nonce, &cert),
            Err(AiPowCertificatePokeError::NonceExternalCommitmentPresent)
        ));
    }

    /// Heavy: runs the real ai-pow prover on TEST_SMALL with a trivial
    /// `FF..FF` target. Should complete in well under 30 s on any
    /// modern machine; marked `#[ignore]` so `cargo test` is fast by
    /// default. Run with `cargo test -p ai-pow-miner --features node
    /// --test node_run_mock_node -- --ignored`.
    #[ignore = "manual mock-node integration test; runs the real prover"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_against_mock_node_submits_ai_pow_command_after_certificate_gate_reopens() {
        let node = MockNode::spawn().await;
        let cfg = test_cfg(node.url());

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        let mining_task = tokio::spawn(async move { run(cfg, shutdown_clone).await });

        // Brief pause for the miner to connect + configure + subscribe.
        tokio::time::sleep(Duration::from_millis(300)).await;
        node.publish_synth_mine_effect(100, 0xFFFF_FFFF, 64);

        // Poll for the miner wire poke. Allow up to 30 s for the trivial-target prover.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut got_mined = false;
        while std::time::Instant::now() < deadline {
            if !node.mined_pokes.lock().await.is_empty() {
                got_mined = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            got_mined,
            "miner did not submit a %mined poke within 30s; observed {} total pokes",
            node.pokes_observed.load(Ordering::SeqCst)
        );

        shutdown.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), mining_task)
            .await
            .expect("miner task did not exit");
        node.shutdown().await;
    }

    /// Cheap: confirms the node runner fails closed before reconnect work when
    /// the configured recursive certificate is not canonical-admissible.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_loop_rejects_before_connect_when_certificate_gate_is_closed() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.params = multi_tile_prod_params();
        let shutdown = CancellationToken::new();
        let r = tokio::time::timeout(Duration::from_secs(2), run(cfg, shutdown))
            .await
            .expect("run didn't terminate");
        match r {
            Err(MinerError::CanonicalCertificateUnavailable(msg)) => {
                assert!(
                    msg.contains("not a full 4-tile matmul"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("expected CanonicalCertificateUnavailable, got {other:?}"),
        }
    }

    /// Cheap: confirms shutdown does not turn the canonical-certificate
    /// preflight failure into a successful run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_shutdown_still_reports_closed_certificate_gate() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.params = multi_tile_prod_params();
        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        let mining_task = tokio::spawn(async move { run(cfg, shutdown_clone).await });
        shutdown.cancel();
        let r = tokio::time::timeout(Duration::from_secs(10), mining_task)
            .await
            .expect("miner did not exit within 10s")
            .expect("miner panicked");
        assert!(
            matches!(r, Err(MinerError::CanonicalCertificateUnavailable(_))),
            "expected closed canonical certificate gate, got {r:?}"
        );
    }
}
