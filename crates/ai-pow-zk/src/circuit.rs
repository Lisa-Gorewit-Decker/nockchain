#![allow(clippy::needless_range_loop)]
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

use p3_challenger::DuplexChallenger;
use p3_commit::ExtensionMmcs;
use p3_dft::Radix2DitParallel;
use p3_field::extension::BinomialExtensionField;
use p3_field::integers::QuotientMap;
use p3_field::{Field, PackedValue, PrimeField64};
use p3_fri::{FriParameters, TwoAdicFriPcs};
use p3_goldilocks::Goldilocks;
use p3_merkle_tree::MerkleTreeMmcs;
use p3_symmetric::{
    CryptographicPermutation, PaddingFreeSponge, Permutation, TruncatedPermutation,
};
use p3_uni_stark::StarkConfig;

use crate::params::ZkParams;

/// Trace base field. Re-exported here so the AIR / public-input / witness
/// modules can spell `crate::circuit::Val` and never touch Plonky3 directly.
pub type Val = Goldilocks;

// `Challenge` is the FRI challenge extension field — defined as a real
// type alias below, alongside the rest of the concrete STARK stack.

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

    /// Test profile for the M10.1c Pearl-style composite AIR.
    ///
    /// Pearl pins `constraint_degree = 3` (see
    /// `pearl/zk-pow/src/circuit/pearl_stark.rs:208-210`); the M10.1c
    /// composite chip set inherits that degree budget because per-chip
    /// constraints get multiplied by a `is_<chip>` boolean selector
    /// before firing. Selectors are degree 1; chip-internal constraints
    /// are degree 2; gated constraints reach degree 3.
    ///
    /// `log_blowup = 1` (the standard [`TEST`] profile) only admits
    /// degree-2 constraints (quotient degree `< 2^log_blowup = 2`).
    /// `TEST_PEARL` bumps to `log_blowup = 2` so degree-3 constraints
    /// fit while keeping tests fast.
    ///
    /// `num_queries = 16` gives ~16 bits of provable soundness — still
    /// non-cryptographic, intended for round-trip / tamper-detection
    /// tests. `PROD` (`log_blowup = 3, num_queries = 80`) handles the
    /// real 120-bit-soundness PoUW verification.
    pub const TEST_PEARL: Self = Self {
        log_blowup: 2,
        pow_bits: 0,
        num_queries: 16,
    };
}

// =====================================================================
//  Type aliases for the concrete Plonky3 STARK stack.
// =====================================================================

/// Tip5 sponge for hashing matrix rows into Merkle leaves.
///   WIDTH = 16, RATE = 10, OUT = 5.
pub type Tip5Sponge = PaddingFreeSponge<Tip5Perm, 16, 10, 5>;

/// Tip5 2-to-1 truncated permutation for internal Merkle node compression.
///   ARITY = 2, OUT = 5, WIDTH = 16.
pub type Tip5Compress = TruncatedPermutation<Tip5Perm, 2, 5, 16>;

/// MMCS over Goldilocks values. `P = PW = <Goldilocks as Field>::Packing`
/// pulls in the SIMD-packed lane type so the Merkle commit step can
/// hash multiple field elements per call. Tip5 is run lane-by-lane via
/// the unpacking adapter `impl Permutation<[PackedGl; 16]>` below.
pub type ValMmcs = MerkleTreeMmcs<
    /* P */ <Goldilocks as Field>::Packing,
    /* PW */ <Goldilocks as Field>::Packing,
    /* H */ Tip5Sponge,
    /* C */ Tip5Compress,
    /* N (arity) */ 2,
    /* DIGEST_ELEMS */ 5,
>;

/// FRI challenge field: degree-2 binomial extension of Goldilocks.
pub type Challenge = BinomialExtensionField<Goldilocks, 2>;

/// MMCS for committing extension-field polynomials (FRI codewords).
pub type ChallengeMmcs = ExtensionMmcs<Goldilocks, Challenge, ValMmcs>;

/// Fiat–Shamir challenger using the same Tip5 permutation as the MMCS.
///   WIDTH = 16, RATE = 10.
pub type Challenger = DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>;

/// DFT used by the FRI low-degree test on Goldilocks.
pub type Dft = Radix2DitParallel<Goldilocks>;

/// Univariate FRI PCS over Goldilocks.
pub type Pcs = TwoAdicFriPcs<Goldilocks, Dft, ValMmcs, ChallengeMmcs>;

/// The concrete `StarkConfig` `ai-pow-zk` uses everywhere.
pub type AiPowStarkConfig = StarkConfig<Pcs, Challenge, Challenger>;

// =====================================================================
//  Builder.
// =====================================================================

/// Assemble the Plonky3 `StarkConfig` for a given `(ZkParams, CircuitConfig)`.
///
/// The `ZkParams` is currently unused — proof shape depends only on the
/// AIR's trace width and height, both of which are computed by
/// `ai_pow_zk::prove` from the witness. The argument is kept for
/// forward-compatibility (e.g. choosing `log_final_poly_len` per matmul
/// shape later).
pub fn build_stark_config(_params: &ZkParams, config: &CircuitConfig) -> AiPowStarkConfig {
    let perm = Tip5Perm;
    let hash = Tip5Sponge::new(perm);
    let compress = Tip5Compress::new(perm);
    // `cap_height = 0` uses only the Merkle root; no cap. The cap is an
    // optimization for parallel verification, irrelevant at our trace
    // sizes.
    let val_mmcs = ValMmcs::new(hash, compress, 0);
    let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());
    let dft = Dft::default();
    let challenger = Challenger::new(perm);
    let fri_params = FriParameters {
        log_blowup: config.log_blowup as usize,
        // log_final_poly_len controls the size of the constant FRI
        // tail. 0 = single-element tail (no early stop). Bumping later
        // shrinks proofs at the cost of slightly weaker soundness per
        // query; we keep it at 0 for the strongest provable bound.
        log_final_poly_len: 0,
        max_log_arity: 1, // binary folding
        num_queries: config.num_queries as usize,
        // We intentionally hold pow_bits == 0 (DESIGN.md §7); both PoW
        // tiers in `FriParameters` come from the same knob.
        commit_proof_of_work_bits: config.pow_bits as usize,
        query_proof_of_work_bits: config.pow_bits as usize,
        mmcs: challenge_mmcs,
    };
    let pcs = Pcs::new(dft, val_mmcs, fri_params);
    StarkConfig::new(pcs, challenger)
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

// Plonky3 wires sponges and challengers through the `Permutation<T>`
// trait, where `T` is the state type. Our state type is
// `[Goldilocks; 16]`. We convert `Goldilocks → u64` via
// `PrimeField64::as_canonical_u64` and back via the `QuotientMap` impl.
// `nockchain_math::tip5::permute` then operates on the raw u64 buffer,
// reducing mod the Goldilocks prime per round constant.
impl Permutation<[Goldilocks; Tip5Perm::WIDTH]> for Tip5Perm {
    fn permute_mut(&self, input: &mut [Goldilocks; Tip5Perm::WIDTH]) {
        let mut raw: [u64; Tip5Perm::WIDTH] = [0u64; Tip5Perm::WIDTH];
        for i in 0..Tip5Perm::WIDTH {
            raw[i] = input[i].as_canonical_u64();
        }
        nockchain_math::tip5::permute(&mut raw);
        // After the permutation, each lane is < ORDER_U64. The Plonky3
        // Goldilocks impl accepts arbitrary u64s; `from_int` is the
        // canonical "reduce a u64 into the field" constructor.
        for i in 0..Tip5Perm::WIDTH {
            input[i] = <Goldilocks as QuotientMap<u64>>::from_int(raw[i]);
        }
    }
}

// Marker: we treat Tip5 as cryptographically secure for our purposes.
impl CryptographicPermutation<[Goldilocks; Tip5Perm::WIDTH]> for Tip5Perm {}

// Packed-Goldilocks variant. Plonky3's `DuplexChallenger: GrindingChallenger`
// (used inside FRI's PoW phase, even when `pow_bits = 0`) bounds the
// permutation over both scalar and packed lanes:
//
//     P: CryptographicPermutation<[F; WIDTH]>
//      + CryptographicPermutation<[<F as Field>::Packing; WIDTH]>
//
// On platforms where Goldilocks has a real SIMD-packed type (aarch64
// Neon, x86_64 AVX2/AVX-512), we add a second `Permutation` impl that
// unpacks lane-by-lane, runs scalar `nockchain_math::tip5::permute` on
// each lane, and repacks. This is functionally correct (each SIMD lane
// is an independent Goldilocks element); a real SIMD-native Tip5 would
// be faster but is out of scope.
//
// We name the concrete packed types directly (rather than going
// through `<Goldilocks as Field>::Packing`) because rustc's coherence
// checker can't disambiguate the projection from the scalar type
// across cfg variants — see the conflicting-impl error you hit if you
// try the projection route.

#[cfg(target_arch = "aarch64")]
mod packed_perm {
    use p3_goldilocks::PackedGoldilocksNeon;

    use super::*;

    impl Permutation<[PackedGoldilocksNeon; Tip5Perm::WIDTH]> for Tip5Perm {
        fn permute_mut(&self, input: &mut [PackedGoldilocksNeon; Tip5Perm::WIDTH]) {
            let lanes = <PackedGoldilocksNeon as PackedValue>::WIDTH;
            for lane in 0..lanes {
                let mut state = [0u64; Tip5Perm::WIDTH];
                for i in 0..Tip5Perm::WIDTH {
                    state[i] = input[i].as_slice()[lane].as_canonical_u64();
                }
                nockchain_math::tip5::permute(&mut state);
                for i in 0..Tip5Perm::WIDTH {
                    input[i].as_slice_mut()[lane] =
                        <Goldilocks as QuotientMap<u64>>::from_int(state[i]);
                }
            }
        }
    }

    impl CryptographicPermutation<[PackedGoldilocksNeon; Tip5Perm::WIDTH]> for Tip5Perm {}
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
mod packed_perm {
    use p3_goldilocks::PackedGoldilocksAVX512;

    use super::*;

    impl Permutation<[PackedGoldilocksAVX512; Tip5Perm::WIDTH]> for Tip5Perm {
        fn permute_mut(&self, input: &mut [PackedGoldilocksAVX512; Tip5Perm::WIDTH]) {
            let lanes = <PackedGoldilocksAVX512 as PackedValue>::WIDTH;
            for lane in 0..lanes {
                let mut state = [0u64; Tip5Perm::WIDTH];
                for i in 0..Tip5Perm::WIDTH {
                    state[i] = input[i].as_slice()[lane].as_canonical_u64();
                }
                nockchain_math::tip5::permute(&mut state);
                for i in 0..Tip5Perm::WIDTH {
                    input[i].as_slice_mut()[lane] =
                        <Goldilocks as QuotientMap<u64>>::from_int(state[i]);
                }
            }
        }
    }

    impl CryptographicPermutation<[PackedGoldilocksAVX512; Tip5Perm::WIDTH]> for Tip5Perm {}
}

#[cfg(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    not(target_feature = "avx512f")
))]
mod packed_perm {
    use p3_goldilocks::PackedGoldilocksAVX2;

    use super::*;

    impl Permutation<[PackedGoldilocksAVX2; Tip5Perm::WIDTH]> for Tip5Perm {
        fn permute_mut(&self, input: &mut [PackedGoldilocksAVX2; Tip5Perm::WIDTH]) {
            let lanes = <PackedGoldilocksAVX2 as PackedValue>::WIDTH;
            for lane in 0..lanes {
                let mut state = [0u64; Tip5Perm::WIDTH];
                for i in 0..Tip5Perm::WIDTH {
                    state[i] = input[i].as_slice()[lane].as_canonical_u64();
                }
                nockchain_math::tip5::permute(&mut state);
                for i in 0..Tip5Perm::WIDTH {
                    input[i].as_slice_mut()[lane] =
                        <Goldilocks as QuotientMap<u64>>::from_int(state[i]);
                }
            }
        }
    }

    impl CryptographicPermutation<[PackedGoldilocksAVX2; Tip5Perm::WIDTH]> for Tip5Perm {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convert a `[Goldilocks; 16]` state to `[u64; 16]` via the public
    /// canonical-u64 view, exactly the way `Tip5Perm` does internally.
    fn to_u64s(state: &[Goldilocks; Tip5Perm::WIDTH]) -> [u64; Tip5Perm::WIDTH] {
        let mut out = [0u64; Tip5Perm::WIDTH];
        for i in 0..Tip5Perm::WIDTH {
            out[i] = state[i].as_canonical_u64();
        }
        out
    }

    /// Convert a `[u64; 16]` back to `[Goldilocks; 16]` via `from_int`.
    fn from_u64s(raw: &[u64; Tip5Perm::WIDTH]) -> [Goldilocks; Tip5Perm::WIDTH] {
        let mut out = [Goldilocks::default(); Tip5Perm::WIDTH];
        for i in 0..Tip5Perm::WIDTH {
            out[i] = <Goldilocks as QuotientMap<u64>>::from_int(raw[i]);
        }
        out
    }

    #[test]
    fn tip5_perm_width_constants_match_nockchain_math() {
        assert_eq!(Tip5Perm::WIDTH, nockchain_math::tip5::STATE_SIZE);
        assert_eq!(Tip5Perm::WIDTH, 16);
        assert_eq!(Tip5Perm::RATE, nockchain_math::tip5::RATE);
        assert_eq!(Tip5Perm::RATE, 10);
        assert_eq!(Tip5Perm::CAPACITY, nockchain_math::tip5::CAPACITY);
        assert_eq!(Tip5Perm::CAPACITY, 6);
        assert_eq!(Tip5Perm::NUM_ROUNDS, nockchain_math::tip5::NUM_ROUNDS);
        assert_eq!(Tip5Perm::NUM_ROUNDS, 7);
        assert_eq!(
            Tip5Perm::WIDTH,
            Tip5Perm::RATE + Tip5Perm::CAPACITY,
            "WIDTH must equal RATE + CAPACITY"
        );
    }

    #[test]
    fn tip5_perm_static_wrapper_matches_nockchain_math() {
        // `Tip5Perm::permute(&mut s)` is just a wrapper; assert the
        // produced state byte-equals direct nockchain_math invocation.
        let mut raw_a: [u64; 16] =
            std::array::from_fn(|i| (0x1234_5678_9abc_def0u64).wrapping_mul((i as u64) + 1));
        let mut raw_b = raw_a;
        Tip5Perm::permute(&mut raw_a);
        nockchain_math::tip5::permute(&mut raw_b);
        assert_eq!(raw_a, raw_b);
    }

    #[test]
    fn tip5_perm_plonky3_permute_matches_static_wrapper() {
        // The trait-method path (used by Plonky3's sponge/challenger)
        // must produce the same final state as the static wrapper
        // applied to the corresponding u64 buffer.
        let perm = Tip5Perm;
        let initial_u64: [u64; 16] = std::array::from_fn(|i| (i as u64) * 0xdeadbeef_0badf00d);
        let initial_gl = from_u64s(&initial_u64);

        let mut via_trait = initial_gl;
        perm.permute_mut(&mut via_trait);
        let via_trait_u64 = to_u64s(&via_trait);

        let mut via_static = initial_u64;
        nockchain_math::tip5::permute(&mut via_static);
        // `from_int`'s canonicalization may not change the value modulo
        // the prime, so compare canonical forms.
        let via_static_canon: [u64; 16] = {
            let gl = from_u64s(&via_static);
            to_u64s(&gl)
        };
        assert_eq!(via_trait_u64, via_static_canon);
    }

    #[test]
    fn tip5_perm_permute_is_deterministic() {
        let perm = Tip5Perm;
        let state: [Goldilocks; 16] = from_u64s(&[7u64; 16]);
        let a = perm.permute(state);
        let b = perm.permute(state);
        assert_eq!(to_u64s(&a), to_u64s(&b));
    }

    #[test]
    fn tip5_perm_permute_is_input_sensitive() {
        // Flipping one lane changes the output non-trivially.
        let perm = Tip5Perm;
        let base: [Goldilocks; 16] = from_u64s(&[0u64; 16]);
        let mut tweaked = base;
        tweaked[3] = <Goldilocks as QuotientMap<u64>>::from_int(1);
        let out_base = to_u64s(&perm.permute(base));
        let out_tweaked = to_u64s(&perm.permute(tweaked));
        assert_ne!(out_base, out_tweaked);
        // Most lanes should change too (diffusion sanity check; not a
        // tight statistical assertion).
        let diffs = (0..16).filter(|i| out_base[*i] != out_tweaked[*i]).count();
        assert!(
            diffs >= 8,
            "expected at least 8 lanes to differ after 7-round Tip5; got {diffs}"
        );
    }

    #[test]
    fn tip5_perm_round_trip_via_clone() {
        // Plonky3's `Permutation<T>` blanket-implements `permute` from
        // `permute_mut` via `Clone`. Confirm both paths agree.
        let perm = Tip5Perm;
        let state: [Goldilocks; 16] = from_u64s(&std::array::from_fn(|i| (i as u64) * 17 + 5));
        let via_owned = perm.permute(state);
        let mut via_mut = state;
        perm.permute_mut(&mut via_mut);
        assert_eq!(to_u64s(&via_owned), to_u64s(&via_mut));
    }

    #[test]
    fn padding_free_sponge_compiles_and_hashes() {
        // Smoke test that the sponge type accepts our adapter and
        // produces a non-zero digest for a small input.
        use p3_symmetric::{CryptographicHasher, PaddingFreeSponge};
        let perm = Tip5Perm;
        let sponge: PaddingFreeSponge<Tip5Perm, 16, 10, 5> = PaddingFreeSponge::new(perm);
        let input: [Goldilocks; 7] = from_u64s(&[1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            [..7]
            .try_into()
            .unwrap();
        let digest: [Goldilocks; 5] = sponge.hash_iter(input.iter().copied());
        let digest_u64 = [
            digest[0].as_canonical_u64(),
            digest[1].as_canonical_u64(),
            digest[2].as_canonical_u64(),
            digest[3].as_canonical_u64(),
            digest[4].as_canonical_u64(),
        ];
        // Determinism.
        let digest2: [Goldilocks; 5] = sponge.hash_iter(input.iter().copied());
        let digest2_u64 = [
            digest2[0].as_canonical_u64(),
            digest2[1].as_canonical_u64(),
            digest2[2].as_canonical_u64(),
            digest2[3].as_canonical_u64(),
            digest2[4].as_canonical_u64(),
        ];
        assert_eq!(digest_u64, digest2_u64);
        // Non-trivial output (at least one lane non-zero).
        assert!(digest_u64.iter().any(|&v| v != 0));
    }

    #[test]
    fn padding_free_sponge_input_sensitive() {
        use p3_symmetric::{CryptographicHasher, PaddingFreeSponge};
        let perm = Tip5Perm;
        let sponge: PaddingFreeSponge<Tip5Perm, 16, 10, 5> = PaddingFreeSponge::new(perm);
        let a = from_u64s(&[1, 2, 3, 4, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])[..5]
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let b = from_u64s(&[1, 2, 3, 4, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])[..5]
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let da: [Goldilocks; 5] = sponge.hash_iter(a);
        let db: [Goldilocks; 5] = sponge.hash_iter(b);
        let to = |d: [Goldilocks; 5]| {
            [
                d[0].as_canonical_u64(),
                d[1].as_canonical_u64(),
                d[2].as_canonical_u64(),
                d[3].as_canonical_u64(),
                d[4].as_canonical_u64(),
            ]
        };
        assert_ne!(to(da), to(db));
    }

    fn sample_zk_params() -> ZkParams {
        ZkParams {
            m: 64,
            k: 64,
            n: 64,
            noise_rank: 32,
            tile: 8,
            difficulty_bits: 0,
        }
    }

    #[test]
    fn circuit_config_constants_are_well_formed() {
        // PROD targets 120 bits of provable FRI soundness with no
        // grinding: 80 queries × log_blowup 3 / 2 = 120.
        let prod = CircuitConfig::PROD;
        assert_eq!(prod.log_blowup, 3);
        assert_eq!(prod.num_queries, 80);
        assert_eq!(prod.pow_bits, 0);
        assert_eq!(prod.num_queries * prod.log_blowup / 2, 120);
        // TEST is just for speed; sanity checks only.
        let test = CircuitConfig::TEST;
        assert!(test.log_blowup >= 1);
        assert!(test.num_queries >= 1);
        assert_eq!(test.pow_bits, 0);
    }

    #[test]
    fn build_stark_config_prod_assembles() {
        // Construction must not panic on PROD knobs.
        let cfg = build_stark_config(&sample_zk_params(), &CircuitConfig::PROD);
        // Clone confirms the whole tree implements Clone (required by
        // `p3_uni_stark` for the prove/verify entry points).
        let _ = cfg.clone();
    }

    #[test]
    fn build_stark_config_test_assembles() {
        let cfg = build_stark_config(&sample_zk_params(), &CircuitConfig::TEST);
        let _ = cfg.clone();
    }

    /// M10.1c smoke test: TEST_PEARL profile assembles and admits a
    /// log_blowup ≥ 2 quotient budget (needed for Pearl's degree-3
    /// constraints when chip evals are gated by a degree-1 selector).
    #[test]
    fn build_stark_config_test_pearl_assembles() {
        let pearl = CircuitConfig::TEST_PEARL;
        assert_eq!(pearl.log_blowup, 2);
        assert_eq!(pearl.pow_bits, 0);
        assert!(pearl.num_queries >= 8);
        // quotient_degree ≤ 2^log_blowup, so the budget admits
        // constraint_degree − 1 = 2 → constraint_degree = 3 (matches
        // Pearl's `constraint_degree() -> 3`).
        assert!(1u32 << pearl.log_blowup >= 3 /* degree-3 quotient bound */);
        let cfg = build_stark_config(&sample_zk_params(), &pearl);
        let _ = cfg.clone();
    }

    #[test]
    fn build_stark_config_accepts_custom_knobs() {
        // The FRI params field on `TwoAdicFriPcs` is `pub(crate)` so
        // we can't read them back directly. Instead, smoke-test that
        // build_stark_config accepts a non-default CircuitConfig
        // without panicking and the resulting StarkConfig is Cloneable
        // (a requirement of p3-uni-stark's prove/verify signatures).
        let custom = CircuitConfig {
            log_blowup: 2,
            num_queries: 30,
            pow_bits: 0,
        };
        let cfg = build_stark_config(&sample_zk_params(), &custom);
        let _ = cfg.clone();
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn packed_tip5_matches_scalar_lane_by_lane_aarch64() {
        // For each SIMD lane, running Tip5 on the packed state must
        // produce the same field element as running scalar Tip5 on the
        // corresponding lane's scalar inputs.
        use p3_goldilocks::PackedGoldilocksNeon;
        type P = PackedGoldilocksNeon;
        let perm = Tip5Perm;
        let mut scalar_states: Vec<[Goldilocks; 16]> = (0..<P as PackedValue>::WIDTH)
            .map(|lane| {
                from_u64s(&std::array::from_fn(|i| {
                    (lane as u64 + 1) * 0xdeadbeef + (i as u64 * 7)
                }))
            })
            .collect();
        let mut packed_state: [P; 16] =
            std::array::from_fn(|i| P::from_fn(|lane| scalar_states[lane][i]));
        for s in scalar_states.iter_mut() {
            perm.permute_mut(s);
        }
        perm.permute_mut(&mut packed_state);
        for lane in 0..<P as PackedValue>::WIDTH {
            for i in 0..16 {
                assert_eq!(
                    packed_state[i].as_slice()[lane].as_canonical_u64(),
                    scalar_states[lane][i].as_canonical_u64(),
                    "lane {lane}, state[{i}]"
                );
            }
        }
    }

    #[test]
    fn build_stark_config_provable_soundness_at_prod() {
        // Sanity assertion of the security claim: log_blowup × num_queries / 2 = 120.
        // (We can't read FRI params back through the PCS — see
        // `build_stark_config_accepts_custom_knobs` for the rationale —
        // so verify the math directly on `CircuitConfig::PROD` which
        // is what gets fed into `build_stark_config`.)
        let prod = CircuitConfig::PROD;
        let _ = build_stark_config(&sample_zk_params(), &prod);
        let provable_bits = (prod.log_blowup * prod.num_queries / 2) as usize;
        assert_eq!(provable_bits, 120);
    }

    #[test]
    fn truncated_permutation_two_to_one_deterministic() {
        // The 2→1 compress used in MerkleTreeMmcs takes two digests
        // (each of size DIGEST), concatenates them into the first
        // 2*DIGEST lanes of the WIDTH state, permutes, and reads back
        // the first DIGEST lanes.
        use p3_symmetric::{PseudoCompressionFunction, TruncatedPermutation};
        let perm = Tip5Perm;
        let compress: TruncatedPermutation<Tip5Perm, 2, 5, 16> = TruncatedPermutation::new(perm);
        let left: [Goldilocks; 5] =
            from_u64s(&[10, 20, 30, 40, 50, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])[..5]
                .try_into()
                .unwrap();
        let right: [Goldilocks; 5] =
            from_u64s(&[60, 70, 80, 90, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])[..5]
                .try_into()
                .unwrap();
        let c1 = compress.compress([left, right]);
        let c2 = compress.compress([left, right]);
        let c1_u64: [u64; 5] = std::array::from_fn(|i| c1[i].as_canonical_u64());
        let c2_u64: [u64; 5] = std::array::from_fn(|i| c2[i].as_canonical_u64());
        assert_eq!(c1_u64, c2_u64, "compress must be deterministic");
        // Order-sensitive: swapping (left, right) must change the
        // output. The state shape is `[left | right | capacity]`, so
        // a non-trivial permutation will diffuse the swap.
        let c_swapped = compress.compress([right, left]);
        let c_swapped_u64: [u64; 5] = std::array::from_fn(|i| c_swapped[i].as_canonical_u64());
        assert_ne!(c1_u64, c_swapped_u64);
    }
}
