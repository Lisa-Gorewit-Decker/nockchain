//! Public inputs for the M10.1c composite proof.
//!
//! Pins the cells the composite AIR exposes externally. The
//! verifier checks these against the prover's claim via the
//! [`CompositeFullAir`] / [`CompositeFullAirWithLookups`]
//! constraints on the trace's last row.
//!
//! ## Layout (20 field elements)
//!
//! ```text
//!   index 0..4   : final CUMSUM_TILE (4 i32 cells, signed —
//!                  encoded canonically into Goldilocks; the
//!                  trace generator's `fill_cumsum_passthrough`
//!                  guarantees every row from the chain end
//!                  through the last row holds this value)
//!   index 4..20  : final JACKPOT_MSG (16 u32 cells, threaded
//!                  by `fill_jackpot_passthrough`)
//! ```
//!
//! ## Deferred from PIs
//!
//! - **Final CV_OUT.** Per-row only meaningful on finalize rows
//!   (where `IS_LAST_ROUND = 1`). Without a trace-side mechanism
//!   to thread the "current" CV through to the last row, binding
//!   it as a PI would always read zero on most traces. Add when a
//!   downstream protocol needs the final hash output.
//! - **`h_a` / `h_b` matrix commitments.** Task #52, multi-week.

use crate::composite_layout::{CUMSUM_TILE_START, JACKPOT_MSG_START, JACKPOT_SIZE, TOTAL_TRACE_WIDTH};
use crate::composite_trace::CompositeTrace;
use crate::Val;

use p3_field::integers::QuotientMap;
use p3_field::PrimeField64;
use serde::{Deserialize, Serialize};

/// Number of field elements in the public-input vector.
pub const NUM_PUBLIC_VALUES: usize = 4 + JACKPOT_SIZE; // 20

/// PI layout offsets (within the `Vec<Val>` of length
/// [`NUM_PUBLIC_VALUES`]).
pub const PI_CUMSUM_OFFSET: usize = 0;
pub const PI_CUMSUM_LEN: usize = 4; // TILE_H × TILE_H
pub const PI_JACKPOT_OFFSET: usize = PI_CUMSUM_OFFSET + PI_CUMSUM_LEN;
pub const PI_JACKPOT_LEN: usize = JACKPOT_SIZE;

/// Typed view of the public inputs the composite AIR commits to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompositePublicInputs {
    /// Final value of `CUMSUM_TILE[0..4]` (signed 32-bit ints).
    pub cumsum: [i32; PI_CUMSUM_LEN],
    /// Final value of `JACKPOT_MSG[0..16]` (32-bit values).
    pub jackpot: [u32; JACKPOT_SIZE],
}

impl Default for CompositePublicInputs {
    fn default() -> Self {
        Self::zero()
    }
}

impl CompositePublicInputs {
    /// All-zero PIs. Matches a baseline trace (no chip activity).
    pub const fn zero() -> Self {
        Self {
            cumsum: [0; PI_CUMSUM_LEN],
            jackpot: [0; JACKPOT_SIZE],
        }
    }

    /// Serialise to the `Vec<Val>` shape `prove_batch` /
    /// `verify_batch` expect.
    pub fn to_vec(&self) -> Vec<Val> {
        let mut out = Vec::with_capacity(NUM_PUBLIC_VALUES);
        for &v in &self.cumsum {
            out.push(<Val as QuotientMap<i64>>::from_int(v as i64));
        }
        for &v in &self.jackpot {
            out.push(<Val as QuotientMap<u64>>::from_int(v as u64));
        }
        debug_assert_eq!(out.len(), NUM_PUBLIC_VALUES);
        out
    }

    /// Read the last row of a `CompositeTrace` and derive the PI
    /// values it would commit to. After
    /// `fill_cumsum_passthrough` and `fill_jackpot_passthrough`,
    /// the last row holds the threaded final state — exactly the
    /// values the verifier checks against.
    pub fn derive_from_trace(trace: &CompositeTrace) -> Self {
        Self::derive_from_matrix(&trace.matrix)
    }

    /// Variant for callers that hold the trace as a
    /// `RowMajorMatrix<Val>` directly (e.g. test code in
    /// `composite_full_air` and `composite_trace`).
    pub fn derive_from_matrix(matrix: &p3_matrix::dense::RowMajorMatrix<Val>) -> Self {
        let n = matrix.values.len() / TOTAL_TRACE_WIDTH;
        let last_base = (n - 1) * TOTAL_TRACE_WIDTH;

        let cumsum: [i32; PI_CUMSUM_LEN] = core::array::from_fn(|i| {
            let raw = matrix.values[last_base + CUMSUM_TILE_START + i].as_canonical_u64();
            goldilocks_to_i32(raw)
        });
        let jackpot: [u32; JACKPOT_SIZE] = core::array::from_fn(|i| {
            matrix.values[last_base + JACKPOT_MSG_START + i].as_canonical_u64() as u32
        });

        Self { cumsum, jackpot }
    }
}

/// Map a Goldilocks raw value back to i32 (preserving the two's-
/// complement representation that `from_int(i32)` uses). Used
/// when extracting signed cumsum cells.
fn goldilocks_to_i32(raw: u64) -> i32 {
    const GOLDILOCKS_P: u64 = 0xFFFF_FFFF_0000_0001;
    if raw > GOLDILOCKS_P / 2 {
        let signed = raw as i128 - GOLDILOCKS_P as i128;
        signed as i32
    } else {
        raw as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_round_trips() {
        let pis = CompositePublicInputs::zero();
        let v = pis.to_vec();
        assert_eq!(v.len(), NUM_PUBLIC_VALUES);
        for x in v {
            assert_eq!(x, Val::default());
        }
    }

    #[test]
    fn derive_from_baseline_trace_is_zero() {
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_eq!(pis, CompositePublicInputs::zero());
    }

    #[test]
    fn derive_picks_up_threaded_cumsum() {
        let mut trace = CompositeTrace::baseline_min();
        let cumsum = [1, -2, 3, -4];
        trace.fill_cumsum_passthrough(0, &cumsum);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_eq!(pis.cumsum, cumsum);
        assert_eq!(pis.jackpot, [0u32; JACKPOT_SIZE]);
    }

    #[test]
    fn derive_picks_up_threaded_jackpot() {
        let mut trace = CompositeTrace::baseline_min();
        let jp: [u32; JACKPOT_SIZE] = core::array::from_fn(|i| (i as u32 + 1) * 0xCAFE);
        trace.fill_jackpot_passthrough(0, &jp);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_eq!(pis.cumsum, [0; 4]);
        assert_eq!(pis.jackpot, jp);
    }

    #[test]
    fn to_vec_layout_is_stable() {
        let pis = CompositePublicInputs {
            cumsum: [10, 20, 30, 40],
            jackpot: core::array::from_fn(|i| 100 + i as u32),
        };
        let v = pis.to_vec();
        use p3_field::PrimeField64;
        for i in 0..PI_CUMSUM_LEN {
            assert_eq!(
                v[PI_CUMSUM_OFFSET + i].as_canonical_u64(),
                pis.cumsum[i] as u64
            );
        }
        for i in 0..PI_JACKPOT_LEN {
            assert_eq!(
                v[PI_JACKPOT_OFFSET + i].as_canonical_u64(),
                pis.jackpot[i] as u64
            );
        }
    }

    #[test]
    fn negative_cumsum_round_trips() {
        let pis = CompositePublicInputs {
            cumsum: [-1, -1000, i32::MIN, i32::MAX],
            jackpot: [0; JACKPOT_SIZE],
        };
        let v = pis.to_vec();
        use p3_field::PrimeField64;
        // -1 in Goldilocks = p - 1.
        assert_eq!(
            v[0].as_canonical_u64(),
            0xFFFF_FFFF_0000_0001u64 - 1
        );
        // Round-trip via goldilocks_to_i32.
        assert_eq!(goldilocks_to_i32(v[0].as_canonical_u64()), -1);
        assert_eq!(goldilocks_to_i32(v[1].as_canonical_u64()), -1000);
        assert_eq!(goldilocks_to_i32(v[2].as_canonical_u64()), i32::MIN);
        assert_eq!(goldilocks_to_i32(v[3].as_canonical_u64()), i32::MAX);
    }
}
