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

use crate::chips::blake3::chip::pack_tweak;
use crate::chips::blake3::compress::{
    blake3_permute_msg, compress_full_state, round_with_snapshots, Blake3Tweak, BLAKE3_IV,
};
use crate::chips::blake3::layout::LIMBS_PER_STATE_SNAPSHOT;
use crate::chips::control::ControlChip;
use crate::chips::i8u8::I8U8Chip;
use crate::chips::matmul::compute::{compute_row, CUMSUM_LEN};
use crate::chips::range_table::{IRange7P1Chip, IRange8Chip, URange13Chip, URange8Chip};
use crate::composite_layout::{
    A_NOISED_UNPACK_START, BLAKE3_CV_START, BLAKE3_MSG_START, BLAKE3_ROUND_START,
    B_NOISED_UNPACK_START, CUMSUM_TILE_START, CV_OR_TWEAK_PREP, CV_OUT_START, STARK_ROW_IDX,
    TILE_D, TILE_H, TOTAL_TRACE_WIDTH,
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

    /// Place an 8-row BLAKE3 hash compression block starting at
    /// `row_start`. Writes BLAKE3_ROUND (4 state snapshots),
    /// BLAKE3_MSG, BLAKE3_CV, CV_OR_TWEAK_PREP, CV_OUT, and the
    /// IS_NEW_BLAKE / IS_LAST_ROUND selectors at composite-layout
    /// offsets.
    ///
    /// `row_start + 8` must be ≤ `height()`. Returns the BLAKE3
    /// output CV (8 packed u32s) so the caller can chain it into
    /// downstream usage (e.g. the next hash's CV_IN).
    pub fn place_blake3_hash(
        &mut self,
        row_start: usize,
        message: &[u32; 16],
        cv_in: &[u32; 8],
        tweak: &Blake3Tweak,
    ) -> [u32; 8] {
        use p3_field::integers::QuotientMap;

        assert!(
            row_start + 8 <= self.height(),
            "BLAKE3 block needs 8 rows; row_start={row_start}, height={}",
            self.height()
        );

        let tweak_packed = pack_tweak(tweak);

        // Run BLAKE3 once to get the final CV_OUT for the finalize
        // row.
        let full_state = compress_full_state(
            cv_in,
            message,
            tweak.counter_low,
            tweak.counter_high as u32,
            tweak.block_len,
            tweak.flags,
        );
        let cv_out: [u32; 8] = core::array::from_fn(|i| full_state[i]);

        // Per-round permuted messages.
        let mut round_msgs: Vec<[u32; 16]> = Vec::with_capacity(7);
        let mut cur_msg = *message;
        round_msgs.push(cur_msg);
        for _ in 1..7 {
            blake3_permute_msg(&mut cur_msg);
            round_msgs.push(cur_msg);
        }

        // Initial state at the start of hash: cv ++ IV[0..4] ++ tweak words.
        let mut state = [0u32; 16];
        for i in 0..8 {
            state[i] = cv_in[i];
        }
        for i in 0..4 {
            state[8 + i] = BLAKE3_IV[i];
        }
        state[12] = tweak.counter_low;
        state[13] = tweak.counter_high as u32;
        state[14] = tweak.block_len;
        state[15] = tweak.flags;

        // Compute snapshots for the 7 mixing rounds + the
        // finalize row.
        let mut current_input_state = state;

        // Helper that writes a 16-word state into the row's 264
        // BLAKE3_ROUND cells for a specific snapshot slot.
        fn write_state(row: &mut [Val], snapshot_offset: usize, state: &[u32; 16]) {
            let dest = &mut row[snapshot_offset..snapshot_offset + LIMBS_PER_STATE_SNAPSHOT];
            let mut off = 0;
            for i in 0..4 {
                dest[off] = <Val as QuotientMap<u64>>::from_int(state[i] as u64);
                off += 1;
            }
            for i in 4..8 {
                for bit in 0..32 {
                    dest[off] =
                        <Val as QuotientMap<u64>>::from_int(((state[i] >> bit) & 1) as u64);
                    off += 1;
                }
            }
            for i in 8..12 {
                dest[off] = <Val as QuotientMap<u64>>::from_int(state[i] as u64);
                off += 1;
            }
            for i in 12..16 {
                for bit in 0..32 {
                    dest[off] =
                        <Val as QuotientMap<u64>>::from_int(((state[i] >> bit) & 1) as u64);
                    off += 1;
                }
            }
            debug_assert_eq!(off, LIMBS_PER_STATE_SNAPSHOT);
        }

        // For each of the 7 mixing-round rows, run round_with_snapshots
        // and place the 4 snapshots at BLAKE3_ROUND_START + i * STATE_W.
        for r in 0..7 {
            let row_idx = row_start + r;
            let base = row_idx * TOTAL_TRACE_WIDTH;
            let row = &mut self.matrix.values[base..base + TOTAL_TRACE_WIDTH];

            let mut s = current_input_state;
            let snaps = round_with_snapshots(&mut s, &round_msgs[r]);

            write_state(row, BLAKE3_ROUND_START, &current_input_state);
            write_state(
                row,
                BLAKE3_ROUND_START + LIMBS_PER_STATE_SNAPSHOT,
                &snaps[0],
            );
            write_state(
                row,
                BLAKE3_ROUND_START + 2 * LIMBS_PER_STATE_SNAPSHOT,
                &snaps[1],
            );
            write_state(
                row,
                BLAKE3_ROUND_START + 3 * LIMBS_PER_STATE_SNAPSHOT,
                &snaps[2],
            );

            // BLAKE3_MSG (this row's permuted message).
            for i in 0..16 {
                row[BLAKE3_MSG_START + i] =
                    <Val as QuotientMap<u64>>::from_int(round_msgs[r][i] as u64);
            }
            // BLAKE3_CV (replicated across all 8 rows).
            for i in 0..8 {
                row[BLAKE3_CV_START + i] =
                    <Val as QuotientMap<u64>>::from_int(cv_in[i] as u64);
            }
            // CV_OR_TWEAK_PREP.
            row[CV_OR_TWEAK_PREP] = <Val as QuotientMap<u64>>::from_int(tweak_packed);

            // Selectors: IS_NEW_BLAKE on row 0.
            let mut selectors = [false; 21];
            if r == 0 {
                selectors[8] = true; // IS_NEW_BLAKE index in SELECTOR_COLS
            }
            ControlChip.fill_row(&selectors, 0, row);

            current_input_state = snaps[3];
        }

        // Finalize row (row_start + 7). STATE0 = final round output,
        // STATE1 encoded so finalize_blake's "abuse" packing works.
        let final_input = current_input_state;
        let mut state1_for_finalize = [0u32; 16];
        // row1 cells (state[0..4]) — free, set to final_input for cleanness.
        state1_for_finalize[0] = final_input[0];
        state1_for_finalize[1] = final_input[1];
        state1_for_finalize[2] = final_input[2];
        state1_for_finalize[3] = final_input[3];
        // row2 bit-decomp slots (state[4..8]) — bits of final_input[0..4].
        state1_for_finalize[4] = final_input[0];
        state1_for_finalize[5] = final_input[1];
        state1_for_finalize[6] = final_input[2];
        state1_for_finalize[7] = final_input[3];
        // row3 cells (state[8..12]) — free.
        state1_for_finalize[8] = final_input[8];
        state1_for_finalize[9] = final_input[9];
        state1_for_finalize[10] = final_input[10];
        state1_for_finalize[11] = final_input[11];
        // row4 bit-decomp slots (state[12..16]) — bits of final_input[8..12].
        state1_for_finalize[12] = final_input[8];
        state1_for_finalize[13] = final_input[9];
        state1_for_finalize[14] = final_input[10];
        state1_for_finalize[15] = final_input[11];

        let row_idx = row_start + 7;
        let base = row_idx * TOTAL_TRACE_WIDTH;
        let row = &mut self.matrix.values[base..base + TOTAL_TRACE_WIDTH];

        write_state(row, BLAKE3_ROUND_START, &final_input);
        write_state(
            row,
            BLAKE3_ROUND_START + LIMBS_PER_STATE_SNAPSHOT,
            &state1_for_finalize,
        );
        // STATE2 and STATE3 stay zero (the chip's eval doesn't
        // constrain them on the finalize row).

        // Last-permuted message + CV + tweak.
        let last_msg = round_msgs[6];
        for i in 0..16 {
            row[BLAKE3_MSG_START + i] =
                <Val as QuotientMap<u64>>::from_int(last_msg[i] as u64);
        }
        for i in 0..8 {
            row[BLAKE3_CV_START + i] =
                <Val as QuotientMap<u64>>::from_int(cv_in[i] as u64);
        }
        row[CV_OR_TWEAK_PREP] = <Val as QuotientMap<u64>>::from_int(tweak_packed);

        // CV_OUT cells (only meaningful on the finalize row).
        for i in 0..8 {
            row[CV_OUT_START + i] = <Val as QuotientMap<u64>>::from_int(cv_out[i] as u64);
        }

        // Selectors: IS_LAST_ROUND on row 7.
        let mut selectors = [false; 21];
        selectors[9] = true; // IS_LAST_ROUND index in SELECTOR_COLS
        ControlChip.fill_row(&selectors, 0, row);

        cv_out
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

    /// Place one BLAKE3 hash at row 0 of the baseline trace; the
    /// composite AIR should still verify (the BLAKE3 chip's
    /// round / init / finalize constraints all fire correctly on
    /// the 8-row block, while the remaining 8184 baseline rows
    /// have all-zero BLAKE3 columns).
    #[test]
    fn blake3_hash_block_at_row_0_verifies() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();

        let cv: [u32; 8] = core::array::from_fn(|i| BLAKE3_IV[i]);
        let msg: [u32; 16] = core::array::from_fn(|i| (i as u32 + 1) * 0x01020304);
        let tweak = Blake3Tweak {
            counter_low: 0,
            counter_high: 0,
            block_len: 64,
            flags: 0x1B,
        };

        let cv_out = trace.place_blake3_hash(0, &msg, &cv, &tweak);
        // Sanity: the returned cv_out matches a fresh BLAKE3 run.
        let full = compress_full_state(
            &cv,
            &msg,
            tweak.counter_low,
            tweak.counter_high as u32,
            tweak.block_len,
            tweak.flags,
        );
        for i in 0..8 {
            assert_eq!(cv_out[i], full[i]);
        }

        let proof =
            prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace.matrix, &[]);
        verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[])
            .expect("BLAKE3 hash block must verify through composite_full_air");
    }

    /// Tamper the BLAKE3 hash block's CV_OUT — the finalize
    /// constraint rejects.
    #[test]
    fn blake3_hash_block_rejects_tampered_cv_out() {
        use crate::composite_layout::CV_OUT_START;
        use p3_field::integers::QuotientMap;

        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();

        let cv: [u32; 8] = core::array::from_fn(|i| BLAKE3_IV[i]);
        let msg: [u32; 16] = core::array::from_fn(|i| (i as u32 + 1) * 0x01020304);
        let tweak = Blake3Tweak {
            counter_low: 0,
            counter_high: 0,
            block_len: 64,
            flags: 0x1B,
        };

        let _ = trace.place_blake3_hash(0, &msg, &cv, &tweak);
        // Tamper row 7's CV_OUT[0].
        let target = 7 * TOTAL_TRACE_WIDTH + CV_OUT_START;
        trace.matrix.values[target] =
            <Val as QuotientMap<u64>>::from_int(0xDEAD_BEEFu64);

        let proof =
            prove::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, trace.matrix, &[]);
        assert!(
            verify::<AiPowStarkConfig, _>(&cfg, &CompositeFullAir, &proof, &[]).is_err(),
            "tampered CV_OUT must reject"
        );
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
