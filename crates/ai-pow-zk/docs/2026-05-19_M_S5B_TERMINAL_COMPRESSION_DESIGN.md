> _Created **2026-05-19** · last updated **2026-05-19**._

# M-S5b / P-C2 — design + de-risk plan: ≤65 KB terminal compression of the ≥120-bit M-S5 cert

> **Status (R1, honest).** DESIGN + STAGED KAT-FIRST DE-RISK PLAN.
> **No invasive code landed by this document.** This is the
> *first stage* of M-S5b (`#131`), authored under R1 discipline:
> design → staged plan → KAT-first de-risk → per-stage exhaustive
> tests → commit per validated stage → honest residual.
>
> **Why a separate milestone.** M-S5 (C3 / `#124`) was re-scoped
> on 2026-05-19 (`259cab2`) to the soundness-correct **≥120-bit**
> vertical-recursion certificate (LANDED, every chain link
> ≥120-bit). The size target ≤65 KB was carved out into **M-S5b
> (P-C2)** because §14 of `2026-05-19_C3_OUTER_CERT_DESIGN.md`
> proved that *with the current `Plonky3-recursion` substrate*
> ≤65 KB is reachable **only at the ~5-bit testing tier** — at
> any honest ≥120-bit tier the cert is **27–42× over budget**
> (L1 ≈ 2.69 MB, L2 ≈ 1.79 MB, L3 ≈ 1.73 MB). Closing the gap
> requires a **substrate addition**, not a parameter tweak. This
> doc inventories the candidate paths, evaluates each against
> the M-S5b soundness + audit-surface bar, and lays the staged
> plan.
>
> **Authoritative cross-refs:** `2026-05-19_C3_OUTER_CERT_DESIGN.md`
> §14 (byte-breakdown measurement), §15 (≥120-bit cert
> LANDED); `2026-05-17_PRODUCTION_ROADMAP.md` Phase C row
> M-S5b; `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` §2c
> (Tip5 AIR — the soundness linchpin compression must inherit
> intact); `2026-05-17_M_S2_PEARL_EVALUATION.md` (origin of the
> ≤65 KB target — Pearl §4.7/§5.1).

---

## 0. Goal & non-goals

### Goal

Produce a verifying **terminal compression** of the LANDED
≥120-bit M-S5 outer-recursive STARK such that:

1. **Size:** the consensus-facing artifact is **≤ 65 536 bytes**
   (the byte-verbatim `assert!(serialized_len <= 65_536, …)` in
   `Plonky3-recursion/recursion/tests/test_tip5_layer0_recursion.rs::tip5_layer0_outer_cert_size_residual`
   passes — that test currently `#[ignore]`s with a reason
   string pointing at this milestone).
2. **Soundness:** ≥ **80 unconditional** bits **end to end**,
   anchored on the **Johnson-radius proximity-gap bound now
   proven** by Ben-Sasson, Carmon, Habock, Kopparty, Saraf,
   *"On Proximity Gaps for Reed–Solomon Codes"*
   (IACR ePrint 2025/2055, Nov 2025) — specifically Theorem 1.5 +
   §1.3.2's "linear dependence on *n*" unlocking proven
   security at Johnson distances. The 2026-05-19 maintainer
   decision recalibrates the M-S5b soundness floor from the
   *legacy* ≥120-conjectured framing to **≥80 unconditional**;
   see §1.4 below for the full reasoning + per-path
   implications. No layer in the compression chain weakens any
   inner link below this floor; the LANDED C3 chain
   (`lb=2, nq=120`) is **comfortably above 80 unconditional**
   under this paper's bound, so the legacy C3 parameters need
   no change.
3. **Pearl-faithfulness on the *mineable unit* is untouched.**
   The byte-equivalent anchor is the plain `TileState` /
   `keyed_hash`; M-S5b changes how the SNARK is *compressed*,
   not what it proves about the mined work. Inner Tip5
   Layer-0 circuit, C3 outer-cert AIR, and DT-4 duplex binding
   (`Plonky3-recursion/circuit/src/ops/tip5_perm/executor.rs:14116b0`)
   must remain **byte-identical**.
4. **Recursion-compatible production config.** The packed-MMCS
   `GoldilocksConfig` prerequisite (§14: the landed unpacked
   `MerkleTreeMmcs` is type-incompatible with
   `verify_p3_batch_proof_circuit` on aarch64-neon) is closed by
   this milestone or as an explicit prior dependency.

### Non-goals (out of scope for M-S5b)

- The external **independent crypto audit** of the substrate
  addition itself (that is **C4 / M-S6 / `#125`**; see
  `2026-05-19_C4_AUDIT_READINESS.md`).
- ai-pow-zk's **M10.1c composite `RecursiveAir`** as the inner
  circuit (R-b, remains **M12 / `#127`**, separate milestone).
- Changes to the Tip5 spec / AIR / round constants. The
  compression must **verify** the C2-validated Tip5 Layer-0
  proof, not redesign it.
- Pearl's FP8 PoUW (deferred; out of scope across the whole
  Phase-C roadmap).

### Hard invariants (R1 — non-negotiable)

- **No edit to the C2.1 / L4 / L5 / C2.4-R-a / DT-4 fenced
  linchpin** without full re-validation. The §15 LANDED set
  proves this is `git diff b8b5d32` empty across
  `air_circuit` / `air_lookup` / `generation_lookup` /
  `tip5_spec` / `circuit.rs` / `mmcs.rs` / `executor.rs` /
  `recompose*` / `recursion/src/verifier/*` / `backend/fri.rs`
  / `config.rs`.
- **No soundness trade for size.** §14 explicitly demonstrated
  that "≤65 KB at ~5-bit" is a soundness reduction the
  maintainer rejected. M-S5b cannot land any artifact that
  trades soundness for the size target; that is the
  fake-completion failure mode this milestone exists to avoid.
  The 2026-05-19 ≥80-unconditional re-calibration is **not** a
  trade — it is a paper-grounded re-anchoring (§1.4) that
  *raises* the floor from "conjectured" to "proven" while
  permitting smaller-FRI parameters at the same bar.
- **Stay at or inside the Johnson radius.** §8 of IACR ePrint 2025/2055
  ("Attacks on STARKs near the list decoding radius") + §1.4
  negative results show that pushing into the list-decoding
  regime *beyond* the Johnson radius is genuinely unsafe — the
  improved proximity-gap results stop at Johnson. M-S5b
  parameters must keep the proximity radius `γ < J(δ)−η` for
  the paper's Theorem 1.5 to apply.
- **Validated subset + precise residual** is the R1 fallback if
  the maintainer-decided path turns out to require multi-session
  invasive work. The first validated subset commits should be
  the KAT-first de-risk artifacts (S0–S2 below), not the
  invasive substrate edit.

---

## 1. Why M-S5b is hard — the §14 measurement, recapped

§14 of the C3 design doc empirically established the following
on a real ≥120-bit `BatchStarkProof` of the C3 outer cert
(via `prove_all_tables` + `postcard`; full byte-faithful, not
estimated):

### 1.1 The honest sizes (≥120-bit chain)

| Layer | Real size | vs 65 KB | Notes |
|---|--:|--:|---|
| Inner Tip5-L0 sweep | ≈ **117 KB** | 1.8× over | the LANDED C3 inner |
| **L1** ≥120-bit (`OuterTier::Bit120`) | ≈ **2.69 MB** | **42×** | wraps inner |
| **L2** ≥120-bit over ≥120-bit L1 | ≈ **1.79 MB** | **27×** | smaller than L1 |
| **L3** ≥120-bit over L2 | ≈ **1.73 MB** | **27×** | smaller than L2 by only 60 KB |
| L3 vs L2 delta | ≈ −60 KB | — | recursion **converges very slowly** |
| Net per-layer reduction at L≥2 | ~3.4 % | — | far below what's needed |

**Extrapolated lower bound:** even if the reduction continued
linearly (it won't — it asymptotes), reaching ≤65 KB would
require >50 layers, each adding ~1.8 MB of intermediate work
and one full 120-query FRI commitment. The recursion is
**bounded below** at well over 65 KB at every measured depth
and config.

### 1.2 The byte breakdown — what dominates

S0.b of §14 measured the L1 PROD breakdown (119 866 B total at
the original ~5-bit wrapper tier, before re-scoping to 120-bit;
post-re-scope ratios shift but the dominant terms hold):

| Term | Bytes | % | What it is |
|---|--:|--:|---|
| `opened_values` (OOD poly evals) | **65 466** | **54.6 %** | tables × cols evaluated at the OOD point |
| `opening_proof` (FRI) | 44 592 | 37.2 % | FRI fold-chain commitments + queries |
| `global_lookup_data` | 8 464 | 7.1 % | LogUp argument |
| commitments | 912 | 0.8 % | Merkle roots |
| rest | < 360 | < 0.3 % | header/PIs/etc |

**The dominant cost is `opened_values`** (the OOD opens of every
column of every table at one extension-field point). It scales
with **table count × column count × D** and is **independent of
FRI query count.** Tightening FRI alone cannot collapse this.

### 1.3 Substrate gaps §14 left open

Two substrate items §14 surfaced as **prerequisites** for *any*
production M-S5b landing, independent of the path choice:

- **Packed-MMCS `GoldilocksConfig`.** Landed
  `p3_circuit_prover::config::GoldilocksConfig` uses
  `MerkleTreeMmcs<Goldilocks, …>` (unpacked), but
  `verify_p3_batch_proof_circuit` requires
  `MerkleTreeMmcs<F::Packing, …>`. On aarch64-neon
  `Goldilocks::Packing ≠ Goldilocks` ⇒ the landed-config cert
  is **type-incompatible** with the recursion verifier. §14's
  measurements used a packed-MMCS, FRI-tier-identical, cap-0
  substitute (verified byte-faithful: total == landed
  119 866 B). M-S5b must either reuse this substitute or close
  the type incompatibility upstream.
- **The L2 verifier-circuit's ~40 KB fixed floor.** §14 S1
  established that the in-circuit Poseidon2-W8 + recompose +
  FRI fold-chain in the verifier circuit alone produce
  ≈ 40 KB of inevitable cost *independent of inner size*. Any
  in-substrate compression strategy stays bounded below by
  this floor unless it edits the verifier circuit itself
  (Path B below). **At the new 80-unconditional bar (§1.4)
  this 40 KB floor remains** — it is dominated by `opened_values`
  (column count × table count × D), which is FRI-query-count
  independent.

---

## 1.4 Soundness bar — 2026-05-19 maintainer decision: **≥80 bits unconditional, Johnson-radius proven**

### 1.4.A The decision

The earlier M-S5 chain (LANDED) was sized at **≥120 conjectured
bits per layer** — a bar that required either accepting
proximity-gap conjectures into the list-decoding regime
(`[BCI⁺20]`-style) or paying the heavy O(n²) loss to stay
unconditional inside Johnson radius. On 2026-05-19 the
maintainer re-calibrated M-S5b (and the full Phase-C target by
extension) to **≥80 bits unconditional, anchored on the
Johnson-radius bound now *proven* by Ben-Sasson, Carmon,
Habock, Kopparty, Saraf, "On Proximity Gaps for Reed–Solomon
Codes" (IACR ePrint 2025/2055, Nov 2025)**.

The argument:

- **Block cadence.** Nockchain blocks finalize every ≈ 2.5 min
  (150 s). A successful proof forgery must succeed *within*
  that window — a stale forgery is worthless once the next
  block locks. So the security bound is `(forge success
  probability) × (work budget in 150 s)`. At 80 unconditional
  bits this requires ≈ 2⁸⁰ / 150 s ≈ 7 × 10²¹ hashes/sec —
  far past any feasible adversary today, and not on the curve
  to be feasible at any reasonable threat horizon.
- **The 120/128-bit margin defends against multi-block /
  long-horizon attacks** (multi-year offline work, retroactive
  rewrite). Per-block PoW that resets every 150 s does not
  need that margin.
- **"Without relying on conjectures."** The maintainer
  explicitly disallowed conjecture-grounded soundness for
  M-S5b. Previously this would have forced the unique-decoding
  bound (heavy penalty); now IACR ePrint 2025/2055 Theorem 1.5 +
  §1.3.2 make the **Johnson radius bound proven**, with
  linear-in-n loss instead of the prior O(n²). The paper's
  own framing (§1.3.2, verbatim): "The new result unlocks
  proven security with distances near the Johnson bound in
  such systems, possibly leading to significant improvements
  in performance."

### 1.4.B Parameter mapping (rough)

The exact FRI-parameter → unconditional-bits map under the new
bound depends on the specific FRI variant in
`Plonky3-recursion/recursion/src/backend/fri.rs` and the
Plonky3 soundness-reduction code path. Pending the audit's
exact analysis, the ballpark mapping is:

| Bar | Rough parameter target | Notes |
|---|---|---|
| Current C3 chain (LANDED) | `lb=2, nq=120` | ≥120 conjectured bits (legacy framing) ⇒ **≥120 bits proven** under Johnson radius via IACR ePrint 2025/2055 Theorem 1.5 (well above the new floor) |
| New M-S5b floor | `lb·nq ≈ 80` (e.g., `lb=2, nq=40` or `lb=3, nq=27`) | ≥80 bits proven under Johnson radius |
| Old "≥120-conjectured M-S5b" sizing (now superseded) | `lb·nq ≈ 240` (i.e., `lb=2, nq=120`) | superseded — was over-engineered for per-block cadence |

The 80-unconditional bar **halves to two-thirds the FRI
query count** versus the legacy 120-conjectured framing, which
materially shrinks the `opening_proof` byte cost (FRI proofs
scale roughly linearly with `nq`).

### 1.4.C Implications for the path tree

The recalibration shifts the path-feasibility analysis in §2.
**Empirical confirmation (2026-05-19, after Stage 1/2 landed —
see `2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md` §2):**

- **C3 (LANDED) was re-parametrized** (not "no change" as
  initially predicted). The maintainer's directive *"Move all
  plonky3 parameter choices lower to meet the 80 bit floor"*
  was applied (`0334943` + `f54ae81`): PROD now `(lb=3, nq=30)`,
  LB-sweep nq's halved-to-thirded across the board, outer-cert
  config `goldilocks_tip5_120bit` now `(lb=2, nq=42, pow=1+1)`.
  Soundness chain at the inner Tip5-L0 cert + L1 + L2 = MIN(90,
  85, 85) ≥ 80 unconditional Johnson per IACR ePrint 2025/2055
  Theorem 1.5 (validated `c3_stage_a` + `c3_stage_b` PASS).
- **L1/L2 sizes shrank ~3×** (real measurement, not projection):
  L1 from 2.69 MB to 961 KB; L2 from 1.79 MB to 618 KB.
- **Path B's reach to ≤65 KB tightened** to a ~9.5× target
  (was ~27×).
- **Stacked recursion confirmed dead** (S3(ii) measurement):
  L3 > L2 at the new bar by ~32 KB. Adding more layers does
  not close the gap; the verifier-circuit ~40 KB fixed floor
  dominates once inner size drops to ~500 KB.
- **Path A may still become optional** *if* Path B's S1 work
  attacks the verifier-circuit floor (Poseidon2-W8 +
  recompose-table + in-circuit FRI fold-chain), not just FRI
  proof bytes.
- **Path C / Path D** unchanged in necessity; same caveat about
  the floor applies to D.

### 1.4.D What the audit (C4) must confirm

The new bar is paper-grounded but requires concrete audit
work to bind the abstract Johnson-radius bound to *our exact
FRI parameter configuration*:

1. ✅ **CLOSED 2026-05-20** (S(−1) landed). Confirm the paper's
   Theorem 1.5 hypotheses are satisfied by our FRI parameters
   (in particular: code distance δ, rate, and the proximity
   radius γ < J(δ)−η we target). See
   `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` §1, §4.
2. **DEFERRED to C4 audit** (`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`
   §6 residual #1). Derive the explicit `(lb, nq, pow_bits)
   → unconditional bits` formula for our exact Plonky3 FRI
   variant under Theorem 1.5. *S(−1) used the community-agreed
   `lb · nq + pow_bits` formula — C4 auditor should walk the
   `p3-fri` internal reduction in detail to confirm the
   constant.*
3. ✅ **CLOSED 2026-05-20** (S(−1) landed). Verify the protocol
   does **not** push proximity testing beyond Johnson radius at
   any point in the M-S5/M-S5b chain (§8 of the paper shows
   beyond-Johnson is genuinely unsafe). See
   `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` §4.3 (per-layer
   J(δ)−η table) + §1.3 (paper §8 attacks summary).
4. ✅ Add the paper to the C4 audit-readiness reference set
   (`2026-05-19_C4_AUDIT_READINESS.md` § 1.3 + § 11).

Items 1–3 were part of the M-S5b S(−1) prerequisite (added to
§3 below); item 4 is done by the original 2026-05-19 commit.
**As of 2026-05-20, items 1+3+4 are closed; item 2 is the only
remaining C4-audit-deferred item from this list.**

---

## 2. Candidate paths (the substrate-addition design space)

Four orthogonal paths exist. Each is evaluated on **five axes**:

- **Soundness preservation** — does the compression inherit
  C3's ≥120-bit floor intact? Any auxiliary primitive (curve,
  hash, polynomial commitment) must be ≥120 conjectured bits at
  the parameter choice and have an explicit soundness argument.
- **Audit surface** — count of *new* primitives an external
  auditor (C4 / `#125`) must independently certify.
- **Dependency footprint** — substrate additions to the
  vendored `Plonky3-recursion` (which is on `c2c51fb`, rev-aligned
  to ai-pow-zk's `6de5cba` — the C1 fixed point).
- **Operational** — prover wall time, peak RSS, verifier
  wall time (consensus-facing).
- **R1 risk** — invasive to the C2.1/DT-4 fenced linchpin? Can
  it be KAT-first de-risked in isolation?

### 2.A Path A — STARK-to-SNARK wrap (BN254/BLS12 outer SNARK)

**Idea.** Outermost layer is a pairing-based SNARK (Groth16 or
Plonk over BN254 / BLS12-381) whose statement is *"the
≥120-bit M-S5 outer-recursive STARK verifies."* The in-SNARK
circuit is a Goldilocks STARK verifier.

**Existence proofs.** This is the technique production zkVMs
(RISC Zero, SP1, Powdr) use to reach <1 KB consensus proofs.
External crates (`arkworks`, `bellperson`, `halo2`) implement
the SNARK side; the in-SNARK Goldilocks-STARK verifier is the
non-trivial component (~10–50 M-gate circuit at the C3 outer
size).

**Final size.** Groth16 = **~192–256 bytes constant** (3 G1 +
2 G2 elements). Plonk = a few KB. **Trivially ≤65 KB.**

| Axis | Assessment |
|---|---|
| Soundness preservation | OK if the outer pairing curve is ≥120-bit and the in-SNARK STARK verifier is sound. BN254 ≈ 128 bits (boundary; some literature says weaker); BLS12-381 ≈ 128 bits, more comfortable margin. **Must choose the curve so the SNARK soundness floor ≥ 120 bits.** |
| Audit surface | **Largest** of the four paths. Adds: pairing-based PCS, the elliptic-curve arithmetic, the trusted-setup ceremony (Groth16) **or** the universal updateable setup (Plonk), the in-SNARK STARK verifier circuit. |
| Dependency footprint | **Largest.** New curve/pairing crate (`arkworks-bn254` or `blst`), new SNARK prover crate, the in-SNARK verifier circuit (could be written in `arkworks` constraint system or a Plonk-ish frontend). |
| Operational | SNARK proving over a ~10 M-gate circuit is **expensive** (minutes wall-time, GBs peak RSS) — but it's the outermost layer so amortized over many tiles. Verifier ≪ 1 ms. |
| R1 risk | **Non-invasive to the fenced linchpin** (the in-SNARK circuit *consumes* the C3 STARK as a public statement; it doesn't edit any AIR). KAT-first de-risk is straightforward: prove a toy Goldilocks STARK with SNARK wrap before any production wiring. |

### 2.B Path B — smaller proven AIR (verifier-circuit refactor)

**Idea.** Rebuild the L2 verifier AIR with **narrower columns
+ lower-degree constraints**, attacking the `opened_values`
54.6 % dominant cost directly. Stay within the Plonky3 substrate;
no new crypto primitive.

**Why it might not be enough.** Even halving the verifier AIR's
column count only halves `opened_values` (~32 KB → ~16 KB at
the same FRI tier) and is bounded below by the ~40 KB fixed
floor (§14 S1). Net floor would still be **well above 65 KB**
at ≥120-bit. Path B alone is almost certainly **insufficient**
to reach the target, but it is the **most R1-friendly** path
and **composes with Path A or D** to reduce their cost.

| Axis | Assessment |
|---|---|
| Soundness preservation | **Highest confidence** — same substrate, same FRI/LogUp/MMCS soundness story, smaller witness. |
| Audit surface | **Smallest.** No new primitive. Audit cost ≈ delta against the existing C2-audited verifier AIR. |
| Dependency footprint | None outside Plonky3-recursion. |
| Operational | Lowest prover cost. |
| R1 risk | **Touches the verifier-circuit AIR**, which is C2/DT-4 adjacent. Must be staged + KAT-first vs the existing verifier behavior. |

### 2.C Path C — genuine proof-folding (Halo/Nova-style accumulation)

**Idea.** Instead of re-proving a full verifier at every layer
(Plonky3-recursion's current model), maintain an *accumulator*
that folds two instances into one in constant work per fold; a
single final "decider" SNARK closes the chain.

**Existence proofs.** Nova / SuperNova / HyperNova / Sangria
(over `pasta` curves, R1CS-based) achieve this in production.
Sonobe is a unifying Rust framework.

**Final size.** ≤ 65 KB easily (the accumulator + a small
decider proof are typically 1–10 KB).

| Axis | Assessment |
|---|---|
| Soundness preservation | OK if (a) the underlying curve is ≥120-bit, (b) the accumulation scheme has a soundness proof (Nova/HyperNova do), and (c) the inner STARK is faithfully embedded as an R1CS / CCS instance — the embedding step is **the audit-heavy crux** (it is *not* a vendored existing implementation; we'd be encoding a Goldilocks-STARK verifier into R1CS over a different curve). |
| Audit surface | **Very large.** New curve, new accumulation scheme, new R1CS embedding of the STARK verifier. Comparable to Path A. |
| Dependency footprint | **Very large.** Most likely vendor `Sonobe` / Nova-IVC + curve crate. None of these are in `Plonky3-recursion` today. |
| Operational | Per-fold work is constant ≪ a full verifier-recursion layer. Final decider is one SNARK proof. |
| R1 risk | **High.** No existing in-tree primitive; the R1CS embedding is novel work. Not currently KAT-able against any existing baseline. |

### 2.D Path D — Plonky2-style "narrow" recursive STARK (Pearl §5.1 mirror)

**Idea.** Pearl's spec (Pearl Whitepaper §4.7/§5.1) reaches
their own small consensus proof using Plonky2's recursion —
which is a STARK over a smaller field (Goldilocks like ours) but
with a *carefully tuned* verifier-circuit (compact gate set,
custom recursion-friendly hashes). Our `Plonky3-recursion` is
Plonky3-based; Plonky2's smaller-proof advantage at parity
soundness comes from its frontend specialization.

**Practically, Path D = Path B + porting Plonky2's
recursion-friendliness conventions** (e.g., compact verifier
gates, single-round Poseidon2 for in-circuit hashing, batched
opens) into the Plonky3-recursion verifier. It is *not* a
distinct primitive; it is a specific instantiation of Path B
with explicit Pearl-fidelity guidance.

| Axis | Assessment |
|---|---|
| Soundness preservation | Same as Path B. |
| Audit surface | Same as Path B (no new primitive) but the audit must explicitly cover the Pearl-fidelity convention adoptions. |
| Dependency footprint | None outside Plonky3-recursion. |
| Operational | Likely lower than naive Path B because Pearl's conventions are battle-tested in their layer-2. |
| R1 risk | Same as Path B. |

### 2.E Comparison summary

Reach-assessment **updated for the §1.4 ≥80-unconditional bar**.
Comparison is at the *new* target, not the legacy
≥120-conjectured bar.

| Path | Reaches ≤65 KB at ≥80 unconditional? | New audit surface | Substrate cost | R1 risk | Composable? |
|---|:--:|---|---|---|---|
| **A** STARK-to-SNARK wrap | **Yes (~256 B)** — always reaches | Large (pairing crypto + setup) | Large (new SNARK crate + curve) | **Low** (outermost, non-invasive) | with B/D |
| **B** smaller verifier AIR | **Plausibly yes alone** at the new bar (Path-B AIR narrowing + ~50% FRI savings from `nq` halving — see §1.4.C); **measurement-gated** | **Smallest** | None | Medium (touches verifier AIR) | with A/C/D |
| **C** Halo/Nova folding | Yes (~few KB) | Very large (new accumulation scheme + R1CS embed) | Very large (Sonobe/Nova vendoring) | **High** (no in-tree analog) | with B |
| **D** Plonky2-style narrow STARK | Plausibly yes alone (= Path B + Pearl-fidelity conventions) | Same as B (+conventions) | None | Medium (touches verifier AIR) | with A/B |

### 2.F Recommendation (for maintainer decision) — re-anchored to §1.4

> **SUPERSEDED 2026-05-20** by the comprehensive S1 routes audit
> (`2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md` § 9). The
> updated recommendation is **Path H = Path B + Path A** (B as
> primary inner optimization, A as primary outer compression,
> composed). The "Path B alone has a real shot at ≤65 KB" claim
> below is **refined** by the L2 structural-floor analysis
> (§ 1.3 of the routes audit): Path B alone bottoms out at
> ~80–175 KB (above the 65 KB target); an outer compression is
> required. **Path D2 (direct Plonky2 vendoring per Pearl's
> approach) is the explicit fallback** if Path A's pairing-crypto
> audit surface is rejected. See the routes audit for the full
> 8-path analysis + per-path quantitative comparison.

**Recommended sequence (updated 2026-05-19, superseded 2026-05-20):
Path B first *as the candidate solution*, not merely de-risk; Path
A held in reserve as the fallback if Path B's measurements miss
≤65 KB.**

Reasoning, re-evaluated at the ≥80-unconditional bar:

- **Path B alone has a real shot at ≤65 KB** at the new bar.
  The §1.4.C analysis: FRI proof bytes shrink ~50 % from `nq`
  halving (`nq=120 → nq≈40`), and a Path-B verifier-AIR
  narrowing can plausibly drop `opened_values` (the dominant
  54.6 % term) by 2–4× via column-count cut + degree-drop +
  table-merge. Combined, the L2 size estimate moves from
  ≈ 1.79 MB to a **predicted ~150–400 KB range** — still over
  65 KB without further work, but the gap to Path A becomes
  thin enough that one additional Path-B sub-stage (e.g.,
  proven AIR-table merging, or DEEP-FRI savings if available)
  might close it.
- **Audit-surface preference inverts.** Previously the
  recommendation favored Path A (guaranteed size) at the cost
  of vendoring pairing crypto + a SNARK frontend. At the new
  bar, Path B's chance of standing alone means the audit can
  stay **entirely inside the Plonky3 substrate** — the C2 +
  C3-audited code path — with no new primitive. This is the
  most audit-friendly + R1-friendly outcome.
- **Path A as the explicit fallback.** If S1 measurements show
  Path B's L2 bottoms out above 65 KB even after maximal
  reduction, switch to Path B + Path A together. Path A
  remains the guaranteed terminal compression; Path B's prior
  AIR shrinking still pays off (smaller in-SNARK verifier
  circuit ⇒ Path-A prover cost drops 2–4×).
- **Path C** is now even less attractive — its main edge was
  the small final proof, which Path B alone may now provide.
  Defer unless A and B both fail.
- **Path D conventions adopted in B.** Pearl-fidelity guidance
  (compact gate set, single-round Poseidon2 for in-circuit
  hashing, batched opens) is folded into the Path-B refactor.

This is a **recommendation**, not a decision. The R1 decision
on which paths to pursue is the maintainer's; the §3 staged plan
below is structured so the maintainer's choice between
**B-alone / B+A / B+D+A / B+C** is made *after* the
S(−1)–S1 measurements return, **not before**.

---

## 3. Staged plan (R1 — commit per validated stage; KAT-first; no rushing)

### 3.0 Stage gate philosophy

Each stage produces a **committed, validated artifact**. No
stage proceeds until the previous stage's gate is green. The
stages are designed so that **S0–S2 commit zero invasive
production code** — they are pure measurement / KAT artifacts
that survive any path decision and inform the path-A/B/C/D
choice empirically.

| Stage | What it commits | Invasive to linchpin? | Cumulative substrate addition? |
|---|---|---|---|
| **S(−1)** ✅ LANDED 2026-05-20 (`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`) | Paper-grounded `(lb, nq, pow_bits) → unconditional bits` mapping for our Plonky3-recursion FRI variant under IACR ePrint 2025/2055 Theorem 1.5; verify γ < J(δ)−η at every M-S5 link | No (analysis only) | No |
| **S0** | Path-A KAT-first prototype (toy Goldilocks STARK SNARK-wrap) in an excluded workspace — **demoted from primary path to fallback prototype after §1.4.C reframing** | No | No |
| **S1** | L2 verifier AIR column-count audit + Path-B reduction map at the new ≥80-unconditional bar; **L2 size estimate at the new FRI parameters** | No (read-only) | No |
| **S2** | Path-C / Sonobe KAT-first prototype, IF maintainer chooses to evaluate after S1 | No | No |
| **S3** | Maintainer decision (**B-alone** vs B+A vs B+D+A vs B+C) | n/a | n/a |
| **S4+** | Invasive substrate addition per the decision | Yes (staged) | Yes |

### 3.0.A S(−1) — Paper-grounded soundness analysis (the new prerequisite) — ✅ **LANDED 2026-05-20**

> **Status update (2026-05-20).** S(−1) landed at
> `crates/ai-pow-zk/docs/2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`.
> Verdict (verbatim from §5.1 of that doc): *"Every layer of the
> LANDED M-S5 chain delivers ≥80 unconditional bits of
> soundness at the Johnson radius under IACR ePrint 2025/2055
> Theorem 1.5. Inner sweep (PROD + 4 LB profiles): 90–92 bits
> per-query, ~82 bits combined. Outer cert (L1, L2): 85–86 bits
> per-query, ~82 bits combined. Chain minimum (any combination):
> ≥ 82 unconditional. Every layer operates at γ\_FRI < J(δ) − η
> with η > 0; the §8 attacks of the paper (beyond-Johnson) are
> structurally avoided."* No parameter change required for the
> LANDED chain. S1 (next deliverable below) is now unblocked.

**Goal.** Concretize the §1.4 abstract bar (≥80 unconditional
bits under IACR ePrint 2025/2055 Theorem 1.5) into the **exact
parameter map** for our `Plonky3-recursion/recursion/src/backend/fri.rs`
FRI variant. This is analytical / written-out work; no code
edit.

**Scope.**
- Confirm our FRI variant matches the Reed–Solomon proximity-
  testing model the paper analyzes (or note any delta and the
  consequence).
- Derive the explicit `(log_blowup, num_queries, pow_bits)
  → bits_unconditional_at_johnson_radius` formula for our
  variant under Theorem 1.5.
- Verify that **every layer** of the M-S5 + M-S5b chain runs at
  γ < J(δ)−η — i.e. proximity testing stays inside the
  Johnson radius (the paper's §8 attacks confirm beyond-Johnson
  is unsafe).
- Produce a per-layer soundness table:
  `(layer, lb, nq, pow_bits, γ, J(δ)−η check, derived
  unconditional bits)`.

**Exit gate.** A committed soundness-analysis note
(`crates/ai-pow-zk/docs/2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`
— ✅ landed) with the formula, the per-layer table, and an
explicit "comfortably above 80 unconditional" verdict for every
link in the LANDED M-S5 chain (no parameter change required
there). **No code change. Required input to S1's parameter
choice — now available for S1.**

**Estimated effort.** 1–2 working days; depends on whether the
audit (C4 / §1.4.D) confirms the mapping or surfaces
adjustments. Actual: ~1 day (2026-05-20).

### 3.1 S0 — Path-A KAT-first prototype (outermost SNARK on a toy STARK; demoted to fallback prototype)

**Note (2026-05-19 reframing per §1.4).** S0 is **no longer the
primary path candidate**; under the ≥80-unconditional bar Path
B has a real shot at ≤65 KB alone (§1.4.C). S0 is retained as
the **fallback prototype** so that if S1 measurements show
Path B doesn't reach ≤65 KB at the new bar, the path-A
SNARK-wrap pipeline is already de-risked. Execute S0 only if
S1 results require it, or in parallel with S1 if the maintainer
wants concurrent de-risk.

**Goal.** Establish a working SNARK-of-STARK pipeline on the
**smallest possible** Goldilocks STARK (a Fibonacci AIR or
similar), so the path-A end-to-end soundness story is exercised
end-to-end in isolation **before** any production wiring.

**Scope.**
- New excluded workspace `Plonky3-recursion-snark-wrap/` (NOT
  in the ai-pow-zk Cargo workspace; analogous to
  `Plonky3-recursion`'s C1 decoupling). Justification: the
  pairing-crypto deps would bleed into ai-pow-zk's audit
  surface if landed in-tree before the maintainer A/B/C
  decision; an excluded workspace lets us measure without
  contamination.
- Choose curve + SNARK frontend: **two prototypes** to compare:
  - (S0.a) **BLS12-381 + Groth16** (smallest proof; requires
    trusted setup per circuit).
  - (S0.b) **BLS12-381 + Plonk (or Halo2-Plonk)** (universal
    updateable setup; ~few KB proof).
- In-SNARK verifier circuit: implement the minimal Goldilocks
  STARK verifier (FRI-fold + opens-check + LogUp) over the
  pairing curve's scalar field.
- KATs:
  1. Toy STARK accepts ⇒ SNARK accepts.
  2. Toy STARK tampered (one row flipped) ⇒ SNARK **rejects**.
  3. Final SNARK byte-size measured (expected: ~256 B Groth16;
     few KB Plonk).
  4. Prover wall + RSS measured on the toy size.

**Exit gate.** Both S0.a and S0.b accept the valid toy STARK
and reject the tampered one; final size is reported to the
maintainer with the prover-cost numbers. **No production
wiring; commit lives in the excluded workspace.**

**Estimated effort.** 3–5 working days of focused work; the
in-SNARK STARK verifier is the bulk.

### 3.2 S1 — L2 verifier AIR column-count audit (Path-B reduction map, **now primary path**)

**Goal.** Quantify the L2 size lower bound Path B alone can
reach **at the ≥80-unconditional bar** (§1.4) and produce a
concrete reduction map.

**Scope.**
- Use S(−1)'s parameter map to determine the smallest FRI
  parameters that hit ≥80 unconditional at the L2 layer
  (expected: `lb=2, nq≈40` or similar — see §1.4.B).
- Read-only audit of `Plonky3-recursion/recursion/src/verifier/*`
  + `circuit/src/ops/tip5_perm/*` + the verifier circuit's
  AIR layout.
- Column-by-column table: `name → role → is_removable_at_80
  unconditional → estimated_byte_savings`.
- Identify constraint degrees > 2 that could be re-expressed
  via auxiliary witness columns at lower degree (degree drops
  reduce FRI commitment depth and OOD-open width).
- Identify table-merge opportunities (multiple AIR tables that
  can fuse into one wider table, saving per-table OOD-open
  overhead).
- Produce a **measured lower bound** for L2 size after a
  maximally aggressive Path-B reduction *at the new bar*.

**Decision branch from S1's output:**
- If predicted L2 size ≤ 65 KB ⇒ proceed directly to S4-B
  (Path-B-alone landing). Path A is not needed.
- If predicted L2 size > 65 KB ⇒ S0 (Path-A fallback prototype)
  executes; S4 lands as B+A.

**Exit gate.** A committed audit document
(`crates/ai-pow-zk/docs/2026-05-XX_PATHB_REDUCTION_MAP.md`) with
the column-by-column table, the predicted post-reduction L2
size at ≥80 unconditional, and the **decision-branch verdict**
(B-alone feasible? yes / no / measurement-uncertain).
**No code change.**

**Estimated effort.** 2–3 working days; the new bar makes the
analysis more useful (real decision gate, not just lower-bound
sanity).

### 3.3 S2 — (Conditional, maintainer-elected) Path-C KAT-first prototype

**Goal.** If the maintainer wants Path C on the option list, do
the same KAT-first toy-circuit pattern as S0 but with
Sonobe / Nova-IVC. Otherwise skip.

**Scope.** Mirror of S0 but with R1CS-encoded toy STARK
verifier and Nova / HyperNova accumulator.

**Exit gate.** Toy IVC accepts + tampered rejects; size +
prover-cost measured; **excluded workspace.**

**Estimated effort.** 5–7 working days if elected; the R1CS
embedding is novel.

### 3.4 S3 — Maintainer decision (**B-alone** vs B+A vs B+D+A vs B+C)

After S(−1)/S1 (and S0/S2 if elected), present the maintainer
with:
- Measured ≤65 KB feasibility **at ≥80 unconditional** per
  path.
- Audit-surface cost per path (per-path delta against the C4
  audit-readiness package — `2026-05-19_C4_AUDIT_READINESS.md`).
- Prover wall + RSS per path.
- R1 risk profile per path.

The maintainer picks. **B-alone is now an explicit option**
(was not pre-§1.4). **No work past S3 begins without that
explicit decision** (R1 — soundness-architecture decisions
require sign-off).

### 3.5 S4 — Invasive substrate addition (path-dependent)

**Path A landing (if chosen):**
- S4-A.1: vendor the chosen SNARK crate + curve crate into a
  new in-tree excluded workspace `Plonky3-recursion-snark/`.
  Rev-pinning + delta documented.
- S4-A.2: build the production in-SNARK verifier circuit of the
  ≥120-bit M-S5 outer-recursive STARK. KAT it against a known-
  valid M-S5 cert (accept) + a tampered one (reject).
- S4-A.3: integrate via a new `terminal_compression` API in
  ai-pow-zk's bridge; do **not** alter `verify_p3_batch_proof_circuit`
  or any AIR.
- S4-A.4: size-validate ≤ 65 KB on the real ≥120-bit M-S5 cert.

**Path B landing (if chosen):**
- S4-B.1: implement the S1 reduction map as a new verifier-AIR
  variant `tip5-circuit-air-narrow` alongside the existing one
  (additive; keep the existing AIR byte-identical until parity
  is proven).
- S4-B.2: KAT the narrow AIR against the existing AIR's accept/
  reject behavior on identical inputs.
- S4-B.3: flip the verify path to the narrow AIR (CRIT-1-style
  staged flip).
- S4-B.4: size-validate.

**Path C landing:** mirror of S4-A with Sonobe + R1CS embedding.

### 3.6 S5 — Acceptance gate

The byte-verbatim test in
`Plonky3-recursion/recursion/tests/test_tip5_layer0_recursion.rs::tip5_layer0_outer_cert_size_residual`
un-`#[ignore]`s and passes, **at ≥120-bit soundness**, with
a *tamper-rejection* gate at the same scale (analogous to the
DT-4 tamper test).

Full regression: `ai-pow-zk --lib`, `ai-pow --features zk`,
`p3-recursion` recursion tests, `p3-tip5-circuit-air` tests — all
green. Fenced linchpin `git diff` empty across the C2/DT-4 set.

---

## 4. Exit gates per stage (gating discipline)

| Stage | Gate (concrete, falsifiable) |
|---|---|
| **S(−1)** | Committed soundness-analysis note: explicit `(lb, nq, pow_bits) → unconditional bits at Johnson radius` formula; per-layer table verifying every M-S5 link is comfortably ≥80 unconditional; explicit `γ < J(δ)−η` check at every layer (paper §8 attacks avoided). **No code change.** |
| **S0** | (Conditional — only if S1 verdict requires Path A.) Excluded workspace builds; toy STARK SNARK-wrap accepts valid + rejects tampered (both Groth16 + Plonk variants); measured final size + prover cost reported in a committed `S0_PATHA_KAT.md`. **No in-tree dependency added.** |
| **S1** | Committed reduction-map doc with **the §3.2 decision-branch verdict** (B-alone feasible: yes / no / measurement-uncertain); measured post-reduction L2 size at ≥80 unconditional; identified all degree-drops + table-merge opportunities. **No code change.** |
| **S2** | (Conditional) Same as S0 for Path C. |
| **S3** | Maintainer-signed path decision (B-alone / B+A / B+D+A / B+C) recorded in a roadmap update + memory entry. |
| **S4** | Per-path: substrate added (in-tree for Path B; vendored excluded workspace for Path A/C); in-circuit verifier KAT'd accept + reject vs the real ≥120-bit M-S5 cert; size measured at ≥80 unconditional; **no fenced linchpin edit** at the M-S5 inner. |
| **S5** | `tip5_layer0_outer_cert_size_residual` passes; full regression green; tamper-reject test added at the M-S5b layer. |

---

## 5. Soundness invariants — what M-S5b must not break

1. **C3 / M-S5 ≥120-bit chain stays byte-identical at every
   internal link** (`14116b0` + `259cab2` diff sets). M-S5b
   wraps the existing outer cert; it does not re-prove it with
   different parameters. Verified by `git diff` against the
   landed M-S5 tree per stage commit.
2. **Tip5 AIR (C2.1 / 62413ba)** untouched. KAT-anchored to
   `nockchain_math::tip5::permute`.
3. **DT-4 duplex binding** in `Plonky3-recursion/circuit/src/ops/tip5_perm/executor.rs`
   (the pre-swap vs post-swap state capture) untouched. The
   tamper test in
   `Plonky3-recursion/recursion/tests/test_tip5_layer0_recursion.rs`
   must continue to reject when M-S5b wraps it.
4. **WitnessChecks LogUp** producer/consumer multiplicity untouched.
   M-S5b adds a *new outer layer*, not new producers/consumers in
   the existing layers.
5. **Pearl-byte-equivalence** of the *mineable unit* unchanged.
   M-S5b operates on the SNARK side only;
   `compute_tile_from_slices` / `TileState::keyed_hash`
   semantics are not in M-S5b's scope.

---

## 6. R1 discipline — explicit residual + escape hatches

### 6.1 What this document commits

- Design doc itself (`2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`).
- Roadmap row M-S5b annotated with link to this doc.
- Memory entry for the M-S5b residual.
- Task `#131` annotated `in_progress` (design stage), with
  follow-on tasks for S0, S1, (S2), S3.

### 6.2 What this document does **not** commit

- **No** invasive code edit to `Plonky3-recursion` or any in-tree
  AIR.
- **No** vendored substrate (pairing curve, SNARK frontend,
  folding scheme).
- **No** parameter change to `goldilocks_tip5*` configs.
- **No** edit to the byte-verbatim `assert!(serialized_len <=
  65_536, …)` test; only the surrounding plan it lives under.

### 6.3 R1 residual (precise, actionable; re-anchored to §1.4)

The *minimum next session*'s deliverable, in order of
recommended sequence:

1. **S(−1) — paper-grounded soundness analysis** (~1–2 days):
   committed soundness-analysis note grounding our FRI
   parameters in IACR ePrint 2025/2055 Theorem 1.5 + Johnson-radius
   bound; per-layer ≥80-unconditional table.
2. **S1 — Path-B reduction map** (~2–3 days): committed audit
   doc with the decision-branch verdict at ≥80 unconditional.
3. **Conditional S0 / S2** (only if S1's verdict requires Path
   A / C as a fallback): excluded-workspace KAT-first
   prototypes.

None of these touch the fenced linchpin or alter the M-S5
≥120-bit cert. The R1 validated-subset + precise-residual
ladder is structurally enforced.

---

## 7. Open maintainer questions (block S3)

These need explicit answers before the substrate addition
begins:

1. **(Conditional, only if Path A is in play) Curve preference.**
   BN254 (boundary 128-bit, smallest proof) vs BLS12-381
   (~128-bit, comfortable margin). At the new ≥80-unconditional
   bar both clear the floor with margin; the prover-cost
   tradeoff dominates. Resolved automatically if S1 verdict =
   B-alone (Path A not needed).
2. **(Conditional) Setup model.** Groth16 trusted-setup-per-
   circuit vs Plonk universal updateable setup. Same
   conditional as above.
3. **Path C inclusion.** Should S2 (Path C KAT-first
   prototype) run, or is the maintainer comfortable narrowing
   the option set to B-alone / B+A / B+D+A after S(−1) / S1?
   (Less attractive at the new bar — Path B-alone may suffice.)
4. **Acceptable amortization horizon.** Path A's outermost SNARK
   prover may take minutes/tile. Only relevant if Path A is
   selected after S1.
5. **External audit timing.** Does M-S5b land before or after
   C4 (`#125`) audit-readiness is shipped? If before, the audit
   surface grows mid-audit; if after, the M-S5b additions are
   in a follow-on audit round. **The new bar makes B-alone a
   strict subset of the existing C4 audit surface (no new
   primitive), so timing flexibility increases.**
6. **S(−1) ownership.** Does this analysis live with the
   maintainer or is it part of the C4 external audit (i.e.,
   ask the auditor to do the formula derivation as part of
   their soundness review)? Recommended: do it internally
   first so the audit *reviews* a concrete claim rather than
   deriving from scratch.

---

## 8. Cross-references

- **Soundness-bar anchor paper (the §1.4 foundation):**
  Ben-Sasson, Carmon, Habock, Kopparty, Saraf, *"On Proximity
  Gaps for Reed–Solomon Codes"* (IACR ePrint 2025/2055, Nov 2025).
  Theorem 1.5 + §1.3.2 (Johnson-radius proven, linear-in-n
  loss). §1.4 negative results + §8 attacks (beyond-Johnson
  unsafe).
- **C3 / M-S5 landed state (cite for invariants):**
  `2026-05-19_C3_OUTER_CERT_DESIGN.md` §13.2 (DT-4 fix),
  §14 (size measurement — the empirical foundation of this
  doc), §15 (≥120-bit cert LANDED — comfortably above the new
  ≥80-unconditional floor).
- **C4 audit-readiness package** (sibling deliverable, this
  session): `2026-05-19_C4_AUDIT_READINESS.md`.
- **C2 Tip5 AIR (soundness linchpin):**
  `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` §2c (L4/L5/C2.4).
- **C1 vendor (substrate floor):**
  `2026-05-18_C1_RECURSION_VENDOR_DESIGN.md`.
- **Roadmap:** `2026-05-17_PRODUCTION_ROADMAP.md` Phase C row
  M-S5b.
- **Pearl §4.7/§5.1** — origin of the ≤65 KB target:
  `2026-05-17_M_S2_PEARL_EVALUATION.md`; Pearl Whitepaper.
- **R1 / R1.1 discipline:** `~/.claude/CLAUDE.md` R1, R1.1.

---

## 9. Acceptance criterion (definition of done for M-S5b)

`tip5_layer0_outer_cert_size_residual` (currently `#[ignore]`d
with reason pointing at this milestone) un-`#[ignore]`s and
passes on the **real M-S5 cert at ≥80 unconditional bits**
(Johnson-radius bound from IACR ePrint 2025/2055) at
`serialized_len ≤ 65_536`, accompanied by:

1. An **accept + tamper-reject pair** at the M-S5b layer (the
   tamper test must reject the same way the DT-4 tamper test
   rejects at M-S5).
2. Full regression green (`ai-pow-zk --lib`, `ai-pow --features
   zk`, `p3-recursion`, `p3-tip5-circuit-air`).
3. Fenced linchpin diff empty against `259cab2`.
4. The new substrate (if vendored) lives in an excluded
   workspace with rev-pinning + delta documented per C1's
   model.
5. The security report + gap audit reflect M-S5b as RESOLVED
   and C3 as terminally compressed.

Until *every* item above is true, M-S5b is **not done** — no
fake completion, no soundness trade, no premature `#[ignore]`
relaxation. R1.
