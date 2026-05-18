//! Tip5 permutation non-primitive operation (C2 / M-S4).
//!
//! **C2.2 (this commit): the configuration / NPO-key bundle only.**
//! The full NPO machinery — `call`, `plugin`, `executor`, `builder`,
//! `state`, `trace` (mirroring `poseidon1_perm`) — is C2.3 (threading
//! Tip5 through the in-circuit challenger / MMCS / FRI verifier). The
//! permutation *constraint system* already exists and is KAT-anchored
//! to `nockchain_math::tip5::permute` in the sibling
//! `p3-tip5-circuit-air` crate (C2.1, validated).

pub(crate) mod config;

pub use config::{Tip5Config, Tip5FieldId};
// `Tip5PermExec` / `Tip5PermConfigData` are defined in `config` and
// re-exported here in C2.3 when the NPO executor/plugin consume them.
