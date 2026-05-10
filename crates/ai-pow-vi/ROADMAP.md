# ai-pow-vi roadmap

Working notes on the remaining work to land the verifiable-inference proof
of work for Nockchain. This is the implementation-level companion to the
high-level plan at `~/.claude/plans/read-these-two-papers-rosy-snail.md`.
Each item is sized for a single commit; subsequent commits stack and the
crate's determinism pins grow with each addition.

## Status snapshot

Branch: `claude/ai-pow-nockchain-sgfNX`. Latest commit: `9d87f6a`.

Shipped so far:

| Crate / Module | Phase | Commit | What it gives |
|---|---|---|---|
| `crates/ai-pow/` (full) | 1 + 1.5 | `5eac594` | BLAKE3 transcript, tile-Merkle (sentinel padding), INT8 tile dot, FS, named LLM-FFN profiles, `mine` / `mine_block`. |
| `ai-pow-vi/quant`, `determinism`, `activation_lut`, `rmsnorm`, `layout` | 2 | `e1e0e17` | Quantization contract + first ops + cross-arch pins. |
| `ai-pow-vi/softmax`, `rope` | 2 | `efcde4b` | Integer softmax with 256-entry exp LUT; INT16 RoPE tables. |
| `ai-pow-vi/matmul_int8`, `layernorm` | 2 | `a82223a` | Inference-side INT8 matmul; integer LayerNorm. |
| `ai-pow-vi/ffn` | 2 | `330b294` | SwiGLU forward block. |
| `ai-pow-vi/attention` | 2 | `73cf097` | Standard + GQA attention: Q/K/V projection, RoPE, causal softmax, V-weighted sum, output projection. |
| `ai-pow-vi/deltanet` | 2 | `52a6f77` | Gated DeltaNet linear-attention recurrence: per-token state matrix update with sigmoid α/β gates, GQA V→QK head mapping. |
| `ai-pow-vi/activations` | 2 | `f3eafcd` | Per-layer activation tile-Merkle log: BLAKE3 leaves, root, sibling-path opening + verification. Wraps `ai-pow::commit`. |
| `ai-pow-vi/layer` | 2 | `cbb543b` | Per-layer composition: `Norm → (Attention\|DeltaNet) → +residual → Norm → FFN → +residual`, with RMSNorm/LayerNorm flavors and shared `LayerContext`. |
| `ai-pow-vi/forward` + `model` | 2 | `7f24cc4` | Forward-pass driver: embed → run layers 0..target_layer → optional final norm; records each per-layer activation into an `ActivationLog`. Minimal `Model` struct (Phase 2.7 extends with comm_W). |
| `ai-pow-vi/prompt` | 2 | `0f834d4` | BLAKE3-XOF Fiat-Shamir prompt synthesis: deterministic `(block_commitment, model_id) → Vec<Token>` with reserved-token rejection. |
| `ai-pow-vi/comm_w` | 2 | `03ecf1b` | Canonical model commitment: weight tile-Merkle root + manifest hash → 32-byte `comm_W`. Sensitive to every weight, scale, eps, LUT byte, and architecture choice. |
| `ai-pow-vi/proof` + `prover` + `verifier` | 3 | `e1d1e1a` | `ViProof` wire format, `mine_vi` prover, and `verify_vi` (FullReplica mode). Composes synth_prompt → forward_prefix → FFN gate tile-Merkle → FS challenge → tile hardness check → σ spot-checks. |
| `ai-pow-vi/oracle/` | 2.8 | `9d87f6a` | Numpy/blake3 reference implementation + 7 binary fixtures (rescale, matmul, rmsnorm, layernorm, softmax, ffn, synth_prompt). Rust `tests/oracle_op_vectors.rs` loads each fixture and asserts byte-equal Rust output. Cross-implementation determinism check on top of the Rust-only pins. |
| `ai-pow-vi/io` + qwen-mini | 2.9.1, 2.9.5, 2.9.8 | `0225f87` | Disk format (`manifest.bin` / `weights.bin` / `comm_w.hex`), `Model::load`/`save`, numpy forward reference, synthetic Qwen-mini end-to-end Rust test. |
| `oracle/gguf_reader.py`, `calibrate.py`, `quantize_qwen.py`, `bin/qwen-eval`, `tests/oracle_qwen.rs` | 2.9.2-2.9.4, 2.9.6-2.9.7 | latest | GGUF dequantize + canonical-name mapping; static + activation-mode scale calibration; quantizer driver that produces a Model directory; gated real-model integration test; `qwen-eval` binary for top-1 next-token agreement. |

Test count: 184 unit + 17 pins + 7 oracle cross-impl + 4 qwen-mini E2E + 3 quantized-synthetic E2E + 1 gated real-model, all green on aarch64.

## Phase 2 — remaining (in dependency order)

### ~~2.1 Attention forward (`src/attention.rs`)~~ ✓ shipped

The single largest remaining piece. Composes `matmul_int8` + `rope` +
`softmax` into the standard transformer attention.

**Public surface:**

```rust
pub struct AttentionWeights {
    pub hidden: u32,
    pub num_q_heads: u32,
    pub num_kv_heads: u32,    // Gemma: 16; Qwen: 4 (grouped-query / multi-query).
    pub head_dim: u32,         // both: 256.
    pub w_q: Vec<i8>,          // (hidden, num_q_heads * head_dim) col-major.
    pub w_k: Vec<i8>,          // (hidden, num_kv_heads * head_dim) col-major.
    pub w_v: Vec<i8>,          // (hidden, num_kv_heads * head_dim) col-major.
    pub w_o: Vec<i8>,          // (num_q_heads * head_dim, hidden) col-major.
}

pub struct AttentionScales {
    pub q: Scale, pub k: Scale, pub v: Scale,
    pub score: Scale,    // pre-softmax: (Q · K^T) / sqrt(head_dim) → softmax-domain.
    pub attn_out: Scale, // post-V multiply.
    pub o: Scale,        // post-output projection.
}

pub fn attention_forward(
    input: &[i8],              // (m, hidden) row-major.
    weights: &AttentionWeights,
    scales: AttentionScales,
    rope_tables: &RopeTables,
    softmax_lut: &ExpLut,
    m: u32,                    // sequence length.
    output: &mut [i8],         // (m, hidden) row-major.
) -> Result<(), AttentionError>;
```

**Computation steps:**

1. `q = input @ W_q` → requantize → reshape to `(m, num_q_heads, head_dim)`.
2. `k = input @ W_k` → requantize → reshape to `(m, num_kv_heads, head_dim)`.
3. `v = input @ W_v` → requantize → reshape to `(m, num_kv_heads, head_dim)`.
4. RoPE: apply `rope_apply` to each `(pos, head)` slot of `q` and `k`.
5. Per-head: `scores[i, j] = (q[i] · k[j]) / sqrt(head_dim)` represented in
   integer via `score` scale. **Causal mask**: `j > i ⇒ score = i32::MIN`
   (so softmax assigns ~zero mass).
6. For each query row `i`, run `softmax_int(scores[i, ..i+1], lut, probs)`.
7. `attn_out[i, head, d] = Σ_j probs[j] * v[j, head_for_kv(head), d]`,
   requantize via `attn_out` scale.
8. Reshape attn_out to `(m, num_q_heads * head_dim)`. Multiply by `W_o`,
   requantize via `o` scale → `output`.

**Determinism gotchas:**

- `i32::MIN` as the masked-position sentinel is non-portable if the
  softmax ever subtracts it from itself; pick a large but representable
  negative (e.g. `-(1 << 30)`) so `(max - score)` never overflows i64.
- Grouped-query: `head_for_kv(head_idx) = head_idx * num_kv_heads / num_q_heads`.
  Pin this mapping in code.
- Reduction order is row-major over `j` for the score dot product, then
  ascending `j` for the V weighted sum.

**Tests:**
- 12 unit: zero-input → zero-output, single-token (no causal interaction),
  identity RoPE + uniform softmax LUT → average-of-V, causal mask
  ordering, GQA head mapping at num_kv=1 vs num_kv=num_q, length-mismatch
  rejections, scale roundtrip (matmul-int8 vs combined), determinism.
- 1 pin `pin_attention_canonical_output` over a small (m=4, hidden=8,
  num_q=2, num_kv=1, head_dim=4) fixture with deterministic LCG inputs.

**Cost:** ~500 lines + tests. One commit.

### ~~2.2 Gated DeltaNet recurrence (`src/deltanet.rs`)~~ ✓ shipped

Qwen 3.6 27B uses 3 DeltaNet blocks for every 1 Attention block (16 hybrid
blocks total → 64 layers). DeltaNet is a linear-attention variant with a
per-step state update:

```text
state_t = state_{t-1} - α_t * k_t * v_t.T + β_t * key_t * value_t.T
output_t = state_t @ q_t
```

**Public surface:**

```rust
pub struct DeltaNetWeights {
    pub hidden: u32,
    pub num_v_heads: u32,        // Qwen: 48 linear V heads.
    pub num_qk_heads: u32,       // Qwen: 16 QK heads.
    pub head_dim: u32,           // Qwen: 128 (different from attention's 256).
    pub w_q: Vec<i8>,            // (hidden, num_qk_heads * head_dim) col-major.
    pub w_k: Vec<i8>,
    pub w_v: Vec<i8>,            // (hidden, num_v_heads * head_dim).
    pub w_alpha: Vec<i8>,        // (hidden, num_qk_heads) — per-head decay.
    pub w_beta: Vec<i8>,         // (hidden, num_qk_heads) — per-head update gate.
    pub w_o: Vec<i8>,
}

pub fn deltanet_forward(
    input: &[i8],
    weights: &DeltaNetWeights,
    scales: DeltaNetScales,
    m: u32,
    output: &mut [i8],
) -> Result<(), DeltaNetError>;
```

**Steps:**
1. Project Q, K, V, α, β.
2. For each token `t = 0..m`: update each head's state matrix
   `S_t = S_{t-1} - α_t * outer(k_t, v_old) + β_t * outer(k_t, v_t)`.
   *Or* the simplified form: `S_t = (I - α_t k_t k_t^T) S_{t-1} + β_t k_t v_t^T`.
3. Output: `o_t = S_t @ q_t`.

**Determinism gotchas:**

- The state matrix `S` is `head_dim_qk × head_dim_v` per head, in i32
  fixed-point. After each update, requantize to i8 to bound state growth.
- `α`, `β` are per-token per-head sigmoids; replace with a small LUT
  (committed alongside softmax LUT).
- The `outer(k_t, v_t)` produces a `head_dim_qk × head_dim_v` matrix; for
  Qwen `head_dim = 128` so the per-head state is 16 KB INT8. With 16 QK
  heads and `m = 4096` we touch ~256 MB of state-update i32 traffic per
  layer; acceptable for offline reference, slow for production miners.

**Tests:**
- Zero-input → zero-output.
- α = 0, β = 1: state replaces, output = k @ k_t^T @ v_t (per-head).
- α = 1, β = 0: state decays to zero across tokens.
- Determinism on a small fixture.
- Pin `pin_deltanet_canonical_output`.

**Cost:** ~700 lines. One commit. Bit-exactness against PyTorch may
require iteration as the reference implementation isn't standard.

### ~~2.3 Activation Merkle log (`src/activations.rs`)~~ ✓ shipped

Wraps `ai-pow`'s tile-Merkle so per-layer activations are committed and
spot-checkable.

**Public surface:**

```rust
pub struct ActivationLog {
    /// Per-layer roots, in order.
    pub layer_roots: Vec<[u8; 32]>,
    /// Retained tile state hashes for prover-side openings.
    /// `tiles[layer_idx][tile_idx]` is the leaf hash.
    pub tiles: Vec<Vec<[u8; 32]>>,
    pub layout: ActivationLayout,
}

pub struct ActivationLayout {
    pub seq_len: u32,
    pub hidden: u32,
    pub tile: u32,           // both axes use the same tile size.
}

impl ActivationLog {
    pub fn record_layer(&mut self, layer_idx: u32, tensor: &[i8]) -> Result<(), Err>;
    pub fn open(&self, layer_idx: u32, tile_idx: u32) -> Result<MerklePath, Err>;
    pub fn root(&self, layer_idx: u32) -> [u8; 32];
}
```

**Wire format:** layer roots concatenated in canonical order; the verifier
recomputes any opened tile from a small slice of the original activation
plus the path.

**Tests:**
- Recording one layer matches `ai-pow::commit::merkle_root` directly.
- Opening any tile recovers the root.
- Tampering one byte changes the root.

**Cost:** ~200 lines + tests. One commit.

### ~~2.4 Layer composition (`src/layer.rs`)~~ ✓ shipped

Stitches norm + attention/deltanet + FFN with residual connections.

```rust
pub fn forward_layer(
    input: &[i8],          // (m, hidden) — input activations to this layer.
    layer: &LayerWeights,  // pulled from ModelWeights.
    layer_idx: u32,
    rope_tables: &RopeTables,
    softmax_lut: &ExpLut,
    m: u32,
    output: &mut [i8],     // (m, hidden) — output activations.
) -> Result<(), LayerError>;
```

`LayerWeights` is a tagged union:

```rust
pub enum LayerWeights {
    Attention { norm1: NormWeights, attn: AttentionWeights, norm2: NormWeights, ffn: FfnWeights, scales: AttentionLayerScales },
    DeltaNet  { norm1: NormWeights, dnet: DeltaNetWeights, norm2: NormWeights, ffn: FfnWeights, scales: DeltaNetLayerScales },
}
```

Standard residual stream: `x → norm1 → attn → +x → norm2 → ffn → +x`.

**Tests:**
- Identity weights → identity output up to integer rounding noise.
- Layer at depth 0 vs layer at depth 1 with different weights → different
  outputs.

**Cost:** ~250 lines. One commit.

### ~~2.5 Forward-pass driver (`src/forward.rs`)~~ ✓ shipped

```rust
pub fn forward_prefix(
    prompt: &[Token],         // length seq_len.
    model: &Model,            // weights + tables + LUTs + comm_W.
    target_layer: u32,
    log: &mut ActivationLog,  // populated as we go.
) -> Result<Vec<i8>, ForwardError>; // returns the (m, hidden) tensor at target_layer's input.
```

Steps:
1. Embed: `x = embed_table[prompt[i]]` per token.
2. For `layer_idx in 0..target_layer`: `forward_layer`, record into log.
3. Apply final norm (or skip if `target_layer < num_layers`).
4. Return.

**Tests:**
- Zero prompt with zero embed table → zero output at every layer.
- Prefix-to-layer-1 vs prefix-to-layer-2 differ.

**Cost:** ~150 lines. One commit.

### ~~2.6 Prompt synthesis (`src/prompt.rs`)~~ ✓ shipped

```rust
pub fn synth_prompt(
    block_commitment: &[u8],
    model_id: &[u8; 32],
    seq_len: u32,
    vocab_size: u32,
    reserved_tokens: &[Token],   // BOS, EOS, special, masked out.
) -> Vec<Token>;
```

BLAKE3-XOF stream over `(block_commitment || model_id || "ai-pow-vi v1 prompt")`.
Sample `u32`s; reduce modulo `vocab_size`; reject reserved; emit `seq_len`
tokens.

**Tests:**
- Determinism.
- Different `block_commitment` → different prompt.
- Reserved tokens never appear.

**Cost:** ~80 lines. One commit (could combine with §2.7).

### ~~2.7 Model and `comm_W` commitment~~ ✓ shipped (sideloaded disk format deferred — `compute_comm_w` is the consensus-critical part)

### Original 2.7 Model and weights (`src/weights.rs`, `src/model.rs`)

```rust
pub struct ModelWeights {
    pub layout: ModelLayout,
    pub embed: Vec<i8>,                // (vocab, hidden).
    pub layers: Vec<LayerWeights>,
    pub final_norm: NormWeights,
    pub lm_head: Vec<i8>,
    pub activation_luts: Vec<ActivationLut>,
    pub softmax_lut: ExpLut,
    pub rope_tables: RopeTables,
}

pub struct Model {
    pub layout: ModelLayout,
    pub weights: ModelWeights,
    pub comm_w: [u8; 32],
}

impl Model {
    /// Load from sideloaded directory. Recomputes comm_W; aborts if it
    /// doesn't match `expected_comm_w`.
    pub fn load(dir: &Path, expected_comm_w: &[u8; 32]) -> Result<Self, LoadError>;

    /// Compute comm_W from scratch (for model-build time / testing).
    pub fn compute_comm_w(weights: &ModelWeights) -> [u8; 32];
}
```

**Sideload directory layout:**

```
$NOCKCHAIN_DATA/models/<model_id_hex>/
  manifest.json    # ModelLayout serialized.
  weights.bin      # All INT8 tensors concatenated in canonical order:
                   #   embed | layer[0].norm1 | layer[0].attn.{q,k,v,o} | ...
                   #   ... | layer[N].ffn.{gate,up,down} | final_norm | lm_head
  activation_luts.bin    # All activation LUTs concatenated.
  softmax_lut.bin
  rope_tables.bin
```

**`comm_W` = tile-Merkle root** over the canonical concatenation, using
`ai-pow::commit::merkle_root` with tile size chosen at registry time
(probably 64 to match the FFN tile).

**Tests:**
- Round-trip: build a synthetic small model, compute `comm_W`, save to
  a temp dir, reload, recompute, check identity.
- 1-byte flip in any file changes `comm_W`.
- Manifest mismatch is rejected.

**Cost:** ~400 lines. One commit.

### ~~2.8 Oracle (`oracle/`)~~ ✓ shipped (numpy reference + 7 fixtures + Rust loader; HF/GGUF skeleton scripts deferred until a real model is converted)

Not consensus code; only run by developers / CI to seed test vectors.

```
oracle/
  README.md
  requirements.txt          # pinned: torch==2.6.0, transformers==4.50.0, ...
  extract_weights.py        # HF model → INT8 binary in canonical order.
  forward_prefix_oracle.py  # Run prefix forward; dump activations as i8.
  save_test_vectors.py      # Wrapper that builds the small fixtures.
  test_vectors/
    gemma_4_31b_layer_8/
      input_tokens.bin
      activations_per_layer.bin   # all post-norm + post-attention bytes.
      ffn_gate_up.bin             # final FFN gate/up output.
    qwen_3_6_27b_layer_8/
      ...
```

The oracle scripts are pinned and the test vectors are checked in
(small enough — ~10 MB per fixture for a 64-token prefix). Rust tests
load the fixture, run `forward_prefix`, and `assert_eq!` byte-for-byte
against the saved activations.

**Tests:**
- `tests/oracle_gemma.rs`: load Gemma test vector, run prefix-to-layer-8,
  byte-equal to saved.
- `tests/oracle_qwen.rs`: same for Qwen.

**Cost:** ~600 lines of Python + ~150 lines of Rust test driver. Two
commits (one for oracle scripts + small synthetic fixture; one for the
real Gemma + Qwen vectors after the rust ops actually match).

### 2.9 Real-model bring-up: Qwen 3.6 27B end-to-end

Goal: convert a downloaded Qwen 3.6 27B (Ollama GGUF or Hugging Face
safetensors) into our canonical INT8 layout, load it into a
[`crate::model::Model`], and produce a real-model fixture that
`tests/oracle_qwen.rs` byte-compares against the numpy reference. This
is what unlocks Phase 4 jet calibration and Phase 6 mainnet weight
distribution; until 2.9 lands, every test runs on synthetic
LCG-generated weights.

The deliverables decompose into seven commits, each independently
testable:

#### 2.9.1 Disk format + `Model::load` / `Model::save` (`src/io.rs`)

Sideloaded directory layout:

```
$NOCKCHAIN_DATA/models/<model_id_hex>/
  manifest.bin       # ModelManifest: dims, per-layer block kinds, every Scale, eps_q
  weights.bin        # i8 tensors in `canonical_weight_bytes` order (Phase 2.7)
  rope.bin           # u32 seq_len || u32 half_head_dim || i16 cos[] || i16 sin[]
  softmax_lut.bin    # 256 LE i32
  sigmoid_lut.bin    # 256 i8
  ffn_lut.bin        # 256 i8
  comm_w.hex         # 64-char hex of expected comm_W (sanity, not authoritative)
```

Hand-rolled binary serializer for `ModelManifest` (no serde dependency
— matches the rest of the crate's "explicit byte stream" style). New API:

```rust
impl Model {
    pub fn load(dir: &Path, expected_comm_w: &[u8; 32]) -> Result<Self, LoadError>;
    pub fn save(&self, dir: &Path) -> Result<(), SaveError>;
}
```

`load` recomputes `comm_W` after parsing and aborts on mismatch — this
is the single chokepoint that protects all downstream code from
weight-tampering at rest.

**Tests:** synthetic round-trip (save → load → equal model); flipping
one byte of `weights.bin` trips the comm_W check; a wrong
`expected_comm_w` argument aborts even if the file is intact.

**Cost:** ~600 lines + tests. One commit.

#### 2.9.2 GGUF reader (`oracle/gguf_reader.py`)

Use the pinned `gguf` Python package (>=0.10) to parse Qwen's GGUF
file. Dequantize Q4_K_M / Q6_K / Q8_0 to `np.float32` per the
established llama.cpp formulas. Emit a flat dict keyed by canonical
tensor names.

Tensor-name mapping (Qwen3-style; extend as needed for the actual
hybrid block kinds):

| GGUF name                       | Canonical                       |
|---------------------------------|---------------------------------|
| `token_embd.weight`             | `embed`                         |
| `blk.{N}.attn_norm.weight`      | `layer[N].norm1.gamma`          |
| `blk.{N}.attn_q.weight`         | `layer[N].attn.w_q`             |
| `blk.{N}.attn_k.weight`         | `layer[N].attn.w_k`             |
| `blk.{N}.attn_v.weight`         | `layer[N].attn.w_v`             |
| `blk.{N}.attn_output.weight`    | `layer[N].attn.w_o`             |
| `blk.{N}.ffn_norm.weight`       | `layer[N].norm2.gamma`          |
| `blk.{N}.ffn_gate.weight`       | `layer[N].ffn.w_gate`           |
| `blk.{N}.ffn_up.weight`         | `layer[N].ffn.w_up`             |
| `blk.{N}.ffn_down.weight`       | `layer[N].ffn.w_down`           |
| `blk.{N}.delta_q.weight` etc.   | `layer[N].dnet.{q,k,v,alpha,beta,o}` |
| `output_norm.weight`            | `final_norm.gamma`              |

Linear weights need a column-major transpose (HF/GGUF stores
`(out, in)`; we want `(in, out)` column-major).

**Tests:** load a tiny synthetic GGUF file (≤10 KB, generated in the
test) and verify the emitted dict shapes match expectations.

**Cost:** ~400 lines Python.

#### 2.9.3 Activation-scale calibration (`oracle/calibrate.py`)

Per-tensor symmetric INT8 needs activation scales. Approach:

1. Take a calibration set of N=256 short prompts (FineWeb-Edu sample
   + the 16-token canonical prompts the puzzle uses).
2. Run a numpy or torch f32 forward over each prompt, tracking
   `max(abs(activation))` at every quantization point (Q/K/V projection,
   pre-softmax score, post-softmax probs * V, FFN gate, up, mid, down,
   each norm post-scale).
3. Pick `Scale::from_f32(max_abs / 127)` per quantization point and
   per layer (we keep the per-layer dimension for now — a future
   tightening can collapse if scales are layer-stable).

Output: `oracle/test_vectors/qwen_3_6_27b/scales.json` consumed by
`quantize_qwen.py`.

**Tests:** calibrate over the synthetic LCG fixture and verify the
emitted scales reproduce the existing pin values to ±1 LSB.

**Cost:** ~300 lines Python + ~30 min calibration runtime per model.

#### 2.9.4 INT8 quantizer (`oracle/quantize_qwen.py`)

Driver: pull f32 tensors from `gguf_reader.py`, the calibration
scales from `calibrate.py`, then:

1. Per-tensor weight scale: `s_w = max(|w|) / 127`.
2. Quantize: `w_int8 = round(w / s_w).clamp(-128, 127)`.
3. Build LUTs: SiLU/GeLU table, sigmoid table for DeltaNet α/β,
   exp table for softmax. All committed inside `comm_W`.
4. Build RoPE tables in i16 fixed-point at `FRACT_BITS=14`.
5. Assemble `Model` and call `Model::save(dir)`.
6. Print computed `comm_W`; user records it for the registry.

**Acceptance:** the emitted directory loads without error via
`Model::load(dir, &computed_comm_w)`.

**Cost:** ~400 lines Python. One commit, depends on 2.9.1 + 2.9.2 + 2.9.3.

#### 2.9.5 Numpy reference for the full forward (`oracle/forward_prefix_oracle.py`)

Complete the Phase 2.8 skeleton: implement `attention_forward`,
`deltanet_forward`, `forward_layer`, `forward_prefix` in numpy using
the existing `reference_ops.py` primitives. Mirrors the Rust
composition exactly (residual saturation, GQA head mapping, causal
slicing, etc.).

The activation tile-Merkle (Phase 2.3) commitment uses BLAKE3
`derive_key` over leaf bytes, same as `crate::activations`. Mirror it
in Python with the `blake3` module.

**Tests:** run `forward_prefix_oracle` on the small synthetic model
the determinism pin uses (`pin_attention_layer_canonical_output`) and
check byte-equality against the Rust output.

**Cost:** ~600 lines Python + ~50 lines Rust test driver.

#### 2.9.6 Real-model fixture + Rust integration test (`tests/oracle_qwen.rs`)

Once 2.9.4 + 2.9.5 land, generate the real fixture:

```
python oracle/forward_prefix_oracle.py \
    --model-dir $NOCKCHAIN_VI_QWEN_DIR \
    --layer 8 \
    --prompt-seed 0xpin \
    --out oracle/test_vectors/qwen_3_6_27b_layer_8/
```

Outputs `input_tokens.bin`, `activations_layer_{0..8}.bin` (each ~30
MB at full seq_len=4096; reduced for the checked-in fixture by using
seq_len=64 → ~50 KB total).

`tests/oracle_qwen.rs` is gated `#[ignore]` and takes the model dir
via `NOCKCHAIN_VI_QWEN_DIR`. CI runs it on a self-hosted box that has
the model; default cargo test skips it.

**Acceptance:** byte-equal forward to layer 8 over a 64-token prefix.

**Cost:** ~150 lines Rust + the fixture (~50 KB checked in; full real-
seq_len fixture available via download script, not Git LFS).

#### 2.9.7 Tokenizer adapter + accuracy gate (`tools/qwen-eval/main.rs`)

Optional but high-value for confidence. A Rust binary that:

1. Loads the INT8 Qwen via `Model::load`.
2. Reads a 100-prompt eval set (e.g. HellaSwag dev or a short slice of
   FineWeb-Edu) tokenized via the HF Qwen tokenizer (called from
   Python through a small subprocess wrapper, or via a Rust wrapper
   like `tokenizers`).
3. For each prompt, runs `forward_prefix` to the final layer, applies
   `lm_head` (shipped from 2.9.4), picks `argmax` next token.
4. Compares against Ollama's response on the same prompt.
5. Reports top-1 next-token agreement.

INT8 vs Ollama's native bf16 won't match exactly, but on calibrated
scales we expect ≥ 90% top-1 agreement on natural text. Lower numbers
indicate calibration drift.

**Cost:** ~250 lines Rust + ~20 min eval runtime per pass.

#### 2.9.8 CI fixture: synthetic Qwen-mini (`oracle/synthetic_qwen_mini.py`)

A 4-layer Qwen-shaped model (hidden=64, num_q=2, num_kv=1, head_dim=4,
intermediate=128, vocab=32) with deterministic random weights. Runs
the full `quantize_qwen → save → Model::load → forward_prefix`
pipeline in < 10s, with no external model download. Lives in CI so
the conversion path doesn't bit-rot between real-model bring-ups.

**Cost:** ~300 lines Python + ~150 lines Rust integration test.

#### 2.9 acceptance gates

1. `Model::save(dir)` then `Model::load(dir, &expected_comm_w)`
   round-trips; one-byte file flip aborts the load.
2. `synthetic_qwen_mini.py` end-to-end pipeline runs in CI in < 10s
   without external downloads.
3. `quantize_qwen.py` produces a valid model dir for the real Qwen 3.6
   27B; `Model::load` succeeds.
4. `tests/oracle_qwen.rs` (with `NOCKCHAIN_VI_QWEN_DIR` set) is
   byte-equal to the numpy oracle on a 64-token forward to layer 8.
5. `tools/qwen-eval` reports ≥ 90% top-1 next-token agreement against
   Ollama on the chosen 100-prompt eval set. (Loosen to ≥ 80% if
   calibration needs more work; treat the gap as a backlog item.)

#### 2.9 dev workflow (in `oracle/README_QWEN.md`)

```
ollama pull qwen3.6:27b              # ≈ 15 GB download
python oracle/gguf_reader.py \
    --gguf ~/.ollama/models/blobs/sha256-... \
    --out /tmp/qwen-f32/
python oracle/calibrate.py \
    --weights /tmp/qwen-f32/ \
    --out oracle/test_vectors/qwen_3_6_27b/scales.json
python oracle/quantize_qwen.py \
    --weights /tmp/qwen-f32/ \
    --scales oracle/test_vectors/qwen_3_6_27b/scales.json \
    --out $NOCKCHAIN_VI_QWEN_DIR
python oracle/forward_prefix_oracle.py \
    --model-dir $NOCKCHAIN_VI_QWEN_DIR --layer 8 \
    --out oracle/test_vectors/qwen_3_6_27b_layer_8/
NOCKCHAIN_VI_QWEN_DIR=$NOCKCHAIN_VI_QWEN_DIR \
    cargo test -p ai-pow-vi --test oracle_qwen -- --ignored
cargo run -p ai-pow-vi --bin qwen-eval -- \
    --model-dir $NOCKCHAIN_VI_QWEN_DIR --eval-set evals/hellaswag-100.txt
```

Expect ~90 min wall-clock from `ollama pull` to all gates green on a
modern laptop.

### 2.10 Extensible architecture support: foundations

Phase 2.9 brought up a generic dense-attention path with `qwen3`-style
tensor names. Real models in the wild use **architecturally different
transformers**: Gemma 4 (`gemma4`) has QK norm, sliding-window
attention, per-block input gating, and final logit softcapping; Qwen
3.6 27B (`qwen35`) is a *hybrid* with 16/64 standard-attention blocks
interleaved with 48/64 gated-attention-plus-Mamba-SSM blocks. To
support both — and any future Qwen / Gemma / Llama variant — we move
to a registry-driven design where each new architecture is a small
module rather than a fork.

**Foundations (this phase, ~700 lines):**

1. `oracle/arch/__init__.py` — `Architecture` abstract base + global
   `REGISTRY: dict[str, Architecture]`. Each concrete subclass declares:
   - `name`: GGUF `general.architecture` value it claims.
   - `arch_dims(reader) -> ArchDims`: pull per-arch KV fields out of
     the GGUF metadata.
   - `block_kind(reader, block_idx) -> BlockKind`: classify each block
     so the same model can have mixed kinds (qwen35 case).
   - `tensor_alias_map() -> dict[str, str]`: GGUF tensor name → our
     canonical name.
   - `feature_flags() -> set[Feature]`: e.g. `QK_NORM`, `INP_GATE`,
     `LOGIT_SOFTCAP`, `SLIDING_WINDOW`, `SSM_PARALLEL`, `FUSED_QKV`.

2. `oracle/arch/qwen3_legacy.py` — keep the existing Phase 2.9 path,
   re-routed through the registry.

3. `oracle/arch/qwen35.py` — name map for Qwen 3.6 27B. Reports
   `STANDARD-ATTENTION` vs `GATED_ATTN_SSM` per block. Does **not**
   yet implement SSM forward (deferred to 2.13).

4. `oracle/arch/gemma4.py` — name map for Gemma 4 8B / 31B; sets
   `feature_flags = {QK_NORM, INP_GATE, POST_FFW_NORM,
   LOGIT_SOFTCAP, SLIDING_WINDOW_PATTERN}`.

5. **Manifest v2** in `src/io.rs`: add `arch_tag: [u8; 16]` and a
   `feature_flags: u64` field right after `version`. Loader rejects
   unsupported feature flags. `comm_W` includes these bytes.

6. **Manifest pin update.** All 17 existing determinism pins refresh
   to v2 — explicit acknowledgment per the working-conventions note.

**Acceptance:**

- `oracle/gguf_reader.py` reads Qwen 3.6 27B and Gemma 4 8B without
  errors; emits f32 tensors with canonical names.
- A new `oracle/tests/test_arch_registry.py` confirms (a) the registry
  has both architectures, (b) name maps produce the expected canonical
  keys for a small sample of blocks, (c) feature_flags are surfaced
  correctly.
- `tests/oracle_qwen_mini.rs` and `tests/oracle_quantized_synthetic.rs`
  still pass under the new manifest v2 (with regenerated pins).

### 2.11 Gemma 4 attention extensions

Gemma 4 8B + 31B share the same `gemma4` architecture. After 2.10's
foundations, this phase implements every Gemma-specific op so the full
forward can run:

1. **QK norm in `crate::attention`**: a per-head RMSNorm applied to
   `Q` and `K` after the linear projections, before RoPE. Pinned norm
   gammas live alongside the Q/K weight tensors.

2. **`GemmaLayer` variant of `LayerWeights`**: carries the 4 norms
   (`norm1`, `norm2`, `post_ffn_norm`, `output_norm`), the optional
   `inp_gate` and `layer_output_scale` 1-D tensors, and a `proj`
   weight for the small per-layer hidden-state down-projection.

3. **Sliding-window attention**: extend `attention_forward` with an
   `Option<u32> window` parameter. When `Some(w)`, the causal mask
   is also bounded below by `j >= i - w`. The block kind tag selects
   between full and sliding attention according to the GGUF
   `sliding_window_pattern` array.

4. **Final logit softcapping**: in `qwen-eval`, after `lm_head`,
   apply `logits = tanh(logits / cap) * cap` using a committed tanh
   LUT (so still INT8-deterministic).

5. **Numpy reference parity** in `oracle/forward_reference.py` for
   each of the above.

6. **End-to-end real-model fixture**: convert Gemma 4 8B → load via
   `Model::load` → `forward_prefix` to layer 8 → byte-equal numpy
   oracle. Then convert Gemma 4 31B (whichever variant the user has
   pulled) → smoke-test load + 1-layer forward.

**Acceptance gates:**

- `tests/oracle_gemma.rs` (gated `#[ignore]`) passes byte-equal on a
  64-token prefix for Gemma 4 8B.
- `qwen-eval` reports ≥ 70% top-1 agreement vs Ollama on a 50-prompt
  eval set for Gemma 4 8B (loosen vs the Qwen 90% bar because Gemma's
  bf16-vs-INT8 gap is wider in practice — calibration is harder).

### 2.12 Qwen 3.6 27B attention-only path

Stage 1 of the Qwen hybrid: implement everything except SSM. Pure
attention blocks become first-class; hybrid blocks return a clear
"unsupported block kind" error that names which sub-component is
missing.

1. **Fused QKV splitting** in `gguf_reader`: when a block has
   `attn_qkv.weight` instead of three separate tensors, split the
   `(hidden, q_dim + k_dim + v_dim)` matrix into three canonical
   tensors using the `key_length`, `value_length` KV fields.

2. **`attn_gate` (gated attention)**: add a per-block 1-D gate
   weight to `AttentionWeights`, default `None`. When present,
   pre-multiply the attention output by `sigmoid(input @ gate)`
   per-token, per-head. Numpy + Rust parity.

3. **QK norm shared with Phase 2.11**.

4. **Block-kind dispatch**: a Qwen 3.6 layer is either
   `QwenStandardAttention` or `QwenHybridSSM`. The latter currently
   panics with "Phase 2.13 not yet implemented"; the former runs.

5. **Acceptance**: a "Qwen-skip-SSM" mode in `Model::load` that
   short-circuits the hybrid blocks to identity. Output is **not**
   semantically meaningful, but `forward_prefix` runs to completion
   and exercises the full dispatch machinery. Documented as a
   debugging mode, not a release path.

### 2.13 Mamba SSM block (qwen35 hybrid layers)

The biggest remaining piece. A hybrid block performs *in parallel*:

- Gated attention path (from 2.12).
- Mamba SSM path: `h_t = A * h_{t-1} + B(x_t) * x_t`,
  `y_t = C(x_t) * h_t + D * x_t`, plus a 1-D causal conv on `x_t`
  before the recurrence.

Sub-items:

1. **`crate::ssm` module**: INT8 forward for a single SSM head.
   State is `head_dim_state × head_dim_v` i8 per V head; updates in
   i32, requantize to i8 between tokens — same pattern as DeltaNet.
2. **1-D causal conv (`ssm_conv1d`)**: small kernel (4 here) over
   the (m, hidden) input; integer, banker-rounded.
3. **`ssm_alpha`, `ssm_beta`**: per-token, per-head gating via a
   sigmoid LUT (shared with DeltaNet).
4. **`ssm_a`, `ssm_dt`**: state-transition constants and per-channel
   time-step adjustments.
5. **`QwenHybridSSM` block forward**: combine gated attention and
   SSM outputs via residual stream.
6. **Cross-impl byte-equality**: numpy reference matches Rust on a
   single SSM head + on a full hybrid block fixture.

Acceptance: `forward_prefix` for the real Qwen 3.6 27B over a
64-token prefix runs to completion and byte-equals the numpy oracle
end-to-end (not just on pure-attention layers).

### 2.14 Multi-architecture acceptance gate

After 2.10–2.13 land, this is the final integration. Adds:

- `tests/oracle_multi_arch.rs` parameterized over every supported
  arch + the user's locally-available model(s).
- ROADMAP "shipped" table grows entries for Qwen 3.6 27B and Gemma 4
  (8B + 31B), each with their pinned `comm_W` and the model_id_hex
  that the consensus registry will reference.
- `qwen-eval` rename → `vi-eval`, with `--arch` flag.

Acceptance gates:

1. Both real models load via `Model::load` with the published
   `comm_W` from this branch.
2. `oracle_multi_arch` passes for every architecture × at least one
   prompt batch.
3. Top-1 next-token agreement vs Ollama ≥ 90% for Qwen 3.6 27B and
   ≥ 70% for Gemma 4 (per arch-specific tolerances established in
   2.11 / 2.12).

### Phase 2 acceptance gate (re-stated)

1. `cargo test -p ai-pow-vi` green on x86_64 and aarch64 CI.
2. Per-op tests vs PyTorch oracle pass on ≥ 16 random inputs each.
3. End-to-end prefix-to-layer-8 byte-equal to oracle on real Gemma +
   Qwen INT8 weights.
4. `comm_W` constants pinned in `weights.rs`.
5. Prefix forward to layer 8 ≤ 200 ms on a single CPU core (scalar
   baseline).

## ~~Phase 3 — VI prover and verifier APIs~~ ✓ shipped (FullReplica mode; light-path + federated deferred)

### 3.1 `ViProof` wire format (`src/proof.rs`)

```rust
pub struct ViProof {
    pub model_id: [u8; 32],
    pub layer_index: u32,
    pub comm_activations: Vec<[u8; 32]>,  // one per layer 0..=L.
    pub comm_m: [u8; 32],                  // tile-Merkle of FFN tile state.
    pub found: TileOpening,
    pub weight_openings: Vec<MerklePath>,  // for spot-checked weight tiles.
    pub activation_openings: Vec<MerklePath>,
    pub spot_checks: Vec<TileOpening>,
}
```

Binary encode/decode following the `ai-pow::proof` pattern.

### 3.2 Prover (`src/prover.rs`)

```rust
pub fn mine_vi(
    block_commitment: &[u8],
    nonce: &[u8],
    registry: &ModelRegistry,
    target: &[u8; 32],
    opts: ProverOptions,
) -> Result<Option<ViProof>, MineError>;
```

Flow:
1. Synth prompt → `forward_prefix` to FS-derived layer.
2. Compute FFN gate/up matmul tile-by-tile, building `comm_M` over tile
   state hashes.
3. For each tile, hash `tile_hardness = BLAKE3(challenge_seed || (i,j) || M_ij)`.
4. If any tile satisfies `tile_hardness ≤ target`: collect σ FS-derived
   spot-check openings and package the proof.
5. Return `Ok(None)` if no tile passes.

### 3.3 Verifier (`src/verifier.rs`)

Three modes:

```rust
pub enum VerifierMode {
    FullReplica,         // re-run prefix forward; spot-check FFN tiles.
    LightSpotCheck,      // verify activations via Merkle paths, not recomputation.
    Federated(Vec<PeerId>),
}

pub fn verify_vi(
    block_commitment: &[u8],
    nonce: &[u8],
    target: &[u8; 32],
    registry: &ModelRegistry,
    proof: &ViProof,
    mode: VerifierMode,
) -> Result<(), VerifyError>;
```

Light path: σ + 1 spot-checks at FS-derived (layer, tile) coordinates;
each verifies a weight tile + activation tile + recomputed step against
the corresponding committed root.

Acceptance: light path ≤ 100 ms at LLM shapes (with SIMD, after Phase 4
jets land).

## Phase 4 — Jets

In `crates/zkvm-jetpack/src/jets/`:

- `blake3_jet.rs`, `tile_merkle_jet.rs` — already needed by Phase 1.5.
- `matmul_int8_jet.rs` — SIMD INT8 dot product (AVX2 / AVX512-VNNI / NEON).
  **Required** for the light-path 100 ms budget.
- `rmsnorm_jet.rs`, `layernorm_jet.rs`.
- `softmax_jet.rs`.
- `rope_jet.rs`.
- `activation_lut_jet.rs`.
- `ai_pow_vi_verify_jet.rs` — full verifier jet, calls into all of the
  above.

Jet output bit-equal to direct `ai-pow-vi` output on every supported
architecture; cross-arch CI matrix.

## Phase 5 — Hoon consensus + persistence + hard fork

File-by-file change list (from the plan):

- `hoon/common/tx-engine-1.hoon` lines 65–77: change
  `pow=$+(pow (unit proof))` to
  `pow=$+(pow (unit $%([%nock proof:sp] [%matmul-vi vi-proof])))`.
- `hoon/common/pow.hoon`: keep `check-target` for `%nock`. Add
  `verify-matmul-vi-proof` arm calling the jet.
- `hoon/apps/dumbnet/lib/consensus.hoon`:
  - Lines 386–395 (`validate-page-without-txs`): branch on tag and
    height. `height < matmul_vi_phase` ⇒ `%nock`; else `%matmul-vi`.
  - Lines 429–436: accumulated-work uses `2^256 / target`; no weighting
    post-cutover.
  - Lines 206–228 (`compute-target-asert`): special-case
    `height == matmul_vi_phase` to reset target to
    `initial_matmul_vi_target_atom`.
- `hoon/apps/dumbnet/lib/types.hoon`: extend `consensus-state:dk` with
  `model-registry=(map model-id model-metadata)`.
- `crates/nockchain-types/src/blockchain_constants.rs` (struct lines
  64–88; builders 175–203): add `pub matmul_vi_phase: u64` and
  `pub initial_matmul_vi_target_atom: BigNum`.
- `crates/nockchain/src/mining.rs` (lines 113–331): branch the prover
  loop on `current_height` vs `matmul_vi_phase`.

Acceptance gates:
1. Hoon verifier tests: handcrafted valid `%matmul-vi` proof accepted;
   tampered fields rejected.
2. Phase-gate test: `%matmul-vi` rejected at `height < phase`; `%nock`
   rejected at `height ≥ phase`.
3. Pre-cutover replay: replaying historical mainnet blocks against the
   new binary still validates them via the legacy STARK path.
4. Fakenet integration: `MATMUL_VI_PHASE = 1`; ≥ 100 blocks of
   `%matmul-vi`; ASERT retargets correctly.

## Phase 6 — Mainnet rollout

- Publish reference INT8 weights for Gemma 4 31B and Qwen 3.6 27B on a
  content-addressed mirror network. Distribute `comm_W` constants pinned
  in the binary.
- Software released ≥ 4 weeks before `matmul_vi_phase`.
- ≥ 1000-block pure-VI testnet run with ≥ 3 miners.
- Difficulty calibration: choose `initial_matmul_vi_target_atom` so the
  first post-cutover block solve-time matches the existing chain's mean
  block time.
- Communication: forum thread, miner outreach, weight-distribution
  mirrors documented; pinned `comm_W` published before binary release.

## Phase 7 — Real-prompt market (deferred)

`[%prompt tokens=(list token) fee=coins response=(list token)]`
transaction type. Miners pick prompts from mempool; block carries the
answer; fee routes to miner. No further consensus changes — proof type
stays `%matmul-vi`; only the prompt source changes from FS-synth to
"first eligible mempool prompt".

## Working conventions (apply to every commit)

- Each new module ships with: function signatures, exhaustive unit
  tests (≥ 4 per public function), at least one cross-architecture
  determinism pin in `tests/determinism_pins.rs`.
- `cargo fmt -p ai-pow-vi` before every commit.
- `cargo test -p ai-pow-vi` green before every commit.
- Pin updates require explicit acknowledgment — pinned `expected` values
  are the protocol-level contract, and silently changing them breaks
  consensus across deployed nodes.
- Reduction order: row-major, ascending index. Vendor SIMD kernels that
  reorder reductions are non-conformant.
- All transcendentals are LUTs committed inside `comm_W`. No `f32`. No
  `expf`, `sinf`, `cosf` on the consensus path.
- INT16 / INT32 fixed-point with a single fixed denominator per op
  (declared in module-level docs as `FRACT_BITS`).
- `#[inline(never)]` on every op the compiler might be tempted to fuse
  with a non-deterministic intrinsic (rounding, division, transcendental).
