# `ai-pow`

A Pearl-style proof-of-useful-work matrix-multiplication puzzle for
Nockchain. The crate implements the `(A + E)(B + F)` tiled matmul puzzle
from the Pearl Whitepaper (`Pearl_Whitepaper.pdf` at the repo root)
end-to-end: low-rank noise generation, tile-by-tile mining with an
iterative 512-bit accumulator state, shape-aware difficulty thresholds,
and a replication-style verifier.

This is **Phase 1** of the AI-PoW track. Phase 2 (`crates/ai-pow-vi`)
extends the same primitives to a verifiable-inference puzzle on real LLM
weights.

## Scope

What `ai-pow` provides:

- **Inputs**: caller-supplied INT8 matrices `A` (m × k row-major) and `B`
  (n × k column-major), each entry in `[-64, 64]` (Pearl §4.1).
- **Mining**: `mine(block_commitment, nonce, a, b, params, opts)` searches
  for a tile whose keyed-hash of the tile state falls below a shape-aware
  difficulty target `2^(256-b) · r · t²` (Pearl §4.5).
- **Verification**: `verify(block_commitment, nonce, params, proof)`
  replays one tile from the supplied row/column strips plus per-block
  noise, then re-checks both the tile-state commitment and σ
  Fiat-Shamir-sampled spot tiles.
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

The Pearl upstream itself is vendored under `pearl/` for read-only
comparison; the Pearl ISC license is reproduced verbatim at
[`LICENSE-PEARL`](LICENSE-PEARL).

## Layout

| Path | Purpose |
|---|---|
| `src/lib.rs` | Public re-exports |
| `src/params.rs` | `MatmulParams` and validation (Pearl §4.8 constraints) |
| `src/prng.rs` | Pearl-compatible PRNG building blocks (`pearl_random_hash`, uniform-noise, permutation, A/B synth) |
| `src/matmul.rs` | `BlockNoise`, `Matrices` (`A' = A + E`, `B' = B + F`), `TileState`, `compute_tile`, `compute_tile_from_slices` |
| `src/tile_hash.rs` | `difficulty_target`, `hash_le_target` (little-endian U256 semantics) |
| `src/commit.rs` | Tile-state Merkle (sentinel-padded) and Pearl `matrix_commitment` chunk-Merkle |
| `src/fiat_shamir.rs` | Pearl §4.3 commitment-hash chain + per-nonce pow-key derivation |
| `src/prover.rs` | `BlockContext`, `mine`, `mine_block` |
| `src/verifier.rs` | `verify` |
| `src/proof.rs` | `MatmulProof`, `TileOpening`, encode / decode |
| `src/synth.rs` | Deterministic `(A, B)` test synthesis |
| `tests/` | End-to-end, adversarial, soundness simulation, LLM-shape, Pearl-compat fixtures |
| `docs/2026-05-13_FLAWS.md` | The Pearl-spec audit that drove the v2 → v3 redesign |
| `docs/2026-05-13_PEARL_COMPARISON.md` | Divergence inventory + closure tracking |

## Tests

`cargo test -p ai-pow` runs 109 tests across 7 binaries:

- 53 unit (params, prng, matmul, tile_hash, commit, fiat_shamir, proof, synth)
- 19 adversarial (every verifier rejection path exercised by tampering)
- 5 block-noise cache (`mine_block` amortization)
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
