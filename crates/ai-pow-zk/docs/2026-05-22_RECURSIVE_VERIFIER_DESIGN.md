# 2026-05-22 — The recursive STARK verifier: how the verifier circuit works and is generated

> Design / orientation doc. Covers the vendored `Plonky3-recursion/`
> substrate as used by the `ai-pow-zk` proving stack: what the
> "verifier circuit" is, the primary entrypoints, the generation
> pipeline, how the inner proof's shape enters, and what is / is not
> wired today. Written after a code walk of `Plonky3-recursion/`
> (entrypoints verified directly; pipeline cross-checked).

## 0. TL;DR

- The "verifier circuit" is **not a file, not a committed artifact,
  not codegen output.** It is built **programmatically, fresh on
  every run**, by calling one function and letting it append
  constraints to a `CircuitBuilder`.
- **Primary entrypoints:**
  - L1 (verify a uni-STARK proof): `verify_p3_uni_proof_circuit`
    — `Plonky3-recursion/recursion/src/verifier/stark.rs:59`.
  - L2 (verify a batch-STARK proof, i.e. verify L1):
    `verify_p3_batch_proof_circuit`
    — `Plonky3-recursion/recursion/src/verifier/batch_stark.rs`.
- The inner AIR is a **compile-time generic** (`A: RecursiveAir`);
  the inner proof's *shape* (trace width, opened-values length,
  FRI params) is consumed **dynamically** and validated against
  `A::width()`. There is **no inner-width value baked into a
  persistent artifact** ⇒ **nothing to "regenerate"**: recompiling
  picks up any change automatically.
- **What is wired today:** the recursion substrate verifies a
  `FibonacciAir` *representative* inner STARK under ai-pow-zk's
  Layer-0 `StarkConfig` **replicated byte-identically**. The real
  `ai-pow-zk` composite AIR is **not** yet recursed (residual R-b,
  below). `Plonky3-recursion` deliberately does **not** depend on
  `ai-pow-zk` (the C1 workspace decoupling).

## 1. The recursion stack — what each layer is

The `ai-pow` PoUW pipeline produces a chain of STARK proofs, each
verifying the previous one in-circuit so the final certificate is
small:

```
  L0  inner composite STARK   — ai-pow-zk `CompositeFullAir`
      (the mined-work proof; TOTAL_TRACE_WIDTH = 1911 cols today)
        │  proof_{L0}
        ▼
  L1  outer cert              — a STARK whose AIR *is* the L0
      (uni-STARK)               verifier expressed as a circuit
        │  proof_{L1}
        ▼
  L2  the shipped certificate — a STARK whose AIR is the L1
      (batch-STARK)             verifier expressed as a circuit
```

"L1 verifies L0" means: L1's AIR proves the statement *"I ran the
L0 STARK verifier on proof_{L0} and it accepted."* The L0 verifier
algorithm — FRI low-degree testing, Merkle openings, quotient
recomposition, AIR-constraint folding — is compiled into an
arithmetic **circuit**, and L1 is an ordinary STARK proving correct
execution of that circuit. L2 does the same to L1.

## 2. What the "verifier circuit" is

It is a `p3_circuit::Circuit<F>` — an ALU/VM-style execution IR:
a list of primitive ops (field arithmetic, comparisons) plus
**non-primitive operations** (NPOs) for expensive gadgets, chiefly
the **Tip5 permutation** (Merkle/sponge hashing) and `Recompose`
(base↔extension-field repacking).

It is produced by a **`CircuitBuilder<F>`**
(`Plonky3-recursion/circuit/src/builder/circuit_builder.rs`):

1. The caller creates a `CircuitBuilder`, enables the NPOs it needs
   (`enable_tip5_perm`, `enable_recompose`).
2. The caller calls a **verifier entrypoint** (§3), which *appends*
   all the verification constraints to the builder.
3. `builder.build()` finalizes a `Circuit<F>`.
4. A *runner* executes the circuit on the public + private inputs,
   filling the witness; that execution trace is what L1 then
   STARK-proves.

There is **no serialized circuit, no `build.rs` codegen, no
committed `.bin`/`.ir`**. `Plonky3-recursion/scripts/` holds only
`benchmark.sh` / `profiling.sh` — no regeneration utility, because
there is nothing static to regenerate.

## 3. Primary entrypoints

### L1 — `verify_p3_uni_proof_circuit` (`verifier/stark.rs:59`)

```rust
pub fn verify_p3_uni_proof_circuit<A, SC, Comm, InputProof,
                                   OpeningProof, CP, WIDTH, RATE>(
    config:                 &SC,                       // STARK config: PCS + challenger
    air:                    &A,                        // the INNER AIR (generic)
    circuit:                &mut CircuitBuilder<..>,    // builder to append to
    proof_targets:          &ProofTargets<..>,          // in-circuit proof representation
    public_values:          &[Target],
    preprocessed_commit:     &Option<Comm>,
    pcs_params:             &PcsVerifierParams<..>,     // FRI log_blowup / num_queries / ...
    challenger_perm_config:  CP,
) -> Result<Vec<NonPrimitiveOpId>, VerificationError>
where A: RecursiveAir<Val<SC>, SC::Challenge, LogUpGadget>, ...
```

Returns the list of `NonPrimitiveOpId`s that still need **private
data** (e.g. MMCS Merkle sibling values) — the caller wires those
in before running the circuit.

### L2 — `verify_p3_batch_proof_circuit` (`verifier/batch_stark.rs`)

Same idea, one level up: it consumes a `BatchStarkProof` (L1 proves
*all* its NPO tables together as one batch-STARK) plus
`CommonData` (preprocessed columns) and the registry of NPO
provers, and appends the L1-verification constraints to the L2
circuit builder.

`StarkVerifierInputsBuilder::allocate` / `ProofTargets::new`
(`recursion/src/public_inputs.rs`, `types/proof.rs`) allocate the
circuit targets that mirror a proof — sized from the **proof's own
shape**, read at allocation time.

## 4. The verification pipeline (inside the entrypoint)

The entrypoint appends, in order:

1. **Proof-target allocation** — targets for trace commitments,
   quotient-chunk commitments, opened values, FRI data. Opened-value
   target counts are taken from the proof (`trace_local.len()` etc.).
2. **Challenge derivation** — a `CircuitChallenger` re-derives the
   Fiat-Shamir challenges (α for constraint folding, ζ / ζ·g for the
   opening points, the FRI betas) by hashing the commitments
   in-circuit with Tip5.
3. **Quotient recomposition** — rebuild the quotient polynomial
   evaluation from its committed chunks
   (`recompose_quotient_from_chunks_circuit`).
4. **Constraint folding** — evaluate every inner-AIR constraint at
   ζ from the opened trace values and fold them with α into one
   value; assert it equals the quotient · vanishing.
5. **Proof-shape validation** — assert the opened trace width equals
   `A::width(air)`; a mismatch is `VerificationError::InvalidProofShape`.
6. **PCS / FRI opening verification** — `pcs.verify_circuit(...)`:
   the FRI fold-chain + Merkle-path (Tip5 NPO) checks that the
   opened values are genuine evaluations of the committed
   polynomials.

## 5. How the inner proof's shape enters — and why nothing is "baked"

This is the load-bearing question for parameter/width changes.

- The inner AIR is a **generic type parameter** `A`. Its width comes
  from `A::width()` — for the real inner, `CompositeFullAir::width()
  == composite_layout::TOTAL_TRACE_WIDTH` (a `const`, **1911**
  today after the Path A column-overlay).
- The verifier **does not embed a width literal**. It (a) allocates
  opened-value targets from the *proof's* `trace_local.len()`, and
  (b) cross-checks that against `A::width()` at step 5.
- Consequence: when the inner AIR's width changes (this session:
  2135 → 2103 → **1911**), the verifier circuit **automatically
  re-shapes on the next compile + run**. There is no artifact to
  regenerate, no codegen to rerun. A *stale* verifier (built for an
  old width) fed a *new* proof would simply fail step 5 with
  `InvalidProofShape` — it cannot silently mis-verify.

**So: the Path A overlay (inner width 2103 → 1911) requires no
verifier-circuit regeneration.** It only changes numbers that a
re-run measures.

## 6. What is wired today — config-faithful, AIR-representative

`Plonky3-recursion` is a **generic** recursion substrate and
**cannot depend on `ai-pow-zk`** (the C1 workspace decoupling —
`test_tip5_layer0_recursion.rs:137`: *"We CANNOT depend on
ai-pow-zk"*). So the recursion tests:

- **Replicate ai-pow-zk's Layer-0 `StarkConfig` byte-identically**
  in-workspace (Tip5 sponge/compress MMCS, challenger, the FRI
  params `lb=4 lfp=0 cpow=1 qpow=1`). The recursion verifier is
  validated to verify *that exact config* in-circuit, accept +
  tamper-reject, across the soundness sweep.
- Use a **`FibonacciAir` representative** as the *inner AIR*, not
  the real `CompositeFullAir`.

Two named residuals (from `test_tip5_layer0_recursion.rs:78-104`):

- **R-a** — a LogUp CTL-orphan in the Layer-0 *recursion* (a
  `Recompose`-coeff producer/consumer imbalance on non-`Hint`
  Tip5-input coeffs). Documented known-gap of the milestone-C3/#124
  vertical-recursion certificate; closing it touches fenced
  soundness-linchpin code.
- **R-b** — recursively verifying **ai-pow-zk's actual composite
  `RecursiveAir`** (vs the representative `FibonacciAir`).
  Explicitly out of scope, "M12-adjacent".

## 7. How to build / measure each layer

| Layer | Where | Entrypoint / test |
|---|---|---|
| L0 inner composite (real, 1911-col) | `ai-pow-zk` | `bench_suite.rs` — `prove_batch`/`verify_batch` over `CompositeFullAirWithLookups`; reports `prove_ms` + proof bytes |
| L1 over the representative inner | `Plonky3-recursion` | `recursion/tests/test_l1_outer_cert_tip5_unified.rs`, `test_tip5_layer0_compression.rs` |
| L2 over L1 | `Plonky3-recursion` | `recursion/tests/test_tip5_l2_over_l1.rs` — `stage5_tip5_l2_over_l1_production_measurement` serialises L1 + L2 with `postcard` and prints KB |

Heavy tests are `#[ignore]`d; run with `--ignored --nocapture`.

## 8. Implications for profiling the width reduction

- **Inner (real) profiling** — `bench_suite.rs` measures the real
  1911-column composite directly; it reflects the Path C + Path A
  width reductions (2135 → 1911, −224 cols). Ready now.
- **L1/L2 recursion profiling** — `test_tip5_l2_over_l1.rs` measures
  L1/L2 at the production config, but over the `FibonacciAir`
  representative. The recursion FRI machinery (queries, fold chain,
  Merkle paths) dominates those proof sizes and is *config*-driven,
  so the numbers are representative of the recursion overhead — but
  the inner trace-opening portion of the L1 circuit is sized to
  Fibonacci, not to 1911 columns, so it **under-counts** the real
  L1 relative to a true composite inner.
- **True end-to-end (composite → L1 → L2)** profiling requires
  residual **R-b**: feeding the real `CompositeFullAir` proof as L0.
  Because `Plonky3-recursion` can't depend on `ai-pow-zk`, R-b means
  either replicating the composite AIR into the recursion workspace
  or adding a glue crate that depends on both. That is a genuine
  build task, not a measurement — and the prerequisite for an
  honest full-stack number.

## 9. Cross-references

- Entrypoints: `Plonky3-recursion/recursion/src/verifier/stark.rs`,
  `.../verifier/batch_stark.rs`.
- Circuit IR: `Plonky3-recursion/circuit/`,
  `circuit/src/builder/circuit_builder.rs`.
- Configs: `Plonky3-recursion/circuit-prover/src/config.rs`
  (`goldilocks_tip5_*`).
- Inner config replication + residuals:
  `Plonky3-recursion/recursion/tests/test_tip5_layer0_recursion.rs`.
- Inner AIR width: `crates/ai-pow-zk/src/composite_layout.rs`
  (`TOTAL_TRACE_WIDTH`).
