"""Phase 2.10 — registry + per-arch GGUF reader tests.

Confirms the architecture registry:
  - has the three expected architectures (qwen3, qwen35, gemma4),
  - reads the real Qwen 3.6 27B + Gemma 4 8B GGUFs and surfaces the
    expected canonical tensor names + block-kind classification,
  - feature_flags match expectations for each arch.

Real-model checks are skipped (not failed) when the Ollama blob isn't
on disk — useful for CI environments without the downloads.
"""

from __future__ import annotations

import os
import sys
import tempfile

import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ORACLE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.insert(0, ORACLE_DIR)
sys.path.insert(0, SCRIPT_DIR)

import arch  # noqa: E402
import gguf_reader as G  # noqa: E402
from arch import BlockKind, Feature  # noqa: E402
from test_gguf_reader import write_tiny_gguf  # noqa: E402


def test_registry_has_three_architectures():
    assert "qwen3" in arch.REGISTRY
    assert "qwen35" in arch.REGISTRY
    assert "gemma4" in arch.REGISTRY
    print("test_registry_has_three_architectures OK")


def test_qwen3_legacy_round_trip_via_registry():
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "tiny.gguf")
        write_tiny_gguf(path)
        m = G.read_model(path)
        assert m.arch.name == "qwen3"
        assert m.arch.num_layers == 1
        # Every block is STANDARD_ATTENTION for qwen3.
        assert m.block_kinds == [BlockKind.STANDARD_ATTENTION]
        assert m.feature_flags == 0  # no special features
        assert "layer[0].attn.w_q" in m.tensors
        print("test_qwen3_legacy_round_trip_via_registry OK")


def _real_blob(digest: str) -> str | None:
    p = os.path.expanduser(f"~/.ollama/models/blobs/sha256-{digest}")
    return p if os.path.exists(p) else None


def test_qwen35_real_blob():
    path = _real_blob("83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926")
    if path is None:
        print("test_qwen35_real_blob SKIP (Ollama blob not on disk)")
        return
    m = G.read_model(path)
    a = m.arch
    assert a.name == "qwen35", a.name
    assert a.num_layers == 64, a.num_layers
    assert a.hidden == 5120, a.hidden
    # qwen35 features:
    flags = m.feature_flags
    for feat in (Feature.QK_NORM, Feature.POST_ATTN_NORM, Feature.FUSED_QKV,
                 Feature.SSM_PARALLEL):
        assert flags & feat.value, f"missing flag {feat.name}"
    # Block-kind census: 16 standard attention, 48 hybrid SSM.
    n_std = sum(k == BlockKind.QWEN_STANDARD_ATTENTION for k in m.block_kinds)
    n_hyb = sum(k == BlockKind.QWEN_HYBRID_SSM for k in m.block_kinds)
    assert n_std == 16, n_std
    assert n_hyb == 48, n_hyb
    # Standard-attention block 3 should have separate Q/K/V/QK norms.
    for sub in ("attn.w_q", "attn.w_k", "attn.w_v", "attn.w_o",
                "attn.q_norm", "attn.k_norm",
                "norm1.gamma", "norm2.gamma",
                "ffn.w_gate", "ffn.w_up", "ffn.w_down"):
        key = f"layer[3].{sub}"
        assert key in m.tensors, f"missing {key}"
    # Hybrid block 0 should have fused QKV, attn_gate, ssm.* tensors.
    for sub in ("attn.w_qkv", "attn.w_gate",
                "ssm.w_alpha", "ssm.w_beta", "ssm.w_conv1d",
                "ssm.w_out", "ssm.norm.gamma",
                "ssm.a", "ssm.dt"):
        key = f"layer[0].{sub}"
        assert key in m.tensors, f"missing {key}"
    # Top-level always present.
    for n in ("embed", "final_norm.gamma", "lm_head"):
        assert n in m.tensors, n
    print(f"test_qwen35_real_blob OK ({len(m.tensors)} canonical tensors)")


def test_gemma4_real_blob():
    path = _real_blob("4c27e0f5b5adf02ac956c7322bd2ee7636fe3f45a8512c9aba5385242cb6e09a")
    if path is None:
        print("test_gemma4_real_blob SKIP (Ollama blob not on disk)")
        return
    m = G.read_model(path)
    a = m.arch
    assert a.name == "gemma4", a.name
    assert a.num_layers == 42, a.num_layers
    assert a.hidden == 2560, a.hidden
    flags = m.feature_flags
    for feat in (Feature.QK_NORM, Feature.INP_GATE, Feature.LAYER_OUTPUT_SCALE,
                 Feature.POST_FFN_NORM, Feature.POST_ATTN_NORM,
                 Feature.SLIDING_WINDOW, Feature.LOGIT_SOFTCAP,
                 Feature.PER_LAYER_EMBED):
        assert flags & feat.value, f"missing flag {feat.name}"
    # All blocks are GEMMA_ATTENTION.
    assert all(k == BlockKind.GEMMA_ATTENTION for k in m.block_kinds), \
        set(m.block_kinds)
    # Block 0 should have all the gemma-specific tensors.
    for sub in ("attn.w_q", "attn.w_k", "attn.w_v", "attn.w_o",
                "attn.q_norm", "attn.k_norm",
                "norm1.gamma", "norm2.gamma",
                "post_attn_norm.gamma", "post_ffn_norm.gamma", "post_norm.gamma",
                "inp_gate", "layer_output_scale", "proj",
                "ffn.w_gate", "ffn.w_up", "ffn.w_down"):
        key = f"layer[0].{sub}"
        assert key in m.tensors, f"missing {key}"
    # Per-layer embed table at top level.
    for n in ("embed", "final_norm.gamma", "per_layer_embed",
              "per_layer_proj", "per_layer_proj_norm.gamma"):
        assert n in m.tensors, n
    print(f"test_gemma4_real_blob OK ({len(m.tensors)} canonical tensors)")


def test_unsupported_architecture_raises():
    """A GGUF whose `general.architecture` isn't in the registry must
    raise a clear error rather than silently dequantize."""
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "fake.gguf")
        import gguf as gg
        w = gg.GGUFWriter(path, "no_such_arch")
        w.add_string("general.architecture", "no_such_arch")
        w.add_uint32("no_such_arch.block_count", 1)
        w.add_tensor("token_embd.weight", np.zeros((4, 4), dtype=np.float32))
        w.write_header_to_file()
        w.write_kv_data_to_file()
        w.write_tensors_to_file()
        w.close()
        try:
            G.read_model(path)
        except KeyError as e:
            assert "no_such_arch" in str(e), str(e)
            print("test_unsupported_architecture_raises OK")
            return
        raise AssertionError("expected KeyError for unsupported architecture")


if __name__ == "__main__":
    test_registry_has_three_architectures()
    test_qwen3_legacy_round_trip_via_registry()
    test_unsupported_architecture_raises()
    test_qwen35_real_blob()
    test_gemma4_real_blob()
    print("all arch registry tests passed")
