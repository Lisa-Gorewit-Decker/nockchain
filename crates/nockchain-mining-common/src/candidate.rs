//! Decoded form of a `%mine` effect emitted by the node's miner kernel.
//!
//! The kernel-side Hoon emits effects shaped `[%mine version commit
//! target pow-len]`; this struct mirrors the in-process `MiningData`
//! the old in-tree driver carried (`crates/nockchain/src/mining.rs`,
//! pre-extraction). Each component is captured as a fresh [`NounSlab`]
//! so the candidate is owned + sendable across threads.

use nockapp::noun::slab::NounSlab;
use nockchain_math::noun_ext::NounMathExt;
use nockvm::ext::NounExt;
use nockvm::noun::Noun;
use thiserror::Error;

/// A mining candidate: everything the miner needs to drive one
/// puzzle-nock attempt.
pub struct MiningCandidate {
    /// Block-version atom from the candidate (chain consensus version).
    pub version: NounSlab,
    /// Tip5 commitment to the block header being mined.
    pub block_header: NounSlab,
    /// PoW difficulty target.
    pub target: NounSlab,
    /// `pow-len` parameter (proof length in bytes).
    pub pow_len: u64,
}

#[derive(Debug, Error)]
pub enum CandidateDecodeError {
    #[error("effect noun is not a cell")]
    NotACell,
    #[error("effect head is not %mine")]
    NotMine,
    #[error("effect tail does not match the 4-tuple [version commit target pow-len]")]
    BadTuple,
    #[error("pow-len is not a u64 atom")]
    BadPowLen,
}

impl MiningCandidate {
    /// Decode a `[%mine version commit target pow-len]` effect noun
    /// (read from a `WatchEffects` stream) into a `MiningCandidate`.
    ///
    /// Returns `Ok(None)` for an effect whose head is not `%mine`
    /// (callers using a server-side head_filter should not see this,
    /// but the check is defensive).
    pub fn from_effect_slab(slab: NounSlab) -> Result<Option<Self>, CandidateDecodeError> {
        // SAFETY: `slab.root()` returns a valid Noun owned by the slab
        // for the lifetime of `slab`. We construct nested slabs via
        // `copy_into`, which is safe.
        let root = unsafe { *slab.root() };
        let effect_cell = root.as_cell().map_err(|_| CandidateDecodeError::NotACell)?;
        if !effect_cell.head().eq_bytes("mine") {
            return Ok(None);
        }
        let [version, commit, target, pow_len_noun] = effect_cell
            .tail()
            .uncell::<4>()
            .map_err(|_| CandidateDecodeError::BadTuple)?;

        let pow_len = pow_len_noun
            .as_atom()
            .map_err(|_| CandidateDecodeError::BadPowLen)?
            .as_u64()
            .map_err(|_| CandidateDecodeError::BadPowLen)?;

        Ok(Some(MiningCandidate {
            version: noun_into_owned_slab(version),
            block_header: noun_into_owned_slab(commit),
            target: noun_into_owned_slab(target),
            pow_len,
        }))
    }
}

/// Copy a noun into a fresh `NounSlab` with that noun as the root, so it
/// can be owned and moved independently of the source slab. Mirrors the
/// `header_slab / version_slab / target_slab` build pattern in the old
/// in-tree driver.
fn noun_into_owned_slab(noun: Noun) -> NounSlab {
    let mut slab = NounSlab::new();
    let copied = slab.copy_into(noun);
    slab.set_root(copied);
    slab
}

#[cfg(test)]
mod tests {
    use super::*;
    use nockvm::noun::{D, T};
    use nockvm_macros::tas;

    /// Synthesize `[%mine version commit target pow-len]` and round-trip
    /// it through `MiningCandidate::from_effect_slab`.
    #[test]
    fn from_effect_slab_decodes_mine_tuple() {
        let mut slab = NounSlab::new();
        let head = D(tas!(b"mine"));
        let version = D(0xAA);
        let commit = D(0xBBBB);
        let target = D(0xCCCC_CCCC);
        let pow_len = D(256);
        let root = T(&mut slab, &[head, version, commit, target, pow_len]);
        slab.set_root(root);

        let candidate = MiningCandidate::from_effect_slab(slab)
            .expect("decode")
            .expect("head is %mine");

        assert_eq!(candidate.pow_len, 256);
        // The owned slabs round-trip the values.
        let v = unsafe { *candidate.version.root() }
            .as_atom()
            .expect("version atom")
            .as_u64()
            .expect("u64");
        assert_eq!(v, 0xAA);
        let h = unsafe { *candidate.block_header.root() }
            .as_atom()
            .expect("commit atom")
            .as_u64()
            .expect("u64");
        assert_eq!(h, 0xBBBB);
        let t = unsafe { *candidate.target.root() }
            .as_atom()
            .expect("target atom")
            .as_u64()
            .expect("u64");
        assert_eq!(t, 0xCCCC_CCCC);
    }

    #[test]
    fn from_effect_slab_returns_none_for_other_heads() {
        let mut slab = NounSlab::new();
        let head = D(tas!(b"irrelvnt")); // 8-char tas! limit
        let root = T(&mut slab, &[head, D(0), D(0), D(0), D(0)]);
        slab.set_root(root);

        let res = MiningCandidate::from_effect_slab(slab).expect("decode");
        assert!(res.is_none(), "non-%mine head should decode to None");
    }

    #[test]
    fn from_effect_slab_errors_on_bad_tuple() {
        // %mine head but the tail is a single atom, not a 4-tuple.
        let mut slab = NounSlab::new();
        let head = D(tas!(b"mine"));
        let root = T(&mut slab, &[head, D(0)]);
        slab.set_root(root);

        let res = MiningCandidate::from_effect_slab(slab);
        assert!(matches!(res, Err(CandidateDecodeError::BadTuple)));
    }
}
