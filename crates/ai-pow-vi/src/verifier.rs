//! Verifiable-inference puzzle verifier (`verify_vi`).
//!
//! Currently provides one mode:
//!
//! - **`VerifierMode::FullReplica`** — re-runs the prover from scratch,
//!   recomputes `comm_M` and the per-layer activation roots, and checks
//!   them byte-for-byte against the proof. Then verifies the found tile
//!   hashes ≤ target and recovers the same `comm_M` from the opened path.
//!   Finally re-derives the σ FS spot-check coordinates from the proof's
//!   own `comm_M` and verifies each opening recovers `comm_M`.
//!
//! Light-path and federated modes are deferred — they require the proof
//! to carry weight-tile and activation-tile openings so the verifier can
//! avoid recomputation. Phase 4 jets and a tightened proof body unlock
//! those.

use ai_pow::commit::{merkle_recover_root, MerkleError};
use ai_pow::fiat_shamir::{block_state, challenge_indices, challenge_seed};
use ai_pow::tile_hash::{hash_le_target, tile_hardness_hash};
use thiserror::Error;

use crate::comm_w::compute_comm_w;
use crate::layer::LayerContext;
use crate::model::Model;
use crate::proof::{TileOpening, ViProof};
use crate::prover::{mine_vi, ProverError, ProverOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifierMode {
    /// Re-run the prover end-to-end and check every commitment.
    FullReplica,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("model_id in proof does not match the registered comm_W")]
    ModelIdMismatch,
    #[error("layer_index in proof does not match the prover options")]
    LayerIndexMismatch,
    #[error("activation root count mismatch: proof has {got}, expected {want}")]
    ActivationCountMismatch { got: u32, want: u32 },
    #[error("activation root {idx} mismatch")]
    ActivationRootMismatch { idx: u32 },
    #[error("comm_M mismatch")]
    CommMMismatch,
    #[error("found tile hardness exceeds target")]
    HardnessFailure,
    #[error("tile {i},{j} path does not recover comm_M")]
    PathMismatch { i: u32, j: u32 },
    #[error("spot-check {idx} not in FS-derived index set")]
    UnexpectedSpotCheck { idx: u32 },
    #[error("missing FS-derived spot-check tile ({i},{j})")]
    MissingSpotCheck { i: u32, j: u32 },
    #[error("prover (recompute): {0}")]
    Prover(#[from] ProverError),
    #[error("merkle: {0}")]
    Merkle(#[from] MerkleError),
}

/// Verify a `ViProof` against `target`. The verifier needs the same
/// `Model` the prover used (via the model registry — `model_id` pins the
/// comm_W) and the same `block_commitment, nonce`. Phase 3 is full-
/// replica only; light-path and federated come later.
pub fn verify_vi(
    model: &Model,
    ctx: &LayerContext,
    block_commitment: &[u8],
    nonce: &[u8],
    target: &[u8; 32],
    proof: &ViProof,
    opts: ProverOptions,
    mode: VerifierMode,
) -> Result<(), VerifyError> {
    // 1. Check model_id matches the loaded model's comm_W.
    let computed_model_id = compute_comm_w(model);
    if proof.model_id != computed_model_id {
        return Err(VerifyError::ModelIdMismatch);
    }
    if proof.layer_index != opts.target_layer {
        return Err(VerifyError::LayerIndexMismatch);
    }

    match mode {
        VerifierMode::FullReplica => verify_full_replica(
            model, ctx, block_commitment, nonce, target, proof, opts, &computed_model_id,
        ),
    }
}

fn verify_full_replica(
    model: &Model,
    ctx: &LayerContext,
    block_commitment: &[u8],
    nonce: &[u8],
    target: &[u8; 32],
    proof: &ViProof,
    opts: ProverOptions,
    model_id: &[u8; 32],
) -> Result<(), VerifyError> {
    // 1. Re-run the prover and assert byte-equal commitments.
    let recomputed = mine_vi(model, model_id, ctx, block_commitment, nonce, target, opts)?
        .ok_or(VerifyError::HardnessFailure)?;

    let want = recomputed.comm_activations.len() as u32;
    let got = proof.comm_activations.len() as u32;
    if want != got {
        return Err(VerifyError::ActivationCountMismatch { got, want });
    }
    for (i, (a, b)) in proof
        .comm_activations
        .iter()
        .zip(recomputed.comm_activations.iter())
        .enumerate()
    {
        if a != b {
            return Err(VerifyError::ActivationRootMismatch { idx: i as u32 });
        }
    }
    if proof.comm_m != recomputed.comm_m {
        return Err(VerifyError::CommMMismatch);
    }

    // 2. Hardness check on the found tile.
    let state = block_state(block_commitment, nonce);
    let seed = challenge_seed(&state, &proof.comm_m, &proof.model_id);
    let h = tile_hardness_hash(&seed, proof.found.i, proof.found.j, &proof.found.m_ij);
    if !hash_le_target(&h, target) {
        return Err(VerifyError::HardnessFailure);
    }

    // 3. Re-derive the tile dimensions to compute (i, j) → flat-index.
    let m = model.dims.seq_len; // prover uses prompt of length seq_len
                                // ...but actual prompt length is the `synth_prompt` output, which
                                // equals seq_len. Both prover and verifier produce the same prompt.
    let nr = m / opts.tile;
    let nc = recomputed_intermediate_tiles(model, &opts)?;
    let total = (nr * nc) as usize;
    let flat = |o: &TileOpening| -> u32 { o.i * nc + o.j };

    // 4. Verify the found path recovers comm_M.
    verify_path(&proof.found, total, &proof.comm_m, flat(&proof.found))?;

    // 5. Re-derive the σ FS spot-check tile set from the proof's own
    // comm_M (not the recomputed one — the spot-check derivation is
    // pinned to what the proof carries) and verify each opening.
    let want_sigma = opts.sigma.min((total - 1).max(0) as u32);
    let mut want_indices = challenge_indices(&seed, want_sigma, total as u32);
    let found_idx = flat(&proof.found);
    want_indices.retain(|&i| i != found_idx);

    if proof.spot_checks.len() != want_indices.len() {
        // Lengths must match exactly so the verifier can pair every spot-check.
        return Err(VerifyError::UnexpectedSpotCheck {
            idx: proof.spot_checks.len() as u32,
        });
    }
    // Spot checks should appear in the same order the prover produced
    // them (FS-derived). Verify each.
    for (k, opening) in proof.spot_checks.iter().enumerate() {
        let want_idx = want_indices[k];
        let got_idx = flat(opening);
        if got_idx != want_idx {
            return Err(VerifyError::UnexpectedSpotCheck { idx: got_idx });
        }
        verify_path(opening, total, &proof.comm_m, got_idx)?;
    }

    Ok(())
}

fn recomputed_intermediate_tiles(model: &Model, opts: &ProverOptions) -> Result<u32, VerifyError> {
    let layer = &model.layers[opts.target_layer as usize];
    let intermediate = match layer {
        crate::layer::LayerWeights::Attention { ffn, .. }
        | crate::layer::LayerWeights::DeltaNet { ffn, .. }
        | crate::layer::LayerWeights::Gemma { ffn, .. }
        | crate::layer::LayerWeights::QwenStandard { ffn, .. } => ffn.intermediate,
    };
    Ok(intermediate / opts.tile)
}

fn verify_path(
    opening: &TileOpening,
    total: usize,
    expected_root: &[u8; 32],
    flat_idx: u32,
) -> Result<(), VerifyError> {
    let recovered = merkle_recover_root(&opening.m_ij, flat_idx as usize, &opening.path, total)?;
    if recovered != *expected_root {
        return Err(VerifyError::PathMismatch {
            i: opening.i,
            j: opening.j,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_lut::ActivationLut;
    use crate::attention::{AttentionScales, AttentionWeights};
    use crate::comm_w::compute_comm_w;
    use crate::ffn::{FfnScales, FfnWeights};
    use crate::layer::{LayerContext, LayerWeights, NormSpec};
    use crate::model::ModelDims;
    use crate::quant::{Scale, SCALE_DENOM_LOG2};
    use crate::rmsnorm::DEFAULT_EPS_Q;
    use crate::rope::RopeTables;
    use crate::softmax::ExpLut;

    fn lcg_bytes(len: usize, seed: u64) -> Vec<i8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.wrapping_shr(56) as i8
            })
            .collect()
    }

    fn small() -> Scale {
        Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap()
    }

    fn build_test_model() -> Model {
        let hidden = 4u32;
        let hu = hidden as usize;
        let intermediate = 8u32;
        let iu = intermediate as usize;
        let seq_len = 4u32;
        Model {
            dims: ModelDims {
                vocab: 16,
                hidden,
                seq_len,
                activation_tile: 2,
            },
            arch_tag: [0u8; 16],
            feature_flags: 0,
            embed: lcg_bytes(16 * hu, 0xa1a1),
            layers: vec![LayerWeights::Attention {
                norm1: NormSpec::RmsNorm {
                    gamma: lcg_bytes(hu, 0xb2b2),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                attn: AttentionWeights {
                    hidden,
                    num_q_heads: 1,
                    num_kv_heads: 1,
                    head_dim: 2,
                    w_q: lcg_bytes(hu * 2, 0xc3c3),
                    w_k: lcg_bytes(hu * 2, 0xd4d4),
                    w_v: lcg_bytes(hu * 2, 0xe5e5),
                    w_o: lcg_bytes(2 * hu, 0xf6f6),
                },
                attn_scales: AttentionScales {
                    q: small(),
                    k: small(),
                    v: small(),
                    score: small(),
                    attn_out: small(),
                    o: small(),
                },
                norm2: NormSpec::RmsNorm {
                    gamma: lcg_bytes(hu, 0x0707),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                ffn: FfnWeights {
                    hidden,
                    intermediate,
                    w_gate: lcg_bytes(hu * iu, 0x1818),
                    w_up: lcg_bytes(hu * iu, 0x2929),
                    w_down: lcg_bytes(iu * hu, 0x3a3a),
                },
                ffn_scales: FfnScales {
                    gate: small(),
                    up: small(),
                    mid: small(),
                    down: small(),
                },
            }],
            final_norm: None,
            rope_tables: RopeTables::identity(seq_len, 1),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        }
    }

    fn make_ctx<'a>(m: &'a Model) -> LayerContext<'a> {
        LayerContext {
            rope_tables: &m.rope_tables,
            softmax_lut: &m.softmax_lut,
            sigmoid_lut: &m.sigmoid_lut,
            ffn_activation: &m.ffn_activation,
        }
    }

    #[test]
    fn full_replica_round_trip() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 3,
            tile: 2,
        };
        let proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &target, opts)
            .unwrap()
            .unwrap();
        verify_vi(
            &m,
            &ctx,
            b"block",
            b"nonce",
            &target,
            &proof,
            opts,
            VerifierMode::FullReplica,
        )
        .unwrap();
    }

    #[test]
    fn rejects_tampered_model_id() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 3,
            tile: 2,
        };
        let mut proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &target, opts)
            .unwrap()
            .unwrap();
        proof.model_id[0] ^= 1;
        assert_eq!(
            verify_vi(
                &m,
                &ctx,
                b"block",
                b"nonce",
                &target,
                &proof,
                opts,
                VerifierMode::FullReplica
            )
            .err(),
            Some(VerifyError::ModelIdMismatch),
        );
    }

    #[test]
    fn rejects_tampered_comm_m() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 3,
            tile: 2,
        };
        let mut proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &target, opts)
            .unwrap()
            .unwrap();
        proof.comm_m[0] ^= 1;
        assert_eq!(
            verify_vi(
                &m,
                &ctx,
                b"block",
                b"nonce",
                &target,
                &proof,
                opts,
                VerifierMode::FullReplica
            )
            .err(),
            Some(VerifyError::CommMMismatch),
        );
    }

    #[test]
    fn rejects_tampered_found_m_ij() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 3,
            tile: 2,
        };
        let mut proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &target, opts)
            .unwrap()
            .unwrap();
        // Flip a byte in the found leaf hash. Path no longer recovers root,
        // and hardness over the new leaf may also change.
        proof.found.m_ij[0] ^= 1;
        assert!(verify_vi(
            &m,
            &ctx,
            b"block",
            b"nonce",
            &target,
            &proof,
            opts,
            VerifierMode::FullReplica
        )
        .is_err(),);
    }

    #[test]
    fn rejects_layer_mismatch() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 3,
            tile: 2,
        };
        let mut proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &target, opts)
            .unwrap()
            .unwrap();
        proof.layer_index = 99;
        assert_eq!(
            verify_vi(
                &m,
                &ctx,
                b"block",
                b"nonce",
                &target,
                &proof,
                opts,
                VerifierMode::FullReplica
            )
            .err(),
            Some(VerifyError::LayerIndexMismatch),
        );
    }

    #[test]
    fn rejects_below_target_when_target_zero() {
        // With target = 0, the prover would have returned None, so any
        // claimed proof can only be produced by a forging attempt. Because
        // mine_vi inside verify_full_replica also returns None, the
        // verifier surfaces HardnessFailure.
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        // Build a proof with EASY target, then verify against IMPOSSIBLE target.
        let easy = [0xffu8; 32];
        let hard = [0u8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 3,
            tile: 2,
        };
        let proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &easy, opts)
            .unwrap()
            .unwrap();
        assert_eq!(
            verify_vi(
                &m,
                &ctx,
                b"block",
                b"nonce",
                &hard,
                &proof,
                opts,
                VerifierMode::FullReplica
            )
            .err(),
            Some(VerifyError::HardnessFailure),
        );
    }
}
