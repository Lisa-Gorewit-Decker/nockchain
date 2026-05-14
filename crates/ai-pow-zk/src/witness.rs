//! Private witness for the matmul SNARK.
//!
//! Carries everything the prover needs to fill in the AIR trace but the
//! verifier never sees in plaintext. Mirrors Pearl's `PrivateProofParams`
//! (`pearl/zk-pow/src/api/proof.rs:83-89`). Constructed by the caller in
//! `ai-pow` from a plain `MatmulProof` plus the matching
//! `(BlockNoise, Matrices)` reconstructed by the prover.

/// The private witness for the SNARK.
#[derive(Debug, Clone)]
pub struct Witness {
    /// `tile` row strips of `A` consumed by the found tile, each of
    /// length `k` (i.e. `tile * k` i8 entries in row-major order).
    pub a_rows: Vec<i8>,
    /// `tile` column strips of `B` consumed by the found tile, each of
    /// length `k`.
    pub b_cols: Vec<i8>,
    /// Per-block `E_L` factor flattened row-major (`m * r` i6 entries).
    pub e_l: Vec<i8>,
    /// Per-block `E_R` choice-matrix positions (one `(p_plus, p_minus)`
    /// pair per column of `E_R`, indexed by column `l ∈ 0..k`).
    pub e_r_pos: Vec<(u32, u32)>,
    /// Per-block `F_R` factor flattened col-major (`n * r` i6 entries).
    pub f_r: Vec<i8>,
    /// Per-block `F_L` choice-matrix positions (one `(p_plus, p_minus)`
    /// pair per row of `F_L`, indexed by row `l ∈ 0..k`).
    pub f_l_pos: Vec<(u32, u32)>,
    /// Per-stripe `M`-state evolution for the found tile, as 16 × i32
    /// per stripe step. `tile_states[step]` is the value of `M` after
    /// folding stripe `step`. Used by the AIR to constrain the
    /// rotate-13-XOR fold step-by-step.
    pub tile_states: Vec<[i32; 16]>,
}
