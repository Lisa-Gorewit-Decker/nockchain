//! §recursion — integrate the ai-pow-zk composite proof with the
//! vendored `Plonky3-recursion` substrate.
//!
//! Feature-gated behind `recursion`. This module lives in `ai-pow-zk`
//! (not the recursion workspace) because it is the *caller* side of a
//! generic API: `p3_recursion`'s verifier entrypoints are generic over
//! the inner AIR, and here we instantiate them with the concrete
//! `CompositeFullAirWithLookups` + `AiPowStarkConfig`. The recursion
//! substrate stays application-agnostic.
//!
//! S2 (this commit): establish the cross-workspace build path — the
//! recursion crates compile against `ai-pow-zk` and their entrypoints
//! are reachable here. S3 wires the composite proof through
//! `verify_batch_circuit`.

// S2 de-risk anchor — these resolving + type-checking proves the
// cross-workspace path-dependency build path works end to end.
pub use p3_recursion::{verify_batch_circuit, verify_p3_uni_proof_circuit};
