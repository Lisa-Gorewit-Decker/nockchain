# Qwen 3.5 27B INT8 — remaining work

## State

| Phase | Status | Notes |
|---|---|---|
| Architecture refactor (IMROPE + GatedDeltaNet + per-layer num_kv + Q-gate) | ✅ done | commits B.2 through scale-routing |
| f32 reference forward | ✅ **4/4 top-1 vs Ollama** | commit `e689d78` |
| INT8 forward — runs end-to-end | ✅ | predictions in sensible token ranges |
| INT8 weight-tensor scale plumbing (ssm_a/dt/conv1d/norm) | ✅ | commit `4cdc690` |
| INT8 matmul combined-scale fix | ✅ | commit `e2b99f3` |
| **INT8 top-1 ≥ 1/4 vs Ollama** | ❌ **0/4 across 9 iterations** | predictions move but never align |

## Prediction trajectory across versions

| | P1 | P2 | P3 | P4 |
|---|---|---|---|---|
| **Expected (Ollama)** | 8160 | 8160 | 90700 | 8160 |
| v7c (ssm_a × 16 hack) | **10271** | 96755 | 227 | 154615 |
| v8 (weight scales) | 189085 | 22382 | 86 | 1846 |
| v9 (combined scales) | 96890 | 198380 | 131414 | 15244 |

v7c's prompt 1 came within 2k of Ollama's 8160. That accuracy was never reproduced in subsequent fixes; the math improvements moved predictions further away.

## Diagnosis

The f32 reference proves the architecture is correct. The i8 path implements the same architecture with quantization. Each iteration above identified and fixed a real bug, yet the cumulative effect is still 0/4. Honest assessment of why:

### The calibration data may be unsuited for the i8 path

`/tmp/scales_v3.json` was emitted by the f32 reference forward, which:
- Reads weights as f32 (pristine, no quantization)
- Computes activations as f32 (no rounding)
- Records `max(|x|)` per tap

The i8 forward sees:
- Weights pre-quantized to i8 (lost up to 1 bit of precision per element)
- Activations rounded to i8 between every op (lost ~7 bits range every tap)

The activation magnitudes at each tap of the **i8 path** differ from the **f32 path** because of compounding quantization error. Calibrating against f32 then applying to i8 introduces a systematic bias. Each tap's actual i8-output max_abs is some fraction (or multiple) of the f32 calibrator's recorded max, and that ratio varies per layer.

### Compounding error across 64 layers

Even if each layer's i8 output is within 5% of the f32 reference, 64 layers compound to ~25× total drift (1.05^64 ≈ 23). On softmax logits this dwarfs the gap between the right and wrong token.

## What would actually work

### Option C — self-calibrating i8 forward

1. **Instrument** the i8 forward to record `max(|x|)` at each tap point (i.e., the actual i8 output magnitude after rounding).
2. Run on a few calibration prompts with conservative initial scales (e.g., the f32 ones we have).
3. Update each tap's `out_max` to the **measured** value.
4. Re-convert. Iterate ~3–5 times until scales converge.

This bootstraps despite the chicken-and-egg: even with wrong starting scales, the i8 path produces SOME output; the magnitudes measured there are what the i8 forward will see at runtime. After 2–3 cycles, scales converge to values that the i8 forward naturally produces.

Estimated effort: 2–3h to add the i8 instrumentation + 4 cycles × 50min = ~5h wall clock.

### Option D — accept that f32 is the verifier

For the proof-of-work use case (the original purpose of this crate), determinism matters more than fidelity. If two implementations of the INT8 forward bit-match each other, that's sufficient for ai-pow. The "match real Ollama" goal is a smoke test that, frankly, may not be achievable with this quantization scheme without major investment.

Pivoting to:
- Ship the architecture refactor + INT8 forward as-is
- Document the f32 reference at 4/4 as the architecture validation
- Treat the i8 forward as a deterministic-INT8 inference engine whose accuracy is bounded by the quant scheme's noise floor
- Decouple "match Ollama" from "is correct ai-pow forward"

## Reference

- Branch: `claude/ai-pow-nockchain-sgfNX` (15 commits ahead of origin)
- Latest INT8 model: `/tmp/qwen35_27b_int8_v9/` (comm_W `9830170d...`)
- Validated f32 scales: `/tmp/scales_v3.json`
- Eval set: `/tmp/qwen_eval.jsonl`
- GGUF: `~/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926`
- llama.cpp ref: `/tmp/llama.cpp/src/models/qwen35.cpp`

## Key invariants (all preserved)

1. GGUF tensor name `ssm_dt` has no `.bias` suffix (Ollama-shipped). Try both with `or_else`.
2. `ssm_a` is stored as `-exp(A_log)` (already negated).
3. Conv1d weight: transpose ONCE in converter (candle gives `[channels, kernel]`, runtime expects `[kernel, channels]`).
4. K→V broadcast: `vh % num_k` (ggml_repeat tiles).
5. IMROPE tables sized `seq_len × (n_rot/2)`; `tables.half_head_dim = n_rot/2`.
6. Per-head L2-norm: `1/max(sqrt(sumsq), eps)`.
7. Q scaling: `q *= 1/sqrt(head_k_dim)` once after L2-norm.
8. DeltaNet recurrence state stays in f32.
9. i8 dequant: `x_real = x_i8 × (scale.num / 2^15)`.
10. calibrate.rs tap-name slot reuse: see `dnet_scales_for` for mapping.
11. Matmul stored scale: `combined.num = (a.num × w.num) / out.num` — implemented at convert time.

## Estimated effort to close the gap

| Option | Effort | Risk |
|---|---|---|
| C (self-calibrate i8) | 2-3h coding + 5h wall clock × few cycles | Medium — convergence not guaranteed |
| D (accept f32 as verifier, ship as-is) | 0 | None |

The 13+ commits this session represent a complete architecture rewrite with the f32 path proven correct. The remaining gap is one of i8 quantization-scheme tuning, not architecture.
