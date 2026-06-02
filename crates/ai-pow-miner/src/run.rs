//! Out-of-process node-connecting run loop for the AI-PoW miner.
//!
//! Mirrors `zk-pow-miner::run` in shape: the miner runs as a separate
//! OS process and talks to a `nockchain` node over the node's private
//! [`nockapp_grpc`] `NockAppService`. The substrate (connect /
//! `set-mining-key` / `enable-mining` / `WatchEffects` / `submit`)
//! is shared via [`nockchain_mining_common::NodeClient`]; only the
//! puzzle-specific bits (the AI-PoW matmul prover and
//! `AiPowMinerWire` submission wire) live here.
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
//!    - new candidate -> cancel any in-flight attempt, derive the
//!      Pearl-compatible mining job, and spawn the worker.
//!    - worker result:
//!      - success → build the canonical recursive certificate only after a
//!        target hit, then poke the node with a canonical `%ai-pow` command on
//!        [`AiPowMinerWire::Mined`].
//!      - error → log + idle.
//! 5. Stream drop → outer loop reconnects.
//!
//! ## Note on submission
//! The canonical payload shape is a `%ai-pow` noun carrying an opaque
//! Rust-owned nonce and the recursive AI-PoW certificate noun. The plain
//! `MatmulProof` and tile index are mining internals; they are not submitted
//! to the kernel as the block proof. In Pearl-compatible mode the run loop
//! constructs the Rust-owned `AIP1` nonce and submits only the Nockchain
//! `%ai-pow` command; it does not submit anything to Pearl. The kernel remains
//! fail-closed until recursive certificate verification is wired.

use std::sync::Arc;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use ai_pow::pearl_compat::{
    pearl_bitcoin_double_sha256_raw, validate_pearl_merge_config_for_recursive_prover,
    PearlAuxInclusionProof, PearlIncompleteBlockHeader, PearlMergeTicketAttempt, PearlMiningConfig,
    PearlNockchainAux, PEARL_NOCKCHAIN_AUX_COMMITMENT_TAG,
};
use ai_pow::prover::ProverOptions;
use ai_pow::zk_bridge::{AiPowRecursiveCertificateRun, ZkPublicCommitments};
use ai_pow_zk::{CompositePublicInputs, ZkParams};
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
    build_ai_pow_pearl_merge_artifact_noun_from_ticket_public_inputs_node,
    build_ai_pow_pearl_merge_artifact_noun_from_ticket_recursive_run,
    decode_ai_pow_pearl_merge_artifact_metadata_slab, AiProofNode, CertificateNounError,
    CertificateNounLimits,
};
use crate::pearl_mining::{
    self, PearlMergeMineOptions, PearlMergeMinedTicket, PearlMergeMiningError, PearlMergeMiningJob,
};
use crate::wire::AiPowMinerWire;
use crate::{DifficultyTarget, MiningCancel};

const MAX_CHAIN_TARGET_U32_LIMBS: usize = 10;

pub type AiPowPearlMergeCertificateBuilder = dyn Fn(&PearlMergeTicketAttempt) -> Result<PearlMergeCertificateProof, AiPowCertificateBuildError>
    + Send
    + Sync
    + 'static;

/// Recursive proof data produced only after a Pearl-compatible ticket clears
/// Nockchain's target.
///
/// Public callers can construct this only from the opaque
/// [`AiPowRecursiveCertificateRun`] returned by the recursive prover. Tests
/// inside this crate may still inject synthetic proof nodes to exercise the
/// surrounding noun and run-loop plumbing without running the prover.
#[derive(Debug, Clone)]
pub struct PearlMergeCertificateProof {
    zk_params: ZkParams,
    found_idx: u32,
    commitments: ZkPublicCommitments,
    public_inputs: CompositePublicInputs,
    trace_height: usize,
    certificate: AiProofNode,
}

impl PearlMergeCertificateProof {
    pub fn from_recursive_run(
        run: &AiPowRecursiveCertificateRun,
    ) -> Result<Self, AiPowCertificateBuildError> {
        let certificate = crate::certificate_noun::recursive_certificate_to_node(run.certificate())
            .map_err(|e| AiPowCertificateBuildError(e.to_string()))?;
        Ok(Self {
            zk_params: run.zk_params(),
            found_idx: run.found_idx(),
            commitments: run.commitments(),
            public_inputs: run.public_inputs().clone(),
            trace_height: run.trace_height(),
            certificate,
        })
    }
}

/// Rust-only Nockchain submission settings for Pearl-compatible mining.
///
/// Hoon still receives only the opaque `AIP1` nonce bytes and recursive
/// certificate; these Pearl fields are used only by the miner to construct the
/// shared attempt transcript and aux commitment.
#[derive(Clone)]
pub struct PearlMergeSubmissionConfig {
    pub header_template: PearlIncompleteBlockHeader,
    pub mining_config: PearlMiningConfig,
    pub aux_template: PearlNockchainAux,
    pub max_pattern_len: usize,
    pub mine_opts: PearlMergeMineOptions,
    pub certificate_builder: Arc<AiPowPearlMergeCertificateBuilder>,
}

#[derive(Debug, Error)]
#[error("AI-PoW recursive certificate build failed: {0}")]
pub struct AiPowCertificateBuildError(pub String);

#[derive(Debug, Error)]
pub enum AiPowCertificatePokeError {
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
    /// Pearl-format-compatible Nockchain submission configuration. Required:
    /// this is the only production submission path.
    pub pearl_merge: Option<PearlMergeSubmissionConfig>,
}

impl AiPuzzleInputs {
    /// Production node-mining preflight: do not spend matmul work unless the
    /// configured puzzle can be converted into the canonical recursive
    /// certificate accepted at the block boundary.
    pub fn validate_canonical_submission_ready(&self) -> Result<(), MinerError> {
        let Some(pearl) = self.pearl_merge.as_ref() else {
            return Err(MinerError::CanonicalCertificateUnavailable(
                "Pearl-format-compatible Nockchain submission requires Pearl merge submission config"
                    .to_string(),
            ));
        };
        validate_pearl_merge_config_for_recursive_prover(
            &pearl.mining_config,
            &self.params,
            pearl.max_pattern_len,
        )
        .map_err(|e| {
            MinerError::CanonicalCertificateUnavailable(format!(
                "configured Pearl merge AI-PoW params/config cannot produce a canonical recursive certificate: {e}"
            ))
        })?;
        pearl.aux_template.to_bytes().map_err(|e| {
            MinerError::CanonicalCertificateUnavailable(format!(
                "Pearl aux template is not canonical: {e}"
            ))
        })?;
        Ok(())
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
        Self {
            node_addr,
            mining_configs: default_v0_configs(),
            mining_pkh_configs,
            puzzle,
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
        let mut worker: Option<MiningWorker> = None;
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
                    if let Some(w) = worker.take() {
                        w.cancel();
                        if let Err(e) = w.await_join().await {
                            debug!(error = %e, "prior worker join error (ignored)");
                        }
                    }
                    let cancel = MiningCancel::new();
                    let pearl_job = match derive_pearl_merge_job_inputs(&cfg, &candidate) {
                        Ok(x) => x,
                        Err(e) => {
                            warn!(error = %e, "could not derive Pearl merge job inputs from candidate; skipping");
                            continue;
                        }
                    };
                    info!(pow_len = candidate.pow_len, "new candidate; dispatching Pearl-compatible ai-pow attempt");
                    let h = spawn_pearl_merge_attempt(&cfg, pearl_job, cancel.clone());
                    worker = Some(MiningWorker::PearlMerge { handle: h, cancel });
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
                        WorkerOutcome::PearlJoined(Ok(Ok(mined))) => {
                            info!(
                                matmul_attempts = mined.ticket.stats.matmul_attempts_tried,
                                elapsed_s = mined.ticket.stats.elapsed.as_secs_f64(),
                                matmul_attempt_rate = mined.ticket.stats.matmul_attempt_rate_per_sec(),
                                "ai-pow-miner: Pearl-compatible solution found; submitting Nockchain %ai-pow"
                            );
                            let Some(pearl_cfg) = cfg.puzzle.pearl_merge.as_ref() else {
                                break InnerOutcome::Fatal(MinerError::CertificateBuild(
                                    "Pearl merge solution found without Pearl config".to_string(),
                                ));
                            };
                            let proof = match (pearl_cfg.certificate_builder)(&mined.ticket.attempt) {
                                Ok(proof) => proof,
                                Err(e) => {
                                    warn!(error = %e, "Pearl-compatible recursive AI-PoW certificate build failed");
                                    break InnerOutcome::Fatal(MinerError::CertificateBuild(e.to_string()));
                                }
                            };
                            let poke = match build_ai_pow_pearl_merge_certificate_poke_from_ticket_proof(
                                &mined.ticket.attempt,
                                &mined.aux_inclusion,
                                &cfg.puzzle.a,
                                &cfg.puzzle.b,
                                pearl_cfg.max_pattern_len,
                                &proof,
                            ) {
                                Ok(poke) => poke,
                                Err(e) => {
                                    warn!(error = %e, "canonical Pearl-compatible AI-PoW certificate poke build failed");
                                    break InnerOutcome::Fatal(MinerError::CertificateBuild(e.to_string()));
                                }
                            };
                            if let Err(e) = client
                                .poke_wire(AiPowMinerWire::Mined.to_wire(), poke)
                                .await
                            {
                                warn!(error = %e, "submit Pearl-compatible ai-pow certificate poke failed (likely stale candidate)");
                            }
                        }
                        WorkerOutcome::PearlJoined(Ok(Err(PearlMergeMiningError::Cancelled))) => {
                            debug!("Pearl-compatible worker cancelled (expected on candidate supersede / shutdown)");
                        }
                        WorkerOutcome::PearlJoined(Ok(Err(e))) => {
                            warn!(error = %e, "Pearl-compatible ai-pow attempt terminated without solution");
                        }
                        WorkerOutcome::PearlJoined(Err(e)) => {
                            break InnerOutcome::Fatal(MinerError::WorkerJoin(format!("{e}")));
                        }
                    }
                }
            }
        };

        // ── cleanup before reconnect or exit ──
        if let Some(w) = worker.take() {
            w.cancel();
            let _ = w.await_join().await;
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

enum MiningWorker {
    PearlMerge {
        handle: JoinHandle<Result<PearlMergeMinedSubmission, PearlMergeMiningError>>,
        cancel: MiningCancel,
    },
}

impl MiningWorker {
    fn cancel(&self) {
        match self {
            MiningWorker::PearlMerge { cancel, .. } => {
                cancel.cancel();
            }
        }
    }

    async fn await_join(self) -> Result<(), tokio::task::JoinError> {
        match self {
            MiningWorker::PearlMerge { handle, .. } => handle.await.map(|_| ()),
        }
    }
}

enum WorkerOutcome {
    /// No worker was running; the future returned immediately.
    None,
    /// Worker joined: outer Result = tokio JoinError, inner = mining result.
    PearlJoined(
        Result<Result<PearlMergeMinedSubmission, PearlMergeMiningError>, tokio::task::JoinError>,
    ),
}

/// Helper to make `tokio::select!` work over an `Option<JoinHandle>`.
/// If the slot is empty, returns `WorkerOutcome::None` immediately
/// (caller pauses to avoid a busy-loop). If the slot has a handle,
/// awaits it (drops it on join). Mutates `worker` in place so the
/// caller doesn't need to thread the take/put.
async fn await_worker(worker: &mut Option<MiningWorker>) -> WorkerOutcome {
    match worker.take() {
        Some(MiningWorker::PearlMerge { handle, .. }) => WorkerOutcome::PearlJoined(handle.await),
        None => WorkerOutcome::None,
    }
}

/// Derive the per-candidate job inputs the AI-PoW prover needs:
/// the 32-byte chain difficulty target and Nockchain block commitment.
///
/// **`nck_commitment`** is `BLAKE3(jam(candidate.block_header))`, where
/// `candidate.block_header` is the kernel-emitted `block-commitment:page:t`
/// noun. The field name is inherited from the shared ZK-miner substrate; for
/// AI-PoW this is a commitment noun, not a raw block header. Hashing its
/// canonical jam gives the 32-byte value carried in the Rust-owned `AIP1`
/// nonce's Nockchain aux commitment. That Hoon commitment is the same mining
/// surface used by zk-pow: it binds the parent block id, tx-id set, coinbase
/// split, timestamp, epoch counter, target, accumulated work, height, and page
/// message before the PoW artifact is installed.
///
/// **`target`** is decoded from the kernel-side bignum noun
/// `[%bn limbs]`, where `limbs` are little-endian u32 chunks. The
/// ai-pow primitive compares BLAKE3 attempt hashes as 256-bit
/// little-endian integers, so bignum values above `2^256 - 1`
/// saturate to `FF..FF`.
fn derive_job_inputs(candidate: &MiningCandidate) -> Result<(DifficultyTarget, [u8; 32]), String> {
    // Hash the jammed block_header to a 32-byte commitment.
    let header_bytes = candidate.block_header.jam();
    let nck = *blake3::hash(&header_bytes).as_bytes();
    let target = decode_chain_target_bignum(&candidate.target)?;
    Ok((target, nck))
}

struct PearlMergeCandidateJob {
    header: PearlIncompleteBlockHeader,
    aux_inclusion: PearlAuxInclusionProof,
    target: DifficultyTarget,
    aux: PearlNockchainAux,
}

struct PearlMergeMinedSubmission {
    ticket: PearlMergeMinedTicket,
    aux_inclusion: PearlAuxInclusionProof,
}

fn derive_pearl_merge_job_inputs(
    cfg: &MinerConfig,
    candidate: &MiningCandidate,
) -> Result<PearlMergeCandidateJob, String> {
    let (target, nck_commitment) = derive_job_inputs(candidate)?;
    let pearl = cfg
        .puzzle
        .pearl_merge
        .as_ref()
        .ok_or_else(|| "missing Pearl merge submission config".to_string())?;
    let mut aux = pearl.aux_template.clone();
    aux.nock_block_commitment = nck_commitment;
    let (header, aux_inclusion) =
        build_coinbase_only_pearl_aux_inclusion(&pearl.header_template, &aux)
            .map_err(|e| format!("build Pearl aux inclusion: {e}"))?;
    Ok(PearlMergeCandidateJob {
        header,
        aux_inclusion,
        target,
        aux,
    })
}

fn build_coinbase_only_pearl_aux_inclusion(
    header_template: &PearlIncompleteBlockHeader,
    aux: &PearlNockchainAux,
) -> Result<(PearlIncompleteBlockHeader, PearlAuxInclusionProof), CertificateNounError> {
    let aux_commitment = aux.commitment()?;
    let coinbase_tx = build_coinbase_only_pearl_aux_tx(&aux_commitment);
    let mut merkle_root = pearl_bitcoin_double_sha256_raw(&coinbase_tx);
    merkle_root.reverse();
    let mut header = header_template.clone();
    header.merkle_root = merkle_root;
    Ok((
        header,
        PearlAuxInclusionProof {
            coinbase_tx,
            merkle_branch: Vec::new(),
        },
    ))
}

fn build_coinbase_only_pearl_aux_tx(aux_commitment: &[u8; 32]) -> Vec<u8> {
    let mut script = Vec::from([0x01, 0x00]);
    script.extend_from_slice(PEARL_NOCKCHAIN_AUX_COMMITMENT_TAG);
    script.extend_from_slice(aux_commitment);

    let mut tx = Vec::new();
    tx.extend_from_slice(&1u32.to_le_bytes());
    tx.push(1);
    tx.extend_from_slice(&[0u8; 32]);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(script.len() as u8);
    tx.extend_from_slice(&script);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(1);
    tx.extend_from_slice(&0u64.to_le_bytes());
    tx.push(1);
    tx.push(0x51);
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx
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

/// Spawn the Pearl-compatible ticket worker. This evaluates ticket attempts
/// only; recursive proof construction happens in the async submission path
/// after the worker returns a Nockchain-target hit.
fn spawn_pearl_merge_attempt(
    cfg: &MinerConfig,
    job_inputs: PearlMergeCandidateJob,
    cancel: MiningCancel,
) -> JoinHandle<Result<PearlMergeMinedSubmission, PearlMergeMiningError>> {
    let params = cfg.puzzle.params;
    let a = cfg.puzzle.a.clone();
    let b = cfg.puzzle.b.clone();
    let pearl = cfg
        .puzzle
        .pearl_merge
        .as_ref()
        .expect("Pearl merge config prechecked")
        .clone();
    tokio::task::spawn_blocking(move || {
        let job = PearlMergeMiningJob {
            header: &job_inputs.header,
            config: &pearl.mining_config,
            params: &params,
            nockchain_target: job_inputs.target,
            a: &a,
            b: &b,
            max_pattern_len: pearl.max_pattern_len,
            aux: job_inputs.aux,
        };
        let ticket = pearl_mining::run(&job, &pearl.mine_opts, cancel)?;
        Ok(PearlMergeMinedSubmission {
            ticket,
            aux_inclusion: job_inputs.aux_inclusion,
        })
    })
}

/// Internal wrapper for a prebuilt Pearl-format-compatible `%ai-pow` artifact:
///
/// ```hoon
/// [%command %pow %ai-pow nonce cert]
/// ```
///
/// `artifact` must already be the Hoon-compatible `%ai-pow` artifact:
///
/// ```hoon
/// [%ai-pow nonce=ai-pow-nonce cert=ai-pow-certificate]
/// ```
///
/// The helper is crate-internal so external callers cannot bypass the
/// recursive-run construction path by handing in an arbitrary prebuilt artifact.
/// It decodes the artifact tag, opaque nonce, and certificate metadata before
/// wrapping it. It deliberately does not traverse the recursive proof-node tail;
/// ticket-derived helpers construct that tail from typed recursive proof data,
/// and consensus verification performs proof-node traversal only after cheap
/// statement checks pass.
pub(crate) fn build_ai_pow_pearl_merge_certificate_poke(
    artifact: &NounSlab,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    decode_ai_pow_pearl_merge_artifact_metadata_slab(artifact, CertificateNounLimits::default())?;

    let artifact_space = artifact.noun_space();
    let mut slab = NounSlab::new();
    let artifact = slab.copy_into(unsafe { *artifact.root() }, &artifact_space);
    let payload = T(&mut slab, &[D(tas!(b"command")), D(tas!(b"pow")), artifact]);
    slab.set_root(payload);
    Ok(slab)
}

/// Test-only poke builder from an already-serialized recursive proof node.
///
/// Production callers use
/// [`build_ai_pow_pearl_merge_certificate_poke_from_ticket_recursive_run`].
#[cfg(test)]
pub(crate) fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(
    attempt: &PearlMergeTicketAttempt,
    aux_inclusion: &PearlAuxInclusionProof,
    a_row_major: &[i8],
    b_col_major: &[i8],
    max_pattern_len: usize,
    certificate: &AiProofNode,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    let artifact =
        crate::certificate_noun::build_ai_pow_pearl_merge_artifact_noun_from_ticket_node(
            attempt, aux_inclusion, a_row_major, b_col_major, max_pattern_len, certificate,
        )?;
    build_ai_pow_pearl_merge_certificate_poke(&artifact)
}

/// Crate-internal poke builder for the run loop after its certificate builder
/// has produced private-field [`PearlMergeCertificateProof`] data. Tests use it
/// with synthetic proof nodes; external callers cannot construct that wrapper
/// except through [`PearlMergeCertificateProof::from_recursive_run`].
#[cfg(test)]
pub(crate) fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_public_inputs_node(
    attempt: &PearlMergeTicketAttempt,
    aux_inclusion: &PearlAuxInclusionProof,
    a_row_major: &[i8],
    b_col_major: &[i8],
    max_pattern_len: usize,
    public_inputs: &CompositePublicInputs,
    certificate: &AiProofNode,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    let artifact = build_ai_pow_pearl_merge_artifact_noun_from_ticket_public_inputs_node(
        attempt, aux_inclusion, a_row_major, b_col_major, max_pattern_len, public_inputs,
        certificate,
    )?;
    build_ai_pow_pearl_merge_certificate_poke(&artifact)
}

/// Crate-internal production handoff for the run loop.
///
/// The ticket-derived statement metadata is recomputed from the candidate,
/// trusted matrices, and aux inclusion. The recursive-run metadata copied into
/// `proof` must match that recomputation before the proof node is serialized
/// into a command. This catches wrong-ticket or stale-run builders before the
/// node receives a doomed block proof.
pub(crate) fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_proof(
    attempt: &PearlMergeTicketAttempt,
    aux_inclusion: &PearlAuxInclusionProof,
    a_row_major: &[i8],
    b_col_major: &[i8],
    max_pattern_len: usize,
    proof: &PearlMergeCertificateProof,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    let artifact = build_ai_pow_pearl_merge_artifact_noun_from_ticket_public_inputs_node(
        attempt, aux_inclusion, a_row_major, b_col_major, max_pattern_len, &proof.public_inputs,
        &proof.certificate,
    )?;
    let decoded = decode_ai_pow_pearl_merge_artifact_metadata_slab(
        &artifact,
        CertificateNounLimits::default(),
    )?;
    if decoded.certificate.zk_params != proof.zk_params {
        return Err(AiPowCertificatePokeError::PearlMergeArtifact(
            CertificateNounError::PearlMergePublicInputMismatch("recursive-run.zk-params"),
        ));
    }
    if decoded.certificate.found_idx != proof.found_idx {
        return Err(AiPowCertificatePokeError::PearlMergeArtifact(
            CertificateNounError::PearlMergePublicInputMismatch("recursive-run.found-idx"),
        ));
    }
    if decoded.certificate.trace_height != proof.trace_height {
        return Err(AiPowCertificatePokeError::PearlMergeArtifact(
            CertificateNounError::PearlMergePublicInputMismatch("recursive-run.trace-height"),
        ));
    }
    if decoded.certificate.commitments != proof.commitments {
        return Err(AiPowCertificatePokeError::PearlMergeArtifact(
            CertificateNounError::PearlMergePublicInputMismatch("recursive-run.commitments"),
        ));
    }
    build_ai_pow_pearl_merge_certificate_poke(&artifact)
}

/// Build the production Pearl-format-compatible Nockchain consensus poke from
/// a successful shared ticket and the matching real recursive prover run.
pub fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_recursive_run(
    attempt: &PearlMergeTicketAttempt,
    aux_inclusion: &PearlAuxInclusionProof,
    a_row_major: &[i8],
    b_col_major: &[i8],
    max_pattern_len: usize,
    run: &AiPowRecursiveCertificateRun,
) -> Result<NounSlab, AiPowCertificatePokeError> {
    let artifact = build_ai_pow_pearl_merge_artifact_noun_from_ticket_recursive_run(
        attempt, aux_inclusion, a_row_major, b_col_major, max_pattern_len, run,
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
        evaluate_pearl_merge_ticket_attempt, verify_pearl_aux_inclusion,
        PearlIncompleteBlockHeader, PearlMiningConfig, PearlNockchainAux, PearlPeriodicPattern,
        PEARL_MINING_CONFIG_RESERVED_SIZE, PEARL_MMA_INT7XINT7_TO_INT32,
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
        build_ai_pow_pearl_merge_artifact_noun_from_node, decode_ai_pow_pearl_merge_artifact_noun,
        pearl_merge_recursive_certificate_parts_from_ticket,
        pearl_merge_recursive_public_inputs_from_work, AiProofNode, PearlMergePublicStatementShape,
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
        fn publish_synth_mine_effect(&self, commitment_seed: u64, target_seed: u64, pow_len: u64) {
            self.publish_synth_mine_effect_with_target_limbs(
                commitment_seed,
                &[target_seed],
                pow_len,
            );
        }

        fn publish_synth_mine_effect_with_target_limbs(
            &self,
            commitment_seed: u64,
            target_limbs: &[u64],
            pow_len: u64,
        ) {
            let mut slab = NounSlab::new();
            let head = D(tas!(b"mine-ai"));
            let version = D(0);
            let commit_source = synth_block_commitment_slab(commitment_seed);
            let commit_space = commit_source.noun_space();
            let commit = slab.copy_into(unsafe { *commit_source.root() }, &commit_space);
            let mut target_list = D(0);
            for limb in target_limbs.iter().rev() {
                target_list = T(&mut slab, &[D(*limb), target_list]);
            }
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
            m: 8,
            k: 1024,
            n: 8,
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

    fn pearl_submission_cfg() -> PearlMergeSubmissionConfig {
        PearlMergeSubmissionConfig {
            header_template: pearl_test_header(),
            mining_config: pearl_test_config(),
            aux_template: pearl_test_aux(),
            max_pattern_len: 16,
            mine_opts: PearlMergeMineOptions {
                max_attempts: Some(1),
                ..PearlMergeMineOptions::default()
            },
            certificate_builder: Arc::new(|attempt: &PearlMergeTicketAttempt| {
                let params = pearl_test_params();
                let (a, b) = synth_matrices(b"pearl-node-run-submit", &params);
                let parts =
                    pearl_merge_recursive_certificate_parts_from_ticket(attempt, &a, &b, 16)
                        .map_err(|e| AiPowCertificateBuildError(e.to_string()))?;
                Ok(PearlMergeCertificateProof {
                    zk_params: parts.zk_params,
                    found_idx: parts.found_idx,
                    commitments: parts.commitments,
                    public_inputs: parts.public_inputs,
                    trace_height: parts.trace_height,
                    certificate: AiProofNode::Unit,
                })
            }),
        }
    }

    fn pearl_test_coinbase_tx(aux_commitment: &[u8; 32]) -> Vec<u8> {
        let mut script = Vec::from([0x01, 0x00]);
        script.extend_from_slice(ai_pow::pearl_compat::PEARL_NOCKCHAIN_AUX_COMMITMENT_TAG);
        script.extend_from_slice(aux_commitment);
        let mut tx = Vec::new();
        tx.extend_from_slice(&1u32.to_le_bytes());
        tx.push(1);
        tx.extend_from_slice(&[0u8; 32]);
        tx.extend_from_slice(&u32::MAX.to_le_bytes());
        tx.push(script.len() as u8);
        tx.extend_from_slice(&script);
        tx.extend_from_slice(&u32::MAX.to_le_bytes());
        tx.push(1);
        tx.extend_from_slice(&0u64.to_le_bytes());
        tx.push(1);
        tx.push(0x51);
        tx.extend_from_slice(&0u32.to_le_bytes());
        tx
    }

    fn pearl_test_aux_inclusion(
        aux_commitment: &[u8; 32],
    ) -> (PearlIncompleteBlockHeader, PearlAuxInclusionProof) {
        let coinbase_tx = pearl_test_coinbase_tx(aux_commitment);
        let mut merkle_root = ai_pow::pearl_compat::pearl_bitcoin_double_sha256_raw(&coinbase_tx);
        merkle_root.reverse();
        let mut header = pearl_test_header();
        header.merkle_root = merkle_root;
        (
            header,
            PearlAuxInclusionProof {
                coinbase_tx,
                merkle_branch: Vec::new(),
            },
        )
    }

    fn test_cfg(node_addr: String) -> MinerConfig {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-node-run-submit", &params);
        let puzzle = AiPuzzleInputs {
            puzzle_id: b"ai-pow-node-run-test-pid".to_vec(),
            params,
            a: Arc::new(a),
            b: Arc::new(b),
            prover_opts: ProverOptions::default(),
            pearl_merge: Some(pearl_submission_cfg()),
        };
        MinerConfig {
            node_addr,
            mining_configs: default_v0_configs(),
            mining_pkh_configs: vec![MiningPkhConfig {
                share: 1,
                pkh: "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV".to_string(),
            }],
            puzzle,
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

    fn synth_block_commitment_slab(commitment_seed: u64) -> NounSlab {
        let mut slab = NounSlab::new();
        let commit = T(
            &mut slab,
            &[
                D(commitment_seed),
                D(commitment_seed + 1),
                D(commitment_seed + 2),
                D(commitment_seed + 3),
                D(commitment_seed + 4),
            ],
        );
        slab.set_root(commit);
        slab
    }

    fn candidate_for_target_and_commitment(
        target: NounSlab,
        commitment_seed: u64,
    ) -> MiningCandidate {
        let mut version = NounSlab::new();
        version.set_root(D(0));
        let block_header = synth_block_commitment_slab(commitment_seed);
        MiningCandidate {
            version,
            block_header,
            target,
            pow_len: 64,
        }
    }

    fn candidate_for_target(target: NounSlab) -> MiningCandidate {
        candidate_for_target_and_commitment(target, 0xCAFE)
    }

    fn expected_aux_commitment_bridge(candidate: &MiningCandidate) -> [u8; 32] {
        *blake3::hash(&candidate.block_header.jam()).as_bytes()
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
    fn derive_pearl_merge_job_inputs_binds_aux_to_candidate_block_commitment() {
        let cfg = test_cfg("http://127.0.0.1:1".to_string());
        let candidate_a =
            candidate_for_target_and_commitment(bignum_target_slab(&[u64::from(u32::MAX)]), 0xCAFE);
        let candidate_b =
            candidate_for_target_and_commitment(bignum_target_slab(&[u64::from(u32::MAX)]), 0xCAFF);

        let job_a = derive_pearl_merge_job_inputs(&cfg, &candidate_a).expect("derive Pearl job A");
        let job_b = derive_pearl_merge_job_inputs(&cfg, &candidate_b).expect("derive Pearl job B");

        assert_eq!(
            job_a.aux.nock_block_commitment,
            expected_aux_commitment_bridge(&candidate_a)
        );
        assert_eq!(
            job_b.aux.nock_block_commitment,
            expected_aux_commitment_bridge(&candidate_b)
        );
        assert_ne!(
            job_a.aux.nock_block_commitment, job_b.aux.nock_block_commitment,
            "distinct kernel block commitments must produce distinct AIP1 aux bindings"
        );
        assert_ne!(
            job_a.aux.nock_block_commitment,
            pearl_test_aux().nock_block_commitment,
            "candidate commitment must replace the static aux template placeholder"
        );
    }

    #[test]
    fn derive_pearl_merge_job_inputs_builds_self_verifying_aux_inclusion() {
        let cfg = test_cfg("http://127.0.0.1:1".to_string());
        let candidate =
            candidate_for_target_and_commitment(bignum_target_slab(&[u64::from(u32::MAX)]), 0xD00D);

        let job = derive_pearl_merge_job_inputs(&cfg, &candidate).expect("derive Pearl job");
        let expected_aux_commitment = job.aux.commitment().expect("aux commitment");
        verify_pearl_aux_inclusion(&job.header, &expected_aux_commitment, &job.aux_inclusion)
            .expect("derived coinbase-only Pearl aux inclusion should verify");

        let mut stale_aux = job.aux.clone();
        stale_aux.nock_block_commitment = [0x99; 32];
        let stale_aux_commitment = stale_aux.commitment().expect("stale aux commitment");
        assert!(
            verify_pearl_aux_inclusion(&job.header, &stale_aux_commitment, &job.aux_inclusion)
                .is_err(),
            "aux inclusion must bind the candidate-derived Nockchain block commitment"
        );
    }

    #[test]
    fn derive_job_inputs_saturates_targets_above_u256() {
        let exact_u256_max = candidate_for_target(bignum_target_slab(&[u64::from(u32::MAX); 8]));
        let (target, _) = derive_job_inputs(&exact_u256_max).expect("derive max u256 target");
        assert_eq!(target, [0xFF; 32]);

        let mut first_overflowing_limb = vec![0u64; 9];
        first_overflowing_limb[8] = 1;
        let candidate = candidate_for_target(bignum_target_slab(&first_overflowing_limb));
        let (target, _) = derive_job_inputs(&candidate).expect("derive job inputs");
        assert_eq!(target, [0xFF; 32]);

        let mut later_overflowing_limb = vec![0u64; 10];
        later_overflowing_limb[9] = 0x8;
        let candidate = candidate_for_target(bignum_target_slab(&later_overflowing_limb));
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

        let mut wrong_tag_target = NounSlab::new();
        let limbs = T(&mut wrong_tag_target, &[D(1), D(0)]);
        let root = T(&mut wrong_tag_target, &[D(tas!(b"not-bn")), limbs]);
        wrong_tag_target.set_root(root);
        let err = derive_job_inputs(&candidate_for_target(wrong_tag_target))
            .expect_err("target with wrong tag is not a bignum");
        assert!(err.contains("%bn"), "unexpected error: {err}");

        let mut improper_list_target = NounSlab::new();
        let limbs = T(&mut improper_list_target, &[D(1), D(7)]);
        let root = T(&mut improper_list_target, &[D(tas!(b"bn")), limbs]);
        improper_list_target.set_root(root);
        let err = derive_job_inputs(&candidate_for_target(improper_list_target))
            .expect_err("target limbs must be a proper list");
        assert!(err.contains("proper list"), "unexpected error: {err}");

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
    fn production_preflight_accepts_configured_pearl_merge_submission() {
        let cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.validate_canonical_submission_ready().expect(
            "configured Pearl mode should mine ticket attempts before Nockchain submission",
        );
    }

    #[test]
    fn production_preflight_rejects_missing_pearl_merge_submission_config() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.pearl_merge = None;

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("node miner must not mine without Pearl merge submission config");
        assert!(
            err.to_string()
                .contains("requires Pearl merge submission config"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_preflight_rejects_pearl_merge_config_param_mismatch() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        let mut pearl = pearl_submission_cfg();
        pearl.mining_config.rank = 32;
        cfg.puzzle.pearl_merge = Some(pearl);

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("Pearl mode must reject mining configs that do not match AI params");
        assert!(
            err.to_string().contains("rank does not match"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_preflight_rejects_pearl_merge_unsupported_recursive_params() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.params.difficulty_bits = 1;

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("Pearl mode must reject unsupported recursive params before mining");
        assert!(
            err.to_string().contains("difficulty_bits must be 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_preflight_rejects_pearl_merge_multi_tile_recursive_params() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.params.m = 16;

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("Pearl mode must reject multi-tile recursive params before mining");
        assert!(
            err.to_string().contains("num_tiles must be 1"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_preflight_rejects_pearl_merge_unsupported_recursive_pattern() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        let mut pearl = pearl_submission_cfg();
        pearl.mining_config.rows_pattern =
            PearlPeriodicPattern::from_list(&[0, 1, 8, 9, 64, 65, 72, 73]).unwrap();
        cfg.puzzle.pearl_merge = Some(pearl);

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("Pearl mode must reject patterns outside the recursive prover subset");
        assert!(
            err.to_string()
                .contains("outside the current recursive prover subset"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_preflight_rejects_noncanonical_pearl_aux_template() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        let pearl = cfg
            .puzzle
            .pearl_merge
            .as_mut()
            .expect("test config has Pearl merge submission");
        pearl.aux_template.nockchain_chain_id.clear();

        let err = cfg
            .puzzle
            .validate_canonical_submission_ready()
            .expect_err("Pearl mode must reject noncanonical aux templates before mining");
        assert!(
            err.to_string()
                .contains("Nockchain aux chain id must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_has_kernel_command_shape() {
        let aux = PearlNockchainAux {
            nockchain_chain_id: b"nockchain-mainnet".to_vec(),
            nock_block_commitment: [0x42; 32],
            nockchain_target_epoch_or_height: 123_456,
            extra_domain_data: b"ai-pow-target-window".to_vec(),
        };
        let expected_aux_commitment = aux.commitment().expect("aux commitment");
        let (header, aux_inclusion) = pearl_test_aux_inclusion(&expected_aux_commitment);
        let statement = PearlMergePublicStatementShape {
            block_header: header.to_bytes(),
            public_data: [0x20; ai_pow::pearl_compat::PEARL_PUBLIC_PROOF_PARAMS_SIZE],
            expected_aux_commitment,
            aux,
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
            &aux_inclusion,
            &params,
            0,
            8_192,
            &commitments,
            &pis,
            &AiProofNode::Unit,
        )
        .expect("build ai-pow artifact");

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

        let ai_pow_noun = pow_cell.tail().noun();
        let ai_pow_cell = ai_pow_noun.in_space(&space).as_cell().expect("ai-pow cell");
        assert!(ai_pow_cell.head().eq_bytes("ai-pow"));

        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pow_noun,
            &space,
            CertificateNounLimits::default(),
        )
        .expect("decode wrapped pearl merge artifact");
        assert_eq!(decoded.statement, statement);
        assert_eq!(decoded.aux_inclusion, aux_inclusion);
        assert_eq!(decoded.certificate.zk_params, params);
        assert_eq!(decoded.certificate.commitments, commitments);
        assert_eq!(decoded.certificate.public_inputs, pis);
        assert_eq!(decoded.certificate.certificate, AiProofNode::Unit);
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_derives_artifact() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-ticket-poke", &params);
        let aux = pearl_test_aux();
        let (header, aux_inclusion) = pearl_test_aux_inclusion(&aux.commitment().unwrap());
        let attempt = evaluate_pearl_merge_ticket_attempt(
            &header,
            &pearl_test_config(),
            &params,
            0,
            0,
            &a,
            &b,
            &[0xff; 32],
            16,
            aux,
        )
        .expect("evaluate Pearl ticket");

        let poke = build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(
            &attempt,
            &aux_inclusion,
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
        let ai_pow_noun = pow_cell.tail().noun();
        let ai_pow_cell = ai_pow_noun.in_space(&space).as_cell().expect("ai-pow cell");
        assert!(ai_pow_cell.head().eq_bytes("ai-pow"));

        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pow_noun,
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
        assert_eq!(decoded.aux_inclusion, aux_inclusion);
        assert_eq!(decoded.certificate.certificate, AiProofNode::Unit);
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_from_ticket_preserves_proof_public_inputs() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-ticket-poke-proof-pis", &params);
        let aux = pearl_test_aux();
        let (header, aux_inclusion) = pearl_test_aux_inclusion(&aux.commitment().unwrap());
        let attempt = evaluate_pearl_merge_ticket_attempt(
            &header,
            &pearl_test_config(),
            &params,
            0,
            0,
            &a,
            &b,
            &[0xff; 32],
            16,
            aux,
        )
        .expect("evaluate Pearl ticket");
        let mut public_inputs =
            pearl_merge_recursive_public_inputs_from_work(&attempt.commitments, &attempt.ticket);
        public_inputs.cumsum = [5, -8, 13, -21];

        let poke = build_ai_pow_pearl_merge_certificate_poke_from_ticket_public_inputs_node(
            &attempt,
            &aux_inclusion,
            &a,
            &b,
            16,
            &public_inputs,
            &AiProofNode::Unit,
        )
        .expect("build pearl merge poke from ticket and proof public inputs");
        let space = poke.noun_space();
        let root = unsafe { *poke.root() };
        let command_cell = root.in_space(&space).as_cell().expect("poke cell");
        let pow_cell = command_cell
            .tail()
            .noun()
            .in_space(&space)
            .as_cell()
            .expect("pow cell");
        let ai_pow_noun = pow_cell.tail().noun();
        let ai_pow_cell = ai_pow_noun.in_space(&space).as_cell().expect("ai-pow cell");
        assert!(ai_pow_cell.head().eq_bytes("ai-pow"));
        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pow_noun,
            &space,
            CertificateNounLimits::default(),
        )
        .expect("decode wrapped pearl merge artifact");

        assert_eq!(decoded.certificate.public_inputs, public_inputs);
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_rejects_stale_recursive_run_metadata() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-ticket-poke-stale-run", &params);
        let aux = pearl_test_aux();
        let (header, aux_inclusion) = pearl_test_aux_inclusion(&aux.commitment().unwrap());
        let attempt = evaluate_pearl_merge_ticket_attempt(
            &header,
            &pearl_test_config(),
            &params,
            0,
            0,
            &a,
            &b,
            &[0xff; 32],
            16,
            aux,
        )
        .expect("evaluate Pearl ticket");
        let parts =
            pearl_merge_recursive_certificate_parts_from_ticket(&attempt, &a, &b, 16).unwrap();
        let stale = PearlMergeCertificateProof {
            zk_params: parts.zk_params,
            found_idx: parts.found_idx + 1,
            commitments: parts.commitments,
            public_inputs: parts.public_inputs.clone(),
            trace_height: parts.trace_height,
            certificate: AiProofNode::Unit,
        };

        let err = build_ai_pow_pearl_merge_certificate_poke_from_ticket_proof(
            &attempt, &aux_inclusion, &a, &b, 16, &stale,
        )
        .expect_err("stale recursive-run metadata must not be submitted");
        assert!(
            err.to_string().contains("recursive-run.found-idx"),
            "unexpected error: {err}"
        );

        let mut forged_public_inputs = parts.public_inputs.clone();
        forged_public_inputs.hash_jackpot[0] ^= 1;
        let forged = PearlMergeCertificateProof {
            zk_params: parts.zk_params,
            found_idx: parts.found_idx,
            commitments: parts.commitments,
            public_inputs: forged_public_inputs,
            trace_height: parts.trace_height,
            certificate: AiProofNode::Unit,
        };
        let err = build_ai_pow_pearl_merge_certificate_poke_from_ticket_proof(
            &attempt, &aux_inclusion, &a, &b, 16, &forged,
        )
        .expect_err("forged recursive-run public inputs must not be submitted");
        assert!(
            err.to_string().contains("public-inputs.hash-jackpot"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn build_ai_pow_pearl_merge_certificate_poke_rejects_stale_aux_inclusion() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-ticket-poke-stale-aux", &params);
        let aux = pearl_test_aux();
        let (header, _) = pearl_test_aux_inclusion(&aux.commitment().unwrap());
        let attempt = evaluate_pearl_merge_ticket_attempt(
            &header,
            &pearl_test_config(),
            &params,
            0,
            0,
            &a,
            &b,
            &[0xff; 32],
            16,
            aux,
        )
        .expect("evaluate Pearl ticket");
        let parts =
            pearl_merge_recursive_certificate_parts_from_ticket(&attempt, &a, &b, 16).unwrap();
        let proof = PearlMergeCertificateProof {
            zk_params: parts.zk_params,
            found_idx: parts.found_idx,
            commitments: parts.commitments,
            public_inputs: parts.public_inputs,
            trace_height: parts.trace_height,
            certificate: AiProofNode::Unit,
        };
        let (_, stale_aux_inclusion) = pearl_test_aux_inclusion(&[0x99; 32]);

        let err = build_ai_pow_pearl_merge_certificate_poke_from_ticket_proof(
            &attempt, &stale_aux_inclusion, &a, &b, 16, &proof,
        )
        .expect_err("stale aux inclusion must not be submitted");
        assert!(
            err.to_string().contains(
                "Pearl aux commitment tag is not present in the txid-committed coinbase script"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pearl_ticket_loop_output_builds_canonical_ai_pow_poke() {
        let params = pearl_test_params();
        let (a, b) = synth_matrices(b"pearl-run-loop-to-poke", &params);
        let config = pearl_test_config();
        let aux = pearl_test_aux();
        let (header, aux_inclusion) = pearl_test_aux_inclusion(&aux.commitment().unwrap());
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
            &aux_inclusion,
            &a,
            &b,
            job.max_pattern_len,
            &AiProofNode::Unit,
        )
        .expect("mined Pearl ticket should build ai-pow poke");
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
        let ai_pow_noun = pow_cell.tail().noun();
        let ai_pow_cell = ai_pow_noun.in_space(&space).as_cell().expect("ai-pow cell");
        assert!(ai_pow_cell.head().eq_bytes("ai-pow"));

        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pow_noun,
            &space,
            CertificateNounLimits::default(),
        )
        .expect("decode mined-ticket ai-pow artifact");
        assert_eq!(
            decoded.certificate.found_idx,
            mined.attempt.public_params.t_rows
        );
        assert_eq!(
            decoded.statement,
            PearlMergePublicStatementShape::from_wire_statement(&mined.attempt.statement)
                .expect("statement shape")
        );
        assert_eq!(decoded.aux_inclusion, aux_inclusion);
    }

    #[test]
    fn pearl_ticket_loop_miss_cannot_build_ai_pow_poke() {
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
        let aux = pearl_test_aux();
        let (header, aux_inclusion) = pearl_test_aux_inclusion(&aux.commitment().unwrap());
        let attempt = evaluate_pearl_merge_ticket_attempt(
            &header,
            &pearl_test_config(),
            &params,
            0,
            0,
            &a,
            &b,
            &[0xff; 32],
            16,
            aux,
        )
        .expect("evaluate Pearl ticket");
        a[0] ^= 1;

        assert!(matches!(
            build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(
                &attempt,
                &aux_inclusion,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_pearl_merge_submits_nockchain_ai_pow_after_ticket_hit() {
        let node = MockNode::spawn().await;
        let cfg = test_cfg(node.url());

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        let mining_task = tokio::spawn(async move { run(cfg, shutdown_clone).await });

        tokio::time::sleep(Duration::from_millis(300)).await;
        let header_seed = 700;
        node.publish_synth_mine_effect_with_target_limbs(
            header_seed,
            &[u64::from(u32::MAX); 8],
            64,
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let poke = loop {
            if let Some(poke) = node.mined_pokes.lock().await.pop() {
                break poke;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Pearl merge miner did not submit a %mined poke within 10s; observed {} total pokes",
                node.pokes_observed.load(Ordering::SeqCst)
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        let expected_nock_commitment =
            *blake3::hash(&synth_block_commitment_slab(header_seed).jam()).as_bytes();
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
        let ai_pow_noun = pow_cell.tail().noun();
        let decoded = decode_ai_pow_pearl_merge_artifact_noun(
            ai_pow_noun,
            &space,
            CertificateNounLimits::default(),
        )
        .expect("decode submitted Pearl-compatible ai-pow artifact");
        assert_eq!(
            decoded.statement.aux.nock_block_commitment,
            expected_nock_commitment
        );
        assert_eq!(decoded.aux_inclusion.merkle_branch.len(), 0);
        assert_eq!(decoded.certificate.certificate, AiProofNode::Unit);

        shutdown.cancel();
        let r = tokio::time::timeout(Duration::from_secs(5), mining_task)
            .await
            .expect("miner task did not exit")
            .expect("miner panicked");
        assert!(matches!(r, Ok(())), "unexpected miner result: {r:?}");
        node.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_rejects_stale_recursive_metadata_without_submitting_poke() {
        let node = MockNode::spawn().await;
        let mut cfg = test_cfg(node.url());
        let pearl_cfg = cfg
            .puzzle
            .pearl_merge
            .as_mut()
            .expect("test config has Pearl merge submission");
        pearl_cfg.certificate_builder = Arc::new(|attempt: &PearlMergeTicketAttempt| {
            let params = pearl_test_params();
            let (a, b) = synth_matrices(b"pearl-node-run-submit", &params);
            let parts = pearl_merge_recursive_certificate_parts_from_ticket(attempt, &a, &b, 16)
                .map_err(|e| AiPowCertificateBuildError(e.to_string()))?;
            Ok(PearlMergeCertificateProof {
                zk_params: parts.zk_params,
                found_idx: parts.found_idx + 1,
                commitments: parts.commitments,
                public_inputs: parts.public_inputs,
                trace_height: parts.trace_height,
                certificate: AiProofNode::Unit,
            })
        });

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        let mining_task = tokio::spawn(async move { run(cfg, shutdown_clone).await });

        tokio::time::sleep(Duration::from_millis(300)).await;
        node.publish_synth_mine_effect_with_target_limbs(701, &[u64::from(u32::MAX); 8], 64);

        let r = tokio::time::timeout(Duration::from_secs(10), mining_task)
            .await
            .expect("miner task did not exit")
            .expect("miner panicked");
        match r {
            Err(MinerError::CertificateBuild(msg)) => {
                assert!(
                    msg.contains("recursive-run.found-idx"),
                    "unexpected certificate build error: {msg}"
                );
            }
            other => panic!("expected stale recursive metadata to fail closed, got {other:?}"),
        }
        assert!(
            node.mined_pokes.lock().await.is_empty(),
            "stale recursive metadata must not be submitted to the node"
        );

        shutdown.cancel();
        node.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_miss_does_not_build_recursive_certificate_or_submit_poke() {
        let node = MockNode::spawn().await;
        let mut cfg = test_cfg(node.url());
        let builder_calls = Arc::new(AtomicU64::new(0));
        let builder_calls_for_cfg = builder_calls.clone();
        let pearl_cfg = cfg
            .puzzle
            .pearl_merge
            .as_mut()
            .expect("test config has Pearl merge submission");
        pearl_cfg.mine_opts = PearlMergeMineOptions {
            max_attempts: Some(1),
            ..PearlMergeMineOptions::default()
        };
        pearl_cfg.certificate_builder = Arc::new(move |_attempt: &PearlMergeTicketAttempt| {
            builder_calls_for_cfg.fetch_add(1, Ordering::SeqCst);
            Err(AiPowCertificateBuildError(
                "certificate builder must not be called on a target miss".to_string(),
            ))
        });

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        let mining_task = tokio::spawn(async move { run(cfg, shutdown_clone).await });

        tokio::time::sleep(Duration::from_millis(300)).await;
        node.publish_synth_mine_effect_with_target_limbs(702, &[0], 64);
        tokio::time::sleep(Duration::from_millis(700)).await;

        assert_eq!(
            builder_calls.load(Ordering::SeqCst),
            0,
            "recursive certificate builder must only run after a ticket target hit"
        );
        assert!(
            node.mined_pokes.lock().await.is_empty(),
            "target misses must not submit %ai-pow pokes"
        );

        shutdown.cancel();
        let r = tokio::time::timeout(Duration::from_secs(5), mining_task)
            .await
            .expect("miner task did not exit")
            .expect("miner panicked");
        assert!(matches!(r, Ok(())), "unexpected miner result: {r:?}");
        node.shutdown().await;
    }

    /// Heavy: runs the real ai-pow prover on TEST_SMALL with a trivial
    /// `FF..FF` target. Should complete in well under 30 s on any
    /// modern machine; marked `#[ignore]` so `cargo test` is fast by
    /// default. Run with `cargo test -p ai-pow-miner --features node
    /// --test node_run_mock_node -- --ignored`.
    #[ignore = "manual mock-node integration test; runs the real prover"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_against_mock_node_submits_ai_pow_command_when_recursive_cert_available() {
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
    async fn run_loop_rejects_before_connect_when_recursive_cert_unavailable() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.params.difficulty_bits = 1;
        let shutdown = CancellationToken::new();
        let r = tokio::time::timeout(Duration::from_secs(2), run(cfg, shutdown))
            .await
            .expect("run didn't terminate");
        match r {
            Err(MinerError::CanonicalCertificateUnavailable(msg)) => {
                assert!(
                    msg.contains("difficulty_bits must be 0"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("expected CanonicalCertificateUnavailable, got {other:?}"),
        }
    }

    /// Cheap: confirms shutdown does not turn the canonical-certificate
    /// preflight failure into a successful run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_shutdown_still_reports_unavailable_recursive_cert() {
        let mut cfg = test_cfg("http://127.0.0.1:1".to_string());
        cfg.puzzle.params.difficulty_bits = 1;
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
            "expected canonical recursive certificate to remain unavailable, got {r:?}"
        );
    }
}
