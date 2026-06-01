# AI-PoW proof: Noun data type & serialization design

**Date:** 2026-05-28
**Branch:** `claude/ai-pow-integration-squash`
**Status:** SUPERSEDED DESIGN ONLY — no code landed. This is the precursor design for
Stage 6 §8 deferred decision #1 ("`%3` proof body shape"). The AI verifier
jet itself is soundness-critical invasive work and is **not** landed here
(see Residual). Per R1: this is the design stage; the linchpin (the Rust
verifier jet + consensus binding checks) lands in disciplined validated
stages after this is reviewed.

> Superseded 2026-05-29 by
> `docs/ai-pow-integration/2026-05-29_AI_ZKP_NOUN_WIRE_SPEC.md`.
> In particular, the production proof artifact is the recursive L1
> certificate, not the raw Layer-0 `BatchProof`; and BLAKE3-style
> 256-bit digests are encoded as single custom-aura atoms
> (`@uxblake`), not as `[u32; 8]` tuples. The tuple design below is
> retained only as historical context.

---

## 1. Context — why this design is needed

Stage 6 wired the *dispatch* for a second PoW puzzle type (AI matmul PoW)
into the kernel: the `proof-version` enum gained a `%3` arm, the
`pow-variant` union gained an `[%ai-pow ...]` arm, per-puzzle ASERT
retargeting walks same-type ancestors, and the activation gate accepts
either `%2` (ZK) or `%3` (AI) past `ai-pow-activation-height`. All of that
landed with **placeholders** for the actual AI proof payload:

- `hoon/common/ztd/four.hoon` — the `%3` arm of `+$ proof` is currently a
  *copy* of the `%0/%1/%2` shape (`objects` / `hashes` / `read-index`),
  carrying nothing AI-specific. ZK helpers crash on `%3` via `?=` guards.
- `hoon/apps/dumbnet/lib/types.hoon` — `+$ pow-variant` carries
  `[%ai-pow placeholder=@]`, a single stub atom.

This document defines the **real** noun data type for the AI proof and its
**serialization** across the Rust ⇄ Hoon boundary, so the verifier jet and
consensus binding checks have a concrete, soundness-legible shape to build
against.

The AI PoW is architecturally different from the existing ZK PoW in one
load-bearing way: **the existing `%dumb-zkpow` proof is verified natively in
Hoon** (`hoon/common/stark/verifier.hoon ++verify` reconstructs
`puzzle-nock` and checks the Hoon STARK). **The AI proof is a Plonky3
batch-STARK over Goldilocks with a Tip5 challenger — it cannot be verified
in Hoon.** Verification is a Rust jet (`composite_verify_pow_pinned_logup`,
`crates/ai-pow-zk`). Therefore the noun's job is to (a) carry the proof
transcript **opaquely** for the jet, and (b) expose the small,
consensus-relevant **public inputs** in a Hoon-readable, structured form so
the kernel can do the binding + difficulty checks it must not delegate.

---

## 2. What goes on-chain (and what does not)

The Rust side has two proof objects:

| Object | Crate | Role | On chain? |
|---|---|---|---|
| `MatmulProof` (BLAKE3 Merkle openings, `found`+`spot` tiles) | `ai-pow/src/proof.rs` | Pearl §4.6 non-ZK opening proof; prover intermediate | **No** |
| `BatchProof<AiPowStarkConfig>` + `CompositePublicInputs` | `ai-pow-zk` | The ZK STARK; matrix binding (`HASH_A/HASH_B`) is in-circuit | **Yes** |

In the production ZK path the STARK **subsumes** the plain spot-check
openings: the matrix commitment is bound in-circuit (M52 `HASH_A`/`HASH_B`
↔ `h_a_chunk`/`h_b_chunk`), and the jackpot/target relation is a circuit
constraint. So the on-chain AI proof is exactly **`BatchProof` bytes +
`CompositePublicInputs`** (eventually the recursed L1/L2 *certificate*
bytes, targeting ≤100 KB — see `proof-size-target-100kb`). `MatmulProof`
stays prover-internal and is **not** serialized into the block.

---

## 3. Hoon data type — `+$ ai-proof`

Place in `hoon/common/ztd/four.hoon` (the proof-stack types module, `sp`),
next to `+$ proof`.

```hoon
::  Superseded: the current spec uses a single @uxblake atom, not
::  this 8-tuple shape.
+$  digest8  @uxblake
::
::  Mirror of `CompositePublicInputs` (crates/ai-pow-zk composite_public.rs),
::  the 60 public field-elements the verifier binds. Field names + order
::  match the Rust struct exactly so the jet reads them positionally.
+$  ai-public
  $:  cumsum=[@s @s @s @s]      ::  CUMSUM_TILE[0..4], signed i32 (Hoon @s)
      jackpot=[@ @ @ @ @ @ @ @ @ @ @ @ @ @ @ @]   ::  JACKPOT_MSG[0..16], u32
      hash-a=digest8            ::  BLAKE3 keyed-hash of pad(A_row_major)
      hash-b=digest8            ::  BLAKE3 keyed-hash of pad(B_col_major)
      job-key=digest8           ::  κ = BLAKE3(block-header ‖ mining-config)
      commitment-hash=digest8   ::  s_a noise seed
      hash-jackpot=digest8      ::  keyed tile-state hash compared to target
  ==
::
::  The on-chain AI proof body. `version=%3` head keeps `+$ proof`
::  self-describing on the wire (no page:v2 needed — same discriminator
::  trick as %0/%1/%2).
+$  ai-proof
  $:  public=ai-public          ::  consensus-readable public inputs
      stark=@                    ::  Plonky3 BatchProof, bincode bytes, opaque
      config-id=@ud             ::  FRI config selector (pinned to allow-set)
  ==
```

### `+$ proof` `%3` arm (replaces the Stage-6 placeholder)

Recommended shape — **Option A (distinct arm)**: the AI proof is not a
Hoon proof-stream and should not pretend to be one.

```hoon
+$  proof
  $%  $:  version=%2  objects=proof-objects  hashes=(list noun-digest:tip5)  read-index=@  ==
      $:  version=%1  objects=proof-objects  hashes=(list noun-digest:tip5)  read-index=@  ==
      $:  version=%0  objects=proof-objects  hashes=(list noun-digest:tip5)  read-index=@  ==
      $:  version=%3  ai=ai-proof  ==        ::  CHANGED: real AI body
  ==
```

The ZK helpers (`get-pow`, `hash-proof`, the `proof-stream` door in
`ztd/five.hoon`) already `?=`-guard `%3` and crash on it — those guards stay
correct because they never reach the AI arm. The AI path is dispatched
separately (§6). **Rejected — Option B** (pack the proof into
`proof-objects` via new `proof-data` variants): keeps structural identity
for free dispatch but conflates two unrelated proof systems and forces the
opaque `stark` blob to masquerade as a `proof-data`; soundness-illegible.
The placeholder comment in `four.hoon` hinted at Option B; this design
supersedes that hint with Option A, matching Stage-6 §5.1's stated intent
(`version=%3 ai-proof=ai-proof-body`).

### `+$ pow-variant` `%ai-pow` arm (replaces `placeholder=@`)

In `hoon/apps/dumbnet/lib/types.hoon`, mirror the `%dumb-zkpow` shape so the
consensus poke handler dispatches uniformly:

```hoon
+$  pow-variant
  $+  pow-variant
  $%  [%dumb-zkpow prf=proof:sp dig=tip5-hash-atom:zeke bc=noun-digest:tip5:zeke nonce=noun-digest:tip5:zeke]
      [%ai-pow prf=ai-proof:sp bc=noun-digest:tip5:zeke nonce=noun-digest:tip5:zeke]
  ==
```

No separate `dig` field: for the AI path the work value **is**
`prf.public.hash-jackpot` (the difficulty-target preimage). `bc` (block
commitment) and `nonce` are carried so consensus can recompute `job-key`
and `commitment-hash` and check them against the proof's claimed values
(§6, soundness).

---

## 4. Serialization rules (Noun ⇄ bytes)

The proof reaches the wire via `jam` of `page.pow=(unit proof)`
(`tx-engine-1.hoon`; `to-local-page` already does
`(bind pow |=(p=proof (jam p)))`). No custom framing — `jam` is
self-describing and byte-stable. The per-field encoding the Rust mirror and
the verifier jet must honor:

| Field | Rust type | Noun representation | Rule |
|---|---|---|---|
| Goldilocks belt | `Goldilocks` (u64) | `@` | atom, value `< 2^64 − 2^32 + 1` |
| u32 word (`jackpot`) | `u32` | `@` | atom `< 2^32` |
| signed `cumsum` | `i32` | `@s` | Hoon signed atom; jet uses `new:si`/`old:si` (zigzag). NOT raw two's-complement |
| `digest8` / BLAKE3 digest | `[u8; 32]` | `@uxblake` | byte 0 is least-significant byte; `met 3 <= 32`; high bytes zero-padded on decode |
| `stark` (BatchProof) | `Vec<u8>` (bincode) | `@` | one byte-atom, **LSB = byte 0** (Hoon atom-bytes convention); length via `met:3`/`(met 3 stark)` |
| `config-id` | `u32`/enum | `@ud` | small atom; pinned to consensus allow-set (§6) |

The earlier `digest8` **8-tuple of u32** choice is superseded. It is
unnecessarily cell-heavy for Hoon/kernel storage. The current spec uses a
single custom-aura `@uxblake` atom and defines byte order explicitly.
`stark` as a single byte-atom is the natural carrier — `NounSlab`
already moves `Vec<u8>` ⇄ atom; the jet recovers `&[u8]` and feeds it
straight to bincode + the Plonky3 verifier.

### Rust mirror

Add `AiProofNoun` (next to the existing `MatmulProof::encode/decode` in
`ai-pow`, or in the kernel-bridge crate) with:

```rust
impl AiProofNoun {
    fn to_noun<A: NounAllocator>(&self, slab: &mut NounSlab) -> Noun;   // build [public stark config-id]
    fn from_noun(noun: Noun, space: &NounSpace) -> Result<Self, DecodeError>;
}
```

Reuse existing primitives: `T(&mut slab, &[...])` for cells, `D(x)` for
direct atoms, atom-from-`&[u8]` for `stark`, and the same
`take_*`/bounds-checking discipline as `MatmulProof::decode`
(`MAX_*` caps, reject trailing). The `CompositePublicInputs` ⇄ `ai-public`
mapping is positional and total (no `Vec`s), so it cannot fail on shape;
`stark` decode bounds the byte length against a `MAX_STARK_LEN`
(≥ the ≤100 KB target with headroom).

---

## 5. End-to-end data flow

```
miner (Rust, ai-pow-zk)
  prove → BatchProof + CompositePublicInputs
  AiProofNoun{public, stark=bincode(BatchProof), config-id}.to_noun()
        │  packed as [%command %pow %ai-pow prf bc nonce]
        ▼
consensus kernel (Hoon)  — poke handler, inner.hoon
  ?- -.pv.command
    %dumb-zkpow  (existing Hoon STARK verify)
    %ai-pow      ++verify-ai   ← NEW
        │
        ├─ recompute job-key' = blake3(header ‖ cfg);  assert = public.job-key
        ├─ recompute commitment-hash' from (bc,nonce);  assert = public.commitment-hash
        ├─ assert config-id ∈ allowed-ai-configs(height)
        ├─ assert (lte hash-jackpot target)            ← difficulty
        └─ assert (ai-pow-verify-jet stark public config-id)   ← Rust jet, soundness linchpin
```

The jet `+ai-pow-verify` is the only piece that parses `stark`; everything
above it is Hoon and consensus-checked.

---

## 6. Soundness requirements the noun shape must support

These are constraints on the *verifier* (deferred), but the data type is
designed to make them expressible — call them out so the shape is not
under-specified:

1. **Public inputs are checked, not trusted.** `job-key` and
   `commitment-hash` are functions of the block (header/`bc`/`nonce`). The
   kernel must **recompute** them and assert equality against
   `public.job-key` / `public.commitment-hash`. The proof carries them only
   so the jet can form the PI vector the STARK binds to; consensus must not
   accept the prover's word. This is why `bc`+`nonce` ride in the
   `pow-variant` arm.
2. **Difficulty uses `hash-jackpot`.** The work value is
   `public.hash-jackpot` (the keyed tile-state digest the circuit's target
   constraint pins). Target check replaces the `dig`/`check-target` step of
   the ZK path.
3. **Config is pinned.** `config-id` must select from a consensus-fixed
   allow-set (FRI `lb`/`nq`/`pow_bits`) tied to activation height — a miner
   must not be able to pick a weaker FRI config. Reject unknown ids.
4. **`hash-a`/`hash-b` bind the model.** These commit to the mined matrices
   (M52 chunk-Merkle, in-circuit). If/when the chain pins a specific model,
   consensus compares them to the canonical model commitment; until then
   they are bound in-circuit only (documented residual).
5. **No Poseidon2 anywhere** (hard rule): the jet's challenger is Tip5
   (`nockchain_math::tip5`), already true in `ai-pow-zk`.

---

## 7. Verification plan (for the implementation stages)

- **KAT (round-trip):** Rust `AiProofNoun.to_noun → jam → cue → from_noun`
  is identity on a real PROD proof; assert byte-equality of `stark` and
  field-equality of every `public` element.
- **Cross-language:** jam a known `ai-proof` in Hoon and `cue` it in Rust
  (and vice-versa); assert the 60 PI field-elements match
  `CompositePublicInputs` element-for-element.
- **Tamper-reject:** flip one bit of `stark`, one PI word, and `config-id`
  in turn → `++verify-ai` must reject each.
- **Binding-reject:** present a valid proof under the wrong block
  (`job-key`/`commitment-hash` mismatch) → reject.
- **Wire-stability:** `%0/%1/%2` jam outputs stay bit-identical (the `%3`
  arm is additive); regression against stored fixtures.
- **Size:** `(met 3 stark)` ≤ 102,400 once the recursed certificate is the
  carried object (`proof-size-target-100kb`).

---

## 8. Residual (NOT done here — explicitly scoped)

This document is design only. The following is the soundness-critical
invasive work that lands in validated stages **after** review:

1. **`+ai-pow-verify` jet** wrapping `composite_verify_pow_pinned_logup` —
   the linchpin. Jet registration, panic-safety, and KAT against the Rust
   verifier.
2. **`++verify-ai`** Hoon gate with the §6 binding + difficulty + config
   checks, wired into the `%ai-pow` poke dispatch in `inner.hoon`.
3. **`AiProofNoun` Rust mirror** + the round-trip/cross-language KATs (§7).
4. **Landing the `%3` arm + `pow-variant` arm** edits in `four.hoon` /
   `types.hoon`, regenerating `assets/*.jam`, and the wire-stability
   regression.
5. **`config-id` allow-set** + the model-commitment comparison for
   `hash-a`/`hash-b` (#4 of §6).

None of the above is started; the AI verifier remains a stub that rejects
all `%3` blocks until these stages land.
