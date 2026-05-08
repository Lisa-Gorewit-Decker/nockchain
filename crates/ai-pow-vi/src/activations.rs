//! Per-layer activation Merkle log.
//!
//! During the forward pass the prover commits each layer's activation
//! tensor as a tile-Merkle root (using `ai_pow::commit::merkle_root`).
//! The verifier later spot-checks a small Fiat-Shamir-sampled subset of
//! tiles by re-running the corresponding piece of the layer and matching
//! against the opened root.
//!
//! Wire format: layer roots concatenated in canonical layer order. Each
//! root is the merkle_root over the per-tile leaf hashes of the layer's
//! activation tensor, tiled into `tile × tile` blocks (row-major within a
//! tile; tiles ordered row-major over the tensor).
//!
//! Layout convention:
//! - Activation tensor: `(seq_len, hidden)` row-major i8.
//! - Tile size: same on both axes; both `seq_len` and `hidden` must be
//!   divisible by `tile` (no implicit padding here — the model layout
//!   chooses dimensions that already divide cleanly).
//! - Tile `(r, c)` covers rows `[r*tile, (r+1)*tile)` and columns
//!   `[c*tile, (c+1)*tile)`. Tile linear index `r * num_col_tiles + c`.

use ai_pow::commit::{merkle_path, merkle_recover_root, merkle_root, MerkleError};
use blake3::Hasher;
use thiserror::Error;

const CTX_ACTIVATION_TILE: &str = "ai-pow-vi v1 activation-tile";

/// Tile-grid description for an activation tensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivationLayout {
    pub seq_len: u32,
    pub hidden: u32,
    /// Tile size along both axes.
    pub tile: u32,
}

impl ActivationLayout {
    pub fn validate(&self) -> Result<(), ActivationError> {
        if self.seq_len == 0 || self.hidden == 0 || self.tile == 0 {
            return Err(ActivationError::ZeroDim);
        }
        if self.seq_len % self.tile != 0 {
            return Err(ActivationError::SeqLenNotMultipleOfTile);
        }
        if self.hidden % self.tile != 0 {
            return Err(ActivationError::HiddenNotMultipleOfTile);
        }
        Ok(())
    }

    pub fn num_row_tiles(&self) -> u32 {
        self.seq_len / self.tile
    }

    pub fn num_col_tiles(&self) -> u32 {
        self.hidden / self.tile
    }

    pub fn num_tiles(&self) -> u32 {
        self.num_row_tiles() * self.num_col_tiles()
    }
}

/// Cumulative log of per-layer activation Merkle commitments. Layers are
/// recorded sequentially; once recorded, a layer's root is fixed and can
/// be opened for individual tiles.
#[derive(Debug, Clone)]
pub struct ActivationLog {
    pub layout: ActivationLayout,
    /// Per-layer Merkle roots, in canonical layer order.
    pub layer_roots: Vec<[u8; 32]>,
    /// `tiles[layer_idx][tile_idx]` is the BLAKE3 leaf hash for that tile.
    /// Retained so the prover can build openings on demand.
    pub tiles: Vec<Vec<[u8; 32]>>,
}

/// Merkle opening for one activation tile: the leaf bytes (i.e. the raw
/// tile contents from the original activation tensor) plus the sibling
/// path. The verifier recomputes the leaf hash from the bytes, then the
/// root from leaf + path, then compares against the layer root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationOpening {
    pub layer_idx: u32,
    pub tile_idx: u32,
    /// `tile * tile` i8 bytes, row-major within the tile.
    pub tile_bytes: Vec<i8>,
    pub path: Vec<[u8; 32]>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActivationError {
    #[error("dimensions must be > 0")]
    ZeroDim,
    #[error("seq_len must be a multiple of tile")]
    SeqLenNotMultipleOfTile,
    #[error("hidden must be a multiple of tile")]
    HiddenNotMultipleOfTile,
    #[error("tensor length must equal seq_len * hidden")]
    BadTensorLen,
    #[error("layers must be recorded sequentially: expected layer_idx={expected}, got {got}")]
    NonSequentialLayer { expected: u32, got: u32 },
    #[error("layer index out of range: have {have} layers, asked for {asked}")]
    LayerOutOfRange { have: u32, asked: u32 },
    #[error("tile index out of range: have {have} tiles, asked for {asked}")]
    TileOutOfRange { have: u32, asked: u32 },
    #[error("merkle: {0}")]
    Merkle(#[from] MerkleError),
}

/// Hash a `tile × tile` block of i8 bytes under a domain-separated context.
/// The leaf bytes are reinterpreted as `u8` and fed in row-major.
fn tile_leaf_hash(tile_bytes: &[i8]) -> [u8; 32] {
    let mut hasher = Hasher::new_derive_key(CTX_ACTIVATION_TILE);
    hasher.update(&(tile_bytes.len() as u64).to_le_bytes());
    // SAFETY: i8 and u8 have the same size and alignment; we only read.
    let as_u8: &[u8] =
        unsafe { core::slice::from_raw_parts(tile_bytes.as_ptr() as *const u8, tile_bytes.len()) };
    hasher.update(as_u8);
    *hasher.finalize().as_bytes()
}

/// Materialize the row-major bytes of tile `(r, c)` from a
/// `(seq_len, hidden)` row-major i8 tensor.
fn extract_tile_bytes(tensor: &[i8], layout: &ActivationLayout, r: u32, c: u32) -> Vec<i8> {
    let tile = layout.tile as usize;
    let hidden = layout.hidden as usize;
    let mut out = Vec::with_capacity(tile * tile);
    let row_start = (r as usize) * tile;
    let col_start = (c as usize) * tile;
    for tr in 0..tile {
        let row = row_start + tr;
        let off = row * hidden + col_start;
        out.extend_from_slice(&tensor[off..off + tile]);
    }
    out
}

impl ActivationLog {
    /// Construct an empty log for a model with the given activation layout.
    pub fn new(layout: ActivationLayout) -> Result<Self, ActivationError> {
        layout.validate()?;
        Ok(Self {
            layout,
            layer_roots: Vec::new(),
            tiles: Vec::new(),
        })
    }

    /// Record one layer's activation tensor. Computes the per-tile leaf
    /// hashes, the layer Merkle root, and appends both. Layers must be
    /// recorded in order: `layer_idx` must equal the current length.
    pub fn record_layer(&mut self, layer_idx: u32, tensor: &[i8]) -> Result<(), ActivationError> {
        let expected = self.layer_roots.len() as u32;
        if layer_idx != expected {
            return Err(ActivationError::NonSequentialLayer {
                expected,
                got: layer_idx,
            });
        }
        let need = (self.layout.seq_len as usize) * (self.layout.hidden as usize);
        if tensor.len() != need {
            return Err(ActivationError::BadTensorLen);
        }

        let nr = self.layout.num_row_tiles();
        let nc = self.layout.num_col_tiles();
        let mut leaves: Vec<[u8; 32]> = Vec::with_capacity((nr * nc) as usize);
        for r in 0..nr {
            for c in 0..nc {
                let bytes = extract_tile_bytes(tensor, &self.layout, r, c);
                leaves.push(tile_leaf_hash(&bytes));
            }
        }
        let root = merkle_root(&leaves)?;
        self.layer_roots.push(root);
        self.tiles.push(leaves);
        Ok(())
    }

    /// Number of layers recorded so far.
    pub fn num_layers(&self) -> u32 {
        self.layer_roots.len() as u32
    }

    /// Lookup the root of a recorded layer.
    pub fn root(&self, layer_idx: u32) -> Result<[u8; 32], ActivationError> {
        let i = layer_idx as usize;
        if i >= self.layer_roots.len() {
            return Err(ActivationError::LayerOutOfRange {
                have: self.num_layers(),
                asked: layer_idx,
            });
        }
        Ok(self.layer_roots[i])
    }

    /// Build an opening for `(layer_idx, tile_idx)`. Pulls the tile bytes
    /// from `tensor` (the prover keeps this around) and the sibling path
    /// from the retained leaf hashes.
    pub fn open(
        &self,
        layer_idx: u32,
        tile_idx: u32,
        tensor: &[i8],
    ) -> Result<ActivationOpening, ActivationError> {
        let li = layer_idx as usize;
        if li >= self.tiles.len() {
            return Err(ActivationError::LayerOutOfRange {
                have: self.num_layers(),
                asked: layer_idx,
            });
        }
        let leaves = &self.tiles[li];
        let total = leaves.len() as u32;
        if tile_idx >= total {
            return Err(ActivationError::TileOutOfRange {
                have: total,
                asked: tile_idx,
            });
        }
        let need = (self.layout.seq_len as usize) * (self.layout.hidden as usize);
        if tensor.len() != need {
            return Err(ActivationError::BadTensorLen);
        }

        let nc = self.layout.num_col_tiles();
        let r = tile_idx / nc;
        let c = tile_idx % nc;
        let tile_bytes = extract_tile_bytes(tensor, &self.layout, r, c);
        let path = merkle_path(leaves, tile_idx as usize)?;
        Ok(ActivationOpening {
            layer_idx,
            tile_idx,
            tile_bytes,
            path,
        })
    }
}

/// Verifier-side: recompute the layer root from an opening and check it
/// matches `expected_root`. Returns `Ok(())` on match; `Err` otherwise.
pub fn verify_opening(
    layout: &ActivationLayout,
    expected_root: &[u8; 32],
    opening: &ActivationOpening,
) -> Result<(), ActivationError> {
    layout.validate()?;
    let tile = layout.tile as usize;
    if opening.tile_bytes.len() != tile * tile {
        return Err(ActivationError::BadTensorLen);
    }
    let total = layout.num_tiles();
    if opening.tile_idx >= total {
        return Err(ActivationError::TileOutOfRange {
            have: total,
            asked: opening.tile_idx,
        });
    }
    let leaf = tile_leaf_hash(&opening.tile_bytes);
    let recovered = merkle_recover_root(
        &leaf, opening.tile_idx as usize, &opening.path, total as usize,
    )?;
    if recovered == *expected_root {
        Ok(())
    } else {
        Err(ActivationError::Merkle(MerkleError::PathLengthMismatch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg_bytes(len: usize, seed: u64) -> Vec<i8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.wrapping_shr(56) as i8
            })
            .collect()
    }

    #[test]
    fn layout_validates_dimensions() {
        let bad = ActivationLayout {
            seq_len: 0,
            hidden: 8,
            tile: 4,
        };
        assert_eq!(bad.validate().err(), Some(ActivationError::ZeroDim));

        let bad = ActivationLayout {
            seq_len: 5,
            hidden: 8,
            tile: 4,
        };
        assert_eq!(
            bad.validate().err(),
            Some(ActivationError::SeqLenNotMultipleOfTile),
        );

        let bad = ActivationLayout {
            seq_len: 8,
            hidden: 7,
            tile: 4,
        };
        assert_eq!(
            bad.validate().err(),
            Some(ActivationError::HiddenNotMultipleOfTile),
        );

        let good = ActivationLayout {
            seq_len: 8,
            hidden: 16,
            tile: 4,
        };
        assert!(good.validate().is_ok());
        assert_eq!(good.num_row_tiles(), 2);
        assert_eq!(good.num_col_tiles(), 4);
        assert_eq!(good.num_tiles(), 8);
    }

    #[test]
    fn record_one_layer_matches_direct_merkle_root() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 8,
            tile: 2,
        };
        let tensor = lcg_bytes(4 * 8, 0xabcd);
        let mut log = ActivationLog::new(layout).unwrap();
        log.record_layer(0, &tensor).unwrap();

        // Recompute leaves directly and compare root.
        let mut leaves = Vec::new();
        for r in 0..layout.num_row_tiles() {
            for c in 0..layout.num_col_tiles() {
                leaves.push(tile_leaf_hash(&extract_tile_bytes(&tensor, &layout, r, c)));
            }
        }
        let direct = merkle_root(&leaves).unwrap();
        assert_eq!(log.root(0).unwrap(), direct);
    }

    #[test]
    fn open_any_tile_recovers_root() {
        let layout = ActivationLayout {
            seq_len: 8,
            hidden: 8,
            tile: 2,
        };
        let tensor = lcg_bytes(8 * 8, 0x1234);
        let mut log = ActivationLog::new(layout).unwrap();
        log.record_layer(0, &tensor).unwrap();
        let root = log.root(0).unwrap();
        let total = layout.num_tiles();
        for t in 0..total {
            let opening = log.open(0, t, &tensor).unwrap();
            verify_opening(&layout, &root, &opening).unwrap();
        }
    }

    #[test]
    fn tampering_one_byte_changes_root() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let mut tensor = lcg_bytes(16, 0x4242);
        let mut log_a = ActivationLog::new(layout).unwrap();
        log_a.record_layer(0, &tensor).unwrap();
        let root_a = log_a.root(0).unwrap();

        tensor[7] ^= 1;
        let mut log_b = ActivationLog::new(layout).unwrap();
        log_b.record_layer(0, &tensor).unwrap();
        let root_b = log_b.root(0).unwrap();

        assert_ne!(root_a, root_b);
    }

    #[test]
    fn tampered_opening_rejected() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let tensor = lcg_bytes(16, 0xfeed);
        let mut log = ActivationLog::new(layout).unwrap();
        log.record_layer(0, &tensor).unwrap();
        let root = log.root(0).unwrap();

        // Flip one byte of the opened tile_bytes; verify_opening must reject.
        let mut opening = log.open(0, 1, &tensor).unwrap();
        opening.tile_bytes[0] ^= 1;
        assert!(verify_opening(&layout, &root, &opening).is_err());

        // Original opening still verifies.
        let good = log.open(0, 1, &tensor).unwrap();
        verify_opening(&layout, &root, &good).unwrap();
    }

    #[test]
    fn record_requires_sequential_layer_idx() {
        let layout = ActivationLayout {
            seq_len: 2,
            hidden: 2,
            tile: 1,
        };
        let tensor = vec![0i8; 4];
        let mut log = ActivationLog::new(layout).unwrap();
        // First record must use layer_idx = 0.
        assert_eq!(
            log.record_layer(1, &tensor).err(),
            Some(ActivationError::NonSequentialLayer {
                expected: 0,
                got: 1
            }),
        );
        log.record_layer(0, &tensor).unwrap();
        // Now next must be layer_idx = 1.
        assert_eq!(
            log.record_layer(0, &tensor).err(),
            Some(ActivationError::NonSequentialLayer {
                expected: 1,
                got: 0
            }),
        );
        log.record_layer(1, &tensor).unwrap();
        assert_eq!(log.num_layers(), 2);
    }

    #[test]
    fn record_validates_tensor_len() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let mut log = ActivationLog::new(layout).unwrap();
        let bad = vec![0i8; 15];
        assert_eq!(
            log.record_layer(0, &bad).err(),
            Some(ActivationError::BadTensorLen),
        );
    }

    #[test]
    fn out_of_range_lookups_rejected() {
        let layout = ActivationLayout {
            seq_len: 2,
            hidden: 2,
            tile: 2,
        };
        let tensor = vec![1i8; 4];
        let mut log = ActivationLog::new(layout).unwrap();
        log.record_layer(0, &tensor).unwrap();

        assert_eq!(
            log.root(1).err(),
            Some(ActivationError::LayerOutOfRange { have: 1, asked: 1 }),
        );
        assert_eq!(
            log.open(1, 0, &tensor).err(),
            Some(ActivationError::LayerOutOfRange { have: 1, asked: 1 }),
        );
        // Only 1 tile (since seq_len=hidden=tile=2 → 1*1 tiles).
        assert_eq!(
            log.open(0, 1, &tensor).err(),
            Some(ActivationError::TileOutOfRange { have: 1, asked: 1 }),
        );
    }

    #[test]
    fn multi_layer_distinct_roots() {
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let mut log = ActivationLog::new(layout).unwrap();
        let t0 = lcg_bytes(16, 0x1);
        let t1 = lcg_bytes(16, 0x2);
        let t2 = lcg_bytes(16, 0x3);
        log.record_layer(0, &t0).unwrap();
        log.record_layer(1, &t1).unwrap();
        log.record_layer(2, &t2).unwrap();
        assert_eq!(log.num_layers(), 3);
        // All three tensors are distinct; all three roots must differ.
        let r0 = log.root(0).unwrap();
        let r1 = log.root(1).unwrap();
        let r2 = log.root(2).unwrap();
        assert_ne!(r0, r1);
        assert_ne!(r1, r2);
        assert_ne!(r0, r2);
    }

    #[test]
    fn determinism_two_logs_same_inputs_same_roots() {
        let layout = ActivationLayout {
            seq_len: 8,
            hidden: 8,
            tile: 2,
        };
        let tensor = lcg_bytes(64, 0xfeed_beef_cafe_babe);
        let mut a = ActivationLog::new(layout).unwrap();
        let mut b = ActivationLog::new(layout).unwrap();
        a.record_layer(0, &tensor).unwrap();
        b.record_layer(0, &tensor).unwrap();
        assert_eq!(a.layer_roots, b.layer_roots);
        assert_eq!(a.tiles, b.tiles);
    }

    #[test]
    fn open_for_unrecorded_layer_rejected() {
        let layout = ActivationLayout {
            seq_len: 2,
            hidden: 2,
            tile: 1,
        };
        let log = ActivationLog::new(layout).unwrap();
        let tensor = vec![0i8; 4];
        assert_eq!(
            log.open(0, 0, &tensor).err(),
            Some(ActivationError::LayerOutOfRange { have: 0, asked: 0 }),
        );
    }

    #[test]
    fn extract_tile_is_row_major_within_tile() {
        // (4, 4) tensor, tile=2. Tensor as row-major:
        // [ 1,  2,  3,  4,
        //   5,  6,  7,  8,
        //   9, 10, 11, 12,
        //  13, 14, 15, 16]
        // Tile (0,0) covers rows 0..2, cols 0..2 → [1, 2, 5, 6].
        // Tile (1,1) covers rows 2..4, cols 2..4 → [11, 12, 15, 16].
        let layout = ActivationLayout {
            seq_len: 4,
            hidden: 4,
            tile: 2,
        };
        let tensor: Vec<i8> = (1..=16).map(|x| x as i8).collect();
        let t00 = extract_tile_bytes(&tensor, &layout, 0, 0);
        assert_eq!(t00, vec![1i8, 2, 5, 6]);
        let t11 = extract_tile_bytes(&tensor, &layout, 1, 1);
        assert_eq!(t11, vec![11i8, 12, 15, 16]);
    }
}
