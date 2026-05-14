//! Column layout for the M10.1c Pearl-style composite AIR.
//!
//! Direct port of `pearl/zk-pow/src/circuit/pearl_layout.rs`. The full
//! column set is mirrored, including Pearl's `NOISED_PACKED` /
//! `A_NOISED` / `B_NOISED` / `MAT_ID` machinery for the matrix-tile
//! RAM lookups. The lookups are essential for production-scale
//! matmul (see `M10_1C_DESIGN.md` decision 2 — without them, large
//! matrices would force per-row inline duplication of matrix bytes
//! that scales with the number of output tile cells rather than the
//! matrix size).
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
//! | 4 | `chips::input`, `chips::i8u8`, `chips::range_table` |
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

/// Bits per range-table limb. Matches Pearl's
/// `pearl_columns::BITS_PER_LIMB = 13` — the u13 range table is
/// what decomposes `MAT_ID` (RAM-lookup index into `NOISED_PACKED`)
/// into limbs for range-checking. Production matrices need MAT_ID
/// values up to `(m + n) × k / BYTES_PER_GOLDILOCKS`, comfortably
/// within `2^26 = 2 × 13` bits.
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
//  Layout order:
//    range tables           (11 cols)
//    control flags          (22 cols incl. CONTROL_PREP)
//    input chip unpacking   (25 cols: MAT_UNPACK, UINT8_DATA, NOISE_PACKED_PREP, NOISE_UNPACK)
//    NOISED_PACKED + indexing (12 cols: NOISED_PACKED, MAT_FREQ, MAT_ID, MAT_ID_LIMBS, AB_ID_PREP, AB_ID_LIMBS, A_ID, B_ID)
//    STARK_ROW_IDX          (1 col)
//    matmul tile data       (80 cols: A_NOISED, A_NOISED_UNPACK, B_NOISED, B_NOISED_UNPACK)
//    matmul accumulator     (8 cols: CUMSUM_TILE, CUMSUM_BUFFER)
//    jackpot state          (56 cols: JACKPOT_MSG, BIT_REG, JACKPOT_IDX)
//    blake3 buffers         (49 cols: BLAKE3_MSG_BUFFER, CV_OR_TWEAK_PREP, CV_IN, BLAKE3_MSG, BLAKE3_CV)
//    blake3 round AIR       (1056 cols: BLAKE3_ROUND)
//    blake3 output          (9 cols: CV_OUT, CV_OUT_FREQ)
//    ───────────────────────────────────
//    TOTAL_TRACE_WIDTH      ≈ 1328 cols
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

/// Preprocessed control word. Packs the selector bits *and* MAT_ID
/// (the index used for NOISED_PACKED RAM lookups). The composite
/// trace generator emits a little-endian unpacking constraint over
/// this column.
pub const CONTROL_PREP: usize = NUM_RANGE_COLS;

/// Per-chip selector bits (unpacked from `CONTROL_PREP`).
/// Pearl-mirror; matches `pearl_layout.rs:22-42` verbatim.
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

// =====================================================================
//  RAM-lookup machinery + input chip columns
//
//  Direct port of `pearl_layout.rs:43-80` covering the input-chip
//  unpacking columns (`MAT_UNPACK`, `UINT8_DATA`, `NOISE_PACKED_PREP`,
//  `NOISE_UNPACK`), the canonical noised-matrix store
//  (`NOISED_PACKED`, `MAT_FREQ`), the matmul-side per-row reads
//  (`A_NOISED`, `A_NOISED_UNPACK`, `B_NOISED`, `B_NOISED_UNPACK`),
//  the MAT_ID / AB_ID indexing infrastructure, and the BLAKE3 +
//  Jackpot column blocks.
//
//  Widths match Pearl exactly so byte-equivalence of the in-circuit
//  hash output to `blake3::Hasher::new_keyed` is preserved at the
//  trace level.
// =====================================================================

// ---- Matrix-byte unpacking ----

/// MAT_UNPACK: 8 i7 elements. Matches Pearl's `MAT_UNPACK: 8`.
pub const MAT_UNPACK_LEN: usize = 8;
pub const MAT_UNPACK_START: usize = CONTROL_PREP + NUM_CONTROL_COLS;

/// UINT8_DATA: 8 u8 elements. When `IS_MSG_MAT` fires, these are
/// the i7 → u8 conversions of `MAT_UNPACK`; otherwise auxiliary
/// data feeding the BLAKE3 message buffer.
pub const UINT8_DATA_LEN: usize = 8;
pub const UINT8_DATA_START: usize = MAT_UNPACK_START + MAT_UNPACK_LEN;

/// NOISE_PACKED_PREP: 1 preprocessed col. Noise associated with the
/// matrix entry packed in this row. Mirrors Pearl
/// `pearl_layout.rs:51`.
pub const NOISE_PACKED_PREP: usize = UINT8_DATA_START + UINT8_DATA_LEN;

/// NOISE_UNPACK: 8 i7 elements. Decomposes `NOISE_PACKED_PREP` for
/// the matmul tile additions.
pub const NOISE_UNPACK_LEN: usize = 8;
pub const NOISE_UNPACK_START: usize = NOISE_PACKED_PREP + 1;

// ---- NOISED_PACKED: canonical noised-matrix data table ----

/// NOISED_PACKED: 2 cols, holding `MAT + NOISE` packed as 4 i8 per
/// Goldilocks. Read by both the matmul chip (via `A_NOISED` /
/// `B_NOISED` LogUp on `MAT_ID`) and the BLAKE3 leaf rows (via
/// `IS_MSG_MAT` → `UINT8_DATA` after i8↔u8 conversion). This is
/// the cryptographic glue forcing matmul and BLAKE3 to see the
/// same bytes.
pub const NOISED_PACKED_LEN: usize = 2;
pub const NOISED_PACKED_START: usize = NOISE_UNPACK_START + NOISE_UNPACK_LEN;

/// MAT_FREQ: 1 col. Number of times this NOISED_PACKED row is read
/// across the matmul + BLAKE3 consumers. LogUp multiplicities.
pub const MAT_FREQ: usize = NOISED_PACKED_START + NOISED_PACKED_LEN;

// ---- MAT_ID and AB_ID indexing ----

/// MAT_ID: 1 col. Compact identifier of this row's matrix position
/// in NOISED_PACKED. Derived from `CONTROL_PREP`.
pub const MAT_ID: usize = MAT_FREQ + 1;

/// MAT_ID_LIMBS: 2 cols. Range check for MAT_ID via two 13-bit
/// limbs (since `BITS_PER_LIMB = 13`, 2 limbs cover values up to
/// `2^26` which accommodates all production matrix sizes).
pub const MAT_ID_LIMBS_LEN: usize = 2;
pub const MAT_ID_LIMBS_START: usize = MAT_ID + 1;

/// AB_ID_PREP: 1 preprocessed col. Packs `A_ID || B_ID` for the
/// matmul tile load.
pub const AB_ID_PREP: usize = MAT_ID_LIMBS_START + MAT_ID_LIMBS_LEN;

/// AB_ID_LIMBS: 4 cols. Range check for AB_ID_PREP.
pub const AB_ID_LIMBS_LEN: usize = 4;
pub const AB_ID_LIMBS_START: usize = AB_ID_PREP + 1;

/// A_ID, B_ID: 1 col each. Unpacked from AB_ID_PREP. These index
/// into NOISED_PACKED via LogUp on the matmul chip.
pub const A_ID: usize = AB_ID_LIMBS_START + AB_ID_LIMBS_LEN;
pub const B_ID: usize = A_ID + 1;

// ---- STARK_ROW_IDX ----

/// STARK_ROW_IDX: 1 col, monotonically increasing from 0. Used by
/// the CV-routing lookup to address arbitrary previous rows.
pub const STARK_ROW_IDX: usize = B_ID + 1;

// ---- Per-row matmul tile data ----

/// A_NOISED: 8 packed Goldilocks cols (TILE_H × TILE_D / 4).
/// Per-matmul-row read of A's tile from NOISED_PACKED[A_ID].
pub const A_NOISED_LEN: usize = TILE_H * TILE_D / BYTES_PER_GOLDILOCKS;
pub const A_NOISED_START: usize = STARK_ROW_IDX + 1;

/// A_NOISED_UNPACK: 32 i8 cols (TILE_H × TILE_D). Unpacked from
/// A_NOISED; range-checked via IRANGE8.
pub const A_NOISED_UNPACK_LEN: usize = TILE_H * TILE_D;
pub const A_NOISED_UNPACK_START: usize = A_NOISED_START + A_NOISED_LEN;

/// B_NOISED: 8 packed Goldilocks cols (TILE_W × TILE_D / 4 = TILE_H
/// × TILE_D / 4 since Pearl uses TILE_W = TILE_H). Per-matmul-row
/// read of B's tile from NOISED_PACKED[B_ID].
pub const B_NOISED_LEN: usize = TILE_H * TILE_D / BYTES_PER_GOLDILOCKS;
pub const B_NOISED_START: usize = A_NOISED_UNPACK_START + A_NOISED_UNPACK_LEN;

/// B_NOISED_UNPACK: 32 i8 cols.
pub const B_NOISED_UNPACK_LEN: usize = TILE_H * TILE_D;
pub const B_NOISED_UNPACK_START: usize = B_NOISED_START + B_NOISED_LEN;

// ---- Cumulative-sum (matmul accumulator) + jackpot ----

/// CUMSUM_TILE: 4 i32 cols (TILE_H × TILE_W = 2 × 2 = 4).
pub const CUMSUM_TILE_LEN: usize = TILE_H * TILE_H;
pub const CUMSUM_TILE_START: usize = B_NOISED_UNPACK_START + B_NOISED_UNPACK_LEN;

/// CUMSUM_BUFFER: 4 i32 cols. Used for buffering CUMSUM_TILE across
/// the jackpot XOR-fold steps.
pub const CUMSUM_BUFFER_LEN: usize = TILE_H * TILE_H;
pub const CUMSUM_BUFFER_START: usize = CUMSUM_TILE_START + CUMSUM_TILE_LEN;

/// JACKPOT_MSG: 16 u32 cols. The 16-slot tile-state value M that
/// the jackpot chip evolves via rotate-XOR-13.
pub const JACKPOT_MSG_LEN: usize = JACKPOT_SIZE;
pub const JACKPOT_MSG_START: usize = CUMSUM_BUFFER_START + CUMSUM_BUFFER_LEN;

/// BIT_REG: 32 boolean cols. Bitwise representation of one u32
/// from JACKPOT_MSG, used for the XOR + rotate operations.
pub const BIT_REG_LEN: usize = 32;
pub const BIT_REG_START: usize = JACKPOT_MSG_START + JACKPOT_MSG_LEN;

/// JACKPOT_IDX: 8 cols. Indicators `is_store[i]` / `is_load[i]`
/// for i in 0..16 — one-hot per row.
pub const JACKPOT_IDX_LEN: usize = 8;
pub const JACKPOT_IDX_START: usize = BIT_REG_START + BIT_REG_LEN;

// ---- BLAKE3 buffers, message, CVs ----

/// BLAKE3_MSG_BUFFER: 16 u32 cols. Multi-round staging buffer; in
/// round 8 holds the message that entered round 1.
pub const BLAKE3_MSG_BUFFER_LEN: usize = 16;
pub const BLAKE3_MSG_BUFFER_START: usize = JACKPOT_IDX_START + JACKPOT_IDX_LEN;

/// CV_OR_TWEAK_PREP: 1 preprocessed col. Either a row index (CV
/// lookup) or BLAKE3 compression tweak flags (counter / block_len
/// / flags).
pub const CV_OR_TWEAK_PREP: usize = BLAKE3_MSG_BUFFER_START + BLAKE3_MSG_BUFFER_LEN;

/// CV_IN: 8 u32 cols. Read from `CV_OUT_PACKED` at the row indexed
/// by `CV_OR_TWEAK_PREP` via LogUp.
pub const CV_IN_LEN: usize = 8;
pub const CV_IN_START: usize = CV_OR_TWEAK_PREP + 1;

/// BLAKE3_MSG: 16 u32 cols. Message entering BLAKE3, packed LE 4
/// bytes per Goldilocks (u8 view).
pub const BLAKE3_MSG_LEN: usize = 16;
pub const BLAKE3_MSG_START: usize = CV_IN_START + CV_IN_LEN;

/// BLAKE3_CV: 8 u32 cols. CVs ready for BLAKE3.
pub const BLAKE3_CV_LEN: usize = 8;
pub const BLAKE3_CV_START: usize = BLAKE3_MSG_START + BLAKE3_MSG_LEN;

/// BLAKE3_ROUND: 1056 cols. The per-round AIR ensuring this row's
/// BLAKE3 round was executed correctly. Pearl pins this at exactly
/// 1056; CV_OUT of the last round contains the BLAKE3 output.
pub const BLAKE3_ROUND_LEN: usize = 1056;
pub const BLAKE3_ROUND_START: usize = BLAKE3_CV_START + BLAKE3_CV_LEN;

/// CV_OUT: 8 u32 cols. Output CV of BLAKE3 for this row.
pub const CV_OUT_LEN: usize = 8;
pub const CV_OUT_START: usize = BLAKE3_ROUND_START + BLAKE3_ROUND_LEN;

// ---- Jackpot chip extensions (Phase 12d) ----
//
// The jackpot chip uses three column blocks not present in
// Pearl's original layout: 32 boolean cells for the rotate-XOR
// operand's bit decomposition, and 16 boolean cells encoding the
// active slot as a one-hot indicator. Appended after CV_OUT to
// preserve all earlier offsets.

/// JACKPOT_X_BITS: 32 boolean cells. Bit-decomposition of the
/// XOR-fold value `x` consumed by the jackpot chip's rotate-XOR-13
/// update. Sourced upstream (Phase 14b will tie these via LogUp to
/// CUMSUM_BUFFER's bit decomposition).
pub const JACKPOT_X_BITS_START: usize = CV_OUT_START + CV_OUT_LEN;
pub const JACKPOT_X_BITS_LEN: usize = 32;

/// JACKPOT_SLOT_SEL: 16 boolean cells, one-hot indicator selecting
/// which slot the current row updates. Sum equals
/// `IS_HASH_JACKPOT` (1 on an active jackpot row, 0 otherwise).
pub const JACKPOT_SLOT_SEL_START: usize = JACKPOT_X_BITS_START + JACKPOT_X_BITS_LEN;
pub const JACKPOT_SLOT_SEL_LEN: usize = 16;

/// CV_OUT_FREQ: 1 col. LogUp multiplicity for the CV-routing
/// lookup (how many later rows read this row's CV_OUT).
pub const CV_OUT_FREQ: usize = JACKPOT_SLOT_SEL_START + JACKPOT_SLOT_SEL_LEN;

// =====================================================================
//  Total trace width
// =====================================================================

/// Total trace width: pinned end-of-layout cursor. Phases 3+ extend
/// chip-internal sub-columns but must not exceed this without bumping
/// the constant.
pub const TOTAL_TRACE_WIDTH: usize = CV_OUT_FREQ + 1;

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase-2 / Phase-2.5 const-pinning anchor: any reshuffling
    /// of the layout triggers this test, forcing the constraint
    /// code / trace generator / lookups to update in lockstep.
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
    }

    /// Pearl byte-equivalence anchor: every column width matches
    /// Pearl's `pearl_layout.rs` definitions verbatim. If Pearl
    /// changes these (extremely unlikely; their column shape is
    /// pinned by the BLAKE3 round-AIR), the lookup configurations
    /// downstream would silently diverge.
    #[test]
    fn ram_lookup_column_widths_match_pearl() {
        assert_eq!(JACKPOT_X_BITS_LEN, 32);
        assert_eq!(JACKPOT_SLOT_SEL_LEN, 16);
        assert_eq!(MAT_UNPACK_LEN, 8);
        assert_eq!(UINT8_DATA_LEN, 8);
        assert_eq!(NOISE_UNPACK_LEN, 8);
        assert_eq!(NOISED_PACKED_LEN, 2);
        assert_eq!(MAT_ID_LIMBS_LEN, 2);
        assert_eq!(AB_ID_LIMBS_LEN, 4);
        assert_eq!(A_NOISED_LEN, 8); // TILE_H × TILE_D / 4
        assert_eq!(A_NOISED_UNPACK_LEN, 32); // TILE_H × TILE_D
        assert_eq!(B_NOISED_LEN, 8);
        assert_eq!(B_NOISED_UNPACK_LEN, 32);
        assert_eq!(CUMSUM_TILE_LEN, 4);
        assert_eq!(CUMSUM_BUFFER_LEN, 4);
        assert_eq!(JACKPOT_MSG_LEN, 16);
        assert_eq!(BIT_REG_LEN, 32);
        assert_eq!(JACKPOT_IDX_LEN, 8);
        assert_eq!(BLAKE3_MSG_BUFFER_LEN, 16);
        assert_eq!(CV_IN_LEN, 8);
        assert_eq!(BLAKE3_MSG_LEN, 16);
        assert_eq!(BLAKE3_CV_LEN, 8);
        assert_eq!(BLAKE3_ROUND_LEN, 1056);
        assert_eq!(CV_OUT_LEN, 8);
    }

    /// All column offsets must be strictly increasing and contiguous.
    #[test]
    fn layout_offsets_are_contiguous() {
        let checkpoints: &[(usize, usize, &str)] = &[
            (NUM_RANGE_COLS, CONTROL_PREP, "range → CONTROL_PREP"),
            (
                CONTROL_PREP + NUM_CONTROL_COLS,
                MAT_UNPACK_START,
                "control → MAT_UNPACK",
            ),
            (
                MAT_UNPACK_START + MAT_UNPACK_LEN,
                UINT8_DATA_START,
                "MAT_UNPACK → UINT8_DATA",
            ),
            (
                UINT8_DATA_START + UINT8_DATA_LEN,
                NOISE_PACKED_PREP,
                "UINT8_DATA → NOISE_PACKED_PREP",
            ),
            (
                NOISE_PACKED_PREP + 1,
                NOISE_UNPACK_START,
                "NOISE_PACKED_PREP → NOISE_UNPACK",
            ),
            (
                NOISE_UNPACK_START + NOISE_UNPACK_LEN,
                NOISED_PACKED_START,
                "NOISE_UNPACK → NOISED_PACKED",
            ),
            (
                NOISED_PACKED_START + NOISED_PACKED_LEN,
                MAT_FREQ,
                "NOISED_PACKED → MAT_FREQ",
            ),
            (MAT_FREQ + 1, MAT_ID, "MAT_FREQ → MAT_ID"),
            (MAT_ID + 1, MAT_ID_LIMBS_START, "MAT_ID → MAT_ID_LIMBS"),
            (
                MAT_ID_LIMBS_START + MAT_ID_LIMBS_LEN,
                AB_ID_PREP,
                "MAT_ID_LIMBS → AB_ID_PREP",
            ),
            (
                AB_ID_PREP + 1,
                AB_ID_LIMBS_START,
                "AB_ID_PREP → AB_ID_LIMBS",
            ),
            (
                AB_ID_LIMBS_START + AB_ID_LIMBS_LEN,
                A_ID,
                "AB_ID_LIMBS → A_ID",
            ),
            (A_ID + 1, B_ID, "A_ID → B_ID"),
            (B_ID + 1, STARK_ROW_IDX, "B_ID → STARK_ROW_IDX"),
            (
                STARK_ROW_IDX + 1,
                A_NOISED_START,
                "STARK_ROW_IDX → A_NOISED",
            ),
            (
                A_NOISED_START + A_NOISED_LEN,
                A_NOISED_UNPACK_START,
                "A_NOISED → A_NOISED_UNPACK",
            ),
            (
                A_NOISED_UNPACK_START + A_NOISED_UNPACK_LEN,
                B_NOISED_START,
                "A_NOISED_UNPACK → B_NOISED",
            ),
            (
                B_NOISED_START + B_NOISED_LEN,
                B_NOISED_UNPACK_START,
                "B_NOISED → B_NOISED_UNPACK",
            ),
            (
                B_NOISED_UNPACK_START + B_NOISED_UNPACK_LEN,
                CUMSUM_TILE_START,
                "B_NOISED_UNPACK → CUMSUM_TILE",
            ),
            (
                CUMSUM_TILE_START + CUMSUM_TILE_LEN,
                CUMSUM_BUFFER_START,
                "CUMSUM_TILE → CUMSUM_BUFFER",
            ),
            (
                CUMSUM_BUFFER_START + CUMSUM_BUFFER_LEN,
                JACKPOT_MSG_START,
                "CUMSUM_BUFFER → JACKPOT_MSG",
            ),
            (
                JACKPOT_MSG_START + JACKPOT_MSG_LEN,
                BIT_REG_START,
                "JACKPOT_MSG → BIT_REG",
            ),
            (
                BIT_REG_START + BIT_REG_LEN,
                JACKPOT_IDX_START,
                "BIT_REG → JACKPOT_IDX",
            ),
            (
                JACKPOT_IDX_START + JACKPOT_IDX_LEN,
                BLAKE3_MSG_BUFFER_START,
                "JACKPOT_IDX → BLAKE3_MSG_BUFFER",
            ),
            (
                BLAKE3_MSG_BUFFER_START + BLAKE3_MSG_BUFFER_LEN,
                CV_OR_TWEAK_PREP,
                "BLAKE3_MSG_BUFFER → CV_OR_TWEAK_PREP",
            ),
            (
                CV_OR_TWEAK_PREP + 1,
                CV_IN_START,
                "CV_OR_TWEAK_PREP → CV_IN",
            ),
            (
                CV_IN_START + CV_IN_LEN,
                BLAKE3_MSG_START,
                "CV_IN → BLAKE3_MSG",
            ),
            (
                BLAKE3_MSG_START + BLAKE3_MSG_LEN,
                BLAKE3_CV_START,
                "BLAKE3_MSG → BLAKE3_CV",
            ),
            (
                BLAKE3_CV_START + BLAKE3_CV_LEN,
                BLAKE3_ROUND_START,
                "BLAKE3_CV → BLAKE3_ROUND",
            ),
            (
                BLAKE3_ROUND_START + BLAKE3_ROUND_LEN,
                CV_OUT_START,
                "BLAKE3_ROUND → CV_OUT",
            ),
            (
                CV_OUT_START + CV_OUT_LEN,
                JACKPOT_X_BITS_START,
                "CV_OUT → JACKPOT_X_BITS",
            ),
            (
                JACKPOT_X_BITS_START + JACKPOT_X_BITS_LEN,
                JACKPOT_SLOT_SEL_START,
                "JACKPOT_X_BITS → JACKPOT_SLOT_SEL",
            ),
            (
                JACKPOT_SLOT_SEL_START + JACKPOT_SLOT_SEL_LEN,
                CV_OUT_FREQ,
                "JACKPOT_SLOT_SEL → CV_OUT_FREQ",
            ),
            (
                CV_OUT_FREQ + 1,
                TOTAL_TRACE_WIDTH,
                "CV_OUT_FREQ → TOTAL_TRACE_WIDTH",
            ),
        ];
        for &(end, next, name) in checkpoints {
            assert_eq!(end, next, "layout discontinuity at {name}: {end} != {next}");
        }
    }

    /// TOTAL_TRACE_WIDTH ≈ Pearl's pinned width (~1300 cols). The
    /// number is dominated by `BLAKE3_ROUND_LEN = 1056` — the per-
    /// round AIR — which is the bulk of any row regardless of which
    /// chip is active.
    #[test]
    fn total_trace_width_in_pearl_ballpark() {
        // Sanity: trace is at least 1200 cols (BLAKE3 round dominates)
        // and at most 1400 cols (no accidental column explosion).
        assert!(
            TOTAL_TRACE_WIDTH > 1200,
            "TOTAL_TRACE_WIDTH suspiciously small: {TOTAL_TRACE_WIDTH}"
        );
        assert!(
            TOTAL_TRACE_WIDTH < 1400,
            "TOTAL_TRACE_WIDTH suspiciously large: {TOTAL_TRACE_WIDTH} — check for unintended column duplication"
        );
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
