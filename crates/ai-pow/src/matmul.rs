//! Pearl-style tiled INT8 matmul with low-rank noise and iterative tile state.
//!
//! Computes `(A + E) * (B + F)` where the noise factors are low-rank:
//!   `E = E_L · E_R`,  `F = F_L · F_R`
//! with `E_L ∈ i6^{m×r}`, `E_R ∈ {-1,0,1}^{r×k}` choice matrix,
//! `F_L ∈ {-1,0,1}^{k×r}` choice matrix, `F_R ∈ i6^{r×n}` (Pearl §4.4).
//!
//! The product is computed tile-by-tile in stripes of width `r` along the
//! `k`-axis (Pearl §4.5 Alg. 4). At each stripe boundary the 16 × `int32`
//! state `M_{i,j}` is folded with the XOR of all accumulator entries:
//!
//!   M[ℓ mod 16] ← (M[ℓ mod 16] ≪ 13) ⊕ X_ℓ
//!
//! and the final per-tile state is keyed-BLAKE3-hashed to produce both the
//! Merkle leaf and the hardness check value (Pearl §4.5 lines 14-16).

use blake3::Hasher;

use crate::params::MatmulParams;
use crate::prng;

const CTX_TILE_HASH: &str = "ai-pow v2 tile-hash";

/// Per-block low-rank noise factors. Derived once per `block_commitment`
/// (via `noise_seed`) and reused across all nonce attempts in that block.
pub struct BlockNoise {
    pub m: u32,
    pub k: u32,
    pub n: u32,
    pub r: u32,
    /// `E_L` row-major: `m × r`. `E_L[i, p] = e_l[i*r + p]`.
    pub e_l: Vec<i8>,
    /// Per-column choice positions of `E_R`. `e_r_pos[l] = (p_plus, p_minus)`
    /// means column `l` of `E_R` has a `+1` at `p_plus` and a `-1` at `p_minus`.
    pub e_r_pos: Vec<(u32, u32)>,
    /// `F_R` stored col-major: `n × r` with column `j` at `j*r..(j+1)*r`,
    /// so `F_R[p, j] = f_r[j*r + p]`.
    pub f_r: Vec<i8>,
    /// Per-row choice positions of `F_L`. `f_l_pos[l] = (p_plus, p_minus)`.
    pub f_l_pos: Vec<(u32, u32)>,
}

impl BlockNoise {
    /// Build the noise factors per Pearl Alg. 1: `(E_L, E_R)` are keyed by
    /// `s_a`, `(F_R, F_L)` are keyed by `s_b`. The two seeds are derived in
    /// `fiat_shamir.rs` from `(block_commitment, params_tag, H_A, H_B)`.
    pub fn expand(s_a: &[u8; 32], s_b: &[u8; 32], params: &MatmulParams) -> Self {
        let m = params.m as usize;
        let k = params.k as usize;
        let n = params.n as usize;
        let r = params.noise_rank as usize;

        let mut e_l = vec![0i8; m * r];
        for i in 0..params.m {
            let off = (i as usize) * r;
            prng::expand_e_l_row(s_a, i, params.noise_rank, &mut e_l[off..off + r]);
        }
        let mut e_r_pos = Vec::with_capacity(k);
        for l in 0..params.k {
            e_r_pos.push(prng::e_r_col_positions(s_a, l, params.noise_rank));
        }
        let mut f_r = vec![0i8; n * r];
        for j in 0..params.n {
            let off = (j as usize) * r;
            prng::expand_f_r_col(s_b, j, params.noise_rank, &mut f_r[off..off + r]);
        }
        let mut f_l_pos = Vec::with_capacity(k);
        for k_idx in 0..params.k {
            f_l_pos.push(prng::f_l_row_positions(s_b, k_idx, params.noise_rank));
        }

        Self {
            m: params.m,
            k: params.k,
            n: params.n,
            r: params.noise_rank,
            e_l,
            e_r_pos,
            f_r,
            f_l_pos,
        }
    }

    /// Reconstruct row `i` of `E` (length `k`) into `out`.
    pub fn e_row_into(&self, i: u32, out: &mut [i8]) {
        let r = self.r as usize;
        let k = self.k as usize;
        debug_assert_eq!(out.len(), k);
        let row_off = (i as usize) * r;
        let e_l_row = &self.e_l[row_off..row_off + r];
        for (l, slot) in out.iter_mut().enumerate() {
            let (pp, pm) = self.e_r_pos[l];
            *slot = e_l_row[pp as usize] - e_l_row[pm as usize];
        }
    }

    /// Reconstruct column `j` of `F` (length `k`) into `out`.
    pub fn f_col_into(&self, j: u32, out: &mut [i8]) {
        let r = self.r as usize;
        let k = self.k as usize;
        debug_assert_eq!(out.len(), k);
        let col_off = (j as usize) * r;
        let f_r_col = &self.f_r[col_off..col_off + r];
        for (l, slot) in out.iter_mut().enumerate() {
            let (pp, pm) = self.f_l_pos[l];
            *slot = f_r_col[pp as usize] - f_r_col[pm as usize];
        }
    }
}

/// Per-attempt perturbed matrices `A' = A + E` (row-major) and `B' = B + F`
/// (column-major). Each entry fits an `i8`: with `|A| <= 64` and `|E| <= 63`
/// (Pearl §4.1/§4.4), `|A + E| <= 127`. Constructed once per nonce attempt
/// and reused across all output tiles.
pub struct Matrices {
    pub m: u32,
    pub k: u32,
    pub n: u32,
    /// `A' = A + E`, row-major: row `i` at `i*k..(i+1)*k`.
    pub a_prime: Vec<i8>,
    /// `B' = B + F`, column-major: column `j` at `j*k..(j+1)*k`.
    pub b_prime: Vec<i8>,
}

impl Matrices {
    /// Build `A' = A + E` (row-major) and `B' = B + F` (column-major) from
    /// caller-supplied input matrices and per-block noise factors.
    /// `a` is `m * k` row-major; `b` is `n * k` column-major (column `j` at
    /// `j*k..(j+1)*k`).
    pub fn build(a: &[i8], b: &[i8], noise: &BlockNoise, params: &MatmulParams) -> Self {
        let m = params.m as usize;
        let k = params.k as usize;
        let n = params.n as usize;
        assert_eq!(a.len(), m * k, "A length mismatch");
        assert_eq!(b.len(), n * k, "B length mismatch");

        let mut a_prime = vec![0i8; m * k];
        let mut row_buf_e = vec![0i8; k];
        for i in 0..params.m {
            noise.e_row_into(i, &mut row_buf_e);
            let off = (i as usize) * k;
            for l in 0..k {
                // |A + E| <= 64 + 63 = 127, fits i8.
                a_prime[off + l] = (a[off + l] as i16 + row_buf_e[l] as i16) as i8;
            }
        }
        let mut b_prime = vec![0i8; k * n];
        let mut col_buf_f = vec![0i8; k];
        for j in 0..params.n {
            noise.f_col_into(j, &mut col_buf_f);
            let off = (j as usize) * k;
            for l in 0..k {
                b_prime[off + l] = (b[off + l] as i16 + col_buf_f[l] as i16) as i8;
            }
        }
        Self {
            m: params.m,
            k: params.k,
            n: params.n,
            a_prime,
            b_prime,
        }
    }

    pub fn a_prime_row(&self, i: u32) -> &[i8] {
        let k = self.k as usize;
        let off = (i as usize) * k;
        &self.a_prime[off..off + k]
    }
    pub fn b_prime_col(&self, j: u32) -> &[i8] {
        let k = self.k as usize;
        let off = (j as usize) * k;
        &self.b_prime[off..off + k]
    }
}

/// 512-bit per-tile state `M_{i,j}` as 16 `int32` slots (Pearl §4.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileState(pub [i32; 16]);

impl TileState {
    pub const fn zero() -> Self {
        Self([0; 16])
    }

    /// Fold stripe `step`'s int32-XOR `x` into the rolling state, matching
    /// Pearl §4.5: `M[step mod 16] ← (M[step mod 16] ≪ 13) ⊕ x`.
    #[inline]
    pub fn fold(&mut self, step: u32, x: i32) {
        let slot = (step as usize) % 16;
        let rotated = (self.0[slot] as u32).rotate_left(13) as i32;
        self.0[slot] = rotated ^ x;
    }

    /// Keyed BLAKE3 hash of the state, used as both the Merkle leaf and the
    /// hardness check value (Pearl §4.5 line 16).
    pub fn keyed_hash(&self, pow_key: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Hasher::new_keyed(pow_key);
        // Domain-separate from any other `new_keyed` callers in this crate.
        hasher.update(CTX_TILE_HASH.as_bytes());
        for v in &self.0 {
            hasher.update(&v.to_le_bytes());
        }
        *hasher.finalize().as_bytes()
    }
}

/// Compute one output tile `(tile_i, tile_j)`, stepping in stripes of width
/// `r` along the `k`-axis and folding each stripe's XOR into the 512-bit
/// state `M` per Pearl §4.5. Returns the final `M` state; the caller derives
/// the keyed hash via `TileState::keyed_hash`.
pub fn compute_tile(
    matrices: &Matrices,
    params: &MatmulParams,
    tile_i: u32,
    tile_j: u32,
) -> TileState {
    let t = params.tile as usize;
    let r = params.noise_rank as usize;
    let row0 = (tile_i * params.tile) as usize;
    let col0 = (tile_j * params.tile) as usize;
    let steps = params.num_stripes() as usize;

    let mut c_blk = vec![0i32; t * t];
    let mut state = TileState::zero();

    for step in 0..steps {
        let lo = step * r;
        // Inner per-stripe r-wide multiply, accumulated into c_blk.
        for di in 0..t {
            let a_row = &matrices.a_prime_row((row0 + di) as u32)[lo..lo + r];
            for dj in 0..t {
                let b_col = &matrices.b_prime_col((col0 + dj) as u32)[lo..lo + r];
                let mut delta: i32 = 0;
                for l in 0..r {
                    delta = delta.wrapping_add((a_row[l] as i32) * (b_col[l] as i32));
                }
                let idx = di * t + dj;
                c_blk[idx] = c_blk[idx].wrapping_add(delta);
            }
        }
        // Fold the int32-XOR of the running accumulator into M (Pearl §4.5).
        let mut x: i32 = 0;
        for &v in &c_blk {
            x ^= v;
        }
        state.fold(step as u32, x);
    }
    state
}

/// Verifier-side compute_tile: same iterative accumulator as `compute_tile`
/// but takes already-extracted row-strips of `A'` and column-strips of `B'`.
/// `a_prime_rows` is `t * k` i8s (row-major over the `t` tile rows);
/// `b_prime_cols` is `t * k` i8s (column-major over the `t` tile columns).
pub fn compute_tile_from_slices(
    a_prime_rows: &[i8],
    b_prime_cols: &[i8],
    params: &MatmulParams,
) -> TileState {
    let t = params.tile as usize;
    let r = params.noise_rank as usize;
    let k = params.k as usize;
    debug_assert_eq!(a_prime_rows.len(), t * k);
    debug_assert_eq!(b_prime_cols.len(), t * k);
    let steps = params.num_stripes() as usize;

    let mut c_blk = vec![0i32; t * t];
    let mut state = TileState::zero();

    for step in 0..steps {
        let lo = step * r;
        for di in 0..t {
            let a_row = &a_prime_rows[di * k + lo..di * k + lo + r];
            for dj in 0..t {
                let b_col = &b_prime_cols[dj * k + lo..dj * k + lo + r];
                let mut delta: i32 = 0;
                for l in 0..r {
                    delta = delta.wrapping_add((a_row[l] as i32) * (b_col[l] as i32));
                }
                let idx = di * t + dj;
                c_blk[idx] = c_blk[idx].wrapping_add(delta);
            }
        }
        let mut x: i32 = 0;
        for &v in &c_blk {
            x ^= v;
        }
        state.fold(step as u32, x);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_ab(seed: u8, params: &MatmulParams) -> (Vec<i8>, Vec<i8>) {
        let m = params.m as usize;
        let k = params.k as usize;
        let n = params.n as usize;
        let mut a = vec![0i8; m * k];
        let mut b = vec![0i8; n * k];
        let state = [seed; 16];
        for i in 0..params.m {
            let off = (i as usize) * k;
            prng::expand_a_row(&state, i, params.k, &mut a[off..off + k]);
        }
        for j in 0..params.n {
            let off = (j as usize) * k;
            prng::expand_b_col(&state, j, params.k, &mut b[off..off + k]);
        }
        (a, b)
    }

    #[test]
    fn block_noise_e_row_matches_full_product() {
        // E[i, l] = sum_p E_L[i, p] * E_R[p, l]. With E_R sparse (one +1 at
        // pp, one -1 at pm), E[i, l] = E_L[i, pp] - E_L[i, pm]. Confirm.
        let params = MatmulParams::TEST_SMALL;
        params.validate().unwrap();
        let s_a = [7u8; 32];
        let s_b = [8u8; 32];
        let noise = BlockNoise::expand(&s_a, &s_b, &params);
        let k = params.k as usize;
        let r = params.noise_rank as usize;

        let mut e_row = vec![0i8; k];
        for i in 0..params.m {
            noise.e_row_into(i, &mut e_row);
            for l in 0..k {
                let (pp, pm) = noise.e_r_pos[l];
                let row_off = (i as usize) * r;
                let expected = noise.e_l[row_off + pp as usize] - noise.e_l[row_off + pm as usize];
                assert_eq!(e_row[l], expected);
            }
        }
    }

    #[test]
    fn perturbed_matrices_fit_i8() {
        // `|A + E| <= 64 + 63 = 127` should always hold.
        let params = MatmulParams::TEST_SMALL;
        let s_a = [3u8; 32];
        let s_b = [4u8; 32];
        let noise = BlockNoise::expand(&s_a, &s_b, &params);
        let (a, b) = synth_ab(7, &params);
        let m = Matrices::build(&a, &b, &noise, &params);
        // i8 already constrains [-128, 127]; verify the tighter `|v| <= 127`
        // bound that follows from |A| <= 64 and |E| <= 63.
        for &v in &m.a_prime {
            assert!(v >= -127, "a_prime entry below -127: {v}");
        }
        for &v in &m.b_prime {
            assert!(v >= -127, "b_prime entry below -127: {v}");
        }
    }

    fn naive_full_product(
        a: &[i8],
        b: &[i8],
        s_a: &[u8; 32],
        s_b: &[u8; 32],
        params: &MatmulParams,
    ) -> Vec<i32> {
        let m = params.m as usize;
        let n = params.n as usize;
        let k = params.k as usize;
        let noise = BlockNoise::expand(s_a, s_b, params);
        let mats = Matrices::build(a, b, &noise, params);
        let mut out = vec![0i32; m * n];
        for i in 0..m {
            let a_row = mats.a_prime_row(i as u32);
            for j in 0..n {
                let b_col = mats.b_prime_col(j as u32);
                let mut acc: i32 = 0;
                for l in 0..k {
                    acc = acc.wrapping_add((a_row[l] as i32) * (b_col[l] as i32));
                }
                out[i * n + j] = acc;
            }
        }
        out
    }

    /// Recompute the final c_blk from the iterative tile loop, side-channeled
    /// out for cross-checking against the naive full product.
    fn iterative_tile_c_blk(
        matrices: &Matrices,
        params: &MatmulParams,
        tile_i: u32,
        tile_j: u32,
    ) -> Vec<i32> {
        let t = params.tile as usize;
        let r = params.noise_rank as usize;
        let row0 = (tile_i * params.tile) as usize;
        let col0 = (tile_j * params.tile) as usize;
        let mut c_blk = vec![0i32; t * t];
        let steps = params.num_stripes() as usize;
        for step in 0..steps {
            let lo = step * r;
            for di in 0..t {
                let a_row = &matrices.a_prime_row((row0 + di) as u32)[lo..lo + r];
                for dj in 0..t {
                    let b_col = &matrices.b_prime_col((col0 + dj) as u32)[lo..lo + r];
                    let mut delta: i32 = 0;
                    for l in 0..r {
                        delta = delta.wrapping_add((a_row[l] as i32) * (b_col[l] as i32));
                    }
                    let idx = di * t + dj;
                    c_blk[idx] = c_blk[idx].wrapping_add(delta);
                }
            }
        }
        c_blk
    }

    #[test]
    fn iterative_c_blk_matches_naive_full_product() {
        let params = MatmulParams::TEST_SMALL;
        let s_a = [3u8; 32];
        let s_b = [4u8; 32];
        let noise = BlockNoise::expand(&s_a, &s_b, &params);
        let (a, b) = synth_ab(11, &params);
        let mats = Matrices::build(&a, &b, &noise, &params);
        let full = naive_full_product(&a, &b, &s_a, &s_b, &params);
        let t = params.tile as usize;
        let n = params.n as usize;
        for tile_i in 0..params.row_tiles() {
            for tile_j in 0..params.col_tiles() {
                let block = iterative_tile_c_blk(&mats, &params, tile_i, tile_j);
                for di in 0..t {
                    for dj in 0..t {
                        let i = tile_i as usize * t + di;
                        let j = tile_j as usize * t + dj;
                        assert_eq!(
                            block[di * t + dj],
                            full[i * n + j],
                            "mismatch at ({tile_i},{tile_j})[{di},{dj}]"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn slice_path_matches_full_path() {
        let params = MatmulParams::TEST_SMALL;
        let s_a = [3u8; 32];
        let s_b = [4u8; 32];
        let noise = BlockNoise::expand(&s_a, &s_b, &params);
        let (a, b) = synth_ab(11, &params);
        let mats = Matrices::build(&a, &b, &noise, &params);

        let tile_i = 1u32;
        let tile_j = 2u32;
        let t = params.tile as usize;
        let k = params.k as usize;
        let row0 = (tile_i * params.tile) as usize;
        let col0 = (tile_j * params.tile) as usize;

        let mut a_rows = vec![0i8; t * k];
        for di in 0..t {
            a_rows[di * k..(di + 1) * k].copy_from_slice(mats.a_prime_row((row0 + di) as u32));
        }
        let mut b_cols = vec![0i8; t * k];
        for dj in 0..t {
            b_cols[dj * k..(dj + 1) * k].copy_from_slice(mats.b_prime_col((col0 + dj) as u32));
        }

        let from_full = compute_tile(&mats, &params, tile_i, tile_j);
        let from_slices = compute_tile_from_slices(&a_rows, &b_cols, &params);
        assert_eq!(from_full, from_slices);
    }

    #[test]
    fn tile_state_fold_depends_on_step_order() {
        // Two folds into the same slot must depend on order: the rotation
        // happens between them, so swapping the inputs changes the result.
        let mut s1 = TileState::zero();
        let mut s2 = TileState::zero();
        s1.fold(0, 0x1234_5678);
        s1.fold(16, 0x0bad_f00d); // back to slot 0 — rotation is applied first
        s2.fold(0, 0x0bad_f00d);
        s2.fold(16, 0x1234_5678);
        assert_ne!(s1, s2);
    }

    #[test]
    fn keyed_hash_depends_on_pow_key() {
        let mut s = TileState::zero();
        s.fold(0, 0x1234_5678);
        let h1 = s.keyed_hash(&[1u8; 32]);
        let h2 = s.keyed_hash(&[2u8; 32]);
        assert_ne!(h1, h2);
    }
}
