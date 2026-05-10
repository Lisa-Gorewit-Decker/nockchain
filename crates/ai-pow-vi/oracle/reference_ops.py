"""Numpy reference implementation of ai-pow-vi integer ops.

Every function here MUST be byte-identical to its Rust counterpart for
all inputs it accepts. The Rust impls live in:

  src/quant.rs        round_half_to_even_div_pow2, rescale_and_requantize, saturate_i8
  src/matmul_int8.rs  dot_int8, matmul_int8, requantize_vec
  src/rmsnorm.rs      isqrt_floor, rmsnorm
  src/layernorm.rs    layernorm
  src/softmax.rs      softmax_int
  src/ffn.rs          elementwise_mul_i8, ffn_forward

We use arbitrary-precision Python ints throughout to make every
truncation, wrap, and shift explicit. numpy is used only for I/O
(reading / writing the fixture binaries).
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Sequence

import numpy as np

# -----------------------------------------------------------------------------
# Quantization primitives (mirror src/quant.rs).
# -----------------------------------------------------------------------------

SCALE_DENOM_LOG2 = 15
I8_MIN = -128
I8_MAX = 127
I32_MIN = -(1 << 31)
I32_MAX = (1 << 31) - 1
I64_MIN = -(1 << 63)
I64_MAX = (1 << 63) - 1


@dataclass(frozen=True)
class Scale:
    """Symmetric INT8 quantization scale: effective value is num / 2^15."""

    num: int

    def __post_init__(self) -> None:
        if self.num <= 0:
            raise ValueError("scale must be > 0")


def saturate_i8(value: int) -> int:
    """Clamp a signed integer to [-128, 127]."""
    return max(I8_MIN, min(I8_MAX, value))


def round_half_to_even_div_pow2(value: int, shift: int) -> int:
    """Round value / 2^shift to the nearest integer, ties-to-even.

    Matches the Rust impl precisely: arithmetic shift floors toward
    negative infinity, then a half-up correction with banker's tie-break.
    """
    assert 0 < shift < 63, f"shift must be in (0, 63); got {shift}"
    half = 1 << (shift - 1)
    trunc = value >> shift  # Python's >> is arithmetic on signed; same as Rust
    frac = value - (trunc << shift)
    assert 0 <= frac < (1 << shift), f"frac out of range: {frac}"
    if frac < half:
        return trunc
    if frac > half:
        return trunc + 1
    if (trunc & 1) == 0:
        return trunc
    return trunc + 1


def rescale_and_requantize(acc: int, scale: Scale) -> int:
    """i32 accumulator → i8 via scale.num/2^15, banker's rounding, saturate."""
    if not (I32_MIN <= acc <= I32_MAX):
        raise ValueError(f"acc {acc} out of i32 range")
    product = acc * scale.num
    if not (I64_MIN <= product <= I64_MAX):
        raise OverflowError("rescale product overflows i64")
    rounded = round_half_to_even_div_pow2(product, SCALE_DENOM_LOG2)
    return saturate_i8(rounded)


# -----------------------------------------------------------------------------
# Matmul (mirror src/matmul_int8.rs).
# -----------------------------------------------------------------------------


def dot_int8(a: Sequence[int], b: Sequence[int]) -> int:
    """k-length INT8 dot product with i32 accumulator semantics.

    For k <= 2^15 the result fits in i32, so wrap is a no-op. We keep
    Python's arbitrary-precision arithmetic and assert the bound, which
    is what the Rust crate enforces via MatmulError::KOutOfRange.
    """
    if len(a) != len(b):
        raise ValueError("dot_int8 length mismatch")
    if len(a) > (1 << 15):
        raise ValueError("k must be <= 2^15")
    acc = 0
    for x, y in zip(a, b):
        acc += int(x) * int(y)
    if not (I32_MIN <= acc <= I32_MAX):
        raise OverflowError("dot_int8 overflows i32")
    return acc


def matmul_int8(a: Sequence[int], b: Sequence[int], m: int, k: int, n: int) -> list[int]:
    """out = A · B with B in column-major layout. Returns flat (m * n) i32."""
    if len(a) != m * k:
        raise ValueError("a length mismatch")
    if len(b) != k * n:
        raise ValueError("b length mismatch")
    out = [0] * (m * n)
    for i in range(m):
        row = a[i * k : (i + 1) * k]
        for j in range(n):
            col = b[j * k : (j + 1) * k]
            out[i * n + j] = dot_int8(row, col)
    return out


def requantize_vec(acc: Sequence[int], scale: Scale) -> list[int]:
    return [rescale_and_requantize(int(v), scale) for v in acc]


def matmul_int8_requant(
    a: Sequence[int],
    b: Sequence[int],
    m: int,
    k: int,
    n: int,
    scale: Scale,
) -> list[int]:
    return requantize_vec(matmul_int8(a, b, m, k, n), scale)


# -----------------------------------------------------------------------------
# RMSNorm / LayerNorm (mirror src/rmsnorm.rs, src/layernorm.rs).
# -----------------------------------------------------------------------------

RMS_FRACT_BITS = 16
DEFAULT_EPS_Q = 1


def isqrt_floor(y: int) -> int:
    """Largest k with k*k <= y; matches Rust's Newton-Raphson result.

    `math.isqrt` returns the exact integer floor sqrt for non-negative
    inputs, which is provably equal to the Rust Newton-Raphson result
    when both converge — and they always do for non-negative inputs.
    """
    assert y >= 0, "isqrt_floor input must be non-negative"
    return math.isqrt(y)


def rmsnorm(input: Sequence[int], gamma: Sequence[int], eps_q: int = DEFAULT_EPS_Q) -> list[int]:
    """RMSNorm matching src/rmsnorm.rs::rmsnorm. Output is i32-clamped."""
    hidden = len(input)
    if hidden == 0:
        raise ValueError("hidden must be > 0")
    if len(gamma) != hidden:
        raise ValueError("gamma length mismatch")

    sumsq = sum(int(x) * int(x) for x in input)
    num = hidden << (2 * RMS_FRACT_BITS)
    den = max(1, sumsq + eps_q)
    inv_rms_fixed = isqrt_floor(num // den)

    half = 1 << (RMS_FRACT_BITS - 1)
    out = []
    for i in range(hidden):
        prod = int(input[i]) * int(gamma[i]) * inv_rms_fixed
        shifted = (prod + half) >> RMS_FRACT_BITS
        out.append(max(I32_MIN, min(I32_MAX, shifted)))
    return out


def layernorm(
    input: Sequence[int],
    gamma: Sequence[int],
    beta: Sequence[int],
    eps_q: int = DEFAULT_EPS_Q,
) -> list[int]:
    """LayerNorm matching src/layernorm.rs::layernorm."""
    hidden = len(input)
    if hidden == 0:
        raise ValueError("hidden must be > 0")
    if len(gamma) != hidden or len(beta) != hidden:
        raise ValueError("gamma/beta length mismatch")

    s = sum(int(x) for x in input)
    # Rust uses signed integer division which truncates toward zero.
    # Python's `//` floors toward negative infinity, so we have to use
    # a manual truncating divide.
    mean = _trunc_div(s, hidden)
    sumsq_dev = sum((int(x) - mean) ** 2 for x in input)
    num = hidden << (2 * RMS_FRACT_BITS)
    den = max(1, sumsq_dev + eps_q)
    inv_std_fixed = isqrt_floor(num // den)

    half = 1 << (RMS_FRACT_BITS - 1)
    out = []
    for i in range(hidden):
        dev = int(input[i]) - mean
        prod = dev * int(gamma[i]) * inv_std_fixed
        shifted = (prod + half) >> RMS_FRACT_BITS
        with_bias = shifted + int(beta[i])
        out.append(max(I32_MIN, min(I32_MAX, with_bias)))
    return out


def _trunc_div(a: int, b: int) -> int:
    """Truncating-toward-zero integer division (Rust's `/` semantics)."""
    q, r = divmod(a, b)
    if r != 0 and (a < 0) != (b < 0):
        q += 1
    return q


# -----------------------------------------------------------------------------
# Softmax (mirror src/softmax.rs).
# -----------------------------------------------------------------------------


@dataclass(frozen=True)
class ExpLut:
    """256-entry i32 LUT, mirrors crate::softmax::ExpLut."""

    table: tuple[int, ...]

    def __post_init__(self) -> None:
        if len(self.table) != 256:
            raise ValueError("ExpLut requires 256 entries")

    @classmethod
    def uniform_test(cls) -> "ExpLut":
        return cls(table=tuple(1 << 16 for _ in range(256)))


def softmax_int(scores_scaled: Sequence[int], lut: ExpLut) -> list[int]:
    """Integer softmax matching crate::softmax::softmax_int output."""
    if len(scores_scaled) == 0:
        raise ValueError("scores must be non-empty")
    m = max(scores_scaled)
    sum_exp = 0
    exp_vals = []
    for s in scores_scaled:
        delta = max(0, min(255, m - int(s)))
        e = int(lut.table[delta])
        exp_vals.append(e)
        sum_exp += e
    if sum_exp <= 0:
        return [0] * len(scores_scaled)
    half = sum_exp // 2
    out = []
    for e in exp_vals:
        scaled = e * 127 + half
        # Rust uses i64 truncating division; for non-negative numerator
        # and denominator, this matches Python's `//`.
        q = scaled // sum_exp if scaled >= 0 else -((-scaled) // sum_exp)
        out.append(max(I8_MIN, min(I8_MAX, q)))
    return out


# -----------------------------------------------------------------------------
# FFN (mirror src/ffn.rs).
# -----------------------------------------------------------------------------


def elementwise_mul_i8(
    a: Sequence[int],
    b: Sequence[int],
    scale: Scale,
) -> list[int]:
    if len(a) != len(b):
        raise ValueError("elementwise length mismatch")
    out = []
    for x, y in zip(a, b):
        prod = int(x) * int(y)
        # Rust's wrapping_mul on i32 inputs of magnitude <=128 cannot wrap
        # (max product = 128*128 = 16384). We just compute it directly.
        if not (I32_MIN <= prod <= I32_MAX):
            raise OverflowError("elementwise i8*i8 overflow (impossible)")
        out.append(rescale_and_requantize(prod, scale))
    return out


def apply_activation_lut(xs: Sequence[int], lut_bytes: Sequence[int]) -> list[int]:
    """Apply a 256-entry INT8 LUT to a sequence of i8 values.

    `lut_bytes` is a sequence of 256 u8s; each one represents an i8
    output via the same wraparound the Rust impl uses (i.e. `byte as i8`).
    """
    if len(lut_bytes) != 256:
        raise ValueError("LUT must have 256 entries")
    table = [_u8_to_i8(b) for b in lut_bytes]
    return [table[(x + 128) & 0xFF] for x in xs]


def _u8_to_i8(b: int) -> int:
    if b >= 128:
        return b - 256
    return b


@dataclass(frozen=True)
class FfnScales:
    gate: Scale
    up: Scale
    mid: Scale
    down: Scale


def ffn_forward(
    input: Sequence[int],
    w_gate: Sequence[int],
    w_up: Sequence[int],
    w_down: Sequence[int],
    activation_lut_bytes: Sequence[int],
    scales: FfnScales,
    m: int,
    hidden: int,
    intermediate: int,
) -> list[int]:
    """SwiGLU FFN forward, matching crate::ffn::ffn_forward."""
    gate_acc = matmul_int8(input, w_gate, m, hidden, intermediate)
    gate_q = requantize_vec(gate_acc, scales.gate)
    gate_q = apply_activation_lut(gate_q, activation_lut_bytes)

    up_acc = matmul_int8(input, w_up, m, hidden, intermediate)
    up_q = requantize_vec(up_acc, scales.up)

    mid_q = elementwise_mul_i8(gate_q, up_q, scales.mid)

    down_acc = matmul_int8(mid_q, w_down, m, intermediate, hidden)
    return requantize_vec(down_acc, scales.down)


# -----------------------------------------------------------------------------
# I/O helpers.
# -----------------------------------------------------------------------------


def write_i8(path: str, xs: Sequence[int]) -> None:
    arr = np.array(xs, dtype=np.int8)
    arr.tofile(path)


def write_i32(path: str, xs: Sequence[int]) -> None:
    arr = np.array(xs, dtype=np.int32)
    arr.tofile(path)


def read_i8(path: str) -> list[int]:
    return np.fromfile(path, dtype=np.int8).tolist()


def read_i32(path: str) -> list[int]:
    return np.fromfile(path, dtype=np.int32).tolist()


# -----------------------------------------------------------------------------
# Canonical LCG (mirrors `canonical_input_i8` in Rust tests).
# -----------------------------------------------------------------------------


def canonical_input_i8(length: int, seed: int) -> list[int]:
    """Same LCG the Rust pin tests use. Stable bytes that exercise
    full i8 dynamic range without depending on rand."""
    s = seed & ((1 << 64) - 1)
    out = []
    for _ in range(length):
        s = (s * 6364136223846793005 + 1442695040888963407) & ((1 << 64) - 1)
        # >>56 picks the top 8 bits, then reinterpret as i8.
        out.append(_u8_to_i8(s >> 56))
    return out
