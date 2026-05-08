//! INT8 quantization spec — the determinism contract.
//!
//! Every numerical op the puzzle relies on must obey this spec. Vendor
//! kernels (cuBLAS-INT8, oneDNN, MLX, etc.) are conformant only if they
//! reproduce these outputs bit-for-bit on every supported architecture.
//!
//! Decisions locked here:
//! - **Per-channel symmetric INT8 weights**, **per-tensor symmetric INT8
//!   activations.** Zero point is always 0. No asymmetric path.
//! - **Scales** are positive numerators stored as `i32` with a fixed
//!   denominator of `2^15`. No `f32` arithmetic on the consensus path.
//! - **i32 accumulators** for matmul; bounded by `k <= 2^15` (this is the
//!   same bound enforced by [`ai_pow::params::MatmulParams::validate`]).
//! - **Banker's rounding** (round-half-to-even) on rescale; never compiled
//!   away (`#[inline(never)]` guard in [`round_half_to_even_div_pow2`]).
//! - **Reduction order** is row-major, ascending index. Vendor kernels that
//!   reorder reductions are non-conformant by construction.

use thiserror::Error;

/// Fixed denominator for all rescale operations. Choosing `2^15` lets us
/// cover scales up to ~65536 with i32 numerators while still leaving room
/// for the multiplication step in i64.
pub const SCALE_DENOM_LOG2: u32 = 15;

/// Symmetric INT8 quantization scale. Numerator is positive `i32`; the
/// effective scale is `num / 2^15`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    pub num: i32,
}

impl Scale {
    /// Build a scale from its numerator. Must be > 0; the puzzle does not
    /// permit zero or negative scales (a zero scale would zero out the
    /// entire output and is meaningless for inference).
    pub fn from_num(num: i32) -> Result<Self, QuantError> {
        if num <= 0 {
            return Err(QuantError::NonPositiveScale);
        }
        Ok(Self { num })
    }

    /// Build a scale from a positive `f32`, rounded to the nearest
    /// representable numerator. Intended for offline conversion of a
    /// PyTorch-extracted scale; never called on the consensus path.
    pub fn from_f32(value: f32) -> Result<Self, QuantError> {
        if !value.is_finite() || value <= 0.0 {
            return Err(QuantError::NonPositiveScale);
        }
        let denom = (1u64 << SCALE_DENOM_LOG2) as f32;
        let num_f = (value * denom).round();
        if num_f < 1.0 || num_f > i32::MAX as f32 {
            return Err(QuantError::ScaleOutOfRange);
        }
        Ok(Self { num: num_f as i32 })
    }

    pub fn as_num(&self) -> i32 {
        self.num
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuantError {
    #[error("scale must be > 0")]
    NonPositiveScale,
    #[error("scale numerator out of i32 range")]
    ScaleOutOfRange,
}

/// Round `value / 2^shift` to the nearest integer, ties-to-even. Implemented
/// via floor-shift + remainder, all in i64. This function MUST stay
/// `#[inline(never)]` so the compiler cannot fuse it into a faster but
/// architecture-dependent rounding mode.
#[inline(never)]
pub fn round_half_to_even_div_pow2(value: i64, shift: u32) -> i64 {
    debug_assert!(shift > 0 && shift < 63);
    let half = 1i64 << (shift - 1);
    let trunc = value >> shift; // arithmetic shift = floor for signed
    let frac = value - (trunc << shift); // always in [0, 2^shift)
    debug_assert!(frac >= 0 && frac < (1i64 << shift));
    if frac < half {
        trunc
    } else if frac > half {
        trunc + 1
    } else if trunc & 1 == 0 {
        trunc
    } else {
        trunc + 1
    }
}

/// Convert an `i32` accumulator back to `i8` via a positive scale, with
/// banker's rounding and saturating clamp to `[-128, 127]`.
///
/// Panics in debug if `acc * scale.num` would overflow `i64` — caller is
/// responsible for the i32 accumulator bound (`k <= 2^15` for k-length dot
/// products of INT8 in symmetric quant).
#[inline(never)]
pub fn rescale_and_requantize(acc: i32, scale: Scale) -> i8 {
    let product = (acc as i64)
        .checked_mul(scale.num as i64)
        .expect("rescale_and_requantize: i64 overflow — accumulator times scale too large");
    let rounded = round_half_to_even_div_pow2(product, SCALE_DENOM_LOG2);
    rounded.clamp(i8::MIN as i64, i8::MAX as i64) as i8
}

/// Saturating clamp from any signed integer to i8. Helper for ops that
/// produce intermediate values outside the i8 range and need the same
/// saturation semantics as [`rescale_and_requantize`].
#[inline(always)]
pub fn saturate_i8(value: i64) -> i8 {
    value.clamp(i8::MIN as i64, i8::MAX as i64) as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_from_num_rejects_non_positive() {
        assert_eq!(Scale::from_num(0), Err(QuantError::NonPositiveScale));
        assert_eq!(Scale::from_num(-1), Err(QuantError::NonPositiveScale));
        assert!(Scale::from_num(1).is_ok());
    }

    #[test]
    fn scale_from_f32_rounds() {
        // 0.5 * 2^15 = 16384, exact.
        let s = Scale::from_f32(0.5).unwrap();
        assert_eq!(s.num, 16384);
        // 1/3 * 32768 = 10922.666... rounds to 10923.
        let s = Scale::from_f32(1.0 / 3.0).unwrap();
        assert_eq!(s.num, 10923);
    }

    #[test]
    fn scale_from_f32_rejects_bad() {
        assert!(Scale::from_f32(0.0).is_err());
        assert!(Scale::from_f32(-1.0).is_err());
        assert!(Scale::from_f32(f32::NAN).is_err());
        assert!(Scale::from_f32(f32::INFINITY).is_err());
    }

    #[test]
    fn round_half_to_even_basic() {
        // 1.5 -> 2 (even). Encode as value=12, shift=3 (12/8 = 1.5).
        assert_eq!(round_half_to_even_div_pow2(12, 3), 2);
        // 0.5 -> 0 (even). value=4, shift=3 (4/8 = 0.5).
        assert_eq!(round_half_to_even_div_pow2(4, 3), 0);
        // 2.5 -> 2 (even). value=20, shift=3 (20/8 = 2.5).
        assert_eq!(round_half_to_even_div_pow2(20, 3), 2);
        // -1.5 -> -2 (even). value=-12, shift=3.
        assert_eq!(round_half_to_even_div_pow2(-12, 3), -2);
        // -0.5 -> 0 (even). value=-4, shift=3.
        assert_eq!(round_half_to_even_div_pow2(-4, 3), 0);
        // -2.5 -> -2 (even). value=-20, shift=3.
        assert_eq!(round_half_to_even_div_pow2(-20, 3), -2);
    }

    #[test]
    fn round_half_to_even_non_half_cases() {
        // 1.25 -> 1, 1.75 -> 2, -1.25 -> -1, -1.75 -> -2.
        assert_eq!(round_half_to_even_div_pow2(10, 3), 1); // 10/8 = 1.25
        assert_eq!(round_half_to_even_div_pow2(14, 3), 2); // 14/8 = 1.75
        assert_eq!(round_half_to_even_div_pow2(-10, 3), -1);
        assert_eq!(round_half_to_even_div_pow2(-14, 3), -2);
    }

    #[test]
    fn round_half_to_even_zero_and_one() {
        assert_eq!(round_half_to_even_div_pow2(0, 15), 0);
        assert_eq!(round_half_to_even_div_pow2(1 << 15, 15), 1);
        assert_eq!(round_half_to_even_div_pow2(-(1 << 15), 15), -1);
    }

    #[test]
    fn rescale_identity_scale() {
        // Scale = 1.0 (num = 2^15). Output = clamp(acc).
        let s = Scale::from_num(1 << SCALE_DENOM_LOG2).unwrap();
        assert_eq!(rescale_and_requantize(0, s), 0);
        assert_eq!(rescale_and_requantize(50, s), 50);
        assert_eq!(rescale_and_requantize(-50, s), -50);
        assert_eq!(rescale_and_requantize(500, s), 127); // clamps
        assert_eq!(rescale_and_requantize(-500, s), -128);
    }

    #[test]
    fn rescale_with_half_scale_uses_bankers_rounding() {
        // Scale = 0.5 (num = 2^14). 5 * 0.5 = 2.5 -> 2 (round-to-even).
        let s = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 1)).unwrap();
        assert_eq!(rescale_and_requantize(5, s), 2);
        // 7 * 0.5 = 3.5 -> 4.
        assert_eq!(rescale_and_requantize(7, s), 4);
        // -5 * 0.5 = -2.5 -> -2.
        assert_eq!(rescale_and_requantize(-5, s), -2);
    }

    #[test]
    fn rescale_clamps_at_int_extremes() {
        let s = Scale::from_num(1 << SCALE_DENOM_LOG2).unwrap();
        assert_eq!(rescale_and_requantize(i32::MAX, s), 127);
        assert_eq!(rescale_and_requantize(i32::MIN, s), -128);
    }

    #[test]
    fn saturate_helper_matches_clamp() {
        assert_eq!(saturate_i8(100), 100);
        assert_eq!(saturate_i8(200), 127);
        assert_eq!(saturate_i8(-200), -128);
        assert_eq!(saturate_i8(0), 0);
    }
}
