# HIGH-2.2 — Honest matmul→fold→C4-hash chain: problem space & design

> **Status:** DESIGN (not started). Implementation tracked as task
> HIGH-2.2 / #95.
> **Class:** completeness / fidelity. **Not** a soundness or
> forgery hole — that part of HIGH-2 (the "C4 hashes a
> prover-free constant") was closed by the keystone, commit
> `15ba9a3`.
> **Authoritative neighbors:** `ZKP_SECURITY_REPORT.md` (HIGH-2
> section + Bottom line), `GAP_AUDIT.md` (post-CRIT-1 + keystone
> summary), memory `ai_pow_zk_crypto_gaps`.

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

### 4.C Committed-matrix end-to-end binding

**Problem.** `A_NOISED_UNPACK` / `B_NOISED_UNPACK` feeding the
matmul chip are not yet provably the C3-committed, CRIT-1-fixed
matrix bytes — only internally consistent.

**Design.** Route the matmul chip's `A_NOISED_UNPACK` /
`B_NOISED_UNPACK` inputs onto the existing `noised_packed` LogUp
bus (M52 4.1) so the same bytes that feed `HASH_A`/`HASH_B`
(C3: `IS_MSG_MAT·IS_NEW_BLAKE·(BLAKE3_MSG − base256(UINT8_DATA))`)
also feed the accumulator. Because `HASH_A`/`HASH_B` are
CRIT-1-pinned PIs, this closes the loop:

```
committed A,B ─C3→ HASH_A/HASH_B (pinned PI)
            └─noised_packed bus→ A/B_NOISED_UNPACK ─MatmulCumsumChip→ CUMSUM
                                                   ─FoldChip→ M
                                                   ─keystone→ JACKPOT_MSG
                                                   ─C4→ HASH_JACKPOT ─C2→ difficulty
```

After 4.C, an attacker cannot substitute a cheaper matrix to make
the fold land on a winning `M`: changing A/B changes `HASH_A`/
`HASH_B`, which are verifier-fixed.

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
(`difficulty_target(&params)`) are currently external. HIGH-2.2
should bind *which* tile is being attested (so a prover cannot
solve an easy tile and claim a hard one) — minimally by adding
the tile index to the pinned program/PI set, or by the
block-context derivation MED-3 will document. Track the precise
obligation with MED-3; HIGH-2.2 must at least not regress it and
should expose `tile_i/tile_j` as bound inputs.

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
