//! Model wrapper: weights, tables, LUTs, and the canonical-order weight
//! commitment (`comm_W`).
//!
//! Phase 2.5 (forward driver) only needs the structural parts: vocabulary
//! size, hidden width, embedding table, layer stack, optional final norm.
//! Phase 2.7 will extend this with sideloaded loading + `compute_comm_w`,
//! and the registry will pin `comm_W` per (model_id, family) tuple.

use std::path::Path;

use thiserror::Error;

use crate::activation_lut::ActivationLut;
use crate::io::{load_model, save_model, LoadError, SaveError};
use crate::layer::{LayerWeights, NormSpec};
use crate::rope::RopeTables;
use crate::softmax::ExpLut;

/// Token type. `u32` covers vocab sizes up to 2^32 — Gemma and Qwen
/// vocabularies are well under 2^20.
pub type Token = u32;

/// Static layout description for a model. All fixed at model build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelDims {
    pub vocab: u32,
    pub hidden: u32,
    pub seq_len: u32,
    pub activation_tile: u32,
}

/// Model weights + LUTs + tables. Phase 2.7 adds `comm_W` commitment and
/// load-from-disk; the structural fields here are stable.
#[derive(Debug, Clone)]
pub struct Model {
    pub dims: ModelDims,
    /// `(vocab, hidden)` row-major i8.
    pub embed: Vec<i8>,
    pub layers: Vec<LayerWeights>,
    /// Applied after every layer iff the forward driver is called with
    /// `target_layer == num_layers`. Most models commit this; setting to
    /// `None` skips it (useful for tests that exit at an interior layer).
    pub final_norm: Option<NormSpec>,
    pub rope_tables: RopeTables,
    pub softmax_lut: ExpLut,
    pub sigmoid_lut: ActivationLut,
    pub ffn_activation: ActivationLut,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelError {
    #[error("vocab, hidden, seq_len, activation_tile must all be > 0")]
    ZeroDim,
    #[error("embed length must equal vocab * hidden")]
    BadEmbedLen,
    #[error("seq_len ({seq_len}) is not a multiple of activation_tile ({tile})")]
    SeqLenNotMultipleOfTile { seq_len: u32, tile: u32 },
    #[error("hidden ({hidden}) is not a multiple of activation_tile ({tile})")]
    HiddenNotMultipleOfTile { hidden: u32, tile: u32 },
    #[error("layer {layer_idx} hidden width ({got}) does not match model hidden ({expected})")]
    LayerHiddenMismatch {
        layer_idx: u32,
        expected: u32,
        got: u32,
    },
}

impl Model {
    /// Validate cross-field invariants. Cheap to call before each forward
    /// pass to detect a malformed model early.
    pub fn validate(&self) -> Result<(), ModelError> {
        let d = self.dims;
        if d.vocab == 0 || d.hidden == 0 || d.seq_len == 0 || d.activation_tile == 0 {
            return Err(ModelError::ZeroDim);
        }
        if self.embed.len() != (d.vocab as usize) * (d.hidden as usize) {
            return Err(ModelError::BadEmbedLen);
        }
        if d.seq_len % d.activation_tile != 0 {
            return Err(ModelError::SeqLenNotMultipleOfTile {
                seq_len: d.seq_len,
                tile: d.activation_tile,
            });
        }
        if d.hidden % d.activation_tile != 0 {
            return Err(ModelError::HiddenNotMultipleOfTile {
                hidden: d.hidden,
                tile: d.activation_tile,
            });
        }
        for (i, layer) in self.layers.iter().enumerate() {
            let lh = match layer {
                LayerWeights::Attention { attn, .. } => attn.hidden,
                LayerWeights::DeltaNet { dnet, .. } => dnet.hidden,
            };
            if lh != d.hidden {
                return Err(ModelError::LayerHiddenMismatch {
                    layer_idx: i as u32,
                    expected: d.hidden,
                    got: lh,
                });
            }
        }
        Ok(())
    }

    pub fn num_layers(&self) -> u32 {
        self.layers.len() as u32
    }

    /// Serialize the model to `dir` as `manifest.bin` + `weights.bin` +
    /// `comm_w.hex`. See [`crate::io`] for the disk format.
    pub fn save(&self, dir: &Path) -> Result<(), SaveError> {
        save_model(self, dir)
    }

    /// Load a model from a sideloaded directory. Recomputes `comm_W`
    /// after parsing and rejects on mismatch with `expected_comm_w`.
    pub fn load(dir: &Path, expected_comm_w: &[u8; 32]) -> Result<Self, LoadError> {
        load_model(dir, expected_comm_w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attention::{AttentionScales, AttentionWeights};
    use crate::ffn::{FfnScales, FfnWeights};
    use crate::quant::{Scale, SCALE_DENOM_LOG2};
    use crate::rmsnorm::DEFAULT_EPS_Q;

    fn small() -> Scale {
        Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap()
    }

    fn dummy_attn_layer(hidden: u32) -> LayerWeights {
        let hu = hidden as usize;
        LayerWeights::Attention {
            norm1: NormSpec::RmsNorm {
                gamma: vec![1i8; hu],
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            },
            attn: AttentionWeights {
                hidden,
                num_q_heads: 1,
                num_kv_heads: 1,
                head_dim: 2,
                w_q: vec![0i8; hu * 2],
                w_k: vec![0i8; hu * 2],
                w_v: vec![0i8; hu * 2],
                w_o: vec![0i8; 2 * hu],
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
                gamma: vec![1i8; hu],
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            },
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
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
        }
    }

    fn dummy_model(hidden: u32, vocab: u32, seq_len: u32, tile: u32, num_layers: u32) -> Model {
        let layers: Vec<LayerWeights> = (0..num_layers).map(|_| dummy_attn_layer(hidden)).collect();
        Model {
            dims: ModelDims {
                vocab,
                hidden,
                seq_len,
                activation_tile: tile,
            },
            embed: vec![0i8; (vocab * hidden) as usize],
            layers,
            final_norm: Some(NormSpec::RmsNorm {
                gamma: vec![1i8; hidden as usize],
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            }),
            rope_tables: RopeTables::identity(seq_len, 1),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        }
    }

    #[test]
    fn validate_accepts_consistent_model() {
        let m = dummy_model(4, 8, 4, 2, 2);
        m.validate().unwrap();
    }

    #[test]
    fn validate_rejects_zero_dim() {
        let mut m = dummy_model(4, 8, 4, 2, 2);
        m.dims.vocab = 0;
        assert_eq!(m.validate().err(), Some(ModelError::ZeroDim));
    }

    #[test]
    fn validate_rejects_bad_embed_len() {
        let mut m = dummy_model(4, 8, 4, 2, 2);
        m.embed = vec![0i8; 5];
        assert_eq!(m.validate().err(), Some(ModelError::BadEmbedLen));
    }

    #[test]
    fn validate_rejects_seq_len_not_multiple_of_tile() {
        let mut m = dummy_model(4, 8, 4, 2, 1);
        m.dims.seq_len = 5;
        assert_eq!(
            m.validate().err(),
            Some(ModelError::SeqLenNotMultipleOfTile {
                seq_len: 5,
                tile: 2,
            }),
        );
    }

    #[test]
    fn validate_rejects_layer_hidden_mismatch() {
        let mut m = dummy_model(4, 8, 4, 2, 2);
        // Build a layer whose hidden differs from the model's claimed hidden.
        m.layers[0] = dummy_attn_layer(8);
        // Keep dims and embed consistent (hidden=4) so we trip the layer-mismatch
        // check rather than BadEmbedLen.
        assert_eq!(
            m.validate().err(),
            Some(ModelError::LayerHiddenMismatch {
                layer_idx: 0,
                expected: 4,
                got: 8,
            }),
        );
    }

    #[test]
    fn num_layers_reports_count() {
        let m = dummy_model(4, 8, 4, 2, 3);
        assert_eq!(m.num_layers(), 3);
    }
}
