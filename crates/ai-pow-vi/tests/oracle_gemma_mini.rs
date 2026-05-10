//! Phase 2.11 — synthetic Gemma 4 end-to-end test.
//!
//! Same shape as `oracle_qwen_mini.rs` but for the Gemma variant of
//! `LayerWeights`. Loads `oracle/test_vectors/gemma_mini/` via
//! `Model::load`, runs `forward_prefix` with the same prompt, and
//! asserts byte-equality with the numpy reference's saved output.
//!
//! What this proves end-to-end:
//! 1. Manifest v2 with `arch_tag = "gemma4"` and `feature_flags = 0x27f`
//!    round-trips through Python encode → Rust load with matching
//!    `comm_W`.
//! 2. The Gemma layer tag (2) parses correctly into the
//!    `LayerWeights::Gemma` variant, with both layers (one full
//!    attention, one sliding-window radius=4) carrying their
//!    Q/K-norm gammas, post-attn norm, post-ffn norm, inp_gate, and
//!    layer_output_scale.
//! 3. The numpy reference `forward_gemma_layer` and Rust
//!    `forward_gemma_layer` produce byte-equal outputs.
//!
//! `cargo test -p ai-pow-vi --test oracle_gemma_mini`

use std::path::{Path, PathBuf};

use ai_pow_vi::activations::{ActivationLayout, ActivationLog};
use ai_pow_vi::comm_w::compute_comm_w;
use ai_pow_vi::forward::forward_prefix;
use ai_pow_vi::layer::LayerContext;
use ai_pow_vi::model::Model;

fn vectors_dir() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("oracle")
        .join("test_vectors")
        .join("gemma_mini")
}

fn read_i8(path: &Path) -> Vec<i8> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .into_iter()
        .map(|b| b as i8)
        .collect()
}

fn read_u32(path: &Path) -> Vec<u32> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn read_hex_32(path: &Path) -> [u8; 32] {
    let s = std::fs::read_to_string(path).unwrap();
    let s = s.trim();
    assert_eq!(s.len(), 64);
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

#[test]
fn gemma_mini_load_succeeds_with_recorded_comm_w() {
    let dir = vectors_dir();
    let expected = read_hex_32(&dir.join("comm_w.hex"));
    let model = Model::load(&dir, &expected).expect("Model::load failed");
    assert_eq!(compute_comm_w(&model), expected);
    // arch_tag should match what Python wrote.
    assert_eq!(&model.arch_tag[..6], b"gemma4");
    // feature_flags should match what Python wrote (0x27f from synthetic_gemma_mini.py).
    assert_eq!(model.feature_flags, 0x27f);
}

#[test]
fn gemma_mini_load_rejects_wrong_comm_w() {
    let dir = vectors_dir();
    let mut wrong = read_hex_32(&dir.join("comm_w.hex"));
    wrong[0] ^= 1;
    assert!(Model::load(&dir, &wrong).is_err());
}

#[test]
fn gemma_mini_forward_prefix_byte_equal_to_oracle() {
    let dir = vectors_dir();
    let expected_comm_w = read_hex_32(&dir.join("comm_w.hex"));
    let model = Model::load(&dir, &expected_comm_w).unwrap();

    let prompt = read_u32(&dir.join("prompt.bin"));
    let expected_output = read_i8(&dir.join("forward_layer_1_output.bin"));

    let layout = ActivationLayout {
        seq_len: model.dims.seq_len,
        hidden: model.dims.hidden,
        tile: model.dims.activation_tile,
    };
    let mut log = ActivationLog::new(layout).unwrap();
    let ctx = LayerContext {
        rope_tables: &model.rope_tables,
        softmax_lut: &model.softmax_lut,
        sigmoid_lut: &model.sigmoid_lut,
        ffn_activation: &model.ffn_activation,
    };
    let actual_output =
        forward_prefix(&model, &prompt, 1, &ctx, &mut log).expect("forward_prefix failed");

    assert_eq!(
        actual_output, expected_output,
        "Rust Gemma forward does not match numpy reference"
    );

    // Also validate the recorded activation roots match what the
    // numpy reference would produce.
    for layer_idx in 0..=1u32 {
        let path = dir.join(format!("activation_layer_{layer_idx}.bin"));
        let want = read_i8(&path);
        let mut sanity_log = ActivationLog::new(layout).unwrap();
        sanity_log.record_layer(0, &want).unwrap();
        let oracle_root = sanity_log.root(0).unwrap();
        let rust_root = log.root(layer_idx).unwrap();
        assert_eq!(
            rust_root, oracle_root,
            "Gemma mini activation root divergence at layer {layer_idx}"
        );
    }
}

#[test]
fn gemma_mini_has_two_gemma_layers_one_with_sliding_window() {
    use ai_pow_vi::layer::LayerWeights;
    let dir = vectors_dir();
    let expected = read_hex_32(&dir.join("comm_w.hex"));
    let model = Model::load(&dir, &expected).unwrap();
    assert_eq!(model.num_layers(), 2);
    let mut saw_sliding = false;
    for layer in &model.layers {
        match layer {
            LayerWeights::Gemma { sliding_window, .. } => {
                if sliding_window.is_some() {
                    saw_sliding = true;
                }
            }
            _ => panic!("expected all Gemma layers; got {:?}", layer),
        }
    }
    assert!(
        saw_sliding,
        "expected at least one Gemma layer with sliding_window=Some(...)"
    );
}
