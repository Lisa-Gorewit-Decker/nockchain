"""Phase 2.9.4 — INT8 quantizer driver.

Take f32 weights from `gguf_reader.py`, scales from `calibrate.py`,
and emit a sideloaded model directory consumable by `Model::load`.

Flow:
1. Read GGUF → f32 tensors via `gguf_reader.read_model`.
2. Read scales JSON via `calibrate` (or directly).
3. Per-tensor quantize: `w_int8 = round(w / s_w).clamp(-128, 127)`.
4. Build LUTs: SiLU for FFN activation, sigmoid for DeltaNet α/β,
   uniform-test exp LUT for softmax (real models would use a calibrated
   exp curve; uniform-test makes the bring-up testable without further
   tuning).
5. Build RoPE tables in i16 fixed-point at FRACT_BITS=14, base=rope_theta.
6. Assemble `forward_reference.Model`, call `synthetic_qwen_mini.encode_manifest`
   + `encode_weights` to write `manifest.bin` + `weights.bin` + `comm_w.hex`.

Acceptance: the emitted directory loads via Rust `Model::load(dir, &comm_w)`
without error. (Verified by `tests/oracle_qwen.rs` and indirectly by
`oracle/tests/test_quantize_qwen.py`.)
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from typing import Iterable, Optional

import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import calibrate as C  # noqa: E402
import forward_reference as F  # noqa: E402
import gguf_reader as G  # noqa: E402
import reference_ops as R  # noqa: E402
import synthetic_qwen_mini as D  # noqa: E402  -- disk format encoders


# -----------------------------------------------------------------------------
# Quantization primitive.
# -----------------------------------------------------------------------------


def quantize_tensor(arr: np.ndarray, scale_f32: float) -> tuple[int, ...]:
    """Quantize an f32 tensor to a flat tuple of i8 values via
    `round(w / s).clamp(-128, 127)`. The flat layout matches whatever
    layout the input array has (caller is responsible for ravel/reshape)."""
    if scale_f32 <= 0:
        scale_f32 = 1.0 / 127.0
    arr_f32 = np.asarray(arr, dtype=np.float32)
    q = np.round(arr_f32 / scale_f32).astype(np.int64)
    q = np.clip(q, -128, 127).astype(np.int8)
    return tuple(int(v) for v in q.ravel())


def scale_num_to_f32(num: int) -> float:
    return num / float(1 << R.SCALE_DENOM_LOG2)


# -----------------------------------------------------------------------------
# LUT and RoPE builders.
# -----------------------------------------------------------------------------


def build_silu_lut(scale_x_f32: float = 1.0 / 8.0) -> tuple[int, ...]:
    """SiLU(x) = x * sigmoid(x), tabulated for i8 input. Each input byte
    `b` maps to value `f32 = (b - 128) * scale_x_f32`; output is
    `clip(round(silu(f32) / scale_y), -128, 127)` with the same scale_y
    chosen so SiLU(2.5) ≈ 2.5 (saturates near i8 range).

    Default `scale_x_f32 = 1/8` means input range is [-16, ~16) — a
    reasonable post-norm activation range. The LUT bytes are committed
    inside comm_W; calibrating to a different scale is a future
    tightening."""
    table_bytes = bytearray(256)
    scale_y = scale_x_f32  # so identity-y just gives back scaled silu
    for i in range(256):
        x = (i - 128) * scale_x_f32
        sig = 1.0 / (1.0 + math.exp(-x))
        y = x * sig
        q = round(y / scale_y)
        q = max(-128, min(127, q))
        table_bytes[i] = q & 0xFF
    return tuple(int(b) for b in table_bytes)


def build_sigmoid_lut(scale_x_f32: float = 1.0 / 8.0) -> tuple[int, ...]:
    """Sigmoid(x) tabulated as i8 in [0, 127] (representing [0, 1]).

    Used by DeltaNet for the α / β gates."""
    table_bytes = bytearray(256)
    for i in range(256):
        x = (i - 128) * scale_x_f32
        sig = 1.0 / (1.0 + math.exp(-x))
        q = round(sig * 127.0)
        q = max(0, min(127, q))
        table_bytes[i] = q & 0xFF
    return tuple(int(b) for b in table_bytes)


def build_softmax_exp_lut() -> R.ExpLut:
    """Use the uniform-test ExpLut for now (every entry = 2^16). Real
    models would use a calibrated `exp(-step * idx)` curve; uniform-test
    keeps the bring-up testable end-to-end without exp calibration. The
    LUT bytes are part of comm_W so this is a model-time choice."""
    return R.ExpLut.uniform_test()


def build_rope_tables(
    seq_len: int, head_dim: int, rope_theta: float
) -> F.RopeTables:
    """Standard RoPE cos/sin table generation in INT16 at FRACT_BITS=14.

    For pair index j in [0, head_dim/2): inv_freq_j = rope_theta^{-2j/head_dim}.
    For each position pos in [0, seq_len): theta = pos * inv_freq_j.
    cos/sin tables have shape (seq_len, head_dim/2)."""
    if head_dim % 2 != 0:
        raise ValueError("head_dim must be even for RoPE")
    half = head_dim // 2
    cos: list[int] = []
    sin: list[int] = []
    fract = 1 << F.ROPE_FRACT_BITS
    for pos in range(seq_len):
        for j in range(half):
            inv_freq = 1.0 / (rope_theta ** (2.0 * j / head_dim))
            theta = pos * inv_freq
            c = round(math.cos(theta) * fract)
            s = round(math.sin(theta) * fract)
            c = max(-32768, min(32767, c))
            s = max(-32768, min(32767, s))
            cos.append(c)
            sin.append(s)
    return F.RopeTables(seq_len=seq_len, half_head_dim=half, cos=tuple(cos), sin=tuple(sin))


# -----------------------------------------------------------------------------
# Layer assembly.
# -----------------------------------------------------------------------------


def _ws(scales: dict, key: str) -> int:
    """Look up a weight scale by canonical name; default to a small
    positive scale if missing (so `Scale::from_num` doesn't fail)."""
    n = scales["weight_scales"].get(key)
    return int(n) if n is not None else 1


def _as(scales: dict, key: str) -> int:
    """Look up an activation scale; fall back to `default`."""
    s = scales["activation_scales"]
    return int(s.get(key, s.get("default", 1)))


def has_layer_tensor(model: G.GgufModel, layer_idx: int, sub: str) -> bool:
    return f"layer[{layer_idx}].{sub}" in model.tensors


def get_layer_tensor(model: G.GgufModel, layer_idx: int, sub: str) -> np.ndarray:
    return model.tensors[f"layer[{layer_idx}].{sub}"]


def quantize_norm(
    model: G.GgufModel, layer_idx: int, scales: dict, slot: str
) -> F.NormSpec:
    """`slot` is "norm1" or "norm2"; build a NormSpec from gamma (and
    beta if present)."""
    gamma_arr = get_layer_tensor(model, layer_idx, f"{slot}.gamma")
    gamma_scale = _ws(scales, f"layer[{layer_idx}].{slot}.gamma")
    gamma = quantize_tensor(gamma_arr, scale_num_to_f32(gamma_scale))
    beta_key = f"{slot}.beta"
    if has_layer_tensor(model, layer_idx, beta_key):
        beta_arr = get_layer_tensor(model, layer_idx, beta_key)
        beta_scale = _ws(scales, f"layer[{layer_idx}].{slot}.{ 'beta' }")
        beta = quantize_tensor(beta_arr, scale_num_to_f32(beta_scale))
        return F.NormSpec(
            kind="ln",
            gamma=gamma,
            beta=beta,
            eps_q=int(scales.get("norm_eps_q", 1)),
            post_scale=R.Scale(num=_as(scales, f"layer[{layer_idx}].norm_post.{slot[-1]}")),
        )
    return F.NormSpec(
        kind="rms",
        gamma=gamma,
        beta=None,
        eps_q=int(scales.get("norm_eps_q", 1)),
        post_scale=R.Scale(num=_as(scales, f"layer[{layer_idx}].norm_post.{slot[-1]}")),
    )


def build_attn_layer(model: G.GgufModel, layer_idx: int, scales: dict) -> F.AttentionLayer:
    a = scales
    arch = model.arch
    norm1 = quantize_norm(model, layer_idx, scales, "norm1")
    norm2 = quantize_norm(model, layer_idx, scales, "norm2")

    def q_weight(name: str) -> tuple[int, ...]:
        scale_num = _ws(scales, f"layer[{layer_idx}].{name}")
        return quantize_tensor(
            get_layer_tensor(model, layer_idx, name), scale_num_to_f32(scale_num)
        )

    attn = F.AttentionWeights(
        hidden=arch.hidden,
        num_q_heads=arch.num_q_heads,
        num_kv_heads=arch.num_kv_heads,
        head_dim=arch.head_dim,
        w_q=q_weight("attn.w_q"),
        w_k=q_weight("attn.w_k"),
        w_v=q_weight("attn.w_v"),
        w_o=q_weight("attn.w_o"),
    )
    attn_scales = F.AttentionScales(
        q=R.Scale(num=_as(a, f"layer[{layer_idx}].attn.q")),
        k=R.Scale(num=_as(a, f"layer[{layer_idx}].attn.k")),
        v=R.Scale(num=_as(a, f"layer[{layer_idx}].attn.v")),
        score=R.Scale(num=_as(a, f"layer[{layer_idx}].attn.score")),
        attn_out=R.Scale(num=_as(a, f"layer[{layer_idx}].attn.attn_out")),
        o=R.Scale(num=_as(a, f"layer[{layer_idx}].attn.o")),
    )
    ffn = F.FfnWeights(
        hidden=arch.hidden,
        intermediate=arch.intermediate,
        w_gate=q_weight("ffn.w_gate"),
        w_up=q_weight("ffn.w_up"),
        w_down=q_weight("ffn.w_down"),
    )
    ffn_scales = R.FfnScales(
        gate=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.gate")),
        up=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.up")),
        mid=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.mid")),
        down=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.down")),
    )
    return F.AttentionLayer(
        norm1=norm1,
        attn=attn,
        attn_scales=attn_scales,
        norm2=norm2,
        ffn=ffn,
        ffn_scales=ffn_scales,
    )


def is_deltanet_layer(model: G.GgufModel, layer_idx: int) -> bool:
    return has_layer_tensor(model, layer_idx, "dnet.w_q")


def build_deltanet_layer(
    model: G.GgufModel, layer_idx: int, scales: dict, num_qk_heads: int, num_v_heads: int
) -> F.DeltaNetLayer:
    a = scales
    arch = model.arch
    norm1 = quantize_norm(model, layer_idx, scales, "norm1")
    norm2 = quantize_norm(model, layer_idx, scales, "norm2")

    def q_weight(name: str) -> tuple[int, ...]:
        scale_num = _ws(scales, f"layer[{layer_idx}].{name}")
        return quantize_tensor(
            get_layer_tensor(model, layer_idx, name), scale_num_to_f32(scale_num)
        )

    dnet = F.DeltaNetWeights(
        hidden=arch.hidden,
        num_qk_heads=num_qk_heads,
        num_v_heads=num_v_heads,
        head_dim_qk=arch.head_dim,
        head_dim_v=arch.head_dim,
        w_q=q_weight("dnet.w_q"),
        w_k=q_weight("dnet.w_k"),
        w_v=q_weight("dnet.w_v"),
        w_alpha=q_weight("dnet.w_alpha"),
        w_beta=q_weight("dnet.w_beta"),
        w_o=q_weight("dnet.w_o"),
    )
    dnet_scales = F.DeltaNetScales(
        q=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.q")),
        k=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.k")),
        v=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.v")),
        alpha_logit=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.alpha_logit")),
        beta_logit=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.beta_logit")),
        u=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.u")),
        decay=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.decay")),
        update=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.update")),
        o=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.o")),
        proj=R.Scale(num=_as(a, f"layer[{layer_idx}].dnet.proj")),
    )
    ffn = F.FfnWeights(
        hidden=arch.hidden,
        intermediate=arch.intermediate,
        w_gate=q_weight("ffn.w_gate"),
        w_up=q_weight("ffn.w_up"),
        w_down=q_weight("ffn.w_down"),
    )
    ffn_scales = R.FfnScales(
        gate=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.gate")),
        up=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.up")),
        mid=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.mid")),
        down=R.Scale(num=_as(a, f"layer[{layer_idx}].ffn.down")),
    )
    return F.DeltaNetLayer(
        norm1=norm1,
        dnet=dnet,
        dnet_scales=dnet_scales,
        norm2=norm2,
        ffn=ffn,
        ffn_scales=ffn_scales,
    )


# -----------------------------------------------------------------------------
# Top-level driver.
# -----------------------------------------------------------------------------


def quantize_to_model(
    gguf_model: G.GgufModel,
    scales: dict,
    seq_len: int,
    activation_tile: int = 64,
    deltanet_num_qk_heads: Optional[int] = None,
    deltanet_num_v_heads: Optional[int] = None,
) -> tuple[F.Model, bytes]:
    """Build a `forward_reference.Model` from quantized weights + scales.
    Returns (model, comm_w_bytes)."""
    arch = gguf_model.arch

    if seq_len > arch.max_position:
        raise ValueError(f"seq_len {seq_len} exceeds model context {arch.max_position}")
    if arch.hidden % activation_tile != 0:
        raise ValueError(
            f"hidden ({arch.hidden}) not divisible by activation_tile ({activation_tile})"
        )
    if seq_len % activation_tile != 0:
        raise ValueError(
            f"seq_len ({seq_len}) not divisible by activation_tile ({activation_tile})"
        )

    layers: list = []
    for n in range(arch.num_layers):
        if is_deltanet_layer(gguf_model, n):
            layers.append(
                build_deltanet_layer(
                    gguf_model,
                    n,
                    scales,
                    deltanet_num_qk_heads or arch.num_q_heads,
                    deltanet_num_v_heads or arch.num_q_heads,
                )
            )
        else:
            layers.append(build_attn_layer(gguf_model, n, scales))

    # Embed (kept (vocab, hidden) row-major flat).
    embed_arr = gguf_model.tensors["embed"]
    embed_scale = _ws(scales, "embed")
    embed = quantize_tensor(embed_arr, scale_num_to_f32(embed_scale))

    # Final norm.
    final_norm = None
    if "final_norm.gamma" in gguf_model.tensors:
        gamma_arr = gguf_model.tensors["final_norm.gamma"]
        gamma_scale = _ws(scales, "final_norm.gamma")
        gamma = quantize_tensor(gamma_arr, scale_num_to_f32(gamma_scale))
        final_norm_post = R.Scale(num=_as(scales, f"final_norm_post"))
        if "final_norm.beta" in gguf_model.tensors:
            beta_arr = gguf_model.tensors["final_norm.beta"]
            beta_scale = _ws(scales, "final_norm.beta")
            beta = quantize_tensor(beta_arr, scale_num_to_f32(beta_scale))
            final_norm = F.NormSpec(
                kind="ln",
                gamma=gamma,
                beta=beta,
                eps_q=int(scales.get("norm_eps_q", 1)),
                post_scale=final_norm_post,
            )
        else:
            final_norm = F.NormSpec(
                kind="rms",
                gamma=gamma,
                beta=None,
                eps_q=int(scales.get("norm_eps_q", 1)),
                post_scale=final_norm_post,
            )

    rope_tables = build_rope_tables(seq_len, arch.head_dim, arch.rope_theta)
    softmax_lut = build_softmax_exp_lut()
    sigmoid_lut_bytes = build_sigmoid_lut()
    ffn_activation_bytes = build_silu_lut()

    model = F.Model(
        dims=F.ModelDims(
            vocab=arch.vocab_size,
            hidden=arch.hidden,
            seq_len=seq_len,
            activation_tile=activation_tile,
        ),
        embed=embed,
        layers=tuple(layers),
        final_norm=final_norm,
        rope_tables=rope_tables,
        softmax_lut=softmax_lut,
        sigmoid_lut_bytes=sigmoid_lut_bytes,
        ffn_activation_bytes=ffn_activation_bytes,
    )
    comm_w = D.compute_comm_w(model)
    return model, comm_w


def save(
    model: F.Model,
    comm_w: bytes,
    out_dir: str,
    lm_head: Optional[tuple[int, ...]] = None,
) -> None:
    os.makedirs(out_dir, exist_ok=True)
    manifest = D.encode_manifest(
        model.dims,
        list(model.layers),
        model.final_norm,
        model.rope_tables,
        ffn_kind="silu",
        sigmoid_kind="silu",  # tag is metadata; the bytes themselves are sigmoid
    )
    weights = D.encode_weights(model)
    with open(os.path.join(out_dir, "manifest.bin"), "wb") as f:
        f.write(manifest)
    with open(os.path.join(out_dir, "weights.bin"), "wb") as f:
        f.write(weights)
    with open(os.path.join(out_dir, "comm_w.hex"), "w") as f:
        f.write(comm_w.hex())
    # lm_head lives outside the consensus-pinned weights (Phase 2.7
    # postpones lm_head consensus to a later tightening). Save it as a
    # sibling file so `qwen-eval` can use it for top-1 evaluation.
    if lm_head is not None:
        np.array(lm_head, dtype=np.int8).tofile(os.path.join(out_dir, "lm_head.bin"))


def main(argv: Optional[Iterable[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    p.add_argument("--gguf", required=True)
    p.add_argument("--scales", required=True, help="path to scales.json from calibrate.py")
    p.add_argument("--out", required=True, help="output directory")
    p.add_argument("--seq-len", type=int, default=64)
    p.add_argument("--activation-tile", type=int, default=64)
    p.add_argument("--arch", help="architecture prefix override")
    args = p.parse_args(list(argv) if argv is not None else None)

    scales = json.loads(open(args.scales).read())
    gguf_model = G.read_model(args.gguf, arch_override=args.arch)
    model, comm_w = quantize_to_model(
        gguf_model,
        scales,
        seq_len=args.seq_len,
        activation_tile=args.activation_tile,
    )
    lm_head: Optional[tuple[int, ...]] = None
    if "lm_head" in gguf_model.tensors:
        lm_scale = _ws(scales, "lm_head")
        lm_head = quantize_tensor(gguf_model.tensors["lm_head"], scale_num_to_f32(lm_scale))
    save(model, comm_w, args.out, lm_head=lm_head)
    print(
        f"wrote {args.out}: comm_W={comm_w.hex()[:16]}... "
        f"(layers={len(model.layers)}, hidden={model.dims.hidden}, seq_len={model.dims.seq_len})",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
