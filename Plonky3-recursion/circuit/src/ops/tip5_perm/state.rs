//! Execution state and (stub) private data for Tip5 permutation
//! operations (C2.3).
//!
//! Mirrors `poseidon1_perm::state`, with the merkle path removed: the
//! deployed Tip5 is sponge/challenger only (no MMCS), so there is no
//! sibling private data and only one `last_output` chain slot.

use alloc::vec::Vec;

use p3_tip5_circuit_air::Tip5CircuitRow;

/// Private data for Tip5 permutation.
///
/// Tip5 has **no** merkle mode, so no private payload is ever
/// consumed. Kept as a typed stub for parity with
/// `Poseidon1PermPrivateData` and to give a stable downcast target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tip5PermPrivateData<F> {
    /// Always empty — Tip5 carries no merkle sibling.
    pub _unused: Vec<F>,
}

/// Execution state for Tip5 permutation operations.
#[derive(Debug, Default)]
pub(crate) struct Tip5ExecutionState<F> {
    /// Previous permutation full-state output, carried into the next
    /// non-`new_start` row's `IN` (sponge overwrite mode, full width
    /// including capacity — matches `PaddingFreeSponge`).
    pub last_output: Option<Vec<F>>,
    /// Circuit rows captured during execution.
    pub rows: Vec<Tip5CircuitRow<F>>,
}
