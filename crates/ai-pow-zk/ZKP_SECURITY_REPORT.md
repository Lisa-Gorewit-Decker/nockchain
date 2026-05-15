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

**Severity: High (PoW *usefulness* not enforced even if CRIT-1 is
fixed).**

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

### MED-3 — difficulty check is out-of-circuit and out-of-transcript

**Severity: Medium (conditional soundness).**

`composite_verify_pow` checks `HASH_JACKPOT ≤ target` *after*
STARK verification, in plain Rust, with `target` a
verifier-supplied argument. `target` is **not** absorbed into the
Fiat-Shamir transcript and **not** an AIR public input. This is
Pearl-Layer-0-faithful (difficulty is external by design) and is
*not itself* a Fiat-Shamir weakness — but it makes C2 soundness
conditional on two things the SNARK does not enforce: (i)
`HASH_JACKPOT` is genuinely bound (broken by CRIT-1), and (ii)
the verifier independently derives the correct chain-pinned
`target` (a caller obligation; if a caller passes an attacker-
influenced `target`, difficulty is meaningless). Acceptable as a
design choice **only** once CRIT-1 is fixed and the
target-derivation obligation is documented at the call site
(today the only call site, `prover.rs`, uses
`difficulty_target(params)` — fine, but the public
`composite_verify_pow` API takes an arbitrary `&[u8;32]`).

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
  deliberate, documented externality, not an FS flaw.

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
3. **MED-3 — document the `target` derivation obligation** on the
   public `composite_verify_pow` API (verifier MUST derive
   `target` from chain-pinned params; never accept a
   counterparty-supplied target), or bind a `difficulty_bits`
   PI into the AIR and derive `target` from it in-verifier.
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
rejected. **Remaining open items: HIGH-2** (HASH_JACKPOT still
hashes an all-zero `JACKPOT_MSG` — the C4 binding is now sound
but attests a constant, not the matmul; the real tile-state fold
is the matmul→jackpot interleave), **MED-3** (document the
`target`-derivation obligation; now actionable since CRIT-1
landed), and the 7-round-Tip5 review. With CRIT-1 closed the
SNARK is PoW-sound *for the statement it proves*; HIGH-2 is what
makes that statement the *useful-work* statement.
