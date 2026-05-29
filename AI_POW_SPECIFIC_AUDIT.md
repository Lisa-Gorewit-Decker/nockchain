# AI-PoW Specific Audit Notes

Scope: AI-PoW only. This excludes generic private gRPC/admin-socket concerns unless they directly affect AI-PoW correctness.

Branch reviewed: `claude/ai-pow-integration-squash`

## Summary

The standalone Rust `ai-pow` proof code is much stronger than the earlier branch. It has parameter caps, proof-shape decoding, nonce format checks, adversarial tests, Pearl compatibility tests, and production-envelope checks.

The Nockchain integration is still not consensus-ready. The critical gaps are not just "stubs". The current AI miner proves work for a locally defined job. Consensus does not yet define the same job, target conversion, puzzle identity, model/matrix selection, or proof envelope.

AI blocks are currently fail-closed because Hoon rejects all `%ai-pow` submissions. That protects consensus today. If AI acceptance were enabled without fixing the issues below, the chain would either reject valid miner output or accept work under rules that are not actually consensus-defined.

## AI-PoW Specific Issues

### 1. AI blocks are rejected by consensus

File: `hoon/apps/dumbnet/inner.hoon`

The `%ai-pow` branch in `do-pow` is a reject-all branch. Even after activation, no AI block can land.

Impact: AI-PoW mining is not live. This is safe from a consensus-soundness view, but means the branch is not a functioning AI-PoW consensus integration.

### 2. The AI miner does not submit the AI proof

Files:

- `crates/ai-pow-miner/src/run.rs`
- `crates/ai-pow-miner/src/wire.rs`
- `hoon/apps/dumbnet/lib/types.hoon`

The miner finds a Rust `MatmulProof`, but the node submission sends only:

```text
[%mined nonce found-idx]
```

The Hoon `pow-variant` still defines `%ai-pow` as a single placeholder atom. There is no consensus envelope carrying the nonce, params, proof bytes, ZK artifact, matrix commitments, or found tile.

Impact: even if the Hoon verifier were enabled, it would not have enough data to verify the proof.

### 3. Fatal for consensus usefulness: the AI job binding is local, not consensus-defined

Files:

- `crates/ai-pow-miner/src/run.rs`
- `crates/nockchain-mining-common/src/candidate.rs`
- `hoon/apps/dumbnet/inner.hoon`

The Hoon kernel emits an AI mining candidate shaped like:

```text
[%mine-ai %3 commit ai-target pow-len]
```

That gives the miner:

- the block commitment
- the AI ASERT target
- the proof version
- the PoW length

The Rust AI miner then derives its own job inputs:

```rust
nck = BLAKE3(jam(candidate.block_header))
target = BLAKE3(jam(candidate.target))
```

This is explicitly documented as local miner behavior, not the final chain rule.

#### Why this is fatal

Proof of work only has consensus meaning if every verifier can rebuild the exact same challenge from chain data.

For AI-PoW, that means consensus must independently derive:

1. the Nockchain block anchor carried in the NCMN nonce
2. the 256-bit difficulty target
3. the AI puzzle id
4. the accepted params
5. the model/matrix data or matrix commitments
6. the exact proof envelope

Today, the miner invents part of that derivation locally. Consensus does not yet define it.

#### Target binding problem

The AI prover expects a 32-byte little-endian target and checks:

```text
tile_hash <= target
```

But the mining candidate target is a Hoon bignum noun. The current miner does not decode that bignum numerically. It hashes the jammed noun:

```rust
target = BLAKE3(jam(candidate.target))
```

That destroys the numeric meaning of the target.

Consequences:

- a harder chain target can hash to an easier AI target
- an easier chain target can hash to a harder AI target
- ASERT no longer controls AI mining difficulty
- verifier and miner can disagree even if both are honest
- if consensus copied this rule, difficulty would be detached from the chain target

Correct behavior should decode the Hoon `%bn` target into the exact 256-bit little-endian bytes used by `ai_pow::tile_hash::hash_le_target`.

#### Block anchor problem

NCMN nonces carry a Nockchain block anchor. That is good design. But the current anchor is:

```rust
BLAKE3(jam(candidate.block_header))
```

That may be deterministic, but it is not yet a consensus rule. The chain must specify exactly what bytes are anchored:

- the existing block commitment directly, or
- a domain-separated hash of a canonical block header encoding, or
- another explicit value in the AI proof envelope

The verifier must then check that the NCMN nonce contains that exact value.

#### What a correct consensus rule needs

A consensus-ready AI-PoW rule should define, in one place:

```text
candidate_block_anchor = canonical_chain_rule(block)
ai_target_bytes        = canonical_bn_to_le_32(ai_target)
puzzle_id              = canonical_ai_puzzle_id(height, epoch, layer, params)
params                 = canonical_params_for_height_or_epoch(...)
matrix_commitments     = canonical_model_or_weight_commitments(...)
```

Then the verifier should check:

```text
nonce.nck_commitment == candidate_block_anchor
proof target          == ai_target_bytes
proof params          == canonical params
proof matrix roots    == canonical matrix commitments
found tile            == tile used by both plain and ZK proof
```

Until that exists, the AI miner is proving a local job, not a chain job.

### 4. The full artifact verifier does not enforce NCMN anchoring

File: `crates/ai-pow/src/zk_bridge.rs`

The high-level artifact verifier calls `verify_prod_at_target`, not `verify_ncmn_at_target`.

That means it verifies the proof against a block commitment argument, but it does not enforce that the nonce itself carries the expected Nockchain block anchor.

Impact: if this verifier were wired into consensus as-is, the NCMN replay protection would not be enforced at the top-level artifact boundary.

### 5. Plain proof tile and ZK tile are not cross-checked

File: `crates/ai-pow/src/zk_bridge.rs`

The high-level artifact verifier verifies:

- the plain proof, using the found tile inside `plain_proof.found`
- the ZK proof, using caller-supplied `found_idx`

It does not check:

```text
found_idx == tile_index(plain_proof.found.i, plain_proof.found.j)
```

Impact: the two halves of the artifact can refer to different tiles. That is fragile and should be rejected explicitly.

### 6. The ZK jackpot is not yet the real tile state

File: `crates/ai-pow/src/zk_bridge.rs`

The ZK bridge comments say `JACKPOT_MSG` is all zero and not the real `TileState M`.

The plain proof checks a real tile. The ZK side binds keys, commitments, and a jackpot hash, but the jackpot message is not yet the real GEMM tile state.

Impact: the ZK artifact should not be treated as a complete proof that the jackpot hash was computed from the real GEMM tile output. It is not yet full GEMM-work fidelity.

### 7. Params are consensus-critical, not miner config

Files:

- `crates/ai-pow/src/params.rs`
- `crates/ai-pow/src/tile_hash.rs`
- `crates/ai-pow-miner/src/bin/ai_pow_mine.rs`

AI difficulty depends on params, especially:

```text
noise_rank * tile^2
```

The miner currently takes puzzle params and matrices from local CLI/config. If consensus ever lets miners influence params, they can influence difficulty and proof cost.

Impact: consensus must pin a narrow profile or deterministic schedule. Do not accept arbitrary envelope-valid params from miners.

### 8. Production-envelope params can still be expensive

Files:

- `crates/ai-pow/src/params.rs`
- `crates/ai-pow/src/proof.rs`

The production envelope is much safer than before, but it still permits large proof shapes if params are too broad. The proof decoder is shape-aware, but the chain still needs hard profile caps before accepting network data.

Impact: if params become chain/user selectable, malformed or oversized proof submissions become a DoS target.

## What Looks Good

The core plain proof path has meaningful hardening:

- tile indices are range-checked
- spot indices are Fiat-Shamir derived
- strip lengths and path depths are checked against params before allocation
- spot-check count is capped
- params have a production envelope
- NCMN nonce format exists and checks malformed headers
- adversarial proof mutation tests pass
- Pearl compatibility fixtures pass

These are good pieces. The issue is that they are not yet assembled into a complete consensus rule.

## Required Fixes Before AI-PoW Can Be Consensus-Useful

1. Define canonical AI mining job derivation in consensus.
2. Decode AI ASERT target numerically into 32 little-endian bytes.
3. Define and enforce the canonical NCMN block anchor.
4. Define chain-pinned puzzle id, params, and matrix/model commitments.
5. Replace the Hoon placeholder `%ai-pow` payload with a real proof envelope.
6. Submit the full proof/artifact from `ai-pow-miner`.
7. Enforce NCMN anchoring in the high-level artifact verifier.
8. Cross-check plain-proof found tile against ZK `found_idx`.
9. Replace the zero jackpot message with the real tile-state `M`.
10. Add hard consensus byte/time caps for AI proof decoding and verification.

## Bottom Line

The AI-PoW branch is not merely unfinished. The current miner and proof code do not yet define the same consensus challenge.

The most important issue is the local target and block-binding derivation in `ai-pow-miner`. Until that is replaced with a chain-defined derivation, AI-PoW cannot be consensus-useful even if the reject-all Hoon stub is removed.
