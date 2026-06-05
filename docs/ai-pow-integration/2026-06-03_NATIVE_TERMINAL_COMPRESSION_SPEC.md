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
- fixed-int bincode for the current integrated terminal backend was re-tested
  on 2026-06-05 and rejected. The heavy integrated-LogUp checkpoint grew from
  `240,160` bytes / `234.5 KiB` under postcard to `259,618` bytes /
  `253.5 KiB` under fixed-int bincode. The small typed production checkpoint
  also grew from `904` bytes to `1,510` bytes. The active production proof-body
  codec therefore stays postcard with explicit no-trailing-bytes decoding.

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
  backend-specific proof body bytes. The public certificate path is production
  only and uses `TerminalProofKind::Production`; the local checkpoint kind and
  helpers are compiled only for tests as internal regression machinery, not as
  an integration or wire path.
- `TerminalProofParameters`: production terminal parameters committed before
  any backend challenge is sampled. The active native-terminal checkpoint
  profile is `security_bits=60, log_blowup=4, num_queries=15,
  query_pow_bits=0`, whose pure-query Johnson accounting is exactly 60 bits.
  Public production verification rejects other parameter tuples, even if their
  `log_blowup * num_queries` product also reaches 60 bits.
- `TerminalProximityProfile`: the transcript-bound terminal proximity schedule
  that the production backend must implement. The current code binds the
  two-adic FRI-style schedule `log_blowup=4`, `num_queries=15`,
  `query_pow_bits=0`, `max_log_arity=3`, and `log_final_poly_len=0` inside the
  relation profile and backend relation digest. Prelude verification rejects
  parameter tuples that do not match this profile, so overprovisioned or
  alternate schedules cannot silently share a transcript. The proximity profile
  still gets its 60 production bits only from pure FRI queries
  (`log_blowup * num_queries = 4 * 15`); no query proof-of-work bits are
  counted.
- `TerminalProofPrelude`: the first proof-body transcript object. It binds the
  terminal proof parameters, compiled relation profile including the
  component-expanded supported-NPO validity domain, full public-values digest,
  backend commitment digests, canonical zero terminal query-PoW nonce, and a
  domain-separated Tip5 challenge digest.
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
  relation. Production exhaustive NPO assignment-witness openings use this
  form: the verifier derives the exact sorted witness-ID set from every
  supported NPO callsite, and shifts those indices past the assignment vector's
  `[1 || public]` prefix, so serializing those assignment indices would be
  redundant. The proof also packs verifier-known boolean MMCS direction-bit
  values into a bitset and reconstructs canonical field values before Merkle
  root checking.
- `TerminalQueryPlan`: verifier-derived query indices for a committed terminal
  oracle. Query positions are sampled from the bound terminal prelude and oracle
  root; they are not accepted from serialized proof-body data. When the queried
  domain can provide at least `num_queries` rows, query derivation samples
  without replacement so the 15-query production profile yields 15 effective
  row checks instead of a smaller duplicate-collapsed set. The public
  standalone query-opening verifier treats its prelude commitment vector as the
  exact one-root shape `[oracle_root]`, rejecting both stale roots and extra
  unused roots that could otherwise steer sampled rows. Terminal FRI component
  verifiers instead require their expected root sequence to appear contiguously
  inside the transcript-bound prelude commitment vector. This permits one
  prelude to carry multiple FRI component roots while still forcing the proof
  body to use the same prelude challenge digest; a proof built under the
  one-root prelude does not verify under a larger prelude because the FRI
  transcript changes.
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
- `TerminalNpoExhaustiveResidual` oracle: a backend-target residual vector for
  supported NPO rows whose components match the production exhaustive row
  verifier, not just the older sampled input/output validity oracle. It includes
  Tip5 input/output residuals, chained hidden-lane residuals, Merkle
  capacity-zero residuals, MMCS direction-bit booleanity, and recompose
  input/output residuals. `TerminalNpoResidualComponentRef` maps every flattened
  component index back to its NPO row, row-local component offset, residual
  kind, limb, and basis component.
- `TerminalNpoPolynomialTable`: the deterministic supported-NPO witness table
  contract for the final backend. Each row carries verifier-derived metadata
  (`npo_index`, row kind, local row, Tip5 mode bits), witness-derived local
  values (call-site inputs/outputs, hidden Tip5 lanes, MMCS bit), the row's
  offset in the flattened residual domain, and residual component values in the
  exact `npo_exhaustive_residual` oracle order. Tests require this table's
  residual vector to match the committed exhaustive residual oracle and require
  MMCS-bit booleanity failures to surface in both row-local and flattened
  residual views.
- `TerminalNpoPolynomialColumnLayout` /
  `TerminalNpoPolynomialColumns`: the fixed polynomial-column projection of
  `TerminalNpoPolynomialTable`. The layout is verifier-key derived and absorbed
  into `TerminalBackendRelationDigest`; it fixes metadata columns, row-kind
  selectors, Tip5 mode bits, padded input/output value and present-bit columns,
  hidden Tip5 lane value and present-bit columns, MMCS direction-bit columns,
  and padded residual value and present-bit columns. Tests reconstruct the
  flattened exhaustive residual oracle from these columns and require MMCS-bit
  tampering to appear in both the local MMCS column and the residual columns.
- `terminal_npo_polynomial_verifier_derived_columns_goldilocks`: the
  verifier-side fixed-column projection for deterministic NPO table columns.
  It fills row metadata, row-kind selectors, mode bits, input/output/hidden
  present bits, MMCS-bit present flags, and residual-present shape directly
  from the verifying key, leaving witness-value columns and residual-value
  columns unset. This is the formal boundary used by the optimized value-column
  FRI path: deterministic selectors and table shape are evaluated by the
  verifier, not committed by the prover. Tests compare these columns against
  the witness-materialized full table and check basis-column interpolation on
  the same terminal two-adic FRI domain.
- `TerminalNpoPolynomialFriProfile`,
  `TerminalNpoPolynomialFriOpeningProof`, and the terminal FRI PCS builder: the
  NPO table columns now have a concrete Plonky3-compatible low-degree
  commitment target. Since terminal FRI commits Goldilocks codewords,
  verifier-circuit field columns are expanded into Goldilocks basis columns and
  padded to the verifier-key-derived two-adic row domain, then committed with
  recursive 5-round Tip5 and the canonical `log_blowup=4`, `num_queries=15`,
  `query_pow_bits=0` schedule. The profile now distinguishes the full fixed
  table from the witness-value column subset so deterministic metadata, present
  bits, and residual columns can be measured separately from witness-bearing
  values. The typed opening proof seeds the FRI challenger with the
  terminal-prelude challenge digest, public-values digest, proof parameters,
  zero query-PoW nonce, column-set selector, and full NPO FRI profile before
  observing the column commitment and sampling the extension opening point. A
  regression test commits D=2 recompose NPO columns through this native FRI
  PCS, verifies the typed proof, round-trips its serialized form, and rejects
  tampered opened values, stale profile metadata, stale prelude roots, and
  preludes that omit the expected FRI root sequence. The standalone NPO
  polynomial FRI prelude still uses the single terminal FRI Merkle-cap root for
  the committed column set; when a larger shared FRI prelude is used, the proof
  must be generated under that larger prelude because the challenge digest is
  different. The proof now stores its inner FRI opening directly in the
  terminal-compressed wrapper, so full-table and witness-value NPO FRI
  checkpoints no longer serialize raw Merkle path material. This is the
  proximity substrate for the final backend; production still does not accept
  it as the full NPO relation proof until the row-polynomial constraints are
  connected to the FRI openings.
- `TerminalNpoPolynomialFriOpenedColumns`: the verifier-derived handoff from a
  verified NPO FRI opening to future row-polynomial checks. Verification now
  returns the transcript-derived opening point, the verifier-selected column
  labels for the active column set, and the opened Goldilocks-basis values in
  terminal challenge field form. Existing public verify methods still return
  `()` for compatibility, while `verify_and_open_*` methods expose the checked
  openings. Tests require the value-column opening labels to contain only
  witness-bearing columns and require the full-table and value-column profiles
  to derive distinct opening points.
- `verify_terminal_npo_polynomial_value_padding_opening_goldilocks`: the first
  value-column relation consumer of the FRI handoff. It verifies the
  value-column FRI proof and enforces zero openings only for columns whose
  verifier-derived present-bit column is identically zero on the whole NPO
  domain. This deliberately does not claim the general AIR constraint
  `(1 - present(X)) * value(X) = 0` at an out-of-domain FRI point; mixed
  present-bit columns need a quotient/vanishing-polynomial argument before that
  relation is sound. Tests build a proof that is FRI-valid over a malformed
  `mmcs_bit` value column but relation-invalid because the verifier-derived
  `mmcs_bit_present` column is globally zero, and require the padding check to
  reject it.
- `TerminalNpoPolynomialPaddingQuotientProof`: the quotient-backed value-column
  checkpoint for mixed padding, MMCS direction-bit booleanity, and Goldilocks
  recompose rows. The prover commits the witness-value columns, samples a
  terminal challenge, commits one extension-valued quotient column flattened
  into Goldilocks basis columns over a verifier-derived disjoint domain, then
  opens both commitments at the same transcript-derived point with one terminal
  FRI proof. Verification recomputes the verifier-derived present-bit and
  recompose-selector polynomials at that point and checks the batched padding
  identity `sum alpha^i * (1 - present_i(zeta)) * value_i(zeta) =
  quotient(zeta) * Z_H(zeta)`, plus MMCS-bit constraints `present(zeta) *
  bit(zeta) * (bit(zeta) - 1)` for the base lane and `present(zeta) *
  bit_tail(zeta)` for non-base lanes, plus Tip5 chain-start zero constraints
  for verifier-selected hidden lanes under `mode_new_start`, plus Merkle
  capacity-zero constraints forcing hidden Tip5 lanes 10 through 15 to zero
  under verifier-derived
  `is_tip5 * mode_merkle_path * hidden_tip5_present_limb` selectors, plus
  recompose constraints forcing non-base input tails to zero and
  `output_basis_i = input_i_base` under the combined
  `is_recompose + is_recompose_coeff` selector. The quotient domain is twice
  the value-column trace domain so the degree-3 booleanity relation has a
  low-degree quotient. Its standalone prelude commitment vector is exactly the
  witness-value FRI root; the quotient root is not prelude material because it
  depends on the value-root-derived folding challenge and is instead observed
  in the PCS transcript before the opening point is sampled. The proof now
  stores the terminal FRI opening in the compressed terminal wrapper rather
  than serializing raw path material; the focused 2-row fixture measured
  `8,378` bytes / `8.2 KiB`, with restored raw FRI payload `15,482` bytes /
  `15.1 KiB` and stored compressed FRI payload `8,153` bytes / `8.0 KiB`.
  This is the Plonky3-style
  quotient/vanishing-polynomial form needed for mixed present-bit padding,
  MMCS direction-bit booleanity, Tip5 chain-start zero lanes, Merkle
  capacity-zero lanes, and recompose value-column semantics; it is still a
  checkpoint, not yet the complete production NPO relation proof.
- `TerminalNpoPolynomialRecomposeResidualQuotientProof`: a FRI-native
  row-relation checkpoint tying committed recompose residual-value columns to
  the committed prover-dependent NPO value columns and verifier-derived
  residual-present selectors. The prover commits the 89 prover-dependent
  witness/residual columns, derives a folding challenge from that root, commits
  one residual-relation quotient column over the doubled disjoint domain, and
  opens both at one transcript-derived point. The quotient checks, under the
  verifier-derived `is_recompose + is_recompose_coeff` selector, that each
  `RecomposeInput` residual slot equals the non-base input limb basis
  component, that each `RecomposeOutput` residual slot equals
  `output_basis_i - input_i_base`, and that residual-value extension tails are
  zero. The prover-side quotient builder originally used naive pointwise
  Lagrange evaluation on the quotient domain and measured `55.9 s`; it was
  replaced with a Plonky3-style `coset_lde_batch` pass over the required
  selector/value/residual basis columns, reducing the real-circuit prove time
  to `1.852 s` without changing proof bytes. This checkpoint is intentionally
  scoped to recompose residual equality; Tip5 permutation algebra and
  predecessor-chain residuals still need their own quotient or lookup-backed
  relation.
- `TerminalNpoPolynomialColumnOracleSet`: the commit-ready 5-round Tip5 oracle
  set for those fixed columns. Each column uses a verifier-derived
  `npo_polynomial_column/<column-label>` oracle label and the shared row count.
  This is a transcript-binding checkpoint for the final backend; it is not a
  substitute for the missing low-degree/proximity proof.
- `TerminalNpoPolynomialColumnResidual` evaluation: the verifier-side row
  predicate reconstructed from fixed columns. It validates row-kind selectors,
  mode bits, present bits, zero padding, duplicate witness-slot consistency,
  hidden Tip5 lane embedding, MMCS-bit binding, Tip5 chain state, and
  recompose coefficient reconstruction, then requires the derived residual
  vector to match the committed residual columns. Tests tamper Tip5 output
  columns, MMCS-bit columns, and D2 recompose input columns.
- `TerminalNpoTip5AirTrace` bridge: a deterministic table source for the
  full 5-round Tip5 permutation relation. It derives one Tip5 AIR input row
  from each terminal Tip5 NPO row, then generates the lookup-free
  `Tip5PermAir` trace using the existing KAT-anchored generator. Tests bind
  the generated AIR trace back to the terminal table inputs and terminal
  exposed outputs. This is not yet a production proof component; it is the
  table source for the pending Tip5 permutation quotient/FRI backend.
- `TerminalNpoTip5LookupAirTrace` bridge: the optimized permutation table
  source for the same terminal Tip5 rows. It generates the narrower global-bus
  lookup AIR main trace plus verifier-fixed preprocessed lookup table, with
  permutation rows placed after the 256 lookup table rows. Tests bind those
  permutation rows to the same terminal-derived Tip5 inputs as the lookup-free
  bridge. This is the preferred table source for the eventual production
  permutation backend because it avoids the lookup-free trace's boolean-bit
  column blowup. Its verifier-derived trace profile is absorbed into
  `TerminalBackendRelationDigest`, binding the lookup-table row count, main
  trace dimensions, preprocessed table dimensions, permutation-row offset,
  max constraint degree, and 5-round Tip5 round count without serializing those
  constants in the production certificate. The prover-controlled main trace is
  now commit-ready as one terminal oracle per column under verifier-derived
  `npo_tip5_lookup_air_main_col/<index>` labels; the verifier-fixed
  preprocessed lookup table is not committed by the prover.
  The lookup trace profile now carries a Tip5 digest of the verifier-fixed
  preprocessed `(is_table, tin, tout)` rows, including padded zero rows. That
  digest is absorbed into the backend relation digest and every terminal FRI
  transcript using the lookup trace profile, so a profile-compatible
  substitution of the fixed L-table cannot share a terminal statement or
  transcript.
  A terminal LogUp accumulator checkpoint now mirrors the Plonky3 LogUp
  rational-sum equation with transcript-derived `(alpha, beta)` after the
  committed full-main lookup trace root. It computes query byte-pair terms from
  committed main-trace columns and subtracts verifier-fixed table
  multiplicities derived from the bound L-table constants; focused tests show
  the honest accumulator is zero and that tampered S-box image bytes, stale
  table multiplicities, stale fixed-table digests, and stale table-row counts
  fail. This pins the terminal lookup-table semantics for the final proof, but
  it is not itself the committed low-degree running-sum/proximity argument.
- `TerminalNpoTip5LookupLogupQuotientProof`: a committed low-degree split
  LogUp checkpoint for the same byte-table relation. The prover commits the
  LogUp-relevant lookup trace projection (`KIND`, `TMULT`, and every split
  `(b,c)` byte column; 322 columns instead of the full 438-column main trace),
  samples one shared LogUp `(alpha, beta)` from that trace root, groups the
  161 interactions into 54 extension-valued running-sum accumulator columns
  with three cleared-denominator terms per group, absorbs their final
  cumulatives, samples `gamma` to fold the per-group first-row,
  transition-row, and final-row constraints, then commits one extension-valued
  quotient over a degree-4 disjoint domain. Verification recomputes the trace,
  accumulator, and quotient profiles from the verifying key, rejects stale
  LogUp-projection trace roots, checks the final cumulatives sum to zero, opens
  the accumulator at both `zeta` and the domain successor `g*zeta`, evaluates
  the verifier-fixed `(is_table,tin,tout)` polynomials at `zeta`, maps
  projected trace openings back to the full AIR column indices required by each
  interaction group, and checks the folded quotient relation under the
  production 60-bit FRI tuple. The focused regression test round-trips the
  proof and rejects stale trace roots, stale accumulator/quotient profiles,
  missing accumulator point openings, and tampered accumulator or quotient
  openings. Measured as a standalone component, it is `144,430` bytes /
  `141.0 KiB` with debug-profile `prove=23.280s`, `verify=87.1ms`; the compact
  FRI payload is `133,248` bytes / `130.1 KiB`, down from a restored raw FRI
  payload of `157,483` bytes / `153.8 KiB`. This closes the committed LogUp
  soundness checkpoint in isolation, and grouping saves another 28.5 KiB
  relative to the LogUp-projected split checkpoint, but appending it to the AIR
  algebra and boundary proofs is still not the final ~100 KiB route. The
  verifier also accepts the same LogUp relation over the full-main trace
  commitment, rejecting unsupported trace column sets; the focused full-main
  regression measures `176,429` bytes / `172.3 KiB`, with compact FRI payload
  `161,232` bytes / `157.5 KiB`, so full-main LogUp is a shared-trace
  ingredient for a future merged AIR+LogUp proof rather than a standalone size
  improvement. `TerminalNpoTip5LookupAirLogupQuotientProof` now performs that
  merge as a checkpoint: it commits the full-main trace once, derives separate
  AIR and LogUp folding challenges from one transcript, commits the AIR
  quotient, LogUp accumulator, and LogUp quotient, and opens all four oracles
  at one shared FRI point. With the earlier binary terminal FRI schedule
  (`max_log_arity=1`), the focused regression measured `185,396` bytes /
  `181.1 KiB`, with compact FRI payload `170,003` bytes / `166.0 KiB`,
  `prove=40.258s`, `verify=101.1ms`. A direct schedule sweep under the same
  60 pure-query bits measured:

  | `max_log_arity` | Candidate bytes | Compact FRI bytes | Prove | Verify |
  | --- | ---: | ---: | ---: | ---: |
  | 2 | `160,269` / `156.5 KiB` | `144,886` / `141.5 KiB` | `40.963s` | `83.6ms` |
  | 3 | `146,972` / `143.5 KiB` | `131,576` / `128.5 KiB` | `40.754s` | `80.4ms` |
  | 4 | `158,264` / `154.6 KiB` | `142,885` / `139.5 KiB` | `40.884s` | `82.6ms` |

  In this component-local sweep, `max_log_arity=3` was smaller than binary and
  arity-16 without increasing the focused prove-time measurement. A focused
  optimized run of the same arity-8 proof measured
  `prove=4.155s`, `verify=21.6ms`; this component is therefore not the latency
  blocker by itself, but the final production backend still has to measure the
  whole recursive certificate end to end against the roughly 30s orphan-rate
  target. This is a substantial reduction versus
  appending standalone full-main AIR algebra plus full-main LogUp proofs, but
  it is still too large to be the final ~100 KiB production NPO theorem by
  itself. A column-dependency audit rules out a sound smaller projection for
  this combined relation: the AIR algebra quotient uses the full-main `KIND`
  column, the 16 input lanes, and every per-round split, inverse, power, and
  round-output column, omitting only `TMULT`; the LogUp quotient needs `TMULT`
  plus `KIND` and the split byte columns. Their union is the full-main trace,
  so a smaller combined projection would either leave `TMULT` unbound for
  LogUp or drop internal AIR witness columns. The remaining size route is a
  composition-polynomial proof that avoids opening the full trace at every FRI
  query, not a narrower column projection of the same relation.
  A follow-up shared-prelude checkpoint first built only the merged
  residual-zero+recompose+value-bridge proof and the full-main AIR+LogUp proof
  under one transcript-bound prelude containing the selected+lookup root
  sequence followed by the full-main trace root sequence. That two-component
  checkpoint serialized to `168,799` bytes / `164.8 KiB`, with `12,356` bytes
  / `12.1 KiB` in the merged value-bridge compact FRI and `140,151` bytes /
  `136.9 KiB` in the AIR+LogUp compact FRI. It proved the FRI components can
  share one prelude and one transcript prefix, but an adversarial audit showed
  it still accepted an honest value-bridge component plus a different
  internally valid AIR+LogUp trace component.

  The next linked checkpoint added a trace-to-IO projection proof and the
  trace-domain support bridge under one prelude ordered as
  `[selected+lookup, full-main trace, trace-domain lookup IO,
  trace-domain NPO IO]`. The projection proof opens the full-main trace and
  the terminal-IO projection at one transcript point and checks a random
  linear combination of the 26 terminal IO lanes, while the support bridge
  rejects a divergent trace when its trace-domain NPO projection is honestly
  derived from the NPO columns. The optimized-debug focused measurement is
  `330,247` bytes / `322.5 KiB`, with compact FRI payloads: merged value
  bridge `11,888` bytes / `11.6 KiB`, AIR+LogUp `140,817` bytes /
  `137.5 KiB`, trace-to-IO projection `98,925` bytes / `96.6 KiB`, and
  support bridge `51,304` bytes / `50.1 KiB`; `prove=5.544s`,
  `verify=56.1ms`. A stronger adversarial audit then forged that trace-domain
  NPO IO projection from modified NPO value columns matching a divergent AIR
  trace, while keeping the merged selected/value proof honest; the linked
  verifier accepted the forged projection checkpoint at `330,010` bytes /
  `322.3 KiB`, proving the missing obligation was not just a documentation
  caveat.

  The current exhaustive-linked checkpoint closes that forged-projection audit
  with `TerminalNpoTip5LookupTraceDomainNpoIoExhaustiveBridgeProof` and now
  merges the AIR+LogUp proof, trace-to-IO projection, and support/NPO bridge
  into `TerminalNpoTip5LookupAirLogupTraceIoSupportProof`. The combined proof
  keeps the four-root prelude span `[selected+lookup, full-main trace,
  trace-domain lookup IO, trace-domain NPO IO]`, samples the support-fold
  challenge only after the trace/lookup-IO/NPO-IO commitments are fixed, checks
  the full-main trace-derived terminal-IO fold at the same sampled point used
  by the support quotient, and keeps the quotient identity that makes lookup
  IO vanish off the permutation window while matching NPO IO on the window.
  The exhaustive bridge still opens the selected+lookup commitment on every
  verifier-derived Tip5 NPO row, opens the trace-domain NPO-IO projection on
  the matching permutation rows, and checks equality of the 26 IO lanes. The
  honest focused measurement is `240,619` bytes / `235.0 KiB`, with compact
  FRI payloads: merged value bridge `11,888` bytes / `11.6 KiB`, combined
  AIR+LogUp trace-IO/support proof `178,820` bytes / `174.6 KiB`, and
  exhaustive bridge `31,589` bytes / `30.8 KiB`; `prove=10.283s`,
  `verify=332.6ms` in the three-test backend audit run. This saves `97,685`
  certificate bytes / `95.4 KiB` relative to the previous `338,304` byte
  exhaustive-linked checkpoint, and `122,194` bytes / `119.3 KiB` relative to
  the earlier split projection/support checkpoint, while preserving the same
  15-query, zero-PoW production profile. The divergent-prelude audit now
  rejects inside the combined AIR+LogUp trace-IO/support proof, and the forged
  trace-domain NPO IO audit still rejects; the forged audit serializes to
  `312,633` bytes / `305.3 KiB` because it carries both exhaustive and LogUp
  rejection witnesses for the same forged state.

  The intended bridge is a LogUp-style multiset equality, not another
  standalone projection commitment. After absorbing the selected+lookup root,
  full-main trace root, trace-domain lookup-IO root, trace-domain NPO-IO root,
  fixed NPO row layout, fixed Tip5 row count, and production proximity profile,
  the verifier samples tuple-compression challenges for
  `(tip5_rank, io_limb, value)`. The row-domain side reads the lookup-IO suffix
  of the selected+lookup commitment on verifier-derived Tip5 NPO rows; the
  trace-domain side reads the NPO-IO projection on permutation-window rows.
  A logarithmic-derivative accumulator then proves the signed sums of
  reciprocals are equal, following Haboeck's LogUp lookup construction
  ([IACR ePrint 2022/1530](https://eprint.iacr.org/2022/1530.pdf)), while FRI
  continues to provide the low-degree/proximity layer
  ([BBHR18 FRI](https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ICALP.2018.14);
  [DEEP-FRI](https://arxiv.org/abs/1903.12243) is relevant background for
  proximity soundness but is not the active in-tree PCS). Fiat-Shamir
  soundness depends on sampling those challenges only after all bridge roots,
  profiles, final accumulator commitments, and the arity schedule are fixed,
  consistent with modern analyses of Fiat-Shamir for multi-round proofs
  ([Journal of Cryptology 2023](https://link.springer.com/article/10.1007/s00145-023-09478-y)).
  The production implementation should merge this accumulator and quotient
  with the existing AIR+LogUp/value-bridge openings; appending a separate
  proof would close the forged-projection audit but stay far above the 100 KiB
  certificate target. An in-tree standalone version now implements this
  signed two-domain LogUp bridge with grouped `(tip5_rank, io_limb, value)`
  denominators. With 3-lane groups it measures `71,679` bytes / `70.0 KiB`
  as a standalone bridge checkpoint, with compact FRI payload `68,797` bytes /
  `67.2 KiB`, `prove=2.902s`, and `verify=18.9ms`; inserted into the combined
  AIR+LogUp trace-IO/support linked checkpoint in place of the exhaustive
  bridge, it measures `279,419` bytes / `272.9 KiB`, worse than the
  exhaustive-linked `240,619` bytes / `235.0 KiB`. The new
  `TerminalNpoTip5LookupAirLogupTraceIoSupportNpoIoLogupProof` integrates the
  same row-domain/trace-domain LogUp accumulators and quotients into the shared
  AIR+LogUp trace-IO/support FRI opening. Its first integrated version measured
  `240,160` bytes / `234.5 KiB`, with compact FRI payload `208,782` bytes /
  `203.9 KiB`, `prove=7.174s`, and `verify=44.4ms` in the three-test backend
  audit run. A follow-up direct-IO support pass removed the separate
  trace-lookup-IO commitment and opening: the support quotient now evaluates
  the terminal-IO projection directly from the committed full trace at the
  quotient-domain points, while the verifier recomputes the same folded
  terminal IO from the opened full trace at `zeta`. This keeps the public
  relation identical but drops one committed oracle from the integrated
  transcript and prelude. A metadata pass then stopped serializing
  verifier-derived profiles in the integrated proof body; the verifier
  recomputes those profiles from the verifying key and canonical production
  proximity profile before seeding Fiat-Shamir, so transcript binding is
  unchanged while the redundant public surface shrinks. A grouped-MMCS pass
  then kept the same Fiat-Shamir staging but committed each stage's known
  matrices under one Plonky3 input MMCS root, with the existing proof fields
  carrying repeated roots and the verifier rejecting unequal repeats before FRI
  verification. At the canonical 15-query, zero-PoW 60-bit profile, that
  native/Rayon checkpoint measured `193,137` bytes / `188.6 KiB`, with compact
  FRI payload `164,238` bytes / `160.4 KiB`, `prove=6.896s` for the integrated
  merged+LogUp path and `verify=38.4ms`. The integrated FRI breakdown is
  `compact_input_batches=130,093`, `compact_commit_rounds=33,825`, and
  `compact_commits_final=320`; the largest remaining batch is the grouped full
  trace/NPO-IO opening at `76,609` bytes, mostly opened values rather than path
  material. A follow-up 2026-06-05 Tip5 lookup layout pass removed the x2/x3
  helper columns for power lanes and computes x^7 directly inside the
  kind-gated MDS relation. This shrinks the lookup main trace from 558 to 438
  columns, raises the lookup AIR max constraint degree from 4 to 8, and leaves
  the production proximity tuple unchanged at `log_blowup=4`, `num_queries=15`,
  `query_pow_bits=0`. The broader backend audit measured that integrated
  checkpoint at `186,319` bytes / `182.0 KiB`, compact FRI payload `159,145`
  bytes / `155.4 KiB`, `prove=10.882s`, and `verify=39.6ms`; the grouped
  full-trace/NPO-IO opening dropped to `62,644` bytes with 464 opened limbs per
  queried row. A follow-up direct trace-NPO-IO pass removed the redundant
  committed trace-domain NPO-IO oracle and the integrated support quotient. The
  trace side of the selected-vs-trace NPO-IO LogUp bridge now derives the 26
  terminal-IO lanes directly from the committed full trace opening, and the AIR
  quotient already enforces terminal IO zero off the permutation window via the
  window-vanishing factor. The transcript domain separator moved to v2 and the
  production FRI tuple remains `log_blowup=4`, `num_queries=15`, and
  `query_pow_bits=0`. At `max_log_arity=3`, the focused native/Rayon backend
  audit measured this checkpoint at `174,258` bytes / `170.2 KiB`, compact FRI
  payload `148,213` bytes / `144.7 KiB`, `prove=9.959s`, and `verify=44.3ms`;
  the largest input batch became the single full-trace opening at `60,828` bytes
  with 438 opened limbs per queried row, and the shared LogUp accumulator batch
  dropped to 38 opened limbs. A post-removal FRI arity sweep kept
  `log_blowup=4`, `num_queries=15`, and `query_pow_bits=0` fixed:
  `max_log_arity=4` improved that then-current integrated checkpoint to `171,895`
  bytes / `167.9 KiB`, compact FRI payload `145,405` bytes / `142.0 KiB`,
  `prove=10.130s`, and `verify=36.4ms`, while `max_log_arity=5` regressed to
  `177,449` bytes / `173.3 KiB`, compact FRI payload `150,237` bytes /
  `146.7 KiB`. At that checkpoint, `max_log_arity=4` was therefore the measured
  whole-certificate winner for the direct-integrated backend. A
  2026-06-06 byte-table LogUp grouping pass then raised the byte-LogUp group
  size from 3 to 7. This keeps the quotient degree at the existing Tip5 lookup
  AIR bound of 8 (`group_size + 1 = 8`) while reducing byte-LogUp accumulator
  width from 54 extension columns to 23. The focused native/Rayon checkpoint
  measured `165,185` bytes / `161.3 KiB`, compact FRI payload `141,775` bytes /
  `138.5 KiB`, `prove=9.786s`, and `verify=36.4ms`. A group size of 8 crossed
  the degree boundary and regressed to `176,332` bytes / `172.2 KiB` with
  `prove=16.071s`. This is a real size win but still not the final ~100 KiB
  route because opened full-trace values and FRI authentication paths dominate
  the payload. The tamper check still rejects a modified trace-derived NPO IO
  opening. The row-per-round Tip5 lookup trace pass then replaced the wide
  one-row-per-permutation lookup trace with five narrower round rows, kept the
  15-query / no-query-PoW production tuple, and measured the integrated
  direct-IO checkpoint at `116,129` bytes / `113.4 KiB`, compact FRI payload
  `97,300` bytes / `95.0 KiB`, `prove=21.089s`, and `verify=51.6ms`; the
  largest compact input batch is now `23,546` bytes. A same-day production
  parameter pass raised the trace-domain NPO-IO LogUp group size from 3 to 7,
  still within the degree-8 quotient bound, and reduced the production
  `max_log_arity` from 4 to 3. The focused native/Rayon integrated backend
  benchmark measured `103,768` bytes / `101.3 KiB`, compact FRI payload
  `85,120` bytes / `83.1 KiB`, `prove=23.313s`, and `verify=49.2ms`.
  Candidate sweeps rejected 15-way NPO-IO LogUp grouping (`119,822` bytes /
  `117.0 KiB`, `prove=27.773s`), `max_log_arity=5` (`112,738` bytes /
  `110.1 KiB`), `max_log_arity=2` (`115,773` bytes / `113.1 KiB`), and
  `log_final_poly_len=4`, which violates the Plonky3 FRI domain-height
  assertion for this backend. A structural pass also tested removing the
  `TIN[16]` carry columns from the row-per-round Tip5 trace and replacing the
  one-row final terminal-IO bridge with a two-row bridge: round 0 `IN[16]`
  supplies input limbs and the final round `OUT[10]` supplies output limbs. This
  reduced the full trace opening from 117 to 101 limbs, but it made the complete
  integrated checkpoint larger: `106,679` bytes / `104.2 KiB` at
  `max_log_arity=3`, and `105,042` bytes / `102.6 KiB` after retuning
  `max_log_arity=4`. The extra two-row support/proximity pressure outweighed the
  saved trace values, so the production layout keeps the carried-input one-row
  terminal-IO bridge. A fixed-width Merkle path digest packing pass then kept
  the same verifier proof shape but serialized compact terminal FRI path
  siblings as canonical 40-byte Goldilocks digests instead of postcard-varint
  field limbs. The focused native/Rayon integrated backend benchmark first
  measured the NPO-only checkpoint at `94,016` bytes / `91.8 KiB`, compact FRI
  payload `76,604` bytes / `74.8 KiB`, `prove=23.161s`, and `verify=48.6ms`.
  A production-candidate measurement then put the assignment root, primitive
  sparse-R1CS row-product proof, merged NPO value proof, and integrated
  Tip5/NPO-IO LogUp proof under one transcript prelude. The public production
  proof body now uses that primitive+polynomial-NPO backend. A hidden-output
  audit rejected deriving all NPO IO directly from the full trace: Merkle Tip5
  calls intentionally expose only selected output lanes, so the sound production
  proof commits a bundled trace matrix containing the full Tip5 lookup trace
  and the verifier-shape-masked trace-domain NPO-IO projection under one FRI
  root. This keeps the AIR proof over the full trace while making the
  selected-vs-trace LogUp multiset compare masked IO values. The current native
  / Rayon production-candidate checkpoint measures `99,289` bytes / `97.0 KiB`,
  integrated NPO compact FRI payload `80,885` bytes / `79.0 KiB`,
  `total_prove=23.688s`, and `total_verify=62.6ms`. The decompressor rejects
  non-canonical packed digest limbs before restoring the ordinary Plonky3 FRI
  proof, so digest packing is representation-only for the verifier algebra. An
  older 2026-06-05 integrated group-size sweep
  kept the 60-bit pure-query FRI tuple
  fixed and tested the accumulator width versus quotient-degree tradeoff
  directly: 1-lane groups measured `252,162` bytes / `246.3 KiB`, compact FRI
  `218,450` bytes / `213.3 KiB`, `prove=6.350s`; 7-lane groups measured
  `256,485` bytes / `250.5 KiB`, compact FRI `225,742` bytes / `220.5 KiB`,
  `prove=9.257s`; and 15-lane groups measured `263,254` bytes / `257.1 KiB`,
  compact FRI `232,773` bytes / `227.3 KiB`, `prove=13.499s`. A 26-lane
  single-group standalone sweep was also worse: `86,187` bytes / `84.2 KiB`,
  compact FRI payload `84,355` bytes / `82.4 KiB`, and `prove=17.634s`. The
  3-lane grouping remains the measured size winner for the integrated backend.
  The result is useful but still negative for the ~100 KiB target: the bridge
  is now integrated, so the next size reduction has to shrink the shared FRI
  oracle payload itself rather than moving the same LogUp bridge between
  standalone and shared proofs.
- `TerminalNpoTip5LookupFriOpeningProof`: a native terminal FRI opening
  checkpoint for the optimized Tip5 lookup AIR main trace. The prover commits
  the whole Goldilocks-valued lookup main matrix with recursive 5-round Tip5
  FRI/MMCS under the production proximity tuple (`log_blowup=4`,
  `num_queries=15`, `query_pow_bits=0`). The transcript domain separator binds
  the terminal prelude challenge digest, public-values digest, proof
  parameters, zero query-PoW nonce, the verifier-key-derived lookup trace
  profile, and the proximity profile before observing the FRI commitment and
  sampling the extension opening point. Verification recomputes the lookup
  profile from the verifying key, rejects profile/proximity mismatches,
  reconstructs canonical extension-field openings from serialized Goldilocks
  limbs, and returns verifier-derived column labels plus checked openings for
  future lookup-AIR quotient checks. Its standalone prelude commitment vector
  is the one FRI Merkle-cap root for the selected lookup column set; a stale
  IO/full-main root or a prelude that omits that root sequence is rejected
  before verifier challenges are sampled. A regression test round-trips the
  proof, rejects tampered opened values, stale profile metadata, and stale
  prelude roots, and verifies that a proof generated under a larger
  transcript-bound prelude can share that prelude with other terminal FRI
  components. This is still a backend checkpoint; production does not yet
  replace exhaustive NPO checking with this lookup trace commitment.
- `TerminalNpoTip5LookupFriColumnSet::TerminalIo`: a measured IO-projection
  variant for the same lookup trace. It commits only the terminal boundary
  columns, namely the 16 Tip5 input lanes and the first 10 final-round output
  lanes consumed by supported NPO rows. The column-set selector is absorbed into
  the FRI transcript, and the full-main verifier rejects IO-projection proofs.
  This is not a standalone permutation proof because it does not bind the
  internal split/lookup/power/MDS witness columns; it is a size-floor checkpoint
  for designs that would combine boundary openings with a smaller quotient or
  batched AIR relation proof.
- `TerminalNpoTip5LookupIoZeroQuotientProof`: a quotient-backed support
  constraint for the terminal-IO lookup projection. The prover commits the
  26-column IO projection, samples a folding challenge, commits one
  extension-valued quotient over a disjoint doubled domain, and opens both at
  one transcript point. Its standalone prelude commitment vector is exactly the
  terminal-IO FRI root; the quotient root depends on the IO-root-derived
  folding challenge and is therefore observed later inside the FRI transcript.
  Verification checks
  `V_perm_window(zeta) * sum alpha^i * io_i(zeta) =
  quotient(zeta) * Z_H(zeta)`, where `V_perm_window` vanishes exactly on the
  verifier-derived terminal Tip5 permutation rows. This forces the IO
  projection to be zero on the lookup-table and padding rows. Tests round-trip
  the proof and reject a malicious trace that inserts a nonzero IO value into a
  lookup-table row. This still does not prove the internal Tip5 lookup-AIR
  transition relation, but it removes an otherwise unconstrained hiding surface
  from the 26-column projection.
- `TerminalNpoTip5LookupIoBridgeQuotientProof`: a quotient-backed bridge
  between the optimized lookup trace's 26-column terminal IO projection and a
  lookup-domain projection derived from the supported-NPO polynomial table. The
  NPO projection maps each verifier-derived Tip5 row to lookup rows starting at
  the fixed 256-row lookup-table offset, reconstructing the 16 input lanes from
  `input_value_*` when present and otherwise from `hidden_tip5_value_*`, then
  copying the first 10 `output_value_*` lanes. The proof commits both IO
  projections, samples a folding challenge, commits one quotient over a
  disjoint doubled domain, and verifies
  `sum alpha^i * (lookup_io_i(zeta) - npo_io_i(zeta)) =
  quotient(zeta) * Z_H(zeta)`. Tests round-trip the proof and reject a stale
  lookup trace whose terminal IO value no longer matches the NPO-derived
  projection. Its standalone prelude commitment vector is exactly
  `[lookup_io_root, npo_io_root]`, in transcript order, with the quotient root
  observed later after the folding challenge. This is the bridge needed to
  replace exhaustive NPO openings, but the NPO-derived projection still has to
  be tied to the final value-column proximity backend before production can
  rely on it.
- `TerminalNpoTip5LookupIoSupportBridgeQuotientProof`: a single-quotient
  batching of the two terminal-IO boundary relations above. After committing
  the lookup IO projection and the NPO-derived IO projection, the prover samples
  `alpha` to fold the 26 IO columns and `beta` to batch the bridge identity
  with the off-window support identity, then commits one extension-valued
  quotient over a disjoint doubled domain. Its standalone prelude commitment
  vector is also exactly `[lookup_io_root, npo_io_root]`; the quotient root is
  challenge-dependent and is observed later inside the FRI transcript.
  Verification checks
  `folded_diff(zeta) + beta * V_perm_window(zeta) * folded_lookup(zeta) =
  quotient(zeta) * Z_H(zeta)`. The focused regression test round-trips the
  proof, stores the compressed FRI payload directly, measures `67,600` bytes /
  `66.0 KiB` at the production `log_blowup=4, num_queries=15,
  query_pow_bits=0` profile, and rejects both a nonzero lookup-table-row IO
  value and a stale permutation-row lookup IO value. The stored FRI payload is
  `66,706` bytes / `65.1 KiB`, down from the restored raw payload's `96,338`
  bytes / `94.1 KiB`. This is the current preferred boundary-check candidate
  because it replaces the separate zero-support and bridge proofs with one FRI
  proof; it still is not a complete production NPO proof until the NPO-derived
  projection is tied to the final value-column proximity backend and the
  internal Tip5 lookup/AIR relation is enforced.
- `TerminalNpoTip5LookupAirAlgebraQuotientProof`: a quotient-backed checkpoint
  for the optimized Tip5 lookup AIR's local algebraic constraints. The prover
  commits the full lookup main trace, samples one folding challenge, commits an
  extension-valued quotient on a degree-8 disjoint domain, and verifies the
  folded kind-booleanity, split-byte recomposition, Goldilocks canonical guard,
  inlined power-lane x^7, and MDS/round-constant output identities, batched
  with `V_perm_window * terminal_io` so table/padding rows cannot hide terminal
  IO values in the full trace. Its standalone prelude commitment vector is
  exactly the full-main lookup trace FRI root; the quotient root is
  challenge-dependent and is absorbed later by the FRI transcript. The focused
  regression test stores the compressed terminal FRI payload directly and now
  measures `104,683` bytes /
  `102.2 KiB`, debug-profile `prove=1.712s`, `verify=15.9ms`, and rejects
  nonzero table-row terminal IO, tampered S-box image bytes, and a tampered
  power-lane input. The terminal compact-FRI payload is `97,004` bytes /
  `94.7 KiB` after decompression restores a `105,865` byte / `103.4 KiB`
  Plonky3 FRI proof.
  This closes a real internal-AIR algebra/support checkpoint, but it is too
  large to combine naively with the current terminal backend. Component accounting shows why: zeta openings are
  `7,425` bytes, transcript-query input row values are `52,553` bytes, input
  Merkle paths are `20,664` bytes, commit-phase sibling values are `8,146`
  bytes, commit-phase Merkle paths are `24,311` bytes, and the remaining
  commitments/final data are the balance. A measurement-only PCS floor for one
  extension-valued composition/quotient polynomial over the same 2048-row
  domain is `55,816` bytes / `54.5 KiB` at the same 60-bit FRI tuple; this is
  not a sound proof by itself, but it measures the terminal compact-FRI floor
  before reusable-profile overhead, and the decompressed
  floor proof verifies. This shows a composition-polynomial terminal backend is
  plausible only if it replaces the full-trace query openings rather than being
  appended as another component.
- `TerminalCompactCompositionFriProof`: the first-class compact FRI primitive
  for one terminal composition/proximity oracle. It binds the terminal prelude,
  production proximity tuple, oracle row count, extension-basis width, and a
  caller-supplied transcript label before committing the composition matrix and
  sampling the opening point. It serializes the path-compressed terminal FRI
  form and verifies by decompressing through the existing Plonky3 PCS verifier.
  The focused Tip5 AIR test measures this reusable proof object at `55,816`
  bytes / `54.5 KiB` for a 2048-row, one-extension-column composition oracle,
  and rejects a corrupted compact Merkle path. A separate nonconstant-oracle
  regression test measures `12,282` bytes / `12.0 KiB` for a 16-row fixture and
  rejects transcript-label substitution, profile substitution, and tampered
  opened values.
- `TerminalNpoExhaustiveResidualCompactCompositionProof`: a residual-zero
  checkpoint built on `TerminalCompactCompositionFriProof`. It maps the
  exhaustive Goldilocks residual vector into the first basis column of the
  compact composition matrix, pads the second basis column with zeroes, and
  requires every transcript-sampled extension opening to be zero. The focused
  regression test measures `17,018` bytes / `16.6 KiB` for a 17-row residual
  domain padded to 32 rows, debug-profile `prove=18.0ms`, `verify=17.8ms`, and
  rejects a valid FRI proof for nonzero residual values. This is still only a
  proximity/zero-opening checkpoint: it does not replace the production
  exhaustive path until a separate relation proof ties this composition
  polynomial to the committed `npo_exhaustive_residual` oracle and the
  witness-derived row table.
- `TerminalNpoPolynomialCompactResidualZeroProof`: a bound compact-FRI
  residual-zero checkpoint for the fixed NPO polynomial columns. It commits the
  randomized combined residual polynomial, opens that same FRI commitment at
  the transcript-derived zeta point and at the sampled row-domain points used
  by `TerminalNpoPolynomialSelectedColumnOpeningProof`, and checks those row
  openings against the residual-column linear combination. Verifier-derived
  metadata, selector, present-bit, and residual-present columns are
  reconstructed locally; the proof commits and opens only prover-dependent
  witness-value and residual-value columns, and the verifier rejects any
  missing, extra, or reordered selected column set before sampling challenges.
  The compact composition now stores all Goldilocks basis limbs of the verifier
  field, so the same proof path covers the real Goldilocks quadratic-extension
  Tip5-L0 verifier circuit instead of only base-Goldilocks fixtures. The
  focused regression test measures `8,763` bytes / `8.6 KiB` for the 2-row NPO
  polynomial fixture, debug-profile `prove=71.9ms`, `verify=60.9ms`, and
  rejects both tampered compact row openings, tampered extension limbs, and
  stale residual columns. On the real Tip5-L0 verifier circuit, the opt-in
  benchmark measures `376,642` bytes / `367.8 KiB`, of which `327,866` bytes
  are selected-column Merkle openings and `48,495` bytes are compact FRI
  material; the selected column commitments serialize to `7,903` bytes for
  `89` selected columns out of `186`. This closes the previous "zero proof not
  tied to residual columns at sampled rows" gap for this checkpoint and removes
  deterministic-column openings, but it also rules out appending this component
  as the production route: the final backend must avoid the sampled
  prover-dependent column-opening layer.
- `TerminalNpoPolynomialFriCompactResidualZeroProof`: the FRI-native successor
  checkpoint for the residual-zero layer. It commits the prover-dependent NPO
  columns as one FRI matrix under `TerminalNpoPolynomialFriColumnSet::ProverDependent`,
  samples a residual-column folding challenge after that selected FRI root is
  fixed, commits the folded residual-composition polynomial, then opens both
  commitments at one transcript-derived zeta point with one compressed
  terminal FRI proof. Verification reconstructs the selected labels from the
  verifying key, requires the selected FRI root sequence to be included in the
  transcript-bound prelude, checks the opened composition is zero, and checks
  that it equals the folded opened residual columns at zeta. This removes the
  sampled selected-column Merkle opening layer. The D2 focused fixture measures
  `6,360` bytes / `6.2 KiB`, with raw inner FRI `11,156` bytes and compressed
  inner FRI `6,101` bytes, and rejects a FRI-valid proof over nonzero residual
  columns. On the real Tip5-L0 verifier circuit, the opt-in benchmark measures
  `61,683` bytes / `60.2 KiB`, with `1,174` bytes of selected zeta openings,
  `60,372` bytes of compact FRI material, `prove=1.567s`, and
  `verify=0.496s`. This is still a checkpoint rather than a production NPO
  proof because the residual columns still need a quotient tying them to the
  witness-value columns and verifier-derived row relation over the whole NPO
  domain.
- `TerminalNpoPolynomialFriResidualZeroRecomposeProof`: a combined FRI-native
  checkpoint that opens the prover-dependent NPO table, the folded residual-zero
  composition polynomial, and the recompose residual quotient in one terminal
  FRI proof. The transcript samples the residual folding challenge and the
  recompose quotient challenge after the selected-column FRI root is fixed,
  then observes both derived commitments before sampling the shared opening
  point. Verification requires the selected FRI root sequence to be included in
  the transcript-bound prelude, checks that the folded residual opening is zero
  and matches the opened residual-value columns, and checks the recompose
  value/residual/selector quotient identity against verifier-derived fixed
  columns. The D2 focused fixture measures `10,475` bytes / `10.2 KiB`, with
  raw inner FRI `19,326` bytes and compressed inner FRI `10,143` bytes, and
  rejects stale roots, nonzero residual columns, and recompose value/residual
  mismatches. On the real Tip5-L0 verifier circuit, the opt-in benchmark
  measures `90,598` bytes / `88.5 KiB`, with `1,179` bytes of selected zeta
  openings, `89,177` bytes of compact FRI material, `prove=2.005s`, and
  `verify=0.645s`. This removes the duplicated selected-column opening and FRI
  query material from the standalone `60.2 KiB` residual-zero plus `79.4 KiB`
  recompose checkpoints, but it is still only the recompose/residual part of
  the production NPO backend. The full production replacement still needs the
  Tip5 permutation/lookup relation tied into the same terminal proximity
  theorem rather than appended as a separate large FRI proof.
- `TerminalNpoTip5LookupNpoRowsValueBridgeQuotientProof`: an NPO-row-domain
  value-binding checkpoint for the optimized lookup terminal IO. Instead of
  trusting a prover-supplied lookup-domain NPO projection, it commits the 26
  terminal IO columns on the supported-NPO row domain alongside the committed
  NPO witness-value columns. A single quotient proves that each lookup IO lane
  equals the verifier-derived selector-weighted value-column expression on
  Tip5 rows and is zero off those rows. Its standalone prelude commitment
  vector is exactly `[lookup_io_root, value_root]`, in transcript order; its
  quotient root is absorbed later after the folding challenge. The focused
  regression test now stores the compressed terminal FRI payload directly and
  measures `9,624` bytes /
  `9.4 KiB` for this binding component, rejects stale committed value columns,
  rejects stale lookup IO, and round-trips the proof.
  This closes the specific "NPO projection not tied to value columns" checkpoint
  in isolation, but production still needs the internal Tip5 lookup/AIR
  relation and a shared final proximity backend before replacing exhaustive NPO
  verification.
- `TerminalCompressedFriProof`: a terminal-only path-compressed wrapper around
  the existing Plonky3 `FriProof` shape. It follows the Plonky2 terminal
  compression model without using Plonky2 code: query indices are not
  serialized, the verifier must rederive them from the Fiat-Shamir transcript,
  and same-tree binary Merkle paths are sorted, deduplicated, and pruned by
  shared ancestors. On the NPO-row value-bridge checkpoint, the plain FRI
  payload measured `15,957` bytes / `15.6 KiB`; the compressed wrapper measured
  `9,046` bytes / `8.8 KiB` and decompresses to a proof accepted by the
  existing verifier. On the real 668-row Tip5-L0 NPO FRI candidates, the full
  table inner FRI payload compresses from `96,669` bytes to `75,736` bytes, and
  the witness-value-column inner FRI payload compresses from `80,471` bytes to
  `58,759` bytes; both decompressed proofs are accepted by the existing
  verifier. The optimized Tip5 lookup full-main opening compresses its inner
  FRI from `130,206` bytes to `109,977` bytes, and the terminal-IO projection
  compresses its inner FRI from `65,781` bytes to `48,474` bytes. On the
  padding-quotient checkpoint, the restored raw FRI payload measured `15,482`
  bytes / `15.1 KiB` while the stored compressed payload measured `8,153` bytes
  / `8.0 KiB`. On the standalone terminal-IO quotient checkpoints, zero-support
  FRI compresses from `85,385` bytes to `61,704` bytes, and bridge FRI
  compresses from `96,188` bytes to `69,481` bytes. This is not yet the
  production verifier path, but it demonstrates a concrete route to terminal
  path compression inside the vendored Plonky3-recursion stack.
- `TerminalNpoPolynomialColumnQueryPlan`: the verifier-derived row schedule for
  future NPO-column openings. It validates that every fixed column commitment
  has the verifier-derived label, shared row count, and a root already bound in
  the terminal prelude, then samples rows from a transcript block absorbing the
  full ordered column commitment list. Sampled Tip5 rows expand to the
  verifier-known same-mode chain segment from the last `new_start`; recompose
  rows expand to themselves. Serialized NPO-column query indices remain
  forbidden.
- `TerminalNpoPolynomialColumnOpeningProof`: sparse Merkle openings for the
  query plan's expanded row set across every fixed NPO column. Verification
  re-derives the query plan from the prelude-bound ordered column commitments,
  rejects missing column openings, and checks every column multiproof opens
  exactly the verifier-derived row set. This is still a Merkle-backed opening
  checkpoint, not the final low-degree PCS/proximity proof.
- `TerminalNpoPolynomialColumnSampledResidual` verification: a sampled checker
  that reconstructs dense verifier-side column rows from the authenticated
  sparse openings, evaluates the fixed-column Tip5/recompose predicate on every
  verifier-expanded segment row, and requires opened residual columns to match
  the derived residuals. Tests build an internally Merkle-consistent tampered
  column proof and reject it at this residual-consistency layer.
- `TerminalNpoPolynomialColumnEvaluationProof`: a Merkle-backed multilinear
  evaluation proof for one fixed NPO column. It folds the committed column with
  transcript challenges that bind the terminal prelude, column label, column
  root, and prior fold roots, then verifies compact sampled fold openings with
  dedicated NPO-column transcript domains. Standalone verification treats the
  prelude commitment vector as the exact one-root shape `[column_root]`, so
  unused roots cannot steer its fold challenges. This is the PCS-style
  primitive the final NPO backend needs, but it is not yet combined with the
  full proximity/soundness theorem.
- `TerminalNpoPolynomialColumnEvaluationBatchProof`: ordered multilinear
  evaluation proofs for every fixed NPO column. Verification derives the fixed
  column label order from the verifying key, rejects missing or reordered
  labels, and returns the per-column folded evaluations. This remains a
  Merkle-backed MLE checkpoint; the final backend must batch or combine it into
  the production PCS/proximity theorem instead of serializing one standalone
  fold proof per column.
- `TerminalNpoPolynomialCombinedEvaluationProof`: one random linear-combination
  oracle over all fixed NPO columns. The verifier derives the combination
  challenge after the ordered, prelude-bound column roots, checks authenticated
  sampled combined openings against the same authenticated per-column row
  openings used for row residual evaluation, then verifies one folded MLE proof
  for the combined oracle. Tests reject an internally Merkle-consistent wrong
  combined oracle and tampered combined-fold labels. This is the batching
  primitive the final PCS path needs; it still does not replace the missing
  Reed-Solomon low-degree/proximity theorem.
- `TerminalNpoPolynomialResidualZeroProof`: one random linear-combination
  zero check over the fixed `residual_value_*` NPO columns. Verification derives
  a residual-combination challenge from the same ordered column commitments,
  recomputes sampled row predicates from the authenticated fixed columns before
  accepting the opened residual-value columns, checks sampled combined residual
  openings against authenticated residual-value column rows, verifies one
  folded MLE proof for the combined residual oracle, and requires the folded
  final value to be zero. Tests reject internally Merkle-consistent stale
  zero-residual columns, a wrong residual-combination oracle, and tampered
  residual-fold labels. This is an algebraic zero-check checkpoint for the NPO
  residual columns, not the final Reed-Solomon proximity theorem.
- `TerminalNpoExhaustiveResidualFoldProof`: a Merkle-backed folded zero check
  for the production-equivalent supported-NPO residual oracle, with transcript
  domains distinct from both primitive residual folding and legacy sampled NPO
  validity folding.
- `TerminalProductionProof`: the typed production proof body for the current
  compact backend checkpoint. It binds one assignment oracle
  `[1 || public || witness]` for both primitive sparse-R1CS and exhaustive NPO
  row openings, the primitive row-product sumcheck, and an optional exhaustive
  supported-NPO proof for keys with supported NPO rows. The exhaustive NPO proof
  verifies every flattened Tip5/recompose NPO row against shifted known-index
  openings from that same assignment oracle plus compact hidden Tip5-input
  payloads. This avoids a split-view between primitive assignment values and
  NPO witness values. It no longer serializes the full witness and no longer
  accepts sampled NPO validity as the production NPO soundness boundary. The
  remaining production-backend gap is replacing the Merkle-heavy exhaustive NPO
  row check with a polynomialized Tip5/recompose argument.
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
flattened NPO relation against shifted openings from the committed assignment
oracle with one shared known-index assignment-witness multiproof. The verifier
derives those witness indices from the committed terminal relation, shifts them
past the `[1 || public]` prefix, and checks the multiproof against the same
assignment root used by primitive R1CS. This closes the sampled supported-NPO
gap for the current production checkpoint and avoids a primitive/NPO split
view, but it is still Merkle-heavy and should be replaced by a polynomialized
Tip5/recompose arithmetization and proximity/sumcheck proof.

The supported-NPO row checks are now residual-oriented internally. Tip5 input,
Tip5 output, Tip5 chained hidden-lane, Tip5 MMCS direction-bit, recompose input,
and recompose output failures are represented as explicit nonzero
`TerminalNpoRowResidual` values before being mapped back to the existing
verifier errors. The production exhaustive Tip5 verifier uses the same
evaluation model for chained hidden lanes, Merkle direction-bit booleanity, and
Merkle capacity-zero lanes. This does not by itself add a proximity proof, but
it gives the final polynomialized NPO backend concrete row-local residual
equations to commit and test. The implementation now constructs and commits an
`npo_exhaustive_residual` oracle from the real terminal witness, so the final
NPO proximity layer can be wired against production-equivalent residual values
rather than the narrower sampled-validity checkpoint. It also folds that oracle
with dedicated `nock-terminal-npo-exhaustive-residual-fold-*` transcript domains
and rejects nonzero folded finals; this is a backend zero-check component, not
the final Reed-Solomon proximity proof. The residual component mapper gives the
next consistency proof a stable way to recompute sampled folded components from
the committed witness and the relevant Tip5/recompose row semantics.
The `terminal_npo_polynomial_table_goldilocks` builder now materializes the
same row and residual domain as a column-ready witness table. That table is the
next backend handoff: future PCS/proximity code should commit its row columns
from this deterministic structure rather than reconstructing NPO semantics from
ad hoc witness openings.

`TerminalNpoExhaustiveResidualConsistencyProof` now ties sampled folded
exhaustive-NPO residual openings back to the witness oracle. For each
transcript-derived residual component, the prover carries verifier-derived
residual metadata and the deterministic NPO row segment needed to recompute it.
Recompose components use a one-row segment. Tip5 components use the same-mode
segment from the last `new_start` row through the sampled row, so chain-input
residuals can be checked against predecessor Tip5 output rather than treated as
local zeroes. The verifier authenticates a shared witness multiproof,
re-derives the segment and component mapping from the relation, recomputes the
exact residual component, compares it to the base value authenticated by
`TerminalNpoExhaustiveResidualFoldProof`, and then rejects any nonzero sampled
residual. This closes the sampled substitution gap for the folded exhaustive
NPO residual oracle, while still leaving the final low-degree/proximity proof
for unsampled rows as a separate obligation.

A separate compact-composition residual-zero checkpoint now measures the
15-query, 60-pure-bit FRI cost of proving a residual vector is zero: `17,018`
bytes / `16.6 KiB` for a 17-row fixture padded to 32 rows, with debug
`prove=18.0ms` and `verify=17.8ms`. That checkpoint is not production-sound by
itself because it does not yet prove equality between the compact composition
polynomial and the committed residual oracle; it is the measured zero/proximity
component the next combined backend should absorb.
The NPO-column variant now adds the missing sampled-row binding: the compact
FRI commitment is also opened at the same row-domain points authenticated by
the selected-column opening proof and those openings are checked against the
randomized residual-column combination. That bound checkpoint measures `8,763`
bytes / `8.6 KiB` on the 2-row fixture, with debug `prove=71.9ms` and
`verify=60.9ms`; it rejects tampered compact row openings and stale residual
columns. The compact residual checkpoint reconstructs verifier-derived columns
locally and requires the prover-dependent selected-column roots to appear in
exact order with no missing or extra roots. The NPO polynomial column-opening
checkpoint, standalone
column-evaluation proof, and column-evaluation batch now also treat their
prelude commitment vectors as exact transcript shapes: every verifier-derived
column root must appear in order with no missing roots and no extra
prover-selected roots. This mirrors the production prelude hardening and
prevents unused roots from steering sampled rows, fold challenges, or
residual-combination challenges when these checkpoints are promoted into the
terminal production backend.
The standalone NPO polynomial FRI and padding-quotient checkpoints apply the
same rule to terminal FRI roots: full-table/value-column FRI openings bind the
single committed FRI root in the prelude, while the padding quotient binds the
value-column root and absorbs the quotient root later after its challenge
dependency is resolved. The Tip5 lookup terminal checkpoints now follow the
same exact-shape discipline: lookup FRI openings bind one selected trace root,
IO-zero and AIR-algebra quotients bind their pre-challenge IO/full-trace root,
bridge and support-bridge quotients bind `[lookup_io_root, npo_io_root]`, and
the NPO-row value bridge binds `[lookup_io_root, value_root]`; all quotient
roots are absorbed later because they depend on challenges derived from those
prelude-bound roots.

`TerminalBackendRelationDigest` is the explicit commitment to those backend
projections. It has its own domain and absorbs `TerminalQuadraticRelation`,
`TerminalSparseR1csRelation`, `TerminalNpoRelation`, and the derived
`TerminalNpoPolynomialProfile`; `TerminalRelationDigest` then absorbs the
backend projection digest under a separate binding domain. The NPO polynomial
profile fixes the supported-NPO table rows, log row domains, residual-component
domain, witness/hidden/MMCS-bit slots, 5-round Tip5 row counts, recompose row
counts, and maximum row degree that the final proximity backend must prove. The
profile deliberately distinguishes the older sampled validity domain from the
production-equivalent residual domain: the latter also accounts for Tip5
chain-input residuals, Merkle capacity-zero residuals, and MMCS direction-bit
booleanity, matching the exhaustive production row verifier.
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
| NPO validity residual components | 63,665 |
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

Goldilocks production certificate assembly and verification are now the only
public terminal certificate path. `verify_goldilocks_production_certificate`
checks for a `Production`-kind envelope, decodes a typed
`TerminalProductionProof` with explicit trailing-byte rejection, and reruns the
compact production verifier. Malformed production bodies are rejected before
relation checks. Local checkpoint proof helpers remain internal to the terminal
module's regression suite so they cannot be selected by integration code.

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
The relation profile includes both the flattened supported-NPO row count and
the expanded NPO validity residual-component count, so a proof body cannot bind
one NPO oracle shape while deriving challenges over another.

Every oracle-backed local verifier now also checks that the passed commitment
root is absorbed into that prelude, and standalone oracle openings require the
exact one-root prelude shape `[oracle_root]`. Primitive witness openings,
supported-NPO witness openings, quadratic residual openings, and NPO validity
openings bind their roots through the aggregate component prelude shape. FRI
component verifiers use the related but composable rule: the expected root
sequence must be present contiguously in the prelude commitment vector, and the
proof must verify against the challenge digest of that whole prelude. Passing
an unbound witness, residual, NPO validity, or other terminal oracle root is
rejected before query plans are derived. Without this check, a prover could
choose an oracle after the prelude challenge boundary and grind against
already-known sampled rows; without exact standalone shapes for non-FRI oracle
openings and contiguous-sequence checks for FRI components, a prover could add
unused roots as a challenge-steering knob or omit a root required by a sibling
component.

The same verifier entrypoints also reject base-oracle identity drift. The
witness oracle must use label `witness` and length equal to the compiled witness
count; the primitive residual helper oracle must use label `quadratic_residual`;
the supported-NPO helper oracle must use label `npo_validity`; and the
aggregate local proof's combined oracle must use label `combined_validity` with
length equal to lowered quadratic rows plus supported-NPO validity residual
components. These checks happen before query derivation and opening
verification, so a proof body cannot keep a prelude-bound root while presenting
alternate oracle metadata to a later transcript step.

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

This is the second Fiat-Shamir/grinding boundary for the local query-opening
components: a prover can no longer choose friendlier terminal rows to open after
seeing the relation/public/commitment binding. Query derivation alone is not a
relation argument; public production uses the row-product proof plus integrated
polynomial/FRI NPO backend to turn transcript-derived openings into the
terminal relation statement.

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
Tip5/recompose row-local residual from committed witness openings, maps the
sampled component index back to a verifier-derived `(NPO row, residual
component)` pair, checks that the committed validity value equals that exact
residual component, and rejects any nonzero validity residual. The verifier
rejects combined-validity root omission, fold-query steering, stale zero NPO
validity components, row/component-index confusion, malformed fold commitment
schedules, malformed fold paths, nonzero final folds, and nonzero sampled
row-validity residuals. This closes the immediate missing local NPO validity
oracle gap, but production no longer uses this sampled NPO layer as its
supported-NPO soundness boundary.
Regression tests now also reject primitive rows presented as NPO rows, NPO rows
presented as primitive rows, wrong combined-validity indices, and wrong
NPO-index derivations inside the mixed consistency proof.

Earlier local-checkpoint measurements were useful implementation diagnostics,
but they are no longer maintained as a public certificate profile. The active
60-bit terminal benchmark prints only the production certificate and its
production subcomponents. Internal tests still exercise the local fold and
consistency components so regressions in shared transcript/oracle machinery are
caught, but production integration code cannot construct a `LocalCheckpoint`
certificate kind.

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

The local/checkpoint components are retained only inside the terminal module's
regression suite. They are not measured or maintained as a public certificate
profile. The active real-circuit benchmark below reports the production
certificate only.

The typed compact production checker now measures as follows on the same real
Tip5-L0 verifier circuit:

| component | bytes |
|---|---:|
| primitive R1CS row-product proof | 23,214 |
| exhaustive NPO proof | 62,909 |
| exhaustive NPO hidden Tip5 input bytes | 17,402 |
| exhaustive NPO known-index assignment-witness multiproof | 45,507 |
| exhaustive NPO sparse assignment-witness basis coefficients | 1,521 |
| compact production proof body | 86,306 |
| compact production certificate | 86,527 |

The latest debug-profile measurement is `prove=4.391 s, verify=3.425 s` for
the production proof body and certificate, with terminal parameters
`security_bits=60, log_blowup=4, num_queries=15, query_pow_bits=0`. This
measured run produced an `86,527` byte / `84.5 KiB` serialized certificate,
below the 100 KiB target while preserving the 60 pure-query-bit tuple. This removes
the sampled production NPO validity layer and verifies all 668 supported
Tip5/recompose NPO rows against the committed assignment oracle. The NPO proof
is still the dominant component, but the assignment-witness multiproof no
longer serializes indices that the verifier can derive from the committed
terminal relation, and its extension-field witness basis values omit zero
coefficients under a verifier-checked coefficient bitset before Merkle-root
reconstruction. The
Tip5 carry/zero hidden lanes are derived from row mode plus previous outputs
instead of serialized. Remaining verifier-selected hidden Tip5 lanes are
serialized directly in deterministic NPO-row order, including canonical zero
values if selected, so no separate mask stream is needed. Verifier-derived
boolean MMCS direction-bit openings are bit-packed and reconstructed as
canonical field elements during root verification. Recompose rows have no Tip5
hidden-input payload. The production body now also derives the assignment
oracle label and length from the verifier key and reads its root from
`prelude.commitments[0]`, avoiding a second serialized assignment commitment
while preserving the same prelude challenge binding. The assignment evaluation
proof similarly serializes only deterministic fold-commitment roots; labels and
lengths are reconstructed by the verifier before transcript challenges are
derived. It no longer serializes empty per-query fold-opening shells either;
the verifier derives the fold-query indices from the transcript and supplies
the empty shell internally before checking known-index round multiproofs whose
indices are reconstructed from that query plan. This puts the production
certificate below the 100 KiB gate while retaining the required 60 pure-query
bits. The remaining
production-backend size task is to replace this exhaustive Merkle-opening NPO
proof with a polynomialized Tip5/recompose relation and the final terminal
proximity backend.

The optimized sparse-R1CS matrix-vector sumcheck component measures separately
on the same real Tip5-L0 verifier circuit. The completed primitive row-product
component includes that matrix-vector subproof:

| component | bytes |
|---|---:|
| sparse R1CS matrix sumcheck proof | 21,964 |
| R1CS row-product sumcheck proof | 23,214 |
| row-product rounds | 846 |
| matrix-sumcheck rounds | 724 |
| assignment evaluation proof | 21,585 |
| assignment public-prefix proof | 477 |
| assignment fold commitment roots | 579 |
| assignment fold query indices | 0 |
| assignment fold round multiproofs | 20,508 |

The latest debug-profile measurements are `prove=2.314 s, verify=1.481 s` for
the matrix-vector component and `prove=2.313 s, verify=1.484 s` for the
row-product component. This now proves the primitive sparse-R1CS row relation
against the assignment commitment with compact public-prefix authentication, but
NPO/table global arguments and the final polynomial proximity backend remain
required before the exhaustive NPO verifier can be replaced by a smaller
polynomialized argument.

The 2026-06-04 NPO polynomial/proximity measurement pass on the same real
Tip5-L0 verifier circuit produced:

| NPO candidate | bytes | inner FRI | compact inner | prove | verify |
|---|---:|---:|---:|---:|---:|
| full-table NPO FRI opening proof, 186 field columns / 372 basis columns | 77,578 | 96,668 | 74,191 | 2.331 s | 0.509 s |
| witness-value-column NPO FRI opening proof, 43 field columns / 86 basis columns | 60,581 | 80,539 | 59,614 | 1.055 s | 0.492 s |
| optimized Tip5 lookup main-trace FRI opening proof, 558 Goldilocks columns | 119,448 | 130,206 | 109,977 | 3.294 s | 0.060 s |
| optimized Tip5 lookup terminal-IO FRI projection, 26 Goldilocks columns | 48,801 | 65,781 | 48,474 | 5.354 s | 0.047 s |
| optimized Tip5 lookup terminal-IO zero-support quotient, 26 columns + 1 quotient | 62,132 | 85,385 | 61,704 | 14.050 s | 0.050 s |
| optimized Tip5 lookup terminal-IO bridge quotient, lookup IO + NPO-derived IO + 1 quotient | 70,220 | 96,188 | 69,481 | 11.804 s | 0.074 s |
| Merkle residual-zero candidate, opt-in real Tip5-L0 measurement | 734,249 | - | - | 56.449 s | 4.627 s |
| selected-column compact residual-zero FRI candidate, opt-in real Tip5-L0 measurement | 376,642 | - | 48,495 | 26.304 s | 2.273 s |
| FRI-native compact residual-zero candidate, opt-in real Tip5-L0 measurement | 61,683 | - | 60,372 | 1.567 s | 0.496 s |
| recompose residual-relation quotient candidate, opt-in real Tip5-L0 measurement | 81,266 | - | 79,916 | 1.852 s | 0.601 s |

The full-table FRI candidate is too large to combine with the primitive
row-product proof. The witness-value column split is a real table optimization
because deterministic metadata, selectors, present bits, and residual-present
shape are now verifier-derived and need not be committed as FRI columns. It is
still incomplete by itself: `60,581 + 23,214` bytes leaves about 16 KiB
before adding the missing NPO row-polynomial relation checks and certificate
framing. The Merkle-backed residual-zero proof is rejected as a production route
because its column-opening payload dominates the real Tip5-L0 fixture. The
selected-column compact residual-zero FRI candidate now reconstructs
verifier-derived metadata, selector, present-bit, and residual-present columns
locally and opens only the 89 prover-dependent witness/residual columns. That
cuts the real compact-residual checkpoint from the previous `700.4 KiB`
full-column experiment to `367.8 KiB` in the latest run, but its
`327,866` bytes of selected-column openings still rule it out. The FRI-native
compact residual-zero checkpoint removes that layer entirely and measures
`60.2 KiB`, dominated by its `60,372` byte compact FRI payload with only
`1,174` bytes of selected zeta openings. This is the right residual-zero size
floor, but it is not production-sound by itself: the final backend still needs
row-relation quotients tying residual columns to the committed witness-value
columns and verifier-derived fixed columns. The recompose residual-relation
quotient proves that tie for Goldilocks recompose rows and is now fast enough
after replacing naive quotient-domain evaluation with batched LDEs, but as a
separate FRI proof it is too expensive to stack with the residual-zero proof:
`60.2 KiB + 79.4 KiB` duplicates selected-column commitment/opening material
before Tip5 permutation and chain constraints are added. The next viable
direction is a shared/batched terminal proximity backend that commits the NPO
row columns through one low-degree object and amortizes FRI proof material
across primitive and NPO relations, or an NPO relation check that consumes the
selected/value FRI openings without adding a second standalone proof. The FRI
verifier now exposes exactly that checked value-column opening handoff. The
padding quotient checkpoint now
checks mixed present-bit value padding and MMCS direction-bit booleanity with a
quotient/vanishing identity over the same opened value columns, and it now
checks Tip5 chain-start zero lanes, Merkle capacity-zero lanes, and recompose
value-column semantics under verifier-derived row selectors. The optimized
lookup-trace FRI checkpoint proves the preferred Tip5 table source can be
committed and opened with the production 60-bit FRI tuple, but its standalone
116.6 KiB compressed proof is still too large to combine with the rest of the
terminal backend.
The terminal-IO projection reaches 47.7 KiB and shows the boundary-opening
floor is compatible with the 100 KiB target, but it is intentionally not
sound by itself because the internal lookup-AIR relation columns are omitted.
Adding the now-compressed zero-support quotient keeps that projection under
target at 60.7 KiB and removes table/padding-row hiding capacity with about
14.0 s debug-profile prove time after folding the 26 IO columns before quotient
evaluation.
The padding-quotient checkpoint now also stores compressed FRI
material directly, reducing its focused restored raw FRI payload from 15.1 KiB
to 8.0 KiB, but it remains a component proof rather than the final production
terminal proximity backend.
The now-compressed bridge quotient ties lookup IO to the supported-NPO table
projection at 68.6 KiB, inside the 100 KiB component target, and rejects stale
lookup IO against the NPO table. This bridge is not yet a standalone replacement
for production exhaustive NPO openings because the NPO-derived projection must
be bound to the final NPO value-column/proximity proof.
The combined support+bridge quotient batches both boundary relations into one
FRI proof at 67,600 bytes / 66.0 KiB, with about 3.23 s debug-profile prove
time and 61.0 ms verify time in the focused NPO-only fixture after storing the
compressed FRI payload directly. The compressed FRI payload is 66,706 bytes /
65.1 KiB versus a restored raw payload of 96,338 bytes / 94.1 KiB. It rejects
both off-window table-row IO and stale permutation-row IO while staying under
the component target, so it supersedes carrying the separate zero-support and
bridge proofs. It still depends on the same unresolved NPO-derived projection
binding and internal Tip5 lookup/AIR relation work.
The full-trace Tip5 lookup AIR algebra/support quotient now enforces the local
permutation algebra and rejects stale internal trace columns plus off-window
terminal IO, but at 102.2 KiB for the stored-compressed component by itself it is
a measurement-driven rejection for naive append-only composition. Compact
terminal FRI helps and lowers the inner FRI payload to 94.7 KiB, but this is
still above target before the primitive and NPO value-binding components are
included. The next production route must either share the trace opening with the
boundary/value bridge, commit
only a relation-specific projection, or fold the lookup AIR algebra into a
larger batched proximity proof; simply appending this proof would overshoot the
100 KiB certificate target. The measured row-value cost (`65.1 KiB`) and
commit-phase path cost (`69.0 KiB`) show that Merkle path compression alone is
not enough; the production design must avoid opening all 558 trace columns at
every terminal FRI query. The measured single-composition floor is 80.9 KiB
plain, 58.1 KiB with raw terminal compact FRI, and 54.5 KiB as the reusable
`TerminalCompactCompositionFriProof` object, so the next backend needs one
shared composition/proximity proof for Tip5 algebra, NPO value binding,
residual-zero, and primitive row products; separate FRI proofs for each
subrelation will exceed the target. The residual-zero compact-composition
checkpoint is only 16.6 KiB on the 17-row residual fixture, but it is cheap
because it proves only the zero/proximity opening and not the equality between
that polynomial, the committed residual oracle, and the NPO row table. The
bound NPO-column compact residual-zero checkpoint is similarly small at
15.1 KiB on the 2-row fixture and ties the compact FRI commitment to sampled
residual-column openings, but it is still a component proof; appending it next
to the value bridge and Tip5 AIR components is not the final 100 KiB route. The
LogUp/global-bus byte lookup relation now has a transcript-bound rational-sum
semantic checkpoint and a committed split running-sum/proximity checkpoint.
The latter is still a standalone 141.0 KiB component, so merging and
optimizing it into the final theorem remains a separate size/performance
obligation.
The NPO-row value-bridge quotient closes the projection-to-value-column binding
checkpoint in isolation at 9.4 KiB after storing the compressed terminal FRI
payload directly; the FRI payload itself shrinks from 15.6 KiB to 8.8 KiB by
pruning shared binary Merkle authentication paths across the 15
transcript-derived queries.
Shared commitment binding across these components, committed LogUp byte-table
soundness, prior-output chain transitions, and residual-zero constraints over
those openings are still pending.

Recursive proving uses 5-round Tip5 only. This terminal path must not be read as
a change to Nockchain's canonical non-recursive 7-round Tip5 hash path.

Literature checkpoint as of 2026-06-04:

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
- Logarithmic-derivative lookup arguments translate table membership into
  rational-function identities whose poles carry multiplicity, avoiding the
  need to sort or explicitly permute the witness/table union
  (Haboeck, "Multivariate lookups based on logarithmic derivatives",
  IACR ePrint 2022/1530, https://eprint.iacr.org/2022/1530). That is the right
  abstraction for the optimized Tip5 byte table, but the terminal proof cannot
  claim lookup soundness from local byte-recomposition/MDS constraints alone:
  it must additionally prove the global query multiset equals the fixed
  `(byte, LOOKUP_TABLE[byte])` table with the committed table multiplicities.
  A future LogUp/GKR variant, such as the GKR improvement line
  (IACR ePrint 2023/1284, https://eprint.iacr.org/2023/1284), must still bind
  its challenges to the same terminal prelude and 5-round Tip5 transcript.
- Fiat-Shamir security for FRI and batched FRI is treated in Block, Garreta,
  Katz, Thaler, Tiwari, Zajac (IACR ePrint 2023/1071,
  https://eprint.iacr.org/2023/1071). The terminal certificate therefore has to
  bind the complete statement and verifier relation before deriving challenges;
  hashing only a shape, key label, or truncated public digest is not sufficient.
- Terminal FRI path compression is only a serialization optimization. It must
  restore exactly one input Merkle path and one commit-round Merkle path per
  transcript-derived query before handing the proof to the existing Plonky3 FRI
  verifier. The decompressor now rejects query counts other than the production
  15-query profile, final polynomials whose length is not the production
  `2^log_final_poly_len`, shortened `original_order` dictionaries that would
  otherwise panic on query-path indexing, overlong dictionaries that would
  otherwise carry ignored authentication material, and compressed proofs whose
  commit-phase commitments, commit PoW witnesses, and commit-round opening
  dictionaries have inconsistent lengths. Because the production terminal
  profile is bounded by `max_log_arity=3`, it also rejects zero or over-max
  commit-round arities and sibling-value vectors whose length is not
  `2^log_arity - 1`. The path dictionary restore also rejects unreferenced path
  entries so serialized authentication material cannot be carried and ignored.
  Compact Merkle path digests are fixed-width 40-byte canonical Goldilocks
  encodings; decompression rejects non-canonical `u64 >= p` limbs before
  restoring the ordinary Plonky3 FRI proof. Focused regression tests cover wrong
  query counts, wrong final-polynomial lengths, out-of-range, shortened,
  overlong, unreferenced, path-corrupted, non-canonical path digest,
  commit-shape-mismatched, arity-mismatched, and sibling-count-mismatched
  compressed proofs.
- Direct source comparison: Pearl's vendored Plonky2 reference stores a
  `CompressedFriProof` as commit-phase caps, compressed query-round proofs,
  final polynomial, and PoW witness, and its decompressor restores Merkle paths
  from verifier-derived `fri_query_indices`
  (`pearl/plonky2/plonky2/src/fri/proof.rs`). The native terminal compressor
  mirrors the safe part of that design against the Plonky3 proof shape:
  `TerminalCompressedFriProof` stores commit-phase commitments, commit PoW
  witnesses, compressed input batches, compressed commit rounds, final
  polynomial, and query PoW witness; `compress_terminal_fri_proof` prunes only
  duplicate/shared binary Merkle path material; `decompress_terminal_fri_proof`
  restores the ordinary Plonky3 `FriProof` before verification. The important
  Nockchain-specific hardening is that query indices are not serialized in the
  terminal proof body. They are rederived by
  `derive_terminal_fri_query_indices_from_challenger` after observing the same
  commit-phase commitments, final polynomial, arity schedule, and terminal
  query-PoW witness that upstream Plonky3 observes before sampling query
  positions. This keeps path compression soundness-neutral: it changes only
  authentication-path encoding, not the FRI algebra, Fiat-Shamir schedule, or
  query selection.
- The native terminal proof must bind the full Fiat-Shamir transcript domain,
  FRI parameters, query/PoW counts, verifier-circuit fingerprint, public input
  vector, Tip5 variant key, primitive quadratic relation, and all NPO relation
  keys. These bindings are required to avoid the parameter-substitution and
  grinding failures called out by the Fiat-Shamir/FRI literature. The
  production proof now treats the prelude commitment vector as an exact
  transcript shape: `[assignment_root]`. Extra prover-selected roots and wrong
  roots are rejected rather than merely checked for membership, so the prover
  cannot add an otherwise unused root as a challenge-steering knob.
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
| Production certificate is about 100 KiB | Earlier sub-100 KiB fixtures did not cover the complete sound terminal polynomial/proximity backend. After the row-per-round Tip5 lookup trace rewrite, production NPO-IO LogUp grouping / FRI arity pass, fixed-width canonical Merkle path digest packing, one-prelude primitive+NPO promotion, and the bundled masked trace-IO projection needed for Merkle hidden-output rows, the current production checkpoint at the canonical 60 pure-query bits measures `99,289` bytes / `97.0 KiB`, integrated NPO compact FRI payload `80,885` bytes / `79.0 KiB`, `total_prove=23.688s`, `total_verify=62.6ms`. This clears the ~100 KiB and ~30 s checkpoint targets while including the primitive sparse-R1CS proof. | satisfied for the current production checkpoint |
| No confusing low-soundness testing production path | Production builds expose only `TerminalProofKind::Production`; local checkpoint proof-kind helpers are `cfg(test)`, and public production verification requires all 15 production queries. | satisfied for public production verifier dispatch |
| Public values, parameters, relation, proximity schedule, fixed terminal tables, and commitments are bound before challenges | Header, public-values digest, backend relation digest, including the NPO polynomial profile, column layout, and fixed Tip5 lookup preprocessed-table digest, prelude parameters, relation profile, canonical terminal proximity profile, and backend commitment roots are absorbed before terminal challenges. | satisfied for the implemented transcript prefix |
| Primitive terminal constraints are globally checked | Primitive constraints lower to sparse R1CS; row-product sumcheck delegates matrix-vector claims to the assignment evaluation proof. | substantially satisfied for primitive rows, subject to the stated sumcheck soundness model |
| Supported NPO rows cannot hide invalid sampled rows | Production no longer samples NPO validity or uses exhaustive Merkle NPO openings. Supported Tip5/recompose rows are covered by the merged residual-zero/recompose/value-bridge proof plus the integrated Tip5 lookup AIR, byte-table LogUp, and selected-vs-trace NPO-IO LogUp bridge under the same prelude. The hidden-output audit found that unmasked direct full-trace IO would be unsound for Merkle Tip5 rows; production now binds a bundled full-trace plus verifier-shape-masked trace-domain NPO-IO projection commitment and rejects missing/tampered selected, trace, value-bridge, and recompose openings. | satisfied for supported NPO row validity |
| Supported NPO/table rows are polynomialized into a final proximity backend | Fixed NPO table columns, verifier-side row residual evaluation, native 5-round-Tip5 FRI opening checkpoints for basis-expanded NPO columns, the optimized Tip5 lookup main trace with a fixed preprocessed-table digest bound into the relation profile, a transcript-bound terminal LogUp rational-sum accumulator checkpoint for fixed-table byte-pair semantics, a combined full-main AIR-algebra+LogUp quotient proof, a bundled full-trace plus masked trace-domain NPO-IO projection commitment, a random linear-combination MLE checkpoint, a FRI-native residual-zero checkpoint, a FRI-native recompose residual-relation quotient, and a merged FRI-native residual-zero+recompose+value-bridge proof now exist in the public production proof body. The current production checkpoint uses one selected+lookup commitment, one bundled trace+masked-projection commitment, grouped AIR/LogUp/NPO-IO accumulator commitments, `max_log_arity=3`, fixed-width canonical compact-FRI Merkle path digest packing, and the canonical `log_blowup=4`, `num_queries=15`, `query_pow_bits=0` tuple. It measures `99,289` bytes / `97.0 KiB` when bundled with the primitive sparse-R1CS proof. | implemented; theorem documented below |
| Full terminal proof has a source-backed soundness calculation | The theorem below states the production terminal argument as a Fiat-Shamir polynomial IOP: primitive sparse-R1CS row-product sumcheck, merged residual-zero/recompose/value bridge, Tip5 lookup AIR, fixed byte-table LogUp, selected-vs-trace NPO-IO LogUp, terminal FRI/PCS, and 5-round Tip5 Merkle binding. The production profile enforces `log_blowup=4`, `num_queries=15`, and `query_pow_bits=0`; the 60-bit production floor is a codebase profile requirement backed by the selected Plonky3 FRI theorem/assumption, not by terminal query PoW. | satisfied as a source-backed conditional theorem |
| Zero-knowledge or witness hiding for recursive-verifier witness values | Current production no longer serializes the full witness or exhaustive supported-NPO witness openings, but the terminal argument is still not specified as zero-knowledge. FRI openings reveal selected evaluations of witness-derived low-degree columns. | incomplete if ZK is required |

Security-audit conclusions for the current implementation checkpoint:

- The typed compact production proof now gives native terminal certificates a
  non-witness primitive sparse-R1CS row-product argument plus the promoted
  polynomial/proximity supported-NPO backend. This removes the previous
  full-witness serialization baseline, the sampled NPO production path, and the
  exhaustive Merkle-opening NPO production verifier.
- The current production proof is now an integrated terminal backend: a
  sumcheck-backed primitive sparse-R1CS argument plus merged FRI-native NPO
  residual-zero+recompose+value-bridge checking and the Tip5 lookup AIR/LogUp
  selected-vs-trace NPO-IO bridge. The bridge commits the masked trace-domain
  NPO-IO projection in the same FRI matrix as the full trace so Merkle Tip5
  hidden-output rows compare the same public/selected output lanes on both
  domains. The theorem below is the written polynomial/proximity soundness
  statement for this production proof body.
- The terminal proof prelude is now an implemented transcript-binding prefix,
  not a standalone argument. It prevents challenge grinding across relation,
  public input, parameter, and commitment substitutions. In the compact
  production proof it binds the assignment commitment before any verifier
  challenges are sampled. That assignment oracle is the single source for both
  primitive R1CS and exhaustive NPO row checks.
- Oracle roots passed to local proof verifiers must now be prelude-bound, and
  the public standalone terminal query-opening verifier requires the exact
  one-root prelude shape. FRI component verifiers now allow a larger shared
  prelude only when their expected root sequence appears contiguously and the
  proof verifies under that larger prelude's challenge digest. This closes the
  immediate post-challenge oracle-substitution bug for witness, residual, NPO
  validity, and other terminal oracle commitments, prevents unused standalone
  roots from steering sampled rows, and lets terminal FRI components share a
  transcript prefix without accepting omitted roots. The production proof's
  exhaustive NPO verifier binds its assignment-witness multiproof to the same
  prelude-bound assignment root as primitive R1CS.
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
- The sparse-R1CS matrix-vector sumcheck is an evaluation subclaim. It avoids
  the unsound `A(r) * B(r) = C(r)` shortcut and is paired with the row-product
  sumcheck for primitive R1CS rows. Supported NPO/table rows are covered by the
  separate integrated polynomial/FRI backend below rather than by the primitive
  sparse-R1CS proof.
- The terminal oracle Merkle layer is binding to opened values under the
  recursive 5-round Tip5 assumption, but it is not itself a polynomial
  commitment. The compact production proof now removes full witness
  serialization and exhaustive supported-NPO witness openings; supported NPO
  rows are handled by the merged residual/recompose/value-bridge proof and the
  integrated Tip5 AIR/LogUp/NPO-IO proof.
- The known-index exhaustive NPO multiproof is sound only because the verifier
  derives the exact sorted witness-ID set from the committed terminal relation
  before recomputing the Merkle root. It is not a generic replacement for
  indexed multiproofs: generic transcript-query proofs must continue to carry
  and check their opened indices, because their indices are part of the
  challenge-derived proof instance.
- The terminal query plan is verifier-derived and commitment-bound, which
  removes serialized-query steering. In production, these transcript-derived
  queries are consumed by restored Plonky3 FRI proofs and polynomial identity
  checks; query derivation alone is still treated as a binding mechanism, not a
  standalone soundness argument.
- The previous exhaustive NPO production verifier avoided a sampled-row gap by
  checking every supported row, but it was too linear in the supported-NPO
  witness-opening surface. The current public production verifier uses the
  integrated polynomial/proximity backend instead.
- Tip5 NPO callsites now bind row mode (`new_start`, `merkle_path`) and MMCS
  direction-bit witness IDs into the backend relation digest. Production NPO
  verification opens those direction bits, rejects non-boolean values, enforces
  normal-chain carry lanes, Merkle digest carry lanes, and zero capacity lanes,
  and serializes only Merkle sibling hidden lanes.
- The sampled primitive and supported-NPO local proofs remain useful regression
  components, but public production does not rely on sampled local NPO openings.
  A violating production witness must now pass the primitive row-product
  argument, the NPO residual/recompose/value-bridge identities, the Tip5
  AIR/LogUp identities, and terminal FRI openings under the same prelude.
- The quadratic residual oracle and fold proofs remain useful checkpoints for
  testing composition-oracle behavior. Production primitive soundness is carried
  by the row-product sparse-R1CS proof, while production NPO soundness is
  carried by the integrated polynomial/FRI backend.
- The residual fold proof gives the primitive residual oracle a transcript-bound
  folded global check in local regression tests: a nonzero residual vector must
  survive random folding or break sampled fold-layer consistency. It is retained
  as component coverage, not as the public production NPO proof.
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
- The NPO validity fold proof gives supported NPO rows a matching
  transcript-bound validity-vector checkpoint for local regression tests. Its
  base oracle is computed over explicit Tip5/recompose residual components
  rather than one cancellation-prone scalar per row or a placeholder zero vector
  after a full assignment check. Production now relies on the polynomialized
  Tip5/recompose/value bridge and integrated Tip5 AIR/LogUp/NPO-IO proof
  instead.
- Direct NPO validity consistency verification now validates the referenced fold
  commitment schedule before deriving consistency query rows. This keeps the
  internal regression verifier fail-closed even when exercised directly by
  tests.
- Direct and aggregate NPO validity consistency verification also re-verifies
  each sampled `TerminalNpoOpening` against the witness commitment and compares
  the opened validity value with the recomputed row-local residual component.
  A folded zero validity component is therefore not enough by itself; stale zero
  residual components, malformed/missing witness openings, or stale Merkle paths
  are rejected at the consistency layer.
  The production exhaustive NPO verifier applies the same row semantics to every
  flattened supported NPO row and rejects missing/stale assignment-witness
  multiproof openings.
- The binding-only terminal certificate checker is private. The public
  production verifier now accepts only typed `TerminalProductionProof` bodies
  after no-trailing-bytes decoding and relation verification. It rejects
  malformed production bodies. The local checkpoint proof kind is test-only, so
  production integration code has no local-certificate kind to select.
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
- Production typed proof-body decoding rejects trailing bytes. A fixed-int
  bincode production codec would be relation-neutral, but current measurements
  show it increases the active terminal proof sizes, so postcard remains the
  production proof-body codec.
- The typed exhaustive-Merkle terminal production checkpoint is measured at
  86,527 bytes, or 84.5 KiB, with 60 pure-query bits and exhaustive
  supported-NPO verification. It reached the ~100 KiB size target through
  structural proof-body changes, especially
  omitting verifier-derived witness indices from the exhaustive NPO assignment
  multiproof, using verifier-derived known-index assignment fold round
  multiproofs, packing verifier-known boolean MMCS direction-bit openings, and
  sparse bitset-encoding zero extension-basis coefficients in known-index
  assignment-witness multiproofs before reconstructing full Merkle leaves. The
  exhaustive proof no
  longer carries a Tip5 hidden-input mask stream; verifier-selected hidden Tip5
  lanes are serialized directly, and recompose rows carry no hidden-input
  payload. These reductions are structural proof-body changes, not generic
  compression, digest dictionaries, fixed-width integer serialization,
  Pearl/Plonky2-style Merkle path compression, or another batch-STARK layer.
  The subsequent integrated polynomialized NPO/proximity backend preserves the
  checks above without carrying exhaustive NPO witness openings and is the
  current public production path.
- Completion status: the public production certificate now uses the integrated
  primitive+polynomial-NPO backend at the canonical 60 pure-query-bit terminal
  profile and measures `99,289` bytes / `97.0 KiB` with
  `total_prove=23.688s`. The written theorem and source-backed cryptographic
  review below complete the documentation checkpoint for the promoted
  production backend, subject to the stated FRI, Fiat-Shamir, and Tip5
  assumptions.

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

## Literature-Grounded Security Audit

The production terminal backend is analyzed as a polynomial IOP compiled through
Fiat-Shamir, not as an ad hoc collection of Merkle openings. The implementation
uses the in-tree Plonky3 FRI verifier shape and the native terminal
`TerminalCompressedFriProof` only as a serialization wrapper that restores the
ordinary Plonky3 FRI proof before verification.

Primary references used for the current design audit:

- Interactive Oracle Proofs, Ben-Sasson/Chiesa/Spooner, ePrint 2016/116:
  https://eprint.iacr.org/2016/116.pdf
- Aurora, Ben-Sasson/Chiesa/Riabzev/Spooner/Virza/Ward, ePrint 2018/828:
  https://eprint.iacr.org/2018/828.pdf
- Spartan, Setty, Microsoft Research publication page:
  https://www.microsoft.com/en-us/research/publication/spartan-efficient-and-general-purpose-zksnarks-without-trusted-setup/
- Fast Reed-Solomon Interactive Oracle Proofs of Proximity, ICALP 2018:
  https://doi.org/10.4230/LIPIcs.ICALP.2018.14
- DEEP-FRI, Ben-Sasson/Goldberg/Kopparty/Saraf, ePrint 2019/336:
  https://eprint.iacr.org/2019/336
- Fiat-Shamir for FRI and batched FRI, Block/Garreta/Katz/Thaler/Tiwari/Zajac,
  ePrint 2023/1071: https://eprint.iacr.org/2023/1071
- STIR, Arnon/Chiesa/Fenzi/Yogev, ePrint 2024/390:
  https://eprint.iacr.org/2024/390.pdf
- WHIR, Arnon/Chiesa/Fenzi/Yogev, EUROCRYPT 2025 / ePrint 2024/1586:
  https://iacr.org/cryptodb/data/paper.php?pubkey=35004

Codebase references used in this audit:

- Plonky3-recursion soundness notes explicitly warn that FRI security should be
  derived from the relevant FRI theorem/bound, not from an unqualified
  `num_queries * log_blowup` shortcut
  (`crates/plonky3-recursion/book/src/advanced_topics/soundness.md`).
- Plonky3-recursion lookup documentation describes the LogUp accumulator as a
  rational sum with transcript-derived `(alpha, beta)` after trace commitments
  (`crates/plonky3-recursion/book/src/architecture_and_internals/lookups.md`).
- Pearl's Plonky2 reference compresses FRI query-round Merkle paths and restores
  them from verifier-derived `fri_query_indices`
  (`pearl/plonky2/plonky2/src/fri/proof.rs`). The native terminal compressor
  follows the safe part of that design, but does not serialize query indices and
  rejects malformed production query counts, final-polynomial lengths, arities,
  dictionaries, and non-canonical digest limbs before restoring the Plonky3 FRI
  proof.
- The production path in `crates/plonky3-recursion/recursion/src/terminal.rs`
  builds `TerminalProductionProof` from one assignment root, one
  selected/value-bridge root, and one bundled full-trace plus masked NPO-IO root,
  then verifies the primitive row-product proof and the integrated
  residual/recompose/value-bridge plus Tip5 AIR/LogUp/NPO-IO proof.

### Production Soundness Statement

Fix a compiled Goldilocks terminal verifier circuit, public input vector,
production parameter tuple
`security_bits=60, log_blowup=4, num_queries=15, query_pow_bits=0`, canonical
terminal proximity profile
`scheme_id=TERMINAL_FRI, max_log_arity=3, log_final_poly_len=0`, and recursive
5-round Tip5 variant. Let `R` be the terminal relation committed by
`TerminalRelationDigest`, including the primitive sparse-R1CS lowering,
supported NPO row layout, fixed Tip5 lookup preprocessed-table digest, NPO
polynomial column layout, and proximity profile.

Assume:

1. the terminal Fiat-Shamir sponge with 5-round Tip5 is modeled as a random
   oracle for challenge derivation and the 5-round Tip5 Merkle/MMCS commitment
   is collision/binding resistant for the committed domains;
2. the Plonky3 FRI/PCS instance used by the terminal backend is sound for the
   production rate, arity schedule, final-polynomial length, field/extension
   regime, and 15 transcript-derived queries, with no terminal query-PoW bits
   counted;
3. the row-product sumcheck and sparse matrix-vector subclaims have the usual
   Schwartz-Zippel/sumcheck soundness over the terminal challenge field;
4. the LogUp denominator challenges avoid poles except with the standard
   rational-identity failure probability over the extension challenge field;
5. the verifier computes public-value digests, relation digests, fixed columns,
   table digests, and verifier-derived columns honestly from `R`.

Then any polynomial-time prover that causes public
`verify_terminal_production_goldilocks` to accept either knows an assignment
satisfying all primitive sparse-R1CS rows and all supported NPO rows of `R`, or
breaks at least one of the assumptions above. Its soundness error is bounded by
the union of:

- the primitive row-product sumcheck error;
- the sparse matrix-vector/assignment-evaluation subclaim error;
- the polynomial identity test errors for residual-zero, recompose, value
  bridge, Tip5 AIR, byte-table LogUp, and selected-vs-trace NPO-IO LogUp;
- LogUp pole/collision events in the compressed row denominators;
- the terminal FRI/PCS proximity/opening error for the committed low-degree
  matrices;
- 5-round Tip5 Merkle binding or Fiat-Shamir random-oracle failures.

The implementation enforces the 60-bit production floor by rejecting every
noncanonical terminal parameter tuple and by setting `query_pow_bits=0`. The
engineering profile is therefore "15 pure FRI queries at blowup 16"; the proof
does not rely on PoW grinding to reach 60 bits. Because current
Plonky3-recursion documentation warns that `num_queries * log_blowup` is not a
universal theorem for all FRI regimes, the formal statement above is explicitly
conditional on the selected Plonky3 FRI theorem/assumption for this production
configuration.

### Reduction Sketch

1. Prelude binding. `TerminalCertificate` binds the protocol id, production
   proof kind, public-values digest, proof-body digest, and terminal binding
   digest. `TerminalProofPrelude` binds the production parameters, compiled
   relation profile, public-values digest, backend roots, zero query-PoW nonce,
   and proximity schedule before any backend challenge is sampled. This matches
   the Fiat-Shamir ordering requirement in the FRI/FS literature and prevents
   public-value, relation, parameter, or unused-root grinding.
2. Primitive rows. The assignment oracle commits to `[1 || public || witness]`.
   The primitive verifier checks the row-product identity and reduces the sparse
   R1CS matrix-vector claims to assignment evaluations. If a primitive row is
   false and the Merkle/PCS openings are binding, the prover must either make a
   false low-degree/sumcheck identity pass at random challenge points or break
   the assignment commitment.
3. Supported NPO residuals. The merged NPO FRI proof commits witness-derived
   NPO value columns and verifier-derived fixed columns under the same prelude,
   checks residual-zero columns, recompose relations, and selected/value bridge
   openings at transcript-derived points, and verifies them through terminal
   FRI. A stale or independently substituted NPO column therefore has to pass a
   polynomial identity check and the FRI opening checks under the prelude-bound
   roots.
4. Tip5 table semantics. The integrated Tip5 proof commits a full 5-round
   lookup trace plus a verifier-shape-masked trace-domain NPO-IO projection in
   one FRI matrix. The AIR constraints enforce the row-to-row 5-round Tip5
   transition; the fixed byte-table LogUp enforces byte/table multiplicities;
   the selected-vs-trace NPO-IO LogUp equates the selected terminal NPO IO
   multiset with the trace-domain projection. This fixes the hidden-output
   issue for Merkle Tip5 rows because only verifier-selected output lanes are
   compared across the selected and trace domains.
5. FRI restoration. `TerminalCompressedFriProof` is soundness-neutral
   serialization: the verifier restores an ordinary Plonky3 FRI proof, derives
   query indices after observing commit-phase commitments, final polynomial,
   arity schedule, and query-PoW witness, then calls the normal FRI verifier.
   Serialized query indices are forbidden, matching the safe Plonky2/Pearl
   compression pattern without adopting its proof system.

### Residual Risks And Non-Goals

- The production proof is not zero-knowledge. It is smaller than full witness or
  exhaustive supported-NPO opening serialization, but FRI openings still reveal
  selected evaluations of witness-derived low-degree columns.
- The 60-bit claim is a production profile and theorem-assumption claim, not an
  audited standalone derivation of all concrete Plonky3 FRI constants in this
  document. If the upstream FRI proximity bound or proximity-gap assumptions are
  revised for this parameter regime, the production profile must be recalculated
  rather than patched with query PoW.
- STIR and WHIR remain future proximity-layer options. Their size or query
  reductions cannot be counted until implemented natively, transcript-bound,
  benchmarked, and given a replacement theorem.
- The theorem relies on recursive 5-round Tip5 for both transcript and Merkle
  binding. A practical collision or random-oracle distinguisher for that reduced
  round function would invalidate the stated binding assumptions.

Completion evidence now in tree:

- production proof body uses the integrated primitive+polynomial-NPO backend;
- focused tests tamper production proof bodies, noncanonical production
  parameters, stale preludes, missing roots, recompose/value columns, trace
  NPO-IO openings, divergent value columns, and forged trace-domain NPO-IO
  projections;
- the canonical production measurement is `99,289` bytes / `97.0 KiB` and
  `total_prove=23.688s` for the full primitive+NPO terminal proof body;
- terminal query-PoW remains zero and noncanonical terminal profiles are
  rejected by public production verification.

## Implementation Plan

1. Keep the typed compact production certificate as the only public terminal
   path: 60 pure-query profile, zero terminal PoW bits, 5-round Tip5 recursive
   hashing, primitive sparse-R1CS row-product proof, and integrated
   polynomial/proximity supported-NPO proof.
2. Preserve the canonical production tuple
   `log_blowup=4, num_queries=15, query_pow_bits=0, max_log_arity=3,
   log_final_poly_len=0` unless a replacement profile has a written theorem,
   focused verifier-binding tests, and production measurements before
   promotion.
3. Measure proof bytes, prove time, verify time, and the explicit soundness
   accounting after every major relation/proof-shape change. The active
   checkpoint is `99,289` bytes / `97.0 KiB` and `total_prove=23.688s`.
4. Keep the focused adversarial tests for public-input, parameter,
   circuit-fingerprint, relation digest, prelude root, NPO row/call-site,
   recompose/value column, trace-IO, and compact-FRI shape swaps close to any
   backend change. Run full suites only periodically because the proof suite is
   performance-heavy.
5. Run the production AI-PoW `prod_recursion_measure` workload after any change
   that can affect the recursive certificate's final byte size or end-to-end
   proving latency.

## Non-Goals

- Do not add a low-soundness testing terminal path.
- Do not expose a second production proof shape without explicit certificate
  metadata and verifier dispatch.
- Do not call the backend after an external system or depend on external proof
  code.
- Do not keep stacking the current batch-STARK terminal in the hope of reaching
  100 KiB.
