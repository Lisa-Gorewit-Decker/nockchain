//! Phase 2.14 — `vi-eval` multi-architecture accuracy gate.
//!
//! Loads an INT8 quantized model (any supported architecture: qwen3_legacy,
//! qwen35, gemma4) via [`Model::load`], runs `forward_prefix` to the final
//! layer + final norm on each prompt in an eval set, applies a separately-
//! loaded `lm_head` to produce a logit row, takes `argmax`, and reports
//! top-1 next-token agreement against a reference set.
//!
//! Renamed from `qwen-eval` in Phase 2.14: the underlying binary was
//! already arch-agnostic (`Model::load` dispatches via `arch_tag` from
//! the manifest), but the name was Qwen-specific. Adds an optional
//! `--arch` flag that, when present, asserts the loaded model's
//! `arch_tag` matches the user-supplied value — useful when running on
//! a directory whose name doesn't make the architecture obvious.
//!
//! The reference set is a tiny JSON-ish format (one prompt per line, no
//! brackets):
//!
//! ```text
//! {"prompt": [12, 47, 803], "expected_top1": 91}
//! {"prompt": [5, 6], "expected_top1": 17}
//! ...
//! ```
//!
//! `lm_head` is not yet a field on `Model`; for now we read it from a
//! sibling file `<model_dir>/lm_head.bin` (raw `(vocab, hidden)` i8
//! row-major bytes). Phase 2.9.4's quantizer skeleton emits the file
//! when the source GGUF has an `output.weight` tensor.
//!
//! Usage:
//!
//! ```sh
//! cargo run -p ai-pow-vi --bin vi-eval -- \
//!     --model-dir /path/to/quantized \
//!     --eval     /path/to/eval.jsonl \
//!     [--lm-head /path/to/lm_head.bin] \
//!     [--arch    qwen3_legacy|qwen35|gemma4]
//! ```
//!
//! Exit code 0 if every prompt parses and produces a result; the
//! top-1 agreement number is printed to stdout. Returns nonzero on I/O
//! or shape errors but **does not** fail on low agreement — that's a
//! calibration-quality signal, not a correctness signal.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ai_pow_vi::activations::{ActivationLayout, ActivationLog};
use ai_pow_vi::forward::forward_prefix;
use ai_pow_vi::layer::LayerContext;
use ai_pow_vi::matmul_int8::matmul_int8;
use ai_pow_vi::model::{Model, Token};

#[derive(Debug)]
struct EvalRow {
    prompt: Vec<Token>,
    expected_top1: Option<Token>,
}

fn parse_eval_line(line: &str) -> Option<EvalRow> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    // Tiny hand-rolled JSON-ish: extract `prompt` array and `expected_top1`.
    let prompt_start = line.find("[")?;
    let prompt_end = line.find("]")?;
    let prompt_str = &line[prompt_start + 1..prompt_end];
    let prompt: Vec<Token> = prompt_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    let expected_top1 = if let Some(idx) = line.find("expected_top1") {
        let tail = &line[idx + "expected_top1".len()..];
        let colon = tail.find(':')?;
        let after = tail[colon + 1..].trim_start();
        // Read until a non-digit.
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if end == 0 {
            None
        } else {
            after[..end].parse::<u32>().ok()
        }
    } else {
        None
    };
    Some(EvalRow {
        prompt,
        expected_top1,
    })
}

fn read_hex_32(path: &Path) -> Option<[u8; 32]> {
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn argmax_logits(logits: &[i32]) -> Option<u32> {
    if logits.is_empty() {
        return None;
    }
    let mut best_idx = 0u32;
    let mut best_val = logits[0];
    for (i, &v) in logits.iter().enumerate().skip(1) {
        if v > best_val {
            best_val = v;
            best_idx = i as u32;
        }
    }
    Some(best_idx)
}

struct Args {
    model_dir: PathBuf,
    eval_path: PathBuf,
    lm_head_path: Option<PathBuf>,
    expected_arch: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut model_dir = None;
    let mut eval_path = None;
    let mut lm_head_path = None;
    let mut expected_arch = None;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--model-dir" => {
                model_dir = Some(PathBuf::from(
                    argv.get(i + 1).ok_or("--model-dir needs an argument")?,
                ));
                i += 2;
            }
            "--eval" => {
                eval_path = Some(PathBuf::from(
                    argv.get(i + 1).ok_or("--eval needs an argument")?,
                ));
                i += 2;
            }
            "--lm-head" => {
                lm_head_path = Some(PathBuf::from(
                    argv.get(i + 1).ok_or("--lm-head needs an argument")?,
                ));
                i += 2;
            }
            "--arch" => {
                expected_arch = Some(argv.get(i + 1).ok_or("--arch needs an argument")?.clone());
                i += 2;
            }
            "-h" | "--help" => {
                eprintln!(
                    "vi-eval --model-dir <dir> --eval <jsonl> \
                     [--lm-head <bin>] [--arch <name>]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    Ok(Args {
        model_dir: model_dir.ok_or("--model-dir required")?,
        eval_path: eval_path.ok_or("--eval required")?,
        lm_head_path,
        expected_arch,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let comm_w_path = args.model_dir.join("comm_w.hex");
    let Some(expected_comm_w) = read_hex_32(&comm_w_path) else {
        eprintln!(
            "error: cannot read or parse comm_w.hex at {}",
            comm_w_path.display()
        );
        return ExitCode::from(2);
    };

    let model = match Model::load(&args.model_dir, &expected_comm_w) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error loading model: {e}");
            return ExitCode::from(2);
        }
    };
    let arch_str: String = model
        .arch_tag
        .iter()
        .take_while(|b| **b != 0)
        .map(|&b| b as char)
        .collect();
    eprintln!(
        "loaded model: arch={} vocab={} hidden={} seq_len={} num_layers={} feature_flags=0x{:x}",
        if arch_str.is_empty() {
            "(unset)".to_string()
        } else {
            arch_str.clone()
        },
        model.dims.vocab,
        model.dims.hidden,
        model.dims.seq_len,
        model.num_layers(),
        model.feature_flags
    );

    if let Some(want) = &args.expected_arch {
        if arch_str.as_str() != want.as_str() {
            eprintln!(
                "error: --arch={} requested but model arch_tag is {:?}",
                want, arch_str
            );
            return ExitCode::from(2);
        }
    }

    let lm_head_path = args
        .lm_head_path
        .clone()
        .unwrap_or_else(|| args.model_dir.join("lm_head.bin"));
    let lm_head_bytes = match std::fs::read(&lm_head_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "warning: lm_head not found at {} ({e}); will run forward only and skip argmax",
                lm_head_path.display()
            );
            Vec::new()
        }
    };
    let expected_lm_len = (model.dims.vocab as usize) * (model.dims.hidden as usize);
    let lm_head: Vec<i8> = if lm_head_bytes.len() == expected_lm_len {
        lm_head_bytes.into_iter().map(|b| b as i8).collect()
    } else if lm_head_bytes.is_empty() {
        Vec::new()
    } else {
        eprintln!(
            "error: lm_head.bin length {} != vocab*hidden {}",
            lm_head_bytes.len(),
            expected_lm_len
        );
        return ExitCode::from(2);
    };

    let eval_text = match std::fs::read_to_string(&args.eval_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading eval set: {e}");
            return ExitCode::from(2);
        }
    };

    let mut total = 0usize;
    let mut compared = 0usize;
    let mut top1_agree = 0usize;
    let layout = ActivationLayout {
        seq_len: model.dims.seq_len,
        hidden: model.dims.hidden,
        tile: model.dims.activation_tile,
    };
    let ctx = LayerContext {
        rope_tables: &model.rope_tables,
        softmax_lut: &model.softmax_lut,
        sigmoid_lut: &model.sigmoid_lut,
        ffn_activation: &model.ffn_activation,
    };

    for (line_no, line) in eval_text.lines().enumerate() {
        let Some(row) = parse_eval_line(line) else {
            continue;
        };
        if row.prompt.is_empty() {
            eprintln!("line {}: empty prompt; skipped", line_no + 1);
            continue;
        }
        if row.prompt.len() as u32 > model.dims.seq_len {
            eprintln!(
                "line {}: prompt length {} > model seq_len {}; skipped",
                line_no + 1,
                row.prompt.len(),
                model.dims.seq_len
            );
            continue;
        }

        // Run forward to the final layer; final norm is applied iff present.
        let mut log = match ActivationLog::new(layout) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("line {}: ActivationLog::new failed: {e}", line_no + 1);
                continue;
            }
        };
        let target_layer = model.num_layers();
        let final_acts = match forward_prefix(&model, &row.prompt, target_layer, &ctx, &mut log) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("line {}: forward_prefix failed: {e}", line_no + 1);
                continue;
            }
        };

        total += 1;

        if lm_head.is_empty() {
            // Forward succeeded but we have no head; cannot compute top-1.
            continue;
        }

        // Take the LAST token's hidden vector and project through lm_head.
        // lm_head shape: (vocab, hidden) row-major. Each output logit is the
        // dot product of the last-token row with the corresponding vocab
        // row. Reformulating as matmul B^T: matmul_int8 wants B col-major, so
        // we treat lm_head (already (vocab, hidden) row-major) as the
        // "B col-major" form with k=hidden, n=vocab.
        let last_off = (row.prompt.len() - 1) * (model.dims.hidden as usize);
        let last_vec = &final_acts[last_off..last_off + model.dims.hidden as usize];
        let mut logits = vec![0i32; model.dims.vocab as usize];
        if let Err(e) = matmul_int8(
            last_vec, &lm_head, 1, model.dims.hidden, model.dims.vocab, &mut logits,
        ) {
            eprintln!("line {}: lm_head matmul failed: {e}", line_no + 1);
            continue;
        }
        let predicted = match argmax_logits(&logits) {
            Some(p) => p,
            None => continue,
        };
        // Print every prediction so a caller can pipe stdout to a top-1
        // comparison without requiring `expected_top1` to be embedded in
        // the eval set up front.
        let top_logit = logits.get(predicted as usize).copied().unwrap_or(i32::MIN);
        let prompt_str: Vec<String> = row.prompt.iter().map(|t| t.to_string()).collect();
        println!(
            "prediction\tline={}\tprompt=[{}]\tpredicted_top1={}\ttop_logit={}",
            line_no + 1,
            prompt_str.join(","),
            predicted,
            top_logit,
        );
        if let Some(want) = row.expected_top1 {
            compared += 1;
            if predicted == want {
                top1_agree += 1;
            }
        }
    }

    println!("prompts_run\t{total}");
    if compared == 0 {
        println!("top1_agreement\tn/a (no expected_top1 fields supplied; predictions printed above)");
    } else {
        let pct = 100.0 * (top1_agree as f64) / (compared as f64);
        println!("top1_agreement\t{top1_agree}/{compared}\t{:.1}%", pct);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_line() {
        let row = parse_eval_line("{\"prompt\": [1, 2, 3], \"expected_top1\": 42}").unwrap();
        assert_eq!(row.prompt, vec![1u32, 2, 3]);
        assert_eq!(row.expected_top1, Some(42));
    }

    #[test]
    fn parse_no_expected_field() {
        let row = parse_eval_line("{\"prompt\": [5, 6, 7]}").unwrap();
        assert_eq!(row.prompt, vec![5u32, 6, 7]);
        assert!(row.expected_top1.is_none());
    }

    #[test]
    fn parse_blank_and_comment() {
        assert!(parse_eval_line("").is_none());
        assert!(parse_eval_line("   ").is_none());
        assert!(parse_eval_line("# this is a comment").is_none());
    }

    #[test]
    fn argmax_picks_largest() {
        assert_eq!(argmax_logits(&[1, 5, 3, 2]), Some(1));
        assert_eq!(argmax_logits(&[10, 1, 1]), Some(0));
        assert_eq!(argmax_logits(&[]), None);
    }

    #[test]
    fn argmax_breaks_ties_to_first() {
        // Strict `>` in the loop means the first occurrence wins.
        assert_eq!(argmax_logits(&[5, 5, 5]), Some(0));
    }
}
