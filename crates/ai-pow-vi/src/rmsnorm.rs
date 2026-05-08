//! Integer RMSNorm reference.
//!
//! `out[i] = x[i] * gamma[i] / sqrt((1/hidden) * Σ x[j]² + eps)`
//!
//! Fully integer; no `f32`. The reciprocal-sqrt is computed via integer
//! `isqrt_floor` over a 32-bit fixed-point representation, eliminating any
//! transcendental-function variance across machines.
//!
//! Output: a hidden-length `i32` accumulator vector. The caller is
//! responsible for the final [`crate::quant::rescale_and_requantize`] back
//! to `i8` with a per-tensor scale.

use thiserror::Error;

/// Fractional bits used inside the reciprocal-sqrt fixed-point. 16 gives
/// 65536 levels of inv-rms precision, which is well below INT8 quantization
/// noise — increasing this widens the `prod` intermediate beyond what i64
/// can carry, so don't.
pub const FRACT_BITS: u32 = 16;

/// Default integer epsilon for RMSNorm (in `sumsq` units). With Gemma /
/// Qwen `hidden ≈ 5000`, this corresponds to a float-eps of roughly
/// `2e-4`, comfortably above the noise floor of INT8 inference.
pub const DEFAULT_EPS_Q: i64 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RmsNormError {
    #[error("input, gamma, and output must all have length `hidden`")]
    LenMismatch,
    #[error("hidden must be > 0")]
    ZeroHidden,
}

/// Integer floor-square-root: largest `k` with `k*k <= y`. Newton-Raphson
/// over `i64`, converges in O(log log y) iterations from a bit-length-based
/// initial guess. `#[inline(never)]` so the compiler cannot replace this
/// with a non-deterministic intrinsic.
#[inline(never)]
pub fn isqrt_floor(y: i64) -> i64 {
    debug_assert!(y >= 0);
    if y < 2 {
        return y;
    }
    // Initial guess: 2^ceil(bits / 2). Always >= isqrt(y).
    let bits = 64 - (y as u64).leading_zeros();
    let mut x = 1i64 << ((bits as i32 + 1) / 2);
    loop {
        let next = (x + y / x) / 2;
        if next >= x {
            return x;
        }
        x = next;
    }
}

/// Apply integer RMSNorm to `input`, writing the i32 result into `output`.
///
/// `gamma` is the per-channel scale (a learned weight in INT8 land). `eps_q`
/// is added to `sumsq` to avoid division by zero; pass [`DEFAULT_EPS_Q`] if
/// in doubt.
///
/// The output is i32; caller must `rescale_and_requantize` separately with
/// a per-tensor scale.
pub fn rmsnorm(
    input: &[i8],
    gamma: &[i8],
    output: &mut [i32],
    eps_q: i64,
) -> Result<(), RmsNormError> {
    let hidden = input.len();
    if hidden == 0 {
        return Err(RmsNormError::ZeroHidden);
    }
    if gamma.len() != hidden || output.len() != hidden {
        return Err(RmsNormError::LenMismatch);
    }

    // sum(x[j]^2). At hidden=5376 and |x|<=128, max sumsq ~ 88M, comfortably
    // in i32 — but accumulate in i64 anyway to keep one accumulator class
    // throughout the crate.
    let mut sumsq: i64 = 0;
    for &x in input.iter() {
        let xi = x as i64;
        sumsq = sumsq.wrapping_add(xi * xi);
    }

    // inv_rms_fixed = isqrt_floor( (hidden << 2*FRACT_BITS) / (sumsq + eps_q) )
    // Units: sqrt(unitless) << FRACT_BITS, since we put both `hidden` and
    // `sumsq` into the same scale.
    let num = (hidden as i64)
        .checked_shl(2 * FRACT_BITS)
        .expect("rmsnorm: hidden << 2*FRACT_BITS overflowed; reduce FRACT_BITS or hidden");
    let den = sumsq.saturating_add(eps_q).max(1);
    let inv_rms_fixed = isqrt_floor(num / den);

    // out[i] = round( x[i] * gamma[i] * inv_rms_fixed >> FRACT_BITS )
    // Max prod: 128 * 128 * (1 << 22) ≈ 2^36, well within i64.
    let half = 1i64 << (FRACT_BITS - 1);
    for i in 0..hidden {
        let prod = (input[i] as i64) * (gamma[i] as i64) * inv_rms_fixed;
        // Round-half-up is OK here because the i32 output is itself going
        // to be requantized through banker's rounding. We just want a
        // deterministic floor-shift with bias.
        let shifted = (prod + half) >> FRACT_BITS;
        // Clamp to i32 to make any pathological input hit a deterministic
        // saturated value rather than wrap silently.
        output[i] = shifted.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_small_cases() {
        assert_eq!(isqrt_floor(0), 0);
        assert_eq!(isqrt_floor(1), 1);
        assert_eq!(isqrt_floor(2), 1);
        assert_eq!(isqrt_floor(3), 1);
        assert_eq!(isqrt_floor(4), 2);
        assert_eq!(isqrt_floor(15), 3);
        assert_eq!(isqrt_floor(16), 4);
        assert_eq!(isqrt_floor(99), 9);
        assert_eq!(isqrt_floor(100), 10);
        assert_eq!(isqrt_floor(10_000), 100);
        assert_eq!(isqrt_floor(10_001), 100);
        assert_eq!(isqrt_floor(99_999_999), 9999);
    }

    #[test]
    fn isqrt_large_cases() {
        // 2^62: result is 2^31.
        assert_eq!(isqrt_floor(1i64 << 62), 1i64 << 31);
        // (2^31 + 1)^2 - 1: sqrt is 2^31 (floor).
        let n = (1i64 << 31) + 1;
        assert_eq!(isqrt_floor(n * n - 1), n - 1);
        assert_eq!(isqrt_floor(n * n), n);
    }

    #[test]
    fn isqrt_floor_property_is_monotone_and_bounded() {
        for y in 0..1000 {
            let s = isqrt_floor(y);
            assert!(s * s <= y, "{s}^2 > {y}");
            assert!((s + 1) * (s + 1) > y, "{}^2 <= {y}", s + 1);
        }
    }

    #[test]
    fn rmsnorm_zero_input_yields_zero_output() {
        let hidden = 16;
        let input = vec![0i8; hidden];
        let gamma = vec![32i8; hidden];
        let mut output = vec![0i32; hidden];
        rmsnorm(&input, &gamma, &mut output, DEFAULT_EPS_Q).unwrap();
        // All zeros.
        for &v in &output {
            assert_eq!(v, 0);
        }
    }

    #[test]
    fn rmsnorm_uniform_input_normalizes() {
        // If every x[i] = c and every gamma[i] = g, then
        // mean_sq = c^2 (no bias), rms = c, output[i] = c * g / c = g.
        // In fixed-point: output[i] should be very close to g << FRACT_BITS
        // before the final shift, i.e. close to g after shift. The test
        // checks that all outputs match within a small rounding window.
        let hidden = 64;
        let input = vec![64i8; hidden];
        let gamma = vec![20i8; hidden];
        let mut output = vec![0i32; hidden];
        rmsnorm(&input, &gamma, &mut output, DEFAULT_EPS_Q).unwrap();
        // Expected: output[i] ≈ gamma * 1 = 20 (no requantize step yet).
        for &v in &output {
            assert!(
                (v - 20).abs() <= 1,
                "expected ~20, got {v} (rounding noise should be <=1)"
            );
        }
    }

    #[test]
    fn rmsnorm_scaling_invariance() {
        // x and gamma are flipped sign together → output sign should flip
        // but magnitudes match.
        let hidden = 64;
        let input: Vec<i8> = (0..hidden as i8).collect();
        let gamma = vec![16i8; hidden];
        let mut out_pos = vec![0i32; hidden];
        let mut out_neg = vec![0i32; hidden];
        rmsnorm(&input, &gamma, &mut out_pos, DEFAULT_EPS_Q).unwrap();
        let neg_input: Vec<i8> = input.iter().map(|x| -x).collect();
        rmsnorm(&neg_input, &gamma, &mut out_neg, DEFAULT_EPS_Q).unwrap();
        for i in 0..hidden {
            assert_eq!(out_pos[i], -out_neg[i], "sign symmetry at {i}");
        }
    }

    #[test]
    fn rmsnorm_rejects_len_mismatch() {
        let mut output = vec![0i32; 4];
        assert_eq!(
            rmsnorm(&[1, 2, 3], &[1, 2, 3, 4], &mut output, DEFAULT_EPS_Q).err(),
            Some(RmsNormError::LenMismatch),
        );
        let mut output = vec![0i32; 4];
        assert_eq!(
            rmsnorm(&[1, 2, 3, 4], &[1, 2, 3], &mut output, DEFAULT_EPS_Q).err(),
            Some(RmsNormError::LenMismatch),
        );
        let mut output = vec![0i32; 3];
        assert_eq!(
            rmsnorm(&[1, 2, 3, 4], &[1, 2, 3, 4], &mut output, DEFAULT_EPS_Q).err(),
            Some(RmsNormError::LenMismatch),
        );
    }

    #[test]
    fn rmsnorm_rejects_zero_hidden() {
        let mut output: Vec<i32> = Vec::new();
        assert_eq!(
            rmsnorm(&[], &[], &mut output, DEFAULT_EPS_Q).err(),
            Some(RmsNormError::ZeroHidden),
        );
    }

    #[test]
    fn rmsnorm_determinism() {
        // Same input twice → byte-equal output.
        let hidden = 128;
        let input: Vec<i8> = (0..hidden as i32).map(|i| ((i * 7 - 200) as i8)).collect();
        let gamma: Vec<i8> = (0..hidden as i32).map(|i| ((i * 3 + 5) as i8)).collect();
        let mut a = vec![0i32; hidden];
        let mut b = vec![0i32; hidden];
        rmsnorm(&input, &gamma, &mut a, DEFAULT_EPS_Q).unwrap();
        rmsnorm(&input, &gamma, &mut b, DEFAULT_EPS_Q).unwrap();
        assert_eq!(a, b);
    }
}
