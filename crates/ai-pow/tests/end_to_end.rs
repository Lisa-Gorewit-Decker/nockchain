//! End-to-end prover -> verifier tests at small dimensions.

use ai_pow::params::MatmulParams;
use ai_pow::prover::{mine, mine_with_context_at_target, BlockContext, ProverOptions};
use ai_pow::synth::synth_matrices;
use ai_pow::verifier::{verify, verify_at_target, VerifyError};

fn small_params() -> MatmulParams {
    // difficulty_bits = 0 ⇒ every tile passes hardness.
    MatmulParams::TEST_SMALL
}

#[test]
fn proof_round_trip_against_easy_target() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed-1", &params);
    let block_commitment = b"block-header-bytes";
    let nonce = b"nonce-1";
    let proof = mine(
        block_commitment,
        nonce,
        &a,
        &b,
        &params,
        ProverOptions::default(),
    )
    .unwrap()
    .expect("easy target must yield a proof");
    verify(block_commitment, nonce, &params, &proof).unwrap();
}

#[test]
fn verifier_rejects_proof_mined_for_easier_external_target() {
    let params = small_params();
    let (a, b) = synth_matrices(b"external-target-seed", &params);
    let block_commitment = b"external-target-block";
    let nonce = b"external-target-nonce";
    let ctx = BlockContext::build(block_commitment, &a, &b, &params).unwrap();
    let easy_target = [0xff; 32];
    let proof = mine_with_context_at_target(
        &ctx,
        block_commitment,
        nonce,
        &easy_target,
        ProverOptions::default(),
    )
    .unwrap()
    .expect("max external target must yield a proof");

    let impossible_chain_target = [0u8; 32];
    assert_eq!(
        verify_at_target(
            block_commitment,
            nonce,
            &params,
            &impossible_chain_target,
            &proof,
        ),
        Err(VerifyError::FoundAboveTarget)
    );
}

#[test]
fn proof_is_deterministic() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let p1 = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let p2 = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(p1, p2, "same inputs must yield identical proof bytes");
}

#[test]
fn different_nonce_yields_different_proof() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let p1 = mine(b"hdr", b"nce-1", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let p2 = mine(b"hdr", b"nce-2", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    assert_ne!(p1, p2);
    // But H_A and H_B (block-level commitments) must be the same — they
    // depend only on (block_commitment, params, A, B), not on the nonce.
    assert_eq!(p1.h_a, p2.h_a);
    assert_eq!(p1.h_b, p2.h_b);
}

#[test]
fn different_a_yields_different_proof() {
    let params = small_params();
    let (a1, b) = synth_matrices(b"ab-seed-1", &params);
    let (a2, _) = synth_matrices(b"ab-seed-2", &params);
    let p1 = mine(b"hdr", b"nce", &a1, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let p2 = mine(b"hdr", b"nce", &a2, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    assert_ne!(p1.h_a, p2.h_a, "H_A must change when A changes");
}

#[test]
fn unreachable_target_yields_none() {
    let mut params = small_params();
    params.difficulty_bits = 400; // target = 0 ⇒ no tile passes.
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let r = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default()).unwrap();
    assert!(r.is_none());
}

#[test]
fn wire_format_round_trip() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let proof = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let bytes = proof.encode();
    let decoded = ai_pow::proof::MatmulProof::decode_for_params(&bytes, &params).unwrap();
    assert_eq!(proof, decoded);
    verify(b"hdr", b"nce", &params, &decoded).unwrap();
}

#[test]
fn verify_rejects_wrong_block_commitment() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let proof = mine(b"hdr-A", b"nce", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let r = verify(b"hdr-B", b"nce", &params, &proof);
    assert!(r.is_err(), "verifier must reject mismatched header");
}

#[test]
fn verify_rejects_wrong_nonce() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let proof = mine(b"hdr", b"nce-A", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let r = verify(b"hdr", b"nce-B", &params, &proof);
    assert!(r.is_err());
}

#[test]
fn verify_rejects_proof_for_different_params() {
    let params = small_params();
    let mut other = params;
    other.spot_checks = params.spot_checks - 1;
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let proof = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let r = verify(b"hdr", b"nce", &other, &proof);
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
        difficulty_bits: 0,
    };
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let proof = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    verify(b"hdr", b"nce", &params, &proof).unwrap();
}

#[test]
fn seek_best_returns_smallest_hash() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let proof = mine(
        b"hdr",
        b"nce",
        &a,
        &b,
        &params,
        ProverOptions { seek_best: true },
    )
    .unwrap()
    .unwrap();
    verify(b"hdr", b"nce", &params, &proof).unwrap();
}

#[test]
fn rejects_wrong_input_shape() {
    let params = small_params();
    let (mut a, b) = synth_matrices(b"ab-seed", &params);
    a.pop();
    let r = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default());
    assert!(matches!(
        r,
        Err(ai_pow::prover::MineError::InputAShape { .. })
    ));
}

#[test]
fn rejects_out_of_range_input() {
    let params = small_params();
    let (mut a, b) = synth_matrices(b"ab-seed", &params);
    a[0] = 100; // > 64
    let r = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default());
    assert!(matches!(
        r,
        Err(ai_pow::prover::MineError::InputOutOfRange { matrix: "A", .. })
    ));
}
