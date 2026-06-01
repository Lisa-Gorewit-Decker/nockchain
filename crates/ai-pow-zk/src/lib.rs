//! Plonky3 SNARK circuit for the `ai-pow` tiling matmul puzzle.
//!
//! Mirrors Pearl's `zk-pow` role: where Pearl uses Plonky2 to compress
//! its multi-MB `PlainProof` into a ~60 KB `ZKProof` (see
//! `Pearl zk-pow api/prove.rs::zk_prove_plain_proof`), this crate
//! uses Plonky3 over Goldilocks + Tip5 + FRI to do the equivalent for
//! the `ai-pow` plain proof.
//!
//! ## Public API
//!
//! - [`recursion::prove_canonical_ai_pow_certificate`] — the canonical
//!   production prover API for Nockchain's AI-PoW certificate. It
//!   proves the Layer-0 composite STARK, recursively verifies that
//!   proof in an L1 circuit, and returns the recursive certificate.
//!   This is the only proof object intended for Nockchain consensus,
//!   block persistence, or wire transmission.
//! - [`composite_proof::composite_prove_pinned_logup`] /
//!   [`composite_proof::composite_verify_pinned_logup`] — Layer-0
//!   composite STARK primitives. These are intermediate inputs to the
//!   recursive certificate and are not canonical production
//!   certificates by themselves.
//! - [`composite_proof::composite_prove`] / [`composite_proof::composite_verify`]
//!   — dev-only unpinned prove + verify pair, wrapping `p3-uni-stark`.
//! - [`composite_full_air::CompositeFullAir`] — the top-level AIR over
//!   `TOTAL_TRACE_WIDTH` columns. 10 chips share every row, gated by
//!   selector bits packed into `CONTROL_PREP`.
//! - [`composite_full_air_with_lookups::CompositeFullAirWithLookups`] —
//!   same AIR with 7 LogUp buses reified via `p3-batch-stark`. Used
//!   when full cross-chip soundness is required.
//! - [`composite_trace::CompositeTrace`] — trace generator with
//!   `place_*` helpers for each instruction type (matmul step, BLAKE3
//!   hash, jackpot rotation).
//! - [`composite_public::CompositePublicInputs`] — typed 20-element PI
//!   vector (4 i32 cumsum + 16 u32 jackpot) bound by the AIR on the
//!   trace's last row.
//! - [`params::ZkParams`] — circuit parameter shape (rows / cols of the
//!   matmul puzzle).
//! - [`circuit::AiPowStarkConfig`] / [`circuit::CircuitConfig`] — the
//!   STARK config factory (Goldilocks + Tip5 + FRI parameters per
//!   profile).
//!
//! ## Architectural note
//!
//! `ai-pow-zk` is intentionally **standalone** — it does **not** depend
//! on `ai-pow`. The proving crate (`ai-pow`) is the consumer; making
//! `ai-pow-zk` depend back on it would introduce a circular workspace
//! dep. The caller in `ai-pow` constructs [`ZkParams`] and the trace +
//! PIs from its own types at the call site.
//!
//! ## Status
//!
//! M10.1c is the Layer-0 composite pipeline. The production
//! Nockchain certificate is the recursive L1 certificate produced by
//! the `recursion` module, not the raw Layer-0 proof. Earlier M9.1 /
//! M10.1b prototypes were retired once M10.1c had full LogUp + PI
//! binding + bench data.
//! See `2026-05-14_ENGINEERING_REPORT.md` for the architectural review and bench
//! numbers; `2026-05-14_M10_1C_PROGRESS.md` for the phase-by-phase history.
//!
//! ## What's not yet bound
//!
//! - **`h_a` / `h_b` matrix bindings** (task #52). The witness's
//!   `a_rows` / `b_cols` aren't yet tied to chain-pinned chunk-Merkle
//!   roots. Multi-week deferred work.
//! - **Structured noun serialization for the recursive certificate.**
//!   The current Rust measurement path serializes the L1 certificate as
//!   a Rust proof object; consensus still needs the proof-shaped noun
//!   format described in `docs/ai-pow-integration/`.
//! - **Hoon/kernel verifier wiring.** The Rust recursive production
//!   certificate now binds the Layer-0 AI-PoW public-input vector as outer
//!   STARK public values and verifies it with
//!   [`recursion::verify_production_certificate`]. Consensus still needs the
//!   Hoon jet/wiring that decodes the structured noun, reconstructs the
//!   verifier-derived statement from block data, and calls that Rust verifier.
//!   Until that hook is wired, the Hoon/kernel path remains fail-closed.

pub mod bench_suite;
pub mod blake3_tree;
pub mod canonical;
pub mod chips;
pub mod circuit;
pub mod composite_full_air;
pub mod composite_full_air_with_lookups;
pub mod composite_layout;
pub mod composite_lookup_proof;
pub mod composite_lookups;
pub mod composite_preprocess;
pub mod composite_proof;
pub mod composite_public;
pub mod composite_trace;
pub mod noise_ref;
pub mod params;

/// §recursion — integration of the composite proof with the vendored
/// `Plonky3-recursion` substrate. Opt-in (`--features recursion`); the
/// default build pulls no recursion crates.
#[cfg(feature = "recursion")]
pub mod recursion;

pub use p3_goldilocks::Goldilocks as Val;

pub use crate::circuit::{AiPowStarkConfig, CircuitConfig};
pub use crate::composite_full_air::{extract_program, CompositeFullAir, CompositeFullAirPinned};
pub use crate::composite_full_air_with_lookups::{
    CompositeFullAirWithLookups, CompositeFullAirWithLookupsPinned,
};
#[cfg(any(test, feature = "dev-unsafe"))]
pub use crate::composite_proof::{
    composite_prove as dev_unpinned_prove, composite_prove_pinned as dev_pinned_no_logup_prove,
    composite_setup as dev_pinned_no_logup_setup, composite_verify as dev_unpinned_verify,
    composite_verify_pinned as dev_pinned_no_logup_verify,
    composite_verify_pow as dev_unpinned_verify_pow,
    composite_verify_pow_pinned as dev_pinned_no_logup_verify_pow,
};
pub use crate::composite_proof::{
    hash_jackpot_le_bytes, CompositeVerificationError, PowVerifyError, ProgramShapeError,
    StarkVerificationError,
};
pub use crate::composite_public::CompositePublicInputs;
pub use crate::composite_trace::CompositeTrace;
pub use crate::params::ZkParams;

/// Concrete pinned+LogUp Layer-0 proof type.
///
/// This is an intermediate proof consumed by the recursive certificate
/// prover. It is not the canonical Nockchain production certificate and
/// must not be persisted in blocks or transmitted as the AI-PoW proof
/// artifact.
pub type AiPowBatchProof = p3_batch_stark::BatchProof<AiPowStarkConfig>;

/// Concrete preprocessed program matrix type used by the pinned AI PoW circuit.
pub type AiPowProgram = p3_matrix::dense::RowMajorMatrix<p3_uni_stark::Val<AiPowStarkConfig>>;
