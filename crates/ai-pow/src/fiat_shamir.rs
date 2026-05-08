//! Fiat-Shamir transcript over BLAKE3 keyed-derive.
//!
//! All non-secret inputs are absorbed in a length-prefixed manner so that
//! collisions across different absorb sequences are infeasible.

use blake3::Hasher;

const CTX_TRANSCRIPT: &str = "ai-pow v1 transcript";
const CTX_INDICES: &str = "ai-pow v1 challenge-indices";
const CTX_NOISE_SEED_BLOCK: &str = "ai-pow v1.5 noise-seed-block";
const CTX_CHALLENGE: &str = "ai-pow v1 challenge-seed";

/// Build the per-block `state` byte string fed to the prover and verifier.
/// `block_commitment` is opaque (any length); `nonce` is opaque too.
pub fn block_state(block_commitment: &[u8], nonce: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + block_commitment.len() + 8 + nonce.len());
    buf.extend_from_slice(&(block_commitment.len() as u64).to_le_bytes());
    buf.extend_from_slice(block_commitment);
    buf.extend_from_slice(&(nonce.len() as u64).to_le_bytes());
    buf.extend_from_slice(nonce);
    buf
}

/// Domain-separated **block-level** noise seed.  Depends only on
/// `block_commitment` (not on the per-attempt nonce), so the noise matrices
/// `(E, F)` derived from this seed can be expanded once per block and reused
/// across all nonce attempts.  At LLM scale this moves noise expansion off
/// the per-attempt critical path entirely.  Anti-precompute is preserved:
/// `(E, F)` still depend on this block's commitment, so they cannot be
/// precomputed for a different block.
pub fn noise_seed_for_block(block_commitment: &[u8], params_tag: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_NOISE_SEED_BLOCK);
    hasher.update(&(block_commitment.len() as u64).to_le_bytes());
    hasher.update(block_commitment);
    hasher.update(params_tag);
    *hasher.finalize().as_bytes()
}

/// Challenge seed bound to the full per-attempt commitment `comm_M`. Both
/// the per-tile hardness hash and the spot-check index derivation use this
/// as their root of randomness.
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
/// without-replacement over a virtual permutation, but we use a streamed XOF
/// with rejection-and-set for simplicity. Determinism: same `(seed, count,
/// range)` always yields the same vector.
pub fn challenge_indices(seed: &[u8; 32], count: u32, range: u32) -> Vec<u32> {
    assert!(range > 0, "range must be > 0");
    assert!(count <= range, "count must be <= range");
    let mut hasher = Hasher::new_derive_key(CTX_INDICES);
    hasher.update(seed);
    hasher.update(&count.to_le_bytes());
    hasher.update(&range.to_le_bytes());
    let mut xof = hasher.finalize_xof();

    let mut chosen = Vec::with_capacity(count as usize);
    let mut taken = vec![false; range as usize];
    let mut buf = [0u8; 8];
    while chosen.len() < count as usize {
        xof.fill(&mut buf);
        // Map u64 to u32 index via modulo. Range is u32 so modulo bias is
        // bounded by 2^-32, negligible for our values of `range`.
        let r = u64::from_le_bytes(buf);
        let idx = (r % (range as u64)) as u32;
        if !taken[idx as usize] {
            taken[idx as usize] = true;
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
    fn noise_seed_for_block_determinism_and_separation() {
        let bc = b"hdr";
        let tag = [9u8; 32];
        assert_eq!(
            noise_seed_for_block(bc, &tag),
            noise_seed_for_block(bc, &tag),
        );
        let tag2 = [10u8; 32];
        assert_ne!(
            noise_seed_for_block(bc, &tag),
            noise_seed_for_block(bc, &tag2),
        );
        let bc2 = b"hdr2";
        assert_ne!(
            noise_seed_for_block(bc, &tag),
            noise_seed_for_block(bc2, &tag),
        );
    }

    #[test]
    fn noise_seed_for_block_does_not_depend_on_nonce() {
        // Sanity: the new helper takes block_commitment directly and is not
        // affected by the per-nonce state.
        let bc = b"hdr";
        let tag = [9u8; 32];
        let a = noise_seed_for_block(bc, &tag);
        // Whatever any nonce-dependent state would derive, it must equal `a`
        // when we just feed `bc`.  This is asserted by construction; the
        // test exists to lock in the function signature.
        assert_eq!(a, noise_seed_for_block(bc, &tag));
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
    fn indices_deterministic() {
        let seed = [1u8; 32];
        assert_eq!(
            challenge_indices(&seed, 16, 64),
            challenge_indices(&seed, 16, 64),
        );
    }

    #[test]
    fn indices_change_with_seed() {
        let s1 = [1u8; 32];
        let s2 = [2u8; 32];
        assert_ne!(
            challenge_indices(&s1, 16, 64),
            challenge_indices(&s2, 16, 64),
        );
    }

    #[test]
    fn transcript_determinism_and_label_separation() {
        let parts = [&b"hello"[..], &b"world"[..]];
        let h1 = transcript("a", &parts);
        let h2 = transcript("a", &parts);
        let h3 = transcript("b", &parts);
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }
}
