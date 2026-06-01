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
- The production recursive-certificate statement precheck re-derives
  `JOB_KEY`, `COMMITMENT_HASH`, `HASH_A`, `HASH_B`, trace height, and jackpot
  target satisfaction from verifier-supplied block data before recursive
  certificate verification is allowed to trust the persisted metadata.
- The structured certificate noun decoder exposes
  `precheck_ai_pow_certificate_statement`, so the Rust/Hoon boundary can run
  those same nonce, target, params, and public-input binding checks immediately
  after bounded noun decoding.
- Hoon consensus remains fail-closed for `%ai-pow`: the kernel does not emit
  `%mine-ai`, does not persist `[%ai-pow cert]`, and rejects typed AI
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
4. The recursive proof must prove the matmul over those nonce-derived noised
   matrices for the verifier-derived `found_idx`.
5. The final jackpot hash must be derived from that same nonce-bound matmul
   result and checked against the target.

Changing the nonce must invalidate the old matmul work. It is acceptable to
cache input validation and raw matrix bytes. It is not acceptable to cache
nonce-independent noised matrices, tile states, or `M` values and treat many
nonce hashes as many PoW attempts.

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

## Required Design Decision

There are two distinct "attempt accounting" questions:

1. Nonce grinding over one matmul result:
   A single tile result `M[i,j]` must not be reusable across many nonces. This
   audit treats this as mandatory and currently broken.

2. Tile scanning within one nonce:
   The current code searches all tiles for one nonce and returns the first
   passing `found_idx`. If the intended rule is literally "one whole matrix
   multiplication yields exactly one jackpot digest", then tile scanning is
   also too permissive and the verifier must derive a single tile from the
   nonce or block data. If the intended Pearl rule is "one tile matmul is one
   work unit", then scanning many tiles is acceptable only if difficulty and
   reward accounting explicitly price the number of tile work units.

The immediate fix below closes nonce grinding while preserving the current
`found_idx`/tile-search model. A separate protocol decision should explicitly
settle whether one full matrix multiplication may expose many tile attempts.

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
jackpot_key   = jackpot_key_for_attempt(s_A)
hash[i]       = BLAKE3(M[i,j], key=jackpot_key)
```

This intentionally minimizes work reuse between attempts. Cache-friendly
derivations are a vulnerability for this protocol goal, not a desired property:
if a miner can carry keyed commitments, noised matrices, tile states, or
jackpot inputs across nonces, the implementation is too close to the current
bug. The recommended fix is to put the nonce in `sigma_attempt` and derive
`kappa` from that.

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
- `crates/ai-pow/src/zk_bridge.rs`: prover entrypoints reject a context used
  with a different nonce, verifier entrypoints derive public inputs from the
  nonce-bound attempt state, and `verify_ai_pow_production_statement` provides
  the same binding checks for persisted recursive-certificate metadata.
- `crates/ai-pow-miner/src/certificate_noun.rs`: decoded Hoon-compatible
  certificate nouns can be prechecked against trusted block state via
  `precheck_ai_pow_certificate_statement` before recursive proof verification
  consumes the miner-controlled proof tree.
- `crates/ai-pow-miner/src/bin/ai_pow_mine.rs`: recursive certificate
  construction rebuilds the context for the winning nonce and refuses to prove
  unless the plain target check succeeds first.

Regression coverage now includes:

- different nonces re-key `kappa`, `H_A`, `H_B`, chunk commitments, `s_A`,
  `s_B`, and at least one tile state before final hashing;
- stale contexts fail if submitted with a different nonce;
- proofs fail under nonce substitution;
- `mine_block` is byte-equivalent to `mine` for the same nonce but rebuilds
  nonce-bound work for each nonce;
- ZK prover entrypoints reject nonce-substituted contexts before proving;
- production recursive-certificate statement metadata rejects wrong nonce,
  wrong public inputs, and jackpots above target before recursive verification.
- decoded structured certificate nouns reject wrong nonce, zero target, and
  parameter mismatch at the statement precheck boundary.

Tile scanning remains the Pearl per-tile block-opening model: all checked tile
hashes are generated from nonce-bound matmul work and the target is
shape-weighted per Pearl Section 4.5. It is not final-hash-only grinding over a
cached matmul result. If Nockchain wants a stricter rule of exactly one digest
per whole matrix product, that is a separate protocol change requiring a
verifier-derived single tile or aggregate digest.

## Concrete Implementation Plan

### Phase 0: Stop Treating Current Path As Production-Sound

1. Keep the Hoon verifier path reject/deferred until this issue is fixed.
2. Do not activate AI-PoW block acceptance on mainnet with the current
   nonce-independent matmul statement.
3. Leave audit comments near `BlockContext`, `pow_key_for_nonce`, and the miner
   loop so future readers do not mistake current behavior for the desired
   invariant.

### Phase 1: Refactor Plain Attempt Context

1. Split `BlockContext` into two concepts:
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

4. Delete or rewrite `mine_block` so every nonce builds a fresh
   `AttemptContext`. If a helper keeps the name, its docs must state that it
   recomputes nonce-derived matmul work per nonce.
5. Update `ai-pow-miner::mining::run_inner` so every extranonce performs a
   fresh nonce-bound matmul attempt. `extranonces_tried` can then legitimately
   mean `matmul_attempts_tried`.

### Phase 2: Update Plain Verification

1. Replace the current commitment helper with a version that consumes the full
   Pearl attempt state:

```rust
attempt_state(block_commitment, nonce) -> Vec<u8>
commitment_key_for_attempt(attempt_state, params_tag) -> [u8; 32]
jackpot_key_for_attempt(s_a) -> [u8; 32]
```

2. In `verify_at_target`, derive `attempt_state = block_state(block_commitment,
   nonce)`, then derive `kappa`, `H_A`, `H_B`, `s_B`, and `s_A` under that
   attempt state.
3. Recompute opened tile values with `VerifierNoise::new(s_A, s_B, params)`.
4. Hash the recomputed tile state with `jackpot_key_for_attempt`.
5. Reject old proofs by changing `params_tag` domain/version or adding a proof
   format version bump so legacy cheap-retry proofs cannot verify under the new
   semantics.

### Phase 3: Update ZK Statement

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
5. Ensure the recursive certificate path only accepts the updated Layer-0
   public input layout and proof version.

### Phase 4: Update Hoon/Noun Statement

1. No new large proof artifact is required; the recursive proof remains the
   only canonical artifact.
2. The Rust noun boundary now exposes
   `precheck_ai_pow_certificate_statement` for the decoded
   `ai-pow-certificate`; the Hoon/Rust verifier path must call it with the
   trusted block commitment, block nonce, params, and target before or alongside
   recursive certificate verification.
3. If proof or params versions change, add a version check in the
   `ai-pow-certificate` decoder and reject version-1 certificates whose public
   inputs correspond to the old nonce-independent matmul statement.

### Phase 5: Regression Tests

Add tests that fail on the current implementation and pass after the repair:

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
   - This likely already fails today through `pow_key`, but keep it as a
     baseline.

4. Matmul nonce substitution:
   - Construct a proof using nonce A's noised matmul but nonce B's jackpot key.
   - Verifier must reject.
   - This is the decisive test that currently would not be expressible through
     public APIs without an adversarial seam; add such a seam under `#[cfg(test)]`
     if needed.

5. ZK public input test:
   - For two nonces, prove the same `found_idx`.
   - Assert `JOB_KEY`, `HASH_A`, `HASH_B`, `COMMITMENT_HASH`, `s_A`, `s_B`,
     noised rows, and `HASH_JACKPOT` differ.

6. Recursive certificate test:
   - Recursive certificate for nonce A must fail verifier reconstruction for
     nonce B.
   - Recursive certificate built with base seeds and nonce-derived jackpot key
     must fail under the new verifier.

7. Miner accounting test:
   - Instrument the miner or attempt builder in tests and assert each
     extranonce increments a matmul-attempt counter exactly once.
   - Rename stats from `extranonces_tried` to `attempts_tried` or add a
     separate `matmul_attempts_tried`.

8. Difficulty calibration test:
   - Measure expected success rate over many small test attempts after the
     change.
   - Confirm the probability model matches the intended "one attempt" unit.

### Phase 6: Benchmarks

Re-benchmark after the semantic fix:

1. Plain per-attempt latency with fresh nonce-derived matmul.
2. Recursive certificate generation latency after a successful attempt.
3. End-to-end miner rate reported as matmul attempts per second.
4. Any allowed non-work reuse:
   - input shape/range validation only;
   - immutable references to input matrix bytes.

Do not cache keyed commitments, `s_A`/`s_B`, low-rank noise, noised matrix
strips, tile states, jackpot preimages, or any other work whose reuse would
reduce the cost of a new nonce attempt.

Do not report BLAKE3-only nonce rate as mining rate.

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

## Open Protocol Question

The repair above stops nonce grinding over one cached matmul result. It does
not decide whether one full matrix product may expose multiple tile jackpot
checks through `found_idx`.

If the rule is "one tile matmul equals one attempt", the current `found_idx`
model can remain, but difficulty and metrics must price tile work explicitly.

If the rule is "one full AI puzzle matmul equals one attempt", then the verifier
must derive a single tile or aggregate digest from the nonce/block data and
reject arbitrary `found_idx` search. That is a larger consensus change and
should be decided before final difficulty calibration.
