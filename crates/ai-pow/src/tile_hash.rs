//! Shape-aware difficulty threshold and 256-bit big-endian unsigned compare.
//!
//! Pearl §4.5 hardness rule:
//!
//!   BLAKE3(M, key = s_a)  <=  2^(256 - b) · r · t_m · t_n
//!
//! For square tiles (`t_m = t_n = tile`) this crate computes the right-hand
//! side as a 256-bit big-endian integer and compares the keyed hash byte-wise.

use crate::params::MatmulParams;

/// Big-endian unsigned 256-bit comparison: `hash <= target`.
pub fn hash_le_target(hash: &[u8; 32], target: &[u8; 32]) -> bool {
    for k in 0..32 {
        match hash[k].cmp(&target[k]) {
            core::cmp::Ordering::Less => return true,
            core::cmp::Ordering::Greater => return false,
            core::cmp::Ordering::Equal => continue,
        }
    }
    true
}

/// Compute the shape-aware target `2^(256 - b) · r · t^2`, saturating at the
/// max 256-bit value when the product would exceed it. When `b == 0` the
/// product is effectively `2^256 · r · t^2`, which always saturates to
/// `2^256 - 1`. When `b >= 256` and `r · t^2 < 2^b`, the product is `< 1`
/// and the function returns zero.
pub fn difficulty_target(params: &MatmulParams) -> [u8; 32] {
    let r = params.noise_rank as u128;
    let t = params.tile as u128;
    let weight = r.saturating_mul(t).saturating_mul(t);
    target_from_weight(params.difficulty_bits, weight)
}

/// Build a 256-bit big-endian target representing `floor(weight · 2^(256-b))`,
/// saturated to `[0, 2^256 - 1]`. The 256-bit number is held as a high-128
/// half (bits 128..256) and a low-128 half (bits 0..128).
fn target_from_weight(b: u32, weight: u128) -> [u8; 32] {
    if weight == 0 {
        return [0u8; 32];
    }
    if b == 0 {
        // weight * 2^256 always overflows since weight >= 1.
        return [0xFFu8; 32];
    }
    if b >= 256 + 128 {
        // Even the top bit of `weight` is below position 0.
        return [0u8; 32];
    }
    let shift = 256i32 - b as i32;
    let (hi, lo): (u128, u128) = if shift >= 128 {
        let s = (shift - 128) as u32;
        // Place `weight` in the high half, shifted by `s` bits. Any bits
        // pushed past position 128 (of the high half) overflow bit 256 of
        // the full target ⇒ saturate.
        if s > 0 && (weight >> (128 - s)) != 0 {
            return [0xFFu8; 32];
        }
        let hi = if s == 128 { 0 } else { weight << s };
        (hi, 0u128)
    } else if shift > 0 {
        let s = shift as u32;
        // weight << s lives in [s..s+128] bits. The low 128 bits of the
        // result are the low 128 bits of (weight << s), the high 128 bits
        // are weight >> (128 - s).
        let lo = if s == 0 { weight } else { weight << s };
        let hi = if s == 0 { 0 } else { weight >> (128 - s) };
        (hi, lo)
    } else {
        // shift <= 0: weight >> |shift|.
        let s = (-shift) as u32;
        if s >= 128 {
            return [0u8; 32];
        }
        (0u128, weight >> s)
    };
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&hi.to_be_bytes());
    out[16..].copy_from_slice(&lo.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn difficulty_b_zero_is_max_target() {
        let params = MatmulParams::TEST_SMALL;
        // b = 0 ⇒ every tile passes hardness.
        let t = difficulty_target(&params);
        assert_eq!(t, [0xFF; 32]);
    }

    #[test]
    fn difficulty_b_huge_is_zero_target() {
        let mut params = MatmulParams::TEST_SMALL;
        params.difficulty_bits = 400;
        let t = difficulty_target(&params);
        assert_eq!(t, [0u8; 32]);
    }

    #[test]
    fn difficulty_target_monotonic_in_b() {
        // Higher `b` → smaller target.
        let mut p = MatmulParams::TEST_SMALL;
        let mut prev: Option<[u8; 32]> = None;
        for b in [0u32, 1, 8, 16, 64, 128, 200, 256] {
            p.difficulty_bits = b;
            let t = difficulty_target(&p);
            if let Some(prev) = prev {
                // prev should be >= t.
                assert!(
                    !hash_le_target(&prev, &t) || prev == t,
                    "target at b={b} ({t:?}) should be <= prev ({prev:?})"
                );
            }
            prev = Some(t);
        }
    }

    #[test]
    fn difficulty_target_shape_weighting() {
        // Higher `r` or `t` at fixed `b` => bigger target (easier).
        let mut small = MatmulParams::TEST_SMALL;
        small.difficulty_bits = 128;
        let small_t = difficulty_target(&small);
        let mut big = small;
        big.tile = small.tile * 2;
        // Won't satisfy validate (tile must divide m,n) — that's fine, we're
        // testing the threshold function in isolation.
        let big_t = difficulty_target(&big);
        assert!(!hash_le_target(&big_t, &small_t) || big_t == small_t);
    }

    #[test]
    fn difficulty_b_256_equals_weight() {
        let mut p = MatmulParams::TEST_SMALL;
        p.difficulty_bits = 256;
        // weight = r * t^2 = 4 * 64 = 256
        let weight = 256u128;
        let mut expected = [0u8; 32];
        let bytes = weight.to_be_bytes();
        expected[32 - 16..].copy_from_slice(&bytes);
        assert_eq!(difficulty_target(&p), expected);
    }
}
