//! BLAKE3 sub-AIR wrapper.
//!
//! Wraps `p3-blake3-air` (the upstream Plonky3 BLAKE3 AIR) so the
//! top-level [`crate::air::MatmulAir`] can mount its constraints on
//! BLAKE3-active rows. This is the analog of Pearl's `blake_program.rs`
//! + `blake3_chip` (see `pearl/zk-pow/src/circuit/blake_program.rs`).
//!
//! Why a sub-AIR vs hashing outside the circuit?
//!
//! Our protocol uses BLAKE3 keyed hashes at every public-input
//! boundary:
//!   * `kappa = BLAKE3(block_commitment || params_tag)`
//!   * `s_B   = BLAKE3(kappa  || h_b)`
//!   * `s_A   = BLAKE3(s_B    || h_a)`
//!   * `h_a   = BLAKE3-keyed(padded_A_bytes, kappa)` (chunk-Merkle root)
//!   * `h_b   = BLAKE3-keyed(padded_B_bytes, kappa)`
//!   * `pow_key = derive_key("pow-key", s_A || nonce)`
//!   * `found_leaf = BLAKE3-keyed(M_bytes, pow_key)` (jackpot hash)
//!
//! The verifier sees only the public inputs; the SNARK has to convince
//! it that the same BLAKE3 sequence executed inside the circuit on the
//! committed witness produces those public bytes. The BLAKE3 sub-AIR
//! does exactly that: each compression-function call becomes a fixed
//! number of trace rows, the round constants live as preprocessed
//! columns, and the input/output CVs are routed in/out as ordinary
//! AIR columns the top-level can constrain to match neighbors.
//!
//! Currently a stub: this module exposes the wrapper types and
//! conversions; the actual constraint composition lives in the
//! top-level AIR once the trace-column layout is finalized
//! (see `DESIGN.md`).

use p3_air::{Air, AirBuilder, BaseAir};
use p3_field::Field;

/// Trace width of the BLAKE3 sub-AIR contributing columns to the
/// shared row. Real value pulled from `p3_blake3_air` once we choose
/// the round-batching factor (one row per BLAKE3 round vs one row per
/// G function vs one row per compression).
pub const BLAKE3_SUB_TRACE_WIDTH: usize = 0;

/// Thin wrapper around the upstream `p3-blake3-air` AIR. Holds the
/// configuration needed to lay out the sub-AIR columns inside the
/// top-level `MatmulAir` trace (e.g. starting column index, number of
/// keyed-hash calls per attempt).
#[derive(Debug, Clone, Copy)]
pub struct Blake3SubAir {
    /// Number of keyed BLAKE3 calls the prover must execute inside the
    /// circuit per attempt. Pinned by the protocol shape (kappa, s_B,
    /// s_A, h_a chunk count, h_b chunk count, pow_key, jackpot hash).
    pub num_keyed_calls: usize,
}

impl Blake3SubAir {
    /// Default call count for the current protocol: 7 keyed BLAKE3s
    /// (kappa, s_B, s_A, h_a, h_b, pow_key, jackpot). `h_a` / `h_b`
    /// each unfold into multiple compression-function calls per chunk;
    /// `p3-blake3-air` handles that decomposition internally.
    pub const DEFAULT_KEYED_CALLS: usize = 7;
}

impl<F: Field> BaseAir<F> for Blake3SubAir {
    fn width(&self) -> usize {
        BLAKE3_SUB_TRACE_WIDTH
    }
}

impl<AB: AirBuilder> Air<AB> for Blake3SubAir
where
    <AB as AirBuilder>::F: Field,
{
    fn eval(&self, builder: &mut AB) {
        // Delegated to the upstream `p3_blake3_air` AIR once the
        // trace-column mapping (which slice of the row belongs to
        // BLAKE3) is finalized.
        let _ = builder;
        todo!(
            "thread `p3_blake3_air::Blake3Air` constraints through the top-level \
             builder over the sub-AIR's column slice"
        )
    }
}
