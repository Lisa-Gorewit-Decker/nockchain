//! In-circuit Pearl-compat found-leaf binding AIR (M10.1b).
//!
//! Wraps [`crate::blake3_chip::Blake3KeyedAir`] (the vendored
//! BLAKE3-compression AIR) and bolts on public-input constraints that
//! pin the trace's row-0 message, key, output, and the protocol-level
//! pinned constants `counter = 0`, `block_len = 64`, `flags = 0x1B`.
//! Together with the underlying BLAKE3 constraints this proves
//!
//! ```text
//!   BLAKE3-keyed([m_final, 0, …, 0], pow_key) == found_leaf
//! ```
//!
//! IN-circuit, where `m_final`, `pow_key`, and `found_leaf` are
//! supplied via the public-values channel by the verifier. The hash
//! is byte-equivalent to `ai_pow::matmul::TileState::keyed_hash`
//! (Pearl §4.5) — see `tests/blake3_chip_kat.rs` for the KAT
//! anchors — so Pearl ↔ Nockchain merge-mining is preserved.
//!
//! ## Public-values layout
//!
//! 17 Goldilocks elements:
//!
//! ```text
//!   pis[0]            : m_final         (1 u32)
//!   pis[1..9]         : pow_key[0..8]   (8 u32s, LE)
//!   pis[9..17]        : found_leaf[0..8] (8 u32s, LE)
//! ```
//!
//! ## How M10.1b stitches into the protocol
//!
//! `crate::prove` produces *two* proofs in one [`crate::ZkProof`]
//! envelope:
//!
//!   * the M9.1 composite tile proof (matmul + state + linkage), with
//!     `m_final` exposed as a public value (M10.1a constraint), and
//!   * a [`Blake3FoundLeafAir`] proof binding the same `m_final` plus
//!     `pow_key` to `found_leaf` cryptographically.
//!
//! `crate::verify` checks both proofs AND asserts the shared public
//! values agree (composite's `m_final` ↔ found-leaf's `m_final`,
//! verifier-derived `pow_key` ↔ found-leaf's `pow_key`,
//! `public_inputs.found_leaf` ↔ found-leaf's `found_leaf`). This
//! upgrades M10.1a's out-of-circuit BLAKE3 check to an in-circuit
//! one without diverging the hash function — the merge-mining
//! requirement is preserved.
//!
//! ## What this AIR does *not* do
//!
//! - Bind `a_rows` / `b_cols` to `h_a` / `h_b`. That requires
//!   per-row hashing + chunk-Merkle path verification — M10.1c.

use core::borrow::Borrow;

use p3_air::utils::pack_bits_le;
use p3_air::{Air, AirBuilder, BaseAir, WindowAccess};
use p3_field::PrimeCharacteristicRing;

use crate::blake3_chip::{Blake3Cols, Blake3KeyedAir, NUM_BLAKE3_COLS};

/// Public-value index for `m_final` (single u32).
pub const PI_M_FINAL: usize = 0;
/// First public-value index of the `pow_key` block (8 u32s, LE).
pub const PI_POW_KEY_START: usize = 1;
/// First public-value index of the `found_leaf` block (8 u32s, LE).
pub const PI_FOUND_LEAF_START: usize = PI_POW_KEY_START + 8;
/// Total public-values count this AIR consumes.
pub const NUM_FOUND_LEAF_PIS: usize = PI_FOUND_LEAF_START + 8;

/// Pearl §4.5 / `ai_pow::matmul::TileState::keyed_hash` constants for
/// the single-block keyed root hash:
///   counter = 0, block_len = 64,
///   flags = CHUNK_START | CHUNK_END | ROOT | KEYED_HASH = 0x1B.
const FOUND_LEAF_BLOCK_LEN: u32 = 64;
const FOUND_LEAF_FLAGS: u32 = 0x1B;

/// AIR for the in-circuit found-leaf binding.
#[derive(Debug, Default)]
pub struct Blake3FoundLeafAir {
    inner: Blake3KeyedAir,
}

impl Blake3FoundLeafAir {
    pub const fn new() -> Self {
        Self {
            inner: Blake3KeyedAir {},
        }
    }
}

impl<F> BaseAir<F> for Blake3FoundLeafAir {
    fn width(&self) -> usize {
        NUM_BLAKE3_COLS
    }

    fn num_public_values(&self) -> usize {
        NUM_FOUND_LEAF_PIS
    }
}

impl<AB: AirBuilder> Air<AB> for Blake3FoundLeafAir {
    fn eval(&self, builder: &mut AB) {
        // 1. Inherit the entire BLAKE3 compression-function constraint
        //    set from the vendored chip. After this returns,
        //    `local.outputs` is constrained to be the BLAKE3
        //    compression of `(local.inputs, local.chaining_values,
        //    counter_low, counter_hi, block_len, flags)`.
        <Blake3KeyedAir as Air<AB>>::eval(&self.inner, builder);

        // 2. Pin the trace's row-0 inputs and outputs against public
        //    values.
        let main = builder.main();
        let local: &Blake3Cols<AB::Var> = main.current_slice().borrow();
        // Snapshot all the PublicVars we'll need before grabbing
        // `builder.when_first_row()` — that takes a mutable borrow on
        // `builder` and would conflict with the live `&[PublicVar]`
        // reference `public_values()` hands back.
        let pis: [AB::PublicVar; NUM_FOUND_LEAF_PIS] = {
            let raw = builder.public_values();
            core::array::from_fn(|i| raw[i])
        };
        let m_final_pi = pis[PI_M_FINAL];

        let mut first = builder.when_first_row();

        // 2a. message[0] = m_final.
        first.assert_eq(
            pack_bits_le::<AB::Expr, _, _>(local.inputs[0].iter().copied()),
            m_final_pi,
        );
        // message[1..16] = 0 — single-slot M9.1 regime hashes
        //                          `[m_final, 0, …, 0]` (16 × i32 LE).
        for i in 1..16 {
            first.assert_eq(
                pack_bits_le::<AB::Expr, _, _>(local.inputs[i].iter().copied()),
                <AB::Expr as PrimeCharacteristicRing>::ZERO,
            );
        }

        // 2b. chaining_values = pow_key split into 8 LE u32s.
        // Upstream packs 4 + 4 across `chaining_values[0]` and `[1]`.
        for i in 0..8 {
            let row_idx = i / 4;
            let col_idx = i % 4;
            first.assert_eq(
                pack_bits_le::<AB::Expr, _, _>(
                    local.chaining_values[row_idx][col_idx].iter().copied(),
                ),
                pis[PI_POW_KEY_START + i],
            );
        }

        // 2c. counter = 0, block_len = 64, flags = 0x1B.
        first.assert_eq(
            pack_bits_le::<AB::Expr, _, _>(local.counter_low.iter().copied()),
            <AB::Expr as PrimeCharacteristicRing>::ZERO,
        );
        first.assert_eq(
            pack_bits_le::<AB::Expr, _, _>(local.counter_hi.iter().copied()),
            <AB::Expr as PrimeCharacteristicRing>::ZERO,
        );
        first.assert_eq(
            pack_bits_le::<AB::Expr, _, _>(local.block_len.iter().copied()),
            <AB::Expr as PrimeCharacteristicRing>::from_u32(FOUND_LEAF_BLOCK_LEN),
        );
        first.assert_eq(
            pack_bits_le::<AB::Expr, _, _>(local.flags.iter().copied()),
            <AB::Expr as PrimeCharacteristicRing>::from_u32(FOUND_LEAF_FLAGS),
        );

        // 2d. outputs[0..2][0..4] = found_leaf split into 8 LE u32s.
        // The 32-byte BLAKE3 hash is exactly the first 8 u32s of the
        // compression output.
        for i in 0..8 {
            let row_idx = i / 4;
            let col_idx = i % 4;
            first.assert_eq(
                pack_bits_le::<AB::Expr, _, _>(local.outputs[row_idx][col_idx].iter().copied()),
                pis[PI_FOUND_LEAF_START + i],
            );
        }
    }
}

/// Encode `(m_final, pow_key, found_leaf)` into the 17-element
/// public-values vector this AIR expects. Mirrors the layout the
/// constraint code above checks.
pub fn build_public_values<F: PrimeCharacteristicRing>(
    m_final: u32,
    pow_key: &[u8; 32],
    found_leaf: &[u8; 32],
) -> [F; NUM_FOUND_LEAF_PIS] {
    fn u32_at<F: PrimeCharacteristicRing>(bytes: &[u8], off: usize) -> F {
        let mut b = [0u8; 4];
        b.copy_from_slice(&bytes[off..off + 4]);
        F::from_u32(u32::from_le_bytes(b))
    }
    let mut out = [F::ZERO; NUM_FOUND_LEAF_PIS];
    out[PI_M_FINAL] = F::from_u32(m_final);
    for i in 0..8 {
        out[PI_POW_KEY_START + i] = u32_at(pow_key, i * 4);
        out[PI_FOUND_LEAF_START + i] = u32_at(found_leaf, i * 4);
    }
    out
}

#[cfg(test)]
mod tests {
    use p3_uni_stark::{prove, verify};

    use super::*;
    use crate::blake3_chip::{generate_trace_for_calls, Blake3HashCall};
    use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
    use crate::params::ZkParams;
    use crate::{binding, Val};

    const CHUNK_START: u32 = 1 << 0;
    const CHUNK_END: u32 = 1 << 1;
    const ROOT: u32 = 1 << 3;
    const KEYED_HASH: u32 = 1 << 4;

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

    /// Build the BLAKE3 call honest miners produce for `(m_final, pow_key)`
    /// in Pearl's single-block keyed-root format.
    fn make_call(m_final: u32, pow_key: &[u8; 32]) -> Blake3HashCall {
        let mut message = [0u32; 16];
        message[0] = m_final;
        let mut key = [0u32; 8];
        for i in 0..8 {
            let mut b = [0u8; 4];
            b.copy_from_slice(&pow_key[i * 4..(i + 1) * 4]);
            key[i] = u32::from_le_bytes(b);
        }
        Blake3HashCall {
            message,
            key,
            counter: 0,
            block_len: 64,
            flags: CHUNK_START | CHUNK_END | ROOT | KEYED_HASH,
        }
    }

    fn pis_for(m_final: u32, pow_key: &[u8; 32]) -> [Val; NUM_FOUND_LEAF_PIS] {
        let leaf = binding::compute_found_leaf(m_final, pow_key);
        build_public_values::<Val>(m_final, pow_key, &leaf)
    }

    #[test]
    fn pi_layout_constants_are_consistent() {
        assert_eq!(PI_M_FINAL, 0);
        assert_eq!(PI_POW_KEY_START, 1);
        assert_eq!(PI_FOUND_LEAF_START, 9);
        assert_eq!(NUM_FOUND_LEAF_PIS, 17);
    }

    #[test]
    fn prove_and_verify_honest_call() {
        let m_final: u32 = 0xDEAD_BEEF;
        let pow_key = [0x55u8; 32];

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = Blake3FoundLeafAir::new();
        let calls = vec![make_call(m_final, &pow_key)];
        let trace =
            generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);
        let pis = pis_for(m_final, &pow_key);

        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis)
            .expect("honest in-circuit found-leaf binding must verify");
    }

    #[test]
    fn verify_rejects_wrong_found_leaf_in_pi() {
        let m_final: u32 = 0x12345678;
        let pow_key = [0x77u8; 32];

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = Blake3FoundLeafAir::new();
        let calls = vec![make_call(m_final, &pow_key)];
        let trace =
            generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);

        let mut pis = pis_for(m_final, &pow_key);
        // Tamper one slot of found_leaf PI block.
        pis[PI_FOUND_LEAF_START + 3] = Val::default();

        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
        let r = verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis);
        assert!(r.is_err(), "tampered found_leaf PI must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_wrong_m_final_in_pi() {
        let m_final: u32 = 0xAABB_CCDD;
        let pow_key = [0x21u8; 32];

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = Blake3FoundLeafAir::new();
        let calls = vec![make_call(m_final, &pow_key)];
        let trace =
            generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);

        let mut pis = pis_for(m_final, &pow_key);
        // Claim a different m_final in the PI vector. The trace's
        // message[0] is set to the real `m_final`, so the binding
        // constraint will fail.
        pis[PI_M_FINAL] = <Val as p3_field::integers::QuotientMap<u32>>::from_int(0);

        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
        let r = verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis);
        assert!(r.is_err(), "wrong m_final PI must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_wrong_pow_key_in_pi() {
        let m_final: u32 = 0xBABE_FACE;
        let pow_key = [0x33u8; 32];

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = Blake3FoundLeafAir::new();
        let calls = vec![make_call(m_final, &pow_key)];
        let trace =
            generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);

        let mut pis = pis_for(m_final, &pow_key);
        pis[PI_POW_KEY_START] =
            <Val as p3_field::integers::QuotientMap<u32>>::from_int(0xFFFF_FFFF);

        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
        let r = verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis);
        assert!(r.is_err(), "wrong pow_key PI must reject; got {r:?}");
    }

    /// If the prover lies and uses different parameters in the trace
    /// from what the PIs declare, the constraint catches it. Here we
    /// build a trace with `flags = 0` (the upstream-broken value) and
    /// expect rejection.
    #[test]
    fn verify_rejects_wrong_flags_in_trace() {
        let m_final: u32 = 0x4242_4242;
        let pow_key = [0x42u8; 32];

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = Blake3FoundLeafAir::new();

        // Trace with flags=0 (BLAKE3-compression-no-flags) instead of 0x1B.
        let mut call = make_call(m_final, &pow_key);
        call.flags = 0;
        let calls = vec![call];
        let trace =
            generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);

        let pis = pis_for(m_final, &pow_key);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
        let r = verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &pis);
        assert!(
            r.is_err(),
            "trace with wrong flags must reject under the binding AIR; got {r:?}"
        );
    }
}
