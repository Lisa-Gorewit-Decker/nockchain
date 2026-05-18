//! Phase A-CR — params-pure `canonical_program` (CRIT-1
//! reconstruction hardening). Design + decisions D-CR1..4 +
//! staged plan CR.0..7: `CANONICAL_PROGRAM_DESIGN.md`.
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

use crate::blake3_tree::{strip_opening_rows, tile_chunk_range};
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
    assert_eq!(
        params.noise_rank % 16,
        0,
        "row_schedule is params-pure only on the 16|r co-location \
         path (Pearl §4.8 is always 16|r); non-16|r is the \
         documented A3.2b test path"
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

    let mut sched = vec![RowClass::Pad; trace_len];
    for (r_idx, c) in sched.iter_mut().enumerate() {
        *c = if r_idx < na {
            RowClass::StripOpenA
        } else if r_idx < mh_end {
            RowClass::StripOpenB
        } else if r_idx == mh_end + 1 || r_idx == mh_end + 2 {
            RowClass::KeyPin
        } else if (sweep_start..store_start).contains(&r_idx) {
            RowClass::Sweep
        } else if (fold_start..fold_end).contains(&r_idx) {
            RowClass::Fold
        } else if r_idx >= jpot_start {
            RowClass::JackpotHash
        } else {
            RowClass::Pad
        };
    }
    sched
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
/// `CANONICAL_PROGRAM_DESIGN.md` §7 (R1 discipline).
///
/// - **CR.1 (landed): `Pad`** — witness-free, exactly
///   [`RowDescriptor::padding`] (all PROGRAM_COLS zero except
///   `STARK_ROW_IDX = row_idx`).
/// - CR.2 `KeyPin` · CR.3 `JackpotHash` · CR.4 `StripOpenA/B`
///   (incl. the 8 co-located noise sub-slice pins via
///   `noise_ref`, the §4.C.2/b2 core) · CR.5 `Sweep`/`Fold` —
///   **residual** (each gated by its own `== extract` KAT).
pub fn is_class_canonical(class: RowClass) -> bool {
    matches!(class, RowClass::Pad)
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
    let sched = row_schedule(params, bp.tile_i, bp.tile_j, trace_len);
    let program: Vec<RowDescriptor> = sched
        .iter()
        .map(|&class| row_descriptor(class, params, bp))
        .collect();
    let rows = build_preprocessed_columns(&program, trace_len);
    let w = rows.first().map(|r| r.len()).unwrap_or(0);
    let flat: Vec<Val> = rows.into_iter().flatten().collect();
    RowMajorMatrix::new(flat, w)
}

/// Params-pure per-row descriptor for a [`RowClass`]. CR.1: only
/// `Pad` is exact (`RowDescriptor::padding`); not-yet-canonical
/// classes return the same neutral placeholder — fenced by
/// [`is_class_canonical`] / the staged §5 KAT (NOT a soundness
/// claim; see [`canonical_program`]). CR.2–CR.5 replace each arm
/// with its params-pure construction (`KeyPin` from `bp.kappa`/
/// `bp.s_a`; `StripOpen*` co-located leaf rows' 8 noise sub-slice
/// pins from `noise_ref(bp.s_a/s_b)`; the §6(b) `Sweep`/`Fold`
/// schedule), each landed behind its own `== extract` gate.
fn row_descriptor(
    class: RowClass,
    _params: &ZkParams,
    _bp: &BlockPublic,
) -> RowDescriptor {
    match class {
        RowClass::Pad => RowDescriptor::padding(),
        // CR.2–CR.5 residual — placeholder, fenced by
        // `is_class_canonical` (never trusted until CR.6).
        _ => RowDescriptor::padding(),
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

        // is_class_canonical fence: CR.1 ⇒ exactly {Pad}.
        assert!(is_class_canonical(RowClass::Pad));
        for c in [
            RowClass::StripOpenA, RowClass::StripOpenB,
            RowClass::KeyPin, RowClass::Sweep, RowClass::Fold,
            RowClass::JackpotHash,
        ] {
            assert!(!is_class_canonical(c), "{c:?} not canonical until CR.2+");
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
    #[should_panic(expected = "16|r")]
    fn row_schedule_rejects_non_16r() {
        // TEST_SMALL-shaped: r=4, 16∤4 — out of params-pure scope.
        let p = ZkParams {
            m: 64, k: 64, n: 64, noise_rank: 4, tile: 8, difficulty_bits: 0,
        };
        let _ = row_schedule(&p, 0, 0, 1 << 13);
    }
}
