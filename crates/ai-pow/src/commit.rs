//! Merkle commitment over fixed-size 32-byte leaves.
//!
//! Domain-separated BLAKE3 with one context for leaves and another for
//! internal nodes. The number of leaves must be a power of two (the puzzle
//! parameters guarantee this for `comm_M`).

use blake3::Hasher;
use thiserror::Error;

const CTX_LEAF: &str = "ai-pow v1 merkle-leaf";
const CTX_NODE: &str = "ai-pow v1 merkle-node";

fn leaf_hash(m: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_LEAF);
    hasher.update(m);
    *hasher.finalize().as_bytes()
}

fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_NODE);
    hasher.update(left);
    hasher.update(right);
    *hasher.finalize().as_bytes()
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MerkleError {
    #[error("number of leaves must be a power of two")]
    NotPowerOfTwo,
    #[error("leaves slice is empty")]
    Empty,
    #[error("leaf index out of range")]
    IndexOutOfRange,
}

/// Compute the Merkle root of `leaves`. Returns an error if the count is not
/// a power of two or the slice is empty.
pub fn merkle_root(leaves: &[[u8; 32]]) -> Result<[u8; 32], MerkleError> {
    let n = leaves.len();
    if n == 0 {
        return Err(MerkleError::Empty);
    }
    if !n.is_power_of_two() {
        return Err(MerkleError::NotPowerOfTwo);
    }
    let mut layer: Vec<[u8; 32]> = leaves.iter().map(leaf_hash).collect();
    while layer.len() > 1 {
        layer = layer
            .chunks_exact(2)
            .map(|pair| node_hash(&pair[0], &pair[1]))
            .collect();
    }
    Ok(layer[0])
}

/// Sibling list for the leaf at `idx`, ordered from leaf-level upward.  Each
/// sibling is the node not on the path from the leaf to the root.
pub fn merkle_path(leaves: &[[u8; 32]], idx: usize) -> Result<Vec<[u8; 32]>, MerkleError> {
    let n = leaves.len();
    if n == 0 {
        return Err(MerkleError::Empty);
    }
    if !n.is_power_of_two() {
        return Err(MerkleError::NotPowerOfTwo);
    }
    if idx >= n {
        return Err(MerkleError::IndexOutOfRange);
    }
    let mut layer: Vec<[u8; 32]> = leaves.iter().map(leaf_hash).collect();
    let mut path = Vec::new();
    let mut pos = idx;
    while layer.len() > 1 {
        let sibling = if pos % 2 == 0 {
            layer[pos + 1]
        } else {
            layer[pos - 1]
        };
        path.push(sibling);
        layer = layer
            .chunks_exact(2)
            .map(|pair| node_hash(&pair[0], &pair[1]))
            .collect();
        pos /= 2;
    }
    Ok(path)
}

/// Recompute the root from a leaf, its position, and the sibling path.
/// Returns the recomputed root.  Caller compares against the canonical root.
pub fn merkle_recover_root(
    leaf: &[u8; 32],
    idx: usize,
    path: &[[u8; 32]],
    num_leaves: usize,
) -> Result<[u8; 32], MerkleError> {
    if num_leaves == 0 {
        return Err(MerkleError::Empty);
    }
    if !num_leaves.is_power_of_two() {
        return Err(MerkleError::NotPowerOfTwo);
    }
    if idx >= num_leaves {
        return Err(MerkleError::IndexOutOfRange);
    }
    let depth = num_leaves.trailing_zeros() as usize;
    if path.len() != depth {
        return Err(MerkleError::IndexOutOfRange);
    }
    let mut node = leaf_hash(leaf);
    let mut pos = idx;
    for sibling in path {
        node = if pos % 2 == 0 {
            node_hash(&node, sibling)
        } else {
            node_hash(sibling, &node)
        };
        pos /= 2;
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_leaves(n: usize) -> Vec<[u8; 32]> {
        (0..n)
            .map(|i| {
                let mut v = [0u8; 32];
                v[..8].copy_from_slice(&(i as u64).to_le_bytes());
                v
            })
            .collect()
    }

    #[test]
    fn root_determinism() {
        let leaves = sample_leaves(8);
        assert_eq!(merkle_root(&leaves).unwrap(), merkle_root(&leaves).unwrap());
    }

    #[test]
    fn round_trip_each_leaf() {
        for n_log in 0..7 {
            let n = 1usize << n_log;
            let leaves = sample_leaves(n);
            let root = merkle_root(&leaves).unwrap();
            for idx in 0..n {
                let path = merkle_path(&leaves, idx).unwrap();
                let recovered = merkle_recover_root(&leaves[idx], idx, &path, n).unwrap();
                assert_eq!(root, recovered, "n={n} idx={idx}");
            }
        }
    }

    #[test]
    fn tampered_leaf_rejects() {
        let leaves = sample_leaves(8);
        let root = merkle_root(&leaves).unwrap();
        let path = merkle_path(&leaves, 3).unwrap();
        let mut tampered = leaves[3];
        tampered[0] ^= 1;
        let recovered = merkle_recover_root(&tampered, 3, &path, 8).unwrap();
        assert_ne!(recovered, root);
    }

    #[test]
    fn tampered_path_rejects() {
        let leaves = sample_leaves(8);
        let root = merkle_root(&leaves).unwrap();
        let mut path = merkle_path(&leaves, 3).unwrap();
        path[0][0] ^= 1;
        let recovered = merkle_recover_root(&leaves[3], 3, &path, 8).unwrap();
        assert_ne!(recovered, root);
    }

    #[test]
    fn rejects_non_pow2() {
        let leaves = sample_leaves(7);
        assert_eq!(merkle_root(&leaves).err(), Some(MerkleError::NotPowerOfTwo));
    }

    #[test]
    fn rejects_empty() {
        let leaves: Vec<[u8; 32]> = Vec::new();
        assert_eq!(merkle_root(&leaves).err(), Some(MerkleError::Empty));
    }
}
