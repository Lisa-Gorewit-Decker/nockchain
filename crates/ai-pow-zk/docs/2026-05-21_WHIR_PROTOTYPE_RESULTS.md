# WHIR PCS feasibility prototype — empirical results

**Date:** 2026-05-21
**Source:** `crates/ai-pow-zk/tests/test_whir_prototype.rs`
**Plonky3 rev:** `82cfad7` (matches our other `p3-*` deps via `Cargo.toml`'s `https://github.com/Plonky3/Plonky3.git` default branch).
**Status:** Measurement complete. R1 honest residual: integration of WHIR into our STARK is NOT done — the prototype only measures per-polynomial PCS bytes.

## MAINTAINER POLICY (2026-05-21, FINAL): Paper-grounded Johnson, anchored between insecure & 80-bit conservative — 60-bit floor

**Production FRI + (future) WHIR use the paper-proven Johnson
bound (IACR ePrint 2025/2055 Thm 1.5), with bits target
anchored between the known-insecure CYCLE-SUM ceiling (~22 bits
at γ ≥ LDR for n ≤ 2^22) and the prior conservative 80-bit
floor. Maintainer-targeted ≥60-bit Johnson floor.**

### Trail of the policy evolution (verbatim, for honest history)

1. **2026-05-21 morning:** maintainer typo "Let's move to the
   Johnson Bound" was corrected to "Let's move to the Capacity
   Bound" — briefly accepted; the intervening commit `18613ab`
   (Johnson-only hard rule) was reverted.
2. **2026-05-21 mid-day:** *"Read the paper 2025-2055.pdf to
   check your soundness math."* Paper re-read (pages 1–15)
   revealed:
   - Thm 1.5 proves up to Johnson only.
   - §§ 1.4.1, 1.4.2, 1.4.3 + Thms 1.6, 1.9, 1.13 give
     constructive negative results above Johnson.
   - § 1.4.5 + § 8 + Thm 1.17 (CYCLE-SUM) give explicit STARK
     attacks at γ ≥ LDR with cheating probability Ω(1/n).
   - The Plonky3 `CapacityBound::log_eta` heuristic sits in the
     no-mans-land between Johnson (proven) and LDR (attacked)
     with no paper support against generic codes.
3. **2026-05-21 afternoon:** *"Rather than relying on the Plonky3
   numbers, let's make a configuration that is as optimistic as
   is reasonable based on the paper's numbers and use that."*
   + *"Let's hop up to 60 bits."* + *"An attacker has 2.5
   minutes to make a proof in our context, hence our
   optimism."* → final policy.

### The two paper end-points

| End-point | Formula | Bits at our params (lb=4, n≤2^22) | Status |
|---|---|---:|---|
| Known **insecure** at γ ≥ LDR (Thm 1.17 CYCLE-SUM) | `log₂(n) + O(1)` | ~22 | constructive attack, paper |
| Known **secure** at γ < J(δ)−η (Thm 1.5) | `lb · nq + pow` | 80+ | proven, paper |

### The anchored-between policy

- **Soundness model:** Johnson (paper-proven, IACR 2025/2055
  Thm 1.5). **CapacityBound NOT adopted** — the paper's
  constructive negative results at γ ≥ LDR make the conjectural
  ~2× per-query bits saving audit-undefendable for consensus.
- **Bits target:** anchored *inside* (22, 80), maintainer-targeted
  ≥60 bits Johnson floor.
- **Justification:** 2.5-min block-cadence threat model. PoW
  forgery is time-bounded; offline 80-bit cryptographic margin
  is unnecessary; 60-bit Johnson-proven floor with ~38-bit
  margin over known-insecure CYCLE-SUM ceiling is "reasonable
  and optimistic."

### Production parameters (2026-05-21, FINAL)

- **Outer-cert L1/L2** `crates/plonky3-recursion/circuit-prover/src/config.rs::goldilocks_tip5_60bit()`:
  `lb=4, nq=15, pow=1+1` ⇒ `4·15 + 1 + 1 = 62` bits Johnson, proven.
- **Inner Tip5-L0** `crates/ai-pow-zk/src/circuit.rs::CircuitConfig::PROD`:
  `lb=4, nq=15, pow=1+1` ⇒ `4·15 + 1 + 1 = 62` bits Johnson, proven.
- **Chain MIN** = MIN(62, 62, 62) = **62 bits**, ≥60-bit anchored floor.

### What the Sweep 2 numbers below mean *under this policy*

The WHIR @ CapacityBound numbers in Sweep 2 remain empirically
accurate as measurements **but are NOT the production target**:
they show what would be available *if* the CapacityBound
conjecture were adopted (rejected). The deployed configuration
follows Sweep 1's WHIR @ JohnsonBound trend (5–15% per-PCS-byte
saving over FRI @ Johnson on the same hash), which is the
proven envelope. Production FRI itself moves to `nq=15` (was
`nq=20`) under the anchored 60-bit Johnson floor, capturing
roughly 25% fewer queries vs the prior 80-bit-floor PROD.

See `~/.claude/projects/-Users-loganallen-Dev-nockchain/memory/soundness_capacity_bound.md`
for the full policy entry + cross-references.

## TL;DR (post-2026-05-21 anchored-between Johnson policy)

| Configuration | Per-PCS bytes (vs FRI@Johnson@nq=20 baseline) | Soundness | Status |
|---|---|---|---|
| FRI @ Johnson, `nq=20` (prior 80-bit PROD) | 1.0× | proven (Thm 1.5) | superseded |
| **FRI @ Johnson, `nq=15` (new 60-bit anchored PROD)** | **~0.75×** | **proven (Thm 1.5)** | **production** |
| WHIR @ Johnson | 0.86–0.95× | proven (Thm 1.5) | integration candidate |
| FRI @ Capacity (heuristic) | ~0.5× | conjectured | **REJECTED — paper has attacks at LDR** |
| WHIR @ Capacity | 0.44–0.49× | conjectured | **REJECTED — same reason** |

**Key finding:** the ~2× saving WHIR claims at CapacityBound
runs into the paper's constructive attacks at the list-decoding
radius (§§ 1.4.5, 8, Thm 1.17). The deployable saving on a
paper-grounded Johnson footing is more modest (FRI-internal
`nq=20→15` retune for 25% query reduction; WHIR @ Johnson adds
another 5–15%). The anchored 60-bit Johnson floor unlocks both
without leaving the proven envelope.

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

**2026-05-21 reanchored target ≤100 KB** (was ≤65 KB; see
`2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md` 2026-05-21
addendum for the relaxation rationale). Current production
L1 = 306.91 KB at the anchored 60-bit Johnson floor
(`lb=4 nq=15 pow=1+1`, Stage 5 measurement post-Rayon +
`mds_cyclomul`). Projecting from per-PCS savings:

| Scenario | Per-PCS savings | L1 projection | vs ≤100 KB target |
|---|---|---:|---:|
| Current FRI @ Johnson (60-bit anchored, post-Angle-A) | baseline | **293 KB** | 2.93× over |
| WHIR @ Johnson (k=4) | ~10% | **~275 KB** | 2.75× over |
| WHIR @ CapacityBound (k=4) | ~55% | **~140 KB** | 1.40× over |

**Even the most aggressive WHIR config doesn't reach ≤100 KB on
its own.** Combining WHIR @ Johnson with higher-lb outer (lb=6 nq=10
at the anchored floor) might close part of the gap; Path A
(SNARK wrap) remains the likely final lever to reach the target.

Note: WHIR @ Capacity is **rejected** as a soundness model (see
the MAINTAINER POLICY section at the top of this doc — paper has
constructive attacks at γ ≥ LDR). The ~140 KB projection is
shown as a comparator only; it would not be deployed.

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
   `crates/plonky3-recursion/circuit-prover/src/config.rs::goldilocks_tip5_60bit()`
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
   reaches ≤100 KB).

### ≤100 KB target status (2026-05-21 reanchored)

The in-substrate Johnson floor at the anchored 60-bit FRI
(`lb=4 nq=15 pow=1+1`, post-Angle-A) is L1 = 293 KB, L2 = 329 KB
(pre-Angle-A was 307 KB / 343 KB; the 2026-05-21 Tip5
A-column-elimination refactor dropped Tip5 AIR width 638→558). Combining
the proven-Johnson levers (WHIR @ Johnson ≈ −10%, higher-lb
outer e.g. `lb=6 nq=10 pow=1+1` ≈ −20-25% queries) plausibly
brings L1 into the ~200 KB range — still ~2× over the ≤100 KB
target. Final closure most likely requires Path A.

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
