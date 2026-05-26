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

- **Per-Alu-source attribution** — RESOLVED via the
  `--features profiling` work in commit `87a78e1`. Per-scope
  op counts now available; see § "Profiled attribution
  results" below.
- **Cell counts** — RESOLVED in B0 refinement. Production L1
  cell hierarchy: Alu 1.24M (60%) > tip5_perm 829K (40%) >
  everything else < 1%. The earlier B1 educated guess
  ("tip5_perm dominates by cells") was WRONG — Alu's per-row
  width is wider than estimated (70 main + 111 prep = 181
  cols, not ~50). Alu reduction = highest-impact lever.
- **L2 inventory not done** — cascading-effect predictions
  remain educated guesses; need an L2-side B0 to validate.
- **`opening_proof` 85% slice opacity** — FRI prover doesn't
  expose per-query byte counts; some savings in tip5_perm
  reduction may not actually shrink opening_proof bytes
  proportionally.

## Profiled attribution results (Stage B0 + `--features profiling`)

Per-scope CircuitBuilder op counts (from `test_path_b_b0_inventory`
with `--features profiling`, sorted by total Alu-equivalent ops):

| Scope | Total Alu ops | bool_checks | mul_adds | divs | NPOs |
|---|--:|--:|--:|--:|---|
| **`reconstruct_index_from_bits`** | **3,840** | 1,920 | 1,920 | 0 | — |
| `fri_fold_one_phase` | 720 | 0 | 270 | 90 | — |
| `fri_commit_phase_mmcs` | 720 | 0 | 360 | 0 | 450 Tip5 |
| `precompute_evaluation_points` | 360 | 0 | 180 | 0 | — |
| `fri_reconstruct_evals` | 360 | 0 | 180 | 0 | — |
| `open_input` | 271 | 0 | 30 | 60 | 420 Tip5 |
| `compute_single_reduced_opening` | 212 | 0 | 0 | 0 | — |
| `compute_final_query_point` | 150 | 0 | 90 | 0 | — |
| `verify_fri_query` | 108 | 0 | 93 | 3 | — |
| `evaluate_polynomial` | 90 | 0 | 0 | 0 | — |
| Other 6 scopes | < 30 ops each | | | | |
| **TOTAL (profiled)** | **~6,840** | | | | |

**Key finding:** `reconstruct_index_from_bits` dominates at 58%
of all profiled Alu ops. It's called by `decompose_to_bits`,
which is in turn called by `sample_bits` in
`recursion/src/challenger/circuit.rs:421`. Each `sample_bits`
call decomposes the full 64-bit base field element + bool-checks
all 64 bits, even when only `num_bits ≪ 64` are returned to
the caller.

Per-call profile: ~30 `sample_bits` invocations × 64 bool-checked
bits each = 1,920 bool_checks (exact match to profiled count).

## Refined Stage B2 candidate (highest-impact specific)

### B2.x — `sample_bits` waste-bit elimination

**Source location:** `recursion/src/challenger/circuit.rs:419-423`:

```rust
let base_sample = self.sample(circuit);
let bits = circuit.decompose_to_bits::<BF>(base_sample, bf_bits)?;
Ok(bits[..num_bits].to_vec())  // ← throws away `bf_bits - num_bits` constrained bits
```

**The waste:** `decompose_to_bits` internally bool-checks +
sum-reconstructs ALL `bf_bits` bits (= 64 for Goldilocks).
Only `num_bits` are returned to the caller. The `bf_bits −
num_bits` bool_checks + mul_adds in the high range are pure
overhead.

**Reduction proposal:** add a new CircuitBuilder primitive
`decompose_to_low_bits_with_high_witness<BF>(x, num_bits)` that:

1. Hints `num_bits` bool-checked bits PLUS a single
   unconstrained `high_witness` field element.
2. Asserts the modular reconstruction:
   `Σ low_bits[i] · 2^i + 2^num_bits · high_witness = x`.
3. Returns only the `num_bits` low bits.

Then change `sample_bits` to call the new primitive.

**Soundness equivalence argument:**

Current: `sample_bits` returns `bits[..num_bits]` such that
`Σ_{i=0..64} bits[i] · 2^i = base_sample` AND every bit is
bool-checked. Downstream consumers (FRI query index, PoW bit
checks) use only the low bits.

Reduced: `sample_bits` returns `low_bits` such that `Σ_{i=0..num_bits}
low_bits[i] · 2^i + 2^num_bits · high_witness = base_sample`
AND every low_bit is bool-checked. The `high_witness` is a
free witness whose value doesn't matter (it's not used
downstream).

The two are observationally equivalent for the FRI verifier:
- `base_sample` is bound by challenger state (the prover can't
  lie about it; challenger absorbs are constrained).
- `low_bits` are bool-checked in both forms.
- The aggregate relation `Σ low_bits · 2^i ≡ base_sample
  (mod 2^num_bits)` is enforced in both forms (current: from
  the full reconstruction; reduced: from the modular
  reconstruction).
- The high bits / high_witness value is not used by any
  downstream consumer, so its constraint status is irrelevant.

A malicious prover gains nothing from the relaxation: they
still can't choose `low_bits` freely (must match
`base_sample mod 2^num_bits`), and they couldn't manipulate
the high bits even in the current form (no downstream
consumer reads them).

**Estimated saving:**
- Per `sample_bits` call: `bf_bits − num_bits` bool_checks +
  mul_adds removed. For Goldilocks (bf_bits=64) at typical
  num_bits ~10, saves ~54 ops per call.
- 30 calls × 54 ops = ~1,620 Alu ops removed from
  `reconstruct_index_from_bits`.
- L1 Alu savings: ~1,620 / 8 lanes ≈ 200 Alu rows = ~3% of
  Alu rows = ~2% of L1 cells.
- L2 cascade: per Phase 0 finding (-15.9% L2 vs -6.5% L1),
  expect ~5% L2 reduction.

**Implementation effort:** 2-3 hours focused work.

  - Add `decompose_to_low_bits_with_high_witness` to
    `circuit/src/builder/circuit_builder.rs` (~100 LOC + 2 KATs).
  - Add a corresponding hint variant in
    `circuit/src/builder/hints.rs` (~50 LOC).
  - Modify `recursion/src/challenger/circuit.rs::sample_bits`
    to use it (~5 LOC).
  - Full regression + Stage 5 re-measure (~30 min).

**Risk:** medium — soundness-critical primitive addition.
KAT-first discipline mandatory; the new primitive needs:
  - ACCEPT test (correct decomposition verifies).
  - TAMPER-REJECT (bad `low_bits` rejected).
  - TAMPER-REJECT (modular constraint violation rejected).
  - Cross-check vs `decompose_to_bits` on values < 2^num_bits.

**R1 framing:** soundness-critical invasive recursion-substrate
work. Per R1.1, must be attempted + driven in a focused session
with KAT-first discipline. NOT a half-session task.

**Status:** identified + designed; implementation deferred to
focused B2 session.

## Cross-references

- B0 inventory: [`2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md`](2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md)
- Recommendation hierarchy: [`2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md`](2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md) § 6
- Investigation results: [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md)
