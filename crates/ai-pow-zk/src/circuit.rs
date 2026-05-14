//! Plonky3 `StarkConfig` factory for the matmul puzzle.
//!
//! Pins the cryptographic stack:
//!
//! | Slot                  | Choice                            | Why |
//! |-----------------------|-----------------------------------|-----|
//! | Trace base field      | `Goldilocks` (p3-goldilocks)      | Native 64-bit prime; matches Pearl; friendly for the 32-bit ops in `p3-blake3-air`. |
//! | FRI challenge field   | `BinomialExtensionField<Goldilocks, 2>` | 128-bit security per challenge; standard pairing for Goldilocks STARKs. |
//! | FRI compression hash  | Nockchain Tip5 (`nockchain_math::tip5`) | 7-round variant; STATE_SIZE=16, RATE=10, CAPACITY=6, DIGEST_LENGTH=5. Plonky3 upstream does *not* ship a `p3-tip5` crate; the in-repo `nockchain-math::tip5` is the canonical source. |
//! | Merkle MMCS           | `MerkleTreeMmcs<Val, Tip5Perm, ...>` | Standard Plonky3 mixed-matrix commitment, wrapping the Tip5 permutation in `PaddingFreeSponge` + `TruncatedPermutation`. |
//! | PCS                   | `TwoAdicFriPcs<…>`                | Univariate FRI; matches `p3-uni-stark`. |
//! | Challenger            | `DuplexChallenger<Val, Tip5Perm, _, _>` | Fiat-Shamir over the same Tip5 permutation. |
//!
//! `CircuitConfig` is the tunable side (rate, query count, PoW bits).
//! Default values are starting points borrowed from Pearl's `prove_block`
//! and will move as the trace size differs from Pearl's STARK.

use p3_goldilocks::Goldilocks;

use crate::params::ZkParams;

/// Trace base field. Re-exported here so the AIR / public-input / witness
/// modules can spell `crate::circuit::Val` and never touch Plonky3 directly.
pub type Val = Goldilocks;

/// FRI challenge extension field. The degree-2 binomial extension is the
/// standard pairing for Goldilocks STARKs.
///
/// Type alias resolution is deferred to the implementation phase — once
/// the FRI / `p3-uni-stark` config plumbing is in place, this becomes
/// `pub type Challenge = BinomialExtensionField<Val, 2>;`. Keeping it
/// as a stub here avoids pulling in `p3_field`'s extension generics
/// before they're used in anger.
#[allow(dead_code)]
pub struct Challenge;

/// Configuration knobs for the Plonky3 STARK over the matmul AIR.
///
/// Security model: **provable** FRI soundness only. Each query gives
/// `log_blowup / 2` bits of soundness in the worst case (unique-
/// decoding radius regime). The `PROD` profile targets 120 bits
/// provable: `num_queries · log_blowup / 2 = 80 · 3 / 2 = 120`.
#[derive(Debug, Clone, Copy)]
pub struct CircuitConfig {
    /// Log2 of the FRI blowup factor. The committed evaluation domain
    /// is `2^log_blowup` times the trace length. `PROD = 3` → rate
    /// `1/8`, which gives `log_blowup / 2 = 1.5` bits of provable
    /// soundness per query in the unique-decoding regime.
    pub log_blowup: u32,
    /// FRI PoW grinding bits at the challenger. We don't use
    /// grinding; soundness comes entirely from query count and rate.
    /// Always `0` for both `PROD` and `TEST`.
    pub pow_bits: u32,
    /// Number of FRI queries. Provable soundness:
    /// `num_queries · log_blowup / 2` bits. `PROD = 80` at
    /// `log_blowup = 3` → 120 bits provable.
    pub num_queries: u32,
}

impl CircuitConfig {
    /// Production defaults. Targets **120 bits of provable FRI
    /// soundness** with no grinding (`pow_bits = 0`).
    ///
    /// Provable soundness: `num_queries · log_blowup / 2 = 80 · 3 / 2
    /// = 120` bits. We do not rely on the list-decoding / capacity-
    /// approaching conjecture; the bound here holds against any
    /// malicious prover in the standard unique-decoding regime.
    pub const PROD: Self = Self {
        log_blowup: 3,
        pow_bits: 0,
        num_queries: 80,
    };

    /// Small profile for unit tests once the circuit is real.
    /// Soundness is not the goal here — we just want a fast
    /// prove/verify round-trip.
    pub const TEST: Self = Self {
        log_blowup: 1,
        pow_bits: 0,
        num_queries: 8,
    };
}

/// Build the Plonky3 `StarkConfig` for a given `ZkParams + CircuitConfig`.
///
/// Returns the concrete type the prover and verifier both consume.
/// Construction steps (TBD — see `DESIGN.md`):
///
/// 1. Wrap `nockchain_math::tip5::permute` in a Plonky3 `Permutation`
///    impl over `[Goldilocks; 16]`. (One-time adapter at the bottom of
///    this file when wiring lands.)
/// 2. `let hash = PaddingFreeSponge::<_, 16, 10, 5>::new(tip5_perm.clone());`
///    (rate 10, capacity 6 → digest of 5 elements, matching
///    `nockchain_math::tip5::DIGEST_LENGTH`).
/// 3. `let compress = TruncatedPermutation::<_, 2, 5, 16>::new(tip5_perm.clone());`
/// 4. `let val_mmcs = MerkleTreeMmcs::<Val, _, _>::new(hash, compress);`
/// 5. `let challenge_mmcs = ExtensionMmcs::<Val, Challenge, _>::new(val_mmcs.clone());`
/// 6. `let dft = Radix2DitParallel::default();`
/// 7. `let challenger = DuplexChallenger::new(tip5_perm);`
/// 8. `let pcs = TwoAdicFriPcs::new(log_blowup, dft, val_mmcs, FriConfig { ... });`
/// 9. `let config = StarkConfig::new(pcs, challenger);`
///
/// Returning the assembled `StarkConfig`.
pub fn build_stark_config(_params: &ZkParams, _config: &CircuitConfig) {
    todo!(
        "assemble Plonky3 StarkConfig over Goldilocks: `nockchain_math::tip5` \
         sponge for hash + compress + challenger, MerkleTreeMmcs, ExtensionMmcs \
         over the degree-2 binomial extension, TwoAdicFriPcs"
    )
}

/// Thin newtype around `nockchain_math::tip5::permute` that wraps the
/// 16-element Goldilocks state so it can plug into Plonky3's
/// `CryptographicPermutation<[Val; 16]>` trait. The actual `permute`
/// call writes the new state in-place via `nockchain_math::tip5::permute`.
///
/// Adapter layer — its `Permutation`/`CryptographicPermutation` impls
/// will land alongside `build_stark_config`'s implementation. For the
/// scaffold this struct just locks in the state shape.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tip5Perm;

impl Tip5Perm {
    /// Width of the Tip5 sponge state, in field elements. Mirrors
    /// `nockchain_math::tip5::STATE_SIZE`.
    pub const WIDTH: usize = nockchain_math::tip5::STATE_SIZE;

    /// Rate (input absorption per permutation call). Mirrors
    /// `nockchain_math::tip5::RATE`.
    pub const RATE: usize = nockchain_math::tip5::RATE;

    /// Capacity (state retained across calls). Mirrors
    /// `nockchain_math::tip5::CAPACITY`.
    pub const CAPACITY: usize = nockchain_math::tip5::CAPACITY;

    /// Number of permutation rounds. Nockchain's 7-round Tip5 variant.
    pub const NUM_ROUNDS: usize = nockchain_math::tip5::NUM_ROUNDS;

    /// Apply the in-place Tip5 permutation to a 16-element state.
    /// One-line wrapper so the call site reads `Tip5Perm::permute(&mut s)`.
    pub fn permute(state: &mut [u64; Self::WIDTH]) {
        nockchain_math::tip5::permute(state);
    }
}
