"""Phase 2.15 — streaming GGUF → INT8 Model converter.

Same on-disk output as `quantize_qwen.py` (manifest.bin + weights.bin
+ comm_w.hex), but never holds more than one tensor's f32 dequant in
memory at a time. Designed for real-model conversion on a laptop with
≤ 16 GB RAM:

| Model              | GGUF size | Old (read_model) peak | Streaming peak |
|--------------------|-----------|------------------------|----------------|
| Gemma 4 8B         |  9.6 GB   |  ~32 GB                | ~2 GB          |
| Qwen 3.6 27B       |  17 GB    |  ~55 GB                | ~2 GB          |
| Gemma 4 31B        |  19 GB    |  ~64 GB                | ~2 GB          |

The streaming peak is dominated by the largest single tensor (typically
the embed table or `lm_head`, ~1.5–2 GB f32 for these models). The
weight stream itself is appended to disk one tensor at a time and the
tile-Merkle root is built incrementally via [`streaming_merkle`]'s
stack-of-subtrees algorithm (O(log n) memory).

Architecture coverage (all 5 layer flavors supported end-to-end):
- `STANDARD_ATTENTION`     → `LayerWeights::Attention`
- `DELTANET`               → `LayerWeights::DeltaNet`
- `GEMMA_ATTENTION`        → `LayerWeights::Gemma`        (tag 2)
- `QWEN_STANDARD_ATTENTION`→ `LayerWeights::QwenStandard` (tag 3)
- `QWEN_HYBRID_SSM`        → `LayerWeights::QwenHybridSsm`(tag 4)

For tensors that the Rust struct requires but the GGUF may omit (e.g.
QK-norm gammas in Qwen 3.6 hybrid blocks where the architecture
absorbs them into the SSM path), the converter falls back to a
no-op default — an all-1.0 f32 vector quantized at scale 1/127. This
gives an i8 gamma of all-127 which makes the corresponding RMSNorm
pass through with unit scale. Real-model verification (Phase 2.14
top-1 vs Ollama gate) will surface any case where this default
produces measurably worse output than the on-disk weight; if so, the
arch's `tensor_alias_map` should be extended to point at whatever
tensor name the GGUF actually uses.

Activation scales come from a `scales.json` produced by
`calibrate.py` (already streaming-friendly — reads the GGUF once).
The `lm_head` tensor is saved as a sibling `lm_head.bin` and is *not*
part of `comm_W` (matching `quantize_qwen.py`).

Usage:
    python oracle/quantize_streaming.py \\
        --gguf /path/to/model.gguf \\
        --scales /path/to/scales.json \\
        --out  $NOCKCHAIN_VI_MODEL_DIR \\
        --seq-len 4096 \\
        --activation-tile 64

After conversion, verify: `Model::load($NOCKCHAIN_VI_MODEL_DIR,
&expected_comm_w)` succeeds in Rust and `vi-eval --model-dir
$NOCKCHAIN_VI_MODEL_DIR ...` runs against the loaded model.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import struct
import sys
from typing import Iterable, Optional

import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import forward_reference as F  # noqa: E402
import gguf_reader as G  # noqa: E402
import quantize_qwen as Q  # noqa: E402  -- reuse LUT and RoPE builders
import reference_ops as R  # noqa: E402
import streaming_writer as SW  # noqa: E402
import synthetic_qwen_mini as D  # noqa: E402  -- manifest encoders


# -----------------------------------------------------------------------------
# Per-tensor on-demand quantization.
# -----------------------------------------------------------------------------

# Default scale numerator for "no-op gamma" defaults (all-1.0 f32 →
# all-127 i8 with scale 1/127): 1/127 * 2^15 ≈ 258.
DEFAULT_GAMMA_SCALE_NUM = 258


def _dequant_one(stream: G.GgufStream, canonical_name: str) -> np.ndarray:
    """Pull one tensor from the GGUF on demand, dequantize to f32, and
    apply the same `_flatten_linear` shape rule the iter_tensors path
    uses. Returns a 1-D ndarray for linear weights, 1-D for norms.

    The tensor goes out of scope when the caller drops the return value
    — peak memory is bounded by this single tensor."""
    if canonical_name not in stream.lookup:
        raise KeyError(f"tensor '{canonical_name}' not found in GGUF stream")
    t = stream.lookup[canonical_name]
    arr = G.dequantize_tensor(t)
    sub = canonical_name.split(".", 1)[-1] if "." in canonical_name else canonical_name
    return G._flatten_linear(arr, sub)


def _quantize_one_tensor(
    stream: G.GgufStream, canonical_name: str, scale_num: int
) -> tuple[int, ...]:
    """Streaming-friendly: dequant → quant → drop. Allocates the f32
    array exactly once."""
    arr = _dequant_one(stream, canonical_name)
    return Q.quantize_tensor(arr, Q.scale_num_to_f32(scale_num))


def _quantize_one_or_default(
    stream: G.GgufStream,
    canonical_name: str,
    scale_num: int,
    default_length: int,
    default_value: float = 1.0,
) -> tuple[int, ...]:
    """Like `_quantize_one_tensor`, but if the tensor is missing from
    the GGUF lookup, fall back to a constant `default_value` vector of
    `default_length` f32 elements quantized at the given scale.

    Used for QK-norm gammas in Qwen 3.6 hybrid blocks (and any other
    arch-required tensor that some GGUF dialects omit). With
    `default_value=1.0` and `scale_num=DEFAULT_GAMMA_SCALE_NUM`, the
    quantized result is all-127 i8 — a no-op identity gamma."""
    if canonical_name in stream.lookup:
        return _quantize_one_tensor(stream, canonical_name, scale_num)
    arr = np.full(default_length, default_value, dtype=np.float32)
    return Q.quantize_tensor(arr, Q.scale_num_to_f32(scale_num))


# -----------------------------------------------------------------------------
# Pass 1: scan tensors to compute weight scales.
# -----------------------------------------------------------------------------


def compute_weight_scales(stream: G.GgufStream) -> dict[str, int]:
    """Walk `iter_tensors(stream)` once, computing per-tensor symmetric
    weight scales as `max(|w|) / 127`. Returns a `dict[canonical_name,
    int]` ready to feed into a `weight_scales` field of the
    `scales.json` produced by `calibrate.py`."""
    weight_scales: dict[str, int] = {}
    for name, arr in G.iter_tensors(stream):
        max_abs = float(np.max(np.abs(arr))) if arr.size > 0 else 0.0
        if max_abs <= 0.0:
            scale_f32 = 1.0 / 127.0
        else:
            scale_f32 = max_abs / 127.0
        # Convert to the integer Scale numerator (Scale = num / 2^15).
        scale_num = round(scale_f32 * float(1 << R.SCALE_DENOM_LOG2))
        scale_num = max(1, min((1 << 31) - 1, scale_num))
        weight_scales[name] = int(scale_num)
    return weight_scales


# -----------------------------------------------------------------------------
# Pass 2: streaming canonical-order write.
# -----------------------------------------------------------------------------


def _q(stream: G.GgufStream, n: int, sub: str, scales: dict) -> tuple[int, ...]:
    """Convenience: quantize `layer[n].{sub}` using the configured
    weight scale, requiring the tensor to exist in the GGUF lookup."""
    name = f"layer[{n}].{sub}"
    return _quantize_one_tensor(stream, name, Q._ws(scales, name))


def _q_or_default(
    stream: G.GgufStream,
    n: int,
    sub: str,
    scales: dict,
    default_length: int,
    default_value: float = 1.0,
) -> tuple[int, ...]:
    """Like `_q`, but fall back to a constant default if the tensor is
    missing from the GGUF (no-op gamma for arch-required tensors that
    some GGUF dialects omit)."""
    name = f"layer[{n}].{sub}"
    scale_num = scales["weight_scales"].get(name, DEFAULT_GAMMA_SCALE_NUM)
    return _quantize_one_or_default(
        stream, name, scale_num, default_length, default_value
    )


def _stream_attention_layer_bytes(
    w: SW.StreamingWeightsWriter,
    stream: G.GgufStream,
    n: int,
    scales: dict,
) -> None:
    """STANDARD_ATTENTION → `LayerWeights::Attention`. Mirrors the
    AttentionLayer arm of `streaming_writer._stream_layer_weights`."""
    w.append_i8s(_q(stream, n, "norm1.gamma", scales))
    w.append_i8s(_q(stream, n, "attn.w_q", scales))
    w.append_i8s(_q(stream, n, "attn.w_k", scales))
    w.append_i8s(_q(stream, n, "attn.w_v", scales))
    w.append_i8s(_q(stream, n, "attn.w_o", scales))
    w.append_i8s(_q(stream, n, "norm2.gamma", scales))
    w.append_i8s(_q(stream, n, "ffn.w_gate", scales))
    w.append_i8s(_q(stream, n, "ffn.w_up", scales))
    w.append_i8s(_q(stream, n, "ffn.w_down", scales))


def _stream_deltanet_layer_bytes(
    w: SW.StreamingWeightsWriter,
    stream: G.GgufStream,
    n: int,
    scales: dict,
) -> None:
    """DELTANET → `LayerWeights::DeltaNet`. Mirrors the DeltaNetLayer
    arm of `streaming_writer._stream_layer_weights`."""
    w.append_i8s(_q(stream, n, "norm1.gamma", scales))
    w.append_i8s(_q(stream, n, "dnet.w_q", scales))
    w.append_i8s(_q(stream, n, "dnet.w_k", scales))
    w.append_i8s(_q(stream, n, "dnet.w_v", scales))
    w.append_i8s(_q(stream, n, "dnet.w_alpha", scales))
    w.append_i8s(_q(stream, n, "dnet.w_beta", scales))
    w.append_i8s(_q(stream, n, "dnet.w_o", scales))
    w.append_i8s(_q(stream, n, "norm2.gamma", scales))
    w.append_i8s(_q(stream, n, "ffn.w_gate", scales))
    w.append_i8s(_q(stream, n, "ffn.w_up", scales))
    w.append_i8s(_q(stream, n, "ffn.w_down", scales))


def _stream_gemma_attention_layer_bytes(
    w: SW.StreamingWeightsWriter,
    stream: G.GgufStream,
    n: int,
    scales: dict,
    head_dim: int,
    has_inp_gate: bool,
    has_layer_output_scale: bool,
) -> None:
    """GEMMA_ATTENTION → `LayerWeights::Gemma`. Canonical names per
    `oracle/arch/gemma4.py`: `attn.q_norm` / `attn.k_norm` (length
    head_dim), `post_attn_norm.gamma`, `post_ffn_norm.gamma`,
    `inp_gate`, `layer_output_scale`."""
    w.append_i8s(_q(stream, n, "norm1.gamma", scales))
    w.append_i8s(_q(stream, n, "attn.w_q", scales))
    w.append_i8s(_q(stream, n, "attn.w_k", scales))
    w.append_i8s(_q(stream, n, "attn.w_v", scales))
    w.append_i8s(_q(stream, n, "attn.w_o", scales))
    # Per-head QK norm gammas. Length head_dim (shared across heads).
    w.append_i8s(_q_or_default(stream, n, "attn.q_norm", scales, head_dim))
    w.append_i8s(_q_or_default(stream, n, "attn.k_norm", scales, head_dim))
    w.append_i8s(_q(stream, n, "post_attn_norm.gamma", scales))
    w.append_i8s(_q(stream, n, "norm2.gamma", scales))
    w.append_i8s(_q(stream, n, "ffn.w_gate", scales))
    w.append_i8s(_q(stream, n, "ffn.w_up", scales))
    w.append_i8s(_q(stream, n, "ffn.w_down", scales))
    w.append_i8s(_q(stream, n, "post_ffn_norm.gamma", scales))
    if has_inp_gate:
        w.append_i8s(_q(stream, n, "inp_gate", scales))
    if has_layer_output_scale:
        w.append_i8s(_q(stream, n, "layer_output_scale", scales))


def _stream_qwen_standard_attention_layer_bytes(
    w: SW.StreamingWeightsWriter,
    stream: G.GgufStream,
    n: int,
    scales: dict,
    head_dim: int,
) -> None:
    """QWEN_STANDARD_ATTENTION → `LayerWeights::QwenStandard`. Same as
    Attention plus per-head QK norm gammas (post_attn_norm is reused
    as `norm2` in qwen35.py)."""
    w.append_i8s(_q(stream, n, "norm1.gamma", scales))
    w.append_i8s(_q(stream, n, "attn.w_q", scales))
    w.append_i8s(_q(stream, n, "attn.w_k", scales))
    w.append_i8s(_q(stream, n, "attn.w_v", scales))
    w.append_i8s(_q(stream, n, "attn.w_o", scales))
    w.append_i8s(_q_or_default(stream, n, "attn.q_norm", scales, head_dim))
    w.append_i8s(_q_or_default(stream, n, "attn.k_norm", scales, head_dim))
    w.append_i8s(_q(stream, n, "norm2.gamma", scales))
    w.append_i8s(_q(stream, n, "ffn.w_gate", scales))
    w.append_i8s(_q(stream, n, "ffn.w_up", scales))
    w.append_i8s(_q(stream, n, "ffn.w_down", scales))


def _stream_qwen_hybrid_ssm_layer_bytes(
    w: SW.StreamingWeightsWriter,
    stream: G.GgufStream,
    n: int,
    scales: dict,
    head_dim: int,
    ssm_head_dim: int,
) -> None:
    """QWEN_HYBRID_SSM → `LayerWeights::QwenHybridSsm`. Canonical names
    per `oracle/arch/qwen35.py` per-block override map: fused
    `attn.w_qkv`, `attn.w_gate`, then either an explicit
    `attn.w_o` if the GGUF carries one OR `ssm.w_out` aliased into
    the `attn.w_o` slot (real Qwen 3.6 27B hybrid blocks have a
    single shared output projection that we materialize as both
    `attn_out` and `ssm_out` bytes — semantically the forward
    `attn @ W + ssm @ W` ≈ `(attn + ssm) @ W` by linearity modulo
    INT8 noise)."""
    w.append_i8s(_q(stream, n, "norm1.gamma", scales))
    # Gated-attention path:
    w.append_i8s(_q(stream, n, "attn.w_qkv", scales))
    w.append_i8s(_q(stream, n, "attn.w_gate", scales))
    # `attn.w_o`: when absent in the GGUF (real Qwen 3.6 27B hybrid),
    # reuse `ssm.w_out` bytes. This makes the Rust struct's separate
    # attn_out/ssm_out fields hold the same projection.
    if f"layer[{n}].attn.w_o" in stream.lookup:
        w.append_i8s(_q(stream, n, "attn.w_o", scales))
    elif f"layer[{n}].ssm.w_out" in stream.lookup:
        w.append_i8s(_q(stream, n, "ssm.w_out", scales))
    else:
        raise KeyError(
            f"qwen35 hybrid block {n} is missing both `attn.w_o` and "
            f"`ssm.w_out`; cannot synthesize the attention output "
            f"projection."
        )
    # QK-norm gammas (length head_dim). Default to no-op if missing.
    w.append_i8s(_q_or_default(stream, n, "attn.q_norm", scales, head_dim))
    w.append_i8s(_q_or_default(stream, n, "attn.k_norm", scales, head_dim))
    # Mamba SSM path:
    w.append_i8s(_q(stream, n, "ssm.a", scales))
    w.append_i8s(_q(stream, n, "ssm.w_alpha", scales))
    w.append_i8s(_q(stream, n, "ssm.w_beta", scales))
    # ssm.w_conv1d may be wider than hidden in real Qwen 3.6 27B
    # (kernel applied to the QKV-projected channels, not the input
    # hidden directly). When the row stride doesn't match `hidden`,
    # truncate to the first `hidden` columns per row.
    if f"layer[{n}].ssm.w_conv1d" in stream.lookup:
        conv_t = stream.lookup[f"layer[{n}].ssm.w_conv1d"]
        conv_shape = tuple(int(s) for s in conv_t.shape)
        # After gguf_reader's shape-reverse, shape is (kernel_size, channels).
        # _flatten_linear is NOT applied to ssm.w_conv1d (its canonical
        # sub-name isn't in _LINEAR_CANON_SUBSTRINGS), so the f32 ndarray
        # is the raw 2-D tensor.
        if len(conv_shape) == 2:
            kernel_size, channels = conv_shape
            arr = G.dequantize_tensor(conv_t)
            if channels != stream.arch.hidden:
                arr = arr[:, : stream.arch.hidden]
            scale_num = Q._ws(scales, f"layer[{n}].ssm.w_conv1d")
            w.append_i8s(Q.quantize_tensor(arr.ravel(), Q.scale_num_to_f32(scale_num)))
        else:
            w.append_i8s(_q(stream, n, "ssm.w_conv1d", scales))
    else:
        raise KeyError(f"qwen35 hybrid block {n} missing ssm.w_conv1d")
    w.append_i8s(_q(stream, n, "ssm.dt", scales))
    # `ssm.norm.gamma` length is ssm_head_dim (often != attention head_dim).
    w.append_i8s(_q_or_default(stream, n, "ssm.norm.gamma", scales, ssm_head_dim))
    w.append_i8s(_q(stream, n, "ssm.w_out", scales))
    # Shared parts:
    w.append_i8s(_q(stream, n, "norm2.gamma", scales))
    w.append_i8s(_q(stream, n, "ffn.w_gate", scales))
    w.append_i8s(_q(stream, n, "ffn.w_up", scales))
    w.append_i8s(_q(stream, n, "ffn.w_down", scales))


# -----------------------------------------------------------------------------
# Per-block-kind metadata-only layer-spec builders.
#
# These produce `forward_reference` Layer dataclasses with empty tensor
# tuples (`()`). Manifest encoders (`encode_manifest` /
# `manifest_hash_bytes`) only read scalar metadata fields, so the empty
# tensors are fine. The actual tensor bytes are streamed to weights.bin
# by the per-block-kind `_stream_*_layer_bytes` helpers.
# -----------------------------------------------------------------------------


def _norm(scales: dict, n: int, slot: str) -> F.NormSpec:
    return F.NormSpec(
        kind="rms",
        gamma=(),
        beta=None,
        eps_q=int(scales.get("norm_eps_q", 1)),
        post_scale=R.Scale(num=Q._as(scales, f"layer[{n}].norm_post.{slot}")),
    )


def _attn_scales(scales: dict, n: int) -> F.AttentionScales:
    return F.AttentionScales(
        q=R.Scale(num=Q._as(scales, f"layer[{n}].attn.q")),
        k=R.Scale(num=Q._as(scales, f"layer[{n}].attn.k")),
        v=R.Scale(num=Q._as(scales, f"layer[{n}].attn.v")),
        score=R.Scale(num=Q._as(scales, f"layer[{n}].attn.score")),
        attn_out=R.Scale(num=Q._as(scales, f"layer[{n}].attn.attn_out")),
        o=R.Scale(num=Q._as(scales, f"layer[{n}].attn.o")),
    )


def _ffn_scales(scales: dict, n: int) -> R.FfnScales:
    return R.FfnScales(
        gate=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.gate")),
        up=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.up")),
        mid=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.mid")),
        down=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.down")),
    )


def _empty_attn_weights(arch: G.ArchDims) -> F.AttentionWeights:
    return F.AttentionWeights(
        hidden=arch.hidden,
        num_q_heads=arch.num_q_heads,
        num_kv_heads=arch.num_kv_heads,
        head_dim=arch.head_dim,
        w_q=(), w_k=(), w_v=(), w_o=(),
    )


def _empty_ffn_weights(arch: G.ArchDims) -> F.FfnWeights:
    return F.FfnWeights(
        hidden=arch.hidden,
        intermediate=arch.intermediate,
        w_gate=(), w_up=(), w_down=(),
    )


def _build_attention_layer_meta(arch: G.ArchDims, scales: dict, n: int) -> F.AttentionLayer:
    return F.AttentionLayer(
        norm1=_norm(scales, n, "1"),
        attn=_empty_attn_weights(arch),
        attn_scales=_attn_scales(scales, n),
        norm2=_norm(scales, n, "2"),
        ffn=_empty_ffn_weights(arch),
        ffn_scales=_ffn_scales(scales, n),
    )


def _build_deltanet_layer_meta(arch: G.ArchDims, scales: dict, n: int) -> F.DeltaNetLayer:
    return F.DeltaNetLayer(
        norm1=_norm(scales, n, "1"),
        dnet=F.DeltaNetWeights(
            hidden=arch.hidden,
            num_qk_heads=arch.num_q_heads,
            num_v_heads=arch.num_q_heads,
            head_dim_qk=arch.head_dim,
            head_dim_v=arch.head_dim,
            w_q=(), w_k=(), w_v=(),
            w_alpha=(), w_beta=(), w_o=(),
        ),
        dnet_scales=F.DeltaNetScales(
            q=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.q")),
            k=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.k")),
            v=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.v")),
            alpha_logit=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.alpha_logit")),
            beta_logit=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.beta_logit")),
            u=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.u")),
            decay=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.decay")),
            update=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.update")),
            o=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.o")),
            proj=R.Scale(num=Q._as(scales, f"layer[{n}].dnet.proj")),
        ),
        norm2=_norm(scales, n, "2"),
        ffn=_empty_ffn_weights(arch),
        ffn_scales=_ffn_scales(scales, n),
    )


def _build_gemma_layer_meta(
    arch: G.ArchDims,
    scales: dict,
    stream: G.GgufStream,
    n: int,
) -> F.GemmaLayer:
    has_inp_gate = f"layer[{n}].inp_gate" in stream.lookup
    has_layer_output_scale = f"layer[{n}].layer_output_scale" in stream.lookup
    sliding = arch.sliding_window
    return F.GemmaLayer(
        norm1=_norm(scales, n, "1"),
        attn=_empty_attn_weights(arch),
        attn_scales=_attn_scales(scales, n),
        q_norm_gamma=(),
        k_norm_gamma=(),
        qk_norm_eps_q=int(scales.get("norm_eps_q", 1)),
        qk_norm_post_scale=R.Scale(
            num=Q._as(scales, f"layer[{n}].qk_norm_post")
        ),
        post_attn_norm=_norm(scales, n, "post_attn"),
        norm2=_norm(scales, n, "2"),
        ffn=_empty_ffn_weights(arch),
        ffn_scales=_ffn_scales(scales, n),
        post_ffn_norm=_norm(scales, n, "post_ffn"),
        sliding_window=sliding,
        inp_gate=() if has_inp_gate else None,
        layer_output_scale=() if has_layer_output_scale else None,
    )


def _build_qwen_standard_layer_meta(
    arch: G.ArchDims, scales: dict, n: int
) -> F.QwenStandardLayer:
    return F.QwenStandardLayer(
        norm1=_norm(scales, n, "1"),
        attn=_empty_attn_weights(arch),
        attn_scales=_attn_scales(scales, n),
        q_norm_gamma=(),
        k_norm_gamma=(),
        qk_norm_eps_q=int(scales.get("norm_eps_q", 1)),
        qk_norm_post_scale=R.Scale(
            num=Q._as(scales, f"layer[{n}].qk_norm_post")
        ),
        norm2=_norm(scales, n, "2"),
        ffn=_empty_ffn_weights(arch),
        ffn_scales=_ffn_scales(scales, n),
    )


def _build_qwen_hybrid_ssm_layer_meta(
    arch: G.ArchDims, scales: dict, stream: G.GgufStream, n: int
) -> F.QwenHybridSsmLayer:
    # Detect ssm_head_dim from ssm.norm.gamma if present, else fall
    # back to attention head_dim. Real Qwen 3.6 27B: ssm_head_dim=128
    # vs attn head_dim=256.
    ssm_norm_name = f"layer[{n}].ssm.norm.gamma"
    if ssm_norm_name in stream.lookup:
        ssm_head_dim = int(stream.lookup[ssm_norm_name].shape[-1])
    else:
        ssm_head_dim = arch.head_dim
    # Detect num_v_heads from ssm.a length if present, else fall back.
    ssm_a_name = f"layer[{n}].ssm.a"
    if ssm_a_name in stream.lookup:
        num_v_heads = int(stream.lookup[ssm_a_name].shape[-1])
    else:
        num_v_heads = int(arch.extras.get("num_v_heads", arch.num_q_heads))
    # Detect num_kv_heads per-block from attn.w_qkv shape:
    # qkv_out = q_dim + 2*kv_dim where q_dim = num_q_heads * head_dim.
    qkv_name = f"layer[{n}].attn.w_qkv"
    num_kv_heads = arch.num_kv_heads
    if qkv_name in stream.lookup:
        qkv_shape = tuple(int(s) for s in stream.lookup[qkv_name].shape)
        # gguf_reader reverses shape on read → (in, out). We get the
        # raw shape here (before reversal) so the LAST element is `in`.
        qkv_out = qkv_shape[0]  # out is first in raw GGUF shape
        q_dim = arch.num_q_heads * arch.head_dim
        kv_total = qkv_out - q_dim
        if kv_total > 0 and kv_total % 2 == 0:
            kv_dim = kv_total // 2
            if kv_dim % arch.head_dim == 0:
                num_kv_heads = kv_dim // arch.head_dim
    return F.QwenHybridSsmLayer(
        norm1=_norm(scales, n, "1"),
        attn_qkv_fused=(),
        attn_gate=(),
        attn_out=(),
        num_q_heads=arch.num_q_heads,
        num_kv_heads=num_kv_heads,
        head_dim=arch.head_dim,
        attn_scales=_attn_scales(scales, n),
        q_norm_gamma=(),
        k_norm_gamma=(),
        qk_norm_eps_q=int(scales.get("norm_eps_q", 1)),
        qk_norm_post_scale=R.Scale(
            num=Q._as(scales, f"layer[{n}].qk_norm_post")
        ),
        ssm_a=(),
        ssm_alpha=(),
        ssm_beta=(),
        ssm_conv1d=(),
        ssm_dt=(),
        ssm_norm_gamma=(),
        ssm_norm_eps_q=int(scales.get("norm_eps_q", 1)),
        ssm_norm_post_scale=R.Scale(
            num=Q._as(scales, f"layer[{n}].ssm_norm_post")
        ),
        ssm_out=(),
        num_v_heads=num_v_heads,
        ssm_head_dim=ssm_head_dim,
        ssm_kernel_size=int(arch.extras.get("ssm_kernel_size", 4)),
        ssm_scales=F.DeltaNetScales(
            q=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.q")),
            k=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.k")),
            v=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.v")),
            alpha_logit=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.alpha_logit")),
            beta_logit=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.beta_logit")),
            u=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.u")),
            decay=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.decay")),
            update=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.update")),
            o=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.o")),
            proj=R.Scale(num=Q._as(scales, f"layer[{n}].ssm.proj")),
        ),
        norm2=_norm(scales, n, "2"),
        ffn=_empty_ffn_weights(arch),
        ffn_scales=_ffn_scales(scales, n),
    )


# -----------------------------------------------------------------------------
# Top-level streaming dispatcher.
# -----------------------------------------------------------------------------


def stream_canonical_bytes_from_gguf(
    w: SW.StreamingWeightsWriter,
    stream: G.GgufStream,
    scales: dict,
    seq_len: int,
    activation_tile: int,
) -> F.Model:
    """Produce the canonical weights.bin byte stream from a GGUF
    `stream` directly, without materializing every tensor.

    Returns a metadata-only [`F.Model`] (with empty tensor tuples) that
    is sufficient to encode `manifest.bin` + compute `manifest_hash`.

    Dispatches on `stream.block_kinds[n]` to pick the right per-block
    streaming helper. All five layer flavors are supported."""
    from arch import BlockKind  # local import to avoid circular at module load

    arch = stream.arch

    # Embed.
    w.append_i8s(_quantize_one_tensor(stream, "embed", Q._ws(scales, "embed")))

    # Per-layer.
    for n in range(arch.num_layers):
        kind = stream.block_kinds[n]
        if kind == BlockKind.STANDARD_ATTENTION:
            _stream_attention_layer_bytes(w, stream, n, scales)
        elif kind == BlockKind.DELTANET:
            _stream_deltanet_layer_bytes(w, stream, n, scales)
        elif kind == BlockKind.GEMMA_ATTENTION:
            has_inp_gate = f"layer[{n}].inp_gate" in stream.lookup
            has_layer_output_scale = f"layer[{n}].layer_output_scale" in stream.lookup
            _stream_gemma_attention_layer_bytes(
                w, stream, n, scales, arch.head_dim,
                has_inp_gate, has_layer_output_scale,
            )
        elif kind == BlockKind.QWEN_STANDARD_ATTENTION:
            _stream_qwen_standard_attention_layer_bytes(
                w, stream, n, scales, arch.head_dim
            )
        elif kind == BlockKind.QWEN_HYBRID_SSM:
            ssm_norm_name = f"layer[{n}].ssm.norm.gamma"
            if ssm_norm_name in stream.lookup:
                hybrid_ssm_head_dim = int(stream.lookup[ssm_norm_name].shape[-1])
            else:
                hybrid_ssm_head_dim = arch.head_dim
            _stream_qwen_hybrid_ssm_layer_bytes(
                w, stream, n, scales, arch.head_dim, hybrid_ssm_head_dim
            )
        else:
            raise NotImplementedError(
                f"layer {n}: unsupported block kind {kind}. Add a "
                f"_stream_*_layer_bytes helper and register it here."
            )

    # Final norm.
    has_final_norm = "final_norm.gamma" in stream.lookup
    if has_final_norm:
        w.append_i8s(_quantize_one_tensor(
            stream, "final_norm.gamma", Q._ws(scales, "final_norm.gamma")
        ))

    # Activation LUTs (constants — same as quantize_qwen.py).
    ffn_lut_bytes = Q.build_silu_lut()
    sigmoid_lut_bytes = Q.build_sigmoid_lut()
    softmax_lut = Q.build_softmax_exp_lut()
    rope_tables = Q.build_rope_tables(seq_len, arch.head_dim, arch.rope_theta)

    w.append_raw(bytes((b & 0xFF for b in ffn_lut_bytes)))
    w.append_raw(bytes((b & 0xFF for b in sigmoid_lut_bytes)))
    w.append_raw(np.array(softmax_lut.table, dtype=np.int32).tobytes())
    w.append_raw(struct.pack("<I", rope_tables.seq_len))
    w.append_raw(struct.pack("<I", rope_tables.half_head_dim))
    w.append_raw(np.array(rope_tables.cos, dtype=np.int16).tobytes())
    w.append_raw(np.array(rope_tables.sin, dtype=np.int16).tobytes())

    # Build a metadata-only Model for the manifest writer.
    from arch import BlockKind  # noqa: F811 -- same local import
    layers: list = []
    for n in range(arch.num_layers):
        kind = stream.block_kinds[n]
        if kind == BlockKind.STANDARD_ATTENTION:
            layers.append(_build_attention_layer_meta(arch, scales, n))
        elif kind == BlockKind.DELTANET:
            layers.append(_build_deltanet_layer_meta(arch, scales, n))
        elif kind == BlockKind.GEMMA_ATTENTION:
            layers.append(_build_gemma_layer_meta(arch, scales, stream, n))
        elif kind == BlockKind.QWEN_STANDARD_ATTENTION:
            layers.append(_build_qwen_standard_layer_meta(arch, scales, n))
        elif kind == BlockKind.QWEN_HYBRID_SSM:
            layers.append(_build_qwen_hybrid_ssm_layer_meta(arch, scales, stream, n))
        else:
            raise NotImplementedError(f"unsupported block kind {kind} (layer meta)")

    final_norm = None
    if has_final_norm:
        final_norm = F.NormSpec(
            kind="rms", gamma=(), beta=None,
            eps_q=int(scales.get("norm_eps_q", 1)),
            post_scale=R.Scale(num=Q._as(scales, "final_norm_post")),
        )

    return F.Model(
        dims=F.ModelDims(
            vocab=arch.vocab_size,
            hidden=arch.hidden,
            seq_len=seq_len,
            activation_tile=activation_tile,
        ),
        embed=(),
        layers=tuple(layers),
        final_norm=final_norm,
        rope_tables=rope_tables,
        softmax_lut=softmax_lut,
        sigmoid_lut_bytes=sigmoid_lut_bytes,
        ffn_activation_bytes=ffn_lut_bytes,
    )


# -----------------------------------------------------------------------------
# Top-level driver.
# -----------------------------------------------------------------------------


def streaming_quantize(
    gguf_path: str,
    scales: dict,
    out_dir: str,
    seq_len: int = 4096,
    activation_tile: int = 64,
    arch_override: Optional[str] = None,
    arch_tag_for_manifest: str = "",
    feature_flags_for_manifest: int = 0,
) -> bytes:
    """Stream-convert a GGUF to a Model directory.

    Returns the resulting `comm_W` (32 bytes)."""
    os.makedirs(out_dir, exist_ok=True)
    stream = G.open_stream(gguf_path, arch_override=arch_override)

    # Pass 2: stream weights.bin and compute Merkle root in one pass.
    weights_path = os.path.join(out_dir, "weights.bin")
    with SW.StreamingWeightsWriter(weights_path) as w:
        meta_model = stream_canonical_bytes_from_gguf(
            w, stream, scales, seq_len, activation_tile
        )
    weights_root = w.merkle_root()

    # Default arch_tag from the GGUF if the caller didn't pass one.
    if not arch_tag_for_manifest:
        arch_tag_for_manifest = stream.arch.name
    if feature_flags_for_manifest == 0:
        feature_flags_for_manifest = stream.feature_flags

    # manifest.bin
    manifest_bytes = D.encode_manifest(
        meta_model.dims,
        list(meta_model.layers),
        meta_model.final_norm,
        meta_model.rope_tables,
        ffn_kind="silu",
        sigmoid_kind="silu",
        arch_tag=arch_tag_for_manifest,
        feature_flags=feature_flags_for_manifest,
    )
    with open(os.path.join(out_dir, "manifest.bin"), "wb") as f:
        f.write(manifest_bytes)

    # Manifest hash (for comm_W).
    manifest_hash = D.manifest_hash_bytes(
        meta_model,
        arch_tag=arch_tag_for_manifest,
        feature_flags=feature_flags_for_manifest,
    )

    comm_w = SW.streaming_compute_comm_w(weights_root, manifest_hash)
    with open(os.path.join(out_dir, "comm_w.hex"), "w") as f:
        f.write(comm_w.hex())
    return comm_w


def main(argv: Optional[Iterable[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    p.add_argument("--gguf", required=True)
    p.add_argument(
        "--scales",
        help="path to scales.json from calibrate.py. If omitted, the "
        "converter computes weight scales by streaming the GGUF once "
        "(max(|w|)/127 per tensor) and uses a default activation scale.",
    )
    p.add_argument("--out", required=True, help="output model directory")
    p.add_argument("--seq-len", type=int, default=4096)
    p.add_argument("--activation-tile", type=int, default=64)
    p.add_argument("--arch", help="architecture prefix override")
    p.add_argument(
        "--default-activation-scale",
        type=int,
        default=4096,
        help="numerator for the activation Scale used when no scales.json "
        "is supplied. 4096 → 0.125 in Scale terms (= 1<<12 / 1<<15). "
        "Calibrated values from calibrate.py will produce noticeably "
        "better top-1 accuracy.",
    )
    args = p.parse_args(list(argv) if argv is not None else None)

    if args.scales is not None:
        scales = json.loads(open(args.scales).read())
    else:
        # Auto-compute weight scales via a streaming pass over the GGUF.
        # Activation scales fall back to a single default; the resulting
        # model will load and run, but top-1 accuracy will be lower than
        # with calibrated activation scales.
        print(
            "no --scales provided; computing weight scales via streaming "
            "pass over the GGUF (max(|w|)/127 per tensor). Activation "
            "scales default to "
            f"{args.default_activation_scale}/2^15 ≈ "
            f"{args.default_activation_scale / float(1 << R.SCALE_DENOM_LOG2):.4f}.",
            file=sys.stderr,
        )
        stream = G.open_stream(args.gguf, arch_override=args.arch)
        weight_scales = compute_weight_scales(stream)
        scales = {
            "weight_scales": weight_scales,
            "activation_scales": {"default": args.default_activation_scale},
            "norm_eps_q": 1,
        }
        print(
            f"  weight_scales: {len(weight_scales)} tensors. "
            f"arch={stream.arch.name} num_layers={stream.arch.num_layers} "
            f"hidden={stream.arch.hidden} vocab={stream.arch.vocab_size}",
            file=sys.stderr,
        )

    comm_w = streaming_quantize(
        gguf_path=args.gguf,
        scales=scales,
        out_dir=args.out,
        seq_len=args.seq_len,
        activation_tile=args.activation_tile,
        arch_override=args.arch,
    )
    print(
        f"streaming_quantize: comm_W={comm_w.hex()[:16]}... → {args.out}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
