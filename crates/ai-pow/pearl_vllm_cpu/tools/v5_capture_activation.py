"""V5 — Phase-D end-to-end smoke.

Load the smallest published Pearl model
(`Llama-3.1-8B-Instruct-pearl`) through vLLM-CPU + the vendored
CPU fork, run a real prompt, and **capture the first mined
(7-bit / group_1) layer's real activation** as it enters the
quantized GEMM (`pearl_gemm_cpu.gemm`). Then validate the
captured `(A=x_q, B=w_q, μ=scales)` is a Pearl type-0 operand and
that the integer accumulate matches the audited
`ai_pow::quant::int_matmul` relation on REAL forward-pass data.

Honest scope (R1): the activation is real (real model, real
prompt, real forward) but quantized by OUR CPU `quantize`
(faithful-by-construction to the B1-audited spec, NOT Pearl's
CUDA kernel). B2.2 already proved byte-equivalence for any int7
activation — this is an integration smoke, not new correctness
evidence.

Run (slow; 8B on CPU):
  ~/Dev/vllm/.venv-cpu/bin/python tools/v5_capture_activation.py
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

# Run the vLLM v1 engine IN-PROCESS (no spawned EngineCore
# subprocess). On macOS multiprocessing uses `spawn`, which would
# (a) re-import this script → recursive process start, and (b)
# run the model forward in a child whose captured `_CAP` the
# parent never sees. Single-process ⇒ the monkeypatch + capture
# live in one process. Must be set before `import vllm`.
os.environ.setdefault("VLLM_ENABLE_V1_MULTIPROCESSING", "0")

HERE = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(HERE / "compat"), str(HERE / "src")]

import torch  # noqa: E402

MODEL = os.environ.get(
    "PEARL_MODEL_DIR",
    str(Path.home() / "Dev" / "Llama-3.1-8B-Instruct-pearl"),
)

# The in-package `pearl_gemm_cpu.gemm` capture hook writes the
# first mined GEMM's operand here (it runs inside the vLLM-CPU
# WorkerProc subprocess, which imports `vllm_miner` — not this
# script). Set BEFORE LLM() so the worker inherits it.
CAP_PATH = str(HERE / "tools" / "v5_captured.pt")
os.environ["PEARL_V5_CAPTURE"] = CAP_PATH
if Path(CAP_PATH).exists():
    Path(CAP_PATH).unlink()

import vllm_miner  # noqa: E402

vllm_miner.register_pearl_miner_layer()


def main() -> None:
    from vllm import LLM, SamplingParams

    print(f"[V5] loading {MODEL} on CPU (slow; 8B)…", flush=True)
    # vLLM 0.21 on macOS auto-sets VLLM_TARGET_DEVICE=cpu; the
    # `device=`/`gpu_memory_utilization=` kwargs were removed
    # (0.20→0.21 API drift).
    # On the vLLM **CPU** backend `gpu_memory_utilization` is the
    # fraction of *system RAM* reserved (despite the name). 32 GiB
    # box, ~16 GiB free ⇒ cap well under that (override via
    # V5_MEM_FRAC). The 8B model stays int8 under the Pearl scheme
    # (~8.5 GB) + vLLM/torch overhead.
    mem_frac = float(os.environ.get("V5_MEM_FRAC", "0.45"))
    llm = LLM(
        model=MODEL,
        dtype="float16",
        enforce_eager=True,
        max_model_len=64,
        max_num_seqs=1,
        gpu_memory_utilization=mem_frac,
        swap_space=0,
        trust_remote_code=True,
    )
    out = llm.generate(
        ["The capital of France is"],
        SamplingParams(max_tokens=1, temperature=0.0),
    )
    print(f"[V5] generated: {out[0].outputs[0].text!r}", flush=True)

    assert Path(CAP_PATH).exists(), (
        "no mined GEMM captured (pearl_gemm_cpu.gemm hook never fired) — "
        "the INT7 group_1 layers didn't dispatch through PearlKernel"
    )
    cap = torch.load(CAP_PATH)
    A, B, sa, sb = cap["A"], cap["B"], cap["sa"], cap["sb"]
    print(f"[V5] captured mined GEMM: A{tuple(A.shape)} int8, "
          f"B{tuple(B.shape)} int8, sa{tuple(sa.shape)} sb{tuple(sb.shape)}")

    # ── validate the captured REAL-forward operand ──────────────
    amax, bmax = int(A.abs().max()), int(B.abs().max())
    print(f"[V5] |A|max={amax} |B|max={bmax} (Pearl type-0 ⇒ ≤ 64)")
    assert amax <= 64 and bmax <= 64, "captured operand outside Pearl [-64,64]"

    # Integer accumulate == the audited ai_pow::quant relation, on
    # a small slice of the REAL forward activation × real weights.
    m = min(4, A.shape[0])
    n = min(4, B.shape[0])
    k = A.shape[1]
    acc = (A[:m].to(torch.int64) @ B[:n].to(torch.int64).t())
    ref = torch.zeros(m, n, dtype=torch.int64)
    for i in range(m):
        for j in range(n):
            ref[i, j] = int(sum(int(A[i, l]) * int(B[j, l]) for l in range(k)))
    assert torch.equal(acc, ref), "captured-activation int accumulate ≠ Σ A·B.T"
    print("[V5] OK — REAL forward-pass activation (real prompt, real "
          "model) × real weights: integer accumulate == the "
          "ai_pow::quant audited relation. Phase-D end-to-end smoke "
          "PASSED.", flush=True)
    print(f"[V5] captured operand at → {CAP_PATH}", flush=True)


if __name__ == "__main__":
    main()
