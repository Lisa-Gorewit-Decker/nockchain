//! Integer softmax for attention.
//!
//! Standard max-subtract softmax, with the exponential implemented as a
//! 256-entry LUT instead of a transcendental call. The LUT is committed
//! alongside the model weights (same as activation LUTs), so the curve is
//! pinned at model release time.
//!
//! Convention: the LUT covers `delta` (= `max_score - score`) in
//! `[0, 256)` LUT units. The caller is responsible for pre-scaling its
//! `i32` attention scores into LUT units before calling [`softmax_int`].
//! For example, if scores live in `[-2^25, 2^25]` and the natural softmax
//! domain is `[-25, 25]`, the caller would pre-multiply scores by
//! `LUT_UNIT * 256 / 2^25` so a 1-unit step in scores corresponds to a
//! `(256 / 2^25)`-step in LUT index.
//!
//! Output is `i8` representing softmax probabilities scaled to `[0, 127]`
//! (so a uniform distribution with `L = 4` produces `[31, 32, 32, 32]` or
//! similar). The exact integer bias depends on the LUT bytes; consensus
//! pins it via the LUT commitment.

use blake3::Hasher;
use thiserror::Error;

const CTX_EXP_LUT: &str = "ai-pow-vi v1 exp-lut";

/// 256-entry `i32` lookup table for `exp(-x)` in fixed-point.
///
/// The LUT bytes (each entry serialized little-endian) are committed inside
/// `comm_W`; a model release pins this curve alongside the weight tensors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpLut {
    /// `table[idx]` is the (caller-defined) fixed-point representation of
    /// `exp(-idx * step_size)` for some `step_size` chosen at model build
    /// time. Consensus does not need to know `step_size`; only the bytes.
    pub table: [i32; 256],
}

impl ExpLut {
    /// Build from a flat byte buffer (`256 * 4` little-endian i32). Used
    /// when loading from the sideloaded model directory.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ExpLutError> {
        if bytes.len() != 256 * 4 {
            return Err(ExpLutError::WrongLength(bytes.len()));
        }
        let mut table = [0i32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let off = i * 4;
            *slot =
                i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        }
        Ok(Self { table })
    }

    /// Serialize as `256 * 4` little-endian bytes for inclusion in
    /// `comm_W`.
    pub fn to_bytes(&self) -> [u8; 256 * 4] {
        let mut out = [0u8; 256 * 4];
        for (i, &v) in self.table.iter().enumerate() {
            let off = i * 4;
            let bytes = v.to_le_bytes();
            out[off..off + 4].copy_from_slice(&bytes);
        }
        out
    }

    /// Domain-separated commitment to the LUT bytes. Used as a leaf within
    /// `comm_W` so model releases pin the softmax curve.
    pub fn commit(&self) -> [u8; 32] {
        let mut hasher = Hasher::new_derive_key(CTX_EXP_LUT);
        hasher.update(&self.to_bytes());
        *hasher.finalize().as_bytes()
    }

    /// "All ones" LUT — every output is `2^16`. Useful for tests: with this
    /// LUT, softmax should produce a uniform distribution regardless of the
    /// input scores.
    pub fn uniform_test() -> Self {
        Self {
            table: [1 << 16; 256],
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExpLutError {
    #[error("LUT byte slice must be exactly 1024 bytes; got {0}")]
    WrongLength(usize),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SoftmaxError {
    #[error("scores and out must have the same length")]
    LenMismatch,
    #[error("scores must be non-empty")]
    Empty,
}

/// Run integer softmax over pre-scaled `i32` scores using `lut`. The output
/// is `i8` in the convention "probability * 127, rounded to nearest, ties
/// away from zero".
///
/// Determinism: every step is integer arithmetic with well-defined Rust
/// operator semantics. No `f32`. No transcendental calls.
pub fn softmax_int(
    scores_scaled: &[i32],
    lut: &ExpLut,
    out: &mut [i8],
) -> Result<(), SoftmaxError> {
    if scores_scaled.is_empty() {
        return Err(SoftmaxError::Empty);
    }
    if scores_scaled.len() != out.len() {
        return Err(SoftmaxError::LenMismatch);
    }

    // Find max (scan, ascending index — same reduction-order rule as matmul).
    let mut m: i32 = scores_scaled[0];
    for &s in &scores_scaled[1..] {
        if s > m {
            m = s;
        }
    }

    // Look up exp(-(m - s_i)) for each score; accumulate sum_exp in i64
    // to avoid overflow when L is large (typical L = seq_len = 4096).
    let mut exp_vals: Vec<i32> = Vec::with_capacity(scores_scaled.len());
    let mut sum_exp: i64 = 0;
    for &s in scores_scaled {
        // delta = m - s ≥ 0. Wrap into 0..=255 (saturate), then index LUT.
        let delta = (m as i64 - s as i64).clamp(0, 255) as usize;
        let e = lut.table[delta];
        exp_vals.push(e);
        sum_exp = sum_exp.wrapping_add(e as i64);
    }

    if sum_exp <= 0 {
        // Pathological LUT or all-equal scores at extreme range. Emit zeros
        // deterministically so the caller-side requantize does the right
        // thing without panicking.
        for o in out.iter_mut() {
            *o = 0;
        }
        return Ok(());
    }

    // Normalize: out[i] = round_to_nearest( exp_vals[i] * 127 / sum_exp ).
    // Round-to-nearest via "add half-divisor before truncate" — well-defined
    // across all rust targets because `i64 / i64` truncates toward zero.
    let half = sum_exp / 2;
    for (i, &e) in exp_vals.iter().enumerate() {
        let scaled = (e as i64).wrapping_mul(127).wrapping_add(half);
        let q = scaled / sum_exp;
        out[i] = q.clamp(i8::MIN as i64, i8::MAX as i64) as i8;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_lut_yields_uniform_distribution() {
        let lut = ExpLut::uniform_test();
        let scores = vec![100i32, -42, 0, 7]; // arbitrary; LUT ignores them.
        let mut out = vec![0i8; 4];
        softmax_int(&scores, &lut, &mut out).unwrap();
        // Each = 127 / 4 ≈ 31.75 → 32 (round to nearest).
        for &v in &out {
            assert_eq!(v, 32);
        }
    }

    #[test]
    fn dominant_score_takes_most_mass() {
        // Build a LUT that decays sharply: e^-x in fixed-point.
        let mut table = [0i32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            // Crude exp(-i) integer approx: 65536 / 2^i, clamped at 0 for
            // large i. Not a real exp; just sharp-enough to peak.
            let v = if i < 16 {
                (1i32 << 16).wrapping_shr(i as u32)
            } else {
                0
            };
            *slot = v;
        }
        let lut = ExpLut { table };
        // Scores: position 2 dominates by 5 LUT-units. delta for position 2
        // is 0 (it's the max), others have delta = 5 → very small mass.
        let scores = vec![-5i32, -5, 0, -5];
        let mut out = vec![0i8; 4];
        softmax_int(&scores, &lut, &mut out).unwrap();
        assert!(out[2] > out[0], "max-score position must dominate: {out:?}");
        assert!(out[2] > out[1]);
        assert!(out[2] > out[3]);
    }

    #[test]
    fn all_equal_scores_produce_uniform_with_decaying_lut() {
        let mut table = [0i32; 256];
        // Decreasing table.
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = 65536 - (i as i32 * 100);
        }
        let lut = ExpLut { table };
        let scores = vec![42i32; 8];
        let mut out = vec![0i8; 8];
        softmax_int(&scores, &lut, &mut out).unwrap();
        // All deltas are 0, so every position gets table[0]; outputs are
        // each 127 / 8 = 15.875 → 16.
        for &v in &out {
            assert_eq!(v, 16, "out: {out:?}");
        }
    }

    #[test]
    fn rejects_bad_inputs() {
        let lut = ExpLut::uniform_test();
        let mut out = vec![0i8; 0];
        assert_eq!(
            softmax_int(&[], &lut, &mut out).err(),
            Some(SoftmaxError::Empty)
        );
        let mut out = vec![0i8; 3];
        assert_eq!(
            softmax_int(&[1, 2], &lut, &mut out).err(),
            Some(SoftmaxError::LenMismatch),
        );
    }

    #[test]
    fn output_sums_close_to_127() {
        let lut = ExpLut::uniform_test();
        let scores: Vec<i32> = (0..32).collect();
        let mut out = vec![0i8; 32];
        softmax_int(&scores, &lut, &mut out).unwrap();
        // Each = 127 / 32 = 3.96875 → 4. Sum ~ 128 (off-by-one rounding).
        let total: i32 = out.iter().map(|&v| v as i32).sum();
        assert!(
            (total - 127).abs() <= 1,
            "softmax sum should be ~127, got {total} (out: {out:?})",
        );
    }

    #[test]
    fn determinism_round_trip() {
        let lut = ExpLut::uniform_test();
        let scores: Vec<i32> = (0..64).map(|i| i * 17 - 100).collect();
        let mut a = vec![0i8; 64];
        let mut b = vec![0i8; 64];
        softmax_int(&scores, &lut, &mut a).unwrap();
        softmax_int(&scores, &lut, &mut b).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn lut_round_trip_bytes() {
        let lut = ExpLut::uniform_test();
        let bytes = lut.to_bytes();
        let again = ExpLut::from_bytes(&bytes).unwrap();
        assert_eq!(lut, again);
    }

    #[test]
    fn lut_commit_changes_on_one_byte_flip() {
        let a = ExpLut::uniform_test();
        let mut bytes = a.to_bytes();
        bytes[42] ^= 1;
        let b = ExpLut::from_bytes(&bytes).unwrap();
        assert_ne!(a.commit(), b.commit());
    }

    #[test]
    fn lut_from_bytes_rejects_bad_length() {
        assert_eq!(
            ExpLut::from_bytes(&[0u8; 100]).err(),
            Some(ExpLutError::WrongLength(100)),
        );
    }
}
