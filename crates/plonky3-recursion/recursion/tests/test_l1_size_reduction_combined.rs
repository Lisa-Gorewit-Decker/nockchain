//! M-S5b S1.B follow-on — Combined-optimization sweep for L1 size
//! reduction (focused subset; ~3 min runtime).
//!
//! Builds on the broader sweep in `test_l1_size_reduction_sweep.rs`
//! with the Pareto-optimal combinations identified from the initial
//! data:
//!
//!   - Baseline:       lb=2 nq=42 pow=1+1, digest=5 → 1010.7 KB [85 bits]
//!   - Lever 3:        lb=3 nq=27 pow=1+1 → 694.8 KB [83 bits] (-31%)
//!   - Lever 4:        lb=4 nq=20 pow=1+1 → 547.9 KB [82 bits] (-46%)
//!   - Lever 1:        digest=4 → -5% from any baseline
//!
//! This test measures the remaining variants:
//!   - Lever 5: mla=3 lfp=2 (FRI high-arity folding)
//!   - Lever 6: cap=3 (MMCS cap height)
//!   - Combined: lb=4 nq=20 + digest=4 (combine biggest wins)
//!   - Combined: lb=4 nq=20 + digest=4 + mla=3 lfp=2 (add high-arity)
//!   - Combined: lb=4 nq=20 + digest=4 + mla=3 lfp=2 + cap=3 (Pareto)

mod common;

use p3_batch_stark::ProverData;
use p3_challenger::DuplexChallenger;
use p3_circuit::CircuitBuilder;
use p3_circuit::ops::{
    Tip5Config, Tip5Goldilocks, generate_recompose_trace, generate_tip5_trace,
};
use p3_circuit::test_utils::{FibonacciAir, generate_trace_rows};
use p3_circuit_prover::batch_stark_prover::{recompose_air_builders, tip5_air_builders};
use p3_circuit_prover::common::{NpoPreprocessor, get_airs_and_degrees_with_prep};
use p3_circuit_prover::{
    BatchStarkProver, CircuitProverData, ConstraintProfile, RecomposePreprocessor, TablePacking,
    Tip5Preprocessor,
};
use p3_commit::ExtensionMmcs;
use p3_dft::Radix2DitParallel;
use p3_field::extension::BinomialExtensionField;
use p3_field::{BasedVectorSpace, PrimeCharacteristicRing};
use p3_fri::{FriParameters, TwoAdicFriPcs};
use p3_goldilocks::Goldilocks;
use p3_merkle_tree::MerkleTreeMmcs;
use p3_recursion::pcs::fri::{FriVerifierParams, InputProofTargets, MerkleCapTargets, RecValMmcs};
use p3_recursion::pcs::set_fri_mmcs_private_data;
use p3_recursion::public_inputs::StarkVerifierInputsBuilder;
use p3_recursion::verify_p3_uni_proof_circuit;
use p3_symmetric::{PaddingFreeSponge, Permutation, TruncatedPermutation};
use p3_tip5_circuit_air::Tip5Perm;
use p3_uni_stark::{StarkConfig, prove, verify};

use crate::common::InnerFriGeneric;

type Val = Goldilocks;
type GlPacking = <Goldilocks as p3_field::Field>::Packing;
type Challenge = BinomialExtensionField<Goldilocks, 2>;
type Tip5Sponge = PaddingFreeSponge<Tip5Perm, 16, 10, 5>;
type Tip5Compress = TruncatedPermutation<Tip5Perm, 2, 5, 16>;
type ValMmcs = MerkleTreeMmcs<GlPacking, GlPacking, Tip5Sponge, Tip5Compress, 2, 5>;
type ChallengeMmcs = ExtensionMmcs<Goldilocks, Challenge, ValMmcs>;
type Layer0Challenger = DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>;
type Dft = Radix2DitParallel<Goldilocks>;
type Pcs = TwoAdicFriPcs<Goldilocks, Dft, ValMmcs, ChallengeMmcs>;
type Tip5Layer0Config = StarkConfig<Pcs, Challenge, Layer0Challenger>;

const DIGEST_ELEMS: usize = 5;
const WIDTH: usize = 16;
const RATE: usize = 10;

type InnerFri = InnerFriGeneric<Tip5Layer0Config, Tip5Sponge, Tip5Compress, DIGEST_ELEMS>;

#[derive(Clone, Copy, Debug, Default)]
struct LiftTip5;
impl Permutation<[Challenge; 16]> for LiftTip5 {
    fn permute(&self, input: [Challenge; 16]) -> [Challenge; 16] {
        let bases: [Goldilocks; 16] = core::array::from_fn(|i| {
            <Challenge as BasedVectorSpace<Goldilocks>>::as_basis_coefficients_slice(&input[i])[0]
        });
        let out = Tip5Perm.permute(bases);
        core::array::from_fn(|i| {
            <Challenge as BasedVectorSpace<Goldilocks>>::from_basis_coefficients_fn(|j| {
                if j == 0 { out[i] } else { Goldilocks::ZERO }
            })
        })
    }
    fn permute_mut(&self, input: &mut [Challenge; 16]) {
        *input = Permutation::permute(self, *input);
    }
}

fn make_layer0_config() -> Tip5Layer0Config {
    let perm = Tip5Perm;
    let hash = Tip5Sponge::new(perm);
    let compress = Tip5Compress::new(perm);
    let val_mmcs = ValMmcs::new(hash, compress, 0);
    let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());
    let dft = Dft::default();
    let challenger = Layer0Challenger::new(perm);
    let fri_params = FriParameters {
        log_blowup: 3, log_final_poly_len: 0, max_log_arity: 1,
        num_queries: 30, commit_proof_of_work_bits: 0, query_proof_of_work_bits: 0,
        mmcs: challenge_mmcs,
    };
    let pcs = Pcs::new(dft, val_mmcs, fri_params);
    StarkConfig::new(pcs, challenger)
}

struct BuiltLayer0Circuit {
    circuit: p3_circuit::Circuit<Challenge>,
    public_inputs: Vec<Challenge>,
    private_inputs: Vec<Challenge>,
    mmcs_op_ids: Vec<p3_circuit::NonPrimitiveOpId>,
    proof: p3_uni_stark::Proof<Tip5Layer0Config>,
}

fn build_layer0_verifier_circuit() -> BuiltLayer0Circuit {
    let config = make_layer0_config();
    let n = 1 << 3;
    let x = 21u64;
    let trace = generate_trace_rows::<Val>(0, 1, n);
    let pis = vec![Val::ZERO, Val::ONE, Val::from_u64(x)];
    let air = FibonacciAir {};

    let proof = prove(&config, &air, trace, &pis);
    assert!(verify(&config, &air, &proof, &pis).is_ok());

    let mut circuit_builder = CircuitBuilder::<Challenge>::new();
    circuit_builder.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<Challenge, Tip5Goldilocks>,
        LiftTip5,
    );
    circuit_builder.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
    circuit_builder.set_recompose_coeff_ctl_for_decompose_links(true);

    let fri_verifier_params =
        FriVerifierParams::with_mmcs(3, 0, 0, 0, Tip5Config::GOLDILOCKS_W16);

    let verifier_inputs = StarkVerifierInputsBuilder::<
        Tip5Layer0Config, MerkleCapTargets<Val, DIGEST_ELEMS>, InnerFri,
    >::allocate(&mut circuit_builder, &proof, None, pis.len());

    let mmcs_op_ids = verify_p3_uni_proof_circuit::<
        FibonacciAir, Tip5Layer0Config, MerkleCapTargets<Val, DIGEST_ELEMS>,
        InputProofTargets<Val, Challenge, RecValMmcs<Val, DIGEST_ELEMS, Tip5Sponge, Tip5Compress>>,
        InnerFri, Tip5Config, WIDTH, RATE,
    >(
        &make_layer0_config(), &FibonacciAir {},
        &mut circuit_builder, &verifier_inputs.proof_targets,
        &verifier_inputs.air_public_targets, &verifier_inputs.preprocessed_commit,
        &fri_verifier_params, Tip5Config::GOLDILOCKS_W16,
    ).expect("L0 verifier circuit");

    let circuit = circuit_builder.build().expect("circuit build");
    let public_inputs = verifier_inputs.pack_public_values(&pis, &proof, &None);
    let private_inputs = verifier_inputs.pack_private_values(&proof);

    BuiltLayer0Circuit { circuit, public_inputs, private_inputs, mmcs_op_ids, proof }
}

#[derive(Clone, Copy, Debug)]
struct FriKnobs {
    log_blowup: usize, log_final_poly_len: usize, max_log_arity: usize,
    num_queries: usize, commit_pow: usize, query_pow: usize,
}

impl FriKnobs {
    fn bits(&self) -> usize {
        self.log_blowup * self.num_queries + self.commit_pow + self.query_pow
    }
}

// Tip5 d=5 builder (production digest size)
mod tip5_d5 {
    use super::*;
    use p3_test_utils::goldilocks_tip5_params::{
        Challenger as TipsChallenger, ChallengeMmcs as TipsChallengeMmcs, Dft as TipsDft,
        MyConfig as TipsCfg, MyCompress as TipsCompress, MyHash as TipsHash,
        MyMmcs as TipsValMmcs, MyPcs as TipsPcs,
    };
    pub fn build_l1(knobs: FriKnobs, cap: usize, label: &str) -> Result<usize, String> {
        let perm = Tip5Perm;
        let hash = TipsHash::new(perm);
        let compress = TipsCompress::new(perm);
        let val_mmcs = TipsValMmcs::new(hash, compress, cap);
        let challenge_mmcs = TipsChallengeMmcs::new(val_mmcs.clone());
        let dft = TipsDft::default();
        let fri_params = FriParameters {
            log_blowup: knobs.log_blowup, log_final_poly_len: knobs.log_final_poly_len,
            max_log_arity: knobs.max_log_arity, num_queries: knobs.num_queries,
            commit_proof_of_work_bits: knobs.commit_pow,
            query_proof_of_work_bits: knobs.query_pow,
            mmcs: challenge_mmcs,
        };
        let pcs = TipsPcs::new(dft, val_mmcs, fri_params);
        let outer_cfg: TipsCfg = TipsCfg::new(pcs, TipsChallenger::new(perm));

        let BuiltLayer0Circuit { circuit, public_inputs, private_inputs, mmcs_op_ids, proof } =
            build_layer0_verifier_circuit();
        let table_packing = TablePacking::new(1, 8);
        let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> = vec![
            Box::new(Tip5Preprocessor), Box::new(RecomposePreprocessor::new(true)),
        ];
        let mut air_builders = tip5_air_builders::<TipsCfg, 2>();
        air_builders.extend(recompose_air_builders::<TipsCfg, 2>(1, true));

        let (airs_degrees, primitive_columns, non_primitive_columns) =
            get_airs_and_degrees_with_prep::<TipsCfg, Challenge, 2>(
                &circuit, &table_packing, &npo_prep, &air_builders,
                ConstraintProfile::Standard,
            ).map_err(|e| format!("[{label}] get_airs: {e:?}"))?;
        let airs_degrees_vec: Vec<_> = airs_degrees.into_iter().collect();
        let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees_vec.into_iter().unzip();

        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        runner.set_private_inputs(&private_inputs).unwrap();
        set_fri_mmcs_private_data::<
            Val, Challenge, ChallengeMmcs, ValMmcs, Tip5Sponge, Tip5Compress, DIGEST_ELEMS,
        >(&mut runner, &mmcs_op_ids, &proof.opening_proof, Tip5Config::GOLDILOCKS_W16)
        .map_err(|e| format!("[{label}] set_fri: {e}"))?;

        let traces = runner.run().map_err(|e| format!("[{label}] runner: {e:?}"))?;
        let prover_data = ProverData::from_airs_and_degrees(&outer_cfg, &airs, &degrees);
        let circuit_prover_data =
            CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);
        let mut prover = BatchStarkProver::new(outer_cfg).with_table_packing(table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);
        let bp = prover.prove_all_tables(&traces, &circuit_prover_data)
            .map_err(|e| format!("[{label}] prove: {e:?}"))?;
        prover.verify_all_tables(&bp).map_err(|e| format!("[{label}] verify: {e:?}"))?;
        Ok(postcard::to_allocvec(&bp).expect("postcard").len())
    }
}

mod tip5_d4 {
    use super::*;
    use p3_test_utils::goldilocks_tip5_out4_params::{
        Challenger as O4Challenger, ChallengeMmcs as O4ChallengeMmcs, Dft as O4Dft,
        MyConfig as O4Cfg, MyCompress as O4Compress, MyHash as O4Hash,
        MyMmcs as O4ValMmcs, MyPcs as O4Pcs,
    };
    pub fn build_l1(knobs: FriKnobs, cap: usize, label: &str) -> Result<usize, String> {
        let perm = Tip5Perm;
        let hash = O4Hash::new(perm);
        let compress = O4Compress::new(perm);
        let val_mmcs = O4ValMmcs::new(hash, compress, cap);
        let challenge_mmcs = O4ChallengeMmcs::new(val_mmcs.clone());
        let dft = O4Dft::default();
        let fri_params = FriParameters {
            log_blowup: knobs.log_blowup, log_final_poly_len: knobs.log_final_poly_len,
            max_log_arity: knobs.max_log_arity, num_queries: knobs.num_queries,
            commit_proof_of_work_bits: knobs.commit_pow,
            query_proof_of_work_bits: knobs.query_pow,
            mmcs: challenge_mmcs,
        };
        let pcs = O4Pcs::new(dft, val_mmcs, fri_params);
        let outer_cfg: O4Cfg = O4Cfg::new(pcs, O4Challenger::new(perm));

        let BuiltLayer0Circuit { circuit, public_inputs, private_inputs, mmcs_op_ids, proof } =
            build_layer0_verifier_circuit();
        let table_packing = TablePacking::new(1, 8);
        let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> = vec![
            Box::new(Tip5Preprocessor), Box::new(RecomposePreprocessor::new(true)),
        ];
        let mut air_builders = tip5_air_builders::<O4Cfg, 2>();
        air_builders.extend(recompose_air_builders::<O4Cfg, 2>(1, true));

        let (airs_degrees, primitive_columns, non_primitive_columns) =
            get_airs_and_degrees_with_prep::<O4Cfg, Challenge, 2>(
                &circuit, &table_packing, &npo_prep, &air_builders,
                ConstraintProfile::Standard,
            ).map_err(|e| format!("[{label}] get_airs: {e:?}"))?;
        let airs_degrees_vec: Vec<_> = airs_degrees.into_iter().collect();
        let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees_vec.into_iter().unzip();

        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        runner.set_private_inputs(&private_inputs).unwrap();
        set_fri_mmcs_private_data::<
            Val, Challenge, ChallengeMmcs, ValMmcs, Tip5Sponge, Tip5Compress, DIGEST_ELEMS,
        >(&mut runner, &mmcs_op_ids, &proof.opening_proof, Tip5Config::GOLDILOCKS_W16)
        .map_err(|e| format!("[{label}] set_fri: {e}"))?;

        let traces = runner.run().map_err(|e| format!("[{label}] runner: {e:?}"))?;
        let prover_data = ProverData::from_airs_and_degrees(&outer_cfg, &airs, &degrees);
        let circuit_prover_data =
            CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);
        let mut prover = BatchStarkProver::new(outer_cfg).with_table_packing(table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);
        let bp = prover.prove_all_tables(&traces, &circuit_prover_data)
            .map_err(|e| format!("[{label}] prove: {e:?}"))?;
        prover.verify_all_tables(&bp).map_err(|e| format!("[{label}] verify: {e:?}"))?;
        Ok(postcard::to_allocvec(&bp).expect("postcard").len())
    }
}

fn report(label: &str, knobs: FriKnobs, cap: usize, d4: bool) {
    let bits = knobs.bits();
    let full = format!(
        "{label} (lb={} nq={} pow={}+{} mla={} lfp={} cap={} d={}) [{bits}b]",
        knobs.log_blowup, knobs.num_queries, knobs.commit_pow, knobs.query_pow,
        knobs.max_log_arity, knobs.log_final_poly_len, cap, if d4 { 4 } else { 5 },
    );
    let result = if d4 { tip5_d4::build_l1(knobs, cap, &full) }
                 else { tip5_d5::build_l1(knobs, cap, &full) };
    match result {
        Ok(s) => eprintln!("{:>62} : {:>7} B ({:>6.1} KB)", full, s, s as f64 / 1024.0),
        Err(e) => eprintln!("{:>62} : FAILED — {e}", full),
    }
}

#[test]
#[ignore = "M-S5b combined-sweep — ~3 min; manual"]
fn l1_combined_optimization_sweep() {
    eprintln!("");
    eprintln!("=== M-S5b S1.B Combined-Optimization Sweep (focused) ===");
    eprintln!("Baseline (production): lb=2 nq=42 pow=1+1 d=5 → ~1011 KB [85b]");
    eprintln!("--------------------------------------------------------------");

    let base = FriKnobs {
        log_blowup: 2, log_final_poly_len: 0, max_log_arity: 1,
        num_queries: 42, commit_pow: 1, query_pow: 1,
    };

    // Levers 5 + 6 (not in first sweep)
    eprintln!("");
    eprintln!("Lever 5: high-arity FRI folding (mla=3 lfp=2; same FRI bits)");
    report("mla=3 lfp=2", FriKnobs { max_log_arity: 3, log_final_poly_len: 2, .. base }, 0, false);

    eprintln!("");
    eprintln!("Lever 6: MMCS cap=3 (shorter Merkle paths)");
    report("cap=3", base, 3, false);

    // Pareto combinations
    eprintln!("");
    eprintln!("=== Pareto combinations (stacked levers) ===");
    report("d4 + lb=3 nq=27", FriKnobs { log_blowup: 3, num_queries: 27, .. base }, 0, true);
    report("d4 + lb=4 nq=20", FriKnobs { log_blowup: 4, num_queries: 20, .. base }, 0, true);
    report("d4 + lb=4 nq=20 + mla=3 lfp=2",
           FriKnobs { log_blowup: 4, num_queries: 20, max_log_arity: 3, log_final_poly_len: 2, .. base },
           0, true);
    report("d4 + lb=4 nq=20 + mla=3 lfp=2 + cap=3",
           FriKnobs { log_blowup: 4, num_queries: 20, max_log_arity: 3, log_final_poly_len: 2, .. base },
           3, true);

    eprintln!("");
    eprintln!("=== Aggressive 80-bit floor combinations ===");
    // lb=2 nq=40 pow=0+0 = 80 bits
    report("d4 + nq=40 pow=0+0",
           FriKnobs { num_queries: 40, commit_pow: 0, query_pow: 0, .. base }, 0, true);
    // lb=4 nq=20 pow=0+0 = 80 bits exactly
    report("d4 + lb=4 nq=20 pow=0+0",
           FriKnobs { log_blowup: 4, num_queries: 20, commit_pow: 0, query_pow: 0, .. base },
           0, true);
    report("d4 + lb=4 nq=20 pow=0+0 + mla=3 lfp=2 + cap=3",
           FriKnobs { log_blowup: 4, num_queries: 20, commit_pow: 0, query_pow: 0,
                      max_log_arity: 3, log_final_poly_len: 2, .. base }, 3, true);
}
