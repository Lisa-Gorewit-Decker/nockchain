//! User-facing call struct for adding Tip5 permutation rows (C2.3).
//!
//! Mirrors `Poseidon1PermCallBase` (the D=1, non-merkle base path),
//! specialised to the deployed Tip5 sponge geometry: 16 base-field
//! input slots, 10 rate output exposure flags.

use crate::ops::tip5_perm::config::Tip5Config;
use crate::types::ExprId;

/// User-facing arguments for adding a Tip5 perm row (one 7-round Tip5
/// permutation), D=1 base field. Mirrors `Poseidon1PermCallBase`.
pub struct Tip5PermCall {
    /// Tip5 configuration for this permutation row (always D=1
    /// Goldilocks, width 16, rate 10).
    pub config: Tip5Config,
    /// Flag indicating whether a new sponge chain is started (no
    /// previous-state carry).
    pub new_start: bool,
    /// Optional CTL exposure for each of the 16 input elements. If
    /// `None`, the element is not exposed via CTL (and is taken from
    /// the chain carry or zero).
    pub inputs: [Option<ExprId>; 16],
    /// Output exposure flags for the rate elements (first RATE=10).
    /// When `out_ctl[i]` is true, output[i] is CTL-verified.
    pub out_ctl: [bool; 10],
    /// Whether to return all 16 output elements (for challenger use).
    /// When true, outputs 10-15 are also allocated and returned, but
    /// NOT CTL-verified (capacity elements, constrained only by the
    /// Tip5 permutation itself).
    pub return_all_outputs: bool,
}

impl Default for Tip5PermCall {
    fn default() -> Self {
        Self {
            config: Tip5Config::GOLDILOCKS_W16,
            new_start: false,
            inputs: [None; 16],
            out_ctl: [false; 10],
            return_all_outputs: false,
        }
    }
}
