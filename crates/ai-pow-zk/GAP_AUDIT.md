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

**C1–C4 resolved 2026-05-15. F1 substrate + profiling infra
landed 2026-05-15** (`d87fd10`): `crates/ai-pow/examples/f1_harness.rs`
is the instrumented cross-crate fixture (real solve → SNARK,
byte-equivalence-asserted), `scripts/profile_f1.sh` +
`PROFILING.md` cover samply / peak-RSS (P2) / CI-bench wiring
(P4). Remaining:

1. **F1 deep** — 🟠 the critical path. The harness exercises the
   matrix-binding + prove/verify pipeline but NOT the faithful
   `MatmulProof → CompositeTrace` jackpot→blake3 instruction
   chain that makes `HASH_JACKPOT` / `JOB_KEY` /
   `COMMITMENT_HASH` non-zero PIs (so C1/C4 are vacuous in the
   harness today). Build that chain with the C1–C4 selectors on
   genuine compression rows; extend the harness to assert the
   now-non-vacuous bindings. The stub at `prover.rs:334-355` is
   still a no-op with stale comments — clean up.
2. **F2 / M12** (recursion) — 🟠 biggest production lever;
   separate track.
3. **P1, P3, P5, P6** — 🟠/🟡 PROD-scale (M12-gated), per-bus
   LogUp ablation, real-workload bench (once F1 deep lands),
   FRI retune. P2/P4 now have infra; wire P4 into CI when ready.

The honest one-line summary: **the SNARK now proves the
proof-of-work statement.** C1+C4 anchor it to the block
(JOB_KEY / COMMITMENT_HASH) and bind the tile-state hash
(HASH_JACKPOT); C2 enforces the difficulty inequality against
that bound hash (Pearl-faithfully, verifier-side); C3 closes the
matrix-binding chain so the hashed bytes equal the committed
matrix. The remaining gap is **F1** — wiring a verified plain
proof into a `CompositeTrace` so these constraints fire on a real
solve (today they're proven correct in form + by targeted/negative
tests, but no end-to-end "real puzzle → SNARK" path exists yet).
Everything else is production-hardening.
