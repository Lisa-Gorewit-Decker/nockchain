//! `Tip5Perm` — a `p3_symmetric` permutation adapter over the
//! in-crate, KAT-anchored, bit-for-bit twin of
//! `nockchain_math::tip5::permute` (C2.3 / M-S4).
//!
//! This is the *single in-workspace* native-reference Tip5
//! permutation. The recursion workspace cannot depend on `ai-pow-zk`
//! (separate excluded workspace), so the native
//! `DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>` /
//! `PaddingFreeSponge<Tip5Perm,16,10,5>` /
//! `TruncatedPermutation<Tip5Perm,2,5,16>` /
//! `MerkleTreeMmcs<Goldilocks, _, …>` used in the bit-for-bit
//! validation gates instantiate over **this** type.
//!
//! It is a thin, faithful adapter: it canonical-`u64` round-trips the
//! Goldilocks state through [`crate::tip5_spec::permute`] (the
//! 7-round deployed Nockchain Tip5, frozen against the committed
//! golden KAT), so `Tip5Perm.permute(state)` is — by construction —
//! the exact permutation the in-circuit Tip5 NPO witnesses.

use p3_field::{PrimeCharacteristicRing, PrimeField64};
use p3_goldilocks::Goldilocks;
use p3_symmetric::{CryptographicPermutation, Permutation};

use crate::tip5_spec::{STATE_SIZE, permute};

/// `Permutation<[Goldilocks; 16]>` over the deployed 7-round Tip5
/// (`crate::tip5_spec::permute`, the in-crate bit-for-bit twin of
/// `nockchain_math::tip5::permute`).
///
/// Public so the recursion crate's tests can build the native
/// `DuplexChallenger` / `PaddingFreeSponge` / `TruncatedPermutation` /
/// `MerkleTreeMmcs` reference oracles over the exact same permutation
/// the in-circuit Tip5 challenger / MMCS reconstructs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tip5Perm;

impl Permutation<[Goldilocks; STATE_SIZE]> for Tip5Perm {
    fn permute(&self, input: [Goldilocks; STATE_SIZE]) -> [Goldilocks; STATE_SIZE] {
        let mut s: [u64; STATE_SIZE] =
            core::array::from_fn(|i| PrimeField64::as_canonical_u64(&input[i]));
        permute(&mut s);
        core::array::from_fn(|i| Goldilocks::from_u64(s[i]))
    }

    fn permute_mut(&self, input: &mut [Goldilocks; STATE_SIZE]) {
        *input = Permutation::permute(self, *input);
    }
}

impl CryptographicPermutation<[Goldilocks; STATE_SIZE]> for Tip5Perm {}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Tip5Perm` is exactly `tip5_spec::permute` lifted to
    /// `[Goldilocks;16]` (canonical-u64 round-trip is the identity on
    /// canonical inputs).
    #[test]
    fn perm_matches_spec_permute() {
        let mut raw: [u64; STATE_SIZE] = core::array::from_fn(|i| (i as u64) * 0x9e37_79b9 + 1);
        let lifted: [Goldilocks; STATE_SIZE] =
            core::array::from_fn(|i| Goldilocks::from_u64(raw[i]));

        let via_perm = Tip5Perm.permute(lifted);
        permute(&mut raw);
        let via_spec: [Goldilocks; STATE_SIZE] =
            core::array::from_fn(|i| Goldilocks::from_u64(raw[i]));

        assert_eq!(via_perm, via_spec);
    }

    #[test]
    fn permute_mut_agrees_with_permute() {
        let lifted: [Goldilocks; STATE_SIZE] =
            core::array::from_fn(|i| Goldilocks::from_u64(0xdead_beef ^ (i as u64)));
        let mut m = lifted;
        Tip5Perm.permute_mut(&mut m);
        assert_eq!(m, Tip5Perm.permute(lifted));
    }
}
