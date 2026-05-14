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
| 5 | `control_chip` (Pearl `control_and_matid_packed`) | ✅ landed | 11 | 194 unit |
| 6 | `composite_preprocess` minimal generator | ✅ landed | 6 | 200 unit |
| 7 | BLAKE3 chip — `compress` + `layout` + `logic` (Pearl scalar + per-round column layout + per-row logic types) | ✅ landed | 21 | 221 unit |
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

### Phase 5 — control chip (landed)

Properties validated:
  - ✅ All 21 selectors are boolean; non-boolean rejected
    (`rejects_non_boolean_selector`).
  - ✅ `CONTROL_PREP = polyval(selectors..., mat_id; base=2)` —
    mis-matched packing rejects
    (`rejects_wrong_control_prep_pack`).
  - ✅ `MAT_ID = limb0 + limb1 << 13` — mismatch rejects
    (`rejects_mat_id_inconsistent_with_limbs`).
  - ✅ Cross-consistency: changing a selector column without
    updating CONTROL_PREP rejects
    (`rejects_selector_without_control_prep_update`).
  - ✅ All-zero, all-one, mixed selector patterns + MAT_ID verify
    (`prove_and_verify_*`).
  - ✅ `SELECTOR_COLS` indices are pairwise unique
    (`selector_columns_are_unique`).
  - ✅ Pack utility matches expected bit layout
    (`pack_round_trips_zeros`, `pack_sets_correct_bits`).

### Phase 6 — composite_preprocess (landed)

Properties validated:
  - ✅ `RowDescriptor::padding()` is all-zero (default for padding
    rows in the trace).
  - ✅ `fill_preprocessed_row` writes correct values into all 5
    preprocessed columns (CONTROL_PREP, NOISE_PACKED_PREP,
    CV_OR_TWEAK_PREP, AB_ID_PREP, STARK_ROW_IDX) from a known
    descriptor.
  - ✅ CONTROL_PREP packing matches the control chip's
    `pack_control_prep` contract byte-for-byte (prover and
    verifier agree).
  - ✅ Batch generator `build_preprocessed_columns` agrees with
    per-row generator on every row.
  - ✅ STARK_ROW_IDX monotonic across the table.
  - ✅ MAT_ID limb decomposition matches BITS_PER_LIMB = 13.

### Phase 7 — BLAKE3 chip foundation (landed)

Three sub-modules under `chips/blake3/`, each Pearl-mirrored:

**`compress.rs`** — Pearl's scalar BLAKE3 compression. Provides the
reference computation Phase 8's AIR will prove correct.
Properties validated:
  - ✅ `BLAKE3_MSG_PERMUTATION` is a bijection over `0..16`
    (`iv_and_permutation_pinned`).
  - ✅ `blake3_permute_msg` matches the constant (Pearl's own
    self-test) — `blake3_permute_msg_matches_constant`.
  - ✅ `BLAKE3_IV`, `BLAKE3_MSG_LEN`, default `Blake3Tweak` values
    pinned (`iv_and_permutation_pinned`, `default_tweak`).
  - ✅ **Cross-check vs M10.1b vendored chip**: same byte output
    for the same `(message, key, counter, block_len, flags)`
    (`matches_m10_1b_vendored_chip`). This is the merge-mining
    anchor — both implementations compute identical leaves.
  - ✅ **Cross-check vs `blake3` crate**: same byte output as
    `blake3::Hasher::new_keyed(...).update(...).finalize()` for
    the single-block keyed-root case (`matches_blake3_crate_keyed`,
    `all_zero_input_matches_blake3_crate`).
  - ✅ Avalanche check: differing inputs produce differing outputs
    (`different_inputs_different_outputs`).
  - ✅ `compress_full_state` and `blake3_compress` agree on the
    first 8 words (the truncated 32-byte hash output).
  - ✅ G function is deterministic and produces zeros on zero input
    (regression anchors).

**`layout.rs`** — per-round column sub-layout inside Pearl's
1056-column `BLAKE3_ROUND` block. 4 state snapshots × 264 limbs
each = 1056. Mirrors `pearl/.../blake3_layout.rs` widths verbatim.
Properties validated:
  - ✅ Per-snapshot limbs = 264 (`per_snapshot_limbs_are_264`).
  - ✅ Total limbs = `BLAKE3_ROUND_LEN` = 1056
    (`total_limbs_matches_blake3_round_len`).
  - ✅ STATE3 ends at `BLAKE3_ROUND_START + BLAKE3_ROUND_LEN`
    (`state3_end_matches_blake3_round_end`).
  - ✅ Snapshot offsets are contiguous — no overlap, no gap
    (`snapshot_offsets_are_contiguous`).
  - ✅ Row widths match Pearl exactly (4, 128, 4, 128 —
    `pearl_row_widths_match`).

**`logic.rs`** — per-row instruction descriptor (`MessageDataType`,
`AuxKind`, `BlakeRoundLogic`). Mirrors Pearl's `logic.rs` 1:1.
Properties validated:
  - ✅ Default logic uses JOB_KEY as the CV source.
  - ✅ Setting `cv_is_commitment = true` switches CV source to
    COMMITMENT_HASH.
  - ✅ Subtle case: PreviousCv data source with routing index still
    uses JOB_KEY (the previous CV is loaded as *message*, not as
    *chaining value*).
  - ✅ CV routing without previous-CV data source switches off
    JOB_KEY (the row pulls CV from another row via the LogUp).
  - ✅ Default `round_idx = 1` (most-permissive option per Pearl).
  - ✅ Default `MessageDataType::None`.

Next: Phase 8 (trace.rs + constraints.rs + program.rs +
chip.rs from Pearl). This is the **AIR side** — the constraint
logic proving each row's state evolution. Pearl's
`constraints.rs` is ~200 lines, `trace.rs` ~343, `program.rs`
~386, `blake3_air.rs` ~356 = ~1300 lines combined. Substantial
follow-on work.

### Phase 7+ — scope decision (resolved)

User picked **option 1** (full Pearl one-round-per-row port).
The scalar foundation (`compress`, `layout`, `logic` — Pearl's
non-AIR machinery) lands in this iteration with cross-checks
against the M10.1b vendored chip + the `blake3` crate. Phase 8
ports the AIR side (`trace.rs`, `constraints.rs`, `program.rs`,
`blake3_air.rs`) — ~1300 more lines.

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
| 2026-05-14 | M10.1c Phase 3 `stark_row_chip` | `152a6f3` |
| 2026-05-14 | M10.1c Phase 4a `range_table` (URange8/13, IRange7P1/8) | `2c6e56b` |
| 2026-05-14 | M10.1c Phase 4b+4c `i8u8` + `input` chips | `2b2ec0a` |
| 2026-05-14 | M10.1c Phase 5 `control_chip` (CONTROL_PREP + MAT_ID) | `cb49931` |
| 2026-05-14 | M10.1c Phase 6 `composite_preprocess` minimal | `e221113` |
| 2026-05-14 | M10.1c Phase 7 BLAKE3 chip foundation (compress + layout + logic) | (this commit) |
