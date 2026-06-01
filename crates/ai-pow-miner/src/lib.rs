//! `ai-pow-miner` — block-mining driver for the `ai-pow` PoW.
//!
//! Wraps [`ai_pow`] with:
//! * The production-oriented NockApp driver, [`run`], which defaults to
//!   Pearl-format-compatible Nockchain `%ai-pow` submission. In that mode the
//!   miner searches Pearl-style ticket attempts, constructs the canonical
//!   recursive certificate only after the Nockchain target is hit, and submits
//!   only a Nockchain command. Pearl-chain block submission is deliberately
//!   outside this crate's current scope.
//! * A Rust-owned opaque nonce envelope for `%ai-pow` artifacts. Hoon sees the
//!   nonce only as `[len data]`; Pearl-format transcript details remain Rust
//!   metadata and must not become Hoon kernel concepts.
//! * Legacy NCMN-v1 smoke tooling: the chain-required Nockchain header
//!   commitment, reserved external slot, and 8-byte extranonce search variable.
//!   See [`build_ncmn_nonce`] / [`parse_ncmn_nonce`]. This path is retained for
//!   explicit diagnostics and tests, not as the production network submission
//!   API.
//! * A synchronous legacy mining entrypoint that loops over `extranonce`
//!   values, invokes [`ai_pow::prover::mine_with_context_at_target`] for each,
//!   and returns the first solution that clears the chain-supplied 32-byte
//!   difficulty target. [`mining::run`] builds a fresh nonce-bound attempt
//!   context for every extranonce; hoisting keyed commitments, noise, matmul
//!   states, jackpot preimages, or witness inputs out of that loop would reopen
//!   cheap nonce grinding.
//!
//! The bare crate (no features) has no async / NockApp dep, which is useful for
//! benchmarks and fuzz harnesses. The connected node driver lives behind
//! `feature = "nockapp"`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ─────────────────────────── NCMN v1 nonce shape ──────────────────────────
//
// The canonical parser and constants live in `ai-pow` so verifier code cannot
// drift from the miner-side nonce construction.
pub use ai_pow::ncmn::{
    build_ncmn_nonce, parse_ncmn_nonce, NcmnNonce, NonceAnchors, NonceFormatError,
    NCMN_EXTERNAL_ABSENT, NCMN_MAGIC, NCMN_NONCE_LEN, NCMN_VERSION,
};
use ai_pow::params::MatmulParams;
use ai_pow::proof::MatmulProof;
use ai_pow::prover::{MineError, ProverOptions};

// ───────────────────────────── mining-job types ─────────────────────────────

/// Caller-supplied 256-bit chain difficulty bound. Compared
/// little-endian (`ai_pow::tile_hash::hash_le_target` semantics).
pub type DifficultyTarget = [u8; 32];

/// One legacy NCMN smoke mining job. Borrows the matrices; the wrapper crate
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
    /// Count of fully rebuilt nonce-bound matmul attempts.
    ///
    /// This is intentionally not a cheap nonce/hash counter: every increment
    /// corresponds to fresh keyed commitments, noise, noised matrices, and tile
    /// states for one NCMN extranonce.
    pub matmul_attempts_tried: u64,
    pub elapsed: Duration,
}

impl MiningStats {
    pub fn matmul_attempt_rate_per_sec(&self) -> f64 {
        let s = self.elapsed.as_secs_f64();
        if s > 0.0 {
            (self.matmul_attempts_tried as f64) / s
        } else {
            0.0
        }
    }
}

/// Returned on success by the legacy NCMN smoke path.
pub struct MinedSolution {
    /// The full 80-byte NCMN nonce that cleared the target.
    pub nonce: NcmnNonce,
    /// Trusted Nockchain candidate anchor from the mining job used to build
    /// `nonce`. Recursive certificate generation must verify the nonce
    /// against this value, never against the nonce's own parsed anchor.
    pub candidate_nck_commitment: [u8; 32],
    /// Chain-derived 32-byte target used by the winning attempt.
    pub target: DifficultyTarget,
    /// Linear tile index of the winning tile.
    pub found_idx: u32,
    /// The plain ai-pow proof for the winning attempt. This is diagnostic
    /// material, not the canonical recursive `%ai-pow` block certificate.
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
    #[error("NCMN external commitment is reserved and must be absent")]
    NonceExternalCommitmentPresent,
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

/// Pearl-compatible merge-mining ticket loop.
pub mod pearl_mining;

/// Wire vocabulary (`AiPowMinerWire`, `SOURCE = "ai-pow-miner"`). Behind
/// the `node` feature because it implements `nockapp::wire::Wire`.
#[cfg(feature = "node")]
pub mod wire;

/// Noun encoder for the canonical recursive AI-PoW certificate.
#[cfg(feature = "node")]
pub mod certificate_noun;

/// Out-of-process node-connecting run loop ([`run::run`]) — the
/// production entry point used by the `ai-pow-mine` binary. Behind the
/// `node` feature because it pulls in the gRPC + nockapp dep tree.
#[cfg(feature = "node")]
pub mod run;

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
            NonceFormatError::BadVersion {
                got: 99,
                expected: 1
            },
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
