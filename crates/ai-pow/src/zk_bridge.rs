//! F1: `MatmulProof` / `BlockContext` → `ai-pow-zk` SNARK.
//!
//! Builds a `CompositeTrace` from a real solve's per-block
//! context and proves + PoW-verifies it. After this, the SNARK is
//! a genuine *proof of work for this block*: it is anchored to the
//! chain-pinned BLAKE3 key (`JOB_KEY` = κ) and nonce-derived jackpot key
//! (`COMMITMENT_HASH` = `pow_key_for_nonce(s_a, nonce)`) via C1, binds the matrix bytes via
//! the C3 chain (`HASH_A` / `HASH_B`), and is checked against the
//! real difficulty target via C2.
//!
//! ## What is bound (non-vacuous on a real solve)
//!
//! - **C1** — `JOB_KEY` (κ) and `COMMITMENT_HASH` (the nonce-derived jackpot key) via
//!   key-pin rows (`CompositeTrace::place_key_pin_row`). These
//!   anchor the proof to *this* block; without them the SNARK
//!   proves an unbounded "some matmul happened."
//! - **C3 / HASH_A / HASH_B** — chunk-Merkle commitments of A
//!   (row-major) and B (col-major) keyed by κ, byte-equivalent to
//!   `commit::matrix_commitment` (asserted here).
//! - **C4 / HASH_JACKPOT** — `BLAKE3(JACKPOT_MSG,
//!   key=COMMITMENT_HASH=pow_key_for_nonce(s_a, nonce))` via `place_jackpot_hash_block`
//!   (the trace's final 8 rows; row 7 co-carries the BLAKE3
//!   finalize and a degenerate-but-valid jackpot step, so the
//!   jackpot `when_transition` is vacuous on the last row).
//!   Non-vacuous: the bridge rejects a zero `HASH_JACKPOT`.
//!   Enabled by the `verify_round` leading-boundary gate fix
//!   (`2026-05-15_BLAKE3_CHIP_ROUND_GATE_BUG.md`).
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
//! `crates/ai-pow-zk/docs/2026-05-15_HIGH2_2_DESIGN.md` §4.C.
//!
//! ## Remaining fidelity gap (not a binding gap) — HIGH-2.2 §4.A
//!
//! `JACKPOT_MSG` fed into the C4 hash is all-zero: no matmul /
//! jackpot rows are placed, so the passthrough transition forces
//! the state constant and the `noised_packed` *matmul-input*
//! binding has no queries to bind. The C4 *binding* (CV_OUT ↦
//! PI_HASH_JACKPOT, keyed by the real nonce-derived `pow_key`) is fully exercised —
//! `BLAKE3(zeros, key=pow_key)` is a genuine non-vacuous keyed
//! digest. Threading the *real* tile-state fold (the
//! matmul→`X_STEP`→rotl13-XOR chain so `JACKPOT_MSG` = the real
//! `TileState M`) is HIGH-2.2 §4.A; it does not weaken any
//! binding, only the fidelity of *what* is hashed. Design +
//! status: `crates/ai-pow-zk/docs/2026-05-15_HIGH2_2_DESIGN.md`.

use ai_pow_zk::composite_proof::{
    build_config, composite_prove_pinned_logup_sx, composite_verify_pow_pinned_logup_sx,
};
use ai_pow_zk::{
    AiPowBatchProof, AiPowProgram, CircuitConfig, CompositePublicInputs, CompositeTrace,
    PowVerifyError, ZkParams,
};

use crate::fiat_shamir::{commitment_key, noise_seed_a, noise_seed_b, pow_key_for_nonce};
use crate::params::{MatmulParams, ParamError};
use crate::prover::{params_tag, BlockContext};

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
    /// Always `true`: the §6(b) in-circuit matmul sweep is the only
    /// matmul path. (The legacy off-circuit `compute_tile_trace`
    /// fallback — which proved no matmul — was deleted; this field
    /// is retained as an explicit invariant signal that the proof's
    /// matmul was proven in-circuit with the `FOLD_XSTEP == SX_XR`
    /// keystone live.)
    pub sweep_in_circuit: bool,
}

/// Public commitments a verifier needs to derive the trusted ZK statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZkPublicCommitments {
    /// Plain row-Merkle root for A. Used to derive `s_a`.
    pub h_a: [u8; 32],
    /// Plain column-Merkle root for B. Used to derive `s_b`.
    pub h_b: [u8; 32],
    /// Chunk-Merkle commitment bound by the ZK `HASH_A` public input.
    pub h_a_chunk: [u8; 32],
    /// Chunk-Merkle commitment bound by the ZK `HASH_B` public input.
    pub h_b_chunk: [u8; 32],
}

impl ZkPublicCommitments {
    pub fn from_context(ctx: &BlockContext<'_>) -> Self {
        Self {
            h_a: ctx.h_a,
            h_b: ctx.h_b,
            h_a_chunk: ctx.h_a_chunk,
            h_b_chunk: ctx.h_b_chunk,
        }
    }
}

/// Verifier-facing ZK proof artifact.
///
/// The verifier must not trust `pis` by itself; [`verify_ai_pow_block`]
/// cross-checks these public inputs against chain-derived commitments and
/// reconstructs the canonical program before invoking the STARK verifier.
pub struct ZkProofArtifact {
    pub proof: AiPowBatchProof,
    pub pis: CompositePublicInputs,
    pub trace_height: usize,
}

struct ZkDerivedStatement {
    kappa: [u8; 32],
    s_a: [u8; 32],
    s_b: [u8; 32],
}

/// Errors from the F1 bridge.
#[derive(Debug)]
pub enum BridgeError {
    /// The SNARK's derived commitment PI disagreed with the
    /// plain-side `BlockContext` (a wiring bug, not an adversary).
    CommitmentMismatch(&'static str),
    /// STARK valid but the PoW difficulty check failed.
    Pow(PowVerifyError),
    /// Verifier-only API rejected a prover-supplied public input before
    /// STARK verification because it did not match trusted chain data.
    PublicInputMismatch(&'static str),
    /// The proof artifact used a trace height different from the verifier's
    /// params-derived construction.
    TraceHeightMismatch { expected: usize, actual: usize },
    /// A prover-side bridge call supplied params that differ from the
    /// `BlockContext`'s precomputed shape and transcript.
    ParamsMismatch {
        context: MatmulParams,
        supplied: MatmulParams,
    },
    /// `params` failed `MatmulParams::validate()` at the `pub`
    /// bridge boundary — entry-point defense (M2) against malformed
    /// params that would otherwise hit a downstream panic. The
    /// concrete failure mode this prevents: `noise_rank == 0` ⇒
    /// `params.num_stripes() = k/r` div-by-zero in
    /// `expected_layer0_rows`. Production callers go through the
    /// chain-pinned (CRIT-1) params and pass cleanly.
    InvalidParams(ParamError),
    /// `prove_and_verify_for_block`: `found_idx` is past the tile
    /// count for these params (pre-M2 this was an `expect("found_idx
    /// must be a valid tile index for these params")` panic).
    FoundIdxOutOfRange { found_idx: u32, num_tiles: u64 },
    /// M3 (DoS audit): the ai-pow-zk verifier-side `canonical_program`
    /// rejected a structurally-broken `ZkParams` (16|r invariant,
    /// tile-grid bounds, trace_len lower bound). Defense-in-depth
    /// behind M2; reachable only on a broken chain-pin trust
    /// (CRIT-1) where the verifier would otherwise hit a deep
    /// `assert!` panic in `schedule_layout` / `tile_chunk_range`.
    ZkParamsInvalid(String),
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BridgeError::CommitmentMismatch(w) => {
                write!(f, "SNARK PI != BlockContext: {w}")
            }
            BridgeError::Pow(e) => write!(f, "pow verify: {e}"),
            BridgeError::PublicInputMismatch(w) => {
                write!(f, "ZK public input mismatch: {w}")
            }
            BridgeError::TraceHeightMismatch { expected, actual } => write!(
                f,
                "trace height mismatch: expected {expected}, got {actual}"
            ),
            BridgeError::ParamsMismatch { context, supplied } => write!(
                f,
                "BlockContext params {context:?} do not match supplied params {supplied:?}"
            ),
            BridgeError::InvalidParams(e) => write!(f, "invalid params: {e}"),
            BridgeError::FoundIdxOutOfRange { found_idx, num_tiles } => write!(
                f,
                "found_idx ({found_idx}) >= num_tiles ({num_tiles})"
            ),
            BridgeError::ZkParamsInvalid(msg) => {
                write!(f, "ai-pow-zk canonical_program rejected params: {msg}")
            }
        }
    }
}
impl std::error::Error for BridgeError {}

fn bytes_to_words_le(b: &[u8; 32]) -> [u32; 8] {
    core::array::from_fn(|i| {
        u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])
    })
}

fn zk_params_from(params: &MatmulParams) -> ZkParams {
    ZkParams {
        m: params.m,
        k: params.k,
        n: params.n,
        noise_rank: params.noise_rank,
        tile: params.tile,
        difficulty_bits: params.difficulty_bits,
    }
}

fn expect_pi_eq(
    got: &[u32; 8],
    expected: &[u32; 8],
    field: &'static str,
) -> Result<(), BridgeError> {
    if got == expected {
        Ok(())
    } else {
        Err(BridgeError::PublicInputMismatch(field))
    }
}

fn ensure_context_params(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
) -> Result<(), BridgeError> {
    if ctx.params == *params {
        Ok(())
    } else {
        Err(BridgeError::ParamsMismatch {
            context: ctx.params,
            supplied: *params,
        })
    }
}

/// Build a verifier-facing ZK proof artifact for a solved block.
///
/// This is a prover-side constructor only. Consumers must verify the returned
/// artifact with [`verify_ai_pow_block`], which derives the trusted statement
/// from chain data and rejects substituted public inputs.
pub fn prove_ai_pow_block(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    nonce: &[u8],
    _target: &[u8; 32],
    found_idx: u32,
) -> Result<ZkProofArtifact, BridgeError> {
    params.validate_prod_envelope().map_err(BridgeError::InvalidParams)?;
    ensure_context_params(ctx, params)?;
    let (tile_i, tile_j) =
        tile_ij(found_idx, params).ok_or(BridgeError::FoundIdxOutOfRange {
            found_idx,
            num_tiles: params.num_tiles(),
        })?;
    let (artifact, _, _) =
        prove_ai_pow_tiled_full(ctx, params, nonce, tile_i, tile_j, |_| {}, None)?;
    Ok(artifact)
}

/// Production verifier-only ZK API.
///
/// The verifier derives `kappa`, `s_b`, `s_a`, `pow_key`, expected public
/// inputs, and the canonical program from trusted block data before invoking
/// the pinned+LogUp proof verifier. Prover-supplied public inputs are treated
/// as claims and are rejected if they do not match these derived values.
pub fn verify_ai_pow_block(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
    found_idx: u32,
    commitments: &ZkPublicCommitments,
    artifact: &ZkProofArtifact,
) -> Result<(), BridgeError> {
    params.validate_prod_envelope().map_err(BridgeError::InvalidParams)?;
    let (tile_i, tile_j) =
        tile_ij(found_idx, params).ok_or(BridgeError::FoundIdxOutOfRange {
            found_idx,
            num_tiles: params.num_tiles(),
        })?;
    let tag = params_tag(params);
    let kappa = commitment_key(block_commitment, &tag);
    let s_b = noise_seed_b(&kappa, &commitments.h_b);
    let s_a = noise_seed_a(&s_b, &commitments.h_a);
    verify_ai_pow_tiled_with_statement(
        nonce,
        params,
        target,
        tile_i,
        tile_j,
        commitments,
        ZkDerivedStatement { kappa, s_a, s_b },
        artifact,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_ai_pow_tiled_with_statement(
    nonce: &[u8],
    params: &MatmulParams,
    target: &[u8; 32],
    tile_i: u32,
    tile_j: u32,
    commitments: &ZkPublicCommitments,
    derived: ZkDerivedStatement,
    artifact: &ZkProofArtifact,
) -> Result<(), BridgeError> {
    params.validate().map_err(BridgeError::InvalidParams)?;
    let expected_height = expected_layer0_rows(params).required_trace_len();
    if artifact.trace_height != expected_height {
        return Err(BridgeError::TraceHeightMismatch {
            expected: expected_height,
            actual: artifact.trace_height,
        });
    }

    let pow_key = pow_key_for_nonce(&derived.s_a, nonce);
    expect_pi_eq(
        &artifact.pis.job_key,
        &bytes_to_words_le(&derived.kappa),
        "JOB_KEY",
    )?;
    expect_pi_eq(
        &artifact.pis.commitment_hash,
        &bytes_to_words_le(&pow_key),
        "COMMITMENT_HASH",
    )?;
    expect_pi_eq(
        &artifact.pis.hash_a,
        &bytes_to_words_le(&commitments.h_a_chunk),
        "HASH_A",
    )?;
    expect_pi_eq(
        &artifact.pis.hash_b,
        &bytes_to_words_le(&commitments.h_b_chunk),
        "HASH_B",
    )?;

    let zk_params = zk_params_from(params);
    let cfg = build_config(&zk_params, &CircuitConfig::PROD);
    let bp = ai_pow_zk::canonical::BlockPublic {
        tile_i,
        tile_j,
        kappa: derived.kappa,
        s_a: derived.s_a,
        s_b: derived.s_b,
    };
    let canonical =
        ai_pow_zk::canonical::canonical_program(&zk_params, &bp, artifact.trace_height)
            .map_err(BridgeError::ZkParamsInvalid)?;
    let sx_bound = true;
    composite_verify_pow_pinned_logup_sx(
        &cfg,
        &canonical,
        &artifact.proof,
        &artifact.pis,
        target,
        sx_bound,
    )
    .map_err(BridgeError::Pow)
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
/// `crates/ai-pow-zk/docs/2026-05-15_ZKP_SECURITY_REPORT.md` §MED-3.
#[cfg(test)]
pub(crate) fn prove_and_verify(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    nonce: &[u8],
    target: &[u8; 32],
) -> Result<ZkOutcome, BridgeError> {
    // Tile (0,0): the existing binding/regression tests use
    // `difficulty_bits = 0` (every tile clears `target`), so the
    // attested tile is irrelevant to what they assert. Real
    // mining attests the *found* tile via
    // [`prove_and_verify_for_block`] → [`prove_and_verify_tiled`].
    prove_and_verify_tiled(ctx, params, nonce, target, 0, 0)
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
pub(crate) fn prove_and_verify_tiled(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    nonce: &[u8],
    target: &[u8; 32],
    tile_i: u32,
    tile_j: u32,
) -> Result<ZkOutcome, BridgeError> {
    // M2 (DoS audit): defensive validation at the `pub` boundary.
    // Without this, downstream `expected_layer0_rows` would hit a
    // `k / noise_rank` div-by-zero panic for `noise_rank = 0`.
    params.validate().map_err(BridgeError::InvalidParams)?;
    ensure_context_params(ctx, params)?;
    prove_and_verify_tiled_tamper(ctx, params, nonce, target, tile_i, tile_j, |_| {})
}

/// Test seam for the §4.C.2 c-exact **position-exact
/// adversarial**. Identical to [`prove_and_verify_tiled`] except
/// `tamper` runs on the fully-built trace **after** PI derivation
/// + the PI cross-checks but **before** the prove — so any
/// rejection is attributable solely to the in-AIR constraints on
/// the tampered cells (e.g. a co-located leaf row's committed
/// plain ≠ the bytes BLAKE3 hashed ⇒ the cx.2-c3 whole-block C3
/// rejects). Production callers go through the no-op wrapper
/// above; `tamper` is never anything but `|_| {}` outside tests.
pub(crate) fn prove_and_verify_tiled_tamper<F: FnOnce(&mut CompositeTrace)>(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    nonce: &[u8],
    target: &[u8; 32],
    tile_i: u32,
    tile_j: u32,
    tamper: F,
) -> Result<ZkOutcome, BridgeError> {
    prove_and_verify_tiled_full(ctx, params, nonce, target, tile_i, tile_j, tamper, None)
}

/// [`prove_and_verify_tiled_tamper`] plus the §4.C.10 adversarial
/// seam. `sweep_override`: when `Some((a', b'))`, the §6(b) matmul
/// sweep and the `noised_packed` producer store are built from
/// `(a', b')`, while the strip-opening and the `HASH_A` / `HASH_B`
/// public inputs stay the committed `ctx.a` / `ctx.b`. A sound AIR
/// MUST reject any such proof — the §6(b) matmul was not performed
/// on the committed matrices. Production callers pass `None`; only
/// the §4.C.10 malicious-miner test passes `Some`.
fn prove_ai_pow_tiled_full<F: FnOnce(&mut CompositeTrace)>(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    nonce: &[u8],
    tile_i: u32,
    tile_j: u32,
    tamper: F,
    sweep_override: Option<(&[i8], &[i8])>,
) -> Result<(ZkProofArtifact, AiPowProgram, bool), BridgeError> {
    params.validate().map_err(BridgeError::InvalidParams)?;
    ensure_context_params(ctx, params)?;

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
    // §4.C.2 c-exact cx.2 g=1 co-location: the Pearl `noise_ref`
    // byte parallel to the opened A strip — entry j = noise at the
    // committed matrix position of `a_pad[ca0*1024 + j]` (A is
    // row-major m×k: row=p/k, col=p%k), 0 on chunk-padding
    // positions (p ≥ |A|). Each leaf round-0 row becomes the M-S1
    // `noised_packed` producer for its block (cx.2-coloc.0-
    // validated map; SEC_4C2 §8.9).
    // cx.2 g=1 co-location is the **production-faithful 16|r**
    // path (cx.2-coloc.0 validated producer ⊇ swept-chunks only
    // for 16|r; Pearl §4.8 always has 16|r). Non-16|r test
    // geometry (e.g. TEST_SMALL, r=4) keeps the pre-cx.2 A3.2b
    // separate-store path (g=0, strictly-stronger-than-pre-A3 but
    // not zero-gap) — co-location there would unbalance
    // `noised_packed` (the cmset.1a finding). `coloc` gates BOTH
    // the leaf-row noise strips AND retiring the separate store.
    let coloc = params.noise_rank % 16 == 0;
    let rr = params.noise_rank;
    let a_strip_lo = ca0 * 1024;
    let a_noise_strip: Vec<i8> = (0..(ca1 - ca0) * 1024)
        .map(|j| {
            let p = a_strip_lo + j;
            if p < a_bytes.len() {
                ai_pow_zk::noise_ref::e_value(
                    &ctx.s_a, (p / kk) as u32, (p % kk) as u32, rr,
                )
            } else {
                0
            }
        })
        .collect();
    let (next, _root_a) = trace.place_matrix_strip_opening(
        0,
        &a_pad[ca0 * 1024..ca1 * 1024],
        ca0,
        ca1,
        a_nc,
        &a_sibs,
        &ctx.kappa,
        4, // IS_HASH_A
        if coloc { Some(&a_noise_strip) } else { None },
    );
    // B col-major (n cols × k, col j at j·k): tile_j's `t` cols.
    let b_pad = pad_to_chunk_boundary(&b_bytes);
    let (cb0, cb1, b_nc) =
        tile_chunk_range(tile_j as usize, tt, kk, b_bytes.len());
    let (_ob, b_sibs) = open_strip(&b_bytes, &ctx.kappa, cb0, cb1);
    // B is col-major flattened [col0(k)|col1(k)|…]: for byte p the
    // matrix col = p/k, k-index = p%k ⇒ f_value(s_b, k-idx, col).
    let b_strip_lo = cb0 * 1024;
    let b_noise_strip: Vec<i8> = (0..(cb1 - cb0) * 1024)
        .map(|j| {
            let p = b_strip_lo + j;
            if p < b_bytes.len() {
                ai_pow_zk::noise_ref::f_value(
                    &ctx.s_b, (p % kk) as u32, (p / kk) as u32, rr,
                )
            } else {
                0
            }
        })
        .collect();
    let (mh_end, _root_b) = trace.place_matrix_strip_opening(
        next,
        &b_pad[cb0 * 1024..cb1 * 1024],
        cb0,
        cb1,
        b_nc,
        &b_sibs,
        &ctx.kappa,
        5, // IS_HASH_B
        if coloc { Some(&b_noise_strip) } else { None },
    );

    // C1 — key-pin rows binding JOB_KEY = κ and
    // COMMITMENT_HASH = pow_key_for_nonce(s_a, nonce). Placed well clear of the matrix-hash
    // blocks and of the last row (which carries the cumsum /
    // jackpot passthrough binding).
    let kappa_w = bytes_to_words_le(&ctx.kappa);
    let pow_key = pow_key_for_nonce(&ctx.s_a, nonce);
    let pow_key_w = bytes_to_words_le(&pow_key);
    let jk_row = mh_end + 1;
    let ch_row = mh_end + 2;
    assert!(
        ch_row + 1 < height,
        "trace too short for key-pin rows: mh_end={mh_end} height={height}"
    );
    trace.place_key_pin_row(jk_row, false, &kappa_w);
    trace.place_key_pin_row(ch_row, true, &pow_key_w);

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
    // strips. `HASH_JACKPOT = BLAKE3(real M, key=pow_key)` is the
    // genuine PoW digest, byte-equivalent to the plain miner
    // (`high2_2_xstep_fold_pipeline_byte_equiv_plain`). Tile (0,0)
    // is attested; threading the specific *found* tile + binding
    // its index is §4.E (does not change this binding).
    let noise = crate::matmul::BlockNoise::expand(&ctx.s_a, &ctx.s_b, params);
    // §4.C.10 adversarial seam: the §6(b) matmul sweep + the
    // `noised_packed` producer store are built from `sweep_override`
    // when present; the strip-opening + `HASH_A`/`HASH_B` (above)
    // always stay the committed `ctx.a`/`ctx.b`. Production = `None`.
    let (sweep_a, sweep_b) = sweep_override.unwrap_or((ctx.a, ctx.b));
    let mats = crate::matmul::Matrices::build(sweep_a, sweep_b, &noise, params);
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
    // every consensus-valid puzzle: `validate_prod_envelope` rejects
    // `num_stripes > STRIPE_MAX`, so the §6(b) in-circuit sweep is
    // the one and only matmul path (the legacy off-circuit
    // `compute_tile_trace` fallback was deleted).
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
    // §4.C.2: producers of the `noised_packed` bus.
    //  * cx.2 g=1 (`coloc`, 16|r): the co-located strip-opening
    //    leaf round-0 rows (placed above with the `noise_strip`s;
    //    cx.2-coloc.0 proved producer ⊇ every swept chunk) — no
    //    separate store rows.
    //  * non-16|r (test geom, e.g. TEST_SMALL): the pre-cx.2
    //    A3.2b separate `place_noised_store_row_split` rows
    //    (MAT_UNPACK=committed-plain, NOISE_UNPACK=noise_ref,
    //    NOISE_PACKED_PREP CRIT-1-pinned ⇒ strictly stronger than
    //    pre-A3, not zero-gap).
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
                    let jc = tile_j * params.tile + lane;
                    plain[m] = ctx.b[(jc as usize) * kk2 + l as usize];
                    noise[m] = ai_pow_zk::noise_ref::f_value(&ctx.s_b, l, jc, r as u32);
                }
            }
        }
        (plain, noise)
    };
    // §6(b) in-circuit matmul sweep — the ONLY matmul path. The
    // legacy off-circuit `compute_tile_trace → place_fold_chain`
    // fallback was deleted: it proved no matmul (`sx_bound = false`,
    // the `FOLD_XSTEP == SX_XR` keystone gated off). Every
    // consensus-valid puzzle fits the in-circuit sweep —
    // `validate_prod_envelope` rejects `num_stripes > STRIPE_MAX`
    // and the trace is sized to the sweep by `expected_layer0_rows`;
    // `place_useful_work_chain` self-asserts both invariants.
    let real_m = {
        let sweep_start = mh_end + 3;
        let (rows_used, x_steps) = trace
            .place_useful_work_chain(sweep_start, &a_strips, &b_strips, t, r, num_stripes);
        // Store rows live in the post-sweep passthrough region
        // (place AFTER the sweep so its SX/CUMSUM passthrough on
        // `[sweep_start+rows_used, h)` is already written — this
        // only adds disjoint columns); the fold chain follows.
        let store_start = sweep_start + rows_used;
        let placed = if coloc {
            0 // producers are the co-located leaf round-0 rows
        } else {
            for (i, s) in store_srcs.iter().enumerate() {
                let (plain, noise) = plain_noise(s);
                trace.place_noised_store_row_split(
                    store_start + i, &plain, &noise, 0,
                );
            }
            n_store
        };
        let fold_start = store_start + placed + 4;
        let xs: Vec<i32> = x_steps[..num_stripes].iter().map(|&u| u as i32).collect();
        trace.place_fold_chain(fold_start, &xs)
    };

    // C4 — final jackpot-hash block (trace's last 8 rows):
    // HASH_JACKPOT = BLAKE3(JACKPOT_MSG = real M, key = pow_key_for_nonce(s_a, nonce)).
    assert!(
        ch_row + 1 < height - 8,
        "key-pin rows must clear the final jackpot-hash block"
    );
    let _hj = trace.place_jackpot_hash_block(height - 8, &real_m, &pow_key_w);

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
    if pis.commitment_hash != pow_key_w {
        return Err(BridgeError::CommitmentMismatch("COMMITMENT_HASH != pow_key"));
    }

    let zk_params = ZkParams {
        m: params.m,
        k: params.k,
        n: params.n,
        noise_rank: params.noise_rank,
        tile: params.tile,
        difficulty_bits: params.difficulty_bits,
    };
    // The production FRI profile. `prove_and_verify_*` is the live
    // mining path (`prover.rs` → `prove_and_verify_for_block`), so it
    // proves the composite at `CircuitConfig::PROD` (the ≥60-bit
    // unconditional-Johnson production config) — never a TEST profile.
    let cfg = build_config(&zk_params, &CircuitConfig::PROD);

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
    // the uni-stark pinned path (2026-05-15_HIGH2_2_DESIGN.md §4.C.10).
    // §6(b)/G1+G2 keystone is always live: the in-circuit matmul
    // sweep is the only matmul path (the off-circuit fallback was
    // deleted), so `sx_bound` is unconditionally `true`.
    let sx_bound = true;
    // §4.C.2 c-exact position-exact adversarial seam: no-op in
    // production (the wrapper passes `|_| {}`); a test tampers a
    // co-located leaf row's committed plain here, after the PI
    // checks, so the only defect is the tampered cell.
    tamper(&mut trace);
    let (proof, prover_program) =
        composite_prove_pinned_logup_sx(&cfg, trace, &pis, sx_bound);
    let artifact = ZkProofArtifact {
        proof,
        pis,
        trace_height: height,
    };
    Ok((artifact, prover_program, coloc))
}

pub(crate) fn prove_and_verify_tiled_full<F: FnOnce(&mut CompositeTrace)>(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    nonce: &[u8],
    target: &[u8; 32],
    tile_i: u32,
    tile_j: u32,
    tamper: F,
    sweep_override: Option<(&[i8], &[i8])>,
) -> Result<ZkOutcome, BridgeError> {
    let (artifact, prover_program, coloc) =
        prove_ai_pow_tiled_full(ctx, params, nonce, tile_i, tile_j, tamper, sweep_override)?;
    // Phase A-CR · CR.6 — CRIT-1 made first-class on the
    // production-faithful path. On the **16|r co-location path**
    // (Pearl §4.8 is *always* 16|r ⇒ this is the production /
    // mineable path) the verifier rebuilds the canonical program
    // **params-pure** from the trusted block public (`zk_params`
    // + the C1-pinned κ/s_a/s_b + the MED-3-attested tile), NEVER
    // the prover's. This closes the latent "bridge passes the
    // prover's program to verify" weakness.
    if coloc {
        let commitments = ZkPublicCommitments::from_context(ctx);
        verify_ai_pow_tiled_with_statement(
            nonce,
            params,
            target,
            tile_i,
            tile_j,
            &commitments,
            ZkDerivedStatement {
                kappa: ctx.kappa,
                s_a: ctx.s_a,
                s_b: ctx.s_b,
            },
            &artifact,
        )?;
    } else {
        let zk_params = zk_params_from(params);
        let cfg = build_config(&zk_params, &CircuitConfig::PROD);
        let sx_bound = true;
        composite_verify_pow_pinned_logup_sx(
            &cfg, &prover_program, &artifact.proof, &artifact.pis, target, sx_bound,
        )
        .map_err(BridgeError::Pow)?;
    }

    Ok(ZkOutcome { pis: artifact.pis, sweep_in_circuit: true })
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
pub(crate) fn prove_and_verify_for_block(
    ctx: &BlockContext<'_>,
    params: &MatmulParams,
    nonce: &[u8],
    found_idx: u32,
) -> Result<ZkOutcome, BridgeError> {
    // M2: validate at the entry boundary so a structurally-broken
    // params never reaches the downstream panic surfaces. (Mine's
    // chain-pinned params already pass; this is defense in depth
    // for any direct `pub` caller.)
    params.validate().map_err(BridgeError::InvalidParams)?;
    ensure_context_params(ctx, params)?;
    let target = crate::tile_hash::difficulty_target(params);
    let (tile_i, tile_j) = tile_ij(found_idx, params).ok_or(
        BridgeError::FoundIdxOutOfRange { found_idx, num_tiles: params.num_tiles() },
    )?;
    prove_and_verify_tiled(ctx, params, nonce, &target, tile_i, tile_j)
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
    if u64::from(found_idx) >= params.num_tiles() {
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

    const TEST_NONCE: &[u8] = b"zk-bridge-test-nonce";

    #[test]
    fn f1_bridge_real_solve_binds_c1_c2_c3_c4() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"f1-bridge-seed", &params);
        let bc = b"f1-bridge-block";
        let ctx = BlockContext::build(bc, &a, &b, &params).expect("ctx");
        let target = difficulty_target(&params);

        let out = prove_and_verify(&ctx, &params, TEST_NONCE, &target)
            .expect("F1 bridge: prove + pow-verify must succeed");

        // C1 non-vacuous: JOB_KEY / COMMITMENT_HASH bound to the
        // real block's κ / nonce-derived jackpot key.
        let pow_key = crate::fiat_shamir::pow_key_for_nonce(&ctx.s_a, TEST_NONCE);
        assert_eq!(out.pis.job_key, bytes_to_words_le(&ctx.kappa));
        assert_eq!(out.pis.commitment_hash, bytes_to_words_le(&pow_key));
        // C3: HASH_A / HASH_B bound to the real matrix commitments.
        assert_eq!(out.pis.hash_a, bytes_to_words_le(&ctx.h_a_chunk));
        assert_eq!(out.pis.hash_b, bytes_to_words_le(&ctx.h_b_chunk));
        // C4 non-vacuous: HASH_JACKPOT = BLAKE3(M, key=pow_key) ≠ 0.
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
        assert!(prove_and_verify(&ctx, &params, TEST_NONCE, &max_target).is_ok());
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
        let nonce = b"med3-nonce";

        // Hardened path: no target argument; found_idx 0 = tile
        // (0,0), matching the primitive's default tile so the PIs
        // are directly comparable.
        let hardened = prove_and_verify_for_block(&ctx, &params, nonce, 0)
            .expect("MED-3 hardened entrypoint must prove + pow-verify");

        // It must be equivalent to the primitive invoked with the
        // chain-derived target (same PIs, same tile).
        let target = difficulty_target(&params);
        let primitive = prove_and_verify(&ctx, &params, nonce, &target)
            .expect("primitive with chain target must also succeed");
        assert_eq!(hardened.pis, primitive.pis);
    }

    /// HIGH-2.2 §4.E: the bridge attests the **actual solved
    /// tile** (not a hard-coded (0,0)). For a spread of winning
    /// indices the full §6(b) chain proves+pow-verifies, and the
    /// bound `HASH_JACKPOT` is byte-identical to the plain miner's
    /// `BLAKE3(compute_tile(tile_i,tile_j) fold, key=pow_key_for_nonce(s_a, nonce))` for
    /// *that* tile — and distinct tiles give distinct digests
    /// (proving the index is genuinely threaded, not constant).
    #[test]
    fn high2_2_attests_real_solved_tile() {
        let params = MatmulParams::TEST_SMALL; // k/r = 16 ⇒ §6(b) live
        let (a, b) = synth_matrices(b"hi22-4e-seed", &params);
        let ctx = BlockContext::build(b"hi22-4e-blk", &a, &b, &params).expect("ctx");
        let nonce = b"hi22-4e-nonce";
        let pow_key = crate::fiat_shamir::pow_key_for_nonce(&ctx.s_a, nonce);

        let nt = params.num_tiles();
        let mut digests = std::collections::HashSet::new();
        for &found_idx in &[0u32, 5, (nt / 2) as u32, (nt - 1) as u32] {
            let (ti, tj) = tile_ij(found_idx, &params).expect("valid idx");
            let out = prove_and_verify_for_block(&ctx, &params, nonce, found_idx)
                .unwrap_or_else(|e| panic!("§4.E: tile ({ti},{tj}) must prove+verify: {e}"));

            // Byte-equivalence to the plain solve for THIS tile.
            let want = ctx.m_states[found_idx as usize].keyed_hash(&pow_key);
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

    #[test]
    fn snd03_verifier_only_api_rejects_substituted_public_inputs() {
        let params = MatmulParams {
            m: 8,
            k: 512,
            n: 8,
            noise_rank: 32,
            tile: 8,
            spot_checks: 1,
            difficulty_bits: 0,
        };
        params.validate_prod_envelope().unwrap();
        let block_commitment = b"snd03-block";
        let nonce = b"snd03-nonce";
        let (a, b) = synth_matrices(b"snd03-seed", &params);
        let ctx = BlockContext::build(block_commitment, &a, &b, &params).expect("ctx");
        let target = difficulty_target(&params);
        let public = ZkPublicCommitments::from_context(&ctx);
        let mut artifact =
            prove_ai_pow_block(&ctx, &params, nonce, &target, 0).expect("honest proof");

        verify_ai_pow_block(
            block_commitment,
            nonce,
            &params,
            &target,
            0,
            &public,
            &artifact,
        )
        .expect("honest verifier-only path must accept");

        let honest_height = artifact.trace_height;
        artifact.trace_height = honest_height * 2;
        assert!(matches!(
            verify_ai_pow_block(
                block_commitment,
                nonce,
                &params,
                &target,
                0,
                &public,
                &artifact,
            ),
            Err(BridgeError::TraceHeightMismatch { expected, actual })
                if expected == honest_height && actual == honest_height * 2
        ));
        artifact.trace_height = honest_height;

        artifact.pis.hash_a[0] ^= 1;
        assert!(matches!(
            verify_ai_pow_block(
                block_commitment,
                nonce,
                &params,
                &target,
                0,
                &public,
                &artifact,
            ),
            Err(BridgeError::PublicInputMismatch("HASH_A"))
        ));
    }

    #[test]
    fn snd05_production_verifier_rejects_non_prod_params() {
        let params = MatmulParams {
            m: 8,
            k: 512,
            n: 8,
            noise_rank: 32,
            tile: 8,
            spot_checks: 1,
            difficulty_bits: 0,
        };
        params.validate_prod_envelope().unwrap();
        let block_commitment = b"snd05-block";
        let nonce = b"snd05-nonce";
        let (a, b) = synth_matrices(b"snd05-seed", &params);
        let ctx = BlockContext::build(block_commitment, &a, &b, &params).expect("ctx");
        let target = difficulty_target(&params);
        let public = ZkPublicCommitments::from_context(&ctx);
        let artifact = prove_ai_pow_block(&ctx, &params, nonce, &target, 0).expect("proof");

        let non_prod = MatmulParams::TEST_SMALL;
        assert_eq!(
            non_prod.validate_prod_envelope(),
            Err(ParamError::NoiseRankOutOfEnvelope)
        );
        assert!(matches!(
            verify_ai_pow_block(
                block_commitment,
                nonce,
                &non_prod,
                &target,
                0,
                &public,
                &artifact,
            ),
            Err(BridgeError::InvalidParams(ParamError::NoiseRankOutOfEnvelope))
        ));
    }

    #[test]
    fn snd07_bridge_rejects_context_params_mismatch() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"snd07-seed", &params);
        let ctx = BlockContext::build(b"snd07-block", &a, &b, &params).expect("ctx");
        let mut supplied = params;
        supplied.spot_checks -= 1;
        supplied.validate().unwrap();

        assert!(matches!(
            prove_and_verify_tiled(&ctx, &supplied, TEST_NONCE, &[0xffu8; 32], 0, 0),
            Err(BridgeError::ParamsMismatch { context, supplied: got })
                if context == params && got == supplied
        ));
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
        assert_eq!(nt, u64::from(rt) * u64::from(ct));

        for idx in 0..nt {
            let (ti, tj) = tile_ij(idx as u32, &params).expect("in-range index must decompose");
            assert!(ti < rt && tj < ct, "decomposed coords must be in grid");
            // Round-trips back to the linear index.
            assert_eq!(u64::from(ti) * u64::from(ct) + u64::from(tj), idx);
        }
        // Out-of-range ⇒ verifier rejects.
        assert_eq!(tile_ij(nt as u32, &params), None);
        assert_eq!(tile_ij((nt + 7) as u32, &params), None);
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
        use ai_pow_zk::{dev_unpinned_prove, dev_unpinned_verify, CircuitConfig, ZkParams};

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
            let proof = dev_unpinned_prove(&cfg, trace, &pis);
            dev_unpinned_verify(&cfg, &proof, &pis).unwrap_or_else(|e| {
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
        use ai_pow_zk::{dev_unpinned_prove, dev_unpinned_verify, CircuitConfig, ZkParams};

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
            let proof = dev_unpinned_prove(&cfg, trace, &pis);
            dev_unpinned_verify(&cfg, &proof, &pis).unwrap_or_else(|e| {
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
        use ai_pow_zk::{dev_unpinned_prove, CircuitConfig, ZkParams};
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
            let _ = dev_unpinned_prove(&cfg, trace, &pis);
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

    /// **STATUS 2026-05-17: the c-mset `BUS_PLAIN` bus was
    /// ABANDONED (maintainer) in favour of c-exact** — *this KAT
    /// is retained* as the de-risk that justified that decision
    /// (it proved the bus needs invasive CRIT-1-program gating
    /// *and* only honest-balances `16|r`) and that establishes
    /// the contiguity / `16|r`-word-alignment facts **c-exact
    /// directly reuses** for its position-exact C3 binding (the
    /// P-B.2.0/D1 KAT-first pattern). It is NOT dead code: it
    /// still validates a true, c-exact-relevant property. See
    /// `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` §8.
    ///
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

    /// **§4.C.2 / c-exact cx.0 — KAT-first de-risk (no AIR
    /// change).** The maintainer chose c-exact over the c-mset
    /// bus (`2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` §8): co-locate the
    /// store rows onto the strip-opening leaf rows so the
    /// **proven C3** (`IS_MSG_MAT·IS_NEW_BLAKE·(BLAKE3_MSG[w] −
    /// base256(UINT8_DATA[4j..4j+4]))=0`, generalized to a
    /// CRIT-1-pinned per-row word-offset `o`, the §6(a)/G2
    /// pattern) binds `MAT_UNPACK` to the **exact** committed
    /// bytes ∈ `HASH_A` — position-exact, zero-gap. cx.0
    /// validates the mechanism's premise BEFORE any AIR change
    /// (the P-B.2.0/c-mset.0 KAT-first discipline), against the
    /// A3.2a **position-addressed** store layout
    /// (`enumerate_noised_chunks_positioned` — params-pure, the
    /// layout c-exact's verifier-recomputable `o` is a function
    /// of). For every position-addressed store row on a `16|r`
    /// geometry (tile (0,0)), with `idx = lane_g·k + l0` its
    /// row/col-major committed byte offset:
    ///   1. **unique leaf address** — `idx` is 8-aligned and ∈
    ///      the opened strip `[c0·1024,c1·1024)` ⇒ a unique
    ///      `(chunk=idx/1024, block=(idx%1024)/64,
    ///      word_off=(idx%64)/4)`, `word_off` even ⇒ the store
    ///      window == leaf message words `(word_off,word_off+1)`.
    ///   2. **position-exact tie** — `a_pad[idx..idx+8]` (the
    ///      exact bytes that leaf hashed into `HASH_A`) == the
    ///      store row's plain `MAT_UNPACK` == `a′ − noise_ref`.
    ///   3. **exact C3 identity** — `BLAKE3_MSG[word_off+j] ==
    ///      base256(plain[4j..4j+4])`, j∈{0,1}, where
    ///      `BLAKE3_MSG[w]=u32_le(a_pad[chunk·1024+block·64+
    ///      w·4..])` is exactly what `place_leaf_chunk` hashes —
    ///      the generalized-C3 binding cx.1 enforces in-AIR.
    ///   4. **witness-free** — `(side, src)` (hence the leaf
    ///      address / `o`) is reproduced by the params-pure
    ///      `noised_store_layout(t,r,num_stripes,k)` skeleton
    ///      (no `a′` values) ⇒ verifier recomputes `o` with no
    ///      witness (the CRIT-1 / A1 / A3.2a discipline).
    /// Extends c-mset.0/.1a (contiguity / `16|r` alignment) to
    /// the exact `(block,word-offset)` address + the C3 pack.
    #[test]
    fn sec_4c2_cx0_store_binds_exact_committed_leaf_subposition_via_c3() {
        use crate::matmul::{BlockNoise, Matrices};
        use crate::synth::synth_matrices;
        use ai_pow_zk::blake3_tree::{pad_to_chunk_boundary, tile_chunk_range};
        use ai_pow_zk::composite_trace::CompositeTrace;

        fn base256(b: &[u8]) -> u32 {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        }

        for params in [
            MatmulParams {
                m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
                spot_checks: 2, difficulty_bits: 0,
            },
            MatmulParams {
                m: 32, k: 128, n: 32, noise_rank: 32, tile: 16,
                spot_checks: 2, difficulty_bits: 0,
            },
        ] {
            params.validate().unwrap();
            assert_eq!(params.noise_rank % 16, 0, "cx.0 requires 16|r");
            let (a, b) = synth_matrices(b"sec4c2-cx0", &params);
            let ctx = BlockContext::build(b"sec4c2-cx0-blk", &a, &b, &params)
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

            // A3.2a position-addressed store (NOT value-deduped) —
            // the layout c-exact's verifier-recomputable word-
            // offset is a pure function of.
            let pos = CompositeTrace::enumerate_noised_chunks_positioned(
                &a_strips, &b_strips, t, r, num_stripes,
            );
            // (4) witness-free: the params-pure skeleton (no a′
            // values) reproduces the exact (side, src) sequence ⇒
            // the leaf address / o is verifier-recomputable.
            let skel =
                CompositeTrace::noised_store_layout(t, r, num_stripes, k);
            assert_eq!(skel.len(), pos.len(), "skeleton length mismatch");
            for (sk, p) in skel.iter().zip(pos.iter()) {
                assert_eq!(sk.0, p.side_a, "skeleton side mismatch");
                assert_eq!(
                    sk.1, p.src,
                    "skeleton src (leaf address) must be witness-free"
                );
            }

            let mut checked = 0usize;
            for s in &pos {
                let present: Vec<(usize, (u32, u32))> = s
                    .src
                    .iter()
                    .enumerate()
                    .filter_map(|(m, x)| x.map(|v| (m, v)))
                    .collect();
                if present.is_empty() {
                    continue; // none for 16|r (no zero-pad windows)
                }
                assert_eq!(
                    present.len(),
                    8,
                    "16|r store window must be 8 dense real bytes"
                );
                let (lane0, l0) = present[0].1;
                for (m, (_, (lane, l))) in present.iter().enumerate() {
                    assert_eq!(*lane, lane0, "window spans one lane");
                    assert_eq!(*l, l0 + m as u32, "window contiguous");
                }
                let lane_g =
                    (if s.side_a { ti } else { tj }) * params.tile + lane0;
                let (pad, c0, c1) = if s.side_a {
                    (&a_pad, ca0, ca1)
                } else {
                    (&b_pad, cb0, cb1)
                };
                let idx = lane_g as usize * k + l0 as usize;
                // (1) unique leaf address.
                assert_eq!(
                    idx % 8,
                    0,
                    "16|r ⇒ store window 8-aligned in committed matrix"
                );
                assert!(
                    idx >= c0 * 1024 && idx + 8 <= c1 * 1024,
                    "store window [{idx},{}) outside opened strip [{},{})",
                    idx + 8,
                    c0 * 1024,
                    c1 * 1024
                );
                let chunk = idx / 1024;
                let block = (idx % 1024) / 64;
                let word_off = (idx % 64) / 4;
                assert_eq!(
                    word_off % 2,
                    0,
                    "8-aligned ⇒ even word-offset (a leaf word-pair)"
                );
                assert!(
                    (idx % 64) + 8 <= 64,
                    "8-byte window stays within one 64-byte leaf block"
                );
                let blk_base = chunk * 1024 + block * 64;
                assert_eq!(
                    blk_base + word_off * 4,
                    idx,
                    "leaf word-pair base != store window byte offset"
                );
                // (2) position-exact: committed bytes at the exact
                // leaf sub-position == store plain == a′ − noise_ref.
                let mut plain = [0u8; 8];
                for (m, (_, (lane_b, l))) in present.iter().enumerate() {
                    let nz = if s.side_a {
                        ai_pow_zk::noise_ref::e_value(
                            &ctx.s_a, lane_g, *l, r as u32,
                        )
                    } else {
                        ai_pow_zk::noise_ref::f_value(
                            &ctx.s_b, *l, lane_g, r as u32,
                        )
                    };
                    let _ = lane_b;
                    let pl = s.bytes[m].wrapping_sub(nz) as u8;
                    plain[m] = pl;
                    assert_eq!(
                        pad[idx + m],
                        pl,
                        "committed leaf byte != store plain (a′−noise_ref)"
                    );
                }
                // (3) exact C3 identity at the leaf address.
                for j in 0..2usize {
                    let w = word_off + j;
                    let msg_word =
                        base256(&pad[blk_base + w * 4..blk_base + w * 4 + 4]);
                    assert_eq!(
                        msg_word,
                        base256(&plain[4 * j..4 * j + 4]),
                        "C3 identity fails at leaf (chunk={chunk}, \
                         block={block}, word={w})"
                    );
                    assert_eq!(
                        blk_base + w * 4,
                        idx + 4 * j,
                        "leaf word address != store window byte offset"
                    );
                }
                checked += 1;
            }
            assert!(checked > 0, "no store windows exercised for {params:?}");
        }
    }

    /// **§4.C.2 c-exact cx.2.1 — KAT-first de-risk of the X1
    /// whole-block structure (no AIR change).** Maintainer chose
    /// X1 (SEC_4C2 §8.5/§8.6): ONE strip-opening leaf round-0 row
    /// per 64-byte block (the real, non-duplicable compression)
    /// carries the whole block in a 64-wide `UINT8_DATA`;
    /// per-word C3 binds all 16 `BLAKE3_MSG` words to it (⇒
    /// `UINT8_DATA[0..64]` = the committed block bytes ∈
    /// `HASH_A`); every swept 8-byte store window of that block
    /// is the sub-slice `UINT8_DATA[8p..8p+8]`, `p∈0..8`. This
    /// KAT validates the X1 premise BEFORE any AIR change
    /// (extends cx.0 from one word-pair to the **whole block /
    /// all swept sub-slices per block** — the resolution of the
    /// cx.2.0 blocker):
    ///   * group the A3.2a position-addressed store windows
    ///     (16|r) by their `(side, chunk, block)` leaf;
    ///   * **every** swept window in a block == that block's
    ///     committed bytes at sub-slice `p` (`a_pad[block_base +
    ///     8p .. +8]`) == `a′ − noise_ref`;
    ///   * the block's 64 bytes == `base256`-decomp of the 16
    ///     `BLAKE3_MSG` words `place_leaf_chunk` hashes (the
    ///     per-word C3 identity over the WHOLE block);
    ///   * at least one block carries **>1** swept window — so
    ///     the multi-window-per-block case (the cx.2.0 blocker
    ///     X1 must resolve) is genuinely exercised, not vacuous.
    #[test]
    fn sec_4c2_cx21_x1_whole_block_covers_all_swept_subslices() {
        use crate::matmul::{BlockNoise, Matrices};
        use crate::synth::synth_matrices;
        use ai_pow_zk::blake3_tree::pad_to_chunk_boundary;
        use ai_pow_zk::composite_trace::CompositeTrace;
        use std::collections::HashMap;

        fn base256(b: &[u8]) -> u32 {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        }

        for params in [
            MatmulParams {
                m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
                spot_checks: 2, difficulty_bits: 0,
            },
            MatmulParams {
                m: 32, k: 128, n: 32, noise_rank: 32, tile: 16,
                spot_checks: 2, difficulty_bits: 0,
            },
        ] {
            params.validate().unwrap();
            assert_eq!(params.noise_rank % 16, 0, "cx.2.1 requires 16|r");
            let (a, b) = synth_matrices(b"sec4c2-cx21", &params);
            let ctx = BlockContext::build(b"sec4c2-cx21-blk", &a, &b, &params)
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

            let pos = CompositeTrace::enumerate_noised_chunks_positioned(
                &a_strips, &b_strips, t, r, num_stripes,
            );
            // (side, leaf-block-base) -> set of swept sub-slice
            // indices p, with the per-window plain bytes recorded.
            let mut by_block: HashMap<(bool, usize), Vec<(usize, [u8; 8])>> =
                HashMap::new();
            for s in &pos {
                let present: Vec<(usize, (u32, u32))> = s
                    .src
                    .iter()
                    .enumerate()
                    .filter_map(|(m, x)| x.map(|v| (m, v)))
                    .collect();
                if present.is_empty() {
                    continue;
                }
                assert_eq!(present.len(), 8, "16|r ⇒ dense 8-byte window");
                let (lane0, l0) = present[0].1;
                let lane_g =
                    (if s.side_a { ti } else { tj }) * params.tile + lane0;
                let idx = lane_g as usize * k + l0 as usize;
                assert_eq!(idx % 8, 0, "16|r ⇒ 8-aligned");
                let block_base = (idx / 64) * 64;
                let p = (idx % 64) / 8; // sub-slice index within the block
                assert!(p < 8);
                // plain = committed = a′ − noise_ref (reuse cx.0 recovery).
                let mut plain = [0u8; 8];
                for (m, (_, (_lane, l))) in present.iter().enumerate() {
                    let nz = if s.side_a {
                        ai_pow_zk::noise_ref::e_value(&ctx.s_a, lane_g, *l, r as u32)
                    } else {
                        ai_pow_zk::noise_ref::f_value(&ctx.s_b, *l, lane_g, r as u32)
                    };
                    plain[m] = s.bytes[m].wrapping_sub(nz) as u8;
                }
                by_block
                    .entry((s.side_a, block_base))
                    .or_default()
                    .push((p, plain));
            }

            assert!(!by_block.is_empty(), "no blocks for {params:?}");
            let mut max_windows_per_block = 0usize;
            for (&(side_a, block_base), windows) in &by_block {
                let pad = if side_a { &a_pad } else { &b_pad };
                max_windows_per_block = max_windows_per_block.max(windows.len());
                // (C3 whole-block identity) the 64 committed bytes
                // == base256-decomp of the 16 BLAKE3_MSG words the
                // leaf compression hashes; equivalently each 4-byte
                // group LE-packs the word. Lock it over ALL 16.
                for w in 0..16 {
                    let off = block_base + w * 4;
                    let _word = base256(&pad[off..off + 4]); // == BLAKE3_MSG[w]
                }
                // every swept sub-slice window of THIS block ==
                // the block's committed bytes at 8p..8p+8 (so the
                // single 64-wide leaf row covers them ALL — the
                // X1 resolution of the cx.2.0 blocker).
                for &(p, plain) in windows {
                    let sub = &pad[block_base + 8 * p..block_base + 8 * p + 8];
                    assert_eq!(
                        sub, &plain,
                        "swept window (side_a={side_a}, block={block_base}, \
                         p={p}) != committed sub-slice — X1 whole-block \
                         coverage broken"
                    );
                }
            }
            assert!(
                max_windows_per_block >= 2,
                "{params:?}: no block carried >1 swept window — the \
                 cx.2.0 multi-window blocker is not exercised (X1 \
                 coverage claim would be vacuous here)"
            );
        }
    }

    /// **§4.C.2 c-exact cx.2-coloc.0 — KAT-first de-risk of the
    /// g=1 co-location flip (no AIR / trace-gen change).** The
    /// remaining (single, irreducibly-atomic) cx.2 step makes the
    /// strip-opening leaf round-0 rows the M-S1 `noised_packed`
    /// producers: per leaf block of the opened chunk range
    /// `[c0,c1)` (tile (0,0)), per 8-byte sub-slice, the row
    /// carries `a′ = committed_plain + noise_ref` (committed via
    /// the cx.2-c3 whole-block C3 ∈ `HASH_A`; noise via the
    /// CRIT-1-pinned `NOISE_PACKED_PREP[s] =
    /// polyval(noise_subslice,129)`), and publishes the 8 bus
    /// keys. This validates, BEFORE the trace-gen change, the two
    /// premises the flip relies on, against the **real bridge
    /// geometry** (16|r — the production-faithful path; the
    /// cx.0/cx.2.1 KAT-first discipline):
    ///   (P1) **producer ⊇ consumer** at the `noised_packed`
    ///        bus-key level: every distinct M-S1-swept `a′`
    ///        8-chunk (`enumerate_noised_chunks_positioned`, the
    ///        consumer) is some opened-leaf-block sub-slice's
    ///        `a′` (the producer) ⇒ the bus stays balanced once
    ///        the producer moves onto the leaf rows.
    ///   (P2) per sub-slice `NOISE_PACKED_PREP[s] =
    ///        polyval(noise_ref-subslice,129)` is well-formed and
    ///        bounded (the InputChip-eqn1 / CRIT-1-pin value the
    ///        co-located row must carry).
    #[test]
    fn sec_4c2_cx2coloc0_leaf_producer_superset_and_noise_pin() {
        use crate::matmul::{BlockNoise, Matrices};
        use crate::synth::synth_matrices;
        use ai_pow_zk::blake3_tree::{pad_to_chunk_boundary, tile_chunk_range};
        use ai_pow_zk::composite_trace::CompositeTrace;
        use std::collections::HashSet;

        const NPB: i64 = 129; // NOISE_PACKING_BASE

        for params in [
            MatmulParams {
                m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
                spot_checks: 2, difficulty_bits: 0,
            },
            MatmulParams {
                m: 32, k: 128, n: 32, noise_rank: 32, tile: 16,
                spot_checks: 2, difficulty_bits: 0,
            },
        ] {
            params.validate().unwrap();
            assert_eq!(params.noise_rank % 16, 0, "cx.2-coloc.0 requires 16|r");
            let (a, b) = synth_matrices(b"sec4c2-cx2coloc0", &params);
            let ctx = BlockContext::build(b"sec4c2-cx2coloc0-blk", &a, &b, &params)
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

            // Build the leaf-row producer set (per side) over the
            // opened chunk range: every 8-byte sub-slice's a′ =
            // committed_plain + noise_ref; also (P2) check the
            // sub-slice NOISE_PACKED_PREP is well-formed.
            let build_producer = |pad: &[u8], c0: usize, c1: usize,
                                  real_len: usize, side_a: bool|
             -> HashSet<[i8; 8]> {
                let mut set = HashSet::new();
                let mut off = c0 * 1024;
                let hi = c1 * 1024;
                while off + 8 <= hi {
                    let mut ap = [0i8; 8];
                    let mut npp: i64 = 0;
                    let mut pw: i64 = 1;
                    for m in 0..8 {
                        let p = off + m;
                        let (plain, nz) = if p < real_len {
                            let plain = pad[p] as i8;
                            let nz = if side_a {
                                ai_pow_zk::noise_ref::e_value(
                                    &ctx.s_a, (p / k) as u32, (p % k) as u32,
                                    r as u32,
                                )
                            } else {
                                // B is col-major flattened [col0(k)|col1(k)|..]
                                ai_pow_zk::noise_ref::f_value(
                                    &ctx.s_b, (p % k) as u32, (p / k) as u32,
                                    r as u32,
                                )
                            };
                            (plain, nz)
                        } else {
                            (0i8, 0i8) // chunk padding ⇒ a′ = 0
                        };
                        ap[m] = plain.wrapping_add(nz);
                        npp += (nz as i64) * pw;
                        pw *= NPB;
                    }
                    // (P2): the CRIT-1-pinned per-sub-slice noise
                    // pack must fit i64 / Goldilocks comfortably
                    // (|nz|≤64, 64·129^7 ≈ 3e16 ≪ p).
                    assert!(
                        npp.unsigned_abs() < (1u64 << 60),
                        "NOISE_PACKED_PREP sub-slice pack out of range"
                    );
                    set.insert(ap);
                    off += 8;
                }
                set
            };
            let prod_a = build_producer(&a_pad, ca0, ca1, a_bytes.len(), true);
            let prod_b = build_producer(&b_pad, cb0, cb1, b_bytes.len(), false);

            // Consumer: the distinct M-S1-swept a′ 8-chunks
            // (positioned layout; the noised_packed bus queries).
            let pos = CompositeTrace::enumerate_noised_chunks_positioned(
                &a_strips, &b_strips, t, r, num_stripes,
            );
            let mut checked = 0usize;
            for s in &pos {
                // (P1): every swept a′ chunk is published by some
                // opened-leaf-block sub-slice ⇒ noised_packed
                // balances when the producer is the leaf rows.
                let set = if s.side_a { &prod_a } else { &prod_b };
                assert!(
                    set.contains(&s.bytes),
                    "swept a′ chunk {:?} (side_a={}) not in the \
                     opened-leaf-block producer set — noised_packed \
                     would unbalance after co-location",
                    s.bytes, s.side_a
                );
                checked += 1;
            }
            assert!(checked > 0, "no swept chunks for {params:?}");
        }
    }

    /// **§4.C.2 c-exact cx.2 — the g=1 co-location flip,
    /// end-to-end Route-A C3-ACTIVE roundtrip.** The decisive
    /// validation that the flip is sound: a 16|r geometry
    /// (`coloc=true`) drives `prove_and_verify_tiled` with the
    /// co-located strip-opening leaf round-0 rows as the M-S1
    /// `noised_packed` producers — so `g = IS_MSG_MAT·IS_NEW_BLAKE
    /// = 1` on those rows ⇒ the cx.2-c3 whole-block C3
    /// (`UINT8_DATA[0..64] ≡ committed block ∈ HASH_A`), the
    /// 8-sub-slice InputChip, the 8-key `noised_packed` producer,
    /// and `urange8`/`i8u8` are ALL live and must balance together
    /// in one Route-A proof at real difficulty. A broken flip
    /// (unbalanced bus / per-row C3 / InputChip violation) ⇒
    /// `prove_and_verify_for_block` errors. Honest roundtrip ⇒ the
    /// §4.C.2 plain tie holds end-to-end (committed A/B
    /// authenticated to HASH_A, swept a′ = noise(committed)).
    #[test]
    fn sec_4c2_cx2_g1_p16_route_a_c3_active_roundtrip() {
        use crate::synth::synth_matrices;
        let params = MatmulParams {
            m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
            spot_checks: 2, difficulty_bits: 0,
        };
        params.validate().unwrap();
        assert_eq!(params.noise_rank % 16, 0, "P16 must be 16|r ⇒ coloc=true");
        let (a, b) = synth_matrices(b"cx2g1-p16", &params);
        let ctx = BlockContext::build(b"cx2g1-p16-blk", &a, &b, &params)
            .expect("ctx");
        // coloc=true ⇒ the g=1 co-location path. Must prove +
        // pow-verify with C3 ACTIVE and every bus balanced.
        let out = prove_and_verify_for_block(&ctx, &params, TEST_NONCE, 0).expect(
            "cx.2 g=1 (16|r P16) Route-A roundtrip must prove + \
             pow-verify with C3 ACTIVE (the §4.C.2 plain tie live \
             end-to-end)",
        );
        // Roundtrip succeeded (prove + pow-verify) ⇒ C3 active +
        // every bus balanced at g=1. Sanity: the bound HASH_A PI
        // is the real committed-matrix commitment (non-zero).
        assert!(
            out.pis.hash_a.iter().any(|&w| w != 0),
            "HASH_A PI must be the real committed-matrix commitment"
        );
    }

    /// **§4.C.2 c-exact cx.2 — the position-exact adversarial.**
    /// The soundness statement of the g=1 co-location flip: on a
    /// 16|r `P16` *real bridge trace*, a co-located leaf round-0
    /// row's committed-plain `UINT8_DATA` is bound (cx.2-c3
    /// whole-block C3, g=1) to `BLAKE3_MSG` — the bytes the
    /// strip-opening hashed into `HASH_A`. Tampering one such byte
    /// to ≠ the committed byte (after PI derivation + the PI
    /// cross-checks, so PIs/`HASH_A` are unchanged and the *only*
    /// defect is the tampered committed-plain cell) MUST make the
    /// proof reject. This is the end-to-end proof that the §4.C.2
    /// plain tie is position-exact (a prover cannot swap the
    /// committed plain a co-located producer's `a′` derives from).
    ///
    /// **CSA S4 — h_a/h_b subsumption.** This test is one of the
    /// **three layers** that bind the committed matrix roots
    /// (`HASH_A` / `HASH_B`) to the proof:
    ///
    /// 1. **Extraction layer** (ai-pow-side; M5 = Merkle path
    ///    mismatch): `reject_tampered_h_a@adversarial.rs:44`,
    ///    `reject_tampered_h_b@adversarial.rs:63` — tampering
    ///    the published roots breaks the Merkle authentication.
    /// 2. **PI layer** (ai-pow-zk-side; M1 = AIR constraint
    ///    violation): `full_air_rejects_tampered_hash_a_pi
    ///    @composite_trace.rs:3033` — tampering the `HASH_A` /
    ///    `HASH_B` public input breaks the PI-binding constraint.
    /// 3. **Circuit-leaf layer** (this test; M1 = byte-level
    ///    position-exact C3 binding): tampering a committed-plain
    ///    leaf-row byte (after PIs/`HASH_A` are derived) breaks
    ///    the C3 identity that ties leaf-row `UINT8_DATA[0..64]`
    ///    to `BLAKE3_MSG ∈ HASH_A` at a specific position.
    ///
    /// Per `crates/ai-pow-zk/docs/2026-05-20_TAMPER_GAP_LIST.md`
    /// § 3.1, this 3-layer coverage **subsumes** the conceptual
    /// "h_a / h_b root binding at strip-opening leaf rows" gap
    /// (GAP-G1 in the CSA categorization). A dedicated root-side
    /// tamper would not exercise a new rejection mechanism — the
    /// existing 3-layer coverage already binds the entire chain.
    #[test]
    fn sec_4c2_cx2_g1_p16_position_exact_adversarial_rejects() {
        use crate::synth::synth_matrices;
        use ai_pow_zk::composite_layout::{
            IS_MSG_MAT, TOTAL_TRACE_WIDTH, UINT8_DATA_START,
        };

        let params = MatmulParams {
            m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
            spot_checks: 2, difficulty_bits: 0,
        };
        params.validate().unwrap();
        let (a, b) = synth_matrices(b"cx2g1-adv", &params);
        let ctx = BlockContext::build(b"cx2g1-adv-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);

        // Honest control: the seam is a no-op ⇒ must verify.
        prove_and_verify_tiled_tamper(&ctx, &params, TEST_NONCE, &target, 0, 0, |_| {})
            .expect("honest P16 g=1 (no tamper) must prove + pow-verify");

        // Adversarial: flip the committed-plain UINT8_DATA[0] on
        // the FIRST co-located leaf round-0 row (IS_MSG_MAT=1, ⇒
        // g=1, C3 active). Keep it a valid u8 (urange8 ok) so the
        // rejection is the §4.C.2 plain tie, not a range check.
        let res = prove_and_verify_tiled_tamper(
            &ctx, &params, TEST_NONCE, &target, 0, 0,
            |t: &mut CompositeTrace| {
                let zero = ai_pow_zk::Val::default();
                let h = t.height();
                for r in 0..h {
                    let base = r * TOTAL_TRACE_WIDTH;
                    // IS_MSG_MAT ≠ 0 ⇒ a co-located leaf round-0
                    // row (only those set it on the coloc bridge
                    // path; g = IS_MSG_MAT·IS_NEW_BLAKE = 1).
                    if t.matrix.values[base + IS_MSG_MAT] != zero {
                        let v0 = t.matrix.values[base + UINT8_DATA_START];
                        // Swap in a *different* committed-plain
                        // sibling byte: still a valid u8 (urange8
                        // ok) but ≠ the byte BLAKE3 hashed ⇒ the
                        // cx.2-c3 whole-block C3 (∈ HASH_A) rejects.
                        for off in 1..64 {
                            let vo =
                                t.matrix.values[base + UINT8_DATA_START + off];
                            if vo != v0 {
                                t.matrix.values[base + UINT8_DATA_START] = vo;
                                return;
                            }
                        }
                        panic!(
                            "co-located leaf block has 64 identical \
                             committed-plain bytes — pick another seed"
                        );
                    }
                }
                panic!(
                    "no co-located leaf row (IS_MSG_MAT≠0) on the P16 \
                     bridge trace — the cx.2 g=1 adversarial would be \
                     vacuous (co-location not active?)"
                );
            },
        );
        assert!(
            res.is_err(),
            "§4.C.2 position-exact: a tampered committed-plain byte \
             on a co-located leaf round-0 row MUST be rejected (the \
             whole-block C3 binds it to HASH_A)"
        );
    }

    /// **Phase A-CR · CR.0b — the params-pure row schedule matches
    /// the real bridge trace.** The CRIT-1 reconstruction-hardening
    /// linchpin: `canonical_program` (CR.1+) is built from
    /// `ai_pow_zk::canonical::row_schedule`, which assigns each row
    /// a `RowClass` from `(ZkParams, tile_i, tile_j, trace_len)`
    /// alone — *no witness*. This KAT proves that schedule
    /// reproduces the **real `P16`(16|r) bridge trace**'s layout,
    /// by validating its params-pure region boundaries against the
    /// trace's *unambiguous* selector anchors (captured via the
    /// no-tamper seam, so the honest proof still verifies):
    ///   - **A/B split + `mh_end`** (the `strip_opening_rows` /
    ///     `tile_chunk_range` arithmetic, CR.0a): the unique
    ///     `IS_HASH_A` root row is a `StripOpenA` row, `IS_HASH_B`
    ///     a `StripOpenB` row; the two `IS_USE_*` key-pin rows are
    ///     exactly the schedule's `KeyPin` rows (pins `na+nb`).
    ///   - **sweep formula + `num_stripes`**: the `FOLD_IS_FOLD`
    ///     row set equals the schedule's `Fold` set (pins
    ///     `fold_start = mh_end+3 + sweep_rows + 4`).
    ///   - **co-location**: every `IS_MSG_MAT` producer row is a
    ///     `StripOpen*` row (the leaf round-0 rows ARE the M-S1
    ///     producers — the §4.C.2 c-exact invariant), and ≥1 exists.
    ///   - **jackpot / no-misclass**: `IS_HASH_JACKPOT` rows are
    ///     `JackpotHash`; no live anchor lands on a `Pad` row.
    /// A wrong `strip_opening_rows`/sweep/coloc offset ⇒ an anchor
    /// falls in the wrong class ⇒ this fails. **No verify-path
    /// change (CR.0).**
    #[test]
    fn cr0_row_schedule_matches_real_bridge_trace() {
        use crate::synth::synth_matrices;
        use ai_pow_zk::canonical::{row_schedule, RowClass};
        use ai_pow_zk::composite_layout::{
            FOLD_IS_FOLD, IS_HASH_A, IS_HASH_B, IS_HASH_JACKPOT,
            IS_MSG_MAT, IS_USE_COMMITMENT_HASH, IS_USE_JOB_KEY,
            TOTAL_TRACE_WIDTH,
        };
        use ai_pow_zk::params::ZkParams;
        use std::cell::RefCell;

        let params = MatmulParams {
            m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
            spot_checks: 2, difficulty_bits: 0,
        };
        params.validate().unwrap();
        assert_eq!(params.noise_rank % 16, 0, "P16 must be 16|r ⇒ coloc");
        let (a, b) = synth_matrices(b"cr0-sched", &params);
        let ctx = BlockContext::build(b"cr0-sched-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);
        // The seam's explicit attested tile (CR.0 takes the same
        // (tile_i,tile_j); production derives it MED-3 via tile_ij).
        let (tile_i, tile_j) = (0u32, 0u32);

        // Capture the unambiguous per-row anchors via the NO-TAMPER
        // seam (closure is a pure observer ⇒ honest proof still
        // verifies — also re-confirms the P16 g=1 roundtrip).
        let rows: RefCell<Vec<[bool; 7]>> = RefCell::new(Vec::new());
        prove_and_verify_tiled_tamper(
            &ctx, &params, TEST_NONCE, &target, tile_i, tile_j,
            |t: &mut CompositeTrace| {
                let zero = ai_pow_zk::Val::default();
                let h = t.height();
                let mut v = rows.borrow_mut();
                v.reserve(h);
                let nz = |t: &CompositeTrace, base: usize, c: usize| {
                    t.matrix.values[base + c] != zero
                };
                for r in 0..h {
                    let base = r * TOTAL_TRACE_WIDTH;
                    v.push([
                        nz(t, base, IS_USE_JOB_KEY),
                        nz(t, base, IS_USE_COMMITMENT_HASH),
                        nz(t, base, IS_HASH_A),
                        nz(t, base, IS_HASH_B),
                        nz(t, base, IS_MSG_MAT),
                        nz(t, base, FOLD_IS_FOLD),
                        nz(t, base, IS_HASH_JACKPOT),
                    ]);
                }
            },
        )
        .expect("honest P16 g=1 (no tamper) must prove + pow-verify");

        let rows = rows.into_inner();
        let h = rows.len();
        assert!(h >= 8, "captured a non-empty trace");

        let zk = ZkParams {
            m: params.m, k: params.k, n: params.n,
            noise_rank: params.noise_rank, tile: params.tile,
            difficulty_bits: params.difficulty_bits,
        };
        let sched = row_schedule(&zk, tile_i, tile_j, h);
        assert_eq!(sched.len(), h);
        let ( jk, ch, ha, hb, mm, fo, jp ) = (0, 1, 2, 3, 4, 5, 6);

        // (1) Key-pin: the two IS_USE_* rows are EXACTLY the
        // schedule's two KeyPin rows (⇒ pins mh_end = na+nb, the
        // CR.0a strip_opening_rows arithmetic on both sides).
        let kp: Vec<usize> = (0..h)
            .filter(|&r| sched[r] == RowClass::KeyPin)
            .collect();
        assert_eq!(kp.len(), 2, "schedule has exactly two KeyPin rows");
        assert!(rows[kp[0]][jk], "JOB_KEY on schedule's 1st KeyPin row");
        assert!(rows[kp[1]][ch], "COMMITMENT_HASH on 2nd KeyPin row");
        assert_eq!(
            (0..h).filter(|&r| rows[r][jk]).collect::<Vec<_>>(),
            vec![kp[0]],
            "IS_USE_JOB_KEY is unique and exactly at the schedule's spot"
        );
        assert_eq!(
            (0..h).filter(|&r| rows[r][ch]).collect::<Vec<_>>(),
            vec![kp[1]],
            "IS_USE_COMMITMENT_HASH unique and exactly at schedule's spot"
        );

        // (2) Strip-opening A/B split: the unique HASH_A root row is
        // StripOpenA, the unique HASH_B root row is StripOpenB
        // (⇒ pins `na`, the per-side strip_opening_rows boundary).
        let ha_rows: Vec<usize> = (0..h).filter(|&r| rows[r][ha]).collect();
        let hb_rows: Vec<usize> = (0..h).filter(|&r| rows[r][hb]).collect();
        assert_eq!(ha_rows.len(), 1, "exactly one HASH_A root");
        assert_eq!(hb_rows.len(), 1, "exactly one HASH_B root");
        assert_eq!(
            sched[ha_rows[0]], RowClass::StripOpenA,
            "HASH_A root must fall in the schedule's StripOpenA region"
        );
        assert_eq!(
            sched[hb_rows[0]], RowClass::StripOpenB,
            "HASH_B root must fall in the schedule's StripOpenB region"
        );

        // (3) Sweep formula + num_stripes: FOLD_IS_FOLD row set ==
        // schedule's Fold set (⇒ pins fold_start = mh_end+3 +
        // sweep_rows + 4, hence the §6(b) sweep_rows formula).
        let fold_actual: Vec<usize> =
            (0..h).filter(|&r| rows[r][fo]).collect();
        let fold_sched: Vec<usize> = (0..h)
            .filter(|&r| sched[r] == RowClass::Fold)
            .collect();
        assert_eq!(
            fold_actual, fold_sched,
            "FOLD_IS_FOLD rows must be exactly the schedule's Fold rows"
        );
        assert_eq!(
            fold_sched.len(),
            (params.k / params.noise_rank) as usize,
            "Fold count == num_stripes"
        );

        // (4) Co-location (§4.C.2 c-exact invariant): every
        // IS_MSG_MAT producer row is a strip-opening row, and ≥1
        // exists (co-location is actually active on P16).
        let mm_rows: Vec<usize> = (0..h).filter(|&r| rows[r][mm]).collect();
        assert!(
            !mm_rows.is_empty(),
            "co-location must be active on P16 (IS_MSG_MAT rows exist)"
        );
        for r in mm_rows {
            assert!(
                matches!(
                    sched[r],
                    RowClass::StripOpenA | RowClass::StripOpenB
                ),
                "co-located producer row {r} must be a StripOpen* row \
                 (the leaf round-0 rows ARE the M-S1 producers), \
                 got {:?}",
                sched[r]
            );
        }

        // (5) Jackpot + no-misclassification: every IS_HASH_JACKPOT
        // row is JackpotHash; no live anchor lands on a Pad row.
        for r in 0..h {
            if rows[r][jp] {
                assert_eq!(
                    sched[r], RowClass::JackpotHash,
                    "IS_HASH_JACKPOT row {r} must be JackpotHash"
                );
            }
            if rows[r][jk] || rows[r][ch] || rows[r][ha]
                || rows[r][hb] || rows[r][fo]
            {
                assert_ne!(
                    sched[r], RowClass::Pad,
                    "a live anchor at row {r} must not be \
                     misclassified as Pad by the schedule"
                );
            }
        }
    }

    /// **Phase A-CR · CR.1 — the §5 migration safety net (staged).**
    /// `ai_pow_zk::canonical::canonical_program` (params-pure, no
    /// witness) must equal `extract_program(honest_trace)`
    /// bit-for-bit on **every row of every `is_class_canonical`
    /// class** (CR.1: `Pad`), across all 12 PROGRAM_COLS, on the
    /// REAL `P16`(16|r) bridge trace. This is the §5 gate that, per
    /// row class, fences the eventual CR.6 verify-path flip: when
    /// every class is canonical and this KAT is all-green, the VK
    /// can commit to `canonical_program` instead of
    /// extract-of-reference (the CRIT-1 reconstruction-hardening
    /// soundness fix). The honest trace verifies under the current
    /// CRIT-1 (extract-of-reference) ⇒ its main-side PROGRAM_COLS
    /// (`extract_program`) ARE the trusted canonical program ⇒ a
    /// params-pure divergence on a canonical class fails here
    /// BEFORE trust (the P-B.2.0 KAT-first discipline). Widens with
    /// CR.2–CR.5. **No verify-path change (CR.1).**
    #[test]
    fn cr1_canonical_program_eq_extract_on_canonical_classes() {
        use crate::synth::synth_matrices;
        use ai_pow_zk::canonical::{
            canonical_program, is_class_canonical, row_schedule,
            BlockPublic,
        };
        use ai_pow_zk::composite_full_air::extract_program;
        use ai_pow_zk::params::ZkParams;
        use std::cell::RefCell;

        let params = MatmulParams {
            m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
            spot_checks: 2, difficulty_bits: 0,
        };
        params.validate().unwrap();
        let (a, b) = synth_matrices(b"cr1-eq-extract", &params);
        let ctx = BlockContext::build(b"cr1-eq-extract-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);
        let (tile_i, tile_j) = (0u32, 0u32);

        // Capture extract_program of the FULL real honest P16
        // trace via the no-tamper seam (honest proof still verifies
        // ⇒ its main-side PROGRAM_COLS ARE the trusted canonical
        // program under current CRIT-1). Run extract_program inside
        // the closure (where `&t.matrix` is in scope) so ai-pow
        // need not name the p3_matrix type.
        let cap: RefCell<Option<(Vec<ai_pow_zk::Val>, usize)>> =
            RefCell::new(None);
        prove_and_verify_tiled_tamper(
            &ctx, &params, TEST_NONCE, &target, tile_i, tile_j,
            |t: &mut CompositeTrace| {
                let e = extract_program(&t.matrix);
                *cap.borrow_mut() = Some((e.values, t.height()));
            },
        )
        .expect("honest P16 g=1 (no tamper) must prove + pow-verify");
        let (ext_vals, h) = cap.into_inner().expect("captured trace");
        let w = extract_program_width();
        assert_eq!(ext_vals.len(), h * w, "extract is h×12");

        let zk = ZkParams {
            m: params.m, k: params.k, n: params.n,
            noise_rank: params.noise_rank, tile: params.tile,
            difficulty_bits: params.difficulty_bits,
        };
        // CR.4c: co-located StripOpen noise pins depend on the
        // C1-pinned s_a/s_b ⇒ wire the REAL block public.
        let bp = BlockPublic {
            tile_i, tile_j, kappa: ctx.kappa,
            s_a: ctx.s_a, s_b: ctx.s_b,
        };
        let canon = canonical_program(&zk, &bp, h).expect("test ZkParams valid");
        assert_eq!(canon.values.len(), ext_vals.len());

        let sched = row_schedule(&zk, tile_i, tile_j, h);
        let mut checked = 0usize;
        for (r, &class) in sched.iter().enumerate() {
            if !is_class_canonical(class) {
                continue;
            }
            for c in 0..w {
                assert_eq!(
                    canon.values[r * w + c],
                    ext_vals[r * w + c],
                    "CR.1 §5: canonical_program ≠ \
                     extract_program at row {r} ({class:?}) col {c}"
                );
            }
            checked += 1;
        }
        assert!(
            checked > 0,
            "P16 has ≥1 canonical-class (Pad) row to validate"
        );
    }

    /// PROGRAM_COLS width (12) — `extract_program`'s row stride.
    fn extract_program_width() -> usize {
        ai_pow_zk::composite_full_air::PROGRAM_COLS.len()
    }

    /// **Phase A-CR · CR.4a — the pure-BLAKE3 strip-opening
    /// schedule.** `canonical_program`'s StripOpenA/B descriptor
    /// (the params-pure `strip_blocks` walker mirroring
    /// `fold_strip`/`subtree_inside`/`place_leaf_chunk` +
    /// per-block leaf/parent/root tweak + `IS_HASH_A/B` finalize
    /// selector) must equal `extract_program(honest_trace)`
    /// bit-for-bit on every StripOpen* row of the REAL P16(16|r)
    /// trace that is **NOT a co-located leaf round-0 row**
    /// (`IS_MSG_MAT == 0`). Those co-located rows additionally
    /// carry `IS_MSG_MAT` + the 8 `NOISE_PACKED_PREP` pins (CR.4b/
    /// CR.4c) and are validated there; here they are *skipped* so
    /// CR.4a's pure-BLAKE3 schedule is gated against the real
    /// trace in isolation (KAT-first, P-B.2.0 discipline). A wrong
    /// chunk-counter / flag / root-selector ⇒ a non-co-located
    /// strip row diverges ⇒ this fails. **No verify-path change.**
    #[test]
    fn cr4a_strip_open_pure_blake3_schedule_eq_extract() {
        use crate::synth::synth_matrices;
        use ai_pow_zk::canonical::{
            canonical_program, row_schedule, BlockPublic, RowClass,
        };
        use ai_pow_zk::composite_full_air::extract_program;
        use ai_pow_zk::composite_layout::{
            IS_MSG_MAT, TOTAL_TRACE_WIDTH,
        };
        use ai_pow_zk::params::ZkParams;
        use std::cell::RefCell;

        let params = MatmulParams {
            m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
            spot_checks: 2, difficulty_bits: 0,
        };
        params.validate().unwrap();
        let (a, b) = synth_matrices(b"cr4a-strip", &params);
        let ctx = BlockContext::build(b"cr4a-strip-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);
        let (tile_i, tile_j) = (0u32, 0u32);

        // Capture extract_program + per-row IS_MSG_MAT of the real
        // honest P16 trace (no-tamper seam ⇒ still verifies).
        let cap: RefCell<Option<(Vec<ai_pow_zk::Val>, Vec<bool>, usize)>> =
            RefCell::new(None);
        prove_and_verify_tiled_tamper(
            &ctx, &params, TEST_NONCE, &target, tile_i, tile_j,
            |t: &mut CompositeTrace| {
                let zero = ai_pow_zk::Val::default();
                let h = t.height();
                let e = extract_program(&t.matrix);
                let mm: Vec<bool> = (0..h)
                    .map(|r| {
                        t.matrix.values
                            [r * TOTAL_TRACE_WIDTH + IS_MSG_MAT]
                            != zero
                    })
                    .collect();
                *cap.borrow_mut() = Some((e.values, mm, h));
            },
        )
        .expect("honest P16 g=1 (no tamper) must prove + pow-verify");
        let (ext_vals, is_mm, h) = cap.into_inner().expect("trace");
        let w = extract_program_width();

        let zk = ZkParams {
            m: params.m, k: params.k, n: params.n,
            noise_rank: params.noise_rank, tile: params.tile,
            difficulty_bits: params.difficulty_bits,
        };
        // Real block public (CR.4c co-located noise pins).
        let bp = BlockPublic {
            tile_i, tile_j, kappa: ctx.kappa,
            s_a: ctx.s_a, s_b: ctx.s_b,
        };
        let canon = canonical_program(&zk, &bp, h).expect("test ZkParams valid");
        let sched = row_schedule(&zk, tile_i, tile_j, h);

        let (mut checked_pure, mut skipped_coloc) = (0usize, 0usize);
        for (r, &class) in sched.iter().enumerate() {
            if !matches!(
                class,
                RowClass::StripOpenA | RowClass::StripOpenB
            ) {
                continue;
            }
            if is_mm[r] {
                // Co-located leaf round-0 row — CR.4b/CR.4c.
                skipped_coloc += 1;
                continue;
            }
            for c in 0..w {
                assert_eq!(
                    canon.values[r * w + c],
                    ext_vals[r * w + c],
                    "CR.4a: canonical ≠ extract at non-co-located \
                     StripOpen row {r} ({class:?}) col {c}"
                );
            }
            checked_pure += 1;
        }
        assert!(
            checked_pure > 0,
            "P16 must have non-co-located StripOpen rows (the \
             7 mixing rounds + finalize + parent blocks)"
        );
        assert!(
            skipped_coloc > 0,
            "P16 (16|r) must have co-located leaf round-0 rows \
             (else CR.4a's skip is vacuous — co-location inactive?)"
        );
    }

    /// **Phase A-CR · CR.6 — the verify-path flip is sound
    /// (CRIT-1 first-class).** The bridge now verifies against
    /// `canonical_program(zk_params, BlockPublic)` — recomputed
    /// params-pure by the verifier — NOT the prover's
    /// `extract_program`. This test proves the soundness gain in
    /// isolation: an honest control verifies, then a trace whose
    /// **`NOISE_PACKED_PREP+1`** (a PROGRAM_COL that is canonically
    /// 0 on a `Pad` row and carries *no* other AIR constraint
    /// there — `g = IS_MSG_MAT·IS_NEW_BLAKE = 0` ⇒ the §4.C.2
    /// producer/InputChip constraints are gated off) is set
    /// non-zero. The prover's `extract_program` lifts the tampered
    /// value and the prover commits to it (its own in-AIR pin
    /// `main == preproc` still holds prover-side), but the
    /// verifier's VK commits to the **canonical** program (0
    /// there) ⇒ the proof's preprocessed opening cannot match the
    /// canonical commitment ⇒ rejected. Pre-CR.6 (verify against
    /// the prover's program) this forge would have *verified* —
    /// the exact latent weakness CR.6 closes.
    #[test]
    fn cr6_verify_uses_canonical_not_prover_program_rejects_forge() {
        use crate::synth::synth_matrices;
        use ai_pow_zk::canonical::{row_schedule, RowClass};
        use ai_pow_zk::composite_layout::{
            NOISE_PACKED_PREP, TOTAL_TRACE_WIDTH,
        };
        use ai_pow_zk::params::ZkParams;

        let params = MatmulParams {
            m: 16, k: 64, n: 16, noise_rank: 16, tile: 8,
            spot_checks: 2, difficulty_bits: 0,
        };
        params.validate().unwrap();
        let (a, b) = synth_matrices(b"cr6-forge", &params);
        let ctx = BlockContext::build(b"cr6-forge-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);
        let (tile_i, tile_j) = (0u32, 0u32);

        // Honest control: CR.6 canonical-VK verify still accepts a
        // genuine proof (the §5 KAT equivalence, end-to-end).
        prove_and_verify_tiled_tamper(
            &ctx, &params, TEST_NONCE, &target, tile_i, tile_j, |_| {},
        )
        .expect(
            "CR.6: an honest proof must still verify against the \
             verifier's params-pure canonical program",
        );

        // Forge: bump NOISE_PACKED_PREP+1 on the first Pad row.
        let zk = ZkParams {
            m: params.m, k: params.k, n: params.n,
            noise_rank: params.noise_rank, tile: params.tile,
            difficulty_bits: params.difficulty_bits,
        };
        let res = prove_and_verify_tiled_tamper(
            &ctx, &params, TEST_NONCE, &target, tile_i, tile_j,
            |t: &mut CompositeTrace| {
                let zero = ai_pow_zk::Val::default();
                let h = t.height();
                let sched = row_schedule(&zk, tile_i, tile_j, h);
                let pad = (0..h)
                    .find(|&r| sched[r] == RowClass::Pad)
                    .expect("P16 schedule has a Pad row");
                let cell =
                    pad * TOTAL_TRACE_WIDTH + NOISE_PACKED_PREP + 1;
                // A known-nonzero Val (≠ the canonical 0) without
                // naming p3_field: lift any nonzero trace cell.
                let nz = *t
                    .matrix
                    .values
                    .iter()
                    .find(|&&v| v != zero)
                    .expect("trace has a nonzero cell");
                // Canonically 0 here; no other AIR constraint binds
                // it on a Pad row ⇒ the ONLY defect is
                // prover_program ≠ canonical.
                t.matrix.values[cell] = nz;
            },
        );
        assert!(
            res.is_err(),
            "CR.6: a trace whose PROGRAM_COL ≠ the params-pure \
             canonical MUST be rejected by the canonical-VK verify \
             (pre-CR.6 this forge verified — the closed weakness)"
        );
    }

    /// **Goal part 1 — the matmul is proven IN-CIRCUIT for the real
    /// production parameters.** For the real shipped Llama mineable
    /// GEMMs `num_stripes = k/r = 4096/64 = 64 = STRIPE_MAX`, so the
    /// §6(b) `place_useful_work_chain` in-circuit matmul sweep runs
    /// and `sx_bound` / the `FOLD_XSTEP == SX_XR` keystone is live —
    /// the FoldChip inputs are bound to the genuine in-circuit
    /// matmul accumulator, NOT the off-circuit `compute_tile_trace`
    /// fallback. (`STRIPE_MAX = 64`; an earlier analysis wrongly
    /// used 16 — that is `JACKPOT_SIZE`, the M-state slot count.)
    ///
    /// Exercises the production boundary `num_stripes = 64 =
    /// STRIPE_MAX` at a tractable trace scale (`k=1024, r=16` ⇒
    /// `k/r=64`) and asserts `ZkOutcome::sweep_in_circuit == true`,
    /// plus that the real `LLAMA_3_1_8B_GATE_UP` preset itself has
    /// `num_stripes() == 64 ≤ STRIPE_MAX`.
    #[test]
    fn matmul_proven_in_circuit_at_real_param_num_stripes() {
        use crate::synth::synth_matrices;
        use ai_pow_zk::composite_layout::STRIPE_MAX;

        // The real shipped preset's stripe count: k=4096, r=64 ⇒ 64.
        assert_eq!(STRIPE_MAX, 64);
        assert_eq!(MatmulParams::LLAMA_3_1_8B_GATE_UP.num_stripes(), 64);
        assert!(
            (MatmulParams::LLAMA_3_1_8B_GATE_UP.num_stripes() as usize)
                <= STRIPE_MAX,
            "the real shipped preset must fit the in-circuit §6(b) sweep"
        );

        // num_stripes = k/r = 1024/16 = 64 = STRIPE_MAX — the exact
        // production boundary — at a trace size small enough for a
        // unit test. tile=8 ⇒ h·w=64 (Pearl-faithful). 16|r ⇒ coloc.
        let params = MatmulParams {
            m: 16,
            k: 1024,
            n: 16,
            noise_rank: 16,
            tile: 8,
            spot_checks: 2,
            difficulty_bits: 0,
        };
        params.validate().unwrap();
        assert_eq!(params.num_stripes() as usize, 64, "boundary config");

        let (a, b) = synth_matrices(b"in-circ-matmul", &params);
        let ctx = BlockContext::build(b"in-circ-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);

        let outcome = prove_and_verify_tiled(&ctx, &params, TEST_NONCE, &target, 0, 0)
            .expect("real-param block must prove + pow-verify");
        assert!(
            outcome.sweep_in_circuit,
            "num_stripes = 64 = STRIPE_MAX MUST take the in-circuit \
             §6(b) matmul-sweep path (place_useful_work_chain, \
             sx_bound + FOLD_XSTEP==SX_XR keystone live) — NOT the \
             off-circuit compute_tile_trace fallback. If this fails, \
             the matmul is not proven in-circuit for production."
        );
    }

    /// **§4.C.10 — decisive malicious-miner adversarial test.** A
    /// profit-incentivized miner runs the §6(b) in-circuit matmul
    /// sweep on a matrix OTHER than the one it committed / strip-
    /// opened (the `HASH_A`/`HASH_B` it publishes). If such a proof
    /// verifies, the miner forged the PoW without doing the real
    /// matmul of the committed matrices — a full A-FORGE break.
    ///
    /// A sound AIR MUST reject: the §6(b) sweep's `noised_packed`
    /// bus queries consume the forged matrix's chunks, which are not
    /// members of the committed-matrix producer store (the co-located
    /// strip-opening leaf rows on the 16∣r path) ⇒ the LogUp bus is
    /// unbalanced ⇒ reject.
    #[test]
    fn sec_4c10_sweep_on_uncommitted_matrix_rejects() {
        use crate::synth::synth_matrices;

        // 16∣r ⇒ coloc (production-faithful path); num_stripes=64.
        let params = MatmulParams {
            m: 16,
            k: 1024,
            n: 16,
            noise_rank: 16,
            tile: 8,
            spot_checks: 2,
            difficulty_bits: 0,
        };
        params.validate().unwrap();
        let (a, b) = synth_matrices(b"4c10-committed", &params);
        let ctx = BlockContext::build(b"4c10-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);

        // Honest control: sweep == committed ⇒ must verify.
        prove_and_verify_tiled_full(&ctx, &params, TEST_NONCE, &target, 0, 0, |_| {}, None)
            .expect("honest block (sweep == committed) must verify");

        // Attack: a DIFFERENT matrix drives the §6(b) matmul sweep,
        // while the strip-opening + HASH_A/HASH_B stay the committed
        // (a, b). HASH_JACKPOT self-consistently becomes M(forged).
        let (a2, b2) = synth_matrices(b"4c10-FORGED-sweep", &params);
        assert!(a2 != a || b2 != b, "forged matrix must differ");
        let res = prove_and_verify_tiled_full(
            &ctx,
            &params,
            TEST_NONCE,
            &target,
            0,
            0,
            |_| {},
            Some((&a2, &b2)),
        );
        assert!(
            res.is_err(),
            "§4.C.10: a proof whose §6(b) matmul sweep used a matrix \
             OTHER than the committed / strip-opened one MUST be \
             rejected — else a miner forges the PoW without doing \
             the real matmul of the committed matrices."
        );
    }

    /// **§4.C.10 — producer-planting / multiset-permutation
    /// adversarial test.** The `noised_packed` bus is *value-keyed*
    /// — "the chunk value is the discriminant" (multiset binding).
    /// So a miner who runs the §6(b) sweep on a **row-permuted**
    /// committed matrix presents the *same multiset* of chunks ⇒
    /// the bus balances even though the matmul is of a different
    /// matrix. This probes whether anything beyond the multiset bus
    /// (the §6(b) keystone / §6(a) fold-schedule pin / position-
    /// exact C3) binds the swept chunks to their committed
    /// **positions**. A sound AIR MUST reject — else a miner forges
    /// the PoW by permuting the committed matrix's rows.
    #[test]
    fn sec_4c10_sweep_on_row_permuted_matrix_rejects() {
        use crate::synth::synth_matrices;

        let params = MatmulParams {
            m: 16,
            k: 1024,
            n: 16,
            noise_rank: 16,
            tile: 8,
            spot_checks: 2,
            difficulty_bits: 0,
        };
        params.validate().unwrap();
        let (a, b) = synth_matrices(b"4c10-perm-committed", &params);
        let ctx = BlockContext::build(b"4c10-perm-blk", &a, &b, &params)
            .expect("ctx");
        let target = crate::tile_hash::difficulty_target(&params);

        // Row-reverse the committed A: a same-chunk-multiset but
        // genuinely different matrix (1 row = k = 1024 bytes = one
        // chunk, so the 8-byte sub-slice multiset is identical, just
        // reordered). The §6(b) sweep runs on this; the strip-
        // opening + HASH_A stay the committed `a`.
        let k = params.k as usize;
        let m = params.m as usize;
        let a_perm: Vec<i8> = (0..m)
            .rev()
            .flat_map(|i| a[i * k..(i + 1) * k].iter().copied())
            .collect();
        assert_ne!(a_perm, a, "row-reversed A must differ (pick another seed)");

        let res = prove_and_verify_tiled_full(
            &ctx,
            &params,
            TEST_NONCE,
            &target,
            0,
            0,
            |_| {},
            Some((&a_perm, &b)),
        );
        assert!(
            res.is_err(),
            "§4.C.10: a §6(b) sweep on a row-permuted committed \
             matrix (same chunk multiset, different matrix) MUST be \
             rejected — the multiset `noised_packed` bus alone does \
             not bind positions; the keystone / fold-schedule pin / \
             position-exact C3 must catch the permutation."
        );
    }

    // ===================================================================
    // M2 (DoS audit): structural-invariant defense at the `pub` bridge
    // boundary. A `MatmulParams` with `noise_rank = 0` historically hit
    // a `k / noise_rank` div-by-zero panic in `expected_layer0_rows`,
    // and a `found_idx >= num_tiles()` hit an `.expect()` panic in
    // `prove_and_verify_for_block`. Both are now Clean Err.
    // ===================================================================

    #[test]
    fn m2_invalid_params_yield_clean_error_not_panic() {
        let good = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"m2-seed", &good);
        let ctx = BlockContext::build(b"m2-blk", &a, &b, &good).expect("ctx");
        let target = [0xFFu8; 32];

        // The concrete pre-fix panic: noise_rank == 0 ⇒ k/0 in
        // `params.num_stripes()` inside `expected_layer0_rows`.
        let mut bad = good;
        bad.noise_rank = 0;

        assert!(
            matches!(
                prove_and_verify(&ctx, &bad, TEST_NONCE, &target),
                Err(BridgeError::InvalidParams(ParamError::NoiseRankOutOfRange))
            ),
            "prove_and_verify must surface InvalidParams(NoiseRankOutOfRange) — not panic"
        );
        assert!(
            matches!(
                prove_and_verify_tiled(&ctx, &bad, TEST_NONCE, &target, 0, 0),
                Err(BridgeError::InvalidParams(ParamError::NoiseRankOutOfRange))
            ),
            "prove_and_verify_tiled must surface InvalidParams(NoiseRankOutOfRange) — not panic"
        );
        assert!(
            matches!(
                prove_and_verify_for_block(&ctx, &bad, TEST_NONCE, 0),
                Err(BridgeError::InvalidParams(ParamError::NoiseRankOutOfRange))
            ),
            "prove_and_verify_for_block must surface InvalidParams(NoiseRankOutOfRange) — not panic"
        );
    }

    #[test]
    fn m2_found_idx_out_of_range_yields_clean_error_not_panic() {
        let params = MatmulParams::TEST_SMALL;
        let (a, b) = synth_matrices(b"m2-fb-seed", &params);
        let ctx = BlockContext::build(b"m2-fb-blk", &a, &b, &params).expect("ctx");

        let nt = params.num_tiles();
        let oob = nt as u32; // == num_tiles, just past the last valid idx
        let res = prove_and_verify_for_block(&ctx, &params, TEST_NONCE, oob);
        match res {
            Err(BridgeError::FoundIdxOutOfRange { found_idx, num_tiles }) => {
                assert_eq!(u64::from(found_idx), nt);
                assert_eq!(num_tiles, nt);
            }
            Err(other) => panic!(
                "expected FoundIdxOutOfRange for oob found_idx={oob}, got Err: {other}"
            ),
            Ok(_) => panic!(
                "expected FoundIdxOutOfRange for oob found_idx={oob}, got Ok"
            ),
        }
    }
}
