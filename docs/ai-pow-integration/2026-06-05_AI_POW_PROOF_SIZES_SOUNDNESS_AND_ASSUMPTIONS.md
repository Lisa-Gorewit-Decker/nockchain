# AI-PoW Proof Sizes, Soundness, And Assumptions

Date: 2026-06-05
Status: current measurement and cryptographic-assumption checkpoint.

## Scope

This note answers four concrete questions for the current AI-PoW proving stack:

1. How large is the regular proof of the AI-PoW puzzle?
2. How large is the recursive proof of that proof?
3. How sound is each layer?
4. Which cryptographic assumptions are taken at each step, and are they cited?

Here "regular proof" means the Layer-0 `CompositeFullAirWithLookupsPinned`
batch-STARK produced by `composite_prove_pinned_logup`. It does not mean the
legacy/plain `MatmulProof`, which is a miner diagnostic and pre-ZKP target-hit
object, not the production block artifact.

The production recursive-certificate target is the native terminal backend
from `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`. The older L1
batch-STARK recursive certificate remains an important hardened verifier path
and regression target, but it is too large for the production wire budget and
must not be treated as the production block/wire artifact.

## Current Sizes

| Artifact | Current production role | Last measured size | Source |
|---|---|---:|---|
| Layer-0 composite proof | Regular STARK proof of the AI-PoW puzzle statement; consumed by recursion; diagnostic/intermediate, not persisted by consensus | `304,048` bytes / `296.9 KiB` | `RUSTFLAGS="-C target-cpu=native" cargo run -p ai-pow-zk --release --features recursion --example prod_recursion_measure -- 15`, 2026-06-05 after `CircuitConfig::PROD.pow_bits=0` |
| Hardened L1 batch-STARK recursive certificate | Soundness-hardened recursive verifier checkpoint/fallback path; not acceptable as the production wire artifact because it exceeds the size budget | L1 proof body `149.1 KiB`; full checkpoint certificate `1,135.5 KiB` legacy postcard / `5,794.7 KiB` fixed-int bincode / `358.3 KiB` gzip-best envelope | same `prod_recursion_measure 15` run |
| Native terminal certificate fixture | Recursion-crate terminal proof over the real Tip5 verifier-circuit fixture; proves the terminal backend can be small, but is not yet the full `ai-pow-zk` composite verifier path | `85,948` bytes / `83.9 KiB`; release prove `1.492s`, verify `1.181s` | `RUSTFLAGS="-C target-cpu=native" cargo test --manifest-path crates/plonky3-recursion/recursion/Cargo.toml --release --test test_l1_outer_cert_tip5_unified terminal_production_certificate_measures_real_tip5_l0_verifier_circuit -- --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier native terminal path | Opt-in diagnostic path for the actual composite L1 verifier circuit; not yet production-qualified because it misses both size and time gates | `lb=6,nq=10,pow=0` reduced-profile run after compact known-index proof encoding: terminal certificate `766,069` bytes / `748.1 KiB`; terminal public inputs `5,180` bytes; postcard wire certificate `771,249` bytes / `753.2 KiB`; release prove `80.377s`, verify `58.496s` | `NOCK_TERMINAL_PROFILE_PROVER=1 RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_recursive_certificate_for_pure_query_lb6_nq10_measures -- --ignored --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier integrated-LogUp polynomial NPO candidate | Diagnostic only; attempts to replace exhaustive NPO openings with the integrated polynomial NPO backend while keeping the native terminal recursive-certificate shape | No completed size measurement. First release/native command compiled in `1m57s`, then the test binary ran for more than `7m35s` without reaching the final size/timing print and was stopped. A phase-instrumented rerun compiled in `1m42s` and showed `38.235s` primitive prove plus `51.902s` merged value-bridge prove before the integrated Tip5 LogUp subproof finished | `NOCK_TERMINAL_PROFILE_PROVER=1 RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_integrated_logup_candidate_for_pure_query_lb6_nq10_measures -- --ignored --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier terminal relation metrics | Non-proving diagnostic for the same path | PROD baseline: `125,961` ops, `221,989` witnesses, `43,443` terminal private inputs, `14,049` NPO rows, `242,798` NPO residual components, `5,319` bytes of terminal public inputs, terminal compile `20.943s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_relation_metrics_for_prod_baseline_composite_are_available -- --ignored --nocapture`, 2026-06-05 |

The active production target is therefore:

- regular Layer-0 proof: **296.9 KiB** if materialized;
- hardened batch-STARK L1 checkpoint: the submitted L1 proof body is
  **149.1 KiB**, and the full checkpoint certificate is **1.1 MiB+** before
  optional compression; soundness-relevant but too large for production wire
  use;
- native terminal recursive fixture: **85,948 bytes / 83.9 KiB**, satisfying
  the about-100 KiB and `<30s` release-proving gates for the recursion-crate
  Tip5 verifier fixture;
- full `ai-pow-zk` composite-verifier terminal integration: now has an opt-in
  API and measurement test, and decoded verification passes after canonical
  verifier-key rebuild. The best measured reduced-profile wire object is still
  **771,249 bytes / 753.2 KiB**, with release prove **80.377s** and verify
  **58.496s**, so this path misses both the about-100 KiB and `<30s` gates.
- full `ai-pow-zk` composite-verifier integrated-LogUp polynomial NPO
  candidate: now has an opt-in measurement test, but the release/native runs
  show it is far outside the proving-time gate. The first run did not complete
  after more than **7m35s** in the test binary. The phase-instrumented rerun
  showed **38.235s** primitive prove plus **51.902s** merged value-bridge prove
  before the integrated Tip5 LogUp subproof finished, so the small synthetic
  **94.0 KiB / 23.070s** checkpoint cannot be promoted as the production
  recursive path.

Verifier status after the 2026-06-05 hardening pass: the batch-STARK
`AiPowRecursiveCertificate` verifier now calls
`BatchStarkProver::verify_all_tables` for the submitted L1 outer proof. That
fix is still required for cryptographic hygiene of the batch-STARK path: the
hardened verifier accepts honest generated L1 outer proofs and rejects outer
proof-body tampering, non-production envelopes, metadata tampering, and wrong
statement public inputs in the Rust test suite. This hardening does not make
the batch-STARK L1 certificate the production wire artifact. The Hoon/kernel
path remains fail-closed for `%ai-pow` until verifier wiring is explicitly
added.

## Recursive Pipeline Bottleneck Audit

There are currently two recursive proof pipelines in the tree, and they must
not be conflated:

1. `crates/ai-pow-zk/src/recursion.rs::AiPowRecursiveCertificate` is the
   hardened batch-STARK checkpoint certificate. It is the object still encoded
   by `crates/ai-pow-miner/src/certificate_noun.rs`.
2. `crates/plonky3-recursion/recursion/src/terminal.rs::TerminalCertificate`
   is the native terminal certificate target that currently meets the size and
   release-time gates for the recursion-crate Tip5 verifier fixture.
3. `crates/ai-pow-zk/src/recursion.rs::AiPowTerminalRecursiveCertificate`
   now wires that terminal backend to the actual composite L1 verifier circuit
   as an opt-in diagnostic path. It verifies after postcard decode and
   canonical verifier-key rebuild, but it has not met the production size or
   time gate.

The batch-STARK checkpoint pipeline is large because it proves the verifier
circuit execution as another full STARK:

```text
Layer-0 composite trace
  -> composite_prove_pinned_logup
  -> build_composite_l1_verifier_circuit
  -> run_composite_l1_verifier
  -> prove_all_tables over the verifier-circuit traces
  -> AiPowRecursiveCertificate { l0_proof, l0_program, l1_outer_proof }
```

The production-faithful `prod_recursion_measure 15` run measured the following
for this batch-STARK checkpoint path:

| Stage | Time | Why it costs |
|---|---:|---|
| L0 composite prove | `34.11s` | Proves a `2^15` row, `1917` column composite AIR with `log_blowup=4`; the main-trace LDE alone is about `7.5 GiB` before permutation, quotient, and Merkle-tree allocations |
| L1 verifier-circuit build | `0.51s` | Builds the recursive verifier circuit and allocates targets; not the bottleneck |
| L1 in-circuit verify | `0.06s` | Runs the verifier circuit once to produce traces; not the bottleneck |
| L1 outer certificate prove + verify | `59.21s` | Proves the verifier-circuit traces with `BatchStarkProver::prove_all_tables` using the production Tip5/recompose NPO tables and FRI profile |
| End-to-end trace-to-recursive-proof time | `93.88s` | Mostly L0 proving plus the L1 batch-STARK outer proof |
| Recursive-only time after L0 proof exists | `59.77s` | Mostly the L1 outer batch-STARK proof |

The size bottleneck in that same batch-STARK checkpoint is not top-level noun
or serde framing. It is FRI opening material in the L1 outer proof:

| Component | Measured size |
|---|---:|
| Fixed-int full batch-STARK checkpoint certificate | `5,933,764` bytes / `5,794.7 KiB` |
| Legacy postcard full checkpoint certificate | `1,162,800` bytes / `1,135.5 KiB` |
| Gzip-best full checkpoint certificate | `366,944` bytes / `358.3 KiB` |
| L1 proof body inside the checkpoint | `149.1 KiB` |
| L1 proof commitments | `4.5 KiB` |
| L1 opened values | `24.0 KiB` |
| L1 opening proof | `117.3 KiB` |
| L1 global lookup data | `3.4 KiB` |

The opening proof dominates because the outer proof is a conventional
multi-table batch-STARK over the executed verifier circuit. The active
`q=9`, `cap_height=5` shape still carries 11 independent Merkle auth-path tree
groups and 1008 raw authentication siblings. A direct path-compression model
saved only `1.4 KiB` on average and `5.4 KiB` in the best sampled case, leaving
the full fixed-int checkpoint floor around `5,789.3 KiB` even in the best
sample. Reducing queries to `q=8` with more query PoW was previously measured
as smaller, but it violates the production terminal policy of not counting
query PoW toward the 60-bit floor and does not address the full-certificate
context overhead.

The native terminal certificate is smaller and faster because it does not wrap
the verifier execution in another batch-STARK proof format. It compiles the
terminal relation directly and proves:

- a primitive sparse-R1CS row-product component, currently `22,631` bytes;
- an exhaustive supported-NPO component, currently `62,909` bytes;
- a single production prelude binding exactly the assignment root.

That is why the recursion-crate terminal fixture measured `85,948` bytes with
release prove `1.492s` and verify `1.181s`. The follow-up `ai-pow-zk`
integration pass added
`prove_terminal_certificate_from_chain_verified_composite_proof`, which proves
the actual composite L1 verifier circuit with the terminal backend and carries
the terminal public input vector alongside the terminal certificate. A
completed release/native reduced-profile run now measures:

| Full composite terminal profile | Certificate | Public inputs | Postcard wire | Compile | Prove | Verify |
|---|---:|---:|---:|---:|---:|---:|
| `lb=6,nq=10,pow=0` | `766,069` bytes / `748.1 KiB` | `5,180` bytes | `771,249` bytes / `753.2 KiB` | `7.579s` | `80.377s` | `58.496s` |

The `lb=6,nq=10,pow=0` profile is the smallest relation measured so far, but
the full composite terminal path is still more than seven times the byte target
and well over the time target. The higher-level miner noun path also still
serializes the batch-STARK checkpoint object.

The integrated-LogUp polynomial NPO backend is not a production escape hatch
for this full composite path yet. In the recursion crate's synthetic NPO-only
Tip5 test, the bundled checkpoint measures `95,403` bytes and the primitive
plus NPO candidate body measures `96,219` bytes with `23.070s` total prove
time. The full composite diagnostic added after this audit binds the assignment
root plus the merged NPO and bundled Tip5 FRI roots, then proves the primitive
row-product component and attempts the integrated NPO proof over the actual
AI-PoW L1 verifier relation. The release/native run did not reach its final
size print after more than `7m35s` in the test binary, which is enough to
reject it for the current `<30s` production proving gate. It remains a
soundness and size-reduction research path, not the active production recursive
certificate backend.

A phase-instrumented rerun made the failure mode more concrete:

| Full composite integrated-LogUp phase | Time |
|---|---:|
| Layer-0 proof generation for the diagnostic fixture | `32.447s` |
| L1 verifier-circuit build | `0.466s` |
| L1 verifier trace execution | `0.045s` |
| Terminal compile | `7.607s` |
| Assignment oracle commitment | `14.281s` |
| Merged NPO prelude root construction | `10.772s` |
| Bundled Tip5 prelude root construction | `13.020s` |
| Terminal prelude build | `7.551s` |
| Primitive R1CS row-product proof | `38.235s` |
| Merged value-bridge proof | `51.902s` |
| Integrated Tip5 LogUp proof | still running when stopped |

The root-construction phases duplicate work that the current subproof provers
redo internally, but removing only that duplication would not be enough: the
primitive row-product and merged value-bridge proofs alone already cost about
`90s`, before the integrated Tip5 LogUp proof completes.

The proof-body split after compact known-index proof encoding is:

| Component | Bytes | Notes |
|---|---:|---|
| Full terminal production body | `765,844` | Excludes outer certificate metadata and terminal public-input vector |
| Primitive R1CS row-product proof | `52,821` | Includes `50,348` bytes of assignment-evaluation material |
| Exhaustive NPO proof | `712,830` | Dominates the body |
| Exhaustive NPO hidden Tip5 values | `92,802` | Deterministic hidden input lanes needed for Merkle Tip5 row checks |
| Exhaustive NPO assignment-witness multiproof | `620,028` | Opens `47,814` assignment values and `5,434` Merkle frontier nodes |

This is after two serialization reductions in the known-index multiproof:
fixed-width little-endian field limbs and fixed-width little-endian frontier
digests. Before those reductions, the same run measured `891,780` bytes on the
postcard wire. The remaining gap is therefore not ordinary varint overhead; it
is the exhaustive requirement to reveal and authenticate tens of thousands of
assignment values.

This integration run also exposed a verifier-key reconstruction soundness bug:
two rebuilds of the same composite L1 verifier circuit produced different
terminal relation digests, so a decoded certificate could fail with
`CertificateHeaderMismatch`. The root cause was hash-ordered emission of global
lookup cumulative checks in the recursive batch-STARK verifier circuit. The
builder now emits those checks in sorted name order, and the regression
`terminal_header_rebuilds_deterministically_for_baseline_composite` confirms
that the same Layer-0 proof rebuilds the same terminal header. The terminal
wrapper also uses postcard encoding with a structural round-trip assertion.

The non-proving terminal-relation metrics explain why this full path is slow:

| Metric | TEST_PEARL baseline | PROD baseline |
|---|---:|---:|
| Terminal compile time | `21.947s` | `20.943s` |
| Terminal public input values | `459` | `459` |
| Terminal public input bytes | `5,381` | `5,319` |
| Terminal private input values | `46,002` | `43,443` |
| Terminal operations | `131,242` | `125,961` |
| Primitive operations | `111,656` | `106,349` |
| Const operations | `578` | `582` |
| Public operations | `459` | `459` |
| ALU add operations | `8,939` | `8,832` |
| ALU multiplication operations | `10,317` | `10,234` |
| ALU boolean-check operations | `240` | `255` |
| ALU fused multiply-add operations | `10,195` | `10,117` |
| ALU Horner-accumulator operations | `80,928` | `75,870` |
| Hint operations | `5,577` | `5,563` |
| Supported NPO operations / rows | `14,009` | `14,049` |
| Tip5 rows | `8,002` | `8,081` |
| Recompose rows | `238` | `225` |
| Recompose/coeff rows | `5,769` | `5,743` |
| NPO callsite input slots | `140,046` | `141,232` |
| NPO callsite output slots | `86,027` | `86,778` |
| NPO residual components | `253,882` | `242,798` |
| Circuit fingerprint | `witness=232,554 public=459 private=46,002 ops=131,242` | `witness=221,989 public=459 private=43,443 ops=125,961` |

The terminal public input vector is about `5.3 KiB`, so including it with the
terminal certificate is not the size blocker. The blockers are the generic L1
verifier relation itself: more than `100k` primitive operations, about `14k`
supported NPO rows, and a terminal compile step that already consumes most of
the `<30s` budget before terminal proving begins. Within the primitive side,
the single biggest class is Horner accumulation: `75,870` PROD operations,
about 71% of all primitive operations. Removing the last proof-system PoW
grinding from `CircuitConfig::PROD` only reduced the relation by 30 operations,
so PoW removal is a soundness-policy cleanup rather than the size/time fix.
Those Horner steps are emitted by the generic
FRI/PCS verifier arithmetic for polynomial opening, quotient, and batch
consistency checks (`crates/plonky3-recursion/recursion/src/pcs/fri/verifier.rs`
uses `horner_acc_step` in the FRI verifier). On the NPO side, the rows are
almost entirely in-circuit Tip5 permutation checks (`8,081` rows) and
coefficient recompose links (`5,743` rows). The recompose/coeff table is
currently part of the D=2 recursive-verifier construction and is enabled at
`build_composite_l1_verifier_circuit`; removing it would be a new soundness
change, not a serialization or performance tweak.

## Current Native Terminal Size And Runtime Breakdown

The current native terminal fixture certificate uses exhaustive supported-NPO
row checking, not the large two-subproof polynomial NPO payload. The
recursion-crate Tip5 verifier-circuit release measurement is:

| Component | Current bytes | Role |
|---|---:|---|
| Production certificate body | `85,726` bytes / `83.7 KiB` | Typed native terminal proof body |
| Production certificate | `85,948` bytes / `83.9 KiB` | Consensus-facing recursive certificate envelope |
| `primitive_r1cs_proof` / row-product sumcheck | `22,631` bytes | Proves primitive recursive-circuit rows through sparse-R1CS row-product sumcheck and assignment evaluation |
| `npo_exhaustive_proof` | `62,909` bytes | Opens every supported Tip5/recompose NPO callsite against the same assignment oracle and checks each row deterministically |
| Exhaustive hidden Tip5 input payload | `17,402` bytes | Revealed hidden Tip5 lanes needed to recompute hidden-input rows |
| Exhaustive assignment-witness multiproof | `45,507` bytes | Known-index Merkle multiproof binding NPO row witnesses to the assignment oracle |

The release-profile timing from the same run is `prove=1.492s` and
`verify=1.181s`. The standalone exhaustive NPO component measured
`prove=0.162s` and `verify=0.288s`; the standalone R1CS row-product component
measured `prove=0.740s` and `verify=0.515s`.

Source-backed production shape:

- `crates/plonky3-recursion/recursion/src/terminal.rs::TerminalProductionProof`
  serializes a prelude, a `TerminalR1csRowProductSumcheckProof`, and an optional
  `TerminalNpoExhaustiveProof`.
- `prove_terminal_production_goldilocks` builds one production prelude binding
  exactly the assignment root, proves the primitive R1CS component against that
  assignment oracle, and for supported NPO rows proves exhaustive row openings
  against the same assignment oracle.
- `verify_terminal_production_goldilocks` rejects extra prelude commitments,
  verifies the primitive row-product proof, and then verifies every supported
  NPO row with `verify_terminal_npo_exhaustive_goldilocks`.

The terminal profile remains the canonical pure-query 60-bit profile:
`TerminalProofParameters::production_60bit()` uses `log_blowup=4`,
`num_queries=15`, and `query_pow_bits=0`. The exhaustive NPO component itself
does not rely on sampling or terminal query PoW: it checks every supported NPO
row deterministically against verifier-derived callsites and assignment-oracle
openings.

The polynomial/proximity NPO backend remains in tree as a diagnostic and future
hardening track, but it is not the production wire artifact. The current
diagnostic measurements explain why it was removed from production:

| Diagnostic candidate | Bytes | Meaning |
|---|---:|---|
| Previous polynomial production certificate | `226,248` bytes / `220.9 KiB` | Failed the hard size gate |
| Previous `TerminalProductionNpoPolynomialProof` | `204,039` bytes | Dominated the old body |
| Previous `merged_value_bridge_proof` | `67,133` bytes | One independent FRI-backed NPO subproof |
| Previous `integrated_logup_proof` | `136,906` bytes | Second independent FRI-backed NPO subproof |
| Full NPO polynomial FRI opening candidate | `48,803` bytes / `47.7 KiB` | A single FRI opening over 668 rows and 186 field columns is far smaller than the old two-subproof NPO body |
| NPO value-column FRI candidate | `30,325` bytes / `29.6 KiB` | Value-only proximity proof is not the blocker by itself |

Engineering conclusion: exhaustive supported-NPO checking made the terminal
fixture small enough. It has not yet made the full `ai-pow-zk` composite
recursive path small or fast enough. Verifier-key reconstruction is now
canonical for the baseline diagnostic, and the `lb=6,nq=10,pow=0` measurement
has complete wire bytes and release timing. The remaining work is to reduce the
actual composite L1 terminal relation, especially the generic FRI/MMCS verifier
circuit and supported-NPO callsite count.

## Soundness Summary

| Layer | Parameters | Soundness claim | PoW counted? |
|---|---|---:|---|
| Layer-0 composite STARK | `log_blowup=4`, `num_queries=15`, `pow_bits=0` in `CircuitConfig::PROD` | 60 pure FRI-query bits under the code's Johnson accounting | No |
| Hardened L1 batch-STARK checkpoint | `log_blowup=4`, `num_queries=9`, `query_pow_bits=24`, `cap_height=5` | 60 bits under mixed query/PoW Johnson accounting | Yes; acceptable for the checkpoint, not for the terminal production target |
| Production native terminal backend | `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`, `max_log_arity=3`, `log_final_poly_len=0` | Intended 60 pure FRI-query bits for the terminal backend, conditionally on the selected Plonky3 FRI theorem/assumption and terminal theorem; fixture proof-size gate passes, but the full `ai-pow-zk` composite-verifier path has not met the release-time gate | No |
| End-to-end production recursive certificate target | L0 proof accepted by the native terminal recursive-verifier certificate | At most the minimum of the L0 and terminal layers: **60 bits** | No terminal query PoW counted |

The recursive certificate does not make the underlying Layer-0 statement more
sound. It replaces the large Layer-0 proof object with a smaller proof that the
recursive verifier accepted that Layer-0 proof. A successful production forgery
must either forge the Layer-0 STARK statement, forge the native terminal proof
that the verifier accepted it, or break one of the transcript/commitment
assumptions that bind the two. The hardened L1 batch-STARK path has its own
60-bit checkpoint claim but is not the production size/time target.

## Logic Flow

```text
AI-PoW attempt data
  |
  | native puzzle checks derive nonce-bound kappa, matrix commitments,
  | noised matmul values, cumsum, jackpot message, and jackpot hash
  v
Layer-0 composite STARK
  proves:
    - canonical program is pinned, not prover-selected;
    - public inputs are bound:
      cumsum, jackpot, HASH_A, HASH_B, JOB_KEY, COMMITMENT_HASH,
      HASH_JACKPOT;
    - BLAKE3 matrix/jackpot hash AIR rows match the public commitments;
    - noised matrix/matmul/range/i8-u8/cv routing lookups are globally
      consistent through LogUp;
    - FRI openings prove low-degree trace/quotient consistency.
  |
  | recursive verifier circuit runs the Layer-0 verifier
  v
Production native terminal certificate target
  proves:
    - the verifier circuit was executed with the committed Layer-0 proof,
      public inputs, relation profile, and production parameters;
    - terminal proof parameters are the canonical pure-query 60-bit tuple;
    - the terminal header, public-values digest, relation digest, proximity
      schedule, fixed terminal tables, and commitments are bound before
      terminal challenges;
    - primitive recursive-circuit rows and supported Tip5/recompose NPO rows
      are covered by the terminal primitive sparse-R1CS row-product proof and
      exhaustive supported-NPO row openings against the assignment oracle.
  |
  v
Nockchain block/wire artifact target: structured native terminal certificate
```

The hardened L1 batch-STARK checkpoint follows the same Layer-0 verifier-circuit
handoff, but proves the executed verifier circuit with `BatchStarkProver`
instead of the native terminal backend. It is useful for regression and
fallback validation, not for the production wire budget.

## Assumptions By Step

### 1. Native AI-PoW Attempt And Public Statement

Cryptographic assumptions:

- BLAKE3 behaves as a collision-resistant hash and keyed hash/MAC for matrix
  commitments and jackpot hashing.
- The nonce/ticket attempt state is unique: changing the nonce, ticket, matrix,
  noise, target, or public commitments changes the derived statement before
  proof construction.
- The verifier recomputes the public statement instead of trusting
  prover-supplied metadata.
- In the Pearl merge-mining path, cheap noun metadata precheck re-derives and
  compares the Pearl-bound slots (`HASH_A`, `HASH_B`, `JOB_KEY`,
  `COMMITMENT_HASH`, `JACKPOT`, `HASH_JACKPOT`) from the ticket and trusted
  matrices. It does not independently derive `cumsum`; `cumsum` remains bound
  by the Layer-0 proof and by full recursive verification of the exact public
  input vector carried in the certificate.

Citations and anchors:

- BLAKE3 specification: O'Connor, Aumasson, Neves, Wilcox-O'Hearn,
  "BLAKE3: one function, fast everywhere",
  <https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf>.
- Current statement-binding docs:
  `2026-05-31_AI_POW_ONE_MATMUL_ONE_ATTEMPT_AUDIT.md` and
  `crates/ai-pow/src/zk_bridge.rs`.

### 2. Layer-0 Composite STARK

Cryptographic assumptions:

- The Plonky3 STARK reduction is sound for the committed AIR, public inputs,
  LogUp buses, and quotient identities.
- FRI proximity/opening verification is sound for the production Goldilocks
  rate and 15 transcript-derived queries.
- Fiat-Shamir challenges are modeled as random-oracle challenges derived after
  all relevant statement data and commitments are bound.
- The Tip5 Merkle/MMCS commitment is binding/collision-resistant for the
  committed trace, quotient, and lookup columns.
- LogUp rational-sum identities are sound except for standard
  Schwartz-Zippel/denominator-pole failure probabilities over the extension
  challenge field.

Implementation anchors:

- `crates/ai-pow-zk/src/composite_proof.rs` documents the production Layer-0
  family: `composite_prove_pinned_logup` /
  `composite_verify_pow_pinned_logup`.
- `crates/ai-pow-zk/src/circuit.rs::CircuitConfig::PROD` sets
  `log_blowup=4`, `num_queries=15`, and `pow_bits=0`.
- `crates/ai-pow-zk/README.md` records the current Layer-0 soundness policy:
  60 pure query bits with no proof-system PoW grinding counted or enabled.

Citations:

- STARKs: Ben-Sasson, Bentov, Horesh, Riabzev, "Scalable, transparent, and
  post-quantum secure computational integrity", IACR ePrint 2018/046,
  <https://eprint.iacr.org/2018/046.pdf>.
- FRI: Ben-Sasson, Bentov, Horesh, Riabzev, "Fast Reed-Solomon Interactive
  Oracle Proofs of Proximity", ICALP 2018,
  <https://doi.org/10.4230/LIPIcs.ICALP.2018.14>.
- DEEP-FRI context: Ben-Sasson, Goldberg, Kopparty, Saraf, IACR ePrint
  2019/336, <https://eprint.iacr.org/2019/336>.
- Fiat-Shamir for FRI and batched FRI: Block, Garreta, Katz, Thaler, Tiwari,
  Zajac, IACR ePrint 2023/1071, <https://eprint.iacr.org/2023/1071>.
- LogUp/logarithmic-derivative lookups: Haboeck, IACR ePrint 2022/1530,
  <https://eprint.iacr.org/2022/1530>.
- Tip5: Szepieniec, Lemmens, Sauer, Threadbare, Al Kindi, "The Tip5 Hash
  Function for Recursive STARKs", IACR ePrint 2023/107,
  <https://eprint.iacr.org/2023/107.pdf>.
- Reed-Solomon proximity-gap policy anchor used by current repo docs:
  Ben-Sasson, Carmon, Haboeck, Kopparty, Saraf, "On Proximity Gaps for
  Reed-Solomon Codes", IACR ePrint 2025/2055,
  <https://eprint.iacr.org/2025/2055>.

### 3. Recursive Verifier Circuit

Cryptographic assumptions:

- The recursive verifier circuit faithfully implements the native Layer-0
  verifier transcript, commitment observations, FRI query derivation, Merkle
  path checks, LogUp checks, and public-input binding.
- The in-circuit 5-round Tip5 operations match the native 5-round Tip5
  permutation used by Layer-0 challenger/MMCS commitments.
- The recursive statement binds the same Layer-0 public-input vector and
  relation/profile metadata that the native verifier would use.

Implementation anchors:

- `crates/ai-pow-zk/src/recursion.rs::recurse_composite_to_l1` defines the
  batch-STARK checkpoint pipeline: prove Layer 0, build the L1 verifier
  circuit, verify Layer 0 in-circuit, and prove the verifier circuit.
- `crates/plonky3-recursion/recursion/src/terminal.rs` defines the native
  terminal compiler/certificate interface for the production recursive target.
- `crates/plonky3-recursion/recursion/src/pcs/fri/params.rs` requires the safe
  `with_mmcs` constructor for production FRI verification; the arithmetic-only
  path is explicitly unsafe/test-only.

Citations:

- Plonky3 recursion model:
  <https://plonky3.github.io/Plonky3-recursion/introduction.html>.
- Tip5 paper as above.
- FRI and Fiat-Shamir-for-FRI references as above.

### 4. Native Terminal Recursive Certificate Target

Cryptographic assumptions:

- The production terminal proof uses the canonical pure-query tuple
  `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`; no terminal query PoW
  is counted toward the 60-bit floor.
- The terminal header, public-values digest, backend relation digest, NPO
  polynomial profile, column layout, fixed Tip5 lookup preprocessed-table
  digest, prelude parameters, relation profile, proximity profile, and backend
  commitment roots are absorbed before terminal challenges.
- Primitive circuit constraints are checked through the sparse-R1CS row-product
  sumcheck plus assignment evaluation proof.
- Supported Tip5/recompose NPO rows are checked exhaustively: verifier-derived
  callsites determine the exact assignment-witness openings, hidden Tip5 lanes,
  MMCS direction bits, row modes, and predecessor-chain semantics to verify.
- Terminal FRI/PCS openings for primitive row-product sumcheck and 5-round Tip5
  Merkle commitments are binding under the stated Plonky3 FRI and Tip5
  assumptions.

Implementation anchors:

- `docs/ai-pow-integration/2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`
  records the production terminal interface and theorem. Its previous
  polynomial-NPO checkpoint was too large, but the current exhaustive-NPO
  recursion-crate fixture measurement is `85,948` bytes / `83.9 KiB`.
- `crates/plonky3-recursion/recursion/src/terminal.rs` implements
  `TerminalCertificate`, `TerminalProofParameters::production_60bit`,
  `prove_terminal_production_goldilocks`, and
  `verify_terminal_production_goldilocks`.
- `crates/ai-pow-zk/src/recursion.rs` now implements the opt-in
  `AiPowTerminalRecursiveCertificate` path for the actual composite L1 verifier
  circuit. Its `lb=6,nq=10,pow=0` release/native measurement verifies after
  postcard decode, but measures `771,249` wire bytes with `80.377s` prove time
  and `58.496s` verify time.
- `crates/plonky3-recursion/recursion/src/terminal.rs` stores known-index
  multiproof field limbs and frontier digests in fixed little-endian bytes,
  reducing the same full-path wire measurement from `891,780` bytes to
  `771,249` bytes without changing the checked Merkle roots or assignment
  values.
- `crates/plonky3-recursion/recursion/src/verifier/batch_stark.rs` emits global
  lookup cumulative checks in sorted order so terminal verifier-key
  reconstruction is deterministic and bound by a stable relation digest.
- Terminal production tests reject malformed proof bodies, wrong proof kind,
  noncanonical parameters, missing commitments, missing exhaustive assignment
  openings, tampered hidden Tip5 input payloads, and tampered assignment
  witness multiproofs. The recursion-crate Tip5 verifier-circuit fixture test
  passes the hard size gate at `85,948` bytes / `83.9 KiB`.

Citations:

- FRI, Fiat-Shamir-for-FRI, LogUp, and Tip5 citations are the same as the
  Layer-0 section because the terminal backend uses these same families of
  assumptions.
- The terminal-specific soundness theorem is documented in
  `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`.

### 5. Hardened L1 Batch-STARK Checkpoint

Cryptographic assumptions:

- The L1 circuit-prover batch-STARK verifies against the submitted proof body,
  not only proof-carried metadata.
- The verifier rebuilds the canonical L1 verifier circuit from the embedded
  Layer-0 proof, pinned program, production profile, and verifier-derived
  public inputs, then rejects if submitted L1 metadata differs from the rebuilt
  canonical shape.
- The verifier registers only the production Tip5 and recompose non-primitive
  tables before calling `verify_all_tables`.
- The L1 FRI/PCS proof is sound under the active mixed query/PoW checkpoint
  profile.

Implementation anchors:

- `crates/ai-pow-zk/src/recursion.rs::verify_recursive_certificate` rebuilds
  and runs the L1 verifier circuit, compares stable metadata against a
  canonical rebuilt proof, and verifies the submitted outer proof with
  `BatchStarkProver::verify_all_tables`.
- `crates/plonky3-recursion/circuit-prover/src/batch_stark_prover.rs` defines
  `verify_all_tables` and `verify_all_tables_with_public_values`.
- `crates/ai-pow-zk/src/recursion.rs::recursive_certificate_rejects_outer_proof_body_tamper`
  is the regression test for metadata-preserving proof-body tampering.

This checkpoint is cryptographically relevant and should remain sound, but it
does not satisfy the production wire budget.

## What Is Not Being Assumed

- No trusted setup.
- No KZG, pairing-friendly curve, Groth16, or Plonkish SNARK wrapper.
- No Plonky2 proof system in production. Pearl/Plonky2 code was read only as a
  design reference for safe FRI path compression, and the native terminal
  backend is implemented in the vendored Plonky3-recursion stack.
- No claim that the multi-hundred-KiB-to-MiB L1 batch-STARK checkpoint is the production
  block/wire artifact.
- No zero-knowledge claim for the production native terminal certificate.
  Exhaustive NPO openings reveal selected recursive-verifier witness material,
  including hidden Tip5 input lanes needed to recompute hidden-input rows.

## Clear End-To-End Claim

For the intended production path, the block-facing recursive proof target is
still the native terminal certificate, not the batch-STARK checkpoint. However,
the full end-to-end production claim is not yet proven. The recursion-crate
Tip5 verifier fixture satisfies the hard constraint at **85,948 bytes /
83.9 KiB** with release prove **1.492 s** and verify **1.181 s**, but the
newly wired full `ai-pow-zk` composite-verifier terminal path measures
**771,249 bytes / 753.2 KiB** on the postcard wire with release prove
**80.377 s** and verify **58.496 s** even under the reduced
`lb=6,nq=10,pow=0` profile. The materialized Layer-0 proof is **304,048 bytes /
296.9 KiB**, but it is an intermediate diagnostic artifact rather than the
target consensus wire object.

The hardened batch-STARK L1 proof body is **149.1 KiB**, and the full checkpoint
certificate is **1,135.5 KiB** as legacy postcard, **5,794.7 KiB** as fixed-int
bincode, and **358.3 KiB** with gzip-best compression. It is retained as a
soundness-hardened checkpoint and fallback verifier target, not as the
production recursive certificate path.

The end-to-end soundness floor is **60 bits**, with the following reduction:

1. If the AI-PoW computation/public statement is false, a valid Layer-0 proof
   requires breaking the Layer-0 STARK/FRI/LogUp/Tip5/BLAKE3 assumptions or
   exploiting a bug in the AIR/public-input binding.
2. If the Layer-0 verifier would reject, a valid production terminal
   certificate requires breaking the terminal primitive sparse-R1CS,
   exhaustive supported-NPO assignment-oracle openings, FRI/PCS, Tip5
   commitment, or transcript-binding assumptions, or exploiting a bug in the
   recursive-verifier relation.
3. The certificate binds public values, relation/profile metadata, commitments,
   and production parameters before challenge derivation, so there is no
   intended grinding surface over public values, profiles, roots, or query
   indices.

The intended weakest production soundness term remains the 60-bit floor shared
by the Layer-0 proof and native terminal certificate. The open engineering
work is to make the actual composite-verifier terminal proof meet the
production size/time gates and to wire that artifact above `ai-pow-zk` without
falling back to the too-large batch-STARK checkpoint. The hardened batch-STARK
path is also required to stay cryptographically sound, but it is not the
production size/time path.
