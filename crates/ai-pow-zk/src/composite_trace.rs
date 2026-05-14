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

use crate::chips::i8u8::I8U8Chip;
use crate::chips::range_table::{IRange7P1Chip, IRange8Chip, URange13Chip, URange8Chip};
use crate::composite_layout::{STARK_ROW_IDX, TOTAL_TRACE_WIDTH};
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
}
