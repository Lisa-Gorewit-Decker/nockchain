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

/// Measure the native terminal relation generated for the actual composite L1
/// verifier circuit without running terminal proving.
pub fn measure_composite_l1_terminal_relation(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    verified: &ChainVerifiedCompositeProof<'_>,
) -> Result<CompositeTerminalRelationMetrics, VerificationError> {
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
        NativeTerminalConstraint, NativeTerminalVerifyingKey, TerminalProductionNpoPolynomialProof,
        TerminalProductionProof,
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

    fn prove_l2_over_l1_outer_for_test_pearl(
        l1: &AiPowL1OuterProof,
        statement_digest_public_values: &[Val],
        l1_config: p3_circuit_prover::config::GoldilocksTipsConfig,
        l1_fri_verifier_params: &FriVerifierParams,
        l2_config: p3_circuit_prover::config::GoldilocksTipsConfig,
    ) -> Result<AiPowL1OuterProof, String> {
        use p3_batch_stark::ProverData;
        use p3_circuit_prover::common::{get_airs_and_degrees_with_prep, NpoPreprocessor};
        use p3_circuit_prover::{
            recompose_air_builders, strip_public_binding_for_lookup_metadata, tip5_air_builders,
            BatchStarkProver, CircuitProverData, ConstraintProfile, RecomposePreprocessor,
            TablePacking, Tip5Preprocessor,
        };
        use p3_recursion::verifier::verify_p3_batch_proof_circuit;

        const TRACE_D: usize = 2;

        let l1_public_values = l2_public_values_for_l1(l1, statement_digest_public_values);
        let mut circuit_builder = CircuitBuilder::<Challenge>::new();
        circuit_builder.enable_tip5_perm::<Tip5Goldilocks, _>(
            generate_tip5_trace::<Challenge, Tip5Goldilocks>, LiftTip5,
        );
        circuit_builder.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
        circuit_builder.set_recompose_coeff_ctl_for_decompose_links(true);

        let lookup_gadget = LogUpGadget::new();
        let l1_table_provers = tip5_recompose_table_provers_for_l2();
        let (verifier_inputs, mmcs_op_ids) = verify_p3_batch_proof_circuit::<
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
        .map_err(|e| format!("build L2 verifier circuit over L1 proof: {e:?}"))?;

        let verification_circuit = circuit_builder
            .build()
            .map_err(|e| format!("build L2 circuit: {e:?}"))?;
        let (public_inputs, private_inputs) =
            verifier_inputs.pack_values(&l1_public_values, &l1.proof, &l1.stark_common);

        let l2_table_packing = TablePacking::new(DIGEST_ELEMS, 8).with_horner_pack_k(5);
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

        let mut runner = verification_circuit.runner();
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
            &mmcs_op_ids,
            &l1.proof.opening_proof,
            Tip5Config::GOLDILOCKS_W16,
        )
        .map_err(|e| format!("L2 set FRI MMCS private data: {e}"))?;
        let traces = runner
            .run()
            .map_err(|e| format!("L2 verifier circuit rejected L1 proof: {e:?}"))?;

        let lookup_metadata_airs = airs
            .iter()
            .map(strip_public_binding_for_lookup_metadata)
            .collect::<Vec<_>>();
        let prover_data =
            ProverData::from_airs_and_degrees(&l2_config, &lookup_metadata_airs, &degrees);
        let circuit_prover_data =
            CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

        let mut prover = BatchStarkProver::new(l2_config).with_table_packing(l2_table_packing);
        prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
        prover.register_recompose_table::<2>(true);
        let proof = prover
            .prove_all_tables(&traces, &circuit_prover_data)
            .map_err(|e| format!("L2 prove_all_tables: {e:?}"))?;
        prover
            .verify_all_tables(&proof)
            .map_err(|e| format!("L2 verify_all_tables: {e:?}"))?;
        Ok(proof)
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
            eprintln!(
                "relaxed L1-only pure-query statement-bound candidate [TEST_PEARL {label}]: l1_outer={} l1_proof_body={} l1_metadata={} l1_public_binding_lanes={} l1_log_blowup={} l1_num_queries={} l1_commit_pow_bits={} l1_query_pow_bits={} l1_johnson_bits={} prove_ms={} verify_ms={}",
                l1_outer_bytes,
                l1_proof_body_bytes,
                l1_metadata_bytes,
                outer.public_binding_lanes,
                log_blowup,
                num_queries,
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

    #[test]
    #[ignore = "pure-query AI-PoW L2-over-L1 recursive compression measurement is opt-in"]
    fn pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl() {
        use std::time::Instant;

        use p3_circuit::ops::Tip5Config;
        use p3_circuit_prover::BatchStarkProver;

        const L1_LOG_BLOWUP: usize = 6;
        const L1_NUM_QUERIES: usize = 10;
        const L1_CAP_HEIGHT: usize = 4;
        const L1_LOG_FINAL_POLY_LEN: usize = 2;
        const L2_LOG_BLOWUP: usize = 4;
        const L2_NUM_QUERIES: usize = 15;
        const L2_CAP_HEIGHT: usize = 4;
        assert_eq!(L1_LOG_BLOWUP * L1_NUM_QUERIES, 60);
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
            L1_LOG_BLOWUP, L1_NUM_QUERIES, L1_CAP_HEIGHT,
        );
        let l1_prove_start = Instant::now();
        let l1 = prove_composite_l1_outer_cert_with_config_and_public_binding_lanes(
            &built,
            &proof,
            l1_config.clone(),
            DIGEST_ELEMS,
        )
        .expect("pure-query cap-4 L1 recursive certificate");
        let l1_prove_ms = l1_prove_start.elapsed().as_millis();

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

        assert_eq!(
            l1.public_binding_lanes, DIGEST_ELEMS,
            "L1 proof must expose the statement digest before L2 wrapping"
        );

        for (l2_label, l2_log_blowup, l2_num_queries) in [
            ("lb4_nq15", L2_LOG_BLOWUP, L2_NUM_QUERIES),
            ("lb5_nq12", 5usize, 12usize),
            ("lb6_nq10", 6usize, 10usize),
        ] {
            assert_eq!(l2_log_blowup * l2_num_queries, 60);
            let l2_config = pure_query_l1_stark_config_with_shape_and_cap(
                l2_log_blowup, l2_num_queries, L2_CAP_HEIGHT,
            );
            let l2_prove_start = Instant::now();
            let l2 = prove_l2_over_l1_outer_for_test_pearl(
                &l1,
                &statement_digest_public_values,
                l1_config.clone(),
                &pure_query_fri_verifier_params_for_l1(L1_LOG_BLOWUP, L1_LOG_FINAL_POLY_LEN),
                l2_config,
            )
            .expect("pure-query L2 proof over statement-bound L1");
            let l2_prove_ms = l2_prove_start.elapsed().as_millis();

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

            eprintln!(
                "pure-query L2-over-L1 statement-bound candidate [TEST_PEARL L1 lb6_nq10_cap4 -> L2 {l2_label}_cap4]: l1_outer={} l1_proof_body={} l1_public_binding_lanes={} l1_log_blowup={} l1_num_queries={} l1_cap_height={} l1_commit_pow_bits={} l1_query_pow_bits={} l1_johnson_bits={} l1_prove_ms={} l1_verify_ms={} l2_outer={} l2_proof_body={} l2_metadata={} l2_commitments={} l2_opened_values={} l2_opening_proof={} l2_global_lookup_data={} l2_public_binding_lanes={} l2_log_blowup={} l2_num_queries={} l2_cap_height={} l2_commit_pow_bits={} l2_query_pow_bits={} l2_johnson_bits={} l2_prove_ms={}",
                l1_outer_bytes,
                l1_proof_body_bytes,
                l1.public_binding_lanes,
                L1_LOG_BLOWUP,
                L1_NUM_QUERIES,
                L1_CAP_HEIGHT,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
                L1_LOG_BLOWUP * L1_NUM_QUERIES,
                l1_prove_ms,
                l1_verify_ms,
                l2_outer_bytes,
                l2_proof_body_bytes,
                l2_outer_bytes.saturating_sub(l2_proof_body_bytes),
                l2_commitments_bytes,
                l2_opened_values_bytes,
                l2_opening_proof_bytes,
                l2_global_lookup_data_bytes,
                l2.public_binding_lanes,
                l2_log_blowup,
                l2_num_queries,
                L2_CAP_HEIGHT,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_COMMIT_POW_BITS,
                p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_PURE_QUERY_QUERY_POW_BITS,
                l2_log_blowup * l2_num_queries,
                l2_prove_ms,
            );

            assert_eq!(
                l2.public_binding_lanes, 0,
                "diagnostic L2 proof currently binds its L1 statement through the verifier public inputs"
            );
            assert!(
                l2_outer_bytes >= l2_proof_body_bytes,
                "L2 metadata split must be well formed"
            );
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

    fn measure_baseline_composite_terminal_relation(
        profile: CircuitConfig,
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
        measure_composite_l1_terminal_relation(&zk, &profile, &verified)
            .expect("terminal relation metrics must build without terminal proving")
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
    #[ignore = "production-profile terminal relation metrics are opt-in"]
    fn terminal_relation_metrics_for_prod_baseline_composite_are_available() {
        measure_and_assert_terminal_relation_for_profile("PROD", CircuitConfig::PROD);
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
        prelude_roots.extend(
            compiler
                .terminal_npo_fri_residual_zero_recompose_value_bridge_prelude_commitments_from_witness_goldilocks(
                    &vk,
                    &witness,
                )
                .expect("integrated LogUp diagnostic must bind merged NPO root"),
        );
        eprintln!(
            "native terminal integrated-LogUp candidate phase [{label}]: merged_npo_root_ms={}",
            merged_root_start.elapsed().as_millis()
        );
        let bundled_tip5_root_start = std::time::Instant::now();
        prelude_roots.extend(
            compiler
                .terminal_npo_tip5_lookup_trace_bundled_io_support_prelude_commitments_from_witness_goldilocks(
                    &vk,
                    &witness,
                )
                .expect("integrated LogUp diagnostic must bind bundled Tip5 root"),
        );
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
            .prove_terminal_npo_polynomial_fri_residual_zero_recompose_value_bridge_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
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
            .prove_terminal_npo_tip5_lookup_air_logup_trace_io_support_npo_io_logup_goldilocks(
                &vk, &witness.public_inputs, &witness, &prelude,
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
