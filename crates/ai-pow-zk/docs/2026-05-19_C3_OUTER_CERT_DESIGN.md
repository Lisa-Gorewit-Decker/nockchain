> _Created **2026-05-19** · last updated **2026-05-19** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

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
  `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md`/this doc/memory; close #124.
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

`2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` §2c.C2.4 (R-a landed, this is the
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
(`git diff --stat 27b1e9d -- crates/plonky3-recursion/` empty; orig 7
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

## 11. DT-2+DT-3 driven — §10 DT-3 premise EMPIRICALLY FALSIFIED by op-trace; corrected root cause (triply confirmed); nothing landed (2026-05-19)

Drove **Edit A (DT-2 prover-label D-scale) + Edit B (DT-3
`hint_output_wids` carve-out)** exactly per §10, worktree-
isolated, full staged gate, reviewed line-by-line + gate
re-run + reverted-and-re-reproduced before recording. The
debug-run resolved the §10 **UNVERIFIED** carry-forward — and
**falsified the DT-3 premise**:

- **G1 D=1 byte-identical — PASS.** orig 7 `test_tip5_layer0`
  7/7, `p3-tip5-circuit-air` 14/14, `test_tip5_lookups` 2/2,
  `challenger_transcript` 46/46, `p3_recursion` 32/32,
  `p3_circuit` 354, `p3_circuit_prover` 40. Edit B required an
  **empirically-forced precise-scoping refinement** (collector
  must skip `op_type.starts_with("recompose")` — `recompose.rs:
  180,185` *also* call `register_non_primitive_output_index`
  per-coeff; without the skip, a recompose-*sole* coeff
  (wid 971: `op[120] Hint OUT → op[121] recompose/coeff IN →
  op[130] tip5_perm IN`) was over-excluded ⇒ net −1
  missing-producer at `["1942"=971·D2,p−1,"0"]` — the §10
  PRECISE-SCOPING-RULE violation, caught + fixed; G1 then held).
- **G2 blast-radius — PASS, no regression.** poseidon1/2
  10/10, **`fibonacci_batch_stark_prover_quintic` 1/1** (the
  other `set_recompose_coeff_ctl_for_decompose_links(true)`
  caller — NOT regressed), recompose proptests, full
  `cargo test --workspace` 0-fail except the new D=2 tests.
  ⇒ **Edit A (DT-2) is verified necessary + correct** (no
  double-scale; quintic green) and **Edit B (DT-3) does not
  break the shared path** — DT-2's namespace fix is a sound
  partial advance.
- **G3 C3 — FAILS ⇒ candidate FALSIFIED.** All 5 sweep
  profiles: D=2 `prove_all_tables` SUCCEEDS (genuine
  `ext_degree==2`), `verify_all_tables` REJECTS
  `GlobalCumulativeMismatch: WitnessChecks`. Debug delta =
  the **same** `["22936","10485455180627170985","0"] net +1,
  instance 3 (Tip5 NPO), lookup 241, row 292`.

**Decisive op-trace (resolves §10 UNVERIFIED; falsifies the
DT-3 premise).** wid 11468 = `op[10576] NPO tip5_perm OUTPUT
(out_hit) → op[10577] NPO tip5_perm INPUT (in_hit)` — **ZERO
Hint provenance, ZERO recompose/coeff involvement**. It is a
pure **Tip5 perm-A-output → perm-B-input duplex-chain
witness**, NOT a `connect`/DSU-merged decompose coeff. ⇒ §10's
"+1 = duplicate `recompose/coeff` phantom creator from a
hint-output merged onto a perm wid, closed by the
`recompose.rs:345` `hint_output_wids` carve-out" is
**empirically false on the real orphan**: the
`recompose.rs:345-350` gate Edit B modifies is **never
evaluated for wid 11468**. DT-3 is *structurally incapable* of
closing this orphan. (The earlier §10 ledger inferred the
connect/DSU/hint-merge from source alone; the debug op-trace
shows that inference does not hold for the actual orphan — the
gate, not the source-reasoning, is the arbiter, as R1 requires.)

**Corrected root cause (TRIPLY confirmed: §7.2 op-trace, §9
DT-2 result, §11 DT-2+DT-3 run).** The +1 is the **C2.4
R-a-tail**: a Tip5 **perm-A-out creator vs perm-B-in consumer
`WitnessChecks` multiplicity mismatch at D=2** — the Tip5
duplex chain's output→input link does not net to 0 at D≥2.
Located in one of three sites, **all soundness-critical**:
(a) the Tip5 circuit-AIR `eval` WitnessChecks emission
(`tip5-circuit-air/src/air_circuit.rs:330-357`, fenced),
(b) `verify_p3_uni_proof_circuit`'s Tip5 challenger/MMCS
perm-chain decompose wiring (fenced forgery-binding linchpin),
or (c) the Tip5 **prover preprocessor**'s perm-A-out/perm-B-in
`out_ctl`/`in_ctl` D-scaling reconciliation
(`circuit-prover/src/batch_stark_prover/tip5.rs:429-443` — the
*least-fenced*, prover-side, where DT-2/Edit A already lives).
Disposition (R1): genuine in-flight attempt #3 → concrete wall
+ design empirically falsified. Validated subset already landed
= C2.0–C2.4-core + R-a (D=1, byte-identical, fenced linchpin
intact); **nothing new safe to land** (no partial soundness
change here is safe ⇒ the clean byte-identical baseline IS the
R1 validated outcome). Reverted byte-identical (`git diff
--stat` empty, file hashes == baseline, workspace green
post-revert). No weakening, no fake, no stubs/ignores.

### 11.1 Precise actionable residual — DT-4 (corrected; the §10/DT-3 design is SPENT/falsified)

C3/#124's closing fix = DT-2 (Edit A, **keep — verified
necessary**) **+** a soundness-faithful reconciliation of the
Tip5 perm-A-out→perm-B-in duplex `WitnessChecks` multiplicity
at D=2. **DT-4 must (R1 KAT-first, before ANY fenced edit; the
§10 reasoning-only ledger was falsified — DT-4 must be
op-trace/debug-confirmed, not source-inferred):**
1. From a real D=2 debug-lookups run, build the op-level
   producer/consumer ledger for wid 11468 (`op[10576]` tip5
   perm-A OUT, `op[10577]` tip5 perm-B IN) across the Tip5
   AIR `eval` emission (`air_circuit.rs:330-357`), the Tip5
   prover preprocessor `out_ctl`/`in_ctl` resolution
   (`tip5.rs:429-443`, DT-2-scaled), and `verify_p3_uni_proof
   _circuit`'s Tip5 perm-chain wiring — pin the exact +1
   off-by-one and which of (a)/(b)/(c) owns it.
2. Prefer the **least-fenced** surface: (c) the prover
   preprocessor (where Edit A already is) — can the perm-A-out
   creator-mult vs perm-B-in consumer-mult be reconciled at
   D=2 in the preprocessor accounting (mirroring how poseidon's
   preprocessor handles its perm-chain) WITHOUT touching the
   fenced AIR `eval` / `verify_p3`? If not, (a) then (b), each
   with its own soundness argument (the duplex link MUST stay
   bound by the Tip5 x⁷/`tip5_l` constraints + challenger/MMCS
   recompute — net-0 must be the *consequence* of the binding,
   not a bookkeeping patch that hides an unbound value).
3. Note: the validated quintic path has **no** perm-output→
   perm-input D=2 chain, so this is **NOT** a mechanical mirror
   of a validated mechanism — it needs its own design +
   soundness proof + KAT-first + the §10 staged gate
   (G1 byte-identical / G2 quintic+poseidon no-regress / G3
   D=2 outer cert accept+tamper+sweep+≤65 KB), per-stage full
   re-validation. M12/#127-adjacent. DT-2 (Edit A) carries
   forward as the verified-necessary partial advance.

## 12. DT-4 diagnosis — DEBUG-CONFIRMED: the +1 is a D=2 EF→base VALUE divergence (NOT bookkeeping); fix surface = non-fenced executor/lift (2026-05-19)

Drove the DT-4 diagnosis (Edit A applied; real D=2 debug-lookups
run; op-level `t.operations` dump; both `with_debug_lookups`
*and* the production `verify_all_tables` path). This is the
**first debug-CONFIRMED** characterization (prior §8/§10 were
source-inferred and falsified) and it refutes 2(i)–(iv) of the
§11.1 plan.

**Confirmed ledger @ idx 22936, D=2 PROD** (instance 3 = Tip5
NPO; `wid 11468`):
- perm-A `OUTPUT_LIMB[0]` SEND **+1** `[22936, V1=
  10485455180627170985, 0]` row 292
- perm-B `INPUT_LIMB[0]` RECEIVE **−1** `[22936, V2=
  2007669758051029367, 0]` row 293

Same idx (Edit A worked — namespace consistent; no idx-11468
Tip5 residue, only a benign Alu-internal ±1 pair), mult
magnitude exactly 1 each (count resolver `tip5.rs:429-443`
correct), **but different VALUES** ⇒ two non-cancelling multiset
tuples. Globally the WitnessChecks multiset sums to 0 (the
+1/+1/−1/−1 across orphan pairs incl. wid 19476) but splits
into 4 non-cancelling tuples; `check_lookups` lex-sorts and
panics on the first = the established `["22936",V1,"0"] +1,
inst 3, lookup 241, row 292`. Production `verify_all_tables`
independently rejects the same (genuine `ext_degree==2`).

**Root cause (op-trace CONFIRMED, raw `t.operations`).** The
duplex idx-wiring is correct (perm-B `in_idx` == perm-A
`out_idx` == 11468..11472). But the **(idx,value) pairing
diverges already in `t.operations` (executor-side)**: perm-A
(op 36) records output `V1 = base_Tip5(coord-0-projected
inputs)` while perm-B (op 37) records input `V2` = the EF
`LiftTip5` value the circuit witness actually holds (==
`ctx.get_witness`, == the clean perms' value). `LiftTip5`
(base-Tip5 on coord-0, re-embed coord-0 only) is **lossy on EF
coord-1**: the base-trace recompute in
`generate_tip5_circuit_main` round-trips perms whose coord-1 is
zero (the "clean" pairs cancel) but diverges for those with
non-zero coord-1 (perm-A-out value ≠ the witnessed value
perm-B reads). ⇒ a genuine **D=2 EF→base value-materialization
inconsistency**, *upstream* of all of (a) AIR `eval`
(`air_circuit.rs:330-357`, faithful), (b) `verify_p3` (no
emission here — inst 3 only), (c) the `tip5.rs:429-443` count
resolver (count correct). 2(i) count, 2(ii) idx-split, 2(iii)
CAP-lane, 2(iv) double-producer — **all refuted by the run**.

**Owning surface = non-fenced Tip5 D=2 executor/lift
EF→base materialization** (`circuit/src/ops/tip5_perm/
executor.rs` + `LiftTip5` + `generate_tip5_trace::<Challenge>`
/ the base recompute in `generate_tip5_circuit_main`). Does
**NOT** require editing fenced `air_circuit.rs`/
`Tip5PermLookupAir`/`tip5_l`/`tip5_spec`/`generation_lookup`/
`verify_p3_uni_proof_circuit`/`recompose*`/`circuit.rs`. The
prover preprocessor (c) and AIR (a) faithfully transcribe
whatever `t.operations` already contains; the orphan is baked
in before they run.

### 12.1 DT-4 fix (soundness constraint + the precise residual)

**Soundness (R1-critical — the agent's explicit forgery
warning).** The +1 is a real value inconsistency, NOT a
miscount. A bookkeeping patch (zero one side's multiplicity)
would **hide an unbound value ⇒ a Tip5-duplex forgery hole** —
strictly forbidden. The fix must make net-0 a **consequence of
the duplex binding**: perm-A's recorded base output value must
equal the EF `LiftTip5` value the circuit witness carries (==
what perm-B reads via `ctx.get_witness`), so perm-A-out and
perm-B-in carry the *identical* value and the multiset cancels
*because* the Tip5 x⁷/`tip5_l` + challenger/MMCS binding holds
— not because a count was nulled.

**Fix direction (debug-confirmed surface; exact line
UNVERIFIED — first impl step).** Make the executor derive the
perm-A output trace value from the **resolved circuit witness
it writes** (the value perm-B will read), not an independent
base recompute of a lossily coord-0-projected `input_values`.
**UNVERIFIED (must pin before editing — one targeted EF-witness
trace, per R1 KAT-first; do NOT infer, §10 fell to exactly
that):** (1) whether the divergence enters at the executor
`ctx.get_witness` EF→base projection (`executor.rs:~408`) vs
the `generate_tip5_circuit_main` base recompute; (2) whether a
complementary `LiftTip5`/`generate_tip5_trace::<Challenge>`
faithfulness fix is needed so *all* perms (not only
coord-1-zero) round-trip. Pin (1)+(2) by a targeted trace
(dump full EF witness 11468 + op-36 true EF inputs) as DT-4
impl step 1, THEN the minimal non-fenced edit, THEN the §10/§11
staged gate **G1** D=1 byte-identical / **G2** quintic+poseidon
+ full-workspace no-regress / **G3** D=2 outer cert
balances+accept+**tamper-reject**+sweep+**≤65 KB**, per-stage
full re-validation. Falsify-and-escalate (no weaken/no fake) on
any gate fail. DT-2/Edit A carries forward verified-necessary.

## 13. DT-4 DRIVEN — SOUNDNESS CLOSING-FIX SOLVED + EXHAUSTIVELY VALIDATED; C3 blocked SOLELY by the orthogonal, fix-independent M-S5 ≤65 KB (2026-05-19)

Drove DT-4 (Edit A + Step-1 debug-pin + the non-fenced fix +
the full staged gate), worktree-isolated. Outcome: **the C3
WitnessChecks duplex-binding closing-fix is correct, complete,
and exhaustively validated.** The §12 root cause was itself
**partly inferred and the run REFUTED its mechanism** — DT-4's
debug-pin is the authoritative truth:

**Step-1 PIN (debug-CONFIRMED; §12's "`LiftTip5` lossy on EF
coord-1" is EMPIRICALLY REFUTED).** Every EF coord in the run
is `[x,0]` (coord-1 always 0 — no lossy EF→base projection).
The actual mechanism is a **merkle-swap slot↔idx desync**:
`apply_merkle_swap` (`circuit/src/ops/tip5_perm/executor.rs:
152-159`) exchanges digest halves `[0,5)↔[5,10)` by the
runtime `mmcs_bit`, but `build_trace_row`/`preprocess_ctl`
record `input_indices` from the **static pre-swap** slot order
while `input_values` is the **post-swap** state. For a leaf-
compress perm (CTL inputs in `[0,5)`, `out_ctl` all false) with
`mmcs_bit=1`, the bus INPUT tuple becomes `(idx=wid 11468,
value=sibling V2)` — a value wid 11468 does not hold — so it
cannot cancel perm-A's `(idx=wid 11468, value=V1)`. Run
classification (1920 merkle perms): 400 leaf-with-CTL-in/
`out_ctl=false` (**the desync class**), 1120 chained-only
(bus-neutral), 400 `out_ctl=true` w/ no CTL inputs (no
input-desync); the risky "leaf CTL-in + `out_ctl=true`" class
= **0** occurrences. Pinned values: perm-A op144/op36 →
`V1=10485455180627170985`; perm-B op145/op37 `resolved_in[0]
=V2=2007669758051029367`, `resolved_in[5]=V1` (swap moved wid
11468 slot 0→5). No `LiftTip5`/`generate_tip5_trace::<Challenge
>` faithfulness fix needed.

**The validated non-fenced fix (recipe — recorded so it is NOT
re-derived).** In `execute_mmcs`, for `!has_ctl_output(outputs)`
perms, record the **pre-swap** state into the trace row
(`build_trace_row(&bus_state)` with the pre-swap `bus_state`)
while `exec`/`write_outputs`/chain-update still use the
**post-swap** state. ⇒ perm-B's INPUT_LIMB carries
`get_witness(inputs[i])` (= V1 = perm-A's bound challenger
output) ⇒ the multiset cancels **because the duplex `connect` +
verbatim `Tip5PermLookupAir` x⁷/`tip5_l` constraints +
challenger/MMCS recompute bind it** — net-0 as a *consequence
of binding*, **no multiplicity changed**. Merkle-root binding
(`update_chain_state` = `exec(post-swap)`, carried forward,
`connect`-ed to the claimed root by the final `out_ctl=true`
perm, `circuit/src/ops/mmcs.rs:108-196`) is **bit-for-bit
untouched**. Edits limited to non-fenced executor + Edit A; no
`air_circuit.rs`/`Tip5PermLookupAir`/`tip5_l`/`tip5_spec`/
`generation_lookup`/`verify_p3`/`recompose*`/`circuit.rs`
/count-resolver edits; no relaxed/stub/ignore.

**Gate (driven, full re-run):**
- **G1 D=1 byte-identical — PASS:** orig 7 `test_tip5_layer0`
  7/7, `test_tip5_lookups` 2/2, `p3-tip5-circuit-air` 14/14,
  `challenger_transcript` 46/46, `p3_recursion` 32/32.
- **G2 no-regression — PASS:** `fibonacci_batch_stark_prover
  _quintic` 1/1 (shared-path arbiter), poseidon1/2 10/10;
  29/30 workspace binaries 0-fail (only the 5 new accept tests
  fail — *solely* at the ≤65 KB assertion, not on soundness).
- **G3 soundness — PASS (orphan CLOSED):** D=2
  `verify_all_tables` **ACCEPTS on ALL 5 sweep profiles**
  (PROD/LB2/LB4/LB5/LB6); tamper-reject **PASS** (in-circuit
  `WitnessConflict`); baseline-revert re-confirms the pre-fix
  rejection (the global-sum check is live, genuinely
  `ext_degree==2`, not bypassed).
- **G3 size — FAIL (the SOLE blocker; fix-INDEPENDENT,
  orthogonal):** serialized `BatchStarkProof` = PROD 119866 /
  LB2 119929 / LB4 119758 / LB5 119735 / LB6 118355 bytes
  (115.58–117.12 KB) vs the 66560-byte (≤65 KB) M-S5 budget —
  **~52 KB over on every profile**. Independent of DT-4 (a
  function of D=2 batch-STARK table heights + FRI
  `log_blowup`/`num_queries`/Merkle-depth over the full Tip5
  Layer-0 verifier circuit; DT-4 changes only `input_values`,
  not heights/table-count/FRI; §9's attempt serialized a
  same-class size). The ≤65 KB assertion was **NOT relaxed**;
  per the hard rule the soundness edit was reverted to
  byte-identical baseline (executor.rs == baseline; orig 7 7/7
  post-revert; baseline D=2 orphan re-confirmed rejecting).

**Status (R1, honest — NO fake completion).** The C3
*soundness closing-fix* is **DONE + exhaustively validated**
(debug-pinned, non-fenced, soundness-argued, G1/G2/G3-soundness
green on all 5 sweeps + tamper-reject). **C3/#124 is NOT done**
— it is blocked **solely** by the orthogonal, fix-independent
**M-S5 ≤65 KB** size target (~117 KB actual). Nothing landed
(clean baseline; only Edit A + the unrelaxed G3 harness
uncommitted). DT-2/Edit A carries forward verified-necessary.

### 13.1 Precise actionable residual — the M-S5 ≤65 KB size (orthogonal to DT-4)

C3's *soundness* is solved (recipe above). The remaining
milestone gap is purely **certificate size**: the D=2 Tip5
Layer-0 verifier batch-STARK serializes to ~116–117 KB under
every sweep profile, ~52 KB over the M-S5 ≤65 KB target. This
is **not a `WitnessChecks`/soundness problem** and no
DT-class change touches it. Two actionable directions (each
its own design + gate; M12/#127-adjacent):
1. **Re-derive / re-scope the M-S5 ≤65 KB budget against the
   *actual* Tip5 Layer-0 verifier circuit.** The 65 KB figure
   may have been set against a smaller/different proven AIR;
   confirm what circuit + FRI params it was derived for vs the
   ~117 KB inherent size of *this* full verifier circuit at
   D=2 across the 120-bit sweep.
2. **Certificate-size reduction**: FRI parameter retune
   targeting serialized size (vs the current 120-bit-soundness
   sweep), recursion/proof folding, or proving a smaller AIR —
   none affecting the DT-4 soundness fix (which is independent
   and ready to re-apply via the recorded recipe).

Carry forward verbatim so the next attempt does NOT re-derive:
**Edit A (DT-2) verified-necessary**; the **debug-pinned root
cause = merkle-swap slot↔idx desync** (NOT EF-coord-1-lossy,
NOT recompose-coeff, NOT a count); and the **validated
non-fenced pre-swap-bus-state fix recipe + soundness argument**
(§13). **R-b** (ai-pow-zk's actual M10.1c composite
`RecursiveAir`) remains M12/#127, out of scope.

## 13.2 DT-4 soundness subset LANDED (user-directed; independently re-validated) — 2026-05-19

§13's "nothing landed" was the state at the end of the DT-4
drive (the soundness edit reverted per the gate's hard rule
because the *orthogonal* ≤65 KB sub-gate failed). User decision:
**land the validated soundness subset** (R1 "maximal correct
exhaustively-validated subset + precise residual"; size recorded
as a precise unmet residual, NOT relaxed/faked). The §13 recipe
was re-applied and **independently re-validated by the
orchestrator** (not blind-trusting the implementing agent):

- **Fenced-linchpin byte-identical proof (decisive):** exactly 3
  files changed (`circuit-prover/.../tip5.rs` Edit A,
  `circuit/src/ops/tip5_perm/executor.rs` the fix,
  `recursion/tests/test_tip5_layer0_recursion.rs` tests);
  `git diff` vs `6bf5bd3` is **empty** for every fenced path
  (`air_circuit`/`air_lookup`/`generation_lookup`/`tip5_spec`/
  `circuit.rs`/`mmcs.rs`/`recompose*`/`verifier`/`fri`). The
  C2.1/L4/L5/C2.4-R-a linchpin is bit-for-bit intact.
- **Diff = §13 recipe exactly:** Edit A only prover arg
  `1`→`witness_ctl_scale` (verifier stays `1`); executor
  `bus_state` cloned *before* `apply_merkle_swap`,
  `exec`/`write_outputs`/`update_chain_state` keep post-swap
  (Merkle-root binding untouched), pre-swap state *only* for
  `!has_ctl_output` trace rows, `has_ctl_output==true` ⇒ exactly
  baseline. **No `out_ctl`/`in_ctl`/multiplicity touched** —
  net-0 is a consequence of the (untouched) duplex binding.
- **Independent gate reproduction (orchestrator-run):**
  `test_tip5_layer0_recursion` 14 pass / 0 fail / 1 ignored
  (orig 7 G1 + 5 D=2 sweep-accepts + 2 D=2 tamper-rejects; the
  ignored = the size-residual), `p3-tip5-circuit-air` 14/14,
  `test_tip5_lookups` 2/2 (G1 D=1 native-equiv + C2.4-R-a D=1),
  `fibonacci_batch_stark_prover_quintic` 1/1 (G2 shared-path
  arbiter, no regression). The full 30-binary G2 workspace pass
  was the implementing agent's run.
- **No fake:** the ≤65 KB M-S5 bar is preserved verbatim
  (`assert!(serialized_len <= 65_536, …)`) in a separate,
  honestly-`#[ignore]`d, openly-tracked residual test (fails
  truthfully under `--ignored`); the always-run soundness tests
  carry no size assertion.

**LANDED (validated):** the C3 DT-4 *soundness closing-fix*
(Edit A + the non-fenced merkle-swap-desync executor fix + the
honest test harness). **C3/#124 is NOT complete** — it remains
blocked **solely** by the orthogonal, fix-independent **M-S5
≤65 KB** size target (~116–117 KB actual; §13.1 residual,
M12/#127-adjacent). No completion claimed for C3; the soundness
linchpin advance is now committed rather than held as a recipe.

## 14. C3 size de-risk — DECISIVE MEASUREMENT (user-directed "measure both tiers first"; 2026-05-19)

The "vertical-recursion ≤65 KB" milestone is Pearl §4.7/§5.1
*compression*; the landed ~117 KB cert is the Pearl-"Layer-1"
recursive STARK. **Critical finding (source-verified):** that
cert's *outer/wrapper* FRI tier is `config::goldilocks_tip5()`
= `FriParameters::new_testing` (log_blowup=2, num_queries=2,
pow=1) ≈ **~5 conjectured FRI bits** — chosen only because
lb=2 is the min that proves the degree-4 Tip5 AIR. The *inner*
Tip5-L0 proof is the genuine 120-bit sweep; the recursive
**wrapper is ~5-bit** (orthogonal to, and not a regression of,
the validated DT-4 WitnessChecks duplex binding). User chose
to measure the size/soundness tradeoff at both tiers before
deciding. Additive-only de-risk (new `config::goldilocks_tip5
_120bit*` siblings — `goldilocks_tip5()` byte-identical; new
`recursion/tests/test_tip5_layer0_compression.rs`, all heavy
tests `#[ignore]`d but genuinely run); fenced linchpin
byte-identical vs `14116b0`; orchestrator-reproduced gate green
(test_tip5_layer0 14/0/1, quintic 1/1, p3-tip5-circuit-air
14/14). All numbers are real `prove_all_tables`+`postcard`
(no fabrication; every tamper path genuinely rejects).

**S0.b — L1 PROD `BatchStarkProof` byte breakdown (119 866 B
total, == landed residual figure):** `opened_values` (OOD
poly evals) **65 466 B / 54.6 %** (the dominant term — table
count × cols at D=2, *independent of FRI query count*);
`opening_proof` (FRI) 44 592 B / 37.2 %; `global_lookup_data`
8 464 B / 7.1 %; commitments 912 B; rest <0.3 %. Hypothesis
"FRI dominates" **partially refuted** — OOD opens dominate,
not FRI.

**S1 — L2 mechanism end-to-end works** (accept + tamper-reject
via `verify_p3_batch_proof_circuit`, the validated quintic
template) BUT the L2 verifier circuit has a **~40 KB fixed
floor** (Poseidon2-W8 + recompose + in-circuit FRI fold-chain)
*independent of inner size* ⇒ recursion is net-negative until
the inner ≫ 40 KB.

**S2 — L2 over the REAL ~117 KB Tip5-L0 cert, both tiers
(real, accept+tamper-reject ✅ both):**
| tier | L2 serialized | vs 65 536 B |
|---|--:|--:|
| (a) ~5-bit (`goldilocks_tip5`, lb2/nq2/pow1 ⇒ ~5 bits) | **45 513 B (44.4 KB)** | **≤65 KB ✅** |
| (b) ≥120-bit (`goldilocks_tip5_120bit`, lb2/nq120/pow1 ⇒ 241 conj. bits) | **1 475 156 B (1.44 MB)** | 22× over ✗ |

**S3 — levers at ≥120-bit (real):** high-arity
(max_log_arity=3, lfp=2, *same 241-bit*) → 1 144 308 B (still
17× over); **L3** over L2 → 1 769 913 B (**L3 > L2 — recursion
DIVERGES** at ≥120-bit: each layer's 120-query FRI + full
re-proven verifier-circuit OOD opens exceed the inner it
wraps).

**Decisive conclusion (exact bytes).** ≤65 KB is reachable
**only at the ~5-bit testing tier** (L2(a)=45 513 B). At any
real ≥120-bit tier the existing substrate **cannot** reach
≤65 KB at any measured depth/config (17–27× over; depth makes
it *worse*). Reaching ≤65 KB at ≥120-bit requires a **substrate
addition** (size-targeted terminal compression — a real
SNARK/STARK-to-SNARK wrap with small constant proof, a smaller
proven AIR, or genuine proof-folding — none in the current
`Plonky3-recursion`; M12/#127-adjacent), OR re-scoping the
M-S5 ≤65 KB budget against this circuit's inherent ≥120-bit
size.

**Additional decisive substrate residual (verified).** The
landed `p3_circuit_prover::config::GoldilocksConfig` uses an
**unpacked** `MerkleTreeMmcs<Goldilocks,…>`, but
`verify_p3_batch_proof_circuit` requires
`MerkleTreeMmcs<F::Packing,…>`; on aarch64-neon
`Goldilocks::Packing ≠ Goldilocks` ⇒ the landed-config cert is
**type-incompatible** with the recursion verifier. S2/S3 used a
packed-MMCS, FRI-tier-identical, cap-0 substitute (verified
byte-faithful: S0.b total == landed 119 866 B; soundness-
neutral — SIMD packing changes only the hasher impl, not
committed values/Merkle structure). A recursion-compatible
packed-MMCS `GoldilocksConfig` (or making the recursion
verifier accept the unpacked config) is a **prerequisite for
ANY production L2 landing**, independent of the tier decision.

**Status (R1, honest).** C3 soundness closing-fix remains
DONE+validated+landed (`14116b0`); the ≤65 KB target is now
**proven to require either an explicit ~5-bit-wrapper soundness
trade OR a major out-of-scope substrate build OR an M-S5
budget re-scope** — a soundness/milestone-scope decision that
is the user's (R1: soundness trades require explicit sign-off;
R1 outranks Stop-hook completion pressure here — presenting a
~5-bit cert as "C3 ≤65 KB done" would be fake completion +
silent soundness weakening). No C3 completion claimed; the
de-risk artifact (additive config siblings + measurement
module) is committed as the precise actionable evidence.

## 15. C3/M-S5 RE-SCOPED + soundness-correct ≥120-bit cert LANDED (user-directed; independently re-validated) — 2026-05-19

User decision after the §14 measurement: **re-scope M-S5 to the
soundness-correct ≥120-bit vertical-recursion certificate; defer
≤65 KB to a separate terminal-compression milestone (M-S5b)**.
No soundness trade, no fake. Driven + independently
re-validated by the orchestrator (not blind-trusting the
implementing agent):

**Landed (additive only; 2 test files):**
- `recursion/tests/test_tip5_layer0_compression.rs` +346
  (Stage A `c3_stage_a_l1_120bit_kat`, Stage B
  `c3_stage_b_l2_over_120bit_l1`, Stage C `c3_stage_c_sweep
  _120bit` + helpers; heavy ⇒ `#[ignore]`d but genuinely run).
- `test_tip5_layer0_recursion.rs` 1 line — the
  `tip5_layer0_outer_cert_size_residual` `#[ignore]` reason
  string now points at the deferred **M-S5b**; the
  `assert!(serialized_len <= 65_536, …)` is **byte-verbatim**
  (not relaxed/deleted).
- No `circuit-prover/src/config.rs` change needed (the §14
  de-risk's packed-MMCS `OuterTier::Bit120` sibling is already
  recursion-compatible + ≥120-bit).

**The soundness-correct chain (every link ≥120-bit — fixes the
§14 S2(b) ~5-bit-L1 defect):** inner Tip5-L0 sweep
(`lb·nq/2 = 120` ∀ profile) + L1-outer `OuterTier::Bit120`
(`lb2·nq120 + 1 = 241` conjectured bits) + L2-wrapper
(`Bit120`, 241) ⇒ end-to-end `min ≥ 120` bits. Net-0/accept is
a consequence of the (untouched, byte-identical) DT-4 duplex
binding + ≥120-bit FRI at each layer; tamper genuinely rejects
(`WitnessConflict` at `runner().run()` — the in-circuit
FRI/quotient `connect`, not a bypass).

**Honest real sizes (no fake-shrink):** ≥120-bit L1 ≈
2 753 359 B (2.69 MB); ≥120-bit L2 over the ≥120-bit L1 ≈
1 878 188 B (1.79 MB). All 5 inner sweep profiles
(PROD/LB2/LB4/LB5/LB6) accept-valid + reject-tampered (Stage C,
490.64 s, no compute wall).

**Independent re-validation (orchestrator-run, not the
implementing agent):**
- Fenced-linchpin byte-identical: `git diff b8b5d32 --` EMPTY
  for `air_circuit`/`air_lookup`/`generation_lookup`/
  `tip5_spec`/`circuit.rs`/`mmcs.rs`/`executor.rs`(DT-4)/
  `recompose*`/`recursion/src/verifier/*`/`backend/fri.rs`/
  `config.rs`; change set = exactly the 2 test files.
- ≤65 KB assert verified byte-verbatim (only the ignore-reason
  string differs).
- Reproduced: `c3_stage_a_l1_120bit_kat` ✅ +
  `c3_stage_b_l2_over_120bit_l1` ✅ (2/2, 96.84 s — the
  genuinely-≥120-bit L1 *and* L2-over-≥120-bit-L1, accept +
  tamper-reject, re-run independently); regression slice green
  (`test_tip5_layer0_recursion` 14/0/1, `quintic` 1/1
  shared-path arbiter, `p3-tip5-circuit-air` 14/14).

**Status (R1, honest).** C3/M-S5 (re-scoped) = the
soundness-correct ≥120-bit vertical-recursion cert: **DONE +
exhaustively validated + LANDED**. **M-S5b (≤65 KB terminal
compression) = a NEW, separate, deferred milestone** (not
started; §14 proved it needs a substrate addition NOT in the
current `Plonky3-recursion`). This is **not** hidden C3
incompleteness — the ≤65 KB target was explicitly carved out by
the maintainer into M-S5b; C3's *re-scoped* deliverable is
complete. **R-b** (ai-pow-zk's actual M10.1c composite
`RecursiveAir`, vs the representative config) remains M12/#127,
out of scope.
