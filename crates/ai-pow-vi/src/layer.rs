//! Per-layer composition: `Norm → (Attention | DeltaNet) → +residual → Norm → FFN → +residual`.
//!
//! Each transformer block in Gemma 4 31B / Qwen 3.6 27B is one of two
//! flavors:
//! - **Attention block** — RMSNorm → multi-head/GQA attention → residual
//!   add → RMSNorm → SwiGLU FFN → residual add.
//! - **DeltaNet block** — same shape, attention swapped for gated DeltaNet
//!   linear-attention recurrence.
//!
//! The residual stream stays in i8 with saturating add. Per-token norm
//! produces an i32 hidden vector that's requantized to i8 before being
//! fed to the (attn|deltanet) and FFN paths. All quantization scales and
//! the choice of norm flavor are stored on `LayerWeights`; shared
//! resources (RoPE tables, softmax LUT, sigmoid LUT, FFN activation LUT)
//! live on a [`LayerContext`].

use thiserror::Error;

use crate::activation_lut::ActivationLut;
use crate::attention::{
    attention_forward, attention_forward_gemma, AttentionError, AttentionScales, AttentionWeights,
    GemmaAttentionOpts,
};
use crate::deltanet::{deltanet_forward, DeltaNetError, DeltaNetScales, DeltaNetWeights};
use crate::ffn::{ffn_forward, FfnError, FfnScales, FfnWeights};
use crate::layernorm::{layernorm, LayerNormError};
use crate::matmul_int8::{dot_int8, matmul_int8, matmul_int8_requant, requantize_vec};
use crate::quant::{
    rescale_and_requantize, round_half_to_even_div_pow2, saturate_i8, Scale, SCALE_DENOM_LOG2,
};
use crate::rmsnorm::{rmsnorm, RmsNormError};
use crate::rope::{rope_apply, RopeTables};
use crate::softmax::{softmax_int, ExpLut};
use crate::ssm::{ssm_forward, SsmError, SsmOpts};

/// Per-layer norm flavor + parameters. Both Gemma and Qwen use RMSNorm in
/// every position — `LayerNorm` is included so a future model registration
/// that wants the more general form can plug in without a hard fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormSpec {
    RmsNorm {
        /// Per-channel scale, length `hidden`.
        gamma: Vec<i8>,
        /// Integer epsilon added to `sumsq`. Default: `1`.
        eps_q: i64,
        /// Rescale norm i32 output back to i8 before downstream ops.
        post_scale: Scale,
    },
    LayerNorm {
        gamma: Vec<i8>,
        beta: Vec<i8>,
        eps_q: i64,
        post_scale: Scale,
    },
}

impl NormSpec {
    pub fn gamma_len(&self) -> usize {
        match self {
            NormSpec::RmsNorm { gamma, .. } => gamma.len(),
            NormSpec::LayerNorm { gamma, .. } => gamma.len(),
        }
    }
}

/// Tagged-union layer weights. Each variant carries a structurally
/// different transformer block — Phase 2.9 ships `Attention` and
/// `DeltaNet`; Phase 2.11 adds `Gemma` for Gemma 4 8B / 31B.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerWeights {
    Attention {
        norm1: NormSpec,
        attn: AttentionWeights,
        attn_scales: AttentionScales,
        norm2: NormSpec,
        ffn: FfnWeights,
        ffn_scales: FfnScales,
    },
    DeltaNet {
        norm1: NormSpec,
        dnet: DeltaNetWeights,
        dnet_scales: DeltaNetScales,
        norm2: NormSpec,
        ffn: FfnWeights,
        ffn_scales: FfnScales,
    },
    /// Qwen 3.6 27B standard-attention block (Phase 2.12). Same
    /// 2-norm residual structure as `Attention`, with per-head **QK
    /// norm** RMSNorm applied to Q and K before RoPE. Used for the
    /// 16/64 pure-attention blocks of Qwen 3.6 27B (block indices
    /// `[3, 7, 11, ..., 63]`). The remaining 48 blocks are hybrid
    /// attention+SSM (`QwenHybridSsm`, Phase 2.13).
    QwenStandard {
        norm1: NormSpec,
        attn: AttentionWeights,
        attn_scales: AttentionScales,
        /// Per-head Q-norm RMSNorm gamma (length = `head_dim`).
        q_norm_gamma: Vec<i8>,
        /// Per-head K-norm RMSNorm gamma.
        k_norm_gamma: Vec<i8>,
        qk_norm_eps_q: i64,
        qk_norm_post_scale: Scale,
        /// Pre-FFN RMSNorm (= GGUF `post_attention_norm.weight` —
        /// applied after the first residual add, before the FFN).
        norm2: NormSpec,
        ffn: FfnWeights,
        ffn_scales: FfnScales,
    },
    /// Qwen 3.6 27B hybrid attention + Mamba-SSM block (Phase 2.13).
    /// Used for blocks `[0, 1, 2, 4, 5, 6, ...]` (48/64) — every block
    /// not in `[3, 7, 11, ..., 63]`. The pre-attn norm feeds **two**
    /// parallel paths that are summed before the residual add:
    /// (a) gated attention with fused QKV and a sigmoid `attn_gate`;
    /// (b) Mamba SSM with a 1D causal conv, per-token α/β gates, a
    /// state-transition diagonal `ssm_a`, per-channel `ssm_dt`,
    /// `ssm_norm`, and a final `ssm_out` projection.
    ///
    /// Phase 2.13 ships the variant, disk format, and forward
    /// implementation. The forward composes [`crate::ssm::ssm_forward`]
    /// (Mamba path) with an inlined gated-attention forward (fused QKV
    /// split + QK norm + RoPE + causal softmax + sigmoid `attn_gate`
    /// element-wise multiply + `attn_out` projection); the two paths are
    /// summed before the residual add.
    QwenHybridSsm {
        norm1: NormSpec,
        // Gated attention path:
        /// `(hidden, q_dim + k_dim + v_dim)` col-major; sizes derived
        /// from `num_q_heads`, `num_kv_heads`, `head_dim` (read from
        /// the GGUF KV at quantization time).
        attn_qkv_fused: Vec<i8>,
        /// `(hidden, num_q_heads * head_dim)` per-head attention
        /// gate (multiplied into the per-head attention output via
        /// sigmoid).
        attn_gate: Vec<i8>,
        /// Output projection: `(num_q_heads * head_dim, hidden)`.
        attn_out: Vec<i8>,
        num_q_heads: u32,
        num_kv_heads: u32,
        head_dim: u32,
        attn_scales: AttentionScales,
        // Per-head Q/K norm (Qwen 3.6 hybrid blocks have these too).
        q_norm_gamma: Vec<i8>,
        k_norm_gamma: Vec<i8>,
        qk_norm_eps_q: i64,
        qk_norm_post_scale: Scale,
        // SSM path:
        /// State-transition diagonal, length `num_v_heads`.
        ssm_a: Vec<i8>,
        /// Per-token decay gate projection (hidden, num_v_heads).
        ssm_alpha: Vec<i8>,
        /// Per-token update gate projection (hidden, num_v_heads).
        ssm_beta: Vec<i8>,
        /// 1D causal conv kernel: `(kernel_size, hidden)`. For Qwen
        /// the kernel is size 4.
        ssm_conv1d: Vec<i8>,
        /// Per-channel time-step bias, length `num_v_heads`.
        ssm_dt: Vec<i8>,
        /// Pre-output RMSNorm gamma (length `head_dim`).
        ssm_norm_gamma: Vec<i8>,
        ssm_norm_eps_q: i64,
        ssm_norm_post_scale: Scale,
        /// Output projection: `(num_v_heads * ssm_head_dim, hidden)`.
        ssm_out: Vec<i8>,
        num_v_heads: u32,
        /// SSM per-V-head state width. Differs from attention's
        /// `head_dim` in real Qwen 3.6 27B (attn=256, ssm=128). Set
        /// equal to `head_dim` for the synthetic-mini fixture and
        /// any model where the two paths share the same head_dim.
        ssm_head_dim: u32,
        ssm_kernel_size: u32,
        ssm_scales: DeltaNetScales, // reuse the DeltaNet scale set
        // Shared parts of the block:
        norm2: NormSpec,
        ffn: FfnWeights,
        ffn_scales: FfnScales,
    },
    /// Gemma 4 block (Phase 2.11). Same 2-residual structure as
    /// `Attention`, but with four RMS norms per block (pre-attn,
    /// post-attn, pre-ffn, post-ffn), a per-head **QK norm** RMSNorm
    /// applied to Q and K before RoPE, optional sliding-window
    /// attention, and optional input gating + output scaling. The
    /// attention sublayer is the same `AttentionWeights` the
    /// `Attention` variant uses (so the matmul + softmax + V-sum
    /// machinery is shared).
    Gemma {
        /// Pre-attention RMSNorm.
        norm1: NormSpec,
        /// Standard MHA/GQA weights. RoPE, causal mask, etc. are
        /// applied as in [`crate::attention::attention_forward`].
        attn: AttentionWeights,
        attn_scales: AttentionScales,
        /// Per-head RMSNorm gammas applied to Q after the projection
        /// and before RoPE. Length = `head_dim`. Same gamma is used
        /// for every Q head (Gemma's published checkpoint shape).
        q_norm_gamma: Vec<i8>,
        /// Same for K.
        k_norm_gamma: Vec<i8>,
        /// Integer epsilon for the QK norm; default 1.
        qk_norm_eps_q: i64,
        /// Post-norm requantize scale for the QK norm output.
        qk_norm_post_scale: Scale,
        /// RMSNorm applied to the attention output **before** the
        /// residual add. Gemma 4's `post_attention_norm`.
        post_attn_norm: NormSpec,
        /// Pre-FFN RMSNorm.
        norm2: NormSpec,
        /// SwiGLU FFN; same as the other variants.
        ffn: FfnWeights,
        ffn_scales: FfnScales,
        /// RMSNorm applied to the FFN output **before** the residual
        /// add. Gemma 4's `post_ffw_norm`.
        post_ffn_norm: NormSpec,
        /// Sliding-window radius (in tokens). `None` is full causal.
        /// When `Some(w)`, the attention causal mask is bounded below
        /// by `j >= i.saturating_sub(w - 1)`.
        sliding_window: Option<u32>,
        /// Per-channel input gating; length = `hidden`. Multiplies the
        /// layer input element-wise (i.e. `x' = saturate(x * gate /
        /// 127)`). `None` skips the step. Gemma 4 ships this; future
        /// arches may not.
        inp_gate: Option<Vec<i8>>,
        /// Per-channel output scaling applied to the FFN residual
        /// stream before the second residual add; length = `hidden`.
        /// `None` skips.
        layer_output_scale: Option<Vec<i8>>,
    },
}

/// Shared resources used by every layer in a forward pass. Borrowed for
/// the duration of one [`forward_layer`] call.
#[derive(Debug, Clone, Copy)]
pub struct LayerContext<'a> {
    /// Required by attention layers; ignored by DeltaNet layers.
    pub rope_tables: &'a RopeTables,
    /// Required by attention layers; ignored by DeltaNet layers.
    pub softmax_lut: &'a ExpLut,
    /// Required by DeltaNet layers; ignored by attention layers.
    pub sigmoid_lut: &'a ActivationLut,
    /// Required by every layer's FFN sublayer.
    pub ffn_activation: &'a ActivationLut,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LayerError {
    #[error("input length must equal m * hidden")]
    BadInputLen,
    #[error("output length must equal m * hidden")]
    BadOutputLen,
    #[error("hidden must be > 0")]
    ZeroHidden,
    #[error("m must be > 0")]
    ZeroM,
    #[error("norm gamma length must equal hidden")]
    BadNormShape,
    #[error("rmsnorm: {0}")]
    Rms(#[from] RmsNormError),
    #[error("layernorm: {0}")]
    Ln(#[from] LayerNormError),
    #[error("attention: {0}")]
    Attn(#[from] AttentionError),
    #[error("deltanet: {0}")]
    Dnet(#[from] DeltaNetError),
    #[error("ffn: {0}")]
    Ffn(#[from] FfnError),
    #[error("ssm: {0}")]
    Ssm(#[from] SsmError),
    #[error("matmul: {0}")]
    Matmul(#[from] crate::matmul_int8::MatmulError),
    #[error("rope: {0}")]
    Rope(#[from] crate::rope::RopeError),
    #[error("softmax: {0}")]
    Softmax(#[from] crate::softmax::SoftmaxError),
}

/// Apply `norm` independently to each row of `input` (`(m, hidden)` row-major)
/// and write the i8-requantized result into `output`.
fn apply_norm_per_token(
    norm: &NormSpec,
    input: &[i8],
    m: u32,
    hidden: u32,
    output: &mut [i8],
) -> Result<(), LayerError> {
    let mu = m as usize;
    let hu = hidden as usize;
    if input.len() != mu * hu {
        return Err(LayerError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(LayerError::BadOutputLen);
    }
    if norm.gamma_len() != hu {
        return Err(LayerError::BadNormShape);
    }

    let mut acc = vec![0i32; hu];
    for t in 0..mu {
        let in_row = &input[t * hu..(t + 1) * hu];
        match norm {
            NormSpec::RmsNorm {
                gamma,
                eps_q,
                post_scale,
            } => {
                rmsnorm(in_row, gamma, &mut acc, *eps_q)?;
                let out_row = &mut output[t * hu..(t + 1) * hu];
                for (a, o) in acc.iter().zip(out_row.iter_mut()) {
                    *o = rescale_and_requantize(*a, *post_scale);
                }
            }
            NormSpec::LayerNorm {
                gamma,
                beta,
                eps_q,
                post_scale,
            } => {
                if beta.len() != hu {
                    return Err(LayerError::BadNormShape);
                }
                layernorm(in_row, gamma, beta, &mut acc, *eps_q)?;
                let out_row = &mut output[t * hu..(t + 1) * hu];
                for (a, o) in acc.iter().zip(out_row.iter_mut()) {
                    *o = rescale_and_requantize(*a, *post_scale);
                }
            }
        }
    }
    Ok(())
}

/// `dst[i] = saturate_i8(dst[i] + addend[i])`.
fn add_residual_inplace(dst: &mut [i8], addend: &[i8]) {
    debug_assert_eq!(dst.len(), addend.len());
    for (d, a) in dst.iter_mut().zip(addend.iter()) {
        let sum = (*d as i32) + (*a as i32);
        *d = saturate_i8(sum as i64);
    }
}

/// Forward one transformer layer.
///
/// `input` is `(m, hidden)` row-major i8; `output` is the same shape.
///
/// The per-layer flow is, identically for both block flavors:
/// ```text
/// x  = input
/// y  = sublayer(norm1(x))         # attention or deltanet
/// x' = saturate_i8(x + y)
/// y' = ffn(norm2(x'))
/// out = saturate_i8(x' + y')
/// ```
pub fn forward_layer(
    input: &[i8],
    layer: &LayerWeights,
    ctx: &LayerContext,
    m: u32,
    output: &mut [i8],
) -> Result<(), LayerError> {
    if m == 0 {
        return Err(LayerError::ZeroM);
    }
    // Gemma's 4-norm structure and Qwen-standard's QK-norm variant
    // don't fit the 2-norm template below; dispatch out early.
    if let LayerWeights::Gemma { .. } = layer {
        return forward_gemma_layer(input, layer, ctx, m, output);
    }
    if let LayerWeights::QwenStandard { .. } = layer {
        return forward_qwen_standard_layer(input, layer, ctx, m, output);
    }
    if let LayerWeights::QwenHybridSsm { .. } = layer {
        return forward_qwen_hybrid_ssm_layer(input, layer, ctx, m, output);
    }
    let mu = m as usize;
    // Hidden is determined by the layer's attention/deltanet sublayer.
    let hidden = match layer {
        LayerWeights::Attention { attn, .. } => attn.hidden,
        LayerWeights::DeltaNet { dnet, .. } => dnet.hidden,
        LayerWeights::Gemma { .. }
        | LayerWeights::QwenStandard { .. }
        | LayerWeights::QwenHybridSsm { .. } => unreachable!(),
    };
    if hidden == 0 {
        return Err(LayerError::ZeroHidden);
    }
    let hu = hidden as usize;
    if input.len() != mu * hu {
        return Err(LayerError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(LayerError::BadOutputLen);
    }

    // Step 1: norm1(input) → normed1 (i8 per-token-normalized).
    let mut normed1 = vec![0i8; mu * hu];
    let (norm1, norm2) = match layer {
        LayerWeights::Attention { norm1, norm2, .. } => (norm1, norm2),
        LayerWeights::DeltaNet { norm1, norm2, .. } => (norm1, norm2),
        LayerWeights::Gemma { .. }
        | LayerWeights::QwenStandard { .. }
        | LayerWeights::QwenHybridSsm { .. } => unreachable!(),
    };
    apply_norm_per_token(norm1, input, m, hidden, &mut normed1)?;

    // Step 2: sublayer(normed1) → sub_out.
    let mut sub_out = vec![0i8; mu * hu];
    match layer {
        LayerWeights::Attention {
            attn, attn_scales, ..
        } => {
            attention_forward(
                &normed1, attn, *attn_scales, ctx.rope_tables, ctx.softmax_lut, m, &mut sub_out,
            )?;
        }
        LayerWeights::DeltaNet {
            dnet, dnet_scales, ..
        } => {
            deltanet_forward(
                &normed1, dnet, *dnet_scales, ctx.sigmoid_lut, m, &mut sub_out,
            )?;
        }
        LayerWeights::Gemma { .. }
        | LayerWeights::QwenStandard { .. }
        | LayerWeights::QwenHybridSsm { .. } => unreachable!(),
    }
    drop(normed1);

    // Step 3: residual1 = input + sub_out (saturating). Reuse sub_out
    // buffer in place: sub_out[i] += input[i].
    add_residual_inplace(&mut sub_out, input);
    let residual1 = sub_out; // rename for clarity

    // Step 4: norm2(residual1) → normed2.
    let mut normed2 = vec![0i8; mu * hu];
    apply_norm_per_token(norm2, &residual1, m, hidden, &mut normed2)?;

    // Step 5: ffn(normed2) → ffn_out.
    let (ffn_w, ffn_scales) = match layer {
        LayerWeights::Attention {
            ffn, ffn_scales, ..
        } => (ffn, *ffn_scales),
        LayerWeights::DeltaNet {
            ffn, ffn_scales, ..
        } => (ffn, *ffn_scales),
        LayerWeights::Gemma { .. }
        | LayerWeights::QwenStandard { .. }
        | LayerWeights::QwenHybridSsm { .. } => unreachable!(),
    };
    let mut ffn_out = vec![0i8; mu * hu];
    ffn_forward(
        &normed2, ffn_w, ctx.ffn_activation, ffn_scales, m, &mut ffn_out,
    )?;
    drop(normed2);

    // Step 6: output = residual1 + ffn_out (saturating).
    output.copy_from_slice(&residual1);
    add_residual_inplace(output, &ffn_out);

    Ok(())
}

/// Per-channel multiply-with-scale-and-saturate. `out[i, c] = saturate_i8(
/// round_half_to_nearest_away_from_zero(in[i, c] * scale[c] / 127))`.
/// Used for `inp_gate` and `layer_output_scale` in Gemma 4.
///
/// Symmetric half-up rounding so positive and negative values are
/// treated identically across Rust (truncating `/`) and Python (floor
/// `//`) — see Phase 2.11 numpy parity discussion.
fn apply_channelwise_scale(input: &mut [i8], scale: &[i8], hidden: usize) {
    debug_assert_eq!(input.len() % hidden, 0);
    debug_assert_eq!(scale.len(), hidden);
    let m = input.len() / hidden;
    for t in 0..m {
        for c in 0..hidden {
            let v = (input[t * hidden + c] as i32) * (scale[c] as i32);
            let abs_v = v.unsigned_abs() as i64;
            let q = ((abs_v + 63) / 127) as i32;
            let signed = if v < 0 { -q } else { q };
            input[t * hidden + c] = signed.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        }
    }
}

/// Phase 2.11: Gemma-style transformer layer forward.
///
/// Composition (one residual stream, four norms):
/// ```text
/// x  = (inp_gate.is_some() ? input · inp_gate : input)
/// y  = post_attn_norm( attention_gemma( norm1(x) ) )   # QK-norm inside attn
/// x' = saturate_i8(x + y)
/// y' = post_ffn_norm( ffn( norm2(x') ) )
/// y' = (layer_output_scale.is_some() ? y' · scale : y')
/// out = saturate_i8(x' + y')
/// ```
fn forward_gemma_layer(
    input: &[i8],
    layer: &LayerWeights,
    ctx: &LayerContext,
    m: u32,
    output: &mut [i8],
) -> Result<(), LayerError> {
    let (
        norm1,
        attn,
        attn_scales,
        q_norm_gamma,
        k_norm_gamma,
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
    ) = match layer {
        LayerWeights::Gemma {
            norm1,
            attn,
            attn_scales,
            q_norm_gamma,
            k_norm_gamma,
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
        } => (
            norm1, attn, attn_scales, q_norm_gamma, k_norm_gamma, *qk_norm_eps_q,
            *qk_norm_post_scale, post_attn_norm, norm2, ffn, ffn_scales, post_ffn_norm,
            *sliding_window, inp_gate, layer_output_scale,
        ),
        _ => unreachable!("forward_gemma_layer requires LayerWeights::Gemma"),
    };

    let hidden = attn.hidden;
    if hidden == 0 {
        return Err(LayerError::ZeroHidden);
    }
    let mu = m as usize;
    let hu = hidden as usize;
    if input.len() != mu * hu {
        return Err(LayerError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(LayerError::BadOutputLen);
    }
    if q_norm_gamma.len() != attn.head_dim as usize || k_norm_gamma.len() != attn.head_dim as usize
    {
        return Err(LayerError::BadNormShape);
    }

    // Step 0: optional input gating.
    let mut x = input.to_vec();
    if let Some(g) = inp_gate {
        if g.len() != hu {
            return Err(LayerError::BadNormShape);
        }
        apply_channelwise_scale(&mut x, g, hu);
    }

    // Step 1: norm1(x) → normed1.
    let mut normed1 = vec![0i8; mu * hu];
    apply_norm_per_token(norm1, &x, m, hidden, &mut normed1)?;

    // Step 2: gemma attention (with QK norm + sliding window).
    let mut sub_out = vec![0i8; mu * hu];
    let opts = GemmaAttentionOpts {
        q_norm_gamma: Some(q_norm_gamma),
        k_norm_gamma: Some(k_norm_gamma),
        qk_norm_eps_q,
        qk_norm_post_scale,
        sliding_window,
    };
    attention_forward_gemma(
        &normed1, attn, *attn_scales, ctx.rope_tables, ctx.softmax_lut, opts, m, &mut sub_out,
    )?;
    drop(normed1);

    // Step 3: post-attn norm (Gemma's `post_attention_norm`).
    let mut normed_post_attn = vec![0i8; mu * hu];
    apply_norm_per_token(post_attn_norm, &sub_out, m, hidden, &mut normed_post_attn)?;
    drop(sub_out);

    // Step 4: residual1 = x + post-attn (saturating).
    let mut residual1 = x;
    add_residual_inplace(&mut residual1, &normed_post_attn);
    drop(normed_post_attn);

    // Step 5: norm2(residual1) → normed2.
    let mut normed2 = vec![0i8; mu * hu];
    apply_norm_per_token(norm2, &residual1, m, hidden, &mut normed2)?;

    // Step 6: FFN.
    let mut ffn_out = vec![0i8; mu * hu];
    ffn_forward(
        &normed2, ffn, ctx.ffn_activation, *ffn_scales, m, &mut ffn_out,
    )?;
    drop(normed2);

    // Step 7: post-FFN norm.
    let mut normed_post_ffn = vec![0i8; mu * hu];
    apply_norm_per_token(post_ffn_norm, &ffn_out, m, hidden, &mut normed_post_ffn)?;
    drop(ffn_out);

    // Step 8: optional layer_output_scale (per-channel multiply).
    if let Some(s) = layer_output_scale {
        if s.len() != hu {
            return Err(LayerError::BadNormShape);
        }
        apply_channelwise_scale(&mut normed_post_ffn, s, hu);
    }

    // Step 9: output = residual1 + scaled-post-ffn.
    output.copy_from_slice(&residual1);
    add_residual_inplace(output, &normed_post_ffn);

    Ok(())
}

/// Phase 2.12: Qwen 3.6 27B standard-attention layer forward.
///
/// Composition (same 2-norm residual structure as `Attention`, but
/// with QK-norm inside the attention sublayer):
/// ```text
/// y  = attention_gemma(norm1(x), q_norm, k_norm, sliding_window=None)
/// x' = saturate_i8(x + y)
/// y' = ffn(norm2(x'))
/// out = saturate_i8(x' + y')
/// ```
fn forward_qwen_standard_layer(
    input: &[i8],
    layer: &LayerWeights,
    ctx: &LayerContext,
    m: u32,
    output: &mut [i8],
) -> Result<(), LayerError> {
    let (
        norm1,
        attn,
        attn_scales,
        q_norm_gamma,
        k_norm_gamma,
        qk_norm_eps_q,
        qk_norm_post_scale,
        norm2,
        ffn,
        ffn_scales,
    ) = match layer {
        LayerWeights::QwenStandard {
            norm1,
            attn,
            attn_scales,
            q_norm_gamma,
            k_norm_gamma,
            qk_norm_eps_q,
            qk_norm_post_scale,
            norm2,
            ffn,
            ffn_scales,
        } => (
            norm1, attn, attn_scales, q_norm_gamma, k_norm_gamma, *qk_norm_eps_q,
            *qk_norm_post_scale, norm2, ffn, ffn_scales,
        ),
        _ => unreachable!("forward_qwen_standard_layer requires QwenStandard"),
    };

    let hidden = attn.hidden;
    if hidden == 0 {
        return Err(LayerError::ZeroHidden);
    }
    let mu = m as usize;
    let hu = hidden as usize;
    if input.len() != mu * hu {
        return Err(LayerError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(LayerError::BadOutputLen);
    }
    if q_norm_gamma.len() != attn.head_dim as usize || k_norm_gamma.len() != attn.head_dim as usize
    {
        return Err(LayerError::BadNormShape);
    }

    let mut normed1 = vec![0i8; mu * hu];
    apply_norm_per_token(norm1, input, m, hidden, &mut normed1)?;

    let mut sub = vec![0i8; mu * hu];
    let opts = GemmaAttentionOpts {
        q_norm_gamma: Some(q_norm_gamma),
        k_norm_gamma: Some(k_norm_gamma),
        qk_norm_eps_q,
        qk_norm_post_scale,
        sliding_window: None,
    };
    attention_forward_gemma(
        &normed1, attn, *attn_scales, ctx.rope_tables, ctx.softmax_lut, opts, m, &mut sub,
    )?;
    drop(normed1);

    add_residual_inplace(&mut sub, input);
    let residual1 = sub;

    let mut normed2 = vec![0i8; mu * hu];
    apply_norm_per_token(norm2, &residual1, m, hidden, &mut normed2)?;

    let mut ffn_out = vec![0i8; mu * hu];
    ffn_forward(
        &normed2, ffn, ctx.ffn_activation, *ffn_scales, m, &mut ffn_out,
    )?;
    drop(normed2);

    output.copy_from_slice(&residual1);
    add_residual_inplace(output, &ffn_out);
    Ok(())
}

/// Phase 2.13 part 2: Qwen 3.6 27B hybrid attention + Mamba-SSM block
/// forward.
///
/// The pre-attn norm output feeds **two parallel sublayers** that are
/// summed before the residual add:
/// 1. Gated attention with fused QKV split, QK norm, RoPE, causal softmax,
///    V-weighted sum, sigmoid `attn_gate` element-wise multiply, and
///    output projection (`attn_out`).
/// 2. Mamba SSM (1D causal conv + per-token α/β gating + selective state
///    recurrence + per-V-head RMSNorm + output projection (`ssm_out`)).
///
/// Composition (one residual stream, two norms — like the standard
/// Attention block, but with the parallel-paths sum inside the first
/// sublayer):
/// ```text
/// y_attn = gated_attention(norm1(x))
/// y_ssm  = ssm_forward(norm1(x))
/// y      = saturate_i8(y_attn + y_ssm)
/// x'     = saturate_i8(x + y)
/// y'     = ffn(norm2(x'))
/// out    = saturate_i8(x' + y')
/// ```
#[allow(clippy::too_many_arguments)]
fn forward_qwen_hybrid_ssm_layer(
    input: &[i8],
    layer: &LayerWeights,
    ctx: &LayerContext,
    m: u32,
    output: &mut [i8],
) -> Result<(), LayerError> {
    let (
        norm1,
        attn_qkv_fused,
        attn_gate,
        attn_out,
        num_q_heads,
        num_kv_heads,
        head_dim,
        attn_scales,
        q_norm_gamma,
        k_norm_gamma,
        qk_norm_eps_q,
        qk_norm_post_scale,
        ssm_a,
        ssm_alpha,
        ssm_beta,
        ssm_conv1d,
        ssm_dt,
        ssm_norm_gamma,
        ssm_norm_eps_q,
        ssm_norm_post_scale,
        ssm_out,
        num_v_heads,
        ssm_head_dim,
        ssm_kernel_size,
        ssm_scales,
        norm2,
        ffn,
        ffn_scales,
    ) = match layer {
        LayerWeights::QwenHybridSsm {
            norm1,
            attn_qkv_fused,
            attn_gate,
            attn_out,
            num_q_heads,
            num_kv_heads,
            head_dim,
            attn_scales,
            q_norm_gamma,
            k_norm_gamma,
            qk_norm_eps_q,
            qk_norm_post_scale,
            ssm_a,
            ssm_alpha,
            ssm_beta,
            ssm_conv1d,
            ssm_dt,
            ssm_norm_gamma,
            ssm_norm_eps_q,
            ssm_norm_post_scale,
            ssm_out,
            num_v_heads,
            ssm_head_dim,
            ssm_kernel_size,
            ssm_scales,
            norm2,
            ffn,
            ffn_scales,
        } => (
            norm1, attn_qkv_fused, attn_gate, attn_out, *num_q_heads, *num_kv_heads, *head_dim,
            *attn_scales, q_norm_gamma, k_norm_gamma, *qk_norm_eps_q, *qk_norm_post_scale, ssm_a,
            ssm_alpha, ssm_beta, ssm_conv1d, ssm_dt, ssm_norm_gamma, *ssm_norm_eps_q,
            *ssm_norm_post_scale, ssm_out, *num_v_heads, *ssm_head_dim, *ssm_kernel_size,
            *ssm_scales, norm2, ffn, *ffn_scales,
        ),
        _ => unreachable!("forward_qwen_hybrid_ssm_layer requires QwenHybridSsm"),
    };

    let hidden = ffn.hidden;
    if hidden == 0 {
        return Err(LayerError::ZeroHidden);
    }
    let mu = m as usize;
    let hu = hidden as usize;
    if input.len() != mu * hu {
        return Err(LayerError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(LayerError::BadOutputLen);
    }
    if q_norm_gamma.len() != head_dim as usize || k_norm_gamma.len() != head_dim as usize {
        return Err(LayerError::BadNormShape);
    }

    // Step 1: norm1(input) → normed1.
    let mut normed1 = vec![0i8; mu * hu];
    apply_norm_per_token(norm1, input, m, hidden, &mut normed1)?;

    // Phase B.1: when the gated-attention output dim
    // (num_q_heads * head_dim) equals the SSM output dim
    // (num_v_heads * ssm_head_dim) AND attn_out bytes == ssm_out
    // bytes, we can sum per-head outputs and project once through
    // the shared `ssm.w_out` matrix — saves one INT8 rounding step.
    // This matches real Qwen 3.6 27B's "single shared output
    // projection" architecture.
    let q_dim = (num_q_heads * head_dim) as usize;
    let ssm_inter_dim = (num_v_heads * ssm_head_dim) as usize;
    let use_shared_projection = q_dim == ssm_inter_dim && attn_out == ssm_out;
    let opts = SsmOpts {
        ssm_a,
        ssm_alpha,
        ssm_beta,
        ssm_conv1d,
        ssm_dt,
        ssm_norm_gamma,
        ssm_norm_eps_q,
        ssm_norm_post_scale,
        ssm_out,
        num_v_heads,
        head_dim: ssm_head_dim,
        kernel_size: ssm_kernel_size,
        scales: ssm_scales,
        sigmoid_lut: ctx.sigmoid_lut,
    };

    let mut sub_out = if use_shared_projection {
        // Compute attn_inter (m, q_dim) and ssm_inter (m, q_dim);
        // sum element-wise; project once.
        let mut attn_inter = gated_attention_inter(
            &normed1,
            attn_qkv_fused,
            attn_gate,
            hidden,
            num_q_heads,
            num_kv_heads,
            head_dim,
            attn_scales,
            q_norm_gamma,
            k_norm_gamma,
            qk_norm_eps_q,
            qk_norm_post_scale,
            ctx.rope_tables,
            ctx.softmax_lut,
            ctx.sigmoid_lut,
            m,
        )?;
        let ssm_inter = crate::ssm::ssm_forward_inter(&normed1, hidden, m, opts)?;
        // Sum element-wise.
        for k in 0..attn_inter.len() {
            attn_inter[k] = saturate_i8((attn_inter[k] as i64) + (ssm_inter[k] as i64));
        }
        drop(ssm_inter);
        drop(normed1);
        // Single shared output projection via ssm_out (= attn_out).
        let mut out = vec![0i8; mu * hu];
        matmul_int8_requant(
            &attn_inter,
            ssm_out,
            m,
            q_dim as u32,
            hidden,
            ssm_scales.proj,
            &mut out,
        )?;
        out
    } else {
        // Legacy two-projection path (separate attn_out / ssm_out).
        let mut y_attn = vec![0i8; mu * hu];
        gated_attention_forward(
            &normed1, attn_qkv_fused, attn_gate, attn_out, hidden, num_q_heads, num_kv_heads,
            head_dim, attn_scales, q_norm_gamma, k_norm_gamma, qk_norm_eps_q,
            qk_norm_post_scale, ctx.rope_tables, ctx.softmax_lut, ctx.sigmoid_lut, m,
            &mut y_attn,
        )?;
        let mut y_ssm = vec![0i8; mu * hu];
        ssm_forward(&normed1, hidden, m, opts, &mut y_ssm)?;
        drop(normed1);
        add_residual_inplace(&mut y_attn, &y_ssm);
        drop(y_ssm);
        y_attn
    };

    // Step 5: residual1 = input + sub_out.
    add_residual_inplace(&mut sub_out, input);
    let residual1 = sub_out;

    // Step 6: norm2(residual1) → normed2.
    let mut normed2 = vec![0i8; mu * hu];
    apply_norm_per_token(norm2, &residual1, m, hidden, &mut normed2)?;

    // Step 7: FFN.
    let mut ffn_out = vec![0i8; mu * hu];
    ffn_forward(
        &normed2, ffn, ctx.ffn_activation, ffn_scales, m, &mut ffn_out,
    )?;
    drop(normed2);

    // Step 8: output = residual1 + ffn_out (saturating).
    output.copy_from_slice(&residual1);
    add_residual_inplace(output, &ffn_out);
    Ok(())
}

/// Inlined gated-attention forward used by [`forward_qwen_hybrid_ssm_layer`].
///
/// Equivalent to [`crate::attention::attention_forward_gemma`] with
/// `sliding_window=None`, plus:
/// - **Fused QKV**: the `(hidden, q_dim + k_dim + v_dim)` `attn_qkv_fused`
///   matrix is sliced in-place into Q, K, V column-major sub-matrices
///   (each column is `hidden` bytes; columns 0..q_dim → Q,
///   q_dim..q_dim+k_dim → K, q_dim+k_dim..total → V).
/// - **`attn_gate`**: a per-head sigmoid gate computed from `input` and
///   element-wise multiplied (with banker's rounding through `attn_out`
///   scale) into the attention output before the final projection.
///
/// Inlined rather than punching a "skip output projection / apply gate"
/// option into [`crate::attention`] because (a) this is the only caller,
/// and (b) keeping the attention-module API small makes future SIMD jets
/// easier to drop in.
#[allow(clippy::too_many_arguments)]
fn gated_attention_forward(
    input: &[i8],
    attn_qkv_fused: &[i8],
    attn_gate: &[i8],
    attn_out_w: &[i8],
    hidden: u32,
    num_q_heads: u32,
    num_kv_heads: u32,
    head_dim: u32,
    scales: AttentionScales,
    q_norm_gamma: &[i8],
    k_norm_gamma: &[i8],
    qk_norm_eps_q: i64,
    qk_norm_post_scale: Scale,
    rope_tables: &RopeTables,
    softmax_lut: &ExpLut,
    sigmoid_lut: &ActivationLut,
    m: u32,
    output: &mut [i8],
) -> Result<(), LayerError> {
    if hidden == 0 || num_q_heads == 0 || head_dim == 0 || m == 0 {
        return Err(LayerError::ZeroHidden);
    }
    if num_kv_heads == 0 || num_kv_heads > num_q_heads || num_q_heads % num_kv_heads != 0 {
        return Err(LayerError::Attn(AttentionError::BadKvHeads));
    }
    if head_dim % 2 != 0 {
        return Err(LayerError::Attn(AttentionError::HeadDimOdd));
    }
    if rope_tables.half_head_dim != head_dim / 2 {
        return Err(LayerError::Attn(AttentionError::RopeHalfHeadDimMismatch));
    }
    if rope_tables.seq_len < m {
        return Err(LayerError::Attn(AttentionError::RopeSeqLenTooShort));
    }
    let mu = m as usize;
    let hu = hidden as usize;
    let num_qu = num_q_heads as usize;
    let num_kvu = num_kv_heads as usize;
    let hdu = head_dim as usize;
    let q_dim = num_qu * hdu;
    let kv_dim = num_kvu * hdu;
    let total_qkv = q_dim + kv_dim + kv_dim;

    if input.len() != mu * hu {
        return Err(LayerError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(LayerError::BadOutputLen);
    }
    if attn_qkv_fused.len() != hu * total_qkv {
        return Err(LayerError::Attn(AttentionError::BadWqLen));
    }
    if attn_gate.len() != hu * q_dim {
        return Err(LayerError::Attn(AttentionError::BadWqLen));
    }
    if attn_out_w.len() != q_dim * hu {
        return Err(LayerError::Attn(AttentionError::BadWoLen));
    }

    // Split fused QKV into Q, K, V column-major sub-matrices. Column-major
    // layout means columns are contiguous, so a contiguous slice of the
    // fused matrix is a valid column-major sub-matrix.
    let w_q = &attn_qkv_fused[0..hu * q_dim];
    let w_k = &attn_qkv_fused[hu * q_dim..hu * (q_dim + kv_dim)];
    let w_v = &attn_qkv_fused[hu * (q_dim + kv_dim)..hu * total_qkv];

    // Q projection.
    let mut q_acc = vec![0i32; mu * q_dim];
    matmul_int8(input, w_q, m, hidden, num_q_heads * head_dim, &mut q_acc)?;
    let mut q_i8 = vec![0i8; mu * q_dim];
    requantize_vec(&q_acc, scales.q, &mut q_i8)?;
    drop(q_acc);

    // K projection.
    let mut k_acc = vec![0i32; mu * kv_dim];
    matmul_int8(input, w_k, m, hidden, num_kv_heads * head_dim, &mut k_acc)?;
    let mut k_i8 = vec![0i8; mu * kv_dim];
    requantize_vec(&k_acc, scales.k, &mut k_i8)?;
    drop(k_acc);

    // V projection.
    let mut v_acc = vec![0i32; mu * kv_dim];
    matmul_int8(input, w_v, m, hidden, num_kv_heads * head_dim, &mut v_acc)?;
    let mut v_i8 = vec![0i8; mu * kv_dim];
    requantize_vec(&v_acc, scales.v, &mut v_i8)?;
    drop(v_acc);

    // QK norm (per (token, head)). Gemma-style: shared gamma across heads.
    {
        let mut acc = vec![0i32; hdu];
        for t in 0..mu {
            for h in 0..num_qu {
                let off = t * q_dim + h * hdu;
                rmsnorm(&q_i8[off..off + hdu], q_norm_gamma, &mut acc, qk_norm_eps_q)?;
                for (d, &a) in acc.iter().enumerate() {
                    q_i8[off + d] = rescale_and_requantize(a, qk_norm_post_scale);
                }
            }
            for h in 0..num_kvu {
                let off = t * kv_dim + h * hdu;
                rmsnorm(&k_i8[off..off + hdu], k_norm_gamma, &mut acc, qk_norm_eps_q)?;
                for (d, &a) in acc.iter().enumerate() {
                    k_i8[off + d] = rescale_and_requantize(a, qk_norm_post_scale);
                }
            }
        }
    }

    // RoPE on Q and K.
    for pos in 0..mu {
        for h in 0..num_qu {
            let off = pos * q_dim + h * hdu;
            rope_apply(&mut q_i8[off..off + hdu], pos as u32, rope_tables)?;
        }
        for h in 0..num_kvu {
            let off = pos * kv_dim + h * hdu;
            rope_apply(&mut k_i8[off..off + hdu], pos as u32, rope_tables)?;
        }
    }

    // Per-head causal attention core (no sliding window for Qwen 3.6).
    let mut attn_inter = vec![0i8; mu * q_dim];
    let mut scores_buf = vec![0i32; mu];
    let mut probs_buf = vec![0i8; mu];
    for i in 0..mu {
        for h in 0..num_qu {
            let kv_h = h * num_kvu / num_qu;
            let q_off = i * q_dim + h * hdu;
            for j in 0..=i {
                let k_off = j * kv_dim + kv_h * hdu;
                let raw = dot_int8(&q_i8[q_off..q_off + hdu], &k_i8[k_off..k_off + hdu]);
                // Same scale_score as attention.rs (banker's rounding).
                let product = (raw as i64) * (scales.score.num as i64);
                let scaled = round_half_to_even_div_pow2(product, SCALE_DENOM_LOG2)
                    .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
                scores_buf[j] = scaled;
            }
            softmax_int(&scores_buf[..i + 1], softmax_lut, &mut probs_buf[..i + 1])?;
            let ao_off = i * q_dim + h * hdu;
            for d in 0..hdu {
                let mut acc = 0i32;
                for j in 0..=i {
                    let v_off = j * kv_dim + kv_h * hdu + d;
                    acc = acc.wrapping_add((probs_buf[j] as i32) * (v_i8[v_off] as i32));
                }
                attn_inter[ao_off + d] = rescale_and_requantize(acc, scales.attn_out);
            }
        }
    }
    drop(q_i8);
    drop(k_i8);
    drop(v_i8);

    // Per-head sigmoid gate: gate = sigmoid_lut(rescale(input @ attn_gate, scales.q)).
    // Reuses scales.q for the gate-projection requantize; this is the same
    // scale convention used elsewhere for "linear-projection-then-LUT".
    let mut gate_acc = vec![0i32; mu * q_dim];
    matmul_int8(input, attn_gate, m, hidden, q_dim as u32, &mut gate_acc)?;
    let mut gate_i8 = vec![0i8; mu * q_dim];
    requantize_vec(&gate_acc, scales.q, &mut gate_i8)?;
    sigmoid_lut.apply(&mut gate_i8);
    drop(gate_acc);

    // Element-wise multiply: gated[i, j] = (attn_inter[i, j] * gate[i, j]) >> 7.
    // gate is sigmoid LUT output in [0, 127] representing [0, 1]. Multiply
    // produces i32 in range about [-128 * 127, 127 * 127]; >> 7 brings back
    // to i8 range. Use saturating clamp to keep determinism crisp.
    for k in 0..mu * q_dim {
        let prod = (attn_inter[k] as i32).wrapping_mul(gate_i8[k] as i32);
        // Symmetric half-up: |v| → (|v|+63)/127 → preserve sign. (Same
        // pattern apply_channelwise_scale uses; see Phase 2.11 numpy parity
        // note for why this differs from `>> 7`.)
        let abs_v = prod.unsigned_abs() as i64;
        let q = ((abs_v + 63) / 127) as i32;
        let signed = if prod < 0 { -q } else { q };
        attn_inter[k] = signed.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    }
    drop(gate_i8);

    // Output projection: (m, q_dim) @ attn_out_w → (m, hidden) i8.
    matmul_int8_requant(
        &attn_inter, attn_out_w, m, q_dim as u32, hidden, scales.o, output,
    )?;
    Ok(())
}

/// Phase B.1: gated-attention intermediate buffer (post-gate,
/// pre-output-projection). Returns the `(m, num_q_heads * head_dim)`
/// i8 buffer. Used by `forward_qwen_hybrid_ssm_layer`'s
/// "single shared output projection" mode: callers sum this with the
/// SSM per-head buffer (from `ssm_forward_inter`) and project once
/// via the shared `ssm.w_out` matrix, matching real Qwen 3.6 27B's
/// architecture and saving one INT8 rounding step.
#[allow(clippy::too_many_arguments)]
fn gated_attention_inter(
    input: &[i8],
    attn_qkv_fused: &[i8],
    attn_gate: &[i8],
    hidden: u32,
    num_q_heads: u32,
    num_kv_heads: u32,
    head_dim: u32,
    scales: AttentionScales,
    q_norm_gamma: &[i8],
    k_norm_gamma: &[i8],
    qk_norm_eps_q: i64,
    qk_norm_post_scale: Scale,
    rope_tables: &RopeTables,
    softmax_lut: &ExpLut,
    sigmoid_lut: &ActivationLut,
    m: u32,
) -> Result<Vec<i8>, LayerError> {
    if hidden == 0 || num_q_heads == 0 || head_dim == 0 || m == 0 {
        return Err(LayerError::ZeroHidden);
    }
    if num_kv_heads == 0 || num_kv_heads > num_q_heads || num_q_heads % num_kv_heads != 0 {
        return Err(LayerError::Attn(AttentionError::BadKvHeads));
    }
    if head_dim % 2 != 0 {
        return Err(LayerError::Attn(AttentionError::HeadDimOdd));
    }
    if rope_tables.half_head_dim != head_dim / 2 {
        return Err(LayerError::Attn(AttentionError::RopeHalfHeadDimMismatch));
    }
    if rope_tables.seq_len < m {
        return Err(LayerError::Attn(AttentionError::RopeSeqLenTooShort));
    }
    let mu = m as usize;
    let hu = hidden as usize;
    let num_qu = num_q_heads as usize;
    let num_kvu = num_kv_heads as usize;
    let hdu = head_dim as usize;
    let q_dim = num_qu * hdu;
    let kv_dim = num_kvu * hdu;
    let total_qkv = q_dim + kv_dim + kv_dim;
    if input.len() != mu * hu {
        return Err(LayerError::BadInputLen);
    }
    if attn_qkv_fused.len() != hu * total_qkv {
        return Err(LayerError::Attn(AttentionError::BadWqLen));
    }
    if attn_gate.len() != hu * q_dim {
        return Err(LayerError::Attn(AttentionError::BadWqLen));
    }
    let w_q = &attn_qkv_fused[0..hu * q_dim];
    let w_k = &attn_qkv_fused[hu * q_dim..hu * (q_dim + kv_dim)];
    let w_v = &attn_qkv_fused[hu * (q_dim + kv_dim)..hu * total_qkv];

    let mut q_acc = vec![0i32; mu * q_dim];
    matmul_int8(input, w_q, m, hidden, num_q_heads * head_dim, &mut q_acc)?;
    let mut q_i8 = vec![0i8; mu * q_dim];
    requantize_vec(&q_acc, scales.q, &mut q_i8)?;
    drop(q_acc);

    let mut k_acc = vec![0i32; mu * kv_dim];
    matmul_int8(input, w_k, m, hidden, num_kv_heads * head_dim, &mut k_acc)?;
    let mut k_i8 = vec![0i8; mu * kv_dim];
    requantize_vec(&k_acc, scales.k, &mut k_i8)?;
    drop(k_acc);

    let mut v_acc = vec![0i32; mu * kv_dim];
    matmul_int8(input, w_v, m, hidden, num_kv_heads * head_dim, &mut v_acc)?;
    let mut v_i8 = vec![0i8; mu * kv_dim];
    requantize_vec(&v_acc, scales.v, &mut v_i8)?;
    drop(v_acc);

    // QK norm.
    {
        let mut acc = vec![0i32; hdu];
        for t in 0..mu {
            for h in 0..num_qu {
                let off = t * q_dim + h * hdu;
                rmsnorm(&q_i8[off..off + hdu], q_norm_gamma, &mut acc, qk_norm_eps_q)?;
                for (d, &a) in acc.iter().enumerate() {
                    q_i8[off + d] = rescale_and_requantize(a, qk_norm_post_scale);
                }
            }
            for h in 0..num_kvu {
                let off = t * kv_dim + h * hdu;
                rmsnorm(&k_i8[off..off + hdu], k_norm_gamma, &mut acc, qk_norm_eps_q)?;
                for (d, &a) in acc.iter().enumerate() {
                    k_i8[off + d] = rescale_and_requantize(a, qk_norm_post_scale);
                }
            }
        }
    }
    // RoPE.
    for pos in 0..mu {
        for h in 0..num_qu {
            let off = pos * q_dim + h * hdu;
            rope_apply(&mut q_i8[off..off + hdu], pos as u32, rope_tables)?;
        }
        for h in 0..num_kvu {
            let off = pos * kv_dim + h * hdu;
            rope_apply(&mut k_i8[off..off + hdu], pos as u32, rope_tables)?;
        }
    }
    // Attention core.
    let mut attn_inter = vec![0i8; mu * q_dim];
    let mut scores_buf = vec![0i32; mu];
    let mut probs_buf = vec![0i8; mu];
    for i in 0..mu {
        for h in 0..num_qu {
            let kv_h = h * num_kvu / num_qu;
            let q_off = i * q_dim + h * hdu;
            for j in 0..=i {
                let k_off = j * kv_dim + kv_h * hdu;
                let raw = dot_int8(&q_i8[q_off..q_off + hdu], &k_i8[k_off..k_off + hdu]);
                let product = (raw as i64) * (scales.score.num as i64);
                let scaled = round_half_to_even_div_pow2(product, SCALE_DENOM_LOG2)
                    .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
                scores_buf[j] = scaled;
            }
            softmax_int(&scores_buf[..i + 1], softmax_lut, &mut probs_buf[..i + 1])?;
            let ao_off = i * q_dim + h * hdu;
            for d in 0..hdu {
                let mut acc = 0i32;
                for j in 0..=i {
                    let v_off = j * kv_dim + kv_h * hdu + d;
                    acc = acc.wrapping_add((probs_buf[j] as i32) * (v_i8[v_off] as i32));
                }
                attn_inter[ao_off + d] = rescale_and_requantize(acc, scales.attn_out);
            }
        }
    }
    drop(q_i8);
    drop(k_i8);
    drop(v_i8);

    // Gate.
    let mut gate_acc = vec![0i32; mu * q_dim];
    matmul_int8(input, attn_gate, m, hidden, q_dim as u32, &mut gate_acc)?;
    let mut gate_i8 = vec![0i8; mu * q_dim];
    requantize_vec(&gate_acc, scales.q, &mut gate_i8)?;
    sigmoid_lut.apply(&mut gate_i8);
    drop(gate_acc);
    for k in 0..mu * q_dim {
        let prod = (attn_inter[k] as i32).wrapping_mul(gate_i8[k] as i32);
        let abs_v = prod.unsigned_abs() as i64;
        let q = ((abs_v + 63) / 127) as i32;
        let signed = if prod < 0 { -q } else { q };
        attn_inter[k] = signed.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    }
    Ok(attn_inter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_lut::{ActivationKind, ActivationLut};
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

    fn small_scale() -> Scale {
        Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap()
    }

    fn unit_scale() -> Scale {
        Scale::from_num(1 << SCALE_DENOM_LOG2).unwrap()
    }

    fn rms_norm(hidden: usize, gamma_seed: u64) -> NormSpec {
        NormSpec::RmsNorm {
            gamma: lcg_bytes(hidden, gamma_seed),
            eps_q: DEFAULT_EPS_Q,
            post_scale: small_scale(),
        }
    }

    fn ln_norm(hidden: usize, gamma_seed: u64, beta_seed: u64) -> NormSpec {
        NormSpec::LayerNorm {
            gamma: lcg_bytes(hidden, gamma_seed),
            beta: lcg_bytes(hidden, beta_seed),
            eps_q: DEFAULT_EPS_Q,
            post_scale: small_scale(),
        }
    }

    fn build_attn_layer(hidden: u32, num_q: u32, num_kv: u32, hd: u32, seed: u64) -> LayerWeights {
        use crate::attention::AttentionScales;
        let hu = hidden as usize;
        LayerWeights::Attention {
            norm1: rms_norm(hu, seed),
            attn: AttentionWeights {
                hidden,
                num_q_heads: num_q,
                num_kv_heads: num_kv,
                head_dim: hd,
                w_q: lcg_bytes(hu * (num_q * hd) as usize, seed.wrapping_add(1)),
                w_k: lcg_bytes(hu * (num_kv * hd) as usize, seed.wrapping_add(2)),
                w_v: lcg_bytes(hu * (num_kv * hd) as usize, seed.wrapping_add(3)),
                w_o: lcg_bytes((num_q * hd) as usize * hu, seed.wrapping_add(4)),
            },
            attn_scales: AttentionScales {
                q: small_scale(),
                k: small_scale(),
                v: small_scale(),
                score: small_scale(),
                attn_out: small_scale(),
                o: small_scale(),
            },
            norm2: rms_norm(hu, seed.wrapping_add(5)),
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
                w_gate: lcg_bytes(hu * (hu * 2), seed.wrapping_add(6)),
                w_up: lcg_bytes(hu * (hu * 2), seed.wrapping_add(7)),
                w_down: lcg_bytes((hu * 2) * hu, seed.wrapping_add(8)),
            },
            ffn_scales: FfnScales {
                gate: small_scale(),
                up: small_scale(),
                mid: small_scale(),
                down: small_scale(),
            },
        }
    }

    fn build_dnet_layer(hidden: u32, num_qk: u32, num_v: u32, hd: u32, seed: u64) -> LayerWeights {
        use crate::deltanet::DeltaNetScales;
        let hu = hidden as usize;
        LayerWeights::DeltaNet {
            norm1: rms_norm(hu, seed),
            dnet: DeltaNetWeights {
                hidden,
                num_qk_heads: num_qk,
                num_v_heads: num_v,
                head_dim_qk: hd,
                head_dim_v: hd,
                w_q: lcg_bytes(hu * (num_qk * hd) as usize, seed.wrapping_add(1)),
                w_k: lcg_bytes(hu * (num_qk * hd) as usize, seed.wrapping_add(2)),
                w_v: lcg_bytes(hu * (num_v * hd) as usize, seed.wrapping_add(3)),
                w_alpha: lcg_bytes(hu * num_qk as usize, seed.wrapping_add(4)),
                w_beta: lcg_bytes(hu * num_qk as usize, seed.wrapping_add(5)),
                w_o: lcg_bytes((num_v * hd) as usize * hu, seed.wrapping_add(6)),
            },
            dnet_scales: DeltaNetScales {
                q: small_scale(),
                k: small_scale(),
                v: small_scale(),
                alpha_logit: small_scale(),
                beta_logit: small_scale(),
                u: small_scale(),
                decay: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
                update: small_scale(),
                o: small_scale(),
                proj: small_scale(),
            },
            norm2: rms_norm(hu, seed.wrapping_add(7)),
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
                w_gate: lcg_bytes(hu * (hu * 2), seed.wrapping_add(8)),
                w_up: lcg_bytes(hu * (hu * 2), seed.wrapping_add(9)),
                w_down: lcg_bytes((hu * 2) * hu, seed.wrapping_add(10)),
            },
            ffn_scales: FfnScales {
                gate: small_scale(),
                up: small_scale(),
                mid: small_scale(),
                down: small_scale(),
            },
        }
    }

    fn ctx<'a>(
        rope_tables: &'a RopeTables,
        softmax_lut: &'a ExpLut,
        sigmoid_lut: &'a ActivationLut,
        ffn_activation: &'a ActivationLut,
    ) -> LayerContext<'a> {
        LayerContext {
            rope_tables,
            softmax_lut,
            sigmoid_lut,
            ffn_activation,
        }
    }

    #[test]
    fn attention_layer_runs_and_produces_output() {
        let hidden = 8u32;
        let m = 3u32;
        let layer = build_attn_layer(hidden, 2, 1, 4, 0xa1a1);
        let rope_tables = RopeTables::identity(m, 2);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xb2b2);
        let mut output = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut output).unwrap();
    }

    #[test]
    fn deltanet_layer_runs_and_produces_output() {
        let hidden = 4u32;
        let m = 3u32;
        let layer = build_dnet_layer(hidden, 1, 2, 2, 0xc3c3);
        // DeltaNet ignores rope_tables and softmax_lut.
        let rope_tables = RopeTables::identity(m, 1);
        let softmax_lut = ExpLut::uniform_test();
        let mut sig = [0u8; 256];
        for (i, b) in sig.iter_mut().enumerate() {
            let x = (i as i32) - 128;
            *b = (64 + x / 2).clamp(0, 127) as u8;
        }
        let sigmoid_lut = ActivationLut::from_bytes(ActivationKind::SiLU, &sig).unwrap();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xd4d4);
        let mut output = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut output).unwrap();
    }

    #[test]
    fn determinism_two_calls_match() {
        let hidden = 4u32;
        let m = 2u32;
        let layer = build_attn_layer(hidden, 1, 1, 2, 0xfeed);
        let rope_tables = RopeTables::identity(m, 1);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xbeef);
        let mut a = vec![0i8; (m * hidden) as usize];
        let mut b = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut a).unwrap();
        forward_layer(&input, &layer, &c, m, &mut b).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn length_mismatch_rejected() {
        let layer = build_attn_layer(4, 1, 1, 2, 0x1234);
        let rope_tables = RopeTables::identity(2, 1);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);

        let short = vec![0i8; 7];
        let mut out = vec![0i8; 8];
        assert_eq!(
            forward_layer(&short, &layer, &c, 2, &mut out).err(),
            Some(LayerError::BadInputLen),
        );
        let input = vec![0i8; 8];
        let mut bad = vec![0i8; 5];
        assert_eq!(
            forward_layer(&input, &layer, &c, 2, &mut bad).err(),
            Some(LayerError::BadOutputLen),
        );
        assert_eq!(
            forward_layer(&input, &layer, &c, 0, &mut out).err(),
            Some(LayerError::ZeroM),
        );
    }

    #[test]
    fn norm_shape_mismatch_rejected() {
        // Build a layer with norm1.gamma of wrong length.
        let mut layer = build_attn_layer(4, 1, 1, 2, 0x4444);
        if let LayerWeights::Attention { norm1, .. } = &mut layer {
            *norm1 = NormSpec::RmsNorm {
                gamma: vec![0i8; 3], // wrong: should be hidden=4
                eps_q: DEFAULT_EPS_Q,
                post_scale: small_scale(),
            };
        }
        let rope_tables = RopeTables::identity(2, 1);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = vec![0i8; 8];
        let mut output = vec![0i8; 8];
        assert_eq!(
            forward_layer(&input, &layer, &c, 2, &mut output).err(),
            Some(LayerError::BadNormShape),
        );
    }

    #[test]
    fn layernorm_flavor_works() {
        let hidden = 4u32;
        let m = 2u32;
        let mut layer = build_attn_layer(hidden, 1, 1, 2, 0x5050);
        if let LayerWeights::Attention { norm1, .. } = &mut layer {
            *norm1 = ln_norm(hidden as usize, 0x6060, 0x7070);
        }
        let rope_tables = RopeTables::identity(m, 1);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0x8080);
        let mut output = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut output).unwrap();
    }

    #[test]
    fn residual_dominates_when_sublayers_zero() {
        // If both sublayer outputs are zero (achievable by zeroing all
        // weights), the layer reduces to: output = saturate_i8(input + 0
        // + 0) = input. Norms still apply but they get zeroed by the all-
        // zero attention weights, so the second residual just passes input.
        let hidden = 4u32;
        let m = 2u32;
        let mut layer = build_attn_layer(hidden, 1, 1, 2, 0x9090);
        if let LayerWeights::Attention { attn, ffn, .. } = &mut layer {
            attn.w_q = vec![0i8; attn.w_q.len()];
            attn.w_k = vec![0i8; attn.w_k.len()];
            attn.w_v = vec![0i8; attn.w_v.len()];
            attn.w_o = vec![0i8; attn.w_o.len()];
            ffn.w_gate = vec![0i8; ffn.w_gate.len()];
            ffn.w_up = vec![0i8; ffn.w_up.len()];
            ffn.w_down = vec![0i8; ffn.w_down.len()];
        }
        let rope_tables = RopeTables::identity(m, 1);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xaaaa);
        let mut output = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut output).unwrap();
        assert_eq!(
            output, input,
            "with zero sublayer weights, layer is identity"
        );
    }

    #[test]
    fn attention_and_deltanet_layers_produce_distinct_outputs() {
        let hidden = 4u32;
        let m = 2u32;
        let attn_layer = build_attn_layer(hidden, 1, 1, 2, 0xcccc);
        let dnet_layer = build_dnet_layer(hidden, 1, 1, 2, 0xcccc);
        let rope_tables = RopeTables::identity(m, 1);
        let softmax_lut = ExpLut::uniform_test();
        let mut sig = [0u8; 256];
        for (i, b) in sig.iter_mut().enumerate() {
            let x = (i as i32) - 128;
            *b = (64 + x / 2).clamp(0, 127) as u8;
        }
        let sigmoid_lut = ActivationLut::from_bytes(ActivationKind::SiLU, &sig).unwrap();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xdddd);
        let mut out_attn = vec![0i8; (m * hidden) as usize];
        let mut out_dnet = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &attn_layer, &c, m, &mut out_attn).unwrap();
        forward_layer(&input, &dnet_layer, &c, m, &mut out_dnet).unwrap();
        assert_ne!(
            out_attn, out_dnet,
            "different sublayer flavors must yield different outputs"
        );
    }

    #[test]
    fn add_residual_saturates() {
        let mut a = vec![100i8, -100, 50, -50];
        let b = vec![50i8, -50, -50, 50];
        add_residual_inplace(&mut a, &b);
        assert_eq!(
            a,
            vec![127, -128, 0, 0],
            "residual must saturate to i8 range"
        );
    }

    #[test]
    fn norm_per_token_validates_lengths() {
        // Wrong input length.
        let norm = rms_norm(4, 0x1234);
        let mut out = vec![0i8; 8];
        let bad = vec![0i8; 7];
        assert_eq!(
            apply_norm_per_token(&norm, &bad, 2, 4, &mut out).err(),
            Some(LayerError::BadInputLen),
        );

        // Wrong output length.
        let input = vec![0i8; 8];
        let mut bad_out = vec![0i8; 5];
        assert_eq!(
            apply_norm_per_token(&norm, &input, 2, 4, &mut bad_out).err(),
            Some(LayerError::BadOutputLen),
        );
    }

    #[test]
    fn unit_post_scale_norm_round_trip() {
        // With post_scale = 1.0 and a constant input, normed = beta_or_zero.
        let hidden = 8u32;
        let m = 1u32;
        let norm = NormSpec::RmsNorm {
            gamma: vec![32i8; hidden as usize],
            eps_q: DEFAULT_EPS_Q,
            post_scale: unit_scale(),
        };
        let input = vec![5i8; (m * hidden) as usize];
        let mut output = vec![0i8; (m * hidden) as usize];
        apply_norm_per_token(&norm, &input, m, hidden, &mut output).unwrap();
        // RMSNorm of constant input → output = (constant / rms) * gamma. With
        // gamma=32 and a uniform constant, the per-channel result is the same
        // value across channels. We only assert all entries are equal.
        let first = output[0];
        for &v in &output {
            assert_eq!(
                v, first,
                "RMSNorm of uniform input must produce uniform output"
            );
        }
    }

    fn build_gemma_layer(hidden: u32, num_q: u32, num_kv: u32, hd: u32, seed: u64) -> LayerWeights {
        use crate::attention::AttentionScales;
        let hu = hidden as usize;
        let hdu = hd as usize;
        LayerWeights::Gemma {
            norm1: rms_norm(hu, seed),
            attn: AttentionWeights {
                hidden,
                num_q_heads: num_q,
                num_kv_heads: num_kv,
                head_dim: hd,
                w_q: lcg_bytes(hu * (num_q * hd) as usize, seed.wrapping_add(1)),
                w_k: lcg_bytes(hu * (num_kv * hd) as usize, seed.wrapping_add(2)),
                w_v: lcg_bytes(hu * (num_kv * hd) as usize, seed.wrapping_add(3)),
                w_o: lcg_bytes((num_q * hd) as usize * hu, seed.wrapping_add(4)),
            },
            attn_scales: AttentionScales {
                q: small_scale(),
                k: small_scale(),
                v: small_scale(),
                score: small_scale(),
                attn_out: small_scale(),
                o: small_scale(),
            },
            q_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(5)),
            k_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(6)),
            qk_norm_eps_q: DEFAULT_EPS_Q,
            qk_norm_post_scale: small_scale(),
            post_attn_norm: rms_norm(hu, seed.wrapping_add(7)),
            norm2: rms_norm(hu, seed.wrapping_add(8)),
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
                w_gate: lcg_bytes(hu * (hu * 2), seed.wrapping_add(9)),
                w_up: lcg_bytes(hu * (hu * 2), seed.wrapping_add(10)),
                w_down: lcg_bytes((hu * 2) * hu, seed.wrapping_add(11)),
            },
            ffn_scales: FfnScales {
                gate: small_scale(),
                up: small_scale(),
                mid: small_scale(),
                down: small_scale(),
            },
            post_ffn_norm: rms_norm(hu, seed.wrapping_add(12)),
            sliding_window: None,
            inp_gate: Some(lcg_bytes(hu, seed.wrapping_add(13))),
            layer_output_scale: Some(lcg_bytes(hu, seed.wrapping_add(14))),
        }
    }

    #[test]
    fn gemma_layer_full_runs() {
        let hidden = 8u32;
        let m = 3u32;
        let layer = build_gemma_layer(hidden, 2, 1, 4, 0x9090);
        let rope_tables = RopeTables::identity(m, 2);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xa1a1);
        let mut output = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut output).unwrap();
    }

    #[test]
    fn gemma_layer_sliding_window_changes_output() {
        let hidden = 8u32;
        let m = 4u32;
        let mut base = build_gemma_layer(hidden, 2, 1, 4, 0x1111);
        let mut sw = build_gemma_layer(hidden, 2, 1, 4, 0x1111);
        if let LayerWeights::Gemma { sliding_window, .. } = &mut sw {
            *sliding_window = Some(2);
        }
        let rope_tables = RopeTables::identity(m, 2);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0x2222);
        let mut out_base = vec![0i8; (m * hidden) as usize];
        let mut out_sw = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &base, &c, m, &mut out_base).unwrap();
        forward_layer(&input, &sw, &c, m, &mut out_sw).unwrap();
        // First token (i=0): both see only itself, so the sliding window
        // doesn't change anything.
        assert_eq!(&out_base[..hidden as usize], &out_sw[..hidden as usize]);
        // For i=3 (m-1=3), base sees [0..=3]; sw sees [2..=3]. Outputs
        // for position 3 must differ for a non-degenerate model.
        let row3_base = &out_base[(3 * hidden as usize)..(4 * hidden as usize)];
        let row3_sw = &out_sw[(3 * hidden as usize)..(4 * hidden as usize)];
        assert_ne!(
            row3_base, row3_sw,
            "sliding window should change late tokens"
        );
        let _ = (&mut base, &mut sw); // silence "unused mut" if any
    }

    fn build_qwen_standard_layer(
        hidden: u32,
        num_q: u32,
        num_kv: u32,
        hd: u32,
        seed: u64,
    ) -> LayerWeights {
        use crate::attention::AttentionScales;
        let hu = hidden as usize;
        let hdu = hd as usize;
        LayerWeights::QwenStandard {
            norm1: rms_norm(hu, seed),
            attn: AttentionWeights {
                hidden,
                num_q_heads: num_q,
                num_kv_heads: num_kv,
                head_dim: hd,
                w_q: lcg_bytes(hu * (num_q * hd) as usize, seed.wrapping_add(1)),
                w_k: lcg_bytes(hu * (num_kv * hd) as usize, seed.wrapping_add(2)),
                w_v: lcg_bytes(hu * (num_kv * hd) as usize, seed.wrapping_add(3)),
                w_o: lcg_bytes((num_q * hd) as usize * hu, seed.wrapping_add(4)),
            },
            attn_scales: AttentionScales {
                q: small_scale(),
                k: small_scale(),
                v: small_scale(),
                score: small_scale(),
                attn_out: small_scale(),
                o: small_scale(),
            },
            q_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(5)),
            k_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(6)),
            qk_norm_eps_q: DEFAULT_EPS_Q,
            qk_norm_post_scale: small_scale(),
            norm2: rms_norm(hu, seed.wrapping_add(7)),
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
                w_gate: lcg_bytes(hu * (hu * 2), seed.wrapping_add(8)),
                w_up: lcg_bytes(hu * (hu * 2), seed.wrapping_add(9)),
                w_down: lcg_bytes((hu * 2) * hu, seed.wrapping_add(10)),
            },
            ffn_scales: FfnScales {
                gate: small_scale(),
                up: small_scale(),
                mid: small_scale(),
                down: small_scale(),
            },
        }
    }

    #[test]
    fn qwen_standard_layer_runs_and_round_trips() {
        use crate::comm_w::compute_comm_w;
        use crate::model::{Model, ModelDims};
        let hidden = 8u32;
        let m = 3u32;
        let layer = build_qwen_standard_layer(hidden, 2, 1, 4, 0xc0c0);
        let rope_tables = RopeTables::identity(m, 2);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xd1d1);
        let mut output = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut output).unwrap();

        // Disk round-trip.
        let model = Model {
            dims: ModelDims {
                vocab: 16,
                hidden,
                seq_len: 4,
                activation_tile: 2,
            },
            arch_tag: crate::model::arch_tag("qwen35"),
            feature_flags: 0,
            embed: lcg_bytes((16 * hidden) as usize, 0xd2d2),
            layers: vec![layer],
            final_norm: None,
            rope_tables: RopeTables::identity(4, 2),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        };
        let dir = std::env::temp_dir().join(format!(
            "ai_pow_vi_qwen_std_rt_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        model.save(&dir).unwrap();
        let comm = compute_comm_w(&model);
        let loaded = Model::load(&dir, &comm).unwrap();
        assert_eq!(model.layers, loaded.layers);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn build_qwen_hybrid_layer(
        hidden: u32,
        num_q: u32,
        num_kv: u32,
        num_v: u32,
        head_dim: u32,
        seed: u64,
    ) -> LayerWeights {
        use crate::attention::AttentionScales;
        use crate::deltanet::DeltaNetScales;
        let hu = hidden as usize;
        let hdu = head_dim as usize;
        let kernel_size = 4u32;
        let qkv_dim = ((num_q + num_kv + num_kv) * head_dim) as usize;
        let nv = num_v as usize;
        LayerWeights::QwenHybridSsm {
            norm1: rms_norm(hu, seed),
            attn_qkv_fused: lcg_bytes(hu * qkv_dim, seed.wrapping_add(1)),
            attn_gate: lcg_bytes(hu * (num_q * head_dim) as usize, seed.wrapping_add(2)),
            attn_out: lcg_bytes((num_q * head_dim) as usize * hu, seed.wrapping_add(3)),
            num_q_heads: num_q,
            num_kv_heads: num_kv,
            head_dim,
            attn_scales: AttentionScales {
                q: small_scale(),
                k: small_scale(),
                v: small_scale(),
                score: small_scale(),
                attn_out: small_scale(),
                o: small_scale(),
            },
            q_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(4)),
            k_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(5)),
            qk_norm_eps_q: DEFAULT_EPS_Q,
            qk_norm_post_scale: small_scale(),
            ssm_a: lcg_bytes(nv, seed.wrapping_add(6)),
            ssm_alpha: lcg_bytes(hu * nv, seed.wrapping_add(7)),
            ssm_beta: lcg_bytes(hu * nv, seed.wrapping_add(8)),
            ssm_conv1d: lcg_bytes((kernel_size as usize) * hu, seed.wrapping_add(9)),
            ssm_dt: lcg_bytes(nv, seed.wrapping_add(10)),
            ssm_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(11)),
            ssm_norm_eps_q: DEFAULT_EPS_Q,
            ssm_norm_post_scale: small_scale(),
            ssm_out: lcg_bytes((num_v * head_dim) as usize * hu, seed.wrapping_add(12)),
            num_v_heads: num_v,
            ssm_head_dim: head_dim,
            ssm_kernel_size: kernel_size,
            ssm_scales: DeltaNetScales {
                q: small_scale(),
                k: small_scale(),
                v: small_scale(),
                alpha_logit: small_scale(),
                beta_logit: small_scale(),
                u: small_scale(),
                decay: small_scale(),
                update: small_scale(),
                o: small_scale(),
                proj: small_scale(),
            },
            norm2: rms_norm(hu, seed.wrapping_add(13)),
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
                w_gate: lcg_bytes(hu * (hu * 2), seed.wrapping_add(14)),
                w_up: lcg_bytes(hu * (hu * 2), seed.wrapping_add(15)),
                w_down: lcg_bytes((hu * 2) * hu, seed.wrapping_add(16)),
            },
            ffn_scales: FfnScales {
                gate: small_scale(),
                up: small_scale(),
                mid: small_scale(),
                down: small_scale(),
            },
        }
    }

    #[test]
    fn qwen_hybrid_ssm_loads_and_forwards() {
        use crate::comm_w::compute_comm_w;
        use crate::model::{Model, ModelDims};
        let hidden = 8u32;
        let layer = build_qwen_hybrid_layer(hidden, 2, 1, 3, 4, 0xefef);
        let model = Model {
            dims: ModelDims {
                vocab: 16,
                hidden,
                seq_len: 4,
                activation_tile: 2,
            },
            arch_tag: crate::model::arch_tag("qwen35"),
            feature_flags: 0,
            embed: lcg_bytes((16 * hidden) as usize, 0xf0f0),
            layers: vec![layer],
            final_norm: None,
            rope_tables: RopeTables::identity(4, 2),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        };
        let dir = std::env::temp_dir().join(format!(
            "ai_pow_vi_qwen_hybrid_rt_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        model.save(&dir).unwrap();
        let comm = compute_comm_w(&model);
        let loaded = Model::load(&dir, &comm).unwrap();
        assert_eq!(model.layers, loaded.layers);

        // Forward now runs end-to-end (Phase 2.13 part 2).
        let rope_tables = RopeTables::identity(2, 2);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((2 * hidden) as usize, 0xc0c0);
        let mut a = vec![0i8; (2 * hidden) as usize];
        let mut b = vec![0i8; (2 * hidden) as usize];
        forward_layer(&input, &loaded.layers[0], &c, 2, &mut a).unwrap();
        forward_layer(&input, &loaded.layers[0], &c, 2, &mut b).unwrap();
        assert_eq!(a, b, "two identical hybrid forward calls must agree");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn qwen_hybrid_ssm_zero_weights_yields_input_passthrough() {
        // With every sublayer weight zero, both attention and SSM produce
        // zero outputs; FFN also produces zero; layer reduces to identity
        // (input passthrough modulo norm scaling, which on zero post-add
        // preserves input bytes).
        let hidden = 8u32;
        let m = 2u32;
        let mut layer = build_qwen_hybrid_layer(hidden, 2, 1, 3, 4, 0xeeee);
        if let LayerWeights::QwenHybridSsm {
            attn_qkv_fused,
            attn_gate,
            attn_out,
            ssm_alpha,
            ssm_beta,
            ssm_conv1d,
            ssm_out,
            ffn,
            ..
        } = &mut layer
        {
            attn_qkv_fused.iter_mut().for_each(|x| *x = 0);
            attn_gate.iter_mut().for_each(|x| *x = 0);
            attn_out.iter_mut().for_each(|x| *x = 0);
            ssm_alpha.iter_mut().for_each(|x| *x = 0);
            ssm_beta.iter_mut().for_each(|x| *x = 0);
            ssm_conv1d.iter_mut().for_each(|x| *x = 0);
            ssm_out.iter_mut().for_each(|x| *x = 0);
            ffn.w_gate.iter_mut().for_each(|x| *x = 0);
            ffn.w_up.iter_mut().for_each(|x| *x = 0);
            ffn.w_down.iter_mut().for_each(|x| *x = 0);
        }
        let rope_tables = RopeTables::identity(m, 2);
        let softmax_lut = ExpLut::uniform_test();
        let sigmoid_lut = ActivationLut::identity();
        let ffn_act = ActivationLut::identity();
        let c = ctx(&rope_tables, &softmax_lut, &sigmoid_lut, &ffn_act);
        let input = lcg_bytes((m * hidden) as usize, 0xdada);
        let mut output = vec![0i8; (m * hidden) as usize];
        forward_layer(&input, &layer, &c, m, &mut output).unwrap();
        assert_eq!(
            output, input,
            "with zero sublayer weights, hybrid layer is identity"
        );
    }

    #[test]
    fn gemma_layer_round_trip_through_disk_format() {
        use crate::comm_w::compute_comm_w;
        use crate::model::{Model, ModelDims};
        let hidden = 8u32;
        let layer = build_gemma_layer(hidden, 2, 1, 4, 0xfeed);
        let model = Model {
            dims: ModelDims {
                vocab: 16,
                hidden,
                seq_len: 4,
                activation_tile: 2,
            },
            arch_tag: crate::model::arch_tag("gemma4"),
            feature_flags: 0,
            embed: lcg_bytes((16 * hidden) as usize, 0xbeef),
            layers: vec![layer],
            final_norm: None,
            rope_tables: RopeTables::identity(4, 2),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        };
        let dir = std::env::temp_dir().join(format!(
            "ai_pow_vi_gemma_rt_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        model.save(&dir).unwrap();
        let comm = compute_comm_w(&model);
        let loaded = Model::load(&dir, &comm).unwrap();
        assert_eq!(model.layers, loaded.layers);
        assert_eq!(model.arch_tag, loaded.arch_tag);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
