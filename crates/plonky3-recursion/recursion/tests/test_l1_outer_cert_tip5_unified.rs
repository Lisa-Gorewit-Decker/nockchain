//! M-S5b S1.B Poseidon2-removal P4 — empirical L1 outer-cert size
//! measurement at the Tip5-unified config vs the Poseidon2-based
//! baseline.
//!
//! This test builds the full L1 outer-cert at BOTH the existing
//! Poseidon2-W8 outer config AND the new Tip5-unified outer config,
//! then compares serialized proof sizes via `postcard::to_allocvec`.
//!
//! **Spec target** (per `crates/ai-pow-zk/docs/2026-05-20_POSEIDON2_REMOVAL_SPEC.md`
//! §4.5 P4): the Tip5-unified L1 should be ≥ 15 KB smaller than the
//! Poseidon2-based L1.
//!
//! **Critical-path note:** this test exercises the predicted C2.4
//! R-a tail at D=2 in the FULL Layer-0 verifier circuit. The P3
//! sub-component test passed; this P4 test confirms the full chain.
//!
//! Marked `#[ignore]` because the full L1 build is heavy for default
//! CI. Run manually:
//! ```text
//! cargo test -p p3-recursion --release --test test_l1_outer_cert_tip5_unified -- --ignored --nocapture
//! ```

mod common;

use p3_batch_stark::ProverData;
use p3_challenger::DuplexChallenger;
use p3_circuit::CircuitBuilder;
use p3_circuit::ops::{Tip5Config, Tip5Goldilocks, generate_recompose_trace, generate_tip5_trace};
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
use p3_recursion::terminal::{
    NativeTerminalCompiler, TerminalCircuitFingerprint, TerminalProofParameters, TerminalWitness,
};
use p3_recursion::verify_p3_uni_proof_circuit;
use p3_symmetric::{PaddingFreeSponge, Permutation, TruncatedPermutation};
use p3_tip5_circuit_air::Tip5Perm;
use p3_uni_stark::{StarkConfig, prove, verify};

use crate::common::InnerFriGeneric;

// ---------------------------------------------------------------------
//  Inner L0 — Tip5 Layer-0 (UNCHANGED)
// ---------------------------------------------------------------------

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
    // Inner Tip5-L0 STARK FRI matches ai-pow-zk::CircuitConfig::PROD:
    // lb=4 nq=15 pow=1+1 = 62 bits Johnson. The production outer
    // certificate is separately parameterized by goldilocks_tip5_60bit().
    let fri_params = FriParameters {
        log_blowup: 4,
        log_final_poly_len: 0,
        max_log_arity: 1,
        num_queries: 15,
        commit_proof_of_work_bits: 1,
        query_proof_of_work_bits: 1,
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

    let fri_verifier_params = FriVerifierParams::with_mmcs(4, 0, 1, 1, Tip5Config::GOLDILOCKS_W16);

    let verifier_inputs = StarkVerifierInputsBuilder::<
        Tip5Layer0Config,
        MerkleCapTargets<Val, DIGEST_ELEMS>,
        InnerFri,
    >::allocate(&mut circuit_builder, &proof, None, pis.len());

    let mmcs_op_ids = verify_p3_uni_proof_circuit::<
        FibonacciAir,
        Tip5Layer0Config,
        MerkleCapTargets<Val, DIGEST_ELEMS>,
        InputProofTargets<Val, Challenge, RecValMmcs<Val, DIGEST_ELEMS, Tip5Sponge, Tip5Compress>>,
        InnerFri,
        Tip5Config,
        WIDTH,
        RATE,
    >(
        &make_layer0_config(),
        &FibonacciAir {},
        &mut circuit_builder,
        &verifier_inputs.proof_targets,
        &verifier_inputs.air_public_targets,
        &verifier_inputs.preprocessed_commit,
        &fri_verifier_params,
        Tip5Config::GOLDILOCKS_W16,
    )
    .expect("L0 verifier circuit construction");

    let circuit = circuit_builder.build().expect("circuit build");
    let public_inputs = verifier_inputs.pack_public_values(&pis, &proof, &None);
    let private_inputs = verifier_inputs.pack_private_values(&proof);

    BuiltLayer0Circuit {
        circuit,
        public_inputs,
        private_inputs,
        mmcs_op_ids,
        proof,
    }
}

#[test]
fn terminal_compiler_covers_real_tip5_l0_verifier_circuit() {
    let BuiltLayer0Circuit {
        circuit,
        public_inputs,
        private_inputs,
        mmcs_op_ids,
        proof,
    } = build_layer0_verifier_circuit();

    let mut runner = circuit.runner();
    runner.set_public_inputs(&public_inputs).unwrap();
    runner.set_private_inputs(&private_inputs).unwrap();
    set_fri_mmcs_private_data::<
        Val,
        Challenge,
        ChallengeMmcs,
        ValMmcs,
        Tip5Sponge,
        Tip5Compress,
        DIGEST_ELEMS,
    >(
        &mut runner,
        &mmcs_op_ids,
        &proof.opening_proof,
        Tip5Config::GOLDILOCKS_W16,
    )
    .expect("set_fri_mmcs_private_data");

    let traces = runner.run().expect("real L0 verifier circuit must execute");
    let terminal_witness = TerminalWitness {
        fingerprint: TerminalCircuitFingerprint::from_circuit(&circuit),
        public_inputs,
        private_inputs,
        traces,
    };

    let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
    let (_pk, vk) = compiler
        .compile_goldilocks_terminal(&circuit)
        .expect("terminal compiler must cover real Tip5 L0 verifier circuit");
    assert!(
        vk.header.relation_digest.is_some(),
        "real terminal verifying key must bind the compiled relation digest"
    );

    assert!(
        vk.inventory
            .non_primitive_types
            .iter()
            .any(|ty| ty == "tip5_perm/goldilocks_w16_r5")
    );
    assert!(
        vk.inventory
            .non_primitive_types
            .iter()
            .any(|ty| ty == "recompose")
    );
    assert!(
        vk.inventory
            .non_primitive_types
            .iter()
            .any(|ty| ty == "recompose/coeff")
    );
    let profile = vk.relation_profile();
    eprintln!(
        "terminal relation profile: witnesses={} public={} private={} ops={} primitive_constraints={} terminal_constraints={} hints={} non_primitive_ops={} tip5_rows={} recompose_rows={} recompose_coeff_rows={} npo_input_slots={} npo_output_slots={}",
        profile.fingerprint.witness_count,
        profile.fingerprint.public_flat_len,
        profile.fingerprint.private_flat_len,
        profile.fingerprint.ops_len,
        profile.primitive_constraints,
        profile.terminal_constraints,
        profile.hint_ops,
        profile.non_primitive_ops,
        profile.tip5_rows,
        profile.recompose_rows,
        profile.recompose_coeff_rows,
        profile.npo_callsite_input_slots,
        profile.npo_callsite_output_slots,
    );
    assert_eq!(
        profile.fingerprint,
        TerminalCircuitFingerprint::from_circuit(&circuit)
    );
    assert_eq!(profile.non_primitive_rows(), profile.non_primitive_ops);
    assert_eq!(profile.terminal_constraints, vk.constraints.len());
    assert_eq!(
        profile.primitive_constraints,
        vk.inventory.total_primitive_ops()
    );
    assert_eq!(profile.fingerprint.witness_count, 3043);
    assert_eq!(profile.fingerprint.public_flat_len, 33);
    assert_eq!(profile.fingerprint.private_flat_len, 156);
    assert_eq!(profile.fingerprint.ops_len, 2620);
    assert_eq!(profile.primitive_constraints, 1881);
    assert_eq!(profile.terminal_constraints, 1884);
    assert_eq!(profile.hint_ops, 71);
    assert_eq!(profile.non_primitive_ops, 668);
    assert_eq!(profile.tip5_rows, 520);
    assert_eq!(profile.recompose_rows, 51);
    assert_eq!(profile.recompose_coeff_rows, 97);
    assert_eq!(profile.npo_callsite_input_slots, 8616);
    assert_eq!(profile.npo_callsite_output_slots, 5348);

    compiler
        .verify_assignment_with_goldilocks_npos(&vk, &terminal_witness)
        .expect("terminal relation must verify the real Tip5 L0 verifier witness");

    let npo_relation = vk.npo_relation();
    assert_eq!(npo_relation.rows.len(), profile.non_primitive_rows());
    assert_eq!(npo_relation.tip5_rows(), profile.tip5_rows);
    assert_eq!(npo_relation.recompose_rows(), profile.recompose_rows);
    assert_eq!(
        npo_relation.recompose_coeff_rows(),
        profile.recompose_coeff_rows
    );
    compiler
        .verify_npo_relation_goldilocks(&vk, &npo_relation, &terminal_witness)
        .expect("real Tip5 L0 verifier witness must satisfy projected NPO relation");

    let quadratic_relation = vk
        .primitive_quadratic_relation()
        .expect("real L1 primitive terminal constraints must lower to quadratic relation");
    assert_eq!(
        quadratic_relation.constraints.len(),
        profile.primitive_constraints + vk.inventory.alu_bool_check_ops
    );
    assert_eq!(
        quadratic_relation.external_npo_rows,
        profile.non_primitive_rows()
    );
    quadratic_relation
        .verify(&terminal_witness.public_inputs, &terminal_witness)
        .expect("real Tip5 L0 verifier witness must satisfy primitive quadratic relation");
}

#[test]
fn terminal_local_certificate_measures_real_tip5_l0_verifier_circuit() {
    let BuiltLayer0Circuit {
        circuit,
        public_inputs,
        private_inputs,
        mmcs_op_ids,
        proof,
    } = build_layer0_verifier_circuit();

    let mut runner = circuit.runner();
    runner.set_public_inputs(&public_inputs).unwrap();
    runner.set_private_inputs(&private_inputs).unwrap();
    set_fri_mmcs_private_data::<
        Val,
        Challenge,
        ChallengeMmcs,
        ValMmcs,
        Tip5Sponge,
        Tip5Compress,
        DIGEST_ELEMS,
    >(
        &mut runner,
        &mmcs_op_ids,
        &proof.opening_proof,
        Tip5Config::GOLDILOCKS_W16,
    )
    .expect("set_fri_mmcs_private_data");

    let traces = runner.run().expect("real L0 verifier circuit must execute");
    let terminal_witness = TerminalWitness {
        fingerprint: TerminalCircuitFingerprint::from_circuit(&circuit),
        public_inputs,
        private_inputs,
        traces,
    };

    let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
    let (_pk, vk) = compiler
        .compile_goldilocks_terminal(&circuit)
        .expect("terminal compiler must cover real Tip5 L0 verifier circuit");
    let parameters = TerminalProofParameters::production_60bit();
    compiler
        .validate_goldilocks_production_query_domains(&vk, parameters)
        .expect("real Tip5 L0 verifier circuit must support production terminal query count");

    let prove_start = std::time::Instant::now();
    let local_proof = compiler
        .prove_terminal_local_queries_goldilocks(
            &vk,
            &terminal_witness.public_inputs,
            &terminal_witness,
            parameters,
        )
        .expect("terminal local proof must build for real Tip5 L0 verifier circuit");
    let local_prove_elapsed = prove_start.elapsed();

    let combined_fold_plan = compiler
        .derive_terminal_combined_validity_fold_query_plan(
            &local_proof.prelude,
            &local_proof.combined_validity_commitment,
            &local_proof.combined_validity_fold_proof.fold_commitments,
        )
        .expect("real terminal combined-validity fold query plan must derive");
    assert_eq!(
        combined_fold_plan.indices.len(),
        parameters.num_queries as usize
    );
    for (pos, index) in combined_fold_plan.indices.iter().enumerate() {
        assert!(
            !combined_fold_plan.indices[..pos].contains(index),
            "real terminal combined-validity fold queries must be distinct"
        );
    }

    let certificate = compiler
        .assemble_goldilocks_local_certificate(&vk, &terminal_witness.public_inputs, &local_proof)
        .expect("terminal local certificate must assemble");
    let body_size = certificate.proof_body.len();
    let certificate_size = postcard::to_allocvec(&certificate)
        .expect("terminal local certificate must serialize")
        .len();
    let prelude_size = postcard::to_allocvec(&local_proof.prelude)
        .expect("serialize terminal local prelude")
        .len();
    let combined_validity_consistency_size =
        postcard::to_allocvec(&local_proof.combined_validity_consistency_proof)
            .expect("serialize terminal combined validity consistency proof")
            .len();
    let combined_validity_fold_size =
        postcard::to_allocvec(&local_proof.combined_validity_fold_proof)
            .expect("serialize terminal combined validity fold proof")
            .len();

    let verify_start = std::time::Instant::now();
    compiler
        .verify_goldilocks_local_certificate(&vk, &certificate, &terminal_witness.public_inputs)
        .expect("terminal local certificate must verify");
    let verify_elapsed = verify_start.elapsed();

    eprintln!(
        "terminal local certificate: body={} bytes ({:.1} KiB) certificate={} bytes ({:.1} KiB) prove={:.3}s verify={:.3}s parameters={{security_bits:{}, log_blowup:{}, num_queries:{}, query_pow_bits:{}}}",
        body_size,
        body_size as f64 / 1024.0,
        certificate_size,
        certificate_size as f64 / 1024.0,
        local_prove_elapsed.as_secs_f64(),
        verify_elapsed.as_secs_f64(),
        parameters.security_bits,
        parameters.log_blowup,
        parameters.num_queries,
        parameters.query_pow_bits,
    );
    eprintln!(
        "terminal local components: prelude={} combined_validity_consistency={} combined_validity_fold={}",
        prelude_size, combined_validity_consistency_size, combined_validity_fold_size,
    );

    assert!(certificate_size > body_size);
}

// ---------------------------------------------------------------------
//  Poseidon2-based L1 (the baseline)
// ---------------------------------------------------------------------

mod poseidon2_l1 {
    use p3_test_utils::goldilocks_params::{
        ChallengeMmcs as PoChallengeMmcs, Challenger as PoChallenger, Dft as PoDft,
        MyCompress as PoCompress, MyConfig as PoCfg, MyHash as PoHash, MyMmcs as PoValMmcs,
        MyPcs as PoPcs, Perm as PoPerm,
    };
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    use super::*;

    pub fn build_l1_poseidon2() -> usize {
        let perm = {
            let mut rng = SmallRng::seed_from_u64(1);
            PoPerm::new_from_rng_128(&mut rng)
        };
        let hash = PoHash::new(perm.clone());
        let compress = PoCompress::new(perm.clone());
        let val_mmcs = PoValMmcs::new(hash, compress, 0);
        let challenge_mmcs = PoChallengeMmcs::new(val_mmcs.clone());
        let dft = PoDft::default();
        let fri_params = FriParameters {
            log_blowup: 2,
            log_final_poly_len: 0,
            max_log_arity: 1,
            num_queries: 42,
            commit_proof_of_work_bits: 1,
            query_proof_of_work_bits: 1,
            mmcs: challenge_mmcs,
        };
        let pcs = PoPcs::new(dft, val_mmcs, fri_params);
        let outer_cfg: PoCfg = PoCfg::new(pcs, PoChallenger::new(perm));

        let BuiltLayer0Circuit {
            circuit,
            public_inputs,
            private_inputs,
            mmcs_op_ids,
            proof,
        } = build_layer0_verifier_circuit();

        let table_packing = TablePacking::new(1, 8);
        let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> = vec![
            Box::new(Tip5Preprocessor),
            Box::new(RecomposePreprocessor::new(true)),
        ];
        let mut air_builders = tip5_air_builders::<PoCfg, 2>();
        air_builders.extend(recompose_air_builders::<PoCfg, 2>(1, true));

        let (airs_degrees, primitive_columns, non_primitive_columns) =
            get_airs_and_degrees_with_prep::<PoCfg, Challenge, 2>(
                &circuit,
                &table_packing,
                &npo_prep,
                &air_builders,
                ConstraintProfile::Standard,
            )
            .expect("get_airs_and_degrees_with_prep [Po]");
        let airs_degrees_vec: Vec<_> = airs_degrees.into_iter().collect();
        let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees_vec.into_iter().unzip();

        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        runner.set_private_inputs(&private_inputs).unwrap();
        set_fri_mmcs_private_data::<
            Val,
            Challenge,
            ChallengeMmcs,
            ValMmcs,
            Tip5Sponge,
            Tip5Compress,
            DIGEST_ELEMS,
        >(
            &mut runner,
            &mmcs_op_ids,
            &proof.opening_proof,
            Tip5Config::GOLDILOCKS_W16,
        )
        .expect("set_fri_mmcs_private_data [Po]");

        let traces = runner.run().expect("[Po] runner().run()");

        let prover_data = ProverData::from_airs_and_degrees(&outer_cfg, &airs, &degrees);
        let circuit_prover_data =
            CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

        let mut prover = BatchStarkProver::new(outer_cfg).with_table_packing(table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);

        let batch_proof = prover
            .prove_all_tables(&traces, &circuit_prover_data)
            .expect("[Po] prove_all_tables");
        prover
            .verify_all_tables(&batch_proof)
            .expect("[Po] verify_all_tables");

        let size = postcard::to_allocvec(&batch_proof).expect("postcard").len();
        eprintln!(
            "[Poseidon2-W8] L1 outer-cert size: {} bytes ({:.1} KB)",
            size,
            size as f64 / 1024.0
        );
        size
    }
}

// ---------------------------------------------------------------------
//  Tip5-unified L1 (the new path)
// ---------------------------------------------------------------------

mod tip5_unified_l1 {
    use p3_test_utils::goldilocks_tip5_params::{
        ChallengeMmcs as TipsChallengeMmcs, Challenger as TipsChallenger, Dft as TipsDft,
        MyCompress as TipsCompress, MyConfig as TipsCfg, MyHash as TipsHash, MyMmcs as TipsValMmcs,
        MyPcs as TipsPcs,
    };

    use super::*;

    pub fn build_l1_tip5_unified() -> Result<usize, String> {
        let perm = Tip5Perm;
        let hash = TipsHash::new(perm);
        let compress = TipsCompress::new(perm);
        let val_mmcs = TipsValMmcs::new(hash, compress, 0);
        let challenge_mmcs = TipsChallengeMmcs::new(val_mmcs.clone());
        let dft = TipsDft::default();
        let fri_params = FriParameters {
            log_blowup: 2,
            log_final_poly_len: 0,
            max_log_arity: 1,
            num_queries: 42,
            commit_proof_of_work_bits: 1,
            query_proof_of_work_bits: 1,
            mmcs: challenge_mmcs,
        };
        let pcs = TipsPcs::new(dft, val_mmcs, fri_params);
        let outer_cfg: TipsCfg = TipsCfg::new(pcs, TipsChallenger::new(perm));

        let BuiltLayer0Circuit {
            circuit,
            public_inputs,
            private_inputs,
            mmcs_op_ids,
            proof,
        } = build_layer0_verifier_circuit();

        let table_packing = TablePacking::new(1, 8);
        let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> = vec![
            Box::new(Tip5Preprocessor),
            Box::new(RecomposePreprocessor::new(true)),
        ];
        let mut air_builders = tip5_air_builders::<TipsCfg, 2>();
        air_builders.extend(recompose_air_builders::<TipsCfg, 2>(1, true));

        let (airs_degrees, primitive_columns, non_primitive_columns) =
            get_airs_and_degrees_with_prep::<TipsCfg, Challenge, 2>(
                &circuit,
                &table_packing,
                &npo_prep,
                &air_builders,
                ConstraintProfile::Standard,
            )
            .map_err(|e| format!("get_airs_and_degrees [Tips]: {e:?}"))?;
        let airs_degrees_vec: Vec<_> = airs_degrees.into_iter().collect();
        let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees_vec.into_iter().unzip();

        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        runner.set_private_inputs(&private_inputs).unwrap();
        set_fri_mmcs_private_data::<
            Val,
            Challenge,
            ChallengeMmcs,
            ValMmcs,
            Tip5Sponge,
            Tip5Compress,
            DIGEST_ELEMS,
        >(
            &mut runner,
            &mmcs_op_ids,
            &proof.opening_proof,
            Tip5Config::GOLDILOCKS_W16,
        )
        .map_err(|e| format!("set_fri_mmcs_private_data [Tips]: {e}"))?;

        let traces = runner.run().map_err(|e| {
            format!(
                "[Tips] runner().run() FAILED: {e:?}\n\
                 This is the predicted C2.4 R-a tail at D=2. Per spec §3.3.B,\n\
                 the workaround is D=1 outer-cert.",
            )
        })?;

        let prover_data = ProverData::from_airs_and_degrees(&outer_cfg, &airs, &degrees);
        let circuit_prover_data =
            CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

        let mut prover = BatchStarkProver::new(outer_cfg).with_table_packing(table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);

        let batch_proof = prover
            .prove_all_tables(&traces, &circuit_prover_data)
            .map_err(|e| format!("[Tips] prove_all_tables: {e:?}"))?;
        prover
            .verify_all_tables(&batch_proof)
            .map_err(|e| format!("[Tips] verify_all_tables: {e:?}"))?;

        let size = postcard::to_allocvec(&batch_proof).expect("postcard").len();
        eprintln!(
            "[Tip5-unified] L1 outer-cert size: {} bytes ({:.1} KB)",
            size,
            size as f64 / 1024.0
        );
        Ok(size)
    }
}

// ---------------------------------------------------------------------
//  Tip5-out-4 variant — Tip5 with DIGEST_ELEMS=4 instead of 5
//  (investigation: is the L1 size delta dominated by digest width?)
// ---------------------------------------------------------------------

mod tip5_out4_l1 {
    use p3_test_utils::goldilocks_tip5_out4_params::{
        ChallengeMmcs as O4ChallengeMmcs, Challenger as O4Challenger, Dft as O4Dft,
        MyCompress as O4Compress, MyConfig as O4Cfg, MyHash as O4Hash, MyMmcs as O4ValMmcs,
        MyPcs as O4Pcs,
    };

    use super::*;

    pub fn build_l1_tip5_out4() -> Result<usize, String> {
        let perm = Tip5Perm;
        let hash = O4Hash::new(perm);
        let compress = O4Compress::new(perm);
        let val_mmcs = O4ValMmcs::new(hash, compress, 0);
        let challenge_mmcs = O4ChallengeMmcs::new(val_mmcs.clone());
        let dft = O4Dft::default();
        let fri_params = FriParameters {
            log_blowup: 2,
            log_final_poly_len: 0,
            max_log_arity: 1,
            num_queries: 42,
            commit_proof_of_work_bits: 1,
            query_proof_of_work_bits: 1,
            mmcs: challenge_mmcs,
        };
        let pcs = O4Pcs::new(dft, val_mmcs, fri_params);
        let outer_cfg: O4Cfg = O4Cfg::new(pcs, O4Challenger::new(perm));

        let BuiltLayer0Circuit {
            circuit,
            public_inputs,
            private_inputs,
            mmcs_op_ids,
            proof,
        } = build_layer0_verifier_circuit();

        let table_packing = TablePacking::new(1, 8);
        let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> = vec![
            Box::new(Tip5Preprocessor),
            Box::new(RecomposePreprocessor::new(true)),
        ];
        let mut air_builders = tip5_air_builders::<O4Cfg, 2>();
        air_builders.extend(recompose_air_builders::<O4Cfg, 2>(1, true));

        let (airs_degrees, primitive_columns, non_primitive_columns) =
            get_airs_and_degrees_with_prep::<O4Cfg, Challenge, 2>(
                &circuit,
                &table_packing,
                &npo_prep,
                &air_builders,
                ConstraintProfile::Standard,
            )
            .map_err(|e| format!("get_airs_and_degrees [O4]: {e:?}"))?;
        let airs_degrees_vec: Vec<_> = airs_degrees.into_iter().collect();
        let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees_vec.into_iter().unzip();

        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        runner.set_private_inputs(&private_inputs).unwrap();
        set_fri_mmcs_private_data::<
            Val,
            Challenge,
            ChallengeMmcs,
            ValMmcs,
            Tip5Sponge,
            Tip5Compress,
            DIGEST_ELEMS,
        >(
            &mut runner,
            &mmcs_op_ids,
            &proof.opening_proof,
            Tip5Config::GOLDILOCKS_W16,
        )
        .map_err(|e| format!("set_fri_mmcs_private_data [O4]: {e}"))?;

        let traces = runner.run().map_err(|e| format!("[O4] runner: {e:?}"))?;

        let prover_data = ProverData::from_airs_and_degrees(&outer_cfg, &airs, &degrees);
        let circuit_prover_data =
            CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

        let mut prover = BatchStarkProver::new(outer_cfg).with_table_packing(table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);

        let batch_proof = prover
            .prove_all_tables(&traces, &circuit_prover_data)
            .map_err(|e| format!("[O4] prove_all_tables: {e:?}"))?;
        prover
            .verify_all_tables(&batch_proof)
            .map_err(|e| format!("[O4] verify_all_tables: {e:?}"))?;

        let size = postcard::to_allocvec(&batch_proof).expect("postcard").len();
        eprintln!(
            "[Tip5-out-4]   L1 outer-cert size: {} bytes ({:.1} KB)",
            size,
            size as f64 / 1024.0
        );
        Ok(size)
    }
}

/// P4 — Full L1 outer-cert size comparison at Tip5-unified vs Poseidon2.
#[test]
#[ignore = "M-S5b S1.B P4 — heavy L1 build (~30s); manual invocation"]
fn p4_l1_size_tip5_unified_vs_poseidon2() {
    let size_poseidon2 = poseidon2_l1::build_l1_poseidon2();
    let size_tip5_result = tip5_unified_l1::build_l1_tip5_unified();
    let size_tip5_out4_result = tip5_out4_l1::build_l1_tip5_out4();

    eprintln!("");
    eprintln!("=== M-S5b S1.B P4 — L1 SIZE COMPARISON ===");
    eprintln!(
        "Poseidon2-W8 baseline:        {} bytes ({:.1} KB)",
        size_poseidon2,
        size_poseidon2 as f64 / 1024.0
    );

    match size_tip5_result {
        Ok(size_tip5) => {
            let delta = size_tip5 as i64 - size_poseidon2 as i64;
            eprintln!(
                "Tip5-unified (digest=5):      {} bytes ({:.1} KB) [{:+} bytes vs Po]",
                size_tip5,
                size_tip5 as f64 / 1024.0,
                delta
            );
        }
        Err(e) => eprintln!("Tip5-unified BLOCKED: {e}"),
    }

    match size_tip5_out4_result {
        Ok(size_tip5_out4) => {
            let delta = size_tip5_out4 as i64 - size_poseidon2 as i64;
            eprintln!(
                "Tip5-out-4 (digest=4):        {} bytes ({:.1} KB) [{:+} bytes vs Po]",
                size_tip5_out4,
                size_tip5_out4 as f64 / 1024.0,
                delta
            );
        }
        Err(e) => eprintln!("Tip5-out-4 BLOCKED: {e}"),
    }

    eprintln!("==========================================");
}
