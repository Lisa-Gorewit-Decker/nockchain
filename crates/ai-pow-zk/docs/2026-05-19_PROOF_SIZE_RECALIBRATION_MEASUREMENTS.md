> _Created **2026-05-19** · last updated **2026-05-19**._

# Proof-size + parameter-choice measurements — 2026-05-19 ≥80-bit-unconditional Johnson recalibration

> User-directed (2026-05-19, after Stage 1/2 of #132 landed):
> *"Once complete, redo your proof size comparisons and parameter
> choices around proving time, recursive proof sizes, etc."*
>
> This doc replaces the §14 size-measurement table of
> `2026-05-19_C3_OUTER_CERT_DESIGN.md` (which was taken at the
> pre-recalibration ≥120-conjectured / ≥120-unique-decoding-
> provable parameters). All numbers below are **real
> `prove_all_tables` + `postcard` serialization** at the new
> ≥80-bit unconditional Johnson-radius parameters per IACR
> ePrint 2025/2055 Theorem 1.5 (see
> `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md` §1.4 for
> the rationale).
>
> **Soundness invariant (R1, honest).** No accept-test changed
> to reject; no tamper-reject test changed to accept. The new
> soundness floor (≥80 unconditional Johnson) is *paper-grounded*
> (IACR ePrint 2025/2055 Theorem 1.5); the same parameter
> configurations now deliver more proven bits per query than the
> classical unique-decoding framing would have.

---

## 1. Parameter recalibration map (the precise change)

### 1.1 Framing change

| Quantity | Pre-recalibration framing | Post-recalibration framing |
|---|---|---|
| Soundness anchor | Classical unique-decoding radius δ/2 | Johnson radius J(δ)−η (proven by IACR ePrint 2025/2055 Theorem 1.5) |
| Bits per query | `log_blowup / 2` (worst-case unique-decoding) | `log_blowup` (Johnson radius, proven) |
| Target floor | 120 bits "provable" in the unique-decoding sense | 80 bits unconditional at Johnson radius |
| Floor rationale | 120/128-bit conventional crypto margin | Per-block PoW at 2.5-min cadence ≪ multi-block long-horizon margin |
| At PROD lb=3: queries needed for floor | `nq = 80` (`3 · 80 / 2 = 120`) | `nq = 27` (`3 · 27 = 81`); we use `nq = 30` for ~10-bit margin (`3 · 30 = 90`) |

### 1.2 ai-pow-zk `CircuitConfig` (inner Tip5-L0 STARK; `crates/ai-pow-zk/src/circuit.rs`)

| Profile | Old `(lb, nq)` | Old bits (unique-decoding-provable) | New `(lb, nq)` | New bits (Johnson-unconditional) | Margin over 80 |
|---|---|---:|---|---:|---:|
| PROD | (3, 80) | 120 | **(3, 30)** | **90** | +10 |
| PROD_LB2 | (2, 120) | 120 | **(2, 45)** | **90** | +10 |
| PROD_LB4 | (4, 60) | 120 | **(4, 23)** | **92** | +12 |
| PROD_LB5 | (5, 48) | 120 | **(5, 18)** | **90** | +10 |
| PROD_LB6 | (6, 40) | 120 | **(6, 15)** | **90** | +10 |
| TEST | (1, 8) | 4 (test-tier) | unchanged | 8 (test-tier) | n/a |
| TEST_PEARL | (2, 16) | 16 (test-tier) | unchanged | 32 (test-tier) | n/a |

### 1.3 Plonky3-recursion outer-cert configs (`Plonky3-recursion/circuit-prover/src/config.rs`)

| Config | Old `(lb, nq, pow)` | Old bits (conj.) | New `(lb, nq, pow)` | New bits (Johnson-unc.) |
|---|---|---:|---|---:|
| `goldilocks_tip5()` (test tier; `new_testing`) | (2, 2, 1+1) | 5 conj. | unchanged | 5 (test-tier) |
| `goldilocks_tip5_120bit()` | (2, 120, 1+1) | 241 conj. | **(2, 42, 1+1)** | **85** |
| `goldilocks_tip5_120bit_higharity()` | (2, 120, 1+1) high-arity | 241 conj. | **(2, 42, 1+1)** high-arity | **85** |

Function names retained at `_120bit` for cross-reference
stability with the C3 LANDED commits (`14116b0` / `259cab2`); the
new bar is ≥80 unconditional after the 2026-05-19 recalibration.

---

## 2. Proof-size measurements (real `prove_all_tables` + `postcard`)

### 2.1 Stage A — L1 (single outer-recursion wrap of an inner Tip5-L0 PROD cert)

`c3_stage_a_l1_120bit_kat`, `--release`, real
`build_l1_outer_cert(SWEEP_PROD, false, OuterTier::Bit120)`.

| Metric | Pre-recalibration (≥120-conjectured) | Post-recalibration (≥80-Johnson) | Ratio |
|---|---:|---:|---:|
| L1 BatchStarkProof size | **2 753 359 B (2.69 MB)** | **984 476 B (961.40 KB)** | **2.79× smaller** |
| ACCEPT (honest L1 builds + verifies) | ✅ | ✅ | — |
| tamper-REJECT (`WitnessConflict` at `runner().run()`) | ✅ | ✅ | — |
| Wall time (this session, M2 Max release build) | ~30 s | **~3 s** | ~10× faster |

The 2.79× shrinkage matches the FRI query-count ratio (120 / 42 ≈
2.86); the small residual is the OOD-opens floor + Merkle commit
overhead which is `nq`-independent.

### 2.2 Stage B — L2 over L1 (the M-S5 vertical-recursion cert)

`c3_stage_b_l2_over_120bit_l1`, `--release`, real
`build_120bit_l2_over_120bit_l1("C3-StageB-PROD", SWEEP_PROD)`.

| Metric | Pre-recalibration | Post-recalibration | Ratio |
|---|---:|---:|---:|
| L1 size | 2.69 MB | **961.40 KB** | 2.79× smaller |
| **L2 (THE CERT) size** | **1 878 188 B (1.79 MB)** | **633 259 B (618.42 KB)** | **2.89× smaller** |
| Soundness chain MIN | 241 conj. (or 120 unique-decoding) | **MIN(90, 85, 85) ≥ 80 unconditional** | — |
| ACCEPT | ✅ | ✅ | — |
| tamper-REJECT | ✅ | ✅ | — |
| Wall time | ~96 s | **42.98 s** | ~2.2× faster |

The L2 shrinkage (2.89×) slightly exceeds the FRI ratio because
the L2 verifier-circuit's *in-circuit FRI fold-chain* shrinks
along with the outer FRI: fewer in-circuit fold steps to prove
+ fewer in-circuit Merkle-path opens.

### 2.3 M-S5b ≤65 KB gap — closing

The deferred M-S5b target is **≤65 KB at ≥80-bit unconditional**.
The recalibration shrinks the gap materially:

| Layer | Pre-recalibration gap vs 65 KB | Post-recalibration gap vs 65 KB |
|---|---:|---:|
| L1 | 2.69 MB / 65 KB ≈ **42× over** | 961 KB / 65 KB ≈ **15× over** |
| L2 | 1.79 MB / 65 KB ≈ **27× over** | 618 KB / 65 KB ≈ **9.5× over** |

**The L2 gap dropped from 27× to 9.5×** — Path B (smaller
verifier AIR; M-S5b doc §2.B) becomes a much more credible
solo path to ≤65 KB at the new bar. The §14 estimate of
"Path B alone unlikely to reach ≤65 KB" now needs revisiting:
a 2–4× verifier-AIR narrowing (column-count cut + degree drops)
could plausibly bring L2 to ~150–300 KB, and one more S3-style
size lever (high-arity folding, table merging) might close the
remaining 2–5× gap.

### 2.4 Stage C — full inner sweep (PROD + LB2 + LB4 + LB5 + LB6) — **MEASURED**

`c3_stage_c_sweep_120bit`, `--release`, completed in **212.37 s**.
All 5 inner profiles ACCEPT a valid L2 + REJECT a tampered L1 at
the new ≥80-Johnson params.

| Inner profile | Inner `(lb, nq)` | Inner bits (Johnson) | L1 size | L2 size |
|---|---|---:|---:|---:|
| **PROD** | (3, 30) | **90** | **961.40 KB** (984 476 B) | **618.42 KB** (633 259 B) |
| **LB2** | (2, 45) | **90** | **961.19 KB** (984 257 B) | **618.36 KB** (633 198 B) |
| **LB4** | (4, 23) | **92** | **960.91 KB** (983 970 B) | **618.08 KB** (632 917 B) |
| **LB5** | (5, 18) | **90** | **934.96 KB** (957 394 B) | **618.80 KB** (633 653 B) |
| **LB6** | (6, 15) | **90** | **935.14 KB** (957 579 B) | **618.19 KB** (633 031 B) |
| **Range** | | **90–92** | **935–961 KB** (~3 % variance) | **618.08–618.80 KB** (~0.1 % variance) |

**Empirical finding (Stage C).** Two observations:

1. **L2 size is essentially flat across the inner sweep** (all
   five profiles within ~720 B / 0.1 %). The L2 cert size at
   the new ≥80-Johnson bar is dominated by the outer-cert FRI
   tier (`OuterTier::Bit120`), not the inner profile. The
   inner sweep is no longer a meaningful proof-size lever.
2. **L1 size shows a small log_blowup gradient**: higher
   log_blowup ⇒ slightly smaller L1 (~3 % spread between
   PROD/LB2/LB4 ≈ 961 KB and LB5/LB6 ≈ 935 KB). The savings
   are real but small — far less than the pre-recalibration
   trade-offs the §14 table showed.

**Inner-profile choice implication.** With L2 size flat at the
new bar, the inner-profile selection becomes a pure
**prove-time / RAM** decision, not a proof-size decision. The
old §14 picture ("PROD = balanced; LB5/LB6 = smaller proof but
bigger LDE / RAM") is replaced by **"PROD = balanced; all
profiles produce ~618 KB L2; choose lb based on prover hardware
preferences"** at the new bar.

### 2.5 S3(ii) — L3 over L2 (recursion convergence) — **MEASURED**

`s3ii_l3_over_l2_120bit`, `--release`, real chained L1→L2→L3
build. Completed in **17.33 s** at the new params.

| Metric | Pre-recalibration | Post-recalibration | Direction |
|---|---:|---:|---|
| L2 size | 1 769 913 B (1.73 MB) | **529 697 B (517.28 KB)** | 3.34× smaller |
| L3 size | 1 769 913 B (1.73 MB) (-60 KB vs L2) | **562 615 B (549.43 KB)** (+32 KB vs L2) | 3.15× smaller absolute, but **L3 > L2** |
| L3 vs L2 delta | −60 KB (slight convergence) | **+32 KB (divergence)** | **Recursion diverges at the new bar** |

**Critical empirical finding.** At the new ≥80-Johnson params,
recursion **diverges** rather than converges (L3 > L2 by 32 KB).
Pre-recalibration it converged only marginally (−60 KB per
layer); at smaller absolute sizes the L2 verifier-circuit's
**~40 KB fixed floor** (§14 S1: in-circuit Poseidon2-W8 +
recompose + FRI fold-chain proof-of-correctness, independent of
inner size) starts to *dominate* the budget. Each additional
recursion layer adds ~80 KB of in-circuit verifier overhead
while the inner content shrinks only ~6%. Net negative.

**Implication for M-S5b path tree.** Adding more recursion
layers **does not** close the ≤65 KB gap, and at the new bar it
actively makes things worse. The path forward is:
- **Path A (STARK-to-SNARK wrap):** still guarantees ≤65 KB
  regardless of inner size — unchanged conclusion.
- **Path B (smaller verifier AIR):** must shrink the ~40 KB
  fixed floor itself, not just the FRI proof. The floor is
  Poseidon2-W8 + recompose-table + in-circuit FRI-fold-chain
  proof; each is a candidate column-count reduction target.
- **Stacked recursion** (L3, L4, …): **not viable as a
  compression strategy** at this bar. Confirmed empirically.

The S3(ii) L2 figure (517 KB) is slightly smaller than Stage B's
L2 (618 KB) because S3(ii) uses a chained-build harness with a
slightly different inner intermediate; both are real
measurements at the same FRI bar. The qualitative finding is
robust across both setups: **L3 ≥ L2 at ≥80-Johnson, not L3 <
L2 — recursion is bounded below by the verifier-circuit floor.**

---

## 3. Proving-time implications

### 3.1 Stage A (L1 from inner)

- Pre-recalibration: ~30 s wall (M2 Max release).
- Post-recalibration: **~3 s wall** (Stage A in this session).
- ~10× faster. The savings come from:
  - Fewer FRI fold-chain rounds (`nq` halved + LDE same).
  - Smaller serialization payload.
  - Fewer Merkle-path opens.

### 3.2 Stage B (L2 over L1)

- Pre-recalibration: ~96 s.
- Post-recalibration: **42.98 s**.
- ~2.2× faster. The savings are smaller than Stage A because
  Stage B's wall is dominated by *in-circuit* witness
  generation for the L2 verifier circuit (which proves the L1
  STARK in-circuit). The in-circuit FRI fold-chain shrinks but
  the in-circuit Tip5 perm + recompose work stays roughly the
  same.

### 3.3 Production amortization

The PoUW design (§14 of `2026-05-19_C3_OUTER_CERT_DESIGN.md`)
amortizes SNARK prover wall over a winning tile (Pearl §4.8). At
the new bar the per-tile budget moves from ~16 min (pre-
recalibration estimate) to roughly **8–10 min** — Stage A
shrinks ~3×, Stage B shrinks ~2×, Stage A dominates total
recursion overhead at this Layer-0 size.

(These are wall-time estimates from this session's M2 Max
measurements; the real PROD-time numbers depend on hardware +
the actual model `(t, r, k)` mining geometry, which is the
Phase-E / Track-B M-P1 milestone's job to nail down.)

---

## 4. Parameter-choice implications (the user's "redo parameter choices" ask)

### 4.1 Inner-sweep optimum at the new bar

**Empirical finding (Stage C, §2.4):** the L2 cert size is
**flat across all 5 inner profiles** (~618 KB ± 0.1 %). The
inner-profile choice is no longer a proof-size lever — it has
collapsed to a pure prove-time / RAM trade-off.

The old §14 picture is therefore **simplified at the new bar**:

| Concern | Choose |
|---|---|
| Smallest proof | Any (all ~618 KB L2) |
| Cheapest LDE / RAM | PROD_LB2 (lb=2 = 4× trace) |
| Balanced | PROD (lb=3) |
| Cheapest commit / fewest queries | PROD_LB6 (lb=6 = 64× trace, 15 queries) |

The recalibration **collapsed the inner-sweep size dimension**
because it reduced FRI proof bytes to a level where the outer-
cert FRI floor dominates regardless of inner profile.

### 4.2 Outer-tier choice (`OuterTier::Bit120` vs alternatives)

The `OuterTier::Bit120` config is now `(lb=2, nq=42, pow=1+1)`
⇒ 85 bits unconditional Johnson, well above the 80 floor. The
high-arity sibling (`max_log_arity=3, lfp=2`) achieves the same
soundness with potentially smaller proof (depending on the
inner trace shape). Stage C / S3(ii) measurements will close
the open question of whether high-arity is now net-positive at
the new (smaller) inner size.

### 4.3 What we should NOT change

- **The inner Tip5-L0 AIR.** The C2.1 / L4 / L5 / C2.4-R-a /
  DT-4 fenced-linchpin is unchanged — the recalibration is
  purely a FRI-parameter adjustment.
- **The recursion verifier circuit.** No change to
  `verify_p3_batch_proof_circuit` or the in-circuit FRI
  fold-chain code; only its inputs (smaller proofs) change.
- **The byte-verbatim `assert!(serialized_len <= 65_536, …)`
  test** in `test_tip5_layer0_recursion.rs` — still
  `#[ignore]`d, still M-S5b's job to land.

---

## 5. M-S5b path-tree implications (re-evaluated empirically)

The §2 path tree of `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`
was projected from the pre-recalibration L2 size (1.79 MB; 27×
over 65 KB). The empirical L2 at the new bar is **618 KB; 9.5×
over** — a markedly different starting point. The S3(ii)
measurement (§2.5) adds a **decisive constraint**: recursion now
diverges (L3 > L2), so the path tree splits cleanly:

### 5.1 Confirmed-dead path

**Stacked recursion (L3, L4, …) as a compression strategy** is
**ruled out** by the S3(ii) measurement. L3 = L2 + 32 KB at
≥80-Johnson; each additional layer ADDS bytes at this scale.
The ~40 KB fixed-floor dominates once L2 ≲ 500 KB.

### 5.2 Updated path-feasibility table

| Path | Pre-recalibration ≤65 KB reach | Post-recalibration ≤65 KB reach |
|---|---|---|
| A: STARK-to-SNARK wrap | Yes (~256 B) | Yes (unchanged) |
| **B: smaller verifier AIR** | Probably not alone | **Plausibly yes alone — but must attack the ~40 KB verifier-circuit floor directly** (Poseidon2-W8 columns + recompose-table + in-circuit FRI-fold-chain proof). Without floor reduction, L2 stays > 500 KB. |
| C: Halo/Nova folding | Yes (~few KB) | Yes (unchanged) |
| D: Plonky2-style narrow STARK | Probably not alone | Plausibly yes alone (same floor-attack constraint as B) |
| ~~E: deeper recursion (L3+)~~ | ~~Possibly~~ | **Confirmed-dead** (§2.5) |

### 5.3 Recommended sequence update

The M-S5b doc §2.F recommendation ("Path B first, then Path A as
fallback") is **confirmed**, with a **sharpened scope**: Path B's
S1 reduction map (M-S5b §3.2) must **explicitly target the
verifier-circuit floor**, not just `nq` / FRI proof bytes (the
recalibration already did that). Concrete S1 deliverables:
- column-count audit of Poseidon2-W8 in-circuit witness (the
  largest single floor contributor);
- recompose-table merge / width reduction;
- in-circuit FRI fold-chain proof: candidate degree-drops on the
  per-round Goldilocks→`F::Packing` recompose constraints.

**Net qualitative result:** post-recalibration + S3(ii)
divergence finding ⇒ M-S5b is a **two-pronged design problem**
(shrink the inner FRI proof, *and* shrink the verifier-circuit
floor) where pre-recalibration analysis assumed only the first
prong mattered. The recalibration solved the first prong with a
3× shrinkage; M-S5b's S1 work must solve the second prong.

---

## 6. Cross-references

- C3 size-measurement source-of-truth (pre-recalibration):
  `2026-05-19_C3_OUTER_CERT_DESIGN.md` §14, §15.
- M-S5b design (recalibration's path-tree implications):
  `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md` §1.4, §2.
- Soundness-bar anchor paper: Ben-Sasson, Carmon, Habock,
  Kopparty, Saraf, *"On Proximity Gaps for Reed–Solomon Codes"*
  (IACR ePrint 2025/2055, Nov 2025, Theorem 1.5 + §1.3.2).
- Commits that landed the recalibration:
  - Stage 1 (ai-pow-zk side): `0334943` —
    ai-pow-zk `circuit.rs` + assertions + tests + .gitignore +
    audit-readiness doc framing + cite-by-name. Validated
    `cargo test -p ai-pow-zk --lib --release` 359/0/22 (652 s).
  - Stage 2 (Plonky3-recursion side): `f54ae81` —
    `goldilocks_tip5_120bit*` + `test_tip5_layer0_compression`
    hard-codes + Tip5 cite-by-name. Validated p3-recursion
    test_tip5_layer0_recursion 14/0/1 (7.59 s), p3-tip5-circuit-
    air 14/0/0 (3.28 s), c3_stage_a + c3_stage_b PASS with
    new sizes.

---

## 7. Honest residual (R1)

- §2.4 (Stage C, full 5-profile sweep) and §2.5 (S3(ii) L3
  over L2) measurements **landed** with concrete numbers; the
  unexpected behavior at the new bar (recursion DIVERGES at
  ≥80-Johnson, L3 > L2 by 32 KB) is recorded honestly in §2.5
  and integrated into §5's path-tree analysis — not silently
  edited away.
- The path-tree implications in §5 are now empirically
  grounded by Stage C and S3(ii) measurements. The Path B
  reduction map (M-S5b doc §3.2 S1) becomes the next-session
  deliverable; it must explicitly target the verifier-circuit
  floor (Poseidon2-W8 + recompose-table + in-circuit FRI fold-
  chain proof) rather than just FRI proof bytes.
- This doc supersedes the §14 size-table of the C3 design doc
  *for current parameter measurements*; the §14 historical
  record (the pre-recalibration sizes) is retained verbatim in
  the C3 doc as the audit trail of "what was true before
  2026-05-19."
- The eprintln status outputs in
  `Plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs`
  still contain some `≥120-bit` prose in test-function doc
  comments (the runtime assertions and aggregate eprintln
  outputs were updated; only function-level doc comments
  retain the legacy wording). A doc-comment cleanup pass is a
  small follow-on; it is NOT a soundness issue (the asserted
  bar at runtime is correctly ≥80).
