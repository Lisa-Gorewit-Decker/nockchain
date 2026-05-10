# Running ai-pow-vi tests against real Ollama-downloaded models

This guide walks through testing the ai-pow-vi crate against real GGUF
files you already have via Ollama. It covers Qwen 3.6 27B (`qwen3.6:27b`,
~17 GB, `qwen35` arch) and Gemma 4 31B (`gemma4:31b`, ~19 GB, `gemma4`
arch).

> **Status (Phase 2.15 + extensions, 2026-05-10):** the streaming
> infrastructure (pass-1 weight-scale scan, pass-2 canonical-order write,
> streaming Merkle root, streaming `comm_W`) is fully implemented and
> tested. End-to-end conversion to a `Model::load`-ready directory works
> for synthetic mini fixtures of all 5 layer flavors. Real-GGUF
> end-to-end conversion has known structural gaps documented in
> §4 below — those gaps need small Rust struct extensions to close.

## 1. Prerequisites

```sh
# 1. Locate your GGUF blobs (Ollama's content-addressed cache).
ollama show qwen3.6:27b --modelfile | grep '^FROM /'
# → FROM /Users/<you>/.ollama/models/blobs/sha256-83c54730a5fea8a0...

ollama show gemma4:31b --modelfile | grep '^FROM /'
# → FROM /Users/<you>/.ollama/models/blobs/sha256-280af6832eca23cb...

# 2. Set environment variables for the rest of this guide.
export QWEN_BLOB=/Users/$(whoami)/.ollama/models/blobs/sha256-83c54730a5fea8a0958598c01617c1419c431e93b33bacf980b49a420c798926
export GEMMA_BLOB=/Users/$(whoami)/.ollama/models/blobs/sha256-280af6832eca23cb322c4dcc65edfea98a21b8f8ab07dc7553bd6f7e6e7a3313

# 3. Create a Python virtualenv with the oracle dependencies.
#    (If /tmp/aipow-venv already exists from prior session, skip this.)
python3 -m venv /tmp/aipow-venv
/tmp/aipow-venv/bin/pip install --quiet numpy blake3 gguf

# 4. Verify the venv works.
/tmp/aipow-venv/bin/python -c "import numpy, blake3, gguf; print('ok')"
```

Recommended: run from the crate root so relative imports resolve:

```sh
cd $NOCKCHAIN_ROOT/crates/ai-pow-vi
```

## 2. Inspect a GGUF (works today, ~5 seconds)

The first sanity check: does our `gguf_reader` correctly detect the
architecture, dimensions, block kinds, and tensor names of your file?

```sh
/tmp/aipow-venv/bin/python oracle/gguf_reader.py "$QWEN_BLOB"
```

Expected output for Qwen 3.6 27B:

```
Architecture: qwen35
  num_layers: 64
  hidden:     5120
  vocab:      248320
  num_q/kv heads: 24/4
  head_dim:   256 (kv=256)
  feature_flags: 0x0191
    set: ['QK_NORM', 'POST_ATTN_NORM', 'FUSED_QKV', 'SSM_PARALLEL']
  block kinds:
    QWEN_HYBRID_SSM: 48
    QWEN_STANDARD_ATTENTION: 16
  canonical tensors: 851
```

For Gemma 4 31B:

```
Architecture: gemma4
  num_layers: 60
  hidden:     5376
  vocab:      262144
  num_q/kv heads: 32/4
  head_dim:   512 (kv=512)
  feature_flags: 0x027f
    set: ['QK_NORM', 'INP_GATE', 'LAYER_OUTPUT_SCALE', ...]
  block kinds:
    GEMMA_ATTENTION: 60
  canonical tensors: 833
```

If either model fails this step, the architecture registry needs an
update for whatever GGUF dialect Ollama shipped. File a bug with the
exact GGUF metadata.

## 3. Memory-bounded streaming pass (works today, ~90 seconds for 17 GB)

This confirms our streaming pass-1 actually walks the full file without
OOMing. It reads every tensor, dequantizes one at a time, computes the
per-tensor weight scale, and drops the array.

```sh
time /tmp/aipow-venv/bin/python -c "
import sys
sys.path.insert(0, 'oracle')
import gguf_reader as G
import quantize_streaming as QS
stream = G.open_stream('$QWEN_BLOB')
print(f'arch={stream.arch.name} layers={stream.arch.num_layers} '
      f'hidden={stream.arch.hidden} vocab={stream.arch.vocab_size}')
weight_scales = QS.compute_weight_scales(stream)
print(f'computed scales for {len(weight_scales)} tensors')
"
```

Expected: ~1.5 minutes wall-clock for Qwen 3.6 27B (851 tensors,
17 GB), ~2 minutes for Gemma 4 31B (833 tensors, 19 GB).

Memory: the *heap* peak is ~2 GB (one tensor at a time). The *RSS*
reported by the OS may be 15-20 GB because the GGUF is mmapped — those
pages can be evicted under memory pressure, so this still runs on a
16 GB laptop.

## 4. End-to-end conversion: status by model

### Gemma 4 31B (most likely to work; some structural mismatches)

```sh
/tmp/aipow-venv/bin/python oracle/quantize_streaming.py \
    --gguf "$GEMMA_BLOB" \
    --out  /tmp/gemma4_31b_int8 \
    --seq-len 4096 \
    --activation-tile 64
```

The streaming converter handles all five layer flavors including
`GemmaLayer`. For Gemma 4 31B the converter will:

1. Walk all 60 blocks in canonical order and emit `weights.bin`.
2. Hash bytes incrementally into the streaming Merkle.
3. Write `manifest.bin` and `comm_w.hex`.

**Known gaps.** Gemma 4 31B's GGUF carries two per-block tensors that
our Rust `LayerWeights::Gemma` variant does *not* model (`post_norm.gamma`
and `proj`, used by the per-layer-embedding path). The streaming
converter ignores them — the produced model loads, but `forward_prefix`
output won't match a Gemma 4 reference forward bit-for-bit. Top-1
agreement should still be reasonable for short prompts.

Additionally: the GGUF stores `inp_gate` as a `(hidden, 256)` matrix,
while our Rust variant expects a `hidden`-length vector. The
streaming converter currently writes the matrix bytes raw (incorrect
shape) — the loaded model will fail at `Model::load`'s `BadInpGateLen`
check. Fix: extend `LayerWeights::Gemma::inp_gate` to a matrix
parameter, or compute a per-channel reduction at convert time. This
is a 1-day Rust struct extension; not done in this session.

### Qwen 3.6 27B (blocked on Rust struct extensions)

```sh
/tmp/aipow-venv/bin/python oracle/quantize_streaming.py \
    --gguf "$QWEN_BLOB" \
    --out  /tmp/qwen35_27b_int8 \
    --seq-len 4096 \
    --activation-tile 64
```

This will currently **fail at the first hybrid block** with:

```
KeyError: qwen35 hybrid block 0 is missing `attn.w_o` (attention output
projection). Extend oracle/arch/qwen35.py per_block_overrides to map
the GGUF's actual output-projection tensor name.
```

**Why it fails.** Our `LayerWeights::QwenHybridSsm` Rust variant has
*two* output projections: `attn_out` (after gated attention) and
`ssm_out` (after the SSM path), summed before the residual add. The
real Qwen 3.6 27B has *one* shared projection (the GGUF's `ssm_out`)
that operates on the *summed* attention+SSM per-head output. There is
no separate `attn_out` tensor in the file.

Two related mismatches:
1. **Different head_dim for attention vs SSM**: real model uses
   `attn_head_dim=256, ssm_head_dim=128, num_v_heads=48` (so
   `num_v_heads * ssm_head_dim = 6144 = num_q_heads * attn_head_dim`).
   Our Rust struct has a single `head_dim` field shared between the
   two paths.
2. **Different head_dim_kv**: real `attn.w_qkv` is `(5120, 10240)`
   which decomposes as `q=24×256 + k=4×512 + v=4×512 = 6144 + 2048 +
   2048`. Our Rust struct assumes `head_dim_q == head_dim_kv == head_dim`.

**To unblock Qwen 3.6 27B end-to-end:**

- Add `attn_head_dim_kv: u32`, `ssm_head_dim: u32`, `num_v_heads: u32`
  fields to `LayerWeights::QwenHybridSsm`. Update `forward_qwen_hybrid_ssm_layer`,
  `gated_attention_forward`, and `ssm_forward` to use the right field
  per code path. (≈ 200 lines of Rust.)
- Drop the `attn_out` field; replace with a single `out_proj` shared
  by both paths, mirroring the real architecture. Adjust
  `forward_qwen_hybrid_ssm_layer` to sum `y_attn + y_ssm` *at per-head
  dim* and project once. (Another ≈ 100 lines.)
- Extend `oracle/arch/qwen35.py::per_block_overrides` to map any
  remaining tensor names.
- Refresh the `pin_ssm_canonical_output` and any qwen-hybrid-mini pins
  that change byte output.

## 5. Confirm Rust can load the conversion (works for archs that converted cleanly)

If you got past §4 for Gemma 4 31B (or once Qwen 3.6 27B's struct
extensions land):

```sh
# The output directory carries comm_w.hex; Model::load will recompute
# comm_W and reject any tampering.
NOCKCHAIN_VI_QWEN_DIR=/tmp/qwen35_27b_int8 \
    cargo test -p ai-pow-vi --test oracle_qwen -- --ignored --nocapture
```

The gated `oracle_qwen.rs` test in `tests/` runs `Model::load(dir,
&recorded_comm_w)` against `$NOCKCHAIN_VI_QWEN_DIR`. Success means the
streaming converter's bytes round-trip through the Rust loader.

## 6. Run vi-eval against the loaded model

```sh
# A tiny eval set (one prompt per line, JSON-ish).
cat > /tmp/eval.jsonl <<'EOF'
{"prompt": [1, 2, 3, 4, 5]}
{"prompt": [9, 17, 42, 100, 200, 300]}
EOF

cargo run --release -p ai-pow-vi --bin vi-eval -- \
    --model-dir /tmp/qwen35_27b_int8 \
    --eval     /tmp/eval.jsonl \
    --arch     qwen35
```

`vi-eval`:

- Loads the model (verifying `comm_W`).
- Runs `forward_prefix` to the final layer + final norm on each prompt.
- If `lm_head.bin` is present (next to `weights.bin`), applies it,
  takes `argmax`, and reports top-1 next-token agreement against the
  `expected_top1` field if you supply one.
- Without `lm_head.bin`, it still runs the full forward and reports
  prompts processed.

## 7. Top-1 vs Ollama (Phase 2.14 acceptance gate, requires §4 fixes)

Once Qwen 3.6 27B converts cleanly, the empirical accuracy gate is:

```sh
# Generate ~50-100 reference next-token predictions from Ollama:
for prompt in $(cat eval_prompts.txt); do
    echo -n "$prompt"
    ollama run qwen3.6:27b --verbose <<<"$prompt" 2>/dev/null | head -1
done > /tmp/ollama_refs.txt

# Compare against vi-eval's top-1.
# Acceptance: ≥ 90% top-1 agreement for Qwen 3.6 27B (§2.14).
```

If top-1 agreement is below 90%, the activation scales need
calibration (use `oracle/calibrate.py`, then re-run §4 with `--scales
scales.json`).

## What works today (without code changes)

- **§2 inspection**: works for both Qwen 3.6 27B and Gemma 4 31B.
- **§3 streaming pass-1**: works for both (memory-bounded).
- **§4 streaming conversion**: works for Gemma 4 31B with caveats
  (post_norm/proj ignored; inp_gate shape mismatch causes
  `Model::load` to fail). Works for Qwen 3.6 27B's *standard*
  attention blocks; the converter errors out at the first hybrid
  block.
- **§5/§6/§7**: blocked on the Rust struct extensions described in §4
  for real models. Synthetic mini fixtures (`qwen_mini`,
  `qwen_hybrid_mini`, `gemma_mini`) already pass these stages.

## TL;DR — fastest "is this real?" test today

```sh
cd $NOCKCHAIN_ROOT/crates/ai-pow-vi
export QWEN_BLOB=$(ollama show qwen3.6:27b --modelfile | grep '^FROM /' | awk '{print $2}')

# Full streaming scan of the real 17 GB GGUF in ~90 seconds.
/tmp/aipow-venv/bin/python -c "
import sys; sys.path.insert(0, 'oracle')
import gguf_reader as G
import quantize_streaming as QS
stream = G.open_stream('$QWEN_BLOB')
ws = QS.compute_weight_scales(stream)
print(f'OK: {stream.arch.name} {stream.arch.num_layers}L {stream.arch.hidden}d, '
      f'{len(ws)} tensors scaled, no OOM.')
"
```

That run validates: GGUF parsing, arch detection, block-kind
classification, every tensor's dequantization path, and that the
streaming pipeline is genuinely memory-bounded. It's the highest-
signal test you can run on a real Qwen 3.6 27B file without further
Rust struct work.
