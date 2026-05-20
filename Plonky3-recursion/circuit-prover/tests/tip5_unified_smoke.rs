//! M-S5b S1.B Poseidon2-removal P2 — toy STARK roundtrip at the
//! [`goldilocks_tip5_80bit`] config (Tip5-throughout outer-cert).
//!
//! Validates that the Tip5-unified Goldilocks STARK config produces
//! verifiable proofs end-to-end on a non-trivial AIR (Fibonacci,
//! degree 2), and that tampered proofs reject.
//!
//! P1 + P2 sub-task of `crates/ai-pow-zk/docs/2026-05-20_POSEIDON2_REMOVAL_SPEC.md`.

use p3_circuit::test_utils::{FibonacciAir, generate_trace_rows};
use p3_circuit_prover::config::goldilocks_tip5_80bit;
use p3_field::PrimeCharacteristicRing;
use p3_goldilocks::Goldilocks;
use p3_uni_stark::{prove, verify};

type F = Goldilocks;

/// Toy STARK roundtrip at `goldilocks_tip5_80bit()`.
/// Honest Fibonacci AIR proof must verify.
#[test]
fn fibonacci_prove_verify_at_tip5_80bit() {
    let n = 1 << 3;
    let x = 21u64; // Fibonacci(8) = 21

    let cfg = goldilocks_tip5_80bit();
    let trace = generate_trace_rows::<F>(0, 1, n);
    let pis = vec![F::ZERO, F::ONE, F::from_u64(x)];
    let air = FibonacciAir {};

    let proof = prove(&cfg, &air, trace, &pis);
    verify(&cfg, &air, &proof, &pis).expect("Fibonacci proof at tip5-80bit MUST verify");
}

/// Tamper test at `goldilocks_tip5_80bit()`. Wrong PI rejects.
#[test]
fn fibonacci_tampered_pi_at_tip5_80bit_rejects() {
    let n = 1 << 3;

    let cfg = goldilocks_tip5_80bit();
    let trace = generate_trace_rows::<F>(0, 1, n);
    let pis_honest = vec![F::ZERO, F::ONE, F::from_u64(21u64)];
    let air = FibonacciAir {};

    let proof = prove(&cfg, &air, trace, &pis_honest);

    let pis_tampered = vec![F::ZERO, F::ONE, F::from_u64(22u64)];
    assert!(
        verify(&cfg, &air, &proof, &pis_tampered).is_err(),
        "tampered PI MUST reject at tip5-80bit config"
    );
}

// NOTE: a `fibonacci_prove_verify_at_tip5_80bit_higharity` test
// existed here through 2026-05-20. It exercised the
// `goldilocks_tip5_80bit_higharity()` builder, which has now been
// folded into the production `goldilocks_tip5_80bit()` builder
// (high-arity FRI fold + non-trivial final polynomial were rolled
// into the production cumulative-lever stack). The sibling builder
// no longer exists; the equivalent coverage now lives in the
// `fibonacci_prove_verify_at_tip5_80bit` test above.
