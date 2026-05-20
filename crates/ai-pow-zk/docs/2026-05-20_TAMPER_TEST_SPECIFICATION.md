> _Created **2026-05-20** · last updated **2026-05-20**._

# S3 — Tamper-test specification: the 4-item S4 backlog from S2

> **Status (R1, honest).** S3 LANDED. Stage S3 of
> `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`.
> Consumes S2 backlog (`2026-05-20_TAMPER_GAP_LIST.md` §6).
> Specifies each new tamper test using the §4.2 template
> from CSA design. Each spec includes: AIR, file:line of
> constraint, mechanism (M1–M5), honest trace, tamper variant,
> predicted rejection, positive control, coverage class
> (C1–C5).
>
> **S4 scope (the implementation):** ~10–12 hours of focused
> tests-only work; 0 fenced-linchpin edits. All tests use
> existing trace-builder infrastructure. Sub-phase order
> within S4: §4.B (range tables) → §4.B (K3 G2 explicit) →
> §4.B+§4.A (doc-comments + cross-refs).

---

## 1. Template (per CSA design §4.2)

Each spec follows:

```
TAMPER TEST: <name>
  AIR / file:line:    where the constraint lives
  Mechanism:          M1–M5 (per CSA §4.1)
  Honest trace:       baseline trace construction
  Tamper variant:     exact cell/selector/multiplicity altered
  Predicted reject:   error signal expected
  Positive control:   paired acceptance test
  Coverage class:     C1–C5 (per CSA §4.3)
  R1 risk:            low/medium/high (per CSA §0.3)
```

---

## 2. S3-1: URange13 + IRange7P1 explicit constraint-rejection tests (priority 1)

S0 §2.2 + S2 §2.1 found: URange13 + IRange7P1 chips have
the same 3-constraint shape as URange8 + IRange8, but only
parametric / smoke tests exist for them. Add the explicit
adversarial variants.

### 2.1 `urange13_verify_rejects_wrong_first_row`

```
AIR:                 RangeTableChip<URANGE13_TABLE, 0, 8191>
File:line:           crates/ai-pow-zk/src/chips/range_table.rs:78
                     (the `when_first_row().assert_eq(cur_val, min_f)` call)
Mechanism:           M1 — AIR `eval()` constraint violation
Honest trace:        build_valid_table_trace::<URANGE13_TABLE, 0, 8191>(8192)
Tamper variant:      trace.values[URANGE13_TABLE] = 5 (should be 0 for row 0)
Predicted reject:    verify returns Err (the first-row equality fails)
Positive control:    prove_and_verify_urange13_table (existing @ :357)
Coverage class:      C1 — Single-cell tamper
R1 risk:             low (tests-only; mirror of urange8 boilerplate)
```

### 2.2 `urange13_verify_rejects_wrong_last_row`

```
AIR:                 RangeTableChip<URANGE13_TABLE, 0, 8191>
File:line:           range_table.rs:80
Mechanism:           M1
Honest trace:        same as 2.1
Tamper variant:      trace.values[8191 * TOTAL_TRACE_WIDTH + URANGE13_TABLE] = 100
Predicted reject:    verify returns Err (last-row equality and/or transition fail)
Positive control:    prove_and_verify_urange13_table
Coverage class:      C1
R1 risk:             low
```

### 2.3 `urange13_verify_rejects_non_boolean_delta`

```
AIR:                 RangeTableChip<URANGE13_TABLE, 0, 8191>
File:line:           range_table.rs:82-87
Mechanism:           M1
Honest trace:        same as 2.1
Tamper variant:      trace.values[100 * TOTAL_TRACE_WIDTH + URANGE13_TABLE] = 50
                     (row 99 was 99, now 100→50 gives δ = -49, not ∈ {0, 1})
Predicted reject:    verify returns Err (transition delta non-boolean)
Positive control:    prove_and_verify_urange13_table
Coverage class:      C1
R1 risk:             low
```

### 2.4 `irange7p1_verify_rejects_wrong_first_row`

```
AIR:                 RangeTableChip<IRANGE7P1_TABLE, -64, 64>
File:line:           range_table.rs:78 (the same with_first_row assertion)
Mechanism:           M1
Honest trace:        build_valid_table_trace::<IRANGE7P1_TABLE, -64, 64>(256)
Tamper variant:      trace.values[IRANGE7P1_TABLE] = 0 (should be -64 for row 0)
Predicted reject:    verify returns Err
Positive control:    prove_and_verify_irange7p1_table (existing @ :281)
Coverage class:      C1
R1 risk:             low
```

### 2.5 `irange7p1_verify_rejects_wrong_last_row`

```
AIR:                 RangeTableChip<IRANGE7P1_TABLE, -64, 64>
File:line:           range_table.rs:80
Mechanism:           M1
Honest trace:        same as 2.4
Tamper variant:      tamper row 255's value to a value different from 64 (the padded MAX)
Predicted reject:    verify returns Err
Positive control:    prove_and_verify_irange7p1_table
Coverage class:      C1
R1 risk:             low
```

### 2.6 `irange7p1_verify_rejects_non_boolean_delta`

```
AIR:                 RangeTableChip<IRANGE7P1_TABLE, -64, 64>
File:line:           range_table.rs:82-87
Mechanism:           M1
Honest trace:        same as 2.4
Tamper variant:      trace.values[80 * TOTAL_TRACE_WIDTH + IRANGE7P1_TABLE] = -32
                     (row 79 was -64+79 = 15; row 80 was 16; tampered to -32 gives δ=-48)
Predicted reject:    verify returns Err
Positive control:    prove_and_verify_irange7p1_table
Coverage class:      C1
R1 risk:             low
```

### 2.7 `irange8_verify_rejects_wrong_first_row`

```
AIR:                 RangeTableChip<IRANGE8_TABLE, -128, 127>
File:line:           range_table.rs:78
Mechanism:           M1
Honest trace:        build_valid_table_trace::<IRANGE8_TABLE, -128, 127>(256)
Tamper variant:      trace.values[IRANGE8_TABLE] = 0 (should be -128 for row 0)
Predicted reject:    verify returns Err
Positive control:    prove_and_verify_irange8_table (existing @ :322)
Coverage class:      C1
R1 risk:             low
```

### 2.8 `irange8_verify_rejects_wrong_last_row`

```
AIR:                 RangeTableChip<IRANGE8_TABLE, -128, 127>
File:line:           range_table.rs:80
Mechanism:           M1
Honest trace:        same as 2.7
Tamper variant:      trace.values[255 * TOTAL_TRACE_WIDTH + IRANGE8_TABLE] = 0
                     (should be 127)
Predicted reject:    verify returns Err
Positive control:    prove_and_verify_irange8_table
Coverage class:      C1
R1 risk:             low
```

**Subtotal:** 8 new tests, all single-cell C1, all M1 mechanism.

---

## 3. S3-2: K3 §6(b)-G2 keystone explicit tamper (priority 2)

S2 §2.2: K3 (FOLD_XSTEP == SX_XR[stripe]) is covered by
`high2_2_fold_chain_pinned_logup@composite_proof.rs:726` as
a happy-path positive control, but no adversarial tamper
test is explicitly labelled for it.

### 3.1 `high2_2_g2_xstep_stripe_pin_rejects`

```
AIR:                 Composite full AIR (with-lookups path)
File:line:           crates/ai-pow-zk/src/composite_full_air.rs:318-334
                     (Σ_s FOLD_STRIPE_SEL[s] · (FOLD_XSTEP − SX_XR[s]) = 0)
Mechanism:           M1
Honest trace:        Same setup as `high2_2_fold_chain_pinned_logup` —
                     real matmul tile + fold-active rows + StripeXor active +
                     CONTROL_PREP pin (FOLD_IS_FOLD, FOLD_SLOT_SEL,
                     FOLD_STRIPE_SEL on row r with stripe index j).
Tamper variant:      Pick a fold-active row r where FOLD_STRIPE_SEL[j] = 1.
                     Tamper trace.values[r * TOTAL_TRACE_WIDTH + FOLD_XSTEP]
                     to SX_XR[j'] for some j' ≠ j. The constraint
                     Σ_s FOLD_STRIPE_SEL[s] · (FOLD_XSTEP − SX_XR[s])
                     = FOLD_STRIPE_SEL[j] · (SX_XR[j'] − SX_XR[j]) ≠ 0.
Predicted reject:    verify returns Err (K3 constraint violated)
Positive control:    high2_2_fold_chain_pinned_logup (existing @ :726)
Coverage class:      C1 — Single-cell tamper, but tests a cross-chip
                     soundness keystone
R1 risk:             medium — requires reading the composite-trace API
                     and identifying a valid fold-active row + valid
                     SX_XR[j'] value. Mitigation: model after the existing
                     positive control's setup.
```

---

## 4. S3-3: h_a/h_b subsumption documentation (priority 3, doc-only)

S2 §3.1 reclassified the h_a/h_b "gap" from G3 to G1
(subsumed by 3-layer existing coverage).

### 4.1 Document the subsumption in code + audit-readiness

```
Spec:                Documentation update; no new test code.
File(s):
  - crates/ai-pow/src/zk_bridge.rs near `sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects`
    (the c-exact circuit-side test that subsumes the h_a/h_b angle):
      Add a doc-comment explicitly stating the test covers the
      strip-opening-leaf-row binding to HASH_A/HASH_B at the circuit
      side, complementing the ai-pow `reject_tampered_h_a@adversarial.rs:44`
      / `reject_tampered_h_b@adversarial.rs:63` (M5 mechanism) and the
      PI binding `full_air_rejects_tampered_hash_a_pi@composite_trace.rs:3033`
      (M1 mechanism).
  - crates/ai-pow-zk/docs/2026-05-19_C4_AUDIT_READINESS.md § 3.3 (MAT)
    or § 3.4 (A3): add a row cross-linking the 3 layers explicitly.

Mechanism:           Audit-readiness — clarifies what each test covers
                     so the auditor doesn't re-investigate "is the h_a
                     / h_b root binding tested?"
Coverage class:      n/a (documentation)
R1 risk:             zero
```

---

## 5. S3-4: Poseidon2 / Poseidon1 upstream-routing label (priority 4, doc-only)

S2 §2.3: Poseidon2/1 perm AIRs are vendored from upstream
Plonky3. No in-tree tamper test needs to be *added* — only
the audit-readiness label.

### 5.1 Vendoring README / lib.rs doc-comment

```
Spec:                Documentation update; no new test code.
File(s):
  - Plonky3-recursion/poseidon2-circuit-air/src/lib.rs (or
    poseidon1-circuit-air/src/lib.rs equivalent) — add a doc-comment
    explicitly stating: "Tamper coverage for this AIR is upstream
    Plonky3 test suite + indirect via c3_stage_a/b/c via the L1+L2
    composite outer-cert tests. No in-tree tamper test is added;
    the vendoring rev `c2c51fb` is the authoritative test-source."
  - Plonky3-recursion/README.md or vendoring notes if present.
  - crates/ai-pow-zk/docs/2026-05-19_C4_AUDIT_READINESS.md § 3.5 (C)
    or § 7: add an entry cross-linking the upstream Plonky3 Poseidon2/1
    tamper coverage.

Mechanism:           Audit-readiness — documents that upstream coverage
                     is the audit-source.
Coverage class:      n/a (documentation)
R1 risk:             zero
```

---

## 6. S4 batch-execution order

Sub-phase ordering per CSA §5.6, smallest-risk-first:

| Order | Item | Effort | R1 risk |
|---:|---|---:|---|
| 1 | §2 — 8 range-table tamper tests (mirror urange8 boilerplate) | 2–4 h | low |
| 2 | §3 — K3 §6(b)-G2 explicit tamper | 4–8 h | medium |
| 3 | §4 — h_a/h_b subsumption doc-comments | 1–2 h | zero |
| 4 | §5 — Poseidon2/1 upstream-routing label | 1–2 h | zero |
| **Total S4** | — | **8–16 hours** | low–medium |

---

## 7. Acceptance criteria for S4 (per-test)

Each S4 test must:

1. **Reject the tamper** — `verify(...)` returns `Err` for
   the tampered trace (M1 mechanism) OR the panic / WitnessConflict
   fires (M4 mechanism).
2. **Pair with a positive control** — the existing happy-path
   test (named in the spec above) continues to pass; no
   false-positives.
3. **Be in CI** — not `#[ignore]`d (these are fast tests; no
   reason to gate them behind manual invocation).
4. **Documented** — each test has a 1-line doc-comment
   explaining what's tampered + the predicted rejection
   mechanism.
5. **Naming convention** — follows the existing
   `<claim>_<adversary_class>_<variant>_<reject_mechanism>`
   pattern.

---

## 8. Honest residuals (R1)

What S3 does not address, deferred:

1. **Property-based variants** are S6 work, not S3+S4. Each
   of the 8 + 1 + 0 + 0 new tests above is deterministic
   (one tamper variant per constraint). S6 will add proptest
   wrappers.

2. **Cross-AIR composition variants** are S5 work, not S3+S4.
   S2 §5 identified ~4 cross-AIR new tests; S3 does not spec
   those.

3. **F3–F20 fine-grained per-FRI-fold-round tampers** are
   S5 work.

4. **The h_a/h_b deeper-than-3-layer audit** — if the auditor
   wants a deeper analysis than the 3-layer subsumption,
   S7 routes to GAP_AUDIT for follow-up. Not S3 scope.

---

## 9. Cross-references

- **S0 inventory.** `2026-05-20_CONSTRAINT_INVENTORY.md`.
- **S1 derivation.** `2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md`.
- **S2 gap list.** `2026-05-20_TAMPER_GAP_LIST.md`.
- **CSA design.** `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`
  § 4 + § 5.6.
- **Existing test patterns to mirror.**
  `crates/ai-pow-zk/src/chips/range_table.rs:200-365`
  (urange8 / irange8 templates).
- **K3 keystone source.**
  `crates/ai-pow-zk/src/composite_full_air.rs:318-334`.
- **K3 positive control.**
  `crates/ai-pow-zk/src/composite_proof.rs:726`.
- **h_a/h_b subsumption sources (3-layer coverage).**
  - `crates/ai-pow/tests/adversarial.rs:44, 63` (extraction
    layer M5);
  - `crates/ai-pow-zk/src/composite_trace.rs:3033` (PI
    layer M1);
  - `crates/ai-pow/src/zk_bridge.rs:2485` (circuit-leaf
    layer M1).
- **C4 audit-readiness §3 + §7** — destination for
  audit-readiness cross-links.
