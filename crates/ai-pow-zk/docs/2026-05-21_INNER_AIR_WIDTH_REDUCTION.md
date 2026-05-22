# 2026-05-21 — Inner composite-AIR width reduction: column inventory + multi-path analysis

> _Created **2026-05-21**. Follow-up to
> `2026-05-21_INNER_POUW_OPTIMIZATION.md`, which identified inner-AIR
> trace width as the linear multiplier on the inner prover's
> dominant cost (trace LDE + Tip5-MMCS commit) and on the inner
> proof size._

## 0. Status (R1, honest)

**Analysis + multi-path investigation deliverable.** The exhaustive
column inventory is landed as a reproducible test
(`composite_layout::tests::inner_air_column_inventory`). The
width-reduction *implementation* is a soundness-critical invasive
change to the PoUW linchpin (the composite AIR is what proves the
mined work) — it is **precisely scoped as a staged residual here**,
NOT rushed. Per R1: design + de-risk first, invasive edits + per-
stage exhaustive validation as the dedicated next drive.

## 1. Column inventory — where the 2135 columns go

`composite_layout::tests::inner_air_column_inventory` (run with
`--nocapture`) partitions all `TOTAL_TRACE_WIDTH = 2135` columns
into 15 chip-groups and asserts the accounting:

| Group | cols | % width |
|---|---:|---:|
| **blake3_round** | **1056** | **49.5%** |
| **sx_stripe** | **390** | **18.3%** |
| input_unpacking | 200 | 9.4% |
| fold | 99 | 4.6% |
| matmul_tile | 80 | 3.7% |
| fold_stripe_sel | 64 | 3.0% |
| jackpot_state | 56 | 2.6% |
| blake3_buffers | 49 | 2.3% |
| jackpot_xbits | 49 | 2.3% |
| noised_packed_indexing | 35 | 1.6% |
| control | 22 | 1.0% |
| range_tables | 11 | 0.5% |
| matmul_accum | 8 | 0.4% |
| blake3_output | 8 | 0.4% |
| msg_pair_sel | 8 | 0.4% |

**`blake3_round` + `sx_stripe` = 67.8% of the trace.** Everything
else is ≤9.4%; the matmul "useful PoUW work" itself
(`matmul_tile` + `matmul_accum` + most of `input_unpacking` +
`noised_packed_indexing`) is well under a quarter of the width.

## 2. The dominant structure — BLAKE3 bit-decomposition

`blake3_round` is 4 state snapshots/round × 264 cols
(`chips/blake3/layout.rs`):

```
per snapshot:  ROW1 (4, ADD-side packed-16bit)
               ROW2 (128, XOR-side 32-bit bit-decomposition)
               ROW3 (4, ADD-side packed-16bit)
               ROW4 (128, XOR-side 32-bit bit-decomposition)
4 snapshots:   INPUT_STATE, STATE1, STATE2, STATE3
```

Of the 1056 columns, **1024 (= 4 snapshots × 2 × 128) are full
32-bit bit-decompositions** of the XOR-side state cells — i.e.
**48% of the entire inner trace is BLAKE3 XOR-side bits.**

### 2.1 Why bits — and where the field is already used

The BLAKE3 G-function does ADD (mod 2³²), XOR, and rotate.
`chips/blake3/round_ops.rs` shows the AIR **already uses
Goldilocks field arithmetic for the ADD-side**: `add3_unchecked`
constrains `res ∈ {sum, sum−2³², sum−2³³}` via a single degree-3
cubic `diff·(diff−2³²)·(diff−2³³)=0` on the *packed* 32-bit field
elements — no bit-decomposition. The ADD-side rows (ROW1, ROW3)
are therefore only 4 columns each.

The **XOR-side is the holdout.** `xor_32_shift_if` needs `b` as a
32-bit bit-decomposition because XOR and rotate are bit-level
operations with no native field representation. Each XOR-side
state cell costs 32 columns.

This is the exact opportunity the maintainer's hint —
"make use of traits in the Goldilocks field" — points at: the
ADD-side already exploits that a Goldilocks element natively
holds a full 32-bit word (P = 2⁶⁴−2³²+1 ⇒ 32-bit values and
their sums fit with room to spare, and the mod-2³² wrap is a
cheap low-degree polynomial). The XOR-side should exploit the
*same* field capacity instead of exploding to 1-bit columns.

## 3. Multiple paths for the width reduction

### Path A — XOR-side limb-decomposition + lookup XOR (largest win)

Replace each 32-column bit-decomposition with a coarser **limb**
decomposition + a **lookup-based XOR**, using the field's
capacity to hold a multi-bit limb:

- Decompose each 32-bit XOR-side word into **4 bytes** (4 cols)
  instead of 32 bits.
- XOR of two bytes ← a preprocessed `256×256` byte-XOR lookup
  table (the composite AIR already runs LogUp lookups —
  `range_tables`, the Tip5 split-and-lookup; a byte-XOR table is
  the same machinery, 65 536 rows).
- ROW2/ROW4 shrink from 128 (4 cells × 32 bits) to 16
  (4 cells × 4 bytes). `blake3_round` → `4 × (4+16+4+16) × 4` =
  **~640 cols → ~896-column reduction (~42% of the whole trace).**

**The rotate complication.** BLAKE3 rotates by 16, 12, 8, 7.
- Rotate 16, 8 are byte-aligned ⇒ free limb permutations.
- Rotate 12, 7 are *not* byte-aligned ⇒ need sub-byte handling:
  either a dedicated rotate-by-k lookup, or a hybrid where the
  rotated operand keeps a finer (e.g. 4-bit nibble) decomposition.

This is the highest-value path and the one the Goldilocks-field
hint endorses, but it is a genuine BLAKE3-AIR redesign:
soundness-critical (the AIR proves the mined PoUW), must
preserve BLAKE3 byte-equivalence (the hash *output* — the AIR
layout may change freely, only the proven function is fixed),
and must hold the degree-3/4 budget. **Estimated: ~1-2 weeks
staged R1 work.**

### Path B — eliminate the redundant INPUT_STATE snapshot

Each round stores 4 snapshots. `STATE3` of row r equals
`INPUT_STATE` of row r+1 (the round output feeds the next round).
The two are distinct physical columns in adjacent rows with an
equality constraint between them — each round's state is stored
twice. Eliminating `INPUT_STATE` (read row r's input from row
r−1's `STATE3` via the AIR row-window; bind round-0's input to
the `CV_IN`/`BLAKE3_MSG` buffers via `verify_init_state`) saves
one snapshot: **−264 cols (−12.4% of the trace).** Smaller than
Path A, and independent of it — composable. Medium risk: a
bounded "drop one snapshot" restructure of the round AIR window.

### Path C — SX-stripe reduction (390 cols, 18.3%)

`sx_stripe` carries `SX_IN_BITS` 128 + `SX_XR_SEL_BITS` 32 +
`SX_NEW_SEL_BITS` 32 + `SX_Q_BITS` 64 = **256 columns of
bit-decomposition** (66% of the SX block). The same
limb+lookup substitution as Path A may apply to the
bit-decomposed SX fields. Requires a study of what each SX
bit-field feeds (XOR/rotate vs range-check vs selector) — a
field used purely for a range-check can move to a lookup
(byte-limbs + range table) with no rotate complication.
**Estimated win: up to ~200 cols** depending on how many SX
bit-fields are range-checks vs genuine bit-ops.

### Path D — adopt Plonky3's `p3-blake3-air`

ai-pow-zk already lists `p3-blake3-air` as a dependency. Plonky3's
upstream BLAKE3 AIR is trait-generic over the field and may have
a more column-efficient layout than the Pearl-ported 1056-column
chip. Switching the inner composite AIR's BLAKE3 sub-AIR to
`p3-blake3-air` would inherit upstream's column budget + any
field-trait-based efficiency. Caveat: the Pearl-ported chip was
chosen for a reason (the M10.1c integration + the BLAKE3
keyed-hash tweak handling); a switch needs to confirm
`p3-blake3-air` supports the keyed/tweaked BLAKE3 variant the
ai-pow protocol uses. **Investigation item, not yet costed.**

## 4. Ranked recommendation

| Path | Win (cols / % trace) | Risk | Effort |
|---|---|---|---|
| A — XOR-side limb + lookup | ~−896 / −42% | High (BLAKE3 AIR redesign) | ~1-2 wk |
| B — drop INPUT_STATE snapshot | −264 / −12.4% | Medium (bounded restructure) | ~3-5 d |
| C — SX-stripe limb + lookup | up to ~−200 / −9% | Medium-High | ~1 wk |
| D — adopt `p3-blake3-air` | unknown; needs costing | Medium | investigate first |

**Path B is the cheapest concrete win and is independent of A** —
it can land first as a de-risked, bounded "remove one snapshot"
change while Path A's larger redesign is staged. Path A is the
largest win and is the direct expression of the Goldilocks-field
hint (limb decomposition exploiting the field's 32-bit-native
capacity + lookup XOR, mirroring how `add3_unchecked` already
exploits the field for the ADD-side).

**Recommended sequence:** B (bounded, ~−12%) → A (the big
redesign, ~−42%) → C (SX, ~−9%). Combined ceiling ≈ −63% inner
trace width ⇒ proportional inner-prove speedup + inner-proof
shrinkage + (cascading) a smaller L1 verifier circuit.

## 5. De-risk plan for the implementation drive (Path B first)

Per R1, before any invasive edit:
1. **KAT-first:** capture the current BLAKE3 round-AIR
   accept/tamper KATs (`prove_and_verify_valid_round`,
   `prove_and_verify_two_different_rounds`) + the composite
   golden-KAT byte-equivalence as the frozen oracle.
2. **Path B staged:** (i) add the row-window read of row r−1's
   `STATE3` as the round-r input; (ii) bind round-0 input to the
   buffers; (iii) delete the `INPUT_STATE` columns + shift the
   layout; (iv) re-run the full KAT + composite regression at
   each sub-step.
3. **Per-stage exhaustive gates:** BLAKE3 chip tests + the
   ai-pow-zk full lib regression (370 tests) + a re-run of
   `bench_prod_8k_baseline` to measure the prove-time win + the
   `inner_air_column_inventory` test (which will need its pinned
   counts updated to the new layout — that update IS the
   integration check).

## 6. Files

- `crates/ai-pow-zk/src/composite_layout.rs`: added
  `inner_air_column_inventory` test (the exhaustive 15-group
  accounting; the measurement this analysis rests on).
- _This doc._
