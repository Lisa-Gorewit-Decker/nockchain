//! `ai-pow-mine` — standalone AI-PoW (matmul puzzle) block miner.
//!
//! Mirrors `zk-pow-mine` in shape: connects to a `nockchain` node's
//! private NockAppService gRPC, subscribes to `%mine` candidate
//! effects, runs the AI-PoW prover once the certificate gate is open, and
//! submits `[%command %pow %ai-pow nonce cert]` on the `AiPowMinerWire::Mined` wire
//! (`SOURCE = "ai-pow-miner"`, `VERSION = 1`) when the recursive certificate
//! builder can prove the configured work unit. The production submission path
//! fails closed for multi-tile configurations until the recursive statement
//! binds a full-matrix aggregate.
//!
//! Quick start (assuming a fakenet node on `127.0.0.1:5555`):
//!
//!   ai-pow-mine \
//!       --node-addr http://127.0.0.1:5555 \
//!       --mining-pkh 9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV \
//!       --synth-seed ai-pow-prod-v1 \
//!       --pearl-prev-block 1111111111111111111111111111111111111111111111111111111111111111 \
//!       --pearl-timestamp 1717171717 \
//!       --pearl-nbits 0x207fffff
//!
//! The CLI defaults to Pearl-compatible submission with single-tile,
//! production-envelope smoke parameters for local Layer-0 development. The
//! Pearl header fields still have to be supplied explicitly because they define
//! the shared attempt transcript. That profile derives canonical seeds from
//! the nonce-keyed chunk commitments bound by the recursive proof as `HASH_A`
//! / `HASH_B`; larger production shapes remain closed until full-matrix
//! aggregation is implemented.
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
use ai_pow::pearl_compat::{
    validate_pearl_merge_config_for_recursive_prover, PearlIncompleteBlockHeader,
    PearlMergeTicketAttempt, PearlMiningConfig, PearlNockchainAux, PearlPeriodicPattern,
    PEARL_MINING_CONFIG_RESERVED_SIZE, PEARL_MMA_INT7XINT7_TO_INT32,
};
use ai_pow::prover::ProverOptions;
use ai_pow::zk_bridge::{
    prove_ai_pow_recursive_certificate, prove_pearl_merge_recursive_certificate,
};
use ai_pow_miner::certificate_noun::build_ai_pow_certificate_noun_from_recursive_run;
use ai_pow_miner::pearl_mining::PearlMergeMineOptions;
use ai_pow_miner::run::{
    default_v0_configs, run, AiPowCertificateBuildError, AiPowSubmissionMode, AiPuzzleInputs,
    MinerConfig, MinerError, PearlMergeCertificateProof, PearlMergeSubmissionConfig,
};
use ai_pow_miner::MineOptions;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, ValueEnum};
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

    /// Matmul puzzle rows. Default is the current single-tile Layer-0 smoke profile.
    #[arg(short = 'm', long, default_value_t = 8)]
    m: u32,
    /// Matmul shared dimension. Default satisfies the production envelope with r=32.
    #[arg(short = 'k', long, default_value_t = 512)]
    k: u32,
    /// Matmul output columns. Default is one tile for local recursive-proof smoke runs.
    #[arg(short = 'n', long, default_value_t = 8)]
    n: u32,
    #[arg(long, default_value_t = 32)]
    noise_rank: u32,
    #[arg(long, default_value_t = 8)]
    tile: u32,
    #[arg(long, default_value_t = 1)]
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

    /// AI-PoW submission mode. `pearl-merge` is the production-compatible
    /// default; `legacy-ncmn` is retained only for explicit dev smoke tests.
    #[arg(long, value_enum, default_value_t = SubmissionModeArg::PearlMerge)]
    submission_mode: SubmissionModeArg,

    // ── Pearl-compatible Rust-only transcript config ───────────────
    /// Pearl header version for `--submission-mode pearl-merge`.
    #[arg(long, default_value_t = 1)]
    pearl_version: u32,

    /// Pearl previous block hash, display-order 32-byte hex. Required for
    /// `--submission-mode pearl-merge`.
    #[arg(long)]
    pearl_prev_block: Option<String>,

    /// Pearl header timestamp. Required for `--submission-mode pearl-merge`.
    #[arg(long)]
    pearl_timestamp: Option<u32>,

    /// Pearl compact target bits as decimal or 0x-prefixed u32. Required for
    /// `--submission-mode pearl-merge`.
    #[arg(long)]
    pearl_nbits: Option<String>,

    /// Rust-only Nockchain chain id committed into the Pearl aux payload.
    #[arg(long, default_value = "nockchain")]
    pearl_nockchain_chain_id: String,

    /// Rust-only Nockchain epoch/height committed into the Pearl aux payload.
    #[arg(long, default_value_t = 0)]
    pearl_nockchain_target_epoch_or_height: u64,

    /// Extra domain bytes committed into the Pearl aux payload.
    #[arg(long, default_value = "")]
    pearl_extra_domain_data: String,

    /// Maximum decoded Pearl periodic-pattern list length accepted by the
    /// Rust-side prechecks and prover.
    #[arg(long, default_value_t = 256)]
    pearl_max_pattern_len: usize,

    /// Stop Pearl-compatible ticket search after this many attempts
    /// (None ⇒ scan all valid offsets).
    #[arg(long)]
    pearl_max_attempts: Option<u64>,

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum SubmissionModeArg {
    /// Explicit dev-only NCMN smoke mode.
    LegacyNcmn,
    /// Production-compatible Nockchain `%ai-pow` submission mode.
    PearlMerge,
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

    let a = Arc::new(a);
    let b = Arc::new(b);

    match args.submission_mode {
        SubmissionModeArg::LegacyNcmn => {
            let puzzle_id_for_builder = puzzle_id.clone();
            let params_for_builder = params;
            let a_for_builder = a.clone();
            let b_for_builder = b.clone();
            let certificate_builder = Arc::new(move |sol: &ai_pow_miner::MinedSolution| {
                ai_pow::verifier::verify_ncmn_at_target(
                    &puzzle_id_for_builder,
                    &sol.candidate_nck_commitment,
                    &sol.nonce,
                    &params_for_builder,
                    &sol.target,
                    &sol.proof,
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
                build_ai_pow_certificate_noun_from_recursive_run(&run)
                    .map_err(|e| AiPowCertificateBuildError(e.to_string()))
            });

            Ok(AiPuzzleInputs {
                puzzle_id,
                params,
                a,
                b,
                prover_opts: ProverOptions::default(),
                certificate_builder: Some(certificate_builder),
                pearl_merge: None,
                submission_mode: AiPowSubmissionMode::LegacyNcmn,
            })
        }
        SubmissionModeArg::PearlMerge => {
            let pearl_merge = build_pearl_merge_submission_config(args, params, &a, &b)?;
            Ok(AiPuzzleInputs {
                puzzle_id,
                params,
                a,
                b,
                prover_opts: ProverOptions::default(),
                certificate_builder: None,
                pearl_merge: Some(pearl_merge),
                submission_mode: AiPowSubmissionMode::PearlMerge,
            })
        }
    }
}

fn build_pearl_merge_submission_config(
    args: &Args,
    params: MatmulParams,
    a: &Arc<Vec<i8>>,
    b: &Arc<Vec<i8>>,
) -> Result<PearlMergeSubmissionConfig> {
    if params.difficulty_bits != 0 || params.spot_checks != 1 {
        bail!(
            "Pearl-compatible recursive certificates require --difficulty-bits 0 and --spot-checks 1"
        );
    }
    params
        .validate_prod_envelope()
        .map_err(|e| anyhow!("Pearl-compatible params are not production-admissible: {e}"))?;
    if args.pearl_max_pattern_len < params.tile as usize {
        bail!(
            "--pearl-max-pattern-len must be at least --tile ({}), got {}", params.tile,
            args.pearl_max_pattern_len
        );
    }

    let prev_block = parse_required_hex_32(
        args.pearl_prev_block.as_deref(),
        "--pearl-prev-block",
        "required when --submission-mode pearl-merge",
    )?;
    let timestamp = args.pearl_timestamp.ok_or_else(|| {
        anyhow!("--pearl-timestamp is required when --submission-mode pearl-merge")
    })?;
    let nbits = parse_required_u32(
        args.pearl_nbits.as_deref(),
        "--pearl-nbits",
        "required when --submission-mode pearl-merge",
    )?;
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

    let header_template = PearlIncompleteBlockHeader {
        version: args.pearl_version,
        prev_block,
        merkle_root: [0u8; 32],
        timestamp,
        nbits,
    };
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

    let params_for_builder = params;
    let a_for_builder = a.clone();
    let b_for_builder = b.clone();
    let max_pattern_len = args.pearl_max_pattern_len;
    let certificate_builder = Arc::new(move |attempt: &PearlMergeTicketAttempt| {
        let run = prove_pearl_merge_recursive_certificate(
            attempt,
            &params_for_builder,
            a_for_builder.as_slice(),
            b_for_builder.as_slice(),
            max_pattern_len,
        )
        .map_err(|e| {
            AiPowCertificateBuildError(format!(
                "refusing to build Pearl-compatible recursive certificate before successful Nockchain target check: {e}"
            ))
        })?;
        PearlMergeCertificateProof::from_recursive_run(&run)
    });

    let mut mine_opts = PearlMergeMineOptions::default();
    mine_opts.max_attempts = args.pearl_max_attempts;

    Ok(PearlMergeSubmissionConfig {
        header_template,
        mining_config,
        aux_template,
        max_pattern_len,
        mine_opts,
        certificate_builder,
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

fn parse_required_hex_32(value: Option<&str>, label: &str, missing: &str) -> Result<[u8; 32]> {
    let Some(value) = value else {
        bail!("{label}: {missing}");
    };
    parse_hex_32(value, label)
}

fn parse_required_u32(value: Option<&str>, label: &str, missing: &str) -> Result<u32> {
    let Some(value) = value else {
        bail!("{label}: {missing}");
    };
    parse_u32_maybe_hex(value, label)
}

fn parse_u32_maybe_hex(s: &str, label: &str) -> Result<u32> {
    let trimmed = s.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).with_context(|| format!("{label}: invalid hex u32"))
    } else {
        trimmed
            .parse::<u32>()
            .with_context(|| format!("{label}: invalid u32"))
    }
}

fn parse_optional_hex_or_utf8(s: &str) -> Result<Vec<u8>> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Ok(hex::decode(hex)?)
    } else {
        Ok(s.as_bytes().to_vec())
    }
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
    use ai_pow_miner::{build_ncmn_nonce, MiningCancel, MiningJob, NonceAnchors};

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
            submission_mode: SubmissionModeArg::LegacyNcmn,
            pearl_version: 1,
            pearl_prev_block: None,
            pearl_timestamp: None,
            pearl_nbits: None,
            pearl_nockchain_chain_id: "nockchain".to_string(),
            pearl_nockchain_target_epoch_or_height: 0,
            pearl_extra_domain_data: String::new(),
            pearl_max_pattern_len: 256,
            pearl_max_attempts: None,
            reconnect_backoff_initial_ms: 1_000,
            reconnect_backoff_max_ms: 30_000,
            reconnect_max_attempts: 5,
            log: "off".to_string(),
        }
    }

    #[test]
    fn cli_defaults_to_pearl_merge_and_requires_header_fields() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-default-layer0-smoke",
        ]);
        assert_eq!((args.m, args.k, args.n), (8, 512, 8));
        assert_eq!(args.noise_rank, 32);
        assert_eq!(args.spot_checks, 1);
        assert_eq!(args.submission_mode, SubmissionModeArg::PearlMerge);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("default Pearl mode needs header fields"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("--pearl-prev-block"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_explicit_legacy_ncmn_smoke_is_submission_ready() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-default-layer0-smoke", "--submission-mode", "legacy-ncmn",
        ]);
        assert_eq!(args.submission_mode, SubmissionModeArg::LegacyNcmn);

        let puzzle = build_puzzle_inputs(&args).expect("explicit legacy puzzle inputs");
        puzzle
            .validate_canonical_submission_ready()
            .expect("explicit legacy smoke preflight should pass");
    }

    #[test]
    fn cli_can_build_configured_default_pearl_merge_submission_inputs() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-cli", "--m", "8", "--k", "512", "--n", "8", "--noise-rank", "32",
            "--tile", "8", "--spot-checks", "1", "--difficulty-bits", "0", "--pearl-prev-block",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--pearl-timestamp", "1717171717", "--pearl-nbits", "0x207fffff",
            "--pearl-nockchain-chain-id", "nockchain-mainnet",
            "--pearl-nockchain-target-epoch-or-height", "42", "--pearl-extra-domain-data",
            "0xfeed", "--pearl-max-attempts", "16",
        ]);

        let puzzle = build_puzzle_inputs(&args).expect("pearl merge puzzle inputs");
        assert_eq!(puzzle.submission_mode, AiPowSubmissionMode::PearlMerge);
        assert!(puzzle.certificate_builder.is_none());
        let pearl = puzzle.pearl_merge.as_ref().expect("pearl config");
        assert_eq!(pearl.header_template.version, 1);
        assert_eq!(pearl.header_template.prev_block, [0x11; 32]);
        assert_eq!(pearl.header_template.timestamp, 1_717_171_717);
        assert_eq!(pearl.header_template.nbits, 0x207f_ffff);
        assert_eq!(pearl.mining_config.common_dim, 512);
        assert_eq!(pearl.mining_config.rank, 32);
        assert_eq!(pearl.max_pattern_len, 256);
        assert_eq!(pearl.mine_opts.max_attempts, Some(16));
        assert_eq!(pearl.aux_template.nockchain_chain_id, b"nockchain-mainnet");
        assert_eq!(pearl.aux_template.nockchain_target_epoch_or_height, 42);
        assert_eq!(pearl.aux_template.extra_domain_data, vec![0xfe, 0xed]);

        puzzle
            .validate_canonical_submission_ready()
            .expect("configured pearl merge submission should pass preflight");
    }

    #[test]
    fn cli_rejects_pearl_merge_without_required_header_fields() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-missing-header", "--submission-mode", "pearl-merge",
        ]);

        let err = match build_puzzle_inputs(&args) {
            Ok(_) => panic!("missing Pearl header must fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("--pearl-prev-block"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn cli_rejects_pearl_merge_noncanonical_recursive_params_before_mining() {
        let args = Args::parse_from([
            "ai-pow-mine", "--mining-pkh",
            "9yPePjfWAdUnzaQKyxcRXKRa5PpUzKKEwtpECBZsUYt9Jd7egSDEWoV", "--synth-seed",
            "ai-pow-pearl-merge-bad-params", "--submission-mode", "pearl-merge", "--m", "16",
            "--n", "8", "--spot-checks", "2", "--pearl-prev-block",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--pearl-timestamp", "1717171717", "--pearl-nbits", "0x207fffff",
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

    #[test]
    fn recursive_certificate_builder_rejects_noncanonical_ncmn_before_zkp() {
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
        sol.nonce = build_ncmn_nonce(
            &NonceAnchors {
                nck_commitment: [7; 32],
                external_commitment: Some([9; 32]),
            },
            0,
        );

        let build = puzzle
            .certificate_builder
            .as_ref()
            .expect("production builder configured");
        let err = build(&sol).expect_err("external anchor must not build a recursive certificate");
        assert!(
            err.to_string().contains(
                "refusing to build recursive certificate before successful matmul target check"
            ) && err.to_string().contains("external commitment"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn recursive_certificate_builder_rejects_nonce_anchor_substitution_before_zkp() {
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
        sol.nonce = build_ncmn_nonce(&NonceAnchors::nck_only([8; 32]), 0);

        let build = puzzle
            .certificate_builder
            .as_ref()
            .expect("production builder configured");
        let err = build(&sol).expect_err("wrong anchor must not build a recursive certificate");
        assert!(
            err.to_string().contains(
                "refusing to build recursive certificate before successful matmul target check"
            ) && err.to_string().contains("does not match candidate block"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn recursive_certificate_builder_rejects_multi_tile_full_matmul_gap_before_zkp() {
        let puzzle = build_puzzle_inputs(&test_args()).expect("test puzzle");
        assert!(puzzle.params.num_tiles() > 1);
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
        let sol =
            ai_pow_miner::mining::run(&job, &opts, MiningCancel::new()).expect("easy solution");

        let build = puzzle
            .certificate_builder
            .as_ref()
            .expect("production builder configured");
        let err = build(&sol)
            .expect_err("multi-tile selected-tile proof must not build a recursive certificate");
        assert!(
            err.to_string()
                .contains("recursive certificate proves one selected tile"),
            "unexpected error: {err}"
        );
    }
}
