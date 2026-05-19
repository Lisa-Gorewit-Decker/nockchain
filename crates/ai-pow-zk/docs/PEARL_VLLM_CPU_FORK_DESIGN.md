> _Created **2026-05-18** · last updated **2026-05-18** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# Pearl vLLM-plugin CPU fork — design pass

> **Status:** DESIGN (2026-05-18). Vendored from
> `pearl/miner/vllm-miner` @ pearl ref `3be33a59` into
> `crates/ai-pow/pearl_vllm_cpu/` (`src/vllm_miner/*` +
> `pyproject.toml.orig`). User-directed, **Phase-D / integration
> — beyond Phase B** (which is COMPLETE, task #120, corroborated
> on all 3 published models). Governed by `~/.claude/CLAUDE.md`
> R1 (no fake completion; honest blockers; the CPU kernels are a
> *reimplementation*, validated against the audited spec).

## 0. Goal + honest scope (read first)

**Goal:** run the smallest published Pearl model
(`Llama-3.1-8B-Instruct-pearl`, 2-shard, k=4096) through a
**CPU-only fork** of the Pearl vLLM plugin on the M2 Max, get a
**real forward-pass activation**, and feed the extracted
`(A,B,μ)` into `ai_pow::quant` + `BlockContext` end-to-end.

**What this does NOT establish (must stay explicit):** B2.2
already proved the quant-extraction contract is **bit-lossless
for *any* int7 activation**, and B1.1 corroborated it on all
three real models' weights — Phase B's byte-equivalence +
correctness gate is *already complete*. A CPU fork's `pearl_gemm`
is **our reimplementation** (faithful-by-construction to the
B1-audited Pearl spec), **not** Pearl's CUDA kernel. So this run
adds **no byte-equivalence / soundness evidence**. Its only value
is a **Phase-D integration smoke**: proving the
real-prompt → real-activation → extract → digest → SNARK
*plumbing* runs end-to-end on this machine, and shaking out
integration bugs. The authoritative faithful run still needs the
real `pearl_gemm` on an NVIDIA GPU.

## 1. Upstream plugin architecture (as investigated)

`vllm-miner` is a vLLM `general_plugins` entry point. **It
extends vLLM's stock compressed-tensors path and swaps only the
scheme/kernel** — the weight *loading* (safetensors I8 +
BF16 scale) is vLLM's, unchanged.

| File | Role | CUDA? |
|---|---|---|
| `register.py` | entry `register_pearl_miner_layer` → `register_quantization_config("pearl")(PearlConfig)`; also inits async mining mgr / pinned pool **only in vLLM workers** (gated by `_is_vllm_worker()` + `no_vllm_plugin`) | no (the mining-mgr init is gated/skippable) |
| `vllm_config.py` | `PearlConfig(CompressedTensorsConfig)` — overrides only `_get_scheme_from_parts`; classifies 7-bit→mining / 8-bit→non-mining → `PearlScheme(mining_enabled=…)` | no |
| `vllm_scheme.py` | `PearlScheme(CompressedTensorsScheme)`; `create_weights`/`apply_weights`→`PearlKernel`; `get_min_capability()→9` (Hopper gate) | no (but Hopper-gated) |
| `vllm_kernels.py` | `PearlKernel(Int8ScaledMMLinearKernel)`; `can_implement`/`is_supported` **hard-require `current_platform.is_cuda()`**; `apply_weights` → int quant + `pearl_gemm_{vanilla,noisy}` | gate only |
| `quantization_operators.py` | `quant_{7,8}bit[_smooth]` → `from pearl_gemm import quantize` (in-place int8 + per-token fp32 scale) | **yes** |
| `gemm_operators.py` | `pearl_gemm_vanilla` (dequant matmul), `pearl_gemm_noisy` (PoW NoisyGEMM + commitment + block submit) — `from pearl_gemm import …`, pervasive `device="cuda"` | **yes** |
| `mining_state.py` | async loop mgr + pinned host pool (mining submission) | host-pinned (CUDA) |
| `config.py`/`vllm_config.py`/`callbacks.py` | settings, `should_use_noisy_gemm`, async callbacks | no |

**The only true CUDA surface is `pearl_gemm`** (a compiled
extension) used for exactly: `quantize`, `pearl_gemm_vanilla`,
`pearl_gemm_noisy`, `commitment_hash_from_merkle_roots` +
noise-factor gen. Everything else is Python glue or
gated/skippable mining-network plumbing.

**Inference vs mining split (decisive):** `PearlKernel.apply
_weights` → `_apply_weights_non_mining` is **int8 quant +
`pearl_gemm_vanilla` only** — a plain dequant matmul, *no noise,
no commitment, no CUDA-essential math*. `_apply_weights_mining`
uses `pearl_gemm_noisy` **only** when `config.should_use_noisy
_gemm(m,n,k) and not settings.no_mining`; with `no_mining=True`
it falls back to `pearl_gemm_vanilla`. ⇒ **for a real forward
pass / activation we need only `quantize` + `pearl_gemm_vanilla`
on CPU.** The noisy/commitment path is out of scope for the
activation goal (and we already have it faithfully in Rust
`ai_pow` + the B1-audited reference if ever needed).

## 2. The CPU-port surface (small, well-specified)

Replace the `pearl_gemm` CUDA extension with a pure-PyTorch-CPU
`pearl_gemm_cpu` providing the **exact** functions the plugin
imports, faithful to the B1-audited spec:

- **`quantize(x, x_q, x_s, fast_math, max_val, smooth_scale)`** —
  symmetric per-token: `s = x.abs().amax(-1,keepdim)/max_val`;
  `x_q = (x/ s).round().clamp(-max_val,max_val).to(int8)`;
  write `x_q`,`x_s` in place. (`smooth_scale`: divide `x` first.)
  Exactly the contract `ai_pow::quant` encodes.
- **`pearl_gemm_vanilla(x_q, w_q, scale_a, scale_b, out_dtype)`**
  — `((x_q.float() @ w_q.float().T) * scale_a[:,None] *
  scale_b[None,:]).to(out_dtype)`. The mined integer accumulate
  is `x_q @ w_q.T` (== `ai_pow::quant::int_matmul`'s relation).
- *(deferred, mining-only)* `pearl_gemm_noisy` +
  `commitment_hash_from_merkle_roots` + noise gen — re-derivable
  from `ai_pow` / the B1-audited Pearl reference; **not built in
  v1** (route through vanilla via `no_mining=True`).

Dependency reduction (vendor-local stubs):

| Upstream dep | v1 CPU treatment |
|---|---|
| `pearl_gemm` (CUDA) | **reimplement** → `pearl_gemm_cpu` (quantize + vanilla) |
| `miner_utils.get_logger` | trivial stdlib `logging` shim |
| `miner_base.settings.MinerSettings` | minimal dataclass: `no_vllm_plugin=False`, `no_mining=True`, … |
| `miner_base.commitment_hash`, `gpu_matmul_config`, `async_loop_manager` | stub (mining-only; unused on the vanilla inference path) |
| `mining_state` (async mgr, pinned pool) | no-op stubs (only touched in vLLM workers / mining) |
| `pearl_gateway.*` | stub (network submission; not on the inference path) |
| `compressed_tensors` | real PyPI dep (pure-python; weight schema) |
| `vllm` | **CPU build from source** (§4 risk) |

vLLM API-drift / CUDA-gate edits (the plugin code itself):
- `vllm_kernels.PearlKernel.can_implement` / `is_supported` /
  `get_min_capability` and `vllm_scheme.PearlScheme.get_min
  _capability` — **strip the `current_platform.is_cuda()` /
  capability≥9 (Hopper) gates** so the CPU platform is accepted.
- `register.py` — ensure the worker-only mining-mgr init is
  skipped (`no_vllm_plugin`/non-worker path) so no async/pinned
  CUDA pool is created.
- vLLM 0.21 (the local checkout) vs the pinned 0.20: reconcile
  `Int8ScaledMMLinearKernel` / `CompressedTensorsScheme` /
  `model_executor.parameter.*` import paths + signatures as the
  build surfaces them (user accepted the drift).

## 3. Validation strategy (keeps the fork faithful — R1)

The CPU kernels are a reimplementation ⇒ they must be pinned to
the audited spec, else the "real activation" is meaningless:

- **K-CPU-1:** `pearl_gemm_cpu.quantize` vs an independent
  reference (the symmetric-int7 math) on random + edge tensors
  (saturation at ±max_val, per-token scale, smooth_scale).
- **K-CPU-2:** `pearl_gemm_cpu.pearl_gemm_vanilla`'s integer
  accumulate == `ai_pow::quant::int_matmul` on the same
  `(x_q,w_q)` — i.e. cross-check the Python CPU kernel against
  the **Rust audited contract** (the B2-contract KAT's relation)
  for a real `gate_proj` tile. This bolts the Python fork to the
  same spec B1-audit proved ≡ real `pearl/zk-pow`.
- **K-CPU-3 (end-to-end):** smallest model → vendored plugin
  forward on a real prompt → capture a mined-layer activation →
  `ai_pow::quant::extract` + `BlockContext` digest runs (the
  Phase-D smoke; correctness already covered by B2.2/B1.1).

## 4. Risks (honest, ordered by likelihood-to-block)

1. **vLLM macOS-arm64-CPU build (the crux).** Experimental,
   from-source, FP32/FP16 only, long/fragile compile. If it
   won't build/run here, the whole vLLM path is blocked
   regardless of the fork — report as a hard env blocker, fall
   back to the lean no-vLLM harness (still gets a real-ish
   activation) or the GPU recipe.
2. **vLLM-CPU may not support this quant path at all.** The CPU
   backend doc says FP32/FP16; vLLM-CPU's compressed-tensors /
   `Int8ScaledMMLinearKernel` machinery may have no CPU code
   path even with the plugin's scheme. Validate **early** with a
   tiny config before the full 8B load.
3. **Python 3.12 pin** — resolved: `uv` 0.11.14 installed; use
   `uv python install 3.12` + a 3.12 venv (3.14/3.13 only
   present system-wide; torch/vLLM lack 3.14 wheels).
4. vLLM 0.21↔0.20 API drift (user-accepted; iterate at build).
5. 8B CPU forward is slow (minutes); 31B/70B impractical (use
   `--max-model-len` small, few tokens, eager).

## 5. Staged plan (R1 — commit per validated stage; honest status)

- **V1 — vendor** ✅ (`crates/ai-pow/pearl_vllm_cpu/`, pearl ref
  `3be33a59`) + this design doc.
- **V2 — CPU kernels + stubs.** `pearl_gemm_cpu` (quantize,
  vanilla) + `miner_utils`/`miner_base`/`mining_state` stubs;
  K-CPU-1/K-CPU-2 green (incl. the Rust `ai_pow::quant`
  cross-check on a real tile). *No vLLM needed — landable +
  validatable independently; de-risks the faithful core before
  the fragile vLLM build.*
- **V3 — vLLM-CPU env.** `uv` Python 3.12 venv; build vLLM from
  the `~/Dev/vllm` checkout for macOS CPU; **probe the
  quant-path feasibility on a toy config (Risk-2) before 8B.**
- **V4 — wire the fork into vLLM-CPU.** Strip CUDA/Hopper gates;
  fix 0.21 API drift; `register("pearl")` loads; the smallest
  model's `_get_scheme_from_parts` dispatches to the CPU kernel.
- **V5 — end-to-end smoke.** Load `Llama-3.1-8B-Instruct-pearl`,
  short prompt, eager, tiny `max-model-len`; capture a mined
  (group_1 7-bit) layer's input activation; run
  `ai_pow::quant::extract` + `BlockContext` → digest. K-CPU-3.

V2 is independent of the vLLM-build crux and carries the
faithful-core validation, so it lands first (R1: de-risk +
validate the part whose correctness matters before the part
whose *feasibility* is the open risk).

## 5b. OUTCOME — V1–V5 COMPLETE (2026-05-18)

**The Phase-D end-to-end smoke PASSED on the M2 Max (zero
CUDA).** Commits `d7ac209` (V1) · `fab5f2f` (V2) · `79db92f`
(V3+V4) · `33b4915` (V5).

- **V1** ✅ vendored + design.
- **V2** ✅ `pearl_gemm_cpu` (quantize+gemm) 4/4 vs the audited
  `ai_pow::quant` spec **on the real Llama-3.1-8B tile**.
- **V3** ✅ vLLM built from source + imports on macOS-arm64 CPU
  (`0.21.1rc1…cpu`). The Risk-1 crux *passed*; Risk-2 reduced
  (vLLM ships `kernels/linear/scaled_mm/cpu.py`).
- **V4** ✅ fork loads + `register("pearl")` against real
  vLLM-CPU 0.21 (CUDA/Hopper gates stripped; compat stubs;
  `pearl_gemm` redirected). No registration-path API drift.
- **V5** ✅ **real prompt → real `Llama-3.1-8B-Instruct-pearl`
  forward on vLLM-CPU → captured the first mined group_1 INT7
  GEMM's REAL activation** `A(64,4096) int8` × `B(4096,4096)
  int8` (|A|max=63 |B|max=62 ∈ Pearl `[−64,64]`); integer
  accumulate == the audited `ai_pow::quant` Σ A·B.T relation,
  bit-exact.

Six real blockers, each fixed honestly (full list:
`33b4915`): 0.20→0.21 `LLM()` kwarg drift; macOS
spawn-multiprocessing (`__main__` guard +
`VLLM_ENABLE_V1_MULTIPROCESSING=0`); `mining_state` rewritten
inert (engine-init touched it); CPU OOM
(`gpu_memory_utilization=0.45`); **group_0 FP8** — vLLM-CPU has
*no* FP8 ScaledMM kernel and its int8 CPU kernel is **x86-only**
(`CPUInt8ScaledMMLinearKernel` requires `CpuArchEnum.X86`), so
the un-mined FP8 layers were routed to a new
`PearlFp8CpuScheme` (CPU dequant + `F.linear`) to let the
forward reach the mined INT7 GEMMs; capture moved into
`pearl_gemm_cpu.gemm` (env-gated) because vLLM-CPU runs the
model in a `WorkerProc` subprocess.

**Findings / caveats (honest):**
- Our `PearlKernel` does the INT7 matmul itself
  (`pearl_gemm_cpu`), so it **bypasses** vLLM's x86-only CPU
  scaled-mm — the mined path works on Apple Silicon.
- group_0 FP8 dequant is *approximate* (not mined,
  B3-scoped-out) ⇒ the generated **text is garbage** — expected
  and acceptable; the mined-layer activation is still a real
  forward-pass tensor.
- Scope unchanged: the kernels are our audited-faithful CPU
  reimpl, **not** Pearl's CUDA kernel; **no new
  byte-equivalence evidence** (B2.2 + V2-real-tile + B1.1
  already cover that). V5's value is the **integration smoke**:
  the full real-prompt → real-model → INT7-mined-activation →
  audited-relation plumbing runs on Apple-Silicon CPU.
- 31B/70B remain CPU-impractical (memory/time); 8B is the
  tractable smallest model (as targeted).

## 6. Cross-references

- `crates/ai-pow/pearl_vllm_cpu/` — the vendored fork.
- `crates/ai-pow/PEARL_COMPARISON.md`,
  `B1_PEARL_FAITHFULNESS_AUDIT.md` — the audited Pearl spec the
  CPU kernels must match.
- `crates/ai-pow/src/quant.rs` (`ai_pow::quant`) — the
  bit-lossless contract (K-CPU-2 cross-check oracle).
- `PHASE_B_DESIGN.md` — Phase B (complete); this is Phase-D.
- Upstream: `pearl/miner/vllm-miner`, `pearl/miner/{miner-base,
  miner-utils,pearl-gemm}` @ pearl ref `3be33a59`.
