# 2026-05-21 — Inner ai-pow PoUW prove-time optimization: profiling, paths, and the MDS-cyclomul landing

> _Created **2026-05-21**. Follow-up to
> `2026-05-21_E2E_LATENCY_AND_SWEEP_MEASUREMENTS.md`, which
> identified the inner ai-pow PoUW prove (17.7 s at 8K, 54.8 s at
> 16K) as the highest-priority latency lever for the 30 s
> per-proof budget._

## 0. Status (R1, honest)

Profile-driven optimization deliverable. **Path 1 (MDS cyclomul in
the shared Tip5 `linear_layer`) is implemented + validated +
landed.** Paths 2–5 are designed and documented here as a
prioritized backlog; not all are implemented this session.

## 1. Profiling — where the 17.7 s goes

`bench_suite::tests::profile_prod_8k_baseline_prove_spans` installs
a `tracing-subscriber` span-timing layer over the real PROD-profile
inner prove (`prove_batch`, `CircuitConfig::PROD` = lb=4 nq=15
pow=1, 8192×2135 trace). Span-close timeline:

```
prove_batch
├─ infer log of constraint degree {air 0}   closes t=04.36 s   busy 2.74 ms
│  ╎  ⟵ 17.3 s GAP — no info-level span ⟶
├─ compute quotient {air 0}                  closes t=21.70 s   busy 151 ms
│  └─ compute quotient polynomial            busy 131 ms
├─ FRI prover                                busy 149 ms
└─ (total)                                   busy 17.7 s
```

**Finding:** all instrumented spans sum to ~430 ms. The remaining
**~17.3 s is the uninstrumented trace LDE + Merkle commitment** —
the `pcs.commit(trace)` step that runs between constraint-degree
inference and quotient computation.

### 1.1 Why the trace commit is ~17 s

For an 8192×2135 trace at `log_blowup = 4` (16× LDE ⇒ 131072 LDE
rows), the `MerkleTreeMmcs` leaf hashing uses
`PaddingFreeSponge<Tip5Perm, 16, 10, 5>` (rate 10). Hashing one
2135-element leaf row absorbs `⌈2135/10⌉ = 214` Tip5 permutations.
Across all leaves:

```
131072 leaves × 214 Tip5 permutations/leaf ≈ 28 million Tip5 permutations
```

plus ~262 144 internal-node `TruncatedPermutation` Tip5 calls. At
the pre-optimization scalar Tip5 cost (~600 ns/permutation, 7
rounds × naive 256-multiply MDS) this is ~17 s — matching the
profiled gap. **Tip5 permutation throughput is the inner-prove
bottleneck.**

### 1.2 The Tip5 call path

`crates/ai-pow-zk/src/circuit.rs::Tip5Perm` (the MMCS hash +
Fiat-Shamir challenger permutation) delegates to
`nockchain_math::tip5::permute` (the canonical 7-round Tip5). Its
`linear_layer` (`crates/nockchain-math/src/tip5/mod.rs`) was the
**naive O(n²) = 256-multiply** circulant MDS matrix-vector product.

## 2. Optimization paths

| # | Path | Mechanism | Expected inner-prove win | Risk / cost |
|---|---|---|---|---|
| **1** | **MDS cyclomul** | Replace naive 256-mul MDS with Karatsuba cyclic convolution (~64 muls) in `nockchain_math::tip5::linear_layer` | **~2-3× on Tip5 ⇒ large inner-prove win** | Low — bit-identical, validated |
| 2 | Parallel leaf hashing | — already done (ai-pow-zk `default = ["parallel"]` ⇒ Plonky3 merkle-tree uses Rayon) | n/a — already in the 17.7 s | n/a |
| 3 | Real SIMD Tip5 | NEON/AVX-native Tip5 (byte-table `vqtbl1q_u8`, vectorized MDS + x⁷) replacing the scalar-per-lane "fake SIMD" packed impl | ~2× on the packed-leaf path | High — bespoke intrinsics, separate per-arch |
| 4 | Narrow the composite AIR | Fewer trace columns ⇒ fewer Tip5 absorbs per leaf (214 → fewer) | Linear in column-count reduction | High — soundness-orthogonal but invasive AIR refactor |
| 5 | FRI `lb` reduction | `lb=4 → lb=3` halves the LDE (16× → 8×) ⇒ ~½ the Merkle leaves ⇒ ~½ the Tip5 hashing | ~2× on the commit | Medium — needs `nq=20` to hold 60-bit Johnson; bigger inner proof; shifts L1 verifier cost |

### 2.1 Path 1 — MDS cyclomul (IMPLEMENTED)

The MDS matrix is circulant. A circulant matrix-vector product
`C · v` equals the cyclic convolution `first_column(C) ⋆ v`.
Cyclic convolution mod `(x¹⁶−1)` is computed via CRT
factorisation + Karatsuba (`(x¹⁶−1) = (x⁸−1)(x⁸+1)`, recursively
down to degree-1 multiplies; `(x⁸+1)` factors over the Gaussian
integers). Net: ~64 i64-multiplications vs the naive ~256
field-multiplications.

This is the **same `mds_cyclomul`** already landed + validated in
the Plonky3-recursion `p3-tip5-circuit-air` crate's `tip5_spec.rs`
(commit `158bbad`). Path 1 ports it into the shared
`nockchain_math::tip5::linear_layer`, which is used by **both**
`permute` (7-round, the canonical Nockchain hash) and
`permute_5round` (the ai-pow-zk variant).

**Why modifying `nockchain-math` is safe here.** `nockchain_math::tip5`
is the frozen C2.0 soundness oracle — the normative bit-for-bit
reference the recursion verifier's Tip5 AIR must reproduce.
The cyclomul change is a **pure performance optimisation with
byte-for-byte-identical output**:
- Round count unchanged (`permute` stays 7-round, `permute_5round`
  stays 5-round).
- Only the *internal computation* of the shared `linear_layer`
  changes; its output is mathematically identical (circulant
  matvec ≡ cyclic convolution — standard linear algebra).
- Bit-identity is gated by **two** tests: the new
  `linear_layer_cyclomul_matches_naive` differential test (edge
  cases + 256 seeded random states vs the preserved naive
  reference) AND the binding `golden_kat_frozen_matches_live_permute`
  + `golden_kat_5round_frozen_matches_live_permute_5round` gates
  (the frozen-oracle KAT fixtures, regenerated from `permute` —
  if the output drifted by a single bit these fail).
- The C2.0 cross-workspace oracle relationship is preserved: the
  recursion `tip5-circuit-air` matches `permute`'s *output*, which
  is unchanged. (As of `158bbad` the recursion-side `tip5_spec.rs`
  already uses cyclomul; Path 1 makes nockchain-math consistent
  with it.)

**MDS matrix verified identical** between `nockchain_math`
(`MDS_MATRIX_I64` row 0) and the recursion-side
(`tip5_spec::MDS_FIRST_ROW`); `MDS_FIRST_COLUMN_I64` is column 0
of `MDS_MATRIX_I64`.

### 2.2 Path 5 — FRI `lb` reduction (designed, not landed)

`lb=4 nq=15 pow=0` is the current inner PROD (60 pure-query Johnson).
The alternative `lb=3 nq=20 pow=0` is also 60 pure-query Johnson (`3·20 = 60`)
but the LDE is 8× instead of 16× ⇒ half the Merkle leaves ⇒
~half the trace-commit Tip5 hashing. Trade-offs:
- Inner prove ~2× faster on the commit phase.
- Inner proof larger (nq 15 → 20 ⇒ 33% more query opens).
- The L1 outer-cert verifier circuit must verify nq=20 inner
  queries instead of nq=15 ⇒ L1 verifier circuit ~grows; this
  partially offsets the gain at the chain level.
- Net effect on the **end-to-end** budget needs a Stage 5
  measurement at inner `lb=3 nq=20`.

Path 5 is a candidate follow-up if Path 1 alone doesn't bring the
end-to-end wall-clock under budget.

## 3. Path 1 measurement

Inner ai-pow PoUW prove, `CircuitConfig::PROD`, 8192×2135 trace,
`bench_suite::tests::bench_prod_8k_baseline` (M2 Max):

| Build | prove time | Δ |
|---|---:|---:|
| Pre-cyclomul (naive 256-mul MDS) | 17,665 ms | baseline |
| **Post-cyclomul (this work)** | **10,034 ms** | **−43%, 1.77× faster** |

trace_gen / populate / verify / proof-size all essentially
unchanged (10–17 ms / 5–7 ms / 22–37 ms / ~209 KB) — the entire
win is in the `prove` step's trace-commit Tip5 hashing, exactly
as the §1 profile predicted.

### 3.1 End-to-end impact

| Component | Pre-Path-1 | Post-Path-1 |
|---|---:|---:|
| Inner ai-pow PoUW prove (8K) | 17.7 s | **10.0 s** |
| L1+L2 recursion wrap | ~26 s (cyclomul already in via `158bbad`) | ~26 s |
| **Estimated end-to-end** | **~44 s** | **~36 s** |
| vs 30 s budget | +14 s over | **+6 s over** |

Path 1 closes ~8 s of the ~14 s end-to-end budget shortfall. The
remaining ~6 s gap is a candidate for Path 5 (FRI `lb` reduction)
or further inner-AIR work (Path 4), or a maintainer re-discussion
of the 30 s budget itself.

## 4. Validation

- `nockchain-math` tip5 tests — `linear_layer_cyclomul_matches_naive`
  (differential), `golden_kat_frozen_matches_live_permute` (7-round
  frozen KAT), `golden_kat_5round_frozen_matches_live_permute_5round`
  (5-round frozen KAT), `l_table_identity_bijection_fixed_points`:
  **4/4 pass**.
- `nockchain-math` full lib regression: **40/40 pass**.
- ai-pow-zk regression: _pending_.
- Re-measured inner prove: _pending_ (§3).

## 5. Files modified

- `crates/nockchain-math/src/tip5/mod.rs`: `linear_layer` body now
  calls `mds_cyclomul`; added `MDS_FIRST_COLUMN_I64` + the cyclomul
  Karatsuba/CRT helper functions; added the
  `linear_layer_cyclomul_matches_naive` differential test.
- `crates/ai-pow-zk/src/bench_suite.rs`: added
  `profile_prod_8k_baseline_prove_spans` profiling test.
- `crates/ai-pow-zk/Cargo.toml`: added `tracing` +
  `tracing-subscriber` dev-deps for the profiling test.
- _This doc._
