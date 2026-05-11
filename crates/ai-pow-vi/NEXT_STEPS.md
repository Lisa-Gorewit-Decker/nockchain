# Qwen 3.5 27B INT8 — remaining work

End-goal: `gguf-convert` produces a Qwen 3.5 27B model dir whose `vi-eval` agrees with Ollama 4/4 on `/tmp/qwen_eval.jsonl` (predicting `8160, 8160, 90700, 8160`).

The f32 reference path already gets 4/4 at commit `e689d78`, so this is mechanical translation — not arch exploration.

## State at session end (branch `claude/ai-pow-nockchain-sgfNX`, 5 ahead of origin)

| Commit | Done |
|---|---|
| `2d01809` | Phase B.2 — packed Q+gate for std blocks |
| `8b137fa` | Phase A (initial) — f32 calibrator scaffold, wrong arch |
| `19f8105` | Real arch in calibrate.rs (IMROPE + GatedDeltaNet + per-layer num_kv + `--verify-top1`) |
| `e689d78` | K→V broadcast fix (`vh / kv_groups` → `vh % num_k`). **4/4 top-1 vs Ollama.** |
| `6c55fd6` | INT8 scaffolding — IMROPE in `src/rope.rs`, `forward_gated_deltanet_qwen35` in `src/deltanet.rs`, IMROPE table build in `gguf-convert`. **Not wired into runtime.** |

All 246 lib + integration tests still pass.

Validated scales already on disk: `/tmp/scales_v3.json` — 1539 taps, computed from the 4/4 f32 reference.

## What's still wrong

`forward_qwen_hybrid_ssm_layer` in `src/layer.rs` still calls the legacy `gated_attention_forward` + `ssm_forward` path (the Mamba interpretation). `gguf-convert::build_qwen_hybrid_layer` still loads tensors with std-block dims. So the INT8 output is architecturally wrong on the 48 hybrid blocks, producing 0/4 against Ollama.

The new `forward_gated_deltanet_qwen35` exists and is correct (it's a step-for-step INT8 port of the validated f32 reference) — but nothing calls it yet, and its `GatedDeltaNetOpts` struct expects a field set that doesn't match what `QwenHybridSsm` currently exposes.

## Step 1 — wire `forward_gated_deltanet_qwen35` into the runtime

**Files:** `src/layer.rs`.

`forward_qwen_hybrid_ssm_layer` (currently ~200 lines around L749) does:

1. RMSNorm (`norm1`)
2. Legacy attention + SSM scan path (this is what's wrong)
3. Residual1
4. RMSNorm (`norm2`)
5. FFN
6. Residual2

Replace step 2 with a single call to `crate::deltanet::forward_gated_deltanet_qwen35` after constructing a `GatedDeltaNetOpts` from the existing struct fields.

Field map (`QwenHybridSsm` → `GatedDeltaNetOpts`):

| QwenHybridSsm field | Opts field | Notes |
|---|---|---|
| `attn_qkv_fused` | `w_qkv` | direct |
| `attn_gate` | `w_gate` | direct |
| `ssm_alpha` | `w_alpha` | direct |
| `ssm_beta` | `w_beta` | direct |
| `ssm_conv1d` | `w_conv1d` | must already be **kernel-major** (k-outer, c-inner). Currently the converter transposes it; keep that. |
| `ssm_dt` | `ssm_dt` | direct |
| `ssm_a` | `ssm_a` | direct |
| `ssm_norm_gamma` | `ssm_norm_gamma` | direct |
| `ssm_norm_eps_q` | `ssm_norm_eps_q` | direct |
| `ssm_norm_post_scale` | `ssm_norm_post_scale` | direct |
| `ssm_out` | `w_out` | direct |
| `num_q_heads` (repurposed) | `num_k_heads` | semantic shift — see Step 2 |
| `num_v_heads` | `num_v_heads` | direct (= 48) |
| `head_dim` (repurposed) | `head_k` | semantic shift — see Step 2 |
| `ssm_head_dim` | `head_v` | direct (= 128) |
| `ssm_kernel_size` | `conv_kernel` | direct (= 4) |
| `ssm_scales` | adapted → `scales: GatedDeltaNetScales` | see below |

LUTs (from `LayerContext`):

| Opts field | Source |
|---|---|
| `sigmoid_lut` | `ctx.sigmoid_lut` |
| `silu_lut` | `ctx.ffn_activation` (it's a SiLU LUT) |

**Scales adapter.** `DeltaNetScales` (10 fields, old Mamba tap names) → `GatedDeltaNetScales` (10 fields, DeltaNet tap names). Build inline at the call site:

```rust
let scales = GatedDeltaNetScales {
    qkv:        ssm_scales.q,
    gate_z:     ssm_scales.v,
    conv_silu:  ssm_scales.u,
    q_norm:     ssm_scales.q,  // not separately tracked in old format
    k_norm:     ssm_scales.k,
    alpha:      ssm_scales.alpha_logit,
    beta:       ssm_scales.beta_logit,
    recurrence: ssm_scales.decay,
    gated_norm: ssm_scales.o,
    out:        ssm_scales.proj,
};
```

Drop `attn_out`, `attn_scales`, `q_norm_gamma`, `k_norm_gamma`, `qk_norm_eps_q`, `qk_norm_post_scale` from the destructure — they're no longer used in this code path. The struct still has them (existing manifest format), they just go unread.

Delete the `gated_attention_inter` / `gated_attention_forward` / `ssm_forward_inter` / `ssm_forward` calls — those become dead code (keep the function definitions for now since they're public API).

## Step 2 — fix the converter

**Files:** `src/bin/gguf_convert.rs`.

`build_qwen_hybrid_layer` (~L735) currently splits `attn_qkv` using std-block dims (`num_q_heads=24, num_kv_heads=auto, head_dim=256`) and reads tensors with Mamba semantics. Replace with DeltaNet:

```rust
let n_k_heads = meta_u32(content, "qwen35.ssm.group_count", Some(16))? as usize;
let n_v_heads = meta_u32(content, "qwen35.ssm.time_step_rank", Some(48))? as usize;
let head_k    = meta_u32(content, "qwen35.ssm.state_size", Some(128))? as usize;
let head_v    = head_k;  // same in qwen35
let conv_K    = meta_u32(content, "qwen35.ssm.conv_kernel", Some(4))? as usize;
let key_dim   = n_k_heads * head_k;
let value_dim = n_v_heads * head_v;
let conv_dim  = 2 * key_dim + value_dim;  // 10240 on qwen3.5-27B
```

Then:

1. **attn_qkv** — read `blk.N.attn_qkv.weight`, expect candle shape `[conv_dim=10240, hidden=5120]`. Direct dequant-quantize.
2. **attn_gate** — read `blk.N.attn_gate.weight`, expect `[value_dim=6144, hidden=5120]`.
3. **ssm_conv1d** — read `blk.N.ssm_conv1d.weight`, expect candle shape `[conv_dim=10240, kernel=4]`. **Transpose** to kernel-outer (`w_new[k*conv_dim + c] = w_raw[c*kk + k]`) before quantizing.
4. **ssm_dt** — try `blk.N.ssm_dt` first; fall back to `blk.N.ssm_dt.bias`. Length `n_v_heads=48`.
5. **ssm_a** — read `blk.N.ssm_a`. Length 48. **Important**: this is stored as `-exp(A_log)` (already negated). The forward uses it as a multiplier — do not re-negate.
6. **ssm_alpha** — `blk.N.ssm_alpha.weight`, candle shape `[n_v_heads=48, hidden=5120]`.
7. **ssm_beta** — `blk.N.ssm_beta.weight`, same shape.
8. **ssm_norm** — `blk.N.ssm_norm.weight`, length `head_v=128`.
9. **ssm_out** — `blk.N.ssm_out.weight`, candle shape `[hidden=5120, value_dim=6144]`.

Set the QwenHybridSsm struct fields:

```rust
num_q_heads:   n_k_heads as u32,   // repurposed
num_kv_heads:  n_k_heads as u32,   // unused, mirror for clarity
head_dim:      head_k as u32,      // repurposed
num_v_heads:   n_v_heads as u32,
ssm_head_dim:  head_v as u32,
ssm_kernel_size: conv_K as u32,
```

**Scales** — replace `dnet_scales_for` to look up new tap names:

```rust
fn dnet_scales_for(scales: &ScaleSource, n: u32) -> DeltaNetScales {
    let tap = |sub: &str| format!("layer[{n}].ssm.{sub}");
    DeltaNetScales {
        q:           Scale::from_num(scales.get(&tap("qkv"))).unwrap(),
        k:           Scale::from_num(scales.get(&tap("k_norm"))).unwrap(),
        v:           Scale::from_num(scales.get(&tap("gate_z"))).unwrap(),
        alpha_logit: Scale::from_num(scales.get(&tap("alpha"))).unwrap(),
        beta_logit:  Scale::from_num(scales.get(&tap("beta"))).unwrap(),
        u:           Scale::from_num(scales.get(&tap("conv_silu"))).unwrap(),
        decay:       Scale::from_num(scales.get(&tap("recurrence"))).unwrap(),
        update:      Scale::from_num(scales.get(&tap("recurrence"))).unwrap(),  // share with decay
        o:           Scale::from_num(scales.get(&tap("gated_norm"))).unwrap(),
        proj:        Scale::from_num(scales.get(&tap("out"))).unwrap(),
    }
}
```

The "unused" `attn_qkv`-related fields (`q_norm_gamma`, `k_norm_gamma`, etc) still need to be populated to satisfy the struct; use `default_no_op_gamma_i8` and `Scale::from_num(1).unwrap()` defaults. They'll be ignored at forward time.

## Step 3 — fixture: rewrite or `#[ignore]`

`oracle/synthetic_qwen_hybrid_mini.py` generates a synthetic small-scale model whose manifest + weights + forward output are byte-equal to the Rust forward. The test is `tests/oracle_qwen_hybrid_mini.rs`.

Two paths:

**Path A (clean):** rewrite the Python generator's hybrid-block forward to mirror the new DeltaNet arithmetic, regenerate the fixture, byte-equal test passes again. Estimated ~half day of Python work.

**Path B (fast):** mark `tests/oracle_qwen_hybrid_mini.rs` `#[ignore]` with a TODO note pointing at this plan. Re-enable when Path A lands. Estimated 5 minutes.

Recommend Path B first to unblock the real-Qwen re-eval, Path A as a follow-up.

## Step 4 — refresh pins

`tests/determinism_pins.rs::pin_comm_w_canonical_model` builds a tiny model with one `LayerWeights::Attention` (not Hybrid), so it should NOT be affected.

`oracle/test_vectors/qwen_hybrid_mini/comm_w.hex` will diverge (the fixture's weights + forward change). Regenerate after Path A or delete if Path B.

## Step 5 — build + test

```sh
cargo build --release -p ai-pow-vi --features gguf-convert --bin gguf-convert --bin vi-eval --bin calibrate
cargo test -p ai-pow-vi --features gguf-convert 2>&1 | grep "test result"
```

Expect all current tests to pass (modulo the ignored `oracle_qwen_hybrid_mini` if Path B). If a test fails, the most likely cause is a tensor-shape mismatch in `build_qwen_hybrid_layer` — re-check the candle shape orientation against the GGUF native order (see the conv1d transpose dance in calibrate.rs at line 806).

## Step 6 — re-convert + re-eval

```sh
# scales already on disk from session: /tmp/scales_v3.json
mkdir -p /tmp/qwen35_27b_int8_v4
./target/release/gguf-convert \
  --gguf "$HOME/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926" \
  --out /tmp/qwen35_27b_int8_v4 \
  --seq-len 64 --activation-tile 64 \
  --scales /tmp/scales_v3.json

./target/release/vi-eval \
  --model-dir /tmp/qwen35_27b_int8_v4 \
  --eval /tmp/qwen_eval.jsonl \
  --arch qwen35
```

Wall clock budget: convert ~25 min, vi-eval ~25 min (25 GB BLAKE3 + 4 forwards). Expected `top1_agreement\t4/4\t100.0%`.

If less than 4/4, the most likely failure mode is INT8 quantization noise on a calibration scale that's too tight. Re-run calibrate with a wider prompt set (16–32 prompts) and try again. If 4/4, refresh the qwen_hybrid_mini fixture (Path A) as the follow-up.

## Key invariants (read before touching the code)

1. **Conv1d weight layout.** GGUF native is `[kernel, channels]` (innermost-fastest). Candle returns the dims as `[channels, kernel]` PyTorch-style **but the memory bytes are in PyTorch order**, i.e. `w_raw[c*kk + k]`. The runtime forward expects `w[k*conv_dim + c]`. Transpose ONCE in the converter; do not transpose at the forward.
2. **`ssm_a` sign.** The GGUF stores `-exp(A_log)` directly. Use it as a multiplier; do not negate.
3. **`ssm_dt` name.** The Ollama-shipped GGUF uses bare `ssm_dt` (no `.bias`). Newer llama.cpp builds emit `ssm_dt.bias`. Try both with `or_else`.
4. **K→V broadcast.** `ggml_repeat` tiles (`dst[i*ne + k] = src[k]`), so v-head `vh` reads k-head `vh % num_k`. NOT `vh / kv_groups` — that's repeat-interleave semantics and gives the wrong head assignments (this is the bug that took the f32 reference from 0/4 to 4/4).
5. **IMROPE.** Rotates only the first `n_rot=64` of each 256-element head. NEOX pairing `(x[j], x[j+n_rot/2])`. Per-pair section dispatch matches `ggml-cpu/ops.cpp:5725`: `sector%3==0 && sector<3*sec[0]` → t, `%3==1 && <3*sec[1]` → h, `%3==2 && <3*sec[2]` → w.
6. **DeltaNet recurrence state.** Keep in f32 inside the forward (not i8). The multiplicative decay over 64 tokens destroys i8 precision; the i8 quantization happens only at the per-token boundaries (`recurrence` scale).
7. **Q scaling.** `q *= 1/sqrt(head_k_dim)` applied **once** after L2-norm, before the recurrence. Easy to forget.
8. **Per-head L2-norm formula.** `1 / max(sqrt(sumsq), eps)`, not `1 / sqrt(sumsq + eps)`. Matches `ggml_l2_norm` exactly.

## Reference files

- `/tmp/llama.cpp/src/models/qwen35.cpp` — canonical graph (llama.cpp commit `1ec7ba0`)
- `/tmp/llama.cpp/src/models/delta-net-base.cpp` — DeltaNet recurrence
- `/tmp/llama.cpp/ggml/src/ggml-cpu/ops.cpp:5725` — IMROPE dispatch
- `/tmp/llama.cpp/ggml/src/ggml-cpu/ops.cpp:1693` — `ggml_compute_forward_repeat_f32` (proves tile, not interleave)
- `crates/ai-pow-vi/src/bin/calibrate.rs` — validated 4/4 f32 reference; treat as the spec
- `/tmp/scales_v3.json` — validated per-tap scales from the 4/4 reference
- `/tmp/qwen_eval.jsonl` — 4-prompt Ollama eval set
- GGUF blob: `~/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926`

## Estimated effort

| Step | Effort |
|---|---|
| 1. Wire forward | ~1h |
| 2. Fix converter | ~1.5h |
| 3. Fixture Path B (ignore) | ~5min |
| 4. Pins | ~5min |
| 5. Build + test | ~15min |
| 6. Convert + eval | ~50min wall, ~15min active |
| **Subtotal to 4/4** | **~3–4h focused work + ~1h wall-clock** |
| 3a. Fixture Path A (rewrite) | follow-up, ~half day |
