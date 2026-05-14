//! Lib-level prove/verify wrappers for the M10.1c composite AIR.
//!
//! Wraps [`p3_uni_stark::prove`] / [`p3_uni_stark::verify`] with
//! the M10.1c composite stack (config + AIR + trace + public
//! inputs) so callers don't have to assemble it manually.
//!
//! This is Phase 14's **structural** deliverable. Phase 14's full
//! deliverable also includes switching to a lookup-aware folder
//! (LogUp interactions reified at proof time); that's bundled
//! with the instruction-list compiler (Phase 13b) and lands
//! together when both are ready.
//!
//! ## Public-input shape
//!
//! Currently the composite proof carries **no public inputs** —
//! the baseline trace shape is fully determined by `TOTAL_TRACE_WIDTH
//! × N`. Phase 13b will add public inputs for:
//!   * The instruction list's terminal CV (post-finalize hash).
//!   * The terminal CUMSUM_TILE (matmul accumulator).
//!   * The terminal JACKPOT_MSG (jackpot state).
//!
//! These will be bound to specific trace cells via the existing
//! [`crate::public::PublicInputs`] machinery once the instruction
//! compiler lands.

use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
use crate::composite_full_air::CompositeFullAir;
use crate::composite_public::CompositePublicInputs;
use crate::composite_trace::CompositeTrace;
use crate::params::ZkParams;

use p3_commit::Pcs;
use p3_uni_stark::{prove, verify, Proof, StarkGenericConfig, Val, VerificationError};

/// Concrete type of the verification error for the composite AIR.
/// Equivalent to `VerificationError<PcsError<AiPowStarkConfig>>`.
pub type CompositeVerificationError = VerificationError<
    <<AiPowStarkConfig as StarkGenericConfig>::Pcs as Pcs<
        <AiPowStarkConfig as StarkGenericConfig>::Challenge,
        <AiPowStarkConfig as StarkGenericConfig>::Challenger,
    >>::Error,
>;

/// Build the composite STARK config for the given parameters +
/// profile. Re-export of [`build_stark_config`] for ergonomics.
pub fn build_config(params: &ZkParams, profile: &CircuitConfig) -> AiPowStarkConfig {
    build_stark_config(params, profile)
}

/// Prove the composite AIR against a given trace + public inputs.
///
/// `trace` must be a [`CompositeTrace`] whose internal matrix has
/// width [`crate::composite_layout::TOTAL_TRACE_WIDTH`] and height
/// a power of 2 ≥ `MIN_STARK_LEN`. `public_inputs` must match the
/// trace's last-row CUMSUM_TILE / JACKPOT_MSG cells — the AIR
/// enforces this binding.
///
/// The returned [`Proof`] can be serialised via [`bincode`] for
/// transport.
pub fn composite_prove(
    config: &AiPowStarkConfig,
    trace: CompositeTrace,
    public_inputs: &CompositePublicInputs,
) -> Proof<AiPowStarkConfig> {
    let pis = public_inputs.to_vec();
    prove::<AiPowStarkConfig, _>(config, &CompositeFullAir, trace.matrix, &pis)
}

/// Verify a composite proof against the claimed public inputs.
/// Returns `Ok(())` if valid; otherwise a
/// [`CompositeVerificationError`] describing the failure.
pub fn composite_verify(
    config: &AiPowStarkConfig,
    proof: &Proof<AiPowStarkConfig>,
    public_inputs: &CompositePublicInputs,
) -> Result<(), CompositeVerificationError> {
    let pis = public_inputs.to_vec();
    verify::<AiPowStarkConfig, _>(config, &CompositeFullAir, proof, &pis)
}

#[allow(dead_code)]
fn _suppress_unused_val_import(_v: Val<AiPowStarkConfig>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_zk_params() -> ZkParams {
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
    fn composite_prove_verify_round_trip() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis).expect("composite proof must verify");
    }

    #[test]
    fn composite_proof_is_serializable() {
        // The proof type derives Serialize/Deserialize (see crates/
        // ai-pow-zk/Cargo.toml for the bincode dep). Verifying a
        // bincode round-trip is the structural soundness check
        // every lib-level consumer cares about.
        use bincode::config::standard as bincode_standard;

        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);

        let encoded =
            bincode::serde::encode_to_vec(&proof, bincode_standard()).expect("encode");
        let (decoded, _len) = bincode::serde::decode_from_slice::<Proof<AiPowStarkConfig>, _>(
            &encoded,
            bincode_standard(),
        )
        .expect("decode");
        composite_verify(&cfg, &decoded, &pis).expect("decoded proof verifies");
    }

    /// Two proofs over baseline traces of different sizes both
    /// verify with the same config (the config is per-params, not
    /// per-trace-size, in TEST_PEARL).
    #[test]
    fn composite_proofs_at_two_trace_sizes() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        let trace_small = CompositeTrace::baseline_min();
        let pis_small = CompositePublicInputs::derive_from_trace(&trace_small);
        let p_small = composite_prove(&cfg, trace_small, &pis_small);
        composite_verify(&cfg, &p_small, &pis_small).expect("small proof");

        let trace_big =
            CompositeTrace::baseline(crate::composite_layout::MIN_STARK_LEN * 2);
        let pis_big = CompositePublicInputs::derive_from_trace(&trace_big);
        let p_big = composite_prove(&cfg, trace_big, &pis_big);
        composite_verify(&cfg, &p_big, &pis_big).expect("big proof");
    }

    // =================================================================
    //  Public-input binding tests
    // =================================================================

    /// Tamper a PI element on the verifier side; verification
    /// rejects (the AIR's `when_last_row` constraint forces the
    /// trace's last-row CUMSUM_TILE to match `pis[0..4]`).
    #[test]
    fn verify_rejects_wrong_cumsum_pi() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);

        let mut bad_pis = pis.clone();
        bad_pis.cumsum[0] = 42; // baseline has 0 everywhere; 42 is wrong.

        assert!(
            composite_verify(&cfg, &proof, &bad_pis).is_err(),
            "wrong CUMSUM PI must reject"
        );
    }

    /// Tamper a JACKPOT PI element on the verifier side.
    #[test]
    fn verify_rejects_wrong_jackpot_pi() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);

        let mut bad_pis = pis.clone();
        bad_pis.jackpot[5] = 0xDEAD_BEEF;

        assert!(
            composite_verify(&cfg, &proof, &bad_pis).is_err(),
            "wrong JACKPOT PI must reject"
        );
    }

    /// Build a trace with threaded non-zero cumsum + jackpot;
    /// PIs derived from it; prove + verify succeeds.
    #[test]
    fn prove_verify_with_threaded_state() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        // Thread a non-zero state through to the last row.
        trace.fill_cumsum_passthrough(0, &[1, -2, 3, -4]);
        let jp: [u32; 16] = core::array::from_fn(|i| (i as u32 + 1) * 0x12345);
        trace.fill_jackpot_passthrough(0, &jp);

        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_eq!(pis.cumsum, [1, -2, 3, -4]);
        assert_eq!(pis.jackpot, jp);

        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis)
            .expect("threaded-state proof must verify with matching PIs");
    }

    /// PIs are part of the verification call, so swapping a
    /// proof's PIs for another proof's still rejects.
    #[test]
    fn verify_rejects_pi_substitution_across_proofs() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        // Proof A: baseline trace + zero PIs.
        let trace_a = CompositeTrace::baseline_min();
        let pis_a = CompositePublicInputs::derive_from_trace(&trace_a);
        let proof_a = composite_prove(&cfg, trace_a, &pis_a);

        // Proof B: threaded state + non-zero PIs.
        let mut trace_b = CompositeTrace::baseline_min();
        trace_b.fill_cumsum_passthrough(0, &[1, 1, 1, 1]);
        let pis_b = CompositePublicInputs::derive_from_trace(&trace_b);
        let _proof_b = composite_prove(&cfg, trace_b, &pis_b);

        // Verifying proof A against B's PIs must reject.
        assert!(
            composite_verify(&cfg, &proof_a, &pis_b).is_err(),
            "proof A with B's PIs must reject"
        );
    }

    /// PROD-shape bench. Ignored by default — run with
    /// `cargo test --release composite_proof_prod_bench -- --ignored --nocapture`.
    ///
    /// Measures prove + verify wall-clock for the baseline trace
    /// at MIN_STARK_LEN under [`CircuitConfig::PROD`] (`log_blowup
    /// = 3`, `num_queries = 80` — 120 bits of provable FRI
    /// soundness). The baseline trace has no chip activity, so
    /// this bench is a structural ceiling: real proofs with
    /// matmul / BLAKE3 activity will take longer because the
    /// dot-product / round constraints actually evaluate to
    /// non-trivial polynomials.
    #[test]
    #[ignore = "PROD bench — expensive; run with --ignored"]
    fn composite_proof_prod_bench() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::PROD);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);

        let t0 = std::time::Instant::now();
        let proof = composite_prove(&cfg, trace, &pis);
        let prove_ms = t0.elapsed().as_millis();

        let t1 = std::time::Instant::now();
        composite_verify(&cfg, &proof, &pis).expect("PROD verify");
        let verify_ms = t1.elapsed().as_millis();

        // Serialise to measure proof size.
        use bincode::config::standard as bincode_standard;
        let bytes = bincode::serde::encode_to_vec(&proof, bincode_standard())
            .expect("encode");
        let proof_bytes = bytes.len();

        println!(
            "ai-pow-zk PROD bench (composite baseline @ MIN_STARK_LEN = {} rows × {} cols):",
            crate::composite_layout::MIN_STARK_LEN,
            crate::composite_layout::TOTAL_TRACE_WIDTH
        );
        println!("  prove    : {prove_ms} ms");
        println!("  verify   : {verify_ms} ms");
        println!("  proof    : {proof_bytes} bytes");
    }
}
