//! `ai-pow-mine` — standalone AI-PoW (matmul puzzle) block miner.
//!
//! Mirrors `zk-pow-mine` in shape: connects to a `nockchain` node's
//! private NockAppService gRPC, subscribes to `%mine` candidate
//! effects, searches Pearl-compatible tickets, builds the recursive
//! certificate only after a Nockchain target hit, and submits
//! `[%command %pow %ai-pow nonce cert]` on the `AiPowMinerWire::Mined` wire
//! (`SOURCE = "ai-pow-miner"`, `VERSION = 1`). The production submission path
//! fails closed for multi-tile configurations until the recursive statement
//! binds a full-matrix aggregate.
//!
//! Quick start (assuming a fakenet node on `127.0.0.1:5555`):
//!
//!   ai-pow-mine \
//!       --node-addr http://127.0.0.1:5555 \
//!       --mining-pkh 9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV
//!
//! The CLI defaults to Pearl-compatible submission with single-tile,
//! production-envelope smoke parameters for local Layer-0 development. The
//! Pearl work source defaults to Pearl Gateway miner RPC over the Unix socket
//! `/tmp/pearlgw.sock`; use `--pearl-gateway tcp://host:port` for a TCP
//! gateway or `--pearl-gateway /path/to.sock` for a different Unix socket. The
//! production profile
//! derives canonical seeds from the nonce-keyed chunk commitments bound by the
//! recursive proof as `HASH_A` / `HASH_B`; larger production shapes remain
//! closed until full-matrix aggregation is implemented.
//!
//! ## AI puzzle inputs (local config)
//! The chain's `%mine-ai` effect carries the candidate block commitment,
//! target, and pow-len. The miner additionally needs matmul `params`, matrices
//! `a` / `b`, and Rust-only Pearl transcript fields. If no matrix paths or
//! seed are supplied, the CLI synthesizes the default local smoke-profile
//! matrices from `ai-pow-prod-v1`. Hoon still receives only the opaque
//! `%ai-pow` nonce plus recursive certificate.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use ai_pow::pearl_compat::{
    validate_pearl_merge_config_for_recursive_prover, PearlMiningConfig, PearlNockchainAux,
    PearlPeriodicPattern, PEARL_MINING_CONFIG_RESERVED_SIZE, PEARL_MMA_INT7XINT7_TO_INT32,
};
use ai_pow_miner::pearl_mining::PearlMergeMineOptions;
use ai_pow_miner::run::{
    default_v0_configs, run, AiPuzzleInputs, MinerConfig, MinerError, PearlGatewayMinerRpcConfig,
    PearlGatewayTransport, PearlMergeHeaderSource, PearlMergeSubmissionConfig,
};
use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use nockchain_mining_common::MiningPkhConfig;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::{fmt, EnvFilter};

const DEFAULT_PEARL_NOCKCHAIN_CHAIN_ID: &str = "nockchain";
const DEFAULT_PEARL_GATEWAY_ENDPOINT: &str = "unix:/tmp/pearlgw.sock";
const DEFAULT_PEARL_GATEWAY_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_PEARL_GATEWAY_REFRESH_MS: u64 = 1_000;
const DEFAULT_SYNTH_SEED: &str = "ai-pow-prod-v1";

/// `ai-pow-mine` — standalone AI-PoW block miner.
#[derive(Parser, Debug)]
#[command(
    name = "ai-pow-mine",
    about = "Standalone AI-PoW block miner. Mines Pearl-compatible tickets and submits canonical recursive %ai-pow commands to a nockchain node.",
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
    /// Matmul puzzle rows. Default is the current single-tile Layer-0 smoke profile.
    #[arg(short = 'm', long, default_value_t = 8, hide = true)]
    m: u32,
    /// Matmul shared dimension. Default satisfies Pearl's public-parameter envelope with r=32.
    #[arg(short = 'k', long, default_value_t = 1024, hide = true)]
    k: u32,
    /// Matmul output columns. Default is one tile for local recursive-proof smoke runs.
    #[arg(short = 'n', long, default_value_t = 8, hide = true)]
    n: u32,
    #[arg(long, default_value_t = 32, hide = true)]
    noise_rank: u32,
    #[arg(long, default_value_t = 8, hide = true)]
    tile: u32,
    #[arg(long, default_value_t = 1, hide = true)]
    spot_checks: u32,
    #[arg(long, default_value_t = 0, hide = true)]
    difficulty_bits: u32,

    /// Path to raw i8 matrix A (length m·k). Mutually exclusive with --synth-seed.
    #[arg(long, value_name = "PATH", conflicts_with = "synth_seed", hide = true)]
    a: Option<PathBuf>,
    /// Path to raw i8 matrix B (length k·n).
    #[arg(long, value_name = "PATH", conflicts_with = "synth_seed", hide = true)]
    b: Option<PathBuf>,
    /// Synthesize A + B deterministically from this seed string. If no matrix
    /// input is supplied, defaults to the local smoke-profile seed
    /// `ai-pow-prod-v1`.
    #[arg(long, hide = true)]
    synth_seed: Option<String>,

    /// Pearl Gateway miner RPC endpoint. Accepts `unix:/path/to.sock`, `/path/to.sock`,
    /// `tcp:host:port`, `tcp://host:port`, or `host:port`.
    #[arg(long, value_name = "ENDPOINT", default_value = DEFAULT_PEARL_GATEWAY_ENDPOINT)]
    pearl_gateway: String,

    /// Pearl Gateway request timeout in milliseconds.
    #[arg(long, default_value_t = DEFAULT_PEARL_GATEWAY_TIMEOUT_MS, hide = true)]
    pearl_gateway_timeout_ms: u64,

    /// Pearl Gateway work refresh interval in milliseconds.
    #[arg(long, default_value_t = DEFAULT_PEARL_GATEWAY_REFRESH_MS, hide = true)]
    pearl_gateway_refresh_ms: u64,

    /// Rust-only Nockchain chain id committed into the Pearl aux payload.
    #[arg(long, default_value = DEFAULT_PEARL_NOCKCHAIN_CHAIN_ID, hide = true)]
    pearl_nockchain_chain_id: String,

    /// Rust-only Nockchain epoch/height committed into the Pearl aux payload.
    #[arg(long, default_value_t = 0, hide = true)]
    pearl_nockchain_target_epoch_or_height: u64,

    /// Extra domain bytes committed into the Pearl aux payload.
    #[arg(long, default_value = "", hide = true)]
    pearl_extra_domain_data: String,

    /// Maximum decoded Pearl periodic-pattern list length accepted by the
    /// Rust-side prechecks and prover.
    #[arg(long, default_value_t = 256, hide = true)]
    pearl_max_pattern_len: usize,

    /// Stop Pearl-compatible ticket search after this many attempts
    /// (None ⇒ scan all valid offsets).
    #[arg(long, hide = true)]
    pearl_max_attempts: Option<u64>,

    // ── reconnect tuning ───────────────────────────────────────────
    /// Initial reconnect backoff in milliseconds.
    #[arg(long, default_value = "1000", hide = true)]
    reconnect_backoff_initial_ms: u64,

    /// Maximum reconnect backoff in milliseconds (cap).
    #[arg(long, default_value = "30000", hide = true)]
    reconnect_backoff_max_ms: u64,

    /// Consecutive reconnect attempts before giving up.
    #[arg(long, default_value = "5", hide = true)]
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

    let cfg = MinerConfig {
        node_addr: args.node_addr,
        mining_configs: default_v0_configs(),
        mining_pkh_configs: pkh_configs,
        puzzle,
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
    validate_pearl_recursive_cli_params(args, params)?;

    let (a, b) = match (&args.a, &args.b, &args.synth_seed) {
        (Some(ap), Some(bp), None) => {
            let a = load_matrix(ap, checked_matrix_len(args.m, args.k, "A")?, "A")?;
            let b = load_matrix(bp, checked_matrix_len(args.n, args.k, "B")?, "B")?;
            (a, b)
        }
        (None, None, Some(seed)) => ai_pow::synth::synth_matrices(seed.as_bytes(), &params),
        (None, None, None) => ai_pow::synth::synth_matrices(DEFAULT_SYNTH_SEED.as_bytes(), &params),
        _ => bail!("provide either both --a + --b, a custom --synth-seed, or neither for the default synth seed"),
    };

    let a = Arc::new(a);
    let b = Arc::new(b);

    let pearl_merge = build_pearl_merge_submission_config(args, params, &a, &b)?;
    Ok(AiPuzzleInputs {
        params,
        a,
        b,
        pearl_merge,
    })
}

fn validate_pearl_recursive_cli_params(args: &Args, params: MatmulParams) -> Result<()> {
    if params.difficulty_bits != 0 || params.spot_checks != 1 {
        bail!(
            "Pearl-compatible recursive certificates require --difficulty-bits 0 and --spot-checks 1"
        );
    }
    params
        .validate_prod_envelope()
        .map_err(|e| anyhow!("Pearl-compatible params are not production-admissible: {e}"))?;
    if params.num_tiles() > 1 {
        bail!(
            "Pearl-compatible recursive certificates require exactly one tile; current params have {} tiles",
            params.num_tiles()
        );
    }
    if args.pearl_max_pattern_len < params.tile as usize {
        bail!(
            "--pearl-max-pattern-len must be at least --tile ({}), got {}", params.tile,
            args.pearl_max_pattern_len
        );
    }
    Ok(())
}

fn build_pearl_merge_submission_config(
    args: &Args,
    params: MatmulParams,
    a: &Arc<Vec<i8>>,
    b: &Arc<Vec<i8>>,
) -> Result<PearlMergeSubmissionConfig> {
    validate_pearl_recursive_cli_params(args, params)?;

    let rows_pattern = contiguous_pearl_pattern(params.tile)?;
    let cols_pattern = contiguous_pearl_pattern(params.tile)?;
    let mining_config = PearlMiningConfig {
        common_dim: params.k,
        rank: u16::try_from(params.noise_rank)
            .map_err(|_| anyhow!("--noise-rank does not fit Pearl mining config u16"))?,
        mma_type: PEARL_MMA_INT7XINT7_TO_INT32,
        rows_pattern,
        cols_pattern,
        reserved: [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
    };
    validate_pearl_merge_config_for_recursive_prover(
        &mining_config, &params, args.pearl_max_pattern_len,
    )
    .map_err(|e| anyhow!("Pearl mining config is not supported for recursive proofs: {e}"))?;

    let request_timeout = Duration::from_millis(args.pearl_gateway_timeout_ms);
    if request_timeout.is_zero() {
        bail!("--pearl-gateway-timeout-ms must be greater than zero");
    };
    let refresh_interval = Duration::from_millis(args.pearl_gateway_refresh_ms);
    if refresh_interval.is_zero() {
        bail!("--pearl-gateway-refresh-ms must be greater than zero");
    }
    let header_source = PearlMergeHeaderSource::Gateway(PearlGatewayMinerRpcConfig {
        transport: resolve_pearl_gateway_transport(args)?,
        request_timeout,
        refresh_interval,
    });
    let aux_template = PearlNockchainAux {
        nockchain_chain_id: args.pearl_nockchain_chain_id.as_bytes().to_vec(),
        nock_block_commitment: [0u8; 32],
        nockchain_target_epoch_or_height: args.pearl_nockchain_target_epoch_or_height,
        extra_domain_data: parse_optional_hex_or_utf8(&args.pearl_extra_domain_data)
            .context("--pearl-extra-domain-data")?,
    };
    aux_template
        .to_bytes()
        .map_err(|e| anyhow!("Pearl aux template is not canonical: {e}"))?;

    let mut mine_opts = PearlMergeMineOptions::default();
    mine_opts.max_attempts = args.pearl_max_attempts;

    Ok(PearlMergeSubmissionConfig::new_recursive(
        header_source,
        mining_config,
        aux_template,
        args.pearl_max_pattern_len,
        mine_opts,
        params,
        a.clone(),
        b.clone(),
    ))
}

fn resolve_pearl_gateway_transport(args: &Args) -> Result<PearlGatewayTransport> {
    parse_pearl_gateway_endpoint(&args.pearl_gateway)
}

fn parse_pearl_gateway_endpoint(endpoint: &str) -> Result<PearlGatewayTransport> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        bail!("--pearl-gateway endpoint must not be empty");
    }

    if let Some(path) = endpoint
        .strip_prefix("unix://")
        .or_else(|| endpoint.strip_prefix("uds://"))
        .or_else(|| endpoint.strip_prefix("unix:"))
        .or_else(|| endpoint.strip_prefix("uds:"))
    {
        if path.is_empty() {
            bail!("--pearl-gateway unix endpoint path must not be empty");
        }
        return Ok(PearlGatewayTransport::UnixSocket {
            path: path.to_string(),
        });
    }

    if endpoint.starts_with('/') {
        return Ok(PearlGatewayTransport::UnixSocket {
            path: endpoint.to_string(),
        });
    }

    let tcp = endpoint
        .strip_prefix("tcp://")
        .or_else(|| endpoint.strip_prefix("tcp:"))
        .unwrap_or(endpoint);
    let Some((host, port)) = tcp.rsplit_once(':') else {
        bail!("--pearl-gateway must be unix:/path, /path, tcp:host:port, or host:port");
    };
    if host.is_empty() {
        bail!("--pearl-gateway TCP host must not be empty");
    }
    let port = port
        .parse::<u16>()
        .with_context(|| "--pearl-gateway TCP port must be a u16")?;
    Ok(PearlGatewayTransport::Tcp {
        host: host.to_string(),
        port,
    })
}

fn contiguous_pearl_pattern(tile: u32) -> Result<PearlPeriodicPattern> {
    if tile == 0 {
        bail!("--tile must be nonzero");
    }
    let indices: Vec<u32> = (0..tile).collect();
    PearlPeriodicPattern::from_list(&indices)
        .map_err(|e| anyhow!("contiguous Pearl pattern for tile {tile} is invalid: {e}"))
}

fn parse_optional_hex_or_utf8(s: &str) -> Result<Vec<u8>> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Ok(hex::decode(hex)?)
    } else {
        Ok(s.as_bytes().to_vec())
    }
}

fn checked_matrix_len(rows: u32, cols: u32, label: &str) -> Result<usize> {
    let len = u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or_else(|| anyhow!("{label}: matrix length overflows u64"))?;
    usize::try_from(len).map_err(|_| anyhow!("{label}: matrix length does not fit usize"))
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
    use ai_pow::pearl_compat::evaluate_pearl_merge_ticket_attempt;
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_defaults_to_pearl_gateway_source() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV",
        ]);
        assert_eq!((args.m, args.k, args.n), (8, 1024, 8));
        assert_eq!(args.noise_rank, 32);
        assert_eq!(args.spot_checks, 1);

        let puzzle = build_puzzle_inputs(&args).expect("default Pearl gateway config");
        let (expected_a, expected_b) =
            ai_pow::synth::synth_matrices(DEFAULT_SYNTH_SEED.as_bytes(), &puzzle.params);
        assert_eq!(puzzle.a.as_slice(), expected_a.as_slice());
        assert_eq!(puzzle.b.as_slice(), expected_b.as_slice());
        let pearl = &puzzle.pearl_merge;
        let PearlMergeHeaderSource::Gateway(cfg) = &pearl.header_source;
        assert_eq!(
            cfg.transport,
            PearlGatewayTransport::UnixSocket {
                path: "/tmp/pearlgw.sock".to_string()
            }
        );
        assert_eq!(
            cfg.request_timeout,
            Duration::from_millis(DEFAULT_PEARL_GATEWAY_TIMEOUT_MS)
        );
        assert_eq!(
            cfg.refresh_interval,
            Duration::from_millis(DEFAULT_PEARL_GATEWAY_REFRESH_MS)
        );
    }

    #[test]
    fn cli_rejects_partial_explicit_matrix_input() {
        let only_a = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--a",
            "/does/not/matter/a.bin",
        ]);
        let err = match build_puzzle_inputs(&only_a) {
            Ok(_) => panic!("partial explicit matrix input must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("both --a + --b"),
            "unexpected error: {err:#}"
        );

        let only_b = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--b",
            "/does/not/matter/b.bin",
        ]);
        let err = match build_puzzle_inputs(&only_b) {
            Ok(_) => panic!("partial explicit matrix input must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("both --a + --b"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_can_configure_pearl_gateway_tcp_source() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--pearl-gateway",
            "127.0.0.1:8337", "--pearl-gateway-timeout-ms", "250", "--pearl-gateway-refresh-ms",
            "500",
        ]);

        let puzzle = build_puzzle_inputs(&args).expect("configured Pearl TCP gateway config");
        let pearl = &puzzle.pearl_merge;
        let PearlMergeHeaderSource::Gateway(cfg) = &pearl.header_source;
        assert_eq!(
            cfg.transport,
            PearlGatewayTransport::Tcp {
                host: "127.0.0.1".to_string(),
                port: 8337
            }
        );
        assert_eq!(cfg.request_timeout, Duration::from_millis(250));
        assert_eq!(cfg.refresh_interval, Duration::from_millis(500));
    }

    #[test]
    fn cli_accepts_unified_pearl_gateway_endpoint_forms() {
        let unix = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--pearl-gateway",
            "unix:/var/run/pearlgw.sock",
        ]);
        assert_eq!(
            resolve_pearl_gateway_transport(&unix).expect("parse unix endpoint"),
            PearlGatewayTransport::UnixSocket {
                path: "/var/run/pearlgw.sock".to_string()
            }
        );

        let bare_unix = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--pearl-gateway",
            "/var/run/pearlgw.sock",
        ]);
        assert_eq!(
            resolve_pearl_gateway_transport(&bare_unix).expect("parse bare unix endpoint"),
            PearlGatewayTransport::UnixSocket {
                path: "/var/run/pearlgw.sock".to_string()
            }
        );

        let tcp = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--pearl-gateway",
            "tcp://pearl.example:18443",
        ]);
        assert_eq!(
            resolve_pearl_gateway_transport(&tcp).expect("parse tcp endpoint"),
            PearlGatewayTransport::Tcp {
                host: "pearl.example".to_string(),
                port: 18443
            }
        );
    }

    #[test]
    fn cli_rejects_malformed_unified_pearl_gateway_endpoint() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--pearl-gateway",
            "tcp://localhost:not-a-port",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("malformed Pearl Gateway endpoint must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("--pearl-gateway TCP port"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_help_shows_unified_gateway_endpoint_not_legacy_split_flags() {
        let help = Args::command().render_long_help().to_string();

        assert!(help.contains("--pearl-gateway <ENDPOINT>"));
        assert!(help.contains("[default: unix:/tmp/pearlgw.sock]"));
        assert!(help.contains("--node-addr <NODE_ADDR>"));
        assert!(help.contains("--mining-pkh <MINING_PKH>"));
        assert!(!help.contains("--pearl-work-source"));
        assert!(!help.contains("--pearl-gateway-transport"));
        assert!(!help.contains("--pearl-gateway-socket"));
        assert!(!help.contains("--pearl-prev-block"));
        assert!(!help.contains("--pearl-timestamp"));
        assert!(!help.contains("--pearl-nbits"));
        assert!(!help.contains("--pearl-max-attempts"));
        assert!(!help.contains("--noise-rank"));
        assert!(!help.contains("--synth-seed"));
        assert!(!help.contains("--pearl-gateway-timeout-ms"));
        assert!(!help.contains("--pearl-nockchain-chain-id"));
        assert!(!help.contains("--reconnect-max-attempts"));
    }

    #[test]
    fn cli_rejects_legacy_pearl_gateway_split_flags() {
        let err = Args::try_parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--pearl-gateway-transport",
            "tcp", "--pearl-gateway-host", "127.0.0.1", "--pearl-gateway-port", "8337",
        ])
        .expect_err("legacy split Pearl Gateway flags should not parse");
        assert!(
            err.to_string().contains("--pearl-gateway-transport"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn cli_rejects_zero_pearl_gateway_timeout() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-zero-gateway-timeout", "--pearl-gateway-timeout-ms", "0",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("zero Pearl Gateway timeout must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("--pearl-gateway-timeout-ms"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_rejects_zero_pearl_gateway_refresh_interval() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-zero-gateway-refresh", "--pearl-gateway-refresh-ms", "0",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("zero Pearl Gateway refresh interval must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("--pearl-gateway-refresh-ms"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_can_build_configured_pearl_merge_submission_inputs() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-cli", "--m", "8", "--k", "1024", "--n", "8", "--noise-rank", "32",
            "--tile", "8", "--spot-checks", "1", "--difficulty-bits", "0", "--pearl-gateway",
            "tcp://127.0.0.1:8337", "--pearl-nockchain-target-epoch-or-height", "42",
            "--pearl-extra-domain-data", "0xfeed", "--pearl-max-attempts", "16",
        ]);

        let puzzle = build_puzzle_inputs(&args).expect("pearl merge puzzle inputs");
        let pearl = &puzzle.pearl_merge;
        let PearlMergeHeaderSource::Gateway(gateway) = &pearl.header_source;
        assert_eq!(
            gateway.transport,
            PearlGatewayTransport::Tcp {
                host: "127.0.0.1".to_string(),
                port: 8337
            }
        );
        assert_eq!(pearl.mining_config.common_dim, 1024);
        assert_eq!(pearl.mining_config.rank, 32);
        assert_eq!(pearl.max_pattern_len, 256);
        assert_eq!(pearl.mine_opts.max_attempts, Some(16));
        assert_eq!(pearl.aux_template.nockchain_chain_id, b"nockchain");
        assert_eq!(pearl.aux_template.nockchain_target_epoch_or_height, 42);
        assert_eq!(pearl.aux_template.extra_domain_data, vec![0xfe, 0xed]);

        puzzle
            .validate_canonical_submission_ready()
            .expect("configured pearl merge submission should pass preflight");
    }

    #[test]
    fn cli_certificate_builder_rejects_target_miss_before_recursive_proof() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-cli-builder-target-miss", "--m", "8", "--k", "1024", "--n", "8",
            "--noise-rank", "32", "--tile", "8", "--spot-checks", "1", "--difficulty-bits", "0",
        ]);

        let puzzle = build_puzzle_inputs(&args).expect("pearl merge puzzle inputs");
        let pearl = &puzzle.pearl_merge;
        let header = ai_pow::pearl_compat::PearlIncompleteBlockHeader {
            version: 1,
            prev_block: [0x11; 32],
            merkle_root: [0u8; 32],
            timestamp: 1_717_171_717,
            nbits: 0x207f_ffff,
        };
        let mut attempt = evaluate_pearl_merge_ticket_attempt(
            &header,
            &pearl.mining_config,
            &puzzle.params,
            0,
            0,
            puzzle.a.as_slice(),
            puzzle.b.as_slice(),
            &[0xff; 32],
            pearl.max_pattern_len,
            pearl.aux_template.clone(),
        )
        .expect("evaluate trivial-target Pearl merge ticket");
        attempt.nockchain_target = [0u8; 32];

        let err = pearl
            .build_certificate_for_attempt(&attempt)
            .expect_err("CLI certificate builder must reject target misses before proving");
        assert!(
            err.to_string()
                .contains("before successful Nockchain target check"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn cli_rejects_noncanonical_pearl_aux_template() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-bad-aux", "--pearl-nockchain-chain-id", "",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("noncanonical Pearl aux template must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Pearl aux template is not canonical"),
            "unexpected error: {err:#}"
        );
        assert!(
            err.to_string().contains("chain id must not be empty"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_rejects_pattern_bound_smaller_than_tile_before_mining() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-small-pattern-bound", "--pearl-max-pattern-len", "7",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("pattern bound smaller than tile must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("--pearl-max-pattern-len"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_rejects_pearl_merge_noncanonical_recursive_params_before_mining() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-bad-params", "--m", "16", "--n", "8", "--spot-checks", "2",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("bad Pearl recursive params must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("require --difficulty-bits 0 and --spot-checks 1"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_rejects_pearl_merge_multi_tile_before_matrix_synthesis() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "must-not-materialize-multi-tile", "--m", "16", "--k", "512", "--n", "8",
            "--noise-rank", "32", "--tile", "8", "--spot-checks", "1", "--difficulty-bits", "0",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("multi-tile Pearl recursive params must fail before matrix synthesis"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("require exactly one tile"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_rejects_nonproduction_shape_before_matrix_synthesis() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "must-not-materialize-matrix", "--m", "16777224", "--k", "512", "--n", "8",
            "--noise-rank", "32", "--tile", "8", "--spot-checks", "1", "--difficulty-bits", "0",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("nonproduction Pearl shape must fail before matrix synthesis"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Pearl-compatible params are not production-admissible"),
            "unexpected error: {err:#}"
        );
    }
}
