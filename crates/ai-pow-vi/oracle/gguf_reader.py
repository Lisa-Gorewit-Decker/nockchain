"""Phase 2.9.2 — GGUF reader with dequantize + canonical-name mapping.

Reads a GGUF file (e.g. an Ollama-cached Qwen 3 blob), dequantizes every
tensor to `np.float32`, and returns a dict keyed by our canonical
weight names (`embed`, `layer[N].attn.w_q`, etc.).

Linear weights are transposed at read time: GGUF / safetensors store
them as `(out, in)` row-major (the PyTorch / transformers convention),
but `crate::matmul_int8` expects `(in, out)` **column-major**. The flat
i8 buffer that gets written to weights.bin is column-major because
column j of B lives at the contiguous range `[j*k, j*k + k)`. So we
transpose to (in, out) and flatten — `arr.T.reshape(-1)` does both at
once.

Tensor name mapping (extends `TENSOR_NAME_MAP` for new architectures):

    token_embd.weight                 → embed
    blk.{N}.attn_norm.weight          → layer[N].norm1.gamma
    blk.{N}.attn_q.weight             → layer[N].attn.w_q
    blk.{N}.attn_k.weight             → layer[N].attn.w_k
    blk.{N}.attn_v.weight             → layer[N].attn.w_v
    blk.{N}.attn_output.weight        → layer[N].attn.w_o
    blk.{N}.ffn_norm.weight           → layer[N].norm2.gamma
    blk.{N}.ffn_gate.weight           → layer[N].ffn.w_gate
    blk.{N}.ffn_up.weight             → layer[N].ffn.w_up
    blk.{N}.ffn_down.weight           → layer[N].ffn.w_down
    output_norm.weight                → final_norm.gamma
    output.weight                     → lm_head      (kept aside; not in Model yet)

DeltaNet-flavored layers add `delta_q`, `delta_k`, `delta_v`,
`delta_alpha`, `delta_beta`, `delta_o` (names vary by model; pass a
custom map via `extra_tensor_aliases` to override).
"""

from __future__ import annotations

import os
import re
import sys
from dataclasses import dataclass
from typing import Iterable, Optional

import gguf
import numpy as np


# Per-block tensor stems we recognize. Maps GGUF stem (after `blk.{N}.`) to
# canonical sub-name. Caller can extend via `extra_tensor_aliases`.
DEFAULT_BLOCK_STEMS: dict[str, str] = {
    "attn_norm.weight": "norm1.gamma",
    "attn_norm.bias": "norm1.beta",
    "attn_q.weight": "attn.w_q",
    "attn_k.weight": "attn.w_k",
    "attn_v.weight": "attn.w_v",
    "attn_output.weight": "attn.w_o",
    "ffn_norm.weight": "norm2.gamma",
    "ffn_norm.bias": "norm2.beta",
    "ffn_gate.weight": "ffn.w_gate",
    "ffn_up.weight": "ffn.w_up",
    "ffn_down.weight": "ffn.w_down",
    # DeltaNet (names not yet stable across all models — override via
    # `extra_tensor_aliases` when bringing up a specific Qwen variant).
    "delta_q.weight": "dnet.w_q",
    "delta_k.weight": "dnet.w_k",
    "delta_v.weight": "dnet.w_v",
    "delta_alpha.weight": "dnet.w_alpha",
    "delta_beta.weight": "dnet.w_beta",
    "delta_o.weight": "dnet.w_o",
}

DEFAULT_TOPLEVEL: dict[str, str] = {
    "token_embd.weight": "embed",
    "output_norm.weight": "final_norm.gamma",
    "output_norm.bias": "final_norm.beta",
    "output.weight": "lm_head",  # not yet in Model; preserved for 2.9.7
}


# Linear weights that need (out, in) → (in, out) col-major transpose.
# Norms (gamma/beta) and the embedding table do not.
_LINEAR_CANON_NAMES = {
    "attn.w_q",
    "attn.w_k",
    "attn.w_v",
    "attn.w_o",
    "ffn.w_gate",
    "ffn.w_up",
    "ffn.w_down",
    "dnet.w_q",
    "dnet.w_k",
    "dnet.w_v",
    "dnet.w_alpha",
    "dnet.w_beta",
    "dnet.w_o",
    "lm_head",
}


@dataclass
class Architecture:
    """High-level architecture facts pulled from GGUF metadata.

    Not all GGUF writers populate every field; the user can override via
    constructor kwargs to `read_model`. Only the fields actually used by
    the quantizer are required.
    """

    name: str
    num_layers: int
    hidden: int
    intermediate: int
    num_q_heads: int
    num_kv_heads: int
    head_dim: int
    vocab_size: int
    rope_theta: float
    max_position: int


@dataclass
class GgufModel:
    """Parsed GGUF blob, dequantized and renamed to canonical layout."""

    arch: Architecture
    tensors: dict[str, np.ndarray]


# -----------------------------------------------------------------------------
# Public API.
# -----------------------------------------------------------------------------


def read_architecture(reader: gguf.GGUFReader, arch_prefix: Optional[str] = None) -> Architecture:
    """Pull architecture facts out of a GGUF reader's KV store.

    `arch_prefix` defaults to whatever `general.architecture` says; pass
    explicitly for unknown / custom architectures.
    """

    def get_str(key: str) -> str:
        f = reader.get_field(key)
        if f is None:
            raise KeyError(f"GGUF missing required str field {key}")
        # Strings come back as bytes-of-codepoints in `parts[-1]`.
        return str(bytes(f.parts[-1]), encoding="utf-8")

    def get_u32(key: str) -> int:
        f = reader.get_field(key)
        if f is None:
            raise KeyError(f"GGUF missing required u32 field {key}")
        return int(f.parts[-1][0])

    def get_f32(key: str) -> float:
        f = reader.get_field(key)
        if f is None:
            raise KeyError(f"GGUF missing required f32 field {key}")
        return float(f.parts[-1][0])

    if arch_prefix is None:
        arch_prefix = get_str("general.architecture")

    num_layers = get_u32(f"{arch_prefix}.block_count")
    hidden = get_u32(f"{arch_prefix}.embedding_length")
    intermediate = get_u32(f"{arch_prefix}.feed_forward_length")
    num_q_heads = get_u32(f"{arch_prefix}.attention.head_count")
    num_kv_heads = get_u32(f"{arch_prefix}.attention.head_count_kv")
    # head_dim sometimes implicit (= hidden / num_q_heads).
    rope_dim = reader.get_field(f"{arch_prefix}.rope.dimension_count")
    if rope_dim is not None:
        head_dim = int(rope_dim.parts[-1][0])
    else:
        head_dim = hidden // num_q_heads
    vocab = reader.get_field(f"{arch_prefix}.vocab_size")
    if vocab is not None:
        vocab_size = int(vocab.parts[-1][0])
    else:
        # Fall back to embed table row count (set later, after tensor scan).
        vocab_size = 0
    rope_theta_field = reader.get_field(f"{arch_prefix}.rope.freq_base")
    rope_theta = (
        float(rope_theta_field.parts[-1][0]) if rope_theta_field is not None else 10_000.0
    )
    max_position_field = reader.get_field(f"{arch_prefix}.context_length")
    max_position = (
        int(max_position_field.parts[-1][0]) if max_position_field is not None else 4096
    )

    return Architecture(
        name=arch_prefix,
        num_layers=num_layers,
        hidden=hidden,
        intermediate=intermediate,
        num_q_heads=num_q_heads,
        num_kv_heads=num_kv_heads,
        head_dim=head_dim,
        vocab_size=vocab_size,
        rope_theta=rope_theta,
        max_position=max_position,
    )


def dequantize_tensor(t: gguf.ReaderTensor) -> np.ndarray:
    """Convert any GGUF-quantized tensor to f32 in its logical shape.

    GGUF stores `t.shape` in reversed (column-major-like) order vs the
    PyTorch / safetensors convention. We reverse on read so the returned
    array matches what HF / Ollama would see for the same tensor.
    """
    qtype = t.tensor_type
    target_shape = tuple(int(s) for s in t.shape[::-1])
    if qtype == gguf.GGMLQuantizationType.F32:
        return np.asarray(t.data, dtype=np.float32).reshape(target_shape)
    if qtype == gguf.GGMLQuantizationType.F16:
        return (
            np.asarray(t.data, dtype=np.float16).astype(np.float32).reshape(target_shape)
        )
    # Generic dequantize for the K-quants and Q*_0 / Q*_1 family.
    deq = gguf.dequantize(np.asarray(t.data), qtype)
    return deq.astype(np.float32).reshape(target_shape)


def map_tensor_name(
    name: str,
    block_stems: dict[str, str],
    toplevel: dict[str, str],
) -> Optional[tuple[Optional[int], str]]:
    """Map a GGUF tensor name to (layer_idx_or_None, canonical_subname).

    Returns None for tensors we don't recognize (e.g. tokenizer
    artifacts that ship inside GGUF but aren't weights).
    """
    if name in toplevel:
        return None, toplevel[name]
    m = re.match(r"^blk\.(\d+)\.(.+)$", name)
    if m is None:
        return None
    layer_idx = int(m.group(1))
    stem = m.group(2)
    if stem in block_stems:
        return layer_idx, block_stems[stem]
    return None


def transpose_linear_to_col_major(arr: np.ndarray, canon: str) -> np.ndarray:
    """For canonical names that are linear weights, transpose (out, in) →
    (in, out) and ravel column-major."""
    if canon not in _LINEAR_CANON_NAMES:
        return arr
    if arr.ndim != 2:
        raise ValueError(f"linear weight {canon} must be 2D, got shape {arr.shape}")
    # GGUF/HF: (out, in). Our matmul_int8 wants column-major (in, out)
    # — column j at [j*k, j*k+k). That is exactly arr stored row-major
    # if arr.shape == (out, in) and we serialize each row contiguously.
    # I.e. row j of arr (length in) becomes column j of B. So we just
    # ravel `arr` in C-order: no transpose needed.
    return np.ascontiguousarray(arr).ravel()


def read_model(
    path: str,
    arch_prefix: Optional[str] = None,
    extra_tensor_aliases: Optional[dict[str, str]] = None,
) -> GgufModel:
    """Open `path`, read architecture metadata, dequantize every tensor,
    rename to canonical layout, and return a GgufModel."""
    reader = gguf.GGUFReader(path)
    arch = read_architecture(reader, arch_prefix=arch_prefix)

    block_stems = dict(DEFAULT_BLOCK_STEMS)
    toplevel = dict(DEFAULT_TOPLEVEL)
    if extra_tensor_aliases:
        for k, v in extra_tensor_aliases.items():
            if k.startswith("blk."):
                # `blk.{N}.attn_q.weight` form; strip the prefix and number,
                # leaving stem.
                m = re.match(r"^blk\.\d+\.(.+)$", k)
                if m:
                    block_stems[m.group(1)] = v
                    continue
            toplevel[k] = v

    out_tensors: dict[str, np.ndarray] = {}
    embed_rows = None
    for t in reader.tensors:
        mapping = map_tensor_name(t.name, block_stems, toplevel)
        if mapping is None:
            continue
        layer_idx, canon = mapping
        arr = dequantize_tensor(t)
        if canon == "embed":
            embed_rows = int(arr.shape[0])
        out_name = canon if layer_idx is None else f"layer[{layer_idx}].{canon}"
        out_tensors[out_name] = transpose_linear_to_col_major(arr, canon)

    if arch.vocab_size == 0 and embed_rows is not None:
        arch = Architecture(
            name=arch.name,
            num_layers=arch.num_layers,
            hidden=arch.hidden,
            intermediate=arch.intermediate,
            num_q_heads=arch.num_q_heads,
            num_kv_heads=arch.num_kv_heads,
            head_dim=arch.head_dim,
            vocab_size=embed_rows,
            rope_theta=arch.rope_theta,
            max_position=arch.max_position,
        )

    return GgufModel(arch=arch, tensors=out_tensors)


# -----------------------------------------------------------------------------
# CLI.
# -----------------------------------------------------------------------------


def main(argv: Optional[Iterable[str]] = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("path", help="Path to .gguf file")
    parser.add_argument("--arch", help="Architecture prefix (default: from GGUF metadata)")
    parser.add_argument("--list-tensors", action="store_true")
    args = parser.parse_args(list(argv) if argv is not None else None)

    model = read_model(args.path, arch_prefix=args.arch)
    print(f"Architecture: {model.arch}")
    print(f"Total canonical tensors: {len(model.tensors)}")
    if args.list_tensors:
        for name, arr in sorted(model.tensors.items()):
            print(f"  {name}: shape={list(arr.shape)} dtype={arr.dtype}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
