//! Forward-pass driver from prompt tokens through `target_layer` layers.
//!
//! Embeds tokens, runs the per-layer pipeline, optionally applies the
//! final norm if exiting at the very last layer, and records each
//! activation tensor into the supplied [`ActivationLog`].
//!
//! Recording convention: `target_layer + 1` activation tensors are
//! recorded into the log. Index `i` ∈ `0..=target_layer` is the input to
//! layer `i` (or the final post-norm output if `i == num_layers`).

use thiserror::Error;

use crate::activations::{ActivationError, ActivationLayout, ActivationLog};
use crate::layer::{forward_layer, LayerContext, LayerError};
use crate::layernorm::{layernorm, LayerNormError};
use crate::model::{Model, ModelError, Token};
use crate::quant::rescale_and_requantize;
use crate::rmsnorm::{rmsnorm, RmsNormError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ForwardError {
    #[error("prompt must be non-empty")]
    EmptyPrompt,
    #[error("prompt length ({got}) exceeds model seq_len ({max})")]
    PromptTooLong { got: u32, max: u32 },
    #[error("token id {0} out of vocab range")]
    TokenOutOfRange(Token),
    #[error("target_layer ({got}) > num_layers ({max})")]
    TargetLayerTooLarge { got: u32, max: u32 },
    #[error("activation log layout does not match model")]
    LogLayoutMismatch,
    #[error("activation log already has recorded layers; pass an empty log")]
    LogAlreadyPopulated,
    #[error("model: {0}")]
    Model(#[from] ModelError),
    #[error("layer: {0}")]
    Layer(#[from] LayerError),
    #[error("activation log: {0}")]
    Activation(#[from] ActivationError),
    #[error("rmsnorm: {0}")]
    Rms(#[from] RmsNormError),
    #[error("layernorm: {0}")]
    Ln(#[from] LayerNormError),
}

/// Run a forward pass over `prompt` tokens through the first
/// `target_layer` layers of `model` and return the resulting `(m, hidden)`
/// activation tensor.
///
/// `target_layer` must be in `[0, model.num_layers()]`. When equal to
/// `num_layers`, the final norm (if present on the model) is applied
/// before returning.
///
/// `log` MUST be a freshly-constructed [`ActivationLog`] with the same
/// `(seq_len, hidden, tile)` layout as the model's `dims.seq_len`,
/// `dims.hidden`, and `dims.activation_tile`. The driver records each
/// per-layer activation in canonical order; the caller can then `open()`
/// any tile.
pub fn forward_prefix(
    model: &Model,
    prompt: &[Token],
    target_layer: u32,
    ctx: &LayerContext,
    log: &mut ActivationLog,
) -> Result<Vec<i8>, ForwardError> {
    model.validate()?;
    if prompt.is_empty() {
        return Err(ForwardError::EmptyPrompt);
    }
    let m = prompt.len() as u32;
    if m > model.dims.seq_len {
        return Err(ForwardError::PromptTooLong {
            got: m,
            max: model.dims.seq_len,
        });
    }
    let num_layers = model.num_layers();
    if target_layer > num_layers {
        return Err(ForwardError::TargetLayerTooLarge {
            got: target_layer,
            max: num_layers,
        });
    }

    // Activation log must match the model's full-seq layout, even if the
    // current prompt is shorter — the log layout is fixed at model build.
    let want_layout = ActivationLayout {
        seq_len: model.dims.seq_len,
        hidden: model.dims.hidden,
        tile: model.dims.activation_tile,
    };
    if log.layout != want_layout {
        return Err(ForwardError::LogLayoutMismatch);
    }
    if log.num_layers() != 0 {
        return Err(ForwardError::LogAlreadyPopulated);
    }

    let hidden = model.dims.hidden;
    let hu = hidden as usize;
    let mu = m as usize;

    // Step 1: embed. Allocate the activation tensor padded to seq_len so
    // the log records at the canonical layout. Real tokens occupy rows
    // 0..m; rows m..seq_len are zero (padding tokens for this prompt).
    let seq_full = model.dims.seq_len as usize;
    let mut x_full = vec![0i8; seq_full * hu];
    for (i, &tok) in prompt.iter().enumerate() {
        if tok >= model.dims.vocab {
            return Err(ForwardError::TokenOutOfRange(tok));
        }
        let src = (tok as usize) * hu;
        let dst = i * hu;
        x_full[dst..dst + hu].copy_from_slice(&model.embed[src..src + hu]);
    }

    // Record activation for the layer-0 input (i.e. the embedded tensor).
    log.record_layer(0, &x_full)?;

    // Step 2: run layers 0..target_layer. Each layer reads from `x_full`
    // (only rows 0..m are meaningful) and writes into a scratch tensor
    // of the same shape; we then record the FULL tensor (padded rows
    // remain zero, which is what the log expects for canonical layout).
    let mut scratch = vec![0i8; seq_full * hu];
    for layer_idx in 0..target_layer {
        let layer = &model.layers[layer_idx as usize];
        let (in_view, out_view) = (&x_full[..mu * hu], &mut scratch[..mu * hu]);
        forward_layer(in_view, layer, ctx, m, out_view)?;
        // Swap: x_full now becomes the new state. Padding rows stay zero
        // because we never wrote past mu * hu.
        std::mem::swap(&mut x_full, &mut scratch);
        // Zero the scratch buffer's padding rows for cleanliness on the
        // next iteration. (forward_layer overwrites only rows 0..m.)
        for byte in scratch[mu * hu..].iter_mut() {
            *byte = 0;
        }
        log.record_layer(layer_idx + 1, &x_full)?;
    }

    // Step 3: if exiting at the very last layer and the model has a final
    // norm, apply it per-token. (It does NOT get its own activation log
    // entry — the log's last entry is the post-final-norm tensor in that
    // case, recorded above as record_layer(target_layer = num_layers).)
    if target_layer == num_layers {
        if let Some(norm) = &model.final_norm {
            let mut acc = vec![0i32; hu];
            for t in 0..mu {
                let in_row = &x_full[t * hu..(t + 1) * hu];
                match norm {
                    crate::layer::NormSpec::RmsNorm {
                        gamma,
                        eps_q,
                        post_scale,
                    } => {
                        rmsnorm(in_row, gamma, &mut acc, *eps_q)?;
                        for (a, o) in acc.iter().zip(scratch[t * hu..(t + 1) * hu].iter_mut()) {
                            *o = rescale_and_requantize(*a, *post_scale);
                        }
                    }
                    crate::layer::NormSpec::LayerNorm {
                        gamma,
                        beta,
                        eps_q,
                        post_scale,
                    } => {
                        layernorm(in_row, gamma, beta, &mut acc, *eps_q)?;
                        for (a, o) in acc.iter().zip(scratch[t * hu..(t + 1) * hu].iter_mut()) {
                            *o = rescale_and_requantize(*a, *post_scale);
                        }
                    }
                }
            }
            // Copy normed prefix back; padding stays zero.
            x_full[..mu * hu].copy_from_slice(&scratch[..mu * hu]);
        }
    }

    // Step 4: return the (m, hidden) prefix only — drop the seq_len
    // padding so callers don't see zeroed pad rows.
    x_full.truncate(mu * hu);
    Ok(x_full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_lut::ActivationLut;
    use crate::attention::{AttentionScales, AttentionWeights};
    use crate::ffn::{FfnScales, FfnWeights};
    use crate::layer::{LayerWeights, NormSpec};
    use crate::model::{Model, ModelDims};
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

    fn build_attn_layer(hidden: u32, seed: u64) -> LayerWeights {
        let hu = hidden as usize;
        LayerWeights::Attention {
            norm1: NormSpec::RmsNorm {
                gamma: lcg_bytes(hu, seed),
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            },
            attn: AttentionWeights {
                hidden,
                num_q_heads: 1,
                num_kv_heads: 1,
                head_dim: 2,
                w_q: lcg_bytes(hu * 2, seed.wrapping_add(1)),
                w_k: lcg_bytes(hu * 2, seed.wrapping_add(2)),
                w_v: lcg_bytes(hu * 2, seed.wrapping_add(3)),
                w_o: lcg_bytes(2 * hu, seed.wrapping_add(4)),
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
                gamma: lcg_bytes(hu, seed.wrapping_add(5)),
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            },
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
                w_gate: lcg_bytes(hu * (hu * 2), seed.wrapping_add(6)),
                w_up: lcg_bytes(hu * (hu * 2), seed.wrapping_add(7)),
                w_down: lcg_bytes((hu * 2) * hu, seed.wrapping_add(8)),
            },
            ffn_scales: FfnScales {
                gate: small(),
                up: small(),
                mid: small(),
                down: small(),
            },
        }
    }

    fn build_model(hidden: u32, vocab: u32, seq_len: u32, num_layers: u32, seed: u64) -> Model {
        let layers: Vec<LayerWeights> = (0..num_layers)
            .map(|i| build_attn_layer(hidden, seed.wrapping_add((i as u64) * 100)))
            .collect();
        Model {
            dims: ModelDims {
                vocab,
                hidden,
                seq_len,
                activation_tile: 2,
            },
            arch_tag: [0u8; 16],
            feature_flags: 0,
            embed: lcg_bytes((vocab * hidden) as usize, seed.wrapping_add(0xeeee)),
            layers,
            final_norm: Some(NormSpec::RmsNorm {
                gamma: lcg_bytes(hidden as usize, seed.wrapping_add(0xffff)),
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            }),
            rope_tables: RopeTables::identity(seq_len, 1),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        }
    }

    fn make_ctx<'a>(model: &'a Model) -> LayerContext<'a> {
        LayerContext {
            rope_tables: &model.rope_tables,
            softmax_lut: &model.softmax_lut,
            sigmoid_lut: &model.sigmoid_lut,
            ffn_activation: &model.ffn_activation,
        }
    }

    #[test]
    fn target_layer_zero_returns_just_embedded_prefix() {
        let model = build_model(4, 8, 4, 2, 0x1111);
        let ctx = make_ctx(&model);
        let mut log = ActivationLog::new(ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        })
        .unwrap();
        let prompt = vec![1u32, 3];
        let out = forward_prefix(&model, &prompt, 0, &ctx, &mut log).unwrap();
        // Output shape: (m=2, hidden=4) → 8 i8 elements.
        assert_eq!(out.len(), 8);
        // It must equal the embed_table[1] || embed_table[3] concatenation.
        let want_row0 = &model.embed[(1 * 4)..(2 * 4)];
        let want_row1 = &model.embed[(3 * 4)..(4 * 4)];
        assert_eq!(&out[..4], want_row0);
        assert_eq!(&out[4..], want_row1);
        // And the log has exactly one entry (layer 0 input).
        assert_eq!(log.num_layers(), 1);
    }

    #[test]
    fn target_layer_full_applies_final_norm_and_records_all() {
        let model = build_model(4, 8, 4, 2, 0x2222);
        let ctx = make_ctx(&model);
        let mut log = ActivationLog::new(ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        })
        .unwrap();
        let prompt = vec![0u32, 1, 2];
        let _out = forward_prefix(&model, &prompt, model.num_layers(), &ctx, &mut log).unwrap();
        // 2 layers + initial embed = 3 records.
        assert_eq!(log.num_layers(), model.num_layers() + 1);
    }

    #[test]
    fn empty_prompt_rejected() {
        let model = build_model(4, 8, 4, 2, 0x3333);
        let ctx = make_ctx(&model);
        let mut log = ActivationLog::new(ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        })
        .unwrap();
        assert_eq!(
            forward_prefix(&model, &[], 0, &ctx, &mut log).err(),
            Some(ForwardError::EmptyPrompt),
        );
    }

    #[test]
    fn prompt_too_long_rejected() {
        let model = build_model(4, 8, 4, 2, 0x4444);
        let ctx = make_ctx(&model);
        let mut log = ActivationLog::new(ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        })
        .unwrap();
        let too_long = vec![0u32; 5];
        assert_eq!(
            forward_prefix(&model, &too_long, 0, &ctx, &mut log).err(),
            Some(ForwardError::PromptTooLong { got: 5, max: 4 }),
        );
    }

    #[test]
    fn token_out_of_range_rejected() {
        let model = build_model(4, 8, 4, 2, 0x5555);
        let ctx = make_ctx(&model);
        let mut log = ActivationLog::new(ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        })
        .unwrap();
        let bad = vec![100u32]; // vocab is 8
        assert_eq!(
            forward_prefix(&model, &bad, 0, &ctx, &mut log).err(),
            Some(ForwardError::TokenOutOfRange(100)),
        );
    }

    #[test]
    fn target_layer_too_large_rejected() {
        let model = build_model(4, 8, 4, 2, 0x6666);
        let ctx = make_ctx(&model);
        let mut log = ActivationLog::new(ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        })
        .unwrap();
        let prompt = vec![1u32];
        assert_eq!(
            forward_prefix(&model, &prompt, 99, &ctx, &mut log).err(),
            Some(ForwardError::TargetLayerTooLarge { got: 99, max: 2 }),
        );
    }

    #[test]
    fn log_layout_must_match() {
        let model = build_model(4, 8, 4, 2, 0x7777);
        let ctx = make_ctx(&model);
        // Wrong tile size.
        let mut log = ActivationLog::new(ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 4,
        })
        .unwrap();
        let prompt = vec![1u32];
        assert_eq!(
            forward_prefix(&model, &prompt, 0, &ctx, &mut log).err(),
            Some(ForwardError::LogLayoutMismatch),
        );
    }

    #[test]
    fn log_must_be_empty() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let model = build_model(4, 8, 4, 2, 0x8888);
        let ctx = make_ctx(&model);
        let mut log = ActivationLog::new(layout).unwrap();
        // Pre-populate log with a record.
        log.record_layer(0, &vec![0i8; 16]).unwrap();
        let prompt = vec![1u32];
        assert_eq!(
            forward_prefix(&model, &prompt, 0, &ctx, &mut log).err(),
            Some(ForwardError::LogAlreadyPopulated),
        );
    }

    #[test]
    fn determinism_two_calls_match() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let model = build_model(4, 8, 4, 2, 0x9999);
        let ctx = make_ctx(&model);
        let prompt = vec![0u32, 2, 5];
        let mut log_a = ActivationLog::new(layout).unwrap();
        let mut log_b = ActivationLog::new(layout).unwrap();
        let a = forward_prefix(&model, &prompt, model.num_layers(), &ctx, &mut log_a).unwrap();
        let b = forward_prefix(&model, &prompt, model.num_layers(), &ctx, &mut log_b).unwrap();
        assert_eq!(a, b);
        assert_eq!(log_a.layer_roots, log_b.layer_roots);
    }

    #[test]
    fn prefix_to_layer_1_differs_from_prefix_to_layer_2() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let model = build_model(4, 8, 4, 3, 0xaaaa);
        let ctx = make_ctx(&model);
        let prompt = vec![1u32, 2];
        let mut log1 = ActivationLog::new(layout).unwrap();
        let mut log2 = ActivationLog::new(layout).unwrap();
        let p1 = forward_prefix(&model, &prompt, 1, &ctx, &mut log1).unwrap();
        let p2 = forward_prefix(&model, &prompt, 2, &ctx, &mut log2).unwrap();
        // They share the first activation root (layer 0 input = embed), but
        // p2 has one more record and a different return tensor.
        assert_eq!(log1.layer_roots[0], log2.layer_roots[0]);
        assert_eq!(log1.num_layers(), 2);
        assert_eq!(log2.num_layers(), 3);
        // Layer-1-input root in p1 should match layer-1-input root in p2.
        assert_eq!(log1.layer_roots[1], log2.layer_roots[1]);
        // Outputs differ.
        assert_ne!(p1, p2);
    }

    #[test]
    fn opening_a_recorded_tile_verifies_against_root() {
        use crate::activations::verify_opening;
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let model = build_model(4, 8, 4, 2, 0xbbbb);
        let ctx = make_ctx(&model);
        let prompt = vec![1u32, 3];
        let mut log = ActivationLog::new(layout).unwrap();
        forward_prefix(&model, &prompt, 1, &ctx, &mut log).unwrap();

        // Reconstruct the layer-0-input tensor (embedded prompt + zero
        // padding) so we can open and verify it.
        let hu = model.dims.hidden as usize;
        let mut tensor = vec![0i8; (model.dims.seq_len as usize) * hu];
        for (i, &tok) in prompt.iter().enumerate() {
            tensor[i * hu..(i + 1) * hu]
                .copy_from_slice(&model.embed[(tok as usize) * hu..((tok as usize) + 1) * hu]);
        }
        let opening = log.open(0, 1, &tensor).unwrap();
        verify_opening(&layout, &log.root(0).unwrap(), &opening).unwrap();
    }
}
