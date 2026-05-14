//! Phase 14b — `CompositeFullAir` extended with LogUp emissions.
//!
//! `CompositeFullAirWithLookups` is a thin wrapper around
//! [`CompositeFullAir`] that requires its builder to implement
//! [`InteractionBuilder`] (the lookup-aware builder from
//! `p3-lookup`). All of `CompositeFullAir`'s constraints fire
//! identically; on top, this AIR pushes the cross-chip lookup
//! interactions that Phase 11 documented.
//!
//! ## Wiring approach
//!
//! Phase 14b-2 wires **one** lookup bus end-to-end as a proof of
//! concept that scales: `urange8` (u8 range check). Subsequent
//! sub-phases add the remaining buses by the same pattern.
//!
//! Each bus has:
//!   * A **table-side** emission of `(table_value, −freq_value)`
//!     per row, with `count_weight = 0`. The range-table chip
//!     provides every value once across `[MIN..=MAX]`; the
//!     `*_FREQ` column carries how many times that value is
//!     consumed.
//!   * A **query-side** emission of `(query_value, +query_flag)`
//!     per row, with `count_weight = 1`. The chip that has u8
//!     cells emits the queries.
//!
//! For the `urange8` POC the query side reuses `UINT8_DATA[0]` —
//! Pearl populates this column with u8 matrix bytes when
//! `IS_MSG_MAT = 1`. On rows with `IS_MSG_MAT = 0` the query
//! multiplicity is zero, so there's no contribution.
//!
//! ## What this gives us
//!
//! With this wired, `prove_batch` + `verify_batch` will reject
//! traces where:
//!   * `URANGE8_FREQ` is over- or under-claimed relative to the
//!     actual `UINT8_DATA[0]` queries.
//!   * A `UINT8_DATA[0]` cell carries a value outside `[0, 256)`
//!     while `IS_MSG_MAT = 1`.
//!
//! Range-table integrity (TABLE column enumerates `[0..256)`)
//! continues to be enforced by `URange8Chip`'s constraints —
//! see [`crate::chips::range_table`].

use p3_air::{Air, AirBuilder, BaseAir, WindowAccess};
use p3_lookup::InteractionBuilder;

use crate::composite_full_air::CompositeFullAir;
use crate::composite_layout::{
    AB_ID_LIMBS_LEN, AB_ID_LIMBS_START, A_NOISED_UNPACK_LEN, A_NOISED_UNPACK_START,
    B_NOISED_UNPACK_LEN, B_NOISED_UNPACK_START, IRANGE7P1_FREQ, IRANGE7P1_TABLE,
    IRANGE8_FREQ, IRANGE8_TABLE, IS_MSG_MAT, MAT_ID_LIMBS_LEN, MAT_ID_LIMBS_START,
    MAT_UNPACK_LEN, MAT_UNPACK_START, NOISE_UNPACK_LEN, NOISE_UNPACK_START,
    TOTAL_TRACE_WIDTH, UINT8_DATA_LEN, UINT8_DATA_START, URANGE13_FREQ, URANGE13_TABLE,
    URANGE8_FREQ, URANGE8_TABLE,
};
use crate::composite_lookups::{
    BUS_IRANGE7P1, BUS_IRANGE8, BUS_URANGE13, BUS_URANGE8,
};

/// Lookup-aware composite AIR.
///
/// Delegates every constraint to `CompositeFullAir` and adds
/// cross-chip interaction emissions on top. Use with
/// `p3-batch-stark`'s `prove_batch` / `verify_batch`.
#[derive(Copy, Clone, Debug, Default)]
pub struct CompositeFullAirWithLookups;

impl<F> BaseAir<F> for CompositeFullAirWithLookups {
    fn width(&self) -> usize {
        TOTAL_TRACE_WIDTH
    }
}

impl<AB> Air<AB> for CompositeFullAirWithLookups
where
    AB: AirBuilder + InteractionBuilder,
{
    fn eval(&self, builder: &mut AB) {
        // (1) Delegate all constraints to CompositeFullAir.
        <CompositeFullAir as Air<AB>>::eval(&CompositeFullAir, builder);

        let main = builder.main();
        let cur = main.current_slice();
        let is_msg_mat: AB::Expr = cur[IS_MSG_MAT].into();

        // ---- (2a) URange8 bus ----
        //
        // Table: (URANGE8_TABLE, −URANGE8_FREQ).
        // Queries: UINT8_DATA[0..8] gated by IS_MSG_MAT (when the
        // BLAKE3 message buffer is loading matrix bytes, each cell
        // is a u8 query).
        builder.push_interaction(
            BUS_URANGE8,
            [<AB::Var as Into<AB::Expr>>::into(cur[URANGE8_TABLE])],
            -<AB::Var as Into<AB::Expr>>::into(cur[URANGE8_FREQ]),
            0,
        );
        for i in 0..UINT8_DATA_LEN {
            builder.push_interaction(
                BUS_URANGE8,
                [<AB::Var as Into<AB::Expr>>::into(cur[UINT8_DATA_START + i])],
                is_msg_mat.clone(),
                1,
            );
        }

        // ---- (2b) URange13 bus ----
        //
        // Table: (URANGE13_TABLE, −URANGE13_FREQ).
        // Queries: MAT_ID_LIMBS[0..2] + AB_ID_LIMBS[0..4]
        // unconditionally (every row's limb decomposition must
        // be u13).
        builder.push_interaction(
            BUS_URANGE13,
            [<AB::Var as Into<AB::Expr>>::into(cur[URANGE13_TABLE])],
            -<AB::Var as Into<AB::Expr>>::into(cur[URANGE13_FREQ]),
            0,
        );
        for i in 0..MAT_ID_LIMBS_LEN {
            builder.push_interaction(
                BUS_URANGE13,
                [<AB::Var as Into<AB::Expr>>::into(cur[MAT_ID_LIMBS_START + i])],
                <AB::Expr as p3_field::PrimeCharacteristicRing>::ONE,
                1,
            );
        }
        for i in 0..AB_ID_LIMBS_LEN {
            builder.push_interaction(
                BUS_URANGE13,
                [<AB::Var as Into<AB::Expr>>::into(cur[AB_ID_LIMBS_START + i])],
                <AB::Expr as p3_field::PrimeCharacteristicRing>::ONE,
                1,
            );
        }

        // ---- (2c) IRange7P1 bus ----
        //
        // Table: (IRANGE7P1_TABLE, −IRANGE7P1_FREQ).
        // Queries: NOISE_UNPACK[0..8] unconditionally (Pearl's
        // signed noise bytes ∈ [-64, 64]).
        builder.push_interaction(
            BUS_IRANGE7P1,
            [<AB::Var as Into<AB::Expr>>::into(cur[IRANGE7P1_TABLE])],
            -<AB::Var as Into<AB::Expr>>::into(cur[IRANGE7P1_FREQ]),
            0,
        );
        for i in 0..NOISE_UNPACK_LEN {
            builder.push_interaction(
                BUS_IRANGE7P1,
                [<AB::Var as Into<AB::Expr>>::into(cur[NOISE_UNPACK_START + i])],
                <AB::Expr as p3_field::PrimeCharacteristicRing>::ONE,
                1,
            );
        }

        // ---- (2d) IRange8 bus ----
        //
        // Table: (IRANGE8_TABLE, −IRANGE8_FREQ).
        // Queries: A_NOISED_UNPACK[0..32] + B_NOISED_UNPACK[0..32]
        // + MAT_UNPACK[0..8] unconditionally (i8 matrix cells).
        builder.push_interaction(
            BUS_IRANGE8,
            [<AB::Var as Into<AB::Expr>>::into(cur[IRANGE8_TABLE])],
            -<AB::Var as Into<AB::Expr>>::into(cur[IRANGE8_FREQ]),
            0,
        );
        for i in 0..A_NOISED_UNPACK_LEN {
            builder.push_interaction(
                BUS_IRANGE8,
                [<AB::Var as Into<AB::Expr>>::into(cur[A_NOISED_UNPACK_START + i])],
                <AB::Expr as p3_field::PrimeCharacteristicRing>::ONE,
                1,
            );
        }
        for i in 0..B_NOISED_UNPACK_LEN {
            builder.push_interaction(
                BUS_IRANGE8,
                [<AB::Var as Into<AB::Expr>>::into(cur[B_NOISED_UNPACK_START + i])],
                <AB::Expr as p3_field::PrimeCharacteristicRing>::ONE,
                1,
            );
        }
        for i in 0..MAT_UNPACK_LEN {
            builder.push_interaction(
                BUS_IRANGE8,
                [<AB::Var as Into<AB::Expr>>::into(cur[MAT_UNPACK_START + i])],
                <AB::Expr as p3_field::PrimeCharacteristicRing>::ONE,
                1,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    //! End-to-end LogUp tests on the full composite trace.
    //! `prove_batch` enforces the `urange8` bus balance at proof
    //! time. Valid traces verify; over-claimed `URANGE8_FREQ` or
    //! out-of-range `UINT8_DATA[0]` queries are rejected.

    use super::*;
    use crate::chips::control::ControlChip;
    use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
    use crate::composite_layout::{UINT8_DATA_START, URANGE8_FREQ};
    use crate::composite_trace::CompositeTrace;
    use crate::params::ZkParams;
    use crate::Val;

    use p3_batch_stark::{prove_batch, verify_batch, ProverData, StarkInstance};
    use p3_field::integers::QuotientMap;

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

    fn run_batch(
        config: &AiPowStarkConfig,
        trace: &p3_matrix::dense::RowMajorMatrix<Val>,
    ) -> Result<(), String> {
        let air = CompositeFullAirWithLookups;
        let instances = vec![StarkInstance {
            air: &air,
            trace,
            public_values: vec![],
        }];
        let prover_data = ProverData::from_instances(config, &instances);
        let proof = prove_batch(config, &instances, &prover_data);
        verify_batch(config, &[air], &proof, &[vec![]], &prover_data.common)
            .map_err(|e| format!("{:?}", e))
    }

    /// Baseline trace + populate_lookup_freq: all-zero query
    /// cells, populate_lookup_freq accumulates the unconditional
    /// queries (MAT_ID_LIMBS, AB_ID_LIMBS, NOISE_UNPACK,
    /// A/B_NOISED_UNPACK, MAT_UNPACK — all 0 on baseline) into
    /// the corresponding FREQ[0] cells. The lookup balance holds
    /// because every query is at value 0 and the table provides
    /// matching multiplicity.
    #[test]
    fn baseline_balances_via_logup_after_populate() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix).expect("baseline must verify with LogUp");
    }

    /// Place a single u8 query at row 0: set UINT8_DATA[0] = 42,
    /// IS_MSG_MAT = 1 (via ControlChip::fill_row, selector index
    /// 10). populate_lookup_freq scans the trace and writes
    /// URANGE8_FREQ.
    fn place_urange8_query(
        trace: &mut CompositeTrace,
        row_idx: usize,
        value: u32,
    ) {
        assert!(row_idx < trace.height());
        let base = row_idx * TOTAL_TRACE_WIDTH;
        let row = &mut trace.matrix.values[base..base + TOTAL_TRACE_WIDTH];

        // Selector: IS_MSG_MAT at SELECTOR_COLS index 10.
        let mut selectors = [false; 21];
        selectors[10] = true;
        ControlChip.fill_row(&selectors, 0, row);

        // UINT8_DATA[0] = value (other UINT8_DATA cells stay 0).
        row[UINT8_DATA_START] = <Val as QuotientMap<u64>>::from_int(value as u64);
    }

    #[test]
    fn single_u8_query_balances_via_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        place_urange8_query(&mut trace, 0, 42);
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix).expect("single in-range query must verify");
    }

    /// Tamper UINT8_DATA[0] on row 0 to 300 (out of u8 range)
    /// without updating any other cell. With IS_MSG_MAT = 0 on
    /// row 0, the query multiplicity is 0, so the lookup
    /// constraint is silenced — this test verifies the gating
    /// works correctly (no rejection just because we wrote junk
    /// in a u8 cell).
    #[test]
    fn out_of_range_uint8_silenced_when_is_msg_mat_zero() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let target = 0 * TOTAL_TRACE_WIDTH + UINT8_DATA_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(300);
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix)
            .expect("out-of-range u8 with IS_MSG_MAT=0 must still verify");
    }

    /// Place a query at value 300 (out of u8 range). The trace
    /// has no URANGE8_TABLE entry for value 300, so the LogUp
    /// argument can't balance the bus → reject.
    #[test]
    fn out_of_range_uint8_with_active_query_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();

        // Set up the query: IS_MSG_MAT = 1, UINT8_DATA[0] = 300.
        let base = 0 * TOTAL_TRACE_WIDTH;
        let row = &mut trace.matrix.values[base..base + TOTAL_TRACE_WIDTH];
        let mut selectors = [false; 21];
        selectors[10] = true;
        ControlChip.fill_row(&selectors, 0, row);
        row[UINT8_DATA_START] = <Val as QuotientMap<u64>>::from_int(300);
        trace.populate_lookup_freq();

        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "out-of-range query must reject via LogUp; got {:?}",
            res
        );
    }

    /// Tamper URANGE8_FREQ AFTER populate_lookup_freq to claim a
    /// query was made when none was. The table over-provides →
    /// unbalanced → reject.
    #[test]
    fn over_claimed_urange8_freq_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        trace.populate_lookup_freq();
        let target = 42 * TOTAL_TRACE_WIDTH + URANGE8_FREQ;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(1);
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "over-claimed URANGE8_FREQ must reject; got {:?}",
            res
        );
    }

    /// Place a query but don't call populate_lookup_freq. The
    /// FREQ column is stale → unbalanced → reject.
    #[test]
    fn under_claimed_urange8_freq_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        place_urange8_query(&mut trace, 0, 42);
        // Intentionally NOT calling populate_lookup_freq.

        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "missing URANGE8_FREQ must reject; got {:?}",
            res
        );
    }

    /// Multiple queries at the same u8 value: FREQ correctly
    /// counts the multiplicity.
    #[test]
    fn multi_query_same_value_balances() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        for r in 0..5 {
            place_urange8_query(&mut trace, r, 42);
        }
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix).expect("5 queries at value 42 must verify");
    }

    /// Several queries at different values: each value's FREQ
    /// reflects its own count.
    #[test]
    fn queries_at_different_values_balance() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        place_urange8_query(&mut trace, 0, 1);
        place_urange8_query(&mut trace, 1, 42);
        place_urange8_query(&mut trace, 2, 200);
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix).expect("3 distinct queries must verify");
    }

    // =================================================================
    //  URange13 / IRange7P1 / IRange8 unconditional-query buses
    // =================================================================

    /// Tamper a MAT_ID_LIMBS cell to an out-of-u13 value (9000 ∉
    /// [0, 8192)). populate_lookup_freq won't increment any FREQ
    /// for 9000 (out of range), so the query side has +1 with no
    /// matching table entry → unbalanced.
    #[test]
    fn out_of_range_mat_id_limb_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let target = 0 * TOTAL_TRACE_WIDTH + crate::composite_layout::MAT_ID_LIMBS_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(9000);
        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "MAT_ID_LIMBS out of u13 range must reject; got {:?}",
            res
        );
    }

    /// Tamper a NOISE_UNPACK cell to an out-of-i7+1 value (100 ∉
    /// [-64, 64]).
    #[test]
    fn out_of_range_noise_unpack_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let target = 0 * TOTAL_TRACE_WIDTH + crate::composite_layout::NOISE_UNPACK_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(100);
        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "NOISE_UNPACK out of [-64, 64] must reject; got {:?}",
            res
        );
    }

    /// Tamper an A_NOISED_UNPACK cell to an out-of-i8 value (200
    /// ∉ [-128, 127]).
    #[test]
    fn out_of_range_a_noised_unpack_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let target =
            0 * TOTAL_TRACE_WIDTH + crate::composite_layout::A_NOISED_UNPACK_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(200);
        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "A_NOISED_UNPACK out of i8 range must reject; got {:?}",
            res
        );
    }

    /// Tamper a B_NOISED_UNPACK cell to a NEGATIVE out-of-i8
    /// value (-200, encoded as Goldilocks_p − 200, which falls
    /// outside [-128, 127]).
    #[test]
    fn out_of_range_b_noised_unpack_negative_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let target =
            0 * TOTAL_TRACE_WIDTH + crate::composite_layout::B_NOISED_UNPACK_START;
        trace.matrix.values[target] = <Val as QuotientMap<i64>>::from_int(-200);
        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "negative out-of-range B_NOISED_UNPACK must reject; got {:?}",
            res
        );
    }

    /// Tamper a MAT_UNPACK cell to an out-of-i8 value.
    #[test]
    fn out_of_range_mat_unpack_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let target = 0 * TOTAL_TRACE_WIDTH + crate::composite_layout::MAT_UNPACK_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(150);
        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "MAT_UNPACK out of i8 range must reject; got {:?}",
            res
        );
    }

    // Note: a positive-direction in_range_noise_unpack_balances
    // would also have to update NOISE_PACKED_PREP to satisfy the
    // input chip's `polyval(NOISE_UNPACK, base=129) ==
    // NOISE_PACKED_PREP` constraint. That conflates the LogUp
    // test with a per-chip constraint. The out-of-range
    // rejection test above is sufficient (both constraints fail
    // in the rejection direction).

    /// In-range A_NOISED_UNPACK with a mix of positive and
    /// negative i8 values balances.
    #[test]
    fn in_range_a_noised_unpack_balances() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let row_start = 0 * TOTAL_TRACE_WIDTH;
        let test_vals: [(usize, i64); 4] = [(0, -128), (1, 127), (2, 0), (3, -1)];
        for (off, v) in test_vals {
            trace.matrix.values
                [row_start + crate::composite_layout::A_NOISED_UNPACK_START + off] =
                <Val as QuotientMap<i64>>::from_int(v);
        }
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix)
            .expect("in-range i8 A_NOISED_UNPACK values must verify");
    }

    /// Tamper URANGE13_FREQ AFTER populate (over-claim) → reject.
    #[test]
    fn tampered_urange13_freq_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        trace.populate_lookup_freq();
        // populate_lookup_freq set URANGE13_FREQ[0] = (2 + 4) ×
        // 8192 = 49152 (all baseline limbs are 0). Bump it.
        let target = 0 * TOTAL_TRACE_WIDTH + crate::composite_layout::URANGE13_FREQ;
        use p3_field::PrimeField64;
        let prev = trace.matrix.values[target].as_canonical_u64();
        trace.matrix.values[target] =
            <Val as QuotientMap<u64>>::from_int(prev + 1);
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "over-claimed URANGE13_FREQ[0] must reject; got {:?}",
            res
        );
    }
}
