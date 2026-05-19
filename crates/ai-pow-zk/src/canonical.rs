//! Phase A-CR — params-pure `canonical_program` (CRIT-1
//! reconstruction hardening). Design + decisions D-CR1..4 +
//! staged plan CR.0..7: `2026-05-17_CANONICAL_PROGRAM_DESIGN.md`.
//!
//! **CR.0 (this module so far): the single params-pure row
//! schedule.** [`row_schedule`] assigns each trace row a
//! [`RowClass`] from `(ZkParams, tile_i, tile_j, trace_len)`
//! alone — *no witness* — reproducing the exact layout
//! `ai-pow::zk_bridge::prove_and_verify_tiled` builds on the
//! **production-faithful 16|r co-location path** (Pearl §4.8 is
//! always 16|r). It is *the* single source of truth for "which
//! row class sits where": CR.1..CR.5 build `canonical_program`'s
//! per-row `RowDescriptor` from this schedule + `block_public` +
//! `noise_ref`; CR.6 flips the verify path to commit to it.
//!
//! Validated by a cross-crate KAT (`ai-pow`,
//! `cr0_row_schedule_matches_real_bridge_trace`) that the
//! schedule's region boundaries match the real `P16`(16|r)
//! bridge trace's unambiguous selector anchors (KeyPin, the
//! Fold range, JackpotHash, the strip-opening / `HASH_A`/`HASH_B`
//! roots, the co-located `IS_MSG_MAT` leaf rows) — the
//! cx.0/cx.2-coloc.0 KAT-first discipline. **No verify-path
//! change in CR.0.**

use crate::blake3_tree::{left_len, strip_opening_rows, tile_chunk_range};
use crate::chips::blake3::chip::pack_tweak;
use crate::chips::blake3::compress::Blake3Tweak;
use crate::chips::control::NUM_SELECTORS;
use crate::chips::input::NOISE_PACKING_BASE;
use crate::noise_ref::{e_value, f_value};
use crate::composite_layout::{TILE_D, TILE_H};
use crate::composite_preprocess::{build_preprocessed_columns, RowDescriptor};
use crate::params::ZkParams;
use crate::Val;
use p3_matrix::dense::RowMajorMatrix;

/// Coarse per-row class — the CR.0 granularity (the bridge's
/// top-level row regions). CR.1..CR.5 refine the
/// PROGRAM_COL-bearing classes (Store sub-slices on the
/// co-located `StripOpen*` leaf round-0 rows; the §6(b) sweep
/// fold-schedule) into the per-cell `RowDescriptor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowClass {
    /// A-side strip-opening BLAKE3 compression rows (rows
    /// `[0, na)`). On the 16|r co-location path the leaf round-0
    /// rows here are also the M-S1 `noised_packed` producers.
    StripOpenA,
    /// B-side strip-opening compression rows (`[na, na+nb)`).
    StripOpenB,
    /// C1 key-pin rows (JOB_KEY = κ, then COMMITMENT_HASH = s_a).
    KeyPin,
    /// §6(b)-G1/G2 sub-block-major matmul sweep + StripeXor.
    Sweep,
    /// FoldChip rows (`num_stripes`).
    Fold,
    /// Final keyed-BLAKE3 jackpot-hash block (trace's last 8 rows).
    JackpotHash,
    /// Padding / inter-region gap (all selectors zero).
    Pad,
}

/// CR.0 — the params-pure row schedule for the **16|r
/// co-location production path**. Returns a `trace_len`-long
/// `Vec<RowClass>` reproducing `prove_and_verify_tiled`'s exact
/// row layout from public data only: `params` + the attested
/// `(tile_i, tile_j)` (MED-3-derived) + `trace_len`
/// (`Layer0RowBudget::required_trace_len`, itself params-pure,
/// P-B). Panics if `params.noise_rank % 16 != 0` (non-16|r is
/// the documented A3.2b *test* path whose separate-store row
/// count is value-deduped / data-dependent — out of the
/// params-pure / `canonical_program` scope; Pearl/production is
/// always 16|r).
pub fn row_schedule(
    params: &ZkParams,
    tile_i: u32,
    tile_j: u32,
    trace_len: usize,
) -> Vec<RowClass> {
    let l = schedule_layout(params, tile_i, tile_j, trace_len);
    (0..trace_len).map(|r| l.class_of(r)).collect()
}

/// CR.0 — the single params-pure source of truth for the bridge's
/// region boundaries (16|r co-location path). `row_schedule`
/// **and** `canonical_program`'s per-row `row_descriptor` both
/// derive from this *one* layout (the CR.0 invariant: there is
/// one schedule, not two constructions ⇒ no prover/verifier
/// divergence). All offsets are params-pure (CR.0a
/// `strip_opening_rows` + A1 `tile_chunk_range` + the §6(b) sweep
/// formula + the 16|r co-located store=0 + fold/jackpot offsets).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScheduleLayout {
    /// A-side strip-opening row count (`[0, na)` = StripOpenA).
    pub na: usize,
    /// End of strip-opening (`[na, mh_end)` = StripOpenB; row
    /// `mh_end` is the Pad gap).
    pub mh_end: usize,
    /// First §6(b) sweep row (`mh_end + 3`).
    pub sweep_start: usize,
    /// One past the last sweep row (`sweep_start + sweep_rows`).
    pub store_start: usize,
    /// First FoldChip row (`store_start + 4`; 16|r ⇒ 0 separate
    /// store rows).
    pub fold_start: usize,
    /// One past the last fold row (`fold_start + num_stripes`).
    pub fold_end: usize,
    /// First jackpot-hash row (`trace_len - 8`).
    pub jpot_start: usize,
}

impl ScheduleLayout {
    /// The [`RowClass`] of `row_idx` (the *one* classification).
    pub fn class_of(&self, r: usize) -> RowClass {
        if r < self.na {
            RowClass::StripOpenA
        } else if r < self.mh_end {
            RowClass::StripOpenB
        } else if r == self.mh_end + 1 || r == self.mh_end + 2 {
            RowClass::KeyPin
        } else if (self.sweep_start..self.store_start).contains(&r) {
            RowClass::Sweep
        } else if (self.fold_start..self.fold_end).contains(&r) {
            RowClass::Fold
        } else if r >= self.jpot_start {
            RowClass::JackpotHash
        } else {
            RowClass::Pad
        }
    }
}

/// Compute the [`ScheduleLayout`] from public data only. Panics
/// on non-16|r (the documented A3.2b test path — out of the
/// params-pure / `canonical_program` scope; Pearl §4.8 is always
/// 16|r).
pub(crate) fn schedule_layout(
    params: &ZkParams,
    tile_i: u32,
    tile_j: u32,
    trace_len: usize,
) -> ScheduleLayout {
    assert_eq!(
        params.noise_rank % 16,
        0,
        "schedule_layout is params-pure only on the 16|r \
         co-location path (Pearl §4.8 is always 16|r); non-16|r \
         is the documented A3.2b test path"
    );
    let t = params.tile as usize;
    let k = params.k as usize;
    let m = params.m as usize;
    let n = params.n as usize;
    let r = params.noise_rank as usize;
    let num_stripes = k / r;

    // Strip-opening A then B (P-B.2.4 + A1 tile_chunk_range +
    // CR.0a strip_opening_rows — all params-pure).
    let (ca0, ca1, a_nc) = tile_chunk_range(tile_i as usize, t, k, m * k);
    let na = strip_opening_rows(ca0, ca1, a_nc);
    let (cb0, cb1, b_nc) = tile_chunk_range(tile_j as usize, t, k, n * k);
    let nb = strip_opening_rows(cb0, cb1, b_nc);
    let mh_end = na + nb;

    // Key-pin: row mh_end is the gap; mh_end+1 = JOB_KEY,
    // mh_end+2 = COMMITMENT_HASH; sweep_start = mh_end+3.
    let sweep_start = mh_end + 3;
    // §6(b)-G1/G2 sweep = (t/TILE_H)² · num_stripes · ⌈r/TILE_D⌉
    // (== place_useful_work_chain's rows_used).
    let sweep_rows =
        (t / TILE_H) * (t / TILE_H) * num_stripes * r.div_ceil(TILE_D);
    let store_start = sweep_start + sweep_rows;
    // 16|r: producers are the co-located StripOpen leaf round-0
    // rows ⇒ ZERO separate store rows. fold_start =
    // store_start + 0 + 4.
    let fold_start = store_start + 4;
    let fold_end = fold_start + num_stripes;

    assert!(
        trace_len >= 8 && fold_end <= trace_len - 8,
        "schedule overflows trace_len={trace_len} (fold_end={fold_end})"
    );
    let jpot_start = trace_len - 8;

    ScheduleLayout {
        na, mh_end, sweep_start, store_start, fold_start, fold_end,
        jpot_start,
    }
}

/// Verifier-known per-block public inputs that, with `params`,
/// fully determine the canonical program (no witness). The
/// MED-3-attested tile, the C1-pinned BLAKE3 key/seeds. `hash_a`
/// / `hash_b` (the strip-opening roots) are *PI-bound*, not
/// PROGRAM_COLS, so they are not needed to build `RowDescriptor`s
/// — included in the design's `BlockPublic` for completeness but
/// omitted here until a class needs them.
#[derive(Debug, Clone, Copy)]
pub struct BlockPublic {
    /// MED-3-attested A-side tile row index.
    pub tile_i: u32,
    /// MED-3-attested B-side tile col index.
    pub tile_j: u32,
    /// C1-pinned keyed-BLAKE3 key κ (JOB_KEY).
    pub kappa: [u8; 32],
    /// C1-pinned A-side public seed s_a (COMMITMENT_HASH; the
    /// `noise_ref` seed for the §4.C.2/b2 store-noise pin).
    pub s_a: [u8; 32],
    /// C1-pinned B-side public seed s_b.
    pub s_b: [u8; 32],
}

/// Phase A-CR — which [`RowClass`]es `canonical_program` already
/// reconstructs **params-pure and `== extract_program`-validated**
/// (the §5 staged-migration gate set). CR.6 (verify-path flip) is
/// permitted only once this is *every* class. Staged per
/// `2026-05-17_CANONICAL_PROGRAM_DESIGN.md` §7 (R1 discipline).
///
/// - **CR.1 (landed): `Pad`** — witness-free, exactly
///   [`RowDescriptor::padding`] (all PROGRAM_COLS zero except
///   `STARK_ROW_IDX = row_idx`).
/// - **CR.2 (landed): `KeyPin`** — witness-free; the two
///   `place_key_pin_row` rows: `mh_end+1` → `IS_USE_JOB_KEY`
///   (SELECTOR_COLS idx 2), `mh_end+2` → `IS_USE_COMMITMENT_HASH`
///   (idx 3); `mat_id=0`, all other PROGRAM_COLS zero. (The
///   pinned PI κ/s_a lives in `CV_IN`, a chip column, *not* a
///   PROGRAM_COL ⇒ the descriptor is `bp`-independent.)
/// - **CR.3 (landed): `JackpotHash`** — witness-free; the final
///   8-row keyed-BLAKE3 block (`place_jackpot_hash_block` →
///   `place_blake3_hash_with_selectors`). Every row:
///   `CV_OR_TWEAK_PREP = pack_tweak(JACKPOT_TWEAK)` (the
///   *params-pure constant* `{counter=0, block_len=64,
///   flags=0x1B}` — the hashed M/key are CV columns, not
///   PROGRAM_COLS); mat_id=0; row 0 → IS_NEW_BLAKE (idx 8); row
///   7 → IS_LAST_ROUND (idx 9) + IS_HASH_JACKPOT (idx 6).
/// - **CR.4 (landed): `StripOpenA/B`** — params-pure
///   strip-opening: CR.4a the `strip_blocks` walker (mirrors
///   `fold_strip`/`subtree_inside`/`place_leaf_chunk`) +
///   per-block leaf/parent/root tweak + `IS_HASH_A/B` finalize
///   selector; CR.4b co-located leaf round-0 `IS_MSG_MAT` (idx
///   10); CR.4c the 8 `NOISE_PACKED_PREP[0..8]` pins =
///   `polyval(noise_ref(bp.s_a/s_b at p=chunk·1024+b·64+g), 129)`
///   (the §4.C.2/b2 core — verifier obtains canonical noise
///   params-pure, not extract-of-reference; **`bp.s_a/s_b`
///   dependent**).
/// - **CR.5 (landed): `Sweep`/`Fold`** — params-pure §6(b)
///   schedule. Sweep (`place_useful_work_chain`→
///   `place_matmul_step`): per the nested (sbi,sbj,step,chunk)
///   loop, IS_RESET_CUMSUM (idx 0) on each sub-block's first
///   micro-step else IS_UPDATE_CUMSUM (idx 1). Fold
///   (`place_fold_chain`): CONTROL_PREP packs is_fold=1,
///   fold_slot=offset%16, fold_stripe=offset.
///
/// **`is_class_canonical` = EVERY class ⇒ CR.6 may flip the
/// verify path (the R1 soundness linchpin: VK commits to
/// `canonical_program`; gate Route-A + crit1_* +
/// debug-assertions-ON + a new adversarial before the flip).**
pub fn is_class_canonical(_class: RowClass) -> bool {
    // CR.0–CR.5: every RowClass is reconstructed params-pure and
    // `== extract_program(real P16(16|r) trace)`-validated.
    true
}

/// `pack_tweak` of the final jackpot-hash block's tweak — the
/// params-pure constant `Blake3Tweak { counter_low: 0,
/// counter_high: 0, block_len: 64, flags: 0x1B }`
/// (KEYED_HASH|CHUNK_START|CHUNK_END|ROOT) that
/// `place_jackpot_hash_block` hard-codes. Witness-independent.
pub(crate) fn jackpot_tweak_packed() -> u64 {
    pack_tweak(&Blake3Tweak {
        counter_low: 0,
        counter_high: 0,
        block_len: 64,
        flags: 0x1B,
    })
}

/// Params-pure PROGRAM_COL descriptor for offset `j` (0..8)
/// within an 8-row BLAKE3 compression block — the *one* schedule
/// `place_blake3_hash_with_selectors` writes: every row carries
/// `CV_OR_TWEAK_PREP = tweak_packed` and `mat_id = 0`; row 0 sets
/// `IS_NEW_BLAKE` (SELECTOR_COLS idx 8); row 7 (finalize) sets
/// `IS_LAST_ROUND` (idx 9) plus `finalize_extra`. Shared by CR.3
/// `JackpotHash` and (CR.4) `StripOpen*` — they differ *only* in
/// the tweak and the finalize-extra selectors (+ CR.4's
/// co-located leaf noise sub-slice pins, layered on top).
fn blake3_block_descriptor(
    j: usize,
    tweak_packed: u64,
    finalize_extra: &[usize],
) -> RowDescriptor {
    let mut selectors = [false; NUM_SELECTORS];
    if j == 0 {
        selectors[8] = true; // IS_NEW_BLAKE
    }
    if j == 7 {
        selectors[9] = true; // IS_LAST_ROUND
        for &idx in finalize_extra {
            selectors[idx] = true;
        }
    }
    RowDescriptor {
        selectors,
        cv_or_tweak: tweak_packed,
        ..RowDescriptor::padding()
    }
}

/// **Phase A-CR — the params-pure canonical program.** Builds the
/// `trace_len × PROGRAM_COLS.len()` preprocessed matrix the CRIT-1
/// pin commits to, from public data **only** (`params` + the
/// attested/pinned `BlockPublic` + the params-pure `trace_len`) —
/// *no witness*. Per row: `row_schedule` (CR.0) → [`RowClass`] →
/// a params-pure [`RowDescriptor`] → the existing
/// [`build_preprocessed_columns`] packing (the *one* shared
/// schedule + the *one* packing — no prover/verifier divergence).
///
/// **Staged (R1 / §7).** Classes in [`is_class_canonical`] are
/// reconstructed exactly; all others currently fall back to
/// [`RowDescriptor::padding`] — a deliberate, KAT-fenced
/// *placeholder*, NOT a soundness claim. The §5 KAT
/// (`canonical_program == extract_program(honest_trace)`) asserts
/// equality **only on `is_class_canonical` rows**, widening as
/// CR.2–CR.5 land. **The verify path is NOT flipped to this until
/// CR.6, gated on every class canonical + the full KAT/Route-A/
/// crit1_*/debug-assertions-ON suite** (the soundness linchpin —
/// R1). Until then this is dead w.r.t. prove/verify.
pub fn canonical_program(
    params: &ZkParams,
    bp: &BlockPublic,
    trace_len: usize,
) -> RowMajorMatrix<Val> {
    let l = schedule_layout(params, bp.tile_i, bp.tile_j, trace_len);
    let sp = StripPlan::build(params, bp);
    let program: Vec<RowDescriptor> = (0..trace_len)
        .map(|r| row_descriptor(r, l.class_of(r), &l, &sp, params, bp))
        .collect();
    let rows = build_preprocessed_columns(&program, trace_len);
    let w = rows.first().map(|r| r.len()).unwrap_or(0);
    let flat: Vec<Val> = rows.into_iter().flatten().collect();
    RowMajorMatrix::new(flat, w)
}

/// One 8-row BLAKE3 block of a tile's strip-opening — the
/// params-pure unit `place_matrix_strip_opening` emits (the
/// block *contents* are witness, but the PROGRAM_COLS — tweak +
/// selector schedule — are not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StripBlock {
    /// Block `b`∈0..16 of the leaf chunk at global index
    /// `chunk_index` (`place_leaf_chunk`). `single_chunk_root` ⇒
    /// the lone-chunk path (block 15 carries `F_ROOT` + the
    /// `IS_HASH_A/B` finalize selector).
    Leaf { chunk_index: u64, b: usize, single_chunk_root: bool },
    /// An auth-fold parent compression (`place_parent`); `is_root`
    /// ⇒ `F_ROOT` + the `IS_HASH_A/B` finalize selector.
    Parent { is_root: bool },
}

/// Params-pure post-order block list for one tile's strip-opening
/// — mirrors `fold_strip` / `subtree_inside` / `place_leaf_chunk`
/// **exactly** (sibling subtrees consume 0 rows; each block = 8
/// rows). `8 * strip_blocks(..).len()` == `strip_opening_rows`.
fn strip_blocks(c0: usize, c1: usize, num_chunks: usize) -> Vec<StripBlock> {
    let mut out = Vec::new();
    if num_chunks == 1 {
        // Lone chunk (place_matrix_strip_opening's num_chunks==1
        // branch): place_leaf_chunk(chunk_index=0,
        // single_chunk_root=true).
        for b in 0..16 {
            out.push(StripBlock::Leaf {
                chunk_index: 0,
                b,
                single_chunk_root: true,
            });
        }
        return out;
    }
    fn subtree_inside(
        out: &mut Vec<StripBlock>,
        lo: usize,
        hi: usize,
        is_root: bool,
    ) {
        if hi - lo == 1 {
            // place_leaf_chunk(chunk_index=lo,
            // single_chunk_root=false) — a leaf is never root when
            // num_chunks>1.
            for b in 0..16 {
                out.push(StripBlock::Leaf {
                    chunk_index: lo as u64,
                    b,
                    single_chunk_root: false,
                });
            }
            return;
        }
        let mid = lo + left_len((hi - lo) as u64) as usize;
        subtree_inside(out, lo, mid, false);
        subtree_inside(out, mid, hi, false);
        out.push(StripBlock::Parent { is_root });
    }
    fn fold(
        out: &mut Vec<StripBlock>,
        lo: usize,
        hi: usize,
        c0: usize,
        c1: usize,
        is_root: bool,
    ) {
        if hi <= c0 || lo >= c1 {
            return; // auth sibling — 0 rows
        }
        if c0 <= lo && hi <= c1 {
            subtree_inside(out, lo, hi, is_root);
            return;
        }
        let mid = lo + left_len((hi - lo) as u64) as usize;
        fold(out, lo, mid, c0, c1, false);
        fold(out, mid, hi, c0, c1, false);
        out.push(StripBlock::Parent { is_root });
    }
    fold(&mut out, 0, num_chunks, c0, c1, true);
    out
}

/// Per-tile strip-opening plan: the params-pure block list + the
/// region's `IS_HASH_A/B` finalize selector (4 = `IS_HASH_A`
/// A-side, 5 = `IS_HASH_B` B-side — `place_matrix_hash_a/b`).
struct StripPlan {
    blocks_a: Vec<StripBlock>,
    blocks_b: Vec<StripBlock>,
}

impl StripPlan {
    fn build(params: &ZkParams, bp: &BlockPublic) -> Self {
        let t = params.tile as usize;
        let k = params.k as usize;
        let m = params.m as usize;
        let n = params.n as usize;
        let (ca0, ca1, a_nc) =
            tile_chunk_range(bp.tile_i as usize, t, k, m * k);
        let (cb0, cb1, b_nc) =
            tile_chunk_range(bp.tile_j as usize, t, k, n * k);
        StripPlan {
            blocks_a: strip_blocks(ca0, ca1, a_nc),
            blocks_b: strip_blocks(cb0, cb1, b_nc),
        }
    }
}

/// BLAKE3 flag bits (mirror `place_leaf_chunk` / `place_parent`).
const F_CHUNK_START: u32 = 1 << 0;
const F_CHUNK_END: u32 = 1 << 1;
const F_PARENT: u32 = 1 << 2;
const F_ROOT: u32 = 1 << 3;
const F_KEYED_HASH: u32 = 1 << 4;

/// Params-pure PROGRAM_COL descriptor for offset `j`∈0..8 of a
/// strip-opening block. CR.4a: the pure-BLAKE3 schedule (tweak +
/// selectors). **CR.4b:** on the 16|r co-location path every
/// *leaf* block's round-0 row (`j==0`) is additionally the M-S1
/// `noised_packed` producer — `place_leaf_chunk` re-fills its
/// control cells with `{IS_NEW_BLAKE, IS_MSG_MAT}` (SELECTOR_COLS
/// idx 8 + 10), `mat_id=0`, `msg_pair=0` (the cx.1c pin). The 8
/// `NOISE_PACKED_PREP` pins on those rows are CR.4c. Parent
/// blocks are never co-located. The tweak/flags are params-pure
/// (`place_leaf_chunk`/`place_parent`): leaf → `counter =
/// chunk_index`, `flags = F_KEYED_HASH | F_CHUNK_START(b==0) |
/// F_CHUNK_END(b==15) | F_ROOT(single-chunk-root&&b==15)`; parent
/// → `F_KEYED_HASH | F_PARENT | F_ROOT(is_root)`; the root
/// block's finalize row gets the `IS_HASH_A/B` extra
/// (`selector_idx`).
fn strip_row_descriptor(
    spec: StripBlock,
    j: usize,
    selector_idx: usize,
) -> RowDescriptor {
    let (tweak, is_root) = match spec {
        StripBlock::Leaf { chunk_index, b, single_chunk_root } => {
            let mut flags = F_KEYED_HASH;
            if b == 0 {
                flags |= F_CHUNK_START;
            }
            if b == 15 {
                flags |= F_CHUNK_END;
            }
            let is_root = single_chunk_root && b == 15;
            if is_root {
                flags |= F_ROOT;
            }
            (
                Blake3Tweak {
                    counter_low: chunk_index as u32,
                    counter_high: (chunk_index >> 32) as u16,
                    block_len: 64,
                    flags,
                },
                is_root,
            )
        }
        StripBlock::Parent { is_root } => {
            let mut flags = F_KEYED_HASH | F_PARENT;
            if is_root {
                flags |= F_ROOT;
            }
            (
                Blake3Tweak {
                    counter_low: 0,
                    counter_high: 0,
                    block_len: 64,
                    flags,
                },
                is_root,
            )
        }
    };
    let extra: &[usize] =
        if is_root { core::slice::from_ref(&selector_idx) } else { &[] };
    let mut desc = blake3_block_descriptor(j, pack_tweak(&tweak), extra);
    // CR.4b: co-located leaf round-0 producer row (16|r path —
    // `row_schedule` guarantees 16|r, and `place_leaf_chunk`
    // co-locates every leaf block's round-0 row when noise is
    // present). Adds IS_MSG_MAT (idx 10) on top of the round-0
    // IS_NEW_BLAKE (idx 8) already set by `blake3_block_descriptor`;
    // mat_id/msg_pair stay 0. Parent blocks are never co-located.
    if matches!(spec, StripBlock::Leaf { .. }) && j == 0 {
        desc.selectors[10] = true; // IS_MSG_MAT ⇒ g = 1
    }
    desc
}

/// **CR.4c — the §4.C.2/b2 core.** Params-pure
/// `NOISE_PACKED_PREP[0..8]` for a co-located leaf round-0 row
/// (block `b` of leaf chunk `chunk_index`, A-side ⇒ `e_value`/
/// `s_a`/`|A|=m·k`; B-side ⇒ `f_value`/`s_b`/`|B|=n·k`). Mirrors
/// `place_leaf_chunk` exactly: for sub-slice `s`, `pin[s] =
/// Σ_{m<8} noise[s·8+m] · NOISE_PACKING_BASE^m`, where the strip
/// byte position is `p = chunk_index·1024 + b·64 + (s·8+m)`
/// (the bridge's `a_strip_lo + j` collapses to this since
/// `strip_lo = c0·1024` and `j` is chunk-`c0`-relative), and
/// `noise = noise_ref(seed, …) if p < |M| else 0` (chunk
/// padding). Witness-free: only `bp.s_a/s_b` + params.
fn coloc_leaf_noise_pins(
    side_a: bool,
    chunk_index: u64,
    b: usize,
    params: &ZkParams,
    bp: &BlockPublic,
) -> [i64; 8] {
    let k = params.k as usize;
    let r = params.noise_rank;
    let limit = if side_a {
        params.m as usize * k
    } else {
        params.n as usize * k
    };
    let mut pins = [0i64; 8];
    for (s, pin) in pins.iter_mut().enumerate() {
        let mut npp: i64 = 0;
        let mut pw: i64 = 1;
        for mm in 0..8 {
            let p = chunk_index as usize * 1024 + b * 64 + s * 8 + mm;
            let no: i8 = if p < limit {
                if side_a {
                    // A row-major m×k: row=p/k, col=p%k.
                    e_value(&bp.s_a, (p / k) as u32, (p % k) as u32, r)
                } else {
                    // B col-major n×k: col=p/k, k-idx=p%k ⇒
                    // f_value(s_b, k-idx, col).
                    f_value(&bp.s_b, (p % k) as u32, (p / k) as u32, r)
                }
            } else {
                0
            };
            npp += (no as i64) * pw;
            pw *= NOISE_PACKING_BASE as i64;
        }
        *pin = npp;
    }
    pins
}

/// Params-pure per-row descriptor for a row. CR.1 `Pad` +
/// CR.2 `KeyPin` are exact; not-yet-canonical classes return the
/// neutral placeholder — fenced by [`is_class_canonical`] / the
/// staged §5 KAT (NOT a soundness claim; see [`canonical_program`]).
/// CR.3–CR.5 replace each arm with its params-pure construction
/// (`StripOpen*` co-located leaf rows' 8 noise sub-slice pins from
/// `noise_ref(bp.s_a/s_b)`, the §4.C.2/b2 core; the §6(b)
/// `Sweep`/`Fold` schedule), each landed behind its own
/// `== extract` gate.
fn row_descriptor(
    row_idx: usize,
    class: RowClass,
    layout: &ScheduleLayout,
    sp: &StripPlan,
    params: &ZkParams,
    bp: &BlockPublic,
) -> RowDescriptor {
    match class {
        RowClass::Pad => RowDescriptor::padding(),
        RowClass::StripOpenA | RowClass::StripOpenB => {
            // CR.4a: pure-BLAKE3 schedule (flat 8-row blocks;
            // sibling subtrees → 0 rows). A selector_idx=4
            // (IS_HASH_A), B=5 (IS_HASH_B). CR.4b: co-located leaf
            // round-0 IS_MSG_MAT. CR.4c: the 8 NOISE_PACKED_PREP
            // pins = polyval(noise_ref(s_a/s_b at the leaf
            // (i,l)),129) — the §4.C.2/b2 core.
            let side_a = class == RowClass::StripOpenA;
            let (offset, blocks, selector_idx) = if side_a {
                (row_idx, &sp.blocks_a, 4usize)
            } else {
                (row_idx - layout.na, &sp.blocks_b, 5usize)
            };
            let block = offset / 8;
            let j = offset % 8;
            debug_assert!(
                block < blocks.len(),
                "strip row offset {offset} past block list"
            );
            let spec = blocks[block];
            let mut desc = strip_row_descriptor(spec, j, selector_idx);
            // CR.4c: layer the 8 noise sub-slice pins onto the
            // co-located leaf round-0 producer rows (16|r path).
            if let StripBlock::Leaf { chunk_index, b, .. } = spec {
                if j == 0 {
                    let pins = coloc_leaf_noise_pins(
                        side_a, chunk_index, b, params, bp,
                    );
                    desc.noise_packed = pins[0];
                    desc.noise_packed_hi
                        .copy_from_slice(&pins[1..8]);
                }
            }
            desc
        }
        RowClass::KeyPin => {
            // CR.2: `place_key_pin_row` sets exactly one selector
            // and `mat_id=0`; row mh_end+1 = JOB_KEY (SELECTOR_COLS
            // idx 2), mh_end+2 = COMMITMENT_HASH (idx 3). The
            // pinned PI (κ / s_a) is written to `CV_IN` — a chip
            // column, not a PROGRAM_COL — so the canonical
            // descriptor is `bp`-independent.
            let mut selectors = [false; NUM_SELECTORS];
            if row_idx == layout.mh_end + 1 {
                selectors[2] = true; // IS_USE_JOB_KEY
            } else {
                debug_assert_eq!(row_idx, layout.mh_end + 2);
                selectors[3] = true; // IS_USE_COMMITMENT_HASH
            }
            RowDescriptor { selectors, ..RowDescriptor::padding() }
        }
        RowClass::JackpotHash => {
            // CR.3: the final 8-row keyed-BLAKE3 block at
            // `jpot_start`. Params-pure constant tweak; finalize
            // extra = IS_HASH_JACKPOT (SELECTOR_COLS idx 6, the
            // `place_jackpot_hash_block` `&[6]`).
            let j = row_idx - layout.jpot_start;
            debug_assert!(j < 8, "jackpot block is 8 rows");
            blake3_block_descriptor(j, jackpot_tweak_packed(), &[6])
        }
        RowClass::Sweep => {
            // CR.5: §6(b) sweep (`place_useful_work_chain` →
            // `place_matmul_step`). Row order is the nested
            // (sbi, sbj, step, chunk) loop; each row sets exactly
            // IS_RESET_CUMSUM (SELECTOR_COLS idx 0) on the
            // sub-block's first micro-step (step==0 && chunk==0)
            // else IS_UPDATE_CUMSUM (idx 1); mat_id=0, no
            // fold/msg_pair, no NOISE/CV/AB. num_stripes = k/r;
            // chunks = ⌈r/TILE_D⌉ (§6(b)-G1).
            let r = params.noise_rank as usize;
            let num_stripes = params.k as usize / r;
            let chunks = r.div_ceil(TILE_D).max(1);
            let per = num_stripes * chunks;
            let within = (row_idx - layout.sweep_start) % per;
            let step = within / chunks;
            let chunk = within % chunks;
            let is_reset = step == 0 && chunk == 0;
            let mut selectors = [false; NUM_SELECTORS];
            selectors[if is_reset { 0 } else { 1 }] = true;
            RowDescriptor { selectors, ..RowDescriptor::padding() }
        }
        RowClass::Fold => {
            // CR.5: `place_fold_chain` row `offset` (0..num_stripes)
            // — no selectors; CONTROL_PREP packs is_fold=1,
            // fold_slot = offset%16, fold_stripe = offset (§6(b)-G2
            // SX_XR lane). mat_id=0; FOLD_* are chip columns.
            let offset = row_idx - layout.fold_start;
            RowDescriptor {
                is_fold: true,
                fold_slot: (offset % 16) as u8,
                fold_stripe: offset as u8,
                ..RowDescriptor::padding()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p16() -> ZkParams {
        ZkParams { m: 16, k: 64, n: 16, noise_rank: 16, tile: 8, difficulty_bits: 0 }
    }

    #[test]
    fn row_schedule_regions_are_contiguous_and_cover_trace() {
        let p = p16();
        let len = 1 << 13; // MIN_STARK_LEN-class (P16 sub-envelope)
        let s = row_schedule(&p, 0, 0, len);
        assert_eq!(s.len(), len);
        // Region order: StripOpenA → StripOpenB → (Pad gap) →
        // KeyPin×2 → Sweep → (Pad) → Fold → (Pad) → JackpotHash.
        assert_eq!(s[0], RowClass::StripOpenA);
        assert_eq!(*s.last().unwrap(), RowClass::JackpotHash);
        assert_eq!(
            s.iter().filter(|&&c| c == RowClass::KeyPin).count(),
            2,
            "exactly two key-pin rows (JOB_KEY, COMMITMENT_HASH)"
        );
        assert_eq!(
            s.iter().filter(|&&c| c == RowClass::JackpotHash).count(),
            8,
            "jackpot-hash block is the last 8 rows"
        );
        let nsweep =
            s.iter().filter(|&&c| c == RowClass::Sweep).count();
        let nfold =
            s.iter().filter(|&&c| c == RowClass::Fold).count();
        assert_eq!(nfold, (p.k / p.noise_rank) as usize, "fold = num_stripes");
        assert!(nsweep > 0 && s.contains(&RowClass::StripOpenB));
    }

    fn bp0() -> BlockPublic {
        BlockPublic {
            tile_i: 0, tile_j: 0, kappa: [0u8; 32],
            s_a: [0u8; 32], s_b: [0u8; 32],
        }
    }

    #[test]
    fn cr1_canonical_program_pad_rows_are_exact_padding_pack() {
        use crate::composite_full_air::PROGRAM_COLS;
        use p3_field::PrimeField64;
        use p3_matrix::Matrix;

        // is_class_canonical fence: CR.0–CR.5 ⇒ EVERY class.
        for c in [
            RowClass::Pad, RowClass::KeyPin, RowClass::JackpotHash,
            RowClass::StripOpenA, RowClass::StripOpenB,
            RowClass::Sweep, RowClass::Fold,
        ] {
            assert!(is_class_canonical(c), "{c:?} canonical by CR.5");
        }

        let p = p16();
        let len = 1 << 13;
        let prog = canonical_program(&p, &bp0(), len);
        assert_eq!(prog.height(), len);
        assert_eq!(prog.width(), PROGRAM_COLS.len(), "12-wide");

        let sched = row_schedule(&p, 0, 0, len);
        let w = PROGRAM_COLS.len();
        let mut saw_pad = false;
        for (r, &class) in sched.iter().enumerate() {
            if class != RowClass::Pad {
                continue;
            }
            saw_pad = true;
            // Pad row: all PROGRAM_COLS zero except STARK_ROW_IDX
            // (== PROGRAM_COLS[11], the monotonic row counter).
            for c in 0..w - 1 {
                assert_eq!(
                    prog.values[r * w + c].as_canonical_u64(),
                    0,
                    "Pad row {r} col {c} must be 0"
                );
            }
            assert_eq!(
                prog.values[r * w + (w - 1)].as_canonical_u64(),
                r as u64,
                "Pad row {r} STARK_ROW_IDX must be row_idx"
            );
        }
        assert!(saw_pad, "P16 schedule has Pad rows");
    }

    #[test]
    fn cr2_canonical_program_keypin_rows_are_exact() {
        use crate::chips::control::ControlChip;
        use crate::composite_full_air::PROGRAM_COLS;
        use p3_field::PrimeField64;

        let p = p16();
        let len = 1 << 13;
        let l = schedule_layout(&p, 0, 0, len);
        let prog = canonical_program(&p, &bp0(), len);
        let w = PROGRAM_COLS.len();

        // Expected CONTROL_PREP for each key-pin row: exactly one
        // selector set (JOB_KEY idx 2 at mh_end+1, COMMITMENT_HASH
        // idx 3 at mh_end+2), mat_id=0, no fold/msg_pair.
        for (row, sel_idx) in
            [(l.mh_end + 1, 2usize), (l.mh_end + 2, 3usize)]
        {
            assert_eq!(l.class_of(row), RowClass::KeyPin);
            let mut sel = [false; NUM_SELECTORS];
            sel[sel_idx] = true;
            let want_cp = ControlChip::pack_control_prep_full(
                &sel, 0, false, 0, 0, 0,
            );
            // PROGRAM_COLS[0] = CONTROL_PREP.
            assert_eq!(
                prog.values[row * w].as_canonical_u64(),
                want_cp,
                "key-pin row {row} CONTROL_PREP must pack only \
                 SELECTOR_COLS idx {sel_idx}"
            );
            // Cols 1..11 (noise×8, CV, AB_ID) zero; col 11
            // (STARK_ROW_IDX) = row.
            for c in 1..w - 1 {
                assert_eq!(
                    prog.values[row * w + c].as_canonical_u64(),
                    0,
                    "key-pin row {row} col {c} must be 0"
                );
            }
            assert_eq!(
                prog.values[row * w + (w - 1)].as_canonical_u64(),
                row as u64,
                "key-pin row {row} STARK_ROW_IDX"
            );
        }
    }

    #[test]
    fn cr3_canonical_program_jackpot_block_is_exact() {
        use crate::chips::control::ControlChip;
        use crate::composite_full_air::PROGRAM_COLS;
        use p3_field::PrimeField64;

        let p = p16();
        let len = 1 << 13;
        let l = schedule_layout(&p, 0, 0, len);
        let prog = canonical_program(&p, &bp0(), len);
        let w = PROGRAM_COLS.len();
        let tw = jackpot_tweak_packed();
        assert_ne!(tw, 0, "jackpot tweak packs non-zero (flags=0x1B)");

        // All 8 rows [jpot_start, len) are JackpotHash.
        for j in 0..8 {
            let row = l.jpot_start + j;
            assert_eq!(l.class_of(row), RowClass::JackpotHash);
            let mut sel = [false; NUM_SELECTORS];
            if j == 0 {
                sel[8] = true; // IS_NEW_BLAKE
            }
            if j == 7 {
                sel[9] = true; // IS_LAST_ROUND
                sel[6] = true; // IS_HASH_JACKPOT
            }
            let want_cp = ControlChip::pack_control_prep_full(
                &sel, 0, false, 0, 0, 0,
            );
            // PROGRAM_COLS: [0]=CONTROL_PREP, [1..9]=NOISE×8,
            // [9]=CV_OR_TWEAK_PREP, [10]=AB_ID, [11]=STARK_ROW_IDX.
            assert_eq!(
                prog.values[row * w].as_canonical_u64(), want_cp,
                "jackpot row j={j} CONTROL_PREP"
            );
            for c in 1..9 {
                assert_eq!(
                    prog.values[row * w + c].as_canonical_u64(), 0,
                    "jackpot row j={j} NOISE_PACKED_PREP[{}] must be 0",
                    c - 1
                );
            }
            assert_eq!(
                prog.values[row * w + 9].as_canonical_u64(), tw,
                "jackpot row j={j} CV_OR_TWEAK_PREP == jackpot tweak"
            );
            assert_eq!(
                prog.values[row * w + 10].as_canonical_u64(), 0,
                "jackpot row j={j} AB_ID_PREP must be 0"
            );
            assert_eq!(
                prog.values[row * w + 11].as_canonical_u64(),
                row as u64,
                "jackpot row j={j} STARK_ROW_IDX"
            );
        }
    }

    #[test]
    fn cr4a_strip_blocks_row_count_matches_strip_opening_rows() {
        // The params-pure block walker must reproduce CR.0a's
        // row count exactly (8 rows/block; 0 for auth siblings).
        for nc in [1usize, 2, 3, 5, 8, 13, 16, 21] {
            for c0 in 0..nc {
                for c1 in (c0 + 1)..=nc {
                    let blocks = strip_blocks(c0, c1, nc);
                    assert_eq!(
                        blocks.len() * 8,
                        strip_opening_rows(c0, c1, nc),
                        "nc={nc} [{c0},{c1}) block*8 != strip_opening_rows"
                    );
                    // Lone chunk ⇒ 16 single-chunk-root leaf blocks.
                    if nc == 1 {
                        assert_eq!(blocks.len(), 16);
                        assert!(blocks.iter().all(|b| matches!(
                            b,
                            StripBlock::Leaf { single_chunk_root: true, .. }
                        )));
                    }
                    // Exactly one root block (the post-order last).
                    if nc > 1 {
                        let roots = blocks
                            .iter()
                            .filter(|b| {
                                matches!(
                                    b,
                                    StripBlock::Parent { is_root: true }
                                ) || matches!(
                                    b,
                                    StripBlock::Leaf {
                                        single_chunk_root: true,
                                        ..
                                    }
                                )
                            })
                            .count();
                        assert_eq!(
                            roots, 1,
                            "nc={nc} [{c0},{c1}) must have exactly one root"
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "16|r")]
    fn row_schedule_rejects_non_16r() {
        // TEST_SMALL-shaped: r=4, 16∤4 — out of params-pure scope.
        let p = ZkParams {
            m: 64, k: 64, n: 64, noise_rank: 4, tile: 8, difficulty_bits: 0,
        };
        let _ = row_schedule(&p, 0, 0, 1 << 13);
    }
}
