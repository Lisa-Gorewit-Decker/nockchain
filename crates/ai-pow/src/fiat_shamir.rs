//! Fiat-Shamir transcript over BLAKE3 keyed-derive.
//!
//! Implements the Pearl §4.3 Algorithm 2-shaped commitment-hash
//! derivation chain:
//!
//!   κ   = derive_key("kappa",        block_state(block, nonce) ‖ params_tag)
//!   H_A = MerkleRoot({ BLAKE3(row_i_of_A, key=κ) }_{i∈[m]})
//!   H_B = MerkleRoot({ BLAKE3(col_j_of_B, key=κ) }_{j∈[n]})
//!   s_B = derive_key("s_b",          κ ‖ H_B)
//!   s_A = derive_key("s_a",          s_B ‖ H_A)
//!
//! Noise generation reads s_A (for `E = E_L · E_R`) and s_B (for `F = F_L ·
//! F_R`). Pearl's asymmetry only permits reuse while `σ` is fixed. For
//! Nockchain production, each nonce attempt changes `σ`, so the keyed
//! commitments, seeds, noise, and matmul-derived values must not be reused
//! across nonces.
//!
//! The final `pow_key = derive_key("pow-key", s_A ‖ nonce)` is the
//! keyed-BLAKE3 key for the tile-state hashes. It is domain separation on top
//! of an already nonce-bound `s_A`; it is not the sole attempt binding.

use std::collections::HashSet;

use blake3::Hasher;

const CTX_TRANSCRIPT: &str = "ai-pow v3 transcript";
const CTX_INDICES: &str = "ai-pow v3 challenge-indices";
const CTX_POW_KEY: &str = "ai-pow v3 pow-key";
const CTX_CHALLENGE: &str = "ai-pow v3 challenge-seed";

/// Build the per-block `state` byte string fed to the prover and verifier.
pub fn block_state(block_commitment: &[u8], nonce: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + block_commitment.len() + 8 + nonce.len());
    buf.extend_from_slice(&(block_commitment.len() as u64).to_le_bytes());
    buf.extend_from_slice(block_commitment);
    buf.extend_from_slice(&(nonce.len() as u64).to_le_bytes());
    buf.extend_from_slice(nonce);
    buf
}

/// Current `κ` helper (Pearl `compute_job_key`,
/// `Pearl zk-pow ffi/mine.rs:156-161`): unkeyed BLAKE3 over the
/// concatenation of the attempt state and `params_tag`. Pearl uses
/// `header.to_bytes() || config.to_bytes()`; we accept the two parts as
/// separate slices but feed them into BLAKE3 in flat order (no length
/// prefix) to match Pearl exactly.
///
/// The caller must pass the full per-attempt state. Omitting the
/// nonce/extranonce would make downstream noise reusable and is not
/// production-sound.
pub fn commitment_key(attempt_state: &[u8], params_tag: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(attempt_state);
    hasher.update(params_tag);
    *hasher.finalize().as_bytes()
}

/// `s_B` (Pearl `compute_commitment_hash` line 4,
/// `Pearl zk-pow ffi/mine.rs:167-170`): unkeyed BLAKE3 of the 64-byte
/// concatenation `κ ‖ H_B`.
pub fn noise_seed_b(kappa: &[u8; 32], h_b: &[u8; 32]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(kappa);
    input[32..].copy_from_slice(h_b);
    *Hasher::new().update(&input).finalize().as_bytes()
}

/// `s_A` (Pearl `compute_commitment_hash` line 5,
/// `Pearl zk-pow ffi/mine.rs:172-175`): unkeyed BLAKE3 of the 64-byte
/// concatenation `s_B ‖ H_A`.
pub fn noise_seed_a(s_b: &[u8; 32], h_a: &[u8; 32]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(s_b);
    input[32..].copy_from_slice(h_a);
    *Hasher::new().update(&input).finalize().as_bytes()
}

/// Per-attempt `pow_key` used as the BLAKE3 key for
/// `BLAKE3(M_{i,j}, key=pow_key)`.
///
/// This function is not the only production attempt binding; callers must
/// derive `s_a` from the nonce-bound attempt state before computing `M`.
pub fn pow_key_for_nonce(s_a: &[u8; 32], nonce: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_POW_KEY);
    hasher.update(s_a);
    hasher.update(&(nonce.len() as u64).to_le_bytes());
    hasher.update(nonce);
    *hasher.finalize().as_bytes()
}

/// Challenge seed bound to the full per-attempt commitment `comm_M`. Used
/// to derive spot-check tile indices for replication verification.
pub fn challenge_seed(state: &[u8], comm_m: &[u8; 32], params_tag: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_CHALLENGE);
    hasher.update(&(state.len() as u64).to_le_bytes());
    hasher.update(state);
    hasher.update(comm_m);
    hasher.update(params_tag);
    *hasher.finalize().as_bytes()
}

/// Generic transcript hash: returns 32 bytes for an arbitrary list of byte
/// strings, length-prefixed individually.
pub fn transcript(label: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_TRANSCRIPT);
    hasher.update(&(label.len() as u64).to_le_bytes());
    hasher.update(label.as_bytes());
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

/// Derive `count` distinct indices in `0..range` from `seed`. Sampling is
/// without-replacement over a streamed XOF with rejection-and-set.
/// Determinism: same `(seed, count, range)` always yields the same vector.
pub fn challenge_indices(seed: &[u8; 32], count: u32, range: u64) -> Vec<u64> {
    assert!(range > 0, "range must be > 0");
    assert!(u64::from(count) <= range, "count must be <= range");
    let mut hasher = Hasher::new_derive_key(CTX_INDICES);
    hasher.update(seed);
    hasher.update(&count.to_le_bytes());
    hasher.update(&range.to_le_bytes());
    let mut xof = hasher.finalize_xof();

    // H2 (DoS audit): the prior implementation allocated
    // `vec![false; range as usize]` — `range = num_tiles`, which (post
    // H1) is bounded only by `u32::MAX`. A crafted call with a huge
    // range would burn up to ~4 GiB. A `HashSet` tracks "already-taken"
    // indices in `O(count)` memory regardless of `range`.
    let mut chosen: Vec<u64> = Vec::with_capacity(count as usize);
    let mut taken: HashSet<u64> = HashSet::with_capacity(count as usize);
    let mut buf = [0u8; 8];
    while chosen.len() < count as usize {
        xof.fill(&mut buf);
        let r = u64::from_le_bytes(buf);
        let idx = r % range;
        if taken.insert(idx) {
            chosen.push(idx);
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_state_round_trip_is_unambiguous() {
        let s1 = block_state(b"abc", b"de");
        let s2 = block_state(b"ab", b"cde");
        assert_ne!(s1, s2, "length-prefixing must disambiguate concatenations");
    }

    #[test]
    fn commitment_key_binds_attempt_state() {
        // κ depends on the full attempt state + params_tag. Production callers
        // construct that attempt state with block_state(block_commitment, nonce).
        let tag = [9u8; 32];
        let k1 = commitment_key(b"hdr", &tag);
        let k2 = commitment_key(b"hdr", &tag);
        assert_eq!(k1, k2);
        assert_ne!(commitment_key(b"hdr2", &tag), k1);
        assert_ne!(commitment_key(b"hdr", &[10u8; 32]), k1);
    }

    #[test]
    fn pearl_derivation_chain_binds_all_inputs() {
        // s_A must differ when *any* of (attempt state, params, h_a, h_b) differs.
        let kappa = commitment_key(b"hdr", &[1u8; 32]);
        let h_a = [2u8; 32];
        let h_b = [3u8; 32];
        let s_b = noise_seed_b(&kappa, &h_b);
        let s_a = noise_seed_a(&s_b, &h_a);

        let kappa2 = commitment_key(b"hdr-other", &[1u8; 32]);
        let s_b2 = noise_seed_b(&kappa2, &h_b);
        let s_a2 = noise_seed_a(&s_b2, &h_a);
        assert_ne!(s_a, s_a2, "changing attempt state must change s_A");

        let s_a3 = noise_seed_a(&noise_seed_b(&kappa, &[7u8; 32]), &h_a);
        assert_ne!(s_a, s_a3, "changing h_b must change s_A");

        let s_a4 = noise_seed_a(&s_b, &[8u8; 32]);
        assert_ne!(s_a, s_a4, "changing h_a must change s_A");
    }

    #[test]
    fn pow_key_changes_with_nonce_but_not_with_unrelated_inputs() {
        let s_a = [4u8; 32];
        let k1 = pow_key_for_nonce(&s_a, b"nce-1");
        let k2 = pow_key_for_nonce(&s_a, b"nce-2");
        assert_ne!(k1, k2);
        assert_eq!(k1, pow_key_for_nonce(&s_a, b"nce-1"));
        assert_ne!(pow_key_for_nonce(&[5u8; 32], b"nce-1"), k1);
    }

    #[test]
    fn pow_key_separate_from_seeds() {
        // Domain contexts must keep pow_key distinct from the seed values
        // it's derived from.
        let s_a = [4u8; 32];
        assert_ne!(pow_key_for_nonce(&s_a, b""), s_a);
        assert_ne!(
            pow_key_for_nonce(&s_a, b"nce"),
            noise_seed_a(&[1u8; 32], &[2u8; 32])
        );
    }

    #[test]
    fn indices_unique_and_in_range() {
        let seed = [1u8; 32];
        let idx = challenge_indices(&seed, 16, 64);
        assert_eq!(idx.len(), 16);
        for &i in &idx {
            assert!(i < 64);
        }
        let mut sorted = idx.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 16);
    }

    #[test]
    fn indices_deterministic_and_seed_sensitive() {
        let s1 = [1u8; 32];
        let s2 = [2u8; 32];
        assert_eq!(
            challenge_indices(&s1, 16, 64),
            challenge_indices(&s1, 16, 64)
        );
        assert_ne!(
            challenge_indices(&s1, 16, 64),
            challenge_indices(&s2, 16, 64)
        );
    }

    #[test]
    fn transcript_determinism_and_label_separation() {
        let parts = [&b"hello"[..], &b"world"[..]];
        assert_eq!(transcript("a", &parts), transcript("a", &parts));
        assert_ne!(transcript("a", &parts), transcript("b", &parts));
    }
}
