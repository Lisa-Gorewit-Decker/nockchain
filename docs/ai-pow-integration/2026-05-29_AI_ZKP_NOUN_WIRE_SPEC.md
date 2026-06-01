# AI-PoW ZKP Noun Wire Specification

Date: 2026-05-29
Status: Draft specification, sizing note, and implementation checklist.

## 1. Goal

The AI-PoW artifact persisted in the Hoon kernel and transmitted in
blocks must be the recursive ZKP certificate of the AI puzzle plus only
the small fields needed to verify that certificate against the candidate
block. It must not carry the plain `MatmulProof`, a raw Layer-0 ZKP,
spot-check openings, full matrix rows, or any local prover-only witness
data.

The proof must also not be serialized as one opaque bincode byte atom. The Hoon/kernel object needs a stable noun type that mirrors the proof structure enough for consensus code and verifier jets to check shape, enforce size limits, reconstruct transcript inputs, and pass a canonical Rust mirror into the Plonky3 verifier.

The intended on-chain object is:

- a structured recursive Plonky3/AI-PoW certificate noun;
- the NCMN nonce needed to bind the attempt to the candidate block;
- fixed-size public inputs and matrix commitments required to reconstruct the trusted verifier statement;
- small version/config/index atoms;
- no plain `MatmulProof`;
- no raw Layer-0 `AiPowBatchProof` / plain ZKP artifact.

Large homogeneous proof vectors are represented as packed atoms inside typed fields, not as a single proof-wide blob and not as Hoon lists of individual bytes. This is the key compromise: the noun exposes the cryptographic proof structure, while vector payloads stay compact enough for block storage and Hoon cue/jam.

## 2. Current State

Current Hoon integration:

- `hoon/apps/dumbnet/lib/types.hoon` admits only `[%ai-pow nonce=ai-ncmn cert=ai-pow-certificate]` for the AI-PoW block-submission variant.
- `hoon/common/tx-engine-1.hoon` keeps `page.pow` as a generic structured `pow-artifact` noun (`*`) so legacy `%dumb-zkpow` pages remain decodable and the Hoon compiler does not recursively expand the AI proof-tree mold in every page consumer. The `%ai-pow` command boundary remains typed as `[%ai-pow nonce=ai-ncmn cert=ai-pow-certificate]`, but current consensus rejects it fail-closed until recursive certificate verification is wired. Rust performs bounded certificate shape validation before verifier work.
- The Rust miner's canonical submission payload is `[%command %pow %ai-pow nonce cert]`. The production binary configures a recursive-certificate noun builder. Library callers that omit that builder must refuse to submit rather than falling back to nonce/tile or plain-proof placeholders.
- Recursive proving is started only after the plain matmul proof is checked against the chain-derived target used by the winning mining attempt.

Current non-admissible Rust prover internals:

- `ZkProofArtifact { proof: AiPowBatchProof, pis: CompositePublicInputs, trace_height }`

The raw Layer-0 proof object is an intermediate prover object. It is not
a Hoon type and is not an admissible block/wire artifact. The commitment
and public-input fields are admissible only as typed statement data inside
`ai-pow-certificate`; they are not sufficient without the recursive
certificate. In particular, Hoon should not grow a `MatmulProof` type or
an `AiPowBatchProof`/Layer-0 ZKP proof arm.

The production proof artifact is the recursive certificate exposed by
`ai_pow_zk::recursion::AiPowRecursiveCertificate`, produced through
`prove_canonical_ai_pow_certificate`. The Hoon/wire type targets that
recursive certificate only.

## 3. Encoding Principles

1. The outer block/page wire format remains normal noun serialization: `jam` of the block/page containing `%ai-pow`.
2. The proof is a structured recursive-certificate noun. The current implementation uses a tagged `ai-proof-node` tree for the recursive certificate internals while preserving typed top-level statement fields.
3. Homogeneous vectors are packed into atoms using fixed-width little-endian limbs. This avoids one Hoon cell per field element.
4. Field/vector lengths are either fixed by the admitted `params` or carried in a small `len` atom and checked against config-derived bounds before verifier execution.
5. Strings and prover-controlled metadata are not consensus proof data. The verifier reconstructs AIR list, recursive FRI parameters, lookup metadata, and program commitment expectations from canonical config.
6. Rust noun decoding must rebuild the exact recursive certificate from this canonical structure, with strict range and shape checks.

## 4. Primitive Noun Types And Auras

```hoon
::  Degree-2 binomial extension element over Goldilocks.
::  Limb c0 occupies bytes 0..8; limb c1 occupies bytes 8..16.
::  Both limbs must be canonical Goldilocks field elements.
+$  ai-ext2  @uxfelt
::
::  256-bit BLAKE3-style digest as a single atom.
::  Byte 0 of the 32-byte Rust digest is the least-significant byte.
::  The @uxblake aura is semantic/documentary; decoders still enforce
::  met 3 <= 32 and zero-pad missing high bytes when reconstructing
::  [u8; 32].
+$  ai-blake  @uxblake
::
::  80-byte NCMN nonce as a single atom.
::  Byte 0 of the Rust nonce is the least-significant byte.
+$  ai-ncmn  @uxncmn
::
::  Packed vector of extension field elements.
::  data stores 2*len little-endian u64 limbs:
::    element i = [limb 2*i, limb 2*i+1].
+$  ai-ext2-vec  [len=@ud data=@uxfelts]
```

`ai-blake` replaces the earlier `[u32; 8]` tuple shape for all
BLAKE3-style commitments and public-input digests. This keeps each
32-byte digest as one noun atom with a domain-specific aura instead of
an eight-cell tuple.

`ai-ext2` similarly replaces the earlier `[c0 c1]` tuple shape for
degree-2 Goldilocks extension values. This matters most for
`ai-lookup-sums`, whose scalar values would otherwise cost one cell per
extension element before even counting list structure.

The recursive certificate internals use the generic packed-node tags
`%ext2`, `%ext2s`, `%bytes`, `%u64s`, and `%i64s` instead of a
proof-wide atom. Hoon can inspect lengths cheaply, and jets can unpack
large homogeneous vectors without traversing a cell per scalar.

## 5. AI Statement Types

```hoon
::  Matrix commitments needed to derive/check the ZK statement.
+$  ai-pow-commitments
  $:  h-a-chunk=ai-blake
      h-b-chunk=ai-blake
  ==
::
::  Rust mirror of ai_pow_zk::CompositePublicInputs.
+$  ai-pow-public-inputs
  $:  cumsum=[@s @s @s @s]
      jackpot=[@ud @ud @ud @ud @ud @ud @ud @ud @ud @ud @ud @ud @ud @ud @ud @ud]
      hash-a=ai-blake
      hash-b=ai-blake
      job-key=ai-blake
      commitment-hash=ai-blake
      hash-jackpot=ai-blake
  ==
```

The verifier must treat `public` as claimed public input data, not trusted data. It is accepted only if it matches values reconstructed from block commitment, nonce, matrix commitments, `found-idx`, and the canonical AI-PoW program.

## 6. Recursive Certificate Types

This is the only AI-PoW proof structure admitted by Hoon. It carries
the recursive certificate as a tagged noun tree. Hoon does
not define or accept a type for the plain Layer-0 ZKP and does not
define or accept a type for the plain `MatmulProof`.

```hoon
+$  ai-proof-node
  $%  [%n ~]
      [%b value=?]
      [%u value=@ud]
      [%i data=@]
      [%ext2 value=ai-ext2]
      [%ext2s len=@ud data=ai-ext2s]
      [%bytes len=@ud data=@]
      [%u64s len=@ud data=@]
      [%i64s len=@ud data=@]
      [%seq items=*]
      [%map entries=*]
      [%none ~]
      [%some value=*]
  ==
+$  ai-recursive-certificate  ai-proof-node
```

Consensus should initially admit exactly the current recursive production
AI-PoW shape:

- `ai-proof-node` recursion depth, list lengths, packed atom lengths, and nested `%seq`/`%map`/`%some` payload shapes are bounded before any verifier work;
- `[%ext2 value]` and `[%ext2s len data]` store degree-2 Goldilocks extension values as custom-aura atoms, not as anonymous two-element tuples;
- `[%u64s len data]`, `[%i64s len data]`, and `[%bytes len data]` atoms never exceed their declared packed byte budget; omitted high zero bytes decode as zero;
- recursive commitment, opening, and FRI parameters are decoded from the tagged node tree and matched against config-derived verifier expectations;
- no Layer-0 `AiPowBatchProof` object, lookup metadata strings, raw matrix openings, or `MatmulProof` fields are accepted.

The Rust miner crate now has an encoder and bounded decoder for this
generic node tree. Remaining verifier-specific reconstruction checks
must be explicit decode checks, not left to panic or deep verifier
errors.

The Hoon mold intentionally avoids recursively expanding `ai-proof-node`
inside `%seq`, `%map`, and `%some`. Those payloads remain structured nouns
instead of a single opaque proof atom, but recursive proof-tree validation lives
in Rust because a fully recursive Hoon mold caused `hoonc` to loop in
`ut_mint`/unifying-equality while compiling `tx-engine-1`.

For the same reason, the persisted page field uses:

```hoon
+$  pow-artifact
  *
```

This is a compiler-safety compromise, not a wire-format relaxation. Once the
recursive verifier is wired, the AI artifact noun persisted by dumbnet will be
`[%ai-pow nonce cert]`, where `nonce` is the NCMN nonce and `cert` is the
recursive certificate structure above. Until then, consensus rejects `%ai-pow`
fail-closed. The block-submission `pow-variant` mold remains typed, and the Rust
decoder/verifier must reject any non-canonical or oversized
recursive-certificate noun.

## 7. Top-Level Certificate Arm

```hoon
::  Hoon/block AI-PoW certificate artifact.
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

Recommended `%ai-pow` block payload:

```hoon
+$  pow-variant
  $%  [%dumb-zkpow prf=proof:sp dig=tip5-hash-atom:zeke bc=noun-digest:tip5:zeke nonce=noun-digest:tip5:zeke]
      [%ai-pow nonce=ai-ncmn cert=ai-pow-certificate]
  ==
```

The NCMN nonce stays outside `ai-pow-certificate` but inside the `%ai-pow`
artifact because it is a necessary commitment parameter, not proof witness data.
The verifier parses it, checks that its `nck_commitment` equals the trusted
candidate block commitment, rejects reserved external commitments, then derives
the trusted statement from `(puzzle_id, nonce, params, target, commitments,
found_idx, trace_height)`. The certificate itself is the recursive production
proof only.

## 8. Serialization Rules

The outer wire format is normal block/page noun serialization: `jam` of
the block/page containing `%ai-pow`. There is no secondary bincode
envelope inside `ai-pow-certificate`.

Within the noun:

- `ai-ext2` is a single `@uxfelt` atom with `met 3 <= 16`; bytes 0..8 decode to `c0`, bytes 8..16 decode to `c1`, and both limbs must be canonical Goldilocks values.
- `[%ext2 value]` carries one `ai-ext2`; `[%ext2s len data]` carries `len` consecutive `ai-ext2` values in an `ai-ext2s` atom with `met 3 data <= 16 * len`.
- `[%bytes len data]` must have `met 3 data <= len`, with high zero padding only.
- `[%u64s len data]` and `[%i64s len data]` must have `met 3 data <= 8 * len`, with high zero padding only.
- `ai-blake` is a single `@uxblake` atom with `met 3 <= 32`; byte 0 of the Rust `[u8; 32]` is the least-significant byte of the atom.
- packed vector atoms use little-endian u64 limbs.
- `met 3 data` for a packed vector must be no larger than `8 * limbs`.
- missing high bytes in a packed vector atom decode as zero; decoders must reject atoms whose byte length exceeds the exact limb budget.
- `(unit x)` uses normal Hoon `~` or `[~ x]`.
- variable lists must have config-bounded lengths before any Rust verifier allocation.

Rust conversion rules:

1. Decode the full artifact noun `[%ai-pow nonce cert]` into the Hoon/Rust
   mirror type with only bounded allocation.
2. Validate every scalar range and vector length.
3. Reconstruct omitted recursive verifier metadata from canonical config.
4. Rebuild `ai_pow_zk::recursion::AiPowRecursiveCertificate`.
5. Invoke `ai_pow_zk::recursion::verify_recursive_certificate` with the
   verifier-derived `CompositePublicInputs`. This verifies both the recursive
   recursive STARK envelope and the cryptographic binding between the outer
   proof's public values and the Layer-0 AI-PoW statement.

The node-side Rust helpers package the required ordering for decoded noun
artifacts:

1. `certificate_noun::decode_ai_pow_artifact_slab` is the canonical bounded
   parser for the persisted/wire artifact `[%ai-pow nonce cert]`. It decodes
   the structured recursive certificate, parses the `@uxncmn` nonce, rejects
   malformed/reserved/external-commitment nonces, and keeps the nonce adjacent
   to the certificate shape.
2. `certificate_noun::decode_ai_pow_artifact_jam` is the byte-oriented
   production boundary for persisted/network artifacts. It enforces
   `CertificateNounLimits::max_jam_bytes` before cueing attacker-controlled
   jam bytes, then runs a no-allocation jam preflight enforcing total noun
   count, noun depth, and atom byte limits before any `NounSlab` allocation.
   It rejects empty input before cue, contains cue panics, then calls the same
   bounded artifact parser. It also requires `jam(cue(bytes)) == bytes`, so
   non-canonical jam encodings such as trailing bytes are rejected instead of
   being silently canonicalized.
3. `certificate_noun::verify_ai_pow_ncmn_artifact_jam` is the intended
   consensus-facing helper when the caller has jam bytes: byte-size cap, cue,
   bounded decode, NCMN anchor check, full-matmul statement precheck, and
   recursive certificate verification happen in that order. Multi-tile
   selected-tile recursive statements fail closed until the certificate binds a
   full-matrix aggregate.
4. `certificate_noun::verify_decoded_ai_pow_ncmn_artifact` is the intended
   verifier-jet Rust boundary after noun decoding. It reconstructs the
   `AiPowRecursiveCertificate` from the structured proof-node tail, checks the
   NCMN nonce anchor, runs the cheap full-matmul statement precheck first, then
   calls the recursive verifier with the decoded public inputs.
5. `certificate_noun::verify_decoded_ai_pow_ncmn_certificate` remains available
   for callers that already separated the nonce from the decoded certificate but
   still need the NCMN consensus wrapper.

The lower-level explicit-attempt helpers are crate-internal implementation
details. They do not parse or enforce the NCMN candidate-block anchor and must
not be used as Nockchain consensus/block-wire entrypoints.

The deprecated `verify_recursive_certificate_outer` helper is outer-only
diagnostic code for old unbound proof objects. Consensus must not use it:
accepting an outer proof while trusting adjacent statement metadata would
permit certificate/metadata swaps.

The verifier jet receives:

```text
params, found-idx, trace-height, commitments, public, recursive-certificate, puzzle id, candidate block commitment, NCMN nonce, target
```

It must reconstruct the canonical program and expected public inputs from trusted data, then verify the ZK proof. Hoon may parse and pre-check shape/length, but soundness-critical Plonky3 verification stays in Rust.

## 9. Size Measurement

### Current Layer-0 Proof, Non-Canonical For Blocks

Measured command:

```text
cargo run -p ai-pow --features zk --example f1_harness
```

Measured output on 2026-05-29:

```text
shape=TEST_SMALL
proof_bytes=382514
num_pis=60
prove_ms=32650
verify_ms=497
```

Interpretation:

| Component | Current measured size |
|---|---:|
| raw Plonky3 `AiPowBatchProof` bincode bytes | 382,514 bytes |
| fixed public inputs, commitments, config, and cells | less than 1 KiB |
| current opaque-bincode artifact, excluding outer block/page | about 383 KiB |

The current uncompressed Plonky3 `BatchProof` is therefore not the
desired ~100 KiB wire artifact and is not the canonical recursive
certificate.

The legacy Rust byte envelopes for Layer-0 proofs and recursive-certificate
diagnostics are crate-internal only. They exist to exercise bridge tests and
codec guards, not as a Hoon/block serialization format. The public production
surface is the recursive certificate prover plus statement verifier boundary;
the persisted and transmitted artifact remains the structured
`[%ai-pow nonce cert]` noun.

The structured noun encoder/decoder is implemented for the generic
recursive-certificate tree, and malformed packed atoms/list shapes have
focused tests. The structured proof-node format is now invertible: a decoded
real recursive certificate noun can be reconstructed into
`AiPowRecursiveCertificate` and verified with
`verify_recursive_certificate`. Reconstruction performs a canonical
re-serialization check, so proof-node structures with ignored extra fields or
lossy sequence encodings are rejected.

The current real recursive certificate noun measurement is:

```text
GNORT_DISABLE=1 cargo test -p ai-pow-miner --features node \
  real_recursive_certificate_noun_roundtrips_and_prints_size \
  --release -- --ignored --nocapture
```

Measured on 2026-06-01 with the current small recursive fixture:

| Artifact | Bytes | KiB |
|---|---:|---:|
| jammed structured `ai-pow-certificate` noun | 193,093 | 188.6 KiB |
| postcard L1 recursive certificate | 125,162 | 122.2 KiB |

That fixture proves the structural path, but it is not yet the final
production-size benchmark. Exact byte counts can vary slightly across proof
runs because some encoded vectors use variable-width atom/jam representation.
The production `prod_recursion_measure` harness below remains the better
estimate for the compact L1 certificate payload. The expected structured-noun
size profile is:

| Encoding choice | Expected size impact |
|---|---:|
| one proof-wide bincode atom | about 383 KiB today, but rejected by this spec |
| fully exploded Hoon lists of every field element and digest limb | likely materially larger than bincode and not acceptable |
| structured noun with packed homogeneous vectors as specified here | currently about 1.54x postcard on the small real recursive fixture |
| future ~100 KiB recursive/compressed certificate using the same structured-vector approach | about 100-110 KiB, depending on certificate shape |

The packed-vector structure is necessary. A pure list encoding would add one cell per field element, per extension limb, and per Merkle digest limb. That is precisely the cost we need to avoid while still giving Hoon a proof-shaped noun.

The implementation includes an ignored measurement test
`real_recursive_certificate_noun_roundtrips_and_prints_size` that prints,
for a real recursive certificate:

- total jammed `ai-pow-certificate` size;
- compact postcard L1 certificate size;
- recursive prove/build/verify timing;
- proof-node reconstruction and recursive verification success after jam/cue.

### Recursive Production Proof Benchmark

The production wire proof is the recursive L1 certificate, not the
Layer-0 composite proof. The current repository has a dedicated
measurement harness:

```text
RUSTFLAGS="-Ctarget-cpu=native" \
  cargo run -p ai-pow-zk --release --features recursion \
  --example prod_recursion_measure -- 15
```

This benchmark uses the production AI-PoW shape documented in the
harness:

- model params: `m=4096 k=4096 n=14336 noise_rank=64 tile=8`;
- profile: `CircuitConfig::PROD` with `log_blowup=4`, `num_queries=15`, `pow_bits=1`;
- trace: `2^15 = 32768` rows by `1911` columns;
- trace contents: zero-activity baseline at production dimensions. The
  STARK/FRI proof cost is data-oblivious for this benchmark, so size
  and prover time are dimension/profile measurements without needing
  16 GB of model weights.

Measured on 2026-05-29:

| Stage | Time |
|---|---:|
| L0 composite prove | 27.14 s |
| L1 verifier-circuit build | 0.44 s |
| L1 in-circuit verify | 0.06 s |
| L1 outer certificate prove + verify | 11.98 s |
| End-to-end trace-to-recursive-proof time | 39.62 s |
| Recursive-only time after L0 proof exists | 12.48 s |

Serialized sizes from the same run:

| Artifact | Bytes | KiB |
|---|---:|---:|
| L0 composite proof | 258,032 | 252.0 KiB |
| L1 recursive certificate | 103,023 | 100.6 KiB |

The non-native build of the same `2^15` run was effectively the same
size and slightly slower:

```text
CSV,15,32768,1911,27808,461,59,11963,257774,102958
```

The native build result was:

```text
CSV,15,32768,1911,27141,436,59,11980,258032,103023
```

CSV columns are:

```text
log2_rows, rows, width, l0_prove_ms, l1_build_ms,
l1_verify_ms, l1_cert_ms, l0_bytes, l1_bytes
```

Important caveat: these byte counts are `postcard` serialization of the
current Rust proof objects. The final consensus artifact is the structured noun
encoder described here. The current structured-noun harness proves the noun
roundtrip on a small real recursive certificate; the final production-size gate
must run the same structured noun measurement on the final production L1
certificate shape.

## 10. Verification Checks Required By This Shape

Before accepting `%ai-pow`, consensus must require:

1. `version == 1`.
2. `params` match the exact admitted AI params, AIR shape, FRI parameters, commitment cap height, lookup declarations, and STARK config.
3. `found-idx < params.num_tiles`.
4. total jammed proof size is `<= MAX_AI_ZKP_NOUN_BYTES`.
5. every packed vector length is `<=` its config-derived maximum before allocation.
6. every packed Goldilocks limb is canonical.
7. every `ai-ext2` atom has `met 3 <= 16` and decodes to two canonical Goldilocks limbs by zero-padding high bytes.
8. every `ai-blake` atom has `met 3 <= 32` and decodes to exactly 32 bytes by zero-padding high bytes.
9. `public-inputs.cumsum` values are canonical signed i32 values.
10. `random == ~` for the current non-ZK Plonky3 PCS configuration.
11. commitment cap lengths match the configured cap height.
12. FRI query count, commit phase count, final polynomial length, and `log-arity` values match the configured FRI parameters.
13. global lookup sums have the verifier-reconstructed nested shape; no serialized lookup names or aux columns are accepted.
14. Hoon/Rust reconstructs `kappa`, `s_b`, `s_a`, and `pow_key_for_nonce(s_a, nonce)`.
15. reconstructed `job-key` equals `public-inputs.job-key`.
16. reconstructed nonce-derived key equals `public-inputs.commitment-hash`.
17. `public-inputs.hash-a == commitments.h-a-chunk`.
18. `public-inputs.hash-b == commitments.h-b-chunk`.
19. `public-inputs.hash-jackpot <= target`.
20. Rust verifies the structured recursive certificate with `verify_recursive_certificate`, including the outer recursive STARK envelope.
21. Rust verifies that the certificate's bound public values are exactly the canonical statement rebuilt from config, commitments, `found-idx`, block commitment, nonce, and target.

## 11. Implementation Plan

1. Add the Hoon command-boundary types above while keeping the page storage mold generic (`pow-artifact` is `*`) to avoid `hoonc` recursive-mold loops. Until the verifier lands, `%ai-pow` remains fail-closed and must not persist `[%ai-pow nonce cert]` in `page.pow`.
2. Keep the miner's node-facing API canonical: the only AI-PoW block-submission payload is `[%command %pow %ai-pow nonce cert]`. If no recursive-certificate noun builder is configured, the miner must refuse to submit a legacy nonce/tile or plain `MatmulProof` artifact.
3. Before recursive proving, require the plain matmul proof to verify against the same chain-derived target that the winning mining attempt used.
4. Add a Rust `AiPowCertificateNoun` mirror type that converts the recursive certificate into the Hoon `ai-proof-node` tree without `MatmulProof`, raw Layer-0 `AiPowBatchProof`, or bincode. Status: implemented as `certificate_noun::AiProofNode` plus top-level certificate noun construction; the legacy Layer-0 and byte-envelope APIs are crate-internal so they cannot be imported as normal production APIs.
5. Implement packed-vector helpers for Goldilocks, extension-field pairs, and Tip5 digest vectors.
6. Reconstruct `LookupData` metadata from canonical AIR/config rather than serializing strings.
7. Add jam/cue round-trip tests for a real proof and malformed nouns. Status: implemented for structured sample certificates, malformed packed/list/tag/version cases, non-canonical `ai-ext2` limbs, and an ignored real recursive certificate noun round-trip/size harness.
8. Add size tests asserting total jammed noun budget and per-vector caps. Status: the ignored real recursive certificate harness asserts a coarse 2 MiB upper bound and prints measured size; production budget constants still need to be set after the harness is run on the final L1 shape.
9. Add adversarial decode tests for oversized lengths, non-canonical field elements, mismatched lookup shapes, invalid FRI arities, and extra/trailing packed bytes.
10. Expose a full recursive certificate verifier that takes verifier-derived public inputs and rejects an otherwise valid certificate when any puzzle id, candidate block commitment, NCMN nonce, target, commitment, params, or `found-idx` field is changed. Status: implemented at the Rust boundary: `decode_ai_pow_artifact_jam` caps jammed bytes before cue, `decode_ai_pow_artifact_slab` parses the full `[%ai-pow nonce cert]` wire artifact, and `verify_ai_pow_ncmn_artifact_jam` / `verify_decoded_ai_pow_ncmn_artifact` reconstruct the structured noun proof, check the NCMN nonce anchor, precheck the full-matmul statement, and call `verify_recursive_certificate`, whose outer proof binds the Layer-0 public-input vector as STARK public values. The precheck rejects multi-tile selected-tile statements with `FullMatmulProofUnavailable`. Single-tile smoke statements derive canonical seeds from `h_a_chunk` / `h_b_chunk`, the same commitments bound by the ZK proof as `HASH_A` / `HASH_B`; Hoon consensus remains fail-closed until the verifier jet calls this boundary and the recursive statement proves the intended full-matmul work unit.
11. Add the verifier jet entrypoint consuming this noun shape.
12. Replace the deferred-verifier accept path with real accept/reject checks only after end-to-end accept/reject tests exist.
