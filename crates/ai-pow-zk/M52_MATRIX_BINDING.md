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
| 1 | Extend `CompositePublicInputs` with `HASH_A` / `HASH_B` (16 Goldilocks) | ✅ landed | +4 | 276 unit |
| 2 | Selector-gated AIR binding: `IS_HASH_A · (CV_OUT − PI_HASH_A) = 0`, ditto for B | ⏳ pending | — | — |
| 3 | `composite_trace::place_matrix_hash_a` / `place_matrix_hash_b` (chunk-Merkle instruction emission) | ⏳ pending | — | — |
| 4 | Cross-chip binding — BLAKE3 absorbs from `noised_packed` LogUp bus | ⏳ pending | — | — |
| 5 | Plain-side wire-up in `ai-pow` (block header + `matrix_commitment` call site) | ⏳ pending | — | — |
| 6 | TEST_SMALL end-to-end bench + correctness test | ⏳ pending | — | — |
| 7 | PROD-scale evaluation (measure or model prove time at 4096²) | ⏳ pending | — | — |

### Step 2 design refinement

Originally proposed `when_last_row()` binding. Switched to
**selector-gated** binding on the producing row:
```
IS_HASH_A · (CV_OUT[i] − PI_HASH_A[i]) = 0   for i in 0..8
IS_HASH_B · (CV_OUT[i] − PI_HASH_B[i]) = 0
```
Reason: avoids 16 dedicated passthrough columns (we already pay
~600+ columns for the chip layout, and 16 more for purely PI
threading would be wasted).

The control chip enforces `IS_HASH_A · (IS_HASH_A − 1) = 0`
(boolean) but **not** `Σ IS_HASH_A = 1`. So the binding is:
- If 0 rows fire → constraint vacuous, PI unconstrained (only OK
  for baseline test traces; step 3's trace generator MUST always
  emit a hash-finalize row when an actual matrix is committed).
- If 1 row fires → CV_OUT on that row must equal PI_HASH_A.
- If >1 rows fire → all firing rows must agree on CV_OUT.

Uniqueness in production traces is a **trace-generator obligation**
(`place_matrix_hash_a` emits exactly one finalize row). Adding an
explicit `Σ IS_HASH_A = 1` AIR constraint is possible but requires
a running-sum auxiliary column; deferred until / unless we see a
soundness need.

## Decisions log

- **2026-05-14**: Option 1 chosen over Option 2-5 (see Design decision section).
- **2026-05-14**: TEST_SMALL is the staging target for steps 1-6; PROD viability is step 7.
- **2026-05-14**: Block-header `H_A` value is the **same** as the SNARK PI `HASH_A` — single field, not two (Option 1 advantage).
- **2026-05-14**: PI binding for HASH_A / HASH_B is **selector-gated** at the row where `IS_HASH_A` / `IS_HASH_B` fires, not last-row-passthrough. Saves 16 trace columns.
- **2026-05-14**: `derive_from_matrix` scans for the first row with `IS_HASH_A == 1` (resp. `IS_HASH_B`); baseline traces with no such row produce zero hash PIs. Matches the AIR semantics where the constraint vacuously holds when the selector is zero.

## Open questions

- How does BLAKE3's chunk-counter / tweak get supplied into the chip
  for the chunk-Merkle internal-node hashing? (BLAKE3 uses different
  tweak flags for chunk-start, chunk-end, parent, root, keyed.)
- Does the existing `noised_packed` bus have enough columns to also
  serve BLAKE3 absorb reads, or do we need a parallel bus?

These get resolved during steps 3-4 and recorded back here.

## Step 3 algorithmic specification

Mirrors `crates/ai-pow/src/commit.rs::matrix_commitment`, which is
defined byte-for-byte by `BLAKE3::new_keyed(κ).update(pad(matrix)).finalize()`.
The BLAKE3 standard's chunk-Merkle internal structure is the
algorithm we unroll into AIR instructions.

### Inputs

- `matrix_bytes`: `&[u8]` — raw row-major A or col-major B bytes.
- `key`: `&[u8; 32]` — the κ key (the same one used in `matrix_commitment`).
- `is_a`: `bool` — set `IS_HASH_A` (true) or `IS_HASH_B` (false) on the root row.

### Algorithm

```
padded = pad_to_chunk_boundary(matrix_bytes)         // 1024-multiple
num_chunks = padded.len() / 1024                     // ≥ 1
key_words = key as 8 little-endian u32 words

// CHUNK LAYER — for each chunk c in 0..num_chunks:
chunk_cvs = []
for c in 0..num_chunks {
    cv = key_words
    for b in 0..16 {
        block_bytes = padded[c*1024 + b*64 ..][..64]
        message = block_bytes as 16 little-endian u32 words
        flags = KEYED_HASH                            // bit 4 = 0x10
            | (CHUNK_START if b == 0 else 0)          // bit 0
            | (CHUNK_END   if b == 15 else 0)         // bit 1
            | (ROOT if num_chunks == 1 && b == 15 else 0)  // bit 3
        tweak = Blake3Tweak {
            counter_low: c as u32,
            counter_high: (c >> 32) as u16,
            block_len: 64,
            flags,
        }
        cv = place_blake3_hash(row, &message, &cv, &tweak)
        row += 8
    }
    chunk_cvs.push(cv)
}

// PARENT LAYER — binary-tree reduce.
// Note: BLAKE3 standard requires non-power-of-2 chunk counts to
// promote unpaired CVs to the next layer (no padding). For our
// `pad_to_chunk_boundary` input, `num_chunks` is always a power
// of 2 in practice because matrix sizes are.
while chunk_cvs.len() > 1 {
    next_layer = []
    is_top_layer = chunk_cvs.len() == 2
    for pair in chunk_cvs.chunks(2) {
        let (l, r) = (pair[0], pair[1])
        message = concat(l_bytes, r_bytes) as 16 u32 words
        flags = KEYED_HASH | PARENT                   // 0x10 | 0x04 = 0x14
            | (ROOT if is_top_layer else 0)           // 0x08
        tweak = Blake3Tweak {
            counter_low: 0, counter_high: 0,
            block_len: 64, flags,
        }
        cv = place_blake3_hash(row, &message, &key_words, &tweak)
        // ROOT compression's CV is the matrix commitment.
        next_layer.push(cv)
        row += 8
    }
    chunk_cvs = next_layer
}

// chunk_cvs.len() == 1, that's H_A (or H_B).
root_row = row - 1   // the finalize row of the last placed block
set selectors[IS_HASH_A_INDEX] = true on root_row
```

### Row budget (estimates)

| Shape | num_chunks | chunk-layer rows | parent rows | Total / matrix |
|---|---|---|---|---|
| TEST_SMALL (4 KiB) | 4 | 4·16·8 = 512 | 3·8 = 24 | 536 |
| TEST_PEARL-ish (4 chunks) | 4 | 512 | 24 | 536 |
| PROD (16 MiB) | 16384 | 16K·16·8 ≈ 2.1M | (16K−1)·8 ≈ 131K | ~2.2M |

### Open detail (for step 3 implementation)

The current `place_blake3_hash` API sets `selectors[9] = true`
(IS_LAST_ROUND) but no other selectors. We need to either extend
the API with `extra_selectors: &[usize]` or expose a small
post-hook helper that ORs in `IS_HASH_A` on the chosen row. The
selector packing in `CONTROL_PREP` must also be updated coherently
(otherwise the control chip's "selector ≡ packed bits" constraint
rejects the row).

The cleanest fix: thread an optional `extra_selectors` parameter
through `place_blake3_hash`. Only the root parent compression
needs it; everything else is `&[]`.
