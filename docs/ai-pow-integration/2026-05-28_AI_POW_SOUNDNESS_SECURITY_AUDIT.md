# AI-PoW / AI-PoW-ZK Soundness and Security Audit

Date: 2026-05-28
Branch audited: `claude/ai-pow-integration-squash`
Scope: `crates/ai-pow`, `crates/ai-pow-zk`, and the immediate miner/consensus integration points needed to evaluate proof soundness and verifier DoS risk.

Update: this is a historical audit from before the grinding remediation. The
2026-05-31 remediation moves the Nockchain nonce into Pearl's attempt state
before `kappa`, commitments, noise seeds, and matmul-derived tile states.
Current code should be evaluated against
`2026-05-31_AI_POW_ONE_MATMUL_ONE_ATTEMPT_AUDIT.md` for nonce-grinding status
and against `2026-05-29_AI_ZKP_NOUN_WIRE_SPEC.md` plus
`2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md` for the recursive certificate
wire target. In particular, the canonical persisted commitment surface is now
only `[h_a_chunk h_b_chunk]`; legacy row/column roots `h_a` / `h_b` are
diagnostic/plain-proof opening roots and must not be reintroduced as block
artifact commitment parameters.

Update, 2026-06-01: references below to `nonce=ai-ncmn` / `@uxncmn` are
historical. The current canonical Hoon block artifact is
`[%ai-pow nonce=ai-pow-nonce cert=ai-pow-certificate]`, where
`ai-pow-nonce` is an opaque Rust-owned `[len data]` envelope. See
`2026-06-01_PEARL_MERGE_MINING_COMPATIBILITY_SPEC.md`.

## Executive Summary

The current branch must not be enabled for accepting `%ai-pow` blocks yet. The Hoon consensus path is still reject-all, which prevents exploitation on-chain today, but the Rust proof/verifier APIs are not yet safe as consensus interfaces.

The two most important soundness problems are:

1. The plain `ai-pow` verifier recomputes difficulty from `params.difficulty_bits`, while the miner mines against a chain-supplied 256-bit target. A verifier using the current API can accept fake work for the real chain target.
2. The ZK bridge proves `BLAKE3(M, key=s_a) <= target`, but the plain PoW and miner use `BLAKE3(M, key=pow_key_for_nonce(s_a, nonce)) <= target`. The ZK proof is therefore not a proof for the winning nonce.

An additional miner-side integration bug has been remediated: the node run
loop used to hash the jammed target noun instead of decoding the Hoon bignum
target. That made the miner's attempted difficulty a serialization hash rather
than the chain target and could cause honest miners to mine the wrong puzzle.

There are also API-level hazards: unsound/dev verifier functions are public and
re-exported, pinned verifier functions accept caller-supplied programs and
`sx_bound`, and Hoon consensus still needs the verifier jet/wiring. Historical
plain-proof DoS issues around full noise expansion and parameter-unaware
decoding have been remediated at the Rust diagnostic boundary, but the
canonical production path remains the bounded recursive certificate noun
verifier.

## Required Production Invariant

A production verifier must accept an AI-PoW block only if all of the following are derived from trusted chain data or verified proof contents:

- `params` are exactly the chain-admitted AI puzzle parameters and pass the production envelope.
- `target` is the exact chain target for the candidate block.
- `nonce` is the exact nonce committed into the block.
- `attempt_state = block_state(block_commitment, nonce)`, and
  `kappa = commitment_key(attempt_state, params_tag(params))`; omitting the
  nonce here would re-open cached-matmul grinding.
- `s_b` and `s_a` are derived from the proof-bound chunk commitments
  `h_a_chunk` / `h_b_chunk`, which are exposed as the recursive proof's
  `HASH_A` / `HASH_B` public inputs. Legacy row/column roots `h_a` / `h_b`
  are not seed inputs and are not persisted commitment parameters.
- `pow_key = pow_key_for_nonce(s_a, nonce)`.
- The winning hash is `BLAKE3(M, key=pow_key)`, not `BLAKE3(M, key=s_a)`.
- The STARK proof is verified only against a verifier-rebuilt canonical program, never a prover-supplied program.
- All proof bytes are decoded under strict byte, count, path, and shape limits before any large allocation or expensive proof verification.

## Critical Findings

### SND-01: Plain verifier ignores the chain target

Severity: Critical
Status: Implemented at the Rust API boundary; consensus still remains
fail-closed until the Hoon verifier wiring calls the target-explicit
recursive verifier.

Evidence:

- `ai_pow_miner::pearl_mining::PearlMergeMiningJob` carries the caller-supplied
  256-bit Nockchain target.
- The production miner no longer exposes the legacy NCMN `MiningJob` /
  `mining::run` path. Its connected run loop derives a Pearl-compatible ticket
  attempt and only constructs the recursive certificate after the ticket clears
  the Nockchain target.
- `ai_pow::verifier::verify` has no target argument and instead computes `difficulty_target(params)` from `params.difficulty_bits`: `crates/ai-pow/src/verifier.rs`.
- `mine_with_context_at_target` explicitly documents that the chain target may not equal `difficulty_target(params)`: `crates/ai-pow/src/prover.rs`.
- The crate root no longer re-exports plain `MatmulProof` objects, plain mining
  helpers, or plain verifier helpers. Intentional plain-proof checks must use
  explicit `ai_pow::proof`, `ai_pow::prover`, or `ai_pow::verifier` module
  paths.
- `ai-pow` README now documents `verify_prod_at_target` as a diagnostic /
  pre-ZKP target-hit check, not as the canonical block verifier, and labels
  `verifier::verify` as non-consensus.

Attack sketch:

1. A malicious miner chooses or receives a hard chain target.
2. It submits a proof whose tile hash does not clear the chain target.
3. If `params.difficulty_bits` corresponds to an easier target, or is left at the current default `0`, `ai_pow::verifier::verify` accepts.

Impact:

Fake proof of work if consensus uses `ai_pow::verifier::verify` directly.

Fix plan:

- Done: add `verify_at_target(block_commitment, nonce, params, target, proof)`.
- Done: keep `verify` only as `ai_pow::verifier::verify` for
  params-derived-target tests/local tools and remove it from the crate-root
  production-facing re-export set.
- In future Hoon/Rust consensus integration, call only the target-explicit
  recursive verifier boundary.
- Consider removing `difficulty_bits` from the plain external-target path or explicitly documenting it as non-consensus metadata.

Tests:

- Mine with an easy external target and verify with a harder chain target; `verify_at_target` must reject.
- Set `params.difficulty_bits = 0` and chain target to zero; verifier must reject unless the actual hash is zero.
- Regression test that `verify` is not advertised from the crate-root
  production API surface.

### SND-02: ZK bridge proves a nonce-independent PoW hash

Severity: Critical
Status: Remediated at production and executable harness boundaries

Evidence:

- Plain PoW derives `pow_key_for_nonce(s_a, nonce)` and hashes tile states with that key: `crates/ai-pow/src/fiat_shamir.rs`, `crates/ai-pow/src/prover.rs`, `crates/ai-pow/src/verifier.rs`.
- Done: `zk_bridge::prove_and_verify_for_block` and the recursive certificate builder take the submitted nonce and reject context/nonce mismatches before proof construction.
- Done: the bridge derives `pow_key = pow_key_for_nonce(s_a, nonce)`, binds it into `CompositePublicInputs.commitment_hash`, and uses it as the BLAKE3 key for `HASH_JACKPOT`.
- Done: the executable `f1_harness` now uses `ctx.pow_key()` for `COMMITMENT_HASH` and the jackpot hash block; the wire regression rejects the old `COMMITMENT_HASH = s_a` harness wording.

Attack sketch:

1. A miner finds or fabricates a tile that clears under `key=s_a`.
2. It provides a nonce that does not clear under `pow_key_for_nonce(s_a, nonce)`.
3. The ZK bridge can still accept because the nonce is absent from the statement.

Impact:

The ZK proof is not a proof for the submitted block nonce. It proves a different, nonce-independent puzzle.

Fix plan:

- Done: add `nonce` to the ZK bridge production entrypoint.
- Done: derive `pow_key = pow_key_for_nonce(s_a, nonce)` outside the circuit from verifier-known data.
- Done: bind `pow_key` as a public input and use it as the BLAKE3 key for `HASH_JACKPOT`.
- Done: keep `COMMITMENT_HASH` as the existing field name for compatibility, but document and test that its production value is the nonce-bound jackpot key, not raw `s_a`.
- Update `CompositePublicInputs` with `pow_key` or replace `commitment_hash` in the jackpot block with a dedicated `jackpot_key`.

Tests:

- Done: generate one honest trace for a nonce and verify it rejects when the nonce is changed.
- Done: assert ZK `HASH_JACKPOT` equals plain `TileState::keyed_hash(pow_key_for_nonce(s_a, nonce))`.
- Done: executable and bridge-level tests exercise `COMMITMENT_HASH` as the
  nonce-bound `pow_key`, not raw `s_a`.
- Done: add a negative test where `BLAKE3(M, key=s_a)` clears target but
  `BLAKE3(M, key=pow_key)` does not; mining must return no solution.

### SND-03: No production verifier-only ZK API derives the trusted statement

Severity: Critical
Status: Remediated at the Rust recursive-certificate boundary; Hoon consensus
still remains fail-closed until the verifier jet calls that boundary.

Evidence:

- `composite_verify_pow_pinned_logup_sx` verifies a proof against caller-provided `program`, `public_inputs`, `target`, and `sx_bound`.
- The verifier-side canonical program needs `BlockPublic { tile_i, tile_j, kappa, s_a, s_b }`.
- `CompositePublicInputs` contains `job_key`, `commitment_hash`, and
  `hash_jackpot`. Current production statement checks derive `s_b`, `s_a`, and
  `pow_key` from verifier-trusted `(block_commitment, nonce, params, target,
  found_idx, h_a_chunk, h_b_chunk)` and then compare the public inputs against
  those derived values.
- The production-facing Rust artifact is the structured Pearl merge-mined
  `%ai-pow` noun. Its verifier path decodes bounded metadata, checks the
  Pearl/Nockchain statement and aux inclusion, re-derives public inputs from
  trusted block data, and only then reconstructs/verifies the recursive proof.

Attack sketch:

1. A downstream verifier accepts `public_inputs` from a prover.
2. The prover supplies self-consistent `job_key`, `s_a`, and matrix commitments that are not derived from the candidate block or authenticated matrix commitments.
3. The STARK verifies a statement, but not the chain statement.

Impact:

Easy misuse can create fake ZK PoW verification even if the AIR constraints are locally sound.

Fix plan:

- Done: `verify_ai_pow_full_matmul_production_statement` derives `kappa`,
  derives `s_a` / `s_b` from `h_a_chunk` / `h_b_chunk`, derives `pow_key`,
  checks `found_idx`, checks target satisfaction, and rejects multi-tile
  selected-tile statements with `FullMatmulProofUnavailable`.
- Superseded: the legacy NCMN Rust artifact verifier entrypoints were removed
  from `ai-pow-miner`. The current recursive-certificate Rust boundary is the
  Pearl merge-mined `%ai-pow` artifact verifier, which rejects cheap statement
  and metadata failures before recursive proof reconstruction.
- Done: lower-level selected-tile statement helpers are crate-private; dev
  `ai-pow-zk` verifier helpers are gated behind explicit `dev-unsafe`.
- Current batch-STARK checkpoint artifact fields are `nonce`, `zk_params`,
  `found_idx`, `trace_height`, `[h_a_chunk h_b_chunk]`,
  `CompositePublicInputs`, and the structured recursive certificate. It must
  not persist legacy `h_a` / `h_b` row/column roots. The production recursive
  proof target is the native terminal certificate.

Tests:

- Verifier rejects if any of `job_key`, `s_a`, `s_b`, `hash_a`, `hash_b`, `pow_key`, `found_idx`, or `target` is substituted.
- Verifier rejects if the canonical program is reconstructed from block data but the proof was generated under a prover-chosen program.

### SND-04: Unsound/dev verifier entrypoints are public and re-exported

Severity: Critical for downstream misuse
Status: Remediated at production-facing exports; Layer-0 modules remain
available for explicit circuit development and are not block/wire APIs.

Evidence:

- `composite_proof.rs` documents `composite_prove` / `composite_verify` as “not sound for PoW” because a prover can zero selectors.
- Current root exports gate the unpinned/no-LogUp helpers behind
  `#[cfg(any(test, feature = "dev-unsafe"))]` and expose them only with
  explicit `dev_*` names.
- The top-level `ai-pow-zk` README now shows `composite_prove_pinned_logup`
  only as a local Layer-0 circuit check and says block/wire/Hoon boundaries
  must use the recursive certificate APIs plus the full-matmul statement
  precheck.
- `ai-pow/src/zk_bridge.rs` previously published the legacy byte
  envelopes (`ZkProofArtifact`, `AiPowConsensusArtifact`,
  `AiPowProductionArtifact`) plus `prove_ai_pow_block`,
  `verify_ai_pow_block`, and `verify_ai_pow_consensus_artifact`, which made the
  Layer-0 bridge easy to mistake for the production block verifier.

Attack sketch:

1. An integrator uses `ai_pow_zk::composite_verify_pow` because it appears to be the full PoW verifier.
2. A malicious prover generates an unpinned trace with selectors disabled and a forged low `HASH_JACKPOT`.
3. The verifier accepts the proof and difficulty check.

Impact:

Fake ZK proof acceptance through public API misuse.

Fix plan:

- Done: move unpinned/no-LogUp root exports under
  `#[cfg(any(test, feature = "dev-unsafe"))]`.
- Done: rename those root exports with explicit names:
  `dev_unpinned_verify`, `dev_pinned_no_logup_verify`, etc.
- Export only production boundaries that derive the canonical statement
  internally. Current `ai-pow` status: `prove_ai_pow_recursive_certificate`
  and `verify_ai_pow_full_matmul_production_statement` remain public, while
  `verify_ai_pow_selected_tile_statement` is crate-internal selected-tile
  statement plumbing for the current Pearl-style recursive certificate. The
  legacy Layer-0 byte artifacts and helper verifier/constructor functions are
  crate-internal.
- Done: top-level docs no longer advertise raw Layer-0 prove/verify as the
  normal block artifact path, and `dev-unsafe` is explicitly not for
  consensus/block-wire consumers.

Tests:

- Existing selector-zero forgery tests should stay, but the forged proof should be impossible to verify through any non-dev public API.
- Add behavior-level consensus integration coverage when the verifier jet is
  wired. Do not use source-grep tests for this; the useful test is an actual
  accepted/rejected block artifact path through the verifier boundary.

### SND-05: Non-production params can bypass canonical program verification

Severity: Critical if non-prod params are accepted by a verifier
Status: Implemented at the production Rust API boundary

Evidence:

- Historical hazard: `zk_bridge::prove_and_verify_tiled` accepted structurally
  valid non-production params and older bridge paths could fall back to
  prover-derived programs when the canonical program could not be rebuilt.
- Current production entrypoints reject this before proving or verifying:
  `prove_and_verify_for_block` and `prove_ai_pow_recursive_certificate` call
  `validate_prod_envelope()`, and
  `verify_ai_pow_full_matmul_production_statement` re-runs the same envelope
  check before accepting recursive certificate metadata. The full-matmul API
  also rejects multi-tile selected-tile statements until the recursive proof
  binds a full-matrix aggregate.
- The legacy Layer-0 helper paths are crate-internal/test-only and are no
  longer normal public production APIs.

Attack sketch:

1. A chain or testnet admits params that pass `validate()` but not the production envelope.
2. `noise_rank` is not divisible by 16.
3. The verifier path cannot rebuild a canonical program and may verify against the prover's program.

Impact:

CRIT-1 is reopened for non-production parameter sets if they are ever accepted outside local tests.

Fix plan:

- Done: production entrypoints call `validate_prod_envelope()` before any ZK
  proof generation or production statement verification.
- Done: `prove_ai_pow_recursive_certificate` now rejects structural test params
  and multi-tile selected-tile statements instead of attempting to build a
  production-facing recursive certificate.
- Keep the non-production Layer-0 seams crate-internal/test-only.

Tests:

- Production verifier rejects `MatmulParams::TEST_SMALL` and any non-envelope
  shape.
- `param01_prove_and_verify_for_block_rejects_non_prod_params` now covers both
  the older hardened block bridge and the public recursive certificate prover.
- Production mining builds the recursive certificate through the explicit
  builder path, and bridge tests cover production-envelope rejection.

## High Severity Findings

### DOS-01: Plain verifier expands full noise before cheap rejection

Severity: High
Status: Remediated for current plain verifier; keep regression coverage

Evidence:

- Historical issue: `ai_pow::verifier::verify` called
  `BlockNoise::expand(&s_a, &s_b, params)` before verifying spot count,
  challenge indices, or proof shape.
- Current `verify_at_target` validates params, checks the proof params tag,
  computes cheap transcript values, checks spot count, prechecks found/spot
  opening shape and coordinates, and only then constructs verifier noise.
- Current verifier noise is on-demand: `VerifierNoise` precomputes only the
  `k`-length sparse-position schedules and derives opened rows/columns during
  `verify_opening`. It no longer allocates full `m * r` and `n * r` noise
  matrices.
- `rejects_spot_count_before_full_noise_expansion` asserts that a huge-`m`
  malformed proof rejects before any `BlockNoise::expand` call.

Attack sketch:

1. Attacker submits params with very large `m` and/or `n` but small tile grid, so `validate()` can pass.
2. Historical verifier allocated and filled huge noise arrays before rejecting the proof.
3. Node could run out of memory or spend excessive CPU.

Impact:

The original full-noise allocation DoS is not present in the current plain
verifier. Plain proofs remain diagnostic/non-consensus; production block
acceptance must use the recursive certificate noun boundary with chain-pinned
params.

Fix plan:

- Done: production wrappers enforce `validate_prod_envelope()`.
- Done: plain verifier prechecks params tag, spot count, coordinates, and
  opening shapes before expensive opening verification.
- Done: full `BlockNoise::expand` was replaced with on-demand row/column
  derivation for opened rows/columns.
- Keep hard resource-cap coverage in recursive-certificate decoders and in
  `MatmulProof::decode_for_params` for legacy/plain diagnostics.

Tests:

- Done: malformed huge-`m` proof with the wrong spot count rejects before full
  noise expansion.
- Done: proof decode DoS tests assert attacker-declared counts and shape
  prefixes do not amplify allocation.

### DOS-02: Proof decoding is not parameter-aware and still permits large real-input bombs

Severity: High
Status: Remediated for verifier-facing plain-proof decoding

Evidence:

- `MatmulProof::decode` remains a loose legacy/offline decoder and is
  documented as such. It no longer preallocates attacker-declared spot/path
  counts.
- `MatmulProof::decode_for_params` is the verifier-facing decoder. It computes
  a parameter-derived maximum encoded length, rejects oversized bodies before
  parsing, and checks spot count, strip lengths, path counts, and path depths
  while reading prefixes.
- `MatmulProof::decode_consensus_for_params` wraps the legacy body in a
  versioned length envelope, rejects bad magic/versions/length mismatches, then
  delegates to `decode_for_params`.

Attack sketch:

1. Send a syntactically valid proof with many actual spot openings or huge strip/path fields.
2. Historical verifier-facing decode allocated and copied the whole object.
3. Verifier later rejected because `spot.len() != params.spot_checks` or strip lengths did not match.

Impact:

The verifier-facing plain-proof decode path is bounded by the expected
parameter shape. Generic `decode` remains non-consensus tooling.

Fix plan:

- Done: `MatmulProof::decode_for_params(bytes, params)` enforces a total byte
  limit before decoding, requires exact spot count, strip lengths, path-list
  counts, path depths, and trailing-byte rejection.
- Done: `decode_consensus_for_params` adds a versioned length envelope for the
  legacy plain proof format.
- Keep generic `decode` documented as loose legacy/offline tooling only.

Tests:

- Done: declared `spot_count > params.spot_checks` rejects before decoding
  spot bodies.
- Done: oversized strip fields reject before allocation.
- Done: the allocator regression covers a prefix-only `decode_for_params`
  strip-length bomb.
- Done: wrong path/strip counts reject in decode, not verify.

### SND-06: Plain proof carries chunk commitments that the plain verifier ignores

Severity: High for combined plain+ZK integration
Status: Remediated for seed binding; legacy plain proof remains diagnostic and
must not be used as the canonical block artifact.

Evidence:

- `MatmulProof` includes `h_a_chunk` and `h_b_chunk` for diagnostic/plain
  pre-ZKP checks.
- Current `ai_pow::verifier::verify_at_target` derives canonical noise seeds
  from `proof.h_a_chunk` / `proof.h_b_chunk`; tampering either value changes
  `s_A`, the verifier-derived `found_idx`, the jackpot key, and/or the Merkle
  leaf checks and is rejected.
- The recursive certificate statement separately checks
  `pis.hash_a == h_a_chunk` and `pis.hash_b == h_b_chunk`, using the same
  proof-bound chunk commitment family as the seed chain.

Attack sketch:

1. A future consensus artifact includes both a plain proof and ZK proof.
2. The plain proof is accepted, but its `h_a_chunk` / `h_b_chunk` fields are arbitrary.
3. If downstream code assumes those fields were authenticated by the plain verifier, it can bind the ZK proof to different matrix commitments than the plain spot-check proof.

Impact:

Commitment substitution risk at the integration boundary.

Fix plan:

- Done: canonical seed derivation uses `h_a_chunk` / `h_b_chunk` through
  `canonical_noise_seeds_from_matrix_commitments`.
- Done: the persisted recursive certificate noun carries only
  `[h_a_chunk h_b_chunk]` in `ai-pow-commitments`; `h_a` / `h_b` stay out of
  the block artifact.
- Keep `MatmulProof` as a legacy diagnostic/pre-ZKP artifact only. Consensus
  block acceptance must enter through the recursive certificate noun verifier,
  not through plain proof verification.

Tests:

- Mutating `h_a_chunk` / `h_b_chunk` must cause combined verification to fail.
- Plain-only verification should not expose authenticated chunk commitments unless it actually authenticated them.

### SND-07: `ctx.params` and explicit `params` can diverge in ZK bridge APIs

Severity: High API hazard
Status: Remediated at bridge entrypoints

Evidence:

- `BlockContext` stores `params`.
- `prove_and_verify_for_block`, `prove_and_verify_tiled`, and `prove_and_verify_tiled_full` also accept `params` separately.
- The bridge indexes `ctx.a`, `ctx.b`, `ctx.s_a`, `ctx.s_b`, `ctx.h_a_chunk`, and `ctx.h_b_chunk` while using the separately supplied `params` for shape, tile range, target, and circuit config.
- Current bridge entrypoints call `ensure_context_params(ctx, params)` before
  proof construction, trace sizing, target derivation, or matrix indexing. A
  mismatch returns `BridgeError::ParamsMismatch`.

Attack / failure sketch:

1. Caller builds `ctx` under one shape and calls the bridge with another shape.
2. The bridge may panic on indexing, prove a statement over a subset/misaligned view, or compare against a target derived from different params.

Impact:

Verifier/prover crash or invalid statement construction through API misuse.

Fix plan:

- Done: retain the separate `params` argument for existing bridge call sites,
  but compare it to `ctx.params` and return a typed error before allocation or
  indexing.

Tests:

- Done: build `ctx` with one params set and call the bridge with another; it
  returns `ParamsMismatch`, not a panic.

### SND-08: Legacy full-matrix `place_matrix_hash` is non-canonical for non-power-of-two chunk counts

Severity: High if used in production traces
Status: Disproven by current in-code equivalence tests

Evidence:

- Historical concern: BLAKE3's tree split is described as
  largest-power-of-two-left, while `CompositeTrace::place_matrix_hash` uses a
  bottom-up pair-adjacent/promote-odd parent reduction.
- Current tests in `ai-pow-zk` show those constructions are equivalent for
  the relevant keyed chunk tree: `place_matrix_hash_equals_true_tree_and_blake3_all_counts`
  sweeps `1..=31` chunks, and
  `place_matrix_hash_equals_blake3_large_nonpow2` covers a 100-chunk
  non-power-of-two case.
- `CompositeTrace::place_matrix_hash_matches_blake3_for_non_power_of_two_chunk_counts`
  also compares 3, 5, 9, 17, 31, and 33 chunk matrices against
  `blake3::Hasher::new_keyed`.
- The production bridge currently uses strip openings, not the full-matrix
  helper, but the helper is not presently contradicted by the BLAKE3 reference.

Attack / failure sketch:

The original attack sketch would require `place_matrix_hash` to diverge from
`commit::matrix_commitment` for a non-power-of-two number of chunks. Current
behavior-level tests do not support that premise.

Impact:

No current soundness issue from this hypothesis. Keep the equivalence tests as
regression coverage because future edits to the tree walker or in-circuit hash
placement could reintroduce the mismatch.

Fix plan:

- Done: keep behavior-level tests comparing `place_matrix_hash` roots to
  `blake3::Hasher::new_keyed` for power-of-two and non-power-of-two chunk
  counts.
- No code change is currently required unless a future BLAKE3 compatibility
  review finds a missed flag/counter/detail not covered by those tests.

Tests:

- Done: 3, 5, 9, 17, 31, and 33 chunk matrices match canonical keyed BLAKE3.
- Done: 1 through 31 chunk matrices match both the off-circuit true-tree walker
  and `CompositeTrace::place_matrix_hash`.
- Done: 100 chunk non-power-of-two matrix matches keyed BLAKE3.

### SND-09: Removed NCMN nonce anchor path must not re-enter consensus

Severity: High integration hazard
Status: Superseded by Pearl merge-mined `%ai-pow`; Hoon consensus remains
fail-closed until the Pearl verifier jet is wired.

Evidence:

- Historical: the removed NCMN miner built an 80-byte NCMN nonce containing the
  Nockchain block commitment and an extranonce.
- Historical: that miner called
  `mine_with_context_at_target(&ctx, job.puzzle_id, &nonce, &job.target, ...)`;
  `job.puzzle_id`, not the candidate block commitment, was the
  `block_commitment` argument to `ai-pow`.
- The low-level `ai_pow::verifier::verify` still treats `nonce` as opaque bytes
  and is not the NCMN production boundary.
- The production miner now submits only Pearl merge-mined `%ai-pow` artifacts.
  The generic `%ai-pow` artifact decoders are crate-internal implementation
  details; public verifier-facing code must use the Pearl merge artifact or
  command APIs.
- `precheck_ai_pow_pearl_merge_artifact_jam` and
  `verify_ai_pow_pearl_merge_artifact_jam` enforce a jammed-byte cap before
  cueing the block artifact, so the future consensus path has a bounded byte
  entrypoint rather than relying on every caller to remember to cap attacker
  input first. They also run a no-allocation jam preflight for noun count,
  depth, and atom bytes before any `NounSlab` allocation, reject empty jam
  input, convert cue panics into verifier errors, preventing malformed block
  artifacts from crashing the verifier process, and reject non-canonical jam
  encodings by
  requiring a byte-identical re-jam.
- The Hoon `%ai-pow` wire now carries `[%ai-pow nonce=ai-pow-nonce
  cert=ai-pow-certificate]`, so the verifier has a Rust-owned opaque nonce
  envelope needed to bind the recursive certificate to the block attempt.

Attack sketch:

1. A future consensus verifier reintroduces the removed NCMN-style explicit
   nonce path.
2. It verifies the PoW hash but does not bind the work to verifier-trusted
   block data.
3. A proof can be accepted as work for a block it did not anchor to.

Impact:

Block-binding failure at the chain integration layer. This is especially dangerous combined with SND-02, where the ZK proof omits the nonce entirely.

Fix status:

- Superseded: the NCMN nonce parser, miner, and Rust artifact verifier helpers
  have been removed from `ai-pow-miner` and `ai-pow`.
- Done: the standalone miner carries the trusted candidate Nockchain
  commitment through the Rust-only Pearl aux payload and recursive certificate
  generation checks ticket-derived metadata against the opaque recursive prover
  run before submission.
- Done: Pearl merge artifact tests cover malformed opaque nonce envelopes,
  replay, aux-inclusion tampering, target misses, stale recursive-run metadata,
  and bounded decode ordering.

Tests:

- Honest Pearl-compatible ticket builds and submits a Nockchain `%ai-pow` poke.
- Reusing or tampering with candidate block commitment / aux data rejects.
- Malformed `AIP1` nonce envelopes reject before proof-tail traversal.
- Target misses do not produce `%ai-pow` artifacts.

### SND-10: Miner hashed the target noun instead of decoding the chain target

Severity: High integration hazard
Status: Remediated in `ai-pow-miner` node run-loop input derivation.

Evidence:

- The Hoon `%mine-ai` effect carries `target=bignum:bignum`, represented as
  `[%bn p=(list u32)]` with limbs in least-significant-first order.
- `ai_pow::tile_hash::hash_le_target` compares BLAKE3 attempt hashes against a
  32-byte little-endian unsigned integer.
- The previous `derive_job_inputs` implementation used
  `BLAKE3(jam(candidate.target))` as the miner's 32-byte target. This is not
  order-preserving, not equal to the chain bignum, and depends on noun
  serialization rather than consensus difficulty.

Attack / failure sketch:

1. The node emits a valid candidate with target `T`.
2. The miner attempts work against `BLAKE3(jam(T))` instead of `T`.
3. If the hashed target is easier than the real target, the miner can waste
   proof-generation effort producing blocks the verifier must reject. If it is
   harder, honest mining is artificially throttled. Either case means the
   miner is not solving the advertised chain puzzle.

Impact:

Incorrect miner behavior and potential block-submission DoS against honest
miners. This is not an on-chain fake-work acceptance path while Hoon remains
fail-closed, but it would become a consensus-integration hazard if verifier and
miner target derivations diverged.

Fix status:

- Done: `derive_job_inputs` decodes `[%bn limbs]` directly.
- Done: the first eight u32 limbs are packed into the 256-bit little-endian
  target used by `ai-pow`.
- Done: bignum values above `2^256 - 1` saturate to `FF..FF`, matching the
  256-bit BLAKE3 hash domain.
- Done: malformed targets, non-u32 limbs, and oversized limb lists reject
  before mining starts.

Tests:

- Behavior-level Rust tests decode real noun bignums into little-endian target
  bytes.
- Over-256-bit Hoon targets saturate to max 256-bit target.
- Malformed target nouns reject without source-text assertions.

## Medium Severity Findings

### API-01: `sx_bound` is caller-controlled in public verifier APIs

Severity: Medium to High depending on misuse
Status: Remediated at public exports

`composite_verify_pinned_logup_sx` and `composite_verify_pow_pinned_logup_sx` accept `sx_bound` directly. If a downstream verifier passes `false`, it disables the `FOLD_XSTEP == SX_XR` keystone. This must be derived from trusted params inside the production verifier, not supplied by the prover or caller.

Fix:

- Done: `_sx` variants are crate-private and are not re-exported from
  `ai-pow-zk`.
- Done: production-facing recursive certificate and statement-verifier
  boundaries derive or reject the selected-tile/full-matmul params before proof
  work; they do not accept a caller-supplied `sx_bound`.

### API-02: Public pinned verifier APIs panic on malformed `program`

Severity: Medium DoS
Status: Remediated for verifier entrypoints; low-level constructors remain
trusted-setup APIs

Historical issue: the pinned verifier path used panic-on-bad-shape helpers before
constructing the verifier's preprocessed AIR. A malformed `program` could
therefore become a verifier crash if it ever crossed a public verification
boundary.

Current behavior:

- Done: `ProgramShapeError` is the shared typed error for malformed program
  width and non-power-of-two height.
- Done: `CompositeFullAirPinned` and `CompositeFullAirWithLookupsPinned` expose
  fallible `try_new` / `try_new_with` constructors.
- Done: `composite_verify_pinned_logup` and
  `composite_verify_pow_pinned_logup` validate the program and construct the
  AIR through fallible constructors before invoking batch verification.
- Done: dev-only no-LogUp pinned verifier APIs also validate before AIR setup.
- Done: malformed width and malformed height regression tests wrap the verifier
  calls in `catch_unwind` and assert typed errors instead of panics.

Residual surface:

- `new` / `new_with` constructors remain convenience APIs for trusted setup and
  internally derived canonical programs. They intentionally `expect` after the
  same validation and should not be used on wire/proof data.
- Production consensus still must rebuild the canonical program from trusted
  params and must not deserialize a prover-supplied program from the block.
- Strip-opening helpers still contain local invariants and should stay behind
  canonical verifier scheduling. If any become directly exposed to block bytes,
  add typed validation at that boundary.

### PARAM-01: Production envelope is documented but not enforced in exposed verifier/miner entrypoints

Severity: Medium to High
Status: Implemented at production boundaries; legacy miner submission removed

Historical issue: `MatmulParams::validate_prod_envelope()` captured the
consensus/security envelope, but production-looking entrypoints previously used
only `validate()`.

Current status:

- `ai-pow-miner::pearl_mining::run` is the only miner loop, and the connected
  run-loop preflight validates the Pearl merge config against the canonical
  recursive prover envelope before enabling mining. Pearl-ticket recursive
  certificates use an explicit row/column strip schedule, so bounded
  rectangular, non-contiguous, and non-native periodic-pattern tickets are not
  reduced to the native square-tile grid.
- `ai_pow::verifier::verify_prod_at_target` enforces the production envelope
  for plain-proof diagnostics. The structured Pearl merge recursive certificate
  noun verifier is the block/wire boundary.
- `zk_bridge::prove_and_verify_for_block`,
  `prove_ai_pow_recursive_certificate`, and
  `verify_ai_pow_full_matmul_production_statement` enforce the production
  envelope. Native full-matmul entrypoints stay on the production square-grid
  envelope, while Pearl-ticket recursive certificates use scheduled validation
  derived from the committed ticket rows and columns.
- `ai-pow-mine`'s recursive certificate builder is Pearl-ticket-derived and
  constructible only after the ticket loop reports a Nockchain target hit, using
  the trusted candidate Nockchain commitment carried by the mining result before
  it starts recursive proof generation.
- `validate()` and `verify_at_target` remain available for tests/local tools
  and are documented as non-production helpers.

Fix:

- Keep source-level guards/tests proving production call sites use the Pearl
  merge recursive envelope.
- Do not re-export or advertise structural helpers as consensus APIs.

### WIRE-01: `%ai-pow` consensus wire is a placeholder and reject-all

Severity: Medium integration blocker
Status: Wire shape defined; consensus verifier remains fail-closed

Historical issue: `hoon/apps/dumbnet/lib/types.hoon` originally defined `%ai-pow`
as a placeholder atom, and `do-pow` rejected all `%ai-pow` submissions after
activation. The reject-all behavior was safe, but the placeholder type made the
intended block artifact ambiguous.

Current behavior:

- Done: the Hoon wire type is now
  `[%ai-pow nonce=ai-pow-nonce cert=ai-pow-certificate]`.
- Done for the batch-STARK checkpoint path: `ai-pow-certificate` is a
  structured recursive certificate noun using custom compact auras for BLAKE3
  digests, AI-PoW nonce bytes, and degree-2 field elements. The production
  recursive proof target is the native terminal certificate, not this oversized
  checkpoint noun.
- Done for checkpoint plumbing: the Rust miner emits
  `[%command %pow %ai-pow nonce cert]` only after it has a recursive
  certificate builder.
- Done: `do-pow` still rejects `%ai-pow` before activation and still rejects it
  after activation with `do-pow: %ai-pow verifier not wired; rejected`.
- Done: `do-mine` still emits only `%mine-zk`; it refuses to emit `%mine-ai`
  while `do-pow` must reject the result.

Deferred verifier work:

- Wire the Hoon/Rust verifier jet only when that milestone is explicitly in
  scope, so consensus can bounded-decode the jammed Pearl merge `%ai-pow`
  artifact, check the Pearl/Nockchain statement against trusted block data, and
  call the full recursive certificate verifier.
- Only after that verifier exists, add a post-activation integration test that
  accepts one honest `%ai-pow` artifact and rejects tampered nonce, params,
  commitments, public inputs, recursive proof, target, and candidate-block
  anchor.

Fix:

- Keep activation defaults safe until the real verifier is wired.
- Keep `%mine-ai` disabled until `%ai-pow` can be accepted by consensus.
- Do not add the honest/tampered post-activation integration suite until real
  verifier work is explicitly scheduled.

### CRYPTO-01: STARK security target and comments need independent sign-off

Severity: Medium
Status: Remediated in tree; requires external activation sign-off

`CircuitConfig::PROD` currently uses `log_blowup = 4`, `num_queries = 15`, `pow_bits = 1`. `build_stark_config` applies `pow_bits` to both the commit-time and query-time FRI PoW tiers, so the in-tree Johnson-radius accounting is `4 * 15 + 2 * 1 = 62` bits. The accepted in-tree floor is `PROD_JOHNSON_FLOOR_BITS = 60`, justified by the documented 2.5-minute block-cadence threat model and the maintainer's 2026-05-21 anchored-between policy.

Current behavior:

- Done: `CircuitConfig::johnson_fri_bits()` is the single executable accounting helper.
- Done: `circuit_config_constants_are_well_formed`, `prod_sweep_profiles_meet_anchored_johnson_floor`, and `build_stark_config_provable_soundness_at_prod` assert the production constants and minimum Johnson floor.
- Done: stale in-tree comments that treated the inner `pow_bits = 1` as only one bit were corrected to `pow=1+1` / 62 bits.

Activation requirement:

- Before enabling `%ai-pow` consensus acceptance, record explicit external protocol sign-off that the 60-bit time-bounded floor is acceptable for Nockchain's block interval and threat model. Without that sign-off, keep activation fail-closed.

### SERIAL-01: Proof serialization needs a consensus envelope

Severity: Medium
Status: Remediated for current Rust/Hoon artifact shape; consensus verifier
wiring remains WIRE-01

Historical issue: `MatmulProof` had an ad hoc binary format with no version byte,
no top-level length prefix, and unchecked `usize -> u32` casts in `encode`.
Earlier `ai-pow-zk` examples also used raw `bincode` proof bytes without a
bounded consensus decoder.

Current behavior:

- Done: `MatmulProof` is documented as a legacy diagnostic/plain-proof
  artifact, not the canonical production block proof.
- Done: legacy `MatmulProof::encode_consensus` / `decode_consensus_for_params`
  wrap the old body in `AIPW || version || body_len`, use checked length
  conversion, and reject bad magic, unknown versions, length mismatch, trailing
  bytes, and shape/parameter mismatches before verifier work.
- Done: production persistence/wire shape is structured Hoon noun data:
  `[%ai-pow nonce=ai-pow-nonce cert=ai-pow-certificate]`, where the nonce is a
  Rust-owned `[len=@ud data=@uxaipownonce]` envelope and the certificate is a
  structured recursive proof-node tree, not a raw Layer-0 STARK blob and not a
  single opaque atom.
- Done: `CertificateNounLimits` bounds recursive proof-node depth, total nodes,
  list lengths, atom bytes, packed item counts, and jammed artifact bytes.
- Done: Pearl merge artifact verification enforces a byte cap before cueing,
  preflights jam structure, rejects noncanonical jam re-encodings, decodes the
  bounded noun, rejects unknown certificate versions/tags, and runs the
  Pearl/Nockchain statement precheck before reconstructing the recursive
  certificate.
- Done: Hoon types use custom auras for compact atoms (`@uxblake`,
  `@uxaipownonce`, `@uxfelt`, `@uxfelts`) instead of digest/field tuples.

Regression coverage:

- Legacy plain-proof consensus envelope roundtrips and rejects bad magic,
  unknown version, length mismatch, trailing bytes, and u32 length overflow.
- Structured certificate/artifact tests cover jam/cue roundtrip, byte-limit
  rejection before cue, noncanonical jam rejection, node/depth/atom/list
  limits, malformed nonce/tag rejection, unsupported certificate versions,
  oversized packed atoms, and noncanonical Goldilocks limbs.

Residual surface:

- Hoon consensus still rejects `%ai-pow` before recursive verifier wiring. Keep
  that fail-closed behavior until WIRE-01 lands and the Hoon/Rust jet calls the
  Pearl merge artifact verifier or the equivalent bounded decode plus full
  recursive verifier path.

## Recommended Fix Order

1. Keep Hoon `%ai-pow` reject-all until this checklist is complete.
2. Add target-explicit plain verification and update all call sites.
3. Add nonce/`pow_key` binding to the ZK statement.
4. Design the final consensus artifact and verifier-only Rust API.
5. Hide or rename all dev/unsafe ZK APIs.
6. Enforce production params at production boundaries.
7. Make decoding parameter-aware and bounded.
8. Parse and enforce the opaque `AIP1` nonce and Pearl/Nockchain statement in
   the production verifier.
9. Replace full verifier noise expansion with on-demand derivation.
10. Fix or remove noncanonical `place_matrix_hash`.
11. Add end-to-end tamper tests for every trusted field.

## Minimum Test Matrix Before Activation

Plain verifier:

- Reject wrong target.
- Reject wrong nonce.
- Reject opaque nonce / aux data whose embedded Nockchain commitment does not
  match the candidate block.
- Reject wrong params tag.
- Reject wrong found tile path.
- Reject wrong spot index.
- Reject wrong A/B row or column opening.
- Reject oversized proof before large allocation.
- Memory use is bounded by opened strips, not full matrix dimensions.

ZK verifier:

- Reject wrong nonce / `pow_key`.
- Reject wrong target.
- Reject wrong `found_idx`.
- Reject wrong `kappa`.
- Reject wrong `s_a` and `s_b`.
- Reject wrong `hash_a` / `hash_b`.
- Reject prover-supplied program.
- Reject `sx_bound=false` for production params.
- Reject non-production params.
- Reject malformed proof bytes without panic.

Consensus integration:

- Pre-activation `%ai-pow` rejects.
- Post-activation honest AI proof accepts.
- Post-activation wrong target, nonce, params, commitments, public inputs, proof bytes, or tile index reject.
- `%dumb-zkpow` and `%ai-pow` target calculation cannot be cross-applied.

## Notes on Current Non-Exploitability

The Hoon `%ai-pow` branch currently rejects all submissions. The payload is no
longer a placeholder: it is the structured
`[%ai-pow nonce=ai-pow-nonce cert=ai-pow-certificate]` artifact, but Hoon keeps
it fail-closed because real verifier wiring is out of scope for the current
milestone. That means the critical Rust issues above are not currently
exploitable through the main consensus path. They become exploitable if a real
`%ai-pow` verifier is wired without entering through the full-matmul recursive
certificate boundary and rejecting selected-tile-only statements.
