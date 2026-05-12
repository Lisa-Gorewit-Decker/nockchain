# Qwen 3.5 27B INT8 — remaining work

End-goal: `gguf-convert` produces a Qwen 3.5 27B model dir whose `vi-eval` agrees with Ollama 4/4 on `/tmp/qwen_eval.jsonl` (predicting `8160, 8160, 90700, 8160`).

The **f32 reference path** gets 4/4 at commit `e689d78`. The **INT8 path** runs end-to-end without panics, produces non-trivial predictions in reasonable token-id ranges, but is at **0/4** because the matmul convention assumes `max(|w|) ≈ 1` for every weight tensor — true for some weights but not all.

## State at session end (branch `claude/ai-pow-nockchain-sgfNX`)

| Commit | Done |
|---|---|
| `2d01809` | Phase B.2 — packed Q+gate for std blocks |
| `19f8105` | Real arch in calibrate.rs (IMROPE + GatedDeltaNet + per-layer num_kv) |
| `e689d78` | K→V broadcast fix. **4/4 top-1 vs Ollama** in f32. |
| `6c55fd6` | INT8 scaffolding — rope IMROPE primitives + deltanet forward |
| `5f37d88` | Wire `forward_gated_deltanet_qwen35` into runtime; converter rewrite |
| `b44246b` | Loader byte counts for DeltaNet + IMROPE table sizing fix |
| `3fd5af8` | Scale tap-name routing fix |
| `d695e5c` | Fix vestigial /127 in deltanet conv-output dequant |
| `0a35eb2` | Validation: ssm_a × 16 hack (proved weight-scale theory) |
| `4cdc690` | Per-tensor weight max_abs scales for ssm_a / dt / conv1d / norm |

All 246 unit + integration tests pass (~10 ignored pending qwen_hybrid_mini fixture regen).

Latest INT8 model: `/tmp/qwen35_27b_int8_v8/` (comm_W `88a723d6...`).

Validated f32 scales on disk: `/tmp/scales_v3.json` — 1539 taps from the 4/4 f32 reference.

## Prediction trajectory across versions

| Prompt | v7 | v7b (/127 fix) | v7c (×16 hack) | v8 (proper scales) | Expected |
|---|---|---|---|---|---|
| 1 | 145890 | 84956 | 10271 | 189085 | 8160 |
| 2 | 100 | 232364 | 96755 | 22382 | 8160 |
| 3 | 219132 | 3503 | 227 | 86 | 90700 |
| 4 | 15277 | 327 | 154615 | 1846 | 8160 |

Predictions are in sensible token-id ranges (86, 1846, 22382, 189085) but still don't match Ollama's exact argmax.

## The remaining gap

Per-tensor weight max_abs scales were plumbed for `ssm_a`, `ssm_dt`, `ssm_conv1d`, `ssm_norm_gamma` (commit `4cdc690`). The same dropped-weight-scale issue likely affects:

- `attn_qkv` (5120 × 10240 = 50M params per hybrid layer, large enough that max_abs deviation has real impact)
- `attn_gate` (5120 × 6144)
- `ssm_alpha`, `ssm_beta` (small but per-token gates)
- `ssm_out` (6144 × 5120)
- All **standard-block** weights (`attn_q`, `attn_k`, `attn_v`, `attn_output`, `ffn_*`) and **FFN** weights in hybrid blocks

These go through `matmul_int8_requant`, whose convention bakes in `max(|w|) ≈ 1`. For Q4_K weights typically <1 (~0.1 to 0.5), this causes consistent under-saturation — each i8 only spans a fraction of [-127,127], reducing effective precision by 2–10×.

The fix is more invasive than the ssm_a one because `matmul_int8_requant` does the scale arithmetic internally; it would need a new variant that accepts a separate `w_max_abs` Scale. Or every matmul weight gets a scale field on its layer struct (significant manifest expansion).

## Concrete fix path

### Option A — proper matmul-with-weight-scale (the right way)

Add `w_max_abs` Scale fields to every weight-bearing struct (AttentionWeights, FfnWeights, DeltaNet-related). Modify `matmul_int8_requant` to accept the weight's max_abs and fold it into the rescale: `output_scale_eff = act_scale × w_max_abs`.

Plumb through:
- `src/attention.rs::AttentionWeights` (add `q_max_abs`, `k_max_abs`, `v_max_abs`, `o_max_abs`)
- `src/ffn.rs::FfnWeights` (add `gate_max_abs`, `up_max_abs`, `down_max_abs`)
- `src/layer.rs::QwenHybridSsm` (add `attn_qkv_max_abs`, `attn_gate_max_abs`, `ssm_alpha_max_abs`, `ssm_beta_max_abs`, `ssm_out_max_abs`)
- `src/matmul_int8.rs::matmul_int8_requant` — accept `w_scale: Scale` parameter; combine with act_scale
- All matmul call sites — pass the weight's `max_abs`
- `src/io.rs` + `src/comm_w.rs` — serialize/deserialize new fields
- `src/bin/gguf_convert.rs` — capture and store every weight's scale_num

Estimated effort: 4–6h focused work + 22min reconvert + 25min eval.

### Option B — bypass the convention (simpler, less elegant)

Normalize every weight tensor at convert time so `max(|w|) = 1` exactly, then bake the scaling factor into the activation scale that feeds the matmul. E.g., for `attn_qkv @ x`: divide weight by `max_abs(w)`, multiply scale `ssm.qkv` (which represents the output activation max) by `max_abs(w)`. Net effect: matmul output as i32 represents the same f32 value.

Estimated effort: 1–2h. But it changes the meaning of the activation scales (they now bake in weight scales), so re-calibration may be needed.

### Step 4 — re-eval

After either option:

```sh
./target/release/gguf-convert --gguf $HOME/.ollama/models/blobs/sha256-83c5... \
  --out /tmp/qwen35_27b_int8_v9 --seq-len 64 --activation-tile 64 \
  --scales /tmp/scales_v3.json
./target/release/vi-eval --model-dir /tmp/qwen35_27b_int8_v9 \
  --eval /tmp/qwen_eval.jsonl --arch qwen35
```

Expected outcome: 3-4/4 top-1 (the f32 reference is 4/4 with these scales, so the only variable is residual quantization noise which should be bounded < 5% per layer).

## Key invariants (don't relearn these)

1. **GGUF tensor name `ssm_dt`** has no `.bias` suffix in the Ollama-shipped GGUF. Try both with `or_else`.
2. **`ssm_a`** is stored as `-exp(A_log)` (already negated); use as a multiplier, not a divisor.
3. **Conv1d weight layout**: candle returns `[channels, kernel]` PyTorch-style with memory `w[c*kk + k]`. Runtime expects kernel-outer `w[k*conv_dim + c]` — transpose ONCE in converter.
4. **K→V broadcast**: `vh % num_k` (ggml_repeat tiles), NOT `vh / kv_groups`.
5. **IMROPE tables**: sized `seq_len * (n_rot/2)`, so `tables.half_head_dim = n_rot/2`. Rope_apply slice length = `2 * tables.half_head_dim` covers exactly the rotated subspace.
6. **Per-head L2-norm**: `1/max(sqrt(sumsq), eps)`, not `1/sqrt(sumsq+eps)`.
7. **Q scaling**: `q *= 1/sqrt(head_k_dim)` applied ONCE after L2-norm, before the recurrence.
8. **DeltaNet recurrence state**: keep in f32; not viable in i8 due to 64-step multiplicative decay.
9. **i8 dequant convention**: `x_real = x_i8 * (scale.num / 2^15)` where scale.num = round(max_abs/127 * 2^15). For a weight tensor where max_abs is captured: `max_abs = scale.num * 127 / 2^15`.
10. **calibrate.rs tap-name reuse**: records under OLD DeltaNetScales slot names with NEW semantics. See `dnet_scales_for` for the full slot-to-tap map.
11. **Matmul convention**: `matmul_int8_requant(a, b, …, scale, out)` implicitly assumes `max(|b|) ≈ 1`. Weights with `max(|b|) < 1` (typical Q4_K) cause under-saturation in the i8 output. This is the dominant remaining bug.

## Reference files

- `/tmp/llama.cpp/src/models/qwen35.cpp` — canonical computation graph
- `/tmp/llama.cpp/src/models/delta-net-base.cpp` — DeltaNet recurrence
- `crates/ai-pow-vi/src/bin/calibrate.rs` — **validated 4/4 f32 reference**; treat as spec
- `/tmp/scales_v3.json` — validated per-tap scales (1539 entries)
- `/tmp/qwen_eval.jsonl` — 4-prompt Ollama reference set
- GGUF blob: `~/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926`
- Latest INT8 model: `/tmp/qwen35_27b_int8_v8/` (comm_W `88a723d6...`)
