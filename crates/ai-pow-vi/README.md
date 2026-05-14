# `ai-pow-vi`

A deterministic INT8 transformer-layer reference for Nockchain's
verifiable-inference proof of useful work. Phase 2 of the AI-PoW plan:
where [`ai-pow`](../ai-pow/) gives the matrix-multiplication primitive
and the Pearl-style mining puzzle, `ai-pow-vi` composes integer
transformer ops on real LLM weights (Gemma 4 31B, Qwen 3.6 27B) into a
verifiable forward pass whose output is itself the proof.

## Scope

What `ai-pow-vi` provides:

- **Bit-exact INT8 ops**, designed to produce byte-identical outputs on
  the same INT8 inputs across CPU architectures (aarch64 + x86_64 pinned;
  see `determinism.rs::ARCH_TAG`):
  - Integer RMSNorm / LayerNorm with Newton-Raphson reciprocal-sqrt
  - Integer softmax with a 256-entry base-2 exponent LUT
  - INT16 fixed-point RoPE (incl. IMROPE for Qwen 3.6)
  - 256-entry INT8 → INT8 activation LUTs (committed alongside weights)
  - Per-tensor / per-channel symmetric INT8 quantization with banker's
    rounding
  - SwiGLU FFN, standard + GQA attention, gated DeltaNet linear-attention,
    Mamba-style SSM
- **Composition primitives**:
  - Per-layer `LayerWeights` enum covering 5 layer flavors: DeltaNet,
    Gemma, QwenStandard, QwenHybridSsm
  - `forward_prefix` driver: embed → run layers 0..target_layer →
    optional final norm
  - Per-layer activation tile-Merkle log (`ActivationLog`) for
    proof-friendly mid-stream commitments
- **Model commitment** (`comm_W`): canonical-order weight tile-Merkle
  root + manifest hash → 32-byte commitment sensitive to every weight,
  scale, eps, LUT byte, and architecture choice (`arch_tag`,
  `feature_flags`)
- **PoW puzzle** (Phase 3 scaffolding): `mine_vi` /`verify_vi` mine a
  layer's FFN-gate output by tile-Merkle-committing the activations and
  running an FS-derived spot-check protocol on top of `ai-pow`'s tile
  hardness primitive
- **GGUF pipeline**: streaming converter from llama.cpp GGUF →
  `manifest.bin` + `weights.bin`, with on-demand dequant (peak memory
  ≈ largest single tensor); supports `qwen3_legacy`, `qwen35`, `gemma4`
  architectures
- **Synthetic prompts** (`synth_prompt`): BLAKE3-XOF Fiat-Shamir
  prompt generation from `(block_commitment, model_id)`, with reserved-
  token rejection
- **Numpy oracle**: a reference implementation in Python (`oracle/`)
  that produces byte-equal outputs to the Rust path for cross-impl
  determinism pinning

What `ai-pow-vi` deliberately does **not** include:

- Hoon-side jets (Phase 4)
- Consensus integration, mempool, RPC (Phase 5)
- The non-INT8 layer flavors (fp16, bf16, fp8) — the protocol is
  intentionally INT8-only

## Status (as of latest commit)

**Numerical state.** The crate's contract is per-tensor symmetric W8A8
with INT8 residual additions and saturating arithmetic. The full
architecture refactor is complete and an f32 reference (Python `oracle/`,
also driving the `calibrate.rs` binary) achieves **4/4 top-1 vs Ollama**
on the eval prompt set for Qwen 3.5 27B.

**Empirical INT8 result (Qwen 3.5 27B).** All literature Tier-1
interventions are implemented and tested:

| Intervention | Commit | Result vs Ollama top-1 |
|---|---|---|
| Combined matmul scales `(a × w) / out` | `e2b99f3` | 0/4 |
| Per-tensor weight scales (`ssm_a`, `ssm_dt`, `ssm_conv1d`, norm γ) | `4cdc690` | 0/4 |
| Manual scale tightening (×4 / ×8 / ×16 / ×32) | various | 0/4 |
| Percentile-99.999 calibration | `6ec1144` | 0/4 |
| SmoothQuant offline fold (Xiao 2022, α = 0.5) | `7af48e7` | 0/4 |

Across all configurations, layer outputs saturate at ±128 from layer 3
onward. The saturation is **structural**, not a plumbing bug: across 64
transformer layers, the residual stream accumulates magnitude, and once
a value hits ±128 the saturating INT8 add destroys information for every
downstream matmul. SmoothQuant Table 4 documents the same failure mode
on OPT-175B (71.6% → 32.3%) under per-tensor symmetric W8A8 with INT8
residual.

The fix in every production INT8 system (TensorRT-LLM, vLLM, llama.cpp,
PyTorch static quant) is to compute the residual in fp16/bf16 or in i32
with explicit rescale, requantizing only at chosen points — not on
every residual add. Either change is incompatible with the current
contract.

**For PoW soundness.** This is shippable: determinism is preserved,
bit-identical reproducibility across implementations works, the FFN
puzzle has end-to-end Rust + Python parity. Matching Ollama top-1 is a
separate question gated on protocol-level contract changes; see
`NEXT_STEPS.md` for the empirical analysis and `QUANT_RESEARCH.md` /
`QUANT_PROBLEM.md` for the literature survey driving the v15 attempts.

## Layout

| Path | Purpose |
|---|---|
| `src/lib.rs` | Public re-exports |
| `src/determinism.rs` | `ARCH_TAG`, `BitExactOp` — cross-arch determinism pins |
| `src/quant.rs` | `Scale`, banker's rounding, `rescale_and_requantize` |
| `src/matmul_int8.rs` | `dot_int8`, `matmul_int8`, requantization |
| `src/rmsnorm.rs`, `layernorm.rs` | Integer norms with NR reciprocal-sqrt |
| `src/softmax.rs` | Integer softmax + 256-entry exp LUT |
| `src/rope.rs` | INT16 fixed-point RoPE / IMROPE tables and application |
| `src/activation_lut.rs`, `activations.rs` | 256-entry INT8 LUTs + per-layer activation Merkle log |
| `src/attention.rs` | Standard + GQA attention, plus Gemma 4 attention variants (QK norm, sliding window, 4-norm composition) |
| `src/deltanet.rs` | Gated DeltaNet linear-attention recurrence |
| `src/ssm.rs` | Mamba-style SSM: causal 1D conv + selective state |
| `src/ffn.rs` | SwiGLU FFN with `elementwise_mul_i8` |
| `src/layer.rs` | `LayerWeights` enum + `forward_layer` (5 flavors) |
| `src/forward.rs` | `forward_prefix` end-to-end driver |
| `src/model.rs`, `src/io.rs` | `Model` struct + on-disk format (manifest.bin / weights.bin / comm_w.hex) |
| `src/layout.rs` | `BlockKind`, `ModelFamily`, `ModelLayout`, `NormType` |
| `src/comm_w.rs` | Canonical weight tile-Merkle commitment |
| `src/prompt.rs` | `synth_prompt` Fiat-Shamir prompt synthesis |
| `src/proof.rs`, `src/prover.rs`, `src/verifier.rs` | `ViProof`, `mine_vi`, `verify_vi` |
| `src/bin/vi_eval.rs` | Top-1 next-token agreement runner |
| `src/bin/gguf_convert.rs` | Streaming GGUF → Model converter (requires `gguf-convert` feature) |
| `src/bin/calibrate.rs` | Static + activation-mode scale calibration driver |
| `oracle/` | Python reference (forward, GGUF reader, calibration, multi-arch fixtures) |
| `tests/` | Unit, oracle cross-impl, qwen-mini / gemma-mini / qwen-hybrid-mini E2E, multi-arch acceptance, streaming-converter parity |
| `ROADMAP.md` | Phase-2 / 3 work log with per-commit status snapshot |
| `NEXT_STEPS.md` | Latest empirical state and structural-saturation analysis |
| `QUANT_PROBLEM.md`, `QUANT_RESEARCH.md` | Quantization problem statement + external literature survey |

## Relationship to `ai-pow`

`ai-pow-vi` depends on `ai-pow` (`Cargo.toml` path dep) and reuses:

- BLAKE3 transcript and Fiat-Shamir machinery
- Tile-Merkle commitment with sentinel padding (`ai_pow::commit`)
- INT8 tile dot-product primitive
- Difficulty-target arithmetic and `hash_le_target`

`ai-pow` is the canonical MatMul puzzle (Pearl-aligned, no model
semantics). `ai-pow-vi` puts integer transformer semantics on top:
the activations themselves become the work the chain witnesses.

## Tests

`cargo test -p ai-pow-vi` runs the per-module unit suite plus several
end-to-end integration tests. As of latest commit:

- 197 unit + 18 determinism pins + 7 oracle cross-impl
- 4 qwen-mini E2E + 3 quantized-synthetic E2E + 4 gemma-mini E2E +
  4 qwen-hybrid-mini E2E + 4 multi-arch acceptance
- 5 oracle-arch (Python), 5 `vi-eval` binary integration
- 11 streaming-Merkle + 4 streaming-writer + 3 streaming-quantize +
  6 streaming-quantize all-archs
- 1 gated real-model test (requires GGUF fixtures)

All green on aarch64; see `ROADMAP.md` for the per-commit breakdown.
