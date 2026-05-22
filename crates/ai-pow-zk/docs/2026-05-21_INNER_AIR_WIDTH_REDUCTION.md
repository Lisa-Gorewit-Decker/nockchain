# 2026-05-21 — Inner composite-AIR width reduction: column inventory + multi-path analysis

> _Created **2026-05-21**. Follow-up to
> `2026-05-21_INNER_POUW_OPTIMIZATION.md`, which identified inner-AIR
> trace width as the linear multiplier on the inner prover's
> dominant cost (trace LDE + Tip5-MMCS commit) and on the inner
> proof size._

## 0. Status (R1, honest)

**Analysis + multi-path investigation + one landed reduction.** The
exhaustive column inventory is landed as a reproducible test
(`composite_layout::tests::inner_air_column_inventory`).

- **LANDED — Path C** (§5c): SX_Q width reduction, −32 inner-trace
  columns (`TOTAL_TRACE_WIDTH` 2135→2103), committed, validated by
  the full ai-pow-zk regression (371 tests incl. the composite
  golden-KAT byte-equivalence) + the 8 SX-chip tests + the
  inventory test.
- **LANDED — Path A column-overlay** (§7.1): the §7 big lever,
  implemented and **rolled across every overlay-eligible SX run**.
  The three StripeXor *bit* runs — `SX_IN_BITS` (128),
  `SX_XR_SEL_BITS` (32), `SX_NEW_SEL_BITS` (32) — now physically
  *alias* `blake3_round[0..192]`. `TOTAL_TRACE_WIDTH` 2103 →
  **1911** (−192). Sub-stages, all landed + committed:
  - **O0-Stage-1** — the SX chip's data-validation constraints are
    `IS_ACTIVE`-gated in `stripe_xor::eval_at` (vacuous on BLAKE3
    rows).
  - **O0-Stage-2** — `verify_round` in the BLAKE3 round AIR is
    gated off on matmul rows (StripeXor activity is co-located on
    the matmul sweep, so an SX row carries the *pinned*
    `IS_RESET_CUMSUM`/`IS_UPDATE_CUMSUM` selectors — the
    mutual-exclusion kernel was already verifier-fixed; **no
    CRIT-1 program-pin change needed**). The gate excludes the
    matmul selectors on *both* the current and next row
    (`verify_round` is cross-row); the gate stays degree 2 — **no
    degree bump, no recursion-config change**.
  - **O2** — `composite_layout.rs` aliases the three bit runs onto
    `blake3_round[0..192]`; the trace-gen is offset-driven so it
    needed no change (SX and BLAKE3 write disjoint rows).
  - **Overlay-eligibility boundary** — only genuine 0/1 *bit*
    columns may alias the `blake3_round` region: the BLAKE3 round
    AIR boolean-checks its XOR-input bit columns *unconditionally*
    (`round_ops.rs xor_32_shift_if` — an ungated `assert_bool`), so
    a bit satisfies it wherever it lands. `SX_IN` (signed-i32
    accumulator cells) and `SX_Q` (`∈{0,1,2}`) are **not**
    bit-valued ⇒ **not overlay-eligible**; they stay block-own.
    This was a de-risk finding — a first attempt that overlaid
    `SX_IN` failed the §6(b) chain with `OodEvaluationMismatch`.
  Validated: full ai-pow-zk regression **371 pass / 0 fail / 23
  ignored** — composite golden-KAT byte-equivalence, §6(b)
  keystone chain, adversarial tamper-rejection (incl. the new
  `path_a_overlay_aliased_blake_columns_still_constrained`
  overlay-tamper test), CRIT-1.
- **No residual on Path A.** The overlay is rolled across all
  overlay-eligible runs; `SX_IN`/`SX_Q` are bounded out by the
  bit-column eligibility rule above, not deferred. Future width
  work would target other chips' bit-decompositions (e.g. the
  jackpot or fold bit columns) under the same mechanism.

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

## 5c. LANDED — SX_Q width reduction (−32 cols, 2026-05-21)

A concrete bounded reduction *was* found and landed inside the SX
block, missed by the first-pass analysis. The SX parity gadget's
quotient `Q[i]` is a **bounded value** (`∈ {0,1,2}`), not an XOR
operand — it was stored as `QBITS = 2` boolean columns per
output-bit position (`SX_Q_BITS`, 64 cols), which over-provisions
the range to `{0,1,2,3}`. Replaced with **one value column per
position** (`SX_Q`, 32 cols) range-bounded by the cubic
`Q·(Q−1)·(Q−2) = 0`:

- −32 columns (`TOTAL_TRACE_WIDTH` 2135 → 2103, −1.5%).
- Constraint degree on that gadget 2 → 3 — within the composite
  AIR's budget (Pearl pins `constraint_degree = 3`).
- Soundness *tighter*: the cubic constrains exactly `{0,1,2}`,
  vs the 2-bit form's `{0,1,2,3}`.
- Validated: 8/8 SX chip tests (incl. the re-pointed
  `rejects_out_of_range_q` tamper test, Q = 3), the
  `inner_air_column_inventory` test (`sx` pin 390 → 358), the
  full ai-pow-zk lib regression.

This is the "store the bounded value, not its bits" pattern (the
same shape as the outer-cert Angle A A-column elimination). It is
the *only* such derivable-column win in the SX block — the other
256 SX bit columns are genuine XOR machinery (§3 Path C).

## 7. The big lever — column overlay / activity-multiplexing (maintainer direction, 2026-05-21)

> Maintainer: *"reuse existing space by adding a column that
> switches a section's purpose."*

This is a stronger architecture than the §3 byte-XOR-lookup
redesign and **needs no new gadget, table, or rotation handling**.

**Observation.** Every composite-AIR row performs exactly *one*
activity — a matmul step, *or* a BLAKE3 round, *or* a jackpot
step, *or* a stripe-XOR fold. The per-activity selector bits
(`IS_HASH_A`, `IS_MSG_MAT`, `SX_IS_ACTIVE`, `FOLD_IS_FOLD`, the
matmul selectors) already gate which constraints fire. But the
*columns* are all dedicated: the 1056 BLAKE3 columns sit inert on
a matmul row, the 358 SX columns sit inert on a BLAKE3 row, etc.

**The overlay.** Mutually-exclusive chips can *share physical
columns* — overlay the smaller chip regions (`sx_stripe` 358,
`fold` 99, `jackpot_*` ~105, `matmul_tile` 80, …) onto sub-windows
of the largest region (`blake3_round`, 1056). The activity
selector already in the trace picks the interpretation. Ceiling:
collapse ~650+ chip-specific columns into the BLAKE3 region ⇒
**~−30% inner trace width**, comparable to the §3 Path A target
but with no XOR-gadget redesign.

**Soundness requirements (the R1 de-risk targets):**

1. **Mutual exclusion** — must *constrain* (not assume) that at
   most one overlaid chip's selector is 1 per row.
2. **Full gating** — every constraint of each overlaid chip must
   be gated by that chip's selector, so an overlaid foreign
   chip's data is vacuous to it. Any ungated constraint breaks
   the overlay. (`stripe_xor::eval_at`'s constraints are
   self-satisfying at all-zero — that property must hold, or be
   made to hold, for every overlaid chip.)
3. **Trace-gen** — the active chip writes the shared region; the
   "zero-default" discipline several layout blocks rely on
   changes and must be re-audited.
4. **Activity granularity** — confirm no row legitimately needs
   two activities at once (e.g. a BLAKE3 row that also carries a
   matmul-accumulator passthrough). Any genuine co-occurrence
   blocks overlaying those two.
5. **Per-row statelessness** — *(added by the 2026-05-21 de-risk,
   §7.1)* the guest chip must carry no cross-row register /
   accumulator that has to stay live on the host chip's rows. A
   stateful chip's register columns cannot be overlaid at all:
   the interleaved host rows would overwrite the register and
   break its propagation. `sx_stripe` **fails** this — see §7.1.

This is the recommended big-win path. It is invasive
(soundness-critical, touches every overlaid chip's column
addressing + the mutual-exclusion proof). The 2026-05-21 de-risk
(§7.1) found it is **not** the "bounded and mechanical per chip"
profile first assumed: it is gated on a composite-wide
gating-model rewrite (precondition stage O0) and the guest chip
must be per-row stateless. Corrected staging is in §7.1.

## 7.1 De-risk findings — the overlay is gated on two preconditions (2026-05-21)

A genuine in-flight de-risk attempt of §7 (tracing the actual
constraint code in `composite_full_air.rs` + `chips/stripe_xor.rs`
and the trace-gen in `stripe_xor::build_trace`, *before* any
invasive edit, per R1's "KAT-first / de-risk before invasive
edits") found **two concrete walls**. The §7 requirement list
above is corrected accordingly.

### Finding D1 — the composite uses *zero-default discipline*, not selector-gating

§7 requirement #2 ("full gating — every constraint of each
overlaid chip is gated by that chip's selector") is **currently
false for the entire composite.** `composite_full_air.rs:36–47`
documents the discipline verbatim: on a passthrough row "all
selectors are 0, all data columns are 0, control_prep = 0" ⇒
every chip's constraints *self-satisfy at all-zero* rather than
being explicitly selector-gated. Confirmed in `stripe_xor::eval_at`
(`chips/stripe_xor.rs:214–369`): every `assert_eq(A,B)` holds
because `A = B = 0`, every `assert_bool(x)` because `x = 0`, every
`assert_zero(e)` because `e = 0` — there is no `is_active` gate
(the doc-comment at L300–302 says so explicitly for the `Q` cubic).

Consequence: an overlay puts *non-zero foreign data* into the
shared columns, which breaks self-satisfaction. So the overlay's
precondition #2 requires first converting **both** the guest chip
**and** the 1056-col `blake3_round` host AIR from zero-default to
explicit per-constraint selector-gating. That is a composite-wide
soundness-critical rewrite — larger and more delicate than any
single §3 path — and is **not** behavior-preserving: gating a
constraint removes its enforcement on inactive rows, so every
cross-row reader of the gated chip's columns must be re-audited
against now-unconstrained inactive-row data (a missed gate or a
missed reader = a forgery hole).

### Finding D2 — `sx_stripe` (the §7-recommended first candidate) is STATEFUL and cannot be overlaid

The SX chip carries a global cross-row register `XR` (16 lanes).
`stripe_xor::eval_at:313–369`: a `when_first_row` boundary forces
`XR = 0`, and a `when_transition` passthrough propagates `XR`
row-to-row. `stripe_xor::build_trace:417–464` threads the register
through *every* row including padding rows — explicitly, because
the HIGH-2.2 §6(b) keystone binding (`composite_full_air.rs:301`,
`FOLD_STRIPE_SEL · (FOLD_XSTEP − SX_XR)`) reads `SX_XR` on the
composite trace's **last row**.

If the `XR` columns were overlaid onto BLAKE3 columns, every
BLAKE3 row would overwrite the register with BLAKE3 data, the
transition passthrough would be destroyed, and the §6(b)
soundness chain `committed A/B → CUMSUM → SX_IN → SX_XR →
FOLD_XSTEP` would break. A stateful chip's register columns are
*structurally* un-overlayable: an overlay-eligible guest chip must
be **per-row stateless**. This is the new requirement #5. SX fails
it (so do `matmul_tile`'s `CUMSUM` accumulator and the `fold`
accumulator — they need the same classification before any
overlay).

### Finding D3 — gating the SX `Q` cubic costs degree 3→4 — a parameter, not a wall (maintainer, 2026-05-21)

The SX `Q` range check `Q·(Q−1)·(Q−2) = 0` (landed by Path C, §5c)
is **degree 3**. Wrapping it in a degree-1 `IS_ACTIVE` gate makes
it degree 4. The composite has historically been kept at
`constraint_degree = 3` (a Pearl design convention — `circuit.rs`
`CircuitConfig` doc-comment), but this is **not a hard limit**:

- The FRI configs already admit degree 4. `quotient_degree <
  2^log_blowup` (`circuit.rs:192–194`) ⇒ `constraint_degree ≤
  2^log_blowup`. `PROD` `log_blowup = 4` ⇒ degree ≤ 16; even the
  test profile `TEST_PEARL` `log_blowup = 2` ⇒ degree ≤ 4. So a
  degree-4 composite needs **no FRI-soundness-config change**.
- Maintainer 2026-05-21: *"We can increase degree in our
  production params. It doesn't have to fit the TEST_PEARL
  params."* — the degree budget is an accepted parameter.

The real cost is **downstream, not local**: raising the composite's
max constraint degree 3→4 changes the inner STARK's quotient
splitting (`quotient_degree` 2→4 chunks), which changes the inner
proof shape, which the L1 outer-cert verifier circuit is
specialized for ⇒ an **L1 verifier-circuit regeneration + recursion
re-validation** (`c3_stage_a`/`c3_stage_b`).

Sequencing consequence: the overlay itself (O2) reduces
`TOTAL_TRACE_WIDTH`, which *also* changes the inner AIR shape and
*also* forces an L1 regeneration. So the `Q`-cubic gating is
**free if folded into O2** (one L1 regen covers both) and wasteful
if done standalone now (an L1 regen for 32 not-yet-overlaid
columns = zero interim saving). Therefore: O0-Stage-1 leaves the
`Q` cubic ungated; gating `Q` (→ all 224 SX bit-columns
overlay-eligible, not just ≈ 192) is scheduled into **O2**, where
the L1 regen happens regardless.

### Corrected staging

- **Stage O0 (precondition).** Convert the composite from
  zero-default discipline to explicit per-chip selector-gating —
  per chip, KAT + full-regression-gated. This is the bulk of the
  soundness-critical work (every overlaid chip *plus* the
  1056-col `blake3_round` AIR) and must precede any column
  re-pointing. Each chip's cross-row readers re-audited per D1.
  - **O0-Stage-1 — LANDED.** The SX chip's data-validation
    constraints (`IN`/`IN_BITS`/`XR_SEL_BITS`/`NEW_SEL`/
    `NEW_SEL_BITS` reconstruction + bit-booleanity + the XOR
    parity) are gated by `IS_ACTIVE` in `stripe_xor::eval_at`.
    Left **ungated** by design: `IS_ACTIVE`-booleanity (the gate
    itself), the `LANE_SEL` structural constraints (the ungated
    register passthrough depends on them — D2), and the `Q` cubic
    (degree budget — D3). Validated: 8/8 SX chip tests + the full
    ai-pow-zk regression (371 pass / 0 fail / 23 ignored, incl.
    the composite golden-KAT byte-equivalence + adversarial
    tamper-rejection + CRIT-1). Behavior-preserving: gating is
    identity on active rows, and the gated columns are zero +
    unread cross-row on inactive rows, so no honest/adversarial
    verdict changes. ≈ 192 SX columns are now overlay-eligible.
  - **O0-Stage-2 — LANDED.** Gate the `blake3_round` AIR so SX data
    is vacuous to it on the overlaid rows. The de-risk found the
    BLAKE3 chip already partially gated — `verify_init_state`
    (`is_new_blake`), `finalize_blake` + `CV_OUT` (`is_last_round`)
    use positive selectors that are 0 on SX rows. Only
    `verify_round` (the bulk) needed a gate. **Key de-risk finding:**
    StripeXor activity is co-located on the matmul sweep
    (`composite_trace.rs` — an SX row *is* a matmul row), so an SX
    row already carries the **pinned** `IS_RESET_CUMSUM` /
    `IS_UPDATE_CUMSUM` selectors. The `verify_round` round-active
    gate now subtracts those matmul selectors — on **both** the
    current and next row, since `verify_round` is cross-row (reads
    `next.STATE0`). No new `SX_IS_ACTIVE` pin, **no CRIT-1
    program-pin change, and no degree bump** (each gate factor stays
    degree 1; the gate stays degree 2) — D3's L1-regen concern never
    arose. `chips/blake3/chip.rs`; composite-only via the optional
    `Blake3Offsets.round_gate_excl` field.
- **Stage O1.** Classify every chip per-row-stateless vs stateful
  (cross-row register/accumulator). Only stateless chips are
  whole-region overlay-eligible. Stateful chips (`sx_stripe` `XR`,
  `matmul_tile` `CUMSUM`, `fold`) can at most overlay their
  *stateless sub-columns* — a finer, more delicate sub-column
  overlay deferred until O0+O2 prove out.
- **Stage O2 — LANDED (first application).** `composite_layout.rs`
  re-points `SX_IN_BITS_START` (128 columns) onto
  `BLAKE3_ROUND_START`; the SX layout chain skips `SX_IN_BITS`
  (`SX_XR` follows `SX_IN` directly) and `TOTAL_TRACE_WIDTH` drops
  by 128 (2103 → 1975). The trace-gen needed **no change** — it is
  offset-driven, and StripeXor (matmul-sweep rows) and BLAKE3
  (blake rows) write disjoint rows, so the shared columns are never
  double-written. Layout contiguity + inventory tests updated; a new
  pin asserts `SX_IN_BITS` stays within the `blake3_round` region.
- **Stage O2+ — residual (roll forward).** The remaining
  O0-Stage-1-gated SX runs (`SX_XR_SEL_BITS` 32, `SX_NEW_SEL_BITS`
  32, `SX_IN` 4; and `SX_Q` 32 once gated) alias further
  `blake3_round` sub-windows by the identical mechanism, for ≈ −100
  more columns. Add a dedicated overlay-tamper adversarial test.

R1 status: Path C **and** the Path A column-overlay are landed —
**not** a precise-residual outcome. Landed + committed + 371-test
regression green: **Path C** (§5c — SX_Q, −32 cols, `074aa0b`) and
the **Path A column-overlay** (O0-Stage-1 SX gating + O0-Stage-2
`verify_round` matmul-gating + O2 `SX_IN_BITS` alias, −128 cols,
`TOTAL_TRACE_WIDTH` 2103 → 1975). The original §7 premise ("overlay
`sx_stripe` first") was corrected by the de-risk (D1/D2/D3) to the
landed mechanism above; rolling the overlay across the remaining
gated SX runs is the only residual.

## 6. Files

- `crates/ai-pow-zk/src/composite_layout.rs`: added
  `inner_air_column_inventory` test (the exhaustive 15-group
  accounting; the measurement this analysis rests on).
- _This doc._
