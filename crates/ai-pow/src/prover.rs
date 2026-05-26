//! Prover: search for a tile whose keyed BLAKE3 hash of the 512-bit Pearl
//! tile state `M_{i,j}` falls below the shape-aware target
//! `2^(256 - b) · r · t^2` (Pearl §4.5).
//!
//! The miner supplies the input matrices `A` and `B`. The prover commits
//! to them via Pearl §4.3 Alg. 2:
//!  1. `κ = derive_key("kappa", block ‖ params_tag)`
//!  2. `H_A = MerkleRoot({ row_leaf_hash(A_i, κ) }_i)`
//!  3. `H_B = MerkleRoot({ col_leaf_hash(B_j, κ) }_j)`
//!  4. `s_B = derive_key("s_b", κ ‖ H_B)`
//!  5. `s_A = derive_key("s_a", s_B ‖ H_A)`
//!
//! The matmul, noise factors, and per-tile `M` states are computed **once
//! per block** (independent of nonce). Per nonce, `pow_key = derive_key(
//! "pow-key", s_A ‖ nonce)` is mixed in and used to re-hash the cached
//! `M` states. This makes `mine_block` amortize the matmul cost across an
//! arbitrary number of nonce attempts.

use thiserror::Error;

use crate::commit::{
    a_row_leaf_hash, b_col_leaf_hash, merkle_path, merkle_root, MerkleError,
};
use crate::fiat_shamir::{
    block_state, challenge_indices, challenge_seed, commitment_key, noise_seed_a, noise_seed_b,
    pow_key_for_nonce,
};
use crate::matmul::{compute_tile, BlockNoise, Matrices, TileState};
use crate::params::{MatmulParams, ParamError};
use crate::proof::{MatmulProof, TileOpening};
use crate::tile_hash::{difficulty_target, hash_le_target};

/// Pearl §4.1 input range: `|A|, |B| <= 64`.
const INPUT_RANGE_MAX: i8 = 64;

#[derive(Debug, Clone, Copy)]
pub struct ProverOptions {
    /// If `true`, after finding a tile that satisfies hardness, continue
    /// scanning to find the smallest hash. Off by default.
    pub seek_best: bool,
}

impl Default for ProverOptions {
    fn default() -> Self {
        Self { seek_best: false }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MineError {
    #[error("invalid params: {0}")]
    Params(#[from] ParamError),
    #[error("merkle: {0}")]
    Merkle(#[from] MerkleError),
    #[error("A has wrong length: expected m*k = {expected}, got {actual}")]
    InputAShape { expected: usize, actual: usize },
    #[error("B has wrong length: expected n*k = {expected}, got {actual}")]
    InputBShape { expected: usize, actual: usize },
    #[error("input entry out of range [-64, 64]: matrix={matrix}, index={index}, value={value}")]
    InputOutOfRange {
        matrix: &'static str,
        index: usize,
        value: i8,
    },
}

/// Compute the 32-byte tag binding a `MatmulParams` instance into the
/// transcript. Both prover and verifier compute and compare this.
pub fn params_tag(p: &MatmulParams) -> [u8; 32] {
    crate::fiat_shamir::transcript(
        "matmul-params-v3",
        &[
            &p.m.to_le_bytes(),
            &p.k.to_le_bytes(),
            &p.n.to_le_bytes(),
            &p.noise_rank.to_le_bytes(),
            &p.tile.to_le_bytes(),
            &p.spot_checks.to_le_bytes(),
            &p.difficulty_bits.to_le_bytes(),
        ],
    )
}

/// Per-block precomputation: commitments to `A` and `B`, derived seeds,
/// noise factors, perturbed matrices, all `M_{i,j}` tile states, and the
/// leaves of `H_A` / `H_B` for opening-path extraction. Independent of
/// the nonce — built once per `(block_commitment, A, B)` triple.
pub struct BlockContext<'a> {
    pub a: &'a [i8],
    pub b: &'a [i8],
    pub params: MatmulParams,
    pub tag: [u8; 32],
    pub kappa: [u8; 32],
    pub h_a: [u8; 32],
    pub h_b: [u8; 32],
    /// M52 step 5: chunk-Merkle BLAKE3 keyed-hash of `pad(A)` —
    /// the commitment shape the `ai-pow-zk` SNARK binds to as
    /// public input `HASH_A`. Distinct from `h_a` (per-row
    /// Merkle, used for spot-check opening) — both coexist so
    /// the plain-side spot-check protocol AND the SNARK PI
    /// binding work without conflict.
    pub h_a_chunk: [u8; 32],
    /// M52 step 5: chunk-Merkle commitment for matrix B
    /// (column-major bytes), analogous to `h_a_chunk`.
    pub h_b_chunk: [u8; 32],
    pub s_a: [u8; 32],
    pub s_b: [u8; 32],
    pub h_a_leaves: Vec<[u8; 32]>,
    pub h_b_leaves: Vec<[u8; 32]>,
    pub m_states: Vec<TileState>,
}

impl<'a> BlockContext<'a> {
    /// Validate inputs and run the full per-block precomputation. Returns
    /// `Err` only for parameter or shape problems.
    pub fn build(
        block_commitment: &[u8],
        a: &'a [i8],
        b: &'a [i8],
        params: &MatmulParams,
    ) -> Result<Self, MineError> {
        params.validate()?;
        let m = params.m as usize;
        let k = params.k as usize;
        let n = params.n as usize;
        if a.len() != m * k {
            return Err(MineError::InputAShape {
                expected: m * k,
                actual: a.len(),
            });
        }
        if b.len() != n * k {
            return Err(MineError::InputBShape {
                expected: n * k,
                actual: b.len(),
            });
        }
        for (idx, &v) in a.iter().enumerate() {
            if v < -INPUT_RANGE_MAX || v > INPUT_RANGE_MAX {
                return Err(MineError::InputOutOfRange {
                    matrix: "A",
                    index: idx,
                    value: v,
                });
            }
        }
        for (idx, &v) in b.iter().enumerate() {
            if v < -INPUT_RANGE_MAX || v > INPUT_RANGE_MAX {
                return Err(MineError::InputOutOfRange {
                    matrix: "B",
                    index: idx,
                    value: v,
                });
            }
        }

        let tag = params_tag(params);
        let kappa = commitment_key(block_commitment, &tag);

        // Row leaves for H_A.
        let mut h_a_leaves = Vec::with_capacity(m);
        for i in 0..params.m {
            let off = (i as usize) * k;
            h_a_leaves.push(a_row_leaf_hash(&a[off..off + k], &kappa));
        }
        let h_a = merkle_root(&h_a_leaves)?;

        // Column leaves for H_B (B is column-major: col j at j*k..(j+1)*k).
        let mut h_b_leaves = Vec::with_capacity(n);
        for j in 0..params.n {
            let off = (j as usize) * k;
            h_b_leaves.push(b_col_leaf_hash(&b[off..off + k], &kappa));
        }
        let h_b = merkle_root(&h_b_leaves)?;

        // M52 step 5: chunk-Merkle commitments for SNARK PI binding.
        // BLAKE3 keyed-hash of pad(A_row_major) and pad(B_col_major).
        // i8 and u8 share layout; reinterpret without copying.
        let a_bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(a.as_ptr() as *const u8, a.len())
        };
        let b_bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(b.as_ptr() as *const u8, b.len())
        };
        let h_a_chunk = crate::commit::matrix_commitment(a_bytes, &kappa);
        let h_b_chunk = crate::commit::matrix_commitment(b_bytes, &kappa);

        let s_b = noise_seed_b(&kappa, &h_b);
        let s_a = noise_seed_a(&s_b, &h_a);

        let noise = BlockNoise::expand(&s_a, &s_b, params);
        let matrices = Matrices::build(a, b, &noise, params);

        // Pre-compute every tile's M state. These are independent of nonce.
        let num_tiles = params.num_tiles() as usize;
        let mut m_states = Vec::with_capacity(num_tiles);
        for tile_i in 0..params.row_tiles() {
            for tile_j in 0..params.col_tiles() {
                m_states.push(compute_tile(&matrices, params, tile_i, tile_j));
            }
        }

        Ok(BlockContext {
            a,
            b,
            params: *params,
            tag,
            kappa,
            h_a,
            h_b,
            h_a_chunk,
            h_b_chunk,
            s_a,
            s_b,
            h_a_leaves,
            h_b_leaves,
            m_states,
        })
    }
}

/// Run one nonce attempt with caller-supplied `A` and `B`. Returns
/// `Ok(Some(proof))` if a tile satisfies the hardness target, `Ok(None)`
/// if no tile does, or `Err` for parameter / shape / range problems.
pub fn mine(
    block_commitment: &[u8],
    nonce: &[u8],
    a: &[i8],
    b: &[i8],
    params: &MatmulParams,
    opts: ProverOptions,
) -> Result<Option<MatmulProof>, MineError> {
    let ctx = BlockContext::build(block_commitment, a, b, params)?;
    mine_with_context(&ctx, block_commitment, nonce, opts)
}

/// Run the prover repeatedly over a sequence of nonces, caching all
/// per-block computation (commitments, noise, matmul, all tile `M`
/// states). Returns the first proof found or `Ok(None)` if no nonce in
/// the iterator satisfies the target.
pub fn mine_block<I, N>(
    block_commitment: &[u8],
    nonces: I,
    a: &[i8],
    b: &[i8],
    params: &MatmulParams,
    opts: ProverOptions,
) -> Result<Option<MatmulProof>, MineError>
where
    I: IntoIterator<Item = N>,
    N: AsRef<[u8]>,
{
    let ctx = BlockContext::build(block_commitment, a, b, params)?;
    for nonce in nonces {
        if let Some(proof) = mine_with_context(&ctx, block_commitment, nonce.as_ref(), opts)? {
            return Ok(Some(proof));
        }
    }
    Ok(None)
}

/// External-target variant of the per-nonce mining attempt. The
/// chain's difficulty bound is passed explicitly rather than derived
/// from `params.difficulty_bits` — needed by `ai-pow-miner` where
/// the chain supplies an arbitrary 32-byte target (which Pearl-style
/// `difficulty_bits` cannot express precisely).
///
/// Returns `Ok(Some(proof))` if a tile clears `target`, `Ok(None)`
/// otherwise.
///
/// **The `#[cfg(feature = "zk")]` SNARK side-effect** that
/// [`mine`] / [`mine_block`] perform (calling
/// `zk_bridge::prove_and_verify_for_block`) is **NOT** invoked here:
/// that bridge re-derives the target from `params` per MED-3 and
/// would mismatch any caller-supplied `target` that doesn't equal
/// `difficulty_target(params)`. Callers wanting the SNARK should
/// drive it themselves on the returned `found` tile after ensuring
/// `params` agrees with `target`.
pub fn mine_with_context_at_target(
    ctx: &BlockContext<'_>,
    block_commitment: &[u8],
    nonce: &[u8],
    target: &[u8; 32],
    opts: ProverOptions,
) -> Result<Option<MatmulProof>, MineError> {
    Ok(mine_inner(ctx, block_commitment, nonce, target, opts)?.map(|(p, _)| p))
}

fn mine_with_context(
    ctx: &BlockContext<'_>,
    block_commitment: &[u8],
    nonce: &[u8],
    opts: ProverOptions,
) -> Result<Option<MatmulProof>, MineError> {
    let target = difficulty_target(&ctx.params);
    let result = mine_inner(ctx, block_commitment, nonce, &target, opts)?;

    // Pearl-analog ZK wrapping (preserved from the original
    // mine_with_context). The bridge re-derives target from params
    // per MED-3, so we only run it on the params-derived-target
    // path; `mine_with_context_at_target` deliberately skips it.
    #[cfg(feature = "zk")]
    if let Some((_, found_idx)) = &result {
        let _zk = crate::zk_bridge::prove_and_verify_for_block(ctx, &ctx.params, *found_idx)
            .expect("F1 zk bridge: prove + pow-verify must succeed for a found tile");
    }

    Ok(result.map(|(p, _)| p))
}

/// Shared inner per-nonce attempt — returns both the plain proof
/// and the winning linear tile index (the SNARK bridge consumes
/// `found_idx`).
fn mine_inner(
    ctx: &BlockContext<'_>,
    block_commitment: &[u8],
    nonce: &[u8],
    target: &[u8; 32],
    opts: ProverOptions,
) -> Result<Option<(MatmulProof, u32)>, MineError> {
    let params = &ctx.params;
    let state = block_state(block_commitment, nonce);
    let pow_key = pow_key_for_nonce(&ctx.s_a, nonce);

    // Per-nonce: re-hash every cached M state with the per-nonce pow_key.
    let num_tiles = params.num_tiles() as usize;
    let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(num_tiles);
    for state in &ctx.m_states {
        leaves.push(state.keyed_hash(&pow_key));
    }

    let comm_m = merkle_root(&leaves)?;
    let chal = challenge_seed(&state, &comm_m, &ctx.tag);

    let mut found: Option<(u32, [u8; 32])> = None;
    for idx in 0..num_tiles as u32 {
        let h = &leaves[idx as usize];
        if hash_le_target(h, target) {
            match found {
                None => found = Some((idx, *h)),
                Some((_, ref best)) if opts.seek_best && h < best => {
                    found = Some((idx, *h));
                }
                _ => {}
            }
            if !opts.seek_best {
                break;
            }
        }
    }
    let Some((found_idx, _)) = found else {
        return Ok(None);
    };

    let spot_indices = challenge_indices(&chal, params.spot_checks, num_tiles as u64);

    let found_opening = build_tile_opening(ctx, &leaves, found_idx.into())?;
    let mut spot = Vec::with_capacity(spot_indices.len());
    for &idx in &spot_indices {
        spot.push(build_tile_opening(ctx, &leaves, idx)?);
    }

    let plain_proof = MatmulProof {
        comm_m,
        params_tag: ctx.tag,
        h_a: ctx.h_a,
        h_b: ctx.h_b,
        h_a_chunk: ctx.h_a_chunk,
        h_b_chunk: ctx.h_b_chunk,
        found: found_opening,
        spot,
    };

    // The Pearl-analog ZK wrapping that the previous monolithic
    // `mine_with_context` performed (`zk_bridge::prove_and_verify_
    // for_block`) is intentionally NOT done here. The bridge
    // re-derives the difficulty target from `params` per MED-3, so
    // running it against an arbitrary external `target` would
    // mismatch. `mine_with_context` (the params-derived-target
    // wrapper) drives the SNARK on the returned `found_idx`
    // exactly when that mismatch is impossible.
    Ok(Some((plain_proof, found_idx)))
}

fn build_tile_opening(
    ctx: &BlockContext<'_>,
    leaves: &[[u8; 32]],
    tile_idx: u64,
) -> Result<TileOpening, MineError> {
    let params = &ctx.params;
    let (i, j) = params.tile_coords(tile_idx);
    let t = params.tile as usize;
    let k = params.k as usize;
    let row0 = (i * params.tile) as usize;
    let col0 = (j * params.tile) as usize;

    let m_path = merkle_path(leaves, tile_idx as usize)?;

    // Row strips of A: t contiguous rows, each k entries.
    let mut a_rows = Vec::with_capacity(t * k);
    let mut a_row_paths = Vec::with_capacity(t);
    for di in 0..t {
        let global_row = row0 + di;
        let off = global_row * k;
        a_rows.extend_from_slice(&ctx.a[off..off + k]);
        a_row_paths.push(merkle_path(&ctx.h_a_leaves, global_row)?);
    }

    // Column strips of B: t contiguous columns, each k entries.
    let mut b_cols = Vec::with_capacity(t * k);
    let mut b_col_paths = Vec::with_capacity(t);
    for dj in 0..t {
        let global_col = col0 + dj;
        let off = global_col * k;
        b_cols.extend_from_slice(&ctx.b[off..off + k]);
        b_col_paths.push(merkle_path(&ctx.h_b_leaves, global_col)?);
    }

    Ok(TileOpening {
        i,
        j,
        m_path,
        a_rows,
        b_cols,
        a_row_paths,
        b_col_paths,
    })
}
