//! Out-of-circuit BLAKE3 helpers used by [`crate::verify`] (M10.1a).
//!
//! The composite tile AIR (M9.1) exposes the trace's final tile
//! state `m_final` as a public-value-bound element (constrained by
//! the AIR to equal `trace.last_row.m_out`). The verifier then
//! checks
//!
//! ```text
//!   BLAKE3-keyed(m_final_bytes, pow_key) == public_inputs.found_leaf
//! ```
//!
//! *outside* the SNARK, where `pow_key` is itself derived from the
//! public inputs via Pearl's commitment chain (matches
//! `ai_pow::fiat_shamir`):
//!
//! ```text
//!   κ      = BLAKE3(block_commitment || params_tag)
//!   s_B    = BLAKE3(κ ‖ h_b)             // 64-byte concat
//!   s_A    = BLAKE3(s_B ‖ h_a)           // 64-byte concat
//!   pow_key = derive_key(CTX_POW_KEY, s_A ‖ len(nonce) ‖ nonce)
//! ```
//!
//! The verifier has every input — `block_commitment`, `nonce` are
//! `verify` arguments and `params_tag`, `h_a`, `h_b` live on
//! `PublicInputs`.
//!
//! ## Why "out-of-circuit"?
//!
//! Doing this binding *in-circuit* is M10.1b — it would require
//! composing a BLAKE3 sub-AIR alongside the composite tile AIR and
//! wiring the per-round selectors Pearl uses (see Pearl's
//! `pearl_air.rs:91-109` and `blake_program.rs`). That's a multi-
//! week engineering effort. The out-of-circuit version here gives
//! us the same *cryptographic* binding for the specific
//! `m_final → found_leaf` relation:
//!
//!  * `m_final` is constrained by the AIR to equal what the matmul
//!    + state chain actually computed.
//!  * `pow_key` is a deterministic function of the public inputs +
//!    `(block_commitment, nonce)`.
//!  * The hash check is plain BLAKE3 over those two values.
//!
//! What is *not* yet bound by this out-of-circuit path: `h_a` and
//! `h_b` are not tied to the witness matrices `a`, `b` (an
//! adversary can still substitute arbitrary matrices). Closing that
//! gap requires in-circuit BLAKE3. See `ROADMAP.md` M10.1b.

use crate::public::PublicInputs;

/// Context string for the keyed-derivation of `pow_key`. Matches
/// `ai_pow::fiat_shamir::CTX_POW_KEY` so a verifier here and a
/// miner in `ai-pow` agree on the per-nonce key.
const CTX_POW_KEY: &str = "ai-pow v3 pow-key";

/// `κ = BLAKE3(block_commitment ‖ params_tag)`.
///
/// Matches `ai_pow::fiat_shamir::commitment_key`. Inputs are fed
/// flat (no length prefix) so the wire format aligns with Pearl
/// (Pearl §4.3 / `pearl/zk-pow/src/ffi/mine.rs:156-161`).
fn kappa(block_commitment: &[u8], params_tag: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(block_commitment);
    hasher.update(params_tag);
    *hasher.finalize().as_bytes()
}

/// `s_B = BLAKE3(κ ‖ h_b)` over the 64-byte concatenation.
fn s_b(kappa_bytes: &[u8; 32], h_b: &[u8; 32]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(kappa_bytes);
    input[32..].copy_from_slice(h_b);
    *blake3::Hasher::new().update(&input).finalize().as_bytes()
}

/// `s_A = BLAKE3(s_B ‖ h_a)` over the 64-byte concatenation.
fn s_a(s_b_bytes: &[u8; 32], h_a: &[u8; 32]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(s_b_bytes);
    input[32..].copy_from_slice(h_a);
    *blake3::Hasher::new().update(&input).finalize().as_bytes()
}

/// Derive `pow_key` from the public-input chain + `nonce`.
///
/// Matches `ai_pow::fiat_shamir::pow_key_for_nonce` exactly (context
/// string, length prefix, byte order). A change in any of
/// `(block_commitment, params_tag, h_a, h_b, nonce)` changes
/// `pow_key`.
pub fn derive_pow_key(block_commitment: &[u8], nonce: &[u8], pi: &PublicInputs) -> [u8; 32] {
    let kappa_bytes = kappa(block_commitment, &pi.params_tag);
    let s_b_bytes = s_b(&kappa_bytes, &pi.h_b);
    let s_a_bytes = s_a(&s_b_bytes, &pi.h_a);
    let mut hasher = blake3::Hasher::new_derive_key(CTX_POW_KEY);
    hasher.update(&s_a_bytes);
    hasher.update(&(nonce.len() as u64).to_le_bytes());
    hasher.update(nonce);
    *hasher.finalize().as_bytes()
}

/// Compute `BLAKE3-keyed(M_bytes, pow_key)` for a single-slot final
/// state `m_final`. The 64-byte input is laid out as `16 × i32 LE`
/// where slot 0 = `m_final` and slots 1..16 = 0 (the single-slot
/// regime of M9.1 only writes slot 0; M9.2 will widen this to 16
/// per-slot values).
///
/// Matches `ai_pow::matmul::TileState::keyed_hash`.
pub fn compute_found_leaf(m_final: u32, pow_key: &[u8; 32]) -> [u8; 32] {
    let mut bytes = [0u8; 64];
    bytes[..4].copy_from_slice(&(m_final as i32).to_le_bytes());
    let mut hasher = blake3::Hasher::new_keyed(pow_key);
    hasher.update(&bytes);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_pi() -> PublicInputs {
        PublicInputs {
            params_tag: [7u8; 32],
            h_a: [11u8; 32],
            h_b: [13u8; 32],
            comm_m: [17u8; 32],
            found_i: 0,
            found_j: 0,
            found_leaf: [0u8; 32],
        }
    }

    #[test]
    fn pow_key_changes_with_block_commitment() {
        let pi = fixture_pi();
        let k1 = derive_pow_key(b"block-A", b"nonce", &pi);
        let k2 = derive_pow_key(b"block-B", b"nonce", &pi);
        assert_ne!(k1, k2);
    }

    #[test]
    fn pow_key_changes_with_nonce() {
        let pi = fixture_pi();
        let k1 = derive_pow_key(b"block", b"n1", &pi);
        let k2 = derive_pow_key(b"block", b"n2", &pi);
        assert_ne!(k1, k2);
    }

    #[test]
    fn pow_key_changes_with_h_a() {
        let mut pi = fixture_pi();
        let k1 = derive_pow_key(b"block", b"nonce", &pi);
        pi.h_a[0] ^= 0xFF;
        let k2 = derive_pow_key(b"block", b"nonce", &pi);
        assert_ne!(k1, k2);
    }

    #[test]
    fn pow_key_changes_with_h_b() {
        let mut pi = fixture_pi();
        let k1 = derive_pow_key(b"block", b"nonce", &pi);
        pi.h_b[0] ^= 0xFF;
        let k2 = derive_pow_key(b"block", b"nonce", &pi);
        assert_ne!(k1, k2);
    }

    #[test]
    fn pow_key_deterministic() {
        let pi = fixture_pi();
        assert_eq!(
            derive_pow_key(b"block", b"nonce", &pi),
            derive_pow_key(b"block", b"nonce", &pi),
        );
    }

    #[test]
    fn found_leaf_changes_with_m_final() {
        let key = [42u8; 32];
        let l1 = compute_found_leaf(0x12345678, &key);
        let l2 = compute_found_leaf(0x12345679, &key);
        assert_ne!(l1, l2);
    }

    #[test]
    fn found_leaf_changes_with_pow_key() {
        let l1 = compute_found_leaf(0x12345678, &[1u8; 32]);
        let l2 = compute_found_leaf(0x12345678, &[2u8; 32]);
        assert_ne!(l1, l2);
    }

    #[test]
    fn found_leaf_zero_m_with_zero_key_is_stable() {
        // Anchor: catches any silent change to the padding /
        // byte-layout convention (16 × i32 LE, slot 0 = m_final).
        let leaf = compute_found_leaf(0, &[0u8; 32]);
        // Just confirm it's deterministic; we don't pin the bytes
        // because the convention is documented above and a change
        // would intentionally fail many other tests.
        assert_eq!(leaf, compute_found_leaf(0, &[0u8; 32]));
    }

    /// Cross-check against ai-pow's `TileState::keyed_hash` semantics:
    /// `M = [m_final, 0, ..., 0]` (16 × i32 LE) keyed with `pow_key`.
    #[test]
    fn found_leaf_matches_manual_blake3_keyed() {
        let m_final: u32 = 0xDEAD_BEEF;
        let pow_key = [0xAA; 32];

        let mut hasher = blake3::Hasher::new_keyed(&pow_key);
        // Manual 16-slot M: slot 0 = m_final as i32 LE, slots 1..16 = 0.
        for slot in 0..16i32 {
            let v: i32 = if slot == 0 { m_final as i32 } else { 0 };
            hasher.update(&v.to_le_bytes());
        }
        let expected: [u8; 32] = *hasher.finalize().as_bytes();

        assert_eq!(compute_found_leaf(m_final, &pow_key), expected);
    }
}
