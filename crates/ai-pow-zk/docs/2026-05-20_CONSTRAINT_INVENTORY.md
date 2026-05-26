> _Created **2026-05-20** · last updated **2026-05-20**._

# Master Constraint Inventory — S0 of the Constraint Soundness Analysis

> **Status (R1, honest).** S0 LANDED. The data foundation for
> S1–S7 of `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`.
> Every constraint family in the M-S5 chain + ai-pow-zk
> production AIR is enumerated with: AIR / chip / file path,
> max polynomial degree, English description, soundness-claim
> cross-link, existing tamper test (✅) or GAP flag (⚠️ + G1/G2/G3
> category). The LogUp / global-bus catalogue is included.
>
> **Source code revisions consulted.** ai-pow-zk @
> `claude/ai-pow-nockchain-sgfNX` HEAD (commit `6385287`,
> 2026-05-20); Plonky3-recursion vendored @ `c2c51fb`
> (rev-aligned to upstream `524665d`); Tip5 paper IACR
> ePrint 2023/107 (`2023-107.pdf` in repo root); FRI
> soundness paper IACR ePrint 2025/2055 (`2025-2055.pdf` —
> already consumed by `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`).
>
> **What this inventory deliberately does and does not do.**
> Does: enumerate every constraint family + cross-link tests.
> Does not: derive per-constraint soundness bits (S1 does
> that), design new tampers (S3), or implement new tests
> (S4). Soundness derivation columns are **placeholders for
> S1**, not committed numbers.

---

## 1. How to read this inventory

### 1.1 Constraint family row template

Each row is `(degree, name, file:line, role, claim, tamper, mech)`:
- **degree**: max polynomial degree (in trace variables) of
  the constraint expression. Used by S1 for the per-constraint
  Schwartz–Zippel bound `d / q_chal`.
- **name**: short English label.
- **file:line**: pointer to the `builder.assert_*` /
  `LogUpGadget::*` / `*Bus::*` call in code.
- **role**: 1-line English description of what the constraint
  enforces.
- **claim**: cross-link to the soundness-claim namespace of
  `2026-05-19_C4_AUDIT_READINESS.md` § 3 (CRIT-1 / HIGH-2.2 §X /
  M-S1 / M52 / A3.x / C2.X / C3 / DT-4 / MED-3 / P-B.2.x / P-A
  / B1.x / ENV).
- **tamper**: existing tamper test pointer
  `<test_name>@<file>:<line>` if present, or `⚠️ GAP-<G>` if
  missing.
- **mech**: predicted rejection mechanism (M1–M5 per CSA
  design §4.1):
  - **M1** = AIR `eval()` constraint violation ⇒
    `verify` returns `Err` (low-degree-extension fail).
  - **M2** = LogUp bus imbalance ⇒ `verify` returns `Err`.
  - **M3** = Preprocessed-commit / CRIT-1 / VK pin mismatch ⇒
    `verify` returns `Err`.
  - **M4** = CTL / NPO `WitnessConflict` at `runner().run()`.
  - **M5** = Merkle-path / commitment fail (ai-pow side).

### 1.2 GAP categorization

- **G1** = constraint covered by another tamper (subsumption);
  no new test needed. The doc will note the subsuming test.
- **G2** = existing tamper exists but is unlabelled / unnamed
  for this constraint (rename-only needed, no new code).
- **G3** = no tamper exists; S3 must design + S4 implement.

### 1.3 Field stack

| Field | Identifier | Size | Used for |
|---|---|---|---|
| `F` (base) | `Goldilocks` (p3-goldilocks) | `2^64 − 2^32 + 1 ≈ 2^64` | Trace cells; AIR constraints |
| `F_ext` (challenge / OOD) | `BinomialExtensionField<Goldilocks, 2>` | `q_chal ≈ 2^128` | Verifier challenges; FRI extension |
| `q_chal` | — | `≈ 2^128` | Schwartz–Zippel + LogUp soundness denominator |

For Theorem 1.5 (S(−1)) the relevant field is also `q_chal`
(per `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` §2.1). All AIR
+ bus constraints checked at random points in `F_ext` with
`q_chal ≈ 2^128`.

---

## 2. ai-pow-zk production AIR — inner Tip5 Layer-0 STARK

The production AIR is the M10.1c composite — wired in
`crates/ai-pow-zk/src/composite_full_air.rs` (Route-A path)
+ `composite_full_air_with_lookups.rs` (with-LogUp-buses
path; the production path). 12 PROGRAM_COLS pinned via
CRIT-1; 6 LogUp buses; 4 composite keystones (CRIT-1 pin,
HIGH-2.2 §4.D, HIGH-2.2 §6(b)-G2, M-S1 pack-link).

### 2.1 StarkRowChip — `crates/ai-pow-zk/src/chips/stark_row.rs`

**Trace columns:** 1 (`STARK_ROW_IDX`).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 1 | First-row zero | `stark_row.rs:54` | `STARK_ROW_IDX[0] == 0` | DT-4 (row-counter monotonicity) | ✅ `verify_rejects_nonzero_first_row@stark_row.rs:177` | M1 |
| 2 | 2 | Transition increment | `stark_row.rs:57-59` | `STARK_ROW_IDX[i+1] == STARK_ROW_IDX[i] + 1` | DT-4 (monotone counter) | ✅ `verify_rejects_broken_increment@stark_row.rs:196`; ✅ `verify_rejects_skipped_index@stark_row.rs:213`; ✅ `verify_rejects_late_tamper@stark_row.rs:245`; ✅ `composite_full_air_rejects_bad_row_idx@composite_full_air.rs:718` | M1 |

**LogUp consumers/producers:** STARK_ROW_IDX is consumed by
the CV routing bus (BLAKE3 CV_OUT → CV_IN) and used as the
position-address for the §4.C.2 cx.0 position-exact noise
binding.

**Coverage:** ✅ full (4 tamper tests covering first-row,
increment, skip, late-tamper).

### 2.2 RangeTableChip × 4 — `crates/ai-pow-zk/src/chips/range_table.rs`

Four instantiations: URange8Chip (0..=255), URange13Chip
(0..=8191), IRange7P1Chip (-64..=64), IRange8Chip (-128..=127).

**Trace columns per chip:** 1.
**Constraint families (per chip, ×4):**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 1 | First-row min | `range_table.rs:78` | `TABLE[0] == min` | DT-1 range table | ✅ `urange8_verify_rejects_wrong_first_row@range_table.rs:219` (+ irange8 variant `:335`) | M1 |
| 2 | 1 | Last-row max | `range_table.rs:80` | `TABLE[N-1] == max` | DT-1 range table | ✅ `urange8_verify_rejects_wrong_last_row@range_table.rs:231` | M1 |
| 3 | 2 | Monotonic step | `range_table.rs:82-87` | `next − cur ∈ {0, 1}` via `δ·(δ-1) = 0` | DT-1 range table | ✅ `urange8_verify_rejects_non_boolean_delta@range_table.rs:249`; ✅ `irange8_verify_rejects_non_boolean_delta@range_table.rs:335`; ✅ `composite_full_air_rejects_bad_range_table@composite_full_air.rs:736` | M1 |

**LogUp consumers (one per chip — see §2.13):** `BUS_URANGE8`,
`BUS_URANGE13`, `BUS_IRANGE7P1`, `BUS_IRANGE8`.

**Coverage:** ✅ full for URange8 + IRange8; ⚠️ GAP-G2 for
URange13 + IRange7P1 (no explicit table-side rejection tests
named for these chips — but they share the parameterized
range-table test infrastructure, and the LogUp-side coverage
of each bus covers indirect rejection).

### 2.3 I8U8Chip — `crates/ai-pow-zk/src/chips/i8u8.rs`

**Trace columns:** 2 (`I8U8_TABLE`, `I8U8_AUX`).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 1 | First-row AUX zero | `i8u8.rs:~115` | `AUX[0] == 0` | i8u8 contract | ✅ `rejects_aux_first_row_nonzero@i8u8.rs:262` | M1 |
| 2 | 1 | Last-row AUX one | `i8u8.rs:~120` | `AUX[N-1] == 1` | i8u8 contract | ✅ `rejects_aux_last_row_zero@i8u8.rs:273` | M1 |
| 3 | 2 | AUX delta binary | `i8u8.rs:~130` | `aux_delta · (aux_delta − 1) = 0` | i8u8 boolean | ✅ `rejects_non_boolean_aux@i8u8.rs:308` | M1 |
| 4 | 2 | AUX delta match | `i8u8.rs:~135` | `aux_delta · (TABLE − boundary_const) = 0` | i8u8 transition | ✅ `rejects_aux_transition_off_boundary@i8u8.rs:323`; ✅ `rejects_aux_non_monotonic@i8u8.rs:344` | M1 |
| 5 | 1 | First-row table | `i8u8.rs:~?` | `TABLE[0] == pack_first` | i8u8 contract | ✅ `rejects_wrong_first_pack@i8u8.rs:285` | M1 |
| 6 | 1 | Last-row table | `i8u8.rs:~?` | `TABLE[N-1] == pack_last` | i8u8 contract | ✅ `rejects_wrong_last_pack@i8u8.rs:296` | M1 |
| 7 | 2 | Delta delta | `i8u8.rs:~?` | `delta_delta · (total_delta − 257) = 0` | i8u8 range | ✅ `rejects_wrong_intermediate_pack@i8u8.rs:360`; ✅ `composite_full_air_rejects_bad_i8u8_aux@composite_full_air.rs:754` | M1 |

**LogUp producer:** `BUS_I8U8` — i8/u8 conversion pairs (§2.13).

**Coverage:** ✅ full (9 tamper tests covering all 7 families
+ composite-level validation).

### 2.4 ControlChip (CRIT-1 substrate) — `crates/ai-pow-zk/src/chips/control.rs`

**Trace columns:** 25 (`CONTROL_PREP`, 21 `IS_*` selectors,
`MAT_ID`, 2 `MAT_ID_LIMBS`).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | Selector boolean × 21 | `control.rs:137-139` | `s · (1 − s) = 0` per selector | CRIT-1 (selector-shape pin) | ✅ `rejects_non_boolean_selector@control.rs:440`; ✅ `crit1_zeroed_selector_forgery_rejected@composite_proof.rs:892` | M1 + M3 |
| 2 | 2 | MAT_ID reconstruction | `control.rs:141-148` | `MAT_ID == MAT_ID_LIMBS[0] + MAT_ID_LIMBS[1] << 13` | CRIT-1 (MAT_ID pin) | ✅ `rejects_mat_id_inconsistent_with_limbs@control.rs:467` | M1 |
| 3 | 2 | CONTROL_PREP polyval | `control.rs:155-237` | `CONTROL_PREP == polyval([21 selectors, MAT_ID, FOLD_IS_FOLD, FOLD_SLOT_SEL, FOLD_STRIPE_SEL, MSG_PAIR_SEL], base=2)` | CRIT-1 (program pin) | ✅ `rejects_wrong_control_prep_pack@control.rs:453`; ✅ `composite_full_air_rejects_inconsistent_control_prep@composite_full_air.rs:773` | M1 + M3 |
| 4 | 2 | FOLD_IS_FOLD boolean | `control.rs:189-190` | `fold_is_fold · (1 − fold_is_fold) = 0` | CRIT-1 + HIGH-2.2 §6 | ✅ `rejects_selector_without_control_prep_update@control.rs:482` | M1 |
| 5 | 2 | FOLD_SLOT_SEL sum | `control.rs:194-199` | `Σ FOLD_SLOT_SEL == FOLD_IS_FOLD` (one-hot 4-bit slot) | HIGH-2.2 §6 (slot pin) | ✅ `fold_slot_mismatch_rejected@control.rs:617`; ✅ `fold_is_fold_without_control_prep_rejected@control.rs:594` | M1 |
| 6 | 2 | FOLD_STRIPE_SEL sum | `control.rs:210-215` | `Σ FOLD_STRIPE_SEL == FOLD_IS_FOLD` (one-hot 6-bit stripe) | HIGH-2.2 §6(b)-G2 (stripe pin) | ✅ `fold_stripe_mismatch_rejected@control.rs:635`; ✅ `control_prep_claims_fold_but_column_zero_rejected@control.rs:689` | M1 |
| 7 | 2 | MSG_PAIR_SEL sum | `control.rs:230-235` | `Σ MSG_PAIR_SEL == g` where `g = IS_MSG_MAT · IS_NEW_BLAKE` | §4.C.2 cx.1 (message-pair pin) | ✅ `msg_pair_mismatch_rejected@control.rs:665` | M1 |

**PROGRAM_COLS pinned (CRIT-1):** `CONTROL_PREP` (bit 0–60;
all selectors, MAT_ID, fold/stripe/pair schedules).

**Coverage:** ✅ full (10 tests across control.rs + composite_proof.rs
+ composite_full_air.rs).

### 2.5 InputChip (M-S1 / A3) — `crates/ai-pow-zk/src/chips/input.rs`

**Trace columns:** 17 (`NOISE_PACKED_PREP × 8`, `NOISED_PACKED × 8`,
plus unpacks).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | NOISE_PACKED_PREP repacking | `input.rs:~80-100` | `NOISE_PACKED_PREP[s] == polyval(NOISE_UNPACK[s*4:(s+1)*4], 129)` (×8 sub-slices) | A3.3 noise-tie | ✅ `rejects_wrong_noise_packed_prep@composite_trace.rs:295` | M1 |
| 2 | 2 | NOISED_PACKED polyval | `input.rs:~100-120` | `NOISED_PACKED[i] == polyval(A/B_MAT[4i:4i+4], 256) + polyval(NOISE_UNPACK[4i:4i+4], 129)` (×8) | M-S1 / A3.3 pack-link | ✅ `rejects_wrong_noised_packed@composite_trace.rs:279`; ✅ `rejects_tampered_mat_byte@composite_trace.rs:311`; ✅ `composite_full_air_rejects_inconsistent_noised_packed@composite_full_air.rs:793`; ✅ `a3_2a_positioned_store_layout_is_witness_free_and_consistent@composite_proof.rs:1115` | M1 |

**LogUp producer:** `BUS_NOISED_PACKED` — emits
`polyval(MAT_UNPACK, 256) + polyval(NOISE_UNPACK, 129)` for
each cell (§2.13).

**PROGRAM_COLS pinned:** `NOISE_PACKED_PREP × 8` (bits 1-8
in the PROGRAM_COLS vector).

**Coverage:** ✅ full (4 tamper tests across composite_trace.rs
+ composite_full_air.rs + composite_proof.rs).

### 2.6 MatmulCumsumChip (HIGH-2.2 §4.A) — `crates/ai-pow-zk/src/chips/matmul/chip.rs`

**Trace columns:** 71 per row (`A_NOISED_UNPACK × 32`,
`B_NOISED_UNPACK × 32`, `CUMSUM_TILE × 4`,
`IS_RESET_CUMSUM`, `IS_UPDATE_CUMSUM`, gating).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | A/B_NOISED polyval | `composite_full_air.rs:410-448` | `A_NOISED[c] == polyval(A_NOISED_UNPACK[4c:4c+4], 256)` (×8 A + ×8 B) | M-S1 pack-link (HIGH-2.2 §4.A) | ✅ `matmul_pack_link_rejects_inconsistent_a_noised@composite_trace.rs:2803` | M1 |
| 2 | 2 | CUMSUM transition | `matmul/chip.rs:176` | On `IS_UPDATE_CUMSUM=1`, `nxt.CUMSUM_TILE == CUMSUM_TILE + dot(A_UNPACK, B_UNPACK)` | HIGH-2.2 §4.A (matmul integrity) | ✅ `verify_rejects_tampered_cumsum@matmul/chip.rs:396`; ✅ `matmul_step_chain_rejects_tampered_input@composite_trace.rs:2766`; ✅ `composite_full_air_rejects_changed_cumsum_without_selectors@composite_full_air.rs:844` | M1 |
| 3 | 7 | Matmul dot product | `matmul/chip.rs:~?` | Inner-product of A/B unpack lanes (4 `i8 × i8` summed; degree ≤ 7 with extension-field multiplication) | HIGH-2.2 §4.A | ✅ `verify_rejects_tampered_a_cell@matmul/chip.rs:411`; ✅ `verify_rejects_tampered_b_cell@matmul/chip.rs:426`; ✅ `verify_rejects_non_boolean_is_reset@matmul/chip.rs:439`; ✅ `verify_rejects_non_boolean_is_update@matmul/chip.rs:452`; ✅ `high2_2_swept_tile_not_in_store_rejects@composite_proof.rs:785` | M1 |

**LogUp consumer:** `BUS_IRANGE8` (A/B unpack lanes must be
i8 — see §2.13).

**Coverage:** ✅ full (8 tests covering the 3 families +
sweep-tile-not-in-store cross-chip composition).

### 2.7 FoldChip (HIGH-2.2 §4.B) — `crates/ai-pow-zk/src/chips/fold.rs`

**Trace columns:** 30 (`FOLD_STATE × 16`, `FOLD_XSTEP`,
`FOLD_STRIPE_SEL × 6`, `FOLD_IS_FOLD`, `XOR_OUT`, intermediates).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | FOLD_STRIPE_SEL sum == FOLD_IS_FOLD | `fold.rs:145` | One-hot property | HIGH-2.2 §6(b)-G2 | ✅ `rejects_double_slot_selection@fold.rs:521`; ✅ via control.rs §2.4 #6 | M1 |
| 2 | 3 | Fold state XOR-reduction | `fold.rs:150-180` | On fold rows, `FOLD_STATE[i] == (x[i] ⊕ rotl13_xor_fold_state)` per 16 words | HIGH-2.2 §4.B (Pearl §4.5 rotl13-XOR fold) | ✅ `rejects_tampered_fold_state@fold.rs:479`; ✅ `rejects_tampered_xstep@fold.rs:494`; ✅ `high2_2_fold_chain_pinned_logup@composite_proof.rs:726` | M1 |
| 3 | 2 | Fold transition | `fold.rs:190-210` | Between rows: passthrough or recomputed | HIGH-2.2 §4.B | ✅ `rejects_passthrough_violation_on_padding@fold.rs:536` | M1 |
| 4 | 2 | First/last-row zero | `fold.rs:170-175` | `FOLD_STATE[0] == 0` on first non-fold rows | HIGH-2.2 §4.B | ✅ `rejects_nonzero_first_row_state@fold.rs:509` | M1 |

**Coverage:** ✅ full (5 tests).

### 2.8 StripeXorChip (HIGH-2.2 §6(b)-G2) — `crates/ai-pow-zk/src/chips/stripe_xor.rs`

**Trace columns:** 70 (`SX_IN × 8`, `SX_XR × 8`, gating).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | SX_IN == nxt.CUMSUM_TILE | `stripe_xor.rs:~160` | On stripe-xor rows, `SX_IN[stripe] == (next row's CUMSUM_TILE)` | HIGH-2.2 §6(b) (SX←matmul) | ✅ `rejects_tampered_register@stripe_xor.rs:570` | M1 |
| 2 | 3 | SX_XOR reduction | `stripe_xor.rs:~180` | `SX_XR[stripe] == XOR(SX_IN[stripe], SX_IN[prev])` | HIGH-2.2 §6(b) | ✅ `rejects_double_lane_selection@stripe_xor.rs:610`; ✅ `rejects_tampered_new_sel_without_bits@stripe_xor.rs:597`; ✅ `rejects_out_of_range_q_bit@stripe_xor.rs:624` | M1 |
| 3 | 2 | SX carry-forward | `stripe_xor.rs:~200` | Passthrough or recomputed | HIGH-2.2 §6(b) | ✅ `rejects_nonzero_first_row_register@stripe_xor.rs:585`; ✅ `rejects_lane_passthrough_violation@stripe_xor.rs:636` | M1 |

**Coverage:** ✅ full (6 tests).

### 2.9 XStepChip — `crates/ai-pow-zk/src/chips/xstep.rs`

**Trace columns:** ~30.
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | XSTEP address decode | `xstep.rs:~?` | `XSTEP` decodes to address bits correctly | HIGH-2.2 §6(b) | ✅ `rejects_tampered_xstep@xstep.rs:320` | M1 |
| 2 | 2 | Accumulator | `xstep.rs:~?` | Address accumulator transitions | HIGH-2.2 §6(b) | ✅ `rejects_tampered_acc_cell_without_bits@xstep.rs:332` | M1 |
| 3 | 2 | Bit-flip with quotient | `xstep.rs:~?` | Quotient invariant on bit flip | HIGH-2.2 §6(b) | ✅ `rejects_flipped_xstep_bit_with_unbounded_q_attempt@xstep.rs:345` | M1 |
| 4 | 2 | Q-bit range | `xstep.rs:~?` | Quotient bit ∈ {0, 1} | HIGH-2.2 §6(b) | ✅ `rejects_out_of_range_q_bit@xstep.rs:363` | M1 |

**Coverage:** ✅ full (4 tests).

### 2.10 Blake3Chip (BLAKE3 round AIR) — `crates/ai-pow-zk/src/chips/blake3/`

**Modules:** `round_air.rs`, `round_ops.rs`, `chip.rs`, `layout.rs`.
**Trace columns:** ~200 (4 state snapshots × 7 rounds × 16
state words + CV + message + tweaks).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 7 | BLAKE3 round function | `blake3/round_air.rs:~300-600` | Native-faithful 7-round permutation per Pearl spec | Pearl BLAKE3 binding (§3.2) | ✅ `verify_rejects_tampered_state1_row1@round_air.rs:642`; ✅ `verify_rejects_tampered_state2_row3@round_air.rs:656`; ✅ `verify_rejects_tampered_message@round_air.rs:672`; ✅ `verify_rejects_non_boolean_bit_in_state2_row2@round_air.rs:687`; ✅ `composite_full_air_rejects_non_boolean_blake3_state_bit@composite_full_air.rs:867` | M1 |
| 2 | 2 | CV packing | `blake3/round_air.rs:392-408` | `CV_OUT[i] == polyval(state_snapshot[4i:4i+4], 2^32)` | M-S1 / BLAKE3 chaining | ✅ via state1/state2 tamper tests + `blake3_hash_block_rejects_tampered_cv_out@composite_trace.rs:2571`; ✅ `verify_rejects_wrong_cv_out@blake3/chip.rs:615` | M1 |
| 3 | 2 | Message packed | `blake3/round_air.rs:250` | `BLAKE3_MSG` matches queried matrix word-pair (gated by `IS_MSG_MAT`) | §4.C.2 cx.0 / §4.D (HIGH-2.2 §4.D keystone) | ✅ `c3_rejects_is_msg_mat_row_with_mismatched_blake_msg@composite_trace.rs:2998`; ✅ `full_air_rejects_tampered_hash_a_pi@composite_trace.rs:3033` | M1 |
| 4 | 2 | CV routing (initial / intermediate) | `blake3/round_air.rs` + `blake3/chip.rs:615-646` | Initial CV setup; intermediate state passthrough | BLAKE3 chain | ✅ `verify_rejects_wrong_initial_cv_row1_cell@blake3/chip.rs:630`; ✅ `verify_rejects_wrong_intermediate_state@blake3/chip.rs:646`; ✅ `verify_rejects_non_boolean_is_new_blake@blake3/chip.rs:662` | M1 |
| 5 | 2 | Low-level round ops (ADD2/3, XOR-shift) | `blake3/round_ops.rs:293-497` | Sub-operations of the round function | Pearl BLAKE3 | ✅ `add3_unchecked_rejects_off_by_one@round_ops.rs:293`; ✅ `add3_unchecked_rejects_unrelated_value@round_ops.rs:303`; ✅ `add2_unchecked_rejects_wrong_sum@round_ops.rs:368`; ✅ `xor_32_shift_if_rejects_wrong_result@round_ops.rs:480`; ✅ `xor_32_shift_if_rejects_non_boolean_bit@round_ops.rs:497` | M1 |

**LogUp consumer / producer:** `BUS_CV_ROUTING` (CV_OUT producer
on hash-end rows; CV_IN consumer via STARK_ROW_IDX address);
`BUS_URANGE8` (UINT8_DATA[0] consumer when `IS_MSG_MAT=1`).

**Coverage:** ✅ full (13 tamper tests across round_air, round_ops,
chip, composite_trace, composite_full_air).

### 2.11 JackpotChip (HIGH-2.2 §4.D) — `crates/ai-pow-zk/src/chips/jackpot/chip.rs`

**Trace columns:** 97 (`JACKPOT_MSG × 16`, `JACKPOT_X_BITS × 32`,
`JACKPOT_SLOT_SEL × 16`, `BIT_REG`, V_BITS).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | SLOT_SEL sum == IS_HASH_JACKPOT | `jackpot/chip.rs:147` | One-hot property | HIGH-2.2 §4.D | ✅ `verify_rejects_non_boolean_slot_sel@jackpot/chip.rs:456`; ✅ `verify_rejects_multiple_slots_selected@jackpot/chip.rs:469`; ✅ `verify_rejects_active_without_selection@jackpot/chip.rs:485` | M1 |
| 2 | 2 | Slot value reconstruction | `jackpot/chip.rs:159` | `JACKPOT_MSG[selected_slot] == polyval(bit decomp, 2)` | HIGH-2.2 §4.D | ✅ `verify_rejects_tampered_jackpot_msg@jackpot/chip.rs:422`; ✅ `verify_rejects_wrong_v_bits@jackpot/chip.rs:437`; ✅ `high2_2_jackpot_nonzero_msg_unit@composite_proof.rs:460`; ✅ `jackpot_step_chain_rejects_tampered_msg@composite_trace.rs:2629` | M1 |
| 3 | 2 | Bit transition (BIT_REG) | `jackpot/chip.rs:~180-210` | Carry-forward or slot transition | HIGH-2.2 §4.D | ✅ `verify_rejects_tampered_x_bits@jackpot/chip.rs:500`; ✅ `verify_rejects_unrotated_value@jackpot/chip.rs:516` | M1 |

**LogUp consumer:** `BUS_JACKPOT_X_BITS` (deferred — bit
decomp ↔ CUMSUM_BUFFER lookup; current POC ties to internal
state).

**Coverage:** ✅ full (10 tests).

### 2.12 Composite keystones (cross-chip pins)

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| K1 | 1 | **CRIT-1 PROGRAM_COL pin** (×12) | `composite_full_air.rs:238-253` | `main[PROGRAM_COLS[k]] == preprocessed[k]` for k=0..11 (CONTROL_PREP + NOISE_PACKED_PREP × 8 + CV_OR_TWEAK_PREP + AB_ID_PREP + STARK_ROW_IDX) | CRIT-1 / Phase A-CR | ✅ `crit1_honest_pinned_roundtrip@composite_proof.rs:864`; ✅ `crit1_zeroed_selector_forgery_rejected@composite_proof.rs:892`; ✅ `crit1_tampered_program_col_rejected@composite_proof.rs:932`; ✅ `crit1_forged_hash_jackpot_with_canonical_program_rejected@composite_proof.rs:967`; ✅ `routea_crit1_zeroed_selector_forgery_rejected@composite_proof.rs:1061`; ✅ `routea_crit1_tampered_program_col_rejected@composite_proof.rs:1088`; ✅ `cr6_verify_uses_canonical_not_prover_program_rejects_forge@ai-pow/src/zk_bridge.rs:2974` | M3 |
| K2 | 1 | **HIGH-2.2 §4.D keystone** (`JACKPOT_MSG[i] == FOLD_STATE[i]`) | `composite_full_air.rs:255-289` | On last row, unconditional 16× equality binding JACKPOT_MSG to FOLD_STATE | HIGH-2.2 §4.D | ✅ `high2_2_jackpot_nonzero_msg_unit@composite_proof.rs:460`; ✅ `high2_free_jackpot_message_rejected@composite_proof.rs:989`; ✅ `routea_high2_free_jackpot_message_rejected@composite_proof.rs:1116` | M1 |
| K3 | 2 | **HIGH-2.2 §6(b)-G2 keystone** (`FOLD_XSTEP == SX_XR[stripe]`) | `composite_full_air.rs:318-334` | `Σ_s FOLD_STRIPE_SEL[s] · (FOLD_XSTEP − SX_XR[s]) = 0` (production: `sx_bound=true`, `num_stripes=64`) | HIGH-2.2 §6(b)-G2 | ✅ `high2_2_fold_chain_pinned_logup@composite_proof.rs:726` (implicit via fold-state pin) | M1 |
| K4 | 2 | **M-S1 pack-link** (`A_NOISED == polyval(A_UNPACK, 256)`, ×16 A+B) | `composite_full_air.rs:410-448` | Packed/unpack equivalence | M-S1 (HIGH-2.2 §4.C) | ✅ `matmul_pack_link_rejects_inconsistent_a_noised@composite_trace.rs:2803`; ✅ `high2_2_swept_tile_not_in_store_rejects@composite_proof.rs:785` (cross-chip composition) | M1 + M2 |

**Coverage:** ✅ full for K1 (7 tests), K2 (3 tests), K4 (2 tests);
⚠️ GAP-G2 for K3 (no test explicitly named for §6(b)-G2; covered
implicitly by `high2_2_fold_chain_pinned_logup` — relabel-only).

### 2.13 LogUp buses (6 buses + 2 deferred)

Sourced from `crates/ai-pow-zk/src/composite_full_air_with_lookups.rs::bus_emit::*`.

| Bus | Producer | Consumer | Binding | Claim | Existing tamper coverage |
|---|---|---|---|---|---|
| **BUS_URANGE8** | `URANGE8_TABLE` row | `UINT8_DATA[0]` query (when `IS_MSG_MAT=1`) | u8 ∈ [0, 256) | DT-1 (range) | ✅ `out_of_range_uint8_with_active_query_rejected_by_logup@composite_full_air_with_lookups.rs:633`; ✅ `over_claimed_urange8_freq_rejected_by_logup@:658`; ✅ `under_claimed_urange8_freq_rejected_by_logup@:675` |
| **BUS_URANGE13** | `URANGE13_TABLE` row | MAT_ID limbs, NOISE_UNPACK | limb ∈ [0, 8192) | DT-1 (limb width) | ✅ `out_of_range_mat_id_limb_rejected_by_logup@:724`; ✅ `out_of_range_noise_unpack_rejected_by_logup@:741`; ✅ `tampered_urange13_freq_rejected_by_logup@:1246` |
| **BUS_IRANGE7P1** | `IRANGE7P1_TABLE` row | `NOISE_UNPACK` cells | noise ∈ [-64, 64] | A3.3 noise-tie | ✅ via out_of_range_noise_unpack tests (above) |
| **BUS_IRANGE8** | `IRANGE8_TABLE` row | `A_NOISED_UNPACK`, `B_NOISED_UNPACK` | i8 ∈ [-128, 128) | HIGH-2.2 §4.A range | ✅ `out_of_range_a_noised_unpack_rejected_by_logup@:758`; ✅ `out_of_range_b_noised_unpack_negative_rejected_by_logup@:777`; ✅ `tampered_a_noised_with_no_matching_table_entry_rejects@:948`; ✅ `tampered_mat_freq_rejected_by_logup@:976`; ✅ `prop_a_noised_unpack_outofrange_rejects@:1306` |
| **BUS_I8U8** | `I8U8_TABLE` | Negative cumsum queries | i8 ↔ u8 | HIGH-2.2 §4.D (negative-value conversion) | ✅ `valid_negative_i8u8_pair_balances`; ✅ `inconsistent_i8u8_pair_rejected_by_logup@:875`; ✅ `tampered_i8u8_freq_rejected_by_logup@:894`; ✅ `prop_inconsistent_i8u8_pair_rejects@:1341` |
| **BUS_NOISED_PACKED** | `NOISED_PACKED` cell | `NOISE_PACKED_PREP + polyval(A_MAT_UNPACK + NOISE_UNPACK, ·)` | InputChip polyval | A3.3 noise-tie | ✅ `out_of_range_mat_unpack_rejected_by_logup@:794` (related); + A3 noise tests |
| **BUS_CV_ROUTING** | `CV_OUT` on hash-end rows (gated by `IS_HASH_*`) | `CV_IN` reads via STARK_ROW_IDX | BLAKE3 CV chaining | BLAKE3 chain | ✅ `cv_routing_dangling_reference_rejected@:1075`; ✅ `cv_routing_wrong_cv_value_rejected@:1101`; ✅ `tampered_cv_out_freq_rejected_by_logup@:1230`; ✅ `prop_cv_routing_nonzero_cv_rejects@:1398` |
| **BUS_MATMUL_INPUT** (deferred) | `A_NOISED` / `B_NOISED` packed cells | Canonical multiset store of (A, B) pairs | M-S1 producer store | M-S1 / matmul-input pin | ⚠️ **GAP-G3** — C2/C3 follow-on per `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`; M12-deferred per `#127` |
| **BUS_JACKPOT_X_BITS** (POC) | Bit decomposition | Lookup table (deferred) | A-TILE / MED-3 binding | HIGH-2.2 §4.D | ⚠️ **GAP-G3** — deferred per the chip docstring |

**Bus-side LogUp soundness (per Habock):** Each bus's
constraint degree (post-L4 fix) is 2; soundness per challenge
is `k_b · 2 / q_chal` where `k_b` is the number of bus
interactions. With `k_b ≤ ~100` per bus and `q_chal ≈ 2^128`:
per-bus error `≤ 200 / 2^128 ≈ 2^(-120)`. **≥80 with ~40-bit
margin.**

**Coverage:** ✅ 6 wired buses fully covered (~30 tamper tests
total); ⚠️ 2 deferred buses (BUS_MATMUL_INPUT and BUS_JACKPOT_X_BITS)
documented as design residuals.

---

## 3. Tip5 circuit AIR (C2.1) — `Plonky3-recursion/tip5-circuit-air/`

The C2.1 soundness linchpin. Native-faithful 7-round Tip5
permutation AIR, KAT-anchored to `nockchain_math::tip5::permute`.

### 3.1 Tip5PermLookupAir (algebraic form, pre-L4) — `tip5-circuit-air/src/air.rs`

**Trace columns:** 9392 (16 base + 7 rounds × 1338 cols/round).
**Constraint families:**

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| 1 | 2 | Boolean bit decomposition (BBITS/CBITS/QBITS) | `air.rs:164-183` | `bit · (bit − 1) = 0` per byte decomposition | C2.0 byte range | ✅ `adversarial_tamper_rejected@air.rs:416` | M1 |
| 2 | 3 | Offset-Fermat-cube identity | `air.rs:185-191` | `(b+1)³ − 1 = 257·q + c` (machine-proved C2.0 identity ⇔ `c = LOOKUP_TABLE[b]`) | C2.0 / C2.1 | ✅ `adversarial_tamper_rejected@air.rs:416` (S-box output tamper) | M1 |
| 3 | 1 | Canonical 8-byte recomposition | `air.rs:193-203` | `recompose_b = Σ b_k · 2^(8k) = sbox_in_col(r, t)` | C2.0 byte canonical | ✅ `adversarial_noncanonical_split_rejected@air.rs:449` | M1 |
| 4 | 1 | Canonical-guard inverse-or-zero | `air.rs:207-212` | For split lanes, `H = 2^32 − 1 ⇒ L = 0` (§4.6 `<p` guard) | C2.0 / Tip5 §4.6 | ✅ `adversarial_noncanonical_split_rejected@air.rs:466` | M1 |
| 5 | 2 | S-box output A (recomposed) | `air.rs:204-205` | `A[t] = recomposed_c` (looked-up L-values) | C2.1 | ✅ via test #2 (sbox-out tamper) | M1 |
| 6 | 3 | Power-lane x² register | `air.rs:223` | `x2 = x · x` for lanes 4..16 | C2.1 (x⁷ staging) | ✅ via test #2 (A tamper, line 432) | M1 |
| 7 | 3 | Power-lane x³ register | `air.rs:224` | `x3 = x2 · x` | C2.1 (x⁷ staging) | ✅ via test #2 | M1 |
| 8 | 3 | Power-lane x⁷ output | `air.rs:226` | `A[j] = x3 · x3 · x` (lanes 4..16) | C2.1 (x⁷ S-box) | ✅ via test #2 | M1 |
| 9 | 1 | MDS matrix (circulant) | `air.rs:235-241` | `ROUT[r][i] = Σ_j MDS[i][j] · A[j] + RC[r][i]` | C2.1 (MDS linear layer; Tip5 §2.3) | ✅ via test #2 (ROUT tamper, line 426) | M1 |
| 10 | 1 | Round constants | `air.rs:240-241` | `RC[r][i]` derived from paper §2.4 constants | C2.1 (round constants; Tip5 §2.1) | ✅ implicit via fixture KAT | M1 |

**KAT anchor:** `crates/ai-pow-zk/tests/fixtures/tip5_golden_kat.txt`
(315 vectors from native `nockchain_math::tip5::permute`).
**Validation tests:** `native_equiv_kat@air.rs:379`,
`air_equals_native_spec_exhaustive_random@air.rs:495` (4096
random).

**Coverage:** ✅ full (~5 tamper tests + 4411 KAT/random vectors).

### 3.2 Tip5PermLookupAir (lookup-table form, post-L4) — `tip5-circuit-air/src/air_lookup.rs`

Same trace shape as 3.1 but with LogUp bus for the byte-cube
identity (degree-2 LogUp; replaces the L4 catastrophe of
degree-226 inlined identity).

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| L1 | 2 | Per-byte LogUp interaction (×224 + ×1 table_entry) | `air_lookup.rs:236-241` (lookup_key); `:244-248` (table_entry) | Each byte `(b, c)` queries the table via `tip5_l` global bus | C2.1 (L4 lookup form) | ✅ `lookup_air_adversarial@air_lookup.rs:537`; ✅ `global_bus_interactions_are_low_degree@:415` (degree-2 proof); ✅ `lookup_air_equals_native_spec@:482` (315 + 2048 random) | M1 + M2 |

**Reuses** all 10 algebraic constraints from 3.1.

**Max constraint degree:** 4 (returned by `Tip5PermLookupAir::max_constraint_degree`
@ `air_lookup.rs:182-190`).

**Coverage:** ✅ full.

### 3.3 Tip5 circuit AIR — C2.3 WitnessChecks CTL — `tip5-circuit-air/src/air_circuit.rs`

D-aware extension of 3.2; adds WitnessChecks CTL for D-padding
(C2.4-R-a).

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| W1 | 1 | WitnessChecks input-send (D-aware) | `air_circuit.rs:326-337` | Push `[idx, value, ZERO × (D−1)]` with mult `−(in_ctl · kind)` | C2.4 R-a (D=1 byte-identical) | ✅ via `tip5_layer0_recursion_*` tests (C2.4 integration) | M4 |
| W2 | 1 | WitnessChecks output-receive (D-aware) | `air_circuit.rs:346-357` | Push `[idx, value, ZERO × (D−1)]` with mult `+(out_ctl · kind)` | C2.4 R-a multiset balance | ✅ via `tip5_layer0_recursion_*` tests; ⚠️ **GAP-G3 (R-a tail residual)** — D=2 recompose-coeff producer multiplicity imbalance on Tip5 verifier circuit; tracked as M12 / `#127` | M4 |

**Coverage:** ✅ for D=1; ⚠️ **GAP-G3** for D=2 R-a tail (M12-deferred).

---

## 4. Plonky3-recursion verifier-circuit AIRs

Verifier-circuit AIRs that prove the L1 / L2 outer-cert. The
fenced linchpin is **byte-identical** vs `259cab2` (per C3
LANDED state); this milestone does not edit any of these
AIRs, only inventories their constraints and verifies tamper
coverage.

### 4.1 Poseidon2 perm AIR — `Plonky3-recursion/poseidon2-circuit-air/src/air.rs` (upstream Plonky3 wrapper)

**Max degree:** 7 (the x⁷ S-box).
**Constraint families:** ~10 (S-box, full-round / partial-round
linear layers, round-constant injection — upstream).

**Coverage:** Upstream Plonky3 tests + indirect via L1/L2
outer-cert (`c3_stage_*` tests). **No direct ai-pow-zk-side
tamper test catalogued** ⇒ ⚠️ **GAP-G2** (upstream tested, but
no in-tree label for the specific tampers). Action: at audit
time, route to upstream Plonky3 test inventory.

### 4.2 Poseidon1 perm AIR — `Plonky3-recursion/poseidon1-circuit-air/src/air.rs` (upstream)

Same shape as 4.1 but D=1-in-D>1 mirror.
**Coverage:** ⚠️ **GAP-G2** (upstream); same disposition.

### 4.3 Recompose AIR — `Plonky3-recursion/circuit-prover/src/air/recompose_air.rs`

**Trace columns:** D per lane (base-field coefficients).
**Constraint families:** ZERO local constraints — all
correctness via CTL bus.

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| R1 | 1 | Receive EF output | `recompose_air.rs:169-173` | Push `[output_idx, v_0, …, v_{D-1}]` with `out_mult` | C2.4 recompose | ✅ via outer-cert tampers | M4 |
| R2 | 1 | Receive per-coefficient (D > 1) | `recompose_air.rs:177-191` | When `coeff_lookups=true`, push D per-coeff `[coeff_i_idx, v_i, 0, …]` per coefficient | C2.4 R-a producer multiplicity | ⚠️ **GAP-G3 (R-a tail)** — D=2 Tip5 verifier has orphaned ±1 (single location, wid 11468); M12 / `#127` | M4 |

**Coverage:** ⚠️ R-a tail GAP-G3 (deferred to M12 per memory).

### 4.4 WitnessChecks CTL — `Plonky3-recursion/circuit-prover/src/batch_stark_prover/tip5.rs`

Mirrors §3.3 W1+W2.

**Coverage:** ✅ for D=1 byte-identical (commit `632cb8c`); ⚠️
**GAP-G3** for D=2 R-a tail.

### 4.5 FRI verifier circuit — `Plonky3-recursion/recursion/src/verifier/**`

**Max degree:** 7 (verifier-circuit composition).
**Constraint families:** ~20 (FRI fold round consistency,
query-path Merkle authentication, in-circuit challenger
duplexing, opening-set consistency).

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| F1 | ? | Query-schedule consistency | `recursion/src/verifier/fri.rs:~?` | Per-query schedule must match the prover's claimed schedule | C3 / FRI | ✅ `test_fri_verifier_rejects_per_query_schedule_mismatch@fri.rs:852` | M1 |
| F2 | ? | Zero-query proof reject | `recursion/src/verifier/fri.rs:~?` | A proof with zero queries must not pass | C3 / FRI | ✅ `test_fri_verifier_rejects_zero_query_proof@fri.rs:912` | M1 |
| F3..F20 | 7 (max) | FRI fold-round constraints | `recursion/src/verifier/fri.rs:~?` | In-circuit consistency of each FRI fold (recompose challenge, Merkle path, low-degree extension) | C3 / FRI | ✅ via `c3_stage_a/b/c_*` (recursion verifier proves FRI verifier in-circuit) | M1 + M4 |

**Coverage:** Partial — explicit F1+F2 tests; F3–F20 indirectly
via c3_stage. ⚠️ **GAP-G3** for fine-grained per-fold-round
constraint-level tamper tests (S5 cross-AIR work).

### 4.6 Batch-STARK verifier — `Plonky3-recursion/recursion/src/verifier/batch.rs`

**Constraint families:** ~10 (PIs binding, common-data layout,
opening-set composition).

| # | deg | name | file:line | role | claim | tamper | mech |
|---|---:|---|---|---|---|---|---|
| B1 | ? | Serialized row counts | `batch.rs:~?` | Row count metadata must match the actual proof | C3 / batch verifier | ✅ `verify_all_tables_rejects_tampered_serialized_row_counts@batch_stark_prover/tests.rs:1135` | M1 |
| B2 | ? | Lookup vector length | `batch.rs:~?` | Lookup vector length matches preprocessing | C3 / batch verifier | ✅ `test_batch_verifier_rejects_short_lookup_vector@preprocessing.rs:514`; ✅ `*_long_lookup_vector@:526` | M1 |
| B3 | ? | Preprocessed metadata layout | `batch.rs:~?` | Metadata layout invariants | C3 / batch verifier | ✅ `test_batch_verifier_rejects_short_preprocessed_metadata@:538`; ✅ `*_out_of_bounds_matrix_to_instance@:555`; ✅ `*_extra_local_permutation_coefficients@:665`; ✅ `*_extra_next_permutation_coefficients@:681` | M1 |
| B4 | ? | Validate-rejects (table packing, row count, NPO zero-lane) | `batch_stark_prover/tests.rs:1043-1120` | Library-level validators | C3 / batch verifier | ✅ `validate_rejects_zero_serialized_row_count@:1043`; ✅ `validate_rejects_invalid_serialized_table_packing@:1056`; ✅ `validate_rejects_zero_lane_npo_entry@:1120` | M1 |

**Coverage:** ✅ ~10 tests covering the major validators.

### 4.7 DT-4 duplex binding executor — `Plonky3-recursion/circuit/src/ops/tip5_perm/executor.rs`

Not a constraint family per se, but a soundness-load-bearing
executor edit. The pre-swap `bus_state` capture (commit
`14116b0`) closes a soundness gap in the Merkle-swap CTL
multiset.

**Coverage:** ✅ implicit via `c3_stage_a/b/c` tamper tests
(Merkle-swap tamper ⇒ `WitnessConflict` at `runner().run()`).
Pre-swap/post-swap differ by 1 if fix is missing.

---

## 5. ai-pow extraction layer (binds the SNARK input)

Not AIR but bind the SNARK's PI. Inventoried for completeness;
S0 does not extend tamper tests here (the 30+ existing tests
are comprehensive).

| Component | File | Existing tamper coverage |
|---|---|---|
| `commit::matrix_commitment` (M52 chunk-Merkle) | `crates/ai-pow/src/commit.rs:254-275` | ✅ `tampered_leaf_rejects@:254`; ✅ `tampered_path_rejects@:265`; ✅ `rejects_empty@:275` |
| `blake3_tree::open_strip` (P-B.2.0) | `crates/ai-pow/src/blake3_tree.rs:577` | ✅ `strip_opening_rejects_tampering` |
| `Block` adversarial (M52 + Merkle + bounds) | `crates/ai-pow/tests/adversarial.rs:22-244` | ✅ 17 tests (comm_m, params, H_A, H_B, found path, target, spot count, spot indices, etc.) |
| End-to-end | `crates/ai-pow/tests/end_to_end.rs:99-184` | ✅ 5 tests (block commit, nonce, params, shape, range) |
| Quant contract | `crates/ai-pow/src/quant.rs:268` | ✅ `b2.3_out_of_domain_operand_is_rejected` |
| Phase A-CR / F1 bridge | `crates/ai-pow/src/zk_bridge.rs:914, 2974, 2485` | ✅ `f1_bridge_rejects_tampered_target`; ✅ `cr6_verify_uses_canonical_not_prover_program_rejects_forge`; ✅ `sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects` |

**Coverage:** ✅ ~30 tests; comprehensive.

---

## 6. GAP summary

The gaps from §2–§5, categorized G1/G2/G3 per CSA design §4.4:

### 6.1 GAP-G1 — covered-by-subsumption (no new test needed)

(None identified at S0 — all enumerated constraints have at
least an indirect test signal.)

### 6.2 GAP-G2 — exists-but-unlabelled (rename only)

| Constraint family | Existing test | Action |
|---|---|---|
| URange13 / IRange7P1 chip-side `first/last/delta` | Parametric range-table tests in `range_table.rs` | Add explicit named variants `urange13_*` / `irange7p1_*` (S3) |
| K3 §6(b)-G2 keystone | `high2_2_fold_chain_pinned_logup` implicit | Add explicit `high2_2_g2_xstep_stripe_pin_rejects` (S3) |
| Poseidon2 perm AIR (Plonky3-recursion 4.1) | Upstream Plonky3 tests | Route to upstream inventory in S7 audit-readiness extension |
| Poseidon1 perm AIR (Plonky3-recursion 4.2) | Upstream Plonky3 tests | Route to upstream inventory in S7 |

### 6.3 GAP-G3 — missing (S3 designs + S4 implements)

| Constraint family | Disposition |
|---|---|
| BUS_MATMUL_INPUT (M-S1 matmul-input pin, ai-pow-zk §2.13) | **DEFERRED** to C2/C3 follow-on per `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`; M12 / `#127`. Not blocking C4 audit; carried as residual. |
| BUS_JACKPOT_X_BITS (HIGH-2.2 §4.D bit decomp lookup) | **DEFERRED** per chip docstring. Not blocking. |
| W2 D=2 WitnessChecks output-receive (recompose-coeff R-a tail) | **DEFERRED** to M12 / `#127`. Already tracked. |
| R2 D=2 per-coefficient receive (Plonky3-recursion recompose, §4.3) | Same as W2 — M12-deferred. |
| F3..F20 fine-grained per-FRI-fold-round constraint-level tampers | **S5 cross-AIR work** — design + implement individual per-fold-round tampers (currently covered indirectly via c3_stage). |
| h_a/h_b circuit-side zero-gap test (production 16∣r geometry) | Existing `sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects` covers c-exact; explicit h_a / h_b root binding at strip-opening leaf rows needs a dedicated tamper. **S4.B deliverable.** |

**Total GAP-G3 count (in-scope for S3+S4):** 1 (h_a/h_b
circuit-side dedicated tamper) + S5 cross-AIR per-FRI-fold-round
work.

**Total GAP-G3 count (deferred / out-of-scope):** 3 (M12-tracked).

**Total GAP-G2 count (rename-only):** 4.

---

## 7. Constraint-count and degree rollup

| Layer | AIRs | Constraint families | LogUp buses | Max degree | Existing tamper tests |
|---|---:|---:|---:|---:|---:|
| ai-pow-zk production AIR (§2) | 11 chips + 4 keystones | ~80 | 6 (+2 deferred) | 7 (BLAKE3 round) | ~120 |
| Tip5 circuit AIR (§3) | 3 (algebraic + lookup + circuit) | ~12 | 1 (tip5_l) | 4 | ~10 (incl. 4411 KAT/random) |
| Plonky3-recursion verifier-circuit (§4) | 7 (Poseidon2/1, Recompose, WitnessChecks, FRI, batch, executor) | ~25 | n/a (uses CTL) | 7 (Poseidon2 S-box) | ~30 |
| ai-pow extraction (§5; not AIR) | 6 | n/a (off-circuit) | n/a | n/a | ~30 |
| **TOTAL (AIR-side)** | **~21** | **~117** | **~7** | **7** | **~160** |
| Plus ai-pow extraction | +6 | n/a | n/a | n/a | +~30 |
| **GRAND TOTAL** | **~27** | **~117** | **~7** | **7** | **~190** |

This rough number aligns with the 217-test tamper inventory
when including infrastructure-level tests (preprocessing format,
challenger-bit bounds, batch-prover validate-rejects).

---

## 8. What S1 consumes from this inventory

S1 (`2026-05-21_CONSTRAINT_SOUNDNESS_DERIVATION.md`, next
session) will use this inventory to:

1. Compute per-constraint Schwartz–Zippel bound
   (`d_constraint / q_chal`) per row above.
2. Compute per-LogUp-bus bound (`k_b · 2 / q_chal`) per bus
   in §2.13.
3. Aggregate per-AIR `Σ d / q_chal` and compute MIN bits.
4. Verify every per-AIR MIN ≥80 with explicit margins.
5. Combine with FRI MIN ≥82 (from S(−1)) to derive chain MIN.
6. Flag any per-AIR with margin < 10 bits for re-examination.

The rough estimate from CSA design §3.3 is total ε ≤
2^(-118), so per-AIR MIN should comfortably clear 80
unconditional with ≥38-bit margin.

---

## 9. Cross-references

- **Sibling FRI-side analysis (closes the FRI half).**
  `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`.
- **CSA design (the parent of S0).**
  `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`.
- **C4 audit-readiness (soundness-claim index).**
  `2026-05-19_C4_AUDIT_READINESS.md` § 3 + § 7.
- **Per-AIR design sources (referenced in claim cross-links).**
  `2026-05-15_HIGH2_2_DESIGN.md` (HIGH-2.2 §4.A–§4.E + §6);
  `2026-05-17_CANONICAL_PROGRAM_DESIGN.md` (CRIT-1);
  `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` (A3.x);
  `2026-05-14_M52_MATRIX_BINDING.md` (M52);
  `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` (P-B.2.x);
  `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` (C2.1 / C2.3);
  `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13/14/15 (C3 / DT-4);
  `2026-05-18_C1_RECURSION_VENDOR_DESIGN.md` (C1 substrate).
- **Tip5 paper (the soundness oracle for §3).**
  `2023-107.pdf` (IACR ePrint 2023/107).
- **FRI soundness paper (the underwriter of §4.5).**
  `2025-2055.pdf` (IACR ePrint 2025/2055; consumed by S(−1)).
- **Production roadmap (parent milestone).**
  `2026-05-17_PRODUCTION_ROADMAP.md` Phase C.

---

## 10. R1 honest accounting

**Validated (this commit):** Inventory of every AIR and every
constraint family identified by the upstream Explore-agent
research; cross-link to existing tamper tests; categorization
of GAPs (G1/G2/G3).

**Precise residual:**
- Some `file:line` markers cite ranges with `~` (approximate)
  because line-exact pinpointing of every constraint requires
  per-AIR re-read of the source. The approximate markers are
  sufficient for S1's per-AIR aggregation; S3's tamper-test
  specifications will tighten them to exact line numbers
  when needed.
- Max polynomial degree per constraint family is best-effort;
  S1 will refine by reading actual Plonky3 symbolic degree
  output for cross-validation.
- Constraint count totals (~117) are order-of-magnitude;
  S1's per-AIR walkthrough will yield exact counts.

**No fake completion.** This inventory is the data foundation
for S1–S7, not a substitute for them. Every constraint family
is identified with at least one test pointer or an explicit
GAP-G3 flag.
