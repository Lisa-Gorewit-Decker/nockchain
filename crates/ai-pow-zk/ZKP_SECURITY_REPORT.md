# ai-pow-zk — ZKP security report

Audit date: 2026-05-15. Auditor pass over the M10.1c composite
STARK (`CompositeFullAir` via `p3-uni-stark`, Goldilocks + Tip5 +
FRI) as wired at HEAD (`d619534`), including the C1–C4 bindings
and the F1 `zk_bridge` integration landed this session.

**Headline:** the proof system's *cryptographic primitives*
(FRI, Fiat-Shamir, Tip5, BLAKE3, the field encodings) are sound
and give ~120-bit provable security. **But the proof does not
soundly bind the proof-of-work statement against a malicious
prover**, because the public-input bindings (HASH_A, HASH_B,
HASH_JACKPOT, JOB_KEY, COMMITMENT_HASH) are gated by
prover-controlled selectors that nothing forces to fire. This is
a **critical** soundness break, not a fidelity nit. It
supersedes the optimistic "C1–C4 resolved, no soundness gap"
line in `GAP_AUDIT.md` on the malicious-prover question (the C1–C4
*constraints* are correct; they are *not enforced*).

## Threat model

A PoW SNARK is only meaningful against a **malicious prover** who
controls the entire witness and wants a verifying proof for the
least possible work (ideally: a winning-difficulty proof with no
matmul at all). The honest-prover path (`zk_bridge`,
`CompositeTrace::derive_*`) is irrelevant to soundness — a real
attacker hand-builds `trace.matrix` and the public-input vector
directly and calls `p3_uni_stark::prove`.

## Findings

### CRIT-1 — selector-gated PI bindings are not enforced to fire → PoW forgeable

**Severity: Critical. STATUS: RESOLVED 2026-05-15 (commit
`9ec529e`).** Fixed by `CompositeFullAirPinned`: the 5 program
columns (`CONTROL_PREP` + the `*_PREP` set) are committed as a
p3-uni-stark **preprocessed trace** in the verifying key, with an
unconditional per-row constraint `main[col] == preprocessed[k]`.
`CONTROL_PREP` pins all 21 selectors + `MAT_ID` via the control
chip's existing packing constraint, so the C1/C3/C4 bindings can
no longer be vacated. The verifier rebuilds the canonical program
from the trusted shape (never from the proof). Production path
(`ai-pow::zk_bridge` → `mine()` gate) and the F1 harness use
`composite_{prove,verify_pow}_pinned`. Exhaustive adversarial
regression `composite_proof::tests::crit1_*` (4/4): the
zeroed-selector forged-winning-PoW proof is **rejected** vs the
canonical VK; tampering any program column is rejected; a forged
`HASH_JACKPOT` fails the now-live C4 binding even with the
correct program. The original analysis below is retained as the
rationale.

**Original severity: Critical (PoW soundness ≈ 0 bits as wired).**

The chain-anchoring / commitment public inputs are bound by
*selector-gated* constraints in `composite_full_air.rs`:

```
IS_HASH_A             · (CV_OUT[i] − PI_HASH_A[i])          = 0
IS_HASH_B             · (CV_OUT[i] − PI_HASH_B[i])          = 0
IS_HASH_JACKPOT       · (CV_OUT[i] − PI_HASH_JACKPOT[i])    = 0
IS_USE_JOB_KEY        · (CV_IN[i]  − PI_JOB_KEY[i])         = 0
IS_USE_COMMITMENT_HASH· (CV_IN[i]  − PI_COMMITMENT_HASH[i]) = 0
```

Each is vacuous when its selector is 0 on every row. The
selectors live in `CONTROL_PREP` + the 21 unpacked selector
columns. The control chip (`chips/control.rs::eval`) enforces
**only**: (a) each selector is boolean, (b) `MAT_ID` limb
decomposition, (c) `CONTROL_PREP = base-2 pack(selectors,
mat_id)`. It does **not** enforce that any selector is ever 1, nor
a `Σ selector = 1` aggregate.

The decisive fact: **`CONTROL_PREP` and the selectors are
prover-controlled main-trace columns, not a verifier-fixed
preprocessed commitment.** `composite_layout.rs:124` *intends*
all `*_PREP` columns to be "PREPROCESSED — committed at setup,"
but `CompositeFullAir`'s `BaseAir` impl
(`composite_full_air.rs:81`) does **not** implement
`preprocessed_trace()`. With the default `None`, `p3-uni-stark`
has no preprocessed trace: every column — including
`CONTROL_PREP`, every selector, `NOISE_PACKED_PREP`,
`CV_OR_TWEAK_PREP`, `AB_ID_PREP` — is part of the single
prover-supplied `trace.matrix` passed to
`prove(config, &CompositeFullAir, trace.matrix, &pis)`. There is
no verifying-key commitment pinning the program; `verify` checks
no preprocessed root.

**Exploit (no work, winning proof).** A malicious prover:

1. builds a trace with **every selector 0 on every row**
   (`CONTROL_PREP = pack(0…0, 0) = 0` on all rows — the control
   chip is satisfied: booleans hold, packing holds);
2. supplies a public-input vector with `HASH_JACKPOT = 0` (or any
   value ≤ the difficulty target), and arbitrary
   `HASH_A/HASH_B/JOB_KEY/COMMITMENT_HASH`;
3. fills the remaining chips with their trivial all-zero
   satisfying assignment (baseline trace — already known to
   verify).

All AIR constraints hold (the five bindings above are
`0 · (…) = 0`). `composite_verify` succeeds. `composite_verify_pow`
then checks the *prover-chosen* `HASH_JACKPOT` against `target`
and passes. **A valid winning-difficulty proof was produced with
no matmul, no hashing, no work, and no tie to any block** (κ /
`s_a` / matrices all unconstrained). PoW soundness is broken.

The earlier M52 / GAP_AUDIT notes ("uniqueness is a
trace-generator obligation", "the F1 path sets it") describe the
*honest* generator; against a malicious prover a trace-generator
obligation is not a soundness guarantee.

**Contrast — what *is* soundly bound:** `CUMSUM_TILE` and
`JACKPOT_MSG` use `builder.when_last_row().assert_eq(cur, pi)` —
an **unconditional** boundary constraint (the `when_last_row`
selector is a fixed row-position predicate, not a prover
witness). Those two PIs cannot be unbound this way. The fix for
the others is to make them analogously unconditional or
program-pinned (see Remediation).

### HIGH-2 — HASH_JACKPOT attests a constant, not the work

**Severity: High. STATUS: soundness gap RESOLVED 2026-05-15
(`15ba9a3`); HIGH-2.2 fidelity LARGELY CLOSED 2026-05-16 — the
honest proof now attests the *real* folded tile state, with one
precisely-scoped useful-work-soundness residual.** Update
2026-05-16: the keystone was generalised to the faithful
last-row `JACKPOT_MSG[0..16] == FOLD_STATE[0..16]` (the full
16-word Pearl §4.5 folded `TileState M`, replacing the 2×2
`CUMSUM_TILE[0..4]`+zero stop-gap; `e6c9c84`). A `FoldChip`
(Pearl §4.5 rotl13-XOR, Option B2) + `place_fold_chain` thread
the real solved tile's per-stripe fold into `FOLD_STATE`, and
`zk_bridge`/`mine()` now place it through the **production
Route-A batch-stark path** (CRIT-1 pin **and** the
`noised_packed` LogUp in one proof — `composite_*_pinned_logup`,
`8ed627e`/`37f5c0f`). So an *honest* proof now attests
`HASH_JACKPOT = BLAKE3(real M, key=s_a)` — byte-equivalent to
the plain miner (`high2_2_xstep_fold_pipeline_byte_equiv_plain`)
— not `BLAKE3(0,s_a)`. A pre-existing latent JackpotChip bug
(the `JACKPOT_MSG` RAM recurrence ungated by `is_active`, which
forbade any non-zero `JACKPOT_MSG`; masked for years because
every test hashed an all-zero message) was root-caused and fixed
(`354b47e`). Validated: full `cargo test -p ai-pow --features
zk` green incl. `end_to_end` 13/0 (every `mine()` via real-M
Route-A), `zk_bridge` 19/0; `crit1_*`/`high2_*`/`routea_*` no
regression. Adversarial `composite_proof::high2_free_jackpot_message_rejected`
still rejects a planted free winning message.

**§6(a) fold-schedule pin — RESOLVED 2026-05-16 (`aa82ce3`).**
The fold/matmul *schedule* (`FOLD_IS_FOLD` + the 4-bit fold-slot
index `stripe%16`) is now packed into the CRIT-1-pinned
`CONTROL_PREP` polyval (bits `2^47`/`2^48`, past the 21 selectors
+ 26-bit `MAT_ID`) and `ControlChip` asserts the extended pack;
`place_fold_chain` writes it and `extract_program` lifts it into
the verifier-rebuilt canonical program. **Which rows fold, and
into which of the 16 slots, is now verifier-fixed** — a malicious
prover can no longer fabricate a fold schedule. Done *without*
widening the preprocessed trace (avoids the §4.C.8 ~10x trap; it
reuses the existing pinned column, and `is_fold=0/slot=0`
contributes exactly 0 so every non-fold row's `CONTROL_PREP` is
byte-identical — zero blast radius). Exhaustively tested
(`chips::control::tests` +6: positive + slot-mismatch /
stale-`CONTROL_PREP` / claimed-but-absent-fold rejects +
bit-layout + zero-blast-radius) and e2e-validated (ai-pow-zk lib
322/0 incl. `high2_2_fold_chain_pinned_logup`/`routea_*`/`crit1_*`;
ai-pow `--features zk` green: lib 64/0, `end_to_end` 13/0,
`adversarial` 19/0; byte-equivalence preserved).

**Remaining useful-work-soundness residual — §6(b) + §4.E, one
*entangled* item (precisely scoped, not a *proof*-forgery
hole):** the per-stripe `X_STEP` fed to the `FoldChip` is placed
by the honest bridge but **not yet in-circuit bound** to the
matmul accumulator (`XStepChip` is byte-equiv-proven standalone
but not composite-wired to force `X_STEP ← ⊕CUMSUM_TILE ←
committed A/B`; the matmul subtile-sweep rows are not placed on
the honest path). Because nothing in-circuit yet ties the digest
to a *specific tile's committed-matrix accumulator*, the attested
`(tile_i,tile_j)` (§4.E) **cannot be soundly bound independently**
— a free tile/`X_STEP` PI would be vacuous. So §6(b) and §4.E are
the *same* cryptographic-core binding and land together,
reconciled with **MED-3** (verifier-side tile derivation). A
*malicious* prover could meanwhile supply a fabricated `X_STEP`
→ fabricated `M` → forged `HASH_JACKPOT`; held in check by CRIT-1
+ the keystone + §6(a) (the proof cannot be forged against the
canonical program and the fold schedule is verifier-fixed); the
open gap is only that the attacker is not yet *forced* to do the
real matmul for `X_STEP`/the tile. Closing it = place the matmul
subtile-sweep rows on the honest path, composite-wire `XStepChip`
to force `FOLD_XSTEP == ⊕CUMSUM_TILE` per stripe (Route-A
`noised_packed` already binds `CUMSUM_TILE`'s inputs to the
committed matrices), and bind `(tile_i,tile_j)` to that
accumulator's offsets per MED-3 — *not* a wide preprocessed block
(§4.C.8 ~10x trap; the §6(a) schedule pin already demonstrates
the cheap CONTROL_PREP-reuse pattern). Tracked as HIGH-2.2
§6(b)+§4.E; invasive, its own focused effort. The original
analysis below stands as the historical rationale.

**Original severity: High (PoW *usefulness* not enforced even if
CRIT-1 is fixed).**

`zk_bridge::prove_and_verify` and `place_jackpot_hash_block` hash
an **all-zero** `JACKPOT_MSG`: `HASH_JACKPOT = BLAKE3(0,
key = s_a)`. This is a fixed function of `s_a` alone. Even with
CRIT-1 fixed (selector forced to fire), the C4 binding would only
prove "the prover computed `BLAKE3(0, key=s_a)`" — which requires
no matmul and no tile-state evolution. The difficulty check (C2)
would then gate on a value independent of any useful work. The
rotate-XOR-13 tile-state fold that *should* feed `JACKPOT_MSG`
(Pearl §4.5; the matmul→jackpot interleave) is not wired. Until
it is, the SNARK does not prove proof-of-*work*; it proves
knowledge of `s_a` (which is public-derivable). Documented in
`GAP_AUDIT.md` as a "fidelity" item — from a security standpoint
it is a PoW-soundness gap and should be labelled as such.

### MED-3 — difficulty check is out-of-circuit and out-of-transcript — ✅ RESOLVED 2026-05-16

**Severity: Medium (conditional soundness). STATUS: RESOLVED
(`prove_and_verify_for_block` hardened entrypoint + derivation
contract; both preconditions now met).**

`composite_verify_pow*` checks `HASH_JACKPOT ≤ target` *after*
STARK verification, in plain Rust (Pearl-Layer-0-faithful:
difficulty is external by design — *not* a Fiat-Shamir weakness).
Soundness of the bound was conditional on two things the SNARK
does not enforce: **(i)** `HASH_JACKPOT` genuinely bound, and
**(ii)** the verifier independently derives the correct
chain-pinned `target`.

- **(i) is closed by CRIT-1** (RESOLVED `15ba9a3`; `HASH_JACKPOT`
  is a selector-gated bound PI on the verifier-fixed program).
- **(ii) is closed by the MED-3 hardening (`aa82ce3`-series):**
  the production entrypoint is now
  `ai_pow::zk_bridge::prove_and_verify_for_block(ctx, params)`,
  which **derives `target = difficulty_target(params)` itself**
  from the chain-pinned `MatmulParams` (a pure deterministic
  function of `noise_rank`/`tile`/`difficulty_bits`) and never
  accepts a counterparty-supplied target. `prover.rs` (the only
  production call site) uses it. The low-level
  `prove_and_verify(.., target)` / `composite_verify_pow*` remain
  the *unhardened primitives*, doc-commented with the MED-3
  obligation (retained for tests that deliberately inject a
  non-chain target).

**Verifier-side derivation contract (the §4.E consumable):**

```text
target            = difficulty_target(params)                 // pure fn of chain-pinned params
(tile_i, tile_j)  = (found_idx / col_tiles, found_idx % col_tiles)
valid iff           found_idx < params.num_tiles()
verifier checks     tile_i < params.row_tiles() ∧ tile_j < params.col_tiles()
```

`found_idx` is the miner's winning linear tile index into
`BlockContext::m_states` (`mine_with_context`). All of
`row_tiles()/col_tiles()/num_tiles()` are pure functions of the
chain-pinned `params`. Exposed as `ai_pow::zk_bridge::tile_ij`
(returns `None` out of range ⇒ verifier rejects). HIGH-2.2 §4.E
binds *this* verifier-recomputed value to the in-circuit matmul
accumulator (the §6(b) work) — it is **not** a free prover PI.
Tests: `med3_prove_and_verify_for_block_roundtrips_and_derives_target`,
`med3_tile_ij_derivation_and_bounds` (ai-pow `--features zk`,
lib 66/0; full e2e green incl. `end_to_end` 13/0 via the hardened
path).

### INFO-4 — `noised_packed` self-query binds nothing on the matrix path

The M52 step-4.1 BLAKE3-side `noised_packed` query is
self-referential (the row publishes and consumes its own table
entry; balances at `MAT_FREQ = 1`). It does **not** cross-bind
BLAKE3 reads to matmul reads. The actual matrix-byte binding is
C3 (`IS_MSG_MAT·IS_NEW_BLAKE·(BLAKE3_MSG − base256(UINT8_DATA))`),
which is itself selector-gated and therefore also subject to
CRIT-1. Not an independent vulnerability; recorded so the
`noised_packed` bus is not mistaken for a binding it does not
provide.

## Fiat-Shamir soundness

- **Transcript construction is delegated to upstream
  `p3-uni-stark` + `DuplexChallenger<Goldilocks, Tip5Perm>`.**
  Upstream observes the trace commitment and the public-input
  vector into the duplex sponge before drawing constraint /
  FRI-folding / query challenges, i.e. challenges depend on the
  committed witness and the PIs. No round of challenges is drawn
  before the data it must bind is absorbed. This is the standard,
  sound BCS/Fiat-Shamir transform for FRI-STARKs; no
  weak-Fiat-Shamir (challenge-before-commit) pattern was found in
  our usage.
- **Public inputs are in the transcript** (`pis` passed to
  `verify`), so the bound PIs cannot be swapped post-hoc — *if*
  they are constrained at all (CRIT-1 is about them being
  unconstrained, not about FS).
- **No preprocessed commitment.** Because `CompositeFullAir`
  declares no preprocessed trace, there is no
  verifying-key/program digest in the transcript. Fiat-Shamir is
  still sound *for the AIR as a relation*, but the "AIR" the
  prover is proving includes prover-chosen selector columns —
  the FS transform faithfully proves a statement that is too weak
  (this is the FS-level restatement of CRIT-1).
- **C2 `target` is outside the transcript** (MED-3) — a
  deliberate, documented externality, not an FS flaw; **RESOLVED**
  by the `prove_and_verify_for_block` hardened entrypoint
  (verifier re-derives `target` from chain-pinned params).

Conclusion: Fiat-Shamir is **sound as applied**; the soundness
problem is in *what statement* is being proven (CRIT-1), not in
the transform.

## Bits of security

| Component | Bound | Notes |
|---|---|---|
| FRI, provable (unique-decoding) | **120 bits** | `num_queries · log_blowup / 2 = 80 · 3 / 2`, `pow_bits = 0` (no grinding). PROD profile. |
| FRI, conjectured (list-decoding) | > 120 | Johnson-bound; not relied upon. |
| Challenge field | ~128 bits | `BinomialExtensionField<Goldilocks, 2>` ≈ `2^128`; per-challenge soundness ≥ FRI floor. |
| Tip5 sponge (FS + Merkle) | ~192 bits | capacity 6 × Goldilocks ≈ 384-bit state → ~192-bit collision/preimage, *assuming the 7-round variant is cryptographically adequate* (reduced-round Tip5 — not independently reviewed here; flagged). |
| BLAKE3 matrix/jackpot commitments | ~128 bits | 256-bit output, birthday collision 2^128 ≥ FRI floor. |
| **System soundness (primitives only)** | **≈ 120 bits** | min of the above; FRI is the floor. |
| **PoW soundness as wired** | **≈ 0 bits** | CRIT-1: forgeable with no work. The 120-bit number applies to "the prover knows *a* satisfying assignment of a too-weak AIR", which is not the PoW statement. |

So: cryptographically ~120-bit; **operationally broken** until
CRIT-1 is fixed. Quote the 120-bit figure only with the CRIT-1
caveat attached.

## Do the commitments degrade security?

**Numeric encoding: no degradation.** 256-bit BLAKE3 digests are
carried as 8 × u32 limbs, one per Goldilocks element. `u32 < 2^32
< p ≈ 2^64`, so each limb is injective into the field — no
modular aliasing, no truncation, full 256 bits preserved. The
binding `selector · (CV_OUT − PI) = 0` pins each PI limb to the
blake3 chip's `CV_OUT` cell, which the chip range-constrains to a
u32; an out-of-range PI limb cannot satisfy the binding (when the
selector fires). BLAKE3's ~128-bit collision resistance exceeds
the 120-bit FRI floor, so the commitment hash is not the
bottleneck.

**Structural: yes, but via CRIT-1, not the encoding.** The
degradation is not numeric — it is that the commitments are
attached to the proof through *prover-controlled, unenforced*
selectors. The encoding is fine; the *enforcement* is absent.
There is no additional collision/length-extension exposure from
the chunk-Merkle (`BLAKE3::new_keyed` over padded bytes is
length-prefixed and keyed; standard). The Merkle MMCS uses Tip5
(`PaddingFreeSponge` + `TruncatedPermutation`) — standard
Plonky3, no degradation beyond the reduced-round Tip5 caveat
above.

## Remediation (priority order)

1. **CRIT-1 — make the program verifier-fixed (blocks everything
   else).** Either:
   - **(preferred)** implement `BaseAir::preprocessed_trace()` on
     `CompositeFullAir` returning the `*_PREP` columns
     (`CONTROL_PREP`, selectors, `NOISE_PACKED_PREP`,
     `CV_OR_TWEAK_PREP`, `AB_ID_PREP`, `STARK_ROW_IDX`) generated
     by `composite_preprocess.rs`. p3-uni-stark then commits the
     preprocessed trace into the verifying key and the FS
     transcript; the prover can no longer choose selectors. This
     is the M10.1c-intended design (`composite_layout.rs:124`),
     simply not yet wired into the AIR trait impl; **or**
   - add unconditional AIR constraints that force each binding to
     be live: a boundary/running-sum argument asserting
     `Σ_rows IS_HASH_JACKPOT = 1` (ditto HASH_A/B,
     JOB_KEY/COMMITMENT_HASH) so the selector-gated equality must
     hold on exactly one row, plus a constraint tying that row's
     CV to the PI unconditionally. Heavier and easy to get
     subtly wrong; the preprocessed-trace route is cleaner and is
     how Pearl does it.
   Add a malicious-prover regression: a hand-built
   all-selectors-zero trace with a forged `HASH_JACKPOT` **must
   fail** `composite_verify`.
2. **HIGH-2 — feed the real tile-state fold into `JACKPOT_MSG`**
   (matmul→jackpot rotate-XOR-13 interleave) so `HASH_JACKPOT`
   commits to the actual work, not `BLAKE3(0, key=s_a)`.
3. **MED-3 — ✅ DONE 2026-05-16.** Resolved via the hardened
   `ai_pow::zk_bridge::prove_and_verify_for_block` entrypoint
   (re-derives `target = difficulty_target(params)` internally;
   the verifier never accepts a counterparty target) + the
   doc-commented MED-3 obligation on the unhardened
   `composite_verify_pow*` primitive + the `tile_ij` derivation
   contract §4.E consumes. `prover.rs` uses the hardened path; see
   the §MED-3 section above.
4. Independent review of the **7-round Tip5** variant's
   cryptographic margin (it underpins both Fiat-Shamir and the
   Merkle MMCS; a weakness there is systemic).
5. Update `GAP_AUDIT.md`: the "no remaining soundness/binding
   gap" summary is incorrect against a malicious prover until
   CRIT-1 lands; this report is the authority on that question.

## Bottom line

**Updated 2026-05-15: CRIT-1 RESOLVED.** The cryptography was
already solid (~120-bit provable FRI, sound Fiat-Shamir as
applied, lossless commitment encoding); the gap was circuit-level
enforcement (no verifier-fixed program ⇒ selector-gated C1/C3/C4
bindings vacatable ⇒ forge a winning proof with zero work). Fixed
by committing the program columns as a preprocessed trace
(`CompositeFullAirPinned`, commit `9ec529e`) — the single,
well-localized root cause with the clean fix anticipated here.
The production path now proves/verifies against a verifier-fixed
program; the `crit1_*` adversarial suite confirms the forgery is
rejected. **HIGH-2's soundness gap is closed** (`15ba9a3`).

**Updated 2026-05-16 — HIGH-2.2 fidelity largely closed.** The
keystone was generalised to the faithful `JACKPOT_MSG[0..16] ==
FOLD_STATE[0..16]` (the full Pearl §4.5 folded `TileState M`),
a `FoldChip` + `place_fold_chain` were added, and
`zk_bridge`/`mine()` now place the **real** solved tile's
matmul→fold chain through the **production Route-A batch-stark
path** (CRIT-1 pin + `noised_packed` LogUp unified). An honest
proof now attests `HASH_JACKPOT = BLAKE3(real folded M,
key=s_a)` — byte-equivalent to the plain miner, *not*
`BLAKE3(0,s_a)`. A pre-existing latent JackpotChip bug (the
`JACKPOT_MSG` RAM recurrence ungated by `is_active`, masked for
years by all-zero messages) was root-caused and fixed
(`354b47e`). Full `cargo test -p ai-pow --features zk` green
incl. `end_to_end` 13/0; no `crit1_*`/`high2_*`/`routea_*`
regression.

**Updated 2026-05-16 — §6(a) fold-schedule pin landed
(`aa82ce3`).** The fold/matmul schedule (`FOLD_IS_FOLD` + 4-bit
fold-slot) is now packed into the CRIT-1-pinned `CONTROL_PREP`
and asserted by `ControlChip`; `place_fold_chain` writes it and
`extract_program` lifts it — **which rows fold, into which slot,
is verifier-fixed**. Done by reusing the existing pinned column
(no preprocessed-width blow-up — the §4.C.8 trap is avoided; zero
blast radius for non-fold rows). +6 exhaustive ControlChip tests;
e2e-validated (ai-pow-zk lib 322/0; ai-pow `--features zk` green).

**Remaining (precisely scoped useful-work-soundness residual —
not a *proof*-forgery hole) — §6(b) + §4.E, one entangled
item:** the per-stripe `X_STEP` is honest-placed but not yet
*in-circuit* bound to the matmul accumulator (`XStepChip`
byte-equiv-proven standalone, not composite-wired; the matmul
subtile-sweep rows aren't placed on the honest path). Because
nothing in-circuit yet ties the digest to a *specific tile's*
committed-matrix accumulator, the attested `(tile_i,tile_j)`
(§4.E) cannot be soundly bound on its own — a free tile/`X_STEP`
PI would be vacuous — so §6(b) and §4.E are the same binding and
land together, consuming **MED-3**'s now-resolved verifier-side
derivation contract (`prove_and_verify_for_block` + `tile_ij`).
Held meanwhile by CRIT-1 + the keystone + §6(a)
(the proof can't be forged against the canonical program and the
fold schedule is verifier-fixed). Closing it = place the matmul
subtile-sweep rows + composite-wire `XStepChip` to force
`FOLD_XSTEP == ⊕CUMSUM_TILE` + bind `(tile_i,tile_j)` to that
accumulator per the MED-3 contract (not a wide preprocessed
block — `HIGH2_2_DESIGN.md` §4.C.8; §6(a) demonstrates the cheap
CONTROL_PREP-reuse pattern). **MED-3 is ✅ RESOLVED**
(`prove_and_verify_for_block`); the 7-round-Tip5 review still
remains. Net: CRIT-1 + HIGH-2
keystone + §6(a) make the SNARK PoW-sound, the fold schedule
verifier-fixed, and an attacker cannot forge a winning proof; the
*honest* proof attests the real, byte-equivalent useful-work
tile; the final residual is forcing a *malicious* prover through
the same matmul for `X_STEP`/the tile.
