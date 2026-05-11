# Qwen 3.5 27B INT8 — remaining work

End-goal: `gguf-convert` produces a Qwen 3.5 27B model dir whose `vi-eval` agrees with Ollama 4/4 on `/tmp/qwen_eval.jsonl` (predicting `8160, 8160, 90700, 8160`).

The **f32 reference path** already gets 4/4 at commit `e689d78`. The **INT8 path** runs end-to-end without panics but is at **0/4** with quantization-noise-driven predictions (logit magnitudes 55–72k, predictions like `145890, 100, 219132, 15277`).

## State at session end (branch `claude/ai-pow-nockchain-sgfNX`, ahead of origin)

| Commit | Done |
|---|---|
| `2d01809` | Phase B.2 — packed Q+gate for std blocks |
| `8b137fa` | Phase A (initial) — f32 calibrator scaffold |
| `19f8105` | Real arch in calibrate.rs (IMROPE + GatedDeltaNet + per-layer num_kv) |
| `e689d78` | K→V broadcast fix. **4/4 top-1 vs Ollama** in f32. |
| `6c55fd6` | INT8 scaffolding — rope IMROPE primitives + deltanet forward |
| `5f37d88` | Wire `forward_gated_deltanet_qwen35` into runtime; converter rewrite |
| `b44246b` | Loader byte counts for DeltaNet + IMROPE table sizing fix |
| `3fd5af8` | Scale tap-name routing fix |

All lib + integration tests pass (10 tests `#[ignore]`'d pending the qwen_hybrid_mini fixture regeneration).

Validated scales on disk: `/tmp/scales_v3.json` — emits OLD slot names with NEW semantics (see calibrate.rs:842-1037 for the exact tap-to-record mapping).

## What's still wrong

INT8 predictions are non-trivial (large logits, vocab-typical token ids) but don't match Ollama's argmax. Three suspects in priority order:

1. **`forward_gated_deltanet_qwen35` (src/deltanet.rs:514, ~300 lines)** — agent-written port of the f32 reference, never validated end-to-end. The most likely place for residual bugs:
   - Sign / rounding convention in the recurrence rank-1 update
   - L2-norm formula (`1/max(sqrt(sumsq), eps)` vs `1/sqrt(sumsq+eps)`)
   - Q-scaling `1/sqrt(head_k_dim)` applied at the wrong point
   - K→V broadcast direction (`vh % num_k` not `vh / kv_groups`) inside the i8 forward — fixed in f32 calibrate.rs at `e689d78`; double-check this same fix is present in deltanet.rs
2. **i8 dequantization of f32-tracked state inside the recurrence** — the agent chose to keep state in f32 between tokens but the i8 boundaries on each side (q, k, v as i8 → f32 → recurrence → f32 → i8 back out) accumulate rounding noise across 64 tokens. May need wider intermediate state or stronger per-tap scales.
3. **Conv1d output channel split offsets** — the conv1d weight is laid out as `[kernel_outer, channels_inner]` after transpose; verify the channel-split offsets `[0..key_dim) → Q, [key_dim..2*key_dim) → K, [2*key_dim..conv_dim) → V` match the converter's output exactly.

## Step-by-step debugging recipe (next session)

### Step A — directly compare i8 vs f32 layer outputs

Write a tiny test fixture that:
1. Loads the v7 model dir (`/tmp/qwen35_27b_int8_v7`)
2. Loads the same Qwen 3.5 27B GGUF
3. Runs the first prompt through *one* hybrid layer using both:
   - Our INT8 `forward_qwen_hybrid_ssm_layer` (residual → norm1 → DeltaNet → res1 → norm2 → FFN → output)
   - The f32 reference's `forward_hybrid_layer` in calibrate.rs
4. Dequantizes the i8 output to f32 (using the layer's `out` scale)
5. Computes per-element relative error vs f32

Run it for hybrid layers 0, 1, 2 (the first three before the std-layer-3 sees them). Wherever relative error first exceeds ~10%, the bug lives in that step.

### Step B — drop-in diff one DeltaNet step at a time

Inside `forward_gated_deltanet_qwen35`, after each major operation:
- attn_qkv matmul
- attn_gate matmul
- conv1d + SiLU
- channel split + L2-norm
- α / β / gate_log
- recurrence step
- gated RMSNorm
- output projection

…dequantize the i8 output to f32 and compare against calibrate.rs's recorded value at that point. The first divergence narrows the bug to one operation. The calibrate.rs file already records max(|x|) per tap which can serve as the sanity oracle.

### Step C — try f32-state-throughout

If quantization noise is the dominant issue, modify `forward_gated_deltanet_qwen35` to keep:
- q, k, v as f32 (not i8)
- recurrence state as f32 (already done)
- everything inside the conv1d and recurrence in f32
- quantize only at the block boundaries (input from prev layer, output to next layer)

This loses the determinism guarantees but tells you whether the i8 path can match Ollama in principle (it should, since f32 reference does). If even this doesn't match, the bug is structural (not quantization).

### Step D — write a forward path test against the v7 fixture

Save the f32 reference's per-layer hidden state from `bin/calibrate.rs` to disk (add a `--dump-layer-outs <dir>` flag — ~30 lines of code). Then write a Rust integration test:

```rust
let model = Model::load("/tmp/qwen35_27b_int8_v7", &expected_comm_w).unwrap();
for layer_idx in 0..64 {
    let actual = forward_through_layer_N(&model, &prompt, layer_idx);
    let expected = read_f32_dump(format!("layer_{layer_idx}.f32"));
    let max_relerr = compare_dequant(&actual, &expected, model.layers[layer_idx].out_scale);
    assert!(max_relerr < 0.20, "layer {layer_idx} max relerr = {max_relerr}");
}
```

This nails down which layer first diverges.

## Step E — regenerate fixtures + clean up

Once Step A-D narrow the bug and INT8 matches Ollama:

1. Rewrite `oracle/synthetic_qwen_hybrid_mini.py` to emit the new DeltaNet arithmetic. Update Mamba-era shapes (num_q_heads/head_dim slots now hold DeltaNet num_k_heads/head_k).
2. Regenerate `oracle/test_vectors/qwen_hybrid_mini/{manifest.bin,weights.bin,comm_w.hex,forward_layer_*.bin}`.
3. Un-`#[ignore]` the 3 fixture tests + the one in oracle_multi_arch's FIXTURES.
4. Verify all 246+ tests pass.

## Key invariants (don't relearn these)

1. **GGUF tensor name `ssm_dt`** has no `.bias` suffix in the Ollama-shipped GGUF. Try both with `or_else`.
2. **`ssm_a`** is stored as `-exp(A_log)` (already negated); use as a multiplier.
3. **Conv1d weight layout**: candle returns `[channels, kernel]` PyTorch-style with memory `w[c*kk + k]`. Runtime expects kernel-outer `w[k*conv_dim + c]` — transpose ONCE in converter.
4. **K→V broadcast**: `vh % num_k` (ggml_repeat tiles), NOT `vh / kv_groups`.
5. **IMROPE tables**: sized `seq_len * (n_rot/2)`, so `tables.half_head_dim = n_rot/2`. Rope_apply slice length = `2 * tables.half_head_dim` covers exactly the rotated subspace.
6. **Per-head L2-norm**: `1/max(sqrt(sumsq), eps)`, not `1/sqrt(sumsq+eps)`.
7. **Q scaling**: `q *= 1/sqrt(head_k_dim)` applied ONCE after L2-norm, before the recurrence.
8. **DeltaNet recurrence state**: keep in f32; not viable in i8 due to 64-step multiplicative decay.
9. **calibrate.rs tap-name reuse**: records under OLD DeltaNetScales slot names with NEW semantics. See `dnet_scales_for` for the full slot-to-tap map.

## Reference files

- `/tmp/llama.cpp/src/models/qwen35.cpp` — canonical computation graph
- `/tmp/llama.cpp/src/models/delta-net-base.cpp` — DeltaNet recurrence reference
- `/tmp/llama.cpp/ggml/src/ggml-cpu/ops.cpp:5725` — IMROPE dispatch
- `/tmp/llama.cpp/ggml/src/ggml-cpu/ops.cpp:1693` — `ggml_repeat_f32` (proves tile, not interleave)
- `crates/ai-pow-vi/src/bin/calibrate.rs` — **validated 4/4 f32 reference**; treat as the spec
- `/tmp/scales_v3.json` — validated per-tap scales (1539 entries)
- `/tmp/qwen_eval.jsonl` — 4-prompt Ollama reference set
- GGUF blob: `~/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926`
- Latest converted INT8 model: `/tmp/qwen35_27b_int8_v7/` (comm_W `16e5f2a3...`)

## Estimated effort

| Step | Effort |
|---|---|
| A. Per-layer i8 vs f32 diff | 2–3h focused |
| B. DeltaNet operation-by-operation diff | 1–2h |
| C. f32-throughout sanity check | 30min |
| D. Integration test on the v7 fixture | 1h |
| E. Fixture rewrite | ~half day |

**Honest expected outcome**: B is most likely to surface a concrete bug in `forward_gated_deltanet_qwen35`. Once that's fixed, top-1 should jump to 3/4 or 4/4 (since the f32 reference path is provably correct with the same scales).
