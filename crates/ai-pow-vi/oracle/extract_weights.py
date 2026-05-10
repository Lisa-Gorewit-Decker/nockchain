"""Skeleton: extract weights from a HuggingFace / GGUF model and
requantize to ai-pow-vi's canonical INT8 layout.

This script is intentionally a skeleton. The dequantization details
(scale calibration, per-channel vs per-tensor, GQA head packing,
RMSNorm gamma extraction, FFN gate/up/down ordering) are model-specific
and require the user to map the HF tensor names of their target model
to the canonical names this crate uses.

Reference Hugging Face → ai-pow-vi mapping (Qwen 3.6 27B dense, indicative):

    Qwen3 transformer block N:
      model.layers.{N}.input_layernorm.weight     → norm1.gamma
      model.layers.{N}.self_attn.q_proj.weight    → attn.w_q  (transpose to col-major)
      model.layers.{N}.self_attn.k_proj.weight    → attn.w_k
      model.layers.{N}.self_attn.v_proj.weight    → attn.w_v
      model.layers.{N}.self_attn.o_proj.weight    → attn.w_o
      model.layers.{N}.post_attention_layernorm.weight → norm2.gamma
      model.layers.{N}.mlp.gate_proj.weight       → ffn.w_gate
      model.layers.{N}.mlp.up_proj.weight         → ffn.w_up
      model.layers.{N}.mlp.down_proj.weight       → ffn.w_down
      model.embed_tokens.weight                   → embed
      model.norm.weight                           → final_norm.gamma

For DeltaNet hybrid layers, additionally:
      model.layers.{N}.linear_attn.q_proj.weight  → dnet.w_q
      ...
      model.layers.{N}.linear_attn.alpha_proj.weight → dnet.w_alpha
      model.layers.{N}.linear_attn.beta_proj.weight  → dnet.w_beta

Quantization:
    Per-tensor symmetric INT8 with scale = max(|w|) / 127, rounded to
    the nearest representable num/2^15. (Use `Scale.from_f32` semantics
    in reference_ops.py.)

Usage (when implemented):
    python oracle/extract_weights.py \\
        --model /path/to/qwen3.6-27b-hf \\
        --out oracle/test_vectors/qwen_3_6_27b/

Output:
    weights.bin        canonical bytes (consumable by Model::load — Phase 2.7b)
    manifest.json      dims, layer kinds, scales, eps_q
    comm_w.hex         the model's pinned 32-byte commitment

Until the user fills in their model-specific mapping, this script
raises NotImplementedError.
"""

from __future__ import annotations

import argparse
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--model", required=True, help="HF model dir or HF hub id")
    parser.add_argument("--out", required=True, help="output directory for vectors")
    args = parser.parse_args()

    raise NotImplementedError(
        f"extract_weights.py is a skeleton. To use it for {args.model}, fill in:\n"
        "  1. The HF tensor-name → canonical-name mapping for your model.\n"
        "  2. Per-tensor scale calibration (max-abs / 127).\n"
        "  3. Column-major transpose for the linear weights.\n"
        "  4. comm_W computation via reference_ops + (Phase 2.7b) save.\n"
        "  5. RoPE table generation, sigmoid LUT, softmax LUT precomputation."
    )


if __name__ == "__main__":
    sys.exit(main())
