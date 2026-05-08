//! End-to-end tests at LLM-shaped, non-power-of-two-tile-count parameters.
//!
//! Production LLM dimensions are huge (Gemma 4 31B FFN tile count is
//! 64 × 336 = 21504; padded depth 15) so we exercise small rectangular
//! shapes that share the relevant structural properties:
//! - `m != n` (not square),
//! - `(m / t) * (n / t)` not a power of two,
//! - tile-Merkle padded to next power of two with a sentinel.

use ai_pow::params::MatmulParams;
use ai_pow::prover::{mine, ProverOptions};
use ai_pow::verifier::verify;

const EASY: [u8; 32] = [0xff; 32];

fn rect_a() -> MatmulParams {
    // (8 row tiles) * (12 col tiles) = 96 tiles, padded to 128.
    MatmulParams {
        m: 64,
        k: 80,
        n: 96,
        noise_rank: 8,
        tile: 8,
        spot_checks: 4,
        lambda: 8,
    }
}

fn rect_b() -> MatmulParams {
    // (12 row tiles) * (10 col tiles) = 120 tiles, padded to 128.
    MatmulParams {
        m: 96,
        k: 80,
        n: 80,
        noise_rank: 8,
        tile: 8,
        spot_checks: 4,
        lambda: 8,
    }
}

#[test]
fn rectangle_round_trip_a() {
    let params = rect_a();
    let proof = mine(b"hdr", b"nce", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    verify(b"hdr", b"nce", &params, &EASY, &proof).unwrap();
}

#[test]
fn rectangle_round_trip_b() {
    let params = rect_b();
    let proof = mine(b"hdr", b"nce", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    verify(b"hdr", b"nce", &params, &EASY, &proof).unwrap();
}

#[test]
fn swapping_m_and_n_changes_proof() {
    let pa = rect_a();
    let pb = MatmulParams {
        m: pa.n,
        n: pa.m,
        ..pa
    };
    let pa_proof = mine(b"hdr", b"nce", &pa, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    let pb_proof = mine(b"hdr", b"nce", &pb, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    assert_ne!(pa_proof, pb_proof);
    // Cross-verify must reject (different params_tag).
    let cross = verify(b"hdr", b"nce", &pb, &EASY, &pa_proof);
    assert!(cross.is_err());
}

#[test]
fn merkle_path_length_is_padded_depth() {
    let params = rect_a();
    let proof = mine(b"hdr", b"nce", &params, &EASY, ProverOptions::default())
        .unwrap()
        .unwrap();
    let padded_depth = params.num_tiles().next_power_of_two().trailing_zeros() as usize;
    assert_eq!(proof.found.path.len(), padded_depth);
    for opening in &proof.spot {
        assert_eq!(opening.path.len(), padded_depth);
    }
}

#[test]
fn llm_profiles_validate() {
    // Just confirm the named LLM profiles exist and pass validation. We do
    // *not* run the prover at LLM scale here — that is left to bench harness.
    MatmulParams::GEMMA_4_31B_FFN.validate().unwrap();
    MatmulParams::QWEN_3_6_27B_FFN.validate().unwrap();
    MatmulParams::llm_ffn(5376, 21504, 4096).validate().unwrap();
    MatmulParams::llm_ffn(5120, 17408, 4096).validate().unwrap();

    // Padded depth at production scale is 15 (next pow2 of 21504).
    assert_eq!(
        MatmulParams::GEMMA_4_31B_FFN
            .num_tiles_padded()
            .trailing_zeros(),
        15,
    );
}
