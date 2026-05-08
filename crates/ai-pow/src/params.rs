//! Puzzle parameters.

use thiserror::Error;

/// Parameters of a matmul PoUW puzzle.
///
/// The matmul has shape `(m, k) * (k, n) = (m, n)` with INT8 inputs and i32
/// accumulator. Output tiles are `t x t`.  `noise_rank` is reserved for the
/// future low-rank decomposition of `E, F`; it is mixed into the Fiat-Shamir
/// transcript so changing it forces a different puzzle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatmulParams {
    pub m: u32,
    pub k: u32,
    pub n: u32,
    pub noise_rank: u32,
    pub tile: u32,
    pub spot_checks: u32,
    pub lambda: u32,
}

impl MatmulParams {
    /// Default test profile — small enough to run end-to-end in milliseconds.
    pub const TEST_SMALL: Self = Self {
        m: 64,
        k: 64,
        n: 64,
        noise_rank: 8,
        tile: 8,
        spot_checks: 8,
        lambda: 8,
    };

    /// Production profile from the plan: 4096^3 INT8 matmul, 128-tile,
    /// 80 spot checks. Use for benches; tests run TEST_SMALL.
    pub const PROD: Self = Self {
        m: 4096,
        k: 4096,
        n: 4096,
        noise_rank: 64,
        tile: 128,
        spot_checks: 80,
        lambda: 80,
    };

    /// Gemma 4 31B FFN gate / up matmul: `(B=4096, hidden=5376, intermediate=21504)`.
    /// At 300 TOPS INT8 ≈ 1.5 ms per attempt; same kernel as the model's gate / up.
    pub const GEMMA_4_31B_FFN: Self = Self {
        m: 4096,
        k: 5376,
        n: 21504,
        noise_rank: 64,
        tile: 64,
        spot_checks: 80,
        lambda: 80,
    };

    /// Qwen 3.6 27B FFN gate / up matmul: `(B=4096, hidden=5120, intermediate=17408)`.
    /// At 300 TOPS INT8 ≈ 1.2 ms per attempt; same kernel as the model's gate / up.
    pub const QWEN_3_6_27B_FFN: Self = Self {
        m: 4096,
        k: 5120,
        n: 17408,
        noise_rank: 64,
        tile: 64,
        spot_checks: 80,
        lambda: 80,
    };

    /// Generic LLM-FFN profile builder. `batch_seq` is the M dimension (the
    /// product of mini-batch and sequence length the GEMM kernel sees);
    /// `hidden` and `intermediate` are the two model dimensions for the FFN
    /// gate / up matmul. Picks `tile = 64` and `sigma = 80`.
    pub const fn llm_ffn(hidden: u32, intermediate: u32, batch_seq: u32) -> Self {
        Self {
            m: batch_seq,
            k: hidden,
            n: intermediate,
            noise_rank: 64,
            tile: 64,
            spot_checks: 80,
            lambda: 80,
        }
    }

    pub fn validate(&self) -> Result<(), ParamError> {
        if self.tile == 0 {
            return Err(ParamError::ZeroTile);
        }
        if self.m % self.tile != 0 || self.n % self.tile != 0 {
            return Err(ParamError::TileDoesNotDivide);
        }
        let row_tiles = self.m / self.tile;
        let col_tiles = self.n / self.tile;
        let total = (row_tiles as u64) * (col_tiles as u64);
        if total == 0 {
            return Err(ParamError::ZeroTiles);
        }
        // No power-of-two requirement: the Merkle tree pads the leaf set to
        // the next power of two with a fixed sentinel. This lets us match real
        // LLM matmul shapes whose dimensions are rarely powers of two.
        // i32 dot product safety: max |A_il + E_il| <= 256, same for B+F.
        // Per-product bound 256*256 = 2^16, summed k times => k*2^16 must
        // fit in i32 (< 2^31), i.e. k < 2^15.
        if self.k == 0 || self.k > (1u32 << 15) {
            return Err(ParamError::KOutOfRange);
        }
        if self.spot_checks == 0 {
            return Err(ParamError::ZeroSpotChecks);
        }
        if (self.spot_checks as u64) > total {
            return Err(ParamError::TooManySpotChecks);
        }
        Ok(())
    }

    pub fn row_tiles(&self) -> u32 {
        self.m / self.tile
    }
    pub fn col_tiles(&self) -> u32 {
        self.n / self.tile
    }
    pub fn num_tiles(&self) -> u32 {
        self.row_tiles() * self.col_tiles()
    }
    /// Number of leaves in the padded Merkle tree (next power of two of
    /// `num_tiles`).
    pub fn num_tiles_padded(&self) -> u32 {
        self.num_tiles().next_power_of_two()
    }
    pub fn tile_index(&self, i: u32, j: u32) -> u32 {
        i * self.col_tiles() + j
    }
    pub fn tile_coords(&self, idx: u32) -> (u32, u32) {
        let cols = self.col_tiles();
        (idx / cols, idx % cols)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParamError {
    #[error("tile size must be > 0")]
    ZeroTile,
    #[error("tile must divide m and n")]
    TileDoesNotDivide,
    #[error("(m/t)*(n/t) must be > 0")]
    ZeroTiles,
    #[error("k must be in 1..=2^15 to keep dot products in i32 range")]
    KOutOfRange,
    #[error("spot_checks must be > 0")]
    ZeroSpotChecks,
    #[error("spot_checks must be <= number of tiles")]
    TooManySpotChecks,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        MatmulParams::TEST_SMALL.validate().unwrap();
        MatmulParams::PROD.validate().unwrap();
        MatmulParams::GEMMA_4_31B_FFN.validate().unwrap();
        MatmulParams::QWEN_3_6_27B_FFN.validate().unwrap();
    }

    #[test]
    fn rejects_bad_params() {
        let mut p = MatmulParams::TEST_SMALL;
        p.tile = 0;
        assert_eq!(p.validate(), Err(ParamError::ZeroTile));

        p = MatmulParams::TEST_SMALL;
        p.tile = 7;
        assert_eq!(p.validate(), Err(ParamError::TileDoesNotDivide));

        p = MatmulParams::TEST_SMALL;
        p.k = (1 << 15) + 1;
        assert_eq!(p.validate(), Err(ParamError::KOutOfRange));

        p = MatmulParams::TEST_SMALL;
        p.spot_checks = 0;
        assert_eq!(p.validate(), Err(ParamError::ZeroSpotChecks));

        p = MatmulParams::TEST_SMALL;
        p.spot_checks = p.num_tiles() + 1;
        assert_eq!(p.validate(), Err(ParamError::TooManySpotChecks));
    }

    #[test]
    fn coord_round_trip() {
        let p = MatmulParams::TEST_SMALL;
        for idx in 0..p.num_tiles() {
            let (i, j) = p.tile_coords(idx);
            assert_eq!(p.tile_index(i, j), idx);
        }
    }

    #[test]
    fn rectangular_non_pow2_validates() {
        // (m/t, n/t) = (8, 12) -> 96 tiles; not a power of two.
        let p = MatmulParams {
            m: 64,
            k: 64,
            n: 96,
            noise_rank: 8,
            tile: 8,
            spot_checks: 8,
            lambda: 8,
        };
        p.validate().unwrap();
        assert_eq!(p.num_tiles(), 96);
        assert_eq!(p.num_tiles_padded(), 128);
    }

    #[test]
    fn llm_profiles_have_padded_merkle() {
        let p = MatmulParams::GEMMA_4_31B_FFN;
        assert_eq!(p.row_tiles(), 64); // 4096 / 64
        assert_eq!(p.col_tiles(), 336); // 21504 / 64
        assert_eq!(p.num_tiles(), 64 * 336);
        assert!(!p.num_tiles().is_power_of_two());
        assert_eq!(p.num_tiles_padded(), p.num_tiles().next_power_of_two());
    }
}
