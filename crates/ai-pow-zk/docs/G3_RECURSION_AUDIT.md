> _Created **2026-05-17** · last updated **2026-05-17** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# G3 Recursion — Security Audit: my design spec vs the Plonky3-recursion reference

> **Status (2026-05-17): AUDIT.** Reviews `G3_RECURSION_AGGREGATION.md`
> (my G3 design) against the reference implementation at
> `Plonky3-recursion/` (origin `github.com/Plonky3/Plonky3-recursion`,
> a *fixed recursive verifier* for Plonky3 uni-/batch-STARK over
> FRI). Read-only; nothing in either repo was modified by this
> review. Verdict + corrections at the end; the G3 spec has been
> annotated with an AUDIT-CORRECTIONS banner pointing here.

> **Decision (2026-05-17).** If G3c proceeds on the
> `Plonky3-recursion` library, we will **vendor it into this repo**
> (pinned/forked, audit-stable, Plonky3-rev aligned to ai-pow-zk —
> resolving F2) and **add Tip5 support to the vendored copy,
> arithmetizing the canonical `nockchain-math::tip5` permutation**
> (a `tip5-circuit-air` analogous to `poseidon2-circuit-air` +
> `CircuitChallenger` Tip5 arms + `PermConfig::Tip5` MMCS + CTL
> wiring). This selects audit remediation **R1b** (keep Tip5
> everywhere; extend the recursion lib) over **R1a** (migrate
> Layer-0 to Poseidon2): the Layer-0 `AiPowStarkConfig` is
> **unchanged**, so the existing 120-bit Tip5 FRI soundness
> analysis (`ai_pow_zk_fri_sweep`) is **preserved, not
> invalidated**. The Tip5 circuit-AIR MUST source its
> constants/round structure from `nockchain-math::tip5` (single
> source of truth) and be guarded by a cross-test asserting the
> in-circuit AIR ≡ native `nockchain_math::tip5::permute` on
> random inputs (a re-implemented hash that subtly differs = silent
> recursion unsoundness — the primary R1b risk). This is intent,
> not yet executed; gated by the prerequisites below; the G4 Pearl
> interim stays authoritative until the vendored+extended stack is
> audited (F7).

**Bottom line.** The G3 *logic* (segment + carry-chain induction +
per-segment CRIT-1 + count/order pinning ⇒ equivalent to the
monolith) is sound and *implementable* on a recursion library of
this shape. **But four spec premises are false or materially
understated against the reference, two of them blocking**, and the
reference is itself explicitly unaudited. G3c is **not** a
"wire-up an existing API" task; it is a soundness-critical
integration with hard prerequisites. The G4 Pearl-faithful interim
is load-bearing **until G3c *and* the recursion stack are
audited**, not merely until code-complete.

---

## 1. What was reviewed

- My spec: `crates/ai-pow-zk/G3_RECURSION_AGGREGATION.md` (all
  sections, esp. §3 recursion primitive, §5 aggregation tree, §6
  `PROGRAM_ROOT`, §8 soundness/error budget, §14 parameters).
- Reference: `Plonky3-recursion/` — `recursion/src/{verifier,
  pcs/fri,challenger,public_inputs,recursion}.rs`,
  `poseidon2-circuit-air/`, `book/src/advanced_topics/
  soundness.md`, README, examples; a full soundness-surface map
  (delegated deep read, cross-checked here).
- Our config: `crates/ai-pow-zk/src/circuit.rs` (`AiPowStarkConfig`),
  `Cargo.toml` (Plonky3 pin), the §6(b) stack just landed.

## 2. Findings (severity-ranked)

Severity: **BLOCKER** (G3c cannot proceed until resolved) ·
**CRITICAL** (silent soundness loss if not built correctly) ·
**HIGH** · **MEDIUM** · **INFO/positive**.

---

### F1 — BLOCKER. Hash mismatch: our Layer-0 is Tip5; the reference recursion verifier arithmetizes **only Poseidon1/Poseidon2**

**Evidence.**
- Ours: `crates/ai-pow-zk/src/circuit.rs:10-13,165-191` —
  `Challenger = DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>`
  **and** `ValMmcs = MerkleTreeMmcs<…, Tip5Sponge, Tip5Compress,…>`
  (FS transcript *and* every Merkle/FRI compression are the
  in-repo `nockchain_math::tip5` 7-round sponge; Plonky3 ships no
  `p3-tip5`).
- Reference: `recursion/src/challenger/circuit.rs:97-129` —
  `CircuitChallenger` duplexes via Poseidon2/Poseidon1 only;
  **`panic!("unsupported challenger permutation")`** at
  `circuit.rs:128` for anything else. MMCS path is the same
  family (`recursion/src/pcs/fri/mmcs.rs:508-513`,
  `params.rs:17-22` `PermConfig ∈ {Poseidon1, Poseidon2}`).
  Repo-wide `tip5` grep ⇒ **zero hits**. Poseidon2 params are
  *fixed* (Goldilocks `WIDTH=8,RATE=4`; 32-bit `16,8`)
  (`book/src/advanced_topics/soundness.md:78-80`).

**Impact on the spec.** `G3_RECURSION_AGGREGATION.md` §1.3, §3.2
("**Tip5 was chosen for recursion**… the recursion cost is
dominated by in-circuit Tip5"), §3.3 cost model, and §14
("recursion field: same Goldilocks + Tip5… self-similar, no
field switch") are **factually wrong**. A recursive verifier must
arithmetize *exactly the hash the inner proof used* for FS +
Merkle. The reference cannot verify a Tip5 proof at all.

**Remediation — CHOSEN: R1b (per the Decision above).**
- **R1b (CHOSEN): vendor the recursion lib and add Tip5 to the
  vendored copy** — a `tip5-circuit-air` arithmetizing the
  canonical `nockchain-math::tip5` permutation (constants/round
  structure sourced from `nockchain-math`, single source of
  truth), `CircuitChallenger` Tip5 arms, `PermConfig::Tip5` MMCS,
  CTL wiring. Layer-0 `AiPowStarkConfig` stays Tip5 ⇒ the base
  proving system and the **120-bit Tip5 FRI analysis
  (`ai_pow_zk_fri_sweep`) are preserved, not invalidated**, and
  the just-landed §6(b)/G1+G2 stack is untouched. Cost: a new
  in-circuit-crypto surface that **itself needs cryptographic
  audit**; the dominant risk is an in-circuit Tip5 that subtly
  differs from native `permute` (silent recursion unsoundness),
  mitigated by sourcing from `nockchain-math` + a mandatory
  cross-test (in-circuit AIR ≡ `nockchain_math::tip5::permute`).
- **R1a (REJECTED): migrate Layer-0 to Poseidon2-over-Goldilocks**
  — would let the reference apply directly, but changes the base
  proving system for **all of ai-pow-zk** and **invalidates the
  120-bit FRI soundness analysis** (must re-derive for Poseidon2)
  + full re-validation of the entire stack. Rejected as
  higher-blast-radius than R1b; the merge-mining invariant is
  unaffected either way (Tip5 is the STARK FRI/Merkle/FS hash,
  *not* the mineable unit — that is the plain BLAKE3-keyed
  `TileState`).

**Net:** G3c is gated on R1b (vendor + Tip5-circuit-AIR). The
spec must stop asserting Tip5-was-chosen-for-recursion; the
*intent* is now to *make* it true in the vendored copy.

---

### F2 — BLOCKER. Plonky3 revision mismatch

**Evidence.** Reference workspace pins every `p3-*` to Plonky3
rev **`56952503e1401a62982ceaf952c5e4a829b61803`**
(`Plonky3-recursion/Cargo.toml:46-78`). `ai-pow-zk` pins Plonky3
via `git = "https://github.com/Plonky3/Plonky3.git"` with the
project rev (`6de5cba`, per memory `ai_pow_zk_state`)
(`crates/ai-pow-zk/Cargo.toml:45-58`). `p3-batch-stark` proof
layout / challenger / FRI internals are **not stable across
revs**; a proof produced under `6de5cba` is not guaranteed
parseable/verifiable by a recursion verifier built against
`56952503…`.

**Impact.** Independent of F1, the segment `BatchProof` our
`composite_prove_pinned_logup_sx` emits may not be ingestible by
the reference verifier. Spec §10/§14 ("confirm
recursion-friendliness at impl") understated this to a footnote;
it is a hard prerequisite.

**Remediation (subsumed by the vendor Decision).** Vendoring the
recursion lib into this repo means **we own and pin its Plonky3
rev inside the vendored tree**, aligned to ai-pow-zk's rev (or
vice-versa) as a one-time migration with **regression risk to the
entire ai-pow-zk stack including the just-landed §6(b)/G1+G2** —
re-run the full `ai-pow-zk --lib` + `ai-pow --features zk` suites
after. Sequence this *before* the R1b Tip5 work so the migration
is done once, on the vendored copy.

---

### F3 — CRITICAL. The inner AIR / verifying-key (program) commitment is **prover-supplied, not pinned** by the library — CRIT-1-across-the-tree is entirely our bespoke responsibility

**Evidence.** The reference allocates *all* commitments — trace,
quotient, **preprocessed/common-data**, FRI caps — as
`alloc_public_input*` **supplied by the prover**
(`recursion/src/pcs/fri/targets.rs:379-382`,
`recursion/src/types/proof.rs:168-172`); the batch verifier
*reconstructs the AIR shape from prover-supplied proof metadata*
(`recursion/src/verifier/batch_stark.rs:227-276`). The circuit
checks openings-hash-to-supplied-cap and cap-observed-into-FS,
but **never that the cap equals a fixed verifying key**.
"Soundness of which program was proven… the recursion crate
itself leaves the inner AIR prover-chosen" (audit map §D).

**Impact on the spec.** §6 (`PROGRAM_ROOT(params)`) and §5.2
step 2 are *correct in intent* but presented as if the binding is
a small membership check the recursion node naturally performs.
In reality the **entire** CRIT-1-across-tree mechanism — exposing
the preprocessed/common-data commitment PI targets, constraining
them to a params-derived `PROGRAM_ROOT` Merkle leaf keyed by
`seg_index`, and proving the FS transcript observed *that*
commitment — is **custom circuit code we must write on the
exposed PI targets; the unified API does none of it**. If we omit
or mis-wire it, a malicious prover supplies the commitment of a
*cheap or forged* program and the recursion accepts it →
**CRIT-1 silently broken across recursion** (the exact class of
bug CRIT-1 fixed at Layer-0, re-opened one layer up). This is the
highest-consequence "looks done but isn't" item.

**Remediation.** Treat `PROGRAM_ROOT`-binding as a first-class,
red-teamed deliverable of G3c (not incidental): explicit
adversarial tests that a wrong/forged/reordered per-segment
preprocessed commitment is **rejected** by the recursion node
(mirror the existing `crit1_*` suite, one layer up).

---

### F4 — CRITICAL. 2-to-1 aggregation binds **no** relation between the two children's public inputs — the carry stitch is bespoke glue we own

**Evidence.** `build_aggregation_layer_circuit` builds the two
child verifiers into one builder; `run_aggregation_verification
_circuit` **concatenates** their public/private inputs
(`recursion/src/recursion.rs:534-571`;
`book/src/.../aggregation.md:24` "The left and right inputs are
independent"). **"There is no built-in constraint relating the
two children's public inputs"** (audit map §D); the example only
verifies the pair. The PI targets *are* exposed and `connect`
exists, so the relation is *expressible* — but never *imposed* by
the API.

**Impact on the spec.** §5.3 step 3 ("Carry stitch:
`L.gamma_hi == Rt.gamma_lo`"), step 2 (span adjacency), step 4
(program-root / block-anchor / `n_segments` equality, `final_seen`
logic) — i.e. **every soundness-bearing cross-child link** — are
**not** provided. They are hand-written `connect()` constraints
on the exposed PI targets. The unified aggregation API, used
naïvely, produces a perfectly-verifying but **completely unsound**
aggregator (it proves "two unrelated proofs exist", not "a
contiguous carry-chained span"). This is *the* central G3
soundness mechanism and it is entirely on us.

**Remediation.** G3c's core is precisely these `connect`
constraints + their adversarial tests (forged carry, non-adjacent
spans, mismatched program-root/block-anchor each **rejected**).
The spec must be reworded from "the aggregation API stitches" to
"we add the stitch constraints on the API-exposed PI targets;
the API does not."

---

### F5 — HIGH. Uni-STARK recursion path has **no LogUp support**; our Layer-0 is batch-STARK + LogUp — must use the batch path exclusively

**Evidence.** `recursion/src/verifier/stark.rs:123,268` —
"Lookups are not supported for recursive single STARK
verification" (empty lookup metadata on the uni path). Batch path
*does* verify LogUp cumulative-sum
(`batch_stark.rs:637-642,1063-1074`). Ours is
`composite_*_pinned_logup` (batch-stark; `noised_packed` / range
/ i8u8 / cv LogUp buses are load-bearing for C3/CRIT-1).

**Impact.** If a segment (or a recursion layer) is ever verified
via the uni-STARK recursion path, the LogUp argument is **not
checked** → the `noised_packed` matrix-binding and range checks
are silently unverified one layer up (a C3/CRIT-1-adjacent hole).
Spec §2/§10 say "batch-stark" but do not *forbid* the uni path.

**Remediation.** Spec + implementation **must** mandate
`RecursionInput::BatchStark` / the batch verifier on every layer
(segment and aggregate), with a compile-time/test guard. Note the
self-similar requirement (§5.7) is satisfied only because the
*aggregate* proof is also batch-stark.

---

### F6 — HIGH. FRI soundness: my error budget used the discredited heuristic; params are trusted config, and `pow_bits == 0` silently disables grinding

**Evidence.** `book/src/advanced_topics/soundness.md:13-44`
explicitly warns the naïve `num_queries × log_blowup` bits model
is **not** a correct soundness bound post-2025 proximity-gap
results; security must use a *proven* `-log2(ε_FRI) +
query_pow_bits`. FRI params (`log_blowup`, `log_final_poly_len`,
`commit_pow_bits`, `query_pow_bits`, `permutation_config`) are
**trusted `FriVerifierParams` config, not derived from the proof**
(`recursion/src/pcs/fri/params.rs:8-23`); a wrong
`log_final_poly_len` weakens the low-degree bound silently.
`check_pow_witness` with `*_pow_bits == 0` is a **no-op** (no
grinding) (`recursion/src/challenger/circuit.rs:391-406`). Query
count is prover-supplied, only `≥ 1` enforced
(`pcs/fri/verifier.rs:1424-1430`).

**Impact on the spec.** §8.3 (`ε_total ≤ 2N·ε_stark + ε_FS`,
`ε_stark ≈ 2⁻¹²⁰`) is *structurally* right (additive in tree
size — good, that part holds) but **`ε_stark` must come from a
proven FRI bound, not the heuristic**, and must be computed
**per layer** including grinding; with conjectured-soundness
params the bound is weaker than stated. §14's "reuse the
`ai_pow_zk_fri_sweep` methodology" is right but that sweep is for
*Tip5* and will be redone under R1a anyway.

**Remediation.** Restate §8.3 against a proven ε_FRI; pin every
layer's `FriVerifierParams` from that bound (never `pow_bits =
0`); make the chain-level root verifier assert the
recursion-layer params equal the params-derived expected values
(MED-3 discipline) so the *trusted config* is in fact
verifier-recomputed, not prover-influenced.

---

### F7 — HIGH (operational/soundness-governance). The reference is explicitly **unaudited / not production-ready**

**Evidence.** README + `book/src/getting_started/
introduction.md:9`: "**under active development, unaudited and as
such not ready for production use.**" Active RFCs
(`docs/rfcs/0001,0002`), roadmap items (configurable WIDTH/RATE,
multi-shape FRI), `TODO: Update once Plonky3 PR #1329 lands` in
examples.

**Impact on the spec.** §13 framed G4 as the interim "*until G3c
lands*". Correction: for a **consensus-critical PoW soundness
path**, depending on an unaudited recursive verifier means the
in-circuit PROD matmul-truth guarantee is only as strong as that
unaudited stack. The G4 Pearl-faithful spot-check externality
must remain authoritative **until G3c *and* the recursion
library *and* our bespoke F3/F4 glue are independently audited** —
strictly later than "code-complete". This strengthens, not
weakens, the honest-scoping posture already in the docs.

---

### F8 — MEDIUM. `unsafe_arithmetic_only_for_tests` FRI params disable **all** Merkle verification

**Evidence.** `recursion/src/pcs/fri/params.rs:18-22,43-71` —
that constructor sets `permutation_config: None`, which **skips
MMCS/Merkle opening verification entirely**; doc says "**unsound
for production**"; the only safe ctor is `with_mmcs`.

**Impact.** Catastrophic if it ever reaches a non-test path of
our integration (FRI openings unverified ⇒ trivially forgeable).
Clearly named/test-gated upstream, so low likelihood — but our
integration must make it *impossible*.

**Remediation.** A build/CI guard in the G3c crate forbidding the
`unsafe_*` ctor outside `#[cfg(test)]`; assert `permutation_config
== Some(expected)` at the chain-level root check.

---

### F9 — MEDIUM. Periodic AIR columns are `unimplemented!` in the reference circuit builder — our composite AIR must be proven periodic-free

**Evidence.** `circuit/src/symbolic/targets.rs:64` —
`unimplemented!("Periodic values are not supported.")`. The
recursive constraint-evaluator cannot handle an AIR that uses
periodic columns.

**Impact.** `CompositeFullAirWithLookupsPinned` aggregates many
chips (range_table, blake3, i8u8, control, input, matmul,
jackpot, fold, stripe_xor). If *any* uses Plonky3 periodic
columns, the reference cannot recursively verify our segment
proof. Not yet confirmed either way (a targeted grep of the
composite AIR did not surface periodic use, but that is not a
proof of absence across all chips).

**Remediation.** G3c prerequisite check: statically confirm no
wired chip emits `Air::eval` periodic/`PeriodicColumns` (or the
symbolic-builder equivalent); add a unit test asserting the
composite AIR's symbolic constraint set is periodic-free.

---

### F10 — INFO / positive. The G3 *architecture* is validated by the reference

The reference confirms the parts of the spec that **hold**:
self-similar **multi-layer recursion** (`into_recursion_input::
<BatchOnly>()`) and **2-to-1 aggregation across heterogeneous
inner shapes** are real and supported; **ZK** is handled
(`stark.rs:162-165`, `batch_stark.rs:425-432`, `zk_aggregation`
tests); FRI is **transparent** (no trusted setup at any layer);
recursion-node cost is **~constant in segment trace size** (only
`log` terms) as the spec claims; the unified API
(`prove_next_layer` / `build_aggregation_layer_circuit`) maps
cleanly to leaf/internal nodes; the inner proof's public inputs
**are** exposed as outer PI targets, so the carry/program/anchor
constraints **are expressible** (F3/F4 are "not provided", not
"not possible"). The G3 induction-soundness argument (§8.2) is
unaffected by F1–F9 — those concern *whether the recursion node
actually checks what the proof sketch assumes*, which is exactly
what F3/F4/F5 say we must build and red-team.

---

## 3. Net verdict & corrected G3 prerequisites

**The G3 design is sound; the spec was over-optimistic about what
the library provides and wrong about the hash.** Revised ordered
prerequisites for G3c (none of which were correctly stated in the
original spec):

| # | Prerequisite | From finding | Class |
|---|---|---|---|
| P0 | **Vendor `Plonky3-recursion` into the repo** (pinned/forked), Plonky3 rev aligned to ai-pow-zk inside the vendored tree; full regression | F2, F7 | one-time dependency migration |
| P1 | **Add `tip5-circuit-air` to the vendored copy** arithmetizing `nockchain-math::tip5` (constants from `nockchain-math`, single source of truth) + `CircuitChallenger`/MMCS Tip5 arms + mandatory cross-test (in-circuit ≡ native `permute`). Layer-0 stays Tip5; the 120-bit FRI sweep is **preserved**. (R1b chosen; R1a rejected.) | F1 | new in-circuit-crypto, soundness-critical, needs crypto audit |
| P2 | Confirm `CompositeFullAirWithLookupsPinned` is periodic-free | F9 | compatibility gate |
| P3 | G3c built **batch-stark path only**, with a guard against the uni path and the `unsafe_*` FRI ctor | F5, F8 | implementation constraint |
| P4 | Build + **red-team** the bespoke recursion-node glue: `PROGRAM_ROOT` per-segment CRIT-1 binding (F3) and the cross-child carry/adjacency/anchor stitch (F4), with adversarial tests mirroring `crit1_*` one layer up | F3, F4 | the soundness core of G3c |
| P5 | Pin every layer's `FriVerifierParams` from a *proven* ε_FRI + grinding; chain-root asserts them = params-derived (MED-3 discipline) | F6 | soundness-critical config |
| P6 | Treat G4 (Pearl spot-check interim) as authoritative until G3c **and** the recursion stack **and** P4 glue are independently audited | F7 | soundness governance |

G3a + G3b (boundary-predicate parameterization, segment schedule,
`canonical_segment_program`/`program_root`) remain **unaffected
and still implementable now** — they are Layer-0-only, Plonky3-
and hash-agnostic, and are exactly the substrate P4 will bind.
The recursion-cost / depth=log N / additive-error / no-trusted-
setup claims **stand**.

## 4. Corrections applied to the design docs

- `G3_RECURSION_AGGREGATION.md`: prepended an
  **AUDIT-CORRECTIONS** banner pointing here; the Tip5
  assertions (§1.3/§3.2/§3.3/§14) and the "API stitches the
  carry / pins the program" implications (§5.2/§5.3/§6) are now
  flagged as superseded by F1/F3/F4; §8.3's `ε_stark` is flagged
  to use a proven FRI bound (F6).
- `HIGH2_2_DESIGN.md` §4.C.4-G3: pointer updated to cite this
  audit as a gating prerequisite list for G3c.
- Task **#108** + memory `ai_pow_zk_crypto_gaps`: residual scope
  re-stated with P0–P6.

## 5. Cross-references

- My spec: `G3_RECURSION_AGGREGATION.md` (annotated).
- Reference: `Plonky3-recursion/` —
  `recursion/src/challenger/circuit.rs:97-129` (hash/panic),
  `pcs/fri/params.rs:8-72` (FRI params / unsafe ctor),
  `pcs/fri/targets.rs:379-382` + `types/proof.rs:168-172`
  (prover-supplied commitments),
  `verifier/batch_stark.rs:227-276` (AIR from prover metadata),
  `verifier/stark.rs:123,268` (no uni LogUp),
  `recursion/src/recursion.rs:534-571` (aggregation concat, no
  cross-child binding), `book/src/advanced_topics/soundness.md`
  (FRI soundness model), `Cargo.toml:46-78` (Plonky3 pin),
  `book/src/getting_started/introduction.md:9` (unaudited).
- Ours: `crates/ai-pow-zk/src/circuit.rs:10-13,165-191` (Tip5
  config), `crates/ai-pow-zk/Cargo.toml:45-58` (Plonky3 pin),
  `composite_proof.rs` (batch-stark Route-A), CRIT-1 / MED-3
  discipline in `ZKP_SECURITY_REPORT.md`.
- Task **#108**; memory `ai_pow_zk_crypto_gaps`,
  `ai_pow_zk_fri_sweep` (the Tip5 FRI budget P1 invalidates).
```
