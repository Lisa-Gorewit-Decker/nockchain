# M10.1c — phase-by-phase progress

Live document tracking the Plonky3 port of Pearl's composite-AIR
zk-PoW circuit. See [`M10_1C_DESIGN.md`](M10_1C_DESIGN.md) for the
architectural plan. This file tracks **what has landed**, **what's
verified**, and **what's still pending**.

Update rule: every commit that lands a phase updates this file in
the same commit. If a phase changes scope mid-flight, document the
delta here so future sessions can pick up cold.

## Tooling preference (locked)

Where Plonky3 ships a crate / primitive that does the job, **use
it directly or with a tiny vendor patch** (per user direction).
Examples:
  * BLAKE3 → vendored M10.1b `blake3_chip` (Pearl-compat) instead of
    a from-scratch one-round-per-row port of Pearl's chip.
  * Range tables → `p3-lookup`'s LogUp gadget instead of hand-rolled
    range-table AIRs.
  * STARK plumbing → `p3-uni-stark` (already in use); switching to
    `p3-batch-stark` for multi-AIR is reconsidered per-phase.

When Plonky3 doesn't have a direct primitive (e.g. Pearl's
`NOISED_PACKED` RAM-lookup architecture), port Pearl's design.

## Phase status

| # | Phase | Status | Tests added | Cumulative tests |
|---|---|---|---|---|
| 1 | Design (`M10_1C_DESIGN.md`) | ✅ landed | — | — |
| 2 | `composite_layout` base + `TEST_PEARL` + `block_commitment` pin | ✅ landed | 3 | 136 unit |
| 2.5 | `composite_layout` RAM-lookup column extension | ✅ landed | 3 | 139 unit |
| 3 | `stark_row_chip` (Pearl `monotonic_increment`) | ✅ landed | 9 | 148 unit |
| 4 | range-table + input chips (Pearl `urange*` / `irange*` / `i8u8` / `input/`) — uses `p3-lookup` | ⬜ pending | | |
| 5 | `control_chip` (Pearl `control_and_matid_packed`) | ⬜ pending | | |
| 6 | preprocessed-trace generation (Pearl `pearl_preprocess`) | ⬜ pending | | |
| 7 | BLAKE3 chip — wrap M10.1b vendored chip (Plonky3 primitive preferred) | ⬜ pending | | |
| 8 | BLAKE3 chip — extend wrapper with multi-round / Merkle linkage | ⬜ pending | | |
| 9 | matmul chip with `NOISED_PACKED` RAM-lookup reads | ⬜ pending | | |
| 10 | jackpot chip (rotate-XOR-13, Pearl `chip/jackpot/`) | ⬜ pending | | |
| 11 | `composite_lookups` — `p3-lookup` config for all 6+ lookups | ⬜ pending | | |
| 12 | `composite_full_air::eval` (Pearl `pearl_air`) | ⬜ pending | | |
| 13 | `composite_trace` (Pearl `pearl_trace`) | ⬜ pending | | |
| 14 | `lib::{prove, verify}` plumbing on composite AIR | ⬜ pending | | |
| 15 | PROD bench full shape | ⬜ pending | | |

**Today's cumulative test count: 139 unit + 7 KAT + 3 ignored
PROD bench.**

## Properties validated per phase

This section tracks **the specific cryptographic / semantic
properties each phase's tests enforce**. The goal is to make every
phase's contribution to the final security argument auditable.

### Phase 2 + 2.5 — layout pinning

- ✅ Every column-width matches Pearl's `pearl_layout.rs` verbatim
  (`composite_layout::tests::ram_lookup_column_widths_match_pearl`).
- ✅ Column offsets are strictly increasing and contiguous — no
  accidental overlap or gap
  (`composite_layout::tests::layout_offsets_are_contiguous`).
- ✅ `TOTAL_TRACE_WIDTH ≈ 1328` (Pearl ballpark) — guards against
  accidental column duplication
  (`composite_layout::tests::total_trace_width_in_pearl_ballpark`).
- ✅ `block_commitment` pinned at 32 bytes = 8 × u32 LE matching
  Tip5 digest size — merge-mining compat anchor
  (`composite_layout::tests::block_commitment_layout_matches_8_u32_le`).
- ✅ `TEST_PEARL` profile admits degree-3 constraints
  (`circuit::tests::build_stark_config_test_pearl_assembles`).

### Phase 3 — `stark_row_chip` (landed)

Properties validated:
  - ✅ First-row constraint: `STARK_ROW_IDX[0] == 0`
    (`prove_and_verify_valid_monotonic_trace`,
    `verify_rejects_nonzero_first_row`).
  - ✅ Transition constraint: `STARK_ROW_IDX[i+1] == STARK_ROW_IDX[i] + 1`
    (`verify_rejects_broken_increment`, `verify_rejects_skipped_index`).
  - ✅ Combined: trace at every row equals its row index
    (`valid_trace_has_correct_row_indices`).
  - ✅ Late tamper detection — constraint chain catches mutations
    deep in the trace (`verify_rejects_late_tamper`).
  - ✅ Production-scale smoke test at `MIN_STARK_LEN = 8192` rows
    (`prove_and_verify_min_stark_len_trace`).
  - ✅ `fill_row` trace-side helper writes correct values
    (`fill_row_writes_row_index`).
  - ✅ Chip constructs (zero-state ZST) (`chip_constructs`).

Test infrastructure established: `StarkRowOnlyAir` wrapper pattern
(thin AIR-trait impl that calls just the chip's `eval`) will be
reused by every subsequent chip's test module.

### Phase 4 — range tables + input chip (pending)

### Phase 4 — range tables + input chip (pending)

Properties to validate:
  - URANGE8: every `URANGE8_TABLE` row in `0..256`, every reader
    in-range via LogUp.
  - URANGE13: every `URANGE13_TABLE` row in `0..8192`.
  - IRANGE7P1: every `IRANGE7P1_TABLE` row in `-64..=64`.
  - IRANGE8: every `IRANGE8_TABLE` row in `-128..=127`.
  - I8U8: `(i8_value, u8_value)` pairs match the two's-complement
    convention.
  - INPUT chip: `MAT_UNPACK[i] in [-64, 64]` (IRANGE7P1) and
    `UINT8_DATA[i] = MAT_UNPACK[i] + 128` when `IS_MSG_MAT` fires.

### Phase 5-15 (pending)

Properties to be enumerated as each phase lands.

## Cumulative cryptographic guarantees

At each milestone, what properties are cryptographically enforced
by the SNARK as a whole:

| Milestone | Property | Strength |
|---|---|---|
| M9 (matmul only) | per-stripe INT8 dot product correctly computed | Bare matmul; nothing tied to public inputs. |
| M9.1 | + rotate-XOR-13 state chain (single slot) | Same; matmul + state internally consistent. |
| M10 | + Fiat-Shamir absorption of `PublicInputs` | PIs can't be swapped at verify time. |
| M10.1a | + `BLAKE3-keyed(m_final, pow_key) == found_leaf` (out-of-circuit) | Closes "fake jackpot" attack. |
| M10.1b | + same relation proved in-circuit (Pearl-compat hash) | Self-contained SNARK; merge-mining preserved. |
| **M10.1c target** | + `a_rows` / `b_cols` bound to `h_a` / `h_b` via in-circuit BLAKE3 + RAM lookups | **Restores PoUW property — adversary cannot substitute matrices.** |
| M12 (future) | + recursion compression to ~60 KB | Block-budget friendly. |

## Open questions / risks

1. **Plonky3 preprocessed trace API maturity.** Pearl's design
   relies heavily on preprocessed columns committed at setup.
   Plonky3 supports them via `Air::preprocessed_main` but our crate
   hasn't exercised this path. Phase 6 will be the proof point;
   if the API doesn't fit cleanly, we may need to commit
   preprocessed values as a separate "public values" block.
2. **`p3-batch-stark` integration.** If Phase 7 ends up using the
   M10.1b chip side-by-side with the composite trace, we may need
   `p3-batch-stark` for multi-AIR proving. Plonky3 ships it but
   we haven't used it yet.
3. **Memory at production shape.** Phase 15 (PROD bench at full
   shape) is the only phase where we'll discover whether the
   ~1300-col trace × 8192+ rows actually fits in reasonable
   prover memory. If not, M11.1-shape benchmarks may need to
   stage matrix chunks across multiple proofs.

## Session log

| Date (PT) | Session deliverable | Commits |
|---|---|---|
| 2026-05-13 | M9.1 composite tile AIR | `0dad313` |
| 2026-05-13 | M10 PI threading | `9d856c6` |
| 2026-05-13 | M11 PROD bench + M12 docs | `f781a0e` |
| 2026-05-13 | M10.1a found-leaf out-of-circuit | `1cc5dc2`, `838fe5c` |
| 2026-05-13 | M10.1b vendored Pearl-compat BLAKE3 chip | `d084e70` |
| 2026-05-13 | M10.1b in-circuit found-leaf | `f7e03cd`, `052288d` |
| 2026-05-13 | M10.1c design (Plonky3 port of Pearl) | `240ce28` |
| 2026-05-13 | M10.1c Phase 2 layout + TEST_PEARL | `be53f3b` |
| 2026-05-13 | M10.1c Phase 2.5 RAM-lookup columns | `571eaf0`, `19a6c47` |
| 2026-05-14 | M10.1c Phase 3 `stark_row_chip` | (this commit) |
