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
use p3_recursion::terminal::{
    NativeTerminalCompiler, NativeTerminalVerifyError, TerminalNpoPolynomialLayoutMetrics,
    TerminalNpoTip5PackedLookupTraceProfile,
};
use p3_recursion::{
    verify_batch_circuit, BatchProofTargets, CommonDataTargets, Recursive, RecursiveAir,
    TerminalCertificate, TerminalCircuitFingerprint, TerminalProofParameters, TerminalWitness,
    VerificationError,
};
use p3_symmetric::Permutation;
use p3_tip5_circuit_air::Tip5Perm as RecTip5Perm;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

fn production_l1_table_packing(public_binding_lanes: usize) -> p3_circuit_prover::TablePacking {
    p3_circuit_prover::TablePacking::new(DIGEST_ELEMS, 8)
        .with_public_binding_lanes(public_binding_lanes)
        .with_horner_pack_k(5)
}

fn production_l1_stark_config() -> p3_circuit_prover::config::GoldilocksTipsConfig {
    p3_circuit_prover::config::goldilocks_tip5_60bit()
}

#[cfg(test)]
fn pure_query_l1_stark_config_with_shape(
    log_blowup: usize,
    num_queries: usize,
) -> p3_circuit_prover::config::GoldilocksTipsConfig {
    p3_circuit_prover::config::goldilocks_tip5_pure_query_60bit_with_shape(log_blowup, num_queries)
}

#[cfg(test)]
fn pure_query_l1_stark_config_with_shape_and_cap(
    log_blowup: usize,
    num_queries: usize,
    cap_height: usize,
) -> p3_circuit_prover::config::GoldilocksTipsConfig {
    p3_circuit_prover::config::goldilocks_tip5_pure_query_60bit_with_shape_and_cap(
        log_blowup, num_queries, cap_height,
    )
}

#[cfg(test)]
fn pure_query_l1_stark_config_with_fri_shape(
    log_blowup: usize,
    num_queries: usize,
    log_final_poly_len: usize,
    max_log_arity: usize,
    cap_height: usize,
) -> p3_circuit_prover::config::GoldilocksTipsConfig {
    p3_circuit_prover::config::goldilocks_tip5_pure_query_60bit_with_fri_shape(
        log_blowup, num_queries, log_final_poly_len, max_log_arity, cap_height,
    )
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
    build_composite_l1_verifier_circuit_with_recompose_coeff_ctl(
        config, composite_air, proof, common_data, public_values, profile, true,
    )
}

fn build_composite_l1_verifier_circuit_with_recompose_coeff_ctl(
    config: &AiPowStarkConfig,
    composite_air: &CompositeFullAirWithLookupsPinned,
    proof: &BatchProof<AiPowStarkConfig>,
    common_data: &CommonData<AiPowStarkConfig>,
    public_values: &[Val],
    profile: &crate::circuit::CircuitConfig,
    recompose_coeff_ctl_for_decompose_links: bool,
) -> Result<BuiltCompositeL1, VerificationError> {
    let mut cb = CircuitBuilder::<Challenge>::new();
    // In-circuit Tip5 permutation NPO + the recompose link (mirror of
    // the validated Layer-0 verifier circuit, `test_tip5_layer0_
    // recursion.rs`).
    cb.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<Challenge, Tip5Goldilocks>, LiftTip5,
    );
    cb.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
    cb.set_recompose_coeff_ctl_for_decompose_links(recompose_coeff_ctl_for_decompose_links);

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
    production_l1_circuit_prover_data_with_public_binding_lanes(built, 0)
}

fn production_l1_circuit_prover_data_with_public_binding_lanes(
    built: &BuiltCompositeL1,
    public_binding_lanes: usize,
) -> Result<
    (
        p3_circuit_prover::TablePacking,
        p3_circuit_prover::CircuitProverData<p3_circuit_prover::config::GoldilocksTipsConfig>,
    ),
    VerificationError,
> {
    l1_circuit_prover_data_with_config_and_public_binding_lanes(
        built,
        &production_l1_stark_config(),
        public_binding_lanes,
    )
}

fn l1_circuit_prover_data_with_config_and_public_binding_lanes(
    built: &BuiltCompositeL1,
    outer_config: &p3_circuit_prover::config::GoldilocksTipsConfig,
    public_binding_lanes: usize,
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
    let prover_data =
        ProverData::from_airs_and_degrees(outer_config, &lookup_metadata_airs, &degrees);
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
    prove_composite_l1_outer_cert_with_public_binding_lanes(built, proof, 0)
}

fn prove_composite_l1_outer_cert_with_public_binding_lanes(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
    public_binding_lanes: usize,
) -> Result<AiPowL1OuterProof, VerificationError> {
    prove_composite_l1_outer_cert_with_config_and_public_binding_lanes(
        built,
        proof,
        production_l1_stark_config(),
        public_binding_lanes,
    )
}

fn prove_composite_l1_outer_cert_with_config_and_public_binding_lanes(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
    outer_config: p3_circuit_prover::config::GoldilocksTipsConfig,
    public_binding_lanes: usize,
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
    if !p3_circuit_prover::common_preprocessed_binding_eq(
        &outer.stark_common, &expected_outer_proof.stark_common,
    ) {
        return Err(VerificationError::InvalidProofShape(
            "AI-PoW recursive certificate outer proof preprocessed commitment \
             binding is not the canonical L1 verifier circuit preprocessed binding"
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

/// Cheap relation-shape metrics for the native terminal composite-verifier
/// path.
///
/// This intentionally does not run the terminal prover. It is the first
/// diagnostic to check when the full terminal path misses the release-time
/// gate, because the terminal proof size/runtime is driven by these relation
/// counts and by the terminal public-input vector length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeTerminalRelationMetrics {
    pub l1_circuit_build_ms: u128,
    pub terminal_compile_ms: u128,
    pub circuit_fingerprint: TerminalCircuitFingerprint,
    pub terminal_public_input_values: usize,
    pub terminal_public_input_bytes: usize,
    pub terminal_private_input_values: usize,
    pub terminal_operation_count: usize,
    pub primitive_operation_count: usize,
    pub const_operation_count: usize,
    pub public_operation_count: usize,
    pub alu_add_operation_count: usize,
    pub alu_mul_operation_count: usize,
    pub alu_bool_check_operation_count: usize,
    pub alu_mul_add_operation_count: usize,
    pub alu_horner_acc_operation_count: usize,
    pub hint_operation_count: usize,
    pub non_primitive_operation_count: usize,
    pub non_primitive_types: Vec<String>,
    pub terminal_constraint_count: usize,
    pub tip5_rows: usize,
    pub recompose_rows: usize,
    pub recompose_coeff_rows: usize,
    pub npo_rows: usize,
    pub npo_callsite_input_slots: usize,
    pub npo_callsite_output_slots: usize,
    pub external_npo_validity_components: usize,
}

/// NPO-polynomial backend layout metrics for the actual composite-L1 terminal
/// relation.
///
/// This is a proof-shape diagnostic for the FRI-native NPO replacement path.
/// It derives row/column counts and zeta-opening floors from the terminal
/// verifying key without running the terminal prover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeTerminalNpoPolynomialLayoutMetrics {
    pub l1_circuit_build_ms: u128,
    pub terminal_compile_ms: u128,
    pub circuit_fingerprint: TerminalCircuitFingerprint,
    pub terminal_public_input_values: usize,
    pub terminal_private_input_values: usize,
    pub npo_polynomial: TerminalNpoPolynomialLayoutMetrics,
    pub packed_tip5_lookup: TerminalNpoTip5PackedLookupTraceProfile,
}

/// Public/private input footprint of the current composite-L1 verifier circuit.
///
/// Pearl keeps the inner STARK proof as witness material and exposes a small
/// fixed public input vector plus preprocessed-data bindings. The current
/// Plonky3 recursion path exposes commitments, FRI public pieces, global lookup
/// cumulative sums, and common-data commitments as L1 public inputs. This
/// diagnostic makes that split explicit before attempting a Pearl-shaped
/// compact final backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeL1VerifierInputFootprint {
    pub l1_circuit_build_ms: u128,
    pub circuit_fingerprint: TerminalCircuitFingerprint,
    pub statement_digest_values: usize,
    pub air_public_values: usize,
    pub proof_public_values: usize,
    pub common_public_values: usize,
    pub verifier_public_values: usize,
    pub total_public_values: usize,
    pub proof_private_values: usize,
    pub total_private_values: usize,
    pub total_public_input_bytes: usize,
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

fn terminal_profile_log(stage: &str, elapsed_ms: u128) {
    if std::env::var_os("NOCK_TERMINAL_PROFILE_PROVER").is_some() {
        eprintln!("ai-pow terminal profile: {stage}_ms={elapsed_ms}");
    }
}

fn prove_composite_l1_terminal_cert(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
) -> Result<(AiPowTerminalRecursiveCertificate, u128, u128, u128, u128), VerificationError> {
    use std::time::Instant;

    let t = Instant::now();
    let traces = run_composite_l1_verifier_traces(built, proof)?;
    let l1_in_circuit_verify_ms = t.elapsed().as_millis();
    terminal_profile_log("l1_in_circuit_verify", l1_in_circuit_verify_ms);

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
    terminal_profile_log("terminal_compile", terminal_compile_ms);
    if std::env::var_os("NOCK_TERMINAL_PROFILE_PROVER").is_some() {
        let relation_profile = vk.relation_profile();
        eprintln!(
            "ai-pow terminal profile: ops={} primitive_ops={} hints={} npos={} constraints={} tip5_rows={} recompose_rows={} recompose_coeff_rows={} npo_residuals={}",
            vk.inventory.total_ops(),
            vk.inventory.total_primitive_ops(),
            vk.inventory.hint_ops,
            vk.inventory.non_primitive_ops,
            relation_profile.terminal_constraints,
            relation_profile.tip5_rows,
            relation_profile.recompose_rows,
            relation_profile.recompose_coeff_rows,
            relation_profile.external_npo_validity_components,
        );
    }

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
    terminal_profile_log("terminal_prove", terminal_prove_ms);

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
    terminal_profile_log("terminal_verify", terminal_verify_ms);

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

/// Measure the public/private input split of the current composite-L1 verifier
/// circuit without running the terminal compiler or prover.
pub fn measure_composite_l1_verifier_input_footprint(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: &ChainVerifiedCompositeProof<'_>,
) -> Result<CompositeL1VerifierInputFootprint, VerificationError> {
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

    let air_public_values = verified.public_inputs.to_vec().len();
    let proof_public_values =
        <BatchProofTargets<AiPowStarkConfig, CompositeComm, InnerFri> as Recursive<
            Challenge,
        >>::get_values(&verified.proof)
        .len();
    let proof_private_values =
        <BatchProofTargets<AiPowStarkConfig, CompositeComm, InnerFri> as Recursive<
            Challenge,
        >>::get_private_values(&verified.proof)
        .len();
    let common_public_values = <CommonDataTargets<AiPowStarkConfig, CompositeComm> as Recursive<
        Challenge,
    >>::get_values(&pd.common)
    .len();
    let verifier_public_values = air_public_values + proof_public_values + common_public_values;
    let total_public_values = built.public_inputs.len();
    let total_private_values = built.private_inputs.len();

    let expected_total_public_values = DIGEST_ELEMS + verifier_public_values;
    if total_public_values != expected_total_public_values {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW composite L1 public input footprint mismatch: built {} \
             values but split accounts for {}",
            total_public_values, expected_total_public_values
        )));
    }
    if total_private_values != proof_private_values {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW composite L1 private input footprint mismatch: built {} \
             values but proof-private split accounts for {}",
            total_private_values, proof_private_values
        )));
    }

    let total_public_input_bytes = postcard::to_allocvec(&built.public_inputs)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW L1 verifier public input measurement failed: {e:?}"
            ))
        })?
        .len();

    Ok(CompositeL1VerifierInputFootprint {
        l1_circuit_build_ms,
        circuit_fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
        statement_digest_values: DIGEST_ELEMS,
        air_public_values,
        proof_public_values,
        common_public_values,
        verifier_public_values,
        total_public_values,
        proof_private_values,
        total_private_values,
        total_public_input_bytes,
    })
}

/// Measure the native terminal relation generated for the actual composite L1
/// verifier circuit without running terminal proving.
pub fn measure_composite_l1_terminal_relation(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: &ChainVerifiedCompositeProof<'_>,
) -> Result<CompositeTerminalRelationMetrics, VerificationError> {
    measure_composite_l1_terminal_relation_with_recompose_coeff_ctl(
        zk_params, profile, verified, true,
    )
}

fn measure_composite_l1_terminal_relation_with_recompose_coeff_ctl(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: &ChainVerifiedCompositeProof<'_>,
    recompose_coeff_ctl_for_decompose_links: bool,
) -> Result<CompositeTerminalRelationMetrics, VerificationError> {
    use std::time::Instant;

    let cfg = crate::composite_proof::build_config(zk_params, profile);
    let t = Instant::now();
    let air = CompositeFullAirWithLookupsPinned::new_with(verified.program.clone(), true);
    let pd = crate::composite_proof::logup_common_for(&cfg, &verified.program, true);
    let built = build_composite_l1_verifier_circuit_with_recompose_coeff_ctl(
        &cfg,
        &air,
        &verified.proof,
        &pd.common,
        &verified.public_inputs.to_vec(),
        profile,
        recompose_coeff_ctl_for_decompose_links,
    )?;
    let l1_circuit_build_ms = t.elapsed().as_millis();

    let compiler = terminal_compiler();
    let t = Instant::now();
    let (_pk, vk) = compiler
        .compile_goldilocks_terminal(&built.circuit)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal relation compile failed: {e:?}"
            ))
        })?;
    let prelude = compiler
        .build_proof_prelude_goldilocks(
            &vk,
            &built.public_inputs,
            TerminalProofParameters::production_60bit(),
            vec![p3_recursion::TerminalCommitmentDigest([0; 5])],
        )
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal relation profile derivation failed: {e:?}"
            ))
        })?;
    let terminal_compile_ms = t.elapsed().as_millis();

    let terminal_public_input_bytes = postcard::to_allocvec(&built.public_inputs)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal public input measurement failed: {e:?}"
            ))
        })?
        .len();
    let relation_profile = prelude.relation_profile;
    Ok(CompositeTerminalRelationMetrics {
        l1_circuit_build_ms,
        terminal_compile_ms,
        circuit_fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
        terminal_public_input_values: built.public_inputs.len(),
        terminal_public_input_bytes,
        terminal_private_input_values: built.private_inputs.len(),
        terminal_operation_count: vk.inventory.total_ops(),
        primitive_operation_count: vk.inventory.total_primitive_ops(),
        const_operation_count: vk.inventory.const_ops,
        public_operation_count: vk.inventory.public_ops,
        alu_add_operation_count: vk.inventory.alu_add_ops,
        alu_mul_operation_count: vk.inventory.alu_mul_ops,
        alu_bool_check_operation_count: vk.inventory.alu_bool_check_ops,
        alu_mul_add_operation_count: vk.inventory.alu_mul_add_ops,
        alu_horner_acc_operation_count: vk.inventory.alu_horner_acc_ops,
        hint_operation_count: vk.inventory.hint_ops,
        non_primitive_operation_count: vk.inventory.non_primitive_ops,
        non_primitive_types: vk.inventory.non_primitive_types,
        terminal_constraint_count: relation_profile.terminal_constraints,
        tip5_rows: relation_profile.tip5_rows,
        recompose_rows: relation_profile.recompose_rows,
        recompose_coeff_rows: relation_profile.recompose_coeff_rows,
        npo_rows: relation_profile.non_primitive_rows(),
        npo_callsite_input_slots: relation_profile.npo_callsite_input_slots,
        npo_callsite_output_slots: relation_profile.npo_callsite_output_slots,
        external_npo_validity_components: relation_profile.external_npo_validity_components,
    })
}

/// Measure the FRI-native supported-NPO polynomial layout generated for the
/// actual composite L1 verifier circuit without running terminal proving.
pub fn measure_composite_l1_terminal_npo_polynomial_layout(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: &ChainVerifiedCompositeProof<'_>,
) -> Result<CompositeTerminalNpoPolynomialLayoutMetrics, VerificationError> {
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

    let compiler = terminal_compiler();
    let t = Instant::now();
    let (_pk, vk) = compiler
        .compile_goldilocks_terminal(&built.circuit)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW terminal NPO polynomial layout compile failed: {e:?}"
            ))
        })?;
    let npo_polynomial =
        NativeTerminalCompiler::terminal_npo_polynomial_layout_metrics::<Challenge>(&vk).map_err(
            |e| {
                VerificationError::InvalidProofShape(format!(
                    "AI-PoW terminal NPO polynomial layout metrics failed: {e:?}"
                ))
            },
        )?;
    let packed_tip5_lookup = npo_polynomial.packed_tip5_lookup_profile.clone();
    let terminal_compile_ms = t.elapsed().as_millis();

    Ok(CompositeTerminalNpoPolynomialLayoutMetrics {
        l1_circuit_build_ms,
        terminal_compile_ms,
        circuit_fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
        terminal_public_input_values: built.public_inputs.len(),
        terminal_private_input_values: built.private_inputs.len(),
        npo_polynomial,
        packed_tip5_lookup,
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

/// Size accounting for the relaxed-size L1-only batch-STARK candidate.
///
/// This is a diagnostic for the possible `~150 KiB` branch. It does not define
/// a production certificate format. In particular, the current
/// [`AiPowRecursiveCertificate`] carries `l0_proof` and `l0_program` so the
/// verifier can rebuild the exact L1 verifier circuit; an actual L1-only
/// certificate would need a separate verifier-key/statement binding contract
/// before these bytes could be omitted safely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelaxedL1OnlyCandidateSizeBreakdown {
    /// Legacy postcard size of the current full batch-STARK checkpoint
    /// certificate, including L0 proof/program context.
    pub current_checkpoint_postcard_bytes: usize,
    /// Fixed-int bincode size used by [`encode_recursive_certificate`].
    pub current_checkpoint_fixed_bincode_bytes: usize,
    /// Postcard size of the embedded L0 proof.
    pub l0_proof_postcard_bytes: usize,
    /// Postcard size of the embedded canonical L0 program.
    pub l0_program_postcard_bytes: usize,
    /// Postcard size of the full L1 outer proof object, including circuit
    /// metadata and `stark_common`.
    pub l1_outer_postcard_bytes: usize,
    /// Postcard size of the cryptographic L1 batch proof body alone.
    pub l1_proof_body_postcard_bytes: usize,
    /// Difference between `l1_outer_postcard_bytes` and
    /// `l1_proof_body_postcard_bytes`: metadata that must either remain on
    /// wire or be rebuilt/pinned by the verifier.
    pub l1_outer_metadata_postcard_bytes: usize,
    /// Current number of L1 public-table lanes exposed as STARK public values.
    ///
    /// This is currently zero for the size-optimized checkpoint. A production
    /// L1-only branch must add an equivalent explicit binding for the statement
    /// digest and verifier public inputs.
    pub l1_public_binding_lanes: usize,
}

/// Serialization errors while measuring the relaxed L1-only candidate.
#[derive(Debug, Error)]
pub enum RelaxedL1OnlyCandidateSizeError {
    #[error("postcard size accounting failed: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("fixed-int bincode size accounting failed: {0}")]
    Bincode(#[from] bincode::error::EncodeError),
}

/// Measure the current checkpoint as if the L1 outer proof were the only
/// on-wire proof body under the relaxed-size branch.
pub fn measure_relaxed_l1_only_candidate_size(
    cert: &AiPowRecursiveCertificate,
) -> Result<RelaxedL1OnlyCandidateSizeBreakdown, RelaxedL1OnlyCandidateSizeError> {
    let current_checkpoint_postcard_bytes = postcard::to_allocvec(cert)?.len();
    let current_checkpoint_fixed_bincode_bytes = encode_recursive_certificate(cert)?.len();
    let l0_proof_postcard_bytes = postcard::to_allocvec(&cert.l0_proof)?.len();
    let l0_program_postcard_bytes = postcard::to_allocvec(&cert.l0_program)?.len();
    let l1_outer_postcard_bytes = postcard::to_allocvec(&cert.l1_outer_proof)?.len();
    let l1_proof_body_postcard_bytes = postcard::to_allocvec(&cert.l1_outer_proof.proof)?.len();
    let l1_outer_metadata_postcard_bytes =
        l1_outer_postcard_bytes.saturating_sub(l1_proof_body_postcard_bytes);

    Ok(RelaxedL1OnlyCandidateSizeBreakdown {
        current_checkpoint_postcard_bytes,
        current_checkpoint_fixed_bincode_bytes,
        l0_proof_postcard_bytes,
        l0_program_postcard_bytes,
        l1_outer_postcard_bytes,
        l1_proof_body_postcard_bytes,
        l1_outer_metadata_postcard_bytes,
        l1_public_binding_lanes: cert.l1_outer_proof.public_binding_lanes,
    })
}

/// Serialize the native terminal recursive certificate and its terminal public
/// input vector into compact postcard bytes.
pub fn encode_terminal_recursive_certificate(
    cert: &AiPowTerminalRecursiveCertificate,
) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(cert)
}

/// Decode bytes previously produced by
/// [`encode_terminal_recursive_certificate`].
pub fn decode_terminal_recursive_certificate(
    bytes: &[u8],
) -> Result<AiPowTerminalRecursiveCertificate, postcard::Error> {
    let (cert, trailing): (AiPowTerminalRecursiveCertificate, &[u8]) =
        postcard::take_from_bytes(bytes)?;
    if !trailing.is_empty() {
        return Err(postcard::Error::DeserializeUnexpectedEnd);
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
    use p3_recursion::terminal::{
        NativeTerminalConstraint, NativeTerminalVerifyingKey, TerminalCompressedFriInputBatch,
        TerminalCompressedFriProof, TerminalNpoPolynomialFriColumnSet, TerminalNpoPolynomialTable,
        TerminalNpoPolynomialTableRow, TerminalNpoRowKind,
        TerminalNpoTip5PackedLookupAirLogupSelectedTraceBridgeProof,
        TerminalProductionNpoPolynomialProof, TerminalProductionProof,
        TerminalR1csRowProductSumcheckProof, TerminalSparseR1csRelation,
    };

    use super::*;
    use crate::composite_proof::{build_config, composite_prove_pinned_logup, logup_common_for};
    use crate::composite_public::CompositePublicInputs;
    use crate::composite_trace::CompositeTrace;
    use crate::params::ZkParams;
    use crate::CircuitConfig;

    struct RecScalarValMmcs<const DIGEST_ELEMS: usize, H, C>(core::marker::PhantomData<(H, C)>);

    impl<const DIGEST_ELEMS: usize, H, C> p3_recursion::RecursiveMmcs<Val, Challenge>
        for RecScalarValMmcs<DIGEST_ELEMS, H, C>
    where
        H: p3_symmetric::CryptographicHasher<Val, [Val; DIGEST_ELEMS]> + Sync,
        C: p3_symmetric::PseudoCompressionFunction<[Val; DIGEST_ELEMS], 2> + Sync,
        [Val; DIGEST_ELEMS]: serde::Serialize + for<'a> serde::Deserialize<'a>,
    {
        type Input = p3_merkle_tree::MerkleTreeMmcs<Val, Val, H, C, 2, DIGEST_ELEMS>;
        type Commitment = MerkleCapTargets<Val, DIGEST_ELEMS>;
        type Proof = p3_recursion::pcs::fri::HashProofTargets<Val, DIGEST_ELEMS>;
    }

    type L2Hash = p3_symmetric::PaddingFreeSponge<RecTip5Perm, WIDTH, RATE, DIGEST_ELEMS>;
    type L2Compress = p3_symmetric::TruncatedPermutation<RecTip5Perm, 2, DIGEST_ELEMS, WIDTH>;
    type L2ValMmcs = p3_merkle_tree::MerkleTreeMmcs<Val, Val, L2Hash, L2Compress, 2, DIGEST_ELEMS>;
    type L2ChallengeMmcs = p3_commit::ExtensionMmcs<Val, Challenge, L2ValMmcs>;
    type L2Comm = MerkleCapTargets<Val, DIGEST_ELEMS>;
    type L2RecValMmcs = RecScalarValMmcs<DIGEST_ELEMS, L2Hash, L2Compress>;
    type L2InputProof = InputProofTargets<Val, Challenge, L2RecValMmcs>;
    type L2InnerFri = FriProofTargets<
        Val,
        Challenge,
        RecExtensionValMmcs<Val, Challenge, DIGEST_ELEMS, L2RecValMmcs>,
        L2InputProof,
        Witness<Val>,
    >;

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

    fn statement_digest_public_values_for_l1(built: &BuiltCompositeL1) -> Vec<Val> {
        built
            .public_inputs
            .iter()
            .take(DIGEST_ELEMS)
            .flat_map(|value| {
                <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(value)
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>()
    }

    fn tip5_recompose_table_provers_for_l2(
    ) -> Vec<Box<dyn p3_circuit_prover::TableProver<p3_circuit_prover::config::GoldilocksTipsConfig>>>
    {
        use p3_circuit_prover::{
            recompose_table_provers, ConstraintProfile, TableProver, Tip5Prover,
        };

        let mut provers: Vec<
            Box<dyn TableProver<p3_circuit_prover::config::GoldilocksTipsConfig>>,
        > = vec![Box::new(Tip5Prover::new(
            Tip5Config::GOLDILOCKS_W16,
            ConstraintProfile::Standard,
        ))];
        provers.extend(recompose_table_provers::<
            p3_circuit_prover::config::GoldilocksTipsConfig,
            2,
        >(1, true));
        provers
    }

    struct L1ProofForTestPearl {
        proof: AiPowL1OuterProof,
        timings: L1ProofTimingsForTestPearl,
    }

    struct L1OuterPrepForTestPearl {
        circuit_prover_data: std::rc::Rc<
            p3_circuit_prover::CircuitProverData<p3_circuit_prover::config::GoldilocksTipsConfig>,
        >,
        prover:
            p3_circuit_prover::BatchStarkProver<p3_circuit_prover::config::GoldilocksTipsConfig>,
        timings: L1PrepTimingsForTestPearl,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct L1PrepTimingsForTestPearl {
        air_setup_ms: u128,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct L1ProofTimingsForTestPearl {
        witness_run_ms: u128,
        stark_prove_ms: u128,
    }

    struct L2ProofForTestPearl {
        proof: AiPowL1OuterProof,
        circuit_prover_data: std::rc::Rc<
            p3_circuit_prover::CircuitProverData<p3_circuit_prover::config::GoldilocksTipsConfig>,
        >,
        timings: L2ProofTimingsForTestPearl,
    }

    #[derive(Clone, Debug, Default)]
    struct L2Tip5ProfileForTestPearl {
        circuit_ops: usize,
        primitive_ops: usize,
        hint_ops: usize,
        non_primitive_ops: usize,
        non_primitive_by_type: std::collections::BTreeMap<String, usize>,
        tip5_ops: usize,
        tip5_mmcs_ops: usize,
        tip5_non_mmcs_ops: usize,
        mmcs_ids_not_tip5: usize,
        tagged_tip5_ops: usize,
        tag_categories: std::collections::BTreeMap<String, usize>,
        trace_rows_by_type: std::collections::BTreeMap<String, usize>,
        tip5_trace_rows: usize,
        tip5_trace_pow2_height: usize,
        tip5_rows_over_previous_pow2: usize,
        tip5_rows_to_current_pow2: usize,
        tip5_new_start_rows: usize,
        tip5_mmcs_bit_rows: usize,
        tip5_exposed_input_limbs: usize,
        tip5_exposed_output_limbs: usize,
        witness_run_ms: u128,
    }

    struct L2VerifierPrepForTestPearl {
        verification_circuit: p3_circuit::Circuit<Challenge>,
        verifier_inputs: BatchStarkVerifierInputsBuilder<
            p3_circuit_prover::config::GoldilocksTipsConfig,
            L2Comm,
            L2InnerFri,
        >,
        mmcs_op_ids: Vec<NonPrimitiveOpId>,
        circuit_prover_data: std::rc::Rc<
            p3_circuit_prover::CircuitProverData<p3_circuit_prover::config::GoldilocksTipsConfig>,
        >,
        prover:
            p3_circuit_prover::BatchStarkProver<p3_circuit_prover::config::GoldilocksTipsConfig>,
        l2_statement_public_binding_lanes: usize,
        timings: L2PrepTimingsForTestPearl,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum L2VerifierProfileModeForTestPearl {
        FullNative,
        FullNativeWithChallengerPhaseTags,
        SkipPreprocessedTranscript,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct L2PrepTimingsForTestPearl {
        circuit_define_ms: u128,
        circuit_build_ms: u128,
        air_setup_ms: u128,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct L2ProofTimingsForTestPearl {
        input_pack_ms: u128,
        witness_run_ms: u128,
        stark_prove_ms: u128,
        stark_verify_ms: u128,
    }

    fn pure_query_goldilocks_tip5_fri_shape(
        log_blowup: usize,
        num_queries: usize,
        log_final_poly_len: usize,
        max_log_arity: usize,
        cap_height: usize,
    ) -> p3_circuit_prover::GoldilocksTip5FriShape {
        p3_circuit_prover::GoldilocksTip5FriShape {
            log_blowup,
            log_final_poly_len,
            max_log_arity,
            num_queries,
            commit_pow_bits:
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            query_pow_bits:
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            cap_height,
        }
    }

    fn l2_public_values_for_l1(
        l1: &AiPowL1OuterProof,
        statement_digest_public_values: &[Val],
    ) -> Vec<Vec<Val>> {
        use p3_circuit::ops::PrimitiveOpType;
        use p3_circuit_prover::batch_stark_prover::NUM_PRIMITIVE_TABLES;

        let mut public_values = Vec::with_capacity(NUM_PRIMITIVE_TABLES + l1.non_primitives.len());
        public_values.resize_with(NUM_PRIMITIVE_TABLES, Vec::new);
        let expected_public_values = l1.public_binding_lanes * l1.ext_degree;
        assert_eq!(
            statement_digest_public_values.len(),
            expected_public_values,
            "L2 diagnostic must bind the same statement digest exposed by the L1 Public AIR"
        );
        public_values[PrimitiveOpType::Public as usize] = statement_digest_public_values.to_vec();
        public_values.extend(
            l1.non_primitives
                .iter()
                .map(|entry| entry.public_values.clone()),
        );
        public_values
    }

    fn l2_statement_public_values_for_l1(statement_digest_public_values: &[Val]) -> Vec<Val> {
        let basis_dim = <Challenge as BasedVectorSpace<Val>>::DIMENSION;
        let mut public_values =
            Vec::with_capacity(statement_digest_public_values.len() * basis_dim);
        for &value in statement_digest_public_values {
            let lifted = Challenge::from(value);
            public_values.extend_from_slice(
                <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(&lifted),
            );
        }
        public_values
    }

    fn pure_query_fri_verifier_params_for_l1(
        log_blowup: usize,
        log_final_poly_len: usize,
    ) -> FriVerifierParams {
        FriVerifierParams::with_mmcs(
            log_blowup,
            log_final_poly_len,
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            Tip5Config::GOLDILOCKS_W16,
        )
    }

    fn build_l1_outer_prep_for_test_pearl(
        built: &BuiltCompositeL1,
        l1_config: p3_circuit_prover::config::GoldilocksTipsConfig,
        public_binding_lanes: usize,
    ) -> Result<L1OuterPrepForTestPearl, String> {
        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

        let air_setup_start = std::time::Instant::now();
        let (table_packing, circuit_prover_data) =
            l1_circuit_prover_data_with_config_and_public_binding_lanes(
                built, &l1_config, public_binding_lanes,
            )
            .map_err(|e| format!("L1 prep: {e:?}"))?;
        let air_setup_ms = air_setup_start.elapsed().as_millis();

        let mut prover = BatchStarkProver::new(l1_config).with_table_packing(table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);

        Ok(L1OuterPrepForTestPearl {
            circuit_prover_data: std::rc::Rc::new(circuit_prover_data),
            prover,
            timings: L1PrepTimingsForTestPearl { air_setup_ms },
        })
    }

    fn prove_l1_outer_with_prep_for_test_pearl(
        prep: &L1OuterPrepForTestPearl,
        built: &BuiltCompositeL1,
        proof: &BatchProof<AiPowStarkConfig>,
    ) -> Result<L1ProofForTestPearl, String> {
        let witness_run_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(built, proof)
            .map_err(|e| format!("L1 verifier circuit rejected L0 proof: {e:?}"))?;
        let witness_run_ms = witness_run_start.elapsed().as_millis();

        let stark_prove_start = std::time::Instant::now();
        let proof = prep
            .prover
            .prove_all_tables(&traces, prep.circuit_prover_data.as_ref())
            .map_err(|e| format!("L1 prove_all_tables: {e:?}"))?;
        let stark_prove_ms = stark_prove_start.elapsed().as_millis();

        Ok(L1ProofForTestPearl {
            proof,
            timings: L1ProofTimingsForTestPearl {
                witness_run_ms,
                stark_prove_ms,
            },
        })
    }

    fn build_l2_over_l1_outer_prep_for_test_pearl(
        l1: &AiPowL1OuterProof,
        l1_config: p3_circuit_prover::config::GoldilocksTipsConfig,
        l1_fri_verifier_params: &FriVerifierParams,
        l2_config: p3_circuit_prover::config::GoldilocksTipsConfig,
        l2_log_blowup: usize,
        l2_log_final_poly_len: usize,
    ) -> Result<L2VerifierPrepForTestPearl, String> {
        build_l2_over_l1_outer_prep_for_test_pearl_with_preprocessed_transcript_profile_mode(
            l1,
            l1_config,
            l1_fri_verifier_params,
            l2_config,
            l2_log_blowup,
            l2_log_final_poly_len,
            L2VerifierProfileModeForTestPearl::FullNative,
        )
    }

    fn build_l2_over_l1_outer_prep_for_test_pearl_with_preprocessed_transcript_profile_mode(
        l1: &AiPowL1OuterProof,
        l1_config: p3_circuit_prover::config::GoldilocksTipsConfig,
        l1_fri_verifier_params: &FriVerifierParams,
        l2_config: p3_circuit_prover::config::GoldilocksTipsConfig,
        l2_log_blowup: usize,
        l2_log_final_poly_len: usize,
        verifier_profile_mode: L2VerifierProfileModeForTestPearl,
    ) -> Result<L2VerifierPrepForTestPearl, String> {
        use p3_batch_stark::ProverData;
        use p3_circuit_prover::common::{get_airs_and_degrees_with_prep, NpoPreprocessor};
        use p3_circuit_prover::{
            recompose_air_builders, strip_public_binding_for_lookup_metadata, tip5_air_builders,
            BatchStarkProver, CircuitProverData, ConstraintProfile, RecomposePreprocessor,
            TablePacking, Tip5Preprocessor,
        };

        const TRACE_D: usize = 2;

        let l2_statement_public_binding_lanes = l1.public_binding_lanes * l1.ext_degree;
        if l2_statement_public_binding_lanes == 0 {
            return Err("L2 prep requires non-empty L1 public binding lanes".to_string());
        }
        let circuit_define_start = std::time::Instant::now();
        let mut circuit_builder = CircuitBuilder::<Challenge>::new();
        circuit_builder.enable_tip5_perm::<Tip5Goldilocks, _>(
            generate_tip5_trace::<Challenge, Tip5Goldilocks>, LiftTip5,
        );
        circuit_builder.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
        circuit_builder.set_recompose_coeff_ctl_for_decompose_links(true);

        let lookup_gadget = LogUpGadget::new();
        let l1_table_provers = tip5_recompose_table_provers_for_l2();
        let (verifier_inputs, mmcs_op_ids) = match verifier_profile_mode {
            L2VerifierProfileModeForTestPearl::FullNative => {
                p3_recursion::verifier::verify_p3_batch_proof_circuit::<
                    p3_circuit_prover::config::GoldilocksTipsConfig,
                    L2Comm,
                    L2InputProof,
                    L2InnerFri,
                    LogUpGadget,
                    Tip5Config,
                    WIDTH,
                    RATE,
                    TRACE_D,
                >(
                    &l1_config,
                    &mut circuit_builder,
                    l1,
                    l1_fri_verifier_params,
                    &l1.stark_common,
                    &lookup_gadget,
                    Tip5Config::GOLDILOCKS_W16,
                    &l1_table_provers,
                )
            }
            L2VerifierProfileModeForTestPearl::FullNativeWithChallengerPhaseTags => {
                p3_recursion::verifier::verify_p3_batch_proof_circuit_profile_tag_challenger_phases_for_test_only::<
                    p3_circuit_prover::config::GoldilocksTipsConfig,
                    L2Comm,
                    L2InputProof,
                    L2InnerFri,
                    LogUpGadget,
                    Tip5Config,
                    WIDTH,
                    RATE,
                    TRACE_D,
                >(
                    &l1_config,
                    &mut circuit_builder,
                    l1,
                    l1_fri_verifier_params,
                    &l1.stark_common,
                    &lookup_gadget,
                    Tip5Config::GOLDILOCKS_W16,
                    &l1_table_provers,
                )
            }
            L2VerifierProfileModeForTestPearl::SkipPreprocessedTranscript => {
                p3_recursion::verifier::verify_p3_batch_proof_circuit_profile_skip_preprocessed_transcript_for_test_only::<
                    p3_circuit_prover::config::GoldilocksTipsConfig,
                    L2Comm,
                    L2InputProof,
                    L2InnerFri,
                    LogUpGadget,
                    Tip5Config,
                    WIDTH,
                    RATE,
                    TRACE_D,
                >(
                    &l1_config,
                    &mut circuit_builder,
                    l1,
                    l1_fri_verifier_params,
                    &l1.stark_common,
                    &lookup_gadget,
                    Tip5Config::GOLDILOCKS_W16,
                    &l1_table_provers,
                )
            }
        }
        .map_err(|e| format!("build L2 verifier circuit over L1 proof: {e:?}"))?;
        let circuit_define_ms = circuit_define_start.elapsed().as_millis();

        let circuit_build_start = std::time::Instant::now();
        let verification_circuit = circuit_builder
            .build()
            .map_err(|e| format!("build L2 circuit: {e:?}"))?;
        let circuit_build_ms = circuit_build_start.elapsed().as_millis();

        let air_setup_start = std::time::Instant::now();
        let l2_table_packing = TablePacking::new(l2_statement_public_binding_lanes, 8)
            .with_public_binding_lanes(l2_statement_public_binding_lanes)
            .with_fri_params(l2_log_final_poly_len, l2_log_blowup)
            .with_horner_pack_k(5);
        let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> =
            vec![Box::new(Tip5Preprocessor), Box::new(RecomposePreprocessor::new(true))];
        let mut air_builders =
            tip5_air_builders::<p3_circuit_prover::config::GoldilocksTipsConfig, 2>();
        air_builders.extend(recompose_air_builders::<
            p3_circuit_prover::config::GoldilocksTipsConfig,
            2,
        >(1, true));

        let (airs_degrees, primitive_columns, non_primitive_columns) =
            get_airs_and_degrees_with_prep::<
                p3_circuit_prover::config::GoldilocksTipsConfig,
                Challenge,
                2,
            >(
                &verification_circuit,
                &l2_table_packing,
                &npo_prep,
                &air_builders,
                ConstraintProfile::Standard,
            )
            .map_err(|e| format!("L2 get_airs_and_degrees: {e:?}"))?;
        let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();
        let lookup_metadata_airs = airs
            .iter()
            .map(strip_public_binding_for_lookup_metadata)
            .collect::<Vec<_>>();
        let prover_data =
            ProverData::from_airs_and_degrees(&l2_config, &lookup_metadata_airs, &degrees);
        let circuit_prover_data = std::rc::Rc::new(CircuitProverData::new(
            prover_data, primitive_columns, non_primitive_columns,
        ));
        let air_setup_ms = air_setup_start.elapsed().as_millis();

        let mut prover = BatchStarkProver::new(l2_config).with_table_packing(l2_table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);

        Ok(L2VerifierPrepForTestPearl {
            verification_circuit,
            verifier_inputs,
            mmcs_op_ids,
            circuit_prover_data,
            prover,
            l2_statement_public_binding_lanes,
            timings: L2PrepTimingsForTestPearl {
                circuit_define_ms,
                circuit_build_ms,
                air_setup_ms,
            },
        })
    }

    fn prove_l2_over_l1_outer_with_prep_for_test_pearl(
        prep: &L2VerifierPrepForTestPearl,
        l1: &AiPowL1OuterProof,
        statement_digest_public_values: &[Val],
    ) -> Result<L2ProofForTestPearl, String> {
        let l1_public_values = l2_public_values_for_l1(l1, statement_digest_public_values);
        let l2_statement_public_values =
            l2_statement_public_values_for_l1(statement_digest_public_values);
        if statement_digest_public_values.len() != prep.l2_statement_public_binding_lanes {
            return Err(format!(
                "L2 prep public binding lane mismatch: prep has {}, proof statement has {}",
                prep.l2_statement_public_binding_lanes,
                statement_digest_public_values.len()
            ));
        }
        let input_pack_start = std::time::Instant::now();
        let (public_inputs, private_inputs) = prep
            .verifier_inputs
            .pack_values(&l1_public_values, &l1.proof, &l1.stark_common);
        let input_pack_ms = input_pack_start.elapsed().as_millis();

        let witness_run_start = std::time::Instant::now();
        let mut runner = prep.verification_circuit.runner();
        runner
            .set_public_inputs(&public_inputs)
            .map_err(|e| format!("L2 set public inputs: {e:?}"))?;
        runner
            .set_private_inputs(&private_inputs)
            .map_err(|e| format!("L2 set private inputs: {e:?}"))?;
        set_fri_mmcs_private_data::<
            Val,
            Challenge,
            L2ChallengeMmcs,
            L2ValMmcs,
            L2Hash,
            L2Compress,
            DIGEST_ELEMS,
        >(
            &mut runner,
            &prep.mmcs_op_ids,
            &l1.proof.opening_proof,
            Tip5Config::GOLDILOCKS_W16,
        )
        .map_err(|e| format!("L2 set FRI MMCS private data: {e}"))?;
        let traces = runner
            .run()
            .map_err(|e| format!("L2 verifier circuit rejected L1 proof: {e:?}"))?;
        let witness_run_ms = witness_run_start.elapsed().as_millis();

        let stark_prove_start = std::time::Instant::now();
        let proof = prep
            .prover
            .prove_all_tables(&traces, prep.circuit_prover_data.as_ref())
            .map_err(|e| format!("L2 prove_all_tables: {e:?}"))?;
        let stark_prove_ms = stark_prove_start.elapsed().as_millis();
        let stark_verify_start = std::time::Instant::now();
        prep.prover
            .verify_all_tables_with_public_values(&proof, &l2_statement_public_values)
            .map_err(|e| format!("L2 verify_all_tables: {e:?}"))?;
        let stark_verify_ms = stark_verify_start.elapsed().as_millis();
        Ok(L2ProofForTestPearl {
            proof,
            circuit_prover_data: std::rc::Rc::clone(&prep.circuit_prover_data),
            timings: L2ProofTimingsForTestPearl {
                input_pack_ms,
                witness_run_ms,
                stark_prove_ms,
                stark_verify_ms,
            },
        })
    }

    fn l2_tip5_tag_category_for_test_pearl(tag: &str) -> String {
        if let Some(rest) = tag.strip_prefix("challenger_phase/") {
            let phase = rest.split('/').next().unwrap_or("unknown");
            format!("challenger:{phase}")
        } else if let Some(rest) = tag.strip_prefix("npo_phase/") {
            let phase = rest.split('/').next().unwrap_or("unknown");
            format!("npo:{phase}")
        } else if tag.contains("mmcs") || tag.contains("merkle") || tag.contains("fri") {
            "fri_mmcs".to_string()
        } else if tag.contains("challenger") || tag.contains("transcript") {
            "challenger".to_string()
        } else if tag.contains("public") {
            "public".to_string()
        } else {
            "other".to_string()
        }
    }

    fn profile_l2_tip5_circuit_ops_for_test_pearl(
        prep: &L2VerifierPrepForTestPearl,
    ) -> L2Tip5ProfileForTestPearl {
        use p3_circuit::ops::{NpoTypeId, Op};

        let tip5_op_type = NpoTypeId::tip5_perm(Tip5Config::GOLDILOCKS_W16);
        let mut profile = L2Tip5ProfileForTestPearl {
            circuit_ops: prep.verification_circuit.ops.len(),
            ..L2Tip5ProfileForTestPearl::default()
        };

        let mmcs_ids = prep
            .mmcs_op_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let mut tip5_ids = std::collections::BTreeSet::new();

        for op in &prep.verification_circuit.ops {
            match op {
                Op::Const { .. } | Op::Public { .. } | Op::Alu { .. } => {
                    profile.primitive_ops += 1;
                }
                Op::Hint { .. } => {
                    profile.hint_ops += 1;
                }
                Op::NonPrimitiveOpWithExecutor {
                    executor, op_id, ..
                } => {
                    profile.non_primitive_ops += 1;
                    let op_type = executor.op_type();
                    *profile
                        .non_primitive_by_type
                        .entry(op_type.as_str().to_string())
                        .or_default() += 1;
                    if *op_type == tip5_op_type {
                        tip5_ids.insert(*op_id);
                    }
                }
            }
        }

        profile.tip5_ops = tip5_ids.len();
        profile.tip5_mmcs_ops = tip5_ids.intersection(&mmcs_ids).count();
        profile.tip5_non_mmcs_ops = profile.tip5_ops.saturating_sub(profile.tip5_mmcs_ops);
        profile.mmcs_ids_not_tip5 = mmcs_ids.difference(&tip5_ids).count();

        let mut tagged_tip5_ids = std::collections::BTreeSet::new();
        for (tag, op_id) in &prep.verification_circuit.tag_to_op_id {
            if tip5_ids.contains(op_id) {
                tagged_tip5_ids.insert(*op_id);
                *profile
                    .tag_categories
                    .entry(l2_tip5_tag_category_for_test_pearl(tag))
                    .or_default() += 1;
            }
        }
        profile.tagged_tip5_ops = tagged_tip5_ids.len();

        profile
    }

    fn profile_l2_tip5_verifier_for_test_pearl(
        prep: &L2VerifierPrepForTestPearl,
        l1: &AiPowL1OuterProof,
        statement_digest_public_values: &[Val],
    ) -> Result<L2Tip5ProfileForTestPearl, String> {
        use p3_circuit::ops::{NpoTypeId, Tip5Trace};

        let tip5_op_type = NpoTypeId::tip5_perm(Tip5Config::GOLDILOCKS_W16);
        let mut profile = profile_l2_tip5_circuit_ops_for_test_pearl(prep);

        let l1_public_values = l2_public_values_for_l1(l1, statement_digest_public_values);
        let input_pack_start = std::time::Instant::now();
        let (public_inputs, private_inputs) = prep
            .verifier_inputs
            .pack_values(&l1_public_values, &l1.proof, &l1.stark_common);
        let input_pack_ms = input_pack_start.elapsed().as_millis();

        let witness_run_start = std::time::Instant::now();
        let mut runner = prep.verification_circuit.runner();
        runner
            .set_public_inputs(&public_inputs)
            .map_err(|e| format!("L2 profile set public inputs: {e:?}"))?;
        runner
            .set_private_inputs(&private_inputs)
            .map_err(|e| format!("L2 profile set private inputs: {e:?}"))?;
        set_fri_mmcs_private_data::<
            Val,
            Challenge,
            L2ChallengeMmcs,
            L2ValMmcs,
            L2Hash,
            L2Compress,
            DIGEST_ELEMS,
        >(
            &mut runner,
            &prep.mmcs_op_ids,
            &l1.proof.opening_proof,
            Tip5Config::GOLDILOCKS_W16,
        )
        .map_err(|e| format!("L2 profile set FRI MMCS private data: {e}"))?;
        let traces = runner
            .run()
            .map_err(|e| format!("L2 profile verifier circuit rejected L1 proof: {e:?}"))?;
        profile.witness_run_ms = witness_run_start.elapsed().as_millis();

        for (op_type, trace) in &traces.non_primitive_traces {
            profile
                .trace_rows_by_type
                .insert(op_type.as_str().to_string(), trace.rows());
        }

        if let Some(tip5_trace) = traces.non_primitive_trace::<Tip5Trace<Val>>(&tip5_op_type) {
            profile.tip5_trace_rows = tip5_trace.total_rows();
            profile.tip5_trace_pow2_height = profile.tip5_trace_rows.next_power_of_two();
            let previous_pow2 = profile.tip5_trace_pow2_height / 2;
            profile.tip5_rows_over_previous_pow2 =
                profile.tip5_trace_rows.saturating_sub(previous_pow2);
            profile.tip5_rows_to_current_pow2 = profile
                .tip5_trace_pow2_height
                .saturating_sub(profile.tip5_trace_rows);
            profile.tip5_new_start_rows = tip5_trace
                .operations
                .iter()
                .filter(|row| row.new_start)
                .count();
            profile.tip5_mmcs_bit_rows = tip5_trace
                .operations
                .iter()
                .filter(|row| row.mmcs_bit_ctl)
                .count();
            profile.tip5_exposed_input_limbs = tip5_trace
                .operations
                .iter()
                .map(|row| row.in_ctl.iter().filter(|&&flag| flag).count())
                .sum();
            profile.tip5_exposed_output_limbs = tip5_trace
                .operations
                .iter()
                .map(|row| row.out_ctl.iter().filter(|&&flag| flag).count())
                .sum();
        }

        eprintln!(
            "selected fast-L1 compact L2 Tip5 verifier profile [TEST_PEARL]: circuit_ops={} primitive_ops={} hint_ops={} non_primitive_ops={} non_primitive_by_type={:?} tip5_ops={} tip5_mmcs_ops={} tip5_non_mmcs_ops={} mmcs_ids_not_tip5={} tagged_tip5_ops={} tag_categories={:?} trace_rows_by_type={:?} tip5_trace_rows={} tip5_trace_pow2_height={} tip5_rows_over_previous_pow2={} tip5_rows_to_current_pow2={} tip5_new_start_rows={} tip5_mmcs_bit_rows={} tip5_exposed_input_limbs={} tip5_exposed_output_limbs={} input_pack_ms={} witness_run_ms={}",
            profile.circuit_ops,
            profile.primitive_ops,
            profile.hint_ops,
            profile.non_primitive_ops,
            profile.non_primitive_by_type,
            profile.tip5_ops,
            profile.tip5_mmcs_ops,
            profile.tip5_non_mmcs_ops,
            profile.mmcs_ids_not_tip5,
            profile.tagged_tip5_ops,
            profile.tag_categories,
            profile.trace_rows_by_type,
            profile.tip5_trace_rows,
            profile.tip5_trace_pow2_height,
            profile.tip5_rows_over_previous_pow2,
            profile.tip5_rows_to_current_pow2,
            profile.tip5_new_start_rows,
            profile.tip5_mmcs_bit_rows,
            profile.tip5_exposed_input_limbs,
            profile.tip5_exposed_output_limbs,
            input_pack_ms,
            profile.witness_run_ms,
        );

        Ok(profile)
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
    #[ignore = "relaxed L1-only batch-STARK candidate size accounting is opt-in"]
    fn relaxed_l1_only_candidate_size_breakdown_for_test_pearl() {
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
        let split = measure_relaxed_l1_only_candidate_size(&cert)
            .expect("measure relaxed L1-only candidate size");

        eprintln!(
            "relaxed L1-only candidate size [TEST_PEARL]: current_postcard={} current_fixed_bincode={} l0_proof={} l0_program={} l1_outer={} l1_proof_body={} l1_metadata={} l1_public_binding_lanes={}",
            split.current_checkpoint_postcard_bytes,
            split.current_checkpoint_fixed_bincode_bytes,
            split.l0_proof_postcard_bytes,
            split.l0_program_postcard_bytes,
            split.l1_outer_postcard_bytes,
            split.l1_proof_body_postcard_bytes,
            split.l1_outer_metadata_postcard_bytes,
            split.l1_public_binding_lanes,
        );

        assert_eq!(
            split.l1_public_binding_lanes, 0,
            "current checkpoint intentionally disables L1 public binding; a \
             production L1-only branch must add an equivalent statement binding"
        );
        assert!(
            split.l1_outer_postcard_bytes >= split.l1_proof_body_postcard_bytes,
            "metadata split must be well formed"
        );
    }

    #[test]
    #[ignore = "statement-bound relaxed L1-only candidate size accounting is opt-in"]
    fn relaxed_l1_only_statement_bound_candidate_size_breakdown_for_test_pearl() {
        use std::time::Instant;

        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

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

        let prove_start = Instant::now();
        let outer =
            prove_composite_l1_outer_cert_with_public_binding_lanes(&built, &proof, DIGEST_ELEMS)
                .expect("statement-bound recursive certificate");
        let prove_ms = prove_start.elapsed().as_millis();

        let mut verifier = BatchStarkProver::new(production_l1_stark_config())
            .with_table_packing(production_l1_table_packing(DIGEST_ELEMS));
        verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        verifier.register_recompose_table::<2>(true);

        let statement_digest_public_values = built
            .public_inputs
            .iter()
            .take(DIGEST_ELEMS)
            .flat_map(|value| {
                <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(value)
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();
        let verify_start = Instant::now();
        verifier
            .verify_all_tables_with_public_values(&outer, &statement_digest_public_values)
            .expect("statement-bound L1 outer proof must verify");
        let verify_ms = verify_start.elapsed().as_millis();

        let l1_outer_bytes = postcard_len(&outer, "statement-bound L1 outer proof");
        let l1_proof_body_bytes = postcard_len(&outer.proof, "statement-bound L1 proof body");
        let l1_metadata_bytes = l1_outer_bytes.saturating_sub(l1_proof_body_bytes);
        eprintln!(
            "relaxed L1-only statement-bound candidate [TEST_PEARL]: l1_outer={} l1_proof_body={} l1_metadata={} l1_public_binding_lanes={} prove_ms={} verify_ms={}",
            l1_outer_bytes,
            l1_proof_body_bytes,
            l1_metadata_bytes,
            outer.public_binding_lanes,
            prove_ms,
            verify_ms,
        );

        assert_eq!(
            outer.public_binding_lanes, DIGEST_ELEMS,
            "diagnostic L1 proof must expose the statement digest"
        );
        assert!(
            l1_outer_bytes >= l1_proof_body_bytes,
            "metadata split must be well formed"
        );
    }

    #[test]
    #[ignore = "pure-query statement-bound relaxed L1-only candidate sweep is opt-in"]
    fn relaxed_l1_only_pure_query_statement_bound_candidate_size_breakdown_for_test_pearl() {
        use std::time::Instant;

        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            0
        );
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            0
        );
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_JOHNSON_BITS,
            60
        );

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

        let statement_digest_public_values = built
            .public_inputs
            .iter()
            .take(DIGEST_ELEMS)
            .flat_map(|value| {
                <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(value)
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();

        for (label, log_blowup, num_queries) in [
            ("lb3_nq20", 3usize, 20usize),
            ("lb4_nq15", 4usize, 15usize),
            ("lb5_nq12", 5usize, 12usize),
            ("lb6_nq10", 6usize, 10usize),
        ] {
            let prove_start = Instant::now();
            let outer = prove_composite_l1_outer_cert_with_config_and_public_binding_lanes(
                &built,
                &proof,
                pure_query_l1_stark_config_with_shape(log_blowup, num_queries),
                DIGEST_ELEMS,
            )
            .expect("pure-query statement-bound recursive certificate");
            let prove_ms = prove_start.elapsed().as_millis();

            let mut verifier = BatchStarkProver::new(pure_query_l1_stark_config_with_shape(
                log_blowup, num_queries,
            ))
            .with_table_packing(production_l1_table_packing(DIGEST_ELEMS));
            verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
            verifier.register_recompose_table::<2>(true);

            let verify_start = Instant::now();
            verifier
                .verify_all_tables_with_public_values(&outer, &statement_digest_public_values)
                .expect("pure-query statement-bound L1 outer proof must verify");
            let verify_ms = verify_start.elapsed().as_millis();

            let l1_outer_bytes = postcard_len(&outer, "pure-query statement-bound L1 outer proof");
            let l1_proof_body_bytes =
                postcard_len(&outer.proof, "pure-query statement-bound L1 proof body");
            let l1_metadata_bytes = l1_outer_bytes.saturating_sub(l1_proof_body_bytes);
            let cap_height =
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_CAP_HEIGHT;
            let log_final_poly_len =
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_LOG_FINAL_POLY_LEN;
            let path_estimate = merkle_path_compression_estimate_for_outer_proof(
                &outer, log_blowup, log_final_poly_len, cap_height,
            );
            let path_pruned_projected_outer_bytes = l1_outer_bytes
                .saturating_sub(path_estimate.mean_digest_savings_bytes.round() as usize);
            let input_batches = fri_input_batch_byte_breakdown_for_outer_proof(&outer);
            let preprocessed_input = input_batch_bytes_by_label(&input_batches, "preprocessed")
                .expect("L1 proof should carry a preprocessed input batch");
            let preprocessed_ood_bytes = preprocessed_ood_opening_bytes_for_outer_proof(&outer);
            let path_without_preprocessed =
                merkle_path_compression_estimate_for_outer_proof_with_omitted_input_batch(
                    &outer,
                    log_blowup,
                    log_final_poly_len,
                    cap_height,
                    Some(preprocessed_input.index),
                );
            let preprocessed_omitted_projected_outer_bytes = l1_outer_bytes
                .saturating_sub(preprocessed_ood_bytes)
                .saturating_sub(preprocessed_input.total_bytes)
                .saturating_sub(
                    path_without_preprocessed.mean_digest_savings_bytes.round() as usize
                );
            eprintln!(
                "relaxed L1-only pure-query statement-bound candidate [TEST_PEARL {label}]: l1_outer={} l1_proof_body={} l1_metadata={} l1_path_pruned_projected_outer={} l1_path_raw_siblings={} l1_path_mean_compressed_siblings={} l1_path_mean_digest_savings={} l1_preprocessed_ood={} l1_preprocessed_input_batch={} l1_preprocessed_input_opened_values={} l1_preprocessed_input_merkle={} l1_preprocessed_omitted_projected_outer={} l1_public_binding_lanes={} l1_log_blowup={} l1_num_queries={} l1_cap_height={} l1_commit_pow_bits={} l1_query_pow_bits={} l1_johnson_bits={} prove_ms={} verify_ms={}",
                l1_outer_bytes,
                l1_proof_body_bytes,
                l1_metadata_bytes,
                path_pruned_projected_outer_bytes,
                path_estimate.raw_siblings,
                path_estimate.mean_compressed_siblings.round() as usize,
                path_estimate.mean_digest_savings_bytes.round() as usize,
                preprocessed_ood_bytes,
                preprocessed_input.total_bytes,
                preprocessed_input.opened_values_bytes,
                preprocessed_input.merkle_bytes,
                preprocessed_omitted_projected_outer_bytes,
                outer.public_binding_lanes,
                log_blowup,
                num_queries,
                cap_height,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_JOHNSON_BITS,
                prove_ms,
                verify_ms,
            );

            assert_eq!(
                outer.public_binding_lanes, DIGEST_ELEMS,
                "diagnostic L1 proof must expose the statement digest"
            );
            assert!(
                l1_outer_bytes >= l1_proof_body_bytes,
                "metadata split must be well formed"
            );
        }
    }

    #[test]
    #[ignore = "pure-query L1-only cap-height size accounting is opt-in"]
    fn relaxed_l1_only_pure_query_lb6_cap_height_candidate_size_breakdown_for_test_pearl() {
        use std::time::Instant;

        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

        const LOG_BLOWUP: usize = 6;
        const NUM_QUERIES: usize = 10;
        assert_eq!(LOG_BLOWUP * NUM_QUERIES, 60);
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            0
        );
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            0
        );

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

        let statement_digest_public_values = built
            .public_inputs
            .iter()
            .take(DIGEST_ELEMS)
            .flat_map(|value| {
                <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(value)
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();

        for cap_height in [4usize, 6usize] {
            let prove_start = Instant::now();
            let outer = prove_composite_l1_outer_cert_with_config_and_public_binding_lanes(
                &built,
                &proof,
                pure_query_l1_stark_config_with_shape_and_cap(LOG_BLOWUP, NUM_QUERIES, cap_height),
                DIGEST_ELEMS,
            )
            .expect("pure-query cap-height recursive certificate");
            let prove_ms = prove_start.elapsed().as_millis();

            let mut verifier = BatchStarkProver::new(
                pure_query_l1_stark_config_with_shape_and_cap(LOG_BLOWUP, NUM_QUERIES, cap_height),
            )
            .with_table_packing(production_l1_table_packing(DIGEST_ELEMS));
            verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
            verifier.register_recompose_table::<2>(true);

            let verify_start = Instant::now();
            verifier
                .verify_all_tables_with_public_values(&outer, &statement_digest_public_values)
                .expect("pure-query cap-height L1 outer proof must verify");
            let verify_ms = verify_start.elapsed().as_millis();

            let l1_outer_bytes = postcard_len(&outer, "pure-query cap-height L1 outer proof");
            let l1_proof_body_bytes =
                postcard_len(&outer.proof, "pure-query cap-height L1 proof body");
            let commitments_bytes = postcard_len(
                &outer.proof.commitments, "pure-query cap-height L1 commitments",
            );
            let opened_values_bytes = postcard_len(
                &outer.proof.opened_values, "pure-query cap-height L1 opened values",
            );
            let opening_proof_bytes = postcard_len(
                &outer.proof.opening_proof, "pure-query cap-height L1 opening proof",
            );
            let global_lookup_data_bytes = postcard_len(
                &outer.proof.global_lookup_data, "pure-query cap-height L1 global lookup data",
            );
            let non_primitives_bytes = postcard_len(
                &outer.non_primitives, "pure-query cap-height L1 non-primitives metadata",
            );
            let l1_metadata_bytes = l1_outer_bytes.saturating_sub(l1_proof_body_bytes);
            eprintln!(
                "relaxed L1-only pure-query cap-height candidate [TEST_PEARL lb6_nq10 cap={}]: l1_outer={} l1_proof_body={} l1_metadata={} commitments={} opened_values={} opening_proof={} global_lookup_data={} non_primitives={} l1_public_binding_lanes={} l1_log_blowup={} l1_num_queries={} l1_commit_pow_bits={} l1_query_pow_bits={} l1_johnson_bits={} prove_ms={} verify_ms={}",
                cap_height,
                l1_outer_bytes,
                l1_proof_body_bytes,
                l1_metadata_bytes,
                commitments_bytes,
                opened_values_bytes,
                opening_proof_bytes,
                global_lookup_data_bytes,
                non_primitives_bytes,
                outer.public_binding_lanes,
                LOG_BLOWUP,
                NUM_QUERIES,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
                LOG_BLOWUP * NUM_QUERIES,
                prove_ms,
                verify_ms,
            );

            assert_eq!(
                outer.public_binding_lanes, DIGEST_ELEMS,
                "diagnostic L1 proof must expose the statement digest"
            );
            assert!(
                l1_outer_bytes >= l1_proof_body_bytes,
                "metadata split must be well formed"
            );
        }
    }

    #[test]
    #[ignore = "pure-query L1-only opening-proof byte accounting is opt-in"]
    fn relaxed_l1_only_pure_query_lb6_cap4_opening_breakdown_for_test_pearl() {
        use std::time::Instant;

        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

        const LOG_BLOWUP: usize = 6;
        const NUM_QUERIES: usize = 10;
        const CAP_HEIGHT: usize = 4;
        assert_eq!(LOG_BLOWUP * NUM_QUERIES, 60);
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            0
        );
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            0
        );

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

        let statement_digest_public_values = built
            .public_inputs
            .iter()
            .take(DIGEST_ELEMS)
            .flat_map(|value| {
                <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(value)
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();

        let prove_start = Instant::now();
        let outer = prove_composite_l1_outer_cert_with_config_and_public_binding_lanes(
            &built,
            &proof,
            pure_query_l1_stark_config_with_shape_and_cap(LOG_BLOWUP, NUM_QUERIES, CAP_HEIGHT),
            DIGEST_ELEMS,
        )
        .expect("pure-query cap-4 recursive certificate");
        let prove_ms = prove_start.elapsed().as_millis();

        let mut verifier = BatchStarkProver::new(pure_query_l1_stark_config_with_shape_and_cap(
            LOG_BLOWUP, NUM_QUERIES, CAP_HEIGHT,
        ))
        .with_table_packing(production_l1_table_packing(DIGEST_ELEMS));
        verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        verifier.register_recompose_table::<2>(true);

        let verify_start = Instant::now();
        verifier
            .verify_all_tables_with_public_values(&outer, &statement_digest_public_values)
            .expect("pure-query cap-4 L1 outer proof must verify");
        let verify_ms = verify_start.elapsed().as_millis();

        let total_bytes = postcard_len(&outer, "pure-query cap-4 L1 outer proof");
        let proof_body_bytes = postcard_len(&outer.proof, "pure-query cap-4 L1 proof body");
        let commitments_bytes =
            postcard_len(&outer.proof.commitments, "pure-query cap-4 L1 commitments");
        let opened_values_bytes = postcard_len(
            &outer.proof.opened_values, "pure-query cap-4 L1 opened values",
        );
        let opening_proof_bytes = postcard_len(
            &outer.proof.opening_proof, "pure-query cap-4 L1 opening proof",
        );
        let global_lookup_data_bytes = postcard_len(
            &outer.proof.global_lookup_data, "pure-query cap-4 L1 global lookup data",
        );

        let fri = &outer.proof.opening_proof;
        let commit_phase_commits_bytes = postcard_len(
            &fri.commit_phase_commits, "pure-query cap-4 L1 FRI commit-phase commits",
        );
        let commit_pow_witnesses_bytes = postcard_len(
            &fri.commit_pow_witnesses, "pure-query cap-4 L1 FRI commit PoW witnesses",
        );
        let query_proofs_bytes =
            postcard_len(&fri.query_proofs, "pure-query cap-4 L1 FRI query proofs");
        let final_poly_bytes =
            postcard_len(&fri.final_poly, "pure-query cap-4 L1 FRI final polynomial");
        let query_pow_witness_bytes = postcard_len(
            &fri.query_pow_witness, "pure-query cap-4 L1 FRI query PoW witness",
        );

        let mut input_proofs_total = 0usize;
        let mut input_opened_values_total = 0usize;
        let mut input_merkle_total = 0usize;
        let mut commit_phase_openings_total = 0usize;
        let mut commit_phase_sibling_values_total = 0usize;
        let mut commit_phase_merkle_total = 0usize;
        for query in &fri.query_proofs {
            input_proofs_total += postcard_len(
                &query.input_proof, "pure-query cap-4 L1 FRI query input proof",
            );
            for batch_opening in &query.input_proof {
                input_opened_values_total += postcard_len(
                    &batch_opening.opened_values, "pure-query cap-4 L1 FRI input opened values",
                );
                input_merkle_total += postcard_len(
                    &batch_opening.opening_proof, "pure-query cap-4 L1 FRI input Merkle path",
                );
            }
            commit_phase_openings_total += postcard_len(
                &query.commit_phase_openings, "pure-query cap-4 L1 FRI commit-phase openings",
            );
            for step in &query.commit_phase_openings {
                commit_phase_sibling_values_total += postcard_len(
                    &step.sibling_values, "pure-query cap-4 L1 FRI commit-phase sibling values",
                );
                commit_phase_merkle_total += postcard_len(
                    &step.opening_proof, "pure-query cap-4 L1 FRI commit-phase Merkle path",
                );
            }
        }

        eprintln!(
            "relaxed L1-only pure-query opening breakdown [TEST_PEARL lb6_nq10 cap4]: total={} proof_body={} commitments={} opened_values={} opening_proof={} global_lookup_data={} fri_commit_phase_commits={} fri_commit_pow_witnesses={} fri_query_proofs={} fri_final_poly={} fri_query_pow_witness={} fri_num_queries={} fri_input_proofs={} fri_input_opened_values={} fri_input_merkle={} fri_commit_phase_openings={} fri_commit_phase_sibling_values={} fri_commit_phase_merkle={} l1_public_binding_lanes={} l1_log_blowup={} l1_num_queries={} l1_commit_pow_bits={} l1_query_pow_bits={} l1_johnson_bits={} prove_ms={} verify_ms={}",
            total_bytes,
            proof_body_bytes,
            commitments_bytes,
            opened_values_bytes,
            opening_proof_bytes,
            global_lookup_data_bytes,
            commit_phase_commits_bytes,
            commit_pow_witnesses_bytes,
            query_proofs_bytes,
            final_poly_bytes,
            query_pow_witness_bytes,
            fri.query_proofs.len(),
            input_proofs_total,
            input_opened_values_total,
            input_merkle_total,
            commit_phase_openings_total,
            commit_phase_sibling_values_total,
            commit_phase_merkle_total,
            outer.public_binding_lanes,
            LOG_BLOWUP,
            NUM_QUERIES,
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            LOG_BLOWUP * NUM_QUERIES,
            prove_ms,
            verify_ms,
        );

        assert_eq!(
            outer.public_binding_lanes, DIGEST_ELEMS,
            "diagnostic L1 proof must expose the statement digest"
        );
        assert_eq!(fri.query_proofs.len(), NUM_QUERIES);
        assert!(
            total_bytes >= proof_body_bytes,
            "metadata split must be well formed"
        );
    }

    #[test]
    #[ignore = "pure-query L1-only final-polynomial shape sweep is opt-in"]
    fn relaxed_l1_only_pure_query_lb6_cap4_fri_shape_sweep_for_test_pearl() {
        use std::time::Instant;

        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

        const LOG_BLOWUP: usize = 6;
        const NUM_QUERIES: usize = 10;
        const CAP_HEIGHT: usize = 4;
        const MAX_LOG_ARITY: usize = 3;
        assert_eq!(LOG_BLOWUP * NUM_QUERIES, 60);
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            0
        );
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            0
        );

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

        let statement_digest_public_values = built
            .public_inputs
            .iter()
            .take(DIGEST_ELEMS)
            .flat_map(|value| {
                <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(value)
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();

        for (label, log_final_poly_len) in [("lfp0_mla3", 0usize), ("lfp1_mla3", 1usize)] {
            let prove_start = Instant::now();
            let outer = prove_composite_l1_outer_cert_with_config_and_public_binding_lanes(
                &built,
                &proof,
                pure_query_l1_stark_config_with_fri_shape(
                    LOG_BLOWUP, NUM_QUERIES, log_final_poly_len, MAX_LOG_ARITY, CAP_HEIGHT,
                ),
                DIGEST_ELEMS,
            )
            .expect("pure-query FRI-shape recursive certificate");
            let prove_ms = prove_start.elapsed().as_millis();

            let mut verifier = BatchStarkProver::new(pure_query_l1_stark_config_with_fri_shape(
                LOG_BLOWUP, NUM_QUERIES, log_final_poly_len, MAX_LOG_ARITY, CAP_HEIGHT,
            ))
            .with_table_packing(production_l1_table_packing(DIGEST_ELEMS));
            verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
            verifier.register_recompose_table::<2>(true);

            let verify_start = Instant::now();
            verifier
                .verify_all_tables_with_public_values(&outer, &statement_digest_public_values)
                .expect("pure-query FRI-shape L1 outer proof must verify");
            let verify_ms = verify_start.elapsed().as_millis();

            let l1_outer_bytes = postcard_len(&outer, "pure-query FRI-shape L1 outer proof");
            let l1_proof_body_bytes =
                postcard_len(&outer.proof, "pure-query FRI-shape L1 proof body");
            let commitments_bytes = postcard_len(
                &outer.proof.commitments, "pure-query FRI-shape L1 commitments",
            );
            let opened_values_bytes = postcard_len(
                &outer.proof.opened_values, "pure-query FRI-shape L1 opened values",
            );
            let opening_proof_bytes = postcard_len(
                &outer.proof.opening_proof, "pure-query FRI-shape L1 opening proof",
            );
            let global_lookup_data_bytes = postcard_len(
                &outer.proof.global_lookup_data, "pure-query FRI-shape L1 global lookup data",
            );
            let fri = &outer.proof.opening_proof;
            let fri_commit_phase_commits_bytes = postcard_len(
                &fri.commit_phase_commits, "pure-query FRI-shape L1 FRI commit-phase commits",
            );
            let fri_query_proofs_bytes = postcard_len(
                &fri.query_proofs, "pure-query FRI-shape L1 FRI query proofs",
            );
            let fri_final_poly_bytes = postcard_len(
                &fri.final_poly, "pure-query FRI-shape L1 FRI final polynomial",
            );
            eprintln!(
                "relaxed L1-only pure-query FRI-shape candidate [TEST_PEARL lb6_nq10 cap4 {label}]: l1_outer={} l1_proof_body={} commitments={} opened_values={} opening_proof={} global_lookup_data={} fri_commit_phase_commits={} fri_query_proofs={} fri_final_poly={} fri_num_queries={} l1_public_binding_lanes={} l1_log_blowup={} l1_num_queries={} l1_log_final_poly_len={} l1_max_log_arity={} l1_commit_pow_bits={} l1_query_pow_bits={} l1_johnson_bits={} prove_ms={} verify_ms={}",
                l1_outer_bytes,
                l1_proof_body_bytes,
                commitments_bytes,
                opened_values_bytes,
                opening_proof_bytes,
                global_lookup_data_bytes,
                fri_commit_phase_commits_bytes,
                fri_query_proofs_bytes,
                fri_final_poly_bytes,
                fri.query_proofs.len(),
                outer.public_binding_lanes,
                LOG_BLOWUP,
                NUM_QUERIES,
                log_final_poly_len,
                MAX_LOG_ARITY,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
                LOG_BLOWUP * NUM_QUERIES,
                prove_ms,
                verify_ms,
            );

            assert_eq!(
                outer.public_binding_lanes, DIGEST_ELEMS,
                "diagnostic L1 proof must expose the statement digest"
            );
            assert_eq!(fri.query_proofs.len(), NUM_QUERIES);
            assert!(
                l1_outer_bytes >= l1_proof_body_bytes,
                "metadata split must be well formed"
            );
        }
    }

    fn run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
        l1_label: &str,
        l1_log_blowup: usize,
        l1_num_queries: usize,
        l1_cap_height: usize,
        l1_log_final_poly_len: usize,
        l2_shapes: &[(&str, usize, usize, usize, usize, usize)],
    ) {
        use std::time::Instant;

        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

        assert!(
            l1_log_blowup * l1_num_queries >= 60,
            "L1 pure-query diagnostic must carry at least 60 Johnson bits"
        );
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
            0
        );
        assert_eq!(
            p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
            0
        );

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

        let statement_digest_public_values = statement_digest_public_values_for_l1(&built);
        let l1_config = pure_query_l1_stark_config_with_shape_and_cap(
            l1_log_blowup, l1_num_queries, l1_cap_height,
        );
        let l1_total_prove_start = Instant::now();
        let l1_prep_wall_start = Instant::now();
        let l1_prep = build_l1_outer_prep_for_test_pearl(&built, l1_config.clone(), DIGEST_ELEMS)
            .expect("pure-query cap-4 L1 prep");
        let l1_prep_wall_ms = l1_prep_wall_start.elapsed().as_millis();
        let l1_prep_timings = l1_prep.timings;
        let l1_cached_prove_start = Instant::now();
        let l1_artifact = prove_l1_outer_with_prep_for_test_pearl(&l1_prep, &built, &proof)
            .expect("pure-query cap-4 L1 recursive certificate with cached prep");
        let l1_cached_prove_ms = l1_cached_prove_start.elapsed().as_millis();
        let l1_prove_ms = l1_total_prove_start.elapsed().as_millis();
        let L1ProofForTestPearl {
            proof: l1,
            timings: l1_timings,
        } = l1_artifact;

        let mut l1_verifier = BatchStarkProver::new(l1_config.clone())
            .with_table_packing(production_l1_table_packing(DIGEST_ELEMS));
        l1_verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        l1_verifier.register_recompose_table::<2>(true);
        let l1_verify_start = Instant::now();
        l1_verifier
            .verify_all_tables_with_public_values(&l1, &statement_digest_public_values)
            .expect("statement-bound L1 proof must verify before L2 wrapping");
        let l1_verify_ms = l1_verify_start.elapsed().as_millis();

        let l1_outer_bytes = postcard_len(&l1, "pure-query L2 diagnostic L1 outer proof");
        let l1_proof_body_bytes = postcard_len(&l1.proof, "pure-query L2 diagnostic L1 proof body");
        let l1_path_estimate = merkle_path_compression_estimate_for_outer_proof(
            &l1, l1_log_blowup, l1_log_final_poly_len, l1_cap_height,
        );
        let l1_path_pruned_projected_outer_bytes = l1_outer_bytes
            .saturating_sub(l1_path_estimate.mean_digest_savings_bytes.round() as usize);
        let l1_input_batches = fri_input_batch_byte_breakdown_for_outer_proof(&l1);
        let l1_preprocessed_input = input_batch_bytes_by_label(&l1_input_batches, "preprocessed")
            .expect("L1 proof should carry a preprocessed input batch");
        let l1_preprocessed_ood_bytes = preprocessed_ood_opening_bytes_for_outer_proof(&l1);
        let l1_path_without_preprocessed =
            merkle_path_compression_estimate_for_outer_proof_with_omitted_input_batch(
                &l1,
                l1_log_blowup,
                l1_log_final_poly_len,
                l1_cap_height,
                Some(l1_preprocessed_input.index),
            );
        let l1_preprocessed_omitted_projected_outer_bytes = l1_outer_bytes
            .saturating_sub(l1_preprocessed_ood_bytes)
            .saturating_sub(l1_preprocessed_input.total_bytes)
            .saturating_sub(
                l1_path_without_preprocessed
                    .mean_digest_savings_bytes
                    .round() as usize,
            );

        assert_eq!(
            l1.public_binding_lanes, DIGEST_ELEMS,
            "L1 proof must expose the statement digest before L2 wrapping"
        );

        for (
            l2_label,
            l2_log_blowup,
            l2_num_queries,
            l2_log_final_poly_len,
            l2_max_log_arity,
            l2_cap_height,
        ) in l2_shapes.iter().copied()
        {
            assert!(
                l2_log_blowup * l2_num_queries >= 60,
                "L2 pure-query diagnostic must carry at least 60 Johnson bits"
            );
            let l2_config = pure_query_l1_stark_config_with_fri_shape(
                l2_log_blowup, l2_num_queries, l2_log_final_poly_len, l2_max_log_arity,
                l2_cap_height,
            );
            let l2_total_prove_start = Instant::now();
            let l2_prep_wall_start = Instant::now();
            let l2_prep = build_l2_over_l1_outer_prep_for_test_pearl(
                &l1,
                l1_config.clone(),
                &pure_query_fri_verifier_params_for_l1(l1_log_blowup, l1_log_final_poly_len),
                l2_config.clone(),
                l2_log_blowup,
                l2_log_final_poly_len,
            )
            .expect("pure-query L2 prep over statement-bound L1");
            let l2_prep_wall_ms = l2_prep_wall_start.elapsed().as_millis();
            let l2_prep_timings = l2_prep.timings;
            let l2_cached_prove_start = Instant::now();
            let l2_artifact = prove_l2_over_l1_outer_with_prep_for_test_pearl(
                &l2_prep, &l1, &statement_digest_public_values,
            )
            .expect("pure-query L2 proof over statement-bound L1 with cached prep");
            let l2_cached_prove_ms = l2_cached_prove_start.elapsed().as_millis();
            let l2_prove_ms = l2_total_prove_start.elapsed().as_millis();
            let L2ProofForTestPearl {
                proof: l2,
                circuit_prover_data: l2_circuit_prover_data,
                timings: l2_timings,
            } = l2_artifact;

            let l2_outer_bytes = postcard_len(&l2, "pure-query L2 diagnostic L2 outer proof");
            let l2_proof_body_bytes =
                postcard_len(&l2.proof, "pure-query L2 diagnostic L2 proof body");
            let l2_commitments_bytes = postcard_len(
                &l2.proof.commitments, "pure-query L2 diagnostic commitments",
            );
            let l2_opened_values_bytes = postcard_len(
                &l2.proof.opened_values, "pure-query L2 diagnostic opened values",
            );
            let l2_opening_proof_bytes = postcard_len(
                &l2.proof.opening_proof, "pure-query L2 diagnostic opening proof",
            );
            let l2_global_lookup_data_bytes = postcard_len(
                &l2.proof.global_lookup_data, "pure-query L2 diagnostic global lookup data",
            );
            let l2_path_estimate = merkle_path_compression_estimate_for_outer_proof(
                &l2, l2_log_blowup, l2_log_final_poly_len, l2_cap_height,
            );
            let l2_path_pruned_projected_outer_bytes = l2_outer_bytes
                .saturating_sub(l2_path_estimate.mean_digest_savings_bytes.round() as usize);
            let l2_input_batches = fri_input_batch_byte_breakdown_for_outer_proof(&l2);
            let l2_preprocessed_input =
                input_batch_bytes_by_label(&l2_input_batches, "preprocessed")
                    .expect("L2 proof should carry a preprocessed input batch");
            let l2_preprocessed_ood_bytes = preprocessed_ood_opening_bytes_for_outer_proof(&l2);
            let l2_path_without_preprocessed =
                merkle_path_compression_estimate_for_outer_proof_with_omitted_input_batch(
                    &l2,
                    l2_log_blowup,
                    l2_log_final_poly_len,
                    l2_cap_height,
                    Some(l2_preprocessed_input.index),
                );
            let l2_preprocessed_omitted_projected_outer_bytes = l2_outer_bytes
                .saturating_sub(l2_preprocessed_ood_bytes)
                .saturating_sub(l2_preprocessed_input.total_bytes)
                .saturating_sub(
                    l2_path_without_preprocessed
                        .mean_digest_savings_bytes
                        .round() as usize,
                );
            let l2_statement_public_values =
                l2_statement_public_values_for_l1(&statement_digest_public_values);
            let l2_statement_public_binding_lanes = statement_digest_public_values.len();
            let l2_public_binding_lanes = l2.public_binding_lanes;
            assert_eq!(
                l2_public_binding_lanes, l2_statement_public_binding_lanes,
                "L2 compact candidate must expose all L1 statement-digest base limbs as final proof public lanes"
            );

            let l2_fri_shape = pure_query_goldilocks_tip5_fri_shape(
                l2_log_blowup, l2_num_queries, l2_log_final_poly_len, l2_max_log_arity,
                l2_cap_height,
            );
            let mut l2_compact_verifier = BatchStarkProver::new(l2_config.clone())
                .with_table_packing(
                    p3_circuit_prover::TablePacking::new(l2_statement_public_binding_lanes, 8)
                        .with_public_binding_lanes(l2_statement_public_binding_lanes)
                        .with_fri_params(l2_log_final_poly_len, l2_log_blowup)
                        .with_horner_pack_k(5),
                );
            l2_compact_verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
            l2_compact_verifier.register_recompose_table::<2>(true);
            let l2_metadata =
                p3_circuit_prover::GoldilocksTip5BatchStarkProofMetadata::from_proof(&l2);
            let l2_compact_start = Instant::now();
            let l2_compact = l2_compact_verifier
                .compact_goldilocks_tip5_path_pruned_preprocessed_with_public_values(
                    l2,
                    &l2_statement_public_values,
                    l2_circuit_prover_data.as_ref(),
                    l2_fri_shape,
                )
                .expect("compact L2 proof with path-pruned preprocessed adapter");
            let l2_compact_ms = l2_compact_start.elapsed().as_millis();
            let l2_compact_bytes =
                postcard_len(&l2_compact, "pure-query L2 diagnostic actual compact proof");
            let l2_compact_body = l2_compact.into_body();
            let l2_compact_body_bytes = postcard_len(
                &l2_compact_body, "pure-query L2 diagnostic actual compact proof body",
            );
            let l2_compact_proof_body_bytes = postcard_len(
                &l2_compact_body.proof, "pure-query L2 diagnostic actual compact core proof body",
            );
            let l2_compact_restoration_bytes = postcard_len(
                &(
                    l2_compact_body.fri_shape, &l2_compact_body.input_batch_paths,
                    &l2_compact_body.commit_phase_paths,
                ),
                "pure-query L2 diagnostic compact restoration payload",
            );
            let l2_compact_input_paths_bytes = postcard_len(
                &l2_compact_body.input_batch_paths,
                "pure-query L2 diagnostic compact input path dictionaries",
            );
            let l2_compact_commit_paths_bytes = postcard_len(
                &l2_compact_body.commit_phase_paths,
                "pure-query L2 diagnostic compact commit-phase path dictionaries",
            );
            let (
                l2_compact_path_sets,
                l2_compact_original_orders,
                l2_compact_pruned_paths,
                l2_compact_pruned_siblings,
            ) = compact_path_dictionary_stats(
                &l2_compact_body.input_batch_paths, &l2_compact_body.commit_phase_paths,
            );
            let l2_compact_frontier_siblings = compact_path_frontier_sibling_count(
                &l2_compact_body.input_batch_paths, &l2_compact_body.commit_phase_paths,
            );
            let l2_compact_frontier_sibling_savings =
                l2_compact_pruned_siblings.saturating_sub(l2_compact_frontier_siblings);
            let l2_compact_frontier_digest_savings_bytes =
                l2_compact_frontier_sibling_savings * core::mem::size_of::<[Val; DIGEST_ELEMS]>();
            let l2_compact_verify_start = Instant::now();
            let l2_compact_context =
                p3_circuit_prover::GoldilocksTip5PathPrunedCompactVerifierContext::new(
                    &l2_metadata,
                    l2_circuit_prover_data.as_ref(),
                    l2_fri_shape,
                    &l2_statement_public_values,
                );
            l2_compact_verifier
                .verify_goldilocks_tip5_path_pruned_preprocessed_compact_body_with_context(
                    l2_compact_body, l2_compact_context,
                )
                .expect("actual compact L2 proof must verify");
            let l2_compact_verify_ms = l2_compact_verify_start.elapsed().as_millis();

            eprintln!(
                "pure-query L2-over-L1 statement-bound candidate [TEST_PEARL L1 {l1_label} -> L2 {l2_label}]: l1_outer={} l1_proof_body={} l1_path_pruned_projected_outer={} l1_path_raw_siblings={} l1_path_mean_compressed_siblings={} l1_path_mean_digest_savings={} l1_preprocessed_ood={} l1_preprocessed_input_batch={} l1_preprocessed_input_opened_values={} l1_preprocessed_input_merkle={} l1_preprocessed_omitted_projected_outer={} l1_public_binding_lanes={} l1_log_blowup={} l1_num_queries={} l1_cap_height={} l1_commit_pow_bits={} l1_query_pow_bits={} l1_johnson_bits={} l1_prove_ms={} l1_prep_wall_ms={} l1_cached_prove_ms={} l1_air_setup_ms={} l1_witness_run_ms={} l1_stark_prove_ms={} l1_verify_ms={} l2_outer={} l2_proof_body={} l2_metadata={} l2_commitments={} l2_opened_values={} l2_opening_proof={} l2_global_lookup_data={} l2_path_pruned_projected_outer={} l2_path_raw_siblings={} l2_path_mean_compressed_siblings={} l2_path_mean_digest_savings={} l2_preprocessed_ood={} l2_preprocessed_input_batch={} l2_preprocessed_input_opened_values={} l2_preprocessed_input_merkle={} l2_preprocessed_omitted_projected_outer={} l2_actual_compact={} l2_actual_compact_body={} l2_actual_compact_proof_body={} l2_actual_compact_restoration={} l2_actual_compact_input_paths={} l2_actual_compact_commit_paths={} l2_actual_compact_path_sets={} l2_actual_compact_original_orders={} l2_actual_compact_pruned_paths={} l2_actual_compact_pruned_siblings={} l2_actual_compact_frontier_siblings={} l2_actual_compact_frontier_sibling_savings={} l2_actual_compact_frontier_digest_savings={} l2_actual_compact_build_ms={} l2_actual_compact_body_verify_ms={} l2_public_binding_lanes={} l2_log_blowup={} l2_num_queries={} l2_log_final_poly_len={} l2_max_log_arity={} l2_cap_height={} l2_commit_pow_bits={} l2_query_pow_bits={} l2_johnson_bits={} l2_prep_wall_ms={} l2_cached_prove_ms={} l2_circuit_define_ms={} l2_circuit_build_ms={} l2_input_pack_ms={} l2_air_setup_ms={} l2_witness_run_ms={} l2_stark_prove_ms={} l2_stark_verify_ms={} l2_prove_ms={}",
                l1_outer_bytes,
                l1_proof_body_bytes,
                l1_path_pruned_projected_outer_bytes,
                l1_path_estimate.raw_siblings,
                l1_path_estimate.mean_compressed_siblings.round() as usize,
                l1_path_estimate.mean_digest_savings_bytes.round() as usize,
                l1_preprocessed_ood_bytes,
                l1_preprocessed_input.total_bytes,
                l1_preprocessed_input.opened_values_bytes,
                l1_preprocessed_input.merkle_bytes,
                l1_preprocessed_omitted_projected_outer_bytes,
                l1.public_binding_lanes,
                l1_log_blowup,
                l1_num_queries,
                l1_cap_height,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
                l1_log_blowup * l1_num_queries,
                l1_prove_ms,
                l1_prep_wall_ms,
                l1_cached_prove_ms,
                l1_prep_timings.air_setup_ms,
                l1_timings.witness_run_ms,
                l1_timings.stark_prove_ms,
                l1_verify_ms,
                l2_outer_bytes,
                l2_proof_body_bytes,
                l2_outer_bytes.saturating_sub(l2_proof_body_bytes),
                l2_commitments_bytes,
                l2_opened_values_bytes,
                l2_opening_proof_bytes,
                l2_global_lookup_data_bytes,
                l2_path_pruned_projected_outer_bytes,
                l2_path_estimate.raw_siblings,
                l2_path_estimate.mean_compressed_siblings.round() as usize,
                l2_path_estimate.mean_digest_savings_bytes.round() as usize,
                l2_preprocessed_ood_bytes,
                l2_preprocessed_input.total_bytes,
                l2_preprocessed_input.opened_values_bytes,
                l2_preprocessed_input.merkle_bytes,
                l2_preprocessed_omitted_projected_outer_bytes,
                l2_compact_bytes,
                l2_compact_body_bytes,
                l2_compact_proof_body_bytes,
                l2_compact_restoration_bytes,
                l2_compact_input_paths_bytes,
                l2_compact_commit_paths_bytes,
                l2_compact_path_sets,
                l2_compact_original_orders,
                l2_compact_pruned_paths,
                l2_compact_pruned_siblings,
                l2_compact_frontier_siblings,
                l2_compact_frontier_sibling_savings,
                l2_compact_frontier_digest_savings_bytes,
                l2_compact_ms,
                l2_compact_verify_ms,
                l2_public_binding_lanes,
                l2_log_blowup,
                l2_num_queries,
                l2_log_final_poly_len,
                l2_max_log_arity,
                l2_cap_height,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
                l2_log_blowup * l2_num_queries,
                l2_prep_wall_ms,
                l2_cached_prove_ms,
                l2_prep_timings.circuit_define_ms,
                l2_prep_timings.circuit_build_ms,
                l2_timings.input_pack_ms,
                l2_prep_timings.air_setup_ms,
                l2_timings.witness_run_ms,
                l2_timings.stark_prove_ms,
                l2_timings.stark_verify_ms,
                l2_prove_ms,
            );

            assert_eq!(
                l2_public_binding_lanes, l2_statement_public_binding_lanes,
                "diagnostic L2 proof must bind its L1 statement digest as final public lanes"
            );
            assert!(
                l2_outer_bytes >= l2_proof_body_bytes,
                "L2 metadata split must be well formed"
            );
            assert!(
                l2_compact_bytes < l2_outer_bytes,
                "actual compact L2 proof should be smaller than the full L2 proof"
            );
            assert!(
                l2_compact_body_bytes < l2_compact_bytes,
                "metadata-free compact L2 body should be smaller than the compact wrapper"
            );
        }
    }

    #[test]
    #[ignore = "pure-query AI-PoW L2-over-L1 recursive compression measurement is opt-in"]
    fn pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl() {
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb6_nq10_cap4",
            6,
            10,
            4,
            2,
            &[
                ("lb4_nq15_lfp2_mla3_cap4", 4, 15, 2, 3, 4),
                ("lb5_nq12_lfp2_mla3_cap4", 5, 12, 2, 3, 4),
                ("lb6_nq10_lfp2_mla3_cap4", 6, 10, 2, 3, 4),
            ],
        );
    }

    #[test]
    #[ignore = "fast-L1 pure-query AI-PoW L2 recursive compression measurement is opt-in"]
    fn pure_query_l2_over_fast_l1_statement_bound_candidate_size_breakdown_for_test_pearl() {
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb3_nq20_cap4",
            3,
            20,
            4,
            2,
            &[
                ("lb4_nq15_lfp2_mla3_cap4", 4, 15, 2, 3, 4),
                ("lb5_nq12_lfp2_mla3_cap4", 5, 12, 2, 3, 4),
                ("lb6_nq10_lfp2_mla3_cap4", 6, 10, 2, 3, 4),
            ],
        );
    }

    #[test]
    #[ignore = "selected fast-L1 compact L2 candidate timing breakdown is opt-in"]
    fn pure_query_l2_over_fast_l1_selected_candidate_timing_breakdown_for_test_pearl() {
        init_batch_stark_profile_tracing_for_test_pearl();
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb3_nq20_cap4",
            3,
            20,
            4,
            2,
            &[("lb5_nq12_lfp2_mla3_cap4", 5, 12, 2, 3, 4)],
        );
    }

    #[test]
    #[ignore = "selected compact L2 verifier Tip5 trace profile is opt-in"]
    fn selected_fast_l1_compact_l2_tip5_verifier_profile_for_test_pearl() {
        const L1_LOG_BLOWUP: usize = 3;
        const L1_NUM_QUERIES: usize = 20;
        const L1_CAP_HEIGHT: usize = 4;
        const L1_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_LOG_BLOWUP: usize = 5;
        const L2_NUM_QUERIES: usize = 12;
        const L2_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_MAX_LOG_ARITY: usize = 3;
        const L2_CAP_HEIGHT: usize = 4;

        assert_eq!(L1_LOG_BLOWUP * L1_NUM_QUERIES, 60);
        assert_eq!(L2_LOG_BLOWUP * L2_NUM_QUERIES, 60);

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

        let statement_digest_public_values = statement_digest_public_values_for_l1(&built);
        let l1_config = pure_query_l1_stark_config_with_shape_and_cap(
            L1_LOG_BLOWUP, L1_NUM_QUERIES, L1_CAP_HEIGHT,
        );
        let l1_prep = build_l1_outer_prep_for_test_pearl(&built, l1_config.clone(), DIGEST_ELEMS)
            .expect("selected L1 prep");
        let l1 = prove_l1_outer_with_prep_for_test_pearl(&l1_prep, &built, &proof)
            .expect("selected L1 proof with cached prep")
            .proof;
        assert_eq!(
            l1.public_binding_lanes, DIGEST_ELEMS,
            "selected L1 proof must expose the statement digest"
        );

        let l2_config = pure_query_l1_stark_config_with_fri_shape(
            L2_LOG_BLOWUP, L2_NUM_QUERIES, L2_LOG_FINAL_POLY_LEN, L2_MAX_LOG_ARITY, L2_CAP_HEIGHT,
        );
        let l2_prep = build_l2_over_l1_outer_prep_for_test_pearl(
            &l1,
            l1_config,
            &pure_query_fri_verifier_params_for_l1(L1_LOG_BLOWUP, L1_LOG_FINAL_POLY_LEN),
            l2_config,
            L2_LOG_BLOWUP,
            L2_LOG_FINAL_POLY_LEN,
        )
        .expect("selected L2 prep over statement-bound L1");
        let profile =
            profile_l2_tip5_verifier_for_test_pearl(&l2_prep, &l1, &statement_digest_public_values)
                .expect("profile selected L2 verifier Tip5 trace");

        assert_eq!(
            profile.mmcs_ids_not_tip5, 0,
            "MMCS private-data op ids should all identify Tip5 rows in the selected L2 circuit"
        );
        assert_eq!(
            profile.tip5_trace_rows, profile.tip5_ops,
            "Tip5 trace rows should match compiled Tip5 operation count"
        );
        assert!(
            profile.tip5_mmcs_ops > 0,
            "selected L2 verifier profile should include MMCS Tip5 work"
        );
        assert!(
            profile.tip5_trace_pow2_height >= profile.tip5_trace_rows,
            "reported Tip5 power-of-two height must cover the trace"
        );
    }

    #[test]
    #[ignore = "selected compact L2 verifier challenger phase profile is opt-in"]
    fn selected_fast_l1_compact_l2_tip5_challenger_phase_profile_for_test_pearl() {
        const L1_LOG_BLOWUP: usize = 3;
        const L1_NUM_QUERIES: usize = 20;
        const L1_CAP_HEIGHT: usize = 4;
        const L1_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_LOG_BLOWUP: usize = 5;
        const L2_NUM_QUERIES: usize = 12;
        const L2_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_MAX_LOG_ARITY: usize = 3;
        const L2_CAP_HEIGHT: usize = 4;

        assert_eq!(L1_LOG_BLOWUP * L1_NUM_QUERIES, 60);
        assert_eq!(L2_LOG_BLOWUP * L2_NUM_QUERIES, 60);

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

        let statement_digest_public_values = statement_digest_public_values_for_l1(&built);
        let l1_config = pure_query_l1_stark_config_with_shape_and_cap(
            L1_LOG_BLOWUP, L1_NUM_QUERIES, L1_CAP_HEIGHT,
        );
        let l1_prep = build_l1_outer_prep_for_test_pearl(&built, l1_config.clone(), DIGEST_ELEMS)
            .expect("selected L1 prep");
        let l1 = prove_l1_outer_with_prep_for_test_pearl(&l1_prep, &built, &proof)
            .expect("selected L1 proof with cached prep")
            .proof;
        assert_eq!(
            l1.public_binding_lanes, DIGEST_ELEMS,
            "selected L1 proof must expose the statement digest"
        );

        let l2_config = pure_query_l1_stark_config_with_fri_shape(
            L2_LOG_BLOWUP, L2_NUM_QUERIES, L2_LOG_FINAL_POLY_LEN, L2_MAX_LOG_ARITY, L2_CAP_HEIGHT,
        );
        let l2_prep =
            build_l2_over_l1_outer_prep_for_test_pearl_with_preprocessed_transcript_profile_mode(
                &l1,
                l1_config,
                &pure_query_fri_verifier_params_for_l1(L1_LOG_BLOWUP, L1_LOG_FINAL_POLY_LEN),
                l2_config,
                L2_LOG_BLOWUP,
                L2_LOG_FINAL_POLY_LEN,
                L2VerifierProfileModeForTestPearl::FullNativeWithChallengerPhaseTags,
            )
            .expect("selected phase-tagged L2 prep over statement-bound L1");
        let phase_profile =
            profile_l2_tip5_verifier_for_test_pearl(&l2_prep, &l1, &statement_digest_public_values)
                .expect("profile selected L2 verifier challenger phases");

        eprintln!(
            "selected fast-L1 compact L2 Tip5 challenger/MMCS phase summary [TEST_PEARL]: tagged_tip5_ops={} tip5_ops={} untagged_tip5_ops={} tip5_mmcs_ops={} tip5_non_mmcs_ops={} phase_counts={:?}",
            phase_profile.tagged_tip5_ops,
            phase_profile.tip5_ops,
            phase_profile
                .tip5_ops
                .saturating_sub(phase_profile.tagged_tip5_ops),
            phase_profile.tip5_mmcs_ops,
            phase_profile.tip5_non_mmcs_ops,
            phase_profile.tag_categories,
        );

        assert_eq!(
            phase_profile.mmcs_ids_not_tip5, 0,
            "MMCS private-data op ids should all identify Tip5 rows"
        );
        assert_eq!(
            phase_profile.tip5_mmcs_ops + phase_profile.tip5_non_mmcs_ops,
            phase_profile.tip5_ops,
            "MMCS plus non-MMCS Tip5 counts should cover every Tip5 row"
        );
        assert!(
            phase_profile.tagged_tip5_ops > 0,
            "phase tags should account for challenger Tip5 rows"
        );
        assert!(
            phase_profile
                .tag_categories
                .contains_key("npo:mmcs_leaf_base_coeffs")
                || phase_profile
                    .tag_categories
                    .contains_key("npo:mmcs_leaf_extension_elements"),
            "MMCS leaf hashing should be present in the Tip5 phase profile"
        );
        assert!(
            phase_profile
                .tag_categories
                .contains_key("npo:mmcs_path_sibling"),
            "MMCS path hashing should be present in the Tip5 phase profile"
        );
        assert!(
            phase_profile
                .tag_categories
                .contains_key("challenger:batch_pcs_verify"),
            "FRI/PCS verification should be present in the challenger phase profile"
        );
    }

    #[test]
    #[ignore = "selected compact L2 verifier L1-query-shape profile is opt-in"]
    fn selected_compact_l2_tip5_l1_lb4_nq15_phase_profile_for_test_pearl() {
        const L1_LOG_BLOWUP: usize = 4;
        const L1_NUM_QUERIES: usize = 15;
        const L1_CAP_HEIGHT: usize = 4;
        const L1_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_LOG_BLOWUP: usize = 5;
        const L2_NUM_QUERIES: usize = 12;
        const L2_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_MAX_LOG_ARITY: usize = 3;
        const L2_CAP_HEIGHT: usize = 4;

        assert_eq!(L1_LOG_BLOWUP * L1_NUM_QUERIES, 60);
        assert_eq!(L2_LOG_BLOWUP * L2_NUM_QUERIES, 60);

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

        let statement_digest_public_values = statement_digest_public_values_for_l1(&built);
        let l1_config = pure_query_l1_stark_config_with_shape_and_cap(
            L1_LOG_BLOWUP, L1_NUM_QUERIES, L1_CAP_HEIGHT,
        );
        let l1_prep_wall_start = std::time::Instant::now();
        let l1_prep = build_l1_outer_prep_for_test_pearl(&built, l1_config.clone(), DIGEST_ELEMS)
            .expect("lb4/nq15 L1 prep");
        let l1_prep_wall_ms = l1_prep_wall_start.elapsed().as_millis();
        let l1_cached_prove_start = std::time::Instant::now();
        let l1_artifact = prove_l1_outer_with_prep_for_test_pearl(&l1_prep, &built, &proof)
            .expect("lb4/nq15 L1 proof with cached prep");
        let l1_cached_prove_ms = l1_cached_prove_start.elapsed().as_millis();
        let l1 = l1_artifact.proof;
        assert_eq!(
            l1.public_binding_lanes, DIGEST_ELEMS,
            "L1 proof must expose the statement digest"
        );

        let l2_config = pure_query_l1_stark_config_with_fri_shape(
            L2_LOG_BLOWUP, L2_NUM_QUERIES, L2_LOG_FINAL_POLY_LEN, L2_MAX_LOG_ARITY, L2_CAP_HEIGHT,
        );
        let l2_prep_wall_start = std::time::Instant::now();
        let l2_prep =
            build_l2_over_l1_outer_prep_for_test_pearl_with_preprocessed_transcript_profile_mode(
                &l1,
                l1_config,
                &pure_query_fri_verifier_params_for_l1(L1_LOG_BLOWUP, L1_LOG_FINAL_POLY_LEN),
                l2_config,
                L2_LOG_BLOWUP,
                L2_LOG_FINAL_POLY_LEN,
                L2VerifierProfileModeForTestPearl::FullNativeWithChallengerPhaseTags,
            )
            .expect("lb4/nq15 phase-tagged L2 prep over statement-bound L1");
        let l2_prep_wall_ms = l2_prep_wall_start.elapsed().as_millis();
        let phase_profile =
            profile_l2_tip5_verifier_for_test_pearl(&l2_prep, &l1, &statement_digest_public_values)
                .expect("profile lb4/nq15 L1 into selected L2 verifier");

        eprintln!(
            "selected compact L2 Tip5 L1-query-shape profile [TEST_PEARL L1 lb4_nq15_cap4 -> L2 lb5_nq12_cap4]: tip5_ops={} tip5_pow2_height={} rows_over_previous_pow2={} rows_to_current_pow2={} tip5_mmcs_ops={} tip5_non_mmcs_ops={} tagged_tip5_ops={} phase_counts={:?} l1_prep_wall_ms={} l1_cached_prove_ms={} l1_air_setup_ms={} l1_witness_run_ms={} l1_stark_prove_ms={} l2_prep_wall_ms={} l2_circuit_define_ms={} l2_circuit_build_ms={} l2_air_setup_ms={} l2_witness_run_ms={}",
            phase_profile.tip5_ops,
            phase_profile.tip5_trace_pow2_height,
            phase_profile.tip5_rows_over_previous_pow2,
            phase_profile.tip5_rows_to_current_pow2,
            phase_profile.tip5_mmcs_ops,
            phase_profile.tip5_non_mmcs_ops,
            phase_profile.tagged_tip5_ops,
            phase_profile.tag_categories,
            l1_prep_wall_ms,
            l1_cached_prove_ms,
            l1_prep.timings.air_setup_ms,
            l1_artifact.timings.witness_run_ms,
            l1_artifact.timings.stark_prove_ms,
            l2_prep_wall_ms,
            l2_prep.timings.circuit_define_ms,
            l2_prep.timings.circuit_build_ms,
            l2_prep.timings.air_setup_ms,
            phase_profile.witness_run_ms,
        );

        assert_eq!(
            phase_profile.mmcs_ids_not_tip5, 0,
            "MMCS private-data op ids should all identify Tip5 rows"
        );
        assert_eq!(
            phase_profile.tip5_trace_rows, phase_profile.tip5_ops,
            "Tip5 trace rows should match compiled Tip5 operation count"
        );
    }

    #[test]
    #[ignore = "profile-only lower bound for verifier-key digest transcript work is opt-in"]
    fn selected_fast_l1_compact_l2_preprocessed_transcript_lower_bound_for_test_pearl() {
        const L1_LOG_BLOWUP: usize = 3;
        const L1_NUM_QUERIES: usize = 20;
        const L1_CAP_HEIGHT: usize = 4;
        const L1_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_LOG_BLOWUP: usize = 5;
        const L2_NUM_QUERIES: usize = 12;
        const L2_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_MAX_LOG_ARITY: usize = 3;
        const L2_CAP_HEIGHT: usize = 4;

        assert_eq!(L1_LOG_BLOWUP * L1_NUM_QUERIES, 60);
        assert_eq!(L2_LOG_BLOWUP * L2_NUM_QUERIES, 60);

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

        let l1_config = pure_query_l1_stark_config_with_shape_and_cap(
            L1_LOG_BLOWUP, L1_NUM_QUERIES, L1_CAP_HEIGHT,
        );
        let l1_prep = build_l1_outer_prep_for_test_pearl(&built, l1_config.clone(), DIGEST_ELEMS)
            .expect("selected L1 prep");
        let l1 = prove_l1_outer_with_prep_for_test_pearl(&l1_prep, &built, &proof)
            .expect("selected L1 proof with cached prep")
            .proof;
        assert_eq!(
            l1.public_binding_lanes, DIGEST_ELEMS,
            "selected L1 proof must expose the statement digest"
        );

        let l2_config = pure_query_l1_stark_config_with_fri_shape(
            L2_LOG_BLOWUP, L2_NUM_QUERIES, L2_LOG_FINAL_POLY_LEN, L2_MAX_LOG_ARITY, L2_CAP_HEIGHT,
        );
        let l2_fri_params =
            pure_query_fri_verifier_params_for_l1(L1_LOG_BLOWUP, L1_LOG_FINAL_POLY_LEN);
        let full_prep = build_l2_over_l1_outer_prep_for_test_pearl(
            &l1,
            l1_config.clone(),
            &l2_fri_params,
            l2_config.clone(),
            L2_LOG_BLOWUP,
            L2_LOG_FINAL_POLY_LEN,
        )
        .expect("selected full-transcript L2 prep over statement-bound L1");
        let skip_preprocessed_prep =
            build_l2_over_l1_outer_prep_for_test_pearl_with_preprocessed_transcript_profile_mode(
                &l1,
                l1_config,
                &l2_fri_params,
                l2_config,
                L2_LOG_BLOWUP,
                L2_LOG_FINAL_POLY_LEN,
                L2VerifierProfileModeForTestPearl::SkipPreprocessedTranscript,
            )
            .expect("selected skip-preprocessed-transcript L2 prep over statement-bound L1");

        let full_profile = profile_l2_tip5_circuit_ops_for_test_pearl(&full_prep);
        let skip_profile = profile_l2_tip5_circuit_ops_for_test_pearl(&skip_preprocessed_prep);
        let saved_tip5 = full_profile.tip5_ops.saturating_sub(skip_profile.tip5_ops);
        let saved_non_mmcs = full_profile
            .tip5_non_mmcs_ops
            .saturating_sub(skip_profile.tip5_non_mmcs_ops);
        let full_pow2 = full_profile.tip5_ops.next_power_of_two();
        let skip_pow2 = skip_profile.tip5_ops.next_power_of_two();
        let full_over_previous = full_profile.tip5_ops.saturating_sub(full_pow2 / 2);
        let skip_over_previous = skip_profile.tip5_ops.saturating_sub(skip_pow2 / 2);

        eprintln!(
            "selected fast-L1 compact L2 preprocessed-transcript lower-bound profile [TEST_PEARL]: full_tip5_ops={} skip_preprocessed_tip5_ops={} saved_tip5={} full_tip5_mmcs_ops={} skip_preprocessed_tip5_mmcs_ops={} full_tip5_non_mmcs_ops={} skip_preprocessed_tip5_non_mmcs_ops={} saved_non_mmcs={} full_mmcs_ids_not_tip5={} skip_preprocessed_mmcs_ids_not_tip5={} full_tip5_pow2_height={} skip_preprocessed_tip5_pow2_height={} full_rows_over_previous_pow2={} skip_preprocessed_rows_over_previous_pow2={} crosses_halving_boundary={} full_circuit_ops={} skip_preprocessed_circuit_ops={} full_non_primitive_by_type={:?} skip_preprocessed_non_primitive_by_type={:?}",
            full_profile.tip5_ops,
            skip_profile.tip5_ops,
            saved_tip5,
            full_profile.tip5_mmcs_ops,
            skip_profile.tip5_mmcs_ops,
            full_profile.tip5_non_mmcs_ops,
            skip_profile.tip5_non_mmcs_ops,
            saved_non_mmcs,
            full_profile.mmcs_ids_not_tip5,
            skip_profile.mmcs_ids_not_tip5,
            full_pow2,
            skip_pow2,
            full_over_previous,
            skip_over_previous,
            skip_pow2 < full_pow2,
            full_profile.circuit_ops,
            skip_profile.circuit_ops,
            full_profile.non_primitive_by_type,
            skip_profile.non_primitive_by_type,
        );

        assert_eq!(
            full_profile.mmcs_ids_not_tip5, 0,
            "full transcript MMCS private-data op ids should all identify Tip5 rows"
        );
        assert_eq!(
            skip_profile.mmcs_ids_not_tip5, 0,
            "skip-preprocessed profile MMCS private-data op ids should all identify Tip5 rows"
        );
        assert!(
            skip_profile.tip5_ops < full_profile.tip5_ops,
            "skipping preprocessed transcript observations should reduce challenger Tip5 rows"
        );
        assert!(
            saved_non_mmcs > 0,
            "preprocessed transcript observations should only affect non-MMCS challenger rows"
        );
    }

    #[test]
    #[ignore = "selected compact L2 verifier Tip5 L1-cap-height sweep is opt-in"]
    fn selected_fast_l1_compact_l2_tip5_l1_cap_height_profile_for_test_pearl() {
        use p3_circuit_prover::BatchStarkProver;

        const L1_LOG_BLOWUP: usize = 3;
        const L1_NUM_QUERIES: usize = 20;
        const L1_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_LOG_BLOWUP: usize = 5;
        const L2_NUM_QUERIES: usize = 12;
        const L2_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_MAX_LOG_ARITY: usize = 3;
        const L2_CAP_HEIGHT: usize = 4;

        assert_eq!(L1_LOG_BLOWUP * L1_NUM_QUERIES, 60);
        assert_eq!(L2_LOG_BLOWUP * L2_NUM_QUERIES, 60);

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
        let statement_digest_public_values = statement_digest_public_values_for_l1(&built);

        for l1_cap_height in [3usize, 4, 5, 6] {
            let l1_config = pure_query_l1_stark_config_with_shape_and_cap(
                L1_LOG_BLOWUP, L1_NUM_QUERIES, l1_cap_height,
            );
            let l1_prep_start = std::time::Instant::now();
            let l1_prep =
                build_l1_outer_prep_for_test_pearl(&built, l1_config.clone(), DIGEST_ELEMS)
                    .expect("selected L1 prep for cap sweep");
            let l1_prep_ms = l1_prep_start.elapsed().as_millis();

            let l1_cached_start = std::time::Instant::now();
            let l1 = prove_l1_outer_with_prep_for_test_pearl(&l1_prep, &built, &proof)
                .expect("selected L1 proof with cached prep for cap sweep")
                .proof;
            let l1_cached_ms = l1_cached_start.elapsed().as_millis();
            assert_eq!(
                l1.public_binding_lanes, DIGEST_ELEMS,
                "selected L1 proof must expose the statement digest"
            );

            let l1_verify_start = std::time::Instant::now();
            let mut l1_verifier = BatchStarkProver::new(l1_config.clone())
                .with_table_packing(production_l1_table_packing(DIGEST_ELEMS));
            l1_verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
            l1_verifier.register_recompose_table::<2>(true);
            l1_verifier
                .verify_all_tables_with_public_values(&l1, &statement_digest_public_values)
                .expect("selected L1 proof must verify natively before L2 cap sweep");
            let l1_verify_ms = l1_verify_start.elapsed().as_millis();

            let l2_config = pure_query_l1_stark_config_with_fri_shape(
                L2_LOG_BLOWUP, L2_NUM_QUERIES, L2_LOG_FINAL_POLY_LEN, L2_MAX_LOG_ARITY,
                L2_CAP_HEIGHT,
            );
            let l2_prep_start = std::time::Instant::now();
            let l2_prep = match build_l2_over_l1_outer_prep_for_test_pearl(
                &l1,
                l1_config,
                &pure_query_fri_verifier_params_for_l1(L1_LOG_BLOWUP, L1_LOG_FINAL_POLY_LEN),
                l2_config,
                L2_LOG_BLOWUP,
                L2_LOG_FINAL_POLY_LEN,
            ) {
                Ok(prep) => prep,
                Err(e) => {
                    eprintln!(
                        "selected fast-L1 compact L2 Tip5 L1-cap profile [TEST_PEARL cap={}]: l1_prep_ms={} l1_cached_ms={} l1_verify_ms={} l2_build_error={}",
                        l1_cap_height, l1_prep_ms, l1_cached_ms, l1_verify_ms, e,
                    );
                    assert!(
                        l1_cap_height > 4,
                        "baseline L1 cap heights must build an L2 verifier profile"
                    );
                    continue;
                }
            };
            let l2_prep_ms = l2_prep_start.elapsed().as_millis();

            let profile = match profile_l2_tip5_verifier_for_test_pearl(
                &l2_prep, &l1, &statement_digest_public_values,
            ) {
                Ok(profile) => profile,
                Err(e) => {
                    eprintln!(
                        "selected fast-L1 compact L2 Tip5 L1-cap profile [TEST_PEARL cap={}]: l1_prep_ms={} l1_cached_ms={} l1_verify_ms={} l2_prep_ms={} l2_profile_error={}",
                        l1_cap_height, l1_prep_ms, l1_cached_ms, l1_verify_ms, l2_prep_ms, e,
                    );
                    assert!(
                        l1_cap_height > 4,
                        "baseline L1 cap heights must run an L2 verifier profile"
                    );
                    continue;
                }
            };

            eprintln!(
                "selected fast-L1 compact L2 Tip5 L1-cap profile [TEST_PEARL cap={}]: l1_prep_ms={} l1_cached_ms={} l1_verify_ms={} l2_prep_ms={} tip5_trace_rows={} tip5_trace_pow2_height={} tip5_rows_over_previous_pow2={} tip5_mmcs_ops={} tip5_non_mmcs_ops={} tip5_mmcs_bit_rows={} non_primitive_by_type={:?}",
                l1_cap_height,
                l1_prep_ms,
                l1_cached_ms,
                l1_verify_ms,
                l2_prep_ms,
                profile.tip5_trace_rows,
                profile.tip5_trace_pow2_height,
                profile.tip5_rows_over_previous_pow2,
                profile.tip5_mmcs_ops,
                profile.tip5_non_mmcs_ops,
                profile.tip5_mmcs_bit_rows,
                profile.non_primitive_by_type,
            );

            assert_eq!(
                profile.mmcs_ids_not_tip5, 0,
                "MMCS private-data op ids should all identify Tip5 rows"
            );
            assert_eq!(
                profile.tip5_trace_rows, profile.tip5_ops,
                "Tip5 trace rows should match compiled Tip5 operation count"
            );
        }
    }

    fn init_batch_stark_profile_tracing_for_test_pearl() {
        use tracing_subscriber::fmt::format::FmtSpan;
        use tracing_subscriber::EnvFilter;

        let mut filter = EnvFilter::from_default_env()
            .add_directive(
                "p3_batch_stark=info"
                    .parse()
                    .expect("valid tracing directive"),
            )
            .add_directive(
                "p3_circuit_prover::batch_stark_prover=info"
                    .parse()
                    .expect("valid tracing directive"),
            );
        if let Some(profile) = std::env::var_os("AI_POW_ZK_DEEP_BATCH_PROFILE") {
            let mut directives =
                vec!["p3_fri=info", "p3_fri::two_adic_pcs=debug", "p3_merkle_tree=debug"];
            if matches!(profile.to_str(), Some("full" | "dft")) {
                directives.push("p3_dft=debug");
            }
            for directive in directives {
                filter = filter.add_directive(
                    directive
                        .parse()
                        .expect("valid deep batch profile tracing directive"),
                );
            }
        }
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_span_events(FmtSpan::CLOSE)
            .with_test_writer()
            .try_init();
    }

    #[test]
    #[ignore = "fast-L1 lb4/nq15 L2 size-time frontier sweep is opt-in"]
    fn pure_query_l2_over_fast_l1_lb4_nq15_frontier_for_test_pearl() {
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb3_nq20_cap4",
            3,
            20,
            4,
            2,
            &[
                ("lb4_nq15_lfp0_mla3_cap4", 4, 15, 0, 3, 4),
                ("lb4_nq15_lfp1_mla3_cap4", 4, 15, 1, 3, 4),
                ("lb4_nq15_lfp2_mla2_cap4", 4, 15, 2, 2, 4),
                ("lb4_nq15_lfp2_mla3_cap2", 4, 15, 2, 3, 2),
                ("lb4_nq15_lfp2_mla3_cap4", 4, 15, 2, 3, 4),
                ("lb4_nq15_lfp2_mla3_cap6", 4, 15, 2, 3, 6),
                ("lb4_nq15_lfp2_mla4_cap4", 4, 15, 2, 4, 4),
            ],
        );
    }

    #[test]
    #[ignore = "L1 lb4/nq15 over selected compact L2 timing breakdown is opt-in"]
    fn pure_query_l2_over_l1_lb4_nq15_selected_l2_timing_for_test_pearl() {
        init_batch_stark_profile_tracing_for_test_pearl();
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb4_nq15_cap4",
            4,
            15,
            4,
            2,
            &[("lb5_nq12_lfp2_mla3_cap4", 5, 12, 2, 3, 4)],
        );
    }

    #[test]
    #[ignore = "pure-query AI-PoW L2 cap-height measurement is opt-in"]
    fn pure_query_l2_over_l1_l2_cap_height_compact_body_breakdown_for_test_pearl() {
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb6_nq10_cap4",
            6,
            10,
            4,
            2,
            &[
                ("lb5_nq12_lfp2_mla3_cap2", 5, 12, 2, 3, 2),
                ("lb5_nq12_lfp2_mla3_cap4", 5, 12, 2, 3, 4),
                ("lb5_nq12_lfp2_mla3_cap6", 5, 12, 2, 3, 6),
                ("lb5_nq12_lfp2_mla3_cap8", 5, 12, 2, 3, 8),
            ],
        );
    }

    #[test]
    #[ignore = "pure-query AI-PoW L2 FRI-shape measurement is opt-in"]
    fn pure_query_l2_over_l1_l2_fri_shape_compact_body_breakdown_for_test_pearl() {
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb6_nq10_cap4",
            6,
            10,
            4,
            2,
            &[
                ("lb5_nq12_lfp0_mla3_cap4", 5, 12, 0, 3, 4),
                ("lb5_nq12_lfp1_mla3_cap4", 5, 12, 1, 3, 4),
                ("lb5_nq12_lfp2_mla2_cap4", 5, 12, 2, 2, 4),
                ("lb5_nq12_lfp2_mla3_cap4", 5, 12, 2, 3, 4),
                ("lb5_nq12_lfp2_mla4_cap4", 5, 12, 2, 4, 4),
            ],
        );
    }

    #[test]
    #[ignore = "pure-query AI-PoW L2 actual-vs-frontier Merkle path measurement is opt-in"]
    fn pure_query_l2_over_l1_l2_multiproof_frontier_estimate_for_test_pearl() {
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb6_nq10_cap4",
            6,
            10,
            4,
            2,
            &[("lb5_nq12_lfp2_mla3_cap4", 5, 12, 2, 3, 4)],
        );
    }

    #[test]
    #[ignore = "Pearl-inspired high-blowup pure-query AI-PoW L2 measurement is opt-in"]
    fn pure_query_l2_over_l1_l2_pearl_rate7_final_shape_for_test_pearl() {
        run_pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl(
            "lb6_nq10_cap4",
            6,
            10,
            4,
            2,
            &[("lb7_nq9_lfp2_mla3_cap4", 7, 9, 2, 3, 4)],
        );
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

    fn measure_baseline_composite_terminal_relation(
        profile: CircuitConfig,
    ) -> CompositeTerminalRelationMetrics {
        measure_baseline_composite_terminal_relation_with_recompose_coeff_ctl(profile, true)
    }

    fn measure_baseline_composite_terminal_relation_with_recompose_coeff_ctl(
        profile: CircuitConfig,
        recompose_coeff_ctl_for_decompose_links: bool,
    ) -> CompositeTerminalRelationMetrics {
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program, proof, &pis,
            )
        };
        measure_composite_l1_terminal_relation_with_recompose_coeff_ctl(
            &zk, &profile, &verified, recompose_coeff_ctl_for_decompose_links,
        )
        .expect("terminal relation metrics must build without terminal proving")
    }

    fn measure_baseline_composite_terminal_relation_recompose_ctl_pair(
        profile: CircuitConfig,
    ) -> (
        CompositeTerminalRelationMetrics,
        CompositeTerminalRelationMetrics,
    ) {
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program, proof, &pis,
            )
        };
        let sound = measure_composite_l1_terminal_relation_with_recompose_coeff_ctl(
            &zk, &profile, &verified, true,
        )
        .expect("sound terminal relation metrics must build");
        let unsafe_floor = measure_composite_l1_terminal_relation_with_recompose_coeff_ctl(
            &zk, &profile, &verified, false,
        )
        .expect("unsafe lower-bound terminal relation metrics must build");
        (sound, unsafe_floor)
    }

    fn measure_baseline_l1_verifier_input_footprint(
        profile: CircuitConfig,
    ) -> CompositeL1VerifierInputFootprint {
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program, proof, &pis,
            )
        };
        measure_composite_l1_verifier_input_footprint(&zk, &profile, &verified)
            .expect("L1 verifier input footprint must build without terminal proving")
    }

    fn measure_baseline_terminal_npo_polynomial_layout(
        profile: CircuitConfig,
    ) -> CompositeTerminalNpoPolynomialLayoutMetrics {
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program, proof, &pis,
            )
        };
        measure_composite_l1_terminal_npo_polynomial_layout(&zk, &profile, &verified)
            .expect("terminal NPO polynomial layout metrics must build without terminal proving")
    }

    fn print_l1_verifier_input_footprint(
        label: &str,
        footprint: &CompositeL1VerifierInputFootprint,
    ) {
        eprintln!(
            "ai-pow composite L1 verifier input footprint [{label}]: build_ms={} statement_digest_values={} air_public_values={} proof_public_values={} common_public_values={} verifier_public_values={} total_public_values={} proof_private_values={} total_private_values={} total_public_bytes={} fingerprint={:?}",
            footprint.l1_circuit_build_ms,
            footprint.statement_digest_values,
            footprint.air_public_values,
            footprint.proof_public_values,
            footprint.common_public_values,
            footprint.verifier_public_values,
            footprint.total_public_values,
            footprint.proof_private_values,
            footprint.total_private_values,
            footprint.total_public_input_bytes,
            footprint.circuit_fingerprint,
        );
    }

    fn print_terminal_relation_metrics(label: &str, metrics: &CompositeTerminalRelationMetrics) {
        eprintln!(
            "ai-pow composite terminal relation metrics [{label}]: build_ms={} compile_ms={} public_values={} public_bytes={} private_values={} ops={} primitive_ops={} const_ops={} public_ops={} alu_add_ops={} alu_mul_ops={} alu_bool_ops={} alu_mul_add_ops={} alu_horner_ops={} hints={} npos={} constraints={} tip5_rows={} recompose_rows={} recompose_coeff_rows={} npo_rows={} npo_in_slots={} npo_out_slots={} npo_residuals={} fingerprint={:?} npo_types={:?}",
            metrics.l1_circuit_build_ms,
            metrics.terminal_compile_ms,
            metrics.terminal_public_input_values,
            metrics.terminal_public_input_bytes,
            metrics.terminal_private_input_values,
            metrics.terminal_operation_count,
            metrics.primitive_operation_count,
            metrics.const_operation_count,
            metrics.public_operation_count,
            metrics.alu_add_operation_count,
            metrics.alu_mul_operation_count,
            metrics.alu_bool_check_operation_count,
            metrics.alu_mul_add_operation_count,
            metrics.alu_horner_acc_operation_count,
            metrics.hint_operation_count,
            metrics.non_primitive_operation_count,
            metrics.terminal_constraint_count,
            metrics.tip5_rows,
            metrics.recompose_rows,
            metrics.recompose_coeff_rows,
            metrics.npo_rows,
            metrics.npo_callsite_input_slots,
            metrics.npo_callsite_output_slots,
            metrics.external_npo_validity_components,
            metrics.circuit_fingerprint,
            metrics.non_primitive_types,
        );
    }

    fn print_terminal_npo_polynomial_layout_metrics(
        label: &str,
        metrics: &CompositeTerminalNpoPolynomialLayoutMetrics,
    ) {
        let npo = &metrics.npo_polynomial;
        let packed = &metrics.packed_tip5_lookup;
        let current_lookup_rows = (packed.lookup_table_rows
            + packed.tip5_rows * packed.tip5_rounds)
            .max(1)
            .next_power_of_two();
        let current_lookup_quotient_rows = current_lookup_rows * packed.max_constraint_degree;
        eprintln!(
            "ai-pow composite terminal NPO polynomial layout [{label}]: build_ms={} compile_ms={} public_values={} private_values={} fingerprint={:?} npo_rows={} residual_components={} sampled_residual_components={} tip5_rows={} tip5_merkle_rows={} tip5_new_start_rows={} recompose_rows={} recompose_coeff_rows={} witness_inputs={} witness_outputs={} hidden_inputs={} max_serialized_hidden_inputs={} mmcs_bits={} column_count={} metadata_columns={} input_value_columns={} output_value_columns={} hidden_value_columns={} residual_value_columns={} witness_value_field_columns={} residual_value_field_columns={} prover_dependent_field_columns={} full_table_basis_columns={} witness_value_basis_columns={} prover_dependent_basis_columns={} rows={} padded_rows={} composition_basis_columns={} recompose_quotient_basis_columns={} terminal_challenge_basis_dim={} residual_zero_opened_basis_columns={} residual_zero_min_opened_limb_bytes={} recompose_opened_basis_columns={} recompose_min_opened_limb_bytes={} packed_tip5_rows={} packed_tip5_padded_rows={} packed_tip5_width={} packed_tip5_round_width={} packed_tip5_logup_tuples={} packed_tip5_quotient_rows={} current_lookup_rows={} current_lookup_width={} current_lookup_quotient_rows={}",
            metrics.l1_circuit_build_ms,
            metrics.terminal_compile_ms,
            metrics.terminal_public_input_values,
            metrics.terminal_private_input_values,
            metrics.circuit_fingerprint,
            npo.relation_profile.rows,
            npo.relation_profile.residual_components,
            npo.relation_profile.sampled_residual_components,
            npo.relation_profile.tip5_rows,
            npo.relation_profile.tip5_merkle_rows,
            npo.relation_profile.tip5_new_start_rows,
            npo.relation_profile.recompose_rows,
            npo.relation_profile.recompose_coeff_rows,
            npo.relation_profile.witness_input_slots,
            npo.relation_profile.witness_output_slots,
            npo.relation_profile.hidden_input_slots,
            npo.relation_profile.max_serialized_hidden_input_slots,
            npo.relation_profile.mmcs_direction_bits,
            npo.column_layout.column_count,
            npo.column_layout.metadata_columns,
            npo.column_layout.input_value_columns,
            npo.column_layout.output_value_columns,
            npo.column_layout.hidden_tip5_value_columns,
            npo.column_layout.residual_value_columns,
            npo.witness_value_field_columns,
            npo.residual_value_field_columns,
            npo.prover_dependent_field_columns,
            npo.full_table_profile.basis_columns,
            npo.witness_value_profile.basis_columns,
            npo.prover_dependent_profile.basis_columns,
            npo.prover_dependent_profile.rows,
            npo.prover_dependent_profile.padded_rows,
            npo.compact_residual_composition_profile.basis_columns,
            npo.residual_relation_quotient_profile.basis_columns,
            npo.terminal_challenge_basis_dimension,
            npo.fri_native_residual_zero_opened_basis_columns,
            npo.fri_native_residual_zero_min_opened_limb_bytes,
            npo.fri_native_recompose_opened_basis_columns,
            npo.fri_native_recompose_min_opened_limb_bytes,
            packed.rows,
            packed.padded_rows,
            packed.main_width,
            packed.round_width,
            packed.logup_query_tuples,
            packed.algebra_quotient_rows,
            current_lookup_rows,
            p3_tip5_circuit_air::tip5_lookup_air_width(),
            current_lookup_quotient_rows,
        );
    }

    fn describe_terminal_npo_residual_component(
        row: &TerminalNpoPolynomialTableRow<Challenge>,
        component_offset: usize,
    ) -> String {
        let basis_dim = <Challenge as BasedVectorSpace<Val>>::DIMENSION;
        let mut offset = component_offset;
        let mut visit = |label: String, width: usize| {
            if offset < width {
                Some(format!("{label}/basis_{offset}"))
            } else {
                offset -= width;
                None
            }
        };

        match row.row_kind {
            TerminalNpoRowKind::Tip5Goldilocks => {
                for (limb, value) in row.inputs.iter().enumerate() {
                    if value.is_some() {
                        if let Some(label) = visit(format!("tip5_input_limb_{limb}"), basis_dim) {
                            return label;
                        }
                    }
                }
                for (limb, value) in row.outputs.iter().enumerate() {
                    if value.is_some() {
                        if let Some(label) = visit(format!("tip5_output_limb_{limb}"), basis_dim) {
                            return label;
                        }
                    }
                }
                if row.mode_merkle_path {
                    if row.mmcs_bit.is_some() {
                        if let Some(label) = visit("tip5_mmcs_bit_limb_16".into(), 1) {
                            return label;
                        }
                    }
                    for limb in 0..5 {
                        if row.inputs.get(limb).is_none_or(Option::is_none) {
                            if let Some(label) =
                                visit(format!("tip5_merkle_chain_input_limb_{limb}"), 1)
                            {
                                return label;
                            }
                        }
                    }
                    for limb in 10..16 {
                        if row.inputs.get(limb).is_none_or(Option::is_none) {
                            if let Some(label) = visit(format!("tip5_merkle_zero_limb_{limb}"), 1) {
                                return label;
                            }
                        }
                    }
                } else {
                    for limb in 0..16 {
                        if row.inputs.get(limb).is_none_or(Option::is_none) {
                            if let Some(label) = visit(format!("tip5_chain_input_limb_{limb}"), 1) {
                                return label;
                            }
                        }
                    }
                }
            }
            TerminalNpoRowKind::Recompose | TerminalNpoRowKind::RecomposeWithCoeffLookups => {
                for (limb, value) in row.inputs.iter().enumerate() {
                    if value.is_some() {
                        if let Some(label) =
                            visit(format!("recompose_input_limb_{limb}"), basis_dim)
                        {
                            return label;
                        }
                    }
                }
                for (limb, value) in row.outputs.iter().enumerate() {
                    if value.is_some() {
                        if let Some(label) =
                            visit(format!("recompose_output_limb_{limb}"), basis_dim)
                        {
                            return label;
                        }
                    }
                }
            }
        }

        format!("unknown_component_{component_offset}")
    }

    fn print_terminal_npo_residual_distribution(
        label: &str,
        table: &TerminalNpoPolynomialTable<Challenge>,
    ) {
        let mut total_nonzero = 0usize;
        let mut rows_with_nonzero = 0usize;
        let mut by_row_kind = std::collections::BTreeMap::<String, usize>::new();
        let mut by_component = std::collections::BTreeMap::<String, usize>::new();
        let mut first_nonzero_rows = Vec::new();

        for row in &table.rows {
            let mut row_has_nonzero = false;
            for (component_offset, value) in row.residual_values.iter().enumerate() {
                if *value == Challenge::ZERO {
                    continue;
                }
                total_nonzero += 1;
                row_has_nonzero = true;
                *by_row_kind
                    .entry(format!("{:?}", row.row_kind))
                    .or_default() += 1;
                let component = describe_terminal_npo_residual_component(row, component_offset);
                *by_component.entry(component.clone()).or_default() += 1;
                if first_nonzero_rows.len() < 8 {
                    let basis =
                        <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(value)
                            .iter()
                            .copied()
                            .collect::<Vec<_>>();
                    first_nonzero_rows.push(format!(
                        "npo={} local={} kind={:?} new_start={} merkle={} offset={} component={} value_basis={:?}",
                        row.npo_index,
                        row.local_row,
                        row.row_kind,
                        row.mode_new_start,
                        row.mode_merkle_path,
                        component_offset,
                        component,
                        basis
                    ));
                }
            }
            rows_with_nonzero += usize::from(row_has_nonzero);
        }

        let mut top_components = by_component.iter().collect::<Vec<_>>();
        top_components.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        top_components.truncate(12);

        eprintln!(
            "native terminal NPO residual distribution [{label}]: rows_with_nonzero={} total_nonzero={} by_row_kind={:?} top_components={:?} first_nonzero_rows={:?}",
            rows_with_nonzero,
            total_nonzero,
            by_row_kind,
            top_components,
            first_nonzero_rows,
        );
    }

    fn init_terminal_prover_profile_tracing() {
        if std::env::var_os("NOCK_TERMINAL_PROFILE_PROVER").is_none() {
            return;
        }
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("p3_recursion::terminal=info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .try_init();
    }

    #[test]
    fn terminal_relation_metrics_for_baseline_composite_are_available() {
        let metrics = measure_baseline_composite_terminal_relation(CircuitConfig::TEST_PEARL);
        print_terminal_relation_metrics("TEST_PEARL", &metrics);

        assert!(metrics.terminal_public_input_values > 0);
        assert!(metrics.terminal_private_input_values > 0);
        assert!(metrics.primitive_operation_count > 0);
        assert!(metrics.tip5_rows > 0);
        assert_eq!(
            metrics.npo_rows,
            metrics.tip5_rows + metrics.recompose_rows + metrics.recompose_coeff_rows
        );
        assert!(metrics.external_npo_validity_components >= metrics.npo_rows);
    }

    #[test]
    #[ignore = "Pearl-shaped L1 verifier input footprint diagnostic is opt-in"]
    fn l1_verifier_input_footprint_for_pure_query_lb6_nq10_composite_is_available() {
        let profile = CircuitConfig {
            log_blowup: 6,
            pow_bits: 0,
            num_queries: 10,
        };
        assert_eq!(profile.johnson_fri_bits(), 60);

        let footprint = measure_baseline_l1_verifier_input_footprint(profile);
        print_l1_verifier_input_footprint("PURE_QUERY_LB6_NQ10", &footprint);

        assert_eq!(footprint.statement_digest_values, DIGEST_ELEMS);
        assert_eq!(
            footprint.air_public_values,
            crate::composite_public::NUM_PUBLIC_VALUES
        );
        assert!(footprint.proof_public_values > footprint.air_public_values);
        assert_eq!(
            footprint.total_public_values,
            footprint.statement_digest_values + footprint.verifier_public_values
        );
        assert_eq!(
            footprint.proof_private_values,
            footprint.total_private_values
        );
    }

    #[test]
    #[ignore = "production-profile terminal relation metrics are opt-in"]
    fn terminal_relation_metrics_for_prod_baseline_composite_are_available() {
        measure_and_assert_terminal_relation_for_profile("PROD", CircuitConfig::PROD);
    }

    #[test]
    #[ignore = "unsound lower-bound relation-size diagnostic is opt-in"]
    fn terminal_relation_metrics_recompose_ctl_lower_bound_for_prod_baseline_composite() {
        let (sound, unsafe_floor) =
            measure_baseline_composite_terminal_relation_recompose_ctl_pair(CircuitConfig::PROD);
        print_terminal_relation_metrics("PROD_RECOMPOSE_CTL_SOUND", &sound);
        print_terminal_relation_metrics("PROD_RECOMPOSE_CTL_DISABLED_UNSOUND_FLOOR", &unsafe_floor);
        eprintln!(
            "ai-pow composite terminal relation recompose ctl delta [PROD]: ops={} primitive_ops={} npo_rows={} recompose_rows={} recompose_coeff_rows={} npo_residuals={}",
            sound
                .terminal_operation_count
                .saturating_sub(unsafe_floor.terminal_operation_count),
            sound
                .primitive_operation_count
                .saturating_sub(unsafe_floor.primitive_operation_count),
            sound.npo_rows.saturating_sub(unsafe_floor.npo_rows),
            unsafe_floor
                .recompose_rows
                .saturating_sub(sound.recompose_rows),
            sound
                .recompose_coeff_rows
                .saturating_sub(unsafe_floor.recompose_coeff_rows),
            sound
                .external_npo_validity_components
                .saturating_sub(unsafe_floor.external_npo_validity_components),
        );

        assert_eq!(
            sound.terminal_public_input_values,
            unsafe_floor.terminal_public_input_values
        );
        assert_eq!(
            sound.tip5_rows, unsafe_floor.tip5_rows,
            "disabling recompose coeff CTL must not change Tip5 rows"
        );
        assert_eq!(
            unsafe_floor.recompose_coeff_rows, 0,
            "lower-bound diagnostic should remove the coeff-control table"
        );
        assert!(
            unsafe_floor.recompose_rows > sound.recompose_rows,
            "disabling coeff control should replace most coeff rows with plain recompose rows"
        );
        assert_eq!(
            unsafe_floor.primitive_operation_count, sound.primitive_operation_count,
            "the coeff-control toggle should not change primitive verifier arithmetic"
        );
        assert!(
            unsafe_floor.terminal_operation_count < sound.terminal_operation_count,
            "without a replacement binding this only measures an unsound total-op floor"
        );
        assert!(
            unsafe_floor.npo_rows < sound.npo_rows,
            "without a replacement binding this only measures an unsound NPO floor"
        );
    }

    #[test]
    #[ignore = "production-profile terminal NPO polynomial layout diagnostic is opt-in"]
    fn terminal_npo_polynomial_layout_for_prod_baseline_composite_is_available() {
        let metrics = measure_baseline_terminal_npo_polynomial_layout(CircuitConfig::PROD);
        print_terminal_npo_polynomial_layout_metrics("PROD", &metrics);

        let npo = &metrics.npo_polynomial;
        assert!(npo.relation_profile.rows > 0);
        assert!(npo.relation_profile.residual_components >= npo.relation_profile.rows);
        assert_eq!(npo.column_layout.rows, npo.relation_profile.rows);
        assert!(
            npo.prover_dependent_field_columns < npo.column_layout.column_count,
            "verifier-derived NPO columns should stay off the prover-dependent FRI matrix"
        );
        assert_eq!(
            npo.prover_dependent_field_columns,
            npo.witness_value_field_columns + npo.residual_value_field_columns
        );
        assert_eq!(
            npo.prover_dependent_profile.field_columns,
            npo.prover_dependent_field_columns
        );
        assert_eq!(
            npo.fri_native_residual_zero_opened_basis_columns,
            npo.prover_dependent_profile.basis_columns
                + npo.compact_residual_composition_profile.basis_columns
        );
        assert!(
            npo.fri_native_residual_zero_min_opened_limb_bytes > 0,
            "layout diagnostic should expose a nonzero opened-limb floor"
        );
        assert_eq!(
            metrics.packed_tip5_lookup.tip5_rows,
            npo.relation_profile.tip5_rows
        );
        assert_eq!(
            metrics.packed_tip5_lookup.algebra_quotient_rows,
            metrics.packed_tip5_lookup.padded_rows
                * metrics.packed_tip5_lookup.max_constraint_degree
        );
        assert!(
            metrics.packed_tip5_lookup.algebra_quotient_rows
                < (metrics.packed_tip5_lookup.lookup_table_rows
                    + metrics.packed_tip5_lookup.tip5_rows
                        * metrics.packed_tip5_lookup.tip5_rounds)
                    .next_power_of_two()
                    * metrics.packed_tip5_lookup.max_constraint_degree,
            "packed one-row-per-permutation trace should reduce the AIR quotient domain"
        );
    }

    #[test]
    #[ignore = "pure-query profile sweep diagnostic is opt-in"]
    fn terminal_relation_metrics_for_pure_query_lb3_nq20_composite_are_available() {
        measure_and_assert_terminal_relation_for_profile(
            "PURE_QUERY_LB3_NQ20",
            CircuitConfig {
                log_blowup: 3,
                pow_bits: 0,
                num_queries: 20,
            },
        );
    }

    #[test]
    #[ignore = "pure-query profile sweep diagnostic is opt-in"]
    fn terminal_relation_metrics_for_pure_query_lb5_nq12_composite_are_available() {
        measure_and_assert_terminal_relation_for_profile(
            "PURE_QUERY_LB5_NQ12",
            CircuitConfig {
                log_blowup: 5,
                pow_bits: 0,
                num_queries: 12,
            },
        );
    }

    #[test]
    #[ignore = "pure-query profile sweep diagnostic is opt-in"]
    fn terminal_relation_metrics_for_pure_query_lb6_nq10_composite_are_available() {
        measure_and_assert_terminal_relation_for_profile(
            "PURE_QUERY_LB6_NQ10",
            CircuitConfig {
                log_blowup: 6,
                pow_bits: 0,
                num_queries: 10,
            },
        );
    }

    fn measure_and_assert_terminal_relation_for_profile(label: &str, profile: CircuitConfig) {
        assert_eq!(profile.johnson_fri_bits(), 60);

        let metrics = measure_baseline_composite_terminal_relation(profile);
        print_terminal_relation_metrics(label, &metrics);

        assert!(metrics.terminal_public_input_values > 0);
        assert!(metrics.terminal_private_input_values > 0);
        assert!(metrics.primitive_operation_count > 0);
        assert!(metrics.tip5_rows > 0);
    }

    fn terminal_vk_for_verified_profile(
        zk: &ZkParams,
        profile: &CircuitConfig,
        verified: &ChainVerifiedCompositeProof<'_>,
    ) -> NativeTerminalVerifyingKey<crate::circuit::Challenge> {
        let cfg = build_config(zk, profile);
        let air = CompositeFullAirWithLookupsPinned::new_with(verified.program.clone(), true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            profile,
        )
        .expect("terminal header diagnostic must build L1 verifier circuit");
        let compiler = terminal_compiler();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("terminal header diagnostic must compile");
        vk
    }

    fn log_first_terminal_constraint_mismatch(
        first: &[NativeTerminalConstraint<crate::circuit::Challenge>],
        second: &[NativeTerminalConstraint<crate::circuit::Challenge>],
    ) {
        if first.len() != second.len() {
            eprintln!(
                "ai-pow terminal header rebuild: constraint length mismatch first={} second={}",
                first.len(),
                second.len()
            );
        }
        for (idx, (left, right)) in first.iter().zip(second).enumerate() {
            if left != right {
                eprintln!(
                    "ai-pow terminal header rebuild: first constraint mismatch index={idx} left={left:?} right={right:?}"
                );
                break;
            }
        }
    }

    fn assert_terminal_header_rebuilds_deterministically(label: &str, profile: CircuitConfig) {
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);

        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program, proof, &pis,
            )
        };

        let first_vk = terminal_vk_for_verified_profile(&zk, &profile, &verified);
        let second_vk = terminal_vk_for_verified_profile(&zk, &profile, &verified);
        let first = first_vk.header.clone();
        let second = second_vk.header.clone();
        eprintln!(
            "ai-pow terminal header rebuild [{label}]: first={:?} second={:?}",
            first, second
        );
        if first != second {
            log_first_terminal_constraint_mismatch(&first_vk.constraints, &second_vk.constraints);
        }
        assert_eq!(
            first, second,
            "terminal certificate header must rebuild deterministically"
        );
    }

    #[test]
    fn terminal_header_rebuilds_deterministically_for_baseline_composite() {
        assert_terminal_header_rebuilds_deterministically("TEST_PEARL", CircuitConfig::TEST_PEARL);
    }

    #[test]
    #[ignore = "pure-query terminal header rebuild diagnostic is opt-in"]
    fn terminal_header_rebuilds_deterministically_for_pure_query_lb6_nq10() {
        assert_terminal_header_rebuilds_deterministically(
            "PURE_QUERY_LB6_NQ10",
            CircuitConfig {
                log_blowup: 6,
                pow_bits: 0,
                num_queries: 10,
            },
        );
    }

    fn prove_test_terminal_certificate_with_profile(
        profile: CircuitConfig,
    ) -> (
        ZkParams,
        CircuitConfig,
        CompositePublicInputs,
        BatchProof<AiPowStarkConfig>,
        crate::AiPowProgram,
        TerminalCertificateRun,
    ) {
        let zk = test_zk_params();
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

    fn prove_test_terminal_certificate() -> (
        ZkParams,
        CircuitConfig,
        CompositePublicInputs,
        BatchProof<AiPowStarkConfig>,
        crate::AiPowProgram,
        TerminalCertificateRun,
    ) {
        prove_test_terminal_certificate_with_profile(CircuitConfig::TEST_PEARL)
    }

    fn measure_and_verify_terminal_certificate_for_profile(label: &str, profile: CircuitConfig) {
        init_terminal_prover_profile_tracing();
        let (zk, profile, pis, proof, program, run) =
            prove_test_terminal_certificate_with_profile(profile);

        let certificate_bytes = postcard::to_allocvec(run.terminal_cert.terminal_certificate())
            .expect("terminal certificate postcard serialization")
            .len();
        let public_input_bytes = postcard::to_allocvec(run.terminal_cert.terminal_public_inputs())
            .expect("terminal public inputs postcard serialization")
            .len();
        let wire_bytes = encode_terminal_recursive_certificate(&run.terminal_cert)
            .expect("postcard terminal recursive certificate encoding");
        eprintln!(
            "native terminal recursive certificate over ai-pow composite verifier [{label}]: certificate={} bytes public_inputs={} bytes wire={} bytes build_ms={} l1_verify_ms={} compile_ms={} prove_ms={} verify_ms={}",
            certificate_bytes,
            public_input_bytes,
            wire_bytes.len(),
            run.l1_circuit_build_ms,
            run.l1_in_circuit_verify_ms,
            run.terminal_compile_ms,
            run.terminal_prove_ms,
            run.terminal_verify_ms,
        );
        log_terminal_production_component_sizes(label, run.terminal_cert.terminal_certificate());

        let decoded = decode_terminal_recursive_certificate(&wire_bytes)
            .expect("decode terminal recursive certificate");
        assert_eq!(
            decoded, run.terminal_cert,
            "terminal recursive certificate postcard encoding must round trip structurally"
        );
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

    fn postcard_len<T: serde::Serialize>(value: &T, label: &str) -> usize {
        postcard::to_allocvec(value)
            .unwrap_or_else(|err| panic!("{label} must serialize for size accounting: {err:?}"))
            .len()
    }

    #[derive(Clone, Debug)]
    struct TerminalCompactFriInputBatchByteBreakdown {
        index: usize,
        total_bytes: usize,
        opened_values_bytes: usize,
        merkle_bytes: usize,
        opened_field_values: usize,
        opened_unique_leaves: usize,
        original_order_entries: usize,
        pruned_paths: usize,
        pruned_siblings: usize,
    }

    #[derive(Clone, Debug)]
    struct TerminalCompactFriByteBreakdown {
        total_bytes: usize,
        commit_phase_commits_bytes: usize,
        commit_pow_witnesses_bytes: usize,
        input_batches_bytes: usize,
        input_opened_values_bytes: usize,
        input_merkle_bytes: usize,
        commit_rounds_bytes: usize,
        commit_round_sibling_values_bytes: usize,
        commit_round_merkle_bytes: usize,
        final_poly_bytes: usize,
        query_pow_witness_bytes: usize,
        input_batches: usize,
        commit_rounds: usize,
        query_count: usize,
        input_batch_breakdowns: Vec<TerminalCompactFriInputBatchByteBreakdown>,
    }

    fn terminal_compact_fri_opened_values_field_count(opened_values: &[Vec<Vec<Val>>]) -> usize {
        opened_values
            .iter()
            .flat_map(|matrix| matrix.iter())
            .map(|point_values| point_values.len())
            .sum()
    }

    fn terminal_compact_fri_input_batch_byte_breakdown(
        index: usize,
        batch: &TerminalCompressedFriInputBatch,
    ) -> TerminalCompactFriInputBatchByteBreakdown {
        let total_bytes = postcard_len(batch, "terminal compact FRI input batch");
        let opened_values_bytes = postcard_len(
            &batch.opened_values, "terminal compact FRI input batch values",
        );
        let merkle_bytes = postcard_len(
            &batch.pruned_opening_proof, "terminal compact FRI input batch Merkle paths",
        );
        let pruned_siblings = batch
            .pruned_opening_proof
            .paths
            .iter()
            .map(|path| path.siblings.len())
            .sum();

        TerminalCompactFriInputBatchByteBreakdown {
            index,
            total_bytes,
            opened_values_bytes,
            merkle_bytes,
            opened_field_values: terminal_compact_fri_opened_values_field_count(
                &batch.opened_values,
            ),
            opened_unique_leaves: batch.opened_values.len(),
            original_order_entries: batch.pruned_opening_proof.original_order.len(),
            pruned_paths: batch.pruned_opening_proof.paths.len(),
            pruned_siblings,
        }
    }

    fn terminal_compact_fri_byte_breakdown(
        proof: &TerminalCompressedFriProof,
        label: &str,
    ) -> TerminalCompactFriByteBreakdown {
        let total_bytes = postcard_len(proof, label);
        let commit_phase_commits_bytes = postcard_len(
            &proof.commit_phase_commits, "terminal compact FRI commit-phase commitments",
        );
        let commit_pow_witnesses_bytes = postcard_len(
            &proof.commit_pow_witnesses, "terminal compact FRI commit POW witnesses",
        );
        let input_batches_bytes =
            postcard_len(&proof.input_batches, "terminal compact FRI input batches");
        let input_batch_breakdowns = proof
            .input_batches
            .iter()
            .enumerate()
            .map(|(index, batch)| terminal_compact_fri_input_batch_byte_breakdown(index, batch))
            .collect::<Vec<_>>();
        let input_opened_values_bytes = proof
            .input_batches
            .iter()
            .map(|batch| {
                postcard_len(
                    &batch.opened_values, "terminal compact FRI input opened values",
                )
            })
            .sum();
        let input_merkle_bytes = proof
            .input_batches
            .iter()
            .map(|batch| {
                postcard_len(
                    &batch.pruned_opening_proof, "terminal compact FRI input Merkle paths",
                )
            })
            .sum();
        let commit_rounds_bytes =
            postcard_len(&proof.commit_rounds, "terminal compact FRI commit rounds");
        let commit_round_sibling_values_bytes = proof
            .commit_rounds
            .iter()
            .map(|round| {
                postcard_len(
                    &round.sibling_values, "terminal compact FRI commit-round sibling values",
                )
            })
            .sum();
        let commit_round_merkle_bytes = proof
            .commit_rounds
            .iter()
            .map(|round| {
                postcard_len(
                    &round.pruned_opening_proof, "terminal compact FRI commit-round Merkle paths",
                )
            })
            .sum();
        let final_poly_bytes =
            postcard_len(&proof.final_poly, "terminal compact FRI final polynomial");
        let query_pow_witness_bytes = postcard_len(
            &proof.query_pow_witness, "terminal compact FRI query POW witness",
        );
        let query_count = proof
            .input_batches
            .first()
            .map(|batch| batch.pruned_opening_proof.original_order.len())
            .unwrap_or(0);

        TerminalCompactFriByteBreakdown {
            total_bytes,
            commit_phase_commits_bytes,
            commit_pow_witnesses_bytes,
            input_batches_bytes,
            input_opened_values_bytes,
            input_merkle_bytes,
            commit_rounds_bytes,
            commit_round_sibling_values_bytes,
            commit_round_merkle_bytes,
            final_poly_bytes,
            query_pow_witness_bytes,
            input_batches: proof.input_batches.len(),
            commit_rounds: proof.commit_rounds.len(),
            query_count,
            input_batch_breakdowns,
        }
    }

    fn log_terminal_compact_fri_byte_breakdown(
        label: &str,
        breakdown: &TerminalCompactFriByteBreakdown,
    ) {
        eprintln!(
            "native terminal compact FRI components [{label}]: total={} commit_phase_commits={} commit_pow_witnesses={} input_batches={} input_opened_values={} input_merkle={} commit_rounds={} commit_round_sibling_values={} commit_round_merkle={} final_poly={} query_pow_witness={} input_batch_count={} commit_round_count={} query_count={}",
            breakdown.total_bytes,
            breakdown.commit_phase_commits_bytes,
            breakdown.commit_pow_witnesses_bytes,
            breakdown.input_batches_bytes,
            breakdown.input_opened_values_bytes,
            breakdown.input_merkle_bytes,
            breakdown.commit_rounds_bytes,
            breakdown.commit_round_sibling_values_bytes,
            breakdown.commit_round_merkle_bytes,
            breakdown.final_poly_bytes,
            breakdown.query_pow_witness_bytes,
            breakdown.input_batches,
            breakdown.commit_rounds,
            breakdown.query_count,
        );
        for batch in &breakdown.input_batch_breakdowns {
            eprintln!(
                "native terminal compact FRI input batch [{label}#{}]: total={} opened_values={} merkle={} opened_field_values={} opened_unique_leaves={} original_order_entries={} pruned_paths={} pruned_siblings={}",
                batch.index,
                batch.total_bytes,
                batch.opened_values_bytes,
                batch.merkle_bytes,
                batch.opened_field_values,
                batch.opened_unique_leaves,
                batch.original_order_entries,
                batch.pruned_paths,
                batch.pruned_siblings,
            );
        }
    }

    #[derive(Clone, Debug)]
    struct TerminalPackedTip5PairedLookupPayloadEstimate {
        current_trace_width: usize,
        paired_trace_width_floor: usize,
        current_split_columns: usize,
        paired_split_columns: usize,
        current_query_interactions: usize,
        paired_query_interactions: usize,
        current_logup_interactions: usize,
        paired_logup_interactions: usize,
        logup_group_size: usize,
        current_logup_groups: usize,
        paired_logup_groups: usize,
        current_logup_accumulator_basis: usize,
        paired_logup_accumulator_basis: usize,
        current_trace_table_opened_basis: usize,
        paired_trace_table_opened_basis: usize,
        current_accumulator_opened_basis_per_point: usize,
        paired_accumulator_opened_basis_per_point: usize,
        trace_table_opened_values_bytes: usize,
        trace_table_estimated_opened_values_bytes: usize,
        accumulator_opened_values_bytes: usize,
        accumulator_estimated_opened_values_bytes: usize,
        fri_opened_value_savings_bytes: usize,
        packed_trace_zeta_opening_bytes: usize,
        packed_trace_zeta_estimated_opening_bytes: usize,
        logup_accumulator_zeta_opening_bytes: usize,
        logup_accumulator_zeta_estimated_opening_bytes: usize,
        non_fri_opening_savings_bytes: usize,
        current_packed_support_fri_bytes: usize,
        estimated_packed_support_fri_after_opened_value_savings_bytes: usize,
        current_optimistic_single_fri_floor_bytes: usize,
        estimated_optimistic_single_fri_floor_after_savings_bytes: usize,
    }

    fn terminal_scaled_byte_count(bytes: usize, current_units: usize, new_units: usize) -> usize {
        assert!(current_units > 0, "current_units must be nonzero");
        ((bytes as u128 * new_units as u128 + current_units as u128 - 1) / current_units as u128)
            as usize
    }

    fn terminal_div_ceil(numerator: usize, denominator: usize) -> usize {
        assert!(denominator > 0, "denominator must be nonzero");
        numerator.div_ceil(denominator)
    }

    fn terminal_packed_tip5_paired_lookup_payload_estimate(
        proof: &TerminalNpoTip5PackedLookupAirLogupSelectedTraceBridgeProof,
        fri_breakdown: &TerminalCompactFriByteBreakdown,
        current_optimistic_single_fri_floor_bytes: usize,
    ) -> TerminalPackedTip5PairedLookupPayloadEstimate {
        let trace_table_batch = fri_breakdown
            .input_batch_breakdowns
            .get(1)
            .expect("packed support FRI batch 1 must be packed trace/table");
        let accumulator_batch = fri_breakdown
            .input_batch_breakdowns
            .get(2)
            .expect("packed support FRI batch 2 must be accumulators");

        let split_lanes = 4usize;
        let split_bytes_per_lane = 8usize;
        let paired_bytes_per_lane = split_bytes_per_lane / 2;
        let split_bc_columns_per_byte = 2usize;
        let current_split_columns = proof
            .packed_trace_profile
            .tip5_rounds
            .checked_mul(split_lanes)
            .and_then(|value| value.checked_mul(split_bytes_per_lane))
            .and_then(|value| value.checked_mul(split_bc_columns_per_byte))
            .expect("current split column count must fit usize");
        let paired_split_columns = proof
            .packed_trace_profile
            .tip5_rounds
            .checked_mul(split_lanes)
            .and_then(|value| value.checked_mul(paired_bytes_per_lane))
            .and_then(|value| value.checked_mul(split_bc_columns_per_byte))
            .expect("paired split column count must fit usize");
        let current_trace_width = proof.packed_trace_profile.main_width;
        let paired_trace_width_floor = current_trace_width
            .checked_sub(current_split_columns)
            .and_then(|value| value.checked_add(paired_split_columns))
            .expect("paired trace width must fit usize");

        let current_query_interactions = proof
            .packed_trace_profile
            .tip5_rounds
            .checked_mul(split_lanes)
            .and_then(|value| value.checked_mul(split_bytes_per_lane))
            .expect("current query interaction count must fit usize");
        let paired_query_interactions = proof
            .packed_trace_profile
            .tip5_rounds
            .checked_mul(split_lanes)
            .and_then(|value| value.checked_mul(paired_bytes_per_lane))
            .expect("paired query interaction count must fit usize");
        let current_logup_interactions = current_query_interactions + 1;
        let paired_logup_interactions = paired_query_interactions + 1;
        let logup_group_size = 7usize;
        let current_logup_groups = terminal_div_ceil(current_logup_interactions, logup_group_size);
        assert_eq!(
            current_logup_groups, proof.logup_accumulator_profile.field_columns,
            "paired lookup estimate must match the measured packed LogUp group shape"
        );
        let paired_logup_groups = terminal_div_ceil(paired_logup_interactions, logup_group_size);
        let paired_logup_accumulator_basis =
            paired_logup_groups * proof.logup_accumulator_profile.field_basis_dimension;

        let current_trace_table_opened_basis =
            current_trace_width + proof.logup_table_profile.basis_columns;
        let paired_trace_table_opened_basis =
            paired_trace_width_floor + proof.logup_table_profile.basis_columns;
        let trace_table_estimated_opened_values_bytes = terminal_scaled_byte_count(
            trace_table_batch.opened_values_bytes, current_trace_table_opened_basis,
            paired_trace_table_opened_basis,
        );

        let current_accumulator_opened_basis_per_point =
            proof.logup_accumulator_profile.basis_columns
                + proof.selected_bridge_accumulator_profile.basis_columns
                + proof.packed_bridge_accumulator_profile.basis_columns;
        let paired_accumulator_opened_basis_per_point = paired_logup_accumulator_basis
            + proof.selected_bridge_accumulator_profile.basis_columns
            + proof.packed_bridge_accumulator_profile.basis_columns;
        let accumulator_estimated_opened_values_bytes = terminal_scaled_byte_count(
            accumulator_batch.opened_values_bytes, current_accumulator_opened_basis_per_point,
            paired_accumulator_opened_basis_per_point,
        );

        let fri_opened_value_savings_bytes = trace_table_batch
            .opened_values_bytes
            .saturating_sub(trace_table_estimated_opened_values_bytes)
            + accumulator_batch
                .opened_values_bytes
                .saturating_sub(accumulator_estimated_opened_values_bytes);

        let packed_trace_zeta_opening_bytes = postcard_len(
            &proof.opened_packed_trace_basis, "packed support opened packed trace basis",
        );
        let packed_trace_zeta_estimated_opening_bytes = terminal_scaled_byte_count(
            packed_trace_zeta_opening_bytes, current_trace_width, paired_trace_width_floor,
        );
        let logup_accumulator_zeta_opening_bytes = postcard_len(
            &proof.opened_logup_accumulator_points_basis,
            "packed support opened LogUp accumulator basis",
        );
        let logup_accumulator_zeta_estimated_opening_bytes = terminal_scaled_byte_count(
            logup_accumulator_zeta_opening_bytes, proof.logup_accumulator_profile.basis_columns,
            paired_logup_accumulator_basis,
        );
        let non_fri_opening_savings_bytes = packed_trace_zeta_opening_bytes
            .saturating_sub(packed_trace_zeta_estimated_opening_bytes)
            + logup_accumulator_zeta_opening_bytes
                .saturating_sub(logup_accumulator_zeta_estimated_opening_bytes);

        TerminalPackedTip5PairedLookupPayloadEstimate {
            current_trace_width,
            paired_trace_width_floor,
            current_split_columns,
            paired_split_columns,
            current_query_interactions,
            paired_query_interactions,
            current_logup_interactions,
            paired_logup_interactions,
            logup_group_size,
            current_logup_groups,
            paired_logup_groups,
            current_logup_accumulator_basis: proof.logup_accumulator_profile.basis_columns,
            paired_logup_accumulator_basis,
            current_trace_table_opened_basis,
            paired_trace_table_opened_basis,
            current_accumulator_opened_basis_per_point,
            paired_accumulator_opened_basis_per_point,
            trace_table_opened_values_bytes: trace_table_batch.opened_values_bytes,
            trace_table_estimated_opened_values_bytes,
            accumulator_opened_values_bytes: accumulator_batch.opened_values_bytes,
            accumulator_estimated_opened_values_bytes,
            fri_opened_value_savings_bytes,
            packed_trace_zeta_opening_bytes,
            packed_trace_zeta_estimated_opening_bytes,
            logup_accumulator_zeta_opening_bytes,
            logup_accumulator_zeta_estimated_opening_bytes,
            non_fri_opening_savings_bytes,
            current_packed_support_fri_bytes: fri_breakdown.total_bytes,
            estimated_packed_support_fri_after_opened_value_savings_bytes: fri_breakdown
                .total_bytes
                .saturating_sub(fri_opened_value_savings_bytes),
            current_optimistic_single_fri_floor_bytes,
            estimated_optimistic_single_fri_floor_after_savings_bytes:
                current_optimistic_single_fri_floor_bytes
                    .saturating_sub(fri_opened_value_savings_bytes)
                    .saturating_sub(non_fri_opening_savings_bytes),
        }
    }

    fn log_terminal_packed_tip5_paired_lookup_payload_estimate(
        label: &str,
        estimate: &TerminalPackedTip5PairedLookupPayloadEstimate,
    ) {
        let target_binary_150k = 150usize * 1024;
        let target_decimal_150k = 150_000usize;
        eprintln!(
            "native terminal packed Tip5 paired 16-bit lookup payload estimate [{label}]: current_trace_width={} paired_trace_width_floor={} current_split_columns={} paired_split_columns={} current_query_interactions={} paired_query_interactions={} current_logup_interactions={} paired_logup_interactions={} logup_group_size={} current_logup_groups={} paired_logup_groups={} current_logup_accumulator_basis={} paired_logup_accumulator_basis={} current_trace_table_opened_basis={} paired_trace_table_opened_basis={} current_accumulator_opened_basis_per_point={} paired_accumulator_opened_basis_per_point={} trace_table_opened_values={} trace_table_estimated_opened_values={} accumulator_opened_values={} accumulator_estimated_opened_values={} fri_opened_value_savings={} packed_trace_zeta_opening={} packed_trace_zeta_estimated_opening={} logup_accumulator_zeta_opening={} logup_accumulator_zeta_estimated_opening={} non_fri_opening_savings={} current_packed_support_fri={} estimated_packed_support_fri_after_opened_value_savings={} current_optimistic_single_fri_floor={} estimated_optimistic_single_fri_floor_after_savings={} over_binary_150k={} over_decimal_150k={} estimate_excludes_two_domain_table_merkle_overhead=1 estimate_excludes_new_quotient_shape=1",
            estimate.current_trace_width,
            estimate.paired_trace_width_floor,
            estimate.current_split_columns,
            estimate.paired_split_columns,
            estimate.current_query_interactions,
            estimate.paired_query_interactions,
            estimate.current_logup_interactions,
            estimate.paired_logup_interactions,
            estimate.logup_group_size,
            estimate.current_logup_groups,
            estimate.paired_logup_groups,
            estimate.current_logup_accumulator_basis,
            estimate.paired_logup_accumulator_basis,
            estimate.current_trace_table_opened_basis,
            estimate.paired_trace_table_opened_basis,
            estimate.current_accumulator_opened_basis_per_point,
            estimate.paired_accumulator_opened_basis_per_point,
            estimate.trace_table_opened_values_bytes,
            estimate.trace_table_estimated_opened_values_bytes,
            estimate.accumulator_opened_values_bytes,
            estimate.accumulator_estimated_opened_values_bytes,
            estimate.fri_opened_value_savings_bytes,
            estimate.packed_trace_zeta_opening_bytes,
            estimate.packed_trace_zeta_estimated_opening_bytes,
            estimate.logup_accumulator_zeta_opening_bytes,
            estimate.logup_accumulator_zeta_estimated_opening_bytes,
            estimate.non_fri_opening_savings_bytes,
            estimate.current_packed_support_fri_bytes,
            estimate.estimated_packed_support_fri_after_opened_value_savings_bytes,
            estimate.current_optimistic_single_fri_floor_bytes,
            estimate.estimated_optimistic_single_fri_floor_after_savings_bytes,
            estimate
                .estimated_optimistic_single_fri_floor_after_savings_bytes
                .saturating_sub(target_binary_150k),
            estimate
                .estimated_optimistic_single_fri_floor_after_savings_bytes
                .saturating_sub(target_decimal_150k),
        );
    }

    fn log_terminal_packed_support_structural_floor(
        label: &str,
        prelude_bytes: usize,
        primitive_bytes: usize,
        merged_bytes: usize,
        packed_support_non_fri_bytes: usize,
        optimistic_duplicate_selected_binding_bytes: usize,
        paired_lookup_estimate: &TerminalPackedTip5PairedLookupPayloadEstimate,
    ) {
        let target_binary_150k = 150usize * 1024;
        let target_decimal_150k = 150_000usize;
        let merged_only_body_floor = prelude_bytes + primitive_bytes + merged_bytes;
        let support_metadata_after_selected_dedup = packed_support_non_fri_bytes
            .saturating_sub(optimistic_duplicate_selected_binding_bytes);
        let paired_support_metadata_after_selected_dedup = support_metadata_after_selected_dedup
            .saturating_sub(paired_lookup_estimate.non_fri_opening_savings_bytes);
        let zero_support_fri_floor = merged_only_body_floor + support_metadata_after_selected_dedup;
        let paired_zero_support_fri_floor =
            merged_only_body_floor + paired_support_metadata_after_selected_dedup;
        let binary_support_payload_headroom =
            target_binary_150k.saturating_sub(merged_only_body_floor);
        let decimal_support_payload_headroom =
            target_decimal_150k.saturating_sub(merged_only_body_floor);
        let current_metadata_required_savings_binary =
            support_metadata_after_selected_dedup.saturating_sub(binary_support_payload_headroom);
        let paired_metadata_required_savings_binary = paired_support_metadata_after_selected_dedup
            .saturating_sub(binary_support_payload_headroom);

        eprintln!(
            "native terminal packed support structural floor [{label}]: merged_only_body_floor={} merged_only_over_binary_150k={} merged_only_headroom_binary_150k={} merged_only_over_decimal_150k={} merged_only_headroom_decimal_150k={} support_metadata_after_selected_dedup={} paired_support_metadata_after_selected_dedup={} zero_support_fri_floor={} zero_support_fri_over_binary_150k={} zero_support_fri_over_decimal_150k={} paired_zero_support_fri_floor={} paired_zero_support_fri_over_binary_150k={} paired_zero_support_fri_over_decimal_150k={} binary_support_payload_headroom={} decimal_support_payload_headroom={} current_metadata_required_savings_binary={} paired_metadata_required_savings_binary={} support_fri_budget_after_current_metadata_binary={} support_fri_budget_after_paired_metadata_binary={}",
            merged_only_body_floor,
            merged_only_body_floor.saturating_sub(target_binary_150k),
            target_binary_150k.saturating_sub(merged_only_body_floor),
            merged_only_body_floor.saturating_sub(target_decimal_150k),
            target_decimal_150k.saturating_sub(merged_only_body_floor),
            support_metadata_after_selected_dedup,
            paired_support_metadata_after_selected_dedup,
            zero_support_fri_floor,
            zero_support_fri_floor.saturating_sub(target_binary_150k),
            zero_support_fri_floor.saturating_sub(target_decimal_150k),
            paired_zero_support_fri_floor,
            paired_zero_support_fri_floor.saturating_sub(target_binary_150k),
            paired_zero_support_fri_floor.saturating_sub(target_decimal_150k),
            binary_support_payload_headroom,
            decimal_support_payload_headroom,
            current_metadata_required_savings_binary,
            paired_metadata_required_savings_binary,
            target_binary_150k.saturating_sub(
                merged_only_body_floor + support_metadata_after_selected_dedup
            ),
            target_binary_150k.saturating_sub(
                merged_only_body_floor + paired_support_metadata_after_selected_dedup
            ),
        );
    }

    fn compact_path_dictionary_stats(
        input_paths: &[p3_circuit_prover::GoldilocksTip5PrunedMerklePaths],
        commit_paths: &[p3_circuit_prover::GoldilocksTip5PrunedMerklePaths],
    ) -> (usize, usize, usize, usize) {
        let path_sets = input_paths.len() + commit_paths.len();
        let mut original_orders = 0usize;
        let mut pruned_paths = 0usize;
        let mut pruned_siblings = 0usize;
        for path_set in input_paths.iter().chain(commit_paths.iter()) {
            original_orders += path_set.paths.original_order.len();
            pruned_paths += path_set.paths.paths.len();
            pruned_siblings += path_set
                .paths
                .paths
                .iter()
                .map(|path| path.siblings.len())
                .sum::<usize>();
        }
        (path_sets, original_orders, pruned_paths, pruned_siblings)
    }

    fn compact_path_frontier_sibling_count(
        input_paths: &[p3_circuit_prover::GoldilocksTip5PrunedMerklePaths],
        commit_paths: &[p3_circuit_prover::GoldilocksTip5PrunedMerklePaths],
    ) -> usize {
        input_paths
            .iter()
            .chain(commit_paths)
            .map(frontier_sibling_count_for_path_set)
            .sum()
    }

    fn frontier_sibling_count_for_path_set(
        path_set: &p3_circuit_prover::GoldilocksTip5PrunedMerklePaths,
    ) -> usize {
        if path_set.full_path_len == 0 || path_set.paths.paths.is_empty() {
            return 0;
        }

        let mut current = path_set
            .paths
            .paths
            .iter()
            .map(|path| path.leaf_index)
            .collect::<std::collections::BTreeSet<_>>();
        let mut frontier_siblings = 0usize;
        for _ in 0..path_set.full_path_len {
            let mut parents = std::collections::BTreeSet::new();
            for &node in &current {
                if !current.contains(&(node ^ 1)) {
                    frontier_siblings += 1;
                }
                parents.insert(node >> 1);
            }
            current = parents;
        }
        frontier_siblings
    }

    #[derive(Clone, Debug)]
    struct AuthPathGroup {
        path_len: usize,
        index_shift: usize,
        index_bits: usize,
    }

    #[derive(Clone, Debug)]
    struct MerklePathCompressionEstimate {
        raw_siblings: usize,
        mean_compressed_siblings: f64,
        mean_digest_savings_bytes: f64,
    }

    fn merkle_path_compression_estimate_for_outer_proof(
        proof: &AiPowL1OuterProof,
        log_blowup: usize,
        log_final_poly_len: usize,
        cap_height: usize,
    ) -> MerklePathCompressionEstimate {
        merkle_path_compression_estimate_for_outer_proof_with_omitted_input_batch(
            proof, log_blowup, log_final_poly_len, cap_height, None,
        )
    }

    fn merkle_path_compression_estimate_for_outer_proof_with_omitted_input_batch(
        proof: &AiPowL1OuterProof,
        log_blowup: usize,
        log_final_poly_len: usize,
        cap_height: usize,
        omitted_input_batch: Option<usize>,
    ) -> MerklePathCompressionEstimate {
        const TRIALS: usize = 256;
        const DIGEST_BYTES: usize = core::mem::size_of::<[u64; DIGEST_ELEMS]>();

        let fri = &proof.proof.opening_proof;
        let groups = auth_path_groups_for_outer_proof(
            proof, log_blowup, log_final_poly_len, cap_height, omitted_input_batch,
        );
        let raw_siblings: usize =
            groups.iter().map(|g| g.path_len).sum::<usize>() * fri.query_proofs.len();

        if groups.is_empty() || fri.query_proofs.is_empty() {
            return MerklePathCompressionEstimate {
                raw_siblings,
                mean_compressed_siblings: raw_siblings as f64,
                mean_digest_savings_bytes: 0.0,
            };
        }

        let num_queries = fri.query_proofs.len();
        let log_global_max_height = groups
            .iter()
            .map(|g| g.index_shift + g.index_bits)
            .max()
            .unwrap_or(0);

        let mut compressed_total = 0usize;
        for trial in 0..TRIALS {
            let global_indices: Vec<usize> = (0..num_queries)
                .map(|query| sample_transcript_shaped_index(trial, query, log_global_max_height))
                .collect();
            for group in &groups {
                let reduced_indices: Vec<usize> = global_indices
                    .iter()
                    .map(|&index| reduced_index(index, group.index_shift, group.index_bits))
                    .collect();
                compressed_total +=
                    compressed_sibling_count(cap_height, group.path_len, &reduced_indices);
            }
        }

        let mean_compressed_siblings = compressed_total as f64 / TRIALS as f64;
        let mean_digest_savings_bytes =
            (raw_siblings as f64 - mean_compressed_siblings) * DIGEST_BYTES as f64;
        MerklePathCompressionEstimate {
            raw_siblings,
            mean_compressed_siblings,
            mean_digest_savings_bytes,
        }
    }

    fn auth_path_groups_for_outer_proof(
        proof: &AiPowL1OuterProof,
        log_blowup: usize,
        log_final_poly_len: usize,
        cap_height: usize,
        omitted_input_batch: Option<usize>,
    ) -> Vec<AuthPathGroup> {
        let Some(first_query) = proof.proof.opening_proof.query_proofs.first() else {
            return Vec::new();
        };

        let log_arities: Vec<usize> = first_query
            .commit_phase_openings
            .iter()
            .map(|step| step.log_arity as usize)
            .collect();
        let log_global_max_height =
            log_arities.iter().sum::<usize>() + log_blowup + log_final_poly_len;

        let mut groups = Vec::new();
        for (batch_idx, batch) in first_query.input_proof.iter().enumerate() {
            if omitted_input_batch == Some(batch_idx) {
                continue;
            }
            let path_len = batch.opening_proof.len();
            let index_bits = cap_height + path_len;
            groups.push(AuthPathGroup {
                path_len,
                index_shift: log_global_max_height.saturating_sub(index_bits),
                index_bits,
            });
        }

        let mut folded_shift = 0usize;
        for step in &first_query.commit_phase_openings {
            folded_shift += step.log_arity as usize;
            let path_len = step.opening_proof.len();
            let index_bits = cap_height + path_len;
            groups.push(AuthPathGroup {
                path_len,
                index_shift: folded_shift,
                index_bits,
            });
        }

        groups
    }

    #[derive(Clone, Copy, Debug)]
    struct FriInputBatchByteBreakdown {
        index: usize,
        label: &'static str,
        total_bytes: usize,
        opened_values_bytes: usize,
        merkle_bytes: usize,
    }

    fn input_batch_bytes_by_label(
        batches: &[FriInputBatchByteBreakdown],
        label: &str,
    ) -> Option<FriInputBatchByteBreakdown> {
        batches.iter().copied().find(|batch| batch.label == label)
    }

    fn fri_input_batch_byte_breakdown_for_outer_proof(
        proof: &AiPowL1OuterProof,
    ) -> Vec<FriInputBatchByteBreakdown> {
        let labels = fri_input_batch_labels_for_outer_proof(proof);
        let mut totals = labels
            .iter()
            .enumerate()
            .map(|(index, label)| FriInputBatchByteBreakdown {
                index,
                label,
                total_bytes: 0,
                opened_values_bytes: 0,
                merkle_bytes: 0,
            })
            .collect::<Vec<_>>();

        for query in &proof.proof.opening_proof.query_proofs {
            assert_eq!(
                query.input_proof.len(),
                labels.len(),
                "FRI input proof batch count must match commitment schedule labels"
            );
            for (index, batch) in query.input_proof.iter().enumerate() {
                totals[index].total_bytes +=
                    postcard_len(batch, "FRI input batch for compact projection");
                totals[index].opened_values_bytes += postcard_len(
                    &batch.opened_values, "FRI input batch opened values for compact projection",
                );
                totals[index].merkle_bytes += postcard_len(
                    &batch.opening_proof, "FRI input batch Merkle proof for compact projection",
                );
            }
        }

        totals
    }

    fn fri_input_batch_labels_for_outer_proof(proof: &AiPowL1OuterProof) -> Vec<&'static str> {
        let mut labels = Vec::new();
        if proof.proof.commitments.random.is_some() {
            labels.push("random");
        }
        labels.push("trace");
        labels.push("quotient");
        if proof.stark_common.preprocessed.is_some() {
            labels.push("preprocessed");
        }
        if proof.proof.commitments.permutation.is_some() {
            labels.push("permutation");
        }
        labels
    }

    fn preprocessed_ood_opening_bytes_for_outer_proof(proof: &AiPowL1OuterProof) -> usize {
        proof
            .proof
            .opened_values
            .instances
            .iter()
            .map(|instance| {
                let local = instance
                    .base_opened_values
                    .preprocessed_local
                    .as_ref()
                    .map(|values| postcard_len(values, "preprocessed OOD local values"))
                    .unwrap_or(0);
                let next = instance
                    .base_opened_values
                    .preprocessed_next
                    .as_ref()
                    .map(|values| postcard_len(values, "preprocessed OOD next values"))
                    .unwrap_or(0);
                local + next
            })
            .sum()
    }

    fn compressed_sibling_count(cap_height: usize, path_len: usize, indices: &[usize]) -> usize {
        if path_len == 0 || indices.is_empty() {
            return 0;
        }
        let height = cap_height + path_len;
        if height >= usize::BITS as usize {
            return path_len * indices.len();
        }

        let num_leaves = 1usize << height;
        let mut known = std::collections::BTreeSet::new();
        for &leaf in indices {
            let mut node = (leaf % num_leaves) + num_leaves;
            for _ in 0..path_len {
                known.insert(node);
                node >>= 1;
            }
        }

        let mut compressed = 0usize;
        for &leaf in indices {
            let mut node = (leaf % num_leaves) + num_leaves;
            for _ in 0..path_len {
                let sibling = node ^ 1;
                if known.insert(sibling) {
                    compressed += 1;
                }
                node >>= 1;
                known.insert(node);
            }
        }
        compressed
    }

    fn reduced_index(index: usize, shift: usize, bits: usize) -> usize {
        if bits == 0 {
            return 0;
        }
        let shifted = index.checked_shr(shift as u32).unwrap_or(0);
        shifted & low_bits_mask(bits)
    }

    fn sample_transcript_shaped_index(trial: usize, query: usize, bits: usize) -> usize {
        if bits == 0 {
            return 0;
        }
        let seed = 0x9e37_79b9_7f4a_7c15u64
            ^ ((trial as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9))
            ^ ((query as u64).wrapping_mul(0x94d0_49bb_1331_11eb));
        (splitmix64(seed) as usize) & low_bits_mask(bits)
    }

    fn low_bits_mask(bits: usize) -> usize {
        if bits >= usize::BITS as usize {
            usize::MAX
        } else {
            (1usize << bits) - 1
        }
    }

    fn splitmix64(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn log_terminal_production_component_sizes(
        label: &str,
        certificate: &p3_recursion::terminal::TerminalCertificate,
    ) {
        let (production_proof, trailing): (TerminalProductionProof, &[u8]) =
            postcard::take_from_bytes(&certificate.proof_body)
                .expect("terminal production proof body must decode for size accounting");
        assert!(
            trailing.is_empty(),
            "terminal production proof size accounting must consume the whole body"
        );

        let prelude_bytes = postcard_len(&production_proof.prelude, "terminal prelude");
        let primitive_bytes = postcard_len(
            &production_proof.primitive_r1cs_proof, "primitive r1cs proof",
        );
        let primitive_rounds_bytes = postcard_len(
            &production_proof.primitive_r1cs_proof.rounds, "primitive r1cs row-product rounds",
        );
        let matrix_sumcheck_bytes = postcard_len(
            &production_proof.primitive_r1cs_proof.matrix_sumcheck,
            "primitive r1cs matrix sumcheck",
        );
        let matrix_rounds_bytes = postcard_len(
            &production_proof.primitive_r1cs_proof.matrix_sumcheck.rounds,
            "primitive r1cs matrix sumcheck rounds",
        );
        let assignment_eval = &production_proof
            .primitive_r1cs_proof
            .matrix_sumcheck
            .assignment_evaluation;
        let assignment_eval_bytes =
            postcard_len(assignment_eval, "primitive r1cs assignment evaluation");
        let assignment_prefix_bytes = postcard_len(
            &assignment_eval.public_prefix_proof, "primitive r1cs assignment public prefix proof",
        );
        let assignment_round_openings_bytes = postcard_len(
            &assignment_eval.round_openings, "primitive r1cs assignment round openings",
        );

        let (
            npo_bytes,
            tip5_hidden_bytes,
            assignment_witness_multi_bytes,
            npo_hidden_values,
            npo_assignment_values,
            npo_boolean_bits,
            npo_frontier_nodes,
            npo_assignment_value_bytes,
            npo_assignment_mask_bytes,
            npo_assignment_boolean_bytes,
            npo_assignment_frontier_bytes,
            npo_assignment_nonzero_coeffs,
            npo_assignment_dense_coeffs,
            npo_assignment_estimated_non_boolean_values,
            npo_assignment_zero_coeffs,
        ) = match &production_proof.npo_exhaustive_proof {
            Some(npo) => {
                let assignment = &npo.assignment_witness_multi_opening;
                let dimension = <Challenge as BasedVectorSpace<Val>>::DIMENSION;
                let dense_coeffs = if dimension > 1 {
                    assignment.value_basis_nonzero_masks.len() * 8
                } else {
                    assignment.value_basis_flat.len()
                };
                let nonzero_coeffs = assignment.value_basis_flat.len();
                (
                    postcard_len(npo, "terminal npo exhaustive proof"),
                    postcard_len(
                        &npo.tip5_hidden_input_values_le, "terminal npo hidden Tip5 values",
                    ),
                    postcard_len(assignment, "terminal npo assignment witness multiproof"),
                    npo.tip5_hidden_input_values_le.len(),
                    assignment.value_basis_flat.len(),
                    assignment.boolean_value_bits.len(),
                    assignment.frontier.len(),
                    postcard_len(
                        &assignment.value_basis_flat, "terminal npo assignment witness value limbs",
                    ),
                    postcard_len(
                        &assignment.value_basis_nonzero_masks,
                        "terminal npo assignment witness nonzero masks",
                    ),
                    postcard_len(
                        &assignment.boolean_value_bits,
                        "terminal npo assignment witness boolean bits",
                    ),
                    postcard_len(
                        &assignment.frontier, "terminal npo assignment witness frontier",
                    ),
                    nonzero_coeffs,
                    dense_coeffs,
                    dense_coeffs / dimension,
                    dense_coeffs.saturating_sub(nonzero_coeffs),
                )
            }
            None => (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
        };

        eprintln!(
            "native terminal production body components [{label}]: body={} bytes prelude={} primitive_r1cs={} primitive_rounds={} matrix_sumcheck={} matrix_rounds={} assignment_eval={} assignment_prefix={} assignment_round_openings={} npo_exhaustive={} npo_tip5_hidden={} npo_assignment_witness_multi={} npo_hidden_values={} npo_assignment_values={} npo_boolean_bits={} npo_frontier_nodes={}",
            certificate.proof_body.len(),
            prelude_bytes,
            primitive_bytes,
            primitive_rounds_bytes,
            matrix_sumcheck_bytes,
            matrix_rounds_bytes,
            assignment_eval_bytes,
            assignment_prefix_bytes,
            assignment_round_openings_bytes,
            npo_bytes,
            tip5_hidden_bytes,
            assignment_witness_multi_bytes,
            npo_hidden_values,
            npo_assignment_values,
            npo_boolean_bits,
            npo_frontier_nodes,
        );
        eprintln!(
            "native terminal production npo assignment multiproof components [{label}]: values_bytes={} masks_bytes={} boolean_bytes={} frontier_bytes={} nonzero_coeffs={} dense_coeffs={} estimated_non_boolean_values={} zero_coeffs={} frontier_nodes={}",
            npo_assignment_value_bytes,
            npo_assignment_mask_bytes,
            npo_assignment_boolean_bytes,
            npo_assignment_frontier_bytes,
            npo_assignment_nonzero_coeffs,
            npo_assignment_dense_coeffs,
            npo_assignment_estimated_non_boolean_values,
            npo_assignment_zero_coeffs,
            npo_frontier_nodes,
        );
    }

    fn log_terminal_primitive_r1cs_proof_breakdown(
        label: &str,
        relation: &TerminalSparseR1csRelation<Challenge>,
        proof: &TerminalR1csRowProductSumcheckProof,
    ) {
        let primitive_bytes = postcard_len(proof, "primitive r1cs proof");
        let primitive_rounds_bytes =
            postcard_len(&proof.rounds, "primitive r1cs row-product rounds");
        let matrix_sumcheck_bytes =
            postcard_len(&proof.matrix_sumcheck, "primitive r1cs matrix sumcheck");
        let matrix_rounds_bytes = postcard_len(
            &proof.matrix_sumcheck.rounds, "primitive r1cs matrix sumcheck rounds",
        );
        let assignment_eval = &proof.matrix_sumcheck.assignment_evaluation;
        let assignment_eval_bytes =
            postcard_len(assignment_eval, "primitive r1cs assignment evaluation");
        let assignment_prefix_bytes = postcard_len(
            &assignment_eval.public_prefix_proof, "primitive r1cs assignment public prefix proof",
        );
        let assignment_fold_roots_bytes = postcard_len(
            &assignment_eval.fold_commitment_roots, "primitive r1cs assignment fold roots",
        );
        let assignment_final_value_bytes = postcard_len(
            &assignment_eval.final_value_basis, "primitive r1cs assignment final value",
        );
        let assignment_round_openings_bytes = postcard_len(
            &assignment_eval.round_openings, "primitive r1cs assignment round openings",
        );
        let assignment_opening_value_bytes: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| {
                postcard_len(
                    &opening.value_basis_flat, "primitive r1cs assignment opening values",
                )
            })
            .sum();
        let assignment_opening_mask_bytes: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| {
                postcard_len(
                    &opening.value_basis_nonzero_masks,
                    "primitive r1cs assignment opening nonzero masks",
                )
            })
            .sum();
        let assignment_opening_boolean_bytes: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| {
                postcard_len(
                    &opening.boolean_value_bits, "primitive r1cs assignment opening boolean bits",
                )
            })
            .sum();
        let assignment_opening_frontier_bytes: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| {
                postcard_len(
                    &opening.frontier, "primitive r1cs assignment opening frontier",
                )
            })
            .sum();
        let assignment_opening_value_limbs: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| opening.value_basis_flat.len())
            .sum();
        let assignment_opening_masks: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| opening.value_basis_nonzero_masks.len())
            .sum();
        let assignment_opening_boolean_bits: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| opening.boolean_value_bits.len())
            .sum();
        let assignment_opening_frontier_nodes: usize = assignment_eval
            .round_openings
            .iter()
            .map(|opening| opening.frontier.len())
            .sum();

        eprintln!(
            "native terminal primitive R1CS relation [{label}]: rows={} log_rows={} variables={} log_variables={} public={} witness={} entries={} row_product_rounds={} matrix_rounds={} assignment_fold_rounds={} assignment_fold_roots={}",
            relation.rows,
            relation.log_rows,
            relation.variables,
            relation.log_variables,
            relation.public_count,
            relation.witness_count,
            relation.entries.len(),
            proof.rounds.len(),
            proof.matrix_sumcheck.rounds.len(),
            assignment_eval.round_openings.len(),
            assignment_eval.fold_commitment_roots.len(),
        );
        eprintln!(
            "native terminal primitive R1CS proof components [{label}]: primitive_r1cs={} row_product_rounds={} matrix_sumcheck={} matrix_rounds={} assignment_eval={} assignment_prefix={} assignment_fold_roots={} assignment_final_value={} assignment_round_openings={}",
            primitive_bytes,
            primitive_rounds_bytes,
            matrix_sumcheck_bytes,
            matrix_rounds_bytes,
            assignment_eval_bytes,
            assignment_prefix_bytes,
            assignment_fold_roots_bytes,
            assignment_final_value_bytes,
            assignment_round_openings_bytes,
        );
        eprintln!(
            "native terminal primitive R1CS assignment openings [{label}]: values_bytes={} masks_bytes={} boolean_bytes={} frontier_bytes={} value_limbs={} mask_bytes={} boolean_bytes_len={} frontier_nodes={} public_prefix_frontier_nodes={}",
            assignment_opening_value_bytes,
            assignment_opening_mask_bytes,
            assignment_opening_boolean_bytes,
            assignment_opening_frontier_bytes,
            assignment_opening_value_limbs,
            assignment_opening_masks,
            assignment_opening_boolean_bits,
            assignment_opening_frontier_nodes,
            assignment_eval.public_prefix_proof.frontier.len(),
        );
    }

    #[test]
    #[ignore = "native terminal proof over the full composite verifier is an opt-in measurement"]
    fn terminal_recursive_certificate_round_trip_verifies() {
        measure_and_verify_terminal_certificate_for_profile(
            "TEST_PEARL",
            CircuitConfig::TEST_PEARL,
        );
    }

    #[test]
    #[ignore = "native terminal proof pure-query profile sweep is opt-in"]
    fn terminal_recursive_certificate_for_pure_query_lb6_nq10_measures() {
        measure_and_verify_terminal_certificate_for_profile(
            "PURE_QUERY_LB6_NQ10",
            CircuitConfig {
                log_blowup: 6,
                pow_bits: 0,
                num_queries: 10,
            },
        );
    }

    fn measure_terminal_fri_native_residual_zero_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("FRI-native residual-zero diagnostic must build L1 verifier circuit");
        let l1_circuit_build_ms = l1_build_start.elapsed().as_millis();
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: l1_circuit_build_ms={l1_circuit_build_ms}"
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("FRI-native residual-zero diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("FRI-native residual-zero diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("FRI-native residual-zero diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let assignment_check_start = std::time::Instant::now();
        let assignment_check_result =
            compiler.verify_assignment_with_goldilocks_npos(&vk, &witness);
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: assignment_npo_check_ms={} assignment_npo_check={:?}",
            assignment_check_start.elapsed().as_millis(),
            assignment_check_result.as_ref().map(|_| "ok"),
        );

        let layout_metrics =
            NativeTerminalCompiler::terminal_npo_polynomial_layout_metrics::<Challenge>(&vk)
                .expect("FRI-native residual-zero diagnostic must derive NPO layout metrics");

        let table = compiler
            .terminal_npo_polynomial_table_goldilocks(&vk, &witness)
            .expect("FRI-native residual-zero diagnostic must build NPO table");
        print_terminal_npo_residual_distribution(label, &table);

        let columns_start = std::time::Instant::now();
        let columns = compiler
            .terminal_npo_polynomial_columns_goldilocks(&vk, &witness)
            .expect("FRI-native residual-zero diagnostic must build NPO columns");
        let columns_elapsed = columns_start.elapsed();
        let mut residual_column_count = 0usize;
        let mut residual_nonzero_values = 0usize;
        let mut first_nonzero_residual = None;
        for (column_index, label) in columns.labels.iter().enumerate() {
            if !label.starts_with("residual_value_") {
                continue;
            }
            residual_column_count += 1;
            for (row, value) in columns.columns[column_index].iter().enumerate() {
                if *value != Challenge::ZERO {
                    residual_nonzero_values += 1;
                    first_nonzero_residual.get_or_insert_with(|| {
                        let basis =
                            <Challenge as BasedVectorSpace<Val>>::as_basis_coefficients_slice(
                                value,
                            );
                        (
                            row,
                            label.clone(),
                            basis.iter().copied().collect::<Vec<_>>(),
                        )
                    });
                }
            }
        }
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: npo_columns_ms={} residual_columns={} residual_nonzero_values={} first_nonzero_residual={:?}",
            columns_elapsed.as_millis(),
            residual_column_count,
            residual_nonzero_values,
            first_nonzero_residual,
        );

        let roots_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_polynomial_fri_prelude_commitments_goldilocks(
                &columns,
                TerminalNpoPolynomialFriColumnSet::ProverDependent,
            )
            .expect("FRI-native residual-zero diagnostic must commit prover-dependent columns");
        let roots_elapsed = roots_start.elapsed();
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: prover_dependent_root_ms={}",
            roots_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("FRI-native residual-zero diagnostic must build terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal FRI-native residual-zero candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let proof = compiler
            .prove_terminal_npo_polynomial_fri_compact_residual_zero_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("FRI-native residual-zero proof must build");
        let prove_elapsed = prove_start.elapsed();

        assert_eq!(
            proof.selected_profile.field_columns,
            layout_metrics.prover_dependent_field_columns
        );
        assert_eq!(
            proof.selected_profile.basis_columns,
            layout_metrics.prover_dependent_profile.basis_columns
        );

        let proof_bytes = postcard_len(&proof, "FRI-native residual-zero proof");
        let selected_commitment_bytes = postcard_len(
            &proof.selected_commitment, "FRI-native residual-zero selected commitment",
        );
        let composition_commitment_bytes = postcard_len(
            &proof.composition_commitment, "FRI-native residual-zero composition commitment",
        );
        let opened_selected_bytes = postcard_len(
            &proof.opened_selected_basis, "FRI-native residual-zero selected zeta openings",
        );
        let opened_composition_bytes = postcard_len(
            &proof.opened_composition_basis, "FRI-native residual-zero composition zeta openings",
        );
        let compact_fri_bytes = postcard_len(&proof.proof, "FRI-native residual-zero compact FRI");

        let verify_start = std::time::Instant::now();
        let verify_result = compiler
            .verify_terminal_npo_polynomial_fri_compact_residual_zero_goldilocks::<Challenge>(
                &vk, &witness.public_inputs, &prelude, &proof,
            );
        let verify_elapsed = verify_start.elapsed();
        let verification_status = match (residual_nonzero_values, verify_result) {
            (0, Ok(_)) => "verified",
            (0, Err(err)) => {
                panic!("FRI-native residual-zero proof must verify for zero residuals: {err:?}")
            }
            (_, Ok(_)) => {
                panic!("FRI-native residual-zero proof must reject nonzero residual columns")
            }
            (_, Err(err)) => {
                assert!(
                    matches!(
                        err,
                        NativeTerminalVerifyError::TerminalNpoPolynomialFriRelationMismatch { .. }
                    ),
                    "nonzero residual columns should fail the residual-zero relation, got {err:?}"
                );
                "rejected_nonzero_residuals"
            }
        };
        let total_candidate_elapsed = total_start.elapsed();

        eprintln!(
            "native terminal FRI-native residual-zero NPO candidate over ai-pow composite verifier [{label}]: proof={} bytes selected_commitment={} composition_commitment={} opened_selected={} opened_composition={} compact_fri={} npo_rows={} padded_rows={} prover_dependent_field_columns={} prover_dependent_basis_columns={} residual_components={} residual_nonzero_values={} verification_status={} residual_zero_opened_basis_columns={} residual_zero_min_opened_limb_bytes={} l1_verify_ms={} compile_ms={} npo_columns_ms={} prover_dependent_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            selected_commitment_bytes,
            composition_commitment_bytes,
            opened_selected_bytes,
            opened_composition_bytes,
            compact_fri_bytes,
            layout_metrics.relation_profile.rows,
            layout_metrics.prover_dependent_profile.padded_rows,
            layout_metrics.prover_dependent_field_columns,
            layout_metrics.prover_dependent_profile.basis_columns,
            layout_metrics.relation_profile.residual_components,
            residual_nonzero_values,
            verification_status,
            layout_metrics.fri_native_residual_zero_opened_basis_columns,
            layout_metrics.fri_native_residual_zero_min_opened_limb_bytes,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            columns_elapsed.as_millis(),
            roots_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_candidate_elapsed.as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite FRI-native residual-zero terminal NPO candidate measurement is opt-in"]
    fn terminal_fri_native_residual_zero_candidate_for_prod_baseline_measures() {
        measure_terminal_fri_native_residual_zero_candidate_for_profile(
            "PROD",
            CircuitConfig::PROD,
        );
    }

    fn measure_terminal_assignment_compact_fri_floor_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };
        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("assignment compact-FRI floor diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("assignment compact-FRI floor diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("assignment compact-FRI floor diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("assignment compact-FRI floor diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let assignment_commit_start = std::time::Instant::now();
        let assignment_oracle = compiler
            .commit_terminal_assignment_goldilocks(&vk, &witness.public_inputs, &witness)
            .expect("assignment compact-FRI floor diagnostic must commit terminal assignment");
        let assignment_commitment = assignment_oracle.commitment();
        let assignment_commit_elapsed = assignment_commit_start.elapsed();
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: assignment_commit_ms={}",
            assignment_commit_elapsed.as_millis()
        );
        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![assignment_commitment.root],
            )
            .expect("assignment compact-FRI floor diagnostic must build terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let fri_floor_prove_start = std::time::Instant::now();
        let fri_floor_proof = compiler
            .prove_terminal_assignment_compact_fri_floor_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("assignment compact-FRI floor proof must build");
        let fri_floor_prove_elapsed = fri_floor_prove_start.elapsed();
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: fri_floor_prove_ms={}",
            fri_floor_prove_elapsed.as_millis()
        );

        let fri_floor_verify_start = std::time::Instant::now();
        let opened =
            NativeTerminalCompiler::verify_terminal_assignment_compact_fri_floor_goldilocks(
                &prelude, assignment_commitment.values_len, &fri_floor_proof,
            )
            .expect("assignment compact-FRI floor proof must verify");
        let fri_floor_verify_elapsed = fri_floor_verify_start.elapsed();
        assert_eq!(opened.len(), fri_floor_proof.profile.basis_columns);
        eprintln!(
            "native terminal assignment compact-FRI floor phase [{label}]: fri_floor_verify_ms={}",
            fri_floor_verify_elapsed.as_millis()
        );

        let proof_bytes = postcard_len(&fri_floor_proof, "assignment compact-FRI floor proof");
        let compact_fri_bytes = postcard_len(
            &fri_floor_proof.proof, "assignment compact-FRI floor compact FRI proof",
        );
        let opened_values_bytes = postcard_len(
            &fri_floor_proof.opened_values_basis, "assignment compact-FRI floor opened values",
        );
        let total_candidate_elapsed = total_start.elapsed();
        eprintln!(
            "native terminal assignment compact-FRI floor over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_values={} rows={} padded_rows={} basis_columns={} assignment_len={} l1_verify_ms={} compile_ms={} assignment_commit_ms={} prelude_ms={} fri_floor_prove_ms={} fri_floor_verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_values_bytes,
            fri_floor_proof.profile.rows,
            fri_floor_proof.profile.padded_rows,
            fri_floor_proof.profile.basis_columns,
            assignment_commitment.values_len,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            assignment_commit_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            fri_floor_prove_elapsed.as_millis(),
            fri_floor_verify_elapsed.as_millis(),
            total_candidate_elapsed.as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite assignment compact-FRI floor measurement is opt-in"]
    fn terminal_assignment_compact_fri_floor_for_prod_baseline_measures() {
        measure_terminal_assignment_compact_fri_floor_for_profile("PROD", CircuitConfig::PROD);
    }

    fn measure_terminal_merged_value_bridge_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };
        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("merged value-bridge diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("merged value-bridge diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("merged value-bridge diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("merged value-bridge diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let assignment_commit_start = std::time::Instant::now();
        let assignment_oracle = compiler
            .commit_terminal_assignment_goldilocks(&vk, &witness.public_inputs, &witness)
            .expect("merged value-bridge diagnostic must commit terminal assignment");
        let assignment_commitment = assignment_oracle.commitment();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: assignment_commit_ms={}",
            assignment_commit_start.elapsed().as_millis()
        );
        let merged_root_start = std::time::Instant::now();
        let merged_value_bridge_prepared = compiler
            .prepare_terminal_npo_fri_residual_zero_recompose_value_bridge_goldilocks(&vk, &witness)
            .expect("merged value-bridge diagnostic must prepare merged NPO proof data");
        let merged_roots = merged_value_bridge_prepared.prelude_commitments().to_vec();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: merged_npo_root_ms={}",
            merged_root_start.elapsed().as_millis()
        );
        let mut prelude_roots = vec![assignment_commitment.root];
        prelude_roots.extend(merged_roots);
        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("merged value-bridge diagnostic must build terminal prelude");
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: prelude_ms={}",
            prelude_start.elapsed().as_millis()
        );

        let primitive_prove_start = std::time::Instant::now();
        let primitive_r1cs_proof = compiler
            .prove_terminal_r1cs_row_product_sumcheck_prelude_checked_goldilocks(
                &vk, &witness.public_inputs, &prelude, &assignment_oracle, &witness,
            )
            .expect("merged value-bridge diagnostic must prove primitive R1CS");
        let primitive_prove_elapsed = primitive_prove_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: primitive_prove_ms={}",
            primitive_prove_elapsed.as_millis()
        );
        let sparse_relation = vk
            .primitive_sparse_r1cs_relation()
            .expect("merged value-bridge diagnostic must derive primitive sparse R1CS relation");
        log_terminal_primitive_r1cs_proof_breakdown(label, &sparse_relation, &primitive_r1cs_proof);

        let merged_prove_start = std::time::Instant::now();
        let merged_value_bridge_proof = compiler
            .prove_terminal_npo_polynomial_fri_residual_zero_recompose_value_bridge_prepared_prelude_checked_goldilocks(
                &prelude,
                &merged_value_bridge_prepared,
            )
            .expect("merged value-bridge diagnostic must prove merged NPO value bridge");
        let merged_prove_elapsed = merged_prove_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: merged_value_bridge_prove_ms={}",
            merged_prove_elapsed.as_millis()
        );

        let primitive_verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_r1cs_row_product_sumcheck_goldilocks(
                &vk, &witness.public_inputs, &prelude, &assignment_commitment,
                &primitive_r1cs_proof,
            )
            .expect("merged value-bridge diagnostic primitive proof must verify");
        let primitive_verify_elapsed = primitive_verify_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: primitive_verify_ms={}",
            primitive_verify_elapsed.as_millis()
        );
        let merged_verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_polynomial_fri_residual_zero_recompose_value_bridge_goldilocks(
                &vk, &witness.public_inputs, &prelude, &merged_value_bridge_proof,
            )
            .expect("merged value-bridge diagnostic NPO proof must verify");
        let merged_verify_elapsed = merged_verify_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge candidate phase [{label}]: merged_value_bridge_verify_ms={}",
            merged_verify_elapsed.as_millis()
        );

        let candidate_body = (
            prelude.clone(),
            primitive_r1cs_proof.clone(),
            merged_value_bridge_proof.clone(),
        );
        let body_bytes = postcard_len(&candidate_body, "merged value-bridge candidate body");
        let prelude_bytes = postcard_len(&prelude, "merged value-bridge candidate prelude");
        let primitive_bytes = postcard_len(
            &primitive_r1cs_proof, "merged value-bridge candidate primitive proof",
        );
        let merged_bytes = postcard_len(
            &merged_value_bridge_proof, "merged value-bridge candidate NPO proof",
        );
        let merged_fri_bytes = postcard_len(
            &merged_value_bridge_proof.proof, "merged value-bridge candidate compact FRI",
        );
        let total_prove_elapsed = primitive_prove_elapsed + merged_prove_elapsed;
        let total_verify_elapsed = primitive_verify_elapsed + merged_verify_elapsed;
        eprintln!(
            "native terminal merged value-bridge candidate over ai-pow composite verifier [{label}]: body={} bytes prelude={} primitive_r1cs={} merged_value_bridge={} merged_value_bridge_fri={} l1_verify_ms={} compile_ms={} primitive_prove_ms={} merged_value_bridge_prove_ms={} total_prove_ms={} primitive_verify_ms={} merged_value_bridge_verify_ms={} total_verify_ms={} total_wall_ms={}",
            body_bytes,
            prelude_bytes,
            primitive_bytes,
            merged_bytes,
            merged_fri_bytes,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            primitive_prove_elapsed.as_millis(),
            merged_prove_elapsed.as_millis(),
            total_prove_elapsed.as_millis(),
            primitive_verify_elapsed.as_millis(),
            merged_verify_elapsed.as_millis(),
            total_verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite merged value-bridge terminal candidate measurement is opt-in"]
    fn terminal_merged_value_bridge_candidate_for_prod_baseline_measures() {
        measure_terminal_merged_value_bridge_candidate_for_profile("PROD", CircuitConfig::PROD);
    }

    fn measure_terminal_packed_tip5_air_algebra_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("packed Tip5 AIR diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("packed Tip5 AIR diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("packed Tip5 AIR diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("packed Tip5 AIR diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect("packed Tip5 AIR diagnostic must build packed lookup trace");
        let trace_elapsed = trace_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={} quotient_rows={}",
            trace_elapsed.as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
            packed_profile.algebra_quotient_rows,
        );

        let root_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_fri_prelude_commitments_goldilocks(
                &packed_profile,
                &packed_trace,
            )
            .expect("packed Tip5 AIR diagnostic must commit packed trace");
        let root_elapsed = root_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: packed_trace_root_ms={}",
            root_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("packed Tip5 AIR diagnostic must build terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let packed_proof = compiler
            .prove_terminal_npo_tip5_packed_lookup_air_algebra_quotient_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("packed Tip5 AIR algebra proof must build");
        let prove_elapsed = prove_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate phase [{label}]: prove_ms={}",
            prove_elapsed.as_millis()
        );

        let verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_air_algebra_quotient_goldilocks::<Challenge>(
                &vk, &witness.public_inputs, &prelude, &packed_proof,
            )
            .expect("packed Tip5 AIR algebra proof must verify");
        let verify_elapsed = verify_start.elapsed();

        let proof_bytes = postcard_len(&packed_proof, "packed Tip5 AIR algebra proof");
        let compact_fri_bytes = postcard_len(
            &packed_proof.proof, "packed Tip5 AIR algebra compact FRI proof",
        );
        let opened_trace_bytes = postcard_len(
            &packed_proof.opened_trace_basis, "packed Tip5 AIR algebra opened trace",
        );
        let opened_quotient_bytes = postcard_len(
            &packed_proof.opened_quotient_basis, "packed Tip5 AIR algebra opened quotient",
        );
        eprintln!(
            "native terminal packed Tip5 AIR algebra candidate over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_trace={} opened_quotient={} packed_rows={} packed_padded_rows={} packed_width={} quotient_rows={} l1_verify_ms={} compile_ms={} packed_trace_ms={} packed_trace_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_trace_bytes,
            opened_quotient_bytes,
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
            packed_profile.algebra_quotient_rows,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            trace_elapsed.as_millis(),
            root_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite packed Tip5 AIR algebra terminal candidate measurement is opt-in"]
    fn terminal_packed_tip5_air_algebra_candidate_for_prod_baseline_measures() {
        measure_terminal_packed_tip5_air_algebra_candidate_for_profile("PROD", CircuitConfig::PROD);
    }

    fn measure_terminal_packed_tip5_npo_io_projection_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("packed Tip5 NPO-IO projection diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("packed Tip5 NPO-IO projection diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("packed Tip5 NPO-IO projection diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("packed Tip5 NPO-IO projection diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect("packed Tip5 NPO-IO projection diagnostic must build packed lookup trace");
        let trace_elapsed = trace_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={}",
            trace_elapsed.as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
        );

        let root_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_npo_io_projection_prelude_commitments_goldilocks(
                &packed_profile,
                &packed_trace,
            )
            .expect("packed Tip5 NPO-IO projection diagnostic must commit packed trace/projection");
        let root_elapsed = root_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: packed_projection_root_ms={}",
            root_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("packed Tip5 NPO-IO projection diagnostic must build terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let packed_proof = compiler
            .prove_terminal_npo_tip5_packed_lookup_npo_io_projection_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("packed Tip5 NPO-IO projection proof must build");
        let prove_elapsed = prove_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate phase [{label}]: prove_ms={}",
            prove_elapsed.as_millis()
        );

        let verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_npo_io_projection_goldilocks::<Challenge>(
                &vk, &witness.public_inputs, &prelude, &packed_proof,
            )
            .expect("packed Tip5 NPO-IO projection proof must verify");
        let verify_elapsed = verify_start.elapsed();

        let proof_bytes = postcard_len(&packed_proof, "packed Tip5 NPO-IO projection proof");
        let compact_fri_bytes = postcard_len(
            &packed_proof.proof, "packed Tip5 NPO-IO projection compact FRI proof",
        );
        let opened_trace_bytes = postcard_len(
            &packed_proof.opened_trace_basis, "packed Tip5 NPO-IO projection opened trace",
        );
        let opened_npo_io_bytes = postcard_len(
            &packed_proof.opened_npo_io_basis, "packed Tip5 NPO-IO projection opened NPO IO",
        );
        let opened_quotient_bytes = postcard_len(
            &packed_proof.opened_quotient_basis, "packed Tip5 NPO-IO projection opened quotient",
        );
        eprintln!(
            "native terminal packed Tip5 NPO-IO projection candidate over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_trace={} opened_npo_io={} opened_quotient={} packed_rows={} packed_padded_rows={} packed_width={} npo_io_columns={} quotient_rows={} l1_verify_ms={} compile_ms={} packed_trace_ms={} packed_projection_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_trace_bytes,
            opened_npo_io_bytes,
            opened_quotient_bytes,
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
            packed_proof.npo_io_profile.basis_columns,
            packed_proof.quotient_profile.padded_rows,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            trace_elapsed.as_millis(),
            root_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite packed Tip5 NPO-IO projection terminal candidate measurement is opt-in"]
    fn terminal_packed_tip5_npo_io_projection_candidate_for_prod_baseline_measures() {
        measure_terminal_packed_tip5_npo_io_projection_candidate_for_profile(
            "PROD",
            CircuitConfig::PROD,
        );
    }

    fn measure_terminal_packed_tip5_selected_npo_io_bridge_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("packed Tip5 selected NPO-IO bridge diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("packed Tip5 selected NPO-IO bridge diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("packed Tip5 selected NPO-IO bridge diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect(
                "packed Tip5 selected NPO-IO bridge diagnostic must use production query domains",
            );
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let columns_start = std::time::Instant::now();
        let columns = compiler
            .terminal_npo_polynomial_columns_goldilocks(&vk, &witness)
            .expect("packed Tip5 selected NPO-IO bridge diagnostic must build NPO columns");
        let columns_elapsed = columns_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: npo_columns_ms={} rows={} padded_rows={} columns={}",
            columns_elapsed.as_millis(),
            columns.layout.rows,
            1usize << columns.layout.log_rows,
            columns.layout.column_count,
        );

        let trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect("packed Tip5 selected NPO-IO bridge diagnostic must build packed lookup trace");
        let trace_elapsed = trace_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={}",
            trace_elapsed.as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
        );

        let root_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_selected_npo_io_logup_bridge_prelude_commitments_goldilocks(
                &vk,
                &columns,
                &packed_profile,
                &packed_trace,
            )
            .expect("packed Tip5 selected NPO-IO bridge diagnostic must commit bridge endpoints");
        let root_elapsed = root_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: bridge_endpoint_root_ms={}",
            root_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("packed Tip5 selected NPO-IO bridge diagnostic must build terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let bridge_proof = compiler
            .prove_terminal_npo_tip5_packed_lookup_selected_npo_io_logup_bridge_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("packed Tip5 selected NPO-IO bridge proof must build");
        let prove_elapsed = prove_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate phase [{label}]: prove_ms={}",
            prove_elapsed.as_millis()
        );

        let verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_selected_npo_io_logup_bridge_goldilocks::<Challenge>(
                &vk,
                &witness.public_inputs,
                &prelude,
                &bridge_proof,
            )
            .expect("packed Tip5 selected NPO-IO bridge proof must verify");
        let verify_elapsed = verify_start.elapsed();

        let proof_bytes = postcard_len(&bridge_proof, "packed Tip5 selected NPO-IO bridge proof");
        let compact_fri_bytes = postcard_len(
            &bridge_proof.proof, "packed Tip5 selected NPO-IO bridge compact FRI proof",
        );
        let opened_selected_lookup_bytes = postcard_len(
            &bridge_proof.opened_selected_lookup_basis,
            "packed Tip5 selected NPO-IO bridge opened selected lookup",
        );
        let opened_packed_npo_io_bytes = postcard_len(
            &bridge_proof.opened_packed_npo_io_basis,
            "packed Tip5 selected NPO-IO bridge opened packed NPO IO",
        );
        let opened_selected_accumulator_bytes = postcard_len(
            &bridge_proof.opened_selected_accumulator_points_basis,
            "packed Tip5 selected NPO-IO bridge opened selected accumulator",
        );
        let opened_packed_accumulator_bytes = postcard_len(
            &bridge_proof.opened_packed_accumulator_points_basis,
            "packed Tip5 selected NPO-IO bridge opened packed accumulator",
        );
        let opened_selected_quotient_bytes = postcard_len(
            &bridge_proof.opened_selected_quotient_basis,
            "packed Tip5 selected NPO-IO bridge opened selected quotient",
        );
        let opened_packed_quotient_bytes = postcard_len(
            &bridge_proof.opened_packed_quotient_basis,
            "packed Tip5 selected NPO-IO bridge opened packed quotient",
        );
        eprintln!(
            "native terminal packed Tip5 selected NPO-IO bridge candidate over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_selected_lookup={} opened_packed_npo_io={} opened_selected_accumulator={} opened_packed_accumulator={} opened_selected_quotient={} opened_packed_quotient={} selected_rows={} selected_padded_rows={} packed_rows={} packed_padded_rows={} packed_width={} packed_npo_io_columns={} selected_accumulator_columns={} packed_accumulator_columns={} selected_quotient_rows={} packed_quotient_rows={} l1_verify_ms={} compile_ms={} npo_columns_ms={} packed_trace_ms={} bridge_endpoint_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_selected_lookup_bytes,
            opened_packed_npo_io_bytes,
            opened_selected_accumulator_bytes,
            opened_packed_accumulator_bytes,
            opened_selected_quotient_bytes,
            opened_packed_quotient_bytes,
            bridge_proof.selected_profile.rows,
            bridge_proof.selected_profile.padded_rows,
            bridge_proof.packed_npo_io_profile.rows,
            bridge_proof.packed_npo_io_profile.padded_rows,
            packed_profile.main_width,
            bridge_proof.packed_npo_io_profile.basis_columns,
            bridge_proof.selected_accumulator_profile.basis_columns,
            bridge_proof.packed_accumulator_profile.basis_columns,
            bridge_proof.selected_quotient_profile.padded_rows,
            bridge_proof.packed_quotient_profile.padded_rows,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            columns_elapsed.as_millis(),
            trace_elapsed.as_millis(),
            root_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite packed Tip5 selected NPO-IO bridge terminal candidate measurement is opt-in"]
    fn terminal_packed_tip5_selected_npo_io_bridge_candidate_for_prod_baseline_measures() {
        measure_terminal_packed_tip5_selected_npo_io_bridge_candidate_for_profile(
            "PROD",
            CircuitConfig::PROD,
        );
    }

    fn measure_terminal_packed_tip5_selected_npo_trace_bridge_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("packed Tip5 selected NPO-trace bridge diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("packed Tip5 selected NPO-trace bridge diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&built.circuit).expect(
            "packed Tip5 selected NPO-trace bridge diagnostic must compile terminal circuit",
        );
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect(
                "packed Tip5 selected NPO-trace bridge diagnostic must use production query domains",
            );
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let columns_start = std::time::Instant::now();
        let columns = compiler
            .terminal_npo_polynomial_columns_goldilocks(&vk, &witness)
            .expect("packed Tip5 selected NPO-trace bridge diagnostic must build NPO columns");
        let columns_elapsed = columns_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: npo_columns_ms={} rows={} padded_rows={} columns={}",
            columns_elapsed.as_millis(),
            columns.layout.rows,
            1usize << columns.layout.log_rows,
            columns.layout.column_count,
        );

        let trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect(
                "packed Tip5 selected NPO-trace bridge diagnostic must build packed lookup trace",
            );
        let trace_elapsed = trace_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={}",
            trace_elapsed.as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
        );

        let root_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_selected_npo_trace_logup_bridge_prelude_commitments_goldilocks(
                &vk,
                &columns,
                &packed_profile,
                &packed_trace,
            )
            .expect("packed Tip5 selected NPO-trace bridge diagnostic must commit bridge endpoints");
        let root_elapsed = root_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: trace_bridge_endpoint_root_ms={}",
            root_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("packed Tip5 selected NPO-trace bridge diagnostic must build terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let bridge_proof = compiler
            .prove_terminal_npo_tip5_packed_lookup_selected_npo_trace_logup_bridge_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("packed Tip5 selected NPO-trace bridge proof must build");
        let prove_elapsed = prove_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate phase [{label}]: prove_ms={}",
            prove_elapsed.as_millis()
        );

        let verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_selected_npo_trace_logup_bridge_goldilocks::<Challenge>(
                &vk,
                &witness.public_inputs,
                &prelude,
                &bridge_proof,
            )
            .expect("packed Tip5 selected NPO-trace bridge proof must verify");
        let verify_elapsed = verify_start.elapsed();

        let proof_bytes =
            postcard_len(&bridge_proof, "packed Tip5 selected NPO-trace bridge proof");
        let compact_fri_bytes = postcard_len(
            &bridge_proof.proof, "packed Tip5 selected NPO-trace bridge compact FRI proof",
        );
        let opened_selected_lookup_bytes = postcard_len(
            &bridge_proof.opened_selected_lookup_basis,
            "packed Tip5 selected NPO-trace bridge opened selected lookup",
        );
        let opened_packed_trace_bytes = postcard_len(
            &bridge_proof.opened_packed_trace_basis,
            "packed Tip5 selected NPO-trace bridge opened packed trace",
        );
        let opened_selected_accumulator_bytes = postcard_len(
            &bridge_proof.opened_selected_accumulator_points_basis,
            "packed Tip5 selected NPO-trace bridge opened selected accumulator",
        );
        let opened_packed_accumulator_bytes = postcard_len(
            &bridge_proof.opened_packed_accumulator_points_basis,
            "packed Tip5 selected NPO-trace bridge opened packed accumulator",
        );
        let opened_selected_quotient_bytes = postcard_len(
            &bridge_proof.opened_selected_quotient_basis,
            "packed Tip5 selected NPO-trace bridge opened selected quotient",
        );
        let opened_packed_quotient_bytes = postcard_len(
            &bridge_proof.opened_packed_quotient_basis,
            "packed Tip5 selected NPO-trace bridge opened packed quotient",
        );
        eprintln!(
            "native terminal packed Tip5 selected NPO-trace bridge candidate over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_selected_lookup={} opened_packed_trace={} opened_selected_accumulator={} opened_packed_accumulator={} opened_selected_quotient={} opened_packed_quotient={} selected_rows={} selected_padded_rows={} packed_rows={} packed_padded_rows={} packed_width={} selected_accumulator_columns={} packed_accumulator_columns={} selected_quotient_rows={} packed_quotient_rows={} l1_verify_ms={} compile_ms={} npo_columns_ms={} packed_trace_ms={} trace_bridge_endpoint_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_selected_lookup_bytes,
            opened_packed_trace_bytes,
            opened_selected_accumulator_bytes,
            opened_packed_accumulator_bytes,
            opened_selected_quotient_bytes,
            opened_packed_quotient_bytes,
            bridge_proof.selected_profile.rows,
            bridge_proof.selected_profile.padded_rows,
            bridge_proof.packed_trace_profile.rows,
            bridge_proof.packed_trace_profile.padded_rows,
            bridge_proof.packed_trace_profile.main_width,
            bridge_proof.selected_accumulator_profile.basis_columns,
            bridge_proof.packed_accumulator_profile.basis_columns,
            bridge_proof.selected_quotient_profile.padded_rows,
            bridge_proof.packed_quotient_profile.padded_rows,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            columns_elapsed.as_millis(),
            trace_elapsed.as_millis(),
            root_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite packed Tip5 selected NPO-trace bridge terminal candidate measurement is opt-in"]
    fn terminal_packed_tip5_selected_npo_trace_bridge_candidate_for_prod_baseline_measures() {
        measure_terminal_packed_tip5_selected_npo_trace_bridge_candidate_for_profile(
            "PROD",
            CircuitConfig::PROD,
        );
    }

    fn measure_terminal_packed_tip5_projection_selected_bridge_fused_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect(
            "packed projection+selected bridge fused diagnostic must build L1 verifier circuit",
        );
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof).expect(
            "packed projection+selected bridge fused diagnostic must run L1 verifier traces",
        );
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&built.circuit).expect(
            "packed projection+selected bridge fused diagnostic must compile terminal circuit",
        );
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect(
                "packed projection+selected bridge fused diagnostic must use production query domains",
            );
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let columns_start = std::time::Instant::now();
        let columns = compiler
            .terminal_npo_polynomial_columns_goldilocks(&vk, &witness)
            .expect("packed projection+selected bridge fused diagnostic must build NPO columns");
        let columns_elapsed = columns_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: npo_columns_ms={} rows={} padded_rows={} columns={}",
            columns_elapsed.as_millis(),
            columns.layout.rows,
            1usize << columns.layout.log_rows,
            columns.layout.column_count,
        );

        let trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect(
                "packed projection+selected bridge fused diagnostic must build packed lookup trace",
            );
        let trace_elapsed = trace_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={}",
            trace_elapsed.as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
        );

        let root_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_npo_io_projection_selected_bridge_prelude_commitments_goldilocks(
                &vk,
                &columns,
                &packed_profile,
                &packed_trace,
            )
            .expect("packed projection+selected bridge fused diagnostic must commit endpoints");
        let root_elapsed = root_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: fused_endpoint_root_ms={}",
            root_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect(
                "packed projection+selected bridge fused diagnostic must build terminal prelude",
            );
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let fused_proof = compiler
            .prove_terminal_npo_tip5_packed_lookup_npo_io_projection_selected_bridge_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("packed projection+selected bridge fused proof must build");
        let prove_elapsed = prove_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate phase [{label}]: prove_ms={}",
            prove_elapsed.as_millis()
        );

        let verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_npo_io_projection_selected_bridge_goldilocks::<Challenge>(
                &vk,
                &witness.public_inputs,
                &prelude,
                &fused_proof,
            )
            .expect("packed projection+selected bridge fused proof must verify");
        let verify_elapsed = verify_start.elapsed();

        let proof_bytes = postcard_len(
            &fused_proof, "packed projection+selected bridge fused proof",
        );
        let compact_fri_bytes = postcard_len(
            &fused_proof.proof, "packed projection+selected bridge fused compact FRI proof",
        );
        let opened_selected_lookup_bytes = postcard_len(
            &fused_proof.opened_selected_lookup_basis,
            "packed projection+selected bridge fused opened selected lookup",
        );
        let opened_packed_trace_bytes = postcard_len(
            &fused_proof.opened_packed_trace_basis,
            "packed projection+selected bridge fused opened packed trace",
        );
        let opened_packed_npo_io_bytes = postcard_len(
            &fused_proof.opened_packed_npo_io_basis,
            "packed projection+selected bridge fused opened packed NPO IO",
        );
        let opened_projection_quotient_bytes = postcard_len(
            &fused_proof.opened_projection_quotient_basis,
            "packed projection+selected bridge fused opened projection quotient",
        );
        let opened_selected_accumulator_bytes = postcard_len(
            &fused_proof.opened_selected_accumulator_points_basis,
            "packed projection+selected bridge fused opened selected accumulator",
        );
        let opened_packed_accumulator_bytes = postcard_len(
            &fused_proof.opened_packed_accumulator_points_basis,
            "packed projection+selected bridge fused opened packed accumulator",
        );
        let opened_selected_quotient_bytes = postcard_len(
            &fused_proof.opened_selected_bridge_quotient_basis,
            "packed projection+selected bridge fused opened selected quotient",
        );
        let opened_packed_quotient_bytes = postcard_len(
            &fused_proof.opened_packed_bridge_quotient_basis,
            "packed projection+selected bridge fused opened packed quotient",
        );
        eprintln!(
            "native terminal packed Tip5 projection+selected bridge fused candidate over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_selected_lookup={} opened_packed_trace={} opened_packed_npo_io={} opened_projection_quotient={} opened_selected_accumulator={} opened_packed_accumulator={} opened_selected_quotient={} opened_packed_quotient={} selected_rows={} selected_padded_rows={} packed_rows={} packed_padded_rows={} packed_width={} packed_npo_io_columns={} projection_quotient_rows={} selected_bridge_quotient_rows={} packed_bridge_quotient_rows={} l1_verify_ms={} compile_ms={} npo_columns_ms={} packed_trace_ms={} fused_endpoint_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_selected_lookup_bytes,
            opened_packed_trace_bytes,
            opened_packed_npo_io_bytes,
            opened_projection_quotient_bytes,
            opened_selected_accumulator_bytes,
            opened_packed_accumulator_bytes,
            opened_selected_quotient_bytes,
            opened_packed_quotient_bytes,
            fused_proof.selected_profile.rows,
            fused_proof.selected_profile.padded_rows,
            fused_proof.packed_trace_profile.rows,
            fused_proof.packed_trace_profile.padded_rows,
            fused_proof.packed_trace_profile.main_width,
            fused_proof.packed_npo_io_profile.basis_columns,
            fused_proof.projection_quotient_profile.padded_rows,
            fused_proof.selected_bridge_quotient_profile.padded_rows,
            fused_proof.packed_bridge_quotient_profile.padded_rows,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            columns_elapsed.as_millis(),
            trace_elapsed.as_millis(),
            root_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite packed Tip5 projection+selected bridge fused measurement is opt-in"]
    fn terminal_packed_tip5_projection_selected_bridge_fused_candidate_for_prod_baseline_measures()
    {
        measure_terminal_packed_tip5_projection_selected_bridge_fused_candidate_for_profile(
            "PROD",
            CircuitConfig::PROD,
        );
    }

    fn measure_terminal_packed_tip5_air_logup_selected_trace_bridge_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("packed AIR+LogUp selected trace bridge diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof).expect(
            "packed AIR+LogUp selected trace bridge diagnostic must run L1 verifier traces",
        );
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&built.circuit).expect(
            "packed AIR+LogUp selected trace bridge diagnostic must compile terminal circuit",
        );
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect(
                "packed AIR+LogUp selected trace bridge diagnostic must use production query domains",
            );
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let columns_start = std::time::Instant::now();
        let columns = compiler
            .terminal_npo_polynomial_columns_goldilocks(&vk, &witness)
            .expect("packed AIR+LogUp selected trace bridge diagnostic must build NPO columns");
        let columns_elapsed = columns_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: npo_columns_ms={} rows={} padded_rows={} columns={}",
            columns_elapsed.as_millis(),
            columns.layout.rows,
            1usize << columns.layout.log_rows,
            columns.layout.column_count,
        );

        let trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect(
                "packed AIR+LogUp selected trace bridge diagnostic must build packed lookup trace",
            );
        let trace_elapsed = trace_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={} logup_tuples={}",
            trace_elapsed.as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
            packed_profile.logup_query_tuples,
        );

        let root_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_air_logup_selected_trace_bridge_prelude_commitments_goldilocks(
                &vk,
                &columns,
                &packed_profile,
                &packed_trace,
            )
            .expect("packed AIR+LogUp selected trace bridge diagnostic must commit endpoints");
        let root_elapsed = root_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: shared_endpoint_root_ms={}",
            root_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect(
                "packed AIR+LogUp selected trace bridge diagnostic must build terminal prelude",
            );
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let shared_proof = compiler
            .prove_terminal_npo_tip5_packed_lookup_air_logup_selected_trace_bridge_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("packed AIR+LogUp selected trace bridge proof must build");
        let prove_elapsed = prove_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 AIR+LogUp selected trace bridge candidate phase [{label}]: prove_ms={}",
            prove_elapsed.as_millis()
        );

        let verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_air_logup_selected_trace_bridge_goldilocks::<Challenge>(
                &vk,
                &witness.public_inputs,
                &prelude,
                &shared_proof,
            )
            .expect("packed AIR+LogUp selected trace bridge proof must verify");
        let verify_elapsed = verify_start.elapsed();

        let proof_bytes = postcard_len(
            &shared_proof, "packed AIR+LogUp selected trace bridge proof",
        );
        let compact_fri_bytes = postcard_len(
            &shared_proof.proof, "packed AIR+LogUp selected trace bridge compact FRI proof",
        );
        let opened_selected_lookup_bytes = postcard_len(
            &shared_proof.opened_selected_lookup_basis,
            "packed AIR+LogUp selected trace bridge opened selected lookup",
        );
        let opened_packed_trace_bytes = postcard_len(
            &shared_proof.opened_packed_trace_basis,
            "packed AIR+LogUp selected trace bridge opened packed trace",
        );
        let opened_air_quotient_bytes = postcard_len(
            &shared_proof.opened_air_quotient_basis,
            "packed AIR+LogUp selected trace bridge opened AIR quotient",
        );
        let opened_logup_table_bytes = postcard_len(
            &shared_proof.opened_logup_table_basis,
            "packed AIR+LogUp selected trace bridge opened LogUp table",
        );
        let opened_logup_accumulator_bytes = postcard_len(
            &shared_proof.opened_logup_accumulator_points_basis,
            "packed AIR+LogUp selected trace bridge opened LogUp accumulator",
        );
        let opened_logup_quotient_bytes = postcard_len(
            &shared_proof.opened_logup_quotient_basis,
            "packed AIR+LogUp selected trace bridge opened LogUp quotient",
        );
        let opened_selected_bridge_accumulator_bytes = postcard_len(
            &shared_proof.opened_selected_bridge_accumulator_points_basis,
            "packed AIR+LogUp selected trace bridge opened selected bridge accumulator",
        );
        let opened_packed_bridge_accumulator_bytes = postcard_len(
            &shared_proof.opened_packed_bridge_accumulator_points_basis,
            "packed AIR+LogUp selected trace bridge opened packed bridge accumulator",
        );
        let opened_selected_bridge_quotient_bytes = postcard_len(
            &shared_proof.opened_selected_bridge_quotient_basis,
            "packed AIR+LogUp selected trace bridge opened selected bridge quotient",
        );
        let opened_packed_bridge_quotient_bytes = postcard_len(
            &shared_proof.opened_packed_bridge_quotient_basis,
            "packed AIR+LogUp selected trace bridge opened packed bridge quotient",
        );
        eprintln!(
            "native terminal coalesced packed Tip5 AIR+LogUp selected trace bridge candidate over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_selected_lookup={} opened_packed_trace={} opened_air_quotient={} opened_logup_table={} opened_logup_accumulator={} opened_logup_quotient={} opened_selected_bridge_accumulator={} opened_packed_bridge_accumulator={} opened_selected_bridge_quotient={} opened_packed_bridge_quotient={} selected_rows={} selected_padded_rows={} packed_rows={} packed_padded_rows={} packed_width={} logup_tuples={} air_quotient_rows={} logup_table_columns={} logup_accumulator_columns={} logup_quotient_rows={} selected_bridge_accumulator_columns={} packed_bridge_accumulator_columns={} selected_bridge_quotient_rows={} packed_bridge_quotient_rows={} l1_verify_ms={} compile_ms={} npo_columns_ms={} packed_trace_ms={} shared_endpoint_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_selected_lookup_bytes,
            opened_packed_trace_bytes,
            opened_air_quotient_bytes,
            opened_logup_table_bytes,
            opened_logup_accumulator_bytes,
            opened_logup_quotient_bytes,
            opened_selected_bridge_accumulator_bytes,
            opened_packed_bridge_accumulator_bytes,
            opened_selected_bridge_quotient_bytes,
            opened_packed_bridge_quotient_bytes,
            shared_proof.selected_profile.rows,
            shared_proof.selected_profile.padded_rows,
            shared_proof.packed_trace_profile.rows,
            shared_proof.packed_trace_profile.padded_rows,
            shared_proof.packed_trace_profile.main_width,
            shared_proof.packed_trace_profile.logup_query_tuples,
            shared_proof.air_quotient_profile.padded_rows,
            shared_proof.logup_table_profile.basis_columns,
            shared_proof.logup_accumulator_profile.basis_columns,
            shared_proof.logup_quotient_profile.padded_rows,
            shared_proof.selected_bridge_accumulator_profile.basis_columns,
            shared_proof.packed_bridge_accumulator_profile.basis_columns,
            shared_proof.selected_bridge_quotient_profile.padded_rows,
            shared_proof.packed_bridge_quotient_profile.padded_rows,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            columns_elapsed.as_millis(),
            trace_elapsed.as_millis(),
            root_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite packed Tip5 AIR+LogUp selected trace bridge measurement is opt-in"]
    fn terminal_packed_tip5_air_logup_selected_trace_bridge_candidate_for_prod_baseline_measures() {
        measure_terminal_packed_tip5_air_logup_selected_trace_bridge_candidate_for_profile(
            "PROD",
            CircuitConfig::PROD,
        );
    }

    fn measure_terminal_merged_value_bridge_packed_support_fusion_floor_for_profile(
        label: &str,
        profile: CircuitConfig,
        parallel_subproofs: bool,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("fusion floor diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("fusion floor diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("fusion floor diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("fusion floor diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let assignment_commit_start = std::time::Instant::now();
        let assignment_oracle = compiler
            .commit_terminal_assignment_goldilocks(&vk, &witness.public_inputs, &witness)
            .expect("fusion floor diagnostic must commit terminal assignment");
        let assignment_commitment = assignment_oracle.commitment();
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: assignment_commit_ms={}",
            assignment_commit_start.elapsed().as_millis()
        );

        let merged_root_start = std::time::Instant::now();
        let merged_value_bridge_prepared = compiler
            .prepare_terminal_npo_fri_residual_zero_recompose_value_bridge_goldilocks(&vk, &witness)
            .expect("fusion floor diagnostic must prepare merged NPO value bridge");
        let merged_roots = merged_value_bridge_prepared.prelude_commitments().to_vec();
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: merged_npo_root_ms={}",
            merged_root_start.elapsed().as_millis()
        );

        let packed_trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect("fusion floor diagnostic must build packed Tip5 trace");
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={} logup_tuples={}",
            packed_trace_start.elapsed().as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
            packed_profile.logup_query_tuples,
        );

        let packed_support_root_start = std::time::Instant::now();
        let packed_support_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_air_logup_selected_trace_bridge_prelude_commitments_goldilocks(
                &vk,
                &merged_value_bridge_prepared.columns,
                &packed_profile,
                &packed_trace,
            )
            .expect("fusion floor diagnostic must commit packed support endpoints");
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: packed_support_root_ms={}",
            packed_support_root_start.elapsed().as_millis()
        );
        assert_eq!(
            merged_roots.first(),
            packed_support_roots.first(),
            "merged value bridge and packed support theorem must share the selected lookup root"
        );

        let mut prelude_roots = vec![assignment_commitment.root];
        prelude_roots.extend(merged_roots);
        for root in packed_support_roots {
            if !prelude_roots.contains(&root) {
                prelude_roots.push(root);
            }
        }

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("fusion floor diagnostic must build shared terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let (
            primitive_r1cs_proof,
            primitive_prove_elapsed,
            merged_value_bridge_proof,
            merged_prove_elapsed,
            packed_support_proof,
            packed_support_prove_elapsed,
            parallel_subproof_prove_elapsed,
        ) = if parallel_subproofs {
            let parallel_prove_start = std::time::Instant::now();
            let (primitive, (merged, packed_support)) = rayon::join(
                || {
                    let prove_start = std::time::Instant::now();
                    let proof = compiler
                        .prove_terminal_r1cs_row_product_sumcheck_prelude_checked_goldilocks(
                            &vk, &witness.public_inputs, &prelude, &assignment_oracle, &witness,
                        )
                        .expect("fusion floor diagnostic must prove primitive R1CS");
                    (proof, prove_start.elapsed())
                },
                || {
                    rayon::join(
                        || {
                            let prove_start = std::time::Instant::now();
                            let proof = compiler
                                .prove_terminal_npo_polynomial_fri_residual_zero_recompose_value_bridge_prepared_prelude_checked_goldilocks(
                                    &prelude,
                                    &merged_value_bridge_prepared,
                                )
                                .expect(
                                    "fusion floor diagnostic must prove merged value bridge",
                                );
                            (proof, prove_start.elapsed())
                        },
                        || {
                            let prove_start = std::time::Instant::now();
                            let proof = compiler
                                .prove_terminal_npo_tip5_packed_lookup_air_logup_selected_trace_bridge_goldilocks(
                                    &vk,
                                    &witness.public_inputs,
                                    &witness,
                                    &prelude,
                                )
                                .expect(
                                    "fusion floor diagnostic must prove packed support theorem",
                                );
                            (proof, prove_start.elapsed())
                        },
                    )
                },
            );
            let parallel_elapsed = parallel_prove_start.elapsed();
            eprintln!(
                "native terminal merged value-bridge + packed support fusion floor phase [{label}]: parallel_subproof_prove_ms={} primitive_prove_ms={} merged_value_bridge_prove_ms={} packed_support_prove_ms={} serial_sum_subproof_prove_ms={}",
                parallel_elapsed.as_millis(),
                primitive.1.as_millis(),
                merged.1.as_millis(),
                packed_support.1.as_millis(),
                (primitive.1 + merged.1 + packed_support.1).as_millis(),
            );
            (
                primitive.0,
                primitive.1,
                merged.0,
                merged.1,
                packed_support.0,
                packed_support.1,
                Some(parallel_elapsed),
            )
        } else {
            let primitive_prove_start = std::time::Instant::now();
            let primitive_r1cs_proof = compiler
                .prove_terminal_r1cs_row_product_sumcheck_prelude_checked_goldilocks(
                    &vk, &witness.public_inputs, &prelude, &assignment_oracle, &witness,
                )
                .expect("fusion floor diagnostic must prove primitive R1CS");
            let primitive_prove_elapsed = primitive_prove_start.elapsed();
            eprintln!(
                "native terminal merged value-bridge + packed support fusion floor phase [{label}]: primitive_prove_ms={}",
                primitive_prove_elapsed.as_millis()
            );

            let merged_prove_start = std::time::Instant::now();
            let merged_value_bridge_proof = compiler
                .prove_terminal_npo_polynomial_fri_residual_zero_recompose_value_bridge_prepared_prelude_checked_goldilocks(
                    &prelude,
                    &merged_value_bridge_prepared,
                )
                .expect("fusion floor diagnostic must prove merged value bridge");
            let merged_prove_elapsed = merged_prove_start.elapsed();
            eprintln!(
                "native terminal merged value-bridge + packed support fusion floor phase [{label}]: merged_value_bridge_prove_ms={}",
                merged_prove_elapsed.as_millis()
            );

            let packed_support_prove_start = std::time::Instant::now();
            let packed_support_proof = compiler
                .prove_terminal_npo_tip5_packed_lookup_air_logup_selected_trace_bridge_goldilocks(
                    &vk, &witness.public_inputs, &witness, &prelude,
                )
                .expect("fusion floor diagnostic must prove packed support theorem");
            let packed_support_prove_elapsed = packed_support_prove_start.elapsed();
            eprintln!(
                "native terminal merged value-bridge + packed support fusion floor phase [{label}]: packed_support_prove_ms={}",
                packed_support_prove_elapsed.as_millis()
            );

            (
                primitive_r1cs_proof, primitive_prove_elapsed, merged_value_bridge_proof,
                merged_prove_elapsed, packed_support_proof, packed_support_prove_elapsed, None,
            )
        };

        let primitive_verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_r1cs_row_product_sumcheck_goldilocks(
                &vk, &witness.public_inputs, &prelude, &assignment_commitment,
                &primitive_r1cs_proof,
            )
            .expect("fusion floor diagnostic primitive proof must verify");
        let primitive_verify_elapsed = primitive_verify_start.elapsed();

        let merged_verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_polynomial_fri_residual_zero_recompose_value_bridge_goldilocks(
                &vk, &witness.public_inputs, &prelude, &merged_value_bridge_proof,
            )
            .expect("fusion floor diagnostic merged value bridge must verify");
        let merged_verify_elapsed = merged_verify_start.elapsed();

        let packed_support_verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_air_logup_selected_trace_bridge_goldilocks::<
                Challenge,
            >(&vk, &witness.public_inputs, &prelude, &packed_support_proof)
            .expect("fusion floor diagnostic packed support theorem must verify");
        let packed_support_verify_elapsed = packed_support_verify_start.elapsed();

        let current_append_body_bytes = postcard_len(
            &(
                &prelude, &primitive_r1cs_proof, &merged_value_bridge_proof, &packed_support_proof,
            ),
            "merged value-bridge + packed support appended body",
        );
        let prelude_bytes = postcard_len(&prelude, "fusion floor prelude");
        let primitive_bytes = postcard_len(&primitive_r1cs_proof, "fusion floor primitive R1CS");
        let merged_bytes = postcard_len(
            &merged_value_bridge_proof, "fusion floor merged value bridge proof",
        );
        let packed_support_bytes =
            postcard_len(&packed_support_proof, "fusion floor packed support proof");
        let merged_fri = terminal_compact_fri_byte_breakdown(
            &merged_value_bridge_proof.proof, "fusion floor merged value bridge compact FRI",
        );
        let packed_support_fri = terminal_compact_fri_byte_breakdown(
            &packed_support_proof.proof, "fusion floor packed support compact FRI",
        );
        log_terminal_compact_fri_byte_breakdown("merged_value_bridge", &merged_fri);
        log_terminal_compact_fri_byte_breakdown("packed_support", &packed_support_fri);

        assert_eq!(
            merged_value_bridge_proof.selected_profile, packed_support_proof.selected_profile,
            "merged and packed support proofs should describe the same selected NPO matrix"
        );
        assert_eq!(
            merged_value_bridge_proof.lookup_io_profile, packed_support_proof.lookup_io_profile,
            "merged and packed support proofs should describe the same selected lookup-IO suffix"
        );
        assert_eq!(
            merged_value_bridge_proof.selected_lookup_commitment,
            packed_support_proof.selected_lookup_commitment,
            "merged and packed support proofs should bind the same selected lookup commitment"
        );

        let duplicate_selected_profile_bytes = postcard_len(
            &merged_value_bridge_proof.selected_profile, "fusion floor duplicate selected profile",
        );
        let duplicate_lookup_io_profile_bytes = postcard_len(
            &merged_value_bridge_proof.lookup_io_profile,
            "fusion floor duplicate lookup-IO profile",
        );
        let duplicate_selected_commitment_bytes = postcard_len(
            &merged_value_bridge_proof.selected_lookup_commitment,
            "fusion floor duplicate selected lookup commitment",
        );
        let merged_selected_lookup_opening_bytes = postcard_len(
            &(
                &merged_value_bridge_proof.opened_selected_basis,
                &merged_value_bridge_proof.opened_lookup_io_basis,
            ),
            "fusion floor merged selected lookup opening",
        );
        let packed_support_selected_lookup_opening_bytes = postcard_len(
            &packed_support_proof.opened_selected_lookup_basis,
            "fusion floor packed support selected lookup opening",
        );
        let optimistic_selected_opening_dedup_bytes =
            merged_selected_lookup_opening_bytes.min(packed_support_selected_lookup_opening_bytes);
        let optimistic_duplicate_selected_binding_bytes = duplicate_selected_profile_bytes
            + duplicate_lookup_io_profile_bytes
            + duplicate_selected_commitment_bytes
            + optimistic_selected_opening_dedup_bytes;

        let merged_non_fri_bytes = merged_bytes.saturating_sub(merged_fri.total_bytes);
        let packed_support_non_fri_bytes =
            packed_support_bytes.saturating_sub(packed_support_fri.total_bytes);
        let single_fri_floor_bytes = merged_fri.total_bytes.max(packed_support_fri.total_bytes);
        let optimistic_single_fri_floor_before_selected_dedup = prelude_bytes
            + primitive_bytes
            + merged_non_fri_bytes
            + packed_support_non_fri_bytes
            + single_fri_floor_bytes;
        let optimistic_single_fri_floor_bytes = optimistic_single_fri_floor_before_selected_dedup
            .saturating_sub(optimistic_duplicate_selected_binding_bytes);
        let target_binary_150k = 150usize * 1024;
        let target_decimal_150k = 150_000usize;
        let paired_lookup_estimate = terminal_packed_tip5_paired_lookup_payload_estimate(
            &packed_support_proof, &packed_support_fri, optimistic_single_fri_floor_bytes,
        );
        log_terminal_packed_tip5_paired_lookup_payload_estimate(label, &paired_lookup_estimate);
        log_terminal_packed_support_structural_floor(
            label, prelude_bytes, primitive_bytes, merged_bytes, packed_support_non_fri_bytes,
            optimistic_duplicate_selected_binding_bytes, &paired_lookup_estimate,
        );

        eprintln!(
            "native terminal merged value-bridge + packed support fusion floor over ai-pow composite verifier [{label}]: current_appended_body={} prelude={} primitive_r1cs={} merged_value_bridge={} merged_value_bridge_fri={} merged_non_fri={} packed_support={} packed_support_fri={} packed_support_non_fri={} single_fri_floor={} duplicate_selected_profile={} duplicate_lookup_io_profile={} duplicate_selected_commitment={} merged_selected_lookup_opening={} packed_support_selected_lookup_opening={} optimistic_selected_opening_dedup={} optimistic_duplicate_selected_binding={} optimistic_single_fri_floor_before_selected_dedup={} optimistic_single_fri_floor={} over_binary_150k={} over_decimal_150k={} l1_verify_ms={} compile_ms={} prelude_ms={} primitive_prove_ms={} merged_value_bridge_prove_ms={} packed_support_prove_ms={} total_subproof_prove_ms={} parallel_subproof_prove_ms={} primitive_verify_ms={} merged_value_bridge_verify_ms={} packed_support_verify_ms={} total_subproof_verify_ms={} total_wall_ms={}",
            current_append_body_bytes,
            prelude_bytes,
            primitive_bytes,
            merged_bytes,
            merged_fri.total_bytes,
            merged_non_fri_bytes,
            packed_support_bytes,
            packed_support_fri.total_bytes,
            packed_support_non_fri_bytes,
            single_fri_floor_bytes,
            duplicate_selected_profile_bytes,
            duplicate_lookup_io_profile_bytes,
            duplicate_selected_commitment_bytes,
            merged_selected_lookup_opening_bytes,
            packed_support_selected_lookup_opening_bytes,
            optimistic_selected_opening_dedup_bytes,
            optimistic_duplicate_selected_binding_bytes,
            optimistic_single_fri_floor_before_selected_dedup,
            optimistic_single_fri_floor_bytes,
            optimistic_single_fri_floor_bytes.saturating_sub(target_binary_150k),
            optimistic_single_fri_floor_bytes.saturating_sub(target_decimal_150k),
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            primitive_prove_elapsed.as_millis(),
            merged_prove_elapsed.as_millis(),
            packed_support_prove_elapsed.as_millis(),
            (primitive_prove_elapsed + merged_prove_elapsed + packed_support_prove_elapsed)
                .as_millis(),
            parallel_subproof_prove_elapsed.map_or(0, |elapsed| elapsed.as_millis()),
            primitive_verify_elapsed.as_millis(),
            merged_verify_elapsed.as_millis(),
            packed_support_verify_elapsed.as_millis(),
            (primitive_verify_elapsed + merged_verify_elapsed + packed_support_verify_elapsed)
                .as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite merged value-bridge + packed support fusion floor measurement is opt-in"]
    fn terminal_merged_value_bridge_packed_support_fusion_floor_for_prod_baseline_measures() {
        measure_terminal_merged_value_bridge_packed_support_fusion_floor_for_profile(
            "PROD",
            CircuitConfig::PROD,
            false,
        );
    }

    #[test]
    #[ignore = "full composite merged value-bridge + packed support parallel subproof measurement is opt-in"]
    fn terminal_merged_value_bridge_packed_support_parallel_subproof_floor_for_prod_measures() {
        measure_terminal_merged_value_bridge_packed_support_fusion_floor_for_profile(
            "PROD-PARALLEL-SUBPROOFS",
            CircuitConfig::PROD,
            true,
        );
    }

    fn measure_terminal_packed_tip5_logup_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };

        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("packed Tip5 LogUp diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("packed Tip5 LogUp diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("packed Tip5 LogUp diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("packed Tip5 LogUp diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let trace_start = std::time::Instant::now();
        let (_, packed_profile, packed_trace) = compiler
            .terminal_npo_tip5_packed_lookup_trace_goldilocks(&vk, &witness)
            .expect("packed Tip5 LogUp diagnostic must build packed lookup trace");
        let trace_elapsed = trace_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: packed_trace_ms={} packed_rows={} packed_padded_rows={} packed_width={} logup_tuples={}",
            trace_elapsed.as_millis(),
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
            packed_profile.logup_query_tuples,
        );

        let root_start = std::time::Instant::now();
        let prelude_roots =
            NativeTerminalCompiler::terminal_npo_tip5_packed_lookup_fri_prelude_commitments_goldilocks(
                &packed_profile,
                &packed_trace,
            )
            .expect("packed Tip5 LogUp diagnostic must commit packed trace");
        let root_elapsed = root_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: packed_trace_root_ms={}",
            root_elapsed.as_millis()
        );

        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("packed Tip5 LogUp diagnostic must build terminal prelude");
        let prelude_elapsed = prelude_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: prelude_ms={}",
            prelude_elapsed.as_millis()
        );

        let prove_start = std::time::Instant::now();
        let packed_proof = compiler
            .prove_terminal_npo_tip5_packed_lookup_logup_quotient_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
            )
            .expect("packed Tip5 LogUp proof must build");
        let prove_elapsed = prove_start.elapsed();
        eprintln!(
            "native terminal packed Tip5 LogUp candidate phase [{label}]: prove_ms={}",
            prove_elapsed.as_millis()
        );

        let verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_packed_lookup_logup_quotient_goldilocks::<Challenge>(
                &vk, &witness.public_inputs, &prelude, &packed_proof,
            )
            .expect("packed Tip5 LogUp proof must verify");
        let verify_elapsed = verify_start.elapsed();

        let proof_bytes = postcard_len(&packed_proof, "packed Tip5 LogUp proof");
        let compact_fri_bytes =
            postcard_len(&packed_proof.proof, "packed Tip5 LogUp compact FRI proof");
        let opened_trace_bytes = postcard_len(
            &packed_proof.opened_trace_basis, "packed Tip5 LogUp opened trace",
        );
        let opened_table_bytes = postcard_len(
            &packed_proof.opened_table_basis, "packed Tip5 LogUp opened table",
        );
        let opened_accumulator_bytes = postcard_len(
            &packed_proof.opened_accumulator_points_basis, "packed Tip5 LogUp opened accumulator",
        );
        let opened_quotient_bytes = postcard_len(
            &packed_proof.opened_quotient_basis, "packed Tip5 LogUp opened quotient",
        );
        eprintln!(
            "native terminal packed Tip5 LogUp candidate over ai-pow composite verifier [{label}]: proof={} bytes compact_fri={} opened_trace={} opened_table={} opened_accumulator={} opened_quotient={} packed_rows={} packed_padded_rows={} packed_width={} logup_tuples={} table_columns={} accumulator_columns={} quotient_rows={} l1_verify_ms={} compile_ms={} packed_trace_ms={} packed_trace_root_ms={} prelude_ms={} prove_ms={} verify_ms={} total_wall_ms={}",
            proof_bytes,
            compact_fri_bytes,
            opened_trace_bytes,
            opened_table_bytes,
            opened_accumulator_bytes,
            opened_quotient_bytes,
            packed_profile.rows,
            packed_profile.padded_rows,
            packed_profile.main_width,
            packed_profile.logup_query_tuples,
            packed_proof.table_profile.basis_columns,
            packed_proof.accumulator_profile.basis_columns,
            packed_proof.quotient_profile.padded_rows,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            trace_elapsed.as_millis(),
            root_elapsed.as_millis(),
            prelude_elapsed.as_millis(),
            prove_elapsed.as_millis(),
            verify_elapsed.as_millis(),
            total_start.elapsed().as_millis(),
        );
    }

    #[test]
    #[ignore = "full composite packed Tip5 LogUp terminal candidate measurement is opt-in"]
    fn terminal_packed_tip5_logup_candidate_for_prod_baseline_measures() {
        measure_terminal_packed_tip5_logup_candidate_for_profile("PROD", CircuitConfig::PROD);
    }

    fn measure_terminal_integrated_logup_candidate_for_profile(
        label: &str,
        profile: CircuitConfig,
    ) {
        init_terminal_prover_profile_tracing();
        assert_eq!(profile.johnson_fri_bits(), 60);

        let total_start = std::time::Instant::now();
        let zk = test_zk_params();
        let cfg = build_config(&zk, &profile);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let l0_prove_start = std::time::Instant::now();
        let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: l0_prove_ms={}",
            l0_prove_start.elapsed().as_millis()
        );
        let verified = unsafe {
            ChainVerifiedCompositeProof::from_parts_after_chain_statement_verification(
                program.clone(),
                proof,
                &pis,
            )
        };
        let l1_build_start = std::time::Instant::now();
        let air = CompositeFullAirWithLookupsPinned::new_with(program, true);
        let pd = logup_common_for(&cfg, &verified.program, true);
        let built = build_composite_l1_verifier_circuit(
            &cfg,
            &air,
            &verified.proof,
            &pd.common,
            &verified.public_inputs.to_vec(),
            &profile,
        )
        .expect("integrated LogUp diagnostic must build L1 verifier circuit");
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: l1_circuit_build_ms={}",
            l1_build_start.elapsed().as_millis()
        );

        let l1_verify_start = std::time::Instant::now();
        let traces = run_composite_l1_verifier_traces(&built, &verified.proof)
            .expect("integrated LogUp diagnostic must run L1 verifier traces");
        let l1_verify_elapsed = l1_verify_start.elapsed();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: l1_trace_verify_ms={}",
            l1_verify_elapsed.as_millis()
        );

        let compiler = terminal_compiler();
        let compile_start = std::time::Instant::now();
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&built.circuit)
            .expect("integrated LogUp diagnostic must compile terminal circuit");
        compiler
            .validate_goldilocks_production_query_domains(
                &vk,
                TerminalProofParameters::production_60bit(),
            )
            .expect("integrated LogUp diagnostic must use production query domains");
        let compile_elapsed = compile_start.elapsed();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: terminal_compile_ms={}",
            compile_elapsed.as_millis()
        );

        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&built.circuit),
            public_inputs: built.public_inputs.clone(),
            private_inputs: built.private_inputs.clone(),
            traces,
        };

        let assignment_commit_start = std::time::Instant::now();
        let assignment_oracle = compiler
            .commit_terminal_assignment_goldilocks(&vk, &witness.public_inputs, &witness)
            .expect("integrated LogUp diagnostic must commit terminal assignment");
        let assignment_commitment = assignment_oracle.commitment();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: assignment_commit_ms={}",
            assignment_commit_start.elapsed().as_millis()
        );
        let mut prelude_roots = vec![assignment_commitment.root];
        let merged_root_start = std::time::Instant::now();
        let merged_value_bridge_prepared = compiler
            .prepare_terminal_npo_fri_residual_zero_recompose_value_bridge_goldilocks(&vk, &witness)
            .expect("integrated LogUp diagnostic must prepare merged NPO proof data");
        prelude_roots.extend(merged_value_bridge_prepared.prelude_commitments().to_vec());
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: merged_npo_root_ms={}",
            merged_root_start.elapsed().as_millis()
        );
        let bundled_tip5_root_start = std::time::Instant::now();
        let integrated_logup_prepared =
            NativeTerminalCompiler::prepare_terminal_npo_tip5_lookup_air_logup_trace_io_support_npo_io_logup_goldilocks(
                &merged_value_bridge_prepared,
            )
            .expect("integrated LogUp diagnostic must prepare bundled Tip5 proof data");
        prelude_roots.extend(integrated_logup_prepared.prelude_commitments().to_vec());
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: bundled_tip5_root_ms={}",
            bundled_tip5_root_start.elapsed().as_millis()
        );
        let prelude_start = std::time::Instant::now();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &witness.public_inputs,
                TerminalProofParameters::production_60bit(),
                prelude_roots,
            )
            .expect("integrated LogUp diagnostic must build terminal prelude");
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: prelude_ms={}",
            prelude_start.elapsed().as_millis()
        );

        let primitive_prove_start = std::time::Instant::now();
        let primitive_r1cs_proof = compiler
            .prove_terminal_r1cs_row_product_sumcheck_goldilocks(
                &vk, &witness.public_inputs, &prelude, &assignment_oracle, &witness,
            )
            .expect("integrated LogUp diagnostic must prove primitive R1CS");
        let primitive_prove_elapsed = primitive_prove_start.elapsed();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: primitive_prove_ms={}",
            primitive_prove_elapsed.as_millis()
        );

        let npo_prove_start = std::time::Instant::now();
        let merged_prove_start = std::time::Instant::now();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: merged_value_bridge_prove_start"
        );
        let merged_value_bridge_proof = compiler
            .prove_terminal_npo_polynomial_fri_residual_zero_recompose_value_bridge_prepared_prelude_checked_goldilocks(
                &prelude,
                &merged_value_bridge_prepared,
            )
            .expect("integrated LogUp diagnostic must prove merged NPO value bridge");
        let merged_prove_elapsed = merged_prove_start.elapsed();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: merged_value_bridge_prove_ms={}",
            merged_prove_elapsed.as_millis()
        );
        let integrated_logup_prove_start = std::time::Instant::now();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: integrated_logup_prove_start"
        );
        let integrated_logup_proof = compiler
            .prove_terminal_npo_tip5_lookup_air_logup_trace_io_support_npo_io_logup_trace_bundle_prepared_prelude_checked_goldilocks(
                &prelude,
                &merged_value_bridge_prepared,
                &integrated_logup_prepared,
            )
            .expect("integrated LogUp diagnostic must prove bundled Tip5 LogUp");
        let integrated_logup_prove_elapsed = integrated_logup_prove_start.elapsed();
        let npo_prove_elapsed = npo_prove_start.elapsed();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: integrated_logup_prove_ms={} npo_prove_ms={}",
            integrated_logup_prove_elapsed.as_millis(),
            npo_prove_elapsed.as_millis()
        );

        let primitive_verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_r1cs_row_product_sumcheck_goldilocks(
                &vk, &witness.public_inputs, &prelude, &assignment_commitment,
                &primitive_r1cs_proof,
            )
            .expect("integrated LogUp diagnostic primitive proof must verify");
        let primitive_verify_elapsed = primitive_verify_start.elapsed();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: primitive_verify_ms={}",
            primitive_verify_elapsed.as_millis()
        );
        let npo_verify_start = std::time::Instant::now();
        compiler
            .verify_terminal_npo_tip5_lookup_backend_trace_value_integrated_logup_bridge_goldilocks(
                &vk, &witness.public_inputs, &prelude, &merged_value_bridge_proof,
                &integrated_logup_proof,
            )
            .expect("integrated LogUp diagnostic NPO proof must verify");
        let npo_verify_elapsed = npo_verify_start.elapsed();
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: npo_verify_ms={}",
            npo_verify_elapsed.as_millis()
        );

        let npo_polynomial_proof = TerminalProductionNpoPolynomialProof {
            merged_value_bridge_proof,
            integrated_logup_proof,
        };
        let candidate_body = (
            prelude.clone(),
            primitive_r1cs_proof.clone(),
            npo_polynomial_proof.clone(),
        );
        let body_bytes = postcard_len(&candidate_body, "integrated LogUp candidate body");
        let prelude_bytes = postcard_len(&prelude, "integrated LogUp candidate prelude");
        let primitive_bytes = postcard_len(
            &primitive_r1cs_proof, "integrated LogUp candidate primitive proof",
        );
        let npo_bytes = postcard_len(
            &npo_polynomial_proof, "integrated LogUp candidate NPO proof",
        );
        let merged_bytes = postcard_len(
            &npo_polynomial_proof.merged_value_bridge_proof,
            "integrated LogUp candidate merged NPO proof",
        );
        let integrated_logup_bytes = postcard_len(
            &npo_polynomial_proof.integrated_logup_proof,
            "integrated LogUp candidate Tip5 LogUp proof",
        );
        let integrated_logup_fri_bytes = postcard_len(
            &npo_polynomial_proof.integrated_logup_proof.proof,
            "integrated LogUp candidate Tip5 compact FRI",
        );
        let total_prove_elapsed = primitive_prove_elapsed + npo_prove_elapsed;
        let total_verify_elapsed = primitive_verify_elapsed + npo_verify_elapsed;

        eprintln!(
            "native terminal integrated-LogUp production candidate over ai-pow composite verifier [{label}]: body={} bytes prelude={} primitive_r1cs={} npo_polynomial={} merged_value_bridge={} integrated_logup={} integrated_logup_fri={} l1_verify_ms={} compile_ms={} primitive_prove_ms={} npo_prove_ms={} total_prove_ms={} primitive_verify_ms={} npo_verify_ms={} total_verify_ms={}",
            body_bytes,
            prelude_bytes,
            primitive_bytes,
            npo_bytes,
            merged_bytes,
            integrated_logup_bytes,
            integrated_logup_fri_bytes,
            l1_verify_elapsed.as_millis(),
            compile_elapsed.as_millis(),
            primitive_prove_elapsed.as_millis(),
            npo_prove_elapsed.as_millis(),
            total_prove_elapsed.as_millis(),
            primitive_verify_elapsed.as_millis(),
            npo_verify_elapsed.as_millis(),
            total_verify_elapsed.as_millis(),
        );
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: total_wall_ms={}",
            total_start.elapsed().as_millis()
        );
    }

    #[test]
    #[ignore = "full composite integrated LogUp terminal candidate measurement is opt-in"]
    fn terminal_integrated_logup_candidate_for_pure_query_lb6_nq10_measures() {
        measure_terminal_integrated_logup_candidate_for_profile(
            "PURE_QUERY_LB6_NQ10",
            CircuitConfig {
                log_blowup: 6,
                pow_bits: 0,
                num_queries: 10,
            },
        );
    }

    #[test]
    #[ignore = "native terminal proof pure-query profile sweep is opt-in"]
    fn terminal_recursive_certificate_for_pure_query_lb5_nq12_measures() {
        measure_and_verify_terminal_certificate_for_profile(
            "PURE_QUERY_LB5_NQ12",
            CircuitConfig {
                log_blowup: 5,
                pow_bits: 0,
                num_queries: 12,
            },
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
    fn recursive_certificate_rejects_outer_preprocessed_binding_tamper() {
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

        cert.l1_outer_proof.stark_common = CommonData::new(None, Vec::new());
        let err = verify_recursive_certificate(&cert, &zk, &profile, &pis)
            .expect_err("recursive verifier must reject non-canonical preprocessed binding");
        assert!(
            err.to_string().contains("preprocessed commitment"),
            "unexpected verifier error: {err}"
        );
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
