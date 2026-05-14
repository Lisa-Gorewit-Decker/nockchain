//! Column layout for the M10.1c Pearl-style composite AIR.
//!
//! Direct port of `pearl/zk-pow/src/circuit/pearl_layout.rs` minus the
//! RAM-lookup columns we don't need (see [`crate::M10_1C_DESIGN`] for
//! the rationale — single-tile-cell MVP doesn't amortize Pearl's
//! `NOISED_PACKED` / `MAT_ID` machinery).
//!
//! Each row of the composite AIR's trace carries columns from every
//! chip side-by-side. Preprocessed control columns determine which
//! chip's constraints fire on which row. This file pins the column
//! offsets and lengths so the constraint code, trace generator, and
//! lookup configurations all agree.
//!
//! Phases that consume the layout:
//!
//! | Phase | Module |
//! |---|---|
//! | 3 | `stark_row_chip` (Pearl's `monotonic_increment.rs`) |
//! | 4 | `input_chip`, `i8u8_chip`, range chips |
//! | 5 | `control_chip` (Pearl's `control_and_matid_packed.rs`) |
//! | 6 | preprocessed-trace generation |
//! | 7-8 | `blake3` chip (one round per row) |
//! | 9 | `matmul` chip |
//! | 10 | `jackpot` chip |
//! | 11 | `composite_lookups` (logUp configuration) |
//! | 12 | `composite_full_air::eval` (top-level) |
//! | 13 | `composite_trace` (top-level) |
//! | 14 | `lib::{prove, verify}` plumbing |
//!
//! ## Per-row schedule
//!
//! The trace is the union of two instruction streams:
//!
//!   * **BLAKE3 stream** — one ROUND per row. Pearl's
//!     `ROUNDS_PER_BLAKE_INSTRUCTION = 8`; an entire BLAKE3 hash
//!     occupies 8 consecutive rows.
//!   * **Matmul + Jackpot stream** — one stripe step or one
//!     XOR-fold instruction per row. Pearl interleaves these with
//!     BLAKE3 rounds; rows are "shared" via preprocessed selectors.
//!
//! Rows where no chip is active are padding (all selectors zero).
//!
//! ## Pearl bytes ↔ Plonky3 mapping
//!
//! Pearl packs `4 × i8` per Goldilocks element via the
//! `BYTES_PER_GOLDILOCKS = 4` constant. We keep that for the
//! `NOISED_PACKED`-equivalent inline a/b storage so byte-equivalence
//! with Pearl's matrix encoding is preserved (important for the
//! cross-AIR LogUp lookup against `blake3::Hasher::new_keyed`).

#![allow(clippy::module_inception)]

/// Packing factor: 4 i8 elements per Goldilocks element.
///
/// Matches Pearl's `pearl_columns::BYTES_PER_GOLDILOCKS` so the
/// underlying matrix-byte representation is byte-equivalent to ai-pow
/// / Pearl. Required for the merge-mining compat guarantee — the
/// in-circuit hash sees the same bytes a Pearl miner hashes.
pub const BYTES_PER_GOLDILOCKS: usize = 4;

/// Bits per range-table limb. Pearl's `pearl_columns::BITS_PER_LIMB =
/// 13` because their u13 range table covers MAT_ID bounds. We skip
/// MAT_ID (no RAM lookups) but keep 13-bit limbs for the limb-style
/// decomposition of other range-checked values that match Pearl's
/// budget.
pub const BITS_PER_LIMB: usize = 13;

/// Block-commitment fixed size in bytes. Pinned by M10.1c design (see
/// `M10_1C_DESIGN.md` decision 4): 32 bytes = 8 × `u32` LE, matching
/// the Tip5 digest size we use elsewhere for Merkle commitments. The
/// in-circuit κ derivation hashes `block_commitment ‖ params_tag` (32
/// + 32 = 64 bytes = one BLAKE3 block, single compression call).
pub const BLOCK_COMMITMENT_BYTES: usize = 32;

/// Same in Goldilocks elements (4 bytes per u32, 8 u32s).
pub const BLOCK_COMMITMENT_WORDS: usize = BLOCK_COMMITMENT_BYTES / BYTES_PER_GOLDILOCKS;

/// Pearl-style tile dimensions (matches `pearl/zk-pow/src/circuit/
/// pearl_program.rs:23-25`).
pub const TILE_D: usize = 16;
pub const TILE_H: usize = 2;
pub const JACKPOT_SIZE: usize = 16;

/// Left-rotation amount for Pearl's tile-state update (`pearl_program.rs:27`).
pub const LROT_PER_TILE: u32 = 13;

/// Per-instruction round count (matches Pearl's
/// `ROUNDS_PER_BLAKE_INSTRUCTION = 8`). BLAKE3 has 7 mixing rounds; the
/// 8th row in Pearl's layout is the output-XOR finalisation step.
pub const ROUNDS_PER_BLAKE_INSTRUCTION: usize = 8;

/// Minimum STARK trace length (matches Pearl's `MIN_STARK_LEN = 1 <<
/// 13 = 8192`). Below this, FRI doesn't have enough rows for our
/// `log_blowup + log_final_poly_len` setup; padding kicks in.
pub const MIN_STARK_LEN: usize = 1 << 13;

// =====================================================================
//  Column groups
//
//  Mirrors `pearl/zk-pow/src/circuit/pearl_layout.rs:7-81`'s
//  `pearl_columns` block. We use a manual const-offset scheme rather
//  than a macro to keep dependencies tight.
//
//  Layout order (offset, width):
//   range tables    : 11
//   control flags   : 21
//   matmul / jackpot data : variable
//   blake3 round AIR: ~1000
//
//  All "PREP" columns are PREPROCESSED — committed at setup. Other
//  columns are part of the main trace, generated per-proof.
// =====================================================================

/// Range and conversion table columns. Each `*_TABLE` column lists
/// every value in the range exactly once; `*_FREQ` carries the
/// multiplicities used in the logUp argument. Pearl uses these to
/// verify all input values stay in their declared ranges.
pub const URANGE8_TABLE: usize = 0; // 0..=255
pub const URANGE8_FREQ: usize = 1;
pub const URANGE13_TABLE: usize = 2; // 0..=8191 (BITS_PER_LIMB = 13)
pub const URANGE13_FREQ: usize = 3;
pub const IRANGE7P1_TABLE: usize = 4; // -64..=64 (Pearl's signed-noise range)
pub const IRANGE7P1_FREQ: usize = 5;
pub const IRANGE8_TABLE: usize = 6; // -128..=127
pub const IRANGE8_FREQ: usize = 7;
pub const I8U8_TABLE: usize = 8; // i8 -> u8 conversion (size = 1 << 8)
pub const I8U8_AUX: usize = 9;
pub const I8U8_FREQ: usize = 10;

/// Number of range / conversion table columns.
pub const NUM_RANGE_COLS: usize = 11;

/// Preprocessed control word. Packs the selector bits + (in Pearl,
/// MAT_ID; we drop that). The composite trace generator emits a
/// little-endian unpacking constraint over this column.
pub const CONTROL_PREP: usize = NUM_RANGE_COLS;

/// Per-chip selector bits (unpacked from `CONTROL_PREP`).
/// Pearl-mirror minus the MAT_ID / NOISED_PACKED machinery.
pub const IS_RESET_CUMSUM: usize = CONTROL_PREP + 1;
pub const IS_UPDATE_CUMSUM: usize = IS_RESET_CUMSUM + 1;
pub const IS_USE_JOB_KEY: usize = IS_UPDATE_CUMSUM + 1;
pub const IS_USE_COMMITMENT_HASH: usize = IS_USE_JOB_KEY + 1;
pub const IS_HASH_A: usize = IS_USE_COMMITMENT_HASH + 1;
pub const IS_HASH_B: usize = IS_HASH_A + 1;
pub const IS_HASH_JACKPOT: usize = IS_HASH_B + 1;
pub const IS_CV_IN: usize = IS_HASH_JACKPOT + 1;
pub const IS_NEW_BLAKE: usize = IS_CV_IN + 1;
pub const IS_LAST_ROUND: usize = IS_NEW_BLAKE + 1;
pub const IS_MSG_MAT: usize = IS_LAST_ROUND + 1;
pub const IS_MSG_JACKPOT: usize = IS_MSG_MAT + 1;
pub const IS_MSG_AUX_DATA: usize = IS_MSG_JACKPOT + 1;
pub const IS_MSG_CV: usize = IS_MSG_AUX_DATA + 1;
pub const IS_LOAD: usize = IS_MSG_CV + 1;
pub const IS_XOR: usize = IS_LOAD + 1;
pub const IS_SHIFT3: usize = IS_XOR + 1;
pub const IS_STORE0: usize = IS_SHIFT3 + 1;
pub const IS_STORE1: usize = IS_STORE0 + 1;
pub const IS_STORE2: usize = IS_STORE1 + 1;
pub const IS_DUMP_CUMSUM_BUFFER: usize = IS_STORE2 + 1;

/// Number of control / selector columns (CONTROL_PREP + 21 bits).
pub const NUM_CONTROL_COLS: usize = 22;

/// First column index after control. Subsequent phases fill in chip
/// columns starting here; we leave the rest of the layout open for
/// Phase 3+ to assign.
pub const CHIP_COLS_START: usize = CONTROL_PREP + NUM_CONTROL_COLS;

// =====================================================================
//  Total trace width
//
//  Pinned in Phase 12 once every chip's column count is known. For
//  Phase 2 we expose `CHIP_COLS_START` as the cursor; downstream
//  phases extend it.
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase-2 const-pinning anchor: any reshuffling of the layout
    /// triggers this test, forcing the constraint code / trace
    /// generator / lookups to update in lockstep.
    #[test]
    fn layout_constants_pin() {
        assert_eq!(BYTES_PER_GOLDILOCKS, 4);
        assert_eq!(BITS_PER_LIMB, 13);
        assert_eq!(BLOCK_COMMITMENT_BYTES, 32);
        assert_eq!(BLOCK_COMMITMENT_WORDS, 8);
        assert_eq!(TILE_D, 16);
        assert_eq!(TILE_H, 2);
        assert_eq!(JACKPOT_SIZE, 16);
        assert_eq!(LROT_PER_TILE, 13);
        assert_eq!(ROUNDS_PER_BLAKE_INSTRUCTION, 8);
        assert_eq!(MIN_STARK_LEN, 8192);

        assert_eq!(NUM_RANGE_COLS, 11);
        assert_eq!(CONTROL_PREP, 11);
        assert_eq!(NUM_CONTROL_COLS, 22);
        assert_eq!(CHIP_COLS_START, 33);
    }

    /// Pearl-byte-compat anchor: 8 × u32 little-endian = 32 bytes
    /// (Tip5 digest size). If anyone bumps `BLOCK_COMMITMENT_*` we
    /// catch the inconsistency here.
    #[test]
    fn block_commitment_layout_matches_8_u32_le() {
        // 8 LE u32s packed into 32 bytes — the in-circuit κ derivation
        // (Phase 7) consumes these as message[0..8] of a single
        // BLAKE3 block (with params_tag as message[8..16]).
        assert_eq!(
            BLOCK_COMMITMENT_WORDS * BYTES_PER_GOLDILOCKS,
            BLOCK_COMMITMENT_BYTES
        );
        assert_eq!(BLOCK_COMMITMENT_WORDS, 8);
    }
}
