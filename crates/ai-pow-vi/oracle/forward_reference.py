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
class LayerContext:
    rope_tables: RopeTables
    softmax_lut: R.ExpLut
    sigmoid_lut_bytes: tuple[int, ...]
    ffn_activation_bytes: tuple[int, ...]


def forward_layer(
    inp: Sequence[int],
    layer,  # AttentionLayer | DeltaNetLayer
    ctx: LayerContext,
    m: int,
) -> list[int]:
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
