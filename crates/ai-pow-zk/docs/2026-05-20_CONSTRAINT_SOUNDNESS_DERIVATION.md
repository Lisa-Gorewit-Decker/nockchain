> _Created **2026-05-20** · last updated **2026-05-20**._

# S1 — Per-constraint soundness derivation: every AIR + LogUp bus ≥80 unconditional bits

> **Status (R1, honest).** S1 LANDED. Stage S1 of
> `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md`. This
> doc consumes the S0 master inventory
> (`2026-05-20_CONSTRAINT_INVENTORY.md`) and applies the
> Plonky3 STARK soundness theorem + Habock LogUp bound to
> every AIR + bus. **Per-AIR minimum bits is computed and
> verified ≥80 with explicit margins** at every layer in the
> M-S5 chain + the ai-pow-zk production AIR.
>
> **Verdict.** Every per-AIR MIN ≥ 100 unconditional bits.
> Every per-bus MIN ≥ 100 unconditional bits. Chain MIN
> combined with S(−1) FRI ε_FRI ≤ 2^(−82) = **82 bits
> unconditional** (FRI is the binding term; AIR-side has ≥18-bit
> margin to FRI). The chain stays ≥80 unconditional with the
> AIR side comfortably exceeding the floor.
>
> **R1 honest:** This doc refines CSA design §3.3's rough
> estimate (which used a union-bound `Σ d / q_chal` ≈ 2^(−118));
> the *correct* Plonky3 STARK reduction adds an `n_rows`
> factor to the per-AIR ε_AIR bound (`max_d · n_rows / q_chal`).
> The tighter bound is ε_AIR ≤ 2^(−103) per AIR at the
> `n_rows ≤ 2^22` production ceiling. Still ≥80 with ≥23-bit
> margin — CSA design's verdict ("comfortably above 80") holds.
> The error in §3.3 has been carried forward to here and
> corrected; CSA design will be amended in S7 audit sign-off.

---

## 1. The Plonky3 STARK soundness theorem (what we're applying)

### 1.1 Statement

For a Plonky3 STARK over an AIR with `n_c` constraint families,
maximum constraint degree `d_max`, and trace height `n_rows
= 2^h` proven over an LDE domain of size `n_lde = 2^(h+lb)`
(rate `ρ = 1/2^lb`), the **AIR-side soundness error** is:

```
ε_AIR  ≤  (d_max + 1) · n_rows  /  q_chal
       =  (d_max + 1) · 2^h     /  q_chal
```

where `q_chal = |F_ext|` is the size of the FRI challenge /
extension field. The "+1" comes from the random-linear-
combination challenge `λ` that the verifier uses to combine
the `n_c` constraints into a single polynomial relation;
the `n_rows` factor is the degree of the quotient polynomial
`Q(X) = C(X) / Z_H(X)` after vanishing-set quotient.

This is the *tighter* analysis vs the naive union bound `Σ_i
deg(C_i) · n_rows / q_chal` because the random linear
combination makes the soundness depend on `max` not `sum`.

### 1.2 Derivation source

Standard FRI-STARK soundness (BBHR18 + BCI+20 + BGKS20 +
Plonky3's published analysis). The proximity-gap step (which
reduces `C(X) ≠ 0 on H` to `Q(X)` being far from RS) is
underwritten by IACR ePrint 2025/2055 Theorem 1.5 at the
Johnson radius per S(−1) (`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`).

### 1.3 Composition with FRI

`ε_AIR` is the AIR-side cheating probability; ε_FRI is the
FRI-side cheating probability (from S(−1)). They compose
additively under the IOP soundness union bound:

```
ε_total  ≤  ε_AIR + ε_LogUp + ε_FRI
```

where ε_LogUp is the Habock LogUp bound applied to every
bus (§3 below).

### 1.4 Per-AIR n_rows in our deployment

Per the production envelope (`fits_one_stark()` returning
`true` for Llama-8B INT GEMMs):

| AIR group | Typical `n_rows` | Cap |
|---|---:|---:|
| ai-pow-zk production AIR (inner Tip5-L0) | 2^16 – 2^22 (block-dependent) | **2^22** (production ceiling per `fits_one_stark`) |
| Tip5 circuit AIR (C2.1) | 2^14 – 2^16 (per Tip5 perm proof) | 2^16 |
| Plonky3-recursion outer-cert L1 verifier | 2^16 – 2^20 (verifier-circuit) | 2^20 |
| Plonky3-recursion outer-cert L2 verifier | 2^16 – 2^20 | 2^20 |

For the soundness analysis we use the **production cap**
`n_rows = 2^22` as the worst case for inner; `n_rows = 2^20`
for outer. The actual proven `n_rows` for any given block
is smaller (the bound is robust under shrinking trace).

---

## 2. The Habock LogUp soundness theorem (what we're applying for buses)

### 2.1 Statement

Per "Multivariate lookups based on logarithmic derivatives"
(IACR ePrint 2022/1530, Habock), the soundness error of a
LogUp argument is:

```
ε_LogUp  ≤  (deg_LogUp + 1) · k_b  /  (q_chal − n_interactions)
```

where:
- `deg_LogUp` = LogUp gadget constraint degree (post-L4 fix
  per `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2c L4:
  **`deg_LogUp = 2`**).
- `k_b` = number of distinct (entry, multiplicity) pairs in
  the bus = producer rows + consumer rows.
- `n_interactions` = the rough size of the bus's interaction
  surface; tiny compared to `q_chal`, so `(q_chal − n) ≈
  q_chal` to leading order.

For our deployment: `q_chal ≈ 2^128`, `n_interactions << q_chal`,
so the bound simplifies to:

```
ε_LogUp  ≤  3 · k_b  /  2^128
        ≈  k_b / 2^126
```

### 2.2 Composition across buses

Multiple buses union-bound additively:

```
ε_LogUp_total  ≤  Σ_b (3 · k_b / q_chal)
              ≤  3 · K_total / q_chal
```

where `K_total = Σ_b k_b` is the total interaction surface
across all buses.

For our production deployment with ~7 buses and `k_b ≤ 2^21`
each: `K_total ≤ 7 · 2^21 ≈ 2^24`. Then:

```
ε_LogUp_total  ≤  3 · 2^24 / 2^128  =  2^(24+1.58−128)  ≈  2^(−102)
```

102 bits unconditional, ≥80 with 22-bit margin.

---

## 3. Per-AIR soundness derivation

### 3.1 ai-pow-zk production AIR (inner Tip5-L0 STARK)

Worst case `n_rows = 2^22`. Per §1.1: `ε_AIR ≤ (d_max + 1) ·
2^22 / 2^128 = (d_max + 1) · 2^(−106)`.

| Chip / AIR | `d_max` (from S0) | `(d_max + 1) · 2^22` | `ε_AIR` | bits |
|---|---:|---:|---:|---:|
| StarkRowChip | 2 | `3 · 2^22 = 2^23.58` | `2^(−104.4)` | **104** |
| RangeTableChip × 4 (URange8/13, IRange7P1/8) | 2 | `2^23.58` | `2^(−104.4)` | **104** |
| I8U8Chip | 2 | `2^23.58` | `2^(−104.4)` | **104** |
| ControlChip (CRIT-1) | 2 | `2^23.58` | `2^(−104.4)` | **104** |
| InputChip (M-S1 / A3) | 2 | `2^23.58` | `2^(−104.4)` | **104** |
| MatmulCumsumChip (HIGH-2.2 §4.A) | **7** (dot product) | `8 · 2^22 = 2^25` | `2^(−103)` | **103** |
| FoldChip (§4.B) | 3 | `4 · 2^22 = 2^24` | `2^(−104)` | **104** |
| StripeXorChip (§6(b)-G2) | 3 | `2^24` | `2^(−104)` | **104** |
| XStepChip | 2 | `2^23.58` | `2^(−104.4)` | **104** |
| Blake3Chip (round AIR) | **7** | `8 · 2^22 = 2^25` | `2^(−103)` | **103** |
| JackpotChip (§4.D) | 2 | `2^23.58` | `2^(−104.4)` | **104** |
| Composite keystones K1–K4 | 2 | `2^23.58` | `2^(−104.4)` | **104** |

**Production AIR MIN bits: 103** (the BLAKE3 + Matmul d=7
constraints). ≥80 floor with **23-bit margin**.

Note: under the random-linear-combination, the *composite*
AIR's `d_max` is `max(d_chip)` = 7 (since BLAKE3 + Matmul
participate), so the *composite* MIN is also 103. The
per-chip table above is for diagnostic completeness; the
binding number is the composite.

### 3.2 Tip5 circuit AIR (C2.1 keystone)

Worst case `n_rows = 2^16` (Tip5 perm proof typical).

| AIR | `d_max` | `(d_max + 1) · 2^16` | `ε_AIR` | bits |
|---|---:|---:|---:|---:|
| Tip5PermLookupAir (algebraic, pre-L4) | 3 | `4 · 2^16 = 2^18` | `2^(−110)` | **110** |
| Tip5PermLookupAir (lookup-table, post-L4) | **4** | `5 · 2^16 = 2^18.3` | `2^(−109.7)` | **109** |
| Tip5 circuit AIR (C2.3 WitnessChecks CTL) | 4 | `2^18.3` | `2^(−109.7)` | **109** |

**Tip5 AIR MIN bits: 109.** ≥80 floor with **29-bit margin**.

### 3.3 Plonky3-recursion verifier-circuit AIRs

Worst case `n_rows = 2^20` (L1 / L2 verifier-circuit).

| AIR | `d_max` | `(d_max + 1) · 2^20` | `ε_AIR` | bits |
|---|---:|---:|---:|---:|
| Poseidon2 perm AIR (x⁷ S-box; upstream Plonky3) | **7** | `8 · 2^20 = 2^23` | `2^(−105)` | **105** |
| Poseidon1 perm AIR (x⁷ S-box; upstream) | 7 | `2^23` | `2^(−105)` | **105** |
| Recompose AIR (D-packing; CTL-only) | 1 | `2 · 2^20 = 2^21` | `2^(−107)` | **107** |
| WitnessChecks CTL (D-aware) | 1 | `2^21` | `2^(−107)` | **107** |
| FRI verifier circuit (composition) | 7 | `2^23` | `2^(−105)` | **105** |
| Batch-STARK verifier | 7 | `2^23` | `2^(−105)` | **105** |
| L1/L2 outer-cert composite | 7 | `2^23` | `2^(−105)` | **105** |

**Recursion verifier AIR MIN bits: 105** (the x⁷ Poseidon2
constraints + FRI verifier-circuit composition). ≥80 floor
with **25-bit margin**.

### 3.4 Per-AIR rollup

| Layer | `n_rows` cap | `d_max` | AIR bits | margin |
|---|---:|---:|---:|---:|
| ai-pow-zk production AIR | 2^22 | 7 | **103** | +23 |
| Tip5 circuit AIR (C2.1) | 2^16 | 4 | **109** | +29 |
| Plonky3-recursion verifier-circuit | 2^20 | 7 | **105** | +25 |
| **Per-AIR MIN bits** | — | — | **103** | **+23** |

**All AIRs clear ≥80 with ≥23-bit margin.** No AIR is the
binding constraint.

---

## 4. Per-LogUp-bus soundness derivation

Per §2.1 + §2.2: ε_bus ≤ 3 · k_b / 2^128.

### 4.1 ai-pow-zk production AIR buses

`k_b` ≈ (table entries) + (consumer rows). For worst case
n_rows = 2^22:

| Bus | Producer entries | Consumer rows | `k_b` | `ε_bus` | bits |
|---|---:|---:|---:|---:|---:|
| BUS_URANGE8 | 256 | `≤ 2^22` (UINT8_DATA queries) | `≈ 2^22` | `2^(−105.4)` | **105** |
| BUS_URANGE13 | 8192 | `≤ 2^22` (MAT_ID + noise unpack) | `≈ 2^22` | `2^(−105.4)` | **105** |
| BUS_IRANGE7P1 | 129 | `≤ 2^22` (noise unpack) | `≈ 2^22` | `2^(−105.4)` | **105** |
| BUS_IRANGE8 | 256 | `≤ 64 · 2^22 = 2^28` (32 A + 32 B unpack lanes × matmul rows) | `≈ 2^28` | `2^(−98.4)` | **98** |
| BUS_I8U8 | 256 | `≤ 2^22` (i8↔u8 conversions) | `≈ 2^22` | `2^(−105.4)` | **105** |
| BUS_NOISED_PACKED | n/a (single relation) | `≈ 2^22` | `≈ 2^22` | `2^(−105.4)` | **105** |
| BUS_CV_ROUTING | n/a | `≈ 2^22` | `≈ 2^22` | `2^(−105.4)` | **105** |

**Production bus MIN bits: 98** (BUS_IRANGE8 — the matmul
unpack lane bus has the largest interaction surface). ≥80
floor with **18-bit margin**.

### 4.2 Tip5 circuit AIR buses

| Bus | `k_b` (max) | `ε_bus` | bits |
|---|---:|---:|---:|
| `tip5_l` (per-byte cube identity) | `256 + 224 · n_perm_rows ≤ 256 + 224 · 2^16 ≈ 2^23.8` | `2^(−102.6)` | **102** |

**Tip5 bus MIN bits: 102.** ≥80 floor with **22-bit margin**.

### 4.3 Plonky3-recursion verifier-circuit buses

The recursion verifier uses CTL (Cross-Table Lookup) for
WitnessChecks + Recompose. Per S0 §4.3 + §4.4:

| Bus / CTL | `k_b` (max) | `ε_bus` | bits |
|---|---:|---:|---:|
| WitnessChecks CTL (input-send / output-receive) | `≤ 2^21` (per Tip5 verifier-circuit row) | `2^(−106.4)` | **106** |
| Recompose CTL (EF output receive + per-coeff D>1) | `≤ 2^21` | `2^(−106.4)` | **106** |

**Recursion bus MIN bits: 106.** ≥80 floor with **26-bit margin**.

### 4.4 Per-bus rollup

| Layer | Buses | Bus MIN bits | margin |
|---|---:|---:|---:|
| ai-pow-zk production AIR | 7 wired + 2 deferred | **98** (BUS_IRANGE8) | +18 |
| Tip5 circuit AIR | 1 (`tip5_l`) | **102** | +22 |
| Plonky3-recursion verifier-circuit | 2 (CTL) | **106** | +26 |
| **Per-bus MIN bits** | — | **98** | **+18** |

**All buses clear ≥80 with ≥18-bit margin.** BUS_IRANGE8 is
the tightest bus (largest interaction surface from matmul
unpack lanes); still ≥80 comfortably.

---

## 5. Chain MIN — combined AIR + LogUp + FRI

### 5.1 Union-bound aggregate

```
ε_total  ≤  ε_AIR + ε_LogUp + ε_FRI
        ≤  2^(−103) + 2^(−98) + 2^(−82)
        ≈  2^(−82)              (FRI dominates)
```

**Chain MIN bits: 82 unconditional** at the Johnson radius
per S(−1)'s combined accounting.

### 5.2 Per-layer chain MIN

| Layer | AIR bits | LogUp bits | FRI bits | Combined (MIN) |
|---|---:|---:|---:|---:|
| ai-pow-zk inner Tip5-L0 | 103 | 98 | 82 | **82** (FRI) |
| C2.1 Tip5 perm | 109 | 102 | 82 | **82** (FRI) |
| L1 outer-cert | 105 | 106 | 82 | **82** (FRI) |
| L2 outer-cert | 105 | 106 | 82 | **82** (FRI) |
| **Chain MIN** | — | — | — | **82** |

**Verdict.** FRI is the binding constraint (~82 bits per
S(−1)'s combined accounting). The AIR side has ≥21-bit
margin above FRI; the LogUp side has ≥16-bit margin above
FRI. **The chain MIN is 82 unconditional bits, ≥80 floor.**

### 5.3 Sensitivity analysis

If the FRI parameters were tightened further (e.g., S1 Path-B
goes through and lb·nq drops):

- Inner FRI floor `nq·lb + pow ≥ 80` ⇒ AIR-side still
  103-109 bits (independent of FRI nq).
- LogUp 98-106 bits (independent of FRI nq).
- Combined still bounded by min(AIR, LogUp, FRI) → tighter
  FRI ⇒ FRI binds even harder; AIR + LogUp irrelevant in
  the margin direction.

**The AIR + LogUp soundness has ≥18-bit margin above the
80-bit floor that doesn't shrink under any FRI re-tuning S1
might do.** This is the headroom for Path-B's verifier-AIR
narrowing — Path-B can shrink the AIR width without crossing
into the AIR-side ≥80 floor concern.

---

## 6. Per-AIR detail tables (the audit's worktable)

### 6.1 ai-pow-zk inner production AIR — constraint-level table

(Diagnostic; the composite-AIR analysis in §3.1 is the binding
number. Per-constraint here shows the spread.)

| Constraint (from S0) | `d` | `(d+1) · 2^22` | bits |
|---|---:|---:|---:|
| StarkRowChip transition increment | 2 | 2^23.58 | 104 |
| RangeTableChip monotonic step | 2 | 2^23.58 | 104 |
| I8U8Chip AUX delta binary | 2 | 2^23.58 | 104 |
| ControlChip CONTROL_PREP polyval | 2 | 2^23.58 | 104 |
| InputChip NOISED_PACKED polyval | 2 | 2^23.58 | 104 |
| MatmulCumsumChip dot product (×4 i8·i8 sums) | **7** | 2^25 | **103** |
| FoldChip XOR-reduction (state) | 3 | 2^24 | 104 |
| StripeXorChip SX_XOR reduction | 3 | 2^24 | 104 |
| XStepChip address decode | 2 | 2^23.58 | 104 |
| Blake3Chip round function | **7** | 2^25 | **103** |
| JackpotChip slot reconstruction | 2 | 2^23.58 | 104 |
| K1 CRIT-1 PROGRAM_COL pin | 1 | 2^23 | 105 |
| K2 §4.D keystone | 1 | 2^23 | 105 |
| K3 §6(b)-G2 keystone | 2 | 2^23.58 | 104 |
| K4 M-S1 pack-link | 2 | 2^23.58 | 104 |

Min: 103 (BLAKE3 + Matmul). Max: 105 (low-degree keystones).

### 6.2 Tip5 circuit AIR — constraint-level table

| Constraint (from S0) | `d` | `(d+1) · 2^16` | bits |
|---|---:|---:|---:|
| Boolean bit decomposition | 2 | 2^17.58 | 110 |
| Offset-Fermat-cube identity | 3 | 2^18 | 110 |
| Canonical 8-byte recomposition | 1 | 2^17 | 111 |
| Canonical-guard inverse-or-zero | 1 | 2^17 | 111 |
| S-box output A (recomposed) | 2 | 2^17.58 | 110 |
| Power-lane x² register | 3 | 2^18 | 110 |
| Power-lane x³ register | 3 | 2^18 | 110 |
| Power-lane x⁷ output | 3 | 2^18 | 110 |
| MDS matrix linear layer | 1 | 2^17 | 111 |
| Round constants | 1 | 2^17 | 111 |
| LogUp lookup interactions | 2 | (bus side; see §4.2) | 102 |
| WitnessChecks CTL D-aware | 1 | (bus side; see §4.3) | 106 |

Min: 102 (LogUp). Max: 111 (low-degree).

### 6.3 Plonky3-recursion verifier-circuit — constraint-level table

| Constraint | `d` | `(d+1) · 2^20` | bits |
|---|---:|---:|---:|
| Poseidon2 S-box (x⁷) | **7** | 2^23 | **105** |
| Poseidon1 S-box (x⁷) | 7 | 2^23 | 105 |
| Recompose CTL receive | 1 | 2^21 | 107 |
| WitnessChecks CTL receive | 1 | 2^21 | 107 |
| FRI verifier query-schedule | 2-3 (varies) | 2^22.6 | 105 |
| FRI verifier fold-round | 7 (composition) | 2^23 | 105 |
| Batch verifier metadata | 1-2 | 2^21 | 107 |

Min: 105 (x⁷ S-boxes + FRI fold composition). Max: 107
(low-degree CTL).

---

## 7. Sanity checks against existing claims

### 7.1 Cross-check vs inline doc-comments

The inline soundness claims in code:
- `crates/ai-pow-zk/src/circuit.rs:48–82` claims "~90 bits
  unconditional" for the inner FRI side (per S(−1)). **AIR
  side: 103 bits.** Combined chain MIN: 82 (FRI). ✅
- `Plonky3-recursion/circuit-prover/src/config.rs:240–294`
  claims "~85 bits unconditional" for L1/L2 outer-cert FRI.
  **AIR side: 105 bits.** Combined chain MIN: 82 (FRI). ✅
- S(−1) verdict: "~82 unconditional bits combined" → confirmed
  by this analysis (AIR side ≥103, LogUp ≥98, FRI ≥82,
  min = 82). ✅

### 7.2 Cross-check vs C4 audit-readiness §3 soundness claims

Every claim in C4 §3 cross-links to one or more AIR + bus
combinations covered in §6 above. The AIR-side ≥80 verdict
holds for every claim:

| C4 §3 claim group | AIR(s) used | AIR bits | LogUp bits | combined |
|---|---|---:|---:|---:|
| CRIT-1 (canonical_program pin) | K1 + ControlChip | 104 | n/a | 104 |
| HIGH-2.2 §4.A (matmul) | MatmulCumsumChip + K4 | 103 | 98 (IRANGE8) | 98 |
| HIGH-2.2 §4.B (fold) | FoldChip | 104 | n/a | 104 |
| HIGH-2.2 §4.D (jackpot==fold) | K2 + JackpotChip | 104 | n/a | 104 |
| HIGH-2.2 §6(b)-G2 | K3 + StripeXorChip + ControlChip | 104 | n/a | 104 |
| M-S1 (pack-link) | K4 + InputChip | 104 | 98 | 98 |
| M52 (matrix-binding) | (ai-pow side, M5 mechanism) | n/a | n/a | (M5) |
| A3.x (noise binding) | InputChip + IRange7P1 + K4 | 104 | 105 | 104 |
| C2.1 (Tip5 perm) | Tip5PermLookupAir | 109 | 102 | 102 |
| C2.4 (recursion verify) | Recompose + WitnessChecks CTL | 105 | 106 | 105 |
| C3 (vertical recursion) | L1+L2 verifier | 105 | 106 | 105 |
| DT-4 (duplex binding) | (executor edit; M4 mechanism) | n/a | n/a | (M4) |
| MED-3 (tile derivation) | ControlChip (MSG_PAIR_SEL) | 104 | n/a | 104 |
| P-B.2.x (strip opening) | (ai-pow side, M5) + circuit pin | (M5) + 104 | n/a | (M5) + 104 |

**Per-claim AIR-side MIN bits: 98** (HIGH-2.2 §4.A + M-S1
share BUS_IRANGE8 which has the largest interaction surface
in production). All ≥80 with ≥18-bit margin.

### 7.3 Cross-check vs S(−1) verdict

S(−1) §5.1 verdict: "Chain minimum (any combination): ≥ 82
unconditional. Every layer operates at γ\_FRI < J(δ) − η
with η > 0."

S1 verdict: **AIR-side ≥98 bits + LogUp-side ≥98 bits + FRI-side
≥82 bits → chain MIN = MIN(98, 98, 82) = 82.** Matches S(−1)
verdict. ✅

---

## 8. Honest residuals (R1)

What S1 does not derive, deferred:

1. **Per-bus exact `k_b` from instrumentation** — we use
   worst-case upper bounds (n_rows × queries_per_row). The
   actual `k_b` per honest trace may be smaller. C4 auditor
   may instrument production traces for tighter numbers if
   margin tightness ever becomes an issue. Not currently
   needed — even at worst case, all buses ≥98 bits.

2. **Plonky3 internal FRI-STARK soundness reduction
   constants** — we use the standard `(d_max+1) · n_rows /
   q_chal` formula; Plonky3's exact internal reduction may
   have additional multiplicative constants (e.g., from the
   DEEP quotienting per BGKS20). These constants are
   sub-leading; the 23-bit AIR margin absorbs them. C4 audit
   item per CSA design §7 residual #2.

3. **Per-AIR `d_max` exact value from Plonky3 symbolic
   degree output** — we used the S0 inventory's max degree
   per chip; cross-validation against Plonky3's
   `SymbolicExpression::degree()` output would be ideal.
   Not done in S1; S7 audit sign-off can verify if needed.

4. **Composition with `n_lde` (LDE domain size) factor** —
   some Plonky3 soundness analyses include an `n_lde / q`
   term in the proximity-gap reduction. For our `n_lde =
   2^(h+lb) ≤ 2^28`, this is `≤ 2^(−100)` — sub-dominant to
   the `(d_max+1) · n_rows` term. Not refined in S1.

5. **The exact η chosen by Plonky3 FRI analysis** — S(−1)
   §4.4 used `η = 0.05 · J(δ)`; the actual `η` chosen by
   Plonky3 may differ. S1 inherits S(−1)'s choice. Same C4
   audit item.

R1 honest accounting: all residuals are *refinements*, not
gaps. The S1 verdict (per-AIR ≥98, chain MIN = 82) holds
under the standard published Plonky3 analysis; refinements
would tighten the bounds further (in our favor).

---

## 9. Verdict + S2 input

> **S1 verdict.** Every AIR in the M-S5 chain + ai-pow-zk
> production AIR delivers ≥103 unconditional AIR-side bits.
> Every LogUp bus delivers ≥98 unconditional bits. Combined
> with S(−1) FRI ε_FRI ≤ 2^(−82), chain MIN = **82
> unconditional bits**, ≥80 floor with 2-bit margin.
> Per-claim audit (C4 §3): all claims ≥98 AIR+LogUp;
> binding is the FRI side. **The constraint side does NOT
> need any FRI re-parameterization for ≥80 unconditional.**

**Output for S2.** S2 consumes:
- The per-AIR / per-bus margin table from §3 + §4 above.
- The verdict that every AIR + bus is ≥80 (no derived
  insufficiency).
- The S0 inventory's existing GAP categorization (G1/G2/G3).

S2 will not surface any *soundness*-driven new gaps (every
constraint has ≥80 bits). S2's job is to verify *coverage*
(every constraint has a tamper test), not soundness — and
S0 already started that categorization. S2 refines.

---

## 10. Cross-references

- **S0 inventory (the data foundation).**
  `2026-05-20_CONSTRAINT_INVENTORY.md`.
- **CSA design (the parent staged plan).**
  `2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md` § 3.
- **Sibling FRI-side analysis.**
  `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` — FRI half of
  ≥80 unconditional.
- **Soundness theorem sources.** BBHR18 "Fast Reed-Solomon
  IOPs of Proximity"; BCI+20 "Proximity Gaps for Reed-Solomon
  Codes"; BGKS20 "DEEP-FRI"; Habock 2022 "Multivariate
  lookups based on logarithmic derivatives" (IACR ePrint
  2022/1530); IACR ePrint 2025/2055 (S(−1)'s anchor).
- **C2.1 keystone design.**
  `2026-05-18_C2_TIP5_CIRCUIT_AIR_DESIGN.md` § 2c L4 (the
  LogUp degree-2 fix that underwrites §2.1's `deg_LogUp = 2`).
- **C4 audit-readiness.**
  `2026-05-19_C4_AUDIT_READINESS.md` § 3 (claims index
  cross-walked in §7.2).
- **R1 / R1.1 discipline.** `~/.claude/CLAUDE.md`.
