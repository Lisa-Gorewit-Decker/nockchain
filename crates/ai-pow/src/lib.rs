//! AI-PoW: replication-verified INT8 matrix-multiplication proof of useful work.
//!
//! Standalone Rust implementation of a Pearl-style PoUW puzzle. Miners search
//! for an output tile of `(A + E)(B + F)` whose hash falls below a 256-bit
//! target. Verifiers rerun a Fiat-Shamir-sampled subset of tiles and check
//! Merkle openings against a commitment. No SNARK / STARK is used.
//!
//! See `crates/ai-pow/README` and the project plan for design rationale.

pub mod commit;
pub mod fiat_shamir;
pub mod matmul;
pub mod params;
pub mod prng;
pub mod proof;
pub mod prover;
pub mod tile_hash;
pub mod verifier;

pub use crate::params::MatmulParams;
pub use crate::proof::{MatmulProof, TileOpening};
pub use crate::prover::{mine, ProverOptions};
pub use crate::verifier::{verify, VerifyError};
