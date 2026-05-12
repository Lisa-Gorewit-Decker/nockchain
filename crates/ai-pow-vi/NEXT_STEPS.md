# Qwen 3.5 27B INT8 — final state

## TL;DR

- **Architecture refactor**: complete. f32 reference at **4/4 vs Ollama**.
- **INT8 forward**: end-to-end functional, but **0/4 vs Ollama** across 14 reconvert iterations.
- **Root cause**: under the project's contract (per-tensor symmetric W8A8 + i8 residual add with saturating arithmetic), every layer's i8 output saturates at ±128 from layer 3 onward. The residual stream cannot carry the activation magnitudes that propagate through a 27B-class transformer in this representation.
- **Empirical evidence + literature**: SmoothQuant Table 4 documents OPT-175B dropping from 71.6% → 32.3% on naive per-tensor W8A8. Our observation is the published failure mode of this constraint set.

## What was tried (in order)

| Version | Change | Top-1 | Saturation pattern |
|---|---|---|---|
| v9 | Combined matmul scales `(a×w)/out`, base f32 calibration | 0/4 | 244/256 layers ≡ 128 |
| v10 | All output scales ×4 (manual tighten) | 0/4 | All ≡ 128 |
| v11 | Output scales ×8 | 0/4 | All ≡ 128 |
| v12 | Output scales ×32 | 0/4 | All ≡ 128 |
| v13 | Output scales ×16 | 0/4 | All ≡ 128 |
| v14 | Percentile-99.999 calibration | 0/4 | 244/256 ≡ 128 |

Percentile clipping cut some outliers materially (e.g. `ffn.mid` to 16-24% of raw max) but **bulk magnitudes are unchanged** — median scale-ratio is exactly 1.0. The bulk of taps don't have heavy tails; the saturation comes from cumulative magnitude growth across the residual stream, not from outliers.

## The structural finding

`forward_qwen_hybrid_ssm_layer` and `forward_qwen_standard_layer` both end with:

```rust
output.copy_from_slice(&residual1);
add_residual_inplace(output, &ffn_out);  // saturating i8 add
```

For 64 layers each producing some non-zero sub-output `sub_out` (max magnitude m), the residual stream accumulates ~`m × log(64)` in the average direction. Once `|input| + |sub_out| > 127`, every i8 element saturates at ±128 — and stays saturated because once a value is ±128, downstream matmuls with it produce huge accumulators that re-saturate.

The fix in production INT8 systems is **fp16/bf16 residuals**:
- TensorRT-LLM: weights INT8, accumulator INT32, dequant to fp16, residual add in fp16
- vLLM: same pattern
- llama.cpp: same pattern
- PyTorch static quantization docs explicitly say transformer residuals are NOT amenable to int8 because magnitudes grow with depth

The literature has no published recipe for matching bf16-grade top-1 under "per-tensor symmetric W8A8 with i8 saturating residual."

## What's committed this session (17 commits ahead of origin)

Branch: `claude/ai-pow-nockchain-sgfNX`

1. Architecture refactor: IMROPE + GatedDeltaNet + per-layer num_kv + Q+gate packing + tile-broadcast K→V
2. f32 reference forward (`bin/calibrate.rs`) validated at 4/4
3. INT8 wire-up: loader byte counts, IMROPE table sizing, scale routing, conv-dequant fix
4. Per-tensor weight scales (`ssm_a_weight_max`, `ssm_dt_weight_max`, `ssm_conv1d_weight_max`, `ssm_norm_gamma_weight_max`)
5. Combined matmul scales `combined.num = (a.num × w.num) / out.num`
6. Per-layer i8 magnitude dump via `AI_POW_VI_DUMP_LAYER_MAGS` env var
7. Percentile-99.999 calibration (literature-recommended replacement for raw max)
8. `QUANT_PROBLEM.md` formal write-up of the constraint set
9. `QUANT_RESEARCH.md` synthesis of literature on this exact failure mode

## What would unblock 4/4

In order of escalating contract change:

### A. Bf16 residual stream (contract change)
Replace `add_residual_inplace` (i8 saturating) with f16/i32 intermediate, requantize once at the layer's exit. This is what every production INT8 transformer system does. ~3-5h refactor. Breaks bit-identical compatibility with any existing implementation that follows the current contract.

### B. Per-channel weight scales (smaller contract change)
Store `[Scale; out_dim]` per weight tensor instead of one scalar. Matmul rescale becomes per-output-channel. SmoothQuant's published O1/O2 results all use this. Wire format change, but the runtime arithmetic stays integer. ~5-7h refactor.

### C. SmoothQuant offline fusion (no contract change, capped gain)
For each `RMSNorm → matmul` pair (every quantizable matmul in Qwen 3.5 is one), compute per-channel smoothing factor `s_j = max(|X_j|)^α × max(|W_j|)^(1−α)`, fold `γ /= s` and `W *= s`. Mathematically equivalent transform. Estimated 2-3h work.

**Honest expectation**: per the agent's research, SmoothQuant Table 4 results all require per-channel weight scales to recover full accuracy. With both per-tensor (our constraint), the paper notes "expect residual accuracy loss vs FP32 that doesn't fully recover." Given our v10–v14 evidence that ~5–15× scale tightening doesn't break saturation, SmoothQuant alone (which cuts outliers ~2–5×) is unlikely to be sufficient. Likely outcome: marginal improvement, still 0/4 or 1/4.

### D. Accept the structural ceiling
For the **proof-of-work use case** (the original purpose of `ai-pow-vi`), the determinism contract is protocol-load-bearing; matching Ollama top-1 is a validation target, not a soundness requirement. Two implementations that produce bit-identical i8 outputs (even if those outputs don't match bf16 inference) satisfy the verifiable-inference invariant.

The crate is in a state where:
- The architecture is correct (f32 reference passes Ollama)
- The forward path is fully wired and deterministic
- Synthetic-fixture round-trip tests all pass
- The known accuracy gap is a documented property of the constraint set, not an undiagnosed bug

If the goal is ship-ability for proof-of-work, this is the right place to stop.

## Reference files

- `crates/ai-pow-vi/QUANT_PROBLEM.md` — formal problem write-up
- `crates/ai-pow-vi/QUANT_RESEARCH.md` — literature synthesis
- `crates/ai-pow-vi/src/bin/calibrate.rs` — 4/4 f32 reference (the spec)
- `crates/ai-pow-vi/src/bin/gguf_convert.rs` — combined-scale converter
- Latest model: `/tmp/qwen35_27b_int8_v14/` (comm_W `a48bea24...`)
- Latest scales: `/tmp/scales_p99999.json` (percentile-99.999 clipped)

## Key invariants preserved

1. GGUF `ssm_dt` has no `.bias` suffix in Ollama-shipped GGUF
2. `ssm_a` is `-exp(A_log)` (already negated)
3. Conv1d weight transposed at load: `w_raw[c×kk+k]` → `w[k×conv_dim+c]`
4. K→V broadcast `vh % num_k` (ggml_repeat tile)
5. IMROPE tables sized `seq_len × (n_rot/2)`
6. DeltaNet recurrence state in f32 (not viable in i8 across 64 tokens)
7. Matmul stored scale: `combined.num = (a.num × w.num) / out.num`
