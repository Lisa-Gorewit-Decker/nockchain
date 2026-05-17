# ai-pow / ai-pow-zk — gap audit & remaining work

Audit date: 2026-05-15. Scope: `crates/ai-pow` (plain PoUW
puzzle) + `crates/ai-pow-zk` (Plonky3 SNARK wrapper). Based on a
walk of the current source — **not** the stale root-level plan
doc (see "Corrected stale assumptions" below).

Severity: 🔴 blocks a correctness/soundness claim · 🟠 limits
production readiness · 🟡 polish / observability.

## Corrected stale assumptions

The earlier paper-alignment evaluation (`evaluate-the-existing-int8-*`
plan) flagged three structural PoUW gaps. **Two are now closed**
in the current code; do not re-report them:

- ✅ **Low-rank noise.** `prng.rs` implements `E = E_L·E_R`,
  `F = F_L·F_R` per Pearl §4.4 — `E_L`/`F_R` signed 6-bit
  `[-32,31]`, `E_R`/`F_L` ±1 choice matrices, `rank_mask = r-1`.
  `noise_rank` is load-bearing, not decorative.
- ✅ **Step-bound tile state.** `matmul.rs::TileState::fold` does
  `rotate_left(13)` XOR-fold along the k-axis (Pearl §4.5);
  `compute_tile` iterates per r-stripe. Test
  `tile_state_fold_depends_on_step_order` pins the order
  dependence.
- ◑ **Miner-chosen A,B with binding commitments.** M52 — landed
  at TEST_SMALL, see `M52_MATRIX_BINDING.md`. Residual gap below.

## Cryptographic gaps

> **C1–C4 RESOLVED (2026-05-15).** Commits a6f8480 (C1+C4),
> 4e9d79d (C2), 1a67aa1 (C3). The original C1 framing
> ("bind comm_m / difficulty / found-tile in-circuit") was
> over-stated relative to Pearl: Pearl's Layer-0 STARK
> (`pearl_circuit.rs:12-22`) binds `JOB_KEY, COMMITMENT_HASH,
> HASH_A, HASH_B, HASH_JACKPOT` and checks the difficulty
> inequality + comm_m membership **externally** by design.
> The resolutions adopt Pearl's canonical scoping.

### ✅ C1 — chain-binding PIs (RESOLVED, `a6f8480`)

`CompositePublicInputs` now carries Pearl's canonical set:
`+ job_key(8) + commitment_hash(8) + hash_jackpot(8)`
(NUM_PUBLIC_VALUES 36 → 60). Selector-gated AIR constraints
(same proven form as M52's HASH_A binding):
`IS_USE_JOB_KEY·(CV_IN−PI_JOB_KEY)=0`,
`IS_USE_COMMITMENT_HASH·(CV_IN−PI_COMMITMENT_HASH)=0`. This
ties the proof to *this block's* header-derived κ and `s_a`
noise seed — the proof is now anchored, not unbounded.
`comm_m` / found-tile membership stays external (Pearl Layer-0
does the same; also consistent with ai-pow's existing
spot-check protocol).

### ✅ C4 — HASH_JACKPOT bound (RESOLVED, `a6f8480`)

`IS_HASH_JACKPOT·(CV_OUT[i]−PI_HASH_JACKPOT[i])=0`. The
tile-state keyed hash is now a bound PI (Pearl
`pearl_circuit.rs:22` constraint d). Derivation tests confirm
`derive_from_matrix` reads the right cells; full prove+verify of
a HASH_JACKPOT trace needs the F1 jackpot→blake3 chain
(IS_HASH_JACKPOT is multiplexed as the jackpot chip's
is_active) — its constraint form is byte-identical to the
end-to-end-proven HASH_A binding.

### ✅ C2 — difficulty check (RESOLVED, `4e9d79d`)

`composite_verify_pow(cfg, proof, pis, target)` verifies the
STARK then checks the **bound** `HASH_JACKPOT` PI against the
32-byte LE target. Pearl-faithful: Pearl's Layer-0 STARK does
*not* do this in-circuit either (external by design); an in-AIR
256-bit comparator was rejected as strictly-more-than-Pearl,
costly, and absorbed by M12 recursion anyway. Soundness rests
on HASH_JACKPOT being a bound PI (C4).

### ✅ C3 — MAT_UNPACK ↔ BLAKE3_MSG (RESOLVED, `1a67aa1`)

`IS_MSG_MAT·(BLAKE3_MSG[j]−Σ_b UINT8_DATA[4j+b]·256^b)=0`.
Completes the binding chain
`store ─noised_packed─ MAT_UNPACK ─i8u8─ UINT8_DATA ─C3─
BLAKE3_MSG → CV_OUT → HASH_A`. Negative test proves the
constraint bites. Architectural finding: BLAKE3_MSG is
blake3-chip-owned, so IS_MSG_MAT must live on real
matrix-leaf compression rows (the F1 path); the M52 4.2
"separate staging row" model is superseded. C3's constraint
is what makes the F1 path sound.

## Feature gaps

### 🟠 F1 — ai-pow → ai-pow-zk integration is a no-op stub

`prover.rs:334-355` `#[cfg(feature = "zk")]` block does nothing
(`let _ = (...)`). Its comments reference `ai_pow_zk::prove` /
`ai_pow_zk::Witness` / `ai_pow_zk::PublicInputs` — **all stale**;
the real API is `composite_prove` / `composite_verify` /
`CompositePublicInputs` / `CompositeTrace`. M52 step 5 wired the
plain-side `h_a_chunk`/`h_b_chunk`; what's missing is the
`MatmulProof → CompositeTrace` construction (place the matmul /
jackpot / blake3 / matrix-hash instructions from a verified plain
proof) then call `composite_prove`.

**Work:** the `MatmulProof → CompositeTrace` builder. Large
(it's the actual integration), but unblocked — every primitive
exists. Gated in practice by C1/C2 (no point proving until the
PIs bind the PoW). Update the stale comments regardless.

### 🟠 F2 — No recursion / proof compression (M12)

PROD proofs are ~900 KB baseline / ~1.65 MB with activity.
Pearl ships ~60 KB via Plonky2 recursion. Plonky3 has no
compressor. Also gates PROD-scale matrix binding (C3/M52 step 7).

**Work:** out of current scope; tracked as M12. Largest single
lever on both proof size and PROD viability.

### 🟡 F3 — Difficulty adjustment (WTEMA) absent

`difficulty_target` is a static shape-aware bound; no Poisson /
WTEMA retarget. Pearl §5.4. Arguably out of this crate's scope
(belongs in the chain layer), but flag for whoever wires this
into Nockchain consensus.

## Performance gaps

(See `ENGINEERING_REPORT.md` §6 + `M52_MATRIX_BINDING.md` §7.)

### 🟠 P1 — PROD-scale matrix binding ≈ 16 h/attempt

M52 step 7 analysis: 4096² matrix → ~4.5M trace rows → ~16 h
prove at LB=3. M12-gated. Until then, matrix binding ships at
TEST_SMALL/TEST_PEARL only.

### 🟡 P2 — No memory profiling (§6.2)

Sub-OOM on 32 GB confirmed; no hard upper bound. Commodity
miners at 16/8 GB unvalidated. `dhat`/`flamegraph` run needed.

### 🟡 P3 — LogUp bus-overhead not isolated (§6.3)

~17% LogUp overhead known in aggregate; per-bus distribution
(esp. `cv_routing` 9-elem key vs. range tables) unmeasured.
Ablation bench, ~½ day.

### 🟡 P4 — No CI bench tracking (§6.4)

Bench numbers captured in docs but not a tracked artifact; perf
regressions only caught by manual `--ignored` runs. criterion +
GH Actions, ~1 day.

### 🟡 P5 — No PROD @ 32K, no real-workload bench (§6.5/6.6)

Synthetic activity only; never benched against an actual
ai-pow puzzle solve fed through the prover. Closes once F1 lands.

### 🟡 P6 — FRI operating point not retuned (deliberate)

`PROD_LB4` (−22% proof / +2× prove) available but PROD held at
LB=3. Revisit when on-chain weight proves to be the bottleneck
or M12 lands. See `ENGINEERING_REPORT.md` §11.

## Prioritized remaining work

**C1–C4 resolved 2026-05-15. F1 integration landed 2026-05-15.**
`crates/ai-pow/src/zk_bridge.rs` builds a `CompositeTrace` from a
real `BlockContext` and `composite_prove` + `composite_verify_pow`s
it; the historical no-op stub at `prover.rs:334-355` is replaced
by a real call (a hard correctness gate under the `zk` feature —
every `mine()` now also produces + PoW-verifies a SNARK). The
F1 harness + `scripts/profile_f1.sh` + `PROFILING.md` (samply /
peak-RSS P2 / CI-bench P4) remain the instrumentation substrate.

**Bound non-vacuously on a real solve (zk_bridge):**
- **C1** — `JOB_KEY` = κ and `COMMITMENT_HASH` = `s_a` via
  `CompositeTrace::place_key_pin_row` (key-pin rows: `CV_IN` =
  the chain-pinned key, no other chip activity, only the C1
  selector-gated constraint live). Asserted == `BlockContext`.
- **C3** — `HASH_A` / `HASH_B` = chunk-Merkle of A/B keyed by κ,
  asserted byte-equal to `commit::matrix_commitment`.
- **C2** — `composite_verify_pow` checks the bound `HASH_JACKPOT`
  vs the real `difficulty_target`.

### ✅ C4 — HASH_JACKPOT bound (RESOLVED 2026-05-15)

Two stacked obstacles, both now cleared:

- **(a) Selector multiplexing** — `IS_HASH_JACKPOT` is the
  jackpot `is_active` (`chips/jackpot/chip.rs:112`,
  `Σ slot_sel == is_active` `chip.rs:142`). Resolved by
  `CompositeTrace::place_jackpot_hash_block`: the trace's final
  8 rows are a keyed BLAKE3 of `JACKPOT_MSG` (key = `s_a`); row 7
  (= last trace row) co-carries the BLAKE3 finalize AND a
  degenerate-but-valid jackpot step (slot 0,
  `V_BITS = bitdecomp(JACKPOT_MSG[0])`), so the jackpot
  `when_transition` is vacuous on the last row (mirrors Pearl
  `structure_jackpot_blake`).
- **(b) `verify_round` leading-boundary gate bug** — the deeper
  blocker (a bare blake block only verified row-0-contiguous)
  was root-caused and **fixed**: `Blake3Chip::eval_at` now gates
  the cross-row round with `(1 − is_last_round) ·
  (1 − next_is_new_blake)` instead of just `1 − is_last_round`.
  Full write-up + rationale: `BLAKE3_CHIP_ROUND_GATE_BUG.md`
  (status: FIXED). Regression
  `blake_block_verifies_off_row_zero_after_gate_fix` proves a
  bare block now verifies mid-trace and trace-terminal.

`HASH_JACKPOT` is now a non-vacuous bound PI on a real solve
(`zk_bridge` rejects a zero `HASH_JACKPOT`); C2 checks it against
the real `difficulty_target`. **Fidelity caveat (not a binding
gap):** the hashed `JACKPOT_MSG` is all-zero — threading the real
matmul→jackpot rotate-XOR-13 tile-state fold is a remaining
*fidelity* item (what is hashed), not a soundness/binding gap.
`BLAKE3(zeros, key=s_a)` is a genuine keyed digest and the
binding constraint is fully exercised.

Remaining:

1. **Matmul→jackpot fidelity** — feed the real rotate-XOR-13
   tile-state fold into the C4 hash (non-zero `JACKPOT_MSG`).
   Pure fidelity; the C4 binding already holds. The interleaved
   `structure_matmul_in_stark` schedule is the reference.
2. **F2 / M12** (recursion) — 🟠 biggest production lever;
   separate track.
3. **P1, P3, P5, P6** — PROD-scale (M12-gated), per-bus LogUp
   ablation, real-workload bench, FRI retune. P2/P4 have infra.

> ✅ **CRIT-1 RESOLVED (2026-05-15, commit `9ec529e`).** The
> earlier banner here flagged that the C1/C3/C4 bindings were
> vacatable by a malicious prover (no verifier-fixed program).
> Fixed: `CompositeFullAirPinned` commits the program columns
> (`CONTROL_PREP` + `*_PREP`) as a p3-uni-stark preprocessed
> trace with an unconditional `main[col]==preprocessed[k]`
> constraint; `CONTROL_PREP` pins all 21 selectors via the
> control-chip packing. Production (`ai-pow::zk_bridge` →
> `mine()` gate) + F1 harness use `composite_*_pinned`. The
> `crit1_*` adversarial suite (4/4) proves the zeroed-selector
> forged-winning-PoW is rejected vs the canonical VK.
> `ZKP_SECURITY_REPORT.md` is the authority and is updated to
> STATUS: RESOLVED.

Post-CRIT-1 + HIGH-2-keystone summary: C1–C4 bindings are
**enforced** against a malicious prover (program-pinned, CRIT-1).
C1 ties κ / `s_a`; C3 binds matrix bytes; C4 binds the jackpot
keyed-hash; C2 checks difficulty against that hash. **HIGH-2
soundness gap RESOLVED** (`15ba9a3`). **HIGH-2.2 fidelity
LARGELY CLOSED 2026-05-16:** keystone generalised to
`JACKPOT_MSG[0..16] == FOLD_STATE[0..16]` (full Pearl §4.5
folded `TileState M`); a `FoldChip` + `place_fold_chain` +
`zk_bridge`/`mine()` now place the **real** solved tile's
matmul→fold chain via the production **Route-A batch-stark**
path (CRIT-1 pin + `noised_packed` LogUp unified) ⇒ an honest
proof attests `BLAKE3(real M, s_a)`, byte-equivalent to the
plain miner (not `BLAKE3(0,s_a)`). A pre-existing latent
JackpotChip bug (the `JACKPOT_MSG` RAM recurrence ungated by
`is_active`, masked for years by all-zero messages) was
root-caused & fixed (`354b47e`). **§6(a) fold-schedule pin
RESOLVED 2026-05-16 (`aa82ce3`):** `FOLD_IS_FOLD` + the 4-bit
fold-slot are packed into the CRIT-1-pinned `CONTROL_PREP` and
asserted by `ControlChip`; `place_fold_chain` writes it,
`extract_program` lifts it ⇒ **which rows fold, into which slot,
is verifier-fixed**. Done by reusing the existing pinned column
(no preprocessed-width blow-up — §4.C.8 trap avoided; zero blast
radius for non-fold rows). +6 exhaustive ControlChip tests
(positive + 4 adversarial + bit-layout + zero-blast-radius).
Full `cargo test -p ai-pow --features zk` green incl.
`end_to_end` 13/0; ai-pow-zk lib 322/0 incl.
`high2_2_fold_chain_pinned_logup`/`routea_*`/`crit1_*`; no
regression. **§6(b) ✅ CLOSED for the primary mining geometry +
§4.E ✅ DONE 2026-05-16** (`072d840`/`c63fbc1`/`69e420d`/`e7f59f7`):
`X_STEP` is now in-circuit forced to `⊕` the real `t×t`
committed-matrix accumulator — `place_useful_work_chain`
(sub-block-major matmul sweep + co-located `StripeXorChip`) +
`SX_IN == nxt.CUMSUM_TILE` binding + Pinned
`FOLD_XSTEP == SX_XR[stripe]` keystone, so **a malicious prover
must do the real matmul** for `num_stripes ≤ 16` (TEST_SMALL /
the headline e2e). The bridge attests the *actual solved tile*
via MED-3 `tile_ij`. Validated end-to-end (ai-pow-zk lib 331/0;
ai-pow `--features zk` lib 70/0, `end_to_end` 13/0,
byte-equivalence preserved). **MED-3 ✅ RESOLVED**
(`prove_and_verify_for_block` re-derives `target`; `tile_ij`
contract). **Remaining (scoped, not a forgery hole):** (1)
`num_stripes > 16` (rect / PROD `k/r=64`) — legacy path, §6(b)
keystone gated off via the verifier-set `sx_bound` (sound as
CRIT-1); needs a wider StripeXor register + per-fold-row stripe
selector. (2) deep tile↔committed-store ≡ §4.C
`noised_packed`-non-vacuity on sweep rows (`place_matmul_step`
sets `MAT_ID=0`). Plus 7-round-Tip5 review, recursion (M12),
production-hardening (P1/P3/P5/P6).
