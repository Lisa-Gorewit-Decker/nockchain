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
//!      - success → poke the node with `[%mined nonce found-idx]` on
//!        [`AiPowMinerWire::Mined`], then idle until the next candidate.
//!      - error → log + idle.
//! 5. Stream drop → outer loop reconnects.
//!
//! ## Note on submission
//! The current deployed Hoon kernel has **no AI-PoW verifier** —
//! `AiPowMinerWire::Mined` pokes will be NACK'd or ignored by the
//! kernel until the kernel-side AI puzzle handler lands. That's
//! by design for this session: the goal is to wire the substrate
//! end-to-end (gRPC connect / configure / watch / submit) so the
//! kernel-side integration has a working client to land against.

use std::sync::Arc;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use ai_pow::prover::ProverOptions;
use futures::StreamExt;
use nockapp::noun::slab::NounSlab;
use nockapp::nockapp::wire::Wire;
use nockchain_mining_common::{
    MiningCandidate, MiningKeyConfig, MiningPkhConfig, NodeClient,
};
use nockvm::noun::{Noun, D, T};
use nockvm_macros::tas;
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::mining;
use crate::wire::AiPowMinerWire;
use crate::{
    DifficultyTarget, MineOptions, MinedSolution, MiningCancel, MiningError as PuzzleMiningError,
    MiningJob, NonceAnchors,
};

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
}

/// Production entry point. Returns `Ok(())` on clean shutdown, `Err` on
/// unrecoverable failure.
pub async fn run(
    cfg: MinerConfig,
    shutdown: CancellationToken,
) -> Result<(), MinerError> {
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
            return Err(MinerError::Configure(format!(
                "enable_mining(true): {e}"
            )));
        }
        info!("ai-pow-miner: subscribed + mining enabled; awaiting candidates");

        // ── inner loop ──
        // `worker` is the currently-running spawn-blocking task (if
        // any). `cancel` is the AI-PoW MiningCancel handle for it.
        // On a new candidate we cancel the existing attempt + spawn
        // a fresh one. On shutdown we cancel + drain.
        let mut worker: Option<(JoinHandle<Result<MinedSolution, PuzzleMiningError>>, MiningCancel)> = None;
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
                                attempts = sol.stats.extranonces_tried,
                                elapsed_s = sol.stats.elapsed.as_secs_f64(),
                                rate = sol.stats.hash_rate_per_sec(),
                                "ai-pow-miner: solution found; submitting"
                            );
                            let poke = build_mined_poke(&sol.nonce, sol.found_idx);
                            if let Err(e) = client
                                .poke_wire(AiPowMinerWire::Mined.to_wire(), poke)
                                .await
                            {
                                warn!(error = %e, "submit_mined poke failed (likely stale candidate or no AI verifier yet)");
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
    worker: &mut Option<(JoinHandle<Result<MinedSolution, PuzzleMiningError>>, MiningCancel)>,
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
/// **`target`** is the low 32 bytes of `candidate.target` jammed.
/// The kernel-side `%mine` effect carries the target as a noun
/// `[%bn limbs]` (big-num); for the substrate v1 we just hash it
/// to a fixed-shape 32-byte bound. The exact little-endian
/// derivation is again local to this miner — the chain-side
/// puzzle pin will replace this with the consensus-correct
/// extraction in the same follow-up.
fn derive_job_inputs(
    candidate: &MiningCandidate,
) -> Result<(DifficultyTarget, NonceAnchors), String> {
    // Hash the jammed block_header to a 32-byte commitment.
    let header_bytes = candidate.block_header.jam();
    let nck = *blake3::hash(&header_bytes).as_bytes();
    // Hash the jammed target to a 32-byte target.
    let target_bytes = candidate.target.jam();
    let target = *blake3::hash(&target_bytes).as_bytes();
    Ok((target, NonceAnchors::nck_only(nck)))
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

/// Build the `[%mined nonce-atom found-idx]` poke shape (mirrors
/// `nockapp_driver::poke_mined`, just hoisted up so the gRPC path
/// can use the same noun shape as the in-process driver).
fn build_mined_poke(nonce: &[u8; crate::NCMN_NONCE_LEN], found_idx: u32) -> NounSlab {
    let mut slab = NounSlab::new();
    let nonce_atom = bytes_to_atom(&mut slab, nonce);
    let head = D(tas!(b"mined"));
    let payload: Noun = T(&mut slab, &[head, nonce_atom, D(found_idx as u64)]);
    slab.set_root(payload);
    slab
}

fn bytes_to_atom(slab: &mut NounSlab, bytes: &[u8]) -> Noun {
    use nockvm::noun::IndirectAtom;
    let atom = <IndirectAtom as nockapp::IndirectAtomExt>::from_bytes(slab, bytes);
    atom.as_noun()
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
    use ai_pow::synth::synth_matrices;
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
    use crate::wire::AiPowMinerWire;

    // Shared NockAppMetrics — gnort rejects double-registration.
    static METRICS: Lazy<Arc<nockapp::nockapp::metrics::NockAppMetrics>> = Lazy::new(|| {
        Arc::new(
            nockapp::nockapp::metrics::NockAppMetrics::register(
                gnort::global_metrics_registry(),
            )
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

    fn test_cfg(node_addr: String) -> MinerConfig {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"ai-pow-node-run-test", &params);
        let puzzle = AiPuzzleInputs {
            puzzle_id: b"ai-pow-node-run-test-pid".to_vec(),
            params,
            a: Arc::new(a),
            b: Arc::new(b),
            prover_opts: ProverOptions::default(),
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

    /// Heavy: runs the real ai-pow prover on TEST_SMALL with a trivial
    /// `FF..FF` target. Should complete in well under 30 s on any
    /// modern machine; marked `#[ignore]` so `cargo test` is fast by
    /// default. Run with `cargo test -p ai-pow-miner --features node
    /// --test node_run_mock_node -- --ignored`.
    #[ignore]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_against_mock_node_submits_mined() {
        let node = MockNode::spawn().await;
        let cfg = test_cfg(node.url());

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        let mining_task = tokio::spawn(async move { run(cfg, shutdown_clone).await });

        // Brief pause for the miner to connect + configure + subscribe.
        tokio::time::sleep(Duration::from_millis(300)).await;
        node.publish_synth_mine_effect(100, 0xFFFF_FFFF, 64);

        // Poll for the %mined poke. Allow up to 30 s for the trivial-target prover.
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

    /// Cheap: confirms reconnect-with-backoff terminates as
    /// `TooManyReconnects` when nothing is listening.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_loop_reconnects_when_node_unreachable_then_exits() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);
        let cfg = test_cfg(format!("http://{addr}"));
        let shutdown = CancellationToken::new();
        let r = tokio::time::timeout(Duration::from_secs(2), run(cfg, shutdown))
            .await
            .expect("run didn't terminate");
        match r {
            Err(MinerError::TooManyReconnects { count }) => {
                assert!(count >= 3, "expected at least 3 attempts, got {count}");
            }
            other => panic!("expected TooManyReconnects, got {other:?}"),
        }
    }

    /// Cheap: confirms clean shutdown drains any in-flight worker
    /// (without actually completing a heavy proof — we cancel
    /// quickly enough that the worker terminates on its first
    /// MiningCancel check).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_loop_exits_cleanly_on_shutdown() {
        let node = MockNode::spawn().await;
        // Use an impossible target so the worker keeps running until cancel.
        // (build_mined_poke etc. won't be called.)
        let mut cfg = test_cfg(node.url());
        cfg.mine_opts.max_extranonces = None;
        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        let mining_task = tokio::spawn(async move { run(cfg, shutdown_clone).await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Push a candidate so the worker dispatches.
        node.publish_synth_mine_effect(50, 0x0000_0001, 64);
        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.cancel();
        let r = tokio::time::timeout(Duration::from_secs(10), mining_task)
            .await
            .expect("miner did not exit within 10s")
            .expect("miner panicked");
        assert!(r.is_ok(), "expected clean Ok on shutdown, got {r:?}");
        node.shutdown().await;
    }
}
