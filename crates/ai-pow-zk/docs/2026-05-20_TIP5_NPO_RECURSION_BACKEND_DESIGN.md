# Tip5-NPO recursion backend — design (M-S5b C2.2/C2.3 wiring)

**Date:** 2026-05-20
**Author:** Claude (with maintainer R1 discipline)
**Status:** Stage 1 design — KAT-first staged plan; no invasive
edits yet.

## Goal

Implement a Tip5-based NPO (non-primitive op) recursion backend
that lets the L2 outer-cert verifier circuit verify a Tip5-
throughout L1 outer-cert, with **zero Poseidon2 in the trust
surface** at any layer (per user hard rule + `MEMORY.md`
[no_poseidon2_anywhere]).

The deployed shape:

```
inner Tip5-L0 STARK (ai-pow-zk)
   |   verified by
   v
L1 outer-cert  (Tip5-throughout MMCS + Tip5 challenger)
   |   verified by — NEW PATH
   v
L2 outer-cert  (Tip5-throughout MMCS + Tip5 challenger,
                Tip5 NPO in-circuit verifier)
```

The L1 path already works (`test_l1_outer_cert_tip5_unified.rs`,
`test_l1_size_reduction_sweep.rs`, etc.). The L2 path currently
falls into the `Poseidon2` else-branch of
`FriRecursionBackend::non_primitive_air_builders` — which is
forbidden by the user rule. This document plans the staged work
to add the Tip5 dispatch arm and exhaustively validate it.

## Scope (this milestone)

**In scope:** D=2 (Goldilocks quadratic extension, the deployed
outer-cert challenge field) — the only path Tier B L2 needs.

**Out of scope:** D=4 / D=5 Tip5 paths. They have no deployed
consumer; `Tip5AirBuilder<4>` and `Tip5AirBuilder<5>` don't even
exist. Adding them is speculative future-proofing. Keep the gap
honest: this milestone closes the **D=2 only** Tip5 routing gap.

**Soundness inheritance:** the Tip5 NPO AIR is C2.0/C2.1 KAT-
anchored against the native `nockchain_math::tip5::permute`
oracle (per [c2_tip5_circuit_air] memory; commits dc4e217 +
62413ba). C2.3 (in-circuit Tip5 MMCS verify) is the
soundness gate for in-circuit Tip5 hashing
(`recursion/src/pcs/mmcs.rs:1789+`). This milestone *wires*
those existing-and-validated pieces into the L2 dispatch;
it does **not** introduce new soundness assumptions.

## The exact gap (from Stage 0 survey)

Three dispatch points in
`crates/plonky3-recursion/recursion/src/backend/fri.rs` D=2 impl block
(lines 444-556), each currently a binary if-poseidon1-else-poseidon2:

1. **`non_primitive_preprocessors`** (lines 512-520) — selects
   between `poseidon1_preprocessor::<Val<SC>>()` and
   `poseidon2_preprocessor::<Val<SC>>()`. Need Tip5 arm
   selecting `tip5_preprocessor::<Val<SC>>()`.

2. **`non_primitive_provers`** (lines 522-544) — match on
   `(as_poseidon1(), as_poseidon2())` routes to `Poseidon1ProverD2`
   / `Poseidon2ProverD2`. Need Tip5 arm routing to
   `Tip5Prover::new(*c, ConstraintProfile::Standard)` **directly**
   — `Tip5Prover` natively supports D=2 (per
   `circuit-prover/src/batch_stark_prover/tip5.rs:293-304`); no
   `Tip5ProverD2` wrapper is needed.

3. **`non_primitive_air_builders`** (lines 546-555) — selects
   between `poseidon1_air_builders::<SC, 2>()` and
   `poseidon2_air_builders::<SC, 2>()`. Need Tip5 arm selecting
   `tip5_air_builders::<SC, 2>()`.

**Plus** an impl-block trait bound:
`Tip5Preprocessor: NpoPreprocessor<Val<SC>>` (line ~452).

That's the entire D=2 wiring change — net ~12-15 lines.

## R1 staging plan

### Stage 0 — Survey (COMPLETE)

Mapped above. No code touched.

### Stage 1 — Design (THIS DOC; COMPLETE)

Design + scope + per-stage acceptance gates. No code touched.

### Stage 2 — KAT-first de-risk (BEFORE invasive edit)

Goal: prove that the Tip5 NPO pieces (`tip5_air_builders<SC,2>`,
`Tip5Prover` at D=2, `Tip5Preprocessor`) **compose correctly in
the recursion-verifier shape** before touching the dispatch
logic. If they don't, the invasive edit later would land a broken
substrate.

**Test:** new file
`crates/plonky3-recursion/recursion/tests/test_tip5_npo_recursion_kat.rs`
that:

- Builds the Tip5 NPO trio (`tip5_air_builders<SC,2>`,
  `Tip5Prover::new(*tip5_cfg, ConstraintProfile::Standard)`,
  `tip5_preprocessor`) directly — no recursion-backend dispatch.
- Runs a TOY L2-shape circuit (the smallest verifier-circuit-
  shaped runner that exercises Tip5 NPO trace generation + AIR
  evaluation + preprocessor consistency).
- Asserts: trace+air round-trip; preprocessor binding agrees with
  prover-side preprocessed; lookups (witness bus, tip5_l) close.
- ACCEPT + TAMPER-REJECT.

**Gate:** all assertions pass. If any fail, fix the upstream
NPO/preprocessor (NOT the dispatch — the dispatch is a one-line
change; the soundness is in the pieces).

**Commit:** "Tip5 NPO recursion backend — Stage 2 KAT-first
de-risk: standalone Tip5 NPO trio composes correctly in recursion-
verifier shape (ACCEPT + tamper-REJECT)."

### Stage 3 — Land the wiring (THE INVASIVE EDIT)

Edit `recursion/src/backend/fri.rs` D=2 impl block:

1. Add trait bound `Tip5Preprocessor: NpoPreprocessor<Val<SC>>`.
2. Refactor `non_primitive_preprocessors` to a 3-way match
   (poseidon1 → poseidon2 → tip5).
3. Refactor `non_primitive_provers` 2-tuple match to 3-tuple
   including `as_tip5()`.
4. Refactor `non_primitive_air_builders` to 3-way match.

**Imports:** add
`tip5_air_builders, Tip5Preprocessor, tip5_preprocessor` from
`p3_circuit_prover::batch_stark_prover` and
`Tip5Prover` from `p3_circuit_prover` to the use list at the top
of `recursion/src/backend/fri.rs`.

**Acceptance gate:** project compiles. All previously-green tests
still PASS at the new dispatch (the Poseidon1 / Poseidon2
branches must remain byte-identical; only the new Tip5 arm
should produce different output, and it has no caller yet).

**Commit:** "Tip5 NPO recursion backend — Stage 3 land D=2
dispatch wiring (as_tip5 arm in non_primitive_{preprocessors,
provers,air_builders}; Tip5Prover natively D=2 so no wrapper)."

### Stage 4 — L2-over-L1 in Tip5-throughout validation

Write a `c3_stage_b_l2_over_tier_b_l1_tip5_throughout`-style
test that uses **the Tip5-throughout test-utils**
(`goldilocks_tip5_params`) and Tip5 challenger throughout the
outer-cert. Build L1 + L2 + tamper-reject.

This is the FIRST end-to-end test of L2-over-L1 in
Tip5-throughout substrate. It's heavy (~many min) so it's
`#[ignore]`d behind a manual flag, like the existing L2 tests.

**Acceptance gate:** L2 ACCEPTS a valid L1; L2 REJECTS a
tampered inner Tip5-L0 proof (loud panic if it doesn't reject).

**Commit:** "Tip5 NPO recursion backend — Stage 4 L2-over-L1
in Tip5-throughout substrate ACCEPTS + TAMPER-REJECTS at Tier B
FRI params."

### Stage 5 — Tier B L2 measurement (the original ask)

Now possible cleanly. Add a measurement test mirroring
`m_s5b_tier_b_l1_l2_measurement` (the one reverted earlier) but
in the **Tip5-throughout substrate** via the goldilocks_tip5_params
infrastructure. Record L1 + L2 sizes. Update the investigation
doc and README.

**Acceptance gate:** measurement runs; size is recorded; no
parity regression against the Poseidon1 / Recompose-only paths
that are still unchanged.

**Commit:** "ai-pow-zk: M-S5b S1.B Tier B L2 measurement at
Tip5-throughout substrate — L1 X KB; L2 Y KB (record).
Updates investigation doc + README."

### Stage 6 — Exhaustive regression + cross-refs

- Full `cargo test -p p3-recursion --release --tests` green.
- Full `cargo test -p p3-circuit-prover --release` green.
- ai-pow-zk crate tests green (no downstream regression).
- Update [c2_tip5_circuit_air] memory (mark C2.2/C2.3 as
  LANDED for D=2, deferred for D=4/D=5 as honest residual).
- Update [no_poseidon2_anywhere] memory if any new lessons.
- Cross-ref the investigation doc + README + this design doc.

**Acceptance gate:** green project regression + memory + cross-
refs consistent.

**Commit:** "Tip5 NPO recursion backend — Stage 6 exhaustive
regression + cross-refs. Closes C2.2/C2.3 for D=2; D=4/D=5
remain explicit residual (no current deployed consumer)."

## Tamper-reject test design (R1 critical)

The L2 verifier circuit's job is to PROVE that L1 verifies an
inner Tip5-L0 STARK. Two adversarial scenarios MUST be rejected:

1. **Tampered inner Tip5-L0 proof** — the inner STARK proof is
   modified to claim a wrong final state. The L1 outer-cert should
   reject (so no valid L1 is even built). If by some bug the
   tampered inner produces a "valid" L1, the L2 verifier must
   reject; otherwise we have a soundness hole (logged loudly per
   `c3_stage_b_l2_over_120bit_l1`'s pattern).

2. **Tampered L1 outer-cert** — directly modify the L1 proof bytes
   after building. The L2 verifier circuit's in-circuit FRI
   verification + Tip5 MMCS path should reject. If it doesn't,
   the Tip5 NPO wiring has a hole — STOP and investigate.

Both scenarios run in Stage 4. The expected outcome: tampered
inner is rejected at L1 build (no L1 to L2 over); tampered L1
fails L2 verification. We log clearly in either case.

## Honest residuals (per R1)

- **D=4 / D=5 Tip5 paths** — not in scope; no current consumer;
  Tier B (D=2) covers production. Adding them is a separate
  speculative future task.
- **Tip5 NPO performance at L2 scale** — Tip5's wider state +
  more rounds (W=16, 7 rounds) may make the in-circuit verifier
  larger than the Poseidon2 equivalent. This is a measured
  trade-off (Stage 5 will record it). Acceptable given the user
  rule (Tip5-throughout is mandatory; performance is a knob).
- **Tip5 NPO in D=4 outer-cert paths** — `Tip5Prover::batch_instance_d4`
  returns `None` (intentional, per source comment at
  `tip5.rs:318-325`). Any future D=4 deployment would need
  `Tip5AirBuilder<4>` AND the prover's d4 path.

## Files this milestone will touch

**New:**
- `crates/ai-pow-zk/docs/2026-05-20_TIP5_NPO_RECURSION_BACKEND_DESIGN.md` (THIS)
- `crates/plonky3-recursion/recursion/tests/test_tip5_npo_recursion_kat.rs` (Stage 2)
- `crates/plonky3-recursion/recursion/tests/test_tip5_l2_over_l1.rs` (Stage 4)

**Edited (Stage 3):**
- `crates/plonky3-recursion/recursion/src/backend/fri.rs` (D=2 impl block;
  ~15 lines net)

**Edited (Stage 4 + 5):**
- `crates/plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs`
  — DOES NOT add Poseidon2 substrate. May add a Tip5-throughout
  parallel of the `make_outer_cfg` helper using `goldilocks_tip5_params`.

**Edited (Stage 5):**
- `crates/ai-pow-zk/docs/2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`
  (add L2 measurement at Tier B)
- `crates/ai-pow-zk/README.md` (link this design doc; note
  C2.2/C2.3 landed)

**Edited (Stage 6):**
- `~/.claude/projects/.../memory/c2_tip5_circuit_air.md`
- `~/.claude/projects/.../memory/MEMORY.md`

## Verification at the end (success criteria)

1. Project-wide `cargo test` green.
2. L2-over-L1 in Tip5-throughout substrate ACCEPTS valid + REJECTS
   tampered.
3. Tier B L2 size measured + documented.
4. Zero new Poseidon2 references anywhere in the trust surface.
5. Honest residuals (D=4/D=5) clearly logged for future work.

R1 honest: this design commits to driving Stages 2-6 in this
session. If a concrete soundness wall surfaces mid-stage, the
last validated stage's commits stay landed and a precise residual
is recorded — but the default expectation is full completion.
