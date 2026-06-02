# Path B status — what landed, what's residual (R1 honest)

**Date:** 2026-05-20
**Goal:** "Implement Phase B. Test exhaustively. Integrate
completely. Bubble up any major decisions." (user 2026-05-20)
**Status:** B0 + B1 + B2-prep complete; B2 implementation +
B3 + B4 are precise residuals requiring a focused multi-hour
session for safe R1 landing.

## 2026-05-20 UPDATE — B2 landed; pivots the Path B recommendation

**B2 reduction landed (commit `ce3e6a4`):** `sample_bits` now
uses the new `decompose_to_low_bits_with_high_witness` primitive.
Measured impact:

  - Alu rows: 6,851 → 3,401 (**−50%**).
  - bool_checks (global): 1,920 → 180 (**−91%**).
  - Total verifier-circuit ops: ~6,840 → ~3,500 (**−49%**).
  - L1 size: 499,353 B → 500,198 B (+845 B, **+0.17% GROWTH**).
  - All tests green (challenger_transcript 46/46; full
    p3-recursion + p3-circuit-prover regressions).

**Surprising finding — Alu reduction is L1-byte-neutral in our
substrate:**

The TALLEST table determines FRI Merkle path depth. Both Alu
(pre-B2: 6851 ops → 1024-row trace via lane-packing) and
tip5_perm (881 → 1024 rows) were tied as tallest. Post-B2,
Alu dropped to 512-row trace, but `tip5_perm` is still 1024
rows — it remains the unilateral FRI height bottleneck.
Per-query Merkle path bytes are unchanged.

The L1-byte impact is the 30 new high_witness elements
(~28 bytes each in opened_values), netting +845 B growth.

**This invalidates the B1 prediction** that Alu reduction
would cascade to L1 size (the "~2-3% L1 saving" estimate was
based on counting Alu cells, ignoring the FRI Merkle path
height effect).

## NEW Phase B recommendation (post-B2 finding)

For future verifier-AIR slim work:

| Target | Why | Effort |
|---|---|---|
| **Reduce `tip5_perm` ROW COUNT** | tip5_perm is THE FRI height bottleneck (1024-row trace) — every row removed lowers the per-query Merkle path depth, compounding across all queries | high (each Tip5 call has soundness role; batched Fiat-Shamir absorbs save ~10-20 perms; MMCS prefix sharing saves ~50-100; ultra-batched challenger absorb saves ~5) |
| Reduce FRI overhead directly | cap=3 already maxed; further would need substrate changes | substrate change |
| Path B Alu reduction (B2 type) | NEUTRAL L1 impact; positive prover-cost | low value per L1 saving |

**Alu reduction is still valuable** for prover-cost (smaller LDE,
smaller commitment work). But it's NOT the right lever for L1
size in our substrate.

**Updated Phase A vs B trade-off** (per [path_b_status]):

- Path B sub-targets that move L1: only those that lower the
  TALLEST table's height. In our case: tip5_perm row reduction.
- The dominant lever for `tip5_perm` row count is the number
  of Tip5 perm calls in the verifier circuit (881 calls):
  - ~450 from FRI commit-phase MMCS (1 per query × per fold
    round × per commitment path).
  - ~420 from `open_input` (MMCS path verification for openings).
  - ~11 from `tip5_perm_for_challenger_base` (Fiat-Shamir
    absorbs).
- The 11 Fiat-Shamir absorbs are batchable (per B1 §B.1) —
  small saving (~1%).
- The 870+ MMCS-related Tip5 calls would require either:
  - A different MMCS scheme with fewer hashes per path
    (substrate change).
  - Reducing the NUMBER OF QUERIES (already at nq=20 floor).
  - Reducing per-path depth (cap=3 already maxed).

**Reality check:** in-substrate Path B reductions to `tip5_perm`
calls bottom out at very small savings. The TIP5 NPO is intrinsic
to the recursion verifier's job (every Merkle path requires
hashing). Reducing them meaningfully requires architectural
change — which is essentially Path A territory (different MMCS
construction, or a different verifier circuit shape entirely).

**This sharpens the Phase A decision:** Path B's
post-quantum-preserving in-substrate floor for L1 is plausibly
~480 KB (where we are now). ≤65 KB requires Path A.

## What landed this session

### Stage B0 — Production L1 cert column inventory (COMPLETE)

- Built production L1 (`lb=4 nq=20 mla=3 lfp=2 cap=3 d=5`,
  82-bit Johnson, Tip5-throughout).
- Per-AIR row + width + cell breakdown.
- Per-proof-section byte breakdown.
- Discovered + fixed substrate divergence between the test
  infrastructure (cap=0) and production builder (cap=3);
  re-measured Stage 5 at production-faithful cap=3.
- Production-faithful numbers: L1 = 487.65 KB (~−51.8% vs the
  pre-2026-05-20 baseline ~1011 KB), L2 = 519.18 KB,
  L2/L1 = 1.06×.

Tool: `crates/plonky3-recursion/recursion/tests/test_path_b_b0_inventory.rs`
(commit `e8b...` then `11a58d8` refinement).

Doc: [`2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md`](2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md)

### Stage B1 — Reduction map (COMPLETE)

- Enumerated 4 reduction families with soundness equivalence
  arguments + sort keys.
- Identified Alu reduction as the highest-impact family
  (Alu dominates by both row count and cell count — 60% of
  L1 cells).
- Refined post-profiling: `reconstruct_index_from_bits` is
  the single dominant Alu contributor (58% of profiled ops).

Doc: [`2026-05-20_PATH_B_STAGE_1_REDUCTION_MAP.md`](2026-05-20_PATH_B_STAGE_1_REDUCTION_MAP.md)

### Profiling infrastructure (PREREQUISITE FOR SAFE B2)

- Wired `--features profiling` passthrough through
  `p3-recursion`, `p3-circuit-prover`, `p3-circuit`.
- Inventory tool gains per-scope op count attribution
  (`CircuitBuilder::scope_op_counts()`).
- Confirmed per-source attribution for all profiled scopes.

Commit: `87a78e1`.

### Specific B2 candidate identified (precise residual)

The dominant verifier-circuit scope is
`reconstruct_index_from_bits` (3,840 ops; called by
`decompose_to_bits` via `sample_bits` in
`recursion/src/challenger/circuit.rs:421`). It over-decomposes
the full 64-bit base field element when only `num_bits ≪ 64`
are returned.

**Reduction:** add a `decompose_to_low_bits_with_high_witness`
primitive that decomposes only the low `num_bits` (bool-checked)
plus a single unconstrained high_witness. Soundness-equivalent
for downstream consumers (FRI query index, PoW checks).

**Estimated saving:** ~1,620 Alu ops removed; ~2% L1 cell
reduction; ~5% L2 cascade per the Phase 0 amplification factor.

**Implementation effort:** 2-3 hours focused work.

Full design + soundness argument: see B1 § "Refined Stage B2
candidate (highest-impact specific)".

## R1 honest residuals (precise, actionable)

### B2 implementation

Add `decompose_to_low_bits_with_high_witness` to CircuitBuilder
+ KATs + integration + Stage 5 re-measure.

This is **soundness-critical invasive recursion-substrate
work** — per R1, requires:

- KAT-first de-risk before invasive landing.
- ACCEPT + TAMPER-REJECT × 3 (modular constraint violation,
  bad low_bits, cross-check vs `decompose_to_bits`).
- Full project regression + Stage 5 re-measure.
- Per-stage validation; revert if soundness wall.

**Recommended:** schedule a dedicated focused B2 session.
Attempting it half-heartedly in this multi-task session would
violate R1 ("no rushing soundness-critical work").

### B3 re-audit + B4 final measurement

Contingent on B2 landing. After a real reduction lands:
- Re-derive per-layer Johnson bits at the reduced verifier
  (FRI params unchanged → bits unchanged, but document the
  AIR + LogUp side shifts).
- Re-derive Schwartz-Zippel per-AIR bounds.
- Update [`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md) and
  [`2026-05-19_C4_AUDIT_READINESS.md`](2026-05-19_C4_AUDIT_READINESS.md).
- Run full project regression.
- Record post-B2 L1+L2 size; update investigation doc +
  design report.

### Additional reductions (lower priority)

Per B1's sort:

1. **A.1 Custom FRI-fold AIR** — biggest single reduction
   (~−15-25% L1); requires multi-day focused session.
   Major decision required: do we accept the maintenance
   burden of a vendored-Plonky3 patch + new audit surface?
2. **D Preprocessed-trace hoisting** — modest (~−5-10% L1);
   methodology validation; can be combined with A.1.
3. **A.3 Lagrange basis hoisting** — small (~−1%); warm-up.
4. **B.1 Batch Fiat-Shamir absorbs** — small (~−1%).
5. **B.2 MMCS prefix sharing** — medium (~−3%); high effort.

## Why not just attempt B2 in this session

Per R1 ("don't rush soundness-critical invasive work") + R1.1
("attempt + drive once de-risked, but staging discipline still
applies"):

- B2 = adding a new soundness-critical primitive to the recursion
  substrate.
- Half-attempted = high risk of subtle soundness hole that
  passes ACCEPT but fails tamper-REJECT under specific
  inputs.
- The R1 honest "validated subset + precise residual" outcome
  is **better** than a rushed B2 that risks the trust
  linchpin.
- This session has already covered substantial ground:
  Tip5-throughout L2 wiring (Stage 3-6), Tier B flip,
  Phase 0 lever stacking, cap=3 substrate correction, B0 + B1.

## Major decisions bubbled up (per goal)

### Decision 1 — B2 implementation session timing

Schedule a focused B2 session (2-3 hours) for the
`decompose_to_low_bits_with_high_witness` primitive landing.
Should it happen now (extending this session) or as a
separate session?

Recommendation: **separate session**, fresh attention. This
session has covered ~10 distinct deliverables already; B2 is
soundness-critical and deserves dedicated focus.

### Decision 2 — Custom FRI-fold AIR (A.1)

Pursue or skip? Trade-offs:
- Pursue: ~−20% L2 cascade single-step win.
- Skip: Path B ceiling drops from ~−25-35% to ~−10%.

Recommendation: defer until after B2 lands + we have a real
L1 cascade measurement to recalibrate the cost/benefit.

### Decision 3 — When to commit to Path A (SNARK wrap)?

The cumulative production reduction is now ~−51.8% (1011 →
488 KB). Full Path B (all reductions in B1) would plausibly
get us to ~200-300 KB L1. Still ~3-5× over the ≤65 KB
target.

Recommendation: revisit after B2 lands. If B2 + A.1 together
demonstrate cascading recursion compression (L_{n+1} < L_n),
the case for staying post-quantum + skipping Path A
strengthens. If the L2 inflation persists, Path A becomes
the architectural commitment.

## Cross-references

- [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md) (corrected production numbers)
- [`2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md`](2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md) (architecture)
- [`2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md`](2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md) (inventory data)
- [`2026-05-20_PATH_B_STAGE_1_REDUCTION_MAP.md`](2026-05-20_PATH_B_STAGE_1_REDUCTION_MAP.md) (candidate reductions)
- Per-scope profiling data: `cargo test -p p3-recursion --release --test test_path_b_b0_inventory --features profiling -- --ignored --nocapture`
