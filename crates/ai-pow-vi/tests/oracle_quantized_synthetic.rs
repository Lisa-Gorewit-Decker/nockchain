//! Phase 2.9.4 acceptance test: load a directory produced end-to-end
//! by `oracle/quantize_qwen.py` (which itself fed a synthetic GGUF
//! through `gguf_reader.py` + `calibrate.py`), and confirm:
//!
//! 1. `Model::load(dir, &expected_comm_w)` succeeds.
//! 2. The recovered model has the dimensions implied by the synthetic
//!    GGUF (hidden=8, intermediate=16, num_layers=2, vocab=16, seq_len=8).
//! 3. A forward_prefix runs without error on a short prompt — i.e. the
//!    quantized weights are dimensionally consistent with the manifest.
//!
//! `cargo test -p ai-pow-vi --test oracle_quantized_synthetic`

use std::path::PathBuf;

use ai_pow_vi::activations::{ActivationLayout, ActivationLog};
use ai_pow_vi::forward::forward_prefix;
use ai_pow_vi::layer::LayerContext;
use ai_pow_vi::model::Model;

fn fixture_dir() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("oracle")
        .join("test_vectors")
        .join("quantized_synthetic")
}

fn expected_comm_w() -> [u8; 32] {
    let s = std::fs::read_to_string(fixture_dir().join("comm_w.hex")).unwrap();
    let s = s.trim();
    assert_eq!(s.len(), 64);
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

#[test]
fn loads_quantize_qwen_output() {
    let dir = fixture_dir();
    let want = expected_comm_w();
    let model = Model::load(&dir, &want).expect("Model::load failed on quantize_qwen output");
    assert_eq!(model.dims.vocab, 16);
    assert_eq!(model.dims.hidden, 8);
    assert_eq!(model.dims.seq_len, 8);
    assert_eq!(model.dims.activation_tile, 2);
    assert_eq!(model.num_layers(), 2);
}

#[test]
fn forward_runs_on_quantize_qwen_output() {
    let dir = fixture_dir();
    let want = expected_comm_w();
    let model = Model::load(&dir, &want).unwrap();
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
    // Short prompt; forward through 1 layer.
    let prompt = vec![0u32, 3, 7];
    let out = forward_prefix(&model, &prompt, 1, &ctx, &mut log).expect("forward_prefix failed");
    assert_eq!(
        out.len(),
        (prompt.len() as u32 * model.dims.hidden) as usize
    );
    // Activation log has prompt-input at idx 0 and post-layer-0 input at idx 1.
    assert_eq!(log.num_layers(), 2);
}

#[test]
fn flipping_a_byte_in_weights_trips_load() {
    let dir = fixture_dir();
    let mut bytes = std::fs::read(dir.join("weights.bin")).unwrap();
    let idx = bytes.len() / 3;
    bytes[idx] ^= 0x01;
    let tmpdir =
        std::env::temp_dir().join(format!("ai_pow_vi_quantize_tamper_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir).unwrap();
    std::fs::write(
        tmpdir.join("manifest.bin"),
        std::fs::read(dir.join("manifest.bin")).unwrap(),
    )
    .unwrap();
    std::fs::write(tmpdir.join("weights.bin"), &bytes).unwrap();
    std::fs::write(
        tmpdir.join("comm_w.hex"),
        std::fs::read(dir.join("comm_w.hex")).unwrap(),
    )
    .unwrap();

    let want = expected_comm_w();
    let result = Model::load(&tmpdir, &want);
    assert!(
        result.is_err(),
        "tampered weights must trip the comm_W check"
    );
    let _ = std::fs::remove_dir_all(&tmpdir);
}
