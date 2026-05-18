"""V2 validation — the CPU `pearl_gemm` reimplementation is
faithful to the B1-audited Pearl / `ai_pow::quant` spec.

K-CPU-1  quantize: symmetric per-token, saturation, zero-token,
                   smooth_scale, dequant round-trip.
K-CPU-2  gemm: the integer accumulate == Σ x_q·w_q (exactly the
               `ai_pow::quant::int_matmul` relation Rust
               B2-contract KAT'd bit-lossless), incl. on a REAL
               Llama-3.1-8B `gate_proj` tile (read via the same
               offsets, anchored to the Python oracle that B1.1a
               uses) — ties this Python kernel to the same
               real-data spec B1.1 validated in Rust.

Run: `.venv/bin/python -m pytest tests/ -q` from
`crates/ai-pow/pearl_vllm_cpu/`.
"""

from __future__ import annotations

import json
import os
import struct
from pathlib import Path

import torch

# Load pearl_gemm_cpu in ISOLATION (it has zero plugin deps) —
# bypass vllm_miner/__init__.py, which eagerly pulls miner_base
# (a V4 stub concern, not V2's faithful-core validation).
import importlib.util as _ilu  # noqa: E402

_pgc_path = Path(__file__).resolve().parents[1] / "src" / "vllm_miner" / "pearl_gemm_cpu.py"
_spec = _ilu.spec_from_file_location("pearl_gemm_cpu", _pgc_path)
_pgc = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_pgc)
gemm, int_accumulate, quantize = _pgc.gemm, _pgc.int_accumulate, _pgc.quantize

MAX7 = 63


def _ref_quant(x: torch.Tensor, max_val: int, smooth=None):
    y = x if smooth is None else x / smooth
    y = y.to(torch.float64)
    s = y.abs().amax(-1, keepdim=True) / max_val
    q = torch.where(s > 0, torch.round(y / s.clamp_min(1e-300)),
                    torch.zeros_like(y)).clamp(-max_val, max_val)
    return q.to(torch.int8), s.to(torch.float32)


# ── K-CPU-1 ─────────────────────────────────────────────────────

def test_kcpu1_quantize_matches_reference_and_saturates():
    torch.manual_seed(1)
    for shape in [(4, 16), (1, 1), (8, 4096), (3, 7)]:
        x = (torch.randn(*shape) * 5.0)
        xq = torch.empty(shape, dtype=torch.int8)
        xs = torch.empty((shape[0], 1), dtype=torch.float32)
        quantize(x, xq, xs, max_val=MAX7)
        rq, rs = _ref_quant(x, MAX7)
        assert torch.equal(xq, rq), f"quantize ≠ reference at {shape}"
        assert torch.allclose(xs, rs, atol=0, rtol=1e-6)
        assert int(xq.abs().max()) <= MAX7, "must saturate within ±max_val"

    # Explicit saturation: a huge outlier ⇒ ±63 exactly.
    x = torch.tensor([[1.0, 1e9, -1e9, 0.5]])
    xq = torch.empty_like(x, dtype=torch.int8)
    xs = torch.empty((1, 1), dtype=torch.float32)
    quantize(x, xq, xs, max_val=MAX7)
    assert xq[0, 1].item() == MAX7 and xq[0, 2].item() == -MAX7

    # Zero token ⇒ scale 0, q all zero (no div-by-zero / NaN).
    z = torch.zeros(2, 5)
    zq = torch.empty(2, 5, dtype=torch.int8)
    zs = torch.empty(2, 1, dtype=torch.float32)
    quantize(z, zq, zs, max_val=MAX7)
    assert torch.count_nonzero(zq) == 0 and float(zs.sum()) == 0.0

    # smooth_scale path.
    x = torch.randn(4, 8)
    ss = torch.rand(8) + 0.1
    xq = torch.empty(4, 8, dtype=torch.int8)
    xs = torch.empty(4, 1, dtype=torch.float32)
    quantize(x, xq, xs, max_val=MAX7, smooth_scale=ss)
    rq, rs = _ref_quant(x, MAX7, smooth=ss)
    assert torch.equal(xq, rq) and torch.allclose(xs, rs, rtol=1e-6)


def test_kcpu1_dequant_roundtrip_is_within_quant_error():
    torch.manual_seed(2)
    x = torch.randn(6, 256) * 3.0
    xq = torch.empty(6, 256, dtype=torch.int8)
    xs = torch.empty(6, 1, dtype=torch.float32)
    quantize(x, xq, xs, max_val=MAX7)
    deq = xq.to(torch.float32) * xs
    # per-token max error ≤ half a quant step (= scale/2)
    assert torch.all((x - deq).abs().amax(-1, keepdim=True) <= xs * 0.5 + 1e-5)


# ── K-CPU-2 ─────────────────────────────────────────────────────

def test_kcpu2_gemm_integer_accumulate_equals_audited_relation():
    """`gemm`'s accumulate == Σ A·B.T == `ai_pow::quant::int_matmul`
    (Rust B2-contract proved that relation bit-lossless)."""
    torch.manual_seed(3)
    M, K, N = 5, 64, 7
    A = torch.randint(-MAX7, MAX7 + 1, (M, K), dtype=torch.int8)
    B = torch.randint(-MAX7, MAX7 + 1, (N, K), dtype=torch.int8)
    sa = torch.rand(M) * 0.01 + 1e-3
    sb = torch.rand(N) * 0.02 + 1e-3
    C = torch.empty((M, N), dtype=torch.bfloat16)
    gemm(A=A, B=B, A_scales=sa, B_scales=sb, C=C)

    # independent int reference (the exact ai_pow::quant relation)
    want_acc = torch.zeros(M, N, dtype=torch.int64)
    for m in range(M):
        for n in range(N):
            want_acc[m, n] = int(
                sum(int(A[m, l]) * int(B[n, l]) for l in range(K))
            )
    assert torch.equal(int_accumulate(A, B), want_acc), "int accumulate ≠ Σ A·B.T"
    # dequant matches the same accumulate scaled (bf16 tolerance)
    want = (want_acc.to(torch.float32) * sa[:, None] * sb[None, :]).to(torch.bfloat16)
    assert torch.equal(C, want), "gemm dequant ≠ scaled accumulate"


def _read_real_gate_proj_tile(out_rows=64, k=4096):
    """Real Llama-3.1-8B layer-0 gate_proj tile via the SAME
    offsets the Rust B1.1a integrity anchor uses; asserted vs the
    Python oracle so a wrong read can't yield a silent result."""
    d = os.environ.get(
        "PEARL_MODEL_DIR",
        str(Path.home() / "Dev" / "Llama-3.1-8B-Instruct-pearl"),
    )
    shard = Path(d) / "model-00001-of-00002.safetensors"
    if not shard.exists():
        return None
    with open(shard, "rb") as f:
        hdr_len = struct.unpack("<Q", f.read(8))[0]
        assert hdr_len == 36_688, "model differs from recorded oracle"
        hdr = json.loads(f.read(hdr_len))
        t = hdr["model.layers.0.mlp.gate_proj.weight"]
        assert t["dtype"] == "I8" and t["data_offsets"][0] == 2_512_125_952
        f.seek(8 + hdr_len + t["data_offsets"][0])
        raw = f.read(out_rows * k)
    w = torch.frombuffer(bytearray(raw), dtype=torch.int8).reshape(out_rows, k)
    # B1.1a oracle anchors:
    assert w[0, :8].tolist() == [-23, -10, -2, -1, 11, 5, -5, -35]
    assert int(w.min()) == -61 and int(w.max()) == 61
    assert int(w.to(torch.int64).sum()) == -1690
    return w


def test_kcpu2_on_real_llama8b_gate_proj_tile():
    """End-to-end on REAL model weights: quantize a deterministic
    activation, gemm against the real `gate_proj` int7 tile, and
    assert the integer accumulate == the independent Σ x_q·w_q
    reference (the B1.1 real-data property, now in the Python
    CPU kernel)."""
    w = _read_real_gate_proj_tile()
    if w is None:
        import pytest
        pytest.skip("Llama-3.1-8B absent (set PEARL_MODEL_DIR)")
    n, k = w.shape  # [64, 4096], int7 ∈ [-61,61] ⊂ [-64,64]
    assert int(w.abs().max()) <= 64, "real int7 weights ∈ Pearl [-64,64]"

    tok = 8
    x = torch.sin(torch.arange(tok * k, dtype=torch.float32)).reshape(tok, k)
    xq = torch.empty(tok, k, dtype=torch.int8)
    xs = torch.empty(tok, 1, dtype=torch.float32)
    quantize(x, xq, xs, max_val=MAX7)
    sb = torch.full((n,), 0.002, dtype=torch.float32)
    C = torch.empty((tok, n), dtype=torch.bfloat16)
    gemm(A=xq, B=w, A_scales=xs.squeeze(-1), B_scales=sb, C=C)

    acc = int_accumulate(xq, w)
    want = torch.zeros(tok, n, dtype=torch.int64)
    for m in range(tok):
        for j in range(n):
            want[m, j] = int(sum(int(xq[m, l]) * int(w[j, l]) for l in range(k)))
    assert torch.equal(acc, want), (
        "CPU gemm integer accumulate ≠ Σ x_q·w_q on the REAL "
        "Llama-3.1-8B gate_proj tile (the B1.1 real-data spec)"
    )
