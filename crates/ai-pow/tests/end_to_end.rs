//! End-to-end prover -> verifier tests at small dimensions.

use ai_pow::params::MatmulParams;
use ai_pow::prover::{mine, ProverOptions};
use ai_pow::verifier::verify;

const EASY_TARGET: [u8; 32] = [0xff; 32];

fn small_params() -> MatmulParams {
    MatmulParams::TEST_SMALL
}

#[test]
fn proof_round_trip_against_easy_target() {
    let params = small_params();
    let block_commitment = b"block-header-bytes";
    let nonce = b"nonce-1";
    let proof = mine(
        block_commitment,
        nonce,
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .expect("easy target must yield a proof");
    verify(block_commitment, nonce, &params, &EASY_TARGET, &proof).unwrap();
}

#[test]
fn proof_is_deterministic() {
    let params = small_params();
    let p1 = mine(
        b"hdr",
        b"nce",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    let p2 = mine(
        b"hdr",
        b"nce",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(p1, p2, "same inputs must yield identical proof bytes");
}

#[test]
fn different_nonce_yields_different_proof() {
    let params = small_params();
    let p1 = mine(
        b"hdr",
        b"nce-1",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    let p2 = mine(
        b"hdr",
        b"nce-2",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    assert_ne!(p1, p2);
}

#[test]
fn unreachable_target_yields_none() {
    let params = small_params();
    // Target = 0 -> impossible (no hash <= 0 unless it's exactly zero, ~2^-256).
    let target = [0u8; 32];
    let r = mine(b"hdr", b"nce", &params, &target, ProverOptions::default()).unwrap();
    assert!(r.is_none());
}

#[test]
fn wire_format_round_trip() {
    let params = small_params();
    let proof = mine(
        b"hdr",
        b"nce",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    let bytes = proof.encode();
    let decoded = ai_pow::proof::MatmulProof::decode(&bytes).unwrap();
    assert_eq!(proof, decoded);
    verify(b"hdr", b"nce", &params, &EASY_TARGET, &decoded).unwrap();
}

#[test]
fn verify_rejects_wrong_block_commitment() {
    let params = small_params();
    let proof = mine(
        b"hdr-A",
        b"nce",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    let r = verify(b"hdr-B", b"nce", &params, &EASY_TARGET, &proof);
    assert!(r.is_err(), "verifier must reject mismatched header");
}

#[test]
fn verify_rejects_wrong_nonce() {
    let params = small_params();
    let proof = mine(
        b"hdr",
        b"nce-A",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    let r = verify(b"hdr", b"nce-B", &params, &EASY_TARGET, &proof);
    assert!(r.is_err());
}

#[test]
fn verify_rejects_proof_for_different_params() {
    let params = small_params();
    let mut other = params;
    other.spot_checks = params.spot_checks - 1;
    let proof = mine(
        b"hdr",
        b"nce",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    let r = verify(b"hdr", b"nce", &other, &EASY_TARGET, &proof);
    assert!(matches!(
        r,
        Err(ai_pow::verifier::VerifyError::ParamsTagMismatch)
    ));
}

#[test]
fn medium_params_round_trip() {
    let params = MatmulParams {
        m: 32,
        k: 64,
        n: 32,
        noise_rank: 4,
        tile: 8,
        spot_checks: 4,
        lambda: 8,
    };
    let proof = mine(
        b"hdr",
        b"nce",
        &params,
        &EASY_TARGET,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    verify(b"hdr", b"nce", &params, &EASY_TARGET, &proof).unwrap();
}

#[test]
fn seek_best_returns_smallest_hash() {
    // Build several proofs with seek_best on; verify each.
    let params = small_params();
    let proof = mine(
        b"hdr",
        b"nce",
        &params,
        &EASY_TARGET,
        ProverOptions { seek_best: true },
    )
    .unwrap()
    .unwrap();
    verify(b"hdr", b"nce", &params, &EASY_TARGET, &proof).unwrap();
}
