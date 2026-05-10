"""Phase 2.10 — Qwen 3.6 27B (`qwen35` GGUF arch tag).

64 transformer blocks. Block indices `{3, 7, 11, ..., 63}` are
**STANDARD_ATTENTION** with separate Q/K/V projections and QK norm
(`attn_q_norm`, `attn_k_norm`). The remaining 48 blocks are hybrid
**QWEN_HYBRID_SSM**: a single fused `attn_qkv` projection plus an
`attn_gate`, in parallel with a Mamba state-space-model recurrence
(`ssm_a`, `ssm_alpha`, `ssm_beta`, `ssm_conv1d`, `ssm_dt`, `ssm_norm`,
`ssm_out`).

This phase (2.10) only registers the architecture and surfaces the
canonical names. Phase 2.12 adds the standard-attention forward and
Phase 2.13 adds the SSM forward.
"""

from __future__ import annotations

from typing import Optional

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

# A block is "standard attention" iff it has separate `attn_q.weight`,
# `attn_k.weight`, `attn_v.weight`. We probe the tensor list to decide.


def _block_has_separate_qkv(reader: gguf.GGUFReader, block_idx: int) -> bool:
    want = f"blk.{block_idx}.attn_q.weight"
    for t in reader.tensors:
        if t.name == want:
            return True
    return False


class Qwen35(Architecture):
    name = "qwen35"

    def read_dims(self, reader: gguf.GGUFReader) -> ArchDims:
        num_layers = field_u32(reader, "qwen35.block_count")
        hidden = field_u32(reader, "qwen35.embedding_length")
        intermediate = field_u32(reader, "qwen35.feed_forward_length")
        num_q = field_u32(reader, "qwen35.attention.head_count")
        # head_count_kv is an ARRAY (per-layer) in qwen35.
        kv_field = reader.get_field("qwen35.attention.head_count_kv")
        num_kv = int(kv_field.parts[-1][0]) if kv_field is not None else num_q
        key_len = field_u32(reader, "qwen35.attention.key_length", default=hidden // num_q)
        val_len = field_u32(reader, "qwen35.attention.value_length", default=key_len)
        rope_theta = field_f32(reader, "qwen35.rope.freq_base", default=10_000.0)
        ctx = field_u32(reader, "qwen35.context_length", default=4096)
        # Vocab from embed table row count (set later in gguf_reader)
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
            extras={
                "full_attention_interval": field_u32(
                    reader, "qwen35.full_attention_interval", default=4
                ),
            },
        )

    def block_kind(self, reader: gguf.GGUFReader, block_idx: int) -> BlockKind:
        if _block_has_separate_qkv(reader, block_idx):
            return BlockKind.QWEN_STANDARD_ATTENTION
        return BlockKind.QWEN_HYBRID_SSM

    def tensor_alias_map(self) -> dict[str, str]:
        # Standard-attention blocks have these names. The hybrid blocks
        # use a disjoint subset (handled by per_block_overrides).
        return {
            "attn_norm.weight": "norm1.gamma",
            "attn_q.weight": "attn.w_q",
            "attn_q_norm.weight": "attn.q_norm",
            "attn_k.weight": "attn.w_k",
            "attn_k_norm.weight": "attn.k_norm",
            "attn_v.weight": "attn.w_v",
            "attn_output.weight": "attn.w_o",
            "post_attention_norm.weight": "norm2.gamma",
            "ffn_gate.weight": "ffn.w_gate",
            "ffn_up.weight": "ffn.w_up",
            "ffn_down.weight": "ffn.w_down",
        }

    def per_block_overrides(
        self, reader: gguf.GGUFReader, block_idx: int
    ) -> Optional[dict[str, str]]:
        kind = self.block_kind(reader, block_idx)
        if kind == BlockKind.QWEN_STANDARD_ATTENTION:
            return None  # tensor_alias_map already covers it
        # Hybrid block (gated attention + Mamba SSM).
        return {
            "attn_norm.weight": "norm1.gamma",
            "attn_qkv.weight": "attn.w_qkv",  # fused; split at quantize time
            "attn_gate.weight": "attn.w_gate",
            "post_attention_norm.weight": "norm2.gamma",
            "ssm_a": "ssm.a",
            "ssm_alpha.weight": "ssm.w_alpha",
            "ssm_beta.weight": "ssm.w_beta",
            "ssm_conv1d.weight": "ssm.w_conv1d",
            "ssm_dt": "ssm.dt",
            "ssm_norm.weight": "ssm.norm.gamma",
            "ssm_out.weight": "ssm.w_out",
            "ffn_gate.weight": "ffn.w_gate",
            "ffn_up.weight": "ffn.w_up",
            "ffn_down.weight": "ffn.w_down",
        }

    def toplevel_alias_map(self) -> dict[str, str]:
        return {
            "token_embd.weight": "embed",
            "output_norm.weight": "final_norm.gamma",
            "output.weight": "lm_head",
        }

    def feature_flags(self) -> int:
        return (
            Feature.QK_NORM.value
            | Feature.POST_ATTN_NORM.value
            | Feature.FUSED_QKV.value
            | Feature.SSM_PARALLEL.value
        )


register(Qwen35())
