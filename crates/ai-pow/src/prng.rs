//! Domain-separated BLAKE3-XOF expansion for the Pearl-style PoUW input and
//! low-rank noise factors.
//!
//! Public ranges follow Pearl Whitepaper §4.1 / §4.4:
//!  * `A` (rows), `B` (columns): signed 7-bit, i.e. `[-64, 63]`. This is one
//!    value short of Pearl's `[-64, 64]` but lets us mask cleanly from a byte
//!    stream without rejection sampling; the per-multiply bound `(64+63)^2 =
//!    16129` still fits inside `k = 2^16` accumulations.
//!  * `E_L`, `F_R`: signed 6-bit, i.e. `[-32, 31]` (Pearl §4.4 verbatim).
//!  * `E_R`, `F_L`: choice matrices. Pearl §4.4 specifies that each column of
//!    `E_R` (resp. each row of `F_L`) has exactly one `+1` and one `-1` at
//!    two uniformly random distinct positions in `0..r`; all other entries
//!    are `0`.
//!
//! Each named stream is independent: changing the label or the index produces
//! an unrelated draw. Verifiers can re-derive a single row, column, or pair
//! of choice-matrix indices addressably without expanding the full noise.

use blake3::Hasher;

const CTX_A_ROW: &str = "ai-pow v2 expand A-row";
const CTX_B_COL: &str = "ai-pow v2 expand B-col";
const CTX_E_L_ROW: &str = "ai-pow v2 expand E_L row";
const CTX_E_R_COL: &str = "ai-pow v2 expand E_R col";
const CTX_F_L_ROW: &str = "ai-pow v2 expand F_L row";
const CTX_F_R_COL: &str = "ai-pow v2 expand F_R col";

fn xof(context: &str, root: &[u8], idx: u64) -> blake3::OutputReader {
    let mut hasher = Hasher::new_derive_key(context);
    hasher.update(root);
    hasher.update(&idx.to_le_bytes());
    hasher.finalize_xof()
}

fn fill_bytes(context: &str, root: &[u8], idx: u64, buf: &mut [u8]) {
    xof(context, root, idx).fill(buf);
}

/// Mask a byte into a signed 7-bit integer in `[-64, 63]`.
#[inline]
fn byte_to_i7(b: u8) -> i8 {
    ((b & 0x7F) as i32 - 64) as i8
}

/// Mask a byte into a signed 6-bit integer in `[-32, 31]`.
#[inline]
fn byte_to_i6(b: u8) -> i8 {
    ((b & 0x3F) as i32 - 32) as i8
}

/// Row `i` of the input matrix `A` (length `k`), values in `[-64, 63]`.
pub fn expand_a_row(state: &[u8], i: u32, k: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    let mut buf = vec![0u8; k as usize];
    fill_bytes(CTX_A_ROW, state, i as u64, &mut buf);
    for (o, b) in out.iter_mut().zip(buf.iter()) {
        *o = byte_to_i7(*b);
    }
}

/// Column `j` of the input matrix `B` (length `k`), values in `[-64, 63]`.
pub fn expand_b_col(state: &[u8], j: u32, k: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    let mut buf = vec![0u8; k as usize];
    fill_bytes(CTX_B_COL, state, j as u64, &mut buf);
    for (o, b) in out.iter_mut().zip(buf.iter()) {
        *o = byte_to_i7(*b);
    }
}

/// Row `i` of the noise factor `E_L` (length `r`), values in `[-32, 31]`.
pub fn expand_e_l_row(noise_seed: &[u8; 32], i: u32, r: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), r as usize);
    let mut buf = vec![0u8; r as usize];
    fill_bytes(CTX_E_L_ROW, noise_seed, i as u64, &mut buf);
    for (o, b) in out.iter_mut().zip(buf.iter()) {
        *o = byte_to_i6(*b);
    }
}

/// Column `j` of the noise factor `F_R` (length `r`), values in `[-32, 31]`.
pub fn expand_f_r_col(noise_seed: &[u8; 32], j: u32, r: u32, out: &mut [i8]) {
    debug_assert_eq!(out.len(), r as usize);
    let mut buf = vec![0u8; r as usize];
    fill_bytes(CTX_F_R_COL, noise_seed, j as u64, &mut buf);
    for (o, b) in out.iter_mut().zip(buf.iter()) {
        *o = byte_to_i6(*b);
    }
}

/// Sample two distinct indices in `0..r` from a per-(label, idx) XOF stream.
///
/// Returns `(p_plus, p_minus)`. Rejection-samples u16 chunks; the first
/// in-range value becomes `p_plus`, then the first different in-range value
/// becomes `p_minus`. Modulo bias against `r` up to `2^16` is bounded by
/// `r / 2^16 <= 1`, which is acceptable for our `r <= 2^16` regime (Pearl
/// §4.8 caps `r` at `2^10`).
fn sample_two_distinct(context: &str, root: &[u8], idx: u64, r: u32) -> (u32, u32) {
    debug_assert!(r >= 2);
    let mut reader = xof(context, root, idx);
    let mut buf = [0u8; 2];
    let mut take = || {
        reader.fill(&mut buf);
        let v = u16::from_le_bytes(buf) as u32;
        v % r
    };
    let p_plus = take();
    let mut p_minus = take();
    while p_minus == p_plus {
        p_minus = take();
    }
    (p_plus, p_minus)
}

/// `(p_plus, p_minus)` for column `j` of `E_R ∈ {-1, 0, 1}^{r × k}`.
/// `E_R[p_plus, j] = +1`, `E_R[p_minus, j] = -1`, all other entries are `0`.
pub fn e_r_col_positions(noise_seed: &[u8; 32], j: u32, r: u32) -> (u32, u32) {
    sample_two_distinct(CTX_E_R_COL, noise_seed, j as u64, r)
}

/// `(p_plus, p_minus)` for row `k_idx` of `F_L ∈ {-1, 0, 1}^{k × r}`.
/// `F_L[k_idx, p_plus] = +1`, `F_L[k_idx, p_minus] = -1`, all other entries
/// are `0`.
pub fn f_l_row_positions(noise_seed: &[u8; 32], k_idx: u32, r: u32) -> (u32, u32) {
    sample_two_distinct(CTX_F_L_ROW, noise_seed, k_idx as u64, r)
}

/// Allocate-and-expand wrappers for callers that don't have a buffer.
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
pub fn expand_e_l_row_vec(noise_seed: &[u8; 32], i: u32, r: u32) -> Vec<i8> {
    let mut v = vec![0i8; r as usize];
    expand_e_l_row(noise_seed, i, r, &mut v);
    v
}
pub fn expand_f_r_col_vec(noise_seed: &[u8; 32], j: u32, r: u32) -> Vec<i8> {
    let mut v = vec![0i8; r as usize];
    expand_f_r_col(noise_seed, j, r, &mut v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_b_in_range() {
        let state = [7u8; 48];
        let v = expand_a_row_vec(&state, 0, 4096);
        for x in &v {
            assert!(*x >= -64 && *x <= 63, "A entry {x} out of range");
        }
        let v = expand_b_col_vec(&state, 0, 4096);
        for x in &v {
            assert!(*x >= -64 && *x <= 63, "B entry {x} out of range");
        }
    }

    #[test]
    fn e_l_in_range() {
        let seed = [3u8; 32];
        let v = expand_e_l_row_vec(&seed, 0, 256);
        for x in &v {
            assert!(*x >= -32 && *x <= 31, "E_L entry {x} out of range");
        }
    }

    #[test]
    fn determinism() {
        let state = [7u8; 48];
        let a = expand_a_row_vec(&state, 3, 64);
        let b = expand_a_row_vec(&state, 3, 64);
        assert_eq!(a, b);

        let seed = [3u8; 32];
        let p1 = e_r_col_positions(&seed, 0, 16);
        let p2 = e_r_col_positions(&seed, 0, 16);
        assert_eq!(p1, p2);
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
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&state[..32]);
        let a = expand_a_row_vec(&state, 3, 64);
        let e = expand_e_l_row_vec(&seed, 3, 64);
        // Different domain-separation contexts -> different streams.
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
    fn choice_indices_distinct_and_in_range() {
        let seed = [42u8; 32];
        for j in 0..256u32 {
            let (p, m) = e_r_col_positions(&seed, j, 16);
            assert!(p < 16, "p out of range: {p}");
            assert!(m < 16, "m out of range: {m}");
            assert_ne!(p, m, "p and m must differ");
        }
    }

    #[test]
    fn choice_index_distribution_roughly_uniform() {
        // Histogram check: over many j, both p_plus and p_minus should hit
        // every position with roughly equal frequency.
        let seed = [42u8; 32];
        let r = 16u32;
        let trials = 20_000u32;
        let mut hist = vec![0u32; r as usize];
        for j in 0..trials {
            let (p, m) = e_r_col_positions(&seed, j, r);
            hist[p as usize] += 1;
            hist[m as usize] += 1;
        }
        let expected = (2 * trials) as f64 / r as f64;
        let mut max_dev = 0.0f64;
        for &h in &hist {
            let dev = ((h as f64) - expected).abs() / expected;
            if dev > max_dev {
                max_dev = dev;
            }
        }
        // 95% CI with 2500 per bucket is ~4%; allow generous slack.
        assert!(max_dev < 0.10, "max relative deviation was {max_dev}");
    }

    #[test]
    fn distribution_is_centered() {
        // Sanity: mean of an i7 XOF stream should be near `-0.5` (centered
        // on the midpoint of `[-64, 63]`).
        let state = [42u8; 48];
        let n = 100_000u32;
        let v = expand_a_row_vec(&state, 0, n);
        let mean = v.iter().map(|&x| x as i64).sum::<i64>() as f64 / n as f64;
        assert!(mean.abs() < 1.0, "mean was {mean}");
    }
}
