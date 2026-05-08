//! Tiled INT8 matmul: `C' = (A + E) * (B + F)`.
//!
//! `A`, `E` are `m x k` row-major INT8.  `B`, `F` are `k x n` column-major
//! INT8 (each column is contiguous, so a single column of `B` or `F` is a
//! `k`-byte slice). The accumulator is `i32`; `params.validate()` guarantees
//! `k * 2^16` fits in `i32`.

use crate::params::MatmulParams;
use crate::prng;

/// A full set of expanded matrices for one nonce attempt.
///
/// The prover allocates this once and iterates all output tiles. Memory cost
/// is `2 * (m*k + k*n)` bytes; at the production profile (4096^3) this is
/// 64 MiB total, comfortable on any modern machine.
pub struct Matrices {
    pub m: u32,
    pub k: u32,
    pub n: u32,
    pub a: Vec<i8>, // m * k, row-major
    pub e: Vec<i8>, // m * k, row-major
    pub b: Vec<i8>, // k * n, column-major (col j at j*k .. (j+1)*k)
    pub f: Vec<i8>, // k * n, column-major
}

impl Matrices {
    pub fn expand(state: &[u8], noise_seed: &[u8; 32], params: &MatmulParams) -> Self {
        let m = params.m as usize;
        let k = params.k as usize;
        let n = params.n as usize;
        let mut a = vec![0i8; m * k];
        let mut e = vec![0i8; m * k];
        for i in 0..params.m {
            let off = (i as usize) * k;
            prng::expand_a_row(state, i, params.k, &mut a[off..off + k]);
            prng::expand_e_row(noise_seed, i, params.k, &mut e[off..off + k]);
        }
        let mut b = vec![0i8; k * n];
        let mut f = vec![0i8; k * n];
        for j in 0..params.n {
            let off = (j as usize) * k;
            prng::expand_b_col(state, j, params.k, &mut b[off..off + k]);
            prng::expand_f_col(noise_seed, j, params.k, &mut f[off..off + k]);
        }
        Self {
            m: params.m,
            k: params.k,
            n: params.n,
            a,
            e,
            b,
            f,
        }
    }

    pub fn a_row(&self, i: u32) -> &[i8] {
        let k = self.k as usize;
        let off = (i as usize) * k;
        &self.a[off..off + k]
    }
    pub fn e_row(&self, i: u32) -> &[i8] {
        let k = self.k as usize;
        let off = (i as usize) * k;
        &self.e[off..off + k]
    }
    pub fn b_col(&self, j: u32) -> &[i8] {
        let k = self.k as usize;
        let off = (j as usize) * k;
        &self.b[off..off + k]
    }
    pub fn f_col(&self, j: u32) -> &[i8] {
        let k = self.k as usize;
        let off = (j as usize) * k;
        &self.f[off..off + k]
    }
}

/// Compute the partial sum of one output tile `(tile_i, tile_j)` of size `t x t`,
/// returned as a row-major `Vec<i32>` of length `t*t`.
pub fn compute_tile(
    matrices: &Matrices,
    params: &MatmulParams,
    tile_i: u32,
    tile_j: u32,
) -> Vec<i32> {
    let t = params.tile as usize;
    let mut out = vec![0i32; t * t];
    let row0 = (tile_i * params.tile) as u32;
    let col0 = (tile_j * params.tile) as u32;
    for di in 0..t {
        let i = row0 + di as u32;
        let a_row = matrices.a_row(i);
        let e_row = matrices.e_row(i);
        for dj in 0..t {
            let j = col0 + dj as u32;
            let b_col = matrices.b_col(j);
            let f_col = matrices.f_col(j);
            out[di * t + dj] = dot_axby(a_row, e_row, b_col, f_col);
        }
    }
    out
}

/// Compute one output tile from already-extracted row-tiles and column-tiles.
/// Used by the verifier, which only re-derives the rows / columns it needs
/// rather than the full `Matrices`. All four input slices contain `t * k`
/// INT8s; rows / columns are contiguous in `k`-stride.
pub fn compute_tile_from_slices(
    a_rows: &[i8],
    e_rows: &[i8],
    b_cols: &[i8],
    f_cols: &[i8],
    tile: u32,
    k: u32,
) -> Vec<i32> {
    let t = tile as usize;
    let kk = k as usize;
    debug_assert_eq!(a_rows.len(), t * kk);
    debug_assert_eq!(e_rows.len(), t * kk);
    debug_assert_eq!(b_cols.len(), t * kk);
    debug_assert_eq!(f_cols.len(), t * kk);
    let mut out = vec![0i32; t * t];
    for di in 0..t {
        let a_row = &a_rows[di * kk..(di + 1) * kk];
        let e_row = &e_rows[di * kk..(di + 1) * kk];
        for dj in 0..t {
            let b_col = &b_cols[dj * kk..(dj + 1) * kk];
            let f_col = &f_cols[dj * kk..(dj + 1) * kk];
            out[di * t + dj] = dot_axby(a_row, e_row, b_col, f_col);
        }
    }
    out
}

#[inline]
fn dot_axby(a: &[i8], e: &[i8], b: &[i8], f: &[i8]) -> i32 {
    debug_assert_eq!(a.len(), e.len());
    debug_assert_eq!(b.len(), f.len());
    debug_assert_eq!(a.len(), b.len());
    let mut acc: i32 = 0;
    for l in 0..a.len() {
        let ax = a[l] as i32 + e[l] as i32;
        let by = b[l] as i32 + f[l] as i32;
        acc = acc.wrapping_add(ax * by);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_full(state: &[u8], seed: &[u8; 32], params: &MatmulParams) -> Vec<i32> {
        let m = params.m as usize;
        let n = params.n as usize;
        let k = params.k as usize;
        let mats = Matrices::expand(state, seed, params);
        let mut out = vec![0i32; m * n];
        for i in 0..m {
            for j in 0..n {
                let mut acc: i32 = 0;
                for l in 0..k {
                    let ax = mats.a_row(i as u32)[l] as i32 + mats.e_row(i as u32)[l] as i32;
                    let by = mats.b_col(j as u32)[l] as i32 + mats.f_col(j as u32)[l] as i32;
                    acc += ax * by;
                }
                out[i * n + j] = acc;
            }
        }
        out
    }

    #[test]
    fn tile_matches_naive() {
        let p = MatmulParams::TEST_SMALL;
        let state = b"hello".to_vec();
        let seed = [3u8; 32];
        let full = naive_full(&state, &seed, &p);
        let mats = Matrices::expand(&state, &seed, &p);
        let t = p.tile as usize;
        let n = p.n as usize;
        for tile_i in 0..p.row_tiles() {
            for tile_j in 0..p.col_tiles() {
                let block = compute_tile(&mats, &p, tile_i, tile_j);
                for di in 0..t {
                    for dj in 0..t {
                        let i = tile_i as usize * t + di;
                        let j = tile_j as usize * t + dj;
                        assert_eq!(block[di * t + dj], full[i * n + j]);
                    }
                }
            }
        }
    }

    #[test]
    fn slice_path_matches_full_path() {
        let p = MatmulParams::TEST_SMALL;
        let state = b"hello".to_vec();
        let seed = [3u8; 32];
        let mats = Matrices::expand(&state, &seed, &p);

        let tile_i = 1u32;
        let tile_j = 2u32;
        let t = p.tile as u32;
        let k = p.k as usize;
        let mut a_rows = vec![0i8; (t as usize) * k];
        let mut e_rows = vec![0i8; (t as usize) * k];
        for di in 0..t {
            let i = tile_i * t + di;
            a_rows[(di as usize) * k..(di as usize + 1) * k].copy_from_slice(mats.a_row(i));
            e_rows[(di as usize) * k..(di as usize + 1) * k].copy_from_slice(mats.e_row(i));
        }
        let mut b_cols = vec![0i8; (t as usize) * k];
        let mut f_cols = vec![0i8; (t as usize) * k];
        for dj in 0..t {
            let j = tile_j * t + dj;
            b_cols[(dj as usize) * k..(dj as usize + 1) * k].copy_from_slice(mats.b_col(j));
            f_cols[(dj as usize) * k..(dj as usize + 1) * k].copy_from_slice(mats.f_col(j));
        }
        let from_full = compute_tile(&mats, &p, tile_i, tile_j);
        let from_slices = compute_tile_from_slices(&a_rows, &e_rows, &b_cols, &f_cols, p.tile, p.k);
        assert_eq!(from_full, from_slices);
    }
}
