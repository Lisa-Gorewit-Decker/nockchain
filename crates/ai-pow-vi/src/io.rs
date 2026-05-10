//! Sideloaded model directory format and `Model::load` / `Model::save`.
//!
//! Disk layout:
//!
//! ```text
//! $NOCKCHAIN_DATA/models/<model_id_hex>/
//!   manifest.bin   # architecture + every Scale + every eps_q
//!   weights.bin    # exact bytes of `canonical_weight_bytes(model)` —
//!                  # i8 weight tensors + LUTs + RoPE tables in canonical order.
//!   comm_w.hex     # 64-char hex; sanity only (the loader recomputes
//!                  # comm_W and compares to the caller-provided expected).
//! ```
//!
//! `manifest.bin` is a hand-rolled little-endian binary stream — same
//! style as `crate::proof` and `crate::comm_w`. No serde dependency, no
//! JSON. The format starts with an 8-byte magic and a u32 version so a
//! future incompatible change is a hard error rather than a silent
//! mis-decode.
//!
//! Loading is the only chokepoint that protects all downstream code from
//! tampered-at-rest weights: after parsing, [`Model::load`] recomputes
//! `comm_W` and aborts on mismatch with `expected_comm_w`. The
//! `comm_w.hex` file is a sanity helper for humans; the loader does not
//! trust it.

use std::path::{Path, PathBuf};
use std::{fs, io};

use thiserror::Error;

use crate::activation_lut::{ActivationKind, ActivationLut, LutError};
use crate::attention::{AttentionScales, AttentionWeights};
use crate::comm_w::{canonical_weight_bytes, compute_comm_w};
use crate::deltanet::{DeltaNetScales, DeltaNetWeights};
use crate::ffn::{FfnScales, FfnWeights};
use crate::layer::{LayerWeights, NormSpec};
use crate::model::{ArchTag, FeatureFlags, Model, ModelDims};
use crate::quant::{QuantError, Scale};
use crate::rope::{RopeError, RopeTables};
use crate::softmax::{ExpLut, ExpLutError};

const MAGIC: &[u8; 8] = b"AIPOWVI1";
/// Manifest format version. Bumped to 2 in Phase 2.10 to add
/// `arch_tag: [u8; 16]` and `feature_flags: u64` immediately after the
/// version field. Loaders reject mismatched versions.
const VERSION: u32 = 2;

const MANIFEST_FILENAME: &str = "manifest.bin";
const WEIGHTS_FILENAME: &str = "weights.bin";
const COMM_W_HEX_FILENAME: &str = "comm_w.hex";

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("magic mismatch: file is not a valid ai-pow-vi model manifest")]
    BadMagic,
    #[error("manifest version {0} unsupported (expected {VERSION})")]
    UnsupportedVersion(u32),
    #[error("unexpected EOF while parsing manifest at offset {0}")]
    ManifestEof(u64),
    #[error("unexpected EOF while parsing weights at offset {0}")]
    WeightsEof(u64),
    #[error("trailing bytes in manifest after parse")]
    ManifestTrailing,
    #[error("trailing bytes in weights after parse")]
    WeightsTrailing,
    #[error("unknown norm tag {0}")]
    UnknownNormTag(u8),
    #[error("unknown layer tag {0}")]
    UnknownLayerTag(u8),
    #[error("layer hidden ({got}) does not match model hidden ({want}) at layer {layer_idx}")]
    LayerHiddenMismatch { layer_idx: u32, got: u32, want: u32 },
    #[error("comm_W mismatch: file is corrupt or model_id is wrong")]
    CommWMismatch,
    #[error("scale: {0}")]
    Scale(#[from] QuantError),
    #[error("activation LUT: {0}")]
    Lut(#[from] LutError),
    #[error("exp LUT: {0}")]
    ExpLut(#[from] ExpLutError),
    #[error("rope tables: {0}")]
    Rope(#[from] RopeError),
}

// =========================================================================
// Public API (proxied through Model::save / Model::load in model.rs).
// =========================================================================

pub fn save_model(model: &Model, dir: &Path) -> Result<(), SaveError> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(MANIFEST_FILENAME), encode_manifest(model))?;
    fs::write(dir.join(WEIGHTS_FILENAME), canonical_weight_bytes(model))?;
    let comm_w = compute_comm_w(model);
    fs::write(dir.join(COMM_W_HEX_FILENAME), hex_encode(&comm_w))?;
    Ok(())
}

pub fn load_model(dir: &Path, expected_comm_w: &[u8; 32]) -> Result<Model, LoadError> {
    let manifest = fs::read(dir.join(MANIFEST_FILENAME))
        .map_err(|e| io_with_path(e, dir.join(MANIFEST_FILENAME)))?;
    let weights = fs::read(dir.join(WEIGHTS_FILENAME))
        .map_err(|e| io_with_path(e, dir.join(WEIGHTS_FILENAME)))?;

    let parsed = parse_manifest(&manifest)?;
    let model = parse_weights(parsed, &weights)?;

    let actual_comm_w = compute_comm_w(&model);
    if &actual_comm_w != expected_comm_w {
        return Err(LoadError::CommWMismatch);
    }
    Ok(model)
}

fn io_with_path(e: io::Error, _path: PathBuf) -> LoadError {
    LoadError::Io(e)
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

// =========================================================================
// Manifest encoding.
// =========================================================================

fn encode_manifest(model: &Model) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());

    // v2: arch_tag (16 bytes) + feature_flags (u64 LE).
    buf.extend_from_slice(&model.arch_tag);
    buf.extend_from_slice(&model.feature_flags.to_le_bytes());

    let d = model.dims;
    buf.extend_from_slice(&d.vocab.to_le_bytes());
    buf.extend_from_slice(&d.hidden.to_le_bytes());
    buf.extend_from_slice(&d.seq_len.to_le_bytes());
    buf.extend_from_slice(&d.activation_tile.to_le_bytes());

    buf.extend_from_slice(&(model.layers.len() as u32).to_le_bytes());
    for layer in &model.layers {
        match layer {
            LayerWeights::Attention {
                norm1,
                attn,
                attn_scales,
                norm2,
                ffn,
                ffn_scales,
            } => {
                buf.push(0u8);
                encode_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&attn.hidden.to_le_bytes());
                buf.extend_from_slice(&attn.num_q_heads.to_le_bytes());
                buf.extend_from_slice(&attn.num_kv_heads.to_le_bytes());
                buf.extend_from_slice(&attn.head_dim.to_le_bytes());
                buf.extend_from_slice(&attn_scales.q.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.k.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.v.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.score.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.attn_out.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.o.num.to_le_bytes());
                encode_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.gate.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.up.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.mid.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.down.num.to_le_bytes());
            }
            LayerWeights::DeltaNet {
                norm1,
                dnet,
                dnet_scales,
                norm2,
                ffn,
                ffn_scales,
            } => {
                buf.push(1u8);
                encode_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&dnet.hidden.to_le_bytes());
                buf.extend_from_slice(&dnet.num_qk_heads.to_le_bytes());
                buf.extend_from_slice(&dnet.num_v_heads.to_le_bytes());
                buf.extend_from_slice(&dnet.head_dim_qk.to_le_bytes());
                buf.extend_from_slice(&dnet.head_dim_v.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.q.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.k.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.v.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.alpha_logit.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.beta_logit.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.u.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.decay.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.update.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.o.num.to_le_bytes());
                buf.extend_from_slice(&dnet_scales.proj.num.to_le_bytes());
                encode_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.gate.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.up.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.mid.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.down.num.to_le_bytes());
            }
            LayerWeights::Gemma {
                norm1,
                attn,
                attn_scales,
                q_norm_gamma: _,
                k_norm_gamma: _,
                qk_norm_eps_q,
                qk_norm_post_scale,
                post_attn_norm,
                norm2,
                ffn,
                ffn_scales,
                post_ffn_norm,
                sliding_window,
                inp_gate,
                layer_output_scale,
            } => {
                buf.push(2u8);
                encode_norm_meta(&mut buf, norm1);
                buf.extend_from_slice(&attn.hidden.to_le_bytes());
                buf.extend_from_slice(&attn.num_q_heads.to_le_bytes());
                buf.extend_from_slice(&attn.num_kv_heads.to_le_bytes());
                buf.extend_from_slice(&attn.head_dim.to_le_bytes());
                buf.extend_from_slice(&attn_scales.q.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.k.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.v.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.score.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.attn_out.num.to_le_bytes());
                buf.extend_from_slice(&attn_scales.o.num.to_le_bytes());
                buf.extend_from_slice(&qk_norm_eps_q.to_le_bytes());
                buf.extend_from_slice(&qk_norm_post_scale.num.to_le_bytes());
                encode_norm_meta(&mut buf, post_attn_norm);
                encode_norm_meta(&mut buf, norm2);
                buf.extend_from_slice(&ffn.intermediate.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.gate.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.up.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.mid.num.to_le_bytes());
                buf.extend_from_slice(&ffn_scales.down.num.to_le_bytes());
                encode_norm_meta(&mut buf, post_ffn_norm);
                buf.extend_from_slice(&sliding_window.unwrap_or(0).to_le_bytes());
                buf.push(if inp_gate.is_some() { 1 } else { 0 });
                buf.push(if layer_output_scale.is_some() { 1 } else { 0 });
            }
        }
    }

    if let Some(fn_norm) = &model.final_norm {
        buf.push(1u8);
        encode_norm_meta(&mut buf, fn_norm);
    } else {
        buf.push(0u8);
    }

    // RoPE table shape (the actual cos/sin bytes live in weights.bin).
    buf.extend_from_slice(&model.rope_tables.seq_len.to_le_bytes());
    buf.extend_from_slice(&model.rope_tables.half_head_dim.to_le_bytes());

    // Activation LUT kind tags (the LUT bytes live in weights.bin).
    buf.push(activation_kind_tag(model.ffn_activation.kind));
    buf.push(activation_kind_tag(model.sigmoid_lut.kind));

    buf
}

fn encode_norm_meta(buf: &mut Vec<u8>, norm: &NormSpec) {
    match norm {
        NormSpec::RmsNorm {
            eps_q, post_scale, ..
        } => {
            buf.push(0u8);
            buf.extend_from_slice(&eps_q.to_le_bytes());
            buf.extend_from_slice(&post_scale.num.to_le_bytes());
        }
        NormSpec::LayerNorm {
            eps_q, post_scale, ..
        } => {
            buf.push(1u8);
            buf.extend_from_slice(&eps_q.to_le_bytes());
            buf.extend_from_slice(&post_scale.num.to_le_bytes());
        }
    }
}

fn activation_kind_tag(k: ActivationKind) -> u8 {
    match k {
        ActivationKind::SiLU => 0x01,
        ActivationKind::GeLU => 0x02,
        ActivationKind::Swish => 0x03,
        ActivationKind::Identity => 0xff,
    }
}

fn activation_kind_from_tag(t: u8) -> ActivationKind {
    match t {
        0x01 => ActivationKind::SiLU,
        0x02 => ActivationKind::GeLU,
        0x03 => ActivationKind::Swish,
        _ => ActivationKind::Identity,
    }
}

// =========================================================================
// Manifest parse.
// =========================================================================

/// Intermediate representation: parsed manifest, ready to be paired with
/// the weights byte stream to construct a Model.
struct ParsedManifest {
    arch_tag: ArchTag,
    feature_flags: FeatureFlags,
    dims: ModelDims,
    layer_metas: Vec<LayerMeta>,
    final_norm: Option<NormMeta>,
    rope_seq_len: u32,
    rope_half_head_dim: u32,
    ffn_activation_kind: ActivationKind,
    sigmoid_lut_kind: ActivationKind,
}

#[derive(Debug, Clone)]
enum LayerMeta {
    Attention {
        norm1: NormMeta,
        hidden: u32,
        num_q_heads: u32,
        num_kv_heads: u32,
        head_dim: u32,
        attn_scales: AttentionScales,
        norm2: NormMeta,
        intermediate: u32,
        ffn_scales: FfnScales,
    },
    DeltaNet {
        norm1: NormMeta,
        hidden: u32,
        num_qk_heads: u32,
        num_v_heads: u32,
        head_dim_qk: u32,
        head_dim_v: u32,
        dnet_scales: DeltaNetScales,
        norm2: NormMeta,
        intermediate: u32,
        ffn_scales: FfnScales,
    },
    Gemma {
        norm1: NormMeta,
        hidden: u32,
        num_q_heads: u32,
        num_kv_heads: u32,
        head_dim: u32,
        attn_scales: AttentionScales,
        qk_norm_eps_q: i64,
        qk_norm_post_scale: Scale,
        post_attn_norm: NormMeta,
        norm2: NormMeta,
        intermediate: u32,
        ffn_scales: FfnScales,
        post_ffn_norm: NormMeta,
        sliding_window: u32, // 0 = full causal
        has_inp_gate: bool,
        has_layer_output_scale: bool,
    },
}

#[derive(Debug, Clone)]
enum NormMeta {
    Rms { eps_q: i64, post_scale: Scale },
    LayerNorm { eps_q: i64, post_scale: Scale },
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        if self.pos + n > self.bytes.len() {
            return Err(LoadError::ManifestEof(self.pos as u64));
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, LoadError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, LoadError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Result<i32, LoadError> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i64(&mut self) -> Result<i64, LoadError> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }
}

fn parse_manifest(bytes: &[u8]) -> Result<ParsedManifest, LoadError> {
    let mut c = Cursor::new(bytes);
    let magic = c.take(8)?;
    if magic != MAGIC {
        return Err(LoadError::BadMagic);
    }
    let version = c.u32()?;
    if version != VERSION {
        return Err(LoadError::UnsupportedVersion(version));
    }

    let arch_tag_bytes = c.take(16)?;
    let mut arch_tag: ArchTag = [0u8; 16];
    arch_tag.copy_from_slice(arch_tag_bytes);
    let ff_bytes = c.take(8)?;
    let feature_flags = u64::from_le_bytes([
        ff_bytes[0], ff_bytes[1], ff_bytes[2], ff_bytes[3], ff_bytes[4], ff_bytes[5], ff_bytes[6],
        ff_bytes[7],
    ]);

    let dims = ModelDims {
        vocab: c.u32()?,
        hidden: c.u32()?,
        seq_len: c.u32()?,
        activation_tile: c.u32()?,
    };

    let n_layers = c.u32()?;
    let mut layer_metas = Vec::with_capacity(n_layers as usize);
    for layer_idx in 0..n_layers {
        let tag = c.u8()?;
        match tag {
            0 => {
                let norm1 = parse_norm_meta(&mut c)?;
                let hidden = c.u32()?;
                if hidden != dims.hidden {
                    return Err(LoadError::LayerHiddenMismatch {
                        layer_idx,
                        got: hidden,
                        want: dims.hidden,
                    });
                }
                let num_q_heads = c.u32()?;
                let num_kv_heads = c.u32()?;
                let head_dim = c.u32()?;
                let attn_scales = AttentionScales {
                    q: Scale::from_num(c.i32()?)?,
                    k: Scale::from_num(c.i32()?)?,
                    v: Scale::from_num(c.i32()?)?,
                    score: Scale::from_num(c.i32()?)?,
                    attn_out: Scale::from_num(c.i32()?)?,
                    o: Scale::from_num(c.i32()?)?,
                };
                let norm2 = parse_norm_meta(&mut c)?;
                let intermediate = c.u32()?;
                let ffn_scales = FfnScales {
                    gate: Scale::from_num(c.i32()?)?,
                    up: Scale::from_num(c.i32()?)?,
                    mid: Scale::from_num(c.i32()?)?,
                    down: Scale::from_num(c.i32()?)?,
                };
                layer_metas.push(LayerMeta::Attention {
                    norm1,
                    hidden,
                    num_q_heads,
                    num_kv_heads,
                    head_dim,
                    attn_scales,
                    norm2,
                    intermediate,
                    ffn_scales,
                });
            }
            1 => {
                let norm1 = parse_norm_meta(&mut c)?;
                let hidden = c.u32()?;
                if hidden != dims.hidden {
                    return Err(LoadError::LayerHiddenMismatch {
                        layer_idx,
                        got: hidden,
                        want: dims.hidden,
                    });
                }
                let num_qk_heads = c.u32()?;
                let num_v_heads = c.u32()?;
                let head_dim_qk = c.u32()?;
                let head_dim_v = c.u32()?;
                let dnet_scales = DeltaNetScales {
                    q: Scale::from_num(c.i32()?)?,
                    k: Scale::from_num(c.i32()?)?,
                    v: Scale::from_num(c.i32()?)?,
                    alpha_logit: Scale::from_num(c.i32()?)?,
                    beta_logit: Scale::from_num(c.i32()?)?,
                    u: Scale::from_num(c.i32()?)?,
                    decay: Scale::from_num(c.i32()?)?,
                    update: Scale::from_num(c.i32()?)?,
                    o: Scale::from_num(c.i32()?)?,
                    proj: Scale::from_num(c.i32()?)?,
                };
                let norm2 = parse_norm_meta(&mut c)?;
                let intermediate = c.u32()?;
                let ffn_scales = FfnScales {
                    gate: Scale::from_num(c.i32()?)?,
                    up: Scale::from_num(c.i32()?)?,
                    mid: Scale::from_num(c.i32()?)?,
                    down: Scale::from_num(c.i32()?)?,
                };
                layer_metas.push(LayerMeta::DeltaNet {
                    norm1,
                    hidden,
                    num_qk_heads,
                    num_v_heads,
                    head_dim_qk,
                    head_dim_v,
                    dnet_scales,
                    norm2,
                    intermediate,
                    ffn_scales,
                });
            }
            2 => {
                let norm1 = parse_norm_meta(&mut c)?;
                let hidden = c.u32()?;
                if hidden != dims.hidden {
                    return Err(LoadError::LayerHiddenMismatch {
                        layer_idx,
                        got: hidden,
                        want: dims.hidden,
                    });
                }
                let num_q_heads = c.u32()?;
                let num_kv_heads = c.u32()?;
                let head_dim = c.u32()?;
                let attn_scales = AttentionScales {
                    q: Scale::from_num(c.i32()?)?,
                    k: Scale::from_num(c.i32()?)?,
                    v: Scale::from_num(c.i32()?)?,
                    score: Scale::from_num(c.i32()?)?,
                    attn_out: Scale::from_num(c.i32()?)?,
                    o: Scale::from_num(c.i32()?)?,
                };
                let qk_norm_eps_q = c.i64()?;
                let qk_norm_post_scale = Scale::from_num(c.i32()?)?;
                let post_attn_norm = parse_norm_meta(&mut c)?;
                let norm2 = parse_norm_meta(&mut c)?;
                let intermediate = c.u32()?;
                let ffn_scales = FfnScales {
                    gate: Scale::from_num(c.i32()?)?,
                    up: Scale::from_num(c.i32()?)?,
                    mid: Scale::from_num(c.i32()?)?,
                    down: Scale::from_num(c.i32()?)?,
                };
                let post_ffn_norm = parse_norm_meta(&mut c)?;
                let sliding_window = c.u32()?;
                let has_inp_gate = c.u8()? != 0;
                let has_layer_output_scale = c.u8()? != 0;
                layer_metas.push(LayerMeta::Gemma {
                    norm1,
                    hidden,
                    num_q_heads,
                    num_kv_heads,
                    head_dim,
                    attn_scales,
                    qk_norm_eps_q,
                    qk_norm_post_scale,
                    post_attn_norm,
                    norm2,
                    intermediate,
                    ffn_scales,
                    post_ffn_norm,
                    sliding_window,
                    has_inp_gate,
                    has_layer_output_scale,
                });
            }
            _ => return Err(LoadError::UnknownLayerTag(tag)),
        }
    }

    let has_final_norm = c.u8()?;
    let final_norm = if has_final_norm == 1 {
        Some(parse_norm_meta(&mut c)?)
    } else {
        None
    };

    let rope_seq_len = c.u32()?;
    let rope_half_head_dim = c.u32()?;
    let ffn_activation_kind = activation_kind_from_tag(c.u8()?);
    let sigmoid_lut_kind = activation_kind_from_tag(c.u8()?);

    if !c.at_end() {
        return Err(LoadError::ManifestTrailing);
    }

    Ok(ParsedManifest {
        arch_tag,
        feature_flags,
        dims,
        layer_metas,
        final_norm,
        rope_seq_len,
        rope_half_head_dim,
        ffn_activation_kind,
        sigmoid_lut_kind,
    })
}

fn parse_norm_meta(c: &mut Cursor) -> Result<NormMeta, LoadError> {
    let tag = c.u8()?;
    let eps_q = c.i64()?;
    let post_scale = Scale::from_num(c.i32()?)?;
    match tag {
        0 => Ok(NormMeta::Rms { eps_q, post_scale }),
        1 => Ok(NormMeta::LayerNorm { eps_q, post_scale }),
        _ => Err(LoadError::UnknownNormTag(tag)),
    }
}

// =========================================================================
// Weight stream parse.
// =========================================================================

struct WeightCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> WeightCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take_i8(&mut self, n: usize) -> Result<Vec<i8>, LoadError> {
        if self.pos + n > self.bytes.len() {
            return Err(LoadError::WeightsEof(self.pos as u64));
        }
        let out: Vec<i8> = self.bytes[self.pos..self.pos + n]
            .iter()
            .map(|&b| b as i8)
            .collect();
        self.pos += n;
        Ok(out)
    }
    fn take_bytes(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        if self.pos + n > self.bytes.len() {
            return Err(LoadError::WeightsEof(self.pos as u64));
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
}

fn parse_weights(parsed: ParsedManifest, bytes: &[u8]) -> Result<Model, LoadError> {
    let mut c = WeightCursor::new(bytes);
    let dims = parsed.dims;
    let arch_tag = parsed.arch_tag;
    let feature_flags = parsed.feature_flags;
    let hu = dims.hidden as usize;

    // 1. Embed: (vocab, hidden) i8, row-major.
    let embed = c.take_i8((dims.vocab as usize) * hu)?;

    // 2. Per layer.
    let mut layers: Vec<LayerWeights> = Vec::with_capacity(parsed.layer_metas.len());
    for meta in parsed.layer_metas.into_iter() {
        layers.push(parse_one_layer(&mut c, meta, hu)?);
    }

    // 3. Final norm bytes (if present).
    let final_norm = if let Some(meta) = parsed.final_norm {
        Some(materialize_norm(&mut c, &meta, hu)?)
    } else {
        None
    };

    // 4. ffn_activation LUT (256 i8 bytes).
    let ffn_lut_bytes = c.take_bytes(256)?;
    let ffn_activation = ActivationLut::from_bytes(parsed.ffn_activation_kind, ffn_lut_bytes)?;

    // 5. sigmoid LUT (256 i8 bytes).
    let sig_bytes = c.take_bytes(256)?;
    let sigmoid_lut = ActivationLut::from_bytes(parsed.sigmoid_lut_kind, sig_bytes)?;

    // 6. softmax LUT (256 LE i32 = 1024 bytes).
    let exp_bytes = c.take_bytes(256 * 4)?;
    let softmax_lut = ExpLut::from_bytes(exp_bytes)?;

    // 7. RoPE tables: u32 seq_len, u32 half_head_dim, i16 cos[], i16 sin[].
    let rope_header = c.take_bytes(8)?;
    let r_seq_len =
        u32::from_le_bytes([rope_header[0], rope_header[1], rope_header[2], rope_header[3]]);
    let r_half =
        u32::from_le_bytes([rope_header[4], rope_header[5], rope_header[6], rope_header[7]]);
    if r_seq_len != parsed.rope_seq_len || r_half != parsed.rope_half_head_dim {
        return Err(LoadError::Rope(RopeError::ShapeMismatch));
    }
    let rope_n = (r_seq_len as usize) * (r_half as usize);
    let cos_bytes = c.take_bytes(rope_n * 2)?;
    let sin_bytes = c.take_bytes(rope_n * 2)?;
    let cos: Vec<i16> = cos_bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    let sin: Vec<i16> = sin_bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    let rope_tables = RopeTables {
        seq_len: r_seq_len,
        half_head_dim: r_half,
        cos,
        sin,
    };
    rope_tables.validate()?;

    if c.pos != c.bytes.len() {
        return Err(LoadError::WeightsTrailing);
    }

    Ok(Model {
        dims,
        arch_tag,
        feature_flags,
        embed,
        layers,
        final_norm,
        rope_tables,
        softmax_lut,
        sigmoid_lut,
        ffn_activation,
    })
}

fn parse_one_layer(
    c: &mut WeightCursor,
    meta: LayerMeta,
    hu: usize,
) -> Result<LayerWeights, LoadError> {
    match meta {
        LayerMeta::Attention {
            norm1,
            hidden,
            num_q_heads,
            num_kv_heads,
            head_dim,
            attn_scales,
            norm2,
            intermediate,
            ffn_scales,
        } => {
            let n1 = materialize_norm(c, &norm1, hu)?;
            let q_dim = (num_q_heads * head_dim) as usize;
            let kv_dim = (num_kv_heads * head_dim) as usize;
            let w_q = c.take_i8(hu * q_dim)?;
            let w_k = c.take_i8(hu * kv_dim)?;
            let w_v = c.take_i8(hu * kv_dim)?;
            let w_o = c.take_i8(q_dim * hu)?;
            let n2 = materialize_norm(c, &norm2, hu)?;
            let iu = intermediate as usize;
            let w_gate = c.take_i8(hu * iu)?;
            let w_up = c.take_i8(hu * iu)?;
            let w_down = c.take_i8(iu * hu)?;
            Ok(LayerWeights::Attention {
                norm1: n1,
                attn: AttentionWeights {
                    hidden,
                    num_q_heads,
                    num_kv_heads,
                    head_dim,
                    w_q,
                    w_k,
                    w_v,
                    w_o,
                },
                attn_scales,
                norm2: n2,
                ffn: FfnWeights {
                    hidden,
                    intermediate,
                    w_gate,
                    w_up,
                    w_down,
                },
                ffn_scales,
            })
        }
        LayerMeta::DeltaNet {
            norm1,
            hidden,
            num_qk_heads,
            num_v_heads,
            head_dim_qk,
            head_dim_v,
            dnet_scales,
            norm2,
            intermediate,
            ffn_scales,
        } => {
            let n1 = materialize_norm(c, &norm1, hu)?;
            let qk_dim = (num_qk_heads * head_dim_qk) as usize;
            let v_dim = (num_v_heads * head_dim_v) as usize;
            let w_q = c.take_i8(hu * qk_dim)?;
            let w_k = c.take_i8(hu * qk_dim)?;
            let w_v = c.take_i8(hu * v_dim)?;
            let w_alpha = c.take_i8(hu * num_qk_heads as usize)?;
            let w_beta = c.take_i8(hu * num_qk_heads as usize)?;
            let w_o = c.take_i8(v_dim * hu)?;
            let n2 = materialize_norm(c, &norm2, hu)?;
            let iu = intermediate as usize;
            let w_gate = c.take_i8(hu * iu)?;
            let w_up = c.take_i8(hu * iu)?;
            let w_down = c.take_i8(iu * hu)?;
            Ok(LayerWeights::DeltaNet {
                norm1: n1,
                dnet: DeltaNetWeights {
                    hidden,
                    num_qk_heads,
                    num_v_heads,
                    head_dim_qk,
                    head_dim_v,
                    w_q,
                    w_k,
                    w_v,
                    w_alpha,
                    w_beta,
                    w_o,
                },
                dnet_scales,
                norm2: n2,
                ffn: FfnWeights {
                    hidden,
                    intermediate,
                    w_gate,
                    w_up,
                    w_down,
                },
                ffn_scales,
            })
        }
        LayerMeta::Gemma {
            norm1,
            hidden,
            num_q_heads,
            num_kv_heads,
            head_dim,
            attn_scales,
            qk_norm_eps_q,
            qk_norm_post_scale,
            post_attn_norm,
            norm2,
            intermediate,
            ffn_scales,
            post_ffn_norm,
            sliding_window,
            has_inp_gate,
            has_layer_output_scale,
        } => {
            // Matches `comm_w.rs::append_layer_weights` for the Gemma
            // arm: norm1 → attn(q,k,v,o) → qk_norm gammas →
            // post_attn_norm → norm2 → ffn(gate,up,down) →
            // post_ffn_norm → [inp_gate] → [layer_output_scale].
            let n1 = materialize_norm(c, &norm1, hu)?;
            let q_dim = (num_q_heads * head_dim) as usize;
            let kv_dim = (num_kv_heads * head_dim) as usize;
            let w_q = c.take_i8(hu * q_dim)?;
            let w_k = c.take_i8(hu * kv_dim)?;
            let w_v = c.take_i8(hu * kv_dim)?;
            let w_o = c.take_i8(q_dim * hu)?;
            let hd = head_dim as usize;
            let q_norm_gamma = c.take_i8(hd)?;
            let k_norm_gamma = c.take_i8(hd)?;
            let post_attn = materialize_norm(c, &post_attn_norm, hu)?;
            let n2 = materialize_norm(c, &norm2, hu)?;
            let iu = intermediate as usize;
            let w_gate = c.take_i8(hu * iu)?;
            let w_up = c.take_i8(hu * iu)?;
            let w_down = c.take_i8(iu * hu)?;
            let post_ffn = materialize_norm(c, &post_ffn_norm, hu)?;
            let inp_gate = if has_inp_gate {
                Some(c.take_i8(hu)?)
            } else {
                None
            };
            let layer_output_scale = if has_layer_output_scale {
                Some(c.take_i8(hu)?)
            } else {
                None
            };
            let sw_opt = if sliding_window == 0 {
                None
            } else {
                Some(sliding_window)
            };
            Ok(LayerWeights::Gemma {
                norm1: n1,
                attn: AttentionWeights {
                    hidden,
                    num_q_heads,
                    num_kv_heads,
                    head_dim,
                    w_q,
                    w_k,
                    w_v,
                    w_o,
                },
                attn_scales,
                q_norm_gamma,
                k_norm_gamma,
                qk_norm_eps_q,
                qk_norm_post_scale,
                post_attn_norm: post_attn,
                norm2: n2,
                ffn: FfnWeights {
                    hidden,
                    intermediate,
                    w_gate,
                    w_up,
                    w_down,
                },
                ffn_scales,
                post_ffn_norm: post_ffn,
                sliding_window: sw_opt,
                inp_gate,
                layer_output_scale,
            })
        }
    }
}

fn materialize_norm(
    c: &mut WeightCursor,
    meta: &NormMeta,
    hu: usize,
) -> Result<NormSpec, LoadError> {
    match meta {
        NormMeta::Rms { eps_q, post_scale } => {
            let gamma = c.take_i8(hu)?;
            Ok(NormSpec::RmsNorm {
                gamma,
                eps_q: *eps_q,
                post_scale: *post_scale,
            })
        }
        NormMeta::LayerNorm { eps_q, post_scale } => {
            let gamma = c.take_i8(hu)?;
            let beta = c.take_i8(hu)?;
            Ok(NormSpec::LayerNorm {
                gamma,
                beta,
                eps_q: *eps_q,
                post_scale: *post_scale,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::SCALE_DENOM_LOG2;
    use crate::rmsnorm::DEFAULT_EPS_Q;

    fn lcg_bytes(len: usize, seed: u64) -> Vec<i8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.wrapping_shr(56) as i8
            })
            .collect()
    }

    fn small() -> Scale {
        Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap()
    }

    fn build_attn_model() -> Model {
        let hidden = 4u32;
        let hu = hidden as usize;
        Model {
            dims: ModelDims {
                vocab: 8,
                hidden,
                seq_len: 4,
                activation_tile: 2,
            },
            arch_tag: [0u8; 16],
            feature_flags: 0,
            embed: lcg_bytes(8 * hu, 0xa1a1),
            layers: vec![LayerWeights::Attention {
                norm1: NormSpec::RmsNorm {
                    gamma: lcg_bytes(hu, 0xb2b2),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                attn: AttentionWeights {
                    hidden,
                    num_q_heads: 1,
                    num_kv_heads: 1,
                    head_dim: 2,
                    w_q: lcg_bytes(hu * 2, 0xc3c3),
                    w_k: lcg_bytes(hu * 2, 0xd4d4),
                    w_v: lcg_bytes(hu * 2, 0xe5e5),
                    w_o: lcg_bytes(2 * hu, 0xf6f6),
                },
                attn_scales: AttentionScales {
                    q: small(),
                    k: small(),
                    v: small(),
                    score: small(),
                    attn_out: small(),
                    o: small(),
                },
                norm2: NormSpec::RmsNorm {
                    gamma: lcg_bytes(hu, 0x0707),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                ffn: FfnWeights {
                    hidden,
                    intermediate: hidden * 2,
                    w_gate: lcg_bytes(hu * (hu * 2), 0x1818),
                    w_up: lcg_bytes(hu * (hu * 2), 0x2929),
                    w_down: lcg_bytes((hu * 2) * hu, 0x3a3a),
                },
                ffn_scales: FfnScales {
                    gate: small(),
                    up: small(),
                    mid: small(),
                    down: small(),
                },
            }],
            final_norm: Some(NormSpec::RmsNorm {
                gamma: lcg_bytes(hu, 0x4b4b),
                eps_q: DEFAULT_EPS_Q,
                post_scale: small(),
            }),
            rope_tables: RopeTables::identity(4, 1),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        }
    }

    fn build_deltanet_model() -> Model {
        let hidden = 4u32;
        let hu = hidden as usize;
        let num_qk = 1u32;
        let num_v = 2u32;
        let hd = 2u32;
        Model {
            dims: ModelDims {
                vocab: 8,
                hidden,
                seq_len: 4,
                activation_tile: 2,
            },
            arch_tag: [0u8; 16],
            feature_flags: 0,
            embed: lcg_bytes(8 * hu, 0xa1a1),
            layers: vec![LayerWeights::DeltaNet {
                norm1: NormSpec::LayerNorm {
                    gamma: lcg_bytes(hu, 0xb2b2),
                    beta: lcg_bytes(hu, 0xc3c3),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                dnet: DeltaNetWeights {
                    hidden,
                    num_qk_heads: num_qk,
                    num_v_heads: num_v,
                    head_dim_qk: hd,
                    head_dim_v: hd,
                    w_q: lcg_bytes(hu * (num_qk * hd) as usize, 0xd4d4),
                    w_k: lcg_bytes(hu * (num_qk * hd) as usize, 0xe5e5),
                    w_v: lcg_bytes(hu * (num_v * hd) as usize, 0xf6f6),
                    w_alpha: lcg_bytes(hu * num_qk as usize, 0x0707),
                    w_beta: lcg_bytes(hu * num_qk as usize, 0x1818),
                    w_o: lcg_bytes((num_v * hd) as usize * hu, 0x2929),
                },
                dnet_scales: DeltaNetScales {
                    q: small(),
                    k: small(),
                    v: small(),
                    alpha_logit: small(),
                    beta_logit: small(),
                    u: small(),
                    decay: small(),
                    update: small(),
                    o: small(),
                    proj: small(),
                },
                norm2: NormSpec::RmsNorm {
                    gamma: lcg_bytes(hu, 0x3a3a),
                    eps_q: DEFAULT_EPS_Q,
                    post_scale: small(),
                },
                ffn: FfnWeights {
                    hidden,
                    intermediate: hidden * 2,
                    w_gate: lcg_bytes(hu * (hu * 2), 0x4b4b),
                    w_up: lcg_bytes(hu * (hu * 2), 0x5c5c),
                    w_down: lcg_bytes((hu * 2) * hu, 0x6d6d),
                },
                ffn_scales: FfnScales {
                    gate: small(),
                    up: small(),
                    mid: small(),
                    down: small(),
                },
            }],
            final_norm: None,
            rope_tables: RopeTables::identity(4, 1),
            softmax_lut: ExpLut::uniform_test(),
            sigmoid_lut: ActivationLut::identity(),
            ffn_activation: ActivationLut::identity(),
        }
    }

    fn tmpdir(suffix: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("ai_pow_vi_io_test_{pid}_{nanos}_{suffix}"));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn round_trip_attention_model() {
        let m = build_attn_model();
        let dir = tmpdir("attn");
        save_model(&m, &dir).unwrap();
        let comm = compute_comm_w(&m);
        let loaded = load_model(&dir, &comm).unwrap();
        assert_eq!(m.dims, loaded.dims);
        assert_eq!(m.embed, loaded.embed);
        assert_eq!(m.layers, loaded.layers);
        assert_eq!(m.final_norm, loaded.final_norm);
        assert_eq!(m.rope_tables, loaded.rope_tables);
        assert_eq!(m.softmax_lut, loaded.softmax_lut);
        assert_eq!(m.sigmoid_lut, loaded.sigmoid_lut);
        assert_eq!(m.ffn_activation, loaded.ffn_activation);
        assert_eq!(compute_comm_w(&loaded), comm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip_deltanet_model() {
        let m = build_deltanet_model();
        let dir = tmpdir("dnet");
        save_model(&m, &dir).unwrap();
        let comm = compute_comm_w(&m);
        let loaded = load_model(&dir, &comm).unwrap();
        assert_eq!(m.layers, loaded.layers);
        assert_eq!(m.final_norm, loaded.final_norm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flipping_one_weight_byte_trips_comm_w() {
        let m = build_attn_model();
        let dir = tmpdir("flip_w");
        save_model(&m, &dir).unwrap();
        let comm = compute_comm_w(&m);
        // Flip a byte in weights.bin.
        let weights_path = dir.join(WEIGHTS_FILENAME);
        let mut bytes = fs::read(&weights_path).unwrap();
        bytes[7] ^= 1;
        fs::write(&weights_path, &bytes).unwrap();
        let result = load_model(&dir, &comm);
        assert!(matches!(result, Err(LoadError::CommWMismatch)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flipping_one_manifest_byte_trips_comm_w() {
        let m = build_attn_model();
        let dir = tmpdir("flip_m");
        save_model(&m, &dir).unwrap();
        let comm = compute_comm_w(&m);
        let manifest_path = dir.join(MANIFEST_FILENAME);
        let mut bytes = fs::read(&manifest_path).unwrap();
        // Skip the magic + version (12 bytes); flip a byte in dims.
        bytes[15] ^= 1;
        fs::write(&manifest_path, &bytes).unwrap();
        // Either the manifest fails to parse or comm_W mismatches —
        // both are acceptable.
        assert!(load_model(&dir, &comm).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_expected_comm_w_rejected() {
        let m = build_attn_model();
        let dir = tmpdir("wrong_expected");
        save_model(&m, &dir).unwrap();
        let mut wrong = compute_comm_w(&m);
        wrong[0] ^= 1;
        let result = load_model(&dir, &wrong);
        assert!(matches!(result, Err(LoadError::CommWMismatch)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_manifest_returns_io_error() {
        let dir = tmpdir("missing_manifest");
        fs::create_dir_all(&dir).unwrap();
        // Don't save anything; just try to load.
        let comm = [0u8; 32];
        assert!(matches!(load_model(&dir, &comm), Err(LoadError::Io(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_weights_returns_eof() {
        let m = build_attn_model();
        let dir = tmpdir("truncated_w");
        save_model(&m, &dir).unwrap();
        // Truncate weights.bin.
        let path = dir.join(WEIGHTS_FILENAME);
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        let comm = compute_comm_w(&m);
        let r = load_model(&dir, &comm);
        // Either WeightsEof or WeightsTrailing depending on cut point;
        // accept any non-success.
        assert!(r.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_magic_rejected() {
        let dir = tmpdir("bad_magic");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MANIFEST_FILENAME), b"NOTAIPOW").unwrap();
        fs::write(dir.join(WEIGHTS_FILENAME), b"").unwrap();
        let r = load_model(&dir, &[0u8; 32]);
        assert!(matches!(r, Err(LoadError::BadMagic)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_version_rejected() {
        let dir = tmpdir("bad_version");
        fs::create_dir_all(&dir).unwrap();
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&999u32.to_le_bytes());
        fs::write(dir.join(MANIFEST_FILENAME), &buf).unwrap();
        fs::write(dir.join(WEIGHTS_FILENAME), b"").unwrap();
        let r = load_model(&dir, &[0u8; 32]);
        assert!(matches!(r, Err(LoadError::UnsupportedVersion(999))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn comm_w_hex_file_written_correctly() {
        let m = build_attn_model();
        let dir = tmpdir("hex_check");
        save_model(&m, &dir).unwrap();
        let hex = fs::read_to_string(dir.join(COMM_W_HEX_FILENAME)).unwrap();
        assert_eq!(hex.len(), 64);
        let want = compute_comm_w(&m);
        assert_eq!(hex, hex_encode(&want));
        let _ = fs::remove_dir_all(&dir);
    }
}
