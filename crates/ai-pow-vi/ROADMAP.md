# ai-pow-vi roadmap

Working notes on the remaining work to land the verifiable-inference proof
of work for Nockchain. This is the implementation-level companion to the
high-level plan at `~/.claude/plans/read-these-two-papers-rosy-snail.md`.
Each item is sized for a single commit; subsequent commits stack and the
crate's determinism pins grow with each addition.

## Status snapshot

Branch: `claude/ai-pow-nockchain-sgfNX`. Latest commit: `52a6f77`.

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

Test count: 100 unit + 12 cross-architecture pins, all green on aarch64.

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

### 2.3 Activation Merkle log (`src/activations.rs`)

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

### 2.4 Layer composition (`src/layer.rs`)

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

### 2.5 Forward-pass driver (`src/forward.rs`)

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

### 2.6 Prompt synthesis (`src/prompt.rs`)

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

### 2.7 Model and weights (`src/weights.rs`, `src/model.rs`)

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

### 2.8 PyTorch oracle (`oracle/`)

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

### Phase 2 acceptance gate (re-stated)

1. `cargo test -p ai-pow-vi` green on x86_64 and aarch64 CI.
2. Per-op tests vs PyTorch oracle pass on ≥ 16 random inputs each.
3. End-to-end prefix-to-layer-8 byte-equal to oracle on real Gemma +
   Qwen INT8 weights.
4. `comm_W` constants pinned in `weights.rs`.
5. Prefix forward to layer 8 ≤ 200 ms on a single CPU core (scalar
   baseline).

## Phase 3 — VI prover and verifier APIs

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
