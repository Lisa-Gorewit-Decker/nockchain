//! M-S5b WHIR PCS feasibility prototype (2026-05-21).
//!
//! Builds a WHIR PCS configuration at parameters comparable to
//! our production FRI (Goldilocks + ext D=2, ≥80-bit Johnson) and
//! measures the serialized proof bytes head-to-head vs FRI for
//! single-polynomial commits + opens at varying sizes.
//!
//! **Substrate match:**
//! - F = Goldilocks (matches our STARK base field).
//! - EF = BinomialExtensionField<F, 2> (matches our Challenge).
//! - Hash family = Poseidon2-Goldilocks-W8 (Plonky3 stock; used by
//!   BOTH the WHIR and FRI configs in this test so the comparison
//!   isolates the PCS algorithm difference, not the hash. Production
//!   uses Tip5 5-round; per the P5 measurement Tip5 vs Poseidon2 ≈
//!   ±5% L1, so the ratio measured here transfers within ~5%).
//! - Soundness model = JohnsonBound (paper IACR ePrint 2025/2055
//!   Theorem 1.5; matches our FRI config).
//! - Both configs use Merkle cap = 0 for parity.
//!
//! **Scope (R1 honest):** measures PCS bytes for ONE polynomial
//! commitment + opening at ONE evaluation point. NOT a full STARK
//! proof. A real WHIR-based STARK would commit to multiple
//! polynomials (trace, quotient, random) and do batched openings.
//! This prototype gives us a per-polynomial baseline.

#![allow(clippy::too_many_arguments)]

use std::time::Instant;

use p3_challenger::DuplexChallenger;
use p3_commit::{ExtensionMmcs, MultilinearPcs, Pcs};
use p3_dft::Radix2DitParallel;
use p3_field::extension::BinomialExtensionField;
use p3_field::{Field, PrimeCharacteristicRing};
use p3_fri::{FriParameters, TwoAdicFriPcs};
use p3_goldilocks::{Goldilocks, Poseidon2Goldilocks};
use p3_merkle_tree::MerkleTreeMmcs;
use p3_multilinear_util::poly::Poly;
use p3_symmetric::{PaddingFreeSponge, TruncatedPermutation};
use p3_whir::fiat_shamir::domain_separator::DomainSeparator;
use p3_whir::parameters::{
    FoldingFactor, ProtocolParameters, SecurityAssumption, WhirConfig,
};
use p3_whir::pcs::prover::WhirProver;
use p3_whir::sumcheck::layout::{Layout as _, SuffixProver, Table};
use p3_whir::sumcheck::{OpeningProtocol, PointSchedule, TableShape, TableSpec};

// -----------------------------------------------------------------------------
//  Type aliases — Goldilocks + ext D=2 + Poseidon2-W8 (stock Plonky3).
// -----------------------------------------------------------------------------

type F = Goldilocks;
type EF = BinomialExtensionField<F, 2>;
type GlPacking = <F as Field>::Packing;

const W: usize = 8;
const RATE: usize = 4;
const DIGEST: usize = 4;

type Perm = Poseidon2Goldilocks<W>;
type MerkleHash = PaddingFreeSponge<Perm, W, RATE, DIGEST>;
type MerkleCompress = TruncatedPermutation<Perm, 2, DIGEST, W>;
type MyMmcs = MerkleTreeMmcs<GlPacking, GlPacking, MerkleHash, MerkleCompress, 2, DIGEST>;
type MyChallenger = DuplexChallenger<F, Perm, W, RATE>;
type ChallengeMmcs = ExtensionMmcs<F, EF, MyMmcs>;
type MyDft = Radix2DitParallel<F>;
type Layout = SuffixProver<F, EF>;
type MyWhirPcs = WhirProver<EF, F, MyDft, MyMmcs, MyChallenger, Layout>;
type MyFriPcs = TwoAdicFriPcs<F, MyDft, MyMmcs, ChallengeMmcs>;

fn make_perm() -> Perm {
    // Xoshiro256PlusPlus is exported by rand 0.10 directly (no
    // feature flag); used to avoid chacha20 transitive-dep conflict.
    use rand_010::SeedableRng;
    let mut rng = rand_010::rngs::Xoshiro256PlusPlus::seed_from_u64(1);
    Perm::new_from_rng_128(&mut rng)
}

// -----------------------------------------------------------------------------
//  WHIR PCS bytes for a single polynomial of size 2^num_variables.
// -----------------------------------------------------------------------------

fn measure_whir_bytes(
    num_variables: usize,
    security_level: usize,
    starting_log_inv_rate: usize,
    folding_factor_k: usize,
    pow_bits: usize,
) -> (usize, u128, u128) {
    let perm = make_perm();
    let merkle_hash = MerkleHash::new(perm.clone());
    let merkle_compress = MerkleCompress::new(perm.clone());
    let mmcs = MyMmcs::new(merkle_hash, merkle_compress, 0);

    let folding_factor = FoldingFactor::Constant(folding_factor_k);

    // Compute round_log_inv_rates per the example's default scheme:
    // rate += folding_factor.at_round(r) - 1 per round.
    let (num_rounds, _) = folding_factor.compute_number_of_rounds(num_variables);
    let mut rate = starting_log_inv_rate;
    let mut round_log_inv_rates = Vec::with_capacity(num_rounds);
    for round in 0..num_rounds {
        rate += folding_factor.at_round(round) - 1;
        round_log_inv_rates.push(rate);
    }

    let whir_params = ProtocolParameters {
        security_level,
        pow_bits,
        folding_factor: folding_factor.clone(),
        soundness_type: SecurityAssumption::JohnsonBound,
        starting_log_inv_rate,
        round_log_inv_rates,
    };
    let config = WhirConfig::<EF, F, MyChallenger>::new(num_variables, whir_params);

    let challenger = MyChallenger::new(perm);
    let dft = Radix2DitParallel::<F>::default();
    let pcs = MyWhirPcs::new(config, dft, mmcs);

    // Deterministic polynomial of size 2^num_variables.
    let polynomial = Poly::<F>::new((0..1u64 << num_variables).map(F::from_u64).collect());
    let table = Table::new(vec![polynomial]);
    let witness = Layout::new_witness(vec![table], folding_factor_k);

    let point_schedule: PointSchedule = (0..1).map(|_| vec![0]).collect();
    let protocol = OpeningProtocol::new(vec![TableSpec::new(
        TableShape::new(num_variables, 1),
        point_schedule,
    )])
    .pad_to_min_num_variables(folding_factor_k);
    assert_eq!(witness.table_shapes(), protocol.table_shapes());

    let mut prover_challenger = challenger.clone();
    let mut domainsep = DomainSeparator::new(vec![]);
    pcs.add_domain_separator::<8>(&mut domainsep);
    domainsep.observe_domain_separator(&mut prover_challenger);

    let t = Instant::now();
    let (commitment, prover_data) =
        <MyWhirPcs as MultilinearPcs<EF, MyChallenger>>::commit(
            &pcs,
            witness,
            &mut prover_challenger,
        );
    let _commit_us = t.elapsed().as_micros();

    let t = Instant::now();
    let proof = <MyWhirPcs as MultilinearPcs<EF, MyChallenger>>::open(
        &pcs,
        prover_data,
        protocol.clone(),
        &mut prover_challenger,
    );
    let open_ms = t.elapsed().as_millis();

    // Verify (catch bugs).
    let mut verifier_challenger = challenger;
    let mut domainsep = DomainSeparator::new(vec![]);
    pcs.add_domain_separator::<8>(&mut domainsep);
    domainsep.observe_domain_separator(&mut verifier_challenger);
    let t = Instant::now();
    <MyWhirPcs as MultilinearPcs<EF, MyChallenger>>::verify(
        &pcs,
        &commitment,
        &proof,
        &mut verifier_challenger,
        protocol,
    )
    .expect("WHIR verification failed");
    let verify_ms = t.elapsed().as_millis();

    let proof_bytes = postcard::to_allocvec(&proof).expect("ser proof").len();
    let commit_bytes = postcard::to_allocvec(&commitment).expect("ser commit").len();
    (proof_bytes + commit_bytes, open_ms, verify_ms)
}

// -----------------------------------------------------------------------------
//  FRI PCS bytes for the same polynomial size — head-to-head comparator.
// -----------------------------------------------------------------------------

fn measure_fri_bytes(
    log_height: usize,
    log_blowup: usize,
    num_queries: usize,
    log_final_poly_len: usize,
    max_log_arity: usize,
    commit_pow: usize,
    query_pow: usize,
) -> (usize, u128, u128) {
    let perm = make_perm();
    let merkle_hash = MerkleHash::new(perm.clone());
    let merkle_compress = MerkleCompress::new(perm.clone());
    let val_mmcs = MyMmcs::new(merkle_hash, merkle_compress, 0);
    let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());
    let dft = Radix2DitParallel::<F>::default();
    let fri_params = FriParameters {
        log_blowup,
        log_final_poly_len,
        max_log_arity,
        num_queries,
        commit_proof_of_work_bits: commit_pow,
        query_proof_of_work_bits: query_pow,
        mmcs: challenge_mmcs,
    };
    let pcs = MyFriPcs::new(dft, val_mmcs, fri_params);

    // Deterministic univariate polynomial of degree 2^log_height (one column).
    let domain = <MyFriPcs as Pcs<EF, MyChallenger>>::natural_domain_for_degree(&pcs, 1 << log_height);
    let evals: Vec<F> = (0..1u64 << log_height).map(F::from_u64).collect();
    let matrix = p3_matrix::dense::RowMajorMatrix::new(evals, 1);

    let t = Instant::now();
    let (commitment, prover_data) = <MyFriPcs as Pcs<EF, MyChallenger>>::commit(
        &pcs,
        vec![(domain, matrix)],
    );
    let _commit_us = t.elapsed().as_micros();

    use p3_challenger::{CanObserve, FieldChallenger};
    let mut prover_challenger = MyChallenger::new(perm);
    prover_challenger.observe(commitment.clone());
    let zeta: EF = prover_challenger.sample_algebra_element();

    let t = Instant::now();
    let (_opened_values, opening_proof) = <MyFriPcs as Pcs<EF, MyChallenger>>::open(
        &pcs,
        vec![(&prover_data, vec![vec![zeta]])],
        &mut prover_challenger,
    );
    let open_ms = t.elapsed().as_millis();

    let proof_bytes = postcard::to_allocvec(&opening_proof).expect("ser proof").len();
    let commit_bytes = postcard::to_allocvec(&commitment).expect("ser commit").len();
    (proof_bytes + commit_bytes, open_ms, 0)
}

// =============================================================================
//  Tests
// =============================================================================

/// Compile-time + small-scale smoke test for the WHIR config.
/// Runs on default `cargo test` (no `--ignored` needed).
#[test]
fn whir_prototype_compiles_and_small_smoke() {
    let num_vars = 12; // 2^12 = 4096 — small/fast.
    let (bytes, open_ms, verify_ms) = measure_whir_bytes(
        num_vars,
        80, // security_level
        4,  // starting_log_inv_rate (matches FRI lb=4)
        4,  // folding_factor_k
        2,  // pow_bits
    );
    eprintln!(
        "[WHIR smoke] num_vars={num_vars} ({} elts) → {bytes} B ({:.2} KB) | \
         open={open_ms}ms verify={verify_ms}ms",
        1usize << num_vars,
        bytes as f64 / 1024.0,
    );
    assert!(bytes > 0);
}

/// Shared body for a WHIR-vs-FRI sweep at a given soundness model.
fn sweep_whir_vs_fri(label: &str, soundness: SecurityAssumption) {
    let sizes = [12usize, 14, 16, 18, 20];

    eprintln!("\n=== {label} ===");
    eprintln!("  WHIR soundness: {soundness:?}");
    eprintln!(
        "  {:>10}  {:>14}  {:>14}  {:>8}  {:>10}  {:>10}",
        "log_size", "WHIR bytes", "FRI bytes", "ratio", "WHIR(ms)", "FRI(ms)",
    );
    eprintln!("  {}", "-".repeat(80));

    for &n in &sizes {
        let (whir_bytes, whir_open_ms, _) = measure_whir_bytes_with_soundness(
            n, 80, 4, 4, 2, soundness,
        );
        let (fri_bytes, fri_open_ms, _) = measure_fri_bytes(
            n, 4, 20, 2, 3, 1, 1,
        );
        let ratio = whir_bytes as f64 / fri_bytes as f64;
        eprintln!(
            "  {:>10}  {:>14}  {:>14}  {:>7.3}×  {:>10}  {:>10}",
            format!("2^{n}"),
            whir_bytes,
            fri_bytes,
            ratio,
            whir_open_ms,
            fri_open_ms,
        );
    }
}

/// Same as `measure_whir_bytes` but takes the soundness assumption
/// as a parameter. Existing callers go through `measure_whir_bytes`
/// which fixes JohnsonBound.
fn measure_whir_bytes_with_soundness(
    num_variables: usize,
    security_level: usize,
    starting_log_inv_rate: usize,
    folding_factor_k: usize,
    pow_bits: usize,
    soundness: SecurityAssumption,
) -> (usize, u128, u128) {
    let perm = make_perm();
    let merkle_hash = MerkleHash::new(perm.clone());
    let merkle_compress = MerkleCompress::new(perm.clone());
    let mmcs = MyMmcs::new(merkle_hash, merkle_compress, 0);

    let folding_factor = FoldingFactor::Constant(folding_factor_k);

    let (num_rounds, _) = folding_factor.compute_number_of_rounds(num_variables);
    let mut rate = starting_log_inv_rate;
    let mut round_log_inv_rates = Vec::with_capacity(num_rounds);
    for round in 0..num_rounds {
        rate += folding_factor.at_round(round) - 1;
        round_log_inv_rates.push(rate);
    }

    let whir_params = ProtocolParameters {
        security_level,
        pow_bits,
        folding_factor: folding_factor.clone(),
        soundness_type: soundness,
        starting_log_inv_rate,
        round_log_inv_rates,
    };
    let config = WhirConfig::<EF, F, MyChallenger>::new(num_variables, whir_params);

    let challenger = MyChallenger::new(perm);
    let dft = Radix2DitParallel::<F>::default();
    let pcs = MyWhirPcs::new(config, dft, mmcs);

    let polynomial = Poly::<F>::new((0..1u64 << num_variables).map(F::from_u64).collect());
    let table = Table::new(vec![polynomial]);
    let witness = Layout::new_witness(vec![table], folding_factor_k);

    let point_schedule: PointSchedule = (0..1).map(|_| vec![0]).collect();
    let protocol = OpeningProtocol::new(vec![TableSpec::new(
        TableShape::new(num_variables, 1),
        point_schedule,
    )])
    .pad_to_min_num_variables(folding_factor_k);
    assert_eq!(witness.table_shapes(), protocol.table_shapes());

    let mut prover_challenger = challenger.clone();
    let mut domainsep = DomainSeparator::new(vec![]);
    pcs.add_domain_separator::<8>(&mut domainsep);
    domainsep.observe_domain_separator(&mut prover_challenger);

    let (commitment, prover_data) =
        <MyWhirPcs as MultilinearPcs<EF, MyChallenger>>::commit(
            &pcs, witness, &mut prover_challenger,
        );

    let t = Instant::now();
    let proof = <MyWhirPcs as MultilinearPcs<EF, MyChallenger>>::open(
        &pcs, prover_data, protocol.clone(), &mut prover_challenger,
    );
    let open_ms = t.elapsed().as_millis();

    let mut verifier_challenger = challenger;
    let mut domainsep = DomainSeparator::new(vec![]);
    pcs.add_domain_separator::<8>(&mut domainsep);
    domainsep.observe_domain_separator(&mut verifier_challenger);
    let t = Instant::now();
    <MyWhirPcs as MultilinearPcs<EF, MyChallenger>>::verify(
        &pcs, &commitment, &proof, &mut verifier_challenger, protocol,
    )
    .expect("WHIR verification failed");
    let verify_ms = t.elapsed().as_millis();

    let proof_bytes = postcard::to_allocvec(&proof).expect("ser proof").len();
    let commit_bytes = postcard::to_allocvec(&commitment).expect("ser commit").len();
    (proof_bytes + commit_bytes, open_ms, verify_ms)
}

/// **M-S5b WHIR feasibility — WHIR vs FRI PCS byte comparison.**
///
/// At parameters matched to our production STARK (≥80-bit Johnson,
/// Goldilocks + ext D=2, Poseidon2-W8 hash held constant; log_blowup=4).
/// Heavy at large num_vars; `#[ignore]`'d.
#[test]
#[ignore = "M-S5b WHIR feasibility (heavy ~5-15 min at large num_vars)"]
fn whir_vs_fri_pcs_byte_comparison() {
    eprintln!("\n=== M-S5b WHIR PCS feasibility (vs FRI head-to-head) ===");
    eprintln!("Substrate: Goldilocks + BinomialExtensionField<F,2>, Poseidon2-W8 (shared)");
    eprintln!("Soundness: ≥80-bit Johnson (matches production FRI chain MIN)\n");

    let sizes = [12usize, 14, 16, 18, 20];

    eprintln!(
        "  {:>10}  {:>14}  {:>14}  {:>8}  {:>10}  {:>10}",
        "log_size", "WHIR bytes", "FRI bytes", "ratio", "WHIR(ms)", "FRI(ms)",
    );
    eprintln!("  {}", "-".repeat(80));

    for &n in &sizes {
        let (whir_bytes, whir_open_ms, _) = measure_whir_bytes(
            n,  // num_variables
            80, // security_level
            4,  // starting_log_inv_rate = log_blowup = 4
            4,  // folding_factor (vars per round)
            2,  // pow_bits
        );
        let (fri_bytes, fri_open_ms, _) = measure_fri_bytes(
            n, // log_height
            4, // log_blowup
            20, // num_queries (4*20+2 = 82 bits)
            2, // log_final_poly_len
            3, // max_log_arity
            1, // commit_pow
            1, // query_pow
        );
        let ratio = whir_bytes as f64 / fri_bytes as f64;
        eprintln!(
            "  {:>10}  {:>14}  {:>14}  {:>7.3}×  {:>10}  {:>10}",
            format!("2^{n}"),
            whir_bytes,
            fri_bytes,
            ratio,
            whir_open_ms,
            fri_open_ms,
        );
    }

    eprintln!("\n=== INTERPRETATION ===");
    eprintln!("  ratio < 1.0 → WHIR is SMALLER (paper claims 3-5×).");
    eprintln!("  ratio ≈ 1.0 → WHIR has no byte advantage at these params.");
    eprintln!("  ratio > 1.0 → WHIR is LARGER (parameter choice suboptimal).\n");
    eprintln!("Caveat: per-polynomial PCS bytes, NOT a full STARK proof.");
}

/// **WHIR at CapacityBound (stronger conjectured soundness)** vs
/// FRI at JohnsonBound (proven). The CapacityBound regime is what
/// the WHIR paper's "3-5× smaller" claim typically refers to.
///
/// **Soundness caveat:** CapacityBound is CONJECTURED, not proven.
/// Adopting it for production means relying on the conjecture
/// that proximity testing at γ < 1 − rate is sound (not just
/// γ < 1 − √rate, the Johnson bound). The IACR ePrint 2025/2055
/// Theorem 1.5 only proves up to Johnson; capacity bound is
/// folklore + heuristic.
#[test]
#[ignore = "M-S5b WHIR feasibility — CapacityBound sweep (heavy ~5-15 min)"]
fn whir_capacity_bound_vs_fri() {
    sweep_whir_vs_fri(
        "M-S5b WHIR @ CapacityBound vs FRI @ JohnsonBound",
        SecurityAssumption::CapacityBound,
    );
    eprintln!(
        "\nNOTE: WHIR uses CapacityBound (conjectured); FRI uses JohnsonBound\n\
         (proven IACR 2025/2055 Theorem 1.5). This isolates the PCS savings\n\
         WHIR can achieve IF the audit board accepts CapacityBound for it.\n",
    );
}

/// **WHIR folding-factor sweep at fixed num_vars (2^18).** The
/// folding factor `k` controls how many variables WHIR folds per
/// round. Larger `k` = fewer rounds × bigger per-round work.
///
/// The WHIR paper's smaller-proof claims often involve k≥5 or
/// k=8. This sweep measures whether our k=4 choice is suboptimal.
#[test]
#[ignore = "M-S5b WHIR feasibility — folding factor sweep (heavy ~5 min)"]
fn whir_folding_factor_sweep() {
    let num_vars = 18; // 2^18 = 262144 — production-scale.
    let folding_factors = [3usize, 4, 5, 6, 8];

    eprintln!("\n=== M-S5b WHIR folding-factor sweep at num_vars={num_vars} ===");
    eprintln!("  JohnsonBound soundness; security_level=80, pow_bits=2, log_inv_rate=4");
    eprintln!(
        "  {:>5}  {:>14}  {:>10}  {:>10}",
        "k", "WHIR bytes", "open(ms)", "verify(ms)",
    );
    eprintln!("  {}", "-".repeat(60));

    let mut best_k = 0;
    let mut best_bytes = usize::MAX;
    for &k in &folding_factors {
        let (bytes, open_ms, verify_ms) = measure_whir_bytes_with_soundness(
            num_vars, 80, 4, k, 2, SecurityAssumption::JohnsonBound,
        );
        eprintln!(
            "  {:>5}  {:>14}  {:>10}  {:>10}",
            k, bytes, open_ms, verify_ms,
        );
        if bytes < best_bytes {
            best_bytes = bytes;
            best_k = k;
        }
    }

    eprintln!(
        "\n  Best k = {best_k}; bytes = {best_bytes} ({:.2} KB)",
        best_bytes as f64 / 1024.0,
    );

    // Also sweep at CapacityBound for completeness.
    eprintln!("\n--- same sweep at CapacityBound ---");
    eprintln!(
        "  {:>5}  {:>14}  {:>10}  {:>10}",
        "k", "WHIR bytes", "open(ms)", "verify(ms)",
    );
    eprintln!("  {}", "-".repeat(60));
    let mut best_k_cap = 0;
    let mut best_bytes_cap = usize::MAX;
    for &k in &folding_factors {
        let (bytes, open_ms, verify_ms) = measure_whir_bytes_with_soundness(
            num_vars, 80, 4, k, 2, SecurityAssumption::CapacityBound,
        );
        eprintln!(
            "  {:>5}  {:>14}  {:>10}  {:>10}",
            k, bytes, open_ms, verify_ms,
        );
        if bytes < best_bytes_cap {
            best_bytes_cap = bytes;
            best_k_cap = k;
        }
    }
    eprintln!(
        "\n  Best k (CapacityBound) = {best_k_cap}; bytes = {best_bytes_cap} ({:.2} KB)",
        best_bytes_cap as f64 / 1024.0,
    );
}
