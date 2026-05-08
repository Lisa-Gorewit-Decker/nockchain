//! Per-tile hardness hash and 256-bit big-endian target comparison.

use blake3::Hasher;

const CTX_TILE_STATE: &str = "ai-pow v1 tile-state";
const CTX_TILE_HASH: &str = "ai-pow v1 tile-hash";

/// Hash the partial-sum block of an output tile.  The block is the row-major
/// `tile x tile` array of i32 values; each is serialized little-endian before
/// hashing.  Returns the 32-byte tile state `M_{i,j}`.
pub fn tile_state_hash(partial_sum: &[i32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_TILE_STATE);
    hasher.update(&(partial_sum.len() as u64).to_le_bytes());
    for &v in partial_sum {
        hasher.update(&v.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

/// 32-byte hardness hash of tile `(i, j)` against the per-block challenge seed.
/// Block-headers compare this hash to the `target_matmul` atom as a big-endian
/// 256-bit unsigned integer.
pub fn tile_hardness_hash(challenge_seed: &[u8; 32], i: u32, j: u32, m_ij: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_TILE_HASH);
    hasher.update(challenge_seed);
    hasher.update(&i.to_le_bytes());
    hasher.update(&j.to_le_bytes());
    hasher.update(m_ij);
    *hasher.finalize().as_bytes()
}

/// Big-endian unsigned 256-bit comparison: `hash <= target`.
pub fn hash_le_target(hash: &[u8; 32], target: &[u8; 32]) -> bool {
    // Both hash and target are interpreted as big-endian 256-bit integers.
    for k in 0..32 {
        match hash[k].cmp(&target[k]) {
            core::cmp::Ordering::Less => return true,
            core::cmp::Ordering::Greater => return false,
            core::cmp::Ordering::Equal => continue,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_state_determinism_and_input_sensitivity() {
        let a = [1i32, 2, 3, 4];
        let b = [1i32, 2, 3, 5];
        assert_eq!(tile_state_hash(&a), tile_state_hash(&a));
        assert_ne!(tile_state_hash(&a), tile_state_hash(&b));
    }

    #[test]
    fn hardness_hash_separation() {
        let seed = [1u8; 32];
        let m = [2u8; 32];
        let h0 = tile_hardness_hash(&seed, 0, 0, &m);
        let h1 = tile_hardness_hash(&seed, 1, 0, &m);
        let h2 = tile_hardness_hash(&seed, 0, 1, &m);
        let h3 = tile_hardness_hash(&[3u8; 32], 0, 0, &m);
        assert_ne!(h0, h1);
        assert_ne!(h0, h2);
        assert_ne!(h0, h3);
    }

    #[test]
    fn target_compare_edges() {
        let zero = [0u8; 32];
        let max = [0xffu8; 32];
        assert!(hash_le_target(&zero, &zero));
        assert!(hash_le_target(&zero, &max));
        assert!(hash_le_target(&max, &max));
        assert!(!hash_le_target(&max, &zero));

        let mut a = [0u8; 32];
        a[31] = 0x10;
        let mut b = [0u8; 32];
        b[31] = 0x11;
        assert!(hash_le_target(&a, &b));
        assert!(!hash_le_target(&b, &a));

        // Most-significant byte dominates.
        let mut c = [0u8; 32];
        c[0] = 0x01;
        let mut d = [0u8; 32];
        d[0] = 0x00;
        d[31] = 0xff;
        assert!(!hash_le_target(&c, &d));
        assert!(hash_le_target(&d, &c));
    }
}
