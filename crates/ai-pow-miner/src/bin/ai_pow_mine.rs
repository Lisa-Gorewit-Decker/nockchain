//! `ai-pow-mine` — standalone AI-PoW (matmul puzzle) block miner.
//!
//! Mirrors `zk-pow-mine` in shape: connects to a `nockchain` node's
//! private NockAppService gRPC, subscribes to `%mine` candidate
//! effects, runs the AI-PoW prover, and submits
//! `[%command %pow %ai-pow cert]` on the `AiPowMinerWire::Mined` wire
//! (`SOURCE = "ai-pow-miner"`, `VERSION = 1`) when a recursive
//! certificate builder is configured.
//!
//! Quick start (assuming a fakenet node on `127.0.0.1:5555`):
//!
//!   ai-pow-mine \
//!       --node-addr http://127.0.0.1:5555 \
//!       --mining-pkh 9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV \
//!       --synth-seed ai-pow-prod-v1
//!
//! ## AI puzzle inputs (local config)
//! The chain's `%mine` effect carries only the block header + target +
//! pow-len. The AI puzzle additionally needs `puzzle_id` + matmul
//! `params` + matrices `a` / `b`. For now these come from CLI config
//! (operator-supplied or synth-derived); a future chain-AI integration
//! may derive them from chain state (layer/epoch). The substrate is
//! structured so the run loop is unchanged when that swap lands.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use ai_pow::prover::ProverOptions;
use ai_pow::zk_bridge::prove_ai_pow_recursive_certificate;
use ai_pow_miner::certificate_noun::build_ai_pow_certificate_noun;
use ai_pow_miner::run::{
    default_v0_configs, run, AiPowCertificateBuildError, AiPuzzleInputs, MinerConfig, MinerError,
};
use ai_pow_miner::MineOptions;
use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use nockchain_mining_common::MiningPkhConfig;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::{fmt, EnvFilter};

/// `ai-pow-mine` — standalone AI-PoW block miner.
#[derive(Parser, Debug)]
#[command(
    name = "ai-pow-mine",
    about = "Standalone AI-PoW block miner. Subscribes to a nockchain node's %mine effects via gRPC and submits AI-puzzle solutions back.",
    version
)]
struct Args {
    /// Node's private gRPC URL.
    #[arg(long, default_value = "http://127.0.0.1:5555")]
    node_addr: String,

    /// Single-recipient mining pubkey hash. Mutually exclusive with --mining-pkh-adv.
    #[arg(long, conflicts_with = "mining_pkh_adv")]
    mining_pkh: Option<String>,

    /// Multi-recipient mining pkh configs. Each entry is `share,pkh`.
    #[arg(long, value_parser = clap::value_parser!(MiningPkhConfig), num_args = 1..)]
    mining_pkh_adv: Option<Vec<MiningPkhConfig>>,

    // ── AI puzzle config ───────────────────────────────────────────
    /// Stable puzzle id bound into κ (32-byte hex; defaults to BLAKE3
    /// of the matmul params if omitted).
    #[arg(long)]
    puzzle_id: Option<String>,

    /// Matmul puzzle shape (defaults to TEST_SMALL: m=k=n=64, r=4, t=8, σ=8).
    #[arg(short = 'm', long, default_value_t = 64)]
    m: u32,
    #[arg(short = 'k', long, default_value_t = 64)]
    k: u32,
    #[arg(short = 'n', long, default_value_t = 64)]
    n: u32,
    #[arg(long, default_value_t = 4)]
    noise_rank: u32,
    #[arg(long, default_value_t = 8)]
    tile: u32,
    #[arg(long, default_value_t = 8)]
    spot_checks: u32,
    #[arg(long, default_value_t = 0)]
    difficulty_bits: u32,

    /// Path to raw i8 matrix A (length m·k). Mutually exclusive with --synth-seed.
    #[arg(long, value_name = "PATH", conflicts_with = "synth_seed")]
    a: Option<PathBuf>,
    /// Path to raw i8 matrix B (length k·n).
    #[arg(long, value_name = "PATH", conflicts_with = "synth_seed")]
    b: Option<PathBuf>,
    /// Synthesize A + B deterministically from this seed string.
    #[arg(long)]
    synth_seed: Option<String>,

    /// Stop the per-attempt extranonce loop after this many tries
    /// (None ⇒ unbounded). Useful for testing.
    #[arg(long)]
    max_extranonces: Option<u64>,

    // ── reconnect tuning ───────────────────────────────────────────
    /// Initial reconnect backoff in milliseconds.
    #[arg(long, default_value = "1000")]
    reconnect_backoff_initial_ms: u64,

    /// Maximum reconnect backoff in milliseconds (cap).
    #[arg(long, default_value = "30000")]
    reconnect_backoff_max_ms: u64,

    /// Consecutive reconnect attempts before giving up.
    #[arg(long, default_value = "5")]
    reconnect_max_attempts: u32,

    /// Log filter (env-filter syntax). Override with the `RUST_LOG` env var.
    #[arg(
        long,
        default_value = "info,ai_pow_miner=info,nockchain_mining_common=info"
    )]
    log: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    init_tracing(&args.log);

    let Some(pkh_configs) = build_pkh_configs(&args) else {
        eprintln!("ai-pow-mine: must supply --mining-pkh <HASH> or --mining-pkh-adv \"share,pkh\"");
        return ExitCode::from(1);
    };

    let puzzle = match build_puzzle_inputs(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ai-pow-mine: invalid puzzle config: {e:#}");
            return ExitCode::from(1);
        }
    };

    let mut mine_opts = MineOptions::default();
    mine_opts.prover = puzzle.prover_opts;
    mine_opts.max_extranonces = args.max_extranonces;

    let cfg = MinerConfig {
        node_addr: args.node_addr,
        mining_configs: default_v0_configs(),
        mining_pkh_configs: pkh_configs,
        puzzle,
        mine_opts,
        reconnect_backoff_initial: Duration::from_millis(args.reconnect_backoff_initial_ms),
        reconnect_backoff_max: Duration::from_millis(args.reconnect_backoff_max_ms),
        reconnect_max_attempts: args.reconnect_max_attempts,
    };

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ai-pow-mine: failed to build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let r: Result<(), MinerError> = rt.block_on(async {
        info!(
            node = %cfg.node_addr,
            puzzle_m = cfg.puzzle.params.m,
            puzzle_k = cfg.puzzle.params.k,
            puzzle_n = cfg.puzzle.params.n,
            "ai-pow-mine: starting"
        );
        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("ai-pow-mine: SIGINT received; shutting down");
                shutdown_clone.cancel();
            }
        });
        run(cfg, shutdown).await
    });

    match r {
        Ok(()) => {
            info!("ai-pow-mine: clean shutdown");
            ExitCode::from(0)
        }
        Err(MinerError::TooManyReconnects { count }) => {
            error!("ai-pow-mine: gave up after {count} consecutive reconnect failures");
            ExitCode::from(2)
        }
        Err(e) => {
            error!(error = %e, "ai-pow-mine: unrecoverable error");
            ExitCode::from(1)
        }
    }
}

fn init_tracing(filter: &str) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));
    let _ = fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

fn build_pkh_configs(args: &Args) -> Option<Vec<MiningPkhConfig>> {
    if let Some(pkh) = &args.mining_pkh {
        Some(vec![MiningPkhConfig {
            share: 1,
            pkh: pkh.clone(),
        }])
    } else {
        args.mining_pkh_adv.clone()
    }
}

fn build_puzzle_inputs(args: &Args) -> Result<AiPuzzleInputs> {
    let params = MatmulParams {
        m: args.m,
        k: args.k,
        n: args.n,
        noise_rank: args.noise_rank,
        tile: args.tile,
        spot_checks: args.spot_checks,
        difficulty_bits: args.difficulty_bits,
    };
    params
        .validate()
        .map_err(|e| anyhow!("matmul params invalid: {e}"))?;

    let (a, b) = match (&args.a, &args.b, &args.synth_seed) {
        (Some(ap), Some(bp), None) => {
            let a = load_matrix(ap, (args.m * args.k) as usize, "A")?;
            let b = load_matrix(bp, (args.k * args.n) as usize, "B")?;
            (a, b)
        }
        (None, None, Some(seed)) => ai_pow::synth::synth_matrices(seed.as_bytes(), &params),
        _ => bail!("provide either --a + --b OR --synth-seed (not both, not neither)"),
    };

    let puzzle_id = match &args.puzzle_id {
        Some(s) => parse_hex_32(s, "--puzzle-id")?.to_vec(),
        None => blake3::hash(ai_pow::prover::params_tag(&params).as_slice())
            .as_bytes()
            .to_vec(),
    };

    let puzzle_id_for_builder = puzzle_id.clone();
    let params_for_builder = params;
    let a = Arc::new(a);
    let b = Arc::new(b);
    let a_for_builder = a.clone();
    let b_for_builder = b.clone();
    let certificate_builder = Arc::new(move |sol: &ai_pow_miner::MinedSolution| {
        ai_pow::verify_at_target(
            &puzzle_id_for_builder, &sol.nonce, &params_for_builder, &sol.target, &sol.proof,
        )
        .map_err(|e| {
            AiPowCertificateBuildError(format!(
                "refusing to build recursive certificate before successful matmul target check: {e}"
            ))
        })?;
        let ctx = ai_pow::prover::BlockContext::build(
            &puzzle_id_for_builder,
            &sol.nonce,
            a_for_builder.as_slice(),
            b_for_builder.as_slice(),
            &params_for_builder,
        )
        .map_err(|e| AiPowCertificateBuildError(e.to_string()))?;
        let run = prove_ai_pow_recursive_certificate(
            &ctx, &params_for_builder, &sol.nonce, &sol.target, sol.found_idx,
        )
        .map_err(|e| AiPowCertificateBuildError(e.to_string()))?;
        build_ai_pow_certificate_noun(
            &run.zk_params, run.found_idx, run.trace_height, &run.commitments, &run.pis,
            &run.certificate,
        )
        .map_err(|e| AiPowCertificateBuildError(e.to_string()))
    });

    Ok(AiPuzzleInputs {
        puzzle_id,
        params,
        a,
        b,
        prover_opts: ProverOptions::default(),
        certificate_builder: Some(certificate_builder),
    })
}

fn parse_hex_32(s: &str, label: &str) -> Result<[u8; 32]> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(trimmed).with_context(|| format!("{label}: invalid hex"))?;
    if bytes.len() != 32 {
        bail!("{label}: expected 32 bytes, got {}", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn load_matrix(path: &PathBuf, expected_len: usize, label: &str) -> Result<Vec<i8>> {
    let bytes = fs::read(path).with_context(|| format!("{label}: read {}", path.display()))?;
    if bytes.len() != expected_len {
        bail!(
            "{label}: expected {expected_len} bytes (i8 entries), got {}",
            bytes.len()
        );
    }
    Ok(bytes.into_iter().map(|b| b as i8).collect())
}

#[cfg(test)]
mod tests {
    use ai_pow_miner::{MiningCancel, MiningJob, NonceAnchors};

    use super::*;

    fn test_args() -> Args {
        Args {
            node_addr: "http://127.0.0.1:5555".to_string(),
            mining_pkh: Some("9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV".to_string()),
            mining_pkh_adv: None,
            puzzle_id: None,
            m: 64,
            k: 512,
            n: 64,
            noise_rank: 32,
            tile: 8,
            spot_checks: 8,
            difficulty_bits: 0,
            a: None,
            b: None,
            synth_seed: Some("ai-pow-zkp-builder-target-check".to_string()),
            max_extranonces: Some(1),
            reconnect_backoff_initial_ms: 1_000,
            reconnect_backoff_max_ms: 30_000,
            reconnect_max_attempts: 5,
            log: "off".to_string(),
        }
    }

    #[test]
    fn recursive_certificate_builder_rejects_before_zkp_when_target_check_fails() {
        let puzzle = build_puzzle_inputs(&test_args()).expect("test puzzle");
        let easy_target = [0xFF; 32];
        let job = MiningJob {
            puzzle_id: &puzzle.puzzle_id,
            anchors: NonceAnchors::nck_only([7; 32]),
            params: &puzzle.params,
            target: easy_target,
            a: puzzle.a.as_slice(),
            b: puzzle.b.as_slice(),
        };
        let mut opts = MineOptions::default();
        opts.max_extranonces = Some(1);
        let mut sol =
            ai_pow_miner::mining::run(&job, &opts, MiningCancel::new()).expect("easy solution");
        sol.target = [0; 32];

        let build = puzzle
            .certificate_builder
            .as_ref()
            .expect("production builder configured");
        let err = build(&sol).expect_err("bad target must not build a recursive certificate");
        assert!(
            err.to_string().contains(
                "refusing to build recursive certificate before successful matmul target check"
            ),
            "unexpected error: {err}"
        );
    }
}
