# Phase A-CR — first-class params-pure `canonical_program` (CRIT-1 reconstruction hardening; subsumes §4.C.2-b2)

> **Status:** DESIGN (2026-05-17). New roadmap milestone,
> sequenced **after Phase A, before Phase B proper**
> (`PRODUCTION_ROADMAP.md` §2). This is "b2" from the §4.C.2
> decision (`SEC_4C2_NOISE_BINDING_DESIGN.md`) **promoted to
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
> **Cross-refs:** `SEC_4C2_NOISE_BINDING_DESIGN.md` (§4.C.2 /
> b1), `P_B2_STRIP_OPENING_DESIGN.md` (A1 `tile_chunk_range`),
> A3.2a `noised_store_layout`, `ZKP_SECURITY_REPORT.md` CRIT-1,
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

## 7. Staged landing plan

- **CR.0** — extract the single params-pure **row schedule**
  (`RowClass` per row from params + `block_public`), shared by
  the bridge trace generator and `canonical_program`. KAT:
  schedule(params) reproduces the bridge's actual row layout
  (incl. A1 `tile_chunk_range`, A3.2a `noised_store_layout`).
  *No verify-path change.*
- **CR.1..CR.5** — `canonical_program` per row class
  (CONTROL_PREP/CV/AB_ID/ROW_IDX classes, then the §4.C.2 store
  `NOISE_PACKED_PREP` via `noise_ref`), each gated by §5
  `canonical==extract(honest)` for that class + full
  `ai-pow-zk --lib`.
- **CR.6** — flip the verify path: VK = commitment to
  `canonical_program(params, block_public)`; `prove_and_verify`
  verifies against it (not the prover-passed program). Route-A +
  the full `crit1_*` suite + new adversarial: a trace whose any
  PROGRAM_COL (esp. store `NOISE_PACKED_PREP`) ≠ the params-pure
  canonical is rejected; `ai-pow --features zk` all-green;
  debug-assertions-ON.
- **CR.7** — docs/audit flip: `ZKP_SECURITY_REPORT`/`GAP_AUDIT`
  CRIT-1 upgraded from "extract-of-reference + adversarial
  discipline" to "first-class params-pure reconstruction";
  §4.C.2 b2 marked subsumed.

Each stage: commit per validated stage; honest status + precise
residual (R1); `Plonky3-recursion/` untracked.

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

- §4.C.2 / b1 (shipped in Phase A): `SEC_4C2_NOISE_BINDING_DESIGN.md`.
- A1 schedule: `blake3_tree::tile_chunk_range`
  (`P_B2_STRIP_OPENING_DESIGN.md`).
- A3.2a witness-free store layout:
  `composite_trace::noised_store_layout`.
- Noise spec: `noise_ref` (A3.0; cross-crate KAT == `BlockNoise`).
- CRIT-1 mechanics: `composite_proof.rs`
  (`composite_setup`/`extract_program`/`crit1_*`),
  `composite_preprocess.rs`, `composite_full_air.rs`
  (`CompositeFullAirPinned`, `PROGRAM_COLS`).
- Roadmap: `PRODUCTION_ROADMAP.md` §2 Phase A-CR, §3 critical
  path; `HIGH2_2_DESIGN.md` §7.
- Governing rule: `~/.claude/CLAUDE.md` R1.
