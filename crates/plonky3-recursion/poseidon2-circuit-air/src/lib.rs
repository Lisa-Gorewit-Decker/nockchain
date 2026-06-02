//! An AIR for the Poseidon2 table for recursion. Handles sponge operations and compressions.
//!
//! ## CSA tamper-coverage routing
//!
//! Per the **Constraint Soundness Analysis** (CSA) — see
//! `crates/ai-pow-zk/docs/2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`
//! and `2026-05-20_CONSTRAINT_INVENTORY.md` § 4.1 — the Poseidon2
//! perm AIR (`d_max = 7` from the x⁷ S-box) contributes to the L1+L2
//! outer-cert soundness with per-AIR ε ≤ 2^(−105) at the production
//! `n_rows ≤ 2^20` ceiling (`2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md`
//! § 3.3) — ≥80 unconditional with +25-bit margin.
//!
//! Tamper coverage for this AIR is **upstream Plonky3 + indirect
//! via C3 stage tests**. The C1 substrate vendoring fixed-point
//! (`Plonky3-recursion` @ `c2c51fb`, aligned to upstream Plonky3
//! @ `524665d`) inherits upstream's tamper test suite. Our in-tree
//! coverage exercises Poseidon2 indirectly through the L1+L2
//! outer-cert composite acceptance + tamper-reject tests:
//! `c3_stage_a_l1_120bit_kat`, `c3_stage_b_l2_over_120bit_l1`,
//! `c3_stage_c_sweep_120bit` (in
//! `Plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs`).
//!
//! **No in-tree dedicated Poseidon2 tamper test is added** — the
//! vendoring rev `c2c51fb` is the authoritative test-source per CSA
//! S2 § 2.3 (GAP-G2 disposition: upstream-routing label, no new test
//! code).

#![no_std]

extern crate alloc;

mod air;
mod columns;
mod public_types;

pub use air::*;
pub use columns::*;
pub use public_types::*;
