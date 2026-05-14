//! Jackpot chip — 16-slot rotate-XOR-13 tile-state evolution.
//!
//! Port of `pearl/zk-pow/src/circuit/chip/jackpot/` — Pearl's
//! per-tile state update from §4.5 of the Pearl whitepaper.
//!
//! The state is a 16-slot register; each row updates exactly one
//! slot via the rotate-XOR-13 primitive:
//!
//! ```text
//!   JACKPOT_MSG_NEW[selected_slot] = rotate_left_13(JACKPOT_MSG[selected_slot]) XOR X
//!   JACKPOT_MSG_NEW[i] = JACKPOT_MSG[i]    for all other i
//! ```
//!
//! Selection is encoded one-hot via `SLOT_SEL[0..16]` (one element
//! is 1, the rest are 0). `X` is the XOR-fold value injected this
//! row (in the composite layout this comes from CUMSUM_BUFFER).
//!
//! ## Module layout (mirrors Pearl's `chip/jackpot/`)
//!
//! | Pearl file | Our module | Phase | Status |
//! |---|---|---|---|
//! | scalar reference (`rotate_xor_13`)            | [`compute`] | 10 | landed |
//! | AIR + trace generator (`jackpot_chip.rs`)     | [`chip`] | 10 | landed |
//!
//! ## Single-slot primitive
//!
//! The single-slot rotate-XOR-13 is already validated by
//! [`crate::state_chip`] (M9.1). This chip generalizes it to the
//! 16-slot routed variant.

pub mod chip;
pub mod compute;
