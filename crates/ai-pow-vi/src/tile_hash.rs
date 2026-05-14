//! Tile leaf + hardness hashing for the verifiable-inference puzzle.
//!
//! Built on top of [`ai_pow::matmul::TileState`] and `TileState::keyed_hash`
//! so this crate stays as close to `ai-pow`'s Pearl-aligned primitives as
//! possible. The only thing local to `ai-pow-vi` is the per-tile **hardness
//! hash**, which binds the FS-derived `challenge_seed` together with the
//! tile coordinate `(i, j)` and the tile leaf. `ai-pow` doesn't expose
//! such a step because Pearl's `pow_key` is derived independently of the
//! tile leaves; `ai-pow-vi` derives its challenge from
//! `(state, comm_m, model_id)` and so must apply the binding after `comm_m`
//! is known.
//!
//! Use directly:
//!
//!   * [`tile_leaf`]   — hash an i32 sub-block of FFN output into the
//!                       32-byte Merkle leaf, byte-equivalent to running
//!                       Pearl's rotate-XOR fold (`ai_pow::matmul::TileState::fold`)
//!                       and then `TileState::keyed_hash(&[0u8; 32])`.
//!   * [`tile_hardness_hash`] — bind `(challenge_seed, i, j, leaf)` into a
//!                       32-byte hardness value, compared against the
//!                       difficulty target by `ai_pow::tile_hash::hash_le_target`.

use ai_pow::matmul::TileState;
use blake3::Hasher;

const CTX_TILE_HARDNESS: &str = "ai-pow-vi tile-hardness";

/// Fold a flat `&[i32]` sub-block (e.g., a `t × t` FFN gate tile in
/// row-major order) into Pearl's 16 × i32 [`TileState`]:
///
/// ```text
///   for step, chunk in block.chunks(16):
///       x = u32-XOR of chunk entries
///       M[step mod 16] = M[step mod 16].rotate_left(13) ^ x
/// ```
///
/// Exactly the same fold rule `ai-pow` uses for per-stripe accumulator
/// updates (`ai_pow::matmul::TileState::fold` with `LROT_PER_TILE = 13`).
/// Treating each 16-entry chunk of the FFN tile as one "stripe" gives a
/// well-defined `TileState` for blocks of any length.
pub fn tile_state_from_block(block: &[i32]) -> TileState {
    let mut state = TileState::zero();
    for (step, chunk) in block.chunks(16).enumerate() {
        let x = chunk.iter().fold(0u32, |acc, &v| acc ^ (v as u32));
        state.fold(step as u32, x as i32);
    }
    state
}

/// Tile-Merkle leaf for an i32 sub-block. Equivalent to
/// `tile_state_from_block(block).keyed_hash(&[0u8; 32])` — reuses
/// `ai-pow`'s Pearl-aligned keyed-BLAKE3 of the 16 × `u32` LE state with a
/// sentinel zero key (the per-tile `(i, j)` binding happens later in the
/// hardness hash, so the leaf needs no per-tile key).
pub fn tile_leaf(block: &[i32]) -> [u8; 32] {
    tile_state_from_block(block).keyed_hash(&[0u8; 32])
}

/// FS-bound hardness value for tile `(i, j)` with leaf `m_ij` under the
/// challenge seed `seed`. The verifier compares this 32-byte value against
/// the target via `ai_pow::tile_hash::hash_le_target`.
pub fn tile_hardness_hash(seed: &[u8; 32], i: u32, j: u32, m_ij: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_TILE_HARDNESS);
    hasher.update(seed);
    hasher.update(&i.to_le_bytes());
    hasher.update(&j.to_le_bytes());
    hasher.update(m_ij);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_state_from_empty_is_zero() {
        let state = tile_state_from_block(&[]);
        assert_eq!(state, TileState::zero());
    }

    #[test]
    fn tile_leaf_is_deterministic() {
        let block = vec![1i32, -2, 3, -4, 5, -6, 7, -8, 9, -10, 11, -12, 13, -14, 15, -16, 17];
        let h1 = tile_leaf(&block);
        let h2 = tile_leaf(&block);
        assert_eq!(h1, h2);
    }

    #[test]
    fn tile_leaf_changes_with_content() {
        let mut block = vec![0i32; 16];
        let h0 = tile_leaf(&block);
        block[0] = 1;
        let h1 = tile_leaf(&block);
        assert_ne!(h0, h1);
    }

    #[test]
    fn tile_state_uses_pearl_rotate_xor() {
        // Manual cross-check: two i32s a, b XOR-folded into slot 0 give
        // (((0 << 13) ^ a) << 13) ^ b. Build a 32-entry block such that
        // chunk 0 = a, chunk 1 = b (with each chunk XOR-ing to those values).
        let a = 0x1234_5678u32 as i32;
        let b = 0x0bad_f00du32 as i32;
        let mut block = vec![0i32; 32];
        block[0] = a; // chunk 0 XOR -> a (rest are zero)
        block[16] = b; // chunk 1 XOR -> b
        let state = tile_state_from_block(&block);
        // step 0: M[0] = (0 << 13) ^ a = a
        // step 1: M[1] = (0 << 13) ^ b = b
        // Other slots remain 0.
        assert_eq!(state.0[0] as u32, a as u32);
        assert_eq!(state.0[1] as u32, b as u32);
        for v in &state.0[2..] {
            assert_eq!(*v, 0);
        }
    }

    #[test]
    fn tile_state_step_two_into_same_slot_rotates() {
        // step 0 -> slot 0 with value `a`.
        // step 16 -> slot 0 again: M[0] = M[0].rotate_left(13) ^ b.
        let a = 0x1234_5678u32;
        let b = 0x0bad_f00du32;
        let mut block = vec![0i32; 17 * 16];
        block[0] = a as i32;
        block[16 * 16] = b as i32;
        let state = tile_state_from_block(&block);
        let expected = a.rotate_left(13) ^ b;
        assert_eq!(state.0[0] as u32, expected);
    }

    #[test]
    fn hardness_hash_binds_all_inputs() {
        let seed = [9u8; 32];
        let m = [7u8; 32];
        let base = tile_hardness_hash(&seed, 1, 2, &m);
        // Different (i, j) → different hash.
        assert_ne!(tile_hardness_hash(&seed, 2, 2, &m), base);
        assert_ne!(tile_hardness_hash(&seed, 1, 3, &m), base);
        // Different seed → different hash.
        assert_ne!(tile_hardness_hash(&[10u8; 32], 1, 2, &m), base);
        // Different leaf → different hash.
        let mut m2 = m;
        m2[0] ^= 1;
        assert_ne!(tile_hardness_hash(&seed, 1, 2, &m2), base);
    }
}
