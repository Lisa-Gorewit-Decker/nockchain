//! Prover: search for a tile of `(A+E)(B+F)` whose hardness hash falls below
//! the 256-bit target.
//!
//! Two entry points:
//! - `mine` — one nonce attempt at a time. The simple Bitcoin-style outer
//!   loop: caller varies `nonce` and retries.
//! - `mine_block` — caches the block-level noise `(E, F)` once, then sweeps
//!   a caller-supplied iterator of nonces. At LLM-scale shapes this saves
//!   noise-XOF cost (which would otherwise dominate per-attempt time).

use thiserror::Error;

use crate::commit::{merkle_path, merkle_root, MerkleError};
use crate::fiat_shamir::{block_state, challenge_indices, challenge_seed, noise_seed_for_block};
use crate::matmul::{compute_tile_split, AttemptInputs, BlockNoise};
use crate::params::{MatmulParams, ParamError};
use crate::proof::{MatmulProof, TileOpening};
use crate::tile_hash::{hash_le_target, tile_hardness_hash, tile_state_hash};

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
}

/// Compute the 32-byte tag binding a `MatmulParams` instance into the
/// transcript. Both prover and verifier compute and compare this. The label
/// is bumped to `v1.5` because Phase 1.5 changes noise derivation from
/// per-nonce to per-block; an old (`v1`) tag must not validate against the
/// new code path.
pub fn params_tag(p: &MatmulParams) -> [u8; 32] {
    crate::fiat_shamir::transcript(
        "matmul-params-v1.5",
        &[
            &p.m.to_le_bytes(),
            &p.k.to_le_bytes(),
            &p.n.to_le_bytes(),
            &p.noise_rank.to_le_bytes(),
            &p.tile.to_le_bytes(),
            &p.spot_checks.to_le_bytes(),
            &p.lambda.to_le_bytes(),
        ],
    )
}

/// Run one nonce attempt. Returns `Ok(Some(proof))` if a tile satisfies the
/// hardness target, `Ok(None)` if no tile does (the caller should retry with
/// a fresh nonce), or `Err` for parameter / structural problems.
pub fn mine(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
    opts: ProverOptions,
) -> Result<Option<MatmulProof>, MineError> {
    params.validate()?;
    let tag = params_tag(params);
    let n_seed = noise_seed_for_block(block_commitment, &tag);
    let noise = BlockNoise::expand(&n_seed, params);
    mine_with_cached_noise(block_commitment, nonce, params, target, opts, &noise, &tag)
}

/// Run the prover repeatedly over a sequence of nonces, caching the
/// block-level noise expansion across attempts. Returns the first proof
/// found or `Ok(None)` if no nonce in the iterator satisfies the target.
pub fn mine_block<I, N>(
    block_commitment: &[u8],
    nonces: I,
    params: &MatmulParams,
    target: &[u8; 32],
    opts: ProverOptions,
) -> Result<Option<MatmulProof>, MineError>
where
    I: IntoIterator<Item = N>,
    N: AsRef<[u8]>,
{
    params.validate()?;
    let tag = params_tag(params);
    let n_seed = noise_seed_for_block(block_commitment, &tag);
    let noise = BlockNoise::expand(&n_seed, params);
    for nonce in nonces {
        if let Some(proof) = mine_with_cached_noise(
            block_commitment,
            nonce.as_ref(),
            params,
            target,
            opts,
            &noise,
            &tag,
        )? {
            return Ok(Some(proof));
        }
    }
    Ok(None)
}

fn mine_with_cached_noise(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
    opts: ProverOptions,
    noise: &BlockNoise,
    tag: &[u8; 32],
) -> Result<Option<MatmulProof>, MineError> {
    let state = block_state(block_commitment, nonce);
    let inputs = AttemptInputs::expand(&state, params);

    // First pass: compute every tile's M_{i,j} hash.
    let num_tiles = params.num_tiles() as usize;
    let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(num_tiles);
    for tile_i in 0..params.row_tiles() {
        for tile_j in 0..params.col_tiles() {
            let block = compute_tile_split(noise, &inputs, params, tile_i, tile_j);
            leaves.push(tile_state_hash(&block));
        }
    }

    let comm_m = merkle_root(&leaves)?;
    let chal = challenge_seed(&state, &comm_m, tag);

    // Scan for a tile with hardness hash <= target.
    let mut found: Option<(u32, [u8; 32])> = None;
    for idx in 0..num_tiles as u32 {
        let (i, j) = params.tile_coords(idx);
        let hh = tile_hardness_hash(&chal, i, j, &leaves[idx as usize]);
        if hash_le_target(&hh, target) {
            match found {
                None => found = Some((idx, hh)),
                Some((_, ref best)) if opts.seek_best && &hh < best => {
                    found = Some((idx, hh));
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

    let spot_indices = challenge_indices(&chal, params.spot_checks, num_tiles as u32);

    let (fi, fj) = params.tile_coords(found_idx);
    let found_path = merkle_path(&leaves, found_idx as usize)?;
    let mut spot = Vec::with_capacity(spot_indices.len());
    for &idx in &spot_indices {
        let (i, j) = params.tile_coords(idx);
        let path = merkle_path(&leaves, idx as usize)?;
        spot.push(TileOpening { i, j, path });
    }

    Ok(Some(MatmulProof {
        comm_m,
        params_tag: *tag,
        found: TileOpening {
            i: fi,
            j: fj,
            path: found_path,
        },
        spot,
    }))
}
