//! Synchronous block-mining entrypoint. Single-threaded for v1.
//!
//! [`run`] takes a [`MiningJob`], iterates `extranonce` values
//! (NCMN-v1-wrapped), calls
//! [`ai_pow::prover::mine_with_context_at_target`] per attempt, and
//! returns the first solution that clears the chain's
//! [`DifficultyTarget`]. Returns one of [`MiningError::Cancelled`] /
//! [`MiningError::DeadlineElapsed`] / [`MiningError::BudgetExhausted`]
//! on non-success termination.

use std::time::{Duration, Instant};

use ai_pow::prover::{mine_with_context_at_target, BlockContext};

use crate::{
    build_ncmn_nonce, MinedSolution, MineOptions, MiningCancel, MiningError, MiningJob,
    MiningStats,
};

/// Run a block-mining attempt. Builds the [`BlockContext`] once
/// (the noise expansion + cached `m_states` matmul — the expensive
/// part), then loops over `extranonce` values, invoking the
/// per-attempt ai-pow primitive for each.
///
/// Termination order is checked at each `extranonce`:
///   1. Solution → `Ok(MinedSolution)`.
///   2. `cancel.is_cancelled()` → `Err(Cancelled)`.
///   3. `deadline` elapsed → `Err(DeadlineElapsed)`.
///   4. `max_extranonces` reached → `Err(BudgetExhausted)`.
///
/// `params` is validated at entry (defense in depth; the same
/// validation runs inside ai-pow primitives, but failing early
/// gives a clean error before any allocation).
pub fn run(
    job: &MiningJob<'_>,
    opts: &MineOptions,
    cancel: MiningCancel,
) -> Result<MinedSolution, MiningError> {
    job.params.validate().map_err(ai_pow::prover::MineError::from)?;
    let start = Instant::now();
    let ctx = BlockContext::build(job.puzzle_id, job.a, job.b, job.params)?;

    let mut stats = MiningStats {
        extranonces_tried: 0,
        elapsed: Duration::ZERO,
    };
    let mut extranonce = opts.extranonce_start;
    let mut last_progress = start;

    loop {
        if cancel.is_cancelled() {
            return Err(MiningError::Cancelled);
        }
        if let Some(dl) = opts.deadline {
            if Instant::now() >= dl {
                return Err(MiningError::DeadlineElapsed);
            }
        }
        if let Some(max) = opts.max_extranonces {
            if stats.extranonces_tried >= max {
                return Err(MiningError::BudgetExhausted { max });
            }
        }

        let nonce = build_ncmn_nonce(&job.anchors, extranonce);
        let result = mine_with_context_at_target(
            &ctx,
            job.puzzle_id,
            &nonce,
            &job.target,
            opts.prover,
        )?;

        stats.extranonces_tried += 1;
        stats.elapsed = start.elapsed();

        if let Some(proof) = result {
            // Recover the winning linear tile index from the proof's
            // (i, j). `tile_index` is u64 (post H1); for validated
            // params the count fits u32 by definition of TooManyTiles.
            let found_idx_u64 = job.params.tile_index(proof.found.i, proof.found.j);
            debug_assert!(found_idx_u64 <= u64::from(u32::MAX));
            let found_idx = found_idx_u64 as u32;
            return Ok(MinedSolution {
                nonce,
                found_idx,
                proof,
                stats,
            });
        }

        if let Some(interval) = opts.progress_interval {
            if last_progress.elapsed() >= interval {
                tracing::info!(
                    target: "ai_pow_mining",
                    extranonces = stats.extranonces_tried,
                    elapsed_s = stats.elapsed.as_secs_f64(),
                    rate = stats.hash_rate_per_sec(),
                    "mining progress"
                );
                last_progress = Instant::now();
            }
        }

        extranonce = extranonce.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NonceAnchors;
    use ai_pow::params::MatmulParams;
    use ai_pow::synth::synth_matrices;

    /// Stable puzzle-id stand-in for tests. Production callers will
    /// derive this from layer/epoch/params_tag.
    fn puzzle_id() -> Vec<u8> {
        b"ai-pow-mining-test-puzzle-id-v1".to_vec()
    }

    fn test_job<'a>(
        params: &'a MatmulParams,
        a: &'a [i8],
        b: &'a [i8],
        target: [u8; 32],
        puzzle_id: &'a [u8],
    ) -> MiningJob<'a> {
        MiningJob {
            puzzle_id,
            anchors: NonceAnchors::nck_only([0xAB; 32]),
            params,
            target,
            a,
            b,
        }
    }

    #[test]
    fn mine_returns_solution_at_trivial_difficulty() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"mining-trivial-seed", &params);
        let pid = puzzle_id();
        let job = test_job(&params, &a, &b, [0xFFu8; 32], &pid);
        let sol = run(&job, &MineOptions::default(), MiningCancel::new())
            .expect("trivial target ⇒ first extranonce wins");
        // Sanity: nonce parses cleanly + the anchors round-trip.
        let (anchors, xn) = crate::parse_ncmn_nonce(&sol.nonce).expect("nonce");
        assert_eq!(anchors, job.anchors);
        assert_eq!(xn, 0, "first extranonce should win at trivial target");
        // The winning idx is in range.
        let row_tiles = params.m / params.tile;
        let col_tiles = params.n / params.tile;
        assert!(sol.found_idx < row_tiles * col_tiles);
        assert!(sol.stats.extranonces_tried >= 1);
    }

    #[test]
    fn mine_returns_budget_exhausted_at_impossible_difficulty() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"mining-impossible-seed", &params);
        let pid = puzzle_id();
        // Target = [0; 32] ⇒ only `hash == 0` clears (≈ never).
        let job = test_job(&params, &a, &b, [0u8; 32], &pid);
        let opts = MineOptions {
            max_extranonces: Some(3),
            ..MineOptions::default()
        };
        match run(&job, &opts, MiningCancel::new()) {
            Err(MiningError::BudgetExhausted { max }) => assert_eq!(max, 3),
            Ok(_) => panic!("expected BudgetExhausted, got Ok"),
            Err(e) => panic!("expected BudgetExhausted, got Err: {e}"),
        }
    }

    #[test]
    fn mine_returns_cancelled_when_cancel_set() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"mining-cancel-seed", &params);
        let pid = puzzle_id();
        let job = test_job(&params, &a, &b, [0u8; 32], &pid);
        let cancel = MiningCancel::new();
        cancel.cancel();
        match run(&job, &MineOptions::default(), cancel) {
            Err(MiningError::Cancelled) => {}
            Ok(_) => panic!("expected Cancelled, got Ok"),
            Err(e) => panic!("expected Cancelled, got Err: {e}"),
        }
    }

    #[test]
    fn mine_returns_deadline_elapsed_at_past_deadline() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"mining-deadline-seed", &params);
        let pid = puzzle_id();
        let job = test_job(&params, &a, &b, [0u8; 32], &pid);
        let opts = MineOptions {
            deadline: Some(Instant::now() - Duration::from_secs(1)),
            ..MineOptions::default()
        };
        match run(&job, &opts, MiningCancel::new()) {
            Err(MiningError::DeadlineElapsed) => {}
            Ok(_) => panic!("expected DeadlineElapsed, got Ok"),
            Err(e) => panic!("expected DeadlineElapsed, got Err: {e}"),
        }
    }

    #[test]
    fn mine_validates_params_at_entry() {
        let mut params = MatmulParams::TEST_SMALL;
        params.noise_rank = 0; // structurally invalid
        let (a, b) = synth_matrices(b"mining-bad-seed", &MatmulParams::TEST_SMALL);
        let pid = puzzle_id();
        let job = test_job(&params, &a, &b, [0xFFu8; 32], &pid);
        match run(&job, &MineOptions::default(), MiningCancel::new()) {
            Err(MiningError::Mine(_)) => {} // ParamError wrapped via From
            Ok(_) => panic!("expected MiningError::Mine, got Ok"),
            Err(e) => panic!("expected MiningError::Mine, got Err: {e}"),
        }
    }
}
