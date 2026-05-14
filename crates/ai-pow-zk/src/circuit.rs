//! Plonky3 `StarkConfig` factory for the matmul puzzle.
//!
//! This module owns the choice of:
//!
//! - **Prime field**: BabyBear (`p3_baby_bear::BabyBear`) on the trace,
//!   with an extension field for FRI challenges.
//! - **Hash for FRI**: Poseidon2 over BabyBear (`p3_poseidon2`).
//! - **Merkle tree**: `p3_merkle_tree` over the FRI commitment hash.
//! - **PoW grinding** at the FRI challenger.
//! - **Rate, blowup factor, and proof-of-work bits** so the resulting
//!   proof size lands inside the certificate budget.
//!
//! All of this is TBD; the type is a placeholder so other crates can
//! import [`CircuitConfig`] and pass it through without circular feature
//! issues.

use crate::params::ZkParams;

/// Configuration knobs for the Plonky3 STARK over the matmul AIR.
#[derive(Debug, Clone, Copy)]
pub struct CircuitConfig {
    /// Log2 of the FRI blowup factor (rate). Pearl's analog uses 7 at
    /// `degree_bits >= 15` and 7 elsewhere; we'll tune empirically.
    pub log_blowup: u32,
    /// Plonky3 PoW grinding bits at the FRI challenger.
    pub pow_bits: u32,
    /// Number of FRI queries.
    pub num_queries: u32,
}

impl CircuitConfig {
    /// Production defaults. Numbers borrowed from Pearl's `prove_block`
    /// (`pearl/zk-pow/src/api/prove.rs:49-54`) as a starting point; will
    /// shift as the Plonky3 AIR's trace size differs from Pearl's STARK.
    pub const PROD: Self = Self {
        log_blowup: 1,
        pow_bits: 18,
        num_queries: 80,
    };

    /// Small profile for unit tests once the circuit is real.
    pub const TEST: Self = Self {
        log_blowup: 1,
        pow_bits: 0,
        num_queries: 16,
    };
}

/// Build the Plonky3 `StarkConfig` for a given `MatmulParams` + `CircuitConfig`.
///
/// Returns the concrete type the prover and verifier both consume.
pub fn build_stark_config(_params: &ZkParams, _config: &CircuitConfig) {
    // Will return something like `StarkConfig<Pcs, Challenge, Challenger>`
    // once the FRI + Poseidon2 + Merkle stack is wired together.
    todo!(
        "instantiate Plonky3 StarkConfig: Poseidon2-BabyBear hash, \
         MerkleTreeMmcs, TwoAdicFriPcs with the chosen log_blowup / \
         num_queries / pow_bits"
    )
}
