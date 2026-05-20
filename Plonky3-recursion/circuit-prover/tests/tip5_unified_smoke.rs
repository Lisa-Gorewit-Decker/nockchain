//! M-S5b S1.B Poseidon2-removal P2 — toy STARK roundtrip at the new
//! [`goldilocks_tip5_unified_80bit`] config. Validates that the
//! Tip5-unified Goldilocks STARK config produces verifiable proofs
//! end-to-end (prove + verify) on a non-trivial AIR (Fibonacci, degree
//! 2), and that tampered proofs reject.
//!
//! This is the P2 deliverable per
//! `crates/ai-pow-zk/docs/2026-05-20_POSEIDON2_REMOVAL_SPEC.md` §4.3.
//! Builds on P1 (the type alias + builder; landed in `circuit-prover/src/config.rs`).
//!
//! Mirrors the existing `goldilocks_tip5_120bit()`-style usage but
//! with the new Tip5 hash throughout — no Poseidon2 anywhere in the
//! STARK config.

use p3_circuit::test_utils::{FibonacciAir, generate_trace_rows};
use p3_circuit_prover::config::{
    goldilocks_tip5_unified_80bit, goldilocks_tip5_unified_80bit_higharity,
};
use p3_field::PrimeCharacteristicRing;
use p3_goldilocks::Goldilocks;
use p3_uni_stark::{prove, verify};

type F = Goldilocks;

/// P2 — Toy STARK roundtrip at `goldilocks_tip5_unified_80bit()`.
/// Honest Fibonacci AIR proof must verify.
#[test]
fn p2_fibonacci_prove_verify_at_tip5_unified() {
    let n = 1 << 3;
    let x = 21u64; // Fibonacci(8) = 21

    let cfg = goldilocks_tip5_unified_80bit();
    let trace = generate_trace_rows::<F>(0, 1, n);
    let pis = vec![F::ZERO, F::ONE, F::from_u64(x)];
    let air = FibonacciAir {};

    let proof = prove(&cfg, &air, trace, &pis);
    verify(&cfg, &air, &proof, &pis).expect("Fibonacci proof at tip5-unified MUST verify");
}

/// P2 — Tamper test at `goldilocks_tip5_unified_80bit()`. Wrong
/// public input must reject.
#[test]
fn p2_fibonacci_tampered_pi_at_tip5_unified_rejects() {
    let n = 1 << 3;

    let cfg = goldilocks_tip5_unified_80bit();
    let trace = generate_trace_rows::<F>(0, 1, n);
    let pis_honest = vec![F::ZERO, F::ONE, F::from_u64(21u64)];
    let air = FibonacciAir {};

    let proof = prove(&cfg, &air, trace, &pis_honest);

    // Tamper: claim Fibonacci(8) = 22 (wrong; honest value is 21).
    let pis_tampered = vec![F::ZERO, F::ONE, F::from_u64(22u64)];
    assert!(
        verify(&cfg, &air, &proof, &pis_tampered).is_err(),
        "tampered PI MUST reject at tip5-unified config"
    );
}

/// P2 — High-arity sibling smoke test. Same Fibonacci roundtrip at
/// `goldilocks_tip5_unified_80bit_higharity()`. Validates that the
/// high-arity FRI variant produces verifiable proofs.
#[test]
fn p2_fibonacci_prove_verify_at_tip5_unified_higharity() {
    let n = 1 << 3;
    let x = 21u64;

    let cfg = goldilocks_tip5_unified_80bit_higharity();
    let trace = generate_trace_rows::<F>(0, 1, n);
    let pis = vec![F::ZERO, F::ONE, F::from_u64(x)];
    let air = FibonacciAir {};

    let proof = prove(&cfg, &air, trace, &pis);
    verify(&cfg, &air, &proof, &pis).expect(
        "Fibonacci proof at tip5-unified high-arity MUST verify",
    );
}

/// P2 — Parity check vs the legacy Poseidon2-based config.
/// Both configs prove + verify the same Fibonacci AIR; both succeed.
/// (Proof bytes will differ — the hash output is different — but
/// both verify successfully against their respective configs. This
/// is the de-risk check before P3's deeper parity testing.)
#[test]
fn p2_fibonacci_parity_tip5_unified_vs_poseidon2() {
    use p3_circuit_prover::config::goldilocks_tip5_120bit;

    let n = 1 << 3;
    let pis = vec![F::ZERO, F::ONE, F::from_u64(21u64)];
    let air = FibonacciAir {};

    // Tip5-unified path.
    let cfg_unified = goldilocks_tip5_unified_80bit();
    let trace_u = generate_trace_rows::<F>(0, 1, n);
    let proof_u = prove(&cfg_unified, &air, trace_u, &pis);
    verify(&cfg_unified, &air, &proof_u, &pis)
        .expect("tip5-unified Fibonacci must verify");

    // Legacy Poseidon2-based path.
    let cfg_poseidon2 = goldilocks_tip5_120bit();
    let trace_p = generate_trace_rows::<F>(0, 1, n);
    let proof_p = prove(&cfg_poseidon2, &air, trace_p, &pis);
    verify(&cfg_poseidon2, &air, &proof_p, &pis)
        .expect("legacy Poseidon2 Fibonacci must verify");

    // Both configs prove + verify the same AIR; this confirms the
    // new config is a drop-in replacement at the toy-STARK level.
    // (Proof size comparison is P4's job; this test is just smoke.)
}
