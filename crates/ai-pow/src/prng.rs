//! Domain-separated BLAKE3-XOF expansion for matrix and noise derivation.
//!
//! Every named stream is independent: changing the label or the index produces
//! an unrelated INT8 sequence. Verifiers can re-derive a single row or column
//! addressably without expanding the full matrix.

use blake3::Hasher;

const CTX_A_ROW: &str = "ai-pow v1 expand A-row";
const CTX_B_COL: &str = "ai-pow v1 expand B-col";
const CTX_E_ROW: &str = "ai-pow v1 expand E-row";
const CTX_F_COL: &str = "ai-pow v1 expand F-col";

fn expand_into(context: &str, root: &[u8], idx: u64, out: &mut [i8]) {
    let mut hasher = Hasher::new_derive_key(context);
    hasher.update(root);
    hasher.update(&idx.to_le_bytes());
    let mut xof = hasher.finalize_xof();
    // SAFETY: i8 and u8 have identical layout; we fill bytes then reinterpret.
    let buf: &mut [u8] =
        unsafe { core::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u8, out.len()) };
    xof.fill(buf);
}

/// Row `i` of the input matrix `A` (length `k`), derived from `state`.
pub fn expand_a_row(state: &[u8], i: u32, k: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    expand_into(CTX_A_ROW, state, i as u64, out);
}

/// Column `j` of the input matrix `B` (length `k`), derived from `state`.
pub fn expand_b_col(state: &[u8], j: u32, k: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    expand_into(CTX_B_COL, state, j as u64, out);
}

/// Row `i` of the noise matrix `E` (length `k`), derived from the noise seed.
pub fn expand_e_row(noise_seed: &[u8; 32], i: u32, k: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    expand_into(CTX_E_ROW, noise_seed, i as u64, out);
}

/// Column `j` of the noise matrix `F` (length `k`), derived from the noise seed.
pub fn expand_f_col(noise_seed: &[u8; 32], j: u32, k: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    expand_into(CTX_F_COL, noise_seed, j as u64, out);
}

/// Allocate-and-expand wrapper for callers that don't have a buffer.
pub fn expand_a_row_vec(state: &[u8], i: u32, k: u32) -> Vec<i8> {
    let mut v = vec![0i8; k as usize];
    expand_a_row(state, i, k, &mut v);
    v
}
pub fn expand_b_col_vec(state: &[u8], j: u32, k: u32) -> Vec<i8> {
    let mut v = vec![0i8; k as usize];
    expand_b_col(state, j, k, &mut v);
    v
}
pub fn expand_e_row_vec(noise_seed: &[u8; 32], i: u32, k: u32) -> Vec<i8> {
    let mut v = vec![0i8; k as usize];
    expand_e_row(noise_seed, i, k, &mut v);
    v
}
pub fn expand_f_col_vec(noise_seed: &[u8; 32], j: u32, k: u32) -> Vec<i8> {
    let mut v = vec![0i8; k as usize];
    expand_f_col(noise_seed, j, k, &mut v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinism() {
        let state = [7u8; 48];
        let a = expand_a_row_vec(&state, 3, 64);
        let b = expand_a_row_vec(&state, 3, 64);
        assert_eq!(a, b);
    }

    #[test]
    fn idx_separation() {
        let state = [7u8; 48];
        let a = expand_a_row_vec(&state, 3, 64);
        let b = expand_a_row_vec(&state, 4, 64);
        assert_ne!(a, b);
    }

    #[test]
    fn label_separation() {
        let state = [7u8; 48];
        let a = expand_a_row_vec(&state, 3, 64);
        let mut e = [0i8; 64];
        // Use a (state.try_into() — but expand_e_row expects 32-byte seed; mock one)
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&state[..32]);
        expand_e_row(&seed, 3, 64, &mut e);
        // The streams should differ (different domain-separation contexts).
        assert_ne!(&a[..], &e[..]);
    }

    #[test]
    fn state_separation() {
        let s1 = [7u8; 48];
        let s2 = [8u8; 48];
        let a = expand_a_row_vec(&s1, 3, 64);
        let b = expand_a_row_vec(&s2, 3, 64);
        assert_ne!(a, b);
    }

    #[test]
    fn distribution_is_centered() {
        // Sanity: mean of an i8-XOF stream should be near zero over many samples.
        let state = [42u8; 48];
        let mut sum: i64 = 0;
        let n = 100_000u32;
        let v = expand_a_row_vec(&state, 0, n);
        for &x in &v {
            sum += x as i64;
        }
        let mean = (sum as f64) / (n as f64);
        // |mean| should be << 1; allow 2 (very loose).
        assert!(mean.abs() < 2.0, "mean was {mean}");
    }
}
