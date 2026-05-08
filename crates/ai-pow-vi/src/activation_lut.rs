//! 256-entry INT8→INT8 activation lookup tables.
//!
//! All non-linearities (SiLU, GeLU, Swish, gated tanh, etc.) are replaced
//! by lookup tables in the verifiable-inference puzzle. The LUT is *the*
//! activation: a model release pins the curve by committing the LUT bytes
//! into `comm_W` alongside the weight tensors.
//!
//! Why LUTs:
//! - Bit-exact across any platform, no transcendental-function variance.
//! - Cheap (single load + sign-extend on every supported CPU).
//! - Re-targetable: a model that wants a different SiLU shape just publishes
//!   a different LUT.

use blake3::Hasher;
use thiserror::Error;

const CTX_LUT: &str = "ai-pow-vi v1 activation-lut";

/// Identifier for the canonical activation curve a LUT was built from.
/// Stored alongside the LUT in the model registry so reviewers can sanity-
/// check that a published LUT matches the documented family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationKind {
    /// Sigmoid Linear Unit: `x * sigmoid(x)`. Used by Gemma family.
    SiLU,
    /// Gaussian Error Linear Unit (the GELU "tanh approximation"). Used by
    /// some Qwen variants.
    GeLU,
    /// Swish-beta with `beta = 1` (= SiLU); kept distinct so reviewers can
    /// tell which name a model card used.
    Swish,
    /// Identity, only used by tests.
    Identity,
}

/// 256-entry INT8→INT8 lookup table.
///
/// Layout: `table[i + 128]` is the output for input byte `i` (i ∈ [-128, 127]),
/// so `lookup(x)` is a single bias-and-load with no branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationLut {
    pub kind: ActivationKind,
    pub table: [i8; 256],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LutError {
    #[error("LUT byte slice must be exactly 256 entries; got {0}")]
    WrongLength(usize),
}

impl ActivationLut {
    /// Build a LUT from a byte buffer interpreted as `i8` values. Byte order
    /// is fixed: index `i` of `bytes` corresponds to input value `i - 128`.
    pub fn from_bytes(kind: ActivationKind, bytes: &[u8]) -> Result<Self, LutError> {
        if bytes.len() != 256 {
            return Err(LutError::WrongLength(bytes.len()));
        }
        let mut table = [0i8; 256];
        for (dst, &src) in table.iter_mut().zip(bytes.iter()) {
            *dst = src as i8;
        }
        Ok(Self { kind, table })
    }

    /// Serialize the LUT to canonical 256 bytes (i8 reinterpreted as u8).
    pub fn to_bytes(&self) -> [u8; 256] {
        let mut out = [0u8; 256];
        for (dst, src) in out.iter_mut().zip(self.table.iter()) {
            *dst = *src as u8;
        }
        out
    }

    /// Look up the LUT output for an INT8 input. Branchless: a single
    /// addition by 128 plus an array index.
    #[inline]
    pub fn lookup(&self, x: i8) -> i8 {
        let idx = (x as i32 + 128) as usize;
        debug_assert!(idx < 256);
        self.table[idx]
    }

    /// Apply the LUT in place to a slice. Equivalent to a per-element call
    /// to [`Self::lookup`].
    pub fn apply(&self, xs: &mut [i8]) {
        for x in xs.iter_mut() {
            *x = self.lookup(*x);
        }
    }

    /// Domain-separated commitment to the LUT bytes. Used as a leaf within
    /// `comm_W` so model releases pin the activation curve.
    pub fn commit(&self) -> [u8; 32] {
        let mut hasher = Hasher::new_derive_key(CTX_LUT);
        hasher.update(&[kind_tag(self.kind)]);
        hasher.update(&self.to_bytes());
        *hasher.finalize().as_bytes()
    }

    /// Identity LUT: `f(x) = x`. Useful for testing and for layers that
    /// don't apply a non-linearity.
    pub fn identity() -> Self {
        let mut table = [0i8; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = (i as i32 - 128) as i8;
        }
        Self {
            kind: ActivationKind::Identity,
            table,
        }
    }
}

fn kind_tag(k: ActivationKind) -> u8 {
    match k {
        ActivationKind::SiLU => 0x01,
        ActivationKind::GeLU => 0x02,
        ActivationKind::Swish => 0x03,
        ActivationKind::Identity => 0xff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips() {
        let lut = ActivationLut::identity();
        for x in i8::MIN..=i8::MAX {
            assert_eq!(lut.lookup(x), x, "identity LUT mismatch at {x}");
        }
    }

    #[test]
    fn from_bytes_rejects_bad_length() {
        assert_eq!(
            ActivationLut::from_bytes(ActivationKind::SiLU, &[0u8; 100]).err(),
            Some(LutError::WrongLength(100)),
        );
        assert!(ActivationLut::from_bytes(ActivationKind::SiLU, &[0u8; 256]).is_ok());
    }

    #[test]
    fn round_trip_bytes() {
        let lut = ActivationLut::identity();
        let bytes = lut.to_bytes();
        let again = ActivationLut::from_bytes(ActivationKind::Identity, &bytes).unwrap();
        assert_eq!(lut, again);
    }

    #[test]
    fn apply_in_place_matches_per_element() {
        let lut = ActivationLut::identity();
        let mut xs: Vec<i8> = vec![-50, -1, 0, 1, 50, 127, -128];
        let expected = xs.clone();
        lut.apply(&mut xs);
        assert_eq!(xs, expected);
    }

    #[test]
    fn commitment_is_kind_sensitive() {
        let mut bytes = [0u8; 256];
        for (i, slot) in bytes.iter_mut().enumerate() {
            *slot = (i as u8).wrapping_sub(128);
        }
        let silu = ActivationLut::from_bytes(ActivationKind::SiLU, &bytes).unwrap();
        let gelu = ActivationLut::from_bytes(ActivationKind::GeLU, &bytes).unwrap();
        // Same LUT bytes but different kind tag must produce different
        // commitments — protects against a registry that mislabels a curve.
        assert_ne!(silu.commit(), gelu.commit());
    }

    #[test]
    fn commitment_changes_on_one_byte_flip() {
        let lut_a = ActivationLut::identity();
        let mut bytes = lut_a.to_bytes();
        bytes[42] ^= 1;
        let lut_b = ActivationLut::from_bytes(ActivationKind::Identity, &bytes).unwrap();
        assert_ne!(lut_a.commit(), lut_b.commit());
    }
}
