//! Production caller — drives the ai-pow-zk → Plonky3-recursion
//! pipeline at production parameters and measures it.
//!
//! Run one trace size per process (so peak RSS is clean):
//!
//! ```text
//! RUSTFLAGS="-Ctarget-cpu=native" \
//!   cargo run --release --example prod_recursion_measure \
//!   --features recursion -- <log2_trace_height>
//! ```
//!
//! `ZkParams` is `ai_pow::params::MatmulParams::LLAMA_3_1_8B_GATE_UP`
//! — the real shipped `pearl-ai/Llama-3.1-8B-Instruct-pearl` FFN
//! gate/up GEMM, the production mining target — and the FRI profile
//! is `CircuitConfig::PROD` (log_blowup = 4, num_queries = 15). At the
//! Pearl-faithful `tile = 8` (`h·w = 64`, Pearl's `default_mining_
//! config`) the production composite trace for that model is
//! **2^15 rows** (the `ai_pow::zk_bridge::expected_layer0_rows`
//! budget — strip Merkle + §6(b) sweep + noised store — rounds up to
//! 2^15). The earlier `tile = 64` over-tiling put it at 2^19.
//!
//! STARK proving is data-oblivious (same FFTs / Merkle tree / FRI
//! folding regardless of cell values), so a zero-activity
//! `CompositeTrace::baseline(n)` at height `n` gives a faithful
//! size + time + memory measurement without the 16 GB model weights:
//! the recursion cost is fixed by trace dimensions and FRI params.

use ai_pow_zk::composite_layout::TOTAL_TRACE_WIDTH;
use ai_pow_zk::recursion::recurse_composite_to_l1;
use ai_pow_zk::{CircuitConfig, CompositeTrace, ZkParams};

/// Production composite-trace height for `LLAMA_3_1_8B_GATE_UP` at
/// the Pearl-faithful `tile = 8`.
const PROD_TRACE_LOG2: u32 = 15;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let log2_n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    let n = 1usize << log2_n;

    // The production mining target — Llama-3.1-8B FFN gate/up GEMM.
    let zk = ZkParams {
        m: 4096,
        k: 4096,
        n: 14336,
        noise_rank: 64,
        tile: 8,
        difficulty_bits: 0,
    };
    let profile = CircuitConfig::PROD;

    // Inner-composite main-trace LDE memory projection — the
    // dominant single allocation (log_blowup = 4 ⇒ 16× blowup).
    let lde_rows = (n as u128) << 4;
    let main_lde_bytes = lde_rows * (TOTAL_TRACE_WIDTH as u128) * 8;
    let gb = |b: u128| b as f64 / (1u64 << 30) as f64;

    eprintln!("══ ai-pow-zk → Plonky3-recursion · production caller ══");
    eprintln!("target model : Llama-3.1-8B-Instruct-pearl — FFN gate/up GEMM");
    eprintln!("ZkParams     : m=4096 k=4096 n=14336 r=64 tile=8");
    eprintln!("FRI profile  : CircuitConfig::PROD (log_blowup=4, num_queries=15, pow_bits=1)");
    eprintln!("trace width  : {TOTAL_TRACE_WIDTH} columns");
    eprintln!(
        "trace height : 2^{log2_n} = {n} rows   (production target = 2^{PROD_TRACE_LOG2} = {})",
        1usize << PROD_TRACE_LOG2
    );
    eprintln!(
        "projected inner main-trace LDE (16×): {:.2} GB  \
         — peak RSS is higher (permutation-trace LDE + quotient + Merkle trees)",
        gb(main_lde_bytes)
    );
    eprintln!();

    let trace = CompositeTrace::baseline(n);

    eprintln!("running… (L0 composite prove → L1 verifier circuit → L1 outer cert)");
    let run = match recurse_composite_to_l1(&zk, &profile, trace) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FAILED at 2^{log2_n}: {e:?}");
            std::process::exit(1);
        }
    };

    let composite_bytes = postcard::to_allocvec(&run.composite_proof)
        .expect("serialize composite proof")
        .len();
    let l1_cert_bytes = postcard::to_allocvec(&run.l1_cert)
        .expect("serialize L1 certificate")
        .len();

    let kb = |b: usize| b as f64 / 1024.0;
    let s = |ms: u128| ms as f64 / 1000.0;
    let total = run.composite_prove_ms
        + run.l1_circuit_build_ms
        + run.l1_in_circuit_verify_ms
        + run.l1_outer_cert_ms;

    eprintln!();
    eprintln!(
        "── results · composite trace 2^{log2_n} ({n} rows × {} cols) ──",
        run.composite_trace_width
    );
    eprintln!("L0  composite prove        : {:>9.2} s", s(run.composite_prove_ms));
    eprintln!("L1  verifier-circuit build : {:>9.2} s", s(run.l1_circuit_build_ms));
    eprintln!("L1  in-circuit verify (S3) : {:>9.2} s", s(run.l1_in_circuit_verify_ms));
    eprintln!("L1  outer cert    (S5)     : {:>9.2} s", s(run.l1_outer_cert_ms));
    eprintln!("    TOTAL wall-clock       : {:>9.2} s", s(total));
    eprintln!();
    eprintln!("composite (L0) proof       : {:>9.1} KB", kb(composite_bytes));
    eprintln!("L1 recursive certificate   : {:>9.1} KB", kb(l1_cert_bytes));
    // Machine-greppable row: log2_n,n,width,prove_ms,build_ms,verify_ms,cert_ms,L0_bytes,L1_bytes
    println!(
        "CSV,{log2_n},{n},{},{},{},{},{},{},{}",
        run.composite_trace_width,
        run.composite_prove_ms,
        run.l1_circuit_build_ms,
        run.l1_in_circuit_verify_ms,
        run.l1_outer_cert_ms,
        composite_bytes,
        l1_cert_bytes,
    );
}
