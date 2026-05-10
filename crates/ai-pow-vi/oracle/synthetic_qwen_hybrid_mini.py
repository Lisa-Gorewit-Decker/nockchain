"""Phase 2.13.1 — synthetic Qwen-hybrid-mini fixture (end-to-end test).

Builds a 2-layer Qwen 3.6 27B-shaped model:
- Layer 0: `LayerWeights::QwenStandard` (pure attention with QK norm).
- Layer 1: `LayerWeights::QwenHybridSsm` (gated attention + Mamba SSM).
- hidden=8, num_q_heads=2, num_kv_heads=1, head_dim=4, num_v_heads=3,
  ssm_kernel_size=3, intermediate=16
- vocab=16, seq_len=8, activation_tile=2

Writes the canonical disk format (`manifest.bin` + `weights.bin` +
`comm_w.hex`) plus the numpy reference's `forward_prefix` output for
target_layer=2 so the Rust integration test can assert byte-equality.

This is the qwen35 counterpart of `synthetic_gemma_mini.py` — exercises
both the Phase 2.12 attention-only path and the Phase 2.13 hybrid SSM
path through the full save → load → forward pipeline.
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

VEC_DIR = os.path.join(SCRIPT_DIR, "test_vectors", "qwen_hybrid_mini")


def small() -> R.Scale:
    return R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 4))


def build_qwen_standard(hidden: int, seed: int) -> F.QwenStandardLayer:
    s = small()
    hu = hidden
    nq = 2
    nkv = 1
    hd = 4
    interm = hidden * 2
    return F.QwenStandardLayer(
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
        norm2=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, seed + 7)),
            beta=None, eps_q=1, post_scale=s,
        ),
        ffn=F.FfnWeights(
            hidden=hidden, intermediate=interm,
            w_gate=tuple(R.canonical_input_i8(hu * interm, seed + 8)),
            w_up=tuple(R.canonical_input_i8(hu * interm, seed + 9)),
            w_down=tuple(R.canonical_input_i8(interm * hu, seed + 10)),
        ),
        ffn_scales=R.FfnScales(gate=s, up=s, mid=s, down=s),
    )


def build_qwen_hybrid_ssm(hidden: int, seed: int) -> F.QwenHybridSsmLayer:
    s = small()
    hu = hidden
    nq = 2
    nkv = 1
    nv = 3
    hd = 4
    kernel = 3
    interm = hidden * 2
    q_dim = nq * hd
    kv_dim = nkv * hd
    total_qkv = q_dim + kv_dim + kv_dim
    return F.QwenHybridSsmLayer(
        norm1=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, seed)),
            beta=None, eps_q=1, post_scale=s,
        ),
        attn_qkv_fused=tuple(R.canonical_input_i8(hu * total_qkv, seed + 1)),
        attn_gate=tuple(R.canonical_input_i8(hu * q_dim, seed + 2)),
        attn_out=tuple(R.canonical_input_i8(q_dim * hu, seed + 3)),
        num_q_heads=nq,
        num_kv_heads=nkv,
        head_dim=hd,
        attn_scales=F.AttentionScales(q=s, k=s, v=s, score=s, attn_out=s, o=s),
        q_norm_gamma=tuple(R.canonical_input_i8(hd, seed + 4)),
        k_norm_gamma=tuple(R.canonical_input_i8(hd, seed + 5)),
        qk_norm_eps_q=1,
        qk_norm_post_scale=s,
        ssm_a=tuple(R.canonical_input_i8(nv, seed + 6)),
        ssm_alpha=tuple(R.canonical_input_i8(hu * nv, seed + 7)),
        ssm_beta=tuple(R.canonical_input_i8(hu * nv, seed + 8)),
        ssm_conv1d=tuple(R.canonical_input_i8(kernel * hu, seed + 9)),
        ssm_dt=tuple(R.canonical_input_i8(nv, seed + 10)),
        ssm_norm_gamma=tuple(R.canonical_input_i8(hd, seed + 11)),
        ssm_norm_eps_q=1,
        ssm_norm_post_scale=s,
        ssm_out=tuple(R.canonical_input_i8(nv * hd * hu, seed + 12)),
        num_v_heads=nv,
        ssm_kernel_size=kernel,
        ssm_scales=F.DeltaNetScales(
            q=s, k=s, v=s, alpha_logit=s, beta_logit=s,
            u=s, decay=s, update=s, o=s, proj=s,
        ),
        norm2=F.NormSpec(
            kind="rms", gamma=tuple(R.canonical_input_i8(hu, seed + 13)),
            beta=None, eps_q=1, post_scale=s,
        ),
        ffn=F.FfnWeights(
            hidden=hidden, intermediate=interm,
            w_gate=tuple(R.canonical_input_i8(hu * interm, seed + 14)),
            w_up=tuple(R.canonical_input_i8(hu * interm, seed + 15)),
            w_down=tuple(R.canonical_input_i8(interm * hu, seed + 16)),
        ),
        ffn_scales=R.FfnScales(gate=s, up=s, mid=s, down=s),
    )


def build_qwen_hybrid_mini() -> F.Model:
    hidden = 8
    seq_len = 8
    layers = (
        build_qwen_standard(hidden, seed=0x100),
        build_qwen_hybrid_ssm(hidden, seed=0x200),
    )
    final_norm = F.NormSpec(
        kind="rms", gamma=tuple(R.canonical_input_i8(hidden, 0xAAAA)),
        beta=None, eps_q=1, post_scale=small(),
    )
    rope = F.RopeTables.identity(seq_len, half_head_dim=2)
    # Hard-sigmoid LUT (matches the deltanet/ssm pin). Used for both
    # SSM α/β gating AND attn_gate logits in the hybrid layer.
    sig_lut_bytes = bytearray(256)
    for i in range(256):
        x = i - 128
        v = max(0, min(127, 64 + x // 2))
        sig_lut_bytes[i] = v
    sig_lut = tuple(sig_lut_bytes)
    # Identity FFN activation.
    ffn_lut_bytes = tuple(((i - 128) & 0xFF) for i in range(256))
    softmax_lut = R.ExpLut(table=tuple(1 << 16 for _ in range(256)))
    return F.Model(
        dims=F.ModelDims(vocab=16, hidden=hidden, seq_len=seq_len, activation_tile=2),
        embed=tuple(R.canonical_input_i8(16 * hidden, 0xBEEF_FACE)),
        layers=layers,
        final_norm=final_norm,
        rope_tables=rope,
        softmax_lut=softmax_lut,
        sigmoid_lut_bytes=sig_lut,
        ffn_activation_bytes=ffn_lut_bytes,
    )


def main() -> int:
    os.makedirs(VEC_DIR, exist_ok=True)
    model = build_qwen_hybrid_mini()
    comm_w = D.compute_comm_w(model, arch_tag="qwen35", feature_flags=0)
    manifest_bytes = D.encode_manifest(
        model.dims, list(model.layers), model.final_norm, model.rope_tables,
        ffn_kind="identity", sigmoid_kind="silu",
        arch_tag="qwen35", feature_flags=0,
    )
    weights_bytes = D.encode_weights(model)
    with open(os.path.join(VEC_DIR, "manifest.bin"), "wb") as f:
        f.write(manifest_bytes)
    with open(os.path.join(VEC_DIR, "weights.bin"), "wb") as f:
        f.write(weights_bytes)
    with open(os.path.join(VEC_DIR, "comm_w.hex"), "w") as f:
        f.write(comm_w.hex())

    prompt = [1, 5, 9, 0]
    output, log = F.forward_prefix(model, prompt, target_layer=2)
    np.array(prompt, dtype=np.uint32).tofile(os.path.join(VEC_DIR, "prompt.bin"))
    np.array(output, dtype=np.int8).tofile(
        os.path.join(VEC_DIR, "forward_layer_2_output.bin")
    )
    for i, layer_tensor in enumerate(log):
        np.array(layer_tensor, dtype=np.int8).tofile(
            os.path.join(VEC_DIR, f"activation_layer_{i}.bin")
        )

    with open(os.path.join(VEC_DIR, "meta.txt"), "w") as f:
        f.write(
            f"vocab={model.dims.vocab} hidden={model.dims.hidden} "
            f"seq_len={model.dims.seq_len} activation_tile={model.dims.activation_tile} "
            f"num_layers={len(model.layers)} target_layer=2 prompt_len={len(prompt)} "
            f"arch_tag=qwen35 feature_flags=0x0\n"
        )
    print(
        f"qwen_hybrid_mini: comm_W={comm_w.hex()[:16]}... manifest={len(manifest_bytes)}B "
        f"weights={len(weights_bytes)}B output_len={len(output)}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
