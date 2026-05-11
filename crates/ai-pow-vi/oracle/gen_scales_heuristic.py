#!/usr/bin/env python3
"""Generate a scales.json with heuristic per-tap activation scales for
Qwen 3.6 27B. Based on rough magnitude estimates derived from the
matmul structure (no real calibration data).

The scale of a Scale field is `num / 2^15`. For an i32 accumulator of
expected magnitude M, the scale numerator should be `127 / M * 2^15`
to map M → ±127 after rescale.

Conventions:
- Post-matmul i32 magnitudes scale as sqrt(reduction_dim) * weight_magnitude * input_magnitude.
- With input i8 magnitude ~64 (average) and weight i8 magnitude ~64:
  M ≈ sqrt(k) * 64 * 64 = 4096 * sqrt(k).
- For k=5120 (hidden), M ≈ 4096 * 72 ≈ 295k → scale_num ≈ 127 * 2^15 / 295k ≈ 14.
- For k=17408 (FFN intermediate), M ≈ 4096 * 132 ≈ 540k → scale_num ≈ 7.
- For score (k=head_dim=256): M ≈ 4096 * 16 ≈ 65k → scale_num ≈ 64.
"""
import json
import math

num_layers = 64
HIDDEN = 5120
INTERMEDIATE = 17408
HEAD_DIM = 256
NUM_Q = 24
NUM_KV = 4

def scale_for_matmul_out(reduction_dim, target_i8=127):
    """For i32 accumulator from matmul with given reduction dim, return
    the scale numerator that maps the expected magnitude to ±target_i8.
    Assumes input and weight both have typical i8 magnitude ~32 (half of full range)."""
    typical_input = 32
    typical_weight = 32
    expected_mag = math.sqrt(reduction_dim) * typical_input * typical_weight
    scale_num = round(target_i8 / expected_mag * (1 << 15))
    return max(1, min(scale_num, (1 << 30)))

activation_scales = {
    "default": 4096,
    "norm_post": 4096,
    "final_norm_post": 4096,
}

q_dim = NUM_Q * HEAD_DIM       # 6144
kv_dim = NUM_KV * HEAD_DIM     # 1024

# Per-layer activation scales
for n in range(num_layers):
    # Norm post-scales: norms produce ~i32 max ~hidden, so scale ~127/5120 * 2^15 ≈ 813
    # but in practice RmsNorm post-scales are calibrated to bring output into i8 range
    # of normalized magnitudes. Let's start with 2048.
    activation_scales[f"layer[{n}].norm_post.1"] = 2048
    activation_scales[f"layer[{n}].norm_post.2"] = 2048
    activation_scales[f"layer[{n}].qk_norm_post"] = 2048
    activation_scales[f"layer[{n}].ssm_norm_post"] = 2048

    # Attention path
    activation_scales[f"layer[{n}].attn.q"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].attn.k"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].attn.v"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].attn.score"] = scale_for_matmul_out(HEAD_DIM)
    activation_scales[f"layer[{n}].attn.attn_out"] = scale_for_matmul_out(64)  # softmax-weighted sum, small accum
    activation_scales[f"layer[{n}].attn.o"] = scale_for_matmul_out(q_dim)

    # SSM path
    activation_scales[f"layer[{n}].ssm.q"] = scale_for_matmul_out(4)  # conv1d, small kernel
    activation_scales[f"layer[{n}].ssm.k"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].ssm.v"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].ssm.alpha_logit"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].ssm.beta_logit"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].ssm.u"] = 4096
    activation_scales[f"layer[{n}].ssm.decay"] = 4096  # gate product
    activation_scales[f"layer[{n}].ssm.update"] = 4096
    activation_scales[f"layer[{n}].ssm.o"] = 4096
    activation_scales[f"layer[{n}].ssm.proj"] = scale_for_matmul_out(6144)  # ssm_out (num_v * ssm_head_dim, hidden)

    # FFN
    activation_scales[f"layer[{n}].ffn.gate"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].ffn.up"] = scale_for_matmul_out(HIDDEN)
    activation_scales[f"layer[{n}].ffn.mid"] = 4096  # elementwise, no matmul
    activation_scales[f"layer[{n}].ffn.down"] = scale_for_matmul_out(INTERMEDIATE)

scales = {
    "activation_scales": activation_scales,
    "norm_eps_q": 1,
}

with open("/tmp/scales.json", "w") as f:
    json.dump(scales, f, indent=2)

print(f"wrote /tmp/scales.json with {len(activation_scales)} taps")
print(f"sample scales: attn.q={activation_scales['layer[0].attn.q']}, "
      f"attn.score={activation_scales['layer[0].attn.score']}, "
      f"ffn.down={activation_scales['layer[0].ffn.down']}, "
      f"ssm.proj={activation_scales['layer[0].ssm.proj']}")
