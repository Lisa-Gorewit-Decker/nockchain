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
| Layer-0 composite proof | Regular STARK proof of the AI-PoW puzzle statement; consumed by recursion; diagnostic/intermediate, not persisted by consensus | `303,896` bytes / `296.8 KiB` | `2026-05-29_AI_ZKP_NOUN_WIRE_SPEC.md`, `prod_recursion_measure 15` |
| Hardened L1 batch-STARK recursive certificate | Soundness-hardened recursive verifier checkpoint/fallback path; not acceptable as the production wire artifact because it exceeds the size budget | `205,446` bytes / `200.6 KiB` fixed-int bincode (`231,235` bytes / `225.8 KiB` legacy postcard) | `prod_recursion_measure 15` |
| Native terminal certificate fixture | Recursion-crate terminal proof over the real Tip5 verifier-circuit fixture; proves the terminal backend can be small, but is not yet the full `ai-pow-zk` composite verifier path | `85,948` bytes / `83.9 KiB`; release prove `1.492s`, verify `1.181s` | `RUSTFLAGS="-C target-cpu=native" cargo test --manifest-path crates/plonky3-recursion/recursion/Cargo.toml --release --test test_l1_outer_cert_tip5_unified terminal_production_certificate_measures_real_tip5_l0_verifier_circuit -- --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier native terminal path | Newly wired opt-in diagnostic path; not yet production-qualified | Release/native run of `terminal_recursive_certificate_round_trip_verifies` was stopped after more than two minutes without completing the proof, so no production size/time claim is valid yet | 2026-06-05 follow-up integration run |

The active production target is therefore:

- regular Layer-0 proof: **296.8 KiB** if materialized;
- hardened batch-STARK L1 checkpoint: **200.6 KiB**, soundness-relevant but too
  large for production wire use;
- native terminal recursive fixture: **85,948 bytes / 83.9 KiB**, satisfying
  the about-100 KiB and `<30s` release-proving gates for the recursion-crate
  Tip5 verifier fixture;
- full `ai-pow-zk` composite-verifier terminal integration: now has an opt-in
  API and measurement test, but the first release/native run exceeded two
  minutes before producing a proof, so this path has not met the production
  `<30s` gate and cannot yet be called the production recursive artifact.

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
   as an opt-in diagnostic path. It has not yet met the production time gate.

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
| L0 composite prove | `32.29s` | Proves a `2^15` row, `1917` column composite AIR with `log_blowup=4`; the main-trace LDE alone is about `7.5 GiB` before permutation, quotient, and Merkle-tree allocations |
| L1 verifier-circuit build | `0.50s` | Builds the recursive verifier circuit and allocates targets; not the bottleneck |
| L1 in-circuit verify | `0.07s` | Runs the verifier circuit once to produce traces; not the bottleneck |
| L1 outer certificate prove + verify | `28.69s` | Proves the verifier-circuit traces with `BatchStarkProver::prove_all_tables` using the production Tip5/recompose NPO tables and FRI profile |
| End-to-end trace-to-recursive-proof time | `61.55s` | Mostly L0 proving plus the L1 batch-STARK outer proof |
| Recursive-only time after L0 proof exists | `29.26s` | Almost entirely the L1 outer batch-STARK proof |

The size bottleneck in that same batch-STARK checkpoint is not top-level noun
or serde framing. It is FRI opening material in the L1 outer proof:

| Component | Measured size |
|---|---:|
| Fixed-int batch-STARK checkpoint certificate | `205,446` bytes / `200.6 KiB` |
| Legacy postcard checkpoint certificate | `231,235` bytes / `225.8 KiB` |
| L1 proof commitments | `4.5 KiB` |
| L1 opened values | `49.5 KiB` |
| L1 opening proof | `163.5 KiB` |
| L1 global lookup data | `6.8 KiB` |

The opening proof dominates because the outer proof is a conventional
multi-table batch-STARK over the executed verifier circuit. The active
`q=9`, `cap_height=5` shape still carries 10 independent Merkle auth-path tree
groups and 819 raw authentication siblings. A direct path-compression model
saved only `1.2 KiB` on average and `6.8 KiB` in the best sampled case, leaving
an estimated fixed-int floor around `199.4 KiB`. Reducing queries to `q=8` with
more query PoW got the checkpoint to `185.6 KiB`, but the L1 outer stage rose
to `61.90s`; it also violates the production terminal policy of not counting
query PoW toward the 60-bit floor.

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
the terminal public input vector alongside the terminal certificate. The first
release/native run of the opt-in
`terminal_recursive_certificate_round_trip_verifies` measurement exceeded two
minutes before printing a proof size. That is the current blocker: the terminal
backend is small on the recursion fixture, but the full composite verifier path
is not yet proven under the `<30s` production gate. The higher-level miner noun
path also still serializes the batch-STARK checkpoint object.

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
recursive path fast enough. The remaining work is to reduce the actual
composite L1 terminal relation, make verifier-key reconstruction canonical
without carrying the Layer-0 proof, and then re-run the opt-in terminal
measurement to obtain complete wire bytes and release timing.

## Soundness Summary

| Layer | Parameters | Soundness claim | PoW counted? |
|---|---|---:|---|
| Layer-0 composite STARK | `log_blowup=4`, `num_queries=15`, `pow_bits=1` in `CircuitConfig::PROD` | 60 pure FRI-query bits, 62 bits under the code's Johnson accounting including the two one-bit PoW hooks | No PoW is needed to reach 60 bits; the two bits are extra margin |
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
  `log_blowup=4`, `num_queries=15`, and `pow_bits=1`.
- `crates/ai-pow-zk/README.md` records the current Layer-0 soundness policy:
  60 pure query bits, 62 under the code's Johnson accounting with the two
  one-bit PoW hooks.

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
  circuit. Its release/native measurement has not yet completed inside the
  production time gate.
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
- No claim that the 200.6 KiB L1 batch-STARK checkpoint is the production
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
newly wired full `ai-pow-zk` composite-verifier terminal path exceeded two
minutes in release/native measurement before completing. The materialized
Layer-0 proof is **303,896 bytes / 296.8 KiB**, but it is an intermediate
diagnostic artifact rather than the target consensus wire object.

The hardened batch-STARK L1 certificate is **205,446 bytes / 200.6 KiB** under
the fixed-int helper. It is retained as a soundness-hardened checkpoint and
fallback verifier target, not as the production recursive certificate path.

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
