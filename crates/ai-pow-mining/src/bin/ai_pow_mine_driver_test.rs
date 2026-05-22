//! `ai-pow-mine-driver-test` — end-to-end smoke test for the
//! NockApp mining driver.
//!
//! Constructs a `NockAppHandle` from raw channel primitives (no
//! real Hoon kernel), spawns the ai-pow-mining driver against it,
//! pushes one TEST_SMALL / trivial-target job in, observes
//! the driver's `DriverEvent::Mined`, and acks the outbound
//! `[%mined ...]` poke as a stand-in kernel would.
//!
//! Exit code 0 on success, 1 on failure.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use ai_pow::synth::synth_matrices;
use ai_pow_mining::nockapp_driver::{
    create_driver, AiPowMiningWire, DriverConfig, DriverEvent, OwnedMiningJob,
};
use ai_pow_mining::{MineOptions, NonceAnchors};
use nockapp::nockapp::driver::{IOAction, NockAppHandle, PokeResult};
use nockapp::nockapp::metrics::NockAppMetrics;
use nockapp::nockapp::wire::Wire;
use nockapp::nockapp::NockAppExit;
use tokio::sync::{broadcast, mpsc, Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eprintln!("ai-pow-mine-driver-test: building NockAppHandle...");

    // ── Manual NockAppHandle assembly ────────────────────────────
    let (action_tx, mut action_rx) = mpsc::channel::<IOAction>(64);
    let (effect_tx, _effect_init_rx) = broadcast::channel::<nockapp::noun::slab::NounSlab>(64);
    let effect_tx = Arc::new(effect_tx);
    let effect_rx_for_handle = effect_tx.subscribe();
    let metrics = Arc::new(
        NockAppMetrics::register(gnort::global_metrics_registry())
            .expect("register NockAppMetrics"),
    );
    let (exit, _exit_rx) = NockAppExit::new();
    let handle = NockAppHandle {
        io_sender: action_tx,
        effect_sender: effect_tx,
        effect_receiver: Mutex::new(effect_rx_for_handle),
        metrics,
        exit,
    };

    // ── Stand-in kernel: drain pokes + ack ──────────────────────
    let pokes_observed = Arc::new(AtomicUsize::new(0));
    let pokes_observed_clone = pokes_observed.clone();
    let kernel_task = tokio::spawn(async move {
        while let Some(action) = action_rx.recv().await {
            match action {
                IOAction::Poke { wire, ack_channel, .. } => {
                    pokes_observed_clone.fetch_add(1, Ordering::SeqCst);
                    eprintln!(
                        "[kernel] poke received: source={} version={} (#{:})",
                        wire.source,
                        wire.version,
                        pokes_observed_clone.load(Ordering::SeqCst),
                    );
                    debug_assert_eq!(wire.source, <AiPowMiningWire as Wire>::SOURCE);
                    let _ = ack_channel.send(PokeResult::Ack);
                }
                IOAction::Peek { .. } => {
                    eprintln!("[kernel] unexpected peek (ignored)");
                }
            }
        }
        eprintln!("[kernel] io channel closed");
    });

    // ── Driver wiring ─────────────────────────────────────────────
    let (jobs_tx, jobs_rx) = mpsc::channel::<OwnedMiningJob>(4);
    let (events_tx, mut events_rx) = mpsc::channel::<DriverEvent>(32);
    let (init_tx, init_rx) = tokio::sync::oneshot::channel::<()>();

    let cfg = DriverConfig {
        mining_enabled: true,
        jobs_rx: Arc::new(Mutex::new(jobs_rx)),
        events_tx: Some(events_tx),
        init_complete_tx: Some(init_tx),
    };

    let driver_fn = create_driver(cfg);
    let driver_handle = tokio::spawn(driver_fn(handle));

    // Wait for the driver to signal init complete.
    init_rx.await.expect("driver init signal");
    eprintln!("ai-pow-mine-driver-test: driver started ✓");

    // ── Build + submit one job ─────────────────────────────
    let params = MatmulParams::TEST_SMALL;
    let (a, b) = synth_matrices(b"driver-test-seed", &params);
    let job = OwnedMiningJob {
        puzzle_id: b"driver-test-puzzle-id".to_vec(),
        anchors: NonceAnchors::nck_only([0xCD; 32]),
        params,
        target: [0xFFu8; 32], // trivial target ⇒ first extranonce wins
        a: Arc::new(a),
        b: Arc::new(b),
        opts: MineOptions::default(),
    };
    jobs_tx.send(job).await.expect("send job");
    eprintln!("ai-pow-mine-driver-test: job submitted ✓");

    // ── Wait for Mined event (timeout 30 s as a smoke ceiling) ───
    let mut got_mined: Option<(u32, u64, Duration)> = None;
    let collect = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(ev) = events_rx.recv().await {
            match ev {
                DriverEvent::Mined {
                    nonce, found_idx, extranonces_tried, elapsed,
                } => {
                    eprintln!(
                        "[event] Mined: tile_idx={} attempts={} elapsed={:?}",
                        found_idx, extranonces_tried, elapsed,
                    );
                    // Sanity: nonce parses cleanly.
                    let _ = ai_pow_mining::parse_ncmn_nonce(&nonce)
                        .expect("nonce parses");
                    got_mined = Some((found_idx, extranonces_tried, elapsed));
                    return Ok::<_, anyhow::Error>(());
                }
                DriverEvent::MiningError(msg) => {
                    anyhow::bail!("driver reported MiningError: {msg}");
                }
                other => eprintln!("[event] {:?}", other),
            }
        }
        anyhow::bail!("events channel closed before Mined");
    })
    .await;
    match collect {
        Ok(Ok(())) => {}
        Ok(Err(e)) => anyhow::bail!(e),
        Err(_) => anyhow::bail!("timeout waiting for Mined event"),
    }

    // ── Cleanup: close jobs channel ⇒ driver exits ──────────────
    drop(jobs_tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), driver_handle)
        .await
        .map_err(|_| anyhow::anyhow!("driver did not exit within 2s"))?
        .map_err(|e| anyhow::anyhow!("driver task panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!("driver returned error: {e}"))?;
    let _ = tokio::time::timeout(Duration::from_secs(1), kernel_task).await;

    let pokes = pokes_observed.load(Ordering::SeqCst);
    let (idx, attempts, elapsed) = got_mined.expect("got_mined set");
    eprintln!(
        "ai-pow-mine-driver-test: ✓ round-trip OK \
         (mined tile_idx={} attempts={} elapsed={:?} pokes_observed={})",
        idx, attempts, elapsed, pokes,
    );
    if pokes < 1 {
        anyhow::bail!("expected ≥1 poke to the kernel, observed {pokes}");
    }
    Ok(())
}
