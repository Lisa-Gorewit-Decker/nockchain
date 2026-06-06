# Native Terminal Recursive Proof Reduction Directions

Date: 2026-06-05
Status: decision checkpoint, revised after stack-level integration audit. The
exhaustive-NPO terminal fixture passes the byte and time gates, but the full
`ai-pow-zk` composite-verifier terminal path has not yet met either the
production byte gate or the production time gate.

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
for the actual composite L1 verifier circuit. A completed release/native
reduced-profile measurement verifies after postcard decode, but it is not close
to the hard target:

| Full composite terminal profile | Certificate | Public inputs | Postcard wire | Compile | Prove | Verify |
|---|---:|---:|---:|---:|---:|---:|
| `lb=6,nq=10,pow=0` after compact known-index proof encoding | `766,069` bytes / `748.1 KiB` | `5,180` bytes | `771,249` bytes / `753.2 KiB` | `7.606s` | `80.829s` | `58.825s` |

Therefore the fixture measurement is evidence that the backend can be small on
a much smaller verifier relation, not proof that the full AI-PoW production
recursive artifact already satisfies the byte or time gates.

A non-proving production-profile relation diagnostic now measures the actual
composite L1 terminal relation:

| Metric | PROD baseline |
|---|---:|
| Terminal compile time | `20.943s` |
| Terminal public input bytes | `5,319` |
| Terminal private input values | `43,443` |
| Terminal operations | `125,961` |
| Primitive operations | `106,349` |
| Const operations | `582` |
| Public operations | `459` |
| ALU add operations | `8,832` |
| ALU multiplication operations | `10,234` |
| ALU boolean-check operations | `255` |
| ALU fused multiply-add operations | `10,117` |
| ALU Horner-accumulator operations | `75,870` |
| Supported NPO rows | `14,049` |
| Tip5 rows | `8,081` |
| Recompose/coeff rows | `5,743` |
| NPO input/output callsite slots | `141,232` / `86,778` |
| NPO residual components | `242,798` |
| Circuit fingerprint | `witness=221,989 public=459 private=43,443 ops=125,961` |

This shifts the immediate reduction target. The terminal public input vector is
not the blocker at about `5.3 KiB`; the generic composite verifier relation is.
Any production candidate has to reduce more than `100k` primitive operations,
about `14k` supported NPO rows, and a terminal compile step that already
consumes most of the `<30s` budget before proving starts.

The operation-class breakdown makes the cause more specific. In the production
profile, Horner accumulation accounts for `75,870` of `106,349` primitive
operations. These are verifier-arithmetic steps from the generic FRI/PCS
opening, quotient, and batch-consistency checks, not matrix-multiplication work
or terminal public-input framing. The NPO rows are also concentrated:
`8,081` Tip5 permutation rows and `5,743` `recompose/coeff` rows. The
`recompose/coeff` rows are emitted because
`build_composite_l1_verifier_circuit` enables
`set_recompose_coeff_ctl_for_decompose_links(true)` for the D=2 recursive
verifier. Disabling that table may make a diagnostic smaller, but it is not a
production reduction unless there is a replacement proof that every hinted
extension-field decomposition remains connected to a creator and every affected
WitnessChecks bus entry is sound.

The current `CircuitConfig::PROD` profile is now exactly 60 pure-query bits
(`log_blowup=4`, `num_queries=15`, `pow_bits=0`). Removing the previous
one-bit commit/query proof-system PoW hooks was the right soundness-policy
cleanup, but it only changed the relation from `125,991` to `125,961`
operations. It is not the terminal-size fix.

Pure-query 60-bit Layer-0 profile diagnostics show the real tradeoff:

| L0 profile | Test wall time | Terminal compile | Ops | Horner ops | NPO rows | Assessment |
|---|---:|---:|---:|---:|---:|---|
| `lb=3,nq=20,pow=0` | `32.51s` | `27.692s` | `155,604` | `101,160` | `16,229` | Lower LDE may help L0 proving, but it makes the recursive terminal relation much larger. |
| `lb=4,nq=15,pow=0` | `29.49s` | `20.943s` | `125,961` | `75,870` | `14,049` | Current PROD pure-query baseline. |
| `lb=5,nq=12,pow=0` | `34.05s` | `17.325s` | `108,176` | `60,696` | `12,741` | Meaningful relation reduction, but higher LDE already costs wall time on the tiny baseline. |
| `lb=6,nq=10,pow=0` | `47.60s` | `14.553s` | `96,319` | `50,580` | `11,868` | Best relation reduction measured, but the 64x LDE makes promotion unlikely without separate L0 prover acceleration. |

These are non-proving terminal-relation diagnostics, not production terminal
proof measurements. The test wall time includes Layer-0 proof generation,
L1 verifier construction, and terminal relation compilation for the
`CompositeTrace::baseline_min()` fixture.

The full proof measurement for the most relation-favorable row in the table,
`lb=6,nq=10,pow=0`, still produces a `771,249` byte postcard wire object and
spends `80.829s` in terminal proving. That confirms that simply increasing
Layer-0 blowup to reduce query count is not enough; the terminal relation and
assignment-oracle opening material are still far too large.

That `lb=6,nq=10` row is a lower-bound diagnostic for recursive terminal size,
not a production-profile recommendation. The current PROD baseline remains the
pure-query `lb=4,nq=15,pow=0` inflection point. The `lb=6,nq=10` diagnostic was
chosen because it gives the smallest recursive verifier relation in the
pure-query 60-bit sweep; failing the size/time gates there means the current
terminal proof shape is structurally too large. Its L0 proving cost remains too
high for an unqualified production default.

The polynomial NPO path remains useful diagnostic evidence, but it is not a
drop-in production replacement for the exhaustive NPO proof. The recursion-crate
synthetic Tip5-only integrated-LogUp checkpoint measures below the byte and
time targets:

| Synthetic integrated-LogUp checkpoint | Bytes / Time |
|---|---:|
| Bundled masked-IO NPO checkpoint | `95,403` bytes / `93.2 KiB` |
| Primitive + bundled NPO production-candidate body | `96,219` bytes / `94.0 KiB` |
| NPO prove time | `23.057s` |
| Total primitive + NPO prove time | `23.070s` |
| Total verify time | `64.4ms` |

That test is intentionally small. It proves a synthetic NPO-only Tip5 circuit,
not the full `ai-pow-zk` composite verifier. A full composite diagnostic,
`terminal_integrated_logup_candidate_for_pure_query_lb6_nq10_measures`, now
builds the actual L1 verifier circuit, binds the assignment root plus the
merged NPO and bundled Tip5 roots, proves the primitive row-product component,
and then attempts the integrated polynomial NPO proof. The first release/native
run compiled in `1m57s`, then the test binary ran for more than `7m35s` without
reaching the final size/timing print and was stopped. This already violates the
`<30s` production proving constraint, so the synthetic `94.0 KiB` checkpoint
must not be treated as evidence that the full composite recursive certificate
path meets the milestone.

A second release/native run with phase instrumentation compiled in `1m42s` and
then isolated the full-composite costs before stopping the still-running
integrated Tip5 LogUp subproof:

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

The cumulative recursive-side work before the integrated Tip5 LogUp proof
finishes is already far beyond the production proving budget. The root
construction phases also duplicate work that the current subproof provers do
again internally. Avoiding that duplicate commit/matrix work would be a useful
engineering cleanup, but it cannot make this candidate production-viable by
itself: primitive proving plus the merged value-bridge proof already cost
about `90s`, and the integrated Tip5 LogUp proof had not completed.

The older two-subproof polynomial NPO production candidate had a precise size
blocker:

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

The terminal certificate wrapper now uses postcard encoding and a structural
round-trip assertion for the terminal public inputs plus certificate. The
recursive verifier-key rebuild also has a deterministic-header regression. This
was necessary for soundness: before the fix, the same Layer-0 proof could
rebuild a different terminal relation digest because global lookup cumulative
checks were emitted through hash-map value iteration in the recursive verifier
circuit. The builder now emits those checks in sorted name order, so the
terminal relation digest is a stable cryptographic binding rather than an
artifact of hash iteration order.

The current compact known-index multiproof encoding stores field limbs and
frontier digests as fixed little-endian bytes. This reduced the same
`lb=6,nq=10,pow=0` wire measurement from `891,780` bytes to `771,249` bytes,
but it did not change the structural bottleneck:

| Full composite terminal body component | Bytes |
|---|---:|
| Full production proof body | `765,844` |
| Primitive R1CS row-product proof | `52,821` |
| Exhaustive NPO proof | `712,830` |
| Exhaustive NPO hidden Tip5 values | `92,802` |
| Exhaustive NPO assignment-witness multiproof | `620,028` |

The NPO assignment-witness multiproof still opens `47,814` assignment values
and carries `5,434` Merkle frontier nodes. Ordinary encoding work cannot close
the remaining gap to about `100 KiB`; a production-sized path has to avoid
exhaustively opening this many assignment values.

The latest full measurement decomposes that multiproof further:

| Assignment-witness multiproof component | Bytes / Count |
|---|---:|
| Nonzero value limbs | `382,515` bytes |
| Sparse nonzero masks | `20,126` bytes |
| Boolean bits | `25` bytes |
| Merkle frontier | `217,362` bytes |
| Estimated non-boolean opened values | `80,492` |
| Nonzero coefficients | `47,814` |
| Zero coefficients already elided | `113,170` |

The existing sparse coefficient encoding has already removed about `905 KiB`
of dense zero coefficients. The remaining size is mostly nonzero value limbs
and Merkle authentication data, so further varint/fixed-width encoding tweaks
are not enough.

The latest measurement also printed useful comparison floors:

| Candidate | Bytes | Interpretation |
|---|---:|---|
| Full NPO polynomial FRI opening candidate | `48,803` | A single opening over 668 rows and 186 field columns is much smaller than the current two-subproof NPO body |
| NPO value-column FRI candidate | `30,325` | Value columns alone are not expensive enough to explain the current size |
| Sparse R1CS matrix sumcheck | `20,873` | Primitive matrix component is already small enough |
| R1CS row-product sumcheck | `22,631` | Assignment fold openings dominate this component, but it is not the main target |

## Pearl/Plonky2 Reference: What Actually Makes Its Proof Small

The Pearl implementation is useful evidence that the target size is plausible
with a STARK-family proof, but the mechanism is not "batch-STARK the recursive
verifier harder." In the read-only Pearl checkout, the submitted `ZKProof`
contains only a 22-byte preamble
(`pow_bits[3] | rate_bits[3] | zeta[16]`) plus the final compact Plonky2 proof
bytes (`pearl/zk-pow/src/api/proof.rs` and `proof_utils.rs`). The public proof
data is separate and fixed-size, and verification reconstructs the final proof
public inputs from public params, cached verifier data, and deterministic
preprocessed columns.

The architecture is materially different from the current full
`ai-pow-zk` terminal path:

- Pearl proves the AI-PoW computation with a specialized `PearlStark` AIR, not
  with a generic recursive-verifier terminal relation. The AIR interleaves input,
  Blake3, matmul, and jackpot chips in one trace
  (`pearl/zk-pow/src/circuit/pearl_air.rs`).
- Its "program" side is encoded as preprocessed control/noise/routing columns.
  Starky commits online and preprocessed trace columns in one Merkle oracle,
  absorbs a preprocessed/public-data digest before deriving challenges, and
  recursively connects the preprocessed openings at `zeta` and `g*zeta`
  (`pearl/plonky2/starky/src/prover.rs` and
  `recursive_verifier.rs`).
- The first recursive circuit verifies the base STARK proof and exposes the
  base public inputs, public-data commitment, STARK `zeta`, and preprocessed
  evaluations as public inputs. Verification later recomputes those
  preprocessed evaluations natively from public parameters instead of carrying
  the base STARK proof on the wire
  (`pearl/zk-pow/src/circuit/pearl_circuit.rs`).
- The second recursive circuit verifies the first recursive proof and serializes
  only a compact final proof. Pearl's `CompactProofWithPublicInputs` omits
  deterministic `constants_sigmas` evaluations and Merkle proofs from FRI query
  rounds, then reconstructs them during verification from trusted cached
  polynomial coefficients (`pearl/plonky2/plonky2/src/plonk/proof.rs`).
- Pearl explicitly binds a gap in its second-recursion verifier by exposing the
  first circuit's `constants_sigmas_cap` and `circuit_digest` as public inputs,
  because `builder_2.verify_proof` alone does not prove that the cap is related
  to the digest.

That explains why Pearl does not pay our measured `620,028` byte
assignment-witness multiproof cost. It does not terminalize a generic composite
verifier and then authenticate tens of thousands of verifier-assignment values.
It proves a purpose-built AIR, recursively compresses that proof twice, and
puts only the compact second-recursion proof on the wire.

The exact Pearl parameters are not directly portable to Nockchain's stated
soundness policy. Pearl's defaults are `pow_bits=[18,18,22]` and
`rate_bits=[1 or 2,3,7]`, with `num_query_rounds =
ceil((120 - pow_bits) / rate_bits)` in all three stages
(`pearl/zk-pow/src/circuit/circuit_utils.rs`). Nockchain's current production
policy is a 60-bit pure-query floor with `query_pow_bits=0`. Therefore Pearl's
high proof-system PoW values are useful as an engineering comparison but cannot
be counted toward Nockchain's production soundness unless that policy is
explicitly changed.

## Pearl-Informed Plonky3-Compatible Tracks

The portable lessons are the proof shape and the bindings, not Plonky2 itself.
Viable Nockchain tracks that do not use Plonky2 directly are:

1. **Specialized AI-PoW base AIR plus recursive compression.** Build a
   Plonky3-native AIR for the actual AI-PoW statement, with matrix/noise/hash
   and jackpot constraints directly in the trace and deterministic public data
   represented as preprocessed columns. This is the closest analogue to Pearl
   and avoids the current generic verifier relation before recursion. It is the
   largest AIR implementation, but it attacks both measured blockers: the
   `106,349` primitive terminal operations and the `14,049` supported NPO rows.
2. **Two-stage Plonky3 recursive compressor with compact final serialization.**
   Keep the current Layer-0 proof or a future specialized AIR proof, then add a
   first recursive verifier circuit and a second proof-compression circuit whose
   on-wire proof omits only deterministic verifier-key openings. This requires
   a Plonky3 analogue of Pearl's compact proof format: cached verifier
   polynomials, public binding of verifier digests/caps, strict verifier-key
   reconstruction, and explicit tests that stale cached polynomials, swapped
   caps, wrong circuit digests, and malformed compact openings are rejected.
3. **Preprocessed-program binding instead of assignment-value revelation.**
   Move deterministic verifier/program data out of terminal assignment openings
   and into digest-bound preprocessed columns whose evaluations at verifier
   challenge points are recomputed by the verifier. This is a narrower form of
   the Pearl design and could reduce the current exhaustive NPO multiproof, but
   it is sound only if every omitted value is either deterministically
   reconstructable or still proven derived from committed witness columns.
4. **Unified STARK/IOP for terminal NPO data.** Continue the Direction 1 work,
   but treat Pearl as evidence that the final proof should be one compact
   recursively compressed object rather than two independent terminal FRI
   payloads plus a large assignment-opening proof. The current integrated
   candidate is too slow, so a viable version has to share commitments,
   challenges, and openings structurally and avoid rebuilding the same matrices.
5. **Pure-query parameter search after the proof shape changes.** Pearl's
   `rate_bits=7,pow_bits=22` final stage is small partly because it counts
   proof-system PoW. For Nockchain, parameter sweeps must keep `pow_bits=0`
   unless the soundness policy changes. The useful search space is therefore
   higher-rate/fewer-query pure-query recursion after compact serialization and
   specialized/preprocessed bindings have reduced the relation.

The expected production route is probably a combination of the first two
tracks: prove the AI-PoW statement with a specialized Plonky3 STARK/AIR, then
recursively compress it to one compact final proof. The current native terminal
backend remains valuable as a verifier-relation diagnostic and fallback, but
its full composite path is paying costs that Pearl's architecture avoids
entirely.

### Concrete Pearl-Shaped Plan Without Plonky2

The important conclusion from Pearl is that the target is achievable by changing
the proof shape, not by retuning the current generic terminalized verifier. A
Plonky3-native route should copy the structural ideas below while keeping the
Nockchain soundness policy (`pow_bits=0` for production accounting) and the
existing Tip5/AI-PoW bindings.

| Pearl ingredient | What Pearl gets | Plonky3-native analogue | Production acceptance gate |
|---|---|---|---|
| Specialized base AIR | The base proof is over the work statement, not over a generic verifier execution | Continue from `CompositeFullAirWithLookupsPinned` or replace it with a narrower dedicated AI-PoW AIR; do not put the final artifact on the raw L0 proof | Base proof verifies the matrix/noise/jackpot work, target hit, nonce/job binding, matrix commitments, and public params with no cache-only witness shortcut |
| Two recursive compression layers | The final proof verifies a proof that already verified the base STARK | Build a first Plonky3 recursive verifier circuit for L0, then a second Plonky3 proof-compression layer over that verifier proof | The on-wire final object is only the L2 compact proof plus explicitly required public data, not L0 and not the L1 batch-STARK checkpoint |
| Compact final proof format | Deterministic verifier-key material is cached and reconstructed by the verifier | Add a compact p3 proof format that omits only verifier-deterministic constants/preprocessed openings and reconstructs them from pinned verifier data | Tampering tests reject stale cached polynomials, swapped verifier caps, wrong circuit digest, wrong preprocessed commitment, wrong public inputs, and malformed compact openings |
| Public verifier-data binding | Pearl exposes `constants_sigmas_cap` and `circuit_digest` because `verify_proof` alone is not enough | Every compact p3 recursive layer must expose or otherwise transcript-bind the previous verifier key digest, cap/root, parameter tuple, and public-input digest | The verifier recomputes those values from canonical code/config and rejects any mismatch before accepting proof bytes |
| Preprocessed/program columns | Deterministic program/routing data is digest-bound and re-evaluated by the verifier | Move deterministic AI-PoW program data and verifier data into digest-bound preprocessed columns where possible | Every omitted value is either verifier-recomputable or still opened from a committed witness; no hidden witness value may become a verifier hint |
| High-rate final layer | Pearl's final stage is small partly because it uses high rate and query PoW | Sweep pure-query final-layer parameters only after compact recursion exists | A candidate must meet at least 60 pure-query Johnson bits without counting proof-system PoW |

This plan deliberately separates three concerns that the current terminal path
mixes together:

1. **Base statement soundness.** The L0 proof must prove the AI-PoW work itself:
   matrix commitments, noised matrix strips, selected tile multiplication,
   jackpot hash, target comparison, and all chain/public metadata bindings.
2. **Recursive compression soundness.** L1/L2 proofs must prove verifier
   execution and bind exactly the verifier parameters, verifier key,
   transcript-visible commitments, and public inputs used by the previous
   layer.
3. **Serialization compactness.** Compact encoding may omit deterministic
   verifier data, but it must never omit a witness value unless the verifier can
   recompute it from public data or another proof obligation already binds it.

The first implementation milestone should therefore not be another
`lb=6,nq=10` terminal measurement. It should be a p3-native compression
prototype over the existing pinned+LogUp L0 proof with the following outputs:

- L1 proof size and proving time for verifying the current pinned+LogUp L0
  proof.
- L2/final proof size and proving time for verifying the L1 proof.
- A compact-vs-full serialization split that identifies exactly which bytes
  are omitted and which verifier-known values reconstruct them.
- Negative tests for every omitted binding: stale verifier data, wrong
  preprocessed commitment, wrong public input vector, wrong L0 proof
  commitments, wrong L1 circuit digest, and wrong final proof public inputs.

If that prototype still lands above `100 KiB`, the next lever is AIR
specialization/narrowing, not higher `log_blowup` alone. The current
`lb=6,nq=10,pow=0` row was useful only as a lower-bound diagnostic for the
generic terminal relation. It should not be treated as the production
inflection point: `lb=4,nq=15,pow=0` remains the current pure-query baseline,
while `lb=3,nq=20` and `lb=4,nq=15` are the more plausible L0 proving-time
profiles once recursion and compact serialization remove the raw opening proof
from the wire.

### Relaxed 150 KiB Size Gate

If the production size gate can relax from about `100 KiB` to about `150 KiB`,
the existing batch-STARK L1 checkpoint becomes worth re-evaluating, but only as
a new L1-only certificate shape. It does not become production-ready in its
current envelope.

The production-faithful `prod_recursion_measure 15` run already measured the
raw L1 proof body at `149.1 KiB`, which is close to the relaxed byte target.
However, the current `AiPowRecursiveCertificate` serializes the L0 proof and
program with the L1 proof so that verification can rebuild the exact L1
verifier circuit from the submitted L0 proof and reject proof-carried circuit
metadata substitutions. That is why the full checkpoint certificate remains
`1,135.5 KiB` under legacy postcard and `358.3 KiB` even with gzip-best
compression.

The new diagnostic
`relaxed_l1_only_candidate_size_breakdown_for_test_pearl` measures this split
directly for the small `TEST_PEARL` profile:

| Relaxed L1-only diagnostic (`TEST_PEARL`) | Bytes |
|---|---:|
| Current full checkpoint, postcard | `588,162` |
| Current full checkpoint, fixed-int bincode | `1,981,331` |
| Embedded L0 proof | `262,404` |
| Embedded L0 program | `171,908` |
| Full L1 outer object | `153,850` |
| L1 proof body | `152,205` |
| L1 metadata outside proof body | `1,645` |
| L1 public binding lanes | `0` |

The byte split supports the relaxed-size intuition: once the L0 proof/program
context is removed, the L1 object is approximately at the `150 KiB` target and
almost all of it is the actual cryptographic proof body. The same run took
`75.17s` inside the release test binary, however, so this is not yet a
time-qualified production path.

The follow-up diagnostic
`relaxed_l1_only_statement_bound_candidate_size_breakdown_for_test_pearl`
checks whether binding the statement digest into the L1 proof changes that size
picture. It proves the same `TEST_PEARL` L1 outer object with five public
binding lanes and verifies the proof against the explicit public values:

| Statement-bound L1-only diagnostic (`TEST_PEARL`) | Measurement |
|---|---:|
| Full L1 outer object | `153,904 bytes` |
| L1 proof body | `152,259 bytes` |
| L1 metadata outside proof body | `1,645 bytes` |
| L1 public binding lanes | `5` |
| Prove time | `54.62s` |
| Verify time | `18ms` |

An earlier run of the same diagnostic measured `153,888 bytes` and `70.14s`, so
postcard size has small run-to-run variation and prover time has larger runtime
variation. The larger observed proof adds only `54 bytes` over the unbound
L1-only object, so explicit statement-digest binding is compatible with the
relaxed-size target. It does not solve the remaining soundness contract by
itself: an L1-only production wire format must still pin or reconstruct the
verifier key, L0 proof shape, preprocessed commitment, L1 circuit fingerprint,
table packing, and L1 public values without accepting proof-carried
substitutions. It also does not solve the proof-system soundness policy: the
current recursive prover profile still uses proof-system PoW in addition to
queries, while the production target is 60-bit soundness without relying on
verifier-accepted PoW grinding.

The no-PoW diagnostic
`relaxed_l1_only_pure_query_statement_bound_candidate_size_breakdown_for_test_pearl`
then reruns the statement-bound L1-only object with `commit_pow_bits=0` and
`query_pow_bits=0`. It sweeps the three 60-bit pure-query shapes that are most
relevant to the current parameter discussion:

| Pure-query statement-bound L1-only diagnostic (`TEST_PEARL`) | L1 outer | L1 proof body | Prove | Verify |
|---|---:|---:|---:|---:|
| `lb=4,nq=15,pow=0` | `226,542 bytes` | `224,897 bytes` | `49.290s` | `26ms` |
| `lb=5,nq=12,pow=0` | `196,488 bytes` | `194,845 bytes` | `98.009s` | `22ms` |
| `lb=6,nq=10,pow=0` | `176,362 bytes` | `174,727 bytes` | `195.574s` | `24ms` |

All three variants bind five public lanes, use zero commit/query proof-system
PoW, and reach 60 Johnson bits by `log_blowup * num_queries`. This closes the
parameter-only version of the relaxed L1-only batch-STARK route: removing PoW
from the soundness accounting pushes the proof well above `150 KiB`, and higher
blowup reduces bytes only by spending far more prover time. The smallest
measured pure-query variant is still `172.2 KiB` and takes `195.574s`, so the
batch-STARK L1-only route needs structural compression or another recursive
compression layer before it can satisfy the production policy.

A relaxed-size L1-only path would need to replace those proof-carried rebuild
inputs with a pinned verifier-key contract:

- verifier rebuilds the canonical program from trusted public block/attempt
  data via the params-pure canonical program path, not from proof bytes;
- verifier reconstructs the L0 proof shape from public profile/program/common
  data, or a small canonical proof-shape descriptor, so
  `build_composite_l1_verifier_circuit` no longer needs the whole L0 proof just
  to allocate targets;
- the final certificate binds the statement public-input digest, L0 profile,
  L0 preprocessed commitment, L1 circuit fingerprint, table-packing tuple,
  rows/degrees, non-primitive metadata, and L1 proof public values;
- the current L1 proof has `public_binding_lanes=0`; production must either
  enable public-value binding for the statement digest/public inputs or add an
  equivalent transcript-visible binding with the same rejection tests;
- negative tests reject swapped programs, stale preprocessed commitments,
  wrong statement public inputs, wrong L0 profile, wrong L1 circuit metadata,
  wrong proof public values, and tampered L1 proof body.

This is less invasive than a full Pearl-shaped two-recursion-layer compressor,
but it still requires new verifier-key plumbing. Simply dropping `l0_proof` and
`l0_program` from `AiPowRecursiveCertificate` would be unsound because the
current verifier would no longer have an independent way to know which L1
verifier circuit the submitted outer proof is supposed to prove.

The relaxed size gate also does not solve the time gate. The production-faithful
measurement spent `59.21s` on the L1 outer batch-STARK prove+verify after the
L0 proof already existed, and `93.88s` end to end; the focused
statement-bound `TEST_PEARL` diagnostic spent `54.62s` to `70.14s` proving the
mixed query/PoW L1 outer object across two release runs, and the pure-query
sweep spent `49.290s` to `195.574s` while missing the byte target. The `150 KiB`
branch is therefore a candidate only if the L1 proof can be both
wire-minimized and made materially faster through structural changes. The likely
short-term levers are:

- avoid verifier-side reproving during metadata validation by replacing it with
  deterministic verifier-key reconstruction and direct metadata hashing;
- remove duplicate runner/prover trace materialization in the L1 checkpoint
  path;
- benchmark whether the canonical pinned+LogUp L0 baseline plus L1-only proof
  can meet `<30s` on the actual production trace with release flags after those
  engineering cuts;
- if it cannot, fall back to the Pearl-shaped two-layer compact route or a
  narrower specialized AIR rather than spending more effort on the batch-STARK
  envelope.

### Current Specialized Layer-0 Proof Baseline

The tree now has an ignored Layer-0 pinned+LogUp size diagnostic,
`composite_pinned_logup_*_l0_size_breakdown`, to quantify the proof object that
a Pearl-shaped compressor would consume if we start from the existing
specialized AI-PoW AIR instead of the generic terminal verifier relation. The
diagnostic proves and verifies `CompositeTrace::baseline_min()` with
`composite_prove_pinned_logup`, checks `pow_bits=0`, and prints component
sizes for the proof fields.

Release/native measurements on 2026-06-05:

| Layer-0 pinned+LogUp profile | Prove | Verify | Bincode proof | Bincode opening proof | Bincode opened values | Global lookup data |
|---|---:|---:|---:|---:|---:|---:|
| `lb=4,nq=15,pow=0` | `8.695s` | `0.118s` | `260,987` bytes / `254.9 KiB` | `229,849` bytes | `24,188` bytes | `6,808` bytes |
| `lb=6,nq=10,pow=0` | `32.314s` | `0.381s` | `199,882` bytes / `195.2 KiB` | `168,744` bytes | `24,188` bytes | `6,808` bytes |

Postcard sizes for the same two proofs were `273,043` bytes and `208,726`
bytes. The component split shows that the base proof is still dominated by FRI
opening material. Increasing blowup and reducing queries lowers the base proof
by about `61 KiB`, but it also makes this baseline proof about `3.7x` slower.

Consequences for the Pearl-shaped route:

- Directly serializing the existing Layer-0 proof is not enough; even the
  `lb=6,nq=10,pow=0` diagnostic is about `195 KiB` before any recursive
  certificate framing.
- A production-sized recursive path has to replace the Layer-0 FRI opening
  proof on the wire with a compact recursive proof that verifies it, not merely
  re-encode the Layer-0 proof.
- The existing specialized AIR is a plausible base statement for the
  Pearl-shaped route, but the final compressor must stay pure-query and avoid
  importing Pearl's proof-system PoW accounting.

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

Why this is still the relevant proof-shape direction, but not yet a production
solution:

- The production NPO proof is currently `204,039` bytes because two independent
  FRI payloads are serialized.
- The current NPO-only integrated checkpoint measures `96,219` bytes /
  `94.0 KiB` including the primitive proof on the small synthetic circuit, but
  it is not the full composite production proof body.
- The full composite integrated diagnostic ran for more than `7m35s` after
  compile without reaching its final size print, so this path currently misses
  the proving-time gate even before it can be considered for promotion.
- Phase instrumentation shows that even before the integrated Tip5 LogUp
  subproof finishes, the full composite candidate spends `38.235s` proving the
  primitive component and `51.902s` proving the merged value bridge.
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
5. The `ai-pow-zk` composite L1 terminal path is wired as an opt-in diagnostic.
   Its `lb=6,nq=10,pow=0` release/native run verifies after postcard decode,
   but measures `771,249` wire bytes, `prove=80.829s`, and `verify=58.825s`.
6. A production-profile non-proving relation metric shows the full path has
   `125,961` terminal operations, `14,049` supported NPO rows, and `242,798`
   NPO residual components before terminal proving begins.
7. The terminal relation digest rebuild is now deterministic for the baseline
   composite diagnostic; the fixed source was hash-ordered global lookup
   cumulative check emission in the recursive batch-STARK verifier circuit.
8. Fixed-width known-index proof limb/frontier encoding saves about `120.8 KiB`
   on the full-path reduced-profile wire object, but leaves `620,028` bytes in
   the NPO assignment-witness multiproof.

Assessment: this is still the preferred production direction, but not yet the
active stack-level production path. Its trade-off is witness exposure and, on
the full composite verifier relation, a much larger proof than the fixture:
about `753.2 KiB` on wire even after selecting the smallest measured
pure-query relation profile. Witness exposure is acceptable only if the final
terminal certificate is explicitly not specified as zero-knowledge and the full
composite verifier path is reduced under both production gates.

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
and it remains necessary for the full composite terminal path. The promoted
exhaustive path satisfies the `<30s` release target only for the
recursion-crate Tip5 verifier fixture. The actual `ai-pow-zk` composite
terminal path now has a completed reduced-profile release measurement:
`l1_verify=49ms`, `compile=7.606s`, `prove=80.829s`, `verify=58.825s`, and
postcard wire size `771,249` bytes.

The first runtime-instrumentation pass landed after this analysis. The
production prover now emits per-stage timings when
`NOCK_TERMINAL_PROFILE_PROVER=1` is set. Source inspection still shows
remaining repeated work:

- `verify_assignment_with_goldilocks_npos` checks the full assignment before
  proof construction.
- Prelude commitment helpers hash selected+lookup and full-trace+masked-NPO-IO
  matrices before the actual subproof provers commit the same matrix families.
- The integrated LogUp subproof builds several accumulator and quotient
  matrices over extension fields.

Immediate work:

1. Keep the real release measurement in the hot loop with `RUSTFLAGS="-C
   target-cpu=native"` and `NOCK_TERMINAL_PROFILE_PROVER=1`; the current
   `lb=6,nq=10,pow=0` proof is `771,249` bytes and `80.829s` to prove.
2. Keep the non-proving relation metric in the hot loop. The current PROD
   relation has `75,870` Horner operations before proof construction, so
   optimizing terminal proof serialization alone cannot satisfy the `<30s`
   full-stack target.
3. Cache selected+lookup matrix, trace bundle matrix, and prelude commitment
   digests across the prelude builder and subproof builders.
4. Avoid recomputing verifier-derived columns/layout/profile in the hot path
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
certificate measurement is now multiple MiB for the full checkpoint envelope,
and even the L1 proof body alone is `149.1 KiB`, already above target before
considering the native terminal path. It is still useful for:

- regression testing the recursive verifier relation;
- comparing terminal verifier behavior against a conventional batch-STARK
  wrapper;
- fallback development while terminal proof-shape work continues.

Assessment: keep it sound, but do not spend milestone effort trying to make it
the production certificate unless the hard size target changes.

## Recommendation

I would pursue five tracks in this order:

1. **Prototype the Pearl-shaped Plonky3 route: specialized AI-PoW AIR plus
   two-stage compact recursion.** This is the most plausible path to the Pearl
   class of proof sizes without importing Plonky2 or counting proof-system PoW.
   The prototype should first measure final compact recursive proof size under
   `pow_bits=0`, then add soundness documentation for every public,
   preprocessed, verifier-key, and cached-polynomial binding.
2. **Keep exhaustive NPO as the leading native-terminal fallback, but do not
   call it fully production-integrated yet.** It is the only current native
   terminal fixture measured below 100 KiB and below 30s, but the actual
   composite L1 verifier path still exceeds both the size and time gates.
3. **Reduce the full composite L1 terminal relation only if we keep pursuing
   the generic-verifier terminal route.** The current blocker is relation size:
   `106,349` primitive operations and `14,049` supported NPO rows in the PROD
   baseline. The primitive reduction should focus first on generic FRI/PCS
   verifier Horner work; the NPO reduction should focus on Tip5 and
   recompose/coeff callsite count without removing their bindings.
4. **Continue the unified NPO proof as hardening/future work.** It would reduce
   witness leakage if it can share one FRI payload and stay under target, but
   the current full-composite integrated candidate is too slow.
5. **Keep batch-STARK hardened as checkpoint only.** It is soundness-relevant
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
