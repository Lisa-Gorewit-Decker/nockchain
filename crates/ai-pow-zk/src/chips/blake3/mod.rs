//! BLAKE3 chip for the M10.1c composite AIR.
//!
//! Port of `pearl/zk-pow/src/circuit/chip/blake3/` — Pearl's
//! one-round-per-row BLAKE3 chip. Compared to M10.1b's vendored
//! `crate::blake3_chip` (one full hash per row, ~10k cols), this
//! chip spreads one BLAKE3 compression across 8 trace rows × ~1k
//! cols, which is the layout the composite AIR is sized for
//! (`composite_layout::BLAKE3_ROUND_LEN = 1056`).
//!
//! ## Module layout (mirrors Pearl's `chip/blake3/`)
//!
//! | Pearl file | Our module | Phase | Status |
//! |---|---|---|---|
//! | `blake3_compress.rs` (scalar reference) | [`compress`] | 7 | landed |
//! | `blake3_layout.rs` (per-round columns) | [`layout`] | 7 | landed |
//! | `logic.rs` (per-row instruction types) | [`logic`] | 7 | landed |
//! | `trace.rs` (trace-side generator)       | [`trace`] | 8 | pending |
//! | `constraints.rs` (AIR-side eval)        | [`constraints`] | 8 | pending |
//! | `program.rs` (instruction compilation)  | [`program`] | 8 | pending |
//! | `blake3_air.rs` (chip struct + setup)   | [`chip`] | 8 | pending |
//!
//! ## Pearl byte-equivalence (validated)
//!
//! [`compress::blake3_compress`] produces the same output as
//! [`crate::blake3_chip::reference_compression_output`] (our
//! M10.1b vendored chip's scalar reference) AND as
//! `blake3::Hasher::new_keyed(...).update(...).finalize()` for the
//! single-block keyed-root case. See `tests/blake3_compress_kat.rs`
//! and the in-module tests in [`compress`].

pub mod compress;
pub mod layout;
pub mod logic;

// Phase 8 modules (re-exports staged here for forward-compat):
//
// pub mod constraints;
// pub mod program;
// pub mod trace;
//
// pub use chip::Blake3Chip;
