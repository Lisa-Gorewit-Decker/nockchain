# 2026-05-21 — End-to-end wall-clock + lb=6 nq=10 sweep + L3-over-L2 evaluation

> _Created **2026-05-21** as a follow-up to the session's parallel +
> mds_cyclomul + Angle A work. Captures the three measurements
> needed to inform Path B vs Path A scoping at the new ≤100 KB
> target + 30 s per-proof budget._

> **Status (R1, honest).** Measurement deliverable; no production
> code edits beyond adding a measurement-only `Lb6Nq10` outer-cert
> tier in `test_tip5_l2_over_l1.rs` (`stage5_tip5_l2_over_l1_lb6_nq10_measurement`).
> The production `goldilocks_tip5_60bit()` builder is unchanged.

## TL;DR

| Measurement | Result | Verdict |
|---|---|---|
| **Real ai-pow PoUW** (PROD @ 8K baseline) | **17.7 s** inner-prove, 204 KB inner-proof | Inner is ~half the 30 s budget |
| **Real ai-pow PoUW** (PROD @ 16K baseline) | **54.8 s** inner-prove, 218 KB inner-proof | Inner ALONE exceeds 30 s budget at 16K |
| **End-to-end per-proof wall-clock estimate** | ~45 s at 8K shape | **OVER 30 s budget by ~15 s** |
| **`lb=6 nq=10` outer cert (Stage 5)** | L1=222 KB (−24%), L2=239 KB (−27%), wall-clock 77 s (+2.5×) | Big size win; **latency cost makes it production-infeasible at the 30 s budget** |
| **L3-over-L2 at 60-bit anchored Johnson** | Residual — Tip5-throughout L3 test absent | Analytical projection only (§3) |

## 1. Real ai-pow PoUW end-to-end wall-clock

### 1.1 Why this matters

Prior `stage5_tip5_l2_over_l1_production_measurement` numbers
(25.1 s pre-Angle-A, 30.4 s post) measured the **L1+L2 recursion
wrap only**, with a Fibonacci-AIR inner (n=8 rows) that completes
in microseconds. **The real per-block budget includes the real
ai-pow PoUW prove cost** on top of L1+L2.

The bench harness at `crates/ai-pow-zk/src/bench_suite.rs` runs the
real ai-pow stack (`CircuitConfig::PROD` = `lb=4 nq=15 pow=1`) on
concrete `(trace_height × non_mining_rows)` shapes and reports
`trace_gen, populate, prove, verify, proof bytes` for each.

### 1.2 Measurement (M2 Max, 10P + 4E cores; release build)

**`bench_prod_8k_baseline`** (8192-row trace, baseline activity):

```
trace_gen  =     10 ms
populate   =      5 ms
prove      = 17,665 ms  (≈ 17.7 s)
verify     =     37 ms
proof      = 208,937 B (≈ 204 KB)
```

**`bench_prod_16k_baseline`** (16384-row trace, baseline activity):

```
trace_gen  =     26 ms
populate   =     14 ms
prove      = 54,766 ms  (≈ 54.8 s)
verify     =     42 ms
proof      = 222,807 B (≈ 218 KB)
```

**Scaling:** 8K → 16K = 2× trace ⇒ 3.1× prove time (17.7 s →
54.8 s). Roughly `O(n log n)` as expected for STARK provers
(FFT + Merkle commit dominate). Production workload size depends
on the inner ai-pow trace at deployed difficulty; the 16K shape
already exceeds the 30 s per-proof budget **on the inner alone**,
before even considering L1+L2.

### 1.3 End-to-end estimate vs the 30 s budget

Combining the real-PoUW prove time with the Stage 5 L1+L2 wrap
cost (extrapolated from the 30.4 s honest-only-no-tamper portion):

| Component | Time | Notes |
|---|---:|---|
| Real ai-pow PoUW inner prove (PROD @ 8K) | 17.7 s | Bench-measured |
| L1 recursion wrap | ~14 s | Stage 5 honest L1 build, est. |
| L2 wrap over L1 | ~12 s | Stage 5 L2 build, est. |
| **Estimated end-to-end** | **≈ 44 s** | **vs 30 s budget** |
| **Gap** | **+14 s over** | — |

**Conclusion:** at production parameters with the real ai-pow
inner, the per-block wall-clock **exceeds the 30 s budget by
roughly 14 s** on M2 Max. The Stage 5 number alone was misleading
— it captured only the recursion-wrap cost, not the real PoUW.

**Latency is now the binding constraint, not just size.** Any
size-reduction lever that costs prover wall-clock must be
evaluated against this budget shortfall, not against the prior
"~5 s of headroom" assumption.

## 2. `lb=6 nq=10` outer-cert Stage 5 measurement

### 2.1 Setup

A measurement-only `Tip5OuterTier::Lb6Nq10` variant added to
`Plonky3-recursion/recursion/tests/test_tip5_l2_over_l1.rs`
(`stage5_tip5_l2_over_l1_lb6_nq10_measurement`). Same paper-proven
Johnson soundness as `Production` (`lb=4 nq=15 pow=1+1` ⇒
`4·15 + 1 + 1 = 62` bits; `lb=6 nq=10 pow=1+1` ⇒ `6·10 + 1 + 1 =
62` bits). Trade-off: 33% fewer FRI queries vs 4× larger LDE
(16× → 64× trace size at the outer cert).

### 2.2 Result

```
serialized L1 = 227,773 B (222.43 KB)
serialized L2 = 244,596 B (238.86 KB)
Stage 5 wall-clock = 77.11 s   (= 2× L1 build + 1× L2 build, with one tamper-rejected attempt)
ACCEPT ✅  TAMPER-REJECT ✅
```

Comparison vs Production `lb=4 nq=15` (post-Angle-A):

| Metric | `lb=4 nq=15` (PROD) | `lb=6 nq=10` | Δ |
|---|---:|---:|---:|
| L1 bytes | 292.92 KB | **222.43 KB** | **−24.1%** |
| L2 bytes (cert) | 328.83 KB | **238.86 KB** | **−27.4%** |
| Stage 5 wall-clock | 30.4 s | **77.1 s** | **+2.5× slower** |
| Vs ≤100 KB target | 3.29× over | **2.39× over** | Closes ~30% of the gap |

### 2.3 Interpretation

**Size win is real and substantial** — `lb=6 nq=10` saves
~25-27% on both L1 and L2, closing the ≤100 KB gap from 3.29×
to 2.39× over. The fewer FRI queries directly reduce Merkle
path opens (33% fewer queries × slightly deeper paths).

**Latency cost is prohibitive at the 30 s budget.** Stage 5
wall-clock 2.5× higher means the outer-cert L1+L2 portion alone
takes ~63 s (vs ~26 s at PROD). Adding the real ai-pow inner
(17.7 s) gives an end-to-end estimate of **~80-90 s** per proof
at lb=6 nq=10 — nearly 3× over the 30 s budget.

**Verdict:** `lb=6 nq=10` is **not a viable production
deployment** at the current 30 s wall-clock budget. The 4× LDE
(16× → 64× trace) cost at the outer cert dominates whatever
prover-side speedup the 33% query reduction buys. Reserved as
a *measurement comparator* — useful for projecting Path B + Path
A combined outcomes (a Path A SNARK wrap over a 239 KB L2 has
less to do than over a 329 KB L2), but not deployable as-is.

## 3. L3-over-L2 stacked recursion at the 60-bit anchored Johnson

### 3.1 Status — honest residual

**There is no Tip5-throughout L3-over-L2 test in the codebase.**
The existing `s3ii_l3_over_l2_120bit` test
(`Plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs:1056`)
uses **Poseidon2** for its L2 verifier circuit
(`enable_poseidon2_perm_width_8`, `Poseidon2Config::GOLDILOCKS_D2_W8`)
and runs at the legacy `OuterTier::Bit120` (`lb=2 nq=42 = 86 bits`)
FRI tier. Per the `no_poseidon2_anywhere` hard rule, this test
**cannot** be invoked as a Tip5-throughout measurement with a
disclaimer.

### 3.2 Analytical projection (no empirical measurement)

Prior 2026-05-19 finding (at the **old 80-bit Johnson** baseline):
*"L3 > L2 ⇒ stacked recursion confirmed-dead."* That conclusion
was anchored on L2 ≈ 619 KB (old Poseidon2-based, lb=2 nq=42).

At the new 60-bit anchored Johnson with Tip5-throughout +
`mds_cyclomul` + Angle A (this session's measurements):

- L1 = 292.92 KB  
- L2 = 328.83 KB  
- **L2/L1 expansion ratio = 1.122×**

If L3 follows the same per-recursion-layer expansion ratio
(plausible — each recursion layer wraps the previous with its
own verifier-circuit overhead, which is dominated by the FRI
verification of the inner proof, ~independent of the inner
proof's content):

  **L3 ≈ L2 × 1.122 = 329 × 1.122 ≈ 369 KB**

This is **40 KB LARGER than L2 (+12%)** — consistent with the
prior 2026-05-19 conclusion that *L3 > L2 ⇒ stacking dead*. The
constant factor `1.122×` is what matters: as long as each new
recursion layer is BIGGER than the previous (overhead > inner
content compression), stacking is a net negative.

**Caveats on the projection:**
- The L2/L1 ratio was measured at lb=4 nq=15. At different FRI
  params the ratio could shift; at `lb=6 nq=10` (§2.2)
  L2/L1 = 239/222 = 1.074× — smaller expansion ratio. Suggests
  higher-lb outer might bring the per-layer expansion below 1×
  in some regime, but this regime is already infeasible on
  latency (§2.3).
- The projection assumes the L3 verifier circuit at the
  60-bit anchored params has the same shape as L2's. This is
  approximately true (same Tip5 NPO chip, same constraint
  structure), but the exact L3 size could differ by a few %
  due to LDE rounding to next power of 2.

**Verdict on stacked recursion at the new anchored floor:**
projection says L3 ≈ 369 KB > L2 = 329 KB ⇒ **stacking still
appears dead.** The 2026-05-19 conclusion holds at the new
floor; we cannot compound recursion further to shrink the
exchanged cert.

### 3.3 Implementation residual

Confirming the §3.2 projection with empirical measurement
requires writing a Tip5-throughout `stage6_tip5_l3_over_l2_*`
test (~1 day). The projection's `1.122×` ratio is grounded in
hard L1+L2 measurements at the same FRI tier, so the empirical
L3 measurement would have to deviate significantly from the
ratio to flip the verdict. Low-priority follow-up; the
analytical signal is strong enough to deprioritize the test
until other Path B levers are exhausted.

### 3.3 Implementation residual

Writing a Tip5-throughout `stage6_tip5_l3_over_l2_*` test
(~1 day) is the open work to close this evaluation. Sketch of
what's needed:
- L3 verifier circuit: mirror `l2_over_tip5_l1` but consume a
  `BatchStarkProof<TipsCfg>` (the L2) as its inner-witnessed
  proof.
- Reuse `inner_tip5_npo_provers()` for the L3-side Tip5 NPO
  dispatch (Stage 3 wiring already in place).
- `make_tip5_outer_cfg(Tip5OuterTier::Production)` for the L3
  outer FRI.
- Stage 6 test runs L3 over L2, reports `L3 bytes` and
  `L3 wall-clock`, asserts ACCEPT + TAMPER-REJECT.

If `L3 < L2`, this is a new lever (stack further → L4 over L3
→ ... until the recursion overhead exceeds the wrap's savings).
If `L3 ≥ L2`, the 2026-05-19 conclusion still holds at the new
floor; stacking remains a dead end and Path A is the only
remaining size lever.

## 4. Synthesis — implications for Path B vs Path A

### 4.1 The two binding constraints, post-measurement

1. **Latency (30 s per proof):** **over budget by ~14 s** at the
   production params (real ai-pow inner + L1 + L2). Levers
   that COST latency are risky.
2. **Size (≤100 KB cert):** L2 = 329 KB ⇒ **3.29× over target**.
   Closing this via Path B alone requires soundness-orthogonal
   work that doesn't add latency.

### 4.2 What this means for the levers we've been weighing

| Lever | Size win | Latency cost | Viable? |
|---|---|---|---|
| **Angle A** (Tip5 A-col elim) — **DONE** | L1 −4.6%, L2 −4.1% | +5 s Stage 5 (constraint clone overhead) | ✓ landed |
| **lb=6 nq=10 outer** (§2.2 measured) | **L1 −24%, L2 −27%** | **+47 s Stage 5 (+2.5×)** | ✗ **blows the latency budget** |
| **Angle B** (Tip5 multi-row packing) | ~20% L1 (projected) | Neutral / favourable (fewer rows → smaller LDE per perm) | ✓ promising — biggest latency-friendly lever remaining |
| **WHIR @ Johnson** | ~10% L1 (per WHIR prototype Sweep 1) | Neutral | ✓ orthogonal |
| **Tighter serialization** | 5-15% L1 | Neutral (encoding-only) | ✓ orthogonal |
| **L3-over-L2** (§3 projection) | **NEGATIVE — L3 > L2** | Adds full layer prove | ✗ stacking still dead at 60-bit |
| **Inner ai-pow PoUW optimization** | Doesn't shrink cert | **Reduces inner prove (17.7 s today)** | ✓ **highest-priority latency lever** |
| **Path A SNARK wrap** | Reaches ≤100 KB | +SNARK prove time (likely seconds-to-minutes) | Strategic — final lever |

### 4.3 Recommended next decisions

Given that latency (~14 s over budget) is the BINDING constraint:

1. **Levers with neutral latency cost** (Angle B refactor,
   WHIR @ Johnson, tighter serialization) should be prioritized
   over levers with positive latency cost.
2. **The latency budget itself needs a re-discussion** with the
   maintainer. Options:
   - Accept end-to-end ~45 s and revise the per-proof budget
     (consensus-side analysis: does the 2.5-min block cadence
     allow 45 s for prove + 105 s for prop/verify? On
     non-M2-Max miners with faster CPUs, the 45 s number may
     drop further).
   - Spend prover-side optimization budget on the inner ai-pow
     PoUW (currently 17.7 s for prove on 8192 rows; reducing
     this by ~1.5× via further AIR / SIMD work would close
     much of the latency gap).
   - Re-evaluate which AIR families dominate the inner prove
     (composite_trace.rs is the big inner; profiling could
     show where the 17.7 s goes).
3. **`s3ii_*` port to Tip5-throughout** is the unblocking work
   for the L3-over-L2 question; ~1 day's work, gates the
   stacked-recursion decision.

## 5. Files added / modified for these measurements

- `Plonky3-recursion/recursion/tests/test_tip5_l2_over_l1.rs`:
  added `Tip5OuterTier::Lb6Nq10` variant + corresponding `name`,
  `fri`, `unconditional_bits` arms; added
  `stage5_tip5_l2_over_l1_lb6_nq10_measurement` test (`#[ignore]`).
- _This doc._
