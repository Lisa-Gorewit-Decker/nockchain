//! `ViProof` wire format.
//!
//! Same shape as `ai-pow::proof::MatmulProof`, plus the LLM-side
//! commitments: model identifier, target-layer index, and the per-layer
//! activation Merkle roots up to and including the puzzle layer. The
//! proof body itself is the FFN tile-Merkle: a "found" tile that hashes
//! below `target`, plus σ Fiat-Shamir spot-checks.
//!
//! A future tightening pass will add weight-tile and activation-tile
//! openings (so the light verifier can avoid recomputation entirely);
//! the encoding leaves space for those by length-prefixing the spot-check
//! list.

use thiserror::Error;

/// One opened tile of the puzzle Merkle tree (`comm_M`). Carries the tile
/// coordinates, the leaf hash itself, and the sibling path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileOpening {
    pub i: u32,
    pub j: u32,
    pub m_ij: [u8; 32],
    pub path: Vec<[u8; 32]>,
}

/// Minimum-viable verifiable-inference proof.
///
/// `model_id` is the registered `comm_W` of the LLM. `layer_index` is the
/// FS-derived target layer at which the FFN gate matmul puzzle runs.
/// `comm_activations[i]` is the i-th per-layer activation Merkle root,
/// length `layer_index + 1` (one entry per layer 0..=layer_index).
/// `comm_m` is the tile-Merkle root over the FFN gate matmul tile-state
/// hashes at `layer_index`. `found` is the tile satisfying the hardness
/// target; `spot_checks` are σ FS-derived openings against `comm_m`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViProof {
    pub model_id: [u8; 32],
    pub layer_index: u32,
    pub comm_activations: Vec<[u8; 32]>,
    pub comm_m: [u8; 32],
    pub found: TileOpening,
    pub spot_checks: Vec<TileOpening>,
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
    #[error("activation roots count too large")]
    ActivationCountTooLarge,
}

const MAX_PATH_LEN: u32 = 64; // covers up to 2^64 leaves
const MAX_SPOT: u32 = 1 << 20; // 1M openings — sanity bound
const MAX_ACTIVATION_ROOTS: u32 = 1 << 16; // 64K layers — sanity bound

impl ViProof {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.model_id);
        out.extend_from_slice(&self.layer_index.to_le_bytes());
        out.extend_from_slice(&(self.comm_activations.len() as u32).to_le_bytes());
        for r in &self.comm_activations {
            out.extend_from_slice(r);
        }
        out.extend_from_slice(&self.comm_m);
        encode_opening(&self.found, &mut out);
        out.extend_from_slice(&(self.spot_checks.len() as u32).to_le_bytes());
        for s in &self.spot_checks {
            encode_opening(s, &mut out);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut cur = bytes;
        let model_id = take_arr32(&mut cur)?;
        let layer_index = take_u32(&mut cur)?;
        let n_act = take_u32(&mut cur)?;
        if n_act > MAX_ACTIVATION_ROOTS {
            return Err(DecodeError::ActivationCountTooLarge);
        }
        let mut comm_activations = Vec::with_capacity(n_act as usize);
        for _ in 0..n_act {
            comm_activations.push(take_arr32(&mut cur)?);
        }
        let comm_m = take_arr32(&mut cur)?;
        let found = decode_opening(&mut cur)?;
        let n = take_u32(&mut cur)?;
        if n > MAX_SPOT {
            return Err(DecodeError::SpotTooLarge);
        }
        let mut spot_checks = Vec::with_capacity(n as usize);
        for _ in 0..n {
            spot_checks.push(decode_opening(&mut cur)?);
        }
        if !cur.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(ViProof {
            model_id,
            layer_index,
            comm_activations,
            comm_m,
            found,
            spot_checks,
        })
    }
}

fn encode_opening(o: &TileOpening, out: &mut Vec<u8>) {
    out.extend_from_slice(&o.i.to_le_bytes());
    out.extend_from_slice(&o.j.to_le_bytes());
    out.extend_from_slice(&o.m_ij);
    out.extend_from_slice(&(o.path.len() as u32).to_le_bytes());
    for h in &o.path {
        out.extend_from_slice(h);
    }
}

fn decode_opening(cur: &mut &[u8]) -> Result<TileOpening, DecodeError> {
    let i = take_u32(cur)?;
    let j = take_u32(cur)?;
    let m_ij = take_arr32(cur)?;
    let pl = take_u32(cur)?;
    if pl > MAX_PATH_LEN {
        return Err(DecodeError::PathTooLarge);
    }
    let mut path = Vec::with_capacity(pl as usize);
    for _ in 0..pl {
        path.push(take_arr32(cur)?);
    }
    Ok(TileOpening { i, j, m_ij, path })
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

    fn sample() -> ViProof {
        ViProof {
            model_id: [1u8; 32],
            layer_index: 8,
            comm_activations: vec![[2u8; 32], [3u8; 32], [4u8; 32]],
            comm_m: [5u8; 32],
            found: TileOpening {
                i: 6,
                j: 7,
                m_ij: [8u8; 32],
                path: vec![[9u8; 32], [10u8; 32]],
            },
            spot_checks: vec![
                TileOpening {
                    i: 11,
                    j: 12,
                    m_ij: [13u8; 32],
                    path: vec![[14u8; 32]],
                },
                TileOpening {
                    i: 15,
                    j: 16,
                    m_ij: [17u8; 32],
                    path: vec![],
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let p = sample();
        let bytes = p.encode();
        let q = ViProof::decode(&bytes).unwrap();
        assert_eq!(p, q);
    }

    #[test]
    fn rejects_truncated() {
        let p = sample();
        let bytes = p.encode();
        for cut in 0..bytes.len() {
            assert!(
                ViProof::decode(&bytes[..cut]).is_err(),
                "expected error at cut={cut}",
            );
        }
    }

    #[test]
    fn rejects_trailing() {
        let p = sample();
        let mut bytes = p.encode();
        bytes.push(0xff);
        assert_eq!(ViProof::decode(&bytes).err(), Some(DecodeError::Trailing));
    }

    #[test]
    fn rejects_path_too_large() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0u8; 32]); // model_id
        bytes.extend_from_slice(&0u32.to_le_bytes()); // layer_index
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 0 activation roots
        bytes.extend_from_slice(&[0u8; 32]); // comm_m
        bytes.extend_from_slice(&0u32.to_le_bytes()); // i
        bytes.extend_from_slice(&0u32.to_le_bytes()); // j
        bytes.extend_from_slice(&[0u8; 32]); // m_ij
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // path len
        assert_eq!(
            ViProof::decode(&bytes).err(),
            Some(DecodeError::PathTooLarge)
        );
    }

    #[test]
    fn rejects_too_many_activation_roots() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0u8; 32]); // model_id
        bytes.extend_from_slice(&0u32.to_le_bytes()); // layer_index
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // activation count
        assert_eq!(
            ViProof::decode(&bytes).err(),
            Some(DecodeError::ActivationCountTooLarge),
        );
    }

    #[test]
    fn empty_proof_round_trips() {
        let p = ViProof {
            model_id: [0u8; 32],
            layer_index: 0,
            comm_activations: vec![[42u8; 32]],
            comm_m: [0u8; 32],
            found: TileOpening {
                i: 0,
                j: 0,
                m_ij: [0u8; 32],
                path: vec![],
            },
            spot_checks: vec![],
        };
        assert_eq!(ViProof::decode(&p.encode()).unwrap(), p);
    }
}
