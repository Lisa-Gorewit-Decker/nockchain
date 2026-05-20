# Path B Stage B1 — verifier-AIR reduction map

**Date:** 2026-05-20
**Source data:** Stage B0 inventory (commit landing this work)
**Status:** Design document. No invasive code changes yet.

## Methodology

For each candidate reduction we record:
- **What it does** in the verifier circuit (the soundness role).
- **Why it's currently emitted** (the constraint it enforces).
- **Reduction proposal** (remove / merge / lazy-compute / hoist).
- **Soundness equivalence argument** — REQUIRED. No
  hand-waves; must be a mathematical/structural argument.
- **Estimated L1 saving** (cell count × bytes-per-cell heuristic).
- **L2 cascade** (cells removed at L2 because the verifier
  circuit is simpler).
- **Implementation effort** (files touched, lines of code).
- **Risk** (low/medium/high — soundness, not implementation).
- **Sort key:** `estimated_l2_saving / (effort × risk)`.

## Reduction families (priority order)

### Family A — Alu-row reductions (highest leverage by row count)

The Alu primitive table has 6,851 rows (84% of L1 row count) at
the post-Phase-0 production config. Alu rows are emitted by the
verifier circuit for every arithmetic operation it performs. The
main sources, in order of expected contribution:

#### A.1 — FRI fold-equation rows

**What:** Each FRI fold step computes the standard FRI invariant
`folded(z) = ((folded(βz) + folded(-βz))/2) + β · ((folded(βz) - folded(-βz))/(2z))`
(or a higher-arity variant when `max_log_arity > 1`). At Tier B
+ Phase 0 (`lb=4 nq=20 mla=3`), we have ~20 queries × ~3-4 fold
phases ≈ 60-80 fold-step invocations, each emitting ~20-40
Alu ops.

**Why emitted:** the FRI verifier asserts the fold-chain
consistency at every query × phase; each such assert requires
arithmetic.

**Reduction proposal:** custom FRI-fold AIR. Emit a single
DEDICATED AIR (one row per fold step) with constraints that
implement the fold equation directly, instead of generic Alu
ops. A dedicated AIR with 5-10 columns per fold step beats
~30 generic Alu rows.

**Soundness equivalence:** the custom AIR's constraints must be
algebraically equivalent to the unfolded Alu sequence. Provable
by symbolic constraint evaluation + tamper-reject tests.

**Estimated saving:** L1 Alu rows from ~6,851 to ~4,000-5,000
(remove ~1,500-2,000 fold-related Alu rows; replace with ~80
new fold-step AIR rows). Net: ~−15-25% L1 Alu cells.

**L2 cascade:** L2's Alu shrinks more (verifier-of-L1 has 60-80
fold-step rows to verify; if those are in a custom AIR, L2's
recomposition logic simplifies). Hypothesis: ~−20% L2 cells.

**Implementation effort:** medium-high — new AIR type, register
with `BatchStarkProver`, update `verify_p3_uni_proof_circuit` /
`verify_fri_circuit` to use it. ~500-1000 LOC + tests. **R1
soundness-critical invasive.**

**Risk:** medium — the custom AIR's constraint set must be
proven equivalent to the existing constraint set. KAT-first
discipline mandatory.

**Sort key:** high impact (L1 −15-25%, L2 −20% predicted) / high
effort. Top candidate for a focused multi-day session.

#### A.2 — OOD constraint evaluation

**What:** `air.eval_folded_circuit(...)` evaluates the inner
Tip5-L0 AIR's constraint set at the out-of-domain point `zeta`,
batched against the random challenge `alpha`. Each constraint
the inner AIR has emits Alu rows for the polynomial evaluation.

**Why emitted:** verifying that `Σ alpha^i · constraint_i(zeta)
= quotient(zeta) · vanishing(zeta)`. Required by STARK soundness.

**Reduction proposal:** **NOT REDUCIBLE without re-architecting
the inner Tip5-L0 AIR.** The work scales with the inner AIR's
constraint count; that's a downstream fix (Path B for the inner
AIR, which is a separate milestone).

**Soundness equivalence:** N/A (no reduction proposed).

**Sort key:** zero. Mark not-reducible-at-this-layer.

#### A.3 — Lagrange basis evaluations

**What:** the verifier computes Lagrange basis values
`L_i(zeta)` for each opened point. Each basis evaluation = a few
divisions + multiplications.

**Why emitted:** required to interpolate polynomial values from
their coefficients via the basis.

**Reduction proposal:** **constant hoisting** — if some basis
denominators are deterministic (independent of `zeta`), hoist
them to the preprocessed trace. The preprocessed trace is
committed at setup; verifier knows its values; doesn't count
toward L1 row counts.

**Soundness equivalence:** preprocessed-trace values are bound
by the preprocessed commitment (already verified). Moving a
constant from Alu to preprocessed-trace doesn't change what's
constrained, only where it's stored.

**Estimated saving:** L1 ~50-100 Alu rows removed. Small (~1%
of Alu).

**L2 cascade:** proportional ~1% L2 reduction.

**Implementation effort:** low — identify the constants, hoist
them, prove the equivalence. ~50-100 LOC.

**Risk:** low — preprocessed-trace hoisting is a well-known
soundness-neutral optimization.

**Sort key:** low impact / low effort = medium attractiveness.
Worth doing as a warm-up reduction to validate the methodology.

#### A.4 — Quotient polynomial recomposition

**What:** `recompose_quotient_from_chunks_circuit` reconstructs
`quotient(zeta)` from its `quotient_degree` chunks at `zeta`.
Each chunk contributes an Alu row.

**Why emitted:** required to compare `quotient(zeta)` against
the LHS `Σ alpha^i · constraint_i(zeta) / vanishing(zeta)`.

**Reduction proposal:** marginal — quotient recomposition is
already tight; only ~quotient_degree rows. Skip.

**Sort key:** zero. Mark not-reducible.

### Family B — `tip5_perm` NPO reductions (highest leverage by cell count)

The `tip5_perm` NPO has 881 packed rows × ~886 columns ≈ 780K
cells — likely the biggest cell-count contributor (vs Alu's
~343K cells at ~50 cols). Cell-by-cell, this table is the
dominant column-cost in the verifier circuit. Reduction
proposals:

#### B.1 — Batch Fiat-Shamir absorbs

**What:** the challenger absorbs each commitment one perm at a
time. With ~5 commitments (trace, quotient_chunks, random?,
preprocessed?, openings), there are ~5 perm calls just for
challenger absorbs.

**Why emitted:** Fiat-Shamir transcript binding — each absorb
must commit to all-prior-absorbed.

**Reduction proposal:** batch absorbs that occur "atomically"
(no intervening challenge squeeze) into a single longer absorb
+ single perm.

**Soundness equivalence:** sponge absorption is associative as
long as no squeeze interrupts; batching adjacent absorbs into
one perm produces the same final sponge state. Provable via the
Tip5 sponge spec.

**Estimated saving:** ~5-10 perm calls removed = ~5-10
`tip5_perm` rows × 886 cols ≈ ~5-9K cells. **Tiny relative to
~780K total tip5_perm cells.**

**Sort key:** very low impact / low-medium effort. Skip.

#### B.2 — MMCS path batch hashing

**What:** Each Merkle authentication path requires
log_path_height hash operations (Tip5 compress). At Tier B + cap=3
with ~7-level trees, each query × commitment = 4 hashes.
20 queries × ~3 commitments × 4 hashes/path = ~240 perm calls.

**Why emitted:** Merkle path verification — each level requires
a hash of `(left, right)`.

**Reduction proposal:** **Salt-pre-hash** — if multiple queries
share Merkle paths (which they do when query indices share
prefixes), the shared prefix hashes can be computed once. But
this is complex and the FRI random queries are designed to
distribute uniformly, so prefix-sharing is rare.

Alternative: **wider hash absorption** — if MMCS could absorb
more elements per call, fewer calls per path.

**Soundness equivalence:** for shared prefix hashes, the
sharing is observationally equivalent. For wider absorption,
requires a different MMCS instance (substrate change).

**Estimated saving:** prefix-sharing: ~10-20% reduction in
MMCS perms (~25-50 perms). Wider absorption: substrate change
out-of-scope.

**Sort key:** medium impact / high effort (FRI verifier change).
Defer.

#### B.3 — FRI commit-phase hash sharing

**What:** Each FRI commit-phase commitment hashes the
folded polynomial. With ~3-4 fold phases × 20 queries (each
verifying its query path through every commitment), there are
~60-80 commit-phase perm calls.

**Why emitted:** each FRI commitment must be re-derived in the
verifier for soundness.

**Reduction proposal:** none obvious — these are intrinsic to
FRI.

**Sort key:** zero. Mark not-reducible-at-this-layer.

### Family C — Recompose NPO reductions

Total recompose rows: 96 + 187 = 283 packed rows (3.5% of
total). Cell count: ~283 × ~10 = ~3K cells (~0.4% of total).
**Negligible.**

Skip — even maximal recompose reduction would save <1% of L1.

### Family D — Preprocessed-trace hoisting (cross-cutting)

**What:** any constant the verifier circuit currently computes
inside Alu (via `define_const` followed by arithmetic) could be
pre-baked into the preprocessed trace.

**Why proposed:** preprocessed-trace cells don't count toward
main-trace row counts (they're committed at setup, verified
once via the preprocessed_commit). Moving constants there is
soundness-neutral.

**Soundness equivalence:** the preprocessed commitment is
verified inside the verifier circuit (part of the FRI
verification). Constants moved to preprocessed are bound by
that commitment, equivalent to direct embedding in the Alu
constraints.

**Estimated saving:** depends on how many compile-time
constants live in the verifier circuit. Educated guess: ~5-10%
of Alu cells. Compounds with A.1 (custom FRI-fold AIR).

**Implementation effort:** low-medium per constant. Requires
identifying candidates (instrumentation work).

**Risk:** low — well-understood optimization pattern.

**Sort key:** medium impact / low effort. Strong warm-up
candidate.

## Sorted candidate list (top → bottom)

| Rank | Candidate | L1 cells saved | L2 cascade | Effort | Risk |
|---|---|--:|--:|--:|--:|
| 1 | **A.1 Custom FRI-fold AIR** | 50K-150K (~−15-25%) | ~−20% L2 | high | medium |
| 2 | **D Preprocessed-trace hoisting** | 15K-50K (~−5-10%) | ~−5-10% L2 | low-medium | low |
| 3 | **A.3 Lagrange basis hoisting** | ~5K (~−1%) | ~−1% L2 | low | low |
| 4 | **B.1 Batch Fiat-Shamir absorbs** | ~5-9K (~−1%) | ~−1% L2 | low-medium | low |
| 5 | **B.2 MMCS prefix sharing** | ~25K (~−3%) | ~−3% L2 | high | medium |
| 6 | A.2 OOD constraint reduction | N/A | N/A | n/a | n/a |
| 7 | A.4 Quotient recomposition | <1K | <1% | n/a | n/a |
| 8 | B.3 FRI commit-phase hashes | 0 | 0 | n/a | n/a |
| 9 | C Recompose | <3K (<1%) | <1% | low | low |

## Stage B2 recommendation

**Start with D (preprocessed-trace hoisting)** as a warm-up:

- Low risk, low-medium effort, validated methodology.
- Even modest savings (5-10%) cascade to L2 (~5-10% there too).
- Builds confidence + tooling for the more invasive A.1.
- R1-compliant: validated equivalence argument is straightforward.

**Then attempt A.1 (custom FRI-fold AIR)** as the main work:

- Highest single-reduction impact (predicted ~−15-25% L1 cells,
  ~−20% L2 cells).
- Requires a focused multi-day session.
- KAT-first discipline mandatory: every fold-step constraint
  must be proven equivalent before invasive landing.

**Defer** B.1, B.2, B.3, A.3 to follow-on sessions if A.1 lands.

## Major decisions to bubble up

### Decision 1 — Custom FRI-fold AIR or stay generic?

A.1 requires landing a new dedicated AIR in the recursion crate.
This is a vendored-Plonky3 patch (we'd diverge from upstream).
Trade-off:
- Pro: ~−20% L2 cascade, biggest single-step Path B win.
- Con: maintenance burden (carry the patch forward); audit
  surface (the custom AIR is new soundness-critical code).

If declined: skip A.1, do only D + maybe A.3. Path B caps out
at ~−10% rather than ~−25-35%.

### Decision 2 — How far to drive Phase B in this session?

The user goal says "test exhaustively, integrate completely."
Realistic options within reasonable session scope:
- **Option α**: land D (preprocessed hoisting) only. Concrete
  win, ~−10%. ~1 day.
- **Option β**: land D + start A.1 design + KAT-first de-risk.
  ~2-3 days; A.1 implementation is the bulk.
- **Option γ**: full A.1 + D + A.3. ~1 week. Risk of partial
  landings hitting R1 walls.

**Recommended:** Option α (land D); attempt A.1 design + KAT
in a separate focused session. R1 honest: don't fake "complete
Path B" — the validated subset (D) + precise residual (A.1
design landed, implementation deferred) is the correct R1
outcome.

## Honest residuals

- **Per-Alu-source attribution** — we don't have a row-level map
  of "which lines of verifier code produce which Alu rows."
  Without it, the A.1 / A.3 candidates have wide uncertainty.
  Future B0 refinement: instrument the CircuitBuilder to tag
  emitted Alu rows by source location.
- **Cell counts not yet measured** — Stage B0 only has row
  counts. The cell-count refinement is landing concurrently
  (`test_path_b_b0_inventory.rs` v2 reports widths × rows).
  Will update this map once measured.
- **L2 inventory not done** — cascading-effect predictions are
  educated guesses; need an L2-side B0 to validate.
- **`opening_proof` 85% slice opacity** — FRI prover doesn't
  expose per-query byte counts; some savings in tip5_perm
  reduction may not actually shrink opening_proof bytes
  proportionally.

## Cross-references

- B0 inventory: [`2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md`](2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md)
- Recommendation hierarchy: [`2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md`](2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md) § 6
- Investigation results: [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md)
