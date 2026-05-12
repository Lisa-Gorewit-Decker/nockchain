# Qwen 3.5 27B INT8 — remaining work

End-goal: `gguf-convert` produces a Qwen 3.5 27B model dir whose `vi-eval` agrees with Ollama 4/4 on `/tmp/qwen_eval.jsonl` (predicting `8160, 8160, 90700, 8160`).

The **f32 reference path** already gets 4/4 at commit `e689d78`. The **INT8 path** runs end-to-end without panics, produces non-trivial predictions, but is at **0/4** because several weight-tensor scales are dropped in the i8 forward.

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

All lib + integration tests pass (~10 ignored pending qwen_hybrid_mini fixture regen).

Validated f32 scales on disk: `/tmp/scales_v3.json` — 1539 taps from the 4/4 f32 reference.

Latest INT8 model: `/tmp/qwen35_27b_int8_v7/` (comm_W `16e5f2a3...`), predictions across reruns:

| Prompt | v4-v6 | v7 (scale routing) | v7b (/127 fix) | Expected |
|---|---|---|---|---|
| 1 | 1482 | 145890 | **84956** | 8160 |
| 2 | 96515 | 100 | **232364** | 8160 |
| 3 | 22822 | 219132 | **3503** | 90700 |
| 4 | 53523 | 15277 | **327** | 8160 |

Each fix shifts predictions meaningfully, confirming the changes are flowing through. Magnitude of post-/127-fix tokens is now smaller (matches Ollama's small-token expectation), but argmax is wrong by ~10² positions — typical for a forward path with ~10–30% per-layer cumulative error.

## The remaining gap — dropped weight-tensor scales

The crate's INT8 convention bakes in **`max(|w|) ≈ 1` for every weight tensor** and uses only per-tap *activation* scales. This breaks down for tensors whose true `max(|w|)` is not ~1:

| Weight tensor | True `max(|w|)` | Off by | Where used |
|---|---|---|---|
| `ssm_a` | up to ~16 (`-exp(A_log)` with A_log ∈ log U(0,16)) | up to 16× | gate_log = softplus(α+dt) × ssm_a; affects state decay rate |
| `ssm_dt` | ~1 (initialized to 1) | ~1× | additive bias to α; small impact |
| `ssm_norm_gamma` | ~1 (RMSNorm γ) | ~1× | post-recurrence gated norm; small impact |
| `ssm_conv1d` | varies; typically <1 | up to ~3× | depthwise conv before SiLU; affects Q/K/V conv outputs |

The `gguf-convert` code calls `quantize_with_scale` for each, gets back `(i8_bytes, scale_num)`, then discards `scale_num` (line 881 et al). The runtime dequants via `(x as f32) / 127.0`, which only recovers values normalized to `[-1, 1]` instead of `[-max_abs, max_abs]`.

For `ssm_a` specifically, a 16× error in the gate scalar means `exp(gate_log)` is wildly wrong — either too close to 1 (no decay; state diverges) or too close to 0 (state collapses). This is the largest single source of error.

## Concrete fix path

### Step 1 — store ssm_a, ssm_dt, ssm_norm_gamma as f32 in the manifest

These are tiny vectors (48 + 48 + 128 = 224 values per hybrid layer × 48 layers = 10,752 f32 ≈ 43 KB extra). Cheaper than wiring per-weight Scale fields, and the dequant becomes a memcpy.

**Files to edit**:
- `src/layer.rs`: change `LayerWeights::QwenHybridSsm` field types
  - `ssm_a: Vec<i8>` → `Vec<f32>`
  - `ssm_dt: Vec<i8>` → `Vec<f32>`
  - `ssm_norm_gamma: Vec<i8>` → `Vec<f32>`
- `src/io.rs`: update `parse_one_layer` and `LayerMeta::QwenHybridSsm` byte counts. Each f32 is 4 bytes vs 1 byte i8. Add a meta byte sequence so the loader knows it's a hybrid-arch (DeltaNet) variant vs legacy Mamba — bump the variant tag.
- `src/comm_w.rs`: update `canonical_weight_bytes` to serialize as f32 LE bytes for these three fields.
- `src/bin/gguf_convert.rs::build_qwen_hybrid_layer`: stop calling `dequant_quantize` for these; load via `dequant_to_vec_f32` and store directly.
- `src/deltanet.rs::forward_gated_deltanet_qwen35`: the `opts.ssm_a / ssm_dt / ssm_norm_gamma` fields become `&[f32]`. Drop the `(x as f32) / 127.0` dequants at lines 711-712 and 785.

### Step 2 — store ssm_conv1d weight scale

`ssm_conv1d` is 4 × 10240 = 40960 i8 bytes per layer. Storing as f32 is 4× bigger but still only ~7.5 MB extra total. Or store the scale alongside:

- Add `ssm_conv1d_scale: Scale` to `QwenHybridSsm`
- Plumb through io.rs / comm_w.rs / gguf_convert.rs (the converter already has the scale; just stop discarding it)
- In `forward_gated_deltanet_qwen35` line 629, the formula becomes:
  ```rust
  // acc_i32 has units of i8_w * i8_x. Convert to f32 via both scales:
  let w_max_f = (opts.ssm_conv1d_scale.num as f32) / (1<<15) as f32 * 127.0;
  let approx_f = (acc_i32 as f32) * w_max_f * qkv_scale_f / 127.0;
  ```

### Step 3 — regenerate fixtures + un-ignore tests

After Steps 1-2 land, the qwen_hybrid_mini Python generator needs to be rewritten anyway (the manifest format and weight tensor types changed). Then un-`#[ignore]` the 10 tests.

### Step 4 — re-convert + re-eval

```sh
mkdir -p /tmp/qwen35_27b_int8_v8
./target/release/gguf-convert \
  --gguf "$HOME/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926" \
  --out /tmp/qwen35_27b_int8_v8 \
  --seq-len 64 --activation-tile 64 \
  --scales /tmp/scales_v3.json

./target/release/vi-eval --model-dir /tmp/qwen35_27b_int8_v8 \
  --eval /tmp/qwen_eval.jsonl --arch qwen35
```

Wall clock: ~22min convert + ~25min vi-eval.

**Expected outcome**: top-1 ≥ 3/4 (likely 4/4 since f32 reference is 4/4 and the only remaining variable is the bounded per-tap quantization noise after fixing weight-scale loss).

## Key invariants (don't relearn these)

1. **GGUF tensor name `ssm_dt`** has no `.bias` suffix in the Ollama-shipped GGUF. Try both with `or_else`.
2. **`ssm_a`** is stored as `-exp(A_log)` (already negated); use as a multiplier, not a divisor.
3. **Conv1d weight layout**: candle returns `[channels, kernel]` PyTorch-style with memory `w[c*kk + k]`. Runtime expects kernel-outer `w[k*conv_dim + c]` — transpose ONCE in converter.
4. **K→V broadcast**: `vh % num_k` (ggml_repeat tiles), NOT `vh / kv_groups`.
5. **IMROPE tables**: sized `seq_len * (n_rot/2)`, so `tables.half_head_dim = n_rot/2`. Rope_apply slice length = `2 * tables.half_head_dim` covers exactly the rotated subspace.
6. **Per-head L2-norm**: `1/max(sqrt(sumsq), eps)`, not `1/sqrt(sumsq+eps)`.
7. **Q scaling**: `q *= 1/sqrt(head_k_dim)` applied ONCE after L2-norm, before the recurrence.
8. **DeltaNet recurrence state**: keep in f32; not viable in i8 due to 64-step multiplicative decay.
9. **i8 dequant convention**: `x_real = x_i8 * (scale.num / 2^15)`. Do NOT multiply or divide by 127 again — the scale numerator already encodes max_abs/127.
10. **calibrate.rs tap-name reuse**: records under OLD DeltaNetScales slot names with NEW semantics. See `dnet_scales_for` for the full slot-to-tap map.

## Reference files

- `/tmp/llama.cpp/src/models/qwen35.cpp` — canonical computation graph
- `/tmp/llama.cpp/src/models/delta-net-base.cpp` — DeltaNet recurrence reference
- `/tmp/llama.cpp/ggml/src/ggml-cpu/ops.cpp:5725` — IMROPE dispatch
- `crates/ai-pow-vi/src/bin/calibrate.rs` — **validated 4/4 f32 reference**; treat as the spec
- `/tmp/scales_v3.json` — validated per-tap scales (1539 entries)
- `/tmp/qwen_eval.jsonl` — 4-prompt Ollama reference set
- GGUF blob: `~/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926`
- Latest INT8 model: `/tmp/qwen35_27b_int8_v7/` (comm_W `16e5f2a3...`)

## Estimated effort

| Step | Effort |
|---|---|
| 1. f32-store ssm_a/dt/norm + plumb | 2-3h focused (manifest format bump) |
| 2. Add ssm_conv1d weight scale | 1h |
| 3. Regenerate fixtures + un-ignore | ~half day Python |
| 4. Re-convert + re-eval | ~1h wall, 30min active |

**Honest expected outcome**: Step 1 alone likely jumps top-1 from 0/4 to 2-3/4 (since ssm_a is the largest single error). Step 2 closes the rest. Step 3 is cleanup. Step 4 is the validation.
