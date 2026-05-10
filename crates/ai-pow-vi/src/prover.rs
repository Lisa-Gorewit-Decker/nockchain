//! Verifiable-inference puzzle prover (`mine_vi`).
//!
//! Per-attempt flow:
//! 1. Synthesize the prompt from `(block_commitment, model_id)` via
//!    [`crate::prompt::synth_prompt`].
//! 2. Run [`crate::forward::forward_prefix`] to a Fiat-Shamir-derived
//!    `target_layer` (passed in via [`ProverOptions`] for now), recording
//!    every per-layer activation Merkle root.
//! 3. Run the FFN gate matmul at `target_layer` tile-by-tile. Each tile
//!    output (an i32 sub-block) is hashed via [`ai_pow::tile_hash::tile_state_hash`]
//!    to a 32-byte leaf. The leaves are reduced via [`ai_pow::commit::merkle_root`]
//!    to produce `comm_M`.
//! 4. Derive a challenge seed from `(block_commitment, nonce, comm_M, model_id)`.
//!    For each tile, compute its hardness as
//!    `BLAKE3(challenge_seed || (i, j) || M_{i,j})`.
//! 5. The "found" tile is the lowest-index tile (row-major) whose
//!    hardness ≤ `target`. If none, return `Ok(None)`.
//! 6. Otherwise, derive σ FS spot-check tile coordinates from the same
//!    challenge seed and assemble a [`ViProof`].
//!
//! Notes:
//! - Reduction order, tile order, and challenge derivation are pinned;
//!   any divergence is a consensus break.
//! - The prover replays its own Merkle paths from the leaf set it just
//!   built; this is an O(n) operation, not a separate I/O step.

use ai_pow::commit::{merkle_path, merkle_root};
use ai_pow::fiat_shamir::{block_state, challenge_indices, challenge_seed};
use ai_pow::tile_hash::{hash_le_target, tile_hardness_hash, tile_state_hash};
use thiserror::Error;

use crate::activations::{ActivationLayout, ActivationLog};
use crate::forward::{forward_prefix, ForwardError};
use crate::layer::{LayerContext, LayerWeights};
use crate::matmul_int8::{matmul_int8, MatmulError};
use crate::model::Model;
use crate::prompt::{synth_prompt, PromptError};
use crate::proof::{TileOpening, ViProof};

/// Tile size (rows × cols of the FFN gate output) used for the puzzle.
/// Same value as `ai-pow::params::MatmulParams` FFN tile, so a future
/// SIMD jet kernel can be reused.
pub const FFN_PUZZLE_TILE: u32 = 64;

/// Tunables for one mining attempt. `target_layer` and `sigma` are
/// supplied by the caller — a future tightening will derive them from
/// FS itself, but for Phase 3 they're explicit.
#[derive(Debug, Clone, Copy)]
pub struct ProverOptions {
    pub target_layer: u32,
    pub sigma: u32,
    pub tile: u32,
}

impl Default for ProverOptions {
    fn default() -> Self {
        Self {
            target_layer: 0,
            sigma: 8,
            tile: FFN_PUZZLE_TILE,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProverError {
    #[error("forward: {0}")]
    Forward(#[from] ForwardError),
    #[error("matmul: {0}")]
    Matmul(#[from] MatmulError),
    #[error("prompt: {0}")]
    Prompt(#[from] PromptError),
    #[error("model has no FFN at the target layer")]
    NoFfn,
    #[error("FFN intermediate ({i}) is not a multiple of tile ({t})")]
    IntermediateNotMultipleOfTile { i: u32, t: u32 },
    #[error("seq_len ({s}) is not a multiple of tile ({t})")]
    SeqLenNotMultipleOfTile { s: u32, t: u32 },
    #[error("merkle: {0}")]
    Merkle(#[from] ai_pow::commit::MerkleError),
}

/// Run one mining attempt against `target`. Returns `Ok(Some(ViProof))`
/// if a tile was found whose hardness ≤ `target`; `Ok(None)` if not.
///
/// `block_commitment` and `nonce` are the per-attempt inputs (the chain
/// pins the block commitment; the miner sweeps `nonce`). `model_id` is
/// the model's `comm_W`.
pub fn mine_vi(
    model: &Model,
    model_id: &[u8; 32],
    ctx: &LayerContext,
    block_commitment: &[u8],
    nonce: &[u8],
    target: &[u8; 32],
    opts: ProverOptions,
) -> Result<Option<ViProof>, ProverError> {
    // 1. Synth prompt.
    let prompt = synth_prompt(
        block_commitment,
        model_id,
        model.dims.seq_len,
        model.dims.vocab,
        &[],
    )?;

    // 2. Run forward prefix to target_layer.
    let layout = ActivationLayout {
        seq_len: model.dims.seq_len,
        hidden: model.dims.hidden,
        tile: model.dims.activation_tile,
    };
    let mut log = ActivationLog::new(layout)
        .map_err(|e| ProverError::Forward(ForwardError::Activation(e)))?;
    let layer_input = forward_prefix(model, &prompt, opts.target_layer, ctx, &mut log)?;

    // 3. Compute FFN gate matmul at target_layer tile by tile.
    let layer = &model.layers[opts.target_layer as usize];
    let (ffn_w, hidden, intermediate) = match layer {
        LayerWeights::Attention { ffn, .. } | LayerWeights::DeltaNet { ffn, .. } => {
            (ffn, ffn.hidden, ffn.intermediate)
        }
    };
    let m = prompt.len() as u32;
    if m % opts.tile != 0 {
        return Err(ProverError::SeqLenNotMultipleOfTile { s: m, t: opts.tile });
    }
    if intermediate % opts.tile != 0 {
        return Err(ProverError::IntermediateNotMultipleOfTile {
            i: intermediate,
            t: opts.tile,
        });
    }

    // Compute the full gate matmul into i32. (Phase 4 SIMD jets will
    // tile this and avoid the dense intermediate.)
    let mu = m as usize;
    let _hu = hidden as usize;
    let iu = intermediate as usize;
    let mut gate = vec![0i32; mu * iu];
    matmul_int8(
        &layer_input, &ffn_w.w_gate, m, hidden, intermediate, &mut gate,
    )?;

    // Tile-Merkle leaves from i32 sub-block hashes. Tile (r, c) covers
    // rows [r*tile, (r+1)*tile) and cols [c*tile, (c+1)*tile).
    let nr = m / opts.tile;
    let nc = intermediate / opts.tile;
    let total = (nr * nc) as usize;
    let tile_us = opts.tile as usize;
    let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(total);
    for r in 0..nr as usize {
        for c in 0..nc as usize {
            // Materialize the tile (row-major within tile).
            let mut block: Vec<i32> = Vec::with_capacity(tile_us * tile_us);
            for tr in 0..tile_us {
                let row_off = (r * tile_us + tr) * iu;
                let col_off = c * tile_us;
                block.extend_from_slice(&gate[row_off + col_off..row_off + col_off + tile_us]);
            }
            leaves.push(tile_state_hash(&block));
        }
    }
    drop(gate);
    let comm_m = merkle_root(&leaves)?;

    // 4. Challenge seed.
    let state = block_state(block_commitment, nonce);
    let seed = challenge_seed(&state, &comm_m, model_id);

    // 5. Find a tile whose hardness ≤ target. Linear scan in canonical
    // (row-major) order so the prover and verifier agree on which tile
    // is "the" hit (the first one).
    let mut found_idx: Option<u32> = None;
    for idx in 0..total {
        let r = (idx / nc as usize) as u32;
        let c = (idx % nc as usize) as u32;
        let h = tile_hardness_hash(&seed, r, c, &leaves[idx]);
        if hash_le_target(&h, target) {
            found_idx = Some(idx as u32);
            break;
        }
    }
    let Some(found_idx) = found_idx else {
        return Ok(None);
    };
    let found_r = found_idx / nc;
    let found_c = found_idx % nc;
    let found_path = merkle_path(&leaves, found_idx as usize)?;
    let found = TileOpening {
        i: found_r,
        j: found_c,
        m_ij: leaves[found_idx as usize],
        path: found_path,
    };

    // 6. σ FS spot-checks. Sample `sigma` distinct tile indices from
    // `seed`. (If sigma > total, we cap to `total - 1` to keep the proof
    // valid; the verifier matches this rule.)
    let want = opts.sigma.min((total - 1).max(0) as u32);
    let mut spot_checks: Vec<TileOpening> = Vec::new();
    if total > 1 && want > 0 {
        let mut indices = challenge_indices(&seed, want, total as u32);
        // Drop any sample that collides with `found_idx` to avoid duplication.
        indices.retain(|&i| i != found_idx);
        for idx in indices {
            let r = idx / nc;
            let c = idx % nc;
            let path = merkle_path(&leaves, idx as usize)?;
            spot_checks.push(TileOpening {
                i: r,
                j: c,
                m_ij: leaves[idx as usize],
                path,
            });
        }
    }

    // 7. Pull per-layer activation roots from the log.
    let comm_activations: Vec<[u8; 32]> = log.layer_roots.clone();

    Ok(Some(ViProof {
        model_id: *model_id,
        layer_index: opts.target_layer,
        comm_activations,
        comm_m,
        found,
        spot_checks,
    }))
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
        // Use FFN with dims that match the puzzle tile (4) so the test
        // exercises a real tile-Merkle.
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
    fn easy_target_returns_some() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        // Target = 0xff..ff: every hash ≤ target, so the FIRST tile wins.
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 2,
            tile: 2,
        };
        let proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &target, opts)
            .unwrap()
            .expect("expected Some on max target");
        assert_eq!(proof.layer_index, 0);
        assert_eq!(proof.comm_activations.len(), 1); // target_layer + 1
        assert_eq!(proof.found.i, 0);
        assert_eq!(proof.found.j, 0);
    }

    #[test]
    fn impossible_target_returns_none() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        // Target = 0x00..00: only the all-zero hash satisfies this. Negligible.
        let target = [0u8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 2,
            tile: 2,
        };
        let proof = mine_vi(&m, &model_id, &ctx, b"block", b"nonce", &target, opts).unwrap();
        assert!(proof.is_none());
    }

    #[test]
    fn determinism_two_attempts_same_proof() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 3,
            tile: 2,
        };
        let p1 = mine_vi(&m, &model_id, &ctx, b"b", b"n", &target, opts)
            .unwrap()
            .unwrap();
        let p2 = mine_vi(&m, &model_id, &ctx, b"b", b"n", &target, opts)
            .unwrap()
            .unwrap();
        assert_eq!(p1.encode(), p2.encode());
    }

    #[test]
    fn changing_nonce_changes_challenge_seed_only() {
        // Different nonce → same comm_m (since prompt depends on
        // block_commitment, not nonce) but different challenge seed →
        // potentially different `found` tile (or different spot-checks).
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 4,
            tile: 2,
        };
        let p1 = mine_vi(&m, &model_id, &ctx, b"b", b"n1", &target, opts)
            .unwrap()
            .unwrap();
        let p2 = mine_vi(&m, &model_id, &ctx, b"b", b"n2", &target, opts)
            .unwrap()
            .unwrap();
        // Same comm_M (prompt does not depend on nonce).
        assert_eq!(p1.comm_m, p2.comm_m);
        // Same comm_activations.
        assert_eq!(p1.comm_activations, p2.comm_activations);
    }

    #[test]
    fn rejects_misaligned_tile() {
        let m = build_test_model();
        let ctx = make_ctx(&m);
        let model_id = compute_comm_w(&m);
        let target = [0xffu8; 32];
        // Tile=3 doesn't divide intermediate=8.
        let opts = ProverOptions {
            target_layer: 0,
            sigma: 2,
            tile: 3,
        };
        let r = mine_vi(&m, &model_id, &ctx, b"b", b"n", &target, opts);
        assert!(r.is_err());
    }
}
