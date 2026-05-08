//! Verifier: replication-style spot-check of an AI-PoW matmul proof.
//!
//! Re-derives the same matrices and noise seed the prover used, recomputes
//! the partial sum of each opened tile (the found tile and the σ
//! Fiat-Shamir-sampled spot-check tiles), and checks every Merkle opening
//! against `comm_m`. Per-block verification cost is `(σ + 1)` tile recomputes
//! plus `(σ + 1)` Merkle path checks.

use thiserror::Error;

use crate::commit::{merkle_recover_root, MerkleError};
use crate::fiat_shamir::{block_state, challenge_indices, challenge_seed, noise_seed_for_block};
use crate::matmul::compute_tile_from_slices;
use crate::params::{MatmulParams, ParamError};
use crate::prng::{expand_a_row, expand_b_col, expand_e_row, expand_f_col};
use crate::proof::MatmulProof;
use crate::prover::params_tag;
use crate::tile_hash::{hash_le_target, tile_hardness_hash, tile_state_hash};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("invalid params: {0}")]
    Params(#[from] ParamError),
    #[error("merkle: {0}")]
    Merkle(#[from] MerkleError),
    #[error("params tag in proof does not match expected")]
    ParamsTagMismatch,
    #[error("found tile coordinates out of range")]
    FoundOutOfRange,
    #[error("found tile hardness check failed")]
    FoundAboveTarget,
    #[error("found tile Merkle path does not recover comm_m")]
    FoundMerkleMismatch,
    #[error("spot check count does not match params")]
    SpotCountMismatch,
    #[error("spot check tile index does not match Fiat-Shamir derivation")]
    SpotIndexMismatch,
    #[error("spot check tile coordinates out of range")]
    SpotOutOfRange,
    #[error("spot check Merkle path does not recover comm_m")]
    SpotMerkleMismatch,
}

/// Verify a `MatmulProof` for the given block context.
pub fn verify(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
    proof: &MatmulProof,
) -> Result<(), VerifyError> {
    params.validate()?;
    let tag = params_tag(params);
    if tag != proof.params_tag {
        return Err(VerifyError::ParamsTagMismatch);
    }

    let state = block_state(block_commitment, nonce);
    let n_seed = noise_seed_for_block(block_commitment, &tag);
    let chal = challenge_seed(&state, &proof.comm_m, &tag);
    let num_tiles = params.num_tiles();

    // Range-check + recompute the found tile.
    if proof.found.i >= params.row_tiles() || proof.found.j >= params.col_tiles() {
        return Err(VerifyError::FoundOutOfRange);
    }
    let m_ij_found = recompute_m_ij(&state, &n_seed, params, proof.found.i, proof.found.j);

    let found_idx = params.tile_index(proof.found.i, proof.found.j);
    let recovered = merkle_recover_root(
        &m_ij_found, found_idx as usize, &proof.found.path, num_tiles as usize,
    )?;
    if recovered != proof.comm_m {
        return Err(VerifyError::FoundMerkleMismatch);
    }

    let hh = tile_hardness_hash(&chal, proof.found.i, proof.found.j, &m_ij_found);
    if !hash_le_target(&hh, target) {
        return Err(VerifyError::FoundAboveTarget);
    }

    // Spot checks.
    if (proof.spot.len() as u32) != params.spot_checks {
        return Err(VerifyError::SpotCountMismatch);
    }
    let expected_indices = challenge_indices(&chal, params.spot_checks, num_tiles);
    for (k, opening) in proof.spot.iter().enumerate() {
        if opening.i >= params.row_tiles() || opening.j >= params.col_tiles() {
            return Err(VerifyError::SpotOutOfRange);
        }
        let claimed_idx = params.tile_index(opening.i, opening.j);
        if claimed_idx != expected_indices[k] {
            return Err(VerifyError::SpotIndexMismatch);
        }
        let m_ij = recompute_m_ij(&state, &n_seed, params, opening.i, opening.j);
        let recovered = merkle_recover_root(
            &m_ij, claimed_idx as usize, &opening.path, num_tiles as usize,
        )?;
        if recovered != proof.comm_m {
            return Err(VerifyError::SpotMerkleMismatch);
        }
    }

    Ok(())
}

/// Recompute `M_{i,j}` for a single output tile by re-deriving the row and
/// column inputs and running one tile of the matmul.
fn recompute_m_ij(
    state: &[u8],
    n_seed: &[u8; 32],
    params: &MatmulParams,
    tile_i: u32,
    tile_j: u32,
) -> [u8; 32] {
    let t = params.tile as usize;
    let k = params.k as usize;
    let mut a_rows = vec![0i8; t * k];
    let mut e_rows = vec![0i8; t * k];
    let mut b_cols = vec![0i8; t * k];
    let mut f_cols = vec![0i8; t * k];
    let row0 = tile_i * params.tile;
    let col0 = tile_j * params.tile;
    for di in 0..t {
        let ri = row0 + di as u32;
        expand_a_row(state, ri, params.k, &mut a_rows[di * k..(di + 1) * k]);
        expand_e_row(n_seed, ri, params.k, &mut e_rows[di * k..(di + 1) * k]);
    }
    for dj in 0..t {
        let cj = col0 + dj as u32;
        expand_b_col(state, cj, params.k, &mut b_cols[dj * k..(dj + 1) * k]);
        expand_f_col(n_seed, cj, params.k, &mut f_cols[dj * k..(dj + 1) * k]);
    }
    let block = compute_tile_from_slices(&a_rows, &e_rows, &b_cols, &f_cols, params.tile, params.k);
    tile_state_hash(&block)
}
