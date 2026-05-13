//! Fiat-Shamir transcript over BLAKE3 keyed-derive.
//!
//! Implements the Pearl §4.3 Algorithm 2 commitment-hash derivation chain:
//!
//!   κ   = derive_key("kappa",        block_commitment ‖ params_tag)
//!   H_A = MerkleRoot({ BLAKE3(row_i_of_A, key=κ) }_{i∈[m]})
//!   H_B = MerkleRoot({ BLAKE3(col_j_of_B, key=κ) }_{j∈[n]})
//!   s_B = derive_key("s_b",          κ ‖ H_B)
//!   s_A = derive_key("s_a",          s_B ‖ H_A)
//!
//! Noise generation reads s_A (for `E = E_L · E_R`) and s_B (for `F = F_L ·
//! F_R`). Per Pearl §4.3 the asymmetry lets future AI workloads pre-noise
//! `B` once per σ update without re-noising on every change of `A`.
//!
//! The per-nonce `pow_key = derive_key("pow-key", s_A ‖ nonce)` is the
//! keyed-BLAKE3 key for the tile-state hashes. This is an extension over
//! Pearl-pure (which uses `s_A` directly): it amortizes the matmul across
//! nonce attempts and keeps a Bitcoin-style search loop.

use blake3::Hasher;

const CTX_TRANSCRIPT: &str = "ai-pow v3 transcript";
const CTX_INDICES: &str = "ai-pow v3 challenge-indices";
const CTX_KAPPA: &str = "ai-pow v3 commitment-key kappa";
const CTX_S_B: &str = "ai-pow v3 noise-seed s_B";
const CTX_S_A: &str = "ai-pow v3 noise-seed s_A";
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

/// `κ`: Pearl §4.3, the commitment-hash key. Depends only on
/// `block_commitment` and `params_tag` (NOT on the nonce, NOT on the
/// miner-supplied `A, B`). This is the key used for the matrix-row and
/// matrix-column leaf hashes feeding `H_A` and `H_B`.
pub fn commitment_key(block_commitment: &[u8], params_tag: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_KAPPA);
    hasher.update(&(block_commitment.len() as u64).to_le_bytes());
    hasher.update(block_commitment);
    hasher.update(params_tag);
    *hasher.finalize().as_bytes()
}

/// `s_B`: noise seed for `F = F_L · F_R`. Pearl §4.3 line 4.
/// Binds `F` to `(block, params, B)` via `κ` and `H_B`.
pub fn noise_seed_b(kappa: &[u8; 32], h_b: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_S_B);
    hasher.update(kappa);
    hasher.update(h_b);
    *hasher.finalize().as_bytes()
}

/// `s_A`: noise seed for `E = E_L · E_R` AND base for `pow_key`. Pearl §4.3
/// line 5. Binds `E` to `(block, params, A, B)` via `s_B` and `H_A`.
pub fn noise_seed_a(s_b: &[u8; 32], h_a: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_S_A);
    hasher.update(s_b);
    hasher.update(h_a);
    *hasher.finalize().as_bytes()
}

/// Per-nonce `pow_key` used as the BLAKE3 key for `BLAKE3(M_{i,j},
/// key=pow_key)` (Pearl §4.5 line 16). Pearl-pure would use `s_A` here; we
/// extend by hashing `s_A ‖ nonce` so a single `(block, A, B)` admits many
/// cheap nonce retries without re-running the matmul.
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
    fn commitment_key_independent_of_nonce() {
        // κ depends on block_commitment + params_tag only. Any value the
        // caller wants to vary across nonces must NOT feed into κ.
        let tag = [9u8; 32];
        let k1 = commitment_key(b"hdr", &tag);
        let k2 = commitment_key(b"hdr", &tag);
        assert_eq!(k1, k2);
        // But it does depend on block_commitment and params_tag.
        assert_ne!(commitment_key(b"hdr2", &tag), k1);
        assert_ne!(commitment_key(b"hdr", &[10u8; 32]), k1);
    }

    #[test]
    fn pearl_derivation_chain_binds_all_inputs() {
        // s_A must differ when *any* of (block, params, h_a, h_b) differs.
        let kappa = commitment_key(b"hdr", &[1u8; 32]);
        let h_a = [2u8; 32];
        let h_b = [3u8; 32];
        let s_b = noise_seed_b(&kappa, &h_b);
        let s_a = noise_seed_a(&s_b, &h_a);

        let kappa2 = commitment_key(b"hdr-other", &[1u8; 32]);
        let s_b2 = noise_seed_b(&kappa2, &h_b);
        let s_a2 = noise_seed_a(&s_b2, &h_a);
        assert_ne!(s_a, s_a2, "changing block must change s_A");

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
        assert_ne!(pow_key_for_nonce(&s_a, b"nce"), noise_seed_a(&[1u8; 32], &[2u8; 32]));
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
        assert_eq!(challenge_indices(&s1, 16, 64), challenge_indices(&s1, 16, 64));
        assert_ne!(challenge_indices(&s1, 16, 64), challenge_indices(&s2, 16, 64));
    }

    #[test]
    fn transcript_determinism_and_label_separation() {
        let parts = [&b"hello"[..], &b"world"[..]];
        assert_eq!(transcript("a", &parts), transcript("a", &parts));
        assert_ne!(transcript("a", &parts), transcript("b", &parts));
    }
}
