//! Integer LayerNorm reference.
//!
//! `out[i] = ((x[i] - mean) / sqrt(var + eps)) * gamma[i] + beta[i]`
//! where `mean`, `var` are per-token statistics.
//!
//! Bit-exact integer implementation: the variance reciprocal-sqrt uses the
//! same `isqrt_floor` as [`crate::rmsnorm`]; the multiplication and final
//! shift mirror RMSNorm. The only added work is the mean subtract and a
//! per-channel `beta` bias.
//!
//! Forward-compatibility: Gemma 4 31B and Qwen 3.6 27B both use RMSNorm, so
//! this module is not on the cutover critical path. It exists so a future
//! model registration that wants LayerNorm can be added without a hard
//! fork.

use thiserror::Error;

use crate::rmsnorm::{isqrt_floor, FRACT_BITS as RMS_FRACT_BITS};

/// Default integer epsilon; same magnitude convention as
/// `rmsnorm::DEFAULT_EPS_Q` (units: `var` = mean of squared deviations).
pub const DEFAULT_EPS_Q: i64 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LayerNormError {
    #[error("input, gamma, beta, and output must all have length `hidden`")]
    LenMismatch,
    #[error("hidden must be > 0")]
    ZeroHidden,
}

/// Apply integer LayerNorm to `input`, writing the i32 result into `output`.
///
/// `gamma` (per-channel scale) and `beta` (per-channel bias) are the two
/// learned weight vectors. `eps_q` is added to `var` to avoid division by
/// zero. Output is i32; the caller is responsible for the final
/// [`crate::quant::rescale_and_requantize`] back to i8.
pub fn layernorm(
    input: &[i8],
    gamma: &[i8],
    beta: &[i8],
    output: &mut [i32],
    eps_q: i64,
) -> Result<(), LayerNormError> {
    let hidden = input.len();
    if hidden == 0 {
        return Err(LayerNormError::ZeroHidden);
    }
    if gamma.len() != hidden || beta.len() != hidden || output.len() != hidden {
        return Err(LayerNormError::LenMismatch);
    }

    // mean = sum(x) / hidden. sum fits in i32: |x|<=128, hidden<=8192 →
    // |sum| <= 1_048_576, fits in i32. Compute in i64 for one accumulator
    // class.
    let mut sum: i64 = 0;
    for &x in input.iter() {
        sum = sum.wrapping_add(x as i64);
    }
    // Banker-rounded mean to nearest i32; for our integer range, simply
    // floor-divide is fine (rounding error cancels into beta on average).
    // We use floor toward zero (Rust's signed division semantics).
    let mean = sum / (hidden as i64);

    // var = mean( (x - mean)^2 ). Accumulate sum of squares of deviations
    // in i64. Max deviation per element: ~256, squared: ~65k, * 8192 →
    // ~5.4e8, fits in i32 — but use i64 again.
    let mut sumsq_dev: i64 = 0;
    for &x in input.iter() {
        let d = (x as i64) - mean;
        sumsq_dev = sumsq_dev.wrapping_add(d * d);
    }

    // inv_std_fixed = isqrt_floor( (hidden << 2*FRACT_BITS) / (sumsq_dev + eps_q) )
    let num = (hidden as i64)
        .checked_shl(2 * RMS_FRACT_BITS)
        .expect("layernorm: hidden << 2*FRACT_BITS overflowed");
    let den = sumsq_dev.saturating_add(eps_q).max(1);
    let inv_std_fixed = isqrt_floor(num / den);

    // out[i] = round( (x[i] - mean) * gamma[i] * inv_std_fixed >> FRACT_BITS )
    //          + beta[i]
    let half = 1i64 << (RMS_FRACT_BITS - 1);
    for i in 0..hidden {
        let dev = (input[i] as i64) - mean;
        let prod = dev * (gamma[i] as i64) * inv_std_fixed;
        let shifted = (prod + half) >> RMS_FRACT_BITS;
        let with_bias = shifted + (beta[i] as i64);
        output[i] = with_bias.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_input_with_zero_beta_yields_zeros() {
        let hidden = 8;
        let input = vec![0i8; hidden];
        let gamma = vec![32i8; hidden];
        let beta = vec![0i8; hidden];
        let mut output = vec![0i32; hidden];
        layernorm(&input, &gamma, &beta, &mut output, DEFAULT_EPS_Q).unwrap();
        for &v in &output {
            assert_eq!(v, 0);
        }
    }

    #[test]
    fn zero_input_with_nonzero_beta_yields_beta() {
        let hidden = 8;
        let input = vec![0i8; hidden];
        let gamma = vec![32i8; hidden];
        let beta: Vec<i8> = (0..hidden as i8).collect();
        let mut output = vec![0i32; hidden];
        layernorm(&input, &gamma, &beta, &mut output, DEFAULT_EPS_Q).unwrap();
        for (i, &v) in output.iter().enumerate() {
            assert_eq!(v, i as i32, "beta should pass through at zero input");
        }
    }

    #[test]
    fn constant_input_yields_zero_then_beta() {
        // If every x[i] = c, mean = c, all deviations = 0, var = 0,
        // (x - mean) * gamma * inv_std = 0, so output = beta.
        let hidden = 16;
        let input = vec![42i8; hidden];
        let gamma = vec![100i8; hidden];
        let beta: Vec<i8> = (0..hidden as i8).map(|v| v * 2 - 16).collect();
        let mut output = vec![0i32; hidden];
        layernorm(&input, &gamma, &beta, &mut output, DEFAULT_EPS_Q).unwrap();
        for (i, &v) in output.iter().enumerate() {
            assert_eq!(v, beta[i] as i32);
        }
    }

    #[test]
    fn sign_symmetric_around_mean() {
        // Symmetric input around 0 with zero beta → output is sign-symmetric.
        let hidden = 32;
        let input: Vec<i8> = (0..hidden as i8).map(|v| v - 16).collect();
        let gamma = vec![16i8; hidden];
        let beta = vec![0i8; hidden];
        let mut out_pos = vec![0i32; hidden];
        let mut out_neg = vec![0i32; hidden];
        layernorm(&input, &gamma, &beta, &mut out_pos, DEFAULT_EPS_Q).unwrap();
        let neg_input: Vec<i8> = input.iter().map(|x| -x).collect();
        layernorm(&neg_input, &gamma, &beta, &mut out_neg, DEFAULT_EPS_Q).unwrap();
        for i in 0..hidden {
            assert_eq!(out_pos[i], -out_neg[i], "mismatch at {i}");
        }
    }

    #[test]
    fn rejects_len_mismatch() {
        let mut output = vec![0i32; 4];
        assert_eq!(
            layernorm(&[1, 2, 3], &[1; 4], &[0; 4], &mut output, DEFAULT_EPS_Q).err(),
            Some(LayerNormError::LenMismatch),
        );
        let mut output = vec![0i32; 3];
        assert_eq!(
            layernorm(&[1; 4], &[1; 4], &[0; 4], &mut output, DEFAULT_EPS_Q).err(),
            Some(LayerNormError::LenMismatch),
        );
    }

    #[test]
    fn rejects_zero_hidden() {
        let mut output: Vec<i32> = Vec::new();
        assert_eq!(
            layernorm(&[], &[], &[], &mut output, DEFAULT_EPS_Q).err(),
            Some(LayerNormError::ZeroHidden),
        );
    }

    #[test]
    fn determinism_round_trip() {
        let hidden = 64;
        let input: Vec<i8> = (0..hidden as i32).map(|i| (i * 11 - 100) as i8).collect();
        let gamma: Vec<i8> = (0..hidden as i32).map(|i| (i * 3 + 5) as i8).collect();
        let beta: Vec<i8> = (0..hidden as i32).map(|i| (i - 32) as i8).collect();
        let mut a = vec![0i32; hidden];
        let mut b = vec![0i32; hidden];
        layernorm(&input, &gamma, &beta, &mut a, DEFAULT_EPS_Q).unwrap();
        layernorm(&input, &gamma, &beta, &mut b, DEFAULT_EPS_Q).unwrap();
        assert_eq!(a, b);
    }
}
