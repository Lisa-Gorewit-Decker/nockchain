//! Top-level AIR composing the four "chips" (sub-AIRs) of the ai-pow puzzle.
//!
//! Logical layout mirrors Pearl's `pearl_air.rs` (see
//! `pearl/zk-pow/src/circuit/pearl_air.rs:1-37`) but built on Plonky3
//! over Goldilocks. Each trace row carries columns from four sub-AIRs
//! interleaved together, with preprocessed control bits picking which
//! sub-AIR is "live" on that row:
//!
//! 1. **Input chip** — range-checks the i7 / i6 / i8 values fed in
//!    from the `Witness` (matrix strips, noise factors).
//! 2. **BLAKE3 chip** — runs the [`crate::blake3_air`] sub-AIR for
//!    every keyed BLAKE3 the protocol uses: kappa, s_A, s_B, pow_key,
//!    matrix-commitment chunk roots, jackpot hash. Pearl uses one big
//!    BLAKE3 sub-AIR for the same role (`pearl_program.rs`).
//! 3. **Matmul chip** — per-stripe r-wide INT8 dot product into the
//!    `C_blk` running accumulator (`tile × tile` of i32).
//! 4. **Jackpot chip** — int32-XOR fold of `C_blk` entries and the
//!    `rotate_left(13) ^ X` update of the 16-slot `M` state (Pearl §4.5
//!    Alg. 4).
//!
//! Constraints currently `todo!()` — see `DESIGN.md` for the row-by-row
//! plan and trace-column layout.

use p3_air::{Air, AirBuilder, BaseAir};
use p3_field::Field;

use crate::params::ZkParams;

/// Width of the combined trace (sum of all four sub-AIR widths).
/// Replace with the actual column count when the column layout is
/// finalized — see `DESIGN.md` "Trace column layout".
pub const TRACE_WIDTH: usize = 0;

/// Top-level AIR for the ai-pow tile-matmul circuit, composed of four
/// chips interleaved per row. Built once per [`ZkParams`].
#[derive(Debug, Clone)]
pub struct MatmulAir {
    pub params: ZkParams,
}

impl MatmulAir {
    pub fn new(params: ZkParams) -> Self {
        Self { params }
    }
}

impl<F: Field> BaseAir<F> for MatmulAir {
    fn width(&self) -> usize {
        TRACE_WIDTH
    }
}

impl<AB: AirBuilder> Air<AB> for MatmulAir
where
    <AB as AirBuilder>::F: Field,
{
    fn eval(&self, builder: &mut AB) {
        // Per-chip evaluation (TBD — see `DESIGN.md`):
        //
        //   1. Range tables (u8 / u13 / i7+1 / i8 / i8↔u8) gate the
        //      witness columns into their declared ranges.
        //   2. `blake3_air::eval_constraints(builder, row_view)` for
        //      BLAKE3-active rows.
        //   3. Matmul: enforce
        //          C_blk[step+1] = C_blk[step] + Σ_{l=lo..hi} A'[i,l]·B'[l,j]
        //      on stripe-boundary rows.
        //   4. Jackpot: enforce
        //          X = XOR_{e ∈ C_blk} e         (u32 XOR via bit-decomp)
        //          M_new[step mod 16]
        //              = rotate_left_13(M_old[step mod 16]) ⊕ X
        //
        // Public-input binding (Pearl §4.5 line 16 analog):
        //   final BLAKE3(M, key = s_A) == public input `found_leaf`.
        //
        // Matrix-commitment binding (Pearl §4.3):
        //   BLAKE3(pad(a_rows), key=kappa) == public input `h_a`
        //   BLAKE3(pad(b_cols), key=kappa) == public input `h_b`
        //   (where `kappa` derives from public inputs and BLAKE3-chip
        //    rows compute the chain.)
        let _ = builder;
        todo!("encode the four-chip interleaved constraints")
    }
}
