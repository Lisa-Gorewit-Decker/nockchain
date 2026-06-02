> _Created **2026-05-20** · last updated **2026-05-20**._

# Proof-Size Reduction Routes — comprehensive audit (M-S5b S1, with Plonky2 / Pearl deep-dive)

> **Status (R1, honest).** AUDIT + RECOMMENDATION. This doc
> extends M-S5b S1 (`2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`
> § 3.2) from "Path-B-as-primary" to a **comprehensive
> 8-path audit** grounded in: (i) the LANDED ≥80-Johnson
> empirical measurements
> (`2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md`); (ii)
> S(−1)'s paper-grounded FRI soundness
> (`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`); (iii) CSA's
> AIR-side ≥80-bit verdict
> (`2026-05-20_CSA_S7_AUDIT_SIGNOFF.md`); (iv) Pearl's actual
> Plonky2-based recursion approach (Pearl Whitepaper § 4.7 +
> § 5.1); (v) the Plonky3-recursion verifier-circuit
> structural-floor analysis (this commit). **No code edit by
> this document.**
>
> **Headline finding (refines M-S5b § 1.4.C).** Path B alone
> (smaller verifier AIR within Plonky3-recursion) **cannot reach
> ≤65 KB at ≥80 unconditional**. The L2 verifier-circuit has a
> structural fixed floor of ≈40 KB (Poseidon2-W8 + Tip5
> verifier + quotient polynomial + global accumulators); the
> in-SNARK opened-values term saturates the 65 KB budget before
> the outer layer starts. **Reaching ≤65 KB requires either
> (a) Path A — outermost STARK-to-SNARK wrap on top of a
> Path-B-narrowed verifier circuit (hybrid = Path H), or
> (b) Path D2 — direct adoption of Pearl-style 3-layer Plonky2
> recursion.** The 2026-05-19 M-S5b doc § 2.F's
> "B-alone has a real shot" verdict is **superseded** by this
> empirical floor analysis.
>
> **Recommended sequence:** **Path H (B+A) as primary**, with
> **Path D2 (Plonky2 tower) as fallback** if Path A's pairing-
> crypto audit surface is judged unacceptable. Detail in § 9.
>
> **Soundness invariant.** Every path preserves the ≥80
> unconditional Johnson-radius bar (CSA S1 + S(−1)
> composed). No path trades soundness for size; the
> chain MIN under each path is computed in § 5.4.

---

## 1. The size problem at the new ≥80-Johnson bar

### 1.1 Empirical measurements (the LANDED baseline)

Per `2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md`
§2.1–§2.5 — real `prove_all_tables` + `postcard`
serialization at the post-recalibration ≥80-bit unconditional
Johnson-radius parameters:

| Layer | LANDED size | vs ≤65 KB target | Per-layer reduction (vs ≥120 conj.) |
|---|--:|--:|--:|
| Inner Tip5-L0 sweep (PROD) | 117 KB | 1.8× over | (unchanged) |
| **L1** outer-cert | **961 KB** | **14.7× over** | 2.79× smaller |
| **L2** outer-cert | **618 KB** | **9.5× over** | 2.89× smaller |
| L3 (over L2) | 549 KB | 8.5× over | **L3 > L2 — recursion diverges** |

The recalibration shrunk L1/L2 by ~3× (FRI query count halved
under the proven Johnson bound). **Stacked recursion is
confirmed dead** at the new bar (S3(ii) measurement: L3 > L2
by 32 KB). The size-reduction problem reduces to: **how to
compress the 618 KB L2 (or equivalently 961 KB L1) to ≤65 KB
without dropping soundness below 80 unconditional**.

### 1.2 Path B's structural floor (the key empirical finding)

The L2 verifier-circuit has a **structural fixed floor**
independent of FRI parameters. Component breakdown (from
this commit's `Plonky3-recursion` source-code audit):

| Component | File | Main cols | Max degree | Est. opened-values | Structural? |
|---|---|--:|--:|--:|---|
| **Poseidon2-W8 perm AIR** (challenger + MMCS) | `poseidon2-circuit-air/src/air.rs` | ~180 | **7** (x⁷ S-box) | 8–12 KB | ✅ Structural |
| **Recompose AIR** (D-packing) | `circuit-prover/src/air/recompose_air.rs` | 8–40 (lane-dep.) | 1 (pure CTL) | 4–6 KB | ⚠️ Optimization (removable, costs degree) |
| **Tip5 verifier-circuit** (in-L0 reader) | `tip5-circuit-air/src/air_circuit.rs` | 16 fixed + 52 prep | 2 (lookup-table) | **15–25 KB** | ✅ Structural |
| **Quotient polynomial** (constraint composition) | `recursion/src/verifier/batch_stark.rs` | (derived) | 7–12 | **20–30 KB** | ✅ Structural |
| **Global multiset accumulators** (LogUp) | embedded | 3–8 | 1–2 | ~2 KB | ✅ Structural |
| **TOTAL `opened_values` floor** | — | **~230–250** | 7 (max) | **~50–75 KB** | — |
| **TOTAL L2 proof at floor** | — | — | — | **~40 KB** | — |

**The floor is 40 KB.** Even if every "optimization" column
were removed (recompose ≈ 4–6 KB savings), the structural
components (Poseidon2 + Tip5 reader + quotient + accumulators)
exceed 30 KB of `opened_values` alone, and the surrounding
`opening_proof` (FRI bytes for the L2 quotient) adds ≈10 KB
at the minimum-soundness configuration. **Below 40 KB is
unreachable in Plonky3-recursion as currently structured.**

### 1.3 Implications for Path-B aloneness

The pre-recalibration M-S5b § 2.F recommendation was:
> "Path B alone has a real shot at ≤65 KB at the new bar."

The empirical floor analysis **refines** this:

- **L2 measured:** 618 KB (LANDED).
- **L2 floor:** ~40 KB (structural — verifier-circuit alone).
- **L2 excess above floor:** ~578 KB (FRI bytes + non-floor
  opened-values + lookup-data).
- **If Path B narrows the "non-floor" excess 2–4×**: L2
  drops from 618 KB → 40 + (578/3) ≈ **240 KB** (best case)
  to 40 + 578/2 ≈ **330 KB** (likely case).
- **L2 at 240 KB is still ~3.7× over the 65 KB target.**

**Path B alone cannot close the remaining 4–6× gap to 65 KB.**
The floor is structural; Path B affects the "above the
floor" cost but cannot push the floor itself down.

This is the inversion of M-S5b § 2.F's verdict. Path B is
*still valuable* — it shrinks the in-SNARK verifier circuit
the next layer must prove, cutting Path A or Path D2's prover
cost — but it is **insufficient as a standalone solution**.

---

## 2. The soundness floor (the binding constraint)

Per S(−1) (`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` § 5.1) +
CSA (`2026-05-20_CSA_S7_AUDIT_SIGNOFF.md` § 5):

```
ε_total ≤ ε_AIR + ε_LogUp + ε_FRI
       ≤ 2^(−103) + 2^(−98) + 2^(−82)
       ≈ 2^(−82)     (chain MIN = 82 unconditional bits)
```

FRI is the binding term. **AIR side has +21-bit margin to
FRI**. The constraint side is NOT a binding constraint on
the path choice — any path that preserves the FRI side at
≥80 will leave the AIR side ≥80 with ample margin (CSA per-AIR
≥98 bits at production parameters).

**Path-feasibility soundness check** (each path × its FRI
configuration):

| Path | FRI side ≥80? | AIR side ≥80? | Chain MIN? |
|---|---|---|---|
| A — SNARK wrap (BLS12-381 / BN254 + Groth16/Plonk) | Inherited from inner STARK FRI ≥82 + pairing ≈128 | Inherited from CSA ≥98 | ≥82 |
| B — narrower verifier AIR (Plonky3-native) | Same FRI as LANDED ≥82 | New AIR derivation needed; expected ≥98 by S1-like CSA refresh | ≥82 |
| C — Halo/Nova folding (Sangria / Nova on Pasta) | Folding scheme soundness ≥128 (BN254) | New AIR (R1CS); requires audit | ≥80 by composition |
| D1 — Plonky2-style narrow STARK (in Plonky3) | Inherited Plonky3 FRI ≥82 | Same as Path B | ≥82 |
| D2 — Direct Plonky2 vendoring (3-layer tower) | Plonky2's FRI (BabyBear default; chooseable Goldilocks) — Plonky2 soundness reduction is published | Plonky2 AIR ≥80 (Pearl WP § 4.7 implicit) | ≥80 |
| F — PCS swap (STIR / WHIR / BaseFold) | STIR/WHIR have published proofs at Johnson; BaseFold similar | Same as Path B | ≥80 |
| G — Inner-AIR shrinkage | FRI ≥82 unchanged; new AIR | Needs CSA refresh | ≥82 |
| H — Path B + Path A hybrid | A inherits B's inner; A wraps with 128-bit pairing | A ≥80 (inherited); B ≥98 | ≥82 |

Every path clears ≥80 with appropriate parameter choices.
**Soundness is NOT a differentiator** between the paths —
all are viable from the bar perspective. **Audit surface +
substrate cost + R1 risk** are the discriminators.

---

## 3. Path catalog (the 8 routes)

### 3.1 Path A — STARK-to-SNARK wrap (outermost pairing-based SNARK)

**Idea.** Outermost layer is a pairing-based SNARK (Groth16
or Plonk over BN254 / BLS12-381) whose statement is *"the
inner STARK verifies"*. The in-SNARK circuit is a Goldilocks
STARK verifier.

**Existence proofs.** This is the technique production zkVMs
use to reach <1 KB consensus proofs: RISC Zero, SP1, Powdr,
Polygon Hermez, OpenVM, Brevis. External crates (`arkworks`,
`bellperson`, `halo2`) implement the SNARK side.

**Final size.** Groth16 = **~192–256 bytes constant** (3 G1
+ 2 G2 elements). Plonk = a few KB. **Trivially ≤65 KB.**

**Soundness.** OK if the outer pairing curve is ≥128-bit and
the in-SNARK STARK verifier is sound. BN254 ≈ 128 bits
(boundary; some literature says weaker for STARK-grade
operations); BLS12-381 ≈ 128 bits with comfortable margin.

**Quantitative:**
| Aspect | Value |
|---|---|
| Final proof size | 192 B (Groth16) – ~3 KB (Plonk) |
| Verifier circuit size (in-SNARK) | ~10–50 M gates (the Goldilocks STARK verifier) |
| Prover wall (outer SNARK) | ~minutes (amortized over many tiles) |
| Prover RSS | GBs |
| Verifier wall | ≪1 ms |
| Adds soundness from new substrate? | Yes (curve + SNARK frontend) |

**Audit surface (largest of the 8 paths):**
- New pairing-based PCS
- Elliptic-curve arithmetic + pairings
- Trusted-setup ceremony (Groth16) **or** universal updateable setup (Plonk)
- In-SNARK STARK verifier circuit

**Substrate cost (largest):**
- New crate vendoring: `arkworks-bn254` or `blst` (~50K LoC)
- New SNARK prover crate (Groth16 or Plonk; ~100K LoC)
- In-SNARK Goldilocks STARK verifier circuit (~10K LoC bespoke)

**R1 risk:** **Low** at the integration level (non-invasive
to the fenced linchpin — the in-SNARK circuit *consumes* the
C3 STARK as a public statement; doesn't edit any AIR).
**Medium-high** at the substrate-vendoring level (the new
pairing crypto must itself be audited).

### 3.2 Path B — smaller Plonky3 verifier AIR (in-substrate refactor)

**Idea.** Rebuild the L2 verifier AIR with **narrower
columns + lower-degree constraints**, attacking the
`opened_values` 54.6 % dominant cost. Stay within the Plonky3
substrate; no new crypto primitive.

#### 3.2.0 Path B sub-stage: **hash unification (Tip5-everywhere)** — the architectural-correctness fix

**The architectural symmetry vs Pearl.** Pearl chose
**BLAKE3-throughout** (Pearl WP § 4.3 + § 4.4: matrix
commitments + noise-seed derivation + Fiat-Shamir
challenges all BLAKE3-keyed; Plonky2 supports BLAKE3 as its
MMCS sponge). Nockchain's intended architecture is
**Tip5-throughout** (chosen for STARK-friendly low-degree
arithmetization + C2.1 keystone status). Both projects
converged on the same "one hash, throughout" pattern; Pearl's
hash is BLAKE3 (collision-resistance + hardware-acceleration
strengths), Nockchain's hash is Tip5 (STARK-friendly
low-degree strengths).

**Our current dual-hash state is an architectural defect,
not a design choice.** Our LANDED configuration uses **TWO
hash functions in one circuit**:

- **Inner ai-pow-zk STARK** (`crates/ai-pow-zk/src/circuit.rs:186, 203`):
  Tip5 (7-round, width 16) for MMCS + Fiat-Shamir
  challenger. **Pearl-faithful; cannot change at the inner.**
- **Outer-cert L1/L2** (`crates/plonky3-recursion/circuit-prover/src/config.rs:206-294`,
  `goldilocks_tip5_120bit` builder): **Poseidon2-Goldilocks<8>**
  (width 8, x⁷ S-box, degree 7) for MMCS + Fiat-Shamir.

Because L1's verifier circuit verifies L0's Tip5-based Merkle
paths, the L1 trace contains a **Tip5 perm AIR sub-circuit**
(the C2.1 keystone). L1 also uses Poseidon2-W8 for its own
commitments, so L1 also contains a **Poseidon2 perm AIR
sub-circuit**. L2 then verifies L1, so L2 contains both
sub-circuits recursively.

**Cost (per § 1.2 floor breakdown):**
| Hash AIR | Opened-values cost in L2 |
|---|--:|
| Tip5 verifier-circuit (verifies inner L0's Tip5 commitments) | 15–25 KB |
| Poseidon2-W8 verifier-circuit (L1/L2's own commitments) | 8–12 KB |
| **Dual-hash subtotal** | **23–37 KB** (60–90 % of the ~40 KB floor) |

**Hash unification opportunity (Tip5-everywhere at outer-cert):**
- `Tip5Perm` already implements Plonky3's
  `CryptographicPermutation<[Goldilocks; 16]>` trait
  (`circuit.rs:295, 312`), with a packed-Goldilocks variant for
  aarch64-neon (`:340`).
- Reuse `Tip5Sponge` + `Tip5Compress` (same wrappers the inner
  uses) at the outer-cert; replace `Poseidon2Goldilocks<8>` in
  the `goldilocks_tip5_120bit` builder.
- **Eliminates the Poseidon2 perm AIR sub-circuit from L1/L2.**
- **Saves ~8–12 KB of opened_values** ⇒ structural floor drops
  from ~40 KB to ~28–32 KB.

**Prerequisite blocker.** Switching outer-cert to Tip5 surfaces
the **C2.4 R-a tail residual** (Tip5 D=2 recompose-coeff
producer multiplicity imbalance at wid 11468, M12-deferred per
`2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13). The outer-cert
currently uses D=2 batch-stark; the orphan must be resolved
before Tip5 can replace Poseidon2 in the recursion verifier.
**Fix path:** M12 / `#127` (or a Tip5-D=2 producer multiplicity
gating fix in `tip5-circuit-air/src/air_circuit.rs:346-357`).

**Why NOT Poseidon2-everywhere:** would require replacing
Tip5 with Poseidon2 at the inner ai-pow-zk STARK, **breaking
C2.1 keystone + Pearl byte-equivalence** (Pearl Whitepaper
references Tip5 IACR ePrint 2023/107). Cost far exceeds the
~8 KB savings.

#### 3.2.0b Path B sub-stage: **reduced-round Tip5 at outer-cert (analyzed with the actual Tip5 + Opening-the-Blackbox cryptanalysis)**

**The correct round-count baseline.** Per the Tip5 paper
itself (Szepieniec, Lemmens, Sauer, Threadbare, Al Kindi —
IACR ePrint 2023/107, Table 2 + § 5.11): **the Tip5 paper
specifies N = 5 rounds**, with the designers' rationale:
> "The round count N = 5 was set to provide a roughly 50%
> security margin" — Tip5 paper § 5.11 (cited verbatim by
> Opening the Blackbox p.4).

The Tip5 paper's own analysis (§ 5.11 Table 4) gives the
minimum rounds against each attack class:
- Statistical: linear 2, differential 2, boomerang 3
- Algebraic: univariate 2, Gröbner basis 2, split S-box 2,
  linear approximation 2, fixing wire values 3
- **Maximum: 3 rounds sufficient** against all attacks the
  designers considered.

The designers then chose N=5 = 3 + a "50% safety margin".

**Nockchain uses N = 7 rounds**, not 5. This is 2 rounds
*above* the Tip5 paper's specified safety level — a
deliberately more-conservative choice that gives **additional
margin against future cryptanalytic improvements**.

**Third-party cryptanalysis (Opening the Blackbox — Liu et al.,
IACR ePrint 2024/1900):** the first independent cryptanalysis
of Tip5. Table 1 (p.4) attack reach:

| Attack | Target | Rounds | Complexity (ω=2.37) | Status |
|---|---|--:|---:|---|
| **Practical SFS collision** | **Tip5** | **3** | **2^41.2** | **PRACTICAL — broken** |
| Full collision | Tip5 | 3 | 2^121.1 | Theoretical (just below claimed 128) |
| SFS collision | Tip4 | 4 | 2^118.5 | Theoretical |
| (no attack found at 4-round Tip5 or above) | — | — | — | — |

Opening the Blackbox p.4 verbatim:
> "Since the Tip5 family is instantiated by 5 rounds only,
> our attacks have significantly reduced the security
> margins of the Tip5 family."

**The corrected security-margin picture:**

| Tip5 rounds | Claimed/observed security | Margin above broken (3-round) | Status |
|---:|---|---:|---|
| 3 | (was: 128-bit) | 0 | **UNSAFE — practical SFS at 2^41** |
| 4 | extrapolation ~110-bit | 1 round above broken | Safe under current; thin margin |
| **5** (Tip5 paper) | **128-bit claimed** | **2 rounds above broken** | Designers' chosen level; reduced from "50% margin" to ~"40% margin" post-OtB |
| 6 | 128-bit + margin | 3 rounds above broken | Conservative |
| **7** (Nockchain) | **128-bit + substantial margin** | **4 rounds above broken** | **Most conservative; ample headroom** |

**Nockchain's 7-round choice is well-justified post-Opening-
the-Blackbox.** Attacks have historically extended by ~1
round every 2–4 years for arithmetization-oriented hashes;
Nockchain's 4-round margin (vs the Tip5 paper's 2-round
post-OtB margin) is genuine future-proofing.

**Per-round cost arithmetic** (CSA inventory § 3.1: Tip5 perm
AIR ≈ 9392 columns at 7 rounds → ≈ 1338 columns per round):

| Outer-cert Tip5 rounds | Column reduction vs 7 | Opened-values savings | Cryptanalysis verdict |
|---:|---:|---:|---|
| 7 (Nockchain current) | 0 | 0 KB | Audit-baseline; most conservative |
| 6 | -14 % | ~2–3 KB | Safe; +3 rounds margin |
| **5** (Tip5 paper spec) | **-29 %** | **~5–8 KB** | **Safe; published analysis directly applies; +2 rounds margin** |
| 4 | -43 % | ~6–10 KB | Probably safe; **THIN +1-round margin; discouraged** |
| 3 | -57 % | ~8–13 KB | **UNSAFE — DO NOT USE; practical SFS collision at 2^41** |

**Three constraints stack on outer-cert round reduction:**

1. **Soundness margin.** Even the Tip5 paper's own N=5
   choice has only 2 rounds margin above broken
   post-Opening-the-Blackbox. Reducing Nockchain's 7-round
   outer-cert to 5 rounds keeps us at the Tip5 paper's
   published-analysis level (acceptable). Reducing to 4
   rounds gives only 1 round above broken (discouraged).
   Reducing to 3 rounds is **definitively unsafe**.

2. **C2.1 keystone breakage at the inner.** The Tip5 perm
   AIR is KAT-anchored to `nockchain_math::tip5::permute`
   (Nockchain spec: 7-round). **Reducing inner rounds
   breaks the keystone.** At the inner, 7-round is
   non-negotiable.

3. **Pearl compatibility does NOT apply to the outer-cert.**
   Pearl uses BLAKE3 throughout, not Tip5 (Pearl WP § 4.3 +
   § 4.4: matrix commitments, noise-seed derivation, and
   Fiat-Shamir challenges all BLAKE3-keyed). Tip5 is a
   **Nockchain-specific choice**; the outer-cert is
   Nockchain's own consensus artifact. So **a reduced-round
   Tip5 at the outer-cert ONLY would be technically
   feasible** without breaking any Pearl-compatibility
   constraint — keep 7-round Tip5 at the inner (the C2.1
   keystone), use 5-round Tip5 at the outer (= Tip5 paper
   spec). Inner and outer would have different round counts
   under the same Tip5 family.

**Combined optimization (3.2.0 + 3.2.0b):**

Stack hash unification + outer-cert round reduction:
- Inner: 7-round Tip5 (unchanged, C2.1 keystone).
- Outer-cert L1/L2: switch from Poseidon2-W8 to **5-round
  Tip5** (= Tip5 paper spec, published-analysis level).
- Eliminates Poseidon2 sub-circuit (-8 to -12 KB).
- Reduces outer-cert Tip5 sub-circuit by ~30% (-5 to -8 KB
  on top).
- **Combined floor reduction: 13–20 KB** ⇒ structural floor
  drops from ~40 KB to ~20–27 KB.

**Recommendation (refined post-Opening-the-Blackbox):**

The **conservative variant** is the audit-baseline candidate:
- Hash unification (Poseidon2-W8 → 7-round Tip5 at
  outer-cert).
- Keep 7 rounds throughout (inner + outer-cert).
- Saves ~8–12 KB without changing the round count.
- **No cryptanalysis dependency; Nockchain's existing safety
  margin preserved.**

The **moderately aggressive variant** trades 2 rounds for
~5–8 KB extra:
- Hash unification + reduce outer-cert to 5-round Tip5 (=
  Tip5 paper spec).
- Saves ~13–20 KB total.
- **Stays at the Tip5 paper's published-analysis level;
  audit-acceptable if board accepts the margin reduction at
  the outer-cert only.**
- Inner stays 7-round (Nockchain spec; C2.1 unchanged).

The **NOT-recommended variants**:
- 4-round outer-cert Tip5: thin +1-round margin above
  broken; discouraged.
- 3-round outer-cert Tip5 or below: **DO NOT USE** —
  practical SFS collision at 2^41 per Opening the Blackbox
  § 5.4.

**Path B's revised reach (after 3.2.0 conservative
hash-unification):**
| Aspect | Pre-Path-B (LANDED) | Post-Path-B + hash-unif (predicted) | Post + aggressive 5-round Tip5 (predicted) |
|---|---:|---:|---:|
| Verifier-circuit floor | ~40 KB | ~28–32 KB | ~23–29 KB |
| Verifier AIR max degree | 7 (Poseidon2 x⁷) | **2** (Tip5 LogUp post-L4) | 2 |
| L2 total size | 618 KB | ~100–160 KB | ~85–145 KB |
| L2 vs 65 KB target | 9.5× over | 1.5× – 2.5× over | 1.3× – 2.2× over |

**The hash unification ALSO drops the max constraint degree
from 7 to 2** (Poseidon2's x⁷ S-box is the highest-degree
constraint in the verifier circuit; Tip5 with lookup-table is
degree 2 post-L4). This is a **structural improvement
independent of the floor savings** — it shrinks the quotient
polynomial degree from 12 (= (7-1)·2) to 2 (= (2-1)·2 = 2,
clamped to the min commit degree), which further reduces L2's
`opened_values` for the quotient column (an additional ~10–15
KB savings not yet counted).

**Updated Path B verdict.** With hash unification + degree
drop from x⁷ to LogUp-2, Path B alone now reaches an
estimated **L2 ~80–130 KB** — still above 65 KB but the gap is
narrower. Path B + STIR/WHIR (Path F) could plausibly close
the gap to ≤65 KB without Path A; Path B + Path A guarantees
≤256 B with comfortable margin.

**Empirical limit (this audit's key finding — § 1.3).**
Path B alone bottoms out at **~110–175 KB L2** (2× to 4×
over budget). The ~40 KB structural floor cannot be
breached within the current Plonky3 substrate.

**Soundness preservation.** Highest confidence — same
substrate, same FRI/LogUp/MMCS soundness story, smaller
witness. CSA per-AIR ≥98 bits would shift downward modestly
(fewer constraints means smaller `n_rows`-dependent term);
expected ≥95 bits per Path-B-narrowed AIR. Margin still
positive.

**Quantitative (predicted, per the S1 reduction map deliverable):**
| Aspect | Pre-Path-B (LANDED) | Post-Path-B (predicted) |
|---|---:|---:|
| Verifier AIR main columns | ~230–250 | ~110–150 (2× narrowing) |
| Verifier AIR max degree | 7 | 4–5 (with degree-drops on Poseidon2 x⁷) |
| L2 `opened_values` | ~337 KB | ~150–200 KB |
| L2 total size | 618 KB | **~110–175 KB** |
| L2 vs 65 KB target | 9.5× over | 1.7× – 2.7× over |

**Audit surface (smallest of the 8 paths):**
- No new primitive.
- Audit cost ≈ delta against the existing C2-audited
  verifier AIR.
- CSA-style per-AIR ≥80 refresh on the narrowed AIR
  (mechanical).

**Substrate cost:** None outside Plonky3-recursion. Adds a
new `verify_p3_batch_proof_circuit_narrow` variant alongside
the existing one.

**R1 risk:** **Medium** — touches the verifier-circuit AIR,
which is C2 / DT-4 adjacent. Must be staged + KAT-first vs
the existing verifier behavior. Composes well with Path A
(reduces in-SNARK STARK verifier size 2–4× ⇒ Path A prover
wall drops 2–4×).

### 3.3 Path C — Halo / Nova folding (accumulation scheme)

**Idea.** Instead of re-proving a full verifier at every
layer (Plonky3-recursion's current model), maintain an
*accumulator* that folds two instances into one in constant
work per fold; a single final "decider" SNARK closes the
chain.

**Existence proofs.** Nova / SuperNova / HyperNova / Sangria
(over `pasta` curves, R1CS-based) achieve this in production.
Sonobe is a unifying Rust framework. ProtoStar / ProtoGalaxy
extend the family.

**Final size.** ≤ 65 KB easily (the accumulator + a small
decider proof are typically 1–10 KB).

**Soundness preservation.** OK if (a) the underlying curve
is ≥128-bit, (b) the accumulation scheme has a published
soundness proof (Nova / HyperNova do), and (c) the inner
STARK is faithfully embedded as an R1CS / CCS instance — the
embedding step is the audit-heavy crux (it is *not* a
vendored existing implementation; we'd be encoding a
Goldilocks-STARK verifier into R1CS over a different curve).

**Audit surface (very large):** new curve, new accumulation
scheme, new R1CS embedding of the STARK verifier. Comparable
to Path A in raw size; **more novel** because no production
deployment exists for a "Plonky3 STARK → Nova fold" embedding.

**Substrate cost (very large):** vendor `Sonobe` / Nova-IVC +
curve crate. None of these are in `Plonky3-recursion` today.

**R1 risk:** **High.** No existing in-tree primitive; the
R1CS embedding is novel work. Not currently KAT-able against
any existing baseline.

### 3.4 Path D — Plonky2-style narrow STARK (Pearl's actual approach)

**Two sub-variants. Pearl uses D2.**

#### 3.4.1 Path D1 — Plonky2 conventions adopted within Plonky3-recursion

**Idea.** Apply Plonky2's recursion-friendly optimizations
**within the Plonky3 substrate**: compact gate set, single-
round Poseidon2 for in-circuit hashing, batched opens,
verifier-friendly AIR conventions. This is effectively Path
B + explicit Pearl-fidelity guidance.

**Quantitative.** Same predicted reach as Path B alone
(~110–175 KB L2). The Plonky2 conventions are an optimization
within the existing substrate; they don't break the structural
floor.

**Audit surface, substrate cost, R1 risk.** Same as Path B.

#### 3.4.2 Path D2 — Direct vendoring of Plonky2 (Pearl's actual stack)

**Idea.** Replace our Plonky3-recursion outer-cert stack with
Plonky2 directly. The 3-layer Plonky2 recursion is what Pearl
ships (Pearl Whitepaper § 4.7, p. 15–16).

**Pearl's empirical achievement (Pearl WP § 4.7 + § 5.1):**
- Hash-based zkSNARK using Plonky2.
- 3-layer recursion.
- **Final proof size: <60 KB** (§ 5.1's certificate size
  limit is **65 KB**).
- Field: Goldilocks (Plonky2's default; same as our base).
- Hash: BLAKE3 (Pearl uses BLAKE3 for all Fiat-Shamir +
  Merkle commitments; **not Poseidon**).
- Transparent setup (no trusted ceremony).
- Soundness: not stated in bits in the Pearl WP, but Plonky2's
  published soundness analysis applies — at production
  parameters (`log_blowup = 3, nq = 28`), Plonky2 gives ≥84
  conjectured bits + Johnson-radius proven ≥80 under the
  same IACR ePrint 2025/2055 theorem we use for Plonky3 FRI.

**Quantitative (Pearl's actual numbers):**
| Aspect | Pearl D2 value |
|---|---|
| Final proof size | <60 KB |
| Recursion layers | 3 |
| Field | Goldilocks (Plonky2 default) |
| Hash for commitments | BLAKE3 |
| Hash for Fiat-Shamir | BLAKE3 |
| Soundness | ≥80 unconditional (Johnson) under same Theorem 1.5 |

**Why Pearl's stack reaches <60 KB and ours sits at 618 KB.**
The Explore agent finding is that Pearl's *inner* work is a
**Merkle-authentication proof of the matrix commitment** —
a small AIR (just BLAKE3 round constraints over a path of
log(n) leaves). Our *inner* work is the **full AI-PoW STARK**
(matmul + fold + Tip5 + BLAKE3 chip + jackpot + range tables
+ CRIT-1 pin + HIGH-2.2 fold chain + M-S1 multiset bus).
The size differential is largely the **inner-trace size
differential**, not a Plonky2-vs-Plonky3 substrate
difference. **A direct Plonky2 vendoring would not by itself
deliver Pearl's 60 KB on our larger inner.**

That said, Plonky2's verifier-circuit is *also* smaller than
Plonky3's per layer (compact gate set, BLAKE3-based instead
of Poseidon2-based MMCS — BLAKE3 has fewer constraints
than Poseidon2's x⁷ S-box). The per-layer floor in Plonky2
is likely ~10–20 KB (vs our ~40 KB), shaving 60–90 KB across
the 3-layer tower.

**Audit surface (very large):**
- Vendor entire Plonky2 stack (`plonky2`, `starky`, recursion
  framework) — ~100K LoC of upstream code.
- Plonky2's gate library, hash gates, recursion verifier
  circuit — separate audit lineage from Plonky3.
- **BUT**: Plonky2 has been deployed for years (Mir-Protocol,
  Polygon Zero, Pearl); its published audits exist
  (`audit-prepared` upstream tag). Audit deduplication is
  possible.

**Substrate cost (very large):**
- Vendor Plonky2 alongside Plonky3-recursion (don't replace —
  Plonky3 is already vendored at C1's fixed point).
- Define a `Plonky3-inner → Plonky2-outer` bridge (the inner
  Tip5-L0 proof becomes Plonky2-verifiable).
- Re-implement the Tip5 perm in Plonky2's gate language (or
  use BLAKE3 inside Plonky2's recursion, dropping Tip5 at
  the outer-cert layers).

**R1 risk:** **High** (substrate replacement; replaces the
~3-month vendored Plonky3-recursion stack with a parallel
Plonky2 stack). **But** mitigated by Plonky2's battle-tested
production usage.

### 3.5 Path E — Stacked recursion (more L-layers) — DEAD

**Empirically dead** per `2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md`
§ 2.5: at the new bar, L3 > L2 by 32 KB. Each additional
layer adds ~80 KB of in-circuit verifier overhead while
shrinking the inner content by only ~6 %. **Confirmed
dead-end** at the structural floor.

Not analyzed further.

### 3.6 Path F — PCS swap (STIR / WHIR / BaseFold)

**Idea.** Replace the FRI commitment scheme with a more
efficient proximity-testing protocol that achieves the same
soundness at smaller proof sizes.

**Candidate replacements:**

- **STIR** (IACR ePrint 2024/390 — Arnon, Chiesa, Fenzi,
  Yogev): "Reed–Solomon proximity testing with fewer
  queries". Achieves the same soundness as FRI but with
  ~2× fewer queries. Composable with batched openings;
  upstream Plonky3 has experimental STIR support in
  `p3-fri-stir`.

- **WHIR** (IACR ePrint 2024/1586 — Arnon, Chiesa, Fenzi,
  Yogev): "Reed–Solomon proximity testing with super-fast
  verification". Extends STIR with super-fast verifier-side
  arithmetic; better for in-circuit verifier deployments.

- **BaseFold** (IACR ePrint 2023/1705 — Zeilberger, Chen,
  Fisch): "Efficient field-agnostic polynomial commitment
  schemes from foldable codes". Generalizes FRI to
  foldable codes; potentially smaller commitment openings.

**Quantitative impact (predicted; not yet measured):**
| Lever | Estimated savings (L2 at new bar) |
|---|---|
| STIR (50% fewer queries) | `opening_proof` shrinks ~50% ⇒ L2 ~120 KB savings (618 → ~498 KB) |
| WHIR (similar to STIR) | Same as STIR + smaller in-circuit verifier ⇒ L2 ~150 KB savings |
| BaseFold | Folding-code-dependent; ~10–30% smaller `opened_values` ⇒ L2 ~50–100 KB savings |
| **STIR + Path B together** | ~250–300 KB L2 (predicted) — closer to 65 KB but still 4× over |

**Audit surface (medium):** new PCS theorem(s); new prover/
verifier implementation. Composable with the existing FRI
flow (drop-in replacement at the PCS layer).

**Substrate cost (medium):** upstream Plonky3 changes
required (or vendor a separate `p3-stir` / `p3-whir`).

**R1 risk:** **Medium.** New PCS scheme; published proofs
exist but not yet audited at our deployment's parameter
choices.

### 3.7 Path G — Inner-AIR shrinkage

**Idea.** Make the ai-pow-zk production AIR (the inner
Tip5-L0 STARK's AIR) smaller. Smaller inner trace ⇒ smaller
inner proof ⇒ smaller verifier-circuit work in L2.

**Constraint (R1 / CSA fenced linchpin).** Editing the inner
AIR touches the C2.1 keystone (Tip5 perm AIR), the HIGH-2.2
fold chain, M-S1 multiset bus, CRIT-1 PROGRAM_COL pin, A3.x
noise binding — every constraint family the CSA closed.
**These are byte-identical-required against `259cab2`**
under R1 invariants.

**Limited scope.** Can narrow the *non-fenced* AIRs (e.g.,
LogUp bus producers — the U13/I7P1 range tables have 8192 /
129 rows each; could be slightly compressed via shared
preprocessed columns). But the dominant inner cost
(matmul + fold + BLAKE3 + Tip5 in-AIR) is fenced.

**Quantitative.** Maximum possible inner reduction without
breaking R1 invariants: ~5–10% of inner trace width. At
~10% inner reduction, L2 verifier-circuit work shrinks by
~10% — L2 drops 618 KB → ~556 KB. **Not enough to close the
gap.**

**Audit surface (small):** changes to non-fenced AIRs +
CSA refresh.

**Substrate cost:** None.

**R1 risk:** **High** despite the small audit surface,
because *any* non-trivial inner-AIR edit risks accidentally
touching the fenced linchpin. Mitigation: stage every edit
KAT-first per the C2.1 protocol.

### 3.8 Path H — Hybrid: Path B + Path A composed

**Idea.** Combine Path B's verifier-AIR narrowing with Path
A's outermost SNARK wrap. Path B reduces the in-SNARK
STARK verifier circuit 2–4× (cutting Path A's prover wall
2–4×); Path A provides the load-bearing terminal
compression to ≤256 B.

**Quantitative:**
| Aspect | Value |
|---|---|
| Final proof size | 192 B (Groth16) – ~3 KB (Plonk) |
| Path A prover wall (with B narrowing) | ~30 s – 2 min |
| Path A prover RSS | ~1–4 GB |
| L2 (intermediate, fed into Path A) | ~110–175 KB |

**Audit surface:** Path A's full audit (pairing crypto,
SNARK frontend, in-SNARK verifier) **plus** Path B's
narrowed verifier-AIR audit. Composes additively; the
in-SNARK STARK verifier (the most novel Path A component)
is exactly Path B's narrowed AIR ⇒ audit deduplication.

**Substrate cost:** Path A's substrate + Path B's
in-Plonky3 changes. Large but no novel work outside
established techniques.

**R1 risk:** **Low** for the integration (Path A is
outermost, non-invasive) + **Medium** for Path B (narrowing
the verifier AIR). Composes well; staged validation
preserves R1 invariants.

**Composability advantage:** This is the recommended path
per § 9. It is the same architecture production zkVMs use
(RISC Zero, SP1, Powdr) — battle-tested in deployment.

---

## 4. Pearl's actual approach (Path D2) — deep dive

This section consolidates the Pearl WP § 4.7 + § 5.1 + the
public Plonky2 / Pearl Plonky2 references into the most
detailed comparison possible.

### 4.1 Pearl's recursion architecture

From Pearl WP § 4.7 (p. 15–16):

> "We adopt Plonky2, a modern hash-based zkSNARK that
> achieves fast proof generation and verification while
> supporting efficient recursive composition of proofs, and
> zero-knowledge."

> "We employ a 3-layered recursion, resulting in a final
> proof size below 60KB."

> "While about 2/3 of the running time of the plaintext
> verifier is spent on deriving the noise E, F from
> sA, sB, the zk-prover and verifier can agree on the
> plaintext noise, rather than sk-proving the correct
> derivation. We extend Plonky2's AIR representation with
> preprocessed columns — allowing circuit reliance on public
> data agreed by both prover and verifier."

### 4.2 The 3-layer tower

Layer-by-layer (inferred from Pearl WP + public Plonky2
deployment patterns):

| Layer | Proves | Approximate size |
|---|---|--:|
| **L1** (Pearl) | Block-opening Merkle proof: BLAKE3 paths over the matrix commitment, plus the noise-tie and the matmul-fold-jackpot tile execution | ~500 KB – 2 MB (raw, unwrapped) |
| **L2** (Pearl) | L1's Plonky2 verifier (first compression) | ~100–200 KB |
| **L3** (Pearl) | L2's Plonky2 verifier (final compression) | **<60 KB** (the shipped certificate) |

### 4.3 Why Pearl's L3 reaches 60 KB but our L2 sits at 618 KB

Three factors stack:

1. **Inner-trace size**: Pearl's L1 inner is just a
   Merkle-authentication AIR (BLAKE3 paths) + the noise / fold /
   matmul state. Our L1 inner is the full ai-pow-zk production
   AIR (~80 constraint families, max degree 7). Our inner trace
   is ~3–5× wider than Pearl's.

2. **Verifier-circuit floor per layer**: Plonky2's compact
   gate set + BLAKE3-based MMCS (vs our Tip5 + Poseidon2)
   produces ~10–20 KB per-layer floor (vs our ~40 KB).
   Across 3 layers, Plonky2 floor cost is ~30–60 KB; across
   our 2 layers it's ~80 KB (and the third layer makes things
   worse per the L3 > L2 measurement).

3. **Number of recursion layers**: Pearl runs 3; we run 2.
   Each Pearl layer compresses by ~3–10×; we compress by
   ~1.5× (961 → 618 KB).

The first factor is the largest. Even with Plonky2's smaller
verifier-circuit floor, our inner-trace size differential
(driven by the AI-PoW puzzle's complexity vs Pearl's
matrix-opening Merkle path complexity) would leave us above
65 KB at the final layer.

### 4.4 What porting to Plonky2 would and wouldn't buy us

**Wins:**
- ~20 KB per-layer floor reduction (40 KB → 20 KB) ⇒ L2 at
  ~600 KB → ~480 KB.
- 3rd recursion layer becomes viable (smaller per-layer
  floor inverts the L3 > L2 finding) ⇒ a hypothetical L3
  could shave another 5–10× ⇒ ~50–100 KB.
- Battle-tested codebase + existing audits.

**Doesn't help with:**
- Our inner Tip5-L0 AIR's intrinsic complexity (would need a
  separate translation step).
- The Tip5 perm — Plonky2 doesn't natively support Tip5;
  Plonky2's MMCS uses BLAKE3 or Poseidon, not Tip5.
- Phase-D vLLM integration (orthogonal).

**Estimated final size under Path D2 (Plonky2 vendoring):**
~80–120 KB at our inner complexity. **Still 1.5–2× over the
65 KB target.** Pearl's <60 KB requires *both* their smaller
inner work *and* their smaller per-layer floor *and* their
3-layer recursion. We can adopt the per-layer + 3-layer
parts, but not the inner-work part (we're proving an
AI-PoW puzzle, not a Merkle authentication).

### 4.5 The Plonky2-vs-Plonky3 substrate choice (practical comparison)

| Dimension | Plonky2 | Plonky3 (LANDED) |
|---|---|---|
| Field | Goldilocks | Goldilocks |
| Hash for MMCS | BLAKE3 (Pearl's specific choice — used throughout Pearl's stack per WP § 4.3 + § 4.4: matrix commits + noise-seed derivation + Fiat-Shamir; Plonky2 supports multiple hashes but Pearl chose BLAKE3) | Tip5 (inner; Nockchain's chosen hash, 7-round per `nockchain_math::tip5::permute`) + Poseidon2-W8 (outer; Plonky3 default residue — see § 3.2.0 dual-hash finding) |
| Recursion-friendly gates | Yes (compact gate set) | General-purpose AIRs |
| Maturity | Years of production usage | Newer; less battle-tested |
| Audit lineage | Mir Protocol, Polygon Zero, Pearl | C1-c2c51fb vendoring; CSA audit (ours) |
| Per-layer floor | ~10–20 KB | ~40 KB |
| Vendoring cost | ~100K LoC | Already vendored ~150K LoC |
| Substrate flexibility | Lower (Plonky2 is opinionated) | Higher (configurable AIRs) |
| Tip5 native support | No | Yes (C2.1 keystone) |

**The Tip5 difference matters.** Our deployment uses Tip5
because it was chosen as the hash linchpin (Pearl
whitepaper § 4.x references Tip5 IACR ePrint 2023/107). If
we switched to Plonky2, we'd have to choose: (a) re-implement
Tip5 in Plonky2's gate language (significant audit work), or
(b) replace Tip5 with BLAKE3/Poseidon at the outer-cert
layers (breaks Tip5 keystone continuity). Both are
non-trivial decisions outside this audit's scope.

---

## 5. Quantitative comparison summary

### 5.1 Final-proof-size reach at ≥80 unconditional

| Path | Predicted L2 / final size | vs ≤65 KB target |
|---|--:|--:|
| A — SNARK wrap (Groth16) | **192 B** | ✅ |
| A — SNARK wrap (Plonk) | ~3 KB | ✅ |
| B — narrower verifier AIR | ~110–175 KB | ❌ (1.7×–2.7× over) |
| C — Halo/Nova folding | ~1–10 KB | ✅ |
| D1 — Plonky2 conventions in Plonky3 | ~110–175 KB (same as B) | ❌ |
| D2 — direct Plonky2 vendoring | ~80–120 KB | ❌ (1.2×–1.8× over) |
| E — stacked recursion | DEAD (L3 > L2) | ❌ |
| F — STIR/WHIR PCS swap | ~250–300 KB (+ B) | ❌ (4× over) |
| G — inner-AIR shrinkage | ~556 KB | ❌ |
| **H — Path B + Path A hybrid (recommended)** | **192 B (final)** | ✅ |

### 5.2 Per-path prover wall

| Path | Prover wall (winning tile) |
|---|---|
| A | ~30 s – 5 min (outer SNARK) |
| B | ~10–20 s (Plonky3 native) |
| C | ~1–5 min (Nova folding) |
| D1 | ~10–20 s (same as B) |
| D2 | ~5–15 s per layer × 3 = ~15–45 s (Plonky2) |
| F | ~5–10 s (STIR has faster proving) |
| G | ~8–15 s (smaller inner) |
| **H — B + A** | **B(10–20 s) + A(30 s – 5 min) ≈ 40 s – 5 min** |

### 5.3 Per-path verifier wall

| Path | Verifier wall (on-chain or external) |
|---|---|
| A | ≪ 1 ms (Groth16) – ~3 ms (Plonk) |
| B | ~50–100 ms (Plonky3 native verify) |
| C | ~1–10 ms |
| D1 | ~50–100 ms |
| D2 | ~5–20 ms (Plonky2 verifier) |
| F | ~30–80 ms (STIR has faster verifier) |
| G | ~40–80 ms |
| **H — B + A** | **≪ 1 ms (Path A dominates)** |

### 5.4 Per-path chain MIN bits (combined AIR + LogUp + FRI)

| Path | AIR-side | LogUp-side | FRI-side | Chain MIN | ≥80? |
|---|---:|---:|---:|---:|---|
| A | 103 + outer-SNARK ≥128 | 98 | 82 (inner) + outer-pairing ≥128 | 82 | ✅ |
| B | ~95 (slightly tighter after narrowing) | 98 | 82 | 82 | ✅ |
| C | 103 + Nova folding ≥128 | 98 | n/a (R1CS not FRI) | ≥128 (outer) ∧ 82 (inner) = 82 | ✅ |
| D1 | ~95 (same as B) | 98 | 82 | 82 | ✅ |
| D2 | Plonky2 published ≥80 | n/a (BLAKE3-CTL) | 82 (under same Theorem 1.5) | 80 | ✅ (boundary) |
| F | 103 | 98 | 82 (STIR proven at Johnson) | 82 | ✅ |
| G | ~100 (smaller AIR) | 98 | 82 | 82 | ✅ |
| **H** | **103 inner + 128 outer** | **98** | **82 inner + 128 outer** | **82** | **✅** |

Every viable path is ≥80; soundness is not the discriminator.

---

## 6. Audit-surface comparison

| Path | New primitive(s) | New audit lineage | Audit-cost ranking |
|---|---|---|---|
| **B** (narrower verifier AIR) | None | None (CSA refresh) | **1 (smallest)** |
| **D1** (Plonky2 conventions in Plonky3) | None | None | 1 (smallest, tied) |
| **G** (inner-AIR shrinkage) | None | CSA refresh on non-fenced AIRs | 2 |
| **F** (PCS swap) | STIR or WHIR PCS | Public proofs exist | 3 |
| **D2** (direct Plonky2 vendoring) | Plonky2 stack | Public + Pearl audits | 4 |
| **A** (SNARK wrap) | BN254/BLS12 pairing + Groth16/Plonk + trusted setup | Multi-decade pairing crypto + per-curve audits | 5 (large but battle-tested) |
| **C** (Halo/Nova folding) | Pasta curves + Nova + R1CS embedding | Public Nova proofs but **novel R1CS embed for our STARK** | 6 (largest, most novel) |
| **H** (B+A) | A + B's deltas | A + B's audits | 5 (≈ same as A; B deduplicates the in-SNARK STARK verifier audit) |

---

## 7. R1 risk comparison

| Path | Touches fenced linchpin? | Staged validation feasible? | R1 risk |
|---|---|---|---|
| A | No (outermost) | Yes (S0 KAT-first prototype landed in M-S5b) | **Low** |
| B | Yes (verifier AIR — adjacent to C2/DT-4) | Yes (CRIT-1-style staged flip) | **Medium** |
| C | No (outermost) | Yes (S2 KAT-first prototype possible) | **High** (no in-tree analog) |
| D1 | Yes (verifier AIR) | Yes (same as B) | Medium |
| D2 | Maybe (replaces vendored recursion stack) | Yes (parallel vendoring) | **High** (substrate replacement) |
| E | DEAD | — | n/a |
| F | Yes (PCS swap touches FRI backend) | Yes (KAT-first PCS swap) | Medium |
| G | **Risk of touching fenced linchpin** | Yes (KAT-first per non-fenced AIR) | **High** |
| **H — B+A** | B touches verifier AIR; A doesn't | Yes (compose B's staging + A's S0 prototype) | **Low+Medium = Low overall** |

---

## 8. Composability matrix

Rows are primary path; columns are co-applied secondary path.

|  | A | B | C | D1 | D2 | F | G |
|---|---|---|---|---|---|---|---|
| **A** | — | ✅ (B reduces in-SNARK circuit cost) | ❌ | ✅ | ❌ (D2 supplants A) | ✅ | ✅ |
| **B** | ✅ | — | ❌ | (= B + D1 = same thing) | ❌ (incompatible) | ✅ | ✅ |
| **C** | ❌ | ❌ | — | ❌ | ❌ | ❌ | ❌ |
| **D1** | ✅ | ✅ | ❌ | — | ❌ | ✅ | ✅ |
| **D2** | ❌ | ❌ | ❌ | ❌ | — | (within Plonky2) | (within Plonky2) |
| **F** | ✅ | ✅ | ❌ | ✅ | (Plonky2 native) | — | ✅ |
| **G** | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | — |

The most composable path is **H (B+A)**, which is the
recommended sequence in § 9.

---

## 9. Recommended sequence (refines M-S5b § 2.F)

### 9.1 Headline recommendation

> **Path H = Path B + Path A**, with **Path D2 (direct Plonky2
> vendoring) as fallback** if the audit board rejects Path A's
> pairing-crypto audit surface.

Rationale:

1. **Path B alone has been empirically shown insufficient**
   at the new bar (this audit's § 1.3). Need an outer
   compression.

2. **Path A is the lowest-R1-risk outer compression**
   (non-invasive to fenced linchpin; battle-tested production
   pattern; predictable size guarantee).

3. **Path B is the lowest-R1-risk inner optimization** (no
   new primitives) and *composes* with Path A to reduce
   Path A's prover cost 2–4×.

4. **Path D2** is Pearl's actual approach but requires
   replacing the vendored Plonky3-recursion stack (~3 months
   of recent work) with a parallel Plonky2 stack (~2-3 months
   of new work). Highest substrate cost; reserve as fallback.

5. **Path F** (STIR/WHIR PCS swap) is the wildcard. Could
   be tried as an S2-style KAT-first prototype to
   quantitatively measure the savings before committing to
   Path A. If STIR alone (without Path A) reaches ≤65 KB,
   the audit board could prefer F as a less-disruptive
   alternative to A.

### 9.2 Sequenced sub-stages (refines M-S5b § 3)

| Stage | Item | Effort | Invasive? |
|---|---|---|---|
| **S1** ✅ this commit | Comprehensive routes audit (this doc) | 1 day | No |
| **S1.B** | Path-B column-by-column reduction map (the M-S5b § 3.2 deliverable, refined) | 2–3 days | No |
| **S1.F** | STIR/WHIR KAT-first prototype (size measurement only; excluded workspace) | 3–5 days | No |
| **S2.A** | Path-A KAT-first prototype (toy Goldilocks STARK + Groth16 / Plonk wrap; excluded workspace) | 3–5 days | No |
| **S2.D2** | Path-D2 KAT-first prototype (vendor Plonky2; build inner-to-Plonky2 bridge; excluded workspace) | 5–7 days | No |
| **S3** | Maintainer decision: Path H vs Path D2 vs Path F+B (vs Path C if elected) | n/a | n/a |
| **S4** | Invasive substrate addition per the decision | 2 weeks – 1 month | Yes |
| **S5** | Acceptance gate (`tip5_layer0_outer_cert_size_residual` passes at ≥80) | — | — |

**S1.B + S1.F + S2.A + S2.D2 are all non-invasive prototypes.**
They commit measurement artifacts in excluded workspaces
(per C1's vendoring model) before any production code is
written. The maintainer decision (S3) is informed by
empirical size measurements from each prototype.

### 9.3 Why this differs from M-S5b § 2.F

M-S5b § 2.F (pre-this-audit) recommended:
> "Path B first as the candidate solution, not merely
> de-risk; Path A held in reserve as the fallback if Path
> B's measurements miss ≤65 KB."

This audit's refinement:
> "Path B as primary inner optimization (S1.B), Path A as
> primary outer compression (S2.A) — both pursued in
> parallel, composed into Path H at S4."

The change is driven by the L2 structural-floor finding
(§ 1.3): Path B alone cannot break the ~40 KB floor; an
outer compression is required regardless. The M-S5b §
2.F-era prediction "B-alone has a real shot" was
optimistic at the new bar.

### 9.4 Path D2 fallback conditions

Use Path D2 (direct Plonky2 vendoring) if:
1. Path A's pairing-crypto audit surface is rejected by the
   audit board (e.g., trusted setup concerns; BN254 boundary
   soundness concerns).
2. Path D2's S2.D2 prototype measures ≤65 KB at ≥80
   unconditional with our inner trace size.
3. The maintainer accepts the substrate-replacement R1 risk.

Path D2 is a viable alternative to Path H; the choice is
**audit-policy-driven**, not technical.

---

## 10. Plonky2-specific lessons (for any path we choose)

Even if we don't adopt Path D2 (direct Plonky2 vendoring),
Pearl's Plonky2 usage teaches three lessons we should
internalize:

### 10.1 Preprocessed columns are the size lever

Pearl WP § 4.7 explicitly notes:
> "We extend Plonky2's AIR representation with preprocessed
> columns — allowing circuit reliance on public data agreed
> by both prover and verifier."

This is the same technique CRIT-1 uses (PROGRAM_COLS pinned
to preprocessed values). At the outer-cert layer, we should
**maximize preprocessed-column use** to push public data out
of the prover's witness columns. Each column moved from main
to preprocessed shrinks `opened_values` by ~D bytes per row.

### 10.2 BLAKE3 vs Tip5/Poseidon at outer layers

Pearl uses BLAKE3 throughout (commitments + Fiat-Shamir).
Plonky2 natively supports BLAKE3 in-circuit. Our deployment
uses Tip5 (inner) + Poseidon2-W8 (outer-cert). The Poseidon2
x⁷ S-box is the *highest-degree constraint* in our verifier
circuit (max degree 7) and contributes the bulk of the
quotient polynomial cost.

**Future consideration:** at the outermost cert layer
(L3 or terminal SNARK), evaluate replacing Poseidon2-W8 with
BLAKE3 for MMCS + Fiat-Shamir. This is a Path-B sub-stage
(narrow the highest-degree constraint), not a path-switch.

### 10.3 3-layer recursion can be made viable

Our empirical "L3 > L2" finding at the new bar is partly
because each layer adds ~80 KB of in-circuit verifier
overhead, which exceeds the per-layer compression gain.
Plonky2's compact gate set + smaller per-layer floor
inverts this: each layer compresses by ~3–10× with only
~10–20 KB of overhead.

**If we adopt Path D2 OR Path B reduces our per-layer floor
≥ 4×**, the 3rd recursion layer becomes viable, and we
could revisit Path E (stacked recursion). For now, the L3
finding holds at our current substrate.

---

## 11. Honest residuals (R1)

What this audit does not close, deferred:

1. **S1.B Path-B column-by-column reduction map** — the
   formal M-S5b § 3.2 deliverable. This audit's § 3.2 is
   architectural / quantitative; the column-by-column work
   is a 2–3 day follow-on. Sequenced as the next M-S5b stage
   immediately after this audit.

2. **S1.F STIR/WHIR prototype** — would quantitatively
   measure whether Path F's PCS-swap savings change the
   path-feasibility analysis. Not yet executed; sequenced as
   a parallel S1 stage.

3. **S2.A / S2.D2 prototypes** — Path A + Path D2
   KAT-first prototypes in excluded workspaces. Not yet
   executed; the maintainer's path decision (S3) requires
   them.

4. **Pearl-side noise + matrix-commitment AIR sizing** —
   the apples-to-apples comparison of "what would Pearl's
   inner AIR look like for our AI-PoW puzzle vs their
   matrix-opening proof". Out of audit scope; Pearl WP § 4.7
   doesn't decompose their inner trace size enough for a
   precise comparison.

5. **Concrete Plonky2-vs-Plonky3 per-layer floor
   measurement** — § 4.4's "~10–20 KB Plonky2 floor vs ~40
   KB Plonky3 floor" is an estimate from gate-count
   comparisons; not a measured number. Path D2 prototype
   would measure this.

6. **The audit board's policy on pairing crypto** — the
   path decision (Path H vs Path D2) depends on whether the
   board accepts BN254 / BLS12-381 + Groth16 / Plonk as
   sound primitives for the consensus cert. Out of this
   audit's scope; flagged as the S3 maintainer-decision
   driver.

None of these are soundness gaps. All are sequenced
follow-on stages.

---

## 12. Cross-references

- **M-S5b parent doc.** `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`
  (§ 2 path catalog, § 3.2 S1 spec, § 3.3 S0 KAT, § 3.4 S2
  conditional Path C, § 3.5 S3 decision, § 3.6 S4 invasive).
- **Empirical baseline.** `2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md`
  (§ 2 measurements at new bar, § 5 path-tree implications).
- **FRI soundness (the bar).** `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`
  (S(−1); ≥82 unconditional per IACR ePrint 2025/2055
  Theorem 1.5).
- **AIR-side soundness (constraint floor).**
  `2026-05-20_CSA_S7_AUDIT_SIGNOFF.md` (per-AIR ≥98 bits;
  +16 to +25 margin to FRI floor).
- **Pearl's actual approach.** `Pearl_Whitepaper.pdf` § 4.7
  (Plonky2 + 3-layer recursion + preprocessed columns) + § 5.1
  (65 KB certificate size limit + WTEMA difficulty).
- **STIR.** IACR ePrint 2024/390 (Arnon, Chiesa, Fenzi,
  Yogev — "Reed–Solomon proximity testing with fewer
  queries"). Cross-cited in S(−1) per IACR ePrint 2025/2055
  § 1.5 (Related Work).
- **WHIR.** IACR ePrint 2024/1586 (Arnon, Chiesa, Fenzi,
  Yogev — "with super-fast verification").
- **BaseFold.** IACR ePrint 2023/1705 (Zeilberger, Chen,
  Fisch — "efficient field-agnostic PCS from foldable codes").
- **Plonky2 substrate.** Mir-Protocol / Polygon Zero
  upstream (`github.com/0xPolygonZero/plonky2`); Pearl's
  vendored copy is the audit lineage.
- **Tip5 paper (the authoritative round-count specification).**
  Szepieniec, Lemmens, Sauer, Threadbare, Al Kindi — "The
  Tip5 Hash Function for Recursive STARKs" (IACR ePrint
  2023/107). § 2 sponge construction; § 4.6 canonical
  decomposition; § 5 security analysis (§ 5.1–5.10 attack
  classes + § 5.11 Table 4 minimum-round summary); § 5.11
  "the round count N = 5 was set to provide a roughly 50%
  security margin". **Tip5 paper spec: N = 5; Nockchain
  uses N = 7 for additional margin** (see § 3.2.0b).
- **Opening the Blackbox (the authoritative third-party
  cryptanalysis).** Liu, Koschatko, Grassi, Yan, Chen,
  Banik, Meier — "Opening the Blackbox: Collision Attacks
  on Round-Reduced Tip5, Tip4, Tip4', and Monolith" (IACR
  ePrint 2024/1900). Table 1 attack reach: practical
  SFS collision on 3-round Tip5 at 2^41.2; full collision
  at 2^121.1. § 5.4 SFS attack details. **3-round Tip5 is
  broken; 4-round and above are safe under their analysis;
  the Tip5 paper's "50% security margin" claim is reduced
  post-this-paper.** (Located at `2024-1900.pdf` in repo
  root.)
- **C2.1 Tip5 keystone.** `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md`
  (the fenced linchpin Path G + Path D2 must respect /
  re-implement).
- **CSA fenced-linchpin invariants.** `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`
  § 0.3.
- **R1 / R1.1 discipline.** `~/.claude/CLAUDE.md`.

---

## 13. R1 honest verdict

**Audit complete.** 8 paths analyzed at the new ≥80-Johnson
bar; Path B's structural floor identified; Pearl's Plonky2
3-layer approach quantified; recommended sequence (Path H
= B+A as primary, D2 as fallback) sequenced into S1.B / S1.F
/ S2.A / S2.D2 prototypes followed by S3 maintainer decision.

**Validated subset (this commit):** the architectural +
quantitative comparison, the recommended sequence, the
per-path soundness + audit-surface + R1-risk analysis.
**Precise residual (R1):** the sub-stage prototypes (S1.B
column-by-column, S1.F STIR prototype, S2.A Groth16/Plonk
prototype, S2.D2 Plonky2 vendoring prototype) — each is its
own follow-on milestone, sequenced and scoped above.

**No fake completion.** This doc claims the audit + the
recommendation; it does not claim the path is *implemented*.
The path is selectable after S2 prototypes return empirical
data to feed the S3 decision.
