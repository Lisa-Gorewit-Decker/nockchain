//! Phase 2.15.2 — Rust GGUF → INT8 streaming converter.
//!
//! The Python `oracle/quantize_streaming.py` does the same work but
//! takes ~60 minutes on a 17 GB Qwen 3.6 27B GGUF because Python's
//! Q4_K/Q6_K dequantization is interpreter-bound. This binary uses
//! `candle-core`'s SIMD-optimized k-quants kernels (NEON on aarch64,
//! AVX2 on x86_64) for the same op, which is 20-50x faster end-to-
//! end.
//!
//! Same on-disk output as the Python converter: manifest.bin +
//! weights.bin + comm_w.hex in `--out` directory. The Rust loader's
//! `Model::load` is the round-trip verification.
//!
//! Currently supports `qwen35` (Qwen 3.6 27B) and `qwen3` (legacy)
//! end-to-end. `gemma4` skipped (shape mismatches in `inp_gate` /
//! `proj` need handling; track in a follow-up).
//!
//! Usage:
//!
//!   cargo run --release -p ai-pow-vi --bin gguf-convert \
//!       --features gguf-convert -- \
//!       --gguf /path/to/model.gguf \
//!       --out  /tmp/model_int8 \
//!       --seq-len 64 \
//!       --activation-tile 64

use std::path::PathBuf;
use std::process::ExitCode;

use ai_pow_vi::activation_lut::{ActivationKind, ActivationLut};
use ai_pow_vi::attention::{AttentionScales, AttentionWeights};
use ai_pow_vi::comm_w::compute_comm_w;
use ai_pow_vi::deltanet::DeltaNetScales;
use ai_pow_vi::ffn::{FfnScales, FfnWeights};
use ai_pow_vi::layer::{LayerWeights, NormSpec};
use ai_pow_vi::model::{arch_tag, Model, ModelDims};
use ai_pow_vi::quant::{Scale, SCALE_DENOM_LOG2};
use ai_pow_vi::rope::RopeTables;
use ai_pow_vi::softmax::ExpLut;

use candle_core::quantized::gguf_file::Content;
use candle_core::Device;

const ROPE_FRACT_BITS: u32 = 14;
const DEFAULT_NORM_EPS_Q: i64 = 1;
const SIGMOID_LUT_KIND: ActivationKind = ActivationKind::SiLU;
const FFN_LUT_KIND: ActivationKind = ActivationKind::SiLU;

struct Args {
    gguf: PathBuf,
    out: PathBuf,
    seq_len: u32,
    activation_tile: u32,
    default_activation_scale_num: i32,
    /// When true, do NOT write manifest.bin / weights.bin / comm_w.hex;
    /// only emit lm_head.bin into `out`. Lets users add `lm_head.bin`
    /// to an existing converted model directory without re-running
    /// the ~25 minute full conversion.
    lm_head_only: bool,
    /// When false (default), skip lm_head.bin emission entirely. Useful
    /// when the GGUF has tied embeddings (output.weight aliases
    /// token_embd.weight) and the embed tensor already carries the
    /// classification head.
    emit_lm_head: bool,
    /// Optional path to a `scales.json` overriding the uniform
    /// `default_activation_scale_num`. Format mirrors the Python
    /// `oracle/calibrate.py` output:
    /// ```json
    /// {
    ///   "activation_scales": {
    ///     "default": 4096,
    ///     "layer[0].attn.q": 64,
    ///     "layer[0].attn.score": 32,
    ///     ...
    ///   }
    /// }
    /// ```
    /// Keys missing from `activation_scales` fall back to `default`
    /// inside `activation_scales` (if present) or to
    /// `--default-activation-scale`. Mirrors the lookup pattern of
    /// `oracle/quantize_qwen.py::_as`.
    scales_path: Option<PathBuf>,
}

/// Activation-scale source. Mirrors the Python `scales["activation_scales"]`
/// dict from `oracle/calibrate.py`: a per-tap key → scale numerator map
/// with a `default` fallback. When no scales.json is supplied, every
/// lookup returns the uniform `--default-activation-scale`.
struct ScaleSource {
    per_tap: std::collections::HashMap<String, i32>,
    default_num: i32,
}

impl ScaleSource {
    fn uniform(num: i32) -> Self {
        Self {
            per_tap: std::collections::HashMap::new(),
            default_num: num,
        }
    }

    fn load(path: &std::path::Path, fallback: i32) -> Result<Self, String> {
        let body = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        let json: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("parse {}: {e}", path.display()))?;
        let mut per_tap = std::collections::HashMap::new();
        let mut default_num = fallback;
        if let Some(obj) = json.get("activation_scales").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(n) = v.as_i64() {
                    let n = n.clamp(1, i32::MAX as i64) as i32;
                    if k == "default" {
                        default_num = n;
                    } else {
                        per_tap.insert(k.clone(), n);
                    }
                }
            }
        }
        eprintln!(
            "loaded scales: {} per-tap entries, default_num={}",
            per_tap.len(),
            default_num
        );
        Ok(Self {
            per_tap,
            default_num,
        })
    }

    /// Look up a tap by name; falls back to `default_num` if missing.
    fn get(&self, tap: &str) -> i32 {
        self.per_tap.get(tap).copied().unwrap_or(self.default_num)
    }

    fn default_num(&self) -> i32 {
        self.default_num
    }
}

fn parse_args() -> Result<Args, String> {
    let mut gguf = None;
    let mut out = None;
    let mut seq_len = 64u32;
    let mut activation_tile = 64u32;
    let mut default_activation_scale_num = 4096i32;
    let mut lm_head_only = false;
    let mut emit_lm_head = true;
    let mut scales_path: Option<PathBuf> = None;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--gguf" => {
                gguf = Some(PathBuf::from(argv.get(i + 1).ok_or("--gguf needs arg")?));
                i += 2;
            }
            "--out" => {
                out = Some(PathBuf::from(argv.get(i + 1).ok_or("--out needs arg")?));
                i += 2;
            }
            "--seq-len" => {
                seq_len = argv.get(i + 1).ok_or("--seq-len needs arg")?.parse().map_err(|_| "bad --seq-len")?;
                i += 2;
            }
            "--activation-tile" => {
                activation_tile = argv.get(i + 1).ok_or("--activation-tile needs arg")?.parse().map_err(|_| "bad --activation-tile")?;
                i += 2;
            }
            "--default-activation-scale" => {
                default_activation_scale_num = argv.get(i + 1).ok_or("--default-activation-scale needs arg")?.parse().map_err(|_| "bad --default-activation-scale")?;
                i += 2;
            }
            "--lm-head-only" => {
                lm_head_only = true;
                emit_lm_head = true;
                i += 1;
            }
            "--no-lm-head" => {
                emit_lm_head = false;
                i += 1;
            }
            "--scales" => {
                scales_path = Some(PathBuf::from(argv.get(i + 1).ok_or("--scales needs arg")?));
                i += 2;
            }
            "-h" | "--help" => {
                eprintln!(
                    "gguf-convert --gguf <path> --out <dir> [--seq-len N] [--activation-tile N] \n  [--default-activation-scale NUM] [--scales scales.json]\n  [--lm-head-only] [--no-lm-head]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    Ok(Args {
        gguf: gguf.ok_or("--gguf required")?,
        out: out.ok_or("--out required")?,
        seq_len,
        activation_tile,
        default_activation_scale_num,
        lm_head_only,
        emit_lm_head,
        scales_path,
    })
}

/// Dequantize one tensor by name to a flat Vec<f32> with the tensor's
/// PyTorch-convention shape `(out, in)` for linear weights.
fn dequant_to_vec_f32(
    content: &Content,
    file: &mut std::fs::File,
    name: &str,
) -> Result<(Vec<f32>, Vec<usize>), String> {
    let qt = content
        .tensor(file, name, &Device::Cpu)
        .map_err(|e| format!("tensor `{name}`: {e}"))?;
    let t = qt
        .dequantize(&Device::Cpu)
        .map_err(|e| format!("dequant `{name}`: {e}"))?;
    let shape: Vec<usize> = t.shape().dims().to_vec();
    let flat = t
        .flatten_all()
        .and_then(|x| x.to_vec1::<f32>())
        .map_err(|e| format!("flatten `{name}`: {e}"))?;
    Ok((flat, shape))
}

/// Quantize an f32 slice to i8 using a single scalar scale.
/// `q = round(x / scale_f32).clamp(-128, 127)`. Matches Python.
fn quantize_to_i8(arr: &[f32], scale_num: i32) -> Vec<i8> {
    let scale_f32 = (scale_num as f64) / (1u64 << SCALE_DENOM_LOG2) as f64;
    let scale_f32 = if scale_f32 > 0.0 { scale_f32 } else { 1.0 / 127.0 };
    arr.iter()
        .map(|&x| {
            let q = (x as f64 / scale_f32).round();
            q.clamp(-128.0, 127.0) as i8
        })
        .collect()
}

/// max(|x|) / 127 → scale numerator (in Scale units, num/2^15).
fn compute_scale_num(arr: &[f32]) -> i32 {
    let max_abs = arr.iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
    let scale_f32 = if max_abs <= 0.0 { 1.0 / 127.0 } else { (max_abs / 127.0) as f64 };
    let n = (scale_f32 * (1u64 << SCALE_DENOM_LOG2) as f64).round() as i64;
    n.clamp(1, i32::MAX as i64) as i32
}

/// `quantize_to_i8` with a freshly-computed scale; returns (i8 bytes, scale_num).
fn quantize_with_scale(arr: &[f32]) -> (Vec<i8>, i32) {
    let s = compute_scale_num(arr);
    (quantize_to_i8(arr, s), s)
}

fn arch_str_from_content(content: &Content) -> Result<String, String> {
    use candle_core::quantized::gguf_file::Value;
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
    use candle_core::quantized::gguf_file::Value;
    match content.metadata.get(key) {
        Some(Value::U32(v)) => Ok(*v),
        Some(Value::U64(v)) => Ok(*v as u32),
        Some(Value::I32(v)) => Ok(*v as u32),
        // Per-layer arrays: pick the LARGEST non-zero value (matches
        // qwen35's convention where hybrid blocks have 0 num_kv and
        // standard blocks have the "real" value).
        Some(Value::Array(arr)) => {
            let mut best = 0u32;
            for v in arr {
                let candidate = match v {
                    Value::U32(x) => *x,
                    Value::U64(x) => *x as u32,
                    Value::I32(x) => *x as u32,
                    _ => continue,
                };
                if candidate > best {
                    best = candidate;
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
    use candle_core::quantized::gguf_file::Value;
    match content.metadata.get(key) {
        Some(Value::F32(v)) => Ok(*v),
        Some(Value::F64(v)) => Ok(*v as f32),
        Some(v) => Err(format!("{key}: unexpected type {v:?}")),
        None => default.ok_or_else(|| format!("missing required field {key}")),
    }
}

struct QwenDims {
    hidden: u32,
    intermediate: u32,
    num_layers: u32,
    num_q_heads: u32,
    num_kv_heads: u32,
    head_dim: u32,
    rope_theta: f32,
    /// IMROPE: number of dims to rotate (rest pass through). For Qwen 3.5
    /// 27B this is 64 (out of head_dim = 256). Falls back to head_dim
    /// for legacy qwen3 / when the GGUF lacks the metadata.
    n_rot: u32,
    /// IMROPE sector lengths `[t, h, w, extra]`. All-zero falls back to
    /// plain NEOX RoPE for compat with qwen3.
    rope_sections: [usize; 4],
}

fn read_qwen35_dims(content: &Content) -> Result<QwenDims, String> {
    let head_dim = meta_u32(content, "qwen35.attention.key_length", None)?;
    let n_rot = meta_u32(content, "qwen35.rope.dimension_count", Some(head_dim))?;
    // Read rope dimension sections array (4 u32s) if present; otherwise zero.
    let mut rope_sections = [0usize; 4];
    if let Some(candle_core::quantized::gguf_file::Value::Array(arr)) =
        content.metadata.get("qwen35.rope.dimension_sections")
    {
        for (i, v) in arr.iter().take(4).enumerate() {
            let n = match v {
                candle_core::quantized::gguf_file::Value::U32(x) => *x as usize,
                candle_core::quantized::gguf_file::Value::U64(x) => *x as usize,
                candle_core::quantized::gguf_file::Value::I32(x) => *x as usize,
                _ => 0,
            };
            rope_sections[i] = n;
        }
    }
    Ok(QwenDims {
        hidden: meta_u32(content, "qwen35.embedding_length", None)?,
        intermediate: meta_u32(content, "qwen35.feed_forward_length", None)?,
        num_layers: meta_u32(content, "qwen35.block_count", None)?,
        num_q_heads: meta_u32(content, "qwen35.attention.head_count", None)?,
        num_kv_heads: meta_u32(content, "qwen35.attention.head_count_kv", None)?,
        head_dim,
        rope_theta: meta_f32(content, "qwen35.rope.freq_base", Some(10_000.0))?,
        n_rot,
        rope_sections,
    })
}

/// Block classifier: hybrid blocks lack `attn_q.weight`; they have
/// `attn_qkv.weight` instead.
fn qwen35_block_is_standard(content: &Content, n: u32) -> bool {
    let want = format!("blk.{n}.attn_q.weight");
    content.tensor_infos.contains_key(&want)
}

/// RMS norm spec helper.
fn make_norm_rms(gamma: Vec<i8>, post_scale_num: i32) -> NormSpec {
    NormSpec::RmsNorm {
        gamma,
        eps_q: DEFAULT_NORM_EPS_Q,
        post_scale: Scale::from_num(post_scale_num).unwrap(),
    }
}

/// Build i16 RoPE tables (mirror of Python `Q.build_rope_tables`).
fn build_rope_tables(seq_len: u32, head_dim: u32, rope_theta: f32) -> RopeTables {
    assert!(head_dim % 2 == 0);
    let half = head_dim / 2;
    let fract = (1u64 << ROPE_FRACT_BITS) as f64;
    let mut cos = Vec::with_capacity((seq_len * half) as usize);
    let mut sin = Vec::with_capacity((seq_len * half) as usize);
    for pos in 0..seq_len {
        for j in 0..half {
            let inv_freq = 1.0 / (rope_theta as f64).powf(2.0 * (j as f64) / head_dim as f64);
            let theta = (pos as f64) * inv_freq;
            let c = (theta.cos() * fract).round().clamp(-32768.0, 32767.0) as i16;
            let s = (theta.sin() * fract).round().clamp(-32768.0, 32767.0) as i16;
            cos.push(c);
            sin.push(s);
        }
    }
    RopeTables {
        seq_len,
        half_head_dim: half,
        cos,
        sin,
    }
}

/// SiLU LUT (mirror of Python `Q.build_silu_lut`).
fn build_silu_lut() -> ActivationLut {
    let scale_x = 1.0 / 8.0;
    let mut bytes = [0u8; 256];
    for i in 0..256 {
        let x = ((i as i32) - 128) as f64 * scale_x;
        let sig = 1.0 / (1.0 + (-x).exp());
        let y = x * sig;
        let q = (y / scale_x).round().clamp(-128.0, 127.0) as i8;
        bytes[i] = q as u8;
    }
    ActivationLut::from_bytes(FFN_LUT_KIND, &bytes).unwrap()
}

fn build_sigmoid_lut() -> ActivationLut {
    let scale_x = 1.0 / 8.0;
    let mut bytes = [0u8; 256];
    for i in 0..256 {
        let x = ((i as i32) - 128) as f64 * scale_x;
        let sig = 1.0 / (1.0 + (-x).exp());
        let q = (sig * 127.0).round().clamp(0.0, 127.0) as i8;
        bytes[i] = q as u8;
    }
    ActivationLut::from_bytes(SIGMOID_LUT_KIND, &bytes).unwrap()
}

fn build_softmax_lut() -> ExpLut {
    ExpLut::uniform_test()
}

/// Build per-layer `AttentionScales` from the scale source. Each tap
/// is looked up by its canonical name `layer[N].attn.{q,k,v,score,attn_out,o}`;
/// missing keys fall back to the source's `default`.
fn attn_scales_for(scales: &ScaleSource, n: u32) -> AttentionScales {
    let tap = |sub: &str| format!("layer[{n}].attn.{sub}");
    AttentionScales {
        q: Scale::from_num(scales.get(&tap("q"))).unwrap(),
        k: Scale::from_num(scales.get(&tap("k"))).unwrap(),
        v: Scale::from_num(scales.get(&tap("v"))).unwrap(),
        score: Scale::from_num(scales.get(&tap("score"))).unwrap(),
        attn_out: Scale::from_num(scales.get(&tap("attn_out"))).unwrap(),
        o: Scale::from_num(scales.get(&tap("o"))).unwrap(),
    }
}

fn ffn_scales_for(scales: &ScaleSource, n: u32) -> FfnScales {
    let tap = |sub: &str| format!("layer[{n}].ffn.{sub}");
    FfnScales {
        gate: Scale::from_num(scales.get(&tap("gate"))).unwrap(),
        up: Scale::from_num(scales.get(&tap("up"))).unwrap(),
        mid: Scale::from_num(scales.get(&tap("mid"))).unwrap(),
        down: Scale::from_num(scales.get(&tap("down"))).unwrap(),
    }
}

fn dnet_scales_for(scales: &ScaleSource, n: u32) -> DeltaNetScales {
    let tap = |sub: &str| format!("layer[{n}].ssm.{sub}");
    DeltaNetScales {
        q: Scale::from_num(scales.get(&tap("q"))).unwrap(),
        k: Scale::from_num(scales.get(&tap("k"))).unwrap(),
        v: Scale::from_num(scales.get(&tap("v"))).unwrap(),
        alpha_logit: Scale::from_num(scales.get(&tap("alpha_logit"))).unwrap(),
        beta_logit: Scale::from_num(scales.get(&tap("beta_logit"))).unwrap(),
        u: Scale::from_num(scales.get(&tap("u"))).unwrap(),
        decay: Scale::from_num(scales.get(&tap("decay"))).unwrap(),
        update: Scale::from_num(scales.get(&tap("update"))).unwrap(),
        o: Scale::from_num(scales.get(&tap("o"))).unwrap(),
        proj: Scale::from_num(scales.get(&tap("proj"))).unwrap(),
    }
}

/// Convert one f32 vec → i8 vec using a freshly-computed scale,
/// returning the i8 bytes for the canonical write order. Used for
/// 1-D weights (norms, single-axis biases).
fn dequant_quantize(
    content: &Content,
    file: &mut std::fs::File,
    name: &str,
) -> Result<Vec<i8>, String> {
    let (f, _shape) = dequant_to_vec_f32(content, file, name)?;
    let (i8s, _scale) = quantize_with_scale(&f);
    Ok(i8s)
}

/// Like `dequant_quantize` but truncates to first `keep_len` elements
/// of `axis=0` (which after dequantize is the "out" dim for linear
/// weights, since shape is `(out, in)`).
fn dequant_quantize_truncate_out(
    content: &Content,
    file: &mut std::fs::File,
    name: &str,
    target_out: usize,
) -> Result<Vec<i8>, String> {
    let (f, shape) = dequant_to_vec_f32(content, file, name)?;
    if shape.len() != 2 || shape[0] == target_out {
        // No truncation needed.
        let (i8s, _) = quantize_with_scale(&f);
        return Ok(i8s);
    }
    let (out_dim, in_dim) = (shape[0], shape[1]);
    if target_out > out_dim {
        return Err(format!("{name}: target_out {target_out} > actual out {out_dim}"));
    }
    let mut sub = Vec::with_capacity(target_out * in_dim);
    sub.extend_from_slice(&f[0..target_out * in_dim]);
    let (i8s, _) = quantize_with_scale(&sub);
    Ok(i8s)
}

/// Generic helper: dequantize, quantize, return.
fn default_no_op_gamma_i8(len: usize) -> Vec<i8> {
    vec![127i8; len]
}

/// Stream-emit `lm_head.bin` to `out_dir`. The lm_head tensor is
/// dequantized through candle, quantized to i8 via the same
/// `max(|w|)/127` weight scale convention as the rest of the model,
/// and written to disk in `(vocab, hidden)` row-major order. Memory
/// peak is bounded by one tensor at a time (~1.3 GB f32 for Qwen 3.6
/// 27B's output.weight).
///
/// Returns `(true, vocab, hidden)` if the GGUF carries an explicit
/// `output.weight` tensor; `(false, ...)` if it doesn't (tied
/// embeddings — caller should fall back to using the embed table as
/// the lm_head).
fn emit_lm_head(
    content: &Content,
    file: &mut std::fs::File,
    out_dir: &std::path::Path,
    expected_hidden: u32,
) -> Result<bool, String> {
    let tname = "output.weight";
    if !content.tensor_infos.contains_key(tname) {
        eprintln!(
            "  no output.weight in GGUF; lm_head.bin not emitted (model likely uses tied embeddings)"
        );
        return Ok(false);
    }
    eprintln!("→ emit_lm_head");
    let (f, shape) = dequant_to_vec_f32(content, file, tname)?;
    if shape.len() != 2 {
        return Err(format!("output.weight has unexpected rank: {shape:?}"));
    }
    let (vocab_dim, hidden_dim) = (shape[0], shape[1]);
    if hidden_dim as u32 != expected_hidden {
        return Err(format!(
            "output.weight hidden {} != model hidden {}",
            hidden_dim, expected_hidden
        ));
    }
    let (i8s, scale_num) = quantize_with_scale(&f);
    let lm_head_path = out_dir.join("lm_head.bin");
    let bytes: Vec<u8> = i8s.iter().map(|&v| v as u8).collect();
    std::fs::write(&lm_head_path, &bytes).map_err(|e| format!("write lm_head.bin: {e}"))?;
    eprintln!(
        "  wrote {} ({} bytes, vocab={} hidden={}, scale_num={})",
        lm_head_path.display(),
        bytes.len(),
        vocab_dim,
        hidden_dim,
        scale_num,
    );
    Ok(true)
}

fn convert_qwen35(args: &Args) -> Result<(), String> {
    use candle_core::quantized::gguf_file;
    let mut file = std::fs::File::open(&args.gguf).map_err(|e| format!("open gguf: {e}"))?;
    eprintln!("opening GGUF: {}", args.gguf.display());
    let content = gguf_file::Content::read(&mut file).map_err(|e| format!("read gguf: {e}"))?;
    let arch = arch_str_from_content(&content)?;
    if arch != "qwen35" && arch != "qwen3" {
        return Err(format!("this binary supports qwen35/qwen3 only; got arch={arch}"));
    }
    let dims = read_qwen35_dims(&content)?;
    eprintln!(
        "arch={} num_layers={} hidden={} intermediate={} num_q={} num_kv={} head_dim={} rope_theta={}",
        arch, dims.num_layers, dims.hidden, dims.intermediate,
        dims.num_q_heads, dims.num_kv_heads, dims.head_dim, dims.rope_theta
    );

    // --lm-head-only short-circuit: skip the model conversion entirely
    // and just emit lm_head.bin into the existing output directory.
    if args.lm_head_only {
        std::fs::create_dir_all(&args.out).map_err(|e| format!("mkdir: {e}"))?;
        emit_lm_head(&content, &mut file, &args.out, dims.hidden)?;
        return Ok(());
    }

    let scales = match &args.scales_path {
        Some(p) => ScaleSource::load(p, args.default_activation_scale_num)?,
        None => ScaleSource::uniform(args.default_activation_scale_num),
    };

    // Embed.
    eprintln!("→ embed");
    let embed = dequant_quantize(&content, &mut file, "token_embd.weight")?;
    let vocab = embed.len() as u32 / dims.hidden;
    eprintln!("  vocab = {}", vocab);

    // final_norm. The `final_norm_post` tap controls its post-scale.
    let final_norm = if content.tensor_infos.contains_key("output_norm.weight") {
        let g = dequant_quantize(&content, &mut file, "output_norm.weight")?;
        let post = scales.get("final_norm_post");
        Some(make_norm_rms(g, post))
    } else {
        None
    };

    let mut layers: Vec<LayerWeights> = Vec::with_capacity(dims.num_layers as usize);
    for n in 0..dims.num_layers {
        let is_std = qwen35_block_is_standard(&content, n);
        if n % 4 == 0 || n + 1 == dims.num_layers {
            eprintln!(
                "→ layer {} ({})",
                n,
                if is_std { "standard" } else { "hybrid" }
            );
        }
        let layer = if is_std {
            build_qwen_standard_layer(&content, &mut file, n, &dims, &scales)?
        } else {
            build_qwen_hybrid_layer(&content, &mut file, n, &dims, &scales)?
        };
        layers.push(layer);
    }

    eprintln!("→ rope tables, LUTs");
    // For qwen35 with non-empty rope_sections metadata, build IMROPE tables
    // (interleaved multi-axis RoPE on the first `n_rot` dims). All other
    // archs use plain NEOX RoPE — IMROPE with empty sections falls back to
    // NEOX, so this is safe for qwen3 too.
    let rope_tables = ai_pow_vi::rope::build_imrope_tables(
        args.seq_len,
        dims.head_dim,
        dims.n_rot,
        dims.rope_sections,
        dims.rope_theta,
    );
    let softmax_lut = build_softmax_lut();
    let sigmoid_lut = build_sigmoid_lut();
    let ffn_activation = build_silu_lut();

    eprintln!("→ Model::save → {}", args.out.display());
    let model = Model {
        dims: ModelDims {
            vocab,
            hidden: dims.hidden,
            seq_len: args.seq_len,
            activation_tile: args.activation_tile,
        },
        arch_tag: arch_tag(&arch),
        feature_flags: 0,
        embed,
        layers,
        final_norm,
        rope_tables,
        softmax_lut,
        sigmoid_lut,
        ffn_activation,
    };
    std::fs::create_dir_all(&args.out).map_err(|e| format!("mkdir: {e}"))?;
    model.save(&args.out).map_err(|e| format!("save: {e}"))?;
    let comm = compute_comm_w(&model);
    std::fs::write(args.out.join("comm_w.hex"), comm.iter().map(|b| format!("{:02x}", b)).collect::<String>()).map_err(|e| format!("write comm_w: {e}"))?;
    eprintln!("comm_W = {}", comm.iter().map(|b| format!("{:02x}", b)).collect::<String>());

    if args.emit_lm_head {
        emit_lm_head(&content, &mut file, &args.out, dims.hidden)?;
    } else {
        eprintln!("--no-lm-head: skipping lm_head.bin emission");
    }
    Ok(())
}

fn build_qwen_standard_layer(
    content: &Content,
    file: &mut std::fs::File,
    n: u32,
    dims: &QwenDims,
    scales: &ScaleSource,
) -> Result<LayerWeights, String> {
    let h = dims.hidden as usize;
    let hd = dims.head_dim as usize;
    let nq = dims.num_q_heads as usize;
    let nkv = dims.num_kv_heads as usize;
    let q_target = nq * hd;
    let prefix = format!("blk.{n}");
    let norm1_post = scales.get(&format!("layer[{n}].norm_post.1"));
    let norm2_post = scales.get(&format!("layer[{n}].norm_post.2"));
    let qk_norm_post = scales.get(&format!("layer[{n}].qk_norm_post"));

    let norm1_g = dequant_quantize(content, file, &format!("{prefix}.attn_norm.weight"))?;
    // Phase B.2: real Qwen 3.6 27B std blocks pack [Q || gate] in attn_q.weight,
    // doubling the output dim. Detect and keep the full tensor instead of
    // truncating away the gate half.
    let wq_info = content
        .tensor_infos
        .get(&format!("{prefix}.attn_q.weight"))
        .ok_or_else(|| format!("layer {n} std missing attn_q.weight"))?;
    let wq_out = wq_info.shape.dims()[0];
    let q_has_gate = if wq_out == 2 * q_target {
        true
    } else if wq_out == q_target {
        false
    } else {
        return Err(format!(
            "layer {n} std attn_q out dim {wq_out} matches neither q_target {q_target} nor 2*q_target {}",
            2 * q_target
        ));
    };
    let w_q = dequant_quantize(content, file, &format!("{prefix}.attn_q.weight"))?;
    let w_k = dequant_quantize(content, file, &format!("{prefix}.attn_k.weight"))?;
    let w_v = dequant_quantize(content, file, &format!("{prefix}.attn_v.weight"))?;
    let w_o = dequant_quantize(content, file, &format!("{prefix}.attn_output.weight"))?;
    let q_norm = dequant_quantize(content, file, &format!("{prefix}.attn_q_norm.weight"))
        .unwrap_or_else(|_| default_no_op_gamma_i8(hd));
    let k_norm = dequant_quantize(content, file, &format!("{prefix}.attn_k_norm.weight"))
        .unwrap_or_else(|_| default_no_op_gamma_i8(hd));
    let norm2_g = dequant_quantize(content, file, &format!("{prefix}.post_attention_norm.weight"))?;
    let ffn_gate = dequant_quantize(content, file, &format!("{prefix}.ffn_gate.weight"))?;
    let ffn_up = dequant_quantize(content, file, &format!("{prefix}.ffn_up.weight"))?;
    let ffn_down = dequant_quantize(content, file, &format!("{prefix}.ffn_down.weight"))?;

    // Verify shapes match what our struct expects.
    let want_wq = if q_has_gate { 2 * h * q_target } else { h * q_target };
    if w_q.len() != want_wq {
        return Err(format!(
            "layer {n} std w_q len {} != expected {} (q_has_gate={q_has_gate})",
            w_q.len(),
            want_wq
        ));
    }
    let want_wo = q_target * h;
    if w_o.len() != want_wo {
        // The real attn_output might be (q_target, hidden) directly = q_target*hidden bytes.
        return Err(format!("layer {n} std w_o len {} != q_target*hidden {}", w_o.len(), want_wo));
    }

    Ok(LayerWeights::QwenStandard {
        norm1: make_norm_rms(norm1_g, norm1_post),
        attn: AttentionWeights {
            hidden: dims.hidden,
            num_q_heads: dims.num_q_heads,
            num_kv_heads: dims.num_kv_heads,
            head_dim: dims.head_dim,
            w_q,
            w_k,
            w_v,
            w_o,
            q_has_gate,
        },
        attn_scales: attn_scales_for(scales, n),
        q_norm_gamma: q_norm,
        k_norm_gamma: k_norm,
        qk_norm_eps_q: DEFAULT_NORM_EPS_Q,
        qk_norm_post_scale: Scale::from_num(qk_norm_post).unwrap(),
        norm2: make_norm_rms(norm2_g, norm2_post),
        ffn: FfnWeights {
            hidden: dims.hidden,
            intermediate: dims.intermediate,
            w_gate: ffn_gate,
            w_up: ffn_up,
            w_down: ffn_down,
        },
        ffn_scales: ffn_scales_for(scales, n),
    })
}

fn build_qwen_hybrid_layer(
    content: &Content,
    file: &mut std::fs::File,
    n: u32,
    dims: &QwenDims,
    scales: &ScaleSource,
) -> Result<LayerWeights, String> {
    let h = dims.hidden as usize;
    let hd = dims.head_dim as usize;
    let nq = dims.num_q_heads as usize;
    let prefix = format!("blk.{n}");
    let norm1_post = scales.get(&format!("layer[{n}].norm_post.1"));
    let norm2_post = scales.get(&format!("layer[{n}].norm_post.2"));
    let qk_norm_post = scales.get(&format!("layer[{n}].qk_norm_post"));
    let ssm_norm_post = scales.get(&format!("layer[{n}].ssm_norm_post"));

    let norm1_g = dequant_quantize(content, file, &format!("{prefix}.attn_norm.weight"))?;
    // Detect num_kv per-block from attn_qkv shape: total = q_dim + 2*kv_dim.
    let qkv_info = content
        .tensor_infos
        .get(&format!("{prefix}.attn_qkv.weight"))
        .ok_or_else(|| format!("layer {n} hybrid missing attn_qkv.weight"))?;
    let qkv_shape: Vec<usize> = qkv_info.shape.dims().to_vec();
    // GGUF native shape: innermost-first. For a (out, in) HF matrix it's (in, out).
    // After candle's internal handling, `shape.dims()` returns the PyTorch-style
    // (out, in) order. So qkv_shape[0] = out, [1] = in.
    let qkv_out = qkv_shape[0];
    let q_dim = nq * hd;
    let kv_dim = (qkv_out - q_dim) / 2;
    let num_kv = (kv_dim / hd) as u32;
    let attn_qkv = dequant_quantize(content, file, &format!("{prefix}.attn_qkv.weight"))?;
    let attn_gate = dequant_quantize(content, file, &format!("{prefix}.attn_gate.weight"))?;
    // attn_out: real Qwen 3.6 hybrid blocks have NO separate attn_output; the
    // ssm_out projection is shared. We alias ssm_out bytes here.
    let attn_out = dequant_quantize(content, file, &format!("{prefix}.ssm_out.weight"))?;
    let q_norm = default_no_op_gamma_i8(hd);
    let k_norm = default_no_op_gamma_i8(hd);

    // SSM tensors.
    let ssm_a = dequant_quantize(content, file, &format!("{prefix}.ssm_a"))?;
    let num_v = ssm_a.len() as u32;
    let ssm_alpha = dequant_quantize(content, file, &format!("{prefix}.ssm_alpha.weight"))?;
    let ssm_beta = dequant_quantize(content, file, &format!("{prefix}.ssm_beta.weight"))?;
    let ssm_dt = dequant_quantize(content, file, &format!("{prefix}.ssm_dt"))?;
    let ssm_norm_g = dequant_quantize(content, file, &format!("{prefix}.ssm_norm.weight"))?;
    let ssm_head_dim = ssm_norm_g.len() as u32;
    // ssm_conv1d: GGUF native shape is (kernel_size, channels) with
    // kernel_size innermost. Candle's dims() reverses → (channels,
    // kernel_size). Data layout in conv_f is (channels, kernel_size)
    // row-major. We want (kernel_size, hidden) row-major for Rust's
    // SSM forward, so we transpose AND truncate channels to hidden.
    let (conv_f, conv_shape) = dequant_to_vec_f32(content, file, &format!("{prefix}.ssm_conv1d.weight"))?;
    if conv_shape.len() != 2 {
        return Err(format!("layer {n} hybrid ssm_conv1d has wrong rank: {conv_shape:?}"));
    }
    let conv_channels = conv_shape[0];
    let kernel_size = conv_shape[1];
    if conv_channels < h {
        return Err(format!(
            "layer {n} hybrid ssm_conv1d channels {} < hidden {}",
            conv_channels, h
        ));
    }
    // Transpose+truncate: out[k * h + c] = conv_f[c * kernel_size + k]
    let mut conv_truncated = vec![0.0f32; kernel_size * h];
    for c in 0..h {
        for k in 0..kernel_size {
            conv_truncated[k * h + c] = conv_f[c * kernel_size + k];
        }
    }
    let (ssm_conv1d, _) = quantize_with_scale(&conv_truncated);

    let ssm_out = dequant_quantize(content, file, &format!("{prefix}.ssm_out.weight"))?;
    let norm2_g = dequant_quantize(content, file, &format!("{prefix}.post_attention_norm.weight"))?;
    let ffn_gate = dequant_quantize(content, file, &format!("{prefix}.ffn_gate.weight"))?;
    let ffn_up = dequant_quantize(content, file, &format!("{prefix}.ffn_up.weight"))?;
    let ffn_down = dequant_quantize(content, file, &format!("{prefix}.ffn_down.weight"))?;

    Ok(LayerWeights::QwenHybridSsm {
        norm1: make_norm_rms(norm1_g, norm1_post),
        attn_qkv_fused: attn_qkv,
        attn_gate,
        attn_out,
        num_q_heads: dims.num_q_heads,
        num_kv_heads: num_kv,
        head_dim: dims.head_dim,
        attn_scales: attn_scales_for(scales, n),
        q_norm_gamma: q_norm,
        k_norm_gamma: k_norm,
        qk_norm_eps_q: DEFAULT_NORM_EPS_Q,
        qk_norm_post_scale: Scale::from_num(qk_norm_post).unwrap(),
        ssm_a,
        ssm_alpha,
        ssm_beta,
        ssm_conv1d,
        ssm_dt,
        ssm_norm_gamma: ssm_norm_g,
        ssm_norm_eps_q: DEFAULT_NORM_EPS_Q,
        ssm_norm_post_scale: Scale::from_num(ssm_norm_post).unwrap(),
        ssm_out,
        num_v_heads: num_v,
        ssm_head_dim,
        ssm_kernel_size: kernel_size as u32,
        ssm_scales: dnet_scales_for(scales, n),
        norm2: make_norm_rms(norm2_g, norm2_post),
        ffn: FfnWeights {
            hidden: dims.hidden,
            intermediate: dims.intermediate,
            w_gate: ffn_gate,
            w_up: ffn_up,
            w_down: ffn_down,
        },
        ffn_scales: ffn_scales_for(scales, n),
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
    let start = std::time::Instant::now();
    if let Err(e) = convert_qwen35(&args) {
        eprintln!("conversion failed: {e}");
        return ExitCode::from(1);
    }
    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    ExitCode::SUCCESS
}
