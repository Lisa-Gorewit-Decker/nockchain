//! Verifier: replication-style spot-check of an AI-PoW matmul proof.
//!
//! For each opened tile the verifier:
//!   1. Range-checks the supplied row/column strips of `A` and `B`.
//!   2. Recomputes each row/column leaf via `a_row_leaf_hash` /
//!      `b_col_leaf_hash` and confirms its Merkle path recovers the
//!      committed `h_a` / `h_b` (Pearl §4.3 binds the noise to these).
//!   3. Reconstructs `A' = A + E` and `B' = B + F` rows / columns from the
//!      supplied strips and the re-derived noise factors.
//!   4. Runs the iterative tile loop to recompute `M_{i,j}` (Pearl §4.5).
//!   5. Keyed-hashes `M` with `pow_key = derive_key("pow-key", s_A ‖ nonce)`
//!      and checks the path against `comm_m`. For the found tile, also
//!      checks the keyed hash is `<= 2^(256-b) · r · t^2`.

use thiserror::Error;

use crate::commit::{a_row_leaf_hash, b_col_leaf_hash, merkle_recover_root, MerkleError};
use crate::fiat_shamir::{
    block_state, challenge_indices, challenge_seed, commitment_key, noise_seed_a, noise_seed_b,
    pow_key_for_nonce,
};
use crate::matmul::{compute_tile_from_slices, BlockNoise};
use crate::params::{MatmulParams, ParamError};
use crate::proof::{MatmulProof, TileOpening};
use crate::prover::params_tag;
use crate::tile_hash::{difficulty_target, hash_le_target};

const INPUT_RANGE_MAX: i8 = 64;

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
    #[error("opening A-row strip length wrong (expected {expected}, got {actual})")]
    BadAStripLen { expected: usize, actual: usize },
    #[error("opening B-col strip length wrong (expected {expected}, got {actual})")]
    BadBStripLen { expected: usize, actual: usize },
    #[error("opening has wrong number of A-row paths (expected {expected}, got {actual})")]
    BadAPathCount { expected: usize, actual: usize },
    #[error("opening has wrong number of B-col paths (expected {expected}, got {actual})")]
    BadBPathCount { expected: usize, actual: usize },
    #[error("A-row strip value out of range [-64, 64]")]
    AStripOutOfRange,
    #[error("B-col strip value out of range [-64, 64]")]
    BStripOutOfRange,
    #[error("A-row strip does not authenticate against h_a")]
    ARowMerkleMismatch,
    #[error("B-col strip does not authenticate against h_b")]
    BColMerkleMismatch,
}

/// Verify a `MatmulProof` for the given block context using
/// `difficulty_target(params)`.
///
/// Consensus callers with an externally supplied chain target must use
/// [`verify_at_target`] instead. This wrapper is retained for tests and
/// non-consensus callers whose target is intentionally derived from
/// `params.difficulty_bits`.
pub fn verify(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    proof: &MatmulProof,
) -> Result<(), VerifyError> {
    let target = difficulty_target(params);
    verify_at_target(block_commitment, nonce, params, &target, proof)
}

/// Verify a `MatmulProof` against an explicit 256-bit little-endian target.
///
/// This is the production-safe entry point for chain integration: the target
/// must be the exact target for the candidate block, not a value recomputed
/// from local proof parameters.
pub fn verify_at_target(
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

    let kappa = commitment_key(block_commitment, &tag);
    let s_b = noise_seed_b(&kappa, &proof.h_b);
    let s_a = noise_seed_a(&s_b, &proof.h_a);
    let pow_key = pow_key_for_nonce(&s_a, nonce);

    let state = block_state(block_commitment, nonce);
    let noise = BlockNoise::expand(&s_a, &s_b, params);
    let chal = challenge_seed(&state, &proof.comm_m, &tag);
    let num_tiles = params.num_tiles();

    // Found tile.
    let leaf_found = verify_opening(
        &proof.found,
        proof,
        params,
        &noise,
        &kappa,
        &pow_key,
        OpeningRole::Found,
    )?;
    if !hash_le_target(&leaf_found, target) {
        return Err(VerifyError::FoundAboveTarget);
    }

    // Spot checks.
    if (proof.spot.len() as u32) != params.spot_checks {
        return Err(VerifyError::SpotCountMismatch);
    }
    let expected_indices = challenge_indices(&chal, params.spot_checks, num_tiles);
    for (k, opening) in proof.spot.iter().enumerate() {
        // Range-check (i, j) before computing tile_index, so an
        // out-of-range coordinate reports as such rather than as an index
        // mismatch.
        if opening.i >= params.row_tiles() || opening.j >= params.col_tiles() {
            return Err(VerifyError::SpotOutOfRange);
        }
        let claimed_idx = params.tile_index(opening.i, opening.j);
        if claimed_idx != expected_indices[k] {
            return Err(VerifyError::SpotIndexMismatch);
        }
        verify_opening(
            opening,
            proof,
            params,
            &noise,
            &kappa,
            &pow_key,
            OpeningRole::Spot,
        )?;
    }

    Ok(())
}

#[derive(Copy, Clone)]
enum OpeningRole {
    Found,
    Spot,
}

/// Validate one tile opening end-to-end. Returns the keyed-hash leaf so the
/// caller can apply the per-role hardness check (only meaningful for the
/// `Found` opening).
fn verify_opening(
    opening: &TileOpening,
    proof: &MatmulProof,
    params: &MatmulParams,
    noise: &BlockNoise,
    kappa: &[u8; 32],
    pow_key: &[u8; 32],
    role: OpeningRole,
) -> Result<[u8; 32], VerifyError> {
    let t = params.tile as usize;
    let k = params.k as usize;
    let m = params.m as usize;
    let n = params.n as usize;
    let num_tiles = params.num_tiles() as usize;

    // Range checks on (i, j).
    if opening.i >= params.row_tiles() || opening.j >= params.col_tiles() {
        return Err(match role {
            OpeningRole::Found => VerifyError::FoundOutOfRange,
            OpeningRole::Spot => VerifyError::SpotOutOfRange,
        });
    }

    // Strip shape checks.
    if opening.a_rows.len() != t * k {
        return Err(VerifyError::BadAStripLen {
            expected: t * k,
            actual: opening.a_rows.len(),
        });
    }
    if opening.b_cols.len() != t * k {
        return Err(VerifyError::BadBStripLen {
            expected: t * k,
            actual: opening.b_cols.len(),
        });
    }
    if opening.a_row_paths.len() != t {
        return Err(VerifyError::BadAPathCount {
            expected: t,
            actual: opening.a_row_paths.len(),
        });
    }
    if opening.b_col_paths.len() != t {
        return Err(VerifyError::BadBPathCount {
            expected: t,
            actual: opening.b_col_paths.len(),
        });
    }

    // Strip value range checks (Pearl §4.1).
    for &v in &opening.a_rows {
        if v < -INPUT_RANGE_MAX || v > INPUT_RANGE_MAX {
            return Err(VerifyError::AStripOutOfRange);
        }
    }
    for &v in &opening.b_cols {
        if v < -INPUT_RANGE_MAX || v > INPUT_RANGE_MAX {
            return Err(VerifyError::BStripOutOfRange);
        }
    }

    // Authenticate each row strip against h_a.
    let row0 = (opening.i * params.tile) as usize;
    let col0 = (opening.j * params.tile) as usize;
    for di in 0..t {
        let row = &opening.a_rows[di * k..(di + 1) * k];
        let leaf = a_row_leaf_hash(row, kappa);
        let recovered = merkle_recover_root(&leaf, row0 + di, &opening.a_row_paths[di], m)?;
        if recovered != proof.h_a {
            return Err(VerifyError::ARowMerkleMismatch);
        }
    }
    for dj in 0..t {
        let col = &opening.b_cols[dj * k..(dj + 1) * k];
        let leaf = b_col_leaf_hash(col, kappa);
        let recovered = merkle_recover_root(&leaf, col0 + dj, &opening.b_col_paths[dj], n)?;
        if recovered != proof.h_b {
            return Err(VerifyError::BColMerkleMismatch);
        }
    }

    // Reconstruct A' rows and B' cols by adding the noise.
    let mut a_prime_rows = vec![0i8; t * k];
    let mut e_buf = vec![0i8; k];
    for di in 0..t {
        let ri = row0 + di;
        noise.e_row_into(ri as u32, &mut e_buf);
        let a_row = &opening.a_rows[di * k..(di + 1) * k];
        for l in 0..k {
            a_prime_rows[di * k + l] = (a_row[l] as i16 + e_buf[l] as i16) as i8;
        }
    }
    let mut b_prime_cols = vec![0i8; t * k];
    let mut f_buf = vec![0i8; k];
    for dj in 0..t {
        let cj = col0 + dj;
        noise.f_col_into(cj as u32, &mut f_buf);
        let b_col = &opening.b_cols[dj * k..(dj + 1) * k];
        for l in 0..k {
            b_prime_cols[dj * k + l] = (b_col[l] as i16 + f_buf[l] as i16) as i8;
        }
    }

    // Iterative Pearl §4.5 tile loop.
    let m_state = compute_tile_from_slices(&a_prime_rows, &b_prime_cols, params);
    let leaf = m_state.keyed_hash(pow_key);

    let tile_idx = params.tile_index(opening.i, opening.j) as usize;
    let recovered = merkle_recover_root(&leaf, tile_idx, &opening.m_path, num_tiles)?;
    if recovered != proof.comm_m {
        return Err(match role {
            OpeningRole::Found => VerifyError::FoundMerkleMismatch,
            OpeningRole::Spot => VerifyError::SpotMerkleMismatch,
        });
    }
    Ok(leaf)
}
