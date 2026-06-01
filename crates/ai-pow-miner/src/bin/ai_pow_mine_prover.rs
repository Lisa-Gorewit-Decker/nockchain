//! `ai-pow-mine` — single-attempt mining CLI for the `ai-pow` PoW.
//!
//! The minimal entry point on top of `ai_pow_miner::mining::run`.
//! Useful for smoke tests, benchmark capture, and replaying captured
//! candidates in a shell without standing up a full node.
//!
//! Quick start (synth-matrices smoke test):
//! ```sh
//! ai-pow-mine --synth-seed smoke --nck-commitment 0xAB...AB \
//!             --target 0xFF..FF                # trivial target
//! ```

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ai_pow::params::MatmulParams;
use ai_pow_miner::{mining, MineOptions, MiningCancel, MiningError, MiningJob, NonceAnchors};
use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

/// `ai-pow-mine` — single-attempt block-mining CLI.
#[derive(Debug, Parser)]
#[command(
    name = "ai-pow-mine",
    about = "Single-attempt block miner for ai-pow.",
    version
)]
struct Args {
    /// Stable puzzle id bound into κ (32-byte hex; defaults to
    /// BLAKE3 of the matmul params if omitted).
    #[arg(long)]
    puzzle_id: Option<String>,

    /// Required Nockchain commitment (32-byte hex). Defaults to
    /// `[0xAB; 32]` for smoke tests.
    #[arg(
        long,
        default_value = "abababababababababababababababababababababababababababababababab"
    )]
    nck_commitment: String,

    /// Optional external-chain commitment (32-byte hex). Reserved
    /// for Pearl-compat etc.; absent ⇒ NCMN sentinel.
    #[arg(long)]
    external_commitment: Option<String>,

    /// Chain difficulty target (32-byte hex, little-endian).
    /// `FF…FF` (default) = trivial target, every hash wins.
    #[arg(
        long,
        default_value = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    )]
    target: String,

    // ── puzzle shape (defaults to TEST_SMALL: m=k=n=64, r=4, t=8, σ=8) ──
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

    // ── matrix inputs ──
    /// Path to raw i8 matrix A (length m·k). Mutually exclusive
    /// with --synth-seed.
    #[arg(long, value_name = "PATH")]
    a: Option<PathBuf>,
    /// Path to raw i8 matrix B (length k·n).
    #[arg(long, value_name = "PATH")]
    b: Option<PathBuf>,
    /// Synthesize A and B deterministically from this seed string
    /// (uses ai_pow::synth::synth_matrices). Mutually exclusive
    /// with --a / --b.
    #[arg(long)]
    synth_seed: Option<String>,

    // ── loop tuning ──
    #[arg(long, default_value_t = 0)]
    extranonce_start: u64,
    #[arg(long)]
    max_extranonces: Option<u64>,
    #[arg(long)]
    deadline_secs: Option<u64>,

    /// Optional path to write the encoded MatmulProof bytes.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
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
    // SAFETY: i8 and u8 share layout.
    Ok(bytes.into_iter().map(|b| b as i8).collect())
}

fn main() -> Result<()> {
    let args = Args::parse();

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

    // Matrices.
    let (a, b) = match (&args.a, &args.b, &args.synth_seed) {
        (Some(ap), Some(bp), None) => {
            let a = load_matrix(ap, (args.m * args.k) as usize, "A")?;
            let b = load_matrix(bp, (args.k * args.n) as usize, "B")?;
            (a, b)
        }
        (None, None, Some(seed)) => {
            let (a, b) = ai_pow::synth::synth_matrices(seed.as_bytes(), &params);
            (a, b)
        }
        _ => bail!("provide either --a + --b OR --synth-seed (not both, not neither)"),
    };

    // Puzzle id: explicit, or BLAKE3 of params for smoke runs.
    let puzzle_id = match &args.puzzle_id {
        Some(s) => parse_hex_32(s, "--puzzle-id")?.to_vec(),
        None => blake3::hash(ai_pow::prover::params_tag(&params).as_slice())
            .as_bytes()
            .to_vec(),
    };

    let nck = parse_hex_32(&args.nck_commitment, "--nck-commitment")?;
    let ext = args
        .external_commitment
        .as_ref()
        .map(|s| parse_hex_32(s, "--external-commitment"))
        .transpose()?;
    let target = parse_hex_32(&args.target, "--target")?;
    let deadline = args
        .deadline_secs
        .map(|s| Instant::now() + Duration::from_secs(s));

    let anchors = NonceAnchors {
        nck_commitment: nck,
        external_commitment: ext,
    };
    let job = MiningJob {
        puzzle_id: &puzzle_id,
        anchors,
        params: &params,
        target,
        a: &a,
        b: &b,
    };
    let opts = MineOptions {
        extranonce_start: args.extranonce_start,
        max_extranonces: args.max_extranonces,
        deadline,
        prover: ai_pow::prover::ProverOptions::default(),
        progress_interval: Some(Duration::from_secs(2)),
    };

    eprintln!(
        "ai-pow-mine: m={} k={} n={} r={} t={} σ={} target={}",
        args.m,
        args.k,
        args.n,
        args.noise_rank,
        args.tile,
        args.spot_checks,
        &args.target[..16.min(args.target.len())],
    );
    let started = Instant::now();
    let cancel = MiningCancel::new();
    let result = mining::run(&job, &opts, cancel);

    match result {
        Ok(sol) => {
            eprintln!(
                "ai-pow-mine: ✓ solution: extranonce={} tile_idx={} matmul_attempts={} elapsed={:?} matmul_attempt_rate={:.2}/s",
                u64::from_be_bytes(sol.nonce[72..80].try_into().unwrap()),
                sol.found_idx,
                sol.stats.matmul_attempts_tried,
                started.elapsed(),
                sol.stats.matmul_attempt_rate_per_sec(),
            );
            // Stdout: the 80-byte nonce hex (for piping to a verifier).
            println!("{}", hex::encode(sol.nonce));
            if let Some(out) = args.output {
                let bytes = sol.proof.encode();
                fs::write(&out, &bytes)
                    .with_context(|| format!("write proof to {}", out.display()))?;
                eprintln!(
                    "ai-pow-mine: wrote {} proof bytes → {}",
                    bytes.len(),
                    out.display()
                );
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("ai-pow-mine: ✗ {e}");
            // Exit codes: 2 for the loop-terminated-without-success cases,
            // 1 for real errors.
            let code = match e {
                MiningError::Cancelled
                | MiningError::DeadlineElapsed
                | MiningError::BudgetExhausted { .. } => 2,
                MiningError::Mine(_) => 1,
            };
            std::process::exit(code);
        }
    }
}
