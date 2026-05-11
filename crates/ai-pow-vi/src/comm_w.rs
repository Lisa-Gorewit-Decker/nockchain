//! Canonical model commitment (`comm_W`).
//!
//! `comm_W` is a 32-byte BLAKE3 hash that pins every consensus-relevant
//! byte of a [`crate::model::Model`]: weights, scales, LUTs, RoPE tables,
//! and architecture metadata (layer counts, head counts, dims). A model
//! release publishes `comm_W` so verifiers can detect any tampered byte.
//!
//! Two-level structure:
//! 1. **Weight tile-Merkle root.** Every `i8` weight tensor in the model
//!    is concatenated in canonical order (see [`canonical_weight_bytes`])
//!    and tile-hashed with [`ai_pow::commit::merkle_root`] over 64-byte
//!    tile leaves. This is the part the verifier spot-checks.
//! 2. **Manifest hash.** Architecture metadata (dims, head counts,
//!    block kinds, scales, eps) is serialized canonically and hashed
//!    under a domain-separated context.
//!
//! `comm_W = BLAKE3.derive_key("ai-pow-vi v1 comm-w")
//!          .update(weights_root || manifest_hash)
//!          .finalize()`
//!
//! Tile size is fixed at 64 bytes (matches the FFN tile size in
//! `ai-pow::params::MatmulParams`). Padding to the next power of two is
//! handled by the underlying merkle_root with sentinel hashes.

use ai_pow::commit::merkle_root;
use blake3::Hasher;

use crate::activation_lut::ActivationLut;
use crate::layer::{LayerWeights, NormSpec};
use crate::model::Model;
use crate::quant::Scale;
use crate::rope::RopeTables;
use crate::softmax::ExpLut;

const CTX_COMM_W: &str = "ai-pow-vi v1 comm-w";
const CTX_WEIGHT_TILE: &str = "ai-pow-vi v1 weight-tile";
const CTX_MANIFEST: &str = "ai-pow-vi v1 manifest";

/// Tile size (in bytes) for the weight tile-Merkle commitment. Chosen to
/// match the FFN puzzle tile so the verifier can reuse the same opening
/// shape it uses for spot-checked weight tiles in [`crate::ffn`].
pub const WEIGHT_TILE_BYTES: usize = 64;

fn append_i8s(out: &mut Vec<u8>, xs: &[i8]) {
    out.reserve(xs.len());
    for &x in xs {
        out.push(x as u8);
    }
}

fn append_i16s_le(out: &mut Vec<u8>, xs: &[i16]) {
    out.reserve(xs.len() * 2);
    for &x in xs {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn append_norm_weights(out: &mut Vec<u8>, norm: &NormSpec) {
    match norm {
        NormSpec::RmsNorm { gamma, .. } => append_i8s(out, gamma),
        NormSpec::LayerNorm { gamma, beta, .. } => {
            append_i8s(out, gamma);
            append_i8s(out, beta);
        }
    }
}

fn append_layer_weights(out: &mut Vec<u8>, layer: &LayerWeights) {
    match layer {
        LayerWeights::Attention {
            norm1,
            attn,
            norm2,
            ffn,
            ..
        } => {
            append_norm_weights(out, norm1);
            append_i8s(out, &attn.w_q);
            append_i8s(out, &attn.w_k);
            append_i8s(out, &attn.w_v);
            append_i8s(out, &attn.w_o);
            append_norm_weights(out, norm2);
            append_i8s(out, &ffn.w_gate);
            append_i8s(out, &ffn.w_up);
            append_i8s(out, &ffn.w_down);
        }
        LayerWeights::DeltaNet {
            norm1,
            dnet,
            norm2,
            ffn,
            ..
        } => {
            append_norm_weights(out, norm1);
            append_i8s(out, &dnet.w_q);
            append_i8s(out, &dnet.w_k);
            append_i8s(out, &dnet.w_v);
            append_i8s(out, &dnet.w_alpha);
            append_i8s(out, &dnet.w_beta);
            append_i8s(out, &dnet.w_o);
            append_norm_weights(out, norm2);
            append_i8s(out, &ffn.w_gate);
            append_i8s(out, &ffn.w_up);
            append_i8s(out, &ffn.w_down);
        }
        LayerWeights::QwenStandard {
            norm1,
            attn,
            q_norm_gamma,
            k_norm_gamma,
            norm2,
            ffn,
            ..
        } => {
            // Canonical order for QwenStandard layer bytes:
            // norm1 → attn(q,k,v,o) → qk_norm gammas → norm2 → ffn(gate,up,down).
            append_norm_weights(out, norm1);
            append_i8s(out, &attn.w_q);
            append_i8s(out, &attn.w_k);
            append_i8s(out, &attn.w_v);
            append_i8s(out, &attn.w_o);
            append_i8s(out, q_norm_gamma);
            append_i8s(out, k_norm_gamma);
            append_norm_weights(out, norm2);
            append_i8s(out, &ffn.w_gate);
            append_i8s(out, &ffn.w_up);
            append_i8s(out, &ffn.w_down);
        }
        LayerWeights::QwenHybridSsm {
            norm1,
            attn_qkv_fused,
            attn_gate,
            attn_out,
            q_norm_gamma,
            k_norm_gamma,
            ssm_a,
            ssm_alpha,
            ssm_beta,
            ssm_conv1d,
            ssm_dt,
            ssm_norm_gamma,
            ssm_out,
            norm2,
            ffn,
            ..
        } => {
            // Canonical order for QwenHybridSsm layer bytes:
            // norm1 → attn_qkv_fused → attn_gate → attn_out → qk_norm gammas →
            // ssm{a,alpha,beta,conv1d,dt,norm_gamma,out} → norm2 → ffn{gate,up,down}.
            append_norm_weights(out, norm1);
            append_i8s(out, attn_qkv_fused);
            append_i8s(out, attn_gate);
            append_i8s(out, attn_out);
            append_i8s(out, q_norm_gamma);
            append_i8s(out, k_norm_gamma);
            append_i8s(out, ssm_a);
            append_i8s(out, ssm_alpha);
            append_i8s(out, ssm_beta);
            append_i8s(out, ssm_conv1d);
            append_i8s(out, ssm_dt);
            append_i8s(out, ssm_norm_gamma);
            append_i8s(out, ssm_out);
            append_norm_weights(out, norm2);
            append_i8s(out, &ffn.w_gate);
            append_i8s(out, &ffn.w_up);
            append_i8s(out, &ffn.w_down);
        }
        LayerWeights::Gemma {
            norm1,
            attn,
            q_norm_gamma,
            k_norm_gamma,
            post_attn_norm,
            norm2,
            ffn,
            post_ffn_norm,
            inp_gate,
            layer_output_scale,
            ..
        } => {
            // Canonical order for Gemma layer bytes (matches Phase 2.11
            // numpy `encode_weights` in synthetic_qwen_mini.py):
            // norm1 → attn(q,k,v,o) → qk_norm gammas → post_attn_norm →
            // norm2 → ffn(gate,up,down) → post_ffn_norm → inp_gate (if
            // present) → layer_output_scale (if present).
            append_norm_weights(out, norm1);
            append_i8s(out, &attn.w_q);
            append_i8s(out, &attn.w_k);
            append_i8s(out, &attn.w_v);
            append_i8s(out, &attn.w_o);
            append_i8s(out, q_norm_gamma);
            append_i8s(out, k_norm_gamma);
            append_norm_weights(out, post_attn_norm);
            append_norm_weights(out, norm2);
            append_i8s(out, &ffn.w_gate);
            append_i8s(out, &ffn.w_up);
            append_i8s(out, &ffn.w_down);
            append_norm_weights(out, post_ffn_norm);
            if let Some(g) = inp_gate {
                append_i8s(out, g);
            }
            if let Some(s) = layer_output_scale {
                append_i8s(out, s);
            }
        }
    }
}

/// Canonical byte stream over all weight tensors in `model`. Output is
/// `i8`-as-`u8` reinterpretation, in the order documented at the top of
/// this module.
///
/// Determinism: identical fields → identical bytes → identical
/// `comm_W`.
pub fn canonical_weight_bytes(model: &Model) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    append_i8s(&mut out, &model.embed);
    for layer in &model.layers {
        append_layer_weights(&mut out, layer);
    }
    if let Some(fn_norm) = &model.final_norm {
        append_norm_weights(&mut out, fn_norm);
    }
    // Embedded LUTs / tables
    append_lut_bytes(&mut out, &model.ffn_activation);
    append_lut_bytes(&mut out, &model.sigmoid_lut);
    append_explut_bytes(&mut out, &model.softmax_lut);
    append_rope_bytes(&mut out, &model.rope_tables);
    out
}

fn append_lut_bytes(out: &mut Vec<u8>, lut: &ActivationLut) {
    out.extend_from_slice(&lut.to_bytes());
}

fn append_explut_bytes(out: &mut Vec<u8>, lut: &ExpLut) {
    out.extend_from_slice(&lut.to_bytes());
}

fn append_rope_bytes(out: &mut Vec<u8>, t: &RopeTables) {
    out.extend_from_slice(&t.seq_len.to_le_bytes());
    out.extend_from_slice(&t.half_head_dim.to_le_bytes());
    append_i16s_le(out, &t.cos);
    append_i16s_le(out, &t.sin);
}

/// Hash the canonical weight byte stream into 32-byte tile leaves and
/// reduce via Merkle. Tile size [`WEIGHT_TILE_BYTES`]; the final partial
/// tile (if any) is zero-padded to the tile size before hashing — the
/// `next_power_of_two` padding inside `merkle_root` handles tree shape.
fn weights_merkle_root(bytes: &[u8]) -> [u8; 32] {
    let n_full = bytes.len() / WEIGHT_TILE_BYTES;
    let rem = bytes.len() % WEIGHT_TILE_BYTES;
    let n_leaves = n_full + if rem != 0 { 1 } else { 0 };
    let n_leaves = n_leaves.max(1); // merkle_root rejects empty.

    let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(n_leaves);
    for i in 0..n_full {
        let chunk = &bytes[i * WEIGHT_TILE_BYTES..(i + 1) * WEIGHT_TILE_BYTES];
        leaves.push(tile_hash(chunk));
    }
    if rem != 0 {
        // Zero-pad the last tile to WEIGHT_TILE_BYTES so the leaf hash
        // input length is constant across all leaves.
        let mut tail = [0u8; WEIGHT_TILE_BYTES];
        tail[..rem].copy_from_slice(&bytes[n_full * WEIGHT_TILE_BYTES..]);
        leaves.push(tile_hash(&tail));
    }
    if leaves.is_empty() {
        // Synthetic single-leaf tree over an all-zero tile so empty models
        // (which shouldn't exist in practice) still produce a well-defined
        // root rather than panicking.
        leaves.push(tile_hash(&[0u8; WEIGHT_TILE_BYTES]));
    }
    merkle_root(&leaves).expect("merkle_root over non-empty leaves cannot fail")
}

fn tile_hash(chunk: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_WEIGHT_TILE);
    hasher.update(&(chunk.len() as u64).to_le_bytes());
    hasher.update(chunk);
    *hasher.finalize().as_bytes()
}

/// Serialize a [`Scale`] canonically: 4-byte LE i32 numerator.
fn append_scale(out: &mut Vec<u8>, s: &Scale) {
    out.extend_from_slice(&s.num.to_le_bytes());
}

/// Hash architecture metadata (dims, layer kinds, scales, eps_q, etc.)
/// under a domain-separated context. Unlike weights, this is a pure
/// summary; no spot-checking — the verifier compares the manifest hash
/// directly.
fn manifest_hash(model: &Model) -> [u8; 32] {
    let mut buf: Vec<u8> = Vec::new();
    // Phase 2.10: arch_tag + feature_flags come first so a model
    // labeled with a different arch cannot share `comm_W` with one of
    // a different lineage that happens to have identical dims.
    buf.extend_from_slice(&model.arch_tag);
    buf.extend_from_slice(&model.feature_flags.to_le_bytes());

    let d = model.dims;
    buf.extend_from_slice(&d.vocab.to_le_bytes());
    buf.extend_from_slice(&d.hidden.to_le_bytes());
    buf.extend_from_slice(&d.seq_len.to_le_bytes());
    buf.extend_from_slice(&d.activation_tile.to_le_bytes());

    buf.extend_from_slice(&(model.layers.len() as u32).to_le_bytes());
    for layer in &model.layers {
        match layer {
            LayerWeights::Attention {
                norm1,
                attn,
                attn_scales,
                norm2,
                ffn,
                ffn_scales,
            } => {
                buf.push(0u8); // tag: attention
                append_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&attn.hidden.to_le_bytes());
                buf.extend_from_slice(&attn.num_q_heads.to_le_bytes());
                buf.extend_from_slice(&attn.num_kv_heads.to_le_bytes());
                buf.extend_from_slice(&attn.head_dim.to_le_bytes());
                append_scale(&mut buf, &attn_scales.q);
                append_scale(&mut buf, &attn_scales.k);
                append_scale(&mut buf, &attn_scales.v);
                append_scale(&mut buf, &attn_scales.score);
                append_scale(&mut buf, &attn_scales.attn_out);
                append_scale(&mut buf, &attn_scales.o);
                append_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.hidden.to_le_bytes());
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                append_scale(&mut buf, &ffn_scales.gate);
                append_scale(&mut buf, &ffn_scales.up);
                append_scale(&mut buf, &ffn_scales.mid);
                append_scale(&mut buf, &ffn_scales.down);
            }
            LayerWeights::DeltaNet {
                norm1,
                dnet,
                dnet_scales,
                norm2,
                ffn,
                ffn_scales,
            } => {
                buf.push(1u8); // tag: deltanet
                append_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&dnet.hidden.to_le_bytes());
                buf.extend_from_slice(&dnet.num_qk_heads.to_le_bytes());
                buf.extend_from_slice(&dnet.num_v_heads.to_le_bytes());
                buf.extend_from_slice(&dnet.head_dim_qk.to_le_bytes());
                buf.extend_from_slice(&dnet.head_dim_v.to_le_bytes());
                append_scale(&mut buf, &dnet_scales.q);
                append_scale(&mut buf, &dnet_scales.k);
                append_scale(&mut buf, &dnet_scales.v);
                append_scale(&mut buf, &dnet_scales.alpha_logit);
                append_scale(&mut buf, &dnet_scales.beta_logit);
                append_scale(&mut buf, &dnet_scales.u);
                append_scale(&mut buf, &dnet_scales.decay);
                append_scale(&mut buf, &dnet_scales.update);
                append_scale(&mut buf, &dnet_scales.o);
                append_scale(&mut buf, &dnet_scales.proj);
                append_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.hidden.to_le_bytes());
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                append_scale(&mut buf, &ffn_scales.gate);
                append_scale(&mut buf, &ffn_scales.up);
                append_scale(&mut buf, &ffn_scales.mid);
                append_scale(&mut buf, &ffn_scales.down);
            }
            LayerWeights::QwenStandard {
                norm1,
                attn,
                attn_scales,
                q_norm_gamma: _,
                k_norm_gamma: _,
                qk_norm_eps_q,
                qk_norm_post_scale,
                norm2,
                ffn,
                ffn_scales,
            } => {
                buf.push(3u8); // tag: qwen-standard
                append_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&attn.hidden.to_le_bytes());
                buf.extend_from_slice(&attn.num_q_heads.to_le_bytes());
                buf.extend_from_slice(&attn.num_kv_heads.to_le_bytes());
                buf.extend_from_slice(&attn.head_dim.to_le_bytes());
                append_scale(&mut buf, &attn_scales.q);
                append_scale(&mut buf, &attn_scales.k);
                append_scale(&mut buf, &attn_scales.v);
                append_scale(&mut buf, &attn_scales.score);
                append_scale(&mut buf, &attn_scales.attn_out);
                append_scale(&mut buf, &attn_scales.o);
                buf.extend_from_slice(&qk_norm_eps_q.to_le_bytes());
                append_scale(&mut buf, qk_norm_post_scale);
                append_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.hidden.to_le_bytes());
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                append_scale(&mut buf, &ffn_scales.gate);
                append_scale(&mut buf, &ffn_scales.up);
                append_scale(&mut buf, &ffn_scales.mid);
                append_scale(&mut buf, &ffn_scales.down);
            }
            LayerWeights::QwenHybridSsm {
                norm1,
                num_q_heads,
                num_kv_heads,
                head_dim,
                attn_scales,
                qk_norm_eps_q,
                qk_norm_post_scale,
                num_v_heads,
                ssm_head_dim,
                ssm_kernel_size,
                ssm_norm_eps_q,
                ssm_norm_post_scale,
                ssm_scales,
                norm2,
                ffn,
                ffn_scales,
                ..
            } => {
                buf.push(4u8); // tag: qwen-hybrid-ssm
                append_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&ffn.hidden.to_le_bytes());
                buf.extend_from_slice(&num_q_heads.to_le_bytes());
                buf.extend_from_slice(&num_kv_heads.to_le_bytes());
                buf.extend_from_slice(&head_dim.to_le_bytes());
                append_scale(&mut buf, &attn_scales.q);
                append_scale(&mut buf, &attn_scales.k);
                append_scale(&mut buf, &attn_scales.v);
                append_scale(&mut buf, &attn_scales.score);
                append_scale(&mut buf, &attn_scales.attn_out);
                append_scale(&mut buf, &attn_scales.o);
                buf.extend_from_slice(&qk_norm_eps_q.to_le_bytes());
                append_scale(&mut buf, qk_norm_post_scale);
                buf.extend_from_slice(&num_v_heads.to_le_bytes());
                buf.extend_from_slice(&ssm_head_dim.to_le_bytes());
                buf.extend_from_slice(&ssm_kernel_size.to_le_bytes());
                buf.extend_from_slice(&ssm_norm_eps_q.to_le_bytes());
                append_scale(&mut buf, ssm_norm_post_scale);
                append_scale(&mut buf, &ssm_scales.q);
                append_scale(&mut buf, &ssm_scales.k);
                append_scale(&mut buf, &ssm_scales.v);
                append_scale(&mut buf, &ssm_scales.alpha_logit);
                append_scale(&mut buf, &ssm_scales.beta_logit);
                append_scale(&mut buf, &ssm_scales.u);
                append_scale(&mut buf, &ssm_scales.decay);
                append_scale(&mut buf, &ssm_scales.update);
                append_scale(&mut buf, &ssm_scales.o);
                append_scale(&mut buf, &ssm_scales.proj);
                append_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                append_scale(&mut buf, &ffn_scales.gate);
                append_scale(&mut buf, &ffn_scales.up);
                append_scale(&mut buf, &ffn_scales.mid);
                append_scale(&mut buf, &ffn_scales.down);
            }
            LayerWeights::Gemma {
                norm1,
                attn,
                attn_scales,
                q_norm_gamma: _,
                k_norm_gamma: _,
                qk_norm_eps_q,
                qk_norm_post_scale,
                post_attn_norm,
                norm2,
                ffn,
                ffn_scales,
                post_ffn_norm,
                sliding_window,
                inp_gate,
                layer_output_scale,
            } => {
                buf.push(2u8); // tag: gemma
                append_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&attn.hidden.to_le_bytes());
                buf.extend_from_slice(&attn.num_q_heads.to_le_bytes());
                buf.extend_from_slice(&attn.num_kv_heads.to_le_bytes());
                buf.extend_from_slice(&attn.head_dim.to_le_bytes());
                append_scale(&mut buf, &attn_scales.q);
                append_scale(&mut buf, &attn_scales.k);
                append_scale(&mut buf, &attn_scales.v);
                append_scale(&mut buf, &attn_scales.score);
                append_scale(&mut buf, &attn_scales.attn_out);
                append_scale(&mut buf, &attn_scales.o);
                // QK-norm metadata: eps_q (i64) + post_scale (Scale).
                buf.extend_from_slice(&qk_norm_eps_q.to_le_bytes());
                append_scale(&mut buf, qk_norm_post_scale);
                append_norm_meta(&mut buf, post_attn_norm);
                append_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.hidden.to_le_bytes());
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                append_scale(&mut buf, &ffn_scales.gate);
                append_scale(&mut buf, &ffn_scales.up);
                append_scale(&mut buf, &ffn_scales.mid);
                append_scale(&mut buf, &ffn_scales.down);
                append_norm_meta(&mut buf, post_ffn_norm);
                // Sliding window: 0 = full causal, >0 = window radius.
                buf.extend_from_slice(&sliding_window.unwrap_or(0).to_le_bytes());
                // Presence flags for inp_gate and layer_output_scale.
                buf.push(if inp_gate.is_some() { 1 } else { 0 });
                buf.push(if layer_output_scale.is_some() { 1 } else { 0 });
            }
        }
    }
    if let Some(fn_norm) = &model.final_norm {
        buf.push(1u8); // has-final-norm tag
        append_norm_meta(&mut buf, fn_norm);
    } else {
        buf.push(0u8);
    }

    let mut hasher = Hasher::new_derive_key(CTX_MANIFEST);
    hasher.update(&(buf.len() as u64).to_le_bytes());
    hasher.update(&buf);
    *hasher.finalize().as_bytes()
}

fn append_norm_meta(buf: &mut Vec<u8>, norm: &NormSpec) {
    match norm {
        NormSpec::RmsNorm {
            eps_q, post_scale, ..
        } => {
            buf.push(0u8); // tag: rmsnorm
            buf.extend_from_slice(&eps_q.to_le_bytes());
            append_scale(buf, post_scale);
        }
        NormSpec::LayerNorm {
            eps_q, post_scale, ..
        } => {
            buf.push(1u8); // tag: layernorm
            buf.extend_from_slice(&eps_q.to_le_bytes());
            append_scale(buf, post_scale);
        }
    }
}

/// Compute the canonical model commitment.
///
/// `comm_W = derive_key("ai-pow-vi v1 comm-w").update(weights_root || manifest_hash).finalize()`
///
/// Two byte-stream inputs:
/// 1. `weights_root` — tile-Merkle root over all i8 weight bytes plus
///    embedded LUT/table bytes.
/// 2. `manifest_hash` — domain-separated BLAKE3 over architecture
///    metadata and quantization scales.
pub fn compute_comm_w(model: &Model) -> [u8; 32] {
    let weight_bytes = canonical_weight_bytes(model);
    let weights_root = weights_merkle_root(&weight_bytes);
    let manifest_root = manifest_hash(model);
    let mut hasher = Hasher::new_derive_key(CTX_COMM_W);
    hasher.update(&weights_root);
    hasher.update(&manifest_root);
    *hasher.finalize().as_bytes()
}

/// Stack-of-subtrees streaming tile-Merkle reducer. Used by
/// [`compute_comm_w_streaming`] to derive `comm_W` from a `weights.bin`
/// on disk without materializing the full canonical byte stream in RAM
/// (which would require ~25 GB on Qwen 3.6 27B).
pub struct StreamingMerkle {
    stack: Vec<(u32, [u8; 32])>,
    tile_buf: Vec<u8>,
    n_leaves: u64,
    finalized: bool,
}

impl StreamingMerkle {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            tile_buf: Vec::with_capacity(WEIGHT_TILE_BYTES),
            n_leaves: 0,
            finalized: false,
        }
    }

    /// Append a byte chunk. Completed 64-byte tiles are hashed
    /// incrementally; partial trailing bytes wait in the buffer until
    /// either filled or `finalize()` runs.
    pub fn update(&mut self, data: &[u8]) {
        assert!(!self.finalized, "StreamingMerkle update after finalize");
        let mut offset = 0;
        // First, fill the partial buffer if any.
        if !self.tile_buf.is_empty() {
            let need = WEIGHT_TILE_BYTES - self.tile_buf.len();
            let take = need.min(data.len());
            self.tile_buf.extend_from_slice(&data[..take]);
            offset = take;
            if self.tile_buf.len() == WEIGHT_TILE_BYTES {
                let buf: [u8; WEIGHT_TILE_BYTES] =
                    self.tile_buf.as_slice().try_into().unwrap();
                self.tile_buf.clear();
                self.add_data_leaf(tile_hash(&buf));
            }
        }
        // Then process full tiles directly from `data`.
        while data.len() - offset >= WEIGHT_TILE_BYTES {
            let chunk = &data[offset..offset + WEIGHT_TILE_BYTES];
            let h = tile_hash(chunk);
            self.add_data_leaf(h);
            offset += WEIGHT_TILE_BYTES;
        }
        // Stash the remainder.
        if offset < data.len() {
            self.tile_buf.extend_from_slice(&data[offset..]);
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        assert!(!self.finalized, "StreamingMerkle finalize twice");
        // Flush partial tail with zero padding.
        if !self.tile_buf.is_empty() {
            let mut tail = [0u8; WEIGHT_TILE_BYTES];
            tail[..self.tile_buf.len()].copy_from_slice(&self.tile_buf);
            self.tile_buf.clear();
            self.add_data_leaf(tile_hash(&tail));
        }
        // Empty input → one all-zero tile leaf (matches non-streaming).
        if self.n_leaves == 0 {
            self.add_data_leaf(tile_hash(&[0u8; WEIGHT_TILE_BYTES]));
        }
        // Pad with sentinel leaves to next power of two.
        let mut target = 1u64;
        while target < self.n_leaves {
            target *= 2;
        }
        let sent = sentinel_leaf();
        while self.n_leaves < target {
            self.add_sentinel_leaf(sent);
        }
        debug_assert_eq!(self.stack.len(), 1);
        self.finalized = true;
        self.stack[0].1
    }

    fn add_data_leaf(&mut self, leaf_h: [u8; 32]) {
        self.push_level0(merkle_leaf_hash(&leaf_h));
        self.n_leaves += 1;
    }

    fn add_sentinel_leaf(&mut self, sent: [u8; 32]) {
        self.push_level0(sent);
        self.n_leaves += 1;
    }

    fn push_level0(&mut self, h: [u8; 32]) {
        self.stack.push((0, h));
        while self.stack.len() >= 2 {
            let top = self.stack.len() - 1;
            if self.stack[top].0 != self.stack[top - 1].0 {
                break;
            }
            let (lv, right) = self.stack.pop().unwrap();
            let (_, left) = self.stack.pop().unwrap();
            self.stack.push((lv + 1, merkle_node_hash(&left, &right)));
        }
    }
}

impl Default for StreamingMerkle {
    fn default() -> Self {
        Self::new()
    }
}

fn merkle_leaf_hash(leaf: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key("ai-pow v1 merkle-leaf");
    hasher.update(leaf);
    *hasher.finalize().as_bytes()
}

fn merkle_node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key("ai-pow v1 merkle-node");
    hasher.update(left);
    hasher.update(right);
    *hasher.finalize().as_bytes()
}

fn sentinel_leaf() -> [u8; 32] {
    let hasher = Hasher::new_derive_key("ai-pow v1 merkle-sentinel");
    *hasher.finalize().as_bytes()
}

/// Compute `comm_W` against an on-disk `weights.bin` (must equal
/// `canonical_weight_bytes(model)`). Streams the file through a
/// stack-of-subtrees Merkle so peak RAM is O(log num_tiles) — fits on
/// a 32 GB Mac even for 25 GB weights.bin where the materializing
/// `compute_comm_w(&model)` path needs ~50 GB.
pub fn compute_comm_w_streaming(
    weights_path: &std::path::Path,
    model: &Model,
) -> std::io::Result<[u8; 32]> {
    use std::io::Read;
    let mut file = std::fs::File::open(weights_path)?;
    let mut sm = StreamingMerkle::new();
    let mut buf = vec![0u8; 16 * 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sm.update(&buf[..n]);
    }
    let weights_root = sm.finalize();
    let manifest_root = manifest_hash(model);
    let mut hasher = Hasher::new_derive_key(CTX_COMM_W);
    hasher.update(&weights_root);
    hasher.update(&manifest_root);
    Ok(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_lut::ActivationLut;
    use crate::attention::{AttentionScales, AttentionWeights};
    use crate::ffn::{FfnScales, FfnWeights};
    use crate::layer::{LayerWeights, NormSpec};
    use crate::model::{Model, ModelDims};
    use crate::quant::SCALE_DENOM_LOG2;
    use crate::rmsnorm::DEFAULT_EPS_Q;

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

    fn build_model() -> Model {
        let hidden = 4u32;
        let hu = hidden as usize;
        Model {
            dims: ModelDims {
                vocab: 8,
                hidden,
                seq_len: 4,
                activation_tile: 2,
            },
            arch_tag: [0u8; 16],
            feature_flags: 0,
            embed: lcg_bytes(8 * hu, 0x1111),
            layers: vec![LayerWeights::Attention {
                norm1: NormSpec::RmsNorm {
                    gamma: lcg_bytes(hu, 0x2222),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                attn: AttentionWeights {
                    hidden,
                    num_q_heads: 1,
                    num_kv_heads: 1,
                    head_dim: 2,
                    w_q: lcg_bytes(hu * 2, 0x3333),
                    w_k: lcg_bytes(hu * 2, 0x4444),
                    w_v: lcg_bytes(hu * 2, 0x5555),
                    w_o: lcg_bytes(2 * hu, 0x6666),
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
                    gamma: lcg_bytes(hu, 0x7777),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                ffn: FfnWeights {
                    hidden,
                    intermediate: hidden * 2,
                    w_gate: lcg_bytes(hu * (hu * 2), 0x8888),
                    w_up: lcg_bytes(hu * (hu * 2), 0x9999),
                    w_down: lcg_bytes((hu * 2) * hu, 0xaaaa),
                },
                ffn_scales: FfnScales {
                    gate: small(),
                    up: small(),
                    mid: small(),
                    down: small(),
                },
            }],
            final_norm: Some(NormSpec::RmsNorm {
                gamma: lcg_bytes(hu, 0xbbbb),
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            }),
            rope_tables: RopeTables::identity(4, 1),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        }
    }

    #[test]
    fn comm_w_is_deterministic() {
        let m = build_model();
        let a = compute_comm_w(&m);
        let b = compute_comm_w(&m);
        assert_eq!(a, b);
    }

    #[test]
    fn comm_w_changes_when_one_weight_byte_flips() {
        let mut m = build_model();
        let original = compute_comm_w(&m);
        // Flip one byte of a layer's W_q.
        if let LayerWeights::Attention { attn, .. } = &mut m.layers[0] {
            attn.w_q[0] ^= 1;
        }
        let after = compute_comm_w(&m);
        assert_ne!(original, after);
    }

    #[test]
    fn comm_w_changes_when_scale_changes() {
        let mut m = build_model();
        let original = compute_comm_w(&m);
        if let LayerWeights::Attention { attn_scales, .. } = &mut m.layers[0] {
            attn_scales.q = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 5)).unwrap();
        }
        let after = compute_comm_w(&m);
        assert_ne!(
            original, after,
            "scale change must propagate into comm_W via the manifest hash"
        );
    }

    #[test]
    fn comm_w_changes_when_norm_eps_changes() {
        let mut m = build_model();
        let original = compute_comm_w(&m);
        if let LayerWeights::Attention { norm1, .. } = &mut m.layers[0] {
            if let NormSpec::RmsNorm { eps_q, .. } = norm1 {
                *eps_q = 2;
            }
        }
        let after = compute_comm_w(&m);
        assert_ne!(original, after);
    }

    #[test]
    fn comm_w_changes_when_lut_byte_flips() {
        let mut m = build_model();
        let original = compute_comm_w(&m);
        let mut bytes = m.softmax_lut.to_bytes();
        bytes[7] ^= 1;
        m.softmax_lut = ExpLut::from_bytes(&bytes).unwrap();
        let after = compute_comm_w(&m);
        assert_ne!(original, after);
    }

    #[test]
    fn comm_w_distinguishes_attention_vs_deltanet_at_same_dims() {
        // Build two models with the same hidden/vocab but one Attention
        // layer vs one DeltaNet layer (with matching hidden). Their
        // manifests must differ.
        use crate::deltanet::{DeltaNetScales, DeltaNetWeights};
        let mut m = build_model();
        let comm_attn = compute_comm_w(&m);
        let hu = m.dims.hidden as usize;
        m.layers[0] = LayerWeights::DeltaNet {
            norm1: NormSpec::RmsNorm {
                gamma: vec![0i8; hu],
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            },
            dnet: DeltaNetWeights {
                hidden: m.dims.hidden,
                num_qk_heads: 1,
                num_v_heads: 1,
                head_dim_qk: 2,
                head_dim_v: 2,
                w_q: vec![0i8; hu * 2],
                w_k: vec![0i8; hu * 2],
                w_v: vec![0i8; hu * 2],
                w_alpha: vec![0i8; hu],
                w_beta: vec![0i8; hu],
                w_o: vec![0i8; 2 * hu],
            },
            dnet_scales: DeltaNetScales {
                q: small(),
                k: small(),
                v: small(),
                alpha_logit: small(),
                beta_logit: small(),
                u: small(),
                decay: small(),
                update: small(),
                o: small(),
                proj: small(),
            },
            norm2: NormSpec::RmsNorm {
                gamma: vec![0i8; hu],
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            },
            ffn: FfnWeights {
                hidden: m.dims.hidden,
                intermediate: m.dims.hidden * 2,
                w_gate: vec![0i8; hu * (hu * 2)],
                w_up: vec![0i8; hu * (hu * 2)],
                w_down: vec![0i8; (hu * 2) * hu],
            },
            ffn_scales: FfnScales {
                gate: small(),
                up: small(),
                mid: small(),
                down: small(),
            },
        };
        let comm_dnet = compute_comm_w(&m);
        assert_ne!(comm_attn, comm_dnet);
    }

    #[test]
    fn canonical_weight_bytes_is_stable() {
        let m = build_model();
        let a = canonical_weight_bytes(&m);
        let b = canonical_weight_bytes(&m);
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn weights_merkle_root_handles_zero_padding() {
        // Trivially small byte stream that doesn't reach a full tile.
        let bytes = vec![1u8, 2, 3];
        let r1 = weights_merkle_root(&bytes);
        // Padding the same data to exactly WEIGHT_TILE_BYTES with zeros
        // should produce the same root because we always zero-pad.
        let mut padded = vec![0u8; WEIGHT_TILE_BYTES];
        padded[..3].copy_from_slice(&bytes);
        let r2 = weights_merkle_root(&padded);
        assert_eq!(r1, r2);
    }
}
