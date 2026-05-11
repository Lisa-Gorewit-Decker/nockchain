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

/// Build standard RoPE tables (NEOX pairing — pair `j` is `(x[j], x[j+half])`)
/// for the first `n_rot` of `head_dim`. The dims past `n_rot` are unrotated.
/// `theta_per_pair[j] = base^(-2j / n_rot)`.
pub fn build_rope_tables(seq_len: u32, head_dim: u32, rope_theta: f32) -> RopeTables {
    build_imrope_tables(seq_len, head_dim, head_dim, [0; 4], rope_theta)
}

/// Build IMROPE (interleaved multi-axis RoPE) cos/sin tables matching the
/// validated f32 reference in `bin/calibrate.rs::apply_imrope_f32`.
///
/// For text-only inputs every "axis position" equals the token index `t`,
/// so the per-pair theta for token `t`, pair index `j` reduces to
/// `t * base^(-2j / n_rot)`. We still preserve the sector dispatch from
/// llama.cpp's ggml-cpu ops so multi-axis inputs would work identically.
/// `sections = [t_sec, h_sec, w_sec, e_sec]` partitions the rotated halves.
/// Pairs with index `[n_rot/2, head_dim/2)` are unrotated (cos=1, sin=0)
/// — caller must respect `n_rot` and skip rotating the tail dims.
///
/// Returned table dims: `seq_len × (head_dim / 2)`. The first `n_rot / 2`
/// columns carry the real IMROPE angles; the rest carry identity.
pub fn build_imrope_tables(
    seq_len: u32,
    _head_dim: u32,
    n_rot: u32,
    sections: [usize; 4],
    rope_theta: f32,
) -> RopeTables {
    let seq_len_us = seq_len as usize;
    // Allocate tables sized for the rotated subspace only (n_rot/2 pairs).
    // This makes `tables.half_head_dim = n_rot/2`, so existing callers that
    // pass a slice of length `2 * tables.half_head_dim` to `rope_apply`
    // automatically rotate exactly the first `n_rot` dims and leave the
    // tail untouched — no IMROPE-specific call site needed.
    let half_head_dim = (n_rot as usize) / 2;
    let half_rot = half_head_dim;
    let mut cos = vec![1i16 << FRACT_BITS; seq_len_us * half_head_dim];
    let mut sin = vec![0i16; seq_len_us * half_head_dim];

    let scale = (1i64 << FRACT_BITS) as f64;
    let theta_scale = if n_rot > 0 {
        (rope_theta as f64).powf(-2.0 / n_rot as f64)
    } else {
        1.0
    };
    let sect_dims: usize = sections[0] + sections[1] + sections[2] + sections[3];

    for t in 0..seq_len_us {
        // For text-only inference all axes share the same position t.
        let mut tt_p = t as f64;
        let mut th_p = t as f64;
        let mut tw_p = t as f64;
        let mut te_p = t as f64;
        for j in 0..half_rot {
            let theta = if sect_dims > 0 {
                let sector = j % sect_dims;
                if sector % 3 == 0 && sector < 3 * sections[0] {
                    tt_p
                } else if sector % 3 == 1 && sector < 3 * sections[1] {
                    th_p
                } else if sector % 3 == 2 && sector < 3 * sections[2] {
                    tw_p
                } else {
                    te_p
                }
            } else {
                tt_p
            };
            let c = theta.cos();
            let s = theta.sin();
            let off = t * half_head_dim + j;
            cos[off] = (c * scale).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            sin[off] = (s * scale).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            tt_p *= theta_scale;
            th_p *= theta_scale;
            tw_p *= theta_scale;
            te_p *= theta_scale;
        }
    }
    RopeTables {
        seq_len,
        half_head_dim: half_head_dim as u32,
        cos,
        sin,
    }
}

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

/// Apply IMROPE in place to `x` (one head, length `head_dim`) using NEOX
/// pair layout `(x[j], x[j + n_rot/2])`. Only the first `n_rot` dims are
/// rotated; the rest pass through. Matches the f32 reference in
/// `bin/calibrate.rs::apply_imrope_f32` modulo INT16 fixed-point rounding.
pub fn apply_imrope_to_head_i8(
    x: &mut [i8],
    pos: u32,
    n_rot: u32,
    tables: &RopeTables,
) -> Result<(), RopeError> {
    if pos >= tables.seq_len {
        return Err(RopeError::PosOutOfRange);
    }
    let half = (n_rot as usize) / 2;
    let head_dim = 2 * tables.half_head_dim as usize;
    if x.len() < head_dim {
        return Err(RopeError::LenMismatch);
    }
    if (n_rot as usize) > head_dim {
        return Err(RopeError::ShapeMismatch);
    }
    let pos_us = pos as usize;
    let half_head_dim = tables.half_head_dim as usize;
    for j in 0..half {
        let off = pos_us * half_head_dim + j;
        let cos = tables.cos[off] as i64;
        let sin = tables.sin[off] as i64;
        let a = x[j] as i64;
        let b = x[j + half] as i64;
        let r0 = round_half_to_even_div_pow2(a * cos - b * sin, FRACT_BITS);
        let r1 = round_half_to_even_div_pow2(a * sin + b * cos, FRACT_BITS);
        x[j] = saturate_i8(r0);
        x[j + half] = saturate_i8(r1);
    }
    Ok(())
}

/// Apply IMROPE to a `(m, num_heads, head_dim)` row-major i8 buffer in place.
pub fn apply_imrope_to_qk_i8(
    x: &mut [i8],
    m: u32,
    num_heads: u32,
    head_dim: u32,
    n_rot: u32,
    tables: &RopeTables,
) -> Result<(), RopeError> {
    let mu = m as usize;
    let nh = num_heads as usize;
    let hd = head_dim as usize;
    if x.len() != mu * nh * hd {
        return Err(RopeError::LenMismatch);
    }
    for t in 0..mu {
        for h in 0..nh {
            let off = (t * nh + h) * hd;
            apply_imrope_to_head_i8(&mut x[off..off + hd], t as u32, n_rot, tables)?;
        }
    }
    Ok(())
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
