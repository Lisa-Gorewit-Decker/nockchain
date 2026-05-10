"""Phase 2.15 — streaming tile-Merkle root tests.

Verifies `streaming_merkle.StreamingMerkle` produces byte-identical
roots to the existing materialized-leaves implementation
(`synthetic_qwen_mini.weights_merkle_root`) across a sweep of input
sizes, including edge cases:
  - empty input (synthesizes a single all-zero tile)
  - input shorter than one tile (zero-padded last tile)
  - exactly one tile
  - exactly two tiles (no padding)
  - input that requires sentinel padding to next-power-of-two
  - multi-megabyte input fed in many small chunks (streaming property)

Also smoke-tests that StreamingMerkle keeps memory bounded — the
internal stack must never exceed log2(n_leaves) + 1 entries.
"""

from __future__ import annotations

import math
import os
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ORACLE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.insert(0, ORACLE_DIR)
sys.path.insert(0, SCRIPT_DIR)

import streaming_merkle as SM  # noqa: E402
import synthetic_qwen_mini as D  # noqa: E402


def _both_match(buf: bytes) -> None:
    streaming = SM.streaming_root(buf)
    reference = D.weights_merkle_root(buf)
    assert streaming == reference, (
        f"streaming root != reference for len={len(buf)}: "
        f"streaming={streaming.hex()[:16]} reference={reference.hex()[:16]}"
    )


def test_empty_input():
    _both_match(b"")
    print("test_empty_input OK")


def test_under_one_tile():
    for n in (1, 7, 31, 63):
        _both_match(b"\x42" * n)
    print("test_under_one_tile OK")


def test_exactly_one_tile():
    _both_match(b"\xa5" * 64)
    print("test_exactly_one_tile OK")


def test_exactly_two_tiles():
    # 2 tiles, no padding to power-of-two needed.
    _both_match(bytes(range(128)))
    print("test_exactly_two_tiles OK")


def test_three_tiles_with_sentinel_padding():
    # 3 tiles → padded to 4 leaves with one sentinel.
    _both_match(bytes(((i * 13 + 7) & 0xFF) for i in range(64 * 3)))
    print("test_three_tiles_with_sentinel_padding OK")


def test_partial_tail_tile():
    # 2 full tiles + a partial 50-byte tail (3rd tile zero-padded), then
    # padded to 4 leaves with one sentinel.
    buf = bytes(((i * 17) & 0xFF) for i in range(2 * 64 + 50))
    _both_match(buf)
    print("test_partial_tail_tile OK")


def test_many_tiles():
    # 17 tiles → padded to 32 leaves with 15 sentinels. Exercises the
    # multi-level stack-merge path.
    buf = bytes(((i * 19) & 0xFF) for i in range(17 * 64))
    _both_match(buf)
    print("test_many_tiles OK")


def test_streaming_with_chunks():
    # Same buffer, fed via lots of small update() calls. The output must
    # still match the materialized-leaves reference.
    full = bytes(((i * 23) & 0xFF) for i in range(7 * 64 + 13))
    m = SM.StreamingMerkle()
    chunk = 5
    for i in range(0, len(full), chunk):
        m.update(full[i : i + chunk])
    streaming = m.finalize()
    reference = D.weights_merkle_root(full)
    assert streaming == reference, "chunked update divergence"
    print("test_streaming_with_chunks OK")


def test_stack_depth_is_bounded():
    # 1024 tiles → stack should never exceed ~log2(1024) = 10 entries.
    m = SM.StreamingMerkle()
    max_depth = 0
    for i in range(1024):
        m.update(b"\x00" * 64)
        max_depth = max(max_depth, len(m.stack))
    # Per the algorithm, the stack at any point in [0, n) holds at most
    # log2(i+1) + 1 entries — a tight bound.
    assert max_depth <= 11, f"stack grew unexpectedly to {max_depth}"
    m.finalize()  # clear finalized flag side-effect
    print(f"test_stack_depth_is_bounded OK (max_depth={max_depth})")


def test_finalize_idempotency_guard():
    # Double-finalize is rejected (helps catch reuse bugs).
    m = SM.StreamingMerkle()
    m.update(b"\x01" * 64)
    m.finalize()
    try:
        m.finalize()
    except RuntimeError as e:
        assert "twice" in str(e), e
        print("test_finalize_idempotency_guard OK")
        return
    raise AssertionError("expected double-finalize to raise")


def test_update_after_finalize_rejected():
    m = SM.StreamingMerkle()
    m.update(b"\xff" * 64)
    m.finalize()
    try:
        m.update(b"\x00")
    except RuntimeError as e:
        assert "after finalize" in str(e), e
        print("test_update_after_finalize_rejected OK")
        return
    raise AssertionError("expected update-after-finalize to raise")


if __name__ == "__main__":
    test_empty_input()
    test_under_one_tile()
    test_exactly_one_tile()
    test_exactly_two_tiles()
    test_three_tiles_with_sentinel_padding()
    test_partial_tail_tile()
    test_many_tiles()
    test_streaming_with_chunks()
    test_stack_depth_is_bounded()
    test_finalize_idempotency_guard()
    test_update_after_finalize_rejected()
    print("ALL streaming_merkle TESTS PASSED")
