# C3 / #124 — DT-1 design: D=2 Tip5 outer-cert recompose-coeff producer balancing

> **Status:** DESIGN SUPERSEDED — root cause **empirically
> FALSIFIED** by the 2026-05-19 implementation attempt (see **§7**).
> The DT-1 §1/§2 mechanics survey misidentifies the orphan as a
> `recompose/coeff` producer-gating issue; the genuine orphan is a
> **Tip5-perm output↔input `WitnessChecks` D=2 mismatch** (C2.4
> R-a-tail), inside fenced-linchpin territory. **Path-(i)/1d as
> written does NOT fix C3/#124 — do not re-attempt it.** C3.0
> premise (recompose==x binding) is moot for this orphan (it is not
> a recompose coeff). C3.1/C3.2 **NOT done, NOT faked** — nothing
> landed; the worktree is byte-identical to baseline and the full
> gate is green. R1 residual = the corrected root cause in §7.
> Original DESIGN text (§0–§6) retained verbatim below for the
> audit trail; it is now **historical / incorrect on the cause**.
>
> _(orig.)_ DESIGN (DT-1, #129; 2026-05-19). The C3/M-S5 enabler.
> Governed by R1: this touches fenced-linchpin-*adjacent* shared
> wiring (the recompose-coeff producer / `hint_output_wids`
> semantics), so it is **design + soundness-argument first**, then
> staged + KAT-first + per-stage full re-validation. **No
> fenced-linchpin code edited by this doc.** Inputs: the DT-1
> mechanics survey (verbatim file:line below).

## 0. TL;DR

The C2-validated Tip5 Layer-0 verifier circuit verifies a real
proof **in-circuit** (`runner().run()`, accept + tamper-reject,
full 120-bit sweep — done, C2 #123 complete). The **outer
recursive STARK certificate** of that circuit
(`prove_all_tables`/`verify_all_tables` = C3/#124) fails the D=2
cross-table `WitnessChecks` LogUp global-sum by an **orphaned net
±1 over correctly-3-wide tuples** `[idx, value, 0]`. The
WitnessChecks D-padding itself is already fixed + D=1-byte-
identical-re-validated (commit `632cb8c`). The remaining,
*narrower*, precisely-located cause:

- A base-coeff gets a **producer** multiplicity on `WitnessChecks`
  **iff its witness id ∈ `hint_output_wids`**
  (`circuit-prover/src/batch_stark_prover/recompose.rs:345`):
  ```rust
  let n_coeff_reads = if prep.hint_output_wids.contains(&(coeff_wid as u32)) {
      prep.ext_reads.get(coeff_wid).copied().unwrap_or(0)
  } else { 0 };
  ```
- `hint_output_wids` = outputs of **`Op::Hint`** ops only, minus
  Const/Public (`circuit/src/circuit.rs:272-284`).
- The **validated** poseidon2 D=1-in-D=5 quintic outer cert
  balances because its perm-input coeffs pass through
  `recompose_base_coeffs_to_ext_with_coeff_lookups`
  (`circuit_builder.rs:~1495`) which **emits `Op::Hint`** per
  coeff ⇒ they land in `hint_output_wids` ⇒ producer = consumer.
- `verify_p3_uni_proof_circuit` feeds the Tip5 perm inputs from
  the in-circuit challenger; their base coeffs are allocated by
  `decompose_ext_to_base_coeffs` → `push_unconstrained_op
  (ExtDecompositionHint)` — a **computed/unconstrained-hint**
  witness, **not an `Op::Hint` output** ⇒ ∉ `hint_output_wids`
  ⇒ producer **+0**, while the (correctly D-padded) Tip5
  WitnessChecks consumer sends **−1** ⇒ orphan **+1**.

**Recommended fix (path i / sub-option 1d):** register the
`ExtDecompositionHint`-allocated coeff witnesses in a
**`computed_decompose_coeffs`** set alongside `hint_output_wids`,
and OR it into the `recompose.rs:345` predicate. Localized to the
decompose/recompose+preprocessor plumbing; **does not touch**
`verify_p3_uni_proof_circuit`, the validated `Tip5PermLookupAir`/
`tip5_l` constraints, or any AIR. Sound by the *same argument*
as the validated quintic path (§2).

## 1. Exact mechanics (verbatim file:line)

| Component | Loc | Role |
|---|---|---|
| `hint_output_wids` build | `circuit/src/circuit.rs:272-284` | set = `Op::Hint` outputs, minus Const/Public |
| coeff producer gate | `circuit-prover/.../recompose.rs:345-352` | producer mult = `ext_reads[wid]` iff `wid ∈ hint_output_wids`, else **0** |
| coeff consumer send | `circuit-prover/src/air/recompose_air.rs:~190` | `push_interaction("WitnessChecks",[idx,val,0..],mult,1)` per coeff read (always) |
| quintic working path | `fibonacci_batch_stark_prover_quintic.rs:136-143` + `circuit_builder.rs:~1495` | perm-input coeffs wrapped via `recompose_base_coeffs_to_ext_with_coeff_lookups` ⇒ `Op::Hint` ⇒ in `hint_output_wids` ⇒ balances |
| Tip5 verifier feed | `recursion/src/challenger/circuit.rs:236-250` `duplexing_base_tip5` → `circuit_builder.rs:1698` `add_tip5_perm_for_challenger_base` | Tip5 inputs = challenger state; their coeffs come from `decompose_ext_to_base_coeffs` |
| non-Hint alloc | `circuit/src/builder/circuit_builder.rs:~1512` | `push_unconstrained_op(ExtDecompositionHint)` ⇒ computed witness, ∉ `hint_output_wids` |
| failing gate | LogUp `verify_global_sum` (WitnessChecks) | net **+1** over 3-wide `[idx,val,0]` |

## 2. Soundness analysis (the crux — rigorous, not hand-waved)

**What actually binds a decomposed coeff `c_i` (independent of
the `WitnessChecks` multiset).** A decomposed coeff is pinned by
two constraints that exist **regardless** of producer/consumer
bookkeeping:

1. **decompose↔recompose linkage** — `decompose_ext_to_base_
   coeffs` allocates `[c_0..c_{D-1}]` then the circuit enforces
   `recompose_base_coeffs_to_ext([c_0..c_{D-1}]) == x` (a
   polynomial AIR constraint via the recompose table; the
   quintic path's `self.connect(x, reconstructed)`,
   `circuit_builder.rs:~1530`). So the `c_i` are *uniquely
   determined* by `x`.
2. **`x` is FRI/Merkle-bound** — in `verify_p3_uni_proof_circuit`
   `x` is a challenger/opened/transcript value already bound to
   the proof commitments by the FRI low-degree + MMCS-root +
   transcript-recompute constraints (the C2/L5-validated
   binding).

⇒ **The `WitnessChecks` producer/consumer balance is pure
multiset bookkeeping, not the soundness binding.** Whether the
recompose table emits a +producer for `c_i` does **not** change
what value `c_i` may take (that is fixed by (1)+(2)); it only
makes the global LogUp sum close. Therefore making the
`ExtDecompositionHint` coeffs producer-emitting is
**sound-preserving by the identical argument that validates the
quintic path** (where the same `c_i` are `Op::Hint`-wrapped and
producer-emitting, and the same (1)+(2) bind them). This is a
**mechanical mirror of a validated mechanism, not a soundness
fork.**

**Two soundness obligations the implementation must discharge
(and the gate that empirically discharges them):**

- **O1 — no double-production.** If a witness is *both*
  producer-emitted by the recompose table *and* by Const/Public
  (or another producer), the multiset over-balances (silent
  imbalance ⇒ a forged proof could be made to balance). The
  existing `hint_output_wids` build *already excludes
  Const/Public* (`circuit.rs:283`); the new
  `computed_decompose_coeffs` set **must** be constructed with
  the same exclusion and be **disjoint from `hint_output_wids`**
  (an `Op::Hint` output cannot also be an `ExtDecompositionHint`
  output — assert this in construction). Empirically discharged
  by: the **entire poseidon1/2 + quintic + recompose + C2.0–C2.4
  D=1 regression staying byte-identical-green** (any new
  double-production there orphans those gates).
- **O2 — correct multiplicity.** Producer mult must equal Σ
  consumer reads. It reuses the *same* already-audited
  `ext_reads[wid]` counter the validated paths use; correctness
  is empirically discharged by the **D=2 outer-cert
  `WitnessChecks` global-sum balancing to exactly zero AND a
  tampered proof still rejecting** (a wrong count would either
  orphan honest or fail to catch tamper).

**Escalation criterion (genuine soundness fork — hard stop).**
The §2 argument *depends on premise (1)*: every Tip5 perm-input
coeff that this fix makes producer-emitting **is** pinned by a
decompose↔recompose constraint tying it to a FRI/Merkle-bound
`x`. **Implementation MUST first verify this premise empirically**
(trace each `computed_decompose_coeffs` witness to its
`recompose ... == x` connect + `x`'s FRI/Merkle binding). If any
such coeff is found **not** so bound (producer-emitting but
otherwise free), making it a producer is a **forgery hole** — the
fix must instead *add the missing binding constraint*, and this
becomes a genuine soundness-decision to surface, not a mechanical
mirror. Do not proceed past Stage 1 until the premise is
verified.

## 3. Decision: path (i)/1d (recommended) vs path (ii) (fallback)

- **Path (i), sub-option 1d — RECOMMENDED.** New
  `PreprocessedColumns::computed_decompose_coeffs: HashSet<u32>`,
  populated in `circuit.rs` by scanning
  `Op::Unconstrained{ kind: ExtDecompositionHint, outputs, .. }`
  (same flatten + Const/Public-exclude + assert-disjoint-from-
  `hint_output_wids` as the existing `hint_output_wids` build).
  `recompose.rs:345` predicate becomes
  `hint_output_wids.contains(w) || computed_decompose_coeffs
  .contains(w)`. **Blast radius:** only paths that decompose ext
  values via `ExtDecompositionHint` and read the coeffs through
  recompose — i.e. exactly the Tip5-verifier (and any future
  D≥2-in-D=1-perm) path; poseidon1/2/quintic are unaffected
  (their coeffs are `Op::Hint`, already covered by the first
  disjunct; the second disjunct is empty for them). **No fenced
  linchpin / no `verify_p3_uni_proof_circuit` / no AIR edit.**
- **Path (ii) — FALLBACK only.** Drop the `hint_output_wids`
  gate entirely (`n_coeff_reads = ext_reads[wid]` always).
  Simplest diff but **maximal blast radius** (every poseidon1/2
  D=1+D≥2 + recompose self-test) and a real **O1 double-
  production risk** (a coeff that is *both* Const/Public and
  recompose-read would now double-produce — the current gate
  prevents this implicitly). Requires full re-validation of all
  poseidon paths and an explicit double-production audit. Use
  only if (i)/1d proves infeasible.

**Recommendation: path (i)/1d.** Minimal, localized, sound by the
same argument as the validated quintic path, no fenced-linchpin
edit, and the regression gate cleanly discharges O1/O2.

## 4. Staged plan (R1 — commit per validated stage; D=1 byte-identical invariant)

- **C3.0 — premise verification (the escalation gate).** Trace
  every Tip5-verifier `ExtDecompositionHint` coeff to its
  `recompose==x` connect and `x`'s FRI/Merkle binding; document.
  If any unbound ⇒ STOP, surface the soundness fork (§2
  escalation). KAT artifact: a written enumeration + a debug
  assertion in the circuit builder.
- **C3.1 — `computed_decompose_coeffs` plumbing.** Add the field
  + population (`circuit.rs`) + the `recompose.rs:345` OR-clause
  + the disjointness assertion (O1). **Gate:** the ENTIRE
  C2.0–C2.4 D=1 suite byte-identical-green (p3-tip5-circuit-air
  14/14, test_tip5_lookups 2/2, test_tip5_layer0_recursion 7/7,
  challenger_transcript 46, p3_recursion 32) **and** poseidon1/2
  + `fibonacci_batch_stark_prover_quintic` + recompose +
  full-workspace regression green (proves no poseidon/quintic
  perturbation: for them the new set is empty/disjoint).
- **C3.2 — D=2 outer recursive certificate.** Re-add the
  `prove_all_tables`/`verify_all_tables` accept tests (≥PROD,
  ideally all 5 sweep profiles) + tamper-reject, to
  `test_tip5_layer0_recursion.rs`. **Gate:** WitnessChecks LogUp
  global-sum = 0 (no orphan), accept-valid, **reject-tampered**
  (O2), full workspace still green. The ≤65 KB certificate-size
  check (the M-S5 target) is asserted here.
- **C3.3 — honest status + docs/memory + commit.** Update
  `C2_TIP5_CIRCUIT_AIR_DESIGN.md`/this doc/memory; close #124.
  R-b (actual M10.1c composite AIR) remains M12/#127.

Each stage: independent re-reproduction (not trusting any agent),
soundness-critical diffs reviewed by hand, commit only on a
green per-stage gate. A genuine failed validation or the §2
escalation ⇒ R1 residual, not a rushed landing.

## 5. Acceptance criterion (definition of done for C3/#124)

`prove_all_tables`+`verify_all_tables` of the Tip5 Layer-0
verifier circuit: WitnessChecks balances (zero orphan) and
verification **accepts a valid** real-Layer-0-config proof and
**rejects a tampered** one, across the 120-bit sweep; the
recursive certificate meets the **≤65 KB** M-S5 size target; the
full C2.0–C2.4 D=1 gate is byte-identical-green and the entire
poseidon/recursion/quintic regression is green; the §2 premise
is verified (no producer-emitting coeff is otherwise unbound).

## 6. Cross-references

`C2_TIP5_CIRCUIT_AIR_DESIGN.md` §2c.C2.4 (R-a landed, this is the
remaining wall); commit `632cb8c` (WITNESS_EXT_D-aware CTL, D=1
byte-identical); tasks #124 (C3, blocked by #129=this),
#127 (D2/M12 = R-b composite-AIR bridge); memory
`c2-tip5-circuit-air`. Validated reference path:
`fibonacci_batch_stark_prover_quintic.rs` (poseidon2 D=1-in-D=5
outer cert — the producer-balance mechanism this mirrors).

## 7. C3.0–C3.2 implementation attempt (2026-05-19) — R1 residual: DT-1 root-cause empirically FALSIFIED; NOT landed; NO fake completion

> **Status: ATTEMPTED + DRIVEN this session (R1.1-compliant), hit a
> concrete correctness wall. Path-(i)/1d as specified does NOT fix
> the orphan; the genuine orphan is NOT a recompose-coeff. Nothing
> landed in `Plonky3-recursion` (worktree byte-identical to the
> C3-design baseline); the full workspace gate remains green. This
> is the R1 validated-subset (= the *clean baseline*, since no
> partial soundness change is safe to land) + this precise
> residual. The DT-1 §1/§2 mechanics survey is empirically wrong on
> the orphan's source — do not re-attempt path-(i)/1d as written.**

### 7.1 What was done (independent re-reproduction; R1 discipline)

- **Reproduced the exact documented failure** on a real PROD Tip5
  Layer-0 verifier circuit via a new
  `prove_all_tables`/`verify_all_tables` outer-cert harness
  (`config::goldilocks_tip5`, D=2 challenge field, Tip5 NPO D=1,
  recompose `split_coeff_tables=true`, the validated quintic
  pattern): `WitnessChecks` global-sum orphan, tuple
  `["22936","10485455180627170985","0"]` net **+1**, ONE location
  `instance 3 / lookup 241 / row 292`. `22936 = wid 11468 × D(2)`.
- **Implemented path-(i)/1d** exactly per §3: new
  `PreprocessedColumns::computed_decompose_coeffs`, populated in
  `circuit.rs`, OR-ed into the `recompose.rs:345` predicate, with
  the O1 Const/Public exclusion.
- **Empirically traced** every `recompose/coeff` coeff witness +
  the specific orphan wids to their defining/reading ops (the §2
  escalation-gate premise check, done by independent re-derivation,
  not trusting the doc).

### 7.2 The concrete wall (empirically established, decisive)

The DT-1 §1/§2 premise is **"the orphan is a `recompose/coeff`
base-coeff whose producer is gated off because it is an
`ExtDecompositionHint` (`push_unconstrained`) output ∉
`hint_output_wids`."** Direct op-level tracing of the **actual**
orphaned witness (`wid 11468`, idx `22936`) shows this is **false**:

- `wid 11468`: `ext_reads = 1`, `in hint_output_wids = false`, **not
  a `recompose/coeff` input at all** (`computed_decompose_coeffs`
  membership = false even after the fix). Its **creator** is
  `op[10576] = NPO(tip5_perm/goldilocks_w16_r7) OUTPUT`; its **sole
  reader** is `op[10577] = NPO(tip5_perm/goldilocks_w16_r7) INPUT`,
  `tip5_dup_flag = false`. ⇒ It is a **Tip5-perm-output →
  Tip5-perm-input chain** witness (the recursion challenger/MMCS
  duplex chains Tip5 perms), **not** a decompose/recompose coeff.
  The orphan having a *single* location = a lone Tip5 output-limb
  **producer** push (`+ext_reads = +1`) whose matching Tip5
  input-limb **consumer** push never cancels it (a producer/consumer
  tuple-shape or idx mismatch at D=2 between the two Tip5 perm
  instances) — squarely the **C2.4 R-a-tail** Tip5/D=2
  `WitnessChecks` accounting residual.
- The literal path-(i)/1d set (`ExtDecompositionHint` outputs) is
  also too narrow even *for* recompose coeffs: of 974 PROD
  `recompose/coeff` coeff slots only **14** are `ExtDecompositionHint`
  outputs; **960** are `select(b,tc,sc)` results the optimizer fuses
  into computed `Op::Alu(MulAdd)` outputs (the Merkle-path/challenger
  select-provenance branch of `decompose_ext_to_base_coeffs`). A
  faithful generalization (register the recompose-coeff NPO's own
  input wids) **does** cover all 974 — but those Alu-out coeffs are
  **already produced by the ALU table**, so adding a recompose
  producer **double-produces** them (verified: it converts the
  original lone Tip5 orphan into a *new* `wid 11429` Alu-vs-recompose
  double-production orphan — strictly **worse**, which R1 forbids
  landing).

**Conclusion (no fake):** the genuine C3/#124 orphan is the
**Tip5-perm output↔input `WitnessChecks` producer/consumer
mismatch at D=2** inside the fenced Tip5 circuit-AIR `eval`
(`tip5-circuit-air/src/air_circuit.rs`) and/or the
`verify_p3_uni_proof_circuit` Tip5-perm-chain wiring and/or the
Tip5 prover preprocessor idx accounting (`circuit-prover/.../
tip5.rs`, whose `idx_scale = 1` is *already* the C2.4-R-a
deliberate D≥2 fix and is byte-identical-validated — re-touching it
regresses C2.4 R-a). **All three are inside the task's hard-ruled
no-edit set** (fenced linchpin / `verify_p3_uni_proof_circuit` /
AIR `eval`). The DT-1 recommended fix (path-(i)/1d) targets a
mechanism (`recompose-coeff` producer gating) that is **not** the
cause and **cannot** close this orphan. This is **not** the §2
recompose-coeff escalation case (no producer-emitting coeff is
unbound) — it is a **misdiagnosis in the DT-1 mechanics survey
itself**.

### 7.3 Disposition (R1 / R1.1)

- **Landed: nothing.** The `Plonky3-recursion` worktree is
  **byte-identical** to the C3-design baseline (`circuit.rs`,
  `recompose.rs`, `executor.rs`, `circuit_builder.rs`, the test
  file — all reverted). No half-landed invasive soundness change.
- **Re-validated green (post-revert, independent re-run):** full
  `cargo test --workspace` — `test_tip5_layer0_recursion` **7/7**
  (original D=1, byte-identical), `p3-tip5-circuit-air` **14/14**,
  `test_tip5_lookups` **2/2**, `challenger_transcript` **46**,
  `p3_recursion` **32**, `p3_circuit` **358**, `p3_circuit_prover`
  **40**, `fibonacci_batch_stark_prover_quintic` **1/1**, recompose
  + poseidon1/2 unperturbed; **zero failures**.
- **Precise residual (exact remaining work, why, where):** C3/#124
  is blocked on the **C2.4 R-a-tail**: the Tip5-perm output-limb
  *producer* and input-limb *consumer* `WitnessChecks` tuples must
  cancel at D=2 for a perm-output-feeds-perm-input chain. Closing
  it requires a soundness-critical edit to **fenced-linchpin**
  territory (the Tip5 circuit-AIR `eval` interaction emission, or
  the `verify_p3_uni_proof_circuit` Tip5 challenger/MMCS perm-chain
  decompose wiring, or the Tip5 prover preprocessor's D-scaling),
  each of which the current task hard-rules forbid and each of
  which demands its own design + soundness argument + KAT-first +
  full re-validation (it is **not** a localized bookkeeping mirror
  of the validated quintic path — the quintic path has no
  perm-output→perm-input D=2 chain). **DT-1 must be re-opened**
  with the corrected root cause (Tip5 perm-chain D=2 CTL, *not*
  recompose-coeff `hint_output_wids`) before any C3 landing.
- **R-b unchanged:** ai-pow-zk's actual M10.1c composite
  `RecursiveAir` (vs the representative `FibonacciAir` under the
  exact Layer-0 config) remains M12/#127, explicitly out of scope.

---

## 8. DT-2 (corrected, source-verified) — root cause + gate-arbitrated fix

**Root cause (source-verified).** The `WitnessChecks` bus keys a
witness by `idx`. Canonical namespace is **D-scaled**:
`WitnessId::base_field_index::<F,D> = wid.0 * D`
(`circuit/src/types.rs:16`) via
`register_non_primitive_output_index` (`circuit/src/
circuit.rs:105`); every table (poseidon1/2, recompose, the Tip5
*verifier/committed* prep through `air_with_committed
_preprocessed`) is on it. But the Tip5 **prover** instance
(`circuit-prover/.../tip5.rs::batch_instance_base` line 227)
builds its producer AIR from
`build_tip5_circuit_preprocessed(&t.operations, height,
idx_scale=1)`, and `t.operations` rows carry **UNSCALED**
`input/output_indices = wid.0`
(`circuit/src/ops/tip5_perm/executor.rs:272,283`). ⇒ prover
labels Tip5 perm in/out witnesses `wid.0`; the rest of the bus
labels them `wid.0·D`. **D=1: coincide** (why C2.4-R-a's D=1
gate is green). **D≥2: split** ⇒ a Tip5 duplex-chain witness
(perm-A out → perm-B in; the recursion challenger/MMCS duplexes
Tip5) orphans at `wid·D` — exactly the observed
`["22936",·,"0"]`, `22936 = wid 11468 × D(2)`.

**Candidate fix (localized, soundness-faithful).**
`tip5.rs:227` prover call `…, height, 1)` →
`…, height, witness_ctl_scale)` (the `batch_instance_base`
param, = circuit ext degree D). Rows are unscaled ⇒ this stores
`wid.0·D`, aligning the prover producer with the canonical
D-scaled/verifier namespace. **D=1 byte-identical**
(`witness_ctl_scale==1` ⇒ literally today's value). The line-491
*verifier* call stays `1` (its rows are *already* D-scaled from
`resolved`; re-scaling there would double-scale — the C2.4-R-a
author's comment is correct *for line 491* and **mis-applied to
line 227**, whose rows are unscaled per `executor.rs:272,283`).

**Soundness.** Changes only the prover's *bus label* (`idx`) to
match the single canonical D-scaled namespace the verifier and
every other table already use; **no constraint / multiplicity /
tuple value / binding changes**. `WitnessChecks` global-sum is a
multiset identity over `(idx,value)` requiring one consistent
injective labeling — the Tip5 prover producer is currently on a
*different* namespace at D≥2 (so the check is split there and
would not reliably catch tamper); aligning it *restores* the
bus's soundness function at D≥2. The contested "double-scale /
regress C2.4-R-a" point is **decided by the exhaustive gate, not
argument**: prover rows are unscaled (source-verified) so ×D is
the *first* scaling, not a double.

**Decisive gate (the arbiter; R1 staged, KAT-first).**
1. **D=1 byte-identical**: full C2.0–C2.4 D=1 suite green
   unchanged (`p3-tip5-circuit-air` 14, `test_tip5_lookups` 2,
   `test_tip5_layer0_recursion` orig 7, `challenger_transcript`
   46, `p3_recursion` 32) — verified, not assumed.
2. **No regression**: poseidon1/2 10/10, `fibonacci_batch
   _stark_prover_quintic`, recompose, full
   `cargo test --workspace` green (this empirically settles the
   double-scale question — any double-scale orphans these).
3. **C3 done**: D=2 Tip5 Layer-0 outer cert
   `prove_all_tables`/`verify_all_tables` balances + accepts a
   valid real proof + **rejects tampered**, 120-bit sweep,
   recursive cert **≤ 65 KB** (M-S5).

**Escalation (hard stop, R1).** If (1) not byte-identical, or
(2) poseidon/quintic regresses (the author's double-scale
warning materializing on the prover path), or (3) the D=2 orphan
persists/moves: candidate falsified — land nothing, record the
precise empirical delta, escalate the prover-commit vs
verifier-reconstruct preprocessed-contract fork. No weakening,
no fake.

## 9. DT-2 empirical result (2026-05-19) — NECESSARY-BUT-NOT-SUFFICIENT; candidate FALSIFIED at gate (3); nothing landed

Drove the candidate exactly per §8 in an isolated worktree
(Plonky3-recursion @ 6de5cba), full gate run, line-by-line
review + gate re-run before recording. Outcome:

- **Gate (1) D=1 byte-identical — PASS.** `witness_ctl_scale==1`
  at D=1: `p3-tip5-circuit-air` 14/14, `test_tip5_lookups` 2/2,
  `test_tip5_layer0_recursion` orig 7/7 (5 accept + 2 tamper),
  `challenger_transcript` 46/46, `p3_recursion` 32/32 — green,
  unchanged.
- **Gate (2) no regression — PASS.** poseidon1 10/10,
  poseidon2 10/10, `fibonacci_batch_stark_prover_quintic` 1/1,
  recompose proptests, full `cargo test --release --workspace`
  0 failed. **This empirically SETTLES the contested question:
  the C2.4-R-a author's "double-scale" warning does NOT apply to
  the prover line-227 call — its rows are genuinely unscaled
  (`executor.rs:272,283`), so ×D is the first scaling.** DT-2's
  source-verified root cause + namespace diagnosis is *correct*
  and the prover-label fix is *necessary*.
- **Gate (3) C3 D=2 outer cert — FAILS ⇒ candidate FALSIFIED.**
  Built the exact `BuiltLayer0Circuit` (same circuit/proof as
  the orig 7), D=2 `prove_all_tables` SUCCEEDS (ext_degree==2,
  cert serialized) but `verify_all_tables` REJECTS:
  `LookupError(GlobalCumulativeMismatch(None): WitnessChecks)`,
  **all 5 sweep profiles** (PROD/LB2/LB4/LB5/LB6). prove/verify
  genuinely ran the real D=2 batch-STARK incl. the WitnessChecks
  global-sum (it is that check that rejected — not bypassed);
  the PROD + orig tamper tests still correctly reject; ≤65 KB
  assertion never relaxed (cert rejected ⇒ size moot, not
  faked).
- **Precise empirical delta** (`with_debug_lookups()`):
  `tuple ["22936","10485455180627170985","0"]  net +1`,
  `instance 3 (= Tip5 NPO table), lookup 241, row 292`.
  `22936 = 11468 × D(2)` — the orphan is now **at the canonical
  D-scaled address** `wid·D` (the prover-label fix worked: the
  producer moved onto the correct namespace). A **net +1 surplus
  persists at `idx = wid·D`** ⇒ the residual defect is **not
  pure prover labeling** but a **producer/consumer MULTIPLICITY
  (count) mismatch** at D=2 for the Tip5 duplex-chain witness.

**Disposition (R1).** Genuine in-flight attempt → concrete
correctness wall mid-validated-stage. Validated subset already
landed (C2.0–C2.4-core + R-a, D=1, byte-identical, fenced
linchpin intact). Candidate reverted to byte-identical baseline
(`git diff --stat 27b1e9d -- Plonky3-recursion/` empty; orig 7
re-run 7/7 post-revert). **Nothing landed, no weakening, no
fake.** DT-2's diagnosis stands as a *partial advance*: the
prover-label namespace split is real and the fix is necessary;
it is **not sufficient** alone.

### 9.1 Precise actionable residual — DT-3 (the closing fix; scoped, NOT yet attempted)

The remaining net +1 at `idx = wid·D` (Tip5 NPO, instance 3) is
a **multiplicity** discrepancy, not a label namespace. The C3
certificate needs the DT-2 prover-label fix **plus** a count
reconciliation between:

1. the **recompose-coeff producer** `hint_output_wids` /
   `RecomposePreprocessor` emission multiplicity for the
   Tip5 duplex-chain witness at D=2, and
2. the **Tip5-input decompose consumer** wiring inside
   `verify_p3_uni_proof_circuit` (the fenced recursion
   forgery-binding linchpin) at D=2.

**DT-3 must (R1 KAT-first, before ANY invasive fenced edit):**
source-verify, at D=2, the exact per-`(idx,value)` send/receive
multiplicity ledger for the Tip5 perm-A-out → perm-B-in duplex
witness across {prover Tip5 producer (post-DT-2-label),
recompose-coeff producer, `verify_p3_uni_proof_circuit`
decompose consumer}; identify which side is off-by-one at D≥2
(extra produce vs. short consume); design the minimal scoped
correction with a D=1-byte-identical + poseidon/quintic-no-
regress + D=2-balances gate **before** editing the shared
`RecomposePreprocessor`/`hint_output_wids` or the fenced
`verify_p3_uni_proof_circuit` (both are shared with the
validated poseidon1/2/quintic paths ⇒ full re-validation
mandatory; this is invasive soundness-linchpin work and is
M12/#127-adjacent, NOT a one-liner). DT-2 (§8) is *spent* — it
was the prover-label hypothesis; DT-3 needs its own
source-verified multiplicity hypothesis.

## 10. DT-3 — source-verified multiplicity ledger + fix (2026-05-19; design-first, then driven)

**Bus sign convention (source-pinned):** `p3 lookup/src/
builder.rs:30,66` — `count>0` = SEND (produce), `count<0` =
RECEIVE (consume); per-witness balance = creator SENDs
`+n_reads` once, each reader RECEIVEs `-1`, net 0.

**Exact ledger for wid 11468 @ D=2, tuple `[22936,·,0]`** (DT-2
prover-label fix assumed in effect ⇒ Tip5 producer at
`idx=wid·D=22936`; Tip5 NPO = instance 3):

| # | site | role | ±mult | gate | D-dep? |
|---|------|------|-------|------|--------|
| 1 | `tip5-circuit-air air_circuit.rs:350,357` (mult `circuit-prover .../tip5.rs:429-441`) | Tip5 perm-A rate-OUT (creator, not dup — defined first `circuit.rs:466-486`) | **+`ext_reads`(=1)** | `out_ctl·kind` | label D-scaled |
| 2 | `air_circuit.rs:330,337` | Tip5 perm-B IN (consumer) | **−1** | `−(in_ctl·kind)` | label D-scaled |
| 3 | `recompose_air.rs:190` (mult `recompose.rs:345-350`) | `recompose/coeff` SEND | **+`ext_reads`(=1)** | `if hint_output_wids∋wid {ext_reads} else {0}` | **YES** |

Rows 1+2 (the Tip5 table's own creator/consumer pair) **net
0**. Row 3 is a **surplus duplicate creator** ⇒ **net +1**,
matching the observed delta.

**Root cause (source-confirmed).** The decompose coeff is an
`Op::Hint` output (`circuit_builder.rs:1511-1522` →
`npo.rs:67-68`) ⇒ ∈ `hint_output_wids` (`circuit.rs:272-284`,
built from `Op::Hint` outputs). `set_recompose_coeff_ctl_for
_decompose_links(true)` (the test + `fibonacci_batch_stark
_prover_quintic.rs:143`) routes the duplex EF value through
`decompose_ext_to_base_coeffs` → `recompose_base_coeffs_to_ext
_with_coeff_lookups` (`circuit_builder.rs:1525-1530`), whose
coeff `connect`s (DSU `connect_dsu.rs:91-104`) into the Tip5
duplex-link witness class — so that wid is **both** a Tip5
perm-A creator **and** ∈ `hint_output_wids`. The
`recompose/coeff` producer's mult resolver
(`recompose.rs:345-350`) tests *only* `hint_output_wids
.contains(wid)` — too coarse: it does not exclude hint wids
`connect`-merged onto an NPO/Tip5 creator ⇒ emits a **duplicate
+1 creator**. **Only D≥2:** the decompose round-trip is reached
only via the `permutation_config.d()==1 && ext_degree>1` branch
(`mmcs.rs:200`); at D=1 there is no decompose / no
`recompose/coeff` rows, so rows 1+2 alone net 0 (= why C2.4-R-a
D=1 is green). Off-by-one = **extra PRODUCE**, single
responsible site = `recompose.rs:345-350` / `recompose_air
.rs:190`.

**Fix (DT-3, principled minimal — the exact analog of an
existing validated carve-out).** `circuit.rs:263-284` already
excludes Const/Public-defined wids (`const_public_wids`) from
`hint_output_wids` *precisely* to stop the Recompose table
double-creating a wid `ConstAir` already creates. Generalize
that carve-out to also exclude **NPO/Tip5 output-index wids**
(`register_non_primitive_output_index` targets) from
`hint_output_wids`. Then for the duplex link row 3's mult →
`else {0}` (the existing arm), net = +1 −1 +0 = **0**. The
recompose arithmetic constraint `Σcᵢ·basisᵢ==x`
(`circuit_builder.rs:1524-1530`) and the Tip5 perm-A/perm-B
creator/consumer pair are **untouched**; only the *duplicate
phantom creator's* multiplicity goes to 0. Soundness: the
duplex link is still produced once (Tip5 perm-A, value pinned
by the verbatim `Tip5PermLookupAir` x⁷/`tip5_l` constraints)
and consumed once (perm-B), so net-0 IS the duplex-chain
binding (perm-B-in == perm-A-out bit-for-bit); the +1 was pure
bookkeeping surplus, not binding. Exactly the spirit of the
`const_public_wids` precedent.

**Blast radius (R1-critical) — SHARED, not Tip5-local.**
`hint_output_wids` (`circuit.rs:272-284`) + the
`RecomposePreprocessor` coeff path are shared with
poseidon1/2 **and the quintic poseidon2 D=1-in-D=5
decompose-link** (`fibonacci_batch_stark_prover_quintic.rs:143`
= the *other* `set_recompose_coeff_ctl_for_decompose_links`
caller). ⇒ invasive soundness-linchpin change; the carve-out
must be **precisely scoped** (exclude a wid only when it is
*both* a hint output *and* an NPO output-index — i.e. genuinely
double-created; never drop a wid recompose is the *sole*
creator of, else it orphans the other direction).

**Decisive staged gate (the arbiter; R1, KAT-first).**
- **G1 D=1 byte-identical** (the precise-scoping arbiter — if
  the carve-out over-excludes, D=1 is no longer byte-identical
  or poseidon orphans the other way): full C2.0–C2.4 D=1 suite
  + orig 7 `test_tip5_layer0_recursion` +
  `p3-tip5-circuit-air` 14 + `test_tip5_lookups` 2 +
  `challenger_transcript` 46 + `p3_recursion` 32, green &
  unchanged.
- **G2 blast-radius arbiter**: poseidon1/2 10/10 **+
  `fibonacci_batch_stark_prover_quintic`** (THE specific
  shared-path regression the `hint_output_wids` change risks)
  + recompose proptests + full `cargo test --workspace`
  0-fail. Quintic regression ⇒ carve-out broke the validated
  quintic producer-balance ⇒ **falsified, escalate**.
- **G3 C3 done**: D=2 Tip5 Layer-0 outer cert
  `prove_all_tables`/`verify_all_tables` balances + accepts
  valid + **rejects tampered**, full 120-bit sweep, recursive
  cert **≤ 65 KB**.

**Escalation (hard stop, R1).** Any of G1/G2/G3 fails (esp. G2
quintic = the author's shared-path warning materializing, or
G3 orphan persists/moves): land nothing, revert byte-identical,
record the precise empirical delta, escalate. No weakening, no
fake. **UNVERIFIED (carry forward):** the numeric `wid 11468 →
specific duplex call-site` map + exact `ext_reads` count are
debug-run-only (not source-confirmable); robust to either way —
the responsible site (`recompose.rs:345-350`) and the fix
(scoped `hint_output_wids` carve-out) are unchanged, the gate
is the arbiter.
