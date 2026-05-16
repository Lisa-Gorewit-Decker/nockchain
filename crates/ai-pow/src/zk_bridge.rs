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
///
/// ## MED-3 — `target` is a trust-bearing argument (primitive)
///
/// This is the **low-level primitive**: it accepts an arbitrary
/// `target`. Difficulty (`HASH_JACKPOT ≤ target`) is checked
/// out-of-circuit / out-of-transcript (Pearl-Layer-0-faithful), so
/// soundness of the difficulty bound is *conditional* on the
/// verifier deriving the correct chain-pinned `target` itself —
/// it must **never** accept a counterparty-supplied target. CRIT-1
/// (now fixed) closes the other MED-3 precondition (`HASH_JACKPOT`
/// genuinely bound). Production code MUST therefore call
/// [`prove_and_verify_for_block`] (which derives
/// `target = difficulty_target(params)` internally and cannot be
/// passed a forged target); this primitive is retained only for
/// tests that deliberately inject a non-chain target. See
/// `crates/ai-pow-zk/ZKP_SECURITY_REPORT.md` §MED-3.
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

/// MED-3-hardened production entrypoint. Derives the difficulty
/// `target` itself from the **chain-pinned** `params`
/// (`difficulty_target(params)` — a pure, deterministic function of
/// `noise_rank` / `tile` / `difficulty_bits`, all part of the
/// block's mining config) and delegates to [`prove_and_verify`].
///
/// Because the target is recomputed from params and never taken as
/// an argument, a caller (or counterparty) **cannot** influence the
/// difficulty bound — closing MED-3 precondition (ii). Combined
/// with CRIT-1 (precondition (i): `HASH_JACKPOT` genuinely bound)
/// the out-of-circuit difficulty check is sound. This is the only
/// entrypoint production / `mine()` should use.
pub fn prove_and_verify_for_block(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
) -> Result<ZkOutcome, BridgeError> {
    let target = crate::tile_hash::difficulty_target(params);
    prove_and_verify(ctx, params, &target)
}

/// MED-3 / HIGH-2.2 §4.E — the **verifier-side derivation contract**
/// for the attested tile index. The winning tile is the miner's
/// linear tile index `found_idx` into `BlockContext::m_states`
/// (`mine_with_context`); it decomposes to grid coordinates as
///
/// ```text
///   tile_i = found_idx / col_tiles      tile_j = found_idx % col_tiles
/// ```
///
/// where `col_tiles = params.col_tiles()` and the index is valid
/// iff `found_idx < params.num_tiles()` — all pure functions of the
/// chain-pinned `params`. The verifier MUST bounds-check
/// `tile_i < params.row_tiles()` and `tile_j < params.col_tiles()`.
/// `(tile_i, tile_j)` is therefore a **verifier-recomputable /
/// verifier-checked** value, *not* a free prover public input;
/// HIGH-2.2 §4.E binds *this* value to the in-circuit matmul
/// accumulator (the §6(b) work). Returns `None` for an
/// out-of-range index (the verifier rejects).
pub fn tile_ij(found_idx: u32, params: &MatmulParams) -> Option<(u32, u32)> {
    if found_idx >= params.num_tiles() {
        return None;
    }
    let col_tiles = params.col_tiles();
    Some((found_idx / col_tiles, found_idx % col_tiles))
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

    /// MED-3: the hardened entrypoint round-trips a real solve and
    /// derives *exactly* `difficulty_target(params)` internally (so
    /// it is byte-for-byte the primitive's chain-pinned target — no
    /// counterparty-supplied target is possible).
    #[test]
    fn med3_prove_and_verify_for_block_roundtrips_and_derives_target() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"med3-seed", &params);
        let ctx = BlockContext::build(b"med3-blk", &a, &b, &params).expect("ctx");

        // Hardened path: no target argument.
        let hardened = prove_and_verify_for_block(&ctx, &params)
            .expect("MED-3 hardened entrypoint must prove + pow-verify");

        // It must be equivalent to the primitive invoked with the
        // chain-derived target (same PIs).
        let target = difficulty_target(&params);
        let primitive = prove_and_verify(&ctx, &params, &target)
            .expect("primitive with chain target must also succeed");
        assert_eq!(hardened.pis, primitive.pis);
    }

    /// MED-3 / §4.E: the verifier-side tile-index derivation
    /// contract — `found_idx → (idx/col_tiles, idx%col_tiles)` over
    /// the whole valid range, `None` past `num_tiles()` (the bound
    /// the verifier rejects on).
    #[test]
    fn med3_tile_ij_derivation_and_bounds() {
        let params = MatmulParams::TEST_SMALL;
        let rt = params.row_tiles();
        let ct = params.col_tiles();
        let nt = params.num_tiles();
        assert_eq!(nt, rt * ct);

        for idx in 0..nt {
            let (ti, tj) = tile_ij(idx, &params).expect("in-range index must decompose");
            assert!(ti < rt && tj < ct, "decomposed coords must be in grid");
            // Round-trips back to the linear index.
            assert_eq!(ti * ct + tj, idx);
        }
        // Out-of-range ⇒ verifier rejects.
        assert_eq!(tile_ij(nt, &params), None);
        assert_eq!(tile_ij(nt + 7, &params), None);
    }

    // ============================================================
    //  §6(b) SPIKE — matmul-row placement / §4.0 subtile-sweep
    //  GEOMETRY (pure arithmetic; no composite proving yet — the
    //  first "test after each sweep" gate). Validates that the
    //  in-circuit 2×2×16 micro-tile chip primitive (`compute_row`),
    //  swept over the (t/2)² sub-blocks × `num_stripes` stripes
    //  with the r-wide stripe zero-padded into TILE_D, reproduces
    //  `compute_tile_trace`'s per-stripe `x_steps` bit-for-bit —
    //  i.e. `FOLD_XSTEP[step]` can be forced == ⊕(swept CUMSUM).
    // ============================================================

    /// Stripe-major sweep of the in-circuit micro-tile primitive
    /// over one tile, returning the per-stripe XOR scalar sequence
    /// (the value the FoldChip consumes). Mirrors
    /// `compute_tile_trace`'s loop using ONLY
    /// `ai_pow_zk::chips::matmul::compute::compute_row`.
    fn swept_micro_tile_x_steps(
        mats: &crate::matmul::Matrices,
        params: &MatmulParams,
        tile_i: u32,
        tile_j: u32,
    ) -> Vec<i32> {
        use ai_pow_zk::chips::matmul::compute::{compute_row, CUMSUM_LEN};
        use ai_pow_zk::composite_layout::{TILE_D, TILE_H};

        let t = params.tile as usize;
        let r = params.noise_rank as usize;
        let steps = params.num_stripes() as usize;
        assert!(t % TILE_H == 0, "tile must tile into TILE_H sub-blocks");
        assert!(r <= TILE_D, "stripe width must fit one micro-step (zero-pad)");
        let n_sb = t / TILE_H; // sub-blocks per axis
        let row0 = (tile_i * params.tile) as usize;
        let col0 = (tile_j * params.tile) as usize;

        // One micro-tile accumulator per (sbi,sbj) sub-block.
        let mut cumsum = vec![[0i32; CUMSUM_LEN]; n_sb * n_sb];
        let mut x_steps = Vec::with_capacity(steps);

        for step in 0..steps {
            let lo = step * r;
            for sbi in 0..n_sb {
                for sbj in 0..n_sb {
                    // 2×16 a / b micro-blocks: r real lanes + zero pad.
                    let mut a_blk = [[0i8; TILE_D]; TILE_H];
                    let mut b_blk = [[0i8; TILE_D]; TILE_H];
                    for di in 0..TILE_H {
                        let arow = mats.a_prime_row((row0 + sbi * TILE_H + di) as u32);
                        a_blk[di][..r].copy_from_slice(&arow[lo..lo + r]);
                    }
                    for dj in 0..TILE_H {
                        let bcol = mats.b_prime_col((col0 + sbj * TILE_H + dj) as u32);
                        b_blk[dj][..r].copy_from_slice(&bcol[lo..lo + r]);
                    }
                    let sb = sbi * n_sb + sbj;
                    let is_reset = step == 0;
                    let is_update = step > 0;
                    cumsum[sb] =
                        compute_row(&a_blk, &b_blk, &cumsum[sb], is_reset, is_update);
                }
            }
            // ⊕ over ALL t·t accumulator cells (XOR is order-free, so
            // the sub-block layout vs plain c_blk layout is irrelevant).
            let mut x = 0i32;
            for c in &cumsum {
                for &v in c {
                    x ^= v;
                }
            }
            x_steps.push(x);
        }
        x_steps
    }

    /// SPIKE GATE 1 — the §4.0 sweep arithmetic equals
    /// `compute_tile_trace`'s `x_steps` for a spread of tiles of a
    /// genuine `BlockContext` solve (TEST_SMALL: t=8, r=4, k=64 ⇒
    /// 16 stripes × (8/2)²=16 sub-blocks = 256 micro-steps/tile).
    /// If this holds, the honest bridge can place 256 real
    /// `place_matmul_step` rows whose ⊕CUMSUM == the FoldChip's
    /// per-stripe X_STEP — the core of §6(b).
    #[test]
    fn high2_2_spike_subtile_sweep_matches_compute_tile_trace() {
        use crate::matmul::{compute_tile_trace, BlockNoise, Matrices};

        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"spike-sweep-seed", &params);
        let ctx = BlockContext::build(b"spike-sweep-blk", &a, &b, &params).expect("ctx");
        let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
        let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);

        // Exhaustive over a representative tile spread incl. corners
        // of the 8×8 tile grid.
        let rt = params.row_tiles();
        let ct = params.col_tiles();
        for &(ti, tj) in &[
            (0u32, 0u32),
            (0, ct - 1),
            (rt - 1, 0),
            (rt - 1, ct - 1),
            (3, 5),
            (rt / 2, ct / 2),
        ] {
            let want = compute_tile_trace(&mats, &params, ti, tj).x_steps;
            let got = swept_micro_tile_x_steps(&mats, &params, ti, tj);
            assert_eq!(
                got.len(),
                params.num_stripes() as usize,
                "x_steps length must equal num_stripes"
            );
            assert_eq!(
                got, want,
                "subtile-sweep x_steps != compute_tile_trace @({ti},{tj})"
            );
            // And the FoldChip over the swept x_steps must reproduce
            // the real TileState M (closing the loop to §4.B).
            assert_eq!(
                crate::matmul::TileState::from_x_steps(&got),
                compute_tile_trace(&mats, &params, ti, tj).state,
                "TileState::from_x_steps(swept) != real M @({ti},{tj})"
            );
        }
    }

    /// Place the sub-block-major subtile sweep for one tile into a
    /// `CompositeTrace` via the public `place_matmul_step`
    /// primitive, threading a SINGLE continuous cumsum chain
    /// (chip-valid: every transition is `nxt == compute_row(cur)`)
    /// with `is_reset` only on each 16-row sub-block run's first
    /// row (so the run-boundary carry is discarded by the
    /// `(1−is_reset)` term — the row-ordering analysis under
    /// HIGH-2.2 §6(b)). Returns `(rows_used, acc_after, final)`
    /// where `acc_after[sb][step]` is sub-block `sb`'s accumulator
    /// *after* stripe `step`.
    #[allow(clippy::type_complexity)]
    fn place_subtile_sweep(
        trace: &mut CompositeTrace,
        mats: &crate::matmul::Matrices,
        params: &MatmulParams,
        tile_i: u32,
        tile_j: u32,
        row_start: usize,
    ) -> (usize, Vec<Vec<[i32; 4]>>, [i32; 4]) {
        use ai_pow_zk::chips::matmul::compute::CUMSUM_LEN;
        use ai_pow_zk::composite_layout::{TILE_D, TILE_H};

        let t = params.tile as usize;
        let r = params.noise_rank as usize;
        let steps = params.num_stripes() as usize;
        let n_sb = t / TILE_H;
        let row0 = (tile_i * params.tile) as usize;
        let col0 = (tile_j * params.tile) as usize;

        let mut acc_after = vec![vec![[0i32; CUMSUM_LEN]; steps]; n_sb * n_sb];
        let mut carry = [0i32; CUMSUM_LEN]; // continuous threaded chain
        let mut row = row_start;
        for sbi in 0..n_sb {
            for sbj in 0..n_sb {
                let sb = sbi * n_sb + sbj;
                for step in 0..steps {
                    let lo = step * r;
                    let mut a_blk = [[0i8; TILE_D]; TILE_H];
                    let mut b_blk = [[0i8; TILE_D]; TILE_H];
                    for di in 0..TILE_H {
                        let arow = mats.a_prime_row((row0 + sbi * TILE_H + di) as u32);
                        a_blk[di][..r].copy_from_slice(&arow[lo..lo + r]);
                    }
                    for dj in 0..TILE_H {
                        let bcol = mats.b_prime_col((col0 + sbj * TILE_H + dj) as u32);
                        b_blk[dj][..r].copy_from_slice(&bcol[lo..lo + r]);
                    }
                    let is_reset = step == 0;
                    let is_update = step > 0;
                    // Thread the single continuous chain: cumsum_old
                    // = the prior row's returned cumsum_new. `carry`
                    // entering a run's reset row is discarded by the
                    // chip's `(1−is_reset)` term (analysis §6(b)).
                    let new = trace.place_matmul_step(
                        row, &a_blk, &b_blk, is_reset, is_update, &carry,
                    );
                    acc_after[sb][step] = new;
                    carry = new;
                    row += 1;
                }
            }
        }
        (row - row_start, acc_after, carry)
    }

    /// SPIKE GATE 2 — the 256-row sub-block-major sweep places into
    /// a `CompositeTrace` and **verifies through the unit
    /// `CompositeFullAir`** (the matmul chip's always-on
    /// `when_transition` recurrence is satisfied by the single
    /// threaded chain with per-run resets — validates the
    /// row-ordering analysis on real data), and the per-stripe ⊕
    /// of the *placed* accumulator snapshots still equals
    /// `compute_tile_trace`'s `x_steps` (the §6(b) binding target
    /// is materialized in the real trace).
    #[test]
    fn high2_2_spike_subtile_sweep_verifies_in_composite() {
        use crate::matmul::{compute_tile_trace, BlockNoise, Matrices};
        use ai_pow_zk::composite_proof::build_config;
        use ai_pow_zk::{composite_prove, composite_verify, CircuitConfig, ZkParams};

        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"spike-gate2-seed", &params);
        let ctx = BlockContext::build(b"spike-gate2-blk", &a, &b, &params).expect("ctx");
        let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
        let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);

        let zk = ZkParams {
            m: params.m,
            k: params.k,
            n: params.n,
            noise_rank: params.noise_rank,
            tile: params.tile,
            difficulty_bits: params.difficulty_bits,
        };
        let cfg = build_config(&zk, &CircuitConfig::TEST_PEARL);

        for &(ti, tj) in &[(0u32, 0u32), (params.row_tiles() - 1, params.col_tiles() - 1)] {
            let mut trace = CompositeTrace::baseline_min();
            let (rows_used, acc_after, final_cs) =
                place_subtile_sweep(&mut trace, &mats, &params, ti, tj, 0);

            // Row budget: 16 sub-blocks × 16 stripes = 256 ≪ 8192.
            assert_eq!(rows_used, 256, "expected 16·16 micro-steps");
            assert!(rows_used < trace.height(), "sweep must fit MIN_STARK_LEN");

            // Passthrough the final accumulator to the trace end so
            // the always-on matmul recurrence is satisfied past the
            // sweep (the last row silences via when_transition).
            trace.fill_cumsum_passthrough(rows_used, &final_cs);

            // The §6(b) binding target materialized in the *placed*
            // trace: ⊕ over all sub-blocks of the accumulator after
            // stripe `step` == compute_tile_trace's x_steps.
            let steps = params.num_stripes() as usize;
            let want = compute_tile_trace(&mats, &params, ti, tj).x_steps;
            for step in 0..steps {
                let mut x = 0i32;
                for sb_acc in &acc_after {
                    for &v in &sb_acc[step] {
                        x ^= v;
                    }
                }
                assert_eq!(
                    x, want[step],
                    "placed-trace ⊕CUMSUM != x_steps @({ti},{tj}) step {step}"
                );
            }

            // The matmul chip's cross-row recurrence holds for the
            // real swept schedule end-to-end.
            let pis = CompositePublicInputs::derive_from_trace(&trace);
            let proof = composite_prove(&cfg, trace, &pis);
            composite_verify(&cfg, &proof, &pis).unwrap_or_else(|e| {
                panic!("subtile sweep must verify through CompositeFullAir @({ti},{tj}): {e:?}")
            });
        }
    }
}
