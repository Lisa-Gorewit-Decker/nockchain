"""Generate a small Qwen-shaped model end-to-end through the
quantize → save → load → forward pipeline (Phase 2.9.8).

The model is intentionally tiny (4 layers, hidden=8, vocab=16) so the
whole pipeline runs in a few seconds with zero external downloads.
This fixture is what `tests/oracle_qwen_mini.rs` loads to validate that
Phase 2.9.1 (Model::save/load), Phase 2.9.5 (numpy reference forward),
and the Rust forward driver are all consistent end-to-end.

Layout (mirrors what a real Qwen 3 hybrid would look like at small scale):
- 4 transformer blocks, all Attention flavor for now (DeltaNet adds
  intra-call recurrent state which makes byte-equality more brittle to
  schedule; mini fixture covers the Attention path. Real Qwen hybrid
  patterns extend this once 2.9.5 attention/deltanet match in real-model
  scales).
- hidden=8, num_q_heads=2, num_kv_heads=1, head_dim=4, intermediate=16.
- vocab=16, seq_len=8, activation_tile=2.
- RMSNorm pre-attn and pre-ffn; final RMSNorm.
"""

from __future__ import annotations

import os
import struct
import sys

import blake3
import numpy as np

import forward_reference as F
import reference_ops as R

ROOT = os.path.dirname(os.path.abspath(__file__))
VEC_DIR = os.path.join(ROOT, "test_vectors", "qwen_mini")

# -----------------------------------------------------------------------------
# Manifest writer (mirrors src/io.rs encode_manifest).
# -----------------------------------------------------------------------------

MAGIC = b"AIPOWVI1"
VERSION = 2  # Phase 2.10 — arch_tag + feature_flags follow the version field.


def _u8(v: int) -> bytes:
    return bytes([v & 0xFF])


def _u32(v: int) -> bytes:
    return struct.pack("<I", v & 0xFFFFFFFF)


def _i32(v: int) -> bytes:
    return struct.pack("<i", v)


def _i64(v: int) -> bytes:
    return struct.pack("<q", v)


def _i16(v: int) -> bytes:
    return struct.pack("<h", v)


def encode_norm_meta(norm: F.NormSpec) -> bytes:
    if norm.kind == "rms":
        tag = 0
    elif norm.kind == "ln":
        tag = 1
    else:
        raise ValueError(norm.kind)
    return _u8(tag) + _i64(norm.eps_q) + _i32(norm.post_scale.num)


ACTIVATION_KIND_TAG = {"silu": 1, "gelu": 2, "swish": 3, "identity": 0xFF}


def _arch_tag(name: str) -> bytes:
    """Pad an architecture name to 16 bytes with NULs."""
    b = name.encode()[:16]
    return b + b"\x00" * (16 - len(b))


def encode_manifest(
    dims: F.ModelDims,
    layers: list,
    final_norm: F.NormSpec | None,
    rope_tables: F.RopeTables,
    ffn_kind: str,
    sigmoid_kind: str,
    arch_tag: str = "",
    feature_flags: int = 0,
) -> bytes:
    buf = bytearray()
    buf += MAGIC
    buf += _u32(VERSION)
    buf += _arch_tag(arch_tag)
    buf += struct.pack("<Q", feature_flags & 0xFFFFFFFFFFFFFFFF)
    buf += _u32(dims.vocab) + _u32(dims.hidden) + _u32(dims.seq_len) + _u32(dims.activation_tile)
    buf += _u32(len(layers))
    for layer in layers:
        if isinstance(layer, F.AttentionLayer):
            buf += _u8(0)
            buf += encode_norm_meta(layer.norm1)
            buf += _u32(layer.attn.hidden) + _u32(layer.attn.num_q_heads)
            buf += _u32(layer.attn.num_kv_heads) + _u32(layer.attn.head_dim)
            for s in (
                layer.attn_scales.q,
                layer.attn_scales.k,
                layer.attn_scales.v,
                layer.attn_scales.score,
                layer.attn_scales.attn_out,
                layer.attn_scales.o,
            ):
                buf += _i32(s.num)
            buf += encode_norm_meta(layer.norm2)
            buf += _u32(layer.ffn.intermediate)
            for s in (
                layer.ffn_scales.gate,
                layer.ffn_scales.up,
                layer.ffn_scales.mid,
                layer.ffn_scales.down,
            ):
                buf += _i32(s.num)
        elif isinstance(layer, F.DeltaNetLayer):
            buf += _u8(1)
            buf += encode_norm_meta(layer.norm1)
            buf += _u32(layer.dnet.hidden) + _u32(layer.dnet.num_qk_heads)
            buf += _u32(layer.dnet.num_v_heads)
            buf += _u32(layer.dnet.head_dim_qk) + _u32(layer.dnet.head_dim_v)
            for s in (
                layer.dnet_scales.q,
                layer.dnet_scales.k,
                layer.dnet_scales.v,
                layer.dnet_scales.alpha_logit,
                layer.dnet_scales.beta_logit,
                layer.dnet_scales.u,
                layer.dnet_scales.decay,
                layer.dnet_scales.update,
                layer.dnet_scales.o,
                layer.dnet_scales.proj,
            ):
                buf += _i32(s.num)
            buf += encode_norm_meta(layer.norm2)
            buf += _u32(layer.ffn.intermediate)
            for s in (
                layer.ffn_scales.gate,
                layer.ffn_scales.up,
                layer.ffn_scales.mid,
                layer.ffn_scales.down,
            ):
                buf += _i32(s.num)
        elif isinstance(layer, F.GemmaLayer):
            buf += _u8(2)
            buf += encode_norm_meta(layer.norm1)
            buf += _u32(layer.attn.hidden) + _u32(layer.attn.num_q_heads)
            buf += _u32(layer.attn.num_kv_heads) + _u32(layer.attn.head_dim)
            for s in (
                layer.attn_scales.q,
                layer.attn_scales.k,
                layer.attn_scales.v,
                layer.attn_scales.score,
                layer.attn_scales.attn_out,
                layer.attn_scales.o,
            ):
                buf += _i32(s.num)
            buf += _i64(layer.qk_norm_eps_q)
            buf += _i32(layer.qk_norm_post_scale.num)
            buf += encode_norm_meta(layer.post_attn_norm)
            buf += encode_norm_meta(layer.norm2)
            buf += _u32(layer.ffn.intermediate)
            for s in (
                layer.ffn_scales.gate,
                layer.ffn_scales.up,
                layer.ffn_scales.mid,
                layer.ffn_scales.down,
            ):
                buf += _i32(s.num)
            buf += encode_norm_meta(layer.post_ffn_norm)
            buf += _u32(layer.sliding_window if layer.sliding_window is not None else 0)
            buf += _u8(1 if layer.inp_gate is not None else 0)
            buf += _u8(1 if layer.layer_output_scale is not None else 0)
        else:
            raise TypeError(type(layer))
    if final_norm is not None:
        buf += _u8(1)
        buf += encode_norm_meta(final_norm)
    else:
        buf += _u8(0)
    buf += _u32(rope_tables.seq_len) + _u32(rope_tables.half_head_dim)
    buf += _u8(ACTIVATION_KIND_TAG[ffn_kind])
    buf += _u8(ACTIVATION_KIND_TAG[sigmoid_kind])
    return bytes(buf)


# -----------------------------------------------------------------------------
# Canonical weight stream (mirrors crate::comm_w::canonical_weight_bytes).
# -----------------------------------------------------------------------------


def _i8s(xs) -> bytes:
    return np.array(xs, dtype=np.int8).tobytes()


def encode_weights(model: F.Model) -> bytes:
    buf = bytearray()
    buf += _i8s(model.embed)
    for layer in model.layers:
        # norm1
        buf += _i8s(layer.norm1.gamma)
        if layer.norm1.beta is not None:
            buf += _i8s(layer.norm1.beta)
        if isinstance(layer, F.AttentionLayer):
            buf += _i8s(layer.attn.w_q)
            buf += _i8s(layer.attn.w_k)
            buf += _i8s(layer.attn.w_v)
            buf += _i8s(layer.attn.w_o)
            buf += _i8s(layer.norm2.gamma)
            if layer.norm2.beta is not None:
                buf += _i8s(layer.norm2.beta)
            buf += _i8s(layer.ffn.w_gate)
            buf += _i8s(layer.ffn.w_up)
            buf += _i8s(layer.ffn.w_down)
        elif isinstance(layer, F.DeltaNetLayer):
            buf += _i8s(layer.dnet.w_q)
            buf += _i8s(layer.dnet.w_k)
            buf += _i8s(layer.dnet.w_v)
            buf += _i8s(layer.dnet.w_alpha)
            buf += _i8s(layer.dnet.w_beta)
            buf += _i8s(layer.dnet.w_o)
            buf += _i8s(layer.norm2.gamma)
            if layer.norm2.beta is not None:
                buf += _i8s(layer.norm2.beta)
            buf += _i8s(layer.ffn.w_gate)
            buf += _i8s(layer.ffn.w_up)
            buf += _i8s(layer.ffn.w_down)
        elif isinstance(layer, F.GemmaLayer):
            # Order mirrors crate::comm_w::append_layer_weights Gemma arm:
            # attn(q,k,v,o) → q_norm gamma → k_norm gamma → post_attn_norm →
            # norm2 → ffn(gate,up,down) → post_ffn_norm → [inp_gate] →
            # [layer_output_scale].
            buf += _i8s(layer.attn.w_q)
            buf += _i8s(layer.attn.w_k)
            buf += _i8s(layer.attn.w_v)
            buf += _i8s(layer.attn.w_o)
            buf += _i8s(layer.q_norm_gamma)
            buf += _i8s(layer.k_norm_gamma)
            buf += _i8s(layer.post_attn_norm.gamma)
            if layer.post_attn_norm.beta is not None:
                buf += _i8s(layer.post_attn_norm.beta)
            buf += _i8s(layer.norm2.gamma)
            if layer.norm2.beta is not None:
                buf += _i8s(layer.norm2.beta)
            buf += _i8s(layer.ffn.w_gate)
            buf += _i8s(layer.ffn.w_up)
            buf += _i8s(layer.ffn.w_down)
            buf += _i8s(layer.post_ffn_norm.gamma)
            if layer.post_ffn_norm.beta is not None:
                buf += _i8s(layer.post_ffn_norm.beta)
            if layer.inp_gate is not None:
                buf += _i8s(layer.inp_gate)
            if layer.layer_output_scale is not None:
                buf += _i8s(layer.layer_output_scale)
        else:
            raise TypeError(type(layer))
    if model.final_norm is not None:
        buf += _i8s(model.final_norm.gamma)
        if model.final_norm.beta is not None:
            buf += _i8s(model.final_norm.beta)
    # ffn_activation LUT (256 i8 bytes), sigmoid LUT (256 i8 bytes),
    # softmax LUT (256 LE i32), RoPE tables.
    buf += bytes((b & 0xFF for b in model.ffn_activation_bytes))
    buf += bytes((b & 0xFF for b in model.sigmoid_lut_bytes))
    buf += np.array(model.softmax_lut.table, dtype=np.int32).tobytes()
    buf += _u32(model.rope_tables.seq_len) + _u32(model.rope_tables.half_head_dim)
    buf += np.array(model.rope_tables.cos, dtype=np.int16).tobytes()
    buf += np.array(model.rope_tables.sin, dtype=np.int16).tobytes()
    return bytes(buf)


# -----------------------------------------------------------------------------
# comm_W (mirrors crate::comm_w::compute_comm_w).
# -----------------------------------------------------------------------------

CTX_COMM_W = "ai-pow-vi v1 comm-w"
CTX_WEIGHT_TILE = "ai-pow-vi v1 weight-tile"
CTX_MANIFEST = "ai-pow-vi v1 manifest"
CTX_LEAF = "ai-pow v1 merkle-leaf"
CTX_NODE = "ai-pow v1 merkle-node"
CTX_SENTINEL = "ai-pow v1 merkle-sentinel"

WEIGHT_TILE_BYTES = 64


def tile_hash(chunk: bytes) -> bytes:
    h = blake3.blake3(derive_key_context=CTX_WEIGHT_TILE)
    h.update(struct.pack("<Q", len(chunk)))
    h.update(chunk)
    return h.digest(length=32)


def merkle_leaf_hash(leaf: bytes) -> bytes:
    h = blake3.blake3(derive_key_context=CTX_LEAF)
    h.update(leaf)
    return h.digest(length=32)


def merkle_node_hash(left: bytes, right: bytes) -> bytes:
    h = blake3.blake3(derive_key_context=CTX_NODE)
    h.update(left)
    h.update(right)
    return h.digest(length=32)


def sentinel_leaf() -> bytes:
    h = blake3.blake3(derive_key_context=CTX_SENTINEL)
    return h.digest(length=32)


def merkle_root(leaves: list[bytes]) -> bytes:
    if not leaves:
        raise ValueError("empty leaves")
    n = len(leaves)
    padded = 1
    while padded < n:
        padded *= 2
    layer = [merkle_leaf_hash(leaf) for leaf in leaves]
    sent = sentinel_leaf()
    while len(layer) < padded:
        layer.append(sent)
    while len(layer) > 1:
        new = []
        for i in range(0, len(layer), 2):
            new.append(merkle_node_hash(layer[i], layer[i + 1]))
        layer = new
    return layer[0]


def weights_merkle_root(buf: bytes) -> bytes:
    n_full = len(buf) // WEIGHT_TILE_BYTES
    rem = len(buf) % WEIGHT_TILE_BYTES
    leaves = []
    for i in range(n_full):
        leaves.append(tile_hash(buf[i * WEIGHT_TILE_BYTES : (i + 1) * WEIGHT_TILE_BYTES]))
    if rem != 0:
        last = bytearray(WEIGHT_TILE_BYTES)
        last[:rem] = buf[n_full * WEIGHT_TILE_BYTES :]
        leaves.append(tile_hash(bytes(last)))
    if not leaves:
        leaves.append(tile_hash(b"\x00" * WEIGHT_TILE_BYTES))
    return merkle_root(leaves)


def append_norm_meta_for_manifest_hash(buf: bytearray, norm: F.NormSpec) -> None:
    if norm.kind == "rms":
        buf += _u8(0)
    else:
        buf += _u8(1)
    buf += _i64(norm.eps_q)
    buf += _i32(norm.post_scale.num)


def manifest_hash_bytes(model: F.Model, arch_tag: str = "", feature_flags: int = 0) -> bytes:
    """Mirrors crate::comm_w::manifest_hash."""
    buf = bytearray()
    # Phase 2.10: arch_tag + feature_flags first.
    buf += _arch_tag(arch_tag)
    buf += struct.pack("<Q", feature_flags & 0xFFFFFFFFFFFFFFFF)
    d = model.dims
    buf += _u32(d.vocab) + _u32(d.hidden) + _u32(d.seq_len) + _u32(d.activation_tile)
    buf += _u32(len(model.layers))
    for layer in model.layers:
        if isinstance(layer, F.AttentionLayer):
            buf += _u8(0)
            append_norm_meta_for_manifest_hash(buf, layer.norm1)
            buf += _u32(layer.attn.hidden) + _u32(layer.attn.num_q_heads)
            buf += _u32(layer.attn.num_kv_heads) + _u32(layer.attn.head_dim)
            for s in (
                layer.attn_scales.q,
                layer.attn_scales.k,
                layer.attn_scales.v,
                layer.attn_scales.score,
                layer.attn_scales.attn_out,
                layer.attn_scales.o,
            ):
                buf += _i32(s.num)
            append_norm_meta_for_manifest_hash(buf, layer.norm2)
            buf += _u32(layer.ffn.hidden) + _u32(layer.ffn.intermediate)
            for s in (
                layer.ffn_scales.gate,
                layer.ffn_scales.up,
                layer.ffn_scales.mid,
                layer.ffn_scales.down,
            ):
                buf += _i32(s.num)
        elif isinstance(layer, F.DeltaNetLayer):
            buf += _u8(1)
            append_norm_meta_for_manifest_hash(buf, layer.norm1)
            buf += _u32(layer.dnet.hidden) + _u32(layer.dnet.num_qk_heads)
            buf += _u32(layer.dnet.num_v_heads)
            buf += _u32(layer.dnet.head_dim_qk) + _u32(layer.dnet.head_dim_v)
            for s in (
                layer.dnet_scales.q,
                layer.dnet_scales.k,
                layer.dnet_scales.v,
                layer.dnet_scales.alpha_logit,
                layer.dnet_scales.beta_logit,
                layer.dnet_scales.u,
                layer.dnet_scales.decay,
                layer.dnet_scales.update,
                layer.dnet_scales.o,
                layer.dnet_scales.proj,
            ):
                buf += _i32(s.num)
            append_norm_meta_for_manifest_hash(buf, layer.norm2)
            buf += _u32(layer.ffn.hidden) + _u32(layer.ffn.intermediate)
            for s in (
                layer.ffn_scales.gate,
                layer.ffn_scales.up,
                layer.ffn_scales.mid,
                layer.ffn_scales.down,
            ):
                buf += _i32(s.num)
        elif isinstance(layer, F.GemmaLayer):
            buf += _u8(2)
            append_norm_meta_for_manifest_hash(buf, layer.norm1)
            buf += _u32(layer.attn.hidden) + _u32(layer.attn.num_q_heads)
            buf += _u32(layer.attn.num_kv_heads) + _u32(layer.attn.head_dim)
            for s in (
                layer.attn_scales.q,
                layer.attn_scales.k,
                layer.attn_scales.v,
                layer.attn_scales.score,
                layer.attn_scales.attn_out,
                layer.attn_scales.o,
            ):
                buf += _i32(s.num)
            buf += _i64(layer.qk_norm_eps_q)
            buf += _i32(layer.qk_norm_post_scale.num)
            append_norm_meta_for_manifest_hash(buf, layer.post_attn_norm)
            append_norm_meta_for_manifest_hash(buf, layer.norm2)
            buf += _u32(layer.ffn.hidden) + _u32(layer.ffn.intermediate)
            for s in (
                layer.ffn_scales.gate,
                layer.ffn_scales.up,
                layer.ffn_scales.mid,
                layer.ffn_scales.down,
            ):
                buf += _i32(s.num)
            append_norm_meta_for_manifest_hash(buf, layer.post_ffn_norm)
            buf += _u32(layer.sliding_window if layer.sliding_window is not None else 0)
            buf += _u8(1 if layer.inp_gate is not None else 0)
            buf += _u8(1 if layer.layer_output_scale is not None else 0)
        else:
            raise TypeError(type(layer))
    if model.final_norm is not None:
        buf += _u8(1)
        append_norm_meta_for_manifest_hash(buf, model.final_norm)
    else:
        buf += _u8(0)
    h = blake3.blake3(derive_key_context=CTX_MANIFEST)
    h.update(struct.pack("<Q", len(buf)))
    h.update(bytes(buf))
    return h.digest(length=32)


def compute_comm_w(
    model: F.Model, arch_tag: str = "", feature_flags: int = 0
) -> bytes:
    weights = encode_weights(model)
    weights_root = weights_merkle_root(weights)
    manifest = manifest_hash_bytes(model, arch_tag=arch_tag, feature_flags=feature_flags)
    h = blake3.blake3(derive_key_context=CTX_COMM_W)
    h.update(weights_root)
    h.update(manifest)
    return h.digest(length=32)


# -----------------------------------------------------------------------------
# Build a tiny synthetic Qwen-shaped model.
# -----------------------------------------------------------------------------


def small_scale() -> R.Scale:
    return R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 4))


def build_attn_layer(hidden: int, seed: int) -> F.AttentionLayer:
    s = small_scale()
    hu = hidden
    return F.AttentionLayer(
        norm1=F.NormSpec(
            kind="rms",
            gamma=tuple(R.canonical_input_i8(hu, seed)),
            beta=None,
            eps_q=1,
            post_scale=s,
        ),
        attn=F.AttentionWeights(
            hidden=hidden,
            num_q_heads=2,
            num_kv_heads=1,
            head_dim=4,
            w_q=tuple(R.canonical_input_i8(hu * 2 * 4, seed + 1)),
            w_k=tuple(R.canonical_input_i8(hu * 1 * 4, seed + 2)),
            w_v=tuple(R.canonical_input_i8(hu * 1 * 4, seed + 3)),
            w_o=tuple(R.canonical_input_i8(2 * 4 * hu, seed + 4)),
        ),
        attn_scales=F.AttentionScales(q=s, k=s, v=s, score=s, attn_out=s, o=s),
        norm2=F.NormSpec(
            kind="rms",
            gamma=tuple(R.canonical_input_i8(hu, seed + 5)),
            beta=None,
            eps_q=1,
            post_scale=s,
        ),
        ffn=F.FfnWeights(
            hidden=hidden,
            intermediate=hidden * 2,
            w_gate=tuple(R.canonical_input_i8(hu * (hu * 2), seed + 6)),
            w_up=tuple(R.canonical_input_i8(hu * (hu * 2), seed + 7)),
            w_down=tuple(R.canonical_input_i8((hu * 2) * hu, seed + 8)),
        ),
        ffn_scales=R.FfnScales(gate=s, up=s, mid=s, down=s),
    )


def build_qwen_mini() -> F.Model:
    hidden = 8
    seq_len = 8
    layers = tuple(build_attn_layer(hidden, seed=0x100 * (i + 1)) for i in range(4))
    s = small_scale()
    final_norm = F.NormSpec(
        kind="rms",
        gamma=tuple(R.canonical_input_i8(hidden, 0xAAAA)),
        beta=None,
        eps_q=1,
        post_scale=s,
    )
    rope = F.RopeTables.identity(seq_len, half_head_dim=2)
    # Identity ffn_activation LUT (i8 reinterpretation of byte b is b - 128).
    ffn_lut_bytes = tuple(((i - 128) & 0xFF) for i in range(256))
    sig_lut_bytes = tuple(((i - 128) & 0xFF) for i in range(256))
    softmax_lut = R.ExpLut(table=tuple(1 << 16 for _ in range(256)))
    return F.Model(
        dims=F.ModelDims(vocab=16, hidden=hidden, seq_len=seq_len, activation_tile=2),
        embed=tuple(R.canonical_input_i8(16 * hidden, 0xBEEF_FEED)),
        layers=layers,
        final_norm=final_norm,
        rope_tables=rope,
        softmax_lut=softmax_lut,
        sigmoid_lut_bytes=sig_lut_bytes,
        ffn_activation_bytes=ffn_lut_bytes,
    )


def main() -> int:
    os.makedirs(VEC_DIR, exist_ok=True)
    model = build_qwen_mini()

    # Compute comm_W, write the disk format.
    comm_w = compute_comm_w(model)
    manifest_bytes = encode_manifest(
        model.dims,
        list(model.layers),
        model.final_norm,
        model.rope_tables,
        ffn_kind="identity",
        sigmoid_kind="identity",
    )
    weights_bytes = encode_weights(model)
    with open(os.path.join(VEC_DIR, "manifest.bin"), "wb") as f:
        f.write(manifest_bytes)
    with open(os.path.join(VEC_DIR, "weights.bin"), "wb") as f:
        f.write(weights_bytes)
    with open(os.path.join(VEC_DIR, "comm_w.hex"), "w") as f:
        f.write(comm_w.hex())

    # Run a forward_prefix to layer 2 over a fixed prompt; dump output.
    prompt = [1, 5, 9, 0]
    output, log = F.forward_prefix(model, prompt, target_layer=2)

    # Save prompt and reference output.
    np.array(prompt, dtype=np.uint32).tofile(os.path.join(VEC_DIR, "prompt.bin"))
    np.array(output, dtype=np.int8).tofile(
        os.path.join(VEC_DIR, "forward_layer_2_output.bin")
    )
    # Save per-layer recorded full-seq tensors (for activation log root checks).
    for i, layer_tensor in enumerate(log):
        np.array(layer_tensor, dtype=np.int8).tofile(
            os.path.join(VEC_DIR, f"activation_layer_{i}.bin")
        )

    with open(os.path.join(VEC_DIR, "meta.txt"), "w") as f:
        f.write(
            f"vocab={model.dims.vocab} hidden={model.dims.hidden} "
            f"seq_len={model.dims.seq_len} activation_tile={model.dims.activation_tile} "
            f"num_layers={len(model.layers)} target_layer=2 prompt_len={len(prompt)}\n"
        )
    print(
        f"qwen_mini: comm_W={comm_w.hex()[:16]}... manifest={len(manifest_bytes)}B "
        f"weights={len(weights_bytes)}B output_len={len(output)}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
