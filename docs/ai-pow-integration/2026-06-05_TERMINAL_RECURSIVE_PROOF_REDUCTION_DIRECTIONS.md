# AI-PoW Recursive Proof Reduction Directions

Date: 2026-06-05
Status: accepted decision checkpoint, revised after stack-level integration
audit and 2026-06-07 route decision. The exhaustive-NPO terminal fixture passes
the byte and time gates, but the full `ai-pow-zk` composite-verifier terminal
path has not yet met either the production byte gate or the production time
gate. The compact final-layer batch-STARK route is now the selected primary
production-proof direction because the corrected fast-L1/L2 sweep hits the
relaxed final-proof size target with explicit public-value binding. Native
terminal remains the fallback route if this path cannot meet the time gate. The
large batch-STARK checkpoint envelope remains too large.

## Goal

The production recursive proof target is a compact recursive certificate, not
the current full batch-STARK L1 checkpoint envelope. The original hard target
is approximately `100 KiB` for the recursive proof and `<30s` release proving
time. The active engineering target is the maintainer-relaxed milestone of
about `150 KiB` total recursive proof size and about `30s` total release
proving time. Every candidate, native terminal or compact batch-STARK, must
preserve soundness without letting a miner skip the AI-PoW matrix-multiplication
work and without relying on undocumented or unproven shortcuts.

## Current Most Viable Path (2026-06-07)

### Important Current Production-Proof Summary

This is the live source of truth for the current production recursive-proof
direction. The `ai-pow-zk` README links here because changes to the recursive
proof path, certificate wire format, FRI parameters, terminal commitment shape,
or packed Tip5/NPO bridge code must be checked against this summary first.

Route decision, 2026-06-07: accept compact final-layer batch-STARK L2 over a
fast statement-bound L1 proof as the primary production direction. The goal is
not to force the native terminal backend if the batch-STARK route is smaller,
faster, and has the cleaner soundness/binding story. Native terminal remains a
fallback only if compact batch-STARK cannot be brought under the proving-time
gate without weakening the proof. The large full-checkpoint batch-STARK
envelope remains rejected for production wire use because it is too large. New
implementation work should advance this compact L2 route first unless a later
measurement or soundness finding falsifies it.

The relaxed milestone is not yet fully claimed. The target is approximately
`150 KiB` total recursive-proof size and approximately `30s` total release
proving time, with 60 bits of soundness coming from FRI queries rather than
proof-system grinding. Compact batch-STARK L2 is now the committed primary
production-proof direction. Native terminal remains the fallback if compact
batch-STARK proving time cannot be reduced without invasive changes.

#### Clean Checkpoint

The most viable route is the Pearl-shaped two-layer STARK compression path:
build a statement-bound L1 proof, then produce a compact final-layer L2
batch-STARK proof whose verifier owns setup/metadata and whose final public
lanes bind the L1 statement digest. The large batch-STARK recursive checkpoint
certificate is still too large; the candidate is only the compact L2 body plus
the explicitly required public values and verifier-key/setup binding. The
native terminal production profile remains a fallback at pure-query FRI
soundness with terminal Merkle cap height `3`.

The native-terminal fallback base is close but incomplete: the cap-height `3`
merged-only body is `142,807` bytes and is sound for its included primitive
R1CS and merged NPO value-bridge relations. It does not yet bind the internal
packed Tip5 AIR/LogUp/selected-trace work. Because binary `150 KiB` leaves only
`10,793` bytes above that base, the remaining terminal support binding cannot
be an appended standalone packed proof. That terminal fallback would need a
genuinely merged packed Tip5 theorem, an almost metadata-free support binding,
or a further base-proof reduction.

Done and verified:

- Compact batch-STARK L2 is now the selected primary production-proof
  direction. The full batch-STARK checkpoint remains a hardened
  checkpoint/fallback path, not the production wire artifact. Native terminal
  remains the fallback route.
- Batch-STARK opening shape now respects each AIR's declared next-row usage
  through the circuit-table and dynamic-AIR wrappers. The Tip5 circuit wrapper
  explicitly keeps main next-row openings because its inner lookup AIR links
  permutation rounds with `main.next_slice()`, while single-row Const/Public/
  Recompose and Tip5 preprocessed openings can omit unnecessary next-row
  values. The recursive verifier circuit and challenge-generation replay now
  accept the same optional trace/preprocessed-next shape as the native
  batch-STARK verifier. A focused wrapper regression passes, and the selected
  fast-L1/L2 release measurement verifies end to end.
- The corrected fast-L1/L2 compact batch-STARK diagnostic verifies with L1
  `lb=3,nq=20,cap=4,pow=0` and L2 `lb=5,nq=12,cap=4,pow=0`: actual compact
  wrapper `143,762` bytes, metadata-free compact body `142,878` bytes, core
  compact proof `90,307` bytes, restoration payload `52,571` bytes. The full
  three-row sweep measured L1 prove `30.448s` and L2 prove `54.137s`; the
  focused selected-row rerun measured L1 prove `24.865s` and L2 prove
  `48.281s`. This is the first soundly bound row inside the relaxed `150 KiB`
  size target, but it still misses the total `~30s` proving-time target.
- The selected L2 path now has a reusable verifier-prep cache in the diagnostic
  harness. It separates verifier-circuit definition/build, canonical
  AIR/prover-data setup, verifier input packing, witness execution, and STARK
  proving while reusing the canonical setup and prover for a fixed L2 proof
  shape. `prove_all_tables` now reuses compatible cached `ProverData` instead
  of rebuilding the same preprocessed commitment/prover data per proof; it
  falls back to the old rebuild path when runtime lookup/preprocessed metadata
  shape differs. After next-row opening forwarding, the latest release/native
  selected rerun measured L1 prep wall `4.772s`, cached L1 prove `15.305s`,
  total L1 prove `20.077s`, L2 prep wall `9.364s`, cached L2 prove `28.726s`,
  and uncached L2 total `38.090s`. The cached L1 proof was witness run `58ms`
  and STARK prove `15.246s`; the cached L2 proof was input packing `0ms`,
  witness run `38ms`, STARK prove `28.667s`, and STARK self-verify `19ms`.
  The selected L2 proof is now under `30s` by itself, but cached serial
  L1+L2 proving is still about `44.031s` with both prep stages cached. The
  setup-included serial L1+L2 time is `58.167s`. The prior deep
  `AI_POW_ZK_DEEP_BATCH_PROFILE=pcs` profile, run before next-row forwarding,
  identified the L2 bottleneck as main/permutation trace Merkle commitments
  rather than recursive-verifier witness execution, reusable setup, quotient
  evaluation, or FRI query work. The remaining total-time lever is now specific:
  reduce or overlap L1 proving, and shrink committed recursive-verifier matrix
  volume, especially the Tip5/MMCS verifier-table main and permutation traces.
- The selected compact L2 table-packing sweep now verifies low/default/high ALU
  packing variants through the same verifier-owned compact context. It rules
  out simple ALU lane retuning as the time lever. Baseline
  `alu_lanes=8,horner_k=5` remains the best time row in the same run: compact
  wrapper `143,762` bytes, metadata-free body `142,878` bytes, cached L2 prove
  `28.530s`, cached serial L1+L2 `44.198s`. Smaller ALU lanes give real size
  margin but not time: `alu_lanes=4` verifies at `133,736` bytes compact /
  `30.875s` cached L2, and `alu_lanes=2` verifies at `126,862` bytes compact /
  `30.801s` cached L2. Larger lanes are worse: `alu_lanes=16` verifies at
  `163,875` bytes / `30.945s`, `alu_lanes=32` at `203,804` bytes / `36.279s`,
  and `alu_lanes=16,horner_k=8` at `167,211` bytes / `32.108s`. The AIR
  layouts explain the tradeoff: low lanes reduce ALU width (`lanes=2` uses
  `38` main columns) but increase ALU rows (`23,136`), while high lanes keep
  rows flat at `5,960` and widen columns (`150` or `278` main columns). This is
  a useful emergency size knob, not a route to the `~30s` total target.
- The selected compact L2 over L1 table-packing sweep now verifies that the
  inner L1 proof's table-packing choice is also bound through the same
  verifier-owned L2 context and explicit final public values. This is the
  closest measured complete compact batch-STARK route so far. L1
  `alu_lanes=4,horner_k=5` gives compact wrapper `143,106` bytes,
  metadata-free body `142,225` bytes, cached L1 prove `15.029s`, cached L2
  prove `28.555s`, and cached serial L1+L2 `43.584s`. L1
  `alu_lanes=2,horner_k=5` is the smallest measured row at `141,148` bytes
  compact / `140,260` bytes body, but cached serial L1+L2 is `44.391s`.
  Baseline L1 `alu_lanes=8,horner_k=5` remains close at `143,762` bytes and
  `43.893s` cached serial in the same run. L1 `alu_lanes=16` is worse on both
  size and time for the relaxed route (`149,688` bytes, `45.773s`). This
  supports committing to compact batch-STARK as the primary production route,
  while also showing that packing alone is not the remaining time lever.
- A selected L2 verifier trace profile now quantifies the Tip5 boundary. The
  L2 verifier circuit has `75,391` ops: `66,564` primitive ops, `1,981` hints,
  and `6,846` non-primitive ops. Non-primitives split into `4,791` Tip5 rows,
  `2,001` `recompose/coeff` rows, and `54` regular recompose rows. The Tip5
  table is padded to `8,192` rows, so the selected route needs to remove only
  `695` Tip5 rows to halve the dominant Tip5 table height to `4,096`. The
  compiled Tip5 calls split into `2,220` MMCS private-data op IDs and `2,571`
  non-MMCS rows; the generated trace has `2,620` rows carrying explicit
  `mmcs_bit` bindings. A new full phase-tag profile accounts for all `4,791`
  rows: `2,220` Merkle-path sibling compressions, `1,620` base-field MMCS leaf
  hashes, `400` path digest-injection compressions, `180` extension-element
  MMCS leaf hashes, and `371` Fiat-Shamir challenger rows. A profile-only
  circuit that skips deterministic preprocessed opened-value transcript
  observations saves only `70` Tip5 rows (`4,791 -> 4,721`) and still pads to
  `8,192`, so a verifier-key/setup-digest transcript by itself does not cross
  the `4,096` boundary. L1 query-count retuning is also negative as a total-time
  lever: L1 `lb=4,nq=15` cuts selected-L2 Tip5 rows to `3,851` and pads the
  Tip5 table to `4,096`, but the full timing diagnostic still measures cached
  L2 proving at `28.743s` while cached L1 proving rises to `29.584s`; the
  compact wrapper is `142,649` bytes, but cached serial L1+L2 proving is about
  `58.327s`. A hidden-L1 cap-height profile does not provide a cheap crossing:
  cap `3` has `4,967` Tip5 rows, cap `4` has `4,791`, and cap `5/6` L1 proofs
  verify natively but the current L2 recursive verifier rejects them with
  witness conflicts. Cap retuning is therefore not a production lever until the
  higher-cap recursive-verifier support gap is fixed, and even then the cap
  `3 -> 4` slope suggests it will not remove `695` rows without other changes.
- The smaller L2 `lb=6,nq=10` row verifies at `132,682` bytes actual compact
  wrapper and `131,803` bytes metadata-free body, but L2 proving rises to
  `107.617s`. The faster L2 `lb=4,nq=15` row proves in `27.342s` but measures
  `178,272` bytes, so it needs additional proof-body compression before it can
  satisfy the relaxed size gate.
- A focused fast-L1 `lb4,nq15` frontier sweep rules out cheap FRI-shape or cap
  tuning as the way to combine the faster L2 time with the relaxed size gate.
  The best measured `lb4,nq15` compact wrapper is `174,676` bytes
  (`lfp=2,mla=4,cap=4`) with `24.456s` L2 proving, still about `24.7 KiB`
  above a decimal `150,000` byte gate and about `24.7 KiB` above the current
  selected wrapper.
- Production terminal cap height `3` is implemented, transcript-bound, and
  release/native measured. The best measured merged-only structural floor is
  `142,807` bytes.
- Full multi-root FRI caps are digested into the terminal prelude commitment
  list while the FRI verifier still observes the full cap.
- The packed Tip5 checkpoints verify independently: compact packed AIR,
  packed byte-table LogUp, lane-selector-aware selected-to-packed NPO bridge,
  direct selected-to-packed-trace bridge, and the shared packed-trace
  AIR+LogUp+selected-trace support theorem.
- Negative measurements rule out simple append/fusion: the shared support
  theorem is still `198,287` bytes / `33.277s`, and the cap-height `3`
  optimistic single-FRI merged-plus-support floor is `249,184` bytes.
- Paired 16-bit lookup is measured as a useful component but not a primary
  route: the optimistic floor remains `219,138` bytes before new table/domain
  overhead.
- Even a zero-support-FRI model does not fit with current metadata:
  `156,696` bytes with current support metadata or `152,612` bytes after the
  paired-lookup non-FRI opening estimate, leaving only `988` bytes before any
  sound support FRI payload or excluded overhead.
- Verifier-derived profiles are omitted from the serialized packed
  AIR+LogUp+selected-trace support proof; the verifier recomputes the expected
  profiles and uses them for transcript seeding, domains, opening dimensions,
  and relation checks. Focused round-trip validation passes, and the
  release/native fusion-floor diagnostic remeasures the packed-support non-FRI
  payload at `16,065` bytes.
- Verifier-derived profiles are also omitted from the serialized merged
  residual-zero/recompose/value-bridge proof; the verifier recomputes them
  from the VK and uses the expected profiles for transcript seeding, domains,
  opening dimensions, returned opened-column profile, and relation checks.
- Sumcheck `eval_1` values are omitted from serialized primitive R1CS matrix
  and row-product sumcheck rounds. The verifier reconstructs them from
  `eval_0 + eval_1 = current_claim` before Fiat-Shamir challenge derivation;
  focused primitive round-trip tests and the release/native fusion-floor
  diagnostic pass.
- Packed-support bridge final cumulatives on the packed side are omitted from
  serialization. The verifier reconstructs them as the additive inverse of the
  selected-side bridge finals, absorbs the reconstructed values before gamma
  challenge sampling, and rejects inconsistent non-empty in-memory packed
  finals. Focused round-trip/rejection validation passes.
- Outer task parallelism has been measured, not just estimated. A Rayon-joined
  diagnostic proves the primitive R1CS, merged value-bridge, and packed-support
  subproofs from the same prepared prelude in parallel. It leaves proof bytes
  unchanged and measures `39.448s` parallel subproof wall time, dominated by the
  packed-support branch, versus a `53.355s` sum of the three subproof timers.
  The full diagnostic wall remains `171.422s`, so parallelizing the existing
  arguments is useful cleanup but is not enough for the `~30s` production goal.
- The compact batch-STARK L2 candidate now treats verifier key/setup material
  the Pearl way: metadata is verifier-owned, preprocessed openings are restored
  from canonical setup, and the final L2 proof binds all L1 statement-digest
  base limbs as final public lanes. For the D=2 candidate this is
  `DIGEST_ELEMS * TRACE_D = 10` base-valued L2 public lanes; the verifier API
  receives their D=2 basis expansion. The compact body now verifies through a
  single verifier-owned context containing the metadata template, canonical
  setup/prep data, expected FRI shape, and public values. Focused regressions
  reject wrong public values, wrong FRI shape, and metadata/setup mismatches.
- The selected L2 diagnostic now builds reusable verifier-prep/cache material
  and proves through that cache, mirroring the generic recursion
  `NextLayerPrepCache` pattern. The circuit prover now also reuses compatible
  cached `ProverData` during `prove_all_tables`, eliminating the repeated
  preprocessed-commitment rebuild for the selected shape. This is measured and
  verified in the release/native selected timing test. The selected route is
  now promoted inside `ai-pow-zk` as `AiPowCompactBatchRecursiveCertificate`,
  `AiPowCompactBatchVerifierContext`,
  `prove_compact_batch_recursive_certificate_from_chain_verified_composite_proof`,
  and encode/decode/context-verifier helpers. The promoted crate-level
  certificate carries a Tip5 verifier-key/setup digest plus the final L2 compact
  body. The release/native round-trip measures `142,274` encoded certificate
  bytes, `19.223s` L1 outer proving, `9.440s` L2 prep, `29.743s` L2 proving,
  `37ms` compact verification, and `58.603s` uncached proof wall; decoded
  verification passes, wrong public inputs reject, wrong certificate digest
  rejects, and stale context digest rejects. The Rust bridge now exposes
  compact recursive run builders, and the miner's selected Pearl-compatible
  recursive builder now packages the compact certificate as canonical postcard
  bytes inside a bounded proof-node atom. A release/native miner artifact
  measurement now confirms the final Pearl-compatible `%ai-pow` noun boundary
  stays in range: jammed artifact `141,916` bytes (`138.59 KiB`), compact
  certificate `141,103` bytes (`137.80 KiB`), with bounded decode, statement
  precheck, and canonical compact byte-node re-encoding all passing. Hoon
  verifier wiring and production-pinned expected digest selection are still
  open.

What remains:

- Finish the Hoon-facing verifier path for the promoted compact batch-STARK
  certificate. The crate-level certificate/context API now exists, and the
  Rust bridge plus miner noun builder now package canonical compact bytes. The
  remaining verifier work is to derive/pin the verifier-key/setup digest,
  derive public values from chain-owned statement metadata, and reject any
  prover-supplied verifier metadata instead of accepting it as context.
- Reduce end-to-end proving time. The current selected size row is about
  `58.167s` serial L1+uncached-L2 proving in the focused baseline timing run
  (`20.077s + 38.090s`). With measured L1 and L2 prep caches, the comparable
  baseline per-attempt time is about `44.031s` (`15.305s + 28.726s`), and the
  best measured L1-packing row improves that only to `43.584s` (`15.029s +
  28.555s`). This is the best measured in-size route so far, but it still
  misses the `~30s` target.
- Move the crate-level L2 prep/cache and verifier-owned context into the
  production prover/verifier-key path. The test harness now proves that
  reusable setup is available and that compatible cached `ProverData` can be
  reused for the fixed selected shape, but production still needs canonical
  cache construction, setup digest pinning, and Hoon/verifier integration.
- Reduce core batch-STARK proving time. After cached setup and cached
  `ProverData` reuse, `28.667s` remains in L2 STARK proving and `15.246s`
  remains in cached L1 STARK proving. Reaching `~30s` total requires a real L1/L2
  batch-STARK/PCS reduction rather than only setup caching, quotient
  optimization, or recursive-witness optimization.
- Reduce the L2 STARK prover's dominant commitment spans rather than only
  cutting selected verifier rows. The phase-tag profile shows the selected-L2
  Tip5 rows are already fully explained by MMCS leaf/path hashing plus
  challenger rows. L1 `lb=4,nq=15` proves that halving the Tip5 table alone is
  not enough on this machine: cached L2 proving remains `28.743s`, and the
  slower L1 row makes the cached serial total worse. ALU lane retuning also
  does not solve time: lower lanes shrink compact bytes while raising L2 prove
  time, and higher lanes widen the ALU table. The next viable work must either
  remove real verifier operations, reduce both L1 and L2 commitment work
  together, overlap the L1/L2 stages, or change the recursive verifier proof
  shape so these large tables are not committed at the current L2 LDE volume.
- Count the final certificate bytes exactly, including any carried public-value
  limbs, verifier-key/setup digest, and chain-owned statement metadata that is
  not otherwise derivable by the verifier.
- Extend negative tests from the verified subproof checkpoints to the final
  compact batch-STARK artifact: stale verifier-key/setup digest, stale
  preprocessed caps/openings, wrong public values, noncanonical FRI shape,
  malformed compact FRI payloads, tampered path dictionaries, and
  transcript/prelude substitutions.
- Remeasure the full production path in release with native CPU codegen.

Non-claim: the relaxed size gate is now plausibly met for compact batch-STARK
L2, but the relaxed `~30s` total proving-time gate has not been met yet. Any
route that swaps production back to the large batch-STARK checkpoint envelope,
counts proof-system PoW grinding toward soundness, omits explicit statement
binding, accepts prover-supplied verifier metadata as trusted, or appends
standalone packed proofs is not a viable production route.

#### Most Viable Path

The current most viable production route is compact batch-STARK L2 over a fast,
statement-bound L1 proof. The target is the relaxed milestone of about
`150 KiB` total recursive proof size and about `30s` total release proving
time, while keeping soundness at 60 pure FRI query bits for both layers. The
current full batch-STARK checkpoint remains soundness-relevant but too large;
only the compact final-layer body with verifier-owned metadata and explicit
public-value binding is a plausible batch-STARK production candidate. Native
terminal reduction remains the fallback if this route cannot meet the time
gate.

The highest-signal immediate lever was to reduce the native-terminal base proof
before redesigning more Tip5 algebra. The current code checkpoint sets the
production terminal FRI/MMCS Merkle cap height to `3`, binds that value into
`TerminalProofParameters`, `TerminalProximityProfile`, and terminal
transcript/profile absorption before challenge sampling, and digests full
multi-root FRI caps into the terminal prelude commitment list. This is
soundness-neutral: it changes how much of each Merkle tree is sent as a cap
versus per-query authentication paths, while preserving the same FRI query
soundness target. The FRI verifier still observes the full cap commitment; the
prelude digest only binds that cap into the terminal transcript boundary.

The cap-height sweep brackets the tradeoff. Cap height `4` was best for the
pre-profile-elision oversized appended packed-support proof, but cap height `3`
gives the best measured structural headroom for the final route, where the
current support-FRI shape is rejected and must be redesigned. The cap-height
`3` row below is the latest release/native measurement after omitting
verifier-derived support-proof and merged-value-bridge profiles and primitive
sumcheck `eval_1` values; the other rows remain the prior cap-sweep brackets.

| Cap height | Current appended body | Optimistic single-FRI floor | Merged-only structural floor | Paired zero-support-FRI floor | Total subproof prove |
|---:|---:|---:|---:|---:|---:|
| 2 | `341,585` | `249,951` | `146,020` | `155,825` | `46.698s` |
| 3 | `336,210` | `249,184` | `142,807` | `152,612` | `46.134s` |
| 4 | `335,064` | `249,227` | `146,442` | `157,925` | `46.470s` |
| 5 | `345,711` | `258,780` | `151,420` | `165,191` | `53.182s` |

The separate cap-height `3` Rayon diagnostic runs the current three
post-prelude subproofs in parallel and measures `39.448s` wall time for that
join. The individual proof timers in that run sum to `53.355s` because the
packed-support branch consumes the whole parallel window. This is the best
measured "slap the viable arguments together and parallelize" timing floor for
the current proof language, and it still misses the full `~30s` production
pipeline before counting setup work.

A narrower cap-height `1` merged-value-bridge run measured `153,229` bytes for
the base body, so it is ruled out. The historical cap-height `0` fusion floor
was `252,753` optimistic single-FRI floor and `146,032` merged-only structural
floor. The retained cap height is therefore `3`: it does not make the current
support theorem viable, but it gives the final proof-shape work the largest
measured structural headroom under the production pure-query soundness profile.

Cap-height/base reduction does not create enough headroom by itself. The next
viable path is a genuinely merged packed Tip5 support theorem with almost no
additional serialized metadata. The latest fusion-floor diagnostic shows why a
support-only FRI shrink is not enough: the already-sound merged base is
`142,807` bytes at cap height `3`, leaving only `10,793` bytes under a binary
`150 KiB` target for all remaining Tip5 support binding. Current support
metadata with no support FRI at all would put the body at `156,696` bytes; the
paired-lookup metadata estimate with no support FRI would still put it at
`152,612` bytes, leaving only `988` bytes under binary `150 KiB` before paying
any sound support proof payload or the two-domain table/quotient overhead
excluded from the paired estimate. Therefore a successful fallback design must
either reduce the primitive/merged base, compress support metadata below the
paired-lookup floor, or merge the packed Tip5 relations so their serialized
metadata is not additive.

The paired 16-bit Tip5 lookup candidate is measured useful but insufficient as
the primary route. Pairing adjacent bytes as `x = b0 + 256*b1` and
`y = L(b0) + 256*L(b1)` estimates about `25.9 KiB` compact-FRI opened-value
savings and about `4.0 KiB` non-FRI zeta-opening savings, but the cap-height
`3` optimistic single-FRI floor remains `219,138` bytes before new two-domain
table overhead.
It should be treated only as a component of a larger proof-shape change.

The relations that still need to coexist in the final production theorem are
the merged terminal NPO value bridge, packed one-row-per-permutation Tip5 AIR
algebra, packed Tip5 lookup membership, and lane-selector-aware selected
NPO-value to packed trace-lane binding.

#### Done And Verified

- The merged residual-zero/recompose/padding/value-bridge checkpoint verifies
  over the actual full `ai-pow-zk` composite verifier relation at
  `142,807` bytes / `139.5 KiB` with `11.757s` primitive+merged subproof
  construction. It is sound for its included relations, including
  Merkle-direction-aware `mmcs_bit` value projection, but it intentionally does
  not yet bind internal Tip5 AIR/LogUp work.
- The compact packed Tip5 trace now omits duplicate explicit round-input
  columns for rounds 1-4, reducing the committed packed trace width from `500`
  to `436` while keeping those inputs bound as aliases of the previous round's
  output.
- The standalone packed Tip5 AIR algebra checkpoint verifies at `129,471`
  bytes and `19.875s` proving with the compact trace.
- The standalone packed byte-table LogUp checkpoint verifies at `155,974`
  bytes and `22.284s` proving with the compact trace.
- The standalone lane-selector selected-to-packed NPO-IO bridge verifies at
  `137,355` bytes and `28.526s` proving.
- The direct selected-to-packed-trace bridge verifies at `205,950` bytes and
  `35.863s` proving with the compact trace, deriving the 26 NPO-IO lanes
  directly from the opened packed trace instead of carrying a separate
  projection commitment.
- The naive projection-plus-selected bridge fusion verifies, but measures
  `243,516` bytes and `35.423s` proving, so it is negative evidence against
  simply batching more standalone proof components into one opening proof.
- The shared packed-trace AIR+LogUp+selected-trace bridge theorem verifies.
  Same-phase PCS-input coalescing and compact duplicate-input removal reduce
  PROD size from the uncoalesced `273,113` bytes / `266.7 KiB` to `198,287`
  bytes / `193.6 KiB`, with compact FRI down from `256,261` to `183,018`
  bytes. It still proves in `33.277s`, so it remains too large and slightly
  too slow as a standalone support theorem.
- The merged-value plus packed-support fusion-floor diagnostic verifies against
  one terminal circuit and one shared prelude. The current appended body is
  `336,210` bytes at cap height `3`. Even the deliberately optimistic floor
  that keeps only the larger compact FRI body and subtracts duplicate
  selected-lookup profile, commitment, and opening payload is `249,184` bytes.
  That is still `95,584` bytes over a binary `150 KiB` gate, so simple
  transcript/FRI sharing is ruled out as the relaxed-target route.
- The parallel-subproof variant of the same diagnostic verifies with unchanged
  proof-language size accounting. The Rayon join reports `39.448s` subproof
  wall time: primitive R1CS `10.619s`, merged value-bridge `3.287s`, and
  packed support `39.448s`, with `53.355s` total individual subproof time and
  `171.422s` total diagnostic wall. This rules out outer task parallelism
  alone as the route to the relaxed time target.
- The current packed trace and Tip5 spec confirm the shape of the next
  relation-level candidate: the active packed trace has five rounds, four
  split lanes, eight split bytes per lane, and width `436`; the split lanes use
  the fixed Tip5 `L` table, while lanes 4-15 use the separate `x^7` power map.
  Therefore a direct `x^7` replacement for split lanes is not sound; any
  smaller binding must still prove the fixed `L`-table semantics and its
  selected-value/round-link bindings.
- The paired 16-bit lookup payload estimate verifies inside the same
  fusion-floor diagnostic. Packed support FRI input batches are now attributed:
  selected lookup `20,314` bytes, packed trace/table `70,787` bytes, accumulators
  `20,813` bytes, quotients `11,252` bytes. The packed trace/table batch alone
  carries `62,311` opened-value bytes. Pairing bytes estimates the packed trace
  width floor at `276`, LogUp accumulator basis columns at `24`, and packed
  support compact FRI at `151,376` bytes before any two-domain table Merkle
  overhead or new quotient-shape cost. This rules out paired lookup as the
  primary route to the relaxed target.
- The structural lower-bound diagnostic now reports that the merged-only body
  is `142,807` bytes at cap height `3`. Current support metadata with no
  support FRI at all would put the body at `156,696` bytes; paired-lookup
  metadata with no support FRI would still put it at `152,612` bytes. The
  remaining solution cannot be
  "make support FRI smaller" alone; there is not enough metadata headroom.
- The terminal cap-height lever has been implemented in the current code
  checkpoint and bound into proof parameters, proximity profile, and transcript
  material. Multi-root FRI caps are now digested into the prelude commitment
  list instead of being rejected as non-root commitments, while the full cap is
  still observed by the FRI verifier. Focused `cargo check` validation passed
  for the recursion crate and for `ai-pow-zk --features recursion`; targeted
  recursion lib tests pass for prelude/profile binding and canonical
  production-parameter rejection.
- Release/native cap-height diagnostics now pass. Cap height `3` is retained
  because it gives the best measured final-target structural floor:
  `142,807` merged-only bytes, `152,612` paired zero-support-FRI floor, and
  `46.134s` total subproof proving for the current oversized support theorem.

#### Remaining Work

- Replace the measured coalesced shared packed-trace support theorem with a
  relation-level smaller packed Tip5 binding. The cap-height sweep confirms
  Merkle-cap tuning is useful but not decisive: at the retained cap height `3`,
  the current packed support compact FRI is `177,338` bytes and the optimistic
  single-FRI floor is still `249,184` bytes.
- Next measurement target: redesign the packed Tip5 support theorem so it does
  not preserve the current support-FRI shape. A useful candidate must remove
  substantially more than the paired lookup's estimated `29.9 KiB` combined
  opened-value/zeta-opening savings, and must directly reduce the cap-height
  `3` support theorem's `36,664` byte input-Merkle payload and `41,842` byte
  commit-round Merkle payload or avoid paying those support-theorem costs
  separately.
- Do not expect a free partial-column opening win from the retained packed
  trace. A focused liveness test now marks every one of the current `436`
  packed trace columns as consumed by the AIR algebra, byte LogUp, or
  selected-trace bridge quotient language. Any large packed-trace opening
  reduction therefore needs a new trace/quotient/commitment shape, such as a
  paired lookup or another relation that stops committing the current split-byte
  rows, rather than simply omitting unused columns from the existing proof.
- Also measure any candidate against the `142,807` byte merged-base floor. To
  meet binary `150 KiB` without reducing the base proof, all remaining support
  binding metadata and proof payload together must fit in `10,793` bytes. That
  is still below the paired-lookup support metadata estimate before any FRI
  payload.
- Fold the resulting Tip5 binding into the merged value-bridge proof instead
  of appending another standalone packed proof body.
- Reuse prepared terminal compile output, NPO columns, packed traces,
  commitment roots, and prelude material so the measured production pipeline is
  the total path a miner would actually run.
- Extend negative tests to the final fused production artifact, covering stale
  packed trace roots, stale table/profile data, wrong selected-value bridge
  roots, wrong `mmcs_bit` projection, malformed compact FRI payloads, and
  transcript/prelude substitutions.
- Remeasure the full production path with release flags and native CPU codegen.

#### Current Non-Claim

The relaxed milestone is therefore not yet claimed. The verified checkpoints
show which relations are sound, and coalescing same-phase PCS inputs is a real
compression lever, but the current support theorem still cannot be added to the
merged value-bridge checkpoint and meet the budget. The Merkle cap-height
change is implemented, bound, and measured; it reduces the structural floor but
does not replace the required packed Tip5 support-theorem redesign.

### Production Status Summary

| Question | Current answer |
|---|---|
| Production recursive proof path | Committed primary route is compact batch-STARK L2 over a fast statement-bound L1 proof; native terminal reduction is fallback; not the large batch-STARK checkpoint envelope |
| Relaxed target | About `150 KiB` recursive proof artifact and about `30s` total release proving |
| Soundness target | 60 pure FRI query bits per promoted layer; selected compact row uses L1 `lb=3,nq=20,pow=0` and L2 `lb=5,nq=12,pow=0` |
| Most viable shape | Compact batch-STARK L2 with verifier-owned metadata/setup, canonical preprocessed-opening restoration, pruned paths, and explicit final public-value binding of the L1 statement digest |
| Best measured compact batch-STARK candidate | Fast L1 `lb=3,nq=20,cap=4,pow=0` plus L2 `lb=5,nq=12,cap=4,pow=0`, with L1 `alu_lanes=4,horner_k=5`: actual compact wrapper `143,106` bytes, metadata-free body `142,225` bytes, cached L1 prove `15.029s`, cached L2 prove `28.555s`, cached serial L1+L2 `43.584s`. The baseline L1 `alu_lanes=8` row remains close at `143,762` bytes / `142,878` bytes body, cached L1 `15.305s`, cached L2 `28.726s`, cached serial `44.031s` in the focused rerun and `43.893s` in the L1-packing sweep |
| Promoted `ai-pow-zk` API and Rust wire integration | `AiPowCompactBatchRecursiveCertificate` now carries a Tip5 verifier-key/setup digest plus the final L2 compact body, with `AiPowCompactBatchVerifierContext` held by the verifier. The Rust `ai-pow` bridge has compact recursive run builders for native and Pearl-compatible attempts, and `ai-pow-miner`'s selected Pearl-compatible recursive builder now serializes the compact certificate as canonical postcard bytes in a bounded proof-node atom. The release/native round-trip measures `142,274` encoded certificate bytes, L1 build `126ms`, L1 outer prove `19.223s`, L2 prep `9.440s`, L2 prove `29.743s`, compact verify `37ms`, and uncached proof wall `58.603s`; decoded verification passes, wrong public inputs reject, wrong certificate digest rejects, and stale context digest rejects. The full Pearl-compatible `%ai-pow` artifact now measures `141,916` jammed bytes with a `141,103` byte compact certificate, and bounded decode plus canonical byte-node re-encoding pass. Hoon verifier wiring and production-pinned expected digest remain open |
| Best measured compact-L2 size reserve | The selected L2 table-packing sweep verifies the same compact body with `alu_lanes=2,horner_k=5` at `126,862` bytes actual compact wrapper and `125,979` bytes metadata-free body, but cached L2 proving rises to `30.801s`, so this is useful size margin, not the current time route |
| Best measured L1-packing size reserve | The selected compact L2 over L1-packing sweep verifies L1 `alu_lanes=2,horner_k=5` at `141,148` bytes compact wrapper and `140,260` bytes metadata-free body, with cached serial L1+L2 `44.391s`; useful for byte headroom, not a time fix |
| Best measured complete base | Cap-height `3` full-context merged-only structural floor at `142,807` bytes; sound for its included relations, but missing internal Tip5 binding |
| Best near-target standalone missing binding | Lane-selector-aware selected-to-packed NPO-IO bridge at `137,355` bytes / `134.1 KiB`, prove `28.526s`, verify `14.510s` |
| Direct bridge diagnostic | Binding selected NPO values directly to compact packed trace lanes verifies at `205,950` bytes / `201.1 KiB`, prove `35.863s`; it removes the projection commitment/domain but is too large standalone because it still opens the full `436`-column packed trace |
| Negative fusion results | Naive projection+selected fusion verifies at `243,516` bytes / `237.8 KiB`, prove `35.423s` on the older width-500 trace; uncoalesced shared packed-trace support theorem verifies at `273,113` bytes / `266.7 KiB`, prove `36.590s`; compact-trace coalesced shared support theorem verifies at `198,287` bytes / `193.6 KiB`, prove `33.277s`; cap-height `3` merged-value plus packed-support optimistic single-FRI floor is `249,184` bytes, `95,584` bytes over binary `150 KiB`; final-capacity-lane elision was measured and rejected at `197,259` bytes, prove `35.362s`; packed byte-LogUp group size 15 was measured and rejected at `206,759` bytes, prove `38.515s` |
| Best measured outer task parallelism | Rayon-joining the current primitive R1CS, merged value-bridge, and packed-support subproofs gives `39.448s` post-prelude subproof wall time versus `53.355s` summed subproof timers, but leaves the same `249,184` byte optimistic single-FRI floor and `171.422s` full diagnostic wall |
| Main current blocker | The compact batch-STARK L2 size row is in range and cached L2 proving is now under `30s`, but measured cached serial L1+L2 proving is still `43.584s` at the best measured L1-packing row (`15.029s` cached L1 + `28.555s` cached L2). The selected L2 verifier has `4,791` Tip5 rows padded to `8,192`, only `695` rows above the `4,096` halving boundary. L2 and L1 table-packing sweeps show ALU lane retuning can trade size for time but does not reduce total proving time enough: L2 `alu_lanes=2` shrinks compact L2 to `126,862` bytes but raises cached L2 proving to `30.801s`, while the best L1-packing row saves only about `0.4s` cached serial over baseline. |
| Next implementation step | Finish the Hoon-facing verifier path and production-pinned setup-digest policy for the compact byte-node artifact, then reduce or overlap L1 proving and reduce committed L2 verifier matrix volume without merely shifting the ALU size/time tradeoff. Hidden-L1 cap retuning is not enough as tested: cap `3 -> 4` saves only `176` rows, and cap `5/6` currently fail inside the recursive verifier despite native L1 verification passing. Add final artifact rejection tests as the production wire path is promoted |

### Decision

The current committed route to the relaxed production target is compact
batch-STARK L2 over a fast statement-bound L1 proof. This is a route decision,
not a production-readiness claim. The target remains
approximately `150 KiB` for the recursive proof artifact and approximately
`30s` total release proving time, with no proof-system PoW bits counted toward
soundness.

The selected measurement row uses L1 `lb=3,nq=20,cap=4,pow=0` and L2
`lb=5,nq=12,cap=4,pow=0`. It verifies with ten final L2 public-binding lanes
for the L1 statement-digest base limbs. The baseline L1 `alu_lanes=8` row
measures `143,762` bytes for the actual compact wrapper, `142,878` bytes for
the metadata-free compact body, and `90,307` bytes for the core compact proof.
That is inside the relaxed size gate. The latest L1-packing sweep finds a
slightly better complete row at L1 `alu_lanes=4,horner_k=5`: `143,106` bytes
actual compact wrapper, `142,225` bytes metadata-free compact body,
`15.029s` cached L1 proving, `28.555s` cached L2 proving, and `43.584s`
cached serial L1+L2 proving. L1 `alu_lanes=2` gives more byte headroom
(`141,148` bytes compact) but is slower overall (`44.391s`). The earlier
focused serial proving time was still too high: `24.865s` for L1 plus
`48.281s` for L2 in the pre-cache diagnostic. The cached-prep route is
materially closer but still too high for the total route: the best current row
is `43.584s` cached serial, with reusable L1/L2 prep outside the per-attempt
path.

The terminal fallback keeps the production profile at pure-query 60-bit FRI
soundness: `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`, and
production Merkle cap height `3`. The cap height is bound into terminal proof
parameters, proximity profile, and transcript material before challenge
sampling. It also keeps the primitive sparse-R1CS row-product proof, but it
must reuse prepared
assignment/relation data and parallelize independent post-prelude work. The
parallelism is now measured: it saves the current subproof stage from a
`53.355s` serial sum to a `39.448s` wall, but the packed-support branch still
sets the whole parallel window and setup phases remain far too large. The main
remaining size and time lever is replacing exhaustive supported-NPO openings
with a single NPO theorem that contains:

- merged residual-zero, recompose, padding, Merkle-direction-aware value bridge,
  and `mmcs_bit` constraints;
- packed one-row-per-permutation Tip5 AIR algebra;
- packed Tip5 lookup membership, with paired 16-bit lookup available only as a
  partial column/interactions reduction unless paired with a larger FRI-shape
  redesign;
- lane-selector-aware selected NPO-value to packed trace-lane binding, deriving
  the 16 input lanes and 10 final-output lanes from the same packed trace
  opening used by packed AIR/LogUp;
- a reduced committed-domain/quotient shape so the packed trace binding does
  not become another `250 KiB+` compact-FRI support theorem.

The large batch-STARK recursive checkpoint envelope is not the production wire
target. Its compact final-layer diagnostics are now a valid production
candidate direction and have been promoted inside `ai-pow-zk` as a typed
compact certificate/context API, but the measured end-to-end batch-STARK
pipeline is still dominated by L1/L2 proving work and misses the total
proving-time target.

### Why This Is The Best Measured Path

The old production native terminal body is ruled out as-is. Even the
relation-favorable `lb=6,nq=10,pow=0` full-composite diagnostic measured
`771,249` bytes postcard wire and `80.829s` terminal proving. That row is a
lower-bound diagnostic for verifier-relation size, not a production parameter
recommendation.

The merged padding/value-bridge checkpoint verifies over the actual
`ai-pow-zk` composite verifier relation and serializes
`(prelude, primitive R1CS, merged NPO)` to `151,448` bytes / `147.9 KiB`. Its
post-prelude proof body builds in `14.914s` serial. This is close enough to the
relaxed byte target to be a credible base, but it is not production complete
because it does not bind the internal Tip5 lookup/AIR/LogUp work.

The packed Tip5 checkpoints show the missing internal Tip5 binding can be made
sound and materially faster than the row-per-round shape. The packed trace maps
`8,081` Tip5 calls into an `8,192 x 436` trace and cuts the algebra quotient
domain from `524,288` rows to `65,536` rows. The width dropped from `500` to
`436` by omitting explicit round-input columns for rounds 1-4 and reading those
inputs from the previous round's output columns. The standalone packed AIR
algebra proof verifies at `129,471` bytes / `126.4 KiB` with `19.875s`
proving. The standalone packed byte-table LogUp proof verifies at `155,974`
bytes / `152.3 KiB` with `22.284s` proving.

The packed trace NPO-IO projection checkpoint now verifies too. It commits a
26-column packed-domain projection containing the packed trace's 16 round-0
input lanes and 10 final-round output lanes, then proves those projection
columns are derived from the packed trace commitment. On the full PROD
composite relation, the pre-compaction standalone projection proof verified at
`149,525` bytes / `146.0 KiB`, with compact FRI `138,727` bytes, full-trace
zeta openings `10,007` bytes, NPO-IO openings `527` bytes, opened quotient
`7` bytes, prove `20.379s`, and verify `10.692s`. It has not yet been rerun
after the compact trace width change because the direct selected-to-packed
trace bridge is the sharper production diagnostic.

The lane-selector-aware selected-to-packed NPO-IO bridge now verifies. It
binds the selected value-bridge endpoint to the packed NPO-IO projection
endpoint with verifier-derived per-lane selectors: all 16 input lanes are
included for every Tip5 row, and each final-output lane is included only when
its `output_present_limb` selector is one. On the full PROD composite relation,
the standalone bridge proof now verifies at `137,355` bytes / `134.1 KiB`,
with compact FRI `132,756` bytes, selected lookup opening `1,691` bytes,
packed NPO-IO opening `525` bytes, accumulator openings `721 + 729` bytes,
quotient openings `39 + 40` bytes, selected quotient rows `65,536`, packed
quotient rows `32,768`, prove `28.526s`, and verify `14.510s`. This uses a
3-lane LogUp grouping, which keeps the bridge sound while cutting the bridge
quotient blowup from 8 to 4; a measured one-lane grouping reduced quotient rows
further but increased proof size because accumulator openings dominated.

A direct fused projection+selected-bridge checkpoint also verifies, but it is
not the production route. It shares the selected lookup, packed trace, packed
NPO-IO, projection quotient, and bridge quotient openings in one transcript and
one compact FRI proof. On the full PROD composite relation, that naive
multi-domain fusion measures `243,516` bytes / `237.8 KiB`, compact FRI
`228,782` bytes, prove `35.423s`, and verify `14.835s`. This is useful
negative evidence: the final production fusion has to reduce or algebraically
combine opened domains/quotients, not just batch more standalone matrices into
one PCS opening.

The direct selected-to-packed-trace bridge now verifies as a sharper
diagnostic. It removes the intermediate packed NPO-IO projection commitment and
projection quotient domain entirely: the prover commits to the selected lookup
and packed trace, and the verifier derives the 26 NPO-IO lanes from the opened
packed trace at `zeta`. On the full PROD composite relation it measures
`205,950` bytes / `201.1 KiB`, compact FRI `193,160` bytes, full packed-trace
opening `8,718` bytes, prove `35.863s`, and verify `14.955s`. This is
smaller than the naive projection+selected fusion, but still not production
viable standalone. The full `436`-column packed trace opening is too expensive
as a separate bridge proof, and the next measurement below shows that sharing
that opening without reducing the surrounding quotient/accumulator domains is
still not enough.

The first shared packed-trace AIR+LogUp+selected-trace theorem now verifies,
and the follow-up PCS input-batch coalescing is a real compression lever. The
uncoalesced version shared one packed trace opening, but still committed each
support matrix separately; it measured `273,113` bytes / `266.7 KiB`, compact
FRI `256,261` bytes, and prove `36.590s`. The first coalesced version grouped
same-transcript-phase matrices into four PCS input batches and measured
`208,799` bytes / `203.9 KiB`, compact FRI `192,253` bytes, and prove
`33.313s` on the width-500 packed trace. The compact-trace coalesced version
now verifies at `198,287` bytes / `193.6 KiB`, compact FRI `183,018` bytes,
selected lookup opening `1,698` bytes, packed trace opening `8,728` bytes, AIR
quotient opening `41` bytes, LogUp table opening `21` bytes, LogUp accumulator
opening `1,840` bytes, LogUp quotient opening `42` bytes, selected/packed
bridge accumulator openings `716 + 724` bytes, and selected/packed bridge
quotient openings `42 + 41` bytes. It uses AIR and LogUp quotient domains of
`65,536` rows, selected bridge quotient rows `65,536`, and packed bridge
quotient rows `32,768`. It proves in `33.277s`, verifies in `14.358s`, and
has `94.874s` total diagnostic wall time. This confirms that Merkle-path
duplication and duplicate round-input columns were meaningful payload, but the
remaining full-trace and accumulator leaf payload is still too large for the
relaxed target.

The merged-value plus packed-support fusion-floor diagnostic makes the negative
result stronger. It builds the merged value bridge and compact packed support
theorem against one terminal circuit and one shared prelude. The current
appended body is `336,210` bytes at cap height `3`. The merged compact FRI is
`84,850` bytes and the packed support compact FRI is `177,338` bytes. Even an
intentionally optimistic single-FRI floor that keeps only the larger FRI body,
keeps the primitive R1CS proof (`54,320` bytes), keeps both non-FRI NPO payloads
(`3,349 + 16,065` bytes), and subtracts duplicate selected-lookup profile,
commitment, and opening payload is `249,184` bytes. This is still `95,584`
bytes over a binary `150 KiB` gate and `99,184` bytes over a decimal
`150,000` byte gate. The packed support FRI alone therefore consumes more than
the final combined FRI budget, which is only about `80-83 KiB` after primitive
and non-FRI metadata are accounted for. Ordinary transcript/FRI sharing is now
ruled out as the route to the relaxed milestone.

A narrower final-round capacity-lane elision was also measured and rejected.
That transient layout kept the 10 final output lanes selected by the NPO
bridge, omitted unused final output lanes 10-15, and reduced packed trace width
from `436` to `430`. On the full PROD composite relation, the shared theorem
measured `197,259` bytes / `192.6 KiB`, compact FRI `182,152` bytes, opened
packed trace `8,584` bytes, and prove `35.362s`. It saved only `1,028` bytes
versus the current compact-trace coalesced theorem and worsened proof time by
about `2.1s`, so it is not retained as the current production route.

A packed byte-LogUp group-size increase was also measured and rejected. The
current packed byte LogUp groups 161 interactions into size-7 groups, giving
46 accumulator basis columns and a `65,536`-row LogUp quotient domain. The
candidate used size-15 groups, reducing the accumulator opening to 22 basis
columns but increasing the LogUp quotient domain to `131,072` rows. On the
full PROD composite relation, the shared theorem measured `206,759` bytes /
`201.9 KiB`, compact FRI `192,689` bytes, opened LogUp accumulator `883`
bytes, and prove `38.515s`. The accumulator opening shrink was overwhelmed by
the larger quotient-domain FRI payload, so the current group size 7 remains
the measured better point.

Those standalone proofs are evidence, not an appendable production proof.
Appending any standalone packed proof to the `151,448` byte merged
value-bridge checkpoint would exceed the relaxed size target because it would
duplicate full-trace and FRI opening material. The current coalesced shared
packed-trace support theorem is also not appendable. The remaining opportunity
is a leaner algebraic packed Tip5 binding that reduces opened trace and
accumulator payload before it is folded into the same transcript and opening
set as the merged value bridge.

The paired 16-bit lookup candidate has now been measured as a payload estimate,
and the result is negative as a standalone route. Instead of 160 separate
`(b,c)` byte lookups per packed trace row group across the five rounds, the
prover would expose 80 paired words `(x,y)` where `x = b0 + 256*b1` and
`y = L(b0) + 256*L(b1)`. That cuts the packed split columns from 320 to 160 and
the packed trace width floor from 436 to 276. It also cuts LogUp interactions
from 161 to 81, reducing accumulator basis columns from 46 to 24 at the current
group size 7. Against the actual packed-support proof, this estimates compact
FRI opened-value savings of `25,912` bytes and non-FRI zeta-opening savings of
`4,084` bytes. The packed support compact FRI would still be about `151,376`
bytes, and the optimistic merged-value plus support floor would still be
`219,138` bytes. The estimate deliberately excludes new two-domain table Merkle
overhead and any new quotient-shape cost, so the implemented result would not
be smaller than this estimate unless it also changes the support-FRI shape.
Therefore paired lookup remains a possible component, not the primary
production route.

The tighter structural floor is now the primary design constraint. The
already-verified merged base in the cap-height `3` fusion-floor run is
`142,807` bytes (`288` prelude + `54,320` primitive R1CS + `88,199` merged
value bridge).
Current packed-support metadata after selected-binding deduplication is
`13,889` bytes, so a hypothetical zero-FRI support theorem would still be
`156,696` bytes. Applying the paired-lookup non-FRI opening estimate reduces
that metadata to `9,805` bytes and the zero-support-FRI floor to `152,612`
bytes, leaving only `988` bytes under binary `150 KiB` before any support FRI
payload or excluded overhead. This means the next useful implementation has to
make the support binding almost metadata-free, reduce the primitive/merged
base, or replace the current decomposition with a single theorem whose
serialized metadata is not the sum of the current subproofs.

The per-input-batch breakdown explains why: the current packed-support compact
FRI has `123,167` bytes of input batches and `51,080` bytes of commit rounds.
The input batches split as selected lookup `20,314`, packed trace/table
`70,787`, accumulators `20,813`, and quotients `11,252` bytes. The trace/table
batch has `62,311` opened-value bytes and `8,476` Merkle bytes; the accumulator
batch has `11,737` opened-value bytes and `9,076` Merkle bytes.
Pairing bytes attacks only part of the opened-value payload and leaves the
support theorem's Merkle and FRI commit-round structure largely intact. The
next viable measurement must remove those costs or avoid paying them as a
separate support theorem.

The selected-to-packed bridge should compare only lanes that are semantically
present. The selected value bridge masks final-output lanes by
`output_present_limb`, while the packed NPO-IO projection checkpoint keeps the
unmasked final-round output lanes so it can be derived directly from the packed
trace. A naive all-26-lane equality would therefore fail on absent output lanes:
the selected side contains zero and the packed side contains the trace value.
The least invasive sound path is to keep the unmasked packed projection proof
and add verifier-derived per-lane selectors to the selected-to-packed LogUp
equality: all 16 input lanes are included for every Tip5 row, and each of the 10
output lanes is included only when its `output_present_limb` selector is one.
This preserves the existing packed trace binding and the
Merkle-direction-aware `mmcs_bit` value bridge without introducing a second
masked projection commitment.

### Done And Verified

- Native terminal remains the selected production backend; the README for
  `ai-pow-zk` links directly to this section as the current important status.
- The old full-composite native terminal proof shape is measured and ruled out
  at `771,249` bytes and `80.829s` proving.
- The full-composite merged padding/value-bridge checkpoint verifies and proves
  residual-zero, recompose semantics, mixed padding, dynamic hidden Tip5
  padding, new-start zero constraints, Merkle capacity-zero constraints,
  `mmcs_bit` zero/boolean constraints, and Merkle-direction-aware bus-to-trace
  value projection.
- The packed Tip5 trace source is implemented and checked against the existing
  lookup trace and terminal-derived Tip5 inputs/outputs.
- The packed Tip5 AIR algebra quotient checkpoint verifies. Its tests
  round-trip the proof and reject stale roots/profiles, malformed openings, and
  tampered packed round links, split bytes, and power lanes.
- The packed byte-table LogUp quotient checkpoint verifies. Its tests
  round-trip the proof and reject stale trace/table commitments, stale table
  profiles, malformed openings, tampered table/accumulator/quotient openings,
  non-table byte pairs, and stale table multiplicities.
- The packed trace NPO-IO projection checkpoint verifies. Its tests round-trip
  the proof and reject stale trace/projection commitments, stale projection
  profiles, malformed openings, tampered projection openings, and stale prelude
  roots for a changed packed trace.
- The lane-selector-aware selected-to-packed NPO-IO bridge checkpoint verifies.
  Its tests round-trip the proof, require selected and packed endpoint roots to
  match the merged value-bridge and packed projection commitments, compare only
  semantically present lanes, reject stale endpoint commitments/profiles,
  reject malformed openings, reject tampered selected openings, and reject a
  stale prelude for a changed packed projection endpoint.
- The direct selected-to-packed-trace bridge checkpoint verifies. Its tests
  round-trip the proof, require the selected endpoint to match the merged
  value-bridge root and the packed endpoint to match the packed trace root,
  derive the packed NPO-IO lanes from the opened packed trace, reject stale
  endpoint commitments/profiles, reject malformed packed trace openings, reject
  tampered packed trace openings, and reject a stale prelude for a changed
  packed trace.
- The shared packed-trace AIR+LogUp+selected-trace bridge checkpoint verifies.
  Its tests round-trip the proof, require the selected endpoint and packed
  trace/table endpoint roots to match the prelude, share one packed trace
  opening across packed AIR, packed LogUp, and selected-trace bridge checks,
  coalesce same-phase PCS input batches, and reject tampered packed trace
  commitments/openings, tampered LogUp table openings, and tampered bridge
  final cumulatives. Packed-side bridge finals are reconstructed from
  selected-side finals in the serialized form. The full PROD coalesced
  measurement is still negative at
  `198,287` bytes and `33.277s` proving.
- The merged-value plus packed-support fusion-floor diagnostic verifies and
  shows ordinary fusion is not enough: at retained cap height `3`, current
  appended body `336,210` bytes, optimistic single-FRI floor `249,184` bytes,
  and packed support compact FRI `177,338` bytes.
- The production soundness policy is explicit: 60 bits must come from FRI query
  soundness at `pow=0`, not from proof-system grinding.
- The paired 16-bit lookup path is measured as useful but insufficient by
  itself: estimated optimistic floor `219,138` bytes, still `65,538` bytes over
  binary `150 KiB`, before two-domain table overhead.
- Verifier-derived profiles are now omitted from the serialized shared packed
  AIR+LogUp+selected-trace support proof. The verifier recomputes expected
  profiles and uses them for transcript seeding, domains, opening dimensions,
  and relation checks; focused round-trip validation and the release/native
  fusion-floor diagnostic pass.
- Verifier-derived profiles are now also omitted from the serialized merged
  residual-zero/recompose/value-bridge proof. The verifier recomputes expected
  profiles from the VK and uses them for transcript seeding, domains, opening
  dimensions, returned opened-column profile, and relation checks; focused
  round-trip validation and the release/native fusion-floor diagnostic pass.
- Sumcheck `eval_1` values are now omitted from serialized primitive R1CS
  matrix and row-product sumcheck rounds. The verifier reconstructs them from
  `eval_0 + eval_1 = current_claim` before Fiat-Shamir challenge derivation;
  focused primitive round-trip tests and the release/native fusion-floor
  diagnostic pass.
- The structural zero-support-FRI lower bound is now measured: `156,696` bytes
  with current support metadata, or `152,612` bytes after paired-lookup
  non-FRI opening savings. The paired metadata-only floor has only `988` bytes
  of binary headroom before any sound support FRI payload or excluded overhead.

### Remaining Work

- Replace the current coalesced shared packed-trace AIR+LogUp+selected-trace
  support theorem with a leaner packed Tip5 binding. The failed
  projection+selected fusion, the still-too-large standalone direct bridge,
  the `273,113` byte uncoalesced theorem, the `208,799` byte width-500
  coalesced theorem, and the `198,287` byte compact-trace coalesced theorem
  show this must reduce opened trace/accumulator payload, not merely remove
  duplicate Merkle paths, duplicate round-input columns, or unused final
  capacity output lanes. The retained `436`-column packed trace is now covered
  by a liveness test showing every column is consumed by the current
  AIR/LogUp/bridge quotient language, so reducing the packed-trace opening
  requires a new trace or commitment shape. The cap-height `3` `249,184` byte
  optimistic
  single-FRI floor further rules out merely sharing one transcript/FRI proof
  while keeping the current packed support FRI payload. It also cannot be
  achieved by increasing packed byte-LogUp group size to trade accumulator
  width for a doubled quotient domain.
- Do not spend the next implementation checkpoint on paired 16-bit lookup as a
  standalone support theorem. Its measured estimate leaves the floor too large.
  Use it only if a larger redesign also removes support-theorem Merkle paths,
  commit rounds, or quotient/accumulator domains.
- Measure the next relation-level support-FRI redesign against the same
  per-input-batch ledger. The success condition is not "saves opened values";
  it must bring the merged-value plus Tip5-support floor to approximately
  `150 KiB` while preserving 60 pure-query FRI soundness.
- Track the `142,807` byte merged-base floor in every candidate. If the
  candidate does not reduce that base, its complete sound support binding must
  serialize under `10,793` additional bytes for binary `150 KiB`.
- Remove duplicated production prover setup by reusing terminal compile
  outputs, prepared NPO columns, packed traces, roots, and prelude material
  wherever verifier key and public inputs are unchanged.
- Remeasure the fused full-composite proof in release with
  `RUSTFLAGS="-C target-cpu=native"` and the production pure-query profile.
  The milestone requires a total production-pipeline proving measurement, not
  only post-prelude subproof timings.
- Extend negative tests from the verified subproof checkpoints to the final
  fused production artifact: stale packed trace roots, stale fixed
  table/profile data, wrong selected-value bridge commitments, wrong
  `mmcs_bit` projection, malformed compact FRI payloads, and
  transcript/prelude substitutions.

### Current Non-Claims

- The relaxed milestone has not been met yet.
- The standalone packed AIR, packed LogUp, packed NPO-IO projection, and
  selected-to-packed bridge proofs are sound checkpoints, but they are not
  production artifacts by themselves.
- The 16-bit paired lookup/two-domain LogUp route is not yet a soundness claim
  and is now measured insufficient as the primary size route. It remains a
  possible component in a larger design.
- The batch-STARK L2/final-layer route does not replace the native terminal
  production certificate path.
- Any route that changes to proof-system PoW grinding, omits the selected
  NPO-value to packed projection bridge, or appends standalone packed proofs
  without fusing openings is not the current viable production path.

The current recursion-crate Tip5 verifier-circuit terminal measurement passes
both targets in release mode:

| Item | Measurement |
|---|---:|
| Terminal certificate body | `85,726` bytes / `83.7 KiB` |
| Terminal certificate | `85,948` bytes / `83.9 KiB` |
| Prove time | `1.492s` in release with `RUSTFLAGS="-C target-cpu=native"` |
| Verify time | `1.181s` |
| Required terminal profile | `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`, `max_log_arity=3`, `log_final_poly_len=0` |

The stack-level follow-up added
`crates/ai-pow-zk/src/recursion.rs::prove_terminal_certificate_from_chain_verified_composite_proof`
for the actual composite L1 verifier circuit. A completed release/native
reduced-profile measurement verifies after postcard decode, but it is not close
to the hard target:

| Full composite terminal profile | Certificate | Public inputs | Postcard wire | Compile | Prove | Verify |
|---|---:|---:|---:|---:|---:|---:|
| `lb=6,nq=10,pow=0` after compact known-index proof encoding | `766,069` bytes / `748.1 KiB` | `5,180` bytes | `771,249` bytes / `753.2 KiB` | `7.606s` | `80.829s` | `58.825s` |

Therefore the fixture measurement is evidence that the backend can be small on
a much smaller verifier relation, not proof that the full AI-PoW production
recursive artifact already satisfies the byte or time gates.

A non-proving production-profile relation diagnostic now measures the actual
composite L1 terminal relation:

| Metric | PROD baseline |
|---|---:|
| Terminal compile time | `20.943s` |
| Terminal public input bytes | `5,319` |
| Terminal private input values | `43,443` |
| Terminal operations | `125,961` |
| Primitive operations | `106,349` |
| Const operations | `582` |
| Public operations | `459` |
| ALU add operations | `8,832` |
| ALU multiplication operations | `10,234` |
| ALU boolean-check operations | `255` |
| ALU fused multiply-add operations | `10,117` |
| ALU Horner-accumulator operations | `75,870` |
| Supported NPO rows | `14,049` |
| Tip5 rows | `8,081` |
| Recompose/coeff rows | `5,743` |
| NPO input/output callsite slots | `141,232` / `86,778` |
| NPO residual components | `242,798` |
| Circuit fingerprint | `witness=221,989 public=459 private=43,443 ops=125,961` |

This shifts the immediate reduction target. The terminal public input vector is
not the blocker at about `5.3 KiB`; the generic composite verifier relation is.
Any production candidate has to reduce more than `100k` primitive operations,
about `14k` supported NPO rows, and a terminal compile step that already
consumes most of the `<30s` budget before proving starts.

The operation-class breakdown makes the cause more specific. In the production
profile, Horner accumulation accounts for `75,870` of `106,349` primitive
operations. These are verifier-arithmetic steps from the generic FRI/PCS
opening, quotient, and batch-consistency checks, not matrix-multiplication work
or terminal public-input framing. The NPO rows are also concentrated:
`8,081` Tip5 permutation rows and `5,743` `recompose/coeff` rows. The
`recompose/coeff` rows are emitted because
`build_composite_l1_verifier_circuit` enables
`set_recompose_coeff_ctl_for_decompose_links(true)` for the D=2 recursive
verifier. Disabling that table may make a diagnostic smaller, but it is not a
production reduction unless there is a replacement proof that every hinted
extension-field decomposition remains connected to a creator and every affected
WitnessChecks bus entry is sound.

The opt-in recompose-control lower-bound diagnostic makes that tradeoff
concrete. It builds the same production-profile composite proof once, then
compiles the terminal relation with the production binding enabled and with the
binding disabled:

| PROD terminal relation toggle | Ops | Primitive ops | Tip5 rows | Recompose rows | Recompose/coeff rows | NPO rows | NPO residuals | Compile |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Production `recompose_coeff_ctl=true` | `125,961` | `106,349` | `8,081` | `225` | `5,743` | `14,049` | `242,798` | `21.045s` |
| Unsound floor `recompose_coeff_ctl=false` | `125,571` | `106,349` | `8,081` | `5,578` | `0` | `13,659` | `240,458` | `20.596s` |
| Delta | `390` | `0` | `0` | `+5,353` | `-5,743` | `390` | `2,340` | `0.449s` |

This rules out "remove or replace the recompose/coeff table" as a primary
size lever. The disabled-table row is unsound and still saves only `390`
terminal operations, no primitive verifier arithmetic, and `390` supported NPO
rows because most coefficient-control calls become ordinary recompose rows.
The hard blocker remains the generic verifier relation and assignment-opening
shape, not the marginal overhead of this specific coefficient-binding table.

The full-composite FRI-native residual-zero NPO measurement changes the shape
of the promising path. After fixing terminal Tip5 Merkle-direction
input/hidden-lane reconstruction, the residual-zero layer is byte-plausible and
verifies:

| Full composite FRI-native residual-zero checkpoint | Value |
|---|---:|
| NPO rows / padded rows | `14,049` / `16,384` |
| Prover-dependent field columns / basis columns | `89` / `178` |
| Residual-value columns | `46` |
| Residual-zero opened basis columns | `180` |
| Proof body / compact FRI | `55,344` bytes / `54,023` bytes |
| Nonzero residual values | `0` |
| Prove / verify / total diagnostic wall | `19.635s` / `11.930s` / `87.229s` |
| Verification status | `verified` |

This is a useful lower bound because it uses the actual composite verifier
layout rather than the small synthetic NPO-only fixture, and the proof body is
comfortably below both the hard `~100 KiB` proof-size target and the relaxed
`150 KiB` budget. It is not a complete production proof by itself:
residual-zero only proves that committed residual columns are zero. The next
NPO work should therefore focus on integrating or replacing the
verifier-key-derived row-relation quotients, recompose/value-bridge checks,
Tip5 lookup/AIR binding, primitive row-product checks, and their shared
root/prelude work, rather than on further byte serialization of this single
layer.

The Merkle-direction fix also identifies an explicit binding requirement for
the next value-bridge/AIR step. Tip5 callsite inputs are in pre-swap bus-limb
coordinates, while the Tip5 trace and lookup AIR consume post-direction
permutation-input lanes. When `mmcs_bit=1`, input and hidden trace lanes swap.
A sound polynomial bridge must either keep committed NPO value columns in
bus-limb coordinates and prove the `mmcs_bit`-selected projection into the
Tip5 trace, or commit/constrain direction-dependent trace-lane present
selectors. Treating trace-lane hidden/input selectors as verifier-fixed across
both directions is not a sound explicit binding.

The follow-up merged padding/value-bridge candidate now proves direction-aware
bus-to-trace projection, mixed value padding, `mmcs_bit` zero/boolean
constraints, dynamic hidden Tip5 padding, new-start zero constraints, Merkle
capacity-zero constraints, and recompose value semantics in the same
FRI-native NPO proof as residual-zero and recompose. It serializes that proof
with the terminal prelude and primitive row-product proof:

| Full composite merged padding/value-bridge checkpoint | Value |
|---|---:|
| Serialized body | `151,448` bytes / `147.9 KiB` |
| Prelude / primitive / merged NPO proof | `240` / `57,501` / `93,707` bytes |
| Merged compact FRI payload | `91,501` bytes |
| Post-prelude primitive prove / merged prove / serial total prove | `8.372s` / `6.541s` / `14.914s` |
| Primitive verify / merged verify / total verify | `30.268s` / `13.525s` / `43.794s` |
| Full diagnostic setup before proof body | terminal compile `10.153s`; assignment commitment `6.843s`; prepared merged NPO root `14.661s`; prelude `10.031s` |
| Total diagnostic wall | `108.606s` |
| Verification status | `verified` |

This is the first full-composite polynomial NPO checkpoint that is both under a
binary `150 KiB` relaxed byte gate and verifier-accepted, though it is still
`1,448` bytes over a strict decimal `150,000` byte gate. It is still not a
production recursive proof because the merged NPO proof does not include the
internal Tip5 lookup/AIR/LogUp binding. The padding quotient now uses
`mmcs_bit` dynamically for Merkle hidden-lane selectors, rather than treating
hidden Tip5 presence as verifier-fixed across both Merkle directions. That
keeps serialized sibling lanes `5..9` legal on new-start Merkle rows while
still zeroing prior-state hidden lanes and capacity lanes. The timing result
narrows the remaining production question: after the prelude is already fixed,
serial proof-body construction is under the relaxed `<30s` proving gate at the
same body size. The full diagnostic wall time is still high because it includes
Layer-0 proving, terminal compilation, assignment commitment, prepared NPO-root
construction, prelude construction, and verification.

The primitive proof is not dominated by scalar sumcheck messages. The PROD
sparse R1CS relation has `106,604` rows, `222,449` variables, and `489,990`
entries. The `57,501` byte primitive proof contains only `1,327` bytes of
row-product rounds and `1,088` bytes of matrix-sumcheck rounds. Its assignment
evaluation proof is `55,025` bytes, including `53,586` bytes of fold-round
openings and `48,462` bytes / `1,211` nodes of Merkle frontier material across
the 18 assignment fold layers. A follow-up equality-table precomputation for
sparse R1CS evaluation preserved proof shape but did not materially change
release timing (`50.169s` primitive prove, `33.081s` primitive verify), so the
primitive route needs a different assignment-opening/proximity backend rather
than more sumcheck serialization cleanup.

Deterministic parallel parent hashing in the terminal oracle Merkle tree
builder then reduced constants without changing proof shape: assignment
commitment construction fell from about `15.854s` to `7.557s`, and primitive
prove time first fell from `50.169s` to `42.210s`. The later
relation/assignment reuse and prelude-checked prover path drops post-prelude
primitive proving to `9.232s` in the value-bridge-only run and `8.372s` in the
latest padding-merged run. This confirms terminal oracle hashing and
prover-work reuse are meaningful engineering costs, while the primitive proof
body remains dominated by assignment-evaluation Merkle frontier material.

The assignment compact-FRI floor confirms that simply FRI-opening the whole
assignment vector is not the missing size lever. The opt-in diagnostic
`terminal_assignment_compact_fri_floor_for_prod_baseline_measures` commits the
full `222,449`-entry terminal assignment as one extension-field FRI column over
`262,144` padded rows. The resulting proof is `56,307` bytes: `56,202` bytes
of compact FRI payload, `41` bytes of opened values, and `2` Goldilocks basis
columns. It proves in `18.233s` and verifies in `9ms` after terminal setup, but
it is slightly larger than the current `55,025` byte Merkle-fold assignment
evaluation proof. It also lacks the critical production binding: the sparse
R1CS matrix-sumcheck needs a proof that the opened commitment equals the
multilinear assignment evaluation at its random point, while this lower-bound
proof only opens the assignment vector as a univariate FRI codeword. A real
replacement therefore needs a shared proximity/PCS design for the multilinear
evaluation relation, or a primitive arithmetization that removes that
obligation.

The integrated Tip5 LogUp route remains the most promising proof-shape route
for the relaxed `150 KiB` target, but not yet for the full `<30s` pipeline.
Two prover-side changes move it in the right direction without changing the
cryptographic statement:

- The trace-domain NPO-IO LogUp quotient builder and the Tip5 AIR quotient
  builder now evaluate quotient rows with batched coset LDEs instead of
  interpolating the full committed matrices independently at every quotient
  point. The Tip5 AIR quotient evaluator also precomputes its folded-relation
  constants once, uses stack arrays for opened rows, and evaluates quotient rows
  in parallel. These changes preserve the quotient relations and verifier
  transcript; they only change prover work. On the recursion-crate synthetic
  backend, the production-candidate integrated-LogUp checkpoint remains
  `96,017` bytes / `93.8 KiB` and improves from `25.117s` to `9.918s` in the
  focused run.
- The full-composite diagnostic now prepares NPO columns and the Tip5 trace
  once, then reuses that data for merged roots, bundled Tip5 roots, the merged
  value-bridge proof, and the integrated LogUp proof. This is also
  proof-preserving: the same commitments are still checked against the
  transcript-bound prelude, and verifier acceptance is unchanged. In the partial
  full-composite `lb=6,nq=10,pow=0` run, selected+lookup prepared-data reuse
  reduced merged value-bridge proving to `2.340s`; the selected+lookup root
  phase measured `11.075s` including a `3.85s` commit, and the trace-bundle
  root phase measured `6.261s` including a `5.84s` commit.

The same partial full-composite run explains why this is not yet a production
claim. Before the integrated Tip5 LogUp subproof finished, the diagnostic had
already spent `31.983s` in L0 proving, `7.539s` terminal compilation, `5.933s`
assignment commitment, `11.135s` selected+lookup root construction, `6.678s`
trace-bundle root construction, `7.468s` prelude construction, `14.702s`
primitive proving, and `2.355s` merged NPO proving. The integrated proof then
stayed inside `terminal_npo_integrated_logup.air_quotient_matrix` for more than
two minutes and was stopped. The next engineering lever is no longer prelude
root reuse; it is reducing or specializing the full Tip5 AIR quotient work on
the actual composite trace. This path also cannot make the whole pipeline
`<30s` if the budget includes L0 proving at `lb=6,nq=10`, because that L0
profile alone exceeded `30s`.

The next run after the folded-AIR evaluator cleanup moved the synthetic time but
not the full-composite bottleneck. It spent `29.834s` in L0, `7.543s` terminal
compilation, `5.793s` assignment commitment, `11.075s` selected+lookup root
construction, `6.261s` trace-bundle root construction, `7.600s` prelude
construction, `14.435s` primitive proving, and `2.340s` merged NPO proving,
then remained inside the same integrated AIR quotient phase for more than 90
seconds before the run was stopped. The active production baseline layout has
`8,081` terminal Tip5 calls, so the current lookup-trace AIR would prove a
`65,536`-row full-main trace with `81` prover columns plus `9` fixed columns
over a `524,288`-point degree-8 quotient domain. That is the structural cost to
remove; loop-level cleanup is not sufficient.

Pearl is useful as an architecture reference, but not as a direct parameter
match. Its `zk-pow` path compiles and caches two recursive Plonky2 verifier
circuits, then proves the STARK and two recursive layers from the cache. Its
default parameters use proof-system PoW bits `[18,18,22]` and rate bits
`[1 or 2,3,7]`, so its small final proof is not evidence that our pure-query
`pow=0` terminal profile can keep the current generic verifier-terminal shape.
The transferable lesson is the shape: a cached, specialized recursive verifier
with a compact final layer. For this codebase, the closest non-Plonky2 route is
not the existing lookup-free one-row Tip5 AIR, because that AIR is about
`5,436` columns wide; it is a new narrow one-row-per-permutation or otherwise
hash-specialized terminal argument that avoids a `5x` round-row domain without
opening thousands of bit-decomposition columns.

The first concrete checkpoint for that route is now implemented as a data
source and profile, not yet as an accepted proof: `TerminalNpoTip5PackedLookupTraceProfile`
and `terminal_npo_tip5_packed_lookup_trace_goldilocks`. The packed trace reuses
the already tested lookup-trace generator, pads to zero-input permutations, and
copies each permutation's five round rows horizontally into one row. Round 0
stores `IN`, split `(b,c)` byte pairs, guard inverses, and `OUT`; rounds 1-4
store only split `(b,c)` byte pairs, guard inverses, and `OUT`, with their
`IN` values aliased to the previous round's `OUT`. The focused
regression `goldilocks_npo_tip5_air_trace_matches_terminal_rows` now checks
that these packed rows match the existing lookup trace and terminal-derived
Tip5 inputs/outputs.

The production-profile layout diagnostic now shows the expected structural
gain for the actual composite verifier relation:

| Tip5 terminal trace shape | Rows | Width | Algebra quotient rows | Notes |
|---|---:|---:|---:|---|
| Current row-per-round lookup trace | `65,536` | `117` | `524,288` | Includes the 256 lookup-table rows and five rows per Tip5 permutation. |
| Compact packed one-row-per-permutation lookup trace | `8,192` | `436` | `65,536` | Omits duplicate explicit round-input columns for rounds 1-4; those inputs are previous-round output aliases. |

The packed route therefore removes the `8x` quotient-domain blowup that kept
the integrated Tip5 AIR proof in `air_quotient_matrix`. The first packed proof
checkpoint now implements and verifies the internal Tip5 algebra quotient over
that one-row-per-permutation trace. On the full PROD composite relation, the
standalone packed algebra proof is now `129,471` bytes / `126.4 KiB`, with
`120,507` bytes of compact FRI, `8,756` bytes of full-trace zeta openings, and
`41` bytes of quotient opening. It proves in `19.875s` and verifies in
`10.402s` after setup. This confirms the quotient-domain specialization and
compact trace layout are real runtime levers, but this is not yet a production
theorem.

The second packed proof checkpoint now implements the byte-table LogUp binding
over the same one-row-per-permutation trace. It commits a packed-domain
table-multiplicity column before sampling LogUp challenges, keeps the fixed
Tip5 table verifier-derived, and proves the grouped rational running-sum
transition over the packed trace. On the full PROD composite relation, the
standalone packed LogUp proof is now `155,974` bytes / `152.3 KiB`, with
`144,547` bytes of compact FRI, `8,709` bytes of full-trace zeta openings,
`21` bytes of table opening, `1,848` bytes of accumulator openings, and `42`
bytes of quotient opening. It proves in `22.284s` and verifies in `11.055s`
after setup.
This closes the standalone byte-table soundness gap, but it is not appendable:
both standalone packed algebra and standalone packed LogUp duplicate the
full-trace/Fri opening cost and exceed the relaxed target when combined with
the `151,448` byte merged value-bridge checkpoint. The next step is to fuse
packed AIR, packed LogUp, and selected-value bridge openings under the same
prelude and measure the fused proof body.

The third packed checkpoint now implements the packed trace NPO-IO projection
binding. It commits a 26-column packed-domain projection of round-0 inputs and
final-round outputs, then proves the projection is derived from the committed
packed trace. On the full PROD composite relation, the standalone packed
NPO-IO projection proof is `149,525` bytes / `146.0 KiB`, with `138,727`
bytes of compact FRI, `10,007` bytes of full-trace zeta openings, `527` bytes
of opened projection lanes, and `7` bytes of opened quotient. It proves in
`20.379s` and verifies in `10.692s` after setup. This is the pre-compaction
width-500 measurement and should not be read as a current compact-trace size.
It closes the packed-trace-side projection binding needed by the selected-value
bridge, but it is also not appendable as a standalone proof. The remaining
bridge work is
to bind the selected NPO-value commitment to this packed projection and then
fuse the packed AIR, packed LogUp, projection, and selected bridge openings
under one transcript.

The current `CircuitConfig::PROD` profile is now exactly 60 pure-query bits
(`log_blowup=4`, `num_queries=15`, `pow_bits=0`). Removing the previous
one-bit commit/query proof-system PoW hooks was the right soundness-policy
cleanup, but it only changed the relation from `125,991` to `125,961`
operations. It is not the terminal-size fix.

Pure-query 60-bit Layer-0 profile diagnostics show the real tradeoff:

| L0 profile | Test wall time | Terminal compile | Ops | Horner ops | NPO rows | Assessment |
|---|---:|---:|---:|---:|---:|---|
| `lb=3,nq=20,pow=0` | `32.51s` | `27.692s` | `155,604` | `101,160` | `16,229` | Lower LDE may help L0 proving, but it makes the recursive terminal relation much larger. |
| `lb=4,nq=15,pow=0` | `29.49s` | `20.943s` | `125,961` | `75,870` | `14,049` | Current PROD pure-query baseline. |
| `lb=5,nq=12,pow=0` | `34.05s` | `17.325s` | `108,176` | `60,696` | `12,741` | Meaningful relation reduction, but higher LDE already costs wall time on the tiny baseline. |
| `lb=6,nq=10,pow=0` | `47.60s` | `14.553s` | `96,319` | `50,580` | `11,868` | Best relation reduction measured, but the 64x LDE makes promotion unlikely without separate L0 prover acceleration. |

These are non-proving terminal-relation diagnostics, not production terminal
proof measurements. The test wall time includes Layer-0 proof generation,
L1 verifier construction, and terminal relation compilation for the
`CompositeTrace::baseline_min()` fixture.

The full proof measurement for the most relation-favorable row in the table,
`lb=6,nq=10,pow=0`, still produces a `771,249` byte postcard wire object and
spends `80.829s` in terminal proving. That confirms that simply increasing
Layer-0 blowup to reduce query count is not enough; the terminal relation and
assignment-oracle opening material are still far too large.

That `lb=6,nq=10` row is a lower-bound diagnostic for recursive terminal size,
not a production-profile recommendation. The current PROD baseline remains the
pure-query `lb=4,nq=15,pow=0` inflection point. The `lb=6,nq=10` diagnostic was
chosen because it gives the smallest recursive verifier relation in the
pure-query 60-bit sweep; failing the size/time gates there means the current
terminal proof shape is structurally too large. Its L0 proving cost remains too
high for an unqualified production default.

## Batch-STARK L2 Compression Check

A Pearl-shaped architecture needs a second compression step. The current
batch-STARK L2 path is now a permitted production candidate, but only if the
compact final body meets the relaxed gates with explicit public-value and
verifier-key/setup binding. The opt-in diagnostic
`pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl`
builds a real AI-PoW L1 proof with explicit statement digest public-binding
lanes, then proves a second batch-STARK verifier circuit over that L1 proof.

Current code correction: the L2 candidate now also exposes the statement digest
as final-layer STARK public values. The L1 proof binds `DIGEST_ELEMS` D=2
statement-digest elements, i.e. ten base limbs. The L2 proof must therefore
bind all ten base limbs as base-valued final public lanes, not only
`DIGEST_ELEMS` lanes. In verifier API terms, the compact body is checked with
the D=2 basis expansion of those ten base-valued lanes. The compact constructor
and compact-body verifier use the `*_with_public_values` APIs, so a verifier
must supply the same statement digest when checking the metadata-free compact
L2 body. A focused compact-body regression rejects wrong caller-supplied
public values.

This exercise found one soundness-critical wiring gap before measurement:
`verify_p3_batch_proof_circuit` did not allocate or constrain primitive Public
AIR values for `proof.public_binding_lanes`. The recursive verifier now
allocates `proof.public_binding_lanes * TRACE_D` public inputs for the Public
table and reconstructs `PublicAir` with those binding lanes. That is required
for any L2 proof over a statement-bound L1 object to bind the statement digest
cryptographically.

With public-binding fixed, the first release/native L2 sweep measured the raw
proof, Merkle-only path pruning, and a stronger compact-final projection that
also omits verifier-deterministic preprocessed openings:

| Shape | Final L2 proof | Path-only projection | Preprocessed-omitted projection | L2 prove time | Shared L1 witness proof |
|---|---:|---:|---:|---:|---:|
| L2 `lb=4,nq=15,cap=4,pow=0` over L1 `lb=6,nq=10,cap=4,pow=0` | `207,241` bytes | `201,034` bytes | `156,726` bytes | `12.651s` | L1 `173,171` bytes, path-only `169,609`, preprocessed-omitted `135,701`, L1 prove `192.974s` |
| L2 `lb=5,nq=12,cap=4,pow=0` over same L1 | `178,719` bytes | `174,507` bytes | `136,888` bytes | `24.403s` | same |
| L2 `lb=6,nq=10,cap=4,pow=0` over same L1 | `159,734` bytes | `156,652` bytes | `123,583` bytes | `48.740s` | same |

The pruning projection models Plonky2/Pearl-style authentication-path omission
only when the verifier rederives the Fiat-Shamir query indices from the
transcript. Serializing miner-supplied query indices would be unsound. The
projection also subtracts only omitted digest bytes and does not charge a new
compact-format overhead, so it is an optimistic floor for this proof shape.

The preprocessed-omitted projection subtracts:

- the preprocessed OOD openings in `BatchProof.opened_values`;
- the FRI input batch for the global preprocessed commitment, including opened
  codeword rows and its Merkle authentication path;
- Merkle path-pruning savings for the remaining input and commit-phase
  batches.

This is only sound if the verifier recomputes the preprocessed commitment/cap
and the queried preprocessed codeword rows from pinned verifier data, feeds the
same values into the Fiat-Shamir transcript, and rejects any mismatch in the
verifier-key digest, circuit digest, FRI parameter tuple, public-input digest,
or preprocessed commitment. Treating these values as prover hints would be
unsound.

The historical high-blowup-L1 result was more nuanced than the Merkle-only
check. Duplicate Merkle authentication paths were not the dominant size issue,
but verifier-deterministic preprocessed openings were a real Pearl-style
compactness lever: after the Tip5 direction-binding fix, the `lb5,nq12` final
L2 proof projected to `134,877` bytes with `24.516s` L2 proving, inside the
relaxed size/time budget for the **final layer alone**. That row still failed
production because it first materialized a roughly **194s** L1 batch-STARK
witness proof. The fast-L1 row below supersedes that blocker: the selected path
now hits the relaxed size gate, and the remaining metric gap is total proving
time, especially L2 proving. The Goldilocks/Tip5 compact wrapper implements the
preprocessed-opening reconstruction plus Merkle path-pruning portion of this
projection with binding/tamper tests. The follow-up canonical-metadata body
format also removes the `BatchStarkProof` metadata from the wire and verifies
against verifier-owned metadata, but this saves only about `0.9 KiB` per L2
row. The suspected large overhead is therefore not generic proof metadata; it
is the path dictionary plus remaining core opening material.

The natural follow-up was to pair the fast L1 profile (`lb=3,nq=20,cap=4`)
with the compact final-layer projection. A pre-fix release/native diagnostic
built and verified the fast L1 proof, then failed while self-verifying the L2
proof with `GlobalCumulativeMismatch(None): WitnessChecks`. Lookup debugging
isolated the mismatch to Tip5 Merkle-direction witness binding: when the
direction bit was `1`, the row's input lookup claimed the running-digest witness
id while the value was the sibling value. The fixed Tip5 AIR below supersedes
that failed run, and the current fast-L1 measurements are recorded after the
fix.

The Tip5 MMCS gap has since been fixed in the batch-STARK Tip5 wrapper AIR:

- the Tip5 row now carries the resolved `mmcs_bit` in a wrapper-owned main
  column;
- the preprocessed CTL block now carries `mmcs_bit_ctl` and `mmcs_bit_idx`;
- the AIR boolean-constrains the direction bit when present and sends
  `[mmcs_bit_idx, mmcs_bit]` on `WitnessChecks`;
- input-side `WitnessChecks` now selects the pre-swap value for static input
  indices when `mmcs_bit=1`, while the lookup AIR still proves the post-swap
  Tip5 permutation input used for the native Merkle root.

The regression
`test_tip5_mmcs_direction_one_ctl_lookups` proves and verifies a direction-bit
`1` Merkle row whose running digest is a prior Tip5 output. The broader
`test_tip5_lookups` file passes, and the release Stage-4 Tip5-throughout L2
wrapper now accepts (`L1=185,821` bytes, `L2=188,541` bytes, full test
`20.11s`).

After that fix, the AI-PoW statement-bound L2 sweep completed again on the
older high-blowup L1 profile. This table is retained as historical evidence for
the compact wrapper and cap-height tradeoff, but the fast-L1 table below is the
current production-candidate measurement after the final D=2 public-binding
correction. A release/native rerun on 2026-06-06 measured the actual
`GoldilocksTip5PathPrunedCompactBatchStarkProof` wrapper as well as the older
projection:

| Shape | Final L2 proof | Path-only projection | Preprocessed-omitted projection | Actual compact wrapper | Metadata-free compact body | Core compact `BatchProof` | L2 prove time | Shared L1 witness proof |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| L2 `lb=4,nq=15,cap=4,pow=0` over L1 `lb=6,nq=10,cap=4,pow=0` | `209,802` bytes | `203,540` bytes | `158,800` bytes | `160,826` bytes | `159,945` bytes | `102,178` bytes | `12.571s` | L1 `173,868` bytes, path-only `170,306`, preprocessed-omitted `136,081`, L1 prove `192.807s`, verify `23ms` |
| L2 `lb=5,nq=12,cap=4,pow=0` over same L1 | `176,628` bytes | `172,820` bytes | `134,877` bytes | `138,707` bytes | `137,816` bytes | `87,207` bytes | `24.318s` | same |
| L2 `lb=6,nq=10,cap=4,pow=0` over same L1 | `160,762` bytes | `157,679` bytes | `124,344` bytes | `127,133` bytes | `126,251` bytes | `77,364` bytes | `48.074s` | same |

The actual compact wrapper adds roughly `2-4 KiB` over the optimistic
preprocessed-omitted projection, builds in about `3-4ms`, and the
metadata-free body verifies in `34-41ms` in this diagnostic. The
metadata-free body saves only `881-891` bytes over the wrapper. The much larger
gap between the core compact `BatchProof` and the body, about `49-58 KiB`, is
the pruned-path/Fri-shape restoration payload. On this older high-blowup L1
profile, the final-layer `lb=5,nq=12` metadata-free compact body is inside the
relaxed `150 KiB` and `<30s` final-layer gates (`137,816` bytes, `24.318s`),
but the L1 proof is far too slow. The fast-L1 measurement below supersedes this
as the current production-candidate row.

A cap-height sweep on the relaxed-final-layer row confirms that `cap=4` is the
best measured point for this L2 shape. Raising the cap height does reduce the
path restoration payload, but it moves more bytes into commitments and the core
`BatchProof` than it saves. Lowering the cap height keeps commitments small but
increases the restoration payload.

| L2 `lb=5,nq=12,pow=0` over L1 `lb=6,nq=10,cap=4,pow=0` | Metadata-free compact body | Core compact `BatchProof` | Restoration payload | Input paths | Commit paths | Pruned siblings | L2 prove |
|---|---:|---:|---:|---:|---:|---:|---:|
| `cap=2` | `140,056` bytes | `82,546` bytes | `57,510` bytes | `28,429` bytes | `29,074` bytes | `1,200` | `25.422s` |
| `cap=4` | `137,816` bytes | `87,207` bytes | `50,609` bytes | `25,840` bytes | `24,762` bytes | `1,056` | `24.403s` |
| `cap=6` | `146,931` bytes | `105,475` bytes | `41,456` bytes | `22,400` bytes | `19,049` bytes | `864` | `24.320s` |
| `cap=8` | `205,134` bytes | `172,198` bytes | `32,936` bytes | `18,987` bytes | `13,942` bytes | `684` | `24.174s` |

All four cap-height rows verify as compact bodies. The sweep rules out cap
height alone as the route under `~100 KiB`: even the best measured row remains
about `37.8 KiB` above the hard target, and the cap choices that shrink path
payload make the core proof substantially larger.

The same relaxed-final-layer row was swept over soundness-neutral FRI
final-polynomial and folding shape:

| L2 `lb=5,nq=12,cap=4,pow=0` over L1 `lb=6,nq=10,cap=4,pow=0` | Metadata-free compact body | Core compact `BatchProof` | Restoration payload | Path sets | Pruned siblings | L2 prove |
|---|---:|---:|---:|---:|---:|---:|
| `lfp=0,mla=3` | `138,170` bytes | `88,576` bytes | `49,594` bytes | `9` | `1,034` | `24.512s` |
| `lfp=1,mla=3` | `141,282` bytes | `88,034` bytes | `53,248` bytes | `9` | `1,110` | `24.273s` |
| `lfp=2,mla=2` | `146,316` bytes | `86,990` bytes | `59,326` bytes | `10` | `1,238` | `24.317s` |
| `lfp=2,mla=3` | `137,816` bytes | `87,207` bytes | `50,609` bytes | `8` | `1,056` | `24.238s` |
| `lfp=2,mla=4` | `137,766` bytes | `88,118` bytes | `49,648` bytes | `8` | `1,036` | `24.345s` |

All rows verify as compact bodies. The best measured alternate, `lfp=2,mla=4`,
saves only `50` bytes relative to the current `lfp=2,mla=3` row. This
effectively rules out FRI fold/final-poly tuning as the missing hard-size lever
for the batch-STARK L2 route.

The next check measured the actual compact path dictionaries against an ideal
Merkle multiproof frontier on the current relaxed-final-layer row. The current
predecessor-suffix encoding is already close to that lower bound:

| L2 `lb=5,nq=12,lfp=2,mla=3,cap=4,pow=0` over L1 `lb=6,nq=10,cap=4,pow=0` | Metadata-free compact body | Restoration payload | Path sets | Current pruned siblings | Ideal frontier siblings | Digest-only frontier savings | L2 prove |
|---|---:|---:|---:|---:|---:|---:|---:|
| Actual query set | `137,816` bytes | `50,609` bytes | `8` | `1,056` | `1,024` | `1,280` bytes | `24.608s` |

This rules out a direct Merkle-frontier rewrite as the missing hard-size lever
for this batch-STARK L2 shape. It would save only `32` Tip5 digests before any
frontier-position metadata. It would also be more invasive than the current
restoration format because a true multiproof verifier would need to reconstruct
internal nodes from opened values plus frontier siblings, or replace the
upstream verifier path, rather than simply restoring ordinary per-query
authentication paths.

Pearl's checked-in proof fixture gives a concrete reference point:
`pearl/zk-pow/fixures/stark_proof.bin` is `59,724` bytes total, split into
`164` bytes of public data, a `22` byte proof preamble, and `59,538` bytes of
compact final Plonky2 proof. Its preamble records `pow_bits=[18,18,22]` and
`rate_bits=[2,3,7]`. Pearl's compact proof omits the final Plonky2
constants/sigmas oracle openings and Merkle proofs, then verifies against
cached verifier data and verifier-recomputed public inputs. That maps to the
same sound pattern as our canonical-metadata and preprocessed-oracle omission:
omitted data must be verifier-deterministic and recomputed from pinned
verifier state, not supplied as prover hints.

The analogous verifier-deterministic oracle in the current Plonky3
batch-STARK L2 body is already omitted. The remaining input batches are
prover-dependent trace, quotient, permutation, and random oracles. To test
Pearl's high-blowup final-stage clue without relying on proof-system PoW, the
pure-query profile factory was relaxed to reject weaker profiles but allow
stronger ones. The measured no-PoW `rate_bits=7` analogue uses `lb=7,nq=9`
for `63` Johnson bits:

| L2 high-blowup row over L1 `lb=6,nq=10,cap=4,pow=0` | Metadata-free compact body | Core compact `BatchProof` | Restoration payload | Path sets | Pruned siblings | Ideal frontier siblings | L2 prove |
|---|---:|---:|---:|---:|---:|---:|---:|
| `lb=7,nq=9,lfp=2,mla=3,cap=4,pow=0` | `120,722` bytes | `72,415` bytes | `48,307` bytes | `9` | `1,008` | `972` | `97.358s` |

This is the smallest measured Plonky3 batch-STARK final body so far, but it
still misses both production gates: it is above the hard `~100 KiB` target and
the final-layer proof alone is over three times the `30s` time budget. The
shared L1 witness proof in the same run took `196.064s`. Pearl's small fixture
therefore does not translate as "use rate 7" in this stack; the larger gap is
the proof system and recursive-relation shape, plus Pearl's explicit use of
proof-system PoW in its production parameters.

The fast-L1 follow-up with L1 `lb=3,nq=20,cap=4,pow=0` completes and verifies
after the Tip5 MMCS direction-binding fix and the final D=2 L2 public-binding
correction:

| Shape | Final L2 proof | Path-only projection | Preprocessed-omitted projection | Actual compact wrapper | Metadata-free compact body | Core compact `BatchProof` | L2 prove time | Shared fast L1 witness proof |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| L2 `lb=4,nq=15,lfp=2,mla=3,cap=4,pow=0` over L1 `lb=3,nq=20,cap=4,pow=0` | `226,891` bytes | `219,974` bytes | `172,738` bytes | `178,272` bytes | `177,394` bytes | `106,907` bytes | `27.342s` | L1 `279,719` bytes, path-only `268,439`, preprocessed-omitted `210,823`, L1 prove `30.448s`, verify `33ms` |
| L2 `lb=5,nq=12,lfp=2,mla=3,cap=4,pow=0` over same L1 | `191,362` bytes | `187,086` bytes | `147,107` bytes | `149,743` bytes | `148,866` bytes | `91,402` bytes | `54.137s` | same |
| L2 `lb=6,nq=10,lfp=2,mla=3,cap=4,pow=0` over same L1 | `168,604` bytes | `165,492` bytes | `130,299` bytes | `132,682` bytes | `131,803` bytes | `80,948` bytes | `107.617s` | same |

This no longer rules out the fast-L1/two-layer batch-STARK route. It is now the
committed primary production route. The selected row is L2 `lb=5,nq=12`: it is
inside the relaxed final-proof size gate with explicit public binding, but it
is still too slow. In the full three-row sweep it takes `54.137s` for the L2
proof alone and about `84.585s` with the L1 proof timer added; the focused
cached-prep timing breakdown below also includes AIR next-row declaration
forwarding, so the selected row is smaller and faster than that older sweep.
The L2 `lb=4,nq=15` row is closer to the time gate, but it is too large. The L2
`lb=6,nq=10` row has ample size headroom, but its proving time is too high. The
immediate engineering target is therefore not a new proof family; it is
reducing the selected compact batch-STARK path's total L1+L2 proving time.

A focused selected-row timing diagnostic narrows the remaining blocker. It
reruns only the selected `lb5,nq12` row after the next-row forwarding fix, so
timings and bytes differ from the older full three-row sweep:

| Selected fast-L1/L2 row | Actual compact wrapper | Metadata-free compact body | Reusable L1 prep | Cached L1 prove | Total L1 prove | Reusable L2 prep | Cached L2 prove | Uncached L2 total | L2 witness run | L2 STARK prove |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| L1 `lb=3,nq=20,cap=4,pow=0`; L2 `lb=5,nq=12,lfp=2,mla=3,cap=4,pow=0` | `143,762` bytes | `142,878` bytes | `4.772s` | `15.305s` | `20.077s` | `9.364s` | `28.726s` | `38.090s` | `38ms` | `28.667s` |

The follow-up L1 table-packing sweep keeps the same FRI shapes and compact L2
serialization, but varies the inner L1 table packing before recursively proving
the L1 proof. This is the latest best measured complete compact batch-STARK
route:

| L1 table packing | L1 outer proof | L1 cached prove | L2 compact wrapper | Metadata-free L2 body | L2 cached prove | Cached serial L1+L2 |
|---|---:|---:|---:|---:|---:|---:|
| `alu_lanes=2,horner_k=5` | `240,303` bytes | `15.956s` | `141,148` bytes | `140,260` bytes | `28.435s` | `44.391s` |
| `alu_lanes=4,horner_k=5` | `251,897` bytes | `15.029s` | `143,106` bytes | `142,225` bytes | `28.555s` | `43.584s` |
| `alu_lanes=8,horner_k=5` baseline | `278,037` bytes | `15.472s` | `143,762` bytes | `142,878` bytes | `28.421s` | `43.893s` |
| `alu_lanes=16,horner_k=5` | `330,420` bytes | `16.792s` | `149,688` bytes | `148,803` bytes | `28.981s` | `45.773s` |

This reinforces the route decision. Compact batch-STARK is now the closest
soundly-bound route to the relaxed target, and batch STARK is acceptable if it
is the smaller, faster, explicitly-bound proof path. The remaining work is not
to switch back to the native terminal backend or the large checkpoint envelope;
it is to cut or overlap the `~15s` L1 proving stage and the `~28.5s` L2 STARK
stage.

The recursive verifier circuit and witness execution are not the bottleneck:
definition/build/input packing sum to under `100ms`, L1 witness execution is
`58ms`, L2 witness execution is `38ms`, reusable L1/L2 setup is now split out
of the cached proof path, and compatible cached `ProverData` removes the
repeated preprocessed-commitment rebuild inside `prove_all_tables`. Span
profiling shows quotient evaluation is under `1s`; the expensive piece is the
upstream batch-STARK/PCS proof itself.

The selected L2 verifier trace profile makes the immediate threshold concrete.
The circuit has `75,391` ops: `66,564` primitive ops, `1,981` hints, and
`6,846` non-primitive ops. Non-primitives are `4,791` Tip5 rows, `2,001`
`recompose/coeff` rows, and `54` regular recompose rows. The Tip5 table is
padded to `8,192` rows, so reducing the selected verifier by `695` Tip5 rows
would cross the `4,096` boundary and should materially reduce the dominant
main/permutation trace Merkle commitments. The split is not purely MMCS:
`2,220` compiled Tip5 op IDs are MMCS private-data ops, while `2,571` are
non-MMCS rows; the generated trace has `2,620` rows with explicit `mmcs_bit`
bindings.

A phase-tagged full-transcript diagnostic then accounts for every Tip5 row:

| Tip5 phase | Rows |
|---|---:|
| MMCS Merkle-path sibling compressions | `2,220` |
| MMCS base-field leaf hashes | `1,620` |
| MMCS path digest-injection compressions | `400` |
| MMCS extension-element leaf hashes | `180` |
| Fiat-Shamir challenger duplexes | `371` |

The challenger rows split as:

| Challenger phase | Tip5 rows |
|---|---:|
| Opened permutation values | `88` |
| Opened trace values | `84` |
| Opened preprocessed values | `69` |
| PCS challenge derivation | `58` |
| Permutation commitment + global cumulatives | `30` |
| Preprocessed commitment/widths | `9` |
| Trace commitments + public values | `9` |
| Quotient commitment | `8` |
| Quotient opened values | `8` |
| Instance shape binding | `5` |
| PCS verify transcript | `2` |
| Lookup challenge sampling | `1` |

This rules out the earlier "unknown non-MMCS/non-challenger" hypothesis: those
rows are ordinary PCS/MMCS leaf and path hashing rows that were not returned as
private-data op ids. A profile-only lower bound that skips deterministic
preprocessed opened-value transcript observations removes only `70` Tip5 rows:
`4,791 -> 4,721`. The table still pads to `8,192` rows, with `625` rows over
the previous power-of-two boundary. A verifier-key/setup-digest transcript may
still be useful for wire-size cleanliness, but it is not the primary time lever.

The natural query-count retune was then measured directly. Switching L1 from
the selected `lb=3,nq=20` shape to `lb=4,nq=15` removes one quarter of the
query-driven MMCS work in the L2 verifier. The row profile crosses the desired
Tip5 boundary:

| L1 shape, selected L2 `lb=5,nq=12` | Tip5 rows | Tip5 padded height | MMCS sibling rows | Base leaf rows | Extension leaf rows | Path injection rows |
|---|---:|---:|---:|---:|---:|---:|
| `lb=3,nq=20,cap=4` | `4,791` | `8,192` | `2,220` | `1,620` | `180` | `400` |
| `lb=4,nq=15,cap=4` | `3,851` | `4,096` | `1,830` | `1,215` | `135` | `300` |

That crossing is not enough to meet the total time gate. The full one-row
timing diagnostic for L1 `lb=4,nq=15,cap=4` and L2
`lb=5,nq=12,lfp=2,mla=3,cap=4` reports compact wrapper `142,649` bytes and
metadata-free compact body `141,769` bytes, but L1 cached proving rises to
`29.584s` (`38.977s` with L1 prep) and L2 cached proving remains `28.743s`
(`38.115s` with L2 prep). Cached serial L1+L2 proving is therefore about
`58.327s`, worse than the selected `lb=3,nq=20` L1 row. The L2 prove time
staying flat after halving the Tip5 padded height means the remaining
commitment bottleneck is not solved by Tip5-row reduction alone. Later
table-packing measurements show that ALU lane count retuning only trades bytes
against time, so the remaining lever is real verifier-operation reduction,
cross-stage overlap, or a proof shape that avoids committing the same large
verifier matrices at the current L2 LDE volume.

A hidden-L1 cap-height profile checked the cheapest possible way to reduce
MMCS path hashing before redesigning the verifier relation. It is not enough.
With the same selected L2 shape, L1 cap `3` profiles at `4,967` Tip5 rows and
L1 cap `4` profiles at `4,791` rows. L1 cap `5` and `6` proofs verify natively
in `32-33ms`, but the current L2 recursive verifier rejects both during witness
execution with `WitnessConflict`. The cap `3 -> 4` slope saves only `176` Tip5
rows, so even after fixing the higher-cap recursive-verifier support gap, cap
retuning alone is unlikely to remove the needed `695` rows without increasing
other circuit volume.

A deeper release/native selected-row profile with
`AI_POW_ZK_DEEP_BATCH_PROFILE=pcs`, run before next-row opening forwarding,
makes that upstream bottleneck concrete. It measured the old `149,743` byte
compact wrapper, with L1 prove `22.501s`, reusable L2 prep `10.475s`, cached
L2 prove `31.815s`, uncached L2 total `42.290s`, and L2 STARK prove `31.756s`.

| Cached L2 selected proof span | Time | Dimensions / implication |
|---|---:|---|
| Main trace Merkle commitment | `13.3s` | `[2x8192, 20x8192, 86x262144, 118x1048576, 2x8192, 2x131072]`; dominated by the Tip5 verifier table LDE |
| Permutation trace Merkle commitment | `12.9s` | `[2x8192, 20x8192, 80x262144, 120x1048576, 2x8192, 6x131072]`; same Tip5-scale LDE appears in the permutation argument |
| Quotient computation | `<1s` | Largest single AIR quotient span is Tip5 at about `632ms` |
| Quotient-chunk Merkle commitment | `3.12s` | Multiple two-column quotient chunks, including repeated `2x1048576` chunks |
| FRI commit/query | `704ms` | Query phase is sub-millisecond; FRI folding/query is not the current blocker |

This is why the compact batch-STARK route is closer than the earlier
native-terminal direction but still not production-complete. The final artifact
is already inside the relaxed size gate, and the recursive verifier circuit
definition/build/input packing/witness path is negligible. Generic Rayon
tuning is unlikely to close the gap because the default parallel feature is on
and the Merkle tree code already parallelizes chunk hashing. ALU lane retuning
is now also measured as a size reserve rather than a time lever. The next
meaningful implementation lever is to reduce the committed matrix volume of
the recursive verifier, especially the Tip5/MMCS verification tables at
`2^20` LDE in L2, overlap the L1/L2 proving stages, or use a verifier
relation/proof shape that avoids committing those large Tip5/permutation
traces.

A focused fast-L1 `lb4,nq15` frontier sweep then checked whether the faster
L2 row could be tuned under the relaxed size gate without proof-system PoW:

| L2 `lb=4,nq=15,pow=0` over fast L1 `lb=3,nq=20,cap=4,pow=0` | Actual compact wrapper | Metadata-free compact body | Core compact `BatchProof` | Restoration payload | L2 prove |
|---|---:|---:|---:|---:|---:|
| `lfp=0,mla=3,cap=4` | `179,992` bytes | `179,111` bytes | `109,224` bytes | `69,887` bytes | `24.712s` |
| `lfp=1,mla=3,cap=4` | `181,257` bytes | `180,376` bytes | `108,151` bytes | `72,225` bytes | `25.155s` |
| `lfp=2,mla=2,cap=4` | `179,534` bytes | `178,656` bytes | `106,328` bytes | `72,328` bytes | `28.426s` |
| `lfp=2,mla=3,cap=2` | `178,766` bytes | `178,452` bytes | `101,285` bytes | `77,167` bytes | `24.566s` |
| `lfp=2,mla=3,cap=4` | `178,272` bytes | `177,394` bytes | `106,907` bytes | `70,487` bytes | `24.815s` |
| `lfp=2,mla=3,cap=6` | `189,204` bytes | `186,041` bytes | `129,707` bytes | `56,334` bytes | `24.568s` |
| `lfp=2,mla=4,cap=4` | `174,676` bytes | `173,798` bytes | `109,981` bytes | `63,817` bytes | `24.456s` |

This rules out cheap FRI final-polynomial, folding-arity, or cap-height tuning
as the way to combine the `lb4,nq15` proving time with the relaxed size gate.
Even the best `lb4,nq15` wrapper remains `24,676` bytes above a decimal
`150,000` byte gate. The primary row therefore stays `lb5,nq12`, and the next
optimization target is core L2 proving time after cached setup rather than a
simple shape retune.

Compact verifier artifacts now exist for the verifier-deterministic
preprocessed material in that projection.

`GoldilocksTip5PathPrunedCompactVerifierContext` is the current verifier-owned
contract for the metadata-free compact body. It binds the trusted
`GoldilocksTip5BatchStarkProofMetadata`, canonical `CircuitProverData`, expected
`GoldilocksTip5FriShape`, and final public values into one API boundary before
restoration. The context verifier rejects metadata/setup binding mismatches,
FRI-shape mismatches, malformed public-value lengths, and wrong public values.
The AI-PoW selected-row diagnostic now verifies compact bodies through this
context entrypoint.

`PreprocessedOodCompactBatchStarkProof` is the generic first step. It consumes
a full `BatchStarkProof`, omits the verifier-deterministic
`preprocessed_local`/`preprocessed_next` OOD vectors from
`BatchProof.opened_values`, and verifies only when the caller supplies the
canonical `CircuitProverData` whose preprocessed commitment and metadata match
the proof's serialized `stark_common` binding. Verification replays the
batch-STARK transcript through `zeta`, recomputes the omitted preprocessed
polynomial evaluations from canonical setup data, rejects any serialized value
that disagrees, restores missing values, then calls the normal upstream
`p3-batch-stark` verifier.

`GoldilocksTip5PreprocessedCompactBatchStarkProof` is the PCS-specific next
step for the production-candidate Goldilocks/Tip5 path. It also removes the
preprocessed commitment's FRI input batch from each query proof. Verification
uses explicit `GoldilocksTip5FriShape` metadata only as a restoration hint,
checks the canonical setup binding, restores preprocessed OOD values from the
committed bit-reversed Tip5 LDE exactly as `TwoAdicFriPcs::open` does, replays
the PCS transcript through FRI query-index sampling, regenerates the omitted
`Mmcs::open_batch` results from canonical preprocessed prover data, inserts the
missing batches, and then delegates to normal upstream verification. The tests
`test_goldilocks_tip5_compact_preprocessed_fri_round_trip_uses_canonical_setup`
and `test_goldilocks_tip5_compact_preprocessed_fri_rejects_wrong_setup_binding`
cover ordinary-verifier rejection, OOD-only verifier rejection, byte-for-byte
restoration against the original full proof, successful compact verification,
and wrong-setup rejection.

`GoldilocksTip5PathPrunedCompactBatchStarkProof` implements the next portion of
the measured projection. Its constructor consumes a full Goldilocks/Tip5 proof,
rebuilds the verifier statement, replays the Fiat-Shamir transcript to derive
FRI query indices, prunes/deduplicates binary Merkle authentication paths for
the remaining input batches and FRI commit-phase openings, then applies the
preprocessed-omission adapter. Verification rejects inner proofs that carry
non-empty auth paths, checks dictionary leaf indices against transcript-derived
query indices, restores the exact paths from the compact dictionaries, restores
preprocessed OOD and input-batch openings from canonical setup data, and then
delegates to normal upstream `p3-batch-stark` verification. The tests
`test_goldilocks_tip5_path_pruned_compact_round_trip_restores_full_proof` and
`test_goldilocks_tip5_path_pruned_compact_rejects_tampered_merkle_path` cover
byte-for-byte full-proof restoration, ordinary-verifier rejection, compact
verification, tampered leaf-index rejection, and tampered-pruned-path
rejection.

`GoldilocksTip5PathPrunedCompactBatchStarkProofBody` is the canonical-metadata
wire-shape variant of that adapter. It carries the compact `BatchProof`, FRI
shape, and pruned-path dictionaries, but drops all `BatchStarkProof` metadata.
Verification rehydrates the proof with a verifier-owned
`GoldilocksTip5BatchStarkProofMetadata` template before running the same
restoration and upstream verification path. The metadata template must be
rebuilt or pinned by the verifier for the exact statement; accepting it from
the prover would make the compact body self-describing and would reintroduce
noncanonical metadata risk. The tests
`test_goldilocks_tip5_path_pruned_compact_body_uses_canonical_metadata` and
`test_goldilocks_tip5_path_pruned_compact_body_rejects_wrong_metadata` cover
body serialization/verification and rejection under a different canonical
metadata/setup pair.

This checkpoint now wires that adapter as the compact recursive certificate API
inside `ai-pow-zk` and through the Rust bridge/miner noun path as canonical
compact bytes; it still does **not** wire it through the Hoon verifier path or
production-pinned expected verifier-key/setup digest. The final
Pearl-compatible `%ai-pow` wire-size measurement now passes the relaxed size
gate: `141,916` jammed bytes with a `141,103` byte compact certificate.
It removes the prior public-binding measurement blocker and shows that the
compact batch-STARK route is closer to the relaxed target than the native
terminal route. Omitting any additional value without transcript replay and
reconstituting the same PCS openings would be an unsound prover hint. The
remaining promotion work requires a verifier-key contract that pins or rebuilds
the L1 and L2 metadata from canonical code/config rather than from
prover-supplied bytes, plus release/native proof-time work on the selected
fast-L1/L2 row.

The polynomial NPO path remains useful diagnostic evidence, but it is not a
drop-in production replacement for the exhaustive NPO proof. The recursion-crate
synthetic Tip5-only integrated-LogUp checkpoint measures below the byte and
time targets:

| Synthetic integrated-LogUp checkpoint | Bytes / Time |
|---|---:|
| Bundled masked-IO NPO checkpoint | `95,201` bytes / `93.0 KiB` |
| Primitive + bundled NPO production-candidate body | `96,017` bytes / `93.8 KiB` |
| NPO prove time | `9.913s` |
| Total primitive + NPO prove time | `9.918s` |
| Total verify time | `62.5ms` |

That test is intentionally small. It proves a synthetic NPO-only Tip5 circuit,
not the full `ai-pow-zk` composite verifier. A full composite diagnostic,
`terminal_integrated_logup_candidate_for_pure_query_lb6_nq10_measures`, now
builds the actual L1 verifier circuit, binds the assignment root plus the
selected+lookup and bundled trace roots, proves the primitive row-product
component, proves the merged value bridge from the prepared selected+lookup
PCS data, and then attempts the integrated polynomial NPO proof. The latest
release/native run reached `14.435s` primitive prove and `2.340s` merged
padding/value-bridge prove, then stayed inside the integrated Tip5
`air_quotient_matrix` phase for more than 90 seconds and was stopped. This
already violates the `<30s` production proving constraint, so the synthetic
`93.8 KiB` checkpoint must not be treated as evidence that the full composite
recursive certificate path meets the milestone without additional prover
reductions.

A later release/native run with phase instrumentation compiled in `2m00s` and
then isolated the current full-composite costs before stopping the still-running
integrated Tip5 LogUp subproof:

| Full composite integrated-LogUp phase | Time |
|---|---:|
| Layer-0 proof generation for the diagnostic fixture | `29.834s` |
| L1 verifier-circuit build | `0.417s` |
| L1 verifier trace execution | `0.044s` |
| Terminal compile | `7.543s` |
| Assignment oracle commitment | `5.793s` |
| Selected+lookup root construction | `11.075s` total, including `3.85s` selected+lookup commit |
| Bundled trace root construction | `6.261s` total, including `5.84s` trace-bundle commit |
| Terminal prelude build | `7.600s` |
| Primitive R1CS row-product proof | `14.435s` |
| Merged padding/value-bridge proof | `2.340s` |
| Integrated Tip5 LogUp proof | stopped after more than 90 seconds inside `air_quotient_matrix` |

The cumulative recursive-side work before the integrated Tip5 LogUp proof
finishes is already far beyond the production proving budget. The selected
lookup and trace-bundle root construction phases are now reused by the subproof
provers instead of rebuilt internally, which cuts merged proving to `2.340s`.
That still cannot make this candidate production-viable by itself: primitive
proving remains `14.435s`, and the integrated Tip5 AIR quotient had not
completed.

A later PROD merged padding/value-bridge run fixes the Merkle-direction value
projection, folds the padding/MMCS-bit quotient into the same merged FRI
object, and reaches a verifier-accepted body. After removing duplicated
primitive relation/assignment construction, preparing merged NPO data once for
the prelude, using prelude-checked prover entry points, and replacing the
padding and value-bridge quotient per-point work with batched coset LDEs under
one combined quotient commitment, the serialized `(prelude, primitive, merged
NPO)` body is `151,448` bytes and post-prelude serial proof-body construction
is `14.914s` (`8.372s` primitive plus `6.541s` merged NPO). That makes it the
best full-composite partial checkpoint so far. The most promising complete
proof shape is the integrated-LogUp bundled masked-IO path measured above at
`96,017` bytes / `9.918s` in the synthetic backend, but its full-composite
integrated subproof still misses the time gate. Neither checkpoint is a
production certificate yet: the merged full-composite proof leaves the Tip5
lookup/AIR/LogUp relation outside the theorem, and the integrated proof shape
still needs full-composite prover-time reduction.

The older two-subproof polynomial NPO production candidate had a precise size
blocker:

| Component | Bytes | Notes |
|---|---:|---|
| Primitive R1CS row-product proof | `21,709` in the production body | Not the size blocker |
| `TerminalProductionNpoPolynomialProof` | `204,039` | Dominates the body |
| `merged_value_bridge_proof` | `67,133` | FRI proof for residual-zero/recompose/value bridge |
| `integrated_logup_proof` | `136,906` | FRI proof for Tip5 AIR, byte LogUp, and selected-vs-trace NPO-IO LogUp |

The important consequence was that generic serialization compression or small
primitive-R1CS tweaks could not make the polynomial NPO payload production
sized. The viable near-term path was to re-audit and promote exhaustive
supported-NPO checking.

## Current Pipeline

The production terminal entrypoint is
`crates/plonky3-recursion/recursion/src/terminal.rs::prove_terminal_production_goldilocks`.
For a verifier key with supported Tip5/recompose NPO rows it does the following:

1. Validate the canonical terminal production parameters.
2. Verify the full terminal assignment and all registered NPO traces with
   `verify_assignment_with_goldilocks_npos`.
3. Commit the assignment oracle.
4. Build one production prelude binding exactly the assignment root.
5. Prove the primitive sparse-R1CS row-product component.
6. If the verifier key has supported NPO rows, prove
   `TerminalNpoExhaustiveProof`, which opens every verifier-derived
   Tip5/recompose callsite against the same assignment oracle.

Verification rejects extra production prelude commitments, verifies the
primitive row-product proof against the assignment root, then verifies every
supported NPO row deterministically. There is no sampled NPO validity path and
no terminal query PoW counted for NPO checking.

The terminal certificate wrapper now uses postcard encoding and a structural
round-trip assertion for the terminal public inputs plus certificate. The
recursive verifier-key rebuild also has a deterministic-header regression. This
was necessary for soundness: before the fix, the same Layer-0 proof could
rebuild a different terminal relation digest because global lookup cumulative
checks were emitted through hash-map value iteration in the recursive verifier
circuit. The builder now emits those checks in sorted name order, so the
terminal relation digest is a stable cryptographic binding rather than an
artifact of hash iteration order.

The current compact known-index multiproof encoding stores field limbs and
frontier digests as fixed little-endian bytes. This reduced the same
`lb=6,nq=10,pow=0` wire measurement from `891,780` bytes to `771,249` bytes,
but it did not change the structural bottleneck:

| Full composite terminal body component | Bytes |
|---|---:|
| Full production proof body | `765,844` |
| Primitive R1CS row-product proof | `52,821` |
| Exhaustive NPO proof | `712,830` |
| Exhaustive NPO hidden Tip5 values | `92,802` |
| Exhaustive NPO assignment-witness multiproof | `620,028` |

The NPO assignment-witness multiproof still opens `47,814` assignment values
and carries `5,434` Merkle frontier nodes. Ordinary encoding work cannot close
the remaining gap to about `100 KiB`; a production-sized path has to avoid
exhaustively opening this many assignment values.

The latest full measurement decomposes that multiproof further:

| Assignment-witness multiproof component | Bytes / Count |
|---|---:|
| Nonzero value limbs | `382,515` bytes |
| Sparse nonzero masks | `20,126` bytes |
| Boolean bits | `25` bytes |
| Merkle frontier | `217,362` bytes |
| Estimated non-boolean opened values | `80,492` |
| Nonzero coefficients | `47,814` |
| Zero coefficients already elided | `113,170` |

The existing sparse coefficient encoding has already removed about `905 KiB`
of dense zero coefficients. The remaining size is mostly nonzero value limbs
and Merkle authentication data, so further varint/fixed-width encoding tweaks
are not enough.

The latest measurement also printed useful comparison floors:

| Candidate | Bytes | Interpretation |
|---|---:|---|
| Full NPO polynomial FRI opening candidate | `48,803` | A single opening over 668 rows and 186 field columns is much smaller than the current two-subproof NPO body |
| NPO value-column FRI candidate | `30,325` | Value columns alone are not expensive enough to explain the current size |
| Sparse R1CS matrix sumcheck | `20,873` | Primitive matrix component is already small enough |
| R1CS row-product sumcheck | `22,631` | Assignment fold openings dominate this component, but it is not the main target |

## Pearl/Plonky2 Reference: What Actually Makes Its Proof Small

The Pearl implementation is useful evidence that the target size is plausible
with a STARK-family proof, but the mechanism is not "batch-STARK the recursive
verifier harder." In the read-only Pearl checkout, the submitted `ZKProof`
contains only a 22-byte preamble
(`pow_bits[3] | rate_bits[3] | zeta[16]`) plus the final compact Plonky2 proof
bytes (`pearl/zk-pow/src/api/proof.rs` and `proof_utils.rs`). The public proof
data is separate and fixed-size, and verification reconstructs the final proof
public inputs from public params, cached verifier data, and deterministic
preprocessed columns.

The architecture is materially different from the current full
`ai-pow-zk` terminal path:

- Pearl proves the AI-PoW computation with a specialized `PearlStark` AIR, not
  with a generic recursive-verifier terminal relation. The AIR interleaves input,
  Blake3, matmul, and jackpot chips in one trace
  (`pearl/zk-pow/src/circuit/pearl_air.rs`).
- Its "program" side is encoded as preprocessed control/noise/routing columns.
  Starky commits online and preprocessed trace columns in one Merkle oracle,
  absorbs a preprocessed/public-data digest before deriving challenges, and
  recursively connects the preprocessed openings at `zeta` and `g*zeta`
  (`pearl/plonky2/starky/src/prover.rs` and
  `recursive_verifier.rs`).
- The first recursive circuit verifies the base STARK proof and exposes the
  base public inputs, public-data commitment, STARK `zeta`, and preprocessed
  evaluations as public inputs. Verification later recomputes those
  preprocessed evaluations natively from public parameters instead of carrying
  the base STARK proof on the wire
  (`pearl/zk-pow/src/circuit/pearl_circuit.rs`).
- The second recursive circuit verifies the first recursive proof and serializes
  only a compact final proof. Pearl's `CompactProofWithPublicInputs` omits
  deterministic `constants_sigmas` evaluations and Merkle proofs from FRI query
  rounds, then reconstructs them during verification from trusted cached
  polynomial coefficients (`pearl/plonky2/plonky2/src/plonk/proof.rs`).
- Pearl explicitly binds a gap in its second-recursion verifier by exposing the
  first circuit's `constants_sigmas_cap` and `circuit_digest` as public inputs,
  because `builder_2.verify_proof` alone does not prove that the cap is related
  to the digest.

That explains why Pearl does not pay our measured `620,028` byte
assignment-witness multiproof cost. It does not terminalize a generic composite
verifier and then authenticate tens of thousands of verifier-assignment values.
It proves a purpose-built AIR, recursively compresses that proof twice, and
puts only the compact second-recursion proof on the wire.

The current Plonky3 L1 verifier input split is now measured explicitly by
`l1_verifier_input_footprint_for_pure_query_lb6_nq10_composite_is_available`.
For the reduced pure-query profile (`lb=6,nq=10,pow=0`) the release/native run
prints:

| L1 verifier footprint component | Count / Bytes |
|---|---:|
| Statement digest public values | `5` |
| Layer-0 AIR public values | `60` |
| Proof-derived public values | `389` |
| Common-data public values | `5` |
| Total terminal public values | `459` |
| Terminal public input serialization | `5,180` bytes |
| Proof-derived private values | `30,648` |
| L1 circuit fingerprint | `witness_count=168,292`, `ops_len=96,319` |

This refines the Pearl comparison: the current terminal miss is not mainly a
large public-input vector. Most Layer-0 proof openings are already private
witness values in the L1 verifier circuit. The expensive part is proving the
generic verifier relation itself with the native terminal backend, especially
the NPO-heavy Tip5/recompose checks and the exhaustive assignment-opening proof.

The exact Pearl parameters are not directly portable to Nockchain's stated
soundness policy. Pearl's defaults are `pow_bits=[18,18,22]` and
`rate_bits=[1 or 2,3,7]`, with `num_query_rounds =
ceil((120 - pow_bits) / rate_bits)` in all three stages
(`pearl/zk-pow/src/circuit/circuit_utils.rs`). Nockchain's current production
policy is a 60-bit pure-query floor with `query_pow_bits=0`. Therefore Pearl's
high proof-system PoW values are useful as an engineering comparison but cannot
be counted toward Nockchain's production soundness unless that policy is
explicitly changed.

## Pearl-Informed Plonky3-Compatible Tracks

The portable lessons are the proof shape and the bindings, not Plonky2 itself.
Viable Nockchain tracks that do not use Plonky2 directly are:

1. **Specialized AI-PoW base AIR plus recursive compression.** Build a
   Plonky3-native AIR for the actual AI-PoW statement, with matrix/noise/hash
   and jackpot constraints directly in the trace and deterministic public data
   represented as preprocessed columns. This is the closest analogue to Pearl
   and avoids the current generic verifier relation before recursion. It is the
   largest AIR implementation, but it attacks both measured blockers: the
   `106,349` primitive terminal operations and the `14,049` supported NPO rows.
2. **Two-stage Plonky3 recursive compressor with compact final serialization.**
   Keep the current Layer-0 proof or a future specialized AIR proof, then add a
   first recursive verifier circuit and a second proof-compression circuit whose
   on-wire proof omits only deterministic verifier-key openings. This requires
   a Plonky3 analogue of Pearl's compact proof format: cached verifier
   polynomials, public binding of verifier digests/caps, strict verifier-key
   reconstruction, and explicit tests that stale cached polynomials, swapped
   caps, wrong circuit digests, and malformed compact openings are rejected.
   This route must avoid simply wrapping the current L1 verifier in another
   batch-STARK. The measured L2 batch-STARK proof can project below the relaxed
   final-layer size target only after omitting verifier-deterministic
   preprocessed openings, and the pipeline is still too slow once the required
   L1 proof is included.
3. **Preprocessed-program binding instead of assignment-value revelation.**
   Move deterministic verifier/program data out of terminal assignment openings
   and into digest-bound preprocessed columns whose evaluations at verifier
   challenge points are recomputed by the verifier. This is a narrower form of
   the Pearl design, but the new L1 footprint measurement shows it cannot by
   itself close the current gap: public/proof-value exposure is far smaller
   than the exhaustive NPO assignment-opening cost. It remains useful only for
   deterministic verifier-key data that can be omitted from a compact final
   proof without weakening the transcript binding.
4. **Unified STARK/IOP for terminal NPO data.** Continue the Direction 1 work,
   but treat Pearl as evidence that the final proof should be one compact
   recursively compressed object rather than two independent terminal FRI
   payloads plus a large assignment-opening proof. The current integrated
   candidate is too slow, so a viable version has to share commitments,
   challenges, and openings structurally and avoid rebuilding the same matrices.
5. **Pure-query parameter search after the proof shape changes.** Pearl's
   `rate_bits=7,pow_bits=22` final stage is small partly because it counts
   proof-system PoW. For Nockchain, parameter sweeps must keep `pow_bits=0`
   unless the soundness policy changes. The useful search space is therefore
   higher-rate/fewer-query pure-query recursion after compact serialization and
   specialized/preprocessed bindings have reduced the relation.

The expected production route is probably a combination of the first two
tracks: prove the AI-PoW statement with a specialized Plonky3 STARK/AIR, then
recursively compress it to one compact final proof. The current native terminal
backend remains valuable as a verifier-relation diagnostic and fallback, but
its full composite path is paying costs that Pearl's architecture avoids
entirely.

### Concrete Pearl-Shaped Plan Without Plonky2

The important conclusion from Pearl is that the target is achievable by changing
the proof shape, not by retuning the current generic terminalized verifier. A
Plonky3-native route should copy the structural ideas below while keeping the
Nockchain soundness policy (`pow_bits=0` for production accounting) and the
existing Tip5/AI-PoW bindings.

| Pearl ingredient | What Pearl gets | Plonky3-native analogue | Production acceptance gate |
|---|---|---|---|
| Specialized base AIR | The base proof is over the work statement, not over a generic verifier execution | Continue from `CompositeFullAirWithLookupsPinned` or replace it with a narrower dedicated AI-PoW AIR; do not put the final artifact on the raw L0 proof | Base proof verifies the matrix/noise/jackpot work, target hit, nonce/job binding, matrix commitments, and public params with no cache-only witness shortcut |
| Two recursive compression layers | The final proof verifies a proof that already verified the base STARK | Build a first Plonky3 recursive verifier circuit for L0, then a second Plonky3 proof-compression layer over that verifier proof | The on-wire final object is only the L2 compact proof plus explicitly required public data, not L0 and not the L1 batch-STARK checkpoint |
| Compact final proof format | Deterministic verifier-key material is cached and reconstructed by the verifier | Add a compact p3 proof format that omits only verifier-deterministic constants/preprocessed openings and reconstructs them from pinned verifier data | Tampering tests reject stale cached polynomials, swapped verifier caps, wrong circuit digest, wrong preprocessed commitment, wrong public inputs, and malformed compact openings |
| Public verifier-data binding | Pearl exposes `constants_sigmas_cap` and `circuit_digest` because `verify_proof` alone is not enough | Every compact p3 recursive layer must expose or otherwise transcript-bind the previous verifier key digest, cap/root, parameter tuple, and public-input digest | The verifier recomputes those values from canonical code/config and rejects any mismatch before accepting proof bytes |
| Preprocessed/program columns | Deterministic program/routing data is digest-bound and re-evaluated by the verifier | Move deterministic AI-PoW program data and verifier data into digest-bound preprocessed columns where possible | Every omitted value is either verifier-recomputable or still opened from a committed witness; no hidden witness value may become a verifier hint |
| High-rate final layer | Pearl's final stage is small partly because it uses high rate and query PoW | Sweep pure-query final-layer parameters only after compact recursion exists | A candidate must meet at least 60 pure-query Johnson bits without counting proof-system PoW |

This plan deliberately separates three concerns that the current terminal path
mixes together:

1. **Base statement soundness.** The L0 proof must prove the AI-PoW work itself:
   matrix commitments, noised matrix strips, selected tile multiplication,
   jackpot hash, target comparison, and all chain/public metadata bindings.
2. **Recursive compression soundness.** L1/L2 proofs must prove verifier
   execution and bind exactly the verifier parameters, verifier key,
   transcript-visible commitments, and public inputs used by the previous
   layer.
3. **Serialization compactness.** Compact encoding may omit deterministic
   verifier data, but it must never omit a witness value unless the verifier can
   recompute it from public data or another proof obligation already binds it.

The first implementation milestone should therefore not be another
`lb=6,nq=10` terminal measurement. With the Tip5 MMCS direction-binding fix and
direction-bit-`1` regression in place, it should be a p3-native compression
prototype over the existing pinned+LogUp L0 proof with the following outputs:

- compact final-layer reconstruction beyond the implemented Goldilocks/Tip5
  preprocessed OOD, preprocessed FRI input-batch, and path-pruned Merkle
  adapter: final compact serialization, measured L2 integration, and
  production-path integration;
- L1 proof size and proving time for verifying the current pinned+LogUp L0
  proof.
- L2/final proof size and proving time for verifying the L1 proof.
- A compact-vs-full serialization split that identifies exactly which bytes
  are omitted and which verifier-known values reconstruct them.
- Negative tests for every omitted binding: stale verifier data, wrong
  preprocessed commitment, wrong public input vector, wrong L0 proof
  commitments, wrong L1 circuit digest, and wrong final proof public inputs.

If that prototype still lands above `100 KiB`, the next lever is AIR
specialization/narrowing, not higher `log_blowup` alone. The current
`lb=6,nq=10,pow=0` row was useful only as a lower-bound diagnostic for the
generic terminal relation. It should not be treated as the production
inflection point for the terminal fallback. The active compact batch-STARK route
uses fast L1 `lb=3,nq=20` and selected L2 `lb=5,nq=12`; the older
`lb=4,nq=15,pow=0` baseline language applies only to terminal lower-bound
comparisons, not to the committed compact L2 candidate.

### Relaxed 150 KiB Size Gate

If the production size gate can relax from about `100 KiB` to about `150 KiB`,
the existing batch-STARK L1 checkpoint becomes worth re-evaluating, but only as
a new L1-only certificate shape. It does not become production-ready in its
current envelope.

The production-faithful `prod_recursion_measure 15` run already measured the
raw L1 proof body at `149.1 KiB`, which is close to the relaxed byte target.
However, the current `AiPowRecursiveCertificate` serializes the L0 proof and
program with the L1 proof so that verification can rebuild the exact L1
verifier circuit from the submitted L0 proof and reject proof-carried circuit
metadata substitutions. That is why the full checkpoint certificate remains
`1,135.5 KiB` under legacy postcard and `358.3 KiB` even with gzip-best
compression.

The new diagnostic
`relaxed_l1_only_candidate_size_breakdown_for_test_pearl` measures this split
directly for the small `TEST_PEARL` profile:

| Relaxed L1-only diagnostic (`TEST_PEARL`) | Bytes |
|---|---:|
| Current full checkpoint, postcard | `588,162` |
| Current full checkpoint, fixed-int bincode | `1,981,331` |
| Embedded L0 proof | `262,404` |
| Embedded L0 program | `171,908` |
| Full L1 outer object | `153,850` |
| L1 proof body | `152,205` |
| L1 metadata outside proof body | `1,645` |
| L1 public binding lanes | `0` |

The byte split supports the relaxed-size intuition: once the L0 proof/program
context is removed, the L1 object is approximately at the `150 KiB` target and
almost all of it is the actual cryptographic proof body. The same run took
`75.17s` inside the release test binary, however, so this is not yet a
time-qualified production path.

The follow-up diagnostic
`relaxed_l1_only_statement_bound_candidate_size_breakdown_for_test_pearl`
checks whether binding the statement digest into the L1 proof changes that size
picture. It proves the same `TEST_PEARL` L1 outer object with five public
binding lanes and verifies the proof against the explicit public values:

| Statement-bound L1-only diagnostic (`TEST_PEARL`) | Measurement |
|---|---:|
| Full L1 outer object | `153,904 bytes` |
| L1 proof body | `152,259 bytes` |
| L1 metadata outside proof body | `1,645 bytes` |
| L1 public binding lanes | `5` |
| Prove time | `54.62s` |
| Verify time | `18ms` |

An earlier run of the same diagnostic measured `153,888 bytes` and `70.14s`, so
postcard size has small run-to-run variation and prover time has larger runtime
variation. The larger observed proof adds only `54 bytes` over the unbound
L1-only object, so explicit statement-digest binding is compatible with the
relaxed-size target. It does not solve the remaining soundness contract by
itself: an L1-only production wire format must still pin or reconstruct the
verifier key, L0 proof shape, preprocessed commitment, L1 circuit fingerprint,
table packing, and L1 public values without accepting proof-carried
substitutions. It also does not solve the proof-system soundness policy: the
current recursive prover profile still uses proof-system PoW in addition to
queries, while the production target is 60-bit soundness without relying on
verifier-accepted PoW grinding.

The no-PoW diagnostic
`relaxed_l1_only_pure_query_statement_bound_candidate_size_breakdown_for_test_pearl`
then reruns the statement-bound L1-only object with `commit_pow_bits=0` and
`query_pow_bits=0`. It sweeps the 60-bit pure-query shapes that are most
relevant to the current parameter discussion:

| Pure-query statement-bound L1-only diagnostic (`TEST_PEARL`) | L1 outer | Path-only projection | Preprocessed-omitted projection | Prove | Verify |
|---|---:|---:|---:|---:|---:|
| `lb=3,nq=20,pow=0` | `276,354 bytes` | `270,365 bytes` | `213,669 bytes` | `25.710s` | `39ms` |
| `lb=4,nq=15,pow=0` | `226,542 bytes` | `222,826 bytes` | `177,535 bytes` | `49.311s` | `26ms` |
| `lb=5,nq=12,pow=0` | `196,488 bytes` | `193,955 bytes` | `155,714 bytes` | `96.907s` | `27ms` |
| `lb=6,nq=10,pow=0` | `176,362 bytes` | `174,409 bytes` | `140,856 bytes` | `193.649s` | `22ms` |

All measured variants bind five public lanes, use zero commit/query proof-system
PoW, and reach 60 Johnson bits by `log_blowup * num_queries`. This closes the
parameter-only version of the relaxed L1-only batch-STARK route: removing PoW
from the soundness accounting pushes the raw proof well above `150 KiB`, and
higher blowup reduces bytes only by spending far more prover time.

The compact-preprocessed projection is an important structural compression
lever, but it still does not make the one-layer L1-only route production-ready.
The lower-blowup inflection point, `lb3,nq20`, is the only measured row below
the `<30s` proving-time target, but it still projects to `213,669` bytes. The
only L1-only row that projects below the relaxed `150 KiB` target is
`lb6,nq10`, and it takes `193.649s`. Therefore a one-layer batch-STARK final
proof cannot satisfy both the size and time targets even with
verifier-deterministic preprocessed openings omitted. The remaining route has
to either reduce the L1 verifier relation/prover work directly, or use a
different compact recursion/compression proof that avoids this L1 batch-STARK
proving cost.

The cap-height diagnostic
`relaxed_l1_only_pure_query_lb6_cap_height_candidate_size_breakdown_for_test_pearl`
then varies only the MMCS cap height for the smallest pure-query shape:

| Pure-query `lb=6,nq=10,pow=0` cap-height diagnostic (`TEST_PEARL`) | L1 outer | L1 proof body | Commitments | Opened values | Opening proof | Global lookup | Prove |
|---|---:|---:|---:|---:|---:|---:|---:|
| `cap=4` | `173,171 bytes` | `172,280 bytes` | `2,278 bytes` | `24,535 bytes` | `141,987 bytes` | `3,473 bytes` | `191.448s` |
| `cap=5` | `176,362 bytes` | `174,727 bytes` | not split in that run | not split in that run | not split in that run | not split in that run | `195.574s` |
| `cap=6` | `187,961 bytes` | `184,797 bytes` | `9,117 bytes` | `24,530 bytes` | `147,649 bytes` | `3,494 bytes` | `193.219s` |

Cap height is therefore not a hidden path to the relaxed target. Lowering the
cap from `5` to `4` saves only `3,191` bytes, while raising it to `6` increases
the cap material faster than it saves Merkle-path material. The cap-4 proof is
still `169.1 KiB`, and the opening proof alone is `141,987` bytes. Reducing the
batch-STARK envelope further requires fewer/lighter openings or a compact
recursive proof shape, not cap-height tuning.

The opening-proof diagnostic
`relaxed_l1_only_pure_query_lb6_cap4_opening_breakdown_for_test_pearl`
splits that best cap-4 point further:

| Pure-query `lb=6,nq=10,pow=0,cap=4` opening-proof breakdown (`TEST_PEARL`) | Bytes |
|---|---:|
| Total L1 outer object | `173,171` |
| L1 proof body | `172,280` |
| FRI opening proof | `141,987` |
| FRI query proofs | `136,577` |
| Input proof total | `97,424` |
| Input opened leaf values | `63,201` |
| Input Merkle paths | `34,213` |
| Commit-phase openings total | `39,152` |
| Commit-phase sibling values | `4,813` |
| Commit-phase Merkle paths | `34,259` |

The dominant bytes are not one removable metadata field. The current proof pays
for both opened leaf values and Merkle paths at every query. Removing all Merkle
paths would save about `68.5 KiB`, but the object would still carry the
`63.2 KiB` input opened-value payload plus commitments, opened OOD values,
global lookup data, and metadata. That is why the next viable reduction is a
sound compact opening format, fewer/lighter opened columns, or a second
recursive compression proof. A Merkle-only serialization change cannot satisfy
the `100 KiB` production target and is not a robust route to the relaxed
`150 KiB` target either.

The FRI-shape diagnostic
`relaxed_l1_only_pure_query_lb6_cap4_fri_shape_sweep_for_test_pearl` then keeps
`lb=6,nq=10,pow=0,cap=4,max_log_arity=3` and varies only
`log_final_poly_len`:

| Pure-query `lb=6,nq=10,pow=0,cap=4` FRI-shape diagnostic (`TEST_PEARL`) | L1 outer | L1 proof body | Opening proof | FRI query proofs | FRI final poly | Prove |
|---|---:|---:|---:|---:|---:|---:|
| `lfp=0,mla=3` | `175,304 bytes` | `174,413 bytes` | `144,120 bytes` | `137,988 bytes` | `21 bytes` | `195.531s` |
| `lfp=1,mla=3` | `173,481 bytes` | `172,590 bytes` | `142,297 bytes` | `136,908 bytes` | `40 bytes` | `196.417s` |
| `lfp=2,mla=3` | `173,171 bytes` | `172,280 bytes` | `141,987 bytes` | `136,577 bytes` | `77 bytes` | `191.448s` |

The existing `lfp=2,mla=3` shape is still the smallest measured cap-4
pure-query object, and the lower-tail variants remain around `170 KiB` with
roughly `196s` proving. This closes the soundness-neutral final-polynomial
retune as a production route.

A relaxed-size L1-only path would need to replace those proof-carried rebuild
inputs with a pinned verifier-key contract:

- verifier rebuilds the canonical program from trusted public block/attempt
  data via the params-pure canonical program path, not from proof bytes;
- verifier reconstructs the L0 proof shape from public profile/program/common
  data, or a small canonical proof-shape descriptor, so
  `build_composite_l1_verifier_circuit` no longer needs the whole L0 proof just
  to allocate targets;
- the final certificate binds the statement public-input digest, L0 profile,
  L0 preprocessed commitment, L1 circuit fingerprint, table-packing tuple,
  rows/degrees, non-primitive metadata, and L1 proof public values;
- the current L1 proof has `public_binding_lanes=0`; production must either
  enable public-value binding for the statement digest/public inputs or add an
  equivalent transcript-visible binding with the same rejection tests;
- negative tests reject swapped programs, stale preprocessed commitments,
  wrong statement public inputs, wrong L0 profile, wrong L1 circuit metadata,
  wrong proof public values, and tampered L1 proof body.

This is less invasive than a full Pearl-shaped two-recursion-layer compressor,
but it still requires new verifier-key plumbing. Simply dropping `l0_proof` and
`l0_program` from `AiPowRecursiveCertificate` would be unsound because the
current verifier would no longer have an independent way to know which L1
verifier circuit the submitted outer proof is supposed to prove.

The relaxed size gate also does not solve the time gate. The production-faithful
measurement spent `59.21s` on the L1 outer batch-STARK prove+verify after the
L0 proof already existed, and `93.88s` end to end; the focused
statement-bound `TEST_PEARL` diagnostic spent `54.62s` to `70.14s` proving the
mixed query/PoW L1 outer object across two release runs, and the pure-query
sweep spent `49.290s` to `195.574s` while missing the byte target. The cap
height follow-up spent `191.448s` to `193.219s` and still missed the byte
target. The `150 KiB` branch is therefore a candidate only if the L1 proof can
be both wire-minimized and made materially faster through structural changes.
The likely short-term levers are:

- avoid verifier-side reproving during metadata validation by replacing it with
  deterministic verifier-key reconstruction and direct metadata hashing;
- remove duplicate runner/prover trace materialization in the L1 checkpoint
  path;
- benchmark whether the canonical pinned+LogUp L0 baseline plus L1-only proof
  can meet `<30s` on the actual production trace with release flags after those
  engineering cuts;
- if it cannot, fall back to the Pearl-shaped two-layer compact route or a
  narrower specialized AIR rather than spending more effort on the batch-STARK
  envelope.

### Current Specialized Layer-0 Proof Baseline

The tree now has an ignored Layer-0 pinned+LogUp size diagnostic,
`composite_pinned_logup_*_l0_size_breakdown`, to quantify the proof object that
a Pearl-shaped compressor would consume if we start from the existing
specialized AI-PoW AIR instead of the generic terminal verifier relation. The
diagnostic proves and verifies `CompositeTrace::baseline_min()` with
`composite_prove_pinned_logup`, checks `pow_bits=0`, and prints component
sizes for the proof fields.

Release/native measurements on 2026-06-05:

| Layer-0 pinned+LogUp profile | Prove | Verify | Bincode proof | Bincode opening proof | Bincode opened values | Global lookup data |
|---|---:|---:|---:|---:|---:|---:|
| `lb=4,nq=15,pow=0` | `8.695s` | `0.118s` | `260,987` bytes / `254.9 KiB` | `229,849` bytes | `24,188` bytes | `6,808` bytes |
| `lb=6,nq=10,pow=0` | `32.314s` | `0.381s` | `199,882` bytes / `195.2 KiB` | `168,744` bytes | `24,188` bytes | `6,808` bytes |

Postcard sizes for the same two proofs were `273,043` bytes and `208,726`
bytes. The component split shows that the base proof is still dominated by FRI
opening material. Increasing blowup and reducing queries lowers the base proof
by about `61 KiB`, but it also makes this baseline proof about `3.7x` slower.

Consequences for the Pearl-shaped route:

- Directly serializing the existing Layer-0 proof is not enough; even the
  `lb=6,nq=10,pow=0` diagnostic is about `195 KiB` before any recursive
  certificate framing.
- A production-sized recursive path has to replace the Layer-0 FRI opening
  proof on the wire with a compact recursive proof that verifies it, not merely
  re-encode the Layer-0 proof.
- The existing specialized AIR is a plausible base statement for the
  Pearl-shaped route, but the final compressor must stay pure-query and avoid
  importing Pearl's proof-system PoW accounting.

## Direction 1: Unified Production NPO FRI/IOP

Build one production NPO proof that combines the current
`merged_value_bridge_proof` and `integrated_logup_proof` into a single
FRI-backed argument.

The unified proof would commit/open, under one transcript and one terminal FRI
proof, the matrices currently split across the two NPO subproofs:

- selected NPO row-domain table plus selected lookup IO;
- residual-zero composition;
- recompose quotient;
- NPO-row value bridge quotient;
- full Tip5 lookup trace plus masked trace-domain NPO-IO projection;
- Tip5 AIR quotient;
- byte-table LogUp accumulator and quotient;
- selected-domain and trace-domain NPO-IO LogUp accumulators and quotients.

The current verifier already enforces the most important cross-proof binding:
the selected+lookup commitment must match between the two subproofs. A unified
proof would make that equality structural, then share the FRI query set,
opening point, authentication paths, and transcript across all NPO identities.

Why this is still the relevant proof-shape direction, but not yet a production
solution:

- The production NPO proof is currently `204,039` bytes because two independent
  FRI payloads are serialized.
- The current NPO-only integrated checkpoint measures `96,017` bytes /
  `93.8 KiB` including the primitive proof on the small synthetic circuit, but
  it is not the full composite production proof body.
- The full-composite FRI-native residual-zero checkpoint now measures
  `55,344` bytes with a `54,023` byte compact FRI body over the actual
  composite NPO layout, has `0` nonzero residual values, and verifies. That
  makes this a real byte floor for the residual-zero layer, but not a candidate
  production proof until the remaining NPO quotient/value-bridge/lookup and
  primitive-row identities are included or replaced.
- The full-composite merged residual-zero/recompose/padding/value-bridge
  checkpoint now verifies at `151,448` bytes for `(prelude, primitive, merged
  NPO)`, with a `93,707` byte merged NPO proof and `91,501` byte compact FRI
  payload. It is still not production-qualified, but after prover-work reuse
  its post-prelude serial proof-body construction is `14.914s` (`8.372s`
  primitive, `6.541s` merged NPO). The internal Tip5 lookup/AIR/LogUp relation
  remains outside this proof, and the full diagnostic wall time still contains
  setup work that is not represented by `total_prove_ms`.
- The full composite integrated diagnostic did not reach its final size print,
  so this path currently misses the proving-time gate before it can be
  considered for promotion.
- Phase instrumentation shows that before the integrated Tip5 LogUp subproof
  finishes, the full composite candidate still spends `14.435s` proving the
  primitive component and then remains inside the integrated AIR quotient for
  more than 90 seconds after quotient-loop cleanup. Prepared selected+lookup
  reuse cuts the merged padding/value bridge to `2.340s`, so that component is
  no longer the first recursive-side bottleneck.
- The full production certificate can tolerate roughly `75-78 KiB` of NPO
  payload after the primitive R1CS component and certificate framing. A unified
  proof must therefore cut the NPO payload by about `125 KiB`.
- Prior component measurements show that opening payload sharing, not primitive
  R1CS compression, is the meaningful lever.

Soundness obligations:

- All profiles and commitments must be absorbed before challenges.
- Residual-zero, recompose, value-bridge, Tip5 AIR, byte LogUp, and NPO-IO
  LogUp challenges must have explicit domain separation and an ordering proof
  in the written theorem.
- For the full composite relation, verifier-derived residual columns must be
  identically zero under the committed witness-value columns and row relation;
  the current diagnostic rejection must become an honest-verifier acceptance
  before any size number can be counted as production evidence.
- The verifier must recompute every profile from the verifying key and reject
  any proof-carried profile mismatch.
- The unified proof must preserve the hidden-output masking rule for Merkle
  Tip5 rows; deriving all NPO IO directly from the full trace was already
  rejected as unsound for hidden-output rows.
- The proof must still reject stale value columns, stale selected+lookup
  roots, stale trace roots, forged trace-domain NPO IO, and recompose quotient
  tampering.

Implementation sketch:

1. Add a new `TerminalProductionUnifiedNpoProof` struct with one
   `TerminalCompressedFriProof`.
2. Move the quotient/accumulator matrix construction currently split between
   the two subproof provers into one builder that returns all domains and
   matrices.
3. Seed one challenger with the production prelude, all verifier-derived
   profiles, and the staged commitments.
4. Sample the same relation challenges currently used by both subproofs, but
   from one transcript.
5. Open all committed matrices with one `Pcs::open`.
6. Verify all identities from the same opened values.
7. Keep the old two-subproof verifier as a regression/fallback until the
   unified proof has equivalent tamper tests.

Tests required before promotion:

- Honest real Tip5 L0 verifier-circuit production measurement.
- Keep the full-composite residual-zero acceptance measurement passing, plus a
  regression test that a nonzero residual column is rejected.
- Body and certificate size assertions at or below the target.
- Tamper tests for each identity: residual-zero, recompose, value bridge, AIR
  quotient, byte LogUp, selected NPO-IO LogUp, trace NPO-IO LogUp.
- Cross-binding tests that swap selected+lookup, trace, accumulator, quotient,
  and final-cumulative data between independently generated proofs.
- Hidden-output Merkle Tip5 test proving that unmasked full-trace IO cannot
  satisfy the selected-vs-trace bridge.
- Noncanonical terminal parameter and proximity-profile rejection.

Assessment: this is the main cryptographically clean direction. It is also the
largest implementation, but it directly targets the measured size culprit.

## Direction 2: Re-Audit The Exhaustive NPO Terminal Proof As A Production Fallback

This direction was promoted for the recursion-crate terminal fixture, then
re-opened at the stack level. The current exhaustive supported-NPO terminal
fixture proof measures below the size target:

| Component | Historical measurement |
|---|---:|
| Primitive R1CS row-product proof | `22,631` bytes |
| Exhaustive NPO proof | `62,909` bytes |
| Compact production certificate | `85,948` bytes |
| Prove / verify | `1.492s` / `1.181s` in release measurement |

This route does not try to make the current polynomial NPO proof smaller.
Instead, the native terminal candidate uses exhaustive NPO row checking and
keeps the polynomial backend as a diagnostic/future hardening track.

Why it might work:

- The historical fixture measurement met both size and time targets.
- It checked every supported Tip5/recompose NPO row rather than sampling NPO
  rows.
- It avoided the current two-FRI-subproof duplication.

Why it was retired:

- The docs describe it as a checkpoint that still needed replacement by a
  final polynomial/proximity backend.
- Its soundness theorem is not currently the active production theorem.
- It serializes deterministic hidden-input and assignment-witness opening
  material and may reveal more witness data.
- It relies on exhaustive Merkle openings to an assignment oracle rather than a
  low-degree/proximity proof over NPO tables.

The key question is not whether it is smaller; it is whether the proof is
cryptographically sound for the terminal relation we need. A re-audit should
answer:

- Does primitive sparse-R1CS row-product plus exhaustive NPO row checking cover
  every operation in the recursive verifier circuit?
- Are all NPO callsites, row modes, hidden Tip5 lanes, Merkle direction bits,
  and recompose rows bound by the backend relation digest?
- Can a malicious prover choose an assignment that satisfies primitive R1CS and
  all exhaustive NPO row checks while representing a false Layer-0 verifier
  execution?
- Are the assignment Merkle commitments, derived known-index openings, hidden
  input payloads, and public prefix bound before all challenges?
- Does this route require zero knowledge? If not, is witness leakage acceptable
  for AI-PoW?

Implementation result:

1. `TerminalProductionProof` now carries optional `TerminalNpoExhaustiveProof`
   instead of the two-subproof polynomial NPO payload.
2. The production prelude binds exactly the assignment root; extra roots are
   rejected.
3. The recursion-crate Tip5 verifier-circuit release measurement passes at
   `85,948` bytes / `83.9 KiB`, `prove=1.492s`, `verify=1.181s`.
4. Focused production tests reject missing exhaustive assignment-opening
   material, tampered hidden Tip5 inputs, tampered assignment-witness Merkle
   frontier material, and recompose-row witness tampering.
5. The `ai-pow-zk` composite L1 terminal path is wired as an opt-in diagnostic.
   Its `lb=6,nq=10,pow=0` release/native run verifies after postcard decode,
   but measures `771,249` wire bytes, `prove=80.829s`, and `verify=58.825s`.
6. A production-profile non-proving relation metric shows the full path has
   `125,961` terminal operations, `14,049` supported NPO rows, and `242,798`
   NPO residual components before terminal proving begins.
7. The terminal relation digest rebuild is now deterministic for the baseline
   composite diagnostic; the fixed source was hash-ordered global lookup
   cumulative check emission in the recursive batch-STARK verifier circuit.
8. Fixed-width known-index proof limb/frontier encoding saves about `120.8 KiB`
   on the full-path reduced-profile wire object, but leaves `620,028` bytes in
   the NPO assignment-witness multiproof.

Assessment: this is still the preferred production direction, but not yet the
active stack-level production path. Its trade-off is witness exposure and, on
the full composite verifier relation, a much larger proof than the fixture:
about `753.2 KiB` on wire even after selecting the smallest measured
pure-query relation profile. Witness exposure is acceptable only if the final
terminal certificate is explicitly not specified as zero-knowledge and the full
composite verifier path is reduced under both production gates.

## Direction 3: Relation-Specific Projection Instead Of Full Trace Opening

The integrated LogUp proof remains large because it opens a wide Tip5 lookup
trace, even after several successful layout passes. Earlier work reduced the
lookup trace from a very wide lookup-free one-row-per-permutation shape to a
narrow row-per-round lookup shape, tuned LogUp grouping, and packed Merkle path
digests. The new packed lookup data source keeps the narrow lookup encoding
but restores one row per permutation, so the remaining work is the proof
theorem rather than another trace-source rewrite.

This direction tries to avoid opening all trace columns needed to evaluate the
Tip5 AIR relation directly at `zeta`. Instead, the prover would commit a
smaller relation-specific projection or composition polynomial that already
folds the required AIR, byte LogUp, terminal IO, and NPO-IO bridge identities.

Why it might work:

- Prior measurements show a single compact composition proof can be much
  smaller than opening all trace columns.
- The current integrated proof spends many bytes on opened trace values, not
  just Merkle authentication paths.
- The row-per-round layout has probably exhausted the easiest trace-width
  reductions.

Soundness risk:

- A prover-supplied composition polynomial is not sound by itself. The verifier
  must still know it was computed from committed trace/value columns.
- If the projection omits hidden-output masking or terminal IO support
  semantics, it can reintroduce the forged trace-domain NPO-IO bug.
- This path needs a written polynomial IOP theorem, not only a smaller
  serialized object.

Implementation sketch:

1. Define the minimal relation projection needed for Tip5 AIR and bridge
   checks.
2. Commit that projection under the same transcript as selected/value columns.
3. Prove, with low-degree quotients or sumcheck-style identities, that the
   projection is derived from the committed trace and selected NPO row data.
4. Open only the projection plus the few trace/value columns needed for the
   derivation proof.

Assessment: promising but theory-heavy. This may become the right long-term
backend if the unified two-subproof merge still lands above the target.

## Direction 4: Runtime Instrumentation And Prover Work Reuse

This direction was useful for diagnosing the old polynomial production path,
and it remains necessary for the full composite terminal path. The promoted
exhaustive path satisfies the `<30s` release target only for the
recursion-crate Tip5 verifier fixture. The actual `ai-pow-zk` composite
terminal path now has a completed reduced-profile release measurement:
`l1_verify=49ms`, `compile=7.606s`, `prove=80.829s`, `verify=58.825s`, and
postcard wire size `771,249` bytes.

The first runtime-instrumentation and reuse pass landed after this analysis.
The production prover emits per-stage timings when
`NOCK_TERMINAL_PROFILE_PROVER=1` is set. For the merged value-bridge diagnostic,
the prover now:

- reuses the primitive sparse-R1CS relation and assignment vector across the
  row-product, matrix-sumcheck, and assignment-evaluation substeps;
- prepares merged NPO columns, verifier-derived columns, trace, and prelude
  commitment once, then proves from that prepared data;
- uses prelude-checked prover entry points after the prelude has already been
  built from the same verifier key;
- computes the value-bridge quotient on the quotient coset with a batched LDE
  rather than interpolating each quotient-domain point separately.

Those changes preserved the value-bridge-only proof body and verifier language,
but changed its measured post-prelude proof-body construction from `97.796s` to
`16.159s`. The later padding-merged proof changes the NPO proof body and now
measures `151,448` bytes with `14.914s` post-prelude body construction.
Remaining repeated or expensive work:

- `verify_assignment_with_goldilocks_npos` checks the full assignment before
  production proof construction.
- Full diagnostic setup still pays Layer-0 prove, terminal compile, assignment
  commitment, prepared NPO-root construction, and prelude construction before
  the measured proof-body timer starts.
- The integrated LogUp subproof builds several accumulator and quotient
  matrices over extension fields and remains outside the relaxed-size
  checkpoint.

Immediate work:

1. Keep the real release measurement in the hot loop with `RUSTFLAGS="-C
   target-cpu=native"`. The current merged-plus-packed-support diagnostic is
   the active timing floor for the production route, not the older
   value-bridge-only timer.
2. Keep the non-proving relation metric in the hot loop. The current PROD
   relation has `75,870` Horner operations before proof construction, so
   optimizing terminal proof serialization alone cannot satisfy the `<30s`
   full-stack target.
3. Promote the prepared-data pattern into any production candidate before
   measuring production proving time.
4. Keep independent post-prelude subproofs parallel in the final prover, but do
   not count it as sufficient. The measured Rayon diagnostic lowers the current
   three-subproof stage to `39.448s` wall from a `53.355s` summed timer, still
   above the relaxed `~30s` target before setup work.
5. Avoid recomputing verifier-derived columns/layout/profile in the hot path
   when the verifier key is unchanged.

Assessment: low soundness risk and important for time because it changes only
prover work reuse, not verifier acceptance. For the full current
merged-plus-packed-support proof language it is not enough: the packed-support
branch dominates the parallel window, does not reduce the proof bytes, and does
not remove full diagnostic setup cost.

## Direction 5: Terminal FRI Parameter Tradeoff

The current terminal policy reaches 60 bits with pure FRI queries:
`log_blowup=4`, `num_queries=15`, and `query_pow_bits=0`. Reducing
`num_queries` would immediately reduce opening/authentication bytes, but the
lost query soundness has to be replaced.

Possible variants:

- Increase blowup and reduce queries. This tends to increase domains and can
  hurt proving time and memory.
- Add terminal query PoW and reduce queries. This can reduce proof bytes but
  changes the terminal soundness accounting and adds grinding work.
- Change FRI arity/final polynomial schedule. Prior sweeps already rejected
  several arity/final-poly variants for this backend, but a unified proof might
  have a different optimum.

Assessment: keep this as a policy-dependent fallback, not the first choice.
The current docs intentionally do not count terminal query PoW toward the 60-bit
floor. If that policy changes, it needs a fresh soundness calculation and a
clear statement that terminal query PoW is part of the production proof-system
security budget.

## Direction 6: Batch-STARK Checkpoint Hardening And Compact L2 Candidate

The batch-STARK L1 checkpoint is now soundness-hardened and should remain so,
but the full checkpoint envelope is not a route to the production proof-size
target. Its fixed-int certificate measurement is now multiple MiB for the full
checkpoint envelope, and even the L1 proof body alone is `149.1 KiB`, already
above the old hard target before considering verifier-key binding. It is still
useful for:

- regression testing the recursive verifier relation;
- comparing terminal verifier behavior against a conventional batch-STARK
  wrapper;
- fallback development while terminal proof-shape work continues.

The production verifier now compares the submitted outer proof's
preprocessed commitment binding against the canonical rebuilt L1 verifier
circuit binding before calling `verify_all_tables`; a regression tampers
`stark_common.preprocessed` and requires rejection. This closes the gap where a
self-consistent proof could otherwise carry non-canonical verifier-preprocessed
data with matching table metadata.

Assessment: keep the large checkpoint sound, but do not spend milestone effort
trying to make that envelope the production certificate. Compact batch-STARK
L2 remains a separate candidate: it may be production-viable if the final body,
explicit public values, and verifier-owned metadata/setup fit the relaxed
`150 KiB` / `30s` target without proof-system PoW.

## Recommendation

I would pursue five tracks in this order:

1. **Promote compact batch-STARK L2 as the main production candidate.** The key
   idea is specialized statement proof plus compact final recursion, not a
   specific backend label. The compact batch-STARK L2 adapter now has the right
   verifier-key shape: verifier-owned metadata/setup, reconstructed
   preprocessed openings, pruned paths derived from Fiat-Shamir queries, and
   final statement-digest public binding. The corrected `lb5,nq12` row is
   inside the relaxed size gate; the next checkpoint is proof-time reduction and
   production certificate wiring.
2. **Keep exhaustive NPO as the native-terminal fallback, but do not call it
   fully production-integrated yet.** It is the only current native terminal
   fixture measured below 100 KiB and below 30s, but the actual composite L1
   verifier path still exceeds both the size and time gates.
3. **Reduce the full composite L1 terminal relation only if we resume pursuing
   the generic-verifier terminal route.** The current blocker is relation size:
   `106,349` primitive operations and `14,049` supported NPO rows in the PROD
   baseline, not a large terminal public input vector. The primitive reduction
   should focus first on generic FRI/PCS verifier Horner work; the NPO
   reduction should focus on reducing Tip5/recompose callsite count or changing
   the terminal proof shape, not on removing the recompose/coeff binding table
   by itself.
4. **Continue the unified NPO proof as hardening/future work.** It would reduce
   witness leakage if it can share one FRI payload and stay under target, but
   the current full-composite integrated candidate is too slow.
5. **Keep the large batch-STARK checkpoint hardened, but separate it from the
   compact batch-STARK candidate.** The full checkpoint envelope is still too
   large; the compact L2 body can be considered for production only with
   verifier-owned metadata/setup, explicit public-value binding, and exact
   production certificate byte accounting.

I would not spend milestone effort on terminal query-PoW parameter changes. The
compact batch-STARK path already has rows at 60 pure FRI query bits that are
closer to the relaxed target than the native terminal route.

## Minimum Promotion Checklist

Any candidate production direction must satisfy all of the following before it
becomes the production recursive proof path:

- Full `ai-pow-zk` recursive certificate measurement at or below about
  `150 KiB`, including all public inputs and verifier-key/setup bindings
  required for verification.
- Release-profile proving time under `30s` on the agreed production machine
  class.
- No proof-system query PoW counted unless the production soundness policy is
  explicitly changed and documented.
- Compact batch-STARK candidates must verify against verifier-owned metadata,
  canonical setup/preprocessed data, explicit public values, and pinned FRI
  parameters; prover-supplied metadata must never be trusted as the verifier
  key.
- Full verifier rejection tests for malformed bodies, noncanonical parameters,
  stale preludes, swapped roots, missing roots, tampered FRI openings,
  residual-zero tampering, recompose tampering, value-bridge tampering, byte
  LogUp tampering, NPO-IO LogUp tampering, hidden-output Merkle Tip5 cases, and
  Tip5 MMCS direction-bit-`1` rows with both CTL inputs and CTL outputs, and
  wrong public values.
- Written soundness theorem that names every binding: public values, terminal
  header, backend relation digest, NPO layout/profile, fixed Tip5 table digest,
  production proximity profile, assignment root, selected/value roots, trace
  roots, accumulator roots, quotient roots, final cumulatives, and FRI query
  derivation.
- No Hoon/kernel verifier acceptance until Rust verifier wiring is explicitly
  in scope and fail-closed behavior is intentionally changed.
