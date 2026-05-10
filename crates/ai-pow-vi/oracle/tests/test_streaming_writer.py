"""Phase 2.15 — streaming weights writer parity tests.

Confirms `streaming_writer.streaming_write_model_dir` produces a
directory whose `weights.bin` is byte-identical to
`synthetic_qwen_mini.encode_weights(model)` and whose `comm_w.hex`
matches `compute_comm_w(model)` for every layer flavor we ship:

  - AttentionLayer   (qwen_mini fixture)
  - GemmaLayer       (gemma_mini fixture)
  - QwenStandardLayer + QwenHybridSsmLayer (qwen_hybrid_mini fixture)

This is the primary acceptance test for the streaming infrastructure.
Once it's green, the streaming path can replace the materializing path
without changing on-disk bytes — which is the prerequisite for
converting real 17-19 GB GGUFs without OOM.
"""

from __future__ import annotations

import filecmp
import os
import sys
import tempfile

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ORACLE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.insert(0, ORACLE_DIR)
sys.path.insert(0, SCRIPT_DIR)

import streaming_writer as SW  # noqa: E402
import synthetic_gemma_mini as GM  # noqa: E402
import synthetic_qwen_hybrid_mini as HM  # noqa: E402
import synthetic_qwen_mini as D  # noqa: E402


def _check_parity(model, arch_tag: str, feature_flags: int, ffn_kind: str, sigmoid_kind: str) -> None:
    # Reference path (materialized).
    ref_weights = D.encode_weights(model)
    ref_manifest = D.encode_manifest(
        model.dims, list(model.layers), model.final_norm, model.rope_tables,
        ffn_kind=ffn_kind, sigmoid_kind=sigmoid_kind,
        arch_tag=arch_tag, feature_flags=feature_flags,
    )
    ref_comm_w = D.compute_comm_w(model, arch_tag=arch_tag, feature_flags=feature_flags)

    with tempfile.TemporaryDirectory() as td:
        streaming_comm_w = SW.streaming_write_model_dir(
            model, td, arch_tag=arch_tag, feature_flags=feature_flags,
            ffn_kind=ffn_kind, sigmoid_kind=sigmoid_kind,
        )

        assert streaming_comm_w == ref_comm_w, (
            f"streaming comm_W != reference: "
            f"{streaming_comm_w.hex()[:16]} != {ref_comm_w.hex()[:16]}"
        )

        with open(os.path.join(td, "weights.bin"), "rb") as f:
            streaming_weights = f.read()
        assert streaming_weights == ref_weights, (
            f"weights.bin divergence: "
            f"{len(streaming_weights)} vs {len(ref_weights)} bytes"
        )

        with open(os.path.join(td, "manifest.bin"), "rb") as f:
            streaming_manifest = f.read()
        assert streaming_manifest == ref_manifest, "manifest.bin divergence"

        with open(os.path.join(td, "comm_w.hex"), "r") as f:
            streaming_hex = f.read().strip()
        assert streaming_hex == ref_comm_w.hex(), "comm_w.hex divergence"


def test_attention_layer_parity():
    model = D.build_qwen_mini()  # 4-layer AttentionLayer model
    _check_parity(model, arch_tag="", feature_flags=0,
                  ffn_kind="identity", sigmoid_kind="identity")
    print("test_attention_layer_parity OK")


def test_gemma_layer_parity():
    model = GM.build_gemma_mini()  # 2-layer GemmaLayer model
    _check_parity(model, arch_tag="gemma4", feature_flags=0x27f,
                  ffn_kind="silu", sigmoid_kind="silu")
    print("test_gemma_layer_parity OK")


def test_qwen_standard_and_hybrid_parity():
    model = HM.build_qwen_hybrid_mini()  # QwenStandard + QwenHybridSsm
    _check_parity(model, arch_tag="qwen35", feature_flags=0,
                  ffn_kind="identity", sigmoid_kind="silu")
    print("test_qwen_standard_and_hybrid_parity OK")


def test_streaming_directory_loadable_via_rust():
    """Sanity: the streaming output sits at the same path layout as the
    non-streaming output, so existing Rust loader tests would consume it
    without modification. We don't shell out to cargo here (Python test),
    just verify the three expected files exist with sane sizes."""
    model = D.build_qwen_mini()
    with tempfile.TemporaryDirectory() as td:
        SW.streaming_write_model_dir(model, td)
        for name in ("manifest.bin", "weights.bin", "comm_w.hex"):
            path = os.path.join(td, name)
            assert os.path.exists(path), f"{name} missing"
            assert os.path.getsize(path) > 0, f"{name} is empty"
        # comm_w.hex must be exactly 64 ASCII chars (no newline).
        with open(os.path.join(td, "comm_w.hex")) as f:
            hx = f.read()
        assert len(hx) == 64, f"comm_w.hex length: {len(hx)}"
    print("test_streaming_directory_loadable_via_rust OK")


if __name__ == "__main__":
    test_attention_layer_parity()
    test_gemma_layer_parity()
    test_qwen_standard_and_hybrid_parity()
    test_streaming_directory_loadable_via_rust()
    print("ALL streaming_writer TESTS PASSED")
