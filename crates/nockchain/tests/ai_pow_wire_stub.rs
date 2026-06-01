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
const AI_POW_ZK_RECURSION_RS: &str = include_str!("../../ai-pow-zk/src/recursion.rs");
const AI_POW_ZK_BRIDGE_RS: &str = include_str!("../../ai-pow/src/zk_bridge.rs");

#[test]
fn ai_pow_consensus_wire_is_structured_but_fail_closed_without_verifier() {
    assert!(
        TX_ENGINE_1_HOON.contains("+$  ai-blake  @uxblake")
            && TX_ENGINE_1_HOON.contains("+$  ai-ext2   @uxfelt")
            && TX_ENGINE_1_HOON.contains("+$  ai-proof-node")
            && TX_ENGINE_1_HOON.contains("[%ext2 value=ai-ext2]")
            && TX_ENGINE_1_HOON.contains("[%ext2s len=@ud data=ai-ext2s]")
            && TX_ENGINE_1_HOON.contains("[%u64s len=@ud data=@]")
            && TX_ENGINE_1_HOON.contains("+$  pow-artifact")
            && TX_ENGINE_1_HOON.contains("?=([%ai-pow *] u.pow.pag)")
            && CONSENSUS_HOON.contains("++  pow-artifact-to-proof-version")
            && CONSENSUS_HOON.contains("?=([%ai-pow *] pow)")
            && TYPES_HOON.contains("[%ai-pow cert=ai-pow-certificate]")
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
            && !INNER_HOON.contains("(set-pow:min [%ai-pow cert.pv.command])")
            && !INNER_HOON.contains("(heard-block /poke/ai-pow-miner now candidate-block.m.k eny)"),
        "%ai-pow consensus must fail closed until the recursive certificate \
         verifier is wired; it must not persist or broadcast unverified AI proofs"
    );
    assert!(
        AI_POW_MINER_RUN_RS.contains("build_ai_pow_certificate_poke")
            && AI_POW_MINER_RUN_RS.contains("[%command %pow %ai-pow cert]")
            && AI_POW_MINER_RUN_RS.contains("refusing to submit legacy nonce/tile artifact")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn decode_ai_pow_certificate_slab")
            && AI_POW_MINER_CERT_NOUN_RS.contains("pub fn precheck_ai_pow_certificate_statement")
            && AI_POW_ZK_BRIDGE_RS.contains("pub fn verify_ai_pow_production_statement")
            && AI_POW_MINER_CERT_NOUN_RS.contains("CertificateNounLimits")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("certificate_noun_decoder_rejects_oversized_packed_atom")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("certificate_noun_roundtrips_through_jam_cue_and_bounded_decoder")
            && AI_POW_MINER_CERT_NOUN_RS
                .contains("certificate_statement_precheck_binds_noun_metadata_to_nonce_and_target")
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
    let target_check = AI_POW_MINER_BIN_RS
        .find("verify_at_target")
        .expect("production miner must check the plain matmul target hit");
    let recursive_prove = AI_POW_MINER_BIN_RS
        .find("let run = prove_ai_pow_recursive_certificate")
        .expect("production miner must build the recursive certificate");
    assert!(
        AI_POW_MINER_LIB_RS.contains("pub target: DifficultyTarget")
            && AI_POW_MINER_BIN_RS.contains("certificate_builder: Some")
            && !AI_POW_MINER_BIN_RS.contains("certificate_builder: None")
            && target_check < recursive_prove
            && AI_POW_ZK_BRIDGE_RS.contains("pub fn prove_ai_pow_recursive_certificate")
            && AI_POW_ZK_BRIDGE_RS
                .contains("prove_canonical_ai_pow_certificate_from_composite_proof")
            && AI_POW_ZK_RECURSION_RS
                .contains("pub fn prove_canonical_ai_pow_certificate_from_composite_proof"),
        "production miner must only start recursive ZKP generation after a \
         successful plain matmul target check, and must submit the recursive \
         certificate noun rather than a Layer-0 or plain proof artifact"
    );
}
