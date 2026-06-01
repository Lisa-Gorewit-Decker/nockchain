//! Structured noun encoder for the canonical recursive AI-PoW certificate.
//!
//! This module intentionally accepts the recursive certificate object, not
//! `MatmulProof` and not the raw Layer-0 `AiPowBatchProof`. Its verifier
//! boundary also runs the full-matmul statement precheck before recursive proof
//! reconstruction or verification.

use std::panic::{catch_unwind, AssertUnwindSafe};

use ai_pow::ncmn::{parse_ncmn_nonce, NonceFormatError};
use ai_pow::params::MatmulParams;
use ai_pow::zk_bridge::{
    verify_ai_pow_full_matmul_production_statement, zk_params_from_matmul, BridgeError,
    ZkPublicCommitments,
};
use ai_pow_zk::{CompositePublicInputs, ZkParams};
use nockapp::noun::slab::{CueError, NounSlab};
use nockapp::Bytes;
use nockvm::jets::bits::util::met;
use nockvm::noun::{Noun, NounAllocator, NounSpace, D, T};
use nockvm_macros::tas;
use serde::de::{
    self, DeserializeOwned, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};
use serde::ser::{self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeTuple};

const AI_POW_CERT_VERSION: u64 = 1;
const GOLDILOCKS_MODULUS: u64 = 0xffff_ffff_0000_0001;
const NCMN_NONCE_LEN: usize = ai_pow::ncmn::NCMN_NONCE_LEN;

#[derive(Debug, thiserror::Error)]
pub enum CertificateNounError {
    #[error("recursive certificate serialization: {0}")]
    Serialize(String),
    #[error("recursive certificate deserialization: {0}")]
    Deserialize(String),
    #[error("recursive certificate noun has invalid shape: {0}")]
    Shape(&'static str),
    #[error("recursive certificate proof-node is not canonical")]
    NonCanonicalProofNode,
    #[error("unsupported AI-PoW certificate version {0}")]
    UnsupportedVersion(u64),
    #[error("recursive certificate noun exceeds {0} limit")]
    LimitExceeded(&'static str),
    #[error("{tag} declares {declared} bytes but atom contains {actual} bytes")]
    PackedLengthMismatch {
        tag: &'static str,
        declared: usize,
        actual: usize,
    },
    #[error("recursive certificate noun has invalid proof-node tag {0}")]
    InvalidTag(u64),
    #[error("integer field {field} is out of range")]
    IntegerOutOfRange { field: &'static str },
    #[error("field element {field} is not canonical")]
    NonCanonicalField { field: &'static str },
    #[error("certificate ZK params do not match trusted AI-PoW params: expected {expected:?}, got {actual:?}")]
    ZkParamsMismatch {
        expected: ZkParams,
        actual: ZkParams,
    },
    #[error("NCMN nonce: {0}")]
    Nonce(#[from] NonceFormatError),
    #[error("NCMN nonce Nockchain commitment does not match candidate block")]
    NonceAnchorMismatch,
    #[error("NCMN external commitment is reserved and must be absent")]
    NonceExternalCommitmentPresent,
    #[error("certificate statement metadata is not bound to trusted block state: {0}")]
    Statement(#[from] BridgeError),
    #[error("recursive certificate verification failed: {0}")]
    RecursiveCertificate(String),
    #[error("jammed AI-PoW artifact is {actual} bytes, exceeding {limit} byte limit")]
    JammedLengthExceeded { limit: usize, actual: usize },
    #[error("jammed AI-PoW artifact cue failed: {0}")]
    Cue(#[from] CueError),
    #[error("jammed AI-PoW artifact cue panicked")]
    CuePanic,
    #[error("jammed AI-PoW artifact is not canonical jam")]
    NonCanonicalJam,
}

/// Resource limits for decoding the structured certificate noun.
///
/// These limits are deliberately independent of ZK verification: they bound
/// parser work before Hoon/Rust code walks a miner-controlled proof artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertificateNounLimits {
    pub max_depth: usize,
    pub max_total_nodes: usize,
    pub max_list_items: usize,
    pub max_atom_bytes: usize,
    pub max_packed_items: usize,
    /// Maximum jammed artifact bytes accepted before cueing attacker input.
    pub max_jam_bytes: usize,
}

impl Default for CertificateNounLimits {
    fn default() -> Self {
        Self {
            max_depth: 256,
            max_total_nodes: 1_000_000,
            max_list_items: 1_000_000,
            max_atom_bytes: 1 << 20,
            max_packed_items: 1_000_000,
            max_jam_bytes: 4 << 20,
        }
    }
}

/// Decoded top-level `ai-pow-certificate` shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPowCertificateShape {
    pub version: u64,
    pub zk_params: ZkParams,
    pub found_idx: u32,
    pub trace_height: usize,
    pub commitments: ZkPublicCommitments,
    pub public_inputs: CompositePublicInputs,
    pub certificate: AiProofNode,
}

/// Decoded `%ai-pow` block artifact shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPowArtifactShape {
    pub nonce: ai_pow::ncmn::NcmnNonce,
    pub certificate: AiPowCertificateShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AiPowCertificateMetadata {
    version: u64,
    zk_params: ZkParams,
    found_idx: u32,
    trace_height: usize,
    commitments: ZkPublicCommitments,
    public_inputs: CompositePublicInputs,
}

/// Generic Hoon-compatible tree used for the recursive certificate internals.
///
/// Homogeneous integer vectors are packed into atoms so the real recursive
/// proof remains much closer to the compact certificate size than a scalar
/// list encoding. This is still a structured noun: field boundaries and
/// recursive proof containers are preserved by the surrounding node tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiProofNode {
    Unit,
    Bool(bool),
    U64(u64),
    I64(i64),
    Ext2([u64; 2]),
    Ext2s(Vec<[u64; 2]>),
    Bytes(Vec<u8>),
    U64s(Vec<u64>),
    I64s(Vec<i64>),
    Seq(Vec<AiProofNode>),
    Map(Vec<(AiProofNode, AiProofNode)>),
    None,
    Some(Box<AiProofNode>),
}

/// Convert a recursive certificate into the generic proof-node tree.
pub fn recursive_certificate_to_node<C: Serialize>(
    certificate: &C,
) -> Result<AiProofNode, CertificateNounError> {
    let node = certificate
        .serialize(NodeSerializer)
        .map_err(|e| CertificateNounError::Serialize(e.to_string()))?;
    Ok(node.normalized())
}

/// Reconstruct a serde-backed recursive certificate from a decoded proof-node
/// tree.
///
/// This is the inverse of [`recursive_certificate_to_node`]. It exists so the
/// production Rust/Hoon boundary can verify the structured noun artifact
/// directly instead of requiring an adjacent compact byte blob.
pub fn recursive_certificate_from_node<C: DeserializeOwned>(
    node: &AiProofNode,
) -> Result<C, CertificateNounError> {
    C::deserialize(NodeDeserializer { node: node.clone() })
        .map_err(|e| CertificateNounError::Deserialize(e.to_string()))
}

fn canonical_certificate_from_node<C>(node: &AiProofNode) -> Result<C, CertificateNounError>
where
    C: DeserializeOwned + Serialize,
{
    let certificate: C = recursive_certificate_from_node(node)?;
    let canonical = recursive_certificate_to_node(&certificate)?;
    if &canonical != node {
        return Err(CertificateNounError::NonCanonicalProofNode);
    }
    Ok(certificate)
}

/// Reconstruct the canonical recursive certificate from a decoded
/// Hoon-compatible proof-node tree.
pub fn ai_pow_recursive_certificate_from_node(
    node: &AiProofNode,
) -> Result<ai_pow_zk::recursion::AiPowRecursiveCertificate, CertificateNounError> {
    canonical_certificate_from_node(node)
}

/// Build the Hoon `ai-pow-certificate` noun:
///
/// ```hoon
/// [version params found-idx trace-height commitments public-inputs certificate]
/// ```
pub fn build_ai_pow_certificate_noun<C: Serialize>(
    zk_params: &ZkParams,
    found_idx: u32,
    trace_height: usize,
    commitments: &ZkPublicCommitments,
    pis: &CompositePublicInputs,
    recursive_certificate: &C,
) -> Result<NounSlab, CertificateNounError> {
    let certificate = recursive_certificate_to_node(recursive_certificate)?;
    Ok(build_ai_pow_certificate_noun_from_node(
        zk_params, found_idx, trace_height, commitments, pis, &certificate,
    ))
}

/// Build the same top-level certificate noun from an already-serialized proof
/// node. This is mainly useful for focused shape tests.
pub fn build_ai_pow_certificate_noun_from_node(
    zk_params: &ZkParams,
    found_idx: u32,
    trace_height: usize,
    commitments: &ZkPublicCommitments,
    pis: &CompositePublicInputs,
    certificate: &AiProofNode,
) -> NounSlab {
    let mut slab = NounSlab::new();
    let params = encode_params(&mut slab, zk_params);
    let commitments = encode_commitments(&mut slab, commitments);
    let public_inputs = encode_public_inputs(&mut slab, pis);
    let certificate = certificate.to_noun(&mut slab);
    let root = T(
        &mut slab,
        &[
            D(AI_POW_CERT_VERSION),
            params,
            D(found_idx as u64),
            D(trace_height as u64),
            commitments,
            public_inputs,
            certificate,
        ],
    );
    slab.set_root(root);
    slab
}

/// Decode and validate the Hoon `ai-pow-certificate` root in a slab.
pub fn decode_ai_pow_certificate_slab<J>(
    slab: &NounSlab<J>,
    limits: CertificateNounLimits,
) -> Result<AiPowCertificateShape, CertificateNounError> {
    let space = slab.noun_space();
    let root = unsafe { *slab.root() };
    decode_ai_pow_certificate_noun(root, &space, limits)
}

/// Decode and validate a Hoon `ai-pow-certificate` noun.
///
/// This parser enforces the production shape and bounded proof-node recursion.
/// It does not verify the recursive ZKP itself.
pub fn decode_ai_pow_certificate_noun(
    root: Noun,
    space: &NounSpace,
    limits: CertificateNounLimits,
) -> Result<AiPowCertificateShape, CertificateNounError> {
    let fields = tuple7(root, space, "ai-pow-certificate")?;
    let metadata = decode_ai_pow_certificate_metadata_fields(&fields, space, limits)?;
    let mut state = DecodeState::new(limits);
    Ok(AiPowCertificateShape {
        version: metadata.version,
        zk_params: metadata.zk_params,
        found_idx: metadata.found_idx,
        trace_height: metadata.trace_height,
        commitments: metadata.commitments,
        public_inputs: metadata.public_inputs,
        certificate: decode_proof_node(fields[6], space, &mut state, 0)?,
    })
}

fn decode_ai_pow_certificate_metadata_noun(
    root: Noun,
    space: &NounSpace,
    limits: CertificateNounLimits,
) -> Result<AiPowCertificateMetadata, CertificateNounError> {
    let fields = tuple7(root, space, "ai-pow-certificate")?;
    decode_ai_pow_certificate_metadata_fields(&fields, space, limits)
}

fn decode_ai_pow_certificate_metadata_fields(
    fields: &[Noun; 7],
    space: &NounSpace,
    limits: CertificateNounLimits,
) -> Result<AiPowCertificateMetadata, CertificateNounError> {
    let version = expect_u64(fields[0], space, "version")?;
    if version != AI_POW_CERT_VERSION {
        return Err(CertificateNounError::UnsupportedVersion(version));
    }
    Ok(AiPowCertificateMetadata {
        version,
        zk_params: decode_params(fields[1], space)?,
        found_idx: u32::try_from(expect_u64(fields[2], space, "found-idx")?)
            .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "found-idx" })?,
        trace_height: usize::try_from(expect_u64(fields[3], space, "trace-height")?).map_err(
            |_| CertificateNounError::IntegerOutOfRange {
                field: "trace-height",
            },
        )?,
        commitments: decode_commitments(fields[4], space, limits)?,
        public_inputs: decode_public_inputs(fields[5], space, limits)?,
    })
}

/// Decode and validate a full Hoon `%ai-pow` block artifact in a slab.
///
/// The expected noun shape is:
///
/// ```hoon
/// [%ai-pow nonce=ai-ncmn cert=ai-pow-certificate]
/// ```
///
/// This is the production artifact shape persisted in blocks and received from
/// miner pokes. It includes the NCMN nonce because the nonce is a verifier
/// commitment parameter for the recursive certificate statement, not an
/// optional side channel.
pub fn decode_ai_pow_artifact_slab<J>(
    slab: &NounSlab<J>,
    limits: CertificateNounLimits,
) -> Result<AiPowArtifactShape, CertificateNounError> {
    let space = slab.noun_space();
    let root = unsafe { *slab.root() };
    decode_ai_pow_artifact_noun(root, &space, limits)
}

/// Decode and validate a jammed Hoon `%ai-pow` block artifact.
///
/// This is the byte-oriented boundary a consensus verifier should use when it
/// starts from persisted or network-transmitted jam bytes. It enforces the
/// configured jam byte limit before cueing so attacker-controlled bytes cannot
/// force unbounded noun allocation before the structured certificate limits
/// apply.
pub fn decode_ai_pow_artifact_jam(
    jammed: &[u8],
    limits: CertificateNounLimits,
) -> Result<AiPowArtifactShape, CertificateNounError> {
    if jammed.is_empty() {
        return Err(CertificateNounError::Cue(CueError::TruncatedBuffer));
    }
    if jammed.len() > limits.max_jam_bytes {
        return Err(CertificateNounError::JammedLengthExceeded {
            limit: limits.max_jam_bytes,
            actual: jammed.len(),
        });
    }
    preflight_ai_pow_artifact_jam(jammed, limits)?;

    let mut slab: NounSlab = NounSlab::new();
    let root = catch_unwind(AssertUnwindSafe(|| {
        slab.cue_into(Bytes::copy_from_slice(jammed))
    }))
    .map_err(|_| CertificateNounError::CuePanic)??;
    slab.set_root(root);
    if slab.jam().as_ref() != jammed {
        return Err(CertificateNounError::NonCanonicalJam);
    }
    decode_ai_pow_artifact_slab(&slab, limits)
}

fn preflight_ai_pow_artifact_jam(
    jammed: &[u8],
    limits: CertificateNounLimits,
) -> Result<(), CertificateNounError> {
    let total_bits = jammed
        .len()
        .checked_mul(8)
        .ok_or(CertificateNounError::LimitExceeded("jam bytes"))?;
    let mut cursor = 0usize;
    let mut total_nodes = 0usize;
    let mut stack = vec![1usize];

    while let Some(depth) = stack.pop() {
        if depth > limits.max_depth {
            return Err(CertificateNounError::LimitExceeded("jam noun depth"));
        }
        total_nodes = total_nodes
            .checked_add(1)
            .ok_or(CertificateNounError::LimitExceeded("jam noun count"))?;
        if total_nodes > limits.max_total_nodes {
            return Err(CertificateNounError::LimitExceeded("jam noun count"));
        }

        let first = bit_at(jammed, cursor).ok_or(CueError::TruncatedBuffer)?;
        cursor += 1;
        if first {
            let second = bit_at(jammed, cursor).ok_or(CueError::TruncatedBuffer)?;
            cursor += 1;
            if second {
                let backref = rub_usize(&mut cursor, jammed, total_bits, "jam backref")?;
                if backref >= cursor {
                    return Err(CertificateNounError::Cue(CueError::BadBackref));
                }
            } else {
                let child_depth = depth
                    .checked_add(1)
                    .ok_or(CertificateNounError::LimitExceeded("jam noun depth"))?;
                stack.push(child_depth);
                stack.push(child_depth);
            }
        } else {
            let atom_bits = rub_usize(&mut cursor, jammed, total_bits, "jam atom size")?;
            let atom_bytes = atom_bits.saturating_add(7) / 8;
            if atom_bytes > limits.max_atom_bytes {
                return Err(CertificateNounError::LimitExceeded("jam atom bytes"));
            }
            cursor = cursor
                .checked_add(atom_bits)
                .ok_or(CertificateNounError::LimitExceeded("jam atom size"))?;
            if cursor > total_bits {
                return Err(CertificateNounError::Cue(CueError::TruncatedBuffer));
            }
        }
    }

    Ok(())
}

fn rub_usize(
    cursor: &mut usize,
    jammed: &[u8],
    total_bits: usize,
    field: &'static str,
) -> Result<usize, CertificateNounError> {
    let Some(idx) = first_one(jammed, *cursor, total_bits) else {
        return Err(CertificateNounError::Cue(CueError::TruncatedBuffer));
    };
    if idx == 0 {
        *cursor = (*cursor)
            .checked_add(1)
            .ok_or(CertificateNounError::LimitExceeded(field))?;
        return Ok(0);
    }

    let size_bits = idx - 1;
    if size_bits >= usize::BITS as usize {
        return Err(CertificateNounError::LimitExceeded(field));
    }
    *cursor = (*cursor)
        .checked_add(idx + 1)
        .ok_or(CertificateNounError::LimitExceeded(field))?;
    if total_bits < (*cursor).saturating_add(size_bits) {
        return Err(CertificateNounError::Cue(CueError::TruncatedBuffer));
    }

    let mut value = 1usize << size_bits;
    for bit in 0..size_bits {
        if bit_at(jammed, *cursor + bit).ok_or(CueError::TruncatedBuffer)? {
            value |= 1usize << bit;
        }
    }
    *cursor = (*cursor)
        .checked_add(size_bits)
        .ok_or(CertificateNounError::LimitExceeded(field))?;
    Ok(value)
}

fn first_one(jammed: &[u8], start: usize, total_bits: usize) -> Option<usize> {
    let mut offset = 0usize;
    let mut cursor = start;
    while cursor < total_bits {
        if bit_at(jammed, cursor)? {
            return Some(offset);
        }
        offset += 1;
        cursor += 1;
    }
    None
}

fn bit_at(jammed: &[u8], bit: usize) -> Option<bool> {
    let byte = *jammed.get(bit / 8)?;
    Some(((byte >> (bit % 8)) & 1) == 1)
}

/// Decode and validate a full Hoon `%ai-pow` block artifact noun.
pub fn decode_ai_pow_artifact_noun(
    root: Noun,
    space: &NounSpace,
    limits: CertificateNounLimits,
) -> Result<AiPowArtifactShape, CertificateNounError> {
    let fields = tuple3(root, space, "ai-pow artifact")?;
    let tag = expect_u64(fields[0], space, "ai-pow artifact tag")?;
    if tag != tas!(b"ai-pow") {
        return Err(CertificateNounError::Shape("expected %ai-pow artifact"));
    }
    let nonce = expect_fixed_bytes::<NCMN_NONCE_LEN>(fields[1], space, "ncmn nonce", limits)?;
    let (anchors, _) = parse_ncmn_nonce(&nonce)?;
    if anchors.external_commitment.is_some() {
        return Err(CertificateNounError::NonceExternalCommitmentPresent);
    }
    Ok(AiPowArtifactShape {
        nonce,
        certificate: decode_ai_pow_certificate_noun(fields[2], space, limits)?,
    })
}

/// Low-level statement precheck for verifier-derived AI-PoW metadata.
///
/// This is deliberately separate from recursive proof verification. It rejects
/// metadata replay across `(block_commitment, nonce, target)` before a caller
/// spends verifier work on the miner-controlled recursive certificate tree.
/// It also fails closed for multi-tile params until the recursive certificate
/// statement binds a full-matrix aggregate, because the current recursive proof
/// only opens one verifier-derived jackpot tile.
///
/// Production NCMN consensus callers should prefer
/// [`precheck_ai_pow_ncmn_certificate_statement`], which also checks that the
/// submitted nonce is an NCMN nonce anchored to the trusted candidate block.
fn precheck_ai_pow_certificate_statement(
    shape: &AiPowCertificateShape,
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    let metadata = AiPowCertificateMetadata {
        version: shape.version,
        zk_params: shape.zk_params.clone(),
        found_idx: shape.found_idx,
        trace_height: shape.trace_height,
        commitments: shape.commitments.clone(),
        public_inputs: shape.public_inputs.clone(),
    };
    precheck_ai_pow_certificate_metadata(&metadata, block_commitment, nonce, params, target)
}

fn precheck_ai_pow_certificate_metadata(
    metadata: &AiPowCertificateMetadata,
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    let expected = zk_params_from_matmul(params);
    if metadata.zk_params != expected {
        return Err(CertificateNounError::ZkParamsMismatch {
            expected,
            actual: metadata.zk_params.clone(),
        });
    }
    verify_ai_pow_full_matmul_production_statement(
        block_commitment, nonce, params, target, metadata.found_idx, &metadata.commitments,
        &metadata.public_inputs, metadata.trace_height,
    )
    .map_err(CertificateNounError::Statement)
}

fn precheck_ncmn_nonce(
    candidate_nck_commitment: &[u8; 32],
    nonce: &[u8],
) -> Result<(), CertificateNounError> {
    let (anchors, _) = parse_ncmn_nonce(nonce)?;
    if anchors.nck_commitment != *candidate_nck_commitment {
        return Err(CertificateNounError::NonceAnchorMismatch);
    }
    if anchors.external_commitment.is_some() {
        return Err(CertificateNounError::NonceExternalCommitmentPresent);
    }
    Ok(())
}

/// Production NCMN statement precheck for decoded recursive certificate nouns.
///
/// `puzzle_id` is the stable AI puzzle identity bound into the Pearl attempt
/// state. `candidate_nck_commitment` is the trusted 32-byte commitment to the
/// candidate Nockchain block and must appear inside the NCMN nonce. This is the
/// precheck consensus wiring should call before spending recursive verifier
/// work on the miner-controlled proof tree.
pub fn precheck_ai_pow_ncmn_certificate_statement(
    shape: &AiPowCertificateShape,
    puzzle_id: &[u8],
    candidate_nck_commitment: &[u8; 32],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    precheck_ncmn_nonce(candidate_nck_commitment, nonce)?;
    precheck_ai_pow_certificate_statement(shape, puzzle_id, nonce, params, target)
}

/// Production NCMN statement precheck for a decoded `%ai-pow` artifact.
pub fn precheck_ai_pow_ncmn_artifact_statement(
    artifact: &AiPowArtifactShape,
    puzzle_id: &[u8],
    candidate_nck_commitment: &[u8; 32],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    precheck_ai_pow_ncmn_certificate_statement(
        &artifact.certificate, puzzle_id, candidate_nck_commitment, &artifact.nonce, params, target,
    )
}

/// Verify decoded certificate metadata and recursive proof for an NCMN-wrapped
/// production AI-PoW attempt.
///
/// This is the production-safe variant for callers that already reconstructed
/// the recursive certificate object from the structured noun tail. It uses the
/// full-matmul statement precheck, so multi-tile selected-tile certificates are
/// rejected before recursive verifier work.
pub fn verify_ai_pow_ncmn_certificate_statement_and_proof(
    shape: &AiPowCertificateShape,
    puzzle_id: &[u8],
    candidate_nck_commitment: &[u8; 32],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
    certificate: &ai_pow_zk::recursion::AiPowRecursiveCertificate,
) -> Result<(), CertificateNounError> {
    precheck_ai_pow_ncmn_certificate_statement(
        shape, puzzle_id, candidate_nck_commitment, nonce, params, target,
    )?;
    ai_pow_zk::recursion::verify_recursive_certificate(certificate, &shape.public_inputs)
        .map_err(|e| CertificateNounError::RecursiveCertificate(e.to_string()))
}

/// Verify a fully decoded Hoon-compatible `ai-pow-certificate` noun against an
/// explicit attempt tuple.
///
/// This lower-level helper cheaply re-derives and checks the full-matmul
/// statement metadata before decoding the proof tree into the canonical
/// recursive certificate type, then verifies the recursive certificate against
/// those verifier-derived Layer-0 public inputs. It does not parse or enforce
/// the NCMN candidate-block anchor, so it is not the Nockchain
/// consensus/block-wire entrypoint.
fn verify_decoded_ai_pow_certificate(
    shape: &AiPowCertificateShape,
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    precheck_ai_pow_certificate_statement(shape, block_commitment, nonce, params, target)?;
    let certificate = ai_pow_recursive_certificate_from_node(&shape.certificate)?;
    ai_pow_zk::recursion::verify_recursive_certificate(&certificate, &shape.public_inputs)
        .map_err(|e| CertificateNounError::RecursiveCertificate(e.to_string()))
}

/// Verify a fully decoded Hoon-compatible `ai-pow-certificate` noun for an
/// NCMN-wrapped production attempt.
///
/// This is the consensus-facing Rust boundary for Nockchain AI-PoW: it checks
/// the nonce format and candidate-block anchor, re-derives the
/// full-matmul-admissible statement from verifier-trusted data, reconstructs
/// the canonical recursive certificate only after those cheap checks pass, and
/// then verifies the recursive certificate against those public inputs.
pub fn verify_decoded_ai_pow_ncmn_certificate(
    shape: &AiPowCertificateShape,
    puzzle_id: &[u8],
    candidate_nck_commitment: &[u8; 32],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    precheck_ai_pow_ncmn_certificate_statement(
        shape, puzzle_id, candidate_nck_commitment, nonce, params, target,
    )?;
    let certificate = ai_pow_recursive_certificate_from_node(&shape.certificate)?;
    ai_pow_zk::recursion::verify_recursive_certificate(&certificate, &shape.public_inputs)
        .map_err(|e| CertificateNounError::RecursiveCertificate(e.to_string()))
}

/// Verify a fully decoded Hoon `%ai-pow` artifact against trusted block data.
///
/// This is the canonical Rust API a Hoon verifier jet should target after
/// bounded noun decoding: it uses the nonce carried inside the artifact and
/// verifies that the recursive certificate's statement matches the trusted
/// puzzle id, candidate block commitment, params, and target.
pub fn verify_decoded_ai_pow_ncmn_artifact(
    artifact: &AiPowArtifactShape,
    puzzle_id: &[u8],
    candidate_nck_commitment: &[u8; 32],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    verify_decoded_ai_pow_ncmn_certificate(
        &artifact.certificate, puzzle_id, candidate_nck_commitment, &artifact.nonce, params, target,
    )
}

/// Decode a jammed `%ai-pow` artifact and verify it against trusted block data.
///
/// This combines the intended production ordering: byte-size cap, cue, bounded
/// structured decode, NCMN anchor check, cheap statement precheck, and recursive
/// certificate verification.
pub fn verify_ai_pow_ncmn_artifact_jam(
    jammed: &[u8],
    limits: CertificateNounLimits,
    puzzle_id: &[u8],
    candidate_nck_commitment: &[u8; 32],
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<(), CertificateNounError> {
    if jammed.is_empty() {
        return Err(CertificateNounError::Cue(CueError::TruncatedBuffer));
    }
    if jammed.len() > limits.max_jam_bytes {
        return Err(CertificateNounError::JammedLengthExceeded {
            limit: limits.max_jam_bytes,
            actual: jammed.len(),
        });
    }
    preflight_ai_pow_artifact_jam(jammed, limits)?;

    let mut slab: NounSlab = NounSlab::new();
    let root = catch_unwind(AssertUnwindSafe(|| {
        slab.cue_into(Bytes::copy_from_slice(jammed))
    }))
    .map_err(|_| CertificateNounError::CuePanic)??;
    slab.set_root(root);
    if slab.jam().as_ref() != jammed {
        return Err(CertificateNounError::NonCanonicalJam);
    }

    let space = slab.noun_space();
    let root = unsafe { *slab.root() };
    let fields = tuple3(root, &space, "ai-pow artifact")?;
    let tag = expect_u64(fields[0], &space, "ai-pow artifact tag")?;
    if tag != tas!(b"ai-pow") {
        return Err(CertificateNounError::Shape("expected %ai-pow artifact"));
    }
    let nonce = expect_fixed_bytes::<NCMN_NONCE_LEN>(fields[1], &space, "ncmn nonce", limits)?;
    precheck_ncmn_nonce(candidate_nck_commitment, &nonce)?;
    let metadata = decode_ai_pow_certificate_metadata_noun(fields[2], &space, limits)?;
    precheck_ai_pow_certificate_metadata(&metadata, puzzle_id, &nonce, params, target)?;

    let certificate_shape = decode_ai_pow_certificate_noun(fields[2], &space, limits)?;
    let certificate = ai_pow_recursive_certificate_from_node(&certificate_shape.certificate)?;
    ai_pow_zk::recursion::verify_recursive_certificate(&certificate, &metadata.public_inputs)
        .map_err(|e| CertificateNounError::RecursiveCertificate(e.to_string()))
}

#[derive(Debug)]
struct DecodeState {
    limits: CertificateNounLimits,
    total_nodes: usize,
}

impl DecodeState {
    fn new(limits: CertificateNounLimits) -> Self {
        Self {
            limits,
            total_nodes: 0,
        }
    }

    fn enter(&mut self, depth: usize) -> Result<(), CertificateNounError> {
        if depth > self.limits.max_depth {
            return Err(CertificateNounError::LimitExceeded("proof-node depth"));
        }
        self.total_nodes = self
            .total_nodes
            .checked_add(1)
            .ok_or(CertificateNounError::LimitExceeded("proof-node count"))?;
        if self.total_nodes > self.limits.max_total_nodes {
            return Err(CertificateNounError::LimitExceeded("proof-node count"));
        }
        Ok(())
    }
}

fn decode_params(noun: Noun, space: &NounSpace) -> Result<ZkParams, CertificateNounError> {
    let fields = tuple6(noun, space, "zk-params")?;
    Ok(ZkParams {
        m: u32::try_from(expect_u64(fields[0], space, "params.m")?)
            .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "params.m" })?,
        k: u32::try_from(expect_u64(fields[1], space, "params.k")?)
            .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "params.k" })?,
        n: u32::try_from(expect_u64(fields[2], space, "params.n")?)
            .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "params.n" })?,
        noise_rank: u32::try_from(expect_u64(fields[3], space, "params.noise-rank")?).map_err(
            |_| CertificateNounError::IntegerOutOfRange {
                field: "params.noise-rank",
            },
        )?,
        tile: u32::try_from(expect_u64(fields[4], space, "params.tile")?).map_err(|_| {
            CertificateNounError::IntegerOutOfRange {
                field: "params.tile",
            }
        })?,
        difficulty_bits: u32::try_from(expect_u64(fields[5], space, "params.difficulty-bits")?)
            .map_err(|_| CertificateNounError::IntegerOutOfRange {
                field: "params.difficulty-bits",
            })?,
    })
}

fn decode_commitments(
    noun: Noun,
    space: &NounSpace,
    limits: CertificateNounLimits,
) -> Result<ZkPublicCommitments, CertificateNounError> {
    let fields = tuple4(noun, space, "ai-pow-commitments")?;
    Ok(ZkPublicCommitments {
        h_a: expect_fixed_bytes(fields[0], space, "commitments.h-a", limits)?,
        h_b: expect_fixed_bytes(fields[1], space, "commitments.h-b", limits)?,
        h_a_chunk: expect_fixed_bytes(fields[2], space, "commitments.h-a-chunk", limits)?,
        h_b_chunk: expect_fixed_bytes(fields[3], space, "commitments.h-b-chunk", limits)?,
    })
}

fn decode_public_inputs(
    noun: Noun,
    space: &NounSpace,
    limits: CertificateNounLimits,
) -> Result<CompositePublicInputs, CertificateNounError> {
    let fields = tuple7(noun, space, "ai-pow-public-inputs")?;
    let cumsum_fields = tuple4(fields[0], space, "public-inputs.cumsum")?;
    let jackpot_fields = tuple16(fields[1], space, "public-inputs.jackpot")?;
    let mut jackpot = [0u32; 16];
    for (i, field) in jackpot_fields.iter().enumerate() {
        jackpot[i] = u32::try_from(expect_u64(*field, space, "jackpot")?)
            .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "jackpot" })?;
    }
    Ok(CompositePublicInputs {
        cumsum: [
            i32::try_from(expect_i64(cumsum_fields[0], space, "cumsum.0", limits)?)
                .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "cumsum.0" })?,
            i32::try_from(expect_i64(cumsum_fields[1], space, "cumsum.1", limits)?)
                .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "cumsum.1" })?,
            i32::try_from(expect_i64(cumsum_fields[2], space, "cumsum.2", limits)?)
                .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "cumsum.2" })?,
            i32::try_from(expect_i64(cumsum_fields[3], space, "cumsum.3", limits)?)
                .map_err(|_| CertificateNounError::IntegerOutOfRange { field: "cumsum.3" })?,
        ],
        jackpot,
        hash_a: decode_digest_words(fields[2], space, "hash-a", limits)?,
        hash_b: decode_digest_words(fields[3], space, "hash-b", limits)?,
        job_key: decode_digest_words(fields[4], space, "job-key", limits)?,
        commitment_hash: decode_digest_words(fields[5], space, "commitment-hash", limits)?,
        hash_jackpot: decode_digest_words(fields[6], space, "hash-jackpot", limits)?,
    })
}

fn decode_digest_words(
    noun: Noun,
    space: &NounSpace,
    field: &'static str,
    limits: CertificateNounLimits,
) -> Result<[u32; 8], CertificateNounError> {
    let bytes = expect_fixed_bytes::<32>(noun, space, field, limits)?;
    Ok(core::array::from_fn(|i| {
        u32::from_le_bytes(bytes[i * 4..(i + 1) * 4].try_into().expect("slice len"))
    }))
}

fn decode_proof_node(
    noun: Noun,
    space: &NounSpace,
    state: &mut DecodeState,
    depth: usize,
) -> Result<AiProofNode, CertificateNounError> {
    state.enter(depth)?;
    let cell = noun
        .in_space(space)
        .as_cell()
        .map_err(|_| CertificateNounError::Shape("proof node must be a tagged cell"))?;
    let tag = expect_u64(cell.head().noun(), space, "proof-node tag")?;
    let tail = cell.tail().noun();
    match tag {
        x if x == tas!(b"n") => {
            expect_nil(tail, space, "unit proof-node tail")?;
            Ok(AiProofNode::Unit)
        }
        x if x == tas!(b"b") => match expect_u64(tail, space, "bool proof-node value")? {
            0 => Ok(AiProofNode::Bool(false)),
            1 => Ok(AiProofNode::Bool(true)),
            _ => Err(CertificateNounError::Shape(
                "bool proof-node value must be 0 or 1",
            )),
        },
        x if x == tas!(b"u") => Ok(AiProofNode::U64(expect_u64(
            tail, space, "u64 proof-node value",
        )?)),
        x if x == tas!(b"i") => Ok(AiProofNode::I64(expect_i64(
            tail, space, "i64 proof-node value", state.limits,
        )?)),
        x if x == tas!(b"ext2") => Ok(AiProofNode::Ext2(expect_ext2(
            tail, space, "ext2", state.limits,
        )?)),
        x if x == tas!(b"ext2s") => {
            let fields = tuple2(tail, space, "ext2s proof-node")?;
            let len = expect_len(
                fields[0], space, "ext2s length", state.limits.max_packed_items,
            )?;
            let bytes = expect_declared_bytes(
                fields[1],
                space,
                len.checked_mul(16)
                    .ok_or(CertificateNounError::LimitExceeded("ext2s bytes"))?,
                "ext2s",
                state.limits,
            )?;
            let mut values = Vec::with_capacity(len);
            for chunk in bytes.chunks_exact(16) {
                let c0 = u64::from_le_bytes(chunk[0..8].try_into().expect("chunk len"));
                let c1 = u64::from_le_bytes(chunk[8..16].try_into().expect("chunk len"));
                expect_goldilocks(c0, "ext2s.c0")?;
                expect_goldilocks(c1, "ext2s.c1")?;
                values.push([c0, c1]);
            }
            Ok(AiProofNode::Ext2s(values))
        }
        x if x == tas!(b"bytes") => {
            let fields = tuple2(tail, space, "bytes proof-node")?;
            let len = expect_len(
                fields[0], space, "bytes length", state.limits.max_packed_items,
            )?;
            Ok(AiProofNode::Bytes(expect_declared_bytes(
                fields[1], space, len, "bytes", state.limits,
            )?))
        }
        x if x == tas!(b"u64s") => {
            let fields = tuple2(tail, space, "u64s proof-node")?;
            let len = expect_len(
                fields[0], space, "u64s length", state.limits.max_packed_items,
            )?;
            let bytes = expect_declared_bytes(
                fields[1],
                space,
                len.checked_mul(8)
                    .ok_or(CertificateNounError::LimitExceeded("u64s bytes"))?,
                "u64s",
                state.limits,
            )?;
            Ok(AiProofNode::U64s(
                bytes
                    .chunks_exact(8)
                    .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("chunk len")))
                    .collect(),
            ))
        }
        x if x == tas!(b"i64s") => {
            let fields = tuple2(tail, space, "i64s proof-node")?;
            let len = expect_len(
                fields[0], space, "i64s length", state.limits.max_packed_items,
            )?;
            let bytes = expect_declared_bytes(
                fields[1],
                space,
                len.checked_mul(8)
                    .ok_or(CertificateNounError::LimitExceeded("i64s bytes"))?,
                "i64s",
                state.limits,
            )?;
            Ok(AiProofNode::I64s(
                bytes
                    .chunks_exact(8)
                    .map(|chunk| i64::from_le_bytes(chunk.try_into().expect("chunk len")))
                    .collect(),
            ))
        }
        x if x == tas!(b"seq") => Ok(AiProofNode::Seq(decode_list(tail, space, state, depth)?)),
        x if x == tas!(b"map") => Ok(AiProofNode::Map(decode_map_entries(
            tail, space, state, depth,
        )?)),
        x if x == tas!(b"none") => {
            expect_nil(tail, space, "none proof-node tail")?;
            Ok(AiProofNode::None)
        }
        x if x == tas!(b"some") => Ok(AiProofNode::Some(Box::new(decode_proof_node(
            tail,
            space,
            state,
            depth + 1,
        )?))),
        other => Err(CertificateNounError::InvalidTag(other)),
    }
}

fn decode_list(
    mut list: Noun,
    space: &NounSpace,
    state: &mut DecodeState,
    depth: usize,
) -> Result<Vec<AiProofNode>, CertificateNounError> {
    let mut out = Vec::new();
    while !is_nil(list, space)? {
        if out.len() >= state.limits.max_list_items {
            return Err(CertificateNounError::LimitExceeded(
                "proof-node list length",
            ));
        }
        let cell = list
            .in_space(space)
            .as_cell()
            .map_err(|_| CertificateNounError::Shape("improper proof-node list"))?;
        out.push(decode_proof_node(
            cell.head().noun(),
            space,
            state,
            depth + 1,
        )?);
        list = cell.tail().noun();
    }
    Ok(out)
}

fn decode_map_entries(
    mut list: Noun,
    space: &NounSpace,
    state: &mut DecodeState,
    depth: usize,
) -> Result<Vec<(AiProofNode, AiProofNode)>, CertificateNounError> {
    let mut out = Vec::new();
    while !is_nil(list, space)? {
        if out.len() >= state.limits.max_list_items {
            return Err(CertificateNounError::LimitExceeded("proof-node map length"));
        }
        let cell = list
            .in_space(space)
            .as_cell()
            .map_err(|_| CertificateNounError::Shape("improper proof-node map"))?;
        let pair = tuple2(cell.head().noun(), space, "proof-node map entry")?;
        let key = decode_proof_node(pair[0], space, state, depth + 1)?;
        let value = decode_proof_node(pair[1], space, state, depth + 1)?;
        out.push((key, value));
        list = cell.tail().noun();
    }
    Ok(out)
}

fn tuple2(
    noun: Noun,
    space: &NounSpace,
    name: &'static str,
) -> Result<[Noun; 2], CertificateNounError> {
    let c = noun
        .in_space(space)
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    Ok([c.head().noun(), c.tail().noun()])
}

fn tuple3(
    noun: Noun,
    space: &NounSpace,
    name: &'static str,
) -> Result<[Noun; 3], CertificateNounError> {
    let c1 = noun
        .in_space(space)
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c2 = c1
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    Ok([c1.head().noun(), c2.head().noun(), c2.tail().noun()])
}

fn tuple4(
    noun: Noun,
    space: &NounSpace,
    name: &'static str,
) -> Result<[Noun; 4], CertificateNounError> {
    let c1 = noun
        .in_space(space)
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c2 = c1
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c3 = c2
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    Ok([c1.head().noun(), c2.head().noun(), c3.head().noun(), c3.tail().noun()])
}

fn tuple6(
    noun: Noun,
    space: &NounSpace,
    name: &'static str,
) -> Result<[Noun; 6], CertificateNounError> {
    let c1 = noun
        .in_space(space)
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c2 = c1
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c3 = c2
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c4 = c3
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c5 = c4
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    Ok([
        c1.head().noun(),
        c2.head().noun(),
        c3.head().noun(),
        c4.head().noun(),
        c5.head().noun(),
        c5.tail().noun(),
    ])
}

fn tuple7(
    noun: Noun,
    space: &NounSpace,
    name: &'static str,
) -> Result<[Noun; 7], CertificateNounError> {
    let c1 = noun
        .in_space(space)
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c2 = c1
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c3 = c2
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c4 = c3
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c5 = c4
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    let c6 = c5
        .tail()
        .as_cell()
        .map_err(|_| CertificateNounError::Shape(name))?;
    Ok([
        c1.head().noun(),
        c2.head().noun(),
        c3.head().noun(),
        c4.head().noun(),
        c5.head().noun(),
        c6.head().noun(),
        c6.tail().noun(),
    ])
}

fn tuple16(
    mut noun: Noun,
    space: &NounSpace,
    name: &'static str,
) -> Result<[Noun; 16], CertificateNounError> {
    let mut fields = [D(0); 16];
    for field in fields.iter_mut().take(15) {
        let cell = noun
            .in_space(space)
            .as_cell()
            .map_err(|_| CertificateNounError::Shape(name))?;
        *field = cell.head().noun();
        noun = cell.tail().noun();
    }
    fields[15] = noun;
    Ok(fields)
}

fn expect_u64(
    noun: Noun,
    space: &NounSpace,
    field: &'static str,
) -> Result<u64, CertificateNounError> {
    noun.in_space(space)
        .as_atom()
        .and_then(|atom| atom.as_u64())
        .map_err(|_| CertificateNounError::IntegerOutOfRange { field })
}

fn expect_len(
    noun: Noun,
    space: &NounSpace,
    field: &'static str,
    max: usize,
) -> Result<usize, CertificateNounError> {
    let len = usize::try_from(expect_u64(noun, space, field)?)
        .map_err(|_| CertificateNounError::IntegerOutOfRange { field })?;
    if len > max {
        return Err(CertificateNounError::LimitExceeded(field));
    }
    Ok(len)
}

fn expect_i64(
    noun: Noun,
    space: &NounSpace,
    field: &'static str,
    limits: CertificateNounLimits,
) -> Result<i64, CertificateNounError> {
    let bytes = expect_declared_bytes(noun, space, 8, field, limits)?;
    Ok(i64::from_le_bytes(bytes.try_into().expect("declared len")))
}

fn expect_ext2(
    noun: Noun,
    space: &NounSpace,
    field: &'static str,
    limits: CertificateNounLimits,
) -> Result<[u64; 2], CertificateNounError> {
    let bytes = expect_declared_bytes(noun, space, 16, field, limits)?;
    let c0 = u64::from_le_bytes(bytes[0..8].try_into().expect("chunk len"));
    let c1 = u64::from_le_bytes(bytes[8..16].try_into().expect("chunk len"));
    expect_goldilocks(c0, "ext2.c0")?;
    expect_goldilocks(c1, "ext2.c1")?;
    Ok([c0, c1])
}

fn expect_goldilocks(value: u64, field: &'static str) -> Result<(), CertificateNounError> {
    if value < GOLDILOCKS_MODULUS {
        Ok(())
    } else {
        Err(CertificateNounError::NonCanonicalField { field })
    }
}

fn expect_fixed_bytes<const N: usize>(
    noun: Noun,
    space: &NounSpace,
    tag: &'static str,
    limits: CertificateNounLimits,
) -> Result<[u8; N], CertificateNounError> {
    let bytes = expect_declared_bytes(noun, space, N, tag, limits)?;
    Ok(bytes.try_into().expect("declared len"))
}

fn expect_declared_bytes(
    noun: Noun,
    space: &NounSpace,
    declared: usize,
    tag: &'static str,
    limits: CertificateNounLimits,
) -> Result<Vec<u8>, CertificateNounError> {
    if declared > limits.max_atom_bytes {
        return Err(CertificateNounError::LimitExceeded("atom bytes"));
    }
    let atom = noun
        .in_space(space)
        .as_atom()
        .map_err(|_| CertificateNounError::Shape("expected atom bytes"))?;
    let actual = met(3, atom.atom(), space);
    if actual > declared {
        return Err(CertificateNounError::PackedLengthMismatch {
            tag,
            declared,
            actual,
        });
    }
    if actual > limits.max_atom_bytes {
        return Err(CertificateNounError::LimitExceeded("atom bytes"));
    }
    let mut out = vec![0u8; declared];
    out[..actual].copy_from_slice(&atom.as_ne_bytes()[..actual]);
    Ok(out)
}

fn expect_nil(
    noun: Noun,
    space: &NounSpace,
    field: &'static str,
) -> Result<(), CertificateNounError> {
    if is_nil(noun, space)? {
        Ok(())
    } else {
        Err(CertificateNounError::Shape(field))
    }
}

fn is_nil(noun: Noun, space: &NounSpace) -> Result<bool, CertificateNounError> {
    match noun.in_space(space).as_atom() {
        Ok(atom) => Ok(atom.as_u64().map(|value| value == 0).unwrap_or(false)),
        Err(_) => Ok(false),
    }
}

impl AiProofNode {
    fn normalized(self) -> Self {
        match self {
            AiProofNode::Seq(items) => normalize_seq(items),
            AiProofNode::Map(items) => AiProofNode::Map(
                items
                    .into_iter()
                    .map(|(k, v)| (k.normalized(), v.normalized()))
                    .collect(),
            ),
            AiProofNode::Some(inner) => AiProofNode::Some(Box::new(inner.normalized())),
            other => other,
        }
    }

    pub fn to_noun<A: NounAllocator>(&self, allocator: &mut A) -> Noun {
        match self {
            AiProofNode::Unit => T(allocator, &[D(tas!(b"n")), D(0)]),
            AiProofNode::Bool(value) => T(allocator, &[D(tas!(b"b")), D(u64::from(*value))]),
            AiProofNode::U64(value) => T(allocator, &[D(tas!(b"u")), D(*value)]),
            AiProofNode::I64(value) => {
                let data = i64_to_atom(allocator, *value);
                T(allocator, &[D(tas!(b"i")), data])
            }
            AiProofNode::Ext2(value) => {
                let data = ext2_to_atom(allocator, *value);
                T(allocator, &[D(tas!(b"ext2")), data])
            }
            AiProofNode::Ext2s(values) => {
                let data = packed_ext2s_to_atom(allocator, values);
                T(
                    allocator,
                    &[D(tas!(b"ext2s")), D(values.len() as u64), data],
                )
            }
            AiProofNode::Bytes(bytes) => {
                let data = bytes_to_atom(allocator, bytes);
                T(allocator, &[D(tas!(b"bytes")), D(bytes.len() as u64), data])
            }
            AiProofNode::U64s(values) => {
                let data = packed_u64s_to_atom(allocator, values);
                T(allocator, &[D(tas!(b"u64s")), D(values.len() as u64), data])
            }
            AiProofNode::I64s(values) => {
                let data = packed_i64s_to_atom(allocator, values);
                T(allocator, &[D(tas!(b"i64s")), D(values.len() as u64), data])
            }
            AiProofNode::Seq(items) => {
                let list = encode_list(allocator, items, |allocator, item| item.to_noun(allocator));
                T(allocator, &[D(tas!(b"seq")), list])
            }
            AiProofNode::Map(items) => {
                let list = encode_list(allocator, items, |allocator, (k, v)| {
                    let key = k.to_noun(allocator);
                    let value = v.to_noun(allocator);
                    T(allocator, &[key, value])
                });
                T(allocator, &[D(tas!(b"map")), list])
            }
            AiProofNode::None => T(allocator, &[D(tas!(b"none")), D(0)]),
            AiProofNode::Some(inner) => {
                let inner = inner.to_noun(allocator);
                T(allocator, &[D(tas!(b"some")), inner])
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SeqKind {
    Seq,
    Tuple,
    Struct,
}

fn normalize_seq(items: Vec<AiProofNode>) -> AiProofNode {
    normalize_seq_with_kind(items, SeqKind::Seq)
}

fn normalize_seq_with_kind(items: Vec<AiProofNode>, kind: SeqKind) -> AiProofNode {
    let items = items
        .into_iter()
        .filter_map(|item| match item.normalized() {
            AiProofNode::Unit => None,
            other => Some(other),
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return AiProofNode::Seq(items);
    }
    if matches!(kind, SeqKind::Tuple) && items.len() == 2 {
        if let [AiProofNode::U64(c0), AiProofNode::U64(c1)] = items.as_slice() {
            return AiProofNode::Ext2([*c0, *c1]);
        }
    }
    if items
        .iter()
        .all(|item| matches!(item, AiProofNode::Ext2(_)))
    {
        return AiProofNode::Ext2s(
            items
                .into_iter()
                .map(|item| match item {
                    AiProofNode::Ext2(value) => value,
                    _ => unreachable!(),
                })
                .collect(),
        );
    }
    if items.iter().all(|item| matches!(item, AiProofNode::U64(_))) {
        return AiProofNode::U64s(
            items
                .into_iter()
                .map(|item| match item {
                    AiProofNode::U64(value) => value,
                    _ => unreachable!(),
                })
                .collect(),
        );
    }
    if items.iter().all(|item| matches!(item, AiProofNode::I64(_))) {
        return AiProofNode::I64s(
            items
                .into_iter()
                .map(|item| match item {
                    AiProofNode::I64(value) => value,
                    _ => unreachable!(),
                })
                .collect(),
        );
    }
    AiProofNode::Seq(items)
}

fn encode_params<A: NounAllocator>(allocator: &mut A, params: &ZkParams) -> Noun {
    T(
        allocator,
        &[
            D(params.m as u64),
            D(params.k as u64),
            D(params.n as u64),
            D(params.noise_rank as u64),
            D(params.tile as u64),
            D(params.difficulty_bits as u64),
        ],
    )
}

fn encode_commitments<A: NounAllocator>(
    allocator: &mut A,
    commitments: &ZkPublicCommitments,
) -> Noun {
    let h_a = bytes_to_atom(allocator, &commitments.h_a);
    let h_b = bytes_to_atom(allocator, &commitments.h_b);
    let h_a_chunk = bytes_to_atom(allocator, &commitments.h_a_chunk);
    let h_b_chunk = bytes_to_atom(allocator, &commitments.h_b_chunk);
    T(allocator, &[h_a, h_b, h_a_chunk, h_b_chunk])
}

fn encode_public_inputs<A: NounAllocator>(allocator: &mut A, pis: &CompositePublicInputs) -> Noun {
    let c0 = i64_to_atom(allocator, pis.cumsum[0] as i64);
    let c1 = i64_to_atom(allocator, pis.cumsum[1] as i64);
    let c2 = i64_to_atom(allocator, pis.cumsum[2] as i64);
    let c3 = i64_to_atom(allocator, pis.cumsum[3] as i64);
    let cumsum = T(allocator, &[c0, c1, c2, c3]);
    let jackpot = T(
        allocator,
        &[
            D(pis.jackpot[0] as u64),
            D(pis.jackpot[1] as u64),
            D(pis.jackpot[2] as u64),
            D(pis.jackpot[3] as u64),
            D(pis.jackpot[4] as u64),
            D(pis.jackpot[5] as u64),
            D(pis.jackpot[6] as u64),
            D(pis.jackpot[7] as u64),
            D(pis.jackpot[8] as u64),
            D(pis.jackpot[9] as u64),
            D(pis.jackpot[10] as u64),
            D(pis.jackpot[11] as u64),
            D(pis.jackpot[12] as u64),
            D(pis.jackpot[13] as u64),
            D(pis.jackpot[14] as u64),
            D(pis.jackpot[15] as u64),
        ],
    );
    let hash_a = bytes_to_atom(allocator, &digest_words_to_bytes(&pis.hash_a));
    let hash_b = bytes_to_atom(allocator, &digest_words_to_bytes(&pis.hash_b));
    let job_key = bytes_to_atom(allocator, &digest_words_to_bytes(&pis.job_key));
    let commitment_hash = bytes_to_atom(allocator, &digest_words_to_bytes(&pis.commitment_hash));
    let hash_jackpot = bytes_to_atom(allocator, &digest_words_to_bytes(&pis.hash_jackpot));
    T(
        allocator,
        &[cumsum, jackpot, hash_a, hash_b, job_key, commitment_hash, hash_jackpot],
    )
}

fn digest_words_to_bytes(words: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, word) in words.iter().enumerate() {
        out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

fn encode_list<A, T, F>(allocator: &mut A, items: &[T], mut encode: F) -> Noun
where
    A: NounAllocator,
    F: FnMut(&mut A, &T) -> Noun,
{
    let mut list = D(0);
    for item in items.iter().rev() {
        let head = encode(allocator, item);
        list = T(allocator, &[head, list]);
    }
    list
}

fn bytes_to_atom<A: NounAllocator>(allocator: &mut A, bytes: &[u8]) -> Noun {
    use nockvm::noun::IndirectAtom;
    let atom = <IndirectAtom as nockapp::IndirectAtomExt>::from_bytes(allocator, bytes);
    atom.as_noun()
}

fn packed_u64s_to_atom<A: NounAllocator>(allocator: &mut A, values: &[u64]) -> Noun {
    let mut bytes = Vec::with_capacity(values.len() * 8);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes_to_atom(allocator, &bytes)
}

fn packed_i64s_to_atom<A: NounAllocator>(allocator: &mut A, values: &[i64]) -> Noun {
    let mut bytes = Vec::with_capacity(values.len() * 8);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes_to_atom(allocator, &bytes)
}

fn ext2_to_atom<A: NounAllocator>(allocator: &mut A, value: [u64; 2]) -> Noun {
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&value[0].to_le_bytes());
    bytes.extend_from_slice(&value[1].to_le_bytes());
    bytes_to_atom(allocator, &bytes)
}

fn packed_ext2s_to_atom<A: NounAllocator>(allocator: &mut A, values: &[[u64; 2]]) -> Noun {
    let mut bytes = Vec::with_capacity(values.len() * 16);
    for value in values {
        bytes.extend_from_slice(&value[0].to_le_bytes());
        bytes.extend_from_slice(&value[1].to_le_bytes());
    }
    bytes_to_atom(allocator, &bytes)
}

fn i64_to_atom<A: NounAllocator>(allocator: &mut A, value: i64) -> Noun {
    bytes_to_atom(allocator, &value.to_le_bytes())
}

#[derive(Debug)]
struct SerError(String);

impl std::fmt::Display for SerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SerError {}

impl ser::Error for SerError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

type SerResult<T> = Result<T, SerError>;

struct NodeSerializer;

impl ser::Serializer for NodeSerializer {
    type Ok = AiProofNode;
    type Error = SerError;
    type SerializeSeq = NodeSeq;
    type SerializeTuple = NodeSeq;
    type SerializeTupleStruct = NodeSeq;
    type SerializeTupleVariant = NodeSeq;
    type SerializeMap = NodeMap;
    type SerializeStruct = NodeSeq;
    type SerializeStructVariant = NodeSeq;

    fn serialize_bool(self, v: bool) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> SerResult<Self::Ok> {
        Ok(AiProofNode::I64(v as i64))
    }
    fn serialize_i16(self, v: i16) -> SerResult<Self::Ok> {
        Ok(AiProofNode::I64(v as i64))
    }
    fn serialize_i32(self, v: i32) -> SerResult<Self::Ok> {
        Ok(AiProofNode::I64(v as i64))
    }
    fn serialize_i64(self, v: i64) -> SerResult<Self::Ok> {
        Ok(AiProofNode::I64(v))
    }
    fn serialize_i128(self, v: i128) -> SerResult<Self::Ok> {
        let value = i64::try_from(v)
            .map_err(|_| SerError("i128 values are not valid in AI-PoW certificates".into()))?;
        Ok(AiProofNode::I64(value))
    }
    fn serialize_u8(self, v: u8) -> SerResult<Self::Ok> {
        Ok(AiProofNode::U64(v as u64))
    }
    fn serialize_u16(self, v: u16) -> SerResult<Self::Ok> {
        Ok(AiProofNode::U64(v as u64))
    }
    fn serialize_u32(self, v: u32) -> SerResult<Self::Ok> {
        Ok(AiProofNode::U64(v as u64))
    }
    fn serialize_u64(self, v: u64) -> SerResult<Self::Ok> {
        Ok(AiProofNode::U64(v))
    }
    fn serialize_u128(self, v: u128) -> SerResult<Self::Ok> {
        let value = u64::try_from(v)
            .map_err(|_| SerError("u128 values are not valid in AI-PoW certificates".into()))?;
        Ok(AiProofNode::U64(value))
    }
    fn serialize_f32(self, _v: f32) -> SerResult<Self::Ok> {
        Err(SerError(
            "floating-point values are not valid in AI-PoW certificates".into(),
        ))
    }
    fn serialize_f64(self, _v: f64) -> SerResult<Self::Ok> {
        Err(SerError(
            "floating-point values are not valid in AI-PoW certificates".into(),
        ))
    }
    fn serialize_char(self, v: char) -> SerResult<Self::Ok> {
        Ok(AiProofNode::U64(v as u32 as u64))
    }
    fn serialize_str(self, v: &str) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Bytes(v.as_bytes().to_vec()))
    }
    fn serialize_bytes(self, v: &[u8]) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Bytes(v.to_vec()))
    }
    fn serialize_none(self) -> SerResult<Self::Ok> {
        Ok(AiProofNode::None)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Some(Box::new(
            value.serialize(NodeSerializer)?,
        )))
    }
    fn serialize_unit(self) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Unit)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Unit)
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
    ) -> SerResult<Self::Ok> {
        Ok(AiProofNode::U64(variant_index as u64))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> SerResult<Self::Ok> {
        value.serialize(NodeSerializer)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Seq(vec![
            AiProofNode::U64(variant_index as u64),
            value.serialize(NodeSerializer)?,
        ]))
    }
    fn serialize_seq(self, _len: Option<usize>) -> SerResult<Self::SerializeSeq> {
        Ok(NodeSeq::default())
    }
    fn serialize_tuple(self, _len: usize) -> SerResult<Self::SerializeTuple> {
        Ok(NodeSeq::new(SeqKind::Tuple))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> SerResult<Self::SerializeTupleStruct> {
        Ok(NodeSeq::new(SeqKind::Tuple))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> SerResult<Self::SerializeTupleVariant> {
        Ok(NodeSeq {
            items: vec![AiProofNode::U64(variant_index as u64)],
            kind: SeqKind::Tuple,
        })
    }
    fn serialize_map(self, _len: Option<usize>) -> SerResult<Self::SerializeMap> {
        Ok(NodeMap::default())
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> SerResult<Self::SerializeStruct> {
        Ok(NodeSeq::new(SeqKind::Struct))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> SerResult<Self::SerializeStructVariant> {
        Ok(NodeSeq {
            items: vec![AiProofNode::U64(variant_index as u64)],
            kind: SeqKind::Struct,
        })
    }
}

struct NodeSeq {
    items: Vec<AiProofNode>,
    kind: SeqKind,
}

impl NodeSeq {
    fn new(kind: SeqKind) -> Self {
        Self {
            items: Vec::new(),
            kind,
        }
    }
}

impl Default for NodeSeq {
    fn default() -> Self {
        Self::new(SeqKind::Seq)
    }
}

impl SerializeSeq for NodeSeq {
    type Ok = AiProofNode;
    type Error = SerError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        self.items.push(value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> SerResult<Self::Ok> {
        Ok(normalize_seq_with_kind(self.items, self.kind))
    }
}

impl SerializeTuple for NodeSeq {
    type Ok = AiProofNode;
    type Error = SerError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> SerResult<Self::Ok> {
        SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for NodeSeq {
    type Ok = AiProofNode;
    type Error = SerError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> SerResult<Self::Ok> {
        SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleVariant for NodeSeq {
    type Ok = AiProofNode;
    type Error = SerError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> SerResult<Self::Ok> {
        SerializeSeq::end(self)
    }
}

impl SerializeStruct for NodeSeq {
    type Ok = AiProofNode;
    type Error = SerError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> SerResult<()> {
        self.items.push(value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> SerResult<Self::Ok> {
        SerializeSeq::end(self)
    }
}

impl ser::SerializeStructVariant for NodeSeq {
    type Ok = AiProofNode;
    type Error = SerError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> SerResult<()> {
        SerializeStruct::serialize_field(self, _key, value)
    }

    fn end(self) -> SerResult<Self::Ok> {
        SerializeSeq::end(self)
    }
}

#[derive(Default)]
struct NodeMap {
    entries: Vec<(AiProofNode, AiProofNode)>,
    next_key: Option<AiProofNode>,
}

impl SerializeMap for NodeMap {
    type Ok = AiProofNode;
    type Error = SerError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> SerResult<()> {
        self.next_key = Some(key.serialize(NodeSerializer)?);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        let key = self
            .next_key
            .take()
            .ok_or_else(|| SerError("serialize_value called before serialize_key".into()))?;
        self.entries.push((key, value.serialize(NodeSerializer)?));
        Ok(())
    }

    fn end(self) -> SerResult<Self::Ok> {
        Ok(AiProofNode::Map(self.entries))
    }
}

#[derive(Debug)]
struct DeError(String);

impl std::fmt::Display for DeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

type DeResult<T> = Result<T, DeError>;

struct NodeDeserializer {
    node: AiProofNode,
}

impl<'de> de::Deserializer<'de> for NodeDeserializer {
    type Error = DeError;

    fn deserialize_any<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::Unit => visitor.visit_unit(),
            AiProofNode::Bool(value) => visitor.visit_bool(*value),
            AiProofNode::U64(value) => visitor.visit_u64(*value),
            AiProofNode::I64(value) => visitor.visit_i64(*value),
            AiProofNode::Ext2(value) => visitor.visit_seq(NodeSeqAccess::from_owned(vec![
                AiProofNode::U64(value[0]),
                AiProofNode::U64(value[1]),
            ])),
            AiProofNode::Ext2s(values) => visitor.visit_seq(NodeSeqAccess::from_owned(
                values.iter().copied().map(AiProofNode::Ext2).collect(),
            )),
            AiProofNode::Bytes(bytes) => visitor.visit_byte_buf(bytes.clone()),
            AiProofNode::U64s(values) => visitor.visit_seq(NodeSeqAccess::from_owned(
                values.iter().copied().map(AiProofNode::U64).collect(),
            )),
            AiProofNode::I64s(values) => visitor.visit_seq(NodeSeqAccess::from_owned(
                values.iter().copied().map(AiProofNode::I64).collect(),
            )),
            AiProofNode::Seq(items) => visitor.visit_seq(NodeSeqAccess::from_slice(items)),
            AiProofNode::Map(items) => visitor.visit_map(NodeMapAccess::new(items)),
            AiProofNode::None => visitor.visit_none(),
            AiProofNode::Some(inner) => visitor.visit_some(NodeDeserializer {
                node: inner.as_ref().clone(),
            }),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::Bool(value) => visitor.visit_bool(*value),
            other => Err(DeError(format!("expected bool, got {other:?}"))),
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }

    fn deserialize_i16<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }

    fn deserialize_i32<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }

    fn deserialize_i64<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::I64(value) => visitor.visit_i64(*value),
            AiProofNode::U64(value) => {
                let value = i64::try_from(*value)
                    .map_err(|_| DeError("unsigned integer does not fit i64".into()))?;
                visitor.visit_i64(value)
            }
            other => Err(DeError(format!("expected signed integer, got {other:?}"))),
        }
    }

    fn deserialize_i128<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::I64(value) => visitor.visit_i128(*value as i128),
            AiProofNode::U64(value) => visitor.visit_i128(*value as i128),
            other => Err(DeError(format!(
                "expected i128-compatible integer, got {other:?}"
            ))),
        }
    }

    fn deserialize_u8<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }

    fn deserialize_u16<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }

    fn deserialize_u32<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }

    fn deserialize_u64<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::U64(value) => visitor.visit_u64(*value),
            AiProofNode::I64(value) => {
                let value = u64::try_from(*value)
                    .map_err(|_| DeError("negative integer does not fit u64".into()))?;
                visitor.visit_u64(value)
            }
            other => Err(DeError(format!("expected unsigned integer, got {other:?}"))),
        }
    }

    fn deserialize_u128<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::U64(value) => visitor.visit_u128(*value as u128),
            AiProofNode::I64(value) => {
                let value = u128::try_from(*value)
                    .map_err(|_| DeError("negative integer does not fit u128".into()))?;
                visitor.visit_u128(value)
            }
            other => Err(DeError(format!(
                "expected u128-compatible integer, got {other:?}"
            ))),
        }
    }

    fn deserialize_f32<V>(self, _visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        Err(DeError(
            "floating-point values are not valid in AI-PoW certificates".into(),
        ))
    }

    fn deserialize_f64<V>(self, _visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        Err(DeError(
            "floating-point values are not valid in AI-PoW certificates".into(),
        ))
    }

    fn deserialize_char<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        let value = match &self.node {
            AiProofNode::U64(value) => u32::try_from(*value)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| DeError("invalid char scalar value".into()))?,
            other => return Err(DeError(format!("expected char integer, got {other:?}"))),
        };
        visitor.visit_char(value)
    }

    fn deserialize_str<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::Bytes(bytes) => {
                let s = std::str::from_utf8(bytes)
                    .map_err(|e| DeError(format!("invalid utf8 string: {e}")))?;
                visitor.visit_str(s)
            }
            other => Err(DeError(format!("expected string bytes, got {other:?}"))),
        }
    }

    fn deserialize_string<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::Bytes(bytes) => {
                let s = String::from_utf8(bytes.clone())
                    .map_err(|e| DeError(format!("invalid utf8 string: {e}")))?;
                visitor.visit_string(s)
            }
            other => Err(DeError(format!("expected string bytes, got {other:?}"))),
        }
    }

    fn deserialize_bytes<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::Bytes(bytes) => visitor.visit_bytes(bytes),
            AiProofNode::U64s(values) => {
                let mut bytes = Vec::with_capacity(values.len());
                for &value in values {
                    bytes.push(
                        u8::try_from(value)
                            .map_err(|_| DeError("packed byte value out of range".into()))?,
                    );
                }
                visitor.visit_byte_buf(bytes)
            }
            other => Err(DeError(format!("expected bytes, got {other:?}"))),
        }
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::None => visitor.visit_none(),
            AiProofNode::Some(inner) => visitor.visit_some(NodeDeserializer {
                node: inner.as_ref().clone(),
            }),
            other => visitor.visit_some(NodeDeserializer {
                node: other.clone(),
            }),
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::Unit => visitor.visit_unit(),
            other => Err(DeError(format!("expected unit, got {other:?}"))),
        }
    }

    fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(NodeSeqAccess::for_node(&self.node))
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        match &self.node {
            AiProofNode::Map(items) => visitor.visit_map(NodeMapAccess::new(items)),
            other => Err(DeError(format!("expected map, got {other:?}"))),
        }
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(NodeSeqAccess::for_struct_node(&self.node, fields.len()))
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_enum(NodeEnumAccess::new(&self.node)?)
    }

    fn deserialize_identifier<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_u32(visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }
}

struct NodeSeqAccess {
    items: Vec<AiProofNode>,
    index: usize,
}

impl NodeSeqAccess {
    fn from_slice(items: &[AiProofNode]) -> Self {
        Self {
            items: items.to_vec(),
            index: 0,
        }
    }

    fn from_owned(items: Vec<AiProofNode>) -> Self {
        Self { items, index: 0 }
    }

    fn for_node(node: &AiProofNode) -> Self {
        match node {
            AiProofNode::Seq(items) => Self::from_slice(items),
            AiProofNode::Ext2(value) => {
                Self::from_owned(vec![AiProofNode::U64(value[0]), AiProofNode::U64(value[1])])
            }
            AiProofNode::Ext2s(values) => {
                Self::from_owned(values.iter().copied().map(AiProofNode::Ext2).collect())
            }
            AiProofNode::U64s(values) => {
                Self::from_owned(values.iter().copied().map(AiProofNode::U64).collect())
            }
            AiProofNode::I64s(values) => {
                Self::from_owned(values.iter().copied().map(AiProofNode::I64).collect())
            }
            other => Self::from_owned(vec![other.clone()]),
        }
    }

    fn for_struct_node(node: &AiProofNode, fields: usize) -> Self {
        let mut access = Self::for_node(node);
        while access.items.len() < fields {
            access.items.push(AiProofNode::Unit);
        }
        access
    }
}

impl<'de> SeqAccess<'de> for NodeSeqAccess {
    type Error = DeError;

    fn next_element_seed<T>(&mut self, seed: T) -> DeResult<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        let Some(node) = self.items.get(self.index).cloned() else {
            return Ok(None);
        };
        self.index += 1;
        seed.deserialize(NodeDeserializer { node }).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len().saturating_sub(self.index))
    }
}

struct NodeMapAccess {
    items: Vec<(AiProofNode, AiProofNode)>,
    index: usize,
    value_ready: bool,
}

impl NodeMapAccess {
    fn new(items: &[(AiProofNode, AiProofNode)]) -> Self {
        Self {
            items: items.to_vec(),
            index: 0,
            value_ready: false,
        }
    }
}

impl<'de> MapAccess<'de> for NodeMapAccess {
    type Error = DeError;

    fn next_key_seed<K>(&mut self, seed: K) -> DeResult<Option<K::Value>>
    where
        K: de::DeserializeSeed<'de>,
    {
        let Some((key, _)) = self.items.get(self.index).cloned() else {
            return Ok(None);
        };
        self.value_ready = true;
        seed.deserialize(NodeDeserializer { node: key }).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> DeResult<V::Value>
    where
        V: de::DeserializeSeed<'de>,
    {
        if !self.value_ready {
            return Err(DeError("deserialize map value before key".into()));
        }
        let (_, value) = self
            .items
            .get(self.index)
            .cloned()
            .ok_or_else(|| DeError("deserialize map value past end".into()))?;
        self.index += 1;
        self.value_ready = false;
        seed.deserialize(NodeDeserializer { node: value })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len().saturating_sub(self.index))
    }
}

struct NodeEnumAccess {
    variant: u32,
    payload: Option<AiProofNode>,
}

impl NodeEnumAccess {
    fn new(node: &AiProofNode) -> DeResult<Self> {
        match node {
            AiProofNode::U64(variant) => Ok(Self {
                variant: u32::try_from(*variant)
                    .map_err(|_| DeError("enum variant index out of range".into()))?,
                payload: None,
            }),
            AiProofNode::Seq(items) => {
                let Some(AiProofNode::U64(variant)) = items.first() else {
                    return Err(DeError("enum sequence missing numeric variant tag".into()));
                };
                let payload = match &items[1..] {
                    [] => None,
                    [single] => Some(single.clone()),
                    rest => Some(AiProofNode::Seq(rest.to_vec())),
                };
                Ok(Self {
                    variant: u32::try_from(*variant)
                        .map_err(|_| DeError("enum variant index out of range".into()))?,
                    payload,
                })
            }
            AiProofNode::U64s(values) => {
                let Some(&variant) = values.first() else {
                    return Err(DeError("enum packed integer sequence is empty".into()));
                };
                let payload = match &values[1..] {
                    [] => None,
                    [single] => Some(AiProofNode::U64(*single)),
                    rest => Some(AiProofNode::U64s(rest.to_vec())),
                };
                Ok(Self {
                    variant: u32::try_from(variant)
                        .map_err(|_| DeError("enum variant index out of range".into()))?,
                    payload,
                })
            }
            other => Err(DeError(format!("expected enum node, got {other:?}"))),
        }
    }
}

impl<'de> EnumAccess<'de> for NodeEnumAccess {
    type Error = DeError;
    type Variant = Self;

    fn variant_seed<V>(self, seed: V) -> DeResult<(V::Value, Self::Variant)>
    where
        V: de::DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(self.variant.into_deserializer())?;
        Ok((variant, self))
    }
}

impl<'de> VariantAccess<'de> for NodeEnumAccess {
    type Error = DeError;

    fn unit_variant(self) -> DeResult<()> {
        if self.payload.is_some() {
            return Err(DeError("unit enum variant has payload".into()));
        }
        Ok(())
    }

    fn newtype_variant_seed<T>(self, seed: T) -> DeResult<T::Value>
    where
        T: de::DeserializeSeed<'de>,
    {
        let node = self
            .payload
            .ok_or_else(|| DeError("newtype enum variant missing payload".into()))?;
        seed.deserialize(NodeDeserializer { node })
    }

    fn tuple_variant<V>(self, _len: usize, visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        let node = self
            .payload
            .ok_or_else(|| DeError("tuple enum variant missing payload".into()))?;
        de::Deserializer::deserialize_seq(NodeDeserializer { node }, visitor)
    }

    fn struct_variant<V>(self, _fields: &'static [&'static str], visitor: V) -> DeResult<V::Value>
    where
        V: Visitor<'de>,
    {
        let node = self
            .payload
            .ok_or_else(|| DeError("struct enum variant missing payload".into()))?;
        de::Deserializer::deserialize_seq(NodeDeserializer { node }, visitor)
    }
}

#[cfg(test)]
mod tests {
    use ai_pow::fiat_shamir::{
        attempt_tile_index, block_state, commitment_key, noise_seed_a, noise_seed_b,
        pow_key_for_nonce,
    };
    use ai_pow::ncmn::{build_ncmn_nonce, NonceAnchors};
    use ai_pow::prover::params_tag;
    use ai_pow::zk_bridge::{expected_layer0_rows, zk_params_from_matmul};

    use super::*;

    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct FakeRecursiveCert {
        cap: Vec<[u64; 5]>,
        ext_values: Vec<[u64; 2]>,
        maybe: Option<Vec<u64>>,
    }

    fn sample_params() -> ZkParams {
        ZkParams {
            m: 64,
            k: 64,
            n: 64,
            noise_rank: 4,
            tile: 8,
            difficulty_bits: 1,
        }
    }

    fn sample_pis() -> CompositePublicInputs {
        let mut pis = CompositePublicInputs::zero();
        pis.cumsum = [-1, 2, -3, 4];
        pis.jackpot = core::array::from_fn(|i| i as u32);
        pis.hash_a = [0x1111_1111; 8];
        pis.hash_b = [0x2222_2222; 8];
        pis.job_key = [0x3333_3333; 8];
        pis.commitment_hash = [0x4444_4444; 8];
        pis.hash_jackpot = [0x5555_5555; 8];
        pis
    }

    fn sample_commitments() -> ZkPublicCommitments {
        ZkPublicCommitments {
            h_a: [0x10; 32],
            h_b: [0x20; 32],
            h_a_chunk: [0x30; 32],
            h_b_chunk: [0x40; 32],
        }
    }

    fn words_le(b: &[u8; 32]) -> [u32; 8] {
        core::array::from_fn(|i| {
            u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])
        })
    }

    fn single_tile_prod_params() -> MatmulParams {
        MatmulParams {
            m: 8,
            k: 512,
            n: 8,
            noise_rank: 32,
            tile: 8,
            spot_checks: 1,
            difficulty_bits: 0,
        }
    }

    fn production_statement_fixture_for_params(
        params: MatmulParams,
        block_commitment: &[u8],
        nonce: &[u8],
    ) -> (
        MatmulParams,
        ZkPublicCommitments,
        CompositePublicInputs,
        usize,
        u32,
    ) {
        params.validate_prod_envelope().unwrap();
        let commitments = ZkPublicCommitments {
            h_a: [0x11; 32],
            h_b: [0x22; 32],
            h_a_chunk: [0x33; 32],
            h_b_chunk: [0x44; 32],
        };
        let tag = params_tag(&params);
        let state = block_state(block_commitment, nonce);
        let kappa = commitment_key(&state, &tag);
        let s_b = noise_seed_b(&kappa, &commitments.h_b);
        let s_a = noise_seed_a(&s_b, &commitments.h_a);
        let found_idx = attempt_tile_index(&state, &tag, &s_a, params.num_tiles()) as u32;
        let pow_key = pow_key_for_nonce(&s_a, nonce);
        let mut pis = CompositePublicInputs::zero();
        pis.job_key = words_le(&kappa);
        pis.commitment_hash = words_le(&pow_key);
        pis.hash_a = words_le(&commitments.h_a_chunk);
        pis.hash_b = words_le(&commitments.h_b_chunk);
        pis.hash_jackpot = [1, 0, 0, 0, 0, 0, 0, 0];
        let trace_height = expected_layer0_rows(&params).required_trace_len();
        (params, commitments, pis, trace_height, found_idx)
    }

    fn production_statement_fixture(
        block_commitment: &[u8],
        nonce: &[u8],
    ) -> (
        MatmulParams,
        ZkPublicCommitments,
        CompositePublicInputs,
        usize,
        u32,
    ) {
        production_statement_fixture_for_params(single_tile_prod_params(), block_commitment, nonce)
    }

    fn build_certificate_slab_with_raw_node<F>(build_certificate: F) -> NounSlab
    where
        F: FnOnce(&mut NounSlab) -> Noun,
    {
        let mut slab = NounSlab::new();
        let params = encode_params(&mut slab, &sample_params());
        let commitments = encode_commitments(&mut slab, &sample_commitments());
        let public_inputs = encode_public_inputs(&mut slab, &sample_pis());
        let certificate = build_certificate(&mut slab);
        let root = T(
            &mut slab,
            &[
                D(AI_POW_CERT_VERSION),
                params,
                D(7),
                D(8_192),
                commitments,
                public_inputs,
                certificate,
            ],
        );
        slab.set_root(root);
        slab
    }

    fn build_ai_pow_artifact_slab(nonce: &[u8], certificate: &NounSlab) -> NounSlab {
        let cert_space = certificate.noun_space();
        let mut slab = NounSlab::new();
        let nonce = bytes_to_atom(&mut slab, nonce);
        let cert = slab.copy_into(unsafe { *certificate.root() }, &cert_space);
        let root = T(&mut slab, &[D(tas!(b"ai-pow")), nonce, cert]);
        slab.set_root(root);
        slab
    }

    fn build_certificate_slab_with_statement_and_raw_node<F>(
        zk_params: &ZkParams,
        found_idx: u32,
        trace_height: usize,
        commitments: &ZkPublicCommitments,
        pis: &CompositePublicInputs,
        build_certificate: F,
    ) -> NounSlab
    where
        F: FnOnce(&mut NounSlab) -> Noun,
    {
        let mut slab = NounSlab::new();
        let params = encode_params(&mut slab, zk_params);
        let commitments = encode_commitments(&mut slab, commitments);
        let public_inputs = encode_public_inputs(&mut slab, pis);
        let certificate = build_certificate(&mut slab);
        let root = T(
            &mut slab,
            &[
                D(AI_POW_CERT_VERSION),
                params,
                D(found_idx as u64),
                D(trace_height as u64),
                commitments,
                public_inputs,
                certificate,
            ],
        );
        slab.set_root(root);
        slab
    }

    #[test]
    fn recursive_certificate_serializer_packs_homogeneous_integer_vectors() {
        let cert = FakeRecursiveCert {
            cap: vec![[1, 2, 3, 4, 5], [6, 7, 8, 9, 10]],
            ext_values: vec![[11, 12], [13, 14]],
            maybe: Some(vec![15, 16, 17]),
        };
        let node = recursive_certificate_to_node(&cert).expect("serialize fake cert");
        let AiProofNode::Seq(fields) = node else {
            panic!("fake certificate struct should encode as seq");
        };
        assert!(matches!(fields[0], AiProofNode::Seq(_)));
        assert!(matches!(fields[1], AiProofNode::Ext2s(_)));
        assert!(matches!(fields[2], AiProofNode::Some(_)));
    }

    #[test]
    fn recursive_certificate_node_roundtrips_through_deserializer() {
        let cert = FakeRecursiveCert {
            cap: vec![[1, 2, 3, 4, 5], [6, 7, 8, 9, 10]],
            ext_values: vec![[11, 12], [13, 14]],
            maybe: Some(vec![15, 16, 17]),
        };
        let node = recursive_certificate_to_node(&cert).expect("serialize fake cert");
        let decoded: FakeRecursiveCert =
            recursive_certificate_from_node(&node).expect("deserialize fake cert");
        assert_eq!(decoded, cert);
    }

    #[test]
    fn canonical_certificate_deserializer_rejects_ignored_extra_fields() {
        let cert = FakeRecursiveCert {
            cap: vec![[1, 2, 3, 4, 5]],
            ext_values: vec![[11, 12]],
            maybe: Some(vec![15]),
        };
        let mut node = recursive_certificate_to_node(&cert).expect("serialize fake cert");
        match &mut node {
            AiProofNode::Seq(fields) => fields.push(AiProofNode::U64(999)),
            _ => panic!("fake certificate struct should encode as seq"),
        }

        let decoded: FakeRecursiveCert =
            recursive_certificate_from_node(&node).expect("serde ignores trailing fields");
        assert_eq!(decoded, cert);
        assert!(matches!(
            canonical_certificate_from_node::<FakeRecursiveCert>(&node),
            Err(CertificateNounError::NonCanonicalProofNode)
        ));

        match &mut node {
            AiProofNode::Seq(fields) => {
                fields.pop();
                fields.push(AiProofNode::Unit);
            }
            _ => panic!("fake certificate struct should encode as seq"),
        }
        assert!(matches!(
            canonical_certificate_from_node::<FakeRecursiveCert>(&node),
            Err(CertificateNounError::NonCanonicalProofNode)
        ));
    }

    #[test]
    fn recursive_certificate_serializer_packs_two_felt_tuples_as_ext2_aura_nodes() {
        #[derive(serde::Serialize)]
        struct TwoFeltCarrier {
            scalar: [u64; 2],
            vector: Vec<[u64; 2]>,
            plain_u64s: Vec<u64>,
        }

        let cert = TwoFeltCarrier {
            scalar: [1, 2],
            vector: vec![[3, 4], [5, 6]],
            plain_u64s: vec![7, 8],
        };
        let node = recursive_certificate_to_node(&cert).expect("serialize two-felt carrier");
        let AiProofNode::Seq(fields) = node else {
            panic!("carrier struct should encode as seq");
        };
        assert_eq!(fields[0], AiProofNode::Ext2([1, 2]));
        assert_eq!(fields[1], AiProofNode::Ext2s(vec![[3, 4], [5, 6]]));
        assert_eq!(fields[2], AiProofNode::U64s(vec![7, 8]));
    }

    #[test]
    fn top_level_certificate_noun_has_hoon_shape_and_structured_certificate_tail() {
        let params = sample_params();
        let commitments = sample_commitments();
        let pis = sample_pis();
        let cert = AiProofNode::Seq(vec![AiProofNode::U64s(vec![1, 2, 3, 4])]);
        let slab =
            build_ai_pow_certificate_noun_from_node(&params, 7, 8_192, &commitments, &pis, &cert);
        let jammed = slab.jam();
        assert!(
            jammed.len() > 64,
            "certificate noun must persist structure, not a tiny placeholder"
        );
        let space = slab.noun_space();
        let root = unsafe { *slab.root() };
        let cell = root.in_space(&space).as_cell().expect("certificate cell");
        assert_eq!(
            cell.head().as_atom().unwrap().as_u64().unwrap(),
            AI_POW_CERT_VERSION
        );
    }

    #[test]
    fn certificate_noun_roundtrips_through_jam_cue_and_bounded_decoder() {
        let params = sample_params();
        let commitments = sample_commitments();
        let pis = sample_pis();
        let cert = AiProofNode::Seq(vec![
            AiProofNode::Bytes(vec![1, 2, 0, 0]),
            AiProofNode::Ext2([7, 8]),
            AiProofNode::Ext2s(vec![[9, 10], [11, 12]]),
            AiProofNode::U64s(vec![3, 4]),
            AiProofNode::I64s(vec![-5, 6]),
        ]);
        let slab =
            build_ai_pow_certificate_noun_from_node(&params, 9, 16_384, &commitments, &pis, &cert);

        let jammed = slab.jam();
        let mut cued: NounSlab = NounSlab::new();
        let root = cued.cue_into(jammed).expect("cue certificate noun");
        cued.set_root(root);

        let decoded = decode_ai_pow_certificate_slab(&cued, CertificateNounLimits::default())
            .expect("decode certificate noun");
        assert_eq!(decoded.version, AI_POW_CERT_VERSION);
        assert_eq!(decoded.zk_params, params);
        assert_eq!(decoded.found_idx, 9);
        assert_eq!(decoded.trace_height, 16_384);
        assert_eq!(decoded.commitments, commitments);
        assert_eq!(decoded.public_inputs, pis);
        assert_eq!(decoded.certificate, cert);
    }

    #[test]
    fn certificate_statement_precheck_binds_noun_metadata_to_nonce_and_target() {
        let block = b"noun-certificate-block";
        let nonce = b"noun-certificate-nonce";
        let target = [0xffu8; 32];
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture(block, nonce);
        let certificate = AiProofNode::Seq(vec![AiProofNode::U64(42)]);
        let slab = build_ai_pow_certificate_noun_from_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            &certificate,
        );
        let decoded = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect("decode certificate noun");

        precheck_ai_pow_certificate_statement(&decoded, block, nonce, &params, &target)
            .expect("honest certificate metadata should precheck");

        assert!(matches!(
            precheck_ai_pow_certificate_statement(
                &decoded, block, b"wrong-nonce", &params, &target
            ),
            Err(CertificateNounError::Statement(_))
        ));
        assert!(matches!(
            precheck_ai_pow_certificate_statement(&decoded, block, nonce, &params, &[0u8; 32]),
            Err(CertificateNounError::Statement(
                BridgeError::FoundAboveTarget
            ))
        ));

        let mut wrong_params = params;
        wrong_params.difficulty_bits += 1;
        assert!(matches!(
            precheck_ai_pow_certificate_statement(&decoded, block, nonce, &wrong_params, &target),
            Err(CertificateNounError::ZkParamsMismatch { .. })
        ));
    }

    #[test]
    fn certificate_statement_precheck_fails_closed_for_multi_tile_full_matmul_claim() {
        let block = b"multi-tile-noun-certificate-block";
        let nonce = b"multi-tile-noun-certificate-nonce";
        let target = [0xffu8; 32];
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture_for_params(MatmulParams::PROD, block, nonce);
        assert!(params.num_tiles() > 1);
        let certificate = AiProofNode::Seq(vec![AiProofNode::U64(42)]);
        let slab = build_ai_pow_certificate_noun_from_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            &certificate,
        );
        let decoded = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect("decode certificate noun");

        assert!(matches!(
            precheck_ai_pow_certificate_statement(&decoded, block, nonce, &params, &target),
            Err(CertificateNounError::Statement(
                BridgeError::FullMatmulProofUnavailable { .. }
            ))
        ));
    }

    #[test]
    fn decoded_certificate_verify_prechecks_statement_before_proof_node_reconstruction() {
        let block = b"precheck-before-proof-node-block";
        let nonce = b"precheck-before-proof-node-nonce";
        let target = [0xffu8; 32];
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture(block, nonce);
        let slab = build_ai_pow_certificate_noun_from_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            &AiProofNode::Unit,
        );
        let decoded = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect("decode certificate noun");

        assert!(matches!(
            verify_decoded_ai_pow_certificate(&decoded, block, b"wrong-nonce", &params, &target),
            Err(CertificateNounError::Statement(_))
        ));
    }

    #[test]
    fn decoded_ncmn_certificate_verify_prechecks_anchor_before_proof_node_reconstruction() {
        let puzzle_id = b"ncmn-precheck-before-proof-node-puzzle";
        let candidate_nck = [0x5au8; 32];
        let nonce = build_ncmn_nonce(&NonceAnchors::nck_only(candidate_nck), 17);
        let target = [0xffu8; 32];
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture(puzzle_id, &nonce);
        let slab = build_ai_pow_certificate_noun_from_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            &AiProofNode::Unit,
        );
        let decoded = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect("decode certificate noun");
        let mut wrong_anchor = candidate_nck;
        wrong_anchor[0] ^= 1;

        assert!(matches!(
            verify_decoded_ai_pow_ncmn_certificate(
                &decoded, puzzle_id, &wrong_anchor, &nonce, &params, &target
            ),
            Err(CertificateNounError::NonceAnchorMismatch)
        ));
    }

    #[test]
    fn jammed_artifact_verify_prechecks_anchor_before_proof_node_decode() {
        let puzzle_id = b"jam-anchor-before-proof-node-puzzle";
        let candidate_nck = [0x61u8; 32];
        let nonce = build_ncmn_nonce(&NonceAnchors::nck_only(candidate_nck), 21);
        let target = [0xffu8; 32];
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture(puzzle_id, &nonce);
        let cert_slab = build_certificate_slab_with_statement_and_raw_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            |_| D(0),
        );
        let artifact_slab = build_ai_pow_artifact_slab(&nonce, &cert_slab);
        let jammed = artifact_slab.jam();
        let mut wrong_anchor = candidate_nck;
        wrong_anchor[0] ^= 1;

        assert!(matches!(
            verify_ai_pow_ncmn_artifact_jam(
                &jammed,
                CertificateNounLimits::default(),
                puzzle_id,
                &wrong_anchor,
                &params,
                &target
            ),
            Err(CertificateNounError::NonceAnchorMismatch)
        ));
    }

    #[test]
    fn jammed_artifact_verify_prechecks_statement_before_proof_node_decode() {
        let puzzle_id = b"jam-statement-before-proof-node-puzzle";
        let candidate_nck = [0x62u8; 32];
        let nonce = build_ncmn_nonce(&NonceAnchors::nck_only(candidate_nck), 22);
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture(puzzle_id, &nonce);
        let cert_slab = build_certificate_slab_with_statement_and_raw_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            |_| D(0),
        );
        let artifact_slab = build_ai_pow_artifact_slab(&nonce, &cert_slab);
        let jammed = artifact_slab.jam();

        assert!(matches!(
            verify_ai_pow_ncmn_artifact_jam(
                &jammed,
                CertificateNounLimits::default(),
                puzzle_id,
                &candidate_nck,
                &params,
                &[0u8; 32]
            ),
            Err(CertificateNounError::Statement(
                BridgeError::FoundAboveTarget
            ))
        ));
    }

    #[test]
    fn ai_pow_artifact_decoder_binds_nonce_and_certificate_shape() {
        let puzzle_id = b"artifact-puzzle-id";
        let candidate_nck = [0x42u8; 32];
        let nonce = build_ncmn_nonce(&NonceAnchors::nck_only(candidate_nck), 99);
        let target = [0xffu8; 32];
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture(puzzle_id, &nonce);
        let certificate = AiProofNode::Seq(vec![AiProofNode::U64(42)]);
        let cert_slab = build_ai_pow_certificate_noun_from_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            &certificate,
        );
        let artifact_slab = build_ai_pow_artifact_slab(&nonce, &cert_slab);

        let decoded = decode_ai_pow_artifact_slab(&artifact_slab, CertificateNounLimits::default())
            .expect("decode ai-pow artifact");
        assert_eq!(decoded.nonce, nonce);
        assert_eq!(decoded.certificate.found_idx, found_idx);
        assert_eq!(decoded.certificate.commitments, commitments);
        assert_eq!(decoded.certificate.public_inputs, pis);
        assert_eq!(decoded.certificate.certificate, certificate);

        precheck_ai_pow_ncmn_artifact_statement(
            &decoded, puzzle_id, &candidate_nck, &params, &target,
        )
        .expect("honest artifact statement should precheck");

        let mut wrong_anchor = candidate_nck;
        wrong_anchor[0] ^= 1;
        assert!(matches!(
            precheck_ai_pow_ncmn_artifact_statement(
                &decoded, puzzle_id, &wrong_anchor, &params, &target,
            ),
            Err(CertificateNounError::NonceAnchorMismatch)
        ));
    }

    #[test]
    fn ai_pow_artifact_jam_decoder_enforces_byte_limit_before_cue() {
        let params = sample_params();
        let commitments = sample_commitments();
        let pis = sample_pis();
        let cert_slab = build_ai_pow_certificate_noun_from_node(
            &params,
            0,
            16_384,
            &commitments,
            &pis,
            &AiProofNode::Unit,
        );
        let nonce = build_ncmn_nonce(&NonceAnchors::nck_only([0x44; 32]), 8);
        let artifact_slab = build_ai_pow_artifact_slab(&nonce, &cert_slab);
        let jammed = artifact_slab.jam();

        let decoded = decode_ai_pow_artifact_jam(&jammed, CertificateNounLimits::default())
            .expect("decode artifact jam");
        assert_eq!(decoded.nonce, nonce);

        let mut non_canonical = jammed.to_vec();
        non_canonical.push(0xff);
        assert!(matches!(
            decode_ai_pow_artifact_jam(&non_canonical, CertificateNounLimits::default()),
            Err(CertificateNounError::NonCanonicalJam)
        ));

        let mut node_limits = CertificateNounLimits::default();
        node_limits.max_total_nodes = 1;
        assert!(matches!(
            decode_ai_pow_artifact_jam(&jammed, node_limits),
            Err(CertificateNounError::LimitExceeded("jam noun count"))
        ));

        let mut depth_limits = CertificateNounLimits::default();
        depth_limits.max_depth = 1;
        assert!(matches!(
            decode_ai_pow_artifact_jam(&jammed, depth_limits),
            Err(CertificateNounError::LimitExceeded("jam noun depth"))
        ));

        let mut atom_limits = CertificateNounLimits::default();
        atom_limits.max_atom_bytes = 0;
        assert!(matches!(
            decode_ai_pow_artifact_jam(&jammed, atom_limits),
            Err(CertificateNounError::LimitExceeded("jam atom bytes"))
        ));

        let mut limits = CertificateNounLimits::default();
        limits.max_jam_bytes = jammed.len() - 1;
        assert!(matches!(
            decode_ai_pow_artifact_jam(&jammed, limits),
            Err(CertificateNounError::JammedLengthExceeded { limit, actual })
                if limit == jammed.len() - 1 && actual == jammed.len()
        ));

        let err = decode_ai_pow_artifact_jam(&[], CertificateNounLimits::default())
            .expect_err("malformed jam must reject");
        assert!(matches!(err, CertificateNounError::Cue(_)));
    }

    #[test]
    fn ai_pow_artifact_decoder_rejects_malformed_nonce_and_tag() {
        let params = sample_params();
        let commitments = sample_commitments();
        let pis = sample_pis();
        let cert_slab = build_ai_pow_certificate_noun_from_node(
            &params,
            0,
            16_384,
            &commitments,
            &pis,
            &AiProofNode::Unit,
        );

        let bad_magic = [0u8; ai_pow::ncmn::NCMN_NONCE_LEN];
        let bad_magic_slab = build_ai_pow_artifact_slab(&bad_magic, &cert_slab);
        assert!(matches!(
            decode_ai_pow_artifact_slab(&bad_magic_slab, CertificateNounLimits::default()),
            Err(CertificateNounError::Nonce(_))
        ));

        let external_nonce = build_ncmn_nonce(
            &NonceAnchors {
                nck_commitment: [0x11; 32],
                external_commitment: Some([0x22; 32]),
            },
            7,
        );
        let external_slab = build_ai_pow_artifact_slab(&external_nonce, &cert_slab);
        assert!(matches!(
            decode_ai_pow_artifact_slab(&external_slab, CertificateNounLimits::default()),
            Err(CertificateNounError::NonceExternalCommitmentPresent)
        ));

        let oversized_nonce = [0xFFu8; ai_pow::ncmn::NCMN_NONCE_LEN + 1];
        let oversized_slab = build_ai_pow_artifact_slab(&oversized_nonce, &cert_slab);
        assert!(matches!(
            decode_ai_pow_artifact_slab(&oversized_slab, CertificateNounLimits::default()),
            Err(CertificateNounError::PackedLengthMismatch {
                tag: "ncmn nonce",
                declared: ai_pow::ncmn::NCMN_NONCE_LEN,
                actual: 81,
            })
        ));

        let cert_space = cert_slab.noun_space();
        let mut wrong_tag_slab: NounSlab = NounSlab::new();
        let nonce = build_ncmn_nonce(&NonceAnchors::nck_only([0x33; 32]), 7);
        let nonce = bytes_to_atom(&mut wrong_tag_slab, &nonce);
        let cert = wrong_tag_slab.copy_into(unsafe { *cert_slab.root() }, &cert_space);
        let root = T(&mut wrong_tag_slab, &[D(tas!(b"not-ai")), nonce, cert]);
        wrong_tag_slab.set_root(root);
        assert!(matches!(
            decode_ai_pow_artifact_slab(&wrong_tag_slab, CertificateNounLimits::default()),
            Err(CertificateNounError::Shape("expected %ai-pow artifact"))
        ));
    }

    #[test]
    fn ncmn_certificate_statement_precheck_enforces_nonce_anchor() {
        let puzzle_id = b"noun-certificate-puzzle-id";
        let candidate_nck = [0x4eu8; 32];
        let nonce = build_ncmn_nonce(&NonceAnchors::nck_only(candidate_nck), 7);
        let target = [0xffu8; 32];
        let (params, commitments, pis, trace_height, found_idx) =
            production_statement_fixture(puzzle_id, &nonce);
        let certificate = AiProofNode::Seq(vec![AiProofNode::U64(42)]);
        let slab = build_ai_pow_certificate_noun_from_node(
            &zk_params_from_matmul(&params),
            found_idx,
            trace_height,
            &commitments,
            &pis,
            &certificate,
        );
        let decoded = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect("decode certificate noun");

        precheck_ai_pow_ncmn_certificate_statement(
            &decoded, puzzle_id, &candidate_nck, &nonce, &params, &target,
        )
        .expect("honest NCMN certificate metadata should precheck");

        let mut wrong_anchor = candidate_nck;
        wrong_anchor[0] ^= 1;
        assert!(matches!(
            precheck_ai_pow_ncmn_certificate_statement(
                &decoded, puzzle_id, &wrong_anchor, &nonce, &params, &target,
            ),
            Err(CertificateNounError::NonceAnchorMismatch)
        ));

        let external_nonce = build_ncmn_nonce(
            &NonceAnchors {
                nck_commitment: candidate_nck,
                external_commitment: Some([0x77u8; 32]),
            },
            7,
        );
        assert!(matches!(
            precheck_ai_pow_ncmn_certificate_statement(
                &decoded, puzzle_id, &candidate_nck, &external_nonce, &params, &target,
            ),
            Err(CertificateNounError::NonceExternalCommitmentPresent)
        ));

        assert!(matches!(
            precheck_ai_pow_ncmn_certificate_statement(
                &decoded, puzzle_id, &candidate_nck, b"not-an-ncmn-nonce", &params, &target,
            ),
            Err(CertificateNounError::Nonce(_))
        ));
    }

    #[test]
    fn certificate_noun_decoder_rejects_oversized_packed_atom() {
        let slab = build_certificate_slab_with_raw_node(|slab| {
            let data = bytes_to_atom(slab, &[0xffu8; 9]);
            T(slab, &[D(tas!(b"u64s")), D(1), data])
        });

        let err = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect_err("oversized packed u64 atom should be rejected");
        assert!(matches!(
            err,
            CertificateNounError::PackedLengthMismatch {
                tag: "u64s",
                declared: 8,
                actual: 9,
            }
        ));
    }

    #[test]
    fn certificate_noun_decoder_rejects_noncanonical_ext2_limb() {
        let slab = build_certificate_slab_with_raw_node(|slab| {
            let data = ext2_to_atom(slab, [GOLDILOCKS_MODULUS, 1]);
            T(slab, &[D(tas!(b"ext2")), data])
        });

        let err = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect_err("non-canonical Goldilocks limb should be rejected");
        assert!(matches!(
            err,
            CertificateNounError::NonCanonicalField { field: "ext2.c0" }
        ));
    }

    #[test]
    fn certificate_noun_decoder_enforces_list_limits() {
        let params = sample_params();
        let commitments = sample_commitments();
        let pis = sample_pis();
        let cert = AiProofNode::Seq(vec![AiProofNode::U64(1), AiProofNode::U64(2)]);
        let slab =
            build_ai_pow_certificate_noun_from_node(&params, 7, 8_192, &commitments, &pis, &cert);
        let limits = CertificateNounLimits {
            max_list_items: 1,
            ..CertificateNounLimits::default()
        };

        let err = decode_ai_pow_certificate_slab(&slab, limits)
            .expect_err("proof-node list limit should be enforced");
        assert!(matches!(
            err,
            CertificateNounError::LimitExceeded("proof-node list length")
        ));
    }

    #[test]
    fn certificate_noun_decoder_rejects_wrong_version_and_bad_tag() {
        let slab = build_certificate_slab_with_raw_node(|slab| T(slab, &[D(tas!(b"wat")), D(0)]));
        let err = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect_err("bad proof-node tag should be rejected");
        assert!(matches!(err, CertificateNounError::InvalidTag(_)));

        let mut slab: NounSlab = NounSlab::new();
        let params = encode_params(&mut slab, &sample_params());
        let commitments = encode_commitments(&mut slab, &sample_commitments());
        let public_inputs = encode_public_inputs(&mut slab, &sample_pis());
        let cert = AiProofNode::Unit.to_noun(&mut slab);
        let root = T(
            &mut slab,
            &[D(99), params, D(7), D(8_192), commitments, public_inputs, cert],
        );
        slab.set_root(root);

        let err = decode_ai_pow_certificate_slab(&slab, CertificateNounLimits::default())
            .expect_err("wrong certificate version should be rejected");
        assert!(matches!(err, CertificateNounError::UnsupportedVersion(99)));
    }

    /// Heavy opt-in measurement: proves a real recursive L1 certificate,
    /// converts that certificate into the Hoon noun, jams/cues it, and decodes
    /// it back through the bounded parser.
    ///
    /// Run with:
    ///
    /// ```text
    /// cargo test -p ai-pow-miner --features node \
    ///   real_recursive_certificate_noun_roundtrips_and_prints_size -- --ignored --nocapture
    /// ```
    #[ignore = "real recursive proof generation is intentionally opt-in"]
    #[test]
    fn real_recursive_certificate_noun_roundtrips_and_prints_size() {
        let zk = ZkParams {
            m: 8,
            k: 16,
            n: 8,
            noise_rank: 2,
            tile: 2,
            difficulty_bits: 0,
        };
        let profile = ai_pow_zk::CircuitConfig::TEST_PEARL;
        let trace = ai_pow_zk::CompositeTrace::baseline_min();

        let start = std::time::Instant::now();
        eprintln!("real recursive certificate noun: proving canonical certificate");
        let run = ai_pow_zk::recursion::prove_canonical_ai_pow_certificate(&zk, &profile, trace)
            .expect("real recursive certificate should prove");
        let recursive_prove_ms = start.elapsed().as_millis();
        eprintln!(
            "real recursive certificate noun: serializing recursive certificate to proof-node"
        );
        let certificate_node = recursive_certificate_to_node(&run.l1_cert)
            .expect("serialize real recursive cert node");
        eprintln!("real recursive certificate noun: reconstructing directly from proof-node");
        let direct_cert = ai_pow_recursive_certificate_from_node(&certificate_node)
            .expect("reconstruct direct recursive certificate from proof-node");
        ai_pow_zk::recursion::verify_recursive_certificate(&direct_cert, &run.public_inputs)
            .expect("direct reconstructed recursive certificate verifies");
        eprintln!("real recursive certificate noun: encoding structured noun");

        let commitments = sample_commitments();
        let cert = build_ai_pow_certificate_noun_from_node(
            &zk, 0, run.composite_trace_height, &commitments, &run.public_inputs, &certificate_node,
        );
        let jammed = cert.jam();
        let l1_postcard_bytes = ai_pow_zk::recursion::encode_recursive_certificate(&run.l1_cert)
            .expect("postcard L1 certificate")
            .len();

        let mut cued: NounSlab = NounSlab::new();
        eprintln!("real recursive certificate noun: cueing jammed noun");
        let root = cued.cue_into(jammed.clone()).expect("cue real cert noun");
        cued.set_root(root);
        eprintln!("real recursive certificate noun: bounded decoding noun");
        let decoded = decode_ai_pow_certificate_slab(&cued, CertificateNounLimits::default())
            .expect("bounded decode real cert noun");
        eprintln!("real recursive certificate noun: reconstructing recursive certificate");
        let decoded_cert = ai_pow_recursive_certificate_from_node(&decoded.certificate)
            .expect("reconstruct recursive certificate from proof-node");
        eprintln!("real recursive certificate noun: verifying reconstructed recursive certificate");
        ai_pow_zk::recursion::verify_recursive_certificate(&decoded_cert, &run.public_inputs)
            .expect("reconstructed recursive certificate verifies");

        assert_eq!(decoded.version, AI_POW_CERT_VERSION);
        assert_eq!(decoded.zk_params, zk);
        assert_eq!(decoded.found_idx, 0);
        assert_eq!(decoded.trace_height, run.composite_trace_height);
        assert_eq!(decoded.public_inputs, run.public_inputs);
        assert_eq!(decoded.commitments, commitments);
        assert!(
            !matches!(decoded.certificate, AiProofNode::Unit),
            "real recursive certificate must not collapse to a unit proof-node"
        );
        assert!(
            jammed.len() < 2 * 1024 * 1024,
            "real recursive certificate noun unexpectedly large: {} bytes",
            jammed.len()
        );

        eprintln!(
            "real recursive certificate noun: jammed={} bytes ({:.2} KiB), postcard_l1={} bytes ({:.2} KiB), prove_ms={}, l1_build_ms={}, l1_verify_ms={}, l1_cert_ms={}",
            jammed.len(),
            jammed.len() as f64 / 1024.0,
            l1_postcard_bytes,
            l1_postcard_bytes as f64 / 1024.0,
            recursive_prove_ms,
            run.l1_circuit_build_ms,
            run.l1_in_circuit_verify_ms,
            run.l1_outer_cert_ms,
        );
    }
}
