# Native Terminal Compression Spec

Date: 2026-06-03
Status: implementation plan and interface checkpoint.

## Goal

Produce a native compact terminal certificate for the AI-PoW recursive verifier
circuit, targeting about 100 KiB while retaining the production 60-bit minimum
soundness floor and keeping recursive proving near the current 30 s budget.

This is not a dependency integration and not a renamed external proof system.
External code may be read as design reference, but the implementation must be
native to the vendored `plonky3-recursion` / `p3-circuit` stack and must emit a
Nockchain-owned certificate format.

## Current Bottleneck

The current recursion pipeline is:

1. build a `p3_circuit::Circuit` that verifies the previous proof;
2. execute that circuit to produce `p3_circuit::tables::Traces`;
3. prove those traces with `BatchStarkProver`.

Step 3 is the size bottleneck. After the 2026-06-03 batch-STARK FRI parameter
pass (`lb=4, nq=9, query_pow=24`, mixed query/PoW accounting), the outer
certificate is 200.6 KiB under the canonical fixed-int bincode byte helper
(225.8 KiB under legacy postcard), with about 163.5 KiB in FRI opening proof
material under postcard component accounting. That profile is useful historical
context, but it is not a production terminal profile because terminal soundness
must come from FRI queries alone. Another layer of the same batch-STARK proof
format does not address that floor.

Rejected size routes measured on 2026-06-03:

- generic lossless compression: 328.7 KiB raw became only about 291.1 KiB
  under best zlib/gzip before the parameter pass, so it cannot reach 100 KiB;
- Merkle-digest dictionarying: all authentication digests in the sampled L1
  proof were unique (`1365/1365` before the parameter pass, `819/819` at
  the active profile), so a digest dictionary is larger than the raw paths;
- query-count reduction alone: `nq=9, query_pow=24` was the best measured
  mixed-accounting size/time point; combined with fixed-int bincode it reaches
  200.6 KiB and 28.69 s for the L1 outer stage, but it is still above the
  terminal target and does not meet the pure-query terminal requirement;
- pushing further to `nq=8, query_pow=28` reduced the fixed-int L1 certificate
  to 185.6 KiB under mixed accounting, but the extra query PoW pushed the L1
  outer stage to 61.90 s and total trace-to-certificate time to 94.17 s, so it
  is rejected for the production path.
- raising the L1 MMCS cap from 5 to 6 is soundness-neutral but lost on both
  bytes and time in the production run: auth digests fell from 819 to 729, but
  commitment/cap bytes doubled from 4.5 KiB to 8.9 KiB, the fixed-int L1
  certificate grew to 209.6 KiB, and the L1 outer stage rose to 56.86 s. The
  active cap stays at 5.
- Pearl/Plonky2-style position-aware Merkle path compression was modeled
  directly against the active q=9/cap=5 recursive certificate shape on
  2026-06-03 (`/tmp/prod_recursion_measure_15_path_model_20260603.log`). The
  L1 proof has 10 independent auth-path tree groups and 819 raw sibling
  digests. Across 256 transcript-shaped random index trials, multi-proof path
  compression reduced this to 787.7 siblings on average: only 1.2 KiB mean
  digest savings, 6.8 KiB best sampled savings, and 0 KiB worst sampled
  savings. The fixed-int certificate floor would remain about 199.4 KiB on
  average and 193.8 KiB even in the best sampled case. This route is rejected
  as a route to ~100 KiB.

The path-compression result is security-relevant: the only safe version would
rederive query indices from the Fiat-Shamir transcript after all commitments,
final-polynomial coefficients, and arity schedule are bound.
Trusting serialized query indices would let the prover steer Merkle openings
outside the verifier's transcript challenges and would be a soundness bug. Since
the measured size win is negligible, no terminal wire format should be added
for this route.

## Native Terminal Interface

`p3_recursion::terminal` now defines the handoff:

- `TerminalCircuitFingerprint`: stable structural identity for the verifier
  circuit.
- `TerminalRelationDigest`: a Tip5 digest for Goldilocks terminal keys that
  commits to the compiled constraint relation, not only the circuit shape.
- `TerminalBackendRelationDigest`: a domain-separated Tip5 digest of the
  backend-projected relation. It binds the lowered primitive quadratic relation
  and the flattened supported-NPO row relation, then the outer
  `TerminalRelationDigest` absorbs this sub-digest.
- `TerminalWitness`: public inputs, private inputs, and executed circuit traces.
- `TerminalCompressor`: a proof-format-independent trait for compiling,
  proving, verifying, and serializing a compact terminal proof.
- `TerminalCertificateHeader`: versioned metadata that binds protocol id,
  soundness bits, circuit fingerprint, and, for Goldilocks terminal keys, the
  relation digest.
- `TerminalCertificate`: the terminal wire object. It carries the header, a
  `TerminalProofKind`, a Tip5 digest of the full public input vector, a Tip5
  digest of the backend proof body, an aggregate `TerminalBindingDigest` over
  header + proof kind + public digest + proof body digest, and the
  backend-specific proof body bytes. The current typed local checkpoint is
  marked `LocalCheckpoint`; the completed backend must use a distinct
  production kind.
- `TerminalProofParameters`: production terminal parameters committed before
  any backend challenge is sampled. The active native-terminal checkpoint
  profile is `security_bits=60, log_blowup=4, num_queries=15,
  query_pow_bits=0`, whose pure-query Johnson accounting is exactly 60 bits.
  Public production verification rejects other parameter tuples, even if their
  `log_blowup * num_queries` product also reaches 60 bits.
- `TerminalProofPrelude`: the first proof-body transcript object. It binds the
  terminal proof parameters, compiled relation profile, full public-values
  digest, backend commitment digests, canonical zero terminal query-PoW nonce,
  and a domain-separated Tip5 challenge digest.
- `TerminalOracleMerkleTree` / `TerminalOracleCommitment` /
  `TerminalOracleOpening`: the first backend oracle commitment layer. It commits
  to terminal field-value vectors with 5-round Tip5 Merkle roots and verifies
  authenticated single-value openings against those roots.
- `TerminalOracleMultiProof`: the sparse multi-opening form for terminal
  oracle values. It requires sorted unique leaves, verifies a shared frontier
  back to the committed 5-round Tip5 Merkle root, and is now used by production
  supported-NPO verification so the exhaustive row check does not repeat the
  same witness-path siblings.
- `TerminalOracleKnownIndexMultiProof`: the sparse multi-opening form for
  oracle values whose indices are verifier-derived from the surrounding
  relation. Production exhaustive NPO witness openings use this form: the
  verifier derives the exact sorted witness-ID set from every supported NPO
  callsite, so serializing those witness indices would be redundant. The proof
  also packs verifier-known boolean MMCS direction-bit values into a bitset and
  reconstructs canonical field values before Merkle root checking.
- `TerminalQueryPlan`: verifier-derived query indices for a committed terminal
  oracle. Query positions are sampled from the bound terminal prelude and oracle
  root; they are not accepted from serialized proof-body data. When the queried
  domain can provide at least `num_queries` rows, query derivation samples
  without replacement so the 15-query production profile yields 15 effective
  row checks instead of a smaller duplicate-collapsed set.
- `TerminalPrimitiveConstraintProof`: sampled local proof for primitive terminal
  constraints. It opens committed witness-oracle values for transcript-derived
  primitive constraint indices and checks constants, public bindings, and ALU
  equations against those openings.
- `TerminalNpoProof`: sampled local proof for supported non-primitive terminal
  rows. It uses transcript-derived NPO-row indices, opens committed witness
  values for the sampled row's circuit-implied call-site witnesses, and checks
  recursive 5-round Tip5 or Goldilocks-base recompose equations locally.
- `TerminalCombinedValidityConsistencyProof`: sampled consistency openings
  tying one combined validity oracle to the witness oracle. The combined oracle
  is ordered as `[primitive quadratic residuals || supported NPO validity
  residuals]`; fold-derived primitive rows recompute `A(w) * B(w) - C(w)`, and
  fold-derived NPO rows recompute the supported 5-round Tip5 or recompose row
  locally.
- `TerminalQuadraticResidualProof`: sampled openings of the backend primitive
  residual oracle. The residual vector is `A(w) * B(w) - C(w)` for every
  lowered primitive quadratic equation, committed as its own 5-round Tip5
  Merkle oracle and sampled from the transcript-bound prelude.
- `TerminalResidualFoldProof`: a Merkle-backed folded validity-oracle proof.
  In the aggregate terminal-local checkpoint it folds the combined validity
  oracle once, instead of carrying separate primitive-residual and NPO-validity
  folds. It commits each transcript-challenge fold layer, samples fold paths
  only after all fold roots are fixed, checks sampled pair-fold consistency, and
  requires the final one-value fold to be zero.
- `TerminalAssignmentEvaluationProof`: a Merkle-backed multilinear evaluation
  proof for the assignment vector `[1 || public || witness]`. It authenticates
  the public prefix against the assignment commitment with one compact
  prefix-frontier proof, then folds the whole assignment vector to one value. It
  authenticates each assignment fold layer with one sparse multiproof over the
  transcript-derived left/right leaves. It supports both its standalone
  transcript-derived point and an externally supplied point from a sparse-R1CS
  sumcheck. This is the native PCS primitive the sumcheck needs for its final
  `z(y*)` check; it is not accepted as a standalone production proof.
- `TerminalSparseR1csSumcheckProof`: a batched matrix-vector sumcheck component
  for the sparse terminal R1CS matrices. It proves claimed `A`, `B`, and `C`
  matrix-vector evaluations against the committed assignment vector and consumes
  `TerminalAssignmentEvaluationProof` at the final sumcheck point. It can use
  either its own transcript-derived row point or the externally supplied row
  point from the row-product sumcheck. It deliberately does not check
  `A(r) * B(r) = C(r)`, because that is not a sound multilinear R1CS shortcut;
  the row-product relation still needs its own sumcheck.
- `TerminalR1csRowProductSumcheckProof`: the degree-3 sumcheck for the primitive
  row relation `sum_x eq(r, x) * (A(x) * B(x) - C(x)) = 0`. Its final random row
  point is delegated to `TerminalSparseR1csSumcheckProof`, so the verifier checks
  the row product against matrix-vector claims that are themselves tied to the
  assignment commitment.
- `TerminalNpoValidityFoldProof`: a Merkle-backed folded validity-oracle proof
  for supported NPO rows. This remains a local/checkpoint component and
  authenticates each fold layer with one sparse terminal-oracle multiproof over
  the transcript-derived left/right leaves, rather than serializing an
  independent Merkle path per sampled query and round.
- `TerminalLocalProof`: the implemented local-proof envelope. It carries the
  prelude, witness commitment, combined-validity commitment, mixed consistency
  proof, and combined-validity fold proof as one serializable object. It is an
  integration checkpoint, not the final global terminal proof.
- `TerminalProductionProof`: the typed production proof body for the current
  compact backend checkpoint. It binds a witness oracle for exhaustive NPO row
  openings, an assignment oracle for primitive sparse-R1CS, the primitive
  row-product sumcheck, and an optional exhaustive supported-NPO proof for keys
  with supported NPO rows. The exhaustive NPO proof verifies every flattened
  Tip5/recompose NPO row against one shared known-index witness multiproof and
  compact hidden Tip5-input payloads. It no longer serializes the full witness
  and no longer accepts sampled NPO validity as the production NPO soundness
  boundary. The remaining production-backend gap is replacing the Merkle-heavy
  exhaustive NPO row check with a polynomialized Tip5/recompose argument.
- `TerminalQuadraticRelation`: backend-ready R1CS-style primitive relation.
  Primitive terminal gates lower to equations `A * B = C` over linear
  combinations of `{1, public input, witness}`. `BoolCheck` lowers to two
  equations (booleanity plus output binding), while supported NPO rows remain
  counted as external rows whose validity is handled by the NPO relation,
  validity oracle, and future dedicated algebraic arithmetization.
- `TerminalSparseR1csRelation`: sparse multilinear table view of that primitive
  relation. It indexes the assignment vector as `[1 || public || witness]`,
  stores nonzero entries for the `A`, `B`, and `C` matrices, and records padded
  row/variable log sizes for the native sumcheck/polynomial backend.
- `TerminalNpoRelation`: backend-ready external-row relation for supported NPO
  gates. It flattens compiled Tip5/recompose aggregates into stable global NPO
  row IDs, preserving each row's op key, local row number, row kind, and exact
  circuit-implied call-site witnesses.

The current production path still uses `BatchStarkProver`, but
`build_terminal_witness` now packages the verifier-circuit execution before that
proof step. A native compact backend should consume the same circuit and witness
bundle while replacing the terminal proof format.

The current native compiler checkpoint is intentionally conservative:

- primitive operations (`Const`, `Public`, ALU add/mul/bool/mul-add/Horner)
  are compiled into an explicit native terminal constraint IR and accepted into
  proving/verifying key profiles;
- hint operations are inventoried as witness-generation metadata, matching
  their current p3-circuit semantics;
- recursive 5-round Goldilocks Tip5 NPO rows are accepted and checked against
  `p3-tip5-circuit-air` / `nockchain_math::tip5::permute_5round`;
- Goldilocks-base `recompose` and `recompose/coeff` NPO rows are accepted and
  checked by binding every input coefficient witness and the recomposed output
  witness;
- supported NPO relations bind the exact circuit-implied row count per NPO type,
  and Goldilocks terminal verification rejects missing rows, extra rows, and
  unexpected NPO trace tables;
- supported NPO rows also bind the circuit-implied call-site witness IDs, so a
  prover cannot duplicate one valid NPO row while omitting a different circuit
  call with the same NPO type and row count;
- supported NPO op layouts are validated at terminal-compile time; malformed
  Tip5/recompose layouts with otherwise recognized NPO keys are rejected instead
  of being truncated or normalized into a weaker relation;
- terminal witnesses must match the fingerprinted private-input length, witness
  trace row count, and sequential witness-ID index exactly;
- other table-backed non-primitive operations are rejected with their operation
  type until their constraints are ported into the compact terminal
  arithmetization;
- requested terminal profiles below 60 bits are rejected at compile time.

The primitive constraint checker validates constants, public-input bindings,
ALU equations, Horner accumulator equations, recursive Tip5 input/output CTL
bindings, exact NPO call-site witness IDs, exact NPO trace row counts, absence
of unexpected NPO tables, recompose coefficient/output bindings, and
verifier-circuit fingerprints against an executed `TerminalWitness`. The checker
is not itself a compact proof; it is the verifier-side relation that the compact
terminal proof protocol must prove.

The primitive part of that relation now also lowers into
`TerminalQuadraticRelation`: constants, public bindings, add, mul, mul-add, and
Horner accumulator gates each become one quadratic equation, and bool-check gates
become two equations. The evaluator verifies the lowered equations against the
same executed `TerminalWitness` and rejects tampered witness values. This gives
the future transparent backend a concrete algebraic primitive relation to commit
and compose, instead of depending on Rust-only match-arm semantics. Supported
Tip5 and recompose NPO rows are deliberately not forced into this primitive
quadratic form; they remain external rows that need dedicated global
arithmetization.

The same quadratic relation now also lowers into `TerminalSparseR1csRelation`.
Rows are primitive quadratic equations, columns are `[1 || public || witness]`,
and each nonzero entry records its matrix (`A`, `B`, or `C`), row, variable
kind, stable variable index, and coefficient. On the real Tip5 Layer-0 verifier
circuit this table has the same row count as the primitive quadratic relation
and variables `1 + 33 + 3043`; its padded multilinear dimensions are tested in
the integration profile. This is the concrete table a Spartan/Aurora-style
global sumcheck backend must bind. It is not accepted as a proof by itself.

The terminal checkpoint also now has a native assignment-evaluation component.
The prover commits to `[1 || public || witness]`, opens the `1 || public`
prefix explicitly, then folds the full assignment vector with transcript-derived
challenges to produce a committed multilinear evaluation. This closes the
obvious public-prefix substitution bug for a future assignment PCS: the
assignment commitment cannot silently use different public values. The same
component can now verify at an externally supplied point, so the sparse-R1CS
sumcheck can bind its final random `y*` to the assignment commitment instead of
proving an unrelated assignment evaluation. The component still needs to be
consumed inside a full relation argument before it can replace the direct
witness-serializing production checker.

The sparse-R1CS matrix-vector sumcheck component is implemented and tested. It
is intentionally scoped to matrix-vector evaluation claims. A previous tempting
check, `A(r) * B(r) = C(r)`, is not used: even when every Boolean row satisfies
`A_i * B_i = C_i`, the product of multilinear extensions at a non-Boolean row
point is not equal to the multilinear extension of the row products. The
primitive-relation step is now a separate degree-3 row-product sumcheck over
`eq(r, x) * (A(x) * B(x) - C(x))`, with its final `A(x*)`, `B(x*)`, and `C(x*)`
claims delegated to the matrix-vector sumcheck at the same transcript-derived
row point.

Performance note: the first correct matrix-vector round evaluator used dense
partial sums of the product of the matrix MLE and assignment MLE and was stopped
after exceeding two minutes on the real Tip5 Layer-0 verifier. That path has
been replaced by a folded-vector evaluator: the prover builds the batched sparse
matrix coefficient vector once, pads the assignment vector once, then folds both
vectors in lockstep through the sumcheck rounds. The component now also verifies
at an externally supplied row point, which is the hook the row-product sumcheck
needs for its final `x*` claims.

Those supported NPO rows now also project into `TerminalNpoRelation`: a stable
global row domain over `tip5_perm/goldilocks_w16_r5`, `recompose`, and
`recompose/coeff`. Goldilocks verification rejects a supplied NPO relation that
does not exactly match the verifying-key projection before checking the executed
witness. This gives the future backend a concrete external-row target for the
668 production NPO rows.

The terminal backend checkpoint now has two supported-NPO verifier forms. The
local/checkpoint form constructs a supported-NPO validity oracle and folds it
with NPO-specific domains `nock-terminal-npo-validity-fold-challenge-v1` and
`nock-terminal-npo-validity-fold-query-v1`; each fold layer is authenticated by
a shared sparse multiproof, and sampled fold-query rows are linked back to the
witness oracle by `TerminalNpoValidityConsistencyProof`. The production form no
longer accepts this sampled NPO validity layer. Instead,
`TerminalNpoExhaustiveProof` verifies every supported Tip5/recompose row in the
flattened NPO relation against the committed witness oracle with one shared
known-index witness multiproof. The verifier derives those witness indices from
the committed terminal relation before checking the multiproof root. This
closes the sampled supported-NPO gap for the current production checkpoint, but
it is still Merkle-heavy and should be replaced by a polynomialized
Tip5/recompose arithmetization and proximity/sumcheck proof.

`TerminalBackendRelationDigest` is the explicit commitment to those backend
projections. It has its own domain and absorbs both `TerminalQuadraticRelation`
`TerminalSparseR1csRelation`, and `TerminalNpoRelation`; `TerminalRelationDigest`
then absorbs the backend projection digest under a separate binding domain.
Regression tests mutate a compiled quadratic equation and a compiled NPO
row/call-site ordering under a stale signed header and confirm both the backend
digest and the outer relation digest change, with verification failing before
witness checks. This prevents a future terminal proof body from proving a
projected relation that is not the one committed by the terminal key.

The relation is now tested on the real Tip5 Layer-0 verifier circuit used by
the L1 path (`terminal_compiler_covers_real_tip5_l0_verifier_circuit`): it
builds the verifier circuit, injects Tip5 MMCS private data, executes the
runner, compiles terminal constraints with a populated relation digest, confirms
all three production NPO keys (`tip5_perm/goldilocks_w16_r5`, `recompose`,
`recompose/coeff`) are present, verifies the executed witness, lowers the
primitive relation to `TerminalQuadraticRelation`, and verifies those quadratic
equations against the same real witness, projects the supported NPO rows into
`TerminalNpoRelation`, and verifies that projected relation against the same
real witness. This is still a relation-coverage gate, not a compact proof.

That test now also locks the concrete relation profile for the representative
real L1 verifier circuit:

| item | count |
|---|---:|
| witness values | 3,043 |
| public input values | 33 |
| private input values | 156 |
| circuit operations | 2,620 |
| primitive terminal constraints | 1,881 |
| terminal constraints after NPO aggregation | 1,884 |
| hint operations | 71 |
| non-primitive operations / rows | 668 |
| Tip5 rows | 520 |
| `recompose` rows | 51 |
| `recompose/coeff` rows | 97 |
| NPO input callsite slots | 8,616 |
| NPO output callsite slots | 5,348 |

This profile is the next backend's proving target. It rules out a proof body
that simply serializes the witness or the checked NPO trace: the raw witness
surface is already too large, and exposing it would not be a zero-knowledge or
succinct terminal certificate. The viable backend has to commit to the witness
and prove the aggregated primitive + NPO relation with sublinear verifier data.
For the primitive part, the backend target is now the quadratic relation above;
for the 668 supported NPO rows, the backend target now includes the NPO relation
and exhaustive row verifier, but still needs a polynomialized Tip5/recompose
arithmetization to reduce the Merkle-heavy production payload.

Goldilocks terminal verification rejects keys with a missing relation digest and
rejects stale digests after recomputing the relation digest from the verifying
key. This prevents accidentally verifying a production terminal witness against
an unbound or shape-only terminal key.

Goldilocks terminal certificate assembly and pre-verification now bind the
certificate header, relation digest, full public input vector, and proof body
with domain-separated Tip5 digests. In addition to separate public-values and
proof-body digests, the wire certificate carries an aggregate binding digest
over the complete terminal metadata/public/body commitment tuple. The terminal
precheck rejects stale headers, public-value substitution, direct proof-body
tampering, and stale aggregate binding digests before a backend-specific proof
verifier can run. This is a wire-format and binding checkpoint; it is not yet a
compact proof backend.

There is now a typed checkpoint for the implemented local-proof body:
`assemble_goldilocks_local_certificate` verifies a `TerminalLocalProof`, encodes
it as the certificate proof body, and assembles the standard terminal
certificate binding with `TerminalProofKind::LocalCheckpoint`.
`verify_goldilocks_local_certificate` first checks the certificate
header/proof-kind/public/body/binding digests, then decodes the body as a
`TerminalLocalProof` with explicit rejection of trailing bytes, then reruns the
local proof verifier. A raw certificate can still bind arbitrary bytes for a
future backend, but the proof kind is committed and the local-proof checkpoint
no longer treats typed local proof material as an opaque blob once the local
verifier is selected. The raw binding checker is not a public production
verifier; `verify_goldilocks_production_certificate` checks for a
`Production`-kind envelope, decodes a typed `TerminalProductionProof` with
explicit trailing-byte rejection, and reruns the compact production
verifier. Malformed production bodies are rejected before relation checks, and
`LocalCheckpoint` certificates still fail by proof-kind mismatch.

Goldilocks terminal proof prelude assembly and public production verification
now reject:

- empty backend commitment lists;
- parameters below 60 bits;
- terminal parameter security labels that disagree with the verifying key;
- terminal parameter labels that exceed their own Johnson accounting;
- nonzero terminal `query_pow_bits`;
- noncanonical nonzero terminal query-PoW nonces;
- production proofs whose prelude parameters are not exactly
  `TerminalProofParameters::production_60bit()`;
- production terminal query domains with fewer rows than `num_queries`;
- production/local-checkpoint proof-kind confusion at local-certificate
  verification;
- stale relation profiles;
- public-value substitution;
- stale challenge digests after commitment tampering.

The prelude challenge uses domain `nock-terminal-transcript-v1` and absorbs the
certificate header, relation digest, terminal parameters, measured relation
profile, full public-values digest, and every backend commitment digest before
finalizing. This closes the first Fiat-Shamir/grinding boundary for the future
compact backend: no query/challenge material may be sampled before these values
are fixed. The prelude is still not a proof; it is the mandatory transcript
prefix that the remaining polynomial/sumcheck/commitment proof must extend.

Every oracle-backed local verifier now also checks that the passed commitment
root is one of the roots absorbed into that prelude. This applies to the generic
terminal query-opening helper, primitive witness openings, supported-NPO witness
openings, quadratic residual openings, and NPO validity openings. Passing an
unbound witness, residual, NPO validity, or other terminal oracle root is
rejected before query plans are derived. Without this check, a prover could
choose an oracle after the prelude challenge boundary and grind against
already-known sampled rows.

The same verifier entrypoints also reject base-oracle identity drift. The
witness oracle must use label `witness` and length equal to the compiled witness
count; the primitive residual helper oracle must use label `quadratic_residual`;
the supported-NPO helper oracle must use label `npo_validity`; and the aggregate
local proof's combined oracle must use label `combined_validity` with length
equal to lowered quadratic rows plus flattened supported-NPO rows. These checks
happen before query derivation and opening verification, so a proof body cannot
keep a prelude-bound root while presenting alternate oracle metadata to a later
transcript step.

The first backend oracle commitment layer now uses domain-separated 5-round
Tip5 Merkle commitments:

- leaf domain: `nock-terminal-oracle-leaf-v1`, absorbing oracle label,
  vector length, index, and Goldilocks-basis coefficients;
- padding-leaf domain: `nock-terminal-oracle-empty-leaf-v1`, absorbing label
  and real vector length;
- internal-node domain: `nock-terminal-oracle-node-v1`, absorbing label,
  left digest, and right digest.

Openings carry the index, opened basis coefficients, and directional sibling
path. Verification rejects empty oracle vectors, out-of-bounds openings, path
length mismatches, root mismatches, and expected-value mismatches. The prelude
test binds an oracle root into the terminal commitment list, so any later
terminal challenges are sampled after the oracle root is fixed.

Terminal query indices now use domain `nock-terminal-query-plan-v1`. The
derivation absorbs the prelude challenge digest, terminal parameters, measured
relation profile, public-values digest, oracle label, oracle length, oracle root,
and a counter, then uses rejection sampling into the oracle length. Verification
recomputes the query plan and rejects:

- proof bodies with too few or too many query openings;
- openings whose indices do not match the transcript-derived query index;
- empty oracle commitments;
- root/path/value tampering through the underlying oracle-opening verifier.

This is the second Fiat-Shamir/grinding boundary: a prover can no longer choose
friendlier terminal rows to open after seeing the relation/public/commitment
binding. The remaining backend proof still must turn those query openings into
a full relation argument with a soundness calculation; query derivation alone is
not sufficient.

The first local relation proof component samples primitive terminal constraints
with domain `nock-terminal-primitive-constraint-query-v1`. The verifier
recomputes the primitive-constraint query plan from the prelude, opens the
witness-oracle values needed by each sampled primitive constraint, verifies each
Merkle opening, reconstructs the Goldilocks-basis field values, and checks:

- `Const`: opened witness value equals the compiled constant;
- `Public`: opened witness value equals the public input at the compiled
  position;
- `Alu`: add, mul, bool, mul-add, and Horner accumulator equations.

The verifier rejects constraint-index steering, missing witness openings,
opening/witness-ID mismatches, noncanonical opened basis limbs, and locally
invalid committed witness values.

The primitive quadratic residual oracle is now an explicit backend target. The
prover computes every lowered primitive residual, commits the vector with label
`quadratic_residual`, binds that root into the prelude, and opens
transcript-derived residual positions using the general terminal oracle query
domain. The verifier rejects unbound residual roots, residual-domain length
mismatches, query steering, Merkle tampering, noncanonical residual limbs, and
nonzero sampled residuals. This is still not the final primitive proof: the
backend must still prove that the committed residual oracle was derived from
the committed witness oracle for all rows, not only sampled openings.

The primitive residual-witness consistency component samples the same quadratic
row domain and opens both the residual oracle and witness oracle at the row's
compiled witness IDs. The verifier recomputes the lowered equation from opened
witness values and public inputs, rejects residual-oracle values that do not
match `A(w) * B(w) - C(w)`, and then rejects nonzero residuals. This closes the
sampled substitution gap where a prover could otherwise commit an all-zero
residual oracle unrelated to the witness oracle. It still does not replace the
global proximity/sumcheck argument: unsampled rows and low-degree consistency
remain future backend obligations.

The primitive residual fold component is the first global-style residual
argument checkpoint. Starting from the prelude-bound residual oracle, the prover
derives a field challenge for each round, folds adjacent residual values as
`left * (1 - r) + right * r`, commits every folded layer with 5-round Tip5
Merkle roots, then derives query indices after all fold roots are fixed. The
verifier checks sampled fold paths across every layer, rejects query steering,
missing or unexpected right openings on odd layers, fold-root tampering, and
nonzero final folded values. This moves the primitive residual proof toward a
sumcheck/Fri-style global check, but it is not yet a complete production
soundness proof for the whole terminal relation.

The fold opening format has been compacted once. For each sampled fold pair,
the right leaf no longer serializes a duplicate Merkle path; its value is
authenticated by matching its leaf digest against the first sibling digest in
the left opening. The next-layer value also no longer serializes a duplicate
path; it is linked to the following round's current-layer opening, or to the
final one-leaf fold root in the last round. The verifier rejects non-empty
right/next paths and rejects right/next values that fail those authentication
links.

The aggregate combined-validity fold component mirrors that residual-fold
discipline across primitive residual rows and external NPO validity rows in one
domain. Starting from the prelude-bound `combined_validity` oracle, the prover
folds adjacent validity residuals, commits each fold layer with 5-round Tip5
Merkle roots, samples fold paths only after all fold roots are fixed, and
requires the final folded value to be zero. A mixed consistency component opens
the same fold-query rows, verifies primitive rows by recomputing
`A(w) * B(w) - C(w)`, verifies NPO rows by recomputing the sampled
Tip5/recompose row from committed witness openings, and rejects any nonzero
validity residual. The verifier rejects combined-validity root omission,
fold-query steering, malformed fold commitment schedules, malformed fold paths,
nonzero final folds, and nonzero sampled row-validity residuals. This closes the
  immediate missing local NPO validity oracle gap, but production no longer uses
  this sampled NPO layer as its supported-NPO soundness boundary.
Regression tests now also reject primitive rows presented as NPO rows, NPO rows
presented as primitive rows, wrong combined-validity indices, and wrong
NPO-index derivations inside the mixed consistency proof.

On the real Tip5 L0 verifier fixture, fold compaction reduced the
terminal-local certificate from 408,453 bytes (398.9 KiB) before compaction, to
328,150 bytes (320.5 KiB) after right-leaf compaction, and then to 248,200 bytes
(242.4 KiB) after next-link compaction. The measured terminal-local
prove/verify times at the last point were about 5.95 s and 3.58 s respectively
in the debug-profile test harness. The direct primitive samples, direct
supported-NPO samples, and standalone residual zero openings were then removed
from `TerminalLocalProof` because they duplicate the stronger consistency/fold
checkpoint components without establishing production global soundness. That
brings the typed local certificate to 178,073 bytes (173.9 KiB), with
debug-profile prove/verify around 5.45 s and 2.49 s. The residual-witness and
NPO-validity consistency proofs then stopped serializing duplicate
residual/validity Merkle openings. They now link to the same transcript-derived
base values already authenticated by the corresponding fold proof. That brings
the typed local certificate to 157,348 bytes (153.7 KiB), with debug-profile
prove/verify around 4.93 s and 2.42 s. A terminal query-PoW profile was
implemented and measured, but production soundness must not count query-PoW
bits. The active production checkpoint therefore rejects nonzero terminal
`query_pow_bits` and uses pure-query `num_queries=15, query_pow_bits=0`. The
next structural pass combined primitive
quadratic residuals and supported-NPO validity residuals into one
`combined_validity` oracle and one fold proof, ordered as
`[primitive residuals || NPO validity residuals]`. That removes the second fold
proof and the separate NPO consistency query set from the aggregate local proof.
The active 60-bit pure-query checkpoint most recently measured 83,414 bytes
(81.5 KiB) for the typed local certificate, with debug-profile prove/verify
around 5.51 s and 1.97 s. Recent runs remain below the 100 KiB target for the
local checkpoint, but it is still not a production terminal certificate.

`TerminalLocalProof` now packages all implemented local proof components behind
one verifier entrypoint. The verifier rechecks the prelude, then verifies the
combined-validity consistency proof and combined-validity fold proof against the
same public input vector and the same prelude-bound roots. Corrupting the
combined validity commitment, fold proof, primitive consistency opening, or NPO
consistency opening is rejected. This prevents integration code from accepting a
proof body that quietly omits one current local component.

The second local relation proof component samples supported non-primitive rows
with domain `nock-terminal-npo-query-v1`. The verifier recomputes the NPO-row
query plan from the prelude and relation profile, rejects prover-selected row
indices, opens the row's circuit-implied witness IDs from the witness oracle,
and checks:

- `tip5_perm/goldilocks_w16_r5`: the opened input witnesses match the sampled
  row's 16 input values where the circuit call-site exposes them, the verifier
  recomputes the recursive 5-round Tip5 permutation, and opened output
  witnesses match the first 10 rate outputs;
- `recompose` and `recompose/coeff`: opened input witnesses are base-field
  Goldilocks coefficients embedded in the extension field, and the opened
  output witness equals the extension element reconstructed from those
  coefficients.

The verifier rejects NPO-row steering, missing witness openings,
opening/witness-ID mismatches, malformed Tip5 local values, noncanonical opened
basis limbs, and locally invalid committed Tip5/recompose witness values. This
is still a sampled local component, not the complete argument: global
composition, unsampled-row coverage, and a polynomial/proximity or sumcheck
argument are still required for production soundness.

The current real-circuit terminal-local size profile after fold compaction is:

| component | bytes |
|---|---:|
| prelude | 216 |
| combined validity consistency openings | 42,968 |
| combined validity fold | 61,089 |
| typed local proof body | 104,399 |
| typed local certificate | 104,618 |

This profile says the remaining size problem is not generic serialization
overhead. The large items are still Merkle-authenticated local openings and the
fold-path commitments. A production terminal backend must replace these local
samples with a real global composition/proximity argument, or add a proper
multi-opening commitment layer. This checkpoint is useful as a typed
local-proof baseline, but it is not production terminal soundness and is no
longer the size path.

The typed compact production checker now measures as follows on the same real
Tip5-L0 verifier circuit:

| component | bytes |
|---|---:|
| primitive R1CS row-product proof | 24,562 |
| exhaustive NPO proof | 64,738 |
| exhaustive NPO nonzero masks | 1,065 |
| exhaustive NPO hidden Tip5 input bytes | 17,402 |
| exhaustive NPO known-index witness multiproof | 46,271 |
| exhaustive NPO full-width witness openings | 1,377 |
| compact production proof body | 89,642 |
| compact production certificate | 89,861 |

The debug-profile measurement is `prove=4.834 s, verify=3.132 s` for the
production proof body and certificate, with terminal parameters
`security_bits=60, log_blowup=4, num_queries=15, query_pow_bits=0`. This removes
the sampled production NPO validity layer and verifies all 668 supported
Tip5/recompose NPO rows against the committed witness oracle. The NPO proof is
still the dominant component, but the witness multiproof no longer serializes
indices that the verifier can derive from the committed terminal relation, and
Tip5 carry/zero hidden lanes are derived from row mode plus previous outputs
instead of serialized. Verifier-derived boolean MMCS direction-bit openings are
bit-packed and reconstructed as canonical field elements during root
verification. This
puts the production certificate below the 100 KiB gate while retaining the
required 60 pure-query bits. The remaining production-backend size task is to
replace this exhaustive Merkle-opening NPO proof with a polynomialized
Tip5/recompose relation and the final terminal proximity backend.

The optimized sparse-R1CS matrix-vector sumcheck component measures separately
on the same real Tip5-L0 verifier circuit. The completed primitive row-product
component includes that matrix-vector subproof:

| component | bytes |
|---|---:|
| sparse R1CS matrix sumcheck proof | 22,681 |
| R1CS row-product sumcheck proof | 23,763 |
| row-product rounds | 850 |
| matrix-sumcheck rounds | 718 |
| assignment evaluation proof | 22,135 |
| assignment public-prefix proof | 477 |
| assignment fold commitments | 802 |
| assignment fold query indices | 46 |
| assignment fold round multiproofs | 20,791 |

The latest debug-profile measurements are `prove=2.094 s, verify=1.380 s` for
the matrix-vector component and `prove=2.239 s, verify=1.367 s` for the
row-product component. This now proves the primitive sparse-R1CS row relation
against the assignment commitment with compact public-prefix authentication, but
NPO/table global arguments and the final polynomial proximity backend remain
required before the exhaustive NPO verifier can be replaced by a smaller
polynomialized argument.

Recursive proving uses 5-round Tip5 only. This terminal path must not be read as
a change to Nockchain's canonical non-recursive 7-round Tip5 hash path.

Literature checkpoint as of 2026-06-03:

- Tip5 was specified for recursive STARK use with paper-spec round count `N=5`
  (IACR ePrint 2023/107, "The Tip5 Hash Function for Recursive STARKs",
  https://eprint.iacr.org/2023/107; mirror:
  https://cryptography.academy/papers/tip5-2023).
- The third-party "Opening the Blackbox" cryptanalysis (IACR ToSC 2024(4),
  ePrint 2024/1900, https://eprint.iacr.org/2024/1900) reports collision
  attacks on reduced-round Tip5, including practical/semi-free-start attacks on
  3-round Tip5. That reinforces that the terminal path must remain pinned to 5
  rounds and must not expose a prover- or certificate-selectable round count.
- DEEP-FRI improves proximity-test soundness by sampling outside the original
  evaluation domain while retaining linear proving and logarithmic verifier
  arithmetic complexity (Ben-Sasson, Goldberg, Kopparty, Saraf,
  arXiv:1903.12243 / ePrint 2019/336,
  https://eprint.iacr.org/2019/336 and https://arxiv.org/abs/1903.12243). Our
  current terminal baseline is still Plonky3 FRI rather than a new DEEP-FRI
  terminal proof, so parameter changes must continue to use the active Plonky3
  FRI verifier's query/PoW accounting.
- FRI itself is an interactive oracle proof of proximity for Reed-Solomon
  codes, with linear prover arithmetic and logarithmic verifier arithmetic in
  the original Fast RS IOPP formulation (ECCC TR17-134,
  https://eccc.weizmann.ac.il/report/2017/134/). Any replacement terminal
  proximity layer must therefore state exactly which code, rate, query count,
  and Fiat-Shamir transcript it is instantiating; a Merkle opening protocol over
  arbitrary witness values is not a FRI proximity proof by itself.
- Fiat-Shamir security for FRI and batched FRI is treated in Block, Garreta,
  Katz, Thaler, Tiwari, Zajac (IACR ePrint 2023/1071,
  https://eprint.iacr.org/2023/1071). The terminal certificate therefore has to
  bind the complete statement and verifier relation before deriving challenges;
  hashing only a shape, key label, or truncated public digest is not sufficient.
- The native terminal proof must bind the full Fiat-Shamir transcript domain,
  FRI parameters, query/PoW counts, verifier-circuit fingerprint, public input
  vector, Tip5 variant key, primitive quadratic relation, and all NPO relation
  keys. These bindings are required to avoid the parameter-substitution and
  grinding failures called out by the Fiat-Shamir/FRI literature.
- Aurora (Ben-Sasson, Chiesa, Riabzev, Spooner, Virza, Ward, EUROCRYPT 2019,
  ePrint 2018/828, https://eprint.iacr.org/2018/828) is the closest transparent
  R1CS-sized route in the literature: its proof sizes are sublinear in the
  constraint count but still not automatically near 100 KiB at 60-bit security,
  and its verifier is linear in the instance size unless the relation/key
  commitment is handled carefully. A native terminal backend can use this
  literature as a design reference, but must bind the full compiled relation
  digest above rather than accepting a generic R1CS blob.
- Recent Reed-Solomon proximity-gap work keeps the safe side of the design
  inside Johnson-style accounting. Any terminal protocol that replaces the
  current batch-STARK must redo the proximity/soundness calculation for its own
  polynomial IOP; the existing `lb=4, nq=9, query_pow=24` number cannot be
  copied over unless the terminal backend uses the same FRI verifier model.

Completion audit against the active terminal-compression requirements:

| requirement | current evidence | status |
|---|---|---|
| Production profile gets exactly the canonical 60 pure-query bits without query PoW | `TerminalProofParameters::production_60bit()` uses `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`; low-soundness and nonzero terminal-PoW profiles are rejected by prelude tests, and public production verification rejects noncanonical 60-bit parameter tuples. | satisfied for the current terminal profile |
| Recursive terminal hashing uses 5-round Tip5 only | Recursive Tip5 terminal relation is KAT-checked against `nockchain_math::tip5::permute_5round`; tests reject tampering and bind each callsite. | satisfied for recursive terminal proving |
| Production certificate is about 100 KiB | Real Tip5-L0 verifier measurement: `89,861` bytes / `87.8 KiB`, debug-profile `prove=4.834s`, `verify=3.132s`. | satisfied on the measured production fixture |
| No confusing low-soundness testing production path | Production certificate verifier accepts only `TerminalProofKind::Production`; `LocalCheckpoint` is rejected by kind mismatch and query domains must support all 15 production queries. | satisfied for public production verifier dispatch |
| Public values, parameters, relation, and commitments are bound before challenges | Header, public-values digest, backend relation digest, prelude parameters, relation profile, and backend commitment roots are absorbed before terminal challenges. | satisfied for the implemented transcript prefix |
| Primitive terminal constraints are globally checked | Primitive constraints lower to sparse R1CS; row-product sumcheck delegates matrix-vector claims to the assignment evaluation proof. | substantially satisfied for primitive rows, subject to the stated sumcheck soundness model |
| Supported NPO rows cannot hide invalid sampled rows | Production no longer samples NPO validity; it exhaustively checks every supported Tip5/recompose NPO row against a prelude-bound witness oracle. | satisfied for supported NPO row validity |
| Supported NPO/table rows are polynomialized into a final proximity backend | Current production uses exhaustive Merkle openings for NPO rows, not a low-degree/proximity argument for a polynomialized Tip5/recompose relation. | not complete |
| Full terminal proof has a source-backed soundness calculation | Current doc records 60 pure-query Johnson accounting for the terminal profile and tests verifier binding, but it does not yet derive a complete theorem for the hybrid row-product plus exhaustive-NPO Merkle backend. | incomplete |
| Zero-knowledge or witness hiding for recursive-verifier witness values | Current production opens 1,377 full-width verifier-circuit witness values plus packed MMCS direction bits for exhaustive NPO checking. That is smaller than full witness serialization, but it is not a zero-knowledge terminal backend. | incomplete if ZK is required |

Security-audit conclusions for the current implementation checkpoint:

- The typed compact production proof now gives native terminal certificates a
  non-witness primitive sparse-R1CS row-product argument plus exhaustive
  supported-NPO row verification. This removes the previous full-witness
  serialization baseline and removes the sampled NPO production path, but the
  exhaustive NPO layer is still not the final polynomialized Tip5/recompose
  terminal backend.
- The current production proof is therefore a hybrid terminal backend: a
  sumcheck-backed primitive sparse-R1CS argument plus exhaustive Merkle-backed
  supported-NPO checking. This is a production-only proof shape under the
  current implementation, but it must not be described as the final
  polynomial/proximity backend requested for terminal completion.
- The terminal proof prelude is now an implemented transcript-binding prefix,
  not a standalone argument. It prevents challenge grinding across relation,
  public input, parameter, and commitment substitutions. In the compact
  production proof it binds witness and assignment commitments before any
  verifier challenges are sampled.
- Oracle roots passed to local proof verifiers must now be prelude-bound. This
  includes the generic terminal query-opening verifier and closes the immediate
  post-challenge oracle-substitution bug for witness, residual, NPO validity,
  and other terminal oracle commitments. The production proof's exhaustive NPO
  verifier binds its witness multiproof to the same prelude-bound witness root.
- Local proof verifiers now also enforce canonical base-oracle labels and
  lengths for witness, primitive residual, and supported-NPO validity
  commitments. This closes the softer commitment-identity drift class where a
  proof body could retain a bound root but present a different oracle label or
  domain length to downstream query derivation.
- Typed local terminal certificates now decode with a no-trailing-bytes rule
  before local proof verification. This closes a proof-body ambiguity observed
  with raw `postcard::from_bytes`, where appended bytes could otherwise be
  ignored after certificate binding succeeded around the longer byte string.
- The backend-projection digest is implemented and bound into the terminal
  relation digest. That closes a key-substitution class where a compact proof
  could otherwise be generated for a different lowered quadratic, sparse R1CS,
  or NPO layout than the one implied by the compiled verifier circuit. It is
  still only a commitment; the proof system must enforce the committed relation
  globally.
- The assignment-evaluation component binds the public prefix of
  `[1 || public || witness]` before folding the committed assignment vector.
  It also supports verification at an externally supplied point, which is
  necessary for a future multilinear PCS opening inside sumcheck. It is only an
  evaluation proof; it must be consumed by the sparse-R1CS sumcheck before it
  gives a terminal relation proof.
- The sparse-R1CS matrix-vector sumcheck is only an evaluation subclaim. It
  avoids the unsound `A(r) * B(r) = C(r)` shortcut and is now paired with the
  row-product sumcheck for primitive R1CS rows. It is still not a complete
  terminal proof for the supported NPO/table rows.
- The terminal oracle Merkle layer is binding to opened values under the
  recursive 5-round Tip5 assumption, but it is not itself a polynomial
  commitment. The compact production proof now removes full witness
  serialization for primitive rows and exhaustively checks supported NPO rows;
  the remaining backend must replace Merkle-heavy NPO openings with a
  polynomialized PCS/sumcheck argument at >=60-bit soundness.
- The known-index exhaustive NPO multiproof is sound only because the verifier
  derives the exact sorted witness-ID set from the committed terminal relation
  before recomputing the Merkle root. It is not a generic replacement for
  indexed multiproofs: generic transcript-query proofs must continue to carry
  and check their opened indices, because their indices are part of the
  challenge-derived proof instance.
- The terminal query plan is now verifier-derived and commitment-bound, which
  removes serialized-query steering. It does not by itself justify the 60-bit
  soundness claim; the final backend still needs the algebraic proximity or
  sumcheck argument that makes sampled openings imply global relation
  satisfaction.
- The current NPO production verifier avoids a sampled-row soundness gap by
  checking all supported rows, not by relying on a proximity test. This is
  acceptable as a row-validity checkpoint for the measured terminal circuit, but
  it is linear in the supported-NPO witness opening surface and therefore not
  the literature-style terminal PCS/proximity construction.
- Tip5 NPO callsites now bind row mode (`new_start`, `merkle_path`) and MMCS
  direction-bit witness IDs into the backend relation digest. Production NPO
  verification opens those direction bits, rejects non-boolean values, enforces
  normal-chain carry lanes, Merkle digest carry lanes, and zero capacity lanes,
  and serializes only Merkle sibling hidden lanes.
- The sampled primitive and supported-NPO local proofs check real committed
  witness openings, but they are only components. A witness that violates a
  small number of unsampled primitive or NPO rows, or violates a global
  consistency/proximity relation, is not ruled out by these components alone.
  The production supported-NPO proof no longer has this sampled-row gap; it
  checks every supported NPO row, at the cost of a larger Merkle multiproof.
- The quadratic residual oracle checks sampled zero residuals and gives the
  future primitive backend a concrete composition oracle. The consistency proof
  now ties sampled residual values back to sampled witness values, so a residual
  oracle cannot be substituted independently at sampled rows. It does not yet
  prove global residual/witness consistency across unsampled rows.
- The residual fold proof gives the primitive residual oracle a transcript-bound
  folded global check: a nonzero residual vector must survive random folding or
  break sampled fold-layer consistency. This is meaningful progress toward the
  terminal backend's global argument, but the final production proof still needs
  a complete soundness calculation and global residual/witness consistency.
- Transcript-derived query plans now skip duplicate indices whenever the domain
  can support the requested count. The real Tip5 L0 terminal measurement asserts
  that the production combined-validity fold uses 15 distinct rows, so the
  pure-query soundness accounting is not silently weakened by duplicate samples.
  The public production verifier also rejects production terminal relations
  whose active query domains have fewer rows than `num_queries`; the duplicate
  fallback is retained only so tiny local regression circuits do not hang.
- The aggregate combined-validity fold verifier now has explicit adversarial
  coverage for the final boundary: a tampered nonzero final value with the
  stale zero root is rejected by final-root mismatch, and a re-rooted nonzero
  one-row aggregate fold is rejected by the final-value nonzero check.
- The NPO validity fold proof gives supported NPO rows the matching
  transcript-bound validity-vector checkpoint. A nonzero NPO validity vector
  must survive random folding or break sampled fold-layer/row consistency. This
  remains useful local/checkpoint coverage, but production no longer relies on
  it as the supported-NPO soundness boundary. The final production proof still
  needs a polynomialized Tip5/recompose relation and full soundness accounting
  to replace the exhaustive Merkle-opening verifier.
- Direct NPO validity consistency verification now validates the referenced fold
  commitment schedule before deriving consistency query rows. This keeps the
  direct verifier entrypoint fail-closed even if it is called outside the
  aggregate `TerminalLocalProof` verifier.
- Direct and aggregate NPO validity consistency verification also re-verifies
  each sampled `TerminalNpoOpening` against the witness commitment. A folded
  zero validity row is therefore not enough by itself; malformed/missing
  witness openings or stale Merkle paths are rejected at the consistency layer.
  The production exhaustive NPO verifier applies the same row semantics to every
  flattened supported NPO row and rejects missing/stale witness multiproof
  openings.
- The binding-only terminal certificate checker is private. The public
  production verifier now accepts only typed `TerminalProductionProof` bodies
  after no-trailing-bytes decoding and relation verification. It rejects
  malformed production bodies and rejects `LocalCheckpoint` certificates by
  proof-kind mismatch.
- `TerminalLocalProof` prevents a proof body from omitting one implemented local
  component while still passing a single local verifier entrypoint. It is still
  a local-proof envelope; it must be extended with the global
  proximity/sumcheck proof before it can carry production terminal soundness.
- The mixed combined-validity verifier has explicit regression coverage for
  branch confusion (`Quadratic` vs `Npo`) and NPO-index confusion. This protects
  the current aggregate local proof from accepting a row opened under the wrong
  relation branch, but it is not a substitute for a polynomial IOP.
- The current batch-STARK L1 profile historically used `lb=4, nq=9,
  query_pow=24`, but that does not satisfy the pure-query terminal requirement.
  A production terminal profile must get the full 60-bit floor from FRI queries.
- The native terminal production checkpoint does not count query-PoW toward
  production soundness. Its active profile is `log_blowup=4, num_queries=15,
  query_pow_bits=0`, for exactly 60 pure-query Johnson bits. Nonzero terminal
  `query_pow_bits` are rejected rather than maintained as a lower-query or
  mixed-accounting path.
- Fixed-int bincode serialization is size-only: it changes the Rust helper's
  byte encoding and rejects trailing bytes on decode, but does not alter the
  proof relation, Fiat-Shamir transcript, FRI parameters, or public inputs.
- The terminal production checkpoint is now 89,861 bytes, or 87.8 KiB, with 60
  pure-query bits and exhaustive supported-NPO verification. It reached the
  ~100 KiB size target through structural proof-body changes, especially
  omitting verifier-derived witness indices from the exhaustive NPO multiproof
  and packing verifier-known boolean MMCS direction-bit openings, not generic
  compression, digest dictionaries, fixed-width integer serialization,
  Pearl/Plonky2-style Merkle path compression, or another batch-STARK layer.
  The remaining size work is a real polynomialized NPO/proximity backend that
  preserves the checks above without carrying exhaustive NPO witness openings.
- Completion status: the measured production certificate now meets the user's
  size and 60-bit pure-query constraints, but the broader goal remains open
  until the terminal backend either implements the final polynomial/proximity
  argument for supported NPO/table rows or the project explicitly accepts the
  current exhaustive-Merkle NPO verifier as the production terminal backend with
  a complete, written soundness theorem.

## Certificate Binding Requirements

A terminal certificate must commit to:

1. protocol version and protocol id;
2. terminal proof parameters, including soundness bits;
3. verifier-circuit fingerprint;
4. verifier/proving key digest;
5. full public input vector, or a digest constrained to the full vector inside
   the verifier circuit;
6. transcript domain separators for AI-PoW terminal compression;
7. an aggregate binding digest tying the metadata, public values, and proof
   body digest together;
8. all commitment roots and challenge material needed by the terminal verifier.

The verifier must reject parameter drift. No proof may select a smaller
soundness tier, alternate transcript, alternate verifier circuit, or alternate
public-input digest while retaining production metadata.

## Backend Requirements

The compact backend must be native over the current field/circuit stack:

- consume the Goldilocks recursive verifier `p3_circuit::Circuit` and the
  assignment exposed through `TerminalWitness`;
- avoid producing another multi-table batch-STARK as the terminal artifact;
- support the non-primitive operations used by the recursive verifier
  circuit, especially Tip5/challenger and MMCS verification paths;
- use the canonical production profile with 60 pure-query soundness bits;
- serialize into a Nockchain-owned proof body, not a generic opaque external
  proof blob.
- prove the 1,884-constraint / 668-NPO-row terminal relation above without
  exposing the 3,043-value witness as certificate body material;
- derive all terminal challenges after binding the terminal relation digest,
  backend relation digest, public input digest, NPO type keys, Tip5 5-round
  variant, and proof-parameter profile.

## Implementation Plan

1. Keep the typed compact production certificate as the active terminal
   checkpoint: 60 pure-query bits, zero terminal PoW bits, 5-round Tip5 in the
   recursive verifier, primitive sparse-R1CS row-product proof, and exhaustive
   supported-NPO verification.
2. Replace the exhaustive Merkle-opening supported-NPO proof with a native
   polynomialized Tip5/recompose argument that preserves exact call-site
   binding and relation-digest binding.
3. Add the final proximity/PCS backend for the primitive and NPO relations
   under an explicit 60-bit-or-higher soundness calculation while preserving the
   canonical production parameter tuple unless a new production profile is
   explicitly specified and measured.
4. Measure proof bytes, prove time, verify time, and soundness bits after every
   major relation/proof-shape change.
5. Run the production AI-PoW `prod_recursion_measure` workload with the compact
   terminal path.
6. Promote only after the compact terminal meets:
   - final recursive certificate near 100 KiB;
   - the canonical 60-bit profile from queries, without query PoW;
   - recursive proving near 30 s;
   - rejection tests for public-input, parameter, circuit-fingerprint, relation
     digest, and NPO-row/call-site swaps.

## Non-Goals

- Do not add a low-soundness testing terminal path.
- Do not expose a second production proof shape without explicit certificate
  metadata and verifier dispatch.
- Do not call the backend after an external system or depend on external proof
  code.
- Do not keep stacking the current batch-STARK terminal in the hope of reaching
  100 KiB.
