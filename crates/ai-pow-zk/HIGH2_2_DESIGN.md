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
| MED-3 — verifier-side `target` + `(tile_i,tile_j)` derivation contract (§4.E prerequisite) | ✅ **done & e2e-validated** (`ai_pow::zk_bridge::prove_and_verify_for_block` re-derives `target` from chain-pinned params; `tile_ij` derivation contract; unhardened primitives doc-commented; `prover.rs` on hardened path). +2 tests; ai-pow `--features zk` lib 66/0, e2e green |
| §6(b) — bind `FOLD_XSTEP ← ⊕CUMSUM_TILE ← committed A/B` in-circuit (StripeXorChip + sub-block-major sweep + Pinned keystone) | ✅ **done & e2e-validated for every single-Layer-0 params set** — TEST_SMALL **and** the rectangular LLM-FFN `llm_shape` shapes (G1+G2, `010ccd3`). `place_useful_work_chain` + `StripeXorChip` + `SX_IN==nxt.CUMSUM` binding + Pinned `FOLD_XSTEP==SX_XR[stripe]` keystone; ai-pow-zk lib 332/0; ai-pow `--features zk` green (lib 71/0, `end_to_end` 13/0, **`llm_shape` 5/0 now via §6(b)**). A *malicious* prover is forced through the real matmul for `X_STEP`. Commits `072d840`/`c63fbc1`/`69e420d`/`010ccd3`. |
| §4.E — attest the **actual solved tile** | ✅ **done** (`e7f59f7`): `prove_and_verify_for_block(ctx,params,found_idx)` decomposes via the MED-3 `tile_ij` contract; `high2_2_attests_real_solved_tile` (4 indices incl. corners, each byte-identical to the plain digest for *that* tile). All tiles share `difficulty_target(params)` so the index is not a PoW-soundness req; attesting the real tile is the substantive deliverable. |
| §6(b)-G1+G2 — generalize to all single-Layer-0 params (`r > TILE_D` chunking; `num_stripes ≤ STRIPE_MAX=64` lanes + pinned `FOLD_STRIPE_SEL`) | ✅ **done** (`010ccd3`): StripeXorChip `STATE_LEN=64`, `place_useful_work_chain` chunks `⌈r/TILE_D⌉`, ControlChip pins a 6-bit stripe index, keystone binds via `FOLD_STRIPE_SEL`. `high2_2_g1g2_chunked_and_wide_stripes` (r=32/num_stripes=32, debug-assertions ON) + `llm_shape` 5/0 via §6(b). `sx_bound = sweep_fits`. |
| §6(b)-G3 + §4.C residual — true PROD (`k/r=64`, sweep ≈ 2²⁰ ≫ one Layer-0) needs segmentation/M12; deep tile↔committed-store ≡ §4.C `noised_packed`-non-vacuity on sweep rows | ⬜ remaining (scoped, **not** a PoW-forgery hole): true-PROD takes the legacy path, §6(b) keystone `sx_bound=false` (verifier-set from trusted params, sound). G3 = M12-coupled (designed §4.C.4-G3). Soundness held by CRIT-1 + keystone + §6(a) + §6(b) for every single-Layer-0 params set. |
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

**§6(b) — CLOSED for every single-Layer-0 params set (DONE,
`072d840`/`c63fbc1`/`69e420d`/`010ccd3`; G1+G2 generalized it
beyond the primary geometry to the rectangular LLM-FFN
`llm_shape` shapes).** The per-stripe `X_STEP` fed to
the FoldChip is now **in-circuit forced** to equal the XOR of the
real `t×t` committed-matrix accumulator:
`CompositeTrace::place_useful_work_chain` places the sub-block-
major matmul sweep (spike GATE-2 row-ordering) + a co-located
`StripeXorChip` reduction; the matmul chip forces
`nxt.CUMSUM == compute_row(cur)`, `StripeXorChip::eval_composite`
binds `SX_IN == nxt.CUMSUM_TILE`, the chip XOR-reduces to
`SX_XR`, and the Pinned §6(b) keystone forces
`FOLD_XSTEP == SX_XR[stripe]`. So a *malicious* prover can no
longer fabricate `x_steps` — it must do the real matmul. The
honest bridge (`prove_and_verify_tiled`) places this for the
attested tile; byte-equivalent to the plain miner (GATEs 1/3).
Validated end-to-end through the production Route-A batch-stark
path (the debug-assertions-OFF hazard surface, root-caused once —
the signed-IN/CUMSUM encoding fix — and cleared):
`high2_2_fold_chain_pinned_logup`, `high2_2_useful_work_chain_unit`,
`chips::stripe_xor` 8/0; ai-pow-zk lib 331/0; ai-pow `--features
zk` green (lib 70/0, `end_to_end` 13/0, `llm_shape` 5/0,
byte-equivalence preserved). **§4.E — DONE (`e7f59f7`):** the
bridge attests the *actual solved tile* via the MED-3 `tile_ij`
contract (all tiles share `difficulty_target(params)` so the
index is not a PoW-soundness requirement; attesting a real tile's
genuine fold is the substantive deliverable —
`high2_2_attests_real_solved_tile`).

**G1+G2 — DONE (`010ccd3`), extending §6(b) to all single-Layer-0
params.** `StripeXorChip` `STATE_LEN = STRIPE_MAX = 64`;
`place_useful_work_chain` chunks the `r`-wide stripe dot into
`⌈r/TILE_D⌉` accumulating micro-steps (G1, `r > 16`); a 6-bit
fold-stripe index is pinned into `CONTROL_PREP` (§6(a) pattern)
and the keystone binds `FOLD_XSTEP == SX_XR[stripe]` via the
pinned one-hot `FOLD_STRIPE_SEL` (G2, `num_stripes ≤ 64`). The
rectangular LLM-FFN `llm_shape` shapes (`k/r = 20`) now run the
**full §6(b) binding** (`llm_shape` 5/0 via §6(b);
`high2_2_g1g2_chunked_and_wide_stripes` r=32/num_stripes=32
debug-assertions-ON clean). `sx_bound = sweep_fits`.

**Remaining (scoped; NOT a PoW-forgery hole).** Two tied items:
(1) **true PROD** (`k/r = 64`, but the chunked sweep ≈ 2²⁰ rows
≫ one Layer-0 STARK): takes the legacy `compute_tile_trace →
place_fold_chain` path with the §6(b) keystone gated **off** via
`sx_bound` — a value the *verifier* derives from the trusted
block params/height, never from the proof, so it is as sound as
CRIT-1 (a malicious prover cannot turn the binding off for a
params set the verifier runs it for). Closing it = **G3**
(segmentation + M12 recursion: `SX_XR`/`CUMSUM` as public
carry-in/out; designed in §4.C.4-G3). (2) **deep
tile↔committed-store**: that the swept `A_NOISED`/`B_NOISED` are
the *block's committed* A/B rows/cols reduces to the §4.C
`noised_packed`-non-vacuity on sweep rows (`place_matmul_step`
sets `MAT_ID = 0` / emits no `noised_packed` query — §4.C.10).
Both are tracked jointly. Soundness **meanwhile held by CRIT-1 +
the keystone + §6(a) + §6(b)** (the proof can't be forged
against the canonical program, the fold schedule is
verifier-fixed, and for **every single-Layer-0 params set** the
attacker *is* now forced through the real matmul for `X_STEP`).

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

#### 4.C.4-S §6(b) spike results — geometry & row-ordering DE-RISKED (2026-05-16)

Per user direction ("matmul-row placement spike; exhaustively
test after each sweep"). The two hardest §6(b) unknowns from the
"what would it take to finish" evaluation are now **resolved &
tested** (`ai_pow::zk_bridge` tests, `--features zk`):

- **GATE 1 — subtile-sweep geometry (`adf766a`).** The §4.0
  geometry "mismatch" (in-circuit `2×2×16` micro-tile vs the plain
  `t·t=64`-cell stripe accumulator, `MatmulParams::TEST_SMALL`:
  `t=8, r=4, k=64`) is a *solved, tested reduction*: sweeping ONLY
  the in-circuit primitive `compute_row`, stripe-major over
  `(t/TILE_H)²=16` sub-blocks × `num_stripes=16` stripes with the
  `r=4`-wide stripe zero-padded into `TILE_D=16`, reproduces
  `compute_tile_trace(...).x_steps` **bit-for-bit** (all 4 grid
  corners + interior) and `TileState::from_x_steps(swept)==M`.
- **GATE 2 — composite-AIR validity (`57b7cf9`).** The 256-row
  sweep placed via `place_matmul_step` as a **single continuous
  cumsum chain** with `is_reset` only on each 16-row sub-block
  run's first row **verifies through the unit `CompositeFullAir`**
  (tile (0,0) + far corner). Key analysis (validated on real
  data, in the exact area the prior multi-day bug lived): the
  matmul chip's always-on `when_transition` recurrence
  `nxt=(is_reset+is_update)·cur.dot+(1−is_reset)·cur.CUMSUM` is
  satisfied because every transition is `nxt==compute_row(cur)` by
  construction *and* the run-boundary carry into a reset row is
  discarded by the `(1−is_reset)=0` term — so 16 concatenated
  sub-block chains are chip-valid as one threaded chain. The
  per-stripe ⊕ of the *placed-trace* accumulator snapshots ==
  `compute_tile_trace`'s `x_steps`. Row budget `256 ≪ 8192`.

**Precisely-characterized remaining §6(b)+§4.E work** (now on a
de-risked foundation): sub-block `sb`'s accumulator *after stripe
`step`* lives at row `sb·16+step` (a non-adjacent set of 16 rows
per stripe). So the X_STEP binding is a **per-stripe ⊕-reduction
over 16 non-adjacent rows' CUMSUM == `FOLD_XSTEP[step]`** — a
cross-row argument (LogUp bus, or a co-located running-XOR
reduction column pinned by the §6(a) `CONTROL_PREP` schedule
pattern), *not* a single-row read. Plus: production
`prove_and_verify` wiring (place the sweep + the reduction on the
honest path) and Route-A batch-stark validation, then bind the
MED-3-derived `(tile_i,tile_j)` (resolved contract,
`zk_bridge::tile_ij`) to the swept accumulator's `row0/col0`
offsets. The arithmetic, the geometry, and the matmul-chip
interaction are no longer unknowns.

#### 4.C.4-G §6(b) production generalization — DESIGN (2026-05-16)

§6(b) is closed & validated for the **primary mining geometry**
(`num_stripes ≤ 16`, `r ≤ TILE_D`, sweep fits one Layer-0 STARK —
TEST_SMALL / the headline e2e). Three orthogonal sub-problems
separate "everything else" from a single fixed assumption; this
section designs each.

**Grounded scale (why this is three problems, not one).** The
direct in-circuit sweep is
`(t/TILE_H)² · num_stripes · ⌈r/TILE_D⌉` matmul micro-steps for
the attested tile (`TILE_H=2`, `TILE_D=16`):

| params | t | r | num_stripes=k/r | sub-blocks=(t/2)² | C=⌈r/16⌉ | sweep rows |
|---|---|---|---|---|---|---|
| TEST_SMALL | 8 | 4 | 16 | 16 | 1 | 256 ✅ fits |
| llm rect (`llm_shape`) | 8 | 4 | 20 | 16 | 1 | 320 — fits, needs only **G2** |
| PROD | 128 | 64 | 64 | 4096 | 4 | ≈ 2²⁰ — needs **G1+G2+G3** |

PROD also chunk-Merkle-hashes 4096×4096 i8 matrices (~2¹⁸ BLAKE3
rows for A alone), so a **single** Layer-0 STARK already cannot
hold a PROD proof *regardless of §6(b)* — PROD is inherently a
recursion/segmentation (M12) workload. Hence the design splits:

**G1 — inner-dimension chunking (`r > TILE_D`).** Each stripe's
`r`-wide dot is covered by `C = ⌈r/TILE_D⌉` accumulating
micro-steps over disjoint `≤TILE_D`-wide lane chunks. Per
sub-block: row 0 `is_reset`, all subsequent `(stripe,chunk)` rows
`is_update` (the matmul chip's existing reset/update recurrence
already sums chunks into the same `c_blk` cell and continues
accumulating across stripes — no chip change). `StripeXorChip`
`SX_IS_ACTIVE=1` only on each stripe's **last** chunk (lane =
stripe index), where `CUMSUM` = the post-stripe accumulator. Pure
trace-generation extension to `place_useful_work_chain`; relaxes
the current `assert!(r ≤ TILE_D)`. Cost ×C rows.

**G2 — `num_stripes` lanes (single-STARK params).** Replace the
`STATE_LEN = 16 = JACKPOT_SIZE` conflation (the FoldChip's
M-slots are Pearl-fixed at 16 and are a *different* concept from
per-stripe lanes) with a dedicated `STRIPE_MAX` (≥ max single-
STARK `k/r`; choose 64 for headroom incl. PROD-per-segment).
`StripeXorChip`: `SX_XR[STRIPE_MAX]`, `SX_LANE_SEL[STRIPE_MAX]`
(≈ +96 cols). The §6(b) keystone today indexes `SX_XR` via
`FOLD_SLOT_SEL` (= `step % 16`) which only equals the stripe for
`num_stripes ≤ 16`; generalize by adding a per-fold-row
`FOLD_STRIPE_SEL` (stripe one-hot, set by `place_fold_chain`) and
binding `Σ_{s<STRIPE_MAX} FOLD_STRIPE_SEL[s]·(FOLD_XSTEP −
SX_XR[s]) == 0`. `FOLD_STRIPE_SEL` must be **schedule-pinned**
(§4.C.6) — extend the §6(a) `CONTROL_PREP` pack with a
`⌈log2 STRIPE_MAX⌉ = 6`-bit stripe index (the §6(a) pattern,
NOT a wide preprocessed block — §4.C.8) and recompute the
one-hot in-circuit. With G1+G2, **`sx_bound` is universally
`true`** for all single-STARK params (the legacy gate / its
documented residual disappears for `llm_shape` and any
`num_stripes ≤ STRIPE_MAX` params). Implementable & exhaustively
testable now (rect ≈ 320 rows ≪ 8192) — the concrete next
integration to retire residual #107(1).

> **⚠️ CORRECTION (2026-05-17) — `M_S2_PEARL_EVALUATION.md` is
> authoritative on the Pearl relationship.** A Pearl
> implementation+paper evaluation established: **Pearl does NOT
> segment and has NO matmul spot-check.** Pearl caps params
> (whitepaper §4.8: `k ≤ 2¹⁶`, `k(h+w) ≤ 2²²`, `r ≤ 2¹⁰`) so one
> opened tile = one STARK, and recurses only *vertically* for
> ≤65KB-certificate compression (§4.7); its per-tile matmul
> truth is **already zero-gap** within those caps. The phrases
> below "*strictly stronger than Pearl's spot-checks*", "*G4 =
> Pearl §4.8 spot-check*", and "*Pearl-faithful interim
> spot-check*" are **inaccurate** and corrected in-place below.
> **Maintainer decision 2026-05-17 (γ):** Track-A PROD pursues
> the **Pearl-faithful path P-A/P-B** (param caps + raised
> Layer-0 ceiling + cost measurement); **G3 segmentation is
> fully designed but DEFERRED** (pursued only if a load beyond
> Pearl's `k = 2¹⁶` envelope is ever required). See
> `M_S2_PEARL_EVALUATION.md` §4–5.

**G3 — segmentation / recursion (DESIGNED, DEFERRED per the
2026-05-17 γ decision; only needed beyond Pearl's `k ≤ 2¹⁶`
envelope).** See **§4.C.4-G3** below + `M_S2_G3AB_DESIGN.md` for
the full design (segment model, carry vector, boundary-predicate
parameterization, the M12 recursion obligations, the soundness
theorem, the attack surface, and the G3a/G3b/G3c phasing).
Summary: the Layer-0 STARK proves a bounded **segment**; threaded
registers become public **carry-in / carry-out**; an M12
recursion verifies every segment proof + carry chaining +
per-segment CRIT-1 program + segment count/order. This is a
**novel architecture with no Pearl precedent** (Pearl proves the
whole opened tile in one STARK); it buys matmul truth for
per-tile `k` *beyond* Pearl's `2¹⁶` cap, at the cost of the
`Γ`/carry-stitch/`PROGRAM_ROOT`-across-tree/adjacency soundness
surface (`G3_RECURSION_AUDIT.md` P0–P6). The §6(b) per-row
constraints are unchanged — only boundary predicates are
parameterized.

**Pearl-faithful PROD path (P-A/P-B/P-C — the chosen γ path,
replaces the immediate M-S2/G3 milestone).** Match what Pearl
actually does:
- **P-A — ✅ LANDED 2026-05-17 (`crates/ai-pow/src/params.rs`).**
  Adopt Pearl §4.8's parameter caps as the ai-pow-zk PROD
  envelope, split into two verifier-enforced layers:
  - `MatmulParams::validate()` now enforces the **universal**
    Pearl §4.8 trace bound `k·(h+w) ≤ 2²²` (square tiles ⇒
    `k·2·tile ≤ 2²²`, const `PEARL_TRACE_BOUND`,
    `ParamError::TraceBoundExceeded`). This is THE
    one-tile-one-STARK guarantee and holds for **every**
    accepted puzzle (test *and* prod) — it is *why*
    segmentation (G3) is unnecessary. Already wired:
    `verifier::verify` + `prover` call `validate()`, so an
    un-provable puzzle is now rejected at the verifier.
  - New `MatmulParams::validate_prod_envelope()` = the §4.8
    **security** caps (`m,n ≤ 2²⁴`, `r ∈ {2⁵..2¹⁰}`,
    `16r ≤ k ≤ 4r²`, `64 | k`, `h·w ≥ 32`) layered on
    `validate()` — the **consensus admission rule** (the future
    M-C1 block-admission calls it). In-crate sub-envelope test
    profiles (`TEST_SMALL`, `r=4`) intentionally pass
    `validate()` but **not** `validate_prod_envelope()`; this
    test/consensus split is by design.
  Tests: all prod/LLM presets satisfy the envelope; the
  **envelope ⇒ one-STARK** theorem is property-tested over the
  whole §4.8 security band; each cap has a targeted reject test;
  `ai-pow --lib` + `--features zk` green (the new universal
  bound breaks nothing — every preset is ≤ 2²²). Result: every
  legal PROD puzzle provably fits **one** Layer-0 trace —
  segmentation is *defined away*.
- **P-B — ✅ LANDED 2026-05-17 (`crates/ai-pow/src/zk_bridge.rs`).**
  Params-driven Layer-0 sizing (the faithful analogue of Pearl's
  `degree_bits()`): `prove_and_verify_tiled` now sizes the trace
  via `expected_layer0_rows(params).required_trace_len()`
  (= `total.next_power_of_two().max(MIN_STARK_LEN)`) instead of
  the fixed `baseline_min()`. Sub-envelope test profiles round
  back up to `MIN_STARK_LEN` ⇒ **bit-identical to before, zero
  regression**; PROD-class params grow the trace, and the
  **decomposed `Layer0RowBudget`** (`mhash_a`, `mhash_b`,
  `sweep`, `store`, `fixed`) makes the scale blocker explicit.

  **Go/no-go finding (the γ measurement — pinned as tests
  `prod_matrix_hash_is_the_scale_blocker` /
  `prod_sweep_alone_fits_one_stark`):**
  - **The full-matrix chunk-Merkle is the blocker, NOT the
    matmul.** It re-hashes *all* of A and B in-circuit:
    `≈ 136·⌈|M|/1024⌉` rows per matrix. PROD (4096²): `mhash_a +
    mhash_b ≈ 4.46M` rows — **alone exceeding Pearl's one-STARK
    bound `2²² = 4.19M`** (GEMMA/QWEN: ≈ 14–18M, far worse).
  - **The §6(b) sweep — the matmul truth P-A guarantees — fits
    one STARK fine.** PROD sweep+store+fixed ≈ `next_pow2(1.18M)
    = 2²¹ ≤ 2²²`.
  - **Empirical prover cost** (`pb_prover_cost_scaling`, this
    dev box, parallel + `target-cpu=native`): **≈ 450 µs/row,
    linear** across 2¹³→2¹⁶ (3.7 s → 29.6 s). Projections: a
    full `2²²` trace ≈ **~31 min/proof**; the PROD sweep-only
    `2²¹` ≈ **~16 min** (feasible — amortized, SNARK only on a
    win, Pearl Remark 2.3; prod HW/Track-B optimizes further);
    the current full-matrix-hash `~2²³` ≈ **~63 min** AND past
    `2²²`.
  - **Conclusion / Pearl-faithful remedy → P-B.2 (next):** P-A
    proves the *matmul sweep* is one-STARK-bounded; P-B's
    measurement isolates that the **in-circuit full-matrix
    re-hash is what breaks one-STARK at PROD**. Pearl never does
    this — Pearl §4.6: the commitment `HASH_A/HASH_B` is a
    **public input** (computed once off-circuit / by the chain),
    and the circuit proves only the attested tile's revealed
    strips against it via in-circuit **Merkle authentication
    paths** (≈ `2t strips · log₂(num_chunks) · 8` rows ≈ a few
    thousand — negligible). That collapses `mhash_*` and brings
    the whole proof into the sweep-dominated `~2²¹` regime.
    **P-B.2 = replace the bridge's full-matrix `place_matrix_hash`
    with §4.6 commitment-as-PI + tile-strip Merkle-path opening**
    — the remaining work to make PROD genuinely one-tile-one-STARK
    (scoped as the next Pearl-faithful milestone; M-S1's
    `noised_packed` store already binds the swept strips, so this
    is the matrix-hash side only).
- **P-C** — vertical recursion *for the ≤65KB on-chain
  certificate only* (Pearl §4.7 / §5.1), reusing the M-S3/M-S4
  recursion+Tip5 substrate but **dropping** all of G3's
  `Γ`/carry/`PROGRAM_ROOT`/adjacency machinery.

**G4 — interim until P-B/P-C, via ai-pow's OWN spot-check (NOT
Pearl's).** Until the raised-ceiling single-tile path + cert
land, PROD-scale matmul-truth is carried by **ai-pow's own**
probabilistic light-verification — `MatmulProof.spot:
Vec<TileOpening>` + `params.spot_checks` (= 80 PROD;
`crates/ai-pow/src/proof.rs:54`, `params.rs:33`): the verifier
recomputes `spot_checks` **random** tiles vs. the committed M
states — **plus** the C3 commitment binding. This is **ai-pow's
design, not Pearl's** (Pearl §4.8 = param caps; Pearl §4.6
reveals one opened tile's strips; Pearl's per-tile truth is
zero-gap, so ai-pow's random-sample spot-check is *weaker than
Pearl, not parity*). It remains a deliberate, documented scoped
externality, **not** a forgery hole (CRIT-1 + keystone + §6(a)
hold unconditionally; §6(b) holds in-circuit for every params
set that fits one Layer-0 once G1+G2 land; M-S1 binds the sweep
inputs to a declared store).

**Ordering / status.** **G1+G2 — ✅ DONE & exhaustively tested
(`010ccd3`).** StripeXorChip `STATE_LEN = STRIPE_MAX = 64`;
`place_useful_work_chain` chunks the `r`-wide dot into
`⌈r/TILE_D⌉` accumulating micro-steps; `ControlChip` pins a 6-bit
fold-stripe index into `CONTROL_PREP` (bit 2^52, §6(a) pattern);
the keystone binds `FOLD_XSTEP == SX_XR[stripe]` via the pinned
one-hot `FOLD_STRIPE_SEL` for any `num_stripes ≤ STRIPE_MAX`.
`sx_bound = sweep_fits` (verifier-derived from trusted params).
Validated: `high2_2_g1g2_chunked_and_wide_stripes` (r=32 ⇒
chunks=2, num_stripes=32 — `x_steps == compute_tile_trace`
bit-for-bit + composite verify, debug-assertions ON clean);
`chips::stripe_xor` 8/0 incl. num_stripes∈{20,64}; control
`fold_stripe_mismatch_rejected`; ai-pow-zk lib 332/0; ai-pow
`--features zk` green incl. **`llm_shape` 5/0 now through §6(b)**.
The `sx_bound` legacy gate now fires only for true PROD. **G3** is
the M12 interface (designed in §4.C.4-G3, implemented with
recursion). **G4** is the precise interim scoping (documentation),
analogous to MED-3 — in force only for true-PROD until G3.

#### 4.C.4-G3 §6(b) PROD segmentation + M12 recursion — DETAILED DESIGN (2026-05-16)

> **The full implementation-ready recursion/aggregation spec is
> `crates/ai-pow-zk/G3_RECURSION_AGGREGATION.md`** (conceptual
> model → recursion primitive → carry vector → aggregation tree →
> `PROGRAM_ROOT` → soundness theorem → API → G3a/G3b/G3c).
> **It has been audited against the `Plonky3-recursion` reference
> — see `crates/ai-pow-zk/G3_RECURSION_AUDIT.md`, AUTHORITATIVE
> where they conflict.** The audit found the G3 logic sound but
> the spec wrong/optimistic on four points (F1 hash: our Layer-0
> is Tip5, the reference arithmetizes only Poseidon2 → Layer-0
> must migrate to Poseidon2-Goldilocks first; F2 Plonky3-rev
> mismatch; F3 inner program/VK not pinned by the lib — bespoke
> CRIT-1-across-tree glue; F4 aggregation binds no cross-child
> relation — the carry stitch is bespoke `connect` glue). G3c is
> gated on prerequisites **P0–P6** (audit §3); **G3a/G3b are
> unaffected and implementable now**; the G4 Pearl interim is
> authoritative until G3c **and** the (currently unaudited)
> recursion stack are audited.

**Why segmentation is mandatory for true PROD (grounded).** PROD
= `m=k=n=4096, tile=128, r=64`. One attested tile's §6(b) sweep
is `(t/TILE_H)² · num_stripes · ⌈r/TILE_D⌉ = 64² · 64 · 4 ≈
2²⁰` micro-step rows. Independently, the C3 chunk-Merkle of the
two `4096×4096` i8 matrices is ≈ `2 · 16384` 1 KiB leaves +
tree ≈ `2²⁰` BLAKE3 rows. A single Goldilocks Layer-0 STARK at
≈ several · 2²⁰ rows × ≈1900 cols is multi-gigabyte / hours —
infeasible. **PROD is inherently a recursion workload regardless
of §6(b)**; G3 is the project's instantiation of Pearl's
Layer-0 + aggregation architecture (M12).

**Segment model.** The global canonical PROD computation is the
(huge) trace `T` = `matrix-hash A ‖ matrix-hash B ‖ key-pin ‖
useful-work sweep ‖ fold ‖ jackpot-hash`. Fix
`S = MAX_SEGMENT_ROWS` (a power of two chosen from Layer-0
economics, e.g. `2¹⁸`). Partition `T` into `N = ⌈rows(params)/S⌉`
contiguous `S`-row segments `T_0 … T_{N-1}`. `rows(params)` and
hence `N` and each segment's `[k·S,(k+1)·S)` row range are a
**pure function of the chain-pinned params** (matrix dims → hash
rows; `tile`/`r`/`k` → sweep rows) — the verifier derives the
segmentation independently, exactly like the §6(a) schedule pin.
Each `T_k` is a normal `TOTAL_TRACE_WIDTH × S` composite trace
proved by the **same** `CompositeFullAirWithLookupsPinned` (the
AIR is *not* forked); only **boundary predicates** and **which
keystones fire** are parameterized by the segment's role.

**Carry vector `Γ` (the segment hand-off, public IO).** Exactly
the cross-row-stateful registers, exposed as Layer-0 public
inputs at each boundary:

```text
Γ = { CUMSUM_TILE[4]          (matmul accumulator, sub-block-major chain)
      SX_XR[STRIPE_MAX=64]    (StripeXor per-stripe XOR register)
      FOLD_STATE[16]          (FoldChip M, if the fold spans a boundary)
      MERKLE_A, MERKLE_B      (running chunk-Merkle CV/node-stack of the
                               C3 hash, if a matrix-hash spans a boundary)
      STARK_ROW_IDX_end       (monotonic-counter continuity) }
```

`Γ` is ≈ 90 field elements — a small per-boundary PI vector. The
canonical pinned **program is per-segment**: the verifier rebuilds
`program_k = canonical(params, k)` witness-free from the trusted
shape **and the segment index `k`** (`k` is pinned into the
segment's program/PI so segments cannot be permuted/duplicated).

**Boundary-predicate parameterization (the only AIR change).**
Every §6(b) (and matmul / Fold / BLAKE3 / keystone) *per-row*
constraint is **byte-identical** to the single-STARK case. Only
the boundary predicates are swapped, gated by a verifier-fixed
per-segment descriptor `(is_first, is_final, Γ_in)`:

- StripeXor `when_first_row: SX_XR == 0` → `SX_XR == Γ_in.SX_XR`
  (`Γ_in = 0` iff `is_first`). Same swap for the matmul
  `CUMSUM` and FoldChip `FOLD_STATE` first-row predicates.
- Each segment's last row exposes `Γ_out = {…}` as public
  outputs (`when_last_row: out_PI == register`).
- The §4.D + §6(b) keystones + the C2/HASH_JACKPOT difficulty
  binding fire **only on `is_final`'s last row** (identical
  scoping to the existing `sx_bound` / last-row predicates —
  this is the keystone-style boundary swap, now also indexed by
  segment role). A sub-block run / chunk run / fold chain split
  by a boundary is *transparent*: the matmul recurrence is local
  (`nxt == compute_row(cur)`), the `is_reset` schedule is the
  global pinned schedule (unaffected by where `S` falls), and
  the split is absorbed by `CUMSUM`/`SX_XR` carry — no special
  boundary case in the chips.

**M12 recursion obligations.** A recursion circuit (verify a
Layer-0 batch-stark proof inside a STARK; the M12 workstream)
that, for the `N` segment proofs of a PROD block:

1. **Verifies** each segment's Layer-0 proof against
   `program_k = canonical(params, k)` (per-segment CRIT-1 — the
   verifier rebuilds it from trusted shape; `k` pinned so a
   wrong-index or reordered program is rejected).
2. **Chains the carry:** `Γ_out(k) == Γ_in(k+1)` for all
   `k < N-1` (field-element equality of the ≈90-wide vector).
3. **Anchors the ends:** `Γ_in(0) ==` the canonical zero/initial
   boundary; the final segment's `Γ`/last row produces
   `HASH_JACKPOT`, the §4.D+§6(b) keystones hold there, and C2
   (`HASH_JACKPOT ≤ target`, target = MED-3
   `difficulty_target(params)`) is checked.
4. **Pins the count/shape:** `N == num_segments(params)` and each
   `program_k` is the canonical one for `(params, k)` — so a
   prover cannot drop the sweep segments, duplicate cheap
   segments, or reorder.
5. **Aggregates** to one succinct proof (binary aggregation tree,
   `O(log N)` depth — standard).

**Soundness theorem (zero probabilistic gap).** Segmenting the
single canonical trace `T` into contiguous `S`-row blocks: each
`T_k` satisfies the composite AIR for `program_k` with first-row
predicate `register == Γ_in(k)` and exposes
`Γ_out(k) = lastrow(register)`. Layer-0 soundness ⇒ each segment
proof attests this for its verifier-fixed `program_k` and the
claimed `Γ` PIs (which are Fiat-Shamir-bound STARK public IO,
*not* free). The recursion enforces (2)+(3)+(4). By induction the
concatenation `T_0‖…‖T_{N-1}` is *exactly* a single trace
satisfying the full composite AIR + §4.D/§6(b) keystones + C2 —
**identical soundness to the single-STARK §6(b)**, with the
*additional* recursion-enforced guarantees that the segments are
the right count, order, and per-segment program. Zero
probabilistic gap (`ε_FRI` only). *[CORRECTED 2026-05-17: the
original here claimed "strictly stronger than Pearl's shipped
soundness." That is wrong — Pearl's per-tile SNARK is **already
zero-gap** within its §4.8 param caps; G3 is not "stronger than
Pearl," it is a **different (novel) architecture** that extends
the zero-gap guarantee to per-tile `k` beyond Pearl's `2¹⁶`
cap. See `M_S2_PEARL_EVALUATION.md`. G3 is DEFERRED per the γ
decision.]*

**New attack surface G3 introduces (and the recursion closes).**
(a) *Segment drop/duplicate/reorder* → closed by `program_k`
pinning `k` + `N == num_segments(params)` + carry chaining (a
dropped sweep segment breaks `Γ_out==Γ_in`). (b) *Carry forgery*
→ `Γ` are FS-bound Layer-0 public IO, equality-checked by the
recursion. (c) *Wrong per-segment program* → per-segment CRIT-1
(verifier rebuilds `canonical(params,k)`). (d) *Mixed-block
splice* (segments from different blocks) → the per-segment
program/PI is anchored to the block via the existing C1
`JOB_KEY/COMMITMENT_HASH` PIs, carried/checked across segments.

**Orthogonality.** G3 preserves the §4.C `noised_packed` LogUp
*per segment* (batch-stark within a segment, unchanged); it does
**not** fix the §4.C-non-vacuity-on-sweep deep
tile↔committed-store residual — that is independent and tracked
jointly (#108). MED-3 (difficulty derivation) and the §6(a)
schedule pin are unchanged (the latter generalizes to per-segment
programs). *[CORRECTED 2026-05-17: "G4 (Pearl spot-check
externality)" is a misnomer. (i) Pearl has **no** spot-check
(Pearl §4.8 = param caps; per-tile truth is zero-gap). (ii)
ai-pow's `MatmulProof.spot`/`params.spot_checks` is **test-only
scaffolding — never used in production** (maintainer,
2026-05-17). So there is **no production spot-check interim** at
all: until the Pearl-faithful P-B/P-C lands, true-PROD
matmul-truth simply rests on the unmeasured assumption that a
single big trace is provable — which P-B will *measure*. The
production target is the zero-gap single-tile SNARK, not any
spot-check. `MatmulProof.spot` is a cleanup candidate (mark
test-only / remove). See `M_S2_PEARL_EVALUATION.md`.]*

**Alternative axis (noted, not the primary path).** A more
compact matmul argument (sumcheck/GKR-style, or a Freivalds
random-combination check) could cut the `2²⁰` enumeration itself
rather than segment it. That is a *different argument* with its
own (re-)validation and soundness surface; the recommended G3 is
**segmentation of the already-validated §6(b) enumeration**
(minimal new soundness surface, reuses the exhaustively-tested
StripeXor/matmul/Fold chips verbatim, Pearl-architecture-aligned).
The compact-argument route is recorded as a future optimization
axis, gated behind G3 shipping first.

**Phasing.**

- **G3a — boundary-predicate parameterization (M12-independent,
  implementable & testable now).** Add the verifier-fixed
  per-segment descriptor `(is_first, is_final, Γ_in)` and swap
  the StripeXor/matmul/Fold first-row + last-row predicates and
  the keystone gating to read it. Default = single segment
  (`is_first ∧ is_final`, `Γ_in = 0`) ⇒ **bit-identical to
  today, zero regression**. Test: artificially split a small
  TEST_SMALL sweep into 2 segments, thread `Γ` by hand, prove
  each segment + assert `Γ_out(0)==Γ_in(1)` and the
  concatenation's final `HASH_JACKPOT` equals the single-segment
  one. This delivers a *multi-segment-capable Layer-0* before
  M12.
- **G3b — segment-schedule derivation (M12-independent).**
  `num_segments(params)` + per-segment `program_k` + the
  `k`-pinned per-segment program (extend the §6(a) `CONTROL_PREP`
  pattern with a segment-index field, or a dedicated 1-column
  pinned `SEGMENT_IDX`). Verifier rebuilds `canonical(params,k)`.
- **G3c — the M12 recursion verifier + aggregation.** Recursive
  Layer-0-proof verification + the obligations (1)–(5). This is
  the heavy, recursion-stack-dependent part (the M12 workstream
  proper); G3a+G3b make Layer-0 G3-ready so G3c is purely the
  recursion.

#### 4.C.11 M-S1 — completing the matmul-input↔committed-store binding (DESIGN, 2026-05-17)

Track-A milestone **M-S1** ("§4.C sweep-input binding
non-vacuous", roadmap §7 / inflection **I2**). A code audit
(2026-05-17) found M-S1 is **the §4.C cryptographic core**, not a
localized tweak — the roadmap's "cheap, highest value-per-effort"
framing was over-optimistic on *effort* (the *value/scale-freedom*
stands; it is a multi-day build). Grounded findings:

- **Granularity.** `BYTES_PER_GOLDILOCKS = 4` ⇒
  `A_NOISED_LEN = TILE_H·TILE_D/4 = 8` packed cells (each =
  `polyval(4 i8, base 256)`); `A_NOISED_UNPACK_LEN = 32` i8 lanes
  (what `MatmulCumsumChip`'s dot multiplies). The `noised_packed`
  LogUp query (`composite_full_air_with_lookups::bus_emit::
  noised_packed`) binds only `A_NOISED_START[0..2]` — **2 of 8
  cells** (the module doc: this bus is a "proof-of-pattern"
  partial wiring, not a complete matmul-input commitment).
- **Missing pack-link (the core gap).** `MatmulCumsumChip::eval`
  constrains only the dot over `A_NOISED_UNPACK`/`B_NOISED_UNPACK`
  + the cumsum recurrence. **No constraint ties
  `A_NOISED_UNPACK` (dot inputs) to `A_NOISED[0..8]` (the
  bus-bound packed cells).** So even a hypothetical non-vacuous
  matmul row's committed-store binding (on `A_NOISED`) does *not*
  constrain what the dot actually multiplies. This decoupling —
  not the sweep `MAT_ID=0` — is the deepest part of the gap.
- **Producer mismatch.** M52 (`place_matrix_hash_*`) publishes
  *plain-byte* `NOISED_PACKED` entries for the BLAKE3-side
  self-query (M52 explicitly excluded low-rank noise). The matmul
  side needs the **noised** matrix (`a_prime`/`b_prime` = matrix
  + low-rank noise, per `InputChip`'s
  `NOISED_PACKED = polyval(MAT)+polyval(NOISE)`) published as
  canonical store entries, addressed by `A_ID`/`B_ID`, with
  `MAT_FREQ` multiplicities accounting for **every** sweep read
  (the 256-row sub-block-major × chunked sweep).
- **CRIT-1 (§4.C.6).** `A_ID`/`B_ID`/`MAT_FREQ` feeding the bus
  must be verifier-fixed (`AB_ID_PREP`/`CONTROL_PREP` are already
  `PROGRAM_COLS`; the §6(a)/G2 schedule-pin pattern extends to
  the per-sweep-row `A_ID`/`B_ID`).

**Target chain (what M-S1 must close).**
`committed A,B  → (C3/HASH_A,HASH_B + M52 chunk-Merkle)
→ canonical NOISED_PACKED store  → (noised_packed LogUp, A_ID/B_ID,
CRIT-1-pinned, MAT_FREQ)  → A_NOISED[0..8]/B_NOISED[0..8] on each
sweep row  → (NEW pack-link)  → A_NOISED_UNPACK/B_NOISED_UNPACK
→ (MatmulCumsumChip dot)  → CUMSUM  → (§6(b))  → … → C2`.
Today the arrow "`A_NOISED ↔ A_NOISED_UNPACK`" and the arrow
"committed store → A_NOISED on sweep rows" are both **absent**.

**⚠️ Empirical finding (2026-05-17) — the increments are NOT
independently Route-A-landable; M-S1 is one atomic landing.**
M-S1.1 (the pack-link) was implemented and tested: it is
per-row-correct and **passes the unit `CompositeFullAir`**
(incl. debug-assertions-ON) and its adversarial test. But the
full regression caught a **Route-A (production) regression**:
`high2_2_fold_chain_pinned_logup` fails at verify. Root cause —
the pack-link *forces* `A_NOISED` non-zero on the §6(b) sweep
rows (`A_NOISED == pack(A_NOISED_UNPACK) ≠ 0`); the **pre-existing
`noised_packed` LogUp query** on every `matmul_active` row then
consumes `(A_ID, A_NOISED[0..2])` with no producer supplying
those keys ⇒ **the LogUp bus no longer balances** ⇒ Route-A
rejects. The unit AIR has no LogUp so it passed; only the
production Route-A path exposes it. Therefore M-S1.1's
trace-side (writing real `A_NOISED`) is *coupled* to M-S1.3
(producer) + M-S1.4 (`A_ID` pin) + `populate_lookup_freq` via the
bus: **they must land atomically as one Route-A-valid change.**
The change was **reverted to green** (no half-wired state); the
pack-link constraint + its adversarial test are correct and are
re-applied as part of the atomic landing below. This confirms
§4.C.11's headline: M-S1 is the multi-day §4.C **core**, not a
sequence of independently-shippable steps — the "M-S1.1 landable
now" framing was the over-optimistic part, now empirically
disproven.

**Progress (2026-05-17, fix-forward per user directive — LANDED
& exhaustively validated).** The atomic landing closed Route-A
**green**: the pack-link makes `A_NOISED` provably the dot
inputs, the matmul query was widened to the **whole micro-tile**
(all `A_NOISED_LEN`/`B_NOISED_LEN` cells, chunked into
`A_NOISED_LEN/2` + `B_NOISED_LEN/2` 2-cell sub-queries), and a
pure producer store re-balances the bus. The §6(b) sweep's A/B
inputs are now **non-vacuously bound** (LogUp multiset) to a
declared canonical store — proven by the adversarial I2 test
(`high2_2_swept_tile_not_in_store_rejects`: a swept tile that is
NOT the published store leaves the bus unbalanced ⇒ Route-A
rejects). This upgrades "fold of *a* matmul" → "fold of *the
declared store's* matmul". **Residual (precise, separately
scoped): store ↔ committed-`HASH_A` is §4.C.2** (the
noise-derivation tie binding the producer's `a_prime`/`b_prime`
to the CRIT-1-pinned plain `A`/`B`); M-S1 does not pin the store
to `HASH_A`, so a prover may still declare *a* store — it just
can no longer sweep anything other than what it declared.

**Atomic landing (one Route-A-valid change — all parts LANDED &
green together):**

1. **Pack-link constraint** — ✅ LANDED & validated:
   `A_NOISED[c] == polyval(A_NOISED_UNPACK[4c..4c+4], 256)` ∀
   `c<A_NOISED_LEN` (and B), gated by `matmul_active`, degree ≤ 2,
   vacuous off matmul rows. `place_matmul_step` writes the packed
   cells. Unit `CompositeFullAir` green (incl. debug-assertions-ON
   via `high2_2_useful_work_chain_unit`); adversarial
   `matmul_pack_link_rejects_inconsistent_a_noised` green.
2. **Whole-micro-tile bus query** — ✅ LANDED:
   `bus_emit::noised_packed` emits `A_NOISED_LEN/2` A + `B_…/2`
   B 2-cell chunk sub-queries per matmul-active row (was a single
   2-cell prefix query — the prior vacuity). Each chunk
   `(A_ID, A_NOISED[2j], A_NOISED[2j+1])` is a value-as-key
   multiset lookup (`A_ID`/`B_ID = 0`, the constant store
   namespace; the *chunk value* is the discriminant).
3. **Producer store** — ✅ LANDED:
   `CompositeTrace::place_noised_store_row` (pure table entry —
   identical column writes to `place_matrix_staging_row` but
   `IS_MSG_MAT = 0`, so no self-query / no blake3-chip
   `BLAKE3_MSG` assertion) + `enumerate_noised_chunks`
   (deterministic, de-duplicated, byte-mirrors the sweep's
   `a_blk`/`b_blk`) + `place_noised_store` (one row per distinct
   swept chunk). The ungated table side
   (`(MAT_ID, NOISED_PACKED) × -MAT_FREQ` on every row) means a
   pure producer needs no selector. M52 plain-byte keys cannot
   unbalance: `populate_lookup_freq` routes ALL freq for a key to
   its first row, so a collision just concentrates `-MAT_FREQ`
   there (still net-zero).
4. **`populate_lookup_freq`** — ✅ LANDED: chunked A/B accounting
   mirrors the emission (`A_NOISED_LEN/2` + `B_…/2` keys per
   matmul row). Coverage net `noised_store_covers_every_swept_chunk`
   guards against enumerator↔sweep index drift.
5. **CRIT-1 / `A_ID` pin** — value-as-key with `A_ID = B_ID = 0`
   needs no per-row id column: store rows are no-op-program
   passthrough rows (CONTROL_PREP unchanged vs. the prior
   all-passthrough region), so the verifier's witness-free
   canonical-program rebuild is unaffected. The store *data*
   lives in non-pinned witness columns — exactly the §4.C.2
   residual boundary.
6. **Exhaustive validation** — ✅ ALL GREEN:
   `ai-pow-zk --lib` 335 pass / 0 fail / 22 ign (parallel);
   Route-A `high2_2_fold_chain_pinned_logup` green (parallel +
   debug-assertions-ON); unit `high2_2_useful_work_chain_unit`;
   coverage `noised_store_covers_every_swept_chunk`; **adversarial
   I2 `high2_2_swept_tile_not_in_store_rejects`** (rejects);
   `ai-pow --features zk` all green incl. MED-3 bridge roundtrip
   (`med3_prove_and_verify_for_block_roundtrips_and_derives_target`,
   real params through the store-wired `prove_and_verify_tiled`).

Soundness: CRIT-1 + keystone + §6(a) + §6(b) hold AND the §6(b)
sweep inputs are now multiset-bound to a declared store (M-S1).
Remaining to full §4.C: §4.C.2 (store ↔ `HASH_A` noise
derivation) — the precise, documented residual; not a forgery
hole (the swept work is still pinned to *something* the prover
committed to in the proof, and CRIT-1/§4.D/§6 hold independently).

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

**MED-3 derivation contract — ✅ RESOLVED 2026-05-16.** The
verifier-side derivation §4.E must consume is now concrete and
tested:

```text
target            = difficulty_target(params)                 // ai_pow::tile_hash, pure fn of chain params
(tile_i, tile_j)  = ai_pow::zk_bridge::tile_ij(found_idx, params)
                  = (found_idx / col_tiles, found_idx % col_tiles), or None if found_idx ≥ num_tiles()
production verify = ai_pow::zk_bridge::prove_and_verify_for_block(ctx, params)  // derives target itself
```

`prove_and_verify_for_block` re-derives `target` from chain-pinned
`params` (never accepts a counterparty target — MED-3 (ii)
closed; CRIT-1 closed (i)). `tile_ij` is the single source of
truth for the verifier-recomputed/checked tile coordinates.
`prover.rs` uses the hardened path; the unhardened primitives are
doc-commented with the obligation. See `ZKP_SECURITY_REPORT.md`
§MED-3.

**Status (2026-05-16): §4.E's *binding* is still entangled with
§6(b) — see "Remaining soundness scope".** MED-3 gives §4.E the
verifier-derivation contract, but a standalone `tile_i/tile_j`
public input is still **not** a sound closure on its own: nothing
in-circuit yet ties the hashed digest to a *specific tile's
committed-matrix accumulator* (the honest bridge places no matmul
subtile-sweep rows; `place_fold_chain` consumes prover-supplied
`x_steps`), so a free tile PI would be vacuous. The meaningful
binding requires §6(b)/§4.C.4 (place the subtile-sweep rows;
force `FOLD_XSTEP == ⊕CUMSUM_TILE` of the committed `A/B`) and
*then* binding the MED-3-derived `(tile_i,tile_j)` to that
accumulator's row/col offsets. Until §6(b) lands, HIGH-2.2 does
not regress MED-3 (MED-3 itself is resolved; the attested tile is
the honest bridge's choice; soundness held by CRIT-1 + keystone +
§6(a)), and the §4.E *binding* obligation is tracked jointly with
§6(b) as the one entangled cryptographic-core residual.

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
--lib` (expect 330+ green) and `cargo test -p ai-pow --features
zk` (the pinned mine-gate end-to-end suite).

### Dev-loop performance (the two Plonky3 levers)

Plonky3's prover hot paths (Goldilocks/Tip5 field arithmetic, the
Tip5 permutation, DFT, Merkle) are gated behind CPU vector
instructions and optional rayon parallelism that **rustc/Cargo do
not enable by default** (Plonky3 README §"Benchmarks" / §"CPU
features"). Both are deterministic — vectorized/data-parallel
with index-ordered reduction ⇒ **bit-identical proofs** (no
byte-equivalence / soundness / Fiat-Shamir impact):

1. **`parallel` feature — now default-on** for `ai-pow-zk`
   (`[features] default = ["parallel"]` → `p3-{dft,uni-stark,
   batch-stark}/parallel` → `p3-maybe-rayon`). No flag needed;
   `--no-default-features` for a serial comparison.
2. **`RUSTFLAGS="-Ctarget-cpu=native"`** — emits AVX/NEON for the
   field/Tip5/DFT kernels. Cannot be a Cargo feature; pass via
   env (or a `.cargo/config.toml [build] rustflags`, a
   maintainer/portability decision — non-portable binaries
   workspace-wide, so left to env by default).

Recommended dev-loop commands:

```
RUSTFLAGS="-Ctarget-cpu=native" cargo test -p ai-pow-zk --lib -- <filter>
# per-row check_constraints hazard pass (the debug-assertions-OFF trap):
RUSTFLAGS="-Ctarget-cpu=native -C debug-assertions=on" cargo test -p ai-pow-zk --lib -- <filter>
```

Measured: a prover-bound unit test
(`high2_2_useful_work_chain_unit`) went **≈32 s → ≈3.8 s (~8–9×,
~10 cores)** with both levers, proof still verifying. This is the
primary accelerator for the M-S1 fix-forward loop and the full
Route-A regression.

---

## 7. Road to production (current; supersedes the original sequencing)

> The original §7 (4.A→4.E→CRIT-1→docs) is **fully executed**
> (see the Progress table + §4.C.4-G/-G3). This is the live
> roadmap from *here* to "production with arbitrary useful
> inference loads". Three independent tracks; production needs
> all three — the bulk of work to date is Track A only.
>
> **For the end-to-end production *sequence* targeting the real
> shipped model (`Llama-3.1-8B-Instruct-pearl`), incl. the
> phased critical path (P-B.2.3 → P-B.2.4 → §4.C.2 → P-C),
> byte-equivalence reconciliation, the INT-only/FP8 scoping, and
> the (vague) vLLM + consensus integration: see
> `PRODUCTION_ROADMAP.md`** — the execution wrapper that ties
> this Track-A table together with Tracks B/C and correctness.

**Done & validated (the foundation).** C1–C4, CRIT-1 (program
pin), HIGH-2 keystone, MED-3 (verifier-derived target/tile),
§4.A–§4.D, §4.C Route-A, §6(a) (fold-schedule pin), **§6(b)
G1+G2** (X_STEP↔CUMSUM↔fold forced in-circuit for *every
single-Layer-0 params set* — TEST_SMALL **and** rectangular
LLM-FFN `llm_shape`), §4.E (attest the real tile), **M-S1** (§4.C
sweep-input ↔ declared-store multiset binding, non-vacuous —
adversarial I2 rejects a swept tile ∉ store). Byte-identical
to the plain Pearl miner throughout. **Guarantee today: for any
shape that fits one Layer-0 STARK, a malicious prover *must* do
the real matmul over the noised tile it declared in the store
(store ↔ committed `HASH_A` = §4.C.2, the one remaining §4.C
tie).**

### Track A — cryptographic soundness at arbitrary scale

| Milestone | Feature / guarantee it adds | Depends on |
|---|---|---|
| **M-S1 · §4.C sweep-input binding non-vacuous** — ✅ **LANDED & validated 2026-05-17** (design + landing record: **§4.C.11**). Pack-link + whole-micro-tile chunked `noised_packed` query + pure producer store + chunked `populate_lookup_freq`; value-as-key (`A_ID=B_ID=0`) so no per-row id / no CONTROL_PREP change. Route-A green (parallel + debug-assertions-ON), adversarial **I2** `high2_2_swept_tile_not_in_store_rejects` rejects, `ai-pow-zk --lib` 335/0/22, `ai-pow --features zk` green incl. MED-3 bridge roundtrip. | Upgraded "fold of *a* matmul" → "fold of *the declared store's* matmul". **Inflection I2 reached.** Remaining §4.C tie: store ↔ committed `HASH_A` = **§4.C.2** (noise derivation), the precise documented residual. | DONE |
| **M-S2 · P-A + P-B (Pearl-faithful PROD path)** — ✅ **LANDED 2026-05-17** (eval: `M_S2_PEARL_EVALUATION.md`; G3 design retained for the beyond-Pearl-envelope case: `M_S2_G3AB_DESIGN.md`) | **P-A ✅:** Pearl §4.8 envelope in `MatmulParams` — universal `k(h+w) ≤ 2²²` one-STARK bound in `validate()` (auto-enforced via `verify`/`prover`) + `validate_prod_envelope()` consensus rule. **P-B ✅:** params-driven trace sizing (`expected_layer0_rows` ≈ Pearl `degree_bits`) + go/no-go measured (≈450 µs/row linear). **Finding:** the §6(b) matmul *sweep* is one-STARK-bounded (PROD ≈ 2²¹ ✓, P-A holds); the **full-matrix in-circuit re-hash is the blocker** (PROD `mhash` ≈ 4.46M > 2²²) ⇒ **P-B.2** next. Zero regression (sub-envelope profiles bit-identical). | DONE |
| **M-S2.2 · P-B.2 — §4.6 strip-opening matrix-hash** (design + D1–D4: `P_B2_STRIP_OPENING_DESIGN.md`). Replace the bridge's full-matrix `place_matrix_hash` with `HASH_A/HASH_B` = off-circuit PI commitment + in-circuit BLAKE3-tree auth-path opening of only the attested tile's `t` rows of A / `t` cols of B (`O(t·k)`, size-independent). **P-B.2.0 ✅ DONE 2026-05-17** (`blake3_tree.rs`: true-tree walker + strip-opening + KAT vs `blake3::Hasher` for all chunk counts incl. non-pow2 to 1000; adversarial). **D1 finding: the hypothesised latent fidelity gap does NOT exist** — `place_matrix_hash`'s pairwise loop already == BLAKE3's true tree for *all* counts (proved in-circuit 1..=31, 100) ⇒ **P-B.2.1 (realign) subsumed/no-op**. **P-B.2.2 ✅ DONE 2026-05-17** (`composite_trace.rs::place_matrix_strip_opening` + helpers): leaf layer over only the opened chunk range + true-tree auth fold with witness siblings; `IS_HASH_A/B` on the recomputed-root row ⇒ **reuses the unchanged C3 binding, no AIR change**. Tests: in-circuit honest-equivalence (== full `place_matrix_hash` == `blake3::Hasher`, all ranges/non-pow2/lone-chunk), positive AIR prove/verify, adversarial tampered-strip + forged-sibling reject. Remaining: P-B.2.3 (verifier-fixed schedule) → .4 (bridge swap). | Collapses the `mhash` term ⇒ whole PROD proof in the `~2²¹` sweep regime ⇒ genuinely **one tile = one STARK** (Pearl §4.6 faithful). M-S1 binds the swept strips, so this is matrix-hash-side only. | P-B.2.0/.2 DONE; .3/.4 remain |
| **M-S3 · P0 vendor Plonky3-recursion** + align Plonky3 rev in the vendored tree | Audit-stable, owned recursion substrate for the **vertical certificate compression** (P-C), resolves F2/F7. | reference (cloned) |
| **M-S4 · P1 `tip5-circuit-air`** from `nockchain-math::tip5` + Tip5 challenger/MMCS arms + native≡in-circuit cross-test | The recursion verifier can verify our **Tip5** Layer-0 proofs at all; Layer-0 unchanged ⇒ the 120-bit FRI sweep preserved. | M-S3 |
| **M-S5 · P-C vertical-recursion certificate** (Pearl §4.7/§5.1: compress *one* Layer-0 proof to a ≤65KB on-chain cert; 3-layer vertical recursion, NOT segment-aggregation) | Succinct on-chain PoUW certificate at PROD scale, **zero probabilistic gap, parity with Pearl** (no `Γ`/`PROGRAM_ROOT`/adjacency surface — that is the deferred G3c only). **Inflection I3.** | M-S2,3,4 |
| **M-S6 · Independent crypto audit** of (a) the 7-round Tip5 variant (now also in-circuit), (b) the vendored+extended recursion stack | Removes the "experimental/unaudited" gate. **Inflection I4.** | M-S5 |
| **(deferred) G3a/G3b/G3c** — carry-vector segmentation + recursive *aggregation* | Matmul truth for per-tile `k` **beyond Pearl's `2¹⁶` cap** (strictly more ambitious than Pearl). Pursued **only if** a concrete load past the Pearl envelope is required. Design preserved in `M_S2_G3AB_DESIGN.md`. | deferred (γ decision) |

### Track B — prover economics

| Milestone | Feature / guarantee | Depends on |
|---|---|---|
| **M-P1 · PROD-scale profiling & economics** (segment ≈2²⁰ + chunk-Merkle ≈2²⁰; parallel segment proving; recursion scheduling/memory; bench at real FFN shapes — GAP_AUDIT P1/P3/P5/P6) | The prover is *economically mineable* at production loads, not merely sound. Co-gates **I4**. | M-S5 (for the recursion cost), partial now |

### Track C — consensus & useful-work-economics integration

| Milestone | Feature / guarantee | Depends on |
|---|---|---|
| **M-C1 · Block-certificate format** — make the aggregate root proof + PIs the consensus artifact (today the plain `MatmulProof` is the cert and the SNARK is an out-of-band correctness gate) | The chain actually *consumes* the ZKP. **Inflection I-consensus.** | M-S5 |
| **M-C2 · Chain-wire MED-3 derivation** — the node feeds chain-pinned params (header → `difficulty_target`/`tile_ij`/`PROGRAM_ROOT`) into `verify_pow_aggregate` | The verifier-side derivation is consensus-enforced, not test-only. | M-C1 |
| **M-C3 · Merge-mining dual-accept** — same mineable unit accepted by Nockchain *and* Pearl once PoW difficulty is hit (byte-equivalence already preserved; the dual cert/verify path is the integration) | Honors the standing merge-mining invariant in production. | M-C1 |
| **M-U1 · Useful-work sourcing / marketplace** — *who* supplies A,B (real model weights/activations), how the network agrees a load is genuine inference vs adversarial busywork, pricing, anti-grief | Turns "a correct-matmul proof" into **arbitrary *useful inference* loads**. **Inflection I5** — largely *outside* the ZKP; the defining milestone for the user's actual phrasing. | protocol/econ design |
| **M-U2 · Fidelity envelope** — characterize which inference workloads the low-rank-noised INT8 tiled matmul faithfully serves (error budget; which models/layers; the requester de-noises via known seeds) | Bounds & specifies the "arbitrary" in "arbitrary useful inference". | M-U1 |
| **M-U3 · Result delivery & consumer trust** — the de-noised result reaches the inference consumer with a cheap attestation derived from the PoW aggregate | The useful output is actually *usable & trusted* by the requester. | M-C1, M-U2 |

### Inflection points (qualitative thresholds)

- **I1 — PASSED:** single-Layer-0 §6(b) → honest-fidelity becomes
  *malicious-prover-forced* for bounded shapes.
- **I2 — M-S1 ✅ REACHED (2026-05-17):** the bound matmul is now
  *the prover's declared-store* matmul — the §6(b) sweep inputs
  are multiset-bound (LogUp) to a canonical producer store, so a
  matrix-swap *on the sweep* is impossible (adversarial I2 test
  rejects). Scale-free. The matmul-swap surface is reduced to the
  store ↔ committed-`HASH_A` tie = **§4.C.2** (noise derivation),
  the single precise residual to full §4.C. See §4.C.11 for the
  landing record. *Was* the multi-day §4.C core (the pre-audit
  "cheap" framing was wrong); now landed.
- **I3 — M-S5 (P-C):** bounded → *production scale* with zero
  probabilistic gap, **at parity with Pearl** (Pearl's per-tile
  SNARK is already zero-gap within its §4.8 caps; the
  Pearl-faithful P-A/P-B/P-C reaches the same — *not* "strictly
  stronger than Pearl," corrected 2026-05-17). Below I3 there is
  **no production fallback** — `MatmulProof.spot` is test-only
  (NOT a PROD mechanism); P-B must *measure* single-big-trace
  provability. ("Beyond-Pearl-envelope" / `k > 2¹⁶` is the
  deferred G3 axis, not I3.)
- **I4 — M-S6 + M-P1:** experimental → *consensus-deployable*
  (audited + economical + base proving system frozen). **The
  production gate.**
- **I-consensus — M-C1/M-C2:** the ZKP becomes the consensus
  artifact rather than an out-of-band gate.
- **I5 — M-U1:** "correct-matmul proof" → "*arbitrary useful
  inference* economy". The defining inflection for the headline
  goal; mostly a protocol/economic layer the ZKP enables but
  does not itself provide.

### Cross-cutting integration seams

Vendored-recursion ↔ ai-pow-zk Plonky3 rev; `nockchain-math::tip5`
as the *single* Tip5 source (native + in-circuit, cross-tested);
`composite_proof` Route-A ↔ G3a `SegmentPI` boundary predicates;
`PROGRAM_ROOT(params)` / `difficulty_target` / `tile_ij` ↔ the
node's block-context derivation (MED-3 discipline → consensus);
aggregate root proof ↔ block-certificate ↔ merge-mining
dual-accept; PoW aggregate ↔ useful-result delivery channel.
None of these seams are wired yet; M-S1 + M-S2 are unblocked and
independent of all the others.

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
- **Resolved (2026-05-16):** MED-3 was landed first (the user's
  directive) — `prove_and_verify_for_block` + the `tile_ij`
  derivation contract. §4.E now consumes MED-3's verifier-derived
  `(tile_i,tile_j)`; the remaining §4.E work is purely the
  in-circuit *binding* of that value to the accumulator (entangled
  with §6(b)).

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
