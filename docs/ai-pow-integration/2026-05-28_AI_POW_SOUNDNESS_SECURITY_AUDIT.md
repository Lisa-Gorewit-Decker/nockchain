# AI-PoW / AI-PoW-ZK Soundness and Security Audit

Date: 2026-05-28
Branch audited: `claude/ai-pow-integration-squash`
Scope: `crates/ai-pow`, `crates/ai-pow-zk`, and the immediate miner/consensus integration points needed to evaluate proof soundness and verifier DoS risk.

Update: this is a historical audit. The 2026-05-31 grinding remediation moves
the Nockchain nonce into Pearl's attempt state before `kappa`, commitments,
noise seeds, and matmul-derived tile states. Current code should be evaluated
against `2026-05-31_AI_POW_ONE_MATMUL_ONE_ATTEMPT_AUDIT.md` for nonce-grinding
status.

## Executive Summary

The current branch must not be enabled for accepting `%ai-pow` blocks yet. The Hoon consensus path is still reject-all, which prevents exploitation on-chain today, but the Rust proof/verifier APIs are not yet safe as consensus interfaces.

The two most important soundness problems are:

1. The plain `ai-pow` verifier recomputes difficulty from `params.difficulty_bits`, while the miner mines against a chain-supplied 256-bit target. A verifier using the current API can accept fake work for the real chain target.
2. The ZK bridge proves `BLAKE3(M, key=s_a) <= target`, but the plain PoW and miner use `BLAKE3(M, key=pow_key_for_nonce(s_a, nonce)) <= target`. The ZK proof is therefore not a proof for the winning nonce.

There are also API-level hazards: unsound/dev verifier functions are public and re-exported, pinned verifier functions accept caller-supplied programs and `sx_bound`, and there is no production verifier-only wrapper that derives all public inputs from chain data. DoS risk is also material: the plain verifier expands full matrix noise before doing cheap rejection and the decoder is not parameter-aware.

## Required Production Invariant

A production verifier must accept an AI-PoW block only if all of the following are derived from trusted chain data or verified proof contents:

- `params` are exactly the chain-admitted AI puzzle parameters and pass the production envelope.
- `target` is the exact chain target for the candidate block.
- `nonce` is the exact nonce committed into the block.
- `kappa = commitment_key(block_commitment, params_tag(params))`.
- `s_b` and `s_a` are derived from authenticated row/column commitments, or otherwise are public inputs that the verifier can recompute from committed data.
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

- `ai_pow_miner::MiningJob` carries a caller-supplied 256-bit chain target: `crates/ai-pow-miner/src/lib.rs`.
- The mining loop passes that target to `mine_with_context_at_target`: `crates/ai-pow-miner/src/mining.rs`.
- `ai_pow::verifier::verify` has no target argument and instead computes `difficulty_target(params)` from `params.difficulty_bits`: `crates/ai-pow/src/verifier.rs`.
- `mine_with_context_at_target` explicitly documents that the chain target may not equal `difficulty_target(params)`: `crates/ai-pow/src/prover.rs`.
- The crate root no longer re-exports `verify`; production-facing imports
  expose `verify_at_target`, `verify_prod_at_target`, and
  `verify_ncmn_at_target`.
- `ai-pow` README now documents `verify_ncmn_at_target` as the production
  verifier and labels `verifier::verify` as non-consensus.

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
Status: Confirmed

Evidence:

- Plain PoW derives `pow_key_for_nonce(s_a, nonce)` and hashes tile states with that key: `crates/ai-pow/src/fiat_shamir.rs`, `crates/ai-pow/src/prover.rs`, `crates/ai-pow/src/verifier.rs`.
- `zk_bridge::prove_and_verify_for_block` has no `nonce` argument.
- The bridge places the jackpot hash using `s_a` / `COMMITMENT_HASH`: `trace.place_jackpot_hash_block(height - 8, &real_m, &s_a_w)`.
- `CompositePublicInputs` documents `HASH_JACKPOT = BLAKE3(JACKPOT_MSG, key=COMMITMENT_HASH)`, i.e. `s_a`.

Attack sketch:

1. A miner finds or fabricates a tile that clears under `key=s_a`.
2. It provides a nonce that does not clear under `pow_key_for_nonce(s_a, nonce)`.
3. The ZK bridge can still accept because the nonce is absent from the statement.

Impact:

The ZK proof is not a proof for the submitted block nonce. It proves a different, nonce-independent puzzle.

Fix plan:

- Add `nonce` to the ZK bridge production entrypoint.
- Derive `pow_key = pow_key_for_nonce(s_a, nonce)` outside the circuit from verifier-known data.
- Bind `pow_key` as a public input and use it as the BLAKE3 key for `HASH_JACKPOT`.
- Rename `COMMITMENT_HASH` usage in the jackpot path to avoid confusing `s_a` with `pow_key`.
- Update `CompositePublicInputs` with `pow_key` or replace `commitment_hash` in the jackpot block with a dedicated `jackpot_key`.

Tests:

- Generate one honest trace for a nonce and verify it rejects when the nonce is changed.
- Assert ZK `HASH_JACKPOT` equals plain `TileState::keyed_hash(pow_key_for_nonce(s_a, nonce))`.
- Add a negative test where `BLAKE3(M, key=s_a)` clears target but `BLAKE3(M, key=pow_key)` does not.

### SND-03: No production verifier-only ZK API derives the trusted statement

Severity: Critical
Status: Confirmed API gap

Evidence:

- `composite_verify_pow_pinned_logup_sx` verifies a proof against caller-provided `program`, `public_inputs`, `target`, and `sx_bound`.
- The verifier-side canonical program needs `BlockPublic { tile_i, tile_j, kappa, s_a, s_b }`.
- `CompositePublicInputs` contains `job_key`, `commitment_hash`, and `hash_jackpot`, but not `s_b`, the plain row Merkle roots `h_a`/`h_b`, or a nonce-derived `pow_key`.
- The bridge currently proves and immediately verifies locally; it returns `ZkOutcome { pis, sweep_in_circuit }` but no serialized proof artifact for consensus.

Attack sketch:

1. A downstream verifier accepts `public_inputs` from a prover.
2. The prover supplies self-consistent `job_key`, `s_a`, and matrix commitments that are not derived from the candidate block or authenticated matrix commitments.
3. The STARK verifies a statement, but not the chain statement.

Impact:

Easy misuse can create fake ZK PoW verification even if the AIR constraints are locally sound.

Fix plan:

- Add a single production verification API, for example:
  `verify_ai_pow_block(block_commitment, nonce, params, target, found_idx, proof_bytes, public_artifact)`.
- This API must derive `kappa`, derive or authenticate `s_a`/`s_b`, derive `pow_key`, derive canonical program, derive expected public inputs, and then call the pinned+LogUp verifier.
- Make lower-level verifier functions crate-private or mark them `unsafe_dev_*` behind a feature.
- Define the consensus artifact before implementing verification: it likely needs `params_tag`, `nonce`, `target`, `found_idx`, `h_a`, `h_b`, `h_a_chunk`, `h_b_chunk`, `s_b` derivation data or enough data to recompute it, `CompositePublicInputs`, and the STARK proof.

Tests:

- Verifier rejects if any of `job_key`, `s_a`, `s_b`, `hash_a`, `hash_b`, `pow_key`, `found_idx`, or `target` is substituted.
- Verifier rejects if the canonical program is reconstructed from block data but the proof was generated under a prover-chosen program.

### SND-04: Unsound/dev verifier entrypoints are public and re-exported

Severity: Critical for downstream misuse
Status: Partially remediated

Evidence:

- `composite_proof.rs` documents `composite_prove` / `composite_verify` as “not sound for PoW” because a prover can zero selectors.
- `composite_verify_pow` is public.
- `ai-pow-zk/src/lib.rs` re-exports `composite_verify`, `composite_verify_pow`, `composite_verify_pinned`, and `composite_verify_pinned_logup_sx`.
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

- Move unpinned APIs under `#[cfg(any(test, feature = "dev-unsafe"))]`.
- Rename remaining exported helpers with explicit names: `dev_unpinned_verify`, `dev_pinned_no_logup_verify`.
- Export only production boundaries that derive the canonical statement
  internally. Current `ai-pow` status: `prove_ai_pow_recursive_certificate`
  and `verify_ai_pow_production_statement` remain public, while the legacy
  Layer-0 byte artifacts and helper verifier/constructor functions are
  crate-internal.
- Add rustdoc `compile_fail` or lints preventing consensus crates from importing dev APIs.

Tests:

- Existing selector-zero forgery tests should stay, but the forged proof should be impossible to verify through any non-dev public API.
- Add a dependency-level test in the intended consensus crate that only the production verifier is used. Status: `ai_pow_wire_stub` now guards that the
  legacy Layer-0 byte envelopes and `prove_ai_pow_block` /
  `verify_ai_pow_block` / `verify_ai_pow_consensus_artifact` are not public
  APIs.

### SND-05: Non-production params can bypass canonical program verification

Severity: Critical if non-prod params are accepted by a verifier
Status: Implemented at the production Rust API boundary

Evidence:

- Historical hazard: `zk_bridge::prove_and_verify_tiled` accepted structurally
  valid non-production params and older bridge paths could fall back to
  prover-derived programs when the canonical program could not be rebuilt.
- Current production entrypoints reject this before proving or verifying:
  `prove_and_verify_for_block` and `prove_ai_pow_recursive_certificate` call
  `validate_prod_envelope()`, and `verify_ai_pow_production_statement`
  re-runs the same envelope check before accepting recursive certificate
  metadata.
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
  instead of attempting to build a canonical production certificate.
- Keep the non-production Layer-0 seams crate-internal/test-only.

Tests:

- Production verifier rejects `MatmulParams::TEST_SMALL` and any non-envelope
  shape.
- `param01_prove_and_verify_for_block_rejects_non_prod_params` now covers both
  the older hardened block bridge and the public recursive certificate prover.
- `ai_pow_wire_stub` guards that production mining builds the recursive
  certificate and that the bridge source still contains the production envelope
  check.

## High Severity Findings

### DOS-01: Plain verifier expands full noise before cheap rejection

Severity: High
Status: Confirmed

Evidence:

- `ai_pow::verifier::verify` calls `BlockNoise::expand(&s_a, &s_b, params)` before verifying spot count, challenge indices, or proof shape.
- `BlockNoise::expand` allocates `m * r` and `n * r` noise vectors and iterates over `m`, `n`, and `k`.
- `MatmulParams::validate()` is much looser than `validate_prod_envelope()`.

Attack sketch:

1. Attacker submits params with very large `m` and/or `n` but small tile grid, so `validate()` can pass.
2. Verifier allocates and fills huge noise arrays before rejecting the proof.
3. Node runs out of memory or spends excessive CPU.

Impact:

Remote verifier memory/CPU DoS if params are attacker-controlled or not strictly chain-pinned.

Fix plan:

- Enforce consensus params before decoding or verifying.
- Reorder verification: params admission, params tag, spot count, coordinate/path/strip shape, then expensive work.
- Replace full `BlockNoise::expand` in verifier with on-demand row/column derivation for only opened rows/columns.
- Add hard verifier resource caps independent of production envelope: maximum proof bytes, maximum `t * k`, maximum spot checks, maximum row/column openings, maximum total hash work.

Tests:

- Malformed params with huge `m`/`n` return an error without allocating proportional memory.
- Instrumented test or allocator test proving verifier memory is `O(spot_checks * tile * k)`, not `O((m+n) * r)`.

### DOS-02: Proof decoding is not parameter-aware and still permits large real-input bombs

Severity: High
Status: Confirmed

Evidence:

- `MatmulProof::decode` decodes `spot` using attacker-declared count up to `MAX_SPOT = 2^20`.
- `decode_path_list` allows attacker-declared strip path counts up to `MAX_STRIP_COUNT = 2^20`.
- `decode_i8_slice` allows each strip-concat field up to 16 MiB.
- Shape checks against `params.spot_checks`, `tile`, and `k` happen later in `verify`.

The prior up-front allocation bomb is partially fixed by avoiding `Vec::with_capacity(n)` for spot/path-list counts, but the decoder still accepts attacker-supplied actual bytes proportional to these loose caps before a params-aware verifier can reject them.

Attack sketch:

1. Send a syntactically valid proof with many actual spot openings or huge strip/path fields.
2. Decoder allocates and copies the whole object.
3. Verifier later rejects because `spot.len() != params.spot_checks` or strip lengths do not match.

Impact:

Remote memory/CPU DoS through large but syntactically valid proof blobs.

Fix plan:

- Add `MatmulProof::decode_for_params(bytes, params)` that:
  - enforces a total byte limit before decoding,
  - requires `spot_count == params.spot_checks`,
  - requires `a_rows.len() == tile * k`,
  - requires `b_cols.len() == tile * k`,
  - requires path-list counts equal `tile`,
  - requires path depths equal the expected Merkle depths,
  - rejects trailing bytes.
- Keep generic `decode` only for tests/tools, or make it private.
- Define network-level max proof byte size per admitted params.

Tests:

- Declared `spot_count > params.spot_checks` rejects before decoding spot bodies.
- Oversized strip fields reject before allocation.
- A proof with valid total bytes but wrong path count rejects in decode, not verify.

### SND-06: Plain proof carries chunk commitments that the plain verifier ignores

Severity: High for combined plain+ZK integration
Status: Confirmed

Evidence:

- `MatmulProof` includes `h_a_chunk` and `h_b_chunk`.
- `ai_pow::verifier::verify` validates `h_a` and `h_b` row/column Merkle roots but does not check or use `h_a_chunk` / `h_b_chunk`.
- The ZK bridge separately checks `pis.hash_a == ctx.h_a_chunk` and `pis.hash_b == ctx.h_b_chunk`.

Attack sketch:

1. A future consensus artifact includes both a plain proof and ZK proof.
2. The plain proof is accepted, but its `h_a_chunk` / `h_b_chunk` fields are arbitrary.
3. If downstream code assumes those fields were authenticated by the plain verifier, it can bind the ZK proof to different matrix commitments than the plain spot-check proof.

Impact:

Commitment substitution risk at the integration boundary.

Fix plan:

- Decide whether `h_a_chunk` / `h_b_chunk` belong in `MatmulProof`.
- If they remain, the plain verifier should recompute or cross-check them against a trusted source. If recomputation is too expensive, the verifier must return them as unauthenticated fields or ignore them entirely.
- The combined verifier must explicitly check that the plain proof commitments and ZK public inputs refer to the same matrix commitment family and seeds.

Tests:

- Mutating `h_a_chunk` / `h_b_chunk` must cause combined verification to fail.
- Plain-only verification should not expose authenticated chunk commitments unless it actually authenticated them.

### SND-07: `ctx.params` and explicit `params` can diverge in ZK bridge APIs

Severity: High API hazard
Status: Confirmed

Evidence:

- `BlockContext` stores `params`.
- `prove_and_verify_for_block`, `prove_and_verify_tiled`, and `prove_and_verify_tiled_full` also accept `params` separately.
- The bridge indexes `ctx.a`, `ctx.b`, `ctx.s_a`, `ctx.s_b`, `ctx.h_a_chunk`, and `ctx.h_b_chunk` while using the separately supplied `params` for shape, tile range, target, and circuit config.

Attack / failure sketch:

1. Caller builds `ctx` under one shape and calls the bridge with another shape.
2. The bridge may panic on indexing, prove a statement over a subset/misaligned view, or compare against a target derived from different params.

Impact:

Verifier/prover crash or invalid statement construction through API misuse.

Fix plan:

- Remove the redundant `params` argument from bridge functions; use `ctx.params`.
- If a separate argument is retained, assert equality and return a typed error before any allocation or indexing.

Tests:

- Build `ctx` with one params set and call bridge with another; must return `ParamsMismatch`, not panic.

### SND-08: Legacy full-matrix `place_matrix_hash` is non-canonical for non-power-of-two chunk counts

Severity: High if used in production traces
Status: Confirmed by in-code documentation

Evidence:

- `blake3_tree.rs` says BLAKE3's tree is not a naive pairwise-with-promotion reduction and that `CompositeTrace::place_matrix_hash` only coincides with the true tree for power-of-two chunk counts.
- `CompositeTrace::place_matrix_hash` still uses a pairwise-with-promotion parent layer and comments that this is the BLAKE3 spec for non-power-of-two chunk counts.
- The production bridge currently uses strip openings, not this full-matrix helper.

Attack / failure sketch:

If a future path uses `place_matrix_hash` for a matrix with a non-power-of-two number of chunks, the circuit can bind to a root different from `commit::matrix_commitment`. Depending on how the root is compared, this can either reject honest proofs or authenticate a non-standard commitment.

Impact:

Potential commitment mismatch or matrix-binding unsoundness if the helper is reused.

Fix plan:

- Replace `place_matrix_hash` parent reduction with the true `left_len` BLAKE3 tree.
- Add tests comparing `place_matrix_hash` roots to `blake3::Hasher::new_keyed` for non-power-of-two chunk counts.
- Until fixed, mark the helper dev-only or panic for non-power-of-two `num_chunks`.

Tests:

- 3, 5, 9, 17, 31, 33 chunk matrices must match `commit::matrix_commitment`.

### SND-09: NCMN nonce anchor is miner-side only unless consensus explicitly checks it

Severity: High integration hazard
Status: Implemented at the Rust and wire boundary; Hoon consensus remains
fail-closed until the verifier jet is wired.

Evidence:

- `ai-pow-miner` builds an 80-byte NCMN nonce containing the Nockchain block commitment and an extranonce.
- The miner calls `mine_with_context_at_target(&ctx, job.puzzle_id, &nonce, &job.target, ...)`; `job.puzzle_id`, not the candidate block commitment, is the `block_commitment` argument to `ai-pow`.
- The low-level `ai_pow::verifier::verify` still treats `nonce` as opaque bytes
  and is not the NCMN production boundary.
- `ai_pow::verifier::verify_ncmn_at_target` and
  `ai_pow_miner::certificate_noun::decode_ai_pow_artifact_jam` /
  `decode_ai_pow_artifact_slab` /
  `verify_decoded_ai_pow_ncmn_artifact` parse the full `[%ai-pow nonce cert]`
  artifact, reject malformed/reserved fields, reject nonzero external
  commitments, and require the embedded Nockchain commitment to match the
  verifier-trusted candidate block commitment.
- `decode_ai_pow_artifact_jam` enforces a jammed-byte cap before cueing the
  block artifact, so the future consensus path has a bounded byte entrypoint
  rather than relying on every caller to remember to cap attacker input first.
  It also runs a no-allocation jam preflight for noun count, depth, and atom
  bytes before any `NounSlab` allocation, rejects empty jam input, converts cue
  panics into verifier errors, preventing malformed block artifacts from
  crashing the verifier process, and rejects non-canonical jam encodings by
  requiring a byte-identical re-jam.
- The Hoon `%ai-pow` wire now carries `[%ai-pow nonce=ai-ncmn
  cert=ai-pow-certificate]`, so the verifier has the NCMN nonce needed to bind
  the recursive certificate to the block attempt.

Attack sketch:

1. A future consensus verifier calls the AI-PoW verifier with opaque nonce bytes.
2. It verifies the PoW hash but never checks that the nonce's `nck_commitment` equals the candidate block commitment.
3. A proof can be accepted as work for a block it did not anchor to.

Impact:

Block-binding failure at the chain integration layer. This is especially dangerous combined with SND-02, where the ZK proof omits the nonce entirely.

Fix status:

- Done: the production verifier parses the NCMN nonce.
- Done: bad magic, version, reserved bytes, and length reject.
- Done: `nonce.nck_commitment != candidate block commitment` rejects.
- Done: the opaque external commitment slot is reserved and must be zero until
  a future consensus rule specifies it.
- Done: parsed nonce fields and malformed/external-anchor cases are covered by
  Rust tests and the AI-PoW wire regression guard.

Tests:

- Honest NCMN nonce verifies.
- Mutating the embedded Nockchain commitment rejects.
- Bad magic/version/reserved bytes reject before proof verification.
- Reusing a valid proof/nonce on a different candidate block rejects.

## Medium Severity Findings

### API-01: `sx_bound` is caller-controlled in public verifier APIs

Severity: Medium to High depending on misuse
Status: Confirmed

`composite_verify_pinned_logup_sx` and `composite_verify_pow_pinned_logup_sx` accept `sx_bound` directly. If a downstream verifier passes `false`, it disables the `FOLD_XSTEP == SX_XR` keystone. This must be derived from trusted params inside the production verifier, not supplied by the prover or caller.

Fix:

- Hide `_sx` variants from public exports.
- Production verifier computes `sx_bound` internally and rejects params that would set it false until segmented proving is implemented.

### API-02: Public pinned verifier APIs panic on malformed `program`

Severity: Medium DoS
Status: Confirmed

`program_degree_bits` uses `assert!(h.is_power_of_two())`. `CompositeFullAirPinned::new_with` asserts the program width. Several strip-opening helpers also assert on malformed ranges/siblings.

If these helpers are reachable with attacker-controlled data, malformed inputs can crash a verifier.

Fix:

- Production verifier must never accept a program from the wire.
- Convert public verifier setup helpers to return typed errors instead of panicking.
- Add `catch_unwind` tests only as temporary regression guards; the real fix is typed validation.

### PARAM-01: Production envelope is documented but not enforced in exposed verifier/miner entrypoints

Severity: Medium to High
Status: Implemented at production boundaries; legacy helpers remain explicit

Historical issue: `MatmulParams::validate_prod_envelope()` captured the
consensus/security envelope, but production-looking entrypoints previously used
only `validate()`.

Current status:

- `ai-pow-miner::mining::run` enforces `validate_prod_envelope()` at entry.
- `ai_pow::verifier::verify_ncmn_at_target`,
  `verify_prod_at_target`, and the structured certificate noun verifier enforce
  the production envelope.
- `zk_bridge::prove_and_verify_for_block`,
  `prove_ai_pow_recursive_certificate`, and
  `verify_ai_pow_production_statement` enforce the production envelope.
- `ai-pow-mine`'s recursive certificate builder runs the plain target precheck
  through `verify_ncmn_at_target`, not the structural low-level verifier, before
  it starts recursive proof generation.
- `validate()` and `verify_at_target` remain available for tests/local tools
  and are documented as non-production helpers.

Fix:

- Keep source-level guards/tests proving production call sites use the envelope
  and NCMN boundary.
- Do not re-export or advertise structural helpers as consensus APIs.

### WIRE-01: `%ai-pow` consensus wire is a placeholder and reject-all

Severity: Medium integration blocker
Status: Confirmed

`hoon/apps/dumbnet/lib/types.hoon` defines `%ai-pow placeholder=@`, and `do-pow` rejects all `%ai-pow` submissions after activation. This is safe today but must not be confused with a complete integration.

Fix:

- Define the final wire artifact only after SND-01 through SND-05 are fixed.
- Keep activation defaults safe until a real verifier is wired.
- Add an integration test that post-activation `%ai-pow` accepts one honest proof and rejects each tampered field.

### CRYPTO-01: STARK security target and comments need independent sign-off

Severity: Medium
Status: Requires review

`CircuitConfig::PROD` currently uses `log_blowup = 4`, `num_queries = 15`, `pow_bits = 1`, with comments justifying a roughly 60-bit time-bounded threat model. Some comments elsewhere still describe older values. This may be a product decision, but it should be explicitly signed off before chain activation.

Fix:

- Record the accepted security level in a short protocol decision.
- Make comments consistent with the actual constants.
- Add a test asserting the configured minimum soundness target.

### SERIAL-01: Proof serialization needs a consensus envelope

Severity: Medium
Status: Design gap

`MatmulProof` has an ad hoc binary format with no version byte, no top-level length prefix, and unchecked `usize -> u32` casts in `encode`. `ai-pow-zk` proof examples use `bincode`, but no bounded consensus decoder exists.

Fix:

- Define versioned consensus bytes for the full AI-PoW artifact.
- Include length prefixes and maximum lengths for plain proof, ZK proof, and public inputs.
- Use bounded bincode config or a custom decoder for STARK proofs.
- Reject unknown versions and trailing bytes.

## Recommended Fix Order

1. Keep Hoon `%ai-pow` reject-all until this checklist is complete.
2. Add target-explicit plain verification and update all call sites.
3. Add nonce/`pow_key` binding to the ZK statement.
4. Design the final consensus artifact and verifier-only Rust API.
5. Hide or rename all dev/unsafe ZK APIs.
6. Enforce production params at production boundaries.
7. Make decoding parameter-aware and bounded.
8. Parse and enforce the NCMN nonce anchor in the production verifier.
9. Replace full verifier noise expansion with on-demand derivation.
10. Fix or remove noncanonical `place_matrix_hash`.
11. Add end-to-end tamper tests for every trusted field.

## Minimum Test Matrix Before Activation

Plain verifier:

- Reject wrong target.
- Reject wrong nonce.
- Reject NCMN nonce whose embedded block commitment does not match the candidate block.
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

The Hoon `%ai-pow` branch currently rejects all submissions, and the `%ai-pow` payload is a placeholder. That means the critical Rust issues above are not currently exploitable through the main consensus path. They become exploitable as soon as a real `%ai-pow` verifier is wired unless the production verifier API is fixed first.
