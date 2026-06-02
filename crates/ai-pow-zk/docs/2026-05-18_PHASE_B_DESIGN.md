> _Created **2026-05-18** · last updated **2026-05-18** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# Phase B — byte-equivalence & correctness vs Pearl for the production model

> **Status:** DESIGN (2026-05-18). Roadmap milestone
> `2026-05-17_PRODUCTION_ROADMAP.md` §2 "Phase B"; sequenced **independent
> of / parallel to Phase A & Phase A-CR** (both now COMPLETE).
> Phase B is the **correctness gate**: it does *not* touch SNARK
> soundness (that is Phase A/A-CR) — it certifies that, for the
> shipped production model, `ai-pow`'s *mineable unit* is
> **bit-identical to Pearl's actual miner**, and that the vLLM
> plugin's quantized-GEMM → Pearl-`(A,B,μ)` extraction is
> correct and useful.
> **Authoritative cross-refs:** `crates/ai-pow/2026-05-13_PEARL_COMPARISON.md`
> (the D1–D6 byte-divergence inventory + the S0–S9 fixture
> harness — the substrate this phase extends),
> `2026-05-17_PRODUCTION_ROADMAP.md` §0/§2/§4, the `pearl_real_production_model`
> memory, `~/Dev/Llama-3.1-8B-Instruct-pearl/config.json`.
> Governed by `~/.claude/CLAUDE.md` R1 for the one
> soundness-adjacent edge (B2's "equals what Pearl *mines*"
> touches the mined integer operand — KAT-first conformance,
> never rushed).

---

## 0. The production model (verified facts)

Target: shipped **`pearl-ai/Llama-3.1-8B-Instruct-pearl`** served
via the Pearl **vLLM mining plugin**. `config.json`
`quantization_config` (`quant_method:"pearl"`) has two groups
(verified from the local config 2026-05-18):

| Group | format | weights | activations | targets | mined? |
|---|---|---|---|---|---|
| **group_1** | `int-quantized` | **INT7**, per-**channel**, symmetric, static (minmax) | INT7, per-**token**, symmetric, dynamic | `o_proj`, `gate_proj`, `up_proj` (all layers) + `q/k/v_proj` layers **16–31** | ✅ **YES** |
| **group_0** | `float-quantized` | FP8, block `[128,128]`, symmetric | FP8-e?, group 128, dynamic | `down_proj` + `q/k/v_proj` layers **0–15** | ❌ **NO** |

Pearl whitepaper §4.1 fixes matmul-accumulate **type-0 = INT
only** (`[−64,64]`, int32 accumulate); §1.1 defers an FP PoUW to
an unshipped upgrade. ⇒ **production mines group_1's INT7 GEMMs
only**; group_0 (FP8) is a *documented production limitation*,
not a defect (B3), and is not on any critical path.

The standing architecture (do **not** relitigate): **only the
mineable unit is byte-equivalent to Pearl** — the plain
`TileState` / keyed-BLAKE3 fold over `(A,B)` + the Pearl noise
`(E,F)` derived from `(s_a,s_b)`. The SNARK is Nockchain's own
Plonky3 stack (Phase A/A-CR), *not* Pearl's Plonky2 proof.

---

## 1. The precise gap (what is *actually* left)

`2026-05-13_PEARL_COMPARISON.md`'s inventory states D1–D4 **CLOSED** and the
`tests/pearl_compat_fixtures.rs` S0–S9 suite is **green with no
`#[ignore]`** (verified 2026-05-18: `11 passed; 0 failed; 0
ignored`). But that green is **against `tests/fixtures/pearl.rs`
— bytes captured by `tests/gen_fixtures.rs` from a *vendored copy
of Pearl's reference functions***, for *hand-picked* inputs, at
*generic* shapes. That is self-consistency against our
understanding of Pearl, **not** against Pearl's real miner for
*this* model's mining config `μ`. Phase B closes exactly that
delta, in three items:

- **B1 — golden vectors from Pearl's real miner for `μ`.** Lift
  the S0–S9 oracle from "vendored reference functions" to
  "Pearl's actual miner output for `pearl-ai/Llama-3.1-8B`'s
  shipped mining config", and assert `ai-pow` (`prng.rs`,
  `commit.rs`, `fiat_shamir.rs`, `matmul.rs`, fold,
  `tile_hash.rs`) is bit-identical on **that model's real
  `(κ, s_a, s_b, E/F, A, B, one tile digest)`**.
- **B2 — the quant-extraction contract.** Specify + validate how
  the vLLM plugin maps group_1's INT7 quantized GEMM operands
  (per-channel weight scale + per-token activation scale applied
  *outside* the mined integer accumulate) to Pearl type-0
  `int8 [−64,64]` `(A,B,μ)`; prove the extracted integers (a)
  digest-equal what Pearl mines and (b) dequantize back to the
  model's true GEMM within the model's own quant tolerance
  (usefulness preserved).
- **B3 — INT-only scoping enforced.** The miner config rejects /
  skips group_0 (FP8) layers; the limitation is in the
  production docs and is *machine-checked*.

Deliberate Nockchain extensions **D5/D6** (the `pow_key =
derive_key("pow-key", s_A‖nonce)` key + the Bitcoin-style nonce
search loop) are **NOT** Phase-B-closed — they are by-design
divergences. Phase B's job re D5/D6 is only to **state the
byte-equivalence claim exactly** (§4).

---

## 2. B1 — golden vectors from Pearl's real miner

### Mechanism

The S0–S9 boundaries already pin the algorithm; B1 changes the
*oracle source*, not the assertions. Three sub-stages:

- **B1.0 (de-risk, no Pearl needed).** Re-run `gen_fixtures.rs`
  at **this model's real shapes** (`MatmulParams` from the
  Llama-3.1-8B `LLAMA_3_1_8B_*` preset — already P-A-encoded:
  `pearl_real_production_model` memory) instead of the generic
  hand-picked inputs. Assert S0–S9 still green at production
  geometry (catches any shape-dependent latent the generic
  fixtures miss — e.g. the 57 344-chunk weight scale that
  P-B.2.0 already exercised for the commitment). *No Pearl-side
  artifact required.*
- **B1.1 (the real oracle).** Obtain golden vectors from
  Pearl's **actual** miner for `pearl-ai/Llama-3.1-8B`'s shipped
  `mining_config`: `(block_header/μ, κ, s_a, s_b, a sampled
  E-row + F-row, A, B, the per-stripe X, jackpot[16], the
  jackpot digest, H_A/H_B, the difficulty target)`. Bake them
  into a new `tests/fixtures/pearl_model.rs` section (mirrors
  the S-layout; provenance + Pearl commit recorded in the
  header).
- **B1.2 (enforce).** A `pearl_model_compat` test asserts every
  `ai-pow` primitive bit-matches the B1.1 vectors. Green ⇒
  "`ai-pow`'s mineable unit is byte-identical to Pearl's real
  miner for the production model".

### Exit gate

`pearl_model_compat` green on real Pearl-miner vectors for the
shipped model `μ` (today: only self-consistency vs vendored
reference at generic shapes). B1.0 green is the *de-risk*
milestone landable immediately; B1.1/B1.2 are **blocked on the
Pearl-side artifact** (see §6 Risk-1).

### Test architecture

Extend, do not fork, the proven harness: `gen_fixtures.rs`
(generator) → `fixtures/pearl_model.rs` (captured golden) →
`pearl_model_compat.rs` (assertions), parallel to the existing
`gen_fixtures`/`pearl.rs`/`pearl_compat_fixtures` triple. Keep
the vendored-reference S0–S9 too (cheap regression with no Pearl
dependency at test time).

---

## 3. B2 — the quant-extraction contract

### The contract (concrete, from the verified config)

For a mined group_1 layer the served computation is

```
Y_fp ≈ dequant( Wq @ Xqᵀ ) ;  Wq ∈ int7^[out,in] (per-channel s_w[out]),
                               Xq ∈ int7^[tok,in]  (per-token   s_x[tok])
Y_fp[o,t] = s_w[o] · s_x[t] · Σ_in Wq[o,in]·Xq[t,in]
```

The **mined integer accumulate** is exactly `Σ_in Wq·Xq`
(int32). The contract `Q` maps `(Wq, Xq)` of one tiled GEMM to
Pearl type-0 `(A, B, μ)`:

- 7-bit symmetric ⇒ values in `[−64, 63] ⊂ [−64, 64]` — already
  inside Pearl's int8 type-0 domain (no requant, no clipping in
  the common case; the contract pins the exact boundary
  handling for the `−64`/`+64` edges).
- `A := Xq`-derived row-strips, `B := Wq`-derived col-strips in
  the `compute_tile_from_slices` layout (the existing ai-pow
  matmul/fold operates on these unchanged).
- `μ` carries the per-channel `s_w` and per-token `s_x` (the
  *dequant* scalars) **outside** the mined integer matmul — they
  are NOT part of the proven integer relation; they are recorded
  for usefulness reconstruction only.

### What B2 must prove

1. **Mining parity.** `ai-pow` digest of the extracted `(A,B,μ)`
   == Pearl's digest for the same extraction (this is the
   soundness-adjacent edge — the *mined integer operand* must be
   exactly Pearl's; KAT-first, R1).
2. **Usefulness preserved.** `dequant(intmatmul(A,B)) ==`
   the model's reference `Y_fp` for that GEMM **within the
   model's own quantization tolerance** (i.e. extraction adds
   *zero* additional error: the integers mined are bit-for-bit
   the integers vLLM already computes; the only error is the
   model's pre-existing INT7 quant error, which is by
   construction acceptable to the shipped model).

### Exit gate

A fixture captured from the **real model** on a real prompt: one
group_1 GEMM → extracted `(A,B,μ)` → (a) `ai-pow` digest ==
Pearl's, (b) `dequant(intmatmul) == vLLM's reference `Y_fp` bit-
or tol-exact. Depends on B1 (needs the validated `ai-pow`
primitive + the Pearl digest oracle).

### Where the code lives

B2 is **greenfield in `ai-pow`** (verified: no quant/dequant/
scale code in `crates/ai-pow/src/`). Candidate home: a new
`ai-pow::quant` module (the pure `Q` contract + its inverse for
the usefulness check) + a conformance fixture; the *live*
extraction is Phase D's vLLM plugin (external — `ai-pow-vi` /
Pearl `miner/vllm-miner`). B2 ships the **contract + offline
conformance proof**, not the plugin.

---

## 4. The deliberate-divergence boundary (D5/D6 — document, don't close)

Phase B must make the byte-equivalence claim **exact**, because
"byte-equivalent to Pearl" is false unqualified. Precisely:

> Given **identical** `(κ, s_a, s_b, A, B)` and the **same tile
> index**, `ai-pow` and Pearl produce the **bit-identical
> mineable unit** — the noise `(E,F)`, the per-stripe `X`, the
> `TileState` fold, and the keyed-BLAKE3 jackpot **message**
> (16×u32 LE) — and the **same** `H_A/H_B` chunk-Merkle roots
> and difficulty target. They differ **by design** in: (D5) the
> hash *key* — Pearl keys the jackpot hash with `a_noise_seed`;
> Nockchain keys with `pow_key = derive_key("pow-key",
> s_A‖nonce)`. (D6) was superseded by the 2026-05-31 grinding
> audit: Nockchain still carries a Bitcoin-style nonce, but that
> nonce is part of Pearl's attempt state before `κ`, commitments,
> noise seeds, and matmul-derived tile states are computed.
> There is no matmul/noise amortization across nonces; treating a
> cached matmul result plus many nonce hashes as many attempts is
> the grinding vulnerability.

Deliverable: a `2026-05-13_PEARL_COMPARISON.md` "Byte-equivalence claim
(precise)" section stating exactly the above, and a test
`mineable_unit_byte_equiv_modulo_key` that asserts the jackpot
**message** + `(E,F,X,M,H_A,H_B,target)` are byte-identical when
the key/nonce are normalised out (this already exists in spirit
as `m52_unit_of_work_byte_equiv.rs` — B promotes it to the
real-model oracle from B1).

---

## 5. Decisions to surface (recommendations)

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **DB-1** | B1.1 golden-vector **source** | (a) run Pearl's real miner ourselves on the shipped model; (b) request golden vectors from the Pearl team; (c) vendor Pearl's miner into `gen_fixtures` at the real `μ` (still "our understanding", not the real miner) | **(a) if a Pearl miner build is runnable here; else (b)**. (c) is only a stronger B1.0, not a true B1.1 — it does not discharge Risk-1. Secure this **early** (longest-lead item). |
| **DB-2** | Quant tolerance for B2.2 "usefulness" | (a) **bit-exact** `intmatmul == vLLM's integer GEMM` (extraction adds zero error — the strong, checkable claim); (b) fp tolerance on the dequantized output | **(a)** — the honest claim is "we mine *exactly the integers vLLM already computed*". The model's INT7 error is the model's, not ours; asserting (a) makes B2 a clean bit-equality, not a fuzzy numeric gate. (b) only as a secondary sanity. |
| **DB-3** | B3 enforcement point | (a) `MatmulParams`/miner-config validation rejects FP8-targeted layers; (b) the vLLM plugin filters (external) | **(a) as the machine-checked guard** (consensus-adjacent: mirrors `validate_prod_envelope`), **+(b)** as the operational filter. (a) is in-repo and testable now. |
| **DB-4** | Doc home | (a) extend `2026-05-13_PEARL_COMPARISON.md`; (b) this new doc is authoritative | **(a) for the divergence inventory + the precise claim** (it already owns D1–D6); this doc owns the Phase-B *plan*; `2026-05-17_PRODUCTION_ROADMAP.md` §2 stays the index. |

---

## 6. Risks / watch-items

- **Risk-1 (hard, external): B1.1/B2 depend on Pearl-side
  artifacts** — golden vectors from Pearl's real miner for the
  shipped `μ`, and the exact quant-extraction the vLLM plugin
  performs. These gate every *real* "byte-identical to Pearl"
  claim and their timeline is **not controlled here**. Mitigate:
  land B1.0 + B3 + the §4 precise-claim doc + the B2 *contract*
  immediately (all Pearl-independent); keep B1.1/B1.2/B2-fixture
  as a clean, pre-wired drop-in (the harness extension is built
  and tested against a *synthetic* stand-in oracle so flipping
  to the real vectors is one fixture file).
- **Risk-2: shape-dependent latents.** The vendored S0–S9 use
  hand-picked generic inputs; the real model is 4096-wide,
  57 344 weight-chunks. P-B.2.0 already KAT'd the commitment at
  that scale, but B1.0 must re-pin *all* S0–S9 at the real
  preset (cheap, do first).
- **Risk-3: FP8 surface creep.** group_0 must stay out until
  Pearl ships FP PoUW; B3's machine guard prevents a silent
  attempt to mine a `down_proj`/early-qkv GEMM.
- **Risk-4: D5/D6 mis-stated.** An over-broad "byte-equivalent
  to Pearl" claim is *false*; §4's precise, key/nonce-normalised
  statement is mandatory before any external communication.

---

## 7. Staged landing plan (R1 where soundness-adjacent)

**STATUS 2026-05-18 — every Pearl-independent stage DONE +
validated + committed; the residual is the one external blocker.**

| Stage | Pearl-dep? | Status | Gate / commit |
|---|---|---|---|
| **B0** | no | ✅ DONE | `2026-05-13_PEARL_COMPARISON.md` precise (D5/D6-normalized) claim + honest oracle scope; roadmap §2 updated. `f05862d` |
| **B3** | no | ✅ DONE | `LlamaFfnLayer`/`QuantGroup` + `mineable_matmul_params` FP8 guard; **fixed the `LLAMA_3_1_8B_DOWN` mis-doc** (down_proj is FP8, not mineable); `b3_*` 3/3. `a420e94` |
| **B2-contract** | no | ✅ DONE | `ai-pow::quant` `Q`/inverse; bit-lossless conformance KAT (R1 KAT-first on the mined-integer edge) `b2_1..4` 4/4. `94eaafc` |
| **B1.0** | no | ✅ DONE | `pearl_model_compat` real-`μ` invariants (chunk-scale 57 344, r=64 noise structure, 64-stripe fold wrap, difficulty) 5/0. `4916a6b` |
| **B1-audit** | no | ✅ DONE | `2026-05-18_B1_PEARL_FAITHFULNESS_AUDIT.md`: vendored ref ≡ **current real `pearl/zk-pow`** (builds clean) line-for-line ⇒ **B1 protocol-equivalence CLOSED**. |
| **B1.1 (real weights)** | no¹ | ✅ DONE | User supplied the shipped 16 GB weights. `pearl_model_compat::b1_1{a,b,c}`: a safetensors reader anchored bit-for-bit to an independent Python oracle (R1 integrity); the real `gate_proj` int7 weights ∈ Pearl `[−64,64]` + B2.1 lossless on **real data**; `BlockContext::build` runs ai-pow's full audited pipeline on the **real weight tile** at the real μ (det., weight-sensitive, `H_B == matrix_commitment(real bytes)`). `30bb92f` |
| **B1.1 (all 3 models)** | no¹ | ✅ DONE | **Corroborated on ALL THREE published Pearl models**, spanning 2 architectures / 3 contraction dims / 1·2·15-shard layouts: Llama-3.1-8B (2-shard, k=4096; `30bb92f`), Gemma-4-31B (single-file 31 GB, gemma4, k=5376; `db3e193`), Llama-3.3-70B (15-shard 135 GB, k=8192, the largest; `9f6d97d`). Each: `b1_1_<m>_{a,b,c}` — reader anchored to its own independent Python oracle, real int7 ∈ Pearl `[−64,64]` + B2.1 lossless on real data, full audited pipeline at the real μ (det., weight-sensitive, `H_B == matrix_commitment`). `pearl_model_compat` **14/0/0**. |
| **B2-fixture / live-vLLM** | **yes** | ⚠ Phase-D (B2.2-covered) | The *only* untested path: a **live vLLM forward-pass activation from a real prompt** (needs the model loaded for inference + GPU/vLLM, not just static weights). This is **not a byte-equivalence gap** — B2.2 proved the quant contract is bit-lossless for *any* int7 activation; it is a Phase-D end-to-end-deployment *usefulness* verification, deferred with Phase D (external). |

¹ B1.1 needed the static weights (now present), not a GPU/vLLM
runtime; the live forward pass is the Phase-D row below.

Each stage: commit per validated stage; honest status + precise
residual (R1); the soundness-adjacent B2 mined-integer edge is
KAT-first and never rushed even though Phase B is a correctness
(not soundness) gate. `crates/plonky3-recursion/` untracked.

**Definition of done:** for `pearl-ai/Llama-3.1-8B-Instruct-pearl`'s
group_1 INT7 GEMMs, `ai-pow`'s mineable unit is bit-identical to
Pearl's real miner (B1), the vLLM quant→`(A,B,μ)` extraction is
specified and proven lossless+useful (B2), FP8 is machine-scoped
out (B3), and the byte-equivalence claim is stated exactly
(D5/D6, §4). Phase B does not gate SNARK soundness (Phase A/A-CR,
COMPLETE) — it gates the *correctness* of what that SNARK proves
for the production model.

## 8. Cross-references

- `crates/ai-pow/2026-05-13_PEARL_COMPARISON.md` — D1–D6 inventory, S0–S9
  harness, `gen_fixtures`/`fixtures/pearl.rs` provenance.
- `2026-05-17_PRODUCTION_ROADMAP.md` §0 (model), §2 Phase B, §4 (risks).
- `2026-05-17_CANONICAL_PROGRAM_DESIGN.md` — Phase A-CR (COMPLETE; the
  soundness foundation Phase B sits beside, not on).
- `pearl_real_production_model` memory; `~/Dev/Llama-3.1-8B-
  Instruct-pearl/config.json` (the verified quant config).
- `crates/ai-pow/tests/{gen_fixtures,pearl_compat_fixtures,
  m52_unit_of_work_byte_equiv}.rs` — the harness B extends.
