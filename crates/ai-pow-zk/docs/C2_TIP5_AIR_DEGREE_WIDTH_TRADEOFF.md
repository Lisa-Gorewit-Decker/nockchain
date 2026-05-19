> _Created **2026-05-18** · last updated **2026-05-18** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# C2 — Tip5 AIR degree↔width tradeoff (split to degree 4) + relative performance

> **⚠ SUPERSEDED (2026-05-18).** This analysed the *lookup-free*
> `Tip5PermAir` (7604 cols, deg 3) and concluded "the real lever is
> the lookup table". That lever was then taken: `Tip5PermLookupAir`
> drops the main width to **886**. But it surfaced a new dominant
> factor this doc did not model — the **LogUp gadget's own
> constraint degree** (`1 + Σ tuple degrees`): the as-committed
> single 225-tuple interaction is degree ≈226 (FRI-infeasible). The
> feasible narrow form is the multi-interaction shared bus
> (degree ≈3–4, B=4, + O(#lookups·D) aux EF cols). The
> performance analysis is being **rewritten on that bus-form
> basis** (`C2_TIP5_CIRCUIT_AIR_DESIGN.md` §2c, L4). The §1–§6
> figures below are kept only as the lookup-free baseline.

> **Status:** DESIGN (2026-05-18). A study of widening the
> `tip5-circuit-air` AIR to cap constraint degree, with exact
> per-variant width and a prover-cost analysis. No code change yet —
> this is the design + the performance argument; implementation +
> benchmark is the follow-up ("then let's consider the relative
> performance"). Governed by R1: any variant is a *re-expression* of
> the C2.1-validated permutation and must re-pass the full native-
> equivalence + adversarial gate before it can replace the default.

## 0. TL;DR (the key, slightly counter-intuitive finding)

- The current AIR is **max constraint degree 3**, width **7604**
  (`tip5_perm_air_width()`), one row per permutation.
- Plonky3/ai-pow-zk FRI rule (verbatim, `circuit.rs`): a profile with
  `log_blowup = L` admits **quotient degree `< 2^L`**, i.e.
  **max constraint degree ≤ 2^L**:

  | max constraint degree | min `log_blowup` | blowup `B` | ai-pow-zk tier |
  |---|---|---|---|
  | ≤ 2 | 1 | 2 | `TEST` |
  | 3 or **4** | 2 | 4 | `TEST_PEARL` / `LB2` |
  | 5 … 8 | 3 | 8 | `PROD` |

  ⇒ **degree 3 and degree 4 are the *same* FRI tier (B=4).** "Splitting
  to degree 4" therefore changes the FRI blowup by **nothing**. Its
  *only* effect is that the higher budget lets us **delete staging
  columns** (carry x⁷ with one register instead of two, and inline the
  S-box output into the MDS+RC constraint), shrinking the trace.
- Net of the degree-4 redesign: **−196 columns (7604 → 7408, −2.6%)**,
  same B=4 tier, same soundness, same constraints' semantics ⇒ a
  **free ≈2.6 % prover-cost reduction**. Real, but small.
- The trace width is **dominated by an irreducible ~7168-column
  degree-2 boolean range-check core** (the byte/quotient bit
  decompositions of the split-and-lookup S-box). **No degree budget
  can shrink it** — only a *lookup argument* can (that is the C2.3
  LogUp/CTL form, a separate, soundness-critical residual). So degree,
  beyond reaching the B=4 tier, is a ~1–3 % lever; the big levers are
  the FRI profile sweep (M-P1) and the lookup-table range check (C2.3).
- Crossing FRI tiers (degree ≤ 2 → B=2, or natural degree 7 → B=8) is
  **not a free lunch**: at fixed 120-bit soundness ai-pow-zk pins
  `num_queries · log_blowup / 2 = 120`, so a smaller blowup means
  proportionally **more FRI queries**. That is exactly the ai-pow-zk
  FRI sweep tradeoff and must be *measured*, not assumed.

## 1. Current AIR (exact)

`Plonky3-recursion/tip5-circuit-air`, one row per permutation,
`max_constraint_degree() = Some(3)`. Per-round column group
(`ROUND_GROUP = 1084`):

| block | cols/round | constraint degree | notes |
|---|---|---|---|
| split bits `b,c,q` (4 lanes×8 bytes×(8+8+16)) | 1024 | 2 (boolean `x(x−1)`) | the **irreducible range-check core** |
| §4.6 inverse-or-zero `inv` | 4 | — | guard witness |
| `x2` (power lanes) | 12 | 2 (`x2−x·x`) | x⁷ stage 1 |
| `x3` (power lanes) | 12 | 2 (`x3−x2·x`) | x⁷ stage 2 |
| `A` (S-box output) | 16 | 1 (`A−recompose_c`) / 3 (`A−x3·x3·x`) | feeds MDS |
| `ROUT` (post-round state) | 16 | 1 | inter-round carrier |

`WIDTH = STATE_SIZE + NUM_ROUNDS·ROUND_GROUP = 16 + 7·1084 = 7604`.
Degree-3 constraints: the offset-Fermat cube `u³−1−257q−c`
(`u=b+1`), the x⁷ closer `A−x3·x3·x`, the §4.6 guard
`g·(prod−1)` and `(1−prod)·low`. Everything else is degree ≤ 2.

## 2. The degree-bounded family (precise constraint splits)

The S-box is, per the Tip5 paper (ePrint 2023/107) §2.2: split lanes
`S = ρ∘L⁸∘σ` (L = `(x+1)³−1 mod 257`, our lookup-free cube), power
lanes `T = x⁷`; §2.3 MDS; §2.4 RC; §2.1 ×7. Re-expressing for a
degree cap only *adds/removes intermediate witness columns*; the
permutation computed is **identical** (so the C2.0/C2.1 oracle and
the 315-vector + 4096-random native-equivalence KAT + §4.6
adversarial suite are unchanged and remain the soundness gate).

### 2a. Degree ≤ 4 — the requested target (FREE, same B=4 tier)

The budget rises 3→4; nothing must be split *down* (nothing exceeds
4). We *exploit* it to remove staging:

- **x⁷ with one register.** Keep `x2` (`x2 − x·x`, deg 2). Replace
  `x3` + closer with `S_pow = x2·x2·x2·x` (deg **4**) used directly.
  ⇒ delete the 12 `x3` cols/round (**−84** total).
- **Inline the S-box into MDS+RC.** Drop the explicit `A` columns;
  enforce `ROUT[i] − Σⱼ M[i][j]·Sboxⱼ − rc = 0` where `Sboxⱼ` is
  `recompose_c` (deg 1) for split lanes and `x2·x2·x2·x` (deg 4) for
  power lanes. The whole constraint is degree **4**. ⇒ delete the 16
  `A` cols/round (**−112** total).
- Cube `u³−1−257q−c` (deg 3) and §4.6 guard (deg 3): unchanged
  (already ≤ 4; no removable column).

`ROUND_GROUP = 1024 + 4 + 12(x2) + 16(ROUT) = 1056`;
`WIDTH = 16 + 7·1056 = 7408` (**−196, −2.6 %**). `max_constraint
_degree = Some(4)`. FRI tier **unchanged** (B=4, `log_blowup=2`,
`num_queries=120` for 120-bit). ⇒ prover cost ≈ ×(7408/7604) ≈
**−2.6 %**, zero soundness/FRI cost. This is the recommended
"degree-4" design — a small, free, monotone win.

### 2b. Degree ≤ 2 — the only true tier change (B=2), not free

Split every deg-3/4 to deg-2 with extra columns:

- cube: `u2=u·u`, `cu=u2·u` (2 cols/byte, deg-2 each), then
  `cu−1−257q−c` deg 1 → **+448**.
- x⁷: `x2,x4=x2·x2,x6=x4·x2`, `A=x6·x` (deg-2 each); +1 col/lane vs
  current → **+84**.
- §4.6: `prod=g·inv` as a column, `g·(prod−1)` & `(1−prod)·low`
  deg-2 → **+28**.

`WIDTH ≈ 7604 + 560 ≈ 8164` (**+7.4 %**); `max degree 2` →
**B=2 (`log_blowup=1`)**. At fixed 120-bit soundness this needs
`num_queries = 240` (vs 120). Commit/NTT term ∝ `W·B`:
`8164·2 / (7604·4) ≈ 0.54×` (≈ −46 %) **but** FRI query/opening
work ≈ **2×** (240 vs 120 paths). Net is the classic ai-pow-zk FRI
sweep balance — **measure, do not assume**.

### 2c. Natural degree 7 (no x⁷ staging) — B=8, generally worse here

Inline x⁷ unstaged (deg 7) into MDS+RC; keep only bits+inv+ROUT.
`WIDTH ≈ 16 + 7·(1024+4+16) = 7324` (−3.7 %) but **B=8
(`log_blowup=3`, `num_queries=80`)**: commit term ∝ `7324·8` vs
`7604·4` ≈ **1.93×** worse, only modestly fewer queries. For a
width-dominated trace this loses. (This is why the Tip5 paper keeps
x⁷ staged and why our default is not the "natural" degree-7 AIR.)

## 3. Why width barely moves with degree — the real lever

Across **every** variant the width stays ≈ 7.3 k–8.2 k because the
**1024 cols/round (7168 total) of `b/c/q` boolean range-check bits**
are intrinsic to a *lookup-free* byte decomposition and are degree-2
*regardless of the global degree cap*. A higher degree budget removes
only the few hundred *staging* columns (x3, A). The only way to
collapse the 7168-col core is to replace the bit decomposition with a
**256-row LogUp lookup table** for `L` (paper §4.1/§4.7) — which is
precisely the **C2.3 CTL/witness-bus form** (soundness-critical,
needs its own bus-form≡standalone≡native KAT). That single change
(≈ 7168 → ≈ few hundred cols) dwarfs any degree tuning. Degree
tuning and the FRI sweep are ≤10 % levers; the lookup-table range
check is the order-of-magnitude one.

## 4. Relative performance — model + recommendation

Plonky3 `TwoAdicFriPcs` prover, trace height `n` (= padded #perms),
width `W`, blowup `B`, quotient chunks `Q = max_deg − 1`,
`q = num_queries`:

- **Commit / LDE / NTT** ∝ `(W + Q)·B·n·log(B·n)` — falls with `W`
  and `B`.
- **FRI query / opening** ∝ `q · log(B·n)` Merkle paths over `W+Q`
  — *rises* with `q`; at fixed soundness `q = 240/B_log` so it rises
  as `B` falls.
- Verifier / proof size ∝ `q·log(B·n)` — same `B↔q` tension.

Conclusions:
1. **Degree 3 → 4 (§2a): unambiguous free −2.6 %** (same `B`, same
   `q`, fewer `W`). Recommended, low-risk (pure column removal).
2. **Degree → 2 (§2b): a `B↔q` trade**, identical in character to
   ai-pow-zk's existing FRI sweep (M-P1). Likely net-neutral for a
   width-dominated trace; only worth it if measurement shows the
   commit phase dominates at the target `n`.
3. **Degree budget is a ≤3 % lever overall.** The real prover wins
   are (a) the FRI profile (owned by M-P1's sweep), and (b) the
   C2.3 LogUp range-check (≈ −90 % width). Spend effort there.

## 5. Soundness invariance (R1)

Every variant computes the **same** 7-round Tip5 permutation; the
re-expression only re-buckets algebraic degree across witness
columns. Therefore the soundness gate is unchanged and **mandatory
for any variant before it may become the default**:

- `nockchain-math c2_kat` (L-table identity ∀b; 315-vector fixture ≡
  live `nockchain_math::tip5::permute`),
- `tip5-circuit-air`: `native_equiv_kat` (315-pt), `air_equals_native
  _spec_exhaustive_random` (4096 random + `check_constraints`),
  `tip5_spec_matches_fixture_permute`, `embedded_constants_match
  _fixture`, `adversarial_*` (incl. the §4.6 forgery vector), full
  `prove`→`verify`.

Implementation plan (additive, R1-safe): add the degree-4 variant as
a *parallel* AIR (e.g. `Tip5PermAirD4` / a const-generic degree
parameter), keep the validated degree-3 default, run the **entire
gate against the variant**, and only then consider switching the
default. No mutation of the validated artifact without full
re-validation.

## 6. Measurement plan ("then let's consider the relative performance")

A `criterion`/timed bench in `tip5-circuit-air` (or an ai-pow-zk
harness) over a fixed batch (e.g. 4096 perms, n=4096):

- variants: **deg-3 (current, B=4)**, **deg-4 (§2a, B=4)**,
  **deg-2 (§2b, B=2)**;
- per variant at the matching ai-pow-zk profile *and* a fixed
  120-bit-soundness profile (`LB2` 120q vs an `LB1` 240q);
- report: `prove` ms, `verify` ms, proof bytes, trace cells `W·B·n`,
  peak NTT size.

Expected: deg-4 ≈ deg-3 −2–3 % across the board; deg-2 a wash
(commit −≈46 % vs query ×≈2) → confirms degree is a minor lever and
focuses effort on the FRI sweep + the C2.3 lookup range-check.

## 7. Cross-references

`C2_TIP5_CIRCUIT_AIR_DESIGN.md` (§2b lookup-free arithmetization,
C2.3 residual = the LogUp/CTL form that actually collapses the
range-check width); `crates/ai-pow-zk/src/circuit.rs` (FRI profiles,
the `quotient degree < 2^log_blowup` rule, the
`num_queries·log_blowup/2` soundness law); `c2-tip5-circuit-air`
memory; Tip5 paper IACR ePrint 2023/107 §6 (the degree↔column
prover-time tradeoff is called out by the authors).
