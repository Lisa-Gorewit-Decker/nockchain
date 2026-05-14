//! Public inputs to the matmul SNARK.
//!
//! These are the values the verifier sees in plaintext; the witness
//! (`crate::witness::Witness`) carries everything else. Constructed by
//! the caller from a plain `MatmulProof` at the `ai-pow → ai-pow-zk`
//! boundary so this crate doesn't depend back on `ai-pow`.
//!
//! ## Field encoding
//!
//! Each public input is mapped to a sequence of Goldilocks field
//! elements. 32-byte hashes are split into 8 × `u32` little-endian
//! chunks (one `u32` per Goldilocks element); scalar `u32` values
//! occupy one Goldilocks each. The full vector has
//! [`NUM_PUBLIC_INPUTS`] entries laid out in the order:
//!
//! ```text
//!   [ params_tag (8) | h_a (8) | h_b (8) | comm_m (8)
//!   | found_i (1) | found_j (1)
//!   | found_leaf (8) ]
//!   = 42 elements
//! ```
//!
//! The AIR binds these via dedicated "public input rows" the prover
//! pins to these exact values; the verifier reads back the same vector
//! through the standard `p3_uni_stark::verify` signature.

use p3_field::PrimeField64;
use p3_field::integers::QuotientMap;
use p3_goldilocks::Goldilocks;

/// Number of Goldilocks public-input elements (see field encoding).
pub const NUM_PUBLIC_INPUTS: usize = 8 + 8 + 8 + 8 + 1 + 1 + 8;

/// The public values the SNARK attests to.
///
/// Mirrors Pearl's `PublicProofParams` (see
/// `pearl/zk-pow/src/api/proof.rs:58-71`) in spirit — every byte of state
/// the chain pins down ahead of the SNARK, plus the tile coordinate that
/// "wins" the hardness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInputs {
    /// `params_tag`: the 32-byte canonical hash of the matmul parameters
    /// (`ai_pow::prover::params_tag`).
    pub params_tag: [u8; 32],
    /// `h_a`: Pearl chunk-Merkle root over the row-major bytes of `A`.
    pub h_a: [u8; 32],
    /// `h_b`: Pearl chunk-Merkle root over the column-major bytes of `B`.
    pub h_b: [u8; 32],
    /// `comm_M`: Merkle root over the per-tile keyed-BLAKE3 leaves.
    pub comm_m: [u8; 32],
    /// `(i, j)` coordinates of the tile that satisfied the difficulty
    /// target.
    pub found_i: u32,
    pub found_j: u32,
    /// The keyed-hash leaf for the found tile (= `TileState::keyed_hash`).
    pub found_leaf: [u8; 32],
}

/// Errors decoding a Goldilocks public-input vector back into
/// [`PublicInputs`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    /// Wrong number of field elements.
    #[error("public-input vector has {got} elements, expected {expected}")]
    WrongLength { got: usize, expected: usize },
    /// A field element that should encode a `u32` was outside the
    /// canonical range `[0, 2^32)`.
    #[error("field element {value} at index {index} is out of u32 range")]
    OutOfRange { index: usize, value: u64 },
}

impl PublicInputs {
    /// Encode `self` into a flat `Vec<Goldilocks>` of length
    /// [`NUM_PUBLIC_INPUTS`].
    pub fn to_field_elements(&self) -> Vec<Goldilocks> {
        let mut out = Vec::with_capacity(NUM_PUBLIC_INPUTS);
        append_hash(&mut out, &self.params_tag);
        append_hash(&mut out, &self.h_a);
        append_hash(&mut out, &self.h_b);
        append_hash(&mut out, &self.comm_m);
        out.push(u32_to_goldilocks(self.found_i));
        out.push(u32_to_goldilocks(self.found_j));
        append_hash(&mut out, &self.found_leaf);
        debug_assert_eq!(out.len(), NUM_PUBLIC_INPUTS);
        out
    }

    /// Decode a flat public-input vector. Errors if the length is
    /// wrong or if a `u32`-shaped element exceeds `2^32 - 1`.
    pub fn from_field_elements(values: &[Goldilocks]) -> Result<Self, DecodeError> {
        if values.len() != NUM_PUBLIC_INPUTS {
            return Err(DecodeError::WrongLength {
                got: values.len(),
                expected: NUM_PUBLIC_INPUTS,
            });
        }
        let mut idx = 0usize;
        let params_tag = take_hash(values, &mut idx)?;
        let h_a = take_hash(values, &mut idx)?;
        let h_b = take_hash(values, &mut idx)?;
        let comm_m = take_hash(values, &mut idx)?;
        let found_i = take_u32(values, &mut idx)?;
        let found_j = take_u32(values, &mut idx)?;
        let found_leaf = take_hash(values, &mut idx)?;
        debug_assert_eq!(idx, NUM_PUBLIC_INPUTS);
        Ok(Self {
            params_tag,
            h_a,
            h_b,
            comm_m,
            found_i,
            found_j,
            found_leaf,
        })
    }
}

fn take_u32(values: &[Goldilocks], idx: &mut usize) -> Result<u32, DecodeError> {
    let v = values[*idx].as_canonical_u64();
    if v > u32::MAX as u64 {
        return Err(DecodeError::OutOfRange {
            index: *idx,
            value: v,
        });
    }
    *idx += 1;
    Ok(v as u32)
}

fn take_hash(values: &[Goldilocks], idx: &mut usize) -> Result<[u8; 32], DecodeError> {
    let mut out = [0u8; 32];
    for chunk in 0..8 {
        let v = values[*idx].as_canonical_u64();
        if v > u32::MAX as u64 {
            return Err(DecodeError::OutOfRange {
                index: *idx,
                value: v,
            });
        }
        out[chunk * 4..(chunk + 1) * 4].copy_from_slice(&(v as u32).to_le_bytes());
        *idx += 1;
    }
    Ok(out)
}

/// Split a 32-byte hash into 8 × `u32` little-endian chunks, encode
/// each as a Goldilocks element, and push onto `out`.
fn append_hash(out: &mut Vec<Goldilocks>, hash: &[u8; 32]) {
    for chunk_idx in 0..8 {
        let bytes: [u8; 4] = hash[chunk_idx * 4..(chunk_idx + 1) * 4]
            .try_into()
            .unwrap();
        let v = u32::from_le_bytes(bytes);
        out.push(u32_to_goldilocks(v));
    }
}

#[inline]
fn u32_to_goldilocks(v: u32) -> Goldilocks {
    <Goldilocks as QuotientMap<u64>>::from_int(v as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PublicInputs {
        PublicInputs {
            params_tag: std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(3)),
            h_a: std::array::from_fn(|i| (i as u8).wrapping_mul(31).wrapping_add(7)),
            h_b: std::array::from_fn(|i| (i as u8).wrapping_mul(53).wrapping_add(11)),
            comm_m: std::array::from_fn(|i| (i as u8).wrapping_mul(71).wrapping_add(13)),
            found_i: 0x1234_5678,
            found_j: 0x9abc_def0,
            found_leaf: std::array::from_fn(|i| (i as u8).wrapping_mul(97).wrapping_add(17)),
        }
    }

    #[test]
    fn num_public_inputs_is_42() {
        // 4 × 32-byte hashes (each = 8 u32 LE chunks) + 2 × u32 scalars + 1 × 32-byte hash
        // = 4*8 + 2 + 8 = 42.
        assert_eq!(NUM_PUBLIC_INPUTS, 42);
    }

    #[test]
    fn encoded_length_matches_constant() {
        let pi = sample();
        let enc = pi.to_field_elements();
        assert_eq!(enc.len(), NUM_PUBLIC_INPUTS);
    }

    #[test]
    fn round_trip_recovers_struct() {
        let pi = sample();
        let enc = pi.to_field_elements();
        let dec = PublicInputs::from_field_elements(&enc).unwrap();
        assert_eq!(pi, dec);
    }

    #[test]
    fn encoding_is_deterministic() {
        let pi = sample();
        assert_eq!(pi.to_field_elements(), pi.to_field_elements());
    }

    #[test]
    fn hash_bytes_recover_exactly() {
        // Test every hash field's bytes pass through unchanged.
        let pi = sample();
        let dec = PublicInputs::from_field_elements(&pi.to_field_elements()).unwrap();
        assert_eq!(pi.params_tag, dec.params_tag);
        assert_eq!(pi.h_a, dec.h_a);
        assert_eq!(pi.h_b, dec.h_b);
        assert_eq!(pi.comm_m, dec.comm_m);
        assert_eq!(pi.found_leaf, dec.found_leaf);
    }

    #[test]
    fn u32_scalars_recover_exactly() {
        let pi = sample();
        let dec = PublicInputs::from_field_elements(&pi.to_field_elements()).unwrap();
        assert_eq!(pi.found_i, dec.found_i);
        assert_eq!(pi.found_j, dec.found_j);
    }

    #[test]
    fn changing_any_field_changes_encoding() {
        let pi = sample();
        let base = pi.to_field_elements();

        let mut pi2 = pi.clone();
        pi2.params_tag[0] ^= 1;
        assert_ne!(base, pi2.to_field_elements());

        let mut pi3 = pi.clone();
        pi3.h_a[31] ^= 1;
        assert_ne!(base, pi3.to_field_elements());

        let mut pi4 = pi.clone();
        pi4.found_i = pi.found_i.wrapping_add(1);
        assert_ne!(base, pi4.to_field_elements());

        let mut pi5 = pi.clone();
        pi5.found_leaf[15] ^= 1;
        assert_ne!(base, pi5.to_field_elements());
    }

    #[test]
    fn decode_rejects_wrong_length() {
        let pi = sample();
        let mut enc = pi.to_field_elements();
        enc.pop();
        assert_eq!(
            PublicInputs::from_field_elements(&enc).unwrap_err(),
            DecodeError::WrongLength {
                got: 41,
                expected: 42
            }
        );
        let mut enc = pi.to_field_elements();
        enc.push(u32_to_goldilocks(0));
        assert_eq!(
            PublicInputs::from_field_elements(&enc).unwrap_err(),
            DecodeError::WrongLength {
                got: 43,
                expected: 42
            }
        );
    }

    #[test]
    fn decode_rejects_out_of_range_u32_chunk() {
        // Build a valid encoding, then overwrite one element with a
        // value that exceeds `u32::MAX as u64`.
        let pi = sample();
        let mut enc = pi.to_field_elements();
        // Index 5 lands inside the params_tag hash; replace with 2^32.
        enc[5] = <Goldilocks as QuotientMap<u64>>::from_int(1u64 << 32);
        let err = PublicInputs::from_field_elements(&enc).unwrap_err();
        assert!(
            matches!(err, DecodeError::OutOfRange { index: 5, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn boundary_u32_values_round_trip() {
        let mut pi = sample();
        // u32::MAX exactly should round-trip cleanly.
        pi.found_i = u32::MAX;
        pi.found_j = 0;
        let dec = PublicInputs::from_field_elements(&pi.to_field_elements()).unwrap();
        assert_eq!(dec.found_i, u32::MAX);
        assert_eq!(dec.found_j, 0);
    }

    #[test]
    fn zero_public_inputs_round_trip() {
        let zero = PublicInputs {
            params_tag: [0u8; 32],
            h_a: [0u8; 32],
            h_b: [0u8; 32],
            comm_m: [0u8; 32],
            found_i: 0,
            found_j: 0,
            found_leaf: [0u8; 32],
        };
        let enc = zero.to_field_elements();
        assert!(enc.iter().all(|v| v.as_canonical_u64() == 0));
        let dec = PublicInputs::from_field_elements(&enc).unwrap();
        assert_eq!(zero, dec);
    }
}
