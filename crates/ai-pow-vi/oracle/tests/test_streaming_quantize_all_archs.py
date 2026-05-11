"""Phase 2.15 — multi-arch parity test for the streaming converter.

For each layer flavor (Attention, DeltaNet, Gemma, QwenStandard,
QwenHybridSsm), build a synthetic `forward_reference` layer and run
the corresponding per-block-kind streaming helper through a mock
`GgufStream`. Verify that the bytes the helper emits to a
`StreamingWeightsWriter` match the bytes
`streaming_writer._stream_layer_weights` emits for the same layer.

This proves the streaming GGUF converter handles all five canonical
layer kinds byte-equivalently to the reference Python writer, which
the existing `test_streaming_writer.py` already proved is byte-equal
to the materialized `synthetic_qwen_mini.encode_weights` reference.

The mock GgufStream wraps a `dict[canonical_name, np.ndarray f32]` and
exposes the same `.lookup` interface the streaming converter expects.
We pre-cast each layer's i8 tensor to f32 (1:1 — no rescaling) and use
weight scale 1.0 (`scale_num = 1 << 15 = 32768`) so the
quantize round-trip is exact and byte-equality holds.
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass
from typing import Iterable

import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ORACLE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.insert(0, ORACLE_DIR)
sys.path.insert(0, SCRIPT_DIR)

import forward_reference as F  # noqa: E402
import quantize_streaming as QS  # noqa: E402
import reference_ops as R  # noqa: E402
import streaming_writer as SW  # noqa: E402
import synthetic_gemma_mini as GM  # noqa: E402
import synthetic_qwen_hybrid_mini as HM  # noqa: E402
import synthetic_qwen_mini as D  # noqa: E402

# Use scale = 1.0 (num = 32768) so f32 → i8 round-trip is identity for
# integer-valued f32 inputs in [-128, 127].
IDENTITY_SCALE_NUM = 1 << R.SCALE_DENOM_LOG2  # 32768


# -----------------------------------------------------------------------------
# Mock GgufStream for testing.
# -----------------------------------------------------------------------------


@dataclass
class _FakeReaderTensor:
    """Stand-in for `gguf.ReaderTensor`. Only `.shape` and a callable
    that produces the f32 ndarray are needed by our streaming
    converter (via `dequantize_tensor`)."""

    name: str
    arr: np.ndarray  # already f32

    @property
    def shape(self):
        return tuple(self.arr.shape)


class MockGgufStream:
    """In-memory substitute for `gguf_reader.GgufStream`.

    Built from a `dict[canonical_name, np.ndarray f32]`. Honors the
    interface the streaming converter calls:
      - `.lookup[name]` → fake reader tensor (with `.shape`)
      - `.arch` → an ArchDims-like with `hidden`, `num_q_heads`, etc.
      - `.block_kinds` → list[BlockKind]

    Monkey-patches `gguf_reader.dequantize_tensor` for the lifetime of
    the test to return our pre-built f32 ndarray (instead of trying to
    decode GGUF-quantized bytes)."""

    def __init__(self, arch, block_kinds, tensors_f32: dict[str, np.ndarray]):
        self.arch = arch
        self.block_kinds = block_kinds
        self.feature_flags = 0
        self.lookup = {
            name: _FakeReaderTensor(name=name, arr=arr)
            for name, arr in tensors_f32.items()
        }

    def __enter__(self) -> "MockGgufStream":
        # Patch dequantize_tensor + _flatten_linear's interaction.
        import gguf_reader as G
        self._saved_dequant = G.dequantize_tensor

        def fake_dequant(t):
            # `t` is our `_FakeReaderTensor`. Return its f32 ndarray.
            return t.arr

        G.dequantize_tensor = fake_dequant
        return self

    def __exit__(self, *args):
        import gguf_reader as G
        G.dequantize_tensor = self._saved_dequant


# -----------------------------------------------------------------------------
# Helpers to convert a synthetic `forward_reference.Model` layer into a
# (canonical_name → f32 ndarray) dict and a `scales` dict that round-
# trips through quantize_tensor.
# -----------------------------------------------------------------------------


def _i8_tuple_to_f32(t: tuple[int, ...]) -> np.ndarray:
    return np.asarray(t, dtype=np.int8).astype(np.float32)


def _gather_layer_tensors(layer, n: int) -> dict[str, np.ndarray]:
    """Return a {canonical_name → f32 ndarray} dict for a single layer
    in the same convention `gguf_reader` produces."""
    out: dict[str, np.ndarray] = {}

    def add(sub: str, vals: tuple[int, ...] | None):
        if vals is None or len(vals) == 0:
            return
        out[f"layer[{n}].{sub}"] = _i8_tuple_to_f32(vals)

    add("norm1.gamma", layer.norm1.gamma)

    if isinstance(layer, F.AttentionLayer):
        add("attn.w_q", layer.attn.w_q)
        add("attn.w_k", layer.attn.w_k)
        add("attn.w_v", layer.attn.w_v)
        add("attn.w_o", layer.attn.w_o)
        add("norm2.gamma", layer.norm2.gamma)
        add("ffn.w_gate", layer.ffn.w_gate)
        add("ffn.w_up", layer.ffn.w_up)
        add("ffn.w_down", layer.ffn.w_down)
    elif isinstance(layer, F.DeltaNetLayer):
        add("dnet.w_q", layer.dnet.w_q)
        add("dnet.w_k", layer.dnet.w_k)
        add("dnet.w_v", layer.dnet.w_v)
        add("dnet.w_alpha", layer.dnet.w_alpha)
        add("dnet.w_beta", layer.dnet.w_beta)
        add("dnet.w_o", layer.dnet.w_o)
        add("norm2.gamma", layer.norm2.gamma)
        add("ffn.w_gate", layer.ffn.w_gate)
        add("ffn.w_up", layer.ffn.w_up)
        add("ffn.w_down", layer.ffn.w_down)
    elif isinstance(layer, F.GemmaLayer):
        add("attn.w_q", layer.attn.w_q)
        add("attn.w_k", layer.attn.w_k)
        add("attn.w_v", layer.attn.w_v)
        add("attn.w_o", layer.attn.w_o)
        add("attn.q_norm", layer.q_norm_gamma)
        add("attn.k_norm", layer.k_norm_gamma)
        add("post_attn_norm.gamma", layer.post_attn_norm.gamma)
        add("norm2.gamma", layer.norm2.gamma)
        add("ffn.w_gate", layer.ffn.w_gate)
        add("ffn.w_up", layer.ffn.w_up)
        add("ffn.w_down", layer.ffn.w_down)
        add("post_ffn_norm.gamma", layer.post_ffn_norm.gamma)
        if layer.inp_gate is not None:
            add("inp_gate", layer.inp_gate)
        if layer.layer_output_scale is not None:
            add("layer_output_scale", layer.layer_output_scale)
    elif isinstance(layer, F.QwenStandardLayer):
        add("attn.w_q", layer.attn.w_q)
        add("attn.w_k", layer.attn.w_k)
        add("attn.w_v", layer.attn.w_v)
        add("attn.w_o", layer.attn.w_o)
        add("attn.q_norm", layer.q_norm_gamma)
        add("attn.k_norm", layer.k_norm_gamma)
        add("norm2.gamma", layer.norm2.gamma)
        add("ffn.w_gate", layer.ffn.w_gate)
        add("ffn.w_up", layer.ffn.w_up)
        add("ffn.w_down", layer.ffn.w_down)
    elif isinstance(layer, F.QwenHybridSsmLayer):
        add("attn.w_qkv", layer.attn_qkv_fused)
        add("attn.w_gate", layer.attn_gate)
        add("attn.w_o", layer.attn_out)
        add("attn.q_norm", layer.q_norm_gamma)
        add("attn.k_norm", layer.k_norm_gamma)
        add("ssm.a", layer.ssm_a)
        add("ssm.w_alpha", layer.ssm_alpha)
        add("ssm.w_beta", layer.ssm_beta)
        add("ssm.w_conv1d", layer.ssm_conv1d)
        add("ssm.dt", layer.ssm_dt)
        add("ssm.norm.gamma", layer.ssm_norm_gamma)
        add("ssm.w_out", layer.ssm_out)
        add("norm2.gamma", layer.norm2.gamma)
        add("ffn.w_gate", layer.ffn.w_gate)
        add("ffn.w_up", layer.ffn.w_up)
        add("ffn.w_down", layer.ffn.w_down)
    else:
        raise TypeError(type(layer))

    return out


def _identity_scales_for(tensors: dict[str, np.ndarray]) -> dict:
    """Build a `scales` dict that quantizes f32 → i8 with scale 1.0 (no
    change in magnitude). Use the same scale for every tensor so the
    streaming converter's `_ws` lookup always returns IDENTITY_SCALE_NUM."""
    return {
        "weight_scales": {name: IDENTITY_SCALE_NUM for name in tensors},
        "activation_scales": {"default": 1024},  # arbitrary; not used in pure layer write
        "norm_eps_q": 1,
    }


# -----------------------------------------------------------------------------
# Per-block-kind parity tests.
# -----------------------------------------------------------------------------


def _streaming_layer_bytes_via_helper(layer, n, helper, *args) -> bytes:
    """Run a `_stream_*_layer_bytes` helper into a tempfile-backed
    `StreamingWeightsWriter` and return the written bytes."""
    import tempfile
    tensors = _gather_layer_tensors(layer, n)
    scales = _identity_scales_for(tensors)
    arch = type("ArchDims", (), {})()  # only block-kind helpers need head_dim
    arch.head_dim = (
        layer.attn.head_dim
        if hasattr(layer, "attn") and hasattr(layer.attn, "head_dim")
        else (layer.head_dim if hasattr(layer, "head_dim") else 4)
    )
    # MockGgufStream doesn't need .arch for per-layer helpers, only .lookup
    # and the patched dequantize_tensor.
    block_kinds: list = []
    with MockGgufStream(arch=arch, block_kinds=block_kinds, tensors_f32=tensors) as stream:
        with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as tf:
            tmppath = tf.name
        try:
            with SW.StreamingWeightsWriter(tmppath) as w:
                helper(w, stream, n, scales, *args)
            with open(tmppath, "rb") as f:
                return f.read()
        finally:
            os.unlink(tmppath)


def _reference_layer_bytes(layer) -> bytes:
    """Run `streaming_writer._stream_layer_weights` on the same layer
    and return the bytes."""
    import tempfile
    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as tf:
        tmppath = tf.name
    try:
        with SW.StreamingWeightsWriter(tmppath) as w:
            SW._stream_layer_weights(w, layer)
        with open(tmppath, "rb") as f:
            return f.read()
    finally:
        os.unlink(tmppath)


def test_attention_layer_bytes_match():
    layer = D.build_attn_layer(hidden=8, seed=0x100)
    actual = _streaming_layer_bytes_via_helper(
        layer, 0, QS._stream_attention_layer_bytes
    )
    expected = _reference_layer_bytes(layer)
    assert actual == expected, (
        f"AttentionLayer streaming bytes diverge: "
        f"{len(actual)} vs {len(expected)} bytes"
    )
    print("test_attention_layer_bytes_match OK")


def test_gemma_layer_bytes_match():
    layer = GM.build_gemma_layer(hidden=8, seed=0x200, sliding_window=None)
    actual = _streaming_layer_bytes_via_helper(
        layer, 0, QS._stream_gemma_attention_layer_bytes,
        layer.attn.head_dim,
        layer.inp_gate is not None,
        layer.layer_output_scale is not None,
    )
    expected = _reference_layer_bytes(layer)
    assert actual == expected, (
        f"GemmaLayer streaming bytes diverge: "
        f"{len(actual)} vs {len(expected)} bytes"
    )
    print("test_gemma_layer_bytes_match OK")


def test_qwen_standard_layer_bytes_match():
    layer = HM.build_qwen_standard(hidden=8, seed=0x300)
    actual = _streaming_layer_bytes_via_helper(
        layer, 0, QS._stream_qwen_standard_attention_layer_bytes,
        layer.attn.head_dim,
    )
    expected = _reference_layer_bytes(layer)
    assert actual == expected, (
        f"QwenStandardLayer streaming bytes diverge: "
        f"{len(actual)} vs {len(expected)} bytes"
    )
    print("test_qwen_standard_layer_bytes_match OK")


def test_qwen_hybrid_ssm_layer_bytes_match():
    layer = HM.build_qwen_hybrid_ssm(hidden=8, seed=0x400)
    actual = _streaming_layer_bytes_via_helper(
        layer, 0, QS._stream_qwen_hybrid_ssm_layer_bytes,
        layer.head_dim,
        layer.ssm_head_dim,
    )
    expected = _reference_layer_bytes(layer)
    assert actual == expected, (
        f"QwenHybridSsmLayer streaming bytes diverge: "
        f"{len(actual)} vs {len(expected)} bytes"
    )
    print("test_qwen_hybrid_ssm_layer_bytes_match OK")


def test_deltanet_layer_bytes_match():
    """DeltaNet layer parity. Build a small DeltaNetLayer manually
    (synthetic_qwen_mini.py only ships AttentionLayer)."""
    s = D.small_scale()
    hu = 8
    nq = 2
    nv = 2
    hd = 4
    layer = F.DeltaNetLayer(
        norm1=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, 0x500)),
            beta=None, eps_q=1, post_scale=s,
        ),
        dnet=F.DeltaNetWeights(
            hidden=hu, num_qk_heads=nq, num_v_heads=nv,
            head_dim_qk=hd, head_dim_v=hd,
            w_q=tuple(R.canonical_input_i8(hu * nq * hd, 0x501)),
            w_k=tuple(R.canonical_input_i8(hu * nq * hd, 0x502)),
            w_v=tuple(R.canonical_input_i8(hu * nv * hd, 0x503)),
            w_alpha=tuple(R.canonical_input_i8(hu * nq, 0x504)),
            w_beta=tuple(R.canonical_input_i8(hu * nq, 0x505)),
            w_o=tuple(R.canonical_input_i8(nv * hd * hu, 0x506)),
        ),
        dnet_scales=F.DeltaNetScales(
            q=s, k=s, v=s, alpha_logit=s, beta_logit=s,
            u=s, decay=s, update=s, o=s, proj=s,
        ),
        norm2=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, 0x507)),
            beta=None, eps_q=1, post_scale=s,
        ),
        ffn=F.FfnWeights(
            hidden=hu, intermediate=hu * 2,
            w_gate=tuple(R.canonical_input_i8(hu * hu * 2, 0x508)),
            w_up=tuple(R.canonical_input_i8(hu * hu * 2, 0x509)),
            w_down=tuple(R.canonical_input_i8(hu * 2 * hu, 0x50A)),
        ),
        ffn_scales=R.FfnScales(gate=s, up=s, mid=s, down=s),
    )
    actual = _streaming_layer_bytes_via_helper(
        layer, 0, QS._stream_deltanet_layer_bytes
    )
    expected = _reference_layer_bytes(layer)
    assert actual == expected, (
        f"DeltaNetLayer streaming bytes diverge: "
        f"{len(actual)} vs {len(expected)} bytes"
    )
    print("test_deltanet_layer_bytes_match OK")


def test_default_gamma_fallback_for_missing_qk_norm():
    """When `attn.q_norm` / `attn.k_norm` is missing from the GGUF
    lookup (real Qwen 3.6 hybrid blocks), the streaming converter
    falls back to a no-op identity gamma (all-127 i8 with scale
    1/127). Verify the fallback emits exactly head_dim 127s."""
    import tempfile
    layer = HM.build_qwen_hybrid_ssm(hidden=8, seed=0x600)
    # Strip q_norm and k_norm from the tensors dict so the streaming
    # converter has to fall back.
    tensors = _gather_layer_tensors(layer, 0)
    head_dim = layer.head_dim
    del tensors["layer[0].attn.q_norm"]
    del tensors["layer[0].attn.k_norm"]
    scales = _identity_scales_for(tensors)
    # Tell the streaming helper to use the default-gamma scale for
    # the missing names.
    scales["weight_scales"]["layer[0].attn.q_norm"] = QS.DEFAULT_GAMMA_SCALE_NUM
    scales["weight_scales"]["layer[0].attn.k_norm"] = QS.DEFAULT_GAMMA_SCALE_NUM
    arch = type("ArchDims", (), {})()
    arch.head_dim = head_dim
    with MockGgufStream(arch=arch, block_kinds=[], tensors_f32=tensors) as stream:
        with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as tf:
            tmppath = tf.name
        try:
            with SW.StreamingWeightsWriter(tmppath) as w:
                QS._stream_qwen_hybrid_ssm_layer_bytes(
                    w, stream, 0, scales, head_dim, layer.ssm_head_dim
                )
            with open(tmppath, "rb") as f:
                bytes_out = f.read()
        finally:
            os.unlink(tmppath)
    # Find where the q_norm gamma block lands. Layout is:
    #   norm1 (8) + attn.w_qkv (8 * 16) + attn.w_gate (8*8) + attn.w_o (8*8)
    #   + q_norm (4) + k_norm (4) + ...
    # i.e. q_norm starts at offset 8 + 128 + 64 + 64 = 264.
    q_norm_off = 8 + 8 * 16 + 8 * 8 + 8 * 8
    q_norm_bytes = bytes_out[q_norm_off:q_norm_off + head_dim]
    k_norm_bytes = bytes_out[q_norm_off + head_dim:q_norm_off + 2 * head_dim]
    assert q_norm_bytes == bytes([127] * head_dim), (
        f"q_norm fallback wrong: {list(q_norm_bytes)}"
    )
    assert k_norm_bytes == bytes([127] * head_dim), (
        f"k_norm fallback wrong: {list(k_norm_bytes)}"
    )
    print("test_default_gamma_fallback_for_missing_qk_norm OK")


if __name__ == "__main__":
    test_attention_layer_bytes_match()
    test_deltanet_layer_bytes_match()
    test_gemma_layer_bytes_match()
    test_qwen_standard_layer_bytes_match()
    test_qwen_hybrid_ssm_layer_bytes_match()
    test_default_gamma_fallback_for_missing_qk_norm()
    print("ALL streaming_quantize_all_archs TESTS PASSED")
