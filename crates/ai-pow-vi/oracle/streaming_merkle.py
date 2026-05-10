"""Phase 2.15 — streaming tile-Merkle root builder.

The non-streaming `weights_merkle_root` in `synthetic_qwen_mini.py`
materializes every tile leaf hash before reducing them to a root
(`merkle_root(leaves)`). For a 19 GB weights.bin that's 19e9 / 64 ≈
296M leaves at 32 bytes each = ~9.5 GB of leaf hashes, which doesn't
fit on a laptop.

This module implements the same root via a streaming, stack-of-subtrees
algorithm. Memory is O(log n) — at most ~30 stack entries for a billion
leaves.

Algorithm:
- Tiles arrive 64 bytes at a time. Each completed tile's `tile_hash` is
  wrapped in `merkle_leaf_hash` and pushed onto a stack as a level-0
  entry.
- Whenever the top two stack entries have the same level k, pop both
  and push a level-(k+1) entry computed via `merkle_node_hash(left,
  right)`.
- After the last tile, pad to the next power of two with `sentinel_leaf`
  entries (themselves at level 0; do *not* wrap in `merkle_leaf_hash`,
  matching the existing `merkle_root` reducer).
- The final stack has exactly one entry — the root.

Verified byte-equal to `synthetic_qwen_mini.weights_merkle_root` for a
range of input sizes; see `oracle/tests/test_streaming_merkle.py`.
"""

from __future__ import annotations

import struct
from typing import Optional

import blake3

WEIGHT_TILE_BYTES = 64
CTX_WEIGHT_TILE = "ai-pow-vi v1 weight-tile"
CTX_LEAF = "ai-pow v1 merkle-leaf"
CTX_NODE = "ai-pow v1 merkle-node"
CTX_SENTINEL = "ai-pow v1 merkle-sentinel"


def tile_hash(chunk: bytes) -> bytes:
    """Hash a single tile (must be exactly `WEIGHT_TILE_BYTES`).

    Mirror of `crate::comm_w::tile_hash`."""
    if len(chunk) != WEIGHT_TILE_BYTES:
        raise ValueError(
            f"tile_hash expects exactly {WEIGHT_TILE_BYTES} bytes, got {len(chunk)}"
        )
    h = blake3.blake3(derive_key_context=CTX_WEIGHT_TILE)
    h.update(struct.pack("<Q", len(chunk)))
    h.update(chunk)
    return h.digest(length=32)


def merkle_leaf_hash(leaf: bytes) -> bytes:
    h = blake3.blake3(derive_key_context=CTX_LEAF)
    h.update(leaf)
    return h.digest(length=32)


def merkle_node_hash(left: bytes, right: bytes) -> bytes:
    h = blake3.blake3(derive_key_context=CTX_NODE)
    h.update(left)
    h.update(right)
    return h.digest(length=32)


def sentinel_leaf() -> bytes:
    h = blake3.blake3(derive_key_context=CTX_SENTINEL)
    return h.digest(length=32)


class StreamingMerkle:
    """Stack-of-subtrees streaming Merkle reducer.

    Usage:
        m = StreamingMerkle()
        for chunk in chunks:
            m.update(chunk)
        root = m.finalize()
    """

    def __init__(self) -> None:
        # Stack of (level, 32-byte hash). Top of stack is the most
        # recently appended subtree. Invariant: stack levels are
        # strictly decreasing top-to-bottom (largest at bottom).
        self.stack: list[tuple[int, bytes]] = []
        # Buffer for tile bytes that haven't completed a 64-byte tile.
        self._tile_buf = bytearray()
        # Number of leaves added so far (excluding sentinel padding).
        self.n_leaves = 0
        self._finalized = False

    def update(self, data: bytes) -> None:
        """Append `data` to the running tile buffer. Whenever a 64-byte
        tile completes, hash it and push onto the stack."""
        if self._finalized:
            raise RuntimeError("StreamingMerkle.update after finalize")
        self._tile_buf.extend(data)
        # Process any complete tiles.
        while len(self._tile_buf) >= WEIGHT_TILE_BYTES:
            tile = bytes(self._tile_buf[:WEIGHT_TILE_BYTES])
            del self._tile_buf[:WEIGHT_TILE_BYTES]
            self._add_data_leaf(tile_hash(tile))

    def finalize(self) -> bytes:
        """Flush any partial tail tile (zero-padded), pad with sentinel
        leaves to the next power of two, and reduce to the root."""
        if self._finalized:
            raise RuntimeError("StreamingMerkle.finalize twice")
        # Flush partial tail with zero padding.
        if self._tile_buf:
            tail = bytes(self._tile_buf) + b"\x00" * (
                WEIGHT_TILE_BYTES - len(self._tile_buf)
            )
            self._add_data_leaf(tile_hash(tail))
            self._tile_buf.clear()
        # Empty-input case: synthesize a single all-zero tile leaf to
        # match the existing `weights_merkle_root` behavior.
        if self.n_leaves == 0:
            self._add_data_leaf(tile_hash(b"\x00" * WEIGHT_TILE_BYTES))
        # Pad with sentinel leaves to the next power of two.
        target = 1
        while target < self.n_leaves:
            target *= 2
        sent = sentinel_leaf()
        while self.n_leaves < target:
            self._add_sentinel_leaf(sent)
        # The tree is now full — there must be exactly one node on the
        # stack, at log2(target) level.
        if len(self.stack) != 1:
            raise RuntimeError(
                f"unexpected stack after finalize: {[lv for lv, _ in self.stack]}"
            )
        self._finalized = True
        return self.stack[0][1]

    # --- internal helpers ----

    def _add_data_leaf(self, leaf_h: bytes) -> None:
        # Data leaves are wrapped in merkle_leaf_hash before tree insertion
        # (matches `crate::ai_pow::commit::merkle_root`).
        self._push_level0(merkle_leaf_hash(leaf_h))
        self.n_leaves += 1

    def _add_sentinel_leaf(self, sent: bytes) -> None:
        # Sentinel leaves are NOT wrapped in merkle_leaf_hash — they're
        # already pre-hashed leaf-level constants.
        self._push_level0(sent)
        self.n_leaves += 1

    def _push_level0(self, h: bytes) -> None:
        self.stack.append((0, h))
        # Merge while the top two entries have the same level.
        while len(self.stack) >= 2 and self.stack[-1][0] == self.stack[-2][0]:
            (lv, right) = self.stack.pop()
            (_, left) = self.stack.pop()
            self.stack.append((lv + 1, merkle_node_hash(left, right)))


def streaming_root(buf: bytes) -> bytes:
    """Convenience wrapper: hash a complete byte buffer in streaming
    fashion. Useful for tests; production callers feed `update(chunk)`
    incrementally so the buffer is never fully materialized."""
    m = StreamingMerkle()
    m.update(buf)
    return m.finalize()
