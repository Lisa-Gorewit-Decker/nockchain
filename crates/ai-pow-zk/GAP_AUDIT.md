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
does the same — difficulty/membership *are* external by design
in Pearl, MED-3-faithful). *[Note 2026-05-17: an earlier clause
here cited "ai-pow's existing spot-check protocol" — that
mechanism (`MatmulProof.spot`/`params.spot_checks`) is
**test-only scaffolding, never a production path** (maintainer);
it is not a soundness argument and is a cleanup candidate. See
`M_S2_PEARL_EVALUATION.md`.]*

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
>
> ✅ **CRIT-1 made FIRST-CLASS — Phase A-CR (2026-05-18,
> `3671702`..`2a9a18d`).** The verifier's canonical program is
> no longer `extract_program` of a prover-derived reference
> trace: it is a witness-free, params-pure
> `ai_pow_zk::canonical::canonical_program` (one params-pure row
> schedule → per-row `RowDescriptor`, CR.1–CR.5 covering **all
> 12 PROGRAM_COLS, every row**, incl. the §4.C.2 co-located 8
> `NOISE_PACKED_PREP` noise pins via `noise_ref`). On the
> production-faithful **16|r** path (Pearl §4.8 always 16|r) the
> bridge verifies against the verifier-rebuilt canonical program,
> never the prover's (CR.6). KAT-anchored: the §5 cross-crate KAT
> proves `canonical_program == extract_program(honest_trace)`
> bit-for-bit on every row × 12 PROGRAM_COLS of the real
> P16(16|r) trace; adversarial `cr6_*` proves a non-canonical
> PROGRAM_COL is rejected (pre-CR.6 it verified). **Subsumes the
> §4.C.2 "b2" item.** Non-16|r = documented A3.2b test path,
> retains the prior `crit1_*`/`routea_*` discipline.
> `CANONICAL_PROGRAM_DESIGN.md` §7 is the authoritative status.

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
regression. **§6(b) ✅ CLOSED for every single-Layer-0 params set
+ §4.E ✅ DONE 2026-05-16**
(`072d840`/`c63fbc1`/`69e420d`/`e7f59f7`/`010ccd3`):
`X_STEP` is now in-circuit forced to `⊕` the real `t×t`
committed-matrix accumulator — `place_useful_work_chain`
(sub-block-major matmul sweep + co-located `StripeXorChip`) +
`SX_IN == nxt.CUMSUM_TILE` binding + Pinned
`FOLD_XSTEP == SX_XR[stripe]` keystone, so **a malicious prover
must do the real matmul**. **G1+G2 (`010ccd3`)** generalized it
beyond TEST_SMALL: G1 chunks the `r`-wide dot (`⌈r/TILE_D⌉`
micro-steps, `r > 16`); G2 widens StripeXor to `STRIPE_MAX = 64`
lanes + a `CONTROL_PREP`-pinned 6-bit fold-stripe index +
`FOLD_STRIPE_SEL` keystone — so the rectangular LLM-FFN
`llm_shape` shapes (`k/r = 20`) now run the full §6(b) binding.
The bridge attests the *actual solved tile* via MED-3 `tile_ij`.
Validated end-to-end (ai-pow-zk lib 332/0; ai-pow `--features zk`
lib 71/0, `end_to_end` 13/0, **`llm_shape` 5/0 via §6(b)**,
byte-equivalence preserved; `high2_2_g1g2_chunked_and_wide_stripes`
debug-assertions-ON clean). **MED-3 ✅ RESOLVED**
(`prove_and_verify_for_block` re-derives `target`; `tile_ij`
contract). **Remaining (scoped, not a forgery hole):** (1) **true
PROD** (`k/r=64`, chunked sweep ≈ 2²⁰ ≫ *today's* `MIN_STARK_LEN
= 2¹³` Layer-0) — legacy path, §6(b) keystone gated off via the
verifier-set `sx_bound` (sound as CRIT-1); closing it = **the
Pearl-faithful P-A/P-B/P-C path** (adopt Pearl §4.8 param caps so
one tile = one STARK + raise the Layer-0 ceiling toward Pearl's
`≤2²²` + vertical-recursion certificate — `M_S2_PEARL_EVALUATION.md`,
maintainer γ decision 2026-05-17).
*[2026-05-19, recursion-milestone scope — distinct from the
MAT_UNPACK↔BLAKE3_MSG "C3" gap above: the recursion milestone
**C3/M-S5** ("vertical-recursion certificate") was **re-scoped**
to the soundness-correct **≥120-bit** cert (LANDED +
independently re-validated; honest real sizes L1≈2.69 MB /
L2≈1.79 MB; end-to-end ≥120-bit; all 5 inner sweep profiles
accept + tamper-reject; fenced linchpin byte-identical; DT-4
duplex binding intact). The **≤65 KB** size bar is **deferred →
new milestone M-S5b** (size-targeted SNARK/fold wrap not in the
current substrate; §14 proved ≤65 KB unreachable at any real
≥120-bit tier — recursion diverges; only the ~5-bit testing
tier hits ≤65 KB, a soundness trade the maintainer rejected).
See `C3_OUTER_CERT_DESIGN.md` §13.2/§14/§15,
`PRODUCTION_ROADMAP.md` C3/M-S5b.]* *[Corrected: this previously
said "closing it = G3 (segmentation+M12)"; G3 carry-segmentation
has no Pearl precedent and is **deferred** — Pearl caps params so
it never segments. No production spot-check exists
(`MatmulProof.spot` is test-only).]* (2) **M-S1 ✅ RESOLVED 2026-05-17** — §4.C
sweep-input non-vacuity: pack-link + whole-micro-tile chunked
`noised_packed` query + pure producer store bind the §6(b) sweep
A/B inputs to a declared canonical store (LogUp multiset);
adversarial **I2** `high2_2_swept_tile_not_in_store_rejects`
rejects a swept tile ∉ store; Route-A green (parallel +
debug-assertions-ON), `ai-pow-zk --lib` 335/0/22, `ai-pow
--features zk` green incl. MED-3 bridge roundtrip.
**§4.C.2 ✅ RESOLVED 2026-05-18 — ZERO-GAP on the
production-faithful 16|r path (c-exact).** A3.0–A3.2b closed the
*noise* tie (store `NOISE_UNPACK` = `noise_ref(C1 seed)` via
InputChip + the CRIT-1 `NOISE_PACKED_PREP` pin); the *plain* tie
is closed by cx.1 (generalized C3 + CRIT-1 word-pair pin) + cx.2
(the X1 g=1 co-location flip — the strip-opening leaf round-0
rows are the M-S1 `noised_packed` producers; the whole-block C3
binds their `UINT8_DATA[0..64]` to `BLAKE3_MSG` ∈ `HASH_A`).
End-to-end + position-exact adversarially validated on a real
16|r `P16` bridge trace
(`sec_4c2_cx2_g1_p16_route_a_c3_active_roundtrip` proves +
pow-verifies at real difficulty with C3 ACTIVE;
`sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects` — a
tampered co-located committed-plain byte is rejected). Pearl
§4.8 is always 16|r ⇒ the production path is zero-gap; non-16|r
*test* geometry remains the A3.2b separate-store path (strictly
stronger than pre-A3, not a forgery hole). Validated:
`ai-pow-zk --lib` 352/0/22, `ai-pow --features zk` 89/0/1,
debug-assertions-ON P16 g=1 per-row clean. Detail:
`SEC_4C2_NOISE_BINDING_DESIGN.md` §8. Plus
7-round-Tip5 review, recursion (M12),
production-hardening (P1/P3/P5/P6).
