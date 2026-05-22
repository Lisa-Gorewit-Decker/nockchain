//! `ai-pow-mining` — block-mining driver for the `ai-pow` PoW.
//!
//! Wraps [`ai_pow`] with:
//! * The NCMN-v1 **nonce shape** (the chain-required Nockchain header
//!   commitment + an opaque 32-byte slot reserved for a future Pearl-
//!   style external-chain commitment + an 8-byte extranonce search
//!   variable). See [`build_ncmn_nonce`] / [`parse_ncmn_nonce`].
//! * A synchronous mining entrypoint that loops over `extranonce`
//!   values, invokes [`ai_pow::prover::mine_with_context_at_target`]
//!   for each, and returns the first solution that clears the
//!   chain-supplied 32-byte difficulty target. **`mining::run` lives
//!   in the [`mining`] module (added in a later commit) — this
//!   scaffold stage exposes only the types + nonce helpers.**
//! * A NockApp-compatible driver (under the `nockapp` feature) so
//!   the node can register mining alongside its other IO drivers.
//!
//! The bare crate (no features) has no async / NockApp dep — useful
//! for benchmarks, fuzz harnesses, and the smoke-test
//! `ai-pow-mine` binary. The driver lives behind `feature = "nockapp"`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ai_pow::params::MatmulParams;
use ai_pow::prover::{MineError, ProverOptions};
use ai_pow::proof::MatmulProof;

// ─────────────────────────── NCMN v1 nonce shape ──────────────────────────
//
// Layout (offsets in bytes):
//   0    MAGIC               4 bytes  = b"NCMN"      self-describing
//   4    version             1 byte   = 1
//   5    reserved            3 bytes  = 0            (room for v1.x flags)
//   8    nck_commitment      32 bytes              REQUIRED Nockchain anchor
//   40   external_commitment 32 bytes              OPAQUE 32-byte slot. All-
//                                                  zero = "no external chain
//                                                  bound." Reserved for a
//                                                  future Pearl-compat
//                                                  binding — ai-pow-mining
//                                                  treats it as opaque bytes.
//   72   extranonce          8 bytes (u64 BE)     miner's search variable
//   ──── total 80 bytes
//
// Versioned + self-describing: future revisions ("NCMN" magic + bumped
// version, or a new magic) coexist on the wire without ambiguity.

pub const NCMN_MAGIC: [u8; 4] = *b"NCMN";
pub const NCMN_VERSION: u8 = 1;
pub const NCMN_NONCE_LEN: usize = 80;
pub type NcmnNonce = [u8; NCMN_NONCE_LEN];

/// Sentinel value for the `external_commitment` slot: all-zero means
/// "no external-chain commitment supplied" (Nockchain-only mining).
pub const NCMN_EXTERNAL_ABSENT: [u8; 32] = [0u8; 32];

/// The two anchors a single mining attempt commits to.
///
/// `nck_commitment` is the Nockchain header binding (required).
/// `external_commitment` is reserved for a future external-chain
/// binding (e.g. Pearl's `ProofCommitment`). Treated as opaque
/// 32-byte bytes by this crate; the owning chain decides the
/// derivation when integration lands. `None` ⇒ the all-zero
/// sentinel is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonceAnchors {
    pub nck_commitment: [u8; 32],
    pub external_commitment: Option<[u8; 32]>,
}

impl NonceAnchors {
    pub fn nck_only(nck_commitment: [u8; 32]) -> Self {
        Self { nck_commitment, external_commitment: None }
    }
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum NonceFormatError {
    #[error("nonce length {0} != NCMN expected {NCMN_NONCE_LEN}")]
    BadLength(usize),
    #[error("nonce magic {0:?} != NCMN")]
    BadMagic([u8; 4]),
    #[error("nonce version {got} != NCMN version {expected}")]
    BadVersion { got: u8, expected: u8 },
    #[error("nonce reserved bytes must be zero, got {0:?}")]
    BadReserved([u8; 3]),
}

/// Compose an NCMN v1 nonce from the anchors + extranonce.
pub fn build_ncmn_nonce(anchors: &NonceAnchors, extranonce: u64) -> NcmnNonce {
    let mut out = [0u8; NCMN_NONCE_LEN];
    out[0..4].copy_from_slice(&NCMN_MAGIC);
    out[4] = NCMN_VERSION;
    // bytes 5..8 reserved, left as zero.
    out[8..40].copy_from_slice(&anchors.nck_commitment);
    out[40..72].copy_from_slice(
        &anchors.external_commitment.unwrap_or(NCMN_EXTERNAL_ABSENT),
    );
    out[72..80].copy_from_slice(&extranonce.to_be_bytes());
    out
}

/// Reverse direction. The `external_commitment` is reported as
/// `None` iff the slot is the all-zero sentinel.
pub fn parse_ncmn_nonce(
    nonce: &[u8],
) -> Result<(NonceAnchors, u64), NonceFormatError> {
    if nonce.len() != NCMN_NONCE_LEN {
        return Err(NonceFormatError::BadLength(nonce.len()));
    }
    let mut magic = [0u8; 4];
    magic.copy_from_slice(&nonce[0..4]);
    if magic != NCMN_MAGIC {
        return Err(NonceFormatError::BadMagic(magic));
    }
    if nonce[4] != NCMN_VERSION {
        return Err(NonceFormatError::BadVersion {
            got: nonce[4],
            expected: NCMN_VERSION,
        });
    }
    let mut reserved = [0u8; 3];
    reserved.copy_from_slice(&nonce[5..8]);
    if reserved != [0u8; 3] {
        return Err(NonceFormatError::BadReserved(reserved));
    }
    let mut nck = [0u8; 32];
    nck.copy_from_slice(&nonce[8..40]);
    let mut ext = [0u8; 32];
    ext.copy_from_slice(&nonce[40..72]);
    let mut xn = [0u8; 8];
    xn.copy_from_slice(&nonce[72..80]);
    let extranonce = u64::from_be_bytes(xn);
    Ok((
        NonceAnchors {
            nck_commitment: nck,
            external_commitment: if ext == NCMN_EXTERNAL_ABSENT { None } else { Some(ext) },
        },
        extranonce,
    ))
}

// ───────────────────────────── mining-job types ─────────────────────────────

/// Caller-supplied 256-bit chain difficulty bound. Compared
/// little-endian (`ai_pow::tile_hash::hash_le_target` semantics).
pub type DifficultyTarget = [u8; 32];

/// One block's mining job. Borrows the matrices; the wrapper crate
/// does not own the model-weight bytes.
pub struct MiningJob<'a> {
    /// Stable puzzle identity bound into `κ` (ai-pow's
    /// `BlockContext::build` `block_commitment` argument). Convention:
    /// `BLAKE3("ai-pow-puzzle-id-v1" ‖ layer_id ‖ epoch_id ‖
    /// params_tag)`. Changes ⇒ full BlockContext rebuild.
    pub puzzle_id: &'a [u8],
    /// Chain anchors. NCK required; external opaque.
    pub anchors: NonceAnchors,
    pub params: &'a MatmulParams,
    /// Nockchain's difficulty target (the required chain).
    pub target: DifficultyTarget,
    pub a: &'a [i8],
    pub b: &'a [i8],
}

/// Mining-loop tuning. Most callers use [`MineOptions::default`].
#[derive(Clone, Debug)]
pub struct MineOptions {
    /// Where to start the extranonce counter. Useful for sharded
    /// mining (each shard picks a disjoint start).
    pub extranonce_start: u64,
    /// Stop after this many extranonces tried (None ⇒ unbounded).
    pub max_extranonces: Option<u64>,
    /// Wall-clock budget. None ⇒ run until cancelled / exhausted.
    pub deadline: Option<Instant>,
    /// Forwarded to ai-pow's per-attempt loop.
    pub prover: ProverOptions,
    /// Emit a progress event (via the supplied callback) at most
    /// once per `progress_interval`. None ⇒ no progress callbacks.
    pub progress_interval: Option<Duration>,
}

impl Default for MineOptions {
    fn default() -> Self {
        Self {
            extranonce_start: 0,
            max_extranonces: None,
            deadline: None,
            prover: ProverOptions::default(),
            progress_interval: Some(Duration::from_secs(2)),
        }
    }
}

/// Snapshot for progress callbacks + the final solution.
#[derive(Clone, Debug, Default)]
pub struct MiningStats {
    pub extranonces_tried: u64,
    pub elapsed: Duration,
}

impl MiningStats {
    pub fn hash_rate_per_sec(&self) -> f64 {
        let s = self.elapsed.as_secs_f64();
        if s > 0.0 {
            (self.extranonces_tried as f64) / s
        } else {
            0.0
        }
    }
}

/// Returned on success.
pub struct MinedSolution {
    /// The full 80-byte NCMN nonce that cleared the target.
    pub nonce: NcmnNonce,
    /// Linear tile index of the winning tile.
    pub found_idx: u32,
    /// The plain ai-pow proof for the winning attempt.
    pub proof: MatmulProof,
    pub stats: MiningStats,
}

#[derive(thiserror::Error, Debug)]
pub enum MiningError {
    #[error(transparent)]
    Mine(#[from] MineError),
    #[error("cancelled by caller")]
    Cancelled,
    #[error("deadline elapsed without a solution")]
    DeadlineElapsed,
    #[error("extranonce budget exhausted ({max} attempts)")]
    BudgetExhausted { max: u64 },
}

/// Cooperative cancellation. Clone freely; checked at every
/// extranonce boundary.
#[derive(Clone, Default)]
pub struct MiningCancel(Arc<AtomicBool>);

impl MiningCancel {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

// ────────────────────────── re-exports / submodules ─────────────────────────

pub mod mining;

#[cfg(feature = "nockapp")]
pub mod nockapp_driver;

// ─────────────────────────────────── tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ncmn_round_trip_with_external_anchor() {
        let anchors = NonceAnchors {
            nck_commitment: [0x11; 32],
            external_commitment: Some([0x22; 32]),
        };
        for xn in [0u64, 1, 42, u64::MAX] {
            let bytes = build_ncmn_nonce(&anchors, xn);
            assert_eq!(bytes.len(), NCMN_NONCE_LEN);
            let (a2, xn2) = parse_ncmn_nonce(&bytes).expect("parse");
            assert_eq!(a2, anchors);
            assert_eq!(xn2, xn);
        }
    }

    #[test]
    fn ncmn_external_absent_round_trips_as_none() {
        let anchors = NonceAnchors::nck_only([0x33; 32]);
        let bytes = build_ncmn_nonce(&anchors, 7);
        // The 32-byte slot should be all-zero on the wire.
        assert_eq!(&bytes[40..72], &[0u8; 32]);
        let (a2, _) = parse_ncmn_nonce(&bytes).unwrap();
        assert!(a2.external_commitment.is_none());
        assert_eq!(a2.nck_commitment, [0x33; 32]);
    }

    #[test]
    fn ncmn_rejects_bad_length() {
        assert_eq!(
            parse_ncmn_nonce(&[0u8; 32]).unwrap_err(),
            NonceFormatError::BadLength(32),
        );
    }

    #[test]
    fn ncmn_rejects_bad_magic() {
        let mut bytes = build_ncmn_nonce(&NonceAnchors::nck_only([0x44; 32]), 0);
        bytes[0] = b'X';
        match parse_ncmn_nonce(&bytes) {
            Err(NonceFormatError::BadMagic(m)) => assert_eq!(&m, b"XCMN"),
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn ncmn_rejects_bad_version() {
        let mut bytes = build_ncmn_nonce(&NonceAnchors::nck_only([0x55; 32]), 0);
        bytes[4] = 99;
        assert_eq!(
            parse_ncmn_nonce(&bytes).unwrap_err(),
            NonceFormatError::BadVersion { got: 99, expected: 1 },
        );
    }

    #[test]
    fn ncmn_rejects_nonzero_reserved() {
        let mut bytes = build_ncmn_nonce(&NonceAnchors::nck_only([0x66; 32]), 0);
        bytes[6] = 1;
        match parse_ncmn_nonce(&bytes) {
            Err(NonceFormatError::BadReserved(_)) => {}
            other => panic!("expected BadReserved, got {other:?}"),
        }
    }

    #[test]
    fn mining_cancel_works() {
        let c = MiningCancel::new();
        assert!(!c.is_cancelled());
        let c2 = c.clone();
        c.cancel();
        assert!(c2.is_cancelled());
    }
}
