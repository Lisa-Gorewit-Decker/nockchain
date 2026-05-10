"""Phase 2.10 — the existing Phase 2.9 qwen3 path, re-routed through
the architecture registry.

GGUF `general.architecture == "qwen3"`. Every block is plain
multi-head/GQA attention + SwiGLU FFN with two RMS norms — the
"vanilla" transformer the crate started with.
"""

from __future__ import annotations

import gguf

from . import (
    ArchDims,
    Architecture,
    BlockKind,
    field_f32,
    field_u32,
    register,
)


class Qwen3Legacy(Architecture):
    name = "qwen3"

    def read_dims(self, reader: gguf.GGUFReader) -> ArchDims:
        num_layers = field_u32(reader, "qwen3.block_count")
        hidden = field_u32(reader, "qwen3.embedding_length")
        intermediate = field_u32(reader, "qwen3.feed_forward_length")
        num_q = field_u32(reader, "qwen3.attention.head_count")
        num_kv = field_u32(reader, "qwen3.attention.head_count_kv")
        head_dim = field_u32(reader, "qwen3.rope.dimension_count", default=hidden // num_q)
        vocab = field_u32(reader, "qwen3.vocab_size", default=0)
        rope_theta = field_f32(reader, "qwen3.rope.freq_base", default=10_000.0)
        ctx = field_u32(reader, "qwen3.context_length", default=4096)
        return ArchDims(
            name=self.name,
            num_layers=num_layers,
            hidden=hidden,
            intermediate=intermediate,
            num_q_heads=num_q,
            num_kv_heads=num_kv,
            head_dim=head_dim,
            vocab_size=vocab,
            rope_theta=rope_theta,
            max_position=ctx,
        )

    def block_kind(self, reader: gguf.GGUFReader, block_idx: int) -> BlockKind:
        return BlockKind.STANDARD_ATTENTION

    def tensor_alias_map(self) -> dict[str, str]:
        return {
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
        }

    def toplevel_alias_map(self) -> dict[str, str]:
        return {
            "token_embd.weight": "embed",
            "output_norm.weight": "final_norm.gamma",
            "output_norm.bias": "final_norm.beta",
            "output.weight": "lm_head",
        }


register(Qwen3Legacy())
