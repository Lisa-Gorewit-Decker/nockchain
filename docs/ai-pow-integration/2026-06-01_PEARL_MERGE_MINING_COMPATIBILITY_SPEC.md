# Pearl Merge-Mining Compatibility Specification

Date: 2026-06-01
Status: Proposed compatibility target; initial `ai_pow::pearl_compat`
transcript primitives implemented

## Goal

Bring Nockchain AI-PoW to a Pearl-compatible mode where one miner can perform
one Pearl-style useful-work attempt and use that same underlying proof-of-work
attempt to mine:

- a Pearl block, and
- a Nockchain block.

The ZK proof systems do not need to be identical. In fact, Nockchain should keep
its own proof artifact and verifier stack if that is smaller, easier to verify
from Hoon/Rust, or better aligned with Nockchain consensus. Compatibility is
defined at the PoW attempt layer: both chains must be checking the same public
work instance and jackpot digest, even if they receive different certificates
proving that statement.

The compatibility target is not merely "similar algorithms." It is:

> The same `(A, B, mu, sigma)` work instance must produce the same
> `kappa`, `H_A`, `H_B`, `s_A`, `s_B`, tile state `M`, and jackpot digest
> under Pearl and under Nockchain's merge-mining verifier.

If any Nockchain-only nonce, transcript tag, parameter serialization, tile
selection rule, or jackpot key changes those values, the miner is no longer
merge-mining one work instance; they are solving two related but distinct PoW
puzzles.

Proof-system separation is allowed and desirable. "Same attempt" means the
same transcript bytes, public work statement, tile witness, and jackpot digest;
it does not mean the same proof bytes. Pearl can receive Pearl's own block
certificate, while Nockchain receives a Nockchain-native recursive certificate
that proves the same Pearl-compatible public statement and additionally binds
that statement to a Nockchain block.

## Implementation Progress

Implemented in this branch:

- `ai_pow::pearl_compat` module with:
  - exact Pearl `IncompleteBlockHeader` serialization
    (`version || rev(prev_block) || rev(merkle_root) || timestamp || nbits`,
    76 bytes);
  - exact Pearl `MiningConfiguration` serialization
    (`common_dim || rank || mma_type || rows_pattern || cols_pattern ||
    reserved`, 52 bytes);
  - Pearl `kappa = BLAKE3(sigma || mu)`;
  - Pearl matrix commitments `H_A` / `H_B`;
  - Pearl `s_B` / `s_A` derivation;
  - Pearl jackpot hashing with `s_A` directly, not Nockchain
    `pow_key_for_nonce`;
  - Pearl `PeriodicPattern` serialization, `from_list`, `to_list`,
    `offset_is_valid`, `period`, `size`, bounded partition expansion, and
    malformed-pattern rejection;
  - Pearl 164-byte public proof parameter serialization
    (`MiningConfiguration || H_A || H_B || jackpot_hash || m || n || t_rows
    || t_cols`);
  - Pearl public-parameter sanity checks for rank, common dimension, pattern
    offsets, pattern bounds, worker input size, and production envelope;
  - Pearl pattern-indexed ticket recomputation from the public statement,
    full `A` / `B`, and derived commitments, with selected row/column noise
    generated directly instead of allocating full noised matrices;
  - public commitment and jackpot-hash mismatch rejection for the
    pattern-indexed ticket path;
  - Pearl compact `nbits` target parsing, Pearl's `h * w *
    dot_product_length` difficulty adjustment with 256-bit saturation, and
    little-endian jackpot hash comparison;
  - independent Nockchain target checking over the same jackpot digest;
  - a single composed Rust precheck,
    `verify_pearl_compatible_work`, that re-derives Pearl `sigma`/`mu`,
    enforces Pearl and Nockchain targets over the same public jackpot digest,
    validates matrix shape/range, recomputes `H_A` / `H_B`, recomputes the
    pattern-indexed ticket, and rejects any public statement whose commitment
    or jackpot digest does not match the supplied work;
  - all-legal-tile digest construction for the current square-tile
    `MatmulParams` model.
- Release tests in `crates/ai-pow/tests/pearl_merge_compat.rs` proving:
  - header and mining-config byte layouts;
  - production-style `PeriodicPattern` semantics, including the Pearl default
    row/column examples and malformed-pattern rejection;
  - public proof parameter round-trip, offset rejection, envelope rejection,
    and bounded row/column partition expansion;
  - pattern-indexed ticket recomputation, including equivalence with the
    square-tile path when Pearl patterns are contiguous and noncontiguous
    Pearl row/column patterns when they are not;
  - public commitment tamper and jackpot hash tamper rejection;
  - Pearl target arithmetic edge cases, saturation, and independent
    Nockchain target pass/fail behavior;
  - transcript equivalence to the reference formulas;
  - `sigma` changes alter commitments, seeds, and jackpot digest before the
    final hash check;
  - Pearl-compatible attempts are distinct from the native Nockchain
    nonce-bound selected-tile path;
  - mismatched Pearl `common_dim` / `rank` are rejected before work is
    computed;
  - the composed precheck accepts one shared Pearl-style work attempt for both
    target systems and rejects tampered header bytes, tampered mining config,
    tampered jackpot digest, Nockchain target failures, Pearl target failures,
    and malformed matrix inputs.

Still incomplete:

- the Rust precheck can recompute and target-check the Pearl pattern-indexed
  ticket, but the recursive proof circuit still needs to prove that same
  statement; current proof generation remains Nockchain square-tile oriented;
- Nockchain-native recursive certificate whose public inputs prove the
  Pearl-compatible statement;
- AuxPoW inclusion binding from Pearl `sigma` to a Nockchain block commitment;
- Hoon noun type and Rust verifier jet for the merge-mined artifact;
- activation-gated consensus tests.

## Current Blocking Differences

The current branch is sound for Nockchain's fail-closed single-tile recursive
path, but it is not full Pearl merge-mining compatible yet.

### 1. Nockchain Has a Non-Pearl Attempt Transcript

Current Nockchain uses:

```text
attempt_state = block_state(nockchain_block_commitment, ncmn_nonce)
tag           = params_tag(MatmulParams)
kappa         = BLAKE3(attempt_state || tag)
```

Pearl uses:

```text
kappa = BLAKE3(sigma || mu)
```

where `sigma` is Pearl's blockchain/work state and `mu` is Pearl's mining
configuration serialization.

For merge-mining, Nockchain cannot feed a Nockchain-only NCMN nonce into
`kappa` unless Pearl also feeds the exact same bytes into `sigma`. Otherwise
the matrices, noise, and tile states diverge.

### 2. Nockchain Uses a Non-Pearl Jackpot Key

Current Nockchain uses:

```text
pow_key = derive_key("pow-key", s_A || nonce)
hash    = BLAKE3(M, key=pow_key)
```

Pearl uses:

```text
hash = BLAKE3(M, key=s_A)
```

The Nockchain key was introduced to close nonce-binding issues, and it is safe
for Nockchain's own puzzle because `s_A` is now nonce-bound. It is not
byte-compatible with Pearl. A Pearl-compatible mode must use `s_A` directly as
the jackpot key.

### 3. Nockchain Selects One Verifier-Derived Tile

Current Nockchain derives:

```text
found_idx = attempt_tile_index(block_state, params_tag, s_A, num_tiles)
```

and rejects any other tile.

Pearl's Algorithm 4 treats each computed full-rank tile as a potential opened
block/ticket. A miner can submit a tile whose `BLAKE3(M, key=s_A)` clears the
target, subject to the proof authenticating that tile.

For full Pearl compatibility, Nockchain must not impose a Nockchain-only
verifier-selected tile unless Pearl adopts the same rule. The compatible rule
is to accept the tile position revealed by the shared Pearl-compatible public
witness and verify:

- the tile is a legal full-rank Pearl tile;
- the proof binds that tile position;
- the jackpot digest for that tile satisfies Nockchain's target.

This reintroduces Pearl's per-tile ticket model. The target/work accounting
must be reviewed accordingly.

### 4. Nockchain Proof Artifact Is Structurally Different

Pearl's block certificate is a bounded recursive proof certificate, with block
identity bound by a `pouw_meta` commitment to the public PoUW witness rather
than by certificate bytes.

Nockchain currently intends to persist:

```hoon
[%ai-pow nonce=ai-ncmn cert=ai-pow-certificate]
```

For merge-mining, Nockchain should persist a Nockchain certificate proving a
Pearl-compatible public statement, not a Pearl certificate byte-for-byte. The
Hoon noun can remain structured, and the recursive proof may remain
Nockchain-native. Its public inputs must describe the same Pearl work witness
and jackpot digest that Pearl would verify.

## Compatibility Design

Use an AuxPoW-style design: the PoUW work instance is Pearl-compatible, and the
Nockchain block is bound into the Pearl work state through an auxiliary
commitment.

### High-Level Flow

1. Miner builds a Nockchain candidate block and computes its candidate block
   commitment `nock_block_commitment`.
2. Miner constructs a Pearl block candidate whose transaction tree or other
   Pearl-consensus-valid commitment path includes:

   ```text
   aux_commitment = HASH("nockchain-ai-pow-aux-v1" ||
                         nockchain_chain_id ||
                         nock_block_commitment ||
                         nockchain_target_epoch_or_height ||
                         optional_extra_domain_data)
   ```

3. Pearl's candidate header/work state `sigma` therefore commits, through the
   Pearl block, to the Nockchain candidate.
4. Miner runs the Pearl-compatible PoUW attempt using exact Pearl transcript
   rules:

   ```text
   kappa = BLAKE3(sigma || mu)
   H_A   = BLAKE3(pad(A_row_major), key=kappa)
   H_B   = BLAKE3(pad(B_col_major), key=kappa)
   s_B   = BLAKE3(kappa || H_B)
   s_A   = BLAKE3(s_B || H_A)
   hash  = BLAKE3(M_i_j, key=s_A)
   ```

5. If the tile digest clears Pearl's target, the miner can submit the Pearl
   block with Pearl's expected certificate format.
6. If the same tile digest clears Nockchain's target, the miner can submit the
   Nockchain block with a Nockchain-native certificate proving the same
   Pearl-compatible work statement, plus an
   auxiliary inclusion proof linking `nock_block_commitment` into `sigma`.

This is true merge-mining: the expensive matrix commitment, noise, matmul, and
jackpot search work is shared. ZK certificate generation may be separate per
chain.

## Required Nockchain Wire / Noun Shape

Add a Pearl-compatible AI-PoW artifact variant. Do not overload the current
Nockchain-specific `ai-pow-certificate` silently.

Proposed Hoon-level shape:

```hoon
$:  version=%pearl-merge-v1
    pearl-work=pearl-work-statement
    aux=pearl-nockchain-aux
    cert=nockchain-ai-pow-certificate
==
```

Where:

```hoon
+$  pearl-work-statement
  $:  sigma=pearl-sigma
      mu=pearl-mining-config
      tile-i=@ud
      tile-j=@ud
      commitments=pearl-commitments
      public=pearl-public-witness
      pouw-meta=@uxblake
  ==

+$  pearl-commitments
  $:  h-a=@uxblake
      h-b=@uxblake
  ==

+$  pearl-public-witness
  $:  jackpot-hash=@uxblake
      :: Include every public value needed to identify the Pearl-compatible
      :: PoW statement: shape, tile position, commitment roots, proof-version
      :: domain, and any public-input vector commitments.
  ==

+$  pearl-nockchain-aux
  $:  nock-block-commitment=@uxblake
      nock-target=@uxblake
      pearl-header=pearl-header
      aux-path=pearl-aux-inclusion-proof
  ==
```

The exact `pearl-sigma`, `pearl-mining-config`, `pearl-header`, and
`pouw-meta` encodings must be derived from Pearl's current consensus code, not
from informal reimplementation. The Nockchain certificate format can be
Nockchain-native, but the statement it proves must be byte-for-byte compatible
with Pearl's PoW attempt serialization.

## Verifier Contract

The Nockchain verifier for `pearl-merge-v1` must perform these checks in this
order.

### 1. Bound Decode

- Enforce max jam bytes before cueing.
- Enforce max noun depth, list lengths, atom byte sizes, proof-node counts, and
  certificate byte/field sizes.
- Reject unknown versions, unknown tags, noncanonical atoms, and trailing
  data.

### 2. Auxiliary Block Binding

- Recompute `nock_block_commitment` from the candidate Nockchain block.
- Recompute `aux_commitment`.
- Verify the Pearl auxiliary inclusion proof showing `aux_commitment` is
  committed into the Pearl work state used by `sigma`.
- Reject if the Pearl work state can be reused for a different Nockchain block.

This is the replay-protection step. Without it, a miner could use any Pearl
certificate that clears Nockchain's target to mine an unrelated Nockchain
block.

### 3. Exact Pearl Transcript Re-Derivation

From decoded Pearl fields, rederive:

```text
kappa = BLAKE3(sigma || mu)
s_B   = BLAKE3(kappa || H_B)
s_A   = BLAKE3(s_B || H_A)
```

This must use Pearl's exact serialization for `sigma` and `mu`.

Nockchain must not:

- length-prefix these fields differently;
- hash `MatmulParams` into a Nockchain-only `params_tag`;
- include an NCMN nonce outside Pearl's `sigma`;
- domain-separate the jackpot key differently.

### 4. Nockchain Certificate Verification Over a Pearl-Compatible Statement

Verify the Nockchain recursive certificate against the Pearl-compatible public
statement:

- mining config and matrix shape;
- tile position;
- `H_A`, `H_B`;
- `kappa` / job key;
- `s_A` or `COMMITMENT_HASH`, depending on Pearl's actual public input naming;
- jackpot digest;
- Nockchain proof version and recursion verifier parameters.

The verifier must reject if the certificate proves a Nockchain-local attempt
statement instead of the Pearl-compatible work statement. It does not need to
reject merely because the certificate bytes or recursion stack differ from
Pearl's.

### 5. Nockchain Target Check

Interpret `jackpot_hash` as a little-endian `uint256` and compare it to the
Nockchain AI-PoW target for the candidate block.

Pearl and Nockchain targets may differ. A shared work instance can mine:

- Pearl only if it clears Pearl's target;
- Nockchain only if it clears Nockchain's target;
- both if it clears both.

### 6. Tile Legality and Work Accounting

For Pearl-compatible mode, the tile must satisfy Pearl's legality rules:

- tile position is inside the matrix;
- partial boundary tiles are ineligible unless Pearl consensus explicitly
  permits them;
- rank and tile shape satisfy Pearl's production envelope;
- `k`, `r`, and tile shape match Pearl's verifier constraints.

Nockchain's work accounting must use Pearl's per-tile ticket model. Do not
apply the current one-selected-tile rule in this mode.

## Required Code Updates

### `ai-pow`

- Done for the scalar/current square-tile slice: add a `pearl_compat`
  transcript module with exact Pearl byte
  serialization:
  - `pearl_kappa(sigma_bytes, mu_bytes)`;
  - `pearl_matrix_commitments(A, B, kappa)`;
  - `pearl_noise_seeds(kappa, H_A, H_B)`;
  - `pearl_jackpot_hash(M, s_A)`.
- Keep the existing Nockchain nonce-bound transcript for non-merge mode, but
  do not use it for Pearl merge-mining.
- Done for Rust statement prechecks: add full Pearl `PeriodicPattern`
  semantics, 164-byte public proof parameter parsing, pattern-indexed ticket
  recomputation, Pearl target checking, independent Nockchain target checking,
  and the composed `verify_pearl_compatible_work` API. Still required: teach
  the recursive proof statement/circuit to prove Pearl's general row/column
  pattern-indexed ticket instead of only the current square-tile `MatmulParams`
  model.
- Add byte-equivalence tests against Pearl source fixtures for:
  - `sigma || mu` serialization;
  - `kappa`;
  - `H_A`, `H_B`;
  - `s_A`, `s_B`;
  - jackpot hash using `s_A`;
  - target comparison.

### `ai-pow-zk`

- Add/identify a verifier entrypoint for a Nockchain recursive certificate that
  proves the Pearl-compatible PoW statement.
- Ensure public inputs match Pearl's work statement exactly. In
  Pearl-compatible mode, the public slot currently named `COMMITMENT_HASH` must
  contain `s_A`, not `pow_key_for_nonce(s_A, nonce)`.
- Remove Nockchain-only `attempt_tile_index` from the Pearl-compatible
  statement.
- Support Pearl's tile-shape language or explicitly restrict merge-mining to
  the subset of Pearl configs that Nockchain can prove byte-for-byte.
- Add an end-to-end test that verifies a Nockchain-native certificate over
  Pearl fixture data and checks that Pearl's own transcript would derive the
  same jackpot digest.

### `ai-pow-miner`

- Add a merge-mining job type:

  ```rust
  PearlMergeMiningJob {
      nockchain_candidate,
      pearl_candidate_header_or_template,
      pearl_mu,
      matrices,
      nockchain_target,
      pearl_target,
  }
  ```

- Build the auxiliary commitment into the Pearl block template before mining.
- Run one Pearl-compatible work loop.
- On success, emit:
  - Pearl submission payload for Pearl;
  - Nockchain `%ai-pow` payload with Pearl work statement, aux proof, and
    Nockchain-native recursive certificate.

### Hoon / Consensus

- Add a new wire/certificate variant for Pearl-compatible merge-mined AI-PoW.
- Keep the existing `%ai-pow` path fail-closed until the Rust verifier jet can
  verify:
  - bounded decode;
  - auxiliary inclusion;
  - Pearl transcript;
  - Nockchain recursive certificate over the Pearl-compatible statement;
  - Nockchain target.
- Add activation-gated tests:
  - honest Pearl merge-mined block accepted;
  - same certificate rejected for a different Nockchain block;
  - aux inclusion tamper rejected;
  - Pearl header/sigma tamper rejected;
  - `mu` serialization tamper rejected;
  - jackpot key changed to Nockchain `pow_key` rejected in Pearl mode;
  - selected-tile-only proof rejected unless it is also a valid
    Pearl-compatible tile-ticket statement.

## Compatibility Modes

Nockchain should expose two modes explicitly:

### Native Nockchain AI-PoW

Current design:

- NCMN nonce in the attempt state.
- `pow_key_for_nonce(s_A, nonce)`.
- one verifier-derived selected tile.
- recursive Nockchain certificate noun.

This mode is not Pearl merge-mining compatible.

### Pearl Merge-Mining AI-PoW

New design:

- exact Pearl `sigma || mu` transcript;
- jackpot key is `s_A`;
- Pearl tile-ticket semantics;
- Nockchain-native recursive certificate proving a Pearl-compatible statement;
- Nockchain block bound by AuxPoW inclusion in Pearl work state.

This mode is the compatibility target.

Do not blend the modes. A proof must declare which mode it is using, and the
verifier must derive the statement entirely according to that mode.

## Open Decisions

1. **Aux commitment location in Pearl.** Prefer a coinbase/transaction output
   or other Pearl-consensus-valid commitment that is included in Pearl's
   `tx_root` and therefore in `sigma`. Confirm exact Pearl policy and relay
   rules.
2. **Pearl header serialization.** Must be taken from Pearl consensus code.
   The whitepaper's 116-byte header description is not enough for a verifier
   implementation.
3. **Certificate format.** Define the Nockchain-native recursive certificate
   that proves the Pearl-compatible statement. It does not need to match
   Pearl's certificate bytes or recursion stack. Pearl block identity does not
   depend on certificate bytes, so byte-preserving Pearl certificate transport
   is unnecessary for Nockchain consensus.
4. **Target calibration.** Decide whether Nockchain target is independent from
   Pearl target or derived from Pearl work bits. Independent targets are
   currently implemented at the Rust precheck layer and are standard for
   merge-mining; final consensus policy still needs activation review.
5. **Parameter subset.** Decide whether to implement Pearl's full tile-shape
   language now or define a negotiated Pearl-compatible subset.

## Non-Negotiable Requirements

- No Nockchain-only nonce may enter the Pearl-compatible `kappa` unless the
  same bytes are part of Pearl `sigma`.
- No `pow_key_for_nonce` may be used in Pearl-compatible jackpot hashing.
- No verifier-selected `found_idx` may be imposed in Pearl-compatible mode
  unless Pearl also imposes it.
- No row/column diagnostic roots may be reintroduced as production commitment
  parameters. Pearl-compatible production commitments are `H_A` and `H_B`, the
  proof-bound BLAKE3 matrix commitments.
- No source-grep tests. Compatibility must be proven by byte-vector tests,
  statement-derivation tests, and end-to-end verifier tests.

## Minimal Milestone Plan

1. [done] Implement exact Pearl transcript serialization in
   `ai-pow::pearl_compat` for the current square-tile `MatmulParams` slice.
2. [partial] Add byte fixtures generated from Pearl source for `sigma`, `mu`, `kappa`,
   commitments, seeds, tile state, and jackpot hash.
3. [done for Rust precheck] Add a Pearl-compatible statement precheck in Rust
   that rejects Nockchain-native statement fields and verifies Pearl transcript
   commitments, the pattern-indexed ticket, and both target systems over the
   same jackpot digest.
4. Define and implement the Nockchain auxiliary commitment and Pearl inclusion
   proof.
5. [partial] Build a local end-to-end fixture:
   - one Pearl work instance;
   - one Nockchain-native certificate proving that Pearl-compatible statement;
   - one Nockchain block commitment included through aux;
   - [done in Rust precheck] both Pearl target and Nockchain target checks run
     over the same digest.
6. Wire the Hoon noun type and Rust verifier jet.
7. Only after honest/tamper tests pass, consider activating Pearl
   merge-mining mode.
