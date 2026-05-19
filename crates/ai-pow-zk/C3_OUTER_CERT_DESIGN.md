# C3 / #124 — DT-1 design: D=2 Tip5 outer-cert recompose-coeff producer balancing

> **Status:** DESIGN (DT-1, #129; 2026-05-19). The C3/M-S5 enabler.
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
