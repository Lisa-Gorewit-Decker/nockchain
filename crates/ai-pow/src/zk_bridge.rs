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
    composite_prove_pinned_logup_sx, composite_verify_pow_pinned_logup_sx, CircuitConfig,
    CompositePublicInputs, CompositeTrace, PowVerifyError, ZkParams,
};

use crate::params::MatmulParams;
use crate::prover::BlockContext;

// ───────────────────────── P-B (γ Pearl-faithful) ─────────────────────────
//
// Params-driven Layer-0 trace sizing + the single-big-trace
// go/no-go estimator. Pearl sizes its STARK to the computation
// (`pearl_program.rs::degree_bits = expected_num_rows
// .next_power_of_two().max(MIN_STARK_LEN)`); we do the faithful
// analogue here. Crucially this *decomposes* the row budget so the
// γ "measure → go/no-go" question is answerable analytically: it
// shows the **full-matrix chunk-Merkle dominates** at PROD scale
// (≈ `num_chunks·136` rows per matrix, `num_chunks = ⌈|M|/1024⌉`),
// not the §6(b) matmul sweep. See HIGH2_2_DESIGN §4.C.4-G3 P-B.

/// Per-block Layer-0 row budget for the `prove_and_verify_tiled`
/// construction, decomposed so the scale blocker is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer0RowBudget {
    /// Keyed chunk-Merkle of the full A matrix (`m·k` bytes).
    pub mhash_a: u64,
    /// Keyed chunk-Merkle of the full B matrix (`k·n` bytes).
    pub mhash_b: u64,
    /// §6(b) sub-block-major matmul sweep over the attested tile.
    pub sweep: u64,
    /// `noised_packed` producer store (M-S1), conservative bound.
    pub store: u64,
    /// Fold chain + key-pin + jackpot-hash + slack.
    pub fixed: u64,
}

impl Layer0RowBudget {
    /// P-B.2.4 — **strip-opening** cost for one matrix side: the
    /// attested tile's `t·k`-byte strip is `⌈t·k/1024⌉` (+≤1
    /// boundary) BLAKE3 leaf chunks × 16 compressions × 8 rows,
    /// plus the authentication-path parents (≤ leaf-count + a
    /// log-depth spine, 8 rows each) + slack. **`O(t·k)`,
    /// independent of the full matrix size** — vs the old
    /// `O(|matrix|)` full re-hash (`136·⌈|M|/1024⌉`). This is the
    /// production one-tile-one-STARK unblocker.
    fn strip_mhash_rows(t: u64, k: u64) -> u64 {
        let strip_chunks = (t * k).div_ceil(1024).max(1) + 1; // +1: boundary straddle
        strip_chunks * 136 + 2048 // leaves·(16·8) + parents/path + slack
    }

    /// Total Layer-0 rows the construction needs (pre power-of-two
    /// padding).
    pub fn total(&self) -> u64 {
        self.mhash_a + self.mhash_b + self.sweep + self.store + self.fixed
    }

    /// The Layer-0 trace length to allocate: `total`, rounded up to
    /// a power of two, floored at `MIN_STARK_LEN` (the Pearl
    /// `degree_bits` analogue).
    pub fn required_trace_len(&self) -> usize {
        (self.total() as usize)
            .next_power_of_two()
            .max(ai_pow_zk::composite_layout::MIN_STARK_LEN)
    }

    /// Does the whole construction fit one Pearl-§4.8-bounded STARK
    /// (`≤ PEARL_TRACE_BOUND = 2²²`)? After P-B.2.4 (strip-opening)
    /// this is **true for every in-§4.8-envelope params set**
    /// (incl. the real Llama-3.1-8B INT GEMMs) — the matrix-hash is
    /// no longer the blocker.
    pub fn fits_one_stark(&self) -> bool {
        (self.required_trace_len() as u64) <= crate::params::PEARL_TRACE_BOUND
    }
}

/// Decomposed Layer-0 row budget for `prove_and_verify_tiled` on
/// `params` (P-B.2.4 **strip-opening** of the attested tile +
/// the §6(b) sweep). Pure function of the geometry.
pub fn expected_layer0_rows(params: &MatmulParams) -> Layer0RowBudget {
    let t = params.tile as u64;
    let r = params.noise_rank as u64;
    let k = params.k as u64;
    let num_stripes = params.num_stripes() as u64;
    // §6(b)-G1+G2 sweep: (t/2)² sub-blocks · num_stripes · ⌈r/16⌉.
    let sweep = (t / 2) * (t / 2) * num_stripes * r.div_ceil(16);
    // P-B.2.4: each side opens only the attested tile's t·k-byte
    // strip (Pearl §4.6), NOT the whole matrix ⇒ O(t·k), size-
    // independent. `tile_chunk_range` is the verifier-fixed
    // schedule (P-B.2.3).
    let strip = Layer0RowBudget::strip_mhash_rows(t, k);
    Layer0RowBudget {
        mhash_a: strip,
        mhash_b: strip,
        sweep,
        // M-S1 producer store: `enumerate_noised_chunks` de-dups to
        // the tile working set — every 8-i8 sub-window of the t·k
        // A-strips + t·k B-strips ⇒ ≤ 2·(t·k)/8 = t·k/4 distinct
        // chunks (a *sound* upper bound; the actual de-duplicated
        // set is ≤ this).
        store: (t.saturating_mul(params.k as u64)) / 4 + 1,
        // key-pin (3) + fold chain (num_stripes) + jackpot (8) + slack.
        fixed: 3 + num_stripes + 8 + 16,
    }
}

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
    // Tile (0,0): the existing binding/regression tests use
    // `difficulty_bits = 0` (every tile clears `target`), so the
    // attested tile is irrelevant to what they assert. Real
    // mining attests the *found* tile via
    // [`prove_and_verify_for_block`] → [`prove_and_verify_tiled`].
    prove_and_verify_tiled(ctx, params, target, 0, 0)
}

/// HIGH-2.2 §4.E — attest the **actual solved tile**
/// `(tile_i, tile_j)` rather than a hard-coded `(0,0)`. All tiles
/// of a block share `difficulty_target(params)` (the work is
/// finding *any* tile whose keyed digest clears it — Pearl's
/// protocol), so binding the *index* is not a PoW-soundness
/// requirement; what matters is that the SNARK attests a **real**
/// tile's genuine committed-matrix fold (the §6(b) chain), at the
/// tile the plain miner actually cleared. The remaining deep
/// tile↔committed-store binding (a prover proving a tile whose
/// strips are not the block's committed A/B rows/cols) reduces to
/// the §4.C `noised_packed`-non-vacuity-on-sweep-rows residual
/// (place_matmul_step sets `MAT_ID = 0`, emits no `noised_packed`
/// query — HIGH2_2_DESIGN §4.C.10), tracked jointly.
pub fn prove_and_verify_tiled(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    target: &[u8; 32],
    tile_i: u32,
    tile_j: u32,
) -> Result<ZkOutcome, BridgeError> {
    // P-B (γ Pearl-faithful): size the Layer-0 trace from `params`
    // — the faithful analogue of Pearl's `degree_bits()` — instead
    // of the fixed `MIN_STARK_LEN`. For sub-envelope test profiles
    // (e.g. TEST_SMALL) the budget rounds back up to `MIN_STARK_LEN`
    // so behaviour is bit-identical to the prior `baseline_min()`;
    // PROD-class params grow the trace modestly (P-B.2.4: the
    // matrix side is now an O(t·k) strip opening, not the
    // O(|matrix|) full re-hash).
    let budget = expected_layer0_rows(params);
    let mut trace = CompositeTrace::baseline(budget.required_trace_len());
    let height = trace.height();

    // C3 / HASH_A / HASH_B — **Pearl §4.6 strip opening**
    // (P-B.2.4): instead of re-hashing all of A (row-major) and B
    // (col-major) in-circuit (O(|matrix|) ≫ one STARK at PROD —
    // the P-B blocker), open ONLY the attested tile's `t·k`-byte
    // committed plain strips and authenticate them to the
    // off-circuit full-matrix commitment via the BLAKE3 tree.
    // `ctx.h_a_chunk`/`h_b_chunk` (= `matrix_commitment(full)`)
    // stay the bound PI; the recomputed root authenticates to it
    // (P-B.2.0/2.2). `tile_chunk_range` is the verifier-fixed
    // schedule (P-B.2.3) — a pure fn of public params + the
    // attested tile, so the prover cannot open a cheaper region.
    // O(t·k), size-independent ⇒ one tile = one STARK.
    use ai_pow_zk::blake3_tree::{open_strip, pad_to_chunk_boundary, tile_chunk_range};
    let a_bytes: Vec<u8> = ctx.a.iter().map(|&v| v as u8).collect();
    let b_bytes: Vec<u8> = ctx.b.iter().map(|&v| v as u8).collect();
    let tt = params.tile as usize;
    let kk = params.k as usize;
    // A row-major (m rows × k): tile_i's `t` rows, span t·k.
    let a_pad = pad_to_chunk_boundary(&a_bytes);
    let (ca0, ca1, a_nc) =
        tile_chunk_range(tile_i as usize, tt, kk, a_bytes.len());
    let (_oa, a_sibs) = open_strip(&a_bytes, &ctx.kappa, ca0, ca1);
    let (next, _root_a) = trace.place_matrix_strip_opening(
        0,
        &a_pad[ca0 * 1024..ca1 * 1024],
        ca0,
        ca1,
        a_nc,
        &a_sibs,
        &ctx.kappa,
        4, // IS_HASH_A
    );
    // B col-major (n cols × k, col j at j·k): tile_j's `t` cols.
    let b_pad = pad_to_chunk_boundary(&b_bytes);
    let (cb0, cb1, b_nc) =
        tile_chunk_range(tile_j as usize, tt, kk, b_bytes.len());
    let (_ob, b_sibs) = open_strip(&b_bytes, &ctx.kappa, cb0, cb1);
    let (mh_end, _root_b) = trace.place_matrix_strip_opening(
        next,
        &b_pad[cb0 * 1024..cb1 * 1024],
        cb0,
        cb1,
        b_nc,
        &b_sibs,
        &ctx.kappa,
        5, // IS_HASH_B
    );

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

    // HIGH-2.2 §6(b) — place the **real** solved tile's full
    // useful-work chain: the sub-block-major matmul sweep over the
    // committed-matrix tile strips + the co-located StripeXor
    // reduction (`place_useful_work_chain`), then fold the
    // chip-reduced per-stripe `x_steps`. The composite AIR now
    // *forces* the chain
    //   committed A/B → CUMSUM (matmul chip) →
    //   SX_IN (== nxt.CUMSUM) → SX_XR (StripeXor) →
    //   FOLD_XSTEP (§6(b) keystone) → FoldChip → FOLD_STATE →
    //   §4.D keystone → JACKPOT_MSG → C4 → HASH_JACKPOT → C2
    // so a *malicious* prover can no longer fabricate `x_steps` —
    // it must do the real matmul. Reconstruct the noised matrices
    // the same way `BlockContext::build` does (it exposes the
    // seeds), then extract the attested tile's `t·k` row/col
    // strips. `HASH_JACKPOT = BLAKE3(real M, key=s_a)` is the
    // genuine PoW digest, byte-equivalent to the plain miner
    // (`high2_2_xstep_fold_pipeline_byte_equiv_plain`). Tile (0,0)
    // is attested; threading the specific *found* tile + binding
    // its index is §4.E (does not change this binding).
    let noise = crate::matmul::BlockNoise::expand(&ctx.s_a, &ctx.s_b, params);
    let mats = crate::matmul::Matrices::build(ctx.a, ctx.b, &noise, params);
    assert!(
        tile_i < params.row_tiles() && tile_j < params.col_tiles(),
        "attested tile ({tile_i},{tile_j}) out of grid \
         {}×{}",
        params.row_tiles(),
        params.col_tiles()
    );
    let t = params.tile as usize;
    let r = params.noise_rank as usize;
    let num_stripes = params.num_stripes() as usize;
    // `t·k` row-major A-strips / col-major B-strips for the tile
    // (the `compute_tile_from_slices` layout).
    let a_strips: Vec<i8> = (0..t as u32)
        .flat_map(|di| mats.a_prime_row(tile_i * params.tile + di).to_vec())
        .collect();
    let b_strips: Vec<i8> = (0..t as u32)
        .flat_map(|dj| mats.b_prime_col(tile_j * params.tile + dj).to_vec())
        .collect();
    // HIGH-2.2 §6(b)+G1+G2: `StripeXorChip` now has
    // `STRIPE_MAX = 64` per-stripe lanes and `place_useful_work_chain`
    // chunks the `r`-wide stripe dot into `⌈r/TILE_D⌉` accumulating
    // micro-steps, so the full malicious-prover binding covers
    // **every params set with `num_stripes ≤ STRIPE_MAX` whose
    // sweep fits one Layer-0 STARK** — TEST_SMALL (`k/r = 16`) *and*
    // the rectangular `llm_shape` shapes (`k/r = 20`). Only true
    // PROD (`k/r = 64` but sweep ≈ 2²⁰ rows ≫ one STARK) still takes
    // the legacy `compute_tile_trace → place_fold_chain` path
    // (HIGH2_2_DESIGN §4.C.4-G3: segmentation/M12 — the residual,
    // not a forgery hole; soundness held by CRIT-1 + §4.D + §6(a)).
    let sweep_rows = (t / 2) * (t / 2) * num_stripes * r.div_ceil(16);
    // HIGH-2.2 §4.C.11 / M-S1 — the `noised_packed` producer store:
    // one row per *distinct* swept 8-i8 micro-tile chunk. The
    // chunked whole-micro-tile matmul query
    // (`bus_emit::noised_packed`) is balanced only if every consumed
    // chunk is a multiset member of this declared store, so the
    // §6(b) sweep's A/B inputs are now *bound* (not free). §4.C.2
    // / A3.2b (b1): each store row carries the explicit
    // `(plain, noise)` split — `MAT_UNPACK = committed-plain`
    // (`ctx.a`/`ctx.b` at the chunk's tile-strip src — A3.1
    // `enumerate_noised_chunks_with_src`), `NOISE_UNPACK =
    // noise_ref(s_a/s_b)`, `NOISE_PACKED_PREP = polyval(noise,
    // 129)` (CRIT-1-pinned ⇒ the prover cannot choose the noise).
    // `NOISED_PACKED = plain+noise = a′` is unchanged ⇒ M-S1's
    // `noised_packed` LogUp / `populate_lookup_freq` balance
    // exactly as before. Closes the §4.C.2 *noise* tie (store
    // noise == Pearl `noise_ref` of the C1-pinned seed); the
    // *plain* tie (MAT_UNPACK ↔ HASH_A via C3) is A3.2c.
    let store_srcs = CompositeTrace::enumerate_noised_chunks_with_src(
        &a_strips, &b_strips, t, r, num_stripes,
    );
    let n_store = store_srcs.len();
    let kk2 = params.k as usize;
    let plain_noise = |s: &ai_pow_zk::composite_trace::NoisedChunkSrc|
        -> ([i8; 8], [i8; 8]) {
        let mut plain = [0i8; 8];
        let mut noise = [0i8; 8];
        for m in 0..8 {
            if let Some((lane, l)) = s.src[m] {
                if s.side_a {
                    let i = tile_i * params.tile + lane;
                    plain[m] = ctx.a[(i as usize) * kk2 + l as usize];
                    noise[m] = ai_pow_zk::noise_ref::e_value(&ctx.s_a, i, l, r as u32);
                } else {
                    let j = tile_j * params.tile + lane;
                    plain[m] = ctx.b[(j as usize) * kk2 + l as usize];
                    noise[m] = ai_pow_zk::noise_ref::f_value(&ctx.s_b, l, j, r as u32);
                }
            }
        }
        (plain, noise)
    };
    let sweep_fits = num_stripes <= ai_pow_zk::composite_layout::STRIPE_MAX
        && mh_end + 3 + sweep_rows + n_store + 4 + num_stripes < height - 8;
    let real_m = if sweep_fits {
        let sweep_start = mh_end + 3;
        let (rows_used, x_steps) = trace
            .place_useful_work_chain(sweep_start, &a_strips, &b_strips, t, r, num_stripes);
        // Store rows live in the post-sweep passthrough region
        // (place AFTER the sweep so its SX/CUMSUM passthrough on
        // `[sweep_start+rows_used, h)` is already written — this
        // only adds the disjoint MAT_UNPACK/NOISE_UNPACK/
        // NOISED_PACKED/NOISE_PACKED_PREP/CONTROL columns); the
        // fold chain follows them.
        let store_start = sweep_start + rows_used;
        for (i, s) in store_srcs.iter().enumerate() {
            let (plain, noise) = plain_noise(s);
            trace.place_noised_store_row_split(
                store_start + i, &plain, &noise, 0,
            );
        }
        let placed = n_store;
        let fold_start = store_start + placed + 4;
        let xs: Vec<i32> = x_steps[..num_stripes].iter().map(|&u| u as i32).collect();
        trace.place_fold_chain(fold_start, &xs)
    } else {
        let tile_trace = crate::matmul::compute_tile_trace(&mats, params, tile_i, tile_j);
        let fold_start = mh_end + 3;
        assert!(
            fold_start + tile_trace.x_steps.len() < height - 8,
            "fold chain + jackpot block must fit: fold_start={fold_start} \
             stripes={} height={height}",
            tile_trace.x_steps.len()
        );
        trace.place_fold_chain(fold_start, &tile_trace.x_steps)
    };

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
    // §6(b)/G1+G2 keystone is live iff the full useful-work chain
    // was placed (`sweep_fits`: num_stripes ≤ STRIPE_MAX and the
    // chunked sweep fits one Layer-0). `sweep_fits` is a pure
    // function of the trusted `params`/height — the verifier-side
    // value, never the proof — so it is as sound as CRIT-1. Only
    // true PROD (G3/M12) takes the legacy path with sx_bound=false.
    let sx_bound = sweep_fits;
    let (proof, program) = composite_prove_pinned_logup_sx(&cfg, trace, &pis, sx_bound);
    composite_verify_pow_pinned_logup_sx(&cfg, &program, &proof, &pis, target, sx_bound)
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
/// the out-of-circuit difficulty check is sound. `found_idx` is the
/// miner's winning linear tile index (`mine_with_context`); it is
/// decomposed via the MED-3 [`tile_ij`] contract and the **actual
/// solved tile** is attested (HIGH-2.2 §4.E). This is the only
/// entrypoint production / `mine()` should use.
pub fn prove_and_verify_for_block(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    found_idx: u32,
) -> Result<ZkOutcome, BridgeError> {
    let target = crate::tile_hash::difficulty_target(params);
    let (tile_i, tile_j) = tile_ij(found_idx, params)
        .expect("found_idx must be a valid tile index for these params");
    prove_and_verify_tiled(ctx, params, &target, tile_i, tile_j)
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

        // Hardened path: no target argument; found_idx 0 = tile
        // (0,0), matching the primitive's default tile so the PIs
        // are directly comparable.
        let hardened = prove_and_verify_for_block(&ctx, &params, 0)
            .expect("MED-3 hardened entrypoint must prove + pow-verify");

        // It must be equivalent to the primitive invoked with the
        // chain-derived target (same PIs, same tile).
        let target = difficulty_target(&params);
        let primitive = prove_and_verify(&ctx, &params, &target)
            .expect("primitive with chain target must also succeed");
        assert_eq!(hardened.pis, primitive.pis);
    }

    /// HIGH-2.2 §4.E: the bridge attests the **actual solved
    /// tile** (not a hard-coded (0,0)). For a spread of winning
    /// indices the full §6(b) chain proves+pow-verifies, and the
    /// bound `HASH_JACKPOT` is byte-identical to the plain miner's
    /// `BLAKE3(compute_tile(tile_i,tile_j) fold, key=s_a)` for
    /// *that* tile — and distinct tiles give distinct digests
    /// (proving the index is genuinely threaded, not constant).
    #[test]
    fn high2_2_attests_real_solved_tile() {
        let params = MatmulParams::TEST_SMALL; // k/r = 16 ⇒ §6(b) live
        let (a, b) = synth_matrices(b"hi22-4e-seed", &params);
        let ctx = BlockContext::build(b"hi22-4e-blk", &a, &b, &params).expect("ctx");

        let nt = params.num_tiles();
        let mut digests = std::collections::HashSet::new();
        for &found_idx in &[0u32, 5, nt / 2, nt - 1] {
            let (ti, tj) = tile_ij(found_idx, &params).expect("valid idx");
            let out = prove_and_verify_for_block(&ctx, &params, found_idx)
                .unwrap_or_else(|e| panic!("§4.E: tile ({ti},{tj}) must prove+verify: {e}"));

            // Byte-equivalence to the plain solve for THIS tile.
            let want = ctx.m_states[found_idx as usize].keyed_hash(&ctx.s_a);
            assert_eq!(
                ai_pow_zk::hash_jackpot_le_bytes(&out.pis.hash_jackpot),
                want,
                "§4.E: SNARK HASH_JACKPOT != plain digest @tile ({ti},{tj})"
            );
            assert!(
                digests.insert(want),
                "distinct tiles must give distinct digests (idx {found_idx})"
            );
        }
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

    /// SPIKE GATE 3 — the route-independent §6(b) core
    /// (`StripeXorChip`) reduces the **real** sub-block-major
    /// sweep's per-row accumulator-after-step to
    /// `compute_tile_trace`'s `x_steps` bit-for-bit. Visitation is
    /// sub-block-major (`for sb { for step { fold acc_after[sb][step]
    /// into lane=step } }`); XOR is order-free so the final
    /// `STATE_LEN`-lane register equals the per-stripe XOR scalars.
    /// `final_register(build_trace(..))` exercises the chip's
    /// witness generator; the chip's STARK correctness
    /// (`constraints ⇔ build_trace`) is proven in `ai-pow-zk`'s own
    /// `chips::stripe_xor` suite (the legal-direction split).
    #[test]
    fn high2_2_spike_stripe_xor_reduces_swept_to_x_steps() {
        use crate::matmul::{compute_tile_trace, BlockNoise, Matrices};
        use ai_pow_zk::chips::stripe_xor::{
            build_trace as sx_build, final_register, ref_stripe_xor, IN_LEN,
        };

        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"spike-gate3-seed", &params);
        let ctx = BlockContext::build(b"spike-gate3-blk", &a, &b, &params).expect("ctx");
        let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
        let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);
        let steps = params.num_stripes() as usize;

        for &(ti, tj) in &[
            (0u32, 0u32),
            (params.row_tiles() - 1, params.col_tiles() - 1),
            (2, 5),
        ] {
            let mut trace = CompositeTrace::baseline_min();
            let (_rows, acc_after, _final) =
                place_subtile_sweep(&mut trace, &mats, &params, ti, tj, 0);

            // Sub-block-major visitation: lane = stripe index.
            let mut events: Vec<(usize, [i32; IN_LEN])> = Vec::new();
            for sb_acc in &acc_after {
                for (step, cells) in sb_acc.iter().enumerate() {
                    events.push((step, *cells));
                }
            }

            let want = compute_tile_trace(&mats, &params, ti, tj).x_steps;
            let reg = final_register(&sx_build(&events));
            let refr = ref_stripe_xor(&events);
            for step in 0..steps {
                assert_eq!(
                    reg[step], want[step] as u32,
                    "StripeXorChip register != x_steps @({ti},{tj}) step {step}"
                );
                assert_eq!(
                    refr[step], want[step] as u32,
                    "ref_stripe_xor != x_steps @({ti},{tj}) step {step}"
                );
            }
            // Unused high lanes (step ≥ num_stripes) stay 0.
            for s in steps..16 {
                assert_eq!(reg[s], 0, "unused lane {s} must be 0");
            }
        }
    }

    /// HIGH-2.2 §6(b)-G1+G2 — the generalized `place_useful_work_chain`
    /// reproduces `compute_tile_trace`'s `x_steps` and verifies
    /// through the composite AIR for params that exercise **both**
    /// G1 (`r = 32 > TILE_D = 16` ⇒ `⌈r/16⌉ = 2` accumulating
    /// inner-chunks per stripe) **and** G2 (`num_stripes = k/r =
    /// 1024/32 = 32 > 16` ⇒ the STRIPE_MAX-lane register +
    /// FOLD_STRIPE_SEL keystone). This is the case the legacy path
    /// could not bind; G1+G2 close it for any single-Layer-0 tile.
    #[test]
    fn high2_2_g1g2_chunked_and_wide_stripes() {
        use crate::matmul::{compute_tile_trace, BlockNoise, Matrices};
        use ai_pow_zk::composite_proof::build_config;
        use ai_pow_zk::{composite_prove, composite_verify, CircuitConfig, ZkParams};

        let params = MatmulParams {
            m: 8,
            k: 1024,
            n: 8,
            noise_rank: 32, // r > TILE_D ⇒ G1 chunking (chunks=2)
            tile: 4,
            spot_checks: 2,
            difficulty_bits: 0,
        };
        params.validate().expect("g1g2 params valid");
        let num_stripes = params.num_stripes() as usize; // 32 > 16 ⇒ G2
        assert_eq!(num_stripes, 32);
        assert_eq!((params.noise_rank as usize).div_ceil(16), 2); // G1 chunks

        let (a, b) = synth_matrices(b"g1g2-seed", &params);
        let ctx = BlockContext::build(b"g1g2-blk", &a, &b, &params).expect("ctx");
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

        let t = params.tile as usize;
        let r = params.noise_rank as usize;
        for &(ti, tj) in &[(0u32, 0u32), (params.row_tiles() - 1, params.col_tiles() - 1)] {
            let a_strips: Vec<i8> = (0..t as u32)
                .flat_map(|di| mats.a_prime_row(ti * params.tile + di).to_vec())
                .collect();
            let b_strips: Vec<i8> = (0..t as u32)
                .flat_map(|dj| mats.b_prime_col(tj * params.tile + dj).to_vec())
                .collect();

            let mut trace = CompositeTrace::baseline_min();
            let (rows_used, x_steps) = trace
                .place_useful_work_chain(8, &a_strips, &b_strips, t, r, num_stripes);
            // (t/2)² sub-blocks · num_stripes · ⌈r/16⌉ chunks.
            assert_eq!(rows_used, (t / 2) * (t / 2) * num_stripes * 2);

            // Cross-crate parity: the chunked, wide-lane sweep ⊕
            // == the reference per-stripe x_steps, bit-for-bit.
            let want = compute_tile_trace(&mats, &params, ti, tj).x_steps;
            for step in 0..num_stripes {
                assert_eq!(
                    x_steps[step], want[step] as u32,
                    "§6(b)-G1+G2 x_steps mismatch @({ti},{tj}) step {step}"
                );
            }

            let xs: Vec<i32> = x_steps[..num_stripes].iter().map(|&u| u as i32).collect();
            let m = trace.place_fold_chain(8 + rows_used + 4, &xs);
            let ch: [u32; 8] = core::array::from_fn(|i| 0x9E37_0000 + i as u32);
            let h = trace.height();
            let _ = trace.place_jackpot_hash_block(h - 8, &m, &ch);

            // The full G1+G2 chain verifies through the composite
            // AIR (matmul chunked sweep recurrence + StripeXor
            // 64-lane transport + SX_IN==nxt.CUMSUM binding + Fold).
            let pis = CompositePublicInputs::derive_from_trace(&trace);
            let proof = composite_prove(&cfg, trace, &pis);
            composite_verify(&cfg, &proof, &pis).unwrap_or_else(|e| {
                panic!("§6(b)-G1+G2 chain must verify @({ti},{tj}): {e:?}")
            });
        }
    }

    // ───────────────── P-B: trace sizing + go/no-go ─────────────────

    /// Sub-envelope test profiles round back up to `MIN_STARK_LEN`,
    /// so P-B's params-driven sizing is **bit-identical** to the
    /// prior `baseline_min()` for them (zero regression — this is
    /// why the whole `ai-pow --features zk` suite stays green).
    #[test]
    fn test_small_sizing_is_min_stark_len() {
        let b = expected_layer0_rows(&MatmulParams::TEST_SMALL);
        assert!(
            b.total() < ai_pow_zk::composite_layout::MIN_STARK_LEN as u64,
            "TEST_SMALL total {} should be < MIN_STARK_LEN",
            b.total()
        );
        assert_eq!(
            b.required_trace_len(),
            ai_pow_zk::composite_layout::MIN_STARK_LEN
        );
        assert!(b.fits_one_stark());
    }

    /// **P-B.2.4 resolution (pinned).** P-B found the *full-matrix*
    /// chunk-Merkle was the one-STARK blocker (≈4.5M rows ≫ 2²² at
    /// PROD). With the §4.6 strip-opening swap, the matrix side is
    /// now `O(t·k)` (size-independent) and **every in-§4.8-envelope
    /// params set — incl. the real Llama-3.1-8B INT GEMMs — fits
    /// one STARK** (`fits_one_stark()` flips true: the production
    /// unblocker). The matrix-hash no longer dominates the sweep.
    #[test]
    fn prod_strip_opening_fits_one_stark() {
        for p in [
            MatmulParams::PROD,
            MatmulParams::GEMMA_4_31B_FFN,
            MatmulParams::QWEN_3_6_27B_FFN,
            MatmulParams::LLAMA_3_1_8B_GATE_UP,
            MatmulParams::LLAMA_3_1_8B_DOWN,
        ] {
            let b = expected_layer0_rows(&p);
            assert!(
                b.fits_one_stark(),
                "{p:?}: must fit one STARK after strip-opening \
                 (total {} > 2²²)",
                b.total()
            );
            // The matrix side is now O(t·k), NOT O(|matrix|): for
            // PROD it is ≪ the old 4.46M full-matrix rows.
            assert!(
                b.mhash_a + b.mhash_b < crate::params::PEARL_TRACE_BOUND / 2,
                "{p:?}: strip mhash {}+{} should be ≪ 2²²",
                b.mhash_a,
                b.mhash_b
            );
        }
        // Concretely PROD: strip = ⌈t·k/1024⌉ chunks, NOT m·k/1024.
        let prod = expected_layer0_rows(&MatmulParams::PROD);
        let t = MatmulParams::PROD.tile as u64;
        let k = MatmulParams::PROD.k as u64;
        let strip_chunks = (t * k).div_ceil(1024) + 1;
        assert_eq!(prod.mhash_a, strip_chunks * 136 + 2048);
        assert!(prod.total() <= crate::params::PEARL_TRACE_BOUND);
    }

    /// Conversely, the **§6(b) sweep alone** (the matmul truth P-A
    /// guarantees) is comfortably within one STARK for PROD —
    /// isolating that the matrix-hash, not the matmul, is what
    /// needs the §4.6 fix.
    #[test]
    fn prod_sweep_alone_fits_one_stark() {
        let b = expected_layer0_rows(&MatmulParams::PROD);
        let sweep_only = (b.sweep + b.store + b.fixed)
            .next_power_of_two()
            .max(ai_pow_zk::composite_layout::MIN_STARK_LEN as u64);
        assert!(
            sweep_only <= crate::params::PEARL_TRACE_BOUND,
            "PROD sweep-only {sweep_only} should fit 2²² (P-A holds)"
        );
    }

    /// Prover-cost scaling measurement (the empirical half of the γ
    /// go/no-go — calibrates the analytic projection to the cap).
    /// Heavy; `#[ignore]` by default. Run:
    /// `cargo test -p ai-pow --features zk pb_prover_cost_scaling
    ///  -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement harness — opt-in (heavy)"]
    fn pb_prover_cost_scaling() {
        use ai_pow_zk::composite_proof::build_config;
        use ai_pow_zk::{composite_prove, CircuitConfig, ZkParams};
        use std::time::Instant;

        let zk = ZkParams {
            m: 64,
            k: 64,
            n: 64,
            noise_rank: 4,
            tile: 8,
            difficulty_bits: 0,
        };
        let cfg = build_config(&zk, &CircuitConfig::TEST_PEARL);
        let min = ai_pow_zk::composite_layout::MIN_STARK_LEN;
        eprintln!("rows,prove_ms,us_per_row");
        for shift in 0..=3 {
            let n = min << shift; // 2^13 .. 2^16
            let trace = CompositeTrace::baseline(n);
            let pis = CompositePublicInputs::derive_from_trace(&trace);
            let t0 = Instant::now();
            let _ = composite_prove(&cfg, trace, &pis);
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            eprintln!("{n},{ms:.1},{:.3}", ms * 1e3 / n as f64);
        }
    }

    /// **§4.C.2 / A3.1 gate (the verifier-recomputable W1/W2
    /// data, KAT-validated; no AIR change).** For the real
    /// bridge geometry, every distinct `noised_packed` store
    /// chunk decomposes as `committed_plain + noise`, where
    /// `noise` is **exactly** `ai_pow_zk::noise_ref` of the
    /// C1-pinned `s_a`/`s_b` at the chunk's deterministic
    /// tile-strip source `(lane,l)`. This is precisely what
    /// A3.2 will write to the store rows
    /// (`MAT_UNPACK=plain`, `NOISE_UNPACK=noise`) and pin into
    /// `NOISE_PACKED_PREP` — de-risked off-circuit first (the
    /// P-B.2.0 discipline).
    #[test]
    fn sec_4c2_store_chunks_decompose_as_committed_plus_noise_ref() {
        use crate::matmul::{BlockNoise, Matrices};
        use crate::synth::synth_matrices;
        use ai_pow_zk::composite_trace::CompositeTrace;

        for params in [
            MatmulParams::TEST_SMALL,
            // a second, distinct geometry (rectangular, r=4|k).
            MatmulParams { m: 16, k: 64, n: 24, noise_rank: 4, tile: 8,
                spot_checks: 2, difficulty_bits: 0 },
        ] {
            params.validate().unwrap();
            let (a, b) = synth_matrices(b"sec4c2-a3.1", &params);
            let ctx = BlockContext::build(b"sec4c2-blk", &a, &b, &params)
                .expect("ctx");
            let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
            let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);
            let (t, r, k) = (
                params.tile as usize,
                params.noise_rank,
                params.k as usize,
            );
            let num_stripes = params.num_stripes() as usize;
            let (ti, tj) = (0u32, 0u32);
            let a_strips: Vec<i8> = (0..t as u32)
                .flat_map(|di| mats.a_prime_row(ti * params.tile + di).to_vec())
                .collect();
            let b_strips: Vec<i8> = (0..t as u32)
                .flat_map(|dj| mats.b_prime_col(tj * params.tile + dj).to_vec())
                .collect();
            // Validate the decomposition over BOTH the value-deduped
            // map (A3.1) AND the position-addressed, witness-free
            // layout (A3.2a) — the latter is what the verifier
            // recomputes to pin NOISE_PACKED_PREP per store row.
            let mut srcs = CompositeTrace::enumerate_noised_chunks_with_src(
                &a_strips, &b_strips, t, r as usize, num_stripes,
            );
            srcs.extend(CompositeTrace::enumerate_noised_chunks_positioned(
                &a_strips, &b_strips, t, r as usize, num_stripes,
            ));
            assert!(!srcs.is_empty());
            for s in &srcs {
                for m in 0..8 {
                    match s.src[m] {
                        None => assert_eq!(
                            s.bytes[m], 0,
                            "zero-pad byte must be 0"
                        ),
                        Some((lane, l)) => {
                            let (plain, nz) = if s.side_a {
                                let i = ti * params.tile + lane;
                                (
                                    ctx.a[(i as usize) * k + l as usize],
                                    ai_pow_zk::noise_ref::e_value(
                                        &ctx.s_a, i, l, r,
                                    ),
                                )
                            } else {
                                let j = tj * params.tile + lane;
                                (
                                    // B is column-major: col j at j*k.
                                    ctx.b[(j as usize) * k + l as usize],
                                    ai_pow_zk::noise_ref::f_value(
                                        &ctx.s_b, l, j, r,
                                    ),
                                )
                            };
                            assert_eq!(
                                s.bytes[m],
                                (plain as i16 + nz as i16) as i8,
                                "chunk byte != committed_plain + \
                                 noise_ref @ side_a={} lane={lane} l={l}",
                                s.side_a
                            );
                        }
                    }
                }
            }
        }
    }

    /// **§4.C.2 / A3.2c c-mset.0 (off-circuit de-risk; no AIR
    /// change).** The B1 plain tie ships as a LogUp multiset bus
    /// (store `MAT_UNPACK` ⊆ the committed-plain windows the A2
    /// strip-opening hashes ∈ `HASH_A`). This KAT proves the
    /// bus's honest-balance + producer-granularity premise
    /// against the *real* bridge geometry: every store row's
    /// plain `MAT_UNPACK` is a **contiguous 8-byte window of the
    /// exact committed bytes the strip-opening hashed** for the
    /// attested tile (within `[c0,c1)·1024`). So the bus producer
    /// = contiguous 8-byte windows of the strip-opening's hashed
    /// plain bytes; every store query is a member ⇒ honest
    /// balance. (The M-S1 coverage-net / P-B.2.0 KAT-first
    /// discipline, applied to c-mset before any bus AIR.)
    #[test]
    fn sec_4c2_cmset0_store_plain_is_contiguous_window_of_strip_opening() {
        use crate::matmul::{BlockNoise, Matrices};
        use crate::synth::synth_matrices;
        use ai_pow_zk::blake3_tree::{pad_to_chunk_boundary, tile_chunk_range};
        use ai_pow_zk::composite_trace::CompositeTrace;

        for params in [
            MatmulParams::TEST_SMALL,
            MatmulParams { m: 16, k: 64, n: 24, noise_rank: 4, tile: 8,
                spot_checks: 2, difficulty_bits: 0 },
        ] {
            params.validate().unwrap();
            let (a, b) = synth_matrices(b"sec4c2-cmset0", &params);
            let ctx = BlockContext::build(b"sec4c2-cmset0-blk", &a, &b, &params)
                .expect("ctx");
            let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
            let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);
            let (t, r, k) = (params.tile as usize, params.noise_rank,
                params.k as usize);
            let num_stripes = params.num_stripes() as usize;
            let (ti, tj) = (0u32, 0u32);
            let a_strips: Vec<i8> = (0..t as u32)
                .flat_map(|di| mats.a_prime_row(ti * params.tile + di).to_vec())
                .collect();
            let b_strips: Vec<i8> = (0..t as u32)
                .flat_map(|dj| mats.b_prime_col(tj * params.tile + dj).to_vec())
                .collect();
            // The exact committed bytes the A2 strip-opening hashes
            // (the producer's byte source), per side.
            let a_bytes: Vec<u8> = ctx.a.iter().map(|&v| v as u8).collect();
            let b_bytes: Vec<u8> = ctx.b.iter().map(|&v| v as u8).collect();
            let a_pad = pad_to_chunk_boundary(&a_bytes);
            let b_pad = pad_to_chunk_boundary(&b_bytes);
            let (ca0, ca1, _) =
                tile_chunk_range(ti as usize, t, k, a_bytes.len());
            let (cb0, cb1, _) =
                tile_chunk_range(tj as usize, t, k, b_bytes.len());

            let srcs = CompositeTrace::enumerate_noised_chunks_with_src(
                &a_strips, &b_strips, t, r as usize, num_stripes,
            );
            assert!(!srcs.is_empty());
            for s in &srcs {
                // A store window's bytes are 8 contiguous columns
                // of ONE strip lane (enumerate splits a chunk into
                // di-fixed 8-col windows) ⇒ a contiguous run in
                // the row/col-major committed matrix.
                let present: Vec<(u32, u32)> =
                    s.src.iter().filter_map(|x| *x).collect();
                if present.is_empty() {
                    continue; // all zero-pad
                }
                let (lane0, l0) = present[0];
                for (m, &(lane, l)) in present.iter().enumerate() {
                    assert_eq!(lane, lane0, "window spans one lane");
                    assert_eq!(
                        l, l0 + m as u32,
                        "window is contiguous in the committed matrix"
                    );
                }
                // The contiguous run lies inside the strip-opening's
                // hashed chunk span, and the store plain bytes equal
                // those exact committed bytes.
                let (pad, c0, c1, lane_g) = if s.side_a {
                    (&a_pad, ca0, ca1, ti * params.tile + lane0)
                } else {
                    (&b_pad, cb0, cb1, tj * params.tile + lane0)
                };
                let idx = lane_g as usize * k + l0 as usize;
                assert!(
                    idx >= c0 * 1024 && idx + present.len() <= c1 * 1024,
                    "store window [{idx},{}) outside strip-opening \
                     hashed span [{},{})",
                    idx + present.len(),
                    c0 * 1024,
                    c1 * 1024,
                );
                for (m, &(_, _)) in present.iter().enumerate() {
                    // committed byte (∈ HASH_A via the strip-opening)
                    // == the store row's plain MAT_UNPACK byte.
                    assert_eq!(
                        pad[idx + m] as i8, s.bytes[m].wrapping_sub(
                            // plain = a′ − noise; recover via the
                            // A3.1-proven decomposition.
                            if s.side_a {
                                ai_pow_zk::noise_ref::e_value(
                                    &ctx.s_a, lane_g, l0 + m as u32, r as u32)
                            } else {
                                ai_pow_zk::noise_ref::f_value(
                                    &ctx.s_b, l0 + m as u32, lane_g, r as u32)
                            }
                        ),
                        "store plain byte != committed (strip-opening) byte"
                    );
                }
            }
        }
    }

    /// **§4.C.2 / A3.2c c-mset.1a — KAT-first de-risk at the exact
    /// `BUS_PLAIN` AIR key (no AIR change).** c-mset.0 validated
    /// the *abstract* byte membership (store plain == committed at
    /// contiguous positions inside the hashed span) but explicitly
    /// `continue`d past zero-pad and never checked the property is
    /// expressible as a *balancing LogUp bus* between the
    /// strip-opening leaf rows and the store rows. This KAT carries
    /// the P-B.2.0 / c-mset.0 discipline to the precise key the
    /// `BUS_PLAIN` AIR would emit:
    ///   * **Producer** = the strip-opening leaf-chunk round-0
    ///     (`IS_NEW_BLAKE`) rows' *unpermuted* `BLAKE3_MSG` — 16
    ///     u32-LE words = the 64 committed bytes of each hashed
    ///     block — split into the 8 disjoint 8-byte word-pair
    ///     windows `(BLAKE3_MSG[2j], BLAKE3_MSG[2j+1])`, j∈0..8,
    ///     over the opened strip `[c0,c1)` (the only chunks that
    ///     get leaf rows; off-range subtrees are auth-sibling CVs,
    ///     not published — and c-mset.0 already proved every store
    ///     window lies in `[c0·1024, c1·1024)`).
    ///   * **Consumer** = each store row's plain 8-byte
    ///     `MAT_UNPACK` window, packed identically (u32-LE of its
    ///     `UINT8_DATA` u8 view = `polyval(.,256)` per 4 bytes).
    ///
    /// Decisive de-risk: is `consumer ⊆ producer` (the exact LogUp
    /// balance premise) at *this* key? **FINDING (validated here):
    /// YES iff `16 | r`** — then every store window is 8 *dense*
    /// contiguous committed bytes, 8-aligned in the row/col-major
    /// matrix (`i·k + l0` with `k, step·r, chunk·16, {0,8}` all
    /// multiples of 8), so it equals exactly one producer
    /// word-pair. Pearl §4.8 pins `r ∈ {2⁵..2¹⁰}` (every value a
    /// multiple of 16) ⇒ **production is always clean**.
    /// `TEST_SMALL` (`r=4`, `16∤4`) is **not**: its windows carry a
    /// zero-pad tail (`col ≥ w`) with no committed counterpart, so
    /// the naive bus does *not* balance there. This is the precise
    /// residual scoping c-mset.1b: the AIR emission must be
    /// `16|r`-gated and Route-A-validated on a `16|r` §6(b)-live
    /// single-STARK geometry, **not** `TEST_SMALL`.
    #[test]
    fn sec_4c2_cmset1a_air_key_producer_superset_of_store_iff_16_divides_r() {
        use crate::matmul::{BlockNoise, Matrices};
        use crate::synth::synth_matrices;
        use ai_pow_zk::blake3_tree::{pad_to_chunk_boundary, tile_chunk_range};
        use ai_pow_zk::composite_trace::CompositeTrace;
        use std::collections::HashSet;

        // The exact 8-byte BUS_PLAIN key (2 u32-LE words = the
        // producer's BLAKE3_MSG word-pair = the consumer's
        // polyval(UINT8_DATA[0..4]) / polyval(UINT8_DATA[4..8])).
        fn key8(b: &[u8]) -> (u32, u32) {
            (
                u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            )
        }
        // Producer key SET: every 8-aligned word-pair window the
        // strip-opening leaf rows expose over `[c0,c1)·1024`.
        fn producer_set(pad: &[u8], c0: usize, c1: usize) -> HashSet<(u32, u32)> {
            let mut s = HashSet::new();
            let (lo, hi) = (c0 * 1024, c1 * 1024);
            let mut off = lo;
            while off + 8 <= hi {
                s.insert(key8(&pad[off..off + 8]));
                off += 8;
            }
            s
        }

        // For `params`: build the real bridge geometry; return
        // (A-side ⊆, B-side ⊆) of consumer-in-producer.
        let check = |params: MatmulParams| -> (bool, bool) {
            params.validate().unwrap();
            let (a, b) = synth_matrices(b"sec4c2-cmset1a", &params);
            let ctx = BlockContext::build(b"sec4c2-cmset1a-blk", &a, &b, &params)
                .expect("ctx");
            let noise = BlockNoise::expand(&ctx.s_a, &ctx.s_b, &params);
            let mats = Matrices::build(ctx.a, ctx.b, &noise, &params);
            let (t, r, k) = (
                params.tile as usize,
                params.noise_rank as usize,
                params.k as usize,
            );
            let num_stripes = params.num_stripes() as usize;
            let (ti, tj) = (0u32, 0u32);
            let a_strips: Vec<i8> = (0..t as u32)
                .flat_map(|di| mats.a_prime_row(ti * params.tile + di).to_vec())
                .collect();
            let b_strips: Vec<i8> = (0..t as u32)
                .flat_map(|dj| mats.b_prime_col(tj * params.tile + dj).to_vec())
                .collect();
            let a_bytes: Vec<u8> = ctx.a.iter().map(|&v| v as u8).collect();
            let b_bytes: Vec<u8> = ctx.b.iter().map(|&v| v as u8).collect();
            let a_pad = pad_to_chunk_boundary(&a_bytes);
            let b_pad = pad_to_chunk_boundary(&b_bytes);
            let (ca0, ca1, _) =
                tile_chunk_range(ti as usize, t, k, a_bytes.len());
            let (cb0, cb1, _) =
                tile_chunk_range(tj as usize, t, k, b_bytes.len());
            let prod_a = producer_set(&a_pad, ca0, ca1);
            let prod_b = producer_set(&b_pad, cb0, cb1);

            let srcs = CompositeTrace::enumerate_noised_chunks_with_src(
                &a_strips, &b_strips, t, r, num_stripes,
            );
            assert!(!srcs.is_empty());
            let (mut a_ok, mut b_ok) = (true, true);
            for s in &srcs {
                // The store row's plain 8-byte window exactly as
                // `write_noised_row_split` lays it out: real byte =
                // committed plain at src; src=None ⇒ 0 (zero-pad).
                let mut win = [0u8; 8];
                let mut all_pad = true;
                for m in 0..8 {
                    if let Some((lane, l)) = s.src[m] {
                        all_pad = false;
                        let lane_g = (if s.side_a { ti } else { tj })
                            * params.tile
                            + lane;
                        let pad = if s.side_a { &a_pad } else { &b_pad };
                        win[m] = pad[lane_g as usize * k + l as usize];
                    }
                }
                if all_pad {
                    continue; // canonical all-zero key; balances trivially
                }
                let kk = key8(&win);
                if s.side_a {
                    a_ok &= prod_a.contains(&kk);
                } else {
                    b_ok &= prod_b.contains(&kk);
                }
            }
            (a_ok, b_ok)
        };

        // POSITIVE — 16|r geometries: every store window is a
        // strip-opening producer member ⇒ BUS_PLAIN honest-balances.
        for p in [
            // single-chunk tile, r=16; §6(b)-live single-STARK class
            // (num_stripes = k/r = 4 ≤ STRIPE_MAX).
            MatmulParams {
                m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
                spot_checks: 2, difficulty_bits: 0,
            },
            // multi-chunk tile (t·k = 2048 = 2 chunks), r=32.
            MatmulParams {
                m: 32, k: 128, n: 32, noise_rank: 32, tile: 16,
                spot_checks: 2, difficulty_bits: 0,
            },
        ] {
            let (a_ok, b_ok) = check(p);
            assert!(
                a_ok && b_ok,
                "16|r (r={}): every store window must be a \
                 strip-opening producer member (BUS_PLAIN honest \
                 balance premise)",
                p.noise_rank
            );
        }

        // NEGATIVE (the precise residual) — TEST_SMALL r=4 (16∤4):
        // store windows carry a zero-pad tail with no committed
        // counterpart ⇒ consumer ⊄ producer. This is *why*
        // c-mset.1b's emission must be 16|r-gated and Route-A
        // validated on a 16|r geometry (Pearl is always 16|r).
        let (a_ok_s, b_ok_s) = check(MatmulParams::TEST_SMALL);
        assert!(
            !(a_ok_s && b_ok_s),
            "TEST_SMALL (r=4, 16∤r): naive BUS_PLAIN must NOT \
             balance (zero-pad-tail residual) — documents the \
             16|r constraint c-mset.1b is gated on"
        );
    }
}
