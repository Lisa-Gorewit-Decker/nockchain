# AI-PoW One-Matmul-One-Attempt Soundness Audit

Date: 2026-05-31
Status: Critical audit finding, repair plan, and implementation tracking

## Executive Summary

The intended production invariant is:

> A miner must not be able to change a nonce and get a fresh proof-of-work trial
> unless that nonce also forces fresh AI matmul work.

The pre-fix `ai-pow` and `ai-pow-miner` implementation did not satisfy that
invariant.

Before the fix, `BlockContext::build` computed `kappa`, `H_A`, `H_B`, `s_B`, `s_A`,
the noise factors, noised matrices, and every per-tile `M` state without the
nonce. Then `mine_inner` derives `pow_key_for_nonce(s_A, nonce)` and re-hashes
the cached `M` states. The production miner builds one `BlockContext` and loops
over extranonces. A malicious miner can therefore perform one expensive matmul
precomputation and then run many cheap nonce-derived BLAKE3 trials against the
same matmul outputs.

The pre-fix recursive ZKP path did not repair this. It proved the selected tile's
matmul and binds `HASH_JACKPOT = BLAKE3(M, key=pow_key_for_nonce(s_A, nonce))`,
but the noised matrices and matmul inputs still come from nonce-independent
`ctx.s_A` and `ctx.s_B`. The proof certifies "this cached matmul result was
hashed with this nonce-derived key", not "this nonce forced this matmul work".

This is a critical proof-of-work soundness issue, not just a misleading
comment.

Implementation status:

- `BlockContext::build` now takes `(block_commitment, nonce, A, B, params)` and
  derives `kappa` from `block_state(block_commitment, nonce) || params_tag`.
- `mine_block` and `ai-pow-miner` rebuild a fresh nonce-bound context for each
  nonce/extranonce.
- `mine_with_context_at_target` and ZK bridge prover entrypoints reject a
  context supplied with a different nonce.
- Plain and ZK verifiers derive `kappa`, `s_B`, and `s_A` from the same
  nonce-bound attempt state.
- The production miner and verifier now derive exactly one eligible jackpot
  tile from the nonce-bound attempt state, parameter tag, and nonce-bound
  `s_A` seed. `found_idx` is no longer miner-selected search space, and the
  eligible tile is not knowable from `(block, nonce, params)` alone.
- The production recursive-certificate statement precheck re-derives
  `JOB_KEY`, `COMMITMENT_HASH`, `HASH_A`, `HASH_B`, trace height, and jackpot
  target satisfaction from verifier-supplied block data before recursive
  certificate verification is allowed to trust the persisted metadata.
- The structured certificate noun decoder exposes
  `precheck_ai_pow_ncmn_certificate_statement`, so the Rust/Hoon boundary can
  check the NCMN nonce anchor and run those same nonce, target, params, and
  public-input binding checks immediately after bounded noun decoding.
- Hoon consensus remains fail-closed for `%ai-pow`: the kernel does not emit
  `%mine-ai`, does not persist `[%ai-pow nonce cert]`, and rejects typed AI
  certificates until recursive certificate verification is wired.
- Release regression tests now assert that changing the nonce changes `kappa`,
  `H_A`, `H_B`, chunk commitments, `s_A`, `s_B`, and matmul-derived tile
  states before final hashing.
- Re-audit searches over active source, tests, benches, and README found no
  remaining code path or live documentation that presents final-hash-only nonce
  retrying as a valid production mode.

## Security Property

No `%ai-pow` block is currently consensus-admissible: the Hoon/kernel path is
fail-closed until recursive certificate verification is wired. Once that
verifier is enabled, every consensus-admissible AI-PoW block must satisfy:

1. The trusted block data includes a candidate block commitment, AI parameters,
   matrix commitments, a nonce, a target, and a claimed `found_idx`.
2. The verifier derives an attempt-specific transcript from that trusted data.
3. The transcript used to generate the noised matrices must include the nonce,
   directly or through an attempt seed derived from the nonce.
4. The verifier derives the single eligible jackpot tile from the nonce-bound
   attempt state after recomputing `s_A` from the matrix commitments. The
   submitted `found_idx` must match that tile.
5. The recursive proof must prove the matmul over those nonce-derived noised
   matrices for that verifier-derived `found_idx`.
6. The final jackpot hash must be derived from that same nonce-bound matmul
   result and checked against the target.

Changing the nonce must invalidate the old matmul work. Consensus can tolerate
reuse of raw matrix bytes, shape constants, and validation results only because
those are not attempt work; this reuse is not a protocol goal. It is not
acceptable to cache nonce-independent noised matrices, tile states, or `M`
values and treat many nonce hashes as many PoW attempts.

This matches the Pearl whitepaper's intended dependency chain: the noise seeds
are derived from commitments to `A`, `B`, mining configuration, and blockchain
state `sigma`, and the noisy matmul is the work whose trace is certified. For
Nockchain, the NCMN nonce is part of the blockchain attempt state. Therefore
changing the nonce must change the commitment key, matrix commitments, noise
seeds, noised matrices, matmul tile states, and jackpot preimages. Any design
that lets a miner keep the same noised matmul and merely resample the nonce hash
violates that Pearl-style lottery model.

## Pre-Fix Data Flow

### Plain Prover

Relevant files:

- `crates/ai-pow/src/prover.rs`
- `crates/ai-pow/src/fiat_shamir.rs`
- `crates/ai-pow/src/verifier.rs`
- `crates/ai-pow-miner/src/mining.rs`

Pre-fix derivation:

```text
tag     = params_tag(params)
kappa   = commitment_key(block_commitment, tag)
H_A     = MerkleRoot(row_leaf_hash(A_i, kappa))
H_B     = MerkleRoot(col_leaf_hash(B_j, kappa))
s_B     = noise_seed_b(kappa, H_B)
s_A     = noise_seed_a(s_B, H_A)
noise   = BlockNoise::expand(s_A, s_B, params)
M[i,j]  = compute_tile(A + E, B + F, i, j)

pow_key = pow_key_for_nonce(s_A, nonce)
leaf[i] = BLAKE3(M[i,j], key=pow_key)
```

The nonce only entered `block_state` for spot-check challenge derivation and
`pow_key_for_nonce` for final tile hashing. It does not enter `kappa`, `H_A`,
`H_B`, `s_B`, `s_A`, `BlockNoise`, `Matrices`, or `M`.

This was confirmed by:

- `BlockContext::build(block_commitment, a, b, params)` had no nonce argument.
- `BlockContext` stores `m_states: Vec<TileState>`.
- `mine_inner(ctx, block_commitment, nonce, target, opts)` hashes `ctx.m_states`
  with `pow_key_for_nonce(&ctx.s_a, nonce)`.
- `mine_block` built one `BlockContext` and looped over nonces.
- `ai-pow-miner::mining::run_inner` built one `BlockContext` and looped over
  `extranonce`.

### Plain Verifier

The pre-fix verifier mirrored the same bug-compatible statement:

```text
kappa   = commitment_key(block_commitment, tag)
s_B     = noise_seed_b(kappa, proof.H_B)
s_A     = noise_seed_a(s_B, proof.H_A)
pow_key = pow_key_for_nonce(s_A, nonce)
noise   = VerifierNoise::new(s_A, s_B, params)
M       = compute_tile_from_slices(A + E, B + F)
hash    = BLAKE3(M, key=pow_key)
```

Because this verifier explicitly accepted nonce-independent noise and
nonce-dependent final hashing, it accepted the cheap-retry strategy.

### Miner Loop

Pre-fix `crates/ai-pow-miner/src/mining.rs` made the exploit the production
behavior:

```text
ctx = BlockContext::build(job.puzzle_id, job.a, job.b, job.params)
loop extranonce:
  nonce = build_ncmn_nonce(...)
  result = mine_with_context_at_target(&ctx, job.puzzle_id, &nonce, target, ...)
```

The stats field `extranonces_tried` was therefore not a count of fresh matmul
attempts. It was a count of cheap final-hash retry attempts over one cached
matmul context.

### ZK And Recursive Certificate

Relevant files:

- `crates/ai-pow/src/zk_bridge.rs`
- `crates/ai-pow-zk/src/composite_trace.rs`
- `crates/ai-pow-zk/src/composite_public.rs`
- `crates/ai-pow-zk/src/recursion.rs`
- `crates/ai-pow-miner/src/certificate_noun.rs`

The pre-fix ZK bridge did this:

```text
pow_key = pow_key_for_nonce(ctx.s_A, nonce)
trace pins JOB_KEY = ctx.kappa
trace pins COMMITMENT_HASH = pow_key
noise = BlockNoise::expand(ctx.s_A, ctx.s_B, params)
M = in-circuit matmul over A + E(ctx.s_A), B + F(ctx.s_B)
HASH_JACKPOT = BLAKE3(M, key=pow_key)
```

This proved that the selected `M` was honestly computed from the committed
matrices and hashed under the nonce-derived jackpot key. It does not prove that
the nonce caused the matmul work, because `ctx.s_A`, `ctx.s_B`, and the noised
matrix values are the same for every nonce under the same block/matrix context.

The pre-fix recursive certificate only recursively verified this Layer-0
statement. It could not strengthen a statement that did not include
nonce-derived matmul work.

## Pearl Whitepaper Cross-Check

Source: `Pearl_Whitepaper.pdf`, Sections 4.2 through 4.6 and 7.

Pearl's intended derivation is straightforward:

```text
kappa = BLAKE3(sigma || mu)
H_A   = BLAKE3(flatten(A), key=kappa)
H_B   = BLAKE3(flatten(B^T), key=kappa)
s_B   = BLAKE3(kappa || H_B)
s_A   = BLAKE3(s_B || H_A)
E     = NoiseGeneration(key=s_A)
F     = NoiseGeneration(key=s_B)
M     = TiledMatMul(A + E, B + F)
hash  = BLAKE3(M, key=s_A)
```

The whitepaper says the commitment hash takes `A`, `B`, miner config `mu`, and
blockchain state `sigma`, and that the two noise seeds depend on all of those
inputs. Algorithm 2 then derives `kappa` from `sigma || mu`, derives `H_A` and
`H_B` as keyed matrix hashes under `kappa`, and derives `s_A`/`s_B` from
`kappa`, `H_A`, and `H_B`. Algorithm 1 generates the noise from those seeds
before the tiled matmul.

Pearl also removes the Bitcoin-style nonce field from the header and replaces
it with a variable-size block certificate. For Nockchain, which currently has
an explicit NCMN nonce/extranonce mechanism, the faithful translation is:

```text
sigma_attempt = encode(block_commitment, nonce)
kappa         = BLAKE3(sigma_attempt || params_tag)
```

In other words, if the nonce is the miner-controlled attempt variable, it must
be part of Pearl's `sigma` before the matrix commitments and noise seeds are
derived. Changing the nonce then changes `kappa`, `H_A`, `H_B`, `s_A`, `s_B`,
the low-rank noise matrices, and the tile states `M`.

Pearl does not describe a construction where miners compute nonce-independent
noise once and then obtain many independent attempts by changing only a final
hash key. The pre-fix `pow_key_for_nonce(s_A, nonce)` path was a Nockchain
extension, and using it as the only nonce binding was exactly the cheap-retry
bug. The current implementation keeps the final key but derives `s_A` from the
nonce-bound attempt transcript first.

## Attack

Assume a miner has fixed `(block_commitment, params, A, B)`.

Pre-fix honest-looking but unsound strategy:

1. Build `BlockContext` once.
2. Compute all noised matrix tile states `M[i,j]` once.
3. For nonce `n = 0, 1, 2, ...`, derive `pow_key_for_nonce(s_A, n)`.
4. Hash the cached `M[i,j]` values under that key.
5. When one hash clears the target, generate the recursive certificate for that
   nonce and `found_idx`.

The pre-fix verifier accepted because it also derived the final hash key from
the nonce while leaving the matmul noise nonce-independent.

Impact:

- The miner receives many independent final-hash trials for one matmul.
- The cost per additional nonce is roughly BLAKE3 over cached tile states plus
  Merkle recomputation, not AI matmul.
- Difficulty no longer prices the intended useful work.
- The recursive ZKP does not prevent this; it proves the wrong statement for
  the desired work accounting.

## Scope Of Pre-Fix Violation

### Definite Violations

- `crates/ai-pow/src/prover.rs`: `BlockContext` was nonce-independent and stored
  reusable `m_states`.
- `crates/ai-pow/src/fiat_shamir.rs`: `pow_key_for_nonce` was the only current
  nonce binding for the jackpot hash path.
- `crates/ai-pow/src/verifier.rs`: verifier derived the same nonce-independent
  noise and accepted the same cheap-retry statement.
- `crates/ai-pow-miner/src/mining.rs`: production miner looped extranonces over
  one cached `BlockContext`.
- `crates/ai-pow/src/zk_bridge.rs`: ZK trace bound `COMMITMENT_HASH` to the
  nonce-derived final hash key, but used nonce-independent `ctx.s_A`/`ctx.s_B`
  for noised matmul.
- `crates/ai-pow-miner/src/bin/ai_pow_mine.rs`: production certificate builder
  correctly waits for the plain target hit before proving, but that target hit
  may have been found via a cheap nonce rehash over cached matmul output.

### Not A Fix

- Checking `verify_at_target` before recursive proving is necessary, but it
  verifies the current unsound statement.
- Making recursive proofs canonical is necessary, but the recursive proof only
  certifies the Layer-0 statement it is given.
- Serializing only the recursive certificate into Hoon is necessary, but it
  does not affect the mathematical statement.
- Binding `found_idx` into the recursive certificate is necessary, but it only
  selects which tile was proved; it does not make the tile computation
  nonce-dependent.

## Attempt Accounting Rule

The production accounting rule is stricter than a cache-friendly per-tile scan:

- one nonce-bound full matmul attempt gets exactly one eligible jackpot tile;
- the eligible tile is derived by the verifier from
  `block_state(block_commitment, nonce)`, `params_tag`, and `s_A`;
- because `s_A = H(noise_seed_b(kappa, H_B), H_A)`, the tile cannot be known
  until the nonce-bound matrix commitments are fixed;
- a miner must not scan all tile hashes from one full matmul and submit the
  best or first passing tile;
- a tile result `M[i,j]` must not be re-keyed or rehashed across many nonces;
- the submitted `found_idx` is only verifier-checkable metadata and must equal
  the derived jackpot tile.

This avoids two forms of work reuse:

- nonce grinding over one cached noised matmul, where only the final hash key
  changes;
- tile grinding over one cached full matmul, where one expensive attempt yields
  `params.num_tiles()` cheap jackpot trials.

The code now has explicit regression guards that `seek_best` is a no-op in
production mining and that the verifier rejects an in-range substituted
`found_idx` before accepting a Merkle path or target check. The target remains
checked against the eligible tile's digest, but the grid size does not create
additional miner-selected lottery tickets inside one nonce-bound attempt.

## Recommended Repair

Use the Pearl-faithful binding: the nonce is part of the attempt `sigma` before
the commitment hash, matrix commitments, noise seeds, and noised matmul are
computed.

Recommended production design:

```text
tag           = params_tag(params)
attempt_state = block_state(block_commitment, nonce)
kappa         = commitment_key(attempt_state, tag)
H_A           = MerkleRoot(row_leaf_hash(A_i, kappa))
H_B           = MerkleRoot(col_leaf_hash(B_j, kappa))
s_B           = noise_seed_b(kappa, H_B)
s_A           = noise_seed_a(s_B, H_A)
noise         = BlockNoise::expand(s_A, s_B, params)
M[i,j]        = compute_tile(A + E(s_A), B + F(s_B), i, j)
pow_key       = pow_key_for_nonce(s_A, nonce)
hash[i]       = BLAKE3(M[i,j], key=pow_key)
```

This intentionally minimizes work reuse between attempts. Cache-friendly
attempt derivations are a vulnerability for this protocol goal, not a desired
property: if a miner can carry keyed commitments, noised matrices, tile states,
jackpot inputs, Layer-0 witness data, or recursive proof inputs across nonces,
the implementation is too close to the current bug. The recommended fix is to
put the nonce in `sigma_attempt` and derive `kappa` from that. Raw matrix bytes
may be read repeatedly, but any consensus-significant value derived under an
attempt transcript must be rebuilt for the new attempt.

The production design is acceptable only if:

- `s_A` and `s_B` are verifier-derived from trusted block data and nonce.
- The ZK canonical program receives the nonce-bound seeds.
- The in-circuit noised matrix values are generated from the nonce-bound seeds.
- The final `HASH_JACKPOT` key is derived from the same nonce-bound transcript.
- The old API path that rehashes cached `M` states across nonce values is
  removed or made test-only and clearly non-consensus.

## Implemented Resolution

Current code follows the recommended production design above:

- `crates/ai-pow/src/prover.rs`: `BlockContext` is now a nonce-bound attempt
  context. It stores `block_commitment`, `nonce`, and `attempt_state`; `kappa`,
  commitments, seeds, noise, noised matrices, and `m_states` are all built from
  that attempt state.
- `crates/ai-pow/src/verifier.rs`: `verify_at_target` derives `kappa` from
  `block_state(block_commitment, nonce)` before deriving `s_B`, `s_A`, and
  verifier noise.
- `crates/ai-pow-miner/src/mining.rs`: the extranonce loop builds a fresh
  `BlockContext` for each NCMN nonce before checking the target.
- `crates/ai-pow-miner/src/lib.rs`: miner telemetry now reports
  `matmul_attempts_tried` and `matmul_attempt_rate_per_sec`, not a
  BLAKE3-style hash rate or cheap extranonce rate.
- `crates/ai-pow/src/zk_bridge.rs`: prover entrypoints reject a context used
  with a different nonce, verifier entrypoints derive public inputs from the
  nonce-bound attempt state, and `verify_ai_pow_production_statement` provides
  the same binding checks for persisted recursive-certificate metadata.
- `crates/ai-pow-miner/src/certificate_noun.rs`: decoded Hoon-compatible
  certificate nouns can be reconstructed into
  `AiPowProductionCertificate` and verified via
  `verify_decoded_ai_pow_ncmn_certificate`, which checks the NCMN nonce anchor
  and runs the trusted statement precheck before recursive proof verification
  consumes the miner-controlled proof tree.
- `crates/ai-pow-miner/src/bin/ai_pow_mine.rs`: recursive certificate
  construction rebuilds the context for the winning nonce and refuses to prove
  unless the production NCMN verifier confirms that the plain target check
  succeeds first against the trusted candidate Nockchain commitment carried by
  the mining result. This precheck enforces the production parameter envelope,
  NCMN nonce shape, candidate anchor, absent external commitment, and target
  satisfaction before recursive proof generation begins.

Regression coverage now includes:

- different nonces re-key `kappa`, `H_A`, `H_B`, chunk commitments, `s_A`,
  `s_B`, and at least one tile state before final hashing;
- stale contexts fail if submitted with a different nonce;
- proofs fail under nonce substitution;
- `mine_block` is byte-equivalent to `mine` for the same nonce but rebuilds
  nonce-bound work for each nonce;
- ZK prover entrypoints reject nonce-substituted contexts before proving;
- the standalone miner's recursive certificate builder rejects bad targets and
  non-canonical, wrong-anchor, or externally anchored NCMN nonces before
  recursive proof generation;
- production recursive-certificate statement metadata rejects wrong nonce,
  wrong public inputs, and jackpots above target before recursive verification;
- the recursive production certificate itself binds the Layer-0 public-input
  vector as outer STARK public values, so swapping verifier-derived statement
  inputs rejects at recursive certificate verification.
- real recursive certificate nouns roundtrip through structured proof-node
  serialization, jam/cue, bounded decode, reconstruction into
  `AiPowProductionCertificate`, canonical re-serialization, and recursive
  verification.
- decoded structured certificate nouns reject wrong nonce, zero target, and
  parameter mismatch at the statement precheck boundary.

Tile scanning is no longer a production optimization. The only jackpot tile
hash checked for a nonce-bound full matmul attempt is the verifier-derived
attempt tile, sampled after `s_A` is fixed. The target is still shape-weighted
per Pearl Section 4.5 (`r * t_m * t_n`), but grid size does not silently grant
extra cheap jackpot trials inside one cached full matmul.

## Latest Recursive-Certificate Re-Audit

The recursive certificate has two distinct verification layers:

1. Outer recursive STARK verification: prove that the Plonky3-recursion L1
   verifier circuit accepted its embedded Layer-0 composite proof.
2. Statement binding: prove that the embedded Layer-0 public inputs are exactly
   the verifier-derived AI-PoW statement for this block commitment, nonce,
   target, matrix commitments, params, and `found_idx`.

`crates/ai-pow-zk/src/recursion.rs` now exposes the production verifier API:

- `verify_production_certificate(cert, public_inputs)` accepts only the
  canonical recursive certificate and a verifier-derived
  `CompositePublicInputs`.
- `verify_production_certificate_with_public_values(cert, public_values)` is the
  lower-level equivalent for callers that already hold the exact Layer-0 public
  input vector. It rejects empty statement vectors on the production path.
- `verify_production_certificate_outer` is deprecated outer-only diagnostic
  code for old unbound proof objects. Canonical bound certificates reject on
  that helper and direct callers to the full verifier.

The implemented binding is:

1. `build_composite_l1_verifier_circuit` records the Layer-0 AI-PoW public
   input vector passed to the L1 verifier circuit.
2. `prove_composite_l1_outer_cert` sets
   `TablePacking::with_public_binding_lanes(public_values.len())`.
3. The circuit-prover `PublicAir` exposes those leading lanes as STARK public
   values and constrains the first row to equal the supplied values.
4. `BatchStarkProof` serializes `public_binding_lanes` and validates that it
   agrees with `table_packing`.
5. `verify_production_certificate` recomputes the expected production table
   packing from the caller's public inputs, flattens each Goldilocks value into
   the D=2 outer field representation `[value, 0]`, and calls
   `verify_all_tables_with_public_values`.

This closes the metadata-swap gap at the Rust recursive verifier boundary:
reusing a valid recursive certificate with a different verifier-derived
Layer-0 public-input vector fails recursive certificate verification. Hoon
consensus still remains fail-closed for `%ai-pow` until the jet/wiring decodes
the structured noun, derives the trusted statement from block data, and calls
this full Rust verifier.

Do not accept: verifying only the outer certificate and trusting adjacent block
metadata. That permits metadata swapping or replay of a valid recursive
certificate for a different statement.

The minimal-reuse mining rule remains stronger than "do not cache final hashes".
Fresh attempts must rebuild every nonce-dependent work product:

- `kappa`;
- matrix commitments under `kappa`;
- `s_B` and `s_A`;
- low-rank noise;
- noised matrix strips;
- tile states and jackpot preimages;
- Layer-0 witness/proof inputs for the selected winning attempt.

Allowed reuse is limited to nonce-independent non-work inputs: immutable input
matrix bytes, shape constants, chain-pinned params, and read-only model
metadata. Even those caches should be treated as an engineering convenience
rather than a protocol objective. They must never contain transcript-derived
commitments, seeds, noised values, tile outputs, jackpot hashes, or proof
witnesses. Cache-friendly reuse between attempts is a consensus vulnerability,
not an optimization target.

## Concrete Implementation Plan

### Phase 0: Stop Treating Unbound Paths As Production-Sound

Status: implemented for current Hoon consensus; still required operationally
until the verifier jet is wired.

1. Keep the Hoon verifier path reject/deferred until the full recursive
   certificate verifier is callable from consensus.
2. Do not activate AI-PoW block acceptance on mainnet through any legacy
   nonce-independent matmul statement or outer-only recursive verifier.
3. Leave audit comments near `BlockContext`, `pow_key_for_nonce`,
   `attempt_tile_index`, and the miner loop so future readers do not mistake
   cache-friendly behavior for the desired invariant.

### Phase 1: Refactor Plain Attempt Context

Status: functionally implemented with `BlockContext` now representing one
nonce-bound attempt. A future cleanup may still split raw immutable inputs from
attempt-local state, but the soundness boundary no longer exposes nonce-free
`m_states`.

1. Split `BlockContext` into two concepts if the API is cleaned up further:
   - `BlockInputs`: immutable references to `A`, `B`, `params`, and at most
     trivial shape/range validation results. It must not contain keyed
     commitments, seeds, noised matrices, tile states, or jackpot inputs.
   - `AttemptContext`: `attempt_state`, `kappa`, nonce-bound `H_A`/`H_B`,
     nonce-bound seeds, noise, noised matrices, tile states, and openings for
     exactly one nonce.
2. Remove `m_states` from any context type that does not contain a nonce.
3. Replace:

```rust
BlockContext::build(block_commitment, a, b, params)
mine_with_context_at_target(&ctx, block_commitment, nonce, target, opts)
```

with:

```rust
BlockInputs::build(a, b, params)
AttemptContext::build(&inputs, block_commitment, nonce)
mine_attempt_at_target(&attempt, target, opts)
```

4. Done: `mine_block` builds a fresh nonce-bound context per nonce and its docs
   state that it recomputes nonce-derived matmul work per nonce.
5. Done: `ai-pow-miner::mining::run_inner` builds a fresh nonce-bound context
   per extranonce before target checking.

### Phase 2: Update Plain Verification

Status: implemented. The current code keeps `pow_key_for_nonce(s_A, nonce)` as
extra domain separation, but `s_A` is already nonce-bound before noise and
matmul.

1. The commitment helper consumes the full Pearl attempt state:

```rust
attempt_state(block_commitment, nonce) -> Vec<u8>
commitment_key_for_attempt(attempt_state, params_tag) -> [u8; 32]
pow_key_for_nonce(s_a, nonce) -> [u8; 32]
```

2. In `verify_at_target`, derive `attempt_state = block_state(block_commitment,
   nonce)`, then derive `kappa`, `H_A`, `H_B`, `s_B`, and `s_A` under that
   attempt state.
3. Recompute opened tile values with `VerifierNoise::new(s_A, s_B, params)`.
4. Hash the recomputed tile state with `pow_key_for_nonce(s_A, nonce)`.
5. Reject old proofs by deriving verifier noise from the nonce-bound attempt
   state; legacy cheap-retry statements no longer verify under the new
   semantics.

### Phase 3: Update ZK Statement

Status: implemented for the Rust ZK bridge and recursive production
certificate; Hoon consensus still needs the verifier jet/wiring.

1. In `prove_ai_pow_tiled_full`, build the plain/ZK context from
   `(block_commitment, nonce)` and use nonce-bound `kappa`, `H_A`, `H_B`,
   `s_A`, and `s_B` for:
   - noise strips passed into `place_matrix_strip_opening`;
   - `BlockNoise::expand`;
   - `Matrices::build`;
   - `noise_ref::e_value` / `noise_ref::f_value` producer rows;
   - `BlockPublic { s_a, s_b }` used to build the canonical program.
2. Set `JOB_KEY = kappa`, where `kappa` is now nonce-bound because it is
   derived from `attempt_state || params_tag`.
3. Set `COMMITMENT_HASH` to the attempt jackpot key derived from the same
   nonce-bound transcript.
4. In `verify_ai_pow_block`, derive the same attempt seeds from trusted
   `(block_commitment, nonce, params, commitments)` and pass those into
   `verify_ai_pow_tiled_with_statement`.
5. Done: the recursive certificate path accepts the updated Layer-0 public
   input layout through `verify_production_certificate`.

### Phase 4: Update Hoon/Noun Statement

1. No new large proof artifact is required; the recursive proof remains the
   only canonical proof artifact.
2. The Rust noun boundary now exposes `decode_ai_pow_artifact_jam`,
   `decode_ai_pow_artifact_slab`, and `verify_ai_pow_ncmn_artifact_jam` /
   `verify_decoded_ai_pow_ncmn_artifact` for the full persisted/wire artifact
   `[%ai-pow nonce cert]`; the Hoon/Rust verifier path must call this boundary
   with the trusted puzzle id, candidate block commitment, params, and target.
   The jam-byte entrypoint caps attacker input and preflights noun count,
   depth, and atom bytes before cueing.
3. The `%ai-pow` wire carries `[%ai-pow nonce cert]`, where `nonce` is an
   `@uxncmn` atom. This keeps the recursive certificate as the only proof
   artifact while still carrying the nonce commitment parameter needed to prove
   one NCMN nonce equals one fresh matmul attempt.
4. The Rust decoder/precheck rejects statement data that does not match trusted
   block data before recursive proof verification; if proof or params versions
   change again, add an explicit version check in the `ai-pow-certificate`
   decoder.
5. Do not optimize this protocol around cached matrix/noise work across
   nonces. Cache-friendly reuse between attempts is a soundness risk: a miner
   must not be able to keep one expensive matmul result and cheaply grind
   nonce-dependent hashes until the target hits.

### Phase 5: Regression Tests

Status: implemented for the main soundness paths; keep the remaining items as
future hardening/measurement work.

1. Plain attempt seed test:
   - Same `(block, A, B, params)`, different nonce.
   - Assert `kappa`, `H_A`, `H_B`, `s_A`, `s_B`, and at least one opened tile
     `M` differ before final hashing.

2. No cached `M` across nonces source/API guard:
   - No nonce-free context type may expose `m_states`.
   - `mine_block` must call the nonce-bound attempt constructor once per nonce
     or be removed.

3. Proof nonce substitution:
   - A proof generated for nonce A must fail with nonce B.
   - Implemented as a baseline regression.

4. Matmul nonce substitution:
   - Covered by stale-context rejection and verifier recomputation from the
     nonce-bound attempt state; add a dedicated adversarial seam if future API
     changes make mixed-context construction reachable again.

5. ZK public input test:
   - For two nonces, prove the same `found_idx`.
   - Assert `JOB_KEY`, `HASH_A`, `HASH_B`, `COMMITMENT_HASH`, `s_A`, `s_B`,
     noised rows, and `HASH_JACKPOT` differ.

6. Recursive certificate test:
   - Implemented: a recursive certificate rejects when verified with a different
     Layer-0 public-input vector.
   - Keep a follow-up integration test at the noun/jet boundary once Hoon calls
     the Rust verifier.

7. Miner accounting test:
   - Implemented: miner stats expose `matmul_attempts_tried` and
     `matmul_attempt_rate_per_sec`; every increment occurs after building a
     fresh nonce-bound `BlockContext` and running the target check.
   - Future hardening: instrument the attempt builder in tests and assert the
     constructor is invoked exactly once per extranonce tried.

8. Difficulty calibration test:
   - Done: `difficulty_target_is_per_tile_not_per_grid` pins that the Pearl
     target formula itself does not scale with `num_tiles`.
   - Done: `seek_best_does_not_scan_beyond_verifier_derived_attempt_tile` and
     the found-index substitution tests pin that production gets exactly one
     eligible jackpot tile per nonce-bound attempt.
   - Future measurement: sample many small test attempts and confirm the
     empirical success rate matches a single eligible tile digest per full
     matmul attempt.

### Phase 6: Benchmarks

Re-benchmark after the semantic fix:

1. Plain per-attempt latency with fresh nonce-derived matmul.
2. Recursive certificate generation latency after a successful attempt.
3. End-to-end miner rate reported as matmul attempts per second via
   `matmul_attempt_rate_per_sec`.
4. Any allowed non-work reuse:
   - input shape/range validation only;
   - immutable references to input matrix bytes.

Do not cache keyed commitments, `s_A`/`s_B`, low-rank noise, noised matrix
strips, tile states, jackpot preimages, or any other work whose reuse would
reduce the cost of a new nonce attempt.

Do not report BLAKE3-only nonce rate as mining rate. Do not optimize for cache
locality across attempts if the cache would hold anything below
`sigma_attempt` in the derivation tree.

## Acceptance Criteria

The fix is complete only when all of these are true:

1. There is no consensus path where changing only `nonce` changes only the
   final hash key while reusing old noised matmul output.
2. The verifier derives the same nonce-bound matmul seeds from trusted block
   data and rejects old cheap-retry statements.
3. The recursive certificate proves the nonce-bound matmul statement.
4. The production miner performs a fresh nonce-bound matmul attempt before each
   target check.
5. Tests explicitly cover old-proof rejection and nonce-substituted matmul
   rejection.
6. Documentation and comments no longer describe cheap nonce retries as a
   feature.
7. AI-PoW consensus acceptance remains disabled or reject-all until the above
   tests pass.

## Resolved Protocol Question: No Miner-Selected Tile Search

The repair above stops nonce grinding over one cached matmul result and also
settles the `found_idx` interpretation for the current protocol: one
nonce-bound full matmul attempt produces one verifier-derived jackpot tile.
The miner cannot turn a single full matmul into many lottery tickets by scanning
all tile digests. That cache-friendly behavior is a vulnerability because it
discounts wide matrix products by `params.num_tiles()`.

The certificate must prove the verifier-derived tile, and the verifier must
reject any other in-range `found_idx` even if its opening is internally
well-formed. If a future design wants multi-tile search, it must price that
search as separate consensus work or specify a different aggregate digest rule.
It must not reintroduce arbitrary `found_idx` selection over a cached full
matmul.

## Latest Re-Audit: Tile Preselection Boundary

The first derived-tile repair sampled the jackpot tile from
`block_state(block_commitment, nonce)` and `params_tag`. That removed
miner-selected tile submission, but it still revealed the eligible tile before
any nonce-bound matrix commitment or noise seed had been fixed. A custom miner
could therefore specialize all later work around the known tile.

The implementation now samples `attempt_tile_index` from
`block_state(block_commitment, nonce)`, `params_tag`, and `s_A`. Since `s_A`
depends on `kappa`, `H_B`, and `H_A`, changing either the nonce or the matrix
commitment transcript changes the eligible tile before the target check. The
production statement precheck rejects a certificate whose `found_idx` matches
the old commitments but not the submitted commitment transcript.

Residual design note: the current recursive certificate proves the selected
opened tile's in-circuit matmul and hash. It does not prove a full `comm_m`
Merkle tree over every tile state. If consensus requires cryptographic proof of
an entire full-matrix product per attempt, the remaining design work is to add a
full-matrix aggregate/commitment proof or keep AI-PoW block acceptance
fail-closed until that proof exists. The current Hoon path remains fail-closed,
so this residual issue is not consensus-admissible yet.
