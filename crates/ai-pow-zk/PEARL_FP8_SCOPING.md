# FP8 scoping — how Pearl handles it, and what it means for Nockchain

> **Status:** SCOPING (2026-05-18). Authoritative analysis of the
> production models' **group_0 FP8** layers: what Pearl's
> protocol *and shipped code* actually do with them, and the
> exact, defensible scope boundary for the Nockchain ai-pow /
> ai-pow-zk side. Sources: Pearl whitepaper (`Pearl_Whitepaper.pdf`
> §1.1, §3.1, §4.1, §4.2, §4.8), Pearl reference code
> (`pearl/zk-pow`, `pearl/miner/vllm-miner` @ pearl ref
> `3be33a59`), the shipped model configs.

## 0. TL;DR

**FP8 is out of the proof-of-useful-work scope by Pearl's own
protocol design — not a Nockchain limitation.**

- Pearl's PoUW mines **exactly one thing**: a noised, tiled,
  **8-bit *integer*** matmul (`matmul-accumulate type = 0`,
  `[−64,64]` operands, `int32` accumulate). This is the *only*
  accumulate type the protocol defines as valid (§4.1).
- The shipped Pearl prover/verifier (`pearl/zk-pow`) contains
  **zero floating-point / FP8 code** — it is INT-only.
- Pearl's vLLM mining plugin **does not mine FP8 layers**: its
  scheme dispatch claims only the INT7 (mined) and INT8
  (non-mined) layers; **FP8 (group_0) falls through to vLLM's
  stock compressed-tensors path and runs as ordinary,
  *un-mined* inference** — exactly as it would with no Pearl
  plugin at all.
- An FP PoUW is an **explicitly planned, unshipped, not-yet-
  security-analysed future upgrade** (§1.1) that will *not*
  preserve exact matmul.

⇒ For the production models, the Nockchain SNARK proves the
**group_1 INT7 GEMMs and only those**. group_0 FP8 layers are
served-model inference that is never mined, never committed,
never proven — correctly. `ai-pow`'s B3 guard already enforces
this; the vLLM-CPU fork's `PearlFp8CpuScheme` is a *forward
enabler for un-mined layers*, fully consistent with how Pearl
itself treats FP8.

## 1. What the production models contain

`quant_method:"pearl"` `config.json` `config_groups` (verified on
all three shipped models — Llama-3.1-8B, Gemma-4-31B,
Llama-3.3-70B):

| group | format | bits / strategy | targets | Pearl status |
|---|---|---|---|---|
| **group_1** | `int-quantized` | **INT7**, weights per-**channel**, acts per-**token**, symmetric, dynamic-act | `o_proj`, `gate_proj`, `up_proj` (+ late-layer `qkv`) | **MINED** (PoUW) |
| **group_0** | `float-quantized` | **FP8**, weights block `[128,128]`, acts dynamic | `down_proj` (+ early-layer `qkv`) | **NOT mined** (plain inference) |

## 2. The whitepaper basis (verbatim anchors)

**§4.1 Mining Configuration — "Matmul-accumulate type":**
> *"An identifier of the exact matmul algorithm being done.
> **Initially, must be 0 denoting input matrices have [−64,64]
> entries and matmul-accumulate being done in a `int32`
> datatype.**"*

There is exactly one defined accumulate type, and it is INT.
An FP type is not assigned, not valid, not minable.

**§4.2 MatMul Framework:**
> *"For protocols over 8-bit integers, we quantize both A, B to
> integers in `[−64,64]` and the noise matrices E, F are to the
> range `[−63,63]`."*

The mined operation is an **integer** matmul; the entire noised
peel-off identity `A·B = A'B' − (A·F_L)F_R − E_L(E_R·B')` and the
`int32` accumulator + `int32`-XOR → `M`-state →
`BLAKE3(M) ≤ 2^{256−b}·r·t_m·t_n` machinery (§4.5) is integer.

**§1.1 Planned Upgrades — the FP statement:**
> *"The core of the Pearl network is a new implementation of
> **exact (INT) MatMul** … many modern AI workloads are not done
> natively in INT, but rather in low-precision floating-point
> formats (e.g., BF16 …, FP8 and FP4 for inference). … Extending
> the current PoUW scheme to floating-point datatypes is
> technically challenging.² Thus, in parallel …, we have been
> developing a **future PoUW protocol** … This future version
> will **not preserve exact matrix multiplication**. Instead, it
> computes a high-accuracy approximate MatMul … We will only
> propose upgrading the network after we complete extensive
> testing of both its accuracy and its security."*
> Footnote 2: *"Floating-point formats have a non-uniform
> dynamic range. … even small perturbations to (A,B) can produce
> disproportionately large changes in intermediate values,
> harming numerical stability …"*

FP PoUW: **future, unshipped, approximate (not exact), and
explicitly gated on a not-yet-done security/accuracy analysis.**

**§4.8 Supported PoW Parameters** (all INT-matmul shape caps):
`m,n ≤ 2²⁴`; `16r ≤ k ≤ 4r²`; `k ≤ 2¹⁶`; `64 | k`;
`r ∈ {2⁵…2¹⁰}`; `h·w ≥ 32`; `k(h+w) ≤ 2²²`. (No FP parameters
exist.)

## 3. What Pearl's *code* does with FP8 (not just the paper)

### 3.1 `pearl/zk-pow` (the real prover/verifier) — INT-only

A full scan of `pearl/zk-pow/src` for `fp8/float8/e4m3/e5m2/
type-1/floating-point/PoUW-float` returns **nothing**. The tile
loop is `jackpot_tile[u][v] += a_noised[a_idx][l] *
b_noised_t[b_idx][l]` over `i8→i32` (matches §4.2/§4.5). There is
no FP path to prove; the SNARK provably *cannot* attest an FP
matmul because no such circuit/accumulate type exists.

### 3.2 `pearl/miner/vllm-miner` — FP8 is delegated, never mined

`PearlConfig._get_scheme_from_parts` (extends vLLM
`CompressedTensorsConfig`) classifies a layer as:

- **mining** iff `_is_mining_layer`: `num_bits == 7` (weight &
  act), weight strategy ∈ {tensor, channel}, act strategy =
  token, symmetric, static-weight/dynamic-act → `PearlScheme
  (mining_enabled=True)` (the noisy/INT7 mined GEMM);
- **non-mining** iff `_is_non_mining_layer`: same but
  `num_bits == 8` (still **int** + tensor/channel strategy) →
  `PearlScheme(mining_enabled=False)` (plain int8 GEMM);
- **otherwise** → `super()._get_scheme_from_parts(...)` =
  vLLM's stock compressed-tensors scheme.

group_0 is FP8 with weight strategy **`block`** ⇒ it fails *both*
predicates (block ∉ {tensor, channel}) ⇒ it takes the
`super()` branch = **vLLM's ordinary FP8 inference kernel**. The
Pearl plugin never wraps it, never noises it, never commits it,
never emits a PoUW for it. FP8 layers are *exactly* the
served model's normal quantized inference; Pearl only *adds*
mining to the INT7 GEMMs (the §1.1 "drop-in plugin … augments
these calls with negligible additional overhead", applied only
to the exact-INT layers).

## 4. Implications for the Nockchain side (ai-pow / ai-pow-zk)

- **The proof scope is exactly the group_1 INT7 type-0 GEMMs.**
  This is Pearl-protocol-defined, not a Nockchain shortcut. The
  Nockchain SNARK (Phase A/A-CR, complete) proves the noised
  int8 tiled matmul + M-state + C1–C4; that is precisely and
  only what Pearl mines.
- **B3 is the correct, faithful enforcement.** `ai_pow::params`
  `LlamaFfnLayer`/`QuantGroup` + `mineable_matmul_params`
  rejecting `QuantGroup::Fp8Deferred` mirrors Pearl: FP8 layers
  are not consensus-mineable. (This also fixed the
  `LLAMA_3_1_8B_DOWN` mis-doc — `down_proj` is FP8, not a
  mineable preset.)
- **The vLLM-CPU fork's `PearlFp8CpuScheme` is in-scope and
  consistent.** It only lets the *un-mined* FP8 layers complete
  a CPU forward (dequant + `F.linear`) so a real INT7 mined
  activation can be captured downstream — i.e. it reproduces, on
  CPU, Pearl's own "FP8 = ordinary inference, not PoUW"
  behaviour. It is explicitly **not** a mined/proven path and
  carries no soundness weight (V5 / `PEARL_VLLM_CPU_FORK_DESIGN`).
  Its dequant (`pearl_gemm_cpu.fp8_block_dequant`, vLLM-free) is
  now **test-proven canonical**: `tests/test_pearl_fp8_cpu.py`
  (8/8) validates it bit-for-bit against the **verbatim vLLM
  block-FP8 formula** (`fp8_utils.py`), an **independent
  nested-loop spec**, torch-core's `float8_e4m3fn` codec, and the
  **real shipped Llama-3.1-8B `down_proj` FP8 weights** (anchored
  to a Python ground truth). So the FP8-layer numerics are
  faithful to what Pearl's GPU stack computes; it is *not* a
  source of soundness/byte-equivalence error (and never could
  be — FP8 is outside the proven scope).
- **Byte-equivalence is unaffected.** Phase B's "mineable unit
  byte-identical to Pearl" claim is about the INT7 mined GEMMs;
  FP8 layers are not part of the mineable unit on either side,
  so there is nothing to byte-match (and nothing Pearl would
  prove either).

## 5. If/when Pearl ships an FP PoUW (future, out of scope)

Per §1.1 the future scheme will: (a) use a **new
matmul-accumulate type ≠ 0**; (b) compute an **approximate**
(not exact) MatMul leveraging native quantization noise; (c)
still force commitment to the pre-quantization weight/activation
matrices; (d) ship **only after Pearl's own accuracy + security
analysis**. The Nockchain consequences would be a *new
milestone*, not an extension of the current work:

- a distinct AIR / accumulate-type-≠0 circuit (the current
  `MatmulParams`/CRIT-1/§4.C machinery is hard-wired to the
  exact-int relation and the `[−64,64]` domain);
- a fresh soundness argument (FP non-uniform range + approximate
  matmul changes the noise/peel-off + the M-state extraction —
  §1.1 footnote 2 is exactly why this is hard);
- gated on Pearl publishing the FP spec + its security analysis.

**Recommendation:** treat FP8 as *permanently out of scope*
until Pearl ships and audits its FP PoUW. Track it as a
not-started future milestone (no design needed now — Pearl owns
the protocol). Production framing must say: *"Nockchain mines
this model's INT (group_1) GEMMs; the FP8 (group_0) layers are
the served model's ordinary inference and are out of PoUW scope,
matching Pearl."*

## 6. Cross-references

- `Pearl_Whitepaper.pdf` §1.1/§3.1/§4.1/§4.2/§4.8;
  `pearl/zk-pow/src`, `pearl/miner/vllm-miner/src/vllm_miner/
  vllm_config.py` @ pearl ref `3be33a59`.
- `PRODUCTION_ROADMAP.md` §0 (INT-only scoping), Phase B B3.
- `crates/ai-pow/src/params.rs` (`QuantGroup`/`LlamaFfnLayer`
  B3 guard); `PEARL_COMPARISON.md`;
  `PHASE_B_DESIGN.md` (Phase B complete);
  `PEARL_VLLM_CPU_FORK_DESIGN.md` (`PearlFp8CpuScheme` =
  un-mined forward enabler).
- `pearl_real_production_model` / `phase_b_pearl_byte_equiv` /
  `pearl_vllm_cpu_fork` memory.
