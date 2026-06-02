> _Created **2026-05-20** · last updated **2026-05-20**._

# S2 — Tamper-coverage gap list: actionable backlog for S3 + S4

> **Status (R1, honest).** S2 LANDED. Stage S2 of
> `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`.
> Consumes S0 inventory (`2026-05-20_CONSTRAINT_INVENTORY.md`)
> + S1 derivation (`2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md`).
> Produces the prioritized S3 backlog: every constraint family
> categorized G1/G2/G3 with concrete dispositions.
>
> **Verdict.** S1 verified every AIR + bus ≥98 unconditional
> bits. **No soundness gaps.** S2's job is purely coverage:
> verify every constraint has a tamper test exercising its
> rejection. After S0's first pass and S2's refinement, the
> in-scope actionable backlog is:
>
> - **0 GAP-G3 in-scope items requiring new tamper-test design.**
>   The h_a/h_b item that S0 conservatively marked G3 is
>   reclassified as **G1 — subsumed** after closer analysis
>   (§3.2 below).
> - **4 GAP-G2 rename-only items** with concrete S3/S4 actions
>   (§4 below).
> - **3 deferred GAP-G3** (M12 / `#127` — BUS_MATMUL_INPUT,
>   BUS_JACKPOT_X_BITS, Tip5 D=2 R-a tail), explicitly out of
>   M-S6 scope.
> - **S5 cross-AIR composition coverage** (the F3–F20 fine-grained
>   per-FRI-fold-round tampers) — handled in S5, not S3/S4.
>
> **Net S3/S4 in-scope work: 4 GAP-G2 rename tests + 0 new
> tamper designs.** The audit hardness is comprehensive at
> ~190 existing tests; the gaps S0 surfaced were almost all
> labeling / naming questions rather than missing coverage.

---

## 1. Methodology

For each GAP flagged in S0, S2 walks the following decision tree:

```
GAP flagged in S0 (G1 / G2 / G3)
  │
  ├── Is there an existing test that covers the constraint?
  │     ├── YES, named for it          → confirmed coverage; remove from gap list
  │     ├── YES, but unnamed for it    → G2 (rename-only); add to S3 backlog
  │     └── YES, but indirectly        → check if direct test is needed (G1 or G3?)
  │
  ├── Is the constraint covered by another constraint's tamper?
  │     ├── YES (subsumed by a stronger tamper) → G1 (subsumed); document, drop from backlog
  │     └── NO                                  → G3 (missing); add to S3 backlog
  │
  └── Is the constraint deferred to a later milestone (M12, M-S5b, Phase D)?
        └── YES → out of scope; track as M-S6 residual; do NOT add to S3 backlog
```

S0's first-pass categorization was conservative (any uncertainty
→ G3); S2 refines after closer inspection.

---

## 2. Resolved GAP-G2 — rename-only items (S3 backlog)

These items are covered by existing tests, but the test names
don't carry the explicit cross-link to the constraint family /
soundness claim they discharge. Action: in S3, design a
**rename-only** test (or add the explicit label as a doc-comment
referencing the existing test); in S4, no new code beyond the
label/test-alias.

### 2.1 URange13Chip / IRange7P1Chip explicit names

**S0 finding** (§2.2):
> ⚠️ GAP-G2 for URange13 + IRange7P1 (no explicit table-side
> rejection tests named for these chips — but they share the
> parameterized range-table test infrastructure, and the
> LogUp-side coverage of each bus covers indirect rejection).

**Resolution.**
- `range_table.rs` has parametric tests (`urange8_*`, `irange8_*`)
  that exercise the *same* code path (`RangeTableChip<…>::eval()`)
  with different bounds. The U13 / I7P1 variants only differ
  by the `(MIN, MAX)` const generic.
- The LogUp-side has explicit `out_of_range_*` rejection tests
  per bus (`out_of_range_mat_id_limb_rejected_by_logup` for
  URange13; `out_of_range_noise_unpack_rejected_by_logup` for
  IRange7P1) — these cover the practical rejection at the
  *consumer* side.
- The constraint-side rejection (chip's own AIR constraints
  on `TABLE[0]`, `TABLE[N-1]`, `delta ∈ {0,1}`) is exercised
  whenever the chip is included in the composite trace.

**S3 backlog item U13-IR7P1-RENAME:**
- Add an explicit `urange13_table_chip_constraint_rejections` test
  (mirrors `urange8_verify_rejects_*` for U13 bounds).
- Add an explicit `irange7p1_table_chip_constraint_rejections` test
  (mirrors `irange8_*` for I7P1 bounds).
- Each test calls the existing `RangeTableChip<U13, …>::eval()`
  with deliberately-bad tables; verifies rejection.

**Estimated effort:** 2–4 hours (mostly boilerplate from existing
`urange8_*` template).

### 2.2 K3 §6(b)-G2 keystone explicit name

**S0 finding** (§2.12):
> ⚠️ GAP-G2 for K3 (no test explicitly named for §6(b)-G2;
> covered implicitly by `high2_2_fold_chain_pinned_logup` —
> relabel-only).

**Resolution.**
- `high2_2_fold_chain_pinned_logup@composite_proof.rs:726` is
  the existing happy-path test exercising K3 (FOLD_XSTEP ==
  SX_XR[stripe]) under production geometry.
- The §6(b)-G2 constraint is `Σ_s FOLD_STRIPE_SEL[s] · (FOLD_XSTEP
  − SX_XR[s]) = 0` (composite_full_air.rs:329–333). A tamper
  that perturbs FOLD_XSTEP to a different SX_XR lane should
  reject.

**S3 backlog item K3-G2-EXPLICIT:**
- Add an explicit `high2_2_g2_xstep_stripe_pin_rejects` test:
  build an honest trace, tamper FOLD_XSTEP[r] to equal a
  *different* SX_XR lane than FOLD_STRIPE_SEL claims, verify
  rejection (M1: AIR `eval()` non-zero).

**Estimated effort:** 4–8 hours (requires understanding the
composite-trace-builder API to inject the specific tamper).

### 2.3 Poseidon2 / Poseidon1 perm AIR — upstream-routing label

**S0 finding** (§4.1, §4.2):
> ⚠️ GAP-G2 (upstream tested, but no in-tree label for the
> specific tampers). Action: at audit time, route to upstream
> Plonky3 test inventory.

**Resolution.**
- The Poseidon2 + Poseidon1 perm AIRs are *vendored from
  upstream Plonky3* at rev `c2c51fb` (C1 substrate). Upstream
  has its own tamper test suite; our deployment inherits.
- Our in-tree audit's job is to verify the *correct version*
  is vendored and the parameters match Plonky3's published
  soundness analysis. No in-tree tamper test needs to be
  *added*; the audit-readiness pointer is sufficient.

**S3 backlog item POS-UPSTREAM-LABEL:**
- Add a doc-comment in `crates/plonky3-recursion/poseidon2-circuit-air/src/lib.rs`
  (or the vendoring README) explicitly stating: "Poseidon2 perm
  AIR is vendored upstream; tamper coverage is upstream Plonky3
  test suite + indirect via c3_stage_a/b/c via the L1+L2
  composite outer-cert tests."
- No new test code; documentation-only.

**Estimated effort:** 1–2 hours (doc-only).

### 2.4 GAP-G2 rollup

| Item | S3 spec | S4 impl | Effort |
|---|---|---|---|
| U13-IR7P1-RENAME | Spec the 2 new tests | Implement (mirror urange8) | 2–4 h |
| K3-G2-EXPLICIT | Spec the tamper variant | Implement + paired acceptance | 4–8 h |
| POS-UPSTREAM-LABEL | Spec the doc-comment | Add doc | 1–2 h |
| **Total GAP-G2 work** | — | — | **7–14 hours** |

---

## 3. Resolved GAP-G3 — subsumed (G1 after S2 refinement)

S0's conservative classification flagged 1 in-scope GAP-G3.
After closer analysis, this item is **subsumed** by existing
tamper coverage at a different layer.

### 3.1 h_a/h_b circuit-side dedicated tamper — RECLASSIFIED to G1

**S0 finding** (§6.3 + §2.13 BUS_NOISED_PACKED):
> The h_a/h_b zero-gap circuit-side test (production 16∣r
> geometry) | Existing `sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects`
> covers c-exact; explicit h_a / h_b root binding at strip-opening
> leaf rows needs a dedicated tamper. **S4.B deliverable.**

**S2 refinement.**

Question: does the *SNARK circuit* need a dedicated h_a/h_b
root tamper, or is the existing position-exact test plus the
ai-pow-side root tampers (`reject_tampered_h_a@adversarial.rs:44`,
`reject_tampered_h_b@adversarial.rs:63`) sufficient?

Walking the binding chain:

1. **ai-pow extraction layer.** The miner computes `H_A`,
   `H_B` from the noised matrix; the verifier expects them
   as block PIs.
   - **ai-pow Tamper coverage:** ✅ `reject_tampered_h_a`,
     `reject_tampered_h_b` (M5: Merkle path mismatch at
     extraction).

2. **SNARK PI layer.** The SNARK takes `HASH_A`, `HASH_B`
   as public inputs (= `H_A`, `H_B` from extraction).
   - **SNARK Tamper coverage:** ✅ `full_air_rejects_tampered_hash_a_pi@composite_trace.rs:3033`
     (M1: PI binding inside the AIR — the constraint
     `BLAKE3_OUT[r] == HASH_A_PI` fails when PI is tampered).

3. **SNARK circuit binding to leaf rows.** The §4.C.2 / cx.0
   constraint binds `UINT8_DATA[0..64]` (the strip-opening
   leaf rows) ↔ `BLAKE3_MSG` ∈ `HASH_A` via the C3 identity.
   - **Tamper coverage:** ✅ `sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects@zk_bridge.rs:2485`
     (M1: position-exact byte-level tamper at leaf rows;
     the constraint fails).

So the binding is covered at all three layers:
- Extraction layer: h_a/h_b leaf/path/root tampers (M5).
- PI layer: HASH_A/HASH_B tampers (M1).
- Circuit-leaf-row layer: byte-level tampers (M1).

**A dedicated "h_a/h_b root binding at strip-opening leaf
rows" tamper would not exercise a new rejection mechanism**
— it would tamper the same `UINT8_DATA[0..64]` region that
the existing position-exact test already covers, just from
a different conceptual angle (root-side vs leaf-side).

**Disposition: GAP-G1 (subsumed).** No new tamper test
needed. The audit's understanding of "h_a/h_b binding" is
already covered by:
- ai-pow side: `reject_tampered_h_a`, `reject_tampered_h_b`
- PI side: `full_air_rejects_tampered_hash_a_pi`
- Circuit-binding side: `sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects`

**Document the subsumption** (S3 deliverable):
- Add a comment to `sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects`
  explicitly stating it covers the h_a/h_b root-to-leaf
  binding via the position-exact byte-level tamper.
- Add an audit-readiness note in C4 §3.4 / §7 cross-linking
  the three test layers explicitly.

**Estimated effort:** 1–2 hours (doc-only).

---

## 4. Out-of-scope GAP-G3 — deferred to M12 / `#127`

These are explicitly deferred per `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`
and `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13. **Not in M-S6
audit scope.**

### 4.1 BUS_MATMUL_INPUT (M-S1 matmul-input pin)

- **S0 finding:** ⚠️ GAP-G3 — C2/C3 follow-on per
  `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`; M12 / `#127`.
- **Status:** Designed, wiring deferred. The constraint alone
  (K4 M-S1 pack-link) is sound; the full M-S1 closure (bus +
  producer) unlocks matmul soundness end-to-end. K4 + the
  existing tamper tests on K4 cover the pack-link side
  (one of the two prongs of M-S1).
- **S2 disposition:** Out of scope for M-S6 (the audit). The
  bus residual is tracked as M12 / `#127` and noted in C4 §8.

### 4.2 BUS_JACKPOT_X_BITS (HIGH-2.2 §4.D bit decomp lookup)

- **S0 finding:** ⚠️ GAP-G3 — deferred per chip docstring.
- **Status:** POC wiring; bit decomp ↔ CUMSUM_BUFFER lookup
  pending. The §4.D keystone (K2: JACKPOT_MSG == FOLD_STATE)
  is sound; the bit-decomp-side lookup adds a refinement
  that closes A-TILE / MED-3 binding.
- **S2 disposition:** Out of scope for M-S6.

### 4.3 W2 D=2 WitnessChecks output-receive (Tip5 R-a tail)

- **S0 finding:** ⚠️ GAP-G3 — D=2 recompose-coeff producer
  multiplicity imbalance on Tip5 verifier circuit; tracked as
  M12 / `#127`.
- **Status:** D=1 byte-identical re-validated (`632cb8c`);
  D=2 has an orphan ±1 at wid 11468 (single location). The
  R-a tail does not break C3 soundness at D=1 (the production
  config); only affects future M12 use cases.
- **S2 disposition:** Out of scope for M-S6 (production
  config is D=1).

### 4.4 R2 D=2 per-coefficient receive (Plonky3-recursion recompose)

- **S0 finding:** Same as W2 — M12-deferred.
- **S2 disposition:** Out of scope for M-S6.

### 4.5 Deferred-G3 rollup

| Item | Disposition |
|---|---|
| BUS_MATMUL_INPUT | Out of scope; tracked M12 |
| BUS_JACKPOT_X_BITS | Out of scope; deferred |
| W2 D=2 R-a tail | Out of scope; D=1 production |
| R2 D=2 per-coefficient | Out of scope; D=1 production |

**Total deferred-G3 work in M-S6 scope: 0.** Tracked in C4
§8 (already noted in the existing residual rows).

---

## 5. S5 work — cross-AIR composition tampers

These are NOT GAP-G3 items in S2/S3/S4 scope; they're S5
work per CSA design §5.7. Listed here for cross-reference:

| Boundary | Currently covered by | S5 deliverable |
|---|---|---|
| FOLD_STATE → JACKPOT_MSG (§4.D) | K2 `high2_2_jackpot_nonzero_msg_unit` + `high2_free_jackpot_message_rejected` | ✅ covered; cross-AIR composition explicit-label in S5 |
| FOLD_XSTEP → SX_XR (§6(b)-G2) | Will be covered by K3-G2-EXPLICIT (§2.2) | S5 add cross-tamper at *producer* side (perturb SX_XR not FOLD_XSTEP) |
| CUMSUM_TILE → SX_IN | `rejects_tampered_register@stripe_xor.rs:570` (consumer side) | S5 add cross-tamper at *producer* side (matmul outputs perturbed; SX_IN should diverge) |
| A_NOISED packed ↔ unpack (M-S1) | K4 + matmul tests; LogUp `prop_a_noised_unpack_outofrange_rejects` | S5 add explicit pack/unpack inconsistency on a matmul row |
| BLAKE3 CV → CV_IN routing | `cv_routing_*` (4 tests) | ✅ covered; S5 extends with cross-row dangling tampers |
| Tip5 input ↔ recompose D=2 | M12-deferred (R-a tail) | Out of scope for M-S6 |
| Inner Tip5-L0 cert → L1 outer-cert | `c3_stage_a/b/c_*` | ✅ comprehensive |
| L1 → L2 outer-cert | `c3_stage_b_l2_over_120bit_l1` | ✅ comprehensive |
| F3–F20 fine-grained FRI fold-round constraints | Currently indirect via c3_stage; S5 design + impl explicit per-round tampers | S5 work |

**S5 has 4 new cross-AIR tamper tests to design + implement
in M-S6 scope.** See `2026-05-20_CSA_S5_CROSS_AIR_TAMPER_TESTS.md`
(next deliverable after S3+S4 land).

---

## 6. S3 prioritized backlog

After S2 categorization, the actionable S3 backlog is:

| Priority | Item | Type | S4 effort |
|---:|---|---|---:|
| 1 | U13-IR7P1-RENAME (§2.1) | New tests (boilerplate from urange8 template) | 2–4 h |
| 2 | K3-G2-EXPLICIT (§2.2) | New tamper variant on K3 keystone | 4–8 h |
| 3 | h_a/h_b subsumption documentation (§3.1) | Doc-comment + C4 §3.4 cross-link | 1–2 h |
| 4 | POS-UPSTREAM-LABEL (§2.3) | Doc-comment only | 1–2 h |

**Total S3+S4 work: 8–16 hours of focused effort.** All
items are low-R1-risk (no fenced-linchpin edits; tests-only
+ docs).

S5 (cross-AIR) and S6 (property) work is sequenced after S3+S4
lands per CSA design §5.

---

## 7. Verdict + S3 input

> **S2 verdict.** S0's first-pass identified 5 gaps;
> S2 refinement resolves them as:
> - **1 GAP-G1** (subsumed, doc-only): h_a/h_b at circuit
>   layer is covered by the existing position-exact byte-level
>   tamper + ai-pow-side root tampers + PI tamper test.
> - **3 GAP-G2** (rename-only): URange13/IRange7P1 names,
>   K3 §6(b)-G2 explicit name, Poseidon2/1 upstream label.
> - **3 deferred** (out of M-S6 scope): BUS_MATMUL_INPUT,
>   BUS_JACKPOT_X_BITS, Tip5 D=2 R-a tail.
> - **0 in-scope new tamper designs needed** beyond the 4
>   G2 rename items.
>
> The audit is fundamentally well-covered. S3 spec + S4 impl
> work is ~8–16 hours (tests-only + docs). S5 cross-AIR work
> adds another ~4 new tests.

**Output for S3.** S3 consumes:
- The §6 prioritized backlog (4 items).
- The §4.2 tamper-test template from CSA design.

---

## 8. Honest residuals (R1)

What S2 does not address, deferred:

1. **F3–F20 fine-grained per-FRI-fold-round tamper tests**
   are S5 cross-AIR work; not S3 backlog. The current
   indirect coverage via `c3_stage_a/b/c` is sufficient
   for the audit baseline; S5 adds direct per-fold tests
   for the audit's depth claim.

2. **Property-based variant of every existing tamper test**
   is S6 work, not S2/S3/S4 scope. Existing `prop_*` tests
   are starting points; S6 expands.

3. **Per-test runtime budget audit** — S4's tests might
   slow CI. R1 honest call: if CI runtime grows past a
   manageable threshold, S7 audit sign-off can rationalize
   which tests `#[ignore]` (manual-invocation gate) vs CI
   default.

---

## 9. Cross-references

- **S0 inventory.** `2026-05-20_CONSTRAINT_INVENTORY.md`.
- **S1 derivation.** `2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md`.
- **CSA design.** `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`.
- **C4 audit-readiness.** `2026-05-19_C4_AUDIT_READINESS.md`
  § 3 + § 7 + § 8.
- **GAP_AUDIT** (where audit-time findings route).
  `2026-05-15_GAP_AUDIT.md`.
- **M12 / `#127` tracking.** `2026-05-14_M10_1C_DESIGN.md` +
  `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13.
- **R1 / R1.1.** `~/.claude/CLAUDE.md`.
