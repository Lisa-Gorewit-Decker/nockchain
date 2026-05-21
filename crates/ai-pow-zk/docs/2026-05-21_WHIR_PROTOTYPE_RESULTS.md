# WHIR PCS feasibility prototype — empirical results

**Date:** 2026-05-21
**Source:** `crates/ai-pow-zk/tests/test_whir_prototype.rs`
**Plonky3 rev:** `82cfad7` (matches our other `p3-*` deps via `Cargo.toml`'s `https://github.com/Plonky3/Plonky3.git` default branch).
**Status:** Measurement complete. R1 honest residual: integration of WHIR into our STARK is NOT done — the prototype only measures per-polynomial PCS bytes.

## TL;DR

| Configuration | Per-PCS bytes (vs FRI@Johnson) | Soundness | Trust surface impact |
|---|---|---|---|
| Current FRI @ JohnsonBound | 1.0× (baseline) | proven (IACR 2025/2055 Thm 1.5) | shipped |
| **WHIR @ JohnsonBound** | **0.86-0.95×** (5-15% smaller) | proven (same) | new substrate (~3 weeks integration) |
| **WHIR @ CapacityBound** | **0.44-0.49×** (~2.1× smaller) | **CONJECTURED**, not proven | new substrate **+ stronger assumption** |

**Key finding:** WHIR's much-marketed "3-5× smaller" claim **only materializes at CapacityBound soundness**, which is unproven and would require maintainer sign-off on the stronger assumption. At our production-faithful JohnsonBound, WHIR saves only 5-15% per PCS.

## Test setup

**Substrate held constant (both PCS):**

- Base field: Goldilocks (`p = 2^64 − 2^32 + 1`).
- Extension: `BinomialExtensionField<Goldilocks, 2>` (matches our Challenge).
- Hash: `Poseidon2Goldilocks<8>` via `PaddingFreeSponge<_, 8, 4, 4>` + `TruncatedPermutation<_, 2, 4, 8>`.
- MMCS: `MerkleTreeMmcs` with cap=0.
- Security target: ≥80 bits.
- PoW bits: 2 total (matches our FRI cpow=1 + qpow=1).
- log_inv_rate: 4 (matches our `log_blowup = 4`).

**Each measurement:** commit + open one polynomial of size `2^num_vars`, serialize commit + opening proof via postcard, report total bytes + open ms + verify ms.

## Sweep 1 — WHIR @ JohnsonBound vs FRI @ JohnsonBound

Production-faithful comparison: both PCS use the proven soundness bound.

| log_size | WHIR bytes | FRI bytes | WHIR/FRI ratio | WHIR open(ms) | FRI open(ms) |
|---:|---:|---:|---:|---:|---:|
| 2^12 (4 K elts) | 43,084 | 48,733 | **0.884×** | 4 | 9 |
| 2^14 (16 K) | 48,805 | 57,077 | **0.855×** | 10 | 26 |
| 2^16 (64 K) | 66,436 | 70,469 | **0.943×** | 48 | 89 |
| 2^18 (256 K) | 73,364 | 85,210 | **0.861×** | 180 | 335 |
| 2^20 (1 M) | 91,325 | 96,575 | **0.946×** | 811 | 1325 |

**WHIR is consistently 5-15% smaller and 1.6-2.6× faster on prover side. No 3-5× saving.** This is the production-honest comparison: same soundness assumptions, fair PCS-vs-PCS measurement.

## Sweep 2 — WHIR @ CapacityBound vs FRI @ JohnsonBound

WHIR uses CapacityBound (γ < 1 − rate, conjectured); FRI keeps JohnsonBound (γ < 1 − √rate, proven). This isolates the saving available IF we accept the conjecture for WHIR.

| log_size | WHIR bytes | FRI bytes | WHIR/FRI ratio |
|---:|---:|---:|---:|
| 2^12 | 22,098 | 48,733 | **0.453×** |
| 2^14 | 25,553 | 57,077 | **0.448×** |
| 2^16 | 33,956 | 70,415 | **0.482×** |
| 2^18 | 37,796 | 85,294 | **0.443×** |
| 2^20 | 47,007 | 96,732 | **0.486×** |

**WHIR @ CapacityBound is ~2.1-2.2× smaller than FRI @ JohnsonBound.** This is closer to (though still below) the paper's "3-5×" headline claim.

### Soundness reality check

| Bound | Proximity radius γ | Bits proven | Source |
|---|---|---|---|
| Unique decoding | `(1 − rate)/2` | `nq · lb / 2` | Reed-Solomon classical |
| **Johnson** | `1 − √rate − η` | **`nq · lb`** | **IACR ePrint 2025/2055 Thm 1.5 (proven)** |
| **Capacity** | `1 − rate − η` | conjectured | folklore + heuristic; no known proof beyond Johnson |

Per the M-S5b S(−1) soundness analysis ([`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md)), we hold to the **proven** Johnson bound. Adopting CapacityBound for WHIR would:
- Move WHIR's soundness story from "proven" to "conjectured".
- Require the audit board to accept a stronger assumption than what we currently document.
- Be inconsistent with our FRI choice (also at Johnson).

**This is a maintainer-level decision**, not a technical optimization.

## Sweep 3 — Folding factor sweep at num_vars = 18

Tested folding factors `k ∈ {3, 4, 5, 6, 8}` at fixed polynomial size 2^18.

### At JohnsonBound

| k | WHIR bytes | open(ms) |
|---:|---:|---:|
| 3 | 86,542 | 242 |
| **4** | **73,210** | 177 |
| 5 | 81,687 | 169 |
| 6 | 84,650 | 116 |
| 8 | 203,156 | 106 |

### At CapacityBound

| k | WHIR bytes | open(ms) |
|---:|---:|---:|
| 3 | 44,532 | 234 |
| **4** | **37,796** | 176 |
| 5 | 40,997 | 167 |
| 6 | 42,794 | 108 |
| 8 | 103,447 | 110 |

**Best `k = 4` at both soundness levels.** Our default is already optimal for proof size.

### Observations

1. **k=3 is slower (more rounds) and bigger** (the small-fold penalty).
2. **k=4 is the sweet spot** for both bytes and a reasonable prover time.
3. **k=5-6 trades ~12-15% more bytes for ~1.5× prover speedup** — could be worth it if prover time matters more than size.
4. **k=8 is a dramatic regression** (~3× more bytes). Over-folding eats both metrics.

The paper's "3-5×" benchmarks may use parameter combinations we haven't tested (different rate, fewer queries due to lower security level, or different polynomial size). Within our production-comparable range, k=4 is empirically optimal.

## Implications for L1 size

Our current production L1 = ~390 KB (commits ~3 polynomials: trace, quotient, possibly random). Projecting from per-PCS savings:

| Scenario | Per-PCS savings | L1 projection | vs ≤65 KB target |
|---|---|---:|---:|
| Current FRI @ Johnson | baseline | **390 KB** | 6.0× over |
| WHIR @ Johnson (k=4) | ~10% | **~350 KB** | 5.4× over |
| WHIR @ CapacityBound (k=4) | ~55% | **~180 KB** | 2.8× over |

**Even the most aggressive WHIR config doesn't reach ≤65 KB.** Path A (SNARK wrap) remains the only known path to the target.

**However, WHIR @ Capacity to ~180 KB IS a meaningful reduction** — closer to the target. The full Path A wrap would then have a smaller cert to wrap, potentially reducing SNARK overhead.

## Caveats + R1 honest residuals

1. **Per-polynomial PCS bytes, not full STARK proof.** A real WHIR-based STARK would commit multiple polynomials (trace, quotient, random) and perform batched openings. The total proof might compound differently than our single-poly projections.

2. **No in-circuit WHIR verifier exists in Plonky3.** Our L1+L2 verifier circuit verifies FRI. Switching to WHIR requires:
   - Writing an in-circuit sumcheck primitive (substantial; WHIR's main novelty).
   - Wiring it through the L2 verifier circuit.
   - Maintaining tamper-reject + soundness gates.
   - This is the bulk of the ~3-week integration estimate (see [`2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md`](2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md) § 4.2 D2).

3. **Prover speedup (1.6-2.6×) IS real.** If block-mining is currently capped by prover time rather than proof bytes, WHIR could be the right move even at JohnsonBound (modest size win + significant time win).

4. **`p3-whir` is multilinear, our STARK is univariate.** The integration adapter is non-trivial (univariate-trace → multilinear-via-Lagrange-basis embedding). Plonky3 doesn't ship this adapter.

5. **CapacityBound conjecture risk:** if a future attack demonstrates the conjecture is false for some parameter regime, all WHIR-Capacity proofs become unsound retroactively. Johnson has been proven safe; Capacity has not.

## Recommendations

### If size is the priority

1. **Acknowledge:** WHIR alone can't reach ≤65 KB at any soundness level.
2. **WHIR @ JohnsonBound (~10% per-PCS saving)** is honest but small. Not worth multi-week integration for the size win alone.
3. **WHIR @ CapacityBound (~55% per-PCS saving)** is a meaningful win but requires accepting a conjectured soundness bound — **maintainer-level decision**.
4. **Path A (SNARK wrap)** remains required for ≤65 KB.

### If prover time is the priority

1. WHIR's **1.6-2.6× prover speedup** at our exact production parameters is a real operational win, independent of byte savings.
2. If we already need WHIR for prover speed, the modest size win is a bonus.

### Combined recommendation

**The "WHIR is a silver bullet for proof size" framing the paper markets does NOT hold at our parameters.** At JohnsonBound (production-faithful), WHIR is a modest improvement. At CapacityBound, the size win is dramatic but the soundness story weakens.

The path to ≤65 KB is **Path A regardless of WHIR**. WHIR might be worth pursuing for prover-time reasons, or as a "warm-up" before Path A (reduce L1 from 390 → 180 KB at Capacity, then wrap in SNARK for the final shrinkage).

## How to reproduce

```bash
# Smoke (runs on default `cargo test`)
cargo test -p ai-pow-zk --release --test test_whir_prototype whir_prototype_compiles_and_small_smoke -- --nocapture

# Sweep 1: WHIR @ JohnsonBound vs FRI @ JohnsonBound (~8s)
cargo test -p ai-pow-zk --release --test test_whir_prototype whir_vs_fri_pcs_byte_comparison -- --ignored --nocapture

# Sweep 2: WHIR @ CapacityBound vs FRI @ JohnsonBound (~8s)
cargo test -p ai-pow-zk --release --test test_whir_prototype whir_capacity_bound_vs_fri -- --ignored --nocapture

# Sweep 3: Folding factor sweep at num_vars=18 (~3s)
cargo test -p ai-pow-zk --release --test test_whir_prototype whir_folding_factor_sweep -- --ignored --nocapture
```

## Cross-references

- WHIR paper: IACR ePrint 2024/1586 (Bisaccia et al.).
- FRI soundness: IACR ePrint 2025/2055 (Ben-Sasson, Carmon, Habock, Kopparty, Saraf) Theorem 1.5 + § 1.3.2.
- Plonky3 WHIR source: `https://github.com/Plonky3/Plonky3/tree/<rev>/whir`.
- Path A vs Path B comparison: [`2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md`](2026-05-20_NPO_RECURSIVE_STARKS_DESIGN_REPORT.md) § 4.2-§ 5.
- M-S5b soundness analysis: [`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md).
- Recursive proof size investigation: [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md).
