//! End-to-end prover -> verifier tests at small dimensions.

use ai_pow::ncmn::{build_ncmn_nonce, NonceAnchors, NonceFormatError};
use ai_pow::params::MatmulParams;
use ai_pow::prover::{mine, mine_with_context_at_target, BlockContext, MineError, ProverOptions};
use ai_pow::synth::synth_matrices;
use ai_pow::verifier::{verify, verify_at_target, verify_ncmn_at_target_structural, VerifyError};

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
    let ctx = BlockContext::build(block_commitment, nonce, &a, &b, &params).unwrap();
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
        verify_at_target(block_commitment, nonce, &params, &impossible_chain_target, &proof,),
        Err(VerifyError::FoundAboveTarget)
    );
}

#[test]
fn ncmn_verifier_enforces_nonce_block_anchor() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ncmn-anchor-seed", &params);
    let puzzle_id = b"ncmn-puzzle-id";
    let nck_commitment = [0x4eu8; 32];
    let nonce = build_ncmn_nonce(&NonceAnchors::nck_only(nck_commitment), 7);
    let ctx = BlockContext::build(puzzle_id, &nonce, &a, &b, &params).unwrap();
    let target = [0xff; 32];
    let proof =
        mine_with_context_at_target(&ctx, puzzle_id, &nonce, &target, ProverOptions::default())
            .unwrap()
            .expect("max target must yield a proof");

    verify_ncmn_at_target_structural(puzzle_id, &nck_commitment, &nonce, &params, &target, &proof)
        .expect("honest NCMN nonce must verify");

    let mut wrong_anchor = nck_commitment;
    wrong_anchor[0] ^= 1;
    assert_eq!(
        verify_ncmn_at_target_structural(
            puzzle_id, &wrong_anchor, &nonce, &params, &target, &proof
        ),
        Err(VerifyError::NonceAnchorMismatch)
    );

    let mut bad_magic = nonce;
    bad_magic[0] = b'X';
    assert_eq!(
        verify_ncmn_at_target_structural(
            puzzle_id, &nck_commitment, &bad_magic, &params, &target, &proof
        ),
        Err(VerifyError::Nonce(NonceFormatError::BadMagic(*b"XCMN")))
    );

    let external_nonce = build_ncmn_nonce(
        &NonceAnchors {
            nck_commitment,
            external_commitment: Some([0x77u8; 32]),
        },
        7,
    );
    assert_eq!(
        verify_ncmn_at_target_structural(
            puzzle_id, &nck_commitment, &external_nonce, &params, &target, &proof,
        ),
        Err(VerifyError::NonceExternalCommitmentPresent)
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
fn nonce_changes_commitments_noise_and_tile_states_before_final_hashing() {
    let params = small_params();
    let (a, b) = synth_matrices(b"nonce-bound-context-seed", &params);
    let block_commitment = b"nonce-bound-block";
    let nonce_a = b"nonce-a";
    let nonce_b = b"nonce-b";

    let ctx_a = BlockContext::build(block_commitment, nonce_a, &a, &b, &params).unwrap();
    let ctx_b = BlockContext::build(block_commitment, nonce_b, &a, &b, &params).unwrap();

    assert_ne!(ctx_a.attempt_state, ctx_b.attempt_state);
    assert_ne!(ctx_a.kappa, ctx_b.kappa, "nonce must change kappa");
    assert_ne!(ctx_a.h_a, ctx_b.h_a, "nonce-bound kappa must re-key H_A");
    assert_ne!(ctx_a.h_b, ctx_b.h_b, "nonce-bound kappa must re-key H_B");
    assert_ne!(
        ctx_a.h_a_chunk, ctx_b.h_a_chunk,
        "nonce-bound kappa must re-key chunk H_A"
    );
    assert_ne!(
        ctx_a.h_b_chunk, ctx_b.h_b_chunk,
        "nonce-bound kappa must re-key chunk H_B"
    );
    assert_ne!(ctx_a.s_a, ctx_b.s_a, "nonce must change s_A");
    assert_ne!(ctx_a.s_b, ctx_b.s_b, "nonce must change s_B");
    assert_eq!(ctx_a.m_states.len(), ctx_b.m_states.len());
    assert!(
        ctx_a
            .m_states
            .iter()
            .zip(ctx_b.m_states.iter())
            .any(|(a, b)| a != b),
        "nonce must change the matmul-derived tile states, not only the final hash key"
    );
}

#[test]
fn stale_attempt_context_cannot_be_reused_for_another_nonce() {
    let params = small_params();
    let (a, b) = synth_matrices(b"stale-context-seed", &params);
    let block_commitment = b"stale-context-block";
    let nonce_a = b"stale-nonce-a";
    let nonce_b = b"stale-nonce-b";
    let ctx_a = BlockContext::build(block_commitment, nonce_a, &a, &b, &params).unwrap();
    let target = [0xff; 32];

    assert_eq!(
        mine_with_context_at_target(
            &ctx_a,
            block_commitment,
            nonce_b,
            &target,
            ProverOptions::default(),
        ),
        Err(MineError::ContextAttemptMismatch)
    );
}

#[test]
fn proof_for_one_nonce_fails_verification_under_another_nonce() {
    let params = small_params();
    let (a, b) = synth_matrices(b"nonce-substitution-seed", &params);
    let block_commitment = b"nonce-substitution-block";
    let nonce_a = b"nonce-substitution-a";
    let nonce_b = b"nonce-substitution-b";
    let target = [0xff; 32];

    let proof = mine(
        block_commitment,
        nonce_a,
        &a,
        &b,
        &params,
        ProverOptions::default(),
    )
    .unwrap()
    .expect("max target must yield a proof");

    verify_at_target(block_commitment, nonce_a, &params, &target, &proof).unwrap();
    assert!(
        verify_at_target(block_commitment, nonce_b, &params, &target, &proof).is_err(),
        "proofs must not survive nonce substitution"
    );
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
    // The nonce is part of Pearl's attempt state, so matrix commitments are
    // re-keyed per nonce before noise and matmul are computed.
    assert_ne!(p1.h_a, p2.h_a);
    assert_ne!(p1.h_b, p2.h_b);
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
fn seek_best_does_not_scan_beyond_verifier_derived_attempt_tile() {
    let params = small_params();
    let (a, b) = synth_matrices(b"ab-seed", &params);
    let default_proof = mine(b"hdr", b"nce", &a, &b, &params, ProverOptions::default())
        .unwrap()
        .unwrap();
    let seek_best_proof = mine(
        b"hdr",
        b"nce",
        &a,
        &b,
        &params,
        ProverOptions { seek_best: true },
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (seek_best_proof.found.i, seek_best_proof.found.j),
        (default_proof.found.i, default_proof.found.j)
    );
    verify(b"hdr", b"nce", &params, &seek_best_proof).unwrap();
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
