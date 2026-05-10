//! Verifiable-inference puzzle: deterministic INT8 transformer-layer reference.
//!
//! Phase 2 of the AI-PoW plan (mainnet hard fork to verifiable LLM inference
//! as the proof-of-work puzzle). This crate's only job is to be **bit-exact**
//! across architectures and against a PyTorch oracle on real Gemma 4 31B and
//! Qwen 3.6 27B INT8 weights. Every numerical op obeys [`quant`] +
//! [`determinism`].
//!
//! Reuses primitives from `ai-pow`: BLAKE3 transcript, tile-Merkle (with
//! sentinel padding), INT8 tile dot product. Adds:
//! - integer RMSNorm / LayerNorm with Newton-Raphson reciprocal-sqrt,
//! - integer softmax with a base-2 exponent LUT,
//! - INT16 fixed-point RoPE,
//! - 256-entry INT8→INT8 activation lookup tables (committed alongside
//!   weights),
//! - per-tensor / per-channel symmetric INT8 quantization with banker's
//!   rounding,
//! - canonical-order weight commitment (`comm_W`) and per-layer activation
//!   Merkle commitments,
//! - FS-derived synthetic prompt sampling.
//!
//! Subsequent phases (3, 4, 5) wrap this crate with prover/verifier APIs,
//! Hoon-side jets, and consensus integration. None of those exist yet.

pub mod activation_lut;
pub mod activations;
pub mod attention;
pub mod comm_w;
pub mod deltanet;
pub mod determinism;
pub mod ffn;
pub mod forward;
pub mod io;
pub mod layer;
pub mod layernorm;
pub mod layout;
pub mod matmul_int8;
pub mod model;
pub mod prompt;
pub mod proof;
pub mod prover;
pub mod quant;
pub mod rmsnorm;
pub mod rope;
pub mod softmax;
pub mod ssm;
pub mod verifier;

pub use crate::activation_lut::{ActivationKind, ActivationLut};
pub use crate::activations::{verify_opening, ActivationLayout, ActivationLog, ActivationOpening};
pub use crate::attention::{
    attention_forward, attention_forward_gemma, AttentionScales, AttentionWeights,
    GemmaAttentionOpts,
};
pub use crate::comm_w::{canonical_weight_bytes, compute_comm_w, WEIGHT_TILE_BYTES};
pub use crate::deltanet::{deltanet_forward, DeltaNetScales, DeltaNetWeights};
pub use crate::determinism::{BitExactOp, ARCH_TAG};
pub use crate::ffn::{elementwise_mul_i8, ffn_forward, FfnScales, FfnWeights};
pub use crate::forward::{forward_prefix, ForwardError};
pub use crate::layer::{forward_layer, LayerContext, LayerWeights, NormSpec};
pub use crate::layernorm::layernorm;
pub use crate::layout::{BlockKind, ModelFamily, ModelLayout, NormType};
pub use crate::matmul_int8::{dot_int8, matmul_int8, matmul_int8_requant, requantize_vec};
pub use crate::model::{arch_tag, ArchTag, FeatureFlags, Model, ModelDims, ModelError, Token};
pub use crate::prompt::{synth_prompt, PromptError};
pub use crate::proof::{TileOpening, ViProof};
pub use crate::prover::{mine_vi, ProverError, ProverOptions, FFN_PUZZLE_TILE};
pub use crate::quant::{rescale_and_requantize, Scale};
pub use crate::rmsnorm::rmsnorm;
pub use crate::rope::{rope_apply, RopeTables};
pub use crate::softmax::{softmax_int, ExpLut};
pub use crate::ssm::{ssm_forward, SsmError, SsmOpts};
pub use crate::verifier::{verify_vi, VerifierMode, VerifyError};
