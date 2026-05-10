# ai-pow-vi oracle

Independent reference implementation of the Phase 2 integer ops, used to
generate test fixtures that the Rust crate verifies byte-for-byte. Not
consensus code — these scripts only run during development / CI to seed
or refresh `oracle/test_vectors/`.

The oracle exists to catch two classes of bugs the Rust-only determinism
pins cannot:

1. **Spec-vs-implementation drift.** The pins prove `ai-pow-vi` is
   deterministic across architectures, but they do not prove it implements
   the *intended* spec. The numpy reference is a second implementation
   that re-derives the same outputs from the same canonical inputs.
2. **Real-model regression.** `extract_weights.py` (skeleton) takes a
   Hugging Face / GGUF model, requantizes to our INT8 layout, and runs a
   prefix-to-layer-K forward, dumping activations. The fixture lets
   `tests/oracle_real.rs` assert the full prover stack matches a
   third-party reference on a non-toy model.

## Workflow

```
# One-time setup
python3 -m venv .venv
. .venv/bin/activate
pip install -r oracle/requirements.txt

# Regenerate the synthetic op fixtures (fast, no model download)
python oracle/synthetic_fixture.py

# Generate a Gemma / Qwen real-model fixture (large download; 30+ min)
python oracle/extract_weights.py --model qwen3.6-27b --out oracle/test_vectors/qwen_layer_8
python oracle/forward_prefix_oracle.py --vectors oracle/test_vectors/qwen_layer_8 --layer 8

# Run the Rust side (loads everything in oracle/test_vectors/ and asserts)
cargo test -p ai-pow-vi --test oracle_op_vectors
```

## Layout

- `reference_ops.py` — numpy implementations of `round_half_to_even_div_pow2`,
  `rescale_and_requantize`, `dot_int8`, `matmul_int8`, `requantize_vec`,
  `rmsnorm`, `layernorm`, `softmax_int`, `ffn_forward`. All integer-only,
  using arbitrary-precision Python ints internally so wrap/overflow
  behavior is explicit.
- `synth_prompt_oracle.py` — Python reference of the BLAKE3-XOF prompt
  synthesis. Mirrors `crate::prompt::synth_prompt`.
- `synthetic_fixture.py` — drives `reference_ops` over the same canonical
  LCG inputs the Rust pins use; writes `test_vectors/<op>/`.
- `extract_weights.py` — **skeleton**. Reads HF safetensors weights for
  a target model, computes per-tensor symmetric INT8 scales, requantizes,
  and writes our canonical `weights.bin` + `manifest.json`. Currently
  raises `NotImplementedError` for the dequantize path; the user fills in
  per-model details (architecture mapping, tokenizer integration).
- `forward_prefix_oracle.py` — **skeleton**. Loads the requantized model
  and runs a forward pass to a target layer using `reference_ops`,
  dumping per-layer activations.

## Determinism contract

Every numpy op in `reference_ops.py` MUST produce byte-identical output
to the Rust function it mirrors when given the same inputs. If a fixture
ever drifts, exactly one of two things is true:

1. The Rust impl changed (and the fixture should be regenerated).
2. The numpy impl drifted from the spec (and the bug is in
   `reference_ops.py`).

Both cases want a code review and an explicit fixture refresh; the test
makes the change loud rather than silent.

## File formats

- `*.bin` files are raw little-endian byte streams. `i8` tensors are
  written as `np.int8.tobytes()`; `i32` tensors as `np.int32.tobytes()`
  (LE on every supported platform).
- `meta.txt` is a one-line shape summary so a reader can sanity-check
  the file size: e.g. `m=4 k=8 n=4 dtype=i32`.

The Rust loader picks file lengths from the meta + dtype, never trusts
embedded length prefixes (so a corrupted `meta.txt` triggers a length
mismatch rather than silently mis-decoding).
