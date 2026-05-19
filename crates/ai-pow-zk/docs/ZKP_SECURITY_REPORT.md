> _Created **2026-05-15** · last updated **2026-05-19** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

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

**Phase A-CR — CRIT-1 reconstruction made first-class
(2026-05-18, commits `3671702`..`2a9a18d`).** The verifier's
canonical program was previously obtained by
`extract_program(&trace)` of a *reference honest trace* — sound
under the `crit1_*` adversarial discipline, but the production
bridge (`zk_bridge`) handed the prover-derived program straight
to verify (a latent "verifier re-runs the prover" coupling). It
is now a **witness-free, params-pure, independently-auditable
`ai_pow_zk::canonical::canonical_program(&ZkParams,
&BlockPublic, trace_len)`**: one params-pure row schedule
(`schedule_layout`/`RowClass`) → per-row `RowDescriptor` (CR.1
Pad · CR.2 KeyPin · CR.3 JackpotHash · CR.4 StripOpenA/B incl.
the §4.C.2 co-located 8 `NOISE_PACKED_PREP` pins via `noise_ref`
· CR.5 Sweep/Fold) → the existing packing — covering **all 12
PROGRAM_COLS, every row**. On the **production-faithful 16|r
co-location path** (Pearl §4.8 is *always* 16|r) the bridge now
verifies against `canonical_program` rebuilt by the verifier
from the trusted block public (`zk_params` + the C1-pinned
κ/s_a/s_b + the MED-3-attested tile), **never** the prover's
(CR.6). Soundness is KAT-anchored: the cross-crate §5 migration
KAT proves `canonical_program == extract_program(honest_trace)`
**bit-for-bit on every row × all 12 PROGRAM_COLS of the real
P16(16|r) trace**, so honest proofs still verify and any forged
PROGRAM_COL ≠ the params-pure canonical fails the in-AIR pin vs
the canonical VK (adversarial `cr6_verify_uses_canonical_not
_prover_program_rejects_forge`: a non-canonical PROGRAM_COL is
rejected — pre-CR.6 it verified). This **subsumes the §4.C.2
"b2" item** (the store-noise pin is now one part of the
first-class reconstruction). Non-16|r is the documented A3.2b
**test** geometry (data-dependent separate-store; out of the
params-pure scope) — it retains the prior extract-of-reference
+ `crit1_*`/`routea_*` discipline (already
strictly-stronger-than-pre-A3, not a forgery hole). Authoritative
design + staged plan + KATs: `CANONICAL_PROGRAM_DESIGN.md`.

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

**§6(b) — ✅ CLOSED for every single-Layer-0 params set (DONE
2026-05-16, `072d840`/`c63fbc1`/`69e420d`; **G1+G2** `010ccd3`
generalized it to the rectangular LLM-FFN `llm_shape` shapes).**
The per-stripe `X_STEP` fed to the `FoldChip` is now **in-circuit
forced** to equal the XOR of the real `t×t` committed-matrix
accumulator.
`CompositeTrace::place_useful_work_chain` places the sub-block-
major matmul sweep + a co-located `StripeXorChip` reduction; the
matmul chip forces `nxt.CUMSUM == compute_row(cur)`,
`StripeXorChip::eval_composite` binds `SX_IN == nxt.CUMSUM_TILE`,
the chip XOR-reduces to `SX_XR`, and the Pinned §6(b) keystone
forces `FOLD_XSTEP == SX_XR[stripe]`. **A malicious prover can no
longer fabricate `X_STEP` — it must do the real matmul.** Honest
bridge places it for the attested tile, byte-equivalent to the
plain miner. Validated end-to-end through the production Route-A
batch-stark path: `chips::stripe_xor` 8/0,
`high2_2_useful_work_chain_unit`, `high2_2_fold_chain_pinned_logup`
(full chain); ai-pow-zk lib 331/0; ai-pow `--features zk` green
(lib 70/0, `end_to_end` 13/0, byte-equivalence preserved).
**§4.E — ✅ DONE (`e7f59f7`):** the bridge attests the *actual
solved tile* via the MED-3 `tile_ij` contract
(`high2_2_attests_real_solved_tile`); all tiles share
`difficulty_target(params)` so the index is not a PoW-soundness
requirement.

**G1+G2 — ✅ DONE (`010ccd3`):** `StripeXorChip` `STATE_LEN =
STRIPE_MAX = 64`; `place_useful_work_chain` chunks the `r`-wide
dot into `⌈r/TILE_D⌉` micro-steps (G1); a 6-bit fold-stripe index
is pinned into `CONTROL_PREP` and the keystone binds via
`FOLD_STRIPE_SEL` (G2). The rectangular LLM-FFN `llm_shape`
(`k/r = 20`) now runs the full §6(b) binding (`llm_shape` 5/0 via
§6(b); `high2_2_g1g2_chunked_and_wide_stripes`). `sx_bound =
sweep_fits`.

**Remaining (scoped; NOT a *proof*-forgery hole).** (1) **true
PROD** (`k/r = 64`, chunked sweep ≈ 2²⁰ ≫ one Layer-0): legacy
path, §6(b) keystone gated **off** via `sx_bound` — a value the
*verifier* derives from trusted params/height, never the proof
(as sound as CRIT-1). Closing it = **the Pearl-faithful
P-A/P-B/P-C path** (Pearl §4.8 param caps so one tile = one
STARK + raise the Layer-0 ceiling + vertical-recursion cert;
maintainer γ decision 2026-05-17 — `M_S2_PEARL_EVALUATION.md`).
*[2026-05-19, recursion-milestone scope — NOT the
MAT_UNPACK-binding "C3": the recursion milestone **C3/M-S5**
("vertical-recursion cert") was **re-scoped** to the
soundness-correct **≥120-bit** cert (LANDED + independently
re-validated; honest real sizes L1≈2.69 MB / L2≈1.79 MB; every
chain link ≥120-bit; all 5 inner sweep profiles accept +
tamper-reject; fenced linchpin byte-identical). The **≤65 KB**
size target is **carved out → deferred M-S5b** (a substrate
addition not in the current `Plonky3-recursion`; §14 proved
≤65 KB unreachable at any real ≥120-bit tier — only at the
~5-bit testing tier, a soundness trade the maintainer rejected).
Earlier "≤65 KB" wording is the deferred M-S5b target, not a C3
acceptance gate. See `C3_OUTER_CERT_DESIGN.md` §13.2/§14/§15,
`PRODUCTION_ROADMAP.md` C3/M-S5b.]*
*[Corrected: previously "G3 (segmentation + M12)"; Pearl caps
params and never segments, so G3 carry-segmentation is
**deferred** to the beyond-Pearl-envelope case only. No
production spot-check exists — `MatmulProof.spot` is test-only.]*
(2) deep tile↔committed-store: **M-S1 ✅ RESOLVED
2026-05-17** — the §4.C `noised_packed` query is now
whole-micro-tile (chunked over all `A_NOISED_LEN`/`B_NOISED_LEN`
cells) and the swept `A_NOISED`/`B_NOISED` are multiset-bound
(LogUp) to a declared producer store; adversarial **I2**
(`high2_2_swept_tile_not_in_store_rejects`) rejects a swept tile
∉ store, so a matrix-swap *on the sweep* is impossible. The
remaining tie was **§4.C.2** (store ↔ committed `HASH_A` via
noise derivation).

**§4.C.2 ✅ RESOLVED 2026-05-18 — ZERO-GAP on the
production-faithful 16|r path (c-exact).** A3.0–A3.2b closed the
*noise* tie (store `NOISE_UNPACK` forced to `noise_ref` of the
C1-public seed via InputChip + the CRIT-1 `NOISE_PACKED_PREP`
pin). The *plain* tie is now closed by **c-exact**: cx.1
(generalized C3 + CRIT-1 word-pair pin) + cx.2 (the X1
co-location flip — the strip-opening leaf round-0 rows are the
M-S1 `noised_packed` producers; the whole-block C3 binds their
`UINT8_DATA[0..64]` to `BLAKE3_MSG` ∈ `HASH_A`). End-to-end +
**position-exact adversarially** validated on a real 16|r `P16`
bridge trace: the honest roundtrip proves + pow-verifies at real
difficulty with C3 ACTIVE
(`sec_4c2_cx2_g1_p16_route_a_c3_active_roundtrip`), and tampering
a co-located leaf row's committed-plain byte is **rejected**
(`sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects`) — a
prover cannot swap the committed plain a producer's `a′` derives
from. Net (16|r): committed A/B ∈ `HASH_A` (position-exact);
swept `a′` = `noise(committed)`; noise = `noise_ref(public
s_a)`. **Pearl §4.8 always has 16|r**, so the production path is
zero-gap. Non-16|r *test* geometry (e.g. `TEST_SMALL`, r=4)
remains the A3.2b separate-store path — strictly stronger than
pre-A3, *not* a forgery hole (co-location only honest-balances
16|r; the documented residual is test-only). Validated:
`ai-pow-zk --lib` 352/0/22 (g=0 path, `crit1_*`/`routea_*`),
`ai-pow --features zk` 89/0/1 (P16 g=1 roundtrip + the
position-exact adversarial + non-16|r A3.2b + the §4.C.2 KAT
family), debug-assertions-ON P16 g=1 per-row clean (M-S1
hazard closed for the honest g=1 path). Detail:
`SEC_4C2_NOISE_BINDING_DESIGN.md` §8. Soundness throughout held
by CRIT-1 + keystone + §6(a) + §6(b) + M-S1 + A2 + the A3.2b
noise pin. The original analysis below stands as the historical
rationale.

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

**Updated 2026-05-16 — §6(b) CLOSED for the primary mining
geometry + §4.E DONE (`072d840`/`c63fbc1`/`69e420d`/`e7f59f7`).**
`X_STEP` is now in-circuit forced to `⊕` the real `t×t`
committed-matrix accumulator: `place_useful_work_chain` (matmul
sub-block-major sweep + co-located `StripeXorChip`) + the
`SX_IN == nxt.CUMSUM_TILE` binding + the Pinned
`FOLD_XSTEP == SX_XR[stripe]` keystone — **a malicious prover
must do the real matmul** for **every single-Layer-0 params set**
(TEST_SMALL *and*, via **G1+G2** `010ccd3`, the rectangular
LLM-FFN `llm_shape` shapes). The bridge attests the *actual
solved tile* via MED-3 `tile_ij`. End-to-end green (ai-pow-zk lib
332/0; ai-pow `--features zk` lib 71/0, `end_to_end` 13/0,
**`llm_shape` 5/0 via §6(b)**; byte-equivalence preserved). MED-3
also ✅ RESOLVED (`prove_and_verify_for_block`).

**Remaining (scoped; NOT a *proof*-forgery hole).** (1) **true
PROD** (`k/r = 64`, chunked sweep ≈ 2²⁰ ≫ one Layer-0): legacy
path, §6(b) keystone gated off via the verifier-set `sx_bound`
(sound as CRIT-1); closing it = **the Pearl-faithful
P-A/P-B/P-C path** (param caps + raised Layer-0 ceiling +
vertical-recursion cert; γ decision 2026-05-17,
`M_S2_PEARL_EVALUATION.md`). *[Corrected: was "G3 (segmentation
+ M12)"; Pearl never segments — G3 deferred to beyond-Pearl-`k`.
`MatmulProof.spot` is test-only, not a PROD fallback.]* (2) deep
tile↔committed-store:
**M-S1 ✅ RESOLVED 2026-05-17** (§4.C `noised_packed`
whole-micro-tile non-vacuity — sweep A/B multiset-bound to a
declared store, adversarial I2 rejects swap-on-sweep);
**§4.C.2 ✅ RESOLVED 2026-05-18 — ZERO-GAP on the 16|r
production path** (c-exact: A3.0–A3.2b noise tie + cx.1/cx.2
plain tie; the co-located strip-opening leaf rows are the M-S1
producers, whole-block C3 binds their committed plain ∈
`HASH_A`, end-to-end + position-exact adversarially validated on
real `P16`; Pearl §4.8 is always 16|r; non-16|r test geometry =
the documented A3.2b strictly-stronger-than-pre-A3 state, not a
forgery hole). Plus
the 7-round-Tip5 review. Net: CRIT-1 + HIGH-2 keystone + §6(a) +
§6(b) + M-S1 make the SNARK PoW-sound, the fold schedule
verifier-fixed,
and — for every single-Layer-0 params set — a *malicious* prover
is now forced through the real matmul for `X_STEP`; the *honest*
proof attests the real, byte-equivalent solved tile.
