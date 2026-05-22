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

### Path B — eliminate a redundant snapshot — ✗ INVALID (de-risk finding, 2026-05-21)

The initial hypothesis was that `INPUT_STATE` is stored twice
(once as row r's `INPUT_STATE`, once as row r−1's `STATE3`) and
one copy could be dropped for −264 cols.

**Reading `round_air.rs::verify_round` before editing (the R1
KAT-first/de-risk step) disproved this.** `verify_round` takes
**5 states** — a BLAKE3 round is **4 half-G transitions**:
`s0→s1→s2→s3→s4` (column-G halves 1+2, diagonal-G halves 1+2).
A row stores 4 snapshots `[s0, s1, s2, s3]`; the 5th state
`s4` (the round *output*) is built from the **next row's
`INPUT_STATE` columns via the AIR window** — `s4[r]` and
`s0[r+1]` are the *same physical columns*. The next row's
`INPUT_STATE` is read by round r's constraint (as output) and
round r+1's constraint (as input): **shared, used twice, stored
once.** There is no duplicated snapshot.

The 4 stored snapshots are already minimal: input + 3
intermediates, one per half-G, each intermediate needed to keep
every half-G a separate degree-3 constraint step. Dropping any
snapshot either loses needed data or collapses two half-G steps
into one higher-degree constraint (degree budget). **Path B is
struck.** The de-risk step did its job — caught a false premise
before any invasive edit.

### Path C — SX-stripe reduction — ✗ RE-SCOPED (de-risk finding, 2026-05-21)

The initial hypothesis was that some of `sx_stripe`'s 256
bit-decomposition columns (`SX_IN_BITS` 128 + `SX_XR_SEL_BITS`
32 + `SX_NEW_SEL_BITS` 32 + `SX_Q_BITS` 64) are pure
range-checks movable to lookup-based range checks.

**Reading `chips/stripe_xor.rs` before editing disproved this.**
*All four* SX bit-fields are genuine XOR machinery — the SX
chip computes `NEW_SEL = XR[lane] ⊕ ⊕SX_IN` (a 5-way 32-bit
XOR) via the per-bit-parity identity
`XR_bit[i] + Σ IN_bit[c][i] = NEW_bit[i] + 2·Q[i]`, where the
`*_BITS` are the bit decompositions of the XOR operands/result
and `Q_BITS` are the parity quotient bits. There are **no
range-check-only bit-fields** in SX.

Moreover the chip's own doc-comment ("Why per-bit parity, not a
lookup") records that a lookup-based XOR was **already
considered and deliberately rejected**: the per-bit-parity
gadget keeps every constraint degree ≤ 2 and is the *uniform
audited XOR-gadget shape* used across the codebase (`xstep`,
the FoldChip rotl13-XOR, and SX).

**Re-scoped conclusion.** "Path C" is not a separate
range-check-to-lookup change. The only SX width reduction is
the *same* core problem as Path A: replace per-bit XOR with a
byte-limb + byte-XOR-lookup gadget. Path C (SX) and Path A
(BLAKE3) therefore share one core deliverable — a sound,
low-degree byte-XOR-lookup gadget — and Path C is just its
rotation-free first application. They are **not separable**;
"do C then A" collapses to "build the byte-XOR-lookup gadget,
apply to SX, then apply to BLAKE3 (+rotation)."

### Path D — adopt Plonky3's `p3-blake3-air` — ✗ NO SHORTCUT (triage finding, 2026-05-21)

Triaged by reading `p3-blake3-air/src/columns.rs`. Two findings
strike Path D as a width-reduction shortcut:

1. **`p3-blake3-air` uses the *same* full 32-bit boolean
   decomposition.** Its `Blake3State` keeps `row1` and `row3` as
   `[[T; 32]; 4]` — the doc-comment states verbatim: *"Rows 1 and
   3 are saved as 32 boolean values."* Upstream Plonky3 has **not**
   solved the XOR-side with limbs+lookups; it explodes to 1-bit
   columns exactly as our Pearl-ported chip does. Adopting it
   gives no XOR-side width win.
2. **Incompatible layout.** `p3-blake3-air` computes a *whole
   BLAKE3 compression per row* (`full_rounds: [FullRound; 7]`,
   `NUM_BLAKE3_COLS` ≈ 9000+ columns wide). Our composite AIR
   spreads one round per trace row (1056 wide, reused across
   non-BLAKE3 activity). Adopting upstream would mean a ~9000-col
   row — far wider — and would break the per-row activity
   multiplexing the composite AIR depends on.

**Conclusion:** a limb+lookup BLAKE3 XOR-side is *not* available
off-the-shelf — neither our chip nor upstream Plonky3 has built
it. Path A is therefore research-grade AIR design, not a port.

## 4. Ranked recommendation

| Path | Win (cols / % trace) | Risk | Effort | Verdict |
|---|---|---|---|---|
| A — XOR-side limb + lookup | ~−896 / −42% | High — research-grade BLAKE3 AIR design | ~2-4 wk | the only viable large lever |
| B — drop INPUT_STATE snapshot | — | — | — | ✗ INVALID (premise disproved by de-risk) |
| C — SX-stripe limb + lookup | up to ~−200 / −9% | Medium-High | ~1 wk | viable, smaller |
| D — adopt `p3-blake3-air` | none for XOR-side | — | — | ✗ NO SHORTCUT (upstream uses the same 32-bit bits) |

Two of the four investigated paths were **struck by the de-risk
investigation**, both before any invasive edit — exactly what the
de-risk step is for. **Path A is the load-bearing lever** and is
the direct expression of the Goldilocks-field hint: limb
decomposition exploiting the field's 32-bit-native capacity +
lookup XOR, mirroring how `add3_unchecked` already exploits the
field for the BLAKE3 ADD-side. Path D's triage additionally
revealed that *no existing BLAKE3 AIR* (ours or upstream
Plonky3's) has built a limb+lookup XOR-side — so Path A is
genuinely new AIR design, raising its effort estimate to
**~2-4 weeks** of staged, soundness-critical work.

Path C (SX stripe) is the smaller, lower-novelty companion: the
256 SX bit-decomposition columns include range-check fields that
can move to lookup-based range checks (the composite AIR already
runs URANGE8/URANGE13 LogUp tables — no new machinery, no rotate
complication for the pure range-check fields).

**Recommended sequence:** C first (bounded, lower-novelty, ~−9%,
reuses existing range-table machinery) → A (the research-grade
−42% BLAKE3 redesign). Combined ceiling ≈ −51% inner trace width
⇒ proportional inner-prove speedup + inner-proof shrinkage +
(cascading) a smaller L1 verifier circuit.

## 5. De-risk plan for the Path A implementation drive

Per R1, the BLAKE3 XOR-side redesign is staged behind KAT-first
de-risk:
1. **KAT-first:** the current BLAKE3 round-AIR accept/tamper KATs
   (`prove_and_verify_valid_round`, `prove_and_verify_two_different_rounds`,
   the `xor_32_shift_if_rotate_{16,12,8,7}_matches_pearl` rotate
   KATs) + the composite golden-KAT byte-equivalence are the
   frozen oracle. Baseline confirmed: **53 BLAKE3 chip tests
   pass** at the current layout.
2. **Path D triage FIRST:** before rebuilding the XOR-side from
   scratch, cost `p3-blake3-air` (already a dependency) — does
   its column layout already solve the XOR-side efficiently, and
   does it support the keyed/tweaked BLAKE3 the ai-pow protocol
   needs? If yes, Path A reduces to an adoption + integration job.
3. **Path A staged (if D doesn't subsume it):** (i) add the
   byte-XOR LogUp table to the composite AIR's lookup set;
   (ii) replace one snapshot's ROW2/ROW4 32-bit decomposition
   with byte-limbs + lookup XOR, keeping a nibble fallback for
   the non-byte-aligned rotate-by-12/7; (iii) re-run the full KAT
   + composite regression; (iv) roll across all 4 snapshots;
   (v) shift the layout, update the pinned counts.
4. **Per-stage exhaustive gates:** the 53 BLAKE3 chip tests + the
   ai-pow-zk full lib regression (370 tests) + a `bench_prod_8k_baseline`
   re-measure + the `inner_air_column_inventory` test (its pinned
   counts updated to the new layout — that update IS the
   integration check).

## 5b. The byte-XOR-lookup gadget has a table-height obstacle (de-risk finding, 2026-05-21)

Designing the unified byte-XOR-lookup gadget surfaced a concrete
feasibility wall:

- A **byte**-XOR lookup table is `256 × 256 = 65 536` entries
  `(a, b, a⊕b)`. A LogUp table must be materialised as that many
  rows. The inner composite trace is `MIN_STARK_LEN = 8192` rows
  (grows with workload). Embedding a 65 536-row table forces the
  trace to **≥ 65 536 rows — 8× the baseline** ⇒ 8× the LDE ⇒
  the prover gets *slower*, defeating the latency purpose, and
  the inner proof grows. A *separate* 65 536-row table-table is
  possible but is its own large commitment.
- A **nibble**-XOR table is `16 × 16 = 256` entries — fits the
  existing trace height trivially (the `URANGE8` table is
  already 256 rows). But nibble decomposition gives only a **4×**
  XOR-side width cut (32 bits → 8 nibbles), not 8×, and the
  BLAKE3 rotate-by-7 is not nibble-aligned (`7 = 1 nibble + 3
  bits`) ⇒ still needs sub-nibble handling.

**Net:** the clean "−42% via byte-XOR-lookup" figure in §3 Path A
is **not achievable as stated** — the byte table is
trace-height-infeasible. The realistic ceiling is the
nibble-table variant: a ~4× XOR-side reduction
(BLAKE3 1024 bit-cols → ~256 nibble-cols, ~−768 cols / −36%
trace) with the rotate-by-7 sub-nibble caveat, OR a separate
byte-XOR table-table (a costing exercise of its own). This is
the genuine, non-obvious research-grade obstacle that makes the
inner-AIR width reduction a multi-session effort, not a
session-scoped change.

## 5a. R1 status of this analysis drive

This drive *attempted* the implementation and the de-risk step
hit concrete walls on the two paths that looked cheap enough to
land this session:

- **Path B attempted → struck.** Reading `verify_round` before
  editing disproved the "duplicated INPUT_STATE snapshot"
  premise — the AIR window already shares the boundary snapshot;
  the 4 stored snapshots are minimal.
- **Path D triaged → struck.** Reading `p3-blake3-air/columns.rs`
  showed upstream Plonky3 uses the *same* 32-bit boolean
  decomposition — no off-the-shelf limb+lookup XOR-side to adopt.

Both findings are genuine in-flight de-risk results, not
avoidance. They leave **Path A** — confirmed as research-grade
BLAKE3 AIR design (no existing implementation has done a
limb+lookup XOR-side) — as the only large lever, and **Path C**
as a bounded smaller one. A research-grade ~2-4-week
soundness-critical redesign of the PoUW-linchpin BLAKE3 AIR
**must not be rushed into a partial landing** (R1: a half-landed
invasive soundness change is strictly worse than a clean
validated subset + precise residual).

The **validated deliverable** of this drive: the exhaustive
column inventory (landed as the `inner_air_column_inventory`
test — executable, regression-protected), this corrected
multi-path analysis with two paths de-risked to negative
conclusions, and the Path C / Path A de-risk plan. The Path C
(then A) implementation is the precise, scoped residual for the
next dedicated drive — which, per R1.1, must drive it (the
design + de-risk now exist) rather than re-analyze.

## 6. Files

- `crates/ai-pow-zk/src/composite_layout.rs`: added
  `inner_air_column_inventory` test (the exhaustive 15-group
  accounting; the measurement this analysis rests on).
- _This doc._
