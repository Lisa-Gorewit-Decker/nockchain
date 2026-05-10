"""Phase 2.11 — synthetic Gemma 4 mini fixture (end-to-end test).

Builds a 2-layer Gemma-shaped model:
- 2 transformer blocks of `LayerWeights::Gemma` flavor (4 norms per
  block, QK norm, sliding-window on layer 1)
- hidden=8, num_q_heads=2, num_kv_heads=1, head_dim=4
- vocab=16, seq_len=8, activation_tile=2

Writes the canonical disk format (`manifest.bin` + `weights.bin` +
`comm_w.hex`) plus the numpy reference's `forward_prefix` output for
target_layer=1 so the Rust integration test can assert byte-equality.
"""

from __future__ import annotations

import os
import sys

import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

import forward_reference as F
import reference_ops as R
import synthetic_qwen_mini as D

VEC_DIR = os.path.join(SCRIPT_DIR, "test_vectors", "gemma_mini")


def small() -> R.Scale:
    return R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 4))


def build_gemma_layer(hidden: int, seed: int, sliding_window: int | None) -> F.GemmaLayer:
    s = small()
    hu = hidden
    nq = 2
    nkv = 1
    hd = 4
    interm = hidden * 2
    return F.GemmaLayer(
        norm1=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, seed)),
            beta=None, eps_q=1, post_scale=s,
        ),
        attn=F.AttentionWeights(
            hidden=hidden, num_q_heads=nq, num_kv_heads=nkv, head_dim=hd,
            w_q=tuple(R.canonical_input_i8(hu * nq * hd, seed + 1)),
            w_k=tuple(R.canonical_input_i8(hu * nkv * hd, seed + 2)),
            w_v=tuple(R.canonical_input_i8(hu * nkv * hd, seed + 3)),
            w_o=tuple(R.canonical_input_i8(nq * hd * hu, seed + 4)),
        ),
        attn_scales=F.AttentionScales(q=s, k=s, v=s, score=s, attn_out=s, o=s),
        q_norm_gamma=tuple(R.canonical_input_i8(hd, seed + 5)),
        k_norm_gamma=tuple(R.canonical_input_i8(hd, seed + 6)),
        qk_norm_eps_q=1,
        qk_norm_post_scale=s,
        post_attn_norm=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, seed + 7)),
            beta=None, eps_q=1, post_scale=s,
        ),
        norm2=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, seed + 8)),
            beta=None, eps_q=1, post_scale=s,
        ),
        ffn=F.FfnWeights(
            hidden=hidden, intermediate=interm,
            w_gate=tuple(R.canonical_input_i8(hu * interm, seed + 9)),
            w_up=tuple(R.canonical_input_i8(hu * interm, seed + 10)),
            w_down=tuple(R.canonical_input_i8(interm * hu, seed + 11)),
        ),
        ffn_scales=R.FfnScales(gate=s, up=s, mid=s, down=s),
        post_ffn_norm=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, seed + 12)),
            beta=None, eps_q=1, post_scale=s,
        ),
        sliding_window=sliding_window,
        inp_gate=tuple(R.canonical_input_i8(hu, seed + 13)),
        layer_output_scale=tuple(R.canonical_input_i8(hu, seed + 14)),
    )


def build_gemma_mini() -> F.Model:
    hidden = 8
    seq_len = 8
    layers = (
        build_gemma_layer(hidden, seed=0x100, sliding_window=None),
        build_gemma_layer(hidden, seed=0x200, sliding_window=4),  # sliding-window layer
    )
    final_norm = F.NormSpec(
        kind="rms", gamma=tuple(R.canonical_input_i8(hidden, 0xAAAA)),
        beta=None, eps_q=1, post_scale=small(),
    )
    rope = F.RopeTables.identity(seq_len, half_head_dim=2)
    ffn_lut_bytes = tuple(((i - 128) & 0xFF) for i in range(256))
    sig_lut_bytes = tuple(((i - 128) & 0xFF) for i in range(256))
    softmax_lut = R.ExpLut(table=tuple(1 << 16 for _ in range(256)))
    return F.Model(
        dims=F.ModelDims(vocab=16, hidden=hidden, seq_len=seq_len, activation_tile=2),
        embed=tuple(R.canonical_input_i8(16 * hidden, 0xBEEF_CAFE)),
        layers=layers,
        final_norm=final_norm,
        rope_tables=rope,
        softmax_lut=softmax_lut,
        sigmoid_lut_bytes=sig_lut_bytes,
        ffn_activation_bytes=ffn_lut_bytes,
    )


def main() -> int:
    os.makedirs(VEC_DIR, exist_ok=True)
    model = build_gemma_mini()
    comm_w = D.compute_comm_w(model, arch_tag="gemma4", feature_flags=0x27f)
    manifest_bytes = D.encode_manifest(
        model.dims, list(model.layers), model.final_norm, model.rope_tables,
        ffn_kind="silu", sigmoid_kind="silu",
        arch_tag="gemma4", feature_flags=0x27f,
    )
    weights_bytes = D.encode_weights(model)
    with open(os.path.join(VEC_DIR, "manifest.bin"), "wb") as f:
        f.write(manifest_bytes)
    with open(os.path.join(VEC_DIR, "weights.bin"), "wb") as f:
        f.write(weights_bytes)
    with open(os.path.join(VEC_DIR, "comm_w.hex"), "w") as f:
        f.write(comm_w.hex())

    prompt = [1, 5, 9, 0]
    output, log = F.forward_prefix(model, prompt, target_layer=1)
    np.array(prompt, dtype=np.uint32).tofile(os.path.join(VEC_DIR, "prompt.bin"))
    np.array(output, dtype=np.int8).tofile(
        os.path.join(VEC_DIR, "forward_layer_1_output.bin")
    )
    for i, layer_tensor in enumerate(log):
        np.array(layer_tensor, dtype=np.int8).tofile(
            os.path.join(VEC_DIR, f"activation_layer_{i}.bin")
        )

    with open(os.path.join(VEC_DIR, "meta.txt"), "w") as f:
        f.write(
            f"vocab={model.dims.vocab} hidden={model.dims.hidden} "
            f"seq_len={model.dims.seq_len} activation_tile={model.dims.activation_tile} "
            f"num_layers={len(model.layers)} target_layer=1 prompt_len={len(prompt)} "
            f"arch_tag=gemma4 feature_flags=0x27f\n"
        )
    print(
        f"gemma_mini: comm_W={comm_w.hex()[:16]}... manifest={len(manifest_bytes)}B "
        f"weights={len(weights_bytes)}B output_len={len(output)}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
