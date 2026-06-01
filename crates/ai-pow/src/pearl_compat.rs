//! Pearl merge-mining compatibility primitives.
//!
//! This module is intentionally separate from the native Nockchain AI-PoW
//! transcript. Pearl-compatible merge mining means sharing the same Pearl work
//! attempt and jackpot digest, not sharing proof bytes. The canonical attempt
//! transcript here is:
//!
//! ```text
//! kappa = BLAKE3(sigma || mu)
//! H_A   = BLAKE3(pad(A_row_major), key=kappa)
//! H_B   = BLAKE3(pad(B_col_major), key=kappa)
//! s_B   = BLAKE3(kappa || H_B)
//! s_A   = BLAKE3(s_B || H_A)
//! hash  = BLAKE3(M_i_j, key=s_A)
//! ```
//!
//! Nockchain-native proof systems may prove this statement with their own
//! recursive certificate format, but they must not change these public work
//! bytes in Pearl-compatible mode.

use blake3::Hasher;
use thiserror::Error;

use crate::commit::matrix_commitment;
use crate::fiat_shamir::{noise_seed_a, noise_seed_b};
use crate::matmul::{compute_tile, BlockNoise, Matrices, TileState};
use crate::params::{MatmulParams, ParamError};
use crate::prng;
use crate::tile_hash::hash_le_target;

const INPUT_RANGE_MAX: i8 = 64;

pub const PEARL_INCOMPLETE_BLOCK_HEADER_SIZE: usize = 76;
pub const PEARL_MINING_CONFIG_SIZE: usize = 52;
pub const PEARL_MINING_CONFIG_RESERVED_SIZE: usize = 32;
pub const PEARL_MMA_INT7XINT7_TO_INT32: u16 = 0;
pub const PEARL_PUBLIC_PROOF_PARAMS_SIZE: usize = 164;
pub const PEARL_TILE_D: u32 = 16;
pub const PEARL_TILE_H: u32 = 2;
pub const PEARL_DWORD_SIZE: u32 = 8;
pub const PEARL_WORKER_INPUT_MAX: u64 = 1 << 22;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PearlCompatError {
    #[error("invalid params: {0}")]
    Params(#[from] ParamError),
    #[error("Pearl encoded header has wrong length: expected 76, got {0}")]
    BadHeaderLen(usize),
    #[error("Pearl encoded mining config has wrong length: expected 52, got {0}")]
    BadMiningConfigLen(usize),
    #[error("Pearl encoded periodic pattern has wrong length: expected 6, got {0}")]
    BadPatternLen(usize),
    #[error("Pearl encoded public proof params have wrong length: expected 164, got {0}")]
    BadPublicParamsLen(usize),
    #[error("unsupported Pearl MMA type: {0}")]
    UnsupportedMmaType(u16),
    #[error("Pearl mining config common_dim does not match params.k")]
    CommonDimMismatch,
    #[error("Pearl mining config rank does not match params.noise_rank")]
    RankMismatch,
    #[error("Pearl mining config reserved field must be all zero")]
    NonzeroReserved,
    #[error("Pearl periodic pattern has non-canonical trailing dimension")]
    NonCanonicalPattern,
    #[error("Pearl periodic pattern must not break a single stride across dimensions")]
    BrokenSingleStride,
    #[error("Pearl periodic pattern stride must be a positive multiple of prior period")]
    BadPatternStride,
    #[error("Pearl periodic pattern factor or length does not fit one byte")]
    PatternByteOverflow,
    #[error("Pearl periodic pattern period exceeds 2^24")]
    PatternPeriodTooLarge,
    #[error("Pearl periodic pattern period must divide the matrix dimension")]
    PatternPeriodDoesNotDivideDimension,
    #[error("Pearl periodic pattern is empty")]
    PatternEmpty,
    #[error("Pearl periodic pattern must be sorted, unique, and strictly increasing")]
    PatternNotStrictlyIncreasing,
    #[error("Pearl periodic pattern must start at zero")]
    PatternMustStartAtZero,
    #[error("Pearl periodic pattern is not representable as three Pearl dimensions")]
    PatternNotRepresentable,
    #[error("Pearl periodic pattern list would exceed caller limit")]
    PatternListTooLarge,
    #[error("Pearl public proof params have an invalid row or column pattern offset")]
    InvalidPatternOffset,
    #[error("Pearl public proof params place the row or column pattern outside the matrix")]
    PatternOutOfMatrix,
    #[error("Pearl public proof params violate the production parameter envelope")]
    PublicParamEnvelope,
    #[error("Pearl public proof commitments do not match the derived work commitments")]
    PublicCommitmentMismatch,
    #[error("Pearl public proof jackpot hash does not match the recomputed pattern ticket")]
    JackpotHashMismatch,
    #[error("Pearl jackpot hash does not satisfy Pearl nbits target")]
    PearlTargetNotMet,
    #[error("Pearl jackpot hash does not satisfy Nockchain target")]
    NockchainTargetNotMet,
    #[error("A has wrong length: expected m*k = {expected}, got {actual}")]
    InputAShape { expected: usize, actual: usize },
    #[error("B has wrong length: expected n*k = {expected}, got {actual}")]
    InputBShape { expected: usize, actual: usize },
    #[error("input entry out of range [-64, 64]: matrix={matrix}, index={index}, value={value}")]
    InputOutOfRange {
        matrix: &'static str,
        index: usize,
        value: i8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PearlIncompleteBlockHeader {
    pub version: u32,
    pub prev_block: [u8; 32],
    pub merkle_root: [u8; 32],
    pub timestamp: u32,
    pub nbits: u32,
}

impl PearlIncompleteBlockHeader {
    pub fn to_bytes(&self) -> [u8; PEARL_INCOMPLETE_BLOCK_HEADER_SIZE] {
        let mut out = [0u8; PEARL_INCOMPLETE_BLOCK_HEADER_SIZE];
        out[0..4].copy_from_slice(&self.version.to_le_bytes());
        for (dst, src) in out[4..36].iter_mut().zip(self.prev_block.iter().rev()) {
            *dst = *src;
        }
        for (dst, src) in out[36..68].iter_mut().zip(self.merkle_root.iter().rev()) {
            *dst = *src;
        }
        out[68..72].copy_from_slice(&self.timestamp.to_le_bytes());
        out[72..76].copy_from_slice(&self.nbits.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PearlCompatError> {
        if bytes.len() != PEARL_INCOMPLETE_BLOCK_HEADER_SIZE {
            return Err(PearlCompatError::BadHeaderLen(bytes.len()));
        }
        let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let mut prev_block: [u8; 32] = bytes[4..36].try_into().unwrap();
        prev_block.reverse();
        let mut merkle_root: [u8; 32] = bytes[36..68].try_into().unwrap();
        merkle_root.reverse();
        let timestamp = u32::from_le_bytes(bytes[68..72].try_into().unwrap());
        let nbits = u32::from_le_bytes(bytes[72..76].try_into().unwrap());
        Ok(Self {
            version,
            prev_block,
            merkle_root,
            timestamp,
            nbits,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PearlPeriodicPattern {
    pub shape: [(u32, u32); 3],
}

impl PearlPeriodicPattern {
    pub const NUM_DIMS: usize = 3;
    pub const MAX_PERIOD: u32 = 1 << 24;

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PearlCompatError> {
        if bytes.len() != 2 * Self::NUM_DIMS {
            return Err(PearlCompatError::BadPatternLen(bytes.len()));
        }
        let mut shape = [(1u32, 1u32); Self::NUM_DIMS];
        let mut min_stride = 1u32;
        let mut done = false;
        for (idx, chunk) in bytes.chunks_exact(2).enumerate() {
            let factor = 1 + u32::from(chunk[0]);
            let length = 1 + u32::from(chunk[1]);
            if length == 1 || done {
                if factor != 1 || length != 1 {
                    return Err(PearlCompatError::NonCanonicalPattern);
                }
                done = true;
            } else if factor <= 1 && min_stride != 1 {
                return Err(PearlCompatError::BrokenSingleStride);
            }
            let Some(period) = min_stride
                .checked_mul(factor)
                .and_then(|s| s.checked_mul(length))
            else {
                return Err(PearlCompatError::PatternPeriodTooLarge);
            };
            if period > Self::MAX_PERIOD {
                return Err(PearlCompatError::PatternPeriodTooLarge);
            }
            let stride = factor * min_stride;
            shape[idx] = (stride, length);
            min_stride = period;
        }
        Ok(Self { shape })
    }

    pub fn to_bytes(&self) -> Result<[u8; 2 * Self::NUM_DIMS], PearlCompatError> {
        let mut out = [0u8; 2 * Self::NUM_DIMS];
        let mut min_stride = 1u32;
        let mut done = false;
        for (idx, &(stride, length)) in self.shape.iter().enumerate() {
            if stride == 0 || length == 0 || stride % min_stride != 0 {
                return Err(PearlCompatError::BadPatternStride);
            }
            let factor = stride / min_stride;
            if length == 1 || done {
                if factor != 1 || length != 1 {
                    return Err(PearlCompatError::NonCanonicalPattern);
                }
                done = true;
            } else if factor <= 1 && min_stride != 1 {
                return Err(PearlCompatError::BrokenSingleStride);
            }
            if factor > 256 || length > 256 {
                return Err(PearlCompatError::PatternByteOverflow);
            }
            let Some(period) = stride.checked_mul(length) else {
                return Err(PearlCompatError::PatternPeriodTooLarge);
            };
            if period > Self::MAX_PERIOD {
                return Err(PearlCompatError::PatternPeriodTooLarge);
            }
            out[2 * idx] = (factor - 1) as u8;
            out[2 * idx + 1] = (length - 1) as u8;
            min_stride = period;
        }
        Ok(out)
    }

    pub fn from_list(indices: &[u32]) -> Result<Self, PearlCompatError> {
        if indices.is_empty() {
            return Err(PearlCompatError::PatternEmpty);
        }
        if !indices.windows(2).all(|w| w[0] < w[1]) {
            return Err(PearlCompatError::PatternNotStrictlyIncreasing);
        }
        if indices[0] != 0 {
            return Err(PearlCompatError::PatternMustStartAtZero);
        }

        let mut pattern = indices.to_vec();
        let mut shape = Vec::new();

        while pattern.len() > 1 {
            let mut found = false;
            for period in 1..pattern.len() {
                if pattern.len() % period != 0 {
                    continue;
                }
                let stride = pattern[period];
                let is_periodic =
                    (0..pattern.len() - period).all(|i| pattern[i] + stride == pattern[i + period]);
                if is_periodic {
                    shape.push((stride, (pattern.len() / period) as u32));
                    pattern.truncate(period);
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(PearlCompatError::PatternNotRepresentable);
            }
            if shape.len() > Self::NUM_DIMS {
                return Err(PearlCompatError::PatternNotRepresentable);
            }
        }

        shape.reverse();
        let period = match shape.last() {
            Some(&(stride, length)) => stride
                .checked_mul(length)
                .ok_or(PearlCompatError::PatternPeriodTooLarge)?,
            None => 1,
        };
        while shape.len() < Self::NUM_DIMS {
            shape.push((period, 1));
        }
        let shape: [(u32, u32); Self::NUM_DIMS] = shape
            .try_into()
            .map_err(|_| PearlCompatError::PatternNotRepresentable)?;
        let pattern = Self { shape };
        if !pattern.is_valid() {
            return Err(PearlCompatError::PatternNotRepresentable);
        }
        Ok(pattern)
    }

    pub fn to_list_bounded(&self, max_len: usize) -> Result<Vec<u32>, PearlCompatError> {
        let size = self.checked_size()?;
        if size > max_len {
            return Err(PearlCompatError::PatternListTooLarge);
        }
        let mut result = vec![0u32];
        for &(stride, length) in &self.shape {
            let next_len = result
                .len()
                .checked_mul(length as usize)
                .ok_or(PearlCompatError::PatternListTooLarge)?;
            if next_len > max_len {
                return Err(PearlCompatError::PatternListTooLarge);
            }
            let mut next = Vec::with_capacity(next_len);
            for i in 0..length {
                for &base in &result {
                    next.push(base + i * stride);
                }
            }
            result = next;
        }
        Ok(result)
    }

    pub fn to_list(&self) -> Result<Vec<u32>, PearlCompatError> {
        self.to_list_bounded(self.checked_size()?)
    }

    pub fn max(&self) -> Result<u32, PearlCompatError> {
        Ok(self.to_list()?.into_iter().max().unwrap_or(0))
    }

    pub fn offset_is_valid(&self, mut offset: u32) -> bool {
        for &(stride, length) in self.shape.iter().rev() {
            let Some(period) = stride.checked_mul(length) else {
                return false;
            };
            if period == 0 {
                return false;
            }
            offset %= period;
            if offset >= stride {
                return false;
            }
        }
        true
    }

    pub fn is_valid(&self) -> bool {
        self.to_bytes()
            .and_then(|bytes| Self::from_bytes(&bytes))
            .is_ok_and(|restored| restored == *self)
    }

    pub fn period(&self) -> Result<u32, PearlCompatError> {
        let (stride, length) = self.shape[Self::NUM_DIMS - 1];
        stride
            .checked_mul(length)
            .ok_or(PearlCompatError::PatternPeriodTooLarge)
    }

    pub fn size(&self) -> Result<u32, PearlCompatError> {
        let size = self.checked_size()?;
        u32::try_from(size).map_err(|_| PearlCompatError::PatternListTooLarge)
    }

    fn checked_size(&self) -> Result<usize, PearlCompatError> {
        self.shape.iter().try_fold(1usize, |acc, &(_, length)| {
            acc.checked_mul(length as usize)
                .ok_or(PearlCompatError::PatternListTooLarge)
        })
    }

    pub fn indices_with_offset_bounded(
        &self,
        offset: u32,
        max_len: usize,
    ) -> Result<Vec<u32>, PearlCompatError> {
        let mut indices = self.to_list_bounded(max_len)?;
        for index in &mut indices {
            *index = index
                .checked_add(offset)
                .ok_or(PearlCompatError::PatternPeriodTooLarge)?;
        }
        Ok(indices)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PearlMiningConfig {
    pub common_dim: u32,
    pub rank: u16,
    pub mma_type: u16,
    pub rows_pattern: PearlPeriodicPattern,
    pub cols_pattern: PearlPeriodicPattern,
    pub reserved: [u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
}

impl PearlMiningConfig {
    pub fn to_bytes(&self) -> Result<[u8; PEARL_MINING_CONFIG_SIZE], PearlCompatError> {
        if self.mma_type != PEARL_MMA_INT7XINT7_TO_INT32 {
            return Err(PearlCompatError::UnsupportedMmaType(self.mma_type));
        }
        if self.reserved != [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE] {
            return Err(PearlCompatError::NonzeroReserved);
        }
        let mut out = [0u8; PEARL_MINING_CONFIG_SIZE];
        out[0..4].copy_from_slice(&self.common_dim.to_le_bytes());
        out[4..6].copy_from_slice(&self.rank.to_le_bytes());
        out[6..8].copy_from_slice(&self.mma_type.to_le_bytes());
        out[8..14].copy_from_slice(&self.rows_pattern.to_bytes()?);
        out[14..20].copy_from_slice(&self.cols_pattern.to_bytes()?);
        out[20..52].copy_from_slice(&self.reserved);
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PearlCompatError> {
        if bytes.len() != PEARL_MINING_CONFIG_SIZE {
            return Err(PearlCompatError::BadMiningConfigLen(bytes.len()));
        }
        let common_dim = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let rank = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let mma_type = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        if mma_type != PEARL_MMA_INT7XINT7_TO_INT32 {
            return Err(PearlCompatError::UnsupportedMmaType(mma_type));
        }
        let rows_pattern = PearlPeriodicPattern::from_bytes(&bytes[8..14])?;
        let cols_pattern = PearlPeriodicPattern::from_bytes(&bytes[14..20])?;
        let reserved: [u8; PEARL_MINING_CONFIG_RESERVED_SIZE] = bytes[20..52].try_into().unwrap();
        if reserved != [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE] {
            return Err(PearlCompatError::NonzeroReserved);
        }
        Ok(Self {
            common_dim,
            rank,
            mma_type,
            rows_pattern,
            cols_pattern,
            reserved,
        })
    }

    pub fn dot_product_length(&self) -> Result<usize, PearlCompatError> {
        if self.rank == 0 {
            return Err(PearlCompatError::PublicParamEnvelope);
        }
        let common_dim = self.common_dim as usize;
        let rank = self.rank as usize;
        Ok(common_dim - common_dim % rank)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PearlPublicProofParams {
    pub block_header: PearlIncompleteBlockHeader,
    pub mining_config: PearlMiningConfig,
    pub hash_a: [u8; 32],
    pub hash_b: [u8; 32],
    pub hash_jackpot: [u8; 32],
    pub m: u32,
    pub n: u32,
    pub t_rows: u32,
    pub t_cols: u32,
}

impl PearlPublicProofParams {
    pub fn to_public_data(&self) -> Result<[u8; PEARL_PUBLIC_PROOF_PARAMS_SIZE], PearlCompatError> {
        let mut out = [0u8; PEARL_PUBLIC_PROOF_PARAMS_SIZE];
        out[0..52].copy_from_slice(&self.mining_config.to_bytes()?);
        out[52..84].copy_from_slice(&self.hash_a);
        out[84..116].copy_from_slice(&self.hash_b);
        out[116..148].copy_from_slice(&self.hash_jackpot);
        out[148..152].copy_from_slice(&self.m.to_le_bytes());
        out[152..156].copy_from_slice(&self.n.to_le_bytes());
        out[156..160].copy_from_slice(&self.t_rows.to_le_bytes());
        out[160..164].copy_from_slice(&self.t_cols.to_le_bytes());
        Ok(out)
    }

    pub fn from_public_data(
        block_header: PearlIncompleteBlockHeader,
        bytes: &[u8],
    ) -> Result<Self, PearlCompatError> {
        if bytes.len() != PEARL_PUBLIC_PROOF_PARAMS_SIZE {
            return Err(PearlCompatError::BadPublicParamsLen(bytes.len()));
        }
        let mining_config = PearlMiningConfig::from_bytes(&bytes[0..52])?;
        let hash_a = bytes[52..84].try_into().unwrap();
        let hash_b = bytes[84..116].try_into().unwrap();
        let hash_jackpot = bytes[116..148].try_into().unwrap();
        let m = u32::from_le_bytes(bytes[148..152].try_into().unwrap());
        let n = u32::from_le_bytes(bytes[152..156].try_into().unwrap());
        let t_rows = u32::from_le_bytes(bytes[156..160].try_into().unwrap());
        let t_cols = u32::from_le_bytes(bytes[160..164].try_into().unwrap());

        if !mining_config.rows_pattern.offset_is_valid(t_rows)
            || !mining_config.cols_pattern.offset_is_valid(t_cols)
        {
            return Err(PearlCompatError::InvalidPatternOffset);
        }

        Ok(Self {
            block_header,
            mining_config,
            hash_a,
            hash_b,
            hash_jackpot,
            m,
            n,
            t_rows,
            t_cols,
        })
    }

    pub fn h(&self) -> Result<u32, PearlCompatError> {
        self.mining_config.rows_pattern.size()
    }

    pub fn w(&self) -> Result<u32, PearlCompatError> {
        self.mining_config.cols_pattern.size()
    }

    pub fn a_rows_indices_bounded(&self, max_len: usize) -> Result<Vec<u32>, PearlCompatError> {
        self.mining_config
            .rows_pattern
            .indices_with_offset_bounded(self.t_rows, max_len)
    }

    pub fn b_cols_indices_bounded(&self, max_len: usize) -> Result<Vec<u32>, PearlCompatError> {
        self.mining_config
            .cols_pattern
            .indices_with_offset_bounded(self.t_cols, max_len)
    }

    pub fn row_thread_partitions_bounded(
        &self,
        max_indices_per_partition: usize,
        max_partitions: usize,
    ) -> Result<Vec<Vec<u32>>, PearlCompatError> {
        pattern_partitions_bounded(
            &self.mining_config.rows_pattern, self.m, max_indices_per_partition, max_partitions,
        )
    }

    pub fn col_thread_partitions_bounded(
        &self,
        max_indices_per_partition: usize,
        max_partitions: usize,
    ) -> Result<Vec<Vec<u32>>, PearlCompatError> {
        pattern_partitions_bounded(
            &self.mining_config.cols_pattern, self.n, max_indices_per_partition, max_partitions,
        )
    }

    pub fn sanity_check(&self) -> Result<(), PearlCompatError> {
        let k = self.mining_config.common_dim;
        let r = u32::from(self.mining_config.rank);
        let h = self.h()?;
        let w = self.w()?;
        let dot_product_len = self.mining_config.dot_product_length()? as u64;
        let worker_input_size = u64::from(h.saturating_add(w)).saturating_mul(dot_product_len);

        if !(r.is_power_of_two() && (32..=1024).contains(&r))
            || !r.is_multiple_of(PEARL_TILE_D)
            || k > (1 << 16)
            || !k.is_multiple_of(64)
            || k > 4 * r * r
            || k < 16 * r
            || k < 1024
            || !h.is_multiple_of(PEARL_TILE_H)
            || !w.is_multiple_of(PEARL_TILE_H)
            || u64::from(h) * u64::from(w) < 32
            || dot_product_len % u64::from(PEARL_DWORD_SIZE) != 0
            || self.m > PearlPeriodicPattern::MAX_PERIOD
            || self.n > PearlPeriodicPattern::MAX_PERIOD
            || worker_input_size > PEARL_WORKER_INPUT_MAX
        {
            return Err(PearlCompatError::PublicParamEnvelope);
        }

        let rmax = self.mining_config.rows_pattern.max()?;
        let cmax = self.mining_config.cols_pattern.max()?;
        let Some(row_max) = self.t_rows.checked_add(rmax) else {
            return Err(PearlCompatError::PatternOutOfMatrix);
        };
        let Some(col_max) = self.t_cols.checked_add(cmax) else {
            return Err(PearlCompatError::PatternOutOfMatrix);
        };
        if row_max >= self.m || col_max >= self.n {
            return Err(PearlCompatError::PatternOutOfMatrix);
        }
        Ok(())
    }

    pub fn difficulty_adjustment_factor(&self) -> Result<u128, PearlCompatError> {
        let h = u128::from(self.h()?);
        let w = u128::from(self.w()?);
        let dot = self.mining_config.dot_product_length()? as u128;
        h.checked_mul(w)
            .and_then(|tile| tile.checked_mul(dot))
            .ok_or(PearlCompatError::PublicParamEnvelope)
    }

    pub fn pearl_adjusted_target(&self) -> Result<[u8; 32], PearlCompatError> {
        let base = pearl_nbits_to_target_le(self.block_header.nbits);
        Ok(u256_le_mul_u128_saturating(
            &base,
            self.difficulty_adjustment_factor()?,
        ))
    }

    pub fn check_pearl_jackpot_difficulty(&self) -> Result<(), PearlCompatError> {
        let target = self.pearl_adjusted_target()?;
        if hash_le_target(&self.hash_jackpot, &target) {
            Ok(())
        } else {
            Err(PearlCompatError::PearlTargetNotMet)
        }
    }

    pub fn check_nockchain_jackpot_target(
        &self,
        nockchain_target: &[u8; 32],
    ) -> Result<(), PearlCompatError> {
        if hash_le_target(&self.hash_jackpot, nockchain_target) {
            Ok(())
        } else {
            Err(PearlCompatError::NockchainTargetNotMet)
        }
    }
}

pub fn pearl_nbits_to_target_le(nbits: u32) -> [u8; 32] {
    let exponent = (nbits >> 24) as usize;
    let mantissa = nbits & 0x00ff_ffff;
    if exponent == 0 || mantissa == 0 || (mantissa & 0x0080_0000) != 0 {
        return [0u8; 32];
    }

    let mut out = [0u8; 32];
    if exponent <= 3 {
        let shifted = mantissa >> (8 * (3 - exponent));
        out[0..4].copy_from_slice(&shifted.to_le_bytes());
    } else {
        let offset = exponent - 3;
        let bytes = mantissa.to_le_bytes();
        for i in 0..3 {
            if offset + i < out.len() {
                out[offset + i] = bytes[i];
            }
        }
    }
    out
}

pub fn pearl_adjust_target_for_config(
    nbits: u32,
    config: &PearlMiningConfig,
) -> Result<[u8; 32], PearlCompatError> {
    let h = u128::from(config.rows_pattern.size()?);
    let w = u128::from(config.cols_pattern.size()?);
    let dot = config.dot_product_length()? as u128;
    let factor = h
        .checked_mul(w)
        .and_then(|tile| tile.checked_mul(dot))
        .ok_or(PearlCompatError::PublicParamEnvelope)?;
    Ok(u256_le_mul_u128_saturating(
        &pearl_nbits_to_target_le(nbits),
        factor,
    ))
}

fn u256_le_mul_u128_saturating(value: &[u8; 32], factor: u128) -> [u8; 32] {
    if factor == 0 || value.iter().all(|&b| b == 0) {
        return [0u8; 32];
    }

    let mut value_limbs = [0u32; 8];
    for i in 0..8 {
        value_limbs[i] = u32::from_le_bytes([
            value[i * 4],
            value[i * 4 + 1],
            value[i * 4 + 2],
            value[i * 4 + 3],
        ]);
    }
    let mut factor_limbs = [0u32; 4];
    let factor_bytes = factor.to_le_bytes();
    for i in 0..4 {
        factor_limbs[i] = u32::from_le_bytes([
            factor_bytes[i * 4],
            factor_bytes[i * 4 + 1],
            factor_bytes[i * 4 + 2],
            factor_bytes[i * 4 + 3],
        ]);
    }

    let mut acc = [0u128; 12];
    for i in 0..8 {
        for j in 0..4 {
            acc[i + j] += u128::from(value_limbs[i]) * u128::from(factor_limbs[j]);
        }
    }

    let mut out_limbs = [0u32; 8];
    let mut carry = 0u128;
    for i in 0..12 {
        let total = acc[i] + carry;
        if i < 8 {
            out_limbs[i] = (total & 0xffff_ffff) as u32;
            carry = total >> 32;
        } else if total != 0 {
            return [0xffu8; 32];
        }
    }
    if carry != 0 {
        return [0xffu8; 32];
    }

    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&out_limbs[i].to_le_bytes());
    }
    out
}

pub fn pattern_partitions_bounded(
    pattern: &PearlPeriodicPattern,
    total_dimension: u32,
    max_indices_per_partition: usize,
    max_partitions: usize,
) -> Result<Vec<Vec<u32>>, PearlCompatError> {
    let period = pattern.period()?;
    if period == 0 || total_dimension % period != 0 {
        return Err(PearlCompatError::PatternPeriodDoesNotDivideDimension);
    }
    let base_indices = pattern.to_list_bounded(max_indices_per_partition)?;
    let mut partitions = Vec::new();
    for offset in 0..total_dimension {
        if pattern.offset_is_valid(offset) {
            if partitions.len() == max_partitions {
                return Err(PearlCompatError::PatternListTooLarge);
            }
            let mut partition = Vec::with_capacity(base_indices.len());
            for &base in &base_indices {
                partition.push(
                    offset
                        .checked_add(base)
                        .ok_or(PearlCompatError::PatternPeriodTooLarge)?,
                );
            }
            partitions.push(partition);
        }
    }
    Ok(partitions)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PearlPatternTicket {
    pub a_rows: Vec<u32>,
    pub b_cols: Vec<u32>,
    pub tile_state: TileState,
    pub jackpot_hash: [u8; 32],
}

pub fn compute_pearl_pattern_ticket(
    public_params: &PearlPublicProofParams,
    a_row_major: &[i8],
    b_col_major: &[i8],
    commitments: &PearlWorkCommitments,
    max_pattern_len: usize,
) -> Result<PearlPatternTicket, PearlCompatError> {
    public_params.sanity_check()?;
    if public_params.hash_a != commitments.h_a || public_params.hash_b != commitments.h_b {
        return Err(PearlCompatError::PublicCommitmentMismatch);
    }
    validate_public_matrix_inputs(a_row_major, b_col_major, public_params)?;

    let a_rows = public_params.a_rows_indices_bounded(max_pattern_len)?;
    let b_cols = public_params.b_cols_indices_bounded(max_pattern_len)?;
    let k = public_params.mining_config.common_dim as usize;
    let r = public_params.mining_config.rank as usize;
    let dot_product_len = public_params.mining_config.dot_product_length()?;
    let steps = dot_product_len / r;

    let mut a_prime_rows = Vec::with_capacity(a_rows.len() * k);
    let mut e_row = vec![0i8; k];
    for &row in &a_rows {
        pearl_e_row_into(
            &commitments.s_a, row, public_params.mining_config.common_dim, r, &mut e_row,
        );
        let off = row as usize * k;
        for l in 0..k {
            a_prime_rows.push((a_row_major[off + l] as i16 + e_row[l] as i16) as i8);
        }
    }

    let mut b_prime_cols = Vec::with_capacity(b_cols.len() * k);
    let mut f_col = vec![0i8; k];
    for &col in &b_cols {
        pearl_f_col_into(
            &commitments.s_b, col, public_params.mining_config.common_dim, r, &mut f_col,
        );
        let off = col as usize * k;
        for l in 0..k {
            b_prime_cols.push((b_col_major[off + l] as i16 + f_col[l] as i16) as i8);
        }
    }

    let h = a_rows.len();
    let w = b_cols.len();
    let mut accum = vec![0i32; h * w];
    let mut tile_state = TileState::zero();

    for step in 0..steps {
        let lo = step * r;
        for u in 0..h {
            let a_row = &a_prime_rows[u * k + lo..u * k + lo + r];
            for v in 0..w {
                let b_col = &b_prime_cols[v * k + lo..v * k + lo + r];
                let mut delta = 0i32;
                for l in 0..r {
                    delta = delta.wrapping_add((a_row[l] as i32) * (b_col[l] as i32));
                }
                let idx = u * w + v;
                accum[idx] = accum[idx].wrapping_add(delta);
            }
        }
        let mut x = 0i32;
        for &value in &accum {
            x ^= value;
        }
        tile_state.fold(step as u32, x);
    }

    let jackpot_hash = pearl_jackpot_hash(&tile_state, &commitments.s_a);
    Ok(PearlPatternTicket {
        a_rows,
        b_cols,
        tile_state,
        jackpot_hash,
    })
}

pub fn verify_pearl_pattern_ticket(
    public_params: &PearlPublicProofParams,
    a_row_major: &[i8],
    b_col_major: &[i8],
    commitments: &PearlWorkCommitments,
    max_pattern_len: usize,
) -> Result<PearlPatternTicket, PearlCompatError> {
    let ticket = compute_pearl_pattern_ticket(
        public_params, a_row_major, b_col_major, commitments, max_pattern_len,
    )?;
    if ticket.jackpot_hash != public_params.hash_jackpot {
        return Err(PearlCompatError::JackpotHashMismatch);
    }
    Ok(ticket)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PearlCompatibleWorkPrecheck {
    pub commitments: PearlWorkCommitments,
    pub ticket: PearlPatternTicket,
    pub pearl_target: [u8; 32],
    pub nockchain_target: [u8; 32],
}

/// Verify the complete Pearl-compatible work precheck shared by Pearl and
/// Nockchain.
///
/// This is the canonical Rust entrypoint for checking that a public
/// Pearl-style work statement is tied to the supplied matrices, clears Pearl's
/// `nbits` target, and clears the independent Nockchain target. It deliberately
/// uses Pearl's serialized `sigma` and `mu` transcript, with no Nockchain nonce
/// or selected-tile derivation mixed in.
pub fn verify_pearl_compatible_work(
    public_params: &PearlPublicProofParams,
    a_row_major: &[i8],
    b_col_major: &[i8],
    nockchain_target: &[u8; 32],
    max_pattern_len: usize,
) -> Result<PearlCompatibleWorkPrecheck, PearlCompatError> {
    public_params.sanity_check()?;

    let pearl_target = public_params.pearl_adjusted_target()?;
    if !hash_le_target(&public_params.hash_jackpot, &pearl_target) {
        return Err(PearlCompatError::PearlTargetNotMet);
    }
    if !hash_le_target(&public_params.hash_jackpot, nockchain_target) {
        return Err(PearlCompatError::NockchainTargetNotMet);
    }
    validate_public_matrix_inputs(a_row_major, b_col_major, public_params)?;

    let sigma = public_params.block_header.to_bytes();
    let mu = public_params.mining_config.to_bytes()?;
    let commitments = derive_pearl_work_commitments(&sigma, &mu, a_row_major, b_col_major);
    let ticket = verify_pearl_pattern_ticket(
        public_params, a_row_major, b_col_major, &commitments, max_pattern_len,
    )?;

    Ok(PearlCompatibleWorkPrecheck {
        commitments,
        ticket,
        pearl_target,
        nockchain_target: *nockchain_target,
    })
}

fn validate_public_matrix_inputs(
    a_row_major: &[i8],
    b_col_major: &[i8],
    public_params: &PearlPublicProofParams,
) -> Result<(), PearlCompatError> {
    let m = public_params.m as usize;
    let k = public_params.mining_config.common_dim as usize;
    let n = public_params.n as usize;
    if a_row_major.len() != m * k {
        return Err(PearlCompatError::InputAShape {
            expected: m * k,
            actual: a_row_major.len(),
        });
    }
    if b_col_major.len() != n * k {
        return Err(PearlCompatError::InputBShape {
            expected: n * k,
            actual: b_col_major.len(),
        });
    }
    for (index, &value) in a_row_major.iter().enumerate() {
        if !(-INPUT_RANGE_MAX..=INPUT_RANGE_MAX).contains(&value) {
            return Err(PearlCompatError::InputOutOfRange {
                matrix: "A",
                index,
                value,
            });
        }
    }
    for (index, &value) in b_col_major.iter().enumerate() {
        if !(-INPUT_RANGE_MAX..=INPUT_RANGE_MAX).contains(&value) {
            return Err(PearlCompatError::InputOutOfRange {
                matrix: "B",
                index,
                value,
            });
        }
    }
    Ok(())
}

fn pearl_e_row_into(seed: &[u8; 32], row: u32, k: u32, r: usize, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    let mut e_l_row = vec![0i8; r];
    prng::expand_e_l_row(seed, row, r as u32, &mut e_l_row);
    for l in 0..k {
        let (pp, pm) = prng::e_r_col_positions(seed, l, r as u32);
        out[l as usize] = e_l_row[pp as usize] - e_l_row[pm as usize];
    }
}

fn pearl_f_col_into(seed: &[u8; 32], col: u32, k: u32, r: usize, out: &mut [i8]) {
    debug_assert_eq!(out.len(), k as usize);
    let mut f_r_col = vec![0i8; r];
    prng::expand_f_r_col(seed, col, r as u32, &mut f_r_col);
    for l in 0..k {
        let (pp, pm) = prng::f_l_row_positions(seed, l, r as u32);
        out[l as usize] = f_r_col[pp as usize] - f_r_col[pm as usize];
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PearlWorkCommitments {
    pub kappa: [u8; 32],
    pub h_a: [u8; 32],
    pub h_b: [u8; 32],
    pub s_a: [u8; 32],
    pub s_b: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PearlTileDigest {
    pub tile_i: u32,
    pub tile_j: u32,
    pub tile_state: TileState,
    pub jackpot_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PearlAttempt {
    pub sigma: Vec<u8>,
    pub mu: Vec<u8>,
    pub params: MatmulParams,
    pub commitments: PearlWorkCommitments,
    pub tile_digests: Vec<PearlTileDigest>,
}

pub fn pearl_kappa(sigma: &[u8], mu: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(sigma);
    hasher.update(mu);
    *hasher.finalize().as_bytes()
}

pub fn pearl_matrix_commitments(
    a_row_major: &[i8],
    b_col_major: &[i8],
    kappa: &[u8; 32],
) -> ([u8; 32], [u8; 32]) {
    let a_bytes = i8_slice_as_u8(a_row_major);
    let b_bytes = i8_slice_as_u8(b_col_major);
    (
        matrix_commitment(a_bytes, kappa),
        matrix_commitment(b_bytes, kappa),
    )
}

pub fn pearl_noise_seeds(kappa: &[u8; 32], h_a: &[u8; 32], h_b: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let s_b = noise_seed_b(kappa, h_b);
    let s_a = noise_seed_a(&s_b, h_a);
    (s_a, s_b)
}

pub fn pearl_jackpot_hash(tile_state: &TileState, s_a: &[u8; 32]) -> [u8; 32] {
    tile_state.keyed_hash(s_a)
}

pub fn derive_pearl_work_commitments(
    sigma: &[u8],
    mu: &[u8],
    a_row_major: &[i8],
    b_col_major: &[i8],
) -> PearlWorkCommitments {
    let kappa = pearl_kappa(sigma, mu);
    let (h_a, h_b) = pearl_matrix_commitments(a_row_major, b_col_major, &kappa);
    let (s_a, s_b) = pearl_noise_seeds(&kappa, &h_a, &h_b);
    PearlWorkCommitments {
        kappa,
        h_a,
        h_b,
        s_a,
        s_b,
    }
}

impl PearlAttempt {
    /// Build a Pearl-compatible attempt from typed Pearl header/config values.
    ///
    /// This is the preferred entrypoint for code that has already decoded the
    /// Pearl block template. It serializes `sigma`/`mu` exactly as Pearl does
    /// and rejects obvious config/params mismatches before computing the
    /// matrix commitments, noise, or tile hashes.
    pub fn build_with_config(
        header: &PearlIncompleteBlockHeader,
        config: &PearlMiningConfig,
        a_row_major: &[i8],
        b_col_major: &[i8],
        params: &MatmulParams,
    ) -> Result<Self, PearlCompatError> {
        let sigma = header.to_bytes();
        let mu = config.to_bytes()?;
        Self::build_from_serialized(&sigma, &mu, a_row_major, b_col_major, params)
    }

    /// Build a Pearl-compatible attempt from Pearl's serialized
    /// `IncompleteBlockHeader` (`sigma`) and `MiningConfiguration` (`mu`).
    ///
    /// Both byte strings are parsed before use. The current Nockchain
    /// `MatmulParams` model is still square-tile-oriented, while Pearl's
    /// production `PeriodicPattern` language is more general; this entrypoint
    /// therefore validates the shared consensus-critical scalar parameters
    /// (`common_dim`, `rank`, MMA type, reserved bytes) and leaves full pattern
    /// support to the merge-mining verifier implementation.
    pub fn build_from_serialized(
        sigma: &[u8],
        mu: &[u8],
        a_row_major: &[i8],
        b_col_major: &[i8],
        params: &MatmulParams,
    ) -> Result<Self, PearlCompatError> {
        let _header = PearlIncompleteBlockHeader::from_bytes(sigma)?;
        let config = PearlMiningConfig::from_bytes(mu)?;
        validate_config_matches_params(&config, params)?;
        validate_attempt_inputs(a_row_major, b_col_major, params)?;
        let commitments = derive_pearl_work_commitments(sigma, mu, a_row_major, b_col_major);
        let noise = BlockNoise::expand(&commitments.s_a, &commitments.s_b, params);
        let matrices = Matrices::build(a_row_major, b_col_major, &noise, params);
        let mut tile_digests = Vec::with_capacity(params.num_tiles() as usize);
        for tile_i in 0..params.row_tiles() {
            for tile_j in 0..params.col_tiles() {
                let tile_state = compute_tile(&matrices, params, tile_i, tile_j);
                let jackpot_hash = pearl_jackpot_hash(&tile_state, &commitments.s_a);
                tile_digests.push(PearlTileDigest {
                    tile_i,
                    tile_j,
                    tile_state,
                    jackpot_hash,
                });
            }
        }
        Ok(Self {
            sigma: sigma.to_vec(),
            mu: mu.to_vec(),
            params: *params,
            commitments,
            tile_digests,
        })
    }
}

fn validate_config_matches_params(
    config: &PearlMiningConfig,
    params: &MatmulParams,
) -> Result<(), PearlCompatError> {
    if config.common_dim != params.k {
        return Err(PearlCompatError::CommonDimMismatch);
    }
    if u32::from(config.rank) != params.noise_rank {
        return Err(PearlCompatError::RankMismatch);
    }
    Ok(())
}

fn validate_attempt_inputs(
    a_row_major: &[i8],
    b_col_major: &[i8],
    params: &MatmulParams,
) -> Result<(), PearlCompatError> {
    params.validate()?;
    let m = params.m as usize;
    let k = params.k as usize;
    let n = params.n as usize;
    if a_row_major.len() != m * k {
        return Err(PearlCompatError::InputAShape {
            expected: m * k,
            actual: a_row_major.len(),
        });
    }
    if b_col_major.len() != n * k {
        return Err(PearlCompatError::InputBShape {
            expected: n * k,
            actual: b_col_major.len(),
        });
    }
    for (index, &value) in a_row_major.iter().enumerate() {
        if !(-INPUT_RANGE_MAX..=INPUT_RANGE_MAX).contains(&value) {
            return Err(PearlCompatError::InputOutOfRange {
                matrix: "A",
                index,
                value,
            });
        }
    }
    for (index, &value) in b_col_major.iter().enumerate() {
        if !(-INPUT_RANGE_MAX..=INPUT_RANGE_MAX).contains(&value) {
            return Err(PearlCompatError::InputOutOfRange {
                matrix: "B",
                index,
                value,
            });
        }
    }
    Ok(())
}

fn i8_slice_as_u8(input: &[i8]) -> &[u8] {
    // SAFETY: i8 and u8 have identical layout and alignment. The commitment
    // hashes raw two's-complement bytes, which is exactly what Pearl hashes.
    unsafe { core::slice::from_raw_parts(input.as_ptr() as *const u8, input.len()) }
}
