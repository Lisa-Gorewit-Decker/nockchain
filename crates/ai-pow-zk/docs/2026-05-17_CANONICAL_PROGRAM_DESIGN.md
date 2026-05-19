> _Created **2026-05-17** · last updated **2026-05-18** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# Phase A-CR — first-class params-pure `canonical_program` (CRIT-1 reconstruction hardening; subsumes §4.C.2-b2)

> **Status:** DESIGN (2026-05-17). New roadmap milestone,
> sequenced **after Phase A, before Phase B proper**
> (`2026-05-17_PRODUCTION_ROADMAP.md` §2). This is "b2" from the §4.C.2
> decision (`2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md`) **promoted to
> its true scope**: a CRIT-1-wide upgrade of the verifier's
> canonical-program reconstruction from *"`extract_program` of a
> reference honest trace"* to a **witness-free, params-pure,
> independently-auditable `canonical_program(params,
> block_public)`**. §4.C.2's store-row noise pin is one part of
> it. **Phase A still ships §4.C.2 on b1** (sound — no weaker
> than any existing PROGRAM_COL pin); Phase A-CR then subsumes
> b1's noise pin into the first-class reconstruction and hardens
> *all five* PROGRAM_COLS the same way.
> **Governed by `~/.claude/CLAUDE.md` R1** (don't rush
> soundness-critical invasive work; staged; validated subset +
> precise residual). This is the **most soundness-sensitive
> surface in the codebase** (the CRIT-1 PoW-forgery linchpin).
> **Cross-refs:** `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md` (§4.C.2 /
> b1), `2026-05-17_P_B2_STRIP_OPENING_DESIGN.md` (A1 `tile_chunk_range`),
> A3.2a `noised_store_layout`, `2026-05-15_ZKP_SECURITY_REPORT.md` CRIT-1,
> `composite_proof.rs` (`composite_setup`, `extract_program`,
> `crit1_*`), `noise_ref` (A3.0).

---

## 0. The problem (precisely)

CRIT-1's soundness claim: *the verifier rebuilds the canonical
program from trusted public params and commits to it in the VK;
the prover cannot choose the pinned schedule.* The five
PROGRAM_COLS — `CONTROL_PREP` (21 selectors + 26-bit MAT_ID +
fold/G2 bits), `NOISE_PACKED_PREP`, `CV_OR_TWEAK_PREP`,
`AB_ID_PREP`, `STARK_ROW_IDX` — are committed as a preprocessed
trace; `CompositeFullAirPinned` enforces `main[col] ==
preprocessed[k]` unconditionally.

**But the verifier's canonical program is currently obtained by
`extract_program(&trace)`** (`composite_proof.rs`): in the
bridge roundtrip (`prove_and_verify` →
`composite_prove_pinned_logup_sx`) the program is extracted from
the *prover's* trace and handed to verify. The "verifier
independently rebuilds from `ZkParams`" property is **only
exercised by the `crit1_*` adversarial suite**, where a
*reference honest trace* is built, `extract_program`'d to a
canonical, and a forged trace is shown to be rejected against
it. So the thing that *defines what is pinned* is an **emergent
property of a ~350-row-class honest-trace construction**, not a
small pure function — and the production verify path does not
yet independently rebuild it at all.

For §4.C.2 this is acute: the store-row `NOISE_PACKED_PREP` must
be the Pearl noise `noise_ref(s_a)` (the steelman below). Under
b1 that holds *iff the reference honest builder placed it
correctly* — a shared-bug blind spot (prover and reference share
the construction; all tests green; the pin pins to the wrong
value). For a cryptographic-soundness linchpin under R1, the
artifact that defines the pinned values should be a tiny,
pure, separately-red-teamed function.

## 1. Why (the steelman, formalized)

1. **Witness-free verifier.** A SNARK verifier (and *a fortiori*
   a recursive verifier / chain node — Phase C/D) must be cheap
   and must not require the prover's witness. "Canonical =
   `extract_program` of a reconstructed honest trace" forces the
   verifier to re-run the prover. `canonical_program(params,
   block_public)` is an `O(#rows)` pure function — the verifier
   CRIT-1 docs *claim* and Phase C/D *require*.
2. **Isolation-auditable defining artifact.** `canonical_program`
   can be KAT'd / red-teamed *with no trace at all* (e.g.
   `canonical_program(...).store_rows.noise_packed ==
   polyval(noise_ref(s_a,·),129)`). The defining artifact of a
   soundness pin should be independently auditable, not an
   emergent property of a 350-test construction with shared-bug
   risk.
3. **Crisp soundness theorem.** With it, §4.C.2 is statable as a
   chain of named, independently-tested artifacts:
   `canonical_program` fixes store noise = `noise_ref(s_a)`
   (pure fn of C1-pinned public seed) → CRIT-1 `main==preproc`
   → InputChip `NOISE_PACKED_PREP==polyval(NOISE_UNPACK,129)` &
   `NOISED_PACKED==MAT_UNPACK+NOISE_UNPACK` → swept `a′ =
   committed-plain + Pearl-noise`. Every link auditable in
   isolation.
4. **Downstream necessity (free-rides on required work).** P-C
   (M-S5 succinct/recursive certificate) and M-C1 (consensus)
   have **no prover trace**; a recursive verifier / chain node
   *must* recompute the canonical program/VK from public params.
   Phase A-CR builds that reusable primitive once; A3.2a's
   witness-free `noised_store_layout` is exactly the schedule it
   needs. Deferring it means building it later anyway and
   possibly reworking how §4.C.2's noise pin is established.

## 2. Scope

**CRIT-1-wide**, not §4.C.2-local. All five PROGRAM_COLS'
per-row values become a pure function of public data:

| PROGRAM_COL | params-pure source (verifier-known public data) |
|---|---|
| `CONTROL_PREP` | the row-class schedule: matrix strip-opening (A1 `tile_chunk_range`) + key-pin (C1) + §6(b)-G1/G2 sweep (sub-block-major + §6(a) fold schedule + G2 fold-stripe) + store (A3.2a `noised_store_layout`) + fold chain + jackpot-hash + padding — each a deterministic fn of `MatmulParams` (+ attested `(tile_i,tile_j)` from MED-3) |
| `NOISE_PACKED_PREP` | **0 everywhere except store rows**, where `= polyval(noise_ref(s_a/s_b at the `noised_store_layout` row's `(i,l)`), 129)` — `s_a/s_b` are the C1-pinned public seeds (§4.C.2 / b2) |
| `CV_OR_TWEAK_PREP` | the BLAKE3 CV-routing/tweak schedule (deterministic from the strip-opening + jackpot-hash block layout) |
| `AB_ID_PREP` | the A_ID/B_ID schedule (constant `0` today; M-S1 value-as-key) |
| `STARK_ROW_IDX` | the row index (trivially `r`) |

`block_public` = the verifier-known per-block public inputs: the
attested `(tile_i,tile_j)` (MED-3-derived), `κ`=`JOB_KEY`,
`s_a`/`s_b` (the C1 `COMMITMENT_HASH` chain — public, derived
from the block header, Pearl §4.3), `HASH_A`/`HASH_B`
(commitment PIs). All already verifier-side inputs.

## 3. Architecture

```rust
// composite_proof.rs (new)
pub fn canonical_program(
    params: &ZkParams,
    bp: &BlockPublic,        // tile_i, tile_j, kappa, s_a, s_b, hash_a, hash_b
    trace_len: usize,        // = Layer0RowBudget::required_trace_len (params-pure, P-B)
) -> RowMajorMatrix<Val>     // trace_len × PROGRAM_COLS.len(); NO witness
```

- A params-pure **row schedule** assigns each row index a
  `RowClass` (StripOpenA, StripOpenB, KeyPin, Sweep{…},
  Store{i,l,side}, Fold{…}, JackpotHash{…}, Pad) — the *same*
  layout the bridge builds (A1 `tile_chunk_range`, the §6(b)
  sweep schedule, A3.2a `noised_store_layout`, the fold/jackpot
  offsets). This schedule is *the* single source of truth for
  "where each row class sits" — the bridge's trace generator and
  `canonical_program` both consume it (eliminating the
  shared-bug class: there is one schedule, not two
  constructions).
- `composite_preprocess::fill_preprocessed_row(row_idx,
  &RowDescriptor, …)` already maps a `RowDescriptor` →
  PROGRAM_COL values; `canonical_program` builds the
  `RowDescriptor` for each row **from the class + `block_public`
  + `noise_ref`**, witness-free, and runs the existing
  `build_preprocessed_columns`.
- The verify path commits to `canonical_program(params, bp)`'s
  preprocessed trace as the VK (`ProverData::from_airs_and_degrees`
  is already witness-free — it only needs the program + height);
  **verify checks the proof against this**, not against any
  prover-supplied program.

## 4. Soundness statement

Verifier computes `P = canonical_program(params, block_public)`
(public-only). VK = commitment to `P`'s preprocessed trace.
`CompositeFullAirPinned` forces every PROGRAM_COL main cell ==
`P`. Nothing prover-influenced enters `P`. Therefore **every
pinned schedule** — selectors, MAT_ID, fold/G2 schedule,
AB_ID, row-idx, **and store `NOISE_PACKED_PREP`** — is
verifier-fixed from public data. §4.C.2 closes via §1.3's
chain. (CRIT-1 is *also* strengthened for the other four
PROGRAM_COLS: the latent "canonical = extract-of-reference"
fragility is removed system-wide.)

## 5. The migration safety net (KAT-first — R1 discipline)

**Gate before flipping the verify path:** for every row class,
over representative geometries (Llama-3.1-8B GATE_UP/DOWN +
TEST_SMALL + a rectangular shape),

```
canonical_program(params, block_public)  ==  extract_program(honest_trace)
```

bit-for-bit (all five PROGRAM_COLS, every row). If they ever
diverge, a bug in *either* the params function or the reference
builder is surfaced **before** trust — the P-B.2.0 win
(KAT-first disproved an over-cautious premise; here it catches
shared-construction bugs). Stage **per row class**: land
`canonical_program` for one class at a time, KAT == extract for
that class, only then the next; flip the verify path last, once
all classes match.

## 6. Decisions (surfaced; recommendations)

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **D-CR1** | Coverage | (a) all 5 PROGRAM_COLS reconstructed params-pure · (b) only store `NOISE_PACKED_PREP` params-pure, leave the other 4 on extract-of-reference | **(a)** — the isolation-auditable defining-artifact property (and the latent-fragility removal) is the *point*; (b) keeps the shared-bug class for selectors/fold/etc. and still needs a row schedule, so (a) is barely more work for the whole win |
| **D-CR2** | Verify-path wiring | (a) verify against `canonical_program(params,bp)` (the actual soundness fix; closes the "bridge passes prover's program to verify" latent weakness) · (b) keep passing the program but make `canonical_program` the authoritative reference the `crit1_*`-style discipline checks | **(a)** — (b) does not remove the latent weakness in the production verify path; (a) is the milestone's purpose |
| **D-CR3** | Migration | per-row-class staged with the §5 `canonical==extract` KAT as the gate, flip verify last · vs one-shot | **staged** (R1) |
| **D-CR4** | §4.C.2 coordination | Phase A ships §4.C.2 on **b1** (now); Phase A-CR subsumes the noise pin into b2 (this milestone) — b1's in-circuit/store wiring (W2/W3) is reused; only *how the verifier obtains canonical `NOISE_PACKED_PREP`* changes (extract-of-reference → params-pure) | confirmed by the §4.C.2 verdict — do **not** double-implement; b1 first, b2 here |

## 7. Staged landing plan + LIVE STATUS (2026-05-18)

Module: `ai_pow_zk::canonical` (`crates/ai-pow-zk/src/canonical.rs`).
`canonical_program(&ZkParams, &BlockPublic, trace_len) ->
RowMajorMatrix<Val>` (12-wide = PROGRAM_COLS); per row:
`schedule_layout` (the ONE boundary source) → `class_of` →
`row_descriptor` → existing `build_preprocessed_columns` packing.
`is_class_canonical(class)` fences which classes are exact +
`==extract`-validated; the §5 KAT asserts only those rows. The
**Phase A-CR is COMPLETE (CR.0–CR.7, 2026-05-18).**

**DONE + validated + committed (each stage: canonical unit + the
cross-crate §5 KAT `canonical_program == extract_program(real
P16(16|r) trace)` on the then-canonical classes, all 12
PROGRAM_COLS, + `cr0`/`cr4a` regression + full `ai-pow-zk --lib`
additive):**

- **CR.0a** (`3671702`) — `blake3_tree::strip_opening_rows`
  (params-pure; mirrors `fold_strip`'s leaf/parent recursion).
- **CR.0b** (`fdde985`) — `row_schedule` + cross-crate KAT vs
  the real P16 trace (KeyPin pins `mh_end=na+nb`, HASH_A/B roots
  pin `na`, FOLD set pins `sweep_rows`+`num_stripes`, every
  `IS_MSG_MAT` producer ∈ StripOpen*).
- **CR.1** (`3c24fe0`) — `canonical_program`+`BlockPublic`
  scaffolding + **Pad** class + the staged §5 cross-crate KAT.
- **CR.2** (`aec1a4e`) — **KeyPin** + the `ScheduleLayout`
  single-source refactor (one struct consumed by `row_schedule`
  AND `row_descriptor`).
- **CR.3** (`395782f`) — **JackpotHash** + the shared
  `blake3_block_descriptor` + `jackpot_tweak_packed` (const
  tweak `{0,0,64,0x1B}`).
- **CR.4a** (`9c8ddc7`) — `strip_blocks` walker
  (mirrors `fold_strip`/`subtree_inside`/`place_leaf_chunk`) +
  per-block leaf/parent/root tweak + `IS_HASH_A/B` finalize
  selector; targeted KAT on non-co-located StripOpen* rows.
- **CR.4b+CR.4c** (`64e75e1`) — co-located leaf round-0
  `IS_MSG_MAT` + the **8 `NOISE_PACKED_PREP` pins** =
  `polyval(noise_ref(s_a/s_b at p=chunk·1024+b·64+g),129)` (the
  §4.C.2/b2 core; `RowDescriptor.noise_packed_hi:[i64;7]` added
  additively). §5 KAT now covers all StripOpen* incl the
  co-located noise pins, real ctx s_a/s_b.
- **CR.5** (`9beee44`) — **Sweep/Fold**; `row_descriptor` match
  exhaustive, `is_class_canonical ≡ true` (every class). §5 KAT
  asserts `== extract_program(real P16)` on **EVERY row × all
  12 PROGRAM_COLS**.
- **CR.6** (`2a9a18d`) — **flipped the verify path** (the
  soundness linchpin). 16|r path: bridge verifies vs the
  verifier-rebuilt `canonical_program`, never the prover's.
  Coloc-gated (the full regression caught the unconditional-flip
  break on 4 non-16|r tests ⇒ R1: no half-landed invasive
  change; non-16|r retains prior extract-of-reference).
  Adversarial `cr6_*` (non-canonical PROGRAM_COL rejected).
  Gated: ai-pow-zk --lib 358/0/22 (crit1_*/routea_* incl);
  ai-pow zk --lib 93/0/1 + integration bins 0 FAILED;
  debug-assertions-ON honest 16|r roundtrip + cr1 §5 per-row
  clean (negative tests panic-detect at prove-time —
  pre-existing `*_rejects_*` convention).
- **CR.7** — this doc + `ZKP_SECURITY_REPORT`/`GAP_AUDIT`
  CRIT-1 flipped to "first-class params-pure reconstruction";
  §4.C.2 b2 subsumed.

`is_class_canonical ≡ true` (every `RowClass`). The historical
sub-staging plan (CR.4a/b/c, CR.5, CR.6, CR.7) is retained below
as the implementation record.

### CR.4 — StripOpenA/B (the §4.C.2/b2 core; sub-staged)

`place_matrix_strip_opening` → recursive `fold_strip` (row counts
already mirrored by `strip_opening_rows`) → `place_leaf_chunk`
(16 BLAKE3 compressions / 1024-byte chunk) + parent (auth-fold)
BLAKE3 blocks. Each block is 8 rows ⇒ reuse
`blake3_block_descriptor`. The traversal is params-pure: chunk
range from A1 `tile_chunk_range(tile, t, k, {m,n}*k)`, leaf/parent
shape from the same recursion as `strip_opening_rows`.

- **CR.4a** — params-pure traversal emitting, per StripOpen* row,
  its block kind + tweak + finalize-extra:
  - leaf-chunk block `b`∈0..16 of global `chunk_index` (from
    `tile_chunk_range`): tweak `counter_low=chunk_index`,
    `block_len=64`, `flags = F_KEYED_HASH | F_CHUNK_START(b==0)
    | F_CHUNK_END(b==15) | F_ROOT(single-chunk-root)`;
  - parent block: `flags = F_KEYED_HASH | F_PARENT |
    F_ROOT(root-parent)`;
  - root row finalize-extra = `selector_idx` (4 `IS_HASH_A`
    A-side / 5 `IS_HASH_B` B-side); root-parent extras per
    `place_matrix_strip_opening` (`extras` at line ~638).
  Gate **non-co-located first** (`noise_strip=None`, A3.2b
  shape) to validate the pure BLAKE3 schedule before noise.
- **CR.4b** — co-located leaf round-0 rows: add `IS_MSG_MAT`
  (SELECTOR_COLS idx 10) + CONTROL_PREP `msg_pair` + per-row
  `mat_id`, leaf F_CHUNK_* tweak. KAT on the real P16(16|r)
  co-located trace (selectors/CONTROL_PREP only).
- **CR.4c** (hardest) — the 8 `NOISE_PACKED_PREP[0..7]` pins on
  each co-located leaf round-0 row =
  `polyval(noise_ref(bp.s_a/s_b at the leaf block's (i,l)), 129)`
  per sub-slice, via the cx.2-coloc.0-validated
  (chunk,block,sub-slice)→matrix-position→`noise_ref` map
  (`e_value` A row-major / `f_value` B col-major; pad p≥len→0;
  see `zk_bridge.rs` `a_noise_strip`/`b_noise_strip`). This is
  the b2 closure (verifier obtains canonical `NOISE_PACKED_PREP`
  params-pure, not extract-of-reference). KAT: §5 on the real
  P16 co-located trace.

### CR.5 — Sweep/Fold

§6(b)-G1/G2 sweep schedule + FoldChip rows' CONTROL_PREP
(`is_fold`/`fold_slot`/`fold_stripe`), `mat_id`, `ab_id`;
KAT-gated. After this `is_class_canonical` = every class.

### CR.6 — flip the verify path (R1 soundness linchpin)

VK = commitment to `canonical_program(params, bp)`;
`prove_and_verify` verifies against it (not the prover-passed
program). Wire `BlockPublic` from `ctx` (real
κ/s_a/s_b/tile_i/tile_j). Gate: Route-A + full `crit1_*` +
`-C debug-assertions=on` + a NEW adversarial (a trace whose any
PROGRAM_COL — esp. co-located `NOISE_PACKED_PREP` — ≠ the
params-pure canonical is rejected) + `ai-pow --features zk`
all-green. The actual soundness fix (closes the latent
"bridge passes prover's program to verify").

### CR.7 — docs/audit flip

`ZKP_SECURITY_REPORT`/`GAP_AUDIT` CRIT-1: "extract-of-reference
+ adversarial discipline" → "first-class params-pure
reconstruction"; §4.C.2 b2 marked subsumed.

Each stage: commit per validated stage; honest status + precise
residual (R1); `Plonky3-recursion/` untracked. **CR.6 is the
soundness linchpin — full R1 staged discipline (KAT-first,
Route-A + crit1_* + debug-assertions-ON + adversarial before
the flip); never rush it to satisfy a hook.**

## 8. Risks

- **The single most soundness-sensitive change in the
  codebase.** A bug in `canonical_program` that *matches* a
  bug in the reference builder passes the §5 KAT (both share the
  schedule under CR.0 — mitigant: CR.0 makes the schedule the
  *one* source so prover/verifier cannot disagree, but a wrong
  schedule is wrong for both ⇒ also red-team `canonical_program`
  against *hand-computed* expected PROGRAM_COLS for small
  geometries, not only against `extract`).
- LogUp / debug-assertions-OFF hazard (M-S1 lesson): unit-AIR
  green ≠ Route-A green; gate every stage with Route-A +
  `-C debug-assertions=on`.
- Interaction with the deferred G3 / `sx_bound` and the legacy
  path: the schedule must handle the `num_stripes>STRIPE_MAX`
  legacy branch (or assert it out of the Pearl §4.8 envelope —
  P-A already bounds PROD).
- Sequencing: must land **after** Phase A (so §4.C.2-b1 + A3.2a
  exist to subsume/reuse) and **before** Phase B (correctness /
  byte-equivalence) and Phase C (which *requires* this
  primitive) — see roadmap §2/§3.

## 9. Why before Phase B

It is a **soundness-foundation** upgrade that (i) §4.C.2 (Phase
A's A3, shipped on b1) is upgraded *onto*, (ii) Phase C's P-C
succinct/recursive certificate and M-C1 consensus **require**
regardless (no prover trace there), and (iii) removes a latent
CRIT-1 fragility system-wide. Sequencing it before the
correctness/byte-equivalence reconciliation (Phase B) and the
cert/integration (Phase C/D) keeps the PoW-soundness linchpin
first-class *before* layering correctness and external
integration on top — and shares its cost with the
params→program/VK reconstruction Phase C needs anyway.

## 10. Cross-references

- §4.C.2 / b1 (shipped in Phase A): `2026-05-17_SEC_4C2_NOISE_BINDING_DESIGN.md`.
- A1 schedule: `blake3_tree::tile_chunk_range`
  (`2026-05-17_P_B2_STRIP_OPENING_DESIGN.md`).
- A3.2a witness-free store layout:
  `composite_trace::noised_store_layout`.
- Noise spec: `noise_ref` (A3.0; cross-crate KAT == `BlockNoise`).
- CRIT-1 mechanics: `composite_proof.rs`
  (`composite_setup`/`extract_program`/`crit1_*`),
  `composite_preprocess.rs`, `composite_full_air.rs`
  (`CompositeFullAirPinned`, `PROGRAM_COLS`).
- Roadmap: `2026-05-17_PRODUCTION_ROADMAP.md` §2 Phase A-CR, §3 critical
  path; `2026-05-15_HIGH2_2_DESIGN.md` §7.
- Governing rule: `~/.claude/CLAUDE.md` R1.
