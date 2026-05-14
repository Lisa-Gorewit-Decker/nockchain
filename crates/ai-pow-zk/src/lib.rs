//! Plonky3 SNARK circuit for the `ai-pow` tiling matmul puzzle.
//!
//! Mirrors Pearl's `zk-pow` role: where Pearl uses Plonky2 to compress
//! its multi-MB `PlainProof` into a ~60 KB `ZKProof` (see
//! `pearl/zk-pow/src/api/prove.rs::zk_prove_plain_proof`), this crate
//! uses Plonky3 over BabyBear to do the equivalent for the `ai-pow`
//! plain proof.
//!
//! ## Architectural note
//!
//! `ai-pow-zk` is intentionally **standalone** — it does **not** depend
//! on `ai-pow`. The proving crate (`ai-pow`) is the consumer; making
//! `ai-pow-zk` depend back on it would introduce a circular workspace
//! dep. The caller in `ai-pow` constructs [`ZkParams`], [`Witness`], and
//! [`PublicInputs`] from its own types ([`ai_pow::params::MatmulParams`],
//! [`ai_pow::proof::MatmulProof`]) at the call site.
//!
//! ## Scope
//!
//! What this crate is intended to provide once the circuit is implemented:
//!
//! 1. A [`MatmulAir`](air::MatmulAir) AIR encoding Pearl §4.5: per-stripe
//!    r-wide INT8 dot product, int32-XOR fold, 16-slot `M`-state
//!    rotate-13-XOR update.
//! 2. A keyed BLAKE3 sub-circuit (or `p3-keccak-air` analog) for the tile
//!    leaf and the matrix-commitment chunk-Merkle roots.
//! 3. A FRI-based commitment configuration producing a fixed-size proof
//!    blob (target: ≤ 60 KB at production matmul shapes).
//! 4. A verifier that reconstructs the AIR and runs `p3_uni_stark::verify`.
//!
//! Currently every entrypoint is a `todo!()` stub. The structure exists
//! so callers in `ai-pow` can wire the call sites now (gated behind the
//! `zk` feature) and have the implementation fill in independently.
//!
//! ## Reference Pearl call shape
//!
//! For analog mapping, see `pearl/zk-pow/src/api/prove.rs`:
//!
//! ```ignore
//! pub fn zk_prove_plain_proof(
//!     block_header: IncompleteBlockHeader,
//!     plain_proof: &PlainProof,
//!     cache: &mut CircuitCache,
//!     sanity_check: bool,
//! ) -> Result<ProveResult>;
//! ```
//!
//! The corresponding entry here is [`prove`], and `verify` is its
//! counterpart.

pub mod air;
pub mod blake3_air;
pub mod circuit;
pub mod params;
pub mod public;
pub mod witness;

// Re-export the concrete field choices so consumers (and the AIR /
// circuit modules) don't have to re-import Plonky3 crates directly.
pub use p3_goldilocks::Goldilocks as Val;

use thiserror::Error;

pub use crate::air::MatmulAir;
pub use crate::circuit::CircuitConfig;
pub use crate::params::ZkParams;
pub use crate::public::PublicInputs;
pub use crate::witness::Witness;

/// Opaque serialized Plonky3 STARK proof. The byte layout is internal to
/// this crate; consumers persist the `Vec<u8>` verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZkProof(pub Vec<u8>);

#[derive(Debug, Error)]
pub enum ProveError {
    /// The supplied `Witness` is malformed or inconsistent with `ZkParams`.
    #[error("witness shape mismatch: {0}")]
    Witness(String),
    /// Plonky3 prover surfaced an error during trace generation,
    /// commitment, or FRI.
    #[error("plonky3: {0}")]
    Plonky3(String),
    /// `ZkParams::validate` failed.
    #[error("invalid params: {0}")]
    Params(String),
}

#[derive(Debug, Error)]
pub enum VerifyError {
    /// The proof bytes are not a well-formed Plonky3 STARK proof.
    #[error("malformed proof: {0}")]
    Malformed(String),
    /// The Plonky3 verifier rejected the proof.
    #[error("plonky3 rejected proof: {0}")]
    Rejected(String),
    /// Public inputs do not pass shape / range validation.
    #[error("invalid public inputs: {0}")]
    PublicInputs(String),
    /// `ZkParams::validate` failed.
    #[error("invalid params: {0}")]
    Params(String),
}

/// Build a Plonky3 STARK that attests to the existence of a [`Witness`]
/// producing [`PublicInputs`] for the given `(block_commitment, nonce,
/// params)`.
///
/// This is the entry the `ai-pow` prover calls after building a plain
/// `MatmulProof`, matching the position of `zk_prove_plain_proof` in
/// Pearl's pipeline.
pub fn prove(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &ZkParams,
    public_inputs: &PublicInputs,
    witness: &Witness,
) -> Result<ZkProof, ProveError> {
    let _ = (block_commitment, nonce, params, public_inputs, witness);
    todo!(
        "build the Plonky3 trace with `Witness`, commit via FRI / Poseidon2 \
         / MerkleTreeMmcs, and call `p3_uni_stark::prove`"
    )
}

/// Verify a [`ZkProof`] against a set of [`PublicInputs`] extracted from
/// the chain. Mirrors Pearl's `ZKProof::verify`.
pub fn verify(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &ZkParams,
    public_inputs: &PublicInputs,
    proof: &ZkProof,
) -> Result<(), VerifyError> {
    let _ = (block_commitment, nonce, params, public_inputs, proof);
    todo!(
        "reconstruct `MatmulAir` from `params`, call `p3_uni_stark::verify` \
         with the same StarkConfig the prover used, and check the public \
         inputs match"
    )
}
