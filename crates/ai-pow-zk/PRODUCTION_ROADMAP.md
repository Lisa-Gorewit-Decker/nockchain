# Production Roadmap — mining `Llama-3.1-8B-Instruct-pearl` end to end

> **Status:** PLANNING (2026-05-17). The authoritative *sequence*
> from today's state to production mining of the real shipped
> Pearl model. Detailed designs live in the referenced docs;
> this is the ordered execution wrapper that ties the SNARK
> (Track A), economics (Track B), correctness, and integration
> (Track C) together.
> **Authoritative cross-refs:** `HIGH2_2_DESIGN.md` §7 (Track-A
> milestone table), `M_S2_PEARL_EVALUATION.md` (γ decision —
> Pearl doesn't segment), `P_B2_STRIP_OPENING_DESIGN.md`
> (P-B.2.x), `M_S2_G3AB_DESIGN.md` (G3 — deferred).

---

## 0. Scope & the production model

The production target is the shipped, Pearl-certified
**`pearl-ai/Llama-3.1-8B-Instruct-pearl`** served via the Pearl
**vLLM mining plugin**: the model's GEMMs become the PoUW
matmuls; on a winning tile the miner emits a Nockchain PoW + a
SNARK.

**Standing architecture (do not relitigate):** only the
**mineable unit** (the plain `TileState`/keyed-`BLAKE3` fold) is
**byte-equivalent to Pearl**. The **SNARK is Nockchain's own
Plonky3 stack** — *not* Pearl's Plonky2 proof. Merge-mining: a
miner that clears the PoW difficulty generates a ZKP separately
for Nockchain (or Pearl). So "mined to production" means: for
this model's GEMMs, ai-pow computes the Pearl-byte-equivalent
mineable unit, and ai-pow-zk produces a *sound, scalable*
Nockchain SNARK of it, consumed by Nockchain consensus.

**Hard scoping fact (model-specific).** `config.json` is
mixed-precision `quant_method:"pearl"`: **group_1 = INT7
channel** (`gate_proj`, `up_proj`, `o_proj`, late-layer qkv) and
**group_0 = FP8 block[128,128]** (`down_proj`, early-layer qkv).
Pearl whitepaper §4.1 fixes matmul-accumulate **type-0 = INT
only** (`[−64,64]`, int32 accumulate); §1.1 defers an FP PoUW to
a future, **not-yet-shipped** upgrade. ⇒ **Production mines this
model's INT GEMMs only** (the bulk of FFN/attn-proj work);
`down_proj`/early-qkv (FP8) are out of scope until Pearl's FP
protocol ships. This is a documented production limitation, not
a defect, and is *not* on the critical path below.

---

## 1. Status — what is already done

| Area | State |
|---|---|
| §4.C soundness foundation: C1–C4, CRIT-1 program-pin, HIGH-2 keystone, §4.A–§4.E, §6(a) fold-schedule pin, §6(b)-G1+G2 | ✅ done |
| MED-3 (verifier-derived difficulty/tile) | ✅ done |
| **M-S1** — §6(b) sweep inputs multiset-bound to a declared `noised_packed` store | ✅ done |
| **P-A** — Pearl §4.8 envelope (`validate_prod_envelope` + universal `k·(h+w)≤2²²`); real `LLAMA_3_1_8B_*` presets in-envelope | ✅ done |
| **P-B** — params-driven Layer-0 sizing + go/no-go (full-matrix hash is the blocker; sweep fits) | ✅ done |
| **P-B.2.0** — off-circuit BLAKE3 true-tree walker + strip-opening primitive; KAT'd at the real 57 344-chunk weight scale; D1 latent-gap disproven (⇒ P-B.2.1 subsumed) | ✅ done |
| **P-B.2.2** — in-circuit `place_matrix_strip_opening` (reuses the unchanged C3 binding, no AIR change) | ✅ done |
| γ decision — Pearl-faithful (no segmentation); **G3 deferred** (only if a load beyond Pearl's `k≤2¹⁶`; this model is in-envelope so G3 is *not* needed) | ✅ decided |

---

## 2. The sequence to production

Five phases. **Phase A is the hard critical path** (it alone
makes the SNARK able to mine this model soundly at scale).
Phase B is the correctness gate. Phase C makes the proof a
consensus artifact. Phase D is integration (kept deliberately
vague — external). Phase E is cross-cutting.

### Phase A — close the SNARK at real-model scale (ai-pow-zk; critical path)

> **Status 2026-05-17.** **A1 ✅** (`3629444`,
> `tile_chunk_range`). **A2 ✅ — the production unblocker, fully
> landed & validated** (`3629444`): bridge swapped to
> strip-opening at the A1 schedule; `fits_one_stark()` flips
> **true for Llama-3.1-8B** (and PROD/GEMMA/QWEN); full
> `ai-pow --features zk` 0-failed across every binary incl.
> `end_to_end` + MED-3 roundtrip through the swap; zero
> regression. **A3:** A3.0 ✅ (`4c6b3e8`, `noise_ref` +
> cross-crate KAT == `BlockNoise`) + a **major design
> correction** (`5cf8e51`): §4.C.2 is Pearl-§4.7 *preprocessed
> noise* (reuse CRIT-1 `NOISE_PACKED_PREP` + InputChip + C3 +
> `noise_ref`), **not** a PRNG sub-AIR. **A3.1 ✅** (`79f748d`
> per-row decomp KAT), **A3.2a ✅** (`41a7005` position-
> addressed witness-free store layout — the conceptual core
> blocker, solved), **A3.2b ✅** (`5a37c8e` split store: §4.C.2
> **noise tie CLOSED** — store noise forced to `noise_ref` of
> the C1-public seed; `ai-pow-zk --lib` 351/0/22, `ai-pow
> --features zk` all-binaries 0-failed incl. end_to_end +
> MED-3). **Remaining = B1 (the *plain* tie). DECISION
> 2026-05-17 (maintainer): done via c-exact — the c-mset
> `BUS_PLAIN` bus is ABANDONED.** c-mset shipped only as the
> KAT-first de-risk arc: **c-mset.0 ✅ + c-mset.1a ✅**
> (`2c2d7c6`) proved `consumer ⊆ producer` holds **iff `16|r`**
> (Pearl always `16|r`; `TEST_SMALL` r=4 proven negative) **and**
> that the bus needs invasive CRIT-1-program producer-row gating
> regardless. ⇒ maintainer chose **c-exact** (co-locate store
> onto strip-opening leaf rows; the *proven* C3 binds
> `MAT_UNPACK` to the exact committed bytes ∈ `HASH_A`;
> position-exact zero-gap; no new bus/FREQ/permutation, no open
> producer-isolation decision — strictly dominates c-mset).
> c-mset.0/.1a retained as the de-risk + the
> contiguity/`16|r`-alignment evidence c-exact reuses. Staged
> KAT-first plan **cx.0→cx.3 → A3.3**. **cx.0 ✅ DONE &
> validated** (`2bbf4cd`): KAT proved every position-addressed
> store row binds — via the exact C3 identity at a witness-free
> `(chunk,block,word_off)` leaf address — to the exact committed
> bytes ∈ `HASH_A` (r=16 + r=32). **Next = cx.1** (first
> invasive AIR stage): generalize the proven C3 to a
> CRIT-1-pinned per-row word-offset via the proven §6(b)/G2
> `FOLD_STRIPE_SEL` pattern (CONTROL_PREP index + 8-wide
> one-hot block; zero *preprocessed*-width, zero-blast at
> `o=0`); §6(a)/§6(b)/G2-scale ⇒ multi-commit, per-sub-stage
> validated cx.1a→cx.1c (R1, not rushed).
> §4.C.2-with-A3.2b is already strictly stronger than
> pre-A3 and **not a forgery hole** (CRIT-1 + §4.D + §6 + M-S1
> + A2 + the noise pin hold). The production-critical unblocker
> (A2) is done. Full comparison + c-exact design + cx.0–cx.3:
> `SEC_4C2_NOISE_BINDING_DESIGN.md` §8.

| # | Item | Depends | Exit gate |
|---|---|---|---|
| **A1** | **P-B.2.3** — verifier-fixed opening schedule: `(c0,c1,num_chunks, auth-tree)` a pure deterministic function of `(params,tile_i,tile_j)`, recomputed by the verifier via the CRIT-1/MED-3 discipline (no new pinned column, D3-A). | P-B.2.2 ✅ | A `crit1_*`-style adversarial test: an opening located off the attested tile (cheaper/zero region) fails the pinned-schedule check; the schedule is byte-reproducible from public params. |
| **A2** | **P-B.2.4 — the production unblocker.** Swap `zk_bridge::prove_and_verify_tiled` from the full-matrix `place_matrix_hash_a/b` to `place_matrix_strip_opening` (tile strips from the padded committed matrix + `blake3_tree::open_strip` siblings + `commit::matrix_commitment` for the PI). Update `expected_layer0_rows` (`mhash`→`O(t·k)`); `fits_one_stark()` flips **true** for the Llama-8B INT GEMMs. | A1 | Full `ai-pow --features zk` green incl. MED-3 roundtrip at a **Llama-8B-class INT param** (e.g. `LLAMA_3_1_8B_GATE_UP`); the tile proof fits one STARK (`≤2²²`) and verifies; prover wall-cost measured (~16 min budget, amortized). **Until A2 the SNARK cannot mine this model at all.** |
| **A3** | **§4.C.2** — bind the store to the *committed plain strips* via the in-circuit noise derivation (close the last §4.C link: committed plain `A`/`B` → `noise(·)` → `noised_packed` store → M-S1 sweep → fold → digest, all in one STARK). | M-S1 ✅, A2 | Adversarial: swept noised strips not equal to `noise(committed strips)` under the verifier-pinned noise schedule must reject. Full §4.C soundness chain holds end-to-end, **zero probabilistic gap** (no spot-check, no G3). |

After Phase A: the Nockchain SNARK soundly proves a real
Llama-8B INT-GEMM tile's committed-matrix matmul→fold→digest in
**one** Layer-0 STARK, Pearl-faithfully (§4.C.2 shipped on
**b1** — sound, no weaker than any existing PROGRAM_COL pin).

### Phase A-CR — first-class params-pure `canonical_program` (CRIT-1 reconstruction hardening; subsumes §4.C.2-b2)

> Full design + decisions D-CR1..4 + staged plan CR.0..7:
> **`CANONICAL_PROGRAM_DESIGN.md`**. Sequenced **after Phase A,
> before Phase B proper.** Promotes the verifier's
> canonical-program reconstruction from *"`extract_program` of a
> reference honest trace"* (today: the bridge passes the
> prover-extracted program to verify; "verifier rebuilds from
> `ZkParams`" is only the `crit1_*` design intent) to a
> **witness-free, params-pure, isolation-auditable
> `canonical_program(params, block_public)`** for all 5
> PROGRAM_COLS. §4.C.2's store-row `NOISE_PACKED_PREP =
> polyval(noise_ref(s_a),129)` becomes one part of it (b2,
> subsuming Phase-A's b1 noise pin — b1's in-circuit/store
> wiring is reused; only *how the verifier obtains canonical
> noise* changes). Removes a latent CRIT-1 fragility
> system-wide and builds the witness-free params→program/VK
> reconstruction **Phase C (P-C) / M-C1 require regardless**.

| # | Item | Depends | Exit gate |
|---|---|---|---|
| **CR.0** | The single params-pure **row schedule** (`RowClass` per row from params + `block_public`), shared by the bridge trace generator *and* `canonical_program` (one source of truth — eliminates the prover/verifier shared-bug class). | Phase A (A1/A3.2a schedules) | `schedule(params)` reproduces the bridge's actual row layout (incl. `tile_chunk_range`, `noised_store_layout`); no verify-path change. |
| **CR.1–CR.5** | `canonical_program` per row class (CONTROL_PREP/CV/AB_ID/ROW_IDX, then store `NOISE_PACKED_PREP` via `noise_ref`). | CR.0; §4.C.2-b1 (Phase A) | Per class, `canonical_program == extract_program(honest_trace)` **bit-for-bit** (Llama-8B + TEST_SMALL + rectangular) **and** vs hand-computed expected PROGRAM_COLS for small geometries; full `ai-pow-zk --lib`. |
| **CR.6** | Flip the verify path: VK = commitment to `canonical_program(params, block_public)`; `prove_and_verify` verifies against it (not the prover-passed program). | CR.1–5 | Route-A + full `crit1_*` + new adversarial (any PROGRAM_COL — esp. store `NOISE_PACKED_PREP` — ≠ params-pure canonical ⇒ reject); `ai-pow --features zk` all-green; debug-assertions-ON. |
| **CR.7** | Docs/audit flip (`ZKP_SECURITY_REPORT`/`GAP_AUDIT` CRIT-1 → first-class reconstruction; §4.C.2-b2 marked subsumed). | CR.6 | Security docs reflect the upgraded CRIT-1 model. |

After Phase A-CR: every PROGRAM_COL (incl. §4.C.2 store noise)
is verifier-fixed by a witness-free params-pure function; the
CRIT-1 latent "extract-of-reference" fragility is removed
system-wide; the params→program/VK primitive Phase C needs
exists. **The most soundness-sensitive milestone in the
codebase — staged/KAT-first/not-rushed per `~/.claude/CLAUDE.md`
R1.**

### Phase B — byte-equivalence & correctness vs Pearl for this model

> **Authoritative plan: `PHASE_B_DESIGN.md`. STATUS 2026-05-18 —
> Phase B byte-equivalence + correctness gate COMPLETE on the
> real shipped model.** B0 (precise D5/D6 claim) ✅, B3
> (INT-only machine guard; fixed the `LLAMA_3_1_8B_DOWN` FP8
> mis-doc) ✅, B2-contract (`ai-pow::quant`, bit-lossless, R1) ✅,
> B1.0 (real-μ scale invariants) ✅, **B1-audit** (vendored ref ≡
> current real `pearl/zk-pow`, line-for-line) ✅, **B1.1** (user
> supplied the 16 GB weights ⇒ ai-pow's full audited pipeline
> byte-processes the **real `gate_proj` INT7 weights** at the
> real μ; reader anchored to an independent Python oracle) ✅.
> `pearl_compat_fixtures` 11/0/0, `pearl_model_compat` 8/0/0,
> `quant` 4/4, `b3_*` 3/3. The **only** untested path is a *live
> vLLM forward-pass activation from a real prompt* — a Phase-D
> end-to-end-usefulness check, **not** a byte-equivalence gap
> (B2.2: the contract is lossless for any int7 activation).
> Commits `f05862d`..`30bb92f`. Precise (D5/D6-normalized) claim
> + audit: `PEARL_COMPARISON.md` / `B1_PEARL_FAITHFULNESS_AUDIT.md`.

| # | Item | Depends | Exit gate |
|---|---|---|---|
| **B1** | Pearl **reference vectors**: obtain/derive golden `(κ, s_a, s_b, E/F, one tile digest)` from Pearl's miner for this model's mining config `μ`; assert `ai-pow` (`prng.rs`/`commit.rs`/`matmul`/fold) is **bit-identical**. | — (parallel to A) | Byte-identical on the Pearl reference vectors (today only self-consistency vs ai-pow's own plain path is tested). |
| **B2** | **Quant-extraction contract**: specify exactly how the vLLM plugin maps the model's INT7/INT8 quantized GEMM operands (per-channel/per-token scales applied *outside* the mined integer matmul) to the Pearl type-0 `[−64,64]` int8 `(A,B,μ)`; validate the extracted integer operand + scales reproduce the model's true GEMM output (usefulness preserved) and equals what Pearl mines. | B1 | A fixture from the real model: extracted `(A,B,μ)` → ai-pow digest == Pearl's, and dequant(int matmul) == the model's reference GEMM within the quant tolerance. |
| **B3** | **INT-only production scoping** documented & enforced (mine group_1 INT GEMMs; FP8 group_0 deferred to Pearl's unshipped FP protocol). | B2 | The miner config rejects/skips FP8 layers; the limitation is in the production docs. |

### Phase C — succinct certificate & audit (consensus-facing SNARK)

| # | Item | Depends | Exit gate |
|---|---|---|---|
| **C1** | **M-S3** — vendor `Plonky3-recursion` + align the Plonky3 rev in the vendored tree. | — | Audit-stable owned recursion substrate (P0/F2/F7 resolved). |
| **C2** | **M-S4** — `tip5-circuit-air` from `nockchain-math::tip5` + Tip5 challenger/MMCS arms; native≡in-circuit cross-test. | C1 | The recursion verifier can verify our Tip5 Layer-0 proofs; the 120-bit FRI sweep preserved. |
| **C3** | **P-C / M-S5** — vertical-recursion ≤65 KB certificate (Pearl §4.7/§5.1 faithful — compress *one* Layer-0 proof; **no** G3 `Γ`/aggregation). | Phase A, **Phase A-CR** (reuses the witness-free `canonical_program`/VK reconstruction), C2 | A real Llama-8B INT-tile proof compresses to a ≤65 KB cert that verifies; `N=1` ≡ the single proof (accept/reject parity). |
| **C4** | **M-S6** — independent crypto audit: 7-round Tip5 (now in-circuit) + the vendored/extended recursion stack. | C3 | Removes the "experimental/unaudited" gate. |

### Phase D — integration (deliberately vague; external to ai-pow-zk)

| # | Item | Notes |
|---|---|---|
| **D1** | **vLLM miner-plugin extraction.** Intercept the served model's GEMMs, produce `(A,B,μ)` per the B2 contract, run the ai-pow mineable unit, and on a winning tile invoke the ai-pow-zk prover. *Lives in Pearl's `miner/vllm-miner` (or a Nockchain analog; `ai-pow-vi` is the related verifiable-inference crate). Detailed plan deferred.* | external |
| **D2** | **Consensus / block-certificate** (Track-C **M-C1**). Make the (cert) proof + PIs the consensus block artifact (`pouw_meta`); Nockchain nodes verify it; `MatmulParams::validate_prod_envelope` becomes the consensus **admission rule**. Today the SNARK is an out-of-band gate. *Detailed plan deferred.* | external |

### Phase E — cross-cutting

- **Track-B economics (M-P1):** PROD profiling — confirm the
  ~450 µs/row ⇒ ~16 min/INT-tile-proof at this model's
  `(t,r,k)`, amortized (SNARK only on a win); parallel proving;
  memory at the one-STARK ceiling.
- **G3 — deferred, not on this path.** This model is inside
  Pearl's `k≤2¹⁶` §4.8 envelope, so carry-vector segmentation is
  *unnecessary*; revive `M_S2_G3AB_DESIGN.md` only if a future
  workload exceeds the envelope.

---

## 3. Critical path & minimal production cut

```
A1 → A2 → A3 (b1)                       (SNARK sound & one-STARK at model scale)
        ↘
          CR.0 → CR.1‥5 → CR.6 → CR.7   (params-pure canonical_program; subsumes §4.C.2-b2)
                              ↘
B1 → B2 (∥ A) → B3 ───────────┤         (byte-equiv + INT-only scoping)
                              ↘
                                C1 → C2 → C3 → C4   (succinct cert + audit;
                                                     C3 reuses CR canonical_program)
                                             ↘
                                               D1, D2 (vLLM + consensus — external)
```

- **Minimal "this model's INT GEMMs mine soundly with a
  Nockchain SNARK"** = **Phase A** (incl. §4.C.2 on b1). A2 is
  the hard unblocker; without it the SNARK cannot prove the
  model at all.
- **Phase A-CR** sits **after Phase A, before Phase B proper**:
  it upgrades CRIT-1's verifier reconstruction to first-class
  params-pure (subsuming §4.C.2-b2) — a soundness-foundation
  hardening that Phase C's P-C/M-C1 *require* regardless (no
  prover trace there).
- **Consensus-grade production** = + **Phase C** (succinct,
  audited cert — C3 builds on Phase A-CR's `canonical_program`)
  + **Phase D** (vLLM extraction + chain consumption).
- Phase B (byte-equiv) is independent of A/A-CR and may run in
  parallel; C1/C2 (vendor recursion / Tip5-AIR) likewise; C3
  depends on Phase A-CR.

**Inflections:** A2 = SNARK can mine the real model (one-STARK).
A3 = full §4.C zero-gap soundness (b1). **A-CR/CR.6 = CRIT-1
verifier reconstruction is first-class params-pure
(witness-free), latent fragility removed system-wide.** C3 =
succinct consensus artifact. C4 = audit gate cleared.

---

## 4. Risks / watch-items

- **A2 is invasive at the bridge** (C3/HASH_A path) but P-B.2.2
  proved the C3 binding is hash-structure-agnostic ⇒ no AIR
  change; main risk is the strip-byte/sibling plumbing + the
  budget update. Stage with the existing Route-A +
  debug-assertions-ON discipline.
- **B1/B2 depend on Pearl-side artifacts** (reference vectors,
  the exact quant-extraction the vLLM plugin performs) — secure
  these early; they gate "byte-equivalent" claims.
- **FP8 layers** are permanently out until Pearl ships its FP
  PoUW; ensure product framing scopes "mine the INT layers."
- Phase D is external; its timeline is not controlled here —
  keep the ai-pow-zk side (A–C) decoupled and
  integration-ready.

---

## 5. Cross-references

- Track-A milestone table & inflections: `HIGH2_2_DESIGN.md` §7.
- Why no segmentation (γ): `M_S2_PEARL_EVALUATION.md`.
- P-B.2.x design + decisions D1–D4: `P_B2_STRIP_OPENING_DESIGN.md`.
- §4.C.2 (Phase A, b1): `SEC_4C2_NOISE_BINDING_DESIGN.md`.
- **Phase A-CR (params-pure `canonical_program`; subsumes
  §4.C.2-b2): `CANONICAL_PROGRAM_DESIGN.md`.**
- G3 (deferred): `M_S2_G3AB_DESIGN.md`.
- Real model facts: `pearl_real_production_model` memory;
  `~/Dev/Llama-3.1-8B-Instruct-pearl/config.json`.
- §4.C soundness thread: `ai_pow_zk_crypto_gaps` memory;
  `ZKP_SECURITY_REPORT.md`, `GAP_AUDIT.md`.
