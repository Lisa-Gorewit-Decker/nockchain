"""FP8 weight-dequant validation for the group_0 (un-mined)
layers — faithful to the **canonical vLLM block-FP8 formula** and
to the **real shipped Pearl model's FP8 weights**.

Framing (PEARL_FP8_SCOPING.md): Pearl-the-protocol has NO FP8
(zk-pow is INT-only); Pearl's plugin delegates FP8 to vLLM's
stock `CompressedTensorsW8A16Fp8`. So the authoritative oracle is
(1) vLLM's own block-FP8 dequant — verbatim
`repeat_interleave(scale,b0,0/1)·fp8.float()`, from
`vllm/.../utils/fp8_utils.py:requant_weight_ue8m0_inplace`; (2)
an *independent* nested-loop reference of the same spec (so it's
not vectorized-vs-vectorized); (3) torch core's
`float8_e4m3fn→float32` (the IEEE-spec fp8 codec authority);
(4) the **real `down_proj` FP8 weights** from a shipped model,
anchored to a Python ground truth so a wrong read can't pass.

`fp8_block_dequant` is vLLM-free ⇒ runs in the fast fork venv:
`.venv/bin/python -m pytest tests/test_pearl_fp8_cpu.py -q`.
"""

from __future__ import annotations

import importlib.util as _ilu
import json
import os
import struct
from pathlib import Path

import torch

# Load the pure dequant in isolation (no vllm_miner/__init__,
# no vLLM) — `fp8_block_dequant` has zero plugin/vLLM deps.
_p = Path(__file__).resolve().parents[1] / "src" / "vllm_miner" / "pearl_gemm_cpu.py"
_s = _ilu.spec_from_file_location("pearl_gemm_cpu", _p)
_m = _ilu.module_from_spec(_s)
_s.loader.exec_module(_m)
fp8_block_dequant = _m.fp8_block_dequant

FP8 = torch.float8_e4m3fn


def _nested_block_ref(w_fp8, scale, b0, b1):
    """Independent (elementwise, no repeat_interleave) reference:
    wd[o,i] = float(w_fp8[o,i]) * scale[o//b0, i//b1]."""
    w = w_fp8.to(torch.float32)
    s = scale.to(torch.float32)
    O, I = w.shape
    out = torch.empty(O, I, dtype=torch.float32)
    for o in range(O):
        so = s[o // b0]
        for i in range(I):
            out[o, i] = w[o, i] * so[i // b1]
    return out


def _canonical_vllm_block(w_fp8, scale, b0, b1):
    """Verbatim vLLM block dequant (fp8_utils.py)."""
    s = scale.to(torch.float32)
    se = torch.repeat_interleave(s, b0, dim=0)
    se = torch.repeat_interleave(se, b1, dim=1)
    se = se[: w_fp8.shape[0], : w_fp8.shape[1]]
    return w_fp8.to(torch.float32) * se


# ── FP8-1 synthetic: the 3 scale-strategy branches ──────────────

def test_fp8_1_per_tensor_channel_block_match_independent_ref():
    torch.manual_seed(0)
    O, I, b0, b1 = 16, 24, 8, 8
    w = (torch.randn(O, I) * 6).to(FP8)

    # per-tensor
    s0 = torch.tensor(0.0137)
    d = fp8_block_dequant(w, s0, None, torch.float32)
    assert torch.equal(d, w.to(torch.float32) * s0)

    # per-(out-)channel ([O] and [O,1])
    sc = torch.rand(O) * 0.02 + 1e-3
    for sv in (sc, sc.reshape(O, 1)):
        d = fp8_block_dequant(w, sv, None, torch.float32)
        assert torch.equal(d, w.to(torch.float32) * sc.reshape(O, 1))

    # block [b0,b1] vs canonical AND independent nested-loop
    sb = torch.rand(O // b0, I // b1) * 0.02 + 1e-3
    d = fp8_block_dequant(w, sb, [b0, b1], torch.float32)
    assert torch.equal(d, _canonical_vllm_block(w, sb, b0, b1)), "≠ canonical vLLM"
    assert torch.allclose(d, _nested_block_ref(w, sb, b0, b1), atol=0, rtol=0), (
        "vectorized block dequant ≠ the independent elementwise spec"
    )
    # block inferred from the scale grid (block=None)
    assert torch.equal(d, fp8_block_dequant(w, sb, None, torch.float32))


def test_fp8_1_out_dtype_and_contiguous():
    w = (torch.randn(8, 8) * 4).to(FP8)
    s = torch.rand(1, 1) * 0.01 + 1e-3
    for dt in (torch.float16, torch.bfloat16, torch.float32):
        d = fp8_block_dequant(w, s, [8, 8], dt)
        assert d.dtype == dt and d.is_contiguous()


# ── FP8-2: the REAL shipped model's down_proj FP8 weights ───────
# Python ground truth (independent, recorded from the real file).
ST_HDR_LEN = 36_688
W_OFF = 1_052_508_160
S_OFF = 1_050_681_344
W_IN = 14_336                       # down_proj weight is [4096, 14336]
ORACLE_W0_F32 = [64.0, -80.0, -48.0, 4.5, -128.0, -30.0]    # w_fp8[0,:6]
ORACLE_W255_F32 = [-20.0, 32.0, 10.0, 3.0, 88.0, 8.0]        # w_fp8[255,250:256]
ORACLE_S22 = [
    [0.0001773834228515625, 0.0001392364501953125],
    [0.00019168853759765625, 0.00017642974853515625],
]
ORACLE_DEQ0 = [0.01135, -0.01419, -0.00851, 0.0008]          # deq[0,:4]
ORACLE_DEQ255 = [0.00176, 0.00053, 0.01553, 0.00141]         # deq[255,252:256]
ORACLE_SUM, ORACLE_MIN, ORACLE_MAX = -1.303, -0.0859, 0.079


def _read_real_fp8_tile(to_=256, ti=256):
    d = os.environ.get(
        "PEARL_MODEL_DIR", str(Path.home() / "Dev" / "Llama-3.1-8B-Instruct-pearl")
    )
    shard = Path(d) / "model-00001-of-00002.safetensors"
    if not shard.exists():
        return None
    with open(shard, "rb") as f:
        hl = struct.unpack("<Q", f.read(8))[0]
        assert hl == ST_HDR_LEN, "model differs from recorded oracle"
        ho = 8 + hl
        wb = bytearray()
        for r in range(to_):
            f.seek(ho + W_OFF + r * W_IN)
            wb += f.read(ti)
        f.seek(ho + S_OFF)
        sb = f.read(32 * 112 * 2)
    w = torch.frombuffer(bytes(wb), dtype=FP8).reshape(to_, ti)
    s = torch.frombuffer(bytes(sb), dtype=torch.bfloat16).reshape(32, 112)
    return w, s


def test_fp8_2_real_down_proj_dequant_is_canonical():
    rt = _read_real_fp8_tile()
    if rt is None:
        import pytest

        pytest.skip("Llama-3.1-8B absent (set PEARL_MODEL_DIR)")
    w, s_full = rt
    # raw-decode integrity anchors (a wrong read can't pass these)
    assert [round(x, 4) for x in w[0, :6].to(torch.float32).tolist()] == ORACLE_W0_F32
    assert [
        round(x, 4) for x in w[255, 250:256].to(torch.float32).tolist()
    ] == ORACLE_W255_F32
    s = s_full[:2, :2]  # the 256×256 tile spans 2×2 blocks of 128
    assert s.to(torch.float32).tolist() == ORACLE_S22

    deq = fp8_block_dequant(w, s, [128, 128], torch.float32)

    # == canonical vLLM block formula, bit-for-bit, on REAL weights
    assert torch.equal(deq, _canonical_vllm_block(w, s, 128, 128)), (
        "real-weight dequant ≠ the verbatim vLLM block formula"
    )
    # == independent nested-loop spec (genuinely independent impl)
    assert torch.allclose(
        deq, _nested_block_ref(w, s, 128, 128), atol=0, rtol=0
    ), "real-weight dequant ≠ the independent elementwise spec"

    # == the recorded Python ground truth
    assert [round(x, 5) for x in deq[0, :4].tolist()] == ORACLE_DEQ0
    assert [round(x, 5) for x in deq[255, 252:256].tolist()] == ORACLE_DEQ255
    assert abs(float(deq.sum()) - ORACLE_SUM) < 1e-2
    assert abs(float(deq.min()) - ORACLE_MIN) < 1e-3
    assert abs(float(deq.max()) - ORACLE_MAX) < 1e-3


def test_fp8_2_apply_is_x_at_wT(rt=None):
    rt = _read_real_fp8_tile()
    if rt is None:
        import pytest

        pytest.skip("model absent")
    w, s = rt[0], rt[1][:2, :2]
    deq = fp8_block_dequant(w, s, [128, 128], torch.float16)  # [256(out),256(in)]
    x = torch.randn(4, 256, dtype=torch.float16)              # [tok, in]
    got = torch.nn.functional.linear(x, deq)                  # x @ deqᵀ
    want = (x.to(torch.float32) @ deq.to(torch.float32).t())
    assert torch.allclose(got.to(torch.float32), want, atol=1e-2, rtol=1e-2), (
        "F.linear(x, dequant) ≠ x @ dequantᵀ (W8A16 path math)"
    )
