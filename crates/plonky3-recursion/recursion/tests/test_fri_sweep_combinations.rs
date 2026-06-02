//! Combinatorial sweep over inner + outer FRI parameters
//! (2026-05-21) — confirms whether the post-C1 production
//! config (inner `lb=4 nq=20 pow=1+1`, outer `lb=4 nq=20 mla=3
//! lfp=2 cap=3 pow=1+1`, both at 82 bits unconditional Johnson)
//! is at a local optimum or whether a better point exists.
//!
//! Each sweep point varies one or two parameters from the
//! current production config and measures L1 + L2 sizes. All
//! points maintain ≥80-bit unconditional Johnson per
//! IACR ePrint 2025/2055 Theorem 1.5 + paper-faithful digest=5.
//!
//! Heavy (~70-90 min wall time for ~8 variants); `#[ignore]`'d.

#![allow(clippy::too_many_arguments)]

mod common;

use p3_batch_stark::ProverData;
use p3_challenger::DuplexChallenger;
use p3_circuit::CircuitBuilder;
use p3_circuit::ops::{
    PrimitiveOpType, Tip5Config, Tip5Goldilocks, generate_recompose_trace, generate_tip5_trace,
};
use p3_circuit::test_utils::{FibonacciAir, generate_trace_rows};
use p3_circuit_prover::batch_stark_prover::{
    BatchStarkProver, recompose_air_builders, recompose_table_provers, tip5_air_builders,
};
use p3_circuit_prover::common::{NpoPreprocessor, get_airs_and_degrees_with_prep};
use p3_circuit_prover::{
    BatchStarkProof, CircuitProverData, ConstraintProfile, RecomposePreprocessor, TableProver,
    TablePacking, Tip5Preprocessor, Tip5Prover,
};
use p3_commit::ExtensionMmcs;
use p3_dft::Radix2DitParallel;
use p3_field::extension::BinomialExtensionField;
use p3_field::{BasedVectorSpace, PrimeCharacteristicRing};
use p3_fri::{FriParameters, TwoAdicFriPcs};
use p3_goldilocks::Goldilocks;
use p3_lookup::logup::LogUpGadget;
use p3_merkle_tree::MerkleTreeMmcs;
use p3_recursion::pcs::fri::{FriVerifierParams, InputProofTargets, MerkleCapTargets, RecValMmcs};
use p3_recursion::pcs::set_fri_mmcs_private_data;
use p3_recursion::public_inputs::StarkVerifierInputsBuilder;
use p3_recursion::verifier::verify_p3_batch_proof_circuit;
use p3_recursion::verify_p3_uni_proof_circuit;
use p3_symmetric::{PaddingFreeSponge, Permutation, TruncatedPermutation};
use p3_test_utils::goldilocks_tip5_params::{
    ChallengeMmcs as TipsChallengeMmcs, Challenger as TipsChallenger, Dft as TipsDft,
    MyCompress as TipsCompress, MyConfig as TipsCfg, MyHash as TipsHash,
    MyMmcs as TipsValMmcs, MyPcs as TipsPcs,
};
use p3_tip5_circuit_air::Tip5Perm;
use p3_uni_stark::{StarkConfig, prove, verify};

use crate::common::InnerFriGeneric;

// ---------------------------------------------------------------------
//  Inner L0 (Tip5 Layer-0) — verbatim type aliases.
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

// ---------------------------------------------------------------------
//  Parameterized FRI knobs for inner + outer.
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct InnerKnobs {
    log_blowup: usize,
    num_queries: usize,
    commit_pow: usize,
    query_pow: usize,
}

#[derive(Clone, Copy, Debug)]
struct OuterKnobs {
    log_blowup: usize,
    log_final_poly_len: usize,
    max_log_arity: usize,
    num_queries: usize,
    commit_pow: usize,
    query_pow: usize,
    cap_height: usize,
}

impl InnerKnobs {
    fn bits(&self) -> usize {
        self.log_blowup * self.num_queries + self.commit_pow + self.query_pow
    }
}

impl OuterKnobs {
    fn bits(&self) -> usize {
        self.log_blowup * self.num_queries + self.commit_pow + self.query_pow
    }
}

// ---------------------------------------------------------------------
//  Build inner Tip5-L0 config + verifier circuit, parameterized.
// ---------------------------------------------------------------------

fn make_layer0_config_with_knobs(k: InnerKnobs) -> Tip5Layer0Config {
    let perm = Tip5Perm;
    let hash = Tip5Sponge::new(perm);
    let compress = Tip5Compress::new(perm);
    let val_mmcs = ValMmcs::new(hash, compress, 0);
    let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());
    let dft = Dft::default();
    let challenger = Layer0Challenger::new(perm);
    let fri_params = FriParameters {
        log_blowup: k.log_blowup,
        log_final_poly_len: 0,
        max_log_arity: 1,
        num_queries: k.num_queries,
        commit_proof_of_work_bits: k.commit_pow,
        query_proof_of_work_bits: k.query_pow,
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

fn build_layer0_verifier_circuit_with_knobs(
    inner: InnerKnobs,
    tamper: bool,
) -> BuiltLayer0Circuit {
    let config = make_layer0_config_with_knobs(inner);
    let n = 1 << 3;
    let x = 21u64;
    let trace = generate_trace_rows::<Val>(0, 1, n);
    let pis = vec![Val::ZERO, Val::ONE, Val::from_u64(x)];
    let air = FibonacciAir {};

    let mut proof = prove(&config, &air, trace, &pis);
    assert!(verify(&config, &air, &proof, &pis).is_ok());

    if tamper {
        proof.opened_values.trace_local[0] += Challenge::ONE;
    }

    let mut circuit_builder = CircuitBuilder::<Challenge>::new();
    circuit_builder.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<Challenge, Tip5Goldilocks>,
        LiftTip5,
    );
    circuit_builder.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
    circuit_builder.set_recompose_coeff_ctl_for_decompose_links(true);

    let fri_verifier_params = FriVerifierParams::with_mmcs(
        inner.log_blowup,
        0,
        inner.commit_pow,
        inner.query_pow,
        Tip5Config::GOLDILOCKS_W16,
    );

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
        &make_layer0_config_with_knobs(inner),
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

// ---------------------------------------------------------------------
//  Outer cert config + L1 build (parameterized).
// ---------------------------------------------------------------------

fn make_outer_cfg_with_knobs(k: OuterKnobs) -> TipsCfg {
    let perm = Tip5Perm;
    let hash = TipsHash::new(perm);
    let compress = TipsCompress::new(perm);
    let val_mmcs = TipsValMmcs::new(hash, compress, k.cap_height);
    let challenge_mmcs = TipsChallengeMmcs::new(val_mmcs.clone());
    let dft = TipsDft::default();
    let fri_params = FriParameters {
        log_blowup: k.log_blowup,
        log_final_poly_len: k.log_final_poly_len,
        max_log_arity: k.max_log_arity,
        num_queries: k.num_queries,
        commit_proof_of_work_bits: k.commit_pow,
        query_proof_of_work_bits: k.query_pow,
        mmcs: challenge_mmcs,
    };
    let pcs = TipsPcs::new(dft, val_mmcs, fri_params);
    TipsCfg::new(pcs, TipsChallenger::new(perm))
}

type L2Val = Val;
type L2Challenge = Challenge;
type L2ChallengeMmcs = TipsChallengeMmcs;
type L2ValMmcs = TipsValMmcs;
type L2Hash = TipsHash;
type L2Compress = TipsCompress;
const L2_DIGEST_ELEMS: usize = DIGEST_ELEMS;
const L2_WIDTH: usize = WIDTH;
const L2_RATE: usize = RATE;
type L2InnerFri = InnerFriGeneric<TipsCfg, L2Hash, L2Compress, L2_DIGEST_ELEMS>;

fn build_l1_with_knobs(
    inner: InnerKnobs,
    outer: OuterKnobs,
) -> Result<BatchStarkProof<TipsCfg>, String> {
    let outer_config = make_outer_cfg_with_knobs(outer);
    let BuiltLayer0Circuit {
        circuit,
        public_inputs,
        private_inputs,
        mmcs_op_ids,
        proof,
    } = build_layer0_verifier_circuit_with_knobs(inner, false);

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
        .map_err(|e| format!("get_airs_and_degrees: {e:?}"))?;
    let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();

    let mut runner = circuit.runner();
    runner.set_public_inputs(&public_inputs)
        .map_err(|e| format!("set_public_inputs: {e:?}"))?;
    runner.set_private_inputs(&private_inputs)
        .map_err(|e| format!("set_private_inputs: {e:?}"))?;
    set_fri_mmcs_private_data::<
        Val, Challenge, ChallengeMmcs, ValMmcs, Tip5Sponge, Tip5Compress, DIGEST_ELEMS,
    >(
        &mut runner, &mmcs_op_ids, &proof.opening_proof, Tip5Config::GOLDILOCKS_W16,
    ).map_err(|e| format!("set_fri_mmcs_private_data: {e}"))?;

    let traces = runner.run().map_err(|e| format!("runner: {e:?}"))?;
    let prover_data = ProverData::from_airs_and_degrees(&outer_config, &airs, &degrees);
    let circuit_prover_data = CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

    let mut prover = BatchStarkProver::new(outer_config).with_table_packing(table_packing);
    prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    prover.register_recompose_table::<2>(true);
    let l1 = prover.prove_all_tables(&traces, &circuit_prover_data)
        .map_err(|e| format!("prove_all_tables: {e:?}"))?;
    prover.verify_all_tables(&l1).map_err(|e| format!("verify_all_tables: {e:?}"))?;
    Ok(l1)
}

fn inner_npo_provers() -> Vec<Box<dyn TableProver<TipsCfg>>> {
    let mut tp: Vec<Box<dyn TableProver<TipsCfg>>> = vec![Box::new(Tip5Prover::new(
        Tip5Config::GOLDILOCKS_W16,
        ConstraintProfile::Standard,
    ))];
    tp.extend(recompose_table_provers::<TipsCfg, 2>(1, true));
    tp
}

fn l2_over_l1_with_knobs(
    label: &str,
    l1: &BatchStarkProof<TipsCfg>,
    outer_inner_knobs: OuterKnobs, // for L2's inner_fri (matching L1's outer)
    l1_config: &TipsCfg,
    l2_config: TipsCfg,
) -> Result<usize, String> {
    let common = &l1.stark_common;
    let batch_proof = &l1.proof;
    const TRACE_D: usize = 2;

    let num_tables = common.preprocessed.as_ref().map(|g| g.instances.len()).unwrap_or(0);
    let pis: Vec<Vec<L2Val>> = vec![vec![]; num_tables];

    let mut circuit_builder = CircuitBuilder::<L2Challenge>::new();
    circuit_builder.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<L2Challenge, Tip5Goldilocks>,
        LiftTip5,
    );
    circuit_builder.enable_recompose::<L2Val>(generate_recompose_trace::<L2Val, L2Challenge>);
    circuit_builder.set_recompose_coeff_ctl_for_decompose_links(true);

    let lookup_gadget = LogUpGadget::new();
    let inner_fri = FriVerifierParams::with_mmcs(
        outer_inner_knobs.log_blowup,
        outer_inner_knobs.log_final_poly_len,
        outer_inner_knobs.commit_pow,
        outer_inner_knobs.query_pow,
        Tip5Config::GOLDILOCKS_W16,
    );

    let (verifier_inputs, mmcs_op_ids) = verify_p3_batch_proof_circuit::<
        TipsCfg,
        MerkleCapTargets<L2Val, L2_DIGEST_ELEMS>,
        InputProofTargets<L2Val, L2Challenge, RecValMmcs<L2Val, L2_DIGEST_ELEMS, L2Hash, L2Compress>>,
        L2InnerFri,
        LogUpGadget,
        _,
        L2_WIDTH,
        L2_RATE,
        TRACE_D,
    >(
        l1_config, &mut circuit_builder, l1, &inner_fri, common, &lookup_gadget,
        Tip5Config::GOLDILOCKS_W16, &inner_npo_provers(),
    ).map_err(|e| format!("[{label}] verify_p3_batch_proof_circuit (L2 build) failed: {e:?}"))?;

    let verification_circuit = circuit_builder.build()
        .map_err(|e| format!("[{label}] L2 circuit build failed: {e:?}"))?;
    let (public_inputs, private_inputs) = verifier_inputs.pack_values(&pis, batch_proof, common);

    let verification_table_packing = TablePacking::new(1, 8);
    let npo_prep: Vec<Box<dyn NpoPreprocessor<L2Val>>> = vec![
        Box::new(Tip5Preprocessor),
        Box::new(RecomposePreprocessor::new(true)),
    ];
    let mut air_builders = tip5_air_builders::<TipsCfg, 2>();
    air_builders.extend(recompose_air_builders::<TipsCfg, 2>(1, true));
    let (v_airs_degrees, v_primitive, v_npo) = get_airs_and_degrees_with_prep::<TipsCfg, L2Challenge, 2>(
        &verification_circuit, &verification_table_packing, &npo_prep, &air_builders,
        ConstraintProfile::Standard,
    ).map_err(|e| format!("[{label}] L2 get_airs_and_degrees: {e:?}"))?;
    let (v_airs, v_degrees): (Vec<_>, Vec<usize>) = v_airs_degrees.into_iter().unzip();

    let mut runner = verification_circuit.runner();
    runner.set_public_inputs(&public_inputs).map_err(|e| format!("[{label}] L2 set_public_inputs: {e:?}"))?;
    runner.set_private_inputs(&private_inputs).map_err(|e| format!("[{label}] L2 set_private_inputs: {e:?}"))?;
    if !mmcs_op_ids.is_empty() {
        set_fri_mmcs_private_data::<
            L2Val, L2Challenge, L2ChallengeMmcs, L2ValMmcs, L2Hash, L2Compress, L2_DIGEST_ELEMS,
        >(&mut runner, &mmcs_op_ids, &l1.proof.opening_proof, Tip5Config::GOLDILOCKS_W16)
            .map_err(|e| format!("[{label}] L2 set_fri_mmcs_private_data: {e}"))?;
    }
    let v_traces = runner.run().map_err(|e| format!("[{label}] L2 runner().run(): {e:?}"))?;
    let v_prover_data = ProverData::from_airs_and_degrees(&l2_config, &v_airs, &v_degrees);
    let v_circuit_prover_data = CircuitProverData::new(v_prover_data, v_primitive, v_npo);
    let mut v_prover = BatchStarkProver::new(l2_config).with_table_packing(verification_table_packing);
    v_prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    v_prover.register_recompose_table::<2>(true);
    let l2_proof: BatchStarkProof<TipsCfg> = v_prover.prove_all_tables(&v_traces, &v_circuit_prover_data)
        .map_err(|e| format!("[{label}] L2 prove_all_tables: {e:?}"))?;
    v_prover.verify_all_tables(&l2_proof).map_err(|e| format!("[{label}] L2 verify_all_tables: {e:?}"))?;
    Ok(postcard::to_allocvec(&l2_proof).map_err(|e| format!("[{label}] serialize L2: {e:?}"))?.len())
}

// ---------------------------------------------------------------------
//  Sweep test
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct SweepPoint {
    label: &'static str,
    inner: InnerKnobs,
    outer: OuterKnobs,
}

#[test]
#[ignore = "FRI sweep combinations (VERY HEAVY, ~70-90 min wall time for 8 variants)"]
fn fri_sweep_inner_outer_combinations() {
    let inner_current = InnerKnobs { log_blowup: 4, num_queries: 20, commit_pow: 1, query_pow: 1 };
    let outer_current = OuterKnobs {
        log_blowup: 4, log_final_poly_len: 2, max_log_arity: 3, num_queries: 20,
        commit_pow: 1, query_pow: 1, cap_height: 3,
    };

    let sweep: Vec<SweepPoint> = vec![
        // S1: current baseline.
        SweepPoint {
            label: "S1 baseline (current production)",
            inner: inner_current, outer: outer_current,
        },
        // S2: smaller inner LDE.
        SweepPoint {
            label: "S2 inner lb=3 nq=27 pow=1+1 (81b)",
            inner: InnerKnobs { log_blowup: 3, num_queries: 27, commit_pow: 1, query_pow: 1 },
            outer: outer_current,
        },
        // S3: bigger inner LDE.
        SweepPoint {
            label: "S3 inner lb=5 nq=16 pow=1+1 (82b)",
            inner: InnerKnobs { log_blowup: 5, num_queries: 16, commit_pow: 1, query_pow: 1 },
            outer: outer_current,
        },
        // S4: outer cap=4.
        SweepPoint {
            label: "S4 outer cap=4",
            inner: inner_current,
            outer: OuterKnobs { cap_height: 4, ..outer_current },
        },
        // S5: outer cap=5.
        SweepPoint {
            label: "S5 outer cap=5",
            inner: inner_current,
            outer: OuterKnobs { cap_height: 5, ..outer_current },
        },
        // S6: outer lb=5 nq=16.
        SweepPoint {
            label: "S6 outer lb=5 nq=16 pow=1+1 (82b)",
            inner: inner_current,
            outer: OuterKnobs { log_blowup: 5, num_queries: 16, ..outer_current },
        },
        // S7: BOTH lb=5.
        SweepPoint {
            label: "S7 inner+outer lb=5 nq=16",
            inner: InnerKnobs { log_blowup: 5, num_queries: 16, commit_pow: 1, query_pow: 1 },
            outer: OuterKnobs { log_blowup: 5, num_queries: 16, ..outer_current },
        },
        // S8: outer mla=1 lfp=0 (revert Phase 0 — sanity baseline).
        SweepPoint {
            label: "S8 outer mla=1 lfp=0 (no Phase-0 levers)",
            inner: inner_current,
            outer: OuterKnobs { max_log_arity: 1, log_final_poly_len: 0, ..outer_current },
        },
        // S9: USER HYPOTHESIS — inner less compression + outer more.
        SweepPoint {
            label: "S9 USER-HYP inner lb=3 nq=27 / outer lb=5 nq=16",
            inner: InnerKnobs { log_blowup: 3, num_queries: 27, commit_pow: 1, query_pow: 1 },
            outer: OuterKnobs { log_blowup: 5, num_queries: 16, ..outer_current },
        },
        // S10: outer lb=6 nq=14 (extreme outer compression).
        SweepPoint {
            label: "S10 outer lb=6 nq=14 (extreme outer compression)",
            inner: inner_current,
            outer: OuterKnobs { log_blowup: 6, num_queries: 14, ..outer_current },
        },
        // S11: inner lb=2 nq=45 (extreme inner DE-compression).
        SweepPoint {
            label: "S11 USER-HYP-EXTREME inner lb=2 nq=45 / outer current",
            inner: InnerKnobs { log_blowup: 2, num_queries: 45, commit_pow: 0, query_pow: 0 },
            outer: outer_current,
        },
        // S12: outer Tier C (digest=5 mla=3 lfp=2 cap=3) + outer max_log_arity bumped further (mla=4 if supported).
        SweepPoint {
            label: "S12 outer max_log_arity=4 (further fold compression)",
            inner: inner_current,
            outer: OuterKnobs { max_log_arity: 4, ..outer_current },
        },
    ];

    eprintln!("\n=== FRI SWEEP COMBINATIONS (inner × outer) ===");
    eprintln!("All points: 5-round Tip5, paper-faithful digest=5, ≥80-bit unconditional Johnson.\n");

    let mut results: Vec<(String, Option<(usize, usize)>, String)> = Vec::new();

    for point in &sweep {
        let inner_bits = point.inner.bits();
        let outer_bits = point.outer.bits();
        let label = format!("{} (inner {}b / outer {}b)", point.label, inner_bits, outer_bits);
        eprintln!("\n>>> Running {label}");

        let start = std::time::Instant::now();
        match build_l1_with_knobs(point.inner, point.outer) {
            Ok(l1) => {
                let l1_bytes = postcard::to_allocvec(&l1).expect("ser L1").len();
                let l2_result = l2_over_l1_with_knobs(
                    point.label,
                    &l1,
                    point.outer,
                    &make_outer_cfg_with_knobs(point.outer),
                    make_outer_cfg_with_knobs(point.outer),
                );
                let elapsed = start.elapsed().as_secs();
                match l2_result {
                    Ok(l2_bytes) => {
                        eprintln!("  L1 = {l1_bytes} B ({:.2} KB)", l1_bytes as f64 / 1024.0);
                        eprintln!("  L2 = {l2_bytes} B ({:.2} KB)", l2_bytes as f64 / 1024.0);
                        eprintln!("  L2/L1 = {:.3}×", l2_bytes as f64 / l1_bytes as f64);
                        eprintln!("  time = {elapsed}s");
                        results.push((label, Some((l1_bytes, l2_bytes)), format!("ok ({elapsed}s)")));
                    }
                    Err(e) => {
                        eprintln!("  L1 OK = {l1_bytes} B, L2 FAILED: {e}");
                        results.push((label, None, format!("L2 fail: {e}")));
                    }
                }
            }
            Err(e) => {
                eprintln!("  L1 FAILED: {e}");
                results.push((label, None, format!("L1 fail: {e}")));
            }
        }
    }

    eprintln!("\n=== SWEEP SUMMARY ===");
    eprintln!("  {:<55}  {:>10}  {:>10}  {:>8}  status", "label", "L1 bytes", "L2 bytes", "L2/L1");
    eprintln!("  {}", "-".repeat(100));
    for (label, sizes, status) in &results {
        match sizes {
            Some((l1, l2)) => eprintln!(
                "  {label:<55}  {l1:>10}  {l2:>10}  {:>7.3}×  {status}",
                *l2 as f64 / *l1 as f64,
            ),
            None => eprintln!("  {label:<55}  {:>10}  {:>10}  {:>8}  {status}", "—", "—", "—"),
        }
    }
}
