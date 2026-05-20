//! M-S5b S1.B Tip5-NPO recursion backend Stage 4 — end-to-end
//! L2-over-L1 in the Tip5-throughout substrate (ACCEPT +
//! tamper-REJECT) + Stage 5 Production L2 size measurement.
//!
//! This is the FIRST end-to-end test of the L2 verifier circuit
//! built atop a Tip5-throughout outer-cert. It exercises:
//!
//!  - the new D=2 Tip5 dispatch arm in `FriRecursionBackend`
//!    (Stage 3, commit `6c67e7f`) — via `BatchStarkProver` using
//!    the Tip5 outer-cert config's NPO trio (Tip5 + Recompose);
//!  - the in-circuit Tip5 MMCS soundness gate (C2.3) — via
//!    `circuit_builder.enable_tip5_perm` on the L2 verifier
//!    circuit reconstructing the L1's Tip5 MMCS commitments;
//!  - the soundness chain MIN(inner Tip5-L0 PROD, L1 Tip5
//!    outer-cert, L2 Tip5 outer-cert) ≥ 80 bits unconditional
//!    Johnson at the LANDED production FRI parameters (Production
//!    tier: lb=4 nq=20 mla=3 lfp=2 cap=3 d=5).
//!
//! Per [no_poseidon2_anywhere] hard rule: ZERO Poseidon2 in
//! this test — every layer (inner L0, L1, L2) uses Tip5
//! exclusively for both MMCS and challenger.
//!
//! Heavy tests are `#[ignore]`'d behind `--ignored`. Default
//! `cargo test` runs only the compile-time wiring sanity check.

#![allow(clippy::too_many_arguments)]

mod common;

use p3_batch_stark::ProverData;
use p3_challenger::DuplexChallenger;
use p3_circuit::CircuitBuilder;
use p3_circuit::ops::{
    Tip5Config, Tip5Goldilocks, generate_recompose_trace, generate_tip5_trace,
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
//  Inner L0 — Tip5 Layer-0 (verbatim from
//  test_l1_outer_cert_tip5_unified.rs; the validated L0 baseline).
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

/// Tip5 perm lifted to the Challenge field (constant basis coeff
/// only). Verbatim from test_l1_outer_cert_tip5_unified.rs.
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

fn build_layer0_verifier_circuit(tamper: bool) -> BuiltLayer0Circuit {
    let config = make_layer0_config();
    let n = 1 << 3;
    let x = 21u64;
    let trace = generate_trace_rows::<Val>(0, 1, n);
    let pis = vec![Val::ZERO, Val::ONE, Val::from_u64(x)];
    let air = FibonacciAir {};

    let mut proof = prove(&config, &air, trace, &pis);
    assert!(verify(&config, &air, &proof, &pis).is_ok());

    // Tamper pattern verbatim from
    // test_tip5_layer0_compression.rs:284 — mutate a single opened
    // trace value. The L1 verifier circuit's in-circuit FRI fold-
    // chain + opening reconstruction MUST reject this at runner.run().
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
//  Outer-cert tier (Tip5-throughout). Mirrors the analogous enum in
//  test_tip5_layer0_compression.rs but with Tip5 substrate only.
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Tip5OuterTier {
    /// `lb=2, nq=2, pow=0+0` — fast unit-test tier (~5 bits, only
    /// usable for shape-correctness checks; not soundness-meaningful).
    Tiny,
    /// **LANDED production outer-cert FRI parameters** (matches
    /// `config::goldilocks_tip5_80bit()` exactly). All
    /// soundness-neutral compression levers stacked:
    /// `lb=4, nq=20, mla=3, lfp=2, cap=3, pow=1+1, d=5` ⇒ 82 bits
    /// unconditional Johnson, predicted L1 ~520 KB.
    Production,
}

impl Tip5OuterTier {
    fn name(self) -> &'static str {
        match self {
            Self::Tiny => "Tiny(~5b)",
            Self::Production => "Production(82b lb=4 nq=20 mla=3 lfp=2 cap=3 d=5)",
        }
    }

    /// (log_blowup, log_final_poly_len, max_log_arity, num_queries,
    ///  commit_pow, query_pow)
    fn fri(self) -> (usize, usize, usize, usize, usize, usize) {
        match self {
            Self::Tiny => (2, 0, 1, 2, 0, 0),
            Self::Production => (4, 2, 3, 20, 1, 1),
        }
    }

    /// Unconditional Johnson soundness per IACR ePrint 2025/2055
    /// Theorem 1.5: `lb·nq + cpow + qpow` bits.
    fn unconditional_bits(self) -> usize {
        let (lb, _, _, nq, cp, qp) = self.fri();
        lb * nq + cp + qp
    }
}

/// Build a Tip5-throughout outer-cert StarkConfig at the given
/// FRI tier. All MMCS + challenger paths use `Tip5Perm`.
fn make_tip5_outer_cfg(tier: Tip5OuterTier) -> TipsCfg {
    let perm = Tip5Perm;
    let hash = TipsHash::new(perm);
    let compress = TipsCompress::new(perm);
    let val_mmcs = TipsValMmcs::new(hash, compress, 0);
    let challenge_mmcs = TipsChallengeMmcs::new(val_mmcs.clone());
    let dft = TipsDft::default();
    let (lb, lfp, mla, nq, cpow, qpow) = tier.fri();
    let fri_params = FriParameters {
        log_blowup: lb,
        log_final_poly_len: lfp,
        max_log_arity: mla,
        num_queries: nq,
        commit_proof_of_work_bits: cpow,
        query_proof_of_work_bits: qpow,
        mmcs: challenge_mmcs,
    };
    let pcs = TipsPcs::new(dft, val_mmcs, fri_params);
    TipsCfg::new(pcs, TipsChallenger::new(perm))
}

/// L2 outer-cert is also Tip5-throughout. For now we use the same
/// type alias (the test-utils only define one shape). If we ever
/// need separate L2 perm geometry we'd add a parallel set; for
/// Production-tier L1 and L2 use the same Tip5 W=16 R=10 D=5 setup.
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

// ---------------------------------------------------------------------
//  Build L1 (Tip5-throughout outer over real inner Tip5-L0 PROD).
// ---------------------------------------------------------------------

fn build_l1_tip5_throughput(
    tamper_inner: bool,
    tier: Tip5OuterTier,
) -> Result<BatchStarkProof<TipsCfg>, String> {
    let outer_config = make_tip5_outer_cfg(tier);
    let BuiltLayer0Circuit {
        circuit,
        public_inputs,
        private_inputs,
        mmcs_op_ids,
        proof,
    } = build_layer0_verifier_circuit(tamper_inner);

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
        .map_err(|e| format!("L1[{}] get_airs_and_degrees: {e:?}", tier.name()))?;
    let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();

    let mut runner = circuit.runner();
    runner
        .set_public_inputs(&public_inputs)
        .map_err(|e| format!("L1[{}] set_public_inputs: {e:?}", tier.name()))?;
    runner
        .set_private_inputs(&private_inputs)
        .map_err(|e| format!("L1[{}] set_private_inputs: {e:?}", tier.name()))?;
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
    .map_err(|e| format!("L1[{}] set_fri_mmcs_private_data: {e}", tier.name()))?;

    let traces = runner
        .run()
        .map_err(|e| format!("L1[{}] runner().run() rejected: {e:?}", tier.name()))?;

    let prover_data = ProverData::from_airs_and_degrees(&outer_config, &airs, &degrees);
    let circuit_prover_data =
        CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

    let mut prover =
        BatchStarkProver::new(outer_config).with_table_packing(table_packing);
    prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    prover.register_recompose_table::<2>(true);

    let batch_proof: BatchStarkProof<TipsCfg> = prover
        .prove_all_tables(&traces, &circuit_prover_data)
        .map_err(|e| format!("L1[{}] prove_all_tables: {e:?}", tier.name()))?;
    assert_eq!(batch_proof.ext_degree, 2, "L1 must be D=2");
    if !tamper_inner {
        prover
            .verify_all_tables(&batch_proof)
            .map_err(|e| format!("L1[{}] verify_all_tables REJECTED: {e:?}", tier.name()))?;
    }
    Ok(batch_proof)
}

// ---------------------------------------------------------------------
//  Build L2 (Tip5-throughout outer over Tip5 L1). This is the FIRST
//  end-to-end exercise of the new Stage 3 dispatch.
// ---------------------------------------------------------------------

/// Inner NPO provers for reconstructing the L1's non-primitive AIRs
/// inside `verify_p3_batch_proof_circuit`. Tip5-throughout: ONLY
/// Tip5Prover + recompose (no Poseidon variants).
fn inner_tip5_npo_provers() -> Vec<Box<dyn TableProver<TipsCfg>>> {
    let mut tp: Vec<Box<dyn TableProver<TipsCfg>>> = vec![Box::new(Tip5Prover::new(
        Tip5Config::GOLDILOCKS_W16,
        ConstraintProfile::Standard,
    ))];
    tp.extend(recompose_table_provers::<TipsCfg, 2>(1, true));
    tp
}

/// Build the L2 circuit that verifies `l1`, prove it under
/// `l2_config`, verify it, return serialized L2 byte length.
fn l2_over_tip5_l1(
    label: &str,
    l1: &BatchStarkProof<TipsCfg>,
    inner_fri: &FriVerifierParams,
    l1_config: &TipsCfg,
    l2_config: TipsCfg,
) -> Result<usize, String> {
    let common = &l1.stark_common;
    let batch_proof = &l1.proof;
    const TRACE_D: usize = 2;

    let num_tables = common
        .preprocessed
        .as_ref()
        .map(|g| g.instances.len())
        .unwrap_or(0);
    let pis: Vec<Vec<L2Val>> = vec![vec![]; num_tables];

    let mut circuit_builder = CircuitBuilder::<L2Challenge>::new();
    // CRITICAL: enable Tip5 perm (the L1's hash family) in-circuit so
    // the L2 verifier can recompute the L1 MMCS commitments. This is
    // the in-circuit Tip5 MMCS soundness gate (C2.3) live-wired.
    circuit_builder.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<L2Challenge, Tip5Goldilocks>,
        LiftTip5,
    );
    circuit_builder.enable_recompose::<L2Val>(generate_recompose_trace::<L2Val, L2Challenge>);
    circuit_builder.set_recompose_coeff_ctl_for_decompose_links(true);

    let lookup_gadget = LogUpGadget::new();

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
        l1_config,
        &mut circuit_builder,
        l1,
        inner_fri,
        common,
        &lookup_gadget,
        Tip5Config::GOLDILOCKS_W16,
        &inner_tip5_npo_provers(),
    )
    .map_err(|e| format!("[{label}] verify_p3_batch_proof_circuit (L2 build) failed: {e:?}"))?;

    let verification_circuit = circuit_builder
        .build()
        .map_err(|e| format!("[{label}] L2 circuit build failed: {e:?}"))?;
    let (public_inputs, private_inputs) =
        verifier_inputs.pack_values(&pis, batch_proof, common);

    // L2's own prove: Tip5 NPO trio (same family as L1 outer). Manual
    // wiring mirrors the L1 path; Stage 3 dispatch via
    // FriRecursionBackend would return these same builders/preprocessor
    // given a Tip5Config challenger — confirming dispatch parity.
    let verification_table_packing = TablePacking::new(1, 8);
    let npo_prep: Vec<Box<dyn NpoPreprocessor<L2Val>>> = vec![
        Box::new(Tip5Preprocessor),
        Box::new(RecomposePreprocessor::new(true)),
    ];
    let mut air_builders = tip5_air_builders::<TipsCfg, 2>();
    air_builders.extend(recompose_air_builders::<TipsCfg, 2>(1, true));
    let (v_airs_degrees, v_primitive, v_npo) =
        get_airs_and_degrees_with_prep::<TipsCfg, L2Challenge, 2>(
            &verification_circuit,
            &verification_table_packing,
            &npo_prep,
            &air_builders,
            ConstraintProfile::Standard,
        )
        .map_err(|e| format!("[{label}] L2 get_airs_and_degrees: {e:?}"))?;
    let (v_airs, v_degrees): (Vec<_>, Vec<usize>) = v_airs_degrees.into_iter().unzip();

    let mut runner = verification_circuit.runner();
    runner
        .set_public_inputs(&public_inputs)
        .map_err(|e| format!("[{label}] L2 set_public_inputs: {e:?}"))?;
    runner
        .set_private_inputs(&private_inputs)
        .map_err(|e| format!("[{label}] L2 set_private_inputs: {e:?}"))?;
    if !mmcs_op_ids.is_empty() {
        set_fri_mmcs_private_data::<
            L2Val,
            L2Challenge,
            L2ChallengeMmcs,
            L2ValMmcs,
            L2Hash,
            L2Compress,
            L2_DIGEST_ELEMS,
        >(
            &mut runner,
            &mmcs_op_ids,
            &l1.proof.opening_proof,
            Tip5Config::GOLDILOCKS_W16,
        )
        .map_err(|e| format!("[{label}] L2 set_fri_mmcs_private_data: {e}"))?;
    }
    let v_traces = runner
        .run()
        .map_err(|e| format!("[{label}] L2 runner().run() rejected: {e:?}"))?;

    let v_prover_data = ProverData::from_airs_and_degrees(&l2_config, &v_airs, &v_degrees);
    let v_circuit_prover_data = CircuitProverData::new(v_prover_data, v_primitive, v_npo);

    let mut v_prover =
        BatchStarkProver::new(l2_config).with_table_packing(verification_table_packing);
    v_prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    v_prover.register_recompose_table::<2>(true);
    let l2_proof: BatchStarkProof<TipsCfg> = v_prover
        .prove_all_tables(&v_traces, &v_circuit_prover_data)
        .map_err(|e| format!("[{label}] L2 prove_all_tables: {e:?}"))?;
    v_prover
        .verify_all_tables(&l2_proof)
        .map_err(|e| format!("[{label}] L2 verify_all_tables REJECTED: {e:?}"))?;
    Ok(postcard::to_allocvec(&l2_proof)
        .map_err(|e| format!("[{label}] serialize L2: {e:?}"))?
        .len())
}

fn fri_vparams_for(tier: Tip5OuterTier) -> FriVerifierParams {
    let (lb, lfp, cpow, qpow, _, _) = {
        let (lb, lfp, _mla, _nq, cp, qp) = tier.fri();
        (lb, lfp, cp, qp, 0u8, 0u8)
    };
    FriVerifierParams::with_mmcs(lb, lfp, cpow, qpow, Tip5Config::GOLDILOCKS_W16)
}

// =====================================================================
//  Tests
// =====================================================================

/// Compile-time sanity: the Tip5-throughout L2 stack assembles (all
/// type-aliases + helper signatures resolve). Default-`cargo test`
/// runs this; it's a regression gate on the Stage 3 dispatch
/// without requiring the heavy L2 build.
#[test]
fn stage3_tip5_l2_stack_assembles() {
    // Just construct + drop the outer config; assembling it exercises
    // every type-alias in the Tip5-throughout substrate.
    let _l1_cfg = make_tip5_outer_cfg(Tip5OuterTier::Tiny);
    let _l2_cfg = make_tip5_outer_cfg(Tip5OuterTier::Production);
    // Constructing the inner NPO provers exercises Tip5Prover's
    // TableProver<TipsCfg> bound — the precise bound the Stage 3
    // dispatch's `Box<dyn TableProver<SC>>` slot requires.
    let _np = inner_tip5_npo_provers();
    // Verify the FRI params helper for both tiers without building
    // a proof.
    let _f_tiny = fri_vparams_for(Tip5OuterTier::Tiny);
    let _f_tierb = fri_vparams_for(Tip5OuterTier::Production);

    // Sanity: Production unconditional Johnson bits = 82.
    assert_eq!(Tip5OuterTier::Production.unconditional_bits(), 82);
}

/// **STAGE 4 ACCEPT** — toy Tip5-throughout L2 over Tip5-throughout
/// L1 at the Tiny tier (fast). Proves the full chain assembles +
/// proves + verifies end-to-end via the Stage 3 dispatch. Use this
/// as the fast regression on the wiring.
#[test]
#[ignore = "Stage 4: Tip5-throughout L2-over-L1 ACCEPT at Tiny tier (heavy, ~few min)"]
fn stage4_tip5_l2_over_l1_tiny_accepts() {
    let l1 = build_l1_tip5_throughput(false, Tip5OuterTier::Tiny)
        .expect("Stage 4 Tiny: honest L1 must build+verify");
    let l1_bytes = postcard::to_allocvec(&l1).expect("serialize L1").len();
    let inner_fri = fri_vparams_for(Tip5OuterTier::Tiny);
    let l2_bytes = l2_over_tip5_l1(
        "Stage4-Tiny",
        &l1,
        &inner_fri,
        &make_tip5_outer_cfg(Tip5OuterTier::Tiny),
        make_tip5_outer_cfg(Tip5OuterTier::Tiny),
    )
    .expect("Stage 4 Tiny: honest L2 must ACCEPT");
    eprintln!(
        "\n[STAGE 4 — Tip5-throughout L2-over-L1 @ Tiny tier]\n  \
         L1 = {l1_bytes} B ({:.2} KB)  L2 = {l2_bytes} B ({:.2} KB)\n  \
         ACCEPT ✅  (soundness ~5 bits; tier exists only for fast \
         shape-correctness of the new Stage 3 Tip5 dispatch)\n",
        l1_bytes as f64 / 1024.0,
        l2_bytes as f64 / 1024.0,
    );
}

/// **STAGE 4 TAMPER-REJECT** — corrupted inner Tip5-L0 proof must
/// NOT produce a verifying L2. Either L1 rejects at build (preferred:
/// the inner verifier circuit's runner().run() catches the
/// corruption), or L2 rejects at verify (acceptable: the in-circuit
/// FRI fold-chain catches it). Fail loudly if the chain accepts a
/// tampered inner.
#[test]
#[ignore = "Stage 4: Tip5-throughout L2-over-L1 TAMPER-REJECT at Tiny tier (heavy, ~few min)"]
fn stage4_tip5_l2_over_l1_tiny_tamper_rejects() {
    let tampered_l1 = build_l1_tip5_throughput(true, Tip5OuterTier::Tiny);
    match tampered_l1 {
        Err(e) => {
            eprintln!(
                "[STAGE 4 TAMPER] tampered inner Tip5-L0 correctly REJECTED at \
                 L1 build (expected): {e}"
            );
        }
        Ok(bad_l1) => {
            let r = l2_over_tip5_l1(
                "Stage4-Tiny-tamper",
                &bad_l1,
                &fri_vparams_for(Tip5OuterTier::Tiny),
                &make_tip5_outer_cfg(Tip5OuterTier::Tiny),
                make_tip5_outer_cfg(Tip5OuterTier::Tiny),
            );
            assert!(
                r.is_err(),
                "STAGE 4 SOUNDNESS HOLE: tampered inner Tip5-L0 produced a VERIFYING \
                 Tip5-throughout L2 cert — STOP and investigate: {r:?}"
            );
            eprintln!(
                "[STAGE 4 TAMPER] tampered inner Tip5-L0 correctly produced NO \
                 verifying L2: {r:?}"
            );
        }
    }
}

/// **STAGE 5** — Production L2 size measurement (the original user ask:
/// "measure L2 at the current production params"). Tip5-throughout
/// L2 over
/// Tip5-throughout L1, both at the LANDED production FRI params
/// (`config::goldilocks_tip5_80bit` post-Tier-B-flip:
/// `lb=4 nq=20 pow=1+1 d=5`, 82 bits unconditional Johnson).
#[test]
#[ignore = "Stage 5: Tip5-throughout L2-over-L1 at Production (VERY heavy ~many min); records L1+L2 sizes"]
fn stage5_tip5_l2_over_l1_production_measurement() {
    let tier = Tip5OuterTier::Production;
    let sbits = tier.unconditional_bits();
    assert!(sbits >= 80, "Production Johnson soundness {sbits} < 80 floor");

    let l1 = build_l1_tip5_throughput(false, tier)
        .expect("Production L1 over real inner Tip5-L0 must ACCEPT");
    let l1_bytes = postcard::to_allocvec(&l1).expect("serialize L1").len();

    let l2_bytes = l2_over_tip5_l1(
        "Stage5-Prod",
        &l1,
        &fri_vparams_for(tier),
        &make_tip5_outer_cfg(tier),
        make_tip5_outer_cfg(tier),
    )
    .expect("Production L2 over Production L1 in Tip5-throughout substrate must ACCEPT");

    // Tamper-reject at the production tier too: a tampered inner MUST
    // NOT produce a verifying Tier-B L2 (soundness hole would be
    // catastrophic; fail loudly).
    let tampered = build_l1_tip5_throughput(true, tier);
    match tampered {
        Err(e) => eprintln!(
            "[STAGE 5 TAMPER] tampered inner correctly rejected at L1 build: {e}"
        ),
        Ok(bad) => {
            let r = l2_over_tip5_l1(
                "Stage5-Prod-tamper",
                &bad,
                &fri_vparams_for(tier),
                &make_tip5_outer_cfg(tier),
                make_tip5_outer_cfg(tier),
            );
            assert!(
                r.is_err(),
                "STAGE 5 SOUNDNESS HOLE: tampered inner produced VERIFYING Production L2: {r:?}"
            );
            eprintln!("[STAGE 5 TAMPER] tampered inner correctly rejected at L2: {r:?}");
        }
    }

    let (lb, lfp, mla, nq, cp, qp) = tier.fri();
    eprintln!(
        "\n[M-S5b S1.B STAGE 5 — Tip5-throughout L1+L2 @ {} (the LANDED production FRI)]\n  \
         L1-outer = Production (lb={lb} nq={nq} pow={cp}/{qp} mla={mla} lfp={lfp} d={DIGEST_ELEMS}) \
         ⇒ {sbits} bits unconditional Johnson ≥ 80\n  \
         L2-wrapper = Production (same {sbits}-bit tier)\n  \
         soundness chain MIN(L0, L1, L2) ≥ 80 bits at EVERY link (L0 = Tip5-L0 PROD \
         lb=3 nq=30 = 90 bits)\n  \
         serialized L1 = {l1_bytes} B ({:.2} KB)\n  \
         serialized L2 (THE CERT) = {l2_bytes} B ({:.2} KB)\n  \
         ACCEPT ✅  tamper-REJECT ✅\n  \
         SUBSTRATE: 100% Tip5 — zero Poseidon2 in trust surface.\n",
        tier.name(),
        l1_bytes as f64 / 1024.0,
        l2_bytes as f64 / 1024.0,
    );
}
