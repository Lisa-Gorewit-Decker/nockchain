//! Phase 2.9.6 — gated real-model integration test.
//!
//! Skipped by default (`#[ignore]`). To run, point the env var
//! `NOCKCHAIN_VI_QWEN_DIR` at a directory written by
//! `oracle/quantize_qwen.py` for a real Qwen 3.6 27B (or any
//! pipeline-compatible) model, and `NOCKCHAIN_VI_QWEN_FIXTURE` at a
//! directory written by `oracle/forward_prefix_oracle.py` that
//! contains:
//!
//! ```text
//! $NOCKCHAIN_VI_QWEN_FIXTURE/
//!   meta.txt              # target_layer=N prompt_len=M
//!   prompt.bin            # M LE u32 token ids
//!   activations_layer_N.bin   # M * hidden i8 — the numpy oracle's output
//! ```
//!
//! Then:
//!
//! ```sh
//! NOCKCHAIN_VI_QWEN_DIR=/path/to/quantized \
//! NOCKCHAIN_VI_QWEN_FIXTURE=/path/to/fixture \
//!     cargo test -p ai-pow-vi --test oracle_qwen -- --ignored
//! ```
//!
//! The Rust forward output must byte-equal the numpy oracle's output.
//! Any divergence indicates a Rust ↔ numpy spec drift or a quantizer
//! bug.

use std::path::PathBuf;

use ai_pow_vi::activations::{ActivationLayout, ActivationLog};
use ai_pow_vi::forward::forward_prefix;
use ai_pow_vi::layer::LayerContext;
use ai_pow_vi::model::Model;

const ENV_MODEL_DIR: &str = "NOCKCHAIN_VI_QWEN_DIR";
const ENV_FIXTURE_DIR: &str = "NOCKCHAIN_VI_QWEN_FIXTURE";

fn read_hex_32(path: &std::path::Path) -> [u8; 32] {
    let s = std::fs::read_to_string(path).unwrap();
    let s = s.trim();
    assert_eq!(s.len(), 64, "comm_w.hex must be 64 chars");
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

fn read_meta(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    let s = std::fs::read_to_string(path).unwrap();
    let mut m = std::collections::HashMap::new();
    for kv in s.trim().split_whitespace() {
        if let Some((k, v)) = kv.split_once('=') {
            m.insert(k.to_string(), v.to_string());
        }
    }
    m
}

fn require_env(var: &str) -> Option<PathBuf> {
    match std::env::var(var) {
        Ok(s) => Some(PathBuf::from(s)),
        Err(_) => {
            eprintln!(
                "skipping: {var} is not set; this test only runs in a workspace \
                 with a real quantized Qwen model + fixture available."
            );
            None
        }
    }
}

#[test]
#[ignore]
fn oracle_qwen_forward_prefix_byte_equal() {
    let Some(model_dir) = require_env(ENV_MODEL_DIR) else {
        return;
    };
    let Some(fixture_dir) = require_env(ENV_FIXTURE_DIR) else {
        return;
    };

    let expected_comm_w = read_hex_32(&model_dir.join("comm_w.hex"));
    let model =
        Model::load(&model_dir, &expected_comm_w).expect("Model::load failed; comm_W mismatch");

    let meta = read_meta(&fixture_dir.join("meta.txt"));
    let target_layer: u32 = meta
        .get("target_layer")
        .expect("fixture meta missing target_layer")
        .parse()
        .unwrap();

    let prompt_bytes = std::fs::read(fixture_dir.join("prompt.bin")).unwrap();
    let prompt: Vec<u32> = prompt_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    assert!(!prompt.is_empty(), "fixture prompt is empty");

    let want_path = fixture_dir.join(format!("activations_layer_{target_layer}.bin"));
    let want_bytes = std::fs::read(&want_path).unwrap_or_else(|e| {
        panic!(
            "fixture missing {}: {e}; regenerate via oracle/forward_prefix_oracle.py",
            want_path.display()
        )
    });
    let want: Vec<i8> = want_bytes.into_iter().map(|b| b as i8).collect();

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
    let actual = forward_prefix(&model, &prompt, target_layer, &ctx, &mut log)
        .expect("forward_prefix failed");

    if actual.len() != want.len() {
        panic!(
            "length mismatch: rust={} oracle={}",
            actual.len(),
            want.len()
        );
    }

    let mut first_mismatch: Option<(usize, i8, i8)> = None;
    let mut mismatches = 0usize;
    for (i, (a, b)) in actual.iter().zip(want.iter()).enumerate() {
        if a != b {
            mismatches += 1;
            if first_mismatch.is_none() {
                first_mismatch = Some((i, *a, *b));
            }
        }
    }
    assert_eq!(
        mismatches,
        0,
        "Rust output diverges from numpy oracle in {mismatches}/{} bytes; \
         first mismatch at index {:?}",
        actual.len(),
        first_mismatch
    );
}
