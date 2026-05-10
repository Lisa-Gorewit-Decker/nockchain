"""Phase 2.10 — architecture registry.

Every supported architecture is a subclass of [`Architecture`]
registered into `REGISTRY` (keyed by the GGUF `general.architecture`
string). The reader dispatches by looking up `general.architecture`
and calling the matching subclass's hooks.

This file owns the abstract base + registry. Concrete subclasses live
next to it: `qwen3_legacy.py`, `qwen35.py`, `gemma4.py`. New
architectures plug in by importing and calling `register(...)`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Optional

import gguf


class BlockKind(Enum):
    """Logical classification of a transformer block within a model.

    Each canonical layer the Rust crate knows about maps to a
    `BlockKind`. New architectures may introduce new kinds (e.g.
    `QWEN_HYBRID_SSM`); the Rust loader must understand the tag, else
    it rejects.
    """

    STANDARD_ATTENTION = "standard_attention"
    """Plain MHA / GQA attention + SwiGLU FFN with 2 RMS norms. The
    Phase 2.9 default. Maps to `LayerWeights::Attention` in Rust."""

    DELTANET = "deltanet"
    """Gated DeltaNet linear-attention recurrence. Maps to
    `LayerWeights::DeltaNet`."""

    GEMMA_ATTENTION = "gemma_attention"
    """Gemma-4-style attention: 4 norms (pre-attn, post-attn, pre-ffn,
    post-ffn), QK norm on Q and K, optional inp_gate and
    layer_output_scale. Phase 2.11."""

    QWEN_STANDARD_ATTENTION = "qwen_standard_attention"
    """Qwen 3.6 27B's pure-attention block: separate Q/K/V + QK norm
    + post_attention_norm. Phase 2.12."""

    QWEN_HYBRID_SSM = "qwen_hybrid_ssm"
    """Qwen 3.6 27B's hybrid block: fused QKV with attn_gate (gated
    attention) PARALLEL with Mamba SSM. Phase 2.13."""


class Feature(Enum):
    """Optional features an architecture may declare. Surfaced in the
    manifest's `feature_flags: u64` bitfield (Phase 2.10's manifest
    v2)."""

    QK_NORM = 1 << 0
    """RMSNorm applied to Q and K after the linear projections."""

    INP_GATE = 1 << 1
    """Per-block input gating (Gemma 4 `inp_gate.weight`)."""

    LAYER_OUTPUT_SCALE = 1 << 2
    """Per-layer scalar output scaling (Gemma 4)."""

    POST_FFN_NORM = 1 << 3
    """Additional RMSNorm after the FFN sublayer (Gemma 4
    `post_ffw_norm`)."""

    POST_ATTN_NORM = 1 << 4
    """RMSNorm between attention output and the residual add
    (Gemma 4, Qwen 3.6 `post_attention_norm`)."""

    SLIDING_WINDOW = 1 << 5
    """Some blocks use sliding-window instead of full causal
    attention. The arch's `sliding_pattern()` returns the per-layer
    mask kind."""

    LOGIT_SOFTCAP = 1 << 6
    """Final logits clipped through `tanh(x / cap) * cap` after
    `lm_head`."""

    SSM_PARALLEL = 1 << 7
    """Some blocks include a Mamba state-space recurrence in
    parallel with attention."""

    FUSED_QKV = 1 << 8
    """Q/K/V projections fused into one weight matrix; arch is
    responsible for splitting at read time."""

    PER_LAYER_EMBED = 1 << 9
    """Per-layer auxiliary token embeddings (Gemma 4
    `per_layer_token_embd.weight`)."""


@dataclass
class ArchDims:
    """Architecture-side facts about a model. Numbers come from KV;
    `tensor_overrides` lets the arch specialize the gguf name → canon
    mapping per-block (rare but handy)."""

    name: str
    num_layers: int
    hidden: int
    intermediate: int
    num_q_heads: int
    num_kv_heads: int
    head_dim: int
    head_dim_kv: Optional[int] = None
    vocab_size: int = 0
    rope_theta: float = 10_000.0
    max_position: int = 4096
    sliding_window: Optional[int] = None
    extras: dict = field(default_factory=dict)


class Architecture:
    """Abstract base for an architecture entry. Concrete subclasses
    implement the hooks below and register themselves via
    `register(name, MyArch())`."""

    name: str = ""

    def matches(self, arch_str: str) -> bool:
        return arch_str == self.name

    def read_dims(self, reader: gguf.GGUFReader) -> ArchDims:
        raise NotImplementedError

    def block_kind(self, reader: gguf.GGUFReader, block_idx: int) -> BlockKind:
        raise NotImplementedError

    def tensor_alias_map(self) -> dict[str, str]:
        """GGUF tensor name (without the `blk.{N}.` prefix where
        applicable) → canonical sub-name (e.g. `attn.w_q`)."""
        raise NotImplementedError

    def toplevel_alias_map(self) -> dict[str, str]:
        """Top-level GGUF tensor names → canonical names."""
        raise NotImplementedError

    def feature_flags(self) -> int:
        """Bit-OR of [`Feature`] values that this architecture
        requires."""
        return 0

    def per_block_overrides(
        self, reader: gguf.GGUFReader, block_idx: int
    ) -> Optional[dict[str, str]]:
        """Optional per-block tensor alias overrides — needed for
        hybrid models like qwen35 where every 4th block uses different
        tensor names. Returns None to fall back to `tensor_alias_map`."""
        return None


REGISTRY: dict[str, Architecture] = {}


def register(arch: Architecture) -> None:
    if not arch.name:
        raise ValueError("Architecture must declare a non-empty name")
    REGISTRY[arch.name] = arch


def get(arch_str: str) -> Architecture:
    if arch_str not in REGISTRY:
        raise KeyError(
            f"unsupported architecture '{arch_str}'. Available: "
            f"{sorted(REGISTRY.keys())}"
        )
    return REGISTRY[arch_str]


def field_u32(reader: gguf.GGUFReader, key: str, default: Optional[int] = None) -> int:
    f = reader.get_field(key)
    if f is None:
        if default is not None:
            return default
        raise KeyError(f"GGUF missing required u32 field {key}")
    return int(f.parts[-1][0])


def field_f32(reader: gguf.GGUFReader, key: str, default: Optional[float] = None) -> float:
    f = reader.get_field(key)
    if f is None:
        if default is not None:
            return default
        raise KeyError(f"GGUF missing required f32 field {key}")
    return float(f.parts[-1][0])


def field_str(reader: gguf.GGUFReader, key: str, default: Optional[str] = None) -> str:
    f = reader.get_field(key)
    if f is None:
        if default is not None:
            return default
        raise KeyError(f"GGUF missing required str field {key}")
    return str(bytes(f.parts[-1]), encoding="utf-8")


# Register concrete subclasses by importing the modules. Each module's
# import side-effect calls `register(...)`. Keep this list in
# alphabetical order.
from . import gemma4  # noqa: E402,F401
from . import qwen3_legacy  # noqa: E402,F401
from . import qwen35  # noqa: E402,F401
