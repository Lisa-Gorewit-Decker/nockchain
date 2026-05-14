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
| 8a | BLAKE3 round-AIR primitives — `round_ops::{add3, add2, xor_shift, xor_packed}` | ✅ landed | 15 | 236 unit |
| 8b | BLAKE3 round-AIR composition — `Blake3State`, `half_g`, `verify_round`, `finalize_blake`, `verify_init_state` | ✅ landed | 8 | 244 unit |
| 8c | BLAKE3 trace generator + top-level chip eval — `Blake3Chip` | ✅ landed | 10 | 254 unit |
| 9 | matmul cumsum chip (`MatmulCumsumChip`); RAM-lookup deferred to Phase 11 | ✅ landed | 20 | 274 unit |
| 10 | jackpot chip (`JackpotChip`; 16-slot rotate-XOR-13) | ✅ landed | 22 | 296 unit |
| 11 | `composite_lookups` — design + multiplicity calculus (proving-side wiring deferred to Phase 14) | ✅ landed | 10 | 306 unit |
| 12a | `composite_full_air::eval` — Phase 3-6 chips wired (stark_row, range_tables, i8u8, control, input) | ✅ landed | 9 | 315 unit |
| 12b | `composite_full_air` — matmul wired via `eval_composite` (BLAKE3, jackpot pending) | ✅ landed | 2 | 317 unit |
| 12c | `composite_full_air` — BLAKE3 wired via `eval_composite` (jackpot pending) | ✅ landed | 1 | 318 unit |
| 12d | `composite_full_air` — wire jackpot | ⬜ pending | | |
| 13a | `composite_trace` baseline builder + type surface | ✅ landed | 7 | 325 unit |
| 13b | `composite_trace` instruction-list compilation (matmul / blake3 / jackpot blocks) | ⬜ pending | | |
| 14a | `composite_proof::{composite_prove, composite_verify}` wrappers + bincode round-trip | ✅ landed | 3 | 328 unit |
| 14b | LogUp-aware folder swap (proving-side interaction wiring) | ⬜ pending | | |
| 15 | PROD bench at MIN_STARK_LEN baseline (ignored) | ✅ landed | 1 ignored | 328 unit + 1 ignored |

**Today's cumulative test count: 328 unit + 7 KAT + 4 ignored
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

### Phase 8a — BLAKE3 round-AIR primitives (landed)

Constraint primitives from `pearl/.../blake3_air.rs` ported as
standalone helpers. Each independently testable.

  - ✅ `add3_unchecked(res, a, b, c)` enforces `res ∈ {a+b+c,
    a+b+c−2^32, a+b+c−2^33}` (cubic constraint, degree 3).
    Tests: clean sum accepts, both wrap modes accept, off-by-one
    rejects, unrelated value rejects.
  - ✅ `add2_unchecked(res, a, b)` enforces `res ∈ {a+b,
    a+b−2^32}` (quadratic, degree 2). Tests: clean sum / wrap /
    wrong sum.
  - ✅ `xor_32_shift_if(res, a, b_bits, is_activated, shift)` —
    `res = a XOR (b <<< shift)` with `b` as 32 boolean bits.
    All 4 G-function rotation amounts tested
    individually (0, 7, 8, 12, 16) against hand-computed
    `b.rotate_left(shift)` references. Non-boolean bit rejected.
    Wrong result rejected.
  - ✅ `xor_32_packed(a_bits, b_bits)` — direct 32-bit XOR via
    bit decomposition, no shift, no gating. Returns the packed
    u32 expression for use in the finalisation row.

Together these primitives are sufficient to implement Pearl's
`half_g` (4 G-function half-steps per round, 4 rounds per BLAKE3
hash). Phase 8b composes them into `verify_round` /
`finalize_blake` / `verify_init_state`.

### Phase 8b — BLAKE3 round-AIR composition (landed)

`round_air.rs` composes the Phase 8a primitives into the full
BLAKE3 round AIR. Five entry points:

  - ✅ `Blake3State<'a, V>` — Pearl-equivalent 16-word state
    layout: 4 packed u32s (row1) + 4×32 bits (row2) + 4 packed
    u32s (row3) + 4×32 bits (row4) = 264 cells.
    `from_slice` routes a contiguous trace slice into the right
    buckets (validated by `blake3_state_from_slice_pins_layout`).
  - ✅ `half_g(a, b, c, d, m, flag, expected_*, is_activated)` —
    one BLAKE3 G-function half-step composing
    `add3_unchecked`, `xor_32_shift_if`, `add2_unchecked`, and a
    second `xor_32_shift_if`. `flag = false` ⇒ rotations (16,
    12); `flag = true` ⇒ rotations (8, 7).
  - ✅ `verify_round(states[0..5], msg, is_activated)` — full
    round: 16 `half_g` calls split across column-G half 1,
    column-G half 2, diagonal-G half 1, diagonal-G half 2
    (matching Pearl's `blake3_air.rs:75-147`).
  - ✅ `finalize_blake(states, is_activated)` — round-8
    feed-forward XOR, with Pearl's "abuse" trick reusing
    `states[1].row2` / `row4` as bit decompositions of
    `states[0].row1` / `row3`.
  - ✅ `verify_init_state(init, is_new_blake, cv, blake3_tweak)`
    — initial state matches `(cv[0..4], cv[4..8], IV[0..4],
    tweak)` and zeros all unused tweak-bit padding cells.

Properties validated:
  - ✅ `round_with_snapshots` produces 4 distinct intermediate
    states equivalent to a single BLAKE3 round
    (`round_with_snapshots_produces_4_snapshots`).
  - ✅ A 4-row trace of valid rounds verifies end-to-end
    (`prove_and_verify_valid_round`).
  - ✅ Tampering an intermediate-state row1 cell rejects
    (`verify_rejects_tampered_state1_row1`).
  - ✅ Tampering an intermediate-state row3 cell rejects
    (`verify_rejects_tampered_state2_row3`).
  - ✅ Tampering a message word rejects
    (`verify_rejects_tampered_message`).
  - ✅ Non-boolean bit columns rejected
    (`verify_rejects_non_boolean_bit_in_state2_row2`).
  - ✅ Two distinct (state, message) inputs across rows verify
    independently (`prove_and_verify_two_different_rounds`).
  - ✅ Layout: `Blake3State::from_slice` correctly routes 264
    sentinel cells into row1/row2/row3/row4 buckets
    (`blake3_state_from_slice_pins_layout`).

### Phase 8c — BLAKE3 trace generator + top-level chip eval (landed)

`chips/blake3/chip.rs` ties Phase 8a + 8b together into a complete
chip:

  - ✅ `Blake3Chip` — ZST AIR implementing `Air<AB>` over Pearl's
    8-row-per-hash layout. Per-row dispatch driven by two boolean
    selector columns: `is_new_blake` (row 0 of each hash) and
    `is_last_round` (row 7 = finalize). Booleanity asserted
    unconditionally.
  - ✅ Cross-row round: `verify_round` runs inside
    `builder.when_transition()` (skips the absolute last trace
    row) AND gated by `is_round_active = 1 − is_last_round`
    (skips per-hash finalize rows). The gating extends down into
    `add3_unchecked` / `add2_unchecked` (these now take an
    `is_activated` parameter — the **gating fix from the
    multi-hash test**) so the cubic / quadratic add constraints
    silence cleanly at hash boundaries.
  - ✅ `verify_init_state` fires on row 0 of each hash, tying
    `STATE0` to `(cv_in, IV, packed_tweak)`.
  - ✅ `finalize_blake` fires on row 7 of each hash, computing
    feed-forward XOR and asserting `CV_OUT[i] == out[i]`.
  - ✅ `Blake3Chip::fill_one_hash` builds all 8 rows from a single
    (msg, cv_in, tweak) tuple via `round_with_snapshots`.
  - ✅ `pack_tweak` packs `(counter_low, counter_high[0..16],
    flags[0..8], block_len[0..7])` into the 63-bit form
    `verify_init_state` expects.

Properties validated:
  - ✅ `fill_one_hash` writes the correct selector pattern across
    8 rows (`fill_one_hash_writes_full_rows`).
  - ✅ `CV_OUT` matches `compress_full_state`'s output for
    arbitrary inputs (`cv_out_matches_compress_full_state`).
  - ✅ End-to-end prove + verify of one 8-row BLAKE3 trace
    (`prove_and_verify_one_hash`).
  - ✅ End-to-end prove + verify of two 8-row hashes in a 16-row
    trace, validating the boundary gating at the row 7 → row 8
    transition (`prove_and_verify_two_hashes`).
  - ✅ Tamper detection: wrong CV_OUT
    (`verify_rejects_wrong_cv_out`), wrong initial CV cell
    (`verify_rejects_wrong_initial_cv_row1_cell`), wrong
    intermediate state (`verify_rejects_wrong_intermediate_state`),
    non-boolean selector
    (`verify_rejects_non_boolean_is_new_blake`).
  - ✅ `pack_tweak` bit layout pinned at u64 positions [0:32],
    [32:48], [48:56], [56:63] (`pack_tweak_round_trips`,
    `pack_tweak_zero_returns_zero`).

**Constraint degree increase:** with `add3_unchecked` now gated by
`is_activated`, its max degree rose from 3 to 4. Within
`CircuitConfig::TEST_PEARL`'s `log_blowup = 2` budget (max
constraint degree ≤ 5 by Plonky3's quotient formula). Pearl's
own circuit stays at degree 3 by leveraging a stricter row
schedule; we trade that off for cleaner chip-internal logic. If
Phase 12's composite AIR needs the degree-3 ceiling back, we can
factor each cubic into two quadratics via an intermediate column.

### Phase 9 — matmul cumsum chip (landed)

`chips/matmul/` ports Pearl's tile-accumulator update. Two
submodules:

**`compute.rs`** — scalar reference for the matmul row update.
Properties validated:
  - ✅ `tile_dot(a, b)` computes `sum_d(a[d] * b[d])` over
    `TILE_D = 16` i8 elements. Tested on simple ramps, signed
    cancellation, zero operands, and extreme `[127, 127]` cases
    (`tile_dot_simple`, `tile_dot_signs`, `tile_dot_zero_when_either_zero`,
    `tile_dot_extreme_values`).
  - ✅ `tile_dot_block(a, b)` returns the full `TILE_H × TILE_H`
    block in row-major order (`tile_dot_block_indexing`).
  - ✅ `apply_cumsum_update` implements Pearl's reset / update /
    pass-through semantics exactly
    (`apply_cumsum_reset_overrides`, `apply_cumsum_update_accumulates`,
    `apply_cumsum_passthrough_when_both_off`).
  - ✅ End-to-end `compute_row` chains reset → update producing
    `2 × dot` (`compute_row_end_to_end_reset_then_update`).
  - ✅ `CUMSUM_LEN = 4 = TILE_H²` pinned
    (`cumsum_len_matches_tile_h_squared`).

**`chip.rs`** — AIR + trace generator. The constraint is a single
per-row equality applied to each of the 4 cumsum cells:
```
  next.CUMSUM[k] = (is_reset + is_update) · dot[k]
                 + (1 − is_reset) · cur.CUMSUM[k]
```
gated by `builder.when_transition()` (so the wraparound from the
last row doesn't fire). Booleanity checks on both selectors
unconditional. Constraint degree 3.

Properties validated:
  - ✅ 4-step (reset, update, update, update) chain verifies
    end-to-end (`prove_and_verify_4_step_chain`).
  - ✅ Pass-through row (both selectors 0) preserves CUMSUM
    (`prove_and_verify_passthrough_row`).
  - ✅ Extreme i8 values `[−128, 127]` produce correct cumsum
    chain (`prove_and_verify_extreme_values`).
  - ✅ Tamper detection: cumsum cell
    (`verify_rejects_tampered_cumsum`), A_UNPACK cell
    (`verify_rejects_tampered_a_cell`), B_UNPACK cell
    (`verify_rejects_tampered_b_cell`), non-boolean is_reset
    (`verify_rejects_non_boolean_is_reset`), non-boolean is_update
    (`verify_rejects_non_boolean_is_update`).
  - ✅ Trace padded to next power-of-2 row count
    (`build_trace_pads_to_power_of_two`).
  - ✅ Chip width pinned at 71 cols (`chip_width_pinned`).

**Deferred to Phase 11:** the `A_NOISED ↔ A_NOISED_UNPACK` (and B)
packing-consistency constraint and the `NOISED_PACKED` RAM-lookup
read (LogUp on `MAT_ID = A_ID`). The packing constraint can land
inside this chip once we know the polyval base Pearl uses for i8
packing; the RAM lookup requires the composite-level LogUp wiring
that's the focus of Phase 11.

### Phase 10 — jackpot chip (landed)

`chips/jackpot/` ports Pearl's 16-slot tile-state rotate-XOR-13
update from `chip/jackpot/jackpot_air.rs`. The single-slot
primitive has been validated since M9.1 by `state_chip.rs`; this
phase wraps it with one-hot slot-routing so it can update any of
16 state slots per row.

**`compute.rs`** — scalar reference. Pinned at `LROT_PER_TILE = 13`
and `JACKPOT_SIZE = 16`. Validated:
  - ✅ `rotate_xor_13(0, 0) = 0` (`rotate_xor_13_zero_zero_is_zero`).
  - ✅ `rotate_xor_13(0, x) = x` (`rotate_xor_13_zero_x_is_x`).
  - ✅ `rotate_xor_13` matches `v.rotate_left(13) ^ x`
    (`rotate_xor_13_matches_definition`).
  - ✅ Avalanche on `rotate_xor_13`: 1-bit input change ⇒ 1-bit
    output change (`rotate_xor_13_avalanche`).
  - ✅ `apply_jackpot_step` only touches the selected slot
    (`apply_jackpot_step_only_touches_selected_slot`).
  - ✅ `apply_jackpot_step(is_active = false)` is a no-op
    (`apply_jackpot_step_inactive_preserves_state`).
  - ✅ `one_hot_select(i)` returns the i-th unit vector
    (`one_hot_select_returns_unit_vector`).
  - ✅ `bit_decompose_u32` round-trips
    (`bit_decompose_round_trips`).
  - ✅ Multi-step chain is deterministic (regression anchor).

**`chip.rs`** — AIR over a chip-local 97-col layout. Constraints:
  1. **Booleanity** on SLOT_SEL, V_BITS, X_BITS, IS_ACTIVE (every
     bit cell satisfies `b(1−b) = 0`).
  2. **One-hot SLOT_SEL sum** = IS_ACTIVE — exactly one slot
     selected when active; all zero when inactive.
  3. **V_BITS = bit_decompose(JACKPOT_MSG[selected])** — encoded as
     `Σ_i SLOT_SEL[i]·JACKPOT_MSG[i] = polyval(V_BITS, 2)`, gated
     by IS_ACTIVE. Degree 2.
  4. **Cross-row rotate-XOR-13**: for each slot `i`,
     `next.JACKPOT_MSG[i] = SLOT_SEL[i]·polyval(rot13(V_BITS) XOR
     X_BITS, 2) + (1 − SLOT_SEL[i])·cur.JACKPOT_MSG[i]`. Gated by
     `when_transition()`. Degree 3.

Properties validated:
  - ✅ 4-step active chain verifies end-to-end
    (`prove_and_verify_4_step_chain`).
  - ✅ Pass-through rows (IS_ACTIVE = 0) leave state unchanged
    (`prove_and_verify_passthrough_row`).
  - ✅ Tamper: JACKPOT_MSG cell
    (`verify_rejects_tampered_jackpot_msg`), V_BITS bit
    (`verify_rejects_wrong_v_bits`), X_BITS bit
    (`verify_rejects_tampered_x_bits`), non-boolean SLOT_SEL
    (`verify_rejects_non_boolean_slot_sel`).
  - ✅ Two slots simultaneously selected rejected
    (`verify_rejects_multiple_slots_selected`).
  - ✅ Active row without selection (IS_ACTIVE = 1 but
    SLOT_SEL all-zero) rejected
    (`verify_rejects_active_without_selection`).
  - ✅ Missing rotation rejected (`verify_rejects_unrotated_value`).
  - ✅ 32-row "rotate every slot once" stress test
    (`prove_and_verify_full_slot_pass`).
  - ✅ Chip width pinned at 97 cols (`chip_width_pinned`).

### Phase 11 — composite_lookups design (landed)

`composite_lookups.rs` pins the lookup-bus architecture and the
multiplicity calculus. The proving-side wiring (switching from
`p3-uni-stark` to a lookup-aware folder via `p3-lookup`'s
`InteractionBuilder`) is deferred to Phase 14 because it requires
swapping out the prover stack, which is a single contained
refactor downstream.

What this phase delivers:

  - ✅ Bus inventory: 8 named buses (`urange8`, `urange13`,
    `irange7p1`, `irange8`, `i8u8`, `noised_packed`, `cv_routing`,
    `stark_row_idx`). Each documents its table chip, queriers, and
    cryptographic role.
  - ✅ Multiplicity helpers: `noised_packed_freq`,
    `cv_out_freq`, `blake3_cv_query_count`,
    `matmul_noised_packed_query_count`,
    `blake3_msg_mat_query_count` — used by Phase 13's trace
    generator to fill `MAT_FREQ` / `CV_OUT_FREQ` etc.
  - ✅ Bus names pinned as `&'static str` constants
    (`bus_name_strings_match_documentation`).
  - ✅ Balance-simulation tests: a 2-hash CV_OUT → CV_IN scenario
    and a multi-querier `noised_packed` scenario both produce
    table-side multiplicity equal to total query count
    (`cv_routing_multi_hash_balance_simulation`,
    `noised_packed_multi_querier_balance`).
  - ✅ All 8 bus names pairwise unique
    (`all_buses_are_pairwise_unique`).
  - ✅ ALL_BUSES count == 8 (`all_buses_count_matches_design`).

**Why scope was reduced:** `p3-lookup` doesn't ship a drop-in
`prove`/`verify` wrapper around `p3-uni-stark`; it provides the
`InteractionBuilder` trait and the `ProverConstraintFolderWithLookups`
folder, both of which need a custom prover. The cleanest place
to land that switch is Phase 14, when the composite trace
generator and the prover plumbing all change together. Phase 11's
design-level deliverable here is what every downstream phase
needs to agree on.

### Phase 12a — composite_full_air (Phase 3-6 chips wired) — landed

`composite_full_air.rs` introduces the top-level
`CompositeFullAir` over `TOTAL_TRACE_WIDTH = 1328` columns. This
slice wires 8 of the chip-side constraint generators:

  * [`StarkRowChip`](crate::chips::stark_row::StarkRowChip)
  * [`URange8Chip`](crate::chips::range_table::URange8Chip)
  * [`URange13Chip`](crate::chips::range_table::URange13Chip)
  * [`IRange7P1Chip`](crate::chips::range_table::IRange7P1Chip)
  * [`IRange8Chip`](crate::chips::range_table::IRange8Chip)
  * [`I8U8Chip`](crate::chips::i8u8::I8U8Chip)
  * [`ControlChip`](crate::chips::control::ControlChip)
  * [`InputChip`](crate::chips::input::InputChip)

These all read columns by `composite_layout::*` offsets directly,
so they slot in via `Chip::default().eval(builder)` calls without
column-projection wiring.

Properties validated:

  * ✅ Baseline trace at `MIN_STARK_LEN = 8192` rows × 1328 cols
    verifies (`composite_full_air_baseline_trace_verifies`).
  * ✅ Range tables, I8U8, STARK_ROW_IDX all filled by their
    `fill_row` helpers; remaining ~1300 columns are zero
    (degenerate but constraint-satisfying for the wired chips).
  * ✅ Tamper detection: STARK_ROW_IDX
    (`composite_full_air_rejects_bad_row_idx`), range table
    (`composite_full_air_rejects_bad_range_table`), I8U8 AUX
    (`composite_full_air_rejects_bad_i8u8_aux`), inconsistent
    CONTROL_PREP selector bit
    (`composite_full_air_rejects_inconsistent_control_prep`),
    inconsistent NOISED_PACKED
    (`composite_full_air_rejects_inconsistent_noised_packed`).
  * ✅ Air width matches `TOTAL_TRACE_WIDTH`
    (`composite_full_air_width_matches_total_trace_width`).
  * ✅ `MIN_STARK_LEN` anchor — Pearl's pinned minimum trace
    length passes (`composite_full_air_min_stark_len_anchor`).
  * ✅ `I8U8_TABLE_SIZE` pinned at 256 (`i8u8_table_size_pinned`).

### Phase 12b — matmul wired into composite_full_air (landed)

Refactor `MatmulCumsumChip` to expose:

  - `MatmulOffsets` struct bundling A_UNPACK / B_UNPACK / CUMSUM /
    selector column offsets.
  - `MatmulCumsumChip::LOCAL_OFFSETS` (chip-local) and
    `COMPOSITE_OFFSETS` (mapped to `composite_layout::*` constants).
  - `MatmulCumsumChip::eval_at(builder, &offsets)` — the shared
    constraint generator parameterized over offsets.
  - `MatmulCumsumChip::eval_composite(builder)` — called from
    `CompositeFullAir::eval`.

`CompositeFullAir::eval` now also calls
`MatmulCumsumChip::eval_composite(builder)`. The existing
`Air<AB>::eval` impl delegates to `eval_at(builder, &LOCAL_OFFSETS)`,
so chip-local tests are unchanged.

Properties validated (cumsum in composite-trace context):
  - ✅ Tampering CUMSUM_TILE on row 1 (with selectors all 0)
    rejects, because the gated update collapses to `next.CUMSUM = cur.CUMSUM`
    (`composite_full_air_rejects_changed_cumsum_without_selectors`).
  - ✅ Changing A_NOISED_UNPACK on row 1 in passthrough mode
    (both selectors 0) STILL verifies, since the dot product term
    is multiplied by `(0 + 0) = 0`. Confirms gating actually
    silences (`composite_full_air_accepts_changed_a_unpack_in_passthrough`).

### Phase 12c — BLAKE3 wired into composite_full_air (landed)

Refactor `Blake3Chip` analogously to `MatmulCumsumChip`:

  - `Blake3Offsets` bundles state-snapshot block start + msg + cv +
    tweak + cv_out + 2 selector columns.
  - `LOCAL_OFFSETS` (chip-local cols) and `COMPOSITE_OFFSETS`
    (`composite_layout::BLAKE3_ROUND_START` + `BLAKE3_MSG_START` +
    `BLAKE3_CV_START` + `CV_OR_TWEAK_PREP` + `CV_OUT_START` +
    `IS_NEW_BLAKE` + `IS_LAST_ROUND`).
  - `eval_at(builder, &offsets)` — shared constraint generator.
  - `eval_composite(builder)` — convenience wrapper.

`CompositeFullAir::eval` now also calls
`Blake3Chip::eval_composite(builder)`. The existing chip-local
tests are unchanged.

CV mapping decision: read CV from `BLAKE3_CV_START` (the value
"ready for BLAKE3" on this row) rather than `CV_IN_START` (which
is the value pulled in from a previous hash via LogUp). When the
LogUp wiring lands in Phase 14, `BLAKE3_CV` will be constrained
equal to `CV_IN` on rows that consume an external CV.

Properties validated:
  - ✅ Baseline trace (all selectors zero) still verifies — all
    BLAKE3 dispatch silences cleanly (`composite_full_air_baseline_trace_verifies`).
  - ✅ Non-boolean BLAKE3 state bit rejects regardless of
    selectors — booleanity in `xor_32_shift_if` fires
    unconditionally (`composite_full_air_rejects_non_boolean_blake3_state_bit`).

### Phase 12d — composite_full_air (jackpot) — pending

Jackpot wiring is held by a column-shape mismatch:

  * Chip-local layout has `V_BITS[32]`, `X_BITS[32]`, `SLOT_SEL[16]`,
    `IS_ACTIVE[1]`.
  * Composite layout has `BIT_REG[32]` (one 32-bit bit-decomp slot)
    and `JACKPOT_IDX[8]` (8 cols, one-hot store/load indicators —
    NOT 16-slot selector).

Two options:
  1. **Reshape the chip** to use BIT_REG + JACKPOT_IDX's contract
     (compact, more like Pearl).
  2. **Extend composite_layout** to accommodate the chip-local
     16-slot select + dedicated X_BITS.

Option 1 is cleaner and matches Pearl. Defer to Phase 12d.

### Phase 13a — composite_trace baseline (landed)

`composite_trace.rs` provides the **type surface** for trace
generation + a minimal baseline-zero builder.

  * `CompositeTrace` — wraps a `RowMajorMatrix<Val>` of size
    `TOTAL_TRACE_WIDTH × N` ready for proving.
  * `CompositeTrace::baseline(n)` — fills 4 range tables, I8U8
    table, STARK_ROW_IDX monotonic; all other columns zero.
    Panics if `n` is not a power of 2 or below `MIN_STARK_LEN`.
  * `CompositeTrace::baseline_min()` — convenience: exactly
    `MIN_STARK_LEN = 8192` rows.

The result verifies end-to-end through `CompositeFullAir`. This
is the foundation every higher-level builder extends.

Properties validated:
  * ✅ Shape: `width = TOTAL_TRACE_WIDTH`, `height = n`
    (`baseline_trace_has_correct_shape`).
  * ✅ `baseline_min` height = `MIN_STARK_LEN`
    (`baseline_min_matches_min_stark_len`).
  * ✅ Baseline verifies through `CompositeFullAir`
    (`baseline_trace_verifies_through_composite_full_air`).
  * ✅ 2× MIN_STARK_LEN also verifies
    (`baseline_larger_than_min_also_verifies`).
  * ✅ STARK_ROW_IDX is `0, 1, ..., n-1` exactly
    (`baseline_stark_row_idx_is_monotonic`).
  * ✅ Panics below MIN_STARK_LEN
    (`baseline_panics_below_min_stark_len`).
  * ✅ Panics for non-power-of-two row counts
    (`baseline_panics_for_non_power_of_two`).

### Phase 13b — composite_trace instruction-list (pending)

Phase 13b would extend `CompositeTrace` with an `Instr` enum
(MatmulStep, Blake3Hash, JackpotStep, Padding) and a compiler
that places each instruction into a contiguous block of trace
rows with CONTROL_PREP / preprocessed columns filled
consistently. Pearl's `pearl_program.rs` + `pearl_trace.rs` do
this in ~700 lines combined.

Instruction-list compilation is naturally bundled with Phase 14
(lookup-aware prover) since the instruction blocks determine
each row's lookup multiplicities (CV routing, NOISED_PACKED
queries, etc.).

### Phase 14a — composite prove/verify wrappers (landed)

`composite_proof.rs` exposes lib-level prove/verify wrappers
around the composite stack:

  * `composite_proof::build_config(params, profile)` —
    re-export of `circuit::build_stark_config` for ergonomics.
  * `composite_proof::composite_prove(&config, trace)` —
    consumes a `CompositeTrace` (Phase 13a) and produces a
    `Proof<AiPowStarkConfig>`.
  * `composite_proof::composite_verify(&config, &proof)` —
    returns `Result<(), CompositeVerificationError>`.
  * `CompositeVerificationError` alias for the concrete
    `VerificationError<PcsError<AiPowStarkConfig>>` type.

Properties validated:
  * ✅ Prove + verify round-trip on the baseline trace
    (`composite_prove_verify_round_trip`).
  * ✅ Bincode serialization round-trip: prove → bincode-encode →
    bincode-decode → verify
    (`composite_proof_is_serializable`).
  * ✅ Same config covers proofs at multiple trace sizes
    (MIN_STARK_LEN and 2× MIN_STARK_LEN)
    (`composite_proofs_at_two_trace_sizes`).

### Phase 14b — LogUp-aware prover (pending)

Phase 14b swaps `p3-uni-stark`'s folder for one that implements
`p3-lookup::InteractionBuilder`, turning the Phase 11 lookup
design into reified constraints. This is the cryptographically
critical wiring that ties chips together via LogUp; without it
the composite proof can't enforce e.g. that `A_NOISED` reads
correspond to actual `NOISED_PACKED` entries.

Practical path: pull `p3-lookup` into `Cargo.toml`, build a
custom prover that uses `ProverConstraintFolderWithLookups` and
`VerifierConstraintFolderWithLookups`, add interaction-emission
to each chip (via the bus helpers in `composite_lookups`).

### Phase 15 — PROD bench at baseline shape (landed)

`composite_proof_prod_bench` (in `composite_proof::tests`) runs
`composite_prove` + `composite_verify` under
[`CircuitConfig::PROD`] (`log_blowup = 3`, `num_queries = 80` —
120 bits of provable FRI soundness) at the baseline trace shape
(`MIN_STARK_LEN = 8192` rows × `TOTAL_TRACE_WIDTH` cols).
`#[ignore]` so the regular `cargo test` doesn't pay the prove
cost. Run with:

```sh
cargo test -p ai-pow-zk --release --lib composite_proof_prod_bench -- --ignored --nocapture
```

Measured one-shot (Apple Silicon, release build):
  * prove   : 43.3 s
  * verify  : 119 ms
  * proof   : ~683 KB (uncompressed)

These numbers are the **structural ceiling** for the baseline
trace (no chip activity). Real proofs with matmul / BLAKE3
activity will scale up because the constraint polynomials become
non-trivial. Proof size will also drop dramatically once
recursion compression lands (deferred to M12 per the original
M10.1c design — Plonky3 doesn't ship a compressor yet).

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
| 2026-05-14 | M10.1c Phase 7 BLAKE3 chip foundation (compress + layout + logic) | `37cdb06` |
| 2026-05-14 | M10.1c Phase 8a BLAKE3 round-AIR primitives (`round_ops`) | `bc546b0` |
| 2026-05-14 | M10.1c Phase 8b BLAKE3 round-AIR composition (`round_air`) | `f233d0b` |
| 2026-05-14 | M10.1c Phase 8c BLAKE3 top-level chip (`chip.rs`) | `105699b` |
| 2026-05-14 | M10.1c Phase 9 matmul cumsum chip (`chips/matmul`) | `d07b16a` |
| 2026-05-14 | M10.1c Phase 10 jackpot chip (`chips/jackpot`) | `5e08fa1` |
| 2026-05-14 | M10.1c Phase 11 lookup design (`composite_lookups`) | `b492465` |
| 2026-05-14 | M10.1c Phase 12a `composite_full_air` (Phase 3-6 chips) | `253a938` |
| 2026-05-14 | M10.1c Phase 12b matmul wired via `eval_composite` | `c883c21` |
| 2026-05-14 | M10.1c Phase 12c BLAKE3 wired via `eval_composite` | `17f161d` |
| 2026-05-14 | M10.1c Phase 13a `composite_trace` baseline builder | `6945714` |
| 2026-05-14 | M10.1c Phase 14a `composite_proof` prove/verify wrappers | `fbbbc18` |
| 2026-05-14 | M10.1c Phase 15 PROD bench at MIN_STARK_LEN baseline | (this commit) |
