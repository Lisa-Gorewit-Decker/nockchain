//! Private witness for the matmul SNARK.
//!
//! Carries everything the prover needs to fill in the AIR trace but the
//! verifier never sees in plaintext. Mirrors Pearl's `PrivateProofParams`
//! (`pearl/zk-pow/src/api/proof.rs:83-89`). Constructed by the caller in
//! `ai-pow` from a plain `MatmulProof` plus the matching
//! `(BlockNoise, Matrices)` reconstructed by the prover.
//!
//! ## Field-element encoding
//!
//! The AIR trace is a row-major matrix of Goldilocks elements. Each
//! witness field is encoded element-by-element via simple bijective
//! casts that the in-circuit range tables (Pearl §pearl_air:62-66
//! analog, our Input chip) gate back to their declared types:
//!
//! | Field type | Encoding | Range table |
//! |------------|----------|-------------|
//! | `i8` (a_rows, b_cols, e_l, f_r) | `v as u8 as u64` (two's-complement bytes) | u8 |
//! | `u32` (e_r_pos, f_l_pos) | `v as u64` | u13 (positions < 2^13 ≤ rank) |
//! | `i32` (tile_states slots) | `v as u32 as u64` (sign-preserving via two's complement) | i32 |
//!
//! Each encoding is reversible by reading back the canonical u64
//! representation and reinterpreting via the corresponding cast.
//!
//! The flat order of fields in [`Witness::to_field_elements`] is:
//!
//! ```text
//!   [ a_rows         (tile * k        i8 cells)
//!   | b_cols         (tile * k        i8 cells)
//!   | e_l            (m * r           i8 cells)
//!   | e_r_pos        (k * 2           u32 cells)
//!   | f_r            (n * r           i8 cells)
//!   | f_l_pos        (k * 2           u32 cells)
//!   | tile_states    ((k/r + 1) * 16  i32 cells) ]
//! ```
//!
//! `field_element_count(params)` is the total length; the AIR's trace
//! width will be a function of this once the chip layout in
//! `crate::air` is finalized.

use p3_field::integers::QuotientMap;
use p3_field::PrimeField64;
use p3_goldilocks::Goldilocks;

use crate::params::ZkParams;

/// The private witness for the SNARK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Witness {
    /// `tile` row strips of `A` consumed by the found tile, each of
    /// length `k` (i.e. `tile * k` i8 entries in row-major order).
    pub a_rows: Vec<i8>,
    /// `tile` column strips of `B` consumed by the found tile, each of
    /// length `k`.
    pub b_cols: Vec<i8>,
    /// Per-block `E_L` factor flattened row-major (`m * r` i6 entries).
    pub e_l: Vec<i8>,
    /// Per-block `E_R` choice-matrix positions (one `(p_plus, p_minus)`
    /// pair per column of `E_R`, indexed by column `l ∈ 0..k`).
    pub e_r_pos: Vec<(u32, u32)>,
    /// Per-block `F_R` factor flattened col-major (`n * r` i6 entries).
    pub f_r: Vec<i8>,
    /// Per-block `F_L` choice-matrix positions (one `(p_plus, p_minus)`
    /// pair per row of `F_L`, indexed by row `l ∈ 0..k`).
    pub f_l_pos: Vec<(u32, u32)>,
    /// Per-stripe `M`-state evolution for the found tile, as 16 × i32
    /// per stripe step. `tile_states[step]` is the value of `M` after
    /// folding stripe `step`. Used by the AIR to constrain the
    /// rotate-13-XOR fold step-by-step.
    pub tile_states: Vec<[i32; 16]>,
}

/// Errors decoding a Goldilocks vector back into [`Witness`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    /// Wrong total number of field elements for the given `ZkParams`.
    #[error("witness vector has {got} elements, expected {expected}")]
    WrongLength { got: usize, expected: usize },
    /// An i8-shaped element exceeded `0xff`.
    #[error("i8 cell at index {index} has value {value} > 0xff")]
    I8OutOfRange { index: usize, value: u64 },
    /// A u32-shaped element exceeded `2^32 - 1`.
    #[error("u32 cell at index {index} has value {value} > u32::MAX")]
    U32OutOfRange { index: usize, value: u64 },
    /// An i32-shaped element exceeded `2^32 - 1`.
    #[error("i32 cell at index {index} has value {value} > u32::MAX")]
    I32OutOfRange { index: usize, value: u64 },
}

/// Number of `(p_plus, p_minus)` pairs per permutation, i.e. `k`.
#[inline]
fn perm_pair_count(params: &ZkParams) -> usize {
    params.k as usize
}

/// Number of stripe-state snapshots in `tile_states`: one initial state
/// + one per stripe step = `k / r + 1`.
#[inline]
fn tile_state_count(params: &ZkParams) -> usize {
    (params.k / params.noise_rank) as usize + 1
}

/// Total number of field elements `to_field_elements` produces for
/// `params`.
pub fn field_element_count(params: &ZkParams) -> usize {
    let tile = params.tile as usize;
    let k = params.k as usize;
    let m = params.m as usize;
    let n = params.n as usize;
    let r = params.noise_rank as usize;
    tile * k                  // a_rows
        + tile * k             // b_cols
        + m * r                // e_l
        + perm_pair_count(params) * 2  // e_r_pos
        + n * r                // f_r
        + perm_pair_count(params) * 2  // f_l_pos
        + tile_state_count(params) * 16 // tile_states
}

impl Witness {
    /// Expected lengths of each witness field given `params`. Useful
    /// for caller-side validation before calling `to_field_elements`.
    pub fn expected_lengths(params: &ZkParams) -> WitnessLengths {
        let tile = params.tile as usize;
        let k = params.k as usize;
        let m = params.m as usize;
        let n = params.n as usize;
        let r = params.noise_rank as usize;
        WitnessLengths {
            a_rows: tile * k,
            b_cols: tile * k,
            e_l: m * r,
            e_r_pos: perm_pair_count(params),
            f_r: n * r,
            f_l_pos: perm_pair_count(params),
            tile_states: tile_state_count(params),
        }
    }

    /// Flatten this witness into a `Vec<Goldilocks>` matching the
    /// canonical column order documented at the top of this module.
    /// Panics if any internal field length disagrees with
    /// `expected_lengths(params)`.
    pub fn to_field_elements(&self, params: &ZkParams) -> Vec<Goldilocks> {
        let want = Self::expected_lengths(params);
        assert_eq!(self.a_rows.len(), want.a_rows, "a_rows length");
        assert_eq!(self.b_cols.len(), want.b_cols, "b_cols length");
        assert_eq!(self.e_l.len(), want.e_l, "e_l length");
        assert_eq!(self.e_r_pos.len(), want.e_r_pos, "e_r_pos length");
        assert_eq!(self.f_r.len(), want.f_r, "f_r length");
        assert_eq!(self.f_l_pos.len(), want.f_l_pos, "f_l_pos length");
        assert_eq!(
            self.tile_states.len(),
            want.tile_states,
            "tile_states length"
        );

        let mut out = Vec::with_capacity(field_element_count(params));
        for v in &self.a_rows {
            out.push(i8_to_goldilocks(*v));
        }
        for v in &self.b_cols {
            out.push(i8_to_goldilocks(*v));
        }
        for v in &self.e_l {
            out.push(i8_to_goldilocks(*v));
        }
        for &(pp, pm) in &self.e_r_pos {
            out.push(u32_to_goldilocks(pp));
            out.push(u32_to_goldilocks(pm));
        }
        for v in &self.f_r {
            out.push(i8_to_goldilocks(*v));
        }
        for &(pp, pm) in &self.f_l_pos {
            out.push(u32_to_goldilocks(pp));
            out.push(u32_to_goldilocks(pm));
        }
        for state in &self.tile_states {
            for v in state {
                out.push(i32_to_goldilocks(*v));
            }
        }
        debug_assert_eq!(out.len(), field_element_count(params));
        out
    }

    /// Inverse of [`Witness::to_field_elements`].
    pub fn from_field_elements(
        values: &[Goldilocks],
        params: &ZkParams,
    ) -> Result<Self, DecodeError> {
        let expected = field_element_count(params);
        if values.len() != expected {
            return Err(DecodeError::WrongLength {
                got: values.len(),
                expected,
            });
        }
        let mut idx = 0usize;
        let want = Self::expected_lengths(params);

        let mut a_rows = Vec::with_capacity(want.a_rows);
        for _ in 0..want.a_rows {
            a_rows.push(take_i8(values, &mut idx)?);
        }
        let mut b_cols = Vec::with_capacity(want.b_cols);
        for _ in 0..want.b_cols {
            b_cols.push(take_i8(values, &mut idx)?);
        }
        let mut e_l = Vec::with_capacity(want.e_l);
        for _ in 0..want.e_l {
            e_l.push(take_i8(values, &mut idx)?);
        }
        let mut e_r_pos = Vec::with_capacity(want.e_r_pos);
        for _ in 0..want.e_r_pos {
            let pp = take_u32(values, &mut idx)?;
            let pm = take_u32(values, &mut idx)?;
            e_r_pos.push((pp, pm));
        }
        let mut f_r = Vec::with_capacity(want.f_r);
        for _ in 0..want.f_r {
            f_r.push(take_i8(values, &mut idx)?);
        }
        let mut f_l_pos = Vec::with_capacity(want.f_l_pos);
        for _ in 0..want.f_l_pos {
            let pp = take_u32(values, &mut idx)?;
            let pm = take_u32(values, &mut idx)?;
            f_l_pos.push((pp, pm));
        }
        let mut tile_states = Vec::with_capacity(want.tile_states);
        for _ in 0..want.tile_states {
            let mut state = [0i32; 16];
            for s in state.iter_mut() {
                *s = take_i32(values, &mut idx)?;
            }
            tile_states.push(state);
        }
        debug_assert_eq!(idx, expected);
        Ok(Self {
            a_rows,
            b_cols,
            e_l,
            e_r_pos,
            f_r,
            f_l_pos,
            tile_states,
        })
    }
}

/// Lengths of each individual witness field, derived from `ZkParams`.
/// Returned by [`Witness::expected_lengths`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WitnessLengths {
    pub a_rows: usize,
    pub b_cols: usize,
    pub e_l: usize,
    /// Number of `(p_plus, p_minus)` pairs (each pair contributes 2 Goldilocks elements to the flat vector).
    pub e_r_pos: usize,
    pub f_r: usize,
    pub f_l_pos: usize,
    /// Number of 16-element `tile_state` snapshots (each contributes 16 i32 elements to the flat vector).
    pub tile_states: usize,
}

// =====================================================================
//  Cell-level encode / decode helpers.
// =====================================================================

#[inline]
fn i8_to_goldilocks(v: i8) -> Goldilocks {
    <Goldilocks as QuotientMap<u64>>::from_int(v as u8 as u64)
}

#[inline]
fn u32_to_goldilocks(v: u32) -> Goldilocks {
    <Goldilocks as QuotientMap<u64>>::from_int(v as u64)
}

#[inline]
fn i32_to_goldilocks(v: i32) -> Goldilocks {
    <Goldilocks as QuotientMap<u64>>::from_int(v as u32 as u64)
}

fn take_i8(values: &[Goldilocks], idx: &mut usize) -> Result<i8, DecodeError> {
    let v = values[*idx].as_canonical_u64();
    if v > 0xff {
        return Err(DecodeError::I8OutOfRange {
            index: *idx,
            value: v,
        });
    }
    *idx += 1;
    Ok(v as u8 as i8)
}

fn take_u32(values: &[Goldilocks], idx: &mut usize) -> Result<u32, DecodeError> {
    let v = values[*idx].as_canonical_u64();
    if v > u32::MAX as u64 {
        return Err(DecodeError::U32OutOfRange {
            index: *idx,
            value: v,
        });
    }
    *idx += 1;
    Ok(v as u32)
}

fn take_i32(values: &[Goldilocks], idx: &mut usize) -> Result<i32, DecodeError> {
    let v = values[*idx].as_canonical_u64();
    if v > u32::MAX as u64 {
        return Err(DecodeError::I32OutOfRange {
            index: *idx,
            value: v,
        });
    }
    *idx += 1;
    Ok(v as u32 as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_params() -> ZkParams {
        // Smallest valid Pearl-spec params: r ≥ 2 power of two,
        // k % r == 0, tile divides m and n.
        ZkParams {
            m: 8,
            k: 16,
            n: 8,
            noise_rank: 2,
            tile: 2,
            difficulty_bits: 0,
        }
    }

    fn sample(params: &ZkParams) -> Witness {
        let lens = Witness::expected_lengths(params);
        let i8s = |n: usize, salt: u32| -> Vec<i8> {
            (0..n)
                .map(|i| ((i as u32).wrapping_mul(salt) as i32 % 256 - 128) as i8)
                .collect()
        };
        let pairs = |n: usize, salt: u32| -> Vec<(u32, u32)> {
            (0..n)
                .map(|i| {
                    let r = (i as u32).wrapping_mul(salt);
                    (
                        r % (params.noise_rank as u32),
                        (r + 1) % (params.noise_rank as u32),
                    )
                })
                .collect()
        };
        let states: Vec<[i32; 16]> = (0..lens.tile_states)
            .map(|step| {
                let mut s = [0i32; 16];
                for (k, slot) in s.iter_mut().enumerate() {
                    *slot = (step as i32).wrapping_mul(13) ^ ((k as i32) * 0x010203);
                }
                s
            })
            .collect();
        Witness {
            a_rows: i8s(lens.a_rows, 0xa1),
            b_cols: i8s(lens.b_cols, 0xb1),
            e_l: i8s(lens.e_l, 0xc1),
            e_r_pos: pairs(lens.e_r_pos, 0xd1),
            f_r: i8s(lens.f_r, 0xe1),
            f_l_pos: pairs(lens.f_l_pos, 0xf1),
            tile_states: states,
        }
    }

    #[test]
    fn field_element_count_matches_formula() {
        let p = small_params();
        // 2*16 + 2*16 + 8*2 + 16*2 + 8*2 + 16*2 + 9*16 = 32+32+16+32+16+32+144 = 304
        let expected = 304;
        assert_eq!(field_element_count(&p), expected);
        assert_eq!(sample(&p).to_field_elements(&p).len(), expected);
    }

    #[test]
    fn expected_lengths_decompose_to_total() {
        let p = small_params();
        let lens = Witness::expected_lengths(&p);
        let total = lens.a_rows
            + lens.b_cols
            + lens.e_l
            + lens.e_r_pos * 2
            + lens.f_r
            + lens.f_l_pos * 2
            + lens.tile_states * 16;
        assert_eq!(total, field_element_count(&p));
    }

    #[test]
    fn round_trip_recovers_witness() {
        let p = small_params();
        let w = sample(&p);
        let enc = w.to_field_elements(&p);
        let dec = Witness::from_field_elements(&enc, &p).unwrap();
        assert_eq!(w, dec);
    }

    #[test]
    fn encoding_is_deterministic() {
        let p = small_params();
        let w = sample(&p);
        assert_eq!(w.to_field_elements(&p), w.to_field_elements(&p));
    }

    #[test]
    fn i8_boundary_values_round_trip() {
        let p = small_params();
        let lens = Witness::expected_lengths(&p);
        // Build a witness with extreme i8 values at every cell.
        let extreme = |n: usize| -> Vec<i8> {
            (0..n)
                .map(|i| if i % 2 == 0 { i8::MIN } else { i8::MAX })
                .collect()
        };
        let w = Witness {
            a_rows: extreme(lens.a_rows),
            b_cols: extreme(lens.b_cols),
            e_l: extreme(lens.e_l),
            e_r_pos: vec![(0, 1); lens.e_r_pos],
            f_r: extreme(lens.f_r),
            f_l_pos: vec![(0, 1); lens.f_l_pos],
            tile_states: vec![[0i32; 16]; lens.tile_states],
        };
        let dec = Witness::from_field_elements(&w.to_field_elements(&p), &p).unwrap();
        assert_eq!(w, dec);
    }

    #[test]
    fn i32_boundary_values_round_trip() {
        let p = small_params();
        let lens = Witness::expected_lengths(&p);
        let mut state_max = [0i32; 16];
        let mut state_min = [0i32; 16];
        for k in 0..16 {
            state_max[k] = i32::MAX;
            state_min[k] = i32::MIN;
        }
        let mut tile_states = Vec::with_capacity(lens.tile_states);
        for step in 0..lens.tile_states {
            tile_states.push(if step % 2 == 0 { state_max } else { state_min });
        }
        let w = Witness {
            a_rows: vec![0i8; lens.a_rows],
            b_cols: vec![0i8; lens.b_cols],
            e_l: vec![0i8; lens.e_l],
            e_r_pos: vec![(0, 1); lens.e_r_pos],
            f_r: vec![0i8; lens.f_r],
            f_l_pos: vec![(0, 1); lens.f_l_pos],
            tile_states,
        };
        let dec = Witness::from_field_elements(&w.to_field_elements(&p), &p).unwrap();
        assert_eq!(w, dec);
    }

    #[test]
    fn u32_max_position_round_trips() {
        // Permutation positions are u32 by type; in practice they're
        // < noise_rank (≤ 2^16), but the encoding round-trips full u32.
        let p = small_params();
        let lens = Witness::expected_lengths(&p);
        let w = Witness {
            a_rows: vec![0i8; lens.a_rows],
            b_cols: vec![0i8; lens.b_cols],
            e_l: vec![0i8; lens.e_l],
            e_r_pos: vec![(u32::MAX, 0); lens.e_r_pos],
            f_r: vec![0i8; lens.f_r],
            f_l_pos: vec![(0, u32::MAX); lens.f_l_pos],
            tile_states: vec![[0i32; 16]; lens.tile_states],
        };
        let dec = Witness::from_field_elements(&w.to_field_elements(&p), &p).unwrap();
        assert_eq!(w, dec);
    }

    #[test]
    fn decode_rejects_wrong_length() {
        let p = small_params();
        let w = sample(&p);
        let mut enc = w.to_field_elements(&p);
        enc.pop();
        let err = Witness::from_field_elements(&enc, &p).unwrap_err();
        assert!(matches!(err, DecodeError::WrongLength { .. }));
        let mut enc = w.to_field_elements(&p);
        enc.push(<Goldilocks as QuotientMap<u64>>::from_int(0));
        let err = Witness::from_field_elements(&enc, &p).unwrap_err();
        assert!(matches!(err, DecodeError::WrongLength { .. }));
    }

    #[test]
    fn decode_rejects_i8_out_of_range() {
        let p = small_params();
        let w = sample(&p);
        let mut enc = w.to_field_elements(&p);
        // Index 0 is the first a_rows cell, encoded as u8 ≤ 0xff.
        // Replace with 0x100 → above the u8 range, triggers the i8 check.
        enc[0] = <Goldilocks as QuotientMap<u64>>::from_int(0x100);
        let err = Witness::from_field_elements(&enc, &p).unwrap_err();
        assert!(matches!(err, DecodeError::I8OutOfRange { index: 0, .. }));
    }

    #[test]
    fn decode_rejects_u32_out_of_range() {
        let p = small_params();
        let w = sample(&p);
        let lens = Witness::expected_lengths(&p);
        // Index of the first e_r_pos u32 = a_rows + b_cols + e_l.
        let first_pos_idx = lens.a_rows + lens.b_cols + lens.e_l;
        let mut enc = w.to_field_elements(&p);
        enc[first_pos_idx] = <Goldilocks as QuotientMap<u64>>::from_int(1u64 << 32);
        let err = Witness::from_field_elements(&enc, &p).unwrap_err();
        assert!(
            matches!(err, DecodeError::U32OutOfRange { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn decode_rejects_i32_out_of_range() {
        let p = small_params();
        let w = sample(&p);
        // Compute the index of the first tile_state cell.
        let lens = Witness::expected_lengths(&p);
        let first_state_idx =
            lens.a_rows + lens.b_cols + lens.e_l + lens.e_r_pos * 2 + lens.f_r + lens.f_l_pos * 2;
        let mut enc = w.to_field_elements(&p);
        enc[first_state_idx] = <Goldilocks as QuotientMap<u64>>::from_int(1u64 << 32);
        let err = Witness::from_field_elements(&enc, &p).unwrap_err();
        assert!(
            matches!(err, DecodeError::I32OutOfRange { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn changing_any_byte_changes_encoding() {
        let p = small_params();
        let w = sample(&p);
        let base = w.to_field_elements(&p);

        let mut w_a = w.clone();
        w_a.a_rows[0] ^= 0x01;
        assert_ne!(base, w_a.to_field_elements(&p));

        let mut w_b = w.clone();
        let last_b = w_b.b_cols.len() - 1;
        w_b.b_cols[last_b] ^= 0x01;
        assert_ne!(base, w_b.to_field_elements(&p));

        let mut w_e = w.clone();
        w_e.e_l[0] ^= 0x01;
        assert_ne!(base, w_e.to_field_elements(&p));

        let mut w_p = w.clone();
        w_p.e_r_pos[0].0 = w_p.e_r_pos[0].0.wrapping_add(1);
        assert_ne!(base, w_p.to_field_elements(&p));

        let mut w_s = w.clone();
        w_s.tile_states[0][0] = w_s.tile_states[0][0].wrapping_add(1);
        assert_ne!(base, w_s.to_field_elements(&p));
    }

    #[test]
    #[should_panic(expected = "a_rows length")]
    fn encode_panics_on_wrong_a_rows_length() {
        let p = small_params();
        let mut w = sample(&p);
        w.a_rows.pop();
        let _ = w.to_field_elements(&p);
    }

    #[test]
    fn larger_params_round_trip() {
        // Realistic-ish shape: m=n=64, k=64, r=32, tile=8.
        let p = ZkParams {
            m: 64,
            k: 64,
            n: 64,
            noise_rank: 32,
            tile: 8,
            difficulty_bits: 0,
        };
        let w = sample(&p);
        let dec = Witness::from_field_elements(&w.to_field_elements(&p), &p).unwrap();
        assert_eq!(w, dec);
    }
}
