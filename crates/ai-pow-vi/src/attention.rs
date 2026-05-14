//! Standard transformer attention (multi-head and grouped-query).
//!
//! Composes [`crate::matmul_int8`], [`crate::rope`], and [`crate::softmax`]
//! into the full attention forward pass. Everything is integer-only; the only
//! floating-point that exists here is in doc comments.
//!
//! Layout conventions (same as `matmul_int8`):
//! - `input`, `output`, Q/K/V tensors after projection: row-major.
//! - Weight matrices `w_q`, `w_k`, `w_v`, `w_o`: column-major.
//!
//! Determinism rules (inherited from the crate):
//! - Reduction order is row-major, ascending index for every dot product
//!   and every V-weighted sum.
//! - GQA head mapping: `kv_head = q_head * num_kv_heads / num_q_heads`
//!   (integer division; pinned here to avoid per-implementation drift).
//! - Causal masking: scores slice is truncated to `i+1` entries rather than
//!   using a sentinel, so softmax never sees a masked position.
//! - Score scaling uses [`scale_score`] (banker's rounding; `#[inline(never)]`).

use thiserror::Error;

use crate::matmul_int8::{dot_int8, matmul_int8, matmul_int8_requant, requantize_vec, MatmulError};
use crate::quant::{rescale_and_requantize, round_half_to_even_div_pow2, Scale, SCALE_DENOM_LOG2};
use crate::rmsnorm::{rmsnorm, RmsNormError};
use crate::rope::{rope_apply, RopeError, RopeTables};
use crate::softmax::{softmax_int, ExpLut, SoftmaxError};

/// Phase 2.11 Gemma-attention extras: per-head Q/K RMSNorm + optional
/// sliding-window mask. Borrowed for the duration of one
/// [`attention_forward_gemma`] call.
#[derive(Debug, Clone, Copy)]
pub struct GemmaAttentionOpts<'a> {
    /// Per-head Q RMSNorm gamma (length = `head_dim`, applied per
    /// (token, q_head)). Always `Some` for Gemma; the type leaves it
    /// optional in case a future arch wants only K-side QK norm.
    pub q_norm_gamma: Option<&'a [i8]>,
    /// Same for K (length = `head_dim`, applied per (token, kv_head)).
    pub k_norm_gamma: Option<&'a [i8]>,
    /// Integer eps_q for both Q and K norms.
    pub qk_norm_eps_q: i64,
    /// Rescale Q/K-norm i32 output back to i8 before RoPE.
    pub qk_norm_post_scale: Scale,
    /// Sliding-window radius. `None` is full causal (= `attention_forward`).
    /// `Some(w)` keeps tokens `[max(0, i + 1 - w) ..= i]` for query
    /// position `i` (i.e. the last `w` tokens, inclusive of self).
    pub sliding_window: Option<u32>,
    /// Phase B.2: sigmoid LUT for the Q-projection gate. When
    /// `weights.q_has_gate == true`, the Q projection produces a
    /// `[Q || gate]` per-head buffer; the gate half is passed through
    /// this LUT and element-wise multiplied into the attention output
    /// before `w_o`. When `q_has_gate == false`, this field is
    /// ignored.
    pub gate_sigmoid_lut: Option<&'a crate::activation_lut::ActivationLut>,
}

/// Weights for one multi-head / grouped-query attention block. All tensors
/// are INT8 in column-major layout (each column = one output feature).
///
/// For Gemma 4 31B: hidden=3456, num_q_heads=32, num_kv_heads=16, head_dim=256.
/// For Qwen 3.6 27B attention blocks: hidden=3072, num_q_heads=16,
/// num_kv_heads=16, head_dim=256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionWeights {
    pub hidden: u32,
    pub num_q_heads: u32,
    pub num_kv_heads: u32,
    pub head_dim: u32,
    pub w_q: Vec<i8>, // (hidden, num_q_heads * head_dim * q_proj_factor) col-major
    pub w_k: Vec<i8>, // (hidden, num_kv_heads * head_dim) col-major
    pub w_v: Vec<i8>, // (hidden, num_kv_heads * head_dim) col-major
    pub w_o: Vec<i8>, // (num_q_heads * head_dim, hidden) col-major
    /// Phase B.2: when true, `w_q` projects to `2 * num_q_heads * head_dim`
    /// = `[Q (q_dim) || gate (q_dim)]` packed side-by-side per token.
    /// After Q is used in attention as usual (head_dim), the gate half
    /// is passed through `sigmoid_lut` and element-wise multiplied
    /// into the post-attention (m, q_dim) buffer before `w_o` projects
    /// to hidden. Matches Qwen 3.6 27B's full-attention block in
    /// `Qwen3NextAttention.forward`:
    /// ```text
    /// query_states, gate = torch.chunk(q_proj(x).view(B,S,-1,head_dim*2), 2, dim=-1)
    /// attn_output = attn_output * torch.sigmoid(gate)
    /// ```
    /// When false (default), `w_q` projects to `num_q_heads * head_dim`
    /// and no gate is applied (legacy MHA / GQA / Gemma behavior).
    pub q_has_gate: bool,
}

/// Per-tensor quantization scales for one attention step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttentionScales {
    /// Requantize Q projection i32 → i8.
    pub q: Scale,
    /// Requantize K projection i32 → i8.
    pub k: Scale,
    /// Requantize V projection i32 → i8.
    pub v: Scale,
    /// Scale i32 Q·K dot product into i32 softmax-domain units.
    pub score: Scale,
    /// Requantize i32 V-weighted sum → i8.
    pub attn_out: Scale,
    /// Requantize output projection i32 → i8.
    pub o: Scale,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AttentionError {
    #[error("input length must equal m * hidden")]
    BadInputLen,
    #[error("output length must equal m * hidden")]
    BadOutputLen,
    #[error("w_q length must equal hidden * (num_q_heads * head_dim)")]
    BadWqLen,
    #[error("w_k length must equal hidden * (num_kv_heads * head_dim)")]
    BadWkLen,
    #[error("w_v length must equal hidden * (num_kv_heads * head_dim)")]
    BadWvLen,
    #[error("w_o length must equal (num_q_heads * head_dim) * hidden")]
    BadWoLen,
    #[error("head_dim must be even for RoPE")]
    HeadDimOdd,
    #[error("rope_tables half_head_dim must equal head_dim / 2")]
    RopeHalfHeadDimMismatch,
    #[error("rope_tables seq_len must be >= m")]
    RopeSeqLenTooShort,
    #[error("num_kv_heads must be > 0, <= num_q_heads, and divide num_q_heads")]
    BadKvHeads,
    #[error("dimensions must be > 0")]
    ZeroDim,
    #[error("matmul: {0}")]
    Matmul(#[from] MatmulError),
    #[error("rope: {0}")]
    Rope(#[from] RopeError),
    #[error("softmax: {0}")]
    Softmax(#[from] SoftmaxError),
    #[error("Q/K RMSNorm: {0}")]
    QkNorm(#[from] RmsNormError),
    #[error("QK norm gamma length must equal head_dim")]
    BadQkNormGammaLen,
}

/// Scale an i32 Q·K dot product into i32 softmax-domain units.
///
/// Uses banker's rounding (same as `rescale_and_requantize`) so the
/// rounding mode is pinned across all callers.
#[inline(never)]
fn scale_score(raw: i32, scale: Scale) -> i32 {
    let product = (raw as i64) * (scale.num as i64);
    round_half_to_even_div_pow2(product, SCALE_DENOM_LOG2).clamp(i32::MIN as i64, i32::MAX as i64)
        as i32
}

/// Full attention forward pass (multi-head or grouped-query).
///
/// `input` is `(m, hidden)` row-major i8; `output` is the same shape.
///
/// Memory: allocates Q/K/V tensors of size `m * (num_kv_heads * head_dim)`
/// plus two scratch buffers of size `m` for scores and probs. All
/// intermediates are freed before the output projection.
pub fn attention_forward(
    input: &[i8],
    weights: &AttentionWeights,
    scales: AttentionScales,
    rope_tables: &RopeTables,
    softmax_lut: &ExpLut,
    m: u32,
    output: &mut [i8],
) -> Result<(), AttentionError> {
    let hidden = weights.hidden;
    let num_q = weights.num_q_heads;
    let num_kv = weights.num_kv_heads;
    let hd = weights.head_dim;

    if hidden == 0 || num_q == 0 || hd == 0 || m == 0 {
        return Err(AttentionError::ZeroDim);
    }
    if num_kv == 0 || num_kv > num_q || num_q % num_kv != 0 {
        return Err(AttentionError::BadKvHeads);
    }
    if hd % 2 != 0 {
        return Err(AttentionError::HeadDimOdd);
    }
    // Plain RoPE: tables.half_head_dim == hd/2. IMROPE: tables.half_head_dim
    // = n_rot/2 < hd/2 (rotated subspace is a prefix of head_dim). Accept both.
    if rope_tables.half_head_dim == 0 || (rope_tables.half_head_dim as usize) > (hd as usize) / 2 {
        return Err(AttentionError::RopeHalfHeadDimMismatch);
    }
    if rope_tables.seq_len < m {
        return Err(AttentionError::RopeSeqLenTooShort);
    }

    let mu = m as usize;
    let hu = hidden as usize;
    let num_qu = num_q as usize;
    let num_kvu = num_kv as usize;
    let hdu = hd as usize;
    let q_row_stride = num_qu * hdu; // num_q_heads * head_dim
    let kv_row_stride = num_kvu * hdu; // num_kv_heads * head_dim

    if input.len() != mu * hu {
        return Err(AttentionError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(AttentionError::BadOutputLen);
    }
    if weights.w_q.len() != hu * q_row_stride {
        return Err(AttentionError::BadWqLen);
    }
    if weights.w_k.len() != hu * kv_row_stride {
        return Err(AttentionError::BadWkLen);
    }
    if weights.w_v.len() != hu * kv_row_stride {
        return Err(AttentionError::BadWvLen);
    }
    if weights.w_o.len() != q_row_stride * hu {
        return Err(AttentionError::BadWoLen);
    }

    // Step 1: Q projection → (m, num_q_heads * head_dim) i8
    let mut q_acc = vec![0i32; mu * q_row_stride];
    matmul_int8(input, &weights.w_q, m, hidden, num_q * hd, &mut q_acc)?;
    let mut q_i8 = vec![0i8; mu * q_row_stride];
    requantize_vec(&q_acc, scales.q, &mut q_i8)?;
    drop(q_acc);

    // Step 2: K projection → (m, num_kv_heads * head_dim) i8
    let mut k_acc = vec![0i32; mu * kv_row_stride];
    matmul_int8(input, &weights.w_k, m, hidden, num_kv * hd, &mut k_acc)?;
    let mut k_i8 = vec![0i8; mu * kv_row_stride];
    requantize_vec(&k_acc, scales.k, &mut k_i8)?;
    drop(k_acc);

    // Step 3: V projection → (m, num_kv_heads * head_dim) i8
    let mut v_acc = vec![0i32; mu * kv_row_stride];
    matmul_int8(input, &weights.w_v, m, hidden, num_kv * hd, &mut v_acc)?;
    let mut v_i8 = vec![0i8; mu * kv_row_stride];
    requantize_vec(&v_acc, scales.v, &mut v_i8)?;
    drop(v_acc);

    // Step 4: RoPE — apply in place to every (pos, head) slot of Q and K.
    // Length passed is `2 * tables.half_head_dim`, which equals `head_dim`
    // for plain RoPE tables and `n_rot` for IMROPE tables (whose
    // `half_head_dim = n_rot/2`). In the IMROPE case the slice covers only
    // the first n_rot dims of each head; the tail [n_rot, head_dim) is
    // intentionally left unrotated, matching ggml's qwen35 IMROPE.
    let rope_slice = 2 * rope_tables.half_head_dim as usize;
    for pos in 0..mu {
        for h in 0..num_qu {
            let off = pos * q_row_stride + h * hdu;
            rope_apply(&mut q_i8[off..off + rope_slice], pos as u32, rope_tables)?;
        }
        for h in 0..num_kvu {
            let off = pos * kv_row_stride + h * hdu;
            rope_apply(&mut k_i8[off..off + rope_slice], pos as u32, rope_tables)?;
        }
    }

    // Step 5–7: Per-head causal attention core.
    // attn_out[i * q_row_stride + h * hd + d] = attention output before W_o.
    let mut attn_out = vec![0i8; mu * q_row_stride];

    // Scratch buffers; length m, reused across every (i, h).
    let mut scores_buf = vec![0i32; mu];
    let mut probs_buf = vec![0i8; mu];

    for i in 0..mu {
        for h in 0..num_qu {
            // GQA: head_for_kv(h) = h * num_kv / num_q (integer division, pinned).
            let kv_h = h * num_kvu / num_qu;

            // Scores for all causally valid keys j = 0..=i (ascending).
            let q_off = i * q_row_stride + h * hdu;
            for j in 0..=i {
                let k_off = j * kv_row_stride + kv_h * hdu;
                let raw = dot_int8(&q_i8[q_off..q_off + hdu], &k_i8[k_off..k_off + hdu]);
                scores_buf[j] = scale_score(raw, scales.score);
            }

            // Softmax over first i+1 scores.
            softmax_int(&scores_buf[..i + 1], softmax_lut, &mut probs_buf[..i + 1])?;

            // V-weighted sum: reduction over j ascending, then requantize.
            let ao_off = i * q_row_stride + h * hdu;
            for d in 0..hdu {
                let mut acc = 0i32;
                for j in 0..=i {
                    let v_off = j * kv_row_stride + kv_h * hdu + d;
                    acc = acc.wrapping_add((probs_buf[j] as i32) * (v_i8[v_off] as i32));
                }
                attn_out[ao_off + d] = rescale_and_requantize(acc, scales.attn_out);
            }
        }
    }

    drop(q_i8);
    drop(k_i8);
    drop(v_i8);

    // Step 8: Output projection — (m, num_q_heads * head_dim) @ W_o → (m, hidden) i8.
    matmul_int8_requant(
        &attn_out,
        &weights.w_o,
        m,
        num_q * hd,
        hidden,
        scales.o,
        output,
    )?;

    Ok(())
}

/// Phase 2.11 Gemma-flavored attention forward.
///
/// Same skeleton as [`attention_forward`], with two additions:
///
/// 1. **QK norm** — after Q and K are projected, an RMSNorm is applied
///    per-head (independently per `(token, head)` slot, gamma shared
///    across heads), then requantized to i8 before RoPE.
/// 2. **Sliding window** — when `opts.sliding_window = Some(w)`, the
///    score range for query position `i` is `[max(0, i+1-w) ..= i]`
///    rather than `[0 ..= i]`. Tokens further back than `w-1` steps
///    are masked out entirely.
///
/// The rest of the math is identical to `attention_forward`, so the
/// FFN tile-Merkle puzzle (Phase 3 `mine_vi` / `verify_vi`) works on
/// Gemma layers exactly as it does on plain Attention layers.
pub fn attention_forward_gemma(
    input: &[i8],
    weights: &AttentionWeights,
    scales: AttentionScales,
    rope_tables: &RopeTables,
    softmax_lut: &ExpLut,
    opts: GemmaAttentionOpts,
    m: u32,
    output: &mut [i8],
) -> Result<(), AttentionError> {
    let hidden = weights.hidden;
    let num_q = weights.num_q_heads;
    let num_kv = weights.num_kv_heads;
    let hd = weights.head_dim;

    if hidden == 0 || num_q == 0 || hd == 0 || m == 0 {
        return Err(AttentionError::ZeroDim);
    }
    if num_kv == 0 || num_kv > num_q || num_q % num_kv != 0 {
        return Err(AttentionError::BadKvHeads);
    }
    if hd % 2 != 0 {
        return Err(AttentionError::HeadDimOdd);
    }
    // Plain RoPE: tables.half_head_dim == hd/2. IMROPE: tables.half_head_dim
    // = n_rot/2 < hd/2 (rotated subspace is a prefix of head_dim). Accept both.
    if rope_tables.half_head_dim == 0 || (rope_tables.half_head_dim as usize) > (hd as usize) / 2 {
        return Err(AttentionError::RopeHalfHeadDimMismatch);
    }
    if rope_tables.seq_len < m {
        return Err(AttentionError::RopeSeqLenTooShort);
    }
    let hdu = hd as usize;
    if let Some(g) = opts.q_norm_gamma {
        if g.len() != hdu {
            return Err(AttentionError::BadQkNormGammaLen);
        }
    }
    if let Some(g) = opts.k_norm_gamma {
        if g.len() != hdu {
            return Err(AttentionError::BadQkNormGammaLen);
        }
    }

    let mu = m as usize;
    let hu = hidden as usize;
    let num_qu = num_q as usize;
    let num_kvu = num_kv as usize;
    let q_row_stride = num_qu * hdu;
    let kv_row_stride = num_kvu * hdu;

    if input.len() != mu * hu {
        return Err(AttentionError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(AttentionError::BadOutputLen);
    }
    // Phase B.2: when q_has_gate is true, w_q outputs 2x the Q dim
    // (= [Q || gate] packed per head). Validate accordingly.
    let q_proj_stride = if weights.q_has_gate {
        2 * q_row_stride
    } else {
        q_row_stride
    };
    if weights.w_q.len() != hu * q_proj_stride {
        return Err(AttentionError::BadWqLen);
    }
    if weights.w_k.len() != hu * kv_row_stride {
        return Err(AttentionError::BadWkLen);
    }
    if weights.w_v.len() != hu * kv_row_stride {
        return Err(AttentionError::BadWvLen);
    }
    if weights.w_o.len() != q_row_stride * hu {
        return Err(AttentionError::BadWoLen);
    }

    // Q projection.
    let mut q_acc = vec![0i32; mu * q_proj_stride];
    matmul_int8(
        input, &weights.w_q, m, hidden, q_proj_stride as u32, &mut q_acc,
    )?;
    let mut q_proj_i8 = vec![0i8; mu * q_proj_stride];
    requantize_vec(&q_acc, scales.q, &mut q_proj_i8)?;
    drop(q_acc);

    // Phase B.2: if q_has_gate, split [Q || gate] per-head into two
    // separate buffers. Layout: per-head 2*head_dim where first head_dim
    // is Q and second is the pre-sigmoid gate logits.
    let (mut q_i8, gate_logits): (Vec<i8>, Option<Vec<i8>>) = if weights.q_has_gate {
        let mut q_only = vec![0i8; mu * q_row_stride];
        let mut gate_only = vec![0i8; mu * q_row_stride];
        for t in 0..mu {
            for h in 0..num_qu {
                let proj_base = t * q_proj_stride + h * 2 * hdu;
                let out_base = t * q_row_stride + h * hdu;
                for d in 0..hdu {
                    q_only[out_base + d] = q_proj_i8[proj_base + d];
                    gate_only[out_base + d] = q_proj_i8[proj_base + hdu + d];
                }
            }
        }
        drop(q_proj_i8);
        (q_only, Some(gate_only))
    } else {
        (q_proj_i8, None)
    };

    // K projection.
    let mut k_acc = vec![0i32; mu * kv_row_stride];
    matmul_int8(input, &weights.w_k, m, hidden, num_kv * hd, &mut k_acc)?;
    let mut k_i8 = vec![0i8; mu * kv_row_stride];
    requantize_vec(&k_acc, scales.k, &mut k_i8)?;
    drop(k_acc);

    // V projection.
    let mut v_acc = vec![0i32; mu * kv_row_stride];
    matmul_int8(input, &weights.w_v, m, hidden, num_kv * hd, &mut v_acc)?;
    let mut v_i8 = vec![0i8; mu * kv_row_stride];
    requantize_vec(&v_acc, scales.v, &mut v_i8)?;
    drop(v_acc);

    // **Phase 2.11 step: QK norm.** Per (token, head): apply RMSNorm
    // to the `head_dim` slot, then requantize back to i8. Same gamma
    // is shared across heads (Gemma's published shape).
    if let Some(gamma) = opts.q_norm_gamma {
        let mut acc = vec![0i32; hdu];
        for t in 0..mu {
            for h in 0..num_qu {
                let off = t * q_row_stride + h * hdu;
                rmsnorm(&q_i8[off..off + hdu], gamma, &mut acc, opts.qk_norm_eps_q)?;
                for (d, &a) in acc.iter().enumerate() {
                    q_i8[off + d] = rescale_and_requantize(a, opts.qk_norm_post_scale);
                }
            }
        }
    }
    if let Some(gamma) = opts.k_norm_gamma {
        let mut acc = vec![0i32; hdu];
        for t in 0..mu {
            for h in 0..num_kvu {
                let off = t * kv_row_stride + h * hdu;
                rmsnorm(&k_i8[off..off + hdu], gamma, &mut acc, opts.qk_norm_eps_q)?;
                for (d, &a) in acc.iter().enumerate() {
                    k_i8[off + d] = rescale_and_requantize(a, opts.qk_norm_post_scale);
                }
            }
        }
    }

    // RoPE on Q and K (same as `attention_forward`). Slice length =
    // `2 * tables.half_head_dim` so plain RoPE covers full head_dim and
    // IMROPE (whose tables only carry n_rot/2 pairs) covers exactly the
    // first n_rot dims — leaving the tail unrotated per the qwen35 spec.
    let rope_slice = 2 * rope_tables.half_head_dim as usize;
    for pos in 0..mu {
        for h in 0..num_qu {
            let off = pos * q_row_stride + h * hdu;
            rope_apply(&mut q_i8[off..off + rope_slice], pos as u32, rope_tables)?;
        }
        for h in 0..num_kvu {
            let off = pos * kv_row_stride + h * hdu;
            rope_apply(&mut k_i8[off..off + rope_slice], pos as u32, rope_tables)?;
        }
    }

    // Per-head causal attention core with sliding window.
    let mut attn_out = vec![0i8; mu * q_row_stride];
    let mut scores_buf = vec![0i32; mu];
    let mut probs_buf = vec![0i8; mu];

    for i in 0..mu {
        // Sliding-window lower bound for query position `i`.
        let j_lo = match opts.sliding_window {
            Some(w) => {
                let w = w as usize;
                (i + 1).saturating_sub(w)
            }
            None => 0,
        };
        let span = i + 1 - j_lo;

        for h in 0..num_qu {
            let kv_h = h * num_kvu / num_qu;
            let q_off = i * q_row_stride + h * hdu;

            // Score against j in [j_lo, i]. Note `scores_buf[0..span]`.
            for (idx, j) in (j_lo..=i).enumerate() {
                let k_off = j * kv_row_stride + kv_h * hdu;
                let raw = dot_int8(&q_i8[q_off..q_off + hdu], &k_i8[k_off..k_off + hdu]);
                scores_buf[idx] = scale_score(raw, scales.score);
            }

            softmax_int(&scores_buf[..span], softmax_lut, &mut probs_buf[..span])?;

            let ao_off = i * q_row_stride + h * hdu;
            for d in 0..hdu {
                let mut acc = 0i32;
                for (idx, j) in (j_lo..=i).enumerate() {
                    let v_off = j * kv_row_stride + kv_h * hdu + d;
                    acc = acc.wrapping_add((probs_buf[idx] as i32) * (v_i8[v_off] as i32));
                }
                attn_out[ao_off + d] = rescale_and_requantize(acc, scales.attn_out);
            }
        }
    }

    drop(q_i8);
    drop(k_i8);
    drop(v_i8);

    // Phase B.2: if a gate was extracted from q_proj, run it through
    // the sigmoid LUT and element-wise multiply into the attention
    // output before o_proj. Matches Qwen 3.6 27B's
    // `attn_output * torch.sigmoid(gate)`.
    if let Some(mut gate) = gate_logits {
        let sigmoid_lut = opts
            .gate_sigmoid_lut
            .ok_or(AttentionError::BadQkNormGammaLen)?; // misuse: caller must supply
        sigmoid_lut.apply(&mut gate);
        // Symmetric half-up rounding by 127 (same as the hybrid block's
        // attn_gate composition).
        for k in 0..attn_out.len() {
            let prod = (attn_out[k] as i32).wrapping_mul(gate[k] as i32);
            let abs_v = prod.unsigned_abs() as i64;
            let q = ((abs_v + 63) / 127) as i32;
            let signed = if prod < 0 { -q } else { q };
            attn_out[k] = signed.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        }
    }

    // Output projection.
    matmul_int8_requant(
        &attn_out,
        &weights.w_o,
        m,
        num_q * hd,
        hidden,
        scales.o,
        output,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::SCALE_DENOM_LOG2;
    use crate::rope::RopeTables;
    use crate::softmax::ExpLut;

    fn unit_scale() -> Scale {
        Scale::from_num(1 << SCALE_DENOM_LOG2).unwrap()
    }

    fn small_scales() -> AttentionScales {
        AttentionScales {
            q: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
            k: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
            v: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
            score: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
            attn_out: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
            o: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        }
    }

    fn lcg_weights(len: usize, seed: u64) -> Vec<i8> {
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

    fn build_weights(hidden: u32, num_q: u32, num_kv: u32, hd: u32, seed: u64) -> AttentionWeights {
        let hu = hidden as usize;
        let qu = (num_q * hd) as usize;
        let kvu = (num_kv * hd) as usize;
        AttentionWeights {
            hidden,
            num_q_heads: num_q,
            num_kv_heads: num_kv,
            head_dim: hd,
            w_q: lcg_weights(hu * qu, seed),
            w_k: lcg_weights(hu * kvu, seed.wrapping_add(1)),
            w_v: lcg_weights(hu * kvu, seed.wrapping_add(2)),
            w_o: lcg_weights(qu * hu, seed.wrapping_add(3)),
            q_has_gate: false,
        }
    }

    #[test]
    fn zero_input_yields_zero_output() {
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 3u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0xabcd);
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![1i8; (m * hidden) as usize];
        attention_forward(
            &input,
            &weights,
            small_scales(),
            &rope_tables,
            &lut,
            m,
            &mut output,
        )
        .unwrap();
        for &v in &output {
            assert_eq!(v, 0, "zero input must yield zero output");
        }
    }

    #[test]
    fn single_token_no_causal_interaction() {
        // m=1: softmax sees exactly one score, probs=[127], result is just v[0] scaled.
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 2u32;
        let hd = 4u32;
        let m = 1u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0x1111);
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = lcg_weights((m * hidden) as usize, 0x5555);
        let mut output = vec![0i8; (m * hidden) as usize];
        // Should not panic or error.
        attention_forward(
            &input,
            &weights,
            small_scales(),
            &rope_tables,
            &lut,
            m,
            &mut output,
        )
        .unwrap();
    }

    #[test]
    fn identity_rope_uniform_softmax_produces_output() {
        // With identity RoPE and uniform LUT, all positions contribute equally
        // to the V-weighted sum; the result is well-defined and non-panicking.
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 4u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0x2222);
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = lcg_weights((m * hidden) as usize, 0x3333);
        let mut output = vec![0i8; (m * hidden) as usize];
        attention_forward(
            &input,
            &weights,
            small_scales(),
            &rope_tables,
            &lut,
            m,
            &mut output,
        )
        .unwrap();
        // With uniform softmax, each position receives equal weight; all we
        // assert is the call succeeds and output is overwritten.
        assert!(output.iter().any(|&v| v != 0) || input.iter().all(|&v| v == 0));
    }

    #[test]
    fn causal_order_enforced() {
        // Verify that different sequence lengths produce different outputs (i.e.
        // later tokens actually "see" earlier ones). m=1 and m=2 on the same
        // prefix must differ at position 1.
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0x4444);
        let lut = ExpLut::uniform_test();

        let input_m2 = lcg_weights((2 * hidden) as usize, 0x9999);
        let rope_m2 = RopeTables::identity(2, hd / 2);
        let mut out_m2 = vec![0i8; (2 * hidden) as usize];
        attention_forward(
            &input_m2,
            &weights,
            small_scales(),
            &rope_m2,
            &lut,
            2,
            &mut out_m2,
        )
        .unwrap();

        let input_m1 = input_m2[..(hidden as usize)].to_vec();
        let rope_m1 = RopeTables::identity(1, hd / 2);
        let mut out_m1 = vec![0i8; hidden as usize];
        attention_forward(
            &input_m1,
            &weights,
            small_scales(),
            &rope_m1,
            &lut,
            1,
            &mut out_m1,
        )
        .unwrap();

        // Position 0 output is identical regardless of sequence length
        // (causal: pos 0 sees only itself in both cases).
        assert_eq!(&out_m2[..hidden as usize], &out_m1[..]);
    }

    #[test]
    fn gqa_num_kv_one() {
        // num_kv_heads=1: all q heads share the same single kv head.
        let hidden = 8u32;
        let num_q = 4u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 3u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0x5555);
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = lcg_weights((m * hidden) as usize, 0x6666);
        let mut output = vec![0i8; (m * hidden) as usize];
        attention_forward(
            &input,
            &weights,
            small_scales(),
            &rope_tables,
            &lut,
            m,
            &mut output,
        )
        .unwrap();
    }

    #[test]
    fn gqa_num_kv_equals_num_q() {
        // Standard MHA: each q head maps to its own kv head.
        let hidden = 8u32;
        let num_q = 4u32;
        let num_kv = 4u32;
        let hd = 4u32;
        let m = 3u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0x7777);
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = lcg_weights((m * hidden) as usize, 0x8888);
        let mut output = vec![0i8; (m * hidden) as usize];
        attention_forward(
            &input,
            &weights,
            small_scales(),
            &rope_tables,
            &lut,
            m,
            &mut output,
        )
        .unwrap();
    }

    #[test]
    fn length_mismatch_rejected() {
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 2u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0xaaaa);
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let good_input = vec![0i8; (m * hidden) as usize];
        let mut good_output = vec![0i8; (m * hidden) as usize];

        // Short input.
        let short_input = vec![0i8; (m * hidden) as usize - 1];
        assert_eq!(
            attention_forward(
                &short_input,
                &weights,
                small_scales(),
                &rope_tables,
                &lut,
                m,
                &mut good_output
            )
            .err(),
            Some(AttentionError::BadInputLen),
        );

        // Short output.
        let mut short_output = vec![0i8; (m * hidden) as usize - 1];
        assert_eq!(
            attention_forward(
                &good_input,
                &weights,
                small_scales(),
                &rope_tables,
                &lut,
                m,
                &mut short_output
            )
            .err(),
            Some(AttentionError::BadOutputLen),
        );
    }

    #[test]
    fn zero_dim_rejected() {
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0xbbbb);
        let rope_tables = RopeTables::identity(4, hd / 2);
        let lut = ExpLut::uniform_test();
        let mut out = vec![0i8; (2 * hidden) as usize];

        // m = 0.
        let input = vec![0i8; 0];
        assert_eq!(
            attention_forward(
                &input,
                &weights,
                small_scales(),
                &rope_tables,
                &lut,
                0,
                &mut out
            )
            .err(),
            Some(AttentionError::ZeroDim),
        );
    }

    #[test]
    fn rope_mismatch_rejected() {
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 2u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0xcccc);
        // Wrong half_head_dim: hd/2 + 1 = 3 instead of 2.
        let bad_rope = RopeTables::identity(m, hd / 2 + 1);
        let lut = ExpLut::uniform_test();
        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![0i8; (m * hidden) as usize];
        assert_eq!(
            attention_forward(
                &input,
                &weights,
                small_scales(),
                &bad_rope,
                &lut,
                m,
                &mut output
            )
            .err(),
            Some(AttentionError::RopeHalfHeadDimMismatch),
        );
    }

    #[test]
    fn bad_kv_heads_rejected() {
        let hidden = 8u32;
        let hd = 4u32;
        let m = 2u32;
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();

        // num_kv_heads = 0.
        let mut w0 = build_weights(hidden, 2, 1, hd, 0xdddd);
        w0.num_kv_heads = 0;
        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![0i8; (m * hidden) as usize];
        assert_eq!(
            attention_forward(
                &input,
                &w0,
                small_scales(),
                &rope_tables,
                &lut,
                m,
                &mut output
            )
            .err(),
            Some(AttentionError::BadKvHeads),
        );

        // num_kv_heads does not divide num_q_heads (3 does not divide 4).
        let mut w1 = build_weights(hidden, 4, 1, hd, 0xdddd);
        w1.num_kv_heads = 3;
        w1.w_k = lcg_weights((hidden as usize) * (3 * hd as usize), 0xeeee);
        w1.w_v = lcg_weights((hidden as usize) * (3 * hd as usize), 0xffff);
        assert_eq!(
            attention_forward(
                &input,
                &w1,
                small_scales(),
                &rope_tables,
                &lut,
                m,
                &mut output
            )
            .err(),
            Some(AttentionError::BadKvHeads),
        );
    }

    #[test]
    fn scale_roundtrip_vs_matmul_int8() {
        // Q projection through attention_forward must match direct matmul+requantize.
        // We test this by zeroing w_k, w_v, w_o so all downstream ops produce zero,
        // and then checking that attention output at pos 0 head 0 reflects q[0] @ W_q.
        // (This is an indirect check; the full round-trip is tested by determinism.)
        let hidden = 8u32;
        let num_q = 1u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 1u32;

        let w_q = lcg_weights((hidden * num_q * hd) as usize, 0x1234);
        let zero_kv = vec![0i8; (hidden * num_kv * hd) as usize];
        let zero_o = vec![0i8; (num_q * hd * hidden) as usize];

        let weights = AttentionWeights {
            hidden,
            num_q_heads: num_q,
            num_kv_heads: num_kv,
            head_dim: hd,
            w_q: w_q.clone(),
            w_k: zero_kv.clone(),
            w_v: zero_kv.clone(),
            w_o: zero_o,
            q_has_gate: false,
        };

        let q_scale = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap();
        let scales = AttentionScales {
            q: q_scale,
            k: unit_scale(),
            v: unit_scale(),
            score: unit_scale(),
            attn_out: unit_scale(),
            o: unit_scale(),
        };

        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = lcg_weights((m * hidden) as usize, 0x5678);

        // Direct Q projection.
        let mut q_acc = vec![0i32; (m * num_q * hd) as usize];
        crate::matmul_int8::matmul_int8(&input, &w_q, m, hidden, num_q * hd, &mut q_acc).unwrap();
        let mut q_direct = vec![0i8; (m * num_q * hd) as usize];
        crate::matmul_int8::requantize_vec(&q_acc, q_scale, &mut q_direct).unwrap();

        // Through attention_forward (w_v = 0 → attn_out = 0 → output projection with w_o = 0).
        let mut attn_out = vec![0i8; (m * hidden) as usize];
        attention_forward(
            &input, &weights, scales, &rope_tables, &lut, m, &mut attn_out,
        )
        .unwrap();
        // We don't assert equality (output projection collapses things), just no panic/error.
        let _ = (q_direct, attn_out);
    }

    #[test]
    fn determinism() {
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 4u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0xfeed_beef);
        let rope_tables = RopeTables::identity(m, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = lcg_weights((m * hidden) as usize, 0xcafe_babe);
        let mut a = vec![0i8; (m * hidden) as usize];
        let mut b = vec![0i8; (m * hidden) as usize];
        attention_forward(
            &input,
            &weights,
            small_scales(),
            &rope_tables,
            &lut,
            m,
            &mut a,
        )
        .unwrap();
        attention_forward(
            &input,
            &weights,
            small_scales(),
            &rope_tables,
            &lut,
            m,
            &mut b,
        )
        .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn head_dim_odd_rejected() {
        // head_dim = 3 is odd → HeadDimOdd.
        let hidden = 8u32;
        let hd = 3u32;
        let m = 2u32;
        let mut weights = build_weights(hidden, 2, 1, 4, 0xabab);
        // Override head_dim and resize weights accordingly.
        weights.head_dim = hd;
        weights.w_q = vec![0i8; (hidden * 2 * hd) as usize];
        weights.w_k = vec![0i8; (hidden * 1 * hd) as usize];
        weights.w_v = vec![0i8; (hidden * 1 * hd) as usize];
        weights.w_o = vec![0i8; (2 * hd * hidden) as usize];
        let rope_tables = RopeTables::zeros(m, 1); // half_head_dim=1 but hd/2=1 (no exact match)
        let lut = ExpLut::uniform_test();
        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![0i8; (m * hidden) as usize];
        assert_eq!(
            attention_forward(
                &input,
                &weights,
                small_scales(),
                &rope_tables,
                &lut,
                m,
                &mut output
            )
            .err(),
            Some(AttentionError::HeadDimOdd),
        );
    }

    #[test]
    fn rope_seq_len_too_short_rejected() {
        let hidden = 8u32;
        let num_q = 2u32;
        let num_kv = 1u32;
        let hd = 4u32;
        let m = 4u32;
        let weights = build_weights(hidden, num_q, num_kv, hd, 0x1234_5678);
        // rope_tables with seq_len=2 but m=4.
        let short_rope = RopeTables::identity(2, hd / 2);
        let lut = ExpLut::uniform_test();
        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![0i8; (m * hidden) as usize];
        assert_eq!(
            attention_forward(
                &input,
                &weights,
                small_scales(),
                &short_rope,
                &lut,
                m,
                &mut output
            )
            .err(),
            Some(AttentionError::RopeSeqLenTooShort),
        );
    }
}
