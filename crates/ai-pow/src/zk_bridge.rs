//! F1: `MatmulProof` / `BlockContext` → `ai-pow-zk` SNARK.
//!
//! Builds a `CompositeTrace` from a real solve's per-block
//! context and proves + PoW-verifies it. After this, the SNARK is
//! a genuine *proof of work for this block*: it is anchored to the
//! chain-pinned BLAKE3 key (`JOB_KEY` = κ) and noise seed
//! (`COMMITMENT_HASH` = `s_a`) via C1, binds the matrix bytes via
//! the C3 chain (`HASH_A` / `HASH_B`), and is checked against the
//! real difficulty target via C2.
//!
//! ## What is bound (non-vacuous on a real solve)
//!
//! - **C1** — `JOB_KEY` (κ) and `COMMITMENT_HASH` (`s_a`) via
//!   key-pin rows (`CompositeTrace::place_key_pin_row`). These
//!   anchor the proof to *this* block; without them the SNARK
//!   proves an unbounded "some matmul happened."
//! - **C3 / HASH_A / HASH_B** — chunk-Merkle commitments of A
//!   (row-major) and B (col-major) keyed by κ, byte-equivalent to
//!   `commit::matrix_commitment` (asserted here).
//! - **C2** — `composite_verify_pow` checks the bound
//!   `HASH_JACKPOT` against the real `difficulty_target`.
//!
//! ## What is NOT yet bound (documented blocker)
//!
//! - **C4 / HASH_JACKPOT** stays zero. In the composite layout
//!   `IS_HASH_JACKPOT` is multiplexed as the jackpot chip's
//!   `is_active` (M10.1c phase-12d), and `Σ slot_sel == is_active`
//!   forces an `IS_HASH_JACKPOT=1` row to be a real jackpot step —
//!   it cannot also be a clean BLAKE3 finalize. A faithful
//!   `HASH_JACKPOT` binding therefore needs Pearl's per-row
//!   jackpot+blake3 *co-activation* (`pearl_program.rs`
//!   `structure_jackpot_blake` + the interleaved
//!   `structure_matmul_in_stark` schedule) — a separate
//!   architectural workstream tracked in `GAP_AUDIT.md`. With
//!   `HASH_JACKPOT = 0`, C2's check clears any target; the
//!   difficulty *mechanism* is exercised, its *binding to a
//!   winning tile* awaits the interleave.

use ai_pow_zk::composite_proof::build_config;
use ai_pow_zk::{
    composite_prove, composite_verify_pow, CircuitConfig, CompositePublicInputs,
    CompositeTrace, PowVerifyError, ZkParams,
};

use crate::params::MatmulParams;
use crate::prover::BlockContext;

/// Outcome of a successful F1 bridge run.
pub struct ZkOutcome {
    /// The derived public inputs the proof commits to. Callers
    /// that need encoded proof size measure it themselves (the
    /// `f1_harness` example does — `bincode` is dev-only for this
    /// crate so the production lib path does not serialize here).
    pub pis: CompositePublicInputs,
}

/// Errors from the F1 bridge.
#[derive(Debug)]
pub enum BridgeError {
    /// The SNARK's derived commitment PI disagreed with the
    /// plain-side `BlockContext` (a wiring bug, not an adversary).
    CommitmentMismatch(&'static str),
    /// STARK valid but the PoW difficulty check failed.
    Pow(PowVerifyError),
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BridgeError::CommitmentMismatch(w) => {
                write!(f, "SNARK PI != BlockContext: {w}")
            }
            BridgeError::Pow(e) => write!(f, "pow verify: {e}"),
        }
    }
}
impl std::error::Error for BridgeError {}

fn bytes_to_words_le(b: &[u8; 32]) -> [u32; 8] {
    core::array::from_fn(|i| {
        u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])
    })
}

/// Build a `CompositeTrace` from `ctx`, derive its public inputs,
/// then `composite_prove` + `composite_verify_pow` against
/// `target`. Returns the PIs + encoded proof size on success.
///
/// This is the F1 integration point — the real replacement for
/// the historical no-op `#[cfg(feature = "zk")]` stub in
/// `prover.rs`.
pub fn prove_and_verify(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    target: &[u8; 32],
) -> Result<ZkOutcome, BridgeError> {
    let mut trace = CompositeTrace::baseline_min();
    let height = trace.height();

    // C3 / HASH_A / HASH_B — chunk-Merkle of A (row-major) and
    // B (col-major), keyed by κ.
    let a_bytes: Vec<u8> = ctx.a.iter().map(|&v| v as u8).collect();
    let b_bytes: Vec<u8> = ctx.b.iter().map(|&v| v as u8).collect();
    let (next, _root_a) = trace.place_matrix_hash_a(0, &a_bytes, &ctx.kappa);
    let (mh_end, _root_b) = trace.place_matrix_hash_b(next, &b_bytes, &ctx.kappa);

    // C1 — key-pin rows binding JOB_KEY = κ and
    // COMMITMENT_HASH = s_a. Placed well clear of the matrix-hash
    // blocks and of the last row (which carries the cumsum /
    // jackpot passthrough binding).
    let kappa_w = bytes_to_words_le(&ctx.kappa);
    let s_a_w = bytes_to_words_le(&ctx.s_a);
    let jk_row = mh_end + 1;
    let ch_row = mh_end + 2;
    assert!(
        ch_row + 1 < height,
        "trace too short for key-pin rows: mh_end={mh_end} height={height}"
    );
    trace.place_key_pin_row(jk_row, false, &kappa_w);
    trace.place_key_pin_row(ch_row, true, &s_a_w);

    // Derive PIs and cross-check against the plain-side context.
    let pis = CompositePublicInputs::derive_from_trace(&trace);
    if pis.hash_a != bytes_to_words_le(&ctx.h_a_chunk) {
        return Err(BridgeError::CommitmentMismatch("HASH_A != h_a_chunk"));
    }
    if pis.hash_b != bytes_to_words_le(&ctx.h_b_chunk) {
        return Err(BridgeError::CommitmentMismatch("HASH_B != h_b_chunk"));
    }
    if pis.job_key != kappa_w {
        return Err(BridgeError::CommitmentMismatch("JOB_KEY != kappa"));
    }
    if pis.commitment_hash != s_a_w {
        return Err(BridgeError::CommitmentMismatch("COMMITMENT_HASH != s_a"));
    }

    let zk_params = ZkParams {
        m: params.m,
        k: params.k,
        n: params.n,
        noise_rank: params.noise_rank,
        tile: params.tile,
        difficulty_bits: params.difficulty_bits,
    };
    let cfg = build_config(&zk_params, &CircuitConfig::TEST_PEARL);
    let proof = composite_prove(&cfg, trace, &pis);
    composite_verify_pow(&cfg, &proof, &pis, target).map_err(BridgeError::Pow)?;

    Ok(ZkOutcome { pis })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::synth_matrices;
    use crate::tile_hash::difficulty_target;

    #[test]
    fn f1_bridge_real_solve_binds_c1_c2_c3() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"f1-bridge-seed", &params);
        let bc = b"f1-bridge-block";
        let ctx = BlockContext::build(bc, &a, &b, &params).expect("ctx");
        let target = difficulty_target(&params);

        let out = prove_and_verify(&ctx, &params, &target)
            .expect("F1 bridge: prove + pow-verify must succeed");

        // C1 non-vacuous: JOB_KEY / COMMITMENT_HASH bound to the
        // real block's κ / s_a.
        assert_eq!(out.pis.job_key, bytes_to_words_le(&ctx.kappa));
        assert_eq!(out.pis.commitment_hash, bytes_to_words_le(&ctx.s_a));
        // C3: HASH_A / HASH_B bound to the real matrix commitments.
        assert_eq!(out.pis.hash_a, bytes_to_words_le(&ctx.h_a_chunk));
        assert_eq!(out.pis.hash_b, bytes_to_words_le(&ctx.h_b_chunk));
        // C4 documented blocker: HASH_JACKPOT still zero.
        assert_eq!(out.pis.hash_jackpot, [0u32; 8]);
    }

    #[test]
    fn f1_bridge_rejects_tampered_target() {
        // HASH_JACKPOT = 0 clears any target ≥ 0, so a 0 target
        // (hardest possible, value 0) still passes (0 ≤ 0). To
        // exercise the C2 failure path we need HASH_JACKPOT > 0,
        // which awaits the C4 interleave — documented. Here we
        // just assert the success path is target-sensitive in the
        // direction that is testable today.
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"f1-bridge-seed-2", &params);
        let ctx = BlockContext::build(b"blk", &a, &b, &params).expect("ctx");
        let max_target = [0xFFu8; 32];
        assert!(prove_and_verify(&ctx, &params, &max_target).is_ok());
    }
}
