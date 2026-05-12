//! Phase 2.9.3+ — Rust f32 activation calibrator (Phase A).
//!
//! Mirrors the llama.cpp `qwen35` graph (`src/models/qwen35.cpp`) exactly:
//!
//!   * 16 standard attention layers (every 4th: idx 3,7,…,63) with packed
//!     Q+gate, per-head Q/K RMSNorm, IMROPE on the first `n_rot` dims,
//!     causal GQA softmax-attention, sigmoid output gate, output proj.
//!   * 48 hybrid GatedDeltaNet layers (the other indices). Two input
//!     projections (`attn_qkv` -> [Q|K|V], `attn_gate` -> z), causal
//!     depthwise conv1d (kernel=4), SiLU, channel-split, per-(k_head)
//!     L2-norm of Q/K, k_head -> v_head broadcast, alpha/beta projections,
//!     softplus(alpha + dt) * ssm_a recurrence decay, and the scalar
//!     DeltaNet update S <- S*g + outer((v - S k)*beta, k).
//!   * SwiGLU FFN, two residuals per block, final RMSNorm.
//!
//! Records `max(|x|)` at every quantization tap point in the INT8
//! forward and emits a `scales.json` consumable by `gguf-convert
//! --scales`.
//!
//! Usage:
//!
//!   cargo run --release -p ai-pow-vi --bin calibrate \
//!       --features gguf-convert -- \
//!       --gguf /path/to/model.gguf \
//!       --prompts /path/to/prompts.jsonl \
//!       --out /tmp/scales.json [--verify-top1]
//!
//! `prompts.jsonl` shares the format `vi-eval` uses — one
//! `{"prompt": [tok, ...], "expected_top1": N}` per line. `expected_top1`
//! is only consulted under `--verify-top1`.

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
    verify_top1: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut gguf = None;
    let mut prompts = None;
    let mut out = None;
    let mut seq_len_cap: usize = 64;
    let mut verify_top1 = false;
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
            "--verify-top1" => verify_top1 = true,
            "-h" | "--help" => {
                return Err(
                    "calibrate --gguf <path> --prompts <jsonl> --out <scales.json> \
                     [--seq-len-cap N] [--verify-top1]"
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
        verify_top1,
    })
}

// ─── GGUF metadata helpers ───────────────────────────────────────────────────

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

fn value_as_u32(v: &Value) -> Option<u32> {
    match v {
        Value::U8(x) => Some(*x as u32),
        Value::U16(x) => Some(*x as u32),
        Value::U32(x) => Some(*x),
        Value::U64(x) => Some(*x as u32),
        Value::I8(x) => Some(*x as u32),
        Value::I16(x) => Some(*x as u32),
        Value::I32(x) => Some(*x as u32),
        Value::I64(x) => Some(*x as u32),
        _ => None,
    }
}

fn meta_u32(content: &Content, key: &str, default: Option<u32>) -> Result<u32, String> {
    match content.metadata.get(key) {
        Some(Value::Array(arr)) => {
            // Scalar fallback when an array is present: take the max non-zero
            // (matches the gguf-convert helper's behaviour).
            let mut best = 0u32;
            for v in arr {
                if let Some(x) = value_as_u32(v) {
                    if x > best {
                        best = x;
                    }
                }
            }
            if best > 0 {
                Ok(best)
            } else {
                default.ok_or_else(|| format!("{key}: array of zeros and no default"))
            }
        }
        Some(v) => value_as_u32(v).ok_or_else(|| format!("{key}: unexpected type {v:?}")),
        None => default.ok_or_else(|| format!("missing required field {key}")),
    }
}

fn meta_u32_array(
    content: &Content,
    key: &str,
    expected_len: Option<usize>,
) -> Result<Vec<u32>, String> {
    match content.metadata.get(key) {
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                out.push(value_as_u32(v).ok_or_else(|| format!("{key}[{i}]: {v:?}"))?);
            }
            if let Some(want) = expected_len {
                if out.len() != want {
                    return Err(format!(
                        "{key}: array length {} != expected {want}",
                        out.len()
                    ));
                }
            }
            Ok(out)
        }
        Some(v) => {
            // Promote a scalar to a length-1 vec.
            let x = value_as_u32(v).ok_or_else(|| format!("{key}: unexpected type {v:?}"))?;
            Ok(vec![x])
        }
        None => Err(format!("missing required field {key}")),
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
    /// Per-layer KV head count. SSM-only layers may report 0; we keep the
    /// std-layer value separately as `num_kv_std`.
    num_kv_heads: Vec<usize>,
    num_kv_std: usize,
    head_dim: usize,
    rope_theta: f32,
    eps: f32,
    n_rot: usize,
    rope_sections: [usize; 4],
    // SSM (GatedDeltaNet) dims.
    ssm_d_conv: usize,
    ssm_d_state: usize,
    ssm_dt_rank: usize,
    ssm_n_group: usize,
    ssm_d_inner: usize,
    full_attn_interval: usize,
}

fn read_qwen35_dims(content: &Content) -> Result<QwenDims, String> {
    let num_layers = meta_u32(content, "qwen35.block_count", None)? as usize;
    let head_dim = meta_u32(content, "qwen35.attention.key_length", None)? as usize;
    let num_q_heads = meta_u32(content, "qwen35.attention.head_count", None)? as usize;

    // Per-layer KV head count: array length num_layers.
    let kv_arr = meta_u32_array(
        content,
        "qwen35.attention.head_count_kv",
        Some(num_layers),
    )
    .or_else(|_| -> Result<Vec<u32>, String> {
        // Fallback: scalar value broadcast to every layer.
        let s = meta_u32(content, "qwen35.attention.head_count_kv", None)?;
        Ok(vec![s; num_layers])
    })?;
    let num_kv_heads: Vec<usize> = kv_arr.into_iter().map(|x| x as usize).collect();
    let num_kv_std = num_kv_heads
        .iter()
        .copied()
        .find(|&v| v > 0)
        .ok_or("no non-zero kv head count for any layer")?;

    let mut rope_sections = [0usize; 4];
    if let Ok(secs) = meta_u32_array(content, "qwen35.rope.dimension_sections", None) {
        for (i, s) in secs.iter().take(4).enumerate() {
            rope_sections[i] = *s as usize;
        }
    }

    let n_rot = meta_u32(content, "qwen35.rope.dimension_count", Some(head_dim as u32))? as usize;

    let eps =
        meta_f32(content, "qwen35.attention.layer_norm_rms_epsilon", Some(DEFAULT_NORM_EPS))?;

    let ssm_d_conv = meta_u32(content, "qwen35.ssm.conv_kernel", Some(4))? as usize;
    let ssm_d_state = meta_u32(content, "qwen35.ssm.state_size", Some(128))? as usize;
    let ssm_dt_rank = meta_u32(content, "qwen35.ssm.time_step_rank", Some(48))? as usize;
    let ssm_n_group = meta_u32(content, "qwen35.ssm.group_count", Some(16))? as usize;
    let ssm_d_inner = meta_u32(content, "qwen35.ssm.inner_size", Some(6144))? as usize;

    let full_attn_interval =
        meta_u32(content, "qwen35.full_attention_interval", Some(4))? as usize;

    Ok(QwenDims {
        hidden: meta_u32(content, "qwen35.embedding_length", None)? as usize,
        intermediate: meta_u32(content, "qwen35.feed_forward_length", None)? as usize,
        num_layers,
        num_q_heads,
        num_kv_heads,
        num_kv_std,
        head_dim,
        rope_theta: meta_f32(content, "qwen35.rope.freq_base", Some(10_000.0))?,
        eps,
        n_rot,
        rope_sections,
        ssm_d_conv,
        ssm_d_state,
        ssm_dt_rank,
        ssm_n_group,
        ssm_d_inner,
        full_attn_interval,
    })
}

/// Block classifier (mirrors gguf_convert + qwen35.cpp:19-21): a block is
/// std when `(i+1) % full_attn_interval == 0`. We also probe for the tensor
/// to stay robust against metadata mismatches.
fn qwen35_block_is_standard(content: &Content, n: usize, dims: &QwenDims) -> bool {
    if (n + 1) % dims.full_attn_interval == 0 {
        return true;
    }
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

fn dequant_opt(
    content: &Content,
    file: &mut std::fs::File,
    name: &str,
) -> Option<(Vec<f32>, Vec<usize>)> {
    dequant_to_vec_f32(content, file, name).ok()
}

// ─── Tap accumulator ─────────────────────────────────────────────────────────

struct ScaleAcc {
    /// Per-tap reservoir of absolute values. We accumulate all observed
    /// |x| samples across prompts and tokens so we can pick a high
    /// percentile (e.g. 99.999%) at finalize time. This is the literature-
    /// recommended replacement for raw max — transformer activations have
    /// heavy-tailed distributions where a single outlier dominates the
    /// per-tensor max and saturates downstream i8 outputs (SmoothQuant
    /// Xiao et al. 2022, NVIDIA TensorRT calibration guide).
    samples: HashMap<String, Vec<f32>>,
    /// Per-tap per-channel running max(|x_j|), keyed by tap name → Vec<f32>
    /// of length channel_dim. Used by the SmoothQuant offline fold
    /// (Xiao 2022 §4.2): for each (RMSNorm γ → matmul) pair, picks per-
    /// channel smoothing factors that migrate activation outliers into
    /// weights, allowing per-tensor symmetric INT8 to recover accuracy
    /// without runtime API change.
    per_channel: HashMap<String, Vec<f32>>,
    /// Quantile to pick (default 0.99999 = 99.999 percentile). 1.0 = raw max.
    percentile: f32,
}

impl ScaleAcc {
    fn new(percentile: f32) -> Self {
        Self {
            samples: HashMap::new(),
            per_channel: HashMap::new(),
            percentile: percentile.clamp(0.0, 1.0),
        }
    }
    /// Update per-channel running max for the given tap. `x` is `(m,
    /// channel_dim)` row-major; iterates each channel across all m rows
    /// and updates the running per-channel max(|x_j|).
    fn record_per_channel(&mut self, tap: &str, x: &[f32], channel_dim: usize) {
        if x.is_empty() || channel_dim == 0 || x.len() % channel_dim != 0 {
            return;
        }
        let rows = x.len() / channel_dim;
        let entry = self
            .per_channel
            .entry(tap.to_string())
            .or_insert_with(|| vec![0.0f32; channel_dim]);
        if entry.len() != channel_dim {
            return;
        }
        for r in 0..rows {
            let off = r * channel_dim;
            for j in 0..channel_dim {
                let av = x[off + j].abs();
                if av > entry[j] {
                    entry[j] = av;
                }
            }
        }
    }
    fn record(&mut self, tap: &str, x: &[f32]) {
        if x.is_empty() {
            return;
        }
        let entry = self.samples.entry(tap.to_string()).or_insert_with(Vec::new);
        for &v in x {
            let av = v.abs();
            if av > 0.0 {
                entry.push(av);
            }
        }
    }
    /// Return the percentile-clipped max for a tap, or 0.0 if no samples.
    fn percentile_max(&self, tap: &str) -> f32 {
        let v = match self.samples.get(tap) {
            Some(v) if !v.is_empty() => v,
            _ => return 0.0,
        };
        let mut sorted: Vec<f32> = v.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((sorted.len() as f32 - 1.0) * self.percentile).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }
    fn merge_default(&mut self, key: &str, fallback: f32) {
        if !self.samples.contains_key(key) {
            // Single-sample reservoir representing the fallback magnitude.
            self.samples.insert(key.to_string(), vec![fallback]);
        }
    }
    fn all_taps(&self) -> impl Iterator<Item = &String> {
        self.samples.keys()
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

/// Per-token RMSNorm: `y = x / rms(x) * gamma`, `rms(x) = sqrt(mean(x^2) + eps)`.
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

/// ggml-style L2 norm over the last axis: `y = x / max(sqrt(sum_x^2), eps)`.
/// In-place modify per (row * num_heads + h, head_dim) chunk.
fn l2_norm_per_head_f32(
    x: &mut [f32],
    m: usize,
    num_heads: usize,
    head_dim: usize,
    eps: f32,
) {
    for row in 0..m {
        for h in 0..num_heads {
            let off = (row * num_heads + h) * head_dim;
            let mut sumsq: f32 = 0.0;
            for k in 0..head_dim {
                sumsq += x[off + k] * x[off + k];
            }
            let inv = 1.0 / sumsq.sqrt().max(eps);
            for k in 0..head_dim {
                x[off + k] *= inv;
            }
        }
    }
}

fn silu(x: f32) -> f32 {
    x / (1.0 + (-x).exp())
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn softplus_f32(x: f32) -> f32 {
    if x > 20.0 {
        x
    } else {
        (1.0 + x.exp()).ln()
    }
}

/// Causal in-place self-attention from per-token Q,K,V buffers.
/// Q layout: `(m, num_q_heads, head_dim)`. K, V: `(m, num_kv_heads, head_dim)`.
fn attention_core_f32(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    m: usize,
    num_q_heads: usize,
    num_kv_heads: usize,
    head_dim: usize,
) -> (Vec<f32>, f32, f32) {
    let kv_groups = num_q_heads / num_kv_heads.max(1);
    let inv_sqrt_dh = (head_dim as f32).powf(-0.5);
    let mut out = vec![0f32; m * num_q_heads * head_dim];
    let mut max_score: f32 = 0.0;
    let mut max_attn_out: f32 = 0.0;
    for h in 0..num_q_heads {
        let kv_h = h / kv_groups;
        for t in 0..m {
            let q_off = (t * num_q_heads + h) * head_dim;
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

/// IMROPE — interleaved multi-axis RoPE on the first `n_rot` dims, NEOX
/// pairing `(x[ic], x[ic + n_rot/2])`. For text-only inference all axis
/// positions equal the token index, so the per-pair theta is just
/// `t * freq_base^(-2j/n_rot)`; the sector dispatch in ggml-cpu/ops.cpp:5725
/// is preserved verbatim so identical behaviour holds even if positions
/// ever differ. Dims `[n_rot, head_dim)` are left unrotated (passthrough).
fn apply_imrope_f32(
    x: &mut [f32],
    m: usize,
    num_heads: usize,
    head_dim: usize,
    n_rot: usize,
    rope_theta: f32,
    sections: [usize; 4],
) {
    assert!(n_rot <= head_dim);
    assert!(n_rot % 2 == 0);
    let half = n_rot / 2;
    let theta_scale = (rope_theta as f64).powf(-2.0 / n_rot as f64);
    let sect_dims: usize = sections[0] + sections[1] + sections[2] + sections[3];
    // Pre-compute (cos, sin) per token for all `half` pairs.
    for t in 0..m {
        // Per-axis position bases. For text-only, all four equal `t`.
        let pt = t as f64;
        let ph = t as f64;
        let pw = t as f64;
        let pe = t as f64;
        // theta_t_running etc. are pt * theta_scale^j, etc.
        let mut tt = pt;
        let mut th = ph;
        let mut tw = pw;
        let mut te = pe;
        for h in 0..num_heads {
            let off = (t * num_heads + h) * head_dim;
            // Snapshot the per-pair theta sequence; ggml advances all four
            // running thetas each pair, so for each h we restart from base.
            let mut tt_p = pt;
            let mut th_p = ph;
            let mut tw_p = pw;
            let mut te_p = pe;
            for j in 0..half {
                let sector = if sect_dims > 0 { j % sect_dims } else { 0 };
                let theta = if sect_dims > 0 {
                    if sector % 3 == 0 && sector < 3 * sections[0] {
                        tt_p
                    } else if sector % 3 == 1 && sector < 3 * sections[1] {
                        th_p
                    } else if sector % 3 == 2 && sector < 3 * sections[2] {
                        tw_p
                    } else {
                        te_p
                    }
                } else {
                    // No sections metadata: fall back to plain NEOX RoPE.
                    tt_p
                };
                let c = theta.cos() as f32;
                let s = theta.sin() as f32;
                let a = x[off + j];
                let b = x[off + half + j];
                x[off + j] = a * c - b * s;
                x[off + half + j] = a * s + b * c;

                tt_p *= theta_scale;
                th_p *= theta_scale;
                tw_p *= theta_scale;
                te_p *= theta_scale;
            }
            // Unused per-head advance vars (avoid dead-code warns).
            let _ = (&mut tt, &mut th, &mut tw, &mut te);
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
    let n_kv = if dims.num_kv_heads[n] > 0 {
        dims.num_kv_heads[n]
    } else {
        dims.num_kv_std
    };

    let (norm1_g, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_norm.weight"))?;
    let (norm2_g, _) = dequant_to_vec_f32(
        content,
        file,
        &format!("{prefix}.post_attention_norm.weight"),
    )
    .or_else(|_| dequant_to_vec_f32(content, file, &format!("{prefix}.attn_post_norm.weight")))?;

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

    let q_norm_gamma = dequant_opt(content, file, &format!("{prefix}.attn_q_norm.weight"))
        .map(|(f, _)| f);
    let k_norm_gamma = dequant_opt(content, file, &format!("{prefix}.attn_k_norm.weight"))
        .map(|(f, _)| f);

    let (w_gate_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_gate.weight"))?;
    let (w_up_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_up.weight"))?;
    let (w_down_f, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_down.weight"))?;

    // ── Attention sub-block ──────────────────────────────────────────────
    let normed1 = rms_norm_f32(input, &norm1_g, m, h, dims.eps);
    acc.record_per_channel(&format!("layer[{n}].norm1.x_pc"), &normed1, h);
    acc.record(&format!("layer[{n}].norm_post.1"), &normed1);

    let q_proj = matmul_f32(&normed1, &w_q_f, m, h, q_eff_out);
    let k_proj = matmul_f32(&normed1, &w_k_f, m, h, n_kv * dims.head_dim);
    let v_proj = matmul_f32(&normed1, &w_v_f, m, h, n_kv * dims.head_dim);

    // Split joint Q+gate per head when present.
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

    let mut q_normed = if let Some(g) = q_norm_gamma.as_ref() {
        head_rms_norm_f32(&q_only, g, m, dims.num_q_heads, dims.head_dim, dims.eps)
    } else {
        q_only
    };
    let mut k_normed = if let Some(g) = k_norm_gamma.as_ref() {
        head_rms_norm_f32(&k_proj, g, m, n_kv, dims.head_dim, dims.eps)
    } else {
        k_proj
    };
    acc.record(&format!("layer[{n}].qk_norm_post"), &q_normed);
    acc.record(&format!("layer[{n}].qk_norm_post"), &k_normed);

    apply_imrope_f32(
        &mut q_normed,
        m,
        dims.num_q_heads,
        dims.head_dim,
        dims.n_rot,
        dims.rope_theta,
        dims.rope_sections,
    );
    apply_imrope_f32(
        &mut k_normed,
        m,
        n_kv,
        dims.head_dim,
        dims.n_rot,
        dims.rope_theta,
        dims.rope_sections,
    );

    let (mut attn_out, max_score, max_attn_out) = attention_core_f32(
        &q_normed,
        &k_normed,
        &v_proj,
        m,
        dims.num_q_heads,
        n_kv,
        dims.head_dim,
    );
    {
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
    let normed2 = rms_norm_f32(&residual1, &norm2_g, m, h, dims.eps);
    acc.record_per_channel(&format!("layer[{n}].norm2.x_pc"), &normed2, h);
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

// ─── Hybrid GatedDeltaNet layer forward ──────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn forward_hybrid_layer(
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

    let num_v = dims.ssm_dt_rank; // 48
    let num_k = dims.ssm_n_group; // 16
    let head_k = dims.ssm_d_state; // 128
    let head_v = dims.ssm_d_inner / num_v; // 128
    let key_dim = head_k * num_k; // 2048
    let value_dim = head_v * num_v; // 6144
    let conv_dim = key_dim * 2 + value_dim; // 10240
    let kk = dims.ssm_d_conv; // 4
    assert_eq!(num_v % num_k, 0, "num_v_heads not divisible by num_k_heads");
    let kv_groups = num_v / num_k;

    let (norm1_g, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_norm.weight"))?;
    let (norm2_g, _) = dequant_to_vec_f32(
        content,
        file,
        &format!("{prefix}.post_attention_norm.weight"),
    )
    .or_else(|_| dequant_to_vec_f32(content, file, &format!("{prefix}.attn_post_norm.weight")))?;

    let (w_qkv, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_qkv.weight"))?;
    let (w_gate, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.attn_gate.weight"))?;

    let (w_conv1d_raw, conv_shape) =
        dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_conv1d.weight"))?;
    // GGUF native shape is [kernel, channels], but candle reports PyTorch
    // outer-first order [channels, kernel] and lays the bytes out the same.
    // Transpose into [kernel, channels] (kernel-outer) row-major so the conv
    // loop below can use `w[kk * conv_dim + c]`.
    assert_eq!(conv_shape[0], conv_dim, "ssm_conv1d channel mismatch");
    assert_eq!(conv_shape[1], kk, "ssm_conv1d kernel mismatch");
    let mut w_conv1d = vec![0f32; kk * conv_dim];
    for c in 0..conv_dim {
        for k in 0..kk {
            w_conv1d[k * conv_dim + c] = w_conv1d_raw[c * kk + k];
        }
    }

    // The Ollama-shipped Qwen 3.5 GGUF uses bare `ssm_dt` (no `.bias` suffix);
    // newer llama.cpp branches emit `ssm_dt.bias`. Accept both.
    let (ssm_dt, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_dt"))
        .or_else(|_| dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_dt.bias")))?;
    let (ssm_a, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_a"))?;
    let (w_beta, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_beta.weight"))?;
    let (w_alpha, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_alpha.weight"))?;
    let (ssm_norm_g, _) =
        dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_norm.weight"))?;
    let (w_ssm_out, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_out.weight"))?;

    let (w_ffn_gate, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_gate.weight"))?;
    let (w_ffn_up, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_up.weight"))?;
    let (w_ffn_down, _) = dequant_to_vec_f32(content, file, &format!("{prefix}.ffn_down.weight"))?;

    // ── Pre-attention norm ───────────────────────────────────────────────
    let normed1 = rms_norm_f32(input, &norm1_g, m, h, dims.eps);
    acc.record_per_channel(&format!("layer[{n}].norm1.x_pc"), &normed1, h);
    acc.record(&format!("layer[{n}].norm_post.1"), &normed1);

    // qkv_mixed: [m, conv_dim], layout per token: [Q(key_dim) | K(key_dim) | V(value_dim)].
    let qkv_mixed = matmul_f32(&normed1, &w_qkv, m, h, conv_dim);
    acc.record(&format!("layer[{n}].ssm.q"), &qkv_mixed);

    // z output gate: [m, value_dim].
    let z_full = matmul_f32(&normed1, &w_gate, m, h, value_dim);
    acc.record(&format!("layer[{n}].ssm.proj"), &z_full);

    // beta: [m, num_v_heads]
    let beta_raw = matmul_f32(&normed1, &w_beta, m, h, num_v);
    acc.record(&format!("layer[{n}].ssm.beta_logit"), &beta_raw);
    let mut beta = vec![0f32; m * num_v];
    for i in 0..m * num_v {
        beta[i] = sigmoid(beta_raw[i]);
    }

    // alpha: [m, num_v_heads]
    let alpha_raw = matmul_f32(&normed1, &w_alpha, m, h, num_v);
    acc.record(&format!("layer[{n}].ssm.alpha_logit"), &alpha_raw);

    // gate_log[t,h] = softplus(alpha[t,h] + dt[h]) * ssm_a[h]
    let mut gate_log = vec![0f32; m * num_v];
    for t in 0..m {
        for hh in 0..num_v {
            let v = alpha_raw[t * num_v + hh] + ssm_dt[hh];
            gate_log[t * num_v + hh] = softplus_f32(v) * ssm_a[hh];
        }
    }
    acc.record(&format!("layer[{n}].ssm.decay"), &gate_log);

    // ── Causal depthwise conv1d (kernel=4) ───────────────────────────────
    // qkv_mixed indexing: x[t * conv_dim + c]
    // weight indexing: w[k * conv_dim + c] for kernel index k in [0, kk).
    let mut conv_out = vec![0f32; m * conv_dim];
    for t in 0..m {
        for c in 0..conv_dim {
            let mut s: f32 = 0.0;
            for kk_i in 0..kk {
                // ssm_conv naming: at output time t, kernel position kk_i (0..K)
                // taps input at t - (K-1) + kk_i. Zero pad negatives.
                let in_t = (t as isize) - (kk as isize - 1) + kk_i as isize;
                if in_t >= 0 {
                    s += w_conv1d[kk_i * conv_dim + c] * qkv_mixed[in_t as usize * conv_dim + c];
                }
            }
            conv_out[t * conv_dim + c] = silu(s);
        }
    }
    acc.record(&format!("layer[{n}].ssm.u"), &conv_out);

    // ── Split conv output along channel: Q | K | V ───────────────────────
    let mut q_conv = vec![0f32; m * num_k * head_k];
    let mut k_conv = vec![0f32; m * num_k * head_k];
    let mut v_conv = vec![0f32; m * num_v * head_v];
    for t in 0..m {
        let row = t * conv_dim;
        // Q: first key_dim channels.
        for kh in 0..num_k {
            for d in 0..head_k {
                q_conv[(t * num_k + kh) * head_k + d] = conv_out[row + kh * head_k + d];
            }
        }
        // K: next key_dim channels.
        for kh in 0..num_k {
            for d in 0..head_k {
                k_conv[(t * num_k + kh) * head_k + d] =
                    conv_out[row + key_dim + kh * head_k + d];
            }
        }
        // V: last value_dim channels.
        for vh in 0..num_v {
            for d in 0..head_v {
                v_conv[(t * num_v + vh) * head_v + d] =
                    conv_out[row + 2 * key_dim + vh * head_v + d];
            }
        }
    }

    // L2-norm Q and K per (k_head). V is NOT L2-normed.
    l2_norm_per_head_f32(&mut q_conv, m, num_k, head_k, dims.eps);
    l2_norm_per_head_f32(&mut k_conv, m, num_k, head_k, dims.eps);

    // Q scaling matches `q = q * 1/sqrt(head_k_dim)` from
    // delta-net-base.cpp:44 (chunking) and :320 (autoregressive). Applied
    // once up front so the recurrence below is unscaled.
    let q_scale = 1.0 / (head_k as f32).sqrt();
    for v in q_conv.iter_mut() {
        *v *= q_scale;
    }

    // Broadcast Q, K from num_k_heads to num_v_heads. qwen35.cpp uses
    // `ggml_repeat_4d` which TILES along the head axis (ops.cpp:1720-1736:
    // `dst[i1*ne01 + k1] = src[k1]`), so v-head `vh` reads k-head
    // `vh % num_k`, NOT `vh / kv_groups` (which would be repeat_interleave
    // semantics — wrong for this op).
    let mut q_br = vec![0f32; m * num_v * head_k];
    let mut k_br = vec![0f32; m * num_v * head_k];
    for t in 0..m {
        for vh in 0..num_v {
            let kh = vh % num_k;
            let src = (t * num_k + kh) * head_k;
            let dst = (t * num_v + vh) * head_k;
            for d in 0..head_k {
                q_br[dst + d] = q_conv[src + d];
                k_br[dst + d] = k_conv[src + d];
            }
        }
    }
    acc.record(&format!("layer[{n}].ssm.k"), &k_br);
    acc.record(&format!("layer[{n}].ssm.v"), &v_conv);

    // ── DeltaNet recurrence, per v_head ─────────────────────────────────
    // State S_h: [head_v, head_k]. For each token t:
    //   S_h *= exp(gate_log[t,h])
    //   kv_mem = S_h @ k_t            (head_v vector)
    //   delta  = (v_t - kv_mem) * beta[t,h]
    //   S_h   += delta outer k_t      (rank-1)
    //   out[t,h,:] = S_h @ q_t
    let mut out = vec![0f32; m * num_v * head_v];
    let mut state = vec![0f32; head_v * head_k];
    let mut max_update: f32 = 0.0;
    for hh in 0..num_v {
        // Reset state per head.
        for s in state.iter_mut() {
            *s = 0.0;
        }
        for t in 0..m {
            let q_off = (t * num_v + hh) * head_k;
            let k_off = (t * num_v + hh) * head_k;
            let v_off = (t * num_v + hh) * head_v;
            let out_off = (t * num_v + hh) * head_v;

            let g = gate_log[t * num_v + hh].exp();
            let bt = beta[t * num_v + hh];

            // Decay state.
            for s in state.iter_mut() {
                *s *= g;
            }

            // kv_mem[i] = sum_j S[i, j] * k_t[j]
            let mut kv_mem = vec![0f32; head_v];
            for i in 0..head_v {
                let row = i * head_k;
                let mut acc_v: f32 = 0.0;
                for j in 0..head_k {
                    acc_v += state[row + j] * k_br[k_off + j];
                }
                kv_mem[i] = acc_v;
            }
            // delta = (v_t - kv_mem) * beta
            let mut delta = vec![0f32; head_v];
            for i in 0..head_v {
                delta[i] = (v_conv[v_off + i] - kv_mem[i]) * bt;
            }
            // S += outer(delta, k_t)
            for i in 0..head_v {
                let row = i * head_k;
                let di = delta[i];
                for j in 0..head_k {
                    let upd = di * k_br[k_off + j];
                    state[row + j] += upd;
                    let au = upd.abs();
                    if au > max_update {
                        max_update = au;
                    }
                }
            }
            // out[t,h,:] = S @ q_t
            for i in 0..head_v {
                let row = i * head_k;
                let mut s: f32 = 0.0;
                for j in 0..head_k {
                    s += state[row + j] * q_br[q_off + j];
                }
                out[out_off + i] = s;
            }
        }
    }
    {
        let s = vec![max_update];
        acc.record(&format!("layer[{n}].ssm.update"), &s);
    }
    acc.record(&format!("layer[{n}].ssm.o"), &out);

    // ── Gated RMSNorm: normalized = head_rms_norm(out, ssm_norm, eps);
    //    gated = normalized * silu(z_reshaped)
    // z is [m, value_dim] = [m, num_v, head_v].
    let normalized = head_rms_norm_f32(&out, &ssm_norm_g, m, num_v, head_v, dims.eps);
    let mut gated = vec![0f32; m * num_v * head_v];
    for i in 0..m * num_v * head_v {
        gated[i] = normalized[i] * silu(z_full[i]);
    }
    acc.record(&format!("layer[{n}].ssm_norm_post"), &gated);

    // Output projection.
    let o_proj = matmul_f32(&gated, &w_ssm_out, m, value_dim, h);
    acc.record(&format!("layer[{n}].attn.o"), &o_proj);

    // Residual after attention.
    let mut residual1 = vec![0f32; m * h];
    for i in 0..m * h {
        residual1[i] = input[i] + o_proj[i];
    }

    // ── FFN sub-block ────────────────────────────────────────────────────
    let normed2 = rms_norm_f32(&residual1, &norm2_g, m, h, dims.eps);
    acc.record_per_channel(&format!("layer[{n}].norm2.x_pc"), &normed2, h);
    acc.record(&format!("layer[{n}].norm_post.2"), &normed2);

    let gate_p = matmul_f32(&normed2, &w_ffn_gate, m, h, dims.intermediate);
    let up_p = matmul_f32(&normed2, &w_ffn_up, m, h, dims.intermediate);
    acc.record(&format!("layer[{n}].ffn.gate"), &gate_p);
    acc.record(&format!("layer[{n}].ffn.up"), &up_p);

    let mut mid = vec![0f32; m * dims.intermediate];
    for i in 0..m * dims.intermediate {
        mid[i] = silu(gate_p[i]) * up_p[i];
    }
    acc.record(&format!("layer[{n}].ffn.mid"), &mid);

    let down = matmul_f32(&mid, &w_ffn_down, m, dims.intermediate, h);
    acc.record(&format!("layer[{n}].ffn.down"), &down);

    let mut outv = residual1;
    for i in 0..m * h {
        outv[i] += down[i];
    }
    Ok(outv)
}

// ─── Prompts parser ──────────────────────────────────────────────────────────

struct PromptEntry {
    tokens: Vec<u32>,
    expected_top1: Option<u32>,
}

fn parse_prompts(path: &std::path::Path, cap: usize) -> Result<Vec<PromptEntry>, String> {
    let body = std::fs::read_to_string(path).map_err(|e| format!("read prompts: {e}"))?;
    let mut prompts = Vec::new();
    for (i, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // "prompt" array — first [...] in the line.
        let lb = line.find('[').ok_or_else(|| format!("line {}: no [", i + 1))?;
        let rb = line[lb..].find(']').ok_or_else(|| format!("line {}: no ]", i + 1))? + lb;
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
        // expected_top1: scan for `"expected_top1"\s*:\s*<int>`.
        let mut expected_top1 = None;
        if let Some(p) = line.find("\"expected_top1\"") {
            let rest = &line[p + "\"expected_top1\"".len()..];
            if let Some(colon) = rest.find(':') {
                let tail = rest[colon + 1..].trim_start();
                let end = tail
                    .find(|c: char| !(c.is_ascii_digit() || c == '-'))
                    .unwrap_or(tail.len());
                if end > 0 {
                    if let Ok(v) = tail[..end].parse::<i64>() {
                        if v >= 0 {
                            expected_top1 = Some(v as u32);
                        }
                    }
                }
            }
        }
        prompts.push(PromptEntry {
            tokens: toks,
            expected_top1,
        });
    }
    if prompts.is_empty() {
        return Err("no prompts parsed".into());
    }
    Ok(prompts)
}

// ─── Main loop ───────────────────────────────────────────────────────────────

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
        "arch={arch} num_layers={} hidden={} intermediate={} num_q={} num_kv_std={} head_dim={} \
         rope_theta={} n_rot={} eps={} sections={:?}",
        dims.num_layers,
        dims.hidden,
        dims.intermediate,
        dims.num_q_heads,
        dims.num_kv_std,
        dims.head_dim,
        dims.rope_theta,
        dims.n_rot,
        dims.eps,
        dims.rope_sections
    );
    eprintln!(
        "ssm: d_inner={} d_state={} dt_rank={} n_group={} d_conv={} full_attn_interval={}",
        dims.ssm_d_inner,
        dims.ssm_d_state,
        dims.ssm_dt_rank,
        dims.ssm_n_group,
        dims.ssm_d_conv,
        dims.full_attn_interval
    );

    let prompts = parse_prompts(&args.prompts, args.seq_len_cap)?;
    eprintln!(
        "loaded {} prompts (cap {} tokens)",
        prompts.len(),
        args.seq_len_cap
    );

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

    let mut prompt_acts: Vec<Vec<f32>> = Vec::with_capacity(prompts.len());
    for ent in &prompts {
        let tokens = &ent.tokens;
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
    // Embed table is needed again for tied logits in --verify-top1. Keep it
    // around only if we'll use it; otherwise free.
    let embed_for_lm = if args.verify_top1 { Some(embed_f) } else {
        drop(embed_f);
        None
    };

    // 99.999 percentile clipping: discards the top 0.001% of outlier
    // |x| values per tap. Recommended by NVIDIA TensorRT + Speechmatics
    // for transformer activations whose tails are dominated by 1-in-100k
    // outliers that bloat the per-tensor max and saturate INT8 forward.
    let mut acc = ScaleAcc::new(0.99999);

    // Per-layer forward.
    for n in 0..dims.num_layers {
        let is_std = qwen35_block_is_standard(&content, n, &dims);
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
                let new_acts =
                    forward_std_layer(n, acts, m, &dims, &content, &mut file, &mut acc)
                        .map_err(|e| format!("layer {n} prompt {pi}: {e}"))?;
                *acts = new_acts;
            }
        } else {
            for (pi, acts) in prompt_acts.iter_mut().enumerate() {
                let m = acts.len() / dims.hidden;
                let new_acts =
                    forward_hybrid_layer(n, acts, m, &dims, &content, &mut file, &mut acc)
                        .map_err(|e| format!("layer {n} prompt {pi}: {e}"))?;
                *acts = new_acts;
            }
        }
    }

    // Final norm.
    eprintln!("→ final norm");
    let (final_gamma, _) =
        dequant_to_vec_f32(&content, &mut file, "output_norm.weight").or_else(|_| {
            dequant_to_vec_f32(&content, &mut file, "norm.weight")
        })?;
    for acts in prompt_acts.iter_mut() {
        let m = acts.len() / dims.hidden;
        let n = rms_norm_f32(acts, &final_gamma, m, dims.hidden, dims.eps);
        acc.record("final_norm_post", &n);
        *acts = n;
    }

    // ── --verify-top1 ────────────────────────────────────────────────────
    if args.verify_top1 {
        // Use output.weight if present, else fall back to token_embd.weight
        // (tied embeddings).
        let (lm_head, lm_shape) =
            match dequant_to_vec_f32(&content, &mut file, "output.weight") {
                Ok(t) => t,
                Err(_) => {
                    let f = embed_for_lm.ok_or("output.weight absent and embed not cached")?;
                    let shape = vec![vocab, dims.hidden];
                    (f, shape)
                }
            };
        let lm_vocab = lm_shape[0];
        if lm_shape[1] != dims.hidden {
            return Err(format!(
                "output.weight shape {:?} doesn't match hidden {}",
                lm_shape, dims.hidden
            ));
        }
        let mut correct = 0usize;
        for (i, (ent, acts)) in prompts.iter().zip(prompt_acts.iter()).enumerate() {
            let m = acts.len() / dims.hidden;
            if m == 0 {
                continue;
            }
            let last_off = (m - 1) * dims.hidden;
            // logits[v] = sum_k acts[last,k] * lm_head[v,k]
            let mut best_v: u32 = 0;
            let mut best_s = f32::NEG_INFINITY;
            for v in 0..lm_vocab {
                let w_off = v * dims.hidden;
                let mut s: f32 = 0.0;
                for k in 0..dims.hidden {
                    s += acts[last_off + k] * lm_head[w_off + k];
                }
                if s > best_s {
                    best_s = s;
                    best_v = v as u32;
                }
            }
            let expected = ent.expected_top1;
            let matched = expected.map(|e| e == best_v).unwrap_or(false);
            let exp_str = expected.map(|e| e.to_string()).unwrap_or_else(|| "?".into());
            println!(
                "verify\tline={}\tpredicted={}\texpected={}\tmatch={}",
                i + 1,
                best_v,
                exp_str,
                matched
            );
            if matched {
                correct += 1;
            }
        }
        let total = prompts.len();
        let pct = if total > 0 {
            100.0 * (correct as f32) / (total as f32)
        } else {
            0.0
        };
        println!("verify_summary\t{}/{}\t{:.1}%", correct, total, pct);
    }

    // ─── Derive scales and emit JSON ─────────────────────────────────────
    eprintln!(
        "→ writing scales.json: {} recorded taps (percentile={})",
        acc.samples.len(),
        acc.percentile
    );

    let default_max_abs: f32 = 1.0;
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
        .all_taps()
        .map(|k| (k.clone(), f32_to_scale_num(acc.percentile_max(k))))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

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

    // SmoothQuant sidecar: per-channel max for every recorded norm output.
    // Format: { "tap_name": [f32; channel_dim], ... }. Used by gguf-convert
    // to fold smoothing factors into RMSNorm gamma + matmul weights at
    // convert time (Xiao 2022 §4.2 offline fusion).
    if !acc.per_channel.is_empty() {
        let pc_path = args.out.with_extension("pc.json");
        let mut pc_out = String::new();
        pc_out.push_str("{\n");
        let mut pc_entries: Vec<(&String, &Vec<f32>)> = acc.per_channel.iter().collect();
        pc_entries.sort_by(|a, b| a.0.cmp(b.0));
        for (i, (k, v)) in pc_entries.iter().enumerate() {
            pc_out.push_str("  \"");
            pc_out.push_str(k);
            pc_out.push_str("\": [");
            for (j, x) in v.iter().enumerate() {
                if j > 0 {
                    pc_out.push(',');
                }
                pc_out.push_str(&format!("{}", x));
            }
            pc_out.push(']');
            if i + 1 < pc_entries.len() {
                pc_out.push(',');
            }
            pc_out.push('\n');
        }
        pc_out.push_str("}\n");
        std::fs::write(&pc_path, pc_out)
            .map_err(|e| format!("write {}: {e}", pc_path.display()))?;
        eprintln!(
            "wrote {} ({} per-channel taps)",
            pc_path.display(),
            pc_entries.len()
        );
    }
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
