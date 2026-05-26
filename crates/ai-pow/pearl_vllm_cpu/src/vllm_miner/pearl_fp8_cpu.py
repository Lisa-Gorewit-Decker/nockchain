"""CPU FP8-dequant scheme for the model's **group_0** layers.

The Pearl models split into group_1 (INT7 — *mined*; handled by
`PearlScheme`/`pearl_gemm_cpu`) and group_0 (FP8 block-quantized —
`down_proj` + early-layer qkv; **NOT mined**, B3-scoped-out). A
full forward must still traverse the group_0 FP8 layers to reach
the first mined INT7 GEMM, but vLLM-CPU ships **no FP8
ScaledMM kernel** (and its INT8 CPU kernel is x86-only), so the
stock `CompressedTensorsW8A16Fp8` scheme fails on Apple Silicon.

This scheme reuses vLLM's FP8 *weight loading* (the parent's
`create_weights` registers `weight`/`weight_scale`/`input_scale`
before the kernel-init line that raises), then **dequantizes the
fp8 weights to the model dtype on CPU** and does a plain
`F.linear`. group_0 is not mined ⇒ this only needs to propagate a
numerically-sound hidden state so the *mined* INT7 activation we
capture downstream is real. (R1: out of the faithful-mining
scope — a CPU forward enabler, not a Pearl-protocol claim.)
"""

from __future__ import annotations

import torch
import torch.nn.functional as F
from vllm.model_executor.layers.quantization.compressed_tensors.schemes.compressed_tensors_w8a16_fp8 import (  # noqa: E501
    CompressedTensorsW8A16Fp8,
)


class PearlFp8CpuScheme(CompressedTensorsW8A16Fp8):
    """W8A16-FP8, CPU: parent loads the params; we dequant + matmul."""

    def create_weights(self, layer: torch.nn.Module, *args, **kwargs) -> None:
        try:
            # Parent registers weight/weight_scale/input_scale, then
            # (last stmt) inits the FP8 ScaledMM kernel → raises on
            # CPU "Failed to find a kernel …". Params already exist.
            super().create_weights(layer, *args, **kwargs)
        except ValueError as e:
            if "ScaledMM" not in str(e) and "kernel" not in str(e):
                raise
            self.linear_kernel = None  # CPU: dequant in apply_weights

    @staticmethod
    def _dequant(layer: torch.nn.Module, out_dtype: torch.dtype) -> torch.Tensor:
        # Delegates to the canonical, vLLM-free, unit-tested
        # `fp8_block_dequant` (verbatim vLLM block formula —
        # tests/test_pearl_fp8_cpu.py validates it bit-for-bit
        # against the canonical reference + an independent
        # nested-loop ref + the REAL shipped model's FP8 weights).
        from .pearl_gemm_cpu import fp8_block_dequant

        return fp8_block_dequant(
            layer.weight.data,
            layer.weight_scale.data,
            getattr(layer, "weight_block_size", None),
            out_dtype,
        )

    def process_weights_after_loading(self, layer: torch.nn.Module) -> None:
        if getattr(self, "linear_kernel", None) is not None:
            return super().process_weights_after_loading(layer)
        od = getattr(layer, "orig_dtype", None) or torch.get_default_dtype()
        layer._pearl_w_deq = self._dequant(layer, od)

    def apply_weights(
        self,
        layer: torch.nn.Module,
        x: torch.Tensor,
        bias: torch.Tensor | None = None,
    ) -> torch.Tensor:
        if getattr(self, "linear_kernel", None) is not None:
            return super().apply_weights(layer, x, bias)
        w = layer._pearl_w_deq
        return F.linear(x.to(w.dtype), w, bias)
