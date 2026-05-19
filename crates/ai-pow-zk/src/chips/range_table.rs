//! Generic range-table chip + four concrete instantiations.
//!
//! **Pearl ISC.** This file is derived from Pearl source code
//! (Copyright (c) 2025-2026 Pearl Research Labs; 2015-2016 The Decred
//! developers); see `crates/ai-pow-zk/LICENSE-PEARL` for the full
//! permission notice.
//!
//! Port of `pearl/zk-pow/src/circuit/chip/range_table.rs`. One generic
//! `RangeTableChip<COL, MIN, MAX>` parameterised by the table's
//! column offset and `[MIN..=MAX]` integer range; four type aliases
//! pin Pearl's specific tables:
//!
//!   * `URange8Chip`    → `[0..=255]`     (column [`URANGE8_TABLE`])
//!   * `URange13Chip`   → `[0..=8191]`    (column [`URANGE13_TABLE`])
//!   * `IRange7P1Chip`  → `[-64..=64]`    (column [`IRANGE7P1_TABLE`])
//!   * `IRange8Chip`    → `[-128..=127]`  (column [`IRANGE8_TABLE`])
//!
//! ## Property enforced (per chip instantiation)
//!
//! The `*_TABLE` column at offset `COL` carries the **complete**
//! enumeration of `[MIN..=MAX]`, possibly with the max value repeated
//! at the tail to pad to a power-of-two trace height. Constraints:
//!
//! ```text
//!   value[0]   == MIN
//!   value[N-1] == MAX
//!   value[i+1] - value[i] ∈ {0, 1}    (transition: monotonic, step 0 or 1)
//! ```
//!
//! Together these force the column to enumerate every integer in
//! `[MIN..=MAX]` in order. The boolean-delta constraint is degree 2
//! (`δ * (δ - 1) = 0`), comfortably within `TEST_PEARL`'s
//! `log_blowup = 2` budget.
//!
//! ## Why "any integer in range" is enforced
//!
//! If `value[0] = MIN`, `value[N-1] = MAX`, and every step is 0 or 1,
//! then by the discrete-mean-value argument every integer between
//! `MIN` and `MAX` must appear *at least once* in the column.
//! Subsequent LogUp lookups (Phase 11) can then prove "every reader
//! row's value appears in this table" with full coverage.

use p3_air::{AirBuilder, WindowAccess};
use p3_field::PrimeCharacteristicRing;

use crate::composite_layout::{
    BITS_PER_LIMB, IRANGE7P1_TABLE, IRANGE8_TABLE, URANGE13_TABLE, URANGE8_TABLE,
};

/// Generic range-table chip. `COL` is the trace column holding the
/// table values; `MIN..=MAX` is the range (both inclusive).
#[derive(Debug, Default, Clone, Copy)]
pub struct RangeTableChip<const COL: usize, const MIN: i32, const MAX: i32>;

impl<const COL: usize, const MIN: i32, const MAX: i32> RangeTableChip<COL, MIN, MAX> {
    pub const fn new() -> Self {
        assert!(MIN < MAX, "RangeTableChip: MIN must be < MAX");
        Self
    }

    /// Inclusive table size (`MAX − MIN + 1`).
    pub const fn span() -> usize {
        (MAX - MIN + 1) as usize
    }

    pub fn eval<AB: AirBuilder>(&self, builder: &mut AB) {
        let main = builder.main();
        let cur = main.current_slice();
        let nxt = main.next_slice();

        let cur_val = cur[COL];
        let nxt_val = nxt[COL];

        let min_f = <AB::F as PrimeCharacteristicRing>::from_i32(MIN);
        let max_f = <AB::F as PrimeCharacteristicRing>::from_i32(MAX);

        // First row = MIN.
        builder.when_first_row().assert_eq(cur_val, min_f);
        // Last row = MAX.
        builder.when_last_row().assert_eq(cur_val, max_f);

        // Transition: next − cur ∈ {0, 1}. Encoded as δ·(δ-1) = 0.
        let delta: AB::Expr = AB::Expr::from(nxt_val) - AB::Expr::from(cur_val);
        let delta_minus_one = delta.clone() - <AB::Expr as PrimeCharacteristicRing>::ONE;
        builder
            .when_transition()
            .assert_zero(delta * delta_minus_one);
    }

    /// Fill the `COL` cell of a trace row with the enumerated table
    /// value. Caller passes the row index; this maps `row_idx → MIN
    /// + min(row_idx, MAX − MIN)` so out-of-range rows replay the
    /// max value.
    pub fn fill_row(&self, row_idx: usize, row: &mut [crate::Val]) {
        use p3_field::integers::QuotientMap;
        let span = Self::span();
        let capped = row_idx.min(span - 1);
        let v: i32 = MIN + capped as i32;
        row[COL] = <crate::Val as QuotientMap<i32>>::from_int(v);
    }
}

/// 0..=255 (`u8`) range table at column [`URANGE8_TABLE`].
pub type URange8Chip = RangeTableChip<URANGE8_TABLE, 0, 255>;

/// 0..=8191 (`u13`) range table at column [`URANGE13_TABLE`].
/// `8191 = 2^BITS_PER_LIMB − 1` (Pearl's limb width).
pub type URange13Chip = RangeTableChip<URANGE13_TABLE, 0, { (1i32 << BITS_PER_LIMB) - 1 }>;

/// -64..=64 (Pearl's `i7+1`) range table at column [`IRANGE7P1_TABLE`].
/// One value wider than i7 — covers the `[-64, 64]` interval Pearl
/// uses for noise.
pub type IRange7P1Chip = RangeTableChip<IRANGE7P1_TABLE, -64, 64>;

/// -128..=127 (`i8`) range table at column [`IRANGE8_TABLE`].
pub type IRange8Chip = RangeTableChip<IRANGE8_TABLE, -128, 127>;

#[cfg(test)]
mod tests {
    use p3_air::{Air, AirBuilder, BaseAir};
    use p3_field::integers::QuotientMap;
    use p3_matrix::dense::RowMajorMatrix;
    use p3_uni_stark::{prove, verify};

    use super::*;
    use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
    use crate::composite_layout::{
        IRANGE7P1_TABLE, IRANGE8_TABLE, TOTAL_TRACE_WIDTH, URANGE13_TABLE, URANGE8_TABLE,
    };
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

    /// Generic test wrapper: AIR that only exercises one
    /// RangeTableChip instantiation.
    #[derive(Debug, Default)]
    struct RangeOnlyAir<C>(core::marker::PhantomData<C>);

    impl<C> RangeOnlyAir<C> {
        fn new() -> Self {
            Self(core::marker::PhantomData)
        }
    }

    impl<const COL: usize, const MIN: i32, const MAX: i32, F> BaseAir<F>
        for RangeOnlyAir<RangeTableChip<COL, MIN, MAX>>
    {
        fn width(&self) -> usize {
            TOTAL_TRACE_WIDTH
        }
    }

    impl<AB: AirBuilder, const COL: usize, const MIN: i32, const MAX: i32> Air<AB>
        for RangeOnlyAir<RangeTableChip<COL, MIN, MAX>>
    {
        fn eval(&self, builder: &mut AB) {
            <RangeTableChip<COL, MIN, MAX>>::new().eval(builder);
        }
    }

    /// Build a valid table trace covering `[MIN..=MAX]` padded to
    /// `rows` total height. `rows` must be ≥ span and a power of two.
    fn build_valid_table_trace<const COL: usize, const MIN: i32, const MAX: i32>(
        rows: usize,
    ) -> RowMajorMatrix<Val> {
        let chip = RangeTableChip::<COL, MIN, MAX>::new();
        let span = RangeTableChip::<COL, MIN, MAX>::span();
        assert!(rows.is_power_of_two() && rows >= span);
        let mut flat: Vec<Val> = vec![Val::default(); rows * TOTAL_TRACE_WIDTH];
        for r in 0..rows {
            let mut row = &mut flat[r * TOTAL_TRACE_WIDTH..(r + 1) * TOTAL_TRACE_WIDTH];
            chip.fill_row(r, &mut row);
        }
        RowMajorMatrix::new(flat, TOTAL_TRACE_WIDTH)
    }

    // =====================================================================
    //  URange8 (0..=255). 256 values fit exactly into 256 rows
    //  (power of two). Tests at 256 rows.
    // =====================================================================

    #[test]
    fn urange8_span_is_256() {
        assert_eq!(URange8Chip::span(), 256);
    }

    #[test]
    fn urange8_table_fills_correctly() {
        let trace = build_valid_table_trace::<URANGE8_TABLE, 0, 255>(256);
        use p3_field::PrimeField64;
        for r in 0..256 {
            let v = trace.values[r * TOTAL_TRACE_WIDTH + URANGE8_TABLE].as_canonical_u64();
            assert_eq!(v, r as u64, "URANGE8 row {r}");
        }
    }

    #[test]
    fn prove_and_verify_urange8_table() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<URange8Chip>::new();
        let trace = build_valid_table_trace::<URANGE8_TABLE, 0, 255>(256);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("valid URANGE8 table must verify");
    }

    /// Property: first row must equal MIN.
    #[test]
    fn urange8_verify_rejects_wrong_first_row() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<URange8Chip>::new();
        let mut trace = build_valid_table_trace::<URANGE8_TABLE, 0, 255>(256);
        // Make row 0's URANGE8_TABLE value 5 (should be 0).
        trace.values[URANGE8_TABLE] = <Val as QuotientMap<u64>>::from_int(5);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    /// Property: last row must equal MAX.
    #[test]
    fn urange8_verify_rejects_wrong_last_row() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<URange8Chip>::new();
        let mut trace = build_valid_table_trace::<URANGE8_TABLE, 0, 255>(256);
        // Force row 255's value to 100 (should be 255).
        let last_row = 255 * TOTAL_TRACE_WIDTH + URANGE8_TABLE;
        // Also adjust row 254 to bridge — keep it 254. Then 254→100
        // delta = -154, not ∈ {0, 1}.
        // Actually simpler: make row 255 = 100 directly; transition
        // 254→100 will fail δ=−154 first.
        trace.values[last_row] = <Val as QuotientMap<u64>>::from_int(100);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    /// Property: transition delta must be 0 or 1. Patch a mid-row to
    /// break the chain (delta ≠ {0, 1}).
    #[test]
    fn urange8_verify_rejects_non_boolean_delta() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<URange8Chip>::new();
        let mut trace = build_valid_table_trace::<URANGE8_TABLE, 0, 255>(256);
        // Row 100 was 100, replace with 50 → transition 99→50
        // is δ = -49 (not boolean).
        let r = 100 * TOTAL_TRACE_WIDTH + URANGE8_TABLE;
        trace.values[r] = <Val as QuotientMap<u64>>::from_int(50);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    // =====================================================================
    //  IRange7P1 (-64..=64). 129 values — repeats max in padding.
    //  Tests at 256 rows (next power of two > 129).
    // =====================================================================

    #[test]
    fn irange7p1_span_is_129() {
        assert_eq!(IRange7P1Chip::span(), 129);
    }

    #[test]
    fn irange7p1_table_starts_at_neg64() {
        let trace = build_valid_table_trace::<IRANGE7P1_TABLE, -64, 64>(256);
        use p3_field::PrimeField64;
        let row0_val = trace.values[IRANGE7P1_TABLE].as_canonical_u64();
        let neg64_field: Val = <Val as QuotientMap<i32>>::from_int(-64);
        assert_eq!(row0_val, neg64_field.as_canonical_u64());
    }

    #[test]
    fn prove_and_verify_irange7p1_table() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<IRange7P1Chip>::new();
        let trace = build_valid_table_trace::<IRANGE7P1_TABLE, -64, 64>(256);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("valid IRANGE7P1 table must verify");
    }

    /// Padding rows after the span repeat MAX (i.e. 64). Validate
    /// the padding semantics specifically.
    #[test]
    fn irange7p1_padding_repeats_max() {
        let trace = build_valid_table_trace::<IRANGE7P1_TABLE, -64, 64>(256);
        let chip = IRange7P1Chip::new();
        // Row 200 is past span (129), should repeat MAX = 64.
        let mut expected_row = vec![Val::default(); TOTAL_TRACE_WIDTH];
        chip.fill_row(200, &mut expected_row);
        assert_eq!(
            trace.values[200 * TOTAL_TRACE_WIDTH + IRANGE7P1_TABLE],
            expected_row[IRANGE7P1_TABLE]
        );
        use p3_field::PrimeField64;
        let v: i32 = 64;
        let expected_f: Val = <Val as QuotientMap<i32>>::from_int(v);
        assert_eq!(
            trace.values[200 * TOTAL_TRACE_WIDTH + IRANGE7P1_TABLE].as_canonical_u64(),
            expected_f.as_canonical_u64()
        );
    }

    // =====================================================================
    //  IRange8 (-128..=127). 256 values, exact fit at 256 rows.
    // =====================================================================

    #[test]
    fn irange8_span_is_256() {
        assert_eq!(IRange8Chip::span(), 256);
    }

    #[test]
    fn prove_and_verify_irange8_table() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<IRange8Chip>::new();
        let trace = build_valid_table_trace::<IRANGE8_TABLE, -128, 127>(256);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("valid IRANGE8 table must verify");
    }

    /// Property: transition delta must be 0 or 1 — even for the
    /// signed range. Patch row 100 (which was -28) to -20 →
    /// delta = +8, not boolean.
    #[test]
    fn irange8_verify_rejects_non_boolean_delta() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<IRange8Chip>::new();
        let mut trace = build_valid_table_trace::<IRANGE8_TABLE, -128, 127>(256);
        // Row 100 was -128 + 100 = -28. Replace with -20.
        let r = 100 * TOTAL_TRACE_WIDTH + IRANGE8_TABLE;
        trace.values[r] = <Val as QuotientMap<i32>>::from_int(-20);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        assert!(verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).is_err());
    }

    // =====================================================================
    //  URange13 (0..=8191). Smallest power of two ≥ 8192 is 8192.
    //  Single smoke test (slow due to 8192-row trace).
    // =====================================================================

    #[test]
    fn urange13_span_is_8192() {
        assert_eq!(URange13Chip::span(), 8192);
    }

    #[test]
    fn prove_and_verify_urange13_table() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let air = RangeOnlyAir::<URange13Chip>::new();
        let trace = build_valid_table_trace::<URANGE13_TABLE, 0, 8191>(8192);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
            .expect("valid URANGE13 table must verify");
    }
}
