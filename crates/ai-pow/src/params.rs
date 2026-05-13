//! Puzzle parameters.
//!
//! Matches Pearl Whitepaper §4.1 (mining configuration) for the in-crate
//! synthetic-`A,B` setting: the protocol multiplies `(A + E) * (B + F)`
//! tile-by-tile, with `E = E_L · E_R` and `F = F_L · F_R` of rank
//! `noise_rank = r` (Pearl §4.4 Alg. 3). The noise rank is also the
//! accumulator stripe width for the Pearl iterative tile-state update
//! (Pearl §4.5 Alg. 4), so `r | k` is required.
//!
//! Difficulty is expressed in log-bits via `difficulty_bits = b` so the
//! hardness condition matches Pearl §4.5:
//!   BLAKE3(M, key = s_a)  <=  2^(256 - b) * r * t_m * t_n
//! (with square tiles, `t_m = t_n = tile`).

use thiserror::Error;

/// Parameters of a Pearl-style matmul PoW puzzle.
///
/// Matmul shape is `(m, k) * (k, n) = (m, n)`. Tiles are square `tile x tile`.
/// `noise_rank` is the rank `r` of the low-rank noise factors **and** the
/// inner-accumulator stripe width. `difficulty_bits` is Pearl's `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatmulParams {
    pub m: u32,
    pub k: u32,
    pub n: u32,
    /// Noise rank `r`. Also the accumulator stripe width. Must satisfy
    /// `1 <= r <= k` and `r | k`. Pearl §4.8 recommends `16r <= k <= 4r^2`
    /// for security; this crate does **not** hard-enforce that bound so that
    /// small test profiles stay valid.
    pub noise_rank: u32,
    pub tile: u32,
    pub spot_checks: u32,
    /// Logarithmic difficulty `b` (Pearl §4.5). A tile is accepted when
    /// `BLAKE3(M, key = s_a) <= 2^(256 - b) * r * t^2`. `b = 0` accepts
    /// every tile; values above 256 reject everything.
    pub difficulty_bits: u32,
}

impl MatmulParams {
    /// Default test profile — small enough to run end-to-end in milliseconds.
    /// Picks `r = 4` so `16r = 64 = k` (Pearl-recommended lower bound).
    pub const TEST_SMALL: Self = Self {
        m: 64,
        k: 64,
        n: 64,
        noise_rank: 4,
        tile: 8,
        spot_checks: 8,
        difficulty_bits: 0,
    };

    /// Production profile: 4096^3 INT8 matmul, 128-tile, 80 spot checks.
    /// `r = 64` so `16r = 1024 <= k = 4096 <= 4r^2 = 16384` (Pearl §4.8 OK).
    pub const PROD: Self = Self {
        m: 4096,
        k: 4096,
        n: 4096,
        noise_rank: 64,
        tile: 128,
        spot_checks: 80,
        difficulty_bits: 0,
    };

    /// Gemma 4 31B FFN gate / up matmul: `(B=4096, hidden=5376, intermediate=21504)`.
    /// `r = 64` works because `64 | 5376`.
    pub const GEMMA_4_31B_FFN: Self = Self {
        m: 4096,
        k: 5376,
        n: 21504,
        noise_rank: 64,
        tile: 64,
        spot_checks: 80,
        difficulty_bits: 0,
    };

    /// Qwen 3.6 27B FFN gate / up matmul: `(B=4096, hidden=5120, intermediate=17408)`.
    pub const QWEN_3_6_27B_FFN: Self = Self {
        m: 4096,
        k: 5120,
        n: 17408,
        noise_rank: 64,
        tile: 64,
        spot_checks: 80,
        difficulty_bits: 0,
    };

    /// Generic LLM-FFN profile builder. `batch_seq` is the M dimension (the
    /// product of mini-batch and sequence length the GEMM kernel sees);
    /// `hidden` and `intermediate` are the two model dimensions for the FFN
    /// gate / up matmul. Picks `tile = 64`, `r = 64`, `sigma = 80`.
    pub const fn llm_ffn(hidden: u32, intermediate: u32, batch_seq: u32) -> Self {
        Self {
            m: batch_seq,
            k: hidden,
            n: intermediate,
            noise_rank: 64,
            tile: 64,
            spot_checks: 80,
            difficulty_bits: 0,
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
        // Pearl §4.8 caps k at 2^16. With `|A| <= 64`, `|E| <= 63`, the per-multiply
        // bound is (64+63)^2 = 16129 < 2^14, so `k * 16129 < 2^31` holds well past
        // Pearl's cap. Use Pearl's cap directly.
        if self.k == 0 || self.k > (1u32 << 16) {
            return Err(ParamError::KOutOfRange);
        }
        // Noise rank requirements: 1 <= r <= k and r | k.
        if self.noise_rank == 0 || self.noise_rank > self.k {
            return Err(ParamError::NoiseRankOutOfRange);
        }
        if self.k % self.noise_rank != 0 {
            return Err(ParamError::NoiseRankDoesNotDivideK);
        }
        // Pearl §4.4: each column of E_R has one +1 and one -1 at two
        // *distinct* positions; requires r >= 2.
        if self.noise_rank < 2 {
            return Err(ParamError::NoiseRankTooSmall);
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
    /// Number of accumulator stripes per tile (`⌊k / r⌋`). Each stripe folds
    /// one update into the 512-bit `M` state.
    pub fn num_stripes(&self) -> u32 {
        self.k / self.noise_rank
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
    #[error("k must be in 1..=2^16 (Pearl §4.8)")]
    KOutOfRange,
    #[error("noise_rank must be in 1..=k")]
    NoiseRankOutOfRange,
    #[error("noise_rank must divide k")]
    NoiseRankDoesNotDivideK,
    #[error("noise_rank must be >= 2 (Pearl §4.4 ChoiceMatrix requires two distinct positions)")]
    NoiseRankTooSmall,
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
        p.k = (1 << 16) + 1;
        assert_eq!(p.validate(), Err(ParamError::KOutOfRange));

        p = MatmulParams::TEST_SMALL;
        p.spot_checks = 0;
        assert_eq!(p.validate(), Err(ParamError::ZeroSpotChecks));

        p = MatmulParams::TEST_SMALL;
        p.spot_checks = p.num_tiles() + 1;
        assert_eq!(p.validate(), Err(ParamError::TooManySpotChecks));

        // Noise rank must divide k.
        p = MatmulParams::TEST_SMALL;
        p.noise_rank = 5; // 5 does not divide 64
        assert_eq!(p.validate(), Err(ParamError::NoiseRankDoesNotDivideK));

        // Noise rank cannot be 1 (ChoiceMatrix needs two distinct positions).
        p = MatmulParams::TEST_SMALL;
        p.noise_rank = 1;
        assert_eq!(p.validate(), Err(ParamError::NoiseRankTooSmall));
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
        // r = 4 divides k = 64.
        let p = MatmulParams {
            m: 64,
            k: 64,
            n: 96,
            noise_rank: 4,
            tile: 8,
            spot_checks: 8,
            difficulty_bits: 0,
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

    #[test]
    fn num_stripes_matches_k_over_r() {
        let p = MatmulParams::TEST_SMALL;
        assert_eq!(p.num_stripes(), p.k / p.noise_rank);
        assert_eq!(MatmulParams::PROD.num_stripes(), 4096 / 64);
    }
}
