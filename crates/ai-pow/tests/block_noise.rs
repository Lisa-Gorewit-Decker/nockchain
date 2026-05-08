//! Tests for the per-block noise cache: `mine_block` must produce the same
//! proof bytes as `mine` at the same nonce, and noise expansion must not
//! depend on the nonce.

use ai_pow::params::MatmulParams;
use ai_pow::prover::{mine, mine_block, ProverOptions};
use ai_pow::verifier::verify;

const EASY: [u8; 32] = [0xff; 32];

#[test]
fn mine_and_mine_block_agree_at_same_nonce() {
    let params = MatmulParams::TEST_SMALL;
    let single = mine(b"hdr", b"nce-1", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    let block_one = mine_block(
        b"hdr",
        std::iter::once(b"nce-1" as &[u8]),
        &params,
        &EASY,
        ProverOptions::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        single, block_one,
        "mine and mine_block must be byte-equivalent"
    );
    verify(b"hdr", b"nce-1", &params, &EASY, &block_one).unwrap();
}

#[test]
fn mine_block_returns_first_satisfying_nonce() {
    let params = MatmulParams::TEST_SMALL;
    // With an easy target every nonce produces a proof, so the first nonce
    // wins. mine_block should return the same proof as mine on that nonce.
    let nonces: Vec<&[u8]> = vec![b"a", b"b", b"c"];
    let block_proof = mine_block(b"hdr", nonces, &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    let single = mine(b"hdr", b"a", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(block_proof, single);
}

#[test]
fn mine_block_returns_none_when_no_nonce_satisfies() {
    let params = MatmulParams::TEST_SMALL;
    let target = [0u8; 32]; // impossible
    let r = mine_block(
        b"hdr",
        [b"a" as &[u8], b"b", b"c"],
        &params,
        &target,
        ProverOptions::default(),
    )
    .unwrap();
    assert!(r.is_none());
}

#[test]
fn mine_block_preserves_per_nonce_diversity() {
    // The two proofs from different nonces (within the same block) must
    // still differ — block-level noise reuse only affects (E, F), not the
    // per-attempt (A, B) and tile hashes.
    let params = MatmulParams::TEST_SMALL;
    let p_a = mine(b"hdr", b"a", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    let p_b = mine(b"hdr", b"b", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    assert_ne!(p_a, p_b);
}

#[test]
fn block_level_noise_is_block_scoped() {
    // Same nonce, different block_commitment must produce a different proof
    // (because (E, F) depend on block_commitment).
    let params = MatmulParams::TEST_SMALL;
    let p_a = mine(b"hdr-A", b"nce", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    let p_b = mine(b"hdr-B", b"nce", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    assert_ne!(p_a, p_b);
}
