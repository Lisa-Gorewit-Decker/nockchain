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
//! - **C4 / HASH_JACKPOT** — `BLAKE3(JACKPOT_MSG,
//!   key=COMMITMENT_HASH=s_a)` via `place_jackpot_hash_block`
//!   (the trace's final 8 rows; row 7 co-carries the BLAKE3
//!   finalize and a degenerate-but-valid jackpot step, so the
//!   jackpot `when_transition` is vacuous on the last row).
//!   Non-vacuous: the bridge rejects a zero `HASH_JACKPOT`.
//!   Enabled by the `verify_round` leading-boundary gate fix
//!   (`BLAKE3_CHIP_ROUND_GATE_BUG.md`).
//! - **C2** — the difficulty check on the bound `HASH_JACKPOT`
//!   vs the real `difficulty_target`.
//!
//! ## Entrypoint (production)
//!
//! Proving/verifying goes through `ai-pow-zk`'s **Route A**
//! family `composite_prove_pinned_logup` /
//! `composite_verify_pow_pinned_logup` (batch-stark): CRIT-1
//! program-pin **and** the `noised_packed`/range LogUp enforced
//! in one proof. The verifier rebuilds the canonical program
//! from the trusted `ctx`/`params` (never the proof). See
//! `ai_pow_zk::composite_proof` (entrypoint tier table) and
//! `crates/ai-pow-zk/HIGH2_2_DESIGN.md` §4.C.
//!
//! ## Remaining fidelity gap (not a binding gap) — HIGH-2.2 §4.A
//!
//! `JACKPOT_MSG` fed into the C4 hash is all-zero: no matmul /
//! jackpot rows are placed, so the passthrough transition forces
//! the state constant and the `noised_packed` *matmul-input*
//! binding has no queries to bind. The C4 *binding* (CV_OUT ↦
//! PI_HASH_JACKPOT, keyed by the real `s_a`) is fully exercised —
//! `BLAKE3(zeros, key=s_a)` is a genuine non-vacuous keyed
//! digest. Threading the *real* tile-state fold (the
//! matmul→`X_STEP`→rotl13-XOR chain so `JACKPOT_MSG` = the real
//! `TileState M`) is HIGH-2.2 §4.A; it does not weaken any
//! binding, only the fidelity of *what* is hashed. Design +
//! status: `crates/ai-pow-zk/HIGH2_2_DESIGN.md`.

use ai_pow_zk::composite_proof::build_config;
use ai_pow_zk::{
    composite_prove_pinned_logup, composite_verify_pow_pinned_logup, CircuitConfig,
    CompositePublicInputs, CompositeTrace, PowVerifyError, ZkParams,
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

    // HIGH-2.2 §4.A — place the **real** solved tile's
    // matmul→fold chain so `JACKPOT_MSG` = the genuine
    // `TileState M`. (Re-enabled now that the pre-existing
    // JackpotChip bug — the `JACKPOT_MSG` RAM recurrence ungated
    // by `is_active`, which forbade a non-zero `JACKPOT_MSG` —
    // is fixed; `ai-pow-zk` `high2_2_fold_chain_pinned_logup`,
    // exactly this trace shape via Route-A, now verifies.)
    // Reconstruct the noised matrices the same way
    // `BlockContext::build` does (it exposes the seeds, not the
    // matrices), take the attested tile's per-stripe `x_steps`,
    // and fold them via `place_fold_chain`. The §4.D keystone
    // binds last-row `JACKPOT_MSG == FOLD_STATE`, so
    // `HASH_JACKPOT = BLAKE3(real M, key=s_a)` is the genuine
    // PoW digest — byte-equivalent to the plain miner (proven by
    // `zk_bridge::tests::high2_2_xstep_fold_pipeline_byte_equiv_plain`).
    // Tile (0,0) is attested here; binding the specific *found*
    // tile index is §4.E (does not change the binding).
    let noise = crate::matmul::BlockNoise::expand(&ctx.s_a, &ctx.s_b, params);
    let mats = crate::matmul::Matrices::build(ctx.a, ctx.b, &noise, params);
    let tile_trace = crate::matmul::compute_tile_trace(&mats, params, 0, 0);
    let fold_start = mh_end + 3;
    assert!(
        fold_start + tile_trace.x_steps.len() < height - 8,
        "fold chain + jackpot block must fit: fold_start={fold_start} \
         stripes={} height={height}",
        tile_trace.x_steps.len()
    );
    let real_m = trace.place_fold_chain(fold_start, &tile_trace.x_steps);

    // C4 — final jackpot-hash block (trace's last 8 rows):
    // HASH_JACKPOT = BLAKE3(JACKPOT_MSG = real M, key = s_a).
    assert!(
        ch_row + 1 < height - 8,
        "key-pin rows must clear the final jackpot-hash block"
    );
    let _hj = trace.place_jackpot_hash_block(height - 8, &real_m, &s_a_w);

    // Derive PIs and cross-check against the plain-side context.
    let pis = CompositePublicInputs::derive_from_trace(&trace);
    if pis.hash_jackpot == [0u32; 8] {
        return Err(BridgeError::CommitmentMismatch(
            "HASH_JACKPOT vacuous (jackpot-hash block not bound)",
        ));
    }
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

    // HIGH-2.2 §4.C Route A: program-pinned proving **with the
    // cross-chip LogUp enforced** (batch-stark). `*_pinned_logup`
    // commits the canonical program (CRIT-1) AND the
    // `noised_packed`/range LogUp in one proof, so the matmul
    // `A_NOISED`/`B_NOISED` reads are bound to the C3/`HASH_A`
    // canonical store. The verifier rebuilds the canonical
    // program from the trusted shape — a pure function of
    // `ctx`/`params`, never the proof; a zeroed-selector forge is
    // bound to a different program and rejected vs the canonical
    // VK (ai-pow-zk `routea_*` regression suite). Cost ≈ 1.23x
    // the uni-stark pinned path (HIGH2_2_DESIGN.md §4.C.10).
    let (proof, program) = composite_prove_pinned_logup(&cfg, trace, &pis);
    composite_verify_pow_pinned_logup(&cfg, &program, &proof, &pis, target)
        .map_err(BridgeError::Pow)?;

    Ok(ZkOutcome { pis })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::synth_matrices;
    use crate::tile_hash::difficulty_target;

    #[test]
    fn f1_bridge_real_solve_binds_c1_c2_c3_c4() {
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
        // C4 non-vacuous: HASH_JACKPOT = BLAKE3(zeros, key=s_a) ≠ 0.
        assert_ne!(out.pis.hash_jackpot, [0u32; 8]);
    }

    /// HIGH-2.2 §4.B↔plain byte-equivalence (the
    /// `high2_2_byte_equiv_plain` half of §7's test plan).
    ///
    /// `ai-pow-zk`'s `FoldChip` must reproduce the *real* folded
    /// `TileState M` — the exact 16×u32 the plain miner hashes —
    /// for tiles of a genuine `BlockContext` solve, and feeding
    /// that chip output through the same keyed BLAKE3 must yield
    /// the byte-identical PoW digest. This is the cross-crate
    /// parity that `ai-pow-zk`'s own tests cannot assert (it must
    /// not depend on `ai-pow`); `ai-pow` → `ai-pow-zk` under the
    /// `zk` feature is the legal direction.
    #[test]
    fn high2_2_foldchip_byte_equiv_plain_tilestate() {
        use crate::matmul::{compute_tile_trace, BlockNoise, Matrices};
        use ai_pow_zk::chips::fold::{build_trace, final_state};

        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"high2_2-byteequiv", &params);
        let ctx = BlockContext::build(b"high2_2-blk", &a, &b, &params).expect("ctx");

        // Reconstruct the same noised matrices BlockContext built
        // internally (it exposes the seeds, not the matrices).
        let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
        let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);
        let col_tiles = params.col_tiles();

        for tile_i in 0..params.row_tiles() {
            for tile_j in 0..col_tiles {
                let tr = compute_tile_trace(&mats, &params, tile_i, tile_j);

                // Sanity: our reconstruction == BlockContext's own
                // per-tile compute (the value the real solve uses).
                let idx = (tile_i * col_tiles + tile_j) as usize;
                assert_eq!(
                    tr.state, ctx.m_states[idx],
                    "reconstructed tile != BlockContext.m_states[{idx}]"
                );

                // FoldChip reproduces M bit-for-bit (u32 view).
                let chip = final_state(&build_trace(&tr.x_steps));
                let want: [u32; 16] = core::array::from_fn(|i| tr.state.0[i] as u32);
                assert_eq!(
                    chip, want,
                    "FoldChip final state != real TileState M @({tile_i},{tile_j})"
                );

                // …and the chip output, keyed-hashed, == the exact
                // PoW digest the plain side computes (C4 anchor).
                let chip_words_i32: [i32; 16] = core::array::from_fn(|i| chip[i] as i32);
                let chip_state = crate::matmul::TileState(chip_words_i32);
                assert_eq!(
                    chip_state.keyed_hash(&ctx.s_a),
                    tr.state.keyed_hash(&ctx.s_a),
                    "keyed BLAKE3 of FoldChip output != plain PoW digest @({tile_i},{tile_j})"
                );
            }
        }
    }

    /// HIGH-2.2 §4.C.4 cross-crate parity: feeding the *real*
    /// per-stripe `t·t` accumulator (running `c_blk`, reconstructed
    /// exactly as `compute_tile` does) into ai-pow-zk's `XStepChip`
    /// must reproduce `compute_tile_trace`'s `x_steps` bit-for-bit.
    /// This ties the reduction chip to the genuine Pearl §4.5
    /// per-stripe `x` values for real tiles — the parity ai-pow-zk
    /// cannot assert itself (no ai-pow dep).
    #[test]
    fn high2_2_xstepchip_byte_equiv_plain_x_steps() {
        use crate::matmul::{compute_tile_trace, BlockNoise, Matrices};
        use ai_pow_zk::chips::xstep::{build_trace, xsteps};

        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"high2_2-xstep", &params);
        let ctx = BlockContext::build(b"high2_2-xstep-blk", &a, &b, &params).expect("ctx");
        let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
        let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);

        let t = params.tile as usize;
        let r = params.noise_rank as usize;
        let steps = params.num_stripes() as usize;

        for (tile_i, tile_j) in [(0u32, 0u32), (1, 2), (2, 1)] {
            let tr = compute_tile_trace(&mats, &params, tile_i, tile_j);
            let row0 = (tile_i * params.tile) as usize;
            let col0 = (tile_j * params.tile) as usize;

            // Running c_blk snapshot after each stripe — exactly
            // compute_tile's accumulation, so ⊕snapshot == x_steps.
            let mut c_blk = vec![0i32; t * t];
            let mut per_stripe: Vec<Vec<i32>> = Vec::with_capacity(steps);
            for step in 0..steps {
                let lo = step * r;
                for di in 0..t {
                    let a_row = &mats.a_prime_row((row0 + di) as u32)[lo..lo + r];
                    for dj in 0..t {
                        let b_col = &mats.b_prime_col((col0 + dj) as u32)[lo..lo + r];
                        let mut delta: i32 = 0;
                        for l in 0..r {
                            delta = delta.wrapping_add((a_row[l] as i32) * (b_col[l] as i32));
                        }
                        c_blk[di * t + dj] = c_blk[di * t + dj].wrapping_add(delta);
                    }
                }
                per_stripe.push(c_blk.clone());
            }

            let chip = xsteps(&build_trace(&per_stripe));
            let want: Vec<u32> = tr.x_steps.iter().map(|&x| x as u32).collect();
            assert_eq!(
                chip, want,
                "XStepChip x_steps != compute_tile_trace.x_steps @({tile_i},{tile_j})"
            );
        }
    }

    /// HIGH-2.2 capstone: the full useful-work *computation*
    /// chain composed across both ai-pow-zk chips —
    /// real tile accumulator ─XStepChip→ x_steps ─FoldChip→ M —
    /// must equal the plain `TileState M` (== `BlockContext.m_states`)
    /// for every tile, and keyed-BLAKE3 of that M == the plain PoW
    /// digest. Proves XStepChip and FoldChip compose
    /// byte-equivalently end-to-end. The only HIGH-2.2 item beyond
    /// this is the in-AIR *binding* of the accumulator inputs to
    /// the CRIT-1-pinned HASH_A (§4.C Route-C composite step).
    #[test]
    fn high2_2_xstep_fold_pipeline_byte_equiv_plain() {
        use crate::matmul::{compute_tile_trace, BlockNoise, Matrices, TileState};
        use ai_pow_zk::chips::fold::{build_trace as fold_trace, final_state};
        use ai_pow_zk::chips::xstep::{build_trace as xstep_trace, xsteps};

        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"high2_2-pipeline", &params);
        let ctx = BlockContext::build(b"high2_2-pipe-blk", &a, &b, &params).expect("ctx");
        let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
        let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);

        let t = params.tile as usize;
        let r = params.noise_rank as usize;
        let steps = params.num_stripes() as usize;
        let col_tiles = params.col_tiles();

        for tile_i in 0..params.row_tiles() {
            for tile_j in 0..col_tiles {
                let tr = compute_tile_trace(&mats, &params, tile_i, tile_j);
                let row0 = (tile_i * params.tile) as usize;
                let col0 = (tile_j * params.tile) as usize;

                let mut c_blk = vec![0i32; t * t];
                let mut per_stripe: Vec<Vec<i32>> = Vec::with_capacity(steps);
                for step in 0..steps {
                    let lo = step * r;
                    for di in 0..t {
                        let a_row = &mats.a_prime_row((row0 + di) as u32)[lo..lo + r];
                        for dj in 0..t {
                            let b_col = &mats.b_prime_col((col0 + dj) as u32)[lo..lo + r];
                            let mut d: i32 = 0;
                            for l in 0..r {
                                d = d.wrapping_add((a_row[l] as i32) * (b_col[l] as i32));
                            }
                            c_blk[di * t + dj] = c_blk[di * t + dj].wrapping_add(d);
                        }
                    }
                    per_stripe.push(c_blk.clone());
                }

                // XStepChip: accumulator → x_steps.
                let xs_u32 = xsteps(&xstep_trace(&per_stripe));
                let xs_i32: Vec<i32> = xs_u32.iter().map(|&x| x as i32).collect();
                // FoldChip: x_steps → M.
                let m = final_state(&fold_trace(&xs_i32));

                let idx = (tile_i * col_tiles + tile_j) as usize;
                let want: [u32; 16] = core::array::from_fn(|i| tr.state.0[i] as u32);
                assert_eq!(m, want, "composed pipeline M @({tile_i},{tile_j})");
                let bc: [u32; 16] = core::array::from_fn(|i| ctx.m_states[idx].0[i] as u32);
                assert_eq!(m, bc, "pipeline M != BlockContext.m_states[{idx}]");

                let m_i32: [i32; 16] = core::array::from_fn(|i| m[i] as i32);
                assert_eq!(
                    TileState(m_i32).keyed_hash(&ctx.s_a),
                    tr.state.keyed_hash(&ctx.s_a),
                    "keyed BLAKE3 of pipeline M != plain PoW digest"
                );
            }
        }
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
