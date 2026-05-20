//! M-S5b Path B Stage B0 — production L1 outer-cert inventory.
//!
//! Builds the production L1 cert (Tip5-throughout, post-Phase-0
//! FRI: `lb=4 nq=20 mla=3 lfp=2 cap=3 d=5`) and dumps a per-AIR
//! breakdown: rows, lanes, preprocessed width, estimated cell
//! counts, lookup-bus tuple counts. The output is the GATE for
//! Stage B1 (reduction map).
//!
//! Per [no_poseidon2_anywhere] hard rule: 100% Tip5 substrate,
//! zero Poseidon2.
//!
//! Heavy (~few minutes) — `#[ignore]`d behind `--ignored`.

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
    BatchStarkProver, recompose_air_builders, tip5_air_builders,
};
use p3_circuit_prover::common::{NpoPreprocessor, get_airs_and_degrees_with_prep};
use p3_circuit_prover::{
    BatchStarkProof, CircuitProverData, ConstraintProfile, RecomposePreprocessor,
    TablePacking, Tip5Preprocessor,
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
use p3_test_utils::goldilocks_tip5_params::{
    ChallengeMmcs as TipsChallengeMmcs, Challenger as TipsChallenger, Dft as TipsDft,
    MyCompress as TipsCompress, MyConfig as TipsCfg, MyHash as TipsHash,
    MyMmcs as TipsValMmcs, MyPcs as TipsPcs,
};
use p3_tip5_circuit_air::Tip5Perm;
use p3_uni_stark::{StarkConfig, prove, verify};

use crate::common::InnerFriGeneric;

// ---------------------------------------------------------------------
//  Inner L0 (Tip5 Layer-0) — verbatim from test_tip5_l2_over_l1.rs.
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
    let fri_params = FriParameters {
        log_blowup: 3,
        log_final_poly_len: 0,
        max_log_arity: 1,
        num_queries: 30,
        commit_proof_of_work_bits: 0,
        query_proof_of_work_bits: 0,
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

// ---------------------------------------------------------------------
//  Production outer cert at the LANDED post-Phase-0 FRI params.
// ---------------------------------------------------------------------

fn make_production_outer_cfg() -> TipsCfg {
    let perm = Tip5Perm;
    let hash = TipsHash::new(perm);
    let compress = TipsCompress::new(perm);
    let val_mmcs = TipsValMmcs::new(hash, compress, 3);
    let challenge_mmcs = TipsChallengeMmcs::new(val_mmcs.clone());
    let dft = TipsDft::default();
    let fri_params = FriParameters {
        log_blowup: 4,
        log_final_poly_len: 2,
        max_log_arity: 3,
        num_queries: 20,
        commit_proof_of_work_bits: 1,
        query_proof_of_work_bits: 1,
        mmcs: challenge_mmcs,
    };
    let pcs = TipsPcs::new(dft, val_mmcs, fri_params);
    TipsCfg::new(pcs, TipsChallenger::new(perm))
}

/// Per-AIR width breakdown (computed from the actual airs vector
/// before proving). Returned alongside the proof so the inventory
/// dump can report (rows × cols) cell counts, not just rows.
struct AirWidths {
    /// One entry per AIR in `airs_degrees` order: (width, prep_width).
    widths: Vec<(usize, usize)>,
}

fn build_production_l1() -> Result<(BatchStarkProof<TipsCfg>, AirWidths), String> {
    let outer_config = make_production_outer_cfg();
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
        .map_err(|e| format!("get_airs_and_degrees: {e:?}"))?;
    let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();

    // Extract per-AIR widths BEFORE consuming `airs` for proving.
    use p3_air::BaseAir;
    use p3_circuit_prover::common::CircuitTableAir;
    let widths: Vec<(usize, usize)> = airs
        .iter()
        .map(|air| match air {
            CircuitTableAir::Const(a) => (BaseAir::<Val>::width(a), BaseAir::<Val>::preprocessed_width(a)),
            CircuitTableAir::Public(a) => (BaseAir::<Val>::width(a), BaseAir::<Val>::preprocessed_width(a)),
            CircuitTableAir::Alu(a) => (BaseAir::<Val>::width(a), BaseAir::<Val>::preprocessed_width(a)),
            CircuitTableAir::Dynamic(a) => (BaseAir::<Val>::width(a), BaseAir::<Val>::preprocessed_width(a)),
        })
        .collect();
    let air_widths = AirWidths { widths };

    let mut runner = circuit.runner();
    runner
        .set_public_inputs(&public_inputs)
        .map_err(|e| format!("set_public_inputs: {e:?}"))?;
    runner
        .set_private_inputs(&private_inputs)
        .map_err(|e| format!("set_private_inputs: {e:?}"))?;
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
    .map_err(|e| format!("set_fri_mmcs_private_data: {e}"))?;

    let traces = runner.run().map_err(|e| format!("runner: {e:?}"))?;
    let prover_data = ProverData::from_airs_and_degrees(&outer_config, &airs, &degrees);
    let circuit_prover_data =
        CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

    let mut prover =
        BatchStarkProver::new(outer_config).with_table_packing(table_packing);
    prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    prover.register_recompose_table::<2>(true);
    let l1 = prover
        .prove_all_tables(&traces, &circuit_prover_data)
        .map_err(|e| format!("prove_all_tables: {e:?}"))?;
    prover
        .verify_all_tables(&l1)
        .map_err(|e| format!("verify_all_tables: {e:?}"))?;
    Ok((l1, air_widths))
}

// ---------------------------------------------------------------------
//  Inventory dump
// ---------------------------------------------------------------------

#[test]
#[ignore = "Path B Stage B0: production L1 outer-cert inventory (heavy ~few min)"]
fn path_b_stage_0_l1_inventory() {
    eprintln!("\n=== M-S5b PATH B STAGE B0 — production L1 cert inventory ===");
    eprintln!("Production FRI: lb=4 nq=20 mla=3 lfp=2 cap=3 d=5 (82-bit Johnson)");
    eprintln!("Substrate: 100% Tip5 (zero Poseidon2)\n");

    let (l1, air_widths) = build_production_l1().expect("L1 must build");
    let total_bytes = postcard::to_allocvec(&l1).expect("serialize L1").len();

    eprintln!("L1 TOTAL serialized: {total_bytes} B ({:.2} KB)\n", total_bytes as f64 / 1024.0);

    // ----- Per-AIR cell counts (rows × cols) -----
    // Iteration order matches the airs vector returned by
    // get_airs_and_degrees_with_prep: primitives first (Const, Public,
    // Alu), then non-primitives in registration order. The widths come
    // from the actual AIR instances at build time, so they reflect
    // production parameters exactly.
    let primitive_names = ["Const", "Public", "Alu"];
    let nprim = primitive_names.len();
    let primitive_rows = [
        l1.rows[PrimitiveOpType::Const],
        l1.rows[PrimitiveOpType::Public],
        l1.rows[PrimitiveOpType::Alu],
    ];

    eprintln!("=== PER-AIR CELL COUNTS (rows × cols) ===");
    eprintln!(
        "  {:<3}  {:<40}  {:>6}  {:>6}  {:>6}  {:>10}",
        "idx", "AIR", "rows", "width", "prep_w", "cells",
    );
    eprintln!("  {}", "-".repeat(82));

    let mut primitive_total_cells = 0usize;
    let mut primitive_total_rows_sum = 0usize;
    for (i, name) in primitive_names.iter().enumerate() {
        let rows = primitive_rows[i];
        let (w, pw) = air_widths.widths.get(i).copied().unwrap_or((0, 0));
        let cells = rows * (w + pw);
        eprintln!(
            "  [{i}]  {:<40}  {:>6}  {:>6}  {:>6}  {:>10}",
            name, rows, w, pw, cells,
        );
        primitive_total_cells += cells;
        primitive_total_rows_sum += rows;
    }
    eprintln!("  {}", "-".repeat(82));
    eprintln!("  primitive_subtotal: rows={primitive_total_rows_sum}, cells={primitive_total_cells}\n");

    let mut npo_total_cells = 0usize;
    let mut npo_total_packed_rows = 0usize;
    for (i, entry) in l1.non_primitives.iter().enumerate() {
        let packed_rows = if entry.lanes > 0 { entry.rows.div_ceil(entry.lanes) } else { entry.rows };
        let (w, pw) = air_widths.widths.get(nprim + i).copied().unwrap_or((0, 0));
        let cells = packed_rows * (w + pw);
        eprintln!(
            "  [{}]  {:<40}  {:>6}  {:>6}  {:>6}  {:>10}",
            nprim + i,
            format!("{:?}", entry.op_type),
            packed_rows,
            w,
            pw,
            cells,
        );
        npo_total_cells += cells;
        npo_total_packed_rows += packed_rows;
    }
    eprintln!("  {}", "-".repeat(82));
    eprintln!("  npo_subtotal: rows={npo_total_packed_rows}, cells={npo_total_cells}\n");
    eprintln!("  GRAND TOTAL: rows={}, cells={}\n",
        primitive_total_rows_sum + npo_total_packed_rows,
        primitive_total_cells + npo_total_cells,
    );

    let primitive_total_rows = primitive_total_rows_sum;
    let npo_total_rows = npo_total_packed_rows;

    // ----- Summary -----
    eprintln!("=== SUMMARY ===");
    eprintln!("  total_bytes = {total_bytes} ({:.2} KB)", total_bytes as f64 / 1024.0);
    eprintln!("  primitive_rows = {primitive_total_rows}");
    eprintln!("  npo_packed_rows = {npo_total_rows}");
    eprintln!("  grand_total_rows = {}", primitive_total_rows + npo_total_rows);
    eprintln!("  ext_degree = {}", l1.ext_degree);
    eprintln!("  alu_variant = {:?}", l1.alu_variant);
    eprintln!("  table_packing = {:?}", l1.table_packing);
    eprintln!();

    // ----- Per-section proof byte breakdown -----
    eprintln!("=== PROOF SECTION BYTES ===");
    let commitments_bytes = postcard::to_allocvec(&l1.proof.commitments).expect("ser").len();
    let opened_values_bytes = postcard::to_allocvec(&l1.proof.opened_values).expect("ser").len();
    let opening_proof_bytes = postcard::to_allocvec(&l1.proof.opening_proof).expect("ser").len();
    let global_lookup_data_bytes = postcard::to_allocvec(&l1.proof.global_lookup_data).expect("ser").len();
    let non_primitives_bytes = postcard::to_allocvec(&l1.non_primitives).expect("ser").len();
    eprintln!("  commitments:        {:>8} B ({:>5.1}%)", commitments_bytes, 100.0 * commitments_bytes as f64 / total_bytes as f64);
    eprintln!("  opened_values:      {:>8} B ({:>5.1}%)", opened_values_bytes, 100.0 * opened_values_bytes as f64 / total_bytes as f64);
    eprintln!("  opening_proof:      {:>8} B ({:>5.1}%)", opening_proof_bytes, 100.0 * opening_proof_bytes as f64 / total_bytes as f64);
    eprintln!("  global_lookup_data: {:>8} B ({:>5.1}%)", global_lookup_data_bytes, 100.0 * global_lookup_data_bytes as f64 / total_bytes as f64);
    eprintln!("  non_primitives:     {:>8} B ({:>5.1}%)", non_primitives_bytes, 100.0 * non_primitives_bytes as f64 / total_bytes as f64);
    eprintln!();
}
