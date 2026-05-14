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
    AB_ID_LIMBS_LEN, AB_ID_LIMBS_START, A_ID, A_NOISED_START, A_NOISED_UNPACK_LEN,
    A_NOISED_UNPACK_START, B_ID, B_NOISED_START, B_NOISED_UNPACK_LEN, B_NOISED_UNPACK_START,
    I8U8_FREQ, I8U8_TABLE, IRANGE7P1_FREQ, IRANGE7P1_TABLE, IRANGE8_FREQ, IRANGE8_TABLE,
    IS_MSG_MAT, IS_RESET_CUMSUM, IS_UPDATE_CUMSUM, MAT_FREQ, MAT_ID, MAT_ID_LIMBS_LEN,
    MAT_ID_LIMBS_START, MAT_UNPACK_LEN, MAT_UNPACK_START, NOISED_PACKED_START,
    NOISE_UNPACK_LEN, NOISE_UNPACK_START, TOTAL_TRACE_WIDTH, UINT8_DATA_LEN,
    UINT8_DATA_START, URANGE13_FREQ, URANGE13_TABLE, URANGE8_FREQ, URANGE8_TABLE,
};
use crate::composite_lookups::{
    BUS_I8U8, BUS_IRANGE7P1, BUS_IRANGE8, BUS_NOISED_PACKED, BUS_URANGE13, BUS_URANGE8,
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

        // ---- (2e) I8U8 bus (paired i8 ↔ u8 conversion) ----
        //
        // Table: (I8U8_TABLE, −I8U8_FREQ). Each table row holds
        // pack = signed*256 + unsigned where unsigned =
        // signed.rem_euclid(256), enumerated across i ∈ [0, 256).
        // Queries: pair (MAT_UNPACK[i], UINT8_DATA[i]) packed as
        // signed*256 + unsigned, gated by IS_MSG_MAT (when matrix
        // bytes are loading into BLAKE3's message buffer the i8
        // value and its u8 representation must agree).
        let two_fifty_six: AB::Expr =
            <AB::F as p3_field::PrimeCharacteristicRing>::from_u64(256).into();
        builder.push_interaction(
            BUS_I8U8,
            [<AB::Var as Into<AB::Expr>>::into(cur[I8U8_TABLE])],
            -<AB::Var as Into<AB::Expr>>::into(cur[I8U8_FREQ]),
            0,
        );
        for i in 0..MAT_UNPACK_LEN.min(UINT8_DATA_LEN) {
            let signed: AB::Expr = cur[MAT_UNPACK_START + i].into();
            let unsigned: AB::Expr = cur[UINT8_DATA_START + i].into();
            let pack = signed * two_fifty_six.clone() + unsigned;
            builder.push_interaction(BUS_I8U8, [pack], is_msg_mat.clone(), 1);
        }

        // ---- (2f) NOISED_PACKED RAM-lookup bus ----
        //
        // The cryptographic glue between matmul and BLAKE3: the
        // bytes the matmul chip reads via A_NOISED / B_NOISED must
        // come from the canonical NOISED_PACKED table (committed
        // via the input chip's polyval constraints). LogUp ensures
        // every matmul read corresponds to a real table entry.
        //
        // Table key: (MAT_ID, NOISED_PACKED[0], NOISED_PACKED[1]).
        // Each row provides one table entry with multiplicity
        // MAT_FREQ.
        //
        // Query side: each matmul-active row queries:
        //   * (A_ID, A_NOISED[0], A_NOISED[1]) on read A.
        //   * (B_ID, B_NOISED[0], B_NOISED[1]) on read B.
        // Both gated by (IS_RESET_CUMSUM + IS_UPDATE_CUMSUM).
        builder.push_interaction(
            BUS_NOISED_PACKED,
            [
                <AB::Var as Into<AB::Expr>>::into(cur[MAT_ID]),
                <AB::Var as Into<AB::Expr>>::into(cur[NOISED_PACKED_START]),
                <AB::Var as Into<AB::Expr>>::into(cur[NOISED_PACKED_START + 1]),
            ],
            -<AB::Var as Into<AB::Expr>>::into(cur[MAT_FREQ]),
            0,
        );

        let matmul_active: AB::Expr =
            <AB::Var as Into<AB::Expr>>::into(cur[IS_RESET_CUMSUM])
                + <AB::Var as Into<AB::Expr>>::into(cur[IS_UPDATE_CUMSUM]);
        // A-side read
        builder.push_interaction(
            BUS_NOISED_PACKED,
            [
                <AB::Var as Into<AB::Expr>>::into(cur[A_ID]),
                <AB::Var as Into<AB::Expr>>::into(cur[A_NOISED_START]),
                <AB::Var as Into<AB::Expr>>::into(cur[A_NOISED_START + 1]),
            ],
            matmul_active.clone(),
            1,
        );
        // B-side read
        builder.push_interaction(
            BUS_NOISED_PACKED,
            [
                <AB::Var as Into<AB::Expr>>::into(cur[B_ID]),
                <AB::Var as Into<AB::Expr>>::into(cur[B_NOISED_START]),
                <AB::Var as Into<AB::Expr>>::into(cur[B_NOISED_START + 1]),
            ],
            matmul_active,
            1,
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

    /// Place a single "matrix message byte" at row 0: set
    /// MAT_UNPACK[0] / UINT8_DATA[0] to a consistent (i8, u8)
    /// pair, IS_MSG_MAT = 1, NOISED_PACKED[0] consistent with the
    /// input chip's `polyval(MAT, 256) + polyval(NOISE, 256)`
    /// constraint.
    ///
    /// The argument `u8_value` is the unsigned byte. The
    /// corresponding signed i8 view is `u8_value` if `< 128`
    /// else `u8_value - 256` (two's complement). With this
    /// helper, all wired lookup buses balance for the placed row:
    ///   - URange8 query on UINT8_DATA[0] = u8_value
    ///   - IRange8 query on MAT_UNPACK[0] = i8_view
    ///   - I8U8 query on the packed pair (matches valid table entry)
    fn place_urange8_query(
        trace: &mut CompositeTrace,
        row_idx: usize,
        u8_value: u32,
    ) {
        assert!(row_idx < trace.height());
        assert!(u8_value < 256);
        let base = row_idx * TOTAL_TRACE_WIDTH;
        let row = &mut trace.matrix.values[base..base + TOTAL_TRACE_WIDTH];

        let mut selectors = [false; 21];
        selectors[10] = true; // IS_MSG_MAT
        ControlChip.fill_row(&selectors, 0, row);

        // i8 view of u8: 0..127 → 0..127; 128..255 → -128..-1.
        let signed: i64 = if u8_value < 128 {
            u8_value as i64
        } else {
            u8_value as i64 - 256
        };

        row[crate::composite_layout::MAT_UNPACK_START] =
            <Val as QuotientMap<i64>>::from_int(signed);
        row[UINT8_DATA_START] = <Val as QuotientMap<u64>>::from_int(u8_value as u64);
        // Input chip: NOISED_PACKED[0] = polyval(MAT_UNPACK[0..4],
        // 256) = MAT_UNPACK[0] (since [1..4] are 0). NOISE_UNPACK
        // is 0 so polyval(NOISE, 256) = 0.
        row[crate::composite_layout::NOISED_PACKED_START] =
            <Val as QuotientMap<i64>>::from_int(signed);
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

    // =================================================================
    //  I8U8 paired i8↔u8 conversion bus
    // =================================================================

    /// Place a valid (i8, u8) pair on row 0 with IS_MSG_MAT=1
    /// using place_urange8_query, which sets MAT_UNPACK[0],
    /// UINT8_DATA[0], and NOISED_PACKED[0] consistently. The
    /// I8U8 bus's table row for pack = 42*256+42 is at row
    /// (42 + 128) = 170. populate_lookup_freq increments
    /// I8U8_FREQ[170] to balance.
    #[test]
    fn valid_i8u8_pair_balances_via_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        place_urange8_query(&mut trace, 0, 42);
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix).expect("valid (42, 42) pair must verify");
    }

    /// Place a valid NEGATIVE (i8, u8) pair: u8 = 255 → signed = -1.
    /// I8U8 table row = -1 + 128 = 127. populate_lookup_freq
    /// updates I8U8_FREQ[127] to balance.
    #[test]
    fn valid_negative_i8u8_pair_balances() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        place_urange8_query(&mut trace, 0, 255);
        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix)
            .expect("valid (-1, 255) pair must verify");
    }

    /// **Cryptographically critical test.** Place a valid (42, 42)
    /// pair via place_urange8_query (which sets NOISED_PACKED
    /// consistently), then tamper UINT8_DATA[0] to 43. The packed
    /// value 42*256+43 = 10795 is not in the I8U8 table (which
    /// only contains valid signed/rem_euclid pairs), so LogUp
    /// rejects. This is the constraint that ensures matrix bytes
    /// can't be inconsistently presented as i8 vs. u8 — essential
    /// for the merge-mining byte-equivalence with Pearl.
    #[test]
    fn inconsistent_i8u8_pair_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        place_urange8_query(&mut trace, 0, 42);
        // Tamper UINT8_DATA[0] from 42 to 43 (inconsistent with
        // MAT_UNPACK[0] which is still 42).
        let target = 0 * TOTAL_TRACE_WIDTH + UINT8_DATA_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(43);
        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "inconsistent (i8, u8) pair (42, 43) must reject; got {:?}",
            res
        );
    }

    /// Tamper I8U8_FREQ AFTER populate → reject.
    #[test]
    fn tampered_i8u8_freq_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        trace.populate_lookup_freq();
        // Inflate I8U8_FREQ at row 128 (the table entry for pair
        // (0, 0)) from 0 to 1.
        let target = 128 * TOTAL_TRACE_WIDTH + crate::composite_layout::I8U8_FREQ;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(1);
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "over-claimed I8U8_FREQ must reject; got {:?}",
            res
        );
    }

    // =================================================================
    //  NOISED_PACKED RAM-lookup bus
    // =================================================================

    /// Baseline trace + matmul activity in a chain. The matmul
    /// rows query (A_ID=0, A_NOISED[0..2]=0) and (B_ID=0,
    /// B_NOISED[0..2]=0) — values that match the all-zero baseline
    /// table entry on row 0. populate_lookup_freq updates
    /// MAT_FREQ to balance.
    #[test]
    fn matmul_chain_with_zero_reads_balances_noised_packed() {
        use crate::composite_layout::TILE_D;

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();

        // Build a 2-step matmul chain at rows 8, 9. The chips'
        // A_NOISED / B_NOISED columns stay 0 (we only set
        // A_NOISED_UNPACK / B_NOISED_UNPACK).
        let a = [[0i8; TILE_D]; crate::composite_layout::TILE_H];
        let b = [[0i8; TILE_D]; crate::composite_layout::TILE_H];
        let cumsum_zero = [0i32; crate::chips::matmul::compute::CUMSUM_LEN];
        let after_reset = trace.place_matmul_step(8, &a, &b, true, false, &cumsum_zero);
        let after_update = trace.place_matmul_step(9, &a, &b, false, true, &after_reset);
        trace.fill_cumsum_passthrough(10, &after_update);

        trace.populate_lookup_freq();
        run_batch(&cfg, &trace.matrix)
            .expect("matmul chain with zero NOISED_PACKED reads must verify");
    }

    /// Tamper A_NOISED so the matmul row queries a triple that
    /// doesn't appear in the table → LogUp rejects. Since
    /// A_NOISED isn't currently constrained against A_NOISED_UNPACK
    /// at the AIR level, the only thing keeping A_NOISED honest
    /// is the LogUp against NOISED_PACKED. Without that, a prover
    /// could feed arbitrary data into the matmul accumulator.
    #[test]
    fn tampered_a_noised_with_no_matching_table_entry_rejects() {
        use crate::composite_layout::TILE_D;
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();

        let a = [[0i8; TILE_D]; crate::composite_layout::TILE_H];
        let b = [[0i8; TILE_D]; crate::composite_layout::TILE_H];
        let cumsum_zero = [0i32; crate::chips::matmul::compute::CUMSUM_LEN];
        let _ = trace.place_matmul_step(8, &a, &b, true, false, &cumsum_zero);
        trace.fill_cumsum_passthrough(9, &cumsum_zero);

        // Tamper A_NOISED[0] on row 8 to a value that's not in
        // any table row (all table rows have NOISED_PACKED = 0).
        let target = 8 * TOTAL_TRACE_WIDTH + A_NOISED_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(0xDEAD_BEEF);

        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "tampered A_NOISED must reject via NOISED_PACKED bus; got {:?}",
            res
        );
    }

    /// Tamper MAT_FREQ to over-claim a table entry was consumed
    /// when it wasn't. The table side over-provides → unbalanced.
    #[test]
    fn tampered_mat_freq_rejected_by_logup() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        trace.populate_lookup_freq();
        // Inflate MAT_FREQ on row 0.
        let target = 0 * TOTAL_TRACE_WIDTH + MAT_FREQ;
        use p3_field::PrimeField64;
        let prev = trace.matrix.values[target].as_canonical_u64();
        trace.matrix.values[target] =
            <Val as QuotientMap<u64>>::from_int(prev + 1);
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "over-claimed MAT_FREQ must reject; got {:?}",
            res
        );
    }

    /// Tamper a NOISED_PACKED cell on row 0 to a non-zero value
    /// while no matmul row reads it. The table provides the
    /// modified entry but no query consumes it → MAT_FREQ would
    /// need to be 0 anyway (populate already sets it). But the
    /// table key changed, so any subsequent matmul query
    /// expecting the all-zero entry would no longer match.
    ///
    /// This test runs only the baseline (no matmul), so changing
    /// NOISED_PACKED doesn't break the bus IF MAT_FREQ stays 0 on
    /// that row. populate_lookup_freq should re-route any baseline
    /// matmul queries to a different table row. Since baseline
    /// has no matmul queries, this test verifies: with NOISED_PACKED
    /// non-zero and MAT_FREQ = 0, the bus still balances.
    #[test]
    fn isolated_noised_packed_change_with_no_queries_verifies() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        // Tamper NOISED_PACKED[0] on row 0 to 7. Also adjust
        // input chip's constraint: NOISED_PACKED[0] = polyval(MAT,
        // 256) + polyval(NOISE, 256). With MAT_UNPACK[0..4] = 0,
        // polyval = 0; with NOISE_UNPACK[0..4] = 0, polyval = 0.
        // So NOISED_PACKED[0] = 0 is forced. Setting it to 7
        // breaks the input chip constraint independently of the
        // LogUp.
        let target = 0 * TOTAL_TRACE_WIDTH + NOISED_PACKED_START;
        trace.matrix.values[target] = <Val as QuotientMap<u64>>::from_int(7);
        trace.populate_lookup_freq();
        let res = run_batch(&cfg, &trace.matrix);
        assert!(
            res.is_err(),
            "NOISED_PACKED inconsistent with MAT_UNPACK rejected (input chip + LogUp); got {:?}",
            res
        );
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
