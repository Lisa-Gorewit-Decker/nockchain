"""Phase 2.9.3 — quantization-scale calibration.

Two scale derivation modes are supported:

1. **Static (`--mode static`, default).** Per-tensor weight scale is
   `s_w = max(|w|) / 127` for every linear weight tensor. Activation
   scales for each quantization point fall back to a configurable
   heuristic (default: post-RMSNorm activations have unit-ish std after
   normalization, so `s_a = 1/127` is a reasonable starting point and
   the user can tune from there).

2. **Activation (`--mode activation --prompts <file>`).** Runs a numpy
   f32 forward over a calibration prompt set (one int-per-line tokens,
   or `--vocab-random N` for a randomly-sampled set), tracks
   `max(|act|)` at every quantization point per layer, and writes a
   per-(layer, op) `s_a = max_abs / 127` into the scales JSON. This
   path requires a numpy f32 forward which is not yet implemented in
   `oracle/forward_reference.py` — for Phase 2.9.3 it falls back to
   static plus a documented stub.

Output schema (JSON):

```json
{
  "model_arch": "qwen3",
  "mode": "static",
  "weight_scales": {
    "embed":        <int scale_num>,
    "layer[0].attn.w_q": <int>,
    ...
  },
  "activation_scales": {
    "default":      <int>,
    "layer[0].attn.q":        <int>,
    "layer[0].attn.k":        <int>,
    ...
  },
  "norm_eps_q": 1
}
```

Numerators are integers in `Scale` (denom = 2^15) units. The quantizer
(`oracle/quantize_qwen.py`) consumes this JSON directly.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Iterable, Optional

import numpy as np

# Allow running from any directory.
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import gguf_reader as G  # noqa: E402
import reference_ops as R  # noqa: E402

SCALE_DENOM = 1 << R.SCALE_DENOM_LOG2  # 2^15


def f32_to_scale_num(scale: float) -> int:
    """Convert a positive f32 scale to a `Scale` numerator (i32 / 2^15).
    Clamps below to 1 and above to i32::MAX so the result is always a
    valid `Scale::from_num` argument."""
    if not np.isfinite(scale) or scale <= 0:
        return 1
    raw = round(scale * SCALE_DENOM)
    return max(1, min(raw, R.I32_MAX))


def derive_weight_scale(tensor: np.ndarray) -> float:
    """Per-tensor symmetric INT8 scale: s = max(|w|) / 127.

    Returns a positive f32. For all-zero tensors, returns 1/127 to keep
    the numerator from collapsing to zero. This is harmless because the
    int8 weights themselves are also zero and the scale just multiplies
    out to zero."""
    mx = float(np.max(np.abs(tensor)))
    if mx == 0.0:
        return 1.0 / 127.0
    return mx / 127.0


# Names of the quantization points we track per layer. Each maps to a
# `Scale` in either `crate::attention::AttentionScales`, ::ffn::FfnScales,
# or `crate::deltanet::DeltaNetScales`.
ATTENTION_QUANT_POINTS = ("q", "k", "v", "score", "attn_out", "o")
FFN_QUANT_POINTS = ("gate", "up", "mid", "down")
DELTANET_QUANT_POINTS = (
    "q",
    "k",
    "v",
    "alpha_logit",
    "beta_logit",
    "u",
    "decay",
    "update",
    "o",
    "proj",
)
NORM_QUANT_POINT = "norm_post"


def static_calibration(
    model: G.GgufModel,
    activation_default_scale: float = 1.0 / 127.0,
) -> dict:
    """Compute weight scales via max-abs/127 and use a flat heuristic
    for every activation scale. Returns a JSON-ready dict."""
    arch = model.arch  # ArchDims
    weight_scales: dict[str, int] = {}
    for name, arr in model.tensors.items():
        s = derive_weight_scale(arr)
        weight_scales[name] = f32_to_scale_num(s)

    activation_scales: dict[str, int] = {}
    activation_scales["default"] = f32_to_scale_num(activation_default_scale)
    activation_scales[NORM_QUANT_POINT] = f32_to_scale_num(activation_default_scale)
    for n in range(arch.num_layers):
        # We don't know yet which layer is attention-flavored vs deltanet
        # without per-layer metadata. Emit both sets keyed by layer; the
        # quantizer picks the right ones for each layer kind at write time.
        for p in ATTENTION_QUANT_POINTS:
            activation_scales[f"layer[{n}].attn.{p}"] = f32_to_scale_num(
                activation_default_scale
            )
        for p in DELTANET_QUANT_POINTS:
            activation_scales[f"layer[{n}].dnet.{p}"] = f32_to_scale_num(
                activation_default_scale
            )
        for p in FFN_QUANT_POINTS:
            activation_scales[f"layer[{n}].ffn.{p}"] = f32_to_scale_num(
                activation_default_scale
            )
        activation_scales[f"layer[{n}].{NORM_QUANT_POINT}.1"] = f32_to_scale_num(
            activation_default_scale
        )
        activation_scales[f"layer[{n}].{NORM_QUANT_POINT}.2"] = f32_to_scale_num(
            activation_default_scale
        )
    activation_scales[f"final_{NORM_QUANT_POINT}"] = f32_to_scale_num(
        activation_default_scale
    )

    return {
        "model_arch": arch.name,
        "mode": "static",
        "vocab": arch.vocab_size,
        "hidden": arch.hidden,
        "intermediate": arch.intermediate,
        "num_layers": arch.num_layers,
        "num_q_heads": arch.num_q_heads,
        "num_kv_heads": arch.num_kv_heads,
        "head_dim": arch.head_dim,
        "rope_theta": arch.rope_theta,
        "max_position": arch.max_position,
        "weight_scales": weight_scales,
        "activation_scales": activation_scales,
        "norm_eps_q": 1,
    }


def activation_calibration(
    model: G.GgufModel,
    prompts: Iterable[Iterable[int]],
    seq_len_cap: int = 64,
) -> dict:
    """Activation-tracking calibration. Currently a stub — relies on a
    yet-to-be-written numpy f32 forward.

    For Phase 2.9.3 the function is exposed but raises NotImplementedError
    if called; the user is expected to use `--mode static` with sensible
    defaults until the forward reference is generalized to f32. Static
    is what `synthetic_qwen_mini.py` already uses successfully end-to-end.
    """
    _ = (model, prompts, seq_len_cap)
    raise NotImplementedError(
        "activation-tracking calibration is a future tightening; "
        "run `--mode static` for now and tune `--activation-scale-f32` "
        "from the reference forward output if you need to refine."
    )


def calibrate(
    gguf_path: str,
    mode: str = "static",
    activation_default_scale: float = 1.0 / 127.0,
    prompts_file: Optional[str] = None,
    seq_len_cap: int = 64,
    arch_prefix: Optional[str] = None,
    extra_tensor_aliases: Optional[dict[str, str]] = None,
) -> dict:
    model = G.read_model(
        gguf_path,
        arch_override=arch_prefix,
        extra_tensor_aliases=extra_tensor_aliases,
    )
    if mode == "static":
        return static_calibration(model, activation_default_scale=activation_default_scale)
    if mode == "activation":
        if prompts_file is None:
            raise SystemExit("activation mode requires --prompts <file>")
        prompts = []
        with open(prompts_file) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                prompts.append([int(t) for t in line.split(",")])
        return activation_calibration(model, prompts, seq_len_cap=seq_len_cap)
    raise SystemExit(f"unknown calibration mode: {mode}")


def main(argv: Optional[Iterable[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    p.add_argument("--gguf", required=True, help="path to .gguf input")
    p.add_argument("--out", required=True, help="path to write scales.json")
    p.add_argument("--mode", choices=("static", "activation"), default="static")
    p.add_argument(
        "--activation-scale-f32",
        type=float,
        default=1.0 / 127.0,
        help="default activation scale (f32) for --mode static; default 1/127",
    )
    p.add_argument("--prompts", help="for --mode activation: token id list, one prompt per line")
    p.add_argument("--seq-len-cap", type=int, default=64)
    p.add_argument("--arch", help="architecture prefix override (defaults to GGUF metadata)")
    args = p.parse_args(list(argv) if argv is not None else None)

    scales = calibrate(
        gguf_path=args.gguf,
        mode=args.mode,
        activation_default_scale=args.activation_scale_f32,
        prompts_file=args.prompts,
        seq_len_cap=args.seq_len_cap,
        arch_prefix=args.arch,
    )
    with open(args.out, "w") as f:
        json.dump(scales, f, indent=2, sort_keys=True)
    print(
        f"wrote {args.out}: "
        f"{len(scales['weight_scales'])} weight scales, "
        f"{len(scales['activation_scales'])} activation scales",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
