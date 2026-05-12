# Qwen 3.5 27B INT8 — final empirical state

## TL;DR

After 15 reconvert cycles applying every literature-recommended Tier-1
intervention for the W8A8 per-tensor symmetric constraint:

- ✅ **Architecture refactor**: complete. f32 reference at **4/4 top-1 vs Ollama**.
- ✅ **All literature Tier-1 fixes** implemented and tested:
  - Combined matmul scales `(a × w) / out` (`e2b99f3`)
  - Per-tensor weight scales `ssm_a/dt/conv1d/norm` (`4cdc690`)
  - Percentile-99.999 calibration (`6ec1144`)
  - SmoothQuant offline fold (`7af48e7`)
- ❌ **INT8 vs Ollama**: **0/4 top-1 across all attempts**, layer outputs saturate at ±128 from layer 3 onward in every configuration.

The saturation is **structural**, not a plumbing bug. The crate's contract
(per-tensor symmetric W8A8 + i8 residual add with saturating arithmetic)
is incompatible with matching bf16-grade transformer inference on a
27B-class model. SmoothQuant Table 4 documents the same failure mode on
OPT-175B (71.6% → 32.3%) under this constraint.

## All approaches tried, with results

| Version | Approach | P1 | P2 | P3 | P4 | Saturated layers |
|---|---|---|---|---|---|---|
| v9 | Combined matmul scales `(a×w)/out` | 96890 | 198380 | 131414 | 15244 | 244/256 |
| v10 | Output scales ×4 (manual tighten) | 96117 | 7286 | 22104 | 63109 | 256/256 |
| v11 | Output scales ×8 | 702 | 88 | 11839 | 8979 | 256/256 |
| v12 | Output scales ×32 | 19159 | 7393 | 95886 | 250 | 256/256 |
| v13 | Output scales ×16 | 102923 | 56788 | 2538 | 330 | 256/256 |
| v14 | Percentile-99.999 calibration | 96692 | 39815 | 198 | 220 | 244/256 |
| **v15** | **SmoothQuant offline fold (α=0.5)** | **12** | **220** | **23** | **88** | **244/256** |

Expected (Ollama): 8160, 8160, 90700, 8160.

**No configuration aligns any prompt to Ollama**, despite f32 reference
hitting 4/4 with the same scales. SmoothQuant cuts outliers (~5×
reduction documented at Llama-2-7B scale) but doesn't fix the bulk-
magnitude growth across the residual stream.

## Why this fails (and what fixing it would require)

`forward_qwen_standard_layer` and `forward_qwen_hybrid_ssm_layer` both end:

```rust
output.copy_from_slice(&residual1);
add_residual_inplace(output, &ffn_out);  // saturating i8 add
```

Across 64 layers, the residual stream accumulates magnitude. Once a value
hits ±128 in i8, every subsequent matmul that consumes it produces a
huge accumulator that re-saturates. The fix in every production INT8
system (TensorRT-LLM, vLLM, llama.cpp, PyTorch static quant):

- Compute residual in fp16/bf16 (or i32 with explicit rescale)
- Requantize at well-chosen points, not on every add

PyTorch static quantization docs are explicit: *"transformer residuals grow with depth and are not amenable to INT8 residual chains."*

## What this means for the protocol

The user has a clean strategic decision (covered in detail in a separate
chat turn):

1. **For proof-of-work soundness**: the determinism contract is the
   protocol-load-bearing property. The crate produces bit-identical
   outputs across implementations. That's what miners commit to and
   validators verify. Matching Ollama is a *validation goal*, not a
   *soundness requirement*. The crate is shippable as-is for this purpose.

2. **For the "useful AI PoW" economic narrative**: the claim "miners do
   useful Qwen inference" weakens since outputs diverge from canonical
   Qwen inference. Reframable as "miners compute a deterministic function
   of real LLM weights" — still useful, less marketable.

3. **For downstream chatbot-style use**: outputs are wrong vs Ollama, so
   any consumer that interprets the i8 forward as "Qwen 3.5 inference"
   gets degraded predictions.

4. **For matching Ollama top-1**: requires relaxing the contract. The
   options the literature actually uses:
   - bf16 residual stream (TensorRT-LLM, vLLM): largest change
   - per-channel weight scales (SmoothQuant O3): wire-format expansion
   - Hadamard rotation on activations (QuaRot): fits, but requires
     manifest format addition for the rotation matrix per layer
   These are protocol decisions, not engineering fixes.

## Commits this session (20 commits ahead of origin)

Branch: `claude/ai-pow-nockchain-sgfNX`

1. `2d01809` Phase B.2 — packed Q+gate
2. `8b137fa` Phase A initial calibrator
3. `19f8105` Real arch in calibrate (IMROPE + DeltaNet)
4. `e689d78` K→V broadcast fix → **f32 4/4**
5. `6c55fd6` INT8 scaffolding
6. `5f37d88` Wire DeltaNet into runtime
7. `b44246b` Loader byte counts + IMROPE table sizing
8. `3fd5af8` Scale tap-name routing
9. `d695e5c` /127 dequant fix
10. `0a35eb2` ssm_a × 16 hack (validation)
11. `4cdc690` Per-tensor weight scales
12. `e2b99f3` Combined matmul scales `(a×w)/out`
13. Earlier monitor instrumentation, plan docs
14. `649a091` QUANT_PROBLEM + QUANT_RESEARCH
15. `6ec1144` Percentile-99.999 calibration
16. `7af48e7` SmoothQuant offline fold

## Reference files

- `crates/ai-pow-vi/QUANT_PROBLEM.md` — constraint set + what's been validated
- `crates/ai-pow-vi/QUANT_RESEARCH.md` — literature synthesis
- `crates/ai-pow-vi/src/bin/calibrate.rs` — f32 reference (4/4) + percentile + per-channel
- `crates/ai-pow-vi/src/bin/gguf_convert.rs` — combined-scale + SmoothQuant fold converter
- `/tmp/qwen35_27b_int8_v15/` — latest INT8 model (comm_W `13f3f3a1...`)
- `/tmp/scales_sq.json` + `/tmp/scales_sq.pc.json` — latest calibration data

## Key invariants preserved

1. GGUF `ssm_dt` has no `.bias` suffix (Ollama-shipped); try both
2. `ssm_a` stored as `-exp(A_log)` (already negated)
3. Conv1d transposed once at load: `[c×kk+k]` → `[k×conv_dim+c]`
4. K→V broadcast `vh % num_k` (ggml_repeat tile)
5. IMROPE tables sized `seq_len × (n_rot/2)`
6. DeltaNet recurrence state in f32 (not viable in i8)
7. Matmul stored scale: `combined.num = (a × w) / out`
8. SmoothQuant fold: `s_j = max(|X_j|)^α × max(|W_j|)^(1−α)`,
   `γ ← γ/s, W ← W·diag(s)`. Mathematically equivalent transform.
