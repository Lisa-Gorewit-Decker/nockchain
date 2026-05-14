//! M10.1c composite AIR — Phase 12 integration layer.
//!
//! Port of `pearl/zk-pow/src/circuit/pearl_air.rs:46-89` — the
//! top-level `eval` that wires every chip's constraints into a
//! single AIR over [`composite_layout`]'s `TOTAL_TRACE_WIDTH`
//! columns.
//!
//! ## Phase scope (12a — what's wired here)
//!
//! Phase 12 lands in two slices so the integration is incremental:
//!
//! * **12a (this commit)** — Phase 3-6 chips that already read by
//!   `composite_layout` offsets. These slot in directly:
//!     * [`stark_row`](crate::chips::stark_row::StarkRowChip)
//!     * [`range_table`](crate::chips::range_table) — `URange8`,
//!       `URange13`, `IRange7P1`, `IRange8`
//!     * [`i8u8`](crate::chips::i8u8::I8U8Chip)
//!     * [`control`](crate::chips::control::ControlChip)
//!     * [`input`](crate::chips::input::InputChip)
//! * **12b (pending)** — Phase 7-10 chips that currently use a
//!   chip-local layout. Wiring them needs a refactor pass: each
//!   chip's eval lifts to a free function taking column offsets
//!   so `CompositeFullAir` can pass `composite_layout`'s offsets.
//!     * [`blake3`](crate::chips::blake3)
//!     * [`matmul`](crate::chips::matmul)
//!     * [`jackpot`](crate::chips::jackpot)
//!
//! ## Per-row dispatch
//!
//! Every chip's constraint is **always on** at this layer. Per-row
//! activity selection (via CONTROL_PREP unpacking, IS_NEW_BLAKE,
//! etc.) is what makes individual chip constraints "fire" or
//! silence on a given row. The composite AIR's job is just to
//! collect them all.
//!
//! ## Trace shape
//!
//! `TOTAL_TRACE_WIDTH × N` where `N >= MIN_STARK_LEN = 8192`.
//! Padding rows that aren't filled by any chip are all-zero; the
//! all-zero pattern satisfies every wired-in chip's constraints
//! (range-table boundaries are filled by `fill_row` past `span`,
//! all selectors are 0, all data columns are 0, control_prep = 0,
//! mat_id = 0, etc.).

use p3_air::{Air, AirBuilder, BaseAir};

use crate::chips::blake3::chip::Blake3Chip;
use crate::chips::control::ControlChip;
use crate::chips::i8u8::I8U8Chip;
use crate::chips::input::InputChip;
use crate::chips::matmul::chip::MatmulCumsumChip;
use crate::chips::range_table::{IRange7P1Chip, IRange8Chip, URange13Chip, URange8Chip};
use crate::chips::stark_row::StarkRowChip;
use crate::composite_layout::TOTAL_TRACE_WIDTH;

/// The M10.1c composite AIR (Phase 12a slice).
///
/// Trace width: [`TOTAL_TRACE_WIDTH`]. The constraint-bearing
/// chips wired here are Phase 3-6's. Phase 12b adds Phase 7-10's
/// chips (BLAKE3, matmul, jackpot).
#[derive(Copy, Clone, Debug, Default)]
pub struct CompositeFullAir;

impl<F> BaseAir<F> for CompositeFullAir {
    fn width(&self) -> usize {
        TOTAL_TRACE_WIDTH
    }
}

impl<AB: AirBuilder> Air<AB> for CompositeFullAir {
    fn eval(&self, builder: &mut AB) {
        // STARK_ROW_IDX monotonic.
        StarkRowChip.eval(builder);

        // Range tables: enforce table integrity.
        URange8Chip::default().eval(builder);
        URange13Chip::default().eval(builder);
        IRange7P1Chip::default().eval(builder);
        IRange8Chip::default().eval(builder);

        // I8U8 conversion table.
        I8U8Chip.eval(builder);

        // CONTROL_PREP unpacking + MAT_ID limb decomposition.
        ControlChip.eval(builder);

        // Input chip: NOISE_PACKED_PREP unpacking + NOISED_PACKED
        // = polyval(MAT, 256) + polyval(NOISE, 256) integrity.
        InputChip.eval(builder);

        // Matmul cumsum-update chip (Phase 12b wiring): reads
        // A_NOISED_UNPACK / B_NOISED_UNPACK / CUMSUM_TILE /
        // IS_RESET_CUMSUM / IS_UPDATE_CUMSUM at composite-layout
        // offsets.
        MatmulCumsumChip::eval_composite(builder);

        // BLAKE3 chip (Phase 12c wiring): reads BLAKE3_ROUND (4
        // state snapshots), BLAKE3_MSG, BLAKE3_CV, CV_OR_TWEAK_PREP,
        // CV_OUT at composite-layout offsets. Dispatch driven by
        // IS_NEW_BLAKE / IS_LAST_ROUND selector bits (unpacked from
        // CONTROL_PREP by ControlChip).
        Blake3Chip::eval_composite(builder);
    }
}

#[cfg(test)]
mod tests {
    //! End-to-end integration test: build a TOTAL_TRACE_WIDTH ×
    //! MIN_STARK_LEN trace where every wired chip's columns are
    //! filled correctly, then prove + verify.

    use super::*;
    use crate::chips::i8u8::I8U8_TABLE_SIZE;
    use crate::chips::range_table::{IRange7P1Chip, IRange8Chip, URange13Chip, URange8Chip};
    use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
    use crate::composite_layout::{MIN_STARK_LEN, STARK_ROW_IDX, TOTAL_TRACE_WIDTH};
    use crate::params::ZkParams;

    use p3_field::integers::QuotientMap;
    use p3_matrix::dense::RowMajorMatrix;
    use p3_uni_stark::{prove, verify};

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

    /// Build a baseline trace of `n` rows where the wired chips
    /// are satisfied:
    ///   * STARK_ROW_IDX = 0, 1, 2, ..., n-1.
    ///   * Range tables filled by their fill_row helpers (so the
    ///     last row equals MAX).
    ///   * I8U8 table filled by its fill_row helper.
    ///   * All other columns = 0 (selectors off, data = 0 satisfies
    ///     control's CONTROL_PREP = 0 and input chip's degenerate
    ///     polyval = 0 + 0 = 0).
    fn build_baseline_trace(n: usize) -> RowMajorMatrix<crate::Val> {
        assert!(n.is_power_of_two(), "trace length must be power of 2");
        let mut flat = vec![crate::Val::default(); n * TOTAL_TRACE_WIDTH];

        for r in 0..n {
            let row_start = r * TOTAL_TRACE_WIDTH;
            let row = &mut flat[row_start..row_start + TOTAL_TRACE_WIDTH];

            // STARK_ROW_IDX = r.
            row[STARK_ROW_IDX] = <crate::Val as QuotientMap<u64>>::from_int(r as u64);

            // Range table cells.
            URange8Chip::default().fill_row(r, row);
            URange13Chip::default().fill_row(r, row);
            IRange7P1Chip::default().fill_row(r, row);
            IRange8Chip::default().fill_row(r, row);

            // I8U8 table cells.
            I8U8Chip.fill_row(r, row);

            // CONTROL_PREP / MAT_ID / NOISE_UNPACK / MAT_UNPACK
            // / NOISED_PACKED all left as 0 — control + input
            // chips' constraints all degenerate to 0 = 0 in this
            // case.
        }

        RowMajorMatrix::new(flat, TOTAL_TRACE_WIDTH)
    }

    #[test]
    fn composite_full_air_baseline_trace_verifies() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = build_baseline_trace(MIN_STARK_LEN);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[])
            .expect("baseline composite trace must verify");
    }

    /// Tamper STARK_ROW_IDX — should reject.
    #[test]
    fn composite_full_air_rejects_bad_row_idx() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        // Set row 3's STARK_ROW_IDX to 999 instead of 3.
        let target = 3 * TOTAL_TRACE_WIDTH + STARK_ROW_IDX;
        trace.values[target] = <crate::Val as QuotientMap<u64>>::from_int(999);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "tampered STARK_ROW_IDX must reject"
        );
    }

    /// Tamper a range table cell (URANGE8_TABLE row 1 — should be
    /// 1, set to 5). The transition delta check `(table[i+1] −
    /// table[i]) ∈ {0, 1}` rejects.
    #[test]
    fn composite_full_air_rejects_bad_range_table() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        use crate::composite_layout::URANGE8_TABLE;
        let target = 1 * TOTAL_TRACE_WIDTH + URANGE8_TABLE;
        trace.values[target] = <crate::Val as QuotientMap<u64>>::from_int(5);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "tampered range table must reject"
        );
    }

    /// Tamper I8U8 AUX. AUX must start at 0, become 1 at the
    /// sign-boundary row, and stay 1. Setting AUX = 1 on row 0
    /// breaks the first-row constraint.
    #[test]
    fn composite_full_air_rejects_bad_i8u8_aux() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        use crate::composite_layout::I8U8_AUX;
        let target = 0 * TOTAL_TRACE_WIDTH + I8U8_AUX;
        trace.values[target] = <crate::Val as QuotientMap<u64>>::from_int(1);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "tampered I8U8_AUX must reject"
        );
    }

    /// Tamper CONTROL_PREP — set a selector bit without updating
    /// CONTROL_PREP. The control chip's constraint
    /// `CONTROL_PREP == polyval(selectors..., mat_id; base=2)`
    /// rejects.
    #[test]
    fn composite_full_air_rejects_inconsistent_control_prep() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        use crate::composite_layout::IS_RESET_CUMSUM;
        // Flip IS_RESET_CUMSUM on row 0 without updating CONTROL_PREP.
        let target = 0 * TOTAL_TRACE_WIDTH + IS_RESET_CUMSUM;
        trace.values[target] = <crate::Val as QuotientMap<u64>>::from_int(1);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "inconsistent CONTROL_PREP must reject"
        );
    }

    /// Tamper NOISED_PACKED without updating MAT_UNPACK / NOISE_UNPACK.
    /// The input chip's constraint forces NOISED_PACKED[i] ==
    /// polyval(MAT[i*4..(i+1)*4], 256) + polyval(NOISE[i*4..(i+1)*4],
    /// 256). Changing NOISED_PACKED but not the unpacks rejects.
    #[test]
    fn composite_full_air_rejects_inconsistent_noised_packed() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        use crate::composite_layout::NOISED_PACKED_START;
        let target = 0 * TOTAL_TRACE_WIDTH + NOISED_PACKED_START;
        trace.values[target] = <crate::Val as QuotientMap<u64>>::from_int(42);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "inconsistent NOISED_PACKED must reject"
        );
    }

    #[test]
    fn composite_full_air_width_matches_total_trace_width() {
        let air = CompositeFullAir;
        let w = <CompositeFullAir as BaseAir<crate::Val>>::width(&air);
        assert_eq!(w, TOTAL_TRACE_WIDTH);
    }

    /// Production-scale anchor: at exactly MIN_STARK_LEN (8192)
    /// rows the trace passes. This is the row count Pearl pins
    /// for its smallest stark proof; bigger sizes are powers of 2
    /// up.
    #[test]
    fn composite_full_air_min_stark_len_anchor() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = build_baseline_trace(MIN_STARK_LEN);
        assert_eq!(
            trace.values.len(),
            MIN_STARK_LEN * TOTAL_TRACE_WIDTH,
            "trace dimensions"
        );
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[])
            .expect("min-stark-len trace must verify");
    }

    /// Sanity: I8U8 table-size matches Pearl's `1 << 8 = 256`.
    #[test]
    fn i8u8_table_size_pinned() {
        assert_eq!(I8U8_TABLE_SIZE, 256);
    }

    /// Tamper a CUMSUM_TILE cell — the matmul cumsum-update
    /// constraint (gated by IS_RESET_CUMSUM + IS_UPDATE_CUMSUM)
    /// becomes `next = (0 + 0) * dot + (1 - 0) * cur = cur`, so
    /// any cross-row change to CUMSUM_TILE rejects.
    #[test]
    fn composite_full_air_rejects_changed_cumsum_without_selectors() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        use crate::composite_layout::CUMSUM_TILE_START;
        // Set CUMSUM_TILE[0] on row 1 to 42 while row 0 is still 0.
        // With both selectors zero (passthrough mode), the matmul
        // constraint forces row 1's CUMSUM = row 0's CUMSUM = 0.
        let target = 1 * TOTAL_TRACE_WIDTH + CUMSUM_TILE_START;
        trace.values[target] = <crate::Val as QuotientMap<u64>>::from_int(42);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "tampered CUMSUM_TILE in passthrough mode must reject"
        );
    }

    /// Tamper a BLAKE3 state bit (in STATE1.row2) — the BLAKE3
    /// round constraint asserts boolean bits in every state
    /// snapshot via xor_32_shift_if's `assert_bool` calls.
    /// Setting a row2 cell to 2 violates booleanity, regardless of
    /// selectors.
    #[test]
    fn composite_full_air_rejects_non_boolean_blake3_state_bit() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        use crate::composite_layout::BLAKE3_ROUND_START;
        // STATE1.row2[0] starts at offset BLAKE3_ROUND_START +
        // STATE_W + 4 (= STATE1 cells 4..36 hold row2[0]'s bits).
        const STATE_W: usize = 264;
        let target = 0 * TOTAL_TRACE_WIDTH + BLAKE3_ROUND_START + STATE_W + 4;
        trace.values[target] = <crate::Val as QuotientMap<u64>>::from_int(2);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "non-boolean BLAKE3 state bit must reject"
        );
    }

    /// Tamper an A_NOISED_UNPACK cell *without* setting either
    /// matmul selector. Since both selectors are 0, the dot
    /// product term is multiplied by `(is_reset + is_update) = 0`
    /// and the change has no effect. The constraint stays
    /// satisfied — this test is a regression anchor confirming
    /// the gating actually silences correctly.
    #[test]
    fn composite_full_air_accepts_changed_a_unpack_in_passthrough() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = build_baseline_trace(MIN_STARK_LEN);
        use crate::composite_layout::A_NOISED_UNPACK_START;
        let target = 1 * TOTAL_TRACE_WIDTH + A_NOISED_UNPACK_START;
        trace.values[target] = <crate::Val as QuotientMap<i64>>::from_int(100);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[])
            .expect("change to A_NOISED_UNPACK in passthrough mode must verify");
    }
}
