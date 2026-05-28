#[test]
fn root_api_does_not_reexport_dev_verifiers() {
    let lib_rs = include_str!("../src/lib.rs");
    let unsafe_exports = [
        "composite_verify,",
        "composite_verify_pow,",
        "composite_verify_pinned,",
        "composite_verify_pow_pinned,",
        "composite_verify_pinned_logup_sx,",
        "composite_verify_pow_pinned_logup_sx,",
    ];

    for needle in unsafe_exports {
        assert!(
            !lib_rs.contains(needle),
            "root ai_pow_zk API must not re-export dev or caller-controlled verifier `{needle}`"
        );
    }
}

#[test]
fn sx_bound_helpers_are_not_public_module_api() {
    let composite_proof_rs = include_str!("../src/composite_proof.rs");
    for needle in [
        "pub fn composite_prove_pinned_logup_sx",
        "pub fn composite_verify_pinned_logup_sx",
        "pub fn composite_verify_pow_pinned_logup_sx",
    ] {
        assert!(
            !composite_proof_rs.contains(needle),
            "`{needle}` exposes caller-controlled sx_bound"
        );
    }
}
