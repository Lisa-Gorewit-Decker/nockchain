> _Created **2026-05-17** · last updated **2026-05-17** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# M-S2 — G3a + G3b Implementation Design (boundary-predicate parameterization + segment schedule)

> **⛔ DEFERRED (2026-05-17, maintainer γ decision).** A Pearl
> implementation+paper evaluation (`M_S2_PEARL_EVALUATION.md`)
> established that **Pearl does NOT segment** — it caps
> parameters (§4.8: `k ≤ 2¹⁶`, `k(h+w) ≤ 2²²`) so one tile = one
> STARK, and recurses only *vertically* for ≤65KB-cert
> compression. The carry-vector segmentation this document
> designs has **no Pearl precedent**. **Track-A PROD now pursues
> the Pearl-faithful P-A/P-B/P-C path** (param caps + raised
> Layer-0 ceiling + vertical-recursion certificate;
> `HIGH2_2_DESIGN.md` §7). **This G3a/G3b/G3c design is
> preserved but DEFERRED** — implement *only if* a concrete load
> **beyond Pearl's `k = 2¹⁶` envelope** is ever required. Do not
> start coding G3a. D1–D6 below remain the locked decisions *for
> the G3 path should it ever be revived*.
>
> **Status:** DESIGN COMPLETE — **decisions D1–D6 LOCKED
> 2026-05-17** (maintainer-approved). Implementation not started;
> this is now the implementation-ready, fully-decided design.
> **Locked path:** D1 = flat `Γ` as PIs · D2 = useful-work+fold
> only (`mhash` identity-carried) · D3 = hybrid pin
> (CONTROL_PREP bits/one-hots + dedicated `SEGMENT_IDX` col) ·
> D4 = arbitrary-row cuts, staged (aligned first) · D5 = BLAKE3
> `program_root`/`block_anchor` · D6 = keep `STARK_ROW_IDX`
> per-segment-local + new `GLOBAL_ROW` in `Γ`. See §10 for the
> full table; the body already assumes these.
> **Upstream contract (authoritative where it conflicts):**
> `G3_RECURSION_AGGREGATION.md` §4 (carry vector `Γ`), §5
> (aggregation tree — G3c only), §6 (`PROGRAM_ROOT`), §12
> (G3a/G3b/G3c acceptance criteria); `G3_RECURSION_AUDIT.md`
> (P0–P6 — all gate **G3c**, none gate G3a/G3b);
> `HIGH2_2_DESIGN.md` §4.C.4-G3 / §4.C.11.
> **This document** is the concrete, code-level implementation
> design for **G3a + G3b only** (the M12-independent, Layer-0-only
> substrate) and surfaces the decisions the maintainer must make
> before coding begins. G3c (the recursion verifier) is out of
> scope here.
> **Predecessor:** M-S1 (§4.C sweep-input ↔ declared-store
> multiset binding) is **DONE & committed `3feae98`**. G3 is
> independent of M-S1; nothing here depends on or changes M-S1.

---

## 0. Why M-S2, and what "done" means

Today the §6(b) useful-work chain (matmul sweep → StripeXor →
Fold → JACKPOT_MSG → C4 → HASH_JACKPOT) must fit in **one**
Layer-0 STARK (`MIN_STARK_LEN = 8192` rows). True-PROD loads
(`k/r = 64`, sweep ≈ 2²⁰ rows) overflow it, so PROD currently
falls to the legacy `compute_tile_trace` path with the §6(b)
keystone gated **off** via the verifier-set `sx_bound` flag —
sound (CRIT-1 + keystone + §6(a) hold) but the in-circuit
matmul-truth guarantee does not reach PROD scale (the **G4**
Pearl-spot-check interim covers it meanwhile).

**G3** closes this by *segmenting* the chain: split the `T`-row
computation into `N` bounded **segments**, prove each as an
ordinary Layer-0 STARK, and (G3c) recursively aggregate. The
per-row constraints are **unchanged**; only the **boundary
predicates** (what a chip asserts on its first/last row) become
*parameterized* by a small verifier-fixed per-segment descriptor.

- **G3a** = parameterize the boundary predicates + thread the
  carry vector `Γ` as public carry-in/out. Deliverable: a
  **multi-segment-capable Layer-0**. Single-segment default is
  **bit-identical to today**.
- **G3b** = derive the segmentation *schedule* from params and
  pin each segment's program (`canonical_segment_program`,
  `program_root`) so a multi-segment prover cannot swap/reorder
  segment programs (CRIT-1 extended across the schedule).
- **G3c** (NOT here) = the recursion verifier that checks every
  segment proof + the `Γ` chaining + `program_root` + segment
  count/order. Gated on P0–P6.

**Definition of done for M-S2:** the §12 acceptance criteria —
(i) zero regression with the default `SegmentPI` (proofs
**bit-identical** to today), (ii) a 2-segment split test, (iii)
arbitrary split points (mid-sub-block-run / mid-chunk /
mid-fold) all verify with debug-assertions ON; plus G3b's
`program_root` determinism / tamper-rejection / `N=1`
continuity. Soundness meanwhile held by CRIT-1 + §4.D keystone +
§6(a) + §6(b) + M-S1 (none of which this touches).

---

## 1. Invariants (must not be violated)

1. **`N = 1` ≡ today, bit-for-bit.** The default `SegmentPI`
   (`n_segments = 1`, `role = First ∧ Final`, `gamma_in = 0`)
   must produce a proof **byte-identical** to the current
   `composite_prove_pinned_logup` output for the same trace.
   This is the zero-regression contract and the single most
   important invariant. Every predicate change is of the form
   `old_predicate` → `is_first · old_first ⊕ (1−is_first) ·
   (state == gamma_in)`, which collapses to `old_predicate` when
   `is_first = 1 ∧ gamma_in = 0`.
2. **Soundness lives in the Pinned AIR only.** Changes go in
   `CompositeFullAirWithLookupsPinned` / `CompositeFullAirPinned`
   and the trace generator. The unit `CompositeFullAir` keeps
   cumsum/jackpot as independent PIs so the ~300 constraint-logic
   tests stay green.
3. **No new probabilistic gap, no trusted setup.** G3a/G3b add
   *equality* constraints only (carry threading, pinned
   descriptor). The error budget is unchanged until G3c.
4. **SNARKs are NOT byte-equivalent across chains; only the
   mineable unit is.** Segmentation is internal to the SNARK; it
   does not touch the plain `TileState`/`keyed_hash` anchor.
5. **Width discipline.** Prefer the §6(a) `CONTROL_PREP`-pack
   pattern (zero preprocessed-width cost) over new preprocessed
   columns; the §4.C.8 ~10× trap is *preprocessed* width. A new
   *witness* column is cheaper than a preprocessed one but still
   widens FRI — justify any addition.
6. **debug-assertions-OFF hazard.** `ai-pow-zk`'s test profile
   compiles out p3 `check_constraints`; a per-row boundary-
   predicate bug surfaces only as `OodEvaluationMismatch` at
   verify. Every G3a step must be gated with
   `RUSTFLAGS="-Ctarget-cpu=native -C debug-assertions=on"` as
   well as the fast default loop.
7. **LogUp coupling (M-S1 lesson).** A trace change that alters a
   bus-queried column on boundary rows is coupled to its producer
   via LogUp balance; unit-AIR green ≠ Route-A green. The carry
   threading touches CUMSUM/SX_XR which are *not* bus-queried, but
   `row_idx`/CV-routing **are** — see §9.

---

## 2. The carry vector `Γ` — exact composition for G3a

Per `G3_RECURSION_AGGREGATION.md` §4.1, a register is in `Γ` iff
its first-row value depends on the previous segment. The full
production `Γ`:

```text
Γ_full = {
  cumsum     : [i32; CUMSUM_LEN=4]      // matmul accumulator
  sx_xr      : [i32; STRIPE_MAX=64]     // StripeXor per-stripe register
  fold_state : [u32; JACKPOT_SIZE=16]   // FoldChip M
  mhash_a    : MerkleRunState           // C3 chunk-Merkle running state of A
  mhash_b    : MerkleRunState           // C3 chunk-Merkle running state of B
  row_idx    : u64                      // STARK_ROW_IDX continuity
}
```

`MerkleRunState` ≈ (CV `[u32;8]` + log-depth partial-subtree
stack) — width fixed by `place_matrix_hash_*`'s chunk-Merkle,
≈ `8 + 8·log2(maxchunks)`.

**G3a scope decision (DECISION D2 — see §10).** The matrix-hash
block (`place_matrix_hash_a/b`) is placed **once** at the front
of today's trace and is *not* part of the §6(b) useful-work
chain that overflows at PROD. Two coherent scopes for G3a:

- **D2-A (recommended): useful-work+fold only.** G3a's `Γ` =
  `{cumsum, sx_xr, fold_state, row_idx}` (≈ 4+64+16+1 = **85
  elems**). The matrix-hash stays single-segment (it fits — a
  PROD A/B chunk-Merkle is ≈2²⁰ rows too, but that is a *separate*
  segmentation axis tracked under G3c/M-P1, not G3a). The 2-segment
  split test cuts **inside the useful-work sweep**. `block_anchor`
  (§3) ties the useful-work segments to the one matrix-hash. This
  is the minimal, exhaustively-testable G3a the spec §12
  describes ("take a small TEST_SMALL useful-work chain, cut it").
- **D2-B: full-trace `Γ` incl. `mhash_a/b`.** Larger `Γ`
  (~110–130), the matrix-hash itself becomes segmentable now.
  More general but materially more work and a bigger `Γ`-equality
  surface for G3c, with **no G3a test benefit** (the spec's G3a
  acceptance only exercises the useful-work cut).

The rest of this document assumes **D2-A** unless the maintainer
chooses D2-B in §10. Under D2-A the `mhash_*` slots are reserved
in the `Γ` *type* (so G3c need not reshape it) but are
**identity-carried** (asserted equal in==out) in G3a — a
single-segment matrix-hash.

`Γ` (D2-A) is **85 field-encoded elements**: flat, no
re-derivation of any matmul.

---

## 3. `SegmentPI` / `SegmentRole` — the descriptor

```text
SegmentRole = First | Middle | Final | Solo      // Solo = First∧Final
SegmentPI = {
  seg_index   : u32        // 0-based; pinned (G3b)
  n_segments  : u32        // N; pinned (G3b)
  role        : SegmentRole// derived from (seg_index, n_segments); pinned
  gamma_in    : Γ          // carry entering this segment   (public input)
  gamma_out   : Γ          // carry leaving  this segment    (public output)
  block_anchor: Digest     // H(job_key‖commitment_hash‖hash_a‖hash_b)
}
```

**Which fields are verifier-fixed (pinned) vs public input —
soundness rationale.**

| Field | Binding | Why |
|---|---|---|
| `seg_index`, `n_segments`, `role` | **Pinned** (G3b, CRIT-1-class) | If a prover could choose `role`, a single-segment prover would claim `Final = false` to **skip the §4.D / §6(b) / C2 keystones** and forge a winning proof. `role` MUST be verifier-fixed from trusted params, exactly like the §6(a) fold schedule. |
| `gamma_in` | **Public input** (free at Layer-0; bound across segments by G3c's `gamma_out(k)==gamma_in(k+1)`) | At Layer-0 a leaf cannot know its neighbour; the chain stitch is G3c's obligation. For the G3a split test it is **hand-threaded**. A free `gamma_in` is *not* a Layer-0 hole because a non-final segment's output is not a PoW digest — only the **chained, final** `hash_jackpot` is, and that chain is closed by G3c (or, in G3a tests, by the explicit assertion `seg0.gamma_out == seg1.gamma_in`). |
| `gamma_out` | **Public output** (derived in-circuit from the segment's last useful-work row) | The recursion reads it; in G3a tests we read it off the PI vector. |
| `block_anchor` | **Public input, asserted == H(C1/C3 PIs)** in-circuit | Ties every segment to **one** block so G3c cannot splice segments from different blocks (the "mixed-block splice" obligation). Cheap: one BLAKE3 of the already-present `job_key‖commitment_hash‖hash_a‖hash_b` PIs (reuse the BLAKE3 chip; or defer to G3c — DECISION D5). |

**Default `SegmentPI` (the zero-regression path):** `seg_index =
0, n_segments = 1, role = Solo, gamma_in = ZERO, gamma_out =
(unconstrained / equals the natural end state), block_anchor =
H(...)`. `role = Solo` ⇒ `is_first = is_final = 1` ⇒ every
parameterized predicate collapses to today's predicate.

---

## 4. Boundary-predicate parameterization — concrete changes

The crux. Today the chips assert their cross-segment-stateful
registers are **zero / baseline on the structural first trace
row** and the keystones fire on the **structural last trace
row**. G3a replaces "structural first/last trace row" with
"**logical segment first/last useful-work row**" and "zero" with
"`gamma_in`".

### 4.1 The logical-boundary marker problem

`when_first_row`/`when_last_row` in p3 mean *structural* trace
row 0 / row `H−1`. But:

- The segment's **logical first useful-work row** is *not* trace
  row 0 (the matrix-hash block precedes the sweep — under D2-A
  the matrix-hash is single-segment and lives at the front of
  *every* segment's trace, or only segment 0's — see D2/§7).
- The segment's **logical last useful-work row** is *not* trace
  row `H−1` (on the final segment the jackpot-hash block is the
  last 8 rows; on non-final segments there is **no** jackpot
  block and `gamma_out` must be read from the last *sweep/fold*
  row).

**DECISION D-marker (folded into D3):** mark the logical
boundary rows with **pinned selectors** `SEG_FIRST_ROW` /
`SEG_LAST_ROW` (one-hot, verifier-fixed via the §6(a)
`CONTROL_PREP`-pack pattern — see §6) rather than relying on
structural `when_first_row`/`when_last_row`. This is the §6(a)
discipline already proven for the fold schedule and keeps the
predicate verifier-fixed (a prover cannot move the boundary).
`SEG_FIRST_ROW` on the sweep's first matmul row, `SEG_LAST_ROW`
on the fold chain's last row.

### 4.2 Per-chip changes (all in the Pinned AIR + trace gen)

For brevity write `gin = gamma_in`, `f0 = SEG_FIRST_ROW` (pinned
one-hot), `fN = SEG_LAST_ROW`, `is_final` (pinned bit from
`role`).

**(a) Matmul cumsum recurrence (`MatmulCumsumChip`).** Today the
sub-block-major single threaded chain starts from an implicit
zero carry on its first run (the run-boundary carry is discarded
by the `(1−is_reset)` term). New: on the `f0` row, the entering
carry is `gin.cumsum` instead of 0:
```text
f0 · (CUMSUM_TILE_cur − gin.cumsum) == 0          // was: ... − 0
```
Off-`f0` rows: the existing recurrence is **unchanged**.

**(b) StripeXor register (`StripeXorChip::eval_composite`).**
Today `SX_XR` enters the sweep at 0 (no prior stripe folds). New:
```text
f0 · (SX_XR[s]_cur − gin.sx_xr[s]) == 0   ∀ s∈[0,STRIPE_MAX)
```
The per-row XOR/passthrough transport is **unchanged**.

**(c) FoldChip (`FoldChip::eval_composite`).** Today
`FOLD_STATE` first-fold-row state must be zero (Pearl `M` starts
at 0). New:
```text
f0 · (FOLD_STATE[i]_cur − gin.fold_state[i]) == 0  ∀ i∈[0,16)
```
The rotl13-XOR step is **unchanged**.

**(d) `row_idx` continuity.** `STARK_ROW_IDX` already increments
per row (CRIT-1-pinned, CV-routing key). New: expose
`gin.row_idx` and `gout.row_idx`; assert
`f0 · (STARK_ROW_IDX_cur − gin.row_idx) == 0` so the recursion
can check `gout.row_idx(k) == gin.row_idx(k+1)` (adjacency / no
gap or overlap). **Caveat (§9):** `STARK_ROW_IDX` is a pinned
program column and a CV-routing-bus key — changing its base per
segment interacts with CRIT-1 and the CV bus. Under D2-A the
simplest sound choice is **per-segment-local `STARK_ROW_IDX`
(always 0-based)** plus a *separate* monotone `GLOBAL_ROW`
carried in `Γ` (so CRIT-1/CV-routing are byte-identical to today
and only the new `GLOBAL_ROW` threads). This is recommended;
flagged as DECISION D6.

**(e) `gamma_out` exposure (the last logical row).** On the `fN`
row, drive the public `gamma_out` from the live registers:
```text
fN · (PI(gamma_out.cumsum)      − CUMSUM_TILE_cur)  == 0
fN · (PI(gamma_out.sx_xr[s])    − SX_XR[s]_cur)      == 0
fN · (PI(gamma_out.fold_state)  − FOLD_STATE_cur)    == 0
fN · (PI(gamma_out.global_row)  − GLOBAL_ROW_cur)    == 0
```
(Exposure mechanism = DECISION D1, §5.)

**(f) Keystones gated by `is_final`.** The §4.D keystone
(last-row `JACKPOT_MSG[0..16] == FOLD_STATE`), the §6(b)
keystone (`FOLD_XSTEP == SX_XR[stripe]`), and the C2 difficulty
check **must fire only on the final segment**:
```text
is_final · (JACKPOT_MSG[i] − FOLD_STATE[i]) == 0          // §4.D
is_final · Σ_s FOLD_STRIPE_SEL[s]·(FOLD_XSTEP − SX_XR[s]) == 0 // §6(b)
// C2 (verifier-side): only check HASH_JACKPOT ≤ target if is_final
```
Non-final segments have **no** jackpot-hash block; their
`HASH_JACKPOT` PI is unconstrained/ignored (defined iff
`is_final`, per `AggClaim.hash_jackpot`). The §6(b) keystone is
*already* one-hot/`sx_bound`-gated; this adds the `is_final`
factor. Degree stays ≤ 2 (multiplying a degree-≤1 pinned bit).

### 4.3 Why this is bit-identical at `N=1`

With `role = Solo`: `is_first` is satisfied by the same row that
is structurally first today, `gin = 0` ⇒ `(state − 0) = state`
(the existing zero-baseline assertion), `is_final = 1` ⇒ every
keystone fires exactly as today, `gamma_out` is exposed but
unconstrained-upward (a single segment has no consumer). The
trace generator writes the same values; the program (CONTROL_PREP
+ the new pinned `SEG_FIRST/LAST/role/seg_index` fields, all at
their default) hashes to the same canonical program **iff** the
default-descriptor pack is the zero/identity contribution (the
§6(a) zero-blast-radius property — see §6). ⇒ proofs
byte-identical. This is the single hardest thing to get right
and is the first acceptance gate.

---

## 5. `Γ` exposure mechanism — DECISION D1

`gamma_in` and `gamma_out` must be public (carry-in/out). Two
designs:

- **D1-A — flat `Γ` as public values (~+170 PIs).** Append
  `gamma_in` (85) + `gamma_out` (85) to the PI vector
  (`NUM_PUBLIC_VALUES 60 → ~230`). Matches
  `G3_RECURSION_AGGREGATION.md` §4.1 ("equality is ≈130
  `assert_eq`s in the recursion node" — i.e. `Γ` travels flat
  between layers). G3c reads/equates flat `Γ` directly. Simple,
  no extra in-circuit hashing, no hash-choice coupling. Cost:
  large PI vector (FS transcript longer; the PI commitment is
  cheap but the verifier-fixed-PI bookkeeping grows).
- **D1-B — `Γ`-digest (~+16 PIs).** Expose
  `gamma_in_digest`, `gamma_out_digest` (8 words each); add an
  in-circuit hash `digest == H(flat Γ cells)` on the `f0`/`fN`
  rows (reuse the BLAKE3 chip). Small PI surface. Cost: an extra
  ~85-cell BLAKE3 absorb per boundary **and** G3c must open the
  digest (it needs the flat `Γ` to check chaining anyway, so the
  digest just adds a hash it must re-verify) — i.e. D1-B mostly
  *moves* the ≈130 equalities into a hash without removing G3c's
  need for flat `Γ`. **Couples G3b/G3c to a hash choice** (Tip5
  vs BLAKE3 vs Poseidon2 — see the audit's P1).

**Recommendation: D1-A (flat).** It is what the upstream spec
assumes, keeps G3c's stitch a plain field-equality (its stated
design), and avoids a premature hash-choice coupling that the
G3_RECURSION_AUDIT explicitly flags (P1). The PI-vector growth
is benign at Layer-0 (no per-row cost; the verifier-fixed PI set
is mechanical). **Surface to user** because it sets the G3c
interface and the PI ABI for the whole tree.

---

## 6. G3b — segment schedule, program pinning, `program_root`

### 6.1 `seg_index` / descriptor pinning — DECISION D3

The descriptor fields that MUST be verifier-fixed: `role`
(`is_first`, `is_final`), `seg_index`, `n_segments`,
`SEG_FIRST_ROW`/`SEG_LAST_ROW` one-hots. Two mechanisms:

- **D3-A — extend the §6(a) `CONTROL_PREP` pack.** Bit layout
  today: selectors[0..20], MAT_ID[21..46], FOLD_IS_FOLD@47,
  fold_slot[48..51], fold_stripe[52..57]. Free headroom ≈ bits
  58..62 before Goldilocks `p ≈ 2⁶⁴−2³²+1` (top safe bit ≈ 62).
  **≈5 spare bits.** Enough for `is_first`+`is_final`(2) but
  **NOT** a production `seg_index` (PROD `N` ≈ 2²⁰/8192 ≈ 256 ⇒
  8 bits, and bigger loads need more). Zero preprocessed-width
  cost (the §4.C.8 trap is avoided); zero blast radius for the
  default (all-zero ⇒ byte-identical CONTROL_PREP).
- **D3-B — dedicated pinned `SEGMENT_IDX` preprocessed column
  (+1 preprocessed col, constant across the trace).** Unbounded
  `N`. Costs one preprocessed column width (small, *constant* —
  this is **not** the §4.C.8 trap, which is about *many* / wide
  preprocessed columns scaling with the witness; one constant
  column is ≈ the existing `STARK_ROW_IDX`/`AB_ID_PREP` columns).
  Cleanest for arbitrary `N`.

**Recommendation: hybrid — `is_first`/`is_final` +
`SEG_FIRST_ROW`/`SEG_LAST_ROW` one-hots via the
`CONTROL_PREP` pack (D3-A, ~free, the §6(a) discipline), and
`seg_index`/`n_segments` via a dedicated pinned `SEGMENT_IDX`
column (D3-B) so `N` is unbounded.** The boundary *predicates*
only need the bits/one-hots (hot path, want them free in
CONTROL_PREP); `seg_index`/`n_segments` are only needed by
`program_root`/G3c (a single constant column is fine). **Surface
to user** (width-vs-N trade-off is a maintainer call).

### 6.2 `num_segments` / `segmentation_plan(params)` — DECISION D4

`num_segments(params)` and the per-segment row span must be a
**pure function of public params** (verifier-recomputable, never
from the proof). The §6(b) sweep is sub-block-major:
`for sbi for sbj for step for chunk { matmul_step }`, then the
fold chain (`num_stripes` rows), then (final only) the
jackpot-hash block (8 rows).

- **D4-A — arbitrary-row cuts.** A segment is any contiguous
  `S`-row window of the logical chain; a cut may fall
  mid-sub-block-run, mid-chunk, or mid-fold. **The spec §12
  acceptance (iii) mandates this works.** Most general; the
  boundary predicates in §4.2 already handle it (they thread the
  *register* state, which is well-defined at every row — the
  matmul carry, SX_XR, fold state are all per-row values). The
  only subtlety: a cut mid-`place_useful_work_chain` must thread
  `carry` (the matmul `cumsum`) **and** the `xr[STRIPE_MAX]`
  StripeXor register **and** any partially-accumulated
  sub-block — all are in `Γ` already, so it is sound; the trace
  generator must emit the segment's first row with `is_reset`
  semantics driven by `gin.cumsum` not 0.
- **D4-B — sub-block-run-aligned cuts.** Cut only at
  sub-block-run boundaries (where `cumsum` resets anyway). Far
  simpler `Γ` at the cut (`cumsum` is 0 at a run boundary;
  effectively only `sx_xr`+`fold_state` thread). But it
  **fails acceptance (iii)** as written and constrains
  `segmentation_plan` to run-aligned `S` (runs may not divide
  evenly into 8192 ⇒ wasted rows / variable segment size).

**Recommendation: D4-A (arbitrary-row), but stage it** — land
D4-B-style *aligned* cuts first as G3a.1 (smaller `Γ`-at-cut,
proves the machinery) then generalize to arbitrary in G3a.2
before claiming acceptance (iii). The end state must be D4-A
(spec-mandated). **Surface to user** (affects
`segmentation_plan` and how aggressively to stage).

### 6.3 `canonical_segment_program` + `program_root`

- `canonical_segment_program(params, seg_index)` → the pinned
  program (CONTROL_PREP schedule + `SEGMENT_IDX`) for segment
  `seg_index`, rebuilt **witness-free** by the verifier exactly
  as today's single `extract_program` but with the descriptor
  fields set per the schedule.
- `program_root(params)` = a Merkle root over
  `[canonical_segment_program(params, k) for k in 0..N]`. Hash:
  **BLAKE3** (we already have the in-circuit BLAKE3 chip + the
  C3 chunk-Merkle infra; consistent with HASH_A/HASH_B; avoids
  the Tip5↔Poseidon2 recursion-hash question which is a G3c/M-S4
  concern — DECISION D5 only matters if the maintainer wants the
  root to be recursion-friendly *now*).
- **`N = 1` continuity:** `program_root(params)` with `N=1` MUST
  equal the existing single program's commitment (a 1-leaf
  Merkle root = `H(leaf)`; define so the existing
  `composite_verify_pinned_logup` path is recovered exactly).
- **Tamper/reorder rejection test:** a per-segment program with
  a swapped/reordered/forged descriptor fails Merkle membership
  against `program_root(params)`.

`program_root` is consumed by **G3c** (CRIT-1-across-tree). G3b
only needs to *produce* it deterministically + the membership
machinery + the `N=1` continuity. It does **not** wire it into
Layer-0 verification (that would be G3c).

---

## 7. Per-segment trace layout (under D2-A)

```
single-segment (today / N=1):
  [ matrix-hash A | matrix-hash B | key-pin | sweep | fold | jackpot-hash ]

multi-segment (N>1), D2-A:
  segment 0  (First) : [ matrix-hash A|B | key-pin | sweep[0..s0] | (fold?) ]   role=First
  segment k  (Middle): [ key-pin? | sweep[sk-1..sk] | (fold?) ]                 role=Middle
  segment N-1(Final) : [ sweep[..end] | fold | jackpot-hash ]                   role=Final
```

Open layout questions (engineering, not user-facing — recorded
for the implementer):

- Does every segment re-place the matrix-hash + key-pin (so C1/C3
  PIs and `block_anchor` are checkable locally on every segment),
  or only segment 0 with `block_anchor` carried in `Γ`/PIs? **D2-A
  recommendation:** every segment carries `block_anchor` as a PI
  asserted `== H(job_key‖commitment_hash‖hash_a‖hash_b)`; the
  *matrix-hash block itself* is placed only on segment 0 (it is
  single-segment under D2-A), and segments k>0 carry `hash_a/b`
  as ordinary PIs (already in the PI set) so `block_anchor` is
  still checkable without re-hashing the matrices. This keeps
  non-segment-0 segments cheap.
- Fold chain placement when a cut falls mid-fold: the fold rows
  are part of the logical chain; `fold_state` ∈ `Γ` so a mid-fold
  cut threads the partially-folded `M`. The `FOLD_STRIPE_SEL`
  one-hot schedule (§6(a)/G2) must be pinned per-segment-local.

---

## 8. Test plan (operationalizing §12 acceptance)

All under the fast loop
(`RUSTFLAGS="-Ctarget-cpu=native" cargo test -p ai-pow-zk --lib`,
parallel default-on) **and** a `-C debug-assertions=on` pass.

**G3a:**
1. **Zero-regression (the gate).** `composite_proof::tests::*`
   (`routea_*`, `crit1_*`, `high2_*`, `noised_store_*`,
   `high2_2_swept_tile_not_in_store_rejects`,
   `high2_2_fold_chain_pinned_logup`) + `ai-pow --features zk`
   e2e: all green, **and** assert the produced proof bytes are
   **identical** to a pre-M-S2 baseline for a fixed trace
   (snapshot the `BatchProof` serialization, compare). This
   proves the default `SegmentPI` is bit-identical.
2. **2-segment split, aligned (G3a.1).** TEST_SMALL useful-work
   chain, cut at a sub-block-run boundary; hand-thread `Γ`;
   `prove_segment` both halves under `CompositeFullAirWith
   LookupsPinned` with the per-segment `SegmentPI`; assert
   `seg0.gamma_out == seg1.gamma_in` and that segment 1's
   `HASH_JACKPOT` (it is `Final`) equals the single-segment
   `high2_2_fold_chain_pinned_logup` digest for the same inputs.
3. **2-segment split, arbitrary (G3a.2 — acceptance (iii)).**
   Repeat (2) with the cut at: mid-sub-block-run, mid-`⌈r/16⌉`
   chunk, mid-fold-chain. Each must verify, debug-assertions ON.
4. **Adversarial:** forged `gamma_in` on segment 1 (≠
   `seg0.gamma_out`) ⇒ the split test's explicit equality fails
   (this is the *test harness* catching it; the *in-circuit*
   rejection is G3c's job — document this boundary clearly so it
   is not mistaken for a Layer-0 hole); `role=Final` flipped to
   `Middle` on the single segment ⇒ keystones don't fire ⇒
   **must be rejected by the pinned program mismatch** (CRIT-1:
   the verifier rebuilds `role` from params; a proof with the
   wrong `role` pin fails against the canonical program — this
   IS a Layer-0 check and must pass as a red-team test).

**G3b:**
5. `program_root(params)` deterministic & verifier-recomputable
   (recompute twice, equal; recompute from params only).
6. Tampered/reordered per-segment program fails Merkle
   membership.
7. `N=1` ⇒ `program_root` == existing single-program commitment
   (continuity); `composite_verify_pinned_logup` path unchanged.

---

## 9. Risks & gotchas

- **`STARK_ROW_IDX` is load-bearing.** It is CRIT-1-pinned **and**
  a CV-routing-bus key. Re-basing it per segment would change the
  canonical program AND unbalance the CV-routing LogUp (the M-S1
  lesson, one level up). **Mitigation (D6):** keep
  `STARK_ROW_IDX` per-segment-local & identical to today; add a
  *separate* `GLOBAL_ROW` (witness col, monotone, threaded in
  `Γ`) for adjacency. This isolates the change from CRIT-1/CV.
- **The default-pack must be the zero contribution.** If the new
  `SEG_*`/`role` CONTROL_PREP bits are non-zero for the default
  `Solo` descriptor, every row's CONTROL_PREP changes ⇒ the
  canonical program hash changes ⇒ **not** byte-identical ⇒
  zero-regression fails. The §6(a) `pack_control_prep_full`
  zero-blast property must extend to the new fields (default =
  all-zero ⇒ identical pack). Encode `role=Solo` as the
  *all-zero* descriptor (i.e. `is_first`/`is_final` are
  represented so that Solo packs to the same bits as today's
  no-op — likely `is_first_bit = 0` meaning "structural first",
  with the parameterized predicate written so `bit=0` ≡ today).
  This inversion must be designed carefully (it is the analogue
  of "`is_fold=0/slot=0` ⇒ contributes 0").
- **debug-assertions-OFF hazard** (Invariant 6) — every step.
- **LogUp coupling** — the carry threading touches CUMSUM/SX_XR
  (not bus-queried) and `GLOBAL_ROW` (new, not bus-queried) ⇒
  low risk; but the `is_final`-gating of the jackpot block
  changes which rows carry `IS_HASH_JACKPOT`/`IS_MSG_MAT` on
  non-final segments — verify the `noised_packed`/CV buses still
  balance on a non-final segment (no jackpot block ⇒ no C4
  query; should be inert, but **test it**, per the M-S1 lesson).
- **Prover-cost** of N segments is N× one Layer-0 + (G3c)
  recursion; M-P1/Track-B own the economics. G3a/G3b add no
  Layer-0 per-row cost.

---

## 10. Decisions for the maintainer (consolidated)

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **D1** | `Γ` exposure | A: flat ~+170 PIs (spec-native, G3c stitch = field-eq) · B: 2 digests +in-circuit hash (small PI, couples to a hash choice) | **A (flat)** — matches upstream, no premature hash coupling |
| **D2** | G3a `Γ` scope | A: useful-work+fold only, matrix-hash single-segment (`mhash` identity-carried) · B: full-trace incl. `mhash_a/b` | **A** — minimal, exactly the spec's G3a test surface |
| **D3** | Descriptor pinning | A: all in `CONTROL_PREP` pack (free, `N≤32`) · B: dedicated `SEGMENT_IDX` col (unbounded `N`, +1 const preproc col) · Hybrid | **Hybrid** — bits/one-hots in CONTROL_PREP, `seg_index/N` in a dedicated col |
| **D4** | Cut granularity | A: arbitrary-row (spec-mandated end state) · B: sub-block-aligned only (simpler, fails acc. (iii)) | **A, staged** (land aligned first, generalize before claiming (iii)) |
| **D5** | `block_anchor`/`program_root` hash | BLAKE3 (have the chip, C3-consistent) · Tip5/Poseidon2 (recursion-friendly now) | **BLAKE3** (defer recursion-hash to G3c/M-S4 per the audit) |
| **D6** | `row_idx` re-basing | A: keep `STARK_ROW_IDX` local + new `GLOBAL_ROW` in `Γ` · B: re-base `STARK_ROW_IDX` per segment | **A** — isolates CRIT-1/CV-routing (the M-S1-class hazard) |

**ALL LOCKED 2026-05-17 (maintainer-approved): D1=A · D2=A ·
D3=Hybrid · D4=A(staged) · D5=BLAKE3 · D6=A.** The body of this
document already assumes exactly this path; no option branches
remain open. Implementation proceeds per §11.

---

## 11. Phased landing plan

Unlike M-S1 (which the §4.C.11 audit proved must land
*atomically* because of LogUp coupling), G3a/G3b **can** be
staged — the parameterization collapses to a no-op at the
default descriptor, so each step is independently
zero-regression-testable:

- **G3a.0 — types & default.** Add `Γ`, `SegmentPI`,
  `SegmentRole`; thread the default `Solo`/`ZERO` everywhere;
  prove **byte-identical** (acceptance (i)). *No predicate
  changes yet — pure plumbing.* Gate: snapshot-equality test.
- **G3a.1 — predicate parameterization + aligned split.** §4.2
  (a)–(f); the `CONTROL_PREP`/`SEGMENT_IDX` pin (D3); the
  aligned 2-segment split test (D4-B-style). Gate: acceptance
  (i)+(ii).
- **G3a.2 — arbitrary cuts.** Generalize the trace generator +
  the `is_reset`/carry handling for mid-run/mid-chunk/mid-fold;
  acceptance (iii), debug-assertions ON. Gate: full §8 G3a
  suite.
- **G3b.1 — schedule + `canonical_segment_program`.**
  `num_segments`/`segmentation_plan`/`canonical_segment_program`;
  `N=1` continuity.
- **G3b.2 — `program_root` + membership.** Merkle root, tamper
  tests, `N=1`==single-program commitment.

Each phase: commit with the `Co-Authored-By: Claude Opus 4.7
(1M context) <noreply@anthropic.com>` trailer; push only when
asked; `Plonky3-recursion/` stays git-untracked; never bake
`target-cpu=native` workspace-wide.

After M-S2: **M-S3** (P0 vendor Plonky3-recursion) → **M-S4**
(P1 `tip5-circuit-air`) → **M-S5** (G3c — the recursion verifier;
gated on P0–P6) per `HIGH2_2_DESIGN.md` §7 Track A.

---

## 12. Cross-references

- Upstream contract: `G3_RECURSION_AGGREGATION.md` §4, §6, §12.
- Audit (P0–P6, all gate **G3c** only):
  `G3_RECURSION_AUDIT.md`.
- Roadmap & inflections: `HIGH2_2_DESIGN.md` §7 Track A
  (M-S2 row), §4.C.4-G3, §4.C.11 (M-S1 — done).
- §6(a) pinning pattern this reuses: `chips/control.rs`
  `pack_control_prep_full` (FOLD_IS_FOLD@47 / fold_slot@48 /
  fold_stripe@52; ~5 spare bits to 62).
- PI ABI: `composite_public.rs` (`NUM_PUBLIC_VALUES = 60`,
  `PI_*_OFFSET`).
- M-S1 (predecessor, done): commit `3feae98`,
  `HIGH2_2_DESIGN.md` §4.C.11.
