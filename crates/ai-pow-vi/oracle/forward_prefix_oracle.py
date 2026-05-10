"""Skeleton: run a full forward_prefix using `reference_ops` and dump
per-layer activations for the Rust verifier to byte-compare against.

Currently this script's per-op primitives (rmsnorm, matmul, ffn, etc.)
are validated by `oracle/synthetic_fixture.py` + `tests/oracle_op_vectors.rs`.
A full attention/deltanet/forward_layer composition in Python would
duplicate a lot of work that is already byte-pinned on the Rust side.

When the user wants to validate a real model end-to-end, the pieces
that need to be added here are:

1. `attention_forward` (compose matmul + rope + softmax with causal mask).
2. `deltanet_forward` (per-token state recurrence with sigmoid α/β).
3. `forward_layer` (norm + sublayer + residual + norm + ffn + residual).
4. `forward_prefix` (embed + sequence of layers + optional final norm).
5. ActivationLog tile-Merkle commitment via `ai-pow::commit::merkle_root`
   semantics (BLAKE3 derive_key over leaf bytes).

Usage (when implemented):
    python oracle/forward_prefix_oracle.py \\
        --vectors oracle/test_vectors/qwen_3_6_27b/ \\
        --layer 8 \\
        --prompt "0,1,2,3,4,..."   # comma-separated tokens

Output:
    activations_layer_0.bin
    activations_layer_1.bin
    ...
    activations_layer_8.bin
"""

from __future__ import annotations

import argparse
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--vectors", required=True, help="weights/manifest dir")
    parser.add_argument("--layer", type=int, required=True, help="target layer K")
    parser.add_argument("--prompt", required=True, help="comma-separated tokens")
    args = parser.parse_args()

    raise NotImplementedError(
        "forward_prefix_oracle.py is a skeleton. The per-op primitives in "
        "reference_ops.py are validated by oracle_op_vectors.rs already; this "
        "script needs additional Python implementations of attention_forward, "
        "deltanet_forward, forward_layer, and forward_prefix to compose them "
        "end-to-end. Until then, the Rust side's pin_vi_proof_round_trip_canonical "
        "covers full-pipeline determinism via cross-architecture replay rather "
        "than cross-implementation replay."
    )


if __name__ == "__main__":
    sys.exit(main())
