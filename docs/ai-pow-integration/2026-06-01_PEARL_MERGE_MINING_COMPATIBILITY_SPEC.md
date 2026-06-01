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

The ZK proof systems do not need to be identical. In fact, Nockchain should not
try to make its production proof artifact byte-compatible with Pearl's proof
artifact unless that independently becomes the best Nockchain verifier design.
Nockchain should keep its own proof artifact and verifier stack if that is
smaller, easier to verify from Hoon/Rust, or better aligned with Nockchain
consensus. Compatibility is defined at the PoW attempt layer: both chains must
be checking the same public work instance and jackpot digest, even if they
receive different certificates proving that statement.

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

This gives the desired merge-mining property without coupling verifier stacks:
a miner can commit to one Pearl block candidate and one Nockchain block
candidate, evaluate one Pearl-compatible work attempt, and then produce whatever
per-chain certificate each chain expects for that same attempt. Pearl does not
need to parse Nockchain's recursive proof, and Nockchain does not need to parse
Pearl's proof. The shared object is the mineable attempt, not the ZKP artifact.

The operational requirement is therefore:

> A miner must be able to commit to a specific Pearl block candidate and a
> specific Nockchain block candidate before mining, run one Pearl-compatible
> AI-PoW attempt, and have that same attempt be eligible for both chains'
> target checks.

The Nockchain recursive ZKP is only Nockchain's certificate for that shared
attempt. It is not part of Pearl compatibility, and it should not be forced to
match Pearl's ZKP format, recursion stack, proof bytes, or verifier API.

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
  - a wire-facing byte verifier,
    `verify_pearl_compatible_public_data`, that accepts exactly the 76-byte
    serialized Pearl header plus exactly the 164-byte public proof parameter
    blob and then runs the same composed precheck;
  - a bounded Nockchain AuxPoW commitment primitive,
    `pearl_nockchain_aux_commitment`, for replay-protecting a Nockchain block
    commitment inside the Pearl work state before mining;
  - canonical Nockchain aux byte encoding/decoding through
    `PearlNockchainAux::to_bytes` / `from_bytes`, with magic `NPA1`, bounded
    variable fields, and trailing-data rejection;
  - a canonical Pearl merge public statement byte envelope,
    `PearlMergePublicStatement`, with magic `PMP1`, fixed Pearl header/public
    data fields, the expected Pearl-included aux digest, bounded aux bytes, and
    trailing-data rejection;
  - single-ticket Pearl merge-mining construction APIs,
    `evaluate_pearl_merge_ticket_attempt` and
    `mine_pearl_merge_ticket_attempt`, that build the public `PMP1` statement
    for one explicit `t_rows` / `t_cols` ticket and return `None` before ZKP
    work when that exact ticket fails either target;
  - a combined merge-mining statement precheck,
    `verify_pearl_merge_mining_public_data`, plus the wire-facing
    `verify_pearl_merge_mining_public_data_with_aux_bytes` and
    `verify_pearl_merge_public_statement_bytes`, that verifies the exact Pearl
    public work bytes and rejects unless the Nockchain aux fields name the
    trusted candidate Nockchain block commitment and hash to the expected
    digest that the caller verified inside Pearl's work state;
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
    and malformed matrix inputs;
  - the wire-facing public-data precheck accepts exact serialized header/public
    data bytes and rejects short/trailing header bytes, short/trailing public
    data bytes, decode-time offset tampering, and commitment tampering;
  - the AuxPoW commitment has an exact byte encoding, length-prefixes variable
    fields to prevent concatenation ambiguity, binds chain id, Nockchain block
    commitment, target epoch/height, and extra domain data, and rejects
    unbounded or missing fields;
  - the aux wire envelope round-trips exactly and rejects malformed length,
    bad magic, empty chain id, oversized extra field, and trailing bytes;
  - the `PMP1` merge public statement envelope round-trips exactly and rejects
    short input, bad magic, bad declared aux length, and malformed nested aux
    bytes;
  - the combined merge-mining precheck accepts a work statement only when the
    expected Pearl-included aux digest matches the supplied Nockchain aux
    fields and those fields name the trusted candidate Nockchain block
    commitment, and rejects aux replay/tamper, wrong-candidate replay, and
    ordinary work tamper;
  - the wire-facing merge precheck accepts canonical aux bytes and rejects
    malformed aux bytes through the same verifier entrypoint.
- Hoon/Rust structured artifact boundary for the merge-mined variant:
  - `hoon/common/tx-engine-1.hoon` defines `pearl-merge-public-statement`,
    `pearl-nockchain-aux`, and `pearl-merge-ai-pow-artifact` with fixed
    digest fields carried as `@uxblake` atoms, variable-length aux fields
    carried as `[len data]` pairs so trailing zero bytes are canonical, and no
    Pearl ZKP arm;
  - `hoon/apps/dumbnet/lib/types.hoon` admits a separate `%ai-pmp` PoW
    variant for Pearl merge-mined AI-PoW;
  - dumbnet consensus dispatch and page hashing recognize `%ai-pmp`, but
    remain fail-closed until recursive proof verification and Pearl aux
    inclusion verification are wired;
  - `ai_pow_miner::certificate_noun` decodes the structured `%ai-pmp`
    artifact, reconstructs the canonical `PMP1`/`NPA1` byte statement for the
    Rust precheck, and rejects recursive-certificate metadata/public inputs
    that prove an NCMN-local statement, wrong tile, wrong trace height, or
    wrong target-mode metadata instead of the Pearl-compatible statement;
    this includes binding both `JACKPOT_MSG` to the recomputed Pearl ticket
    `TileState` and `HASH_JACKPOT` to the recomputed Pearl jackpot digest;
  - the `%ai-pmp` verifier boundary explicitly validates the mirrored
    `MatmulParams` production envelope and rejects Pearl-compatible public
    work statements whose geometry is outside the current Nockchain recursive
    square-tile subset;
  - `precheck_ai_pow_pearl_merge_artifact_jam` performs byte-limit, jam
    preflight, canonical-cue, statement decode, certificate-metadata decode,
    and Pearl merge precheck before walking the recursive proof-node tail;
  - `verify_decoded_ai_pow_pearl_merge_artifact`,
    `verify_ai_pow_pearl_merge_artifact_statement_and_proof`, and
    `verify_ai_pow_pearl_merge_artifact_jam` are the production-shaped Rust
    verifier APIs for decoded nouns, already reconstructed certificates, and
    jammed block artifacts.
  - canonical Rust noun builders now exist for the production artifact shape:
    `build_pearl_merge_public_statement_noun`,
    `build_pearl_merge_public_statement_slab`,
    `build_ai_pow_pearl_merge_artifact_noun_from_node`, and
    `build_ai_pow_pearl_merge_artifact_noun`. These builders emit `%ai-pmp`
    artifacts only; they do not emit Pearl ZKPs, raw Layer-0 `MatmulProof`s,
    or plain nonrecursive ZK proof nouns.
  - `PearlMergePublicStatementShape::from_wire_statement` and
    `from_wire_bytes` bridge the `ai_pow::pearl_compat` `PMP1` statement into
    the structured Hoon noun shape used by `%ai-pmp`.
  - `pearl_merge_recursive_public_inputs_from_work` /
    `pearl_merge_recursive_public_inputs_from_precheck` centralize the
    Pearl-mode mapping into existing recursive public-input slots:
    `HASH_A`, `HASH_B`, `JOB_KEY = kappa`, `COMMITMENT_HASH = s_A`,
    `JACKPOT_MSG = TileState`, and `HASH_JACKPOT = jackpot_hash`.
  - `pearl_merge_recursive_certificate_parts_from_ticket` is the canonical
    producer-side bridge from a successful `PearlMergeTicketAttempt` to the
    recursive certificate metadata in `%ai-pmp`. It rechecks the public ticket
    against the trusted matrices and rejects target misses, forged statement
    drift, forged work/ticket fields, and Pearl geometries outside the current
    square contiguous recursive subset before an artifact can be built.
  - `pearl_merge_recursive_certificate_parts_from_ticket_public_inputs` is the
    production prover handoff variant for real recursive proof runs: it
    preserves the proof-derived `cumsum` public inputs while re-deriving and
    checking every Pearl-bound public-input slot from the successful ticket.
  - `build_ai_pow_pearl_merge_artifact_noun_from_ticket_node` and
    `build_ai_pow_pearl_merge_artifact_noun_from_ticket` build `%ai-pmp`
    directly from that canonical ticket-derived metadata instead of accepting
    caller-supplied `found-idx`, public inputs, or commitments.
  - `build_ai_pow_pearl_merge_artifact_noun_from_ticket_public_inputs_node`
    and `build_ai_pow_pearl_merge_artifact_noun_from_ticket_public_inputs`
    build the same `%ai-pmp` artifact while preserving the actual recursive
    proof public-input vector after the Pearl-bound slots are checked.
  - `build_ai_pow_pearl_merge_artifact_noun_from_ticket_recursive_run` is the
    production handoff from an actual recursive prover run to the `%ai-pmp`
    noun. It uses the proof's own public inputs and certificate, while
    re-deriving and checking the Pearl-compatible statement fields from the
    shared attempt.
  - `ai_pow_miner::run::build_ai_pow_pearl_merge_certificate_poke` constructs
    the node submission noun `[%command %pow %ai-pmp artifact]` after first
    decoding the artifact as `%ai-pmp`, so miner-side code cannot accidentally
    submit a native `%ai-pow` certificate, Pearl ZKP, or raw `MatmulProof` on
    the merge-mining arm.
  - `build_ai_pow_pearl_merge_certificate_poke_from_ticket_node` and
    `build_ai_pow_pearl_merge_certificate_poke_from_ticket` are the
    miner-facing safe submission helpers: they derive `%ai-pmp` from the
    successful ticket and trusted matrices, then wrap only that derived
    artifact as the kernel command.
  - `build_ai_pow_pearl_merge_certificate_poke_from_ticket_public_inputs_node`
    and `build_ai_pow_pearl_merge_certificate_poke_from_ticket_public_inputs`
    are the corresponding safe helpers for real prover runs whose recursive
    public inputs include trace-derived fields such as `cumsum`.
  - `build_ai_pow_pearl_merge_certificate_poke_from_ticket_recursive_run`
    wraps the real recursive prover run into the production node command noun
    after the same ticket-derived checks.
  - `AiPowSubmissionMode` makes the run-loop proof arm explicit. The current
    node loop accepts `NativeNcmn` only and fails closed for `PearlMerge`
    before enabling mining, so a caller cannot accidentally configure Pearl
    mode and submit native `%ai-pow` attempts.
  - `ai_pow_miner::pearl_mining` now provides a standalone
    `PearlMergeMiningJob` / `PearlMergeMineOptions` loop that scans
    Pearl-valid `t_rows` / `t_cols` ticket pairs and returns a
    `PearlMergeTicketAttempt` only after the shared jackpot digest satisfies
    both Pearl and Nockchain targets. It maps linear attempt ordinals to
    Pearl-valid offsets without materializing all valid offset pairs up front,
    and it does not build a recursive proof or `%ai-pmp` artifact on misses.
  - `ai_pow::zk_bridge::prove_pearl_merge_recursive_certificate` builds a
    Nockchain-native recursive certificate for the supported
    square-contiguous Pearl tile-ticket subset. It rechecks the shared `PMP1`
    statement against trusted matrices, proves the exact Pearl ticket tile,
    uses Pearl `s_A` as the jackpot key, and does not serialize or reuse
    Pearl's ZKP.
- Release tests in `crates/ai-pow-miner/src/certificate_noun.rs` proving:
  - `%ai-pmp` keeps the Pearl statement structured rather than as a single
    opaque statement atom;
  - aux `chain-id` and `extra-domain-data` preserve trailing zero bytes through
    explicit `[len data]` fields;
  - the public statement builder round-trips the exact `NPA1`/`PMP1` bytes,
    including trailing zero bytes in aux fields;
  - malformed `[len data]` aux nouns with a declared length shorter than the
    nonzero atom payload are rejected before statement precheck;
  - the public `%ai-pmp` artifact builder round-trips through jam decoding and
    preserves recursive certificate metadata, public inputs, and the structured
    Pearl-compatible statement;
  - the ticket-derived `%ai-pmp` builder derives `zk_params`, `found-idx`,
    `trace-height`, commitments, and Pearl-mode recursive public inputs from
    the successful shared ticket, then round-trips through decode and precheck;
  - ticket-derived artifact construction rejects non-winning tickets,
    tampered `PMP1` statement bytes, forged jackpot work that is not derived
    from the trusted matrices, wrong trusted matrices that do not match the
    ticket's public commitments, and non-contiguous Pearl tickets outside the
    current recursive verifier subset;
  - precheck-derived Pearl statement public-input slots exactly match the
    artifact metadata used for recursive verification;
  - the `%ai-pmp` miner poke builder wraps only a decoded Pearl merge artifact
    as `[%command %pow %ai-pmp artifact]` and rejects the wrong artifact arm;
  - the ticket-derived `%ai-pmp` miner poke helpers derive the wrapped artifact
    from the ticket and trusted matrices, and reject wrong matrices before
    command construction;
  - run-loop preflight rejects explicit `PearlMerge` submission mode until the
    Pearl ticket work loop and recursive Pearl statement prover are wired;
  - the standalone Pearl ticket loop returns a ticket before proof/artifact
    construction on success, returns budget exhaustion without emitting any
    ticket artifact on misses, supports deterministic later-start scanning,
    rejects malformed matrix inputs, and honors cancellation before work;
  - a miner-side integration regression runs the standalone Pearl ticket loop,
    feeds the returned ticket into the ticket-derived `%ai-pmp` poke helper,
    and decodes the wrapped artifact; the miss path has no ticket to submit;
  - the decoded artifact prechecks the exact shared Pearl attempt and
    Nockchain aux binding;
  - wrong-candidate replay, wrong `found-idx`, wrong trace height, wrong
    difficulty metadata, wrong `JACKPOT_MSG`, and recursive-certificate
    public-input tampering are rejected before recursive proof
    reconstruction/verification;
  - Pearl-valid public work statements with matrix geometry outside the
    current recursive square-tile subset are rejected as unsupported instead of
    being floor-divided into a different tile grid;
  - jammed `%ai-pmp` full verification preserves the same failure ordering:
    replay/tamper rejects before proof-node decode, while a valid precheck with
    malformed proof tail reaches the proof-node/proof-verification error.

Still incomplete:

- the Rust precheck can recompute and target-check the Pearl pattern-indexed
  ticket, but the recursive proof circuit still needs to prove that same
  statement; current proof generation remains Nockchain square-tile oriented;
- Nockchain-native recursive certificate whose public inputs prove the
  Pearl-compatible statement;
- AuxPoW inclusion binding from Pearl `sigma` to a Nockchain block commitment;
- Rust verifier jet / consensus callsite for the merge-mined artifact;
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
   aux_commitment = BLAKE3(
       "nockchain-ai-pow-aux-v1" ||
       le32(len(nockchain_chain_id)) ||
       nockchain_chain_id ||
       nock_block_commitment[32] ||
       le64(nockchain_target_epoch_or_height) ||
       le32(len(optional_extra_domain_data)) ||
       optional_extra_domain_data
   )
   ```

   `nockchain_chain_id` is nonempty and at most 64 bytes.
   `optional_extra_domain_data` is at most 1024 bytes. These bounds are
   enforced by `ai_pow::pearl_compat::pearl_nockchain_aux_commitment`.

   The Nockchain-side aux fields have this canonical byte envelope before
   noun/wire embedding:

   ```text
   nockchain-aux-bytes =
       magic[4]                    = "NPA1"
       chain_id_len[1]             = u8, 1..=64
       nockchain_chain_id[*]
       nock_block_commitment[32]
       nockchain_target_epoch_or_height[8] = little-endian u64
       extra_domain_data_len[2]    = little-endian u16, 0..=1024
       optional_extra_domain_data[*]
   ```

   The parser rejects short input, bad magic, zero-length chain id, oversized
   fields, and trailing bytes.

   The combined public statement envelope for Nockchain's merge verifier is:

   ```text
   pearl-merge-public-statement =
       magic[4]                 = "PMP1"
       pearl_header[76]          = serialized Pearl IncompleteBlockHeader
       pearl_public_data[164]    = MiningConfiguration || H_A || H_B ||
                                   jackpot_hash || m || n || t_rows || t_cols
       expected_aux_commitment[32]
       aux_bytes_len[2]          = little-endian u16
       nockchain-aux-bytes[*]    = "NPA1" envelope above
   ```

   The verifier-facing Rust entrypoint is
   `verify_pearl_merge_public_statement_bytes`. It still takes verifier-derived
   `candidate_nock_block_commitment`, Nockchain target, and matrix/proof
   witness data outside the miner-controlled envelope.

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

The phrase "Pearl-compatible proof of work" in this document always means this
shared attempt and jackpot digest. It does not mean "use Pearl's exact ZKP" or
"store Pearl's certificate in Nockchain." Nockchain's production artifact should
remain the structured `%ai-pmp` noun containing the Pearl-compatible public
statement plus Nockchain's canonical recursive certificate for that statement.

### Compatibility Boundary

The shared cross-chain object is the Pearl-compatible PoW attempt:

- the Pearl block/work state bytes `sigma`;
- the Pearl mining configuration bytes `mu`;
- the matrices `A` and `B`;
- the Pearl-derived `kappa`, `H_A`, `H_B`, `s_B`, `s_A`;
- the selected Pearl-legal tile state;
- the jackpot digest `BLAKE3(M_i_j, key=s_A)`.

The non-shared objects are the per-chain certificates and block wrappers:

- Pearl may use Pearl's own block certificate and proof system.
- Nockchain must use its canonical recursive AI-PoW certificate format.
- Nockchain's certificate must prove the shared Pearl-compatible public
  statement and the Nockchain block binding, but its bytes need not be accepted
  by Pearl and Pearl's certificate bytes need not be accepted by Nockchain.

This boundary is consensus-relevant. If Nockchain changes any input that feeds
the shared attempt, it has forked the mineable work and is no longer
merge-mining Pearl. If Nockchain changes only its recursive proof wrapper while
proving the same public statement, compatibility is preserved.

## Required Nockchain Wire / Noun Shape

Add a Pearl-compatible AI-PoW artifact variant. Do not overload the current
Nockchain-specific `ai-pow-certificate` silently.

Implemented Hoon-level shape:

```hoon
+$  pearl-header  @uxpearlhdr
+$  pearl-public-data  @uxpearlpub
+$  pearl-chain-id  [len=@ud data=@uxpearlid]
+$  pearl-extra-data  [len=@ud data=@uxpearlextra]
+$  pearl-nockchain-aux
  $:  nockchain-chain-id=pearl-chain-id
      nock-block-commitment=@uxblake
      nockchain-target-epoch-or-height=@ud
      extra-domain-data=pearl-extra-data
  ==
+$  pearl-merge-public-statement
  $:  block-header=pearl-header
      public-data=pearl-public-data
      expected-aux-commitment=@uxblake
      aux=pearl-nockchain-aux
  ==
+$  pearl-merge-ai-pow-artifact
  $:  statement=pearl-merge-public-statement
      certificate=ai-pow-certificate
  ==
+$  pow-variant
  $%  [%ai-pmp artifact=pearl-merge-ai-pow-artifact]
      ...
  ==
```

The `%ai-pmp` tag is the short on-wire noun tag for Pearl merge-mined
AI-PoW. It maps to the longer spec version name `pearl-merge-v1`.

The structured Hoon noun maps to the Rust `PMP1` byte envelope as follows:

- `block-header` is exactly the 76-byte Pearl `IncompleteBlockHeader`;
- `public-data` is exactly the 164-byte Pearl public proof params
  (`MiningConfiguration || H_A || H_B || jackpot_hash || m || n || t_rows ||
  t_cols`);
- `expected-aux-commitment` is the Pearl-included aux digest;
- `aux` is re-encoded as `NPA1` before the Rust merge precheck.

The `[len data]` aux fields are consensus-significant. The canonical builders
MUST set `len` to the intended byte length before atom trimming, and decoders
MUST reconstruct exactly `len` bytes by zero-padding the atom representation
when necessary. This is what lets Hoon nouns represent `NPA1` fields ending in
zero bytes without losing consensus-visible data. A decoder MUST reject a noun
when the atom contains more nonzero payload bytes than the declared length.

Canonical Rust construction APIs:

- `build_pearl_merge_public_statement_noun(allocator, statement)`;
- `build_pearl_merge_public_statement_slab(statement)`;
- `build_ai_pow_pearl_merge_artifact_noun_from_node(statement, zk_params,
  found_idx, trace_height, commitments, public_inputs, proof_node)`;
- `build_ai_pow_pearl_merge_artifact_noun(statement, zk_params, found_idx,
  trace_height, commitments, public_inputs, recursive_certificate)`;
- `pearl_merge_recursive_certificate_parts_from_ticket(attempt, a_row_major,
  b_col_major, max_pattern_len)`;
- `pearl_merge_recursive_certificate_parts_from_ticket_public_inputs(attempt,
  a_row_major, b_col_major, max_pattern_len, public_inputs)`;
- `build_ai_pow_pearl_merge_artifact_noun_from_ticket_node(attempt,
  a_row_major, b_col_major, max_pattern_len, proof_node)`;
- `build_ai_pow_pearl_merge_artifact_noun_from_ticket(attempt, a_row_major,
  b_col_major, max_pattern_len, recursive_certificate)`;
- `build_ai_pow_pearl_merge_artifact_noun_from_ticket_public_inputs_node(
  attempt, a_row_major, b_col_major, max_pattern_len, public_inputs,
  proof_node)`;
- `build_ai_pow_pearl_merge_artifact_noun_from_ticket_public_inputs(attempt,
  a_row_major, b_col_major, max_pattern_len, public_inputs,
  recursive_certificate)`;
- `build_ai_pow_pearl_merge_artifact_noun_from_ticket_recursive_run(attempt,
  a_row_major, b_col_major, max_pattern_len, run)`;
- `build_ai_pow_pearl_merge_certificate_poke_from_ticket_node(attempt,
  a_row_major, b_col_major, max_pattern_len, proof_node)`;
- `build_ai_pow_pearl_merge_certificate_poke_from_ticket(attempt, a_row_major,
  b_col_major, max_pattern_len, recursive_certificate)`;
- `build_ai_pow_pearl_merge_certificate_poke_from_ticket_public_inputs_node(
  attempt, a_row_major, b_col_major, max_pattern_len, public_inputs,
  proof_node)`;
- `build_ai_pow_pearl_merge_certificate_poke_from_ticket_public_inputs(attempt,
  a_row_major, b_col_major, max_pattern_len, public_inputs,
  recursive_certificate)`;
- `build_ai_pow_pearl_merge_certificate_poke_from_ticket_recursive_run(attempt,
  a_row_major, b_col_major, max_pattern_len, run)`.

Production callers should prefer the ticket-derived `%ai-pmp` builders for
Pearl-compatible mode. Those builders derive certificate metadata from the
successful shared ticket, recompute the public work against the trusted
matrices, and reject misses or forged ticket fields before recursive artifact
construction. Once a real Pearl-compatible recursive prover returns its public
inputs, callers should use the `_public_inputs` variants so trace-derived
fields such as `cumsum` are preserved while `HASH_A`, `HASH_B`, `JOB_KEY`,
`COMMITMENT_HASH`, `JACKPOT_MSG`, and `HASH_JACKPOT` remain derived from the
ticket. Once callers have an `AiPowRecursiveCertificateRun`, they should use
the `_recursive_run` variants so the artifact is built from the exact proof
output that was verified against the shared attempt. Callers should not
separately serialize a Pearl ZKP, raw `MatmulProof`, or plain Layer-0 AI-PoW
proof into Hoon consensus state.

The exact Pearl header, `MiningConfiguration`, public proof parameter, and aux
inclusion encodings must be derived from Pearl's current consensus code, not
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
- Reject if the aux fields' `nock_block_commitment` differs from the
  recomputed candidate Nockchain block commitment.
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

The certificate API should make this separation explicit. Production
`pearl-merge-v1` verification should accept only:

- the canonical Nockchain recursive proof artifact;
- the canonical Pearl-compatible public statement envelope;
- the auxiliary Pearl-inclusion data needed to bind the Nockchain block into
  `sigma`.

It should not expose or accept a "Pearl ZKP" path as the Nockchain production
certificate, because that would confuse proof-format compatibility with
mineable-attempt compatibility.

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
  the composed `verify_pearl_compatible_work` API, and the serialized
  `verify_pearl_compatible_public_data` API. Also done:
  `evaluate_pearl_merge_ticket_attempt` and `mine_pearl_merge_ticket_attempt`
  construct a canonical `PMP1` statement for one explicit Pearl ticket and
  return no work product before recursive proof generation when the ticket
  misses either target. Still required: teach the recursive proof
  statement/circuit to prove Pearl's general row/column pattern-indexed ticket
  instead of only the current square-tile `MatmulParams` model.
- Done for the Nockchain-side AuxPoW digest primitive: add
  `pearl_nockchain_aux_commitment` with bounded, length-prefixed fields and
  `verify_pearl_merge_mining_public_data` to tie the aux digest and candidate
  Nockchain block commitment to the verified Pearl public work statement.
  Also done: canonical `PearlNockchainAux` byte encode/decode and
  `verify_pearl_merge_mining_public_data_with_aux_bytes` for the
  Nockchain-side wire/noun boundary, plus the `PMP1`
  `PearlMergePublicStatement` envelope and
  `verify_pearl_merge_public_statement_bytes`. Still required: embed that
  digest into a Pearl-consensus-valid commitment path and verify the inclusion
  proof against the exact `sigma`.
- Done for Hoon/Rust artifact shape: add `%ai-pmp` as a distinct Hoon PoW
  variant, define structured `pearl-merge-public-statement` /
  `pearl-nockchain-aux` nouns, keep consensus fail-closed for the new arm, and
  add `decode_ai_pow_pearl_merge_artifact_*` plus
  `precheck_ai_pow_pearl_merge_artifact_statement` in the Rust noun boundary.
  Also done: variable-length aux fields are `[len data]`, and
  `precheck_ai_pow_pearl_merge_artifact_jam` rejects replay and metadata
  mismatch before proof-node traversal. Also done: decoded, already
  reconstructed-certificate, and jammed `%ai-pmp` verifier APIs run the same
  precheck before recursive verification, including explicit rejection for
  Pearl-valid geometries outside the current recursive square-tile production
  envelope and explicit binding of both `JACKPOT_MSG` and `HASH_JACKPOT` to
  the recomputed Pearl ticket. Also done: canonical builder APIs for structured
  `pearl-merge-public-statement` and `%ai-pmp` nouns, with tests covering exact
  trailing-zero aux byte preservation, malformed declared aux lengths, and
  jam-decoded artifact round-trips. Also done: wire-statement conversion APIs
  bridge the `ai-pow` `PMP1` construction result into the Hoon-compatible
  structured statement shape, a single helper derives the Pearl statement
  public-input slots, ticket-derived artifact builders derive all recursive
  metadata from a successful shared attempt, recompute the public work against
  trusted matrices, preserve real proof public inputs only after checking the
  Pearl-bound slots, and reject target misses, statement drift, forged
  work/ticket fields, public-input tamper, and unsupported non-contiguous
  tickets. Also done: the square-contiguous recursive prover path
  (`prove_pearl_merge_recursive_certificate`) proves the exact Pearl ticket
  tile with Pearl `s_A` as the jackpot key, and `_recursive_run` artifact and
  poke builders serialize only the resulting Nockchain recursive certificate.
  Also done: the miner-side `%ai-pmp` command-poke helpers either wrap only
  decoded Pearl merge artifacts or derive the artifact directly from the
  successful ticket and trusted matrices. Also done: the node run loop has an
  explicit `AiPowSubmissionMode` and fails closed for `PearlMerge`, and the
  standalone `ai_pow_miner::pearl_mining` loop scans Pearl-valid ticket offset
  pairs without generating recursive proofs or artifacts on misses. Still
  required: wire the verifier jet/callsite for `%ai-pmp`, derive/verify the
  Pearl aux inclusion proof against Pearl consensus data, and either extend
  the prover to Pearl's full row/column pattern language or activate only the
  square-contiguous subset that Nockchain proves byte-for-byte.
- Add byte-equivalence tests against Pearl source fixtures for:
  - `sigma || mu` serialization;
  - `kappa`;
  - `H_A`, `H_B`;
  - `s_A`, `s_B`;
  - jackpot hash using `s_A`;
  - target comparison.

### `ai-pow-zk`

- Done for the current square-contiguous subset: add
  `prove_pearl_merge_recursive_certificate`, a Nockchain recursive certificate
  producer that proves the Pearl-compatible ticket statement with Pearl `s_A`
  as the jackpot key.
- Ensure public inputs match Pearl's work statement exactly. In
  Pearl-compatible mode, the public slot currently named `COMMITMENT_HASH` must
  contain `s_A`, not `pow_key_for_nonce(s_A, nonce)`.
- Done for the current subset: remove Nockchain-only `attempt_tile_index` from
  the Pearl-compatible mineable statement. The prover derives `found_idx` from
  Pearl `t_rows` / `t_cols` only after checking the ticket rows and columns.
- Still required: support Pearl's full tile-shape language or explicitly
  restrict production merge mining to the subset of Pearl configs that
  Nockchain can prove byte-for-byte.
- Add an end-to-end test that verifies a Nockchain-native certificate over
  Pearl fixture data and checks that Pearl's own transcript would derive the
  same jackpot digest.

### `ai-pow-miner`

- Done for the standalone merge-mining ticket loop:

  ```rust
  PearlMergeMiningJob {
      header,
      config,
      params,
      nockchain_target,
      matrices,
      aux,
      max_pattern_len,
  }
  ```

  `pearl_mining::run` scans Pearl-valid offset pairs, counts one matmul
  attempt per explicit ticket, maps attempt ordinals to offsets without
  precomputing the full offset-pair list, returns only a successful
  `PearlMergeTicketAttempt`, and never constructs recursive proof/artifact data
  for target misses. The mined ticket is directly consumable by the
  ticket-derived `%ai-pmp` poke helpers; tests cover this handoff and the
  no-ticket miss path.
- Done for the current node run-loop boundary: `AiPowSubmissionMode` is
  explicit, `NativeNcmn` remains the only runnable mode, and `PearlMerge`
  fails preflight before mining can start. This is intentional until the
  following items are implemented; it prevents a misconfigured merge miner from
  silently producing native `%ai-pow` submissions.
- Build the auxiliary commitment into the Pearl block template before mining.
- Wire the standalone Pearl-compatible work loop into the node run loop once a
  Pearl candidate/job source exists.
- On success, emit:
  - Pearl submission payload for Pearl;
  - Nockchain `%ai-pmp` payload with Pearl work statement, aux proof, and
    Nockchain-native recursive certificate.

### Hoon / Consensus

- Add a new wire/certificate variant for Pearl-compatible merge-mined AI-PoW.
- Keep the new `%ai-pmp` path fail-closed until the Rust verifier jet can
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

This mode deliberately does not require Pearl and Nockchain to share a ZKP
format. It requires only that both proof systems authenticate the same public
work attempt and jackpot digest.

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
   commitments, the pattern-indexed ticket, exact serialized public-data
   boundaries, and both target systems over the same jackpot digest.
4. [partial] Define and implement the Nockchain auxiliary commitment and Pearl
   inclusion proof:
   - [done] bounded Nockchain aux commitment digest;
   - [open] Pearl-consensus-valid inclusion location and verifier.
5. [partial] Build a local end-to-end fixture:
   - one Pearl work instance;
   - one Nockchain-native certificate proving that Pearl-compatible statement;
   - one Nockchain block commitment included through aux;
   - [done in Rust precheck] both Pearl target and Nockchain target checks run
     over the same digest.
6. [partial] Wire the Hoon noun type and Rust verifier jet:
   - [done] Hoon `%ai-pmp` artifact shape;
   - [done] Rust structured noun decoder, statement precheck, and
     production-shaped verifier APIs;
   - [open] Rust verifier jet / consensus callsite.
7. Only after honest/tamper tests pass, consider activating Pearl
   merge-mining mode.
