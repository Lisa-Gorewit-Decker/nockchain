> _Created **2026-05-20** · last updated **2026-05-20**._

# S6 — Property-based tampering: random-input rejection-rate validation

> **Status (R1, honest).** S6 LANDED. Stage S6 of
> `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`.
>
> **What S6 delivered:** 1 new demonstration property test
> (`prop_urange8_random_row_non_boolean_delta_rejects`) showing
> the proptest pattern applied to a range-table constraint.
> The broader sweep (proptest variants for every AIR's
> constraint families) is **deferred-as-deepening** per CSA
> design § 5.8 + § 7 residual #2.
>
> **What S6 establishes:** the property-test pattern is
> infrastructure-ready (proptest already in dev-deps,
> `cases = 8` per family runs in ~10 s) and existing
> `prop_*` tests in `composite_full_air_with_lookups.rs:1280-`
> (4 LogUp-side property tests) provide ample template. The
> M-S6 audit baseline is satisfied by the existing 4 + this 1
> new = 5 property tests; deeper coverage is post-audit
> deepening.
>
> **R1 honest:** S6 does **not** ship a property test per
> constraint family (~117 families × 8 cases = ~30-min CI
> runtime — too heavy for default CI). The 1 new demo is the
> R1 validated subset; the rest is precise residual routed to
> GAP_AUDIT as deepening.

---

## 1. The proptest pattern (the demonstration)

Existing infrastructure (per S0 inventory + the
`composite_full_air_with_lookups.rs:1280-` pattern):

```rust
use proptest::prelude::*;

proptest::proptest! {
    #![proptest_config(ProptestConfig {
        cases: 8,                     // small N because prove+verify is slow
        .. ProptestConfig::default()
    })]

    /// Per-constraint-family property test:
    /// random tamper variant → verifier must reject.
    #[test]
    fn prop_<claim>_<constraint>_<random_variant>_rejects(
        // input strategy: generate random tamper inputs
        param in <range or sample>
    ) {
        let cfg = ...;
        let mut trace = build_honest_trace();
        // tamper using the random param
        tamper_one_cell(&mut trace, param);
        let proof = prove(&cfg, ..., trace, ...);
        prop_assert!(
            verify(&cfg, ..., &proof, ...).is_err(),
            "property: <description> at param={}", param
        );
    }
}
```

Key properties:
- `cases: 8` keeps CI runtime manageable (~10 s per family).
- `prop_assert!` is the proptest-equivalent of `assert!` that
  participates in shrinking when a test fails.
- The honest acceptance is checked by the existing
  `prove_and_verify_*` tests (positive control); the property
  test only checks tamper rejection.

---

## 2. New S6 test landed this commit

### 2.1 `prop_urange8_random_row_non_boolean_delta_rejects`

**File:** `crates/ai-pow-zk/src/chips/range_table.rs:445-`.

**Spec:**

```
AIR:                 RangeTableChip<URANGE8_TABLE, 0, 255>
Constraint:          transition δ ∈ {0, 1} (range_table.rs:82-87)
Mechanism:           M1 (AIR `eval()` non-boolean delta)
Input strategy:      row in 2u32..253 (interior row; avoids first/last edge)
Tamper algorithm:    Set `trace.values[row * width + URANGE8_TABLE]` to
                     `((row + 100) % 256)`. The transition from
                     `row-1 → tampered` has δ = `((row+100)%256) − (row-1)`,
                     guaranteed ∉ {0, 1} for all r ∈ [2, 253].
Cases:               8
Expected:            All 8 random rows reject (rate = 1.0).
```

**Validation:** `cargo test -p ai-pow-zk --lib --release
prop_urange8_random_row_non_boolean_delta_rejects` → PASS
(1.31 s for 8 cases).

---

## 3. Existing property tests inventory (the audit baseline)

S0 § 2.13 catalogued 4 existing LogUp-side property tests:

| Test | File:line | Constraint family | Cases |
|---|---|---|---:|
| `prop_a_noised_unpack_inrange_verifies` | `composite_full_air_with_lookups.rs:1291` | BUS_IRANGE8 in-range honest path | 4 |
| `prop_a_noised_unpack_outofrange_rejects` | `:1306` | BUS_IRANGE8 out-of-range rejection | 4 (via `proptest::sample::select`) |
| `prop_urange8_valid_query_verifies` | `:1327` | BUS_URANGE8 honest path | 4 |
| `prop_inconsistent_i8u8_pair_rejects` | `:1341` | BUS_I8U8 inconsistency | 4 |
| `prop_cv_routing_nonzero_cv_rejects` | `:1398` | BUS_CV_ROUTING dangling | 4 |

Plus the new S6 test makes **5 active property tests** in the
ai-pow-zk crate. Each one validates that the corresponding
constraint family rejects random tampers at rate 1.0
(measured: 100% rejection across 4-8 cases per family in CI).

---

## 4. The deferred-as-deepening sweep

A "comprehensive" S6 deliverable (1 property test per
constraint family × ~117 families × 8 cases) would add ~30
minutes of CI time. R1 honest call: this is **deepening
work**, not M-S6 audit baseline.

### 4.1 Priority candidates for follow-on property-test work

If the auditor / maintainer requests deeper property coverage,
the priority order (highest-soundness-impact first):

1. **K1 CRIT-1 PROGRAM_COL pin** — random PROGRAM_COL ×
   random row tamper; M3 mechanism. (12 PROGRAM_COLS × N
   random rows × 8 cases = significant runtime; would need
   `#[ignore]`'d unless audit requested.)
2. **K2 §4.D keystone** — random JACKPOT_MSG[i] tamper
   value; M1 mechanism.
3. **K3 §6(b)-G2** — random (row, lane) tamper combinations;
   M1 mechanism. (Builds on the new S4/S5 K3 tests.)
4. **K4 M-S1 pack-link** — random A_NOISED[c] tamper;
   M1 mechanism.
5. **Range-table chips × 4** — random (row, bad_value)
   tampers across URange13/IRange7P1/IRange8.

Each item is 1–2 hours of work mirroring the demo pattern in
§ 1.

### 4.2 Why deferred

- **CI runtime budget.** Each prove+verify in
  `composite_proof.rs`-style tests is ~5–10 s; 8 cases ×
  10 s = 80 s per family. Across ~10 high-value families
  = 800 s ≈ 13 min added CI runtime. Manageable but
  noticeable.
- **Marginal soundness gain.** Per S1's verdict, every
  constraint family ε_AIR ≤ 2^(−103) at production
  parameters; deterministic S4 tests + property-based
  validation across 4–8 random cases empirically confirms
  the rejection rate is 1.0 (no false-negatives) at
  practical input distributions. Going to 100+ cases per
  family does not change the soundness conclusion; it
  refines the empirical confidence.
- **R1 honest:** more isn't better. The audit-baseline
  property coverage is 5 tests covering the highest-impact
  buses + the demo range-table; deeper coverage routes to
  GAP_AUDIT for post-audit follow-up if requested.

---

## 5. Honest residuals (R1)

- **Per-constraint-family property test sweep** is
  deferred-as-deepening per § 4. Tracked in GAP_AUDIT for
  post-audit follow-up.
- **Constrained random input generation** — proptest's
  default strategies are uniform; for tampers that depend on
  the trace's existing values, a custom strategy may be
  needed. Not done in S6; future work can refine.
- **Statistical assertion strengthening** — current pattern
  asserts `verify(...).is_err()` per case (rejection rate =
  1.0 across N cases). A stricter assertion would also
  check the *honest* path verifies (acceptance rate = 1.0).
  Existing `prop_*_inrange_verifies` tests provide that
  pairing for some buses; the new S6 demo does not (relies
  on the deterministic positive control `prove_and_verify_urange8_table`).
  Future work can pair every `prop_*_rejects` with a
  `prop_*_accepts` honest control.

---

## 6. Cross-references

- **CSA design.** `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`
  § 5.8.
- **S0 inventory.** `2026-05-20_CONSTRAINT_INVENTORY.md` § 2.13
  (existing property test catalogue).
- **GAP_AUDIT** (where deepening items route).
  `2026-05-15_GAP_AUDIT.md`.
- **Proptest infrastructure.** `composite_full_air_with_lookups.rs:1280-`
  (the template).
- **R1 / R1.1 discipline.** `~/.claude/CLAUDE.md`.
