"""End-to-end test of gguf_reader → calibrate → quantize_qwen.

Builds a tiny synthetic GGUF, calibrates statically, runs the
quantizer, and validates the emitted directory structure + comm_W.
A Rust integration test (`tests/oracle_quantized_synthetic.rs`)
exercises the same fixture from the consumer side via `Model::load`.
"""

from __future__ import annotations

import json
import os
import struct
import sys
import tempfile

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ORACLE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.insert(0, ORACLE_DIR)
sys.path.insert(0, SCRIPT_DIR)

import calibrate  # noqa: E402
import quantize_qwen  # noqa: E402
import synthetic_qwen_mini as D  # noqa: E402
from test_gguf_reader import write_tiny_gguf  # noqa: E402


VEC_DIR = os.path.join(ORACLE_DIR, "test_vectors", "quantized_synthetic")


def test_full_pipeline_and_emit_fixture():
    """Run gguf → calibrate → quantize and write the result to a stable
    test_vectors/ directory. The Rust integration test uses that path."""
    with tempfile.TemporaryDirectory() as td:
        gguf_path = os.path.join(td, "tiny.gguf")
        scales_path = os.path.join(td, "scales.json")
        write_tiny_gguf(gguf_path, hidden=8, intermediate=16, num_layers=2)

        # Calibrate.
        rc = calibrate.main(["--gguf", gguf_path, "--out", scales_path, "--mode", "static"])
        assert rc == 0
        scales = json.loads(open(scales_path).read())

        # Quantize and save into the stable fixture directory.
        os.makedirs(VEC_DIR, exist_ok=True)
        rc = quantize_qwen.main(
            [
                "--gguf",
                gguf_path,
                "--scales",
                scales_path,
                "--out",
                VEC_DIR,
                "--seq-len",
                "8",
                "--activation-tile",
                "2",
            ]
        )
        assert rc == 0

        # Validate output structure.
        for fname in ("manifest.bin", "weights.bin", "comm_w.hex"):
            assert os.path.exists(os.path.join(VEC_DIR, fname)), fname

        # comm_W file should be 64 hex chars.
        with open(os.path.join(VEC_DIR, "comm_w.hex")) as f:
            comm_hex = f.read().strip()
        assert len(comm_hex) == 64, len(comm_hex)
        # Bytes parse cleanly.
        bytes.fromhex(comm_hex)

        # Manifest starts with magic + version.
        with open(os.path.join(VEC_DIR, "manifest.bin"), "rb") as f:
            head = f.read(12)
        assert head[:8] == b"AIPOWVI1", head[:8]
        assert struct.unpack("<I", head[8:12])[0] == 2  # Phase 2.10 manifest v2

        print(
            f"test_full_pipeline_and_emit_fixture OK "
            f"(manifest={os.path.getsize(os.path.join(VEC_DIR, 'manifest.bin'))}B, "
            f"weights={os.path.getsize(os.path.join(VEC_DIR, 'weights.bin'))}B, "
            f"comm_W={comm_hex[:16]}...)"
        )


def test_quantize_round_trip_in_memory():
    """Build a tiny GGUF, calibrate, quantize, and verify the resulting
    Model can be re-encoded to the same comm_W via the disk-format helpers."""
    with tempfile.TemporaryDirectory() as td:
        gguf_path = os.path.join(td, "tiny.gguf")
        scales_path = os.path.join(td, "scales.json")
        out_dir = os.path.join(td, "out")
        write_tiny_gguf(gguf_path, hidden=8, intermediate=16, num_layers=1)
        calibrate.main(["--gguf", gguf_path, "--out", scales_path, "--mode", "static"])

        scales = json.loads(open(scales_path).read())
        import gguf_reader

        gm = gguf_reader.read_model(gguf_path)
        model, comm_w = quantize_qwen.quantize_to_model(
            gm, scales, seq_len=8, activation_tile=2
        )
        # Sanity: re-running compute_comm_w on the same model gives the same hash.
        assert D.compute_comm_w(model) == comm_w
        # And the saved file's hex matches.
        quantize_qwen.save(model, comm_w, out_dir)
        with open(os.path.join(out_dir, "comm_w.hex")) as f:
            assert f.read().strip() == comm_w.hex()
        print("test_quantize_round_trip_in_memory OK")


if __name__ == "__main__":
    test_full_pipeline_and_emit_fixture()
    test_quantize_round_trip_in_memory()
    print("all quantize_qwen tests passed")
