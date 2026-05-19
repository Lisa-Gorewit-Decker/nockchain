"""CPU reimplementation of the Pearl ``pearl_gemm`` CUDA extension.

Phase-D / integration fork (see
``crates/ai-pow-zk/docs/PEARL_VLLM_CPU_FORK_DESIGN.md``). Provides the
exact symbol set the vendored plugin imports from ``pearl_gemm``:

  * ``quantize``      — symmetric per-token int quant (faithful)
  * ``gemm``          — dequant matmul C = (A @ B.T)·sa·sb
                        (faithful; the inference / activation path)
  * mining-only       — ``noisy_gemm``, ``noise_gen``,
                        ``tensor_hash``, ``commitment_hash_from
                        _merkle_roots``, ``make_pow_target_tensor``,
                        ``get_host_signal_sync_size``,
                        ``get_required_scratchpad_bytes``: NOT
                        needed for inference/activation extraction
                        (``no_mining=True`` routes everything
                        through vanilla); stubbed to raise if ever
                        hit, so a wrong code path fails loudly
                        rather than silently mis-mining.

**Honest scope (R1).** This is OUR reimplementation,
faithful-by-construction to the B1-audited Pearl spec
(``crates/ai-pow/docs/B1_PEARL_FAITHFULNESS_AUDIT.md``,
``ai_pow::quant``). It is NOT Pearl's CUDA kernel. B2.2 already
proved the quant contract bit-lossless for any int7 activation;
this fork's only value is a Phase-D integration smoke.
"""

from __future__ import annotations

import os

import torch

# V5 capture hook (Phase-D smoke). When `PEARL_V5_CAPTURE=<path>`
# is set, the FIRST `gemm` call (the first mined / quantized GEMM
# of the forward) atomically writes its `(A,B,A_scales,B_scales)`
# to `<path>`, then computes normally. Lives IN the package so it
# runs inside whatever process executes the GEMM (vLLM-CPU runs
# the model in a WorkerProc subprocess that imports `vllm_miner`,
# not the caller's script).
_V5_DONE = False

__all__ = [
    "quantize",
    "gemm",
    "noisy_gemm",
    "noise_gen",
    "tensor_hash",
    "commitment_hash_from_merkle_roots",
    "make_pow_target_tensor",
    "get_host_signal_sync_size",
    "get_required_scratchpad_bytes",
]


def quantize(
    x: torch.Tensor,
    x_q: torch.Tensor,
    x_s: torch.Tensor,
    *,
    fast_math: bool = False,
    max_val: int = 63,
    smooth_scale: torch.Tensor | None = None,
) -> None:
    """Symmetric per-token quantization, written in place.

    Faithful to the Pearl spec / ``ai_pow::quant`` contract:
    per row (token) ``t``::

        y      = x[t] / smooth_scale            (if given)
        s[t]   = max(|y|) / max_val             (per-token scale)
        x_q[t] = clamp(round(y / s[t]),
                       -max_val, +max_val)       (int8)

    A zero token ⇒ ``s = 0`` ⇒ ``x_q = 0`` (no div-by-zero).
    ``x_q`` is int8 ``[num_tokens, k]``; ``x_s`` is fp32
    ``[num_tokens, 1]`` (matches the upstream out-params).
    """
    y = x
    if smooth_scale is not None:
        y = x / smooth_scale
    y = y.to(torch.float32)
    amax = y.abs().amax(dim=-1, keepdim=True)          # [T,1]
    s = amax / float(max_val)                          # [T,1] fp32
    nonzero = s > 0
    inv = torch.where(nonzero, 1.0 / s.clamp_min(torch.finfo(torch.float32).tiny),
                      torch.zeros_like(s))
    q = torch.round(y * inv).clamp_(-float(max_val), float(max_val))
    x_q.copy_(q.to(torch.int8))
    x_s.copy_(s)


def gemm(
    *,
    A: torch.Tensor,
    B: torch.Tensor,
    A_scales: torch.Tensor,
    B_scales: torch.Tensor,
    C: torch.Tensor,
    tile_size_m: int | None = None,
    tile_size_n: int | None = None,
    tile_size_k: int | None = None,
) -> None:
    """Dequantized matmul written in place: ``C = (A @ B.T) ·
    A_scales[:,None] · B_scales[None,:]``.

    ``A`` int8 ``[M,K]`` (per-token-quantized activation), ``B``
    int8 ``[N,K]`` (per-channel-quantized weight, NON-transposed
    — the upstream ``pearl_gemm_vanilla`` passes ``w_q`` as-is and
    computes ``A @ B.T``). The **integer accumulate**
    ``A @ B.T`` is exactly the relation
    ``ai_pow::quant::int_matmul`` proves bit-lossless. Tile sizes
    are ignored (correctness-only CPU path).
    """
    global _V5_DONE
    _cap = os.environ.get("PEARL_V5_CAPTURE")
    if _cap and not _V5_DONE:
        _V5_DONE = True
        tmp = _cap + f".{os.getpid()}.tmp"
        try:
            torch.save(
                {
                    "A": A.detach().cpu().clone(),
                    "B": B.detach().cpu().clone(),
                    "sa": A_scales.detach().cpu().clone(),
                    "sb": B_scales.detach().cpu().clone(),
                },
                tmp,
            )
            os.replace(tmp, _cap)  # atomic publish
        except Exception:  # never break the forward on capture
            pass

    acc = A.to(torch.int64) @ B.to(torch.int64).t()    # exact int32-domain accumulate
    out = acc.to(torch.float32) * A_scales.reshape(-1, 1).to(torch.float32) \
        * B_scales.reshape(1, -1).to(torch.float32)
    C.copy_(out.to(C.dtype))


def fp8_block_dequant(
    weight: torch.Tensor,
    weight_scale: torch.Tensor,
    block: tuple[int, int] | list[int] | None,
    out_dtype: torch.dtype,
) -> torch.Tensor:
    """Canonical compressed-tensors / vLLM **FP8 weight dequant**
    for the production models' group_0 layers (`float-quantized`,
    symmetric). Pure (no vLLM import) so it is unit-testable in
    isolation. `weight` is `[O, I]` (`torch.float8_e4m3fn` for the
    shipped models); `weight_scale` is one of:

      * scalar / numel 1                → per-tensor;
      * shape `[O]` or `[O, 1]`         → per-(out-)channel;
      * shape `[O//b0, I//b1]`          → **block** (`block=[b0,b1]`).

    Block dequant is the *verbatim* vLLM formula
    (`fp8_utils.requant_weight_ue8m0_inplace`'s dequant step):
    `repeat_interleave(scale, b0, 0)` then `(.., b1, 1)`, crop to
    `[O, I]`, multiply by `weight.float()`. Since Pearl's plugin
    delegates FP8 to vLLM (`PearlConfig` → `super()`), this *is*
    the authoritative reference (FP8 is NOT a Pearl-protocol op —
    see PEARL_FP8_SCOPING.md).
    """
    w = weight.to(torch.float32)
    s = weight_scale.to(torch.float32)
    O, I = w.shape
    if s.numel() == 1:
        wd = w * s.reshape(())
    elif s.shape == (O, 1) or (s.dim() == 1 and s.numel() == O):
        wd = w * s.reshape(O, 1)
    else:
        if block is not None:
            b0, b1 = int(block[0]), int(block[1])
        else:  # infer from the scale grid
            b0 = -(-O // s.shape[0])
            b1 = -(-I // s.shape[1])
        se = torch.repeat_interleave(s, b0, dim=0)
        se = torch.repeat_interleave(se, b1, dim=1)[:O, :I]
        wd = w * se
    return wd.to(out_dtype).contiguous()


def int_accumulate(A: torch.Tensor, B: torch.Tensor) -> torch.Tensor:
    """The bare mined integer accumulate ``A @ B.T`` (int64-exact)
    — the K-CPU-2 cross-check handle vs ``ai_pow::quant``."""
    return A.to(torch.int64) @ B.to(torch.int64).t()


# ── mining-only symbols (deferred; raise if reached) ────────────
class _MiningNotPorted(NotImplementedError):
    pass


def _mining_stub(name: str):
    def _f(*_a, **_k):
        raise _MiningNotPorted(
            f"pearl_gemm_cpu.{name}: the mining/noisy/commitment path "
            "is NOT ported (inference/activation fork only — run with "
            "no_mining=True so the vanilla path is used). The faithful "
            "noisy+commitment path lives in Rust `ai_pow` / the "
            "B1-audited reference."
        )
    return _f


noisy_gemm = _mining_stub("noisy_gemm")
noise_gen = _mining_stub("noise_gen")
tensor_hash = _mining_stub("tensor_hash")
commitment_hash_from_merkle_roots = _mining_stub("commitment_hash_from_merkle_roots")
make_pow_target_tensor = _mining_stub("make_pow_target_tensor")
get_host_signal_sync_size = _mining_stub("get_host_signal_sync_size")
get_required_scratchpad_bytes = _mining_stub("get_required_scratchpad_bytes")
extract_indices = _mining_stub("extract_indices")
get_host_signal_header = _mining_stub("get_host_signal_header")


class HostSignalStatus:  # mining host-signal enum stub (unused on inference path)
    IDLE = 0
    PENDING = 1
    DONE = 2


class HostSignalHeaderPinnedPool:  # mining pinned-pool stub (no-op)
    def __init__(self, *_a, **_k):
        raise _MiningNotPorted(
            "HostSignalHeaderPinnedPool: pinned-pool is mining-only; "
            "the inference fork never constructs it (no_mining=True)."
        )


__all__ += [
    "extract_indices",
    "get_host_signal_header",
    "HostSignalStatus",
    "HostSignalHeaderPinnedPool",
]
