//! Wire format for `MatmulProof`.
//!
//! The proof is intentionally compact: each tile opening contains only the
//! tile coordinates and the Merkle sibling path to `comm_m`. The matrix rows
//! and columns are *not* shipped — the verifier re-derives them from the
//! same `state` and noise seed used by the prover. This makes proofs
//! `~32 + sigma * (8 + depth * 32)` bytes regardless of `(m, k, n)`.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileOpening {
    pub i: u32,
    pub j: u32,
    pub path: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatmulProof {
    pub comm_m: [u8; 32],
    pub params_tag: [u8; 32],
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
}

const MAX_PATH_LEN: u32 = 64; // covers up to 2^64 leaves; far beyond practical
const MAX_SPOT: u32 = 1 << 20; // 1M openings max; sanity bound

impl MatmulProof {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.comm_m);
        out.extend_from_slice(&self.params_tag);
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
        let found = decode_opening(&mut cur)?;
        let n = take_u32(&mut cur)?;
        if n > MAX_SPOT {
            return Err(DecodeError::SpotTooLarge);
        }
        let mut spot = Vec::with_capacity(n as usize);
        for _ in 0..n {
            spot.push(decode_opening(&mut cur)?);
        }
        if !cur.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(MatmulProof {
            comm_m,
            params_tag,
            found,
            spot,
        })
    }
}

fn encode_opening(o: &TileOpening, out: &mut Vec<u8>) {
    out.extend_from_slice(&o.i.to_le_bytes());
    out.extend_from_slice(&o.j.to_le_bytes());
    out.extend_from_slice(&(o.path.len() as u32).to_le_bytes());
    for h in &o.path {
        out.extend_from_slice(h);
    }
}

fn decode_opening(cur: &mut &[u8]) -> Result<TileOpening, DecodeError> {
    let i = take_u32(cur)?;
    let j = take_u32(cur)?;
    let pl = take_u32(cur)?;
    if pl > MAX_PATH_LEN {
        return Err(DecodeError::PathTooLarge);
    }
    let mut path = Vec::with_capacity(pl as usize);
    for _ in 0..pl {
        path.push(take_arr32(cur)?);
    }
    Ok(TileOpening { i, j, path })
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

    fn sample() -> MatmulProof {
        MatmulProof {
            comm_m: [1u8; 32],
            params_tag: [2u8; 32],
            found: TileOpening {
                i: 3,
                j: 4,
                path: vec![[5u8; 32], [6u8; 32]],
            },
            spot: vec![
                TileOpening {
                    i: 7,
                    j: 8,
                    path: vec![[9u8; 32]],
                },
                TileOpening {
                    i: 10,
                    j: 11,
                    path: vec![],
                },
            ],
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
        bytes.extend_from_slice(&0u32.to_le_bytes()); // i
        bytes.extend_from_slice(&0u32.to_le_bytes()); // j
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // path len
        assert_eq!(
            MatmulProof::decode(&bytes).err(),
            Some(DecodeError::PathTooLarge)
        );
    }
}
