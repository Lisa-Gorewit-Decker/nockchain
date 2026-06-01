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
use p3_recursion::{verify_batch_circuit, RecursiveAir, VerificationError};
use p3_symmetric::Permutation;
use p3_tip5_circuit_air::Tip5Perm as RecTip5Perm;

use crate::circuit::{Challenge, Tip5Compress, Tip5Sponge};
use crate::{AiPowStarkConfig, CompositeFullAirWithLookupsPinned, Val};

/// Canonical recursive certificate wrapper for Nockchain's AI proof-of-work
/// puzzle statement.
///
/// This recursive L1 certificate is the only ZK proof artifact intended
/// for Nockchain consensus, block persistence, or wire transmission.
/// Raw Layer-0 `AiPowBatchProof` values are intermediate prover inputs
/// and are not production certificates by themselves. The certificate binds
/// the Layer-0 public input vector supplied by the caller; consensus code must
/// still derive and check that statement, including any full-matmul admission
/// guard, before accepting it.
pub type AiPowRecursiveCertificate =
    p3_circuit_prover::BatchStarkProof<p3_circuit_prover::config::GoldilocksTipsConfig>;

/// Tip5 digest width (`DIGEST_ELEMS`), sponge `WIDTH`, sponge `RATE` —
/// the ai-pow-zk Layer-0 MMCS parameters (`circuit.rs`).
const DIGEST_ELEMS: usize = 5;
const WIDTH: usize = 16;
const RATE: usize = 10;

fn production_l1_table_packing(public_value_count: usize) -> p3_circuit_prover::TablePacking {
    p3_circuit_prover::TablePacking::new(public_value_count.max(1), 8)
        .with_public_binding_lanes(public_value_count)
}

fn flatten_l1_statement_public_values(public_values: &[Val]) -> Vec<Val> {
    let mut out = Vec::with_capacity(public_values.len() * 2);
    for &value in public_values {
        out.push(value);
        out.push(Val::ZERO);
    }
    out
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
/// to `nockchain_math::tip5::permute` (the permutation ai-pow-zk's
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

    let circuit = cb.build()?;
    let (public_inputs, private_inputs) =
        verifier_inputs.pack_values(&[public_values.to_vec()], proof, common_data);

    Ok(BuiltCompositeL1 {
        circuit,
        statement_public_values: public_values.to_vec(),
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
    runner.run().map_err(VerificationError::Circuit)?;
    Ok(())
}

/// S5 — produce the **L1 outer certificate** for a composite proof:
/// prove the composite-L1 verifier circuit itself as a D=2 batch-STARK
/// (`prove_all_tables`), then `verify_all_tables` — the live
/// cross-table `WitnessChecks` LogUp soundness gate. This is the
/// end-to-end recursive certificate: a small STARK whose statement is
/// "I verified the composite proof".
///
/// Mirrors the validated `outer_cert_layer0` machinery
/// (`Plonky3-recursion` `test_tip5_layer0_recursion.rs`) — D=2,
/// Tip5 NPO (D=1 perm) + recompose with split coeff tables — with the
/// composite-L1 circuit in place of the Fibonacci-L0 one.
///
/// Returns the L1 certificate (a `BatchStarkProof`) on accept; an
/// `Err` if `runner.run()` or `verify_all_tables` rejects.
pub fn prove_composite_l1_outer_cert(
    built: &BuiltCompositeL1,
    proof: &BatchProof<AiPowStarkConfig>,
) -> Result<
    p3_circuit_prover::BatchStarkProof<p3_circuit_prover::config::GoldilocksTipsConfig>,
    VerificationError,
> {
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
    let public_binding_lanes = built.statement_public_values.len();
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
    let prover_data = ProverData::from_airs_and_degrees(
        &config::goldilocks_tip5(),
        &lookup_metadata_airs,
        &degrees,
    );
    let circuit_prover_data =
        CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);
    let mut prover =
        BatchStarkProver::new(config::goldilocks_tip5()).with_table_packing(table_packing);
    prover.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    prover.register_recompose_table::<2>(true);

    let batch_proof = prover
        .prove_all_tables(&traces, &circuit_prover_data)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "composite L1 outer cert — prove_all_tables: {e:?}"
            ))
        })?;
    // The cross-table `WitnessChecks` soundness gate.
    let bound_public_values = flatten_l1_statement_public_values(&built.statement_public_values);
    prover
        .verify_all_tables_with_public_values(&batch_proof, &bound_public_values)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "composite L1 outer cert — verify_all_tables rejected: {e:?}"
            ))
        })?;
    Ok(batch_proof)
}

/// Verify only a legacy unbound outer recursive STARK envelope.
///
/// This checks that `cert` is a valid D=2 `BatchStarkProof` over the
/// circuit-prover tables used by [`prove_composite_l1_outer_cert`], including
/// the cross-table `WitnessChecks` LogUp argument. It deliberately enforces
/// the production recursion envelope (D=2, Tip5 + split recompose, ALU lanes
/// 8) instead of accepting arbitrary circuit-prover proof metadata as a
/// recursive certificate.
///
/// Canonical recursive certificates bind the Layer-0 public-input vector as
/// outer STARK public values. This helper rejects those bound certificates and
/// exists only as a diagnostic for old unbound proof objects. Consensus callers
/// must use [`verify_recursive_certificate`] with verifier-derived public
/// inputs, after the outer protocol has checked that those inputs describe the
/// intended full work unit.
#[deprecated(
    note = "outer-only verification is not a production AI-PoW verifier; use verify_recursive_certificate"
)]
pub fn verify_recursive_certificate_outer(
    cert: &AiPowRecursiveCertificate,
) -> Result<(), VerificationError> {
    if cert.public_binding_lanes != 0 {
        return Err(VerificationError::InvalidProofShape(
            "AI-PoW recursive certificate binds public statement values; use \
             verify_recursive_certificate with verifier-derived public inputs"
                .to_string(),
        ));
    }

    verify_recursive_certificate_with_public_values_inner(cert, &[], true)
}

/// Verify the canonical recursive certificate against the verifier-derived
/// Layer-0 AI-PoW public inputs.
pub fn verify_recursive_certificate(
    cert: &AiPowRecursiveCertificate,
    public_inputs: &crate::composite_public::CompositePublicInputs,
) -> Result<(), VerificationError> {
    verify_recursive_certificate_with_public_values(cert, &public_inputs.to_vec())
}

/// Verify the canonical recursive certificate against the verifier-derived
/// Layer-0 AI-PoW public-input vector.
pub fn verify_recursive_certificate_with_public_values(
    cert: &AiPowRecursiveCertificate,
    public_values: &[Val],
) -> Result<(), VerificationError> {
    verify_recursive_certificate_with_public_values_inner(cert, public_values, false)
}

fn verify_recursive_certificate_with_public_values_inner(
    cert: &AiPowRecursiveCertificate,
    public_values: &[Val],
    allow_empty_statement: bool,
) -> Result<(), VerificationError> {
    use p3_circuit_prover::{config, BatchStarkProver};

    if public_values.is_empty() && !allow_empty_statement {
        return Err(VerificationError::InvalidProofShape(
            "AI-PoW recursive certificate verification requires non-empty \
             verifier-derived public inputs"
                .to_string(),
        ));
    }

    let expected_packing = production_l1_table_packing(public_values.len());
    if cert.ext_degree != 2 {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW recursive certificate uses extension degree {}; expected 2",
            cert.ext_degree
        )));
    }
    if cert.table_packing != expected_packing {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW recursive certificate uses non-production table packing {:?}; \
             expected {:?}",
            cert.table_packing, expected_packing
        )));
    }
    if cert.public_binding_lanes != public_values.len() {
        return Err(VerificationError::InvalidProofShape(format!(
            "AI-PoW recursive certificate binds {} statement values; expected {}",
            cert.public_binding_lanes,
            public_values.len()
        )));
    }
    if cert.alu_quintic_trinomial {
        return Err(VerificationError::InvalidProofShape(
            "AI-PoW recursive certificate unexpectedly selected quintic ALU".to_string(),
        ));
    }

    let mut verifier =
        BatchStarkProver::new(config::goldilocks_tip5()).with_table_packing(expected_packing);
    verifier.register_tip5_table::<2>(Tip5Config::GOLDILOCKS_W16);
    verifier.register_recompose_table::<2>(true);
    let bound_public_values = flatten_l1_statement_public_values(public_values);
    verifier
        .verify_all_tables_with_public_values(cert, &bound_public_values)
        .map_err(|e| {
            VerificationError::InvalidProofShape(format!(
                "AI-PoW recursive certificate verification rejected: {e:?}"
            ))
        })
}

/// Per-stage instrumentation of one end-to-end composite→L1
/// recursion run — the production caller's measurement output.
///
/// `l1_cert` is the canonical recursive certificate. The included
/// `composite_proof` is exposed only for benchmarking, diagnostics,
/// and recursive-prover internals; it must not be used as the
/// Nockchain consensus proof artifact.
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
    /// The composite (L0) proof.
    ///
    /// Intermediate only. Do not persist or transmit this as the
    /// Nockchain AI-PoW certificate.
    pub composite_proof: BatchProof<AiPowStarkConfig>,
    /// The L1 recursive certificate.
    ///
    /// This is the canonical recursive proof artifact.
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
    /// The canonical recursive certificate.
    pub l1_cert: AiPowRecursiveCertificate,
}

/// **Canonical recursive caller** — the full ai-pow-zk → Plonky3-recursion
/// pipeline for one composite proof, end to end:
///
/// 1. prove the composite matmul-PoW batch-STARK (Layer 0);
/// 2. build the L1 recursive-verifier circuit and run it — the
///    composite proof is verified in-circuit (S3);
/// 3. outer-prove that verifier circuit as a D=2 batch-STARK and
///    `verify_all_tables` — the L1 recursive certificate (S5).
///
/// Returns per-stage timings and the canonical L1 certificate. The
/// returned Layer-0 proof is for diagnostics/measurement only. The L1
/// recursive certificate is the only artifact that production
/// Nockchain consensus should persist or transmit.
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
    let l1_cert = prove_composite_l1_outer_cert(&built, &composite_proof)?;
    let l1_outer_cert_ms = t.elapsed().as_millis();

    Ok(L1RecursionRun {
        composite_trace_height,
        composite_trace_width,
        composite_prove_ms,
        l1_circuit_build_ms,
        l1_in_circuit_verify_ms,
        l1_outer_cert_ms,
        public_inputs: pis,
        composite_proof,
        l1_cert,
    })
}

/// Produce Nockchain's canonical recursive AI-PoW certificate from an
/// already-generated Layer-0 composite proof.
///
/// The caller supplies the pinned Layer-0 program and public inputs that were
/// produced with the proof. This function recursively verifies that Layer-0
/// proof in-circuit and returns only the recursive L1 certificate. It does not
/// serialize, persist, or bless the Layer-0 proof as a block artifact. It also
/// does not decide whether the Layer-0 statement represents enough consensus
/// work; that check belongs to the chain-derived statement verifier.
pub fn prove_canonical_ai_pow_certificate_from_composite_proof(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    program: &crate::AiPowProgram,
    proof: &BatchProof<AiPowStarkConfig>,
    public_inputs: &crate::composite_public::CompositePublicInputs,
) -> Result<L1CertificateRun, VerificationError> {
    use std::time::Instant;

    let cfg = crate::composite_proof::build_config(zk_params, profile);
    let t = Instant::now();
    let air = CompositeFullAirWithLookupsPinned::new_with(program.clone(), true);
    let pd = crate::composite_proof::logup_common_for(&cfg, program, true);
    let built = build_composite_l1_verifier_circuit(
        &cfg,
        &air,
        proof,
        &pd.common,
        &public_inputs.to_vec(),
        profile,
    )?;
    let l1_circuit_build_ms = t.elapsed().as_millis();

    let t = Instant::now();
    run_composite_l1_verifier(&built, proof)?;
    let l1_in_circuit_verify_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let l1_cert = prove_composite_l1_outer_cert(&built, proof)?;
    let l1_outer_cert_ms = t.elapsed().as_millis();

    Ok(L1CertificateRun {
        l1_circuit_build_ms,
        l1_in_circuit_verify_ms,
        l1_outer_cert_ms,
        l1_cert,
    })
}

/// Produce Nockchain's canonical recursive AI-PoW certificate.
///
/// This is a name-level guardrail for consensus callers: the
/// certificate is recursive. Callers that only need the canonical proof
/// should use this function and persist/transmit `run.l1_cert`, never
/// `run.composite_proof`. Consensus callers must separately derive the exact
/// public statement and reject selected-tile statements that do not prove the
/// intended full-matmul work unit.
pub fn prove_canonical_ai_pow_certificate(
    zk_params: &crate::params::ZkParams,
    profile: &crate::circuit::CircuitConfig,
    trace: crate::composite_trace::CompositeTrace,
) -> Result<L1RecursionRun, VerificationError> {
    recurse_composite_to_l1(zk_params, profile, trace)
}

/// Serialize the canonical recursive AI-PoW certificate into compact bytes.
///
/// This serializes only the recursive L1 certificate. It intentionally does
/// not accept or produce a Layer-0 `AiPowBatchProof`, because Layer-0 proofs
/// are not canonical block/wire certificates for Nockchain AI-PoW.
pub fn encode_recursive_certificate(
    cert: &AiPowRecursiveCertificate,
) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(cert)
}

/// Decode bytes previously produced by [`encode_recursive_certificate`].
///
/// Decoding is structural only; callers still need to verify the certificate
/// against chain-derived statement data once the verifier is wired.
pub fn decode_recursive_certificate(
    bytes: &[u8],
) -> Result<AiPowRecursiveCertificate, postcard::Error> {
    postcard::from_bytes(bytes)
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
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&test_zk_params(), &profile);

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
        let cert =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");

        verify_recursive_certificate(&cert, &pis)
            .expect("recursive certificate verifier must accept honest cert");
        verify_recursive_certificate_with_public_values(&cert, &[])
            .expect_err("recursive verifier must reject empty statement public inputs");
    }

    #[test]
    fn recursive_certificate_outer_verifier_rejects_non_production_envelope() {
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&test_zk_params(), &profile);

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
        let mut cert =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");

        cert.ext_degree = 1;
        verify_recursive_certificate(&cert, &pis)
            .expect_err("recursive verifier must reject non-D=2 recursion envelope");
    }

    #[test]
    fn recursive_certificate_rejects_wrong_statement_public_inputs() {
        let profile = CircuitConfig::TEST_PEARL;
        let cfg = build_config(&test_zk_params(), &profile);

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
        let cert =
            prove_composite_l1_outer_cert(&built, &proof).expect("honest recursive certificate");

        let mut wrong = pis.clone();
        wrong.job_key[0] ^= 1;
        verify_recursive_certificate(&cert, &wrong)
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
