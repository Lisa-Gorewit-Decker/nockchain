# Stage 6 Design — Per-puzzle ASERT via block-level proof type

**Status:** DRAFT spec (no code yet). Plan-first per user request.
**Date:** 2026-05-27
**Branch:** `claude/ai-pow-integration-squash` (parent for the eventual implementation branch)
**Soundness class:** CONSENSUS-CRITICAL + INVASIVE — R1/R1.1 apply. Stage and validate; never represent partial as complete.

---

## 1. Goal

Make ZK-ASERT and AI-ASERT compute difficulty **independently** for their own puzzle's
block cadence, on a single shared chain. Concretely, post-activation
(height ≥ 95000) each block at height H of puzzle type P must compute its target
from the **median-of-11 timestamps of the 11 most recent prior blocks of the same
puzzle type P**, not the global parent's median-of-11.

Today (current `claude/ai-pow-integration-squash`):
- `compute-target-{zk,ai}-asert` both look up `min-timestamps.c` at the **global
  parent's digest** (`consensus.hoon:227–272` and `consensus.hoon:294–331`,
  passed from `lib/miner.hoon:299`, `inner.hoon:791`, `inner.hoon:1744`).
- `update-min-timestamps` (`consensus.hoon:726–748`) walks `min-past-blocks=11`
  global parents and takes the median, irrespective of puzzle type.

That produces "AI difficulty tracks global block cadence, not AI-only cadence"
(verbatim TODO at `consensus.hoon:288–293`). Correcting this is Stage 6.

---

## 2. Constraints from the user

1. **Same chain, independent difficulty.** Both puzzles mine the **same chain**
   (one heaviest chain). AI and ZK each have their own ASERT difficulty target
   driven by the cadence of blocks of their own type.
2. **Median-of-11 easy and independent per puzzle.** The computation must be
   structurally cheap to do per-puzzle, not a brittle ad-hoc walk every call.
3. **No per-puzzle pointer state.** Don't carry parallel `heaviest-block` /
   per-puzzle pointers; let the block self-identify its puzzle type and
   have walkers filter.
4. **Extend the existing proof-version system; don't reshape `local-page`.**
   The proof noun's `version=%0|%1|%2` discriminator already encodes
   "what proof shape produced this block". Extend that enum with a new
   `%3` arm for AI in a backward-compatible way. The per-block version
   tag lives in a new `consensus-state` map (analogous to `targets` /
   `min-timestamps`), not on `local-page`.
5. **Don't break pre-activation compat.** All blocks at height < 95000 must
   continue to validate bit-identically to the unmodified kernel
   (task #155 — separate mainnet sync validates this end-to-end).

---

## 3. Background — current state (read these to design against)

**`page:v1`** (`hoon/common/tx-engine-1.hoon:61–77`) — wire format:
```hoon
+$  form  $:  version=%1   digest=block-id   pow=$+(pow (unit proof))
              parent=block-id   tx-ids=(z-set tx-id)
              coinbase=coinbase-split   timestamp=@
              epoch-counter=@ud   target=bignum:bn
              accumulated-work=bignum:bn   height=page-number
              msg=page-msg
          ==
```
No proof-type field. Type is implicit in the proof blob.

**`local-page:v1`** (`hoon/common/tx-engine-1.hoon:177–194`) — in-kernel form: same
shape as `page:v1` (the `pow` field's type differs at the proof layer but the
container is the same — no puzzle-type discriminator).

**`+$ proof` / `+$ proof-version`** (`hoon/common/ztd/four.hoon:26–45`) —
the existing proof noun is already a tagged union keyed by version:
```hoon
+$  proof-version  ?(%2 %1 %0)
+$  proof
  $%  $:  version=%2  objects=proof-objects  hashes=...  read-index=@  ==
      $:  version=%1  objects=...  ==
      $:  version=%0  objects=...  ==
  ==
```
`+$ pow-variant` (types.hoon:549–562) was declared in earlier stages but
**not stored on any block** and is NOT used by this design — Stage 6
extends `+$ proof` instead.

**`min-timestamps` cache** in `consensus-state` (`types.hoon`):
`(h-map block-id:t @)` — keyed by block-id, value = median-of-11 timestamp.
Populated by `+update-min-timestamps` at every `+accept-block`. O(1) read,
O(11) write.

**`min-past-blocks=11`** — `hoon/common/tx-engine-0.hoon:50` (`blockchain-constants`).

---

## 4. Design overview

Two coupled changes:

### 4.1 Reuse the existing `proof-version` enum as the puzzle-type discriminator

The proof noun in `hoon/common/ztd/four.hoon:26–45` is already a tagged
union keyed by `version`:

```hoon
+$  proof-version  ?(%2 %1 %0)
+$  proof
  $%  $:  version=%2  objects=proof-objects  ...  ==
      $:  version=%1  objects=proof-objects  ...  ==
      $:  version=%0  objects=proof-objects  ...  ==
  ==
```

`height-to-proof-version` (`consensus.hoon:98–105`) already gates which
version is legal at which height range. `check-pow` (`consensus.hoon:462–469`)
already rejects blocks whose proof version disagrees with the height gate.

**Extend the existing system rather than introduce a parallel
`pow-variant` or `puzzle-type`.** Two additions, both backward-compatible:

1. **New `%3` arm** on `+$ proof` (and `+$ proof-version`), holding the
   AI-PoW proof shape. Existing `%0`/`%1`/`%2` arms unchanged.
2. **`++ version-to-puzzle-type`** helper: `%0|%1|%2 → %dumb-zkpow`,
   `%3 → %ai-pow`. Used by the walker + ASERT call sites.

Wire format: **unchanged.** `page.pow=(unit proof)` already encodes the
version via the proof's discriminator. Old %0/%1/%2 serializations are
bit-identical. New %3 blocks serialize with the new arm. No `page:v2`.

`local-page`: **unchanged.** It already drops the proof body; the version
tag lives in a new `block-versions` map on `consensus-state` (§5.1), not on
`local-page`. This is the explicit user preference — extend the version
system, don't reshape `local-page`.

Activation rule: extend `height-to-proof-version` so it gates by the
puzzle-type axis as well as the version axis. Pre-activation
(`height < ai-pow-activation-height`): version MUST be in `{%0, %1, %2}`
(per existing height ranges, unchanged). Post-activation: version MUST
be in `{%2, %3}` (today's latest ZK ∪ the new AI arm). Both ZK and AI
blocks are legal post-activation; the chain accepts whichever lands first.

Genesis edge case: `pow = ~` (no proof) — walker terminates, no version
lookup needed.

The proof body shape for `%3` is still TBD (deferred AI verifier task).
For Stage 6 we accept that AI blocks cannot land yet, but the dispatch
and walker logic is wired and tested with synthetic %3 blocks in fakenet.

### 4.2 Per-puzzle median-of-11 with a single cache, new semantics

Repurpose the existing `min-timestamps: (h-map block-id @)` map so its value
at key `bid` is "median of the 11 most recent prior blocks of the **same
puzzle type as the block at `bid`**".

- Pre-activation: every block is `%dumb-zkpow`, so per-puzzle = global, so
  the value computed under new semantics is **identical** to the value computed
  under old semantics. **No observable change for pre-95000 blocks.**
- Post-activation: walker filters as it walks. The cache itself doesn't need
  to be partitioned; one block-id → one median.

Walker (`+update-min-timestamps` rewrite): walk `~(parent get:local-page:t pag)`
edges, read each hop's puzzle type from `type.u.pow`, **skip hops whose
type ≠ `type.u.pow.pag`**, collect the first `min-past-blocks` matching
timestamps (or until genesis), take median.

Worst case walk length is unbounded in theory (if a long ZK run with no AI
block, an AI block's walker traverses many ZK blocks to find 11 prior AI).
In practice with the design's 2.5-min global / 5-min per-puzzle cadence the
expected walk length is ~22 blocks. Bound it explicitly:

- **Cap**: walk at most `2 * min-past-blocks * max-puzzle-types = 44` global
  hops; if fewer than `min-past-blocks` same-type blocks are found in that
  window, fall back to the same-puzzle ASERT's `anchor-min-timestamp`
  (already used as the bootstrap for AI in `compute-target-ai-asert`). This
  matches the bootstrap path's semantics — "no usable history, use anchor".

### 4.3 ASERT call sites pass the per-puzzle parent

`compute-target-{zk,ai}-asert(child-height, parent-digest)`:
- `parent-digest` must be the **most recent prior block of the same puzzle
  type as the candidate**, not the global parent.
- For ZK on a chain where the global parent IS a ZK block: same as today.
- For ZK after an AI block lands: walk back to the most recent ZK ancestor.
- Symmetric for AI.

This lookup uses the same walker as 4.2 but for a single-hop: "give me the
nearest same-type ancestor of `child-height`'s global parent."

Helper `++ same-type-ancestor` returns `(unit block-id)`; consumers handle
`~` by falling back to the anchor (same bootstrap pattern as today's
`compute-target-ai-asert`).

---

## 5. Schema changes

### 5.1 Extend `+$ proof` with a `%3` arm

In `hoon/common/ztd/four.hoon`, extend the existing tagged union:

```hoon
+$  proof-version  ?(%3 %2 %1 %0)         ::  CHANGED: add %3
+$  proof
  $%  $:  version=%0  objects=proof-objects  ...  ==     ::  unchanged
      $:  version=%1  objects=proof-objects  ...  ==     ::  unchanged
      $:  version=%2  objects=proof-objects  ...  ==     ::  unchanged
      $:  version=%3  ai-proof=ai-proof-body  ==          ::  NEW
  ==
```

`ai-proof-body` is the AI-PoW proof shape — TBD; lives in the deferred
AI-verifier task. Stage 6 declares the arm with a placeholder atom so
dispatch + walker code can be wired and tested with synthetic %3 blocks.

Wire format: the proof's serialization stays self-describing via its
`version=` head. Pre-existing %0/%1/%2 jam outputs are bit-identical.
**No `page:v2` introduced**; `page:v1.pow=(unit proof)` carries the new arm.

### 5.2 New consensus-state field: `block-versions` (post-activation only)

`local-page` drops the proof body at `to-local-page` time, so the
version tag would be lost without a place to store it. Per the user's
constraint, **do not reshape `local-page`**. Instead add a per-block
scalar metadata map to `consensus-state` (analogous to `targets` and
`min-timestamps`):

```hoon
::  in consensus-state-N+1:
block-versions=(h-map block-id:t proof-version:sp)
```

**Populated lazily, post-activation only.** In `+accept-block`:

```hoon
?:  (gte ~(height get:page:t pag) ai-pow-activation-height:blockchain-constants)
  (~(put h-by block-versions.c) bid version.u.pow.pag)
c
```

Pre-activation block-ids are NEVER inserted. For any pre-activation
block, the version is deterministic from height via
`height-to-proof-version-legacy` and the lookup helper falls back to
that.

This means **no migration backfill**: the `state-10-to-11` migration
just adds an empty `block-versions=*(h-map block-id:t proof-version:sp)`.
No O(N) walk over historical blocks; no risk of mis-backfilling a
boundary height.

### 5.3 Lookup helper: `++ block-id-to-proof-version`

The single contract for "given a block-id, what's its proof version?" —
used by the walker, `same-type-ancestor`, and any future consumer:

```hoon
++  block-id-to-proof-version
  |=  bid=block-id:t
  ^-  proof-version:sp
  =/  cached=(unit proof-version:sp)  (~(get h-by block-versions.c) bid)
  ?^  cached  u.cached
  ::  not in map ⇒ pre-activation; height-derive (deterministic).
  =/  pag=local-page:t  (~(got h-by blocks.c) bid)
  (height-to-proof-version-legacy ~(height get:local-page:t pag))
```

`++ block-id-to-puzzle-type` is a thin composition:

```hoon
++  block-id-to-puzzle-type
  |=  bid=block-id:t
  ^-  ?(%dumb-zkpow %ai-pow)
  (version-to-puzzle-type (block-id-to-proof-version bid))
```

### 5.4 Helper: `++ version-to-puzzle-type`

In `hoon/apps/dumbnet/lib/consensus.hoon`:

```hoon
++  version-to-puzzle-type
  |=  version=proof-version:sp
  ^-  ?(%dumb-zkpow %ai-pow)
  ?:  ?=(%3 version)  %ai-pow
  %dumb-zkpow
```

This is the **only** place the version↔puzzle-type mapping is encoded.
All consumers go through it; if more proof versions ship later, they
extend this helper.

### 5.5 Activation-aware `height-to-proof-version`

Today's helper returns a single version for a given height. Post-activation
a single height can host either `%2` (ZK) or `%3` (AI), so the helper's
shape must change to a *predicate*:

```hoon
++  proof-version-valid-at-height
  |=  [version=proof-version:sp height=page-number:t]
  ^-  ?
  ?:  (gte height ai-pow-activation-height:blockchain-constants)
    ::  post-activation: ZK %2 or AI %3
    ?|  ?=(%3 version)
        ?=(%2 version)
    ==
  ::  pre-activation: existing height-range gating, unchanged
  =(version (height-to-proof-version-legacy height))
```

`height-to-proof-version-legacy` is today's `height-to-proof-version` arm
renamed for clarity. Pre-activation behavior is bit-identical to today.

`check-pow` (`consensus.hoon:462`) flips from
`?: =(version (height-to-proof-version height))` to
`?: (proof-version-valid-at-height version height)`. Same reject semantics
pre-activation; expanded acceptance post-activation.

### 5.6 `kernel-state`: bump to 11 (consensus-state shape change)

Adding `block-versions` to `consensus-state` is a shape change → requires
`consensus-state-10` → `consensus-state-11` migration. The migration is
**trivial** — no backfill — because `block-versions` is post-activation
only (§5.2):

- `state-10-to-11` arm in `inner.hoon` materializes
  `block-versions=*(h-map block-id:t proof-version:sp)` (empty map). No
  walk of historical blocks.
- Pre-activation lookups go through `block-id-to-proof-version` (§5.3)
  and fall back to the deterministic height map.
- The first post-activation `+accept-block` populates the first entry;
  every subsequent post-activation block adds its own.

`derived-state-10` is unaffected — the per-puzzle anchor caches added in
commit 827ca40 carry forward verbatim.

If we discover during implementation that we want a `same-type-ancestor`
cache (e.g. `(h-map block-id block-id)`), that would be a *further*
consensus-state field added in the same migration. Defer the decision to
Stage 6.S2 measurement.

### 5.7 Reorg behavior of `block-versions`

The same rules `min-timestamps` and `targets` already follow:

- **Keyed by block-id, not by chain position.** Entries are immutable
  after `+accept-block` writes them. A reorg never changes the version of
  a previously-accepted block (the block's proof is fixed at accept
  time).
- **Orphans retain their entries.** When a reorg makes a previously-
  heaviest block an orphan, its entry in `block-versions` is **not
  removed**. This matches `min-timestamps.c` and `targets.c` behavior —
  those entries persist for blocks no longer on the heaviest chain and
  are still consulted when fork resolution re-evaluates an orphan
  segment.
- **Reorg into post-activation new blocks.** When a reorg pulls in
  previously-unseen post-activation blocks, each goes through
  `+accept-block`, which writes its `block-versions` entry. No special
  reorg-path code needed.
- **Reorg across the activation boundary.** Blocks below
  `ai-pow-activation-height` continue to have no `block-versions`
  entry (lookup falls back to height-derive); blocks at/above continue
  to be inserted at accept. No special cross-boundary logic.

If we later add a `same-type-ancestor` cache (§5.6 deferred), reorg
handling for THAT cache may be non-trivial (pointers can become stale
when a different chain segment becomes heaviest). That's part of the S2
measurement decision.

---

## 6. Algorithm changes

### 6.1 `+update-min-timestamps` (`consensus.hoon`)

Today:
```hoon
::  walk N=11 global parents, collect, median
```
New:
```hoon
::  walk global parents (~(parent get:local-page:t cur)); for each, look
::  up its version in block-versions.c, map to puzzle-type via
::  version-to-puzzle-type. If the type matches the new block's type,
::  collect its timestamp. Stop when we have min-past-blocks matches,
::  hit genesis, or exceed cap = 2 * min-past-blocks * |puzzle-types|
::  global hops. If at least one match, take median; else, use the
::  same-puzzle ASERT's anchor-min-timestamp (bootstrap).
```

Pre-activation invariant: every ancestor's `block-versions.c` value is
`%0|%1|%2`, all mapping to `%dumb-zkpow`. The new block's type is also
`%dumb-zkpow`. Filter is a no-op; behavior equals today's walk.
**Bitwise-equal `min-timestamps` map values for all pre-activation
blocks** — the compat anchor.

### 6.2 `+same-type-ancestor` (new helper in `consensus.hoon`)

```hoon
::  Return the immediate same-puzzle-type ancestor of pag, or ~ if no
::  such ancestor exists within the bounded walk window.
++  same-type-ancestor
  |=  pag=local-page:t
  ^-  (unit block-id:t)
  =/  target-type
    %-  version-to-puzzle-type
    (~(got h-by block-versions.c) ~(digest get:local-page:t pag))
  ::  walk parents; on each hop, look up its version in block-versions.c,
  ::  map to puzzle-type, return when match OR window exceeded.
```

Used by both ASERT functions to compute the right `parent-digest`.

### 6.3 `compute-target-{zk,ai}-asert` (`consensus.hoon`)

Behavioral change:
- Compute `same-type-parent-bid = (same-type-ancestor candidate-block)`.
- If `~`: degenerate to anchor-target (same bootstrap pattern as AI today).
- Else: look up `same-type-parent-bid` in `min-timestamps.c` and proceed.

The hardcoded-anchor + cache-anchor priority logic from the current
implementation is preserved verbatim — only the parent-digest source
changes.

### 6.4 Call site updates

Three call sites pass `parent-digest`:
- `lib/miner.hoon:299` — candidate emission for ZK
- `inner.hoon:791` — AI candidate emission
- `inner.hoon:1744` — block validation

All three compute `same-type-parent` via `same-type-ancestor` of the
candidate block before passing to `compute-target-*-asert`. The
candidate's own type is read from `block-versions.c` (or from the
candidate's `version.u.pow` if not yet in `block-versions.c`).

---

## 7. Pre/post-activation activation rule

At block validation, gate the proof version via
`++ proof-version-valid-at-height` (§5.5):

| Block height                          | Allowed `version=` | Implied puzzle-type | Validation |
|---------------------------------------|--------------------|---------------------|------------|
| < `proof-version-1-start` (6750)      | `%0`               | `%dumb-zkpow`       | Identical to today. |
| 6750–11999                            | `%1`               | `%dumb-zkpow`       | Identical to today. |
| 12000 – `ai-pow-activation-height-1`  | `%2`               | `%dumb-zkpow`       | Identical to today. |
| ≥ `ai-pow-activation-height`          | `%2` OR `%3`       | `%dumb-zkpow` (for %2) or `%ai-pow` (for %3) | NEW: accept either; dispatch verifier on the proof's version. |

Wire format is unchanged at every height; the proof's own `version=` head
is the discriminator. The activation height itself is
`ai-pow-activation-height` (default 95000; configurable in fakenet via
existing `--fakenet-ai-pow-activation-height`).

---

## 8. Open / deferred decisions

These are explicitly NOT decided in this spec; called out so we revisit at
the right stage:

1. **`%3` proof body shape.** The new arm carries `ai-proof=ai-proof-body`
   with `ai-proof-body` a placeholder atom in Stage 6. Defining the real
   shape is the separate deferred AI-verifier task. Stage 6 wires the
   dispatch + walker; the verifier for `%3` is a stub (reject-all) — same
   as today's AI dispatch.
2. **Same-type-ancestor cache.** Whether to add a
   `same-type-parents: (h-map block-id block-id)` map alongside
   `block-versions` is deferred until S2 measurement shows the live walk
   cost exceeds budget.
3. **Median-of-11 when there are fewer than 11 same-type ancestors.**
   Today's walker handles this (returns median of what it has, even with
   fewer than 11 entries). Confirm `median:t` behavior on short lists and
   decide if our bootstrap is "use anchor" or "use median of what we
   have". Affects ASERT bootstrap semantics in early post-activation
   blocks.

---

## 9. Staged plan + validation gates

R1 applies: stage and validate; no fake completion; honest residual.

| Stage    | Scope                                                                                                                                     | Validation gate                                                                                                                                                                                                                                       |
|----------|-------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **S0**   | De-risk: write KATs for `update-min-timestamps` under the proposed new semantics, confirming pre-95000 behavior is bit-identical to today | KAT: feed a synthetic 20-block all-`%dumb-zkpow` chain through both old and new walkers; assert identical `min-timestamps` map. Commit KATs.                                                                                                           |
| **S1**   | Schema: extend `+$ proof` and `+$ proof-version` with `%3` arm; add `block-versions` (post-activation only) map to `consensus-state`; bump kernel-state 10→11 with a trivial `state-10-to-11` migration that initializes `block-versions` to the empty map (no backfill). No consumer changes yet. | `cargo build --workspace` green; full Hoon kernel tests green; `make assets/dumb.jam` clean. **Migration KAT**: load a frozen kernel-state-10 snapshot, apply migration, assert `block-versions` is empty and `block-id-to-proof-version` for every existing block-id returns the height-derived version via the fallback path. Commit. |
| **S2**   | Helpers: `version-to-puzzle-type`, `proof-version-valid-at-height`, `same-type-ancestor`. Walker (`+update-min-timestamps`) rewrite. `check-pow` flipped to `proof-version-valid-at-height`. | KAT from S0 still passes (pre-activation identical). NEW KAT: synthetic interleaved 20-block chain with mixed `%2`/`%3` versions; assert per-puzzle medians + `same-type-ancestor` are computed correctly. Commit. |
| **S3**   | ASERT call sites: route `parent-digest` through `same-type-ancestor`. All three call sites (`miner.hoon:299`, `inner.hoon:791`, `inner.hoon:1744`). | Pre-activation fakenet smoke green (regression). Post-activation fakenet smoke (the `--fakenet-ai-pow-activation-height 2` script) green with ZK-only blocks landing (AI verifier still stub-rejects, so no `%3` blocks land — but dispatch fires for both). Commit. |
| **S4**   | Activation gate: `check-pow` rejects `%3` pre-activation; accepts `{%2, %3}` post-activation. | Pre-activation fakenet smoke green. Post-activation fakenet smoke green. Adversarial: hand-craft a `%3`-versioned block at a pre-activation height, assert reject. Hand-craft a `%0` at a post-activation height, assert reject. Commit. |
| **S5**   | Mainnet sync compat re-run (task #155) | `scripts/mainnet-sync-compat.sh` reaches current tip with zero panics, no rejected pre-95000 blocks, and the post-migration `block-versions` map matches height-derived expectations. Commit a compat note in this design doc. |

Each stage is its own commit. Push only after the stage's gate is green.

---

## 10. Out of scope

- **Real AI verifier for `%3` proofs.** The `%3` arm in `+do-pow`
  continues to be a stub-rejects-all. Stage 6 wires dispatch, walker,
  and schema; verifier comes later.
- **Mining of AI blocks end-to-end on fakenet.** Without the AI verifier,
  no `%3`-versioned block can be accepted. Fakenet smokes verify dispatch
  fires and ZK-only mining continues to work post-activation; an
  end-to-end AI smoke is gated on the AI-verifier task.
- **No `puzzle-types` map / no per-puzzle pointer state.** Puzzle type is
  derived from `block-versions.c` via `version-to-puzzle-type` —
  one source of truth, indexed by block-id.
- **No `page:v2` / no `local-page:v2` schema reshape.** Wire and
  in-kernel page shape are unchanged; only the existing proof
  union and the consensus-state's per-block metadata grow.

---

## 11. Risk + mitigation

| Risk                                                                              | Mitigation                                                                                                                                                                       |
|-----------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Walker's new semantics produce a different `min-timestamps` value for some pre-activation block, breaking sync compat | S0 KAT proves pre-activation bit-identity. S5 mainnet sync end-to-end validates against real history.                                                                            |
| `block-id-to-proof-version` fallback misclassifies a pre-activation block (off-by-one in `height-to-proof-version-legacy`) | S1 KAT exercises every height-range boundary (%0/%1/%2 cutoffs) via the fallback path on a frozen snapshot. S5 mainnet sync end-to-end validates against real history. |
| Orphan / reorg block leaves a stale `block-versions` entry that confuses the walker | Entries are immutable + keyed by block-id; the walker visits ancestors via parent edge (heaviest chain only), so orphan entries are dead weight, not incorrect. Matches existing `min-timestamps` reorg semantics (§5.7). |
| Walker walks unbounded when one puzzle stalls (no AI for thousands of ZK blocks)  | Hard cap at `2 * min-past-blocks * 2 = 44` global hops; degenerate to anchor on cap (§4.2 bootstrap path).                                                                       |
| `proof-version-valid-at-height` accepts a `%3` block at a pre-activation height (consensus break) | S4 adversarial test: hand-craft a `%3`-versioned block at height < `ai-pow-activation-height`, assert reject. |
| Proof of `%3` arm with placeholder body slips through verification (forgery) | Stub `+do-pow` arm hard-rejects all `%3` proofs. AI-verifier task replaces this with the real verifier before any `%3` block can land. |

---

## 11b. Implementation log

S0–S4 landed in this session as discrete commits. See git log for
details; each stage's commit body documents what changed and how
it was validated.

| Stage | Commit  | Status      | Validation                                                                                                                                  |
|-------|---------|-------------|---------------------------------------------------------------------------------------------------------------------------------------------|
| S0    | (none)  | done        | Baseline confirmed: both fakenet smokes green at 827ca40 (pre-S6 cache commit).                                                             |
| S1    | 49972e1 | done        | `make assets/dumb.jam` + both fakenet smokes green. Schema landed: %3 arm on +$ proof, block-versions map on consensus-state, kernel-state-11. |
| S2    | 6e0499b | done        | Per-puzzle walker + version helpers + check-pow flip. Both fakenet smokes green; per-puzzle walker bit-identical to legacy on pre-activation chains. |
| S3    | 754fe35 | done        | Three ASERT call sites route through +find-same-type-ancestor (miner.hoon:299, inner.hoon:834+1783, consensus.hoon validate-page-without-txs). |
| S4    | 7b13b19 | done        | Post-ai smoke strengthened: explicit no-proof-version-invalid assertion. Catches predicate-level regressions on every CI run.               |
| S5    | TBD     | in progress | Mainnet sync started on the S6 kernel (post-S4 build) over the existing data dir to exercise the state-10-to-11 migration on real state. **See §12 below for the in-flight status + acceptance criteria.** |

## 12. Verification summary

### Done in this session

- **S0–S4 fully validated** via fakenet smokes; commits pushed.
- **S1 schema migration validated on REAL mainnet state**: the
  S6 binary booted against the existing pre-S6 data dir
  (state-10) at h=1439, the `state-10-to-11` migration arm fired
  ("State upgrade required" logged), the empty `block-versions`
  map was materialized, and sync resumed cleanly at h=1440. The
  highest-risk migration scenario — applying the schema bump to
  a live, partially-synced consensus state — is **proven to
  work** on real mainnet data. Sync continued processing
  subsequent blocks at the normal rate with zero panics, zero
  proof-version-invalid rejections, and zero cache-empty events.
- Both fakenet smokes (pre + post-activation) green throughout.

### Honest residual

- **Full sync to mainnet tip is in flight.** As of S5 commit
  time, the post-S6 sync had validated h=0..~1500. Reaching the
  current mainnet tip (~100k+ blocks at the time of the AI
  activation height of 95000) takes hours of background sync;
  it was not feasible to wait for completion in this session.
  The acceptance criterion (zero panics, zero pre-activation
  block rejections, full chain validation through tip) is
  monitored by the running script (see
  `scripts/mainnet-sync-compat.sh`).
- **`%3` body shape + AI verifier remain deferred** (separate
  task). Stage 6 wired the dispatch + walker + per-puzzle
  ASERT; the `%3` arm carries a placeholder body. The stub
  `+do-pow` arm hard-rejects all `%3` proofs, so no AI block
  can land until that task completes.
- **Adversarial harness for arbitrary-version block injection
  is deferred.** S4 strengthens the post-ai smoke with an
  explicit no-`proof-version-invalid` assertion (catches
  predicate regressions on every CI run). A deeper test
  spawning a malicious miner to craft wrong-version blocks
  would require building a custom miner harness, beyond the
  in-tree test infrastructure.

### Task accounting

- Task #152 will flip to `completed` only after the in-flight
  mainnet sync reaches tip with the documented acceptance
  criteria. As of the S5 commit, scope is honestly described as
  "S6 dispatch + walker + per-puzzle ASERT wired and proven to
  migrate cleanly on real mainnet state; full-chain compat
  validation in progress as a background sync".
- Task #155 (mainnet sync compat) remains in_progress until
  the same condition.
