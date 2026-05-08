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
pub mod determinism;
pub mod layout;
pub mod quant;
pub mod rmsnorm;

pub use crate::activation_lut::{ActivationKind, ActivationLut};
pub use crate::determinism::{BitExactOp, ARCH_TAG};
pub use crate::layout::{BlockKind, ModelFamily, ModelLayout, NormType};
pub use crate::quant::{rescale_and_requantize, Scale};
pub use crate::rmsnorm::rmsnorm;
