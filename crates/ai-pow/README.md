# `ai-pow`

A Pearl-style proof-of-useful-work matrix-multiplication puzzle for
Nockchain. The crate implements the `(A + E)(B + F)` tiled matmul puzzle
from the Pearl Whitepaper (cited by name; PDF not in repo)
end-to-end: low-rank noise generation, tile-by-tile mining with an
iterative 512-bit accumulator state, shape-aware difficulty thresholds,
and a replication-style verifier.

This is **Phase 1** of the AI-PoW track. The production path is
`ai-pow` (the mineable matmul unit) plus `ai-pow-zk` (the SNARK side).
An earlier experimental verifiable-inference crate (`ai-pow-vi`) was
removed — it was offline tooling, not on the production path.

## Open lines of work

The **active in-flight residuals** on the ai-pow side. Each row
points to the design / status doc that owns it. Pearl-byte-
equivalence on the *mineable unit* is the byte-equiv anchor for
the SNARK side (`ai-pow-zk`); open ai-pow-side residuals are
primarily Pearl-reference + quant-extraction.

| Open work | Doc (in [`docs/`](docs/)) | Status |
|---|---|---|
| **Pearl divergence inventory** (the primary closure tracker) | [`2026-05-13_PEARL_COMPARISON.md`](docs/2026-05-13_PEARL_COMPARISON.md) | Live |
| **Pearl-spec audit** that drove the v2 → v3 redesign | [`2026-05-13_FLAWS.md`](docs/2026-05-13_FLAWS.md) | Live (historical) |
| **Phase B1.1 Pearl-faithfulness audit** — vendored reference ≡ Pearl `zk-pow` line-for-line + real 16 GB weights byte-process | [`2026-05-18_B1_PEARL_FAITHFULNESS_AUDIT.md`](docs/2026-05-18_B1_PEARL_FAITHFULNESS_AUDIT.md) | COMPLETE on the real shipped model (see Phase-B in roadmap) |
| **B1 — Pearl reference vectors** from Pearl's miner (golden `(κ, s_a, s_b, E/F, tile digest)` to bit-anchor `ai-pow` against Pearl) | (planned; see roadmap Phase B1 in [`../ai-pow-zk/docs/2026-05-17_PRODUCTION_ROADMAP.md`](../ai-pow-zk/docs/2026-05-17_PRODUCTION_ROADMAP.md)) | Open — needs Pearl-side artifacts |
| **B2 — Quant-extraction contract** for the vLLM plugin's INT7/INT8 → Pearl type-0 `[−64,64]` int8 `(A,B,μ)` mapping | same roadmap Phase B2 | Open — needs Pearl-side artifacts |
| **Production roadmap** (where Phase B / D1 / D2 / FP8 are scoped) | [`../ai-pow-zk/docs/2026-05-17_PRODUCTION_ROADMAP.md`](../ai-pow-zk/docs/2026-05-17_PRODUCTION_ROADMAP.md) | Live (lives in ai-pow-zk's docs but covers ai-pow too) |

The downstream SNARK side (`ai-pow-zk`) has its own open lines
of work tracker — see [`../ai-pow-zk/README.md#open-lines-of-work`](../ai-pow-zk/README.md#open-lines-of-work)
for M-S5b / C4 / measurements / etc.

The [`docs/`](docs/) directory has the full categorized index in
[`docs/README.md`](docs/README.md).

## Scope

What `ai-pow` provides:

- **Inputs**: caller-supplied INT8 matrices `A` (m × k row-major) and `B`
  (n × k column-major), each entry in `[-64, 64]` (Pearl §4.1).
- **Mining**: `mine(block_commitment, nonce, a, b, params, opts)` searches
  for a tile whose keyed-hash of the tile state falls below a shape-aware
  difficulty target `2^(256-b) · r · t²` (Pearl §4.5).
- **Verification**: production callers use
  `verify_ncmn_at_target(puzzle_id, candidate_nck_commitment, nonce, params, target, proof)`;
  lower-level callers that already enforce the production envelope and chain
  binding use `verify_at_target(block_commitment, nonce, params, target, proof)`.
  The old `verifier::verify` helper derives its target from
  `params.difficulty_bits` and is not a consensus API.
- **Proof format**: 32-byte tile-state commitment `comm_m`, BLAKE3-keyed
  matrix commitments `H_A` and `H_B`, and per-tile openings (raw strips,
  m-path to `comm_m`, per-row/col paths to `H_A` / `H_B`).
- **Synth helper**: `synth_matrices(seed, params)` deterministically
  generates Pearl-valid `(A, B)` pairs for tests; real miners supply
  their own.

What `ai-pow` deliberately does **not** include:

- Plonky2 / STARKy zkSNARK block-opening proof (Pearl §4.7). This crate
  is the pre-SNARK reference; proof sizes scale with σ × t × k.
- Chain integration, mempool, RPC, block-header format.
- Hoon-side jets or consensus glue. Those live downstream.

## Pearl alignment

Cross-implementation byte-equivalence against the Pearl upstream is
tracked in [`2026-05-13_PEARL_COMPARISON.md`](docs/2026-05-13_PEARL_COMPARISON.md). Every
load-bearing protocol surface has a captured Pearl byte-fixture in
`tests/fixtures/pearl.rs`, exercised by `tests/pearl_compat_fixtures.rs`:

| Section | Topic | Status |
|---|---|---|
| S0 | Protocol constants (`JACKPOT_SIZE`, `LROT_PER_TILE`, label seeds, chunk size) | byte-equivalent |
| S1 | `get_random_hash` PRNG byte stream (`prng::pearl_random_hash`) | byte-equivalent |
| S2 | Permutation pairs via XOR trick (`prng::pearl_permutation_pair`) | byte-equivalent |
| S3 | `generate_uniform_random_matrix` (`prng::fill_uniform_row`) | byte-equivalent |
| S4 | `matvec_sparse_perm` reconstruction | byte-equivalent |
| S5 | Tile loop `jackpot[16]` evolution | byte-equivalent |
| S6 | `compute_jackpot_hash` keyed BLAKE3 | byte-equivalent |
| S7 | Commitment-hash chain `kappa → s_b → s_a` | byte-equivalent |
| S8 | Matrix-commitment chunk-Merkle root (`commit::matrix_commitment`) | root byte-equivalent; per-strip proof format is a per-row Merkle (follow-on) |
| S9 | Shape-aware difficulty target in little-endian | byte-equivalent |

The Pearl ISC license is reproduced verbatim at
[`LICENSE-PEARL`](LICENSE-PEARL); see that file for the precise
list of `ai-pow`-side derived portions.

## Layout

| Path | Purpose |
|---|---|
| `src/lib.rs` | Public re-exports |
| `src/params.rs` | `MatmulParams` and validation (Pearl §4.8 constraints) |
| `src/prng.rs` | Pearl-compatible PRNG building blocks (`pearl_random_hash`, uniform-noise, permutation, A/B synth) |
| `src/matmul.rs` | `BlockNoise`, `Matrices` (`A' = A + E`, `B' = B + F`), `TileState`, `compute_tile`, `compute_tile_from_slices` |
| `src/tile_hash.rs` | `difficulty_target`, `hash_le_target` (little-endian U256 semantics) |
| `src/commit.rs` | Tile-state Merkle (sentinel-padded) and Pearl `matrix_commitment` chunk-Merkle |
| `src/fiat_shamir.rs` | Pearl §4.3 commitment-hash chain over nonce-bound attempt state + final pow-key derivation |
| `src/prover.rs` | nonce-bound `BlockContext`, `mine`, `mine_block` |
| `src/verifier.rs` | `verify` |
| `src/proof.rs` | `MatmulProof`, `TileOpening`, encode / decode |
| `src/synth.rs` | Deterministic `(A, B)` test synthesis |
| `tests/` | End-to-end, adversarial, soundness simulation, LLM-shape, Pearl-compat fixtures |
| `docs/2026-05-13_FLAWS.md` | The Pearl-spec audit that drove the v2 → v3 redesign |
| `docs/2026-05-13_PEARL_COMPARISON.md` | Divergence inventory + closure tracking |

## Tests

`cargo test -p ai-pow` runs the crate's unit, integration, and fixture tests:

- 53 unit (params, prng, matmul, tile_hash, commit, fiat_shamir, proof, synth)
- 19 adversarial (every verifier rejection path exercised by tampering)
- nonce-grinding regressions: different nonces re-key commitments, change
  noise/tile states before final hashing, and stale attempt contexts are
  rejected
- 13 end-to-end (round-trip prove → verify)
- 5 LLM-shape (rectangular, non-pow-2 tile counts, Gemma 4 / Qwen 3.6 FFN profiles)
- 11 Pearl-compat fixtures (sections S0 – S9 above)
- 3 soundness Monte-Carlo (rejection rate vs `1 − (1 − f)^σ`)

Plus 1 `#[ignore]`-d `gen_fixtures` test that regenerates `tests/fixtures/pearl.rs`
from vendored Pearl reference code.

## Difficulty-bound parameters

Pearl §4.8 constraints (enforced by `MatmulParams::validate`):

- `noise_rank` must be a power of two with `r ≥ 2`
- `k` must be a multiple of `noise_rank`
- `k ≤ 2^16`
- `tile` must divide both `m` and `n`
- `spot_checks` must be > 0 and ≤ the total tile count
