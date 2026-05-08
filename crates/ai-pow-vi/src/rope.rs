//! Rotary positional embedding (RoPE) over INT16 fixed-point cos/sin tables.
//!
//! For each token position `pos` and each pair index `j ∈ [0, head_dim/2)`,
//! RoPE rotates the pair `(x[2j], x[2j+1])` by an angle `θ_j(pos)`. The
//! standard formula is
//!
//! ```text
//! θ_j(pos) = pos * base^(-2j / head_dim)         (base typically 10_000)
//! x_new[2j]     = x[2j]   * cos θ - x[2j+1] * sin θ
//! x_new[2j+1]   = x[2j]   * sin θ + x[2j+1] * cos θ
//! ```
//!
//! For determinism the cos / sin tables are pre-computed once per model and
//! committed alongside the weights. INT16 fixed-point (denominator `2^14`)
//! gives ~14 bits of angular precision, far above INT8 quantization noise.
//!
//! Layout of the tables (row-major):
//!   `cos[pos * half_head_dim + j]`, `sin[pos * half_head_dim + j]`.

use blake3::Hasher;
use thiserror::Error;

use crate::quant::{round_half_to_even_div_pow2, saturate_i8};

const CTX_ROPE_TABLES: &str = "ai-pow-vi v1 rope-tables";

/// Number of fractional bits in the INT16 cos/sin representation. With
/// `FRACT_BITS = 14`, values in `[-1, 1]` map to `i16 ∈ [-16384, 16384]`.
pub const FRACT_BITS: u32 = 14;

/// Pre-computed RoPE tables. Bytes are committed inside `comm_W`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RopeTables {
    pub seq_len: u32,
    pub half_head_dim: u32,
    pub cos: Vec<i16>,
    pub sin: Vec<i16>,
}

impl RopeTables {
    /// Allocate empty tables of the right shape; entries are zero. Useful
    /// for tests.
    pub fn zeros(seq_len: u32, half_head_dim: u32) -> Self {
        let n = (seq_len as usize) * (half_head_dim as usize);
        Self {
            seq_len,
            half_head_dim,
            cos: vec![0i16; n],
            sin: vec![0i16; n],
        }
    }

    /// Build tables that represent the identity rotation (cos = 1, sin = 0)
    /// at every position. Useful for tests that want to compose RoPE into
    /// a layer without exercising the rotation itself.
    pub fn identity(seq_len: u32, half_head_dim: u32) -> Self {
        let n = (seq_len as usize) * (half_head_dim as usize);
        Self {
            seq_len,
            half_head_dim,
            cos: vec![1i16 << FRACT_BITS; n],
            sin: vec![0i16; n],
        }
    }

    pub fn validate(&self) -> Result<(), RopeError> {
        let n = (self.seq_len as usize) * (self.half_head_dim as usize);
        if self.cos.len() != n || self.sin.len() != n {
            return Err(RopeError::ShapeMismatch);
        }
        Ok(())
    }

    /// Cos / sin for one `(position, pair)` slot.
    #[inline]
    pub fn lookup(&self, pos: u32, j: u32) -> (i16, i16) {
        debug_assert!(pos < self.seq_len);
        debug_assert!(j < self.half_head_dim);
        let off = (pos as usize) * (self.half_head_dim as usize) + j as usize;
        (self.cos[off], self.sin[off])
    }

    /// Domain-separated commitment to the table bytes. Cos and sin are
    /// concatenated (cos first), then hashed. Committed inside `comm_W`.
    pub fn commit(&self) -> [u8; 32] {
        let mut hasher = Hasher::new_derive_key(CTX_ROPE_TABLES);
        hasher.update(&self.seq_len.to_le_bytes());
        hasher.update(&self.half_head_dim.to_le_bytes());
        for &v in self.cos.iter() {
            hasher.update(&v.to_le_bytes());
        }
        for &v in self.sin.iter() {
            hasher.update(&v.to_le_bytes());
        }
        *hasher.finalize().as_bytes()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RopeError {
    #[error("vector length must equal `2 * half_head_dim`")]
    LenMismatch,
    #[error("position out of range")]
    PosOutOfRange,
    #[error("cos / sin tables shape does not match seq_len * half_head_dim")]
    ShapeMismatch,
}

/// Apply RoPE in place to `x`, length `2 * tables.half_head_dim`, at the
/// given `pos`. The rotation is bit-exact in INT16 fixed-point with
/// banker's rounding on the `>> FRACT_BITS` step.
pub fn rope_apply(x: &mut [i8], pos: u32, tables: &RopeTables) -> Result<(), RopeError> {
    if pos >= tables.seq_len {
        return Err(RopeError::PosOutOfRange);
    }
    let n = 2 * tables.half_head_dim as usize;
    if x.len() != n {
        return Err(RopeError::LenMismatch);
    }
    for j in 0..tables.half_head_dim {
        let (cos, sin) = tables.lookup(pos, j);
        let i = (j * 2) as usize;
        let x0 = x[i] as i64;
        let x1 = x[i + 1] as i64;
        let cos = cos as i64;
        let sin = sin as i64;
        // (x0*cos - x1*sin) / 2^14, ties to even.
        let r0 = round_half_to_even_div_pow2(x0 * cos - x1 * sin, FRACT_BITS);
        let r1 = round_half_to_even_div_pow2(x0 * sin + x1 * cos, FRACT_BITS);
        x[i] = saturate_i8(r0);
        x[i + 1] = saturate_i8(r1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_tables_are_no_op() {
        let tables = RopeTables::identity(4, 8);
        let mut x: Vec<i8> = (0..16i8).map(|v| v - 8).collect();
        let original = x.clone();
        rope_apply(&mut x, 2, &tables).unwrap();
        assert_eq!(x, original, "identity rotation must not modify input");
    }

    #[test]
    fn ninety_degree_rotation_swaps_pair() {
        // Tables: cos = 0, sin = 1 → (x0, x1) -> (-x1, x0).
        let mut tables = RopeTables::zeros(2, 4);
        for slot in tables.cos.iter_mut() {
            *slot = 0;
        }
        for slot in tables.sin.iter_mut() {
            *slot = 1 << FRACT_BITS;
        }
        let mut x: Vec<i8> = vec![10, 20, -30, 40, 0, 7, -1, 1];
        let mut expected = vec![0i8; 8];
        for j in 0..4 {
            expected[j * 2] = -x[j * 2 + 1];
            expected[j * 2 + 1] = x[j * 2];
        }
        rope_apply(&mut x, 0, &tables).unwrap();
        assert_eq!(x, expected);
    }

    #[test]
    fn one_eighty_rotation_negates_pair() {
        // cos = -1, sin = 0 → (x0, x1) -> (-x0, -x1).
        let mut tables = RopeTables::zeros(2, 4);
        for slot in tables.cos.iter_mut() {
            *slot = -(1 << FRACT_BITS);
        }
        let mut x: Vec<i8> = vec![10, 20, -30, 40, 0, 7, -1, 1];
        let expected: Vec<i8> = x.iter().map(|v| -v).collect();
        rope_apply(&mut x, 0, &tables).unwrap();
        assert_eq!(x, expected);
    }

    #[test]
    fn rejects_position_out_of_range() {
        let tables = RopeTables::identity(4, 4);
        let mut x = vec![0i8; 8];
        assert_eq!(
            rope_apply(&mut x, 4, &tables).err(),
            Some(RopeError::PosOutOfRange),
        );
    }

    #[test]
    fn rejects_length_mismatch() {
        let tables = RopeTables::identity(4, 4);
        let mut x = vec![0i8; 7];
        assert_eq!(
            rope_apply(&mut x, 0, &tables).err(),
            Some(RopeError::LenMismatch),
        );
    }

    #[test]
    fn validate_catches_shape_mismatch() {
        let mut tables = RopeTables::identity(4, 4);
        tables.cos.pop();
        assert_eq!(tables.validate().err(), Some(RopeError::ShapeMismatch));
    }

    #[test]
    fn commit_is_position_and_table_sensitive() {
        let a = RopeTables::identity(4, 4);
        let b = RopeTables::identity(4, 8); // different shape
        assert_ne!(a.commit(), b.commit());
        let mut c = RopeTables::identity(4, 4);
        c.cos[0] ^= 1;
        assert_ne!(a.commit(), c.commit());
    }

    #[test]
    fn determinism_same_inputs_same_outputs() {
        // Hand-built tables with a few fixed angles.
        let mut tables = RopeTables::zeros(2, 4);
        for j in 0..4 {
            // Position 0: identity. Position 1: rotation by some angle.
            tables.cos[j] = 1 << FRACT_BITS;
            tables.sin[j] = 0;
            tables.cos[4 + j] = 12345;
            tables.sin[4 + j] = -6789;
        }
        let x_seed: Vec<i8> = (0..8i8).map(|v| (v * 13 - 50) as i8).collect();
        let mut a = x_seed.clone();
        let mut b = x_seed.clone();
        rope_apply(&mut a, 1, &tables).unwrap();
        rope_apply(&mut b, 1, &tables).unwrap();
        assert_eq!(a, b);
    }
}
