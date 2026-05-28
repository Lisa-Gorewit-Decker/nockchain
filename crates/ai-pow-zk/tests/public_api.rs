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
