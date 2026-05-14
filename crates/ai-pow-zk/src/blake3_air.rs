//! BLAKE3 permutation sub-AIR integration.
//!
//! Re-exports and thinly wraps `p3_blake3_air::Blake3Air` so the rest
//! of the crate has a single import point for BLAKE3 constraints, and
//! pins down that the upstream AIR plugs into our [`AiPowStarkConfig`]
//! (Goldilocks + Tip5 + FRI) without modification.
//!
//! ## What Pearl wants from BLAKE3
//!
//! Pearl's keyed-BLAKE3 sequence at the public-input boundary:
//!
//!   * `kappa     = BLAKE3(block_commitment ‖ params_tag)`
//!   * `s_B       = BLAKE3(kappa ‖ h_b)`
//!   * `s_A       = BLAKE3(s_B   ‖ h_a)`
//!   * `h_a       = BLAKE3-keyed(padded_A_bytes, kappa)`
//!   * `h_b       = BLAKE3-keyed(padded_B_bytes, kappa)`
//!   * `pow_key   = derive_key("pow-key", s_A ‖ nonce)`
//!   * `found_leaf = BLAKE3-keyed(M_bytes, pow_key)`
//!
//! Each of those decomposes into one or more BLAKE3 *compression*
//! invocations; `Blake3Air` is the AIR for one compression. M9 wires
//! up the composition (chaining values, counters, keyed-mode flags)
//! at the protocol layer. This module's job is just to certify that
//! the compression-AIR proof system itself works in our configuration.
//!
//! ## Field-size check
//!
//! `p3_blake3_air` documents "field size between `2^20` and `2^32`".
//! That bound is conservative — internally it packs 16-bit limbs, so
//! any prime `p ≥ 2^18` works for the AIR's additions, and Goldilocks
//! (`p ≈ 2^64`) is fine. The integration tests below exercise this.

use p3_blake3_air::Blake3Air;

/// Public alias so other modules can refer to the BLAKE3 sub-AIR
/// without re-importing `p3_blake3_air` directly.
pub type Blake3SubAir = Blake3Air;

/// Default number of keyed BLAKE3 calls per attempt under the current
/// Pearl-derived protocol. Used by M9 to budget the BLAKE3 trace
/// height.
///
///   1. `kappa`
///   2. `s_B`
///   3. `s_A`
///   4. `pow_key`
///   5. `found_leaf`
///   + per-chunk calls inside `h_a` and `h_b` (variable; counted
///     dynamically by the witness builder).
pub const FIXED_KEYED_CALLS: usize = 5;

/// Construct the upstream `Blake3Air`. Wrapper kept as a function (vs
/// `Blake3Air {}` literal) so call sites stay stable if upstream adds
/// configuration fields later.
pub const fn sub_air() -> Blake3Air {
    Blake3Air {}
}

#[cfg(test)]
mod tests {
    use p3_air::BaseAir;
    use p3_blake3_air::NUM_BLAKE3_COLS;
    use p3_field::PrimeField64;
    use p3_uni_stark::{prove, verify};

    use super::*;
    use crate::circuit::{build_stark_config, CircuitConfig, Val};
    use crate::params::ZkParams;

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

    /// Sanity: width of the upstream AIR is the documented constant.
    /// This is a pin against silent upstream version drift — if it
    /// moves, the column layout we'll be wiring up in M9 also moves.
    #[test]
    fn upstream_width_is_num_blake3_cols() {
        let air = sub_air();
        assert_eq!(<Blake3Air as BaseAir<Val>>::width(&air), NUM_BLAKE3_COLS);
    }

    /// Single-hash BLAKE3 trace proves and verifies through our
    /// Goldilocks + Tip5 + FRI configuration. This validates:
    ///   - Blake3Air's PrimeField64 bound is satisfied by Goldilocks
    ///   - the trace's bit/limb constraints don't overflow our prime
    ///   - the AiPowStarkConfig stack handles ~10k-wide traces
    #[test]
    fn prove_and_verify_one_hash_via_ai_pow_config() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = sub_air();
        // generate_trace_rows internally uses SmallRng for inputs; the
        // num_hashes count must be a power of two (Blake3Air invariant).
        let trace = air.generate_trace_rows::<Val>(1, CircuitConfig::TEST.log_blowup as usize);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("Blake3Air trace must verify");
    }

    /// 2 hashes packed side-by-side as 2 trace rows. Exercises the
    /// power-of-two height path with non-trivial FRI folding.
    #[test]
    fn prove_and_verify_two_hashes_via_ai_pow_config() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = sub_air();
        let trace = air.generate_trace_rows::<Val>(2, CircuitConfig::TEST.log_blowup as usize);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("2-hash Blake3Air trace must verify");
    }

    /// Tamper detection: mutate one cell and ensure the verifier
    /// rejects. The mutated cell is somewhere in the input bit
    /// columns, which feed the additions for the first round.
    #[test]
    fn verify_rejects_tampered_input_bit() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = sub_air();
        let mut trace = air.generate_trace_rows::<Val>(1, CircuitConfig::TEST.log_blowup as usize);
        // Flip cell 0 (an input bit) from 0/1 to something else. Make
        // it 2 to also break booleanity.
        use p3_field::integers::QuotientMap;
        trace.values[0] = <Val as QuotientMap<u32>>::from_int(2);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "tampered Blake3 trace must reject; got {r:?}");
    }

    /// Read-back: confirm a trace cell is canonicalizable through
    /// PrimeField64. This is a smoke test for the Goldilocks
    /// canonical conversion of trace values populated by
    /// Blake3Air's bit-decomposition logic.
    #[test]
    fn trace_cells_are_canonical() {
        let air = sub_air();
        let trace = air.generate_trace_rows::<Val>(1, 0);
        // Just pull a handful and confirm they don't panic.
        for &v in trace.values.iter().take(64) {
            let _ = v.as_canonical_u64();
        }
    }

    /// `FIXED_KEYED_CALLS` is documentation — pin it so a change
    /// requires an explicit edit alongside `air.rs` / the M9 witness
    /// builder.
    #[test]
    fn fixed_keyed_calls_is_five() {
        assert_eq!(FIXED_KEYED_CALLS, 5);
    }
}
