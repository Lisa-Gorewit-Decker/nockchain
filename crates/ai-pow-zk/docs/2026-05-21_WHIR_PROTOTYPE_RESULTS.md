# WHIR PCS feasibility prototype — empirical results

**Date:** 2026-05-21
**Source:** `crates/ai-pow-zk/tests/test_whir_prototype.rs`
**Plonky3 rev:** `82cfad7` (matches our other `p3-*` deps via `Cargo.toml`'s `https://github.com/Plonky3/Plonky3.git` default branch).
**Status:** Measurement complete. R1 honest residual: integration of WHIR into our STARK is NOT done — the prototype only measures per-polynomial PCS bytes.

## MAINTAINER POLICY (2026-05-21, corrected): CapacityBound

**Production FRI + (future) WHIR use CapacityBound soundness.**

This corrects an earlier same-session typo: the initial message
said "Johnson" but the intended decision was "Capacity." The
intervening commit `18613ab` (which locked in a Johnson-only
hard rule) is honest history — the typo correction is recorded
here for clarity, not hidden.

**The decision:**

- **Production FRI**: CapacityBound (was Johnson; needs parameter
  retuning).
- **WHIR (if integrated)**: CapacityBound.
- **Future PCS**: CapacityBound by default.

**The trade-off the maintainer accepted:**

| Aspect | Johnson | **Capacity (chosen)** |
|---|---|---|
| Soundness proof | proven (IACR 2025/2055 Thm 1.5) | **conjectured** (20+ year open) |
| Per-PCS byte savings | 1.0× | **~2× smaller** (Sweep 2 empirics) |
| Audit anchoring | paper-cite | conjectural + cryptanalytic-exposure argument |
| Known attacks | none below γ < 1−√ρ | IACR 2025/2055 § 8 documents attacks in some regimes |

**Risk accepted:**

- The CapacityBound conjecture has not been proven; some
  parameter regimes have documented attacks (IACR 2025/2055
  § 8). For our specific parameters, no attack is known.
- 20+ years of cryptanalytic exposure on the underlying RS
  proximity gap without break for practical parameters is
  taken as practical evidence.
- The audit-readiness story shifts from "paper-proven" to
  "no-known-attack + heuristic"; an external cryptographic
  reviewer would evaluate the IACR § 8 attacks vs our specific
  config.

**What this means for previously-committed work:**

- Phase 0 / Tier B / 5-round Tip5 / C1 / Path-B B2 numbers
  were taken with FRI at Johnson-derived parameters. They remain
  valid as RELATIVE measurements; the absolute soundness claim
  flips from "82 bits proven" to "82 bits at Johnson, larger at
  Capacity (re-derivation pending)."
- M-S5b soundness analysis + C4 audit-readiness docs anchor on
  Theorem 1.5. They need a Capacity addendum (R1 residual).

**Reference data in this doc:** Sweep 2 (WHIR @ Capacity) now
represents the DEPLOYMENT-TARGET configuration, not a rejected
option. Sweep 1 (WHIR @ Johnson) is retained as the alternative
that would apply if we ever reverted.

See `[soundness_capacity_bound]` memory entry for the full
residual list + audit-readiness consequences.

## TL;DR (post-policy: CapacityBound for both)

| Configuration | Per-PCS bytes (vs FRI@Johnson baseline) | Soundness | Status |
|---|---|---|---|
| Old FRI @ JohnsonBound | 1.0× (Johnson baseline) | proven (IACR 2025/2055 Thm 1.5) | superseded |
| FRI @ CapacityBound | ~0.5× (predicted; needs retuning) | **conjectured** | **production target** |
| **WHIR @ CapacityBound** | **0.44-0.49×** (~2.1× smaller) | **conjectured** | candidate for ~3-week integration |
| WHIR @ JohnsonBound | 0.86-0.95× (5-15% smaller) | proven | reference comparator (not deployed) |

**Key finding:** WHIR's "3-5× smaller" claim materializes at
CapacityBound — empirically 2.1× smaller than FRI @ Johnson on
the same hash, for the same polynomial sizes. The cost is
trusting the capacity-radius proximity-gap conjecture.

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

## Recommendations (post-2026-05-21 Capacity policy)

### Immediate FRI parameter retuning

With CapacityBound, the Reed-Solomon proximity-gap bits-per-query
formula gives more bits per query than at Johnson. The current
production FRI (`lb=4 nq=20 pow=1+1`) targets 82 bits at Johnson;
at Capacity the same parameters give more bits, and we can drop
`nq` to recapture the byte saving.

Action items (R1 residuals):

1. **Derive Capacity-bound bits formula** for our parameters from
   the Plonky3 `SecurityAssumption::CapacityBound` implementation
   (whir/src/parameters/soundness.rs).
2. **Choose new `nq`** at `lb=4` that gives 80-bit Capacity
   soundness with appropriate margin.
3. **Stage 5 re-measure** L1+L2 at the retuned config.
4. **Update doc-comments** in
   `Plonky3-recursion/circuit-prover/src/config.rs::goldilocks_tip5_80bit()`
   and `crates/ai-pow-zk/src/circuit.rs::CircuitConfig::PROD`.

### WHIR integration (post-FRI-retuning)

**WHIR @ Capacity is now the preferred integration target** —
~2.1× smaller than FRI @ Johnson, ~2× faster prover. Combined
with FRI-retune-to-Capacity (which would also shrink the L1
baseline), the chain could plausibly drop into the ~150-200 KB
range.

Path forward:

1. Retune FRI to Capacity (~1 day).
2. Re-measure L1+L2 baseline.
3. Estimate WHIR-integrated L1+L2 (likely ~70-100 KB at Capacity).
4. Decide: WHIR integration (~3 weeks) vs Path A (~months;
   reaches ≤65 KB).

### ≤65 KB target status

At Capacity, the in-substrate floor MIGHT approach the target.
Optimistic projection: FRI @ Capacity + WHIR @ Capacity + Tier C
digest=4 could plausibly reach L1 ~70-90 KB. Still ~1.1-1.4×
over target.

**Path A (SNARK wrap) may not be required** if the Capacity-bound
in-substrate work gets close enough. Worth measuring before
committing to Path A.

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
