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

use ai_pow_zk::blake3_chip::{generate_trace_for_calls, Blake3HashCall};
use ai_pow_zk::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
use ai_pow_zk::composite_air::{MatmulTileAir, NUM_AIR_PUBLIC_VALUES, PI_M_FINAL_IDX};
use ai_pow_zk::found_leaf_air::{build_public_values as build_hash_pis, Blake3FoundLeafAir};
use ai_pow_zk::params::ZkParams;
use ai_pow_zk::public::PublicInputs;
use ai_pow_zk::{binding, Val};
use p3_field::integers::QuotientMap;
use p3_uni_stark::{prove, verify};

/// Build a placeholder public-values vector for bench traces. The 42
/// Pearl public-input slots stay zero (Fiat-Shamir-only at this AIR
/// level) and `pis[PI_M_FINAL_IDX]` is set to the trace's expected
/// `m_final` so M10.1a's last-row constraint is satisfied.
fn bench_pis<const STRIPE: usize>(a: &[i8], b: &[i8]) -> Vec<Val> {
    let mut pis = vec![Val::default(); NUM_AIR_PUBLIC_VALUES];
    let m_final = MatmulTileAir::<STRIPE>::reference_final_state(a, b);
    pis[PI_M_FINAL_IDX] = <Val as QuotientMap<u32>>::from_int(m_final);
    pis
}

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
    let pis = bench_pis::<2>(&a, &b);

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

/// M10.1b bench: the full found-leaf binding adds a *second* PROD-
/// profile proof per attempt — the Blake3FoundLeafAir hash proof.
/// This test measures the combined cost so the integration overhead
/// is visible (the composite proof is what the `prod_profile_round_trip`
/// test above already covers).
#[test]
#[ignore = "PROD profile is slow; run explicitly with --ignored"]
fn prod_profile_found_leaf_air_round_trip() {
    let p = bench_params();
    let cfg: AiPowStarkConfig = build_stark_config(&p, &CircuitConfig::PROD);
    let air = Blake3FoundLeafAir::new();

    // Use a fixture (m_final, pow_key) and compute the matching
    // found_leaf via the M10.1a out-of-circuit helper. The AIR
    // constrains all three.
    let m_final: u32 = 0x1234_5678;
    let pow_key = [0x42u8; 32];
    let pi = PublicInputs {
        params_tag: [0; 32],
        h_a: [0; 32],
        h_b: [0; 32],
        comm_m: [0; 32],
        found_i: 0,
        found_j: 0,
        found_leaf: binding::compute_found_leaf(m_final, &pow_key),
    };

    let mut message = [0u32; 16];
    message[0] = m_final;
    let mut key = [0u32; 8];
    for i in 0..8 {
        let mut b = [0u8; 4];
        b.copy_from_slice(&pow_key[i * 4..(i + 1) * 4]);
        key[i] = u32::from_le_bytes(b);
    }
    let calls = vec![Blake3HashCall {
        message,
        key,
        counter: 0,
        block_len: 64,
        flags: 0x1B,
    }];
    let trace = generate_trace_for_calls::<Val>(&calls, CircuitConfig::PROD.log_blowup as usize);
    let pis = build_hash_pis::<Val>(m_final, &pow_key, &pi.found_leaf);

    let t0 = Instant::now();
    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
    let prove_ms = t0.elapsed().as_millis();
    let proof_bytes = bincode::serde::encode_to_vec(&proof, bincode::config::standard())
        .expect("hash proof must serialise");

    let t1 = Instant::now();
    verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis)
        .expect("M10.1b in-circuit found-leaf binding must verify at PROD");
    let verify_ms = t1.elapsed().as_millis();

    eprintln!(
        "[PROD bench, M10.1b hash leg] prove = {prove_ms} ms | verify = {verify_ms} ms | \
         proof = {} bytes",
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
    let pis = bench_pis::<2>(&a, &b);

    // Corrupt c_out at row 0 (col 1).
    trace.values[1] = <Val as QuotientMap<u64>>::from_int(424242);

    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
    let r = verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis);
    assert!(
        r.is_err(),
        "PROD verifier must reject tampered trace; got {r:?}"
    );
}
