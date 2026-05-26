> _Created **2026-05-14** · last updated **2026-05-14** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# M10.1c — Pearl-style composite AIR design (Plonky3 port)

Restore the PoUW property by binding `witness.a_rows` / `witness.b_cols`
to the chain-pinned `h_a` / `h_b` cryptographically.

**Reference design:** [`pearl/zk-pow/src/circuit/`](../../pearl/zk-pow/src/circuit/)
— Pearl's existing Plonky2 implementation, ~7,000 lines across
`pearl_air.rs` (top-level), four per-chip directories
(`chip/blake3/`, `chip/matmul/`, `chip/jackpot/`, `chip/input/`),
and supporting infrastructure (`pearl_layout.rs`,
`pearl_program.rs`, `pearl_preprocess.rs`, `pearl_trace.rs`,
`pearl_stark.rs`, `utils/`). We port this *one for one* to Plonky3
primitives. Performance traits should match Pearl's **pre-recursion**
baseline; matching their **post-recursion** 60 KB number requires
M12 (recursion), which Plonky3 doesn't ship today.

## Architecture mirror

Pearl runs a single STARK with **four chips sharing every row**:
Input, BLAKE3, Matmul, Jackpot. Each row's behaviour is determined
by preprocessed control columns. We replicate this structure verbatim:

| Pearl module | Plonky3 equivalent | Notes |
|---|---|---|
| `pearl_air.rs` | new `composite_full_air.rs` | top-level eval. |
| `pearl_layout.rs` | new `composite_layout.rs` | column layout macro / consts. |
| `pearl_preprocess.rs` | new `composite_preprocess.rs` | preprocessed columns. |
| `pearl_program.rs` | new `composite_program.rs` | per-row instruction stream. |
| `pearl_trace.rs` | new `composite_trace.rs` | trace generator. |
| `pearl_stark.rs` | new `composite_stark.rs` | `Stark`-trait analog (we have `p3-uni-stark` + `p3-lookup`). |
| `chip/blake3/{logic, blake3_compress, blake3_air, blake3_layout, trace, constraints, program}.rs` | new `chip/blake3/` module set | **one BLAKE3 round per row** (not one full hash). |
| `chip/matmul/{logic, trace, constraints}.rs` | new `chip/matmul/` module set |  |
| `chip/jackpot/{logic, trace, constraints, helper}.rs` | new `chip/jackpot/` module set | rotate-XOR-13 state evolution. |
| `chip/input/{trace, constraints}.rs` + `chip/i8u8.rs` | new `chip/input/` module set | range tables + i8↔u8 conversion. |
| `chip/control_and_matid_packed.rs` | new `chip/control.rs` | unpack `CONTROL_PREP`. |
| `chip/monotonic_increment.rs` | new `chip/stark_row.rs` | `STARK_ROW_IDX = 0, 1, 2, …`. |
| `utils/{air_utils, evaluator, native_evaluator, symbolic_evaluator}.rs` | re-use `p3-air` traits | Plonky3's `AirBuilder` already abstracts native vs. symbolic. |

Plonky3 substitutions for Pearl's Plonky2 idioms:

| Pearl uses | Plonky3 equivalent | Notes |
|---|---|---|
| `starky::stark::Stark` | `p3_uni_stark::StarkGenericConfig` + `p3_air::Air` | we already use this. |
| `starky::lookup::{Lookup, Column, Filter}` | `p3_lookup::{Lookup, LookupTraceBuilder, …}` | already in tree (`p3-lookup`). |
| `ConstraintConsumer` / `RecursiveConstraintConsumer` | `AirBuilder::assert_eq` etc. | Plonky3's builder abstracts both. |
| `eval.constraint_eq_if(selector, lhs, rhs)` | `builder.when(selector).assert_eq(lhs, rhs)` | direct analog. |
| Preprocessed columns committed at setup | Plonky3 preprocessed trace via `Air::preprocessed_main` | needs config support. |
| Plonky2 recursion ladder (3-tier) | **not available** | M12 follow-on. |

## Pearl's key insight: one BLAKE3 ROUND per row (not full hash)

Pearl's `chip/blake3/blake3_layout.rs` confirms the BLAKE3 chip is
narrow — 8 rounds per full BLAKE3 hash, each round occupying one
row with `INPUT_STATE` / `STATE1` / `STATE2` / `STATE3` columns
(~528 cells per round). Compared to our M10.1b vendored chip which
packs the entire 7-round Blake3 compression into one row of ~10,000
cells, this is dramatically narrower **but taller**.

```text
Plonky3 vendored Blake3Air (our M10.1b):  ~10,000 cols × 1 row = ~10,000 cells per hash
Pearl-style one-round-per-row:            ~1,000 cols × 8 rows = ~8,000 cells per hash
```

Similar total cell count, but Pearl's layout *reuses the same row
shape across the matmul / jackpot / range chips*, so the BLAKE3
column block is only ~1k of the ~1.3k-col total trace width.
With our current Blake3Air, a composite trace would carry ~10k
BLAKE3 cols even on matmul-only rows, which is wasteful.

**Decision: reimplement the BLAKE3 chip Pearl-style.** Drop the
vendored `Blake3KeyedAir` for M10.1c. Constants and per-round
arithmetic are ports of Pearl's `chip/blake3/blake3_compress.rs` /
`logic.rs`. We retain M10.1b's vendored chip + KAT tests as
a **reference for byte-equivalence checking** during development —
the new chip must produce the same outputs.

## Cross-row / cross-chip linkage via logUp

Pearl uses six lookup arguments (see `pearl_stark.rs:128-206`),
each enforcing a multiset equality. Plonky3 provides equivalent
machinery via `p3-lookup`. The lookups we need:

```text
URANGE8        : every u8-shaped column entry ∈ {0..255}
URANGE13       : every u13-shaped column entry ∈ {0..8191}     (MAT_ID limbs, etc.)
IRANGE7P1      : every i7+1 entry ∈ {-64..64}                  (raw matrix / noise)
IRANGE8        : every i8 entry  ∈ {-128..127}                 (noised matrix)
I8U8           : (i8 entry, u8 entry) pair valid given IS_MSG_MAT
                 — converts signed → unsigned for BLAKE3 input bytes
NOISED_PACKED  : the matmul chip's A_NOISED / B_NOISED tile rows
                 must come from NOISED_PACKED at matching MAT_ID,
                 i.e. a RAM-style lookup keyed by MAT_ID
CV_IN          : the BLAKE3 chip's CV_IN at row N must equal CV_OUT
                 at the row indexed by CV_OR_TWEAK_PREP — backwards
                 dependency, makes the Merkle chain in-circuit
```

The matmul ↔ BLAKE3 *linkage*, the property that makes M10.1c
restore PoUW, comes for free from the `NOISED_PACKED` lookup
combined with the `I8U8` conversion. The matmul chip pulls a tile
from `NOISED_PACKED[MAT_ID]`; the BLAKE3 leaf-hashing row uses the
same `NOISED_PACKED[MAT_ID]` bytes (after `i8→u8` conversion) as
its message input. The lookup forces both to read the *same* row.

**Cryptographic strength:** logUp with a random challenge in
`BinomialExtensionField<Goldilocks, 2>` (128-bit) gives statistical
soundness at the standard rate. Plonky3 absorbs challenges via
Fiat-Shamir; assumptions match the rest of our FRI stack.

## Preprocessed columns

Pearl's `pearl_preprocess.rs` builds four preprocessed columns:

```text
CONTROL_PREP      — bit-packed selectors + MAT_ID
NOISE_PACKED_PREP — precomputed noise for this row's matrix input
CV_OR_TWEAK_PREP  — either a row index (CV lookup) or BLAKE3 tweak flags
AB_ID_PREP        — A_ID || B_ID for the matmul tile load
```

These are deterministic functions of `PublicProofParams` — they
encode the "program" (which chip runs on which row). Plonky3's
preprocessed-trace mechanism (`Air::preprocessed_main`) is the
direct analog. We commit them at setup time per matmul shape.

## Public-input binding

Pearl's public inputs (5 hash values, 8 Goldilocks each = 40 PIs):

```text
JOB_KEY           : BLAKE3(BlockHeader ‖ MiningConfiguration)
COMMITMENT_HASH   : noise seed (a_noise_seed in Pearl terms; s_A in ai-pow)
HASH_A            : BLAKE3(A, key=JOB_KEY)            — what M10.1c binds to
HASH_B            : BLAKE3(B^t, key=JOB_KEY)          — what M10.1c binds to
HASH_JACKPOT      : BLAKE3(JACKPOT_MSG, key=COMMITMENT_HASH)  — found_leaf
```

We already cover JACKPOT (M10.1b). M10.1c adds HASH_A and HASH_B.
JOB_KEY and COMMITMENT_HASH enter the AIR as CV inputs to the BLAKE3
chip (verified via CV routing lookup).

Public-input binding pattern (`pearl_air.rs:90-109`):

```rust
for i in 0..pearl_public::HASH_A_LEN {
    let pub_hash_a = eval.scalar(public_inputs[pearl_public::HASH_A + i]);
    let out_cv = blake3_output[i];
    eval.constraint_eq_if(blake3_cf.is_hash_a, pub_hash_a, out_cv);
}
```

Plonky3 analog:

```rust
let pis = builder.public_values();
let mut when_hash_a = builder.when(blake3_cf.is_hash_a);
for i in 0..NUM_HASH_LIMBS {
    when_hash_a.assert_eq(blake3_output[i], pis[HASH_A_OFFSET + i]);
}
```

## Performance budget

Pearl's `pearl_program.rs` pins `MIN_STARK_LEN = 1 << 13 = 8192`
rows. Trace width ≈ 1,300 cols. So Pearl's pre-recursion STARK is
roughly `2^13 × 1.3k = ~10M cells`.

We expect a Plonky3 port to land in the same ballpark:

| Metric | Pearl (pre-recursion) | Pearl (post-recursion) | Plonky3 port (M10.1c) | Notes |
|---|---|---|---|---|
| Trace width | ~1.3k cols | (recursive) | ~1.3k cols (target) | one-round-per-row BLAKE3 keeps this narrow |
| Trace height | ≥ 8192 rows | (recursive) | ≥ 8192 rows | same minimum |
| Prove time | seconds | ~30 s end-to-end | seconds (target) | similar |
| Verify time | ms | ~50 ms | ms (target) | similar |
| Proof size | ~1–2 MB | ~60 KB | ~1–2 MB | **recursion gap** |

**To match Pearl's 60 KB post-recursion size, we need recursion**
(M12). Plonky3 doesn't ship a recursion ladder. WHIR (newer FRI
variant in `p3-whir`) is an option for proof-size reduction
without recursion, but doesn't compress to the same degree.

The user's request — "similar performance traits and proof size as
theirs" — divides into two checkpoints:
  * **M10.1c lands.** Performance traits match Pearl's
    pre-recursion architecture (one big STARK, lookups, narrow
    trace, similar prove/verify times). Proof size in the 1–2 MB
    range. PoUW property restored.
  * **M12 lands.** Recursion compresses to Pearl's ~60 KB target.
    Estimate: 2-4 weeks of engineering on top of M10.1c.

## Implementation phases

Direct port of Pearl's structure. Each phase mirrors one of Pearl's
files / chips and is independently testable.

| Phase | Pearl reference | Plonky3 deliverable | Tests |
|---|---|---|---|
| **1** | (this doc) | design + phasing | docs commit |
| **2** | `pearl_layout.rs` | `composite_layout.rs` with column-layout macro | const-pinning tests |
| **3** | `chip/monotonic_increment.rs` | `stark_row` chip (`STARK_ROW_IDX` = 0, 1, 2, …) | round-trip prove/verify |
| **4** | `chip/i8u8.rs`, `chip/{urange8, urange13, irange7p1, irange8}.rs`, `chip/input/` | range-table chips + `INPUT_CHIP` | per-table KAT |
| **5** | `chip/control_and_matid_packed.rs` | `control_chip` unpacking `CONTROL_PREP` | bitfield round-trip |
| **6** | `pearl_preprocess.rs` | preprocessed-trace generation | golden traces vs Pearl fixtures |
| **7** | `chip/blake3/blake3_compress.rs`, `logic.rs`, `blake3_layout.rs` | new `chip/blake3/` (NOT the M10.1b vendored chip) | KAT cross-check vs `blake3` crate AND vs M10.1b reference |
| **8** | `chip/blake3/trace.rs`, `constraints.rs`, `program.rs` | trace gen + constraint eval | per-instruction round trip |
| **9** | `chip/matmul/{logic, trace, constraints}.rs` | new matmul chip (NOT the M9.1 `composite_air`) | tile-correctness tests |
| **10** | `chip/jackpot/{logic, trace, constraints, helper}.rs` | new jackpot chip (Pearl's name for our state chip) | rotate-XOR-13 tests |
| **11** | `pearl_stark.rs::lookups` | `p3_lookup`-based lookup configuration | logUp round trip |
| **12** | `pearl_air.rs` | top-level `composite_full_air.rs::eval` | end-to-end round trip |
| **13** | `pearl_trace.rs` | top-level `composite_trace.rs` | trace generation per (PublicParams, PrivateParams) |
| **14** | `pearl_stark.rs::generate_trace` | wire into `lib::prove` / `lib::verify` | full M10.1c integration test |
| **15** | M11.1 follow-on | PROD bench full shape | report numbers vs Pearl baseline |

Phases 1–10 are independent chips and can be parallelized.
Phases 11–14 require all chips to exist.

## Sizing estimate

| Pearl file | Lines | Direct port estimate |
|---|---|---|
| `pearl_layout.rs` | 91 | ~100 |
| `pearl_preprocess.rs` | n/a (not read) | ~250 |
| `pearl_program.rs` | first 80 (full file longer) | ~400 |
| `pearl_trace.rs` | 352 | ~400 |
| `pearl_stark.rs` | 250+ | ~300 (no Plonky2 recursion plumbing) |
| `pearl_air.rs` | 113 | ~120 |
| `chip/blake3/` (8 files) | 1,500 | ~1,500 |
| `chip/matmul/` (3 files) | 232 | ~300 |
| `chip/jackpot/` (4 files) | 295 | ~350 |
| `chip/input/` + `chip/i8u8.rs` + ranges | 200 | ~250 |
| `chip/control_and_matid_packed.rs` | 132 | ~150 |
| `chip/monotonic_increment.rs` | 49 | ~50 |
| `utils/` | 583 | ~150 (Plonky3 abstracts most of this) |
| **Total** | **~5,800 (Pearl source)** | **~4,300 lines** |

Plus tests at ~1:1 ratio = **~8,500 total lines** of new Plonky3
code, spread across 15 phases.

## Locked decisions (user-confirmed)

1. **Vendored `Blake3KeyedAir` → reference only.** M10.1b's chip
   stays in `src/blake3_chip/` and is kept for byte-equivalence
   KAT testing during M10.1c development. The new Pearl-style
   chip (one BLAKE3 round per row, ~1k cols) replaces it in the
   public API. Tests will cross-check the two against each other
   and against the `blake3` crate.

2. **Adopt Pearl's `NOISED_PACKED` RAM-lookup architecture from the
   start.** Production matmul shapes (e.g.
   [`GEMMA_4_31B_FFN`](../ai-pow/src/params.rs),
   [`QWEN_3_6_27B_FFN`](../ai-pow/src/params.rs)) put `m`, `n` in
   the thousands. With `TILE_H = 2`, each row of A feeds
   `m/2 × n/2 ≈ millions` of output tile cells; inline storage in
   matmul rows would duplicate matrix bytes that many times. The
   RAM-lookup approach stores matrix bytes once in
   `NOISED_PACKED`, then matmul rows pull tile cells via LogUp on
   `MAT_ID`. Memory-per-cell scales with the matrix size, not the
   number of output cells consuming it.

   Test-shape (m = n = tile = 2) overhead is small but non-zero —
   acceptable because designing the MVP without RAM lookups would
   force a major architectural rewrite for production scaling.

   The cryptographic linkage is the same as Pearl's: the
   `NOISED_PACKED` table is read by *both* the matmul chip (via
   `A_NOISED` / `B_NOISED` lookups keyed by `MAT_ID`) and the
   BLAKE3 leaf-hashing rows (via `IS_MSG_MAT` selector +
   `I8U8` conversion table). An adversary cannot diverge the
   matrix bytes between matmul and hash — the lookup forces both
   to read the same `NOISED_PACKED` row.

3. **New `CircuitConfig::TEST_PEARL` profile.** Pearl-style
   chips have degree-3 constraints (Pearl pins
   `constraint_degree() -> 3`). The existing `TEST` profile uses
   `log_blowup = 1` which only admits degree-2 constraints. Add
   `TEST_PEARL` with `log_blowup = 2, num_queries = 16, pow_bits =
   0` — fast for round-trip tests while supporting the M10.1c
   constraint set. `PROD` (`log_blowup = 3`) already handles
   degree 3 with comfortable margin.

4. **`block_commitment` pinned to 32 bytes = 8 × u32.** Matches
   the Tip5 digest size we already use for Merkle commitments.
   In-circuit κ derivation feeds the 32-byte block_commitment as
   the first 8 u32s of the BLAKE3 message, with `params_tag` (also
   32 bytes) as the next 8 u32s. Total 64-byte single-block message;
   one BLAKE3 compression call. The lib's public API stays
   `&[u8]` for back-compat, with `prove` / `verify` asserting
   `block_commitment.len() == 32` up-front.

5. **Recursion deferred.** Plonky3 doesn't ship a recursion ladder.
   M12 picks up the proof-size compression separately; M10.1c lands
   at Pearl's pre-recursion baseline (~1-2 MB at production shape).

## Cross-chip linkage: the `NOISED_PACKED` RAM lookup

Direct port of Pearl's matmul-tile RAM-lookup
(`pearl_stark.rs:167-189`). The single most important LogUp in
M10.1c.

```text
  Table NOISED_PACKED:
    Trace column block holding the canonical noised matrix bytes
    (A + E for A, B + F for B), packed `BYTES_PER_GOLDILOCKS = 4`
    i8 values per Goldilocks element. Two columns × num_dwords
    rows. Each row carries one `dword` (4 bytes) of one matrix.

    Pearl's layout: `NOISED_PACKED: 2` cols, indexed by `MAT_ID`.

  Provers from NOISED_PACKED (= things that read the table):
    matmul chip — `A_NOISED` (8 packed cols) at matmul row s
                  reads the TILE_H × TILE_D bytes of A's tile
                  at row `A_ID[s]`, indexed via `MAT_ID`.
                  Same for `B_NOISED` / `B_ID`.

    blake3 chip — leaf-hashing rows pull `UINT8_DATA` (8 cols)
                  via the `IS_MSG_MAT` selector. The bytes are
                  the same `NOISED_PACKED` row's i8 values
                  converted to u8 by the `I8U8` lookup table.

  LogUp constraints (Pearl's exact pattern):
    indexed_column(A_NOISED + word_idx, A_ID, 1 << 32, idx_shift)
      ≡ indexed_column(NOISED_PACKED + word_idx, MAT_ID, ...)
      ≡ indexed_column(B_NOISED + word_idx, B_ID, ...)

    All gated by `IS_UPDATE_CUMSUM` filter so padding rows don't
    pull from the table.
```

This forces the matmul AIR and the BLAKE3 AIR to read the **same
underlying bytes** from `NOISED_PACKED`. An adversary cannot
substitute fake matrices in matmul rows while feeding real
matrices to the BLAKE3 leaf rows — the LogUp argument enforces
multiset equality (with multiplicity counting via `MAT_FREQ`).

Memory scaling: NOISED_PACKED has `num_a_dwords + num_b_dwords =
(m × k + n × k) / 4` rows total. For
[`GEMMA_4_31B_FFN`](../ai-pow/src/params.rs) (`m = n = 14336`,
`k = 14336`), that's `~100M` rows of NOISED_PACKED — the trace is
matrix-bound, not duplication-bound. Inline storage would have
been `num_tiles × instructions_per_r × per_row_inline_cost` which
scales much worse.

For h_a / h_b binding specifically, the BLAKE3 leaf rows read
`NOISED_PACKED` via `IS_MSG_MAT`. Same table, different consumers.

## Implementation phases

Direct port of Pearl's structure. Each phase mirrors one of Pearl's
files / chips and is independently testable. RAM-lookup architecture
is in scope from the start (production-scale requirement).

| Phase | Pearl reference | Plonky3 deliverable | Tests |
|---|---|---|---|
| **1** | (this doc) | design + phasing | docs commit |
| **2** | `pearl_layout.rs` (range tables + control flags) | `composite_layout.rs` partial + `CircuitConfig::TEST_PEARL` + `block_commitment` 32-byte pinning | const-pinning tests |
| **2.5** | `pearl_layout.rs` (RAM-lookup columns) | Extend `composite_layout.rs` with NOISED_PACKED, A_NOISED, B_NOISED, MAT_ID, AB_ID_PREP, MAT_FREQ, MAT_UNPACK, NOISE_UNPACK, UINT8_DATA, and supporting columns. Full Pearl column-set port. | const-pinning + dimension assertions matching Pearl |
| **3** | `chip/monotonic_increment.rs` | `stark_row` chip (`STARK_ROW_IDX` = 0, 1, 2, …) | round-trip prove/verify |
| **4** | `chip/i8u8.rs`, `chip/{urange8, urange13, irange7p1, irange8}.rs`, `chip/input/` | range-table chips + `INPUT_CHIP` via `p3-lookup` | per-table KAT |
| **5** | `chip/control_and_matid_packed.rs` | `control_chip` — unpack `CONTROL_PREP` selectors AND `MAT_ID` (via limb decomposition through URANGE13 lookups). Full Pearl version. | bitfield + MAT_ID round-trip |
| **6** | `pearl_preprocess.rs` | full preprocessed-trace generation: `CONTROL_PREP`, `NOISE_PACKED_PREP`, `CV_OR_TWEAK_PREP`, `AB_ID_PREP`, `STARK_ROW_IDX` | golden traces |
| **7** | `chip/blake3/blake3_compress.rs`, `logic.rs`, `blake3_layout.rs` | new `chip/blake3/` (one round per row, ~1k cols) | KAT cross-check vs `blake3` crate AND vs M10.1b reference vendored chip |
| **8** | `chip/blake3/trace.rs`, `constraints.rs`, `program.rs` | trace gen + constraint eval | per-instruction round trip |
| **9** | `chip/matmul/{logic, trace, constraints}.rs` | matmul chip with `A_NOISED`/`B_NOISED` RAM-lookup reads keyed on `A_ID`/`B_ID`. NOT a port of M9.1 — different trace shape (one stripe step per row, RAM-fed tile bytes). | tile-correctness tests against M9.1 reference |
| **10** | `chip/jackpot/{logic, trace, constraints, helper}.rs` | new jackpot chip (rotate-XOR-13 state evolution) | rotate-XOR-13 tests against M9.1 state-chip reference |
| **11** | `pearl_stark.rs::lookups` (full) | `p3_lookup`-based lookup configuration: 5 range tables + I8U8 conversion + `NOISED_PACKED` RAM lookup (matmul A/B) + `CV_OUT → CV_IN` BLAKE3 routing lookup | logUp round trip per lookup |
| **12** | `pearl_air.rs` | top-level `composite_full_air.rs::eval` chaining all chips | end-to-end round trip |
| **13** | `pearl_trace.rs` | top-level `composite_trace.rs` (per `(PublicParams, PrivateParams)`) | trace generation matches Pearl shape |
| **14** | `pearl_stark.rs::generate_trace` | wire into `lib::prove` / `lib::verify` | full M10.1c integration test |
| **15** | M11.1 follow-on | PROD bench full shape on small + production-shape matmul | report numbers vs Pearl baseline |

Phases 1–10 are independent and can be parallelized.
Phases 11–14 require all chips to exist.

## Updated sizing estimate (full Pearl port)

Full RAM-lookup architecture in scope. Back to ~Pearl source size:

| Pearl file | Lines | Plonky3 port estimate |
|---|---|---|
| `pearl_layout.rs` | 91 | ~100 (full column set) |
| `pearl_preprocess.rs` | (~250 src) | ~250 |
| `pearl_program.rs` | (~400 src) | ~400 |
| `pearl_trace.rs` | 352 | ~400 |
| `pearl_stark.rs` | ~250 | ~300 |
| `pearl_air.rs` | 113 | ~120 |
| `chip/blake3/` | 1,500 | ~1,500 (one round per row) |
| `chip/matmul/` | 232 | ~300 (RAM-lookup reads) |
| `chip/jackpot/` | 295 | ~350 |
| `chip/input/` + range chips + I8U8 | 200 | ~250 |
| `chip/control_and_matid_packed.rs` | 132 | ~150 |
| `chip/monotonic_increment.rs` | 49 | ~50 |
| `utils/` | 583 | ~150 (Plonky3 abstracts most) |
| **Total Plonky3 port** | | **~4,300 lines** |

Plus tests at ~1:1 = **~8,500 lines** of new code, 15 phases.
Roughly mirrors Pearl's source size, as expected.

## Performance budget (unchanged from previous draft)

| Metric | Pearl (pre-recursion) | Pearl (post-recursion) | Plonky3 port (M10.1c) | Notes |
|---|---|---|---|---|
| Trace width | ~1.3k cols | (recursive) | ~1.2k cols (slightly narrower w/o RAM cols) | one-round-per-row BLAKE3 keeps this narrow |
| Trace height | ≥ 8192 rows | (recursive) | ≥ 8192 rows | same minimum |
| Prove time | seconds | ~30 s end-to-end | seconds (target) | similar |
| Verify time | ms | ~50 ms | ms (target) | similar |
| Proof size | ~1–2 MB | ~60 KB | ~1–2 MB | **recursion gap → M12** |
