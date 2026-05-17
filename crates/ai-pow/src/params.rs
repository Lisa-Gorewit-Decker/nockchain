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
//!
//! # Pearl §4.8 envelope (the Pearl-faithful PROD path, "γ")
//!
//! The Pearl whitepaper §4.8 ("Supported PoW Parameters") *caps* the
//! mining parameters so that **one opened tile's proof always fits a
//! single STARK** — Pearl deliberately never segments (see
//! `crates/ai-pow-zk/M_S2_PEARL_EVALUATION.md`). We adopt that
//! envelope here, split into two layers:
//!
//! * [`MatmulParams::validate`] enforces the **universal** Pearl §4.8
//!   trace bound `k·(h+w) ≤ 2²²` (the verifier's restriction; with
//!   square tiles `h = w = tile`, this is `k·2·tile ≤ 2²²`). This is
//!   the *one-tile-one-STARK* guarantee and holds for **every**
//!   accepted puzzle, test or production — it is why no segmentation
//!   (G3) is needed within the envelope. Called by `verifier::verify`
//!   and `prover` already.
//! * [`MatmulParams::validate_prod_envelope`] additionally enforces
//!   the §4.8 **security** caps (`16r ≤ k ≤ 4r²`, `r ∈ {2⁵..2¹⁰}`,
//!   `64 | k`, `m,n ≤ 2²⁴`, `h·w ≥ 32`). This is the **consensus
//!   admission rule** — a real protocol puzzle MUST satisfy it.
//!   Small in-crate test profiles (e.g. [`MatmulParams::TEST_SMALL`],
//!   `r = 4`) are intentionally *below* this envelope: they exercise
//!   the circuit machinery fast and are **not** consensus-valid by
//!   design. The future consensus/block-admission layer (M-C1) calls
//!   `validate_prod_envelope`.

use thiserror::Error;

/// Pearl §4.8 verifier trace restriction `k·(h+w) ≤ 2²²`. With
/// square `tile×tile` tiles `h = w = tile`. This bounds the Layer-0
/// trace so one opened tile always proves in a single STARK
/// (Pearl-faithful — no segmentation).
pub const PEARL_TRACE_BOUND: u64 = 1 << 22;
/// Pearl §4.8 common-dimension cap `k ≤ 2¹⁶`.
pub const PEARL_K_MAX: u32 = 1 << 16;
/// Pearl §4.8 noise-rank range `r ∈ {2⁵, …, 2¹⁰}` (32 ≤ r ≤ 1024).
pub const PEARL_R_MIN: u32 = 1 << 5;
pub const PEARL_R_MAX: u32 = 1 << 10;
/// Pearl §4.8 matrix-dimension cap `m, n ≤ 2²⁴`.
pub const PEARL_MN_MAX: u32 = 1 << 24;
/// Pearl §4.8 entropy floor `h·w ≥ 32` (sufficient entropy in `M`).
pub const PEARL_HW_MIN: u64 = 32;

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
    /// Noise rank `r`. Also the accumulator stripe width. The lenient
    /// [`validate`](Self::validate) requires `2 <= r <= k`, `r | k`,
    /// and `r` a power of two (so small test profiles stay valid).
    /// The Pearl §4.8 security band (`r ∈ {2⁵..2¹⁰}`,
    /// `16r <= k <= 4r²`) is enforced by
    /// [`validate_prod_envelope`](Self::validate_prod_envelope) — the
    /// consensus admission rule.
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

    /// **Real shipped Pearl-certified model** —
    /// `pearl-ai/Llama-3.1-8B-Instruct-pearl` (run via the Pearl
    /// vLLM mining plugin; `~/Dev/Llama-3.1-8B-Instruct-pearl`).
    /// `hidden_size = 4096`, `intermediate_size = 14336`, 32
    /// layers; `quant_method = "pearl"` (group_1 weights **7-bit
    /// int**, activations 7-bit per-token ⇒ Pearl §4.1's
    /// `[−64,64]` int regime). These are the two binding FFN
    /// GEMMs the miner actually proves (the largest committed
    /// weights, ≈58.7M params = 57 344 BLAKE3 chunks each — far
    /// past any synthetic preset; this is the real P-B.2
    /// motivation). Both satisfy [`validate_prod_envelope`] with
    /// `r = 64`. (`q/o_proj` k=n=4096, `kv_proj` n=1024 are
    /// strictly smaller and also in-envelope.)
    pub const LLAMA_3_1_8B_GATE_UP: Self = Self {
        m: 4096,
        k: 4096, // hidden_size
        n: 14336, // intermediate_size
        noise_rank: 64,
        tile: 64,
        spot_checks: 80,
        difficulty_bits: 0,
    };

    /// `Llama-3.1-8B-Instruct-pearl` `down_proj`: the **largest
    /// `k`** mineable GEMM (`k = intermediate_size = 14336`) — the
    /// binding Pearl §4.8 case for this model (`16r ≤ k ≤ 4r²`
    /// with `r = 64` ⇒ `1024 ≤ 14336 ≤ 16384` ✓; `64 | 14336` ✓;
    /// `k·2·tile = 1 835 008 ≤ 2²²` ✓).
    pub const LLAMA_3_1_8B_DOWN: Self = Self {
        m: 4096,
        k: 14336, // intermediate_size
        n: 4096, // hidden_size
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
        if self.k == 0 || self.k > PEARL_K_MAX {
            return Err(ParamError::KOutOfRange);
        }
        // Pearl §4.8 **universal trace bound** `k·(h+w) ≤ 2²²`
        // (square tiles ⇒ `h = w = tile`). This is THE
        // one-tile-one-STARK guarantee: it bounds the Layer-0 trace
        // so a single opened tile always proves in one STARK — the
        // Pearl-faithful reason segmentation (G3) is unnecessary.
        // Holds for every accepted puzzle (test and production); the
        // §4.8 *security* caps are layered on in
        // `validate_prod_envelope`. See `M_S2_PEARL_EVALUATION.md`.
        if self.pearl_trace_bound() > PEARL_TRACE_BOUND {
            return Err(ParamError::TraceBoundExceeded);
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
        // Pearl's permutation generator uses `rank_mask = r - 1` as a bitmask;
        // this is only well-formed for `r` a power of two
        // (`pearl/zk-pow/src/circuit/pearl_noise.rs:107`).
        if !self.noise_rank.is_power_of_two() {
            return Err(ParamError::NoiseRankNotPowerOfTwo);
        }
        if self.spot_checks == 0 {
            return Err(ParamError::ZeroSpotChecks);
        }
        if (self.spot_checks as u64) > total {
            return Err(ParamError::TooManySpotChecks);
        }
        Ok(())
    }

    /// Pearl §4.8 trace proxy `k·(h+w)`. Square tiles ⇒ `h = w =
    /// tile`, so this is `k·2·tile`. The Pearl whitepaper's verifier
    /// restricts this to `≤ 2²²` ([`PEARL_TRACE_BOUND`]); within that
    /// bound one opened tile always proves in a single STARK.
    pub fn pearl_trace_bound(&self) -> u64 {
        (self.k as u64) * (self.tile as u64 + self.tile as u64)
    }

    /// Pearl §4.8 **consensus admission rule** — the full Supported
    /// PoW Parameters envelope. A real protocol puzzle MUST satisfy
    /// this; in-crate sub-envelope test profiles
    /// ([`MatmulParams::TEST_SMALL`]) intentionally do not (they use
    /// the lenient [`validate`](Self::validate) for fast circuit
    /// tests and are not consensus-valid by design).
    ///
    /// Enforces, on top of [`validate`](Self::validate) (which
    /// already covers `k ≤ 2¹⁶`, `r | k`, `r` a power of two `≥ 2`,
    /// and the universal `k·(h+w) ≤ 2²²` trace bound):
    /// * `m, n ≤ 2²⁴`
    /// * `r ∈ {2⁵, …, 2¹⁰}` (32 ≤ r ≤ 1024)
    /// * `16r ≤ k ≤ 4r²` (the §4.8 security band)
    /// * `64 | k` (commitment-hash alignment)
    /// * `h·w ≥ 32` (entropy in `M`; square tiles ⇒ `tile² ≥ 32`)
    ///
    /// Within this envelope Pearl proves one opened tile in a single
    /// STARK — which is exactly why the Pearl-faithful PROD path
    /// needs no segmentation (`M_S2_PEARL_EVALUATION.md`).
    pub fn validate_prod_envelope(&self) -> Result<(), ParamError> {
        self.validate()?;
        if self.m > PEARL_MN_MAX || self.n > PEARL_MN_MAX {
            return Err(ParamError::MatrixDimTooLarge);
        }
        if self.noise_rank < PEARL_R_MIN || self.noise_rank > PEARL_R_MAX {
            return Err(ParamError::NoiseRankOutOfEnvelope);
        }
        // 16r ≤ k ≤ 4r² (u64 throughout: r ≤ 1024 ⇒ 4r² ≤ 2²²).
        let r = self.noise_rank as u64;
        let k = self.k as u64;
        if k < 16 * r || k > 4 * r * r {
            return Err(ParamError::KOutOfSecurityBand);
        }
        if self.k % 64 != 0 {
            return Err(ParamError::KNotAlignedTo64);
        }
        // h·w ≥ 32 with square tiles (h = w = tile).
        if (self.tile as u64) * (self.tile as u64) < PEARL_HW_MIN {
            return Err(ParamError::TileEntropyTooLow);
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
    #[error("k·(h+w) must be <= 2^22 (Pearl §4.8 universal trace bound — one-tile-one-STARK)")]
    TraceBoundExceeded,
    #[error("m and n must be <= 2^24 (Pearl §4.8 envelope)")]
    MatrixDimTooLarge,
    #[error("noise_rank must be in {{2^5..=2^10}} = 32..=1024 (Pearl §4.8 envelope)")]
    NoiseRankOutOfEnvelope,
    #[error("k must satisfy 16r <= k <= 4r^2 (Pearl §4.8 security band)")]
    KOutOfSecurityBand,
    #[error("k must be a multiple of 64 (Pearl §4.8 commitment-hash alignment)")]
    KNotAlignedTo64,
    #[error("tile^2 (= h·w) must be >= 32 (Pearl §4.8 entropy floor for M)")]
    TileEntropyTooLow,
    #[error("noise_rank must be in 1..=k")]
    NoiseRankOutOfRange,
    #[error("noise_rank must divide k")]
    NoiseRankDoesNotDivideK,
    #[error("noise_rank must be >= 2 (Pearl §4.4 ChoiceMatrix requires two distinct positions)")]
    NoiseRankTooSmall,
    #[error("noise_rank must be a power of two (Pearl permutation bitmask requirement)")]
    NoiseRankNotPowerOfTwo,
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

    // ─────────────── P-A: Pearl §4.8 envelope (γ path) ───────────────

    /// Every production / LLM preset satisfies the full Pearl §4.8
    /// consensus envelope.
    #[test]
    fn prod_presets_satisfy_envelope() {
        for p in [
            MatmulParams::PROD,
            MatmulParams::GEMMA_4_31B_FFN,
            MatmulParams::QWEN_3_6_27B_FFN,
            MatmulParams::llm_ffn(4096, 11008, 4096),
            // Real shipped Pearl-certified model (the production
            // target, not a synthetic guess).
            MatmulParams::LLAMA_3_1_8B_GATE_UP,
            MatmulParams::LLAMA_3_1_8B_DOWN,
        ] {
            p.validate_prod_envelope()
                .unwrap_or_else(|e| panic!("{p:?} not in §4.8 envelope: {e}"));
            // …and therefore trivially within the one-STARK bound.
            assert!(p.pearl_trace_bound() <= PEARL_TRACE_BOUND);
        }
    }

    /// TEST_SMALL is intentionally a *sub-envelope* circuit-test
    /// profile: it passes the lenient structural `validate()` (and
    /// the universal trace bound) but is NOT consensus-valid (`r =
    /// 4 < 32`). This split is by design (see module docs).
    #[test]
    fn test_small_is_below_consensus_envelope() {
        MatmulParams::TEST_SMALL.validate().unwrap();
        assert!(MatmulParams::TEST_SMALL.pearl_trace_bound() <= PEARL_TRACE_BOUND);
        assert_eq!(
            MatmulParams::TEST_SMALL.validate_prod_envelope(),
            Err(ParamError::NoiseRankOutOfEnvelope),
        );
    }

    /// The universal `k·(h+w) ≤ 2²²` trace bound is enforced by the
    /// plain `validate()` (so `verifier::verify`/`prover` already
    /// reject un-provable puzzles) — the one-tile-one-STARK
    /// guarantee, holding for test and prod alike.
    #[test]
    fn universal_trace_bound_enforced_by_validate() {
        // k = 2^16, tile = 64 ⇒ k·2·tile = 2^23 > 2^22.
        let p = MatmulParams {
            m: 64,
            k: 1 << 16,
            n: 64,
            noise_rank: 64,
            tile: 64,
            spot_checks: 1,
            difficulty_bits: 0,
        };
        assert_eq!(p.pearl_trace_bound(), 1 << 23);
        assert_eq!(p.validate(), Err(ParamError::TraceBoundExceeded));
        // Halving the tile brings it back to exactly the bound.
        let ok = MatmulParams { tile: 32, m: 64, n: 64, ..p };
        assert_eq!(ok.pearl_trace_bound(), PEARL_TRACE_BOUND);
        ok.validate().unwrap();
    }

    /// **Envelope ⇒ one-STARK theorem.** Any params accepted by
    /// `validate_prod_envelope` necessarily satisfies the universal
    /// `k·(h+w) ≤ 2²²` bound (it is checked inside the `validate`
    /// the envelope delegates to). Swept over the whole §4.8
    /// security band, the strongest in-envelope load still proves in
    /// one STARK — which is exactly why the Pearl-faithful path
    /// needs no segmentation.
    #[test]
    fn envelope_implies_one_stark_bound() {
        let mut checked = 0u32;
        for r_log in 5..=10u32 {
            let r = 1u32 << r_log; // 32..=1024
            for &kf in &[16u64, 32, 64, 256, 1024] {
                let k64 = kf * r as u64;
                if k64 == 0 || k64 > PEARL_K_MAX as u64 || k64 > 4 * (r as u64) * (r as u64) {
                    continue;
                }
                let k = k64 as u32;
                if k % 64 != 0 || k % r != 0 {
                    continue;
                }
                // Largest tile that still respects the trace bound,
                // rounded to divide m=n; tile≥6 for the entropy floor.
                for &tile in &[8u32, 16, 32, 64, 128] {
                    let p = MatmulParams {
                        m: tile * 4,
                        k,
                        n: tile * 4,
                        noise_rank: r,
                        tile,
                        spot_checks: 1,
                        difficulty_bits: 0,
                    };
                    match p.validate_prod_envelope() {
                        Ok(()) => {
                            assert!(
                                p.pearl_trace_bound() <= PEARL_TRACE_BOUND,
                                "{p:?} in envelope but trace {} > 2^22",
                                p.pearl_trace_bound()
                            );
                            checked += 1;
                        }
                        // Out-of-envelope combos are fine to skip; the
                        // theorem is only about the Ok arm.
                        Err(_) => {}
                    }
                }
            }
        }
        assert!(checked > 0, "swept no in-envelope params — sweep bug");
    }

    /// Each §4.8 *security* cap rejects with its specific error
    /// (built by perturbing the known-good PROD preset minimally).
    #[test]
    fn envelope_rejects_each_security_violation() {
        // m too large (still tile-aligned: 2^24 % 128 == 0).
        let p = MatmulParams { m: (1 << 24) + 128, ..MatmulParams::PROD };
        assert_eq!(p.validate_prod_envelope(), Err(ParamError::MatrixDimTooLarge));

        // r below the {2^5..2^10} band (16 | 4096, pow2, but < 32).
        let p = MatmulParams { noise_rank: 16, ..MatmulParams::PROD };
        assert_eq!(
            p.validate_prod_envelope(),
            Err(ParamError::NoiseRankOutOfEnvelope)
        );

        // k outside the 16r..=4r² band (r=64 ⇒ band [1024, 16384];
        // k=512 is below it; 512%64==0, 64|512, trace ok).
        let p = MatmulParams { k: 512, ..MatmulParams::PROD };
        assert_eq!(p.validate_prod_envelope(), Err(ParamError::KOutOfSecurityBand));

        // k not aligned to 64 (r=32, k=544: 32|544, in band [512,4096],
        // 544%64==32≠0).
        let p = MatmulParams { noise_rank: 32, k: 544, ..MatmulParams::PROD };
        assert_eq!(p.validate_prod_envelope(), Err(ParamError::KNotAlignedTo64));

        // tile entropy floor: tile²<32 (tile=4 ⇒ 16<32; m,n%4==0,
        // k=512,r=32 in band, 512%64==0).
        let p = MatmulParams {
            tile: 4,
            m: 512,
            n: 512,
            k: 512,
            noise_rank: 32,
            ..MatmulParams::PROD
        };
        assert_eq!(p.validate_prod_envelope(), Err(ParamError::TileEntropyTooLow));
    }

    /// `validate_prod_envelope` is strictly stronger than
    /// `validate`: anything it accepts, `validate` accepts.
    #[test]
    fn envelope_implies_validate() {
        for p in [
            MatmulParams::PROD,
            MatmulParams::GEMMA_4_31B_FFN,
            MatmulParams::QWEN_3_6_27B_FFN,
        ] {
            assert!(p.validate_prod_envelope().is_ok());
            assert!(p.validate().is_ok());
        }
    }
}
