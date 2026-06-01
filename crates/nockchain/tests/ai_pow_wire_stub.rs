//! Regression guard for the in-flight AI-PoW consensus wire.
//!
//! Until the final `%ai-pow` recursive verifier lands, the Hoon consensus
//! branch must fail closed: the structured certificate type may exist, but no
//! `%ai-pow` block or miner poke may satisfy consensus without verification.

const TYPES_HOON: &str = include_str!("../../../hoon/apps/dumbnet/lib/types.hoon");
const TX_ENGINE_1_HOON: &str = include_str!("../../../hoon/common/tx-engine-1.hoon");
const CONSENSUS_HOON: &str = include_str!("../../../hoon/apps/dumbnet/lib/consensus.hoon");
const INNER_HOON: &str = include_str!("../../../hoon/apps/dumbnet/inner.hoon");
const AI_POW_MINER_LIB_RS: &str = include_str!("../../ai-pow-miner/src/lib.rs");
const AI_POW_MINER_CERT_NOUN_RS: &str = include_str!("../../ai-pow-miner/src/certificate_noun.rs");
const AI_POW_MINER_RUN_RS: &str = include_str!("../../ai-pow-miner/src/run.rs");
const AI_POW_MINER_BIN_RS: &str = include_str!("../../ai-pow-miner/src/bin/ai_pow_mine.rs");
const AI_POW_LIB_RS: &str = include_str!("../../ai-pow/src/lib.rs");
const AI_POW_VERIFIER_RS: &str = include_str!("../../ai-pow/src/verifier.rs");
const AI_POW_ZK_RECURSION_RS: &str = include_str!("../../ai-pow-zk/src/recursion.rs");
const AI_POW_ZK_BRIDGE_RS: &str = include_str!("../../ai-pow/src/zk_bridge.rs");

#[test]
fn ai_pow_consensus_wire_is_structured_but_fail_closed_without_verifier() {
    assert!(
        TX_ENGINE_1_HOON.contains("+$  ai-blake  @uxblake")
            && TX_ENGINE_1_HOON.contains("+$  ai-ncmn   @uxncmn")
            && TX_ENGINE_1_HOON.contains("+$  ai-ext2   @uxfelt")
            && TX_ENGINE_1_HOON.contains("+$  ai-proof-node")
            && TX_ENGINE_1_HOON.contains("[%ext2 value=ai-ext2]")
            && TX_ENGINE_1_HOON.contains("[%ext2s len=@ud data=ai-ext2s]")
            && TX_ENGINE_1_HOON.contains("[%u64s len=@ud data=@]")
            && TX_ENGINE_1_HOON.contains("+$  pow-artifact")
            && TX_ENGINE_1_HOON.contains("?=([%ai-pow *] u.pow.pag)")
            && CONSENSUS_HOON.contains("++  pow-artifact-to-proof-version")
            && CONSENSUS_HOON.contains("?=([%ai-pow *] pow)")
            && TYPES_HOON.contains("[%ai-pow nonce=ai-ncmn cert=ai-pow-certificate]")
            && !TX_ENGINE_1_HOON.contains("+$  ai-recursive-fri-proof")
            && !TX_ENGINE_1_HOON.contains("+$  ai-fri-proof")
            && !TYPES_HOON.contains("+$  ai-fri-proof"),
        "%ai-pow wire must expose the structured recursive certificate type, \
         custom digest and 2-felt auras, keep page storage generic enough for \
         hoonc, and expose no plain AI ZKP proof type"
    );
    assert!(
        INNER_HOON.contains("do-pow: %ai-pow verifier not wired; rejected")
            && INNER_HOON.contains("Height-gating alone is not proof verification")
            && INNER_HOON.contains("Emitting %mine-ai here would")
            && CONSENSUS_HOON.contains("A typed certificate is not itself a target check")
            && !INNER_HOON.contains("(set-pow:min [%ai-pow")
            && !INNER_HOON.contains("(heard-block /poke/ai-pow-miner now candidate-block.m.k eny)"),
        "%ai-pow consensus must fail closed until the recursive certificate \
         verifier is wired; it must not persist or broadcast unverified AI proofs"
    );
    assert!(
        AI_POW_MINER_RUN_RS.contains("build_ai_pow_certificate_poke")
            && AI_POW_MINER_RUN_RS.contains("[%command %pow %ai-pow nonce cert]")
            && AI_POW_MINER_RUN_RS.contains("nonce: &NcmnNonce")
            && AI_POW_MINER_RUN_RS.contains("AiPowCertificatePokeError")
            && AI_POW_MINER_RUN_RS
                .contains("build_ai_pow_certificate_poke_rejects_non_canonical_nonce")
            && AI_POW_MINER_RUN_RS.contains("refusing to submit legacy nonce/tile artifact")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn decode_ai_pow_certificate_slab")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn decode_ai_pow_artifact_slab")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn decode_ai_pow_artifact_jam")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub struct AiPowArtifactShape")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn precheck_ai_pow_certificate_statement")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("pub fn precheck_ai_pow_ncmn_certificate_statement")
            && AI_POW_MINER_CERT_NOUN_RS.contains("verify_ai_pow_full_matmul_production_statement")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn precheck_ai_pow_ncmn_artifact_statement")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn verify_decoded_ai_pow_ncmn_artifact")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn verify_ai_pow_ncmn_artifact_jam")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn verify_decoded_ai_pow_ncmn_certificate")
            && AI_POW_MINER_CERT_NOUN_RS.contains("fn preflight_ai_pow_artifact_jam")
            && AI_POW_MINER_CERT_NOUN_RS.contains("max_jam_bytes")
            && AI_POW_MINER_CERT_NOUN_RS.contains("CuePanic")
            && AI_POW_MINER_CERT_NOUN_RS.contains("NonCanonicalJam")
            && AI_POW_MINER_CERT_NOUN_RS.contains("jam noun count")
            && AI_POW_MINER_CERT_NOUN_RS.contains("jam atom bytes")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub fn verify_ai_pow_production_statement")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub fn verify_ai_pow_selected_tile_statement")
            && AI_POW_ZK_BRIDGE_RS.contains("fn verify_ai_pow_selected_tile_statement")
            && AI_POW_ZK_BRIDGE_RS
                .contains("pub fn verify_ai_pow_full_matmul_production_statement")
            && AI_POW_ZK_BRIDGE_RS.contains("FullMatmulProofUnavailable")
            && AI_POW_MINER_CERT_NOUN_RS.contains("CertificateNounLimits")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("certificate_noun_decoder_rejects_oversized_packed_atom")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("certificate_noun_roundtrips_through_jam_cue_and_bounded_decoder")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("certificate_statement_precheck_binds_noun_metadata_to_nonce_and_target")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("ncmn_certificate_statement_precheck_enforces_nonce_anchor")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("ai_pow_artifact_decoder_binds_nonce_and_certificate_shape")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("ai_pow_artifact_jam_decoder_enforces_byte_limit_before_cue")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("ai_pow_artifact_decoder_rejects_malformed_nonce_and_tag")
            && AI_POW_MINER_CERT_NOUN_RS.contains("AiProofNode::Ext2s")
            && AI_POW_MINER_CERT_NOUN_RS.contains(
                "recursive_certificate_serializer_packs_two_felt_tuples_as_ext2_aura_nodes"
            )
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("real_recursive_certificate_noun_roundtrips_and_prints_size")
            && !AI_POW_MINER_RUN_RS.contains("build_mined_poke"),
        "AI-PoW miner submissions must use the canonical recursive certificate \
         command payload, not a legacy nonce/tile mined artifact, and must keep \
         bounded structured noun decode coverage"
    );
    let verifier_reexports = AI_POW_LIB_RS
        .split("pub use crate::verifier::{")
        .nth(1)
        .and_then(|tail| tail.split("};").next())
        .expect("ai-pow crate root must have a verifier re-export block");
    assert!(
        verifier_reexports.contains("verify_at_target")
            && verifier_reexports.contains("verify_ncmn_at_target")
            && !verifier_reexports
                .split(',')
                .map(str::trim)
                .any(|item| item == "verify")
            && AI_POW_VERIFIER_RS.contains("Non-production helper")
            && AI_POW_VERIFIER_RS.contains("it is not")
            && AI_POW_VERIFIER_RS.contains("re-exported from the crate root"),
        "plain params-derived-target verification must not be advertised as \
         the production crate-root verifier; consensus must use explicit-target \
         or NCMN verifier boundaries"
    );
    assert!(
        AI_POW_ZK_BRIDGE_RS.contains("pub fn prove_ai_pow_recursive_certificate")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub fn verify_ai_pow_production_statement")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub fn verify_ai_pow_selected_tile_statement")
            && AI_POW_ZK_BRIDGE_RS.contains("fn verify_ai_pow_selected_tile_statement")
            && AI_POW_ZK_BRIDGE_RS
                .contains("pub fn verify_ai_pow_full_matmul_production_statement")
            && AI_POW_ZK_BRIDGE_RS.contains("FullMatmulProofUnavailable")
            && AI_POW_ZK_BRIDGE_RS.contains("pub(crate) struct ZkProofArtifact")
            && AI_POW_ZK_BRIDGE_RS.contains("pub(crate) struct AiPowProductionArtifact")
            && AI_POW_ZK_BRIDGE_RS.contains("pub(crate) struct AiPowConsensusArtifact")
            && AI_POW_ZK_BRIDGE_RS.contains("fn prove_ai_pow_block")
            && AI_POW_ZK_BRIDGE_RS.contains("fn verify_ai_pow_block")
            && AI_POW_ZK_BRIDGE_RS.contains("fn verify_ai_pow_consensus_artifact")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub struct ZkProofArtifact")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub struct AiPowProductionArtifact")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub struct AiPowConsensusArtifact")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub fn prove_ai_pow_block")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub fn verify_ai_pow_block")
            && !AI_POW_ZK_BRIDGE_RS.contains("pub fn verify_ai_pow_consensus_artifact"),
        "ai-pow must not publish legacy Layer-0 proof artifacts or byte \
         envelopes as normal production APIs; the production boundary is \
         recursive certificate generation plus full-matmul statement \
         verification"
    );
    let target_check = AI_POW_MINER_BIN_RS
        .find("verify_ncmn_at_target")
        .expect("production miner must check the plain matmul target hit at the NCMN boundary");
    let recursive_prove = AI_POW_MINER_BIN_RS
        .find("let run = prove_ai_pow_recursive_certificate")
        .expect("production miner must build the recursive certificate");
    let recursive_certificate_fn = AI_POW_ZK_BRIDGE_RS
        .split("pub fn prove_ai_pow_recursive_certificate")
        .nth(1)
        .and_then(|tail| {
            tail.split("/// Crate-internal Layer-0 verifier-only ZK API.")
                .next()
        })
        .expect("recursive certificate prover body must be present");
    let recursive_prod_envelope = recursive_certificate_fn
        .find("validate_prod_envelope")
        .expect("recursive certificate prover must enforce production params");
    let recursive_layer0_prove = recursive_certificate_fn
        .find("prove_ai_pow_tiled_full")
        .expect("recursive certificate prover must build the Layer-0 proof internally");
    let recursive_full_matmul_guard = recursive_certificate_fn
        .find("FullMatmulProofUnavailable")
        .expect("recursive certificate prover must fail closed before ZK for selected-tile multi-tile statements");
    let recursive_target_guard = recursive_certificate_fn
        .find("ensure_found_tile_hits_target")
        .expect("recursive certificate prover must check the plain matmul target hit before ZK");
    assert!(
        AI_POW_MINER_LIB_RS.contains("pub target: DifficultyTarget")
            && AI_POW_MINER_LIB_RS.contains("Count of fully rebuilt nonce-bound matmul attempts")
            && AI_POW_MINER_LIB_RS.contains("candidate_nck_commitment")
            && AI_POW_MINER_BIN_RS.contains("certificate_builder: Some")
            && !AI_POW_MINER_BIN_RS.contains("certificate_builder: None")
            && !AI_POW_MINER_BIN_RS.contains("ai_pow::verify_at_target(")
            && AI_POW_MINER_BIN_RS.contains("&sol.candidate_nck_commitment")
            && AI_POW_MINER_BIN_RS.contains(
                "recursive_certificate_builder_rejects_nonce_anchor_substitution_before_zkp"
            )
            && AI_POW_MINER_BIN_RS.contains(
                "recursive_certificate_builder_rejects_multi_tile_full_matmul_gap_before_zkp"
            )
            && AI_POW_ZK_BRIDGE_RS
                .contains("recursive_certificate_builder_rejects_missed_target_before_zkp")
            && target_check < recursive_prove
            && AI_POW_ZK_BRIDGE_RS.contains("pub fn prove_ai_pow_recursive_certificate")
            && recursive_certificate_fn.contains("validate_prod_envelope")
            && recursive_prod_envelope < recursive_layer0_prove
            && recursive_full_matmul_guard < recursive_layer0_prove
            && recursive_target_guard < recursive_layer0_prove
            && AI_POW_ZK_BRIDGE_RS
                .contains("prove_canonical_ai_pow_certificate_from_composite_proof")
            && !AI_POW_ZK_RECURSION_RS.contains("verify_production_certificate")
            && !AI_POW_ZK_RECURSION_RS.contains("AiPowProductionCertificate")
            && AI_POW_ZK_RECURSION_RS.contains("pub fn verify_recursive_certificate")
            && AI_POW_ZK_RECURSION_RS.contains("pub type AiPowRecursiveCertificate")
            && AI_POW_ZK_RECURSION_RS
                .contains("pub fn prove_canonical_ai_pow_certificate_from_composite_proof"),
        "production miner must only start recursive ZKP generation after a \
         successful plain matmul target check and a full-matmul recursive \
         statement guard, and must submit the recursive certificate noun \
         rather than a Layer-0 or plain proof artifact"
    );
}
