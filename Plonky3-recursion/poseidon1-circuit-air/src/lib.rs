//! An AIR for the Poseidon1 table for recursion. Handles sponge operations and compressions.
//!
//! ## CSA tamper-coverage routing
//!
//! Per the **Constraint Soundness Analysis** (CSA) — see
//! `crates/ai-pow-zk/docs/2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`
//! and `2026-05-20_CONSTRAINT_INVENTORY.md` § 4.2 — the Poseidon1
//! perm AIR (`d_max = 7` from the x⁷ S-box) is the D=1-in-D>1
//! permutation pattern mirror (the template for Tip5's
//! `WITNESS_EXT_D` D-padding). Same soundness shape as Poseidon2:
//! per-AIR ε ≤ 2^(−105) at the production `n_rows ≤ 2^20` ceiling.
//!
//! Tamper coverage for this AIR is **upstream Plonky3 + indirect
//! via C3 stage tests** (same disposition as Poseidon2 — see
//! `poseidon2-circuit-air/src/lib.rs`). No in-tree dedicated
//! tamper test; vendoring rev `c2c51fb` is the authoritative
//! test-source per CSA S2 § 2.3 (GAP-G2 disposition).

#![no_std]

extern crate alloc;

mod air;
mod columns;
mod public_types;

pub use air::*;
pub use columns::*;
pub use public_types::*;
