# Path B Stage B0 — production L1 cert inventory

**Date:** 2026-05-20
**Run:** commit `e2b791b` (post-Phase-0 production FRI:
`lb=4 nq=20 mla=3 lfp=2 cap=3 d=5`, 82 bits unconditional
Johnson, Tip5-throughout substrate).
**Test:** `crates/plonky3-recursion/recursion/tests/test_path_b_b0_inventory.rs::
path_b_stage_0_l1_inventory` (123.20s).

## Headline numbers (production-faithful)

**L1 outer-cert serialized = 499,353 bytes (487.65 KB) at the
LANDED production FRI.**

This is the **first production-faithful L1 measurement** —
prior Stage 5 measurements
(`test_tip5_l2_over_l1.rs::stage5_*`) used `cap=0` in the test
infrastructure's `make_tip5_outer_cfg`, while the production
builder uses `cap=3`. The cap divergence overstated those
measurements by ~5%; fixed in this Stage B0 work.

## Per-table breakdown

### Primitive tables (Const, Public, Alu)

| Table | Rows | % of all rows |
|---|--:|--:|
| Const | 83 | 1.0% |
| Public | 33 | 0.4% |
| **Alu** | **6,851** | **84.3%** |
| **subtotal primitive** | **6,967** | 85.7% |

### Non-primitive (NPO) tables

| Table (op_type) | Rows | Lanes | Packed rows | % of all rows |
|---|--:|--:|--:|--:|
| `tip5_perm/goldilocks_w16_r7` | 881 | 1 | 881 | 10.8% |
| `recompose` | 96 | 1 | 96 | 1.2% |
| `recompose/coeff` | 187 | 1 | 187 | 2.3% |
| **subtotal NPO** | **1,164** | — | 1,164 | 14.3% |

### Grand total

8,131 rows total. **Alu dominates with 84% of all rows.**

## Per-section byte breakdown

| Proof section | Bytes | % of L1 |
|---|--:|--:|
| commitments | 1,143 | 0.2% |
| opened_values | 65,386 | 13.1% |
| **opening_proof (FRI)** | **423,858** | **84.9%** |
| global_lookup_data | 8,467 | 1.7% |
| non_primitives (metadata) | 69 | 0.0% |
| **TOTAL** | **499,353** | 100% |

**The FRI opening proof is 85% of L1 bytes.** All other
sections combined are ~15%.

## Cross-cutting observations

1. **Alu table is the verifier circuit's bulk** (6,851 of
   8,131 rows). This is where the FRI fold equations, OOD
   constraint evaluation, Lagrange-basis arithmetic, and
   quotient recomposition live. **Any Path B reduction must
   target Alu reduction to move L1 size meaningfully.**

2. **`tip5_perm` NPO has 881 rows.** Each Tip5 perm call in
   the verifier circuit produces one row in this table. The
   call sources are: Fiat-Shamir challenger absorbs (every
   commitment), MMCS path verification (every opening's
   Merkle authentication), FRI commit-phase hashes (each
   query's fold-chain).

3. **Recompose NPO has 96 + 187 = 283 rows.** These are
   Goldilocks↔Challenge conversions. Some may be inlineable.

4. **`non_primitives` metadata section is only 69 bytes** —
   the actual NPO trace bytes are inside `opening_proof` (the
   FRI commit-phase commits to the NPO tables along with
   primitive tables).

5. **`opening_proof` (84.9%) dominates bytes**, not row
   counts. This is the FRI fold-chain commitments + opened
   values for every query. FRI parameters (lb, nq, mla, lfp,
   cap) already maxed out in-substrate; further FRI-side
   reduction requires Tier C (digest=4 paper-divergence) or
   Path A (substrate replacement).

6. **Cell count would be more informative than row count.** A
   row in Alu has ~30-50 columns (per `AluAir::total_width`);
   a row in tip5_perm has ~886 columns (per the Tip5 lookup-
   table AIR). So 881 tip5_perm rows × 886 cols = 780K cells
   ≈ 6851 Alu rows × ~50 cols = 343K cells. **The
   `tip5_perm` table likely dominates by CELL count** even
   though Alu dominates by row count. Verifying this needs
   per-column-width instrumentation (next refinement of B0
   if Path B continues).

## Implications for B1 (reduction map)

**Two reduction families to investigate, in priority order:**

### Family A — Reduce Alu rows (~6,851 currently)

Sub-areas inside Alu:

- **FRI fold equations** (~140 fold-step rows × ~50 cols ≈
  7K cells): one fold step per query per FRI round. Can
  potentially use a custom AIR instead of generic-DSL-
  generated Alu rows.
- **OOD constraint evaluation** (per the inner Tip5-L0 AIR's
  constraints, evaluated at zeta + zeta_next): scales with
  inner AIR complexity, not easy to shrink directly.
- **Lagrange basis evaluations** (per opened point): shared
  across all openings; potentially hoistable to a preprocessed
  helper.
- **Quotient polynomial recomposition** (per quotient chunk):
  scales with quotient degree.

### Family B — Reduce `tip5_perm` calls (~881 currently)

Sub-areas:

- **Fiat-Shamir absorbs** (~few dozen): one per
  commitment / opened value batch. Could batch-absorb.
- **MMCS path hashes** (~20 queries × ~7 levels × ~3
  commitments = 420 perms): the bulk of perms. Reducing path
  depth (`cap` already at 3) requires either further-cap or
  trace-height reduction.
- **FRI commit-phase hashes** (~20 queries × ~7 fold rounds
  = 140 perms): tied to FRI structure.

### Family C — Recompose savings (~283 currently)

- Inline simple base→Challenge conversions in the main
  circuit (avoid NPO call overhead).
- Batch sibling recompose calls.

### Family D — Out-of-band

- **Digest=4 (Tier C)** would shrink every tip5_perm output
  by 1 element (20% cell savings on tip5_perm). Paper-
  divergent; documented separately in
  [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md).
- **Reduced-round Tip5** (e.g., 5-round paper-spec): would
  shrink tip5_perm rows in proportion. Paper-divergent for
  Nockchain's 7-round deployment; the 2024/1900
  cryptanalysis paper recommends margins above 5 rounds.

## R1 honest residuals (Stage B0)

1. **Per-column width** within each AIR is not yet measured.
   Row counts alone don't tell us cell counts. The next
   refinement would be to reconstruct each AIR and call
   `width()` + `preprocessed_width()` — small extension to
   the inventory test.
2. **The `opening_proof` 85% slice is opaque** — postcard
   doesn't break it down further. Would need to instrument
   the FRI prover to dump per-query byte counts.
3. **Same instrumentation needed at L2** to confirm the
   cascading-effect hypothesis from Phase 0 (where the
   soundness-neutral levers compounded more at L2).

## Next: B1 reduction map

With B0 data in hand, B1 (the reduction map) can produce
specific column-by-column proposals with soundness
equivalence arguments. The data says Alu is the biggest
table by row count and `tip5_perm` is likely the biggest by
cell count. Family A and Family B reductions are both worth
mapping.

## Cross-references

- [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md)
- [`2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md`](2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md)
- [`2026-05-20_TIP5_NPO_RECURSION_BACKEND_DESIGN.md`](2026-05-20_TIP5_NPO_RECURSION_BACKEND_DESIGN.md)
- Inventory tool: `crates/plonky3-recursion/recursion/tests/test_path_b_b0_inventory.rs`
