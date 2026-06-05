//! §recursion — integrate the ai-pow-zk composite proof with the
//! vendored `Plonky3-recursion` substrate.
//!
//! Feature-gated behind `recursion`. This module is the *caller* side
//! of a generic API: `p3_recursion`'s verifier entrypoints are generic
//! over the inner AIR, and here they are instantiated with the
//! concrete `CompositeFullAirWithLookupsPinned` + `AiPowStarkConfig`.
//! The recursion substrate stays application-agnostic.
//!
//! Staging:
//! - S2 — cross-workspace build path established.
//! - S3a — composite AIR confirmed `RecursiveAir`-conformant.
//! - S3b/c — `build_composite_l1_verifier_circuit`: the composite's
//!   batch-STARK proof is verified in-circuit by `verify_batch_circuit`.
//!   The composite is a single LogUp AIR proven by `p3_batch_stark`, so
//!   it routes through the lookup-aware batch entrypoint with the
//!   composite AIR as the single generic `A` (the de-risk's path 3a).

use p3_batch_stark::{BatchProof, CommonData};
use p3_circuit::ops::{generate_recompose_trace, generate_tip5_trace, Tip5Config, Tip5Goldilocks};
use p3_circuit::{CircuitBuilder, NonPrimitiveOpId};
use p3_field::{BasedVectorSpace, PrimeCharacteristicRing};
use p3_lookup::logup::LogUpGadget;
use p3_recursion::pcs::fri::{
    FriProofTargets, FriVerifierParams, InputProofTargets, MerkleCapTargets, RecExtensionValMmcs,
    RecValMmcs, Witness,
};
use p3_recursion::pcs::set_fri_mmcs_private_data;
use p3_recursion::public_inputs::BatchStarkVerifierInputsBuilder;
use p3_recursion::terminal::{NativeTerminalCompiler, NativeTerminalVerifyError};
use p3_recursion::{
    verify_batch_circuit, RecursiveAir, TerminalCertificate, TerminalCircuitFingerprint,
    TerminalProofParameters, TerminalWitness, VerificationError,
};
use p3_symmetric::Permutation;
use p3_tip5_circuit_air::Tip5Perm as RecTip5Perm;
use serde::{Deserialize, Serialize};

use crate::circuit::{Challenge, Tip5Compress, Tip5Sponge};
use crate::{AiPowStarkConfig, CompositeFullAirWithLookupsPinned, Val};

/// Outer circuit-prover proof produced after recursively verifying Layer 0.
type AiPowL1OuterProof =
    p3_circuit_prover::BatchStarkProof<p3_circuit_prover::config::GoldilocksTipsConfig>;

/// Native terminal recursive proof artifact for the AI-PoW L1 verifier
/// circuit.
///
/// The terminal certificate is the size-bounded production target. It proves
/// execution of the same L1 verifier circuit used by the batch-STARK
/// checkpoint, but the certificate itself binds only a digest of the terminal
/// public input vector. Callers must preserve and verify `terminal_public_inputs`
/// with the certificate; otherwise the terminal binding is incomplete.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiPowTerminalRecursiveCertificate {
    /// Public inputs supplied to the terminal verifier circuit. This vector is
    /// bound by `terminal_certificate.public_values_digest`.
    terminal_public_inputs: Vec<Challenge>,
    /// Native terminal production certificate for the L1 verifier circuit.
    terminal_certificate: TerminalCertificate,
}

impl AiPowTerminalRecursiveCertificate {
    pub fn new(
        terminal_public_inputs: Vec<Challenge>,
        terminal_certificate: TerminalCertificate,
    ) -> Self {
        Self {
            terminal_public_inputs,
            terminal_certificate,
        }
    }

    pub fn terminal_public_inputs(&self) -> &[Challenge] {
        &self.terminal_public_inputs
    }

    pub fn terminal_certificate(&self) -> &TerminalCertificate {
        &self.terminal_certificate
    }
}

/// Canonical recursive certificate for Nockchain's AI proof-of-work puzzle
/// statement.
///
/// The outer proof alone is not a production certificate: its verifier would
/// otherwise trust proof-carried circuit metadata. The canonical certificate
/// carries the Layer-0 proof and pinned program so verification can rebuild the
/// exact L1 verifier circuit, run that verifier against the embedded Layer-0
/// proof, reject outer proof metadata that does not match the rebuilt canonical
/// circuit shape, and cryptographically verify the submitted outer proof body.
///
/// Consensus code must still derive and check the statement metadata
/// externally before accepting this certificate.
#[derive(Serialize, Deserialize)]
pub struct AiPowRecursiveCertificate {
    /// Layer-0 pinned LogUp proof recursively verified by the L1 circuit.
    l0_proof: BatchProof<AiPowStarkConfig>,
    /// Canonical pinned Layer-0 program used to rebuild the L1 verifier
    /// circuit and its expected outer proof binding.
    l0_program: crate::AiPowProgram,
    /// Outer D=2 circuit-prover proof of the L1 verifier circuit execution.
    l1_outer_proof: AiPowL1OuterProof,
}

impl AiPowRecursiveCertificate {
    /// Construct the batch-STARK recursive checkpoint certificate from
    /// chain-verified Layer-0 proof parts and the corresponding L1 outer proof.
    fn new(
        l0_proof: BatchProof<AiPowStarkConfig>,
        l0_program: crate::AiPowProgram,
        l1_outer_proof: AiPowL1OuterProof,
    ) -> Self {
        Self {
            l0_proof,
            l0_program,
            l1_outer_proof,
        }
    }

    /// The outer proof, exposed for diagnostics and size accounting only.
    ///
    /// Production verification must call [`verify_recursive_certificate`], which
    /// rebuilds and runs the canonical L1 verifier circuit, checks this proof's
    /// stable circuit metadata, and verifies the submitted proof body.
    pub fn l1_outer_proof(&self) -> &AiPowL1OuterProof {
        &self.l1_outer_proof
    }

    /// The embedded Layer-0 proof, exposed for diagnostics and size accounting
    /// only.
    ///
    /// Production verification must call [`verify_recursive_certificate`], which
    /// verifies this proof inside the rebuilt L1 verifier circuit.
    pub fn l0_proof(&self) -> &BatchProof<AiPowStarkConfig> {
        &self.l0_proof
    }
}

/// Tip5 digest width (`DIGEST_ELEMS`), sponge `WIDTH`, sponge `RATE` —
/// the ai-pow-zk Layer-0 MMCS parameters (`circuit.rs`).
const DIGEST_ELEMS: usize = 5;
const WIDTH: usize = 16;
const RATE: usize = 10;

fn production_l1_table_packing(_public_value_count: usize) -> p3_circuit_prover::TablePacking {
    p3_circuit_prover::TablePacking::new(DIGEST_ELEMS, 8)
        .with_public_binding_lanes(0)
        .with_horner_pack_k(5)
}

fn production_l1_stark_config() -> p3_circuit_prover::config::GoldilocksTipsConfig {
    p3_circuit_prover::config::goldilocks_tip5_60bit()
}

fn statement_public_digest(public_values: &[Val]) -> Vec<Val> {
    let mut state = [Val::ZERO; WIDTH];
    for chunk in public_values.chunks(RATE) {
        for i in 0..RATE {
            state[i] = chunk.get(i).copied().unwrap_or(Val::ZERO);
        }
        state = RecTip5Perm.permute(state);
    }
    state[..DIGEST_ELEMS].to_vec()
}

fn non_primitive_metadata_eq(
    left: &[p3_circuit_prover::NonPrimitiveTableEntry<
        p3_circuit_prover::config::GoldilocksTipsConfig,
    >],
    right: &[p3_circuit_prover::NonPrimitiveTableEntry<
        p3_circuit_prover::config::GoldilocksTipsConfig,
    >],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.op_type == right.op_type
                && left.rows == right.rows
                && left.lanes == right.lanes
                && left.public_values == right.public_values
                && left.air_variant == right.air_variant
        })
}

/// The recursion `OpeningProof` target type for ai-pow-zk's Layer-0
/// `TwoAdicFriPcs` (the `InnerFriGeneric` alias from the recursion test
/// suite, instantiated with ai-pow-zk's own MMCS hash/compress).
type InnerFri = FriProofTargets<
    Val,
    Challenge,
    RecExtensionValMmcs<
        Val,
        Challenge,
        DIGEST_ELEMS,
        RecValMmcs<Val, DIGEST_ELEMS, Tip5Sponge, Tip5Compress>,
    >,
    InputProofTargets<Val, Challenge, RecValMmcs<Val, DIGEST_ELEMS, Tip5Sponge, Tip5Compress>>,
    Witness<Val>,
>;

/// The recursion `Comm`/commitment target type.
type CompositeComm = MerkleCapTargets<Val, DIGEST_ELEMS>;
/// The recursion `InputProof` target type.
type CompositeInputProof =
    InputProofTargets<Val, Challenge, RecValMmcs<Val, DIGEST_ELEMS, Tip5Sponge, Tip5Compress>>;

/// `Tip5Perm` lifted to act on `Challenge` (`BinomialExtensionField<
/// Goldilocks, 2>`) lanes — reads each lane's constant basis
/// coefficient, runs the base-field scalar Tip5 permutation, and
/// re-embeds with only the constant coefficient set. This is the
/// in-circuit-challenger counterpart of ai-pow-zk's native
/// `DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>`; the in-circuit
/// Tip5 NPO witnesses exactly this. It uses the recursion's
/// `p3_tip5_circuit_air::Tip5Perm`, which is KAT-anchored byte-for-byte
/// to `nockchain_math::tip5::permute_5round` (the permutation ai-pow-zk's
/// native `Tip5Perm` wraps), so the in-circuit transcript matches the
/// native proof's transcript.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiftTip5;

impl Permutation<[Challenge; 16]> for LiftTip5 {
    fn permute(&self, input: [Challenge; 16]) -> [Challenge; 16] {
        let bases: [Val; 16] = core::array::from_fn(|i| {
            <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(&input[i])[0]
        });
        let out = RecTip5Perm.permute(bases);
        core::array::from_fn(|i| {
            <Challenge as BasedVectorSpace<Val>>::from_basis_coefficients_fn(|j| {
                if j == 0 {
                    out[i]
                } else {
                    Val::ZERO
                }
            })
        })
    }

    fn permute_mut(&self, input: &mut [Challenge; 16]) {
        *input = Permutation::permute(self, *input);
    }
}

/// A fully-built L1 verifier circuit for a composite proof, plus
/// everything needed to run it.
pub struct BuiltCompositeL1 {
    /// The L1 verifier circuit (proves "I verified the composite proof").
    pub circuit: p3_circuit::Circuit<Challenge>,
    /// Layer-0 AI-PoW statement values that are exposed and bound by the L1
    /// outer certificate.
    pub statement_public_values: Vec<Val>,
    /// Public inputs for the runner.
    pub public_inputs: Vec<Challenge>,
    /// Private inputs for the runner (opened values etc.).
    pub private_inputs: Vec<Challenge>,
    /// MMCS op ids needing FRI Merkle sibling private data.
    pub mmcs_op_ids: Vec<NonPrimitiveOpId>,
}

/// S3b/c — build the L1 recursive-verification circuit for a composite
/// `BatchProof`.
///
/// The composite (`CompositeFullAirWithLookupsPinned`) is a single
/// LogUp AIR proven by `p3_batch_stark::prove_batch`; its proof is a
/// bare `p3_batch_stark::BatchProof`. It is verified in-circuit by
/// `verify_batch_circuit` with the composite AIR as the single generic
/// `A` (vs the circuit-prover multi-table path of
/// `verify_p3_batch_proof_circuit`).
///
/// `profile` MUST be the same `CircuitConfig` the composite proof was
/// produced under: the L1 verifier circuit's FRI parameters
/// (`log_blowup`, `commit/query_pow_bits`) are derived from it and
/// must match the proof's transcript exactly, or the in-circuit
/// challenger desynchronizes. (`num_queries` is intrinsic to the
/// proof shape and need not be threaded.)
pub fn build_composite_l1_verifier_circuit(
    config: &AiPowStarkConfig,
    composite_air: &CompositeFullAirWithLookupsPinned,
    proof: &BatchProof<AiPowStarkConfig>,
    common_data: &CommonData<AiPowStarkConfig>,
    public_values: &[Val],
    profile: &crate::circuit::CircuitConfig,
) -> Result<BuiltCompositeL1, VerificationError> {
    let mut cb = CircuitBuilder::<Challenge>::new();
    // In-circuit Tip5 permutation NPO + the recompose link (mirror of
    // the validated Layer-0 verifier circuit, `test_tip5_layer0_
    // recursion.rs`).
    cb.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<Challenge, Tip5Goldilocks>, LiftTip5,
    );
    cb.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
    cb.set_recompose_coeff_ctl_for_decompose_links(true);

    // ai-pow-zk Layer-0 FRI verifier params — derived from the same
    // `CircuitConfig` `build_stark_config` used to prove the
    // composite. This mapping MUST mirror `build_stark_config`:
    // `log_final_poly_len = 0` (fixed there), and BOTH the commit-
    // and query-phase PoW tiers take `config.pow_bits`. Hard-coding
    // the PoW bits to 0 (as an earlier revision did) desynchronizes
    // the in-circuit challenger from any `pow_bits > 0` proof —
    // `check_pow_witness` early-returns at 0 bits, skipping the
    // observe+sample the prover's transcript performed.
    let fri_verifier_params = FriVerifierParams::with_mmcs(
        profile.log_blowup as usize,
        0,
        profile.pow_bits as usize,
        profile.pow_bits as usize,
        Tip5Config::GOLDILOCKS_W16,
    );

    // The composite is a single AIR instance.
    let air_public_counts = [public_values.len()];

    let statement_digest_targets = cb.alloc_public_inputs(DIGEST_ELEMS, "statement digest");

    let verifier_inputs =
        BatchStarkVerifierInputsBuilder::<AiPowStarkConfig, CompositeComm, InnerFri>::allocate(
            &mut cb, proof, common_data, &air_public_counts,
        );

    let mmcs_op_ids = verify_batch_circuit::<
        CompositeFullAirWithLookupsPinned,
        AiPowStarkConfig,
        CompositeComm,
        CompositeInputProof,
        InnerFri,
        LogUpGadget,
        Tip5Config,
        WIDTH,
        RATE,
    >(
        config,
        core::slice::from_ref(composite_air),
        &mut cb,
        &verifier_inputs.proof_targets,
        &verifier_inputs.air_public_targets,
        &fri_verifier_params,
        &verifier_inputs.common_data,
        &LogUpGadget,
        Tip5Config::GOLDILOCKS_W16,
    )?;

    let mut digest_state = [None; WIDTH];
    for (block_idx, chunk) in verifier_inputs.air_public_targets[0]
        .chunks(RATE)
        .enumerate()
    {
        let mut inputs = [None; WIDTH];
        for i in 0..RATE {
            inputs[i] = Some(chunk.get(i).copied().unwrap_or(p3_circuit::ExprId::ZERO));
        }
        let outputs = cb.add_tip5_perm_for_challenger_base(
            Tip5Config::GOLDILOCKS_W16,
            block_idx == 0,
            inputs,
        )?;
        digest_state = outputs.map(Some);
    }
    for (target, digest_limb) in statement_digest_targets
        .iter()
        .zip(digest_state.iter().take(DIGEST_ELEMS))
    {
        cb.connect(
            *target,
            digest_limb.expect("statement digest limb must exist"),
        );
    }

    let circuit = cb.build()?;
    let statement_public_values = statement_public_digest(public_values);
    let (verifier_public_inputs, private_inputs) =
        verifier_inputs.pack_values(&[public_values.to_vec()], proof, common_data);
    let mut public_inputs = statement_public_values
        .iter()
        .copied()
        .map(Challenge::from)
        .collect::<Vec<_>>();
    public_inputs.extend(verifier_public_inputs);

    Ok(BuiltCompositeL1 {
        circuit,
        statement_public_values,
        public_inputs,
        private_inputs,
        mmcs_op_ids,
    })
}

/// Run a built composite-L1 verifier circuit against the composite
/// proof's FRI opening data. `Ok(())` iff the in-circuit verification
/// accepts.
pub fn run_composite_l1_verifier(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
) -> Result<(), VerificationError> {
    run_composite_l1_verifier_traces(built, proof)?;
    Ok(())
}

fn run_composite_l1_verifier_traces(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
) -> Result<p3_circuit::tables::Traces<Challenge>, VerificationError> {
    let mut runner = built.circuit.runner();
    runner
        .set_public_inputs(&built.public_inputs)
        .map_err(VerificationError::Circuit)?;
    runner
        .set_private_inputs(&built.private_inputs)
        .map_err(VerificationError::Circuit)?;
    set_fri_mmcs_private_data::<
        Val,
        Challenge,
        crate::circuit::ChallengeMmcs,
        crate::circuit::ValMmcs,
        Tip5Sponge,
        Tip5Compress,
        DIGEST_ELEMS,
    >(
        &mut runner,
        &built.mmcs_op_ids,
        &proof.opening_proof,
        Tip5Config::GOLDILOCKS_W16,
    )
    .map_err(|e| VerificationError::InvalidProofShape(e.to_string()))?;
    runner.run().map_err(VerificationError::Circuit)
}

fn production_l1_circuit_prover_data(
    built: &BuiltCompositeL1,
) -> Result<
    (
        p3_circuit_prover::TablePacking,
        p3_circuit_prover::CircuitProverData<p3_circuit_prover::config::GoldilocksTipsConfig>,
    ),
    VerificationError,
> {
    use p3_batch_stark::ProverData;
    use p3_circuit_prover::common::{get_airs_and_degrees_with_prep, NpoPreprocessor};
    use p3_circuit_prover::{
        config, recompose_air_builders, strip_public_binding_for_lookup_metadata,
        tip5_air_builders, CircuitProverData, ConstraintProfile, RecomposePreprocessor,
        Tip5Preprocessor,
    };

    type OuterConfig = config::GoldilocksTipsConfig;

    let public_binding_lanes = built.public_inputs.len();
    let table_packing = production_l1_table_packing(public_binding_lanes);
    let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> =
        vec![Box::new(Tip5Preprocessor), Box::new(RecomposePreprocessor::new(true))];
    let mut air_builders = tip5_air_builders::<OuterConfig, 2>();
    air_builders.extend(recompose_air_builders::<OuterConfig, 2>(1, true));

    let (airs_degrees, primitive_columns, non_primitive_columns) =
        get_airs_and_degrees_with_prep::<OuterConfig, Challenge, 2>(
            &built.circuit,
            &table_packing,
            &npo_prep,
            &air_builders,
            ConstraintProfile::Standard,
        )
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "composite L1 outer cert — get_airs_and_degrees: {e:?}"
            ))
        })?;
    let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();

    let lookup_metadata_airs = airs
        .iter()
        .map(strip_public_binding_for_lookup_metadata)
        .collect::<Vec<_>>();
    let outer_config = production_l1_stark_config();
    let prover_data =
        ProverData::from_airs_and_degrees(&outer_config, &lookup_metadata_airs, &degrees);
    Ok((
        table_packing,
        CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns),
    ))
}

/// S5 — produce the **L1 outer certificate** for a composite proof:
/// prove the composite-L1 verifier circuit itself as a D=2 batch-STARK
/// (`prove_all_tables`). This is the outer recursive proof object for the
/// statement "I verified the composite proof".
///
/// Mirrors the validated `outer_cert_layer0` machinery
/// (`Plonky3-recursion` `test_tip5_layer0_recursion.rs`) — D=2,
/// Tip5 NPO (D=1 perm) + recompose with split coeff tables — with the
/// composite-L1 circuit in place of the Fibonacci-L0 one.
///
/// Returns the L1 outer proof on accept; an `Err` if the L1 verifier circuit
/// runner rejects before outer proving.
pub fn prove_composite_l1_outer_cert(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
) -> Result<AiPowL1OuterProof, VerificationError> {
    use p3_batch_stark::ProverData;
    use p3_circuit_prover::common::{get_airs_and_degrees_with_prep, NpoPreprocessor};
    use p3_circuit_prover::{
        config, recompose_air_builders, strip_public_binding_for_lookup_metadata,
        tip5_air_builders, BatchStarkProver, CircuitProverData, ConstraintProfile,
        RecomposePreprocessor, Tip5Preprocessor,
    };

    type OuterConfig = config::GoldilocksTipsConfig;

    // D=2 outer-cert table layout — Tip5 NPO (D=1 perm) + recompose
    // with split coeff tables (the verifier circuit set
    // `set_recompose_coeff_ctl_for_decompose_links(true)`).
    let public_binding_lanes = built.public_inputs.len();
    let table_packing = production_l1_table_packing(public_binding_lanes);
    let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> =
        vec![Box::new(Tip5Preprocessor), Box::new(RecomposePreprocessor::new(true))];
    let mut air_builders = tip5_air_builders::<OuterConfig, 2>();
    air_builders.extend(recompose_air_builders::<OuterConfig, 2>(1, true));

    let (airs_degrees, primitive_columns, non_primitive_columns) =
        get_airs_and_degrees_with_prep::<OuterConfig, Challenge, 2>(
            &built.circuit,
            &table_packing,
            &npo_prep,
            &air_builders,
            ConstraintProfile::Standard,
        )
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "composite L1 outer cert — get_airs_and_degrees: {e:?}"
            ))
        })?;
    let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();

    // Run the verifier circuit to obtain its execution traces.
    let mut runner = built.circuit.runner();
    runner
        .set_public_inputs(&built.public_inputs)
        .map_err(VerificationError::Circuit)?;
    runner
        .set_private_inputs(&built.private_inputs)
        .map_err(VerificationError::Circuit)?;
    set_fri_mmcs_private_data::<
        Val,
        Challenge,
        crate::circuit::ChallengeMmcs,
        crate::circuit::ValMmcs,
        Tip5Sponge,
        Tip5Compress,
        DIGEST_ELEMS,
    >(
        &mut runner,
        &built.mmcs_op_ids,
        &proof.opening_proof,
        Tip5Config::GOLDILOCKS_W16,
    )
    .map_err(|e| VerificationError::InvalidProofShape(e.to_string()))?;
    let traces = runner.run().map_err(VerificationError::Circuit)?;

    // Prove the verifier circuit as a D=2 batch-STARK.
    let lookup_metadata_airs = airs
        .iter()
        .map(strip_public_binding_for_lookup_metadata)
        .collect::<Vec<_>>();
    let outer_config = production_l1_stark_config();
    let prover_data =
        ProverData::from_airs_and_degrees(&outer_config, &lookup_metadata_airs, &degrees);
    let circuit_prover_data =
        CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);
    let mut prover = BatchStarkProver::new(outer_config).with_table_packing(table_packing);
    prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    prover.register_recompose_table::<2>(true);

    let batch_proof = prover
        .prove_all_tables(&traces, &circuit_prover_data)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "composite L1 outer cert — prove_all_tables: {e:?}"
            ))
        })?;
    Ok(batch_proof)
}

/// Verify the batch-STARK recursive checkpoint certificate against the
/// verifier-derived Layer-0 AI-PoW public inputs and chain-pinned proving
/// parameters.
///
/// This is the hardened batch-STARK checkpoint verifier. It rejects outer
/// proofs whose circuit-prover metadata is merely self-consistent by rebuilding
/// the canonical L1 verifier circuit from the certificate's Layer-0
/// proof/program, running that circuit against the verifier-derived public
/// inputs, comparing stable rebuilt outer metadata to the submitted outer
/// proof, and verifying the submitted outer proof with the production
/// batch-STARK verifier. It is not the production terminal wire path.
pub fn verify_recursive_certificate(
    cert: &AiPowRecursiveCertificate,
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    public_inputs: &crate::composite_public::CompositePublicInputs,
) -> Result<(), VerificationError> {
    verify_recursive_certificate_inner(cert, zk_params, profile, &public_inputs.to_vec())
}

fn verify_recursive_certificate_inner(
    cert: &AiPowRecursiveCertificate,
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    public_values: &[Val],
) -> Result<(), VerificationError> {
    use p3_circuit_prover::BatchStarkProver;

    if public_values.len() != crate::composite_public::NUM_PUBLIC_VALUES {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW recursive certificate verification requires exactly {} \
                 verifier-derived public inputs; got {}",
            crate::composite_public::NUM_PUBLIC_VALUES,
            public_values.len()
        )));
    }

    let cfg = crate::composite_proof::build_config(zk_params, profile);
    let air = CompositeFullAirWithLookupsPinned::new_with(cert.l0_program.clone(), true);
    let pd = crate::composite_proof::logup_common_for(&cfg, &cert.l0_program, true);
    let built = build_composite_l1_verifier_circuit(
        &cfg, &air, &cert.l0_proof, &pd.common, public_values, profile,
    )?;

    let traces = run_composite_l1_verifier_traces(&built, &cert.l0_proof)?;

    let (expected_circuit_packing, expected_circuit_prover_data) =
        production_l1_circuit_prover_data(&built)?;

    let mut expected_outer_prover = BatchStarkProver::new(production_l1_stark_config())
        .with_table_packing(expected_circuit_packing.clone());
    expected_outer_prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    expected_outer_prover.register_recompose_table::<2>(true);
    let expected_outer_proof = expected_outer_prover
        .prove_all_tables(&traces, &expected_circuit_prover_data)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW recursive certificate verifier could not rebuild canonical \
                 L1 outer proof metadata: {e:?}"
            ))
        })?;
    let outer = &cert.l1_outer_proof;
    if outer.rows != expected_outer_proof.rows
        || outer.alu_variant != expected_outer_proof.alu_variant
        || outer.ext_degree != expected_outer_proof.ext_degree
        || outer.w_binomial != expected_outer_proof.w_binomial
        || outer.alu_quintic_trinomial != expected_outer_proof.alu_quintic_trinomial
        || !non_primitive_metadata_eq(&outer.non_primitives, &expected_outer_proof.non_primitives)
    {
        return Err(VerificationError::InvalidProofShape(
            "AI-PoW recursive certificate outer proof metadata is not the \
             canonical L1 verifier circuit shape for the supplied Layer-0 \
             proof, program, parameters, and public inputs"
                .to_string(),
        ));
    }

    let expected_public_binding_lanes = 0;
    let expected_packing = production_l1_table_packing(expected_public_binding_lanes);
    if outer.ext_degree != 2 {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW recursive certificate uses extension degree {}; expected 2",
            outer.ext_degree
        )));
    }
    if expected_circuit_packing != expected_packing {
        return Err(VerificationError::InvalidProofShape(format!(
            "rebuilt AI-PoW recursive verifier circuit uses table packing {:?}; \
             expected production packing {:?}",
            expected_circuit_packing, expected_packing
        )));
    }
    if outer.table_packing != expected_packing {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW recursive certificate uses non-production table packing {:?}; \
             expected {:?}",
            outer.table_packing, expected_packing
        )));
    }
    if outer.public_binding_lanes != expected_public_binding_lanes {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW recursive certificate binds {} L1 public values; expected {}",
            outer.public_binding_lanes, expected_public_binding_lanes
        )));
    }
    if outer.alu_quintic_trinomial {
        return Err(VerificationError::InvalidProofShape(
            "AI-PoW recursive certificate unexpectedly selected quintic ALU".to_string(),
        ));
    }
    expected_outer_prover
        .verify_all_tables(outer)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW recursive certificate outer proof failed production \
             batch-STARK verification: {e:?}"
            ))
        })?;
    Ok(())
}

/// Per-stage instrumentation of one end-to-end composite→L1 recursion run.
///
/// `l1_cert` is the batch-STARK recursive checkpoint certificate. The Layer-0
/// proof and pinned program are intentionally owned by that certificate so
/// verification can rebuild and bind the exact L1 verifier circuit.
pub struct L1RecursionRun {
    /// Composite (Layer-0) STARK trace height — the dominant cost
    /// and memory driver.
    pub composite_trace_height: usize,
    /// Composite trace width (`composite_layout::TOTAL_TRACE_WIDTH`).
    pub composite_trace_width: usize,
    /// Wall-clock (ms) to prove the composite batch-STARK (L0).
    pub composite_prove_ms: u128,
    /// Wall-clock (ms) to build the L1 recursive-verifier circuit.
    pub l1_circuit_build_ms: u128,
    /// Wall-clock (ms) to run the L1 verifier circuit — the
    /// in-circuit accept check (S3).
    pub l1_in_circuit_verify_ms: u128,
    /// Wall-clock (ms) to outer-prove the L1 verifier circuit as a
    /// D=2 batch-STARK + `verify_all_tables` — the L1 certificate (S5).
    pub l1_outer_cert_ms: u128,
    /// Public inputs bound by the composite proof that the L1 certificate
    /// recursively verifies.
    pub public_inputs: crate::composite_public::CompositePublicInputs,
    /// The L1 recursive certificate.
    ///
    /// This is the batch-STARK recursive checkpoint artifact.
    pub l1_cert: AiPowRecursiveCertificate,
}

/// Timings and certificate for recursively certifying an already-built
/// Layer-0 composite proof.
///
/// This is useful for callers that already used the ai-pow bridge to build
/// the canonical Layer-0 proof and pinned program from a mining solution.
/// The returned `l1_cert` is the recursive proof artifact; consensus admission
/// still belongs to the outer ai-pow statement verifier.
pub struct L1CertificateRun {
    /// Wall-clock (ms) to build the L1 recursive-verifier circuit.
    pub l1_circuit_build_ms: u128,
    /// Wall-clock (ms) to run the L1 verifier circuit.
    pub l1_in_circuit_verify_ms: u128,
    /// Wall-clock (ms) to outer-prove the L1 verifier circuit.
    pub l1_outer_cert_ms: u128,
    /// The batch-STARK recursive checkpoint certificate.
    pub l1_cert: AiPowRecursiveCertificate,
}

/// Timings and certificate for recursively certifying an already-built Layer-0
/// composite proof with the native terminal backend.
///
/// This is the production-size target counterpart to [`L1CertificateRun`].
/// It keeps the terminal public inputs next to the certificate because the
/// terminal certificate binds their digest but does not serialize the values in
/// `TerminalCertificate` itself.
pub struct TerminalCertificateRun {
    /// Wall-clock (ms) to build the L1 recursive-verifier circuit.
    pub l1_circuit_build_ms: u128,
    /// Wall-clock (ms) to run the L1 verifier circuit and materialize terminal
    /// witness traces.
    pub l1_in_circuit_verify_ms: u128,
    /// Wall-clock (ms) to compile the L1 verifier circuit into the terminal
    /// relation.
    pub terminal_compile_ms: u128,
    /// Wall-clock (ms) to produce the native terminal proof body.
    pub terminal_prove_ms: u128,
    /// Wall-clock (ms) to verify the native terminal certificate.
    pub terminal_verify_ms: u128,
    /// The native terminal recursive certificate and its bound public inputs.
    pub terminal_cert: AiPowTerminalRecursiveCertificate,
}

/// **Batch-STARK recursive checkpoint caller** — the full ai-pow-zk →
/// Plonky3-recursion
/// pipeline for one composite proof, end to end:
///
/// 1. prove the composite matmul-PoW batch-STARK (Layer 0);
/// 2. build the L1 recursive-verifier circuit and run it — the
///    composite proof is verified in-circuit (S3);
/// 3. outer-prove that verifier circuit as a D=2 batch-STARK and
///    `verify_all_tables` — the L1 recursive certificate (S5).
///
/// Returns per-stage timings and the canonical L1 certificate. The
/// certificate owns the Layer-0 proof/program context required for
/// verifier-side L1 circuit binding; callers must not persist or transmit
/// any separate Layer-0 proof artifact.
///
/// This is the single public entrypoint a production consumer (or a
/// measurement harness) drives; it hides the crate-internal program-pin
/// / `CommonData` plumbing. The canonical program is extracted from the
/// trace and pinned (CRIT-1), exactly as the Layer-0 proving path
/// (`composite_prove_pinned_logup`).
pub fn recurse_composite_to_l1(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    trace: crate::composite_trace::CompositeTrace,
) -> Result<L1RecursionRun, VerificationError> {
    use std::time::Instant;

    let cfg = crate::composite_proof::build_config(zk_params, profile);
    let composite_trace_height = trace.height();
    let composite_trace_width = trace.width();
    let pis = crate::composite_public::CompositePublicInputs::derive_from_trace(&trace);

    let t = Instant::now();
    let (composite_proof, program) =
        crate::composite_proof::composite_prove_pinned_logup(&cfg, trace, &pis);
    let composite_prove_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
    let pd = crate::composite_proof::logup_common_for(&cfg, &program, true);
    let built = build_composite_l1_verifier_circuit(
        &cfg,
        &air,
        &composite_proof,
        &pd.common,
        &pis.to_vec(),
        profile,
    )?;
    let l1_circuit_build_ms = t.elapsed().as_millis();

    let t = Instant::now();
    run_composite_l1_verifier(&built, &composite_proof)?;
    let l1_in_circuit_verify_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let l1_outer_proof = prove_composite_l1_outer_cert(&built, &composite_proof)?;
    let l1_cert = AiPowRecursiveCertificate::new(composite_proof, program, l1_outer_proof);
    let l1_outer_cert_ms = t.elapsed().as_millis();

    Ok(L1RecursionRun {
        composite_trace_height,
        composite_trace_width,
        composite_prove_ms,
        l1_circuit_build_ms,
        l1_in_circuit_verify_ms,
        l1_outer_cert_ms,
        public_inputs: pis,
        l1_cert,
    })
}

/// Layer-0 proof parts that a caller has already checked against the
/// chain-derived AI-PoW statement.
pub struct ChainVerifiedCompositeProof<'a> {
    program: crate::AiPowProgram,
    proof: BatchProof<AiPowStarkConfig>,
    public_inputs: &'a crate::composite_public::CompositePublicInputs,
}

impl<'a> ChainVerifiedCompositeProof<'a> {
    /// Construct a recursion input after the caller has verified the
    /// Layer-0 proof against the exact chain-derived statement:
    /// canonical program, public inputs, target, selected work unit,
    /// commitments, nonce, and production/full-work admissibility.
    ///
    /// # Safety
    ///
    /// This is unsafe because the type cannot itself prove that the
    /// caller performed the chain statement verification. Constructing
    /// it from arbitrary proof parts can produce a recursive certificate
    /// for a valid STARK statement that is not a valid Nockchain AI-PoW
    /// work unit.
    pub unsafe fn from_parts_after_chain_statement_verification(
        program: crate::AiPowProgram,
        proof: BatchProof<AiPowStarkConfig>,
        public_inputs: &'a crate::composite_public::CompositePublicInputs,
    ) -> Self {
        Self {
            program,
            proof,
            public_inputs,
        }
    }
}

/// Produce a recursive AI-PoW certificate from bridge-verified Layer-0
/// proof parts.
///
/// This function recursively verifies the Layer-0 proof in-circuit and
/// returns only the recursive L1 certificate. It does not serialize,
/// persist, or bless the Layer-0 proof as a block artifact.
pub fn prove_recursive_certificate_from_chain_verified_composite_proof(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: ChainVerifiedCompositeProof<'_>,
) -> Result<L1CertificateRun, VerificationError> {
    use std::time::Instant;

    let cfg = crate::composite_proof::build_config(zk_params, profile);
    let t = Instant::now();
    let air = CompositeFullAirWithLookupsPinned::new_with(verified.program.clone(), true);
    let pd = crate::composite_proof::logup_common_for(&cfg, &verified.program, true);
    let built = build_composite_l1_verifier_circuit(
        &cfg,
        &air,
        &verified.proof,
        &pd.common,
        &verified.public_inputs.to_vec(),
        profile,
    )?;
    let l1_circuit_build_ms = t.elapsed().as_millis();

    let t = Instant::now();
    run_composite_l1_verifier(&built, &verified.proof)?;
    let l1_in_circuit_verify_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let l1_outer_proof = prove_composite_l1_outer_cert(&built, &verified.proof)?;
    let l1_cert = AiPowRecursiveCertificate::new(verified.proof, verified.program, l1_outer_proof);
    let l1_outer_cert_ms = t.elapsed().as_millis();

    Ok(L1CertificateRun {
        l1_circuit_build_ms,
        l1_in_circuit_verify_ms,
        l1_outer_cert_ms,
        l1_cert,
    })
}

fn terminal_compiler() -> NativeTerminalCompiler {
    NativeTerminalCompiler::new("nock-terminal-v0", 60)
}

fn prove_composite_l1_terminal_cert(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
) -> Result<(AiPowTerminalRecursiveCertificate, u128, u128, u128, u128), VerificationError> {
    use std::time::Instant;

    let t = Instant::now();
    let traces = run_composite_l1_verifier_traces(built, proof)?;
    let l1_in_circuit_verify_ms = t.elapsed().as_millis();

    let compiler = terminal_compiler();
    let t = Instant::now();
    let (_pk, vk) = compiler
        .compile_goldilocks_terminal(&built.circuit)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate compile failed: {e:?}"
            ))
        })?;
    compiler
        .validate_goldilocks_production_query_domains(
            &vk,
            TerminalProofParameters::production_60bit(),
        )
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate production parameter check failed: {e:?}"
            ))
        })?;
    let terminal_compile_ms = t.elapsed().as_millis();

    let witness = TerminalWitness {
        fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
        public_inputs: built.public_inputs.clone(),
        private_inputs: built.private_inputs.clone(),
        traces,
    };

    let t = Instant::now();
    let terminal_proof = compiler
        .prove_terminal_production_goldilocks(&vk, &witness.public_inputs, &witness)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate prove failed: {e:?}"
            ))
        })?;
    let terminal_prove_ms = t.elapsed().as_millis();

    let terminal_certificate = compiler
        .assemble_goldilocks_production_certificate(&vk, &witness.public_inputs, &terminal_proof)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate assembly failed: {e:?}"
            ))
        })?;

    let t = Instant::now();
    compiler
        .verify_goldilocks_production_certificate(
            &vk, &terminal_certificate, &witness.public_inputs,
        )
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate verification failed: {e:?}"
            ))
        })?;
    let terminal_verify_ms = t.elapsed().as_millis();

    Ok((
        AiPowTerminalRecursiveCertificate::new(witness.public_inputs, terminal_certificate),
        l1_in_circuit_verify_ms,
        terminal_compile_ms,
        terminal_prove_ms,
        terminal_verify_ms,
    ))
}

/// Produce a native terminal recursive AI-PoW certificate from bridge-verified
/// Layer-0 proof parts.
///
/// This is the production-size recursive target: it verifies the Layer-0 proof
/// in the L1 circuit, then proves the executed L1 verifier relation with the
/// native terminal backend instead of wrapping it in another batch-STARK.
pub fn prove_terminal_certificate_from_chain_verified_composite_proof(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: &ChainVerifiedCompositeProof<'_>,
) -> Result<TerminalCertificateRun, VerificationError> {
    use std::time::Instant;

    let cfg = crate::composite_proof::build_config(zk_params, profile);
    let t = Instant::now();
    let air = CompositeFullAirWithLookupsPinned::new_with(verified.program.clone(), true);
    let pd = crate::composite_proof::logup_common_for(&cfg, &verified.program, true);
    let built = build_composite_l1_verifier_circuit(
        &cfg,
        &air,
        &verified.proof,
        &pd.common,
        &verified.public_inputs.to_vec(),
        profile,
    )?;
    let l1_circuit_build_ms = t.elapsed().as_millis();

    let (
        terminal_cert,
        l1_in_circuit_verify_ms,
        terminal_compile_ms,
        terminal_prove_ms,
        terminal_verify_ms,
    ) = prove_composite_l1_terminal_cert(&built, &verified.proof)?;

    Ok(TerminalCertificateRun {
        l1_circuit_build_ms,
        l1_in_circuit_verify_ms,
        terminal_compile_ms,
        terminal_prove_ms,
        terminal_verify_ms,
        terminal_cert,
    })
}

/// Verify a native terminal recursive certificate against the same
/// chain-verified Layer-0 proof parts used to build the L1 verifier circuit.
///
/// This verifies terminal cryptographic binding for the current implementation
/// shape. A complete production wire verifier still needs a canonical way to
/// rebuild the L1 terminal verifying key without carrying the whole Layer-0
/// proof as verifier context.
pub fn verify_terminal_certificate_from_chain_verified_composite_proof(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: &ChainVerifiedCompositeProof<'_>,
    cert: &AiPowTerminalRecursiveCertificate,
) -> Result<(), VerificationError> {
    let cfg = crate::composite_proof::build_config(zk_params, profile);
    let air = CompositeFullAirWithLookupsPinned::new_with(verified.program.clone(), true);
    let pd = crate::composite_proof::logup_common_for(&cfg, &verified.program, true);
    let built = build_composite_l1_verifier_circuit(
        &cfg,
        &air,
        &verified.proof,
        &pd.common,
        &verified.public_inputs.to_vec(),
        profile,
    )?;
    if built.public_inputs != cert.terminal_public_inputs {
        return Err(VerificationError::InvalidProofShape(
            "AI-PoW terminal certificate public input vector is not the \
             canonical vector for the supplied Layer-0 proof, program, \
             parameters, and statement"
                .to_string(),
        ));
    }
    let compiler = terminal_compiler();
    let (_pk, vk) = compiler
        .compile_goldilocks_terminal(&built.circuit)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate compile failed: {e:?}"
            ))
        })?;
    compiler
        .validate_goldilocks_production_query_domains(
            &vk,
            TerminalProofParameters::production_60bit(),
        )
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate production parameter check failed: {e:?}"
            ))
        })?;
    compiler
        .verify_goldilocks_production_certificate(
            &vk,
            cert.terminal_certificate(),
            cert.terminal_public_inputs(),
        )
        .map_err(|e: NativeTerminalVerifyError| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal certificate verification failed: {e:?}"
            ))
        })
}

/// Produce the hardened batch-STARK recursive AI-PoW checkpoint certificate.
///
/// This is a name-level guardrail against raw Layer-0 proof submission: the
/// returned certificate is recursive and cryptographically verifies the L1
/// verifier-circuit proof body. It is not the production terminal certificate
/// target because the batch-STARK certificate is too large for the wire budget.
/// Consensus callers must separately derive the exact public statement and
/// reject selected-tile statements that do not prove the intended full-matmul
/// work unit.
pub fn prove_canonical_ai_pow_certificate(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    trace: crate::composite_trace::CompositeTrace,
) -> Result<L1RecursionRun, VerificationError> {
    recurse_composite_to_l1(zk_params, profile, trace)
}

/// Serialize the batch-STARK recursive AI-PoW checkpoint certificate into
/// compact bytes.
///
/// This serializes the batch-STARK structured recursive checkpoint, including
/// the Layer-0 proof/program context needed to rebuild the L1 verifier circuit.
/// It does not accept or produce a standalone Layer-0 `AiPowBatchProof`,
/// because raw Layer-0 proofs are not block/wire certificates for Nockchain
/// AI-PoW. This helper is not the native terminal production wire format.
pub fn encode_recursive_certificate(
    cert: &AiPowRecursiveCertificate,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::serde::encode_to_vec(cert, bincode::config::standard().with_fixed_int_encoding())
}

/// Decode bytes previously produced by [`encode_recursive_certificate`].
///
/// Decoding is structural only; callers still need to verify the certificate
/// against chain-derived statement data once the verifier is wired.
pub fn decode_recursive_certificate(
    bytes: &[u8],
) -> Result<AiPowRecursiveCertificate, bincode::error::DecodeError> {
    let (cert, consumed) = bincode::serde::decode_from_slice(
        bytes,
        bincode::config::standard().with_fixed_int_encoding(),
    )?;
    if consumed != bytes.len() {
        return Err(bincode::error::DecodeError::OtherString(format!(
            "recursive certificate decode left {} trailing bytes",
            bytes.len() - consumed
        )));
    }
    Ok(cert)
}

/// Serialize the native terminal recursive certificate and its terminal public
/// input vector into compact fixed-int bytes.
pub fn encode_terminal_recursive_certificate(
    cert: &AiPowTerminalRecursiveCertificate,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::serde::encode_to_vec(cert, bincode::config::standard().with_fixed_int_encoding())
}

/// Decode bytes previously produced by
/// [`encode_terminal_recursive_certificate`].
pub fn decode_terminal_recursive_certificate(
    bytes: &[u8],
) -> Result<AiPowTerminalRecursiveCertificate, bincode::error::DecodeError> {
    let (cert, consumed) = bincode::serde::decode_from_slice(
        bytes,
        bincode::config::standard().with_fixed_int_encoding(),
    )?;
    if consumed != bytes.len() {
        return Err(bincode::error::DecodeError::OtherString(format!(
            "terminal recursive certificate decode left {} trailing bytes",
            bytes.len() - consumed
        )));
    }
    Ok(cert)
}

/// S3a — compile-time proof that the composite AIR satisfies the
/// recursion substrate's `RecursiveAir` bound.
fn _require_recursive_air<A>()
where
    A: RecursiveAir<Val, Challenge, LogUpGadget>,
{
}

#[allow(dead_code)]
fn _composite_conforms_to_recursive_air() {
    _require_recursive_air::<CompositeFullAirWithLookupsPinned>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composite_proof::{build_config, composite_prove_pinned_logup, logup_common_for};
    use crate::composite_public::CompositePublicInputs;
    use crate::composite_trace::CompositeTrace;
    use crate::params::ZkParams;
    use crate::CircuitConfig;

    fn test_zk_params() -> ZkParams {
        ZkParams {
            m: 8,
            k: 16,
            n: 8,
            noise_rank: 2,
            tile: 2,
            difficulty_bits: 0,
        }
    }

    /// S3d — end-to-end: a real composite batch-STARK proof is
    /// recursively verified in-circuit by the L1 recursion verifier,
    /// and the verifier circuit **accepts**.
    ///
    /// Proves a real honest composite proof
    /// (`composite_prove_pinned_logup` over `baseline_min`), builds the
    /// L1 recursive-verifier circuit via
    /// `build_composite_l1_verifier_circuit`, and runs it. This is the
    /// `ai-pow-zk` ↔ `Plonky3-recursion` integration end-to-end:
    /// `runner.run()` succeeding means the in-circuit FRI / Tip5
    /// challenger / MMCS recompute accepted the composite proof.
    ///
    /// (Both sides use 5-round Tip5 — see `circuit::Tip5Perm` and the
    /// `Plonky3-recursion` `tip5-circuit-air`.)
    #[test]
    fn composite_recursively_verified_l1_accepts() {
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&test_zk_params(), &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        // `composite_prove_pinned_logup` extracts + returns the
        // canonical program (CRIT-1 pin); the verifier uses it.
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);

        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);

        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .expect("build the composite L1 verifier circuit");

        run_composite_l1_verifier(&built, &proof)
            .expect("L1 recursive verification of the real composite proof must accept");
    }

    /// S5 — build a real composite proof, recursively verify it in the
    /// L1 circuit, and outer-prove that verifier circuit as a D=2
    /// batch-STARK (the L1 recursive certificate). When `tamper`, one
    /// FRI-bound opened OOD trace evaluation of the composite proof is
    /// corrupted before the L1 circuit is built — the in-circuit
    /// quotient-consistency recompute must then reject. Returns the
    /// serialized certificate byte length on accept.
    fn run_composite_l1_outer_cert(tamper: bool) -> Result<usize, String> {
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&test_zk_params(), &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (mut proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);

        if tamper {
            // Corrupt a single FRI-bound opened OOD trace evaluation.
            proof.opened_values.instances[0]
                .base_opened_values
                .trace_local[0] += Challenge::ONE;
        }

        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);

        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .map_err(|e| format!("build composite L1 verifier circuit: {e:?}"))?;

        let cert = prove_composite_l1_outer_cert(&built, &proof).map_err(|e| format!("{e:?}"))?;
        let bytes =
            postcard::to_allocvec(&cert).map_err(|e| format!("serialize L1 certificate: {e}"))?;
        Ok(bytes.len())
    }

    /// S5 ACCEPT: an honest composite proof yields a valid L1 outer
    /// certificate that `verify_all_tables` (the cross-table
    /// `WitnessChecks` soundness gate) accepts.
    #[test]
    fn composite_l1_outer_cert_accepts() {
        match run_composite_l1_outer_cert(false) {
            Ok(bytes) => eprintln!(
                "[S5] composite→L1 outer certificate ACCEPTED — serialized {} bytes ({:.2} KB)",
                bytes,
                bytes as f64 / 1024.0,
            ),
            Err(e) => panic!("valid composite→L1 outer certificate was REJECTED: {e}"),
        }
    }

    #[test]
    fn recursive_certificate_outer_verifier_accepts_honest_certificate() {
        let zk = test_zk_params();
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .expect("build composite L1 verifier circuit");
        let outer =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");
        let cert = AiPowRecursiveCertificate::new(proof, program, outer);

        verify_recursive_certificate(&cert, &zk, &profile, &pis)
            .expect("recursive certificate verifier must accept honest cert");
        verify_recursive_certificate_inner(&cert, &zk, &profile, &[])
            .expect_err("recursive verifier must reject empty statement public inputs");
    }

    #[test]
    fn recursive_certificate_fixed_bincode_round_trip_verifies() {
        let zk = test_zk_params();
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .expect("build composite L1 verifier circuit");
        let outer =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");
        let cert = AiPowRecursiveCertificate::new(proof, program, outer);

        let bytes = encode_recursive_certificate(&cert).expect("encode recursive certificate");
        let decoded = decode_recursive_certificate(&bytes).expect("decode recursive certificate");
        verify_recursive_certificate(&decoded, &zk, &profile, &pis)
            .expect("decoded recursive certificate must verify");

        let mut trailing = bytes;
        trailing.push(0);
        assert!(
            decode_recursive_certificate(&trailing).is_err(),
            "decoder must reject trailing bytes after certificate"
        );
    }

    fn prove_test_terminal_certificate() -> (
        ZkParams,
        CircuitConfig,
        CompositePublicInputs,
        BatchProof<AiPowStarkConfig>,
        crate::AiPowProgram,
        TerminalCertificateRun,
    ) {
        let zk = test_zk_params();
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };
        let run = prove_terminal_certificate_from_chain_verified_composite_proof(
            &zk, &profile, &verified,
        )
        .expect("native terminal recursive certificate must prove");
        let proof = verified.proof;
        (zk, profile, pis, proof, program, run)
    }

    #[test]
    #[ignore = "native terminal proof over the full composite verifier is an opt-in measurement"]
    fn terminal_recursive_certificate_round_trip_verifies() {
        let (zk, profile, pis, proof, program, run) = prove_test_terminal_certificate();

        let certificate_bytes = postcard::to_allocvec(run.terminal_cert.terminal_certificate())
            .expect("terminal certificate postcard serialization")
            .len();
        let public_input_bytes = postcard::to_allocvec(run.terminal_cert.terminal_public_inputs())
            .expect("terminal public inputs postcard serialization")
            .len();
        let wire_bytes = encode_terminal_recursive_certificate(&run.terminal_cert)
            .expect("fixed-int terminal recursive certificate encoding");
        eprintln!(
            "native terminal recursive certificate over ai-pow composite verifier: certificate={} bytes public_inputs={} bytes wire={} bytes build_ms={} l1_verify_ms={} compile_ms={} prove_ms={} verify_ms={}",
            certificate_bytes,
            public_input_bytes,
            wire_bytes.len(),
            run.l1_circuit_build_ms,
            run.l1_in_circuit_verify_ms,
            run.terminal_compile_ms,
            run.terminal_prove_ms,
            run.terminal_verify_ms,
        );

        let decoded = decode_terminal_recursive_certificate(&wire_bytes)
            .expect("decode terminal recursive certificate");
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program, proof, &pis,
            )
        };
        verify_terminal_certificate_from_chain_verified_composite_proof(
            &zk, &profile, &verified, &decoded,
        )
        .expect("decoded terminal recursive certificate must verify");

        let mut trailing = wire_bytes;
        trailing.push(0);
        assert!(
            decode_terminal_recursive_certificate(&trailing).is_err(),
            "terminal decoder must reject trailing bytes"
        );
    }

    #[test]
    #[ignore = "native terminal proof over the full composite verifier is an opt-in measurement"]
    fn terminal_recursive_certificate_rejects_public_input_tamper() {
        let (zk, profile, pis, proof, program, mut run) = prove_test_terminal_certificate();
        run.terminal_cert.terminal_public_inputs[0] += Challenge::ONE;

        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program, proof, &pis,
            )
        };
        verify_terminal_certificate_from_chain_verified_composite_proof(
            &zk, &profile, &verified, &run.terminal_cert,
        )
        .expect_err("terminal verifier must reject tampered public input vector");
    }

    #[test]
    fn recursive_certificate_outer_verifier_rejects_non_production_envelope() {
        let zk = test_zk_params();
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .expect("build composite L1 verifier circuit");
        let outer =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");
        let mut cert = AiPowRecursiveCertificate::new(proof, program, outer);

        cert.l1_outer_proof.ext_degree = 1;
        verify_recursive_certificate(&cert, &zk, &profile, &pis)
            .expect_err("recursive verifier must reject non-D=2 recursion envelope");
    }

    #[test]
    fn recursive_certificate_rejects_outer_circuit_metadata_tamper() {
        let zk = test_zk_params();
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .expect("build composite L1 verifier circuit");
        let outer =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");
        let mut cert = AiPowRecursiveCertificate::new(proof, program, outer);

        cert.l1_outer_proof.non_primitives.clear();
        verify_recursive_certificate(&cert, &zk, &profile, &pis)
            .expect_err("recursive verifier must reject non-canonical L1 circuit metadata");
    }

    #[test]
    fn recursive_certificate_rejects_outer_proof_body_tamper() {
        let zk = test_zk_params();
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .expect("build composite L1 verifier circuit");
        let outer =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");
        let mut cert = AiPowRecursiveCertificate::new(proof, program, outer);

        let first_opened_value = cert
            .l1_outer_proof
            .proof
            .opened_values
            .instances
            .get_mut(0)
            .and_then(|instance| instance.base_opened_values.trace_local.get_mut(0))
            .expect("outer proof exposes at least one trace opening");
        *first_opened_value += Val::ONE;

        verify_recursive_certificate(&cert, &zk, &profile, &pis)
            .expect_err("recursive verifier must reject tampered L1 proof body");
    }

    #[test]
    fn recursive_certificate_rejects_wrong_statement_public_inputs() {
        let zk = test_zk_params();
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
        let pd = logup_common_for(&cfg, &program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &proof,
            &pd.common,
            &pis.to_vec(),
            &profile,
        )
        .expect("build composite L1 verifier circuit");
        let outer =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");
        let cert = AiPowRecursiveCertificate::new(proof, program, outer);

        let mut wrong = pis.clone();
        wrong.job_key[0] ^= 1;
        verify_recursive_certificate(&cert, &zk, &profile, &wrong)
            .expect_err("recursive certificate must reject metadata-swapped public inputs");
    }

    /// S5 TAMPER-REJECT: a composite proof with one corrupted opened
    /// OOD trace value must NOT yield a certificate — the in-circuit
    /// FRI/quotient-consistency binding rejects it. A rejection via
    /// `Err` (in-circuit `WitnessConflict`) or a panic (debug
    /// assertion) both count; only a produced certificate fails.
    #[test]
    fn composite_l1_outer_cert_tamper_rejects() {
        let res = std::panic::catch_unwind(|| run_composite_l1_outer_cert(true));
        match res {
            Ok(Ok(bytes)) => panic!(
                "tampered composite→L1 outer certificate was ACCEPTED ({bytes} bytes) \
                 — SOUNDNESS FAILURE"
            ),
            Ok(Err(_)) | Err(_) => { /* rejected — correct */ }
        }
    }
}
