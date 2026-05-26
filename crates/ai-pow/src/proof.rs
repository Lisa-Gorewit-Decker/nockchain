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
