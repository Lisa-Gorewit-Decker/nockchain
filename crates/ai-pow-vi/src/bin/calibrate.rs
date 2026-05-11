//! Phase 2.9.3+ — Rust f32 activation calibrator (Phase A in the
//! plan).
//!
//! Walks a GGUF model in f32, runs a reference forward over a
//! calibration prompt set, records `max(|x|)` at every quantization
//! tap point in the INT8 forward, and emits a `scales.json` consumable
//! by `gguf-convert --scales`.
//!
//! Why a Rust port of `oracle/calibrate.py`: the Python version's
//! activation mode is stubbed because numpy is too slow to do an f32
//! forward over Qwen 3.6 27B in reasonable wall-clock. This binary
//! uses candle-core's SIMD-optimized k-quants kernels for the same
//! GGUF read path the converter uses, and Vec<f32>/manual loops for
//! the forward — fast enough to run on real Qwen 3.6 27B in tens of
//! minutes on a single Mac.
//!
//! First cut: calibrates the 48 standard (full-attention) blocks of
//! Qwen 3.6 27B end-to-end. The 16 hybrid (GatedDeltaNet) blocks are
//! treated as residual passthrough — their f32 forward is a separate
//! piece of work tied to the deeper hybrid-arch fix. The output
//! scales.json still includes default keys for SSM tap points; the
//! converter will fall back to the global default for those.
//!
//! Usage:
//!
//!   cargo run --release -p ai-pow-vi --bin calibrate \
//!       --features gguf-convert -- \
//!       --gguf /path/to/model.gguf \
//!       --prompts /path/to/prompts.jsonl \
//!       --out /tmp/scales.json
//!
//! `prompts.jsonl` shares the format `vi-eval` uses — one
//! `{"prompt": [tok, ...]}` per line. `expected_top1` is optional and
//! ignored.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use candle_core::quantized::gguf_file::{Content, Value};
use candle_core::Device;

const SCALE_DENOM_LOG2: u32 = 15;
const DEFAULT_NORM_EPS: f32 = 1e-6;

struct Args {
    gguf: PathBuf,
    prompts: PathBuf,
    out: PathBuf,
    seq_len_cap: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut gguf = None;
    let mut prompts = None;
    let mut out = None;
    let mut seq_len_cap: usize = 64;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--gguf" => gguf = Some(PathBuf::from(it.next().ok_or("--gguf requires a value")?)),
            "--prompts" => {
                prompts = Some(PathBuf::from(it.next().ok_or("--prompts requires a value")?))
            }
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out requires a value")?)),
            "--seq-len-cap" => {
                seq_len_cap = it
                    .next()
                    .ok_or("--seq-len-cap requires a value")?
                    .parse()
                    .map_err(|e| format!("--seq-len-cap parse: {e}"))?
            }
            "-h" | "--help" => {
                return Err(
                    "calibrate --gguf <path> --prompts <jsonl> --out <scales.json> \
                     [--seq-len-cap N]"
                        .into(),
                )
            }
            other => return Err(format!("unknown arg {other}")),
        }
    }
    Ok(Args {
        gguf: gguf.ok_or("--gguf required")?,
        prompts: prompts.ok_or("--prompts required")?,
        out: out.ok_or("--out required")?,
        seq_len_cap,
    })
}

// ─── GGUF metadata helpers (mirror gguf_convert.rs) ──────────────────────────

fn arch_str_from_content(content: &Content) -> Result<String, String> {
    match content
        .metadata
        .get("general.architecture")
        .ok_or("GGUF missing general.architecture")?
    {
        Value::String(s) => Ok(s.clone()),
        v => Err(format!("general.architecture not string: {v:?}")),
    }
}

fn meta_u32(content: &Content, key: &str, default: Option<u32>) -> Result<u32, String> {
    match content.metadata.get(key) {
        Some(Value::U32(v)) => Ok(*v),
        Some(Value::U64(v)) => Ok(*v as u32),
        Some(Value::I32(v)) => Ok(*v as u32),
        Some(Value::Array(arr)) => {
            let mut best = 0u32;
            for v in arr {
                let cand = match v {
                    Value::U32(x) => *x,
                    Value::U64(x) => *x as u32,
                    Value::I32(x) => *x as u32,
                    _ => continue,
                };
                if cand > best {
                    best = cand;
                }
            }
            if best > 0 {
                Ok(best)
            } else {
                default.ok_or_else(|| format!("{key}: array of zeros and no default"))
            }
        }
        Some(v) => Err(format!("{key}: unexpected type {v:?}")),
        None => default.ok_or_else(|| format!("missing required field {key}")),
    }
}

fn meta_f32(content: &Content, key: &str, default: Option<f32>) -> Result<f32, String> {
    match content.metadata.get(key) {
        Some(Value::F32(v)) => Ok(*v),
        Some(Value::F64(v)) => Ok(*v as f32),
        Some(v) => Err(format!("{key}: unexpected type {v:?}")),
        None => default.ok_or_else(|| format!("missing required field {key}")),
    }
}

struct QwenDims {
    hidden: usize,
    intermediate: usize,
    num_layers: usize,
    num_q_heads: usize,
    num_kv_heads: usize,
    head_dim: usize,
    rope_theta: f32,
}

fn read_qwen35_dims(content: &Content) -> Result<QwenDims, String> {
    Ok(QwenDims {
        hidden: meta_u32(content, "qwen35.embedding_length", None)? as usize,
        intermediate: meta_u32(content, "qwen35.feed_forward_length", None)? as usize,
        num_layers: meta_u32(content, "qwen35.block_count", None)? as usize,
        num_q_heads: meta_u32(content, "qwen35.attention.head_count", None)? as usize,
        num_kv_heads: meta_u32(content, "qwen35.attention.head_count_kv", None)? as usize,
        head_dim: meta_u32(content, "qwen35.attention.key_length", None)? as usize,
        rope_theta: meta_f32(content, "qwen35.rope.freq_base", Some(10_000.0))?,
    })
}

fn qwen35_block_is_standard(content: &Content, n: usize) -> bool {
    content
        .tensor_infos
        .contains_key(&format!("blk.{n}.attn_q.weight"))
}

fn dequant_to_vec_f32(
    content: &Content,
    file: &mut std::fs::File,
    name: &str,
) -> Result<(Vec<f32>, Vec<usize>), String> {
    let info = content
        .tensor(file, name, &Device::Cpu)
        .map_err(|e| format!("tensor {name}: {e}"))?;
    let shape = info.shape().dims().to_vec();
    let t = info
        .dequantize(&Device::Cpu)
        .map_err(|e| format!("dequant {name}: {e}"))?;
    let f = t
        .flatten_all()
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(|e| format!("flatten {name}: {e}"))?;
    Ok((f, shape))
}

// ─── Tap accumulator ─────────────────────────────────────────────────────────

struct ScaleAcc {
    inner: HashMap<String, f32>,
}

impl ScaleAcc {
    fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }
    fn record(&mut self, tap: &str, x: &[f32]) {
        if x.is_empty() {
            return;
        }
        let mut m: f32 = 0.0;
        for &v in x {
            let av = v.abs();
            if av > m {
                m = av;
            }
        }
        let e = self.inner.entry(tap.to_string()).or_insert(0.0);
        if m > *e {
            *e = m;
        }
    }
    fn merge_default(&mut self, key: &str, fallback: f32) {
        self.inner.entry(key.to_string()).or_insert(fallback);
    }
}

fn f32_to_scale_num(max_abs: f32) -> i32 {
    if !max_abs.is_finite() || max_abs <= 0.0 {
        return 1;
    }
    let scale = max_abs / 127.0;
    let raw = (scale * ((1u64 << SCALE_DENOM_LOG2) as f32)).round();
    raw.clamp(1.0, i32::MAX as f32) as i32
}

// ─── F32 primitives ──────────────────────────────────────────────────────────

/// `out[m * out_dim + j] = sum_k x[m * in_dim + k] * w[j * in_dim + k]`.
/// `w` is row-major (out_dim, in_dim). Standard linear weight layout.
fn matmul_f32(x: &[f32], w: &[f32], m: usize, in_dim: usize, out_dim: usize) -> Vec<f32> {
    let mut out = vec![0f32; m * out_dim];
    for row in 0..m {
        let x_off = row * in_dim;
        let o_off = row * out_dim;
        for j in 0..out_dim {
            let w_off = j * in_dim;
            let mut s: f32 = 0.0;
            for k in 0..in_dim {
                s += x[x_off + k] * w[w_off + k];
            }
            out[o_off + j] = s;
        }
    }
    out
}

/// Per-token RMSNorm: `y = x / rms(x) * gamma`, where `rms(x) = sqrt(mean(x^2) + eps)`.
fn rms_norm_f32(x: &[f32], gamma: &[f32], m: usize, hidden: usize, eps: f32) -> Vec<f32> {
    let mut out = vec![0f32; m * hidden];
    for row in 0..m {
        let off = row * hidden;
        let mut sumsq: f32 = 0.0;
        for k in 0..hidden {
            sumsq += x[off + k] * x[off + k];
        }
        let inv = (sumsq / (hidden as f32) + eps).sqrt().recip();
        for k in 0..hidden {
            out[off + k] = x[off + k] * inv * gamma[k];
        }
    }
    out
}

/// Per-head RMSNorm: normalize over `head_dim` (last axis), gamma is shared per head.
fn head_rms_norm_f32(
    x: &[f32],
    gamma: &[f32],
    m: usize,
    num_heads: usize,
    head_dim: usize,
    eps: f32,
) -> Vec<f32> {
    let total = m * num_heads * head_dim;
    let mut out = vec![0f32; total];
    for row in 0..m {
        for h in 0..num_heads {
            let off = (row * num_heads + h) * head_dim;
            let mut sumsq: f32 = 0.0;
            for k in 0..head_dim {
                sumsq += x[off + k] * x[off + k];
            }
            let inv = (sumsq / (head_dim as f32) + eps).sqrt().recip();
            for k in 0..head_dim {
                out[off + k] = x[off + k] * inv * gamma[k];
            }
        }
    }
    out
}

fn silu(x: f32) -> f32 {
    x / (1.0 + (-x).exp())
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Causal in-place self-attention from per-token Q,K,V buffers. Outputs
/// `out` of shape `(m, num_q_heads * head_dim)`.
///
/// Q layout: `(m, num_q_heads, head_dim)`.
/// K, V layout: `(m, num_kv_heads, head_dim)`.
/// Returns the per-head intermediate vector AND the pre-softmax score
/// max-abs for tap recording.
fn attention_core_f32(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    m: usize,
    num_q_heads: usize,
    num_kv_heads: usize,
    head_dim: usize,
) -> (Vec<f32>, f32, f32) {
    let kv_groups = num_q_heads / num_kv_heads;
    let inv_sqrt_dh = (head_dim as f32).powf(-0.5);
    let mut out = vec![0f32; m * num_q_heads * head_dim];
    let mut max_score: f32 = 0.0;
    let mut max_attn_out: f32 = 0.0;
    for h in 0..num_q_heads {
        let kv_h = h / kv_groups;
        for t in 0..m {
            let q_off = (t * num_q_heads + h) * head_dim;
            // Causal score row: scores[s] = q · k[s] / sqrt(dh) for s <= t.
            let mut scores = vec![f32::NEG_INFINITY; m];
            for s in 0..=t {
                let k_off = (s * num_kv_heads + kv_h) * head_dim;
                let mut d: f32 = 0.0;
                for j in 0..head_dim {
                    d += q[q_off + j] * k[k_off + j];
                }
                let z = d * inv_sqrt_dh;
                scores[s] = z;
                let az = z.abs();
                if az > max_score {
                    max_score = az;
                }
            }
            // Softmax over scores[0..=t].
            let mut max_s = f32::NEG_INFINITY;
            for s in 0..=t {
                if scores[s] > max_s {
                    max_s = scores[s];
                }
            }
            let mut denom: f32 = 0.0;
            for s in 0..=t {
                scores[s] = (scores[s] - max_s).exp();
                denom += scores[s];
            }
            let inv_denom = if denom > 0.0 { 1.0 / denom } else { 0.0 };
            for s in 0..=t {
                scores[s] *= inv_denom;
            }
            // attn_out = sum_s scores[s] * V[s, kv_h, :]
            let out_off = (t * num_q_heads + h) * head_dim;
            for s in 0..=t {
                let v_off = (s * num_kv_heads + kv_h) * head_dim;
                for j in 0..head_dim {
                    out[out_off + j] += scores[s] * v[v_off + j];
                }
            }
            for j in 0..head_dim {
                let av = out[out_off + j].abs();
                if av > max_attn_out {
                    max_attn_out = av;
                }
            }
        }
    }
    (out, max_score, max_attn_out)
}

/// In-place RoPE: rotate each head's `(d0, d1)` pair by `(pos * inv_freq[j])`
/// where `inv_freq[j] = rope_theta^(-2j/head_dim)`. Standard half-split.
fn apply_rope_f32(
    x: &mut [f32],
    m: usize,
    num_heads: usize,
    head_dim: usize,
    rope_theta: f32,
) {
    let half = head_dim / 2;
    for t in 0..m {
        for h in 0..num_heads {
            let off = (t * num_heads + h) * head_dim;
            for j in 0..half {
                let inv_freq = (rope_theta as f64).powf(-2.0 * (j as f64) / head_dim as f64);
                let theta = (t as f64) * inv_freq;
                let c = theta.cos() as f32;
                let s = theta.sin() as f32;
                let a = x[off + j];
                let b = x[off + half + j];
                x[off + j] = a * c - b * s;
                x[off + half + j] = a * s + b * c;
            }
        }
    }
}

// ─── Standard-layer forward, with tap recording ──────────────────────────────

#[allow(clippy::too_many_arguments)]
fn forward_std_layer(
    n: usize,
    input: &[f32], // (m, hidden)
    m: usize,
    dims: &QwenDims,
    content: &Content,
    file: &mut std::fs::File,
    acc: &mut ScaleAcc,
) -> Result<Vec<f32>, String> {
    let h = dims.hidden;
    let prefix = format!("blk.{n}");

    // Norms.
    let (norm1_g, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_norm.weight"))?;
    let (norm2_g, _) = dequant_to_vec_f32(
        content,
        file,
        &format!("{prefix}.post_attention_norm.weight"),
    )?;

    // Q (possibly with packed gate), K, V, O.
    let (w_q_f, w_q_shape) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_q.weight"))?;
    let q_proj_out = w_q_shape[0];
    let q_target = dims.num_q_heads * dims.head_dim;
    let q_has_gate = if q_proj_out == 2 * q_target {
        true
    } else if q_proj_out == q_target {
        false
    } else {
        return Err(format!(
            "layer {n} attn_q out dim {q_proj_out} not q_target {q_target} nor 2*q_target",
        ));
    };
    let q_eff_out = q_proj_out;
    let (w_k_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_k.weight"))?;
    let (w_v_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_v.weight"))?;
    let (w_o_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_output.weight"))?;

    // Optional Q/K norm tensors.
    let q_norm_gamma = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_q_norm.weight"))
        .map(|(f, _)| f)
        .ok();
    let k_norm_gamma = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_k_norm.weight"))
        .map(|(f, _)| f)
        .ok();

    // FFN.
    let (w_gate_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_gate.weight"))?;
    let (w_up_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_up.weight"))?;
    let (w_down_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_down.weight"))?;

    // ── Attention sub-block ──────────────────────────────────────────────
    let normed1 = rms_norm_f32(input, &norm1_g, m, h, DEFAULT_NORM_EPS);
    acc.record(&format!("layer[{n}].norm_post.1"), &normed1);

    let q_proj = matmul_f32(&normed1, &w_q_f, m, h, q_eff_out);
    let k_proj = matmul_f32(&normed1, &w_k_f, m, h, dims.num_kv_heads * dims.head_dim);
    let v_proj = matmul_f32(&normed1, &w_v_f, m, h, dims.num_kv_heads * dims.head_dim);

    // If q_has_gate, split per-head into Q and gate halves.
    let (q_only, gate_only) = if q_has_gate {
        let mut q_only = vec![0f32; m * q_target];
        let mut gate_only = vec![0f32; m * q_target];
        let hd = dims.head_dim;
        for t in 0..m {
            for hi in 0..dims.num_q_heads {
                let proj_base = t * q_eff_out + hi * 2 * hd;
                let dst_base = t * q_target + hi * hd;
                for d in 0..hd {
                    q_only[dst_base + d] = q_proj[proj_base + d];
                    gate_only[dst_base + d] = q_proj[proj_base + hd + d];
                }
            }
        }
        (q_only, Some(gate_only))
    } else {
        (q_proj, None)
    };
    acc.record(&format!("layer[{n}].attn.q"), &q_only);
    acc.record(&format!("layer[{n}].attn.k"), &k_proj);
    acc.record(&format!("layer[{n}].attn.v"), &v_proj);

    // QK norm (per head, normalize over head_dim).
    let mut q_normed = if let Some(g) = q_norm_gamma.as_ref() {
        head_rms_norm_f32(&q_only, g, m, dims.num_q_heads, dims.head_dim, DEFAULT_NORM_EPS)
    } else {
        q_only
    };
    let mut k_normed = if let Some(g) = k_norm_gamma.as_ref() {
        head_rms_norm_f32(&k_proj, g, m, dims.num_kv_heads, dims.head_dim, DEFAULT_NORM_EPS)
    } else {
        k_proj
    };
    acc.record(&format!("layer[{n}].qk_norm_post"), &q_normed);
    acc.record(&format!("layer[{n}].qk_norm_post"), &k_normed);

    apply_rope_f32(&mut q_normed, m, dims.num_q_heads, dims.head_dim, dims.rope_theta);
    apply_rope_f32(&mut k_normed, m, dims.num_kv_heads, dims.head_dim, dims.rope_theta);

    let (mut attn_out, max_score, max_attn_out) = attention_core_f32(
        &q_normed,
        &k_normed,
        &v_proj,
        m,
        dims.num_q_heads,
        dims.num_kv_heads,
        dims.head_dim,
    );
    {
        // Synthetic single-element vectors so ScaleAcc::record uses the
        // captured maxes directly.
        let s = vec![max_score];
        acc.record(&format!("layer[{n}].attn.score"), &s);
        let ao = vec![max_attn_out];
        acc.record(&format!("layer[{n}].attn.attn_out"), &ao);
    }

    if let Some(gate) = gate_only {
        for k in 0..attn_out.len() {
            attn_out[k] *= sigmoid(gate[k]);
        }
    }

    let o_proj = matmul_f32(&attn_out, &w_o_f, m, q_target, h);
    acc.record(&format!("layer[{n}].attn.o"), &o_proj);

    // Residual.
    let mut residual1 = vec![0f32; m * h];
    for i in 0..m * h {
        residual1[i] = input[i] + o_proj[i];
    }

    // ── FFN sub-block ─────────────────────────────────────────────────────
    let normed2 = rms_norm_f32(&residual1, &norm2_g, m, h, DEFAULT_NORM_EPS);
    acc.record(&format!("layer[{n}].norm_post.2"), &normed2);

    let gate_p = matmul_f32(&normed2, &w_gate_f, m, h, dims.intermediate);
    let up_p = matmul_f32(&normed2, &w_up_f, m, h, dims.intermediate);
    acc.record(&format!("layer[{n}].ffn.gate"), &gate_p);
    acc.record(&format!("layer[{n}].ffn.up"), &up_p);

    let mut mid = vec![0f32; m * dims.intermediate];
    for i in 0..m * dims.intermediate {
        mid[i] = silu(gate_p[i]) * up_p[i];
    }
    acc.record(&format!("layer[{n}].ffn.mid"), &mid);

    let down = matmul_f32(&mid, &w_down_f, m, dims.intermediate, h);
    acc.record(&format!("layer[{n}].ffn.down"), &down);

    let mut out = residual1;
    for i in 0..m * h {
        out[i] += down[i];
    }
    Ok(out)
}

// ─── Main loop ───────────────────────────────────────────────────────────────

fn parse_prompts(path: &std::path::Path, cap: usize) -> Result<Vec<Vec<u32>>, String> {
    let body = std::fs::read_to_string(path).map_err(|e| format!("read prompts: {e}"))?;
    let mut prompts = Vec::new();
    for (i, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lb = line.find('[').ok_or_else(|| format!("line {}: no [", i + 1))?;
        let rb = line.find(']').ok_or_else(|| format!("line {}: no ]", i + 1))?;
        let inner = &line[lb + 1..rb];
        let toks: Vec<u32> = inner
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<u32>().map_err(|e| format!("line {} tok: {e}", i + 1)))
            .collect::<Result<_, _>>()?;
        if toks.is_empty() {
            continue;
        }
        let toks = if toks.len() > cap {
            toks[..cap].to_vec()
        } else {
            toks
        };
        prompts.push(toks);
    }
    if prompts.is_empty() {
        return Err("no prompts parsed".into());
    }
    Ok(prompts)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    eprintln!("opening GGUF: {}", args.gguf.display());
    let mut file = std::fs::File::open(&args.gguf).map_err(|e| format!("open gguf: {e}"))?;
    let content = Content::read(&mut file).map_err(|e| format!("read gguf: {e}"))?;
    let arch = arch_str_from_content(&content)?;
    if arch != "qwen35" && arch != "qwen3" {
        return Err(format!("calibrate supports qwen35/qwen3 only; got arch={arch}"));
    }
    let dims = read_qwen35_dims(&content)?;
    eprintln!(
        "arch={arch} num_layers={} hidden={} intermediate={} num_q={} num_kv={} head_dim={} rope_theta={}",
        dims.num_layers,
        dims.hidden,
        dims.intermediate,
        dims.num_q_heads,
        dims.num_kv_heads,
        dims.head_dim,
        dims.rope_theta
    );

    let prompts = parse_prompts(&args.prompts, args.seq_len_cap)?;
    eprintln!("loaded {} prompts (cap {} tokens)", prompts.len(), args.seq_len_cap);

    // Embed table (vocab, hidden) → embed once, keep in RAM.
    eprintln!("→ embed");
    let (embed_f, embed_shape) =
        dequant_to_vec_f32(&content, &mut file, "token_embd.weight")?;
    let vocab = embed_shape[0];
    if embed_shape[1] != dims.hidden {
        return Err(format!(
            "token_embd shape {:?} doesn't match hidden {}",
            embed_shape, dims.hidden
        ));
    }

    // Build per-prompt activations: (m, hidden) f32 in a flat vec.
    let mut prompt_acts: Vec<Vec<f32>> = Vec::with_capacity(prompts.len());
    for tokens in &prompts {
        let m = tokens.len();
        let mut acts = vec![0f32; m * dims.hidden];
        for (i, &tok) in tokens.iter().enumerate() {
            if (tok as usize) >= vocab {
                return Err(format!("token {tok} >= vocab {vocab}"));
            }
            let src = &embed_f[(tok as usize) * dims.hidden..((tok as usize) + 1) * dims.hidden];
            let dst = &mut acts[i * dims.hidden..(i + 1) * dims.hidden];
            dst.copy_from_slice(src);
        }
        prompt_acts.push(acts);
    }
    drop(embed_f);

    let mut acc = ScaleAcc::new();

    // Per-layer forward.
    for n in 0..dims.num_layers {
        let is_std = qwen35_block_is_standard(&content, n);
        if n % 4 == 0 || n + 1 == dims.num_layers {
            eprintln!(
                "→ layer {} ({}) [acts cached for {} prompts]",
                n,
                if is_std { "standard" } else { "hybrid" },
                prompt_acts.len()
            );
        }
        if is_std {
            for (pi, acts) in prompt_acts.iter_mut().enumerate() {
                let m = acts.len() / dims.hidden;
                let new_acts = forward_std_layer(n, acts, m, &dims, &content, &mut file, &mut acc)
                    .map_err(|e| format!("layer {n} prompt {pi}: {e}"))?;
                *acts = new_acts;
            }
        } else {
            // Hybrid blocks: residual passthrough — leave activations as-is.
            // Records no taps; SSM scales fall back to the default at convert time.
        }
    }

    // Final norm.
    eprintln!("→ final norm");
    let (final_gamma, _) =
        dequant_to_vec_f32(&content, &mut file, "output_norm.weight").or_else(|_| {
            // Some Qwen GGUFs use `final_layernorm`/`norm.weight` instead.
            dequant_to_vec_f32(&content, &mut file, "norm.weight")
        })?;
    for acts in prompt_acts.iter_mut() {
        let m = acts.len() / dims.hidden;
        let n = rms_norm_f32(acts, &final_gamma, m, dims.hidden, DEFAULT_NORM_EPS);
        acc.record("final_norm_post", &n);
        *acts = n;
    }

    // ─── Derive scales and emit JSON ─────────────────────────────────────
    eprintln!(
        "→ writing scales.json: {} recorded taps",
        acc.inner.len()
    );

    // Fill in stable defaults for any tap that was never visited.
    // The converter falls back to `default` when a key is missing — but
    // emitting them explicitly makes diffs easier to read.
    let default_max_abs: f32 = 1.0; // ⇒ scale_num = 258 ≈ 1/127.
    acc.merge_default("default", default_max_abs);
    acc.merge_default("norm_post", default_max_abs);
    acc.merge_default("final_norm_post", default_max_abs);
    for n in 0..dims.num_layers {
        for k in ["q", "k", "v", "score", "attn_out", "o"] {
            acc.merge_default(&format!("layer[{n}].attn.{k}"), default_max_abs);
        }
        for k in ["gate", "up", "mid", "down"] {
            acc.merge_default(&format!("layer[{n}].ffn.{k}"), default_max_abs);
        }
        acc.merge_default(&format!("layer[{n}].norm_post.1"), default_max_abs);
        acc.merge_default(&format!("layer[{n}].norm_post.2"), default_max_abs);
        acc.merge_default(&format!("layer[{n}].qk_norm_post"), default_max_abs);
        for k in [
            "q", "k", "v", "alpha_logit", "beta_logit", "u", "decay", "update", "o", "proj",
        ] {
            acc.merge_default(&format!("layer[{n}].ssm.{k}"), default_max_abs);
        }
        acc.merge_default(&format!("layer[{n}].ssm_norm_post"), default_max_abs);
    }

    let mut entries: Vec<(String, i32)> = acc
        .inner
        .into_iter()
        .map(|(k, v)| (k, f32_to_scale_num(v)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // Hand-roll the JSON: serde_json is fine but the gguf-convert side
    // already parses a tiny subset, no need to pull in another dep here.
    let mut out = String::new();
    out.push_str("{\n  \"model_arch\": \"");
    out.push_str(&arch);
    out.push_str("\",\n  \"mode\": \"activation_f32\",\n  \"activation_scales\": {\n");
    let total = entries.len();
    for (i, (k, v)) in entries.iter().enumerate() {
        out.push_str("    \"");
        out.push_str(&k.replace('"', "\\\""));
        out.push_str("\": ");
        out.push_str(&v.to_string());
        if i + 1 < total {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  },\n  \"norm_eps_q\": 1\n}\n");
    std::fs::write(&args.out, out).map_err(|e| format!("write {}: {e}", args.out.display()))?;
    eprintln!("wrote {} ({} taps)", args.out.display(), total);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("calibrate: {e}");
            ExitCode::from(2)
        }
    }
}
