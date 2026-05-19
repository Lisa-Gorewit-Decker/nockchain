//! STARK proving configurations.
//!
//! This module provides STARK configurations for different prime fields.
//!
//! # Quick Start
//!
//! ```ignore
//! use p3_circuit_prover::config;
//!
//! // Use a preconfigured setup
//! let config = config::baby_bear();
//! ```

use p3_baby_bear::{BabyBear, Poseidon2BabyBear, default_babybear_poseidon2_16};
use p3_challenger::DuplexChallenger;
use p3_commit::ExtensionMmcs;
use p3_dft::Radix2DitParallel;
use p3_field::extension::BinomialExtensionField;
use p3_field::{Field, PrimeCharacteristicRing, PrimeField64, TwoAdicField};
use p3_fri::{FriParameters, TwoAdicFriPcs};
use p3_goldilocks::{Goldilocks, Poseidon2Goldilocks};
use p3_koala_bear::{KoalaBear, Poseidon2KoalaBear, default_koalabear_poseidon2_16};
use p3_merkle_tree::MerkleTreeMmcs;
use p3_symmetric::{CryptographicPermutation, PaddingFreeSponge, TruncatedPermutation};
use p3_uni_stark::StarkConfig;

/// Compression function arity (number of inputs per compression).
const COMPRESS_ARITY: usize = 2;

/// A STARK configuration with all cryptographic primitives specified.
///
/// ### Type Parameters
/// - `F`: Base field.
/// - `PermHash`: Permutation function used for sponge hashing (leaves, transcript absorption).
/// - `PermCompress`: Permutation function used for Merkle tree compression.
/// - `HASH_PERM_WIDTH`: Width of the hash permutation state.
/// - `COMPRESS_PERM_WIDTH`: Width of the compression permutation state.
/// - `RATE`: Number of field elements absorbed per permutation in sponge mode.
/// - `OUT`: Number of output elements squeezed per permutation.
/// - `COMPRESS_CHUNK`: Number of elements per compression chunk in Merkle commitments.
/// - `CHALLENGE_DEGREE`: Extension field degree.
pub type Config<
    F,
    PermHash,
    PermCompress,
    const HASH_PERM_WIDTH: usize,
    const COMPRESS_PERM_WIDTH: usize,
    const RATE: usize,
    const OUT: usize,
    const COMPRESS_CHUNK: usize,
    const CHALLENGE_DEGREE: usize,
> = StarkConfig<
    TwoAdicFriPcs<
        F,
        Radix2DitParallel<F>,
        MerkleTreeMmcs<
            F,
            F,
            PaddingFreeSponge<PermHash, HASH_PERM_WIDTH, RATE, OUT>,
            TruncatedPermutation<PermCompress, COMPRESS_ARITY, COMPRESS_CHUNK, COMPRESS_PERM_WIDTH>,
            2,
            COMPRESS_CHUNK,
        >,
        ExtensionMmcs<
            F,
            BinomialExtensionField<F, CHALLENGE_DEGREE>,
            MerkleTreeMmcs<
                F,
                F,
                PaddingFreeSponge<PermHash, HASH_PERM_WIDTH, RATE, OUT>,
                TruncatedPermutation<
                    PermCompress,
                    COMPRESS_ARITY,
                    COMPRESS_CHUNK,
                    COMPRESS_PERM_WIDTH,
                >,
                2,
                COMPRESS_CHUNK,
            >,
        >,
    >,
    BinomialExtensionField<F, CHALLENGE_DEGREE>,
    DuplexChallenger<F, PermHash, HASH_PERM_WIDTH, RATE>,
>;

/// Builds a STARK configuration directly from the hash and compression permutations.
///
/// This replaces the former `ConfigBuilder` (which was only ever used internally,
/// immediately followed by `.build()`): the field factories below call this and
/// return the concrete config, so callers no longer chain `.build()`.
#[allow(clippy::type_complexity)]
fn build_poseidon2_stark_config<
    F,
    PermHash,
    PermCompress,
    const HASH_PERM_WIDTH: usize,
    const COMPRESS_PERM_WIDTH: usize,
    const RATE: usize,
    const OUT: usize,
    const COMPRESS_CHUNK: usize,
    const CHALLENGE_DEGREE: usize,
>(
    perm_hash: PermHash,
    perm_compress: PermCompress,
) -> Config<
    F,
    PermHash,
    PermCompress,
    HASH_PERM_WIDTH,
    COMPRESS_PERM_WIDTH,
    RATE,
    OUT,
    COMPRESS_CHUNK,
    CHALLENGE_DEGREE,
>
where
    F: Field,
    PermHash: Clone + CryptographicPermutation<[F; HASH_PERM_WIDTH]>,
    PermCompress: Clone + CryptographicPermutation<[F; COMPRESS_PERM_WIDTH]>,
{
    type Hash<Perm, const PERM_WIDTH: usize, const RATE: usize, const OUT: usize> =
        PaddingFreeSponge<Perm, PERM_WIDTH, RATE, OUT>;
    type Compress<Perm, const PERM_WIDTH: usize, const COMPRESS_CHUNK: usize> =
        TruncatedPermutation<Perm, COMPRESS_ARITY, COMPRESS_CHUNK, PERM_WIDTH>;

    let hash = Hash::<PermHash, HASH_PERM_WIDTH, RATE, OUT>::new(perm_hash.clone());
    let compress =
        Compress::<PermCompress, COMPRESS_PERM_WIDTH, COMPRESS_CHUNK>::new(perm_compress);
    let val_mmcs = MerkleTreeMmcs::new(hash, compress, 3);
    let challenge_mmcs = ExtensionMmcs::new(val_mmcs.clone());
    let dft = Radix2DitParallel::default();
    let fri_params = FriParameters::new_benchmark_high_arity(challenge_mmcs);
    let pcs = TwoAdicFriPcs::new(dft, val_mmcs, fri_params);
    let challenger = DuplexChallenger::new(perm_hash);

    StarkConfig::new(pcs, challenger)
}

/// Creates a standard BabyBear configuration.
///
/// BabyBear is a 31-bit prime field (2^31 - 2^27 + 1).
///
/// # Parameters
/// - **Hash permutation width**: 16 (appropriate for 32-bit fields)
/// - **Compression permutation width**: 16
/// - **Rate**: 8 (256 bits / 32 bits per element)
/// - **Output size**: 8 (256 bits / 32 bits per element)
/// - **Challenge degree**: 4
///
/// # Examples
///
/// ```ignore
/// let config = config::baby_bear();
/// let prover = BatchStarkProver::new(config);
/// ```
#[inline]
pub fn baby_bear() -> BabyBearConfig {
    let perm = default_babybear_poseidon2_16();
    build_poseidon2_stark_config(perm.clone(), perm)
}

/// Creates a standard KoalaBear configuration.
///
/// KoalaBear is a 31-bit prime field (2^31 - 2^24 + 1).
///
/// # Parameters
/// - **Hash permutation width**: 16 (appropriate for 32-bit fields)
/// - **Compression permutation width**: 16
/// - **Rate**: 8 (256 bits / 32 bits per element)
/// - **Output size**: 8 (256 bits / 32 bits per element)
/// - **Challenge degree**: 4
///
/// # Examples
///
/// ```ignore
/// let config = config::koala_bear();
/// let prover = BatchStarkProver::new(config);
/// ```
#[inline]
pub fn koala_bear() -> KoalaBearConfig {
    let perm = default_koalabear_poseidon2_16();
    build_poseidon2_stark_config(perm.clone(), perm)
}

/// Creates a standard Goldilocks configuration.
///
/// Goldilocks is a 64-bit prime field (2^64 - 2^32 + 1).
///
/// # Parameters
/// - **Hash permutation width**: 8 (appropriate for 64-bit fields)
/// - **Compression permutation width**: 8
/// - **Rate**: 4 (256 bits / 64 bits per element)
/// - **Output size**: 4 (256 bits / 64 bits per element)
/// - **Challenge degree**: 2
///
/// # Examples
///
/// ```ignore
/// let config = config::goldilocks();
/// let prover = BatchStarkProver::new(config);
/// ```
#[inline]
pub fn goldilocks() -> GoldilocksConfig {
    use rand::SeedableRng;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
    let perm = p3_goldilocks::Poseidon2Goldilocks::<8>::new_from_rng_128(&mut rng);
    build_poseidon2_stark_config(perm.clone(), perm)
}

/// Goldilocks configuration with **`log_blowup = 2` (FRI tier B = 4)**.
///
/// The standard [`goldilocks`] config uses `new_benchmark_high_arity`
/// (`log_blowup = 1`, B = 2), which is sufficient for the
/// low-degree primitive / Poseidon tables but **cannot** prove the
/// **degree-4** `p3_tip5_circuit_air::Tip5PermLookupAir` constraint
/// family (the §4.6 canonical guard / x⁷ closer). This builder uses
/// `FriParameters::new_testing` (`log_blowup = 2`) — the exact FRI
/// tier the Tip5 AIR's own standalone validation suite
/// (`p3-tip5-circuit-air`'s `prove_verify_all_fixture_vectors`,
/// `make_config`) is proven sound at. Use this for any batch that
/// registers the Tip5 NPO table.
#[inline]
pub fn goldilocks_tip5() -> GoldilocksConfig {
    use rand::SeedableRng;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
    let perm = p3_goldilocks::Poseidon2Goldilocks::<8>::new_from_rng_128(&mut rng);
    let hash = PaddingFreeSponge::<_, 8, 4, 4>::new(perm.clone());
    let compress = TruncatedPermutation::<_, COMPRESS_ARITY, 4, 8>::new(perm.clone());
    let val_mmcs = MerkleTreeMmcs::new(hash, compress, 3);
    let challenge_mmcs = ExtensionMmcs::new(val_mmcs.clone());
    let dft = Radix2DitParallel::default();
    // `new_testing` ⇒ log_blowup = 2 (B = 4) — required for the
    // validated degree-4 Tip5 lookup-AIR constraints.
    let fri_params = FriParameters::new_testing(challenge_mmcs, 0);
    let pcs = TwoAdicFriPcs::new(dft, val_mmcs, fri_params);
    let challenger = DuplexChallenger::new(perm);
    StarkConfig::new(pcs, challenger)
}

/// **ADDITIVE (C3 measurement/de-risk only)** — Goldilocks Tip5
/// outer-cert config at a **≥120-bit-equivalent FRI tier**.
///
/// Byte-identical to [`goldilocks_tip5`] (same `Poseidon2Goldilocks<8>`
/// seed-1 perm, same `PaddingFreeSponge`/`TruncatedPermutation`, same
/// `MerkleTreeMmcs` cap height 3, same DFT, same `log_blowup = 2`,
/// `max_log_arity = 1`, `log_final_poly_len = 0`) **except the FRI
/// `num_queries`**: 120 (vs `new_testing`'s 2). The ethSTARK
/// conjectured soundness is `log_blowup · num_queries +
/// query_pow_bits = 2·120 + 1 = 241` bits — well past the ≥120-bit
/// bar, and matching the inner Tip5-L0 sweep's `num_queries ·
/// log_blowup / 2 == 120` convention (here `120 · 2 / 2 == 120`).
/// `commit/query_proof_of_work_bits` stay `1` (the `new_testing`
/// value `goldilocks_tip5` already uses) so the *only* lever vs the
/// ~5-bit tier is `num_queries`, for an apples-to-apples size
/// comparison. `max_log_arity` is kept at the `new_testing` value
/// (`1`, binary folding) here for the same reason; the high-arity
/// size lever is measured separately. This builder is purely
/// additive and does **not** modify [`goldilocks_tip5`].
#[inline]
pub fn goldilocks_tip5_120bit() -> GoldilocksConfig {
    use rand::SeedableRng;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
    let perm = p3_goldilocks::Poseidon2Goldilocks::<8>::new_from_rng_128(&mut rng);
    let hash = PaddingFreeSponge::<_, 8, 4, 4>::new(perm.clone());
    let compress = TruncatedPermutation::<_, COMPRESS_ARITY, 4, 8>::new(perm.clone());
    let val_mmcs = MerkleTreeMmcs::new(hash, compress, 3);
    let challenge_mmcs = ExtensionMmcs::new(val_mmcs.clone());
    let dft = Radix2DitParallel::default();
    // Same tier as `goldilocks_tip5` (`new_testing` ⇒ log_blowup = 2,
    // pow_bits = 1, max_log_arity = 1, log_final_poly_len = 0) but
    // `num_queries = 120` ⇒ conjectured soundness 2·120 + 1 = 241 bits
    // (≥120). Built explicitly (not via `new_testing`, which hardcodes
    // num_queries = 2) to raise *only* num_queries.
    let fri_params = FriParameters {
        log_blowup: 2,
        log_final_poly_len: 0,
        max_log_arity: 1,
        num_queries: 120,
        commit_proof_of_work_bits: 1,
        query_proof_of_work_bits: 1,
        mmcs: challenge_mmcs,
    };
    let pcs = TwoAdicFriPcs::new(dft, val_mmcs, fri_params);
    let challenger = DuplexChallenger::new(perm);
    StarkConfig::new(pcs, challenger)
}

/// **ADDITIVE (C3 S3 size-lever measurement only)** — Goldilocks
/// Tip5 outer-cert config at the **same ≥120-bit FRI soundness** as
/// [`goldilocks_tip5_120bit`] but with **high-arity folding**
/// (`max_log_arity = 3`) and a non-trivial final polynomial
/// (`log_final_poly_len = 2`). These two levers are
/// **soundness-neutral**: conjectured soundness depends only on
/// `log_blowup · num_queries + query_pow_bits` (unchanged at
/// `2·120 + 1 = 241` bits); higher arity / earlier FRI stop only
/// reshape the proof to fewer, fatter commit-phase steps. Purely
/// additive; does not modify [`goldilocks_tip5`] or
/// [`goldilocks_tip5_120bit`].
#[inline]
pub fn goldilocks_tip5_120bit_higharity() -> GoldilocksConfig {
    use rand::SeedableRng;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
    let perm = p3_goldilocks::Poseidon2Goldilocks::<8>::new_from_rng_128(&mut rng);
    let hash = PaddingFreeSponge::<_, 8, 4, 4>::new(perm.clone());
    let compress = TruncatedPermutation::<_, COMPRESS_ARITY, 4, 8>::new(perm.clone());
    let val_mmcs = MerkleTreeMmcs::new(hash, compress, 3);
    let challenge_mmcs = ExtensionMmcs::new(val_mmcs.clone());
    let dft = Radix2DitParallel::default();
    let fri_params = FriParameters {
        log_blowup: 2,
        log_final_poly_len: 2,
        max_log_arity: 3,
        num_queries: 120,
        commit_proof_of_work_bits: 1,
        query_proof_of_work_bits: 1,
        mmcs: challenge_mmcs,
    };
    let pcs = TwoAdicFriPcs::new(dft, val_mmcs, fri_params);
    let challenger = DuplexChallenger::new(perm);
    StarkConfig::new(pcs, challenger)
}

/// Type alias for BabyBear STARK configuration.
pub type BabyBearConfig =
    Config<BabyBear, Poseidon2BabyBear<16>, Poseidon2BabyBear<16>, 16, 16, 8, 8, 8, 4>;

/// Type alias for KoalaBear STARK configuration.
pub type KoalaBearConfig =
    Config<KoalaBear, Poseidon2KoalaBear<16>, Poseidon2KoalaBear<16>, 16, 16, 8, 8, 8, 4>;

/// Type alias for Goldilocks STARK configuration.
pub type GoldilocksConfig =
    Config<Goldilocks, Poseidon2Goldilocks<8>, Poseidon2Goldilocks<8>, 8, 8, 4, 4, 4, 2>;

/// Trait bounds for STARK-compatible fields.
pub trait StarkField: Field + PrimeCharacteristicRing + TwoAdicField + PrimeField64 {}

impl<F> StarkField for F where F: Field + PrimeCharacteristicRing + TwoAdicField + PrimeField64 {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fields_configs_compile() {
        let _bb: BabyBearConfig = baby_bear();
        let _kb: KoalaBearConfig = koala_bear();
        let _gl: GoldilocksConfig = goldilocks();
    }
}
