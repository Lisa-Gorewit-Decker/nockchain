//! Helper that recomputes `comm_W` for every fixture under
//! `oracle/test_vectors/` using the current Rust implementation, and
//! optionally rewrites the matching `comm_w.hex`.
//!
//! Used to refresh fixtures after a numerical or context-string change
//! in `ai-pow::commit` or `ai-pow-vi::comm_w`. The first such refresh
//! was triggered by the `ai-pow` Merkle-context bump v1 → v3 during
//! the Pearl alignment work.
//!
//! Run with:
//!
//!   # dry run — just print old vs new hex for each fixture
//!   cargo test -p ai-pow-vi --test recompute_fixture_comm_w \
//!       -- --ignored --nocapture
//!
//!   # write the new hex into each `comm_w.hex`
//!   WRITE=1 cargo test -p ai-pow-vi --test recompute_fixture_comm_w \
//!       -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::{env, fs};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> Vec<PathBuf> {
    let root = workspace_root().join("oracle/test_vectors");
    ["qwen_mini", "qwen_hybrid_mini", "gemma_mini", "quantized_synthetic"]
        .iter()
        .map(|name| root.join(name))
        .filter(|p| p.exists())
        .collect()
}

#[test]
#[ignore = "manual fixture refresh; run with --ignored --nocapture (and optionally WRITE=1)"]
fn recompute_all() {
    let write = env::var("WRITE").is_ok();
    for dir in fixtures() {
        let weights_path = dir.join("weights.bin");
        let manifest_path = dir.join("manifest.bin");
        let comm_w_path = dir.join("comm_w.hex");

        if !weights_path.exists() || !manifest_path.exists() {
            println!("SKIP {} (missing inputs)", dir.display());
            continue;
        }

        match recompute_comm_w(&manifest_path, &weights_path) {
            Ok(new) => {
                let new_hex = hex_encode(&new);
                let old = fs::read_to_string(&comm_w_path)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                println!(
                    "{:>22}: old={} new={} {}",
                    dir.file_name().unwrap().to_string_lossy(),
                    &old.chars().take(16).collect::<String>(),
                    &new_hex.chars().take(16).collect::<String>(),
                    if old == new_hex {
                        "(unchanged)"
                    } else {
                        "(changed)"
                    }
                );
                if write && old != new_hex {
                    fs::write(&comm_w_path, &new_hex).unwrap();
                    println!("                       → wrote {}", comm_w_path.display());
                }
            }
            Err(e) => {
                println!(
                    "{:>22}: SKIP — parse failed ({})",
                    dir.file_name().unwrap().to_string_lossy(),
                    e
                );
            }
        }
    }
}

fn recompute_comm_w(manifest_path: &Path, weights_path: &Path) -> Result<[u8; 32], String> {
    use ai_pow_vi::io::{parse_manifest_for_test, parse_weights_for_test};
    let manifest_bytes = fs::read(manifest_path).map_err(|e| format!("read manifest: {e}"))?;
    let parsed =
        parse_manifest_for_test(&manifest_bytes).map_err(|e| format!("parse_manifest: {e:?}"))?;
    let weights_bytes = fs::read(weights_path).map_err(|e| format!("read weights: {e}"))?;
    let model = parse_weights_for_test(parsed, &weights_bytes)
        .map_err(|e| format!("parse_weights: {e:?}"))?;
    Ok(ai_pow_vi::compute_comm_w(&model))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}
