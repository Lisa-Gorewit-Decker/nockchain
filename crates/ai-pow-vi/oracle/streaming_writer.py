"""Phase 2.15 — streaming weights.bin writer + comm_W computation.

The non-streaming `synthetic_qwen_mini.encode_weights(model)` builds the
full byte buffer in RAM and then hashes it via `weights_merkle_root`.
For real-model checkpoints (5+ GB of int8 weights for Qwen 3.6 27B,
Gemma 4 31B) that's too much memory.

This module implements the same disk format and the same `comm_W`
output via a streaming pipeline:

  1. `StreamingWeightsWriter` opens `weights.bin` for append, holds an
     internal [`StreamingMerkle`] (Phase 2.15 stack-of-subtrees), and
     exposes `append_i8s(values)` to write a tensor's bytes once and
     forget them. The writer never holds more than one tensor's bytes
     plus an O(log n) Merkle stack in RAM.

  2. `streaming_compute_comm_w(stream_writer, manifest_hash)` produces
     the same 32-byte `comm_W` as the non-streaming `compute_comm_w` —
     the streaming Merkle root of the canonical bytes is hashed under
     `CTX_COMM_W` together with the manifest hash.

  3. `streaming_write_model_dir(model, dir, arch_tag, feature_flags)`
     orchestrates the whole pipeline for a `forward_reference.Model`:
     opens the writer, iterates tensors in canonical order, encodes
     manifest.bin separately, finalizes the Merkle root, derives
     comm_W, writes comm_w.hex.

The output is **byte-equal to `synthetic_qwen_mini.encode_weights` +
`compute_comm_w`** — verified by `tests/test_streaming_writer.py` over
all five layer flavors (Attention, DeltaNet, Gemma, QwenStandard,
QwenHybridSsm). This is what the real-model converter (next step:
`quantize_streaming.py`) plugs into; once you can stream-write a
canonical Model, you only need a streaming source of f32 tensors —
which `gguf_reader.iter_tensors()` (Phase 2.15) provides.
"""

from __future__ import annotations

import os
import struct
from typing import IO, Iterable, Sequence

import blake3
import numpy as np

import forward_reference as F
import streaming_merkle as SM
import synthetic_qwen_mini as D

CTX_COMM_W = "ai-pow-vi v1 comm-w"


class StreamingWeightsWriter:
    """Append i8 tensors to an open `weights.bin` while computing the
    weight tile-Merkle root in a single streaming pass.

    Use as a context manager:
        with StreamingWeightsWriter(path) as w:
            w.append_i8s(embed)
            w.append_i8s(layer0_norm1_gamma)
            ...
        # w.merkle_root() is now valid (it was finalized on __exit__).
    """

    def __init__(self, path: str) -> None:
        # Ensure parent dir exists, then open for binary append.
        os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
        self.path = path
        self._fh: IO = open(path, "wb", buffering=1 << 20)  # 1 MB write buffer
        self._merkle = SM.StreamingMerkle()
        self._bytes_written = 0
        self._root: bytes | None = None

    # --- core API -----------------------------------------------------

    def append_i8s(self, values: Sequence[int] | np.ndarray) -> None:
        """Append a sequence of i8 values to weights.bin and the
        running Merkle tree."""
        arr = np.asarray(values, dtype=np.int8)
        if arr.size == 0:
            return
        chunk = arr.tobytes()
        self._fh.write(chunk)
        self._merkle.update(chunk)
        self._bytes_written += len(chunk)

    def append_raw(self, chunk: bytes) -> None:
        """Append a raw byte chunk (e.g. RoPE i16 cos/sin tables, ExpLut
        i32 entries, manifest u32 fields embedded in the canonical
        weight stream). The chunk is hashed into the Merkle tree as if
        it were an i8 stream — the canonical-byte ordering is what
        matters, not the per-element type."""
        if not chunk:
            return
        self._fh.write(chunk)
        self._merkle.update(chunk)
        self._bytes_written += len(chunk)

    def finalize(self) -> bytes:
        """Close the file and return the 32-byte Merkle root."""
        if self._root is not None:
            return self._root
        self._fh.close()
        self._root = self._merkle.finalize()
        return self._root

    def merkle_root(self) -> bytes:
        if self._root is None:
            raise RuntimeError("call finalize() before merkle_root()")
        return self._root

    @property
    def bytes_written(self) -> int:
        return self._bytes_written

    # --- context manager ----------------------------------------------

    def __enter__(self) -> "StreamingWeightsWriter":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        # On exception, still close the file but skip Merkle finalize
        # so the caller's bug doesn't get masked.
        if exc_type is None:
            self.finalize()
        else:
            try:
                self._fh.close()
            except Exception:
                pass


# -----------------------------------------------------------------------------
# Canonical byte stream emitter for `forward_reference.Model`.
#
# Mirrors `synthetic_qwen_mini.encode_weights` exactly, but emits one
# tensor at a time to a [`StreamingWeightsWriter`].
# -----------------------------------------------------------------------------


def _stream_norm_weights(w: StreamingWeightsWriter, norm: F.NormSpec) -> None:
    w.append_i8s(norm.gamma)
    if norm.beta is not None:
        w.append_i8s(norm.beta)


def _stream_layer_weights(w: StreamingWeightsWriter, layer) -> None:
    """Emit one layer's bytes in the canonical order. Mirrors the
    matching arms of `synthetic_qwen_mini.encode_weights`."""
    # norm1 is shared.
    _stream_norm_weights(w, layer.norm1)
    if isinstance(layer, F.AttentionLayer):
        w.append_i8s(layer.attn.w_q)
        w.append_i8s(layer.attn.w_k)
        w.append_i8s(layer.attn.w_v)
        w.append_i8s(layer.attn.w_o)
        _stream_norm_weights(w, layer.norm2)
        w.append_i8s(layer.ffn.w_gate)
        w.append_i8s(layer.ffn.w_up)
        w.append_i8s(layer.ffn.w_down)
    elif isinstance(layer, F.DeltaNetLayer):
        w.append_i8s(layer.dnet.w_q)
        w.append_i8s(layer.dnet.w_k)
        w.append_i8s(layer.dnet.w_v)
        w.append_i8s(layer.dnet.w_alpha)
        w.append_i8s(layer.dnet.w_beta)
        w.append_i8s(layer.dnet.w_o)
        _stream_norm_weights(w, layer.norm2)
        w.append_i8s(layer.ffn.w_gate)
        w.append_i8s(layer.ffn.w_up)
        w.append_i8s(layer.ffn.w_down)
    elif isinstance(layer, F.GemmaLayer):
        w.append_i8s(layer.attn.w_q)
        w.append_i8s(layer.attn.w_k)
        w.append_i8s(layer.attn.w_v)
        w.append_i8s(layer.attn.w_o)
        w.append_i8s(layer.q_norm_gamma)
        w.append_i8s(layer.k_norm_gamma)
        _stream_norm_weights(w, layer.post_attn_norm)
        _stream_norm_weights(w, layer.norm2)
        w.append_i8s(layer.ffn.w_gate)
        w.append_i8s(layer.ffn.w_up)
        w.append_i8s(layer.ffn.w_down)
        _stream_norm_weights(w, layer.post_ffn_norm)
        if layer.inp_gate is not None:
            w.append_i8s(layer.inp_gate)
        if layer.layer_output_scale is not None:
            w.append_i8s(layer.layer_output_scale)
    elif isinstance(layer, F.QwenStandardLayer):
        w.append_i8s(layer.attn.w_q)
        w.append_i8s(layer.attn.w_k)
        w.append_i8s(layer.attn.w_v)
        w.append_i8s(layer.attn.w_o)
        w.append_i8s(layer.q_norm_gamma)
        w.append_i8s(layer.k_norm_gamma)
        _stream_norm_weights(w, layer.norm2)
        w.append_i8s(layer.ffn.w_gate)
        w.append_i8s(layer.ffn.w_up)
        w.append_i8s(layer.ffn.w_down)
    elif isinstance(layer, F.QwenHybridSsmLayer):
        w.append_i8s(layer.attn_qkv_fused)
        w.append_i8s(layer.attn_gate)
        w.append_i8s(layer.attn_out)
        w.append_i8s(layer.q_norm_gamma)
        w.append_i8s(layer.k_norm_gamma)
        w.append_i8s(layer.ssm_a)
        w.append_i8s(layer.ssm_alpha)
        w.append_i8s(layer.ssm_beta)
        w.append_i8s(layer.ssm_conv1d)
        w.append_i8s(layer.ssm_dt)
        w.append_i8s(layer.ssm_norm_gamma)
        w.append_i8s(layer.ssm_out)
        _stream_norm_weights(w, layer.norm2)
        w.append_i8s(layer.ffn.w_gate)
        w.append_i8s(layer.ffn.w_up)
        w.append_i8s(layer.ffn.w_down)
    else:
        raise TypeError(f"unsupported layer type {type(layer)}")


def stream_canonical_bytes(w: StreamingWeightsWriter, model: F.Model) -> None:
    """Stream the canonical weight byte order to `w`. Caller must
    finalize `w` afterward to get the Merkle root.

    Order mirrors `crate::comm_w::canonical_weight_bytes` exactly:
        embed → layer[0..N] → final_norm? → ffn_lut → sigmoid_lut →
        softmax_lut → rope_tables.
    """
    w.append_i8s(model.embed)
    for layer in model.layers:
        _stream_layer_weights(w, layer)
    if model.final_norm is not None:
        _stream_norm_weights(w, model.final_norm)
    # Activation LUTs — same byte layout the non-streaming `encode_weights` uses.
    w.append_raw(bytes((b & 0xFF for b in model.ffn_activation_bytes)))
    w.append_raw(bytes((b & 0xFF for b in model.sigmoid_lut_bytes)))
    w.append_raw(np.array(model.softmax_lut.table, dtype=np.int32).tobytes())
    # RoPE: u32 seq_len + u32 half_head_dim + i16 cos[] + i16 sin[].
    w.append_raw(struct.pack("<I", model.rope_tables.seq_len))
    w.append_raw(struct.pack("<I", model.rope_tables.half_head_dim))
    w.append_raw(np.array(model.rope_tables.cos, dtype=np.int16).tobytes())
    w.append_raw(np.array(model.rope_tables.sin, dtype=np.int16).tobytes())


def streaming_compute_comm_w(
    weights_root: bytes,
    manifest_hash: bytes,
) -> bytes:
    """Mirror of `crate::comm_w::compute_comm_w` final step:
    `comm_W = derive_key("ai-pow-vi v1 comm-w")(weights_root || manifest_hash)`."""
    h = blake3.blake3(derive_key_context=CTX_COMM_W)
    h.update(weights_root)
    h.update(manifest_hash)
    return h.digest(length=32)


def streaming_write_model_dir(
    model: F.Model,
    out_dir: str,
    arch_tag: str = "",
    feature_flags: int = 0,
    ffn_kind: str = "identity",
    sigmoid_kind: str = "identity",
) -> bytes:
    """End-to-end streaming pipeline: writes manifest.bin + weights.bin
    + comm_w.hex to `out_dir` and returns the resulting `comm_W`.

    Memory bound: O(largest single tensor) for the materialized
    `forward_reference.Model` arguments (callers with a real GGUF
    source should plug in `gguf_reader.iter_tensors()` instead of
    materializing tuples), plus O(log num_tiles) for the Merkle stack.
    """
    os.makedirs(out_dir, exist_ok=True)

    # Manifest is small — encode it in RAM and write once.
    manifest_bytes = D.encode_manifest(
        model.dims,
        list(model.layers),
        model.final_norm,
        model.rope_tables,
        ffn_kind=ffn_kind,
        sigmoid_kind=sigmoid_kind,
        arch_tag=arch_tag,
        feature_flags=feature_flags,
    )
    with open(os.path.join(out_dir, "manifest.bin"), "wb") as f:
        f.write(manifest_bytes)

    # Stream weights.bin and compute the Merkle root in one pass.
    weights_path = os.path.join(out_dir, "weights.bin")
    with StreamingWeightsWriter(weights_path) as w:
        stream_canonical_bytes(w, model)
    weights_root = w.merkle_root()

    # Manifest hash uses the model metadata directly (small).
    manifest_hash = D.manifest_hash_bytes(
        model, arch_tag=arch_tag, feature_flags=feature_flags
    )

    comm_w = streaming_compute_comm_w(weights_root, manifest_hash)
    with open(os.path.join(out_dir, "comm_w.hex"), "w") as f:
        f.write(comm_w.hex())
    return comm_w
