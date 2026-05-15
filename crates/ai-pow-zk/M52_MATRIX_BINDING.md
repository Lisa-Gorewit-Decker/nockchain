# M52 — `H_A` / `H_B` matrix bindings (Option 1)

Live document tracking the implementation of in-circuit matrix
commitments via BLAKE3 chunk-Merkle (Pearl-byte-equivalent), the
deepest gap identified in `ENGINEERING_REPORT.md §6` / task #52.

Update rule: every commit that lands a step updates this file in
the same commit. Decisions and rationale captured inline so future
sessions can pick up cold.

## Goal

Add cryptographic binding from the matmul A / B matrices to public-
input fields `HASH_A` / `HASH_B`. After this lands, an adversary
cannot pick arbitrary A, B; they must commit to A, B in the block
header and the SNARK proves consistency.

This closes the deepest of the three structural gaps identified in
`crates/ai-pow`'s evaluation against the cuPOW + Pearl papers (the
other two — low-rank noise and step-bound tile state — are out of
scope for M52).

## Design decision: Option 1 (BLAKE3 chunk-Merkle in AIR)

Discussed alternatives (2026-05-14):

1. **BLAKE3 chunk-Merkle in AIR.** Matches Pearl's SNARK approach.
   Reuses the existing BLAKE3 chip. Block header `H_A` = SNARK PI
   `HASH_A`. Expensive at PROD scale.
2. **Tip5 over full matrix.** Cheaper per-byte (~half the rows of
   BLAKE3 per matrix). Needs a new Tip5 absorb-chain chip. Block
   header carries both `T_A` (our binding) and `H_A` (Pearl hint).
3. **Per-row / per-col custom Merkle, only-touched opening.** O(σ·t)
   not O(m·n). Big cost win. Breaks SNARK-PI Pearl-compat (block
   header still carries Pearl `H_A` as a merge-mining hint).
4. **Probabilistic spot-checks.** Subset of 3 with statistical
   binding.
5. **Status quo (block-derived A, B).** No PoUW.

**Decision: Option 1.** Rationale captured by user, 2026-05-14:
> "Keep it simple, do Option 1." Earlier: SNARKs do not need to be
> byte-equivalent with Pearl, but the unit of work fed into the
> SNARK does. Merge-mining must be possible; each chain's SNARK is
> separate.

Why Option 1 over Option 2 once the merge-mining constraint was
clarified:
- The existing BLAKE3 chip already covers `is_hash_a` / `is_hash_b`
  selector slots and the BLAKE3 round AIR primitives. Reusing it
  avoids a greenfield Tip5 chip.
- Block-header `H_A` doubles as both SNARK PI and merge-mining
  hint — one value, not two.
- BLAKE3 must remain in the AIR regardless for the **difficulty
  check** (`BLAKE3(M, key=s_a) ≤ target`) and Fiat-Shamir
  transcript. Option 2 doesn't remove BLAKE3, it adds Tip5
  alongside.

## Cost reality (sized 2026-05-14)

Pearl-prod matrix is 4096² i8 = 16 MiB. Inside the AIR:

| Matrix shape | Bytes | Chunks | Leaf compressions | Parent compressions | Total compressions | AIR rows (~7/comp) |
|---|---|---|---|---|---|---|
| **TEST_SMALL** (64²) | 4 KiB | 4 | 64 | 3 | 67 | ~470 |
| **TEST_PEARL** (matches Pearl `MIN_STARK_LEN = 8192`) | varies | — | — | — | — | fits in 8K |
| **PROD** (4096²) | 16 MiB | 16384 | 262144 | 16383 | 278527 | ~2M rows per matrix → 4M total |

Implication: PROD-scale Option 1 needs `MIN_STARK_LEN` bumped from
8192 to ~8M and accepts prove times in the hour range (matches
Pearl, which runs at similar scale). Until M12 recursion lands,
production deployment of PROD-shape proofs isn't practical. **The
work is structured to validate at TEST_SMALL first** and treat
PROD-scale as a separate viability gate (step 7).

## Phase status

| # | Step | Status | Tests added | Cumulative |
|---|---|---|---|---|
| 1 | Extend `CompositePublicInputs` with `HASH_A` / `HASH_B` (16 Goldilocks) | ⏳ pending | — | 272 unit |
| 2 | `composite_full_air::when_last_row()` binding for HASH_A / HASH_B | ⏳ pending | — | — |
| 3 | `composite_trace::place_matrix_hash_a` / `place_matrix_hash_b` (chunk-Merkle instruction emission) | ⏳ pending | — | — |
| 4 | Cross-chip binding — BLAKE3 absorbs from `noised_packed` LogUp bus | ⏳ pending | — | — |
| 5 | Plain-side wire-up in `ai-pow` (block header + `matrix_commitment` call site) | ⏳ pending | — | — |
| 6 | TEST_SMALL end-to-end bench + correctness test | ⏳ pending | — | — |
| 7 | PROD-scale evaluation (measure or model prove time at 4096²) | ⏳ pending | — | — |

## Decisions log

- **2026-05-14**: Option 1 chosen over Option 2-5 (see Design decision section).
- **2026-05-14**: TEST_SMALL is the staging target for steps 1-6; PROD viability is step 7.
- **2026-05-14**: Block-header `H_A` value is the **same** as the SNARK PI `HASH_A` — single field, not two (Option 1 advantage).

## Open questions

- How does BLAKE3's chunk-counter / tweak get supplied into the chip
  for the chunk-Merkle internal-node hashing? (BLAKE3 uses different
  tweak flags for chunk-start, chunk-end, parent, root, keyed.)
- Does the existing `noised_packed` bus have enough columns to also
  serve BLAKE3 absorb reads, or do we need a parallel bus?

These get resolved during steps 3-4 and recorded back here.
