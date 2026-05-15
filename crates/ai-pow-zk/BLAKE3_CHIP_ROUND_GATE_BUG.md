# BLAKE3 chip: `verify_round` leading-boundary gate bug

**Status:** open, latent (does not break any current shipping
path), but **blocks C4 / HASH_JACKPOT and F1-deep completion**.
Diagnosed 2026-05-15 by code reading + bisect.

## One-sentence statement

`Blake3Chip::eval_at` gates the cross-row BLAKE3 round constraint
with `1 − is_last_round` (a property of the *current* row) instead
of the documented-correct `1 − next_is_new_blake` (a property of
the *next* row), so the constraint wrongly fires on the
non-blake row immediately preceding a blake block and demands
`next.STATE0 == Round(that row's state)`, which a fresh blake
init state cannot satisfy. A blake compression therefore only
verifies when it is contiguous from trace row 0 (no preceding
in-trace row → no leading boundary).

## Symptom (bisect, reproducible)

A bare 8-row block from `place_blake3_hash_with_selectors`
(no jackpot, no extra selectors):

| placement | result |
|---|---|
| `row_start = 0` (contiguous from start) | ✅ prove+verify |
| `row_start = 100` (mid-trace) | ❌ `OodEvaluationMismatch` |
| `row_start = height − 8` (trace-terminal) | ❌ `OodEvaluationMismatch` |

`place_matrix_hash_a/b` works only because it lays its blocks
contiguously starting at row 0.

## Root cause (exact)

`crates/ai-pow-zk/src/chips/blake3/chip.rs`, `Blake3Chip::eval_at`:

```rust
// lines ~203-214
let is_round_active: AB::Expr =
    <AB::Expr as PrimeCharacteristicRing>::ONE - is_last_round.clone();
{
    let mut tb = builder.when_transition();
    verify_round::<_>(&mut tb, &states, &msg, is_round_active);
}
```

where (lines ~178-191) `states[4]` is **the next row's STATE0**
(`s4 = nxt[state_start..]`). So `verify_round` is a *cross-row*
constraint: "applying the BLAKE3 round function to this row's
STATE0..3 with this row's message yields the next row's STATE0,"
enforced whenever `is_round_active = 1`.

`verify_round`'s own contract (`round_air.rs:235-238`) says:

> All constraints are gated by `is_activated` — typically
> `next_is_same_blake = 1 − next_is_new_blake`, which fires the
> round only when the next row is a continuation of the current
> BLAKE3 hash.

The caller passes `1 − is_last_round` instead. The two gates
diverge exactly at a block's **leading boundary**:

Let row `R` be a baseline (all-zero) or otherwise non-blake row,
and `R+1` the first row of a blake block (`is_new_blake = 1`,
STATE0 = the fresh init state `cv ‖ IV ‖ tweak`, non-zero).

| gate | value on row `R` | effect |
|---|---|---|
| **wrong:** `1 − is_last_round(R)` | `1 − 0 = 1` | round **active** → asserts `R+1.STATE0 == Round(R.state, R.msg)` |
| **correct:** `1 − is_new_blake(R+1)` | `1 − 1 = 0` | round **disabled** (R is not part of the hash) |

With the wrong gate, on row `R` the constraint becomes
`Round(0-state, 0-msg) == (R+1)'s non-zero blake-init STATE0`,
which is false → `OodEvaluationMismatch`.

### Why row-0 placement escapes the bug

There is no row `−1`, so there is no `R → R+1` leading-boundary
transition. The first transition is `row0 → row1`, both inside
the block (a genuine round). The block's **trailing** boundary
(`row7 → row8`, row8 baseline) is disabled because the wrong gate
`1 − is_last_round` happens to be `0` on the finalize row
(`is_last_round = 1`). So the wrong gate is accidentally correct
for the *trailing* edge and only wrong for the *leading* edge —
which a row-0 block does not have.

## The fix

The round must be active only when **both**:

1. this row is a round row (not the finalize row): `1 − is_last_round`
2. the next row continues the same blake (not the start of a new
   block, and not a non-blake row): `1 − next_is_new_blake`

i.e. gate `verify_round` with the product

```rust
let next_is_new_blake: AB::Expr = nxt[off.is_new_blake_col].into();
let is_round_active: AB::Expr =
    (AB::Expr::ONE - is_last_round.clone())
        * (AB::Expr::ONE - next_is_new_blake);
{
    let mut tb = builder.when_transition();
    verify_round::<_>(&mut tb, &states, &msg, is_round_active);
}
```

Notes / subtleties for whoever implements this:

- `nxt[off.is_new_blake_col]` is already reachable: `nxt =
  main.next_slice()` is in scope and `off.is_new_blake_col` is a
  field of `Blake3Offsets`. `when_transition()` guarantees a next
  row exists.
- Degree: `is_last_round`, `next_is_new_blake` are boolean
  columns, the round body is degree-2-ish; the extra factor adds
  one degree. Confirm it stays within the Plonky3 quotient budget
  at `log_blowup = 2` (TEST_PEARL) — the M52/MM2 note records the
  empirical degree-≈5 ceiling; this should be fine but must be
  re-benched.
- **Leading-boundary state hygiene:** with the round disabled at
  `R → R+1`, nothing constrains `R+1.STATE0` *via the round*. The
  init constraint must carry it: `verify_init_state` is gated by
  `is_new_blake` (`round_air.rs:380-430`) and pins
  `STATE0 = cv ‖ IV ‖ tweak` on the block's first row. That is
  already correct and independent — good. Just verify no other
  constraint silently assumed the (now-removed) leading round
  link.
- **Trailing boundary unchanged:** `1 − is_last_round` is still a
  factor, so `row7 → row8` stays disabled exactly as today.
- After the fix, also re-check the C3 constraint
  (`IS_MSG_MAT · IS_NEW_BLAKE · …` in `composite_full_air`) and
  the C1/C4 selector-gated bindings still hold for non-row-0
  blocks — they are per-row, so they should, but a non-row-0
  blake block is a new trace shape and deserves a regression.

## Why this blocks C4 / F1-deep

Pearl's `structure_jackpot_blake` (`pearl_program.rs:195`) places
the jackpot-hash BLAKE3 block at `num_rows − ROUNDS_PER_BLAKE_INSTRUCTION`
— **trace-terminal**, the worst case for this bug (it has a
leading boundary AND is at the end). The C4 binding
`HASH_JACKPOT = BLAKE3(JACKPOT_MSG, key=COMMITMENT_HASH)` requires
exactly such an end-placed block (so the jackpot chip's
`when_transition` is vacuous on its last row — see
`GAP_AUDIT.md §C4` obstacle (a)). Until this gate bug is fixed, no
end-placed or mid-placed blake block verifies, so `HASH_JACKPOT`
cannot be made a non-vacuous bound PI.

The `place_jackpot_hash_block` trace generator (final-8-rows
jackpot+blake co-activation) was built and its jackpot-side
constraints check out; it was reverted unshipped because this
chip bug makes the blake side unverifiable regardless. Re-land it
once the gate is fixed.

## Impact on currently shipping paths

**None.** Every production / tested blake placement
(`place_matrix_hash_a/b`, the M52 chunk-Merkle, the C1 key-pin
rows which carry no blake activity) is either row-0-contiguous or
blake-inactive. The bug is latent: it only manifests for a blake
block that is not contiguous from row 0, which nothing ships yet.
It is purely a **blocker for future work** (C4, and any design
that needs blake compressions placed at arbitrary row offsets,
e.g. Pearl's interleaved `structure_matmul_in_stark` schedule).

## Suggested validation after the fix

1. Re-enable the bisect: a bare blake block at `row 100` and at
   `height − 8` must prove+verify.
2. Re-land `place_jackpot_hash_block` + the `c4_jackpot_hash_block_binds_hash_jackpot`
   test (non-vacuous `HASH_JACKPOT`, prove+verify).
3. Full `ai-pow-zk` regression (LogUp variant included — the
   round gate change touches the most-exercised chip).
4. Re-run `scripts/profile_f1.sh run` and confirm no prove-time
   regression from the added gate factor.
