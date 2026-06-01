//! AI-PoW v3: Pearl proof-of-useful-work with miner-supplied `A` and `B`.
//!
//! Implements the Pearl Whitepaper PoUW puzzle for caller-chosen INT8
//! matrices `A` and `B`:
//!
//! 1. **Commitments** (Pearl §4.3, Alg. 2): `κ` derived from the
//!    nonce-bound attempt state `(block_commitment, nonce, params_tag)`;
//!    `H_A` and `H_B` are BLAKE3-Merkle roots over rows of `A` and columns
//!    of `B` keyed by `κ`; seeds
//!    `s_B = derive_key("s_b", κ ‖ H_B)` and `s_A = derive_key("s_a",
//!    s_B ‖ H_A)` bind the noise to the chosen matrices.
//! 2. **Low-rank noise** (Pearl §4.4, Alg. 3): `E = E_L · E_R` and
//!    `F = F_L · F_R` of rank `r`; `E_L, F_R` are int6, `E_R, F_L` are
//!    choice matrices (one `+1` and one `-1` per col/row).
//! 3. **Iterative tile state** (Pearl §4.5, Alg. 4): 512-bit `M_{i,j}`
//!    accumulator updated every `r`-stripe along the `k`-axis via
//!    `M[ℓ mod 16] ← (M[ℓ mod 16] ≪ 13) ⊕ X_ℓ`.
//! 4. **Shape-aware hardness**: `BLAKE3(M, key = pow_key) ≤ 2^(256-b) · r · t^2`,
//!    with `pow_key = derive_key("pow-key", s_A ‖ nonce)`. Since `s_A` is
//!    already nonce-bound through `κ`, changing the nonce requires fresh
//!    commitments, noise, and matmul-derived tile states before the hash check.
//!
//! The proof contains `H_A`, `H_B`, and per-opening row/column strips of
//! `A`/`B` with BLAKE3 Merkle authentication paths up to those roots, so
//! the verifier can replicate one tile without seeing the full matrices.
//! No SNARK / STARK is used (Pearl §4.7 — separate work).

pub mod commit;
pub mod fiat_shamir;
pub mod matmul;
pub mod ncmn;
pub mod params;
pub mod prng;
pub mod proof;
pub mod prover;
pub mod quant;
pub mod synth;
pub mod tile_hash;
pub mod verifier;

/// F1 integration: `MatmulProof` → `ai-pow-zk` `CompositeTrace`.
/// Only compiled with the `zk` feature (pulls in `ai-pow-zk`).
#[cfg(feature = "zk")]
pub mod zk_bridge;

pub use crate::params::MatmulParams;
pub use crate::proof::{MatmulProof, TileOpening};
pub use crate::prover::{mine, mine_block, BlockContext, MineError, ProverOptions};
pub use crate::synth::synth_matrices;
pub use crate::verifier::{
    verify_at_target, verify_ncmn_at_target, verify_prod_at_target, VerifyError,
};
