//! Regression guard for the in-flight AI-PoW consensus wire.
//!
//! Until the final `%ai-pow` noun envelope and verifier jet land, the Hoon
//! consensus branch must remain reject-all. This test intentionally checks the
//! source-level contract because accepting even a placeholder `%ai-pow` block
//! would be a consensus-soundness bug.

const TYPES_HOON: &str = include_str!("../../../hoon/apps/dumbnet/lib/types.hoon");
const INNER_HOON: &str = include_str!("../../../hoon/apps/dumbnet/inner.hoon");

#[test]
fn ai_pow_consensus_wire_is_still_reject_all_stub() {
    assert!(
        TYPES_HOON.contains("[%ai-pow placeholder=@]"),
        "%ai-pow wire must stay a single placeholder atom until the final \
         consensus proof envelope is implemented"
    );
    assert!(
        INNER_HOON.contains("do-pow: %ai-pow verifier stub")
            && INNER_HOON.contains("reject-all until real verifier lands"),
        "%ai-pow do-pow branch must stay reject-all until a real verifier lands"
    );
}
