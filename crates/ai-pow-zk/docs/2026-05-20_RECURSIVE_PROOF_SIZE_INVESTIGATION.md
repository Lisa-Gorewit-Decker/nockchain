> _Created **2026-05-20** · last updated **2026-05-20**._

# Recursive Proof Size Investigation — in-substrate levers at ≥80-bit soundness

> **Status (R1, honest).** ANALYTICAL + EMPIRICAL. Per maintainer
> directive: "second look at reducing the size of the recursive
> proofs, with an eye toward maintaining 80-bit soundness but
> flexibility otherwise" (2026-05-20).
>
> **Headline finding.** Lowering `log_blowup` from 2 to 3 (with
> proportional `num_queries` reduction) is the **single largest
> in-substrate L1 size lever**: **~31% smaller** (1011 KB →
> 695 KB) at 83 bits unconditional Johnson. Going further to
> `log_blowup = 4` reaches **~46% smaller** (548 KB) at 82
> bits. Combined with digest=4, FRI high-arity, and MMCS cap,
> the projected best in-substrate L1 at ≥80-bit is in the
> **400–500 KB range** — about 2× shrinkage from the current
> production 1011 KB.
>
> **Bottom line.** In-substrate optimization can shave the L1
> size by ~50–60% (1011 → 400-500 KB) while preserving ≥80-bit
> unconditional Johnson soundness and the Tip5-throughout
> architecture. Reaching ≤65 KB still requires Path A
> (outermost SNARK wrap; per
> `2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`).
>
> **What this doc IS.** A systematic, lever-by-lever
> investigation with empirical L1 measurements at each
> variant. Each lever's soundness preservation is derived
> analytically; size impact is measured at the real Tip5-L0
> verifier circuit (FibonacciAir inner; the L1 build path
> validated by P4 of the Poseidon2-removal spec).
>
> **What this doc IS NOT.** A spec to flip any production
> config. Each lever has prover-wall / RAM trade-offs not
> measured here. The final config selection requires
> maintainer judgment on the wall-time vs size trade-off,
> the audit-board comfort with paper-divergent variants
> (digest=4, MMCS cap), and the operational implications of
> larger LDE (lb=4 means 16× trace size).

---

## 1. Soundness floor & methodology

### 1.1 The ≥80-bit unconditional bar

Per S(−1) (`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`) anchored
on IACR ePrint 2025/2055 Theorem 1.5 (Johnson-radius proven):

```
unconditional_bits ≈ log_blowup · num_queries
                  + commit_proof_of_work_bits
                  + query_proof_of_work_bits
```

For a config to satisfy "≥80 unconditional bits at the
Johnson radius", the above sum must be ≥80 AND proximity
testing must stay strictly inside `γ < J(δ)−η` per the paper
§8 attacks (which Plonky3's FRI does by construction).

### 1.2 Per-lever framework

For each lever, we derive:
1. **Soundness preservation**: under what parameter range
   does the variant stay ≥80?
2. **Predicted size impact**: dominant proof-byte component
   affected (opening_proof vs opened_values vs commitments).
3. **Audit-surface delta**: does the variant diverge from
   Tip5 paper spec / standard FRI?
4. **Operational cost**: prover wall, RAM, verifier time
   impact (qualitative; not measured in this doc).
5. **Empirical L1 size** at the variant.

### 1.3 Measurement infrastructure

Two test files in `Plonky3-recursion/recursion/tests/`:

- `test_l1_size_reduction_sweep.rs` — broad sweep (per-lever
  variants in isolation). All `#[ignore]`'d (manual
  invocation; ~30s per variant).
- `test_l1_size_reduction_combined.rs` — focused
  combinations from the broad-sweep findings (Pareto
  candidates; ~30s each).

All measurements use the same inner Tip5-Layer-0 STARK
(FibonacciAir at lb=3 nq=30) + L0 verifier circuit (Tip5 NPO
+ Recompose CTL at D=2; the production-class path validated
by P3+P4 of the Poseidon2-removal landing). The only
variable is the L1 outer-cert config.

---

## 2. Lever catalogue

### 2.1 Lever 1: Tip5 sponge digest truncation (5 → 4 elements)

**Soundness.** Sponge collision security = `min(capacity/2,
output)`. For Tip5: capacity = 6 elements (= 384 bits);
output = 5 (= 320 bits, paper-spec) or 4 (= 256 bits,
investigation variant). Both ≥ capacity/2 = 192 bits ⇒
**192-bit collision resistance either way**. The output
length only constrains collision when output < capacity/2;
neither 5 nor 4 hits that. Cryptographically equivalent.

**Caveat.** Tip5 paper IACR 2023/107 Table 2 specifies
digest=5. Truncating to 4 is a **Nockchain-local variant**
not validated by the paper's published parameter choice.
Audit board may prefer paper-spec digest=5.

**Predicted size.** ~5% smaller per Merkle-node hash
(40 → 32 bytes per node).

**Empirical L1 (P4 measurement; preserved):**
- digest=5 baseline: **1010.7 KB** [86 bits]
- digest=4: **961.2 KB** [86 bits] — **−5%**

### 2.2 Lever 2: FRI num_queries reduction toward the 80-bit floor

**Soundness.** Direct linear knob: bits = `lb · nq + cp + qp`.
- `lb=2 nq=42 pow=1+1` → 86 bits (production current)
- `lb=2 nq=42 pow=0+0` → 84 bits (dropping PoW)
- `lb=2 nq=41 pow=0+0` → 82 bits
- `lb=2 nq=40 pow=0+0` → **80 bits** (exact floor)

**Predicted size.** Each query opens a full FRI path
(log(domain) Merkle hashes + per-round commits). Linear in
`nq`. Smaller `cp/qp` reduces grinding work (cosmetic for
the proof; main effect is per-query work).

**Empirical L1 (broad sweep):**
- nq=42 pow=1+1: 1010.7 KB (baseline)
- nq=42 pow=0+0: 1011.1 KB (no PoW savings; cosmetic)
- nq=41 pow=0+0: 988.8 KB [82 bits] — **−2%**
- nq=40 pow=0+0: 966.5 KB [80 bits] — **−4%**

**Verdict.** Modest savings (~2-4%); the `nq` reduction is
linear-ish in `opening_proof` but not in total bytes
because `opened_values` (OOD opens; nq-independent)
dominates at lb=2.

### 2.3 Lever 3: Larger `log_blowup` (fewer queries needed at same bits)

**Soundness.** `bits = lb · nq + pow`. Higher `lb` means
each query gives more bits ⇒ fewer queries needed at the
same soundness. Example:
- `lb=2 nq=42` → 84 bits
- `lb=3 nq=27 pow=1+1` → 83 bits (~equiv)
- `lb=4 nq=20 pow=1+1` → 82 bits

**Predicted size.** Two opposing effects:
1. `nq` reduction shrinks `opening_proof` (FRI bytes,
   per-query Merkle paths).
2. `lb` increase enlarges the LDE domain (2× per +1
   blowup), which inflates `opened_values` (the OOD opens
   of `lb`-sized polynomials) AND `commitments` (more
   commit-phase nodes to commit).

The net depends on which term dominates. **Empirical
data is the only honest answer.**

**Empirical L1 (broad sweep) — MAJOR FINDING:**
- baseline lb=2 nq=42: 1010.7 KB
- **lb=3 nq=27 pow=1+1: 694.8 KB [83 bits] — −31%**
- lb=3 nq=28 pow=0+0: 717.9 KB [84 bits] — −29%
- **lb=4 nq=20 pow=1+1: 547.9 KB [82 bits] — −46%**

**Verdict.** The `nq`-shrinkage savings dominate the LDE-
inflation cost at our trace sizes. **`log_blowup` is the
largest single in-substrate L1 size lever.** Operational
cost: lb=3 means 8× LDE (vs 4× at lb=2); lb=4 means 16×.
Prover wall + RAM scale roughly linearly with LDE size; need
to measure separately.

### 2.4 Lever 4: FRI high-arity folding (`max_log_arity` + `log_final_poly_len`)

**Soundness.** Soundness-neutral (depends only on `lb·nq +
pow`). High-arity = larger FRI fold per step ⇒ fewer commit-
phase steps. `log_final_poly_len > 0` keeps a small constant
polynomial at the end of the chain instead of a single
element.

**Predicted size.** Trades commit-phase steps (fewer but
fatter per-round commits) against the final tail (now
non-trivial). Net effect varies by trace size; usually 1-3%
savings.

**Empirical L1 (combined sweep; awaiting):** TBD

### 2.5 Lever 5: MMCS cap height (cap > 0)

**Soundness.** Cap height doesn't affect cryptographic
soundness — it's a Merkle-tree shape parameter. Cap `k`
means the top `k` levels of the tree are kept verbatim
(2^k root commitments) so opens never descend below depth
`tree_depth - k`.

**Constraint.** Per existing comment in
`test_tip5_layer0_compression.rs:200-208`:
> "cap > 0 is unsupported by the in-circuit MMCS path, exactly
> as the validated quintic/`goldilocks.rs` recursion configs
> use `MerkleTreeMmcs::new(.., 0)`"

This restriction may not apply to the outer-cert L1's MMCS
(which is for L1's own STARK trace, not for the in-circuit
verifier of L0). Empirical test below.

**Predicted size.** Each query's Merkle path shrinks by `k`
nodes. For tree_depth=17 + cap=3: ~18% fewer Merkle nodes
per query × ~42 queries = noticeable savings on
`opening_proof`. Predicted: 5-10% smaller.

**Empirical L1 (combined sweep; awaiting):** TBD

### 2.6 Lever 6: MMCS arity > 2

**Soundness.** Doesn't affect collision security (same
hash output per node, just more children per node).

**Implementation cost.** Requires new type aliases:
`MerkleTreeMmcs<..., ARITY=4, ...>` + corresponding
`TruncatedPermutation<..., ARITY=4, ...>`. Each compress
step hashes more children (so each compress call has more
work but tree depth shrinks).

**Predicted size.** Tree depth: `log_arity(domain)`. arity
4 halves depth vs arity 2. Per-query Merkle path nodes are
proportional to depth. But each "sibling" at higher arity
is bigger (3 siblings to commit at arity 4 vs 1 at arity 2).
Net depends on per-node-size vs path-length trade.

**Empirical L1.** **Not measured in this investigation**
(requires building new type aliases; deferred follow-on if
the cap=3 + lb=3 combinations don't reach our target).

### 2.7 Lever 7: Reduced-round Tip5 at outer-cert (paper-divergent)

**Soundness.** Tip5 paper specifies N=5 rounds; we use N=7.
Opening the Blackbox (IACR 2024/1900) attacks reach 3 rounds.
Reducing the OUTER-cert Tip5 to 5 rounds (Tip5 paper spec)
keeps 2-round margin above broken; reducing to 4 rounds is
discouraged (1-round margin); reducing to 3 is UNSAFE.

**Predicted size.** Per-round Tip5 perm AIR has ~1338
columns (CSA §3.1); 7→5 rounds = -29% perm AIR columns ⇒
~5-8 KB savings in the Tip5 NPO sub-circuit at L1.

**Empirical L1.** **Not measured** (requires new Tip5-AIR
variant or `register_tip5_table` parameterization;
substantial work). Deferred as follow-on if other levers
don't reach target.

### 2.8 Lever 8: Inner-trace shrinkage

Out of scope — fenced linchpin per C2.1 keystone.

---

## 3. Empirical results (current data)

### 3.1 Broad-sweep findings (`test_l1_size_reduction_sweep.rs`)

| Variant | L1 size | Δ vs baseline | bits |
|---|--:|--:|--:|
| baseline (production) | **1010.7 KB** | — | 86 |
| digest=4 | 961.2 KB | −5% | 86 |
| nq=42 pow=0+0 | 1011.1 KB | 0% | 84 |
| nq=41 pow=0+0 | 988.8 KB | −2% | 82 |
| **nq=40 pow=0+0** | **966.5 KB** | **−4%** | **80** |
| **lb=3 nq=27 pow=1+1** | **694.8 KB** | **−31%** | **83** |
| lb=3 nq=28 pow=0+0 | 717.9 KB | −29% | 84 |
| **lb=4 nq=20 pow=1+1** | **547.9 KB** | **−46%** | **82** |

### 3.2 Combined-sweep findings (`test_l1_size_reduction_combined.rs`)

Empirical measurement results (full sweep, manual run):

| Variant | Bits | L1 size | Δ vs baseline |
|---|--:|--:|--:|
| Lever 5: mla=3 lfp=2 (high-arity, 86-bit) | 86 | 956.1 KB | −5% |
| **Lever 6: cap=3 (MMCS cap, 86-bit) — works!** | 86 | **930.2 KB** | **−8%** |
| d4 + lb=3 nq=27 (combine Lever 1+3) | 83 | 658.7 KB | −35% |
| d4 + lb=4 nq=20 (combine Lever 1+4) | 82 | 517.9 KB | −49% |
| d4 + lb=4 nq=20 + mla=3 lfp=2 | 82 | 489.9 KB | −52% |
| **d4 + lb=4 nq=20 + mla=3 lfp=2 + cap=3 (Pareto)** | **82** | **470.1 KB** | **−53%** |
| d4 + nq=40 pow=0+0 (80-bit at lb=2) | 80 | 918.5 KB | −9% |
| d4 + lb=4 nq=20 pow=0+0 (80-bit at lb=4) | 80 | 517.9 KB | −49% |
| **d4 + lb=4 nq=20 pow=0+0 + mla=3 lfp=2 + cap=3 (80-bit Pareto)** | **80** | **470.2 KB** | **−53%** |

**Findings:**
1. **Lever 6 (MMCS cap=3) WORKS at the outer-cert** — the
   `cap > 0 is unsupported by in-circuit MMCS path` comment in
   `test_tip5_layer0_compression.rs` does NOT apply to the L1
   outer-cert MMCS (which is for L1's own STARK trace
   commitments, not for the in-circuit L0 verifier). Cap=3
   shrinks per-query Merkle paths by 3 nodes ⇒ ~8% L1 savings.
2. **`pow_bits` doesn't change proof size** — going from
   pow=1+1 to pow=0+0 produces statistically-identical proof
   sizes (the 80-bit-floor lb=2 nq=40 pow=0+0 is 966.5 KB; the
   same with pow shows 1011.1 KB — that's the nq=42 baseline,
   not a pow comparison; the lb=4 nq=20 pow=0+0 vs pow=1+1
   shows 517.9 vs 517.9 KB). PoW costs the prover work but
   doesn't add proof bytes.
3. **Levers compose additively** but with diminishing
   returns — each combination saves ~+5-10% on top of the
   strongest underlying lever.
4. **Pareto-optimal in-substrate: 470.1 KB at 82 bits**
   (digest=4 + lb=4 nq=20 + mla=3 lfp=2 + cap=3) — a 2.15×
   shrinkage from production baseline (1011 KB) while
   maintaining ≥80-bit unconditional Johnson soundness.

---

## 4. Recommendation framework

### 4.1 Conservative path (audit-baseline; paper-faithful)

Stay closest to Tip5 paper + standard FRI:
- digest=5 (Tip5 paper spec; no truncation)
- lb=2 nq=42 pow=1+1 (current 86 bits)
- mla=1, lfp=0, cap=0 (binary FRI; no MMCS cap)
- **L1 size: 1010.7 KB** (current production)

### 4.2 Moderately aggressive (recommended starting point)

Stays paper-faithful on digest; adopts `lb=3` for the size
win without committing to lb=4's larger LDE cost:
- digest=5 (paper spec)
- **lb=3 nq=27 pow=1+1** (83 bits unconditional, +3-bit
  margin over 80 floor)
- mla=1, lfp=0, cap=0
- **L1 size: 694.8 KB** (-31% from production)
- Prover-wall trade-off: ~2× LDE (8× vs 4×); need to
  measure separately.

### 4.3 Aggressive (digest-divergent but cryptographically safe)

Adopts digest=4 + lb=4:
- digest=4 (Nockchain-local variant; 192-bit collision
  preserved; not paper-spec)
- lb=4 nq=20 pow=1+1 (82 bits; +2-bit margin)
- **L1 size: 517.9 KB (measured) — −49%**
- Trade-offs: 16× LDE; paper-spec divergence on digest.

### 4.4 Pareto-aggressive (ALL compatible levers stacked)

- digest=4 + lb=4 nq=20 + mla=3 lfp=2 + cap=3
- **L1 size: 470.1 KB at 82 bits (pow=1+1) — −53%**
- **L1 size: 470.2 KB at 80 bits (pow=0+0) — −53%** (saving
  2 bits of soundness adds zero proof bytes; PoW is prover-
  work-only)
- Maximum in-substrate compression at 80-82 bits; stacks all
  compatible savings:
  - digest=4 (paper-divergent; 192-bit collision preserved)
  - lb=4 (16× LDE; biggest single lever at -46%)
  - mla=3 lfp=2 (high-arity FRI; soundness-neutral)
  - cap=3 (MMCS cap; soundness-neutral)

**The cumulative effect of stacking 4 levers: 2.15× shrinkage**
from 1011 KB → 470 KB. Each additional lever adds ~5-10% on
top of the previous level — diminishing but real returns.

### 4.5 Recommended starting flip (lowest-R1-risk; largest single win)

If the audit board accepts a single FRI parameter change to a
SLIGHTLY larger LDE:

**FLIP: `goldilocks_tip5_80bit()` → `lb=3 nq=27 pow=1+1`**

- digest=5 (paper-faithful; no change from current)
- All other knobs unchanged (mla=1, lfp=0, cap=0)
- ONLY change: `log_blowup 2 → 3` + `num_queries 42 → 27`
- **L1 size: 694.8 KB (measured) — −31%**
- 83 bits unconditional (+3-bit margin over 80 floor)
- Prover-wall trade-off: 8× LDE vs current 4× → roughly 2× prover
  memory + ~1.5-2× wall time

This is the **single highest-value, lowest-risk change** that
maintains ≥80-bit unconditional Johnson, paper-faithful
digest, and doesn't introduce new parameters. **One FRI
parameter change for a 31% L1 savings.**

### 4.6 The path to ≤65 KB

None of the in-substrate variants reach ≤65 KB. **In-substrate
floor: 470 KB** (measured at 82 bits). Per the routes audit
(`2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`): **Path A
(outermost STARK-to-SNARK wrap) is required for ≤65 KB.** This
investigation establishes the empirical in-substrate floor;
below that requires Path A.

The in-substrate floor IS substantially better than the
current 1011 KB though. **Production-flipping to the
moderately aggressive variant (lb=3 nq=27) is the highest-
value, lowest-risk improvement** — single FRI parameter
change, no paper divergence, well-understood soundness math.

---

## 5. L2-over-L1 in Tip5-throughout substrate (M-S5b S1.B Stage 5; 2026-05-20)

The 2026-05-20 production config (`config::goldilocks_tip5_80bit`,
post-Phase-0 cumulative-lever stack: `lb=4 nq=20 mla=3 lfp=2
cap=3 pow=1+1 d=5`, 82 bits unconditional Johnson) was measured
at the real L1 + L2 layers via
`Plonky3-recursion/recursion/tests/test_tip5_l2_over_l1.rs::
stage5_tip5_l2_over_l1_production_measurement`.

**Substrate correction (2026-05-20):** the original Stage 5 runs
(commits `c50e3e8`, `2c12fe3`) used `make_tip5_outer_cfg` with
`cap=0` in the test infrastructure, while the production builder
uses `cap=3`. The Path B Stage B0 inventory (commit `e8b...`)
surfaced this divergence; the test was corrected to `cap=3` and
re-run. Production-faithful numbers below; cap=0 numbers retained
for delta-comparison context.

| Production config | L1 | L2 | L2/L1 | Substrate |
|---|--:|--:|--:|---|
| `lb=4 nq=20 mla=1 lfp=0 cap=0` ("Tier B" at cap=0) | 547.88 KB | 646.76 KB | 1.18× | test (cap=0) |
| `lb=4 nq=20 mla=3 lfp=2 cap=0` (Phase 0 at cap=0) | 512.24 KB | 543.68 KB | 1.06× | test (cap=0) |
| **`lb=4 nq=20 mla=3 lfp=2 cap=3` (Phase 0 production)** | **487.65 KB** | **519.18 KB** | **1.06×** | production (cap=3) |

**Pre-2026-05-20 baseline:** ~1011 KB L1 at `lb=2 nq=42 mla=1
lfp=0 cap=0`. Cumulative L1 reduction at production-faithful
post-Phase-0: **~−51.8% (1011 → 487.65 KB)**.

**Headline finding (counterintuitive but real):** L2 is **larger
than** L1 in the Tip5-throughout substrate. The L2 verifier
circuit recomputes L1's Tip5 MMCS commitments in-circuit +
verifies L1's FRI fold chain in-circuit; that overhead (Tip5 NPO
traces + recompose NPO traces + in-circuit FRI verifier logic)
exceeds the saving from "collapsing" the inner L1 STARK into the
L2 wrapper. Phase 0 narrowed the inflation from 18% to 6% but
did not invert it.

**Secondary finding (Phase 0):** the soundness-neutral levers
(`mla=3`, `lfp=2`) save **more** at L2 than at L1 (-15.9% vs
-6.5%). Hypothesis: high-arity FRI folding + non-trivial final-
polynomial tail compound at the L2 layer because L2's verifier
circuit has more FRI rounds to fold (verifying L1's larger FRI
chain). This is a positive cascading effect — the same lever
helps more at deeper recursion layers.

**Why the contrast vs Poseidon2-substrate L2** (historical
baseline `c3_stage_b_l2_over_120bit_l1`: L1 ~961 KB, L2 ~618 KB,
ratio 0.64×): the Tip5 NPO AIR is meaningfully wider/heavier
than the Poseidon2 NPO AIR (Tip5 paper IACR 2023/107 §4: 7-round
permutation over W=16 state vs Poseidon2-W8 simpler). The
in-circuit Tip5 verifier needed at every recursion layer
generates more rows + columns of NPO trace, inflating L2's
post-FRI bytes despite the smaller L1.

**Implication for further compression:** stacking more recursion
layers (L3 over L2, L4 over L3, …) does NOT compress toward
≤65 KB in this substrate. Each layer ADDS the verifier circuit's
overhead. The recursion chain is **size-monotone-non-decreasing**
in Tip5-throughout — opposite the implicit assumption that
deeper recursion shrinks the cert.

**Acceptance status:** ACCEPT ✅, tamper-REJECT ✅, soundness chain
MIN(inner L0 PROD 90b, L1 Tier B 82b, L2 Tier B 82b) ≥ 80 bits
unconditional Johnson at every link. SUBSTRATE: 100% Tip5
(zero Poseidon2 in the trust surface — per
`MEMORY.md::no_poseidon2_anywhere` hard rule).

**What this changes about the path to ≤65 KB:**

| Path option | Implication after 2026-05-20 Stage 5 |
|---|---|
| **More recursion layers (L3+, L4+)** | NOT a path to ≤65 KB. L_{n+1} > L_n in Tip5-throughout. |
| **Path A (outermost STARK-to-SNARK wrap)** | Still the only known path to ≤65 KB. Now even more clearly the right architectural move. |
| **Path B (verifier-AIR slim)** | Now MORE valuable: every column removed from the verifier circuit cuts size at EVERY recursion layer (compounding). |
| **Tier C (in-substrate Pareto)** | Still applies at L1 (~470 KB measured). L2 effect not measured at Tier C; likely L2 ~550-600 KB by analogy. |

**Production status (post-Phase-0 + Path B B2, cap=3
production-faithful):**

| | L1 | L2 | L2/L1 |
|---|--:|--:|--:|
| Pre-Phase-0 (post-Tier-B alone, cap=3) | ~520 KB (est.) | ~570 KB (est.) | ~1.10× |
| Post-Phase-0 (commit `97db66d`) | 487.65 KB | 519.18 KB | 1.065× |
| Post-Path-B B2 (commit `ce3e6a4`) | 488.47 KB | 518.88 KB | 1.062× |

Cumulative L1 reduction from pre-2026-05-20 baseline (~1011 KB):
**~−51.7%**. L2 inflation barely improved (ratio 1.065× → 1.062×).

**REVISED FINDING (post-Path-B B2):** the B1 hypothesis that
verifier-AIR slim (Path B) would flip the L_{n+1} < L_n ratio
by reducing per-layer overhead is **partially falsified**. The
B2 reduction halved Alu rows (6,851 → 3,401) without moving L1
bytes — because `tip5_perm` is the FRI Merkle height bottleneck
(both tables tied at 1024-row commitments pre-B2; tip5_perm
remains 1024 post-B2 while Alu dropped to 512). Reducing
non-bottleneck-table heights doesn't shrink the FRI proof.

**For L1 reduction to actually move, the reduction must target
the TALLEST table.** In our substrate, that's `tip5_perm`.
Tip5 perm calls in the verifier circuit are intrinsic to:
- FRI commit-phase MMCS (per-query Merkle path hashing).
- Opening-proof MMCS (per-commitment Merkle authentication).
- Fiat-Shamir challenger absorbs (~11 calls).

Of these, only Fiat-Shamir absorbs are batchable in-substrate
(per B1 § B.1, ~1% saving). The 870+ MMCS-related Tip5 calls
are intrinsic to the verifier's job; reducing them requires
either:
- A different MMCS construction (substrate change).
- Reducing query count (already at nq=20 floor).
- Reducing path depth (cap=3 already maxed).

**This sharpens Path A's role:** Path B's in-substrate
post-quantum L1 floor is ~488 KB. ≤65 KB requires Path A.

The path to ≤65 KB still requires Path A (SNARK wrap;
trust-surface trade). Phase 0 + Path B B2 stabilize the
production L1 at ~488 KB; further Path B reductions targeting
non-bottleneck tables won't shrink it. Reaching ≤65 KB requires
either:
- **Path A** (post-quantum trust-surface trade, the only known
  realistic path).
- **STIR/WHIR PCS swap** (substrate change; ~−50% per layer;
  still wouldn't reach ≤65 KB alone).
- **Reducing `tip5_perm` row count substantially** — would
  need a different MMCS construction, which is itself a
  substrate change.

---

## 6. R1 honest residuals (formerly §5)

What this investigation does NOT close:

1. **Lever 6 (MMCS arity > 2)**: not measured; requires new
   type aliases. Defer to follow-on if higher impact
   needed.
2. **Lever 7 (reduced Tip5 rounds at outer-cert)**: not
   measured; requires new Tip5-AIR registration variant.
   Defer to follow-on if size pressure justifies the
   paper divergence.
3. **Prover wall / RAM measurements**: not in scope. The
   lb=3/lb=4 size wins come with proportional LDE
   inflation; prover cost trade-off is the maintainer's
   call.
4. **L2-over-L1 cascading effect**: this investigation
   measures L1 only. Smaller L1 ⇒ smaller in-circuit
   verifier work at L2 ⇒ smaller L2. The L2 reduction
   should be >L1's (verifier-circuit shrinks too) but is
   not measured here.
5. **MMCS cap > 0 compatibility with recursion verifier**:
   the existing comment claims unsupported; the
   combined-sweep test attempts it. Result will determine
   whether cap=3 is a valid lever.
6. **STIR/WHIR PCS swap (routes audit Path F)**: out of
   scope; substrate-replacement effort.

---

## 7. Cross-references (formerly §6)

- **Routes audit (parent doc):**
  `2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`
- **Poseidon2 removal (baseline establishment):**
  `2026-05-20_POSEIDON2_REMOVAL_SPEC.md`
- **FRI soundness (the ≥80 bar source):**
  `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` (S(−1))
- **CSA constraint-side soundness:**
  `2026-05-20_CSA_S7_AUDIT_SIGNOFF.md`
- **Test files (measurement infrastructure):**
  - `Plonky3-recursion/recursion/tests/test_l1_outer_cert_tip5_unified.rs`
    (P4 baseline; Tip5-unified vs Poseidon2 vs Tip5-out-4)
  - `Plonky3-recursion/recursion/tests/test_l1_size_reduction_sweep.rs`
    (broad sweep; all levers in isolation)
  - `Plonky3-recursion/recursion/tests/test_l1_size_reduction_combined.rs`
    (focused combinations; Pareto candidates)
- **README cryptographic assumptions:**
  `crates/ai-pow-zk/README.md` § "Cryptographic assumptions"
- **R1 / R1.1 discipline:** `~/.claude/CLAUDE.md`.
