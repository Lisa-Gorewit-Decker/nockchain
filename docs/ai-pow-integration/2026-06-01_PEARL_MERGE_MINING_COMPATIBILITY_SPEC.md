# Pearl-Compatible Nockchain AI-PoW Submission Specification

Date: 2026-06-01
Status: Nockchain-side submission target

## Goal

Nockchain AI-PoW should support a Pearl-format-compatible work attempt so one
miner can commit to a Pearl block candidate and a Nockchain block candidate,
evaluate one Pearl-style useful-work attempt, and submit the result to
Nockchain when that attempt satisfies Nockchain's target. Pearl-chain
submission plumbing is intentionally outside this milestone.

The chains do not need to share proof systems. Nockchain should use its own
recursive certificate and verifier. Compatibility is at the mineable work layer:
the same Pearl-style `sigma`, `mu`, matrix commitments, noise seeds, ticket
tile state, and jackpot digest must be used.

For the current milestone, only Nockchain-side submission is being wired. That
means:

- Nockchain accepts only the canonical `%ai-pow` block proof arm.
- Hoon does not define Pearl-specific molds or dispatch arms.
- Pearl-format details live inside Rust-owned nonce bytes.
- Nockchain acceptance requires the shared jackpot digest to satisfy the
  Nockchain target.
- Nockchain acceptance does not require the shared jackpot digest to satisfy
  Pearl's `nbits` target. Pearl target handling belongs to a separate
  Pearl-side submission implementation, not this Nockchain wiring.

## Canonical Hoon Artifact

The only production AI-PoW block artifact is:

```hoon
[%ai-pow nonce=ai-pow-nonce cert=ai-pow-certificate]
```

`ai-pow-nonce` is opaque to Hoon:

```hoon
+$  ai-pow-nonce  [len=@ud data=@uxaipownonce]
```

The recursive certificate remains structured:

```hoon
+$  ai-pow-certificate
  $:  version=@ud
      params=[m=@ud k=@ud n=@ud noise-rank=@ud tile=@ud difficulty-bits=@ud]
      found-idx=@ud
      trace-height=@ud
      commitments=ai-pow-commitments
      public-inputs=ai-pow-public-inputs
      certificate=ai-recursive-certificate
  ==
```

There is no Hoon `%ai-pmp` arm. There are no Hoon `pearl-*` types. The Hoon
kernel should not know whether the nonce bytes contain Pearl-compatible
material, a future native encoding, or another Rust-owned encoding. It stores
and hashes the artifact as `%ai-pow` and calls a Rust verifier for semantics.

## Opaque Nonce Bytes

The current Rust-owned nonce byte envelope for Pearl-format-compatible
Nockchain submission is:

```text
ai_pow_nonce_v1 =
    magic[4]              = "AIP1"
    statement_len[2]      = little-endian u16
    statement[statement_len]
    coinbase_tx_len[4]    = little-endian u32
    coinbase_tx[coinbase_tx_len]
    merkle_branch_len[1]
    merkle_branch[merkle_branch_len][32]
```

`statement` is the existing Rust `PMP1` Pearl-compatible public statement:

```text
PMP1 =
    magic[4]                    = "PMP1"
    pearl_incomplete_header[76]
    pearl_public_data[164]
    expected_aux_commitment[32]
    aux_len[2]
    aux_bytes[aux_len]
```

`aux_bytes` is the existing `NPA1` Nockchain aux envelope:

```text
NPA1 =
    magic[4]                    = "NPA1"
    chain_id_len[1]
    chain_id[chain_id_len]
    nock_block_commitment[32]
    target_epoch_or_height[8]
    extra_domain_data_len[2]
    extra_domain_data[extra_domain_data_len]
```

The `coinbase_tx` and `merkle_branch` prove that
`"NOCKCHAIN-AI-POW-AUX" || expected_aux_commitment` appears in the
txid-committed coinbase input script and that the coinbase txid is committed by
the Pearl header transaction merkle root. Nockchain can use a coinbase-only
Pearl block profile, so the current production verifier requires
`merkle_branch_len = 0`. The branch-length byte remains in the Rust-owned nonce
format for forward compatibility, but any nonzero branch is rejected until a
future milestone deliberately supports Pearl transaction merkle trees. A
witness-only occurrence or an output script occurrence is not sufficient for
Nockchain acceptance.

The current maximum nonce size is pinned by Rust tests at 101,424 bytes
(approximately 100.0 KiB):

```text
4 + 2 + PEARL_MERGE_PUBLIC_STATEMENT_MAX_SIZE
+ 4 + 100_000 + 1 + 32 * 32
```

This is separate from the recursive proof node tree. Hoon sees only `[len data]`
for the nonce and the structured recursive certificate.

## Work Transcript

For Pearl-format-compatible mode, Rust derives the mineable attempt with Pearl's
transcript:

```text
kappa = BLAKE3(sigma || mu)
H_A   = BLAKE3(pad(A_row_major), key=kappa)
H_B   = BLAKE3(pad(B_col_major), key=kappa)
s_B   = BLAKE3(kappa || H_B)
s_A   = BLAKE3(s_B || H_A)
hash  = BLAKE3(tile_state, key=s_A)
```

The nonce must not add a second Nockchain-only nonce into the attempt state.
Changing the Pearl header, mining configuration, public proof params, aux
commitment, Nockchain block commitment, or selected ticket offset changes the
work attempt.

Minimal work reuse is intentional. Cache-friendly retry loops are a soundness
risk. A miner must not be able to run one matrix/noise attempt and then grind
many independent Nockchain nonces against it.

## Nockchain Acceptance Contract

This section records the future acceptance contract so the current data shape
does not become under-specified. It is not part of the current implementation
milestone. At present Hoon remains fail-closed for `%ai-pow`, and the Rust
boundary used by this branch performs metadata-only decode/precheck work before
submission.

When the real verifier is explicitly in scope, Nockchain-side verification must
perform these checks before recursive proof verification:

1. Bound the jammed block artifact before cueing.
2. Cue canonically and reject noncanonical jam.
3. Decode only `%ai-pow`.
4. Decode `ai-pow-nonce` as `[len data]` and reject length mismatches,
   oversized nonce bytes, malformed `AIP1`, malformed `PMP1`, malformed
   `NPA1`, malformed coinbase bytes, oversized coinbase bytes, oversized merkle
   branches, and trailing bytes.
5. Verify the aux inclusion proof against the Pearl header merkle root.
6. Verify the `NPA1` aux block commitment equals the trusted candidate
   Nockchain block commitment.
7. Recompute Pearl-compatible `H_A`, `H_B`, noise seeds, tile state, and
   jackpot digest from trusted matrices and public statement bytes.
8. Verify the jackpot digest satisfies the Nockchain target.
9. Do not require the jackpot digest to satisfy Pearl's target for Nockchain
   acceptance.
10. Verify recursive certificate metadata matches the recomputed work:
    `zk_params`, `found_idx`, `trace_height`, `h_a_chunk`, `h_b_chunk`,
    `JOB_KEY = kappa`, `COMMITMENT_HASH = s_A`, `JACKPOT_MSG = tile_state`,
    and `HASH_JACKPOT = jackpot_hash`.
11. Only after those cheap checks, reconstruct and verify the recursive
    certificate.

## Current Implemented Surface

Implemented in this branch:

- `ai_pow::pearl_compat` serializes and parses Pearl header/config/public data,
  `NPA1` aux, and `PMP1` public statements.
- The Rust precheck now treats Pearl and Nockchain targets independently for
  Nockchain submission: Nockchain-side precheck requires the Nockchain target
  only.
- `mine_pearl_merge_ticket_attempt` returns a ticket before ZKP work only when
  the Nockchain target is hit.
- `ai_pow_miner::run` now requires an explicit Rust-only
  `PearlMergeSubmissionConfig`. The connected node run loop derives the
  Nockchain candidate commitment, builds a coinbase-only Pearl-format aux
  inclusion, mines Pearl-compatible ticket attempts, and constructs the
  recursive proof-node payload only after a ticket hits the Nockchain target.
- The connected run loop submits only the Nockchain `%ai-pow` command. It does
  not submit Pearl blocks, and the Hoon kernel still receives no Pearl-specific
  fields beyond the Rust-owned opaque nonce bytes.
- The Pearl-compatible run loop accepts recursive certificate data only through
  a wrapper constructible by public callers from the opaque
  `AiPowRecursiveCertificateRun` returned by the recursive prover. Downstream
  crates cannot synthesize this run object directly. Before wrapping the
  command, the miner rechecks the run's `zk_params`, `found_idx`,
  `trace_height`, commitments, and bound public inputs against the
  ticket-derived metadata, so a stale or wrong-ticket recursive run is rejected
  before it is submitted to the node.
- `ai-pow-mine` no longer has a submission-mode switch. It requires the
  operator to provide the Pearl header fields that define the shared transcript
  (`--pearl-prev-block`, `--pearl-timestamp`, and `--pearl-nbits`) and derives
  the Rust-only Pearl mining config from the canonical recursive AI-PoW params.
  The legacy NCMN miner and prover-only smoke CLI were removed so downstream
  callers cannot accidentally treat them as production submission APIs.
- Miner preflight rejects configurations without Pearl submission config before
  enabling mining. There is no mixed-mode branch in the connected run loop.
- Pearl-compatible miner preflight rejects Rust-side submission configs whose
  `common_dim`, `rank`, recursive params, or row/column patterns do not match
  the configured AI params and the current square-contiguous recursive prover
  subset. The current subset requires `difficulty_bits = 0` because the
  Nockchain target is verifier-supplied, and `spot_checks = 1` because the
  Pearl-compatible recursive statement proves one explicit ticket. This keeps
  unsupported Pearl pattern-language and parameter configs from reaching the
  mining loop and failing only after a target hit.
- `ai_pow_miner::certificate_noun` emits canonical `%ai-pow` artifacts with an
  opaque `[len data]` nonce and structured recursive certificate.
- Public certificate-noun construction is typed around
  the opaque `AiPowRecursiveCertificateRun`; generic serde proof-node
  serializers and raw-node certificate builders are crate-internal
  test/plumbing helpers, not production submission APIs.
- Hoon-shaped Pearl public-statement noun builders/decoders are test-only
  Rust plumbing. Production block submission carries the Pearl statement only
  inside the opaque `AIP1` nonce bytes.
- The Rust metadata APIs parse the opaque nonce back into Pearl-format
  statement and aux inclusion evidence before any recursive proof-node work.
- The Rust noun boundary exposes a metadata-only `%ai-pow` decode/precheck
  path for Pearl-format-compatible artifacts, so malformed nonce bytes,
  candidate-block replay, aux inclusion tamper, target misses, and recursive
  metadata drift are rejected before recursive proof-node traversal.
- The Rust noun boundary also exposes a metadata-only command-shape entrypoint
  for the exact Nockchain submission payload
  `[%command %pow [%ai-pow nonce cert]]`. This is deliberately not a real
  verifier: it parses the command wrapper and runs cheap metadata checks only,
  leaving recursive proof verification disabled in Hoon for now.
- The Rust noun boundary also exposes a slab-level metadata-precheck entrypoint
  for future verifier integration. It accepts a `%ai-pow` artifact noun
  plus trusted Nockchain context and performs the same cheap metadata checks.
  This branch does not wire or pursue the real verifier.
- The trusted Nockchain verifier context is a named Rust struct rather than a
  loose argument list. It contains the candidate block commitment, matrix
  operands, Nockchain target, and Pearl pattern bound, all of which must be
  derived outside the miner-controlled artifact.
- Size-budget tests pin the maximum `AIP1` nonce envelope at 101,424 bytes,
  reject nonce bytes above that cap, and assert a worst-case nonce plus small
  structured certificate jams below 110 KiB.
- The opt-in real recursive certificate harness currently measures a
  representative structured recursive certificate noun at 190,510 jammed bytes
  (186.04 KiB) and the postcard-encoded L1 certificate at 125,089 bytes
  (122.16 KiB) in release mode. It asserts budget caps of 256 KiB and 160 KiB,
  respectively, while the final production proof shape is still settling.
- Hoon exports only `ai-pow-nonce`, `ai-pow-certificate`, and
  `ai-pow-artifact` concepts. No Pearl-specific molds are exported.
- Dumbnet consensus recognizes only `%ai-pow` for AI proof version `%3` and
  remains fail-closed. Real verifier wiring is out of scope for this
  milestone.

Release checks run for this state:

```text
GNORT_DISABLE=1 cargo test -p ai-pow-miner --release --features node -- --nocapture
GNORT_DISABLE=1 cargo test -p ai-pow --release --features zk --test pearl_merge_compat -- --nocapture
```

## Remaining Fix Plan

1. Keep Hoon `%ai-pow` fail-closed and keep real verifier work out of this
   milestone.
2. Keep any future verifier call surface generic in the design: opaque nonce
   bytes, structured certificate, trusted candidate block commitment, target,
   params, and verifier context flow into Rust when that work is explicitly
   scheduled.
3. Decide the production chain-id and extra-domain-data policy for `NPA1`.
4. Done for this milestone: Nockchain production requires coinbase-only
   Pearl-format block templates (`merkle_branch_len = 0`). Revisit only if a
   future milestone deliberately supports Pearl transaction merkle trees.
5. Re-run and tighten real recursive certificate size-budget caps after the
   final production proof shape is fixed.
7. Keep metadata-precheck tests covering malformed `AIP1`, `PMP1`, `NPA1`,
   candidate-block replay, aux inclusion tamper, target miss, metadata drift,
   and proof-node DoS limits without wiring Hoon acceptance.
8. Extend the recursive prover beyond square-contiguous Pearl row/column
   patterns, or keep production admission explicitly restricted to that subset.

## Non-Negotiable Requirements

- The persisted and wire-transmitted Nockchain artifact is `%ai-pow`.
- Hoon must not define or dispatch on Pearl concepts.
- Hoon must not accept a Pearl ZKP, raw `MatmulProof`, or nonrecursive proof as
  the production AI-PoW certificate.
- One work attempt must correspond to one target check per chain. No grinding
  a fresh Nockchain nonce against cached Pearl work.
- Nockchain target satisfaction is sufficient for Nockchain submission; Pearl
  target satisfaction must not be enforced by Nockchain-side submission.
- Cheap replay, target, aux inclusion, metadata, and size checks must happen
  before recursive proof reconstruction or verification.
