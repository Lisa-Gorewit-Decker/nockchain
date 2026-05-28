//! Wire format for `MatmulProof`.
//!
//! Per Pearl §4.6 the block-opening proof contains:
//!  * Matrix commitments `H_A`, `H_B`.
//!  * For each opened tile: the row strips of `A` (and column strips of `B`)
//!    used by the tile, plus a Merkle authentication path per strip up to
//!    the matrix commitment.
//!  * The tile-state Merkle path up to `comm_m`.
//!
//! All variable-length fields are length-prefixed for unambiguous decoding.
//! For sanity-bounded networks we cap path / spot / strip sizes.

use thiserror::Error;

use crate::params::{MatmulParams, ParamError};

/// Opening of a single tile, sufficient for the verifier to reconstruct
/// `M_{i,j}` and verify it against `comm_m`, `h_a`, and `h_b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileOpening {
    pub i: u32,
    pub j: u32,
    /// Sibling path from this tile's leaf up to `comm_m`.
    pub m_path: Vec<[u8; 32]>,
    /// `tile` row strips of `A`, each of length `k`, concatenated row-major.
    /// `a_rows[di * k .. (di + 1) * k]` is row `i * tile + di` of `A`.
    pub a_rows: Vec<i8>,
    /// `tile` column strips of `B`, each of length `k`, concatenated
    /// column-major. `b_cols[dj * k .. (dj + 1) * k]` is column `j * tile + dj`
    /// of `B`.
    pub b_cols: Vec<i8>,
    /// Per-row-strip sibling path up to `h_a`. Length `tile`; each inner
    /// path has length `ceil(log2(m_padded))`.
    pub a_row_paths: Vec<Vec<[u8; 32]>>,
    /// Per-column-strip sibling path up to `h_b`. Length `tile`; each inner
    /// path has length `ceil(log2(n_padded))`.
    pub b_col_paths: Vec<Vec<[u8; 32]>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatmulProof {
    pub comm_m: [u8; 32],
    pub params_tag: [u8; 32],
    /// Pearl §4.3 `H_A`: BLAKE3-Merkle root over the rows of `A`.
    pub h_a: [u8; 32],
    /// Pearl §4.3 `H_B`: BLAKE3-Merkle root over the columns of `B`.
    pub h_b: [u8; 32],
    /// M52 step 5: chunk-Merkle `BLAKE3(pad(A_row_major), key=κ)`.
    /// The commitment shape the `ai-pow-zk` SNARK binds to as
    /// public input `HASH_A`. Distinct from `h_a` (row-Merkle,
    /// used for spot-check opening) — they coexist.
    pub h_a_chunk: [u8; 32],
    /// M52 step 5: chunk-Merkle commitment for matrix B.
    pub h_b_chunk: [u8; 32],
    pub found: TileOpening,
    pub spot: Vec<TileOpening>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("invalid params: {0}")]
    InvalidParams(#[from] ParamError),
    #[error("unexpected end of input")]
    Eof,
    #[error("trailing bytes after decode")]
    Trailing,
    #[error("path length too large")]
    PathTooLarge,
    #[error("spot count too large")]
    SpotTooLarge,
    #[error("strip count too large")]
    StripCountTooLarge,
    #[error("strip length too large")]
    StripLenTooLarge,
    #[error("proof bytes too large for params (max {max}, got {actual})")]
    ProofTooLarge { max: usize, actual: usize },
    #[error("spot count mismatch (expected {expected}, got {actual})")]
    SpotCountMismatch { expected: u32, actual: u32 },
    #[error("{what} length mismatch (expected {expected}, got {actual})")]
    StripLenMismatch {
        what: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("{what} path count mismatch (expected {expected}, got {actual})")]
    PathCountMismatch {
        what: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("{what} path depth mismatch (expected {expected}, got {actual})")]
    PathDepthMismatch {
        what: &'static str,
        expected: usize,
        actual: usize,
    },
}

const MAX_PATH_LEN: u32 = 64;
const MAX_SPOT: u32 = 1 << 20;
const MAX_STRIP_COUNT: u32 = 1 << 20;
const MAX_STRIP_LEN: u32 = 1 << 24; // 16 MiB cap per strip-concat field

impl MatmulProof {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.comm_m);
        out.extend_from_slice(&self.params_tag);
        out.extend_from_slice(&self.h_a);
        out.extend_from_slice(&self.h_b);
        out.extend_from_slice(&self.h_a_chunk);
        out.extend_from_slice(&self.h_b_chunk);
        encode_opening(&self.found, &mut out);
        out.extend_from_slice(&(self.spot.len() as u32).to_le_bytes());
        for s in &self.spot {
            encode_opening(s, &mut out);
        }
        out
    }

    /// Decode a syntactically valid proof without knowing the expected puzzle
    /// shape.
    ///
    /// This decoder is intentionally loose so tests and offline tooling can
    /// inspect malformed proofs. Verifier-facing code should use
    /// [`Self::decode_for_params`], which applies the exact shape and count
    /// limits before allocating attacker-declared fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut cur = bytes;
        let comm_m = take_arr32(&mut cur)?;
        let params_tag = take_arr32(&mut cur)?;
        let h_a = take_arr32(&mut cur)?;
        let h_b = take_arr32(&mut cur)?;
        let h_a_chunk = take_arr32(&mut cur)?;
        let h_b_chunk = take_arr32(&mut cur)?;
        let found = decode_opening(&mut cur)?;
        let n = take_u32(&mut cur)?;
        if n > MAX_SPOT {
            return Err(DecodeError::SpotTooLarge);
        }
        // Do NOT `Vec::with_capacity(n)`: `n` is an attacker-declared
        // count, bounded only by the loose `MAX_SPOT` policy cap
        // (2^20). A ~200-byte blob could declare 2^20 spot entries and
        // force a ~100 MiB up-front allocation — a deserialization
        // bomb. Grow the Vec as openings are *actually* decoded:
        // `decode_opening` fails fast at EOF, so the loop
        // self-terminates and the allocation stays proportional to the
        // real input length.
        let mut spot = Vec::new();
        for _ in 0..n {
            spot.push(decode_opening(&mut cur)?);
        }
        if !cur.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(MatmulProof {
            comm_m,
            params_tag,
            h_a,
            h_b,
            h_a_chunk,
            h_b_chunk,
            found,
            spot,
        })
    }

    /// Decode with the exact shape required by `params`.
    ///
    /// This is the verifier-facing decoder: it rejects count, strip, path,
    /// depth, and total-size mismatches while reading length prefixes, before
    /// allocating attacker-declared large fields that a later verifier would
    /// reject anyway.
    pub fn decode_for_params(bytes: &[u8], params: &MatmulParams) -> Result<Self, DecodeError> {
        params.validate()?;
        let shape = DecodeShape::new(params);
        let max = shape.encoded_len(params.spot_checks as usize);
        if bytes.len() > max {
            return Err(DecodeError::ProofTooLarge {
                max,
                actual: bytes.len(),
            });
        }

        let mut cur = bytes;
        let comm_m = take_arr32(&mut cur)?;
        let params_tag = take_arr32(&mut cur)?;
        let h_a = take_arr32(&mut cur)?;
        let h_b = take_arr32(&mut cur)?;
        let h_a_chunk = take_arr32(&mut cur)?;
        let h_b_chunk = take_arr32(&mut cur)?;
        let found = decode_opening_for_shape(&mut cur, &shape)?;
        let n = take_u32(&mut cur)?;
        if n != params.spot_checks {
            return Err(DecodeError::SpotCountMismatch {
                expected: params.spot_checks,
                actual: n,
            });
        }
        let mut spot = Vec::with_capacity(n as usize);
        for _ in 0..n {
            spot.push(decode_opening_for_shape(&mut cur, &shape)?);
        }
        if !cur.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(MatmulProof {
            comm_m,
            params_tag,
            h_a,
            h_b,
            h_a_chunk,
            h_b_chunk,
            found,
            spot,
        })
    }
}

struct DecodeShape {
    tile: usize,
    strip_len: usize,
    m_path_depth: usize,
    a_path_depth: usize,
    b_path_depth: usize,
}

impl DecodeShape {
    fn new(params: &MatmulParams) -> Self {
        Self {
            tile: params.tile as usize,
            strip_len: params.tile as usize * params.k as usize,
            m_path_depth: params.num_tiles_padded().trailing_zeros() as usize,
            a_path_depth: params.m.next_power_of_two().trailing_zeros() as usize,
            b_path_depth: params.n.next_power_of_two().trailing_zeros() as usize,
        }
    }

    fn encoded_len(&self, spot_checks: usize) -> usize {
        (32 * 6) + self.opening_len() + 4 + spot_checks * self.opening_len()
    }

    fn opening_len(&self) -> usize {
        8 + (4 + self.m_path_depth * 32)
            + (4 + self.strip_len)
            + (4 + self.strip_len)
            + (4 + self.tile * (4 + self.a_path_depth * 32))
            + (4 + self.tile * (4 + self.b_path_depth * 32))
    }
}

fn encode_opening(o: &TileOpening, out: &mut Vec<u8>) {
    out.extend_from_slice(&o.i.to_le_bytes());
    out.extend_from_slice(&o.j.to_le_bytes());
    encode_path_list_single(&o.m_path, out);
    encode_i8_slice(&o.a_rows, out);
    encode_i8_slice(&o.b_cols, out);
    encode_path_list(&o.a_row_paths, out);
    encode_path_list(&o.b_col_paths, out);
}

fn decode_opening(cur: &mut &[u8]) -> Result<TileOpening, DecodeError> {
    let i = take_u32(cur)?;
    let j = take_u32(cur)?;
    let m_path = decode_path_single(cur)?;
    let a_rows = decode_i8_slice(cur)?;
    let b_cols = decode_i8_slice(cur)?;
    let a_row_paths = decode_path_list(cur)?;
    let b_col_paths = decode_path_list(cur)?;
    Ok(TileOpening {
        i,
        j,
        m_path,
        a_rows,
        b_cols,
        a_row_paths,
        b_col_paths,
    })
}

fn decode_opening_for_shape(
    cur: &mut &[u8],
    shape: &DecodeShape,
) -> Result<TileOpening, DecodeError> {
    let i = take_u32(cur)?;
    let j = take_u32(cur)?;
    let m_path = decode_path_single_exact(cur, shape.m_path_depth, "m_path")?;
    let a_rows = decode_i8_slice_exact(cur, shape.strip_len, "a_rows")?;
    let b_cols = decode_i8_slice_exact(cur, shape.strip_len, "b_cols")?;
    let a_row_paths = decode_path_list_exact(
        cur,
        shape.tile,
        shape.a_path_depth,
        "a_row_paths",
    )?;
    let b_col_paths = decode_path_list_exact(
        cur,
        shape.tile,
        shape.b_path_depth,
        "b_col_paths",
    )?;
    Ok(TileOpening {
        i,
        j,
        m_path,
        a_rows,
        b_cols,
        a_row_paths,
        b_col_paths,
    })
}

fn encode_path_list_single(path: &[[u8; 32]], out: &mut Vec<u8>) {
    out.extend_from_slice(&(path.len() as u32).to_le_bytes());
    for h in path {
        out.extend_from_slice(h);
    }
}

fn decode_path_single(cur: &mut &[u8]) -> Result<Vec<[u8; 32]>, DecodeError> {
    let pl = take_u32(cur)?;
    if pl > MAX_PATH_LEN {
        return Err(DecodeError::PathTooLarge);
    }
    let mut path = Vec::with_capacity(pl as usize);
    for _ in 0..pl {
        path.push(take_arr32(cur)?);
    }
    Ok(path)
}

fn decode_path_single_exact(
    cur: &mut &[u8],
    expected: usize,
    what: &'static str,
) -> Result<Vec<[u8; 32]>, DecodeError> {
    let pl = take_u32(cur)? as usize;
    if pl != expected {
        return Err(DecodeError::PathDepthMismatch {
            what,
            expected,
            actual: pl,
        });
    }
    let mut path = Vec::with_capacity(pl);
    for _ in 0..pl {
        path.push(take_arr32(cur)?);
    }
    Ok(path)
}

fn encode_path_list(paths: &[Vec<[u8; 32]>], out: &mut Vec<u8>) {
    out.extend_from_slice(&(paths.len() as u32).to_le_bytes());
    for p in paths {
        encode_path_list_single(p, out);
    }
}

fn decode_path_list(cur: &mut &[u8]) -> Result<Vec<Vec<[u8; 32]>>, DecodeError> {
    let n = take_u32(cur)?;
    if n > MAX_STRIP_COUNT {
        return Err(DecodeError::StripCountTooLarge);
    }
    // Attacker-declared count — no `with_capacity` (see `decode`):
    // grow as path-lists are actually decoded.
    let mut paths = Vec::new();
    for _ in 0..n {
        paths.push(decode_path_single(cur)?);
    }
    Ok(paths)
}

fn decode_path_list_exact(
    cur: &mut &[u8],
    expected_count: usize,
    expected_depth: usize,
    what: &'static str,
) -> Result<Vec<Vec<[u8; 32]>>, DecodeError> {
    let n = take_u32(cur)? as usize;
    if n != expected_count {
        return Err(DecodeError::PathCountMismatch {
            what,
            expected: expected_count,
            actual: n,
        });
    }
    let mut paths = Vec::with_capacity(n);
    for _ in 0..n {
        paths.push(decode_path_single_exact(cur, expected_depth, what)?);
    }
    Ok(paths)
}

fn encode_i8_slice(s: &[i8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    // SAFETY: i8 and u8 share layout; we only ship the raw byte pattern.
    let bytes: &[u8] = unsafe { core::slice::from_raw_parts(s.as_ptr() as *const u8, s.len()) };
    out.extend_from_slice(bytes);
}

fn decode_i8_slice(cur: &mut &[u8]) -> Result<Vec<i8>, DecodeError> {
    let len = take_u32(cur)? as usize;
    if (len as u32) > MAX_STRIP_LEN {
        return Err(DecodeError::StripLenTooLarge);
    }
    if cur.len() < len {
        return Err(DecodeError::Eof);
    }
    let bytes = &cur[..len];
    *cur = &cur[len..];
    // SAFETY: same layout in reverse.
    let v: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Ok(v)
}

fn decode_i8_slice_exact(
    cur: &mut &[u8],
    expected: usize,
    what: &'static str,
) -> Result<Vec<i8>, DecodeError> {
    let len = take_u32(cur)? as usize;
    if len != expected {
        return Err(DecodeError::StripLenMismatch {
            what,
            expected,
            actual: len,
        });
    }
    if cur.len() < len {
        return Err(DecodeError::Eof);
    }
    let bytes = &cur[..len];
    *cur = &cur[len..];
    Ok(bytes.iter().map(|&b| b as i8).collect())
}

fn take_u32(cur: &mut &[u8]) -> Result<u32, DecodeError> {
    if cur.len() < 4 {
        return Err(DecodeError::Eof);
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&cur[..4]);
    *cur = &cur[4..];
    Ok(u32::from_le_bytes(buf))
}

fn take_arr32(cur: &mut &[u8]) -> Result<[u8; 32], DecodeError> {
    if cur.len() < 32 {
        return Err(DecodeError::Eof);
    }
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&cur[..32]);
    *cur = &cur[32..];
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shaped_params() -> MatmulParams {
        MatmulParams {
            m: 8,
            k: 64,
            n: 8,
            noise_rank: 4,
            tile: 8,
            spot_checks: 1,
            difficulty_bits: 0,
        }
    }

    fn shaped_opening(i: u32, j: u32) -> TileOpening {
        TileOpening {
            i,
            j,
            m_path: Vec::new(),
            a_rows: vec![1; 8 * 64],
            b_cols: vec![2; 8 * 64],
            a_row_paths: vec![vec![[3u8; 32]; 3]; 8],
            b_col_paths: vec![vec![[4u8; 32]; 3]; 8],
        }
    }

    fn shaped_proof(spot_count: usize) -> MatmulProof {
        MatmulProof {
            comm_m: [1u8; 32],
            params_tag: [2u8; 32],
            h_a: [3u8; 32],
            h_b: [4u8; 32],
            h_a_chunk: [5u8; 32],
            h_b_chunk: [6u8; 32],
            found: shaped_opening(0, 0),
            spot: (0..spot_count)
                .map(|idx| shaped_opening(0, idx as u32))
                .collect(),
        }
    }

    fn sample_opening(seed: u8) -> TileOpening {
        TileOpening {
            i: seed as u32,
            j: (seed as u32) + 1,
            m_path: vec![[seed; 32], [seed.wrapping_add(1); 32]],
            a_rows: (0..16).map(|x| (x as i8) - 8).collect(),
            b_cols: (0..16).map(|x| (x as i8) - 4).collect(),
            a_row_paths: vec![vec![[seed; 32]], vec![[seed.wrapping_add(2); 32]]],
            b_col_paths: vec![vec![[seed; 32]], vec![]],
        }
    }

    fn sample() -> MatmulProof {
        MatmulProof {
            comm_m: [1u8; 32],
            params_tag: [2u8; 32],
            h_a: [3u8; 32],
            h_b: [4u8; 32],
            h_a_chunk: [8u8; 32],
            h_b_chunk: [9u8; 32],
            found: sample_opening(5),
            spot: vec![sample_opening(6), sample_opening(7)],
        }
    }

    #[test]
    fn round_trip() {
        let p = sample();
        let bytes = p.encode();
        let q = MatmulProof::decode(&bytes).unwrap();
        assert_eq!(p, q);
    }

    #[test]
    fn decode_for_params_round_trip_exact_shape() {
        let params = shaped_params();
        let p = shaped_proof(params.spot_checks as usize);
        let bytes = p.encode();
        let q = MatmulProof::decode_for_params(&bytes, &params).unwrap();
        assert_eq!(p, q);
    }

    #[test]
    fn decode_for_params_rejects_wrong_spot_count_before_spot_bodies() {
        let params = shaped_params();
        let p = shaped_proof(2);
        let bytes = p.encode();
        assert_eq!(
            MatmulProof::decode_for_params(&bytes, &params).err(),
            Some(DecodeError::ProofTooLarge {
                max: DecodeShape::new(&params).encoded_len(params.spot_checks as usize),
                actual: bytes.len(),
            })
        );

        let mut prefix_only = shaped_proof(0);
        prefix_only.spot = Vec::new();
        let mut bytes = prefix_only.encode();
        let count_offset = bytes.len() - 4;
        bytes[count_offset..].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            MatmulProof::decode_for_params(&bytes, &params).err(),
            Some(DecodeError::SpotCountMismatch {
                expected: 1,
                actual: 2,
            })
        );
    }

    #[test]
    fn decode_for_params_rejects_wrong_strip_len_before_allocating_body() {
        let params = shaped_params();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0u8; 32 * 6]);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // i
        bytes.extend_from_slice(&0u32.to_le_bytes()); // j
        bytes.extend_from_slice(&0u32.to_le_bytes()); // m_path depth
        bytes.extend_from_slice(&((params.tile * params.k) + 1).to_le_bytes()); // a_rows len
        assert_eq!(
            MatmulProof::decode_for_params(&bytes, &params).err(),
            Some(DecodeError::StripLenMismatch {
                what: "a_rows",
                expected: (params.tile * params.k) as usize,
                actual: (params.tile * params.k + 1) as usize,
            })
        );
    }

    #[test]
    fn rejects_truncated() {
        let p = sample();
        let bytes = p.encode();
        for cut in 0..bytes.len() {
            let r = MatmulProof::decode(&bytes[..cut]);
            assert!(r.is_err(), "expected error at cut={cut}");
        }
    }

    #[test]
    fn rejects_trailing() {
        let p = sample();
        let mut bytes = p.encode();
        bytes.push(0);
        assert_eq!(
            MatmulProof::decode(&bytes).err(),
            Some(DecodeError::Trailing)
        );
    }

    #[test]
    fn rejects_huge_path() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0u8; 32]); // comm_m
        bytes.extend_from_slice(&[0u8; 32]); // params_tag
        bytes.extend_from_slice(&[0u8; 32]); // h_a
        bytes.extend_from_slice(&[0u8; 32]); // h_b
        bytes.extend_from_slice(&[0u8; 32]); // h_a_chunk
        bytes.extend_from_slice(&[0u8; 32]); // h_b_chunk
        bytes.extend_from_slice(&0u32.to_le_bytes()); // i
        bytes.extend_from_slice(&0u32.to_le_bytes()); // j
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // m_path len
        assert_eq!(
            MatmulProof::decode(&bytes).err(),
            Some(DecodeError::PathTooLarge)
        );
    }
}
