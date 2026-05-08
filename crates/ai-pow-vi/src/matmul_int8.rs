//! Inference-side INT8 matmul.
//!
//! `ai_pow::matmul` exists for the puzzle and bakes in the `(A+E)(B+F)`
//! noise terms; for the inference forward pass we want a plain `A·B` over
//! INT8 inputs with i32 accumulator. This module supplies that, plus a
//! batch requantize from i32 → i8 via [`crate::quant::Scale`].
//!
//! Reduction order is fixed: row-major, ascending `l`. Vendor kernels
//! (cuBLAS-INT8, oneDNN, NEON dotprod) only conform if they produce
//! byte-identical outputs to this reference, which means SIMD backends
//! that reorder reductions are non-conformant by construction. SIMD jets
//! (Phase 4) must take care to preserve the order.
//!
//! Layout convention (matches `ai_pow::matmul`):
//! - `A`: `m * k`, row-major.
//! - `B`: `k * n`, **column-major** — column `j` lives at `[j*k, j*k + k)`.
//! - `out`: `m * n`, row-major i32.

use thiserror::Error;

use crate::quant::{rescale_and_requantize, Scale};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MatmulError {
    #[error("a length must equal m * k")]
    BadALen,
    #[error("b length must equal k * n")]
    BadBLen,
    #[error("out length must equal m * n")]
    BadOutLen,
    #[error("dimensions must be > 0")]
    ZeroDim,
    #[error("k must be in 1..=2^15 to keep dot products in i32 range")]
    KOutOfRange,
}

/// `k`-length INT8 dot product with `i32` accumulator. Bound: `k <= 2^15`
/// keeps the worst-case `k * 128 * 128 = k * 2^14 < 2^29` in i32 range.
#[inline(never)]
pub fn dot_int8(a: &[i8], b: &[i8]) -> i32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc: i32 = 0;
    for l in 0..a.len() {
        acc = acc.wrapping_add((a[l] as i32) * (b[l] as i32));
    }
    acc
}

/// Compute `out = A · B`. `out[i*n + j]` is the dot product of `A`'s
/// `i`-th row and `B`'s `j`-th column.
pub fn matmul_int8(
    a: &[i8],
    b: &[i8],
    m: u32,
    k: u32,
    n: u32,
    out: &mut [i32],
) -> Result<(), MatmulError> {
    if m == 0 || k == 0 || n == 0 {
        return Err(MatmulError::ZeroDim);
    }
    if k > (1u32 << 15) {
        return Err(MatmulError::KOutOfRange);
    }
    let mu = m as usize;
    let ku = k as usize;
    let nu = n as usize;
    if a.len() != mu * ku {
        return Err(MatmulError::BadALen);
    }
    if b.len() != ku * nu {
        return Err(MatmulError::BadBLen);
    }
    if out.len() != mu * nu {
        return Err(MatmulError::BadOutLen);
    }
    for i in 0..mu {
        let row = &a[i * ku..(i + 1) * ku];
        for j in 0..nu {
            let col = &b[j * ku..(j + 1) * ku];
            out[i * nu + j] = dot_int8(row, col);
        }
    }
    Ok(())
}

/// Batch requantize an `i32` accumulator vector to `i8` with a per-tensor
/// scale. Wraps [`rescale_and_requantize`] over a slice.
pub fn requantize_vec(acc: &[i32], scale: Scale, out: &mut [i8]) -> Result<(), MatmulError> {
    if acc.len() != out.len() {
        return Err(MatmulError::BadOutLen);
    }
    for (a, o) in acc.iter().zip(out.iter_mut()) {
        *o = rescale_and_requantize(*a, scale);
    }
    Ok(())
}

/// Combined matmul + requantize: compute `out = round(A · B * scale)` to
/// i8. Convenience for FFN / attention layers that always end with a
/// requantize.
pub fn matmul_int8_requant(
    a: &[i8],
    b: &[i8],
    m: u32,
    k: u32,
    n: u32,
    scale: Scale,
    out: &mut [i8],
) -> Result<(), MatmulError> {
    let mu = m as usize;
    let nu = n as usize;
    if out.len() != mu * nu {
        return Err(MatmulError::BadOutLen);
    }
    let mut acc = vec![0i32; mu * nu];
    matmul_int8(a, b, m, k, n, &mut acc)?;
    requantize_vec(&acc, scale, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::SCALE_DENOM_LOG2;

    fn naive_matmul_oracle(a: &[i8], b: &[i8], m: u32, k: u32, n: u32) -> Vec<i32> {
        let mu = m as usize;
        let ku = k as usize;
        let nu = n as usize;
        let mut out = vec![0i32; mu * nu];
        for i in 0..mu {
            for j in 0..nu {
                let mut acc = 0i32;
                for l in 0..ku {
                    let aa = a[i * ku + l] as i32;
                    let bb = b[j * ku + l] as i32; // column-major
                    acc += aa * bb;
                }
                out[i * nu + j] = acc;
            }
        }
        out
    }

    #[test]
    fn dot_int8_simple() {
        let a = [1i8, 2, 3, 4];
        let b = [5i8, 6, 7, 8];
        // 5 + 12 + 21 + 32 = 70
        assert_eq!(dot_int8(&a, &b), 70);
    }

    #[test]
    fn dot_int8_negative() {
        let a = [-1i8, -2, -3];
        let b = [4i8, 5, 6];
        // -4 + -10 + -18 = -32
        assert_eq!(dot_int8(&a, &b), -32);
    }

    #[test]
    fn dot_int8_max_magnitude_no_overflow() {
        let a = vec![127i8; 1 << 15]; // k = 2^15
        let b = vec![127i8; 1 << 15];
        // 127 * 127 = 16129; * 2^15 = 528_482_304, fits in i32 (max 2_147_483_647).
        let r = dot_int8(&a, &b);
        assert_eq!(r, 16129 * (1 << 15));
    }

    #[test]
    fn matmul_matches_oracle_small() {
        // (3, 4) x (4, 5) -> (3, 5).
        let m = 3u32;
        let k = 4u32;
        let n = 5u32;
        let a: Vec<i8> = (0..m * k).map(|x| ((x as i32 * 3 - 7) as i8)).collect();
        let b: Vec<i8> = (0..k * n).map(|x| ((x as i32 * 5 + 2) as i8)).collect();
        let mut got = vec![0i32; (m * n) as usize];
        matmul_int8(&a, &b, m, k, n, &mut got).unwrap();
        let oracle = naive_matmul_oracle(&a, &b, m, k, n);
        assert_eq!(got, oracle);
    }

    #[test]
    fn matmul_handles_rectangular() {
        // (1, 8) x (8, 1) -> (1, 1) — a single dot product.
        let a = vec![1i8, 2, 3, 4, 5, 6, 7, 8];
        let b = vec![1i8, 1, 1, 1, 1, 1, 1, 1];
        let mut got = vec![0i32; 1];
        matmul_int8(&a, &b, 1, 8, 1, &mut got).unwrap();
        assert_eq!(got[0], 36);
    }

    #[test]
    fn matmul_rejects_bad_dims() {
        // m=2, k=2, n=2 — a is 2*2=4 i8, b is 2*2=4 i8, out is 2*2=4 i32.
        let a = vec![0i8; 4];
        let b = vec![0i8; 4];
        let mut out = vec![0i32; 4];
        matmul_int8(&a, &b, 2, 2, 2, &mut out).unwrap();
        // Now break a length.
        let a_short = vec![0i8; 3];
        assert_eq!(
            matmul_int8(&a_short, &b, 2, 2, 2, &mut out).err(),
            Some(MatmulError::BadALen),
        );
        // Zero dim.
        assert_eq!(
            matmul_int8(&a, &b, 0, 2, 2, &mut out).err(),
            Some(MatmulError::ZeroDim),
        );
        // Oversize k.
        let a_big = vec![0i8; ((1u32 << 15) + 1) as usize];
        let b_big = vec![0i8; ((1u32 << 15) + 1) as usize];
        let mut out_one = vec![0i32; 1];
        assert_eq!(
            matmul_int8(&a_big, &b_big, 1, (1 << 15) + 1, 1, &mut out_one).err(),
            Some(MatmulError::KOutOfRange),
        );
    }

    #[test]
    fn requantize_vec_round_trips_unit_scale() {
        let scale = Scale::from_num(1 << SCALE_DENOM_LOG2).unwrap();
        let acc = vec![0i32, 1, -1, 50, -50, 200, -200];
        let mut out = vec![0i8; acc.len()];
        requantize_vec(&acc, scale, &mut out).unwrap();
        // Identity for in-range; saturate at extremes.
        assert_eq!(out, vec![0, 1, -1, 50, -50, 127, -128]);
    }

    #[test]
    fn matmul_int8_requant_combined() {
        let m = 2u32;
        let k = 4u32;
        let n = 3u32;
        let a = vec![1i8, 2, 3, 4, 5, 6, 7, 8];
        let b: Vec<i8> = (0..k * n).map(|x| (x as i8) + 1).collect();
        let scale = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(); // 1/16
        let mut acc = vec![0i32; (m * n) as usize];
        matmul_int8(&a, &b, m, k, n, &mut acc).unwrap();
        let mut combined = vec![0i8; (m * n) as usize];
        matmul_int8_requant(&a, &b, m, k, n, scale, &mut combined).unwrap();
        let mut by_hand = vec![0i8; (m * n) as usize];
        requantize_vec(&acc, scale, &mut by_hand).unwrap();
        assert_eq!(combined, by_hand);
    }

    #[test]
    fn matmul_determinism() {
        let m = 4u32;
        let k = 8u32;
        let n = 4u32;
        let a: Vec<i8> = (0..m * k).map(|x| ((x as i32 * 7 - 11) as i8)).collect();
        let b: Vec<i8> = (0..k * n).map(|x| ((x as i32 * 5 + 1) as i8)).collect();
        let mut a1 = vec![0i32; (m * n) as usize];
        let mut a2 = vec![0i32; (m * n) as usize];
        matmul_int8(&a, &b, m, k, n, &mut a1).unwrap();
        matmul_int8(&a, &b, m, k, n, &mut a2).unwrap();
        assert_eq!(a1, a2);
    }
}
