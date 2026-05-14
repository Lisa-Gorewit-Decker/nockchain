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
| 4a | `range_table` chip (URange8/13, IRange7P1/8 generic) | ✅ landed | 15 | 163 unit |
| 4b | `i8u8` chip (signed↔unsigned conversion table) | ✅ landed | 11 | 174 unit |
| 4c | `input` chip (Pearl `chip/input/`) | ✅ landed | 9 | 183 unit |
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

### Phase 4a — range tables (landed)

Properties validated by the generic `RangeTableChip<COL, MIN, MAX>`
with four concrete instantiations (`URange8`, `URange13`,
`IRange7P1`, `IRange8`):

  - ✅ First row equals `MIN`
    (`prove_and_verify_*_table`, `*_verify_rejects_wrong_first_row`).
  - ✅ Last row equals `MAX`
    (`urange8_verify_rejects_wrong_last_row`).
  - ✅ Transition delta is boolean — column value stays the same
    or increments by 1
    (`urange8_verify_rejects_non_boolean_delta`,
    `irange8_verify_rejects_non_boolean_delta`).
  - ✅ Combined: column enumerates every integer in `[MIN..=MAX]`
    by discrete intermediate-value argument
    (`*_table_fills_correctly`).
  - ✅ Padding rows past `span` replay `MAX`
    (`irange7p1_padding_repeats_max`).
  - ✅ `span()` const helper matches `MAX − MIN + 1`
    (`*_span_is_*` per chip).
  - ✅ Production-scale `URANGE13` at 8192 rows
    (`prove_and_verify_urange13_table`).

Subsequent LogUp lookups (Phase 11) will tie the *consumer* side
to these tables: every reader's value must appear, with the
correct multiplicity, in the matching range table. The table's
own integrity (it really does enumerate `[MIN..=MAX]`) is what
Phase 4a pins.

### Phase 4b — I8U8 conversion table (landed)

Properties validated:
  - ✅ AUX column is boolean (`rejects_non_boolean_aux`).
  - ✅ AUX starts at 0 (`rejects_aux_first_row_nonzero`) and ends
    at 1 (`rejects_aux_last_row_zero`).
  - ✅ AUX is monotonic non-decreasing — once it flips to 1 it
    stays 1 (`rejects_aux_non_monotonic`).
  - ✅ AUX transitions from 0→1 only when `pack = -1`
    (`rejects_aux_transition_off_boundary`).
  - ✅ Pack starts at `−128 × 256 + 128 = −32640`
    (`rejects_wrong_first_pack`) and ends at `127 × 256 + 127 =
    32639` (`rejects_wrong_last_pack`).
  - ✅ Per-transition step is either +257 (standard) or +1 (boundary)
    (`rejects_wrong_intermediate_pack`).
  - ✅ Combined: column enumerates all 256 valid `(i8, u8)` pairs
    by the discrete-step argument (255 transitions × 257 + 1 ×
    sign-boundary = 65279 = MAX − MIN)
    (`prove_and_verify_valid_i8u8_table`).
  - ✅ `fill_row` writes canonical Pearl-pack values
    (`fill_row_encodes_pearl_pack`).

### Phase 4c — input chip (landed)

Properties validated:
  - ✅ `NOISE_PACKED_PREP == polyval(NOISE_UNPACK, base = 129)` —
    forces the preprocessed noise word to equal the polyval of
    the i7+1 noise bytes (`rejects_wrong_noise_packed_prep`).
  - ✅ `NOISED_PACKED[i] == polyval(MAT_UNPACK[i*4..(i+1)*4], 256)
    + polyval(NOISE_UNPACK[i*4..(i+1)*4], 256)` — ties the
    canonical noised-matrix store to the unpacked bytes
    (`rejects_wrong_noised_packed`).
  - ✅ Tampering with MAT_UNPACK while leaving NOISED_PACKED
    unchanged fails (`rejects_tampered_mat_byte`,
    `cannot_diverge_mat_from_noised_packed`). **This is the
    constraint that makes the matmul ↔ BLAKE3 RAM-lookup linkage
    cryptographically meaningful** — an adversary can't read fake
    matrix bytes through NOISED_PACKED.
  - ✅ Boundary noise values `{-64, 64}` are admitted
    (`handles_boundary_noise_values`).
  - ✅ Packing bases pinned at 129 (noise) and 256 (matrix)
    (`noise_packing_base_is_129`, `matrix_packing_base_is_256`).
  - ✅ `fill_row` matches a hand-computed reference
    (`fill_row_packs_correctly_simple`).

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
