//! Vendored fork of Plonky3 `p3_blake3_air` with Pearl-compatible
//! keyed-mode support (M10.1b).
//!
//! Why a vendored fork?
//!
//! Upstream `Blake3Air` constrains `local.flags` in
//! [`air::Blake3KeyedAir::eval`] (it's used as `initial_row_3[3]`) but
//! the upstream trace generator never populates `row.flags` — flags
//! default to zero. That hard-codes the chip to BLAKE3-compression-
//! with-flags-zero, which is **not** BLAKE3 keyed-mode and so does
//! **not** match what ai-pow / Pearl compute. Diverging the hash would
//! break Pearl ↔ Nockchain merge-mining (the matmul work must produce
//! the same leaves under both protocols' difficulty checks).
//!
//! Upstream also bakes `counter = row_index` and `block_len = num_rows`
//! into the public generator, making it impossible to specify
//! per-call parameters. We need `counter = 0, block_len = 64,
//! flags = 0x1B` for a single-block keyed root hash.
//!
//! The patches in this module:
//!
//!   * `generation::generate_trace_rows_for_perm` now takes
//!     `(counter, block_len, flags)` and writes all three into the
//!     trace row.
//!   * The compression's `state[3]` initialization uses `flags`
//!     instead of `0` so the constraint and the trace agree.
//!   * A new `generate_trace_for_calls` entry point accepts a
//!     `&[Blake3HashCall]` so callers can specify all parameters per
//!     row. Used by M10.1b's found-leaf binding proof.
//!   * The AIR type is renamed [`Blake3KeyedAir`] to make the fork
//!     boundary obvious (no risk of accidentally calling upstream's
//!     `Blake3Air`).
//!
//! Provenance: vendored from `Plonky3/Plonky3 @ af65376`
//! ([upstream `blake3-air/src/`](https://github.com/Plonky3/Plonky3/tree/main/blake3-air/src)).
//! Dual-licensed MIT / Apache-2.0; copies of those licenses live in the
//! Plonky3 repo. The constraint logic in [`air`] is unchanged from
//! upstream; only generation is patched.

mod air;
mod columns;
mod constants;
mod generation;

pub use air::Blake3KeyedAir;
pub use columns::{Blake3Cols, NUM_BLAKE3_COLS};
pub use generation::{generate_trace_for_calls, reference_compression_output, Blake3HashCall};
