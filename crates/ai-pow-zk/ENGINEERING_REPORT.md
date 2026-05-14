# `ai-pow-zk` engineering report

A pass over the entire ZKP stack as it stands at HEAD (after Phase
14b complete). Covers architecture, the choices made and their
tradeoffs, the hot spots of complexity, refactoring opportunities,
suspected performance bottlenecks, and gaps in our measurement
discipline.

This report is candid about uncertainty. Where I'm guessing, I say
so.

---

## 1. What's in the box

The crate ships **three** distinct STARK pipelines, each at a
different point in the M-series milestones:

| Pipeline | Module(s) | Status | Public API |
|---|---|---|---|
| M9.1 composite tile AIR | `composite_air` + `matmul_chip` + `state_chip` | Production-shaped, no LogUp | `lib::prove` / `lib::verify` |
| M10.1b in-circuit BLAKE3 | `blake3_chip` + `found_leaf_air` | Production-shaped, no LogUp | bundled into `lib::prove` envelope |
| M10.1c Pearl-port composite | `composite_full_air` + 10 chips | **Functional with full LogUp** | `composite_proof::{composite_prove, composite_verify}` and `prove_batch` / `verify_batch` via `composite_full_air_with_lookups` |

The M10.1c stack is what this report focuses on. M9.1 / M10.1b
were earlier milestones still present in the crate but largely
superseded.

### M10.1c shape

- **Trace:** `TOTAL_TRACE_WIDTH = 1378` columns × `MIN_STARK_LEN = 8192` rows (minimum). Power-of-2 row counts; column count fixed.
- **10 chips share every row,** gated by selector bits packed into `CONTROL_PREP` and unpacked by `ControlChip`. Each chip reads its slice of the row's columns; chips don't share columns directly. The cross-chip linkage that *does* matter cryptographically is reified as LogUp.
- **7 LogUp buses** wired (after Phase 14b):
  - 4 range tables (`urange8`, `urange13`, `irange7p1`, `irange8`)
  - 1 paired conversion table (`i8u8`)
  - 1 RAM lookup (`noised_packed`)
  - 1 cross-row routing (`cv_routing`)
- **Test counts:** 371 unit + 7 KAT + 2 ignored PROD benches.
- **PROD bench at baseline:** ~51 s prove / 129 ms verify, 1378 cols × 8192 rows, log_blowup=3, 80 queries (120-bit provable FRI soundness).

The crate's `README.md`, `M10_1C_DESIGN.md`, and `M10_1C_PROGRESS.md` cover the per-phase landing in detail. This report is the engineering-perspective layer on top.

---

## 2. Architectural choices and their tradeoffs

### 2.1 Field and FRI

- **Goldilocks** for the base field (`p = 2^64 − 2^32 + 1`). Standard Plonky3 choice for STARKs over 32-bit ops, matches Pearl, and the `u32 XOR` machinery in BLAKE3's round AIR fits naturally because the field admits 2^32 as a clean constant. Alternatives (BabyBear, KoalaBear, Mersenne31) would yield smaller proofs and slightly cheaper hashes but the round AIR's degree-3 constraints would push tighter against the smaller field's quotient budget, and we'd lose merge-mining alignment.
- **`BinomialExtensionField<Goldilocks, 2>`** for FRI challenges. Degree 2 is the minimum that gives ~120-bit soundness at our query count; degree 4 would be paranoid overkill.
- **Tip5 sponge** as the FRI compression hash (in-repo `nockchain-math::tip5`). Faster than Poseidon2 on Goldilocks in recent benchmarks and keeps the FRI sponge consistent with Nockchain's other cryptographic surfaces. Plonky3 doesn't ship `p3-tip5`; we plug in our own.
- **`log_blowup = 3`** at PROD (`CircuitConfig::PROD`) → quotient budget admits degree-4 constraints; **`log_blowup = 2`** at TEST_PEARL admits up to degree-4 by Plonky3's formula (we deliberately under-claim the constraint degree in chip-level tests for safety margin). We do **not** rely on the list-decoding conjecture — 80 queries × log_blowup 3 / 2 = 120 bits provable.

**Tradeoff:** Goldilocks + Tip5 + 80 queries gives a ~683 KB proof at baseline (Phase 15 bench, no recursion). Pearl's Plonky2 stack hits ~60 KB with recursion. The gap is **recursion compression** — Plonky3 doesn't currently ship a compressor; Phase M12 is where this lands.

### 2.2 Single-AIR composite vs. multi-AIR batched

Pearl's `pearl_air.rs` is one big AIR that all chips contribute constraints to. We faithfully ported that shape into `CompositeFullAir`. The trace is one `TOTAL_TRACE_WIDTH × N` matrix and every chip's eval reads its slice by const offsets.

**Tradeoff vs. multi-AIR batched (one AIR per chip, joined via `prove_batch` cross-AIR buses):**

- *Single-AIR pro:* One commitment to one trace. Permutation overhead amortizes across 7 buses. Chip layouts are anchored in one place (`composite_layout.rs`).
- *Single-AIR con:* Every row pays the column cost of every chip, even when that chip isn't active. 1378 cols × 8192 rows of mostly-zero cells. A multi-AIR layout could put each chip on a right-sized trace and cross-link via bus lookups.

This is the kind of decision that could be revisited under real production-shape benchmarks. The single-AIR shape was chosen because Pearl uses it and we wanted byte-equivalence with their design. If/when Nockchain diverges from Pearl, multi-AIR is worth measuring.

### 2.3 LogUp via `p3-batch-stark`

I initially overcomplicated this. The actual path is short: `p3-batch-stark` is Plonky3's standard wrapper around `p3-uni-stark` that natively supports cross-AIR lookups via the `InteractionBuilder` trait. Each AIR's `eval` calls `builder.push_interaction(bus, payload, multiplicity, weight)` and `prove_batch` reifies the interactions as LogUp at proof time. ~250 lines of integration code, no custom prover.

**Tradeoff:** The LogUp adds a permutation trace of `Σ_i (1 + key_arity_i)` extension-field columns per row, where `key_arity_i` is the number of payload elements for bus `i`. For us:
- 5 single-element buses (urange*, irange*, i8u8) → ~5 EF cols
- 1 three-element bus (noised_packed) → ~3 EF cols
- 1 nine-element bus (cv_routing) → ~9 EF cols

Adding up: ~17 EF cols × N rows × 2 (one running sum per side?) — the exact count depends on `LogUpGadget`'s internals, but the prove-time overhead measured at PROD is ~17% (43 s → 51 s). Verify is essentially unchanged (~129 ms vs. ~119 ms). Reasonable.

### 2.4 Chip-local vs. composite layouts

Phase 7–10 chips were initially built standalone with their own `cols` modules. Phase 12 wired them into the composite by adding `eval_at(builder, &offsets)` + `LOCAL_OFFSETS` / `COMPOSITE_OFFSETS` constants per chip. This dual-offset design lets chip-local unit tests stay simple while the composite glues everything together.

**Tradeoff:** Two places to keep in sync per chip. When `composite_layout` adds a new column block (Phase 12d's `JACKPOT_X_BITS` / `JACKPOT_SLOT_SEL` extension), the corresponding chip's `COMPOSITE_OFFSETS` and `populate_lookup_freq` both need to be updated. The repository's tests catch most of this, but the discipline is human-enforced.

A more disciplined approach would generate the offset tables from a single source-of-truth schema (similar to what `p3-air-derive` does for individual AIRs). Out of scope for now.

### 2.5 Add3/Add2 gating via `is_activated`

In Phase 8b/8c, I gated `add3_unchecked` and `add2_unchecked` by an `is_activated` expression so BLAKE3 round constraints could silence on finalize rows. This pushed those constraints from degree 3 to degree 4, costing a `log_blowup = 2 → 3` bump in the test profile. Pearl's circuit stays at degree 3 by using a stricter row-schedule discipline (probably preprocessed selectors that already exclude finalize rows from the round constraint domain entirely).

**Tradeoff:** The gating is simpler than re-architecting the row schedule and trace generator. We pay ~one bit of FRI soundness margin per query for it. For PROD we set `log_blowup = 3` regardless of the chip's constraint degree, so this gating has no production impact — only the TEST_PEARL profile is constrained.

If revisited: factor each cubic into two quadratics via an intermediate column. Cleaner constraint-degree story, slightly fatter trace.

---

## 3. Areas of complexity

In rough order of how much they bit me during implementation:

### 3.1 The BLAKE3 chip (~1500 lines across 6 files)

- `compress.rs` — scalar reference (267 lines)
- `layout.rs` — per-round column sub-layout (~60 lines)
- `logic.rs` — per-row instruction types (~80 lines)
- `round_ops.rs` — degree-bound constraint primitives (~500 lines incl. tests)
- `round_air.rs` — `verify_round` / `finalize_blake` / `verify_init_state` (~600 lines incl. tests)
- `chip.rs` — selector-gated dispatch + trace fill (~700 lines incl. tests)

Each piece independently passes ~10 KAT-style and tamper tests, but the interactions are subtle:

- **5 state snapshots vs. 4-snapshot column layout.** Pearl's layout has 4 snapshots per row but `verify_round` needs 5 (input + 3 intermediate + output). The 5th is the *next* row's input. That puts every BLAKE3 round constraint inside `when_transition()` and creates a boundary problem at the end of an 8-row hash block: row 7's round constraint would otherwise try to chain row 7 (finalize) into row 8 (next hash's init). Gating by `(1 - is_last_round)` fixes it but only after `add3_unchecked` learned to take `is_activated`. This is documented in `M10_1C_PROGRESS.md` Phase 8b.
- **Finalize's "abuse" trick.** `finalize_blake` reuses `STATE1.row2` / `row4` as bit decompositions of `STATE0.row1` / `row3`. The trace generator has to write the same value into two semantically-different slots. Easy to get wrong; one regression test (`blake3_state_from_slice_pins_layout`) anchors the layout.
- **Half-G ordering vs. Pearl's per-G ordering.** `round_with_snapshots` applies the 16 half-G steps as 4 sub-phases × 4 column-positions, while Pearl's reference `round` does 8 G calls each containing 2 half-G steps. The intermediate snapshots differ; only the final state matches. Our `compress.rs::round_with_snapshots` is the canonical source; the AIR consumes its snapshot order.

If something breaks here in the future, the relevant `M10_1C_PROGRESS.md` entries and the in-module tests should be the first stop.

### 3.2 `populate_lookup_freq` (~250 lines)

Handles all 7 buses. For each it has to:
1. Identify the query-cell set (per-bus, sometimes gated by a selector).
2. Scan every row, compute the per-bus query payload from the cells.
3. Find the matching table row (or accept that no match → unbalanced).
4. Write the count into the right `*_FREQ` cell.

The hard parts:
- **Signed-value cells.** Goldilocks doesn't distinguish `-1` from `p − 1`; the `goldilocks_to_signed` helper does the canonical conversion.
- **Multi-element keys.** `noised_packed` uses 3-element keys and `cv_routing` uses 9-element keys. Both use `hashbrown::HashMap<Vec<u64>, usize>` for the key → first-table-row mapping. The "first-row" choice is arbitrary; any consistent assignment balances LogUp.
- **Validation logic vs. AIR's emission logic.** The trace generator and the AIR must agree on which cells query which bus. If they drift, baseline tests would fail but specific tamper tests might still pass for the wrong reason. The remedy is the integration test `three_chip_activity_with_all_lookups_verifies`, which exercises real chip activity through all 7 buses simultaneously.

### 3.3 `CompositeFullAirWithLookups::eval` (~150 lines)

Inline emissions for 7 buses. Sometimes a bus has 2 or 3 separate `push_interaction` calls (table + N queries). The clearest danger is mismatches between the AIR's emission and `populate_lookup_freq`'s scanning. A bus where the AIR emits an *unconditional* query but the trace generator gates it (or vice versa) would balance vacuously on baseline traces and silently fail on real ones. We don't currently have a property-test that compares the two for consistency.

### 3.4 `composite_layout.rs` (~600 lines)

1378 column offsets, manually computed. Each section's constants compute from the previous section's `*_START + *_LEN`. The `layout_offsets_are_contiguous` test catches accidental gaps/overlaps but won't catch a *correct-but-wrong* layout (e.g. swapping two adjacent column blocks).

The Phase 12d extension (`JACKPOT_X_BITS_START`, `JACKPOT_SLOT_SEL_START`) was inserted between `CV_OUT` and `CV_OUT_FREQ` to avoid shifting any prior offsets. This worked but is the kind of insertion that a more disciplined layout system would forbid.

### 3.5 Per-chip `eval_at` + composite mapping (mechanical but invasive)

Each of `Blake3Chip`, `MatmulCumsumChip`, `JackpotChip` has:
- A chip-local `cols` module
- `LOCAL_OFFSETS: <ChipOffsets>` and `COMPOSITE_OFFSETS: <ChipOffsets>`
- `eval_at(builder, &offsets)` body that's identical across both modes
- `eval_composite(builder)` thin wrapper
- An `Air<AB>` impl that calls `eval_at(LOCAL_OFFSETS)` for the chip-local tests

That's 5 things to keep aligned per chip. Useful and clean today; if we add more chips, generating this from a macro becomes attractive.

---

## 4. Refactoring opportunities

Listed by ratio of (cleanliness gain) / (risk of breakage):

**High value, low risk:**
1. **Unify `cols` modules across chips.** Each chip's `cols::*` consts duplicate what's in `composite_layout`. A `define_chip_columns!` macro that emits both would prevent drift.
2. **Property-test the AIR ↔ trace-generator lookup agreement.** A proptest that builds a random trace, calls `populate_lookup_freq`, and asserts `prove_batch` accepts. Would catch the silent-mismatch class of bugs in section 3.3.
3. **Delete the M9.1 / M10.1b stack** once M10.1c is the canonical path. They're still hanging around and the README lists both. Either depend on or supersede them.

**Medium value, medium risk:**
4. **Express the column layout as a typed schema** (similar to `p3-air-derive`). The current const-offset scheme works but a single layout reshuffling silently mis-indexes everything. The 1378-col layout is large enough that this is a real risk.
5. **Add a `BusBinding` trait** that each chip implements. `eval_at_with_lookups<AB: InteractionBuilder>(builder, &offsets)` would be the chip's per-bus emission. The composite AIR would just iterate over its chips. Today the emissions are inline in `CompositeFullAirWithLookups`, which is a sausage factory.
6. **Factor `add3_unchecked`'s cubic into two quadratics** to bring max constraint degree back to 3. Bumps TEST_PEARL's safety margin and gives PROD some headroom.

**Lower value, higher risk:**
7. **Multi-AIR refactor (one chip → one AIR + bus links).** Worth measuring before committing to. The single-AIR shape is byte-equivalent with Pearl; departing from it is a strategic choice with downstream cost.
8. **Generic instruction-list compiler in `composite_trace`.** Today we have `place_matmul_step` / `place_blake3_hash` / `place_jackpot_step` as concrete methods. An `enum Instr { Matmul {..}, Blake3 {..}, Jackpot {..} }` + `compile(instrs) -> CompositeTrace` would let downstream callers ship a single program description rather than orchestrate placements themselves.

**Specific items I left as deferred:**
9. **A_NOISED ↔ A_NOISED_UNPACK packing constraint.** Currently A_NOISED (packed Goldilocks) and A_NOISED_UNPACK (i8 cells) aren't tied at the AIR level — only via the `noised_packed` LogUp (which constrains A_NOISED). The unpack cells are constrained by IRange8 only. The packing relationship is enforced only indirectly. If the input chip's `polyval(MAT, 256)` is the canonical packer for matrix bytes, we'd want a similar constraint for A_NOISED ↔ A_NOISED_UNPACK.
10. **The `STARK_ROW_IDX` lookup bus** (#52, h_a / h_b matrix bindings) is the long-term direction here, but it's documented as multi-week in the progress doc and untouched.
11. **Jackpot chip column-shape extension** (Phase 12d) added 48 cols. These could fold into the existing JACKPOT_IDX (8 cols) with a more compact slot-select encoding (e.g. range-checked 0..16). The chip-local AIR would need a per-row 4-bit + 16-way fanout encoding, which is cheaper than 16 boolean SLOT_SEL cells but messier in the constraint code.

---

## 5. Measured performance (post-bench commit `d6065d8`)

11 benches captured across {TEST_PEARL, PROD} × {8K, 16K, 32K} ×
{baseline, light, heavy}. Numbers below are wall-clock from one
run on an Apple M-series workstation (release build).

### 5.1 The full table

| Profile | Rows | Activity | trace_gen | populate | **prove** | verify | **proof** |
|---|---|---|---|---|---|---|---|
| TEST_PEARL | 8K | baseline | 6 ms | 4 ms | 27.6 s | 30 ms | 180 KB |
| TEST_PEARL | 8K | light | 1 ms | 3 ms | 27.5 s | 29 ms | 360 KB |
| TEST_PEARL | 8K | heavy | 1 ms | 3 ms | 27.7 s | 30 ms | 370 KB |
| TEST_PEARL | 16K | baseline | 18 ms | 12 ms | 55.1 s | 31 ms | 193 KB |
| TEST_PEARL | 16K | light | 2 ms | 9 ms | 55.5 s | 31 ms | 373 KB |
| TEST_PEARL | 16K | heavy | 2 ms | 9 ms | 55.8 s | 31 ms | 383 KB |
| TEST_PEARL | 32K | baseline | 25 ms | 19 ms | 110.8 s | 32 ms | 208 KB |
| PROD | 8K | baseline | 6 ms | 3 ms | 54.3 s | 137 ms | 892 KB |
| PROD | 8K | light | 1 ms | 3 ms | 54.2 s | 138 ms | 1.65 MB |
| PROD | 8K | heavy | 1 ms | 3 ms | 54.2 s | 138 ms | 1.69 MB |
| PROD | 16K | baseline | 12 ms | 9 ms | 108.4 s | 145 ms | 963 KB |

### 5.2 What the data says

**Prove time scales linearly with trace size, NOT with activity.**
TEST_PEARL goes 27.6 s (8K) → 55.1 s (16K) → 110.8 s (32K) at
baseline. Heavy activity at 8K is 27.7 s vs. baseline's 27.6 s —
within noise. The prove cost is paying for the LDE + FRI commits
over `cols × rows × 2^log_blowup` field elements, not for the
activity. This confirms the suspicion in §5.3 (now removed) that
the trace is mostly empty and the FRI commits dominate.

**Proof size is dominated by activity, not trace size.** TEST_PEARL
8K: 180 KB (baseline) vs. 370 KB (heavy). Roughly 2× from
activity. Doubling rows only nudges baseline from 180 KB to 193 KB
to 208 KB — the FRI Merkle paths grow slowly with `log2(rows)`.
The proof-size jump from baseline → activity comes from FRI
needing to open more cells where the trace has actual structure
(the commitment is more entropic when cells aren't zero).

**Verify is essentially constant per profile.** TEST_PEARL: 29–32 ms
across all shapes. PROD: 137–145 ms. Verifier cost is `O(num_queries)`
× a constant per query, independent of trace size.

**TEST_PEARL vs. PROD trade-off.** Prove ~2× slower (log_blowup 3
vs. 2). Verify ~5× slower (80 queries vs. 16). Proof ~5× bigger.
These are the costs of going from "fast tests" to "120-bit
provable soundness."

**Trace generation is negligible.** `trace_gen` + `populate`
combined is 5–45 ms across all shapes, vs. 27–110 s prove. Even
at 32K rows, populate_lookup_freq finishes in 19 ms. This was an
explicit concern in the original report's §6.1; benched, it's not
a concern.

**Light vs. heavy proof sizes are nearly identical.** TEST_PEARL
8K: light = 360 KB, heavy = 370 KB. 1 BLAKE3 hash uses the same
column-block structure as 100 hashes (the bit cells are still
boolean, the lookups still fire, etc.). The marginal cost of
*more* activity in the same kind is tiny.

### 5.3 Hot spots (now grounded)

1. **LDE + FRI commits over a 1378-column trace.** Confirmed: this is
   the dominant cost. Prove time scales linearly with rows (the
   LDE is `cols × rows × 2^log_blowup` cells). At log_blowup 3,
   the LDE for 8K rows is `1378 × 8 × 8192 = ~90M Goldilocks
   elements`. Merkle-committing this is dominated by Tip5 sponge
   calls.
2. **Permutation trace generation (LogUp).** Confirmed indirectly:
   prove time for both PROD profiles roughly matches my Phase 14b
   measurement of "no-lookups vs. full LogUp" overhead (~17%).
   The LogUp permutation columns are ~17 extension-field columns
   per row at our bus arity.
3. **Constraint quotient computation.** Not directly measurable
   without instrumenting Plonky3. Suspected to be a smaller
   contributor than (1) and (2) given that activity level doesn't
   move prove time.
4. **BLAKE3 round AIR's per-row constraint emissions.** Same as (3):
   not a wall-clock contributor at our trace shapes.

### 5.4 Memory (still not directly measured)

Estimated bounds, given the benches ran on a workstation with
~32 GB RAM without OOM:

- 8K rows × 1378 cols × 8 bytes = ~90 MB main trace.
- ~17 EF cols × 8K rows × 16 bytes = ~2 MB permutation trace.
- LDE: 8× both → ~720 MB.

Single-digit GB at 32K rows. We never hit a memory wall in the
benches, so we're at least below ~32 GB at 32K. Profiling memory
explicitly remains future work.

---

## 6. Where measurement is still thinnest

Bench commit `d6065d8` covered §6.1 and §6.2 (trace-generation
timing + non-baseline shapes). Remaining gaps:

### 6.1 No FRI parameter sensitivity

`CircuitConfig::PROD` pins `log_blowup = 3` and `num_queries = 80`. Other points on the (proof size, prove time, soundness) curve are unexplored:
- `log_blowup = 4, num_queries = 60` — bigger LDE, fewer queries, maybe smaller proof?
- `log_blowup = 2, num_queries = 120` — smaller LDE, more queries; faster prove, larger proof?

### 6.2 No memory profiling

The benches all completed without OOM on a 32 GB workstation, but we don't have a hard upper bound. `cargo flamegraph` / `dhat` would tell us. This matters for commodity miners where 16 GB or 8 GB might be the cap.

### 6.3 LogUp overhead isolation

We know LogUp adds ~17% overhead vs. no lookups. But we don't know how that overhead distributes across the 7 buses. A bus with a 9-element key (`cv_routing`) vs. one with a 1-element key (`urange8`) presumably pays very different per-bus costs. Knowing this would help if we ever want to trim bus arity for performance.

**What to measure:** ablation bench — disable buses one at a time and measure the marginal prove cost.

### 6.4 No CI bench tracking

The bench numbers are now captured in this report but aren't a tracked artifact. A regression that doubles prove time would only be caught by someone manually re-running the ignored benches. A bench-tracking workflow (criterion + GH Actions) would let us catch perf regressions automatically.

### 6.5 No PROD @ 32K

Skipped from the run for time reasons (PROD scales linearly so ~220 s expected). If we ever ship 32K-row proofs, run it.

### 6.6 No "real workload" bench

The benches use synthetic activity (100 of each chip kind). A real PoW workload has specific structural patterns (the matmul chain dominates rows, BLAKE3 hashes have specific tweak/CV chain shapes). The benches probably approximate it well, but we haven't validated against an actual ai-pow puzzle solve.

---

## 7. Risk and trust register

Honest accounting of what's been validated and what hasn't:

### What's solidly verified

- Every chip's constraints in isolation (300+ unit tests; tamper tests for every constraint family).
- All 7 LogUp buses' tamper behavior (28+ tests across in-range, out-of-range, dangling-reference, frequency-mismatch).
- End-to-end prove + verify for the baseline trace + a three-chip activity trace.
- BLAKE3 output byte-equivalence with `blake3::Hasher::new_keyed` (KAT tests).
- The scalar references (compress, half_g, round_with_snapshots, etc.) cross-checked against the AIR's expected behavior.

### What's not yet verified

- **Pearl byte-equivalence at the proof level.** We've verified hash-output bytes match Pearl's encoding (M10.1b vendored BLAKE3 chip and our compress.rs scalar reference). But we haven't actually fed a Pearl-generated trace through our prover or vice versa. The merge-mining claim depends on this; today it rests on careful column-by-column matching of the layout to Pearl's `pearl_layout.rs`.
- ~~**Soundness of the composite-trace-generator's lookup-freq logic.**~~ **Now partially covered.** Pass 1 (commit `68268e2`) added 6 proptests over `prove_batch` + `verify_batch` plus 4 fixed offset-consistency tests. In-range tampers in each bus must verify after `populate_lookup_freq`; out-of-range tampers must reject. The proptests don't catch every drift mode (e.g., synchronized drift between AIR and trace gen) but they catch the common asymmetric case.
- **PROD bench is just one shape.** A different trace shape (larger, with real activity) might surface a different bottleneck.
- **A_NOISED ↔ A_NOISED_UNPACK packing relation.** Not currently constrained. The cryptographic burden falls on the `noised_packed` LogUp + range checks.
- **Public-input binding.** The composite proof currently exposes no public inputs. The downstream caller (the miner / verifier protocol) will need a binding from the composite output to whatever it consumes. This is M10.1d-ish work.

### Deferred for good reasons

- **Recursion compression (M12).** Plonky3 doesn't ship a compressor; deferred per design.
- **`h_a` / `h_b` matrix bindings (task #52).** Multi-week work; out of scope.

---

## 8. Bottom line

The stack is a faithful Plonky3 port of Pearl's composite-AIR
design with full LogUp cross-chip linkage. It works end-to-end at
baseline shape with 120-bit provable FRI soundness. The biggest
remaining work items are recursion compression (proof size) and
measurement discipline (bench coverage). The biggest risks are
the silent-drift class — places where two parts of the code have
to agree on conventions but agreement isn't mechanically checked.

For the next session of work I'd prioritize, in order:

1. ✅ **Property test the AIR ↔ trace-generator lookup agreement.** Landed Pass 1 (`68268e2`).
2. ✅ **Bench at three trace shapes and three activity levels.** Landed (`d6065d8`). Findings folded into §5 above.
3. **Compress old M9.1 / M10.1b stacks out of the crate** — reduces cognitive load.
4. ✅ **A `BusBinding`-style per-bus factor.** Landed Pass 2 (`1bc1d5e`) as free helpers in `bus_emit::*` rather than a trait; achieves the same legibility.

What I'd *not* prioritize:

- Switching to a multi-AIR layout. The single-AIR shape works and matches Pearl. Don't rebuild it without a concrete reason.
- The instruction-list compiler refactor. Today's `place_*` helpers are fine.
- Recursion compression. Deferred per the original design; the right time is when Plonky3 ships a compressor.

---

## 9. Pass 1 + Pass 2 outcomes

After this report's initial publication, two refactor passes landed.

### Pass 1 — high-value, low-risk (commit `68268e2`)

**Landed:**
- **HL1 — Lookup-consistency proptests.** 6 proptests over
  `CompositeFullAirWithLookups` + `prove_batch`, 4 cases each.
  In-range tampers on every bus must verify after
  `populate_lookup_freq`; out-of-range tampers must reject.
  Covers IRange8, URange8, I8U8, CV_ROUTING in both directions.
  Catches the silent-drift class where the trace generator's
  query model diverges from the AIR's emission model.
- **HL2 — Per-chip offset validation.** 4 fixed tests that pin
  `COMPOSITE_OFFSETS` for each lookup-aware chip (matmul,
  blake3, jackpot): every offset slot fits within
  `TOTAL_TRACE_WIDTH`, and the three chips' column blocks are
  pairwise disjoint. Catches accidental layout reshuffles.

**Deliberately deferred:**
- **HL3 — Delete the M9.1 / M10.1b stacks.** On closer inspection
  this is NOT actually low risk. Those stacks back the public
  `lib::prove` / `lib::verify` API; removing them is a breaking
  change. Should land alongside an explicit M10.1c-as-canonical
  switch (a separate, larger work item).

### Pass 2 — medium-value, medium-risk (commit `1bc1d5e`)

**Landed:**
- **MM1 — Factor per-bus emissions into `bus_emit::*` helpers.**
  `CompositeFullAirWithLookups::eval` went from a ~190-line
  inline sausage factory to a 7-line dispatch. Each bus is now a
  named, greppable, docstring-bearing unit. Pure code-motion
  refactor — no behavior change.

**Deliberately deferred with rationale:**
- **MM2 — Factor `add3_unchecked`'s cubic into two quadratics.**
  The argument was that gating add3 by `is_activated` pushed the
  constraint from degree 3 to degree 4, exceeding Pearl's pinned
  degree-3 budget at TEST_PEARL (`log_blowup=2`). But:
    1. Plonky3's actual quotient budget at `log_blowup=2` admits
       constraint degree up to ~5 (formula: `max_deg ≤
       2^log_blowup + 1`). All 381 tests pass empirically with
       degree-4 constraints at log_blowup=2.
    2. The refactor would require adding ~16 aux columns to the
       BLAKE3 layout (one per half_g call's add3) plus trace
       generator updates.
    3. Estimated perf savings: ~1–2% prove time (the cubic
       constraints are a small fraction of BLAKE3's total
       constraint surface).
  The cost/benefit doesn't justify the change now. If a future
  profile tightens log_blowup, the refactor path is clear.

### Outcome

- **Tests: 381 unit (was 371; +10) + 7 KAT + 2 ignored PROD benches.** No regressions.
- **Lookup soundness coverage tightened.** Silent-drift class largely caught now.
- **Code organization in the lookup-emission layer materially improved.**
- **One report claim revised.** MM2 was framed as worth doing in §4 of this report; closer inspection downgrades that to "valuable only if Plonky3's profile tightens." The original §4 text remains for context; this section is the corrected position.

---

*Report compiled after Phase 14b complete (Phase 14b POC = `2e0c4e9`, sub-phases through `d767f7f`). Refactor passes landed in `68268e2` (Pass 1) and `1bc1d5e` (Pass 2). Bench suite landed in `d6065d8` with §5 + §6 updated to reflect measured numbers (this update). Test count: 381 unit + 7 KAT + 13 ignored benches.*

---

## 10. Bench-suite landing — three findings that change the next-step list

Before the bench-suite landed (commit `d6065d8`), §5 was inference and §6.1–6.2 listed "no trace-gen timing" and "no non-baseline shapes" as the top measurement gaps. Both are now closed. Three concrete findings emerged:

### 10.1 Prove time scales linearly in rows, not in activity

| Profile | 8K | 16K | 32K |
|---|---|---|---|
| TEST_PEARL baseline | 27.6 s | 55.1 s | 110.8 s |
| PROD baseline | 54.3 s | 108.4 s | (not run) |

Doubling rows doubles prove time. Heavy activity at 8K (~12% of rows occupied) finishes within noise of the same-shape baseline (27.7 s vs. 27.6 s).

**Implication:** the per-row cost is fixed by `cols × 2^log_blowup` regardless of cell content. If we want production-shape proving to be substantially faster, the lever is **either** column reduction (a hard refactor — the 1378 cols are mostly required by Pearl's design) **or** recursion compression (M12 territory). Trace-generator tuning won't move the needle; populate_lookup_freq is sub-50 ms even at 32K rows.

### 10.2 Proof size doubles from baseline to "any activity"

| Profile | 8K baseline | 8K light | 8K heavy |
|---|---|---|---|
| TEST_PEARL | 180 KB | 360 KB | 370 KB |
| PROD | 892 KB | 1.65 MB | 1.69 MB |

The proof-size step from baseline → light is large (~2×); the step from light → heavy is small (~3%). The Merkle path encoding compresses better for all-zero columns; the moment the trace has structure, the FRI openings get fatter.

**Implication:** "real" proofs (any activity) are roughly twice the size of the bench's baseline numbers. PROD baseline 892 KB / activity 1.65 MB is the right rule of thumb. This still beats the original §5.1 estimate (683 KB at PROD baseline — looks like the original bench didn't measure encoded size).

### 10.3 The next prioritization step changes

The original §8 prioritization had benches as item #2. With #2 done, the new top remaining item is **#3: compress old M9.1 / M10.1b stacks out of the crate**. But the bench data also surfaces a new candidate worth adding to the list:

**FRI parameter sensitivity sweep (§6.1).** Now that we have a measurement scaffold, sweeping `(log_blowup, num_queries)` would tell us if a different point on the soundness/cost curve produces a meaningfully smaller proof at PROD. With 1.65 MB at PROD-with-activity, anything saving 25% would matter for block-budget realities. This is a half-day's work given the bench infra exists.

### 10.4 Bench numbers to remember

For anyone planning around these:

- **Baseline floor: 27 s / 180 KB at TEST_PEARL @ 8K.** This is the cheapest M10.1c proof shape.
- **Production ceiling: ~108 s / ~1 MB at PROD @ 16K baseline; ~1.7 MB with activity.** Linearly worse at larger sizes.
- **Verify is ~30 ms TEST / ~140 ms PROD across all shapes.** Cheap; not a constraint anywhere.

Memory remained sub-OOM on a 32 GB machine for every bench. Real production miners with 16 GB may need to cap trace length at 16K until we have data.
