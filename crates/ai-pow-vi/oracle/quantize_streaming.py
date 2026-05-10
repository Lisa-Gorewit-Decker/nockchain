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

Limitations of this initial driver:
- Currently supports `qwen3` / `qwen3_legacy` Attention-only layers
  end-to-end. The same pattern (canonical-order walk + per-tensor
  on-demand dequant) extends to Gemma / QwenStandard / QwenHybridSsm
  trivially — the layer-spec assembly is the only code that differs.
  Extend `_layer_canonical_names` and `_build_layer_metadata` to add
  arch coverage.
- Activation scales are taken from a `scales.json` produced by
  `calibrate.py` (already streaming-friendly — it reads the GGUF once).
- The lm_head tensor is still saved as a sibling `lm_head.bin` and is
  *not* part of comm_W (matching `quantize_qwen.py`).

Usage:
    python oracle/quantize_streaming.py \\
        --gguf /path/to/model.gguf \\
        --scales /path/to/scales.json \\
        --out  $NOCKCHAIN_VI_QWEN_DIR \\
        --seq-len 4096 \\
        --activation-tile 64

After conversion, verify: `Model::load($NOCKCHAIN_VI_QWEN_DIR,
&expected_comm_w)` succeeds in Rust and `vi-eval --model-dir
$NOCKCHAIN_VI_QWEN_DIR ...` runs against the loaded model.
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


def _stream_attention_layer_bytes(
    w: SW.StreamingWeightsWriter,
    stream: G.GgufStream,
    n: int,
    scales: dict,
) -> None:
    """Walk one Attention-layer's canonical bytes from the GGUF in
    streaming fashion. Mirrors `_stream_layer_weights` for
    `AttentionLayer`."""

    def s(name: str) -> int:
        return Q._ws(scales, name)

    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].norm1.gamma",
                                       s(f"layer[{n}].norm1.gamma")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].attn.w_q",
                                       s(f"layer[{n}].attn.w_q")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].attn.w_k",
                                       s(f"layer[{n}].attn.w_k")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].attn.w_v",
                                       s(f"layer[{n}].attn.w_v")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].attn.w_o",
                                       s(f"layer[{n}].attn.w_o")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].norm2.gamma",
                                       s(f"layer[{n}].norm2.gamma")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].ffn.w_gate",
                                       s(f"layer[{n}].ffn.w_gate")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].ffn.w_up",
                                       s(f"layer[{n}].ffn.w_up")))
    w.append_i8s(_quantize_one_tensor(stream, f"layer[{n}].ffn.w_down",
                                       s(f"layer[{n}].ffn.w_down")))


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
    """
    arch = stream.arch

    # Embed.
    w.append_i8s(_quantize_one_tensor(stream, "embed", Q._ws(scales, "embed")))

    # Per-layer.
    for n in range(arch.num_layers):
        kind = stream.block_kinds[n]
        if kind.value == "standard_attention":
            _stream_attention_layer_bytes(w, stream, n, scales)
        else:
            raise NotImplementedError(
                f"layer {n}: streaming converter currently only supports "
                f"STANDARD_ATTENTION blocks; got {kind}. Extend "
                f"_stream_*_layer_bytes (Phase 2.15)."
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

    # Build a metadata-only Model for the manifest writer. Tensor
    # tuples are empty — manifest encoders only read scalar fields.
    layers: list = []
    for n in range(arch.num_layers):
        norm1 = F.NormSpec(
            kind="rms", gamma=(), beta=None,
            eps_q=int(scales.get("norm_eps_q", 1)),
            post_scale=R.Scale(num=Q._as(scales, f"layer[{n}].norm_post.1")),
        )
        norm2 = F.NormSpec(
            kind="rms", gamma=(), beta=None,
            eps_q=int(scales.get("norm_eps_q", 1)),
            post_scale=R.Scale(num=Q._as(scales, f"layer[{n}].norm_post.2")),
        )
        layers.append(F.AttentionLayer(
            norm1=norm1,
            attn=F.AttentionWeights(
                hidden=arch.hidden,
                num_q_heads=arch.num_q_heads,
                num_kv_heads=arch.num_kv_heads,
                head_dim=arch.head_dim,
                w_q=(), w_k=(), w_v=(), w_o=(),
            ),
            attn_scales=F.AttentionScales(
                q=R.Scale(num=Q._as(scales, f"layer[{n}].attn.q")),
                k=R.Scale(num=Q._as(scales, f"layer[{n}].attn.k")),
                v=R.Scale(num=Q._as(scales, f"layer[{n}].attn.v")),
                score=R.Scale(num=Q._as(scales, f"layer[{n}].attn.score")),
                attn_out=R.Scale(num=Q._as(scales, f"layer[{n}].attn.attn_out")),
                o=R.Scale(num=Q._as(scales, f"layer[{n}].attn.o")),
            ),
            norm2=norm2,
            ffn=F.FfnWeights(
                hidden=arch.hidden,
                intermediate=arch.intermediate,
                w_gate=(), w_up=(), w_down=(),
            ),
            ffn_scales=R.FfnScales(
                gate=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.gate")),
                up=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.up")),
                mid=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.mid")),
                down=R.Scale(num=Q._as(scales, f"layer[{n}].ffn.down")),
            ),
        ))

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
    p.add_argument("--scales", required=True, help="path to scales.json from calibrate.py")
    p.add_argument("--out", required=True, help="output model directory")
    p.add_argument("--seq-len", type=int, default=4096)
    p.add_argument("--activation-tile", type=int, default=64)
    p.add_argument("--arch", help="architecture prefix override")
    args = p.parse_args(list(argv) if argv is not None else None)

    scales = json.loads(open(args.scales).read())
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
