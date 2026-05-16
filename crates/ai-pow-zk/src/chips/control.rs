//! `CONTROL_PREP` unpacking chip.
//!
//! Port of `pearl/zk-pow/src/circuit/chip/control_and_matid_packed.rs`.
//!
//! `CONTROL_PREP` is a single preprocessed Goldilocks element that
//! bit-packs every per-row control flag (21 selectors) plus the
//! `MAT_ID` (the matrix-tile index used by the RAM lookup, packed as
//! 2 × `BITS_PER_LIMB = 13`-bit limbs). The chip:
//!
//!   1. Reads the 21 selector columns + 2 MAT_ID limbs from the
//!      trace, asserts each selector is boolean.
//!   2. Recomputes `MAT_ID = limb0 + limb1 << BITS_PER_LIMB`.
//!   3. Repacks the bits + MAT_ID via `polyval` (base 2) and asserts
//!      equality with the original `CONTROL_PREP` element.
//!   4. Asserts the trace's `MAT_ID` column matches the recomputed
//!      value.
//!
//! ## Property enforced
//!
//! ```text
//!   CONTROL_PREP = polyval([is_reset_cumsum, is_update_cumsum, …,
//!                           is_dump_cumsum_buffer, mat_id], base=2)
//!
//!   every selector ∈ {0, 1}
//!   MAT_ID = MAT_ID_LIMBS[0] + MAT_ID_LIMBS[1] << BITS_PER_LIMB
//! ```
//!
//! The recomposition constraint is degree 1 in the selectors (since
//! each is treated as a coefficient of a power-of-2). The boolean
//! constraints are degree 2 (`b · (1 − b) = 0`).
//!
//! ## Why this matters
//!
//! Subsequent phases use the 21 boolean selectors to gate per-chip
//! constraints (e.g. `is_matmul`, `is_hash_a`). The control chip is
//! what ensures every selector is *really* a 0-or-1 value derived
//! from the preprocessed `CONTROL_PREP` — so a malicious prover
//! can't synthesize free-form selector patterns that fire chips on
//! rows where they shouldn't.
//!
//! The 21 selectors covered:
//!
//! ```text
//!  matmul control:    IS_RESET_CUMSUM, IS_UPDATE_CUMSUM
//!  blake3 control:    IS_USE_JOB_KEY, IS_USE_COMMITMENT_HASH,
//!                     IS_HASH_A, IS_HASH_B, IS_HASH_JACKPOT,
//!                     IS_CV_IN, IS_NEW_BLAKE, IS_LAST_ROUND,
//!                     IS_MSG_MAT, IS_MSG_JACKPOT, IS_MSG_AUX_DATA,
//!                     IS_MSG_CV
//!  jackpot control:   IS_LOAD, IS_XOR, IS_SHIFT3,
//!                     IS_STORE0, IS_STORE1, IS_STORE2,
//!                     IS_DUMP_CUMSUM_BUFFER
//! ```

use p3_air::{AirBuilder, WindowAccess};
use p3_field::PrimeCharacteristicRing;

use crate::composite_layout::{
    BITS_PER_LIMB, CONTROL_PREP, FOLD_IS_FOLD, FOLD_SLOT_SEL_LEN, FOLD_SLOT_SEL_START, IS_CV_IN,
    IS_DUMP_CUMSUM_BUFFER, IS_HASH_A, IS_HASH_B, IS_HASH_JACKPOT, IS_LAST_ROUND, IS_LOAD,
    IS_MSG_AUX_DATA, IS_MSG_CV, IS_MSG_JACKPOT, IS_MSG_MAT, IS_NEW_BLAKE, IS_RESET_CUMSUM,
    IS_SHIFT3, IS_STORE0, IS_STORE1, IS_STORE2, IS_UPDATE_CUMSUM, IS_USE_COMMITMENT_HASH,
    IS_USE_JOB_KEY, IS_XOR, MAT_ID, MAT_ID_LIMBS_START,
};

/// 21 selector bits in canonical Pearl pack order. See
/// [`pearl/zk-pow/src/circuit/chip/control_and_matid_packed.rs:87-110`]
/// for Pearl's authoritative ordering. We mirror it exactly.
const SELECTOR_COLS: [usize; 21] = [
    // matmul (2)
    IS_RESET_CUMSUM, IS_UPDATE_CUMSUM, // blake3 (12)
    IS_USE_JOB_KEY, IS_USE_COMMITMENT_HASH, IS_HASH_A, IS_HASH_B, IS_HASH_JACKPOT, IS_CV_IN,
    IS_NEW_BLAKE, IS_LAST_ROUND, IS_MSG_MAT, IS_MSG_JACKPOT, IS_MSG_AUX_DATA, IS_MSG_CV,
    // jackpot (7)
    IS_LOAD, IS_XOR, IS_SHIFT3, IS_STORE0, IS_STORE1, IS_STORE2, IS_DUMP_CUMSUM_BUFFER,
];

/// Number of selector bits packed into CONTROL_PREP.
pub const NUM_SELECTORS: usize = SELECTOR_COLS.len();

/// Bit-width `MAT_ID` occupies inside the `CONTROL_PREP` polyval
/// (2 × `BITS_PER_LIMB` = 26 bits, just past the 21 selectors).
pub const MAT_ID_BITS: usize = 2 * BITS_PER_LIMB;

/// HIGH-2.2 §6 — bit offset of `FOLD_IS_FOLD` inside `CONTROL_PREP`
/// (immediately after the 21 selectors + 26-bit `MAT_ID`).
pub const FOLD_IS_FOLD_BIT: usize = NUM_SELECTORS + MAT_ID_BITS; // 47
/// HIGH-2.2 §6 — bit offset of the 4-bit fold-slot index
/// (`stripe % 16`) inside `CONTROL_PREP`.
pub const FOLD_SLOT_BIT: usize = FOLD_IS_FOLD_BIT + 1; // 48
/// Number of bits the fold-slot index occupies (`0..=15`).
pub const FOLD_SLOT_BITS: usize = 4;

#[derive(Debug, Default, Clone, Copy)]
pub struct ControlChip;

impl ControlChip {
    pub const fn new() -> Self {
        Self
    }

    pub fn eval<AB: AirBuilder>(&self, builder: &mut AB) {
        let main = builder.main();
        let cur = main.current_slice();

        let control_prep: AB::Var = cur[CONTROL_PREP];

        // Each selector must be boolean.
        for &col in SELECTOR_COLS.iter() {
            builder.assert_bool(cur[col]);
        }

        // MAT_ID = limb0 + limb1 << BITS_PER_LIMB.
        let limb0: AB::Var = cur[MAT_ID_LIMBS_START];
        let limb1: AB::Var = cur[MAT_ID_LIMBS_START + 1];
        let two_pow_limb = <AB::F as PrimeCharacteristicRing>::from_u32(1u32 << BITS_PER_LIMB);
        let mat_id_expr: AB::Expr =
            AB::Expr::from(limb0) + AB::Expr::from(limb1) * two_pow_limb.clone();
        let mat_id_col: AB::Var = cur[MAT_ID];
        builder.assert_eq(mat_id_col, mat_id_expr.clone());

        // CONTROL_PREP = polyval([selector_0, …, selector_20, mat_id], base=2).
        // Pearl packs all 21 selectors first then mat_id at exponent 21,
        // but mat_id occupies 2 × 13 = 26 bits, not 1 — so the
        // "polyval" coefficient for mat_id is the next power of 2
        // after 2^21.
        let mut acc: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
        let mut pow: AB::F = <AB::F as PrimeCharacteristicRing>::ONE;
        let two: AB::F = <AB::F as PrimeCharacteristicRing>::from_u32(2);
        for &col in SELECTOR_COLS.iter() {
            acc = acc + cur[col] * pow.clone();
            pow = pow * two.clone();
        }
        // After 21 selectors, mat_id contributes at coefficient 2^21.
        // Pearl uses `eval.polyval(&[selectors, mat_id], 2)` so mat_id
        // sits at the position after the last selector.
        acc = acc + mat_id_expr * pow.clone();

        // HIGH-2.2 §6 — pin the FoldChip *schedule* into CONTROL_PREP.
        //
        // `CONTROL_PREP` is a CRIT-1 PROGRAM_COL (verifier-fixed via
        // the preprocessed commitment). Packing `FOLD_IS_FOLD` and the
        // fold-slot index here makes the *fold schedule* (which rows
        // fold, into which of the 16 slots) verifier-fixed too — a
        // malicious prover can no longer fabricate a fold schedule to
        // forge `M`. Placed *after* the 21 selectors + 26-bit MAT_ID:
        // `FOLD_IS_FOLD` at bit 2^47, the 4-bit slot at bit 2^48
        // (max packed value < 2^52 ≪ Goldilocks p). On every non-fold
        // row `FOLD_IS_FOLD = 0` and all `FOLD_SLOT_SEL = 0`, so the
        // added term is exactly 0 and CONTROL_PREP is *byte-identical*
        // to the pre-§6 packing — no existing trace changes.
        //
        // SLOT_SEL booleanity / one-hot (`Σ == FOLD_IS_FOLD`) is
        // enforced by `FoldChip`, which `CompositeFullAir::eval`
        // always wires alongside this chip; here we only need
        // `FOLD_IS_FOLD` boolean for the pack term to be well-formed.
        let two_pow_mat_id =
            <AB::F as PrimeCharacteristicRing>::from_u32(1u32 << MAT_ID_BITS);
        pow = pow * two_pow_mat_id; // pow = 2^(21+26) = 2^47

        let is_fold: AB::Var = cur[FOLD_IS_FOLD];
        builder.assert_bool(is_fold);
        acc = acc + is_fold * pow.clone(); // 2^47
        pow = pow * two.clone(); // 2^48

        let mut slot_idx: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
        for s in 0..FOLD_SLOT_SEL_LEN {
            let w = <AB::F as PrimeCharacteristicRing>::from_u32(s as u32);
            slot_idx = slot_idx + cur[FOLD_SLOT_SEL_START + s] * w;
        }
        acc = acc + slot_idx * pow; // 2^48 (4-bit slot ⇒ bits 48..52)

        builder.assert_eq(control_prep, acc);
    }

    /// Pack 21 selector bits + a MAT_ID into a single Goldilocks
    /// element. Used to construct the preprocessed `CONTROL_PREP`
    /// value for testing / trace generation. Equivalent to
    /// [`Self::pack_control_prep_full`] with no fold activity
    /// (`is_fold = false`, `fold_slot = 0`) — byte-identical to the
    /// pre-HIGH-2.2-§6 packing for every non-fold row.
    pub fn pack_control_prep(selectors: &[bool; NUM_SELECTORS], mat_id: u32) -> u64 {
        Self::pack_control_prep_full(selectors, mat_id, false, 0)
    }

    /// HIGH-2.2 §6 — pack 21 selectors + MAT_ID **plus** the
    /// FoldChip schedule (`is_fold` at bit 2^47, the 4-bit
    /// `fold_slot` at bit 2^48) into one Goldilocks element. The
    /// `CompositeFullAirPinned` CRIT-1 pin then makes the fold
    /// schedule verifier-fixed. `fold_slot` must be `< 16`.
    pub fn pack_control_prep_full(
        selectors: &[bool; NUM_SELECTORS],
        mat_id: u32,
        is_fold: bool,
        fold_slot: u8,
    ) -> u64 {
        debug_assert!((fold_slot as usize) < FOLD_SLOT_SEL_LEN, "fold_slot out of range");
        let mut packed: u64 = 0;
        for (i, &b) in selectors.iter().enumerate() {
            if b {
                packed |= 1u64 << i;
            }
        }
        packed |= (mat_id as u64) << NUM_SELECTORS;
        packed |= (is_fold as u64) << FOLD_IS_FOLD_BIT;
        packed |= ((fold_slot as u64) & ((1u64 << FOLD_SLOT_BITS) - 1)) << FOLD_SLOT_BIT;
        packed
    }

    /// Fill the control chip's trace cells from canonical selector
    /// bits + MAT_ID.
    pub fn fill_row(&self, selectors: &[bool; NUM_SELECTORS], mat_id: u32, row: &mut [crate::Val]) {
        use p3_field::integers::QuotientMap;

        // Preprocessed CONTROL_PREP (caller commits this; for testing
        // we write it into the trace directly).
        let packed = Self::pack_control_prep(selectors, mat_id);
        row[CONTROL_PREP] = <crate::Val as QuotientMap<u64>>::from_int(packed);

        // Selector columns.
        for (i, &col) in SELECTOR_COLS.iter().enumerate() {
            row[col] = <crate::Val as QuotientMap<u64>>::from_int(selectors[i] as u64);
        }

        // MAT_ID + limbs.
        let limb_mask = (1u32 << BITS_PER_LIMB) - 1;
        let limb0 = (mat_id & limb_mask) as u64;
        let limb1 = ((mat_id >> BITS_PER_LIMB) & limb_mask) as u64;
        row[MAT_ID_LIMBS_START] = <crate::Val as QuotientMap<u64>>::from_int(limb0);
        row[MAT_ID_LIMBS_START + 1] = <crate::Val as QuotientMap<u64>>::from_int(limb1);
        row[MAT_ID] = <crate::Val as QuotientMap<u64>>::from_int(mat_id as u64);
    }
}

#[cfg(test)]
mod tests {
    use p3_air::{Air, AirBuilder, BaseAir};
    use p3_field::integers::QuotientMap;
    use p3_matrix::dense::RowMajorMatrix;
    use p3_uni_stark::{prove, verify};

    use super::*;
    use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
    use crate::composite_layout::TOTAL_TRACE_WIDTH;
    use crate::params::ZkParams;
    use crate::Val;

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

    #[derive(Debug, Default)]
    struct ControlOnlyAir;

    impl<F> BaseAir<F> for ControlOnlyAir {
        fn width(&self) -> usize {
            TOTAL_TRACE_WIDTH
        }
    }

    impl<AB: AirBuilder> Air<AB> for ControlOnlyAir {
        fn eval(&self, builder: &mut AB) {
            ControlChip::new().eval(builder);
        }
    }

    fn build_uniform_trace(
        rows: usize,
        selectors: &[bool; NUM_SELECTORS],
        mat_id: u32,
    ) -> RowMajorMatrix<Val> {
        let chip = ControlChip::new();
        let mut flat = vec![Val::default(); rows * TOTAL_TRACE_WIDTH];
        for r in 0..rows {
            let row = &mut flat[r * TOTAL_TRACE_WIDTH..(r + 1) * TOTAL_TRACE_WIDTH];
            chip.fill_row(selectors, mat_id, row);
        }
        RowMajorMatrix::new(flat, TOTAL_TRACE_WIDTH)
    }

    #[test]
    fn num_selectors_is_21() {
        assert_eq!(NUM_SELECTORS, 21);
    }

    #[test]
    fn pack_round_trips_zeros() {
        let s = [false; NUM_SELECTORS];
        assert_eq!(ControlChip::pack_control_prep(&s, 0), 0);
    }

    #[test]
    fn pack_sets_correct_bits() {
        // Set every alternate selector + mat_id = 42.
        let mut s = [false; NUM_SELECTORS];
        for i in (0..NUM_SELECTORS).step_by(2) {
            s[i] = true;
        }
        let packed = ControlChip::pack_control_prep(&s, 42);
        // First 21 bits: 010101...010101 (odd-position 1s, with 21
        // bits total → bits 0, 2, 4, …, 20 set).
        let mut expected_low: u64 = 0;
        for i in (0..NUM_SELECTORS).step_by(2) {
            expected_low |= 1 << i;
        }
        // mat_id 42 << 21:
        let expected = expected_low | (42u64 << NUM_SELECTORS);
        assert_eq!(packed, expected);
    }

    #[test]
    fn prove_and_verify_all_zero_selectors() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let trace = build_uniform_trace(16, &s, 0);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("all-zero selectors must verify");
    }

    #[test]
    fn prove_and_verify_all_one_selectors() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [true; NUM_SELECTORS];
        let trace = build_uniform_trace(16, &s, 0);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("all-one selectors must verify");
    }

    #[test]
    fn prove_and_verify_with_mat_id() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let mut s = [false; NUM_SELECTORS];
        s[2] = true;
        s[7] = true;
        s[14] = true;
        let trace = build_uniform_trace(16, &s, 12345);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("mixed selectors + mat_id must verify");
    }

    /// Property: each selector must be boolean.
    #[test]
    fn rejects_non_boolean_selector() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 0);
        // Force IS_HASH_A on row 0 to 2 (non-boolean).
        trace.values[IS_HASH_A] = <Val as QuotientMap<i32>>::from_int(2);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    /// Property: CONTROL_PREP must equal polyval of selectors + mat_id.
    #[test]
    fn rejects_wrong_control_prep_pack() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 0);
        // Tamper CONTROL_PREP at row 5 → claims a different selector
        // pattern from what the columns hold.
        trace.values[5 * TOTAL_TRACE_WIDTH + CONTROL_PREP] = <Val as QuotientMap<u64>>::from_int(1);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    /// Property: MAT_ID column must equal limb0 + limb1 << 13.
    #[test]
    fn rejects_mat_id_inconsistent_with_limbs() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 100);
        // Force MAT_ID at row 3 to a wrong value (still matches the
        // pack equation, so the recompose constraint fires).
        trace.values[3 * TOTAL_TRACE_WIDTH + MAT_ID] = <Val as QuotientMap<i32>>::from_int(101);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    /// Property: tampering selector column without updating
    /// CONTROL_PREP → mismatch.
    #[test]
    fn rejects_selector_without_control_prep_update() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 0);
        // Force IS_LOAD on row 7 to 1; CONTROL_PREP still has it
        // as 0 → repack mismatches.
        trace.values[7 * TOTAL_TRACE_WIDTH + IS_LOAD] = <Val as QuotientMap<i32>>::from_int(1);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    /// Reference: every position in `SELECTOR_COLS` is unique.
    #[test]
    fn selector_columns_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &c in SELECTOR_COLS.iter() {
            assert!(seen.insert(c), "duplicate column index {c}");
        }
    }

    // ============================================================
    //  HIGH-2.2 §6 — fold schedule pinned into CONTROL_PREP
    // ============================================================

    use crate::composite_layout::{FOLD_IS_FOLD, FOLD_SLOT_SEL_START};

    /// Make row `r` of an otherwise non-fold trace a fold row into
    /// `slot`, with CONTROL_PREP packing `(is_fold=1, slot)` —
    /// exactly what `place_fold_chain` writes.
    fn make_fold_row(trace: &mut RowMajorMatrix<Val>, r: usize, slot: usize, consistent: bool) {
        let base = r * TOTAL_TRACE_WIDTH;
        trace.values[base + FOLD_IS_FOLD] = <Val as QuotientMap<u64>>::from_int(1);
        trace.values[base + FOLD_SLOT_SEL_START + slot] =
            <Val as QuotientMap<u64>>::from_int(1);
        let packed_slot = if consistent { slot } else { (slot + 1) % 16 };
        trace.values[base + CONTROL_PREP] = <Val as QuotientMap<u64>>::from_int(
            ControlChip::pack_control_prep_full(
                &[false; NUM_SELECTORS],
                0,
                true,
                packed_slot as u8,
            ),
        );
    }

    /// `FOLD_IS_FOLD_BIT` / `FOLD_SLOT_BIT` sit immediately past the
    /// 21 selectors + 26-bit MAT_ID and never collide with them.
    #[test]
    fn fold_pack_bit_layout_is_disjoint() {
        assert_eq!(MAT_ID_BITS, 26);
        assert_eq!(FOLD_IS_FOLD_BIT, 47);
        assert_eq!(FOLD_SLOT_BIT, 48);
        // Max packed value (all selectors + max mat_id + fold) stays
        // well under Goldilocks p (~2^63.99): top bit < 52.
        assert!(FOLD_SLOT_BIT + FOLD_SLOT_BITS <= 52);
    }

    /// No fold activity ⇒ CONTROL_PREP byte-identical to the
    /// pre-§6 packing (zero blast radius for every non-fold row).
    #[test]
    fn non_fold_pack_is_unchanged() {
        let mut s = [false; NUM_SELECTORS];
        s[3] = true;
        s[15] = true;
        let old = {
            // The historical pack: selectors then mat_id << 21, no
            // fold bits.
            let mut p = 0u64;
            for (i, &b) in s.iter().enumerate() {
                if b {
                    p |= 1u64 << i;
                }
            }
            p |= (0x1234u64) << NUM_SELECTORS;
            p
        };
        assert_eq!(ControlChip::pack_control_prep(&s, 0x1234), old);
        assert_eq!(
            ControlChip::pack_control_prep_full(&s, 0x1234, false, 0),
            old
        );
    }

    /// A fold row whose CONTROL_PREP is consistent with
    /// FOLD_IS_FOLD / FOLD_SLOT_SEL verifies (positive).
    #[test]
    fn fold_schedule_consistent_control_prep_verifies() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 0);
        make_fold_row(&mut trace, 4, 5, true);
        make_fold_row(&mut trace, 9, 0, true);
        make_fold_row(&mut trace, 12, 15, true);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("consistent fold-row CONTROL_PREP must verify");
    }

    /// Adversary fabricates a fold schedule (FOLD_IS_FOLD /
    /// FOLD_SLOT_SEL set) but CONTROL_PREP still holds the non-fold
    /// (zero-fold) packing ⇒ `CONTROL_PREP == polyval(.., is_fold,
    /// slot)` is violated.
    #[test]
    fn fold_is_fold_without_control_prep_rejected() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 0);
        let base = 6 * TOTAL_TRACE_WIDTH;
        // FOLD_IS_FOLD = 1, slot 7 selected, but CONTROL_PREP left
        // at the build_uniform_trace value (no fold bits).
        trace.values[base + FOLD_IS_FOLD] = <Val as QuotientMap<u64>>::from_int(1);
        trace.values[base + FOLD_SLOT_SEL_START + 7] =
            <Val as QuotientMap<u64>>::from_int(1);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err(),
            "fabricated fold schedule with stale CONTROL_PREP must reject"
        );
    }

    /// CONTROL_PREP packs a *different* slot than FOLD_SLOT_SEL ⇒
    /// the slot-index term of the polyval mismatches ⇒ reject. This
    /// is the core §6 soundness property: the fold slot is
    /// verifier-fixed.
    #[test]
    fn fold_slot_mismatch_rejected() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 0);
        make_fold_row(&mut trace, 8, 5, false); // SLOT_SEL=5, CONTROL_PREP packs 6
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err(),
            "fold slot ≠ CONTROL_PREP-packed slot must reject"
        );
    }

    /// CONTROL_PREP claims is_fold but FOLD_IS_FOLD column is 0 ⇒
    /// reject (a prover cannot silently drop a scheduled fold).
    #[test]
    fn control_prep_claims_fold_but_column_zero_rejected() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = ControlOnlyAir;
        let s = [false; NUM_SELECTORS];
        let mut trace = build_uniform_trace(16, &s, 0);
        let base = 10 * TOTAL_TRACE_WIDTH;
        trace.values[base + CONTROL_PREP] = <Val as QuotientMap<u64>>::from_int(
            ControlChip::pack_control_prep_full(&[false; NUM_SELECTORS], 0, true, 3),
        );
        // FOLD_IS_FOLD / FOLD_SLOT_SEL left at 0.
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err(),
            "CONTROL_PREP claiming a fold the columns don't perform must reject"
        );
    }
}
