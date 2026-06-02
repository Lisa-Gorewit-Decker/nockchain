# Pearl-Compatible Nockchain AI-PoW Submission Specification

Date: 2026-06-01
Status: Pearl-compatible dual submission plumbing in progress

## Goal

Nockchain AI-PoW should support a Pearl-format-compatible work attempt so one
miner can commit to a Pearl block candidate and a Nockchain block candidate,
evaluate one Pearl-style useful-work attempt, and submit the result to the
chain whose target was satisfied. A Nockchain hit submits the canonical
recursive `%ai-pow` certificate to the Nockchain node. A Pearl hit submits a
Pearl `PlainProof` to Pearl Gateway via `submitPlainProof`.

The chains do not need to share proof systems. Nockchain should use its own
recursive certificate and verifier. Compatibility is at the mineable work layer:
the same Pearl-style `sigma`, `mu`, matrix commitments, noise seeds, ticket
tile state, and jackpot digest must be used.

For the current milestone, the Hoon/kernel surface remains Nockchain-only while
the Rust miner also wires Pearl Gateway submission. That means:

- Nockchain accepts only the canonical `%ai-pow` block proof arm.
- Hoon does not define Pearl-specific molds or dispatch arms.
- Pearl-format details live inside Rust-owned nonce bytes.
- Nockchain acceptance requires the shared jackpot digest to satisfy the
  Nockchain target.
- Nockchain acceptance does not require the shared jackpot digest to satisfy
  Pearl's `nbits` target.
- Pearl submission does not build or submit a Nockchain recursive certificate
  unless the same attempt also satisfies the Nockchain target.
- The attempt does not need to satisfy both targets. If only Pearl's adjusted
  target is hit, submit only to Pearl. If only Nockchain's target is hit,
  submit only to Nockchain. If both are hit, submit both.

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

Rust miner default policy for this milestone:

- `chain_id` defaults to `nockchain` and may be overridden with
  `--pearl-nockchain-chain-id`. The final protocol network/domain string can be
  changed centrally in the Rust miner default without changing Hoon types or the
  `%ai-pow` noun shape.
- `target_epoch_or_height` defaults to zero and may be overridden with
  `--pearl-nockchain-target-epoch-or-height`.
- `extra_domain_data` remains optional bounded bytes for deployments that need
  additional replay protection beyond chain id, candidate commitment, and target
  epoch/height.
- Matrix inputs default to deterministic local smoke-profile synthesis with
  seed `ai-pow-prod-v1` when neither `--a + --b` nor `--synth-seed` is
  supplied. Explicit raw matrix paths must be provided as a complete pair.
- Pearl work headers default to Pearl Gateway miner RPC `getMiningInfo` over
  Unix socket `/tmp/pearlgw.sock`, matching Pearl Gateway's default miner-RPC
  configuration. TCP gateway mode is available with explicit host/port flags.
  Manual Pearl header flags are retained only as an explicit development
  fallback via `--pearl-work-source manual`.
- Pearl Gateway requests use a bounded request timeout, defaulting to 2000 ms
  and configurable with `--pearl-gateway-timeout-ms`. Zero is rejected so a
  silent, wedged, or malicious local Gateway cannot block candidate processing
  indefinitely.
- Pearl Gateway JSON-RPC responses are read as a single bounded line, capped at
  64 KiB before JSON parsing. This is intentionally far above the expected
  `getMiningInfo` response size but prevents a local Gateway from forcing
  unbounded miner allocation by streaming data without a newline.
- In Gateway mode, the miner refreshes `getMiningInfo` while a Nockchain
  candidate remains current. The refresh interval defaults to 1000 ms and is
  configurable with `--pearl-gateway-refresh-ms`; zero is rejected. If the
  refreshed Pearl incomplete header changes, the miner cancels the current
  ticket loop and restarts work for the same Nockchain candidate using the new
  Pearl header. After a solution is turned into a Nockchain poke attempt, the
  cached candidate is cleared and Gateway refresh does not redispatch that
  solved candidate; the miner waits for the node to emit a new candidate.
  Manual/static header mode does not refresh.
- Gateway-backed merge work asks `getMiningInfo` for a Pearl block template
  that already contains the Nockchain aux commitment. The request carries
  generic Pearl-side `coinbase_aux_flags`, encoded as standard base64 of
  `NOCKCHAIN-AI-POW-AUX || aux_commitment`, plus
  `return_aux_inclusion=true`. A compatible Gateway returns an
  `aux_inclusion` object containing standard-base64 `coinbase_tx` and a
  standard-base64 `merkle_branch` list. The Rust miner verifies that returned
  proof against the returned incomplete header before using the work. Hoon
  still receives only the opaque nonce and recursive certificate.
- Pearl Gateway submission uses `submitPlainProof` with the Pearl wire format:
  `plain_proof` is standard base64 of Pearl's `bincode 1.3.3`
  `PlainProof`, and `mining_job` contains the base64 incomplete header bytes
  plus the exact target JSON integer returned by Gateway's `getMiningInfo`.
  Miners use the adjusted Pearl target for local hit detection, but the
  `submitPlainProof` `mining_job` echoes Gateway's original job target. The
  proof contains the two matrix Merkle proofs for `A` and `B^T`; it is not a
  Nockchain block artifact and is never serialized into Hoon.
- Gateway acceptance is header-template sensitive. Pearl Gateway's async
  handler compares `mining_job.incomplete_header_bytes` with its current block
  template's `serialize_without_proof_commitment()` and silently skips old or
  different headers after returning `"submitted"`. Therefore production
  merge-mining requires the Pearl Gateway template to already commit to the
  same aux-bearing Pearl header that the Nockchain attempt used. The Rust miner
  now requests that aux-bearing work through the `getMiningInfo` extension
  above. If a Gateway accepts the request but omits `aux_inclusion`, the miner
  can still build a coinbase-only aux proof for Nockchain-side
  self-verification, but Pearl-side `submitPlainProof` remains fail-closed
  unless the Gateway-issued header equals the mined aux-bearing header. This
  avoids false success from Gateway's pre-async `"submitted"` acknowledgement.

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
- `ai_pow_miner::pearl_mining` evaluates one explicit Pearl ticket attempt per
  counted attempt and returns when either the Pearl adjusted target or the
  Nockchain target is hit. The returned ticket records
  `pearl_target_hit` and `nockchain_target_hit`, so the run loop can submit to
  the appropriate chain without treating one chain's target as the other's
  admission rule.
- `ai_pow_miner::run` now requires an explicit Rust-only
  `PearlMergeSubmissionConfig`. The connected node run loop derives the
  Nockchain candidate commitment, builds a coinbase-only Pearl-format aux
  inclusion, mines Pearl-compatible ticket attempts, submits Pearl hits to
  Gateway when configured, and constructs the recursive proof-node payload only
  after a ticket hits the Nockchain target.
- `ai_pow_miner::pearl_plain_proof` builds Pearl's `PlainProof` from the mined
  attempt, the trusted matrices, and Pearl matrix Merkle proofs, then serializes
  it as standard base64 of Pearl-compatible `bincode 1.3.3` bytes for Gateway
  `submitPlainProof`.
- The connected run loop submits the Nockchain `%ai-pow` command only for
  Nockchain target hits. Pearl-only hits do not build a recursive certificate
  and do not poke Hoon. The Hoon kernel still receives no Pearl-specific fields
  beyond the Rust-owned opaque nonce bytes.
- The Pearl-compatible run loop accepts recursive certificate data only through
  a wrapper constructible by public callers from the opaque
  `AiPowRecursiveCertificateRun` returned by the recursive prover. Downstream
  crates cannot synthesize this run object directly. Before wrapping the
  command, the miner rechecks the run's `zk_params`, `found_idx`,
  `trace_height`, commitments, and bound public inputs against the
  ticket-derived metadata, so a stale or wrong-ticket recursive run is rejected
  before it is submitted to the node.
- `ai-pow-mine` no longer has a submission-mode switch. It always builds
  canonical Pearl-format-compatible Nockchain `%ai-pow` submissions. By
  default it fetches the Pearl incomplete block header from Pearl Gateway
  miner RPC `getMiningInfo`; `--pearl-work-source manual` keeps explicit
  header flags for tests and local development. Gateway fetches use an
  explicit TCP connect timeout plus socket read/write timeouts so local Gateway
  failure is a skipped candidate, not an unbounded miner stall. The miner
  also polls Gateway while a Nockchain candidate is current and redispatches
  the ticket loop if the Pearl header changes. The miner derives the Rust-only
  Pearl mining config from the canonical recursive AI-PoW params. If no matrix
  paths or custom `--synth-seed` are supplied, the CLI uses the default
  `ai-pow-prod-v1` local smoke-profile matrices; the remaining required local
  operator input is the mining key configuration. Once the miner builds a
  `%ai-pow` poke for a candidate and attempts to send it to the node, it clears
  the cached candidate so later Pearl Gateway template changes cannot produce
  duplicate submissions for the same Nockchain candidate. Pearl-only Gateway
  hits also clear the cached Nockchain candidate for now to avoid resubmitting
  the same Pearl hit against a Gateway that acknowledges before async stale
  header checks complete.
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
3. Done for this milestone: `NPA1.chain_id` has an overrideable Rust miner
   default, `target_epoch_or_height` defaults to zero, and `extra_domain_data`
   is optional bounded bytes for deployment-specific replay protection.
4. Done for this milestone: Nockchain production requires coinbase-only
   Pearl-format block templates (`merkle_branch_len = 0`). Revisit only if a
   future milestone deliberately supports Pearl transaction merkle trees.
5. Done for this milestone: `ai-pow-mine` defaults to Pearl Gateway miner RPC
   as its Pearl work-header source, with manual headers retained only as an
   explicit development fallback.
6. Done for this milestone: Pearl Gateway header fetches have bounded request
   timeouts and bounded response-line reads to avoid local Gateway denial of
   service during candidate processing.
7. Done for this milestone: Pearl Gateway work is refreshed while a Nockchain
   candidate remains current, and changed Pearl headers supersede stale ticket
   loops for that candidate.
8. Done for this milestone: `ai-pow-mine` defaults missing matrix input to the
   `ai-pow-prod-v1` local smoke-profile synth seed while preserving explicit
   complete `--a + --b` matrix input for deployments that supply real matrices.
9. Done for this milestone: after a successful Nockchain `%ai-pow` submission,
   the run loop clears the cached candidate so Pearl Gateway refresh cannot
   redispatch solved work. Only a fresh Nockchain candidate restarts mining.
10. Done for the Rust client path: Pearl Gateway `submitPlainProof` plumbing,
    Pearl-compatible `PlainProof` serialization, aux-bearing `getMiningInfo`
    requests, and returned `aux_inclusion` verification exist in Rust. Complete
    production Pearl-side acceptance requires deploying a Pearl Gateway server
    with the matching `getMiningInfo` extension so it issues the exact
    aux-bearing incomplete header used by the Nockchain attempt.
11. Re-run and tighten real recursive certificate size-budget caps after the
   final production proof shape is fixed.
12. Keep metadata-precheck tests covering malformed `AIP1`, `PMP1`, `NPA1`,
   candidate-block replay, aux inclusion tamper, target miss, metadata drift,
   and proof-node DoS limits without wiring Hoon acceptance.
13. Extend the recursive prover beyond square-contiguous Pearl row/column
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
