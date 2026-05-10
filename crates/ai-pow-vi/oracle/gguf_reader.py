"""Phase 2.10 — registry-driven GGUF reader.

Reads any GGUF file whose `general.architecture` is in
`oracle/arch/REGISTRY` (currently `qwen3`, `qwen35`, `gemma4`),
dequantizes every tensor to `np.float32`, and emits a dict keyed by
**canonical** weight names: `embed`, `layer[N].attn.w_q`,
`layer[N].ssm.w_alpha`, etc.

Drop-in compatible with the Phase 2.9 API: `read_model(path)` still
works for `qwen3` GGUFs without any caller change. New `qwen35` /
`gemma4` GGUFs route through their own arch module's name map and
block-kind classifier.
"""

from __future__ import annotations

import os
import re
import sys
from dataclasses import dataclass
from typing import Iterable, Optional

import gguf
import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import arch as _arch  # noqa: E402
from arch import ArchDims, Architecture, BlockKind, Feature  # noqa: E402

# Canonical names that are linear weights (need (out, in) row-major
# storage; gguf stores them in this form already).
_LINEAR_CANON_SUBSTRINGS = {
    "attn.w_q",
    "attn.w_k",
    "attn.w_v",
    "attn.w_o",
    "attn.w_qkv",
    "attn.w_gate",
    "ffn.w_gate",
    "ffn.w_up",
    "ffn.w_down",
    "ssm.w_alpha",
    "ssm.w_beta",
    "ssm.w_out",
    "dnet.w_q",
    "dnet.w_k",
    "dnet.w_v",
    "dnet.w_alpha",
    "dnet.w_beta",
    "dnet.w_o",
    "lm_head",
    "per_layer_proj",
    "proj",
}


@dataclass
class GgufModel:
    arch: ArchDims
    arch_obj: Architecture
    block_kinds: list[BlockKind]
    tensors: dict[str, np.ndarray]
    feature_flags: int


@dataclass
class GgufStream:
    """Phase 2.15 — streaming-friendly handle to a GGUF file.

    Holds the (cheaply mmapped) reader, architecture metadata, and a
    pre-built `canonical_name → gguf.ReaderTensor` lookup so callers
    can iterate tensors lazily and in any order without re-walking the
    file. Created by `open_stream()`; consumed by
    `iter_tensors(stream)` for one-tensor-at-a-time dequant or by
    direct lookup `stream.lookup[name]` for canonical-order writers.
    """

    reader: gguf.GGUFReader
    arch: ArchDims
    arch_obj: Architecture
    block_kinds: list[BlockKind]
    feature_flags: int
    # Maps canonical name (e.g. `embed`, `layer[0].attn.w_q`) to the
    # underlying GGUF reader-tensor. Any tensor that doesn't appear in
    # the architecture's alias maps is dropped (vision/audio/multimodal
    # auxiliaries — see `_map_tensor_name`).
    lookup: dict[str, "gguf.ReaderTensor"]


def open_stream(
    path: str,
    arch_override: Optional[str] = None,
    extra_tensor_aliases: Optional[dict[str, str]] = None,
) -> GgufStream:
    """Open a GGUF file and return a [`GgufStream`] handle. No tensors
    are dequantized here — the cost is only the GGUF index walk, which
    is O(num_tensors) and a few MB of allocation regardless of file
    size."""
    reader = gguf.GGUFReader(path)
    if arch_override is None:
        arch_str = str(
            bytes(reader.get_field("general.architecture").parts[-1]), encoding="utf-8"
        )
    else:
        arch_str = arch_override
    arch_obj = _arch.get(arch_str)
    dims = arch_obj.read_dims(reader)
    block_kinds = [arch_obj.block_kind(reader, i) for i in range(dims.num_layers)]

    default_block_map = arch_obj.tensor_alias_map()
    toplevel_map = arch_obj.toplevel_alias_map()
    if extra_tensor_aliases:
        for k, v in extra_tensor_aliases.items():
            if not k.startswith("blk."):
                toplevel_map[k] = v

    lookup: dict[str, "gguf.ReaderTensor"] = {}
    embed_rows = None
    for t in reader.tensors:
        canon = _map_tensor_name(
            t.name,
            arch_obj=arch_obj,
            reader=reader,
            toplevel_map=toplevel_map,
            default_block_map=default_block_map,
            extra=extra_tensor_aliases or {},
        )
        if canon is None:
            continue
        layer_idx, sub = canon
        out_name = sub if layer_idx is None else f"layer[{layer_idx}].{sub}"
        lookup[out_name] = t
        if sub == "embed":
            # Track embed rows so we can fill in vocab_size when the
            # arch left it as 0.
            embed_rows = int(t.shape[-1])
    if dims.vocab_size == 0 and embed_rows is not None:
        dims = ArchDims(
            name=dims.name,
            num_layers=dims.num_layers,
            hidden=dims.hidden,
            intermediate=dims.intermediate,
            num_q_heads=dims.num_q_heads,
            num_kv_heads=dims.num_kv_heads,
            head_dim=dims.head_dim,
            head_dim_kv=dims.head_dim_kv,
            vocab_size=embed_rows,
            rope_theta=dims.rope_theta,
            max_position=dims.max_position,
            sliding_window=dims.sliding_window,
            extras=dims.extras,
        )
    return GgufStream(
        reader=reader,
        arch=dims,
        arch_obj=arch_obj,
        block_kinds=block_kinds,
        feature_flags=arch_obj.feature_flags(),
        lookup=lookup,
    )


def iter_tensors(stream: GgufStream):
    """Yield `(canonical_name, np.ndarray f32)` one tensor at a time, in
    GGUF on-disk order. Each yielded ndarray goes out of scope when the
    next iteration begins, so peak memory is bounded by the largest
    single tensor (typically the embed table or `lm_head` — a few
    hundred MB for a 17-19 GB GGUF, vs ~55-64 GB if every tensor were
    held simultaneously).

    Iteration order is stable across runs but is *not* canonical
    (Rust-side write order). Callers that need canonical order should
    walk `stream.lookup[canonical_name]` directly and dequantize on
    demand."""
    # Reverse the lookup map for fast on-disk-order traversal: tensor
    # name → canonical name. Keep only mapped tensors.
    by_gguf_name = {t.name: c for c, t in stream.lookup.items()}
    for t in stream.reader.tensors:
        canon = by_gguf_name.get(t.name)
        if canon is None:
            continue
        arr = dequantize_tensor(t)
        # Use the same flatten rule the non-streaming `read_model` uses
        # so streaming and non-streaming consumers see identical shapes.
        sub = canon.split(".", 1)[-1] if "." in canon else canon
        yield canon, _flatten_linear(arr, sub)
        # arr falls out of scope here (caller drops the yielded ref).


def dequantize_tensor(t: gguf.ReaderTensor) -> np.ndarray:
    """Convert any GGUF-quantized tensor to f32 in PyTorch shape order.
    GGUF stores shape reversed; we reverse on read.

    `t.data` is a uint8 byte buffer for every tensor type. We
    reinterpret-via-`.view()` rather than cast-via-`asarray` so the
    bytes are decoded as their actual numerical type rather than 1
    f32 per raw byte."""
    qtype = t.tensor_type
    target_shape = tuple(int(s) for s in t.shape[::-1])
    raw = np.asarray(t.data)
    if qtype == gguf.GGMLQuantizationType.F32:
        if raw.dtype != np.float32:
            raw = raw.view(np.float32)
        return raw.reshape(target_shape)
    if qtype == gguf.GGMLQuantizationType.F16:
        if raw.dtype != np.float16:
            raw = raw.view(np.float16)
        return raw.astype(np.float32).reshape(target_shape)
    if qtype == gguf.GGMLQuantizationType.BF16:
        # numpy has no native bf16; reinterpret 2 bytes per element as
        # uint16, then promote to uint32 and shift left to land in the
        # upper 16 bits of a float32 (zero mantissa fill).
        if raw.dtype != np.uint16:
            raw = raw.view(np.uint16)
        as_u32 = raw.astype(np.uint32) << 16
        return as_u32.view(np.float32).reshape(target_shape).copy()
    # K-quants and Q*_0 / Q*_1 family — generic dequantize path.
    deq = gguf.dequantize(raw, qtype)
    return deq.astype(np.float32).reshape(target_shape)


def _flatten_linear(arr: np.ndarray, canon: str) -> np.ndarray:
    """Linear weights in our crate are stored column-major as a flat
    `(in * out)` 1-D buffer: column j at `[j*k, j*k+k)`. GGUF stores
    them (out, in) row-major. We ravel as-is — that's already
    column-major for the (in, out) interpretation our matmul wants.

    1-D weights (norms, scalars) pass through unchanged."""
    if any(sub in canon for sub in _LINEAR_CANON_SUBSTRINGS):
        if arr.ndim != 2:
            # Some "linear" canonical names are actually 1-D (e.g.
            # `inp_gate`, `layer_output_scale`); pass through.
            return arr.ravel()
        return np.ascontiguousarray(arr).ravel()
    return arr


def read_model(
    path: str,
    arch_override: Optional[str] = None,
    extra_tensor_aliases: Optional[dict[str, str]] = None,
) -> GgufModel:
    """Open `path`, dispatch on architecture, dequantize every tensor,
    rename to canonical layout."""
    reader = gguf.GGUFReader(path)
    if arch_override is None:
        arch_str = str(
            bytes(reader.get_field("general.architecture").parts[-1]), encoding="utf-8"
        )
    else:
        arch_str = arch_override

    arch_obj = _arch.get(arch_str)
    dims = arch_obj.read_dims(reader)

    # Classify each block.
    block_kinds: list[BlockKind] = [
        arch_obj.block_kind(reader, i) for i in range(dims.num_layers)
    ]

    # Build the per-block alias maps once: top-level + default + per-block override.
    default_block_map = arch_obj.tensor_alias_map()
    toplevel_map = arch_obj.toplevel_alias_map()

    out_tensors: dict[str, np.ndarray] = {}
    embed_rows = None

    if extra_tensor_aliases:
        for k, v in extra_tensor_aliases.items():
            if k.startswith("blk."):
                m = re.match(r"^blk\.(\d+)\.(.+)$", k)
                if m is not None:
                    # Per-block extra: applied at lookup time below.
                    pass  # handled inline
            else:
                toplevel_map[k] = v

    for t in reader.tensors:
        canon: Optional[tuple[Optional[int], str]] = _map_tensor_name(
            t.name,
            arch_obj=arch_obj,
            reader=reader,
            toplevel_map=toplevel_map,
            default_block_map=default_block_map,
            extra=extra_tensor_aliases or {},
        )
        if canon is None:
            continue
        layer_idx, sub = canon
        arr = dequantize_tensor(t)
        if sub == "embed":
            embed_rows = int(arr.shape[0])
        out_name = sub if layer_idx is None else f"layer[{layer_idx}].{sub}"
        out_tensors[out_name] = _flatten_linear(arr, sub)

    if dims.vocab_size == 0 and embed_rows is not None:
        dims = ArchDims(
            name=dims.name,
            num_layers=dims.num_layers,
            hidden=dims.hidden,
            intermediate=dims.intermediate,
            num_q_heads=dims.num_q_heads,
            num_kv_heads=dims.num_kv_heads,
            head_dim=dims.head_dim,
            head_dim_kv=dims.head_dim_kv,
            vocab_size=embed_rows,
            rope_theta=dims.rope_theta,
            max_position=dims.max_position,
            sliding_window=dims.sliding_window,
            extras=dims.extras,
        )

    return GgufModel(
        arch=dims,
        arch_obj=arch_obj,
        block_kinds=block_kinds,
        tensors=out_tensors,
        feature_flags=arch_obj.feature_flags(),
    )


def _map_tensor_name(
    name: str,
    arch_obj: Architecture,
    reader: gguf.GGUFReader,
    toplevel_map: dict[str, str],
    default_block_map: dict[str, str],
    extra: dict[str, str],
) -> Optional[tuple[Optional[int], str]]:
    # Skip multimodal/aux tensors that aren't text-path consensus
    # weights (vision `v.*`, audio `a.*`, multimodal merger `mm.*`,
    # multi-token prediction `mtp.*`). Architecture modules can opt
    # those in later by adding entries to their alias maps.
    if name.startswith(("v.", "a.", "mm.", "mtp.")):
        return None

    m = re.match(r"^blk\.(\d+)\.(.+)$", name)
    if m is None:
        # Top-level tensor.
        if name in extra:
            return None, extra[name]
        if name in toplevel_map:
            return None, toplevel_map[name]
        return None

    layer_idx = int(m.group(1))
    stem = m.group(2)
    # Per-block extra override (e.g. extra={"blk.0.custom_q.weight": "attn.w_q"}).
    block_key = f"blk.{layer_idx}.{stem}"
    if block_key in extra:
        return layer_idx, extra[block_key]
    # Per-block arch override (used by hybrid models like qwen35).
    overrides = arch_obj.per_block_overrides(reader, layer_idx) or {}
    if stem in overrides:
        return layer_idx, overrides[stem]
    if stem in default_block_map:
        return layer_idx, default_block_map[stem]
    return None


# -----------------------------------------------------------------------------
# CLI.
# -----------------------------------------------------------------------------


def main(argv: Optional[Iterable[str]] = None) -> int:
    import argparse

    p = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    p.add_argument("path", help="Path to .gguf file")
    p.add_argument("--arch", help="Override `general.architecture` lookup")
    p.add_argument("--list-tensors", action="store_true")
    args = p.parse_args(list(argv) if argv is not None else None)
    model = read_model(args.path, arch_override=args.arch)

    print(f"Architecture: {model.arch.name}")
    print(f"  num_layers: {model.arch.num_layers}")
    print(f"  hidden:     {model.arch.hidden}")
    print(f"  vocab:      {model.arch.vocab_size}")
    print(f"  num_q/kv heads: {model.arch.num_q_heads}/{model.arch.num_kv_heads}")
    print(f"  head_dim:   {model.arch.head_dim} (kv={model.arch.head_dim_kv})")
    print(f"  feature_flags: 0x{model.feature_flags:04x}")
    if model.feature_flags:
        active = [f.name for f in Feature if model.feature_flags & f.value]
        print(f"    set: {active}")
    block_summary: dict[BlockKind, int] = {}
    for k in model.block_kinds:
        block_summary[k] = block_summary.get(k, 0) + 1
    print("  block kinds:")
    for k, n in block_summary.items():
        print(f"    {k.name}: {n}")
    print(f"  canonical tensors: {len(model.tensors)}")
    if args.list_tensors:
        for name in sorted(model.tensors.keys()):
            arr = model.tensors[name]
            print(f"    {name}: shape={list(arr.shape)} dtype={arr.dtype}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
