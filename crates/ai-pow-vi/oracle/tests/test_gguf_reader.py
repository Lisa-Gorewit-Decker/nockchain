"""Self-test for oracle/gguf_reader.py.

Builds a tiny synthetic GGUF blob with one Attention block + one
embedding + one final norm, then exercises `read_model` end-to-end.
Run with:

    /tmp/aipow_oracle_venv/bin/python oracle/tests/test_gguf_reader.py
"""

from __future__ import annotations

import os
import sys
import tempfile

import gguf
import numpy as np

# Allow running from the oracle/tests subdirectory.
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ORACLE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.insert(0, ORACLE_DIR)

import gguf_reader as G  # noqa: E402


def write_tiny_gguf(
    path: str,
    hidden: int = 8,
    intermediate: int = 16,
    num_layers: int = 1,
    context_length: int = 8,
    vocab_size: int = 16,
):
    """Write a one-attention-block GGUF with f32 tensors."""
    w = gguf.GGUFWriter(path, "qwen3")
    w.add_string("general.architecture", "qwen3")
    w.add_uint32("general.alignment", 32)
    w.add_uint32("qwen3.block_count", num_layers)
    w.add_uint32("qwen3.embedding_length", hidden)
    w.add_uint32("qwen3.feed_forward_length", intermediate)
    w.add_uint32("qwen3.attention.head_count", 2)
    w.add_uint32("qwen3.attention.head_count_kv", 1)
    w.add_uint32("qwen3.rope.dimension_count", 4)
    w.add_float32("qwen3.rope.freq_base", 10000.0)
    w.add_uint32("qwen3.context_length", context_length)
    w.add_uint32("qwen3.vocab_size", vocab_size)

    vocab = 16
    head_dim = 4
    num_q = 2
    num_kv = 1

    # token_embd: (vocab, hidden) row-major.
    embed = np.arange(vocab * hidden, dtype=np.float32).reshape(vocab, hidden)
    w.add_tensor("token_embd.weight", embed)

    for n in range(num_layers):
        # Linear weights are (out, in) in HF / GGUF convention.
        w.add_tensor(f"blk.{n}.attn_norm.weight", np.ones(hidden, dtype=np.float32) * 0.5)
        w.add_tensor(
            f"blk.{n}.attn_q.weight",
            np.arange(num_q * head_dim * hidden, dtype=np.float32).reshape(
                num_q * head_dim, hidden
            )
            * 0.01,
        )
        w.add_tensor(
            f"blk.{n}.attn_k.weight",
            np.arange(num_kv * head_dim * hidden, dtype=np.float32).reshape(
                num_kv * head_dim, hidden
            )
            * 0.02,
        )
        w.add_tensor(
            f"blk.{n}.attn_v.weight",
            np.arange(num_kv * head_dim * hidden, dtype=np.float32).reshape(
                num_kv * head_dim, hidden
            )
            * 0.03,
        )
        w.add_tensor(
            f"blk.{n}.attn_output.weight",
            np.arange(hidden * num_q * head_dim, dtype=np.float32).reshape(
                hidden, num_q * head_dim
            )
            * 0.04,
        )
        w.add_tensor(f"blk.{n}.ffn_norm.weight", np.ones(hidden, dtype=np.float32) * 0.7)
        w.add_tensor(
            f"blk.{n}.ffn_gate.weight",
            np.arange(intermediate * hidden, dtype=np.float32).reshape(intermediate, hidden)
            * 0.005,
        )
        w.add_tensor(
            f"blk.{n}.ffn_up.weight",
            np.arange(intermediate * hidden, dtype=np.float32).reshape(intermediate, hidden)
            * 0.006,
        )
        w.add_tensor(
            f"blk.{n}.ffn_down.weight",
            np.arange(hidden * intermediate, dtype=np.float32).reshape(hidden, intermediate)
            * 0.007,
        )
    w.add_tensor("output_norm.weight", np.ones(hidden, dtype=np.float32) * 0.9)
    w.add_tensor(
        "output.weight",
        np.arange(vocab * hidden, dtype=np.float32).reshape(vocab, hidden) * 0.0001,
    )

    w.write_header_to_file()
    w.write_kv_data_to_file()
    w.write_tensors_to_file()
    w.close()


def test_round_trip():
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "tiny.gguf")
        write_tiny_gguf(path)
        model = G.read_model(path)
        a = model.arch
        assert a.name == "qwen3", a
        assert a.num_layers == 1
        assert a.hidden == 8
        assert a.intermediate == 16
        assert a.num_q_heads == 2
        assert a.num_kv_heads == 1
        assert a.head_dim == 4
        assert a.vocab_size == 16
        assert a.max_position == 4

        names = sorted(model.tensors.keys())
        # Required canonical names for one attention layer:
        want = sorted(
            [
                "embed",
                "final_norm.gamma",
                "lm_head",
                "layer[0].norm1.gamma",
                "layer[0].attn.w_q",
                "layer[0].attn.w_k",
                "layer[0].attn.w_v",
                "layer[0].attn.w_o",
                "layer[0].norm2.gamma",
                "layer[0].ffn.w_gate",
                "layer[0].ffn.w_up",
                "layer[0].ffn.w_down",
            ]
        )
        assert names == want, f"got {names}"

        embed = model.tensors["embed"]
        # Embed kept as (vocab, hidden) for our canonical row-major layout.
        assert embed.shape == (16, 8) or embed.shape == (16 * 8,), embed.shape

        # Linear weights are flattened col-major; W_q stored as (in*out,) = 8*8=64.
        assert model.tensors["layer[0].attn.w_q"].shape == (8 * 8,)
        assert model.tensors["layer[0].ffn.w_gate"].shape == (8 * 16,)
        assert model.tensors["layer[0].ffn.w_down"].shape == (16 * 8,)
        # Norms keep (hidden,) shape.
        assert model.tensors["layer[0].norm1.gamma"].shape == (8,)
        assert model.tensors["final_norm.gamma"].shape == (8,)
        print("test_round_trip OK")


def test_unknown_tensor_skipped():
    """Tensors that don't match any canonical mapping are dropped, not crashed."""
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "tiny_with_extra.gguf")
        w = gguf.GGUFWriter(path, "qwen3")
        w.add_string("general.architecture", "qwen3")
        w.add_uint32("qwen3.block_count", 0)
        w.add_uint32("qwen3.embedding_length", 4)
        w.add_uint32("qwen3.feed_forward_length", 8)
        w.add_uint32("qwen3.attention.head_count", 1)
        w.add_uint32("qwen3.attention.head_count_kv", 1)
        w.add_tensor("token_embd.weight", np.zeros((4, 4), dtype=np.float32))
        w.add_tensor("rope_freqs.weight", np.zeros(8, dtype=np.float32))  # unknown
        w.add_tensor("output_norm.weight", np.zeros(4, dtype=np.float32))
        w.write_header_to_file()
        w.write_kv_data_to_file()
        w.write_tensors_to_file()
        w.close()

        model = G.read_model(path)
        assert "embed" in model.tensors
        assert "final_norm.gamma" in model.tensors
        assert "rope_freqs.weight" not in model.tensors  # silently dropped
        # Did not include any layer.* names since num_layers=0
        layer_names = [n for n in model.tensors if n.startswith("layer[")]
        assert layer_names == []
        print("test_unknown_tensor_skipped OK")


def test_extra_aliases_override_default():
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "tiny_aliases.gguf")
        w = gguf.GGUFWriter(path, "qwen3")
        w.add_string("general.architecture", "qwen3")
        w.add_uint32("qwen3.block_count", 1)
        w.add_uint32("qwen3.embedding_length", 4)
        w.add_uint32("qwen3.feed_forward_length", 8)
        w.add_uint32("qwen3.attention.head_count", 1)
        w.add_uint32("qwen3.attention.head_count_kv", 1)
        w.add_tensor("token_embd.weight", np.zeros((4, 4), dtype=np.float32))
        w.add_tensor("blk.0.custom_q.weight", np.zeros((4, 4), dtype=np.float32))
        w.write_header_to_file()
        w.write_kv_data_to_file()
        w.write_tensors_to_file()
        w.close()

        model = G.read_model(
            path,
            extra_tensor_aliases={"blk.0.custom_q.weight": "attn.w_q"},
        )
        assert "layer[0].attn.w_q" in model.tensors, list(model.tensors.keys())
        print("test_extra_aliases_override_default OK")


if __name__ == "__main__":
    test_round_trip()
    test_unknown_tensor_skipped()
    test_extra_aliases_override_default()
    print("all gguf_reader tests passed")
