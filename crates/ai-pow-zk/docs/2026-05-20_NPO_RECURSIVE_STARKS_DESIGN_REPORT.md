# NPO recursive STARKs in Nockchain — design report + alternatives

**Date:** 2026-05-20
**Author:** Claude (audit-grade synthesis)
**Status:** Reference document. Snapshot of the architecture as
of commit `21cf77c` (post-Tip5-throughout L2-over-L1 landing).

## 0. Why this document exists

Nockchain's ai-pow-zk proof system uses **NPO recursive STARKs**:
multi-tier STARK proofs glued together by non-primitive operations
(NPOs) and Fiat-Shamir transcripts, with the outer cert proving
that the inner cert verifies. This document explains how the
machinery works, why we picked the components we picked,
where the soundness lives, and what the alternatives are.

Audience: external cryptographic auditors, second-opinion
reviewers, future maintainers. Aimed at someone who knows STARKs
abstractly and wants the specific Plonky3-recursion realization
without reading the whole vendored crate.

## 1. Background — STARKs, the recursion need, and what "NPO" means

### 1.1 STARK in one paragraph

A STARK takes a computation expressed as an **AIR** (Algebraic
Intermediate Representation — a set of low-degree polynomial
constraints over a trace matrix) and produces a non-interactive
proof that the trace satisfies all constraints. The proof
consists of (i) a **Merkle commitment** to the trace + the AIR's
**quotient polynomial**, and (ii) a **FRI** (Fast Reed-Solomon
Interactive Oracle Proof) low-degree test on randomly opened
columns. Soundness comes from (a) Schwartz-Zippel over a large
field (degree·n / |F|), (b) the FRI proximity test's distance
from any non-RS-codeword (Johnson-radius bound, paper IACR
ePrint 2025/2055 Theorem 1.5), and (c) the Fiat-Shamir
transcript binding all challenges to prior commitments.

Plonky3 (the Polygon-stewarded reference implementation we
vendor) is a STARK library: Goldilocks / BabyBear / KoalaBear /
Mersenne-31 base fields; FRI PCS; configurable hashes
(Poseidon1/Poseidon2/Tip5) for Merkle MMCS + Fiat-Shamir.

### 1.2 The recursion need

A STARK proof at production parameters (our Tier B: ~548 KB; pre-
Tier-B baseline: ~1 MB) is much larger than is convenient for a
block header. Block consensus would prefer ≤65 KB.

**Recursion** is the standard answer: build an **outer STARK**
whose AIR is "I verified the inner STARK". The outer's proof is
itself a STARK, but proves only the validity-of-the-inner-proof
instead of the original computation. If the verifier circuit
shrinks the computation enough (in trace rows / columns / bits),
the outer cert can be much smaller than the inner.

This produces a **recursion chain**:

```
inner computation (e.g. ai-pow-zk Layer-0)
    ↓ STARK
inner STARK proof (Layer-0)
    ↓ "verify the above" AIR
L1 outer-cert proof
    ↓ "verify the above" AIR
L2 outer-cert proof   ← stop here in production
    ↓ optionally
L_n outer-cert proof
```

Soundness: each layer's verifier circuit is a real AIR, so if
the outer cert verifies, the inner did — **transitively** down
to the bottom. The soundness chain is `MIN(layer_n bits)` where
each layer's bits are determined by its FRI parameters
(`bits ≈ log_blowup · num_queries + commit_pow + query_pow`,
paper IACR ePrint 2025/2055 Theorem 1.5).

### 1.3 What "NPO" means in this codebase

"NPO" = **Non-Primitive Operation**. Plonky3-recursion treats
the verifier circuit as a multi-table batch STARK:

- **Primitive tables** (`Const`, `Public`, `Alu`): the basic
  arithmetic + boolean + memory ops the recursion-verifier
  circuit needs.
- **Non-primitive tables** (NPOs): operations whose AIR is too
  expensive to inline into the main circuit's row, so they get
  their own AIR table with the main circuit consuming/producing
  values via a **LogUp lookup bus**.

The NPOs we use:

- `tip5_perm` — Tip5 permutation: a single row of `Tip5PermLookupAir`
  per perm invocation (one Tip5 permute = one NPO table row).
  Source: `crates/plonky3-recursion/circuit-prover/src/batch_stark_prover/tip5.rs`.
- `poseidon1_perm`, `poseidon2_perm_width_8` — analogous Poseidon
  permutations. Retained in upstream Plonky3 for non-nockchain
  consumers; **never used in nockchain's trust surface** per the
  user hard rule (see [no_poseidon2_anywhere] memory).
- `recompose` — converts a packed sequence of base-field elements
  into a circuit Challenge (Goldilocks → quadratic extension)
  and vice versa. Necessary because the in-circuit hash inputs
  live in the base field but Fiat-Shamir challenges live in the
  extension.

Without NPOs, every Tip5 hash inside the verifier circuit would
inflate the main circuit's degree (7-round perm = degree-256
arithmetic per S-box; 4096 hashes per L1 cert = ~10⁶ rows of
quintic-degree constraint each). With NPOs, the perm gets a
**single dedicated AIR table** that handles its rows efficiently
(LogUp Bus, max-deg 4) and the main circuit consumes its outputs
through a low-degree bus channel.

The "NPO recursive STARK" name = **a recursion-verifier circuit
that uses NPO tables for the expensive in-circuit hash work**.

## 2. The architecture in our stack

### 2.1 Layer-by-layer

**Inner Tip5-L0 STARK** (`crates/ai-pow-zk/src/circuit.rs`):

- The bottom STARK. Proves the ai-pow-zk computation (Llama-3.1
  forward pass via INT7 quantized GEMM, BLAKE3 chunk-Merkle for
  weight binding, etc.).
- Single AIR, no NPOs (the inner STARK's AIR contains all the
  computation; Tip5 is used here only as the MMCS + Fiat-Shamir
  hash, NOT as an in-circuit NPO).
- FRI config (`CircuitConfig::PROD`): `log_blowup=3, num_queries=30`,
  base-field Goldilocks, extension Challenge =
  `BinomialExtensionField<Goldilocks, 2>`. ≥90 bits unconditional
  Johnson.
- Tip5 sponge: W=16, R=10, capacity=6, digest=5; 7 rounds.

**L1 outer-cert** (`crates/plonky3-recursion/circuit-prover/src/config.rs`
`goldilocks_tip5_60bit`):

- A **multi-table batch STARK** whose AIRs include:
  1. The Plonky3-recursion verifier-circuit AIR — which embeds
     `verify_p3_uni_proof_circuit` against the inner Tip5-L0 STARK.
     This AIR's rows encode FRI fold-chain verification, Merkle
     MMCS opening verification, Fiat-Shamir transcript replay.
  2. The `Tip5PermLookupAir` NPO table — one row per Tip5 perm
     the verifier circuit needs to evaluate (in-circuit
     re-hashing for MMCS verification).
  3. The `Recompose` NPO table — one row per Goldilocks→Challenge
     decompose / Challenge→Goldilocks compose.
- FRI config (Tier B, landed `63a7f7a`): `log_blowup=4,
  num_queries=20, pow=1+1, d=5`. 82 bits unconditional Johnson.
- Substrate: Tip5-throughout (`goldilocks_tip5_params` test-utils
  parallel to the Poseidon2 `goldilocks_params`).
- Measured size: **547.88 KB** (commit `21cf77c`).

**L2 outer-cert** (same `goldilocks_tip5_60bit` config, applied
to the L1 cert):

- Another multi-table batch STARK whose AIRs are now `verify_p3_
  batch_proof_circuit` against the L1 cert (which is itself a
  batch STARK, not a single uni-stark). Includes its own
  `Tip5PermLookupAir` + `Recompose` NPO tables.
- Same Tier B FRI config; 82 bits.
- Measured size: **646.76 KB** (`21cf77c`). **Note: L2 > L1.** The
  Tip5 NPO overhead at the L2 layer exceeds the savings from
  collapsing the L1 STARK — see §3.5 below.

### 2.2 The verifier-circuit + NPO pattern in detail

The L1 outer-cert's prover takes the inner Tip5-L0 STARK proof
as **private input** and a `CircuitBuilder<Challenge>` template
(declarative). It runs:

```
CircuitBuilder::new()
  .enable_tip5_perm(Tip5Goldilocks, LiftTip5)    // NPO 1
  .enable_recompose(generate_recompose_trace);    // NPO 2

verify_p3_uni_proof_circuit(
  inner_config,         // the Tip5-L0 StarkConfig
  inner_air,            // the L0 AIR (FibonacciAir for tests; ai-pow-zk in prod)
  &mut circuit_builder, // the running circuit DSL
  inner_proof_targets,  // L0 proof, packed as Challenge-typed witnesses
  inner_pi_targets,
  ...,
  Tip5Config::GOLDILOCKS_W16,
);
```

The DSL grows a circuit that:
1. Re-hashes the inner trace + quotient commitments via Tip5
   sponge (Merkle root reconstruction); each hash invocation
   = one `tip5_perm` NPO call (one NPO table row).
2. Recomputes the Fiat-Shamir transcript by absorbing the inner
   commitments into a Tip5 challenger; each absorb = one Tip5
   permutation = one NPO call.
3. Computes the FRI fold-chain queries against the inner FRI
   commitments; each opening verification = several Tip5 hashes
   = several NPO calls.
4. Evaluates the inner AIR's constraints at the queried points,
   asserts they equal zero modulo Z_H. The arithmetic happens
   in the **main circuit**, which has its own `Const/Public/Alu`
   tables.

The result: a `Circuit<Challenge>` that, when run with the
inner proof as private inputs, produces a trace via the
`BatchStarkProver`. The trace contains:

- Main-circuit rows (Const, Public, Alu, plus the L1's own
  `verify_p3_uni_proof_circuit` constraints).
- `tip5_perm` NPO rows (one per perm invocation, ~4000+ rows
  for a real L1).
- `recompose` NPO rows (one per Challenge↔base composition).

Each NPO table is a fully-independent AIR with its own trace
matrix, constraints, and FRI commitments. The connection back
to the main circuit happens via the **LogUp witness bus**: every
NPO row sends `(witness_id, value, multiplicity)` tuples;
every main-circuit consumer receives the matching tuple. The
LogUp gadget enforces global multiset balance at zero, which
binds the NPO outputs to the main circuit's consumption.

### 2.3 The FriRecursionBackend dispatch

In a generic recursion stack (multiple proof systems / multiple
hash families), the choice of NPO trio per layer is config-driven.
`FriRecursionBackend::non_primitive_{preprocessors, provers, air_builders}`
at `crates/plonky3-recursion/recursion/src/backend/fri.rs:444-555` is the
dispatch point. Pre-Stage-3 (commit `6c67e7f`) it routed only
Poseidon1/Poseidon2. Post-Stage-3, it routes Tip5 as the
production path:

```rust
let perm_prep = if self.0.challenger_perm_config.as_tip5().is_some() {
    tip5_preprocessor::<Val<SC>>()
} else if self.0.challenger_perm_config.as_poseidon1().is_some() {
    poseidon1_preprocessor::<Val<SC>>()
} else {
    poseidon2_preprocessor::<Val<SC>>()
};
```

When the outer config carries a `Tip5Config` challenger
(post-Tier-B `goldilocks_tip5_60bit`), all three dispatch
methods return the Tip5 NPO trio. The Poseidon1/Poseidon2
branches remain for upstream Plonky3 consumers but are
**never executed in nockchain's trust surface**.

### 2.4 Soundness chain

Per IACR ePrint 2025/2055 Theorem 1.5, each layer's FRI
soundness is `lb · nq + cpow + qpow` bits unconditional inside
the Johnson radius J(δ) = 1 − √ρ. Our deployed chain:

| Layer | FRI config | Unconditional bits | J(δ) |
|---|---|--:|--:|
| Inner Tip5-L0 PROD | lb=3 nq=30 pow=0+0 | 90 | 0.646 |
| L1 outer (Tier B) | lb=4 nq=20 pow=1+1 | 82 | 0.75 |
| L2 outer (Tier B) | same | 82 | 0.75 |

`MIN(90, 82, 82) = 82 bits` ≥ 80 floor.

Additional soundness:
- **Schwartz-Zippel per AIR**: `(d_max + 1) · n_rows / |F_chal|`
  ≥ 98 bits at production parameters.
- **Per-LogUp-bus**: `3 · k_b / |F_chal|` ≥ 98 bits (k_b = number
  of bus tuples).
- **Fiat-Shamir collision resistance**: bound by Tip5 sponge
  capacity = min(C/2, output) = min(6·32/2, 5·32) = 96 bits per
  permutation in the Goldilocks Tip5 W=16 C=6 setup.

The chain MIN is **dominated by FRI** (82 bits); AIR + LogUp
have ≥16-bit margin to FRI. See
[`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md)
for the per-layer derivation.

### 2.5 What we measured (Stages 5 of the 2026-05-20 work)

| Layer | Tier B size | Notes |
|---|--:|---|
| Inner Tip5-L0 (ai-pow-zk PROD) | (varies; see CSA) | not measured in this report |
| L1 outer-cert (Tip5-throughout) | **547.88 KB** | matches §4.2 prediction in [recursive-proof-size investigation](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md) |
| L2 outer-cert (Tip5-throughout) | **646.76 KB** | L2/L1 = 1.18× — L2 *inflates* L1, not shrinks |

This is the critical empirical surprise — see §3.5.

## 3. Why these specific choices

### 3.1 Why Goldilocks + degree-2 extension

- Goldilocks (`p = 2⁶⁴ − 2³² + 1`) has high two-adicity (2^32),
  needed for FRI's LDE blowups (we use 16× at Tier B).
- The Goldilocks field arithmetic is `montgomery-free` for
  Tip5 (`belt::{bmul, badd}` per `nockchain-math`).
- The quadratic extension `BinomialExtensionField<Goldilocks, 2>`
  gives ~128-bit cardinality for Fiat-Shamir challenges and
  Schwartz-Zippel bounds, while keeping per-element storage at
  16 bytes.

Alternatives considered:
- **BabyBear** (`p = 2³¹ − 2²⁷ + 1`): smaller field; tighter
  FRI proofs but smaller |F_chal| ⇒ Schwartz-Zippel margin
  shrinks; needs degree-4+ extension. Used in some Polygon
  experiments.
- **Mersenne-31** (`p = 2³¹ − 1`): same size issue; chosen
  by Plonky3 in some configurations for SIMD speedups.
- **KoalaBear** (`p = 2³¹ − 2²⁴ + 1`): emerging alternative;
  not yet broadly deployed.

Goldilocks is the conservative, well-analyzed choice.

### 3.2 Why Tip5 specifically (and only)

The hash function used for the Merkle MMCS + Fiat-Shamir
transcript at every layer **must** be in-circuit-efficient
(few rows / low degree per evaluation), because the outer
verifier circuit must re-hash the inner proof's commitments.
Candidates:

| Hash | In-circuit rows | Degree | Notes |
|---|--:|--:|---|
| **Tip5** (paper IACR 2023/107; Nockchain 7-round) | ~7 rows / perm | 4 (with LogUp arithmetization) | Native-field arithmetic over Goldilocks; deployed |
| Poseidon1 | ~1 row / perm | 5 | Older; degree 5 means bigger LDE |
| Poseidon2 | ~1 row / perm | 5 | Successor to Poseidon1; we explicitly removed it (see § 3.4) |
| Rescue-Prime | ~14 rows / perm | 5 | x^α + inverse arithmetic; legacy |
| Anemoi | ~5 rows / perm | 5 | Sponge-friendly; less-deployed |
| Reinforced Concrete | ~3 rows / perm | 7 | Native Goldilocks; cryptanalysis early |
| Monolith | ~3 rows / perm | varies | New (2024); some lookup-based |

Nockchain chose Tip5 because:
1. **One hash family throughout** (analogous to Pearl's BLAKE3-
   throughout choice for their proof system). Dual-hash
   architectures inflate the trust surface and complicate
   audit.
2. **Native Goldilocks arithmetic** (`nockchain_math::tip5::permute`):
   no extension arithmetic needed in the inner trace; matches
   the on-chain hash.
3. **Sponge capacity** 6 elements = 192-bit collision resistance,
   higher than the FRI floor.
4. **Paper-faithful AIR** (KAT-anchored to the `nockchain_math`
   oracle per [c2_tip5_circuit_air] memory). The lookup-free
   arithmetization via per-byte cube + canonical-guard makes the
   permutation a single self-contained AIR provable by plain
   `p3_uni_stark` — independently auditable.

### 3.3 Why a multi-tier (L1/L2) chain

The historical assumption: each recursion layer SHRINKS the
proof, so stacking layers approaches a small final cert.

**Reality (per Stage 5 measurement):** in Tip5-throughout
substrate, L2 > L1 (1.18×). The Tip5 NPO trace overhead at
every recursion layer exceeds the inner-STARK collapse savings.
The recursion chain is **size-monotone-non-decreasing**.

So why have an L2 at all? **Two reasons:**

1. **Fiat-Shamir transcript closure**: the L2 cert is a
   single batch STARK with a known structure, easier for a
   verifier (or a future Plonky2-style SNARK wrap) to consume
   than the L1's longer FRI fold chain.
2. **Architectural insurance**: if a future cert layout
   (smaller Tip5 AIR, Path B verifier-AIR slim, Tier C in-
   substrate aggression) DOES make L_{n+1} < L_n, the multi-
   tier chain machinery is already validated and stays in
   place. Removing L2 would forfeit that.

In production, **L1 is the cert** that goes on the wire (it's
smaller). L2 is plumbing that proves the inner STARK is bound
to the canonical state, useful for cross-layer consistency
arguments and as a target for a future SNARK wrap (Path A).

### 3.4 Why we removed Poseidon2 entirely

Earlier sessions had a **dual-hash** architecture: Tip5 for the
inner L0 STARK (ai-pow-zk side; deployed before recursion was
wired) + Poseidon2 for the recursion outer-cert (because
Plonky3-recursion shipped with Poseidon2-Goldilocks-W8 as
default). This created:

- **Two distinct hash families in the trust surface**: Tip5
  cryptanalysis (IACR 2024/1900 "Opening the Blackbox" paper)
  AND Poseidon2 cryptanalysis (separate body of work) both
  must be tracked + audited.
- **Complex verifier circuits**: the L1 verifier circuit had
  to enable BOTH `tip5_perm` (for the inner) AND
  `poseidon2_perm_width_8` (for the outer MMCS) NPOs;
  `inner_npo_provers()` reflected this.
- **Audit burden**: an external auditor reviews two hash
  primitives + two implementation paths instead of one.

In 2026-05-20 (P0-P7 + the Stage-3 wiring), we removed
Poseidon2 from nockchain's trust surface entirely:
- `goldilocks_tip5_60bit()` is now Tip5-throughout.
- `FriRecursionBackend` dispatches to Tip5 NPOs when the config
  carries `Tip5Config`.
- The Poseidon1/Poseidon2 dispatch branches remain in
  `FriRecursionBackend` for upstream Plonky3 consumers (this is
  a generic library crate, not a nockchain-private fork) but
  are **never executed in our production path**.

Per the hard rule [no_poseidon2_anywhere]: Poseidon2 is
**never** in any nockchain artifact's trust surface — not in
production, not in tests, not in measurement infrastructure,
not in docs. The audit story is now: Tip5 only.

### 3.5 Why L2 INFLATES L1 (the counterintuitive finding)

Stage 5 measured: L1 = 547.88 KB, L2 = 646.76 KB at Tier B.
Compare the legacy Poseidon2 baseline: L1 = ~961 KB,
L2 = ~618 KB, ratio 0.64× (L2 < L1, the expected pattern).

Why does Tip5-throughout invert this?

1. **Tip5 NPO is wider/heavier than Poseidon2-W8 NPO**:
   - Tip5: W=16, 7 rounds, single AIR row per permutation but
     886+ columns wide (per [c2_tip5_circuit_air] lookup-table
     AIR L1/L2).
   - Poseidon2-W8: W=8, fewer rounds, ~1 row per permutation,
     much narrower trace.
   Per recursion layer, the L_{n+1} cert must in-circuit-verify
   L_n's Tip5 hashes — which means generating L_n's count of
   Tip5 NPO rows in L_{n+1}'s NPO table. Tip5's wider rows
   inflate L_{n+1} more than the inner-STARK collapse saves.

2. **Tip5 challenger absorbs more bytes per perm** (digest=5;
   Poseidon2-W8's digest is 4). Slightly more Fiat-Shamir work
   in the verifier circuit.

3. **In-circuit Tip5 MMCS verification has more rows per
   commitment**: every Merkle authentication path needs Tip5
   compress operations; Tip5's W=16 R=10 capacity-6 geometry
   produces wider per-step traces than Poseidon2-W8 (W=8 R=4
   C=4).

The cumulative effect: the L2 verifier circuit's NPO tables
+ Recompose + main-circuit overhead exceed L1's bytes.

This is a **real architectural finding**, not a tuning artifact.
It means:

- **Plain multi-tier recursion is not the route to ≤65 KB** in
  Tip5-throughout. L3, L4, … all get bigger, not smaller.
- **Path A (outermost STARK-to-SNARK wrap)** is the realistic
  way to shrink below ~470 KB (the in-substrate Pareto floor
  measured in [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md)).

## 4. Alternatives we considered (or could consider)

### 4.1 Different hash families (in-substrate)

**Tip5 with fewer rounds**: Tip5 paper IACR 2023/107 specifies
**5 rounds**; Nockchain deploys **7 rounds** as a security margin
(per IACR 2024/1900 "Opening the Blackbox" cryptanalysis,
practical attacks exist at 3 rounds; 5-round margin is narrow).
Dropping to 5 rounds would save ~5-8 KB at L1; we don't because
the cryptanalysis margin matters more than 1% size.

**Different sponge geometry**: Tip5 currently W=16 R=10 D=5.
Variants:
- D=4 (Nockchain-local): saves ~5%; paper-divergent; no
  published cryptanalysis at D=4 specifically. Documented in
  `2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md` § 4.4 as
  Tier C (the Pareto-aggressive floor at ~470 KB L1).
- W=24 R=16 D=8 (hypothetical): higher capacity but more rows
  per perm. Untested.

**Switch to Poseidon2**: forbidden per [no_poseidon2_anywhere].

**Switch to Reinforced Concrete or Monolith**: both are Goldilocks-
friendly recent designs. Less cryptanalysis than Tip5. Reinforced
Concrete in particular uses degree-7 arithmetic which inflates
LDE further. Out of scope for the Tip5-throughout decision.

### 4.2 Different recursion strategies

#### A) STARK-to-STARK recursion (what we do today)

Inner STARK is verified by an outer STARK; same substrate
throughout. Each layer's verifier circuit has primitive +
NPO tables, all proved by FRI.

- **Pro**: single trust surface (Tip5 only); no
  curve-based primitives; post-quantum (FRI / Tip5 only,
  no pairings).
- **Pro**: prover is fully native; same code path at every
  layer; mechanical to debug.
- **Con**: each layer adds bytes (per our Stage 5 finding for
  Tip5). Doesn't compress.
- **Con**: large per-layer proofs.

This is what's deployed.

#### B) STARK-to-SNARK wrap (Plonky2 / Boojum / Lurk style)

Inner cert is a STARK; the outer/terminal cert is a SNARK
(e.g., Groth16, Plonk, Halo) that proves the STARK verifies.

- **Pro**: SNARK proofs are ~32 bytes (Groth16) or ~few KB
  (Plonk/Halo), well under the 65 KB target.
- **Pro**: the SNARK can be batched across many block headers.
- **Con**: introduces a SECOND proof system into the trust
  surface (the SNARK's curve + arithmetic).
- **Con**: SNARKs typically rely on pairing-friendly elliptic
  curves (BN254 / BLS12-381 / Pasta), which are
  **NOT post-quantum**.
- **Con**: trusted setup (Groth16) or universal trusted setup
  (Plonk).

Pearl (a different proof system) uses Plonky2 in a STARK-to-
SNARK wrap to achieve their cert size. To go this route,
nockchain would need to vendor a Plonky2 / Boojum / Plonk-style
proof system + an in-SNARK STARK verifier.

**This is the recommended Path A in [`2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`](2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md)
to reach ≤65 KB.**

#### C) Accumulation schemes (Halo / Halo2)

Instead of verifying the inner cert outright, accumulate it
into an **accumulator** whose verification is deferred to the
final layer. The accumulator can be folded across many
recursions cheaply.

- **Pro**: amortizes verifier cost across many recursions;
  ideal for IVC (Incrementally Verifiable Computation).
- **Pro**: no trusted setup (Halo / Halo-Infinite use
  Pasta curves with discrete-log-only assumptions).
- **Con**: still curve-based (Pallas/Vesta), NOT post-quantum.
- **Con**: substantial implementation work.

Pearl uses a Plonky2-style approach which is closer to (B) above.

#### D) Folding schemes (Nova / SuperNova / HyperNova / ProtoStar)

Generalize accumulation: at each step, fold two relaxed
R1CS / Plonkish instances into one. The folded instance is
then proven only at the final step.

- **Pro**: per-step cost is **just folding** (~1k constraints);
  proof grows logarithmically.
- **Pro**: well-suited for long iteration counts (e.g., AI PoW
  with many forward-pass steps).
- **Con**: requires curve-based commitments (Pedersen on Pasta);
  NOT post-quantum.
- **Con**: ai-pow-zk's structure (single forward pass, not many
  steps) doesn't benefit from folding's IVC angle.

Folding is the natural choice for stream-style computations;
not a clean fit for ai-pow-zk's structure.

#### E) STIR / WHIR PCS (FRI replacements)

STIR (Arnon et al., 2024) and WHIR (Bisaccia et al., 2024) are
polynomial commitment schemes that improve on FRI:
- STIR: ~half the queries for the same soundness ⇒ ~50% proof
  reduction.
- WHIR: similar; supports more flexible code rates.

- **Pro**: drop-in replacement for FRI at the PCS layer; same
  general STARK substrate.
- **Pro**: shrinks every layer by ~50% (would take L1 to ~275 KB,
  L2 to ~324 KB).
- **Con**: new substrate; soundness analysis less mature than
  FRI's; would require porting Plonky3-recursion's FRI backend
  to STIR / WHIR.
- **Con**: still doesn't reach ≤65 KB without Path A.

Could be a future optimization layered on top of the current
architecture. Effort: high (substrate addition); soundness
audit: substantial.

#### F) Brakedown / Orion PCS (alternative non-FRI)

Linear-time prover; trade prover time for proof size. Not used
in our stack.

#### G) Modular recursion (recursion-aware AIR design)

Architectural alternative: design the NPO tables and main
circuit to be small enough that recursion DOES shrink the
proof. This would mean:
- Smaller in-circuit Tip5 (lookup-table approach with fewer
  rows per perm).
- Slimmer verifier circuit (Path B per the routes audit).
- Maybe even custom-gate Plonk-style primitives.

This is the spirit of **Path B**: shrink the verifier circuit
so recursion compresses again. Per the routes audit + task #17
this is the next-highest-value lever after Tier B (which
shipped) and Path A (deferred).

### 4.3 Different AIR architecture alternatives

Within the STARK paradigm, we could change the AIR shape:

| Choice | What | Tradeoff |
|---|---|---|
| **Multi-table batch STARK with LogUp** (current) | NPOs in separate tables; LogUp bus; main circuit consumes via lookups | Native to Plonky3; cleanest soundness story; what we have. |
| **Single-table STARK with embedded perms** | All constraints inlined into one AIR | No cross-table proof; bigger trace; harder soundness reasoning; doesn't fit Tip5's 7-round / degree-256 per-byte arithmetic. |
| **Plonkish / custom gates** | Per-gate constraints with flexible arity | More expressive; better for non-uniform computation; harder to reason about FRI soundness. |
| **AIRs with cross-AIR challenges** (CC-AIR) | Share challenges across non-primitive AIRs | Tighter coupling; modest size savings; complex. |

The multi-table-with-LogUp design we have is standard for
Plonky3 + post-Habock-LogUp. The alternatives don't obviously
beat it for our use case.

## 5. Comparison matrix

| Approach | Reaches ≤65 KB? | Post-quantum? | Trusted setup? | Substrate change? | Effort |
|---|:--:|:--:|:--:|:--:|:--:|
| Current Tip5-throughout multi-tier (deployed) | No (~548 KB L1) | Yes | No | — | shipped |
| In-substrate stacking (Tier C, ~470 KB) | No | Yes | No | parameter | low |
| More recursion layers (L3+) | No (gets worse) | Yes | No | none | low + wasteful |
| STIR / WHIR PCS swap | No (~270 KB L1) | Yes | No | PCS replace | high |
| Path B verifier-AIR slim | Maybe (~200-400 KB L1) | Yes | No | AIR re-design | medium-high |
| Path A: STARK-to-SNARK wrap (Plonky2/Boojum/Plonk) | **Yes** (~few KB) | NO | typically yes | new proof system | very high |
| Accumulation (Halo) | Yes (with curves) | NO | No | new proof system | very high |
| Folding (Nova) | Yes (long-chain only) | NO | No | new proof system | very high |

**Bottom line:** the only known path to ≤65 KB while keeping
the trust surface post-quantum + no-trusted-setup is **Path B
verifier-AIR slim** to shrink each recursion layer enough that
multi-tier stacking actually compresses. Below ~200 KB requires
Path A (with the curve-based downside).

## 6. The recommendation hierarchy (as of 2026-05-20)

1. **Tier B is shipped** (commit `63a7f7a`). L1 = 547.88 KB,
   82 bits, paper-faithful Tip5 + digest=5. Stable production
   baseline.
2. **Tier B + cap=3 + mla=3 lfp=2** (~490 KB L1): trivial
   in-substrate stacking; +1 commit; recommended as the next
   easy win once auditor signs off on the soundness-neutral
   levers.
3. **Tier C** (~470 KB L1 at 80b): requires digest=5→4 paper-
   divergence; needs auditor approval.
4. **Path B verifier-AIR slim** (task #17): the highest-impact
   non-FRI lever. Reduces L1 + cascades to L2 and beyond. Could
   reach 200-400 KB chain.
5. **Path A SNARK wrap** (for ≤65 KB): final architectural move.
   Required for the ≤65 KB target. Brings curve-based primitives
   into the trust surface (NOT post-quantum); requires explicit
   sign-off.

## 7. Open questions / future work

1. **Prover wall + RAM measurements at Tier B**: not measured.
   16× LDE inflates prover cost ~4× over the pre-Tier-B
   baseline; needs operational validation before full deployment.
2. **Tier C empirical L2** (not just L1 ~470 KB): predicted
   ~550-600 KB by analogy with Stage 5; would confirm the
   monotone-non-decreasing observation at the Pareto-aggressive
   point.
3. **STIR PCS feasibility study**: how invasive is the swap
   in Plonky3-recursion? Would the ~50% L1 + L2 reduction
   justify the substrate addition + audit cost?
4. **Path B column-by-column reduction map** (task #17): which
   verifier-circuit columns can be removed without soundness
   loss? Could compound.
5. **Path A architectural decision**: which SNARK system
   (Plonky2 / Boojum / Plonk-with-Halo-recursion)? Trusted-
   setup vs universal-setup trade-off?
6. **D=4 / D=5 Tip5 dispatch**: deferred residual; no current
   consumer but would be needed if we ever wanted Tip5 inside a
   quintic-extension circuit.
7. **L3 measurement** at Tier B: confirm L3 > L2 (predicted by
   the L2/L1 = 1.18× extrapolation; would close the
   "more layers helps" question definitively).

## 8. Authoritative references

- **Tip5 paper**: IACR ePrint 2023/107 (the formal spec). N=5
  rounds in the paper; Nockchain deploys 7-round for security
  margin.
- **Tip5 cryptanalysis**: IACR ePrint 2024/1900 "Opening the
  Blackbox" — practical 3-round attacks; the 5-round (paper)
  and 7-round (Nockchain) margins are derived from this.
- **FRI soundness**: IACR ePrint 2025/2055 Theorem 1.5 (Johnson
  radius proven bound). Replaces the pre-2025 "conjectured
  bound" framing.
- **Plonky3-recursion in-tree at 524665d**: vendored, rev-
  aligned to ai-pow-zk's 6de5cba per [c1_recursion_substrate]
  memory.
- **Per-layer soundness derivations**: [`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md).
- **Recursive proof size investigation + measurements**:
  [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md).
- **Routes audit (Path A/B/C/D)**: [`2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`](2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md).
- **Poseidon2 removal spec + execution addendum**: [`2026-05-20_POSEIDON2_REMOVAL_SPEC.md`](2026-05-20_POSEIDON2_REMOVAL_SPEC.md).
- **Tip5 NPO recursion backend design**: [`2026-05-20_TIP5_NPO_RECURSION_BACKEND_DESIGN.md`](2026-05-20_TIP5_NPO_RECURSION_BACKEND_DESIGN.md).
- **C4 audit-readiness package**: [`2026-05-19_C4_AUDIT_READINESS.md`](2026-05-19_C4_AUDIT_READINESS.md).
- **Cryptographic assumptions (always-current)**: `crates/ai-pow-zk/README.md` § "Cryptographic assumptions".

## 9. Summary

NPO recursive STARKs in nockchain = **multi-table batch STARK
verifier circuits chained via Fiat-Shamir transcripts**, with
Tip5 as the **only** hash family in the trust surface (per the
[no_poseidon2_anywhere] hard rule). The architecture is
post-quantum, no-trusted-setup, fully native Goldilocks. The
production chain (inner Tip5-L0 → L1 outer → L2 outer at Tier B)
has ≥80-bit unconditional Johnson soundness at every link and
measures L1 = 547.88 KB, L2 = 646.76 KB.

The empirical surprise from Stage 5 (L2 INFLATES L1 in
Tip5-throughout, opposite the legacy Poseidon2 pattern) means
**more recursion ≠ smaller cert**. The path to ≤65 KB is no
longer "stack more layers" — it's either Path B (verifier-AIR
slim, post-quantum-preserving) or Path A (SNARK wrap, with the
curve-based trust-surface cost). The next architectural decision
is which of those to invest in; this document is the reference
for that conversation.
