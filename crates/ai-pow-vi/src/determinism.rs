//! Cross-architecture determinism harness.
//!
//! Every numerical op the puzzle relies on must produce identical bytes on
//! every supported CPU (x86_64, aarch64). This module provides:
//!
//! - The [`BitExactOp`] trait, which every consensus-relevant op implements.
//!   `expected_hash()` returns the BLAKE3 of a canonical test-vector output;
//!   CI on each architecture asserts equality.
//! - [`ARCH_TAG`], a build-time string identifying the running platform, so
//!   test failures point at which arch produced the divergent output.

use blake3::Hasher;

/// Build-time tag identifying the architecture. Included in determinism
/// failure messages so reviewers can immediately tell which platform is
/// off-spec.
pub const ARCH_TAG: &str = if cfg!(target_arch = "x86_64") {
    "x86_64"
} else if cfg!(target_arch = "aarch64") {
    "aarch64"
} else if cfg!(target_arch = "arm") {
    "arm"
} else if cfg!(target_arch = "riscv64") {
    "riscv64"
} else {
    "unknown"
};

/// Trait every consensus-relevant op implements to declare its bit-exact
/// canonical output. `name` is a stable identifier used in error messages.
/// `expected_hash` is a `[u8; 32]` literal pinned in code; updating it
/// requires acknowledging that the consensus output of the op is changing.
pub trait BitExactOp {
    fn name(&self) -> &'static str;
    fn expected_hash(&self) -> [u8; 32];
}

/// Hash an arbitrary byte run with the dedicated determinism context. Used
/// by ops to fold canonical test outputs into a 32-byte digest comparable
/// to [`BitExactOp::expected_hash`].
pub fn hash_canonical(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key("ai-pow-vi v1 bit-exact-output");
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

/// Helper for tests: run an op's canonical test vector, hash the output,
/// and assert against `expected_hash`. Caller supplies a closure that
/// produces the canonical-byte output.
#[cfg(test)]
pub fn assert_bit_exact<F: FnOnce() -> Vec<u8>>(op: &dyn BitExactOp, run: F) {
    let bytes = run();
    let actual = hash_canonical(&bytes);
    let expected = op.expected_hash();
    assert_eq!(
        actual,
        expected,
        "{} produced divergent output on {ARCH_TAG}: \
         expected {:02x?}, got {:02x?}",
        op.name(),
        expected,
        actual,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_tag_is_recognized() {
        // Just confirm we're on a supported arch in CI; "unknown" should
        // never appear on a real reviewer's machine.
        assert_ne!(ARCH_TAG, "unknown", "running on an unsupported arch");
    }

    #[test]
    fn hash_canonical_is_deterministic_and_distinct() {
        let a = hash_canonical(b"hello");
        let b = hash_canonical(b"hello");
        let c = hash_canonical(b"world");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hash_canonical_uses_dedicated_context() {
        // The same bytes hashed by a vanilla blake3 call should not match
        // the canonical digest — domain separation is what protects against
        // off-spec ops accidentally matching unrelated hashes.
        let canonical = hash_canonical(b"hello");
        let plain = *blake3::hash(b"hello").as_bytes();
        assert_ne!(canonical, plain);
    }
}
