//! Model dimension metadata.
//!
//! `ModelLayout` is the schema a verifier needs to dispatch the per-block
//! puzzle (which dim, how many layers, what activation, what block pattern).
//! It is committed inside `comm_W` so a model release pins the layout
//! alongside the weight bytes.

use crate::activation_lut::ActivationKind;

/// Hard-coded family identifier. Determines which forward-pass code path
/// applies (Gemma 4 vs Qwen 3.6 hybrid vs generic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    Gemma4,
    Qwen36,
    GenericLLM,
}

/// Per-block kind in a model's layer stack. Gemma 4 31B is `[Attn] * 60`.
/// Qwen 3.6 27B is `[Delta, Delta, Delta, Attn] * 16` — 64 layers total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Attention,
    GatedDeltaNet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormType {
    /// Root-mean-square norm, used by Gemma and Qwen.
    Rms,
    /// Mean-and-variance LayerNorm, kept for forward compatibility.
    Layer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelLayout {
    /// Canonical model identifier; equal to the model's `comm_W` root.
    pub model_id: [u8; 32],
    pub family: ModelFamily,
    pub hidden: u32,
    pub intermediate: u32,
    pub num_layers: u32,
    pub num_q_heads: u32,
    pub num_kv_heads: u32,
    pub head_dim: u32,
    pub vocab_size: u32,
    /// Sequence length the model registers for; the FFN matmul shape is
    /// `(seq_len, hidden, intermediate)`. Both Gemma 4 31B and Qwen 3.6 27B
    /// register at `seq_len = 4096`.
    pub seq_len: u32,
    pub norm_type: NormType,
    pub activation: ActivationKind,
    pub uses_rope: bool,
    /// Block pattern; length must equal `num_layers`. For Gemma 4 31B,
    /// every entry is `Attention`. For Qwen 3.6 27B, the pattern repeats
    /// `[Delta, Delta, Delta, Attn]`.
    pub block_pattern: Vec<BlockKind>,
}

impl ModelLayout {
    /// Documented Gemma 4 31B INT8 layout. Source:
    /// `https://huggingface.co/google/gemma-4-31b/blob/main/config.json`.
    /// `model_id` is left as zeros until the real weight set is published
    /// and `comm_W` is computed.
    pub fn gemma_4_31b_uncommitted() -> Self {
        Self {
            model_id: [0u8; 32],
            family: ModelFamily::Gemma4,
            hidden: 5376,
            intermediate: 21504,
            num_layers: 60,
            num_q_heads: 32,
            num_kv_heads: 16,
            head_dim: 256,
            vocab_size: 256_000,
            seq_len: 4096,
            norm_type: NormType::Rms,
            activation: ActivationKind::GeLU,
            uses_rope: true,
            block_pattern: vec![BlockKind::Attention; 60],
        }
    }

    /// Documented Qwen 3.6 27B INT8 layout. Source:
    /// `https://huggingface.co/Qwen/Qwen3.6-27B/blob/main/config.json`.
    /// 16 hybrid blocks of `[Delta, Delta, Delta, Attn]`.
    pub fn qwen_3_6_27b_uncommitted() -> Self {
        let mut block_pattern = Vec::with_capacity(64);
        for _ in 0..16 {
            block_pattern.push(BlockKind::GatedDeltaNet);
            block_pattern.push(BlockKind::GatedDeltaNet);
            block_pattern.push(BlockKind::GatedDeltaNet);
            block_pattern.push(BlockKind::Attention);
        }
        Self {
            model_id: [0u8; 32],
            family: ModelFamily::Qwen36,
            hidden: 5120,
            intermediate: 17408,
            num_layers: 64,
            num_q_heads: 24,
            num_kv_heads: 4,
            head_dim: 256,
            vocab_size: 248_320,
            seq_len: 4096,
            norm_type: NormType::Rms,
            activation: ActivationKind::SiLU,
            uses_rope: true,
            block_pattern,
        }
    }

    /// Verify that `block_pattern.len() == num_layers`.
    pub fn validate(&self) -> Result<(), LayoutError> {
        if self.block_pattern.len() as u32 != self.num_layers {
            return Err(LayoutError::BlockPatternLength {
                got: self.block_pattern.len(),
                expected: self.num_layers as usize,
            });
        }
        if self.hidden == 0 || self.intermediate == 0 {
            return Err(LayoutError::ZeroDim);
        }
        if self.num_q_heads == 0 || self.num_kv_heads == 0 || self.head_dim == 0 {
            return Err(LayoutError::ZeroDim);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LayoutError {
    #[error("block_pattern length {got} != num_layers {expected}")]
    BlockPatternLength { got: usize, expected: usize },
    #[error("model dimensions must all be > 0")]
    ZeroDim,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemma_layout_validates() {
        let l = ModelLayout::gemma_4_31b_uncommitted();
        l.validate().unwrap();
        assert_eq!(l.block_pattern.len(), 60);
        assert!(l.block_pattern.iter().all(|b| *b == BlockKind::Attention));
    }

    #[test]
    fn qwen_layout_validates() {
        let l = ModelLayout::qwen_3_6_27b_uncommitted();
        l.validate().unwrap();
        assert_eq!(l.block_pattern.len(), 64);
        // Every 4th block is Attention; rest are DeltaNet.
        for (i, b) in l.block_pattern.iter().enumerate() {
            let expected = if (i + 1) % 4 == 0 {
                BlockKind::Attention
            } else {
                BlockKind::GatedDeltaNet
            };
            assert_eq!(*b, expected, "block {i}");
        }
    }

    #[test]
    fn rejects_block_pattern_length_mismatch() {
        let mut l = ModelLayout::gemma_4_31b_uncommitted();
        l.num_layers = 59;
        assert_eq!(
            l.validate().err(),
            Some(LayoutError::BlockPatternLength {
                got: 60,
                expected: 59,
            }),
        );
    }

    #[test]
    fn rejects_zero_dim() {
        let mut l = ModelLayout::gemma_4_31b_uncommitted();
        l.hidden = 0;
        assert_eq!(l.validate().err(), Some(LayoutError::ZeroDim));
    }
}
