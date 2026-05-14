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

/// Prove the composite AIR against a given trace.
///
/// `trace` must be a [`CompositeTrace`] whose internal matrix has
/// width [`crate::composite_layout::TOTAL_TRACE_WIDTH`] and height
/// a power of 2 ≥ `MIN_STARK_LEN`. The returned [`Proof`] can be
/// serialised via [`bincode`] for transport.
pub fn composite_prove(
    config: &AiPowStarkConfig,
    trace: CompositeTrace,
) -> Proof<AiPowStarkConfig> {
    prove::<AiPowStarkConfig, _>(config, &CompositeFullAir, trace.matrix, &[])
}

/// Verify a composite proof. Returns `Ok(())` if valid; otherwise
/// a [`CompositeVerificationError`] describing the failure.
pub fn composite_verify(
    config: &AiPowStarkConfig,
    proof: &Proof<AiPowStarkConfig>,
) -> Result<(), CompositeVerificationError> {
    verify::<AiPowStarkConfig, _>(config, &CompositeFullAir, proof, &[])
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
        let proof = composite_prove(&cfg, trace);
        composite_verify(&cfg, &proof)
            .expect("composite proof must verify");
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
        let proof = composite_prove(&cfg, trace);

        let encoded =
            bincode::serde::encode_to_vec(&proof, bincode_standard()).expect("encode");
        let (decoded, _len) = bincode::serde::decode_from_slice::<Proof<AiPowStarkConfig>, _>(
            &encoded,
            bincode_standard(),
        )
        .expect("decode");
        composite_verify(&cfg, &decoded).expect("decoded proof verifies");
    }

    /// Two proofs over baseline traces of different sizes both
    /// verify with the same config (the config is per-params, not
    /// per-trace-size, in TEST_PEARL).
    #[test]
    fn composite_proofs_at_two_trace_sizes() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        let trace_small = CompositeTrace::baseline_min();
        let p_small = composite_prove(&cfg, trace_small);
        composite_verify(&cfg, &p_small).expect("small proof");

        let trace_big =
            CompositeTrace::baseline(crate::composite_layout::MIN_STARK_LEN * 2);
        let p_big = composite_prove(&cfg, trace_big);
        composite_verify(&cfg, &p_big).expect("big proof");
    }
}
