"""Numpy reference for the composed forward pass.

Mirrors `crate::attention::attention_forward`, `crate::deltanet::deltanet_forward`,
`crate::layer::forward_layer`, and `crate::forward::forward_prefix` exactly.
The per-op primitives live in `reference_ops.py`; this module composes
them into the full inference pipeline.

Cross-impl validation: feeding the same Model + prompt to both the Rust
crate and this module must produce byte-identical output tensors at every
recorded layer.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, Sequence

import reference_ops as R


# -----------------------------------------------------------------------------
# RoPE (mirrors src/rope.rs).
# -----------------------------------------------------------------------------

ROPE_FRACT_BITS = 14


@dataclass(frozen=True)
class RopeTables:
    seq_len: int
    half_head_dim: int
    cos: tuple[int, ...]  # i16 values, length seq_len * half_head_dim
    sin: tuple[int, ...]

    @classmethod
    def identity(cls, seq_len: int, half_head_dim: int) -> "RopeTables":
        n = seq_len * half_head_dim
        return cls(
            seq_len=seq_len,
            half_head_dim=half_head_dim,
            cos=tuple(1 << ROPE_FRACT_BITS for _ in range(n)),
            sin=tuple(0 for _ in range(n)),
        )

    def lookup(self, pos: int, j: int) -> tuple[int, int]:
        off = pos * self.half_head_dim + j
        return self.cos[off], self.sin[off]


def rope_apply(x: list[int], pos: int, tables: RopeTables) -> None:
    """In-place RoPE over a 2*half_head_dim slice."""
    n = 2 * tables.half_head_dim
    if len(x) != n:
        raise ValueError("rope_apply length mismatch")
    if pos >= tables.seq_len:
        raise ValueError("rope position out of range")
    for j in range(tables.half_head_dim):
        cos_v, sin_v = tables.lookup(pos, j)
        i = j * 2
        x0 = x[i]
        x1 = x[i + 1]
        # (x0*cos - x1*sin) / 2^14, banker's; same in src/rope.rs.
        r0 = R.round_half_to_even_div_pow2(x0 * cos_v - x1 * sin_v, ROPE_FRACT_BITS)
        r1 = R.round_half_to_even_div_pow2(x0 * sin_v + x1 * cos_v, ROPE_FRACT_BITS)
        x[i] = R.saturate_i8(r0)
        x[i + 1] = R.saturate_i8(r1)


# -----------------------------------------------------------------------------
# Attention (mirrors src/attention.rs).
# -----------------------------------------------------------------------------


@dataclass(frozen=True)
class AttentionWeights:
    hidden: int
    num_q_heads: int
    num_kv_heads: int
    head_dim: int
    w_q: tuple[int, ...]
    w_k: tuple[int, ...]
    w_v: tuple[int, ...]
    w_o: tuple[int, ...]


@dataclass(frozen=True)
class AttentionScales:
    q: R.Scale
    k: R.Scale
    v: R.Scale
    score: R.Scale
    attn_out: R.Scale
    o: R.Scale


def _scale_score(raw: int, scale: R.Scale) -> int:
    """i32 dot → i32 softmax-domain via banker's rounding."""
    product = raw * scale.num
    rounded = R.round_half_to_even_div_pow2(product, R.SCALE_DENOM_LOG2)
    return max(R.I32_MIN, min(R.I32_MAX, rounded))


def attention_forward(
    inp: Sequence[int],
    weights: AttentionWeights,
    scales: AttentionScales,
    rope_tables: RopeTables,
    softmax_lut: R.ExpLut,
    m: int,
) -> list[int]:
    """(m, hidden) row-major i8 → (m, hidden) row-major i8."""
    hd = weights.head_dim
    if hd % 2 != 0:
        raise ValueError("head_dim must be even for RoPE")
    if rope_tables.half_head_dim != hd // 2:
        raise ValueError("rope half_head_dim mismatch")
    if rope_tables.seq_len < m:
        raise ValueError("rope seq_len too short")
    nq = weights.num_q_heads
    nkv = weights.num_kv_heads
    if nkv == 0 or nkv > nq or nq % nkv != 0:
        raise ValueError("bad kv heads")

    hu = weights.hidden
    q_stride = nq * hd
    kv_stride = nkv * hd

    # Q/K/V projection.
    q_acc = R.matmul_int8(inp, weights.w_q, m, hu, nq * hd)
    q_i8 = R.requantize_vec(q_acc, scales.q)
    k_acc = R.matmul_int8(inp, weights.w_k, m, hu, nkv * hd)
    k_i8 = R.requantize_vec(k_acc, scales.k)
    v_acc = R.matmul_int8(inp, weights.w_v, m, hu, nkv * hd)
    v_i8 = R.requantize_vec(v_acc, scales.v)

    # RoPE on Q and K (in-place over per-head slices).
    for pos in range(m):
        for h in range(nq):
            off = pos * q_stride + h * hd
            slot = q_i8[off : off + hd]
            rope_apply(slot, pos, rope_tables)
            q_i8[off : off + hd] = slot
        for h in range(nkv):
            off = pos * kv_stride + h * hd
            slot = k_i8[off : off + hd]
            rope_apply(slot, pos, rope_tables)
            k_i8[off : off + hd] = slot

    # Per-head causal attention core.
    attn_out = [0] * (m * q_stride)
    for i in range(m):
        for h in range(nq):
            kv_h = (h * nkv) // nq
            scores: list[int] = []
            q_off = i * q_stride + h * hd
            for j in range(i + 1):
                k_off = j * kv_stride + kv_h * hd
                raw = R.dot_int8(q_i8[q_off : q_off + hd], k_i8[k_off : k_off + hd])
                scores.append(_scale_score(raw, scales.score))
            probs = R.softmax_int(scores, softmax_lut)
            ao_off = i * q_stride + h * hd
            for d in range(hd):
                acc = 0
                for j in range(i + 1):
                    v_off = j * kv_stride + kv_h * hd + d
                    acc += probs[j] * v_i8[v_off]
                acc = max(R.I32_MIN, min(R.I32_MAX, acc))
                attn_out[ao_off + d] = R.rescale_and_requantize(acc, scales.attn_out)

    # Output projection.
    return R.matmul_int8_requant(attn_out, weights.w_o, m, nq * hd, hu, scales.o)


# -----------------------------------------------------------------------------
# DeltaNet (mirrors src/deltanet.rs).
# -----------------------------------------------------------------------------


@dataclass(frozen=True)
class DeltaNetWeights:
    hidden: int
    num_qk_heads: int
    num_v_heads: int
    head_dim_qk: int
    head_dim_v: int
    w_q: tuple[int, ...]
    w_k: tuple[int, ...]
    w_v: tuple[int, ...]
    w_alpha: tuple[int, ...]
    w_beta: tuple[int, ...]
    w_o: tuple[int, ...]


@dataclass(frozen=True)
class DeltaNetScales:
    q: R.Scale
    k: R.Scale
    v: R.Scale
    alpha_logit: R.Scale
    beta_logit: R.Scale
    u: R.Scale
    decay: R.Scale
    update: R.Scale
    o: R.Scale
    proj: R.Scale


def deltanet_head_step(
    state_v: list[int],
    head_dim_qk: int,
    head_dim_v: int,
    q: Sequence[int],
    k: Sequence[int],
    v: Sequence[int],
    alpha: int,
    beta: int,
    u_scale: R.Scale,
    decay_scale: R.Scale,
    update_scale: R.Scale,
    o_scale: R.Scale,
) -> list[int]:
    # u[d] = Σ_i state[i, d] * k[i].
    u = [0] * head_dim_v
    for d in range(head_dim_v):
        acc = 0
        for i in range(head_dim_qk):
            acc += state_v[i * head_dim_v + d] * k[i]
        u[d] = max(R.I32_MIN, min(R.I32_MAX, acc))

    # u_i8 = rescale(u, u_scale).
    u_i8 = [R.rescale_and_requantize(uv, u_scale) for uv in u]

    # State update.
    for i in range(head_dim_qk):
        ki32 = k[i]
        alpha_k = alpha * ki32
        beta_k = beta * ki32
        for d in range(head_dim_v):
            off = i * head_dim_v + d
            s_old = state_v[off]
            decay_raw = alpha_k * u_i8[d]
            update_raw = beta_k * v[d]
            decay_i8 = R.rescale_and_requantize(
                max(R.I32_MIN, min(R.I32_MAX, decay_raw)), decay_scale
            )
            update_i8 = R.rescale_and_requantize(
                max(R.I32_MIN, min(R.I32_MAX, update_raw)), update_scale
            )
            s_new = s_old - decay_i8 + update_i8
            state_v[off] = R.saturate_i8(s_new)

    # Output: o[d] = rescale(Σ_i state_new[i, d] * q[i], o_scale).
    out_v = [0] * head_dim_v
    for d in range(head_dim_v):
        acc = 0
        for i in range(head_dim_qk):
            acc += state_v[i * head_dim_v + d] * q[i]
        acc = max(R.I32_MIN, min(R.I32_MAX, acc))
        out_v[d] = R.rescale_and_requantize(acc, o_scale)
    return out_v


def deltanet_forward(
    inp: Sequence[int],
    weights: DeltaNetWeights,
    scales: DeltaNetScales,
    sigmoid_lut_bytes: Sequence[int],
    m: int,
) -> list[int]:
    nq = weights.num_qk_heads
    nv = weights.num_v_heads
    if nv < nq or nv % nq != 0:
        raise ValueError("bad v heads")
    hu = weights.hidden
    hdq = weights.head_dim_qk
    hdv = weights.head_dim_v
    q_stride = nq * hdq
    v_stride = nv * hdv

    # Project Q, K, V.
    q_i8 = R.requantize_vec(R.matmul_int8(inp, weights.w_q, m, hu, nq * hdq), scales.q)
    k_i8 = R.requantize_vec(R.matmul_int8(inp, weights.w_k, m, hu, nq * hdq), scales.k)
    v_i8 = R.requantize_vec(R.matmul_int8(inp, weights.w_v, m, hu, nv * hdv), scales.v)

    # alpha/beta logits → sigmoid LUT.
    alpha_acc = R.matmul_int8(inp, weights.w_alpha, m, hu, nq)
    alpha_i8 = R.requantize_vec(alpha_acc, scales.alpha_logit)
    alpha_i8 = R.apply_activation_lut(alpha_i8, sigmoid_lut_bytes)
    beta_acc = R.matmul_int8(inp, weights.w_beta, m, hu, nq)
    beta_i8 = R.requantize_vec(beta_acc, scales.beta_logit)
    beta_i8 = R.apply_activation_lut(beta_i8, sigmoid_lut_bytes)

    # State per V head: head_dim_qk x head_dim_v i8.
    state_per_head = hdq * hdv
    state = [[0] * state_per_head for _ in range(nv)]

    # Per-token, per-V-head update.
    out_concat = [0] * (m * v_stride)
    for t in range(m):
        for v_head in range(nv):
            qk_head = (v_head * nq) // nv
            q_off = t * q_stride + qk_head * hdq
            k_off = t * q_stride + qk_head * hdq
            v_off = t * v_stride + v_head * hdv
            alpha = alpha_i8[t * nq + qk_head]
            beta = beta_i8[t * nq + qk_head]
            out_v = deltanet_head_step(
                state[v_head],
                hdq,
                hdv,
                q_i8[q_off : q_off + hdq],
                k_i8[k_off : k_off + hdq],
                v_i8[v_off : v_off + hdv],
                alpha,
                beta,
                scales.u,
                scales.decay,
                scales.update,
                scales.o,
            )
            ao_off = t * v_stride + v_head * hdv
            out_concat[ao_off : ao_off + hdv] = out_v

    return R.matmul_int8_requant(out_concat, weights.w_o, m, nv * hdv, hu, scales.proj)


# -----------------------------------------------------------------------------
# Per-layer composition (mirrors src/layer.rs).
# -----------------------------------------------------------------------------


@dataclass(frozen=True)
class FfnWeights:
    hidden: int
    intermediate: int
    w_gate: tuple[int, ...]
    w_up: tuple[int, ...]
    w_down: tuple[int, ...]


@dataclass(frozen=True)
class NormSpec:
    """Tagged norm: kind ∈ {"rms", "ln"}; beta is None for RmsNorm."""

    kind: str
    gamma: tuple[int, ...]
    beta: Optional[tuple[int, ...]]
    eps_q: int
    post_scale: R.Scale


def apply_norm_per_token(
    norm: NormSpec, inp: Sequence[int], m: int, hidden: int
) -> list[int]:
    out: list[int] = [0] * (m * hidden)
    for t in range(m):
        row = inp[t * hidden : (t + 1) * hidden]
        if norm.kind == "rms":
            acc = R.rmsnorm(row, norm.gamma, eps_q=norm.eps_q)
        elif norm.kind == "ln":
            assert norm.beta is not None
            acc = R.layernorm(row, norm.gamma, norm.beta, eps_q=norm.eps_q)
        else:
            raise ValueError(f"unknown norm kind {norm.kind}")
        for d in range(hidden):
            out[t * hidden + d] = R.rescale_and_requantize(acc[d], norm.post_scale)
    return out


def add_residual_inplace(dst: list[int], addend: Sequence[int]) -> None:
    for i in range(len(dst)):
        dst[i] = R.saturate_i8(dst[i] + addend[i])


@dataclass(frozen=True)
class GemmaLayer:
    """Phase 2.11 Gemma 4 transformer block. Mirrors
    `crate::layer::LayerWeights::Gemma` field-for-field."""

    norm1: NormSpec
    attn: AttentionWeights
    attn_scales: AttentionScales
    q_norm_gamma: tuple[int, ...]
    k_norm_gamma: tuple[int, ...]
    qk_norm_eps_q: int
    qk_norm_post_scale: R.Scale
    post_attn_norm: NormSpec
    norm2: NormSpec
    ffn: FfnWeights
    ffn_scales: R.FfnScales
    post_ffn_norm: NormSpec
    sliding_window: Optional[int]
    inp_gate: Optional[tuple[int, ...]]
    layer_output_scale: Optional[tuple[int, ...]]


def _scale_score_py(raw: int, scale: R.Scale) -> int:
    """Mirror of `crate::attention::scale_score`. Used by both
    standard and Gemma attention forward references."""
    product = raw * scale.num
    rounded = R.round_half_to_even_div_pow2(product, R.SCALE_DENOM_LOG2)
    return max(R.I32_MIN, min(R.I32_MAX, rounded))


def attention_forward_gemma(
    inp: Sequence[int],
    weights: AttentionWeights,
    scales: AttentionScales,
    rope_tables: RopeTables,
    softmax_lut: R.ExpLut,
    q_norm_gamma: Sequence[int],
    k_norm_gamma: Sequence[int],
    qk_norm_eps_q: int,
    qk_norm_post_scale: R.Scale,
    sliding_window: Optional[int],
    m: int,
) -> list[int]:
    """Mirror of `crate::attention::attention_forward_gemma`. Adds
    QK norm before RoPE + sliding-window mask."""
    hd = weights.head_dim
    if hd % 2 != 0:
        raise ValueError("head_dim must be even")
    nq = weights.num_q_heads
    nkv = weights.num_kv_heads
    hu = weights.hidden
    q_stride = nq * hd
    kv_stride = nkv * hd

    # Q/K/V projections.
    q_i8 = R.requantize_vec(R.matmul_int8(inp, weights.w_q, m, hu, nq * hd), scales.q)
    k_i8 = R.requantize_vec(R.matmul_int8(inp, weights.w_k, m, hu, nkv * hd), scales.k)
    v_i8 = R.requantize_vec(R.matmul_int8(inp, weights.w_v, m, hu, nkv * hd), scales.v)

    # QK norm.
    for t in range(m):
        for h in range(nq):
            off = t * q_stride + h * hd
            slot = q_i8[off : off + hd]
            normed = R.rmsnorm(slot, q_norm_gamma, eps_q=qk_norm_eps_q)
            for d in range(hd):
                q_i8[off + d] = R.rescale_and_requantize(normed[d], qk_norm_post_scale)
        for h in range(nkv):
            off = t * kv_stride + h * hd
            slot = k_i8[off : off + hd]
            normed = R.rmsnorm(slot, k_norm_gamma, eps_q=qk_norm_eps_q)
            for d in range(hd):
                k_i8[off + d] = R.rescale_and_requantize(normed[d], qk_norm_post_scale)

    # RoPE.
    for pos in range(m):
        for h in range(nq):
            off = pos * q_stride + h * hd
            slot = list(q_i8[off : off + hd])
            rope_apply(slot, pos, rope_tables)
            q_i8[off : off + hd] = slot
        for h in range(nkv):
            off = pos * kv_stride + h * hd
            slot = list(k_i8[off : off + hd])
            rope_apply(slot, pos, rope_tables)
            k_i8[off : off + hd] = slot

    # Per-head attention with sliding-window mask.
    attn_out = [0] * (m * q_stride)
    for i in range(m):
        if sliding_window is None:
            j_lo = 0
        else:
            j_lo = max(0, i + 1 - sliding_window)
        for h in range(nq):
            kv_h = (h * nkv) // nq
            q_off = i * q_stride + h * hd
            scores = []
            for j in range(j_lo, i + 1):
                k_off = j * kv_stride + kv_h * hd
                raw = R.dot_int8(q_i8[q_off : q_off + hd], k_i8[k_off : k_off + hd])
                scores.append(_scale_score_py(raw, scales.score))
            probs = R.softmax_int(scores, softmax_lut)
            ao_off = i * q_stride + h * hd
            for d in range(hd):
                acc = 0
                for idx, j in enumerate(range(j_lo, i + 1)):
                    v_off = j * kv_stride + kv_h * hd + d
                    acc += probs[idx] * v_i8[v_off]
                acc = max(R.I32_MIN, min(R.I32_MAX, acc))
                attn_out[ao_off + d] = R.rescale_and_requantize(acc, scales.attn_out)

    return R.matmul_int8_requant(attn_out, weights.w_o, m, nq * hd, hu, scales.o)


def _apply_channelwise_scale(xs: list[int], scale: Sequence[int], hidden: int) -> None:
    """Mirror of `crate::layer::apply_channelwise_scale`. Symmetric
    half-up: round(|v|/127) preserving sign."""
    for t in range(len(xs) // hidden):
        for c in range(hidden):
            v = xs[t * hidden + c] * scale[c]
            abs_v = abs(v)
            q = (abs_v + 63) // 127
            signed = -q if v < 0 else q
            xs[t * hidden + c] = max(R.I8_MIN, min(R.I8_MAX, signed))


def forward_gemma_layer(
    inp: Sequence[int],
    layer: GemmaLayer,
    ctx: LayerContext,
    m: int,
) -> list[int]:
    """Mirror of `crate::layer::forward_gemma_layer`."""
    hidden = layer.attn.hidden
    # Step 0: input gate (optional).
    x = list(inp)
    if layer.inp_gate is not None:
        _apply_channelwise_scale(x, layer.inp_gate, hidden)

    # Step 1: norm1.
    normed1 = apply_norm_per_token(layer.norm1, x, m, hidden)

    # Step 2: gemma attention.
    sub = attention_forward_gemma(
        normed1,
        layer.attn,
        layer.attn_scales,
        ctx.rope_tables,
        ctx.softmax_lut,
        layer.q_norm_gamma,
        layer.k_norm_gamma,
        layer.qk_norm_eps_q,
        layer.qk_norm_post_scale,
        layer.sliding_window,
        m,
    )

    # Step 3: post-attn norm.
    post_attn = apply_norm_per_token(layer.post_attn_norm, sub, m, hidden)

    # Step 4: residual1.
    residual1 = list(x)
    add_residual_inplace(residual1, post_attn)

    # Step 5: norm2.
    normed2 = apply_norm_per_token(layer.norm2, residual1, m, hidden)

    # Step 6: FFN.
    ffn_out = R.ffn_forward(
        normed2,
        layer.ffn.w_gate,
        layer.ffn.w_up,
        layer.ffn.w_down,
        ctx.ffn_activation_bytes,
        layer.ffn_scales,
        m,
        hidden,
        layer.ffn.intermediate,
    )

    # Step 7: post-FFN norm.
    post_ffn = apply_norm_per_token(layer.post_ffn_norm, ffn_out, m, hidden)

    # Step 8: layer_output_scale (optional).
    if layer.layer_output_scale is not None:
        _apply_channelwise_scale(post_ffn, layer.layer_output_scale, hidden)

    # Step 9: residual2.
    out = list(residual1)
    add_residual_inplace(out, post_ffn)
    return out


@dataclass(frozen=True)
class AttentionLayer:
    norm1: NormSpec
    attn: AttentionWeights
    attn_scales: AttentionScales
    norm2: NormSpec
    ffn: FfnWeights
    ffn_scales: R.FfnScales


@dataclass(frozen=True)
class DeltaNetLayer:
    norm1: NormSpec
    dnet: DeltaNetWeights
    dnet_scales: DeltaNetScales
    norm2: NormSpec
    ffn: FfnWeights
    ffn_scales: R.FfnScales


@dataclass(frozen=True)
class QwenStandardLayer:
    """Phase 2.12 Qwen 3.6 27B standard-attention block. Mirrors
    `crate::layer::LayerWeights::QwenStandard` field-for-field. Same
    2-norm residual structure as `AttentionLayer` plus per-head QK norm
    inside the attention sublayer (no sliding window for Qwen)."""

    norm1: NormSpec
    attn: AttentionWeights
    attn_scales: AttentionScales
    q_norm_gamma: tuple[int, ...]
    k_norm_gamma: tuple[int, ...]
    qk_norm_eps_q: int
    qk_norm_post_scale: R.Scale
    norm2: NormSpec
    ffn: FfnWeights
    ffn_scales: R.FfnScales


@dataclass(frozen=True)
class QwenHybridSsmLayer:
    """Phase 2.13 Qwen 3.6 27B hybrid attention + Mamba-SSM block.
    Mirrors `crate::layer::LayerWeights::QwenHybridSsm` field-for-field.
    The pre-attn norm output feeds two parallel sublayers (gated
    attention + Mamba SSM) that sum before the residual add."""

    norm1: NormSpec
    # Gated attention path:
    attn_qkv_fused: tuple[int, ...]  # (hidden, q_dim + kv_dim + kv_dim) col-major
    attn_gate: tuple[int, ...]  # (hidden, q_dim) col-major
    attn_out: tuple[int, ...]  # (q_dim, hidden) col-major
    num_q_heads: int
    num_kv_heads: int
    head_dim: int
    attn_scales: AttentionScales
    q_norm_gamma: tuple[int, ...]
    k_norm_gamma: tuple[int, ...]
    qk_norm_eps_q: int
    qk_norm_post_scale: R.Scale
    # SSM path:
    ssm_a: tuple[int, ...]  # (num_v_heads,)
    ssm_alpha: tuple[int, ...]  # (hidden, num_v_heads) col-major
    ssm_beta: tuple[int, ...]  # (hidden, num_v_heads) col-major
    ssm_conv1d: tuple[int, ...]  # (kernel_size, hidden) row-major
    ssm_dt: tuple[int, ...]  # (num_v_heads,)
    ssm_norm_gamma: tuple[int, ...]  # (head_dim,)
    ssm_norm_eps_q: int
    ssm_norm_post_scale: R.Scale
    ssm_out: tuple[int, ...]  # (num_v_heads * head_dim, hidden) col-major
    num_v_heads: int
    ssm_kernel_size: int
    ssm_scales: DeltaNetScales
    # Shared parts:
    norm2: NormSpec
    ffn: FfnWeights
    ffn_scales: R.FfnScales


def attention_forward_qwen_standard(
    inp: Sequence[int],
    weights: AttentionWeights,
    scales: AttentionScales,
    rope_tables: RopeTables,
    softmax_lut: R.ExpLut,
    q_norm_gamma: Sequence[int],
    k_norm_gamma: Sequence[int],
    qk_norm_eps_q: int,
    qk_norm_post_scale: R.Scale,
    m: int,
) -> list[int]:
    """Mirror of `crate::layer::forward_qwen_standard_layer`'s attention
    sublayer call: same as `attention_forward_gemma` with
    `sliding_window=None`."""
    return attention_forward_gemma(
        inp, weights, scales, rope_tables, softmax_lut,
        q_norm_gamma, k_norm_gamma, qk_norm_eps_q, qk_norm_post_scale,
        sliding_window=None, m=m,
    )


def forward_qwen_standard_layer(
    inp: Sequence[int],
    layer: QwenStandardLayer,
    ctx: LayerContext,
    m: int,
) -> list[int]:
    """Mirror of `crate::layer::forward_qwen_standard_layer`."""
    hidden = layer.attn.hidden

    # Step 1: norm1.
    normed1 = apply_norm_per_token(layer.norm1, inp, m, hidden)

    # Step 2: attention with QK norm, no sliding window.
    sub = attention_forward_qwen_standard(
        normed1,
        layer.attn,
        layer.attn_scales,
        ctx.rope_tables,
        ctx.softmax_lut,
        layer.q_norm_gamma,
        layer.k_norm_gamma,
        layer.qk_norm_eps_q,
        layer.qk_norm_post_scale,
        m,
    )

    # Step 3: residual1 = inp + sub.
    residual1 = list(sub)
    add_residual_inplace(residual1, inp)

    # Step 4: norm2.
    normed2 = apply_norm_per_token(layer.norm2, residual1, m, hidden)

    # Step 5: FFN.
    ffn_out = R.ffn_forward(
        normed2,
        layer.ffn.w_gate,
        layer.ffn.w_up,
        layer.ffn.w_down,
        ctx.ffn_activation_bytes,
        layer.ffn_scales,
        m,
        hidden,
        layer.ffn.intermediate,
    )

    # Step 6: residual2.
    out = list(residual1)
    add_residual_inplace(out, ffn_out)
    return out


def ssm_forward(
    inp: Sequence[int],
    hidden: int,
    m: int,
    ssm_a: Sequence[int],
    ssm_alpha: Sequence[int],
    ssm_beta: Sequence[int],
    ssm_conv1d: Sequence[int],
    ssm_dt: Sequence[int],
    ssm_norm_gamma: Sequence[int],
    ssm_norm_eps_q: int,
    ssm_norm_post_scale: R.Scale,
    ssm_out_w: Sequence[int],
    num_v_heads: int,
    head_dim: int,
    kernel_size: int,
    scales: DeltaNetScales,
    sigmoid_lut_bytes: Sequence[int],
) -> list[int]:
    """Mirror of `crate::ssm::ssm_forward`. INT8 selective state-space
    recurrence with depthwise causal 1D conv, per-token α/β gating, and
    per-V-head RMSNorm before output projection."""
    nv = num_v_heads
    hd = head_dim
    ksz = kernel_size
    hu = hidden

    # Step 1: depthwise causal 1D conv. Tap k=0 is the current token.
    x_conv = [0] * (m * hu)
    for t in range(m):
        for c in range(hu):
            acc = 0
            for k in range(ksz):
                if t < k:
                    break  # past-prefix taps zero (causal mask)
                in_v = inp[(t - k) * hu + c]
                kw = ssm_conv1d[k * hu + c]
                acc += in_v * kw
            acc = max(R.I32_MIN, min(R.I32_MAX, acc))
            x_conv[t * hu + c] = R.rescale_and_requantize(acc, scales.q)

    # Step 2: α and β projections, both followed by sigmoid LUT.
    alpha_acc = R.matmul_int8(x_conv, ssm_alpha, m, hu, nv)
    alpha_i8 = R.requantize_vec(alpha_acc, scales.alpha_logit)
    alpha_i8 = R.apply_activation_lut(alpha_i8, sigmoid_lut_bytes)

    beta_acc = R.matmul_int8(x_conv, ssm_beta, m, hu, nv)
    beta_i8 = R.requantize_vec(beta_acc, scales.beta_logit)
    beta_i8 = R.apply_activation_lut(beta_i8, sigmoid_lut_bytes)

    # Step 3: per-V-head state recurrence. State zero-initialized each call.
    state = [[0] * hd for _ in range(nv)]
    y_concat = [0] * (m * nv * hd)

    for t in range(m):
        for v in range(nv):
            alpha_v = alpha_i8[t * nv + v]
            beta_v = beta_i8[t * nv + v]
            a_v = ssm_a[v]
            dt_v = ssm_dt[v]
            # Same `>> 7` semantics as Rust arithmetic shift (preserves sign).
            decay_factor = R.saturate_i8((alpha_v * a_v) >> 7)
            update_factor = R.saturate_i8((beta_v * dt_v) >> 7)
            for d in range(hd):
                s_old = state[v][d]
                c = (v * hd + d) % hu
                xv = x_conv[t * hu + c]
                decay_term = s_old * decay_factor
                update_term = update_factor * xv
                decay_i8 = R.rescale_and_requantize(
                    max(R.I32_MIN, min(R.I32_MAX, decay_term)), scales.decay
                )
                update_i8 = R.rescale_and_requantize(
                    max(R.I32_MIN, min(R.I32_MAX, update_term)), scales.update
                )
                s_new = R.saturate_i8(decay_i8 + update_i8)
                state[v][d] = s_new
                y_concat[t * nv * hd + v * hd + d] = s_new

    # Step 4: per-(token, V-head) RMSNorm.
    for t in range(m):
        for v in range(nv):
            off = t * nv * hd + v * hd
            slot = y_concat[off : off + hd]
            normed = R.rmsnorm(slot, ssm_norm_gamma, eps_q=ssm_norm_eps_q)
            for d in range(hd):
                y_concat[off + d] = R.rescale_and_requantize(
                    normed[d], ssm_norm_post_scale
                )

    # Step 5: output projection.
    return R.matmul_int8_requant(
        y_concat, ssm_out_w, m, nv * hd, hu, scales.proj
    )


def gated_attention_forward(
    inp: Sequence[int],
    attn_qkv_fused: Sequence[int],
    attn_gate: Sequence[int],
    attn_out_w: Sequence[int],
    hidden: int,
    num_q_heads: int,
    num_kv_heads: int,
    head_dim: int,
    scales: AttentionScales,
    q_norm_gamma: Sequence[int],
    k_norm_gamma: Sequence[int],
    qk_norm_eps_q: int,
    qk_norm_post_scale: R.Scale,
    rope_tables: RopeTables,
    softmax_lut: R.ExpLut,
    sigmoid_lut_bytes: Sequence[int],
    m: int,
) -> list[int]:
    """Mirror of `crate::layer::gated_attention_forward`. Fused-QKV split
    + QK norm + RoPE + causal attention + sigmoid `attn_gate` ×
    attn_inter + `attn_out` projection. No sliding window (Qwen 3.6 doesn't
    use one in hybrid blocks)."""
    hu = hidden
    nq = num_q_heads
    nkv = num_kv_heads
    hd = head_dim
    q_dim = nq * hd
    kv_dim = nkv * hd
    total_qkv = q_dim + kv_dim + kv_dim

    # Split fused QKV (column-major: contiguous slice = sub-matrix columns).
    w_q = attn_qkv_fused[0 : hu * q_dim]
    w_k = attn_qkv_fused[hu * q_dim : hu * (q_dim + kv_dim)]
    w_v = attn_qkv_fused[hu * (q_dim + kv_dim) : hu * total_qkv]

    # Q/K/V projections.
    q_i8 = R.requantize_vec(R.matmul_int8(inp, w_q, m, hu, q_dim), scales.q)
    k_i8 = R.requantize_vec(R.matmul_int8(inp, w_k, m, hu, kv_dim), scales.k)
    v_i8 = R.requantize_vec(R.matmul_int8(inp, w_v, m, hu, kv_dim), scales.v)

    # QK norm: per (token, head). Order matches Rust:
    # outer loop t, inner h-q then h-kv.
    for t in range(m):
        for h in range(nq):
            off = t * q_dim + h * hd
            slot = q_i8[off : off + hd]
            normed = R.rmsnorm(slot, q_norm_gamma, eps_q=qk_norm_eps_q)
            for d in range(hd):
                q_i8[off + d] = R.rescale_and_requantize(normed[d], qk_norm_post_scale)
        for h in range(nkv):
            off = t * kv_dim + h * hd
            slot = k_i8[off : off + hd]
            normed = R.rmsnorm(slot, k_norm_gamma, eps_q=qk_norm_eps_q)
            for d in range(hd):
                k_i8[off + d] = R.rescale_and_requantize(normed[d], qk_norm_post_scale)

    # RoPE on Q and K.
    for pos in range(m):
        for h in range(nq):
            off = pos * q_dim + h * hd
            slot = list(q_i8[off : off + hd])
            rope_apply(slot, pos, rope_tables)
            q_i8[off : off + hd] = slot
        for h in range(nkv):
            off = pos * kv_dim + h * hd
            slot = list(k_i8[off : off + hd])
            rope_apply(slot, pos, rope_tables)
            k_i8[off : off + hd] = slot

    # Per-head causal attention (no sliding window).
    attn_inter = [0] * (m * q_dim)
    for i in range(m):
        for h in range(nq):
            kv_h = (h * nkv) // nq
            q_off = i * q_dim + h * hd
            scores = []
            for j in range(i + 1):
                k_off = j * kv_dim + kv_h * hd
                raw = R.dot_int8(q_i8[q_off : q_off + hd], k_i8[k_off : k_off + hd])
                scores.append(_scale_score_py(raw, scales.score))
            probs = R.softmax_int(scores, softmax_lut)
            ao_off = i * q_dim + h * hd
            for d in range(hd):
                acc = 0
                for j in range(i + 1):
                    v_off = j * kv_dim + kv_h * hd + d
                    acc += probs[j] * v_i8[v_off]
                acc = max(R.I32_MIN, min(R.I32_MAX, acc))
                attn_inter[ao_off + d] = R.rescale_and_requantize(acc, scales.attn_out)

    # Sigmoid gate: gate_i8 = sigmoid_lut(rescale(input @ attn_gate, scales.q)).
    gate_acc = R.matmul_int8(inp, attn_gate, m, hu, q_dim)
    gate_i8 = R.requantize_vec(gate_acc, scales.q)
    gate_i8 = R.apply_activation_lut(gate_i8, sigmoid_lut_bytes)

    # Element-wise multiply with symmetric half-up rounding by 127.
    for k in range(m * q_dim):
        prod = attn_inter[k] * gate_i8[k]
        abs_v = abs(prod)
        q = (abs_v + 63) // 127
        signed = -q if prod < 0 else q
        attn_inter[k] = max(R.I8_MIN, min(R.I8_MAX, signed))

    # Output projection.
    return R.matmul_int8_requant(attn_inter, attn_out_w, m, q_dim, hu, scales.o)


def forward_qwen_hybrid_ssm_layer(
    inp: Sequence[int],
    layer: QwenHybridSsmLayer,
    ctx: LayerContext,
    m: int,
) -> list[int]:
    """Mirror of `crate::layer::forward_qwen_hybrid_ssm_layer`. Two
    parallel sublayer paths (gated attention + SSM) summed before the
    residual add."""
    hidden = layer.ffn.hidden

    # Step 1: norm1.
    normed1 = apply_norm_per_token(layer.norm1, inp, m, hidden)

    # Step 2: gated attention path.
    y_attn = gated_attention_forward(
        normed1,
        layer.attn_qkv_fused,
        layer.attn_gate,
        layer.attn_out,
        hidden,
        layer.num_q_heads,
        layer.num_kv_heads,
        layer.head_dim,
        layer.attn_scales,
        layer.q_norm_gamma,
        layer.k_norm_gamma,
        layer.qk_norm_eps_q,
        layer.qk_norm_post_scale,
        ctx.rope_tables,
        ctx.softmax_lut,
        ctx.sigmoid_lut_bytes,
        m,
    )

    # Step 3: SSM path.
    y_ssm = ssm_forward(
        normed1,
        hidden,
        m,
        layer.ssm_a,
        layer.ssm_alpha,
        layer.ssm_beta,
        layer.ssm_conv1d,
        layer.ssm_dt,
        layer.ssm_norm_gamma,
        layer.ssm_norm_eps_q,
        layer.ssm_norm_post_scale,
        layer.ssm_out,
        layer.num_v_heads,
        layer.head_dim,
        layer.ssm_kernel_size,
        layer.ssm_scales,
        ctx.sigmoid_lut_bytes,
    )

    # Step 4: y = saturate(y_attn + y_ssm).
    sub = list(y_attn)
    add_residual_inplace(sub, y_ssm)

    # Step 5: residual1 = inp + sub.
    add_residual_inplace(sub, inp)
    residual1 = sub

    # Step 6: norm2.
    normed2 = apply_norm_per_token(layer.norm2, residual1, m, hidden)

    # Step 7: FFN.
    ffn_out = R.ffn_forward(
        normed2,
        layer.ffn.w_gate,
        layer.ffn.w_up,
        layer.ffn.w_down,
        ctx.ffn_activation_bytes,
        layer.ffn_scales,
        m,
        hidden,
        layer.ffn.intermediate,
    )

    # Step 8: output = residual1 + ffn_out.
    out = list(residual1)
    add_residual_inplace(out, ffn_out)
    return out


@dataclass(frozen=True)
class LayerContext:
    rope_tables: RopeTables
    softmax_lut: R.ExpLut
    sigmoid_lut_bytes: tuple[int, ...]
    ffn_activation_bytes: tuple[int, ...]


def forward_layer(
    inp: Sequence[int],
    layer,  # AttentionLayer | DeltaNetLayer | GemmaLayer | QwenStandardLayer | QwenHybridSsmLayer
    ctx: LayerContext,
    m: int,
) -> list[int]:
    if isinstance(layer, GemmaLayer):
        return forward_gemma_layer(inp, layer, ctx, m)
    if isinstance(layer, QwenStandardLayer):
        return forward_qwen_standard_layer(inp, layer, ctx, m)
    if isinstance(layer, QwenHybridSsmLayer):
        return forward_qwen_hybrid_ssm_layer(inp, layer, ctx, m)
    if isinstance(layer, AttentionLayer):
        hidden = layer.attn.hidden
    elif isinstance(layer, DeltaNetLayer):
        hidden = layer.dnet.hidden
    else:
        raise TypeError(f"unknown layer type {type(layer)}")

    normed1 = apply_norm_per_token(layer.norm1, inp, m, hidden)
    if isinstance(layer, AttentionLayer):
        sub = attention_forward(
            normed1, layer.attn, layer.attn_scales, ctx.rope_tables, ctx.softmax_lut, m
        )
    else:
        sub = deltanet_forward(
            normed1, layer.dnet, layer.dnet_scales, ctx.sigmoid_lut_bytes, m
        )

    # First residual: sub += inp.
    residual1 = list(sub)
    add_residual_inplace(residual1, inp)

    normed2 = apply_norm_per_token(layer.norm2, residual1, m, hidden)
    ffn_out = R.ffn_forward(
        normed2,
        layer.ffn.w_gate,
        layer.ffn.w_up,
        layer.ffn.w_down,
        ctx.ffn_activation_bytes,
        layer.ffn_scales,
        m,
        hidden,
        layer.ffn.intermediate,
    )

    out = list(residual1)
    add_residual_inplace(out, ffn_out)
    return out


# -----------------------------------------------------------------------------
# forward_prefix (mirrors src/forward.rs).
# -----------------------------------------------------------------------------


@dataclass(frozen=True)
class ModelDims:
    vocab: int
    hidden: int
    seq_len: int
    activation_tile: int


@dataclass(frozen=True)
class Model:
    dims: ModelDims
    embed: tuple[int, ...]
    layers: tuple
    final_norm: Optional[NormSpec]
    rope_tables: RopeTables
    softmax_lut: R.ExpLut
    sigmoid_lut_bytes: tuple[int, ...]
    ffn_activation_bytes: tuple[int, ...]


def forward_prefix(
    model: Model,
    prompt: Sequence[int],
    target_layer: int,
) -> tuple[list[int], list[list[int]]]:
    """Returns (output_for_target_layer_input, per_layer_full_seq_tensors).

    `per_layer_full_seq_tensors[i]` is the seq_len-padded `(seq_len, hidden)`
    tensor recorded as the input-to-layer-i activation (or post-final-norm
    output if i == num_layers).
    """
    m = len(prompt)
    if m == 0:
        raise ValueError("empty prompt")
    if m > model.dims.seq_len:
        raise ValueError("prompt too long")
    if target_layer > len(model.layers):
        raise ValueError("target_layer too large")

    seq_full = model.dims.seq_len
    hidden = model.dims.hidden
    hu = hidden

    # Embed into a seq_full-padded tensor.
    x_full = [0] * (seq_full * hu)
    for i, tok in enumerate(prompt):
        if tok >= model.dims.vocab:
            raise ValueError(f"token {tok} out of vocab")
        src = tok * hu
        dst = i * hu
        x_full[dst : dst + hu] = list(model.embed[src : src + hu])

    log: list[list[int]] = [list(x_full)]

    ctx = LayerContext(
        rope_tables=model.rope_tables,
        softmax_lut=model.softmax_lut,
        sigmoid_lut_bytes=model.sigmoid_lut_bytes,
        ffn_activation_bytes=model.ffn_activation_bytes,
    )

    for layer_idx in range(target_layer):
        layer = model.layers[layer_idx]
        # Run layer over the prompt prefix only.
        prefix = x_full[: m * hu]
        new_prefix = forward_layer(prefix, layer, ctx, m)
        # Update x_full: prefix replaced, padding stays zero.
        for i in range(m * hu):
            x_full[i] = new_prefix[i]
        for i in range(m * hu, seq_full * hu):
            x_full[i] = 0
        log.append(list(x_full))

    if target_layer == len(model.layers) and model.final_norm is not None:
        prefix = x_full[: m * hu]
        normed = apply_norm_per_token(model.final_norm, prefix, m, hidden)
        for i in range(m * hu):
            x_full[i] = normed[i]

    output = x_full[: m * hu]
    return output, log
