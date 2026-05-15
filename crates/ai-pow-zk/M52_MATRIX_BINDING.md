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
| 1 | Extend `CompositePublicInputs` with `HASH_A` / `HASH_B` (16 Goldilocks) | ✅ landed (`d5b77c7`) | +3 | 275 unit |
| 2 | Selector-gated AIR binding: `IS_HASH_A · (CV_OUT − PI_HASH_A) = 0`, ditto for B | ✅ landed (`d5b77c7`) | (covered in step 3) | — |
| 3 | `composite_trace::place_matrix_hash_a` / `place_matrix_hash_b` (chunk-Merkle instruction emission) | ✅ landed (`dfa5a9f`) | +6 | 303 unit |
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

### Resolved detail (landed in step 3)

`place_blake3_hash` now has a sibling `place_blake3_hash_with_selectors`
that ORs caller-supplied selector indices into the finalize-row
CONTROL_PREP packing. The chunk-Merkle root compression passes
`&[4]` (IS_HASH_A) or `&[5]` (IS_HASH_B); every other compression
passes `&[]`. The `ControlChip.fill_row` helper computes the
packed bits coherently, so the control chip's "selector ≡ packed
bits" constraint is satisfied automatically.

## Step 4 plan (next session)

**Cross-chip binding.** Today the BLAKE3 chip hashes whatever
bytes are in `BLAKE3_MSG` and the matmul reads bytes from
`A_NOISED_UNPACK` / `B_NOISED_UNPACK`. Nothing forces them to be
the same matrix — an adversary could hash matrix X and run matmul
on matrix Y, and both proofs verify.

### Design issue discovered: the existing bus binds the wrong thing

Documentation in `composite_lookups.rs:25` says the
`noised_packed` bus has BLAKE3 as a querier ("blake3 (UINT8_DATA
when IS_MSG_MAT)"). On closer inspection, the **bus emission code
in `composite_full_air_with_lookups.rs::bus_emit::noised_packed`
(lines 271-308) only emits matmul-side queries**. The BLAKE3
querier is documented intent, not implemented.

More fundamentally, the **bus key is `(MAT_ID, NOISED_PACKED[0],
NOISED_PACKED[1])`** — the *noised* matrix bytes (matrix + noise
packed via polyval). But BLAKE3 hashes **plain matrix bytes**
(no noise added). Putting the two on the same bus requires
re-derivation of noise on the BLAKE3 side or a different
binding scheme.

The bus is actually called "noised_packed" because it binds the
noised store. Plain matrix bytes (what BLAKE3 hashes) live in
`MAT_UNPACK` (8 i8 cells per row, no noise), which today binds
to `UINT8_DATA` via the `i8u8` bus when `IS_MSG_MAT = 1` — but
that's a row-local constraint, not a cross-row matrix-store
binding.

### Two viable approaches for step 4

**4-A. Add a `plain_matrix_bytes` LogUp bus.** Key shape:
`(MAT_ID, polyval(MAT_UNPACK[0..4]), polyval(MAT_UNPACK[4..8]))`.
The `input` chip's existing constraint `NOISED_PACKED[i] =
polyval(MAT_UNPACK) + polyval(NOISE_UNPACK)` means we can derive
`polyval(MAT_UNPACK[..])` per row. Both matmul and BLAKE3 query
this bus with their `(MAT_ID, mat_bytes)` views. The table is
emitted by the input-chip rows that load the canonical store
(one entry per matrix-data row).

**4-B. Repurpose `noised_packed` with a derived value.** The
BLAKE3 chip's row already has `MAT_UNPACK` (matrix bytes, since
BLAKE3 absorbs plain bytes), `NOISE_UNPACK` (zero on BLAKE3 rows
— no noise injected), and the polyval-pack relationship still
holds: `NOISED_PACKED[i] = polyval(MAT_UNPACK[..]) + polyval(0..)
= polyval(MAT_UNPACK[..])`. On BLAKE3 rows, `NOISED_PACKED`
*equals* the plain-byte polyval. So if we have BLAKE3 rows query
`(MAT_ID, NOISED_PACKED[0], NOISED_PACKED[1])` with
`NOISE_UNPACK = 0`, they get plain-byte semantics. The matmul
rows already query the same shape but with noise mixed in.
**Distinct table rows for "noised" vs. "plain" entries** (matmul
queries one, BLAKE3 the other) — both come from a canonical
input-chip-populated store.

4-B is cheaper (no new bus). It requires: per-row NOISE_UNPACK is
zero on BLAKE3-hash rows, the input chip's preprocessed-store
rows for matrix-A include both a "noised" entry (matmul reads)
and a "plain" entry (BLAKE3 reads), differentiated by an extra
flag column or by MAT_ID range partitioning.

### Sub-plan for step 4 (approach 4-B selected)

| # | Sub-step | Files touched |
|---|---|---|
| 4.0 | **Design ratification** — confirm approach 4-B (reuse `noised_packed` with NOISE_UNPACK=0 on BLAKE3 rows) over 4-A (new bus). 4-B requires distinct table rows for "noised" vs. "plain" matrix entries, differentiated by NOISE_UNPACK value or MAT_ID range. | — |
| 4.1 | Add BLAKE3-side query in `bus_emit::noised_packed`: when `IS_MSG_MAT = 1`, emit `(MAT_ID, NOISED_PACKED[0], NOISED_PACKED[1])` query with multiplicity 1. Update `composite_lookups::noised_packed_freq` to accept `n_blake_reads` as a third input. | `composite_full_air_with_lookups.rs`, `composite_lookups.rs` |
| 4.2 | New trace helper: on each matrix-hash row, write matrix bytes into `MAT_UNPACK`, zeros into `NOISE_UNPACK`, compute `NOISED_PACKED = polyval(MAT_UNPACK)`, set `MAT_ID` to a unique per-row matrix-byte index, mirror to `UINT8_DATA` via i8↔u8 conversion, set `IS_MSG_MAT = 1`. | `composite_trace.rs` |
| 4.3 | Preprocessed-trace update: emit table-side rows for plain-byte entries (one per matrix byte block) alongside the noised entries. | `composite_preprocess.rs`, `chips/input.rs` |
| 4.4 | Extend `populate_lookup_freq` to count BLAKE3-side queries per matrix-hash row. | `composite_trace.rs` |
| 4.5 | Soundness test: prove a trace with `place_matrix_hash_a` on matrix X, tamper one byte in X post-hash but leave NOISED_PACKED intact. LogUp balance must fail. | `composite_trace.rs` tests |
| 4.6 | Soundness test: BLAKE3 hashes matrix X but NOISED_PACKED is populated with matrix Y. LogUp balance must fail. | `composite_trace.rs` tests |

### Why step 4 expanded vs. the initial scope

Initial scope assumed the `noised_packed` bus already accepted
BLAKE3-side queries (per the comment in `composite_lookups.rs:25`).
Reading the emission code (`composite_full_air_with_lookups.rs:271-308`)
revealed only matmul-side queries are emitted; BLAKE3 querying is
documented intent, not implemented. Also the bus key is over
*noised* bytes (mat + noise polyval-packed), while BLAKE3 hashes
plain matrix bytes — so straight reuse doesn't work without
either (a) a new bus, or (b) BLAKE3 rows asserting `NOISE_UNPACK = 0`
so that `NOISED_PACKED[i] = polyval(MAT_UNPACK[..])` becomes the
plain-byte polyval.

### Why this isn't done in step 3

Step 3 demonstrates the AIR happily proves matrix-hash blocks
when there's no cross-binding requirement (the BLAKE3 chip's
per-row constraints are satisfied because UINT8_DATA stays zero
and IS_MSG_MAT stays zero, so the BLAKE3↔UINT8_DATA consistency
check is vacuous). Step 4 turns IS_MSG_MAT on and requires the
NOISED_PACKED table to actually exist for the hashed bytes.

## Session log (2026-05-14)

**Commits landed:**
- `08d0d37` M52 roadmap doc
- `e8e5920` FRI parameter sweep (separate from M52)
- `d5b77c7` M52 steps 1-2 — PI plumbing + selector-gated AIR binding (275 tests)
- `dfa5a9f` M52 step 3 — `place_matrix_hash_a` / `place_matrix_hash_b` byte-equivalent to `blake3::Hasher::new_keyed` (303 tests)
- `6f592c4` M52 roadmap — mark 1-3 landed, document step 4

**Test gains:**
- +9 tests for M52 (3 PI plumbing + 6 chunk-Merkle generator)
- Steps 1-3 deliver: HASH_A / HASH_B in PIs, selector-gated AIR
  binding, BLAKE3 chunk-Merkle trace generation, end-to-end
  prove+verify with matrix-hash in trace, PI-tamper rejection.

**Outstanding soundness gap:** Without step 4, an adversary can
freely choose what bytes the BLAKE3 chip hashes vs. what the
matmul reads. Step 4 closes this via the existing `noised_packed`
LogUp bus (machinery already in place, just needs trace-generator
wire-up).
