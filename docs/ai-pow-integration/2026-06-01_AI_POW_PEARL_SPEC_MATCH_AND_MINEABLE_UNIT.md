# AI-PoW vs Pearl: Proof / PoW Spec Match and Mineable Unit

Date: 2026-06-01
Status: Current-state audit and fix plan

## Executive Summary

The current `ai-pow` implementation is close to Pearl at the level of the
core mineable primitive:

- BLAKE3 commitment chain shape.
- Row-major `A` and column-major `B` matrix commitments.
- Low-rank noise generation.
- Noised tiled matmul accumulator.
- 16-word jackpot/tile-state update.
- Little-endian 256-bit target comparison.
- Shape-aware target weight `r * tile_m * tile_n`.

The important caveat is that "close to Pearl" is only true after accounting
for deliberate Nockchain extensions:

- Nockchain carries an explicit NCMN nonce and now binds it before `kappa`,
  matrix commitments, noise, noised matrices, and matmul-derived tile states.
- Nockchain uses a derived `pow_key = derive_key("pow-key", s_A || nonce)` in
  the jackpot hash public-input slot that Pearl calls `COMMITMENT_HASH`.
- The canonical Nockchain block artifact is the recursive ZK certificate noun,
  not Pearl's plain block-opening proof and not a raw Layer-0 STARK blob.

The biggest remaining semantic difference is the mineable unit for multi-tile
matmuls. Pearl's paper describes computing the full tiled noised matmul and
checking the block-opening condition on computed full-rank tiles. Current
Nockchain production admission derives exactly one eligible jackpot tile for a
nonce-bound attempt. Today this does not admit multi-tile canonical block
certificates: the miner preflight rejects multi-tile parameters because the
recursive certificate proves one selected tile, not a full-matrix aggregate.

Therefore:

- For the current single-tile canonical smoke profile, the unit of mineable
  work is effectively the same kind of Pearl tile execution, with Nockchain
  nonce binding added.
- For real multi-tile AI workloads, the current canonical proof path is not yet
  Pearl-equivalent because it is fail-closed. Before enabling multi-tile
  consensus, we must explicitly choose and implement the lottery semantics:
  Pearl-style "each computed eligible tile is a ticket" or Nockchain-style
  "one verifier-selected tile per nonce-bound full matmul" with target/work
  accounting adjusted accordingly.

## Pearl Baseline

The Pearl whitepaper's relevant PoW pipeline is:

1. A mining configuration `mu` fixes matrix dimensions, noise rank `r`, tile
   shape, and difficulty `b`.
2. The blockchain state `sigma` and mining configuration feed a job key:
   `kappa = BLAKE3(sigma || mu)`.
3. Matrix commitments are keyed by `kappa`:
   - `H_A = BLAKE3(Flatten(A), key=kappa)`.
   - `H_B = BLAKE3(Flatten(B^T), key=kappa)`, so `B` is committed in
     column-major order for compact column openings.
4. Noise seeds are chained from commitments:
   - `s_B = BLAKE3(kappa || H_B)`.
   - `s_A = BLAKE3(s_B || H_A)`.
5. Low-rank noise matrices `E = E_L * E_R` and `F = F_L * F_R` are generated
   from `s_A` and `s_B`.
6. The miner computes `(A + E) * (B + F)` with a tiled matmul algorithm.
   Each output tile carries a compact 16-word state `M`.
7. A tile wins when the BLAKE3 hash of that tile state is below:
   `2^(256 - b) * r * tile_m * tile_n`, interpreted as a little-endian
   `uint256`.
8. A block-opening proof lets the verifier reconstruct the opened tile from
   matrix openings and commitments. Pearl then compresses this proof with a
   recursive ZK proof.

Pearl's stated unit is therefore tied to the actual tiled noised matmul work:
the ticket is not "hash a nonce"; it is "prove a tile state produced by the
prescribed noised matmul under commitments derived from the chain state."

## Current Nockchain Pipeline

The current Rust pipeline is:

1. `params_tag(params)` binds the `MatmulParams`.
2. `block_state(block_commitment, nonce)` binds the candidate block commitment
   and the exact NCMN nonce bytes.
3. `commitment_key(block_state(...), params_tag)` derives `kappa`.
4. `BlockContext::build(block_commitment, nonce, A, B, params)` computes:
   - legacy diagnostic row/column roots `h_a` / `h_b`;
   - canonical chunk commitments `h_a_chunk` / `h_b_chunk`;
   - `s_A` / `s_B` from the chunk commitments;
   - low-rank noise;
   - noised matrices;
   - all per-tile `M` states for that exact nonce-bound attempt.
5. `mine_inner` derives `pow_key_for_nonce(s_A, nonce)`, hashes tile states
   with that key, derives exactly one eligible `found_idx`, and checks only that
   tile against the target.
6. The ZK statement precheck re-derives `kappa`, `s_A`, `s_B`, `pow_key`,
   `HASH_A`, `HASH_B`, trace height, target satisfaction, and the
   verifier-derived `found_idx` before accepting the recursive certificate
   metadata.
7. The canonical artifact intended for Hoon/block persistence is:
   `[%ai-pow nonce=ai-ncmn cert=ai-pow-certificate]`, with commitments
   `[h-a-chunk h-b-chunk]` only.

This is intentionally not a reusable mining cache. Changing the nonce changes
`kappa`, commitments, noise, noised matrices, tile states, and final jackpot
key. Raw matrix bytes and shape validation can be reused; nonce-independent
noised matrices or tile states cannot.

## What Matches Pearl Closely

### Commitment Chain

Pearl's Algorithm 2 shape is preserved:

```text
kappa = BLAKE3(attempt_state || params_tag)
H_A   = BLAKE3(pad(A_row_major), key=kappa)
H_B   = BLAKE3(pad(B_col_major), key=kappa)
s_B   = BLAKE3(kappa || H_B)
s_A   = BLAKE3(s_B || H_A)
```

The naming differs because Nockchain distinguishes:

- `h_a_chunk` / `h_b_chunk`: production proof-bound chunk commitments.
- `h_a` / `h_b`: legacy plain `MatmulProof` row/column opening roots.

Only `h_a_chunk` / `h_b_chunk` are canonical production commitments.

### Matrix Orientation

Nockchain follows Pearl's useful orientation:

- `A` is row-major.
- `B` is treated column-major, equivalent to Pearl flattening `B^T`.

That preserves compact opening semantics: an opened tile needs rows of `A` and
columns of `B`.

### Noise and Tile State

The low-rank noise and 16-word tile/jackpot state are Pearl-shaped. Existing
fixture tests compare the byte-level primitives against the vendored Pearl
reference. The ZK layer then proves the corresponding composite AIR execution.

### Target Formula

Nockchain's local `difficulty_target(params)` uses:

```text
target = 2^(256 - b) * r * tile^2
```

with little-endian `uint256` comparison, matching Pearl for square tiles.

When consensus supplies an arbitrary chain target, `ai-pow-miner` passes that
explicit 32-byte target instead of relying on `difficulty_bits`.

## Deliberate Differences

### Explicit NCMN Nonce

Pearl removes the Bitcoin-style nonce from the block header and carries a
variable-size proof/certificate. Nockchain still carries an explicit NCMN nonce
inside the AI-PoW command.

This is safe only because the nonce is part of the attempt state before
`kappa`. If the nonce only keyed the final BLAKE3 jackpot hash, one expensive
matmul could be reused for many cheap hash trials. That was the critical bug
class fixed in the current branch.

### Jackpot Key

Pearl's Algorithm 4 uses `BLAKE3(M, key=s_A)` in the tile check.

Nockchain uses:

```text
pow_key = derive_key("pow-key", s_A || nonce)
hash    = BLAKE3(M, key=pow_key)
```

Since `s_A` is already nonce-bound, this is extra domain separation and
statement binding, not the sole nonce binding. The hash primitive and target
comparison are still Pearl-shaped. This difference means Nockchain jackpot
digests are not byte-identical to Pearl unless the key argument is normalized.

### One Verifier-Derived Tile

Pearl's text describes checking the block-opening condition on computed
full-rank tiles during the tiled matmul. In the straightforward reading, a
multi-tile matmul creates many tile-level tickets, each weighted by
`r * tile_m * tile_n`.

Nockchain currently derives one eligible jackpot tile:

```text
found_idx = attempt_tile_index(block_state, params_tag, s_A, num_tiles)
```

and rejects a proof whose submitted `found_idx` differs.

This removes miner-selected tile grinding. It also changes multi-tile
economics unless the target/work accounting is adjusted. A full multi-tile
matmul with only one admissible ticket is not the same lottery as Pearl's
"all eligible computed tiles can win" model.

Current consensus-facing code avoids silently choosing wrong multi-tile
economics by failing closed: multi-tile canonical recursive submissions are
rejected by preflight until the recursive proof can bind a full-matrix
aggregate.

### Proof System and Artifact

Pearl's paper describes a Plonky2 recursive proof stack and reports a final
proof below roughly 60 KB.

Nockchain's current `ai-pow-zk` stack is different:

- Layer-0 is a Plonky3-style composite STARK over Goldilocks.
- Recursive verification uses the local `Plonky3-recursion` substrate with
  Tip5 transcript machinery.
- The canonical artifact is a structured Hoon-compatible recursive certificate
  noun, not a single opaque byte atom and not the legacy plain `MatmulProof`.

This affects proof size, verification code, wire shape, and audit surface, but
it does not by itself change the underlying mineable matmul computation.

### Parameter Shape

Pearl supports a richer tile-shape/configuration language. Current
`MatmulParams` is narrower:

- square tiles only;
- power-of-two rank constraints;
- explicit DoS caps;
- production envelope checks split from fast test profiles.

The core square-tile formula matches Pearl, but the supported parameter space
is not identical.

## Is the Unit of Mineable Work the Same?

### Single-Tile Canonical Path

For the currently admissible single-tile recursive path, yes in the operational
sense that matters for soundness:

- one nonce-bound attempt builds fresh commitments, noise, noised matrices, and
  tile state;
- the proof certifies the tile computation;
- the jackpot hash is checked against a Pearl-shaped target.

The unit is not a cheap nonce hash. It is the nonce-bound noised matmul work for
that tile.

### Multi-Tile Real AI Workloads

Not yet.

Pearl's model gives a large matmul many tile-level opportunities proportional
to the useful work performed. Nockchain currently has the machinery to compute
many tile states in the plain path, but the production recursive certificate
only proves one selected tile and the miner preflight rejects multi-tile
canonical submission.

This is the most important remaining spec decision:

1. **Pearl-style tickets:** every computed eligible tile may be a ticket.
   The proof/certificate must bind the full matmul or a verifier-sampled
   aggregate so a miner cannot skip most tiles and only prove a cherry-picked
   winner.
2. **Nockchain one-tile ticket:** each nonce-bound full matmul yields one
   verifier-selected tile ticket. This minimizes tile grinding but changes the
   target economics for multi-tile matrices. The target must account for the
   fact that only one tile is checked despite all tiles being computed.

Until this is resolved, multi-tile production AI-PoW should remain fail-closed.

## Current Security Posture

The current branch is sounder than the earlier pre-fix design because it closes
the cache-friendly nonce-grinding bug:

- A nonce change forces fresh `kappa`.
- Fresh `kappa` changes matrix commitments.
- Fresh commitments change `s_A` / `s_B`.
- Fresh seeds change low-rank noise.
- Fresh noise changes noised matrices and tile states.
- The recursive statement precheck rejects metadata inconsistent with those
  verifier-derived values.

Remaining caveats:

- Hoon consensus still rejects `%ai-pow` until the verifier jet/path is wired.
- Multi-tile canonical recursive proof is not enabled.
- The current recursive proof stack is not Pearl's Plonky2 three-layer stack,
  so proof size and verification assumptions differ from Pearl's paper.
- Plain `MatmulProof` remains a diagnostic artifact and still carries
  row/column opening roots; it must not be treated as a production block proof.

## Action Plan

1. Keep `%ai-pow` fail-closed in Hoon until recursive certificate verification
   is wired end-to-end.
2. Write the multi-tile lottery rule explicitly before enabling real AI
   workloads:
   - Pearl-style per-tile tickets, or
   - one selected tile per nonce-bound full matmul with adjusted target/work
     accounting.
3. If choosing Pearl-style tickets, design a recursive certificate that proves
   enough of the full matmul aggregate to prevent "compute only the winning
   tile" shortcuts.
4. If choosing one selected tile, update the difficulty/work accounting docs
   and ASERT integration so large matrices are not under- or over-rewarded.
5. Keep the canonical commitment surface as exactly `[h-a-chunk h-b-chunk]`.
   Do not reintroduce row/column opening roots as commitment parameters.
6. Rename or document the `COMMITMENT_HASH` public-input slot as the
   nonce-derived jackpot key to avoid future Pearl/Nockchain confusion.
7. Add behavior tests for the final Hoon/Rust verifier path:
   - honest single-tile recursive certificate accepted after activation;
   - nonce tamper rejected;
   - `h_a_chunk` / `h_b_chunk` tamper rejected;
   - wrong `found_idx` rejected;
   - multi-tile certificate rejected until the chosen multi-tile rule is
     implemented.

