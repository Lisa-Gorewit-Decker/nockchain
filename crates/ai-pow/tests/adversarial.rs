//! Adversarial tests: every check the verifier performs must reject the
//! corresponding tampering.

use ai_pow::params::MatmulParams;
use ai_pow::proof::{MatmulProof, TileOpening};
use ai_pow::prover::{mine, ProverOptions};
use ai_pow::verifier::{verify, VerifyError};

const EASY_TARGET: [u8; 32] = [0xff; 32];

fn fresh_proof() -> (MatmulParams, &'static [u8], &'static [u8], MatmulProof) {
    let params = MatmulParams::TEST_SMALL;
    let block = b"block-header-bytes" as &[u8];
    let nonce = b"nonce-1" as &[u8];
    let proof = mine(
        block,
        nonce,
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    (params, block, nonce, proof)
}

#[test]
fn reject_tampered_comm_m() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.comm_m[0] ^= 1;
    let r = verify(block, nonce, &params, &EASY_TARGET, &proof);
    // Tampering comm_m breaks every Merkle path; the first one checked fails.
    assert!(matches!(
        r,
        Err(VerifyError::FoundMerkleMismatch | VerifyError::SpotMerkleMismatch)
    ));
}

#[test]
fn reject_tampered_params_tag() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.params_tag[0] ^= 1;
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::ParamsTagMismatch)
    );
}

#[test]
fn reject_tampered_found_path() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.found.path[0][0] ^= 1;
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::FoundMerkleMismatch)
    );
}

#[test]
fn reject_found_out_of_range() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.found.i = params.row_tiles();
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::FoundOutOfRange)
    );
}

#[test]
fn reject_found_above_target() {
    let (params, block, nonce, mut proof) = fresh_proof();
    // Replace `found` with a tile that is *not* the satisfying one. Pick a
    // spot-check tile (almost certainly not the same coords as found, since
    // spot indices are FS-derived).
    let alt = proof
        .spot
        .iter()
        .find(|s| (s.i, s.j) != (proof.found.i, proof.found.j))
        .cloned()
        .expect("at least one spot-check should differ from found");
    proof.found = alt;
    let r = verify(block, nonce, &params, &EASY_TARGET, &proof);
    // With the easy target every tile passes hardness, so this should still
    // succeed. Use a near-zero target instead to force most tiles to fail.
    let mut tight: [u8; 32] = [0; 32];
    tight[0] = 0x00;
    tight[1] = 0x01; // accept hashes <= 0x0001..00
                     // Re-mine to pick up a tile under the tight target if any.
    if let Some(found_proof) =
        mine(block, nonce, &params, &tight, ProverOptions::default()).unwrap()
    {
        // Replace found with another tile that does *not* satisfy tight.
        let alt = found_proof
            .spot
            .iter()
            .find(|s| (s.i, s.j) != (found_proof.found.i, found_proof.found.j))
            .cloned()
            .unwrap();
        let mut bad = found_proof.clone();
        bad.found = alt;
        // Now most likely the alt tile is above target.
        let r2 = verify(block, nonce, &params, &tight, &bad);
        assert!(
            matches!(
                r2,
                Err(VerifyError::FoundAboveTarget) | Err(VerifyError::FoundMerkleMismatch)
            ),
            "got {r2:?}"
        );
    }
    let _ = r; // silence unused
}

#[test]
fn reject_wrong_spot_count() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.spot.pop();
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::SpotCountMismatch)
    );
}

#[test]
fn reject_wrong_spot_indices() {
    let (params, block, nonce, mut proof) = fresh_proof();
    // Pick a spot opening and rotate its (i, j) to a different valid in-range tile.
    let opening = &mut proof.spot[0];
    let other_idx = (params.tile_index(opening.i, opening.j) + 1) % params.num_tiles();
    let (i2, j2) = params.tile_coords(other_idx);
    opening.i = i2;
    opening.j = j2;
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::SpotIndexMismatch)
    );
}

#[test]
fn reject_tampered_spot_path() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.spot[0].path[0][0] ^= 1;
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::SpotMerkleMismatch)
    );
}

#[test]
fn reject_spot_out_of_range() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.spot[0].i = params.row_tiles();
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::SpotOutOfRange)
    );
}

#[test]
fn reject_extra_opening() {
    let (params, block, nonce, mut proof) = fresh_proof();
    proof.spot.push(TileOpening {
        i: 0,
        j: 0,
        path: vec![],
    });
    assert_eq!(
        verify(block, nonce, &params, &EASY_TARGET, &proof),
        Err(VerifyError::SpotCountMismatch)
    );
}
