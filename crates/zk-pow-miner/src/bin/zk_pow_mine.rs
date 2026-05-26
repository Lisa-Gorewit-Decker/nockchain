//! `zk-pow-mine` — standalone ZK-PoW (puzzle-nock STARK) miner.
//!
//! Connects to a `nockchain` node's private NockAppService gRPC,
//! subscribes to `%mine` candidate effects, runs the STARK in a
//! worker pool, and pokes solutions back as `%pow` commands on the
//! `MiningWire::Mined` wire.
//!
//! Quick start (assuming a fakenet node on `127.0.0.1:5555`):
//!
//!   zk-pow-mine \
//!       --node-addr http://127.0.0.1:5555 \
//!       --mining-pkh 9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV \
//!       --num-threads 1

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use nockchain_mining_common::MiningPkhConfig;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::{fmt, EnvFilter};
use zk_pow_miner::run::{default_v0_configs, run, MinerConfig, MinerError};

/// `zk-pow-mine` — standalone ZK PoW block miner.
#[derive(Parser, Debug)]
#[command(
    name = "zk-pow-mine",
    about = "Standalone ZK PoW block miner. Subscribes to a nockchain node's %mine effects via gRPC and submits solutions back.",
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

    /// Worker pool size (number of concurrent SerfThreads, each running miner.jam).
    /// Defaults to `num_cpus - 1` (min 1).
    #[arg(long)]
    num_threads: Option<u64>,

    /// Initial reconnect backoff in milliseconds.
    #[arg(long, default_value = "1000")]
    reconnect_backoff_initial_ms: u64,

    /// Maximum reconnect backoff in milliseconds (cap).
    #[arg(long, default_value = "30000")]
    reconnect_backoff_max_ms: u64,

    /// Consecutive reconnect attempts before giving up.
    #[arg(long, default_value = "5")]
    reconnect_max_attempts: u32,

    /// Log filter (env-filter syntax: e.g., `info`, `zk_pow_miner=debug,info`).
    /// Override with the `RUST_LOG` env var.
    #[arg(long, default_value = "info,zk_pow_miner=info,nockchain_mining_common=info")]
    log: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    init_tracing(&args.log);

    // Validate args.
    let Some(pkh_configs) = build_pkh_configs(&args) else {
        eprintln!(
            "zk-pow-mine: must supply --mining-pkh <HASH> or --mining-pkh-adv \"share,pkh\""
        );
        return ExitCode::from(1);
    };

    let num_threads = args.num_threads.unwrap_or_else(|| {
        num_cpus::get().saturating_sub(1).max(1) as u64
    });

    let cfg = MinerConfig {
        node_addr: args.node_addr,
        mining_configs: default_v0_configs(),
        mining_pkh_configs: pkh_configs,
        num_threads,
        reconnect_backoff_initial: Duration::from_millis(args.reconnect_backoff_initial_ms),
        reconnect_backoff_max: Duration::from_millis(args.reconnect_backoff_max_ms),
        reconnect_max_attempts: args.reconnect_max_attempts,
    };

    // Build a tokio runtime here (rather than tokio::main) so we can return
    // a precise ExitCode.
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("zk-pow-mine: failed to build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let r: Result<(), MinerError> = rt.block_on(async {
        info!(node = %cfg.node_addr, threads = cfg.num_threads, "zk-pow-mine: starting");
        let shutdown = CancellationToken::new();
        // Spawn a Ctrl-C watcher.
        let shutdown_clone = shutdown.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("zk-pow-mine: SIGINT received; shutting down");
                shutdown_clone.cancel();
            }
        });
        run(cfg, shutdown).await
    });

    match r {
        Ok(()) => {
            info!("zk-pow-mine: clean shutdown");
            ExitCode::from(0)
        }
        Err(MinerError::TooManyReconnects { count }) => {
            error!("zk-pow-mine: gave up after {count} consecutive reconnect failures");
            ExitCode::from(2)
        }
        Err(e) => {
            error!(error = %e, "zk-pow-mine: unrecoverable error");
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
    } else if let Some(adv) = &args.mining_pkh_adv {
        Some(adv.clone())
    } else {
        None
    }
}
