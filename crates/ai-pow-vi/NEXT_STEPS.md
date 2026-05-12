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

### The precise math

For a matmul `acc = a_i8 · w_i8` (i32) with rescale to i8:

- `a_real ≈ a_i8 × a_max/127`  (a_max = input activation max, captured by calibrator at the preceding tap)
- `w_real ≈ w_i8 × w_max/127`  (w_max = weight max, captured at convert time)
- `out_real = sum a_real × w_real = (a_max × w_max / 127²) × acc`
- `out_i8 = round(out_real × 127 / out_max) = round(acc × (a_max × w_max) / (127 × out_max))`

For the rescale `out_i8 = round(acc × scale.num / 2^15)` to be correct:

> **`scale.num = (a_max × w_max / (127 × out_max)) × 2^15`**

Or equivalently in Scale arithmetic where each Scale.num encodes `max/127 × 2^15`:

> **`combined.num = (a_scale.num × w_scale.num) / out_scale.num`**

The current implementation stores `scale.num = round(out_max/127 × 2^15)` — i.e., just out_max with no input/weight context. This is correct **only when** `a_max × w_max ≈ out_max²`, which never holds in practice (matmul outputs have variance proportional to sqrt(k), with k = inner dim, so out_max ≫ sqrt(a_max × w_max)).

### Option A — proper matmul-with-weight-scale (the right way)

Compute the **combined** scale at convert time and store it (single Scale per matmul output, same wire format size). The runtime forward uses it as-is — no API change to matmul_int8_requant.

Plumb through:
- For each matmul tap, identify its (input_tap, weight_tensor, output_tap) tuple
- At convert time: `combined_scale.num = (a_scale.num × w_scale.num) / out_scale.num` using the calibrator's `a_max` and `out_max`, and the converter's captured `w_max`
- Replace the matmul-output activation scale in the manifest with the combined value
- `src/bin/gguf_convert.rs`: walk every matmul, build the tuple. `dnet_scales_for`, `attn_scales_for`, `ffn_scales_for` all need to know which **input** scale precedes each matmul.

The runtime never needs to know about weight scales individually — they're baked in.

Tap-to-matmul mapping needed:
| Matmul | Input tap | Output tap | Weight |
|---|---|---|---|
| std `w_q` | `norm_post.1` | `attn.q` | attn_q.weight |
| std `w_k` | `norm_post.1` | `attn.k` | attn_k.weight |
| std `w_v` | `norm_post.1` | `attn.v` | attn_v.weight |
| std `w_o` | `attn.attn_out` | `attn.o` | attn_output.weight |
| ffn `w_gate` | `norm_post.2` | `ffn.gate` | ffn_gate.weight |
| ffn `w_up` | `norm_post.2` | `ffn.up` | ffn_up.weight |
| ffn `w_down` | `ffn.mid` | `ffn.down` | ffn_down.weight |
| hybrid `attn_qkv` | `norm_post.1` | `ssm.q` | attn_qkv.weight |
| hybrid `attn_gate` | `norm_post.1` | `ssm.proj` | attn_gate.weight |
| hybrid `ssm_alpha` | `norm_post.1` | `ssm.alpha_logit` | ssm_alpha.weight |
| hybrid `ssm_beta` | `norm_post.1` | `ssm.beta_logit` | ssm_beta.weight |
| hybrid conv1d | `ssm.q` | `ssm.u` | ssm_conv1d.weight |
| hybrid `ssm_out` | `ssm_norm_post` | `attn.o` | ssm_out.weight |

Estimated effort: 4–6h focused work + 22min reconvert + 25min eval.

### Option B — bypass via empirical recalibration

Instead of computing combined scales analytically, modify calibrate.rs to drive an **INT8 forward** (not f32). Each tap's recorded max_abs would then be the i8 output's effective max, baking in whatever convention error the forward has. Re-emit `scales.json` and re-convert.

Caveat: requires the i8 forward to be roughly working first (chicken-and-egg with the convention bug). Could iterate: run i8 → record → re-convert → run i8 → re-record. Each pass takes ~50min. May not converge cleanly.

Estimated effort: 30min to add INT8 forward to calibrate.rs + N × 50min iteration cycles.

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
