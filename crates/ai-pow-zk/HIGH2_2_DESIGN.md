# HIGH-2.2 — Honest matmul→fold→C4-hash chain: problem space & design

> **Status:** IN PROGRESS. Implementation tracked as tasks
> #97–#103.
> **Class:** completeness / fidelity. **Not** a soundness or
> forgery hole — that part of HIGH-2 (the "C4 hashes a
> prover-free constant") was closed by the keystone, commit
> `15ba9a3`.
> **Authoritative neighbors:** `ZKP_SECURITY_REPORT.md` (HIGH-2
> section + Bottom line), `GAP_AUDIT.md` (post-CRIT-1 + keystone
> summary), memory `ai_pow_zk_crypto_gaps`.

### Progress (2026-05-15)

| Step | State | Commit |
|---|---|---|
| §4.A reference — `compute_tile_trace` / `TileTrace` / `TileState::from_x_steps` (per-stripe `x` sequence; `M` proven a pure function of it) | ✅ done, 11/0 tests | `08485ea` |
| §4.0 geometry finding — chip is a 2×2×16 micro-tile, *not* the `t×t` accumulator; FoldChip binds a per-stripe scalar `X_STEP` | ✅ recorded | `08485ea` |
| §4.B FoldChip — standalone AIR, Pearl §4.5 rotl13-XOR, Option B2; 9/0 self-contained tests (correctness + 5 adversarial) | ✅ done | `8cbbbeb` |
| §4.B↔plain byte-equivalence — FoldChip reproduces the real folded `TileState M` for every tile of a genuine `BlockContext` solve; keyed-hash of chip output == plain PoW digest (the `high2_2_byte_equiv_plain` half of §7) | ✅ done | `2964c32` |
| §4.A FoldChip composite wiring + `place_fold_chain` — FOLD_* layout block, `FoldChip::eval_composite` in `CompositeFullAir`, `CompositeTrace::place_fold_chain` | ✅ structurally landed (`e6c9c84`); unit-correct (`high2_2_fold_chain_in_composite_unit` ✓) | `e6c9c84` |
| §4.A blocking bug — pre-existing JackpotChip `JACKPOT_MSG` RAM recurrence ungated by `is_active` (forbade non-zero `JACKPOT_MSG`; latent ∵ all tests used zero msg) | ✅ **FIXED** — gate recurrence by `is_active` (`chips/jackpot/chip.rs`); validated incl. `high2_2_fold_chain_pinned_logup` (the §4.A bridge shape via Route-A), no `crit1_*`/`high2_*`/`routea_*` regression. See §4.A "FIXED". |
| §4.A bridge real-fold-chain (zk_bridge ⇒ `JACKPOT_MSG`=real folded `M`) | ✅ **DONE & e2e-validated** (`37f5c0f`): full `cargo test -p ai-pow --features zk` `CARGO_EXIT=0`, `end_to_end` 13/0 (every `mine()` via real-M Route-A), `zk_bridge` 19/0 (`f1_bridge_real_solve` with non-zero `M`), lib 64/0 |
| §4.D keystone — generalised to `JACKPOT_MSG[0..16] == FOLD_STATE` (last row) | ✅ landed; gate-green with zero-fold (`composite_proof::tests:: 18/0`) | `e6c9c84` |
| §4.C.4 accumulator→`X_STEP` reduction (`XStepChip`) + composed `XStep→Fold` pipeline byte-equivalent to plain | ✅ done, 6/0 + zk_bridge 5/5 | `290af68`, `c78ae67` |
| §4.C committed-matrix *binding* — **Route A: chosen, spiked, productionised & WIRED** (§4.C.10): production API `composite_*_pinned_logup` + exhaustive `routea_*` 4/4; `zk_bridge`(mine() gate)+`f1_harness` switched to it; spike removed; 3-tier entrypoint doc. ~1.23x cost. | ✅ binding complete & wired; §4.A non-vacuity is the separate remaining workstream (#97) |
| §6(a) CRIT-1 program extends to the **fold schedule** — `FOLD_IS_FOLD` + 4-bit slot packed into the pinned `CONTROL_PREP` polyval (NOT a wide preprocessed block; §4.C.8 trap avoided) | ✅ **done & e2e-validated** (`aa82ce3`): ControlChip +6 tests (positive + 4 adversarial + zero-blast-radius); `place_fold_chain` writes it, `extract_program` lifts it; ai-pow-zk lib 322/0 incl. `high2_2_fold_chain_pinned_logup`/`routea_*`/`crit1_*`; ai-pow `--features zk` green (lib 64/0, `end_to_end` 13/0) |
| §6(b) + §4.E — **entangled** residual: bind `X_STEP ← ⊕CUMSUM_TILE ← committed A/B` *and* the attested `(tile_i,tile_j)` to the same in-circuit matmul accumulator; reconcile tile derivation with MED-3 | ⬜ remaining — the single cryptographic-core item (see "Remaining soundness scope"). Cannot be soundly closed independently of placing the matmul subtile-sweep rows (§4.C.4); a free tile/X_STEP PI without that tie is vacuous. Meanwhile soundness held by CRIT-1 + keystone + §6(a). |
| §7 real-difficulty end-to-end + byte-equivalence + docs flip | 🟡 byte-equivalence ✅ (`high2_2_xstep_fold_pipeline_byte_equiv_plain`); real-M e2e ✅ (`end_to_end` 13/0); docs flip ⬜ |

### Current state (2026-05-16)

**HIGH-2.2's headline goal is achieved & validated:** the
*honest* prover now attests the **real folded `TileState M`**
end-to-end through the production proving path — `zk_bridge`
(`mine()` gate) places the real solved tile's matmul→fold chain
via `place_fold_chain`, the §4.D keystone binds last-row
`JACKPOT_MSG == FOLD_STATE == M`, C4 hashes it
(`HASH_JACKPOT = BLAKE3(M, key=s_a)`, byte-equivalent to the
plain miner), C2 checks difficulty. Full `cargo test -p ai-pow
--features zk` green. The pre-existing latent JackpotChip bug
that blocked any non-zero `JACKPOT_MSG` is fixed.

**§6(a) — fold schedule pinned (DONE, `aa82ce3`).** The
fold/matmul *schedule* `FOLD_IS_FOLD` + the 4-bit fold-slot index
(`stripe % 16`) are now packed into the CRIT-1-pinned
`CONTROL_PREP` polyval (bits `2^47` / `2^48`, immediately past the
21 selectors + 26-bit `MAT_ID`) and `ControlChip` asserts the
extended pack. `CONTROL_PREP` is a `PROGRAM_COL`, so
`CompositeFullAirPinned` + the verifier-rebuilt canonical program
now make **which rows fold and into which slot verifier-fixed** —
a malicious prover can no longer fabricate a fold schedule. Done
*without* widening the preprocessed trace (the §4.C.8 ~10x trap is
avoided: it reuses the existing pinned column; `is_fold=0/slot=0`
contributes exactly 0 so every non-fold row's `CONTROL_PREP` is
byte-identical to before — zero blast radius). Exhaustively tested
(ControlChip +6: positive + slot-mismatch / stale-`CONTROL_PREP` /
claimed-but-absent-fold rejects + bit-layout + zero-blast-radius)
and e2e-validated (ai-pow-zk lib 322/0; ai-pow `--features zk`
green).

**Remaining soundness scope — §6(b) + §4.E, one *entangled*
residual.** What is still *not* in-circuit bound: the per-stripe
`X_STEP` fed to the FoldChip is *placed by the honest bridge* but
not forced to equal `⊕CUMSUM_TILE`, and the matmul subtile-sweep
rows that would make `CUMSUM_TILE` the real `t×t` accumulator of
the committed `A/B` for the attested tile are **not placed on the
honest path** (`zk_bridge` places none; `place_fold_chain` takes
prover-supplied `x_steps`). Consequently the attested
`(tile_i,tile_j)` (§4.E) cannot be *soundly* bound independently:
nothing in-circuit yet ties the hashed digest to a *specific
tile's committed-matrix accumulator*, so adding a free
`tile_i/tile_j` public input would be **vacuous security theatre**
(a malicious prover supplying fabricated `x_steps` can set any
tile PI). §6(b) (`X_STEP ← ⊕CUMSUM_TILE ← committed A/B`, via the
§4.C.4 subtile sweep + XOR tree) and §4.E (tile↔accumulator) are
therefore **the same cryptographic-core binding** and must land
together. Closing it = place the matmul subtile-sweep rows on the
honest path, composite-wire `XStepChip` to force
`FOLD_XSTEP == ⊕CUMSUM_TILE` per stripe (Route-A `noised_packed`
already binds `CUMSUM_TILE`'s inputs to the committed matrices),
and bind `(tile_i,tile_j)` to *that* accumulator's
row/col offsets — reconciled with **MED-3**'s block-context tile
derivation (MED-3 documents how the verifier obtains the claimed
`(tile_i,tile_j)`; HIGH-2.2 must consume that, not invent a free
PI). This is the documented multi-day item (§4.C.4 — "the
dominant new width"); it is invasive (composite-layout +
matmul-row placement + CRIT-1 program) and is its own focused
effort, not rushed. Soundness **meanwhile held by CRIT-1 + the
keystone + §6(a)** (the *proof* still can't be forged against the
canonical program, and the fold schedule is now verifier-fixed;
the open gap is only that the attacker isn't yet *forced* to do
the real matmul for `X_STEP`/the tile).

**Precise residual boundary.** The fold *math* is done and
proven (FoldChip ≡ `from_x_steps` ≡ Pearl §4.5). What remains is
**composite-AIR integration** (allocating `FOLD_STATE`/`X_STEP`
columns without widening `TOTAL_TRACE_WIDTH`, wiring `FoldChip`
into `CompositeFullAir`/`Pinned`, extending the CRIT-1
preprocessed program) plus the **cryptographic core §4.C**:
binding the placed `X_STEP` sequence to the committed `A/B` bytes
in-circuit (committed bytes → noised matrices → `t×t`
accumulator → per-stripe XOR). §4.C is the documented multi-day
item; until it lands, an honest prover can place the *real* `M`
(so it can clear real difficulty) but a malicious prover is not
yet *forced* to — that gap is still covered for soundness by the
existing keystone + CRIT-1 (the proof still can't be forged; it
just isn't yet the *useful-work* statement end-to-end).

---

## 1. TL;DR

The keystone forces, on the last trace row of the production
(`CompositeFullAirPinned`) path:

```
JACKPOT_MSG[0..4]  == CUMSUM_TILE[0..4]      (4 = TILE_H² accumulator cells)
JACKPOT_MSG[4..16] == 0
```

so the BLAKE3(·, key=`s_a`) input that C4 binds and C2 checks
against difficulty is no longer a value the prover may choose
freely. That removed the forgery. What it does **not** yet give
us:

1. **No matmul rows on the honest path.** The honest bridge
   (`ai-pow/src/zk_bridge.rs`, `ai-pow/examples/f1_harness.rs`)
   places matrix-hash A/B (C3) + key-pin rows (C1) + the
   jackpot-hash block (C4). It places **no `MatmulCumsumChip`
   step rows**, so `CUMSUM_TILE` stays `0`, and the keystone is
   satisfied trivially (`0 == 0`). The honest proof attests
   `BLAKE3(0, s_a)`.
2. **The bound quantity is the raw accumulator, not the folded
   tile state.** Even with matmul rows, `MatmulCumsumChip` only
   enforces the per-step reset/update accumulator
   `C += Aʹ·Bʹ` (`chips/matmul/compute.rs::apply_cumsum_update`).
   The value the *plain* miner actually hashes is the
   **rotate-left-13 XOR fold** `TileState M` (Pearl §4.5,
   `ai-pow/src/matmul.rs::TileState::fold` →
   `keyed_hash`), not `C`. There is no fold chip; the circuit
   never computes `M`.
3. **`CUMSUM` is not yet provably the product of the *committed*
   matrices.** `A_NOISED_UNPACK` / `B_NOISED_UNPACK` feed the
   matmul chip, but the chain that pins them to the
   C3-committed, CRIT-1-fixed matrix bytes (`HASH_A`/`HASH_B`
   via the `noised_packed` LogUp bus) is not wired through to
   the accumulator.

HIGH-2.2 is the work that makes an **honest** prover able to
produce a proof whose `HASH_JACKPOT` is `BLAKE3(real folded tile
state M, key=s_a)` for a tile derived from the **committed**
matrices — i.e. the digest the plain PoW search actually cleared
— and therefore able to win at a **real** difficulty target.

---

## 2. Why this is fidelity, not soundness

Split by difficulty regime (this is the precise answer to "can
the honest prover produce a winning proof today?"):

| Regime | Honest prover today | Why |
|---|---|---|
| Test (`TEST_SMALL`, `difficulty_bits = 0`) | **Yes — produces a passing proof** | `CUMSUM=0`, `JACKPOT_MSG=0`, keystone is `0==0`, `BLAKE3(0,s_a)` clears a bits=0 target. 13 `end_to_end` tests confirm green. |
| Real / production difficulty | **No** | `HASH_JACKPOT = BLAKE3(0, s_a)` is a *single fixed value per block* with no search variable inside the proven statement. It is decoupled from the real winning tile `mine()` found. That one digest essentially never clears a real target. |

Crucially, the keystone **did not cause or worsen** the
real-difficulty limitation. Pre-keystone the honest bridge set a
free `JACKPOT_MSG = 0`; post-keystone it is pinned to
`CUMSUM_TILE`, also `0` on the honest path — identical honest
`HASH_JACKPOT`. The keystone removed only the *attacker's*
freedom. So:

- **Soundness (attacker cannot forge a winning proof):** met by
  CRIT-1 (program-pinned selectors) + the keystone.
- **Completeness/fidelity (the honest proof attests the *real*
  useful work and can win at real difficulty):** the subject of
  HIGH-2.2.

Production (PROD/M12) is gated and not shipping, so this is not a
live blocker — but until HIGH-2.2 lands the SNARK is not a
*useful-work* proof at real difficulty.

---

## 3. Reference semantics the circuit must reproduce

These are the ground-truth definitions HIGH-2.2 must match
bit-for-bit. Sources are authoritative; do not paraphrase them in
code, port them.

### 3.1 Per-step accumulator (already in-circuit)

`ai-pow/src/matmul.rs::compute_tile`, per stripe `step` of width
`r = noise_rank` along the `k`-axis:

```
for di in 0..t, dj in 0..t:
    delta = Σ_{l<r} Aʹ[row0+di][lo+l] * Bʹ[col0+dj][lo+l]   // i32, wrapping
    c_blk[di*t + dj] += delta                                // i32, wrapping
```

In-circuit equivalent: `MatmulCumsumChip` /
`chips/matmul/compute.rs::apply_cumsum_update`
(`reset` ⇒ `c = dot`, `update` ⇒ `c += dot`, else pass-through),
over `CUMSUM_TILE` at `CUMSUM_TILE_START` (`CUMSUM_TILE_LEN =
TILE_H² = 4`). **This part exists and is constrained.**

### 3.2 The fold (NOT in-circuit — the core missing piece)

After each stripe's accumulator update, Pearl §4.5
(`ai-pow/src/matmul.rs::TileState::fold`):

```
x = 0i32
for v in c_blk:  x ^= v                       // int32-XOR of ALL accumulator cells
slot = step % 16
M[slot] = (M[slot] as u32).rotate_left(13) as i32  ^  x
```

`M` is `TileState([i32;16])`, zero-initialised. After the last
stripe, the PoW digest is the **keyed** BLAKE3 of `M`
(`TileState::keyed_hash`):

```
hasher = blake3::Hasher::new_keyed(pow_key)   // pow_key = s_a (32 bytes)
for v in M:  hasher.update(&v.to_le_bytes())  // 16 × u32 LE = 64 bytes
digest = hasher.finalize()                    // == HASH_JACKPOT
```

Note `JACKPOT_SIZE = JACKPOT_MSG_LEN = 16` already equals
`TileState` width: the jackpot message *is* meant to be `M`. The
keystone's current `[0..4]==CUMSUM, [4..16]==0` shape is the
zero-padded stand-in for "`JACKPOT_MSG == M`".

---

## 4. The three components of HIGH-2.2

### 4.0 Geometry constraint (discovered during §4.A — supersedes the naïve sketch)

The earlier draft of §4.A/§4.B assumed `CUMSUM_TILE` *is* the
accumulator the fold XORs. **It is not.** The in-circuit
`MatmulCumsumChip` is a fixed Pearl-faithful **2×2 micro-tile of
depth `TILE_D=16`** (`CUMSUM_TILE_LEN = TILE_H² = 4`,
`A/B_NOISED_UNPACK_LEN = TILE_H·TILE_D = 32`; chip `dot[i·2+j] =
Σ_{d<16} A[i·16+d]·B[j·16+d]`). The puzzle's `compute_tile` is a
**`t×t` tile (`t = params.tile`, 8 for `TEST_SMALL`) accumulated
over `num_stripes = k/r` stripes of width `r`** (16 stripes,
`r=4` for `TEST_SMALL`). The value the fold consumes per stripe
is

```
x_step = ⊕  over all  t·t  cells of the running accumulator c_blk
```

So `CUMSUM_TILE` (4 cells) ≠ `c_blk` (`t·t` = 64 cells), and the
current keystone's `JACKPOT_MSG[0..4]==CUMSUM_TILE[0..4]` pins a
2×2 micro-tile that is **not** the fold input. B2 removes Pearl's
rotate-on-load RAM but does *not* remove the need to reduce a
`t×t` accumulator across `num_stripes` into the 16-word `M`.

**Decision (B2-consistent, recorded):** the FoldChip binds a
**per-stripe scalar `X_STEP`** (the i32-XOR of the entire stripe
accumulator), *not* `CUMSUM_TILE`. Folding is then exactly
`M[step%16] = rotl13(M[step%16]) ⊕ X_STEP[step]`. The
accumulator→`X_STEP` reduction and its binding to the *committed*
matrices is §4.C's LogUp obligation; the FoldChip itself is a
pure function of the `X_STEP` sequence. This is justified by the
new in-crate reference (landed §4.A):
`ai-pow::matmul::{compute_tile_trace, TileTrace,
TileState::from_x_steps}` — proven (tests
`from_x_steps_is_pure_function_of_sequence`,
`from_x_steps_matches_manual_rotl13_fold`) that
`M = TileState::from_x_steps(&x_steps)` depends on the sequence
**alone**, nothing else. The circuit therefore only has to bind
`x_steps`; it never re-derives the accumulator geometry.

Consequence: the rigid 2×2 `MatmulCumsumChip` is *not* reused
verbatim for the bridge. `place_matmul_tile` (§4.A) and the
`X_STEP` derivation (§4.C) replace it; the genuinely hard part of
HIGH-2.2 is now precisely §4.C (committed-bytes → accumulator →
`X_STEP`), not the fold.

### 4.A — STATUS 2026-05-16: structurally landed, one localized bug

**Landed (`e6c9c84`):** `composite_layout` FOLD_* block
(appended; no existing offset shifts), `FoldChip::eval_composite`
wired into `CompositeFullAir::eval`, §4.D keystone generalised to
last-row `JACKPOT_MSG[0..16] == FOLD_STATE[0..16]`,
`CompositeTrace::place_fold_chain(row_start, x_steps) -> [u32;16]`.

**Validated:**
- `composite_proof::tests::high2_2_fold_chain_in_composite_unit`
  ✓ — baseline + a real fold chain satisfies the **unit**
  `CompositeFullAir` (FoldChip-in-composite + `place_fold_chain`
  are constraint-correct).
- Structural gate `composite_proof::tests:: 18/0` (crit1_* /
  high2_* / routea_* + roundtrip) green with the +98-col trace,
  FoldChip wired, and the new keystone — i.e. **zero-fold**
  traces and the keystone-with-FOLD_STATE are sound.

**Known bug (precisely localized; bridge reverted to zero-jackpot
so production `mine()` stays green):** the *same* baseline +
real-fold-chain trace, taken through the **production Route-A
batch-stark path** (`composite_prove_pinned_logup` /
`composite_verify_pinned_logup`), fails
`OodEvaluationMismatch { index: Some(0) }` —
`high2_2_fold_chain_pinned_logup` (`#[ignore]`d, kept as the
repro). The §4.D keystone *data* precondition is **verified
holding** in that repro (asserted: last-row
`JACKPOT_MSG == FOLD_STATE == M`, and `place_jackpot_hash_block`
writes `JACKPOT_MSG = M`). So it is **not** a keystone
data/PI mismatch nor a FoldChip per-row constraint violation
(unit path passes; debug `check_constraints` would panic
per-row otherwise). It is a polynomial-identity failure
introduced **only** by the batch-stark pinned+LogUp prover when
FOLD columns are non-zero — i.e. a FoldChip↔batch-stark
boundary / quotient-degree / ZK-padding / permutation-trace
interaction. `routea_honest_roundtrip` (same prover, **zero**
FOLD) passes; the unit uni-stark prover with non-zero FOLD
passes; only their *intersection* fails.

## §4.A — ✅ FIXED (2026-05-16)

**Root cause (latent, pre-existing — NOT a HIGH-2.2 design
flaw):** `chips::jackpot::chip`'s `JACKPOT_MSG` RAM recurrence
`nxt[i] = SLOT_SEL[i]·rotl13_xor + (1−SLOT_SEL[i])·cur[i]` was
emitted under `when_transition()` but **not gated by
`is_active`**. Since an inactive row has `SLOT_SEL ≡ 0` (by
`Σ SLOT_SEL == is_active`), the ungated recurrence collapsed to
`nxt.JACKPOT_MSG == cur.JACKPOT_MSG` on every inactive→·
transition — pinning `JACKPOT_MSG` constant across all inactive
rows and so **forbidding the inactive→active(finalize) boundary
from carrying a freshly-placed non-zero `JACKPOT_MSG`**
(`cur` inactive ⇒ `cur.JACKPOT_MSG = 0` ⇒ requires
`nxt.JACKPOT_MSG = 0`). Latent for years: every jackpot
placement (`zk_bridge`, `routea_*`, `crit1_*`, `high2_*`,
`f1_harness`, `bench_suite`) hashed an **all-zero**
`JACKPOT_MSG` (`0 == 0`); HIGH-2.2 §4.A is the first path with a
non-zero `JACKPOT_MSG` (the real folded `M`).

It surfaced as `OodEvaluationMismatch` (not a `check_constraints`
panic) because the `ai-pow-zk` test profile builds with
`debug-assertions = false`, so `p3-uni-stark`'s
`#[cfg(debug_assertions)] p3_air::check_constraints` is compiled
out — a per-row `when_transition` violation silently yields a
bad proof rejected at `verify`. (This also explains why
log_blowup was irrelevant and why ~12 value-level hypotheses
mis-fired; the bug was a plain per-row constraint violation all
along, just invisible without the debug per-row check.)

**Fix:** gate the recurrence by `is_active` —
`tb.assert_zero(is_active · (nxt_msg − rhs))` (`chips/jackpot/
chip.rs`). Matches Pearl (whose RAM persistence is
store/active-gated) and leaves real multi-row jackpot sequences
(consecutive active rows) fully constrained.

**Validated:** `cargo test -p ai-pow-zk --lib
composite_proof::tests:: --include-ignored` → all genuine
reproducers pass, **including `high2_2_fold_chain_pinned_logup`
(the exact §4.A bridge trace via Route-A batch-stark)** and the
minimal `high2_2_jackpot_nonzero_msg_unit`; `crit1_*` /
`high2_*` / `routea_*` all green (no regression). The only
non-passing entries were two deliberately-inconsistent isolation
controls (now removed in cleanup). The bisection scaffolding was
deleted; two permanent regression tests remain.

<details><summary>Superseded mid-bisection analysis (kept for
provenance)</summary>

**DEFINITIVE ROOT CAUSE (2026-05-16, fully bisected — all
earlier analysis in this section is superseded):**

The §4.A `OodEvaluationMismatch` is **not** a HIGH-2.2 design
flaw and **not** in the fold path. It is a **pre-existing
latent bug: `CompositeTrace::place_jackpot_hash_block` (→ the
BLAKE3 keyed-hash chip) fails composite verify whenever
`JACKPOT_MSG` is non-zero.** Minimal reproducer
`composite_proof::tests::high2_2_jackpot_nonzero_msg_unit`
(`#[ignore]`): `baseline_min` + `place_jackpot_hash_block(h-8,
&non_zero_msg, &ch)` — **no fold chain, no keystone, no CRIT-1
pin, no batch-stark, unit `composite_prove`** — fails
`OodEvaluationMismatch { index: None }`.

Bisection chain (all `#[ignore]`d reproducers kept):
| Trace | Result |
|---|---|
| baseline + fold-only (`high2_2_fold_chain_in_composite_unit`) | ✅ pass |
| baseline + jackpot, **zero** msg (`high2_2_jackpot_only_unit`) | ✅ pass |
| baseline + jackpot, **non-zero** msg (`*_nonzero_msg_unit`) | ❌ **FAIL** |
| ditto at log_blowup=4 (`*_nonzero_msg_lb4`) | ❌ FAIL (⇒ not degree/blowup) |
| baseline + fold + jackpot non-zero (`*_fold_chain_jackpot_unit`) | ❌ FAIL (same cause) |

Why latent: **every shipping/test jackpot placement uses
`&[0u32;16]`** (`zk_bridge`, `routea_*`, `crit1_*`, `high2_*`,
`f1_harness` all hashed an all-zero `JACKPOT_MSG`). A zero
message makes the BLAKE3 message-injection terms vanish, masking
the bug. HIGH-2.2 §4.A is simply the first code path that needs
a *non-zero* `JACKPOT_MSG` (the real folded `M`), so it surfaced
a dormant defect in the BLAKE3-keyed-hash trace-gen/chip.

Signature: per-row `p3_air::check_constraints` (run by
`prove` under `cargo test`'s debug-assertions) **passes** — the
trace satisfies every constraint at every row — but the
polynomial `verify` fails `OodEvaluationMismatch`, at both
log_blowup 2 and 4. That points at a constraint that holds at
all trace rows but whose committed quotient disagrees with the
verifier's out-of-domain evaluation only when the message words
are non-zero: i.e. a discrepancy between what
`place_blake3_hash_with_selectors` writes into the round-state /
message columns and what the BLAKE3 round AIR's polynomial
actually constrains, in the message-injection / per-round
message-permutation that zero messages zero out.

**FINAL BOUND (2026-05-16, after 9 systematic controls):** the
trigger is **the `IS_HASH_JACKPOT` finalize selector (CONTROL_PREP
selector idx 6) combined with a non-zero BLAKE3 message**, at the
polynomial level only (per-row `check_constraints` passes;
`OodEvaluationMismatch` from `verify`, blowup-independent):

| Control | Trace | Result |
|---|---|---|
| #7 `high2_2_blake3_plain_nonzero_unit` | `place_blake3_hash`, non-zero msg, **no sel-6** | ✅ PASS |
| #8 `high2_2_blake3_sel6_no_jackpotstep_unit` | + `IS_HASH_JACKPOT` (sel-6), non-zero msg | ❌ FAIL |
| #4 `high2_2_jackpot_nonzero_msg_unit` | full `place_jackpot_hash_block`, non-zero msg | ❌ FAIL |
| `high2_2_jackpot_only_unit` | full `place_jackpot_hash_block`, **zero** msg | ✅ PASS |

Disproven hypotheses (each with a committed `#[ignore]`d
reproducer): FoldChip constraint degree (degree-2 rewrite);
batch-stark / LogUp (uni-stark fails too); CRIT-1 program-pin;
§4.D keystone; degree-vs-blowup (fails at log_blowup 4 too);
`JACKPOT_MSG` persistence discontinuity (#6: propagating it
doesn't help); degenerate-step `BIT_REG` (#9: zeroing it doesn't
help). What `IS_HASH_JACKPOT`=1 on the finalize row activates
that is message-dependent: the **C4 binding**
`IS_HASH_JACKPOT·(CV_OUT − pi_hash_jackpot)` and the
JackpotChip `is_active` constraints. Since per-row
`check_constraints` passes (CV_OUT == pi per row) but the
polynomial `verify` fails only for non-zero CV_OUT/digest, the
defect is a constraint that holds at all trace rows yet whose
committed quotient disagrees with the verifier's OOD evaluation
when the BLAKE3 digest (hence `CV_OUT` / `pi_hash_jackpot`) is
non-trivial — a latent C4/jackpot-finalize defect masked for 7+
years of tests by the universal use of an all-zero
`JACKPOT_MSG`.

**Honest status:** blind bisection (9 controls) has *bounded*
the bug precisely but **not** identified the exact faulty
constraint; I was unable to fix it within this effort. The
correct next step is **instrumentation, not more bisection**:
a custom `DebugConstraintBuilder` (or per-constraint quotient
extraction) that, for the CONTROL#8 failing trace, logs every
constraint's value *and* polynomial degree at the finalize row
+ the OOD point, to name the single constraint whose quotient
contribution is wrong for a non-zero digest. That is a focused
prover-instrumentation task. The fold/keystone/Route-A work is
sound and production-green; this is a contained pre-existing
C4/jackpot-finalize defect, not a HIGH-2.2 design flaw.

<details><summary>Earlier (disproven) leading hypothesis — kept
for provenance</summary>

**Leading fix-target hypothesis (DISPROVEN by CONTROL#6/#9):**
the `place_jackpot_hash_block` **"degenerate jackpot step"** plants
`JACKPOT_MSG[h-1] = jackpot_state` (the non-zero message) on the
last row *without* a matching store indicator, while every
preceding row has `JACKPOT_MSG = 0`. The `JackpotChip`
persistence constraint (Pearl `jackpot/constraints.rs`:
`(1 − next_store_idx[i])·(jackpot_msg[i] − next_jackpot[i])`,
a `when_transition`) at the `h-2 → h-1` transition then
evaluates `(1 − 0)·(0 − msg[i]) = −msg[i]` — **identically 0
for a zero message (masked), non-zero for a real message**:
exactly the observed pattern. (Note `check_constraints` may not
flag it if the jackpot constraints are selector-gated such that
the per-row debug check is vacuous at that transition while the
folded polynomial is not — consistent with "per-row passes,
verify fails".) **Fix candidates:** (a) make the degenerate
step set the store indicator / `JACKPOT_IDX` so the persistence
constraint admits the `0 → msg` write; or (b) write
`JACKPOT_MSG = jackpot_state` consistently on the jackpot
block's rows (not just `h-1`) so no `0 → msg` discontinuity
crosses a `when_transition`; or (c) gate the persistence
constraint by the jackpot-active selector so it's inert on the
keyed-hash finalize row. Verify the chosen fix keeps every
`crit1_*`/`high2_*`/`routea_*` green and flips
`high2_2_jackpot_nonzero_msg_unit` → pass.

**Alternative (if the above is not it):** row-by-row compare
`place_blake3_hash_with_selectors`'s generated trace for a
non-zero message against an independent BLAKE3 keyed-hash
reference *and* the `Blake3Chip` per-round constraint (message
permutation / injection). Either way a contained
jackpot/BLAKE3 trace-gen↔AIR fix, independent of all HIGH-2.2
design work.

</details>

---

<details><summary>Earlier (superseded) bisection notes — degree
& fold×jackpot hypotheses, kept for provenance</summary>

**Degree hypothesis DISPROVEN; locus
re-narrowed by bisection (the analysis below is superseded):**

- Implemented the degree-2 fix (added `FOLD_XOR_OUT`, split the
  deg-3 transition into two deg-2 constraints). FoldChip
  standalone **9/0**, `high2_2_fold_chain_in_composite_unit`
  **✓**, and the full `crit1_*`/`high2_*`/`routea_*` suite
  **19/0** (no regression from the new column). **But
  `high2_2_fold_chain_pinned_logup` still fails
  `OodEvaluationMismatch`** ⇒ FoldChip constraint *degree was
  not the cause*. (The degree-2 rewrite is kept anyway — valid
  hygiene, fully tested.)
- Bisection `high2_2_fold_chain_pinned_unistark` (same
  fold+jackpot trace, **uni-stark** pinned: §4.D keystone +
  CRIT-1 program-pin, *no* LogUp/batch-stark) **also FAILS**
  `OodEvaluationMismatch` ⇒ the bug is **not** batch-stark- or
  LogUp-specific either.
- **Established locus:** `composite_prove` (unit
  `CompositeFullAir`, no pin/keystone) + fold chain **passes**;
  adding the **jackpot-hash block *and* the pinned layer**
  (program-pin + §4.D keystone) makes it fail under *both*
  uni-stark and batch-stark. So it is a constraint interaction
  between **(fold-chain FOLD_STATE propagation) × (jackpot-hash
  block on the last rows) × (the pinned program-pin / §4.D
  keystone)** — not the prover backend, not FoldChip degree.
- **Likely suspect:** the §4.D keystone reads `FOLD_STATE` on
  the **last row**, which is *also* the
  `place_jackpot_hash_block` finalize row, and this is the
  first trace with **non-zero last-row `JACKPOT_MSG`** *and* a
  non-zero `FOLD_STATE` propagated through the jackpot block
  under the pinned `when_last_row` selector. The base
  `CompositeFullAir` `when_last_row` JACKPOT_MSG↔PI binding is
  also exercised with non-zero values here for the first time.
- **Bisection #2 DONE (`high2_2_fold_chain_jackpot_unit`):**
  unit `composite_prove` (`CompositeFullAir`, **no**
  pin/keystone/LogUp) + fold chain + jackpot block →
  **FAILED** `OodEvaluationMismatch { index: None }`.
  ⇒ **the bug is a base `CompositeFullAir` constraint
  interaction between the fold chain and the jackpot-hash
  block.** Every contributing layer is now ruled out:
  FoldChip degree (degree-2 rewrite didn't help), batch-stark /
  LogUp (uni-stark fails too), CRIT-1 program-pin and the §4.D
  keystone (this repro has none and still fails). Fold-chain
  alone ✓; jackpot-block alone (every shipping test) ✓; only
  *together* ✗. `place_jackpot_hash_block` does **not**
  overwrite the FOLD_* columns (source-verified).

- **Precise next step for the fix:** run the
  `high2_2_fold_chain_jackpot_unit` trace under **uni-stark
  debug `check_constraints`** (debug builds evaluate every
  constraint per row and panic naming the first violated one) —
  this names the exact failing chip/row instead of the opaque
  `OodEvaluationMismatch`. Prime suspects given the
  localization: (a) FoldChip's `when_transition` at the
  fold-chain↔propagation or propagation↔jackpot-block
  boundaries with the jackpot block's selector activity; (b) a
  base chip whose `when_transition`/`when_last_row` now sees
  non-zero FOLD-propagated rows colliding with the jackpot
  block's last-8-row writes; (c) the base `when_last_row`
  JACKPOT_MSG↔PI binding exercised with non-zero JACKPOT_MSG
  for the first time *while* FOLD_STATE is also non-zero.

Status: §4.A FoldChip composite wiring + degree-2 rewrite +
keystone are landed and non-regressing (full
`crit1_*`/`high2_*`/`routea_*` 19/0; the 3 fold-chain bisection
repros `#[ignore]`d, one passing). The real-M bridge path is
blocked on this **one fully-bisected, reproducible base
constraint-interaction bug** (fold-chain × jackpot-block in
`CompositeFullAir`); the next action is the named-constraint
debug run above. Production stays green (bridge reverted,
`5b2adfe`); §6/§7 follow the bridge fix.

---

<details><summary>Superseded degree-3 hypothesis (kept for
provenance)</summary>

**Root-cause hypothesis (strong; 2026-05-16):** FoldChip's
transition emits a **degree-3** constraint —
`is_fold · (res_sel − acc)` where
`res_sel = Σ_s sel[s]·nxt_state[s]` is degree 2 and `is_fold`
adds degree 1 (via `xor_32_shift_if(tb, res_sel, …, is_fold,
…)`). The composite AIR's other degree-3 constraint (matmul
`(is_reset+is_update)·dot`) is only ever exercised with **zero**
values by every shipping/test trace (`routea_honest` etc. place
*no* matmul rows and *no* fold rows). So the fold-chain test is
the **first trace in which a degree-3 constraint takes non-zero
values**, and it fails *only* under `p3-batch-stark`
(`composite_prove_pinned_logup`) while passing under
`p3-uni-stark` (`composite_prove`) — i.e. batch-stark's
quotient-chunk degree handling
(`get_log_num_quotient_chunks`) does not accommodate a non-zero
degree-3 constraint that uni-stark does. Zero-valued degree-3
contributes a zero quotient regardless of chunk count, which is
why every prior batch-stark test (zero FOLD/matmul) passed.

**Fix plan (concrete, next session):**
1. Confirm: run `high2_2_fold_chain_pinned_logup` under
   `p3-batch-stark` debug `check_constraints` (per-row should
   PASS — confirming it's a polynomial/quotient-degree issue,
   not a row violation) and print the batch-stark
   `log_num_quotient_chunks` vs the symbolic max degree.
2. Preferred fix — **reduce FoldChip's transition to degree
   ≤ 2**: add one `FOLD_XOR_OUT` column (the 32-bit XOR result),
   constrain `FOLD_XOR_OUT == acc` (degree 2: the `a+b−2ab`
   bit recomposition) as its own row constraint, then bind
   `Σ_s sel[s]·nxt_state[s] == is_fold · FOLD_XOR_OUT` (degree
   2: `sel·nxt` and `is_fold·FOLD_XOR_OUT` are both deg-2). This
   removes the deg-1×deg-2 product. Update `chips::fold`
   (LOCAL+COMPOSITE offsets, `build_trace`, the 9 standalone
   tests), `composite_layout` (+1 FOLD col), `place_fold_chain`.
3. Alternative — raise the composite STARK config's quotient
   degree so batch-stark allocates enough chunks for degree 3
   (smaller code change but also re-tunes the matmul deg-3 path;
   measure prover-cost impact).
4. Re-validate: `high2_2_fold_chain_in_composite_unit` (must
   stay ✓), un-`#[ignore]` `high2_2_fold_chain_pinned_logup`
   (must go ✓), the `routea_*`/`crit1_*`/`high2_*` structural
   gate, then re-wire the §4.A bridge (un-revert zk_bridge) and
   the full `ai-pow --features zk` e2e.

The fold *math*, the FoldChip per-row constraints, and the §4.D
keystone *data path* are all proven correct (unit + standalone +
asserted preconditions); this is a contained prover-layer
quotient-degree bug with a concrete fix, **not** a design flaw.
§6 (pin the fold schedule in CONTROL_PREP, not new PROGRAM_COLS
— avoid the §4.C.8 ~10x preprocessed blow-up) and §7
(real-difficulty e2e + docs flip) follow once the bridge path
is green.

</details>

</details>

</details>

### 4.A Honest matmul-step trace placement

**Problem.** The honest bridge places no matmul rows, so
`CUMSUM` never leaves `0`.

**Design.** Add a bridge routine
`CompositeTrace::place_matmul_tile(row_start, &a_prime_strips,
&b_prime_strips, params) -> rows_used` that, for the specific
solved tile `(tile_i, tile_j)` the plain miner returned, emits
one trace row per stripe `step ∈ 0..num_stripes`:

- writes `A_NOISED_UNPACK` / `B_NOISED_UNPACK` for that stripe's
  `r`-wide slices,
- sets `IS_RESET_CUMSUM` on `step == 0`, `IS_UPDATE_CUMSUM` on
  `step > 0` (mutually exclusive; `MatmulCumsumChip` already
  constrains the reset/update/passthrough transition),
- fills `CUMSUM_TILE` with the running accumulator so the
  existing chip constraints are satisfied.

The strips come from the real solve. `ai-pow` already exposes
`compute_tile_from_slices(a_prime_rows, b_prime_cols, params)`;
the bridge extracts the same `t*k` i8 strips from `BlockContext`
that the miner used, so the trace's accumulator is byte-identical
to the plain `c_blk` evolution.

**Selector-schedule consequence (CRIT-1).** These rows introduce
new `IS_RESET_CUMSUM` / `IS_UPDATE_CUMSUM` activity. Under
`CompositeFullAirPinned` the selector schedule is committed via
`CONTROL_PREP` in the preprocessed program. The canonical program
(`extract_program` / `composite_setup`) must therefore include
the matmul/fold row schedule, and the verifier rebuilds it from
the trusted shape — not from the proof. The number of stripes is
a function of `params` (public), so the schedule is
deterministic and safe to fix in the VK.

### 4.B The fold chip (rotate-left-13 XOR) — core new constraint

**Problem.** Nothing in-circuit computes `M`. The keystone binds
`C` (raw accumulator), but the real digest is `BLAKE3(M)`.

**Design choice (settled in §9.5):** Pearl implements this as a
bit-serial RAM machine with rotate-on-load + back-shift
compensation (Option B1). We take **Option B2** — the direct
per-stripe fold below — because our SNARKs are deliberately not
trace-byte-equivalent to Pearl (only the mineable unit of work
is), and B2 produces a bit-identical `JACKPOT_MSG` without
Pearl's compensation-schedule complexity. See §9 for the full
Pearl analysis and the B1/B2 rationale.

**Design.** A new chip `FoldChip` enforcing, per matmul step row
(gated by an `IS_FOLD` selector co-scheduled with the
update/reset rows):

```
x_step      = ⊕_{k<CUMSUM_TILE_LEN} CUMSUM_TILE[k]            (int32 XOR)
slot        = step mod 16                                     (from schedule)
M_next[slot]= rotl13(M_cur[slot]) ⊕ x_step
M_next[s]   = M_cur[s]   for s ≠ slot
```

with `M` carried in a new `FOLD_STATE` column block (16 × i32),
zero on the first fold row (boundary constraint), and on the
**last trace row** `FOLD_STATE == JACKPOT_MSG` (this generalises
the keystone — see §4.D).

Constraint algebra (all in Goldilocks; reuse the BLAKE3 chip's
existing 32-bit machinery — `chips/blake3/round_ops.rs::
xor_32_shift_if` and the bit-decomposition columns):

- **int32 XOR of the accumulator → `x_step`.** XOR is not native
  in the field; decompose each `CUMSUM_TILE[k]` into 32 bits
  (range-checked by the existing bit buses) and reduce with the
  same XOR gadget the BLAKE3 chip uses. With `CUMSUM_TILE_LEN =
  4` this is 3 pairwise 32-bit XORs.
- **`rotl13`.** `rotate_left(13)` of a 32-bit word is a fixed bit
  permutation: `rotl13(w) = ((w << 13) | (w >> 19)) mod 2³²`.
  Express via the existing `xor_32_shift_if`-style shift/select
  rows, or directly as a linear recombination of the 32
  range-checked bits of `M_cur[slot]` (no new lookup tables).
- **slot routing.** `slot = step mod 16` is schedule-fixed
  (preprocessed), so the per-row update is a *constant* index,
  not a prover-chosen one-hot — implement as 16 conditional
  copy constraints keyed by the preprocessed `slot` indicator,
  exactly mirroring how the jackpot chip's `JACKPOT_SLOT_SEL`
  one-hot already works, but with the selector pinned in the VK.

**i32/u32 boundary.** `rotate_left` operates on the `u32`
reinterpretation; the accumulator XOR is on the `i32`
two's-complement bit pattern. Both are the same 32 bits — the
chip works on the bit decomposition throughout and only the final
`M.to_le_bytes()` feeds BLAKE3, so there is no signedness
ambiguity as long as the 32-bit decomposition is the single
source of truth (assert `Σ bit_i·2ⁱ == CUMSUM_TILE[k]` in the
field with the standard 32-bit reconstruction).

### 4.C Committed-matrix end-to-end binding — RESEARCH (the cryptographic core)

> This is the load-bearing, multi-day sub-item. The fold math
> (§4.A/§4.B) is done; §4.C is what makes the bound message
> provably a function of the *committed* matrices rather than
> prover-chosen accumulator inputs. Researched 2026-05-15 against
> the actual ai-pow-zk + Pearl source; conclusions below.

#### 4.C.0 The precise problem (sharper than the original sketch)

The original sketch said "route `A/B_NOISED_UNPACK` onto the
existing `noised_packed` LogUp bus." Reading the code shows the
real situation is one level deeper:

- The `noised_packed` LogUp **is implemented** — but only in
  `composite_full_air_with_lookups.rs`
  (`CompositeFullAirWithLookups`, Phase 14b), via
  `p3_lookup::InteractionBuilder::push_interaction` and the
  `p3-batch-stark` interaction prover. `bus_emit::noised_packed`
  binds the matmul chip's per-row `A_NOISED`/`B_NOISED` (indexed
  by `A_ID`/`B_ID`) to the canonical `NOISED_PACKED` RAM store.
- The **production proven path does not use that AIR.**
  `composite_prove_pinned` → `CompositeFullAirPinned` →
  `prove_with_preprocessed` over `CompositeFullAir` (plain
  `p3-uni-stark`, **no interactions**). So on the pinned path
  `A_NOISED_UNPACK`/`B_NOISED_UNPACK` are *unconstrained relative
  to the committed matrices* — the matmul chip will compute the
  accumulator against whatever the prover writes there.

So §4.C is not "add a bus." It is: **make the CRIT-1
program-pinned production proof also enforce the `noised_packed`
(and its supporting i8u8 / range) interactions** — i.e. unify
two prover stacks that today are mutually exclusive
(`prove_with_preprocessed`/uni-stark for CRIT-1 vs
`prove_batch`+interactions/batch-stark for LogUp). That
unification is the genuine difficulty.

#### 4.C.1 What must be bound, and what is already there

Target statement: the `t·t`-cell accumulator the FoldChip's
`X_STEP` sequence is reduced from must be the dot-products of
the **committed** `A`,`B`. The existing chain, once the LogUp is
live on the pinned path:

```
HASH_A / HASH_B  (CRIT-1-pinned PIs)
   │  C3  (composite_full_air.rs §"C3", already in the pinned AIR):
   │      IS_MSG_MAT·IS_NEW_BLAKE·(BLAKE3_MSG − base256(UINT8_DATA)) = 0
   ▼
NOISED_PACKED  (canonical store; InputChip enforces, already pinned:
   │            NOISED_PACKED = polyval(MAT_UNPACK,256)+polyval(NOISE_UNPACK,256),
   │            NOISE_PACKED_PREP = polyval(NOISE_UNPACK,129))
   │  noised_packed LogUp on MAT_ID / A_ID / B_ID   ← THE MISSING LINK on the pinned path
   ▼
A_NOISED / B_NOISED → A_NOISED_UNPACK / B_NOISED_UNPACK
   │  MatmulCumsumChip (already pinned): 2×2×16 dot → CUMSUM_TILE
   ▼
per-stripe accumulator → X_STEP  ← NEW reduction constraint (§4.C.4)
   │  FoldChip (§4.B, done)
   ▼
FOLD_STATE → JACKPOT_MSG (§4.D keystone) → C4 → HASH_JACKPOT → C2 difficulty
```

Everything except the two ◀-marked links is already enforced on
the pinned path. C3 and InputChip are already in
`CompositeFullAir::eval` (hence in `Pinned`). The only gaps are
(a) the `noised_packed` LogUp not being live on the pinned path,
and (b) the accumulator→`X_STEP` reduction.

#### 4.C.2 Noise scoping (resolves a major open question)

`NOISE_PACKED_PREP` is a **preprocessed** column and `InputChip`
ties `NOISED_PACKED = pack(MAT_UNPACK) + pack(NOISE_UNPACK)`.
`HASH_A`/`HASH_B` commit the **clean** matrix (BlockContext:
`matrix_commitment(a_bytes, kappa)` over clean i8→u8 bytes), and
C3 binds the *clean* `MAT_UNPACK` to it. The **noise is
program-fixed (VK/preprocessed)**, not prover-chosen, and its
derivation from the seeds `s_a`/`s_b` (Pearl `pearl_noise.rs`:
keyed-BLAKE3 uniform matrix + permutation/choice matrices) is
**not proven in-AIR** today.

**Decision (Pearl-faithful, recorded):** §4.C binds the *clean
committed* matrix into the accumulator and treats "the
preprocessed `NOISE_UNPACK` equals `f(s_a, s_b)`" as a separate,
explicitly out-of-scope **noise-derivation obligation** — the
same external/preprocessed scoping class as C2-difficulty-
external and MED-3-target-external. It is its own (larger) work
item, *not* part of HIGH-2.2. Without it, an adversary who could
choose the preprocessed noise is *already* excluded by CRIT-1
(the preprocessed trace is verifier-fixed), so program-pinning
the noise columns (§6) is the actual safeguard; proving
`noise == f(seeds)` in-AIR is a strengthening, not a HIGH-2.2
blocker. **This must be stated as residual in
`ZKP_SECURITY_REPORT.md` when HIGH-2.2 closes.**

#### 4.C.3 The core challenge: unifying CRIT-1 pinning with LogUp

Three implementation routes, with trade-offs:

- **Route A — preprocessed + interactions in one prover.** Add
  the CRIT-1 program-pin constraints + a preprocessed trace to
  `CompositeFullAirWithLookups`, and prove via a prover stack
  that supports *both* preprocessed columns and interactions.
  Cleanest end state; **feasibility hinges on whether
  `p3-batch-stark` (the interaction prover) supports a
  preprocessed/verifier-fixed trace.** First implementation step
  is to confirm that capability in the pinned p3 revision
  (`6de5cba`). If yes, this is the route.
- **Route B — fold the binding into uni-stark without LogUp.**
  Replace the `noised_packed` *lookup* with an in-AIR
  *equality/permutation* argument that lives in plain
  `p3-uni-stark` (the prover CRIT-1 already uses). E.g. a
  grand-product/permutation constraint that the multiset of
  `(MAT_ID, A_NOISED)` matmul reads equals the multiset of
  `(row, NOISED_PACKED)` table entries — implementable as
  auxiliary columns + a running-product column under
  `prove_with_preprocessed`. Heavier per-row but stays on one
  prover; no batch-stark dependency. Fallback if Route A's
  preprocessed support is absent.
- **Route C — narrow direct binding (no general RAM lookup).**
  The matmul schedule is fixed (CRIT-1/§6), so each matmul row's
  `(A_ID,B_ID)` is a *known constant* per row. Then
  `A_NOISED_UNPACK` can be tied to `NOISED_PACKED` by **direct
  preprocessed equality**: the program (preprocessed trace)
  carries, per matmul row, the committed `NOISED_PACKED` slice
  for that row's fixed `MAT_ID`, and a pinned constraint forces
  `A_NOISED_UNPACK == preprocessed slice`. No lookup at all —
  the binding rides entirely on the CRIT-1 preprocessed
  mechanism that already exists and is audited. This collapses
  §4.C into §6 (program extension) + a pinned equality, at the
  cost of a wider preprocessed trace.

> **SUPERSEDED by §4.C.8 (naive C cost-rejected) and §4.C.9
> (B eliminated, A confirmed feasible). The live fork is A vs
> C2 — read §4.C.9 for the current recommendation.** The
> original text below is kept for provenance.

**Recommendation (original): prototype Route C first.** It reuses
the exact CRIT-1 preprocessed-pinning machinery already shipped
and audited (`9ec529e`), needs **no** new prover stack and **no**
LogUp soundness re-analysis, and the matmul schedule being
fixed makes the per-row `MAT_ID` a constant — so a direct
preprocessed-equality binding is sound by the same argument
CRIT-1 already relies on. Route A is the "right" long-term shape
if batch-stark has (or gains) preprocessed support; Route B is
the general fallback. Decide after a one-day spike confirming
(i) batch-stark preprocessed support (Route A gate) and
(ii) the preprocessed-trace width blow-up of Route C
(`num_matmul_rows × A_NOISED_UNPACK_LEN` extra preprocessed
cells — quantify against `MIN_STARK_LEN` and FRI cost).
[Update: (i) answered YES and (ii) measured prohibitive — see
§4.C.8/§4.C.9.]

#### 4.C.4 Accumulator → `X_STEP` reduction (new constraint)

`X_STEP[step] = ⊕` over **all `t·t`** accumulator cells (Pearl
§4.5 `x_ℓ`), but `MatmulCumsumChip` only maintains a 2×2
micro-tile (`CUMSUM_TILE_LEN=4`, §4.0). Two sub-pieces:

1. **Subtile sweep.** Per stripe, the fixed schedule (CRIT-1/§6)
   sweeps the `(t/2)²` 2×2 sub-blocks (Pearl's
   `pearl_program.rs` subtile loop), each addressing its
   `NOISED_PACKED` rows via `MAT_ID`. A per-stripe running XOR
   accumulator column folds each sub-block's 4 i32 cells in as
   they are produced.
2. **XOR tree.** Reduce the stripe's accumulator cells to one
   32-bit `X_STEP`: bit-decompose each i32 cell (reuse the
   range/bit buses + the `xor_32_shift_if`-style XOR gadget the
   FoldChip already uses), pairwise-XOR to a single 32-bit
   value, expose it as the FoldChip's `X_STEP` input. Cost is
   `O(t²·32)` boolean cells per stripe — the dominant new width;
   quantify and prefer aliasing BLAKE3 bit-scratch on disjoint
   rows (the schedule keeps matmul/fold rows off compression
   rows).

`X_STEP` is the single hand-off scalar between this reduction
and the (already-built, tested) FoldChip — the clean interface
the §4.0 decision was designed around.

#### 4.C.5 Soundness once §4.C lands

With the binding live on the pinned path: an adversary cannot
put matrix *X* in the accumulator inputs and matrix *Y* in
`HASH_A` — `A_NOISED`/`B_NOISED` are bound (Route A/B lookup, or
Route C preprocessed equality) to `NOISED_PACKED`, whose clean
component is C3-bound to `HASH_A`/`HASH_B`, which are
CRIT-1-pinned PIs. The full chain
`committed A,B → accumulator → X_STEP → FoldChip → JACKPOT_MSG →
C4 → HASH_JACKPOT → C2` is then closed, eliminating HIGH-2.2's
useful-work forgeability residual. **Remaining residual after
§4.C:** the noise-derivation obligation (§4.C.2) — explicitly
out of scope, tracked separately, and not a forgery hole because
the noise columns are CRIT-1-pinned.

#### 4.C.6 Interaction with CRIT-1 / §6 (must not be overlooked)

If Route A/B (LogUp / permutation) is taken, the multiplicity /
`*_FREQ` columns and the `MAT_ID`/`A_ID`/`B_ID` index columns are
prover-influenced and **must themselves be part of the CRIT-1
preprocessed program** (else a prover games multiplicities or
re-points indices). Route C sidesteps this (no multiplicities;
the binding *is* preprocessed). Either way, §6 (program
extension) is a hard prerequisite and must enumerate every
column the binding's soundness rests on.

#### 4.C.7 Concrete next steps (for the dedicated session)

1. Spike: does `p3-batch-stark` @ `6de5cba` support a
   preprocessed/verifier-fixed trace? (Route A gate.)
2. Quantify Route C preprocessed-width blow-up vs `MIN_STARK_LEN`
   / FRI cost.
3. Pick Route (recommend C unless the spike makes A cheap).
4. Implement the accumulator→`X_STEP` reduction (§4.C.4) — this
   is route-independent and can land first, tested standalone
   like the FoldChip (against `compute_tile_trace`'s `x_steps`).
5. Implement the chosen binding route; extend §6 program to
   cover every soundness-bearing column.
6. Wire FoldChip + §4.D keystone; §7 end-to-end at real
   difficulty; doc + `ZKP_SECURITY_REPORT.md` flip (note the
   §4.C.2 noise residual).

#### 4.C.8 Empirical result: naive Route C is cost-prohibitive (2026-05-15)

**Done & landed (route-independent core):** §4.C.4 `XStepChip`
(`chips/xstep.rs`, commit `290af68`) — 6/0 self-contained tests +
cross-crate parity (`zk_bridge::high2_2_xstepchip_byte_equiv_plain_x_steps`)
+ the composed `XStep→Fold` pipeline capstone
(`high2_2_xstep_fold_pipeline_byte_equiv_plain`, `c78ae67`):
the entire useful-work *computation* chain (real committed
matrices → accumulator → `X_STEP` → fold → `M` → keyed-BLAKE3 ==
plain PoW digest) is proven byte-equivalent end-to-end.

**Naive Route C tried and REVERTED.** Extending `PROGRAM_COLS`
5 → 69 (append `A_NOISED_UNPACK` ‖ `B_NOISED_UNPACK`) is the
mechanically-least-invasive binding and is *correct*, but it was
measured **cost-prohibitive**: the preprocessed trace is
committed + FRI'd at full trace height (`MIN_STARK_LEN` = 8192)
every `composite_setup`, twice per test, so the composite_proof
suite blew from fast to **~22 min CPU (10x+ prover regression)**.
This is precisely the §4.C.7-step-2 risk, now empirically
confirmed. Compounding it: the binding is **vacuous in the
shipping path today** — `zk_bridge` places no matmul rows until
§4.A, so the 64 pinned columns are all-zero and buy nothing yet.
"Least invasive" must include *not* shipping a 10x prover
blow-up to close a currently-vacuous binding. Reverted to the 5
CRIT-1 anchors; finding recorded here.

**Cost-aware Route C redesign (for the §4.A co-landing):** the
expense is pinning 64 cols × *all 8192 rows*; the matmul inputs
are non-zero on only the few real matmul rows. Options to make
Route C affordable, to be implemented *together with* §4.A
(when real matmul rows exist) and *measured*:

- **C1 — narrow preprocessed block.** Pin only a compact
  `MATMUL_PIN` column group whose width is the per-row matmul
  input (`2·TILE_H·TILE_D`), not the full
  `A/B_NOISED_UNPACK` span, and only where the schedule places
  matmul rows; elsewhere the canonical program is 0 (free to
  commit — constant columns are cheap in FRI but the *width*
  still costs; quantify).
- **C2 — selector-gated equality, not a preprocessed column.**
  Keep `PROGRAM_COLS` at 5; add an in-AIR constraint
  `IS_MATMUL · (A_NOISED_UNPACK − <bound source>) = 0` where the
  bound source is a *single* already-pinned anchor (e.g. derive
  the expected matmul slice from `AB_ID_PREP`, which is already
  in `PROGRAM_COLS`, via the existing C3/`NOISED_PACKED`
  relation). No preprocessed widening at all — collapses §4.C
  into a gated equality + §6 schedule pinning. **Most promising;
  re-evaluate vs Route B.**
- **C3 — accept the cost only at PROD with a smaller
  `MIN_STARK_LEN`/blowup, measured.**

**Net §4.C status:** route-independent core done + proven
byte-equivalent; the *binding* is designed, its naive form
empirically rejected on cost, and a cost-aware redesign (C2
preferred) is specified to co-land with §4.A. Soundness
unaffected throughout (CRIT-1 + keystone hold; the binding is a
useful-work-fidelity strengthening, not a forgery fix).

#### 4.C.9 Route B evaluated — ELIMINATED; Route A feasibility confirmed (2026-05-15)

Route B was "an in-AIR permutation / grand-product multiset
binding on the **uni-stark** prover (no preprocessed widening,
no batch-stark)." Evaluated against the pinned p3 rev `6de5cba`:

**Finding 1 — Route B is infeasible on uni-stark.**
`p3-uni-stark::prove_with_preprocessed` is strictly
**single-phase**: it commits (preprocessed + main), observes the
commitment, draws *one* constraint/quotient challenge, then
quotient + FRI (`uni-stark/src/prover.rs:151–206`). There is
**no second committed-trace round** and **no
post-main-commitment challenge** exposed to an in-trace column.
A sound multiset-permutation / grand-product argument
fundamentally requires a running-product (or LogUp fraction)
column accumulated over a verifier challenge drawn *after* the
data is committed. On uni-stark the prover would have to commit
that column before the challenge exists ⇒ the argument is
unsound (the prover can pre-cook the product). So Route B as
specified **cannot be built** here. Any randomized cross-row
multiset binding necessarily lives in a multi-phase prover.

**Finding 2 — that multi-phase prover already exists, with
preprocessed support: Route A is feasible and largely
pre-built.** `p3-batch-stark::prove_batch`
(`batch-stark/src/prover.rs:88`) is exactly multi-phase:
commit main+preprocessed → draw lookup challenges → generate &
commit permutation traces → quotient. It takes **per-instance
preprocessed widths** *and* lookups/permutation **together**
(`preprocessed_widths`, `generate_permutation`,
`InteractionSymbolicBuilder`; `check_constraints` takes both
`preprocessed` and `permutation`/`permutation_challenges`). This
answers the open §4.C.7 Route-A gate: **YES, batch-stark
supports a preprocessed/verifier-fixed trace simultaneously with
the LogUp argument.** Moreover `ai-pow-zk` *already* has
`CompositeFullAirWithLookups` (the `noised_packed` LogUp emitted
via `InteractionBuilder`) and *already* drives it through
`prove_batch`/`verify_batch` in `bench_suite.rs`. So the LogUp
binding is implemented and provable today; productionising it is
(a) add the CRIT-1 program-pin constraints to
`CompositeFullAirWithLookups` (it already shares
`CompositeFullAir::eval`; add the same preprocessed-equality the
pinned AIR uses) and (b) switch the production
`composite_prove_pinned` / `zk_bridge` path from
`uni-stark::prove_with_preprocessed` to
`batch-stark::prove_batch`.

**Revised route landscape (B removed):**

| Route | Prover | Binding | Status |
|---|---|---|---|
| **A** | batch-stark (multi-phase) | `noised_packed` LogUp + CRIT-1 preprocessed pin, unified | **Feasible, ~½ pre-built** (WithLookups + prove_batch exist; needs the program-pin added + production switch). The principled end state. |
| ~~B~~ | ~~uni-stark~~ | ~~in-AIR permutation~~ | **ELIMINATED** — uni-stark is single-phase; no sound randomized argument possible. |
| **C2** | uni-stark (unchanged) | deterministic schedule-fixed gated equality `IS_MATMUL·(A_NOISED_UNPACK − bound) = 0`, no preprocessed widening | Viable but **needs §4.A** (the fixed matmul↔C3 schedule must exist for `bound` to be well-defined); no prover migration. |

**Recommendation.** The real fork is now **A vs C2**, not B.
Route A is the cryptographically-cleanest and is already
half-built (the audited `noised_packed` LogUp + a working
`prove_batch` path in `bench_suite`), but it migrates the
*production* proof system uni-stark → batch-stark (a real,
testable but non-trivial swap touching `composite_prove_pinned`,
`composite_verify_*`, `zk_bridge`, the CRIT-1 `crit1_*` suite,
and proof-size/perf). Route C2 keeps the prover but defers into
§4.A and is only a deterministic (non-randomized) binding.
**Suggested:** spike Route A by adding the program-pin to
`CompositeFullAirWithLookups` and proving it via `prove_batch`
on the existing bench harness (no production switch yet) to
measure prover cost vs the uni-stark baseline; if acceptable,
Route A becomes the §4.C binding and subsumes what B/C wanted.
This spike is self-contained and testable without the §4.A
composite surgery.

**Net:** Route B is conclusively out (single-phase uni-stark
limitation, evidence-cited). Route A is confirmed feasible and
mostly pre-existing — the recommended binding, pending a
prover-cost spike. Route C2 remains the no-prover-migration
fallback, gated on §4.A. The §4.C *computation* core
(XStepChip + pipeline) stands, byte-equivalent to plain.

#### 4.C.10 Route-A spike result — VIABLE, SOUND, ~1.23x (2026-05-16)

Per user direction ("attempt Route A"). Implemented
`CompositeFullAirWithLookupsPinned`
(`composite_full_air_with_lookups.rs`, commit `7c0cf3e`): it
composes the CRIT-1 program-pin + HIGH-2 keystone (delegating to
`CompositeFullAirPinned`) with every `noised_packed`/range LogUp
bus emission, proven via `p3-batch-stark::prove_batch`.
Preprocessed rides the standard `BaseAir::preprocessed_trace()`
API that `ProverData::from_instances` reads automatically — the
*same* mechanism the uni-stark pinned AIR uses, so **no
preprocessed-width blow-up** (the §4.C.8 trap is avoided
entirely; the program stays the 5 CRIT-1 anchors, the binding
comes from the LogUp permutation argument, not a wide
preprocessed trace).

Self-contained spike (no production switch), both tests green:

- **`route_a_honest_roundtrip_and_cost`** — CRIT-1 program-pin
  **and** every LogUp bus enforced together in one
  `prove_batch`/`verify_batch`. **Route A is viable.**
- **`route_a_crit1_forgery_rejected`** — a zeroed-selector
  forgery is self-consistent vs its own program but **REJECTED**
  vs the canonical program's preprocessed commitment under
  batch-stark. **CRIT-1 soundness holds under Route A.**
- **Measured prover cost:** `prove_batch(pinned+LogUp) = 26621
  ms` vs uni-stark pinned baseline `21693 ms` ⇒ **≈1.23x**.
  Versus naive Route C's ~10x / 22-min blow-up (§4.C.8), this is
  entirely acceptable.

**Decision: Route A is the §4.C binding.** The §4.C.9 open
prover-cost question is conclusively answered. Route table
final:

| Route | Verdict |
|---|---|
| **A** (batch-stark: CRIT-1 pin + `noised_packed` LogUp unified) | ✅ **CHOSEN** — viable, CRIT-1-sound, ~1.23x; spike `7c0cf3e` |
| ~~B~~ (uni-stark in-AIR permutation) | ❌ eliminated (§4.C.9, single-phase) |
| ~~C naive~~ (PROGRAM_COLS 5→69) | ❌ rejected (§4.C.8, ~10x) |
| C2 (uni-stark gated equality) | superseded by A (no longer needed) |

**Production API + exhaustive suite landed (`697cc0e`).**
`composite_proof` now exposes `composite_prove_pinned_logup` /
`composite_verify_pinned_logup` / `composite_verify_pow_pinned_logup`
(batch-stark over `CompositeFullAirWithLookupsPinned`; verifier
rebuilds the canonical preprocessed commitment witness-free via
`ProverData::from_airs_and_degrees` — same CRIT-1 trust model).
Exhaustive adversarial suite `composite_proof::tests::routea_*`
(**4/4 green, 137 s**): honest roundtrip + C2 PoW target
sensitivity; zeroed-selector forgery rejected vs canonical;
tampered PROGRAM_COL rejected for **all 5** cols; HIGH-2
free-jackpot keystone holds — all under batch-stark. 334
existing lib tests untouched. **The §4.C binding mechanism is
now production-grade and exhaustively adversarially tested.**

**Caller integration DONE.** `ai-pow::zk_bridge::prove_and_verify`
(the `mine()` gate) and `f1_harness` now call
`composite_prove_pinned_logup` / `composite_verify_pow_pinned_logup`
— production proving is the Route-A batch-stark path. The
throwaway `route_a_spike` module was removed (superseded by the
production `routea_*` suite; its cost datum is recorded above).
`composite_proof`'s module doc now carries the **three-tier
entrypoint table** (unpinned dev / uni-stark `*_pinned` no-LogUp
harness / **`*_pinned_logup` = production**), and `zk_bridge` /
`f1_harness` headers point at it. The uni-stark `*_pinned`
family is retained (documented) as the lighter no-LogUp variant
that backs the `crit1_*`/`high2_*` constraint-logic suite — not
deleted (still-used, valuable regression coverage).

**Only remaining for *useful-work* end-to-end closure:** the
`noised_packed` binding is non-vacuous only once **§4.A** places
real matmul rows whose `A_NOISED`/`B_NOISED` reads hit the
canonical store (today the bridge places none, so the LogUp
balances trivially and CRIT-1 + C1/C3/C4 are the live bindings).
Route A is proven, sound, cheap (~1.23x), a tested production
API, **and now the wired production path**. §4.C-binding is
complete; §4.A is the distinct remaining workstream (#97).

### 4.D Keystone generalisation

Once 4.B lands, replace the stop-gap keystone

```
JACKPOT_MSG[0..4] == CUMSUM_TILE[0..4];  JACKPOT_MSG[4..16] == 0
```

with the faithful binding

```
last row:  JACKPOT_MSG[0..16] == FOLD_STATE[0..16]
```

i.e. the C4-hashed 64 bytes are exactly the folded `TileState M`.
This is still an unconditional `when_last_row` boundary predicate
(sound the same way the current keystone is — not a prover
selector), placed in `CompositeFullAirPinned` only; the unit
`CompositeFullAir` keeps cumsum/jackpot as independent PIs so the
~300 constraint-logic tests stay untouched.

### 4.E Tile-index / target-derivation (MED-3 interplay)

The chosen `(tile_i, tile_j)` and the difficulty `target`
(`difficulty_target(&params)`) are currently external
(`zk_bridge::prove_and_verify` hard-codes tile `(0,0)`). HIGH-2.2
should bind *which* tile is being attested (so a prover cannot
solve an easy tile and claim a hard one).

**Status (2026-05-16): §4.E is entangled with §6(b) — see
"Remaining soundness scope".** A standalone `tile_i/tile_j`
public input is **not** a sound closure on its own: nothing
in-circuit yet ties the hashed digest to a *specific tile's
committed-matrix accumulator* (the honest bridge places no matmul
subtile-sweep rows; `place_fold_chain` consumes prover-supplied
`x_steps`), so a free tile PI would be vacuous — a malicious
prover with fabricated `x_steps` can set any tile PI. The
meaningful binding requires §6(b)/§4.C.4 (place the subtile-sweep
rows; force `FOLD_XSTEP == ⊕CUMSUM_TILE` of the committed `A/B`)
and *then* binding `(tile_i,tile_j)` to that accumulator's
row/col offsets. **MED-3 reconciliation:** MED-3 documents how
the *verifier* derives the claimed `(tile_i,tile_j)` from the
block context; HIGH-2.2 must consume MED-3's derivation as the
verifier-supplied input bound to the in-circuit accumulator —
**not** introduce an independent free PI. Until §6(b) lands,
HIGH-2.2 does not regress MED-3 (the attested tile is the honest
bridge's choice; soundness held by CRIT-1 + keystone + §6(a)),
and the precise obligation is tracked jointly with §6(b) as the
one entangled cryptographic-core residual.

---

## 5. Constraint sketch (FoldChip)

Per fold row `step` (preprocessed `slot = step mod 16`,
`is_first_fold`, `is_last_fold` indicators in the VK program):

```
// 1. accumulator XOR  (bit-decompose, reuse blake3 XOR gadget)
for k in 0..CUMSUM_TILE_LEN:
    assert  Σ_{i<32} cbits[k][i]·2^i  ==  CUMSUM_TILE[k]
    (each cbits[k][i] booleanity-checked by existing bit bus)
xb[i] = cbits[0][i] ⊕ cbits[1][i] ⊕ cbits[2][i] ⊕ cbits[3][i]   ∀ i<32

// 2. rotl13 of the addressed slot
mbits[i] = bit i of M_cur[slot]            (decomposition asserted == M_cur[slot])
rot[i]   = mbits[(i + 32 - 13) mod 32]     (pure index permutation, no cost)

// 3. folded slot
M_next[slot]  ==  Σ_{i<32} (rot[i] ⊕ xb[i])·2^i
M_next[s]     ==  M_cur[s]                 ∀ s ≠ slot   (schedule-fixed copies)

// 4. boundaries
when is_first_fold:  M_cur[s] == 0          ∀ s
when is_last_fold (= last trace row, pinned):  JACKPOT_MSG[s] == M_next[s] ∀ s<16
```

No new lookup tables: booleanity + 32-bit reconstruction reuse
the URange/bit buses the BLAKE3 chip already drives; XOR reuses
`xor_32_shift_if`. The only new columns are `FOLD_STATE` (16
i32), the per-row `cbits`/`mbits` scratch (can alias existing
BLAKE3 bit scratch if the schedule keeps fold rows disjoint from
compression rows — preferred, to avoid widening
`TOTAL_TRACE_WIDTH`).

---

## 6. Test plan

Unit / constraint (against `CompositeFullAir` harness):

- `fold_single_step_matches_tilestate_fold` — random `C`, one
  step, AIR `M_next` == `TileState::fold` reference.
- `fold_chain_matches_compute_tile` — full `num_stripes` chain,
  AIR `FOLD_STATE` == `compute_tile_from_slices(...)`.
- `fold_rotl13_boundary` — `rotate_left(13)` edge bits
  (bit 18↔19 wrap) exact.
- `fold_first_row_state_must_be_zero` — non-zero initial `M`
  rejected.

End-to-end (against `CompositeFullAirPinned`, real bridge):

- `high2_2_honest_real_tile_roundtrip` — real `mine()` solve →
  `place_matmul_tile` + `FoldChip` rows → `JACKPOT_MSG ==
  TileState M` → `composite_verify_pow_pinned` clears the **real**
  `difficulty_target` (the test that proves §2's "No" becomes
  "Yes").
- `high2_2_byte_equiv_plain` — assert SNARK `HASH_JACKPOT` ==
  `TileState::keyed_hash(&s_a)` from the plain solve, byte-for-byte.

Adversarial (must reject):

- forged `FOLD_STATE` not equal to the constrained fold;
- swapped/cheaper A or B (caught via 4.C → `HASH_A` mismatch vs
  pinned PI);
- skipped/duplicated stripe (caught by the reset/update schedule
  in the pinned program);
- claiming a different tile than the one solved (4.E).
- regression: existing `crit1_*` and
  `high2_free_jackpot_message_rejected` still pass with the
  generalised keystone.

Full regression gate before commit: `cargo test -p ai-pow-zk
--lib` (expect 295+ green) and `cargo test -p ai-pow --features
zk` (the pinned mine-gate end-to-end suite).

---

## 7. Sequencing

1. **4.A** `place_matmul_tile` + wire real strips from
   `BlockContext`; CUMSUM evolves to the real accumulator on the
   honest path (keystone still `[0..4]`/`[4..16]==0`, so
   `JACKPOT_MSG` must now carry the accumulator — interim).
2. **4.B** `FoldChip` + `FOLD_STATE`; unit tests vs
   `TileState::fold` / `compute_tile`.
3. **4.D** generalise the keystone to `JACKPOT_MSG ==
   FOLD_STATE` (16 slots).
4. **4.C** route accumulator inputs onto the `noised_packed` bus;
   end-to-end committed-matrix adversarial tests.
5. **4.E** bind tile index / reconcile with MED-3.
6. CRIT-1 program: extend `extract_program` /
   `composite_setup` to cover the matmul+fold schedule; confirm
   the canonical VK rebuilds it from shape only.
7. Docs: flip HIGH-2 from "soundness resolved / fidelity
   residual" to **fully resolved** in
   `ZKP_SECURITY_REPORT.md`, `GAP_AUDIT.md`, memory.

Each step is independently testable and commits with the
`Co-Authored-By: Claude Opus 4.7 (1M context)
<noreply@anthropic.com>` trailer; push only when explicitly
requested.

---

## 8. Risks & open questions

- **Trace width.** `FOLD_STATE` (16 i32) + bit scratch. Prefer
  aliasing BLAKE3 bit scratch on disjoint rows over widening
  `TOTAL_TRACE_WIDTH` (width drives prover cost / FRI). Confirm
  the schedule keeps fold rows off compression rows.
- **Row budget.** `num_stripes` fold rows + matmul rows must fit
  the padded trace height alongside the matrix-hash and
  jackpot-hash blocks; may push the next power-of-two and raise
  prove time. Measure with `f1_harness` / `scripts/profile_f1.sh`.
- **Merge-mining invariant.** Per the standing constraint, SNARKs
  are *not* byte-equivalent across chains — only the mineable
  unit of work is. `place_matmul_tile`/`FoldChip` must keep the
  *plain* `TileState`/`keyed_hash` as the byte-equivalent anchor
  and not leak circuit-specific encoding into the hashed message
  (the 16×u32-LE layout is the contract; honour it exactly).
- **i32 wrapping.** `wrapping_add` in the accumulator vs field
  arithmetic — the existing `MatmulCumsumChip` already handles
  the 32-bit accumulator domain; FoldChip must consume the same
  representation, not re-range to a different modulus.
- **Open:** does tile-index binding (4.E) belong in HIGH-2.2 or
  is it cleaner to land MED-3 first and have HIGH-2.2 consume its
  derivation? Decision deferred to start-of-implementation; does
  not block 4.A–4.D.

---

## 9. How Pearl accomplishes this (reference implementation)

Pearl already implements the complete matmul→fold→jackpot-hash
chain in its Layer-0 STARK. Studying it both *validates* the
plain-side reference (§3) and *changes* the §4.B design decision.
File references are `pearl/zk-pow/src/...`.

### 9.1 The scalar truth (`circuit/chip/jackpot/helper.rs::compute_jackpot`)

```rust
for ll in (r..=k).step_by(r) {
    for u in 0..h { for v in 0..w { for l in ll-r..ll {
        jackpot[u][v] += (sa[u][l]+na[u][l]) * (sb[v][l]+nb[v][l]); }}}
    let xored_tile = jackpot.iter().flatten().fold(0u32, |a,&x| a ^ x as u32);
    let tid = (ll/r - 1) % JACKPOT_SIZE;            // JACKPOT_SIZE = 16
    jackpot_msg[tid] = jackpot_msg[tid].rotate_left(LROT_PER_TILE) ^ xored_tile;
}                                                   // LROT_PER_TILE = 13
```

This is **structurally identical** to ai-pow's
`matmul.rs::compute_tile` + `TileState::fold` (same constants:
`TILE_H=2`, `TILE_D=16`, `JACKPOT_SIZE=16`, `LROT=13`,
`BITS_PER_LIMB=13`). **Conclusion:** ai-pow's plain side is a
faithful port; §3 is correct; the only gap is the *circuit*.

### 9.2 Matmul chip — the accumulator (`circuit/chip/matmul/constraints.rs`)

Identical semantics to ai-pow's `MatmulCumsumChip`, expressed as
a **transition** constraint on the `next` row:

```
ip        = <noised_a[i], noised_b[j]>
prev      = mux(next_is_reset, cumsum_tile, 0)
updated   = prev + ip
expected  = mux(next_is_update, prev, updated)        // else: pass-through
constraint_eq(next_cumsum_tile[i*TILE_H+j], expected)
```

Crucially it also binds the *committed* matrices in the same
chip: `verify_packing_le(A_NOISED_UNPACK, A_NOISED, 256)` asserts
the unpacked bytes repack (base-256 polyval) to the packed
`A_NOISED`, which is the lookup/commitment form. **This is
exactly the `noised_packed`-bus hook §4.C calls for** — Pearl
ties the accumulator inputs to the committed packed columns right
in the matmul chip, not in a separate pass.

### 9.3 Jackpot chip — the fold, as a bit-serial RAM machine

This is the part ai-pow lacks, and Pearl does **not** implement
it as the one-shot algebraic fold of §4.B. It is a scheduled
register machine (`circuit/chip/jackpot/{constraints,trace}.rs`,
chip doc `jackpot.rs:1-24`):

- **`CUMSUM_BUFFER`** (4×i32) — a cyclic FIFO. `IS_DUMP_CUMSUM_BUFFER`
  loads `CUMSUM_TILE`; otherwise `cumsum_buffer[i] ==
  next_cumsum_buffer[(i+1)%4]` (filled backwards from dump rows).
  Decouples matmul-tile timing from fold consumption so the two
  run concurrently across rows.
- **`BIT_REG`** (32 booleans, each `constraint_bool`) — a 32-bit
  register with three micro-ops, all checked on bit
  decompositions:
  - **LOAD** (`src=Jackpot`): `bit_reg = jackpot_msg[idx]
    .rotate_left(13)`. Constrained as `bitreg_rot1 · is_load ==
    Σ load_ind[i]·jackpot[i]` (`jackpot_idx` is a degree-2 one-hot
    over `0..32`: `0..15` LOAD, `16..31` STORE).
  - **XOR** (`src=Xor`): `bit_reg ^= cumsum_buffer[0]`, via
    per-bit `xor_bit(bit_reg[i], next_bit_reg[i])` reassembled to
    i32 == `cumsum_buffer[0]`, gated by `next_is_xor`.
  - **SHIFT3** (`src=Shift3`): `bit_reg.rotate_right(3·13=39)` —
    back-shift compensation, gated by `next_is_shift3`.
- **`JACKPOT_MSG`** (16×u32 RAM) — STORE writes `bit_reg` rotated
  by `0/13/26` (`IS_STORE0/1/2`) into the addressed word;
  non-stored words persist (`(1-store_ind[i])·(jackpot_msg[i] -
  next_jackpot[i]) == 0`); first row `jackpot_msg == 0`.
- **Back-shift compensation** (`pearl_program.rs:230-275` builder
  + SHIFT3 + Store0/1/2): because every LOAD applies an *extra*
  `rotate_left(13)`, contributions from earlier subtiles pick up
  spurious rotations from later subtiles' loads. The schedule
  pre-compensates at the last write of each `tid` in non-final
  subtiles so the *net* rotation matches `compute_jackpot`
  exactly.

All control bits (`is_load/xor/shift3/store0-2/dump_cumsum_buffer`,
`jackpot_idx`) come from the **packed control column**
(`chip/control_and_matid_packed.rs`) — Pearl's equivalent of our
`CONTROL_PREP`. So in Pearl the entire fold schedule is part of
the program/selector layout, which maps **directly onto our
CRIT-1 preprocessed-program requirement** (§4.A): the fold
schedule must live in the verifier-fixed VK, exactly as Pearl
puts it in the packed control column.

### 9.4 Message → hash, and end-to-end chaining

On the `IS_HASH_JACKPOT` finalize row Pearl's BLAKE3 chip reads
the `JACKPOT_MSG_RANGE` columns *directly* as its 16-word message
(`chip/blake3/constraints.rs:48`, `verify_buffer_advancement(...,
&jackpot, ..., is_msg_jackpot, ...)`), keyed with the
`COMMITMENT_HASH` PI. Scalar truth
(`api/proof_utils.rs:1078`): `blake3(64 LE bytes of jackpot_msg,
key=commitment_hash)`. Matmul and jackpot run in the **same
eval pass** — `matmul` returns `cumsum_tile`, `jackpot` consumes
it as an argument — so accumulator→fold→message→hash is one
continuous constrained dataflow in a single trace, with
difficulty/`comm_m` checked externally (Layer 1+). **This is the
same scoping our C2 already adopted**, and Pearl's "final
JACKPOT_MSG read as the hash message on the finalize row" is
exactly the role our keystone plays — except Pearl's is the
genuine 16-word folded state, not a `[0..4]=CUMSUM, [4..16]=0`
stand-in.

### 9.5 What this changes in our design — B1 vs B2

Pearl's complexity (cyclic `CUMSUM_BUFFER`, 32-bit `BIT_REG`
RAM, rotate-on-load, SHIFT3 back-shift compensation, degree-2
RAM addressing) is an **artifact of its concurrent register-
machine scheduling**, *not* inherent to the fold math. That
yields two implementation options for §4.B:

- **Option B1 — port Pearl's RAM machine.** Maximal structural
  fidelity to a proven design; but imports the back-shift
  compensation schedule and several new column blocks
  (`CUMSUM_BUFFER`, 32-bit `BIT_REG`, `JACKPOT_IDX` one-hot,
  `IS_LOAD/XOR/SHIFT3/STORE0-2/DUMP`). High width + schedule
  complexity.
- **Option B2 — the direct fold chip of §4.B.** One fold row per
  stripe: `M_next[slot] = rotl13(M_cur[slot]) ⊕ x_step`. No
  rotate-on-load, therefore **no back-shift compensation, no
  CUMSUM_BUFFER FIFO** — those exist in Pearl only to service the
  rotate-on-load RAM design. Far fewer columns and selectors.

**Recommendation: B2.** Our standing constraint is that SNARKs
are explicitly **not** byte-equivalent across chains — only the
mineable unit of work is. B2 is admissible iff its output
`JACKPOT_MSG` equals `compute_jackpot`/`TileState::fold`
bit-for-bit, which it does *by construction* (it is the literal
recurrence, no rotation bookkeeping to get wrong). B2 keeps the
byte-equivalent anchor (the plain `TileState`/`keyed_hash`,
16×u32-LE) intact while avoiding the single most error-prone
piece of Pearl's circuit (the compensation schedule). B1's only
advantage — trace-level parity with Pearl — is something we have
explicitly decided we do not need. **Decision recorded; revisit
only if a future merge-mining requirement demands trace
equivalence (it currently does not).**

This also simplifies §4.A/§7: with B2 the FoldChip can read
`CUMSUM_TILE` of the same/adjacent row (like Pearl's matmul
transition constraint) rather than introducing a buffered
handoff.

---

## 10. Cross-references

- Keystone & soundness boundary: `ZKP_SECURITY_REPORT.md`
  (HIGH-2 section + Bottom line), commit `15ba9a3`.
- Post-CRIT-1/keystone status: `GAP_AUDIT.md`.
- CRIT-1 program-pinning mechanism (the VK the schedule must
  extend): `composite_full_air.rs` `CompositeFullAirPinned` /
  `PROGRAM_COLS` / `extract_program`; commit `9ec529e`.
- Plain reference: `ai-pow/src/matmul.rs`
  (`compute_tile`, `TileState::fold`, `keyed_hash`,
  `compute_tile_from_slices`).
- In-circuit accumulator (exists): `ai-pow-zk/src/chips/matmul/`
  (`compute.rs::apply_cumsum_update`, `chip.rs`
  `MatmulCumsumChip::eval_composite`).
- C3 matrix-binding chain: `composite_full_air.rs` §"C3 (M52
  step 4.3+)"; `noised_packed` bus (M52 4.1).
- BLAKE3 32-bit XOR/rotate gadgets to reuse:
  `chips/blake3/round_ops.rs::xor_32_shift_if`.
- **Pearl reference implementation** (§9): scalar truth
  `pearl/zk-pow/src/circuit/chip/jackpot/helper.rs::compute_jackpot`;
  fold circuit `.../chip/jackpot/{constraints,trace}.rs` + chip
  doc `.../chip/jackpot.rs:1-24`; accumulator + committed-matrix
  packing `.../chip/matmul/constraints.rs`; back-shift schedule
  `.../circuit/pearl_program.rs:230-275`; message→hash
  `.../chip/blake3/constraints.rs:48`; scalar hash
  `.../api/proof_utils.rs:1078`; constants
  `.../circuit/pearl_program.rs:23-27` (`TILE_H=2`, `TILE_D=16`,
  `JACKPOT_SIZE=16`, `LROT_PER_TILE=13`).
- Memory: `ai_pow_zk_crypto_gaps` (HIGH-2 entry).
