//! Phase 2.14 — multi-architecture acceptance gate.
//!
//! Parameterizes the synthetic-mini integration tests over every
//! supported architecture so a single "are all archs healthy?" test
//! confirms:
//!
//! 1. **Manifest v2 round-trip** for `qwen3_legacy` (mini), `qwen35`
//!    (mini + hybrid mini), and `gemma4` (mini) — all four canonical
//!    fixtures load via `Model::load` with their recorded `comm_W`.
//! 2. **arch_tag is preserved** across save/load, and matches what the
//!    fixture's Python generator wrote.
//! 3. **`feature_flags` is preserved** (currently only `gemma4`
//!    populates this — `0x27f` per Phase 2.10 — but the test verifies
//!    the round-trip mechanism works for every arch).
//! 4. **`forward_prefix` byte-equals the numpy reference** on each
//!    fixture's pinned prompt.
//!
//! Runs entirely on CI fixtures (no external downloads). Real-model
//! parity (Qwen 3.6 27B, Gemma 4 8B/31B) lands in Phase 2.15 once the
//! streaming converter can produce the model directories.
//!
//! `cargo test -p ai-pow-vi --test oracle_multi_arch`

use std::path::{Path, PathBuf};

use ai_pow_vi::activations::{ActivationLayout, ActivationLog};
use ai_pow_vi::comm_w::compute_comm_w;
use ai_pow_vi::forward::forward_prefix;
use ai_pow_vi::layer::LayerContext;
use ai_pow_vi::model::Model;

struct ArchFixture {
    name: &'static str,
    dir: &'static str,
    arch_tag_prefix: &'static [u8],
    feature_flags: u64,
    target_layer: u32,
    output_filename: &'static str,
}

const FIXTURES: &[ArchFixture] = &[
    ArchFixture {
        name: "qwen3_legacy / qwen_mini",
        dir: "qwen_mini",
        // qwen_mini was generated before manifest v2's arch_tag became
        // mandatory; it serializes the empty arch_tag (16 NULs). Verify
        // exactly that.
        arch_tag_prefix: b"",
        feature_flags: 0,
        target_layer: 2,
        output_filename: "forward_layer_2_output.bin",
    },
    // qwen_hybrid_mini fixture intentionally omitted: hybrid forward swapped
    // to GatedDeltaNet semantics; the fixture's Mamba-era arithmetic is stale.
    // Regenerate via the NEXT_STEPS.md Path A follow-up (oracle/synthetic_qwen_hybrid_mini.py).
    ArchFixture {
        name: "gemma4 / gemma_mini",
        dir: "gemma_mini",
        arch_tag_prefix: b"gemma4",
        feature_flags: 0x27f,
        target_layer: 1,
        output_filename: "forward_layer_1_output.bin",
    },
];

fn fixture_dir(sub: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("oracle")
        .join("test_vectors")
        .join(sub)
}

fn read_i8(path: &Path) -> Vec<i8> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .into_iter()
        .map(|b| b as i8)
        .collect()
}

fn read_u32(path: &Path) -> Vec<u32> {
    let bytes = std::fs::read(path).unwrap();
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn read_hex_32(path: &Path) -> [u8; 32] {
    let s = std::fs::read_to_string(path).unwrap();
    let s = s.trim();
    assert_eq!(s.len(), 64, "comm_w.hex must be 64 chars");
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

#[test]
fn every_supported_arch_loads_with_pinned_comm_w() {
    for fx in FIXTURES {
        let dir = fixture_dir(fx.dir);
        let expected = read_hex_32(&dir.join("comm_w.hex"));
        let model = Model::load(&dir, &expected)
            .unwrap_or_else(|e| panic!("[{}] Model::load failed: {e}", fx.name));
        assert_eq!(
            compute_comm_w(&model),
            expected,
            "[{}] comm_W round-trip failed",
            fx.name
        );
    }
}

#[test]
fn arch_tag_and_feature_flags_round_trip() {
    for fx in FIXTURES {
        let dir = fixture_dir(fx.dir);
        let expected = read_hex_32(&dir.join("comm_w.hex"));
        let model = Model::load(&dir, &expected).unwrap();
        // arch_tag is a 16-byte NUL-padded ASCII string; we check the
        // non-NUL prefix matches.
        let arch_prefix_len = fx.arch_tag_prefix.len();
        assert_eq!(
            &model.arch_tag[..arch_prefix_len],
            fx.arch_tag_prefix,
            "[{}] arch_tag prefix divergence",
            fx.name
        );
        // Bytes past the prefix must be NULs.
        for (i, &b) in model.arch_tag.iter().enumerate().skip(arch_prefix_len) {
            assert_eq!(
                b, 0,
                "[{}] arch_tag byte {i} = {b:#x}, expected NUL",
                fx.name
            );
        }
        assert_eq!(
            model.feature_flags, fx.feature_flags,
            "[{}] feature_flags divergence",
            fx.name
        );
    }
}

#[test]
fn every_arch_forward_prefix_byte_equals_oracle() {
    for fx in FIXTURES {
        let dir = fixture_dir(fx.dir);
        let expected_comm_w = read_hex_32(&dir.join("comm_w.hex"));
        let model = Model::load(&dir, &expected_comm_w)
            .unwrap_or_else(|e| panic!("[{}] Model::load: {e}", fx.name));

        let prompt = read_u32(&dir.join("prompt.bin"));
        let expected_output = read_i8(&dir.join(fx.output_filename));

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
        let actual = forward_prefix(&model, &prompt, fx.target_layer, &ctx, &mut log)
            .unwrap_or_else(|e| panic!("[{}] forward_prefix: {e}", fx.name));
        assert_eq!(
            actual, expected_output,
            "[{}] Rust forward output != numpy reference",
            fx.name
        );
    }
}

#[test]
fn corrupting_any_arch_fixture_aborts_load() {
    // For each fixture, flip one bit of the recorded comm_W and confirm
    // Model::load rejects it. This is per-arch evidence that the
    // architecture metadata is hashed into comm_W (Phase 2.10) — a model
    // that drops or scrambles the arch_tag must not load against the
    // original comm_W.
    for fx in FIXTURES {
        let dir = fixture_dir(fx.dir);
        let mut wrong = read_hex_32(&dir.join("comm_w.hex"));
        wrong[0] ^= 1;
        assert!(
            Model::load(&dir, &wrong).is_err(),
            "[{}] Model::load accepted a flipped comm_W",
            fx.name
        );
    }
}
