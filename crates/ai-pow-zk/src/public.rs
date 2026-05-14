//! Public inputs to the matmul SNARK.
//!
//! These are the values the verifier sees in plaintext; the witness
//! (`crate::witness::Witness`) carries everything else. Constructed by
//! the caller from a plain `MatmulProof` at the `ai-pow → ai-pow-zk`
//! boundary so this crate doesn't depend back on `ai-pow`.

/// The public values the SNARK attests to.
///
/// Mirrors Pearl's `PublicProofParams` (see
/// `pearl/zk-pow/src/api/proof.rs:58-71`) in spirit — every byte of state
/// the chain pins down ahead of the SNARK, plus the tile coordinate that
/// "wins" the hardness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInputs {
    /// `params_tag`: the 32-byte canonical hash of the matmul parameters
    /// (`ai_pow::prover::params_tag`).
    pub params_tag: [u8; 32],
    /// `h_a`: Pearl chunk-Merkle root over the row-major bytes of `A`.
    pub h_a: [u8; 32],
    /// `h_b`: Pearl chunk-Merkle root over the column-major bytes of `B`.
    pub h_b: [u8; 32],
    /// `comm_M`: Merkle root over the per-tile keyed-BLAKE3 leaves.
    pub comm_m: [u8; 32],
    /// `(i, j)` coordinates of the tile that satisfied the difficulty
    /// target.
    pub found_i: u32,
    pub found_j: u32,
    /// The keyed-hash leaf for the found tile (= `TileState::keyed_hash`).
    pub found_leaf: [u8; 32],
}
