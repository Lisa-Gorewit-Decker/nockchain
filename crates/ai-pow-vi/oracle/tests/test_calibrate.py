"""Self-test for oracle/calibrate.py."""

from __future__ import annotations

import json
import os
import sys
import tempfile

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ORACLE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.insert(0, ORACLE_DIR)
sys.path.insert(0, SCRIPT_DIR)

import calibrate  # noqa: E402
from test_gguf_reader import write_tiny_gguf  # noqa: E402


def test_static_calibration_round_trip():
    with tempfile.TemporaryDirectory() as td:
        gguf_path = os.path.join(td, "tiny.gguf")
        write_tiny_gguf(gguf_path, hidden=8, intermediate=16, num_layers=2)
        scales_path = os.path.join(td, "scales.json")
        rc = calibrate.main(
            ["--gguf", gguf_path, "--out", scales_path, "--mode", "static"]
        )
        assert rc == 0
        scales = json.loads(open(scales_path).read())
        assert scales["model_arch"] == "qwen3"
        assert scales["mode"] == "static"
        assert scales["num_layers"] == 2
        assert scales["hidden"] == 8
        assert scales["intermediate"] == 16
        # Every weight tensor we emit should have a scale.
        ws = scales["weight_scales"]
        assert "embed" in ws
        assert "final_norm.gamma" in ws
        assert "lm_head" in ws
        for n in range(2):
            for sub in (
                "norm1.gamma",
                "attn.w_q",
                "attn.w_k",
                "attn.w_v",
                "attn.w_o",
                "norm2.gamma",
                "ffn.w_gate",
                "ffn.w_up",
                "ffn.w_down",
            ):
                assert f"layer[{n}].{sub}" in ws, sub
        # All weight scales are positive integers.
        for k, v in ws.items():
            assert isinstance(v, int)
            assert v >= 1, f"{k} scale is {v}"
        # Activation scales are sane defaults.
        assert "default" in scales["activation_scales"]
        for n in range(2):
            assert f"layer[{n}].attn.q" in scales["activation_scales"]
            assert f"layer[{n}].ffn.gate" in scales["activation_scales"]
            assert f"layer[{n}].dnet.q" in scales["activation_scales"]
        print("test_static_calibration_round_trip OK")


def test_zero_weight_scale_floors_at_one():
    """A tensor of all zeros should not produce a Scale numerator of 0."""
    s = calibrate.f32_to_scale_num(0.0)
    assert s >= 1
    s = calibrate.f32_to_scale_num(-0.5)
    assert s >= 1
    s = calibrate.f32_to_scale_num(1.0 / 127.0)
    assert s >= 1
    print("test_zero_weight_scale_floors_at_one OK")


def test_max_abs_yields_127_at_endpoints():
    """A tensor with max-abs of exactly 1.0 should produce scale ≈ 1/127,
    so int8 would map 1.0 → 127."""
    import numpy as np

    arr = np.array([0.0, 0.5, 1.0, -1.0], dtype=np.float32)
    s_f = calibrate.derive_weight_scale(arr)
    assert abs(s_f - 1.0 / 127.0) < 1e-9, s_f
    s_num = calibrate.f32_to_scale_num(s_f)
    # Quantizing 1.0 with this scale: w_q = round(1.0 / s_f) = 127.
    assert abs(round(1.0 / s_f) - 127) <= 0
    print("test_max_abs_yields_127_at_endpoints OK")


def test_activation_mode_raises_not_implemented():
    """Activation mode is a documented stub."""
    with tempfile.TemporaryDirectory() as td:
        gguf_path = os.path.join(td, "tiny.gguf")
        write_tiny_gguf(gguf_path)
        prompts = os.path.join(td, "prompts.txt")
        with open(prompts, "w") as f:
            f.write("0,1,2,3\n")
        try:
            calibrate.main(
                ["--gguf", gguf_path, "--out", "/tmp/_unused.json",
                 "--mode", "activation", "--prompts", prompts]
            )
        except NotImplementedError:
            print("test_activation_mode_raises_not_implemented OK")
            return
        raise AssertionError("expected NotImplementedError for activation mode")


if __name__ == "__main__":
    test_zero_weight_scale_floors_at_one()
    test_max_abs_yields_127_at_endpoints()
    test_static_calibration_round_trip()
    test_activation_mode_raises_not_implemented()
    print("all calibrate tests passed")
