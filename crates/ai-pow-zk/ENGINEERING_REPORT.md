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

## 5. Suspected performance bottlenecks

These are *suspected*. We have one PROD bench at baseline shape; everything else is inference.

### 5.1 The PROD bench numbers, again

```
baseline @ MIN_STARK_LEN = 8192 rows × 1378 cols
                           PROD (no lookups)   PROD (full LogUp)
prove                      43.3 s              50.9 s
verify                     119 ms              129 ms
proof size                 ~683 KB             (not measured under batch-stark)
```

For comparison, Pearl's published Plonky2 numbers at production shape are ~60 KB final proof size with recursion. We're 11× larger because (a) no recursion compression yet (deferred to M12), and (b) we're using `p3-uni-stark` / `p3-batch-stark`, not Plonky2.

### 5.2 Hot spots in `prove_batch`

Educated guesses, in rough order of suspicion:

1. **LDE + FRI commits over a 1378-column trace.** The blowup is `2^log_blowup = 8`. So the LDE is 1378 × 8 × 8192 = ~90M field elements. Merkle-committing this is dominated by Tip5 sponge calls. This is where the bulk of the 43–51 s prove time goes.
2. **Permutation trace generation (LogUp).** 7 buses × N rows × extension-field arithmetic. The 17% overhead from no-lookups to full LogUp is dominated by this.
3. **Constraint quotient computation.** 1378 cols, max constraint degree 4 (gated `add3_unchecked`) means the quotient polynomial has degree ~3N. Evaluating it on the LDE is ~3N × 8 evaluations. Lots of polynomial multiplication.
4. **The BLAKE3 round AIR's 16 half_g calls.** Each emits ~5 constraints (add3, xor_shift, add2, xor_shift, plus the booleanity). Times 8192 rows. ~640K constraint evaluations per row of the LDE.

### 5.3 Why the prove time might surprise

The trace is **mostly empty.** Baseline has all data columns at zero. Most of the 43 s is paying for cells that are zero. A real workload (say 1000 BLAKE3 hashes + 1000 matmul steps = 8000 active rows of 8192) wouldn't necessarily be much slower — the FRI commits scale with trace size, not with how interesting the cells are.

**This is something to actually measure.** Section 6 below.

### 5.4 Memory

8192 × 1378 = ~11M Goldilocks × 8 bytes = ~90 MB main trace. The permutation trace adds ~17M extension-field elements × 16 bytes = ~270 MB. The LDE is 8× both. We're looking at single-digit GB of prover memory for the baseline shape. Big traces (e.g. 16K or 32K rows for a real PoW shape) would push this up linearly.

Memory is *probably* fine on a workstation but could become the limit on commodity miners.

---

## 6. Where measurement is thinnest

The PROD bench is the only quantitative point we have. Everything else is "tests pass." Specific gaps:

### 6.1 No trace-generation timing

`populate_lookup_freq` scans every row × every queried column. The hashmaps (for `noised_packed` and `cv_routing`) allocate. We don't know how long this takes vs. the actual proving. A miner that's running this loop wants to know if trace generation is 10% of prove time or 90%.

**What to measure:**
- `Instant::now()` around the trace-builder calls (place_blake3_hash, place_matmul_step, populate_lookup_freq).
- Separate from prove + verify.

### 6.2 No bench at non-baseline shapes

- Trace lengths: 8192 (current), 16384, 32768.
- With real chip activity: 1000 BLAKE3 hashes, 1000 matmul steps, 1000 jackpot rotations.
- "Empty trace" vs. "fully-occupied trace" comparison.

### 6.3 No FRI parameter sensitivity

`CircuitConfig::PROD` pins `log_blowup = 3` and `num_queries = 80`. Other points on the (proof size, prove time, soundness) curve are unexplored:
- `log_blowup = 4, num_queries = 60` — bigger LDE, fewer queries, maybe smaller proof?
- `log_blowup = 2, num_queries = 120` — smaller LDE, more queries; faster prove, larger proof?

### 6.4 No memory profiling

Whether 4 GB or 16 GB is the threshold for the PROD bench matters for production scaling. `cargo flamegraph` / `dhat` would tell us. Right now we don't know.

### 6.5 LogUp overhead isolation

We know LogUp adds ~17% overhead vs. no lookups. But we don't know how that overhead distributes across the 7 buses. A bus with a 9-element key (`cv_routing`) vs. one with a 1-element key (`urange8`) presumably pays very different per-bus costs. Knowing this would help if we ever want to trim bus arity for performance.

**What to measure:** ablation bench — enable buses one at a time and measure the marginal prove cost.

### 6.6 No CI bench tracking

The bench number is recorded in PROGRESS.md but isn't a tracked artifact. A regression that doubles prove time would only be caught by someone manually re-running the ignored bench. A bench-tracking workflow (criterion + GH Actions) would let us catch perf regressions automatically.

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
2. **Bench at three trace shapes (8K, 16K, 32K) and three activity levels** — sets a real perf baseline.
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

*Report compiled after Phase 14b complete (Phase 14b POC = `2e0c4e9`, sub-phases through `d767f7f`). Refactor passes landed in `68268e2` (Pass 1) and `1bc1d5e` (Pass 2). Test count: 381 unit + 7 KAT + 2 ignored PROD benches at time of latest update.*
