//! Composite trace generator for the M10.1c AIR.
//!
//! Port of `pearl/zk-pow/src/circuit/pearl_trace.rs` — produces a
//! `TOTAL_TRACE_WIDTH × N` trace matrix from a high-level
//! "instruction list" (the sequence of hashes, matmul tile
//! updates, jackpot rotations that the proof represents).
//!
//! ## Phase 13 scope
//!
//! This phase ships the **baseline trace builder** that fills the
//! constraint-bearing structural columns:
//!   * STARK_ROW_IDX = 0, 1, ..., N-1.
//!   * 4 range tables enumerate [MIN..=MAX] then replay MAX.
//!   * I8U8 table enumerates all 256 (i8, u8) pairs.
//!   * All remaining columns = 0 (no chip activity).
//!
//! Such a "passthrough" trace verifies under
//! [`crate::composite_full_air::CompositeFullAir`] but represents
//! no actual matmul / BLAKE3 / jackpot work. It's the foundation
//! every higher-level builder extends.
//!
//! ## Instruction-list shape (forward-looking)
//!
//! A full Pearl-style trace generator takes a list of high-level
//! instructions:
//!
//! ```text
//!   pub enum Instr {
//!       MatmulStep { a_id, b_id, is_reset, is_update },
//!       Blake3Hash { msg, cv_in, tweak },
//!       JackpotStep { slot, x, is_active },
//!       Padding,
//!   }
//! ```
//!
//! and compiles each into a contiguous block of rows in the
//! composite trace, threading state across blocks (matmul cumsum
//! chain, BLAKE3 CV routing, jackpot state evolution). The
//! instruction compilation also fills CONTROL_PREP and the
//! preprocessed columns ([`crate::composite_preprocess`]) so the
//! control chip's unpacking constraint is satisfied.
//!
//! Phase 13's minimal deliverable establishes the type surface and
//! the baseline; the multi-instruction generator is left as
//! follow-on work tied to Phase 14's lookup wiring (since
//! instruction blocks determine the lookup multiplicities).

use p3_matrix::dense::RowMajorMatrix;

use crate::chips::control::ControlChip;
use crate::chips::i8u8::I8U8Chip;
use crate::chips::matmul::compute::{compute_row, CUMSUM_LEN};
use crate::chips::range_table::{IRange7P1Chip, IRange8Chip, URange13Chip, URange8Chip};
use crate::composite_layout::{
    A_NOISED_UNPACK_START, B_NOISED_UNPACK_START, CUMSUM_TILE_START, STARK_ROW_IDX, TILE_D,
    TILE_H, TOTAL_TRACE_WIDTH,
};
use crate::Val;

/// A composite trace ready for proving by
/// [`crate::composite_full_air::CompositeFullAir`].
#[derive(Clone, Debug)]
pub struct CompositeTrace {
    /// The TOTAL_TRACE_WIDTH × N matrix; `N` is a power of 2 and
    /// `>= composite_layout::MIN_STARK_LEN = 8192`.
    pub matrix: RowMajorMatrix<Val>,
}

impl CompositeTrace {
    /// Number of rows.
    pub fn height(&self) -> usize {
        self.matrix.values.len() / TOTAL_TRACE_WIDTH
    }

    /// Number of columns. Always [`TOTAL_TRACE_WIDTH`].
    pub fn width(&self) -> usize {
        TOTAL_TRACE_WIDTH
    }

    /// Build a baseline-zero trace of `n` rows.
    ///
    /// `n` must be a power of 2 and at least
    /// `composite_layout::MIN_STARK_LEN = 8192`.
    ///
    /// The resulting trace satisfies every constraint wired into
    /// [`crate::composite_full_air::CompositeFullAir`] but
    /// represents no chip-level activity.
    pub fn baseline(n: usize) -> Self {
        use p3_field::integers::QuotientMap;

        assert!(n.is_power_of_two(), "trace length must be a power of 2");
        assert!(
            n >= crate::composite_layout::MIN_STARK_LEN,
            "trace length {n} below MIN_STARK_LEN = {}",
            crate::composite_layout::MIN_STARK_LEN
        );

        let mut flat = vec![Val::default(); n * TOTAL_TRACE_WIDTH];

        for r in 0..n {
            let row_start = r * TOTAL_TRACE_WIDTH;
            let row = &mut flat[row_start..row_start + TOTAL_TRACE_WIDTH];

            // STARK_ROW_IDX = r.
            row[STARK_ROW_IDX] = <Val as QuotientMap<u64>>::from_int(r as u64);

            // Range table cells: 4 range tables, plus I8U8.
            URange8Chip::default().fill_row(r, row);
            URange13Chip::default().fill_row(r, row);
            IRange7P1Chip::default().fill_row(r, row);
            IRange8Chip::default().fill_row(r, row);
            I8U8Chip.fill_row(r, row);

            // Everything else stays zero; chip-level constraints
            // (control, input, matmul, blake3) all degenerate to
            // satisfied for all-zero rows.
        }

        Self {
            matrix: RowMajorMatrix::new(flat, TOTAL_TRACE_WIDTH),
        }
    }

    /// Build a baseline trace at exactly `MIN_STARK_LEN`. The
    /// smallest verifiable composite proof shape.
    pub fn baseline_min() -> Self {
        Self::baseline(crate::composite_layout::MIN_STARK_LEN)
    }

    /// Place a single matmul step at row `row_idx`. The caller is
    /// responsible for supplying `cumsum_old`, the CUMSUM_TILE
    /// value entering this step (must equal the previous matmul
    /// step's `cumsum_new` for the chain to verify).
    ///
    /// Returns the resulting `cumsum_new` so the caller can thread
    /// it into the next step.
    ///
    /// This is the *single-row* primitive Phase 13b uses; the
    /// caller does the threading across rows. A higher-level
    /// `with_matmul_instrs` builder will land alongside the
    /// instruction-list compiler.
    pub fn place_matmul_step(
        &mut self,
        row_idx: usize,
        a: &[[i8; TILE_D]; TILE_H],
        b: &[[i8; TILE_D]; TILE_H],
        is_reset: bool,
        is_update: bool,
        cumsum_old: &[i32; CUMSUM_LEN],
    ) -> [i32; CUMSUM_LEN] {
        use p3_field::integers::QuotientMap;

        assert!(row_idx < self.height(), "row {row_idx} out of bounds");

        // Selector + CONTROL_PREP via control chip's fill_row.
        // Build the 21-bit selector array, with IS_RESET_CUMSUM
        // and IS_UPDATE_CUMSUM at their composite-layout positions.
        let mut selectors = [false; 21];
        // Index of IS_RESET_CUMSUM = 0 (it's the first selector bit
        // packed into CONTROL_PREP); index of IS_UPDATE_CUMSUM = 1.
        // These match composite_layout::SELECTOR_COLS ordering.
        selectors[0] = is_reset;
        selectors[1] = is_update;

        let row_start = row_idx * TOTAL_TRACE_WIDTH;
        let row = &mut self.matrix.values[row_start..row_start + TOTAL_TRACE_WIDTH];

        // Write control + selector + MAT_ID columns.
        // MAT_ID = 0 (we're not using NOISED_PACKED RAM-lookup yet
        // — that's Phase 14b's LogUp wiring).
        ControlChip.fill_row(&selectors, 0, row);

        // Write A / B unpack cells.
        for i in 0..TILE_H {
            for d in 0..TILE_D {
                row[A_NOISED_UNPACK_START + i * TILE_D + d] =
                    <Val as QuotientMap<i64>>::from_int(a[i][d] as i64);
                row[B_NOISED_UNPACK_START + i * TILE_D + d] =
                    <Val as QuotientMap<i64>>::from_int(b[i][d] as i64);
            }
        }

        // Write CUMSUM = cumsum_old (the "entering" cumsum).
        for k in 0..CUMSUM_LEN {
            row[CUMSUM_TILE_START + k] =
                <Val as QuotientMap<i64>>::from_int(cumsum_old[k] as i64);
        }

        // Compute and return the post-step cumsum.
        compute_row(a, b, cumsum_old, is_reset, is_update)
    }

    /// Patch the CUMSUM_TILE cells at `row_idx`. Used to thread
    /// the "exit" cumsum value into the row following the last
    /// matmul step (so the AIR's cross-row equation
    /// `nxt.CUMSUM = cur.CUMSUM` is satisfied when the next row is
    /// not itself an active matmul step).
    pub fn set_cumsum_row(
        &mut self,
        row_idx: usize,
        cumsum: &[i32; CUMSUM_LEN],
    ) {
        use p3_field::integers::QuotientMap;
        assert!(row_idx < self.height());
        let base = row_idx * TOTAL_TRACE_WIDTH;
        for k in 0..CUMSUM_LEN {
            self.matrix.values[base + CUMSUM_TILE_START + k] =
                <Val as QuotientMap<i64>>::from_int(cumsum[k] as i64);
        }
    }

    /// Bulk-fill CUMSUM_TILE on rows `[from_row, self.height())`
    /// with `cumsum`. After a matmul-step chain ends at some
    /// intermediate row, the remaining rows are passthrough
    /// (selectors all 0) and the AIR's cross-row equation collapses
    /// to `nxt.CUMSUM = cur.CUMSUM`. So every subsequent row must
    /// hold the same cumsum value.
    ///
    /// `when_transition()` silences the wraparound constraint at
    /// the very last row, so the trace doesn't need to "close the
    /// loop" — the last row's cumsum doesn't have to equal row 0's.
    pub fn fill_cumsum_passthrough(
        &mut self,
        from_row: usize,
        cumsum: &[i32; CUMSUM_LEN],
    ) {
        for r in from_row..self.height() {
            self.set_cumsum_row(r, cumsum);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
    use crate::composite_full_air::CompositeFullAir;
    use crate::composite_layout::MIN_STARK_LEN;
    use crate::params::ZkParams;

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

    #[test]
    fn baseline_trace_has_correct_shape() {
        let trace = CompositeTrace::baseline(MIN_STARK_LEN);
        assert_eq!(trace.height(), MIN_STARK_LEN);
        assert_eq!(trace.width(), TOTAL_TRACE_WIDTH);
        assert_eq!(
            trace.matrix.values.len(),
            MIN_STARK_LEN * TOTAL_TRACE_WIDTH
        );
    }

    #[test]
    fn baseline_min_matches_min_stark_len() {
        let trace = CompositeTrace::baseline_min();
        assert_eq!(trace.height(), MIN_STARK_LEN);
    }

    #[test]
    fn baseline_trace_verifies_through_composite_full_air() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace.matrix, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[])
            .expect("baseline trace must verify");
    }

    #[test]
    #[should_panic(expected = "below MIN_STARK_LEN")]
    fn baseline_panics_below_min_stark_len() {
        let _ = CompositeTrace::baseline(1024);
    }

    #[test]
    #[should_panic(expected = "power of 2")]
    fn baseline_panics_for_non_power_of_two() {
        // 16384 is a power of 2 (above MIN), but 17000 is not.
        let _ = CompositeTrace::baseline(17000);
    }

    #[test]
    fn baseline_larger_than_min_also_verifies() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline(MIN_STARK_LEN * 2);
        assert_eq!(trace.height(), MIN_STARK_LEN * 2);
        let proof = prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace.matrix, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[])
            .expect("2× baseline must verify");
    }

    #[test]
    fn baseline_stark_row_idx_is_monotonic() {
        use p3_field::PrimeField64;
        let trace = CompositeTrace::baseline_min();
        for r in 0..trace.height() {
            let val = trace.matrix.values[r * TOTAL_TRACE_WIDTH + STARK_ROW_IDX];
            assert_eq!(val.as_canonical_u64(), r as u64);
        }
    }

    /// Place 3 matmul instructions starting at row 0, then thread
    /// the final cumsum into row 3 so the cross-row passthrough
    /// constraint (`cur.CUMSUM = nxt.CUMSUM` when both selectors
    /// are 0) holds on the boundary.
    #[test]
    fn matmul_step_chain_verifies_through_composite_full_air() {
        use crate::composite_layout::TILE_D;

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();

        let mut a = [[0i8; TILE_D]; crate::composite_layout::TILE_H];
        let mut b = [[0i8; TILE_D]; crate::composite_layout::TILE_H];
        for d in 0..TILE_D {
            a[0][d] = (d as i8 + 1) % 5;
            a[1][d] = ((d as i8) * 3) % 7 - 3;
            b[0][d] = ((d as i8 + 2) % 6) - 3;
            b[1][d] = ((d as i8 + 3) % 11) - 5;
        }

        // Step 0: reset.
        let zero: [i32; CUMSUM_LEN] = [0; CUMSUM_LEN];
        let after_reset =
            trace.place_matmul_step(0, &a, &b, /*reset*/ true, /*update*/ false, &zero);
        // Step 1: update.
        let after_u1 =
            trace.place_matmul_step(1, &a, &b, false, true, &after_reset);
        // Step 2: update.
        let after_u2 =
            trace.place_matmul_step(2, &a, &b, false, true, &after_u1);
        // Thread the final cumsum across all subsequent passthrough
        // rows. The matmul cross-row constraint silences only at
        // the trace's very last row (via when_transition), so every
        // intermediate row must hold the value the chain ended at.
        trace.fill_cumsum_passthrough(3, &after_u2);

        let proof =
            prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace.matrix, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[])
            .expect("matmul chain must verify through composite_full_air");
    }

    /// Tamper a matmul step's input — the chain breaks because
    /// the cross-row cumsum constraint depends on the dot product.
    #[test]
    fn matmul_step_chain_rejects_tampered_input() {
        use crate::composite_layout::TILE_D;
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        let a = [[1i8; TILE_D]; crate::composite_layout::TILE_H];
        let b = [[1i8; TILE_D]; crate::composite_layout::TILE_H];

        let zero: [i32; CUMSUM_LEN] = [0; CUMSUM_LEN];
        let after_step = trace.place_matmul_step(0, &a, &b, true, false, &zero);
        trace.fill_cumsum_passthrough(1, &after_step);

        // Tamper row 0's A_NOISED_UNPACK[0]: change from 1 to 2.
        // The dot product changes, so the constraint
        // `nxt.CUMSUM = (1+0) * dot + (0) * cur.CUMSUM` rejects.
        use crate::composite_layout::A_NOISED_UNPACK_START;
        use p3_field::integers::QuotientMap;
        let target = 0 * TOTAL_TRACE_WIDTH + A_NOISED_UNPACK_START;
        trace.matrix.values[target] = <Val as QuotientMap<i64>>::from_int(2);

        let proof =
            prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace.matrix, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "tampered matmul input must reject"
        );
    }
}
