//! Algebraic Intermediate Representation (AIR) for the matmul puzzle.
//!
//! Encodes the Pearl §4.5 tile loop and the keyed-BLAKE3 leaf hash into a
//! polynomial constraint system Plonky3 can prove via `p3_uni_stark`.
//!
//! All trace columns and constraint expressions are TBD; this file is a
//! scaffold whose role is to give the wider integration something to
//! reference.

use p3_air::{Air, AirBuilder, BaseAir};
use p3_field::Field;

use crate::params::ZkParams;

/// Width of the AIR trace (number of columns). Replace with the actual
/// column count once trace generation is designed.
pub const TRACE_WIDTH: usize = 0;

/// Plonky3 AIR for the `ai-pow` tile-matmul circuit.
///
/// Carries enough configuration (matmul shape, tile size, noise rank) to
/// pick the right trace dimensions and constraint counts. Built once per
/// `ZkParams`; reused for every prove / verify with those params.
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
        // Number of trace columns. Will encode the running INT8
        // accumulator, the 16-slot `TileState`, intermediate XOR
        // registers for the rotate-13-XOR fold, and selector flags for
        // the per-stripe boundary rows.
        TRACE_WIDTH
    }
}

impl<AB: AirBuilder> Air<AB> for MatmulAir
where
    <AB as AirBuilder>::F: Field,
{
    fn eval(&self, builder: &mut AB) {
        // Constraints to encode (TBD):
        //   1. Per-stripe r-wide INT8 dot product accumulates into `C_blk`.
        //   2. At stripe boundary, X = u32-XOR of all C_blk entries.
        //   3. `M[step mod 16] ← rotate_left(M[..], 13) XOR X`.
        //   4. After the last stripe, the keyed BLAKE3 of `M` is the
        //      `found_leaf` public input.
        //   5. The matrix-commitment chunk-Merkle roots `h_a`, `h_b`
        //      authenticate the `a_rows`, `b_cols` witnesses.
        let _ = builder;
        todo!("encode Pearl §4.5 tile loop + keyed BLAKE3 as AIR constraints")
    }
}
