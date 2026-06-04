> _Created **2026-05-19** · last updated **2026-06-03**._

# C4 / M-S6 — independent crypto audit: readiness package

> **Status (R1, honest).** This document is the **readiness
> package** for the C4/M-S6 milestone (`#125` — independent crypto
> audit of the ai-pow / ai-pow-zk soundness stack).
>
> **Audience (clarified 2026-05-19).** The team performs this
> audit ourselves; **people other than us will also audit the
> code.** This package is written so that both audiences can use
> the same artifacts — claim index, threat model, KAT catalogue,
> adversarial-test inventory, residuals. The team is not making
> any commitment about who those other auditors will be, what
> their scope is, or when they will deliver; that is outside what
> this document controls.
>
> What this delivers: (a) a threat model + audit scope, (b) the
> soundness-claim index (every claim → exact files / commits /
> tests that back it), (c) the recursion-stack inventory + AIR
> claims, (d) the KAT / adversarial-test catalogue, (e) the
> explicit known-residuals list (no hidden gaps), (f) an
> audit-readiness checklist with a small honest set of items
> still to ship before the audit begins.
>
> **Reference papers are cited by name** (title, authors, IACR
> ePrint / arXiv ID); the PDFs themselves are **not** in the
> repository (`.gitignore`d 2026-05-19). Anyone reading this
> document obtains the papers from their published venues.
>
> **Authoritative cross-refs:** `2026-05-17_PRODUCTION_ROADMAP.md`
> Phase C row C4; `2026-05-15_ZKP_SECURITY_REPORT.md` (the
> definitive soundness report — *this doc indexes it, does not
> replace it*); `2026-05-15_GAP_AUDIT.md` (the gap-tracker);
> `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md` (the current
> native terminal-compression workstream — the only soundness-
> relevant known residual that touches the cert size, not the
> production soundness floor).

---

## 0. Purpose & how to use this document

Anyone opening this repository to audit it (the team in-house;
anyone else who chooses to review the code) should be able to:

1. Read this doc § 1 + § 2 to fix **scope** and **threat model**.
2. Walk § 3's soundness-claim index, each row of which is a
   triple `(claim, where the claim is argued, where the claim
   is tested)`.
3. Use § 4–§ 6 as a map of the *substrate* (recursion stack +
   Tip5 AIR + the C3 outer cert) — the load-bearing crypto.
4. Use § 7 to find every adversarial / tamper-reject test
   already in tree, so the auditor can both reproduce and
   **add** tamper variants.
5. Read § 8 as the honest residuals list — nothing the audit
   should be surprised to discover later.
6. Use § 9–§ 10 to confirm the audit can begin (no missing
   artifacts).

The discipline is `~/.claude/CLAUDE.md` **R1** — no fake
completion. If a claim has only a design argument and no test,
the index says so. If a test exists but is `#[ignore]`d, the
index says that and links the reason. If a residual is open,
it is listed in § 8, not omitted.

---

## 1. Audit scope

### 1.1 In-scope

The audit covers the **Nockchain SNARK soundness stack** for
mining the real shipped `Llama-3.1-8B-Instruct-pearl` model
(see `2026-05-17_PRODUCTION_ROADMAP.md` § 0):

| Component | Crate(s) | Role |
|---|---|---|
| Pearl-byte-equivalent mineable unit | `ai-pow` | the *plain* `TileState` / `keyed_hash` / `compute_tile_*` path the SNARK is *of*; byte-equiv to Pearl spec §4.1/§4.3 on type-0 INT GEMMs |
| ai-pow-zk soundness stack | `ai-pow-zk` | the Plonky3 STARK AIR + prover/verifier bridge proving the mineable unit |
| Recursion substrate | `crates/plonky3-recursion/` (vendored, excluded workspace) | C1: vendored Plonky3-recursion at the C1 fixed-point rev `c2c51fb` (rev-aligned to ai-pow-zk's `6de5cba`) |
| Tip5 circuit AIR | `crates/plonky3-recursion/tip5-circuit-air/` | C2: in-circuit recursive Tip5 permutation, KAT-anchored to `nockchain-math::tip5::permute_5round`; canonical non-recursive Nockchain Tip5 remains `permute` (7 rounds) |
| C3 / M-S5 outer-recursive cert | `crates/plonky3-recursion/recursion/tests/test_tip5_layer0_recursion.rs` + `test_tip5_layer0_compression.rs` | The production ≥60-bit Johnson recursive certificate of the inner Tip5 Layer-0 proof |

### 1.2 Out-of-scope

- **Pearl-side code** (Pearl's vLLM plugin / `pearl/zk-pow`).
  We bind to Pearl byte-equivalence on the *mineable unit*; the
  Pearl SNARK pipeline is a separate (Plonky2-based) audit
  surface owned by Pearl.
- **FP8 PoUW.** Pearl §1.1 defers FP PoUW to an unshipped
  protocol. This audit covers INT (type-0) GEMMs only.
- **External integration (Phase D).** `D1` (vLLM miner-plugin
  extraction) and `D2 / M-C1` (consensus block-certificate
  integration) are external to ai-pow-zk and not in this audit.
- **M-S5b terminal compression (`#131`)** is **deferred** (see
  `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`). When it
  lands, a follow-on audit round covers the substrate addition.
  M-S5b is *not* hidden incompleteness of C3 — the ≤100 KB
  target was explicitly carved out and the C3 milestone is the
  production-sound recursive cert (LANDED at the current ≥60-bit
  Johnson floor).
- **G3 carry-vector segmentation.** Deferred (Pearl-faithful
  evaluation — this model is in-envelope; revive only if a
  workload exceeds `k ≤ 2¹⁶`). See
  `2026-05-17_M_S2_PEARL_EVALUATION.md`.
- **R-b**: ai-pow-zk's M10.1c composite `RecursiveAir` (vs the
  representative `FibonacciAir`) is **M12 / `#127`** — out of
  this audit.

### 1.3 What "soundness" means here

**2026-05-21 anchored-between reanchor (maintainer):** the
per-block / per-link soundness floor is now **≥60 bits
unconditional Johnson**, anchored *inside* the (22, 80)
interval bounded by:

- **Known insecure** (IACR ePrint 2025/2055 § 1.4.5 + Thm 1.17
  CYCLE-SUM): `log₂(n) + O(1)` ≈ 22 bits at n ≤ 2^22 — explicit
  constructive STARK attack at γ ≥ LDR.
- **Known secure** (paper Thm 1.5): `lb · nq + pow` ≥ 80 — the
  prior conservative floor.

The reanchor was triggered by a careful read of the paper's
**negative results** (Thms 1.6, 1.9, 1.13, 1.17 + §§ 1.4, 6, 8)
showing the Plonky3 `CapacityBound::log_eta` heuristic claiming
~2× per-query bits at γ ≈ 1−ρ sits in the no-mans-land between
Johnson (proven) and LDR (attacked) with no paper support
against generic codes; that heuristic is **rejected**.

**The 60-bit floor is justified by the time-bounded threat
model:** PoW forgery in this chain is bounded by the 2.5-min
block cadence (~150 s before a fresh honest block obsoletes the
forge target). At 60 bits, 2^60 ops in 150 s ⇒ ~7.7·10^15
ops/sec sustained throughput required; FRI verification's
random-Merkle-path-access workload disfavors GPU/ASIC, putting
the wall-clock budget beyond the block window even for
state-actor-scale compute. The 80-bit margin only defends
offline / long-horizon attackers, which the 2.5-min cadence
forecloses. Maintainer verbatim (2026-05-21): *"an attacker has
2.5 minutes to make a proof in our context, hence our optimism."*

See `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` §0 (2026-05-21
anchored-between addendum) for the full derivation + paper-
end-point table.

Two soundness objects, both **≥ 60 bits unconditional Johnson**:

1. **Per-block** (one mined tile). A prover that does not know
   a witness clearing the published difficulty cannot produce
   an accepting `(proof, public_inputs)` pair, except with
   probability ≤ 2⁻⁶⁰ over the verifier's randomness, in the
   *unconditional* Johnson-radius regime (no list-decoding
   conjecture).
2. **End-to-end recursion** (M-S5 cert). Every layer of the
   verifier-recursion chain is **≥ 60 bits unconditional
   Johnson** at the anchored params (inner Tip5-L0 PROD
   `lb=4 nq=15 pow=1+1` = 62 bits; outer-cert L1
   `goldilocks_tip5_60bit` `lb=4 nq=9 query_pow=24` = 60 bits;
   chain MIN = 60 bits).

The audit should *also* assess:

- **The paper-grounded soundness map.** Verify
  `(lb, nq, pow_bits) → unconditional bits at Johnson radius`
  for our specific Plonky3-recursion FRI variant under
  IACR ePrint 2025/2055 Theorem 1.5. Confirm γ < J(δ)−η at every
  M-S5 link (the paper's §8 attacks confirm beyond-Johnson is
  unsafe).
- **Knowledge soundness** (extractability) — not just
  computational soundness — for the consensus-facing artifact.
- **Pearl-byte-equivalence** of the mineable unit (the
  byte-equiv anchor for the SNARK).
- **No-forgery against the program pin** (CRIT-1: a prover
  cannot swap in a different program/AIR than the consensus-
  fixed one).

---

## 2. Threat model

### 2.1 Adversary classes

| Class | Capability | Why this audit assesses it |
|---|---|---|
| **A-FORGE** | Produces accepting `(proof, PI)` without a witness clearing real difficulty | Fund/PoW safety — the headline threat |
| **A-PROGRAM** | Forges by swapping the AIR / program / VK | CRIT-1: requires verifier-reconstructed canonical program (now first-class via Phase A-CR) |
| **A-NOISE** | Forges by supplying noised inputs that aren't the noise(committed plain) | §4.C.2 noise tie (A3.3 — zero-gap on 16∣r) |
| **A-SWAP** | Forges by swapping A or B matrix between strips, or skipping/duplicating stripes | §4.A fold-chain + M-S1 multiset bus |
| **A-TILE** | Wins a *cheaper* tile than attested | MED-3: verifier-derived `(tile_i, tile_j)` |
| **A-MAT** | Forges by supplying a different committed matrix than `HASH_A`/`HASH_B` | M52 matrix binding |
| **A-CHAIN** | Forges the recursion chain (claims a valid inner that isn't) | C2.4 in-circuit Tip5 Layer-0 verify + production C3 recursive cert |
| **A-SOUND** | Exploits a sub-60-bit production configuration | Every production FRI tier in the consensus-facing recursive path is ≥60 bits unconditional Johnson; current outer profile is `lb=4, nq=9, query_pow=24` ⇒ 60 bits (§1.3 above) |
| **A-FRI** | Exploits a FRI commitment-scheme weakness | Standard Plonky3 FRI (audited upstream); we use established parameters; **proximity testing stays at γ < J(δ)−η** (Johnson radius, never beyond — IACR ePrint 2025/2055 §8 attacks avoided) |
| **A-LDR** (new) | Pushes proximity testing beyond Johnson radius into the list-decoding regime where the paper's negative results + §8 attacks live | ✅ **Per-layer γ vs J(δ)−η table landed 2026-05-20** in `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` §4.3 — every M-S5 link (inner PROD/LB2/LB4/LB5/LB6 + L1 + L2 outer-cert) operates strictly inside Johnson with `J(δ) ≥ 0.5` and `η > 0` at every layer; paper §8 attacks structurally avoided |
| **A-HASH** | Exploits Tip5 / BLAKE3 weakness | Tip5: KAT-anchored to spec (paper IACR ePrint 2023/107); BLAKE3: as-published |

### 2.2 What is **not** mitigated by this audit alone

- **Pearl-side weakness** of the vLLM extraction pipeline. The
  audit assesses ai-pow's byte-equivalence on a fixture; what
  Pearl actually mines (`B1` reference vectors, `B2` quant-
  extraction contract) is gated by Pearl's own audit.
- **Consensus integration security** (Phase D2 / `M-C1`). Out
  of scope.
- **FP8 layer security.** Not mined in this audit's scope.

---

## 3. Soundness-claim index — every claim, where argued, where tested

Read this as **the audit's main worktable.** Each row is:
`(claim → design doc with argument → test or KAT that
backs it → status)`.

### 3.1 CRIT — program/VK pin

| # | Claim | Where argued | Where tested | Status |
|---|---|---|---|---|
| **CRIT-1** | Verifier reconstructs `canonical_program(params, block_public)` witness-free; VK is fixed by public params, not prover-passed | `2026-05-17_CANONICAL_PROGRAM_DESIGN.md` (CR.0–CR.7) | `ai-pow-zk` test module `crit1_*` (every PROGRAM_COL ≠ canonical ⇒ reject) | ✅ landed (CR.0–CR.7 commits per the canonical_program design doc; subsumes §4.C.2-b2) |
| **MED-3** | Verifier reconstructs `(difficulty_target, tile_i, tile_j)` from public inputs; prover-attested tile must match | `2026-05-17_CANONICAL_PROGRAM_DESIGN.md` + `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` | `ai-pow --features zk` `end_to_end` + MED-3 roundtrip | ✅ landed |
| **CR.6** | Verify-path bound to `canonical_program(params, block_public)`, not prover-passed program | `2026-05-17_CANONICAL_PROGRAM_DESIGN.md` § CR.6 | Full `crit1_*` + new adversarial (any PROGRAM_COL ≠ params-pure canonical ⇒ reject) | ✅ landed |

### 3.2 HIGH — matmul / fold / digest chain

| # | Claim | Where argued | Where tested | Status |
|---|---|---|---|---|
| **HIGH-2.2 §4.A** | `place_matmul_tile` + bridge: real solved tile's t·k INT strips drive `CUMSUM_TILE` | `2026-05-15_HIGH2_2_DESIGN.md` § 4.A | `ai-pow-zk` lib + `ai-pow --features zk`; honest real-tile roundtrip | ✅ landed |
| **HIGH-2.2 §4.B** | `FoldChip` + `FOLD_STATE` (Option B2 direct per-stripe; `M_next[slot] = rotl13(M_cur[slot]) ⊕ x_step`) bit-identical to `TileState::fold` | `2026-05-15_HIGH2_2_DESIGN.md` § 4.B | unit vs `TileState::fold` / `compute_tile_from_slices` | ✅ landed |
| **HIGH-2.2 §4.D** | `JACKPOT_MSG[0..16] == FOLD_STATE` (no zero-pad stand-in) | `2026-05-15_HIGH2_2_DESIGN.md` § 4.D | E2E `high2_2_honest_real_tile_roundtrip` clears real difficulty | ✅ landed |
| **HIGH-2.2 §4.C / A3.3** | `A/B_NOISED_UNPACK` is the noise-of-committed-plain (zero-gap on the 16∣r path; c-exact) | `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` § 8 | `ai-pow-zk` `cx.0` KAT (witness-free `(chunk,block,word_off)` leaf address binds row → committed bytes ∈ `HASH_A`) | ✅ landed for production (Pearl always 16∣r); non-16∣r TEST geom = A3.2b documented strictly-stronger-than-pre-A3 |
| **HIGH-2.2 §4.C / cx.2 full-block split-view closure** | Co-located BLAKE3 leaf rows bind all 64 `UINT8_DATA` bytes to all 64 `MAT_UNPACK` bytes via gated `i8u8`, all 64 `NOISE_UNPACK` bytes via `irange7p1`, all 64 `MAT_UNPACK` bytes via `irange8`, and all 8 `NOISED_PACKED` sub-slices via BLAKE-side self-queries / `MAT_FREQ`. This closes the external-review P0 concern that later 8-byte sub-slices could be split between BLAKE3-facing bytes and matmul-facing bytes. | Prover-side review 2026-06 P0 + `composite_layout.rs` / `composite_full_air_with_lookups.rs` | `non_first_subslice_split_view_rejected_by_i8u8_logup`; `non_first_subslice_mat_freq_drop_rejected_by_logup`; `inconsistent_i8u8_pair_rejected_by_logup` | ✅ landed |
| **HIGH-2.2 §4.E** | Attested `(tile_i, tile_j)` bound (reconciled with MED-3) | `2026-05-15_HIGH2_2_DESIGN.md` § 4.E | `ai-pow --features zk` MED-3 + tile-index roundtrip | ✅ landed |
| **HIGH-2.2 §6(a)** | Fold-schedule pin (verifier reconstructs the stripe schedule) | `2026-05-15_HIGH2_2_DESIGN.md` § 6 | adversarial swap/skip/duplicate stripe ⇒ reject | ✅ landed |
| **HIGH-2.2 §6(b)-G1+G2** | Sweep-input pin (G1: store inputs bound to declared schedule; G2: store reads bound to control prep) | `2026-05-15_HIGH2_2_DESIGN.md` § 6 | adversarial G1/G2 violations ⇒ reject | ✅ landed |
| **M-S1** | §6(b) sweep inputs multiset-bound to a declared `noised_packed` store (no free-cell forgery) | `2026-05-15_HIGH2_2_DESIGN.md` § 7 | adversarial planted free `JACKPOT_MSG` ⇒ reject | ✅ landed |

### 3.3 MAT — matrix binding

| # | Claim | Where argued | Where tested | Status |
|---|---|---|---|---|
| **M52** | A/B matrices are bound via a BLAKE3 chunk-Merkle commitment whose root equals `HASH_A` / `HASH_B` ∈ PI | `2026-05-14_M52_MATRIX_BINDING.md` (Option 1 chunk-Merkle) | ai-pow `commit::matrix_commitment` KAT at 57 344 chunks; `blake3_tree::open_strip` roundtrip | ✅ landed (M12-gated for some tightening) |
| **P-B.2.0** | Off-circuit BLAKE3 true-tree walker; strip-opening primitive at real 57 344-chunk weight scale | `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` § 2 | KAT at the real Llama-8B weight scale | ✅ landed |
| **P-B.2.2** | In-circuit `place_matrix_strip_opening` reuses the *unchanged* C3 binding | `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` § 4 | full `ai-pow-zk --lib` accept + tampered strip ⇒ reject | ✅ landed |
| **P-B.2.3 (A1)** | `tile_chunk_range` = deterministic function of `(params, tile_i, tile_j)`; verifier reproduces from PI | `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` § 3 D3-A | adversarial: opening off attested tile ⇒ reject; byte-reproducible from PI | ✅ landed |
| **P-B.2.4 (A2)** | Bridge swapped from full-matrix `place_matrix_hash_a/b` to `place_matrix_strip_opening`; tile proof fits one STARK (~2²² rows) for Llama-8B-class params | `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` § 5 | `ai-pow --features zk` all-binaries green incl. `end_to_end`; `fits_one_stark()` true for Llama-8B INT GEMMs | ✅ landed (the production unblocker) |

### 3.4 A3 — noise binding (§4.C.2)

| # | Claim | Where argued | Where tested | Status |
|---|---|---|---|---|
| **A3.0** | `noise_ref` cross-crate KAT == `BlockNoise` (Pearl §4.7 preprocessed-noise reference) | `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` § 3 | cross-crate KAT (commit `4c6b3e8`) | ✅ landed |
| **A3.1** | Per-row decomposition KAT (`79f748d`) | same § 4 | per-row KAT | ✅ landed |
| **A3.2a** | Position-addressed witness-free store layout (the conceptual blocker) | same § 5 | layout KAT (commit `41a7005`) | ✅ landed |
| **A3.2b** | Split-store: §4.C.2 *noise* tie CLOSED — store noise forced to `noise_ref` of the C1-public seed | same § 6 | `ai-pow-zk --lib` 351/0/22; `ai-pow --features zk` all 0-failed (commit `5a37c8e`) | ✅ landed |
| **A3.3 / cx.0** | Plain tie: per-row word-pair binds via C3 + CRIT-1 program-pin; store row at `(chunk, block, word_off)` leaf address binds to the exact committed bytes ∈ `HASH_A` (r=16 + r=32) | `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` § 8 (c-exact path) | cx.0 KAT (commit `2bbf4cd`); cx.1 generalization landed (CRIT-1-pinned per-row word-offset via §6(b)/G2 `FOLD_STRIPE_SEL` pattern) | ✅ landed (production 16∣r path) |

### 3.5 C — recursion stack

| # | Claim | Where argued | Where tested | Status |
|---|---|---|---|---|
| **C1 / M-S3** | `Plonky3-recursion` vendored in-tree at C1 fixed point `c2c51fb` (rev-aligned to ai-pow-zk's `6de5cba`) | `2026-05-18_C1_RECURSION_VENDOR_DESIGN.md` | full Plonky3-recursion test suite green at the aligned rev | ✅ landed |
| **C2.1** | 5-round recursive Tip5 permutation AIR, KAT-anchored to `nockchain_math::tip5::permute_5round` (recursive soundness linchpin); canonical non-recursive Nockchain Tip5 remains 7-round `permute` | `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2/§ 3 + 2026-06-03 recursive-only boundary update | `tip5-circuit-air` KAT vs `permute_5round` oracle; `recursive_tip5_adapter_does_not_replace_canonical_nockchain_tip5` | ✅ landed |
| **C2.1 / §2b** | Lookup-free arithmetization (per-byte cube ≡ LOOKUP_TABLE) machine-proved identity per paper §4.6 canonical `<p` guard | `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2b | `c2_0_offset_fermat_cube_identity_machine_check` KAT | ✅ landed (algebraic identity), **superseded operationally by lookup-table** |
| **C2 / lookup-table** | Lookup-table AIR (8.6× narrower; LogUp degree 2 not 226) | `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2c | `tip5-circuit-air` LogUp KAT (commits `a5e7600`, `d97bdb2`, `8233a9e`) | ✅ landed |
| **C2 L4** | Global bus done correctly; LogUp degree 226 → 2 (machine-proven) | `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2c L4 | bus KAT (commit `8233a9e`) | ✅ landed |
| **C2 L5** | In-circuit Tip5 challenger duplexing + MMCS path bit-for-bit vs native | `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2c L5 | bit-for-bit KAT vs native (commit `259dd6f`) | ✅ landed |
| **C2.4** | Real 5-round Tip5 Layer-0 end-to-end recursion verify + production ≥60-bit FRI profile | `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2c.C2.4 + current `goldilocks_tip5_60bit` profile | `recursion/tests/test_tip5_layer0_recursion.rs` accept + tamper-reject across the production profile | ✅ landed |
| **C2.4 R-a** | `WitnessChecks` CTL D=1 byte-identical re-validated; D-aware infrastructure landed | `2026-05-19_C3_OUTER_CERT_DESIGN.md` (the C2.4 R-a tail context) | D=1 byte-identical re-validation; D=5 quintic arbiter (commit `632cb8c`) | ✅ landed |
| **C3 / M-S5 production recursive cert** | Soundness-correct production vertical-recursion cert; every consensus-facing link is ≥60 bits unconditional Johnson, with current outer profile `lb=4, nq=9, query_pow=24` ⇒ 60 bits (§1.3) | `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13.2 + current production-profile docs | `test_tip5_layer0_compression.rs` / production recursion profile tests and `prod_recursion_measure` size/timing runs | ✅ landed; ≤100 KB terminal compression remains open |
| **DT-4 duplex binding** | Merkle-swap slot↔idx desync fix: capture pre-swap `bus_state` for `!has_ctl_output` perms; net-0 duplex binding | `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13 | `crates/plonky3-recursion/circuit/src/ops/tip5_perm/executor.rs` (commit `14116b0`); tamper-reject via `WitnessConflict` at `runner().run()` | ✅ landed (non-fenced executor edit; zero multiplicity changed; Merkle-root binding bit-for-bit untouched) |

### 3.6 ENV / P-A — production envelope

| # | Claim | Where argued | Where tested | Status |
|---|---|---|---|---|
| **P-A** | Pearl §4.8 envelope (`validate_prod_envelope` + universal `k·(h+w) ≤ 2²²`); real `LLAMA_3_1_8B_*` presets in-envelope | `2026-05-17_PRODUCTION_ROADMAP.md` § 1 + `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` | ai-pow `validate_prod_envelope` KAT + real preset acceptance | ✅ landed |
| **B0/B3** | INT-only production scoping; `LLAMA_3_1_8B_DOWN` FP8 mis-doc fixed | `2026-05-18_PHASE_B_DESIGN.md` § B0/§ B3 | `pearl_compat_fixtures` 11/0/0; `pearl_model_compat` 8/0/0 | ✅ landed |
| **B2-contract** | `ai-pow::quant` bit-lossless quant contract | `2026-05-18_PHASE_B_DESIGN.md` § B2 | `quant` 4/4 + `b3_*` 3/3 | ✅ landed |
| **B1-audit / B1.1** | Vendored reference ≡ real `pearl/zk-pow` (line-for-line); 16 GB real weights byte-process under the audited pipeline at real μ | `2026-05-18_B1_PEARL_FAITHFULNESS_AUDIT.md` | `pearl_model_compat` on real `gate_proj` INT7 weights at real μ | ✅ landed |

---

## 4. Recursion-stack inventory (the C1 → C3 chain)

### 4.1 Layered structure

```
                ┌─────────────────────────────────────────────┐
                │   M-S5  ≥60-bit outer-recursive STARK cert  │
                │   (test_tip5_layer0_compression.rs)         │
                │   current L1 cert ≈ 200.6 KiB fixed-bincode │
                │   M-S5b target remains ≈100 KiB             │
                └────────────────────┬────────────────────────┘
                                     │ verifies
                                     ▼
                ┌─────────────────────────────────────────────┐
                │   C2.4  in-circuit Tip5-L0 verifier         │
                │   (test_tip5_layer0_recursion.rs;           │
                │   verify_p3_batch_proof_circuit)            │
                │   production ≥60-bit profile, accept+tamper │
                └────────────────────┬────────────────────────┘
                                     │ verifies
                                     ▼
                ┌─────────────────────────────────────────────┐
                │   Inner Tip5-L0 STARK of the mineable unit  │
                │   (ai-pow-zk CompositeFullAirPinned)        │
                │   matmul + fold + BLAKE3 keyed_hash chain   │
                └─────────────────────────────────────────────┘
```

### 4.2 Substrate inventory

| Layer | Crate / file | Vendored rev | Audit anchor |
|---|---|---|---|
| Goldilocks field, FRI, MMCS | upstream Plonky3 | rev aligned at `c2c51fb` to ai-pow-zk's `6de5cba` | upstream audited (Plonky3 community) |
| Recursion frontend | `crates/plonky3-recursion/` (vendored, **excluded workspace**) | C1 fixed-point `c2c51fb` | `2026-05-18_C1_RECURSION_VENDOR_DESIGN.md` |
| Tip5 perm AIR (linchpin) | `crates/plonky3-recursion/tip5-circuit-air/` | included via C1 | C2.1 KAT vs recursive `nockchain_math::tip5::permute_5round`; canonical `permute` remains 7-round outside recursion |
| Tip5 lookup table | `crates/plonky3-recursion/circuit-prover/.../tip5/` (LogUp arms) | included | C2 L4 bus correctness (`8233a9e`) |
| In-circuit Tip5 challenger duplexing | `crates/plonky3-recursion/recursion/src/challenger/circuit.rs` | included | C2 L5 (`259dd6f`) |
| In-circuit MMCS path | `crates/plonky3-recursion/recursion/src/mmcs/circuit.rs` | included | C2 L5 |
| In-circuit batch-STARK verifier | `crates/plonky3-recursion/circuit-prover/.../batch_stark_prover/` (`verify_p3_batch_proof_circuit`) | included | C2.4 (`fb0bd32`) |
| `WitnessChecks` CTL D-aware | `crates/plonky3-recursion/circuit-prover/.../recompose.rs` | included | C2.4 R-a (`632cb8c`) |
| **DT-4 duplex-binding fix** | `crates/plonky3-recursion/circuit/src/ops/tip5_perm/executor.rs` | included | C3 DT-4 (`14116b0`) |
| Inner mineable-unit AIR | `crates/ai-pow-zk/src/composite/full_air_pinned.rs` (`CompositeFullAirPinned`) | in main workspace | HIGH-2.2 + M-S1 + §4.C.2 |

### 4.3 Standing decoupling invariant (C1)

`Plonky3-recursion` is a **separate Cargo workspace, excluded
from ai-pow-zk**. Recursion crates **must not depend on
ai-pow-zk** (this is the C1 isolation invariant). Any future
C2.x extension that needs to know about Tip5 semantics must
edit the in-tree recursion crates directly (the hash is closed
Poseidon1/2-shaped, not generic — C2 already does this for
Tip5).

---

## 5. Tip5 AIR claims (the soundness linchpin)

### 5.1 What is being claimed (precise)

The Tip5 permutation AIR (`crates/plonky3-recursion/tip5-circuit-air/`)
implements the recursive-proving-only 5-round Tip5 permutation
**identical bit-for-bit** to `nockchain_math::tip5::permute_5round`
over Goldilocks⁸, instantiated per the Tip5 paper (ePrint IACR
ePrint 2023/107) §4.3/§4.6. The canonical non-recursive Nockchain
hash remains `nockchain_math::tip5::permute` with 7 rounds; the
recursive adapter must not replace it.

Components:

| # | Component | Where | Verified by |
|---|---|---|---|
| **5.A** | Tip5 round constants (5-round recursive prefix) | `tip5-circuit-air/src/tip5_spec.rs` | byte-for-byte vs the paper's tabulation + `nockchain_math::tip5::permute_5round` (KAT) |
| **5.B** | Per-byte cube `<p` canonical guard | `tip5-circuit-air/src/air.rs` § cube | algebraic identity machine-checked: `(x mod p)³ mod p == cube_table[x mod p]`; lookup-table arm enforces `0 ≤ x < p` |
| **5.C** | MDS matrix (Tip5 §4.5) | `tip5-circuit-air/src/mds.rs` | KAT vs paper + native |
| **5.D** | Lookup-table arm (LogUp degree-2) | `tip5-circuit-air/src/lookup.rs` + `circuit-prover/.../tip5/` | C2 L4 global-bus correctness (`8233a9e`); LogUp degree machine-proven 2 (not 226) |
| **5.E** | Round wiring (input → cube → MDS → next-round) | `tip5-circuit-air/src/air.rs` | per-row KAT |

### 5.2 Why no separate lookup-free path is required

§ 2b of `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` originally
laid out a *lookup-free* algebraic arithmetization (per-byte
cube ≡ LOOKUP_TABLE via the C2.0 machine-proved identity + the
paper §4.6 canonical `<p` guard). § 2c then **corrected** that
the width was unacceptable for production and chose the
**lookup-table** arm. The lookup-free identity was **retained
as a machine-proved soundness argument** — it proves the
lookup-table arm is semantically the same constraint — but the
operational AIR is the lookup-table arm.

**Auditor note:** both arms reduce to the same algebraic
predicate; § 2b is the soundness anchor, § 2c is the
operational implementation. Auditing the lookup-table arm
suffices for both.

### 5.3 Native ↔ in-circuit equivalence

C2.1's KAT against `nockchain_math::tip5::permute_5round` is the
recursive **oracle**. The KAT iterates a published-large set of inputs
(deterministic + random) and asserts every Tip5 output column
matches bit-for-bit against the frozen native oracle. The
frozen oracle is restricted to the recursive proving stack; the
separate test `recursive_tip5_adapter_does_not_replace_canonical_nockchain_tip5`
asserts the canonical 7-round Nockchain hash remains unchanged.

---

## 6. KAT / test-vector inventory (the auditor's reproducibility kit)

### 6.1 Tip5 KATs

| KAT | File | What it verifies |
|---|---|---|
| `tip5_air_kat_*` | `crates/plonky3-recursion/tip5-circuit-air/src/lib.rs` (test mod) | Recursive Tip5 5-round AIR per-row vs `nockchain_math::tip5::permute_5round` |
| `test_tip5_lookups` | `crates/plonky3-recursion/recursion/tests/test_tip5_lookups.rs` | LogUp bus correctness |
| `recursive_tip5_adapter_does_not_replace_canonical_nockchain_tip5` | `crates/ai-pow-zk/src/circuit.rs` | Recursive 5-round adapter is isolated from canonical 7-round Nockchain Tip5 |
| `test_tip5_layer0_recursion` | `crates/plonky3-recursion/recursion/tests/test_tip5_layer0_recursion.rs` | C2.4: in-circuit Tip5-L0 verifier accept + tamper-reject under the production ≥60-bit profile |
| `test_tip5_layer0_compression` | `crates/plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs` | C3 / M-S5: production recursive cert accept + tamper tests |

### 6.2 ai-pow byte-equivalence KATs

| KAT | File | What it verifies |
|---|---|---|
| `pearl_compat_fixtures` | `crates/ai-pow/tests/pearl_compat_fixtures.rs` | Pearl-faithfulness fixtures (B0/B3) |
| `pearl_model_compat` | `crates/ai-pow/tests/pearl_model_compat.rs` | Real `Llama-3.1-8B-Instruct-pearl` `gate_proj` INT7 weights byte-process |
| `quant` (4/4) | `crates/ai-pow/src/quant.rs` (#[cfg(test)]) | B2-contract bit-lossless |
| `b3_*` (3/3) | `crates/ai-pow/src/...` | BLAKE3 chip vs spec |

### 6.3 ai-pow-zk soundness KATs

| KAT | File | What it verifies |
|---|---|---|
| `crit1_*` | `crates/ai-pow-zk/src/.../crit1_tests.rs` (per the canonical-program design) | Any forged PROGRAM_COL ≠ canonical ⇒ reject |
| `high2_2_*` | `crates/ai-pow-zk/src/.../high2_2_tests.rs` | Honest matmul→fold→C4-hash chain; planted free `JACKPOT_MSG` ⇒ reject |
| §4.C.2 cx.0 KAT | per `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` § 8 | Store row position-exact binds to committed bytes ∈ `HASH_A` |
| `end_to_end` | `crates/ai-pow --features zk` binary `end_to_end` | Full mining roundtrip incl. MED-3 |

### 6.4 Recursion regression slice

Recent focused regression slice (from this session's
verification artifacts):

| Suite | Count | Time |
|---|---|---|
| `cargo test --manifest-path crates/plonky3-recursion/recursion/Cargo.toml terminal --lib` | 18/0/0 | focused terminal relation tests |
| `cargo test --manifest-path crates/plonky3-recursion/recursion/Cargo.toml --test test_l1_outer_cert_tip5_unified terminal_compiler_covers_real_tip5_l0_verifier_circuit` | 1/0/0 | real L1 verifier-circuit coverage |
| `cargo test -p ai-pow-zk --features recursion non_first_subslice_split_view_rejected_by_i8u8_logup` | 1/0/0 | 21.02 s |
| `cargo test -p ai-pow-zk --features recursion non_first_subslice_mat_freq_drop_rejected_by_logup` | 1/0/0 | 4.44 s |

---

## 7. Adversarial-test inventory (what we reject)

The audit should reproduce these and add more. Each row
is `(attack class → test that already rejects it)`.

| Attack class | Concrete test | Source |
|---|---|---|
| A-PROGRAM (forge AIR/program) | `crit1_*` adversarial: any PROGRAM_COL ≠ canonical ⇒ reject | CR.6 + § 3.1 |
| A-NOISE (forge noised inputs) | `§4.C.2`/A3.2b store row ≠ `noise_ref(seed)` ⇒ reject | A3.2b commit `5a37c8e` |
| A-SWAP (swap A/B between strips) | adversarial swapped strip ⇒ reject (`HASH_A`/`HASH_B` mismatch) | M52 + P-B.2.2 |
| A-SWAP (skip/duplicate stripe) | adversarial skip/dup ⇒ reject via §6(b)/G2 | HIGH-2.2 § 6 |
| A-TILE (cheaper tile than attested) | opening off attested tile ⇒ reject pinned schedule | P-B.2.3 / A1 |
| A-MAT (forged committed matrix) | tampered `HASH_A` ≠ attested ⇒ reject | M52 |
| A-CHAIN (forge recursion) | tampered inner proof ⇒ `WitnessConflict` at `runner().run()` | C2.4 + C3 DT-4 tamper tests |
| A-SOUND (sub-60-bit production configuration) | production profile asserts Johnson bits `lb·nq + query_pow ≥ 60` | `goldilocks_tip5_60bit` config test |
| A-CONSTRAINT (64-byte split-view) | later sub-slice `MAT_UNPACK`/`UINT8_DATA` split ⇒ LogUp reject; dropped later-sub-slice `MAT_FREQ` ⇒ LogUp reject | `non_first_subslice_split_view_rejected_by_i8u8_logup`; `non_first_subslice_mat_freq_drop_rejected_by_logup` |
| A-MAT (planted free `JACKPOT_MSG`) | `high2_*` adversarial ⇒ reject (M-S1) | HIGH-2.2 § 7 |
| A-FRI (tampered FRI fold) | `WitnessConflict` at the in-circuit FRI fold-chain `connect` | C3 DT-4 + § 15 |
| **A-CONSTRAINT** (per-AIR constraint-family tamper at single-cell / selector / bus / cross-AIR / property-based levels) | Every constraint family in the M-S5 chain has ≥1 tamper test exercising its rejection mechanism (M1/M2/M3/M4/M5). Per-AIR + per-bus rejection rate empirically 1.0. | CSA S0–S7 deliverables: `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`, `2026-05-20_CSA_S7_AUDIT_SIGNOFF.md` § 5 (per-AIR sign-off table) |

---

## 8. Known residuals (explicit, no hidden gaps)

Honest list, R1-disciplined. Anything the audit might
otherwise be "surprised" by is here.

| Residual | What it is | Where tracked |
|---|---|---|
| **M-S5b / `#131`** | ≈100 KB terminal compression of the production M-S5 cert (size target only; soundness floor remains ≥60-bit Johnson). Current fixed-bincode production recursive certificate is ≈200.6 KiB at `lb=4, nq=9, query_pow=24, cap_height=5`. | `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md` |
| **Phase B1** | Pearl **reference vectors** from Pearl's miner (golden `(κ,s_a,s_b,E/F,one tile digest)`); today only self-consistency vs ai-pow's own plain path is tested | `2026-05-18_PHASE_B_DESIGN.md` § B1; `2026-05-13_PEARL_COMPARISON.md` |
| **Phase B2** | Quant-extraction contract: specify how the vLLM plugin maps the model's INT7/INT8 GEMM operands to Pearl type-0 `[−64,64]` int8 `(A,B,μ)`; integration KAT against a real model fixture | `2026-05-18_PHASE_B_DESIGN.md` § B2 |
| **Packed-MMCS `GoldilocksConfig`** | Landed config is unpacked; `verify_p3_batch_proof_circuit` requires packed; aarch64-neon `Goldilocks::Packing ≠ Goldilocks`. Verified-soundness-neutral substitute used in measurement; production L2 needs the upstream fix or a packed-MMCS sibling. | `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 14 |
| **R-b** (M12 / `#127`) | ai-pow-zk's actual M10.1c composite `RecursiveAir` vs the representative `FibonacciAir` in the recursion harness | `2026-05-14_M10_1C_DESIGN.md`; M12 task |
| **G3** carry-vector segmentation | Deferred — this model is in `k ≤ 2¹⁶` Pearl envelope; revive only if a workload exceeds the envelope | `2026-05-17_M_S2_G3AB_DESIGN.md` |
| **FP8 PoUW** | Pearl's FP protocol unshipped; INT-only production scope (documented limitation, not a defect) | `2026-05-17_PRODUCTION_ROADMAP.md` § 0 |
| **D1 / D2** | vLLM miner-plugin + consensus block-cert integration — external | roadmap Phase D |
| **CSA S0–S7 — LANDED 2026-05-20** | Constraint Soundness Analysis: AIR-side companion to S(−1). All 8 stages landed: S0 (inventory: ~117 families × 27 AIRs × ~190 existing tamper tests), S1 (Plonky3 STARK + Habock LogUp derivation: per-AIR MIN = 98 bits at BUS_IRANGE8, per-AIR MIN otherwise ≥ 103, chain MIN = 82 with S(−1) FRI), S2 (4-item GAP-G2/G1 backlog + 3 M12-deferred items routed), S3 (tamper-test specs per template), S4 (9 new tamper tests + 3 audit-routing doc-comments, all CI green), S5 (1 new K3 producer-side cross-AIR test + 7 boundary disposition rows), S6 (1 new demo proptest + 4 existing covered; deeper sweep deferred-as-deepening), S7 (audit sign-off + per-AIR table). **Verdict**: every AIR + bus ≥80 unconditional with margin; combined chain MIN ≥82. Deferred residuals (deepening, not gaps): F3–F20 fine-grained FRI fold-round tampers, per-constraint proptest sweep. | `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md` (design) + `2026-05-20_CONSTRAINT_INVENTORY.md` (S0) + `2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md` (S1) + `2026-05-20_TAMPER_GAP_LIST.md` (S2) + `2026-05-20_TAMPER_TEST_SPECIFICATION.md` (S3) + `2026-05-20_CSA_S5_CROSS_AIR_TAMPER_TESTS.md` (S5) + `2026-05-20_CSA_S6_PROPERTY_BASED_TESTS.md` (S6) + `2026-05-20_CSA_S7_AUDIT_SIGNOFF.md` (S7) |

### 8.1 Non-residuals (claims a careless audit might list — preempted here)

- "C3 incomplete" — C3 is the soundness-correct production cert,
  LANDED + independently re-validated. The ≈100 KB *size* target
  is a **separate carved-out milestone (M-S5b)**. This is not
  hidden C3 incompleteness. See § 8 of
  `2026-05-19_C3_OUTER_CERT_DESIGN.md`.
- "§4.C.2 has a gap" — production is always `16 ∣ r` (Pearl
  §4.8); the production path is **zero-gap**. The non-16∣r
  TEST geometry is A3.2b documented strictly-stronger-than-
  pre-A3 — not a forgery hole. Memory + § 8 of the §4.C.2
  design doc.
- "CRIT-1 is `extract`-of-reference" — that was the *old* model;
  Phase A-CR (CR.0–CR.7) flipped the verify path to a
  witness-free params-pure `canonical_program(params, block_public)`.

---

## 9. Out-of-scope notes (for the auditor's record)

| Item | Why out of scope | Where to look later |
|---|---|---|
| Pearl's vLLM plugin code | Owned by Pearl, audited separately | Pearl whitepaper §5; Pearl repo |
| FP8 layer security | Pearl FP PoUW unshipped | wait for Pearl FP spec |
| Phase D (consensus integration) | External wiring; not ai-pow-zk's stack | `2026-05-17_PRODUCTION_ROADMAP.md` Phase D |
| Native terminal-compression backend | Out of this audit package until it replaces the current production recursive cert; current relation compiler and binding digest are tracked separately | `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md` |
| Nockchain consensus layer | Out of scope for this proof-system audit | Nockchain consensus docs |

---

## 10. Audit-readiness checklist

Before the auditor begins, confirm:

- [x] **Soundness-claim index complete** (§ 3 above — every
      CRIT/HIGH/MAT/A3/C/ENV claim has a design doc and a test).
- [x] **Recursion stack inventoried** (§ 4).
- [x] **Tip5 AIR claims separately laid out** (§ 5 — the
      linchpin).
- [x] **KAT / test inventory documented** (§ 6).
- [x] **Adversarial tests catalogued** (§ 7).
- [x] **Residuals explicitly listed** (§ 8; no hidden gaps).
- [x] **Out-of-scope explicitly listed** (§ 1.2, § 9).
- [x] **Threat model documented** (§ 2).
- [x] **Standing R1 discipline declared** (no rushing; staged
      validated commits; precise residuals — `~/.claude/CLAUDE.md`
      R1/R1.1).
- [x] **Soundness bar paper-grounded** (production ≥60-bit
      unconditional Johnson floor under the time-bounded PoW threat
      model; §1.3).
- [x] **Per-layer `γ < J(δ)−η` table produced** (terminal-compression
      prerequisite; landed 2026-05-20 in
      `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` §4.3, with chain
      MIN ≥ 82 unconditional under the combined
      per-query + proximity-loss accounting).
- [ ] **Pearl B1 reference vectors obtained** (B1 still open;
      this is a known residual not a blocker for starting the
      audit — the auditor can begin on the in-scope items and
      revisit B1 when the reference vectors arrive).
- [ ] **Packed-MMCS `GoldilocksConfig` substrate decision**
      recorded (either upstream patch or sibling config, per
      § 8 above).
- [ ] **Native terminal-compression backend complete** (tracked in
      `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`); not a
      blocker for this C4 audit package, but required before claiming
      the ≈100 KiB certificate target.

The three open items are honest residuals, not blockers. The
audit can begin on the in-scope items as listed.

---

## 11. Reference doc map (where to find everything)

| Topic | Doc |
|---|---|
| Definitive soundness report | `2026-05-15_ZKP_SECURITY_REPORT.md` |
| Gap tracker | `2026-05-15_GAP_AUDIT.md` |
| Engineering rationale | `2026-05-14_ENGINEERING_REPORT.md` |
| Production roadmap | `2026-05-17_PRODUCTION_ROADMAP.md` |
| Base AIR / per-slot design | `2026-05-13_DESIGN.md` |
| Profiling methodology | `2026-05-15_PROFILING.md` |
| CRIT-1 canonical_program | `2026-05-17_CANONICAL_PROGRAM_DESIGN.md` |
| §4.C.2 noise binding | `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` |
| HIGH-2.2 matmul→fold→C4 chain | `2026-05-15_HIGH2_2_DESIGN.md` |
| M52 matrix binding | `2026-05-14_M52_MATRIX_BINDING.md` |
| P-B.2.x strip opening | `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` |
| C1 recursion vendor | `2026-05-18_C1_RECURSION_VENDOR_DESIGN.md` |
| C2 Tip5 circuit AIR | `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` |
| C2 degree/width tradeoff | `2026-05-18_C2_TIP5_AIR_DEGREE_WIDTH_TRADEOFF.md` |
| C3 outer-cert (DT-1→DT-4 + LANDED) | `2026-05-19_C3_OUTER_CERT_DESIGN.md` |
| M-S5b terminal compression (sibling) | `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md` |
| **M-S5b S(−1) paper-grounded soundness analysis (closes A-LDR / §10 γ<J(δ)−η item)** | `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` |
| **Constraint Soundness Analysis (CSA) — staged design (AIR-side of ≥80 unconditional)** | `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md` |
| **CSA S0 — master constraint inventory (every AIR × every constraint family × tamper-test cross-link)** | `2026-05-20_CONSTRAINT_INVENTORY.md` |
| **CSA S1 — per-constraint Schwartz–Zippel + LogUp derivation; per-AIR ≥80 verified** | `2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md` |
| **CSA S2 — tamper-coverage gap list (G1/G2/G3 + M12 deferrals)** | `2026-05-20_TAMPER_GAP_LIST.md` |
| **CSA S3 — tamper-test specifications (per S2 backlog × §4.2 template)** | `2026-05-20_TAMPER_TEST_SPECIFICATION.md` |
| **CSA S5 — cross-AIR composition tamper tests (per-boundary bidirectional coverage)** | `2026-05-20_CSA_S5_CROSS_AIR_TAMPER_TESTS.md` |
| **CSA S6 — property-based tampering (demo + deferred-as-deepening sweep)** | `2026-05-20_CSA_S6_PROPERTY_BASED_TESTS.md` |
| **CSA S7 — audit sign-off (per-AIR table + GAP_AUDIT routing)** | `2026-05-20_CSA_S7_AUDIT_SIGNOFF.md` |
| **M-S5b S1 — comprehensive proof-size-reduction routes audit (8 paths × Pearl/Plonky2 deep-dive × per-path quantitative comparison)** | `2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md` |
| **M-S5b S1.B Poseidon2-removal spec (Tip5-unified outer-cert; P0–P7 phased plan; predicted ~18–27 KB L2 floor savings)** | `2026-05-20_POSEIDON2_REMOVAL_SPEC.md` |
| Phase B byte-equivalence | `2026-05-18_PHASE_B_DESIGN.md` |
| Pearl divergence inventory | `2026-05-13_PEARL_COMPARISON.md` |
| Phase-B B1 audit | `2026-05-18_B1_PEARL_FAITHFULNESS_AUDIT.md` |
| Pearl FP8 scoping | `2026-05-18_PEARL_FP8_SCOPING.md` |
| vLLM CPU fork design | `2026-05-18_PEARL_VLLM_CPU_FORK_DESIGN.md` |
| G3 (deferred) | `2026-05-17_M_S2_G3AB_DESIGN.md` |
| Pearl 3-layer recursion (origin of the historic ≤65 KB target; relaxed to ≤100 KB 2026-05-21) | `2026-05-17_M_S2_PEARL_EVALUATION.md` |
| **Soundness-bar anchor paper** (Johnson-radius proven; §1.3) | IACR ePrint 2025/2055 — Ben-Sasson, Carmon, Habock, Kopparty, Saraf, *"On Proximity Gaps for Reed–Solomon Codes"* (Nov 2025; Theorem 1.5 + §1.3.2 + §8 attacks) |
| Tip5 paper (5.A round constants + §4.3/§4.6) | IACR ePrint 2023/107 |
| Earlier roadmap (superseded) | `2026-05-13_ROADMAP.md` |
| Earlier flaws audit (resolved) | `2026-05-13_FLAWS.md` |
| BLAKE3 chip bug writeup | `2026-05-15_BLAKE3_CHIP_ROUND_GATE_BUG.md` |
| M10.1c design | `2026-05-14_M10_1C_DESIGN.md` |
| M10.1c progress | `2026-05-14_M10_1C_PROGRESS.md` |
| G3 recursion aggregation | `2026-05-17_G3_RECURSION_AGGREGATION.md` |
| G3 recursion audit | `2026-05-17_G3_RECURSION_AUDIT.md` |
| M-S2 G3-A/B design | `2026-05-17_M_S2_G3AB_DESIGN.md` |

Each doc carries a `created · last updated` header line, dates
derived from git.

---

## 12. Definition of done — when C4 / M-S6 is closed

This package is "ready for audit." **C4 / M-S6 closes when:**

1. The team's in-house audit has independently walked the
   soundness-claim index (§ 3), reproduced every KAT (§ 6),
   exercised every adversarial test (§ 7), and produced an
   in-house audit log recording either "claim defensible per
   evidence X" or "open finding routed to
   `2026-05-15_GAP_AUDIT.md` with R1 residual."
2. Any soundness gaps surfaced (by us or by anyone else
   auditing the code) are tracked in
   `2026-05-15_GAP_AUDIT.md` with the same R1 discipline
   (validated subset + precise residual per finding).
3. The "experimental / unaudited" gate is removed from the
   recursion stack per the roadmap exit gate.

Until items 1–3 are all honestly green, C4 is **in progress**.
This document being committed flips `#125` from `pending` to
`in_progress` (audit-readiness + start-of-in-house-audit
stage), not `completed`.
