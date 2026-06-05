# Native Terminal Recursive Proof Reduction Directions

Date: 2026-06-05
Status: decision checkpoint, revised after stack-level integration audit. The
exhaustive-NPO terminal fixture passes the byte and time gates, but the full
`ai-pow-zk` composite-verifier terminal path has not yet met the production
time gate.

## Goal

The production recursive proof target is the native terminal certificate, not
the batch-STARK L1 checkpoint. The hard target is approximately `100 KiB` for
the recursive proof and `<30s` release proving time, without letting a miner
skip the AI-PoW matrix-multiplication work and without relying on an
undocumented or unproven soundness shortcut.

The current recursion-crate Tip5 verifier-circuit terminal measurement passes
both targets in release mode:

| Item | Measurement |
|---|---:|
| Terminal certificate body | `85,726` bytes / `83.7 KiB` |
| Terminal certificate | `85,948` bytes / `83.9 KiB` |
| Prove time | `1.492s` in release with `RUSTFLAGS="-C target-cpu=native"` |
| Verify time | `1.181s` |
| Required terminal profile | `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`, `max_log_arity=3`, `log_final_poly_len=0` |

The stack-level follow-up added
`crates/ai-pow-zk/src/recursion.rs::prove_terminal_certificate_from_chain_verified_composite_proof`
for the actual composite L1 verifier circuit. Its first release/native
opt-in run of `terminal_recursive_certificate_round_trip_verifies` was stopped
after more than two minutes without completing the proof. Therefore the
fixture measurement is evidence that the backend can be small, not proof that
the full AI-PoW production recursive artifact already satisfies the `<30s`
gate.

The retired polynomial NPO production candidate remains useful diagnostic
evidence. Its size blocker was precise:

| Component | Bytes | Notes |
|---|---:|---|
| Primitive R1CS row-product proof | `21,709` in the production body | Not the size blocker |
| `TerminalProductionNpoPolynomialProof` | `204,039` | Dominates the body |
| `merged_value_bridge_proof` | `67,133` | FRI proof for residual-zero/recompose/value bridge |
| `integrated_logup_proof` | `136,906` | FRI proof for Tip5 AIR, byte LogUp, and selected-vs-trace NPO-IO LogUp |

The important consequence was that generic serialization compression or small
primitive-R1CS tweaks could not make the polynomial NPO payload production
sized. The viable near-term path was to re-audit and promote exhaustive
supported-NPO checking.

## Current Pipeline

The production terminal entrypoint is
`crates/plonky3-recursion/recursion/src/terminal.rs::prove_terminal_production_goldilocks`.
For a verifier key with supported Tip5/recompose NPO rows it does the following:

1. Validate the canonical terminal production parameters.
2. Verify the full terminal assignment and all registered NPO traces with
   `verify_assignment_with_goldilocks_npos`.
3. Commit the assignment oracle.
4. Build one production prelude binding exactly the assignment root.
5. Prove the primitive sparse-R1CS row-product component.
6. If the verifier key has supported NPO rows, prove
   `TerminalNpoExhaustiveProof`, which opens every verifier-derived
   Tip5/recompose callsite against the same assignment oracle.

Verification rejects extra production prelude commitments, verifies the
primitive row-product proof against the assignment root, then verifies every
supported NPO row deterministically. There is no sampled NPO validity path and
no terminal query PoW counted for NPO checking.

The latest measurement also printed useful comparison floors:

| Candidate | Bytes | Interpretation |
|---|---:|---|
| Full NPO polynomial FRI opening candidate | `48,803` | A single opening over 668 rows and 186 field columns is much smaller than the current two-subproof NPO body |
| NPO value-column FRI candidate | `30,325` | Value columns alone are not expensive enough to explain the current size |
| Sparse R1CS matrix sumcheck | `20,873` | Primitive matrix component is already small enough |
| R1CS row-product sumcheck | `22,631` | Assignment fold openings dominate this component, but it is not the main target |

## Direction 1: Unified Production NPO FRI/IOP

Build one production NPO proof that combines the current
`merged_value_bridge_proof` and `integrated_logup_proof` into a single
FRI-backed argument.

The unified proof would commit/open, under one transcript and one terminal FRI
proof, the matrices currently split across the two NPO subproofs:

- selected NPO row-domain table plus selected lookup IO;
- residual-zero composition;
- recompose quotient;
- NPO-row value bridge quotient;
- full Tip5 lookup trace plus masked trace-domain NPO-IO projection;
- Tip5 AIR quotient;
- byte-table LogUp accumulator and quotient;
- selected-domain and trace-domain NPO-IO LogUp accumulators and quotients.

The current verifier already enforces the most important cross-proof binding:
the selected+lookup commitment must match between the two subproofs. A unified
proof would make that equality structural, then share the FRI query set,
opening point, authentication paths, and transcript across all NPO identities.

Why this can hit the target:

- The production NPO proof is currently `204,039` bytes because two independent
  FRI payloads are serialized.
- The old NPO-only integrated checkpoint measured around `94,016` bytes /
  `91.8 KiB`, but it was not the current full production proof body.
- The full production certificate can tolerate roughly `75-78 KiB` of NPO
  payload after the primitive R1CS component and certificate framing. A unified
  proof must therefore cut the NPO payload by about `125 KiB`.
- Prior component measurements show that opening payload sharing, not primitive
  R1CS compression, is the meaningful lever.

Soundness obligations:

- All profiles and commitments must be absorbed before challenges.
- Residual-zero, recompose, value-bridge, Tip5 AIR, byte LogUp, and NPO-IO
  LogUp challenges must have explicit domain separation and an ordering proof
  in the written theorem.
- The verifier must recompute every profile from the verifying key and reject
  any proof-carried profile mismatch.
- The unified proof must preserve the hidden-output masking rule for Merkle
  Tip5 rows; deriving all NPO IO directly from the full trace was already
  rejected as unsound for hidden-output rows.
- The proof must still reject stale value columns, stale selected+lookup
  roots, stale trace roots, forged trace-domain NPO IO, and recompose quotient
  tampering.

Implementation sketch:

1. Add a new `TerminalProductionUnifiedNpoProof` struct with one
   `TerminalCompressedFriProof`.
2. Move the quotient/accumulator matrix construction currently split between
   the two subproof provers into one builder that returns all domains and
   matrices.
3. Seed one challenger with the production prelude, all verifier-derived
   profiles, and the staged commitments.
4. Sample the same relation challenges currently used by both subproofs, but
   from one transcript.
5. Open all committed matrices with one `Pcs::open`.
6. Verify all identities from the same opened values.
7. Keep the old two-subproof verifier as a regression/fallback until the
   unified proof has equivalent tamper tests.

Tests required before promotion:

- Honest real Tip5 L0 verifier-circuit production measurement.
- Body and certificate size assertions at or below the target.
- Tamper tests for each identity: residual-zero, recompose, value bridge, AIR
  quotient, byte LogUp, selected NPO-IO LogUp, trace NPO-IO LogUp.
- Cross-binding tests that swap selected+lookup, trace, accumulator, quotient,
  and final-cumulative data between independently generated proofs.
- Hidden-output Merkle Tip5 test proving that unmasked full-trace IO cannot
  satisfy the selected-vs-trace bridge.
- Noncanonical terminal parameter and proximity-profile rejection.

Assessment: this is the main cryptographically clean direction. It is also the
largest implementation, but it directly targets the measured size culprit.

## Direction 2: Re-Audit The Exhaustive NPO Terminal Proof As A Production Fallback

This direction was promoted for the recursion-crate terminal fixture, then
re-opened at the stack level. The current exhaustive supported-NPO terminal
fixture proof measures below the size target:

| Component | Historical measurement |
|---|---:|
| Primitive R1CS row-product proof | `22,631` bytes |
| Exhaustive NPO proof | `62,909` bytes |
| Compact production certificate | `85,948` bytes |
| Prove / verify | `1.492s` / `1.181s` in release measurement |

This route does not try to make the current polynomial NPO proof smaller.
Instead, the native terminal candidate uses exhaustive NPO row checking and
keeps the polynomial backend as a diagnostic/future hardening track.

Why it might work:

- The historical fixture measurement met both size and time targets.
- It checked every supported Tip5/recompose NPO row rather than sampling NPO
  rows.
- It avoided the current two-FRI-subproof duplication.

Why it was retired:

- The docs describe it as a checkpoint that still needed replacement by a
  final polynomial/proximity backend.
- Its soundness theorem is not currently the active production theorem.
- It serializes deterministic hidden-input and assignment-witness opening
  material and may reveal more witness data.
- It relies on exhaustive Merkle openings to an assignment oracle rather than a
  low-degree/proximity proof over NPO tables.

The key question is not whether it is smaller; it is whether the proof is
cryptographically sound for the terminal relation we need. A re-audit should
answer:

- Does primitive sparse-R1CS row-product plus exhaustive NPO row checking cover
  every operation in the recursive verifier circuit?
- Are all NPO callsites, row modes, hidden Tip5 lanes, Merkle direction bits,
  and recompose rows bound by the backend relation digest?
- Can a malicious prover choose an assignment that satisfies primitive R1CS and
  all exhaustive NPO row checks while representing a false Layer-0 verifier
  execution?
- Are the assignment Merkle commitments, derived known-index openings, hidden
  input payloads, and public prefix bound before all challenges?
- Does this route require zero knowledge? If not, is witness leakage acceptable
  for AI-PoW?

Implementation result:

1. `TerminalProductionProof` now carries optional `TerminalNpoExhaustiveProof`
   instead of the two-subproof polynomial NPO payload.
2. The production prelude binds exactly the assignment root; extra roots are
   rejected.
3. The recursion-crate Tip5 verifier-circuit release measurement passes at
   `85,948` bytes / `83.9 KiB`, `prove=1.492s`, `verify=1.181s`.
4. Focused production tests reject missing exhaustive assignment-opening
   material, tampered hidden Tip5 inputs, tampered assignment-witness Merkle
   frontier material, and recompose-row witness tampering.
5. The `ai-pow-zk` composite L1 terminal path is wired as an opt-in diagnostic,
   but its first release/native run exceeded two minutes without completing.

Assessment: this is still the preferred production direction, but not yet the
active stack-level production path. Its trade-off is witness exposure: it
reveals selected recursive-verifier witness material, including hidden Tip5
lanes. That is acceptable only if the final terminal certificate is explicitly
not specified as zero-knowledge and the full composite verifier path is reduced
under the production time gate.

## Direction 3: Relation-Specific Projection Instead Of Full Trace Opening

The integrated LogUp proof remains large because it opens a wide Tip5 lookup
trace, even after several successful layout passes. Earlier work already
reduced the lookup trace from a very wide one-row-per-permutation shape to a
row-per-round shape, tuned LogUp grouping, and packed Merkle path digests. The
full-trace opening still dominates many checkpoints.

This direction tries to avoid opening all trace columns needed to evaluate the
Tip5 AIR relation directly at `zeta`. Instead, the prover would commit a
smaller relation-specific projection or composition polynomial that already
folds the required AIR, byte LogUp, terminal IO, and NPO-IO bridge identities.

Why it might work:

- Prior measurements show a single compact composition proof can be much
  smaller than opening all trace columns.
- The current integrated proof spends many bytes on opened trace values, not
  just Merkle authentication paths.
- The row-per-round layout has probably exhausted the easiest trace-width
  reductions.

Soundness risk:

- A prover-supplied composition polynomial is not sound by itself. The verifier
  must still know it was computed from committed trace/value columns.
- If the projection omits hidden-output masking or terminal IO support
  semantics, it can reintroduce the forged trace-domain NPO-IO bug.
- This path needs a written polynomial IOP theorem, not only a smaller
  serialized object.

Implementation sketch:

1. Define the minimal relation projection needed for Tip5 AIR and bridge
   checks.
2. Commit that projection under the same transcript as selected/value columns.
3. Prove, with low-degree quotients or sumcheck-style identities, that the
   projection is derived from the committed trace and selected NPO row data.
4. Open only the projection plus the few trace/value columns needed for the
   derivation proof.

Assessment: promising but theory-heavy. This may become the right long-term
backend if the unified two-subproof merge still lands above the target.

## Direction 4: Runtime Instrumentation And Prover Work Reuse

This direction was useful for diagnosing the old polynomial production path,
but the promoted exhaustive path already satisfies the `<30s` release target.
Keep this work for future polynomial/proximity hardening measurements.

The current production measurement prints total production prove time.
The first runtime-instrumentation pass landed after this analysis. The
production prover now emits tracing spans for the main stages, and it reuses
one `TerminalNpoPolynomialTable` to derive both NPO polynomial columns and the
Tip5 lookup trace. Source inspection still shows remaining repeated work:

- `verify_assignment_with_goldilocks_npos` checks the full assignment before
  proof construction.
- Prelude commitment helpers hash selected+lookup and full-trace+masked-NPO-IO
  matrices before the actual subproof provers commit the same matrix families.
- The integrated LogUp subproof builds several accumulator and quotient
  matrices over extension fields.

Immediate work:

1. Run the real measurement in release mode with `RUSTFLAGS="-C
   target-cpu=native"` and `NOCK_TERMINAL_PROFILE_PROVER=1` to capture
   per-stage close-event timings.
2. Cache selected+lookup matrix, trace bundle matrix, and prelude commitment
   digests across the prelude builder and subproof builders.
3. Avoid recomputing verifier-derived columns/layout/profile in the hot path
   when the verifier key is unchanged.

Assessment: low soundness risk and likely important for time. It is not enough
for size unless paired with Direction 1, 2, or 3.

## Direction 5: Terminal FRI Parameter Tradeoff

The current terminal policy reaches 60 bits with pure FRI queries:
`log_blowup=4`, `num_queries=15`, and `query_pow_bits=0`. Reducing
`num_queries` would immediately reduce opening/authentication bytes, but the
lost query soundness has to be replaced.

Possible variants:

- Increase blowup and reduce queries. This tends to increase domains and can
  hurt proving time and memory.
- Add terminal query PoW and reduce queries. This can reduce proof bytes but
  changes the terminal soundness accounting and adds grinding work.
- Change FRI arity/final polynomial schedule. Prior sweeps already rejected
  several arity/final-poly variants for this backend, but a unified proof might
  have a different optimum.

Assessment: keep this as a policy-dependent fallback, not the first choice.
The current docs intentionally do not count terminal query PoW toward the 60-bit
floor. If that policy changes, it needs a fresh soundness calculation and a
clear statement that terminal query PoW is part of the production proof-system
security budget.

## Direction 6: Batch-STARK Checkpoint Hardening Only

The batch-STARK L1 checkpoint is now soundness-hardened and should remain so,
but it is not a route to the production proof-size target. Its fixed-int
certificate measurement is about `200.6 KiB`, already above target before
considering the native terminal path. It is still useful for:

- regression testing the recursive verifier relation;
- comparing terminal verifier behavior against a conventional batch-STARK
  wrapper;
- fallback development while terminal proof-shape work continues.

Assessment: keep it sound, but do not spend milestone effort trying to make it
the production certificate unless the hard size target changes.

## Recommendation

I would pursue three tracks in this order:

1. **Keep exhaustive NPO as the leading terminal direction, but do not call it
   fully production-integrated yet.** It is the only current native terminal
   fixture measured below 100 KiB and below 30s, but the actual composite L1
   verifier path still exceeds the time gate.
2. **Continue the unified NPO proof only as hardening/future work.** It would
   reduce witness leakage if it can share one FRI payload and stay under target.
3. **Keep batch-STARK hardened as checkpoint only.** It is soundness-relevant
   but too large for the production recursive certificate.

I would not spend milestone effort on terminal query-PoW parameter changes
unless the pure-query path is conclusively too large after the composite L1
relation is reduced. The current fixture result meets the target without
terminal query PoW, but the full stack does not yet.

## Minimum Promotion Checklist

Any candidate production direction must satisfy all of the following before it
replaces the current terminal production proof:

- Full `ai-pow-zk` composite-verifier terminal measurement at or below about
  `100 KiB`, including terminal public inputs required for verification.
- Release-profile proving time under `30s` on the agreed production machine
  class.
- No terminal query PoW counted unless the production soundness policy is
  explicitly changed and documented.
- Full verifier rejection tests for malformed bodies, noncanonical parameters,
  stale preludes, swapped roots, missing roots, tampered FRI openings,
  residual-zero tampering, recompose tampering, value-bridge tampering, byte
  LogUp tampering, NPO-IO LogUp tampering, hidden-output Merkle Tip5 cases, and
  wrong public values.
- Written soundness theorem that names every binding: public values, terminal
  header, backend relation digest, NPO layout/profile, fixed Tip5 table digest,
  production proximity profile, assignment root, selected/value roots, trace
  roots, accumulator roots, quotient roots, final cumulatives, and FRI query
  derivation.
- No Hoon/kernel verifier acceptance until Rust verifier wiring is explicitly
  in scope and fail-closed behavior is intentionally changed.
