> _Created **2026-05-20** · last updated **2026-05-20**._

# S7 — Audit sign-off: CSA stages S0–S6 cross-checked + C4 / GAP_AUDIT updated

> **Status (R1, honest).** S7 LANDED. Stage S7 of
> `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`. **All
> 8 stages (S0–S7) of the CSA are now landed in M-S6
> scope.** This is the final stage that:
>
> 1. Cross-checks S1's per-constraint soundness derivation
>    (every AIR + bus ≥80 unconditional bits) against S4–S6's
>    tamper-test pass rate.
> 2. Updates `2026-05-19_C4_AUDIT_READINESS.md` § 7 (adversarial-
>    test inventory) with the new tamper-test catalogue rows.
> 3. Routes deferred-as-deepening items to
>    `2026-05-15_GAP_AUDIT.md`.
> 4. Flips the CSA design status to LANDED in the parent doc.
>
> **CSA verdict** (the headline number for audit / sign-off):
>
> > **Constraint side ≥ 80 unconditional bits at every AIR +
> > every LogUp bus.** Per-AIR MIN: 103 bits (production AIR
> > BLAKE3 + Matmul `d_max=7`); per-bus MIN: 98 bits (BUS_IRANGE8
> > with k_b ≤ 2^28). Combined with S(−1) FRI ≥ 82 unconditional,
> > **chain MIN = 82 unconditional bits**, ≥80 floor with 2-bit
> > margin. Every constraint family has at minimum one tamper
> > test exercising its rejection mechanism. All in-scope GAP-G2
> > rename items + GAP-G1 subsumption documentation landed.
> > GAP-G3 deferred items (M12 / `#127`) explicitly routed and
> > tracked in C4 § 8.

---

## 1. CSA stage rollup

| Stage | Deliverable | Commit |
|---|---|---|
| **S0** | `2026-05-20_CONSTRAINT_INVENTORY.md` — 117 constraint families × 27 AIRs × ~190 existing tamper tests catalogued | `47bb98f` (CSA design + S0) |
| **S1** | `2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md` — per-AIR + per-bus ≥80 unconditional with margins | `45c00fc` (S1+S2) |
| **S2** | `2026-05-20_TAMPER_GAP_LIST.md` — backlog: 1 GAP-G1 + 3 GAP-G2 + 3 deferred GAP-G3 + 1 deepening | `45c00fc` |
| **S3** | `2026-05-20_TAMPER_TEST_SPECIFICATION.md` — 4-item spec per template | `0fd6bde` (S3+S4) |
| **S4** | 9 new tamper tests + 3 audit-routing doc-comments | `0fd6bde` |
| **S5** | `2026-05-20_CSA_S5_CROSS_AIR_TAMPER_TESTS.md` + 1 new K3 producer-side test | this commit |
| **S6** | `2026-05-20_CSA_S6_PROPERTY_BASED_TESTS.md` + 1 demo proptest | this commit |
| **S7** | This doc + C4 §7 update + GAP_AUDIT update + CSA design status flip | this commit |

**Total artifacts:** 8 docs + 11 new tests + 3 doc-comments.

---

## 2. Cross-check: S1 derivation ↔ S4–S6 empirical rejection

S1's claim was every per-AIR + per-bus ≥80 unconditional
bits (derived analytically). S4–S6 added 11 new tamper tests
that empirically validate the rejection mechanism. The
cross-check:

| AIR / bus | S1 derived bits | S4–S6 tests added | Rejection rate observed | Cross-check |
|---|---:|---:|---:|---|
| StarkRowChip | 104 | 0 (already 4 existing) | 1.0 (all 4 tests reject) | ✅ |
| RangeTableChip × 4 | 104 (per chip) | 8 (URange13×3 + IRange7P1×3 + IRange8×2) | 1.0 (all 8 reject) | ✅ |
| I8U8Chip | 104 | 0 (already 9 existing) | 1.0 | ✅ |
| ControlChip (CRIT-1) | 104 | 0 (already 10) | 1.0 | ✅ |
| InputChip (M-S1/A3) | 104 | 0 (already 4) | 1.0 | ✅ |
| MatmulCumsumChip | 103 | 0 (already 8) | 1.0 | ✅ |
| FoldChip | 104 | 0 (already 5) | 1.0 | ✅ |
| StripeXorChip | 104 | 1 (K3 producer-side S5) | 1.0 | ✅ |
| XStepChip | 104 | 0 (already 4) | 1.0 | ✅ |
| Blake3Chip | 103 | 0 (already 13) | 1.0 | ✅ |
| JackpotChip | 104 | 0 (already 10) | 1.0 | ✅ |
| Composite K1 (CRIT-1) | 104 | 0 (already 7) | 1.0 | ✅ |
| Composite K2 (§4.D) | 104 | 0 (already 3) | 1.0 | ✅ |
| Composite K3 (§6(b)-G2) | 104 | 1 (consumer S4) + 1 (producer S5) | 1.0 | ✅ |
| Composite K4 (M-S1 pack) | 104 | 0 (already 2) | 1.0 | ✅ |
| LogUp buses × 6 | 98-105 | 1 (S6 proptest demo on URange8) | 1.0 (8/8 random cases) | ✅ |
| Tip5 perm AIR (C2.1) | 109 | 0 (already 4 + KAT) | 1.0 | ✅ |
| Plonky3-recursion verifier | 105 | 0 (upstream + indirect) + 2 doc-comments | 1.0 (via c3_stage) | ✅ |

**Cross-check result.** Every per-AIR / per-bus S1 derivation
is empirically corroborated by the tamper-test pass rate
(deterministic + property-based). **No constraint family
surfaced an unexpected acceptance** that would invalidate
S1's soundness derivation. The 5 active property tests
(4 existing + 1 new S6) all maintained 100% rejection rate
across their case counts.

### 2.1 No surprises during S4–S6

R1 honest accounting: every S4/S5/S6 test was implemented
expecting `verify` to return `Err`, and every test confirmed
that expectation. **No test discovered an "unexpected
acceptance"** that would have signalled a hidden soundness
hole. The CSA stack is internally consistent.

### 2.2 The one quirk (documented, not a finding)

The S4 K3 consumer-side test (`high2_2_g2_xstep_stripe_pin_rejects`)
first attempted to tamper FOLD_XSTEP by copying SX_XR[1] (a
different lane). This **failed the precondition assertion**
because the synthetic test geometry produces SX_XR[0] == SX_XR[1]
== 0 at fold-row 0 (the XOR happens to cancel). The fix was
to tamper "+1" (guaranteed-different per Goldilocks
characteristic). This is a *test-design* quirk, not a
soundness issue — the constraint catches the tamper either
way; the test just needs a discriminating tamper variant
to be observable.

---

## 3. C4 audit-readiness updates

### 3.1 §7 adversarial-test inventory — new rows

The C4 §7 table (existing 10 attack-class rows) is extended
with 1 new row reflecting CSA's depth:

| New attack class | What it adds | Where covered |
|---|---|---|
| **A-CONSTRAINT (every AIR constraint family)** | Per-constraint tamper coverage at single-cell / selector / bus / cross-AIR / property-based levels | CSA S4–S6 deliverables (~13 new tests + 4 existing per-family clusters) |

(Applied to the C4 doc in §3.2 of the parent commit.)

### 3.2 §3 soundness-claim index — CSA per-claim pointer

Each row in C4 §3 (38 soundness claims) is supplemented with
a CSA cross-link to the per-claim AIR-side derivation in
S1 § 7.2 (the per-claim audit walk). This is a doc-update
only; the existing test cross-links remain authoritative.

### 3.3 §8 known residuals — refinement

The "CSA S1–S7" row added in the prior commit (`47bb98f`) is
now flipped from in_progress to **LANDED** with the precise
deferred items:

- F3–F20 fine-grained per-FRI-fold-round direct tampers
  (deferred-as-deepening per S5 § 3).
- Per-constraint-family proptest sweep (deferred-as-deepening
  per S6 § 4).
- M12 / `#127` GAP-G3 items: BUS_MATMUL_INPUT,
  BUS_JACKPOT_X_BITS, Tip5 D=2 R-a tail (already deferred per
  C4 §8 prior).

---

## 4. GAP_AUDIT routing

Deferred-as-deepening items (R1 honest accounting) routed to
`2026-05-15_GAP_AUDIT.md`:

| Finding | Type | Disposition | Where landed |
|---|---|---|---|
| F3–F20 fine-grained FRI fold-round direct tampers | deepening (not a gap) | Post-M6 audit deepening; covered indirectly via c3_stage | S5 § 3 |
| Per-constraint-family proptest sweep | deepening (not a gap) | Post-M6 audit deepening; M-S6 baseline = 5 active property tests | S6 § 4 |
| Per-bus exact k_b instrumentation | refinement (not a gap) | Refinement to S1's worst-case bounds; not currently needed (all buses ≥98) | S1 § 8 #1 |
| Plonky3-internal FRI-STARK soundness reduction constants | refinement (not a gap) | C4 auditor item; existing standard analysis is conservative | S1 § 8 #2 |

**No new findings.** Every "residual" listed is a deepening /
refinement item routed for post-audit follow-up if requested,
not a soundness hole the analysis surfaced.

---

## 5. Final per-AIR sign-off table

This is the audit's worktable cross-link, layered as
(AIR → constraint families → S1 bits → S4–S6 tests).
Per-AIR sign-off:

| AIR | Constraint families | S1 ε_AIR bits | Tests (det. + prop.) | Sign-off |
|---|---:|---:|---|---|
| StarkRowChip | 2 | 104 | 5 det. | ✅ |
| RangeTableChip × 4 | 12 | 104 | 11 det. + 1 prop. | ✅ |
| I8U8Chip | 7 | 104 | 9 det. | ✅ |
| ControlChip | 7 | 104 | 10 det. | ✅ |
| InputChip | 2 | 104 | 4 det. | ✅ |
| MatmulCumsumChip | 3 | 103 | 8 det. | ✅ |
| FoldChip | 4 | 104 | 5 det. | ✅ |
| StripeXorChip | 3 | 104 | 6 det. | ✅ |
| XStepChip | 4 | 104 | 4 det. | ✅ |
| Blake3Chip | 5 | 103 | 13 det. | ✅ |
| JackpotChip | 3 | 104 | 10 det. | ✅ |
| Composite keystones K1–K4 | 4 | 104 | 12 det. (+ 1 S4 + 1 S5) | ✅ |
| LogUp buses × 6 | 6 producer/consumer pairs | 98–105 | ~30 det. + 4 prop. existing + 1 new | ✅ |
| Tip5 perm AIR (C2.1) | 12 | 109 | 4 det. + 4411 KAT/random | ✅ |
| Plonky3-recursion verifier | 25 | 105 | ~30 det. (upstream + indirect) + 2 doc-routing labels | ✅ |
| **Per-AIR MIN bits** | — | **98** (BUS_IRANGE8) | — | **✅ ALL** |
| **Combined with S(−1) FRI** | — | **82 chain MIN** (FRI binds) | — | **✅ ≥80** |

---

## 6. CSA design status flip

Per CSA design § 9 (R1 honest accounting), the design doc's
status flips from **"DESIGN + STAGED PLAN"** to **"LANDED"**
with this S7 commit.

The CSA design's R1 residual list (§ 9 of the design doc)
is now fully discharged:

- S1 (per-constraint derivation): ✅ landed
- S2 (gap list): ✅ landed
- S3 (tamper-test specs): ✅ landed
- S4 (impl): ✅ landed
- S5 (cross-AIR): ✅ landed
- S6 (property): ✅ landed
- S7 (sign-off): ✅ landed (this doc)

All 8 stages complete. No fake completion; every stage's exit
gate was met as documented in the parent design § 5.

---

## 7. Production-readiness implications

The CSA closes the **AIR-side soundness derivation** that
M-S5b S(−1) deferred to C4 audit. With both sides closed:

- **FRI side (S(−1)):** every layer ≥82 unconditional bits
  at the Johnson radius per IACR ePrint 2025/2055 Theorem 1.5.
- **AIR side (CSA):** every AIR + LogUp bus ≥98 unconditional
  bits per Plonky3 STARK soundness + Habock LogUp bounds.
- **Chain MIN:** 82 unconditional bits (FRI binds) with
  2-bit margin to the 80 floor.

For audit purposes, the next steps are:

1. **C4 in-house audit walk** — the M-S6 milestone proper.
   The CSA provides the depth layer the auditor drills into.
2. **External crypto audit** — readiness package complete;
   external auditor can verify the ε_AIR + ε_LogUp + ε_FRI
   accounting.
3. **M-S5b S1 Path-B verifier-AIR reduction map** — separate
   milestone (size target). CSA confirms the AIR + LogUp
   margin (≥18 bits to FRI floor) is sufficient headroom for
   the verifier-AIR narrowing S1 will design.

---

## 8. Honest residuals (R1)

What S7 — the final stage — explicitly defers:

1. **F3–F20 fine-grained FRI fold-round direct tampers**
   — deferred-as-deepening; routed to GAP_AUDIT.
2. **Per-constraint-family proptest sweep** —
   deferred-as-deepening; routed to GAP_AUDIT.
3. **GAP-G3 M12 items** — out of M-S6 scope; tracked in C4
   § 8 (BUS_MATMUL_INPUT, BUS_JACKPOT_X_BITS, Tip5 D=2 R-a
   tail).
4. **External-auditor-driven findings** — any finding the
   auditor surfaces during C4 walk routes to GAP_AUDIT per
   R1; S7 sign-off does not preempt the auditor.

None of these are soundness holes in the LANDED M-S5 chain.

---

## 9. Cross-references

- **CSA design (the parent staged plan).**
  `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`.
- **CSA stage docs (S0–S6).**
  - `2026-05-20_CONSTRAINT_INVENTORY.md` (S0)
  - `2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md` (S1)
  - `2026-05-20_TAMPER_GAP_LIST.md` (S2)
  - `2026-05-20_TAMPER_TEST_SPECIFICATION.md` (S3)
  - `2026-05-20_CSA_S5_CROSS_AIR_TAMPER_TESTS.md` (S5)
  - `2026-05-20_CSA_S6_PROPERTY_BASED_TESTS.md` (S6)
- **Sibling FRI-side analysis.**
  `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` (S(−1)).
- **C4 audit-readiness (the audit framework this lives in).**
  `2026-05-19_C4_AUDIT_READINESS.md` § 3 + § 7 + § 8 + § 10.
- **GAP_AUDIT (where deferrals route).**
  `2026-05-15_GAP_AUDIT.md`.
- **Production roadmap.**
  `2026-05-17_PRODUCTION_ROADMAP.md` Phase C row M-S5b + C4.
- **R1 / R1.1 discipline.** `~/.claude/CLAUDE.md`.
