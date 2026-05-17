//! Plonky3 SNARK circuit for the `ai-pow` tiling matmul puzzle.
//!
//! Mirrors Pearl's `zk-pow` role: where Pearl uses Plonky2 to compress
//! its multi-MB `PlainProof` into a ~60 KB `ZKProof` (see
//! `pearl/zk-pow/src/api/prove.rs::zk_prove_plain_proof`), this crate
//! uses Plonky3 over Goldilocks + Tip5 + FRI to do the equivalent for
//! the `ai-pow` plain proof.
//!
//! ## Public API
//!
//! - [`composite_proof::composite_prove`] / [`composite_proof::composite_verify`]
//!   — the canonical prove + verify pair, wrapping `p3-uni-stark`.
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
//! M10.1c is the canonical pipeline. Earlier M9.1 / M10.1b prototypes
//! were retired once M10.1c had full LogUp + PI binding + bench data.
//! See `ENGINEERING_REPORT.md` for the architectural review and bench
//! numbers; `M10_1C_PROGRESS.md` for the phase-by-phase history.
//!
//! ## What's not yet bound
//!
//! - **`h_a` / `h_b` matrix bindings** (task #52). The witness's
//!   `a_rows` / `b_cols` aren't yet tied to chain-pinned chunk-Merkle
//!   roots. Multi-week deferred work.
//! - **Recursion compression** (M12). Plonky3 doesn't ship a
//!   compressor yet; deferred per the original M10.1c design.

pub mod bench_suite;
pub mod blake3_tree;
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

pub use p3_goldilocks::Goldilocks as Val;

pub use crate::circuit::{AiPowStarkConfig, CircuitConfig};
pub use crate::composite_full_air::{
    extract_program, CompositeFullAir, CompositeFullAirPinned,
};
pub use crate::composite_full_air_with_lookups::{
    CompositeFullAirWithLookups, CompositeFullAirWithLookupsPinned,
};
pub use crate::composite_proof::{
    composite_prove, composite_prove_pinned, composite_prove_pinned_logup,
    composite_prove_pinned_logup_sx, composite_setup, composite_verify,
    composite_verify_pinned, composite_verify_pinned_logup, composite_verify_pinned_logup_sx,
    composite_verify_pow, composite_verify_pow_pinned, composite_verify_pow_pinned_logup,
    composite_verify_pow_pinned_logup_sx, hash_jackpot_le_bytes, CompositeVerificationError,
    PowVerifyError,
};
pub use crate::composite_public::CompositePublicInputs;
pub use crate::composite_trace::CompositeTrace;
pub use crate::params::ZkParams;
