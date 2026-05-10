"""Phase 2.10 — Gemma 4 (`gemma4` GGUF arch tag).

Used for both Gemma 4 8B and Gemma 4 31B. All transformer blocks are
the same architecture (no hybrid pattern like Qwen 3.6 27B). Each
block has:

- **Four** RMS norms: `attn_norm` (pre-attn), `post_attention_norm`
  (between attn and residual add), `ffn_norm` (pre-ffn), `post_ffw_norm`
  (between ffn and residual add).
- Separate Q/K/V projections, plus **QK norm** RMSNorms applied to Q
  and K after their linear projections, before RoPE.
- `inp_gate` and `layer_output_scale` for per-block input gating and
  per-layer output scaling.
- A `proj` weight and a `post_norm` per block (used in the per-layer
  embedding path).
- Sliding-window attention interleaved with full attention per a
  `sliding_window_pattern` array in the KV.
- Final logit softcapping (`final_logit_softcapping` in KV).

This phase registers the architecture and surfaces canonical names.
Phase 2.11 adds the new Gemma-side forward implementations.
"""

from __future__ import annotations

import gguf

from . import (
    ArchDims,
    Architecture,
    BlockKind,
    Feature,
    field_f32,
    field_u32,
    register,
)


class Gemma4(Architecture):
    name = "gemma4"

    def read_dims(self, reader: gguf.GGUFReader) -> ArchDims:
        num_layers = field_u32(reader, "gemma4.block_count")
        hidden = field_u32(reader, "gemma4.embedding_length")
        intermediate = field_u32(reader, "gemma4.feed_forward_length")
        num_q = field_u32(reader, "gemma4.attention.head_count")
        num_kv = field_u32(reader, "gemma4.attention.head_count_kv")
        key_len = field_u32(reader, "gemma4.attention.key_length")
        val_len = field_u32(reader, "gemma4.attention.value_length", default=key_len)
        rope_theta = field_f32(reader, "gemma4.rope.freq_base", default=1_000_000.0)
        ctx = field_u32(reader, "gemma4.context_length", default=131_072)
        window = field_u32(reader, "gemma4.attention.sliding_window", default=0) or None
        return ArchDims(
            name=self.name,
            num_layers=num_layers,
            hidden=hidden,
            intermediate=intermediate,
            num_q_heads=num_q,
            num_kv_heads=num_kv,
            head_dim=key_len,
            head_dim_kv=val_len,
            vocab_size=0,  # filled in from embed rows
            rope_theta=rope_theta,
            max_position=ctx,
            sliding_window=window,
            extras={
                "final_logit_softcap": field_f32(
                    reader, "gemma4.final_logit_softcapping", default=0.0
                ),
                "key_length_swa": field_u32(
                    reader, "gemma4.attention.key_length_swa", default=key_len
                ),
            },
        )

    def block_kind(self, reader: gguf.GGUFReader, block_idx: int) -> BlockKind:
        return BlockKind.GEMMA_ATTENTION

    def tensor_alias_map(self) -> dict[str, str]:
        return {
            "attn_norm.weight": "norm1.gamma",
            "attn_q.weight": "attn.w_q",
            "attn_q_norm.weight": "attn.q_norm",
            "attn_k.weight": "attn.w_k",
            "attn_k_norm.weight": "attn.k_norm",
            "attn_v.weight": "attn.w_v",
            "attn_output.weight": "attn.w_o",
            "post_attention_norm.weight": "post_attn_norm.gamma",
            "ffn_norm.weight": "norm2.gamma",
            "ffn_gate.weight": "ffn.w_gate",
            "ffn_up.weight": "ffn.w_up",
            "ffn_down.weight": "ffn.w_down",
            "post_ffw_norm.weight": "post_ffn_norm.gamma",
            "inp_gate.weight": "inp_gate",
            "layer_output_scale.weight": "layer_output_scale",
            "post_norm.weight": "post_norm.gamma",
            "proj.weight": "proj",
        }

    def toplevel_alias_map(self) -> dict[str, str]:
        return {
            "token_embd.weight": "embed",
            "output_norm.weight": "final_norm.gamma",
            # Gemma 4 ties output to token_embd (no separate output.weight in some variants).
            "output.weight": "lm_head",
            "per_layer_token_embd.weight": "per_layer_embed",
            "per_layer_model_proj.weight": "per_layer_proj",
            "per_layer_proj_norm.weight": "per_layer_proj_norm.gamma",
            "rope_freqs.weight": "rope_freqs",
        }

    def feature_flags(self) -> int:
        return (
            Feature.QK_NORM.value
            | Feature.INP_GATE.value
            | Feature.LAYER_OUTPUT_SCALE.value
            | Feature.POST_FFN_NORM.value
            | Feature.POST_ATTN_NORM.value
            | Feature.SLIDING_WINDOW.value
            | Feature.LOGIT_SOFTCAP.value
            | Feature.PER_LAYER_EMBED.value
        )


register(Gemma4())
