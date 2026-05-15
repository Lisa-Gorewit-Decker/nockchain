//! F1 end-to-end harness — real `ai-pow` solve → `ai-pow-zk`
//! SNARK, instrumented.
//!
//! This is the substrate the GAP_AUDIT recommends: a genuine
//! cross-crate path (real matrices → real κ → SNARK binds the
//! chunk-Merkle commitment → real difficulty target →
//! `composite_verify_pow`) that doubles as the profiling /
//! benchmarking fixture. It is deliberately deterministic and
//! emits one machine-readable metrics line.
//!
//! ## What it exercises (today)
//!
//! - `ai-pow` real solve: `mine` at `TEST_SMALL` (difficulty_bits
//!   = 0 ⇒ a tile always clears), proving the puzzle path runs.
//! - `BlockContext` → `h_a_chunk` / `h_b_chunk` (M52 step 5).
//! - `ai-pow-zk` `place_matrix_hash_a` / `_b` into a
//!   `CompositeTrace` (the C3 matrix-binding path).
//! - `composite_prove` + `composite_verify_pow` against the real
//!   `difficulty_target` (C2).
//! - Hard assertion: the SNARK's derived `HASH_A` PI byte-equals
//!   `BlockContext::h_a_chunk` — the Pearl-byte-equivalent
//!   "unit of work" anchor.
//!
//! ## What it does NOT yet exercise (deep F1, tracked separately)
//!
//! The faithful jackpot→blake3 instruction chain that would make
//! `HASH_JACKPOT` / `JOB_KEY` / `COMMITMENT_HASH` non-zero PIs.
//! Those PIs are zero here (no such rows placed), so the C1/C4
//! bindings are vacuous and `composite_verify_pow` clears any
//! target. This is honest: the harness measures the matrix-
//! binding + prove/verify pipeline, which is what's wired.
//!
//! ## Running
//!
//!   cargo run -p ai-pow --release --features zk --example f1_harness
//!
//! Env knobs:
//!   F1_SEED   — matrix synth seed (default "f1-harness-v1")
//!   F1_ITERS  — prove+verify iterations for profiling (default 1)
//!
//! See `crates/ai-pow-zk/PROFILING.md` for samply / peak-RSS
//! recipes that wrap this binary.

#[cfg(not(feature = "zk"))]
fn main() {
    eprintln!("f1_harness requires the `zk` feature: cargo run -p ai-pow --release --features zk --example f1_harness");
    std::process::exit(2);
}

#[cfg(feature = "zk")]
fn main() {
    use std::time::Instant;

    use ai_pow::params::MatmulParams;
    use ai_pow::prover::{mine, BlockContext, ProverOptions};
    use ai_pow::synth::synth_matrices;
    use ai_pow::tile_hash::difficulty_target;
    use ai_pow_zk::composite_proof::build_config;
    use ai_pow_zk::{
        composite_prove, composite_verify_pow, CircuitConfig, CompositePublicInputs,
        CompositeTrace, ZkParams,
    };

    let seed = std::env::var("F1_SEED").unwrap_or_else(|_| "f1-harness-v1".to_string());
    let iters: u32 = std::env::var("F1_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    let params = MatmulParams::TEST_SMALL;
    let (a, b) = synth_matrices(seed.as_bytes(), &params);
    let block_commitment = b"f1-harness-block";
    let nonce = b"f1-harness-nonce";

    // 1. Real ai-pow solve (difficulty_bits = 0 ⇒ always finds a tile).
    let t = Instant::now();
    let plain = mine(
        block_commitment,
        nonce,
        &a,
        &b,
        &params,
        ProverOptions::default(),
    )
    .expect("mine must not error")
    .expect("difficulty_bits=0 ⇒ a tile always clears");
    let mine_ms = t.elapsed().as_millis();

    // 2. Per-block context: chunk-Merkle commitments + κ.
    let ctx =
        BlockContext::build(block_commitment, &a, &b, &params).expect("BlockContext build");

    // 3. Build the composite trace: matrix-hash A then B.
    let t = Instant::now();
    let a_bytes: Vec<u8> = a.iter().map(|&v| v as u8).collect();
    let b_bytes: Vec<u8> = b.iter().map(|&v| v as u8).collect();
    let mut trace = CompositeTrace::baseline_min();
    let (next, _root_a) = trace.place_matrix_hash_a(0, &a_bytes, &ctx.kappa);
    let (_end, _root_b) = trace.place_matrix_hash_b(next, &b_bytes, &ctx.kappa);
    let trace_gen_ms = t.elapsed().as_millis();

    // 4. Derive PIs and assert the cross-crate byte-equivalence
    //    anchor: SNARK HASH_A PI == ai-pow h_a_chunk.
    let pis = CompositePublicInputs::derive_from_trace(&trace);
    let expect_a: [u32; 8] = core::array::from_fn(|i| {
        u32::from_le_bytes([
            ctx.h_a_chunk[i * 4],
            ctx.h_a_chunk[i * 4 + 1],
            ctx.h_a_chunk[i * 4 + 2],
            ctx.h_a_chunk[i * 4 + 3],
        ])
    });
    assert_eq!(
        pis.hash_a, expect_a,
        "SNARK HASH_A PI must byte-equal ai-pow BlockContext.h_a_chunk"
    );

    // 5. Prove + PoW-verify (C2: against the real difficulty target).
    let zk_params = ZkParams {
        m: params.m,
        k: params.k,
        n: params.n,
        noise_rank: params.noise_rank,
        tile: params.tile,
        difficulty_bits: params.difficulty_bits,
    };
    let cfg = build_config(&zk_params, &CircuitConfig::TEST_PEARL);
    let target = difficulty_target(&params);

    let mut prove_ms_total = 0u128;
    let mut verify_ms_total = 0u128;
    let mut proof_bytes = 0usize;
    for _ in 0..iters {
        let trace_i = {
            let mut tr = CompositeTrace::baseline_min();
            let (n2, _) = tr.place_matrix_hash_a(0, &a_bytes, &ctx.kappa);
            tr.place_matrix_hash_b(n2, &b_bytes, &ctx.kappa);
            tr
        };
        let t = Instant::now();
        let proof = composite_prove(&cfg, trace_i, &pis);
        prove_ms_total += t.elapsed().as_millis();

        let t = Instant::now();
        composite_verify_pow(&cfg, &proof, &pis, &target)
            .expect("STARK valid + HASH_JACKPOT(=0) clears target");
        verify_ms_total += t.elapsed().as_millis();

        proof_bytes = bincode::serde::encode_to_vec(
            &proof,
            bincode::config::standard(),
        )
        .expect("encode")
        .len();
    }

    println!(
        "f1_harness shape=TEST_SMALL seed={seed} iters={iters} \
         mine_ms={mine_ms} trace_gen_ms={trace_gen_ms} \
         prove_ms={} verify_ms={} proof_bytes={proof_bytes} \
         num_pis={} plain_h_a_chunk_ok=true",
        prove_ms_total / iters as u128,
        verify_ms_total / iters as u128,
        pis.to_vec().len(),
    );
    let _ = &plain; // mine result kept alive for the solve assertion
}
