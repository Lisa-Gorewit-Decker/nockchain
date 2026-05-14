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
    IS_MSG_MAT, TOTAL_TRACE_WIDTH, UINT8_DATA_START, URANGE8_FREQ, URANGE8_TABLE,
};
use crate::composite_lookups::BUS_URANGE8;

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

        // (2) Emit the `urange8` lookup bus on every row.
        let main = builder.main();
        let cur = main.current_slice();

        // Table side: provide entry (URANGE8_TABLE[r], -URANGE8_FREQ[r]).
        builder.push_interaction(
            BUS_URANGE8,
            [<AB::Var as Into<AB::Expr>>::into(cur[URANGE8_TABLE])],
            -<AB::Var as Into<AB::Expr>>::into(cur[URANGE8_FREQ]),
            0, // count_weight = 0 for table entries
        );

        // Query side: UINT8_DATA[0] is a u8 byte when IS_MSG_MAT = 1.
        // Multiplicity = IS_MSG_MAT (0 or 1, so each row contributes
        // at most one query).
        builder.push_interaction(
            BUS_URANGE8,
            [<AB::Var as Into<AB::Expr>>::into(cur[UINT8_DATA_START])],
            <AB::Var as Into<AB::Expr>>::into(cur[IS_MSG_MAT]),
            1, // count_weight = 1 for queries
        );
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

    /// Baseline trace: UINT8_DATA[0] = 0 everywhere; IS_MSG_MAT = 0
    /// everywhere; URANGE8_FREQ = 0 everywhere. The lookup
    /// balance is trivially zero on both sides → verifies.
    #[test]
    fn baseline_urange8_balances_via_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        run_batch(&cfg, &trace.matrix).expect("baseline must verify with LogUp");
    }

    /// Place a single u8 query at row 0: set UINT8_DATA[0] = 42,
    /// IS_MSG_MAT = 1 (via ControlChip::fill_row, selector index
    /// 10), and increment URANGE8_FREQ[42] to balance the bus.
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

        // UINT8_DATA[0] = value.
        row[UINT8_DATA_START] = <Val as QuotientMap<u64>>::from_int(value as u64);

        // Increment URANGE8_FREQ[value] (the table's entry for
        // `value` lives on row `value` of the URange8 table, since
        // URANGE8_TABLE[r] = r for r < 256).
        let freq_row_idx = (value as usize) * TOTAL_TRACE_WIDTH + URANGE8_FREQ;
        use p3_field::PrimeField64;
        let prev = trace.matrix.values[freq_row_idx].as_canonical_u64();
        trace.matrix.values[freq_row_idx] =
            <Val as QuotientMap<u64>>::from_int(prev + 1);
    }

    #[test]
    fn single_u8_query_balances_via_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        place_urange8_query(&mut trace, 0, 42);
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
        // No URANGE8_FREQ update — there's no table entry for 300.

        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "out-of-range query must reject via LogUp; got {:?}",
            res
        );
    }

    /// Tamper URANGE8_FREQ to claim a query was made when none
    /// was. The table over-provides → unbalanced → reject.
    #[test]
    fn over_claimed_urange8_freq_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        // Inflate URANGE8_FREQ[42] from 0 to 1 with no actual
        // query.
        let target = 42 * TOTAL_TRACE_WIDTH + URANGE8_FREQ;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(1);
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "over-claimed URANGE8_FREQ must reject; got {:?}",
            res
        );
    }

    /// Place a query but DON'T increment URANGE8_FREQ. The query
    /// side over-consumes → unbalanced → reject.
    #[test]
    fn under_claimed_urange8_freq_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();

        // Set up the query without updating FREQ.
        let base = 0 * TOTAL_TRACE_WIDTH;
        let row = &mut trace.matrix.values[base..base + TOTAL_TRACE_WIDTH];
        let mut selectors = [false; 21];
        selectors[10] = true;
        ControlChip.fill_row(&selectors, 0, row);
        row[UINT8_DATA_START] = <Val as QuotientMap<u64>>::from_int(42);

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
        run_batch(&cfg, &trace.matrix).expect("5 queries at value 42 must verify");
    }

    /// Several queries at different values: each value's FREQ
    /// reflects its own count.
    #[test]
    fn queries_at_different_values_balance() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        // Row 0 queries 1; row 1 queries 42; row 2 queries 200.
        place_urange8_query(&mut trace, 0, 1);
        place_urange8_query(&mut trace, 1, 42);
        place_urange8_query(&mut trace, 2, 200);
        run_batch(&cfg, &trace.matrix).expect("3 distinct queries must verify");
    }
}
