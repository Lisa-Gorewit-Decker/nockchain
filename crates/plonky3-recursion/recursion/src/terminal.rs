//! Native terminal-compression seam for recursion verifier circuits.
//!
//! The recursion crate already builds a verifier [`Circuit`] for the previous
//! proof.  A terminal compressor is responsible for proving that circuit's
//! execution without forcing the final artifact to be another batch-STARK.
//! The existing batch-STARK path remains the production implementation; this
//! module isolates the handoff needed by a future compact terminal backend.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};

use p3_circuit::ops::{
    AluOpKind, NpoTypeId, Op, RecomposeCircuitRow, RecomposeTrace, RecomposeTraceKind,
    Tip5CircuitRow, Tip5Config, Tip5TerminalMode, Tip5Trace,
};
use p3_circuit::tables::Traces;
use p3_circuit::{Circuit, WitnessId};
use p3_field::{BasedVectorSpace, Field, PrimeCharacteristicRing, PrimeField64};
use p3_goldilocks::Goldilocks;
use p3_symmetric::Permutation;
use p3_tip5_circuit_air::{NUM_ROUNDS as TIP5_PERM_ROUNDS, Tip5Perm};
use serde::{Deserialize, Serialize};

/// Minimum production soundness accepted for native terminal certificates.
pub const MIN_TERMINAL_SECURITY_BITS: u16 = 60;

/// Stable fingerprint for a compiled terminal verifier circuit.
///
/// This intentionally records only structural fields that affect the witness
/// interface and operation sequence.  Backend-specific proving keys may bind
/// more data, but a mismatch here is always a cache miss.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCircuitFingerprint {
    pub witness_count: u32,
    pub public_flat_len: usize,
    pub private_flat_len: usize,
    pub ops_len: usize,
}

impl TerminalCircuitFingerprint {
    pub const fn from_circuit<F>(circuit: &Circuit<F>) -> Self {
        Self {
            witness_count: circuit.witness_count,
            public_flat_len: circuit.public_flat_len,
            private_flat_len: circuit.private_flat_len,
            ops_len: circuit.ops.len(),
        }
    }
}

/// Executed verifier-circuit witness handed to a terminal compressor.
///
/// `public_inputs` are the verifier-visible statement values for the terminal
/// proof. `private_inputs` and `traces` are prover-only data used to witness
/// circuit execution. The final compact certificate must bind the public inputs
/// and the terminal circuit fingerprint/proving key.
pub struct TerminalWitness<F> {
    pub fingerprint: TerminalCircuitFingerprint,
    pub public_inputs: Vec<F>,
    pub private_inputs: Vec<F>,
    pub traces: Traces<F>,
}

/// Contract for a native compact terminal compressor.
///
/// This is deliberately independent of the current batch-STARK certificate
/// type. Implementations compile a `p3_circuit` verifier circuit into their own
/// proving/verifying keys, then prove an executed [`TerminalWitness`].
pub trait TerminalCompressor<F: Field> {
    type ProvingKey;
    type VerifyingKey;
    type Proof;
    type Error;

    /// Protocol identifier committed by wire/certificate metadata.
    fn protocol_id(&self) -> &'static str;

    /// Compile a verifier circuit into terminal-compressor keys.
    fn compile(
        &self,
        circuit: &Circuit<F>,
    ) -> Result<(Self::ProvingKey, Self::VerifyingKey), Self::Error>;

    /// Prove one executed verifier circuit.
    fn prove(
        &self,
        proving_key: &Self::ProvingKey,
        witness: &TerminalWitness<F>,
    ) -> Result<Self::Proof, Self::Error>;

    /// Verify a terminal proof against public inputs and a circuit fingerprint.
    fn verify(
        &self,
        verifying_key: &Self::VerifyingKey,
        proof: &Self::Proof,
        fingerprint: TerminalCircuitFingerprint,
        public_inputs: &[F],
    ) -> Result<(), Self::Error>;

    /// Serialize a proof into the certificate body.
    fn serialize_proof(&self, proof: &Self::Proof) -> Result<Vec<u8>, Self::Error>;
}

/// Minimal metadata all native terminal certificates must expose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCertificateHeader {
    pub version: u32,
    pub protocol_id: String,
    pub security_bits: u16,
    pub fingerprint: TerminalCircuitFingerprint,
    pub relation_digest: Option<TerminalRelationDigest>,
}

/// Terminal proof body semantics bound by the certificate digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalProofKind {
    /// Local sampled/folded checkpoint used for development measurements.
    ///
    /// This kind is intentionally not the production terminal compressor: it
    /// verifies the implemented local proof components, but does not by itself
    /// establish a complete polynomial IOP/proximity argument for the terminal
    /// relation.
    #[cfg(test)]
    LocalCheckpoint,
    /// Production terminal proof backend.
    Production,
}

/// Tip5 digest of a compiled Goldilocks terminal relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalRelationDigest(pub [u64; 5]);

/// Tip5 digest of the backend-projected terminal relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalBackendRelationDigest(pub [u64; 5]);

/// Tip5 digest of the terminal certificate's public input vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPublicValuesDigest(pub [u64; 5]);

/// Tip5 digest of the backend-specific terminal proof body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalProofBodyDigest(pub [u64; 5]);

/// Tip5 digest binding terminal metadata, public values, and proof body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalBindingDigest(pub [u64; 5]);

/// Tip5 digest of a backend commitment root in the terminal transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCommitmentDigest(pub [u64; 5]);

/// Directional sibling in a terminal Merkle opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalMerkleSibling {
    pub digest: TerminalCommitmentDigest,
    pub sibling_is_left: bool,
}

/// Authenticated opening of one terminal oracle value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOracleOpening {
    pub index: usize,
    pub value_basis: Vec<u64>,
    pub path: Vec<TerminalMerkleSibling>,
}

/// One value opened by a terminal oracle multiproof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOracleMultiValueOpening {
    pub index: usize,
    pub value_basis: Vec<u64>,
}

/// Sparse Merkle multiproof for terminal oracle values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOracleMultiProof {
    pub openings: Vec<TerminalOracleMultiValueOpening>,
    pub frontier: Vec<TerminalCommitmentDigest>,
}

/// Sparse Merkle multiproof for terminal oracle values whose indices are
/// verifier-derived from the surrounding relation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOracleKnownIndexMultiProof {
    pub value_basis_flat: Vec<u64>,
    pub boolean_value_bits: Vec<u8>,
    pub frontier: Vec<TerminalCommitmentDigest>,
}

/// Authenticated opening of a contiguous oracle prefix starting at index 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOraclePrefixProof {
    pub prefix_len: usize,
    pub frontier: Vec<TerminalCommitmentDigest>,
}

/// Merkle commitment to a terminal oracle vector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOracleCommitment {
    pub label: String,
    pub values_len: usize,
    pub root: TerminalCommitmentDigest,
}

/// Tip5 digest of the first terminal Fiat-Shamir challenge state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalTranscriptChallengeDigest(pub [u64; 5]);

/// Production terminal proof parameters committed before challenge sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalProofParameters {
    pub security_bits: u16,
    pub log_blowup: u8,
    pub num_queries: u16,
    pub query_pow_bits: u16,
}

impl TerminalProofParameters {
    pub const fn production_60bit() -> Self {
        Self {
            security_bits: MIN_TERMINAL_SECURITY_BITS,
            log_blowup: 4,
            num_queries: 15,
            query_pow_bits: 0,
        }
    }

    pub const fn johnson_bits(&self) -> u32 {
        self.log_blowup as u32 * self.num_queries as u32
    }
}

/// Wire object for a native terminal certificate.
///
/// The `proof_body` is backend-specific; the outer certificate format binds it
/// to public values, terminal relation metadata, and the minimum soundness
/// profile before any backend verifier is allowed to inspect it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCertificate {
    pub header: TerminalCertificateHeader,
    pub proof_kind: TerminalProofKind,
    pub public_values_digest: TerminalPublicValuesDigest,
    pub proof_body_digest: TerminalProofBodyDigest,
    pub binding_digest: TerminalBindingDigest,
    pub proof_body: Vec<u8>,
}

/// First transcript object for a native terminal proof body.
///
/// A compact backend must commit to its witness/oracle material, bind the
/// compiled relation and public values, then derive challenges from this
/// prelude. The prelude is not a proof by itself; it is the fail-closed
/// transcript boundary every backend proof must start from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalProofPrelude {
    pub parameters: TerminalProofParameters,
    pub relation_profile: TerminalRelationProfile,
    pub public_values_digest: TerminalPublicValuesDigest,
    pub commitments: Vec<TerminalCommitmentDigest>,
    pub query_pow_nonce: u64,
    pub challenge_digest: TerminalTranscriptChallengeDigest,
}

/// Verifier-derived query indices for one committed terminal oracle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalQueryPlan {
    pub oracle_label: String,
    pub oracle_len: usize,
    pub indices: Vec<usize>,
}

/// Verifier-derived primitive-constraint indices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalConstraintQueryPlan {
    pub domain_len: usize,
    pub indices: Vec<usize>,
}

/// Verifier-derived non-primitive row indices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoQueryPlan {
    pub domain_len: usize,
    pub indices: Vec<usize>,
}

/// Opened witness values for one sampled primitive terminal constraint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPrimitiveConstraintOpening {
    pub constraint_index: usize,
    pub witness_openings: Vec<TerminalOracleOpening>,
}

/// Sampled local proof for primitive terminal constraints.
///
/// This proves only local primitive equations at transcript-derived constraint
/// indices. It is not the full terminal relation proof because NPO/global
/// consistency still needs its own argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPrimitiveConstraintProof {
    pub openings: Vec<TerminalPrimitiveConstraintOpening>,
}

/// Opened witness links and local values for one sampled NPO row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalTip5HiddenInputValue {
    pub limb: usize,
    pub value_basis: Vec<u64>,
}

/// Opened witness links and local values for one sampled NPO row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoOpening {
    pub npo_index: usize,
    pub tip5_hidden_input_values: Vec<TerminalTip5HiddenInputValue>,
    pub witness_openings: Vec<TerminalOracleOpening>,
}

/// Local NPO row values whose witness links are carried by a shared multiproof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoLocalOpening {
    pub npo_index: usize,
    pub tip5_hidden_input_nonzero_mask: u16,
    pub tip5_hidden_input_values_le: Vec<[u8; 8]>,
}

/// Sampled local proof for supported non-primitive terminal rows.
///
/// This checks transcript-derived Tip5/recompose rows against the committed
/// witness oracle. It is not the full terminal relation proof because a compact
/// backend still needs a global consistency/proximity argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoProof {
    pub openings: Vec<TerminalNpoOpening>,
}

/// Opened NPO row and validity-residual value for one sampled folded NPO row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoValidityConsistencyOpening {
    pub validity_index: usize,
    pub npo_index: usize,
    pub component_offset: usize,
    pub npo_opening: TerminalNpoLocalOpening,
}

/// Sampled consistency proof tying the NPO validity oracle to the witness
/// oracle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoValidityConsistencyProof {
    pub openings: Vec<TerminalNpoValidityConsistencyOpening>,
    pub witness_multi_opening: TerminalOracleMultiProof,
}

/// Exhaustive proof for every supported non-primitive terminal row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoExhaustiveProof {
    /// Verifier-selected hidden Tip5 inputs, in NPO row order. Recompose rows
    /// have no Tip5 hidden inputs, and selected zero-valued Tip5 lanes are
    /// serialized as canonical zero values rather than represented by a mask.
    pub tip5_hidden_input_values_le: Vec<[u8; 8]>,
    pub witness_multi_opening: TerminalOracleKnownIndexMultiProof,
}

/// Opened row material for one sampled combined-validity row.
///
/// The combined validity oracle is ordered as
/// `[primitive quadratic residuals || supported NPO validity residuals]`.
/// Primitive rows carry witness openings needed to recompute
/// `A(w) * B(w) - C(w)`; NPO rows carry the supported Tip5/recompose
/// row opening needed to recompute a zero validity residual.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalCombinedValidityConsistencyOpening {
    Quadratic {
        validity_index: usize,
        witness_openings: Vec<TerminalOracleOpening>,
    },
    Npo {
        validity_index: usize,
        npo_index: usize,
        component_offset: usize,
        npo_opening: TerminalNpoOpening,
    },
}

/// Sampled consistency proof tying the combined-validity oracle to the
/// witness oracle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCombinedValidityConsistencyProof {
    pub openings: Vec<TerminalCombinedValidityConsistencyOpening>,
}

/// Sampled openings of the primitive quadratic residual oracle.
///
/// The residual oracle is the vector `A(w) * B(w) - C(w)` for every lowered
/// primitive quadratic constraint. A complete compact backend still needs a
/// proximity/sumcheck argument tying this oracle to the committed witness; this
/// component binds and checks transcript-derived zero openings of that oracle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalQuadraticResidualProof {
    pub openings: Vec<TerminalOracleOpening>,
}

/// Opened witness and residual values for one sampled primitive quadratic row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalQuadraticConsistencyOpening {
    pub quadratic_index: usize,
    pub witness_openings: Vec<TerminalOracleOpening>,
}

/// Sampled consistency proof tying the primitive residual oracle to the witness
/// oracle.
///
/// For each transcript-derived quadratic row, the verifier opens the residual
/// oracle and every witness used by that row, recomputes `A(w) * B(w) - C(w)`,
/// and checks both equality to the opened residual and zero residual.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalQuadraticConsistencyProof {
    pub openings: Vec<TerminalQuadraticConsistencyOpening>,
}

/// One sampled consistency path through the folded residual oracle layers.
///
/// For a present right leaf, `right` carries only the right index and value
/// with an empty path. The value is authenticated by matching its leaf digest
/// against the first sibling digest in the left opening, avoiding a duplicate
/// Merkle path for the adjacent leaf. `next` also carries only index and value:
/// it is authenticated by the next round's current-layer opening, or by the
/// final one-leaf fold root in the last round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalResidualFoldRoundOpening {
    pub round: usize,
    pub pair_index: usize,
    pub left: TerminalOracleOpening,
    pub right: Option<TerminalOracleOpening>,
    pub next: TerminalOracleOpening,
}

/// Sampled residual fold path starting at one transcript-derived residual row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalResidualFoldQueryOpening {
    pub initial_index: usize,
    pub rounds: Vec<TerminalResidualFoldRoundOpening>,
}

/// Merkle-backed multilinear folding proof for the primitive residual oracle.
///
/// Each round folds adjacent residual values with a transcript-derived field
/// challenge. The verifier checks sampled pair folds and confirms the final
/// one-value folded commitment opens to zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalResidualFoldProof {
    pub fold_commitments: Vec<TerminalOracleCommitment>,
    pub final_value_basis: Vec<u64>,
    pub openings: Vec<TerminalResidualFoldQueryOpening>,
}

/// Merkle-backed multilinear evaluation proof for the terminal assignment
/// vector `[1 || public || witness]`.
///
/// The proof authenticates the public prefix against the assignment commitment
/// with a compact Merkle frontier, then folds the whole assignment vector to one
/// transcript-derived evaluation. This is the PCS primitive the terminal R1CS
/// sumcheck needs for its final `z(y*)` check; it is not accepted as a
/// standalone production proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalAssignmentEvaluationProof {
    pub public_prefix_proof: TerminalOraclePrefixProof,
    pub fold_commitments: Vec<TerminalOracleCommitment>,
    pub final_value_basis: Vec<u64>,
    pub round_openings: Vec<TerminalOracleMultiProof>,
    pub openings: Vec<TerminalResidualFoldQueryOpening>,
}

/// One degree-2 univariate round in the sparse-R1CS matrix-vector sumcheck.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalR1csSumcheckRound {
    pub eval_0_basis: Vec<u64>,
    pub eval_1_basis: Vec<u64>,
    pub eval_2_basis: Vec<u64>,
}

/// Batched sparse-R1CS matrix-vector sumcheck for primitive terminal rows.
///
/// The proof claims `A(r)`, `B(r)`, and `C(r)` at a transcript-derived row
/// point `r`, proves those batched matrix-vector evaluations against the
/// committed assignment vector, and carries the assignment evaluation proof at
/// the final sumcheck point `y*`. It intentionally does not check
/// `A(r) * B(r) = C(r)`, which would be an unsound shortcut for multilinear
/// R1CS; the row-product relation needs its own row-sumcheck.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSparseR1csSumcheckProof {
    pub claimed_a_basis: Vec<u64>,
    pub claimed_b_basis: Vec<u64>,
    pub claimed_c_basis: Vec<u64>,
    pub rounds: Vec<TerminalR1csSumcheckRound>,
    pub assignment_evaluation: TerminalAssignmentEvaluationProof,
}

/// One degree-3 univariate round in the primitive R1CS row-product sumcheck.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalR1csRowProductRound {
    pub eval_0_basis: Vec<u64>,
    pub eval_1_basis: Vec<u64>,
    pub eval_2_basis: Vec<u64>,
    pub eval_3_basis: Vec<u64>,
}

/// Primitive sparse-R1CS row-product sumcheck.
///
/// This proves the random linearized row residual relation
/// `sum_x eq(r, x) * (A(x) * B(x) - C(x)) = 0`, then delegates final
/// `A(x*)`, `B(x*)`, and `C(x*)` claims to the matrix-vector sumcheck at the
/// externally supplied row point `x*`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalR1csRowProductSumcheckProof {
    pub rounds: Vec<TerminalR1csRowProductRound>,
    pub matrix_sumcheck: TerminalSparseR1csSumcheckProof,
}

/// Merkle-backed multilinear folding proof for supported NPO validity rows.
///
/// The base oracle is a row-validity residual vector over supported NPO
/// callsites. Each entry is zero when the corresponding Tip5/recompose row is
/// satisfied by the committed witness. The sampled consistency proof links the
/// same fold-query indices back to row recomputation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNpoValidityFoldProof {
    pub fold_commitments: Vec<TerminalOracleCommitment>,
    pub final_value_basis: Vec<u64>,
    pub round_openings: Vec<TerminalOracleMultiProof>,
    pub openings: Vec<TerminalResidualFoldQueryOpening>,
}

/// Envelope for all implemented terminal local-proof components.
///
/// This is a backend integration checkpoint, not the final compact terminal
/// proof: it binds the current prelude, witness oracle, residual oracle, and
/// residual/NPO validity consistency checks into one proof body shape. A
/// production terminal backend still has to add the global proximity/sumcheck
/// argument.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct TerminalLocalProof {
    pub prelude: TerminalProofPrelude,
    pub witness_commitment: TerminalOracleCommitment,
    pub combined_validity_commitment: TerminalOracleCommitment,
    pub combined_validity_consistency_proof: TerminalCombinedValidityConsistencyProof,
    pub combined_validity_fold_proof: TerminalResidualFoldProof,
}

/// Production proof-body checkpoint for the terminal relation.
///
/// This proof binds a witness oracle for sampled supported-NPO row openings,
/// an assignment oracle for primitive sparse-R1CS sumcheck, and an optional NPO
/// validity oracle for keys with supported NPO rows. It no longer serializes
/// the full witness. The remaining backend gap is polynomializing the supported
/// Tip5/recompose NPO validity relation rather than checking sampled rows
/// directly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalProductionProof {
    pub prelude: TerminalProofPrelude,
    pub witness_commitment: TerminalOracleCommitment,
    pub assignment_commitment: TerminalOracleCommitment,
    pub primitive_r1cs_proof: TerminalR1csRowProductSumcheckProof,
    pub npo_exhaustive_proof: Option<TerminalNpoExhaustiveProof>,
}

/// Operation inventory for a terminal verifier circuit.
///
/// This is the first compiler artifact for the native terminal backend.  It
/// records the exact operation classes that must be arithmetized by the compact
/// terminal proof system.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOpInventory {
    pub const_ops: usize,
    pub public_ops: usize,
    pub alu_add_ops: usize,
    pub alu_mul_ops: usize,
    pub alu_bool_check_ops: usize,
    pub alu_mul_add_ops: usize,
    pub alu_horner_acc_ops: usize,
    pub hint_ops: usize,
    pub non_primitive_ops: usize,
    pub non_primitive_types: Vec<String>,
}

impl TerminalOpInventory {
    pub fn total_primitive_ops(&self) -> usize {
        self.const_ops
            + self.public_ops
            + self.alu_add_ops
            + self.alu_mul_ops
            + self.alu_bool_check_ops
            + self.alu_mul_add_ops
            + self.alu_horner_acc_ops
    }

    pub fn total_ops(&self) -> usize {
        self.total_primitive_ops() + self.hint_ops + self.non_primitive_ops
    }
}

/// Size profile of a compiled native terminal relation.
///
/// This profile is intentionally proof-system agnostic: it describes the
/// concrete relation a compact terminal backend must prove, before choosing a
/// polynomial commitment, sumcheck, or other terminal protocol.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalRelationProfile {
    pub fingerprint: TerminalCircuitFingerprint,
    pub primitive_constraints: usize,
    pub terminal_constraints: usize,
    pub hint_ops: usize,
    pub non_primitive_ops: usize,
    pub tip5_rows: usize,
    pub recompose_rows: usize,
    pub recompose_coeff_rows: usize,
    pub external_npo_validity_components: usize,
    pub npo_callsite_input_slots: usize,
    pub npo_callsite_output_slots: usize,
}

impl TerminalRelationProfile {
    pub fn non_primitive_rows(&self) -> usize {
        self.tip5_rows + self.recompose_rows + self.recompose_coeff_rows
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeTerminalCompileError {
    SecurityBitsTooLow {
        requested: u16,
        minimum: u16,
    },
    UnsupportedNonPrimitive {
        op_index: usize,
        op_type: String,
    },
    MalformedSupportedNonPrimitive {
        op_index: usize,
        op_type: String,
        reason: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeTerminalConstraint<F> {
    Const {
        out: WitnessId,
        val: F,
    },
    Public {
        out: WitnessId,
        public_pos: usize,
    },
    Alu {
        kind: AluOpKind,
        a: WitnessId,
        b: WitnessId,
        c: Option<WitnessId>,
        out: WitnessId,
        intermediate_out: Option<WitnessId>,
    },
    Tip5Goldilocks {
        op_type: String,
        expected_rows: usize,
        callsites: Vec<NativeTerminalNpoCallsite>,
    },
    RecomposeGoldilocks {
        op_type: String,
        expected_rows: usize,
        callsites: Vec<NativeTerminalNpoCallsite>,
    },
}

/// Variable source for the primitive terminal quadratic relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalLinearVariable {
    One,
    Public(usize),
    Witness(WitnessId),
}

/// One term in a terminal linear combination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLinearTerm<F> {
    pub coeff: F,
    pub variable: TerminalLinearVariable,
}

/// Linear combination over terminal constants, public inputs, and witness
/// values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalLinearExpression<F> {
    pub terms: Vec<TerminalLinearTerm<F>>,
}

/// R1CS-style primitive equation `a * b = c`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalQuadraticConstraint<F> {
    pub source_constraint_index: usize,
    pub kind: &'static str,
    pub a: TerminalLinearExpression<F>,
    pub b: TerminalLinearExpression<F>,
    pub c: TerminalLinearExpression<F>,
}

/// Backend-ready algebraic relation for primitive terminal constraints.
///
/// Supported NPO rows remain external rows at this checkpoint; their dedicated
/// Tip5/recompose arithmetization is tracked separately from this primitive
/// quadratic system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalQuadraticRelation<F> {
    pub constraints: Vec<TerminalQuadraticConstraint<F>>,
    pub external_npo_rows: usize,
}

/// R1CS matrix selected by one sparse multilinear entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSparseR1csMatrix {
    A,
    B,
    C,
}

/// Variable column in the sparse terminal R1CS view.
///
/// The final polynomial/sumcheck backend treats `One`, public inputs, and
/// witness values as one concatenated assignment vector. `variable_index` below
/// is the stable column in that concatenated vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSparseR1csVariable {
    One,
    Public(usize),
    Witness(WitnessId),
}

/// One nonzero sparse entry in the terminal R1CS matrices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSparseR1csEntry<F> {
    pub matrix: TerminalSparseR1csMatrix,
    pub row: usize,
    pub variable: TerminalSparseR1csVariable,
    pub variable_index: usize,
    pub coeff: F,
}

/// Sparse multilinear view of the primitive terminal R1CS relation.
///
/// This is the relation table a Spartan/Aurora-style sumcheck backend needs:
/// rows index quadratic constraints, columns index the concatenated assignment
/// vector `[1 || public || witness]`, and entries are sparse matrix weights for
/// `A(z) * B(z) = C(z)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSparseR1csRelation<F> {
    pub rows: usize,
    pub variables: usize,
    pub public_count: usize,
    pub witness_count: usize,
    pub log_rows: usize,
    pub log_variables: usize,
    pub entries: Vec<TerminalSparseR1csEntry<F>>,
}

/// Supported external NPO row kind for backend arithmetization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalNpoRowKind {
    Tip5Goldilocks,
    Recompose,
    RecomposeWithCoeffLookups,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalNpoResidualKind {
    Tip5Input,
    Tip5Output,
    Tip5ChainInput,
    Tip5MmcsBit,
    RecomposeInput,
    RecomposeOutput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalNpoRowResidual {
    pub kind: TerminalNpoResidualKind,
    pub limb: usize,
    pub basis: Vec<Goldilocks>,
}

impl TerminalNpoRowResidual {
    pub fn is_zero(&self) -> bool {
        self.basis
            .iter()
            .copied()
            .all(|coeff| coeff == Goldilocks::ZERO)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalNpoRowEvaluation {
    pub row_kind: TerminalNpoRowKind,
    pub residuals: Vec<TerminalNpoRowResidual>,
}

impl TerminalNpoRowEvaluation {
    pub fn new(row_kind: TerminalNpoRowKind) -> Self {
        Self {
            row_kind,
            residuals: Vec::new(),
        }
    }

    pub fn is_satisfied(&self) -> bool {
        self.residuals.iter().all(TerminalNpoRowResidual::is_zero)
    }

    pub fn first_nonzero(&self) -> Option<&TerminalNpoRowResidual> {
        self.residuals.iter().find(|residual| !residual.is_zero())
    }
}

/// One deterministic external NPO relation row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalNpoRelationRow {
    pub npo_index: usize,
    pub op_type: String,
    pub local_row: usize,
    pub kind: TerminalNpoRowKind,
    pub callsite: NativeTerminalNpoCallsite,
}

/// Backend-ready external NPO relation.
///
/// Rows are ordered by the compiled terminal constraint order and then local
/// row number inside each supported NPO aggregate. This gives future terminal
/// backends a stable global row domain for Tip5/recompose arithmetization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalNpoRelation {
    pub rows: Vec<TerminalNpoRelationRow>,
}

impl TerminalNpoRelation {
    pub fn tip5_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.kind == TerminalNpoRowKind::Tip5Goldilocks)
            .count()
    }

    pub fn recompose_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.kind == TerminalNpoRowKind::Recompose)
            .count()
    }

    pub fn recompose_coeff_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.kind == TerminalNpoRowKind::RecomposeWithCoeffLookups)
            .count()
    }
}

/// Backend profile for the polynomialized supported-NPO table.
///
/// This is the stable table contract for the terminal proximity backend. It is
/// derived from the verifying key, absorbed into the backend relation digest,
/// and deliberately kept out of the prelude's serialized relation profile so it
/// does not increase certificate size.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalNpoPolynomialProfile {
    pub rows: usize,
    pub log_rows: usize,
    pub sampled_residual_components: usize,
    pub residual_components: usize,
    pub log_residual_components: usize,
    pub witness_input_slots: usize,
    pub witness_output_slots: usize,
    pub hidden_input_slots: usize,
    pub max_serialized_hidden_input_slots: usize,
    pub mmcs_direction_bits: usize,
    pub tip5_rows: usize,
    pub tip5_merkle_rows: usize,
    pub tip5_new_start_rows: usize,
    pub recompose_rows: usize,
    pub recompose_coeff_rows: usize,
    pub max_constraint_degree: usize,
    pub tip5_rounds: usize,
}

impl<F: Field> TerminalLinearExpression<F> {
    pub fn zero() -> Self {
        Self { terms: Vec::new() }
    }

    pub fn one() -> Self {
        Self {
            terms: vec![TerminalLinearTerm {
                coeff: F::ONE,
                variable: TerminalLinearVariable::One,
            }],
        }
    }

    pub fn witness(witness_id: WitnessId) -> Self {
        Self {
            terms: vec![TerminalLinearTerm {
                coeff: F::ONE,
                variable: TerminalLinearVariable::Witness(witness_id),
            }],
        }
    }

    pub fn public(public_pos: usize) -> Self {
        Self {
            terms: vec![TerminalLinearTerm {
                coeff: F::ONE,
                variable: TerminalLinearVariable::Public(public_pos),
            }],
        }
    }

    pub fn constant(value: F) -> Self {
        Self {
            terms: vec![TerminalLinearTerm {
                coeff: value,
                variable: TerminalLinearVariable::One,
            }],
        }
    }

    pub fn scaled(mut self, coeff: F) -> Self {
        for term in &mut self.terms {
            term.coeff *= coeff;
        }
        self
    }

    pub fn plus(mut self, rhs: Self) -> Self {
        self.terms.extend(rhs.terms);
        self
    }

    pub fn minus(self, rhs: Self) -> Self {
        self.plus(rhs.scaled(-F::ONE))
    }

    fn evaluate(
        &self,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<F, NativeTerminalVerifyError> {
        let mut acc = F::ZERO;
        for term in &self.terms {
            let value = match term.variable {
                TerminalLinearVariable::One => F::ONE,
                TerminalLinearVariable::Public(public_pos) => public_inputs
                    .get(public_pos)
                    .copied()
                    .ok_or(NativeTerminalVerifyError::PublicInputLengthMismatch {
                        expected: public_pos + 1,
                        got: public_inputs.len(),
                    })?,
                TerminalLinearVariable::Witness(witness_id) => witness
                    .traces
                    .witness_trace
                    .get_value(witness_id)
                    .copied()
                    .ok_or(NativeTerminalVerifyError::MissingWitness {
                        witness_id: witness_id.0,
                    })?,
            };
            acc += term.coeff * value;
        }
        Ok(acc)
    }
}

impl<F: Field> TerminalQuadraticRelation<F> {
    pub fn residuals(
        &self,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<Vec<F>, NativeTerminalVerifyError> {
        let mut residuals = Vec::with_capacity(self.constraints.len());
        for constraint in &self.constraints {
            let a = constraint.a.evaluate(public_inputs, witness)?;
            let b = constraint.b.evaluate(public_inputs, witness)?;
            let c = constraint.c.evaluate(public_inputs, witness)?;
            residuals.push(a * b - c);
        }
        Ok(residuals)
    }

    pub fn verify(
        &self,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError> {
        for (quadratic_index, (constraint, residual)) in self
            .constraints
            .iter()
            .zip(self.residuals(public_inputs, witness)?)
            .enumerate()
        {
            if residual != F::ZERO {
                return Err(
                    NativeTerminalVerifyError::TerminalQuadraticConstraintViolation {
                        quadratic_index,
                        source_constraint_index: constraint.source_constraint_index,
                        kind: constraint.kind,
                    },
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTerminalNpoCallsite {
    pub inputs: Vec<Option<WitnessId>>,
    pub outputs: Vec<Option<WitnessId>>,
    pub tip5_mode: Option<Tip5TerminalMode>,
    pub tip5_mmcs_bit: Option<WitnessId>,
}

#[derive(Clone, Copy, Debug)]
enum NativeTerminalNpoRowRef<'a> {
    Tip5 {
        op_type: &'a str,
        row: usize,
        callsite: &'a NativeTerminalNpoCallsite,
    },
    Recompose {
        op_type: &'a str,
        row: usize,
        callsite: &'a NativeTerminalNpoCallsite,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeTerminalVerifyError {
    FingerprintMismatch {
        expected: TerminalCircuitFingerprint,
        got: TerminalCircuitFingerprint,
    },
    PublicInputLengthMismatch {
        expected: usize,
        got: usize,
    },
    PrivateInputLengthMismatch {
        expected: usize,
        got: usize,
    },
    WitnessTraceLengthMismatch {
        expected: usize,
        got: usize,
    },
    WitnessTraceIndexMismatch {
        row: usize,
        expected: u32,
        got: u32,
    },
    PublicInputMismatch {
        public_pos: usize,
    },
    MissingWitness {
        witness_id: u32,
    },
    MissingNonPrimitiveTrace {
        op_type: String,
    },
    UnexpectedNonPrimitiveTrace {
        op_type: String,
    },
    NonPrimitiveTraceRowCount {
        op_type: String,
        expected: usize,
        got: usize,
    },
    NonPrimitiveCallsiteMismatch {
        op_type: String,
        row: usize,
        field: &'static str,
        limb: usize,
        expected: Option<u32>,
        got: Option<u32>,
    },
    Tip5TraceInputLength {
        row: usize,
        got: usize,
    },
    Tip5TraceCtlLength {
        row: usize,
        field: &'static str,
        got: usize,
    },
    Tip5InputMismatch {
        row: usize,
        limb: usize,
    },
    Tip5OutputMismatch {
        row: usize,
        limb: usize,
    },
    RecomposeTraceValueLength {
        row: usize,
        expected: usize,
        got: usize,
    },
    RecomposeTraceInputLength {
        row: usize,
        expected: usize,
        got: usize,
    },
    RecomposeTraceKindMismatch {
        op_type: String,
    },
    RecomposeInputMismatch {
        row: usize,
        limb: usize,
    },
    RecomposeOutputMismatch {
        row: usize,
    },
    MissingRelationDigest,
    RelationDigestMismatch {
        expected: TerminalRelationDigest,
        got: TerminalRelationDigest,
    },
    CertificateHeaderMismatch,
    PublicValuesDigestMismatch {
        expected: TerminalPublicValuesDigest,
        got: TerminalPublicValuesDigest,
    },
    ProofBodyDigestMismatch {
        expected: TerminalProofBodyDigest,
        got: TerminalProofBodyDigest,
    },
    BindingDigestMismatch {
        expected: TerminalBindingDigest,
        got: TerminalBindingDigest,
    },
    TerminalLocalProofSerialization {
        reason: String,
    },
    TerminalLocalProofDeserialization {
        reason: String,
    },
    TerminalLocalProofTrailingBytes {
        trailing_len: usize,
    },
    TerminalProofParametersTooWeak {
        requested: u16,
        minimum: u16,
    },
    TerminalProofParametersOverstated {
        declared: u16,
        johnson_bits: u32,
    },
    TerminalProofParametersMismatch {
        expected: u16,
        got: u16,
    },
    TerminalProofProductionParametersMismatch {
        expected: TerminalProofParameters,
        got: TerminalProofParameters,
    },
    TerminalProofQueryPowUnsupported {
        bits: u16,
    },
    TerminalProofQueryPowNonceNonCanonical {
        got: u64,
    },
    TerminalProofKindMismatch {
        expected: TerminalProofKind,
        got: TerminalProofKind,
    },
    TerminalProofQueryDomainTooSmall {
        domain: &'static str,
        len: usize,
        num_queries: usize,
    },
    TerminalProductionProofSerialization {
        reason: String,
    },
    TerminalProductionProofDeserialization {
        reason: String,
    },
    TerminalProductionProofTrailingBytes {
        trailing_len: usize,
    },
    TerminalProductionWitnessBasisLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalProductionProofUnsupported,
    MissingTerminalCommitment,
    TerminalPreludeCommitmentNotBound {
        root: TerminalCommitmentDigest,
    },
    TerminalOracleEmpty,
    TerminalOracleIndexOutOfBounds {
        index: usize,
        values_len: usize,
    },
    TerminalOracleOpeningIndexOutOfBounds {
        index: usize,
        values_len: usize,
    },
    TerminalOracleOpeningPathLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalOracleOpeningRootMismatch {
        expected: TerminalCommitmentDigest,
        got: TerminalCommitmentDigest,
    },
    TerminalOracleOpeningValueMismatch {
        expected: Vec<u64>,
        got: Vec<u64>,
    },
    TerminalOracleCommitmentLabelMismatch {
        expected: String,
        got: String,
    },
    TerminalOracleCommitmentLengthMismatch {
        label: String,
        expected: usize,
        got: usize,
    },
    TerminalOracleQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalOracleQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalQueryOracleEmpty,
    TerminalQueryDerivationLimitExceeded,
    TerminalConstraintQueryDomainEmpty,
    TerminalConstraintQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalConstraintQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoQueryDomainEmpty,
    TerminalNpoQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalNpoQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoOpeningCountMismatch {
        npo_index: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoOpeningWitnessMismatch {
        npo_index: usize,
        opening: usize,
        expected: u32,
        got: usize,
    },
    TerminalNpoTip5InputValueLength {
        npo_index: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoTip5InputValueDimensionMismatch {
        npo_index: usize,
        limb: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoTip5HiddenInputDuplicate {
        npo_index: usize,
        limb: usize,
    },
    TerminalNpoTip5HiddenInputUnexpected {
        npo_index: usize,
        limb: usize,
    },
    TerminalConstraintOpeningCountMismatch {
        constraint_index: usize,
        expected: usize,
        got: usize,
    },
    TerminalConstraintOpeningWitnessMismatch {
        constraint_index: usize,
        opening: usize,
        expected: u32,
        got: usize,
    },
    TerminalOracleOpeningValueDimensionMismatch {
        expected: usize,
        got: usize,
    },
    TerminalOracleOpeningValueNonCanonical {
        limb: usize,
        value: u64,
    },
    TerminalPreludeProfileMismatch {
        expected: TerminalRelationProfile,
        got: TerminalRelationProfile,
    },
    TerminalPreludePublicValuesMismatch {
        expected: TerminalPublicValuesDigest,
        got: TerminalPublicValuesDigest,
    },
    TerminalPreludeChallengeMismatch {
        expected: TerminalTranscriptChallengeDigest,
        got: TerminalTranscriptChallengeDigest,
    },
    TerminalQuadraticConstraintViolation {
        quadratic_index: usize,
        source_constraint_index: usize,
        kind: &'static str,
    },
    TerminalNpoRelationMismatch {
        expected_rows: usize,
        got_rows: usize,
    },
    TerminalQuadraticResidualQueryDomainEmpty,
    TerminalQuadraticResidualDomainLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalQuadraticResidualQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalQuadraticResidualQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalQuadraticResidualNonZero {
        query: usize,
        residual_index: usize,
    },
    TerminalQuadraticConsistencyQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalQuadraticConsistencyQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalQuadraticConsistencyOpeningCountMismatch {
        quadratic_index: usize,
        expected: usize,
        got: usize,
    },
    TerminalQuadraticConsistencyOpeningWitnessMismatch {
        quadratic_index: usize,
        opening: usize,
        expected: u32,
        got: usize,
    },
    TerminalQuadraticConsistencyResidualMismatch {
        query: usize,
        quadratic_index: usize,
    },
    TerminalLocalNpoValidityCommitmentMissing {
        expected_rows: usize,
    },
    TerminalLocalNpoValidityCommitmentUnexpected,
    TerminalLocalNpoValidityProofMissing {
        expected_rows: usize,
    },
    TerminalLocalNpoValidityProofUnexpected,
    TerminalResidualFoldCommitmentLengthMismatch {
        round: usize,
        expected: usize,
        got: usize,
    },
    TerminalResidualFoldCommitmentLabelMismatch {
        round: usize,
        expected: String,
        got: String,
    },
    TerminalResidualFoldQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalResidualFoldQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalResidualFoldRoundCountMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalResidualFoldRoundIndexMismatch {
        query: usize,
        round: usize,
        expected: usize,
        got: usize,
    },
    TerminalResidualFoldOpeningIndexMismatch {
        query: usize,
        round: usize,
        field: &'static str,
        expected: usize,
        got: usize,
    },
    TerminalResidualFoldRightOpeningMissing {
        query: usize,
        round: usize,
        index: usize,
    },
    TerminalResidualFoldRightOpeningUnexpected {
        query: usize,
        round: usize,
        index: usize,
    },
    TerminalResidualFoldRightOpeningPathUnexpected {
        query: usize,
        round: usize,
    },
    TerminalResidualFoldNextOpeningPathUnexpected {
        query: usize,
        round: usize,
    },
    TerminalResidualFoldConsistencyMismatch {
        query: usize,
        round: usize,
    },
    TerminalResidualFoldFinalRootMismatch {
        expected: TerminalCommitmentDigest,
        got: TerminalCommitmentDigest,
    },
    TerminalResidualFoldFinalValueNonZero,
    TerminalNpoValidityDomainLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityResidualMismatch {
        query: usize,
        npo_index: usize,
    },
    TerminalNpoValidityNonZero {
        query: usize,
        npo_index: usize,
    },
    TerminalNpoValidityFoldCommitmentLengthMismatch {
        round: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityFoldCommitmentLabelMismatch {
        round: usize,
        expected: String,
        got: String,
    },
    TerminalNpoValidityFoldQueryLengthMismatch {
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityFoldQueryIndexMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityFoldRoundCountMismatch {
        query: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityFoldRoundIndexMismatch {
        query: usize,
        round: usize,
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityFoldOpeningIndexMismatch {
        query: usize,
        round: usize,
        field: &'static str,
        expected: usize,
        got: usize,
    },
    TerminalNpoValidityFoldRightOpeningMissing {
        query: usize,
        round: usize,
        index: usize,
    },
    TerminalNpoValidityFoldRightOpeningUnexpected {
        query: usize,
        round: usize,
        index: usize,
    },
    TerminalNpoValidityFoldRightOpeningPathUnexpected {
        query: usize,
        round: usize,
    },
    TerminalNpoValidityFoldNextOpeningPathUnexpected {
        query: usize,
        round: usize,
    },
    TerminalNpoValidityFoldConsistencyMismatch {
        query: usize,
        round: usize,
    },
    TerminalNpoValidityFoldFinalRootMismatch {
        expected: TerminalCommitmentDigest,
        got: TerminalCommitmentDigest,
    },
    TerminalNpoValidityFoldFinalValueNonZero,
    ConstraintViolation {
        constraint_index: usize,
        kind: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTerminalProvingKey<F> {
    pub header: TerminalCertificateHeader,
    pub inventory: TerminalOpInventory,
    pub constraints: Vec<NativeTerminalConstraint<F>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTerminalVerifyingKey<F> {
    pub header: TerminalCertificateHeader,
    pub inventory: TerminalOpInventory,
    pub constraints: Vec<NativeTerminalConstraint<F>>,
}

impl<F> NativeTerminalVerifyingKey<F> {
    pub fn relation_profile(&self) -> TerminalRelationProfile {
        self.relation_profile_for_goldilocks_basis_dimension(1)
    }

    fn relation_profile_for_goldilocks_basis_dimension(
        &self,
        basis_dimension: usize,
    ) -> TerminalRelationProfile {
        let mut profile = TerminalRelationProfile {
            fingerprint: self.header.fingerprint,
            primitive_constraints: self.inventory.total_primitive_ops(),
            terminal_constraints: self.constraints.len(),
            hint_ops: self.inventory.hint_ops,
            non_primitive_ops: self.inventory.non_primitive_ops,
            external_npo_validity_components:
                NativeTerminalCompiler::terminal_npo_validity_domain_len_for_basis_dimension(
                    self,
                    basis_dimension,
                ),
            ..TerminalRelationProfile::default()
        };

        for constraint in &self.constraints {
            match constraint {
                NativeTerminalConstraint::Tip5Goldilocks {
                    expected_rows,
                    callsites,
                    ..
                } => {
                    profile.tip5_rows += *expected_rows;
                    Self::add_callsite_slots(&mut profile, callsites);
                }
                NativeTerminalConstraint::RecomposeGoldilocks {
                    op_type,
                    expected_rows,
                    callsites,
                } if op_type == NpoTypeId::recompose().as_str() => {
                    profile.recompose_rows += *expected_rows;
                    Self::add_callsite_slots(&mut profile, callsites);
                }
                NativeTerminalConstraint::RecomposeGoldilocks {
                    expected_rows,
                    callsites,
                    ..
                } => {
                    profile.recompose_coeff_rows += *expected_rows;
                    Self::add_callsite_slots(&mut profile, callsites);
                }
                _ => {}
            }
        }

        profile
    }

    pub fn primitive_sparse_r1cs_relation(
        &self,
    ) -> Result<TerminalSparseR1csRelation<F>, NativeTerminalVerifyError>
    where
        F: Field,
    {
        let relation = self.primitive_quadratic_relation()?;
        Ok(self.sparse_r1cs_relation_from_quadratic(&relation))
    }

    fn sparse_r1cs_relation_from_quadratic(
        &self,
        relation: &TerminalQuadraticRelation<F>,
    ) -> TerminalSparseR1csRelation<F>
    where
        F: Field,
    {
        let rows = relation.constraints.len();
        let public_count = self.header.fingerprint.public_flat_len;
        let witness_count = self.header.fingerprint.witness_count as usize;
        let variables = 1 + public_count + witness_count;
        let mut entries = Vec::new();

        for (row, constraint) in relation.constraints.iter().enumerate() {
            Self::push_sparse_r1cs_expression(
                &mut entries,
                TerminalSparseR1csMatrix::A,
                row,
                public_count,
                &constraint.a,
            );
            Self::push_sparse_r1cs_expression(
                &mut entries,
                TerminalSparseR1csMatrix::B,
                row,
                public_count,
                &constraint.b,
            );
            Self::push_sparse_r1cs_expression(
                &mut entries,
                TerminalSparseR1csMatrix::C,
                row,
                public_count,
                &constraint.c,
            );
        }

        TerminalSparseR1csRelation {
            rows,
            variables,
            public_count,
            witness_count,
            log_rows: NativeTerminalCompiler::terminal_mle_log_size(rows),
            log_variables: NativeTerminalCompiler::terminal_mle_log_size(variables),
            entries,
        }
    }

    fn push_sparse_r1cs_expression(
        entries: &mut Vec<TerminalSparseR1csEntry<F>>,
        matrix: TerminalSparseR1csMatrix,
        row: usize,
        public_count: usize,
        expression: &TerminalLinearExpression<F>,
    ) where
        F: Field,
    {
        for term in &expression.terms {
            let (variable, variable_index) =
                Self::sparse_r1cs_variable_index(public_count, term.variable);
            entries.push(TerminalSparseR1csEntry {
                matrix,
                row,
                variable,
                variable_index,
                coeff: term.coeff,
            });
        }
    }

    fn sparse_r1cs_variable_index(
        public_count: usize,
        variable: TerminalLinearVariable,
    ) -> (TerminalSparseR1csVariable, usize) {
        match variable {
            TerminalLinearVariable::One => (TerminalSparseR1csVariable::One, 0),
            TerminalLinearVariable::Public(public_pos) => (
                TerminalSparseR1csVariable::Public(public_pos),
                1 + public_pos,
            ),
            TerminalLinearVariable::Witness(witness_id) => (
                TerminalSparseR1csVariable::Witness(witness_id),
                1 + public_count + witness_id.0 as usize,
            ),
        }
    }

    pub fn primitive_quadratic_relation(
        &self,
    ) -> Result<TerminalQuadraticRelation<F>, NativeTerminalVerifyError>
    where
        F: Field,
    {
        let mut relation = TerminalQuadraticRelation {
            constraints: Vec::new(),
            external_npo_rows: NativeTerminalCompiler::terminal_npo_domain_len(self),
        };

        let one = TerminalLinearExpression::one;
        let zero = TerminalLinearExpression::zero;
        let witness = TerminalLinearExpression::witness;
        let public = TerminalLinearExpression::public;
        let constant = TerminalLinearExpression::constant;

        for (source_constraint_index, constraint) in self.constraints.iter().enumerate() {
            let mut push = |kind: &'static str,
                            a: TerminalLinearExpression<F>,
                            b: TerminalLinearExpression<F>,
                            c: TerminalLinearExpression<F>| {
                relation.constraints.push(TerminalQuadraticConstraint {
                    source_constraint_index,
                    kind,
                    a,
                    b,
                    c,
                });
            };
            match constraint {
                NativeTerminalConstraint::Const { out, val } => {
                    push("const", witness(*out).minus(constant(*val)), one(), zero());
                }
                NativeTerminalConstraint::Public { out, public_pos } => {
                    push(
                        "public",
                        witness(*out).minus(public(*public_pos)),
                        one(),
                        zero(),
                    );
                }
                NativeTerminalConstraint::Alu {
                    kind,
                    a,
                    b,
                    c,
                    out,
                    intermediate_out,
                } => match kind {
                    AluOpKind::Add => {
                        push(
                            "alu_add",
                            witness(*a).plus(witness(*b)).minus(witness(*out)),
                            one(),
                            zero(),
                        );
                    }
                    AluOpKind::Mul => {
                        push("alu_mul", witness(*a), witness(*b), witness(*out));
                    }
                    AluOpKind::BoolCheck => {
                        push(
                            "alu_bool_check",
                            witness(*a),
                            witness(*a).minus(one()),
                            zero(),
                        );
                        push(
                            "alu_bool_output",
                            witness(*out).minus(witness(*a)),
                            one(),
                            zero(),
                        );
                    }
                    AluOpKind::MulAdd => {
                        let c_expr = c.map(witness).unwrap_or_else(zero);
                        push(
                            "alu_mul_add",
                            witness(*a),
                            witness(*b),
                            witness(*out).minus(c_expr),
                        );
                    }
                    AluOpKind::HornerAcc => {
                        let Some(acc) = intermediate_out else {
                            return Err(NativeTerminalVerifyError::ConstraintViolation {
                                constraint_index: source_constraint_index,
                                kind: "horner_acc_missing_acc",
                            });
                        };
                        let c_expr = c.map(witness).unwrap_or_else(zero);
                        push(
                            "alu_horner_acc",
                            witness(*acc),
                            witness(*b),
                            witness(*a).plus(witness(*out)).minus(c_expr),
                        );
                    }
                },
                NativeTerminalConstraint::Tip5Goldilocks { .. }
                | NativeTerminalConstraint::RecomposeGoldilocks { .. } => {}
            }
        }

        Ok(relation)
    }

    pub fn npo_relation(&self) -> TerminalNpoRelation {
        let mut rows = Vec::with_capacity(NativeTerminalCompiler::terminal_npo_domain_len(self));
        for constraint in &self.constraints {
            match constraint {
                NativeTerminalConstraint::Tip5Goldilocks {
                    op_type, callsites, ..
                } => {
                    for (local_row, callsite) in callsites.iter().enumerate() {
                        rows.push(TerminalNpoRelationRow {
                            npo_index: rows.len(),
                            op_type: op_type.clone(),
                            local_row,
                            kind: TerminalNpoRowKind::Tip5Goldilocks,
                            callsite: callsite.clone(),
                        });
                    }
                }
                NativeTerminalConstraint::RecomposeGoldilocks {
                    op_type, callsites, ..
                } => {
                    let kind = if op_type == NpoTypeId::recompose().as_str() {
                        TerminalNpoRowKind::Recompose
                    } else {
                        TerminalNpoRowKind::RecomposeWithCoeffLookups
                    };
                    for (local_row, callsite) in callsites.iter().enumerate() {
                        rows.push(TerminalNpoRelationRow {
                            npo_index: rows.len(),
                            op_type: op_type.clone(),
                            local_row,
                            kind: kind.clone(),
                            callsite: callsite.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
        TerminalNpoRelation { rows }
    }

    fn add_callsite_slots(
        profile: &mut TerminalRelationProfile,
        callsites: &[NativeTerminalNpoCallsite],
    ) {
        for callsite in callsites {
            profile.npo_callsite_input_slots += callsite.inputs.len();
            profile.npo_callsite_output_slots += callsite.outputs.len();
        }
    }
}

/// Tip5 Merkle tree over a terminal oracle vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalOracleMerkleTree {
    label: String,
    values_len: usize,
    levels: Vec<Vec<TerminalCommitmentDigest>>,
}

impl TerminalOracleMerkleTree {
    pub fn commit_goldilocks_values<F>(
        label: impl Into<String>,
        values: &[F],
    ) -> Result<Self, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        if values.is_empty() {
            return Err(NativeTerminalVerifyError::TerminalOracleEmpty);
        }
        let label = label.into();
        let mut leaves: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                NativeTerminalCompiler::terminal_oracle_leaf_digest(
                    &label,
                    values.len(),
                    index,
                    value,
                )
            })
            .collect();
        let padded_len = leaves.len().next_power_of_two();
        let empty_leaf =
            NativeTerminalCompiler::terminal_oracle_empty_leaf_digest(&label, values.len());
        leaves.resize(padded_len, empty_leaf);

        let mut levels = vec![leaves];
        while levels.last().expect("level exists").len() > 1 {
            let prev = levels.last().expect("level exists");
            let mut next = Vec::with_capacity(prev.len() / 2);
            for pair in prev.chunks_exact(2) {
                next.push(NativeTerminalCompiler::terminal_oracle_node_digest(
                    &label, pair[0], pair[1],
                ));
            }
            levels.push(next);
        }

        Ok(Self {
            label,
            values_len: values.len(),
            levels,
        })
    }

    pub fn commitment(&self) -> TerminalOracleCommitment {
        TerminalOracleCommitment {
            label: self.label.clone(),
            values_len: self.values_len,
            root: self.root(),
        }
    }

    pub fn root(&self) -> TerminalCommitmentDigest {
        self.levels
            .last()
            .and_then(|level| level.first())
            .copied()
            .expect("terminal oracle tree has a root")
    }

    pub fn open_goldilocks_value<F>(
        &self,
        index: usize,
        value: &F,
    ) -> Result<TerminalOracleOpening, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        if index >= self.values_len {
            return Err(NativeTerminalVerifyError::TerminalOracleIndexOutOfBounds {
                index,
                values_len: self.values_len,
            });
        }

        let mut path = Vec::with_capacity(self.levels.len().saturating_sub(1));
        let mut node_index = index;
        for level in &self.levels[..self.levels.len().saturating_sub(1)] {
            let sibling_index = node_index ^ 1;
            path.push(TerminalMerkleSibling {
                digest: level[sibling_index],
                sibling_is_left: sibling_index < node_index,
            });
            node_index /= 2;
        }

        Ok(TerminalOracleOpening {
            index,
            value_basis: NativeTerminalCompiler::goldilocks_basis_u64(value),
            path,
        })
    }

    pub fn open_goldilocks_multi_values<F>(
        &self,
        values: &[(usize, &F)],
    ) -> Result<TerminalOracleMultiProof, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let mut previous = None;
        for (opening, (index, _)) in values.iter().enumerate() {
            if *index >= self.values_len {
                return Err(NativeTerminalVerifyError::TerminalOracleIndexOutOfBounds {
                    index: *index,
                    values_len: self.values_len,
                });
            }
            if previous.is_some_and(|previous| *index <= previous) {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query: opening,
                        expected: previous.expect("previous index exists") + 1,
                        got: *index,
                    },
                );
            }
            previous = Some(*index);
        }

        let openings = values
            .iter()
            .map(|(index, value)| TerminalOracleMultiValueOpening {
                index: *index,
                value_basis: NativeTerminalCompiler::goldilocks_basis_u64(*value),
            })
            .collect();
        let mut frontier = Vec::new();
        let root_level = self.levels.len().saturating_sub(1);
        self.collect_multi_frontier(root_level, 0, values, &mut frontier);
        Ok(TerminalOracleMultiProof { openings, frontier })
    }

    pub fn open_goldilocks_known_index_multi_values<F>(
        &self,
        values: &[(usize, &F)],
    ) -> Result<TerminalOracleKnownIndexMultiProof, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        self.open_goldilocks_known_index_multi_values_with_boolean_indices(values, &[])
    }

    pub fn open_goldilocks_known_index_multi_values_with_boolean_indices<F>(
        &self,
        values: &[(usize, &F)],
        boolean_indices: &[usize],
    ) -> Result<TerminalOracleKnownIndexMultiProof, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let mut previous = None;
        for (opening, (index, _)) in values.iter().enumerate() {
            if *index >= self.values_len {
                return Err(NativeTerminalVerifyError::TerminalOracleIndexOutOfBounds {
                    index: *index,
                    values_len: self.values_len,
                });
            }
            if previous.is_some_and(|previous| *index <= previous) {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query: opening,
                        expected: previous.expect("previous index exists") + 1,
                        got: *index,
                    },
                );
            }
            previous = Some(*index);
        }
        let mut previous_boolean = None;
        for (opening, index) in boolean_indices.iter().copied().enumerate() {
            if previous_boolean.is_some_and(|previous| index <= previous) {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query: opening,
                        expected: previous_boolean.expect("previous index exists") + 1,
                        got: index,
                    },
                );
            }
            if values
                .binary_search_by_key(&index, |(index, _)| *index)
                .is_err()
            {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                        index,
                        values_len: self.values_len,
                    },
                );
            }
            previous_boolean = Some(index);
        }

        let mut value_basis_flat = Vec::with_capacity(
            values.len().saturating_sub(boolean_indices.len())
                * <F as BasedVectorSpace<Goldilocks>>::DIMENSION,
        );
        let mut boolean_value_bits = vec![0u8; boolean_indices.len().div_ceil(8)];
        let mut boolean_index = 0usize;
        for (index, value) in values {
            let basis = NativeTerminalCompiler::goldilocks_basis_u64(*value);
            if boolean_index < boolean_indices.len() && *index == boolean_indices[boolean_index] {
                if basis.first().copied().unwrap_or(2) > 1 || basis.iter().skip(1).any(|v| *v != 0)
                {
                    return Err(
                        NativeTerminalVerifyError::TerminalOracleOpeningValueNonCanonical {
                            limb: 0,
                            value: basis.first().copied().unwrap_or(2),
                        },
                    );
                }
                if basis[0] == 1 {
                    boolean_value_bits[boolean_index / 8] |= 1u8 << (boolean_index % 8);
                }
                boolean_index += 1;
            } else {
                value_basis_flat.extend(basis);
            }
        }

        let mut frontier = Vec::new();
        let root_level = self.levels.len().saturating_sub(1);
        self.collect_multi_frontier(root_level, 0, values, &mut frontier);
        Ok(TerminalOracleKnownIndexMultiProof {
            value_basis_flat,
            boolean_value_bits,
            frontier,
        })
    }

    pub fn open_goldilocks_prefix<F>(
        &self,
        values: &[F],
    ) -> Result<TerminalOraclePrefixProof, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        if values.len() > self.values_len {
            return Err(NativeTerminalVerifyError::TerminalOracleIndexOutOfBounds {
                index: values.len(),
                values_len: self.values_len,
            });
        }
        let mut frontier = Vec::new();
        let root_level = self.levels.len().saturating_sub(1);
        self.collect_prefix_frontier(root_level, 0, values.len(), &mut frontier);
        Ok(TerminalOraclePrefixProof {
            prefix_len: values.len(),
            frontier,
        })
    }

    fn collect_prefix_frontier(
        &self,
        level: usize,
        start: usize,
        prefix_len: usize,
        frontier: &mut Vec<TerminalCommitmentDigest>,
    ) {
        let size = 1usize << level;
        if start >= prefix_len {
            frontier.push(self.levels[level][start >> level]);
            return;
        }
        if start + size <= prefix_len || level == 0 {
            return;
        }
        let half = size / 2;
        self.collect_prefix_frontier(level - 1, start, prefix_len, frontier);
        self.collect_prefix_frontier(level - 1, start + half, prefix_len, frontier);
    }

    fn collect_multi_frontier<F>(
        &self,
        level: usize,
        start: usize,
        values: &[(usize, &F)],
        frontier: &mut Vec<TerminalCommitmentDigest>,
    ) {
        let size = 1usize << level;
        if !values
            .iter()
            .any(|(index, _)| *index >= start && *index < start + size)
        {
            frontier.push(self.levels[level][start >> level]);
            return;
        }
        if level == 0 {
            return;
        }
        let half = size / 2;
        self.collect_multi_frontier(level - 1, start, values, frontier);
        self.collect_multi_frontier(level - 1, start + half, values, frontier);
    }
}

/// Native terminal compiler checkpoint.
///
/// This compiler currently accepts primitive p3-circuit operations plus hints,
/// recursive 5-round Goldilocks Tip5, and Goldilocks-base recompose NPO
/// relations. Hints are witness-generation metadata and do not add constraints
/// in the existing p3-circuit semantics. Other table-backed non-primitive
/// operations are rejected until their constraints are ported into the compact
/// terminal arithmetization.
#[derive(Clone, Debug)]
pub struct NativeTerminalCompiler {
    protocol_id: &'static str,
    security_bits: u16,
}

impl NativeTerminalCompiler {
    pub const fn new(protocol_id: &'static str, security_bits: u16) -> Self {
        Self {
            protocol_id,
            security_bits,
        }
    }

    pub fn analyze<F: Field>(&self, circuit: &Circuit<F>) -> TerminalOpInventory {
        let mut inventory = TerminalOpInventory::default();
        for op in &circuit.ops {
            match op {
                Op::Const { .. } => inventory.const_ops += 1,
                Op::Public { .. } => inventory.public_ops += 1,
                Op::Alu { kind, .. } => match kind {
                    AluOpKind::Add => inventory.alu_add_ops += 1,
                    AluOpKind::Mul => inventory.alu_mul_ops += 1,
                    AluOpKind::BoolCheck => inventory.alu_bool_check_ops += 1,
                    AluOpKind::MulAdd => inventory.alu_mul_add_ops += 1,
                    AluOpKind::HornerAcc => inventory.alu_horner_acc_ops += 1,
                },
                Op::Hint { .. } => inventory.hint_ops += 1,
                Op::NonPrimitiveOpWithExecutor { executor, .. } => {
                    inventory.non_primitive_ops += 1;
                    let op_type = executor.op_type().as_str();
                    if !inventory.non_primitive_types.iter().any(|t| t == op_type) {
                        inventory.non_primitive_types.push(op_type.into());
                    }
                }
            }
        }
        inventory.non_primitive_types.sort();
        inventory
    }

    fn terminal_mle_log_size(size: usize) -> usize {
        if size <= 1 {
            return 0;
        }
        size.next_power_of_two().trailing_zeros() as usize
    }

    pub fn compile_primitive_terminal<F: Field>(
        &self,
        circuit: &Circuit<F>,
    ) -> Result<
        (NativeTerminalProvingKey<F>, NativeTerminalVerifyingKey<F>),
        NativeTerminalCompileError,
    > {
        if self.security_bits < MIN_TERMINAL_SECURITY_BITS {
            return Err(NativeTerminalCompileError::SecurityBitsTooLow {
                requested: self.security_bits,
                minimum: MIN_TERMINAL_SECURITY_BITS,
            });
        }

        for (op_index, op) in circuit.ops.iter().enumerate() {
            if let Op::NonPrimitiveOpWithExecutor {
                inputs,
                outputs,
                executor,
                ..
            } = op
            {
                if !Self::is_supported_non_primitive_op(executor.op_type()) {
                    return Err(NativeTerminalCompileError::UnsupportedNonPrimitive {
                        op_index,
                        op_type: executor.op_type().as_str().into(),
                    });
                }
                Self::validate_supported_nonprimitive_layout(
                    op_index,
                    executor.op_type(),
                    inputs,
                    outputs,
                )?;
            }
        }

        let constraints = self.compile_primitive_constraints(circuit);
        let header = TerminalCertificateHeader {
            version: 1,
            protocol_id: self.protocol_id.into(),
            security_bits: self.security_bits,
            fingerprint: TerminalCircuitFingerprint::from_circuit(circuit),
            relation_digest: None,
        };
        let inventory = self.analyze(circuit);
        let proving_key = NativeTerminalProvingKey {
            header: header.clone(),
            inventory: inventory.clone(),
            constraints: constraints.clone(),
        };
        let verifying_key = NativeTerminalVerifyingKey {
            header,
            inventory,
            constraints,
        };
        Ok((proving_key, verifying_key))
    }

    pub fn compile_goldilocks_terminal<F>(
        &self,
        circuit: &Circuit<F>,
    ) -> Result<
        (NativeTerminalProvingKey<F>, NativeTerminalVerifyingKey<F>),
        NativeTerminalCompileError,
    >
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let (mut proving_key, mut verifying_key) = self.compile_primitive_terminal(circuit)?;
        let relation_digest = Self::relation_digest_goldilocks(&verifying_key);
        proving_key.header.relation_digest = Some(relation_digest);
        verifying_key.header.relation_digest = Some(relation_digest);
        Ok((proving_key, verifying_key))
    }

    pub fn backend_relation_digest_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
    ) -> TerminalBackendRelationDigest
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        Self::backend_relation_digest_goldilocks_for_key(verifying_key)
    }

    fn assemble_goldilocks_certificate<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        proof_kind: TerminalProofKind,
        public_inputs: &[F],
        proof_body: Vec<u8>,
    ) -> Result<TerminalCertificate, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_relation_digest_goldilocks(verifying_key)?;
        if public_inputs.len() != verifying_key.header.fingerprint.public_flat_len {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: verifying_key.header.fingerprint.public_flat_len,
                got: public_inputs.len(),
            });
        }

        let header = verifying_key.header.clone();
        let public_values_digest = Self::public_values_digest_goldilocks(public_inputs);
        let proof_body_digest = Self::proof_body_digest(&proof_body);
        let binding_digest =
            Self::binding_digest(&header, proof_kind, public_values_digest, proof_body_digest);

        Ok(TerminalCertificate {
            header,
            proof_kind,
            public_values_digest,
            proof_body_digest,
            binding_digest,
            proof_body,
        })
    }

    fn verify_certificate_binding_goldilocks<'a, F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        certificate: &'a TerminalCertificate,
        expected_kind: TerminalProofKind,
        public_inputs: &[F],
    ) -> Result<&'a [u8], NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_relation_digest_goldilocks(verifying_key)?;
        if certificate.header != verifying_key.header {
            return Err(NativeTerminalVerifyError::CertificateHeaderMismatch);
        }
        if certificate.proof_kind != expected_kind {
            return Err(NativeTerminalVerifyError::TerminalProofKindMismatch {
                expected: expected_kind,
                got: certificate.proof_kind,
            });
        }
        if public_inputs.len() != verifying_key.header.fingerprint.public_flat_len {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: verifying_key.header.fingerprint.public_flat_len,
                got: public_inputs.len(),
            });
        }

        let got_public = Self::public_values_digest_goldilocks(public_inputs);
        if got_public != certificate.public_values_digest {
            return Err(NativeTerminalVerifyError::PublicValuesDigestMismatch {
                expected: certificate.public_values_digest,
                got: got_public,
            });
        }

        let got_body = Self::proof_body_digest(&certificate.proof_body);
        if got_body != certificate.proof_body_digest {
            return Err(NativeTerminalVerifyError::ProofBodyDigestMismatch {
                expected: certificate.proof_body_digest,
                got: got_body,
            });
        }

        let got_binding = Self::binding_digest(
            &certificate.header,
            certificate.proof_kind,
            got_public,
            got_body,
        );
        if got_binding != certificate.binding_digest {
            return Err(NativeTerminalVerifyError::BindingDigestMismatch {
                expected: certificate.binding_digest,
                got: got_binding,
            });
        }

        Ok(&certificate.proof_body)
    }

    pub fn verify_goldilocks_production_certificate<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        certificate: &TerminalCertificate,
        public_inputs: &[F],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let proof_body = self.verify_certificate_binding_goldilocks(
            verifying_key,
            certificate,
            TerminalProofKind::Production,
            public_inputs,
        )?;
        self.validate_goldilocks_production_query_domains(
            verifying_key,
            TerminalProofParameters::production_60bit(),
        )?;
        let (proof, trailing): (TerminalProductionProof, &[u8]) =
            postcard::take_from_bytes(proof_body).map_err(|err| {
                NativeTerminalVerifyError::TerminalProductionProofDeserialization {
                    reason: format!("{err:?}"),
                }
            })?;
        if !trailing.is_empty() {
            return Err(
                NativeTerminalVerifyError::TerminalProductionProofTrailingBytes {
                    trailing_len: trailing.len(),
                },
            );
        }
        self.verify_terminal_production_goldilocks(verifying_key, public_inputs, &proof)
    }

    pub fn assemble_goldilocks_production_certificate<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        proof: &TerminalProductionProof,
    ) -> Result<TerminalCertificate, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_terminal_production_goldilocks(verifying_key, public_inputs, proof)?;
        let proof_body = postcard::to_allocvec(proof).map_err(|err| {
            NativeTerminalVerifyError::TerminalProductionProofSerialization {
                reason: format!("{err:?}"),
            }
        })?;
        self.assemble_goldilocks_certificate(
            verifying_key,
            TerminalProofKind::Production,
            public_inputs,
            proof_body,
        )
    }

    #[cfg(test)]
    fn assemble_goldilocks_local_certificate<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        proof: &TerminalLocalProof,
    ) -> Result<TerminalCertificate, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_terminal_local_queries_goldilocks(verifying_key, public_inputs, proof)?;
        let proof_body = postcard::to_allocvec(proof).map_err(|err| {
            NativeTerminalVerifyError::TerminalLocalProofSerialization {
                reason: format!("{err:?}"),
            }
        })?;
        self.assemble_goldilocks_certificate(
            verifying_key,
            TerminalProofKind::LocalCheckpoint,
            public_inputs,
            proof_body,
        )
    }

    #[cfg(test)]
    fn verify_goldilocks_local_certificate<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        certificate: &TerminalCertificate,
        public_inputs: &[F],
    ) -> Result<TerminalLocalProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let proof_body = self.verify_certificate_binding_goldilocks(
            verifying_key,
            certificate,
            TerminalProofKind::LocalCheckpoint,
            public_inputs,
        )?;
        let (proof, trailing): (TerminalLocalProof, &[u8]) = postcard::take_from_bytes(proof_body)
            .map_err(
                |err| NativeTerminalVerifyError::TerminalLocalProofDeserialization {
                    reason: format!("{err:?}"),
                },
            )?;
        if !trailing.is_empty() {
            return Err(NativeTerminalVerifyError::TerminalLocalProofTrailingBytes {
                trailing_len: trailing.len(),
            });
        }
        self.verify_terminal_local_queries_goldilocks(verifying_key, public_inputs, &proof)?;
        Ok(proof)
    }

    pub fn build_proof_prelude_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        parameters: TerminalProofParameters,
        commitments: Vec<TerminalCommitmentDigest>,
    ) -> Result<TerminalProofPrelude, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_relation_digest_goldilocks(verifying_key)?;
        self.validate_terminal_proof_parameters(verifying_key, parameters)?;
        if commitments.is_empty() {
            return Err(NativeTerminalVerifyError::MissingTerminalCommitment);
        }
        if public_inputs.len() != verifying_key.header.fingerprint.public_flat_len {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: verifying_key.header.fingerprint.public_flat_len,
                got: public_inputs.len(),
            });
        }

        let relation_profile = verifying_key.relation_profile_for_goldilocks_basis_dimension(
            <F as BasedVectorSpace<Goldilocks>>::DIMENSION,
        );
        let public_values_digest = Self::public_values_digest_goldilocks(public_inputs);
        let query_pow_nonce = 0;
        let challenge_digest = Self::transcript_challenge_digest(
            &verifying_key.header,
            parameters,
            &relation_profile,
            public_values_digest,
            &commitments,
            query_pow_nonce,
        );

        Ok(TerminalProofPrelude {
            parameters,
            relation_profile,
            public_values_digest,
            commitments,
            query_pow_nonce,
            challenge_digest,
        })
    }

    pub fn verify_proof_prelude_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_relation_digest_goldilocks(verifying_key)?;
        self.validate_terminal_proof_parameters(verifying_key, prelude.parameters)?;
        if prelude.commitments.is_empty() {
            return Err(NativeTerminalVerifyError::MissingTerminalCommitment);
        }
        if public_inputs.len() != verifying_key.header.fingerprint.public_flat_len {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: verifying_key.header.fingerprint.public_flat_len,
                got: public_inputs.len(),
            });
        }

        let expected_profile = verifying_key.relation_profile_for_goldilocks_basis_dimension(
            <F as BasedVectorSpace<Goldilocks>>::DIMENSION,
        );
        if prelude.relation_profile != expected_profile {
            return Err(NativeTerminalVerifyError::TerminalPreludeProfileMismatch {
                expected: expected_profile,
                got: prelude.relation_profile.clone(),
            });
        }

        let got_public = Self::public_values_digest_goldilocks(public_inputs);
        if prelude.public_values_digest != got_public {
            return Err(
                NativeTerminalVerifyError::TerminalPreludePublicValuesMismatch {
                    expected: prelude.public_values_digest,
                    got: got_public,
                },
            );
        }

        let got_challenge = Self::transcript_challenge_digest(
            &verifying_key.header,
            prelude.parameters,
            &prelude.relation_profile,
            got_public,
            &prelude.commitments,
            prelude.query_pow_nonce,
        );
        if prelude.challenge_digest != got_challenge {
            return Err(
                NativeTerminalVerifyError::TerminalPreludeChallengeMismatch {
                    expected: prelude.challenge_digest,
                    got: got_challenge,
                },
            );
        }
        if prelude.query_pow_nonce != 0 {
            return Err(
                NativeTerminalVerifyError::TerminalProofQueryPowNonceNonCanonical {
                    got: prelude.query_pow_nonce,
                },
            );
        }

        Ok(())
    }

    pub fn verify_terminal_oracle_opening(
        &self,
        commitment: &TerminalOracleCommitment,
        opening: &TerminalOracleOpening,
    ) -> Result<(), NativeTerminalVerifyError> {
        if commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalOracleEmpty);
        }
        if opening.index >= commitment.values_len {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                    index: opening.index,
                    values_len: commitment.values_len,
                },
            );
        }

        let expected_path_len = Self::terminal_oracle_path_len(commitment.values_len);
        if opening.path.len() != expected_path_len {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch {
                    expected: expected_path_len,
                    got: opening.path.len(),
                },
            );
        }

        let mut digest = Self::terminal_oracle_leaf_digest_from_basis(
            &commitment.label,
            commitment.values_len,
            opening.index,
            &opening.value_basis,
        );
        for sibling in &opening.path {
            digest = if sibling.sibling_is_left {
                Self::terminal_oracle_node_digest(&commitment.label, sibling.digest, digest)
            } else {
                Self::terminal_oracle_node_digest(&commitment.label, digest, sibling.digest)
            };
        }
        if digest != commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch {
                    expected: commitment.root,
                    got: digest,
                },
            );
        }
        Ok(())
    }

    pub fn verify_terminal_oracle_opening_value_goldilocks<F>(
        &self,
        commitment: &TerminalOracleCommitment,
        opening: &TerminalOracleOpening,
        expected_value: &F,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let expected_basis = Self::goldilocks_basis_u64(expected_value);
        if opening.value_basis != expected_basis {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningValueMismatch {
                    expected: expected_basis,
                    got: opening.value_basis.clone(),
                },
            );
        }
        self.verify_terminal_oracle_opening(commitment, opening)
    }

    pub fn verify_terminal_oracle_multi_proof_goldilocks<F>(
        &self,
        commitment: &TerminalOracleCommitment,
        proof: &TerminalOracleMultiProof,
    ) -> Result<Vec<(usize, F)>, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        if commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalOracleEmpty);
        }

        let mut previous = None;
        let mut values = Vec::with_capacity(proof.openings.len());
        for (opening_idx, opening) in proof.openings.iter().enumerate() {
            if opening.index >= commitment.values_len {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                        index: opening.index,
                        values_len: commitment.values_len,
                    },
                );
            }
            if previous.is_some_and(|previous| opening.index <= previous) {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query: opening_idx,
                        expected: previous.expect("previous opening exists") + 1,
                        got: opening.index,
                    },
                );
            }
            values.push((
                opening.index,
                Self::field_from_goldilocks_basis_u64::<F>(&opening.value_basis)?,
            ));
            previous = Some(opening.index);
        }

        let root_level = Self::terminal_oracle_path_len(commitment.values_len);
        let mut frontier = proof.frontier.iter().copied();
        let got = Self::terminal_oracle_multi_root_goldilocks(
            &commitment.label,
            commitment.values_len,
            &proof.openings,
            root_level,
            0,
            &mut frontier,
        )?;
        let remaining = frontier.count();
        if remaining != 0 {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch {
                    expected: proof.frontier.len() - remaining,
                    got: proof.frontier.len(),
                },
            );
        }
        if got != commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch {
                    expected: commitment.root,
                    got,
                },
            );
        }
        Ok(values)
    }

    pub fn verify_terminal_oracle_known_index_multi_proof_goldilocks<F>(
        &self,
        commitment: &TerminalOracleCommitment,
        indices: &[usize],
        proof: &TerminalOracleKnownIndexMultiProof,
    ) -> Result<Vec<(usize, F)>, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        self.verify_terminal_oracle_known_index_multi_proof_goldilocks_with_boolean_indices(
            commitment,
            indices,
            &[],
            proof,
        )
    }

    pub fn verify_terminal_oracle_known_index_multi_proof_goldilocks_with_boolean_indices<F>(
        &self,
        commitment: &TerminalOracleCommitment,
        indices: &[usize],
        boolean_indices: &[usize],
        proof: &TerminalOracleKnownIndexMultiProof,
    ) -> Result<Vec<(usize, F)>, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        if commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalOracleEmpty);
        }

        let mut previous = None;
        for (query, index) in indices.iter().copied().enumerate() {
            if index >= commitment.values_len {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                        index,
                        values_len: commitment.values_len,
                    },
                );
            }
            if previous.is_some_and(|previous| index <= previous) {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query,
                        expected: previous.expect("previous opening exists") + 1,
                        got: index,
                    },
                );
            }
            previous = Some(index);
        }
        let mut previous_boolean = None;
        for (query, index) in boolean_indices.iter().copied().enumerate() {
            if previous_boolean.is_some_and(|previous| index <= previous) {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query,
                        expected: previous_boolean.expect("previous opening exists") + 1,
                        got: index,
                    },
                );
            }
            if indices.binary_search(&index).is_err() {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                        index,
                        values_len: commitment.values_len,
                    },
                );
            }
            previous_boolean = Some(index);
        }

        let dimension = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        let expected_basis_len = indices.len().saturating_sub(boolean_indices.len()) * dimension;
        if proof.value_basis_flat.len() != expected_basis_len {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningValueDimensionMismatch {
                    expected: expected_basis_len,
                    got: proof.value_basis_flat.len(),
                },
            );
        }
        let expected_boolean_bytes = boolean_indices.len().div_ceil(8);
        if proof.boolean_value_bits.len() != expected_boolean_bytes {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningValueDimensionMismatch {
                    expected: expected_boolean_bytes,
                    got: proof.boolean_value_bits.len(),
                },
            );
        }
        if let Some(last_byte) = proof.boolean_value_bits.last().copied() {
            let used_bits = boolean_indices.len() % 8;
            if used_bits != 0 {
                let unused_mask = u8::MAX << used_bits;
                if last_byte & unused_mask != 0 {
                    return Err(
                        NativeTerminalVerifyError::TerminalOracleOpeningValueNonCanonical {
                            limb: boolean_indices.len(),
                            value: last_byte as u64,
                        },
                    );
                }
            }
        }

        let mut values = Vec::with_capacity(indices.len());
        let mut openings = Vec::with_capacity(indices.len());
        let mut basis_chunks = proof.value_basis_flat.chunks_exact(dimension);
        let mut boolean_index = 0usize;
        for index in indices.iter().copied() {
            let value_basis = if boolean_index < boolean_indices.len()
                && index == boolean_indices[boolean_index]
            {
                let bit = (proof.boolean_value_bits[boolean_index / 8] >> (boolean_index % 8)) & 1;
                boolean_index += 1;
                let mut basis = vec![0u64; dimension];
                basis[0] = bit as u64;
                basis
            } else {
                basis_chunks
                    .next()
                    .ok_or(
                        NativeTerminalVerifyError::TerminalOracleOpeningValueDimensionMismatch {
                            expected: expected_basis_len,
                            got: proof.value_basis_flat.len(),
                        },
                    )?
                    .to_vec()
            };
            values.push((
                index,
                Self::field_from_goldilocks_basis_u64::<F>(&value_basis)?,
            ));
            openings.push(TerminalOracleMultiValueOpening { index, value_basis });
        }

        let root_level = Self::terminal_oracle_path_len(commitment.values_len);
        let mut frontier = proof.frontier.iter().copied();
        let got = Self::terminal_oracle_multi_root_goldilocks(
            &commitment.label,
            commitment.values_len,
            &openings,
            root_level,
            0,
            &mut frontier,
        )?;
        let remaining = frontier.count();
        if remaining != 0 {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch {
                    expected: proof.frontier.len() - remaining,
                    got: proof.frontier.len(),
                },
            );
        }
        if got != commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch {
                    expected: commitment.root,
                    got,
                },
            );
        }
        Ok(values)
    }

    pub fn verify_terminal_oracle_prefix_goldilocks<F>(
        &self,
        commitment: &TerminalOracleCommitment,
        proof: &TerminalOraclePrefixProof,
        expected_values: &[F],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        if proof.prefix_len != expected_values.len() {
            return Err(
                NativeTerminalVerifyError::TerminalConstraintOpeningCountMismatch {
                    constraint_index: 0,
                    expected: expected_values.len(),
                    got: proof.prefix_len,
                },
            );
        }
        if proof.prefix_len > commitment.values_len {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                    index: proof.prefix_len,
                    values_len: commitment.values_len,
                },
            );
        }

        let root_level = Self::terminal_oracle_path_len(commitment.values_len);
        let mut frontier = proof.frontier.iter().copied();
        let got = Self::terminal_oracle_prefix_root_goldilocks(
            &commitment.label,
            commitment.values_len,
            expected_values,
            proof.prefix_len,
            root_level,
            0,
            &mut frontier,
        )?;
        let remaining = frontier.count();
        if remaining != 0 {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch {
                    expected: proof.frontier.len() - remaining,
                    got: proof.frontier.len(),
                },
            );
        }
        if got != commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch {
                    expected: commitment.root,
                    got,
                },
            );
        }
        Ok(())
    }

    pub fn derive_terminal_query_plan(
        &self,
        prelude: &TerminalProofPrelude,
        commitment: &TerminalOracleCommitment,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError> {
        if commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalQueryOracleEmpty);
        }

        let num_queries = prelude.parameters.num_queries as usize;
        let mut indices = Vec::with_capacity(num_queries);
        let mut counter = 0u64;
        while indices.len() < num_queries {
            if counter
                > (num_queries as u64)
                    .saturating_mul(4096)
                    .saturating_add(4096)
            {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_query_block(prelude, commitment, counter);
            for limb in block {
                if indices.len() == num_queries {
                    break;
                }
                let bound = commitment.values_len as u64;
                let zone = u64::MAX - (u64::MAX % bound);
                if limb < zone {
                    Self::accept_terminal_query_index(
                        &mut indices,
                        (limb % bound) as usize,
                        commitment.values_len,
                        num_queries,
                    );
                }
            }
            counter += 1;
        }

        Ok(TerminalQueryPlan {
            oracle_label: commitment.label.clone(),
            oracle_len: commitment.values_len,
            indices,
        })
    }

    pub fn verify_terminal_query_openings(
        &self,
        prelude: &TerminalProofPrelude,
        commitment: &TerminalOracleCommitment,
        openings: &[TerminalOracleOpening],
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError> {
        Self::verify_prelude_binds_commitment(prelude, commitment)?;
        let plan = self.derive_terminal_query_plan(prelude, commitment)?;
        if openings.len() != plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalOracleQueryLengthMismatch {
                    expected: plan.indices.len(),
                    got: openings.len(),
                },
            );
        }
        for (query, (opening, expected_index)) in openings.iter().zip(&plan.indices).enumerate() {
            if opening.index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.index,
                    },
                );
            }
            self.verify_terminal_oracle_opening(commitment, opening)?;
        }
        Ok(plan)
    }

    pub fn derive_terminal_quadratic_residual_query_plan<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        prelude: &TerminalProofPrelude,
        residual_commitment: &TerminalOracleCommitment,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field,
    {
        let relation = verifying_key.primitive_quadratic_relation()?;
        Self::verify_terminal_oracle_commitment_identity(
            residual_commitment,
            Self::quadratic_residual_oracle_label(),
            relation.constraints.len(),
        )?;
        if relation.constraints.is_empty() {
            return Err(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty);
        }
        self.derive_terminal_query_plan(prelude, residual_commitment)
    }

    pub fn derive_terminal_residual_fold_query_plan(
        &self,
        prelude: &TerminalProofPrelude,
        residual_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError> {
        if residual_commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty);
        }

        let num_queries = prelude.parameters.num_queries as usize;
        let mut indices = Vec::with_capacity(num_queries);
        let mut counter = 0u64;
        while indices.len() < num_queries {
            if counter
                > (num_queries as u64)
                    .saturating_mul(4096)
                    .saturating_add(4096)
            {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_residual_fold_query_block(
                prelude,
                residual_commitment,
                fold_commitments,
                counter,
            );
            for limb in block {
                if indices.len() == num_queries {
                    break;
                }
                let bound = residual_commitment.values_len as u64;
                let zone = u64::MAX - (u64::MAX % bound);
                if limb < zone {
                    Self::accept_terminal_query_index(
                        &mut indices,
                        (limb % bound) as usize,
                        residual_commitment.values_len,
                        num_queries,
                    );
                }
            }
            counter += 1;
        }

        Ok(TerminalQueryPlan {
            oracle_label: residual_commitment.label.clone(),
            oracle_len: residual_commitment.values_len,
            indices,
        })
    }

    pub fn derive_terminal_primitive_constraint_query_plan<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        prelude: &TerminalProofPrelude,
    ) -> Result<TerminalConstraintQueryPlan, NativeTerminalVerifyError> {
        let domain_len = verifying_key.inventory.total_primitive_ops();
        if domain_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalConstraintQueryDomainEmpty);
        }

        let num_queries = prelude.parameters.num_queries as usize;
        let mut indices = Vec::with_capacity(num_queries);
        let mut counter = 0u64;
        while indices.len() < num_queries {
            if counter
                > (num_queries as u64)
                    .saturating_mul(4096)
                    .saturating_add(4096)
            {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_constraint_query_block(prelude, domain_len, counter);
            for limb in block {
                if indices.len() == num_queries {
                    break;
                }
                let bound = domain_len as u64;
                let zone = u64::MAX - (u64::MAX % bound);
                if limb < zone {
                    Self::accept_terminal_query_index(
                        &mut indices,
                        (limb % bound) as usize,
                        domain_len,
                        num_queries,
                    );
                }
            }
            counter += 1;
        }

        Ok(TerminalConstraintQueryPlan {
            domain_len,
            indices,
        })
    }

    pub fn prove_terminal_primitive_constraint_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        prelude: &TerminalProofPrelude,
        witness_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalPrimitiveConstraintProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let plan = self.derive_terminal_primitive_constraint_query_plan(verifying_key, prelude)?;
        let mut openings = Vec::with_capacity(plan.indices.len());
        for constraint_index in &plan.indices {
            let witness_ids = Self::primitive_constraint_witness_ids(
                &verifying_key.constraints[*constraint_index],
            );
            let mut witness_openings = Vec::with_capacity(witness_ids.len());
            for witness_id in witness_ids {
                let value = witness.traces.witness_trace.get_value(witness_id).ok_or(
                    NativeTerminalVerifyError::MissingWitness {
                        witness_id: witness_id.0,
                    },
                )?;
                witness_openings
                    .push(witness_oracle.open_goldilocks_value(witness_id.0 as usize, value)?);
            }
            openings.push(TerminalPrimitiveConstraintOpening {
                constraint_index: *constraint_index,
                witness_openings,
            });
        }
        Ok(TerminalPrimitiveConstraintProof { openings })
    }

    pub fn verify_terminal_primitive_constraint_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_commitment: &TerminalOracleCommitment,
        proof: &TerminalPrimitiveConstraintProof,
    ) -> Result<TerminalConstraintQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_witness_commitment_identity(verifying_key, witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, witness_commitment)?;
        let plan = self.derive_terminal_primitive_constraint_query_plan(verifying_key, prelude)?;
        if proof.openings.len() != plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalConstraintQueryLengthMismatch {
                    expected: plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&plan.indices).enumerate()
        {
            if opening.constraint_index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalConstraintQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.constraint_index,
                    },
                );
            }
            let constraint = &verifying_key.constraints[opening.constraint_index];
            let witness_ids = Self::primitive_constraint_witness_ids(constraint);
            if opening.witness_openings.len() != witness_ids.len() {
                return Err(
                    NativeTerminalVerifyError::TerminalConstraintOpeningCountMismatch {
                        constraint_index: opening.constraint_index,
                        expected: witness_ids.len(),
                        got: opening.witness_openings.len(),
                    },
                );
            }

            let mut values = Vec::with_capacity(witness_ids.len());
            for (opening_idx, (expected_witness, witness_opening)) in witness_ids
                .iter()
                .copied()
                .zip(&opening.witness_openings)
                .enumerate()
            {
                if witness_opening.index != expected_witness.0 as usize {
                    return Err(
                        NativeTerminalVerifyError::TerminalConstraintOpeningWitnessMismatch {
                            constraint_index: opening.constraint_index,
                            opening: opening_idx,
                            expected: expected_witness.0,
                            got: witness_opening.index,
                        },
                    );
                }
                self.verify_terminal_oracle_opening(witness_commitment, witness_opening)?;
                values.push(Self::field_from_goldilocks_basis_u64::<F>(
                    &witness_opening.value_basis,
                )?);
            }
            Self::verify_sampled_primitive_constraint(
                opening.constraint_index,
                constraint,
                public_inputs,
                &witness_ids,
                &values,
            )?;
        }

        Ok(plan)
    }

    pub fn commit_terminal_quadratic_residuals_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalOracleMerkleTree, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let residuals = self.terminal_quadratic_residual_values_goldilocks(
            verifying_key,
            public_inputs,
            witness,
        )?;
        TerminalOracleMerkleTree::commit_goldilocks_values(
            Self::quadratic_residual_oracle_label(),
            &residuals,
        )
    }

    pub fn terminal_quadratic_residual_values_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_relation_digest_goldilocks(verifying_key)?;
        if public_inputs.len() != verifying_key.header.fingerprint.public_flat_len {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: verifying_key.header.fingerprint.public_flat_len,
                got: public_inputs.len(),
            });
        }
        self.verify_witness_shape(&verifying_key.header.fingerprint, witness)?;
        let relation = verifying_key.primitive_quadratic_relation()?;
        relation.residuals(public_inputs, witness)
    }

    pub fn prove_terminal_quadratic_residual_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        residual_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalQuadraticResidualProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let residual_commitment = residual_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &residual_commitment)?;
        let residuals = self.terminal_quadratic_residual_values_goldilocks(
            verifying_key,
            public_inputs,
            witness,
        )?;
        let plan = self.derive_terminal_quadratic_residual_query_plan(
            verifying_key,
            prelude,
            &residual_commitment,
        )?;

        let mut openings = Vec::with_capacity(plan.indices.len());
        for residual_index in &plan.indices {
            openings.push(
                residual_oracle
                    .open_goldilocks_value(*residual_index, &residuals[*residual_index])?,
            );
        }
        Ok(TerminalQuadraticResidualProof { openings })
    }

    pub fn verify_terminal_quadratic_residual_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        residual_commitment: &TerminalOracleCommitment,
        proof: &TerminalQuadraticResidualProof,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_residual_commitment_identity(verifying_key, residual_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, residual_commitment)?;
        let plan = self.derive_terminal_quadratic_residual_query_plan(
            verifying_key,
            prelude,
            residual_commitment,
        )?;
        if proof.openings.len() != plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalQuadraticResidualQueryLengthMismatch {
                    expected: plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&plan.indices).enumerate()
        {
            if opening.index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalQuadraticResidualQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.index,
                    },
                );
            }
            self.verify_terminal_oracle_opening(residual_commitment, opening)?;
            let residual = Self::field_from_goldilocks_basis_u64::<F>(&opening.value_basis)?;
            if residual != F::ZERO {
                return Err(
                    NativeTerminalVerifyError::TerminalQuadraticResidualNonZero {
                        query,
                        residual_index: opening.index,
                    },
                );
            }
        }

        Ok(plan)
    }

    pub fn prove_terminal_quadratic_consistency_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_oracle: &TerminalOracleMerkleTree,
        residual_oracle: &TerminalOracleMerkleTree,
        residual_fold_proof: &TerminalResidualFoldProof,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalQuadraticConsistencyProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let witness_commitment = witness_oracle.commitment();
        let residual_commitment = residual_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, &residual_commitment)?;

        let relation = verifying_key.primitive_quadratic_relation()?;
        let plan = self.derive_terminal_residual_fold_query_plan(
            prelude,
            &residual_commitment,
            &residual_fold_proof.fold_commitments,
        )?;

        let mut openings = Vec::with_capacity(plan.indices.len());
        for quadratic_index in &plan.indices {
            let constraint = &relation.constraints[*quadratic_index];
            let witness_ids = Self::quadratic_constraint_witness_ids(constraint);
            let mut witness_openings = Vec::with_capacity(witness_ids.len());
            for witness_id in witness_ids {
                let value = witness.traces.witness_trace.get_value(witness_id).ok_or(
                    NativeTerminalVerifyError::MissingWitness {
                        witness_id: witness_id.0,
                    },
                )?;
                witness_openings
                    .push(witness_oracle.open_goldilocks_value(witness_id.0 as usize, value)?);
            }
            openings.push(TerminalQuadraticConsistencyOpening {
                quadratic_index: *quadratic_index,
                witness_openings,
            });
        }
        Ok(TerminalQuadraticConsistencyProof { openings })
    }

    pub fn verify_terminal_quadratic_consistency_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_commitment: &TerminalOracleCommitment,
        residual_commitment: &TerminalOracleCommitment,
        residual_fold_proof: &TerminalResidualFoldProof,
        proof: &TerminalQuadraticConsistencyProof,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_witness_commitment_identity(verifying_key, witness_commitment)?;
        Self::verify_terminal_residual_commitment_identity(verifying_key, residual_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, residual_commitment)?;
        let relation = verifying_key.primitive_quadratic_relation()?;
        let plan = self.verify_terminal_residual_fold_goldilocks::<F>(
            verifying_key,
            public_inputs,
            prelude,
            residual_commitment,
            residual_fold_proof,
        )?;
        if proof.openings.len() != plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalQuadraticConsistencyQueryLengthMismatch {
                    expected: plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&plan.indices).enumerate()
        {
            if opening.quadratic_index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalQuadraticConsistencyQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.quadratic_index,
                    },
                );
            }
            let residual_basis = Self::terminal_fold_base_value_basis(
                &residual_fold_proof.openings[query],
                &residual_fold_proof.final_value_basis,
            )
            .ok_or(
                NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                    query,
                    round: 0,
                },
            )?;
            let opened_residual = Self::field_from_goldilocks_basis_u64::<F>(residual_basis)?;

            let constraint = &relation.constraints[*expected_index];
            let witness_ids = Self::quadratic_constraint_witness_ids(constraint);
            if opening.witness_openings.len() != witness_ids.len() {
                return Err(
                    NativeTerminalVerifyError::TerminalQuadraticConsistencyOpeningCountMismatch {
                        quadratic_index: *expected_index,
                        expected: witness_ids.len(),
                        got: opening.witness_openings.len(),
                    },
                );
            }

            let mut witness_values = Vec::with_capacity(witness_ids.len());
            for (opening_idx, (expected_witness, witness_opening)) in witness_ids
                .iter()
                .copied()
                .zip(&opening.witness_openings)
                .enumerate()
            {
                if witness_opening.index != expected_witness.0 as usize {
                    return Err(
                        NativeTerminalVerifyError::TerminalQuadraticConsistencyOpeningWitnessMismatch {
                            quadratic_index: *expected_index,
                            opening: opening_idx,
                            expected: expected_witness.0,
                            got: witness_opening.index,
                        },
                    );
                }
                self.verify_terminal_oracle_opening(witness_commitment, witness_opening)?;
                witness_values.push(Self::field_from_goldilocks_basis_u64::<F>(
                    &witness_opening.value_basis,
                )?);
            }

            let a = Self::evaluate_opened_linear_expression(
                &constraint.a,
                public_inputs,
                &witness_ids,
                &witness_values,
            )?;
            let b = Self::evaluate_opened_linear_expression(
                &constraint.b,
                public_inputs,
                &witness_ids,
                &witness_values,
            )?;
            let c = Self::evaluate_opened_linear_expression(
                &constraint.c,
                public_inputs,
                &witness_ids,
                &witness_values,
            )?;
            let recomputed_residual = a * b - c;
            if opened_residual != recomputed_residual {
                return Err(
                    NativeTerminalVerifyError::TerminalQuadraticConsistencyResidualMismatch {
                        query,
                        quadratic_index: *expected_index,
                    },
                );
            }
            if opened_residual != F::ZERO {
                return Err(
                    NativeTerminalVerifyError::TerminalQuadraticResidualNonZero {
                        query,
                        residual_index: *expected_index,
                    },
                );
            }
        }

        Ok(plan)
    }

    pub fn prove_terminal_residual_fold_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        residual_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalResidualFoldProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let residual_commitment = residual_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &residual_commitment)?;

        let mut layers = vec![self.terminal_quadratic_residual_values_goldilocks(
            verifying_key,
            public_inputs,
            witness,
        )?];
        let mut trees = Vec::new();
        let mut fold_commitments = Vec::new();
        let mut round = 0usize;
        while layers.last().expect("base residual layer exists").len() > 1 {
            let challenge = Self::derive_terminal_residual_fold_challenge::<F>(
                prelude,
                &residual_commitment,
                &fold_commitments,
                round,
            )?;
            let current = layers.last().expect("current fold layer exists");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(F::ZERO);
                next.push(left * (F::ONE - challenge) + right * challenge);
            }
            let label = Self::residual_fold_oracle_label(round);
            let tree = TerminalOracleMerkleTree::commit_goldilocks_values(label, &next)?;
            fold_commitments.push(tree.commitment());
            trees.push(tree);
            layers.push(next);
            round += 1;
        }

        let final_value = layers
            .last()
            .and_then(|layer| layer.first())
            .copied()
            .ok_or(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty)?;
        let final_value_basis = Self::goldilocks_basis_u64(&final_value);
        let query_plan = self.derive_terminal_residual_fold_query_plan(
            prelude,
            &residual_commitment,
            &fold_commitments,
        )?;

        let mut openings = Vec::with_capacity(query_plan.indices.len());
        for initial_index in &query_plan.indices {
            let mut index = *initial_index;
            let mut rounds = Vec::with_capacity(fold_commitments.len());
            for round in 0..fold_commitments.len() {
                let pair_index = (index / 2) * 2;
                let current_tree = if round == 0 {
                    residual_oracle
                } else {
                    &trees[round - 1]
                };
                let current_values = &layers[round];
                let next_values = &layers[round + 1];
                let next_index = index / 2;
                let left =
                    current_tree.open_goldilocks_value(pair_index, &current_values[pair_index])?;
                let right = if pair_index + 1 < current_values.len() {
                    let mut opening = current_tree
                        .open_goldilocks_value(pair_index + 1, &current_values[pair_index + 1])?;
                    opening.path.clear();
                    Some(opening)
                } else {
                    None
                };
                let mut next =
                    trees[round].open_goldilocks_value(next_index, &next_values[next_index])?;
                next.path.clear();
                rounds.push(TerminalResidualFoldRoundOpening {
                    round,
                    pair_index,
                    left,
                    right,
                    next,
                });
                index = next_index;
            }
            openings.push(TerminalResidualFoldQueryOpening {
                initial_index: *initial_index,
                rounds,
            });
        }

        Ok(TerminalResidualFoldProof {
            fold_commitments,
            final_value_basis,
            openings,
        })
    }

    pub fn verify_terminal_residual_fold_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        residual_commitment: &TerminalOracleCommitment,
        proof: &TerminalResidualFoldProof,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_prelude_binds_commitment(prelude, residual_commitment)?;
        self.validate_terminal_residual_fold_commitments::<F>(
            verifying_key,
            residual_commitment,
            &proof.fold_commitments,
        )?;

        let query_plan = self.derive_terminal_residual_fold_query_plan(
            prelude,
            residual_commitment,
            &proof.fold_commitments,
        )?;
        if proof.openings.len() != query_plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldQueryLengthMismatch {
                    expected: query_plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        let mut challenges = Vec::with_capacity(proof.fold_commitments.len());
        for round in 0..proof.fold_commitments.len() {
            challenges.push(Self::derive_terminal_residual_fold_challenge::<F>(
                prelude,
                residual_commitment,
                &proof.fold_commitments[..round],
                round,
            )?);
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&query_plan.indices).enumerate()
        {
            if opening.initial_index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.initial_index,
                    },
                );
            }
            if opening.rounds.len() != proof.fold_commitments.len() {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldRoundCountMismatch {
                        query,
                        expected: proof.fold_commitments.len(),
                        got: opening.rounds.len(),
                    },
                );
            }

            let mut index = opening.initial_index;
            let mut current_len = residual_commitment.values_len;
            for (round, round_opening) in opening.rounds.iter().enumerate() {
                let expected_pair = (index / 2) * 2;
                let next_index = index / 2;
                if round_opening.round != round {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldRoundIndexMismatch {
                            query,
                            round,
                            expected: round,
                            got: round_opening.round,
                        },
                    );
                }
                if round_opening.pair_index != expected_pair {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "pair",
                            expected: expected_pair,
                            got: round_opening.pair_index,
                        },
                    );
                }

                let current_commitment = if round == 0 {
                    residual_commitment
                } else {
                    &proof.fold_commitments[round - 1]
                };
                self.verify_terminal_oracle_opening(current_commitment, &round_opening.left)?;
                if round_opening.left.index != expected_pair {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "left",
                            expected: expected_pair,
                            got: round_opening.left.index,
                        },
                    );
                }
                let left =
                    Self::field_from_goldilocks_basis_u64::<F>(&round_opening.left.value_basis)?;
                let right = if expected_pair + 1 < current_len {
                    let Some(right_opening) = &round_opening.right else {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningMissing {
                                query,
                                round,
                                index: expected_pair + 1,
                            },
                        );
                    };
                    if right_opening.index != expected_pair + 1 {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                                query,
                                round,
                                field: "right",
                                expected: expected_pair + 1,
                                got: right_opening.index,
                            },
                        );
                    }
                    if !right_opening.path.is_empty() {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningPathUnexpected {
                                query,
                                round,
                            },
                        );
                    }
                    let right_digest = Self::terminal_oracle_leaf_digest_from_basis(
                        &current_commitment.label,
                        current_commitment.values_len,
                        right_opening.index,
                        &right_opening.value_basis,
                    );
                    let Some(first_sibling) = round_opening.left.path.first() else {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                                query,
                                round,
                            },
                        );
                    };
                    if first_sibling.sibling_is_left || first_sibling.digest != right_digest {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                                query,
                                round,
                            },
                        );
                    }
                    Self::field_from_goldilocks_basis_u64::<F>(&right_opening.value_basis)?
                } else {
                    if round_opening.right.is_some() {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningUnexpected {
                                query,
                                round,
                                index: expected_pair + 1,
                            },
                        );
                    }
                    F::ZERO
                };

                if round_opening.next.index != next_index {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "next",
                            expected: next_index,
                            got: round_opening.next.index,
                        },
                    );
                }
                if !round_opening.next.path.is_empty() {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldNextOpeningPathUnexpected {
                            query,
                            round,
                        },
                    );
                }
                let opened_next =
                    Self::field_from_goldilocks_basis_u64::<F>(&round_opening.next.value_basis)?;
                let challenge = challenges[round];
                let expected_next = left * (F::ONE - challenge) + right * challenge;
                if opened_next != expected_next {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                            query,
                            round,
                        },
                    );
                }
                let linked_next_basis = if let Some(next_round) = opening.rounds.get(round + 1) {
                    if next_round.left.index == next_index {
                        Some(&next_round.left.value_basis)
                    } else {
                        next_round
                            .right
                            .as_ref()
                            .filter(|right| right.index == next_index)
                            .map(|right| &right.value_basis)
                    }
                } else {
                    Some(&proof.final_value_basis)
                };
                if linked_next_basis != Some(&round_opening.next.value_basis) {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                            query,
                            round,
                        },
                    );
                }
                index = next_index;
                current_len = current_len.div_ceil(2);
            }
        }

        let final_commitment = proof.fold_commitments.last().unwrap_or(residual_commitment);
        let final_root = Self::terminal_oracle_leaf_digest_from_basis(
            &final_commitment.label,
            final_commitment.values_len,
            0,
            &proof.final_value_basis,
        );
        if final_root != final_commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldFinalRootMismatch {
                    expected: final_commitment.root,
                    got: final_root,
                },
            );
        }
        let final_value = Self::field_from_goldilocks_basis_u64::<F>(&proof.final_value_basis)?;
        if final_value != F::ZERO {
            return Err(NativeTerminalVerifyError::TerminalResidualFoldFinalValueNonZero);
        }

        Ok(query_plan)
    }

    pub fn commit_terminal_npo_validity_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalOracleMerkleTree, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let values = self.terminal_npo_validity_values_goldilocks(verifying_key, witness)?;
        TerminalOracleMerkleTree::commit_goldilocks_values(
            Self::npo_validity_oracle_label(),
            &values,
        )
    }

    pub fn commit_terminal_npo_exhaustive_residuals_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalOracleMerkleTree, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let values =
            self.terminal_npo_exhaustive_residual_values_goldilocks(verifying_key, witness)?;
        TerminalOracleMerkleTree::commit_goldilocks_values(
            Self::npo_exhaustive_residual_oracle_label(),
            &values,
        )
    }

    pub fn terminal_npo_validity_values_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let witness_values = Self::terminal_witness_values(witness)?;
        let indexed_witness_values = witness_values
            .iter()
            .copied()
            .enumerate()
            .collect::<Vec<_>>();
        let mut values =
            Vec::with_capacity(Self::terminal_npo_validity_domain_len::<F>(verifying_key));
        for npo_index in 0..Self::terminal_npo_domain_len(verifying_key) {
            let evaluation = self.evaluate_terminal_npo_row_from_witness_goldilocks(
                verifying_key,
                witness,
                &indexed_witness_values,
                npo_index,
            )?;
            values.extend(Self::terminal_npo_row_evaluation_component_values::<F>(
                &evaluation,
            ));
        }
        Ok(values)
    }

    pub fn terminal_npo_exhaustive_residual_values_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let witness_values = Self::terminal_witness_values(witness)?;
        let indexed_witness_values = witness_values
            .iter()
            .copied()
            .enumerate()
            .collect::<Vec<_>>();
        let mut previous_normal_tip5_output = None;
        let mut previous_merkle_tip5_output = None;
        let mut values = Vec::with_capacity(
            Self::terminal_npo_polynomial_profile::<F>(verifying_key).residual_components,
        );
        for npo_index in 0..Self::terminal_npo_domain_len(verifying_key) {
            let evaluation = self.evaluate_terminal_npo_row_exhaustive_from_witness_goldilocks(
                verifying_key,
                witness,
                &indexed_witness_values,
                npo_index,
                &mut previous_normal_tip5_output,
                &mut previous_merkle_tip5_output,
            )?;
            values.extend(Self::terminal_npo_row_evaluation_component_values::<F>(
                &evaluation,
            ));
        }
        Ok(values)
    }

    pub fn commit_terminal_combined_validity_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalOracleMerkleTree, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let values = self.terminal_combined_validity_values_goldilocks(
            verifying_key,
            public_inputs,
            witness,
        )?;
        TerminalOracleMerkleTree::commit_goldilocks_values(
            Self::combined_validity_oracle_label(),
            &values,
        )
    }

    pub fn terminal_combined_validity_values_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let mut values = self.terminal_quadratic_residual_values_goldilocks(
            verifying_key,
            public_inputs,
            witness,
        )?;
        values.extend(self.terminal_npo_validity_values_goldilocks(verifying_key, witness)?);
        Ok(values)
    }

    pub fn derive_terminal_combined_validity_fold_query_plan(
        &self,
        prelude: &TerminalProofPrelude,
        combined_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError> {
        if combined_commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty);
        }

        let num_queries = prelude.parameters.num_queries as usize;
        let mut indices = Vec::with_capacity(num_queries);
        let mut counter = 0u64;
        while indices.len() < num_queries {
            if counter
                > (num_queries as u64)
                    .saturating_mul(4096)
                    .saturating_add(4096)
            {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_combined_validity_fold_query_block(
                prelude,
                combined_commitment,
                fold_commitments,
                counter,
            );
            for limb in block {
                if indices.len() == num_queries {
                    break;
                }
                let bound = combined_commitment.values_len as u64;
                let zone = u64::MAX - (u64::MAX % bound);
                if limb < zone {
                    Self::accept_terminal_query_index(
                        &mut indices,
                        (limb % bound) as usize,
                        combined_commitment.values_len,
                        num_queries,
                    );
                }
            }
            counter += 1;
        }

        Ok(TerminalQueryPlan {
            oracle_label: combined_commitment.label.clone(),
            oracle_len: combined_commitment.values_len,
            indices,
        })
    }

    pub fn derive_terminal_assignment_fold_query_plan(
        &self,
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError> {
        if assignment_commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty);
        }

        let num_queries = prelude.parameters.num_queries as usize;
        let mut indices = Vec::with_capacity(num_queries);
        let mut counter = 0u64;
        while indices.len() < num_queries {
            if counter
                > (num_queries as u64)
                    .saturating_mul(4096)
                    .saturating_add(4096)
            {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_assignment_fold_query_block(
                prelude,
                assignment_commitment,
                fold_commitments,
                counter,
            );
            for limb in block {
                if indices.len() == num_queries {
                    break;
                }
                let bound = assignment_commitment.values_len as u64;
                let zone = u64::MAX - (u64::MAX % bound);
                if limb < zone {
                    Self::accept_terminal_query_index(
                        &mut indices,
                        (limb % bound) as usize,
                        assignment_commitment.values_len,
                        num_queries,
                    );
                }
            }
            counter += 1;
        }

        Ok(TerminalQueryPlan {
            oracle_label: assignment_commitment.label.clone(),
            oracle_len: assignment_commitment.values_len,
            indices,
        })
    }

    pub fn prove_terminal_combined_validity_fold_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        combined_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalResidualFoldProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let combined_commitment = combined_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &combined_commitment)?;

        let mut layers = vec![self.terminal_combined_validity_values_goldilocks(
            verifying_key,
            public_inputs,
            witness,
        )?];
        let mut trees = Vec::new();
        let mut fold_commitments = Vec::new();
        let mut round = 0usize;
        while layers
            .last()
            .expect("base combined validity layer exists")
            .len()
            > 1
        {
            let challenge = Self::derive_terminal_combined_validity_fold_challenge::<F>(
                prelude,
                &combined_commitment,
                &fold_commitments,
                round,
            )?;
            let current = layers.last().expect("current combined fold layer exists");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(F::ZERO);
                next.push(left * (F::ONE - challenge) + right * challenge);
            }
            let label = Self::combined_validity_fold_oracle_label(round);
            let tree = TerminalOracleMerkleTree::commit_goldilocks_values(label, &next)?;
            fold_commitments.push(tree.commitment());
            trees.push(tree);
            layers.push(next);
            round += 1;
        }

        let final_value = layers
            .last()
            .and_then(|layer| layer.first())
            .copied()
            .ok_or(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty)?;
        let final_value_basis = Self::goldilocks_basis_u64(&final_value);
        let query_plan = self.derive_terminal_combined_validity_fold_query_plan(
            prelude,
            &combined_commitment,
            &fold_commitments,
        )?;

        let mut openings = Vec::with_capacity(query_plan.indices.len());
        for initial_index in &query_plan.indices {
            let mut index = *initial_index;
            let mut rounds = Vec::with_capacity(fold_commitments.len());
            for round in 0..fold_commitments.len() {
                let pair_index = (index / 2) * 2;
                let current_tree = if round == 0 {
                    combined_oracle
                } else {
                    &trees[round - 1]
                };
                let current_values = &layers[round];
                let next_values = &layers[round + 1];
                let next_index = index / 2;
                let left =
                    current_tree.open_goldilocks_value(pair_index, &current_values[pair_index])?;
                let right = if pair_index + 1 < current_values.len() {
                    let mut opening = current_tree
                        .open_goldilocks_value(pair_index + 1, &current_values[pair_index + 1])?;
                    opening.path.clear();
                    Some(opening)
                } else {
                    None
                };
                let mut next =
                    trees[round].open_goldilocks_value(next_index, &next_values[next_index])?;
                next.path.clear();
                rounds.push(TerminalResidualFoldRoundOpening {
                    round,
                    pair_index,
                    left,
                    right,
                    next,
                });
                index = next_index;
            }
            openings.push(TerminalResidualFoldQueryOpening {
                initial_index: *initial_index,
                rounds,
            });
        }

        Ok(TerminalResidualFoldProof {
            fold_commitments,
            final_value_basis,
            openings,
        })
    }

    pub fn verify_terminal_combined_validity_fold_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        combined_commitment: &TerminalOracleCommitment,
        proof: &TerminalResidualFoldProof,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_prelude_binds_commitment(prelude, combined_commitment)?;
        self.validate_terminal_combined_validity_fold_commitments::<F>(
            verifying_key,
            combined_commitment,
            &proof.fold_commitments,
        )?;

        let query_plan = self.derive_terminal_combined_validity_fold_query_plan(
            prelude,
            combined_commitment,
            &proof.fold_commitments,
        )?;
        if proof.openings.len() != query_plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldQueryLengthMismatch {
                    expected: query_plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        let mut challenges = Vec::with_capacity(proof.fold_commitments.len());
        for round in 0..proof.fold_commitments.len() {
            challenges.push(Self::derive_terminal_combined_validity_fold_challenge::<F>(
                prelude,
                combined_commitment,
                &proof.fold_commitments[..round],
                round,
            )?);
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&query_plan.indices).enumerate()
        {
            if opening.initial_index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.initial_index,
                    },
                );
            }
            if opening.rounds.len() != proof.fold_commitments.len() {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldRoundCountMismatch {
                        query,
                        expected: proof.fold_commitments.len(),
                        got: opening.rounds.len(),
                    },
                );
            }

            let mut index = opening.initial_index;
            let mut current_len = combined_commitment.values_len;
            for (round, round_opening) in opening.rounds.iter().enumerate() {
                let expected_pair = (index / 2) * 2;
                let next_index = index / 2;
                if round_opening.round != round {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldRoundIndexMismatch {
                            query,
                            round,
                            expected: round,
                            got: round_opening.round,
                        },
                    );
                }
                if round_opening.pair_index != expected_pair {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "pair",
                            expected: expected_pair,
                            got: round_opening.pair_index,
                        },
                    );
                }

                let current_commitment = if round == 0 {
                    combined_commitment
                } else {
                    &proof.fold_commitments[round - 1]
                };
                self.verify_terminal_oracle_opening(current_commitment, &round_opening.left)?;
                if round_opening.left.index != expected_pair {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "left",
                            expected: expected_pair,
                            got: round_opening.left.index,
                        },
                    );
                }
                let left =
                    Self::field_from_goldilocks_basis_u64::<F>(&round_opening.left.value_basis)?;
                let right = if expected_pair + 1 < current_len {
                    let Some(right_opening) = &round_opening.right else {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningMissing {
                                query,
                                round,
                                index: expected_pair + 1,
                            },
                        );
                    };
                    if right_opening.index != expected_pair + 1 {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                                query,
                                round,
                                field: "right",
                                expected: expected_pair + 1,
                                got: right_opening.index,
                            },
                        );
                    }
                    if !right_opening.path.is_empty() {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningPathUnexpected {
                                query,
                                round,
                            },
                        );
                    }
                    let right_digest = Self::terminal_oracle_leaf_digest_from_basis(
                        &current_commitment.label,
                        current_commitment.values_len,
                        right_opening.index,
                        &right_opening.value_basis,
                    );
                    let Some(first_sibling) = round_opening.left.path.first() else {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                                query,
                                round,
                            },
                        );
                    };
                    if first_sibling.sibling_is_left || first_sibling.digest != right_digest {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                                query,
                                round,
                            },
                        );
                    }
                    Self::field_from_goldilocks_basis_u64::<F>(&right_opening.value_basis)?
                } else {
                    if round_opening.right.is_some() {
                        return Err(
                            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningUnexpected {
                                query,
                                round,
                                index: expected_pair + 1,
                            },
                        );
                    }
                    F::ZERO
                };

                if round_opening.next.index != next_index {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "next",
                            expected: next_index,
                            got: round_opening.next.index,
                        },
                    );
                }
                if !round_opening.next.path.is_empty() {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldNextOpeningPathUnexpected {
                            query,
                            round,
                        },
                    );
                }
                let opened_next =
                    Self::field_from_goldilocks_basis_u64::<F>(&round_opening.next.value_basis)?;
                let challenge = challenges[round];
                let expected_next = left * (F::ONE - challenge) + right * challenge;
                if opened_next != expected_next {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                            query,
                            round,
                        },
                    );
                }
                let linked_next_basis = if let Some(next_round) = opening.rounds.get(round + 1) {
                    if next_round.left.index == next_index {
                        Some(&next_round.left.value_basis)
                    } else {
                        next_round
                            .right
                            .as_ref()
                            .filter(|right| right.index == next_index)
                            .map(|right| &right.value_basis)
                    }
                } else {
                    Some(&proof.final_value_basis)
                };
                if linked_next_basis != Some(&round_opening.next.value_basis) {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                            query,
                            round,
                        },
                    );
                }
                index = next_index;
                current_len = current_len.div_ceil(2);
            }
        }

        let final_commitment = proof.fold_commitments.last().unwrap_or(combined_commitment);
        let final_root = Self::terminal_oracle_leaf_digest_from_basis(
            &final_commitment.label,
            final_commitment.values_len,
            0,
            &proof.final_value_basis,
        );
        if final_root != final_commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldFinalRootMismatch {
                    expected: final_commitment.root,
                    got: final_root,
                },
            );
        }
        let final_value = Self::field_from_goldilocks_basis_u64::<F>(&proof.final_value_basis)?;
        if final_value != F::ZERO {
            return Err(NativeTerminalVerifyError::TerminalResidualFoldFinalValueNonZero);
        }

        Ok(query_plan)
    }

    pub fn derive_terminal_npo_validity_fold_query_plan(
        &self,
        prelude: &TerminalProofPrelude,
        validity_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError> {
        if validity_commitment.values_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalNpoQueryDomainEmpty);
        }

        let num_queries = prelude.parameters.num_queries as usize;
        let mut indices = Vec::with_capacity(num_queries);
        let mut counter = 0u64;
        while indices.len() < num_queries {
            if counter
                > (num_queries as u64)
                    .saturating_mul(4096)
                    .saturating_add(4096)
            {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_npo_validity_fold_query_block(
                prelude,
                validity_commitment,
                fold_commitments,
                counter,
            );
            for limb in block {
                if indices.len() == num_queries {
                    break;
                }
                let bound = validity_commitment.values_len as u64;
                let zone = u64::MAX - (u64::MAX % bound);
                if limb < zone {
                    Self::accept_terminal_query_index(
                        &mut indices,
                        (limb % bound) as usize,
                        validity_commitment.values_len,
                        num_queries,
                    );
                }
            }
            counter += 1;
        }

        Ok(TerminalQueryPlan {
            oracle_label: validity_commitment.label.clone(),
            oracle_len: validity_commitment.values_len,
            indices,
        })
    }

    pub fn prove_terminal_npo_validity_fold_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        validity_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalNpoValidityFoldProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let validity_commitment = validity_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &validity_commitment)?;

        let mut layers =
            vec![self.terminal_npo_validity_values_goldilocks(verifying_key, witness)?];
        let mut trees = Vec::new();
        let mut fold_commitments = Vec::new();
        let mut round = 0usize;
        while layers.last().expect("base NPO validity layer exists").len() > 1 {
            let challenge = Self::derive_terminal_npo_validity_fold_challenge::<F>(
                prelude,
                &validity_commitment,
                &fold_commitments,
                round,
            )?;
            let current = layers
                .last()
                .expect("current NPO validity fold layer exists");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(F::ZERO);
                next.push(left * (F::ONE - challenge) + right * challenge);
            }
            let label = Self::npo_validity_fold_oracle_label(round);
            let tree = TerminalOracleMerkleTree::commit_goldilocks_values(label, &next)?;
            fold_commitments.push(tree.commitment());
            trees.push(tree);
            layers.push(next);
            round += 1;
        }

        let final_value = layers
            .last()
            .and_then(|layer| layer.first())
            .copied()
            .ok_or(NativeTerminalVerifyError::TerminalNpoQueryDomainEmpty)?;
        let final_value_basis = Self::goldilocks_basis_u64(&final_value);
        let query_plan = self.derive_terminal_npo_validity_fold_query_plan(
            prelude,
            &validity_commitment,
            &fold_commitments,
        )?;

        let mut expected_round_indices = vec![Vec::new(); fold_commitments.len()];
        for initial_index in &query_plan.indices {
            let mut index = *initial_index;
            let mut current_len = validity_commitment.values_len;
            for round in 0..fold_commitments.len() {
                let pair_index = (index / 2) * 2;
                let next_index = index / 2;
                NativeTerminalCompiler::push_unique_usize(
                    &mut expected_round_indices[round],
                    pair_index,
                );
                if pair_index + 1 < current_len {
                    NativeTerminalCompiler::push_unique_usize(
                        &mut expected_round_indices[round],
                        pair_index + 1,
                    );
                }
                index = next_index;
                current_len = current_len.div_ceil(2);
            }
        }
        let mut round_openings = Vec::with_capacity(fold_commitments.len());
        for (round, round_indices) in expected_round_indices.iter_mut().enumerate() {
            round_indices.sort_unstable();
            let current_tree = if round == 0 {
                validity_oracle
            } else {
                &trees[round - 1]
            };
            let current_values = &layers[round];
            let mut opening_values = Vec::with_capacity(round_indices.len());
            for index in round_indices {
                opening_values.push((*index, &current_values[*index]));
            }
            round_openings.push(current_tree.open_goldilocks_multi_values(&opening_values)?);
        }

        let mut openings = Vec::with_capacity(query_plan.indices.len());
        for initial_index in &query_plan.indices {
            openings.push(TerminalResidualFoldQueryOpening {
                initial_index: *initial_index,
                rounds: Vec::new(),
            });
        }

        Ok(TerminalNpoValidityFoldProof {
            fold_commitments,
            final_value_basis,
            round_openings,
            openings,
        })
    }

    pub fn verify_terminal_npo_validity_fold_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        validity_commitment: &TerminalOracleCommitment,
        proof: &TerminalNpoValidityFoldProof,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_prelude_binds_commitment(prelude, validity_commitment)?;
        self.validate_terminal_npo_validity_fold_commitments(
            verifying_key,
            validity_commitment,
            &proof.fold_commitments,
        )?;

        let query_plan = self.derive_terminal_npo_validity_fold_query_plan(
            prelude,
            validity_commitment,
            &proof.fold_commitments,
        )?;
        if proof.openings.len() != query_plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalNpoValidityFoldQueryLengthMismatch {
                    expected: query_plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        let mut challenges = Vec::with_capacity(proof.fold_commitments.len());
        for round in 0..proof.fold_commitments.len() {
            challenges.push(Self::derive_terminal_npo_validity_fold_challenge::<F>(
                prelude,
                validity_commitment,
                &proof.fold_commitments[..round],
                round,
            )?);
        }

        if proof.round_openings.len() != proof.fold_commitments.len() {
            return Err(
                NativeTerminalVerifyError::TerminalNpoValidityFoldCommitmentLengthMismatch {
                    round: proof.fold_commitments.len(),
                    expected: proof.fold_commitments.len(),
                    got: proof.round_openings.len(),
                },
            );
        }
        let expected_round_indices = Self::terminal_fold_round_indices(
            &query_plan.indices,
            validity_commitment.values_len,
            proof.fold_commitments.len(),
        );
        let mut round_values = Vec::with_capacity(proof.round_openings.len());
        for (round, proof_opening) in proof.round_openings.iter().enumerate() {
            let current_commitment = if round == 0 {
                validity_commitment
            } else {
                &proof.fold_commitments[round - 1]
            };
            let values = self.verify_terminal_oracle_multi_proof_goldilocks::<F>(
                current_commitment,
                proof_opening,
            )?;
            Self::verify_terminal_oracle_value_indices(&expected_round_indices[round], &values)?;
            round_values.push(values);
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&query_plan.indices).enumerate()
        {
            if opening.initial_index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoValidityFoldQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.initial_index,
                    },
                );
            }
            if !opening.rounds.is_empty() {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoValidityFoldRoundCountMismatch {
                        query,
                        expected: 0,
                        got: opening.rounds.len(),
                    },
                );
            }

            let mut index = opening.initial_index;
            let mut current_len = validity_commitment.values_len;
            for (round, challenge) in challenges.iter().copied().enumerate() {
                let expected_pair = (index / 2) * 2;
                let next_index = index / 2;
                let left = Self::terminal_opened_value(&round_values[round], expected_pair)
                    .map_err(|_| {
                        NativeTerminalVerifyError::TerminalNpoValidityFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "left",
                            expected: expected_pair,
                            got: expected_pair,
                        }
                    })?;
                let right = if expected_pair + 1 < current_len {
                    Self::terminal_opened_value(&round_values[round], expected_pair + 1).map_err(
                        |_| NativeTerminalVerifyError::TerminalNpoValidityFoldRightOpeningMissing {
                            query,
                            round,
                            index: expected_pair + 1,
                        },
                    )?
                } else {
                    F::ZERO
                };

                let expected_next = left * (F::ONE - challenge) + right * challenge;
                let opened_next = if let Some(next_round_values) = round_values.get(round + 1) {
                    Self::terminal_opened_value(next_round_values, next_index).map_err(|_| {
                        NativeTerminalVerifyError::TerminalNpoValidityFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "next",
                            expected: next_index,
                            got: next_index,
                        }
                    })?
                } else {
                    Self::field_from_goldilocks_basis_u64::<F>(&proof.final_value_basis)?
                };
                if opened_next != expected_next {
                    return Err(
                        NativeTerminalVerifyError::TerminalNpoValidityFoldConsistencyMismatch {
                            query,
                            round,
                        },
                    );
                }
                index = next_index;
                current_len = current_len.div_ceil(2);
            }
        }

        let final_commitment = proof.fold_commitments.last().unwrap_or(validity_commitment);
        let final_root = Self::terminal_oracle_leaf_digest_from_basis(
            &final_commitment.label,
            final_commitment.values_len,
            0,
            &proof.final_value_basis,
        );
        if final_root != final_commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalNpoValidityFoldFinalRootMismatch {
                    expected: final_commitment.root,
                    got: final_root,
                },
            );
        }
        let final_value = Self::field_from_goldilocks_basis_u64::<F>(&proof.final_value_basis)?;
        if final_value != F::ZERO {
            return Err(NativeTerminalVerifyError::TerminalNpoValidityFoldFinalValueNonZero);
        }

        Ok(query_plan)
    }

    pub fn prove_terminal_npo_validity_consistency_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_oracle: &TerminalOracleMerkleTree,
        validity_oracle: &TerminalOracleMerkleTree,
        validity_fold_proof: &TerminalNpoValidityFoldProof,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalNpoValidityConsistencyProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let witness_commitment = witness_oracle.commitment();
        let validity_commitment = validity_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, &validity_commitment)?;
        let plan = self.derive_terminal_npo_validity_fold_query_plan(
            prelude,
            &validity_commitment,
            &validity_fold_proof.fold_commitments,
        )?;

        let mut openings = Vec::with_capacity(plan.indices.len());
        let mut witness_ids = Vec::new();
        for validity_index in &plan.indices {
            let (npo_index, component_offset) =
                Self::terminal_npo_validity_component_row::<F>(verifying_key, *validity_index)
                    .ok_or(
                        NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                            query: 0,
                            expected: *validity_index,
                            got: *validity_index,
                        },
                    )?;
            let (npo_opening, row_witness_ids) =
                self.terminal_npo_local_opening_goldilocks(verifying_key, witness, npo_index)?;
            for witness_id in row_witness_ids {
                Self::push_unique_witness(&mut witness_ids, witness_id);
            }
            openings.push(TerminalNpoValidityConsistencyOpening {
                validity_index: *validity_index,
                npo_index,
                component_offset,
                npo_opening,
            });
        }
        witness_ids.sort_by_key(|witness_id| witness_id.0);
        let mut witness_values = Vec::with_capacity(witness_ids.len());
        for witness_id in &witness_ids {
            let value = witness.traces.witness_trace.get_value(*witness_id).ok_or(
                NativeTerminalVerifyError::MissingWitness {
                    witness_id: witness_id.0,
                },
            )?;
            witness_values.push((witness_id.0 as usize, value));
        }
        let witness_multi_opening = witness_oracle.open_goldilocks_multi_values(&witness_values)?;
        Ok(TerminalNpoValidityConsistencyProof {
            openings,
            witness_multi_opening,
        })
    }

    pub fn prove_terminal_npo_exhaustive_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalNpoExhaustiveProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let witness_commitment = witness_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &witness_commitment)?;

        let npo_rows = Self::terminal_npo_domain_len(verifying_key);
        let mut tip5_hidden_input_values_le = Vec::new();
        let mut witness_ids = Vec::new();
        let mut boolean_witness_ids = Vec::new();
        for npo_index in 0..npo_rows {
            let (npo_opening, row_witness_ids) =
                self.terminal_npo_local_opening_goldilocks(verifying_key, witness, npo_index)?;
            for witness_id in row_witness_ids {
                Self::push_unique_witness(&mut witness_ids, witness_id);
            }
            match Self::terminal_npo_row(verifying_key, npo_index) {
                Some(NativeTerminalNpoRowRef::Tip5 { callsite, .. }) => {
                    if let Some(witness_id) = callsite.tip5_mmcs_bit {
                        Self::push_unique_witness(&mut boolean_witness_ids, witness_id);
                    }
                    let mode = callsite.tip5_mode.ok_or(
                        NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                            op_type: Self::tip5_op_type().as_str().into(),
                            row: npo_index,
                            field: "tip5_mode",
                            limb: 0,
                            expected: Some(1),
                            got: None,
                        },
                    )?;
                    let mmcs_bit = if let Some(witness_id) = callsite.tip5_mmcs_bit {
                        let value = witness.traces.witness_trace.get_value(witness_id).ok_or(
                            NativeTerminalVerifyError::MissingWitness {
                                witness_id: witness_id.0,
                            },
                        )?;
                        let basis = value.as_basis_coefficients_slice();
                        basis.first().copied().unwrap_or(Goldilocks::ZERO) == Goldilocks::ONE
                    } else {
                        false
                    };
                    let full_hidden = Self::terminal_tip5_hidden_inputs_from_compact(
                        npo_index,
                        verifying_key,
                        npo_opening.tip5_hidden_input_nonzero_mask,
                        &npo_opening.tip5_hidden_input_values_le,
                    )?;
                    let has_ctl_output = callsite.outputs.iter().any(Option::is_some);
                    for hidden in full_hidden {
                        if Self::should_serialize_tip5_hidden_limb(
                            callsite,
                            mode,
                            has_ctl_output,
                            mmcs_bit,
                            hidden.limb,
                        ) {
                            tip5_hidden_input_values_le.push(hidden.value_basis[0].to_le_bytes());
                        }
                    }
                }
                Some(NativeTerminalNpoRowRef::Recompose { .. }) => {
                    debug_assert_eq!(npo_opening.tip5_hidden_input_nonzero_mask, 0);
                    debug_assert!(npo_opening.tip5_hidden_input_values_le.is_empty());
                }
                None => {}
            }
        }

        witness_ids.sort_by_key(|witness_id| witness_id.0);
        boolean_witness_ids.sort_by_key(|witness_id| witness_id.0);
        let mut witness_values = Vec::with_capacity(witness_ids.len());
        for witness_id in &witness_ids {
            let value = witness.traces.witness_trace.get_value(*witness_id).ok_or(
                NativeTerminalVerifyError::MissingWitness {
                    witness_id: witness_id.0,
                },
            )?;
            witness_values.push((witness_id.0 as usize, value));
        }
        let boolean_witness_indices = boolean_witness_ids
            .iter()
            .map(|witness_id| witness_id.0 as usize)
            .collect::<Vec<_>>();
        let witness_multi_opening = witness_oracle
            .open_goldilocks_known_index_multi_values_with_boolean_indices(
                &witness_values,
                &boolean_witness_indices,
            )?;
        Ok(TerminalNpoExhaustiveProof {
            tip5_hidden_input_values_le,
            witness_multi_opening,
        })
    }

    pub fn verify_terminal_npo_exhaustive_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_commitment: &TerminalOracleCommitment,
        proof: &TerminalNpoExhaustiveProof,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_witness_commitment_identity(verifying_key, witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, witness_commitment)?;

        let npo_rows = Self::terminal_npo_domain_len(verifying_key);
        let mut expected_global_witness_ids = Vec::new();
        let mut expected_global_boolean_witness_ids = Vec::new();
        for npo_index in 0..npo_rows {
            let row_ref = Self::terminal_npo_row(verifying_key, npo_index).ok_or(
                NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                    query: npo_index,
                    expected: npo_index,
                    got: npo_index,
                },
            )?;
            match row_ref {
                NativeTerminalNpoRowRef::Tip5 { callsite, .. } => {
                    for witness_id in Self::npo_callsite_witness_ids(callsite) {
                        Self::push_unique_witness(&mut expected_global_witness_ids, witness_id);
                    }
                    if let Some(witness_id) = callsite.tip5_mmcs_bit {
                        Self::push_unique_witness(
                            &mut expected_global_boolean_witness_ids,
                            witness_id,
                        );
                    }
                }
                NativeTerminalNpoRowRef::Recompose { callsite, .. } => {
                    for witness_id in Self::npo_callsite_witness_ids(callsite) {
                        Self::push_unique_witness(&mut expected_global_witness_ids, witness_id);
                    }
                }
            }
        }
        expected_global_witness_ids.sort_by_key(|witness_id| witness_id.0);
        expected_global_boolean_witness_ids.sort_by_key(|witness_id| witness_id.0);
        let expected_global_witness_indices = expected_global_witness_ids
            .iter()
            .map(|witness_id| witness_id.0 as usize)
            .collect::<Vec<_>>();
        let expected_global_boolean_witness_indices = expected_global_boolean_witness_ids
            .iter()
            .map(|witness_id| witness_id.0 as usize)
            .collect::<Vec<_>>();
        let witness_values = self
            .verify_terminal_oracle_known_index_multi_proof_goldilocks_with_boolean_indices::<F>(
                witness_commitment,
                &expected_global_witness_indices,
                &expected_global_boolean_witness_indices,
                &proof.witness_multi_opening,
            )?;

        let mut hidden_value_offset = 0usize;
        let mut previous_normal_tip5_output = None;
        let mut previous_merkle_tip5_output = None;
        for npo_index in 0..npo_rows {
            let row_ref = Self::terminal_npo_row(verifying_key, npo_index).ok_or(
                NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                    query: npo_index,
                    expected: npo_index,
                    got: npo_index,
                },
            )?;
            match row_ref {
                NativeTerminalNpoRowRef::Tip5 { row, callsite, .. } => {
                    let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                    let opened_values = Self::verify_npo_witness_values(
                        npo_index,
                        &expected_witness_ids,
                        &witness_values,
                    )?;
                    let hidden_inputs = Self::terminal_tip5_hidden_inputs_from_exhaustive_compact(
                        npo_index,
                        row,
                        callsite,
                        &proof.tip5_hidden_input_values_le,
                        &mut hidden_value_offset,
                        &expected_witness_ids,
                        &opened_values,
                        &previous_normal_tip5_output,
                        &previous_merkle_tip5_output,
                    )?;
                    self.verify_exhaustive_tip5_npo_row_values::<F>(
                        npo_index,
                        row,
                        callsite,
                        &hidden_inputs,
                        &expected_witness_ids,
                        &opened_values,
                        &mut previous_normal_tip5_output,
                        &mut previous_merkle_tip5_output,
                    )?;
                }
                NativeTerminalNpoRowRef::Recompose {
                    op_type,
                    row,
                    callsite,
                } => {
                    let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                    let opened_values = Self::verify_npo_witness_values(
                        npo_index,
                        &expected_witness_ids,
                        &witness_values,
                    )?;
                    self.verify_sampled_recompose_npo_row_values::<F>(
                        npo_index,
                        op_type,
                        row,
                        callsite,
                        &[],
                        &expected_witness_ids,
                        &opened_values,
                    )?;
                }
            }
        }
        if hidden_value_offset != proof.tip5_hidden_input_values_le.len() {
            return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                npo_index: npo_rows,
                expected: hidden_value_offset,
                got: proof.tip5_hidden_input_values_le.len(),
            });
        }

        Ok(())
    }

    pub fn verify_terminal_npo_validity_consistency_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_commitment: &TerminalOracleCommitment,
        validity_commitment: &TerminalOracleCommitment,
        validity_fold_proof: &TerminalNpoValidityFoldProof,
        proof: &TerminalNpoValidityConsistencyProof,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_witness_commitment_identity(verifying_key, witness_commitment)?;
        Self::verify_terminal_oracle_commitment_identity(
            validity_commitment,
            Self::npo_validity_oracle_label(),
            Self::terminal_npo_validity_domain_len::<F>(verifying_key),
        )?;
        let plan = self.verify_terminal_npo_validity_fold_goldilocks::<F>(
            verifying_key,
            public_inputs,
            prelude,
            validity_commitment,
            validity_fold_proof,
        )?;
        Self::verify_prelude_binds_commitment(prelude, witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, validity_commitment)?;
        if proof.openings.len() != plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalNpoValidityQueryLengthMismatch {
                    expected: plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        let witness_values = self.verify_terminal_oracle_multi_proof_goldilocks::<F>(
            witness_commitment,
            &proof.witness_multi_opening,
        )?;
        let mut expected_global_witness_ids = Vec::new();
        for validity_index in &plan.indices {
            let (npo_index, _) =
                Self::terminal_npo_validity_component_row::<F>(verifying_key, *validity_index)
                    .ok_or(
                        NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                            query: 0,
                            expected: *validity_index,
                            got: *validity_index,
                        },
                    )?;
            let row_ref = Self::terminal_npo_row(verifying_key, npo_index).ok_or(
                NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                    query: 0,
                    expected: *validity_index,
                    got: *validity_index,
                },
            )?;
            match row_ref {
                NativeTerminalNpoRowRef::Tip5 { callsite, .. }
                | NativeTerminalNpoRowRef::Recompose { callsite, .. } => {
                    for witness_id in Self::npo_callsite_witness_ids(callsite) {
                        Self::push_unique_witness(&mut expected_global_witness_ids, witness_id);
                    }
                }
            }
        }
        expected_global_witness_ids.sort_by_key(|witness_id| witness_id.0);
        if witness_values.len() != expected_global_witness_ids.len() {
            return Err(
                NativeTerminalVerifyError::TerminalOracleQueryLengthMismatch {
                    expected: expected_global_witness_ids.len(),
                    got: witness_values.len(),
                },
            );
        }
        for (query, (expected_witness, (got_index, _))) in expected_global_witness_ids
            .iter()
            .zip(&witness_values)
            .enumerate()
        {
            if *got_index != expected_witness.0 as usize {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query,
                        expected: expected_witness.0 as usize,
                        got: *got_index,
                    },
                );
            }
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&plan.indices).enumerate()
        {
            if opening.validity_index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.validity_index,
                    },
                );
            }
            let (expected_npo_index, expected_component_offset) =
                Self::terminal_npo_validity_component_row::<F>(verifying_key, *expected_index)
                    .ok_or(
                        NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                            query,
                            expected: *expected_index,
                            got: *expected_index,
                        },
                    )?;
            if opening.npo_index != expected_npo_index {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                        query,
                        expected: expected_npo_index,
                        got: opening.npo_index,
                    },
                );
            }
            if opening.component_offset != expected_component_offset {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                        query,
                        expected: expected_component_offset,
                        got: opening.component_offset,
                    },
                );
            }
            let validity_basis = Self::terminal_npo_validity_fold_base_value_basis(
                validity_fold_proof,
                *expected_index,
            )
            .ok_or(
                NativeTerminalVerifyError::TerminalNpoValidityFoldConsistencyMismatch {
                    query,
                    round: 0,
                },
            )?;
            let validity = Self::field_from_goldilocks_basis_u64::<F>(validity_basis)?;
            if opening.npo_opening.npo_index != expected_npo_index {
                return Err(NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                    query,
                    expected: expected_npo_index,
                    got: opening.npo_opening.npo_index,
                });
            }
            let row_ref = Self::terminal_npo_row(verifying_key, expected_npo_index).ok_or(
                NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                    query,
                    expected: expected_npo_index,
                    got: expected_npo_index,
                },
            )?;
            match row_ref {
                NativeTerminalNpoRowRef::Tip5 { row, callsite, .. } => {
                    let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                    let opened_values = Self::verify_npo_witness_values(
                        expected_npo_index,
                        &expected_witness_ids,
                        &witness_values,
                    )?;
                    let evaluation = self.evaluate_sampled_tip5_npo_row_values::<F>(
                        *expected_index,
                        row,
                        callsite,
                        &Self::terminal_tip5_hidden_inputs_from_compact(
                            opening.npo_opening.npo_index,
                            verifying_key,
                            opening.npo_opening.tip5_hidden_input_nonzero_mask,
                            &opening.npo_opening.tip5_hidden_input_values_le,
                        )?,
                        &expected_witness_ids,
                        &opened_values,
                    )?;
                    let recomputed_validity = Self::terminal_npo_row_evaluation_component_value::<F>(
                        &evaluation,
                        expected_component_offset,
                    )?;
                    if validity != recomputed_validity {
                        return Err(
                            NativeTerminalVerifyError::TerminalNpoValidityResidualMismatch {
                                query,
                                npo_index: expected_npo_index,
                            },
                        );
                    }
                    if validity != F::ZERO {
                        return Err(NativeTerminalVerifyError::TerminalNpoValidityNonZero {
                            query,
                            npo_index: expected_npo_index,
                        });
                    }
                }
                NativeTerminalNpoRowRef::Recompose {
                    op_type,
                    row,
                    callsite,
                } => {
                    if opening.npo_opening.tip5_hidden_input_nonzero_mask != 0
                        || !opening.npo_opening.tip5_hidden_input_values_le.is_empty()
                    {
                        return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                            npo_index: expected_npo_index,
                            expected: 0,
                            got: opening.npo_opening.tip5_hidden_input_values_le.len(),
                        });
                    }
                    let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                    let opened_values = Self::verify_npo_witness_values(
                        expected_npo_index,
                        &expected_witness_ids,
                        &witness_values,
                    )?;
                    let evaluation = self.evaluate_sampled_recompose_npo_row_values::<F>(
                        *expected_index,
                        op_type,
                        row,
                        callsite,
                        &[],
                        &expected_witness_ids,
                        &opened_values,
                    )?;
                    let recomputed_validity = Self::terminal_npo_row_evaluation_component_value::<F>(
                        &evaluation,
                        expected_component_offset,
                    )?;
                    if validity != recomputed_validity {
                        return Err(
                            NativeTerminalVerifyError::TerminalNpoValidityResidualMismatch {
                                query,
                                npo_index: expected_npo_index,
                            },
                        );
                    }
                    if validity != F::ZERO {
                        return Err(NativeTerminalVerifyError::TerminalNpoValidityNonZero {
                            query,
                            npo_index: expected_npo_index,
                        });
                    }
                }
            }
        }

        Ok(plan)
    }

    pub fn prove_terminal_combined_validity_consistency_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_oracle: &TerminalOracleMerkleTree,
        combined_oracle: &TerminalOracleMerkleTree,
        combined_fold_proof: &TerminalResidualFoldProof,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalCombinedValidityConsistencyProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let witness_commitment = witness_oracle.commitment();
        let combined_commitment = combined_oracle.commitment();
        Self::verify_prelude_binds_commitment(prelude, &witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, &combined_commitment)?;

        let relation = verifying_key.primitive_quadratic_relation()?;
        let quadratic_len = relation.constraints.len();
        let plan = self.derive_terminal_combined_validity_fold_query_plan(
            prelude,
            &combined_commitment,
            &combined_fold_proof.fold_commitments,
        )?;

        let mut openings = Vec::with_capacity(plan.indices.len());
        for validity_index in &plan.indices {
            if *validity_index < quadratic_len {
                let constraint = &relation.constraints[*validity_index];
                let witness_ids = Self::quadratic_constraint_witness_ids(constraint);
                let mut witness_openings = Vec::with_capacity(witness_ids.len());
                for witness_id in witness_ids {
                    let value = witness.traces.witness_trace.get_value(witness_id).ok_or(
                        NativeTerminalVerifyError::MissingWitness {
                            witness_id: witness_id.0,
                        },
                    )?;
                    witness_openings
                        .push(witness_oracle.open_goldilocks_value(witness_id.0 as usize, value)?);
                }
                openings.push(TerminalCombinedValidityConsistencyOpening::Quadratic {
                    validity_index: *validity_index,
                    witness_openings,
                });
            } else {
                let npo_validity_index = *validity_index - quadratic_len;
                let (npo_index, component_offset) = Self::terminal_npo_validity_component_row::<F>(
                    verifying_key,
                    npo_validity_index,
                )
                .ok_or(
                    NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                        query: 0,
                        expected: npo_validity_index,
                        got: npo_validity_index,
                    },
                )?;
                let npo_opening = self.prove_terminal_npo_opening_goldilocks(
                    verifying_key,
                    witness_oracle,
                    witness,
                    npo_index,
                )?;
                openings.push(TerminalCombinedValidityConsistencyOpening::Npo {
                    validity_index: *validity_index,
                    npo_index,
                    component_offset,
                    npo_opening,
                });
            }
        }

        Ok(TerminalCombinedValidityConsistencyProof { openings })
    }

    pub fn verify_terminal_combined_validity_consistency_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_commitment: &TerminalOracleCommitment,
        combined_commitment: &TerminalOracleCommitment,
        combined_fold_proof: &TerminalResidualFoldProof,
        proof: &TerminalCombinedValidityConsistencyProof,
    ) -> Result<TerminalQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_witness_commitment_identity(verifying_key, witness_commitment)?;
        Self::verify_terminal_combined_validity_commitment_identity::<F>(
            verifying_key,
            combined_commitment,
        )?;
        Self::verify_prelude_binds_commitment(prelude, witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, combined_commitment)?;

        let relation = verifying_key.primitive_quadratic_relation()?;
        let quadratic_len = relation.constraints.len();
        let plan = self.verify_terminal_combined_validity_fold_goldilocks::<F>(
            verifying_key,
            public_inputs,
            prelude,
            combined_commitment,
            combined_fold_proof,
        )?;
        if proof.openings.len() != plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalQuadraticConsistencyQueryLengthMismatch {
                    expected: plan.indices.len(),
                    got: proof.openings.len(),
                },
            );
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&plan.indices).enumerate()
        {
            let validity_basis = Self::terminal_fold_base_value_basis(
                &combined_fold_proof.openings[query],
                &combined_fold_proof.final_value_basis,
            )
            .ok_or(
                NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                    query,
                    round: 0,
                },
            )?;
            let opened_validity = Self::field_from_goldilocks_basis_u64::<F>(validity_basis)?;

            match opening {
                TerminalCombinedValidityConsistencyOpening::Quadratic {
                    validity_index,
                    witness_openings,
                } => {
                    if validity_index != expected_index {
                        return Err(
                            NativeTerminalVerifyError::TerminalQuadraticConsistencyQueryIndexMismatch {
                                query,
                                expected: *expected_index,
                                got: *validity_index,
                            },
                        );
                    }
                    if *validity_index >= quadratic_len {
                        return Err(
                            NativeTerminalVerifyError::TerminalQuadraticConsistencyQueryIndexMismatch {
                                query,
                                expected: *expected_index,
                                got: *validity_index,
                            },
                        );
                    }
                    let constraint = &relation.constraints[*validity_index];
                    let witness_ids = Self::quadratic_constraint_witness_ids(constraint);
                    if witness_openings.len() != witness_ids.len() {
                        return Err(
                            NativeTerminalVerifyError::TerminalQuadraticConsistencyOpeningCountMismatch {
                                quadratic_index: *validity_index,
                                expected: witness_ids.len(),
                                got: witness_openings.len(),
                            },
                        );
                    }

                    let mut witness_values = Vec::with_capacity(witness_ids.len());
                    for (opening_idx, (expected_witness, witness_opening)) in witness_ids
                        .iter()
                        .copied()
                        .zip(witness_openings)
                        .enumerate()
                    {
                        if witness_opening.index != expected_witness.0 as usize {
                            return Err(
                                NativeTerminalVerifyError::TerminalQuadraticConsistencyOpeningWitnessMismatch {
                                    quadratic_index: *validity_index,
                                    opening: opening_idx,
                                    expected: expected_witness.0,
                                    got: witness_opening.index,
                                },
                            );
                        }
                        self.verify_terminal_oracle_opening(witness_commitment, witness_opening)?;
                        witness_values.push(Self::field_from_goldilocks_basis_u64::<F>(
                            &witness_opening.value_basis,
                        )?);
                    }

                    let a = Self::evaluate_opened_linear_expression(
                        &constraint.a,
                        public_inputs,
                        &witness_ids,
                        &witness_values,
                    )?;
                    let b = Self::evaluate_opened_linear_expression(
                        &constraint.b,
                        public_inputs,
                        &witness_ids,
                        &witness_values,
                    )?;
                    let c = Self::evaluate_opened_linear_expression(
                        &constraint.c,
                        public_inputs,
                        &witness_ids,
                        &witness_values,
                    )?;
                    let recomputed_residual = a * b - c;
                    if opened_validity != recomputed_residual {
                        return Err(
                            NativeTerminalVerifyError::TerminalQuadraticConsistencyResidualMismatch {
                                query,
                                quadratic_index: *validity_index,
                            },
                        );
                    }
                    if opened_validity != F::ZERO {
                        return Err(
                            NativeTerminalVerifyError::TerminalQuadraticResidualNonZero {
                                query,
                                residual_index: *validity_index,
                            },
                        );
                    }
                }
                TerminalCombinedValidityConsistencyOpening::Npo {
                    validity_index,
                    npo_index,
                    component_offset,
                    npo_opening,
                } => {
                    if validity_index != expected_index {
                        return Err(
                            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                                query,
                                expected: *expected_index,
                                got: *validity_index,
                            },
                        );
                    }
                    if *validity_index < quadratic_len {
                        return Err(
                            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                                query,
                                expected: *expected_index,
                                got: *validity_index,
                            },
                        );
                    }
                    let expected_npo_validity_index = *validity_index - quadratic_len;
                    let (expected_npo_index, expected_component_offset) =
                        Self::terminal_npo_validity_component_row::<F>(
                            verifying_key,
                            expected_npo_validity_index,
                        )
                        .ok_or(
                            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                                query,
                                expected: expected_npo_validity_index,
                                got: expected_npo_validity_index,
                            },
                        )?;
                    if *npo_index != expected_npo_index {
                        return Err(
                            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                                query,
                                expected: expected_npo_index,
                                got: *npo_index,
                            },
                        );
                    }
                    if *component_offset != expected_component_offset {
                        return Err(
                            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                                query,
                                expected: expected_component_offset,
                                got: *component_offset,
                            },
                        );
                    }
                    if npo_opening.npo_index != *npo_index {
                        return Err(NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                            query,
                            expected: *npo_index,
                            got: npo_opening.npo_index,
                        });
                    }
                    let row_ref = Self::terminal_npo_row(verifying_key, *npo_index).ok_or(
                        NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                            query,
                            expected: *npo_index,
                            got: *npo_index,
                        },
                    )?;
                    match row_ref {
                        NativeTerminalNpoRowRef::Tip5 { row, callsite, .. } => {
                            let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                            let opened_values = self.verify_npo_witness_openings::<F>(
                                *npo_index,
                                &expected_witness_ids,
                                witness_commitment,
                                &npo_opening.witness_openings,
                            )?;
                            let evaluation = self.evaluate_sampled_tip5_npo_row_values::<F>(
                                *npo_index,
                                row,
                                callsite,
                                &npo_opening.tip5_hidden_input_values,
                                &expected_witness_ids,
                                &opened_values,
                            )?;
                            let recomputed_validity =
                                Self::terminal_npo_row_evaluation_component_value::<F>(
                                    &evaluation,
                                    expected_component_offset,
                                )?;
                            if opened_validity != recomputed_validity {
                                return Err(
                                    NativeTerminalVerifyError::TerminalNpoValidityResidualMismatch {
                                        query,
                                        npo_index: *npo_index,
                                    },
                                );
                            }
                            if opened_validity != F::ZERO {
                                return Err(
                                    NativeTerminalVerifyError::TerminalNpoValidityNonZero {
                                        query,
                                        npo_index: *npo_index,
                                    },
                                );
                            }
                        }
                        NativeTerminalNpoRowRef::Recompose {
                            op_type,
                            row,
                            callsite,
                        } => {
                            let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                            let opened_values = self.verify_npo_witness_openings::<F>(
                                *npo_index,
                                &expected_witness_ids,
                                witness_commitment,
                                &npo_opening.witness_openings,
                            )?;
                            let evaluation = self.evaluate_sampled_recompose_npo_row_values::<F>(
                                *npo_index,
                                op_type,
                                row,
                                callsite,
                                &npo_opening.tip5_hidden_input_values,
                                &expected_witness_ids,
                                &opened_values,
                            )?;
                            let recomputed_validity =
                                Self::terminal_npo_row_evaluation_component_value::<F>(
                                    &evaluation,
                                    expected_component_offset,
                                )?;
                            if opened_validity != recomputed_validity {
                                return Err(
                                    NativeTerminalVerifyError::TerminalNpoValidityResidualMismatch {
                                        query,
                                        npo_index: *npo_index,
                                    },
                                );
                            }
                            if opened_validity != F::ZERO {
                                return Err(
                                    NativeTerminalVerifyError::TerminalNpoValidityNonZero {
                                        query,
                                        npo_index: *npo_index,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(plan)
    }

    #[cfg(test)]
    fn prove_terminal_local_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
        parameters: TerminalProofParameters,
    ) -> Result<TerminalLocalProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_assignment_with_goldilocks_npos(verifying_key, witness)?;
        let witness_values = Self::terminal_witness_values(witness)?;
        let witness_oracle = TerminalOracleMerkleTree::commit_goldilocks_values(
            Self::witness_oracle_label(),
            &witness_values,
        )?;
        let witness_commitment = witness_oracle.commitment();
        let combined_validity_oracle = self.commit_terminal_combined_validity_goldilocks(
            verifying_key,
            public_inputs,
            witness,
        )?;
        let combined_validity_commitment = combined_validity_oracle.commitment();
        let prelude_commitments = vec![witness_commitment.root, combined_validity_commitment.root];
        let prelude = self.build_proof_prelude_goldilocks(
            verifying_key,
            public_inputs,
            parameters,
            prelude_commitments,
        )?;

        let combined_validity_fold_proof = self.prove_terminal_combined_validity_fold_goldilocks(
            verifying_key,
            public_inputs,
            &prelude,
            &combined_validity_oracle,
            witness,
        )?;
        let combined_validity_consistency_proof = self
            .prove_terminal_combined_validity_consistency_goldilocks(
                verifying_key,
                public_inputs,
                &prelude,
                &witness_oracle,
                &combined_validity_oracle,
                &combined_validity_fold_proof,
                witness,
            )?;

        Ok(TerminalLocalProof {
            prelude,
            witness_commitment,
            combined_validity_commitment,
            combined_validity_consistency_proof,
            combined_validity_fold_proof,
        })
    }

    pub fn prove_terminal_production_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalProductionProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.validate_goldilocks_production_query_domains(
            verifying_key,
            TerminalProofParameters::production_60bit(),
        )?;
        self.verify_assignment_with_goldilocks_npos(verifying_key, witness)?;

        let witness_values = Self::terminal_witness_values(witness)?;
        let witness_oracle = TerminalOracleMerkleTree::commit_goldilocks_values(
            Self::witness_oracle_label(),
            &witness_values,
        )?;
        let witness_commitment = witness_oracle.commitment();
        let assignment_oracle =
            self.commit_terminal_assignment_goldilocks(verifying_key, public_inputs, witness)?;
        let assignment_commitment = assignment_oracle.commitment();

        let prelude_commitments = vec![witness_commitment.root, assignment_commitment.root];
        let prelude = self.build_proof_prelude_goldilocks(
            verifying_key,
            public_inputs,
            TerminalProofParameters::production_60bit(),
            prelude_commitments,
        )?;
        let primitive_r1cs_proof = self.prove_terminal_r1cs_row_product_sumcheck_goldilocks(
            verifying_key,
            public_inputs,
            &prelude,
            &assignment_oracle,
            witness,
        )?;
        let npo_exhaustive_proof = if Self::terminal_npo_domain_len(verifying_key) > 0 {
            Some(self.prove_terminal_npo_exhaustive_goldilocks(
                verifying_key,
                public_inputs,
                &prelude,
                &witness_oracle,
                witness,
            )?)
        } else {
            None
        };

        Ok(TerminalProductionProof {
            prelude,
            witness_commitment,
            assignment_commitment,
            primitive_r1cs_proof,
            npo_exhaustive_proof,
        })
    }

    pub fn commit_terminal_assignment_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalOracleMerkleTree, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let witness_values = Self::terminal_witness_values(witness)?;
        let assignment_values =
            Self::terminal_assignment_values(verifying_key, public_inputs, &witness_values)?;
        TerminalOracleMerkleTree::commit_goldilocks_values(
            Self::assignment_oracle_label(),
            &assignment_values,
        )
    }

    pub fn prove_terminal_assignment_evaluation_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalAssignmentEvaluationProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let assignment_commitment = assignment_oracle.commitment();
        Self::verify_terminal_assignment_commitment_identity(
            verifying_key,
            &assignment_commitment,
        )?;
        Self::verify_prelude_binds_commitment(prelude, &assignment_commitment)?;

        let witness_values = Self::terminal_witness_values(witness)?;
        let assignment_values =
            Self::terminal_assignment_values(verifying_key, public_inputs, &witness_values)?;
        let public_prefix_len = 1 + verifying_key.header.fingerprint.public_flat_len;
        let public_prefix_proof =
            assignment_oracle.open_goldilocks_prefix(&assignment_values[..public_prefix_len])?;

        let mut layers = vec![assignment_values];
        let mut trees = Vec::new();
        let mut fold_commitments = Vec::new();
        let mut round = 0usize;
        while layers.last().expect("base assignment layer exists").len() > 1 {
            let challenge = Self::derive_terminal_assignment_fold_challenge::<F>(
                prelude,
                &assignment_commitment,
                &fold_commitments,
                round,
            )?;
            let current = layers.last().expect("current assignment layer exists");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(F::ZERO);
                next.push(left * (F::ONE - challenge) + right * challenge);
            }
            let label = Self::assignment_fold_oracle_label(round);
            let tree = TerminalOracleMerkleTree::commit_goldilocks_values(label, &next)?;
            fold_commitments.push(tree.commitment());
            trees.push(tree);
            layers.push(next);
            round += 1;
        }

        let final_value = layers
            .last()
            .and_then(|layer| layer.first())
            .copied()
            .ok_or(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty)?;
        let final_value_basis = Self::goldilocks_basis_u64(&final_value);
        let query_plan = self.derive_terminal_assignment_fold_query_plan(
            prelude,
            &assignment_commitment,
            &fold_commitments,
        )?;

        let mut expected_round_indices = Self::terminal_fold_round_indices(
            &query_plan.indices,
            assignment_commitment.values_len,
            fold_commitments.len(),
        );
        let mut round_openings = Vec::with_capacity(fold_commitments.len());
        for (round, round_indices) in expected_round_indices.iter_mut().enumerate() {
            round_indices.sort_unstable();
            let current_tree = if round == 0 {
                assignment_oracle
            } else {
                &trees[round - 1]
            };
            let current_values = &layers[round];
            let mut opening_values = Vec::with_capacity(round_indices.len());
            for index in round_indices {
                opening_values.push((*index, &current_values[*index]));
            }
            round_openings.push(current_tree.open_goldilocks_multi_values(&opening_values)?);
        }

        let mut openings = Vec::with_capacity(query_plan.indices.len());
        for initial_index in &query_plan.indices {
            openings.push(TerminalResidualFoldQueryOpening {
                initial_index: *initial_index,
                rounds: Vec::new(),
            });
        }

        Ok(TerminalAssignmentEvaluationProof {
            public_prefix_proof,
            fold_commitments,
            final_value_basis,
            round_openings,
            openings,
        })
    }

    pub fn prove_terminal_assignment_evaluation_at_point_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
        point: &[F],
    ) -> Result<TerminalAssignmentEvaluationProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let assignment_commitment = assignment_oracle.commitment();
        Self::verify_terminal_assignment_commitment_identity(
            verifying_key,
            &assignment_commitment,
        )?;
        Self::verify_prelude_binds_commitment(prelude, &assignment_commitment)?;
        Self::validate_terminal_assignment_point(verifying_key, point)?;

        let witness_values = Self::terminal_witness_values(witness)?;
        let assignment_values =
            Self::terminal_assignment_values(verifying_key, public_inputs, &witness_values)?;
        let public_prefix_len = 1 + verifying_key.header.fingerprint.public_flat_len;
        let public_prefix_proof =
            assignment_oracle.open_goldilocks_prefix(&assignment_values[..public_prefix_len])?;

        let mut layers = vec![assignment_values];
        let mut trees = Vec::new();
        let mut fold_commitments = Vec::new();
        for (round, challenge) in point.iter().copied().enumerate() {
            let current = layers.last().expect("current assignment layer exists");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(F::ZERO);
                next.push(left * (F::ONE - challenge) + right * challenge);
            }
            let label = Self::assignment_fold_oracle_label(round);
            let tree = TerminalOracleMerkleTree::commit_goldilocks_values(label, &next)?;
            fold_commitments.push(tree.commitment());
            trees.push(tree);
            layers.push(next);
        }

        let final_value = layers
            .last()
            .and_then(|layer| layer.first())
            .copied()
            .ok_or(NativeTerminalVerifyError::TerminalQuadraticResidualQueryDomainEmpty)?;
        let final_value_basis = Self::goldilocks_basis_u64(&final_value);
        let query_plan = self.derive_terminal_assignment_fold_query_plan(
            prelude,
            &assignment_commitment,
            &fold_commitments,
        )?;

        let mut expected_round_indices = Self::terminal_fold_round_indices(
            &query_plan.indices,
            assignment_commitment.values_len,
            fold_commitments.len(),
        );
        let mut round_openings = Vec::with_capacity(fold_commitments.len());
        for (round, round_indices) in expected_round_indices.iter_mut().enumerate() {
            round_indices.sort_unstable();
            let current_tree = if round == 0 {
                assignment_oracle
            } else {
                &trees[round - 1]
            };
            let current_values = &layers[round];
            let mut opening_values = Vec::with_capacity(round_indices.len());
            for index in round_indices {
                opening_values.push((*index, &current_values[*index]));
            }
            round_openings.push(current_tree.open_goldilocks_multi_values(&opening_values)?);
        }

        let mut openings = Vec::with_capacity(query_plan.indices.len());
        for initial_index in &query_plan.indices {
            openings.push(TerminalResidualFoldQueryOpening {
                initial_index: *initial_index,
                rounds: Vec::new(),
            });
        }

        Ok(TerminalAssignmentEvaluationProof {
            public_prefix_proof,
            fold_commitments,
            final_value_basis,
            round_openings,
            openings,
        })
    }

    pub fn verify_terminal_assignment_evaluation_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        proof: &TerminalAssignmentEvaluationProof,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_assignment_commitment_identity(verifying_key, assignment_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, assignment_commitment)?;
        self.validate_terminal_assignment_fold_commitments::<F>(
            verifying_key,
            assignment_commitment,
            &proof.fold_commitments,
        )?;

        let public_prefix_len = 1 + verifying_key.header.fingerprint.public_flat_len;
        let mut expected_public_prefix = Vec::with_capacity(public_prefix_len);
        expected_public_prefix.push(F::ONE);
        expected_public_prefix.extend_from_slice(public_inputs);
        self.verify_terminal_oracle_prefix_goldilocks(
            assignment_commitment,
            &proof.public_prefix_proof,
            &expected_public_prefix,
        )?;

        let query_plan = self.derive_terminal_assignment_fold_query_plan(
            prelude,
            assignment_commitment,
            &proof.fold_commitments,
        )?;
        self.verify_terminal_compact_fold_openings_goldilocks::<F, _>(
            assignment_commitment,
            &proof.fold_commitments,
            &proof.final_value_basis,
            &proof.round_openings,
            &proof.openings,
            &query_plan,
            |round, prior| {
                Self::derive_terminal_assignment_fold_challenge::<F>(
                    prelude,
                    assignment_commitment,
                    prior,
                    round,
                )
            },
        )?;

        Self::field_from_goldilocks_basis_u64::<F>(&proof.final_value_basis)
    }

    pub fn verify_terminal_assignment_evaluation_at_point_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        proof: &TerminalAssignmentEvaluationProof,
        point: &[F],
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_assignment_commitment_identity(verifying_key, assignment_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, assignment_commitment)?;
        Self::validate_terminal_assignment_point(verifying_key, point)?;
        self.validate_terminal_assignment_fold_commitments::<F>(
            verifying_key,
            assignment_commitment,
            &proof.fold_commitments,
        )?;

        let public_prefix_len = 1 + verifying_key.header.fingerprint.public_flat_len;
        let mut expected_public_prefix = Vec::with_capacity(public_prefix_len);
        expected_public_prefix.push(F::ONE);
        expected_public_prefix.extend_from_slice(public_inputs);
        self.verify_terminal_oracle_prefix_goldilocks(
            assignment_commitment,
            &proof.public_prefix_proof,
            &expected_public_prefix,
        )?;

        let query_plan = self.derive_terminal_assignment_fold_query_plan(
            prelude,
            assignment_commitment,
            &proof.fold_commitments,
        )?;
        self.verify_terminal_compact_fold_openings_goldilocks::<F, _>(
            assignment_commitment,
            &proof.fold_commitments,
            &proof.final_value_basis,
            &proof.round_openings,
            &proof.openings,
            &query_plan,
            |round, _prior| Ok(point[round]),
        )?;

        Self::field_from_goldilocks_basis_u64::<F>(&proof.final_value_basis)
    }

    pub fn prove_terminal_sparse_r1cs_sumcheck_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalSparseR1csSumcheckProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let assignment_commitment = assignment_oracle.commitment();
        Self::verify_terminal_assignment_commitment_identity(
            verifying_key,
            &assignment_commitment,
        )?;
        Self::verify_prelude_binds_commitment(prelude, &assignment_commitment)?;

        let sparse_relation = verifying_key.primitive_sparse_r1cs_relation()?;
        let row_point = Self::derive_terminal_r1cs_row_point::<F>(
            prelude,
            &assignment_commitment,
            sparse_relation.log_rows,
        )?;
        self.prove_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks(
            verifying_key,
            public_inputs,
            prelude,
            assignment_oracle,
            witness,
            &row_point,
        )
    }

    pub fn prove_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
        row_point: &[F],
    ) -> Result<TerminalSparseR1csSumcheckProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let assignment_commitment = assignment_oracle.commitment();
        Self::verify_terminal_assignment_commitment_identity(
            verifying_key,
            &assignment_commitment,
        )?;
        Self::verify_prelude_binds_commitment(prelude, &assignment_commitment)?;

        let sparse_relation = verifying_key.primitive_sparse_r1cs_relation()?;
        Self::validate_terminal_r1cs_row_point(&sparse_relation, row_point)?;
        let witness_values = Self::terminal_witness_values(witness)?;
        let assignment_values =
            Self::terminal_assignment_values(verifying_key, public_inputs, &witness_values)?;
        let (claimed_a, claimed_b, claimed_c) = Self::sparse_r1cs_matrix_evaluations_at_row(
            &sparse_relation,
            &row_point,
            &assignment_values,
        )?;
        let claimed_a_basis = Self::goldilocks_basis_u64(&claimed_a);
        let claimed_b_basis = Self::goldilocks_basis_u64(&claimed_b);
        let claimed_c_basis = Self::goldilocks_basis_u64(&claimed_c);
        let alpha = Self::derive_terminal_r1cs_batch_challenge::<F>(
            prelude,
            &assignment_commitment,
            &row_point,
            &claimed_a_basis,
            &claimed_b_basis,
            &claimed_c_basis,
        )?;
        let alpha_sq = alpha.square();
        let mut current_claim = claimed_a + alpha * claimed_b + alpha_sq * claimed_c;
        let mut variable_point = Vec::with_capacity(sparse_relation.log_variables);
        let mut rounds = Vec::with_capacity(sparse_relation.log_variables);
        let mut current_matrix_values =
            Self::sparse_r1cs_matrix_combo_values(&sparse_relation, &row_point, alpha);
        let mut current_assignment_values =
            Self::pad_terminal_values_to_mle_len(&assignment_values, sparse_relation.log_variables);

        for round in 0..sparse_relation.log_variables {
            let evals = Self::sparse_r1cs_folded_round_evaluations(
                &current_matrix_values,
                &current_assignment_values,
            );
            if evals[0] + evals[1] != current_claim {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                        query: 0,
                        round,
                    },
                );
            }
            rounds.push(TerminalR1csSumcheckRound {
                eval_0_basis: Self::goldilocks_basis_u64(&evals[0]),
                eval_1_basis: Self::goldilocks_basis_u64(&evals[1]),
                eval_2_basis: Self::goldilocks_basis_u64(&evals[2]),
            });
            let challenge = Self::derive_terminal_r1cs_sumcheck_round_challenge::<F>(
                prelude,
                &assignment_commitment,
                &row_point,
                &claimed_a_basis,
                &claimed_b_basis,
                &claimed_c_basis,
                &current_claim,
                &rounds,
                round,
            )?;
            variable_point.push(challenge);
            current_claim = Self::interpolate_degree_two_at(evals, challenge);
            current_matrix_values = Self::fold_terminal_values(&current_matrix_values, challenge);
            current_assignment_values =
                Self::fold_terminal_values(&current_assignment_values, challenge);
        }

        let assignment_evaluation = self.prove_terminal_assignment_evaluation_at_point_goldilocks(
            verifying_key,
            public_inputs,
            prelude,
            assignment_oracle,
            witness,
            &variable_point,
        )?;
        let assignment_eval =
            Self::field_from_goldilocks_basis_u64::<F>(&assignment_evaluation.final_value_basis)?;
        let matrix_eval = current_matrix_values.first().copied().ok_or(
            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                query: 0,
                round: sparse_relation.log_variables,
            },
        )?;
        if current_claim != matrix_eval * assignment_eval {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                    query: 0,
                    round: sparse_relation.log_variables,
                },
            );
        }

        Ok(TerminalSparseR1csSumcheckProof {
            claimed_a_basis,
            claimed_b_basis,
            claimed_c_basis,
            rounds,
            assignment_evaluation,
        })
    }

    pub fn verify_terminal_sparse_r1cs_sumcheck_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        proof: &TerminalSparseR1csSumcheckProof,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_assignment_commitment_identity(verifying_key, assignment_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, assignment_commitment)?;
        let sparse_relation = verifying_key.primitive_sparse_r1cs_relation()?;
        let row_point = Self::derive_terminal_r1cs_row_point::<F>(
            prelude,
            assignment_commitment,
            sparse_relation.log_rows,
        )?;
        self.verify_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks(
            verifying_key,
            public_inputs,
            prelude,
            assignment_commitment,
            proof,
            &row_point,
        )
    }

    pub fn verify_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        proof: &TerminalSparseR1csSumcheckProof,
        row_point: &[F],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_assignment_commitment_identity(verifying_key, assignment_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, assignment_commitment)?;
        let sparse_relation = verifying_key.primitive_sparse_r1cs_relation()?;
        Self::validate_terminal_r1cs_row_point(&sparse_relation, row_point)?;
        if proof.rounds.len() != sparse_relation.log_variables {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldRoundCountMismatch {
                    query: 0,
                    expected: sparse_relation.log_variables,
                    got: proof.rounds.len(),
                },
            );
        }

        let claimed_a = Self::field_from_goldilocks_basis_u64::<F>(&proof.claimed_a_basis)?;
        let claimed_b = Self::field_from_goldilocks_basis_u64::<F>(&proof.claimed_b_basis)?;
        let claimed_c = Self::field_from_goldilocks_basis_u64::<F>(&proof.claimed_c_basis)?;
        let alpha = Self::derive_terminal_r1cs_batch_challenge::<F>(
            prelude,
            assignment_commitment,
            &row_point,
            &proof.claimed_a_basis,
            &proof.claimed_b_basis,
            &proof.claimed_c_basis,
        )?;
        let alpha_sq = alpha.square();
        let mut current_claim = claimed_a + alpha * claimed_b + alpha_sq * claimed_c;
        let mut variable_point = Vec::with_capacity(sparse_relation.log_variables);

        for (round, round_proof) in proof.rounds.iter().enumerate() {
            let eval_0 = Self::field_from_goldilocks_basis_u64::<F>(&round_proof.eval_0_basis)?;
            let eval_1 = Self::field_from_goldilocks_basis_u64::<F>(&round_proof.eval_1_basis)?;
            let eval_2 = Self::field_from_goldilocks_basis_u64::<F>(&round_proof.eval_2_basis)?;
            let evals = [eval_0, eval_1, eval_2];
            if eval_0 + eval_1 != current_claim {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                        query: 0,
                        round,
                    },
                );
            }
            let challenge = Self::derive_terminal_r1cs_sumcheck_round_challenge::<F>(
                prelude,
                assignment_commitment,
                &row_point,
                &proof.claimed_a_basis,
                &proof.claimed_b_basis,
                &proof.claimed_c_basis,
                &current_claim,
                &proof.rounds[..=round],
                round,
            )?;
            variable_point.push(challenge);
            current_claim = Self::interpolate_degree_two_at(evals, challenge);
        }

        let assignment_eval = self.verify_terminal_assignment_evaluation_at_point_goldilocks(
            verifying_key,
            public_inputs,
            prelude,
            assignment_commitment,
            &proof.assignment_evaluation,
            &variable_point,
        )?;
        let matrix_eval = Self::evaluate_sparse_r1cs_matrix_combo_at_point(
            &sparse_relation,
            &row_point,
            &variable_point,
            alpha,
        );
        if current_claim != matrix_eval * assignment_eval {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                    query: 0,
                    round: sparse_relation.log_variables,
                },
            );
        }
        Ok(())
    }

    pub fn prove_terminal_r1cs_row_product_sumcheck_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalR1csRowProductSumcheckProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        let assignment_commitment = assignment_oracle.commitment();
        Self::verify_terminal_assignment_commitment_identity(
            verifying_key,
            &assignment_commitment,
        )?;
        Self::verify_prelude_binds_commitment(prelude, &assignment_commitment)?;

        let sparse_relation = verifying_key.primitive_sparse_r1cs_relation()?;
        let witness_values = Self::terminal_witness_values(witness)?;
        let assignment_values =
            Self::terminal_assignment_values(verifying_key, public_inputs, &witness_values)?;
        let anchor_point = Self::derive_terminal_r1cs_row_product_anchor_point::<F>(
            prelude,
            &assignment_commitment,
            sparse_relation.log_rows,
        )?;
        let (mut eq_values, mut a_values, mut b_values, mut c_values) =
            Self::sparse_r1cs_row_product_initial_vectors(
                &sparse_relation,
                &anchor_point,
                &assignment_values,
            )?;
        let mut current_claim =
            Self::row_product_claim(&eq_values, &a_values, &b_values, &c_values);
        if current_claim != F::ZERO {
            return Err(
                NativeTerminalVerifyError::TerminalQuadraticConstraintViolation {
                    quadratic_index: 0,
                    source_constraint_index: 0,
                    kind: "sparse_r1cs_row_product",
                },
            );
        }

        let mut row_point = Vec::with_capacity(sparse_relation.log_rows);
        let mut rounds = Vec::with_capacity(sparse_relation.log_rows);
        for round in 0..sparse_relation.log_rows {
            let evals = Self::sparse_r1cs_row_product_round_evaluations(
                &eq_values, &a_values, &b_values, &c_values,
            );
            if evals[0] + evals[1] != current_claim {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                        query: 0,
                        round,
                    },
                );
            }
            rounds.push(TerminalR1csRowProductRound {
                eval_0_basis: Self::goldilocks_basis_u64(&evals[0]),
                eval_1_basis: Self::goldilocks_basis_u64(&evals[1]),
                eval_2_basis: Self::goldilocks_basis_u64(&evals[2]),
                eval_3_basis: Self::goldilocks_basis_u64(&evals[3]),
            });
            let challenge = Self::derive_terminal_r1cs_row_product_round_challenge::<F>(
                prelude,
                &assignment_commitment,
                &anchor_point,
                &current_claim,
                &rounds,
                round,
            )?;
            row_point.push(challenge);
            current_claim = Self::interpolate_degree_three_at(evals, challenge);
            eq_values = Self::fold_terminal_values(&eq_values, challenge);
            a_values = Self::fold_terminal_values(&a_values, challenge);
            b_values = Self::fold_terminal_values(&b_values, challenge);
            c_values = Self::fold_terminal_values(&c_values, challenge);
        }

        let matrix_sumcheck = self.prove_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks(
            verifying_key,
            public_inputs,
            prelude,
            assignment_oracle,
            witness,
            &row_point,
        )?;
        let claimed_a =
            Self::field_from_goldilocks_basis_u64::<F>(&matrix_sumcheck.claimed_a_basis)?;
        let claimed_b =
            Self::field_from_goldilocks_basis_u64::<F>(&matrix_sumcheck.claimed_b_basis)?;
        let claimed_c =
            Self::field_from_goldilocks_basis_u64::<F>(&matrix_sumcheck.claimed_c_basis)?;
        let eq_eval = Self::terminal_eq_index_point_pair(&anchor_point, &row_point);
        if current_claim != eq_eval * (claimed_a * claimed_b - claimed_c) {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                    query: 0,
                    round: sparse_relation.log_rows,
                },
            );
        }

        Ok(TerminalR1csRowProductSumcheckProof {
            rounds,
            matrix_sumcheck,
        })
    }

    pub fn verify_terminal_r1cs_row_product_sumcheck_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        proof: &TerminalR1csRowProductSumcheckProof,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_assignment_commitment_identity(verifying_key, assignment_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, assignment_commitment)?;
        let sparse_relation = verifying_key.primitive_sparse_r1cs_relation()?;
        if proof.rounds.len() != sparse_relation.log_rows {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldRoundCountMismatch {
                    query: 0,
                    expected: sparse_relation.log_rows,
                    got: proof.rounds.len(),
                },
            );
        }

        let anchor_point = Self::derive_terminal_r1cs_row_product_anchor_point::<F>(
            prelude,
            assignment_commitment,
            sparse_relation.log_rows,
        )?;
        let mut current_claim = F::ZERO;
        let mut row_point = Vec::with_capacity(sparse_relation.log_rows);
        for (round, round_proof) in proof.rounds.iter().enumerate() {
            let eval_0 = Self::field_from_goldilocks_basis_u64::<F>(&round_proof.eval_0_basis)?;
            let eval_1 = Self::field_from_goldilocks_basis_u64::<F>(&round_proof.eval_1_basis)?;
            let eval_2 = Self::field_from_goldilocks_basis_u64::<F>(&round_proof.eval_2_basis)?;
            let eval_3 = Self::field_from_goldilocks_basis_u64::<F>(&round_proof.eval_3_basis)?;
            let evals = [eval_0, eval_1, eval_2, eval_3];
            if eval_0 + eval_1 != current_claim {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                        query: 0,
                        round,
                    },
                );
            }
            let challenge = Self::derive_terminal_r1cs_row_product_round_challenge::<F>(
                prelude,
                assignment_commitment,
                &anchor_point,
                &current_claim,
                &proof.rounds[..=round],
                round,
            )?;
            row_point.push(challenge);
            current_claim = Self::interpolate_degree_three_at(evals, challenge);
        }

        self.verify_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks(
            verifying_key,
            public_inputs,
            prelude,
            assignment_commitment,
            &proof.matrix_sumcheck,
            &row_point,
        )?;
        let claimed_a =
            Self::field_from_goldilocks_basis_u64::<F>(&proof.matrix_sumcheck.claimed_a_basis)?;
        let claimed_b =
            Self::field_from_goldilocks_basis_u64::<F>(&proof.matrix_sumcheck.claimed_b_basis)?;
        let claimed_c =
            Self::field_from_goldilocks_basis_u64::<F>(&proof.matrix_sumcheck.claimed_c_basis)?;
        let eq_eval = Self::terminal_eq_index_point_pair(&anchor_point, &row_point);
        if current_claim != eq_eval * (claimed_a * claimed_b - claimed_c) {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                    query: 0,
                    round: sparse_relation.log_rows,
                },
            );
        }

        Ok(())
    }

    pub fn verify_terminal_production_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        proof: &TerminalProductionProof,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        Self::validate_terminal_production_parameters(proof.prelude.parameters)?;
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, &proof.prelude)?;
        self.validate_goldilocks_production_query_domains(verifying_key, proof.prelude.parameters)?;
        Self::verify_terminal_witness_commitment_identity(
            verifying_key,
            &proof.witness_commitment,
        )?;
        Self::verify_prelude_binds_commitment(&proof.prelude, &proof.witness_commitment)?;
        Self::verify_terminal_assignment_commitment_identity(
            verifying_key,
            &proof.assignment_commitment,
        )?;
        Self::verify_prelude_binds_commitment(&proof.prelude, &proof.assignment_commitment)?;

        self.verify_terminal_r1cs_row_product_sumcheck_goldilocks(
            verifying_key,
            public_inputs,
            &proof.prelude,
            &proof.assignment_commitment,
            &proof.primitive_r1cs_proof,
        )?;

        match (
            Self::terminal_npo_domain_len(verifying_key),
            &proof.npo_exhaustive_proof,
        ) {
            (0, None) => {}
            (0, Some(_)) => {
                return Err(NativeTerminalVerifyError::TerminalLocalNpoValidityProofUnexpected);
            }
            (expected_rows, None) => {
                return Err(
                    NativeTerminalVerifyError::TerminalLocalNpoValidityProofMissing {
                        expected_rows,
                    },
                );
            }
            (_, Some(npo_proof)) => {
                self.verify_terminal_npo_exhaustive_goldilocks(
                    verifying_key,
                    public_inputs,
                    &proof.prelude,
                    &proof.witness_commitment,
                    npo_proof,
                )?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn verify_terminal_local_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        proof: &TerminalLocalProof,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, &proof.prelude)?;
        self.verify_terminal_combined_validity_consistency_goldilocks(
            verifying_key,
            public_inputs,
            &proof.prelude,
            &proof.witness_commitment,
            &proof.combined_validity_commitment,
            &proof.combined_validity_fold_proof,
            &proof.combined_validity_consistency_proof,
        )?;

        Ok(())
    }

    pub fn derive_terminal_npo_query_plan<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        prelude: &TerminalProofPrelude,
    ) -> Result<TerminalNpoQueryPlan, NativeTerminalVerifyError> {
        let domain_len = Self::terminal_npo_domain_len(verifying_key);
        if domain_len == 0 {
            return Err(NativeTerminalVerifyError::TerminalNpoQueryDomainEmpty);
        }

        let num_queries = prelude.parameters.num_queries as usize;
        let mut indices = Vec::with_capacity(num_queries);
        let mut counter = 0u64;
        while indices.len() < num_queries {
            if counter
                > (num_queries as u64)
                    .saturating_mul(4096)
                    .saturating_add(4096)
            {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_npo_query_block(prelude, domain_len, counter);
            for limb in block {
                if indices.len() == num_queries {
                    break;
                }
                let bound = domain_len as u64;
                let zone = u64::MAX - (u64::MAX % bound);
                if limb < zone {
                    Self::accept_terminal_query_index(
                        &mut indices,
                        (limb % bound) as usize,
                        domain_len,
                        num_queries,
                    );
                }
            }
            counter += 1;
        }

        Ok(TerminalNpoQueryPlan {
            domain_len,
            indices,
        })
    }

    pub fn prove_terminal_npo_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        prelude: &TerminalProofPrelude,
        witness_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
    ) -> Result<TerminalNpoProof, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let plan = self.derive_terminal_npo_query_plan(verifying_key, prelude)?;
        let mut openings = Vec::with_capacity(plan.indices.len());
        for npo_index in &plan.indices {
            openings.push(self.prove_terminal_npo_opening_goldilocks(
                verifying_key,
                witness_oracle,
                witness,
                *npo_index,
            )?);
        }
        Ok(TerminalNpoProof { openings })
    }

    pub fn verify_terminal_npo_queries_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        prelude: &TerminalProofPrelude,
        witness_commitment: &TerminalOracleCommitment,
        proof: &TerminalNpoProof,
    ) -> Result<TerminalNpoQueryPlan, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_proof_prelude_goldilocks(verifying_key, public_inputs, prelude)?;
        Self::verify_terminal_witness_commitment_identity(verifying_key, witness_commitment)?;
        Self::verify_prelude_binds_commitment(prelude, witness_commitment)?;
        let plan = self.derive_terminal_npo_query_plan(verifying_key, prelude)?;
        if proof.openings.len() != plan.indices.len() {
            return Err(NativeTerminalVerifyError::TerminalNpoQueryLengthMismatch {
                expected: plan.indices.len(),
                got: proof.openings.len(),
            });
        }

        for (query, (opening, expected_index)) in
            proof.openings.iter().zip(&plan.indices).enumerate()
        {
            if opening.npo_index != *expected_index {
                return Err(NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                    query,
                    expected: *expected_index,
                    got: opening.npo_index,
                });
            }
            let row_ref = Self::terminal_npo_row(verifying_key, opening.npo_index).ok_or(
                NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                    query,
                    expected: *expected_index,
                    got: opening.npo_index,
                },
            )?;
            match row_ref {
                NativeTerminalNpoRowRef::Tip5 { row, callsite, .. } => {
                    self.verify_sampled_tip5_npo_row::<F>(
                        opening.npo_index,
                        row,
                        callsite,
                        witness_commitment,
                        opening,
                    )?;
                }
                NativeTerminalNpoRowRef::Recompose {
                    op_type,
                    row,
                    callsite,
                } => {
                    self.verify_sampled_recompose_npo_row::<F>(
                        opening.npo_index,
                        op_type,
                        row,
                        callsite,
                        witness_commitment,
                        opening,
                    )?;
                }
            }
        }

        Ok(plan)
    }

    pub fn verify_npo_relation_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        relation: &TerminalNpoRelation,
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let expected = verifying_key.npo_relation();
        if relation != &expected {
            return Err(NativeTerminalVerifyError::TerminalNpoRelationMismatch {
                expected_rows: expected.rows.len(),
                got_rows: relation.rows.len(),
            });
        }
        self.verify_assignment_with_goldilocks_npos(verifying_key, witness)
    }

    fn compile_primitive_constraints<F: Field>(
        &self,
        circuit: &Circuit<F>,
    ) -> Vec<NativeTerminalConstraint<F>> {
        let mut constraints = Vec::with_capacity(circuit.ops.len());
        let mut tip5_goldilocks_callsites = Vec::new();
        let mut recompose_callsites = Vec::new();
        let mut recompose_coeff_callsites = Vec::new();
        for op in &circuit.ops {
            match op {
                Op::Const { out, val } => {
                    constraints.push(NativeTerminalConstraint::Const {
                        out: *out,
                        val: *val,
                    });
                }
                Op::Public { out, public_pos } => {
                    constraints.push(NativeTerminalConstraint::Public {
                        out: *out,
                        public_pos: *public_pos,
                    });
                }
                Op::Alu {
                    kind,
                    a,
                    b,
                    c,
                    out,
                    intermediate_out,
                } => {
                    constraints.push(NativeTerminalConstraint::Alu {
                        kind: *kind,
                        a: *a,
                        b: *b,
                        c: *c,
                        out: *out,
                        intermediate_out: *intermediate_out,
                    });
                }
                Op::Hint { .. } => {}
                Op::NonPrimitiveOpWithExecutor {
                    inputs,
                    outputs,
                    executor,
                    ..
                } => {
                    if Self::is_supported_tip5_op(executor.op_type()) {
                        let mode = executor
                            .tip5_terminal_mode()
                            .expect("supported Tip5 terminal op must expose row mode");
                        tip5_goldilocks_callsites.push(Self::tip5_callsite(inputs, outputs, mode));
                    } else if executor.op_type().as_str() == NpoTypeId::recompose().as_str() {
                        recompose_callsites.push(Self::recompose_callsite(inputs, outputs));
                    } else if executor.op_type().as_str()
                        == NpoTypeId::recompose_with_coeff_lookups().as_str()
                    {
                        recompose_coeff_callsites.push(Self::recompose_callsite(inputs, outputs));
                    }
                }
            }
        }
        if !tip5_goldilocks_callsites.is_empty() {
            let expected_rows = tip5_goldilocks_callsites.len();
            constraints.push(NativeTerminalConstraint::Tip5Goldilocks {
                op_type: Self::tip5_op_type().as_str().into(),
                expected_rows,
                callsites: tip5_goldilocks_callsites,
            });
        }
        if !recompose_callsites.is_empty() {
            let expected_rows = recompose_callsites.len();
            constraints.push(NativeTerminalConstraint::RecomposeGoldilocks {
                op_type: NpoTypeId::recompose().as_str().into(),
                expected_rows,
                callsites: recompose_callsites,
            });
        }
        if !recompose_coeff_callsites.is_empty() {
            let expected_rows = recompose_coeff_callsites.len();
            constraints.push(NativeTerminalConstraint::RecomposeGoldilocks {
                op_type: NpoTypeId::recompose_with_coeff_lookups().as_str().into(),
                expected_rows,
                callsites: recompose_coeff_callsites,
            });
        }
        constraints
    }

    fn tip5_callsite(
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        mode: Tip5TerminalMode,
    ) -> NativeTerminalNpoCallsite {
        let mmcs_bit_slot = Tip5Config::GOLDILOCKS_W16.width_ext() + 1;
        let tip5_mmcs_bit = mode
            .merkle_path
            .then(|| {
                inputs
                    .get(mmcs_bit_slot)
                    .and_then(|slot| match slot.as_slice() {
                        [wid] => Some(*wid),
                        _ => None,
                    })
            })
            .flatten();
        NativeTerminalNpoCallsite {
            inputs: Self::single_witness_slots(
                inputs.iter().take(Tip5Config::GOLDILOCKS_W16.width()),
            ),
            outputs: Self::single_witness_slots(
                outputs.iter().take(Tip5Config::GOLDILOCKS_W16.rate()),
            ),
            tip5_mode: Some(mode),
            tip5_mmcs_bit,
        }
    }

    fn recompose_callsite(
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
    ) -> NativeTerminalNpoCallsite {
        NativeTerminalNpoCallsite {
            inputs: inputs
                .first()
                .map(|coeffs| coeffs.iter().copied().map(Some).collect())
                .unwrap_or_default(),
            outputs: Self::single_witness_slots(outputs.iter().take(1)),
            tip5_mode: None,
            tip5_mmcs_bit: None,
        }
    }

    fn single_witness_slots<'a>(
        slots: impl Iterator<Item = &'a Vec<WitnessId>>,
    ) -> Vec<Option<WitnessId>> {
        slots
            .map(|slot| match slot.as_slice() {
                [wid] => Some(*wid),
                _ => None,
            })
            .collect()
    }

    pub fn verify_primitive_assignment<F: Field>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError> {
        if verifying_key.header.fingerprint != witness.fingerprint {
            return Err(NativeTerminalVerifyError::FingerprintMismatch {
                expected: verifying_key.header.fingerprint,
                got: witness.fingerprint,
            });
        }
        if witness.public_inputs.len() != verifying_key.header.fingerprint.public_flat_len {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: verifying_key.header.fingerprint.public_flat_len,
                got: witness.public_inputs.len(),
            });
        }
        self.verify_witness_shape(&verifying_key.header.fingerprint, witness)?;

        let get = |id: WitnessId| {
            witness
                .traces
                .witness_trace
                .get_value(id)
                .copied()
                .ok_or(NativeTerminalVerifyError::MissingWitness { witness_id: id.0 })
        };

        for (constraint_index, constraint) in verifying_key.constraints.iter().enumerate() {
            match constraint {
                NativeTerminalConstraint::Const { out, val } => {
                    if get(*out)? != *val {
                        return Err(NativeTerminalVerifyError::ConstraintViolation {
                            constraint_index,
                            kind: "const",
                        });
                    }
                }
                NativeTerminalConstraint::Public { out, public_pos } => {
                    let Some(public_value) = witness.public_inputs.get(*public_pos).copied() else {
                        return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                            expected: verifying_key.header.fingerprint.public_flat_len,
                            got: witness.public_inputs.len(),
                        });
                    };
                    if get(*out)? != public_value {
                        return Err(NativeTerminalVerifyError::PublicInputMismatch {
                            public_pos: *public_pos,
                        });
                    }
                }
                NativeTerminalConstraint::Alu {
                    kind,
                    a,
                    b,
                    c,
                    out,
                    intermediate_out,
                } => {
                    let a_val = get(*a)?;
                    let b_val = get(*b)?;
                    let c_val = if let Some(c) = c { get(*c)? } else { F::ZERO };
                    let out_val = get(*out)?;
                    let ok = match kind {
                        AluOpKind::Add => a_val + b_val == out_val,
                        AluOpKind::Mul => a_val * b_val == out_val,
                        AluOpKind::BoolCheck => {
                            a_val * (a_val - F::ONE) == F::ZERO && out_val == a_val
                        }
                        AluOpKind::MulAdd => a_val * b_val + c_val == out_val,
                        AluOpKind::HornerAcc => {
                            let Some(acc) = intermediate_out else {
                                return Err(NativeTerminalVerifyError::ConstraintViolation {
                                    constraint_index,
                                    kind: "horner_acc_missing_acc",
                                });
                            };
                            get(*acc)? * b_val + c_val - a_val == out_val
                        }
                    };
                    if !ok {
                        return Err(NativeTerminalVerifyError::ConstraintViolation {
                            constraint_index,
                            kind: "alu",
                        });
                    }
                }
                NativeTerminalConstraint::Tip5Goldilocks {
                    op_type,
                    expected_rows: _,
                    callsites: _,
                } => {
                    let _ = op_type;
                    return Err(NativeTerminalVerifyError::ConstraintViolation {
                        constraint_index,
                        kind: "tip5_requires_goldilocks_verifier",
                    });
                }
                NativeTerminalConstraint::RecomposeGoldilocks {
                    op_type,
                    expected_rows: _,
                    callsites: _,
                } => {
                    let _ = op_type;
                    return Err(NativeTerminalVerifyError::ConstraintViolation {
                        constraint_index,
                        kind: "recompose_requires_goldilocks_verifier",
                    });
                }
            }
        }

        Ok(())
    }

    pub fn verify_assignment_with_goldilocks_npos<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        self.verify_assignment_with_tip5_goldilocks(verifying_key, witness)
    }

    pub fn verify_assignment_with_tip5_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        if verifying_key.header.fingerprint != witness.fingerprint {
            return Err(NativeTerminalVerifyError::FingerprintMismatch {
                expected: verifying_key.header.fingerprint,
                got: witness.fingerprint,
            });
        }
        if witness.public_inputs.len() != verifying_key.header.fingerprint.public_flat_len {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: verifying_key.header.fingerprint.public_flat_len,
                got: witness.public_inputs.len(),
            });
        }
        self.verify_witness_shape(&verifying_key.header.fingerprint, witness)?;
        self.verify_relation_digest_goldilocks(verifying_key)?;
        self.verify_no_unexpected_nonprimitive_traces(verifying_key, witness)?;

        let get = |id: WitnessId| {
            witness
                .traces
                .witness_trace
                .get_value(id)
                .copied()
                .ok_or(NativeTerminalVerifyError::MissingWitness { witness_id: id.0 })
        };

        for (constraint_index, constraint) in verifying_key.constraints.iter().enumerate() {
            match constraint {
                NativeTerminalConstraint::Const { out, val } => {
                    if get(*out)? != *val {
                        return Err(NativeTerminalVerifyError::ConstraintViolation {
                            constraint_index,
                            kind: "const",
                        });
                    }
                }
                NativeTerminalConstraint::Public { out, public_pos } => {
                    let Some(public_value) = witness.public_inputs.get(*public_pos).copied() else {
                        return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                            expected: verifying_key.header.fingerprint.public_flat_len,
                            got: witness.public_inputs.len(),
                        });
                    };
                    if get(*out)? != public_value {
                        return Err(NativeTerminalVerifyError::PublicInputMismatch {
                            public_pos: *public_pos,
                        });
                    }
                }
                NativeTerminalConstraint::Alu {
                    kind,
                    a,
                    b,
                    c,
                    out,
                    intermediate_out,
                } => {
                    let a_val = get(*a)?;
                    let b_val = get(*b)?;
                    let c_val = if let Some(c) = c { get(*c)? } else { F::ZERO };
                    let out_val = get(*out)?;
                    let ok = match kind {
                        AluOpKind::Add => a_val + b_val == out_val,
                        AluOpKind::Mul => a_val * b_val == out_val,
                        AluOpKind::BoolCheck => {
                            a_val * (a_val - F::ONE) == F::ZERO && out_val == a_val
                        }
                        AluOpKind::MulAdd => a_val * b_val + c_val == out_val,
                        AluOpKind::HornerAcc => {
                            let Some(acc) = intermediate_out else {
                                return Err(NativeTerminalVerifyError::ConstraintViolation {
                                    constraint_index,
                                    kind: "horner_acc_missing_acc",
                                });
                            };
                            get(*acc)? * b_val + c_val - a_val == out_val
                        }
                    };
                    if !ok {
                        return Err(NativeTerminalVerifyError::ConstraintViolation {
                            constraint_index,
                            kind: "alu",
                        });
                    }
                }
                NativeTerminalConstraint::Tip5Goldilocks {
                    op_type,
                    expected_rows,
                    callsites,
                } => {
                    self.verify_tip5_goldilocks_assignment(
                        op_type,
                        *expected_rows,
                        callsites,
                        witness,
                    )?;
                }
                NativeTerminalConstraint::RecomposeGoldilocks {
                    op_type,
                    expected_rows,
                    callsites,
                } => {
                    self.verify_recompose_goldilocks_assignment(
                        op_type,
                        *expected_rows,
                        callsites,
                        witness,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn verify_witness_shape<F>(
        &self,
        fingerprint: &TerminalCircuitFingerprint,
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError> {
        if witness.private_inputs.len() != fingerprint.private_flat_len {
            return Err(NativeTerminalVerifyError::PrivateInputLengthMismatch {
                expected: fingerprint.private_flat_len,
                got: witness.private_inputs.len(),
            });
        }
        if witness.traces.witness_trace.num_rows() != fingerprint.witness_count as usize {
            return Err(NativeTerminalVerifyError::WitnessTraceLengthMismatch {
                expected: fingerprint.witness_count as usize,
                got: witness.traces.witness_trace.num_rows(),
            });
        }
        if witness.traces.witness_trace.index.len() != fingerprint.witness_count as usize {
            return Err(NativeTerminalVerifyError::WitnessTraceLengthMismatch {
                expected: fingerprint.witness_count as usize,
                got: witness.traces.witness_trace.index.len(),
            });
        }
        for (row, witness_id) in witness
            .traces
            .witness_trace
            .index
            .iter()
            .copied()
            .enumerate()
        {
            let expected = row as u32;
            if witness_id.0 != expected {
                return Err(NativeTerminalVerifyError::WitnessTraceIndexMismatch {
                    row,
                    expected,
                    got: witness_id.0,
                });
            }
        }
        Ok(())
    }

    fn terminal_witness_values<F: Copy>(
        witness: &TerminalWitness<F>,
    ) -> Result<Vec<F>, NativeTerminalVerifyError> {
        let mut values = Vec::with_capacity(witness.traces.witness_trace.num_rows());
        for idx in 0..witness.traces.witness_trace.num_rows() {
            let witness_id = WitnessId(idx as u32);
            values.push(
                witness
                    .traces
                    .witness_trace
                    .get_value(witness_id)
                    .copied()
                    .ok_or(NativeTerminalVerifyError::MissingWitness {
                        witness_id: witness_id.0,
                    })?,
            );
        }
        Ok(values)
    }

    fn terminal_assignment_values<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        public_inputs: &[F],
        witness_values: &[F],
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: Field + Copy,
    {
        let public_count = verifying_key.header.fingerprint.public_flat_len;
        let witness_count = verifying_key.header.fingerprint.witness_count as usize;
        if public_inputs.len() != public_count {
            return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                expected: public_count,
                got: public_inputs.len(),
            });
        }
        if witness_values.len() != witness_count {
            return Err(
                NativeTerminalVerifyError::TerminalProductionWitnessBasisLengthMismatch {
                    expected: witness_count,
                    got: witness_values.len(),
                },
            );
        }

        let mut values = Vec::with_capacity(1 + public_count + witness_count);
        values.push(F::ONE);
        values.extend_from_slice(public_inputs);
        values.extend_from_slice(witness_values);
        Ok(values)
    }

    fn sparse_r1cs_matrix_evaluations_at_row<F>(
        relation: &TerminalSparseR1csRelation<F>,
        row_point: &[F],
        assignment_values: &[F],
    ) -> Result<(F, F, F), NativeTerminalVerifyError>
    where
        F: Field + Copy,
    {
        let mut a = F::ZERO;
        let mut b = F::ZERO;
        let mut c = F::ZERO;
        for entry in &relation.entries {
            let assignment = assignment_values.get(entry.variable_index).copied().ok_or(
                NativeTerminalVerifyError::TerminalConstraintOpeningWitnessMismatch {
                    constraint_index: entry.row,
                    opening: entry.variable_index,
                    expected: relation.variables as u32,
                    got: assignment_values.len(),
                },
            )?;
            let term =
                entry.coeff * Self::terminal_eq_index_point(entry.row, row_point) * assignment;
            match entry.matrix {
                TerminalSparseR1csMatrix::A => a += term,
                TerminalSparseR1csMatrix::B => b += term,
                TerminalSparseR1csMatrix::C => c += term,
            }
        }
        Ok((a, b, c))
    }

    fn sparse_r1cs_matrix_combo_values<F>(
        relation: &TerminalSparseR1csRelation<F>,
        row_point: &[F],
        alpha: F,
    ) -> Vec<F>
    where
        F: Field + Copy,
    {
        let alpha_sq = alpha.square();
        let mut values = vec![F::ZERO; 1usize << relation.log_variables];
        for entry in &relation.entries {
            let matrix_weight = match entry.matrix {
                TerminalSparseR1csMatrix::A => F::ONE,
                TerminalSparseR1csMatrix::B => alpha,
                TerminalSparseR1csMatrix::C => alpha_sq,
            };
            values[entry.variable_index] +=
                entry.coeff * matrix_weight * Self::terminal_eq_index_point(entry.row, row_point);
        }
        values
    }

    fn sparse_r1cs_folded_round_evaluations<F>(
        matrix_values: &[F],
        assignment_values: &[F],
    ) -> [F; 3]
    where
        F: Field + Copy,
    {
        let mut evals = [F::ZERO; 3];
        for pair_index in 0..matrix_values.len().div_ceil(2) {
            let left_index = pair_index * 2;
            let right_index = left_index + 1;
            let matrix_left = matrix_values[left_index];
            let matrix_right = matrix_values.get(right_index).copied().unwrap_or(F::ZERO);
            let assignment_left = assignment_values[left_index];
            let assignment_right = assignment_values
                .get(right_index)
                .copied()
                .unwrap_or(F::ZERO);

            evals[0] += matrix_left * assignment_left;
            evals[1] += matrix_right * assignment_right;
            evals[2] += (matrix_right + matrix_right - matrix_left)
                * (assignment_right + assignment_right - assignment_left);
        }
        evals
    }

    fn sparse_r1cs_row_product_initial_vectors<F>(
        relation: &TerminalSparseR1csRelation<F>,
        anchor_point: &[F],
        assignment_values: &[F],
    ) -> Result<(Vec<F>, Vec<F>, Vec<F>, Vec<F>), NativeTerminalVerifyError>
    where
        F: Field + Copy,
    {
        let row_domain_len = 1usize << relation.log_rows;
        let eq_values = (0..row_domain_len)
            .map(|row| Self::terminal_eq_index_point(row, anchor_point))
            .collect::<Vec<_>>();
        let mut a_values = vec![F::ZERO; row_domain_len];
        let mut b_values = vec![F::ZERO; row_domain_len];
        let mut c_values = vec![F::ZERO; row_domain_len];

        for entry in &relation.entries {
            let assignment = assignment_values.get(entry.variable_index).copied().ok_or(
                NativeTerminalVerifyError::TerminalConstraintOpeningWitnessMismatch {
                    constraint_index: entry.row,
                    opening: entry.variable_index,
                    expected: relation.variables as u32,
                    got: assignment_values.len(),
                },
            )?;
            let term = entry.coeff * assignment;
            match entry.matrix {
                TerminalSparseR1csMatrix::A => a_values[entry.row] += term,
                TerminalSparseR1csMatrix::B => b_values[entry.row] += term,
                TerminalSparseR1csMatrix::C => c_values[entry.row] += term,
            }
        }

        Ok((eq_values, a_values, b_values, c_values))
    }

    fn row_product_claim<F>(eq_values: &[F], a_values: &[F], b_values: &[F], c_values: &[F]) -> F
    where
        F: Field + Copy,
    {
        eq_values
            .iter()
            .copied()
            .zip(a_values.iter().copied())
            .zip(b_values.iter().copied())
            .zip(c_values.iter().copied())
            .fold(F::ZERO, |acc, (((eq, a), b), c)| acc + eq * (a * b - c))
    }

    fn sparse_r1cs_row_product_round_evaluations<F>(
        eq_values: &[F],
        a_values: &[F],
        b_values: &[F],
        c_values: &[F],
    ) -> [F; 4]
    where
        F: Field + Copy,
    {
        let two = F::ONE + F::ONE;
        let three = two + F::ONE;
        let points = [F::ZERO, F::ONE, two, three];
        let mut evals = [F::ZERO; 4];

        for pair_index in 0..eq_values.len().div_ceil(2) {
            let left_index = pair_index * 2;
            let right_index = left_index + 1;
            let eq_left = eq_values[left_index];
            let eq_right = eq_values.get(right_index).copied().unwrap_or(F::ZERO);
            let a_left = a_values[left_index];
            let a_right = a_values.get(right_index).copied().unwrap_or(F::ZERO);
            let b_left = b_values[left_index];
            let b_right = b_values.get(right_index).copied().unwrap_or(F::ZERO);
            let c_left = c_values[left_index];
            let c_right = c_values.get(right_index).copied().unwrap_or(F::ZERO);

            for (eval, point) in evals.iter_mut().zip(points) {
                let one_minus_point = F::ONE - point;
                let eq = eq_left * one_minus_point + eq_right * point;
                let a = a_left * one_minus_point + a_right * point;
                let b = b_left * one_minus_point + b_right * point;
                let c = c_left * one_minus_point + c_right * point;
                *eval += eq * (a * b - c);
            }
        }

        evals
    }

    fn evaluate_sparse_r1cs_matrix_combo_at_point<F>(
        relation: &TerminalSparseR1csRelation<F>,
        row_point: &[F],
        variable_point: &[F],
        alpha: F,
    ) -> F
    where
        F: Field + Copy,
    {
        let alpha_sq = alpha.square();
        let mut acc = F::ZERO;
        for entry in &relation.entries {
            let matrix_weight = match entry.matrix {
                TerminalSparseR1csMatrix::A => F::ONE,
                TerminalSparseR1csMatrix::B => alpha,
                TerminalSparseR1csMatrix::C => alpha_sq,
            };
            acc += entry.coeff
                * matrix_weight
                * Self::terminal_eq_index_point(entry.row, row_point)
                * Self::terminal_eq_index_point(entry.variable_index, variable_point);
        }
        acc
    }

    fn terminal_eq_index_point_pair<F>(left_point: &[F], right_point: &[F]) -> F
    where
        F: Field + Copy,
    {
        left_point
            .iter()
            .copied()
            .zip(right_point.iter().copied())
            .fold(F::ONE, |acc, (left, right)| {
                acc * ((F::ONE - left) * (F::ONE - right) + left * right)
            })
    }

    fn pad_terminal_values_to_mle_len<F>(values: &[F], log_len: usize) -> Vec<F>
    where
        F: Field + Copy,
    {
        let mut out = values.to_vec();
        out.resize(1usize << log_len, F::ZERO);
        out
    }

    fn fold_terminal_values<F>(values: &[F], challenge: F) -> Vec<F>
    where
        F: Field + Copy,
    {
        let mut out = Vec::with_capacity(values.len().div_ceil(2));
        for pair in values.chunks(2) {
            let left = pair[0];
            let right = pair.get(1).copied().unwrap_or(F::ZERO);
            out.push(left * (F::ONE - challenge) + right * challenge);
        }
        out
    }

    fn terminal_eq_index_point<F>(index: usize, point: &[F]) -> F
    where
        F: Field + Copy,
    {
        Self::terminal_eq_index_point_prefix(index, point)
    }

    fn terminal_eq_index_point_prefix<F>(index: usize, point: &[F]) -> F
    where
        F: Field + Copy,
    {
        let mut acc = F::ONE;
        for (bit_index, challenge) in point.iter().copied().enumerate() {
            if ((index >> bit_index) & 1) == 1 {
                acc *= challenge;
            } else {
                acc *= F::ONE - challenge;
            }
        }
        acc
    }

    fn interpolate_degree_two_at<F>(evals: [F; 3], point: F) -> F
    where
        F: Field + Copy,
    {
        let two_inv = (F::ONE + F::ONE).inverse();
        let second = (evals[2] - evals[1] - evals[1] + evals[0]) * two_inv;
        evals[0] + (evals[1] - evals[0]) * point + second * point * (point - F::ONE)
    }

    fn interpolate_degree_three_at<F>(evals: [F; 4], point: F) -> F
    where
        F: Field + Copy,
    {
        let two = F::ONE + F::ONE;
        let three = two + F::ONE;
        let points = [F::ZERO, F::ONE, two, three];
        let mut acc = F::ZERO;
        for i in 0..4 {
            let mut numerator = F::ONE;
            let mut denominator = F::ONE;
            for j in 0..4 {
                if i == j {
                    continue;
                }
                numerator *= point - points[j];
                denominator *= points[i] - points[j];
            }
            acc += evals[i] * numerator * denominator.inverse();
        }
        acc
    }

    fn flatten_goldilocks_basis_values<F>(values: &[F]) -> Vec<u64>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let mut out =
            Vec::with_capacity(values.len() * <F as BasedVectorSpace<Goldilocks>>::DIMENSION);
        for value in values {
            out.extend(
                value
                    .as_basis_coefficients_slice()
                    .iter()
                    .map(PrimeField64::as_canonical_u64),
            );
        }
        out
    }

    fn verify_no_unexpected_nonprimitive_traces<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError> {
        for op_type in witness.traces.non_primitive_traces.keys() {
            let op_type_str = op_type.as_str();
            if !verifying_key
                .inventory
                .non_primitive_types
                .iter()
                .any(|expected| expected == op_type_str)
            {
                return Err(NativeTerminalVerifyError::UnexpectedNonPrimitiveTrace {
                    op_type: op_type_str.into(),
                });
            }
        }
        Ok(())
    }

    fn tip5_op_type() -> NpoTypeId {
        NpoTypeId::tip5_perm(Tip5Config::GOLDILOCKS_W16)
    }

    fn primitive_constraint_witness_ids<F>(
        constraint: &NativeTerminalConstraint<F>,
    ) -> Vec<WitnessId> {
        let mut out = Vec::new();
        match constraint {
            NativeTerminalConstraint::Const {
                out: witness_id, ..
            }
            | NativeTerminalConstraint::Public {
                out: witness_id, ..
            } => Self::push_unique_witness(&mut out, *witness_id),
            NativeTerminalConstraint::Alu {
                a,
                b,
                c,
                out: output,
                intermediate_out,
                ..
            } => {
                Self::push_unique_witness(&mut out, *a);
                Self::push_unique_witness(&mut out, *b);
                if let Some(c) = c {
                    Self::push_unique_witness(&mut out, *c);
                }
                Self::push_unique_witness(&mut out, *output);
                if let Some(intermediate_out) = intermediate_out {
                    Self::push_unique_witness(&mut out, *intermediate_out);
                }
            }
            NativeTerminalConstraint::Tip5Goldilocks { .. }
            | NativeTerminalConstraint::RecomposeGoldilocks { .. } => {}
        }
        out
    }

    fn quadratic_constraint_witness_ids<F>(
        constraint: &TerminalQuadraticConstraint<F>,
    ) -> Vec<WitnessId> {
        let mut out = Vec::new();
        for expression in [&constraint.a, &constraint.b, &constraint.c] {
            for term in &expression.terms {
                if let TerminalLinearVariable::Witness(witness_id) = term.variable {
                    Self::push_unique_witness(&mut out, witness_id);
                }
            }
        }
        out
    }

    fn evaluate_opened_linear_expression<F: Field>(
        expression: &TerminalLinearExpression<F>,
        public_inputs: &[F],
        witness_ids: &[WitnessId],
        witness_values: &[F],
    ) -> Result<F, NativeTerminalVerifyError> {
        let mut acc = F::ZERO;
        for term in &expression.terms {
            let value = match term.variable {
                TerminalLinearVariable::One => F::ONE,
                TerminalLinearVariable::Public(public_pos) => public_inputs
                    .get(public_pos)
                    .copied()
                    .ok_or(NativeTerminalVerifyError::PublicInputLengthMismatch {
                        expected: public_pos + 1,
                        got: public_inputs.len(),
                    })?,
                TerminalLinearVariable::Witness(witness_id) => witness_ids
                    .iter()
                    .position(|id| *id == witness_id)
                    .and_then(|idx| witness_values.get(idx).copied())
                    .ok_or(NativeTerminalVerifyError::MissingWitness {
                        witness_id: witness_id.0,
                    })?,
            };
            acc += term.coeff * value;
        }
        Ok(acc)
    }

    fn terminal_npo_domain_len<F>(verifying_key: &NativeTerminalVerifyingKey<F>) -> usize {
        verifying_key
            .constraints
            .iter()
            .map(|constraint| match constraint {
                NativeTerminalConstraint::Tip5Goldilocks { expected_rows, .. }
                | NativeTerminalConstraint::RecomposeGoldilocks { expected_rows, .. } => {
                    *expected_rows
                }
                _ => 0,
            })
            .sum()
    }

    fn terminal_npo_polynomial_profile<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
    ) -> TerminalNpoPolynomialProfile
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        Self::terminal_npo_polynomial_profile_for_basis_dimension(
            verifying_key,
            <F as BasedVectorSpace<Goldilocks>>::DIMENSION,
        )
    }

    fn terminal_npo_polynomial_profile_for_basis_dimension<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        basis_dimension: usize,
    ) -> TerminalNpoPolynomialProfile {
        let relation = verifying_key.npo_relation();
        let mut profile = TerminalNpoPolynomialProfile {
            rows: relation.rows.len(),
            log_rows: Self::terminal_mle_log_size(relation.rows.len()),
            sampled_residual_components: Self::terminal_npo_validity_domain_len_for_basis_dimension(
                verifying_key,
                basis_dimension,
            ),
            residual_components:
                Self::terminal_npo_exhaustive_residual_domain_len_for_basis_dimension(
                    verifying_key,
                    basis_dimension,
                ),
            tip5_rounds: TIP5_PERM_ROUNDS,
            ..TerminalNpoPolynomialProfile::default()
        };
        profile.log_residual_components = Self::terminal_mle_log_size(profile.residual_components);

        for row in &relation.rows {
            profile.witness_input_slots += row.callsite.inputs.iter().flatten().count();
            profile.witness_output_slots += row.callsite.outputs.iter().flatten().count();
            match row.kind {
                TerminalNpoRowKind::Tip5Goldilocks => {
                    profile.tip5_rows += 1;
                    profile.max_constraint_degree = profile.max_constraint_degree.max(4);
                    profile.hidden_input_slots += row
                        .callsite
                        .inputs
                        .iter()
                        .filter(|input| input.is_none())
                        .count();
                    if row.callsite.tip5_mmcs_bit.is_some() {
                        profile.mmcs_direction_bits += 1;
                    }
                    if let Some(mode) = row.callsite.tip5_mode {
                        if mode.merkle_path {
                            profile.tip5_merkle_rows += 1;
                        }
                        if mode.new_start {
                            profile.tip5_new_start_rows += 1;
                        }
                        profile.max_serialized_hidden_input_slots +=
                            Self::max_serialized_tip5_hidden_input_slots(&row.callsite, mode);
                    }
                }
                TerminalNpoRowKind::Recompose => {
                    profile.recompose_rows += 1;
                    profile.max_constraint_degree = profile.max_constraint_degree.max(2);
                }
                TerminalNpoRowKind::RecomposeWithCoeffLookups => {
                    profile.recompose_coeff_rows += 1;
                    profile.max_constraint_degree = profile.max_constraint_degree.max(2);
                }
            }
        }

        profile
    }

    fn max_serialized_tip5_hidden_input_slots(
        callsite: &NativeTerminalNpoCallsite,
        mode: Tip5TerminalMode,
    ) -> usize {
        if !mode.merkle_path {
            return 0;
        }
        let has_ctl_output = callsite.outputs.iter().any(Option::is_some);
        let mut count_without_swap = 0usize;
        let mut count_with_swap = 0usize;
        for limb in 0..16 {
            if callsite.inputs[limb].is_some() {
                continue;
            }
            if Self::should_serialize_tip5_hidden_limb(callsite, mode, has_ctl_output, false, limb)
            {
                count_without_swap += 1;
            }
            if Self::should_serialize_tip5_hidden_limb(callsite, mode, has_ctl_output, true, limb) {
                count_with_swap += 1;
            }
        }
        count_without_swap.max(count_with_swap)
    }

    fn terminal_npo_exhaustive_residual_domain_len_for_basis_dimension<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        basis_dimension: usize,
    ) -> usize {
        (0..Self::terminal_npo_domain_len(verifying_key))
            .map(|npo_index| {
                Self::terminal_npo_row(verifying_key, npo_index)
                    .map(|row| {
                        Self::terminal_npo_exhaustive_residual_component_count_for_basis_dimension(
                            &row,
                            basis_dimension,
                        )
                    })
                    .unwrap_or(0)
            })
            .sum()
    }

    fn terminal_npo_exhaustive_residual_component_count_for_basis_dimension(
        row: &NativeTerminalNpoRowRef<'_>,
        basis_dimension: usize,
    ) -> usize {
        let sampled_components =
            Self::terminal_npo_validity_component_count_for_basis_dimension(row, basis_dimension);
        match row {
            NativeTerminalNpoRowRef::Tip5 { callsite, .. } => {
                let Some(mode) = callsite.tip5_mode else {
                    return sampled_components;
                };
                let mut exhaustive_components = sampled_components;
                if mode.merkle_path && callsite.tip5_mmcs_bit.is_some() {
                    exhaustive_components += 1;
                }
                exhaustive_components
                    + Self::terminal_tip5_chain_residual_component_count(callsite, mode)
            }
            NativeTerminalNpoRowRef::Recompose { .. } => sampled_components,
        }
    }

    fn terminal_tip5_chain_residual_component_count(
        callsite: &NativeTerminalNpoCallsite,
        mode: Tip5TerminalMode,
    ) -> usize {
        if mode.merkle_path {
            let chained_rate_lanes = (0..5)
                .filter(|limb| callsite.inputs[*limb].is_none())
                .count();
            let zero_capacity_lanes = (10..16)
                .filter(|limb| callsite.inputs[*limb].is_none())
                .count();
            chained_rate_lanes + zero_capacity_lanes
        } else {
            callsite
                .inputs
                .iter()
                .filter(|input| input.is_none())
                .count()
        }
    }

    fn terminal_npo_validity_domain_len<F>(verifying_key: &NativeTerminalVerifyingKey<F>) -> usize
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        Self::terminal_npo_validity_domain_len_for_basis_dimension(
            verifying_key,
            <F as BasedVectorSpace<Goldilocks>>::DIMENSION,
        )
    }

    fn terminal_npo_validity_domain_len_for_basis_dimension<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        basis_dimension: usize,
    ) -> usize {
        (0..Self::terminal_npo_domain_len(verifying_key))
            .map(|npo_index| {
                Self::terminal_npo_row(verifying_key, npo_index)
                    .map(|row| {
                        Self::terminal_npo_validity_component_count_for_basis_dimension(
                            &row,
                            basis_dimension,
                        )
                    })
                    .unwrap_or(0)
            })
            .sum()
    }

    fn terminal_npo_validity_component_count<F>(row: &NativeTerminalNpoRowRef<'_>) -> usize
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        Self::terminal_npo_validity_component_count_for_basis_dimension(
            row,
            <F as BasedVectorSpace<Goldilocks>>::DIMENSION,
        )
    }

    fn terminal_npo_validity_component_count_for_basis_dimension(
        row: &NativeTerminalNpoRowRef<'_>,
        basis_dimension: usize,
    ) -> usize {
        match row {
            NativeTerminalNpoRowRef::Tip5 { callsite, .. } => {
                let input_components = callsite
                    .inputs
                    .iter()
                    .filter(|input| input.is_some())
                    .count();
                let output_components = callsite
                    .outputs
                    .iter()
                    .filter(|output| output.is_some())
                    .count();
                (input_components + output_components) * basis_dimension
            }
            NativeTerminalNpoRowRef::Recompose { callsite, .. } => {
                let input_components = callsite.inputs.len() * basis_dimension;
                let output_components = if callsite.outputs.first().copied().flatten().is_some() {
                    basis_dimension
                } else {
                    0
                };
                input_components + output_components
            }
        }
    }

    fn terminal_npo_validity_component_row<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        validity_index: usize,
    ) -> Option<(usize, usize)>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let mut base = 0usize;
        for npo_index in 0..Self::terminal_npo_domain_len(verifying_key) {
            let row = Self::terminal_npo_row(verifying_key, npo_index)?;
            let count = Self::terminal_npo_validity_component_count::<F>(&row);
            if validity_index < base + count {
                return Some((npo_index, validity_index - base));
            }
            base += count;
        }
        None
    }

    fn terminal_npo_row<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        npo_index: usize,
    ) -> Option<NativeTerminalNpoRowRef<'_>> {
        let mut base = 0usize;
        for constraint in &verifying_key.constraints {
            match constraint {
                NativeTerminalConstraint::Tip5Goldilocks {
                    op_type,
                    expected_rows,
                    callsites,
                } => {
                    let end = base + *expected_rows;
                    if npo_index < end {
                        let row = npo_index - base;
                        return Some(NativeTerminalNpoRowRef::Tip5 {
                            op_type,
                            row,
                            callsite: &callsites[row],
                        });
                    }
                    base = end;
                }
                NativeTerminalConstraint::RecomposeGoldilocks {
                    op_type,
                    expected_rows,
                    callsites,
                } => {
                    let end = base + *expected_rows;
                    if npo_index < end {
                        let row = npo_index - base;
                        return Some(NativeTerminalNpoRowRef::Recompose {
                            op_type,
                            row,
                            callsite: &callsites[row],
                        });
                    }
                    base = end;
                }
                _ => {}
            }
        }
        None
    }

    fn npo_callsite_witness_ids(callsite: &NativeTerminalNpoCallsite) -> Vec<WitnessId> {
        let mut out = Vec::new();
        for witness_id in callsite.inputs.iter().chain(&callsite.outputs).flatten() {
            Self::push_unique_witness(&mut out, *witness_id);
        }
        if let Some(witness_id) = callsite.tip5_mmcs_bit {
            Self::push_unique_witness(&mut out, witness_id);
        }
        out
    }

    fn push_unique_witness(out: &mut Vec<WitnessId>, witness_id: WitnessId) {
        if !out.iter().any(|existing| *existing == witness_id) {
            out.push(witness_id);
        }
    }

    fn push_unique_usize(out: &mut Vec<usize>, value: usize) {
        if !out.iter().any(|existing| *existing == value) {
            out.push(value);
        }
    }

    fn verify_sampled_primitive_constraint<F: Field>(
        constraint_index: usize,
        constraint: &NativeTerminalConstraint<F>,
        public_inputs: &[F],
        witness_ids: &[WitnessId],
        values: &[F],
    ) -> Result<(), NativeTerminalVerifyError> {
        let value = |wanted: WitnessId| {
            witness_ids
                .iter()
                .position(|id| *id == wanted)
                .map(|idx| values[idx])
                .ok_or(NativeTerminalVerifyError::MissingWitness {
                    witness_id: wanted.0,
                })
        };

        match constraint {
            NativeTerminalConstraint::Const { out, val } => {
                if value(*out)? != *val {
                    return Err(NativeTerminalVerifyError::ConstraintViolation {
                        constraint_index,
                        kind: "sampled_const",
                    });
                }
            }
            NativeTerminalConstraint::Public { out, public_pos } => {
                let Some(public_value) = public_inputs.get(*public_pos).copied() else {
                    return Err(NativeTerminalVerifyError::PublicInputLengthMismatch {
                        expected: *public_pos + 1,
                        got: public_inputs.len(),
                    });
                };
                if value(*out)? != public_value {
                    return Err(NativeTerminalVerifyError::PublicInputMismatch {
                        public_pos: *public_pos,
                    });
                }
            }
            NativeTerminalConstraint::Alu {
                kind,
                a,
                b,
                c,
                out,
                intermediate_out,
            } => {
                let a_val = value(*a)?;
                let b_val = value(*b)?;
                let c_val = if let Some(c) = c { value(*c)? } else { F::ZERO };
                let out_val = value(*out)?;
                let ok = match kind {
                    AluOpKind::Add => a_val + b_val == out_val,
                    AluOpKind::Mul => a_val * b_val == out_val,
                    AluOpKind::BoolCheck => a_val * (a_val - F::ONE) == F::ZERO && out_val == a_val,
                    AluOpKind::MulAdd => a_val * b_val + c_val == out_val,
                    AluOpKind::HornerAcc => {
                        let Some(acc) = intermediate_out else {
                            return Err(NativeTerminalVerifyError::ConstraintViolation {
                                constraint_index,
                                kind: "sampled_horner_acc_missing_acc",
                            });
                        };
                        value(*acc)? * b_val + c_val - a_val == out_val
                    }
                };
                if !ok {
                    return Err(NativeTerminalVerifyError::ConstraintViolation {
                        constraint_index,
                        kind: "sampled_alu",
                    });
                }
            }
            NativeTerminalConstraint::Tip5Goldilocks { .. }
            | NativeTerminalConstraint::RecomposeGoldilocks { .. } => {
                return Err(NativeTerminalVerifyError::ConstraintViolation {
                    constraint_index,
                    kind: "sampled_nonprimitive_not_in_primitive_domain",
                });
            }
        }
        Ok(())
    }

    fn prove_terminal_npo_opening_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness_oracle: &TerminalOracleMerkleTree,
        witness: &TerminalWitness<F>,
        npo_index: usize,
    ) -> Result<TerminalNpoOpening, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let (local_opening, expected_witness_ids) =
            self.terminal_npo_local_opening_goldilocks(verifying_key, witness, npo_index)?;
        let mut witness_openings = Vec::new();
        for witness_id in expected_witness_ids {
            let value = witness.traces.witness_trace.get_value(witness_id).ok_or(
                NativeTerminalVerifyError::MissingWitness {
                    witness_id: witness_id.0,
                },
            )?;
            witness_openings
                .push(witness_oracle.open_goldilocks_value(witness_id.0 as usize, value)?);
        }

        Ok(TerminalNpoOpening {
            npo_index,
            tip5_hidden_input_values: Self::terminal_tip5_hidden_inputs_from_compact(
                local_opening.npo_index,
                verifying_key,
                local_opening.tip5_hidden_input_nonzero_mask,
                &local_opening.tip5_hidden_input_values_le,
            )?,
            witness_openings,
        })
    }

    fn terminal_npo_local_opening_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
        npo_index: usize,
    ) -> Result<(TerminalNpoLocalOpening, Vec<WitnessId>), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let row_ref = Self::terminal_npo_row(verifying_key, npo_index).ok_or(
            NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                query: 0,
                expected: npo_index,
                got: npo_index,
            },
        )?;
        let mut tip5_hidden_input_nonzero_mask = 0u16;
        let mut tip5_hidden_input_values_le = Vec::new();
        let expected_witness_ids = match row_ref {
            NativeTerminalNpoRowRef::Tip5 {
                op_type,
                row,
                callsite,
            } => {
                let trace = witness
                    .traces
                    .non_primitive_traces
                    .get(&NpoTypeId::new(op_type))
                    .and_then(|trace| trace.as_any().downcast_ref::<Tip5Trace<Goldilocks>>())
                    .ok_or_else(|| NativeTerminalVerifyError::MissingNonPrimitiveTrace {
                        op_type: op_type.into(),
                    })?;
                let operation = trace.operations.get(row).ok_or_else(|| {
                    NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                        op_type: op_type.into(),
                        expected: row + 1,
                        got: trace.operations.len(),
                    }
                })?;
                Self::verify_tip5_callsite(op_type, row, callsite, operation)?;
                if operation.input_values.len() != 16 {
                    return Err(NativeTerminalVerifyError::Tip5TraceInputLength {
                        row,
                        got: operation.input_values.len(),
                    });
                }
                for (limb, input_value) in operation.input_values.iter().enumerate() {
                    if callsite.inputs.get(limb).and_then(|input| *input).is_none() {
                        let basis = Self::goldilocks_basis_u64(input_value);
                        if basis.len() != 1 {
                            return Err(
                                NativeTerminalVerifyError::TerminalNpoTip5InputValueDimensionMismatch {
                                    npo_index,
                                    limb,
                                    expected: 1,
                                    got: basis.len(),
                                },
                            );
                        }
                        if basis[0] != 0 {
                            tip5_hidden_input_nonzero_mask |= 1u16 << limb;
                            tip5_hidden_input_values_le.push(basis[0].to_le_bytes());
                        }
                    }
                }
                Self::npo_callsite_witness_ids(callsite)
            }
            NativeTerminalNpoRowRef::Recompose { callsite, .. } => {
                Self::npo_callsite_witness_ids(callsite)
            }
        };

        Ok((
            TerminalNpoLocalOpening {
                npo_index,
                tip5_hidden_input_nonzero_mask,
                tip5_hidden_input_values_le,
            },
            expected_witness_ids,
        ))
    }

    fn terminal_tip5_hidden_inputs_from_compact(
        npo_index: usize,
        verifying_key: &NativeTerminalVerifyingKey<impl Field>,
        nonzero_mask: u16,
        values_le: &[[u8; 8]],
    ) -> Result<Vec<TerminalTip5HiddenInputValue>, NativeTerminalVerifyError> {
        if values_le.len() != nonzero_mask.count_ones() as usize {
            return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                npo_index,
                expected: nonzero_mask.count_ones() as usize,
                got: values_le.len(),
            });
        }
        let row_ref = Self::terminal_npo_row(verifying_key, npo_index).ok_or(
            NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                query: npo_index,
                expected: npo_index,
                got: npo_index,
            },
        )?;
        let callsite = match row_ref {
            NativeTerminalNpoRowRef::Tip5 { callsite, .. } => callsite,
            NativeTerminalNpoRowRef::Recompose { .. } => {
                if nonzero_mask != 0 || !values_le.is_empty() {
                    return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                        npo_index,
                        expected: 0,
                        got: values_le.len(),
                    });
                }
                return Ok(Vec::new());
            }
        };
        let hidden_input_count = callsite
            .inputs
            .iter()
            .filter(|input| input.is_none())
            .count();
        let mut hidden_mask = 0u16;
        for (limb, input) in callsite.inputs.iter().enumerate() {
            if input.is_none() {
                hidden_mask |= 1u16 << limb;
            }
        }
        if (nonzero_mask & !hidden_mask) != 0 {
            return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                npo_index,
                expected: hidden_input_count,
                got: nonzero_mask.count_ones() as usize,
            });
        }
        let mut values = Vec::with_capacity(hidden_input_count);
        let mut value_index = 0usize;
        for limb in 0..16 {
            if callsite.inputs.get(limb).and_then(|input| *input).is_none() {
                let value = if (nonzero_mask & (1u16 << limb)) != 0 {
                    let value = u64::from_le_bytes(values_le[value_index]);
                    value_index += 1;
                    value
                } else {
                    0
                };
                values.push(TerminalTip5HiddenInputValue {
                    limb,
                    value_basis: vec![value],
                });
            }
        }
        Ok(values)
    }

    fn terminal_tip5_hidden_inputs_from_exhaustive_compact<F>(
        npo_index: usize,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        values_le: &[[u8; 8]],
        value_offset: &mut usize,
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
        previous_normal_output: &Option<[Goldilocks; 16]>,
        previous_merkle_output: &Option<[Goldilocks; 16]>,
    ) -> Result<Vec<TerminalTip5HiddenInputValue>, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks> + Copy,
    {
        let mode =
            callsite
                .tip5_mode
                .ok_or(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                    op_type: Self::tip5_op_type().as_str().into(),
                    row,
                    field: "tip5_mode",
                    limb: 0,
                    expected: Some(1),
                    got: None,
                })?;
        let mmcs_bit = if mode.merkle_path {
            let witness_id = callsite.tip5_mmcs_bit.ok_or(
                NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                    op_type: Self::tip5_op_type().as_str().into(),
                    row,
                    field: "mmcs_bit",
                    limb: 16,
                    expected: Some(1),
                    got: None,
                },
            )?;
            let value = Self::tip5_opened_goldilocks_value(
                row,
                16,
                expected_witness_ids,
                opened_values,
                witness_id,
            )?;
            if value == Goldilocks::ZERO {
                false
            } else if value == Goldilocks::ONE {
                true
            } else {
                return Err(NativeTerminalVerifyError::Tip5InputMismatch { row, limb: 16 });
            }
        } else {
            false
        };
        let has_ctl_output = callsite.outputs.iter().any(Option::is_some);
        let mut values = Vec::new();
        for trace_limb in 0..16 {
            if callsite.inputs[trace_limb].is_some() {
                continue;
            }
            let serialized = Self::should_serialize_tip5_hidden_limb(
                callsite,
                mode,
                has_ctl_output,
                mmcs_bit,
                trace_limb,
            );
            let value = if serialized {
                let Some(value_le) = values_le.get(*value_offset) else {
                    return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                        npo_index,
                        expected: *value_offset + 1,
                        got: values_le.len(),
                    });
                };
                *value_offset += 1;
                u64::from_le_bytes(*value_le)
            } else if mode.merkle_path {
                let bus_limb = Self::terminal_tip5_bus_limb_from_trace_limb(
                    mode,
                    has_ctl_output,
                    mmcs_bit,
                    trace_limb,
                );
                if bus_limb < 5 {
                    if mode.new_start {
                        0
                    } else {
                        previous_merkle_output.ok_or(
                            NativeTerminalVerifyError::Tip5InputMismatch {
                                row,
                                limb: trace_limb,
                            },
                        )?[bus_limb]
                            .as_canonical_u64()
                    }
                } else if bus_limb >= 10 {
                    0
                } else {
                    0
                }
            } else if mode.new_start {
                0
            } else {
                previous_normal_output.ok_or(NativeTerminalVerifyError::Tip5InputMismatch {
                    row,
                    limb: trace_limb,
                })?[trace_limb]
                    .as_canonical_u64()
            };
            values.push(TerminalTip5HiddenInputValue {
                limb: trace_limb,
                value_basis: vec![value],
            });
        }
        Ok(values)
    }

    fn verify_npo_witness_openings<F>(
        &self,
        npo_index: usize,
        expected_witness_ids: &[WitnessId],
        witness_commitment: &TerminalOracleCommitment,
        openings: &[TerminalOracleOpening],
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        if openings.len() != expected_witness_ids.len() {
            return Err(NativeTerminalVerifyError::TerminalNpoOpeningCountMismatch {
                npo_index,
                expected: expected_witness_ids.len(),
                got: openings.len(),
            });
        }

        let mut values = Vec::with_capacity(openings.len());
        for (opening_idx, (expected_witness, opening)) in expected_witness_ids
            .iter()
            .copied()
            .zip(openings)
            .enumerate()
        {
            if opening.index != expected_witness.0 as usize {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoOpeningWitnessMismatch {
                        npo_index,
                        opening: opening_idx,
                        expected: expected_witness.0,
                        got: opening.index,
                    },
                );
            }
            self.verify_terminal_oracle_opening(witness_commitment, opening)?;
            values.push(Self::field_from_goldilocks_basis_u64::<F>(
                &opening.value_basis,
            )?);
        }
        Ok(values)
    }

    fn verify_npo_witness_values<F>(
        npo_index: usize,
        expected_witness_ids: &[WitnessId],
        opened_witness_values: &[(usize, F)],
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: Copy,
    {
        let mut values = Vec::with_capacity(expected_witness_ids.len());
        for (opening_idx, expected_witness) in expected_witness_ids.iter().copied().enumerate() {
            let index = expected_witness.0 as usize;
            let Ok(position) =
                opened_witness_values.binary_search_by_key(&index, |(index, _)| *index)
            else {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoOpeningWitnessMismatch {
                        npo_index,
                        opening: opening_idx,
                        expected: expected_witness.0,
                        got: index,
                    },
                );
            };
            values.push(opened_witness_values[position].1);
        }
        Ok(values)
    }

    fn tip5_opened_goldilocks_value<F>(
        row: usize,
        limb: usize,
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
        wanted: WitnessId,
    ) -> Result<Goldilocks, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks> + Copy,
    {
        let value = expected_witness_ids
            .iter()
            .position(|id| *id == wanted)
            .map(|idx| opened_values[idx])
            .ok_or(NativeTerminalVerifyError::MissingWitness {
                witness_id: wanted.0,
            })?;
        let basis = value.as_basis_coefficients_slice();
        if basis
            .iter()
            .copied()
            .skip(1)
            .any(|coeff| coeff != Goldilocks::ZERO)
        {
            return Err(NativeTerminalVerifyError::Tip5InputMismatch { row, limb });
        }
        Ok(basis[0])
    }

    fn apply_terminal_tip5_merkle_swap(state: &mut [Goldilocks; 16]) {
        for limb in 0..5 {
            state.swap(limb, 5 + limb);
        }
    }

    fn terminal_tip5_bus_limb_from_trace_limb(
        mode: Tip5TerminalMode,
        has_ctl_output: bool,
        mmcs_bit: bool,
        trace_limb: usize,
    ) -> usize {
        if mode.merkle_path && has_ctl_output && mmcs_bit {
            if trace_limb < 5 {
                trace_limb + 5
            } else if trace_limb < 10 {
                trace_limb - 5
            } else {
                trace_limb
            }
        } else {
            trace_limb
        }
    }

    fn should_serialize_tip5_hidden_limb(
        callsite: &NativeTerminalNpoCallsite,
        mode: Tip5TerminalMode,
        has_ctl_output: bool,
        mmcs_bit: bool,
        trace_limb: usize,
    ) -> bool {
        if callsite.inputs[trace_limb].is_some() || !mode.merkle_path {
            return false;
        }
        let bus_limb = Self::terminal_tip5_bus_limb_from_trace_limb(
            mode,
            has_ctl_output,
            mmcs_bit,
            trace_limb,
        );
        (5..10).contains(&bus_limb)
    }

    fn terminal_fold_round_indices(
        initial_indices: &[usize],
        base_len: usize,
        rounds: usize,
    ) -> Vec<Vec<usize>> {
        let mut expected_round_indices = vec![Vec::new(); rounds];
        for initial_index in initial_indices {
            let mut index = *initial_index;
            let mut current_len = base_len;
            for round_indices in &mut expected_round_indices {
                let pair_index = (index / 2) * 2;
                Self::push_unique_usize(round_indices, pair_index);
                if pair_index + 1 < current_len {
                    Self::push_unique_usize(round_indices, pair_index + 1);
                }
                index /= 2;
                current_len = current_len.div_ceil(2);
            }
        }
        for round_indices in &mut expected_round_indices {
            round_indices.sort_unstable();
        }
        expected_round_indices
    }

    fn verify_terminal_oracle_value_indices<F>(
        expected_indices: &[usize],
        opened_values: &[(usize, F)],
    ) -> Result<(), NativeTerminalVerifyError> {
        if opened_values.len() != expected_indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalOracleQueryLengthMismatch {
                    expected: expected_indices.len(),
                    got: opened_values.len(),
                },
            );
        }
        for (query, (expected, (got, _))) in expected_indices.iter().zip(opened_values).enumerate()
        {
            if got != expected {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                        query,
                        expected: *expected,
                        got: *got,
                    },
                );
            }
        }
        Ok(())
    }

    fn terminal_opened_value<F>(
        opened_values: &[(usize, F)],
        index: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: Copy,
    {
        let position = opened_values
            .binary_search_by_key(&index, |(opened_index, _)| *opened_index)
            .map_err(
                |_| NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch {
                    query: 0,
                    expected: index,
                    got: index,
                },
            )?;
        Ok(opened_values[position].1)
    }

    fn verify_prelude_binds_commitment(
        prelude: &TerminalProofPrelude,
        commitment: &TerminalOracleCommitment,
    ) -> Result<(), NativeTerminalVerifyError> {
        if !prelude
            .commitments
            .iter()
            .any(|bound_root| *bound_root == commitment.root)
        {
            return Err(
                NativeTerminalVerifyError::TerminalPreludeCommitmentNotBound {
                    root: commitment.root,
                },
            );
        }
        Ok(())
    }

    fn verify_terminal_oracle_commitment_identity(
        commitment: &TerminalOracleCommitment,
        expected_label: &str,
        expected_len: usize,
    ) -> Result<(), NativeTerminalVerifyError> {
        if commitment.label != expected_label {
            return Err(
                NativeTerminalVerifyError::TerminalOracleCommitmentLabelMismatch {
                    expected: expected_label.into(),
                    got: commitment.label.clone(),
                },
            );
        }
        if commitment.values_len != expected_len {
            return Err(
                NativeTerminalVerifyError::TerminalOracleCommitmentLengthMismatch {
                    label: expected_label.into(),
                    expected: expected_len,
                    got: commitment.values_len,
                },
            );
        }
        Ok(())
    }

    fn verify_terminal_witness_commitment_identity<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        commitment: &TerminalOracleCommitment,
    ) -> Result<(), NativeTerminalVerifyError> {
        Self::verify_terminal_oracle_commitment_identity(
            commitment,
            Self::witness_oracle_label(),
            verifying_key.header.fingerprint.witness_count as usize,
        )
    }

    fn verify_terminal_assignment_commitment_identity<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        commitment: &TerminalOracleCommitment,
    ) -> Result<(), NativeTerminalVerifyError> {
        let variables = 1
            + verifying_key.header.fingerprint.public_flat_len
            + verifying_key.header.fingerprint.witness_count as usize;
        Self::verify_terminal_oracle_commitment_identity(
            commitment,
            Self::assignment_oracle_label(),
            variables,
        )
    }

    fn verify_terminal_residual_commitment_identity<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        commitment: &TerminalOracleCommitment,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let relation = verifying_key.primitive_quadratic_relation()?;
        Self::verify_terminal_oracle_commitment_identity(
            commitment,
            Self::quadratic_residual_oracle_label(),
            relation.constraints.len(),
        )
    }

    fn verify_terminal_combined_validity_commitment_identity<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        commitment: &TerminalOracleCommitment,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let relation = verifying_key.primitive_quadratic_relation()?;
        let expected_len =
            relation.constraints.len() + Self::terminal_npo_validity_domain_len::<F>(verifying_key);
        Self::verify_terminal_oracle_commitment_identity(
            commitment,
            Self::combined_validity_oracle_label(),
            expected_len,
        )
    }

    fn quadratic_residual_oracle_label() -> &'static str {
        "quadratic_residual"
    }

    fn witness_oracle_label() -> &'static str {
        "witness"
    }

    fn npo_exhaustive_residual_oracle_label() -> &'static str {
        "npo_exhaustive_residual"
    }

    fn evaluate_terminal_npo_row_from_witness_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
        indexed_witness_values: &[(usize, F)],
        npo_index: usize,
    ) -> Result<TerminalNpoRowEvaluation, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let row_ref = Self::terminal_npo_row(verifying_key, npo_index).ok_or(
            NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                query: 0,
                expected: npo_index,
                got: npo_index,
            },
        )?;
        match row_ref {
            NativeTerminalNpoRowRef::Tip5 { row, callsite, .. } => {
                let (local_opening, expected_witness_ids) =
                    self.terminal_npo_local_opening_goldilocks(verifying_key, witness, npo_index)?;
                let opened_values = Self::verify_npo_witness_values(
                    npo_index,
                    &expected_witness_ids,
                    indexed_witness_values,
                )?;
                let hidden_inputs = Self::terminal_tip5_hidden_inputs_from_compact(
                    local_opening.npo_index,
                    verifying_key,
                    local_opening.tip5_hidden_input_nonzero_mask,
                    &local_opening.tip5_hidden_input_values_le,
                )?;
                self.evaluate_sampled_tip5_npo_row_values::<F>(
                    npo_index,
                    row,
                    callsite,
                    &hidden_inputs,
                    &expected_witness_ids,
                    &opened_values,
                )
            }
            NativeTerminalNpoRowRef::Recompose {
                op_type,
                row,
                callsite,
            } => {
                let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                let opened_values = Self::verify_npo_witness_values(
                    npo_index,
                    &expected_witness_ids,
                    indexed_witness_values,
                )?;
                self.evaluate_sampled_recompose_npo_row_values::<F>(
                    npo_index,
                    op_type,
                    row,
                    callsite,
                    &[],
                    &expected_witness_ids,
                    &opened_values,
                )
            }
        }
    }

    fn evaluate_terminal_npo_row_exhaustive_from_witness_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        witness: &TerminalWitness<F>,
        indexed_witness_values: &[(usize, F)],
        npo_index: usize,
        previous_normal_output: &mut Option<[Goldilocks; 16]>,
        previous_merkle_output: &mut Option<[Goldilocks; 16]>,
    ) -> Result<TerminalNpoRowEvaluation, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let row_ref = Self::terminal_npo_row(verifying_key, npo_index).ok_or(
            NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch {
                query: 0,
                expected: npo_index,
                got: npo_index,
            },
        )?;
        match row_ref {
            NativeTerminalNpoRowRef::Tip5 { row, callsite, .. } => {
                let (local_opening, expected_witness_ids) =
                    self.terminal_npo_local_opening_goldilocks(verifying_key, witness, npo_index)?;
                let opened_values = Self::verify_npo_witness_values(
                    npo_index,
                    &expected_witness_ids,
                    indexed_witness_values,
                )?;
                let hidden_inputs = Self::terminal_tip5_hidden_inputs_from_compact(
                    local_opening.npo_index,
                    verifying_key,
                    local_opening.tip5_hidden_input_nonzero_mask,
                    &local_opening.tip5_hidden_input_values_le,
                )?;
                self.evaluate_exhaustive_tip5_npo_row_values::<F>(
                    npo_index,
                    row,
                    callsite,
                    &hidden_inputs,
                    &expected_witness_ids,
                    &opened_values,
                    previous_normal_output,
                    previous_merkle_output,
                )
            }
            NativeTerminalNpoRowRef::Recompose {
                op_type,
                row,
                callsite,
            } => {
                let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
                let opened_values = Self::verify_npo_witness_values(
                    npo_index,
                    &expected_witness_ids,
                    indexed_witness_values,
                )?;
                self.evaluate_sampled_recompose_npo_row_values::<F>(
                    npo_index,
                    op_type,
                    row,
                    callsite,
                    &[],
                    &expected_witness_ids,
                    &opened_values,
                )
            }
        }
    }

    fn terminal_npo_row_evaluation_component_values<F>(
        evaluation: &TerminalNpoRowEvaluation,
    ) -> Vec<F>
    where
        F: Field + From<Goldilocks>,
    {
        evaluation
            .residuals
            .iter()
            .flat_map(|residual| residual.basis.iter().copied())
            .map(F::from)
            .collect()
    }

    fn terminal_npo_row_evaluation_component_value<F>(
        evaluation: &TerminalNpoRowEvaluation,
        component_offset: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: Field + From<Goldilocks>,
    {
        Self::terminal_npo_row_evaluation_component_values::<F>(evaluation)
            .get(component_offset)
            .copied()
            .ok_or(
                NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch {
                    query: 0,
                    expected: component_offset,
                    got: component_offset,
                },
            )
    }

    fn terminal_goldilocks_residual(
        kind: TerminalNpoResidualKind,
        limb: usize,
        actual: Goldilocks,
        expected: Goldilocks,
    ) -> TerminalNpoRowResidual {
        TerminalNpoRowResidual {
            kind,
            limb,
            basis: vec![actual - expected],
        }
    }

    fn terminal_field_residual<F>(
        kind: TerminalNpoResidualKind,
        limb: usize,
        actual: F,
        expected: F,
    ) -> TerminalNpoRowResidual
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        TerminalNpoRowResidual {
            kind,
            limb,
            basis: (actual - expected).as_basis_coefficients_slice().to_vec(),
        }
    }

    fn push_terminal_npo_residual(
        evaluation: &mut TerminalNpoRowEvaluation,
        residual: TerminalNpoRowResidual,
    ) {
        evaluation.residuals.push(residual);
    }

    fn reject_nonzero_terminal_npo_residual(
        row: usize,
        residual: &TerminalNpoRowResidual,
    ) -> Result<(), NativeTerminalVerifyError> {
        match residual.kind {
            TerminalNpoResidualKind::Tip5Input
            | TerminalNpoResidualKind::Tip5ChainInput
            | TerminalNpoResidualKind::Tip5MmcsBit => {
                Err(NativeTerminalVerifyError::Tip5InputMismatch {
                    row,
                    limb: residual.limb,
                })
            }
            TerminalNpoResidualKind::Tip5Output => {
                Err(NativeTerminalVerifyError::Tip5OutputMismatch {
                    row,
                    limb: residual.limb,
                })
            }
            TerminalNpoResidualKind::RecomposeInput => {
                Err(NativeTerminalVerifyError::RecomposeInputMismatch {
                    row,
                    limb: residual.limb,
                })
            }
            TerminalNpoResidualKind::RecomposeOutput => {
                Err(NativeTerminalVerifyError::RecomposeOutputMismatch { row })
            }
        }
    }

    fn reject_nonzero_terminal_npo_evaluation(
        row: usize,
        evaluation: &TerminalNpoRowEvaluation,
    ) -> Result<(), NativeTerminalVerifyError> {
        if let Some(residual) = evaluation.first_nonzero() {
            return Self::reject_nonzero_terminal_npo_residual(row, residual);
        }
        Ok(())
    }

    fn verify_sampled_tip5_npo_row<F>(
        &self,
        npo_index: usize,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        witness_commitment: &TerminalOracleCommitment,
        opening: &TerminalNpoOpening,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let hidden_input_count = callsite
            .inputs
            .iter()
            .filter(|input| input.is_none())
            .count();
        if opening.tip5_hidden_input_values.len() != hidden_input_count {
            return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                npo_index,
                expected: hidden_input_count,
                got: opening.tip5_hidden_input_values.len(),
            });
        }

        let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
        let opened_values = self.verify_npo_witness_openings::<F>(
            npo_index,
            &expected_witness_ids,
            witness_commitment,
            &opening.witness_openings,
        )?;
        self.verify_sampled_tip5_npo_row_values::<F>(
            npo_index,
            row,
            callsite,
            &opening.tip5_hidden_input_values,
            &expected_witness_ids,
            &opened_values,
        )
    }

    fn verify_exhaustive_tip5_npo_row_values<F>(
        &self,
        npo_index: usize,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        tip5_hidden_input_values: &[TerminalTip5HiddenInputValue],
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
        previous_normal_output: &mut Option<[Goldilocks; 16]>,
        previous_merkle_output: &mut Option<[Goldilocks; 16]>,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let evaluation = self.evaluate_exhaustive_tip5_npo_row_values::<F>(
            npo_index,
            row,
            callsite,
            tip5_hidden_input_values,
            expected_witness_ids,
            opened_values,
            previous_normal_output,
            previous_merkle_output,
        )?;
        Self::reject_nonzero_terminal_npo_evaluation(row, &evaluation)
    }

    fn evaluate_exhaustive_tip5_npo_row_values<F>(
        &self,
        npo_index: usize,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        tip5_hidden_input_values: &[TerminalTip5HiddenInputValue],
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
        previous_normal_output: &mut Option<[Goldilocks; 16]>,
        previous_merkle_output: &mut Option<[Goldilocks; 16]>,
    ) -> Result<TerminalNpoRowEvaluation, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let mut evaluation = self.evaluate_sampled_tip5_npo_row_values::<F>(
            npo_index,
            row,
            callsite,
            tip5_hidden_input_values,
            expected_witness_ids,
            opened_values,
        )?;
        let mode =
            callsite
                .tip5_mode
                .ok_or(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                    op_type: Self::tip5_op_type().as_str().into(),
                    row,
                    field: "tip5_mode",
                    limb: 0,
                    expected: Some(1),
                    got: None,
                })?;
        let opened_value = |wanted: WitnessId, limb: usize| {
            Self::tip5_opened_goldilocks_value(
                row,
                limb,
                expected_witness_ids,
                opened_values,
                wanted,
            )
        };

        let mut trace_state = [Goldilocks::ZERO; 16];
        for (limb, witness_id) in callsite.inputs.iter().copied().enumerate() {
            if let Some(witness_id) = witness_id {
                trace_state[limb] = opened_value(witness_id, limb)?;
            }
        }
        for hidden in tip5_hidden_input_values {
            trace_state[hidden.limb] = Goldilocks::from_u64(hidden.value_basis[0]);
        }

        let mmcs_bit = if mode.merkle_path {
            let witness_id = callsite.tip5_mmcs_bit.ok_or(
                NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                    op_type: Self::tip5_op_type().as_str().into(),
                    row,
                    field: "mmcs_bit",
                    limb: 16,
                    expected: Some(1),
                    got: None,
                },
            )?;
            let value = opened_value(witness_id, 16)?;
            let residual = TerminalNpoRowResidual {
                kind: TerminalNpoResidualKind::Tip5MmcsBit,
                limb: 16,
                basis: vec![value * (value - Goldilocks::ONE)],
            };
            Self::push_terminal_npo_residual(&mut evaluation, residual);
            value == Goldilocks::ONE
        } else {
            false
        };

        let has_ctl_output = callsite.outputs.iter().any(Option::is_some);
        if mode.merkle_path {
            let mut bus_state = trace_state;
            if has_ctl_output && mmcs_bit {
                Self::apply_terminal_tip5_merkle_swap(&mut bus_state);
            }
            for limb in 0..5 {
                if callsite.inputs[limb].is_none() {
                    let expected = if mode.new_start {
                        Goldilocks::ZERO
                    } else {
                        previous_merkle_output
                            .ok_or(NativeTerminalVerifyError::Tip5InputMismatch { row, limb })?
                            [limb]
                    };
                    let residual = Self::terminal_goldilocks_residual(
                        TerminalNpoResidualKind::Tip5ChainInput,
                        limb,
                        bus_state[limb],
                        expected,
                    );
                    Self::push_terminal_npo_residual(&mut evaluation, residual);
                }
            }
            for limb in 10..16 {
                if callsite.inputs[limb].is_none() {
                    let residual = Self::terminal_goldilocks_residual(
                        TerminalNpoResidualKind::Tip5ChainInput,
                        limb,
                        bus_state[limb],
                        Goldilocks::ZERO,
                    );
                    Self::push_terminal_npo_residual(&mut evaluation, residual);
                }
            }

            let mut perm_input = trace_state;
            if !has_ctl_output && mmcs_bit {
                Self::apply_terminal_tip5_merkle_swap(&mut perm_input);
            }
            Tip5Perm.permute_mut(&mut perm_input);
            *previous_merkle_output = Some(perm_input);
        } else {
            for limb in 0..16 {
                if callsite.inputs[limb].is_none() {
                    let expected = if mode.new_start {
                        Goldilocks::ZERO
                    } else {
                        previous_normal_output
                            .ok_or(NativeTerminalVerifyError::Tip5InputMismatch { row, limb })?
                            [limb]
                    };
                    let residual = Self::terminal_goldilocks_residual(
                        TerminalNpoResidualKind::Tip5ChainInput,
                        limb,
                        trace_state[limb],
                        expected,
                    );
                    Self::push_terminal_npo_residual(&mut evaluation, residual);
                }
            }
            Tip5Perm.permute_mut(&mut trace_state);
            *previous_normal_output = Some(trace_state);
        }

        Ok(evaluation)
    }

    fn verify_sampled_tip5_npo_row_values<F>(
        &self,
        npo_index: usize,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        tip5_hidden_input_values: &[TerminalTip5HiddenInputValue],
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let evaluation = self.evaluate_sampled_tip5_npo_row_values::<F>(
            npo_index,
            row,
            callsite,
            tip5_hidden_input_values,
            expected_witness_ids,
            opened_values,
        )?;
        Self::reject_nonzero_terminal_npo_evaluation(row, &evaluation)
    }

    fn evaluate_sampled_tip5_npo_row_values<F>(
        &self,
        npo_index: usize,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        tip5_hidden_input_values: &[TerminalTip5HiddenInputValue],
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
    ) -> Result<TerminalNpoRowEvaluation, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let hidden_input_count = callsite
            .inputs
            .iter()
            .filter(|input| input.is_none())
            .count();
        if tip5_hidden_input_values.len() != hidden_input_count {
            return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                npo_index,
                expected: hidden_input_count,
                got: tip5_hidden_input_values.len(),
            });
        }
        let mut evaluation = TerminalNpoRowEvaluation::new(TerminalNpoRowKind::Tip5Goldilocks);
        let opened_value = |wanted: WitnessId| {
            expected_witness_ids
                .iter()
                .position(|id| *id == wanted)
                .map(|idx| opened_values[idx])
                .ok_or(NativeTerminalVerifyError::MissingWitness {
                    witness_id: wanted.0,
                })
        };

        let mut state = [Goldilocks::ZERO; 16];
        for (limb, witness_id) in callsite.inputs.iter().copied().enumerate() {
            if let Some(witness_id) = witness_id {
                let opened = opened_value(witness_id)?;
                let opened_basis = opened.as_basis_coefficients_slice();
                if opened_basis.is_empty() {
                    return Err(NativeTerminalVerifyError::Tip5InputMismatch { row, limb });
                }
                state[limb] = opened_basis[0];
                Self::push_terminal_npo_residual(
                    &mut evaluation,
                    Self::terminal_field_residual(
                        TerminalNpoResidualKind::Tip5Input,
                        limb,
                        opened,
                        Self::embed_goldilocks(state[limb]),
                    ),
                );
            }
        }

        let mut seen_hidden = [false; 16];
        for hidden in tip5_hidden_input_values {
            if hidden.limb >= 16 {
                return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                    npo_index,
                    expected: 16,
                    got: hidden.limb + 1,
                });
            }
            if seen_hidden[hidden.limb] {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoTip5HiddenInputDuplicate {
                        npo_index,
                        limb: hidden.limb,
                    },
                );
            }
            seen_hidden[hidden.limb] = true;
            if callsite.inputs[hidden.limb].is_some() {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoTip5HiddenInputUnexpected {
                        npo_index,
                        limb: hidden.limb,
                    },
                );
            }
            if hidden.value_basis.len() != 1 {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoTip5InputValueDimensionMismatch {
                        npo_index,
                        limb: hidden.limb,
                        expected: 1,
                        got: hidden.value_basis.len(),
                    },
                );
            }
            if hidden.value_basis[0] >= Goldilocks::ORDER_U64 {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleOpeningValueNonCanonical {
                        limb: hidden.limb,
                        value: hidden.value_basis[0],
                    },
                );
            }
            state[hidden.limb] = Goldilocks::from_u64(hidden.value_basis[0]);
        }

        Tip5Perm.permute_mut(&mut state);
        for (limb, witness_id) in callsite.outputs.iter().copied().enumerate() {
            if let Some(witness_id) = witness_id {
                Self::push_terminal_npo_residual(
                    &mut evaluation,
                    Self::terminal_field_residual(
                        TerminalNpoResidualKind::Tip5Output,
                        limb,
                        opened_value(witness_id)?,
                        Self::embed_goldilocks(state[limb]),
                    ),
                );
            }
        }
        Ok(evaluation)
    }

    fn verify_sampled_recompose_npo_row<F>(
        &self,
        npo_index: usize,
        op_type: &str,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        witness_commitment: &TerminalOracleCommitment,
        opening: &TerminalNpoOpening,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        if !opening.tip5_hidden_input_values.is_empty() {
            return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                npo_index,
                expected: 0,
                got: opening.tip5_hidden_input_values.len(),
            });
        }

        let expected_kind = if op_type == NpoTypeId::recompose().as_str() {
            RecomposeTraceKind::Standard
        } else if op_type == NpoTypeId::recompose_with_coeff_lookups().as_str() {
            RecomposeTraceKind::WithCoeffLookups
        } else {
            return Err(NativeTerminalVerifyError::MissingNonPrimitiveTrace {
                op_type: op_type.into(),
            });
        };
        let _ = expected_kind;

        let expected_witness_ids = Self::npo_callsite_witness_ids(callsite);
        let opened_values = self.verify_npo_witness_openings::<F>(
            npo_index,
            &expected_witness_ids,
            witness_commitment,
            &opening.witness_openings,
        )?;
        self.verify_sampled_recompose_npo_row_values::<F>(
            npo_index,
            op_type,
            row,
            callsite,
            &opening.tip5_hidden_input_values,
            &expected_witness_ids,
            &opened_values,
        )
    }

    fn verify_sampled_recompose_npo_row_values<F>(
        &self,
        npo_index: usize,
        op_type: &str,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        tip5_hidden_input_values: &[TerminalTip5HiddenInputValue],
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let evaluation = self.evaluate_sampled_recompose_npo_row_values::<F>(
            npo_index,
            op_type,
            row,
            callsite,
            tip5_hidden_input_values,
            expected_witness_ids,
            opened_values,
        )?;
        Self::reject_nonzero_terminal_npo_evaluation(row, &evaluation)
    }

    fn evaluate_sampled_recompose_npo_row_values<F>(
        &self,
        npo_index: usize,
        op_type: &str,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        tip5_hidden_input_values: &[TerminalTip5HiddenInputValue],
        expected_witness_ids: &[WitnessId],
        opened_values: &[F],
    ) -> Result<TerminalNpoRowEvaluation, NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        if !tip5_hidden_input_values.is_empty() {
            return Err(NativeTerminalVerifyError::TerminalNpoTip5InputValueLength {
                npo_index,
                expected: 0,
                got: tip5_hidden_input_values.len(),
            });
        }

        let expected_kind = if op_type == NpoTypeId::recompose().as_str() {
            RecomposeTraceKind::Standard
        } else if op_type == NpoTypeId::recompose_with_coeff_lookups().as_str() {
            RecomposeTraceKind::WithCoeffLookups
        } else {
            return Err(NativeTerminalVerifyError::MissingNonPrimitiveTrace {
                op_type: op_type.into(),
            });
        };
        let _ = expected_kind;
        let row_kind = if op_type == NpoTypeId::recompose().as_str() {
            TerminalNpoRowKind::Recompose
        } else {
            TerminalNpoRowKind::RecomposeWithCoeffLookups
        };
        let mut evaluation = TerminalNpoRowEvaluation::new(row_kind);

        let opened_value = |wanted: WitnessId| {
            expected_witness_ids
                .iter()
                .position(|id| *id == wanted)
                .map(|idx| opened_values[idx])
                .ok_or(NativeTerminalVerifyError::MissingWitness {
                    witness_id: wanted.0,
                })
        };

        let expected_d = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        if callsite.inputs.len() != expected_d {
            return Err(NativeTerminalVerifyError::RecomposeTraceInputLength {
                row,
                expected: expected_d,
                got: callsite.inputs.len(),
            });
        }

        let mut coeffs = Vec::with_capacity(expected_d);
        for (limb, witness_id) in callsite.inputs.iter().copied().enumerate() {
            let Some(witness_id) = witness_id else {
                return Err(NativeTerminalVerifyError::RecomposeInputMismatch { row, limb });
            };
            let value = opened_value(witness_id)?;
            let basis = value.as_basis_coefficients_slice();
            if basis.is_empty() {
                return Err(NativeTerminalVerifyError::RecomposeInputMismatch { row, limb });
            }
            coeffs.push(basis[0]);
            Self::push_terminal_npo_residual(
                &mut evaluation,
                Self::terminal_field_residual(
                    TerminalNpoResidualKind::RecomposeInput,
                    limb,
                    value,
                    Self::embed_goldilocks(basis[0]),
                ),
            );
        }

        let expected_output = <F as BasedVectorSpace<Goldilocks>>::from_basis_coefficients_slice(
            &coeffs,
        )
        .ok_or(NativeTerminalVerifyError::RecomposeTraceValueLength {
            row,
            expected: expected_d,
            got: coeffs.len(),
        })?;
        let Some(output_witness_id) = callsite.outputs.first().copied().flatten() else {
            return Err(NativeTerminalVerifyError::RecomposeOutputMismatch { row });
        };
        Self::push_terminal_npo_residual(
            &mut evaluation,
            Self::terminal_field_residual(
                TerminalNpoResidualKind::RecomposeOutput,
                0,
                opened_value(output_witness_id)?,
                expected_output,
            ),
        );
        Ok(evaluation)
    }

    fn validate_terminal_residual_fold_commitments<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        residual_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field,
    {
        let relation = verifying_key.primitive_quadratic_relation()?;
        if residual_commitment.label != Self::quadratic_residual_oracle_label() {
            return Err(
                NativeTerminalVerifyError::TerminalOracleCommitmentLabelMismatch {
                    expected: Self::quadratic_residual_oracle_label().into(),
                    got: residual_commitment.label.clone(),
                },
            );
        }
        if residual_commitment.values_len != relation.constraints.len() {
            return Err(
                NativeTerminalVerifyError::TerminalQuadraticResidualDomainLengthMismatch {
                    expected: relation.constraints.len(),
                    got: residual_commitment.values_len,
                },
            );
        }
        let mut len = residual_commitment.values_len;
        let mut expected_rounds = 0usize;
        while len > 1 {
            len = len.div_ceil(2);
            expected_rounds += 1;
        }
        if fold_commitments.len() != expected_rounds {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                    round: expected_rounds,
                    expected: expected_rounds,
                    got: fold_commitments.len(),
                },
            );
        }

        let mut current_len = residual_commitment.values_len;
        for (round, commitment) in fold_commitments.iter().enumerate() {
            let expected_len = current_len.div_ceil(2);
            if commitment.values_len != expected_len {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                        round,
                        expected: expected_len,
                        got: commitment.values_len,
                    },
                );
            }
            let expected_label = Self::residual_fold_oracle_label(round);
            if commitment.label != expected_label {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldCommitmentLabelMismatch {
                        round,
                        expected: expected_label,
                        got: commitment.label.clone(),
                    },
                );
            }
            current_len = expected_len;
        }
        Ok(())
    }

    fn validate_terminal_npo_validity_fold_commitments<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        validity_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let domain_len = Self::terminal_npo_validity_domain_len::<F>(verifying_key);
        if validity_commitment.values_len != domain_len {
            return Err(
                NativeTerminalVerifyError::TerminalNpoValidityDomainLengthMismatch {
                    expected: domain_len,
                    got: validity_commitment.values_len,
                },
            );
        }
        if validity_commitment.label != Self::npo_validity_oracle_label() {
            return Err(
                NativeTerminalVerifyError::TerminalNpoValidityFoldCommitmentLabelMismatch {
                    round: 0,
                    expected: Self::npo_validity_oracle_label().into(),
                    got: validity_commitment.label.clone(),
                },
            );
        }

        let mut len = validity_commitment.values_len;
        let mut expected_rounds = 0usize;
        while len > 1 {
            len = len.div_ceil(2);
            expected_rounds += 1;
        }
        if fold_commitments.len() != expected_rounds {
            return Err(
                NativeTerminalVerifyError::TerminalNpoValidityFoldCommitmentLengthMismatch {
                    round: expected_rounds,
                    expected: expected_rounds,
                    got: fold_commitments.len(),
                },
            );
        }

        let mut current_len = validity_commitment.values_len;
        for (round, commitment) in fold_commitments.iter().enumerate() {
            let expected_len = current_len.div_ceil(2);
            if commitment.values_len != expected_len {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoValidityFoldCommitmentLengthMismatch {
                        round,
                        expected: expected_len,
                        got: commitment.values_len,
                    },
                );
            }
            let expected_label = Self::npo_validity_fold_oracle_label(round);
            if commitment.label != expected_label {
                return Err(
                    NativeTerminalVerifyError::TerminalNpoValidityFoldCommitmentLabelMismatch {
                        round,
                        expected: expected_label,
                        got: commitment.label.clone(),
                    },
                );
            }
            current_len = expected_len;
        }
        Ok(())
    }

    fn validate_terminal_combined_validity_fold_commitments<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        combined_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        Self::verify_terminal_combined_validity_commitment_identity::<F>(
            verifying_key,
            combined_commitment,
        )?;

        let mut len = combined_commitment.values_len;
        let mut expected_rounds = 0usize;
        while len > 1 {
            len = len.div_ceil(2);
            expected_rounds += 1;
        }
        if fold_commitments.len() != expected_rounds {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                    round: expected_rounds,
                    expected: expected_rounds,
                    got: fold_commitments.len(),
                },
            );
        }

        let mut current_len = combined_commitment.values_len;
        for (round, commitment) in fold_commitments.iter().enumerate() {
            let expected_len = current_len.div_ceil(2);
            if commitment.values_len != expected_len {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                        round,
                        expected: expected_len,
                        got: commitment.values_len,
                    },
                );
            }
            let expected_label = Self::combined_validity_fold_oracle_label(round);
            if commitment.label != expected_label {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldCommitmentLabelMismatch {
                        round,
                        expected: expected_label,
                        got: commitment.label.clone(),
                    },
                );
            }
            current_len = expected_len;
        }
        Ok(())
    }

    fn validate_terminal_assignment_fold_commitments<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        assignment_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
    ) -> Result<(), NativeTerminalVerifyError> {
        Self::verify_terminal_assignment_commitment_identity::<F>(
            verifying_key,
            assignment_commitment,
        )?;

        let mut len = assignment_commitment.values_len;
        let mut expected_rounds = 0usize;
        while len > 1 {
            len = len.div_ceil(2);
            expected_rounds += 1;
        }
        if fold_commitments.len() != expected_rounds {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                    round: expected_rounds,
                    expected: expected_rounds,
                    got: fold_commitments.len(),
                },
            );
        }

        let mut current_len = assignment_commitment.values_len;
        for (round, commitment) in fold_commitments.iter().enumerate() {
            let expected_len = current_len.div_ceil(2);
            if commitment.values_len != expected_len {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                        round,
                        expected: expected_len,
                        got: commitment.values_len,
                    },
                );
            }
            let expected_label = Self::assignment_fold_oracle_label(round);
            if commitment.label != expected_label {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldCommitmentLabelMismatch {
                        round,
                        expected: expected_label,
                        got: commitment.label.clone(),
                    },
                );
            }
            current_len = expected_len;
        }
        Ok(())
    }

    fn validate_terminal_assignment_point<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
        point: &[F],
    ) -> Result<(), NativeTerminalVerifyError> {
        let variables = 1
            + verifying_key.header.fingerprint.public_flat_len
            + verifying_key.header.fingerprint.witness_count as usize;
        let expected = Self::terminal_mle_log_size(variables);
        if point.len() != expected {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                    round: expected,
                    expected,
                    got: point.len(),
                },
            );
        }
        Ok(())
    }

    fn validate_terminal_r1cs_row_point<F>(
        relation: &TerminalSparseR1csRelation<F>,
        point: &[F],
    ) -> Result<(), NativeTerminalVerifyError> {
        if point.len() != relation.log_rows {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                    round: relation.log_rows,
                    expected: relation.log_rows,
                    got: point.len(),
                },
            );
        }
        Ok(())
    }

    fn accept_terminal_query_index(
        indices: &mut Vec<usize>,
        candidate: usize,
        domain_len: usize,
        num_queries: usize,
    ) {
        if domain_len >= num_queries && indices.iter().any(|index| *index == candidate) {
            return;
        }
        indices.push(candidate);
    }

    fn terminal_fold_base_value_basis<'a>(
        opening: &'a TerminalResidualFoldQueryOpening,
        final_value_basis: &'a [u64],
    ) -> Option<&'a [u64]> {
        if let Some(first_round) = opening.rounds.first() {
            if first_round.left.index == opening.initial_index {
                Some(&first_round.left.value_basis)
            } else {
                first_round
                    .right
                    .as_ref()
                    .filter(|right| right.index == opening.initial_index)
                    .map(|right| right.value_basis.as_slice())
            }
        } else {
            Some(final_value_basis)
        }
    }

    fn terminal_npo_validity_fold_base_value_basis(
        proof: &TerminalNpoValidityFoldProof,
        initial_index: usize,
    ) -> Option<&[u64]> {
        if let Some(round_opening) = proof.round_openings.first() {
            round_opening
                .openings
                .iter()
                .find(|opening| opening.index == initial_index)
                .map(|opening| opening.value_basis.as_slice())
        } else {
            Some(&proof.final_value_basis)
        }
    }

    fn verify_terminal_compact_fold_openings_goldilocks<F, D>(
        &self,
        base_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        final_value_basis: &[u64],
        round_openings: &[TerminalOracleMultiProof],
        openings: &[TerminalResidualFoldQueryOpening],
        query_plan: &TerminalQueryPlan,
        derive_challenge: D,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
        D: Fn(usize, &[TerminalOracleCommitment]) -> Result<F, NativeTerminalVerifyError>,
    {
        if openings.len() != query_plan.indices.len() {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldQueryLengthMismatch {
                    expected: query_plan.indices.len(),
                    got: openings.len(),
                },
            );
        }
        if round_openings.len() != fold_commitments.len() {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldCommitmentLengthMismatch {
                    round: fold_commitments.len(),
                    expected: fold_commitments.len(),
                    got: round_openings.len(),
                },
            );
        }

        let expected_round_indices = Self::terminal_fold_round_indices(
            &query_plan.indices,
            base_commitment.values_len,
            fold_commitments.len(),
        );
        let mut round_values = Vec::with_capacity(round_openings.len());
        for (round, proof_opening) in round_openings.iter().enumerate() {
            let current_commitment = if round == 0 {
                base_commitment
            } else {
                &fold_commitments[round - 1]
            };
            let values = self.verify_terminal_oracle_multi_proof_goldilocks::<F>(
                current_commitment,
                proof_opening,
            )?;
            Self::verify_terminal_oracle_value_indices(&expected_round_indices[round], &values)?;
            round_values.push(values);
        }

        let mut challenges = Vec::with_capacity(fold_commitments.len());
        for round in 0..fold_commitments.len() {
            challenges.push(derive_challenge(round, &fold_commitments[..round])?);
        }

        for (query, (opening, expected_index)) in
            openings.iter().zip(&query_plan.indices).enumerate()
        {
            if opening.initial_index != *expected_index {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldQueryIndexMismatch {
                        query,
                        expected: *expected_index,
                        got: opening.initial_index,
                    },
                );
            }
            if !opening.rounds.is_empty() {
                return Err(
                    NativeTerminalVerifyError::TerminalResidualFoldRoundCountMismatch {
                        query,
                        expected: 0,
                        got: opening.rounds.len(),
                    },
                );
            }

            let mut index = opening.initial_index;
            let mut current_len = base_commitment.values_len;
            for (round, challenge) in challenges.iter().copied().enumerate() {
                let expected_pair = (index / 2) * 2;
                let next_index = index / 2;
                let left = Self::terminal_opened_value(&round_values[round], expected_pair)
                    .map_err(|_| {
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "left",
                            expected: expected_pair,
                            got: expected_pair,
                        }
                    })?;
                let right = if expected_pair + 1 < current_len {
                    Self::terminal_opened_value(&round_values[round], expected_pair + 1).map_err(
                        |_| NativeTerminalVerifyError::TerminalResidualFoldRightOpeningMissing {
                            query,
                            round,
                            index: expected_pair + 1,
                        },
                    )?
                } else {
                    F::ZERO
                };
                let expected_next = left * (F::ONE - challenge) + right * challenge;
                let opened_next = if let Some(next_round_values) = round_values.get(round + 1) {
                    Self::terminal_opened_value(next_round_values, next_index).map_err(|_| {
                        NativeTerminalVerifyError::TerminalResidualFoldOpeningIndexMismatch {
                            query,
                            round,
                            field: "next",
                            expected: next_index,
                            got: next_index,
                        }
                    })?
                } else {
                    Self::field_from_goldilocks_basis_u64::<F>(final_value_basis)?
                };
                if opened_next != expected_next {
                    return Err(
                        NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch {
                            query,
                            round,
                        },
                    );
                }
                index = next_index;
                current_len = current_len.div_ceil(2);
            }
        }

        let final_commitment = fold_commitments.last().unwrap_or(base_commitment);
        let final_root = Self::terminal_oracle_leaf_digest_from_basis(
            &final_commitment.label,
            final_commitment.values_len,
            0,
            final_value_basis,
        );
        if final_root != final_commitment.root {
            return Err(
                NativeTerminalVerifyError::TerminalResidualFoldFinalRootMismatch {
                    expected: final_commitment.root,
                    got: final_root,
                },
            );
        }

        Ok(())
    }

    fn derive_terminal_residual_fold_challenge<F>(
        prelude: &TerminalProofPrelude,
        residual_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let expected = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        let mut basis = Vec::with_capacity(expected);
        let mut counter = 0u64;
        while basis.len() < expected {
            if counter > 4096 {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_residual_fold_challenge_block(
                prelude,
                residual_commitment,
                fold_commitments,
                round,
                counter,
            );
            for limb in block {
                if basis.len() == expected {
                    break;
                }
                if limb < Goldilocks::ORDER_U64 {
                    basis.push(limb);
                }
            }
            counter += 1;
        }
        Self::field_from_goldilocks_basis_u64(&basis)
    }

    fn derive_terminal_npo_validity_fold_challenge<F>(
        prelude: &TerminalProofPrelude,
        validity_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let expected = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        let mut basis = Vec::with_capacity(expected);
        let mut counter = 0u64;
        while basis.len() < expected {
            if counter > 4096 {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_npo_validity_fold_challenge_block(
                prelude,
                validity_commitment,
                fold_commitments,
                round,
                counter,
            );
            for limb in block {
                if basis.len() == expected {
                    break;
                }
                if limb < Goldilocks::ORDER_U64 {
                    basis.push(limb);
                }
            }
            counter += 1;
        }
        Self::field_from_goldilocks_basis_u64(&basis)
    }

    fn derive_terminal_combined_validity_fold_challenge<F>(
        prelude: &TerminalProofPrelude,
        combined_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let expected = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        let mut basis = Vec::with_capacity(expected);
        let mut counter = 0u64;
        while basis.len() < expected {
            if counter > 4096 {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_combined_validity_fold_challenge_block(
                prelude,
                combined_commitment,
                fold_commitments,
                round,
                counter,
            );
            for limb in block {
                if basis.len() == expected {
                    break;
                }
                if limb < Goldilocks::ORDER_U64 {
                    basis.push(limb);
                }
            }
            counter += 1;
        }
        Self::field_from_goldilocks_basis_u64(&basis)
    }

    fn derive_terminal_assignment_fold_challenge<F>(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let expected = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        let mut basis = Vec::with_capacity(expected);
        let mut counter = 0u64;
        while basis.len() < expected {
            if counter > 4096 {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            let block = Self::terminal_assignment_fold_challenge_block(
                prelude,
                assignment_commitment,
                fold_commitments,
                round,
                counter,
            );
            for limb in block {
                if basis.len() == expected {
                    break;
                }
                if limb < Goldilocks::ORDER_U64 {
                    basis.push(limb);
                }
            }
            counter += 1;
        }
        Self::field_from_goldilocks_basis_u64(&basis)
    }

    fn derive_terminal_r1cs_row_point<F>(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        log_rows: usize,
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let mut point = Vec::with_capacity(log_rows);
        for round in 0..log_rows {
            point.push(Self::derive_terminal_r1cs_field_challenge(|counter| {
                Self::terminal_r1cs_row_challenge_block(
                    prelude,
                    assignment_commitment,
                    round,
                    counter,
                )
            })?);
        }
        Ok(point)
    }

    fn derive_terminal_r1cs_row_product_anchor_point<F>(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        log_rows: usize,
    ) -> Result<Vec<F>, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let mut point = Vec::with_capacity(log_rows);
        for round in 0..log_rows {
            point.push(Self::derive_terminal_r1cs_field_challenge(|counter| {
                Self::terminal_r1cs_row_product_anchor_block(
                    prelude,
                    assignment_commitment,
                    round,
                    counter,
                )
            })?);
        }
        Ok(point)
    }

    fn derive_terminal_r1cs_batch_challenge<F>(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        row_point: &[F],
        claimed_a_basis: &[u64],
        claimed_b_basis: &[u64],
        claimed_c_basis: &[u64],
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let row_point_basis = Self::flatten_goldilocks_basis_values(row_point);
        Self::derive_terminal_r1cs_field_challenge(|counter| {
            Self::terminal_r1cs_batch_challenge_block(
                prelude,
                assignment_commitment,
                &row_point_basis,
                claimed_a_basis,
                claimed_b_basis,
                claimed_c_basis,
                counter,
            )
        })
    }

    fn derive_terminal_r1cs_sumcheck_round_challenge<F>(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        row_point: &[F],
        claimed_a_basis: &[u64],
        claimed_b_basis: &[u64],
        claimed_c_basis: &[u64],
        current_claim: &F,
        rounds: &[TerminalR1csSumcheckRound],
        round: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let row_point_basis = Self::flatten_goldilocks_basis_values(row_point);
        let current_claim_basis = Self::goldilocks_basis_u64(current_claim);
        Self::derive_terminal_r1cs_field_challenge(|counter| {
            Self::terminal_r1cs_sumcheck_round_challenge_block(
                prelude,
                assignment_commitment,
                &row_point_basis,
                claimed_a_basis,
                claimed_b_basis,
                claimed_c_basis,
                &current_claim_basis,
                rounds,
                round,
                counter,
            )
        })
    }

    fn derive_terminal_r1cs_row_product_round_challenge<F>(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        anchor_point: &[F],
        current_claim: &F,
        rounds: &[TerminalR1csRowProductRound],
        round: usize,
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let anchor_point_basis = Self::flatten_goldilocks_basis_values(anchor_point);
        let current_claim_basis = Self::goldilocks_basis_u64(current_claim);
        Self::derive_terminal_r1cs_field_challenge(|counter| {
            Self::terminal_r1cs_row_product_round_challenge_block(
                prelude,
                assignment_commitment,
                &anchor_point_basis,
                &current_claim_basis,
                rounds,
                round,
                counter,
            )
        })
    }

    fn derive_terminal_r1cs_field_challenge<F>(
        mut block: impl FnMut(u64) -> [u64; 5],
    ) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let expected = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        let mut basis = Vec::with_capacity(expected);
        let mut counter = 0u64;
        while basis.len() < expected {
            if counter > 4096 {
                return Err(NativeTerminalVerifyError::TerminalQueryDerivationLimitExceeded);
            }
            for limb in block(counter) {
                if basis.len() == expected {
                    break;
                }
                if limb < Goldilocks::ORDER_U64 {
                    basis.push(limb);
                }
            }
            counter += 1;
        }
        Self::field_from_goldilocks_basis_u64(&basis)
    }

    fn residual_fold_oracle_label(round: usize) -> String {
        format!("quadratic_residual_fold_{round}")
    }

    fn assignment_oracle_label() -> &'static str {
        "assignment"
    }

    fn assignment_fold_oracle_label(round: usize) -> String {
        format!("assignment_fold_{round}")
    }

    fn npo_validity_oracle_label() -> &'static str {
        "npo_validity"
    }

    fn npo_validity_fold_oracle_label(round: usize) -> String {
        format!("npo_validity_fold_{round}")
    }

    fn combined_validity_oracle_label() -> &'static str {
        "combined_validity"
    }

    fn combined_validity_fold_oracle_label(round: usize) -> String {
        format!("combined_validity_fold_{round}")
    }

    fn is_supported_tip5_op(op_type: &NpoTypeId) -> bool {
        op_type.as_str() == Self::tip5_op_type().as_str()
    }

    fn is_supported_non_primitive_op(op_type: &NpoTypeId) -> bool {
        Self::is_supported_tip5_op(op_type)
            || op_type.as_str() == NpoTypeId::recompose().as_str()
            || op_type.as_str() == NpoTypeId::recompose_with_coeff_lookups().as_str()
    }

    fn validate_supported_nonprimitive_layout(
        op_index: usize,
        op_type: &NpoTypeId,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
    ) -> Result<(), NativeTerminalCompileError> {
        let reason = if Self::is_supported_tip5_op(op_type) {
            Self::validate_tip5_layout(inputs, outputs)
        } else if op_type.as_str() == NpoTypeId::recompose().as_str()
            || op_type.as_str() == NpoTypeId::recompose_with_coeff_lookups().as_str()
        {
            Self::validate_recompose_layout(inputs, outputs)
        } else {
            None
        };

        if let Some(reason) = reason {
            return Err(NativeTerminalCompileError::MalformedSupportedNonPrimitive {
                op_index,
                op_type: op_type.as_str().into(),
                reason,
            });
        }
        Ok(())
    }

    fn validate_tip5_layout(
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
    ) -> Option<&'static str> {
        let config = Tip5Config::GOLDILOCKS_W16;
        let width = config.width();
        let merkle_width = config.width_ext() + 2;
        let rate = config.rate();
        if inputs.len() != width && inputs.len() != merkle_width {
            return Some("tip5 input group count must be width or width+2 merkle layout");
        }
        if outputs.len() != rate && outputs.len() != width {
            return Some("tip5 output group count must be rate or width");
        }
        if inputs.iter().take(width).any(|slot| slot.len() > 1) {
            return Some("tip5 input limbs must contain at most one witness");
        }
        if inputs.len() == merkle_width && inputs[width..].iter().any(|slot| slot.len() > 1) {
            return Some("tip5 merkle helper slots must contain at most one witness");
        }
        if outputs.iter().take(rate).any(|slot| slot.len() > 1) {
            return Some("tip5 output limbs must contain at most one witness");
        }
        None
    }

    fn validate_recompose_layout(
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
    ) -> Option<&'static str> {
        if inputs.len() != 1 {
            return Some("recompose must have exactly one input coefficient group");
        }
        if inputs[0].is_empty() {
            return Some("recompose input coefficient group must be non-empty");
        }
        if outputs.len() != 1 {
            return Some("recompose must have exactly one output group");
        }
        if outputs[0].len() != 1 {
            return Some("recompose output group must contain exactly one witness");
        }
        None
    }

    fn embed_goldilocks<F: Field + From<Goldilocks>>(value: Goldilocks) -> F {
        F::from(value)
    }

    fn verify_relation_digest_goldilocks<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let Some(expected) = verifying_key.header.relation_digest else {
            return Err(NativeTerminalVerifyError::MissingRelationDigest);
        };
        let got = Self::relation_digest_goldilocks(verifying_key);
        if got != expected {
            return Err(NativeTerminalVerifyError::RelationDigestMismatch { expected, got });
        }
        Ok(())
    }

    fn validate_terminal_proof_parameters<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        parameters: TerminalProofParameters,
    ) -> Result<(), NativeTerminalVerifyError> {
        if parameters.security_bits < MIN_TERMINAL_SECURITY_BITS {
            return Err(NativeTerminalVerifyError::TerminalProofParametersTooWeak {
                requested: parameters.security_bits,
                minimum: MIN_TERMINAL_SECURITY_BITS,
            });
        }
        if parameters.security_bits != verifying_key.header.security_bits {
            return Err(NativeTerminalVerifyError::TerminalProofParametersMismatch {
                expected: verifying_key.header.security_bits,
                got: parameters.security_bits,
            });
        }
        if parameters.query_pow_bits != 0 {
            return Err(
                NativeTerminalVerifyError::TerminalProofQueryPowUnsupported {
                    bits: parameters.query_pow_bits,
                },
            );
        }
        let johnson_bits = parameters.johnson_bits();
        if parameters.security_bits as u32 > johnson_bits {
            return Err(
                NativeTerminalVerifyError::TerminalProofParametersOverstated {
                    declared: parameters.security_bits,
                    johnson_bits,
                },
            );
        }
        Ok(())
    }

    fn validate_terminal_production_parameters(
        parameters: TerminalProofParameters,
    ) -> Result<(), NativeTerminalVerifyError> {
        let expected = TerminalProofParameters::production_60bit();
        if parameters != expected {
            return Err(
                NativeTerminalVerifyError::TerminalProofProductionParametersMismatch {
                    expected,
                    got: parameters,
                },
            );
        }
        Ok(())
    }

    pub fn validate_goldilocks_production_query_domains<F>(
        &self,
        verifying_key: &NativeTerminalVerifyingKey<F>,
        parameters: TerminalProofParameters,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        self.validate_terminal_proof_parameters(verifying_key, parameters)?;
        let num_queries = parameters.num_queries as usize;
        Self::validate_terminal_query_domain_len(
            "witness",
            verifying_key.header.fingerprint.witness_count as usize,
            num_queries,
        )?;
        Self::validate_terminal_query_domain_len(
            "primitive_constraints",
            verifying_key.inventory.total_primitive_ops(),
            num_queries,
        )?;
        let quadratic_relation = verifying_key.primitive_quadratic_relation()?;
        Self::validate_terminal_query_domain_len(
            "quadratic_residuals",
            quadratic_relation.constraints.len(),
            num_queries,
        )?;
        let npo_validity_domain_len = Self::terminal_npo_validity_domain_len::<F>(verifying_key);
        if npo_validity_domain_len > 0 {
            Self::validate_terminal_query_domain_len(
                "npo_validity_components",
                npo_validity_domain_len,
                num_queries,
            )?;
        }
        Self::validate_terminal_query_domain_len(
            "combined_validity",
            quadratic_relation.constraints.len() + npo_validity_domain_len,
            num_queries,
        )?;
        Ok(())
    }

    fn validate_terminal_query_domain_len(
        domain: &'static str,
        len: usize,
        num_queries: usize,
    ) -> Result<(), NativeTerminalVerifyError> {
        if len < num_queries {
            return Err(
                NativeTerminalVerifyError::TerminalProofQueryDomainTooSmall {
                    domain,
                    len,
                    num_queries,
                },
            );
        }
        Ok(())
    }

    fn relation_digest_goldilocks<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
    ) -> TerminalRelationDigest
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-relation-v1");
        sponge.absorb_str(&verifying_key.header.protocol_id);
        sponge.absorb_u64(verifying_key.header.version as u64);
        sponge.absorb_u64(verifying_key.header.security_bits as u64);
        sponge.absorb_u64(verifying_key.header.fingerprint.witness_count as u64);
        sponge.absorb_u64(verifying_key.header.fingerprint.public_flat_len as u64);
        sponge.absorb_u64(verifying_key.header.fingerprint.private_flat_len as u64);
        sponge.absorb_u64(verifying_key.header.fingerprint.ops_len as u64);
        Self::absorb_inventory(&mut sponge, &verifying_key.inventory);
        sponge.absorb_u64(verifying_key.constraints.len() as u64);
        for constraint in &verifying_key.constraints {
            match constraint {
                NativeTerminalConstraint::Const { out, val } => {
                    sponge.absorb_u64(1);
                    sponge.absorb_u64(out.0 as u64);
                    Self::absorb_goldilocks_basis(&mut sponge, val);
                }
                NativeTerminalConstraint::Public { out, public_pos } => {
                    sponge.absorb_u64(2);
                    sponge.absorb_u64(out.0 as u64);
                    sponge.absorb_u64(*public_pos as u64);
                }
                NativeTerminalConstraint::Alu {
                    kind,
                    a,
                    b,
                    c,
                    out,
                    intermediate_out,
                } => {
                    sponge.absorb_u64(3);
                    sponge.absorb_u64(match kind {
                        AluOpKind::Add => 1,
                        AluOpKind::Mul => 2,
                        AluOpKind::BoolCheck => 3,
                        AluOpKind::MulAdd => 4,
                        AluOpKind::HornerAcc => 5,
                    });
                    sponge.absorb_u64(a.0 as u64);
                    sponge.absorb_u64(b.0 as u64);
                    sponge.absorb_optional_witness(*c);
                    sponge.absorb_u64(out.0 as u64);
                    sponge.absorb_optional_witness(*intermediate_out);
                }
                NativeTerminalConstraint::Tip5Goldilocks {
                    op_type,
                    expected_rows,
                    callsites,
                } => {
                    sponge.absorb_u64(4);
                    sponge.absorb_str(op_type);
                    sponge.absorb_u64(*expected_rows as u64);
                    Self::absorb_npo_callsites(&mut sponge, callsites);
                }
                NativeTerminalConstraint::RecomposeGoldilocks {
                    op_type,
                    expected_rows,
                    callsites,
                } => {
                    sponge.absorb_u64(5);
                    sponge.absorb_str(op_type);
                    sponge.absorb_u64(*expected_rows as u64);
                    Self::absorb_npo_callsites(&mut sponge, callsites);
                }
            }
        }
        let backend_digest = Self::backend_relation_digest_goldilocks_for_key(verifying_key);
        sponge.absorb_str("nock-terminal-backend-relation-digest-binding-v1");
        sponge.absorb_digest(&backend_digest.0);
        TerminalRelationDigest(sponge.finalize())
    }

    fn backend_relation_digest_goldilocks_for_key<F>(
        verifying_key: &NativeTerminalVerifyingKey<F>,
    ) -> TerminalBackendRelationDigest
    where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        let mut sponge = TerminalDigestSponge::new();
        Self::absorb_backend_relation_projections(&mut sponge, verifying_key);
        TerminalBackendRelationDigest(sponge.finalize())
    }

    fn absorb_backend_relation_projections<F>(
        sponge: &mut TerminalDigestSponge,
        verifying_key: &NativeTerminalVerifyingKey<F>,
    ) where
        F: Field + BasedVectorSpace<Goldilocks>,
    {
        sponge.absorb_str("nock-terminal-backend-relation-digest-v1");
        sponge.absorb_str("nock-terminal-backend-relation-projections-v1");
        match verifying_key.primitive_quadratic_relation() {
            Ok(relation) => {
                sponge.absorb_u64(1);
                Self::absorb_quadratic_relation(sponge, &relation);
                let sparse_relation = verifying_key.sparse_r1cs_relation_from_quadratic(&relation);
                Self::absorb_sparse_r1cs_relation(sponge, &sparse_relation);
            }
            Err(_) => {
                sponge.absorb_u64(0);
            }
        }
        Self::absorb_npo_relation(sponge, &verifying_key.npo_relation());
        Self::absorb_npo_polynomial_profile(
            sponge,
            &Self::terminal_npo_polynomial_profile::<F>(verifying_key),
        );
    }

    fn absorb_quadratic_relation<F>(
        sponge: &mut TerminalDigestSponge,
        relation: &TerminalQuadraticRelation<F>,
    ) where
        F: BasedVectorSpace<Goldilocks>,
    {
        sponge.absorb_str("nock-terminal-quadratic-relation-v1");
        sponge.absorb_u64(relation.external_npo_rows as u64);
        sponge.absorb_u64(relation.constraints.len() as u64);
        for constraint in &relation.constraints {
            sponge.absorb_u64(constraint.source_constraint_index as u64);
            sponge.absorb_str(constraint.kind);
            Self::absorb_linear_expression(sponge, &constraint.a);
            Self::absorb_linear_expression(sponge, &constraint.b);
            Self::absorb_linear_expression(sponge, &constraint.c);
        }
    }

    fn absorb_linear_expression<F>(
        sponge: &mut TerminalDigestSponge,
        expression: &TerminalLinearExpression<F>,
    ) where
        F: BasedVectorSpace<Goldilocks>,
    {
        sponge.absorb_u64(expression.terms.len() as u64);
        for term in &expression.terms {
            Self::absorb_goldilocks_basis(sponge, &term.coeff);
            match term.variable {
                TerminalLinearVariable::One => {
                    sponge.absorb_u64(0);
                }
                TerminalLinearVariable::Public(public_pos) => {
                    sponge.absorb_u64(1);
                    sponge.absorb_u64(public_pos as u64);
                }
                TerminalLinearVariable::Witness(witness_id) => {
                    sponge.absorb_u64(2);
                    sponge.absorb_u64(witness_id.0 as u64);
                }
            }
        }
    }

    fn absorb_sparse_r1cs_relation<F>(
        sponge: &mut TerminalDigestSponge,
        relation: &TerminalSparseR1csRelation<F>,
    ) where
        F: BasedVectorSpace<Goldilocks>,
    {
        sponge.absorb_str("nock-terminal-sparse-r1cs-v1");
        sponge.absorb_u64(relation.rows as u64);
        sponge.absorb_u64(relation.variables as u64);
        sponge.absorb_u64(relation.public_count as u64);
        sponge.absorb_u64(relation.witness_count as u64);
        sponge.absorb_u64(relation.log_rows as u64);
        sponge.absorb_u64(relation.log_variables as u64);
        sponge.absorb_u64(relation.entries.len() as u64);
        for entry in &relation.entries {
            sponge.absorb_u64(match entry.matrix {
                TerminalSparseR1csMatrix::A => 1,
                TerminalSparseR1csMatrix::B => 2,
                TerminalSparseR1csMatrix::C => 3,
            });
            sponge.absorb_u64(entry.row as u64);
            match entry.variable {
                TerminalSparseR1csVariable::One => {
                    sponge.absorb_u64(1);
                    sponge.absorb_u64(0);
                }
                TerminalSparseR1csVariable::Public(public_pos) => {
                    sponge.absorb_u64(2);
                    sponge.absorb_u64(public_pos as u64);
                }
                TerminalSparseR1csVariable::Witness(witness_id) => {
                    sponge.absorb_u64(3);
                    sponge.absorb_u64(witness_id.0 as u64);
                }
            }
            sponge.absorb_u64(entry.variable_index as u64);
            Self::absorb_goldilocks_basis(sponge, &entry.coeff);
        }
    }

    fn absorb_npo_relation(sponge: &mut TerminalDigestSponge, relation: &TerminalNpoRelation) {
        sponge.absorb_str("nock-terminal-npo-relation-v1");
        sponge.absorb_u64(relation.rows.len() as u64);
        for row in &relation.rows {
            sponge.absorb_u64(row.npo_index as u64);
            sponge.absorb_str(&row.op_type);
            sponge.absorb_u64(row.local_row as u64);
            sponge.absorb_u64(match row.kind {
                TerminalNpoRowKind::Tip5Goldilocks => 1,
                TerminalNpoRowKind::Recompose => 2,
                TerminalNpoRowKind::RecomposeWithCoeffLookups => 3,
            });
            Self::absorb_npo_callsites(sponge, core::slice::from_ref(&row.callsite));
        }
    }

    fn absorb_npo_polynomial_profile(
        sponge: &mut TerminalDigestSponge,
        profile: &TerminalNpoPolynomialProfile,
    ) {
        sponge.absorb_str("nock-terminal-npo-polynomial-profile-v1");
        sponge.absorb_u64(profile.rows as u64);
        sponge.absorb_u64(profile.log_rows as u64);
        sponge.absorb_u64(profile.sampled_residual_components as u64);
        sponge.absorb_u64(profile.residual_components as u64);
        sponge.absorb_u64(profile.log_residual_components as u64);
        sponge.absorb_u64(profile.witness_input_slots as u64);
        sponge.absorb_u64(profile.witness_output_slots as u64);
        sponge.absorb_u64(profile.hidden_input_slots as u64);
        sponge.absorb_u64(profile.max_serialized_hidden_input_slots as u64);
        sponge.absorb_u64(profile.mmcs_direction_bits as u64);
        sponge.absorb_u64(profile.tip5_rows as u64);
        sponge.absorb_u64(profile.tip5_merkle_rows as u64);
        sponge.absorb_u64(profile.tip5_new_start_rows as u64);
        sponge.absorb_u64(profile.recompose_rows as u64);
        sponge.absorb_u64(profile.recompose_coeff_rows as u64);
        sponge.absorb_u64(profile.max_constraint_degree as u64);
        sponge.absorb_u64(profile.tip5_rounds as u64);
    }

    fn public_values_digest_goldilocks<F>(public_inputs: &[F]) -> TerminalPublicValuesDigest
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-public-values-v1");
        sponge.absorb_u64(public_inputs.len() as u64);
        for value in public_inputs {
            Self::absorb_goldilocks_basis(&mut sponge, value);
        }
        TerminalPublicValuesDigest(sponge.finalize())
    }

    fn proof_body_digest(proof_body: &[u8]) -> TerminalProofBodyDigest {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-proof-body-v1");
        sponge.absorb_u64(proof_body.len() as u64);
        for byte in proof_body {
            sponge.absorb_u64(*byte as u64);
        }
        TerminalProofBodyDigest(sponge.finalize())
    }

    fn terminal_oracle_path_len(values_len: usize) -> usize {
        values_len.next_power_of_two().trailing_zeros() as usize
    }

    fn terminal_oracle_leaf_digest<F>(
        label: &str,
        values_len: usize,
        index: usize,
        value: &F,
    ) -> TerminalCommitmentDigest
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        Self::terminal_oracle_leaf_digest_from_basis(
            label,
            values_len,
            index,
            &Self::goldilocks_basis_u64(value),
        )
    }

    fn terminal_oracle_empty_leaf_digest(
        label: &str,
        values_len: usize,
    ) -> TerminalCommitmentDigest {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-oracle-empty-leaf-v1");
        sponge.absorb_str(label);
        sponge.absorb_u64(values_len as u64);
        TerminalCommitmentDigest(sponge.finalize())
    }

    fn terminal_oracle_leaf_digest_from_basis(
        label: &str,
        values_len: usize,
        index: usize,
        basis: &[u64],
    ) -> TerminalCommitmentDigest {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-oracle-leaf-v1");
        sponge.absorb_str(label);
        sponge.absorb_u64(values_len as u64);
        sponge.absorb_u64(index as u64);
        sponge.absorb_u64(basis.len() as u64);
        for coeff in basis {
            sponge.absorb_u64(*coeff);
        }
        TerminalCommitmentDigest(sponge.finalize())
    }

    fn terminal_oracle_node_digest(
        label: &str,
        left: TerminalCommitmentDigest,
        right: TerminalCommitmentDigest,
    ) -> TerminalCommitmentDigest {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-oracle-node-v1");
        sponge.absorb_str(label);
        sponge.absorb_digest(&left.0);
        sponge.absorb_digest(&right.0);
        TerminalCommitmentDigest(sponge.finalize())
    }

    fn terminal_oracle_multi_root_goldilocks<I>(
        label: &str,
        values_len: usize,
        openings: &[TerminalOracleMultiValueOpening],
        level: usize,
        start: usize,
        frontier: &mut I,
    ) -> Result<TerminalCommitmentDigest, NativeTerminalVerifyError>
    where
        I: Iterator<Item = TerminalCommitmentDigest>,
    {
        let size = 1usize << level;
        if !openings
            .iter()
            .any(|opening| opening.index >= start && opening.index < start + size)
        {
            return frontier.next().ok_or(
                NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch {
                    expected: 1,
                    got: 0,
                },
            );
        }
        if level == 0 {
            let opening = openings
                .iter()
                .find(|opening| opening.index == start)
                .ok_or(
                    NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                        index: start,
                        values_len,
                    },
                )?;
            return Ok(Self::terminal_oracle_leaf_digest_from_basis(
                label,
                values_len,
                start,
                &opening.value_basis,
            ));
        }

        let half = size / 2;
        let left = Self::terminal_oracle_multi_root_goldilocks(
            label,
            values_len,
            openings,
            level - 1,
            start,
            frontier,
        )?;
        let right = Self::terminal_oracle_multi_root_goldilocks(
            label,
            values_len,
            openings,
            level - 1,
            start + half,
            frontier,
        )?;
        Ok(Self::terminal_oracle_node_digest(label, left, right))
    }

    fn terminal_oracle_prefix_root_goldilocks<F, I>(
        label: &str,
        values_len: usize,
        expected_values: &[F],
        prefix_len: usize,
        level: usize,
        start: usize,
        frontier: &mut I,
    ) -> Result<TerminalCommitmentDigest, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
        I: Iterator<Item = TerminalCommitmentDigest>,
    {
        let size = 1usize << level;
        if start >= prefix_len {
            return frontier.next().ok_or(
                NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch {
                    expected: 1,
                    got: 0,
                },
            );
        }
        if level == 0 {
            let value = expected_values.get(start).ok_or(
                NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                    index: start,
                    values_len,
                },
            )?;
            return Ok(Self::terminal_oracle_leaf_digest(
                label, values_len, start, value,
            ));
        }

        let half = size / 2;
        let left = Self::terminal_oracle_prefix_root_goldilocks(
            label,
            values_len,
            expected_values,
            prefix_len,
            level - 1,
            start,
            frontier,
        )?;
        let right = Self::terminal_oracle_prefix_root_goldilocks(
            label,
            values_len,
            expected_values,
            prefix_len,
            level - 1,
            start + half,
            frontier,
        )?;
        Ok(Self::terminal_oracle_node_digest(label, left, right))
    }

    fn terminal_query_block(
        prelude: &TerminalProofPrelude,
        commitment: &TerminalOracleCommitment,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-query-plan-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        sponge.absorb_str(&commitment.label);
        sponge.absorb_u64(commitment.values_len as u64);
        sponge.absorb_digest(&commitment.root.0);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_residual_fold_challenge_block(
        prelude: &TerminalProofPrelude,
        residual_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-residual-fold-challenge-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, residual_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_residual_fold_query_block(
        prelude: &TerminalProofPrelude,
        residual_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-residual-fold-query-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, residual_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_npo_validity_fold_challenge_block(
        prelude: &TerminalProofPrelude,
        validity_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-npo-validity-fold-challenge-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, validity_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_npo_validity_fold_query_block(
        prelude: &TerminalProofPrelude,
        validity_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-npo-validity-fold-query-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, validity_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_combined_validity_fold_challenge_block(
        prelude: &TerminalProofPrelude,
        combined_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-combined-validity-fold-challenge-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, combined_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_combined_validity_fold_query_block(
        prelude: &TerminalProofPrelude,
        combined_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-combined-validity-fold-query-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, combined_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_assignment_fold_challenge_block(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-assignment-fold-challenge-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, assignment_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_assignment_fold_query_block(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        fold_commitments: &[TerminalOracleCommitment],
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-assignment-fold-query-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, assignment_commitment);
        sponge.absorb_u64(fold_commitments.len() as u64);
        for commitment in fold_commitments {
            Self::absorb_terminal_oracle_commitment(&mut sponge, commitment);
        }
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_r1cs_row_challenge_block(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-sparse-r1cs-row-challenge-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, assignment_commitment);
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_r1cs_row_product_anchor_block(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-r1cs-row-product-anchor-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, assignment_commitment);
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_r1cs_batch_challenge_block(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        row_point_basis: &[u64],
        claimed_a_basis: &[u64],
        claimed_b_basis: &[u64],
        claimed_c_basis: &[u64],
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-sparse-r1cs-batch-challenge-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, assignment_commitment);
        Self::absorb_u64_slice(&mut sponge, row_point_basis);
        Self::absorb_u64_slice(&mut sponge, claimed_a_basis);
        Self::absorb_u64_slice(&mut sponge, claimed_b_basis);
        Self::absorb_u64_slice(&mut sponge, claimed_c_basis);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_r1cs_sumcheck_round_challenge_block(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        row_point_basis: &[u64],
        claimed_a_basis: &[u64],
        claimed_b_basis: &[u64],
        claimed_c_basis: &[u64],
        current_claim_basis: &[u64],
        rounds: &[TerminalR1csSumcheckRound],
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-sparse-r1cs-sumcheck-round-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, assignment_commitment);
        Self::absorb_u64_slice(&mut sponge, row_point_basis);
        Self::absorb_u64_slice(&mut sponge, claimed_a_basis);
        Self::absorb_u64_slice(&mut sponge, claimed_b_basis);
        Self::absorb_u64_slice(&mut sponge, claimed_c_basis);
        Self::absorb_u64_slice(&mut sponge, current_claim_basis);
        sponge.absorb_u64(rounds.len() as u64);
        for round_proof in rounds {
            Self::absorb_u64_slice(&mut sponge, &round_proof.eval_0_basis);
            Self::absorb_u64_slice(&mut sponge, &round_proof.eval_1_basis);
            Self::absorb_u64_slice(&mut sponge, &round_proof.eval_2_basis);
        }
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_r1cs_row_product_round_challenge_block(
        prelude: &TerminalProofPrelude,
        assignment_commitment: &TerminalOracleCommitment,
        anchor_point_basis: &[u64],
        current_claim_basis: &[u64],
        rounds: &[TerminalR1csRowProductRound],
        round: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-r1cs-row-product-round-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        Self::absorb_terminal_oracle_commitment(&mut sponge, assignment_commitment);
        Self::absorb_u64_slice(&mut sponge, anchor_point_basis);
        Self::absorb_u64_slice(&mut sponge, current_claim_basis);
        sponge.absorb_u64(rounds.len() as u64);
        for round_proof in rounds {
            Self::absorb_u64_slice(&mut sponge, &round_proof.eval_0_basis);
            Self::absorb_u64_slice(&mut sponge, &round_proof.eval_1_basis);
            Self::absorb_u64_slice(&mut sponge, &round_proof.eval_2_basis);
            Self::absorb_u64_slice(&mut sponge, &round_proof.eval_3_basis);
        }
        sponge.absorb_u64(round as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_constraint_query_block(
        prelude: &TerminalProofPrelude,
        domain_len: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-primitive-constraint-query-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        sponge.absorb_u64(domain_len as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn terminal_npo_query_block(
        prelude: &TerminalProofPrelude,
        domain_len: usize,
        counter: u64,
    ) -> [u64; 5] {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-npo-query-v1");
        sponge.absorb_digest(&prelude.challenge_digest.0);
        Self::absorb_terminal_parameters(&mut sponge, prelude.parameters);
        Self::absorb_relation_profile(&mut sponge, &prelude.relation_profile);
        sponge.absorb_digest(&prelude.public_values_digest.0);
        sponge.absorb_u64(domain_len as u64);
        sponge.absorb_u64(counter);
        sponge.finalize()
    }

    fn binding_digest(
        header: &TerminalCertificateHeader,
        proof_kind: TerminalProofKind,
        public_values_digest: TerminalPublicValuesDigest,
        proof_body_digest: TerminalProofBodyDigest,
    ) -> TerminalBindingDigest {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-certificate-binding-v1");
        Self::absorb_certificate_header(&mut sponge, header);
        Self::absorb_terminal_proof_kind(&mut sponge, proof_kind);
        sponge.absorb_digest(&public_values_digest.0);
        sponge.absorb_digest(&proof_body_digest.0);
        TerminalBindingDigest(sponge.finalize())
    }

    fn transcript_challenge_digest(
        header: &TerminalCertificateHeader,
        parameters: TerminalProofParameters,
        relation_profile: &TerminalRelationProfile,
        public_values_digest: TerminalPublicValuesDigest,
        commitments: &[TerminalCommitmentDigest],
        query_pow_nonce: u64,
    ) -> TerminalTranscriptChallengeDigest {
        let mut sponge = TerminalDigestSponge::new();
        sponge.absorb_str("nock-terminal-transcript-v1");
        Self::absorb_certificate_header(&mut sponge, header);
        Self::absorb_terminal_parameters(&mut sponge, parameters);
        Self::absorb_relation_profile(&mut sponge, relation_profile);
        sponge.absorb_digest(&public_values_digest.0);
        sponge.absorb_u64(commitments.len() as u64);
        for commitment in commitments {
            sponge.absorb_digest(&commitment.0);
        }
        sponge.absorb_u64(query_pow_nonce);
        TerminalTranscriptChallengeDigest(sponge.finalize())
    }

    fn absorb_certificate_header(
        sponge: &mut TerminalDigestSponge,
        header: &TerminalCertificateHeader,
    ) {
        sponge.absorb_u64(header.version as u64);
        sponge.absorb_str(&header.protocol_id);
        sponge.absorb_u64(header.security_bits as u64);
        sponge.absorb_u64(header.fingerprint.witness_count as u64);
        sponge.absorb_u64(header.fingerprint.public_flat_len as u64);
        sponge.absorb_u64(header.fingerprint.private_flat_len as u64);
        sponge.absorb_u64(header.fingerprint.ops_len as u64);
        match header.relation_digest {
            Some(relation_digest) => {
                sponge.absorb_u64(1);
                sponge.absorb_digest(&relation_digest.0);
            }
            None => sponge.absorb_u64(0),
        }
    }

    fn absorb_terminal_proof_kind(
        sponge: &mut TerminalDigestSponge,
        proof_kind: TerminalProofKind,
    ) {
        sponge.absorb_u64(match proof_kind {
            #[cfg(test)]
            TerminalProofKind::LocalCheckpoint => 1,
            TerminalProofKind::Production => 2,
        });
    }

    fn absorb_terminal_parameters(
        sponge: &mut TerminalDigestSponge,
        parameters: TerminalProofParameters,
    ) {
        sponge.absorb_u64(parameters.security_bits as u64);
        sponge.absorb_u64(parameters.log_blowup as u64);
        sponge.absorb_u64(parameters.num_queries as u64);
        sponge.absorb_u64(parameters.query_pow_bits as u64);
        sponge.absorb_u64(parameters.johnson_bits() as u64);
    }

    fn absorb_relation_profile(
        sponge: &mut TerminalDigestSponge,
        profile: &TerminalRelationProfile,
    ) {
        sponge.absorb_u64(profile.fingerprint.witness_count as u64);
        sponge.absorb_u64(profile.fingerprint.public_flat_len as u64);
        sponge.absorb_u64(profile.fingerprint.private_flat_len as u64);
        sponge.absorb_u64(profile.fingerprint.ops_len as u64);
        sponge.absorb_u64(profile.primitive_constraints as u64);
        sponge.absorb_u64(profile.terminal_constraints as u64);
        sponge.absorb_u64(profile.hint_ops as u64);
        sponge.absorb_u64(profile.non_primitive_ops as u64);
        sponge.absorb_u64(profile.tip5_rows as u64);
        sponge.absorb_u64(profile.recompose_rows as u64);
        sponge.absorb_u64(profile.recompose_coeff_rows as u64);
        sponge.absorb_u64(profile.external_npo_validity_components as u64);
        sponge.absorb_u64(profile.npo_callsite_input_slots as u64);
        sponge.absorb_u64(profile.npo_callsite_output_slots as u64);
    }

    fn absorb_terminal_oracle_commitment(
        sponge: &mut TerminalDigestSponge,
        commitment: &TerminalOracleCommitment,
    ) {
        sponge.absorb_str(&commitment.label);
        sponge.absorb_u64(commitment.values_len as u64);
        sponge.absorb_digest(&commitment.root.0);
    }

    fn absorb_inventory(sponge: &mut TerminalDigestSponge, inventory: &TerminalOpInventory) {
        sponge.absorb_u64(inventory.const_ops as u64);
        sponge.absorb_u64(inventory.public_ops as u64);
        sponge.absorb_u64(inventory.alu_add_ops as u64);
        sponge.absorb_u64(inventory.alu_mul_ops as u64);
        sponge.absorb_u64(inventory.alu_bool_check_ops as u64);
        sponge.absorb_u64(inventory.alu_mul_add_ops as u64);
        sponge.absorb_u64(inventory.alu_horner_acc_ops as u64);
        sponge.absorb_u64(inventory.hint_ops as u64);
        sponge.absorb_u64(inventory.non_primitive_ops as u64);
        sponge.absorb_u64(inventory.non_primitive_types.len() as u64);
        for op_type in &inventory.non_primitive_types {
            sponge.absorb_str(op_type);
        }
    }

    fn absorb_npo_callsites(
        sponge: &mut TerminalDigestSponge,
        callsites: &[NativeTerminalNpoCallsite],
    ) {
        sponge.absorb_u64(callsites.len() as u64);
        for callsite in callsites {
            match callsite.tip5_mode {
                Some(mode) => {
                    sponge.absorb_u64(1);
                    sponge.absorb_u64(mode.new_start as u64);
                    sponge.absorb_u64(mode.merkle_path as u64);
                    sponge.absorb_optional_witness(callsite.tip5_mmcs_bit);
                }
                None => {
                    sponge.absorb_u64(0);
                    sponge.absorb_optional_witness(None);
                }
            }
            sponge.absorb_u64(callsite.inputs.len() as u64);
            for input in &callsite.inputs {
                sponge.absorb_optional_witness(*input);
            }
            sponge.absorb_u64(callsite.outputs.len() as u64);
            for output in &callsite.outputs {
                sponge.absorb_optional_witness(*output);
            }
        }
    }

    fn absorb_goldilocks_basis<F>(sponge: &mut TerminalDigestSponge, value: &F)
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let coeffs = value.as_basis_coefficients_slice();
        sponge.absorb_u64(coeffs.len() as u64);
        for coeff in coeffs {
            sponge.absorb_u64(PrimeField64::as_canonical_u64(coeff));
        }
    }

    fn absorb_u64_slice(sponge: &mut TerminalDigestSponge, values: &[u64]) {
        sponge.absorb_u64(values.len() as u64);
        for value in values {
            sponge.absorb_u64(*value);
        }
    }

    fn goldilocks_basis_u64<F>(value: &F) -> Vec<u64>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        value
            .as_basis_coefficients_slice()
            .iter()
            .map(PrimeField64::as_canonical_u64)
            .collect()
    }

    fn field_from_goldilocks_basis_u64<F>(basis: &[u64]) -> Result<F, NativeTerminalVerifyError>
    where
        F: BasedVectorSpace<Goldilocks>,
    {
        let expected = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        if basis.len() != expected {
            return Err(
                NativeTerminalVerifyError::TerminalOracleOpeningValueDimensionMismatch {
                    expected,
                    got: basis.len(),
                },
            );
        }
        let mut coeffs = Vec::with_capacity(basis.len());
        for (limb, value) in basis.iter().copied().enumerate() {
            if value >= Goldilocks::ORDER_U64 {
                return Err(
                    NativeTerminalVerifyError::TerminalOracleOpeningValueNonCanonical {
                        limb,
                        value,
                    },
                );
            }
            coeffs.push(Goldilocks::from_u64(value));
        }
        <F as BasedVectorSpace<Goldilocks>>::from_basis_coefficients_slice(&coeffs).ok_or(
            NativeTerminalVerifyError::TerminalOracleOpeningValueDimensionMismatch {
                expected,
                got: basis.len(),
            },
        )
    }

    fn verify_tip5_callsite(
        op_type: &str,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        operation: &Tip5CircuitRow<Goldilocks>,
    ) -> Result<(), NativeTerminalVerifyError> {
        let Some(mode) = callsite.tip5_mode else {
            return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                op_type: op_type.into(),
                row,
                field: "tip5_mode",
                limb: 0,
                expected: Some(1),
                got: None,
            });
        };
        if operation.new_start != mode.new_start {
            return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                op_type: op_type.into(),
                row,
                field: "new_start",
                limb: 0,
                expected: Some(mode.new_start as u32),
                got: Some(operation.new_start as u32),
            });
        }
        Self::verify_ctl_callsite(
            op_type,
            row,
            "input_indices",
            &callsite.inputs,
            &operation.in_ctl,
            &operation.input_indices,
        )?;
        Self::verify_ctl_callsite(
            op_type,
            row,
            "output_indices",
            &callsite.outputs,
            &operation.out_ctl,
            &operation.output_indices,
        )
    }

    fn verify_recompose_callsite(
        op_type: &str,
        row: usize,
        callsite: &NativeTerminalNpoCallsite,
        operation: &RecomposeCircuitRow<Goldilocks>,
    ) -> Result<(), NativeTerminalVerifyError> {
        if callsite.inputs.len() != operation.input_wids.len() {
            return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                op_type: op_type.into(),
                row,
                field: "input_wids_len",
                limb: 0,
                expected: Some(callsite.inputs.len() as u32),
                got: Some(operation.input_wids.len() as u32),
            });
        }
        for (limb, expected) in callsite.inputs.iter().copied().enumerate() {
            let got = operation.input_wids.get(limb).map(|wid| wid.0);
            if expected.map(|wid| wid.0) != got {
                return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                    op_type: op_type.into(),
                    row,
                    field: "input_wids",
                    limb,
                    expected: expected.map(|wid| wid.0),
                    got,
                });
            }
        }
        let expected = callsite.outputs.first().copied().flatten().map(|wid| wid.0);
        let got = Some(operation.output_wid.0);
        if expected != got {
            return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                op_type: op_type.into(),
                row,
                field: "output_wid",
                limb: 0,
                expected,
                got,
            });
        }
        Ok(())
    }

    fn verify_ctl_callsite(
        op_type: &str,
        row: usize,
        field: &'static str,
        expected: &[Option<WitnessId>],
        got_ctl: &[bool],
        got_indices: &[u32],
    ) -> Result<(), NativeTerminalVerifyError> {
        if expected.len() != got_ctl.len() || expected.len() != got_indices.len() {
            return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                op_type: op_type.into(),
                row,
                field,
                limb: expected.len(),
                expected: Some(expected.len() as u32),
                got: Some(got_ctl.len().max(got_indices.len()) as u32),
            });
        }
        for (limb, expected_wid) in expected.iter().copied().enumerate() {
            let got = got_ctl[limb].then_some(got_indices[limb]);
            let expected = expected_wid.map(|wid| wid.0);
            if expected != got {
                return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                    op_type: op_type.into(),
                    row,
                    field,
                    limb,
                    expected,
                    got,
                });
            }
            if expected.is_none() && got_indices[limb] != 0 {
                return Err(NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                    op_type: op_type.into(),
                    row,
                    field,
                    limb,
                    expected: None,
                    got: Some(got_indices[limb]),
                });
            }
        }
        Ok(())
    }

    fn verify_tip5_goldilocks_assignment<F>(
        &self,
        op_type: &str,
        expected_rows: usize,
        callsites: &[NativeTerminalNpoCallsite],
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let trace = witness
            .traces
            .non_primitive_traces
            .get(&NpoTypeId::new(op_type))
            .and_then(|trace| trace.as_any().downcast_ref::<Tip5Trace<Goldilocks>>())
            .ok_or_else(|| NativeTerminalVerifyError::MissingNonPrimitiveTrace {
                op_type: op_type.into(),
            })?;
        if trace.operations.len() != expected_rows {
            return Err(NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                op_type: op_type.into(),
                expected: expected_rows,
                got: trace.operations.len(),
            });
        }
        if callsites.len() != expected_rows {
            return Err(NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                op_type: op_type.into(),
                expected: expected_rows,
                got: callsites.len(),
            });
        }

        for (row, operation) in trace.operations.iter().enumerate() {
            Self::verify_tip5_callsite(op_type, row, &callsites[row], operation)?;
            if operation.input_values.len() != 16 {
                return Err(NativeTerminalVerifyError::Tip5TraceInputLength {
                    row,
                    got: operation.input_values.len(),
                });
            }
            if operation.in_ctl.len() != 16 {
                return Err(NativeTerminalVerifyError::Tip5TraceCtlLength {
                    row,
                    field: "in_ctl",
                    got: operation.in_ctl.len(),
                });
            }
            if operation.input_indices.len() != 16 {
                return Err(NativeTerminalVerifyError::Tip5TraceCtlLength {
                    row,
                    field: "input_indices",
                    got: operation.input_indices.len(),
                });
            }
            if operation.out_ctl.len() != 10 {
                return Err(NativeTerminalVerifyError::Tip5TraceCtlLength {
                    row,
                    field: "out_ctl",
                    got: operation.out_ctl.len(),
                });
            }
            if operation.output_indices.len() != 10 {
                return Err(NativeTerminalVerifyError::Tip5TraceCtlLength {
                    row,
                    field: "output_indices",
                    got: operation.output_indices.len(),
                });
            }

            for limb in 0..16 {
                if operation.in_ctl[limb] {
                    let witness_id = WitnessId(operation.input_indices[limb]);
                    let witness_value = witness
                        .traces
                        .witness_trace
                        .get_value(witness_id)
                        .copied()
                        .ok_or(NativeTerminalVerifyError::MissingWitness {
                            witness_id: witness_id.0,
                        })?;
                    if witness_value != Self::embed_goldilocks(operation.input_values[limb]) {
                        return Err(NativeTerminalVerifyError::Tip5InputMismatch { row, limb });
                    }
                }
            }

            let mut state: [Goldilocks; 16] = core::array::from_fn(|i| operation.input_values[i]);
            Tip5Perm.permute_mut(&mut state);

            for (limb, expected) in state.iter().copied().enumerate().take(10) {
                if operation.out_ctl[limb] {
                    let witness_id = WitnessId(operation.output_indices[limb]);
                    let witness_value = witness
                        .traces
                        .witness_trace
                        .get_value(witness_id)
                        .copied()
                        .ok_or(NativeTerminalVerifyError::MissingWitness {
                            witness_id: witness_id.0,
                        })?;
                    if witness_value != Self::embed_goldilocks(expected) {
                        return Err(NativeTerminalVerifyError::Tip5OutputMismatch { row, limb });
                    }
                }
            }
        }

        Ok(())
    }

    fn verify_recompose_goldilocks_assignment<F>(
        &self,
        op_type: &str,
        expected_rows: usize,
        callsites: &[NativeTerminalNpoCallsite],
        witness: &TerminalWitness<F>,
    ) -> Result<(), NativeTerminalVerifyError>
    where
        F: Field + BasedVectorSpace<Goldilocks> + From<Goldilocks>,
    {
        let trace = witness
            .traces
            .non_primitive_traces
            .get(&NpoTypeId::new(op_type))
            .and_then(|trace| trace.as_any().downcast_ref::<RecomposeTrace<Goldilocks>>())
            .ok_or_else(|| NativeTerminalVerifyError::MissingNonPrimitiveTrace {
                op_type: op_type.into(),
            })?;
        if trace.operations.len() != expected_rows {
            return Err(NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                op_type: op_type.into(),
                expected: expected_rows,
                got: trace.operations.len(),
            });
        }
        if callsites.len() != expected_rows {
            return Err(NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                op_type: op_type.into(),
                expected: expected_rows,
                got: callsites.len(),
            });
        }

        let expected_kind = if op_type == NpoTypeId::recompose().as_str() {
            RecomposeTraceKind::Standard
        } else if op_type == NpoTypeId::recompose_with_coeff_lookups().as_str() {
            RecomposeTraceKind::WithCoeffLookups
        } else {
            return Err(NativeTerminalVerifyError::MissingNonPrimitiveTrace {
                op_type: op_type.into(),
            });
        };
        if trace.kind != expected_kind {
            return Err(NativeTerminalVerifyError::RecomposeTraceKindMismatch {
                op_type: op_type.into(),
            });
        }

        let expected_d = <F as BasedVectorSpace<Goldilocks>>::DIMENSION;
        for (row, operation) in trace.operations.iter().enumerate() {
            Self::verify_recompose_callsite(op_type, row, &callsites[row], operation)?;
            if operation.values.len() != expected_d {
                return Err(NativeTerminalVerifyError::RecomposeTraceValueLength {
                    row,
                    expected: expected_d,
                    got: operation.values.len(),
                });
            }
            if operation.input_wids.len() != expected_d {
                return Err(NativeTerminalVerifyError::RecomposeTraceInputLength {
                    row,
                    expected: expected_d,
                    got: operation.input_wids.len(),
                });
            }

            for (limb, (witness_id, value)) in operation
                .input_wids
                .iter()
                .copied()
                .zip(operation.values.iter().copied())
                .enumerate()
            {
                let witness_value = witness
                    .traces
                    .witness_trace
                    .get_value(witness_id)
                    .copied()
                    .ok_or(NativeTerminalVerifyError::MissingWitness {
                        witness_id: witness_id.0,
                    })?;
                if witness_value != Self::embed_goldilocks(value) {
                    return Err(NativeTerminalVerifyError::RecomposeInputMismatch { row, limb });
                }
            }

            let expected_output =
                <F as BasedVectorSpace<Goldilocks>>::from_basis_coefficients_slice(
                    &operation.values,
                )
                .ok_or(NativeTerminalVerifyError::RecomposeTraceValueLength {
                    row,
                    expected: expected_d,
                    got: operation.values.len(),
                })?;
            let output_value = witness
                .traces
                .witness_trace
                .get_value(operation.output_wid)
                .copied()
                .ok_or(NativeTerminalVerifyError::MissingWitness {
                    witness_id: operation.output_wid.0,
                })?;
            if output_value != expected_output {
                return Err(NativeTerminalVerifyError::RecomposeOutputMismatch { row });
            }
        }

        Ok(())
    }
}

struct TerminalDigestSponge {
    state: [Goldilocks; 16],
    pos: usize,
}

impl TerminalDigestSponge {
    const RATE: usize = 10;

    fn new() -> Self {
        Self {
            state: [Goldilocks::ZERO; 16],
            pos: 0,
        }
    }

    fn absorb_u64(&mut self, value: u64) {
        if self.pos == Self::RATE {
            Tip5Perm.permute_mut(&mut self.state);
            self.pos = 0;
        }
        self.state[self.pos] += Goldilocks::from_u64(value);
        self.pos += 1;
    }

    fn absorb_optional_witness(&mut self, witness_id: Option<WitnessId>) {
        match witness_id {
            Some(witness_id) => {
                self.absorb_u64(1);
                self.absorb_u64(witness_id.0 as u64);
            }
            None => self.absorb_u64(0),
        }
    }

    fn absorb_digest(&mut self, digest: &[u64; 5]) {
        for limb in digest {
            self.absorb_u64(*limb);
        }
    }

    fn absorb_str(&mut self, value: &str) {
        self.absorb_u64(value.len() as u64);
        for byte in value.as_bytes() {
            self.absorb_u64(*byte as u64);
        }
    }

    fn finalize(mut self) -> [u64; 5] {
        self.absorb_u64(1);
        Tip5Perm.permute_mut(&mut self.state);
        core::array::from_fn(|i| PrimeField64::as_canonical_u64(&self.state[i]))
    }
}

#[cfg(test)]
mod tests {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec;
    use core::any::Any;

    use hashbrown::HashMap;
    use p3_baby_bear::BabyBear;
    use p3_circuit::ops::tip5_perm::Tip5PermCallMmcs;
    use p3_circuit::ops::{
        ExecutionContext, NonPrimitiveExecutor, NpoTypeId, Tip5Goldilocks, Tip5PermCall,
        Tip5PermPrivateData, generate_recompose_trace, generate_tip5_trace,
    };
    use p3_circuit::tables::WitnessTrace;
    use p3_circuit::{
        Circuit, CircuitBuilder, CircuitError, ExprId, NonPrimitiveOpId, NpoPrivateData, WitnessId,
    };
    use p3_field::PrimeCharacteristicRing;
    use p3_field::extension::BinomialExtensionField;
    use p3_tip5_circuit_air::NUM_ROUNDS;

    use super::*;

    type GoldilocksD2 = BinomialExtensionField<Goldilocks, 2>;

    #[test]
    fn primitive_terminal_compile_succeeds_and_counts_ops() {
        let mut builder = CircuitBuilder::<BabyBear>::new();
        let x = builder.public_input();
        let y = builder.public_input();
        let expected = builder.public_input();
        let c5 = builder.define_const(BabyBear::from_u64(5));
        let mul = builder.mul(x, y);
        let sum = builder.add(x, mul);
        let sum = builder.add(sum, c5);
        let diff = builder.sub(sum, expected);
        builder.assert_zero(diff);
        let circuit = builder.build().unwrap();

        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();

        assert_eq!(pk.header, vk.header);
        assert_eq!(pk.header.security_bits, 60);
        assert_eq!(pk.inventory.non_primitive_ops, 0);
        assert_eq!(pk.inventory.public_ops, 3);
        assert!(pk.inventory.const_ops >= 1);
        assert!(pk.inventory.alu_add_ops >= 1);
        assert!(pk.inventory.alu_mul_ops + pk.inventory.alu_mul_add_ops >= 1);
        assert!(pk.inventory.total_primitive_ops() >= pk.inventory.public_ops);
        assert_eq!(
            pk.header.fingerprint,
            TerminalCircuitFingerprint::from_circuit(&circuit)
        );
    }

    #[test]
    fn primitive_terminal_rejects_low_soundness_profile() {
        let circuit = Circuit::<BabyBear>::new(0, HashMap::new());
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 59);
        let err = compiler.compile_primitive_terminal(&circuit).unwrap_err();
        assert_eq!(
            err,
            NativeTerminalCompileError::SecurityBitsTooLow {
                requested: 59,
                minimum: MIN_TERMINAL_SECURITY_BITS,
            }
        );
    }

    #[test]
    fn primitive_terminal_rejects_non_primitive_ops_until_ported() {
        let mut circuit = Circuit::<BabyBear>::new(0, HashMap::new());
        circuit.ops.push(Op::NonPrimitiveOpWithExecutor {
            inputs: vec![],
            outputs: vec![],
            executor: Box::new(DummyNpo {
                op_type: NpoTypeId::new("tip5_perm/goldilocks"),
            }),
            op_id: NonPrimitiveOpId(0),
        });

        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let inventory = compiler.analyze(&circuit);
        assert_eq!(inventory.non_primitive_ops, 1);
        assert_eq!(
            inventory.non_primitive_types,
            vec![String::from("tip5_perm/goldilocks")]
        );

        let err = compiler.compile_primitive_terminal(&circuit).unwrap_err();
        assert_eq!(
            err,
            NativeTerminalCompileError::UnsupportedNonPrimitive {
                op_index: 0,
                op_type: String::from("tip5_perm/goldilocks"),
            }
        );
    }

    #[test]
    fn goldilocks_terminal_rejects_malformed_supported_tip5_layout() {
        let (mut circuit, _public_inputs) = build_tip5_test_circuit();
        let (op_index, op) = circuit
            .ops
            .iter_mut()
            .enumerate()
            .find(|(_, op)| {
                matches!(
                    op,
                    Op::NonPrimitiveOpWithExecutor { executor, .. }
                        if NativeTerminalCompiler::is_supported_tip5_op(executor.op_type())
                )
            })
            .expect("Tip5 NPO op must be present");
        let Op::NonPrimitiveOpWithExecutor { inputs, .. } = op else {
            unreachable!("matched Tip5 NPO op");
        };
        inputs.pop();

        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let err = compiler.compile_goldilocks_terminal(&circuit).unwrap_err();
        assert_eq!(
            err,
            NativeTerminalCompileError::MalformedSupportedNonPrimitive {
                op_index,
                op_type: NpoTypeId::tip5_perm(Tip5Config::GOLDILOCKS_W16)
                    .as_str()
                    .into(),
                reason: "tip5 input group count must be width or width+2 merkle layout",
            }
        );
    }

    #[test]
    fn goldilocks_terminal_rejects_malformed_supported_recompose_layout() {
        let (mut circuit, _public_inputs) = build_recompose_test_circuit();
        let (op_index, op) = circuit
            .ops
            .iter_mut()
            .enumerate()
            .find(|(_, op)| {
                matches!(
                    op,
                    Op::NonPrimitiveOpWithExecutor { executor, .. }
                        if executor.op_type().as_str() == NpoTypeId::recompose().as_str()
                )
            })
            .expect("standard recompose NPO op must be present");
        let Op::NonPrimitiveOpWithExecutor { outputs, .. } = op else {
            unreachable!("matched recompose NPO op");
        };
        outputs.clear();

        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let err = compiler.compile_goldilocks_terminal(&circuit).unwrap_err();
        assert_eq!(
            err,
            NativeTerminalCompileError::MalformedSupportedNonPrimitive {
                op_index,
                op_type: NpoTypeId::recompose().as_str().into(),
                reason: "recompose must have exactly one output group",
            }
        );
    }

    #[test]
    fn primitive_terminal_verifies_executed_assignment() {
        let (circuit, public_inputs, expected_public_len) = build_primitive_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let witness = execute_terminal_witness(&circuit, public_inputs);

        assert_eq!(vk.constraints.len(), vk.inventory.total_primitive_ops());
        assert_eq!(witness.public_inputs.len(), expected_public_len);
        compiler
            .verify_primitive_assignment(&vk, &witness)
            .expect("honest primitive assignment must verify");
    }

    #[test]
    fn primitive_quadratic_relation_verifies_executed_assignment() {
        let (circuit, public_inputs, _) = build_primitive_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let witness = execute_terminal_witness(&circuit, public_inputs);
        let relation = vk
            .primitive_quadratic_relation()
            .expect("primitive constraints must lower to quadratic relation");

        assert_eq!(relation.external_npo_rows, 0);
        assert!(relation.constraints.len() >= vk.inventory.total_primitive_ops());
        relation
            .verify(&witness.public_inputs, &witness)
            .expect("honest primitive witness must satisfy quadratic relation");
    }

    #[test]
    fn primitive_sparse_r1cs_relation_indexes_assignment_vector() {
        let (circuit, _public_inputs, _) = build_primitive_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let quadratic_relation = vk
            .primitive_quadratic_relation()
            .expect("primitive constraints must lower to quadratic relation");
        let sparse_relation = vk
            .primitive_sparse_r1cs_relation()
            .expect("primitive constraints must lower to sparse R1CS");
        let expected_variables = 1
            + vk.header.fingerprint.public_flat_len
            + vk.header.fingerprint.witness_count as usize;

        assert_eq!(sparse_relation.rows, quadratic_relation.constraints.len());
        assert_eq!(sparse_relation.variables, expected_variables);
        assert_eq!(sparse_relation.public_count, 3);
        assert_eq!(
            sparse_relation.witness_count,
            vk.header.fingerprint.witness_count as usize
        );
        assert_eq!(
            sparse_relation.log_rows,
            sparse_relation.rows.next_power_of_two().trailing_zeros() as usize
        );
        assert_eq!(
            sparse_relation.log_variables,
            sparse_relation
                .variables
                .next_power_of_two()
                .trailing_zeros() as usize
        );
        assert!(sparse_relation.entries.iter().any(|entry| {
            matches!(entry.variable, TerminalSparseR1csVariable::One) && entry.variable_index == 0
        }));
        assert!(sparse_relation.entries.iter().any(|entry| {
            matches!(entry.variable, TerminalSparseR1csVariable::Public(0))
                && entry.variable_index == 1
        }));
        assert!(sparse_relation.entries.iter().any(|entry| {
            matches!(entry.variable, TerminalSparseR1csVariable::Witness(witness_id)
                if entry.variable_index
                    == 1 + sparse_relation.public_count + witness_id.0 as usize)
        }));
    }

    #[test]
    fn primitive_quadratic_relation_rejects_bad_witness_value() {
        let (circuit, public_inputs, _) = build_primitive_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let mut witness = execute_terminal_witness(&circuit, public_inputs);
        let relation = vk
            .primitive_quadratic_relation()
            .expect("primitive constraints must lower to quadratic relation");

        let mut values = witness_values(&witness);
        values[0] += BabyBear::ONE;
        witness.traces.witness_trace = WitnessTrace::new(values);
        let err = relation
            .verify(&witness.public_inputs, &witness)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalQuadraticConstraintViolation { .. }
        ));
    }

    #[test]
    fn primitive_terminal_rejects_public_input_swap() {
        let (circuit, public_inputs, _) = build_primitive_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let mut witness = execute_terminal_witness(&circuit, public_inputs);

        witness.public_inputs[0] += BabyBear::ONE;
        let err = compiler
            .verify_primitive_assignment(&vk, &witness)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::PublicInputMismatch { public_pos: 0 }
        );
    }

    #[test]
    fn primitive_terminal_rejects_private_input_length_mismatch() {
        let (circuit, public_inputs, private_inputs) = build_private_input_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let mut witness =
            execute_terminal_witness_with_private(&circuit, public_inputs, private_inputs);

        witness.private_inputs.clear();
        let err = compiler
            .verify_primitive_assignment(&vk, &witness)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::PrivateInputLengthMismatch {
                expected: 1,
                got: 0,
            }
        );
    }

    #[test]
    fn primitive_terminal_rejects_witness_trace_length_mismatch() {
        let (circuit, public_inputs, _) = build_primitive_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let mut witness = execute_terminal_witness(&circuit, public_inputs);

        witness.traces.witness_trace = WitnessTrace::new(vec![BabyBear::ZERO]);
        let err = compiler
            .verify_primitive_assignment(&vk, &witness)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::WitnessTraceLengthMismatch {
                expected: vk.header.fingerprint.witness_count as usize,
                got: 1,
            }
        );
    }

    #[test]
    fn primitive_terminal_rejects_witness_trace_index_mismatch() {
        let (circuit, public_inputs, _) = build_primitive_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let mut witness = execute_terminal_witness(&circuit, public_inputs);

        witness.traces.witness_trace.index[0] = WitnessId(99);
        let err = compiler
            .verify_primitive_assignment(&vk, &witness)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::WitnessTraceIndexMismatch {
                row: 0,
                expected: 0,
                got: 99,
            }
        );
    }

    #[test]
    fn goldilocks_relation_digest_binds_constraint_values() {
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk_a, vk_a) = compiler
            .compile_goldilocks_terminal(&build_goldilocks_const_circuit(5))
            .unwrap();
        let (_pk_b, vk_b) = compiler
            .compile_goldilocks_terminal(&build_goldilocks_const_circuit(6))
            .unwrap();

        let digest_a = vk_a.header.relation_digest.expect("digest A");
        let digest_b = vk_b.header.relation_digest.expect("digest B");
        assert_ne!(digest_a, digest_b);
        assert_eq!(vk_a.header.fingerprint, vk_b.header.fingerprint);
    }

    #[test]
    fn goldilocks_backend_relation_digest_binds_quadratic_projection() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, mut vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);

        let backend_digest = compiler.backend_relation_digest_goldilocks(&vk);
        let relation_digest = NativeTerminalCompiler::relation_digest_goldilocks(&vk);
        let NativeTerminalConstraint::Const { val, .. } = vk
            .constraints
            .iter_mut()
            .find(|constraint| matches!(constraint, NativeTerminalConstraint::Const { .. }))
            .expect("const constraint must be present")
        else {
            unreachable!("matched const constraint");
        };
        *val += Goldilocks::ONE;

        assert_ne!(
            backend_digest,
            compiler.backend_relation_digest_goldilocks(&vk),
            "backend digest must bind the quadratic const equation"
        );
        assert_ne!(
            relation_digest,
            NativeTerminalCompiler::relation_digest_goldilocks(&vk),
            "relation digest must absorb the backend projection digest"
        );
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&vk, &witness)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::RelationDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_backend_relation_digest_binds_npo_projection() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, mut vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);

        let backend_digest = compiler.backend_relation_digest_goldilocks(&vk);
        let relation_digest = NativeTerminalCompiler::relation_digest_goldilocks(&vk);
        let NativeTerminalConstraint::Tip5Goldilocks { callsites, .. } = vk
            .constraints
            .iter_mut()
            .find(|constraint| {
                matches!(
                    constraint,
                    NativeTerminalConstraint::Tip5Goldilocks { callsites, .. }
                        if callsites.len() >= 2
                )
            })
            .expect("two-row Tip5 constraint must be present")
        else {
            unreachable!("matched Tip5 constraint");
        };
        callsites.swap(0, 1);

        assert_ne!(
            backend_digest,
            compiler.backend_relation_digest_goldilocks(&vk),
            "backend digest must bind NPO row order and callsites"
        );
        assert_ne!(
            relation_digest,
            NativeTerminalCompiler::relation_digest_goldilocks(&vk),
            "relation digest must absorb the NPO projection digest"
        );
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&vk, &witness)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::RelationDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_backend_relation_digest_binds_tip5_row_mode() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, mut vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);

        let backend_digest = compiler.backend_relation_digest_goldilocks(&vk);
        let relation_digest = NativeTerminalCompiler::relation_digest_goldilocks(&vk);
        let NativeTerminalConstraint::Tip5Goldilocks { callsites, .. } = vk
            .constraints
            .iter_mut()
            .find(|constraint| {
                matches!(constraint, NativeTerminalConstraint::Tip5Goldilocks { .. })
            })
            .expect("Tip5 constraint must be present")
        else {
            unreachable!("matched Tip5 constraint");
        };
        let mode = callsites[0]
            .tip5_mode
            .as_mut()
            .expect("Tip5 callsite must carry terminal mode");
        mode.new_start = !mode.new_start;

        assert_ne!(
            backend_digest,
            compiler.backend_relation_digest_goldilocks(&vk),
            "backend digest must bind Tip5 new_start mode"
        );
        assert_ne!(
            relation_digest,
            NativeTerminalCompiler::relation_digest_goldilocks(&vk),
            "relation digest must absorb Tip5 row mode"
        );
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&vk, &witness)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::RelationDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_npo_polynomial_profile_tracks_supported_table_shape() {
        let (circuit, _public_inputs, _private_data) = build_many_merkle_tip5_test_circuit(2);
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let npo_relation = vk.npo_relation();
        let profile = NativeTerminalCompiler::terminal_npo_polynomial_profile::<Goldilocks>(&vk);

        assert_eq!(profile.rows, npo_relation.rows.len());
        assert_eq!(profile.log_rows, 1);
        assert_eq!(
            profile.sampled_residual_components,
            NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
        );
        assert_eq!(profile.sampled_residual_components, 20);
        assert_eq!(profile.residual_components, 34);
        assert_eq!(
            profile.log_residual_components,
            NativeTerminalCompiler::terminal_mle_log_size(profile.residual_components)
        );
        assert_eq!(profile.tip5_rows, 2);
        assert_eq!(profile.tip5_merkle_rows, 2);
        assert_eq!(profile.tip5_new_start_rows, 2);
        assert_eq!(profile.recompose_rows, 0);
        assert_eq!(profile.recompose_coeff_rows, 0);
        assert_eq!(profile.witness_input_slots, 10);
        assert_eq!(profile.witness_output_slots, 10);
        assert_eq!(profile.hidden_input_slots, 22);
        assert_eq!(profile.max_serialized_hidden_input_slots, 10);
        assert_eq!(profile.mmcs_direction_bits, 2);
        assert_eq!(profile.max_constraint_degree, 4);
        assert_eq!(profile.tip5_rounds, NUM_ROUNDS);
    }

    #[test]
    fn goldilocks_npo_exhaustive_residual_values_match_polynomial_profile() {
        let (circuit, public_inputs, private_data) = build_many_merkle_tip5_test_circuit(2);
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness =
            execute_tip5_terminal_witness_with_private_data(&circuit, public_inputs, private_data);
        let profile = NativeTerminalCompiler::terminal_npo_polynomial_profile::<Goldilocks>(&vk);

        let sampled_values = compiler
            .terminal_npo_validity_values_goldilocks(&vk, &witness)
            .expect("sampled NPO validity values must compute");
        let exhaustive_values = compiler
            .terminal_npo_exhaustive_residual_values_goldilocks(&vk, &witness)
            .expect("exhaustive NPO residual values must compute");
        assert_eq!(sampled_values.len(), profile.sampled_residual_components);
        assert_eq!(exhaustive_values.len(), profile.residual_components);
        assert!(
            sampled_values
                .iter()
                .all(|value| *value == Goldilocks::ZERO)
        );
        assert!(
            exhaustive_values
                .iter()
                .all(|value| *value == Goldilocks::ZERO)
        );

        let commitment = compiler
            .commit_terminal_npo_exhaustive_residuals_goldilocks(&vk, &witness)
            .expect("exhaustive NPO residual oracle must commit")
            .commitment();
        assert_eq!(
            commitment.label,
            NativeTerminalCompiler::npo_exhaustive_residual_oracle_label()
        );
        assert_eq!(commitment.values_len, profile.residual_components);
    }

    #[test]
    fn goldilocks_npo_exhaustive_residual_values_include_mmcs_booleanity() {
        let (circuit, public_inputs, private_data) = build_many_merkle_tip5_test_circuit(1);
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness =
            execute_tip5_terminal_witness_with_private_data(&circuit, public_inputs, private_data);
        let mmcs_bit = vk
            .npo_relation()
            .rows
            .iter()
            .find_map(|row| row.callsite.tip5_mmcs_bit)
            .expect("Merkle Tip5 row must bind an MMCS direction bit");
        let mut bad_witness_values = witness_values(&witness);
        bad_witness_values[mmcs_bit.0 as usize] = Goldilocks::from_u64(2);
        let mut bad_traces = witness.traces.clone();
        bad_traces.witness_trace = WitnessTrace::new(bad_witness_values);
        let bad_witness = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: bad_traces,
        };

        let sampled_values = compiler
            .terminal_npo_validity_values_goldilocks(&vk, &bad_witness)
            .expect("sampled NPO validity values must compute");
        assert!(
            sampled_values
                .iter()
                .all(|value| *value == Goldilocks::ZERO),
            "legacy sampled validity does not cover MMCS-bit booleanity"
        );

        let exhaustive_values = compiler
            .terminal_npo_exhaustive_residual_values_goldilocks(&vk, &bad_witness)
            .expect("exhaustive NPO residual values must compute");
        let nonzero_values = exhaustive_values
            .iter()
            .filter(|value| **value != Goldilocks::ZERO)
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(nonzero_values, vec![Goldilocks::from_u64(2)]);
    }

    #[test]
    fn goldilocks_backend_relation_digest_binds_npo_polynomial_profile() {
        let (circuit, _public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, mut vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();

        let profile = NativeTerminalCompiler::terminal_npo_polynomial_profile::<Goldilocks>(&vk);
        let backend_digest = compiler.backend_relation_digest_goldilocks(&vk);
        let NativeTerminalConstraint::Tip5Goldilocks { callsites, .. } = vk
            .constraints
            .iter_mut()
            .find(|constraint| {
                matches!(constraint, NativeTerminalConstraint::Tip5Goldilocks { .. })
            })
            .expect("Tip5 constraint must be present")
        else {
            unreachable!("matched Tip5 constraint");
        };
        let mode = callsites[0]
            .tip5_mode
            .as_mut()
            .expect("Tip5 callsite must carry terminal mode");
        mode.new_start = !mode.new_start;
        let tampered_profile =
            NativeTerminalCompiler::terminal_npo_polynomial_profile::<Goldilocks>(&vk);

        assert_ne!(
            profile, tampered_profile,
            "NPO polynomial profile must bind Tip5 row mode counts"
        );
        assert_ne!(
            backend_digest,
            compiler.backend_relation_digest_goldilocks(&vk),
            "backend digest must absorb the NPO polynomial profile"
        );
    }

    #[test]
    fn goldilocks_verifier_rejects_missing_or_stale_relation_digest() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, unsigned_vk) = compiler.compile_primitive_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());

        let err = compiler
            .verify_assignment_with_goldilocks_npos(&unsigned_vk, &witness)
            .unwrap_err();
        assert_eq!(err, NativeTerminalVerifyError::MissingRelationDigest);

        let (_pk, mut signed_vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let digest = signed_vk
            .header
            .relation_digest
            .as_mut()
            .expect("digest must be present");
        digest.0[0] ^= 1;
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&signed_vk, &witness)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::RelationDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_relation_digest_binds_operation_inventory() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, mut vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);

        vk.inventory.non_primitive_ops += 1;
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&vk, &witness)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::RelationDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_certificate_round_trips_and_binds_body() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let proof_body = b"native terminal proof body placeholder".to_vec();

        let certificate = compiler
            .assemble_goldilocks_certificate(
                &vk,
                TerminalProofKind::Production,
                &public_inputs,
                proof_body.clone(),
            )
            .unwrap();
        assert_eq!(certificate.proof_kind, TerminalProofKind::Production);
        assert_eq!(certificate.proof_body, proof_body);

        let encoded = postcard::to_allocvec(&certificate).expect("serialize terminal cert");
        let decoded: TerminalCertificate =
            postcard::from_bytes(&encoded).expect("deserialize terminal cert");
        let body = compiler
            .verify_certificate_binding_goldilocks(
                &vk,
                &decoded,
                TerminalProofKind::Production,
                &public_inputs,
            )
            .expect("honest terminal certificate binding must verify");
        assert_eq!(body, proof_body.as_slice());

        let mut tampered_binding = decoded.clone();
        tampered_binding.binding_digest.0[0] ^= 1;
        let err = compiler
            .verify_certificate_binding_goldilocks(
                &vk,
                &tampered_binding,
                TerminalProofKind::Production,
                &public_inputs,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::BindingDigestMismatch { .. }
        ));

        let mut tampered_body = decoded.clone();
        tampered_body.proof_body[0] ^= 1;
        let err = compiler
            .verify_certificate_binding_goldilocks(
                &vk,
                &tampered_body,
                TerminalProofKind::Production,
                &public_inputs,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::ProofBodyDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_certificate_binds_public_values_and_header() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let certificate = compiler
            .assemble_goldilocks_certificate(
                &vk,
                TerminalProofKind::Production,
                &public_inputs,
                vec![1, 2, 3, 4],
            )
            .unwrap();

        let mut tampered_public = public_inputs.clone();
        tampered_public[0] += Goldilocks::ONE;
        let err = compiler
            .verify_certificate_binding_goldilocks(
                &vk,
                &certificate,
                TerminalProofKind::Production,
                &tampered_public,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::PublicValuesDigestMismatch { .. }
        ));

        let mut tampered_header = certificate.clone();
        tampered_header.header.security_bits += 1;
        let err = compiler
            .verify_certificate_binding_goldilocks(
                &vk,
                &tampered_header,
                TerminalProofKind::Production,
                &public_inputs,
            )
            .unwrap_err();
        assert_eq!(err, NativeTerminalVerifyError::CertificateHeaderMismatch);

        let mut stale_binding = certificate.clone();
        stale_binding.public_values_digest.0[0] ^= 1;
        let err = compiler
            .verify_certificate_binding_goldilocks(
                &vk,
                &stale_binding,
                TerminalProofKind::Production,
                &public_inputs,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::PublicValuesDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_production_certificate_verifier_rejects_malformed_body_and_wrong_kind() {
        let mut production_circuit = Circuit::<Goldilocks>::new(15, HashMap::new());
        for index in 0..15 {
            production_circuit.ops.push(Op::Const {
                out: WitnessId(index),
                val: Goldilocks::from_u64(10 + index as u64),
            });
        }
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler
            .compile_goldilocks_terminal(&production_circuit)
            .unwrap();

        let production_certificate = compiler
            .assemble_goldilocks_certificate(
                &vk,
                TerminalProofKind::Production,
                &public_inputs,
                b"future production terminal proof body".to_vec(),
            )
            .expect("production-kind binding envelope can assemble");
        let err = compiler
            .verify_goldilocks_production_certificate(&vk, &production_certificate, &public_inputs)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalProductionProofDeserialization { .. }
        ));

        let (local_circuit, local_public_inputs) = build_tip5_test_circuit();
        let (_local_pk, local_vk) = compiler
            .compile_goldilocks_terminal(&local_circuit)
            .unwrap();
        let witness = execute_tip5_terminal_witness(&local_circuit, local_public_inputs.clone());
        let local_proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &local_vk,
                &local_public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        let local_certificate = compiler
            .assemble_goldilocks_local_certificate(&local_vk, &local_public_inputs, &local_proof)
            .expect("local checkpoint certificate must assemble");
        let err = compiler
            .verify_goldilocks_production_certificate(
                &local_vk,
                &local_certificate,
                &local_public_inputs,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofKindMismatch {
                expected: TerminalProofKind::Production,
                got: TerminalProofKind::LocalCheckpoint,
            }
        );
    }

    #[test]
    fn goldilocks_terminal_production_certificate_round_trips_compact_relation_proof() {
        let mut circuit = Circuit::<Goldilocks>::new(15, HashMap::new());
        for index in 0..15 {
            circuit.ops.push(Op::Const {
                out: WitnessId(index),
                val: Goldilocks::from_u64(10 + index as u64),
            });
        }
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());

        let proof = compiler
            .prove_terminal_production_goldilocks(&vk, &public_inputs, &witness)
            .expect("production compact relation proof must build");
        compiler
            .verify_terminal_production_goldilocks(&vk, &public_inputs, &proof)
            .expect("production compact relation proof must verify");
        let certificate = compiler
            .assemble_goldilocks_production_certificate(&vk, &public_inputs, &proof)
            .expect("production certificate must assemble");
        compiler
            .verify_goldilocks_production_certificate(&vk, &certificate, &public_inputs)
            .expect("production certificate must verify typed body");

        let mut tampered = proof.clone();
        tampered.primitive_r1cs_proof.rounds[0].eval_0_basis[0] ^= 1;
        let err = compiler
            .verify_terminal_production_goldilocks(&vk, &public_inputs, &tampered)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_production_certificate_requires_canonical_parameters() {
        let mut circuit = Circuit::<Goldilocks>::new(15, HashMap::new());
        for index in 0..15 {
            circuit.ops.push(Op::Const {
                out: WitnessId(index),
                val: Goldilocks::from_u64(20 + index as u64),
            });
        }
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());

        let mut proof = compiler
            .prove_terminal_production_goldilocks(&vk, &public_inputs, &witness)
            .expect("production compact relation proof must build");
        let noncanonical_sixty_bits = TerminalProofParameters {
            security_bits: 60,
            log_blowup: 5,
            num_queries: 12,
            query_pow_bits: 0,
        };
        proof.prelude.parameters = noncanonical_sixty_bits;
        proof.prelude.challenge_digest = NativeTerminalCompiler::transcript_challenge_digest(
            &vk.header,
            proof.prelude.parameters,
            &proof.prelude.relation_profile,
            proof.prelude.public_values_digest,
            &proof.prelude.commitments,
            proof.prelude.query_pow_nonce,
        );
        let proof_body =
            postcard::to_allocvec(&proof).expect("tampered production proof must serialize");
        let certificate = compiler
            .assemble_goldilocks_certificate(
                &vk,
                TerminalProofKind::Production,
                &public_inputs,
                proof_body,
            )
            .expect("production envelope must bind tampered body");

        let err = compiler
            .verify_goldilocks_production_certificate(&vk, &certificate, &public_inputs)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofProductionParametersMismatch {
                expected: TerminalProofParameters::production_60bit(),
                got: noncanonical_sixty_bits,
            }
        );
    }

    #[test]
    fn goldilocks_terminal_production_exhaustive_npo_known_index_proof_rejects_tampering() {
        let (circuit, public_inputs) = build_many_tip5_test_circuit(15);
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());

        let proof = compiler
            .prove_terminal_production_goldilocks(&vk, &public_inputs, &witness)
            .expect("production proof with exhaustive NPO rows must build");
        compiler
            .verify_terminal_production_goldilocks(&vk, &public_inputs, &proof)
            .expect("honest production proof with exhaustive NPO rows must verify");

        let mut missing_value = proof.clone();
        missing_value
            .npo_exhaustive_proof
            .as_mut()
            .expect("fixture must include exhaustive NPO proof")
            .witness_multi_opening
            .value_basis_flat
            .pop();
        let err = compiler
            .verify_terminal_production_goldilocks(&vk, &public_inputs, &missing_value)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningValueDimensionMismatch { .. }
        ));

        let (hidden_circuit, hidden_public_inputs, hidden_private_data) =
            build_many_merkle_tip5_test_circuit(15);
        let (_hidden_pk, hidden_vk) = compiler
            .compile_goldilocks_terminal(&hidden_circuit)
            .unwrap();
        let hidden_witness = execute_tip5_terminal_witness_with_private_data(
            &hidden_circuit,
            hidden_public_inputs.clone(),
            hidden_private_data,
        );
        let hidden_proof = compiler
            .prove_terminal_production_goldilocks(
                &hidden_vk,
                &hidden_public_inputs,
                &hidden_witness,
            )
            .expect("production proof with merkle Tip5 rows must build");
        let hidden_values = &hidden_proof
            .npo_exhaustive_proof
            .as_ref()
            .expect("fixture must include exhaustive NPO proof")
            .tip5_hidden_input_values_le;
        assert!(
            !hidden_values.is_empty(),
            "merkle Tip5 rows must serialize verifier-selected hidden lanes"
        );

        let mut missing_hidden_lane = hidden_proof.clone();
        missing_hidden_lane
            .npo_exhaustive_proof
            .as_mut()
            .expect("fixture must include exhaustive NPO proof")
            .tip5_hidden_input_values_le
            .pop();
        let err = compiler
            .verify_terminal_production_goldilocks(
                &hidden_vk,
                &hidden_public_inputs,
                &missing_hidden_lane,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoTip5InputValueLength { .. }
        ));

        let mut extra_hidden_lane = hidden_proof;
        extra_hidden_lane
            .npo_exhaustive_proof
            .as_mut()
            .expect("fixture must include exhaustive NPO proof")
            .tip5_hidden_input_values_le
            .push(1u64.to_le_bytes());
        let err = compiler
            .verify_terminal_production_goldilocks(
                &hidden_vk,
                &hidden_public_inputs,
                &extra_hidden_lane,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoTip5InputValueLength { .. }
        ));

        let mut bad_value = proof;
        let value = &mut bad_value
            .npo_exhaustive_proof
            .as_mut()
            .expect("fixture must include exhaustive NPO proof")
            .witness_multi_opening
            .value_basis_flat[0];
        *value = if *value == 0 { 1 } else { 0 };
        let err = compiler
            .verify_terminal_production_goldilocks(&vk, &public_inputs, &bad_value)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
                | NativeTerminalVerifyError::TerminalOracleOpeningValueNonCanonical { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_production_exhaustive_npo_omits_recompose_hidden_inputs() {
        let (circuit, public_inputs) = build_many_recompose_test_circuit(8);
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_recompose_terminal_witness(&circuit, public_inputs.clone());

        let proof = compiler
            .prove_terminal_production_goldilocks(&vk, &public_inputs, &witness)
            .expect("production proof with recompose rows must build");
        let npo_proof = proof
            .npo_exhaustive_proof
            .as_ref()
            .expect("fixture must include exhaustive NPO proof");
        assert!(
            npo_proof.tip5_hidden_input_values_le.is_empty(),
            "recompose rows have no Tip5 hidden-input payload"
        );
        compiler
            .verify_terminal_production_goldilocks(&vk, &public_inputs, &proof)
            .expect("honest recompose production proof must verify");

        let mut extra_hidden = proof;
        extra_hidden
            .npo_exhaustive_proof
            .as_mut()
            .expect("fixture must include exhaustive NPO proof")
            .tip5_hidden_input_values_le
            .push(0u64.to_le_bytes());
        let err = compiler
            .verify_terminal_production_goldilocks(&vk, &public_inputs, &extra_hidden)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoTip5InputValueLength { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_assignment_evaluation_proof_round_trips_and_binds_public_prefix() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let assignment_oracle = compiler
            .commit_terminal_assignment_goldilocks(&vk, &public_inputs, &witness)
            .expect("assignment oracle must commit");
        let assignment_commitment = assignment_oracle.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![assignment_commitment.root],
            )
            .expect("assignment prelude must build");
        let proof = compiler
            .prove_terminal_assignment_evaluation_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_oracle,
                &witness,
            )
            .expect("assignment evaluation proof must build");
        let final_value = compiler
            .verify_terminal_assignment_evaluation_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_commitment,
                &proof,
            )
            .expect("honest assignment evaluation proof must verify");
        assert_eq!(
            final_value,
            NativeTerminalCompiler::field_from_goldilocks_basis_u64::<Goldilocks>(
                &proof.final_value_basis
            )
            .unwrap()
        );
        assert_eq!(proof.round_openings.len(), proof.fold_commitments.len());
        assert!(
            proof
                .openings
                .iter()
                .all(|opening| opening.rounds.is_empty())
        );

        let mut bad_public_prefix = proof.clone();
        bad_public_prefix.public_prefix_proof.frontier[0].0[0] ^= 1;
        assert!(
            compiler
                .verify_terminal_assignment_evaluation_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &bad_public_prefix,
                )
                .is_err(),
            "assignment proof must bind public prefix openings"
        );

        let mut missing_round_leaf = proof.clone();
        missing_round_leaf.round_openings[0].openings.pop();
        assert!(
            compiler
                .verify_terminal_assignment_evaluation_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &missing_round_leaf,
                )
                .is_err(),
            "assignment proof must include every transcript-derived fold leaf"
        );

        let mut bad_round_value = proof.clone();
        bad_round_value.round_openings[0].openings[0].value_basis[0] =
            bad_round_value.round_openings[0].openings[0].value_basis[0].wrapping_add(1);
        assert!(
            compiler
                .verify_terminal_assignment_evaluation_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &bad_round_value,
                )
                .is_err(),
            "assignment proof must bind compact fold multiproof values"
        );

        let mut stale_round_payload = proof.clone();
        let stale_opening = TerminalOracleOpening {
            index: stale_round_payload.round_openings[0].openings[0].index,
            value_basis: stale_round_payload.round_openings[0].openings[0]
                .value_basis
                .clone(),
            path: Vec::new(),
        };
        stale_round_payload.openings[0]
            .rounds
            .push(TerminalResidualFoldRoundOpening {
                round: 0,
                pair_index: 0,
                left: stale_opening.clone(),
                right: None,
                next: stale_opening,
            });
        assert!(
            compiler
                .verify_terminal_assignment_evaluation_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &stale_round_payload,
                )
                .is_err(),
            "assignment proof must reject stale per-query fold payloads"
        );

        let mut bad_final = proof.clone();
        bad_final.final_value_basis[0] += 1;
        assert!(
            compiler
                .verify_terminal_assignment_evaluation_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &bad_final,
                )
                .is_err(),
            "assignment proof must bind final folded evaluation"
        );

        let variables = 1
            + vk.header.fingerprint.public_flat_len
            + vk.header.fingerprint.witness_count as usize;
        let explicit_point: Vec<_> = (0..NativeTerminalCompiler::terminal_mle_log_size(variables))
            .map(|i| Goldilocks::from_u64(17 + i as u64))
            .collect();
        let explicit_proof = compiler
            .prove_terminal_assignment_evaluation_at_point_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_oracle,
                &witness,
                &explicit_point,
            )
            .expect("assignment evaluation proof at explicit point must build");
        let _explicit_final = compiler
            .verify_terminal_assignment_evaluation_at_point_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_commitment,
                &explicit_proof,
                &explicit_point,
            )
            .expect("assignment evaluation proof at explicit point must verify");
        let mut wrong_point = explicit_point;
        wrong_point[0] += Goldilocks::ONE;
        assert!(
            compiler
                .verify_terminal_assignment_evaluation_at_point_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &explicit_proof,
                    &wrong_point,
                )
                .is_err(),
            "assignment proof must bind the externally supplied evaluation point"
        );
    }

    #[test]
    fn goldilocks_terminal_sparse_r1cs_matrix_sumcheck_round_trips_and_rejects_tampering() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let assignment_oracle = compiler
            .commit_terminal_assignment_goldilocks(&vk, &public_inputs, &witness)
            .expect("assignment oracle must commit");
        let assignment_commitment = assignment_oracle.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![assignment_commitment.root],
            )
            .expect("sumcheck prelude must build");
        let proof = compiler
            .prove_terminal_sparse_r1cs_sumcheck_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_oracle,
                &witness,
            )
            .expect("sparse R1CS matrix sumcheck proof must build");
        compiler
            .verify_terminal_sparse_r1cs_sumcheck_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_commitment,
                &proof,
            )
            .expect("honest sparse R1CS matrix sumcheck must verify");

        let mut tampered = proof;
        tampered.claimed_a_basis[0] += 1;
        assert!(
            compiler
                .verify_terminal_sparse_r1cs_sumcheck_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &tampered,
                )
                .is_err(),
            "sumcheck must bind claimed matrix-vector evaluations"
        );

        let sparse_relation = vk
            .primitive_sparse_r1cs_relation()
            .expect("sparse relation must lower");
        let explicit_row_point: Vec<_> = (0..sparse_relation.log_rows)
            .map(|i| Goldilocks::from_u64(23 + i as u64))
            .collect();
        let explicit_proof = compiler
            .prove_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_oracle,
                &witness,
                &explicit_row_point,
            )
            .expect("sparse R1CS matrix sumcheck proof at explicit row point must build");
        compiler
            .verify_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_commitment,
                &explicit_proof,
                &explicit_row_point,
            )
            .expect("sparse R1CS matrix sumcheck at explicit row point must verify");
        let mut wrong_row_point = explicit_row_point;
        wrong_row_point[0] += Goldilocks::ONE;
        assert!(
            compiler
                .verify_terminal_sparse_r1cs_sumcheck_at_row_point_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &explicit_proof,
                    &wrong_row_point,
                )
                .is_err(),
            "sumcheck must bind the externally supplied row point"
        );
    }

    #[test]
    fn goldilocks_terminal_r1cs_row_product_sumcheck_round_trips_and_rejects_tampering() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let assignment_oracle = compiler
            .commit_terminal_assignment_goldilocks(&vk, &public_inputs, &witness)
            .expect("assignment oracle must commit");
        let assignment_commitment = assignment_oracle.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![assignment_commitment.root],
            )
            .expect("row-product prelude must build");
        let proof = compiler
            .prove_terminal_r1cs_row_product_sumcheck_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_oracle,
                &witness,
            )
            .expect("R1CS row-product sumcheck proof must build");
        compiler
            .verify_terminal_r1cs_row_product_sumcheck_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &assignment_commitment,
                &proof,
            )
            .expect("honest R1CS row-product sumcheck must verify");

        let mut tampered_round = proof.clone();
        tampered_round.rounds[0].eval_0_basis[0] += 1;
        assert!(
            compiler
                .verify_terminal_r1cs_row_product_sumcheck_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &tampered_round,
                )
                .is_err(),
            "row-product sumcheck must bind degree-3 round evaluations"
        );

        let mut tampered_matrix_claim = proof;
        tampered_matrix_claim.matrix_sumcheck.claimed_a_basis[0] += 1;
        assert!(
            compiler
                .verify_terminal_r1cs_row_product_sumcheck_goldilocks(
                    &vk,
                    &public_inputs,
                    &prelude,
                    &assignment_commitment,
                    &tampered_matrix_claim,
                )
                .is_err(),
            "row-product sumcheck must bind delegated matrix-vector claims"
        );
    }

    #[test]
    fn goldilocks_terminal_production_query_domains_must_support_all_queries() {
        let mut circuit = Circuit::<Goldilocks>::new(1, HashMap::new());
        circuit.ops.push(Op::Const {
            out: WitnessId(0),
            val: Goldilocks::from_u64(5),
        });
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();

        let production_certificate = compiler
            .assemble_goldilocks_certificate(
                &vk,
                TerminalProofKind::Production,
                &public_inputs,
                b"future production terminal proof body".to_vec(),
            )
            .expect("production-kind binding envelope can assemble");
        let err = compiler
            .verify_goldilocks_production_certificate(&vk, &production_certificate, &public_inputs)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofQueryDomainTooSmall {
                domain: "witness",
                len: 1,
                num_queries: 15,
            }
        );
    }

    #[test]
    fn goldilocks_terminal_local_certificate_round_trips_and_verifies_typed_body() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");

        let certificate = compiler
            .assemble_goldilocks_local_certificate(&vk, &public_inputs, &proof)
            .expect("typed terminal local certificate must assemble");
        assert_eq!(certificate.proof_kind, TerminalProofKind::LocalCheckpoint);
        let decoded = compiler
            .verify_goldilocks_local_certificate(&vk, &certificate, &public_inputs)
            .expect("typed terminal local certificate must verify");
        assert_eq!(decoded, proof);

        let mut wrong_kind = certificate.clone();
        wrong_kind.proof_kind = TerminalProofKind::Production;
        wrong_kind.binding_digest = NativeTerminalCompiler::binding_digest(
            &wrong_kind.header,
            wrong_kind.proof_kind,
            wrong_kind.public_values_digest,
            wrong_kind.proof_body_digest,
        );
        let err = compiler
            .verify_goldilocks_local_certificate(&vk, &wrong_kind, &public_inputs)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofKindMismatch {
                expected: TerminalProofKind::LocalCheckpoint,
                got: TerminalProofKind::Production,
            }
        );

        let mut tampered_body = certificate.clone();
        tampered_body.proof_body[0] ^= 1;
        let err = compiler
            .verify_goldilocks_local_certificate(&vk, &tampered_body, &public_inputs)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::ProofBodyDigestMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_local_certificate_rejects_invalid_typed_body() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        proof.combined_validity_commitment.label = "invalid_combined_validity".into();
        let proof_body =
            postcard::to_allocvec(&proof).expect("invalid typed proof still serializes");
        let certificate = compiler
            .assemble_goldilocks_certificate(
                &vk,
                TerminalProofKind::LocalCheckpoint,
                &public_inputs,
                proof_body,
            )
            .expect("raw certificate binding can assemble around invalid typed body");

        let err = compiler
            .verify_goldilocks_local_certificate(&vk, &certificate, &public_inputs)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleCommitmentLabelMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_local_certificate_rejects_trailing_body_bytes() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        let mut proof_body = postcard::to_allocvec(&proof).expect("proof serializes");
        proof_body.push(0);
        let certificate = compiler
            .assemble_goldilocks_certificate(
                &vk,
                TerminalProofKind::LocalCheckpoint,
                &public_inputs,
                proof_body,
            )
            .expect("raw certificate binding can assemble around malformed typed body");

        let err = compiler
            .verify_goldilocks_local_certificate(&vk, &certificate, &public_inputs)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalLocalProofTrailingBytes { trailing_len: 1 }
        );
    }

    #[test]
    fn goldilocks_terminal_prelude_binds_parameters_public_profile_and_commitments() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let commitments = vec![TerminalCommitmentDigest([11, 22, 33, 44, 55])];

        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                commitments,
            )
            .expect("honest terminal prelude must build");
        compiler
            .verify_proof_prelude_goldilocks(&vk, &public_inputs, &prelude)
            .expect("honest terminal prelude must verify");
        assert_eq!(prelude.parameters.johnson_bits(), 60);
        assert_eq!(prelude.parameters.num_queries, 15);
        assert_eq!(prelude.parameters.query_pow_bits, 0);
        assert_eq!(prelude.query_pow_nonce, 0);
        assert_eq!(prelude.relation_profile.tip5_rows, 1);
        assert_eq!(
            prelude.relation_profile.external_npo_validity_components,
            NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
        );
        assert_eq!(prelude.commitments.len(), 1);

        let mut tampered_commitment = prelude.clone();
        tampered_commitment.commitments[0].0[0] ^= 1;
        let err = compiler
            .verify_proof_prelude_goldilocks(&vk, &public_inputs, &tampered_commitment)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalPreludeChallengeMismatch { .. }
        ));

        let mut tampered_profile = prelude.clone();
        tampered_profile.relation_profile.tip5_rows += 1;
        let err = compiler
            .verify_proof_prelude_goldilocks(&vk, &public_inputs, &tampered_profile)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalPreludeProfileMismatch { .. }
        ));

        let mut tampered_npo_components = prelude.clone();
        tampered_npo_components
            .relation_profile
            .external_npo_validity_components += 1;
        let err = compiler
            .verify_proof_prelude_goldilocks(&vk, &public_inputs, &tampered_npo_components)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalPreludeProfileMismatch { .. }
        ));

        let mut tampered_public = public_inputs.clone();
        tampered_public[0] += Goldilocks::ONE;
        let err = compiler
            .verify_proof_prelude_goldilocks(&vk, &tampered_public, &prelude)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalPreludePublicValuesMismatch { .. }
        ));

        let mut stale_challenge = prelude.clone();
        stale_challenge.challenge_digest.0[0] ^= 1;
        let err = compiler
            .verify_proof_prelude_goldilocks(&vk, &public_inputs, &stale_challenge)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalPreludeChallengeMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_prelude_rejects_missing_or_weak_commitment_profile() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let commitment = TerminalCommitmentDigest([1, 2, 3, 4, 5]);

        let err = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![],
            )
            .unwrap_err();
        assert_eq!(err, NativeTerminalVerifyError::MissingTerminalCommitment);

        let weak_params = TerminalProofParameters {
            security_bits: 59,
            log_blowup: 4,
            num_queries: 15,
            query_pow_bits: 0,
        };
        let err = compiler
            .build_proof_prelude_goldilocks(&vk, &public_inputs, weak_params, vec![commitment])
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofParametersTooWeak {
                requested: 59,
                minimum: MIN_TERMINAL_SECURITY_BITS,
            }
        );

        let mismatched_params = TerminalProofParameters {
            security_bits: 61,
            log_blowup: 4,
            num_queries: 16,
            query_pow_bits: 0,
        };
        let err = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                mismatched_params,
                vec![commitment],
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofParametersMismatch {
                expected: 60,
                got: 61,
            }
        );

        let overstated_params = TerminalProofParameters {
            security_bits: 60,
            log_blowup: 4,
            num_queries: 14,
            query_pow_bits: 0,
        };
        let err = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                overstated_params,
                vec![commitment],
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofParametersOverstated {
                declared: 60,
                johnson_bits: 56,
            }
        );

        let pure_query_48_params = TerminalProofParameters {
            security_bits: 60,
            log_blowup: 4,
            num_queries: 12,
            query_pow_bits: 0,
        };
        let err = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                pure_query_48_params,
                vec![commitment],
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofParametersOverstated {
                declared: 60,
                johnson_bits: 48,
            }
        );

        let pow_only_params = TerminalProofParameters {
            security_bits: 60,
            log_blowup: 4,
            num_queries: 12,
            query_pow_bits: 12,
        };
        let err = compiler
            .build_proof_prelude_goldilocks(&vk, &public_inputs, pow_only_params, vec![commitment])
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofQueryPowUnsupported { bits: 12 }
        );

        let pow_params = TerminalProofParameters {
            security_bits: 60,
            log_blowup: 4,
            num_queries: 15,
            query_pow_bits: 12,
        };
        let err = compiler
            .build_proof_prelude_goldilocks(&vk, &public_inputs, pow_params, vec![commitment])
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofQueryPowUnsupported { bits: 12 }
        );

        let zero_pow_params = TerminalProofParameters {
            security_bits: 60,
            log_blowup: 4,
            num_queries: 15,
            query_pow_bits: 0,
        };
        let mut noncanonical_zero_pow = compiler
            .build_proof_prelude_goldilocks(&vk, &public_inputs, zero_pow_params, vec![commitment])
            .expect("zero-query-PoW prelude must build");
        noncanonical_zero_pow.query_pow_nonce = 1;
        noncanonical_zero_pow.challenge_digest =
            NativeTerminalCompiler::transcript_challenge_digest(
                &vk.header,
                noncanonical_zero_pow.parameters,
                &noncanonical_zero_pow.relation_profile,
                noncanonical_zero_pow.public_values_digest,
                &noncanonical_zero_pow.commitments,
                noncanonical_zero_pow.query_pow_nonce,
            );
        let err = compiler
            .verify_proof_prelude_goldilocks(&vk, &public_inputs, &noncanonical_zero_pow)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalProofQueryPowNonceNonCanonical { got: 1 }
        );
    }

    #[test]
    fn goldilocks_terminal_oracle_commitment_opens_and_binds_prelude() {
        let values: Vec<_> = (0..5).map(|i| Goldilocks::from_u64(10 + i)).collect();
        let tree = TerminalOracleMerkleTree::commit_goldilocks_values("witness", &values)
            .expect("non-empty terminal oracle must commit");
        let commitment = tree.commitment();
        let opening = tree
            .open_goldilocks_value(3, &values[3])
            .expect("in-bounds terminal oracle opening");

        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        compiler
            .verify_terminal_oracle_opening(&commitment, &opening)
            .expect("opening must verify against root");
        compiler
            .verify_terminal_oracle_opening_value_goldilocks(&commitment, &opening, &values[3])
            .expect("opening must bind expected field value");

        let (circuit, public_inputs) = build_tip5_test_circuit();
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![commitment.root],
            )
            .expect("oracle root must bind into terminal prelude");
        assert_eq!(prelude.commitments, vec![commitment.root]);
        compiler
            .verify_proof_prelude_goldilocks(&vk, &public_inputs, &prelude)
            .expect("prelude with oracle root must verify");
    }

    #[test]
    fn goldilocks_terminal_known_index_multi_proof_compacts_boolean_values() {
        let values = vec![
            Goldilocks::from_u64(11),
            Goldilocks::ONE,
            Goldilocks::from_u64(13),
            Goldilocks::ZERO,
            Goldilocks::from_u64(17),
        ];
        let tree = TerminalOracleMerkleTree::commit_goldilocks_values("witness", &values)
            .expect("non-empty terminal oracle must commit");
        let commitment = tree.commitment();
        let opening_values = values
            .iter()
            .enumerate()
            .map(|(index, value)| (index, value))
            .collect::<Vec<_>>();
        let proof = tree
            .open_goldilocks_known_index_multi_values_with_boolean_indices(&opening_values, &[1, 3])
            .expect("known-index proof with compact booleans must open");
        assert_eq!(proof.boolean_value_bits, vec![1]);
        assert_eq!(proof.value_basis_flat.len(), 3);

        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let verified = compiler
            .verify_terminal_oracle_known_index_multi_proof_goldilocks_with_boolean_indices::<
                Goldilocks,
            >(&commitment, &[0, 1, 2, 3, 4], &[1, 3], &proof)
            .expect("compact boolean known-index proof must verify");
        assert_eq!(
            verified.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
            values
        );

        let mut missing_bits = proof.clone();
        missing_bits.boolean_value_bits.pop();
        let err = compiler
            .verify_terminal_oracle_known_index_multi_proof_goldilocks_with_boolean_indices::<
                Goldilocks,
            >(&commitment, &[0, 1, 2, 3, 4], &[1, 3], &missing_bits)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningValueDimensionMismatch { .. }
        ));

        let mut bad_bit = proof;
        bad_bit.boolean_value_bits[0] ^= 1;
        let err = compiler
            .verify_terminal_oracle_known_index_multi_proof_goldilocks_with_boolean_indices::<
                Goldilocks,
            >(&commitment, &[0, 1, 2, 3, 4], &[1, 3], &bad_bit)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
        ));

        let mut noncanonical_tail = bad_bit;
        noncanonical_tail.boolean_value_bits[0] = 0b1000_0001;
        let err = compiler
            .verify_terminal_oracle_known_index_multi_proof_goldilocks_with_boolean_indices::<
                Goldilocks,
            >(
                &commitment,
                &[0, 1, 2, 3, 4],
                &[1, 3],
                &noncanonical_tail,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningValueNonCanonical { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_oracle_opening_rejects_tampering() {
        let values: Vec<_> = (0..5).map(|i| Goldilocks::from_u64(20 + i)).collect();
        let tree = TerminalOracleMerkleTree::commit_goldilocks_values("witness", &values)
            .expect("non-empty terminal oracle must commit");
        let commitment = tree.commitment();
        let opening = tree
            .open_goldilocks_value(2, &values[2])
            .expect("in-bounds terminal oracle opening");
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);

        let mut tampered_index = opening.clone();
        tampered_index.index = 3;
        let err = compiler
            .verify_terminal_oracle_opening(&commitment, &tampered_index)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
        ));

        let mut tampered_value = opening.clone();
        tampered_value.value_basis[0] ^= 1;
        let err = compiler
            .verify_terminal_oracle_opening(&commitment, &tampered_value)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
        ));
        let err = compiler
            .verify_terminal_oracle_opening_value_goldilocks(
                &commitment,
                &tampered_value,
                &values[2],
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningValueMismatch { .. }
        ));

        let mut tampered_path = opening.clone();
        tampered_path.path[0].digest.0[0] ^= 1;
        let err = compiler
            .verify_terminal_oracle_opening(&commitment, &tampered_path)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
        ));

        let mut tampered_root = commitment.clone();
        tampered_root.root.0[0] ^= 1;
        let err = compiler
            .verify_terminal_oracle_opening(&tampered_root, &opening)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
        ));

        let mut short_path = opening.clone();
        short_path.path.pop();
        let err = compiler
            .verify_terminal_oracle_opening(&commitment, &short_path)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch {
                expected: 3,
                got: 2,
            }
        );

        let err = tree.open_goldilocks_value(5, &values[0]).unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleIndexOutOfBounds {
                index: 5,
                values_len: 5,
            }
        );

        let mut out_of_bounds = opening.clone();
        out_of_bounds.index = 5;
        let err = compiler
            .verify_terminal_oracle_opening(&commitment, &out_of_bounds)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningIndexOutOfBounds {
                index: 5,
                values_len: 5,
            }
        );

        let empty = TerminalOracleMerkleTree::commit_goldilocks_values::<Goldilocks>("empty", &[])
            .unwrap_err();
        assert_eq!(empty, NativeTerminalVerifyError::TerminalOracleEmpty);
    }

    #[test]
    fn goldilocks_terminal_query_plan_is_transcript_derived_and_opened() {
        let values: Vec<_> = (0..17).map(|i| Goldilocks::from_u64(100 + i)).collect();
        let tree = TerminalOracleMerkleTree::commit_goldilocks_values("witness", &values)
            .expect("non-empty terminal oracle must commit");
        let commitment = tree.commitment();

        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![commitment.root],
            )
            .expect("terminal prelude must bind oracle commitment");

        let plan_a = compiler
            .derive_terminal_query_plan(&prelude, &commitment)
            .expect("query plan must derive");
        let plan_b = compiler
            .derive_terminal_query_plan(&prelude, &commitment)
            .expect("query plan must be deterministic");
        assert_eq!(plan_a, plan_b);
        assert_eq!(
            plan_a.indices.len(),
            TerminalProofParameters::production_60bit().num_queries as usize
        );
        for (pos, index) in plan_a.indices.iter().enumerate() {
            assert!(
                !plan_a.indices[..pos].contains(index),
                "query plan must not duplicate an index when the domain can provide enough rows"
            );
        }
        assert!(plan_a.indices.iter().all(|index| *index < values.len()));

        let openings: Vec<_> = plan_a
            .indices
            .iter()
            .map(|index| {
                tree.open_goldilocks_value(*index, &values[*index])
                    .expect("derived query index must open")
            })
            .collect();
        let verified = compiler
            .verify_terminal_query_openings(&prelude, &commitment, &openings)
            .expect("query openings must verify against derived plan");
        assert_eq!(verified, plan_a);

        let unbound_prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![TerminalCommitmentDigest([7, 7, 7, 7, 7])],
            )
            .expect("syntactically valid prelude with unrelated commitment");
        let err = compiler
            .verify_terminal_query_openings(&unbound_prelude, &commitment, &openings)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalPreludeCommitmentNotBound {
                root: commitment.root,
            }
        );

        let mut steered = openings.clone();
        steered[0] = tree
            .open_goldilocks_value(
                (plan_a.indices[0] + 1) % values.len(),
                &values[(plan_a.indices[0] + 1) % values.len()],
            )
            .expect("alternate opening exists");
        let err = compiler
            .verify_terminal_query_openings(&prelude, &commitment, &steered)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleQueryIndexMismatch { .. }
        ));

        let mut short = openings.clone();
        short.pop();
        let err = compiler
            .verify_terminal_query_openings(&prelude, &commitment, &short)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleQueryLengthMismatch {
                expected: plan_a.indices.len(),
                got: plan_a.indices.len() - 1,
            }
        );
    }

    #[test]
    fn goldilocks_terminal_query_plan_allows_tiny_domains_without_hanging() {
        let values = vec![Goldilocks::from_u64(1), Goldilocks::from_u64(2)];
        let tree = TerminalOracleMerkleTree::commit_goldilocks_values("tiny", &values)
            .expect("tiny oracle must commit");
        let commitment = tree.commitment();

        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![commitment.root],
            )
            .expect("terminal prelude must bind tiny oracle commitment");

        let plan = compiler
            .derive_terminal_query_plan(&prelude, &commitment)
            .expect("tiny-domain query plan must still derive for local tests");
        assert_eq!(
            plan.indices.len(),
            TerminalProofParameters::production_60bit().num_queries as usize
        );
        assert!(plan.indices.iter().all(|index| *index < values.len()));
        assert!(
            plan.indices.windows(2).any(|pair| pair[0] == pair[1])
                || plan.indices.iter().filter(|index| **index == 0).count() > 1
                || plan.indices.iter().filter(|index| **index == 1).count() > 1,
            "two-row domain must necessarily reuse at least one query index"
        );
    }

    #[test]
    fn goldilocks_terminal_query_plan_changes_with_prelude_or_root() {
        let values: Vec<_> = (0..17).map(|i| Goldilocks::from_u64(200 + i)).collect();
        let tree_a = TerminalOracleMerkleTree::commit_goldilocks_values("witness", &values)
            .expect("oracle A must commit");
        let mut values_b = values.clone();
        values_b[0] += Goldilocks::ONE;
        let tree_b = TerminalOracleMerkleTree::commit_goldilocks_values("witness", &values_b)
            .expect("oracle B must commit");

        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let prelude_a = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![tree_a.root()],
            )
            .expect("prelude A");
        let prelude_b = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![tree_b.root()],
            )
            .expect("prelude B");

        let plan_a = compiler
            .derive_terminal_query_plan(&prelude_a, &tree_a.commitment())
            .expect("query plan A");
        let plan_b = compiler
            .derive_terminal_query_plan(&prelude_b, &tree_b.commitment())
            .expect("query plan B");
        assert_ne!(
            plan_a.indices, plan_b.indices,
            "query plan must change when the bound oracle root changes"
        );

        let empty_commitment = TerminalOracleCommitment {
            label: "empty".into(),
            values_len: 0,
            root: TerminalCommitmentDigest([0; 5]),
        };
        let err = compiler
            .derive_terminal_query_plan(&prelude_a, &empty_commitment)
            .unwrap_err();
        assert_eq!(err, NativeTerminalVerifyError::TerminalQueryOracleEmpty);
    }

    #[test]
    fn goldilocks_terminal_primitive_constraint_queries_verify_committed_witness() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root],
            )
            .expect("prelude must bind witness commitment");

        let proof = compiler
            .prove_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &prelude,
                &witness_tree,
                &witness,
            )
            .expect("primitive query proof must build");
        assert_eq!(
            proof.openings.len(),
            TerminalProofParameters::production_60bit().num_queries as usize
        );
        let plan = compiler
            .verify_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &proof,
            )
            .expect("primitive query proof must verify");
        assert_eq!(plan.indices.len(), proof.openings.len());

        let mut steered = proof.clone();
        steered.openings[0].constraint_index =
            (steered.openings[0].constraint_index + 1) % plan.domain_len;
        let err = compiler
            .verify_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &steered,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalConstraintQueryIndexMismatch { .. }
        ));

        let mut missing_opening = proof.clone();
        missing_opening.openings[0].witness_openings.pop();
        let err = compiler
            .verify_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &missing_opening,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalConstraintOpeningCountMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_primitive_constraint_queries_reject_invalid_committed_value() {
        let mut circuit = Circuit::<Goldilocks>::new(1, HashMap::new());
        circuit.ops.push(Op::Const {
            out: WitnessId(0),
            val: Goldilocks::from_u64(5),
        });

        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let public_inputs = Vec::<Goldilocks>::new();
        let bad_witness_values = vec![Goldilocks::from_u64(6)];
        let bad_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &bad_witness_values)
                .expect("bad witness oracle still commits");
        let bad_commitment = bad_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![bad_commitment.root],
            )
            .expect("prelude must bind bad witness commitment");
        let bad_witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&circuit),
            public_inputs: public_inputs.clone(),
            private_inputs: Vec::new(),
            traces: one_value_traces(bad_witness_values[0]),
        };
        let proof = compiler
            .prove_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &prelude,
                &bad_tree,
                &bad_witness,
            )
            .expect("proof can be built for committed bad witness");
        let err = compiler
            .verify_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &bad_commitment,
                &proof,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::ConstraintViolation {
                constraint_index: 0,
                kind: "sampled_const",
            }
        );

        let mut noncanonical = proof.clone();
        noncanonical.openings[0].witness_openings[0].value_basis[0] = Goldilocks::ORDER_U64;
        let err = compiler
            .verify_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &bad_commitment,
                &noncanonical,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch {
                expected: bad_commitment.root,
                got: NativeTerminalCompiler::terminal_oracle_leaf_digest_from_basis(
                    "witness",
                    1,
                    0,
                    &[Goldilocks::ORDER_U64],
                ),
            }
        );
    }

    #[test]
    fn goldilocks_terminal_quadratic_residual_queries_verify_zero_oracle() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let residual_tree = compiler
            .commit_terminal_quadratic_residuals_goldilocks(&vk, &public_inputs, &witness)
            .expect("honest residual oracle must commit");
        let residual_commitment = residual_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_tree.root(), residual_commitment.root],
            )
            .expect("prelude must bind witness and residual commitments");

        let proof = compiler
            .prove_terminal_quadratic_residual_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &residual_tree,
                &witness,
            )
            .expect("honest residual proof must build");
        let plan = compiler
            .verify_terminal_quadratic_residual_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &residual_commitment,
                &proof,
            )
            .expect("honest residual proof must verify");
        assert_eq!(
            proof.openings.len(),
            TerminalProofParameters::production_60bit().num_queries as usize
        );
        assert_eq!(
            plan.oracle_label,
            NativeTerminalCompiler::quadratic_residual_oracle_label()
        );
    }

    #[test]
    fn goldilocks_terminal_local_oracle_proofs_require_prelude_bound_roots() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let residual_tree = compiler
            .commit_terminal_quadratic_residuals_goldilocks(&vk, &public_inputs, &witness)
            .expect("residual oracle must commit");
        let residual_commitment = residual_tree.commitment();
        let unbound_prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![TerminalCommitmentDigest([9, 8, 7, 6, 5])],
            )
            .expect("syntactically valid prelude with unrelated commitment");

        let err = compiler
            .verify_terminal_primitive_constraint_queries_goldilocks(
                &vk,
                &public_inputs,
                &unbound_prelude,
                &witness_commitment,
                &TerminalPrimitiveConstraintProof { openings: vec![] },
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalPreludeCommitmentNotBound {
                root: witness_commitment.root,
            }
        );

        let err = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &unbound_prelude,
                &witness_commitment,
                &TerminalNpoProof { openings: vec![] },
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalPreludeCommitmentNotBound {
                root: witness_commitment.root,
            }
        );

        let err = compiler
            .verify_terminal_quadratic_residual_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &unbound_prelude,
                &residual_commitment,
                &TerminalQuadraticResidualProof { openings: vec![] },
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalPreludeCommitmentNotBound {
                root: residual_commitment.root,
            }
        );

        let err = compiler
            .verify_terminal_quadratic_consistency_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &unbound_prelude,
                &witness_commitment,
                &residual_commitment,
                &TerminalResidualFoldProof {
                    fold_commitments: vec![],
                    final_value_basis: vec![0],
                    openings: vec![],
                },
                &TerminalQuadraticConsistencyProof { openings: vec![] },
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalPreludeCommitmentNotBound {
                root: witness_commitment.root,
            }
        );
    }

    #[test]
    fn goldilocks_terminal_local_proof_rejects_base_oracle_identity_drift() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");

        let mut wrong_witness_label = proof.clone();
        wrong_witness_label.witness_commitment.label = "alternate_witness".into();
        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &wrong_witness_label)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleCommitmentLabelMismatch {
                expected: NativeTerminalCompiler::witness_oracle_label().into(),
                got: "alternate_witness".into(),
            }
        );

        let mut wrong_witness_len = proof.clone();
        wrong_witness_len.witness_commitment.values_len += 1;
        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &wrong_witness_len)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleCommitmentLengthMismatch {
                label: NativeTerminalCompiler::witness_oracle_label().into(),
                expected: vk.header.fingerprint.witness_count as usize,
                got: vk.header.fingerprint.witness_count as usize + 1,
            }
        );

        let mut wrong_combined_label = proof.clone();
        wrong_combined_label.combined_validity_commitment.label =
            "alternate_combined_validity".into();
        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &wrong_combined_label)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalOracleCommitmentLabelMismatch {
                expected: NativeTerminalCompiler::combined_validity_oracle_label().into(),
                got: "alternate_combined_validity".into(),
            }
        );
    }

    #[test]
    fn goldilocks_terminal_quadratic_residual_queries_reject_nonzero_residuals() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let relation = vk
            .primitive_quadratic_relation()
            .expect("quadratic relation must lower");
        let bad_residuals = vec![Goldilocks::ONE; relation.constraints.len()];
        let bad_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::quadratic_residual_oracle_label(),
            &bad_residuals,
        )
        .expect("bad residual oracle still commits");
        let bad_commitment = bad_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![bad_commitment.root],
            )
            .expect("prelude must bind bad residual commitment");
        let plan = compiler
            .derive_terminal_quadratic_residual_query_plan(&vk, &prelude, &bad_commitment)
            .expect("residual query plan must derive");
        let openings = plan
            .indices
            .iter()
            .map(|index| {
                bad_tree
                    .open_goldilocks_value(*index, &bad_residuals[*index])
                    .expect("bad residual query index must open")
            })
            .collect();
        let proof = TerminalQuadraticResidualProof { openings };

        let err = compiler
            .verify_terminal_quadratic_residual_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &bad_commitment,
                &proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalQuadraticResidualNonZero { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_quadratic_residual_queries_reject_steering() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let residual_tree = compiler
            .commit_terminal_quadratic_residuals_goldilocks(&vk, &public_inputs, &witness)
            .expect("honest residual oracle must commit");
        let residual_commitment = residual_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![residual_commitment.root],
            )
            .expect("prelude must bind residual commitment");
        let mut proof = compiler
            .prove_terminal_quadratic_residual_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &residual_tree,
                &witness,
            )
            .expect("honest residual proof must build");
        proof.openings[0].index = (proof.openings[0].index + 1) % residual_commitment.values_len;

        let err = compiler
            .verify_terminal_quadratic_residual_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &residual_commitment,
                &proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalQuadraticResidualQueryIndexMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_quadratic_consistency_queries_bind_witness_and_residual_oracles() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let residual_tree = compiler
            .commit_terminal_quadratic_residuals_goldilocks(&vk, &public_inputs, &witness)
            .expect("honest residual oracle must commit");
        let residual_commitment = residual_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root, residual_commitment.root],
            )
            .expect("prelude must bind witness and residual commitments");

        let proof = compiler
            .prove_terminal_residual_fold_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &residual_tree,
                &witness,
            )
            .expect("honest residual fold proof must build");
        let consistency_proof = compiler
            .prove_terminal_quadratic_consistency_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_tree,
                &residual_tree,
                &proof,
                &witness,
            )
            .expect("honest quadratic consistency proof must build");
        let plan = compiler
            .verify_terminal_quadratic_consistency_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &residual_commitment,
                &proof,
                &consistency_proof,
            )
            .expect("honest quadratic consistency proof must verify");
        assert_eq!(
            consistency_proof.openings.len(),
            TerminalProofParameters::production_60bit().num_queries as usize
        );
        assert_eq!(plan.oracle_len, residual_commitment.values_len);
    }

    #[test]
    fn goldilocks_terminal_quadratic_consistency_rejects_bad_residual_oracle() {
        let mut circuit = Circuit::<Goldilocks>::new(1, HashMap::new());
        circuit.ops.push(Op::Const {
            out: WitnessId(0),
            val: Goldilocks::from_u64(5),
        });
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&circuit),
            public_inputs: public_inputs.clone(),
            private_inputs: Vec::new(),
            traces: one_value_traces(Goldilocks::from_u64(5)),
        };
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let bad_residuals = vec![Goldilocks::ONE];
        let bad_residual_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::quadratic_residual_oracle_label(),
            &bad_residuals,
        )
        .expect("bad residual oracle still commits");
        let bad_residual_commitment = bad_residual_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root, bad_residual_commitment.root],
            )
            .expect("prelude must bind bad residual commitment");
        let fold_proof = TerminalResidualFoldProof {
            fold_commitments: Vec::new(),
            final_value_basis: vec![1],
            openings: compiler
                .derive_terminal_residual_fold_query_plan(&prelude, &bad_residual_commitment, &[])
                .expect("one-row residual fold plan must derive")
                .indices
                .iter()
                .map(|index| TerminalResidualFoldQueryOpening {
                    initial_index: *index,
                    rounds: Vec::new(),
                })
                .collect(),
        };
        let plan = compiler
            .derive_terminal_residual_fold_query_plan(&prelude, &bad_residual_commitment, &[])
            .expect("quadratic plan must derive");
        let openings = plan
            .indices
            .iter()
            .map(|index| TerminalQuadraticConsistencyOpening {
                quadratic_index: *index,
                witness_openings: vec![
                    witness_tree
                        .open_goldilocks_value(0, &witness_values[0])
                        .expect("witness must open"),
                ],
            })
            .collect();
        let proof = TerminalQuadraticConsistencyProof { openings };

        let err = compiler
            .verify_terminal_quadratic_consistency_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &bad_residual_commitment,
                &fold_proof,
                &proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldFinalValueNonZero
        ));
    }

    #[test]
    fn goldilocks_terminal_quadratic_consistency_rejects_bad_witness_oracle() {
        let mut circuit = Circuit::<Goldilocks>::new(1, HashMap::new());
        circuit.ops.push(Op::Const {
            out: WitnessId(0),
            val: Goldilocks::from_u64(5),
        });
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let honest_witness = TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(&circuit),
            public_inputs: public_inputs.clone(),
            private_inputs: Vec::new(),
            traces: one_value_traces(Goldilocks::from_u64(5)),
        };
        let residual_tree = compiler
            .commit_terminal_quadratic_residuals_goldilocks(&vk, &public_inputs, &honest_witness)
            .expect("honest residual oracle must commit");
        let residual_commitment = residual_tree.commitment();
        let bad_witness_values = vec![Goldilocks::from_u64(6)];
        let bad_witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &bad_witness_values)
                .expect("bad witness oracle still commits");
        let bad_witness_commitment = bad_witness_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![bad_witness_commitment.root, residual_commitment.root],
            )
            .expect("prelude must bind bad witness commitment");
        let residual_fold_proof = compiler
            .prove_terminal_residual_fold_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &residual_tree,
                &honest_witness,
            )
            .expect("honest residual fold proof must build");
        let plan = compiler
            .derive_terminal_residual_fold_query_plan(
                &prelude,
                &residual_commitment,
                &residual_fold_proof.fold_commitments,
            )
            .expect("quadratic plan must derive");
        let openings = plan
            .indices
            .iter()
            .map(|index| TerminalQuadraticConsistencyOpening {
                quadratic_index: *index,
                witness_openings: vec![
                    bad_witness_tree
                        .open_goldilocks_value(0, &bad_witness_values[0])
                        .expect("bad witness must open"),
                ],
            })
            .collect();
        let proof = TerminalQuadraticConsistencyProof { openings };

        let err = compiler
            .verify_terminal_quadratic_consistency_queries_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &bad_witness_commitment,
                &residual_commitment,
                &residual_fold_proof,
                &proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalQuadraticConsistencyResidualMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_local_proof_round_trips_all_local_components() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());

        let proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        assert_eq!(
            proof.combined_validity_commitment.label,
            NativeTerminalCompiler::combined_validity_oracle_label()
        );
        assert!(
            !proof
                .combined_validity_fold_proof
                .fold_commitments
                .is_empty()
        );
        assert_eq!(
            proof.combined_validity_consistency_proof.openings.len(),
            TerminalProofParameters::production_60bit().num_queries as usize
        );

        let encoded = postcard::to_allocvec(&proof).expect("serialize terminal local proof");
        let decoded: TerminalLocalProof =
            postcard::from_bytes(&encoded).expect("deserialize terminal local proof");
        compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &decoded)
            .expect("decoded terminal local proof must verify");
    }

    #[test]
    fn goldilocks_terminal_local_proof_rejects_malformed_combined_validity_commitment() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        proof.combined_validity_commitment.values_len += 1;

        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &proof)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleCommitmentLengthMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_local_proof_accepts_primitive_only_without_npo_component() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("primitive-only terminal local proof must build");

        assert_eq!(
            proof.combined_validity_commitment.values_len,
            vk.primitive_quadratic_relation()
                .expect("quadratic relation must lower")
                .constraints
                .len()
        );
        compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &proof)
            .expect("primitive-only terminal local proof must verify without NPO component");
    }

    #[test]
    fn goldilocks_terminal_combined_validity_rejects_primitive_row_as_npo() {
        let circuit = build_goldilocks_const_circuit(5);
        let public_inputs = vec![Goldilocks::from_u64(5)];
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("primitive-only terminal local proof must build");
        let validity_index = match &proof.combined_validity_consistency_proof.openings[0] {
            TerminalCombinedValidityConsistencyOpening::Quadratic { validity_index, .. } => {
                *validity_index
            }
            TerminalCombinedValidityConsistencyOpening::Npo { .. } => {
                panic!("primitive-only proof must sample a quadratic row")
            }
        };
        proof.combined_validity_consistency_proof.openings[0] =
            TerminalCombinedValidityConsistencyOpening::Npo {
                validity_index,
                npo_index: 0,
                component_offset: 0,
                npo_opening: TerminalNpoOpening {
                    npo_index: 0,
                    tip5_hidden_input_values: Vec::new(),
                    witness_openings: Vec::new(),
                },
            };

        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &proof)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_combined_validity_rejects_npo_row_as_primitive() {
        let (circuit, public_inputs) = build_npo_only_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("NPO-only terminal local proof must build");
        let npo_query = proof
            .combined_validity_consistency_proof
            .openings
            .iter()
            .position(|opening| {
                matches!(
                    opening,
                    TerminalCombinedValidityConsistencyOpening::Npo { .. }
                )
            })
            .expect("NPO-only proof must sample an NPO row");
        let validity_index = match &proof.combined_validity_consistency_proof.openings[npo_query] {
            TerminalCombinedValidityConsistencyOpening::Npo { validity_index, .. } => {
                *validity_index
            }
            TerminalCombinedValidityConsistencyOpening::Quadratic { .. } => unreachable!(),
        };
        proof.combined_validity_consistency_proof.openings[npo_query] =
            TerminalCombinedValidityConsistencyOpening::Quadratic {
                validity_index,
                witness_openings: Vec::new(),
            };

        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &proof)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalQuadraticConsistencyQueryIndexMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_combined_validity_rejects_npo_index_confusion() {
        let (circuit, public_inputs) = build_npo_only_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        let relation = vk
            .primitive_quadratic_relation()
            .expect("quadratic relation must lower");
        let npo_query = proof
            .combined_validity_consistency_proof
            .openings
            .iter()
            .position(|opening| {
                matches!(
                    opening,
                    TerminalCombinedValidityConsistencyOpening::Npo { .. }
                )
            })
            .expect("NPO-only proof must sample an NPO row");
        let TerminalCombinedValidityConsistencyOpening::Npo {
            validity_index,
            npo_index,
            component_offset,
            npo_opening,
        } = proof.combined_validity_consistency_proof.openings[npo_query].clone()
        else {
            unreachable!("position above selected an NPO opening");
        };

        let mut wrong_validity_index = proof.clone();
        wrong_validity_index
            .combined_validity_consistency_proof
            .openings[npo_query] = TerminalCombinedValidityConsistencyOpening::Npo {
            validity_index: validity_index + 1,
            npo_index,
            component_offset,
            npo_opening: npo_opening.clone(),
        };
        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &wrong_validity_index)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch { .. }
        ));

        let mut wrong_npo_index = proof.clone();
        wrong_npo_index.combined_validity_consistency_proof.openings[npo_query] =
            TerminalCombinedValidityConsistencyOpening::Npo {
                validity_index,
                npo_index: npo_index + 1,
                component_offset,
                npo_opening,
            };
        let err = compiler
            .verify_terminal_local_queries_goldilocks(&vk, &public_inputs, &wrong_npo_index)
            .unwrap_err();
        let (expected_npo_index, expected_component_offset) =
            NativeTerminalCompiler::terminal_npo_validity_component_row::<Goldilocks>(
                &vk,
                validity_index - relation.constraints.len(),
            )
            .expect("combined NPO validity index must map to a component");
        assert_eq!(expected_npo_index, npo_index);
        assert_eq!(expected_component_offset, component_offset);
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityQueryIndexMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_residual_fold_proof_verifies_fold_layers() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");

        assert!(
            !proof
                .combined_validity_fold_proof
                .fold_commitments
                .is_empty()
        );
        assert_eq!(
            proof
                .combined_validity_fold_proof
                .fold_commitments
                .last()
                .expect("final fold commitment must exist")
                .values_len,
            1
        );
        compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &proof.prelude,
                &proof.combined_validity_commitment,
                &proof.combined_validity_fold_proof,
            )
            .expect("honest combined validity fold proof must verify");
    }

    #[test]
    fn goldilocks_terminal_residual_fold_rejects_query_steering() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        proof.combined_validity_fold_proof.openings[0].initial_index =
            (proof.combined_validity_fold_proof.openings[0].initial_index + 1)
                % proof.combined_validity_commitment.values_len;

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &proof.prelude,
                &proof.combined_validity_commitment,
                &proof.combined_validity_fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldQueryIndexMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_residual_fold_rejects_missing_right_opening() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        let round = proof.combined_validity_fold_proof.openings[0]
            .rounds
            .iter_mut()
            .find(|round| round.right.is_some())
            .expect("at least one sampled fold round should have a right opening");
        round.right = None;

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &proof.prelude,
                &proof.combined_validity_commitment,
                &proof.combined_validity_fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningMissing { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_residual_fold_rejects_right_opening_path() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        let round = proof.combined_validity_fold_proof.openings[0]
            .rounds
            .iter_mut()
            .find(|round| round.right.is_some())
            .expect("at least one sampled fold round should have a right opening");
        round.right.as_mut().expect("right opening").path = round.left.path.clone();

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &proof.prelude,
                &proof.combined_validity_commitment,
                &proof.combined_validity_fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldRightOpeningPathUnexpected { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_residual_fold_rejects_bad_compact_right_value() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        let right = proof.combined_validity_fold_proof.openings[0]
            .rounds
            .iter_mut()
            .find_map(|round| round.right.as_mut())
            .expect("at least one sampled fold round should have a right opening");
        right.value_basis[0] = right.value_basis[0].wrapping_add(1);

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &proof.prelude,
                &proof.combined_validity_commitment,
                &proof.combined_validity_fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_residual_fold_rejects_next_opening_path() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        proof.combined_validity_fold_proof.openings[0].rounds[0]
            .next
            .path = proof.combined_validity_fold_proof.openings[0].rounds[0]
            .left
            .path
            .clone();

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &proof.prelude,
                &proof.combined_validity_commitment,
                &proof.combined_validity_fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldNextOpeningPathUnexpected { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_residual_fold_rejects_bad_compact_next_value() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let mut proof = compiler
            .prove_terminal_local_queries_goldilocks(
                &vk,
                &public_inputs,
                &witness,
                TerminalProofParameters::production_60bit(),
            )
            .expect("terminal local proof must build");
        proof.combined_validity_fold_proof.openings[0].rounds[0]
            .next
            .value_basis[0] = proof.combined_validity_fold_proof.openings[0].rounds[0]
            .next
            .value_basis[0]
            .wrapping_add(1);

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &proof.prelude,
                &proof.combined_validity_commitment,
                &proof.combined_validity_fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldConsistencyMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_residual_fold_rejects_nonzero_final_value() {
        let mut circuit = Circuit::<Goldilocks>::new(1, HashMap::new());
        circuit.ops.push(Op::Const {
            out: WitnessId(0),
            val: Goldilocks::from_u64(5),
        });
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let bad_residual = vec![Goldilocks::ONE];
        let residual_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::quadratic_residual_oracle_label(),
            &bad_residual,
        )
        .expect("bad residual oracle must commit");
        let residual_commitment = residual_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![residual_commitment.root],
            )
            .expect("prelude must bind residual commitment");
        let plan = compiler
            .derive_terminal_residual_fold_query_plan(&prelude, &residual_commitment, &[])
            .expect("one-row fold query plan must derive");
        let proof = TerminalResidualFoldProof {
            fold_commitments: Vec::new(),
            final_value_basis: vec![1],
            openings: plan
                .indices
                .iter()
                .map(|index| TerminalResidualFoldQueryOpening {
                    initial_index: *index,
                    rounds: Vec::new(),
                })
                .collect(),
        };

        let err = compiler
            .verify_terminal_residual_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &residual_commitment,
                &proof,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldFinalValueNonZero
        );
    }

    #[test]
    fn goldilocks_terminal_combined_validity_fold_rejects_stale_final_root() {
        let mut circuit = Circuit::<Goldilocks>::new(1, HashMap::new());
        circuit.ops.push(Op::Const {
            out: WitnessId(0),
            val: Goldilocks::from_u64(5),
        });
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let combined_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::combined_validity_oracle_label(),
            &[Goldilocks::ZERO],
        )
        .expect("one-row combined validity oracle must commit");
        let combined_commitment = combined_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![combined_commitment.root],
            )
            .expect("prelude must bind combined validity commitment");
        let plan = compiler
            .derive_terminal_combined_validity_fold_query_plan(&prelude, &combined_commitment, &[])
            .expect("one-row combined fold query plan must derive");
        let proof = TerminalResidualFoldProof {
            fold_commitments: Vec::new(),
            final_value_basis: vec![1],
            openings: plan
                .indices
                .iter()
                .map(|index| TerminalResidualFoldQueryOpening {
                    initial_index: *index,
                    rounds: Vec::new(),
                })
                .collect(),
        };

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &combined_commitment,
                &proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldFinalRootMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_combined_validity_fold_rejects_re_rooted_nonzero_final_value() {
        let mut circuit = Circuit::<Goldilocks>::new(1, HashMap::new());
        circuit.ops.push(Op::Const {
            out: WitnessId(0),
            val: Goldilocks::from_u64(5),
        });
        let public_inputs = Vec::<Goldilocks>::new();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let bad_combined_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::combined_validity_oracle_label(),
            &[Goldilocks::ONE],
        )
        .expect("one-row nonzero combined validity oracle must commit");
        let bad_combined_commitment = bad_combined_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![bad_combined_commitment.root],
            )
            .expect("prelude must bind nonzero combined validity commitment");
        let plan = compiler
            .derive_terminal_combined_validity_fold_query_plan(
                &prelude,
                &bad_combined_commitment,
                &[],
            )
            .expect("one-row combined fold query plan must derive");
        let proof = TerminalResidualFoldProof {
            fold_commitments: Vec::new(),
            final_value_basis: vec![1],
            openings: plan
                .indices
                .iter()
                .map(|index| TerminalResidualFoldQueryOpening {
                    initial_index: *index,
                    rounds: Vec::new(),
                })
                .collect(),
        };

        let err = compiler
            .verify_terminal_combined_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &bad_combined_commitment,
                &proof,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalResidualFoldFinalValueNonZero
        );
    }

    #[test]
    fn goldilocks_terminal_npo_validity_fold_proof_verifies_fold_layers() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, witness_commitment, validity_commitment, fold_proof, consistency_proof) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);

        assert_eq!(
            validity_commitment.values_len,
            NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
        );
        assert_eq!(
            fold_proof
                .fold_commitments
                .last()
                .expect("NPO validity oracle must fold")
                .values_len,
            1
        );
        assert_eq!(
            fold_proof.round_openings.len(),
            fold_proof.fold_commitments.len()
        );
        assert!(
            fold_proof
                .openings
                .iter()
                .all(|opening| opening.rounds.is_empty())
        );
        let fold_plan = compiler
            .verify_terminal_npo_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &validity_commitment,
                &fold_proof,
            )
            .expect("honest NPO validity fold proof must verify");
        let consistency_plan = compiler
            .verify_terminal_npo_validity_consistency_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &validity_commitment,
                &fold_proof,
                &consistency_proof,
            )
            .expect("honest NPO validity consistency proof must verify");
        assert_eq!(fold_plan.indices, consistency_plan.indices);
    }

    #[test]
    fn goldilocks_terminal_npo_validity_consistency_rejects_tampered_npo_opening() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, witness_commitment, validity_commitment, fold_proof, consistency_proof) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);

        let mut missing_witness_opening = consistency_proof.clone();
        missing_witness_opening.witness_multi_opening.openings.pop();
        let err = compiler
            .verify_terminal_npo_validity_consistency_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &validity_commitment,
                &fold_proof,
                &missing_witness_opening,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch { .. }
                | NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
                | NativeTerminalVerifyError::TerminalOracleQueryLengthMismatch { .. }
        ));

        let mut bad_witness_value = consistency_proof.clone();
        let value = &mut bad_witness_value.witness_multi_opening.openings[0].value_basis[0];
        *value = if *value == 0 { 1 } else { 0 };
        let err = compiler
            .verify_terminal_npo_validity_consistency_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &validity_commitment,
                &fold_proof,
                &bad_witness_value,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
                | NativeTerminalVerifyError::TerminalOracleOpeningValueMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_validity_fold_rejects_query_steering() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, _, validity_commitment, mut fold_proof, _) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);
        fold_proof.openings[0].initial_index =
            (fold_proof.openings[0].initial_index + 1) % validity_commitment.values_len;

        let err = compiler
            .verify_terminal_npo_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &validity_commitment,
                &fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityFoldQueryIndexMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_validity_fold_rejects_right_opening_path() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, _, validity_commitment, mut fold_proof, _) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);
        fold_proof.round_openings[0].openings.pop();

        let err = compiler
            .verify_terminal_npo_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &validity_commitment,
                &fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningPathLengthMismatch { .. }
                | NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
                | NativeTerminalVerifyError::TerminalOracleQueryLengthMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_validity_fold_rejects_bad_compact_right_value() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, _, validity_commitment, mut fold_proof, _) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);
        let right = fold_proof.round_openings[0]
            .openings
            .first_mut()
            .expect("validity fold must open at least one leaf");
        right.value_basis[0] = right.value_basis[0].wrapping_add(1);

        let err = compiler
            .verify_terminal_npo_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &validity_commitment,
                &fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalOracleOpeningRootMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_validity_fold_rejects_next_opening_path() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, _, validity_commitment, mut fold_proof, _) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);
        let stale_opening = TerminalOracleOpening {
            index: fold_proof.round_openings[0].openings[0].index,
            value_basis: fold_proof.round_openings[0].openings[0].value_basis.clone(),
            path: Vec::new(),
        };
        fold_proof.openings[0]
            .rounds
            .push(TerminalResidualFoldRoundOpening {
                round: 0,
                pair_index: 0,
                left: stale_opening.clone(),
                right: None,
                next: stale_opening,
            });

        let err = compiler
            .verify_terminal_npo_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &validity_commitment,
                &fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityFoldRoundCountMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_validity_fold_rejects_bad_compact_next_value() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, _, validity_commitment, mut fold_proof, _) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);
        fold_proof.final_value_basis[0] = fold_proof.final_value_basis[0].wrapping_add(1);

        let err = compiler
            .verify_terminal_npo_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &validity_commitment,
                &fold_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityFoldConsistencyMismatch { .. }
                | NativeTerminalVerifyError::TerminalNpoValidityFoldFinalRootMismatch { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_validity_consistency_rejects_malformed_fold_schedule() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let (prelude, witness_commitment, validity_commitment, mut fold_proof, consistency_proof) =
            standalone_npo_validity_components(&compiler, &vk, &public_inputs, &witness);
        fold_proof.fold_commitments[0].label = "wrong_npo_validity_fold".into();

        let err = compiler
            .verify_terminal_npo_validity_consistency_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &validity_commitment,
                &fold_proof,
                &consistency_proof,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityFoldCommitmentLabelMismatch {
                round: 0,
                expected: NativeTerminalCompiler::npo_validity_fold_oracle_label(0),
                got: "wrong_npo_validity_fold".into(),
            }
        );
    }

    #[test]
    fn goldilocks_terminal_npo_validity_consistency_rejects_nonzero_validity_row() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let mut bad_validity =
            vec![
                Goldilocks::ZERO;
                NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
            ];
        bad_validity[0] = Goldilocks::ONE;
        let bad_validity_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::npo_validity_oracle_label(),
            &bad_validity,
        )
        .expect("bad NPO validity oracle must commit");
        let bad_validity_commitment = bad_validity_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root, bad_validity_commitment.root],
            )
            .expect("prelude must bind bad NPO validity commitment");
        let fold_proof = npo_validity_fold_proof_for_values(
            &compiler,
            &prelude,
            &bad_validity_commitment,
            &bad_validity,
        );
        let plan = compiler
            .derive_terminal_npo_validity_fold_query_plan(
                &prelude,
                &bad_validity_commitment,
                &fold_proof.fold_commitments,
            )
            .expect("NPO validity plan must derive");
        assert_eq!(plan.indices.len(), 15);
        let consistency_proof = compiler
            .prove_terminal_npo_validity_consistency_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_tree,
                &bad_validity_tree,
                &fold_proof,
                &witness,
            )
            .expect("honest NPO consistency proof must build");

        let err = compiler
            .verify_terminal_npo_validity_consistency_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &bad_validity_commitment,
                &fold_proof,
                &consistency_proof,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityFoldFinalValueNonZero
        );
    }

    #[test]
    fn goldilocks_terminal_npo_validity_fold_rejects_nonzero_final_value() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let mut bad_validity =
            vec![
                Goldilocks::ZERO;
                NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
            ];
        bad_validity[0] = Goldilocks::ONE;
        let bad_validity_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::npo_validity_oracle_label(),
            &bad_validity,
        )
        .expect("bad NPO validity oracle must commit");
        let bad_validity_commitment = bad_validity_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![bad_validity_commitment.root],
            )
            .expect("prelude must bind bad NPO validity commitment");
        let proof = npo_validity_fold_proof_for_values(
            &compiler,
            &prelude,
            &bad_validity_commitment,
            &bad_validity,
        );

        let err = compiler
            .verify_terminal_npo_validity_fold_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &bad_validity_commitment,
                &proof,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityFoldFinalValueNonZero
        );
    }

    #[test]
    fn goldilocks_terminal_npo_relation_projects_tip5_rows() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);
        let relation = vk.npo_relation();

        assert_eq!(relation.rows.len(), 2);
        assert_eq!(relation.tip5_rows(), 2);
        assert_eq!(relation.recompose_rows(), 0);
        assert_eq!(relation.recompose_coeff_rows(), 0);
        for (idx, row) in relation.rows.iter().enumerate() {
            assert_eq!(row.npo_index, idx);
            assert_eq!(row.local_row, idx);
            assert_eq!(row.kind, TerminalNpoRowKind::Tip5Goldilocks);
            assert_eq!(row.op_type, "tip5_perm/goldilocks_w16_r5");
            assert_eq!(row.callsite.inputs.len(), 16);
            assert_eq!(row.callsite.outputs.len(), 10);
        }
        compiler
            .verify_npo_relation_goldilocks(&vk, &relation, &witness)
            .expect("projected Tip5 NPO relation must verify");

        let mut truncated = relation.clone();
        truncated.rows.pop();
        let err = compiler
            .verify_npo_relation_goldilocks(&vk, &truncated, &witness)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::TerminalNpoRelationMismatch {
                expected_rows: 2,
                got_rows: 1,
            }
        );
    }

    #[test]
    fn goldilocks_terminal_npo_relation_projects_recompose_rows() {
        let (circuit, public_inputs) = build_recompose_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_recompose_terminal_witness(&circuit, public_inputs);
        let relation = vk.npo_relation();

        assert_eq!(relation.rows.len(), 2);
        assert_eq!(relation.tip5_rows(), 0);
        assert_eq!(relation.recompose_rows(), 1);
        assert_eq!(relation.recompose_coeff_rows(), 1);
        assert_eq!(relation.rows[0].kind, TerminalNpoRowKind::Recompose);
        assert_eq!(relation.rows[0].op_type, "recompose");
        assert_eq!(
            relation.rows[1].kind,
            TerminalNpoRowKind::RecomposeWithCoeffLookups
        );
        assert_eq!(relation.rows[1].op_type, "recompose/coeff");
        compiler
            .verify_npo_relation_goldilocks(&vk, &relation, &witness)
            .expect("projected recompose NPO relation must verify");
    }

    #[test]
    fn goldilocks_terminal_npo_row_evaluations_report_tip5_residuals() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);
        let witness_values = witness_values(&witness);
        let relation = vk.npo_relation();
        let row = relation
            .rows
            .iter()
            .find(|row| row.kind == TerminalNpoRowKind::Tip5Goldilocks)
            .expect("Tip5 NPO row must be projected");
        let expected_witness_ids = NativeTerminalCompiler::npo_callsite_witness_ids(&row.callsite);
        let opened_values = expected_witness_ids
            .iter()
            .map(|witness_id| witness_values[witness_id.0 as usize])
            .collect::<Vec<_>>();

        let evaluation = compiler
            .evaluate_sampled_tip5_npo_row_values::<Goldilocks>(
                row.npo_index,
                row.local_row,
                &row.callsite,
                &[],
                &expected_witness_ids,
                &opened_values,
            )
            .expect("honest Tip5 row must evaluate");
        assert!(evaluation.is_satisfied());

        let output_witness_id = row
            .callsite
            .outputs
            .iter()
            .copied()
            .flatten()
            .next()
            .expect("Tip5 row must bind an output witness");
        let output_pos = expected_witness_ids
            .iter()
            .position(|witness_id| *witness_id == output_witness_id)
            .expect("output witness must be opened");
        let mut bad_opened_values = opened_values;
        bad_opened_values[output_pos] += Goldilocks::ONE;
        let evaluation = compiler
            .evaluate_sampled_tip5_npo_row_values::<Goldilocks>(
                row.npo_index,
                row.local_row,
                &row.callsite,
                &[],
                &expected_witness_ids,
                &bad_opened_values,
            )
            .expect("tampered Tip5 row still has a well-formed evaluation");
        let residual = evaluation
            .first_nonzero()
            .expect("tampered Tip5 output must produce a residual");
        assert_eq!(residual.kind, TerminalNpoResidualKind::Tip5Output);
        assert!(!residual.is_zero());
    }

    #[test]
    fn goldilocks_terminal_npo_row_evaluations_report_recompose_residuals() {
        let (circuit, public_inputs) = build_recompose_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_recompose_terminal_witness(&circuit, public_inputs);
        let witness_values = witness_values(&witness);
        let relation = vk.npo_relation();
        let row = relation
            .rows
            .iter()
            .find(|row| row.kind == TerminalNpoRowKind::Recompose)
            .expect("recompose NPO row must be projected");
        let expected_witness_ids = NativeTerminalCompiler::npo_callsite_witness_ids(&row.callsite);
        let opened_values = expected_witness_ids
            .iter()
            .map(|witness_id| witness_values[witness_id.0 as usize])
            .collect::<Vec<_>>();

        let evaluation = compiler
            .evaluate_sampled_recompose_npo_row_values::<GoldilocksD2>(
                row.npo_index,
                &row.op_type,
                row.local_row,
                &row.callsite,
                &[],
                &expected_witness_ids,
                &opened_values,
            )
            .expect("honest recompose row must evaluate");
        assert!(evaluation.is_satisfied());

        let output_witness_id = row
            .callsite
            .outputs
            .first()
            .copied()
            .flatten()
            .expect("recompose row must bind an output witness");
        let output_pos = expected_witness_ids
            .iter()
            .position(|witness_id| *witness_id == output_witness_id)
            .expect("output witness must be opened");
        let mut bad_opened_values = opened_values;
        bad_opened_values[output_pos] += GoldilocksD2::ONE;
        let evaluation = compiler
            .evaluate_sampled_recompose_npo_row_values::<GoldilocksD2>(
                row.npo_index,
                &row.op_type,
                row.local_row,
                &row.callsite,
                &[],
                &expected_witness_ids,
                &bad_opened_values,
            )
            .expect("tampered recompose row still has a well-formed evaluation");
        let residual = evaluation
            .first_nonzero()
            .expect("tampered recompose output must produce a residual");
        assert_eq!(residual.kind, TerminalNpoResidualKind::RecomposeOutput);
        assert!(!residual.is_zero());
    }

    #[test]
    fn goldilocks_terminal_npo_validity_values_are_row_residuals() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);
        let honest_values = compiler
            .terminal_npo_validity_values_goldilocks(&vk, &witness)
            .expect("honest NPO validity values must compute");
        assert_eq!(
            honest_values,
            vec![
                Goldilocks::ZERO;
                NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
            ]
        );

        let output_wids = vk
            .constraints
            .iter()
            .find_map(|constraint| match constraint {
                NativeTerminalConstraint::Tip5Goldilocks { callsites, .. } => Some(
                    callsites
                        .first()?
                        .outputs
                        .iter()
                        .copied()
                        .flatten()
                        .take(2)
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .expect("Tip5 output witnesses must be bound");
        assert_eq!(output_wids.len(), 2);
        let mut bad_witness_values = witness_values(&witness);
        for output_wid in &output_wids {
            bad_witness_values[output_wid.0 as usize] += Goldilocks::ONE;
        }
        let mut bad_traces = witness.traces.clone();
        bad_traces.witness_trace = WitnessTrace::new(bad_witness_values);
        let bad_witness = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: bad_traces,
        };
        let bad_values = compiler
            .terminal_npo_validity_values_goldilocks(&vk, &bad_witness)
            .expect("tampered NPO validity values must compute");
        assert_eq!(
            bad_values.len(),
            NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
        );
        assert_eq!(
            bad_values
                .iter()
                .filter(|value| **value != Goldilocks::ZERO)
                .count(),
            2,
            "independent residual components must not be row-compressed"
        );
    }

    #[test]
    fn goldilocks_terminal_npo_validity_consistency_rejects_stale_zero_residual() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let output_wid = vk
            .constraints
            .iter()
            .find_map(|constraint| match constraint {
                NativeTerminalConstraint::Tip5Goldilocks { callsites, .. } => {
                    callsites.first()?.outputs.first().copied().flatten()
                }
                _ => None,
            })
            .expect("Tip5 output witness must be bound");
        let mut bad_witness_values = witness_values(&witness);
        bad_witness_values[output_wid.0 as usize] += Goldilocks::ONE;
        let witness_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::witness_oracle_label(),
            &bad_witness_values,
        )
        .expect("bad witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let mut bad_traces = witness.traces.clone();
        bad_traces.witness_trace = WitnessTrace::new(bad_witness_values);
        let bad_witness = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: bad_traces,
        };
        let stale_validity =
            vec![
                Goldilocks::ZERO;
                NativeTerminalCompiler::terminal_npo_validity_domain_len::<Goldilocks>(&vk)
            ];
        let stale_validity_tree = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::npo_validity_oracle_label(),
            &stale_validity,
        )
        .expect("stale zero NPO validity oracle must commit");
        let stale_validity_commitment = stale_validity_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root, stale_validity_commitment.root],
            )
            .expect("prelude must bind stale zero validity oracle");
        let fold_proof = npo_validity_fold_proof_for_values(
            &compiler,
            &prelude,
            &stale_validity_commitment,
            &stale_validity,
        );
        let consistency_proof = compiler
            .prove_terminal_npo_validity_consistency_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_tree,
                &stale_validity_tree,
                &fold_proof,
                &bad_witness,
            )
            .expect("stale zero consistency openings must build");

        let err = compiler
            .verify_terminal_npo_validity_consistency_goldilocks::<Goldilocks>(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &stale_validity_commitment,
                &fold_proof,
                &consistency_proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoValidityResidualMismatch { npo_index: 0, .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_queries_verify_tip5_rows() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root],
            )
            .expect("prelude must bind witness commitment");

        let proof = compiler
            .prove_terminal_npo_queries_goldilocks(&vk, &prelude, &witness_tree, &witness)
            .expect("NPO query proof must build");
        let plan = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &proof,
            )
            .expect("NPO query proof must verify");
        assert_eq!(plan.domain_len, 2);
        assert_eq!(
            proof.openings.len(),
            TerminalProofParameters::production_60bit().num_queries as usize
        );
        assert!(
            proof
                .openings
                .iter()
                .all(|opening| opening.tip5_hidden_input_values.is_empty()),
            "fully witness-bound Tip5 rows should not serialize redundant input snapshots"
        );

        let mut steered = proof.clone();
        steered.openings[0].npo_index = (steered.openings[0].npo_index + 1) % plan.domain_len;
        let err = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &steered,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoQueryIndexMismatch { .. }
        ));

        let mut missing_opening = proof.clone();
        missing_opening.openings[0].witness_openings.pop();
        let err = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &missing_opening,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoOpeningCountMismatch { .. }
        ));

        let mut unexpected_hidden = proof.clone();
        unexpected_hidden.openings[0]
            .tip5_hidden_input_values
            .push(TerminalTip5HiddenInputValue {
                limb: 0,
                value_basis: vec![0],
            });
        let err = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &unexpected_hidden,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoTip5InputValueLength { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_queries_reject_bad_committed_tip5_output() {
        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs.clone());
        let output_wid = vk
            .constraints
            .iter()
            .find_map(|constraint| match constraint {
                NativeTerminalConstraint::Tip5Goldilocks { callsites, .. } => {
                    callsites.first()?.outputs.first().copied().flatten()
                }
                _ => None,
            })
            .expect("Tip5 output witness must be bound");
        let mut bad_witness_values = witness_values(&witness);
        bad_witness_values[output_wid.0 as usize] += Goldilocks::ONE;
        let bad_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &bad_witness_values)
                .expect("bad witness oracle still commits");
        let bad_commitment = bad_tree.commitment();
        let mut bad_traces = witness.traces.clone();
        bad_traces.witness_trace = WitnessTrace::new(bad_witness_values);
        let bad_witness = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: bad_traces,
        };
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![bad_commitment.root],
            )
            .expect("prelude must bind bad witness commitment");
        let proof = compiler
            .prove_terminal_npo_queries_goldilocks(&vk, &prelude, &bad_tree, &bad_witness)
            .expect("proof can be built for committed bad witness");
        let err = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &bad_commitment,
                &proof,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::Tip5OutputMismatch { row: 0, limb: 0 }
        );
    }

    #[test]
    fn goldilocks_terminal_npo_queries_verify_recompose_rows() {
        let (circuit, public_inputs) = build_recompose_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_recompose_terminal_witness(&circuit, public_inputs.clone());
        let witness_values = witness_values(&witness);
        let witness_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &witness_values)
                .expect("witness oracle must commit");
        let witness_commitment = witness_tree.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root],
            )
            .expect("prelude must bind witness commitment");

        let proof = compiler
            .prove_terminal_npo_queries_goldilocks(&vk, &prelude, &witness_tree, &witness)
            .expect("NPO query proof must build");
        let plan = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &proof,
            )
            .expect("recompose NPO query proof must verify");
        assert_eq!(plan.domain_len, 2);

        let mut bad_shape = proof.clone();
        bad_shape.openings[0]
            .tip5_hidden_input_values
            .push(TerminalTip5HiddenInputValue {
                limb: 0,
                value_basis: vec![0],
            });
        let err = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &witness_commitment,
                &bad_shape,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::TerminalNpoTip5InputValueLength { .. }
        ));
    }

    #[test]
    fn goldilocks_terminal_npo_queries_reject_bad_committed_recompose_output() {
        let (circuit, public_inputs) = build_recompose_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_recompose_terminal_witness(&circuit, public_inputs.clone());
        let output_wids: Vec<_> = vk
            .constraints
            .iter()
            .filter_map(|constraint| match constraint {
                NativeTerminalConstraint::RecomposeGoldilocks { callsites, .. } => {
                    callsites.first()?.outputs.first().copied().flatten()
                }
                _ => None,
            })
            .collect();
        assert!(!output_wids.is_empty());

        let mut bad_witness_values = witness_values(&witness);
        for output_wid in output_wids {
            bad_witness_values[output_wid.0 as usize] += GoldilocksD2::ONE;
        }
        let bad_tree =
            TerminalOracleMerkleTree::commit_goldilocks_values("witness", &bad_witness_values)
                .expect("bad witness oracle still commits");
        let bad_commitment = bad_tree.commitment();
        let mut bad_traces = witness.traces.clone();
        bad_traces.witness_trace = WitnessTrace::new(bad_witness_values);
        let bad_witness = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: bad_traces,
        };
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                &vk,
                &public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![bad_commitment.root],
            )
            .expect("prelude must bind bad witness commitment");
        let proof = compiler
            .prove_terminal_npo_queries_goldilocks(&vk, &prelude, &bad_tree, &bad_witness)
            .expect("proof can be built for committed bad witness");
        let err = compiler
            .verify_terminal_npo_queries_goldilocks(
                &vk,
                &public_inputs,
                &prelude,
                &bad_commitment,
                &proof,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::RecomposeOutputMismatch { .. }
        ));
    }

    #[test]
    fn recursive_tip5_terminal_relation_is_5_round_and_tamper_checked() {
        assert_eq!(Tip5Config::GOLDILOCKS_W16.num_rounds(), 5);
        assert_eq!(NUM_ROUNDS, 5);
        assert_eq!(
            Tip5Config::GOLDILOCKS_W16.variant_name(),
            "goldilocks_w16_r5"
        );

        let (circuit, public_inputs) = build_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);

        assert!(vk.constraints.iter().any(|constraint| matches!(
            constraint,
            NativeTerminalConstraint::Tip5Goldilocks {
                expected_rows: 1,
                ..
            }
        )));
        compiler
            .verify_assignment_with_tip5_goldilocks(&vk, &witness)
            .expect("honest 5-round Tip5 assignment must verify");

        let op_type = NpoTypeId::tip5_perm(Tip5Config::GOLDILOCKS_W16);
        let mut stale_mode_vk = vk.clone();
        let NativeTerminalConstraint::Tip5Goldilocks { callsites, .. } = stale_mode_vk
            .constraints
            .iter_mut()
            .find(|constraint| {
                matches!(constraint, NativeTerminalConstraint::Tip5Goldilocks { .. })
            })
            .expect("Tip5 constraint must be present")
        else {
            unreachable!("matched Tip5 constraint");
        };
        let mode = callsites[0]
            .tip5_mode
            .as_mut()
            .expect("Tip5 callsite must carry terminal mode");
        mode.new_start = !mode.new_start;
        stale_mode_vk.header.relation_digest = Some(
            NativeTerminalCompiler::relation_digest_goldilocks(&stale_mode_vk),
        );
        let err = compiler
            .verify_assignment_with_tip5_goldilocks(&stale_mode_vk, &witness)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                op_type: op_type.as_str().into(),
                row: 0,
                field: "new_start",
                limb: 0,
                expected: Some(0),
                got: Some(1),
            }
        );

        let mut tampered = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let trace = tampered
            .traces
            .non_primitive_traces
            .get(&op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<Tip5Trace<Goldilocks>>())
            .expect("Tip5 trace must be present")
            .clone();
        let mut trace = trace;
        trace.operations[0].input_values[0] += Goldilocks::ONE;
        tampered
            .traces
            .non_primitive_traces
            .insert(op_type.clone(), Box::new(trace));

        let err = compiler
            .verify_assignment_with_tip5_goldilocks(&vk, &tampered)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::Tip5InputMismatch { row: 0, limb: 0 }
        );

        let mut missing_row = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let trace = missing_row
            .traces
            .non_primitive_traces
            .get(&op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<Tip5Trace<Goldilocks>>())
            .expect("Tip5 trace must be present")
            .clone();
        let mut trace = trace;
        trace.operations.clear();
        missing_row
            .traces
            .non_primitive_traces
            .insert(op_type.clone(), Box::new(trace));
        let err = compiler
            .verify_assignment_with_tip5_goldilocks(&vk, &missing_row)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                op_type: op_type.as_str().into(),
                expected: 1,
                got: 0,
            }
        );

        let mut extra_row = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let trace = extra_row
            .traces
            .non_primitive_traces
            .get(&op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<Tip5Trace<Goldilocks>>())
            .expect("Tip5 trace must be present")
            .clone();
        let mut trace = trace;
        trace.operations.push(trace.operations[0].clone());
        extra_row
            .traces
            .non_primitive_traces
            .insert(op_type.clone(), Box::new(trace));
        let err = compiler
            .verify_assignment_with_tip5_goldilocks(&vk, &extra_row)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                op_type: op_type.as_str().into(),
                expected: 1,
                got: 2,
            }
        );

        let mut unexpected_table = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let trace = unexpected_table
            .traces
            .non_primitive_traces
            .get(&op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<Tip5Trace<Goldilocks>>())
            .expect("Tip5 trace must be present")
            .clone();
        let unexpected_op_type = NpoTypeId::new("unexpected/terminal-npo");
        unexpected_table
            .traces
            .non_primitive_traces
            .insert(unexpected_op_type.clone(), Box::new(trace));
        let err = compiler
            .verify_assignment_with_tip5_goldilocks(&vk, &unexpected_table)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::UnexpectedNonPrimitiveTrace {
                op_type: unexpected_op_type.as_str().into(),
            }
        );
    }

    #[test]
    fn goldilocks_terminal_exhaustive_tip5_checks_hidden_chain_and_merkle_lanes() {
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let mut previous_input = [Goldilocks::ZERO; 16];
        for (limb, value) in previous_input.iter_mut().enumerate() {
            *value = Goldilocks::from_u64(0x700 + limb as u64);
        }
        let mut previous_output = previous_input;
        Tip5Perm.permute_mut(&mut previous_output);
        let chain_callsite = NativeTerminalNpoCallsite {
            inputs: vec![None; 16],
            outputs: Vec::new(),
            tip5_mode: Some(Tip5TerminalMode {
                new_start: false,
                merkle_path: false,
            }),
            tip5_mmcs_bit: None,
        };
        let mut chain_hidden = previous_output
            .iter()
            .enumerate()
            .map(|(limb, value)| TerminalTip5HiddenInputValue {
                limb,
                value_basis: vec![value.as_canonical_u64()],
            })
            .collect::<Vec<_>>();
        let mut normal_state = Some(previous_output);
        let mut merkle_state = None;
        compiler
            .verify_exhaustive_tip5_npo_row_values::<Goldilocks>(
                0,
                1,
                &chain_callsite,
                &chain_hidden,
                &[],
                &[],
                &mut normal_state,
                &mut merkle_state,
            )
            .expect("normal chained hidden state must match previous output");
        chain_hidden[3].value_basis[0] ^= 1;
        let mut normal_state = Some(previous_output);
        let mut merkle_state = None;
        let evaluation = compiler
            .evaluate_exhaustive_tip5_npo_row_values::<Goldilocks>(
                0,
                1,
                &chain_callsite,
                &chain_hidden,
                &[],
                &[],
                &mut normal_state,
                &mut merkle_state,
            )
            .expect("tampered chained row must still evaluate");
        let residual = evaluation
            .first_nonzero()
            .expect("tampered chained hidden lane must produce a residual");
        assert_eq!(residual.kind, TerminalNpoResidualKind::Tip5ChainInput);
        assert_eq!(residual.limb, 3);
        let mut normal_state = Some(previous_output);
        let mut merkle_state = None;
        let err = compiler
            .verify_exhaustive_tip5_npo_row_values::<Goldilocks>(
                0,
                1,
                &chain_callsite,
                &chain_hidden,
                &[],
                &[],
                &mut normal_state,
                &mut merkle_state,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::Tip5InputMismatch { row: 1, limb: 3 }
        );

        let merkle_callsite = NativeTerminalNpoCallsite {
            inputs: vec![None; 16],
            outputs: Vec::new(),
            tip5_mode: Some(Tip5TerminalMode {
                new_start: true,
                merkle_path: true,
            }),
            tip5_mmcs_bit: Some(WitnessId(100)),
        };
        let merkle_hidden = (0..16)
            .map(|limb| TerminalTip5HiddenInputValue {
                limb,
                value_basis: vec![if (5..10).contains(&limb) {
                    0x900 + limb as u64
                } else {
                    0
                }],
            })
            .collect::<Vec<_>>();
        let mut normal_state = None;
        let mut merkle_state = None;
        compiler
            .verify_exhaustive_tip5_npo_row_values::<Goldilocks>(
                0,
                0,
                &merkle_callsite,
                &merkle_hidden,
                &[WitnessId(100)],
                &[Goldilocks::ZERO],
                &mut normal_state,
                &mut merkle_state,
            )
            .expect("fresh Merkle row may carry sibling limbs and zero capacity");

        let mut bad_zero_lane = merkle_hidden.clone();
        bad_zero_lane[12].value_basis[0] = 1;
        let mut normal_state = None;
        let mut merkle_state = None;
        let evaluation = compiler
            .evaluate_exhaustive_tip5_npo_row_values::<Goldilocks>(
                0,
                0,
                &merkle_callsite,
                &merkle_hidden,
                &[WitnessId(100)],
                &[Goldilocks::from_u64(2)],
                &mut normal_state,
                &mut merkle_state,
            )
            .expect("nonboolean MMCS bit must still evaluate");
        let residual = evaluation
            .first_nonzero()
            .expect("nonboolean MMCS bit must produce a residual");
        assert_eq!(residual.kind, TerminalNpoResidualKind::Tip5MmcsBit);
        assert_eq!(residual.limb, 16);
        let mut normal_state = None;
        let mut merkle_state = None;
        let err = compiler
            .verify_exhaustive_tip5_npo_row_values::<Goldilocks>(
                0,
                0,
                &merkle_callsite,
                &bad_zero_lane,
                &[WitnessId(100)],
                &[Goldilocks::ZERO],
                &mut normal_state,
                &mut merkle_state,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::Tip5InputMismatch { row: 0, limb: 12 }
        );

        let mut normal_state = None;
        let mut merkle_state = None;
        let err = compiler
            .verify_exhaustive_tip5_npo_row_values::<Goldilocks>(
                0,
                0,
                &merkle_callsite,
                &merkle_hidden,
                &[WitnessId(100)],
                &[Goldilocks::from_u64(2)],
                &mut normal_state,
                &mut merkle_state,
            )
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::Tip5InputMismatch { row: 0, limb: 16 }
        );
    }

    #[test]
    fn recursive_tip5_terminal_relation_binds_each_callsite() {
        let (circuit, public_inputs) = build_two_tip5_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_tip5_terminal_witness(&circuit, public_inputs);
        let op_type = NpoTypeId::tip5_perm(Tip5Config::GOLDILOCKS_W16);

        assert!(vk.constraints.iter().any(|constraint| matches!(
            constraint,
            NativeTerminalConstraint::Tip5Goldilocks {
                expected_rows: 2,
                ..
            }
        )));
        compiler
            .verify_assignment_with_tip5_goldilocks(&vk, &witness)
            .expect("honest two-call Tip5 assignment must verify");

        let mut duplicated_first_callsite = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let trace = duplicated_first_callsite
            .traces
            .non_primitive_traces
            .get(&op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<Tip5Trace<Goldilocks>>())
            .expect("Tip5 trace must be present")
            .clone();
        let mut trace = trace;
        trace.operations[1] = trace.operations[0].clone();
        duplicated_first_callsite
            .traces
            .non_primitive_traces
            .insert(op_type, Box::new(trace));
        let err = compiler
            .verify_assignment_with_tip5_goldilocks(&vk, &duplicated_first_callsite)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch { row: 1, .. }
        ));
    }

    #[test]
    fn recursive_recompose_terminal_relation_checks_standard_and_coeff_tables() {
        let (circuit, public_inputs) = build_recompose_test_circuit();
        let compiler = NativeTerminalCompiler::new("nock-terminal-v0", 60);
        let (_pk, vk) = compiler.compile_goldilocks_terminal(&circuit).unwrap();
        let witness = execute_recompose_terminal_witness(&circuit, public_inputs);

        assert!(vk.constraints.iter().any(|constraint| matches!(
            constraint,
            NativeTerminalConstraint::RecomposeGoldilocks {
                op_type,
                expected_rows: 1,
                ..
            } if op_type == NpoTypeId::recompose().as_str()
        )));
        assert!(vk.constraints.iter().any(|constraint| matches!(
            constraint,
            NativeTerminalConstraint::RecomposeGoldilocks {
                op_type,
                expected_rows: 1,
                ..
            } if op_type == NpoTypeId::recompose_with_coeff_lookups().as_str()
        )));
        compiler
            .verify_assignment_with_goldilocks_npos(&vk, &witness)
            .expect("honest recompose assignment must verify");

        let mut tampered_input = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let std_op_type = NpoTypeId::recompose();
        let std_trace = tampered_input
            .traces
            .non_primitive_traces
            .get(&std_op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<RecomposeTrace<Goldilocks>>())
            .expect("standard recompose trace must be present")
            .clone();
        let mut std_trace = std_trace;
        std_trace.operations[0].values[0] += Goldilocks::ONE;
        tampered_input
            .traces
            .non_primitive_traces
            .insert(std_op_type.clone(), Box::new(std_trace));
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&vk, &tampered_input)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::RecomposeInputMismatch { row: 0, limb: 0 }
        );

        let mut tampered_output = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let coeff_op_type = NpoTypeId::recompose_with_coeff_lookups();
        let coeff_trace = tampered_output
            .traces
            .non_primitive_traces
            .get(&coeff_op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<RecomposeTrace<Goldilocks>>())
            .expect("coeff recompose trace must be present")
            .clone();
        let mut coeff_trace = coeff_trace;
        coeff_trace.operations[0].output_wid = coeff_trace.operations[0].input_wids[0];
        tampered_output
            .traces
            .non_primitive_traces
            .insert(coeff_op_type, Box::new(coeff_trace));
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&vk, &tampered_output)
            .unwrap_err();
        assert!(matches!(
            err,
            NativeTerminalVerifyError::NonPrimitiveCallsiteMismatch {
                row: 0,
                field: "output_wid",
                ..
            }
        ));

        let mut missing_row = TerminalWitness {
            fingerprint: witness.fingerprint,
            public_inputs: witness.public_inputs.clone(),
            private_inputs: witness.private_inputs.clone(),
            traces: witness.traces.clone(),
        };
        let missing_trace = missing_row
            .traces
            .non_primitive_traces
            .get(&std_op_type)
            .and_then(|trace| trace.as_any().downcast_ref::<RecomposeTrace<Goldilocks>>())
            .expect("standard recompose trace must be present")
            .clone();
        let mut missing_trace = missing_trace;
        missing_trace.operations.clear();
        missing_row
            .traces
            .non_primitive_traces
            .insert(std_op_type.clone(), Box::new(missing_trace));
        let err = compiler
            .verify_assignment_with_goldilocks_npos(&vk, &missing_row)
            .unwrap_err();
        assert_eq!(
            err,
            NativeTerminalVerifyError::NonPrimitiveTraceRowCount {
                op_type: std_op_type.as_str().into(),
                expected: 1,
                got: 0,
            }
        );
    }

    fn build_primitive_test_circuit() -> (Circuit<BabyBear>, Vec<BabyBear>, usize) {
        let mut builder = CircuitBuilder::<BabyBear>::new();
        let x = builder.public_input();
        let y = builder.public_input();
        let expected = builder.public_input();
        let c5 = builder.define_const(BabyBear::from_u64(5));
        let mul = builder.mul(x, y);
        let sum = builder.add(x, mul);
        let sum = builder.add(sum, c5);
        let diff = builder.sub(sum, expected);
        builder.assert_zero(diff);
        let circuit = builder.build().unwrap();

        let x_val = BabyBear::from_u64(7);
        let y_val = BabyBear::from_u64(3);
        let expected_val = x_val + x_val * y_val + BabyBear::from_u64(5);
        let public_inputs = vec![x_val, y_val, expected_val];
        let expected_public_len = public_inputs.len();
        (circuit, public_inputs, expected_public_len)
    }

    fn build_private_input_test_circuit() -> (Circuit<BabyBear>, Vec<BabyBear>, Vec<BabyBear>) {
        let mut builder = CircuitBuilder::<BabyBear>::new();
        let private = builder.alloc_private_input("secret");
        let expected = builder.public_input();
        let diff = builder.sub(private, expected);
        builder.assert_zero(diff);
        let circuit = builder.build().unwrap();

        let value = BabyBear::from_u64(11);
        (circuit, vec![value], vec![value])
    }

    fn build_tip5_test_circuit() -> (Circuit<Goldilocks>, Vec<Goldilocks>) {
        let mut builder = CircuitBuilder::<Goldilocks>::new();
        builder.enable_tip5_perm::<Tip5Goldilocks, _>(
            generate_tip5_trace::<Goldilocks, Tip5Goldilocks>,
            Tip5Perm,
        );
        let inputs: [ExprId; 16] = core::array::from_fn(|_| builder.public_input());
        let expected_outputs: [ExprId; 10] = core::array::from_fn(|_| builder.public_input());
        let input_exprs = inputs.map(Some);
        let outputs = builder
            .add_tip5_perm(&Tip5PermCall {
                config: Tip5Config::GOLDILOCKS_W16,
                new_start: true,
                inputs: input_exprs,
                out_ctl: [true; 10],
                return_all_outputs: false,
            })
            .unwrap()
            .1;
        for (output, expected) in outputs.iter().take(10).zip(expected_outputs) {
            let diff = builder.sub(output.expect("rate output must be allocated"), expected);
            builder.assert_zero(diff);
        }
        let circuit = builder.build().unwrap();
        let input_state: [Goldilocks; 16] =
            core::array::from_fn(|i| Goldilocks::from_u64(0x100 + i as u64));
        let output_state = Tip5Perm.permute(input_state);
        let mut public_inputs = input_state.to_vec();
        public_inputs.extend_from_slice(&output_state[..10]);
        (circuit, public_inputs)
    }

    fn build_two_tip5_test_circuit() -> (Circuit<Goldilocks>, Vec<Goldilocks>) {
        let mut builder = CircuitBuilder::<Goldilocks>::new();
        builder.enable_tip5_perm::<Tip5Goldilocks, _>(
            generate_tip5_trace::<Goldilocks, Tip5Goldilocks>,
            Tip5Perm,
        );

        let inputs_a: [ExprId; 16] = core::array::from_fn(|_| builder.public_input());
        let expected_a: [ExprId; 10] = core::array::from_fn(|_| builder.public_input());
        let outputs_a = builder
            .add_tip5_perm(&Tip5PermCall {
                config: Tip5Config::GOLDILOCKS_W16,
                new_start: true,
                inputs: inputs_a.map(Some),
                out_ctl: [true; 10],
                return_all_outputs: false,
            })
            .unwrap()
            .1;
        for (output, expected) in outputs_a.iter().take(10).zip(expected_a) {
            let diff = builder.sub(output.expect("rate output must be allocated"), expected);
            builder.assert_zero(diff);
        }

        let inputs_b: [ExprId; 16] = core::array::from_fn(|_| builder.public_input());
        let expected_b: [ExprId; 10] = core::array::from_fn(|_| builder.public_input());
        let outputs_b = builder
            .add_tip5_perm(&Tip5PermCall {
                config: Tip5Config::GOLDILOCKS_W16,
                new_start: true,
                inputs: inputs_b.map(Some),
                out_ctl: [true; 10],
                return_all_outputs: false,
            })
            .unwrap()
            .1;
        for (output, expected) in outputs_b.iter().take(10).zip(expected_b) {
            let diff = builder.sub(output.expect("rate output must be allocated"), expected);
            builder.assert_zero(diff);
        }

        let input_state_a: [Goldilocks; 16] =
            core::array::from_fn(|i| Goldilocks::from_u64(0x300 + i as u64));
        let output_state_a = Tip5Perm.permute(input_state_a);
        let input_state_b: [Goldilocks; 16] =
            core::array::from_fn(|i| Goldilocks::from_u64(0x500 + i as u64));
        let output_state_b = Tip5Perm.permute(input_state_b);

        let mut public_inputs = input_state_a.to_vec();
        public_inputs.extend_from_slice(&output_state_a[..10]);
        public_inputs.extend_from_slice(&input_state_b);
        public_inputs.extend_from_slice(&output_state_b[..10]);
        (builder.build().unwrap(), public_inputs)
    }

    fn build_many_tip5_test_circuit(rows: usize) -> (Circuit<Goldilocks>, Vec<Goldilocks>) {
        let mut builder = CircuitBuilder::<Goldilocks>::new();
        builder.enable_tip5_perm::<Tip5Goldilocks, _>(
            generate_tip5_trace::<Goldilocks, Tip5Goldilocks>,
            Tip5Perm,
        );

        let mut expected_public_inputs = Vec::with_capacity(rows * 26);
        for row in 0..rows {
            let inputs: [ExprId; 16] = core::array::from_fn(|_| builder.public_input());
            let expected: [ExprId; 10] = core::array::from_fn(|_| builder.public_input());
            let outputs = builder
                .add_tip5_perm(&Tip5PermCall {
                    config: Tip5Config::GOLDILOCKS_W16,
                    new_start: true,
                    inputs: inputs.map(Some),
                    out_ctl: [true; 10],
                    return_all_outputs: false,
                })
                .unwrap()
                .1;
            for (output, expected) in outputs.iter().take(10).zip(expected) {
                let diff = builder.sub(output.expect("rate output must be allocated"), expected);
                builder.assert_zero(diff);
            }

            let input_state: [Goldilocks; 16] =
                core::array::from_fn(|i| Goldilocks::from_u64(0x700 + (row * 16 + i) as u64));
            let output_state = Tip5Perm.permute(input_state);
            expected_public_inputs.extend_from_slice(&input_state);
            expected_public_inputs.extend_from_slice(&output_state[..10]);
        }

        (builder.build().unwrap(), expected_public_inputs)
    }

    fn build_many_merkle_tip5_test_circuit(
        rows: usize,
    ) -> (
        Circuit<Goldilocks>,
        Vec<Goldilocks>,
        Vec<(NonPrimitiveOpId, Vec<Goldilocks>)>,
    ) {
        let mut builder = CircuitBuilder::<Goldilocks>::new();
        builder.enable_tip5_perm::<Tip5Goldilocks, _>(
            generate_tip5_trace::<Goldilocks, Tip5Goldilocks>,
            Tip5Perm,
        );

        let mut public_inputs = Vec::with_capacity(rows * 11);
        let mut private_data = Vec::with_capacity(rows);
        for row in 0..rows {
            let digest_inputs: Vec<_> = (0..5).map(|_| builder.public_input()).collect();
            let expected_outputs: Vec<_> = (0..5).map(|_| builder.public_input()).collect();
            let mmcs_bit = builder.public_input();
            let mut inputs = vec![None; 16];
            for (slot, digest_input) in inputs.iter_mut().zip(digest_inputs.iter().copied()) {
                *slot = Some(digest_input);
            }
            let mut out_ctl = vec![false; 10];
            for expose in out_ctl.iter_mut().take(5) {
                *expose = true;
            }
            let (op_id, outputs) = builder
                .add_tip5_perm_mmcs(&Tip5PermCallMmcs {
                    config: Tip5Config::GOLDILOCKS_W16,
                    new_start: true,
                    merkle_path: true,
                    mmcs_bit: Some(mmcs_bit),
                    inputs,
                    out_ctl,
                    return_all_outputs: false,
                    mmcs_index_sum: None,
                })
                .unwrap();
            for (output, expected) in outputs.iter().take(5).zip(expected_outputs) {
                let diff = builder.sub(output.expect("digest output must be allocated"), expected);
                builder.assert_zero(diff);
            }

            let digest: [Goldilocks; 5] =
                core::array::from_fn(|i| Goldilocks::from_u64(0x900 + (row * 10 + i) as u64));
            let sibling: Vec<Goldilocks> = (0..5)
                .map(|i| Goldilocks::from_u64(0xA00 + (row * 10 + i) as u64))
                .collect();
            let mut state = [Goldilocks::ZERO; 16];
            state[..5].copy_from_slice(&digest);
            state[5..10].copy_from_slice(&sibling);
            let output = Tip5Perm.permute(state);

            public_inputs.extend_from_slice(&digest);
            public_inputs.extend_from_slice(&output[..5]);
            public_inputs.push(Goldilocks::ZERO);
            private_data.push((op_id, sibling));
        }

        (builder.build().unwrap(), public_inputs, private_data)
    }

    fn build_npo_only_tip5_test_circuit() -> (Circuit<Goldilocks>, Vec<Goldilocks>) {
        let mut builder = CircuitBuilder::<Goldilocks>::new();
        builder.enable_tip5_perm::<Tip5Goldilocks, _>(
            generate_tip5_trace::<Goldilocks, Tip5Goldilocks>,
            Tip5Perm,
        );
        builder
            .add_tip5_perm(&Tip5PermCall {
                config: Tip5Config::GOLDILOCKS_W16,
                new_start: true,
                inputs: [None; 16],
                out_ctl: [true; 10],
                return_all_outputs: false,
            })
            .expect("NPO-only Tip5 row must build");
        (builder.build().unwrap(), Vec::new())
    }

    fn build_goldilocks_const_circuit(value: u64) -> Circuit<Goldilocks> {
        let mut builder = CircuitBuilder::<Goldilocks>::new();
        let expected = builder.public_input();
        let c = builder.define_const(Goldilocks::from_u64(value));
        let diff = builder.sub(c, expected);
        builder.assert_zero(diff);
        builder.build().unwrap()
    }

    fn build_recompose_test_circuit() -> (Circuit<GoldilocksD2>, Vec<GoldilocksD2>) {
        let mut builder = CircuitBuilder::<GoldilocksD2>::new();
        builder
            .enable_recompose::<Goldilocks>(generate_recompose_trace::<Goldilocks, GoldilocksD2>);

        let std_c0 = builder.public_input();
        let std_c1 = builder.public_input();
        let std_expected = builder.public_input();
        let std_out = builder
            .recompose_base_coeffs_to_ext::<Goldilocks>(&[std_c0, std_c1])
            .unwrap();
        let diff = builder.sub(std_out, std_expected);
        builder.assert_zero(diff);

        let coeff_c0 = builder.public_input();
        let coeff_c1 = builder.public_input();
        let coeff_expected = builder.public_input();
        let coeff_out = builder
            .recompose_base_coeffs_to_ext_with_coeff_lookups::<Goldilocks>(&[coeff_c0, coeff_c1])
            .unwrap();
        let diff = builder.sub(coeff_out, coeff_expected);
        builder.assert_zero(diff);

        let std_values = [Goldilocks::from_u64(7), Goldilocks::from_u64(11)];
        let coeff_values = [Goldilocks::from_u64(13), Goldilocks::from_u64(17)];
        let std_expected_value = GoldilocksD2::from_basis_coefficients_slice(&std_values).unwrap();
        let coeff_expected_value =
            GoldilocksD2::from_basis_coefficients_slice(&coeff_values).unwrap();
        let public_inputs = vec![
            GoldilocksD2::from(std_values[0]),
            GoldilocksD2::from(std_values[1]),
            std_expected_value,
            GoldilocksD2::from(coeff_values[0]),
            GoldilocksD2::from(coeff_values[1]),
            coeff_expected_value,
        ];

        (builder.build().unwrap(), public_inputs)
    }

    fn build_many_recompose_test_circuit(
        rows: usize,
    ) -> (Circuit<GoldilocksD2>, Vec<GoldilocksD2>) {
        let mut builder = CircuitBuilder::<GoldilocksD2>::new();
        builder
            .enable_recompose::<Goldilocks>(generate_recompose_trace::<Goldilocks, GoldilocksD2>);

        let mut public_inputs = Vec::with_capacity(rows * 6);
        for row in 0..rows {
            let std_c0 = builder.public_input();
            let std_c1 = builder.public_input();
            let std_expected = builder.public_input();
            let std_out = builder
                .recompose_base_coeffs_to_ext::<Goldilocks>(&[std_c0, std_c1])
                .unwrap();
            let diff = builder.sub(std_out, std_expected);
            builder.assert_zero(diff);

            let coeff_c0 = builder.public_input();
            let coeff_c1 = builder.public_input();
            let coeff_expected = builder.public_input();
            let coeff_out = builder
                .recompose_base_coeffs_to_ext_with_coeff_lookups::<Goldilocks>(&[
                    coeff_c0, coeff_c1,
                ])
                .unwrap();
            let diff = builder.sub(coeff_out, coeff_expected);
            builder.assert_zero(diff);

            let offset = row as u64 * 8;
            let std_values = [
                Goldilocks::from_u64(7 + offset),
                Goldilocks::from_u64(11 + offset),
            ];
            let coeff_values = [
                Goldilocks::from_u64(13 + offset),
                Goldilocks::from_u64(17 + offset),
            ];
            let std_expected_value =
                GoldilocksD2::from_basis_coefficients_slice(&std_values).unwrap();
            let coeff_expected_value =
                GoldilocksD2::from_basis_coefficients_slice(&coeff_values).unwrap();
            public_inputs.extend_from_slice(&[
                GoldilocksD2::from(std_values[0]),
                GoldilocksD2::from(std_values[1]),
                std_expected_value,
                GoldilocksD2::from(coeff_values[0]),
                GoldilocksD2::from(coeff_values[1]),
                coeff_expected_value,
            ]);
        }

        (builder.build().unwrap(), public_inputs)
    }

    fn execute_terminal_witness(
        circuit: &Circuit<BabyBear>,
        public_inputs: Vec<BabyBear>,
    ) -> TerminalWitness<BabyBear> {
        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        let traces = runner.run().unwrap();
        TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(circuit),
            public_inputs,
            private_inputs: vec![],
            traces,
        }
    }

    fn execute_terminal_witness_with_private(
        circuit: &Circuit<BabyBear>,
        public_inputs: Vec<BabyBear>,
        private_inputs: Vec<BabyBear>,
    ) -> TerminalWitness<BabyBear> {
        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        runner.set_private_inputs(&private_inputs).unwrap();
        let traces = runner.run().unwrap();
        TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(circuit),
            public_inputs,
            private_inputs,
            traces,
        }
    }

    fn execute_tip5_terminal_witness(
        circuit: &Circuit<Goldilocks>,
        public_inputs: Vec<Goldilocks>,
    ) -> TerminalWitness<Goldilocks> {
        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        let traces = runner.run().unwrap();
        TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(circuit),
            public_inputs,
            private_inputs: vec![],
            traces,
        }
    }

    fn execute_tip5_terminal_witness_with_private_data(
        circuit: &Circuit<Goldilocks>,
        public_inputs: Vec<Goldilocks>,
        private_data: Vec<(NonPrimitiveOpId, Vec<Goldilocks>)>,
    ) -> TerminalWitness<Goldilocks> {
        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        for (op_id, sibling) in private_data {
            runner
                .set_private_data(op_id, NpoPrivateData::new(Tip5PermPrivateData { sibling }))
                .unwrap();
        }
        let traces = runner.run().unwrap();
        TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(circuit),
            public_inputs,
            private_inputs: vec![],
            traces,
        }
    }

    fn standalone_npo_validity_components(
        compiler: &NativeTerminalCompiler,
        vk: &NativeTerminalVerifyingKey<Goldilocks>,
        public_inputs: &[Goldilocks],
        witness: &TerminalWitness<Goldilocks>,
    ) -> (
        TerminalProofPrelude,
        TerminalOracleCommitment,
        TerminalOracleCommitment,
        TerminalNpoValidityFoldProof,
        TerminalNpoValidityConsistencyProof,
    ) {
        let witness_values = witness_values(witness);
        let witness_oracle = TerminalOracleMerkleTree::commit_goldilocks_values(
            NativeTerminalCompiler::witness_oracle_label(),
            &witness_values,
        )
        .expect("witness oracle must commit");
        let witness_commitment = witness_oracle.commitment();
        let validity_oracle = compiler
            .commit_terminal_npo_validity_goldilocks(vk, witness)
            .expect("NPO validity oracle must commit");
        let validity_commitment = validity_oracle.commitment();
        let prelude = compiler
            .build_proof_prelude_goldilocks(
                vk,
                public_inputs,
                TerminalProofParameters::production_60bit(),
                vec![witness_commitment.root, validity_commitment.root],
            )
            .expect("NPO validity prelude must build");
        let fold_proof = compiler
            .prove_terminal_npo_validity_fold_goldilocks(
                vk,
                public_inputs,
                &prelude,
                &validity_oracle,
                witness,
            )
            .expect("NPO validity fold proof must build");
        let consistency_proof = compiler
            .prove_terminal_npo_validity_consistency_goldilocks(
                vk,
                public_inputs,
                &prelude,
                &witness_oracle,
                &validity_oracle,
                &fold_proof,
                witness,
            )
            .expect("NPO validity consistency proof must build");
        (
            prelude,
            witness_commitment,
            validity_commitment,
            fold_proof,
            consistency_proof,
        )
    }

    fn npo_validity_fold_proof_for_values(
        compiler: &NativeTerminalCompiler,
        prelude: &TerminalProofPrelude,
        validity_commitment: &TerminalOracleCommitment,
        values: &[Goldilocks],
    ) -> TerminalNpoValidityFoldProof {
        let mut layers = vec![values.to_vec()];
        let mut trees = Vec::new();
        let mut fold_commitments = Vec::new();
        let mut round = 0usize;
        while layers.last().expect("base validity layer exists").len() > 1 {
            let challenge = NativeTerminalCompiler::derive_terminal_npo_validity_fold_challenge::<
                Goldilocks,
            >(prelude, validity_commitment, &fold_commitments, round)
            .expect("fold challenge must derive");
            let current = layers.last().expect("current validity layer exists");
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            for pair in current.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(Goldilocks::ZERO);
                next.push(left * (Goldilocks::ONE - challenge) + right * challenge);
            }
            let tree = TerminalOracleMerkleTree::commit_goldilocks_values(
                NativeTerminalCompiler::npo_validity_fold_oracle_label(round),
                &next,
            )
            .expect("fold layer must commit");
            fold_commitments.push(tree.commitment());
            trees.push(tree);
            layers.push(next);
            round += 1;
        }

        let final_value_basis =
            NativeTerminalCompiler::goldilocks_basis_u64(layers.last().unwrap().first().unwrap());
        let query_plan = compiler
            .derive_terminal_npo_validity_fold_query_plan(
                prelude,
                validity_commitment,
                &fold_commitments,
            )
            .expect("validity fold query plan must derive");
        let mut expected_round_indices = vec![Vec::new(); fold_commitments.len()];
        for initial_index in &query_plan.indices {
            let mut index = *initial_index;
            let mut current_len = validity_commitment.values_len;
            for round in 0..fold_commitments.len() {
                let pair_index = (index / 2) * 2;
                NativeTerminalCompiler::push_unique_usize(
                    &mut expected_round_indices[round],
                    pair_index,
                );
                if pair_index + 1 < current_len {
                    NativeTerminalCompiler::push_unique_usize(
                        &mut expected_round_indices[round],
                        pair_index + 1,
                    );
                }
                index /= 2;
                current_len = current_len.div_ceil(2);
            }
        }
        let mut round_openings = Vec::new();
        for (round, round_indices) in expected_round_indices.iter_mut().enumerate() {
            round_indices.sort_unstable();
            let tree = if round == 0 {
                None
            } else {
                Some(&trees[round - 1])
            };
            let values = &layers[round];
            let mut opening_values = Vec::new();
            for index in round_indices {
                opening_values.push((*index, &values[*index]));
            }
            let opening = if let Some(tree) = tree {
                tree.open_goldilocks_multi_values(&opening_values)
            } else {
                TerminalOracleMerkleTree::commit_goldilocks_values(
                    NativeTerminalCompiler::npo_validity_oracle_label(),
                    &layers[0],
                )
                .expect("base validity tree must commit")
                .open_goldilocks_multi_values(&opening_values)
            }
            .expect("fold opening must build");
            round_openings.push(opening);
        }
        let openings = query_plan
            .indices
            .iter()
            .map(|index| TerminalResidualFoldQueryOpening {
                initial_index: *index,
                rounds: Vec::new(),
            })
            .collect();
        TerminalNpoValidityFoldProof {
            fold_commitments,
            final_value_basis,
            round_openings,
            openings,
        }
    }

    fn execute_recompose_terminal_witness(
        circuit: &Circuit<GoldilocksD2>,
        public_inputs: Vec<GoldilocksD2>,
    ) -> TerminalWitness<GoldilocksD2> {
        let mut runner = circuit.runner();
        runner.set_public_inputs(&public_inputs).unwrap();
        let traces = runner.run().unwrap();
        TerminalWitness {
            fingerprint: TerminalCircuitFingerprint::from_circuit(circuit),
            public_inputs,
            private_inputs: vec![],
            traces,
        }
    }

    fn witness_values<F: Copy>(witness: &TerminalWitness<F>) -> Vec<F> {
        (0..witness.traces.witness_trace.num_rows())
            .map(|idx| {
                *witness
                    .traces
                    .witness_trace
                    .get_value(WitnessId(idx as u32))
                    .expect("witness value must exist")
            })
            .collect()
    }

    fn one_value_traces(value: Goldilocks) -> Traces<Goldilocks> {
        Traces {
            witness_trace: WitnessTrace::new(vec![value]),
            const_trace: p3_circuit::tables::ConstTrace {
                index: vec![WitnessId(0)],
                values: vec![value],
            },
            public_trace: p3_circuit::tables::PublicTrace {
                index: vec![],
                values: vec![],
            },
            alu_trace: p3_circuit::tables::AluTrace {
                op_kind: vec![AluOpKind::Add],
                values: vec![[Goldilocks::ZERO; 4]],
                indices: vec![[WitnessId(0); 4]],
            },
            non_primitive_traces: HashMap::new(),
            tag_to_witness: HashMap::new(),
        }
    }

    #[derive(Debug, Clone)]
    struct DummyNpo {
        op_type: NpoTypeId,
    }

    impl NonPrimitiveExecutor<BabyBear> for DummyNpo {
        fn execute(
            &self,
            _inputs: &[Vec<WitnessId>],
            _outputs: &[Vec<WitnessId>],
            _ctx: &mut ExecutionContext<'_, BabyBear>,
        ) -> Result<(), CircuitError> {
            Ok(())
        }

        fn op_type(&self) -> &NpoTypeId {
            &self.op_type
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn boxed(&self) -> Box<dyn NonPrimitiveExecutor<BabyBear>> {
            Box::new(self.clone())
        }
    }
}
