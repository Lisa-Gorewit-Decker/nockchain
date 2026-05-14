//! PROD-profile bench — M11.
//!
//! Runs the M9.1 composite tile AIR under `CircuitConfig::PROD`
//! (`log_blowup = 3`, `num_queries = 80`, target 120 bits of provable
//! FRI soundness) and reports proof size + rough timing.
//!
//! This test is `#[ignore]`d so the default `cargo test -p ai-pow-zk`
//! run stays fast (the PROD profile is several seconds per prove/
//! verify cycle even on a small trace). To run it:
//!
//! ```sh
//! cargo test -p ai-pow-zk --test prod_bench --release -- --ignored --nocapture
//! ```
//!
//! Bypasses the [`ai_pow_zk::prove`] / [`verify`] lib.rs entries to
//! pick `CircuitConfig::PROD` explicitly — the entries hardcode TEST
//! for fast iteration. M9.2+ will plumb the config selection through
//! the public API.

use std::time::Instant;

use ai_pow_zk::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
use ai_pow_zk::composite_air::MatmulTileAir;
use ai_pow_zk::params::ZkParams;
use ai_pow_zk::public::NUM_PUBLIC_INPUTS;
use ai_pow_zk::Val;
use p3_uni_stark::{prove, verify};

fn bench_params() -> ZkParams {
    // Smallest shape that still exercises the full constraint set:
    // k = 16, r = 2 → 8 stripes (power of 2), each accumulating an
    // r=2-wide INT8 dot product.
    ZkParams {
        m: 8,
        k: 16,
        n: 8,
        noise_rank: 2,
        tile: 2,
        difficulty_bits: 0,
    }
}

#[test]
#[ignore = "PROD profile is slow; run explicitly with --ignored"]
fn prod_profile_round_trip() {
    let p = bench_params();
    let cfg: AiPowStarkConfig = build_stark_config(&p, &CircuitConfig::PROD);
    let air = MatmulTileAir::<2>::new();

    // Same kind of witness data the lib.rs MVP test uses, distilled
    // down to the (a, b) tile-row + tile-col the composite AIR needs.
    let a: Vec<i8> = (0..p.k as usize)
        .map(|i| (((i as i32) * 5 - 40) % 64) as i8)
        .collect();
    let b: Vec<i8> = (0..p.k as usize)
        .map(|i| (((i as i32) * 7 - 30) % 64) as i8)
        .collect();
    let trace = MatmulTileAir::<2>::generate_trace(&a, &b);
    let pis = vec![Val::default(); NUM_PUBLIC_INPUTS];

    let t0 = Instant::now();
    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
    let prove_ms = t0.elapsed().as_millis();

    let proof_bytes = bincode::serde::encode_to_vec(&proof, bincode::config::standard())
        .expect("PROD proof must serialise");

    let t1 = Instant::now();
    verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis).expect("PROD trace must verify");
    let verify_ms = t1.elapsed().as_millis();

    eprintln!(
        "[PROD bench] prove = {prove_ms} ms | verify = {verify_ms} ms | proof = {} bytes",
        proof_bytes.len()
    );

    // Sanity sentinel: the PROD profile must produce a non-trivial
    // proof (the TEST profile already does, with 8 queries; PROD with
    // 80 queries should be larger). 1 KiB is a generous lower bound
    // that catches "proof object accidentally empty" regressions.
    assert!(
        proof_bytes.len() > 1024,
        "PROD proof unexpectedly small ({} bytes)",
        proof_bytes.len()
    );
}

#[test]
#[ignore = "PROD profile is slow; run explicitly with --ignored"]
fn prod_profile_rejects_tampered_witness() {
    // Verifier soundness check under PROD: an incorrect trace must
    // also reject at 120-bit security. This is the same logic the
    // TEST-profile tests exercise, just at the higher security level.
    let p = bench_params();
    let cfg: AiPowStarkConfig = build_stark_config(&p, &CircuitConfig::PROD);
    let air = MatmulTileAir::<2>::new();

    let a: Vec<i8> = (0..p.k as usize).map(|i| (i as i8) - 8).collect();
    let b: Vec<i8> = (0..p.k as usize).map(|i| (i as i8) - 6).collect();
    let mut trace = MatmulTileAir::<2>::generate_trace(&a, &b);
    let pis = vec![Val::default(); NUM_PUBLIC_INPUTS];

    // Corrupt c_out at row 0 (col 1).
    use p3_field::integers::QuotientMap;
    trace.values[1] = <Val as QuotientMap<u64>>::from_int(424242);

    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
    let r = verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis);
    assert!(
        r.is_err(),
        "PROD verifier must reject tampered trace; got {r:?}"
    );
}
