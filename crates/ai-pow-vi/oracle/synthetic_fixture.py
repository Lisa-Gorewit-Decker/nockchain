"""Generate synthetic op fixtures into oracle/test_vectors/.

Each fixture is `<op>/{input.bin, output.bin, meta.txt}` (or with
multiple input files for ops that take more than one tensor). The Rust
test driver `tests/oracle_op_vectors.rs` loads everything and asserts
byte-equality against its in-tree implementation.

Run with:
    python oracle/synthetic_fixture.py

Adding a new fixture: extend the generators below, document the shape in
`meta.txt`, and add a corresponding case to `oracle_op_vectors.rs`.
"""

from __future__ import annotations

import os
import sys

import numpy as np

import reference_ops as R
import synth_prompt_oracle as PromptOracle

ROOT = os.path.dirname(os.path.abspath(__file__))
VEC_DIR = os.path.join(ROOT, "test_vectors")


def _meta(path: str, kvs: dict[str, object]) -> None:
    with open(path, "w") as f:
        f.write(" ".join(f"{k}={v}" for k, v in kvs.items()) + "\n")


def _ensure(path: str) -> str:
    os.makedirs(path, exist_ok=True)
    return path


def gen_rescale_sweep() -> None:
    """1024 i32 inputs × scale=0.5 (banker's rounding sweep), → 1024 i8."""
    out_dir = _ensure(os.path.join(VEC_DIR, "rescale_sweep"))
    scale = R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 1))
    inputs = list(range(-512, 512))
    outputs = [R.rescale_and_requantize(v, scale) for v in inputs]
    R.write_i32(os.path.join(out_dir, "input.bin"), inputs)
    R.write_i8(os.path.join(out_dir, "output.bin"), outputs)
    _meta(
        os.path.join(out_dir, "meta.txt"),
        {"len": len(inputs), "scale_num": scale.num},
    )


def gen_matmul_canonical() -> None:
    """(4, 8) × (8, 4) → 16 i32 outputs from canonical LCG inputs."""
    out_dir = _ensure(os.path.join(VEC_DIR, "matmul_canonical"))
    m, k, n = 4, 8, 4
    a = R.canonical_input_i8(m * k, 0xFEED_BEEF_CAFE_BABE)
    b = R.canonical_input_i8(k * n, 0x0123_4567_89AB_CDEF)
    out = R.matmul_int8(a, b, m, k, n)
    R.write_i8(os.path.join(out_dir, "a.bin"), a)
    R.write_i8(os.path.join(out_dir, "b.bin"), b)
    R.write_i32(os.path.join(out_dir, "output.bin"), out)
    _meta(os.path.join(out_dir, "meta.txt"), {"m": m, "k": k, "n": n})


def gen_rmsnorm_canonical() -> None:
    """64-wide RMSNorm with canonical inputs. Output is i32 vector."""
    out_dir = _ensure(os.path.join(VEC_DIR, "rmsnorm_canonical"))
    hidden = 64
    inp = R.canonical_input_i8(hidden, 0xAAAA_BBBB_CCCC_DDDE)
    gamma = R.canonical_input_i8(hidden, 0x1111_2222_3333_4444)
    out = R.rmsnorm(inp, gamma, eps_q=R.DEFAULT_EPS_Q)
    R.write_i8(os.path.join(out_dir, "input.bin"), inp)
    R.write_i8(os.path.join(out_dir, "gamma.bin"), gamma)
    R.write_i32(os.path.join(out_dir, "output.bin"), out)
    _meta(os.path.join(out_dir, "meta.txt"), {"hidden": hidden, "eps_q": R.DEFAULT_EPS_Q})


def gen_layernorm_canonical() -> None:
    """64-wide LayerNorm. Output is i32 vector."""
    out_dir = _ensure(os.path.join(VEC_DIR, "layernorm_canonical"))
    hidden = 64
    inp = R.canonical_input_i8(hidden, 0x9999_AAAA_BBBB_CCCC)
    gamma = R.canonical_input_i8(hidden, 0x1357_2468_ACEF_BD13)
    beta = R.canonical_input_i8(hidden, 0x4242_4242_4242_4242)
    out = R.layernorm(inp, gamma, beta, eps_q=R.DEFAULT_EPS_Q)
    R.write_i8(os.path.join(out_dir, "input.bin"), inp)
    R.write_i8(os.path.join(out_dir, "gamma.bin"), gamma)
    R.write_i8(os.path.join(out_dir, "beta.bin"), beta)
    R.write_i32(os.path.join(out_dir, "output.bin"), out)
    _meta(os.path.join(out_dir, "meta.txt"), {"hidden": hidden, "eps_q": R.DEFAULT_EPS_Q})


def gen_softmax_canonical() -> None:
    """16-position softmax with the same hand-coded decay LUT the Rust pin uses."""
    out_dir = _ensure(os.path.join(VEC_DIR, "softmax_canonical"))
    table = []
    for i in range(256):
        v = (1 << 16) >> i if i < 16 else 0
        # Rust's wrapping_shr behavior matches Python `>>` for non-negative i32.
        table.append(v)
    lut = R.ExpLut(table=tuple(table))
    scores = [(i * 3) % 17 - 6 for i in range(16)]
    out = R.softmax_int(scores, lut)
    R.write_i32(os.path.join(out_dir, "scores.bin"), scores)
    # LUT is 256 i32 LE bytes (1024 bytes total).
    np.array(table, dtype=np.int32).tofile(os.path.join(out_dir, "lut.bin"))
    R.write_i8(os.path.join(out_dir, "output.bin"), out)
    _meta(os.path.join(out_dir, "meta.txt"), {"len": len(scores)})


def gen_synth_prompt_canonical() -> None:
    """32-token prompt with the same fixture the Rust pin uses."""
    out_dir = _ensure(os.path.join(VEC_DIR, "synth_prompt_canonical"))
    block = b"ai-pow-vi pin block-commitment v1"
    model_id = bytes([0xA5] * 32)
    reserved = [0, 1, 2]
    seq_len = 32
    vocab = 256
    out = PromptOracle.synth_prompt(block, model_id, seq_len, vocab, reserved)
    # Tokens are u32 LE.
    np.array(out, dtype=np.uint32).tofile(os.path.join(out_dir, "tokens.bin"))
    with open(os.path.join(out_dir, "block.bin"), "wb") as f:
        f.write(block)
    with open(os.path.join(out_dir, "model_id.bin"), "wb") as f:
        f.write(model_id)
    np.array(reserved, dtype=np.uint32).tofile(os.path.join(out_dir, "reserved.bin"))
    _meta(
        os.path.join(out_dir, "meta.txt"),
        {"seq_len": seq_len, "vocab": vocab, "n_reserved": len(reserved)},
    )


def gen_ffn_canonical() -> None:
    """Small SwiGLU FFN (m=2, hidden=8, intermediate=16) with identity activation."""
    out_dir = _ensure(os.path.join(VEC_DIR, "ffn_canonical"))
    m, hidden, intermediate = 2, 8, 16
    inp = R.canonical_input_i8(m * hidden, 0xFEED_FACE_DEAD_BEEF)
    w_gate = R.canonical_input_i8(hidden * intermediate, 0xA1A1_B2B2_C3C3_D4D4)
    w_up = R.canonical_input_i8(hidden * intermediate, 0xE5E5_F6F6_0707_1818)
    w_down = R.canonical_input_i8(intermediate * hidden, 0x2929_3A3A_4B4B_5C5C)
    # Identity LUT: byte b → i8 value (b - 128). Same as Rust ActivationLut::identity.
    identity_lut = bytes((i & 0xFF) - 128 & 0xFF for i in range(256))
    scales = R.FfnScales(
        gate=R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 6)),
        up=R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 6)),
        mid=R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 4)),
        down=R.Scale(num=1 << (R.SCALE_DENOM_LOG2 - 6)),
    )
    out = R.ffn_forward(
        inp, w_gate, w_up, w_down, identity_lut, scales, m, hidden, intermediate
    )
    R.write_i8(os.path.join(out_dir, "input.bin"), inp)
    R.write_i8(os.path.join(out_dir, "w_gate.bin"), w_gate)
    R.write_i8(os.path.join(out_dir, "w_up.bin"), w_up)
    R.write_i8(os.path.join(out_dir, "w_down.bin"), w_down)
    R.write_i8(os.path.join(out_dir, "output.bin"), out)
    _meta(
        os.path.join(out_dir, "meta.txt"),
        {
            "m": m,
            "hidden": hidden,
            "intermediate": intermediate,
            "gate_scale": scales.gate.num,
            "up_scale": scales.up.num,
            "mid_scale": scales.mid.num,
            "down_scale": scales.down.num,
        },
    )


def main() -> int:
    os.makedirs(VEC_DIR, exist_ok=True)
    gen_rescale_sweep()
    gen_matmul_canonical()
    gen_rmsnorm_canonical()
    gen_layernorm_canonical()
    gen_softmax_canonical()
    gen_ffn_canonical()
    gen_synth_prompt_canonical()
    print(f"wrote test vectors to {VEC_DIR}/", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
