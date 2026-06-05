# `ai-pow-zk`

EXPERIMENTAL — a Plonky3 STARK and recursive-certificate stack for the
[`ai-pow`](../ai-pow/) tiling matmul puzzle. The production-facing role is to
build the canonical recursive AI-PoW certificate for Nockchain block
submission. The plain `MatmulProof` remains a miner diagnostic / pre-ZKP
target-hit check; it is not the persisted block artifact.

## Cryptographic assumptions (the load-bearing primitives)

> **This is the AUTHORITATIVE list of cryptographic primitives the
> ai-pow-zk soundness rests on.** Nothing else is allowed in the
> AIR or the recursive proving stack. If you see a primitive
> outside this list (e.g., Poseidon2, BLAKE3 inside the SNARK
> circuit at the wrong layer, KZG, pairing-based curves), stop and
> consult the maintainer — it is either a bug or a milestone
> in-flight that hasn't updated this README. Last updated 2026-05-20
> (M-S5b S1.B Poseidon2-removal P5+P6 landing).

### Hash functions

The recursive certificate/proving stack uses Tip5, split by role:

- **Recursive proving Tip5** — the 5-round, paper-spec variant
  (`nockchain_math::tip5::permute_5round`, also exposed through
  `p3-tip5-circuit-air::Tip5Perm`). This is the only Tip5 variant
  used by ai-pow-zk's recursive certificate stack: the recursively
  wrapped inner STARK MMCS / Fiat-Shamir path and the outer recursive
  verifier circuit via `config::goldilocks_tip5_60bit()`.
- **Canonical Nockchain Tip5** — the separate 7-round variant
  (`nockchain_math::tip5::permute`). This remains Nockchain's
  non-recursive hash path and is not selected by the recursive
  certificate stack.

- **BLAKE3** (outside the SNARK; in the ai-pow puzzle's plain
  data path). Used by `ai-pow` for the matrix commitment
  (`HASH_A`, `HASH_B`), the strip-opening Merkle paths, and the
  Jackpot hash on the mineable unit. **BLAKE3 is NOT in the SNARK
  arithmetic circuit** — it appears as an AIR (`Blake3Chip` in
  `crates/ai-pow-zk/src/chips/blake3/`) that proves the BLAKE3
  computation matched the public input commitment. The
  out-of-circuit BLAKE3 is used by the plain miner; the in-circuit
  Blake3Chip is the prover-side AIR for it.

#### Tip5 soundness

- **Spec**: Tip5 paper (Szepieniec, Lemmens, Sauer, Threadbare,
  Al Kindi, "The Tip5 Hash Function for Recursive STARKs",
  **IACR ePrint 2023/107**). The paper specifies N=5 rounds.
- **Recursive proving choice**: **N=5 rounds** — the paper-spec
  round count, used only for ai-pow-zk recursive certificate
  proving.
- **Canonical Nockchain choice**: **N=7 rounds** — 2 rounds above the
  paper's spec for non-recursive Nockchain hashing.
- **Third-party cryptanalysis**: "Opening the Blackbox" (Liu,
  Koschatko, Grassi, Yan, Chen, Banik, Meier, **IACR ePrint
  2024/1900**). Practical SFS collision on 3-round Tip5 at 2^41.2;
  full collision on 3-round at 2^121.1. No attack reaches 4-round
  Tip5 or above.
- **Recursive-proving safety margin**: **2 rounds above broken**
  (5 − 3), matching the Tip5 paper's post-OtB margin.
- **Canonical Nockchain safety margin**: **4 rounds above broken**
  (7 − 3).
- **Sponge collision security**: `min(capacity/2, output)` =
  `min(6×64/2, 5×64)` = `min(192, 320)` = **192 bits**. Well
  above the ≥80-bit floor.

#### What's NOT in the recursive proving stack

- **No Poseidon2 (any variant: W8, W16, W24, Fused).** The
  outer-cert flipped to Tip5 in P5 of M-S5b S1.B (2026-05-20)
  per maintainer directive "I'm not willing to use Poseidon2".
  Poseidon2 remains in:
  - `crates/plonky3-recursion/circuit-prover/src/config.rs::goldilocks()`
    — the GENERAL-PURPOSE Goldilocks STARK config; NOT used by
    ai-pow-zk's recursive proving. Kept for non-recursive
    test cases.
  - `crates/plonky3-recursion/circuit-prover/src/batch_stark_prover/poseidon2.rs`
    — Poseidon2 NPO prover impls; available but unregistered in
    the ai-pow-zk batch-STARK. The ai-pow-zk path registers only
    `Tip5Preprocessor` + `RecomposePreprocessor`.
  - `p3_test_utils::goldilocks_params` (test-utils) — used by
    one legacy measurement test (`test_tip5_layer0_compression.rs`)
    that retains Poseidon2 only for historical-baseline
    comparability. NOT a production code path; documented residual
    pending follow-on cleanup.
- **No Rescue, Rescue-Prime, Reinforced Concrete, Anemoi,
  Griffin, Monolith, MiMC, Marvellous, Tip4, Tip4',** or other
  arithmetization-oriented hashes. Tip5 is the sole recursive
  proving choice.
- **No SHA-2, SHA-3, Keccak inside the SNARK.** BLAKE3 is used
  out-of-SNARK (matrix commit + strip openings) and via the
  in-circuit `Blake3Chip` AIR (mirroring Pearl's spec).
- **No pairing-friendly curves (BN254, BLS12-381, etc.).** No
  pairing-based PCS. No KZG. No Groth16/Plonk SNARK wrap
  currently. Path A SNARK-wrap (per
  `docs/2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md` §3.1) is
  a future option but NOT landed.
- **No Halo/Nova/Sangria-style accumulation schemes.** No Pasta
  curves. No R1CS.
- **No Plonky2.** We are Plonky3-based throughout.

### FRI soundness bounds

**2026-05-21 anchored-between Johnson policy (maintainer FINAL):**
production targets ≥60-bit Johnson floor (was ≥80), anchored
between the paper's known-insecure CYCLE-SUM ceiling at γ ≥ LDR
(~22 bits at n ≤ 2^22, IACR ePrint 2025/2055 §1.4.5 + Thm 1.17)
and the prior conservative 80-bit offline-cryptographic floor.
Justified by the **2.5-min block-cadence threat model** (PoW
forgery is time-bounded, so the 80-bit margin is unnecessary;
the 60-bit Johnson-proven floor with ~38-bit margin over the
known-insecure ceiling is "reasonable and optimistic"). See
[`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](docs/2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md)
2026-05-21 addendum for the full policy + paper-end-points
derivation, and
[`2026-05-21_WHIR_PROTOTYPE_RESULTS.md`](docs/2026-05-21_WHIR_PROTOTYPE_RESULTS.md)
for the policy trail (including why the Plonky3 `CapacityBound`
heuristic is **not** adopted).

- **Provable bound**: **≥60 bits unconditional at the Johnson
  radius** (paper-proven, anchored on Ben-Sasson, Carmon, Habock,
  Kopparty, Saraf, "On Proximity Gaps for Reed–Solomon Codes"
  (**IACR ePrint 2025/2055**, Nov 2025) Theorem 1.5 + §1.3.2).
- **Formula**: `unconditional_bits ≈ log_blowup · num_queries +
  query_proof_of_work_bits` for the FRI query phase. Commit PoW is a
  batching-challenge grind and is not counted toward this floor.
- **Per-layer (LANDED FRI configurations, 2026-05-21
  anchored-between)**:
  - Inner Tip5-L0 PROD: `lb=4, nq=15, pow=1+1` ⇒ **62 bits**
    unconditional Johnson (`crates/ai-pow-zk/src/circuit.rs::CircuitConfig::PROD`).
  - Outer-cert L1 (`goldilocks_tip5_60bit`): **production FRI
    parameters as of 2026-06-03 (anchored-between)** stack every
    soundness-neutral compression lever — `lb=4, nq=9, mla=3,
    lfp=2, cap=5, query_pow=24, d=2` ⇒ **60 bits** unconditional
    Johnson (`crates/plonky3-recursion/circuit-prover/src/config.rs`).
    Historical context: prior `lb=4 nq=20 mla=3 lfp=2 cap=3
    pow=1+1 d=5` ⇒ **82 bits** pre-anchored-between.
    Pre-2026-05-20 baseline (`lb=2 nq=42 mla=1 lfp=0 cap=0`) was
    85 bits, ~1011 KB L1.
    **Measured at production-faithful params (2026-06-03,
    `prod_recursion_measure 15`):** canonical fixed-int bincode L1 =
    **200.6 KiB** in **28.69 s** for the outer certificate stage
    (legacy postcard for the same proof was **225.8 KiB**), down from
    **215.7 KiB** at the prior `nq=10, query_pow=20` L1 point and
    **328.8 KiB** at the earlier `nq=15, query_pow=1` L1 point.
    A direct Pearl/Plonky2-style Merkle path-compression model over this
    same q=9/cap=5 proof shape saved only **1.2 KiB** on average (best
    sampled **6.8 KiB**), leaving an estimated fixed-int floor of
    **~199.4 KiB**; this route is not a path to the **≤100 KiB** target.
    **2026-06-04 terminal backend checkpoint:** the production terminal
    FRI profile is pure-query 60-bit (`log_blowup=4`, `num_queries=15`,
    `query_pow_bits=0`; no PoW bits counted toward soundness). The
    current production compact recursive certificate measures **87,059
    bytes (85.0 KiB)** with prove **4.471 s** and verify **3.454 s** in
    `terminal_production_certificate_measures_real_tip5_l0_verifier_circuit`.
    The FRI-native NPO residual-zero+recompose+value-bridge candidate
    verifies at the same pure 60-bit profile and measures **99,647 bytes
    (97.3 KiB)**, prove **9.216 s**, verify **0.677 s**. This clears
    the ≤100 KiB production target for this terminal-backend candidate.
    Trade-off: `lb=4` ⇒ 16× LDE (vs pre-2026-05-20 4×) ⇒ ~4×
    prover memory; 5-round Tip5 dropped prover time ~57% (22 min
    → 9.5 min). **The ai-pow-zk-specific 5-round Tip5 (paper-spec
    per IACR 2023/107 §2.4 N=5; canonical Nockchain 7-round
    `permute` UNCHANGED) was the single biggest proof-size lever
    — bigger than Tier B (−46%), Phase 0 (additional −1%), Path B
    B2 (~0%), and the 2026-05-21 anchored-between reanchor
    (additional −24%) combined.**
    **For ≤100 KB** (2026-05-21 maintainer-relaxed target, was ≤65 KB):
    in-substrate floor is now ~293 KB L1 / ~329 KB L2 (vs ≤100 KB
    target = ~2.93× over after the 2026-05-21 Angle-A Tip5 A-column
    elimination, was 3.07× over at 307 KB pre-Angle-A). Combinable
    in-substrate levers (WHIR @ Johnson, higher-lb outer with the
    new 25 s L1+L2 latency headroom, further Tip5 AIR refactor)
    may close part of the gap; a Path A SNARK wrap remains the
    most likely final lever. See `docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`
    2026-05-21 addendum for the target-relaxation rationale.
    See [`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](docs/2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md)
    § 4 + § 5.
- **γ < J(δ)−η**: every layer operates strictly inside the
  Johnson radius (no list-decoding-regime attacks per paper §8).
  Per-layer J(δ) ∈ {0.5, 0.646, 0.75, 0.823, 0.875} across the
  inner sweep; outer-cert J(δ) at the production rate (`lb=4`,
  ρ=1/16) = **1 − √(1/16) = 0.75** (wider Johnson radius than
  the pre-2026-05-20 `lb=2` ρ=1/4 → J(δ)=0.5; more headroom).
- **AIR-side soundness** (Plonky3 STARK reduction + Habock LogUp):
  - Per-AIR Schwartz–Zippel: `(d_max+1) · n_rows / q_chal` ≥98
    unconditional bits per AIR at production parameters.
  - Per-LogUp-bus: `3 · k_b / q_chal` ≥98 bits.
  - **Combined chain MIN** (2026-06-03 anchored-between):
    **60 bits unconditional Johnson** (= MIN(inner 62, L1 60);
    FRI is the binding term; AIR + LogUp have ≥36-bit
    margin over FRI). Historical pre-anchored-between chain
    MIN was 82 bits.

Full derivations: see
- [`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](docs/2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md) (FRI side, S(−1))
- [`2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md`](docs/2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md) (CSA S1; AIR + LogUp side)
- [`2026-05-20_CSA_S7_AUDIT_SIGNOFF.md`](docs/2026-05-20_CSA_S7_AUDIT_SIGNOFF.md) (chain MIN sign-off)

### Field stack

- **Base field**: **Goldilocks** (`2^64 − 2^32 + 1`; `p3_goldilocks::Goldilocks`).
  The single base field used throughout — inner STARK, outer-cert,
  recursion verifier circuit.
- **FRI challenge / extension field**: **`BinomialExtensionField<Goldilocks, 2>`**
  (≈ `2^128`). D=2 across the M-S5 chain.
- **No alternative fields.** No KoalaBear, BabyBear, M31, Mersenne,
  or other primes. (These exist in `crates/plonky3-recursion/circuit-prover/src/config.rs`
  for upstream Plonky3 compatibility — `baby_bear()`, `koala_bear()`
  builders — but are NOT used by ai-pow-zk.)

### Commitment scheme

- **PCS**: `TwoAdicFriPcs<Goldilocks, _, _, _>` (univariate FRI;
  upstream Plonky3 `p3-fri`).
- **MMCS**: `MerkleTreeMmcs<F::Packing, F::Packing, Tip5Sponge,
  Tip5Compress, 2, DIGEST_ELEMS=5>` (Tip5-based, packed-Goldilocks
  for SIMD; cap height 0 at outer-cert per recursion-verifier
  requirements; cap height 3 inner).
- **Challenger**: `DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>`
  (Fiat-Shamir over Tip5).
- **No KZG.** No vector commitments other than the Tip5-MMCS.

### How to find every cryptographic primitive in code

Grep these patterns in `crates/ai-pow-zk/src/` and
`crates/plonky3-recursion/`:

| Primitive | Where it lives | Grep pattern |
|---|---|---|
| Tip5 (the only in-SNARK hash) | `Tip5Perm`, `Tip5Sponge`, `Tip5Compress`, `Tip5Goldilocks`, `Tip5Config`, `Tip5Preprocessor`, `Tip5PermLookupAir` | `Tip5\b` |
| BLAKE3 (out-of-SNARK + Blake3Chip AIR) | `crates/ai-pow/src/commit.rs`, `crates/ai-pow/src/blake3_tree.rs`, `crates/ai-pow-zk/src/chips/blake3/` | `blake3\b\|BLAKE3\|Blake3` |
| Goldilocks field | `p3_goldilocks::Goldilocks` | `Goldilocks` |
| FRI | `TwoAdicFriPcs`, `FriParameters` | `Fri\b\|FRI` |
| MMCS | `MerkleTreeMmcs`, `ExtensionMmcs` | `Mmcs\b` |
| **Poseidon2** (FORBIDDEN in ai-pow-zk recursive proving) | `goldilocks()` (general STARK only); `batch_stark_prover/poseidon2.rs` (NPO; not registered by ai-pow-zk); `test_tip5_layer0_compression.rs` (legacy measurement only) | `Poseidon2\|poseidon2` |

If you ever see a primitive outside this table being **introduced** into the SNARK arithmetic circuit (`composite_full_air*.rs` or any AIR registered via `BatchStarkProver::register_*`), **that is a soundness change requiring maintainer review and a CSA AIR-inventory update** (see [`2026-05-20_CONSTRAINT_INVENTORY.md`](docs/2026-05-20_CONSTRAINT_INVENTORY.md)).



**Status:** M10.1c is the canonical pipeline. A full composite AIR
mirroring Pearl's design, with all 7 LogUp buses enforced at proof
time via `p3-batch-stark`, public-input binding on the trace's last
row, and a multi-shape / multi-activity bench suite.

The earlier M9.1 (composite tile AIR) and M10.1b (in-circuit
BLAKE3 keyed-hash) stacks have been retired — see
[`2026-05-14_ENGINEERING_REPORT.md`](docs/2026-05-14_ENGINEERING_REPORT.md) for the why and
[`2026-05-14_M10_1C_PROGRESS.md`](docs/2026-05-14_M10_1C_PROGRESS.md) for the phase-by-phase
history.

**272 unit tests + 13 ignored benches passing.** Latest PROD bench
(commit `d6065d8`): ~50 s prove / ~140 ms verify / ~890 KB
baseline (~1.65 MB with activity) at `MIN_STARK_LEN = 8192` rows ×
1378 cols, 120-bit provable FRI soundness. *(Note: as of 2026-05-19
the FRI parameter floor was recalibrated to **≥80 bits
unconditional at the Johnson radius** — see "Open lines of work"
below; benches will be re-measured at the new bar.)*

## Open lines of work

These are the **active in-flight residuals**. Each row points to
the design / status doc that owns it.

| Open work | Doc (in [`docs/`](docs/)) | Status |
|---|---|---|
| **Production roadmap** (the index of every milestone) | [`2026-05-17_PRODUCTION_ROADMAP.md`](docs/2026-05-17_PRODUCTION_ROADMAP.md) | Live |
| **M-S5b / P-C2** — ≤100 KB terminal compression of the M-S5 cert (target relaxed from ≤65 KB on 2026-05-21) | [`2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`](docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md) | S(−1) FRI soundness analysis LANDED ([`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](docs/2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md)). S1 routes audit LANDED ([`2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`](docs/2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md)). **S1.B Poseidon2 removal LANDED 2026-05-20 (P0–P7)** ([`2026-05-20_POSEIDON2_REMOVAL_SPEC.md`](docs/2026-05-20_POSEIDON2_REMOVAL_SPEC.md)) — outer-cert flipped from Poseidon2-Goldilocks<8> to Tip5 (one hash family throughout, analogous to Pearl's BLAKE3-throughout). **S1.B size-reduction investigation LANDED 2026-05-20** ([`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](docs/2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md)): full per-lever empirical sweep at ≥80-bit Johnson; **Tier B production flip LANDED** — outer-cert FRI moved from `lb=2 nq=42` (~1011 KB, 85 bits) to `lb=4 nq=20` (~548 KB, 82 bits) for **−46% L1** at +2-bit margin / paper-faithful digest=5; trade-off is 16× LDE (4× prover memory). Tier C (~470 KB Pareto floor) deferred — requires digest=4 paper-divergence. Empirical L2 measurement at Tier B pending. In-substrate floor ~470 KB; ≤100 KB target (relaxed 2026-05-21 from ≤65 KB) likely requires Path A (outermost SNARK wrap) per routes-audit recommendation, possibly attainable via WHIR @ Johnson + higher-lb outer first. |
| **C4 / M-S6** — independent crypto audit | [`2026-05-19_C4_AUDIT_READINESS.md`](docs/2026-05-19_C4_AUDIT_READINESS.md) | Readiness package landed (threat model + soundness-claim index + KAT/adversarial catalogue + known residuals). Team in-house audit walk is the next deliverable; people other than us will also audit. |
| **CSA — Constraint Soundness Analysis (AIR-side of ≥80 unconditional, complements S(−1))** | [Design](docs/2026-05-20_CONSTRAINT_SOUNDNESS_ANALYSIS_DESIGN.md) + [S0](docs/2026-05-20_CONSTRAINT_INVENTORY.md) + [S1](docs/2026-05-20_CONSTRAINT_SOUNDNESS_DERIVATION.md) + [S2](docs/2026-05-20_TAMPER_GAP_LIST.md) + [S3](docs/2026-05-20_TAMPER_TEST_SPECIFICATION.md) + [S5](docs/2026-05-20_CSA_S5_CROSS_AIR_TAMPER_TESTS.md) + [S6](docs/2026-05-20_CSA_S6_PROPERTY_BASED_TESTS.md) + [S7](docs/2026-05-20_CSA_S7_AUDIT_SIGNOFF.md) | **LANDED 2026-05-20 (all 8 stages S0–S7)**. Verdict: per-AIR MIN bits ≥98 (BUS_IRANGE8 the tightest), chain MIN 82 unconditional bits combined with S(−1) FRI; ≥80 floor with margin. 11 new tamper tests + 3 audit-routing doc-comments landed; rejection rate empirically 1.0. Deferred-as-deepening (not gaps): F3–F20 FRI fold-round + per-constraint proptest sweep. M12 GAP-G3 items (BUS_MATMUL_INPUT, BUS_JACKPOT_X_BITS, Tip5 D=2 R-a tail) remain out of M-S6 scope. |
| **Proof-size + parameter-choice measurements** (the post-recalibration source of truth) | [`2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md`](docs/2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md) | Stage A/B/C + S3(ii) measured; L2 = 618 KB; L3 > L2 ⇒ stacked recursion confirmed-dead at the new ≥80-Johnson bar. |
| **C3 / M-S5** vertical-recursion cert — historical record + DT-4 fix | [`2026-05-19_C3_OUTER_CERT_DESIGN.md`](docs/2026-05-19_C3_OUTER_CERT_DESIGN.md) | LANDED (the ≥120-bit version; subsequently re-parametrized to ≥80-Johnson in commits `0334943` / `f54ae81`). |
| **Soundness/security report** | [`2026-05-15_ZKP_SECURITY_REPORT.md`](docs/2026-05-15_ZKP_SECURITY_REPORT.md) | Live |
| **Gap inventory** | [`2026-05-15_GAP_AUDIT.md`](docs/2026-05-15_GAP_AUDIT.md) | Live — new C4 findings (in-house + external) route here per R1 |
| **R-b / M12 / `#127`** — composite `RecursiveAir` (replaces representative `FibonacciAir`) | [`2026-05-14_M10_1C_DESIGN.md`](docs/2026-05-14_M10_1C_DESIGN.md) | Deferred milestone |

The [`docs/`](docs/) directory has the full categorized index in
[`docs/README.md`](docs/README.md) — start there for the broader
context (status reports, AIR designs, M52 / Phase A-CR / §4.C.2 /
Pearl byte-equivalence / C1–C3 recursion substrate / Phase B etc.).

## What works today

For local Layer-0 circuit checks:

```rust
use ai_pow_zk::{
    CircuitConfig, CompositePublicInputs, CompositeTrace, ZkParams,
    composite_proof::{build_config, composite_prove_pinned_logup, composite_verify_pinned_logup},
};

let params  = ZkParams { m: 8, k: 16, n: 8, noise_rank: 2, tile: 2, difficulty_bits: 0 };
let config  = build_config(&params, &CircuitConfig::PROD);

// 1. Build the composite trace. Place instructions via
//    place_blake3_hash / place_matmul_step / place_jackpot_step;
//    use the fill_*_passthrough helpers to thread the final
//    state to the last row.
let mut trace = CompositeTrace::baseline_min();
// ... (place activity here) ...
trace.populate_lookup_freq();  // only needed when proving with
                               // CompositeFullAirWithLookups

// 2. Derive the public-input vector from the trace's last row.
let pis = CompositePublicInputs::derive_from_trace(&trace);

// 3. Prove + verify.
let (proof, program) = composite_prove_pinned_logup(&config, trace, &pis);
composite_verify_pinned_logup(&config, &program, &proof, &pis)?;
```

`composite_prove_pinned_logup` builds a
[`composite_full_air_with_lookups::CompositeFullAirWithLookupsPinned`] trace
from the per-row column layout in [`composite_layout`], runs the Plonky3
batch-STARK pipeline through [`circuit::AiPowStarkConfig`] (Goldilocks + Tip5
sponge + FRI), and returns the proof plus the canonical program needed by the
verifier.

This is still only the Layer-0 circuit primitive. Nockchain block, wire, and
Hoon boundaries must use the recursive certificate APIs and the
full-matmul statement precheck; they must not persist or accept a raw
`AiPowBatchProof`. The old unpinned `composite_prove` / `composite_verify`
helpers are test/dev-only and require the explicit `dev-unsafe` feature.
See the `bench_suite` module for the full pattern.

The proof attests that:

1. **Matmul cumsum** evolves correctly per row, gated by
   `IS_RESET_CUMSUM` / `IS_UPDATE_CUMSUM` selectors (`chips::matmul`).
2. **BLAKE3 hash compressions** are performed correctly when
   placed in 8-row blocks (`chips::blake3`).
3. **Jackpot state** evolves via rotate-XOR-13 with one-hot slot
   routing (`chips::jackpot`).
4. **Cell range checks** for u8 / u13 / i7+1 / i8 are enforced via
   LogUp (`urange8`, `urange13`, `irange7p1`, `irange8` buses).
5. **i8 ↔ u8 conversion** consistency on matrix bytes (`i8u8` bus).
6. **NOISED_PACKED RAM lookup** — matmul A/B reads come from the
   canonical position-keyed matrix store (`noised_packed` bus).
   Merge-mining byte-equivalence anchor.
7. **BLAKE3 CV routing** across non-adjacent rows (`cv_routing` bus).
8. The trace's last-row CUMSUM_TILE and JACKPOT_MSG match the
   claimed `CompositePublicInputs`.

## What's still unbound

(See the "Open lines of work" table above for the doc-pointer
form. This subsection is the in-narrative description.)

- **Full-matmul recursive statement.** The proof binds the chunk-Merkle
  matrix commitments exposed as `HASH_A` / `HASH_B`, and the Rust statement
  derives `s_a` / `s_b` from those proof-bound commitments. Multi-tile
  consensus remains fail-closed until the recursive statement binds a
  full-matmul aggregate or equivalent full-work certificate.
- **Final CV_OUT in public inputs.** The composite trace doesn't
  yet thread "current CV" forward to the last row. Add when
  downstream protocols need the final hash output.
- **Recursion compression (M-S5b / #131).** Vertical-recursion
  cert lands at L1 = 292.92 KB / L2 = 328.83 KB at the 2026-05-21
  anchored 60-bit Johnson floor (Stage 5 measured, 25.1 s wall-clock
  with Rayon + `mds_cyclomul`); the **≤100 KB target** (relaxed
  2026-05-21 from the original ≤65 KB) remains a deferred terminal-
  compression milestone — see
  [`docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`](docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md)
  2026-05-21 addendum for the path tree (WHIR @ Johnson + higher-lb
  outer in-substrate; Path A SNARK wrap as the final lever; Path B
  verifier-AIR floor-attack already explored; stacked recursion
  confirmed-dead at the prior 80-bit bar — re-evaluation pending
  at the new 60-bit anchored floor).

## Module map

| Module | Role |
|---|---|
| [`circuit`] | Plonky3 `StarkConfig` factory. Pins Goldilocks + Tip5 sponge + FRI parameters per profile. `CircuitConfig::PROD` targets 120-bit provable FRI soundness (`80 queries · log_blowup 3 / 2`). |
| [`params`] | `ZkParams` mirror of `MatmulParams` (keeps this crate standalone — no back-dep on `ai-pow`). |
| [`composite_layout`] | The 1378-column composite trace layout (Pearl byte-equivalent). All per-chip column blocks anchored here. |
| [`composite_full_air`] | `CompositeFullAir` — top-level AIR over `TOTAL_TRACE_WIDTH` cols. Calls all 10 chip evals via per-chip `eval_composite` methods. Public-input binding on the trace's last row. |
| [`composite_full_air_with_lookups`] | `CompositeFullAirWithLookups` — same AIR + 7 LogUp bus emissions in a `bus_emit::*` submodule. Used with `p3-batch-stark`'s `prove_batch` / `verify_batch`. |
| [`composite_trace`] | `CompositeTrace` — composite-trace builder. `place_blake3_hash`, `place_matmul_step`, `place_jackpot_step`, `fill_*_passthrough`, `populate_lookup_freq`. |
| [`composite_public`] | `CompositePublicInputs` — typed 60-element PI vector: cumsum, jackpot, matrix roots, job key, jackpot key, and jackpot hash. `derive_from_trace` snapshots and binds the trace-owned values. |
| [`composite_proof`] | Lib-level `composite_prove` / `composite_verify` wrappers around `p3-uni-stark`. |
| [`composite_lookups`] | Lookup-bus design + multiplicity calculus. Names the 7 LogUp buses (`urange8`, `urange13`, `irange7p1`, `irange8`, `i8u8`, `noised_packed`, `cv_routing`). |
| [`composite_preprocess`] | Preprocessed-trace generation (CONTROL_PREP / NOISE_PACKED_PREP / CV_OR_TWEAK_PREP / AB_ID_PREP / A_ID / B_ID / STARK_ROW_IDX). |
| [`composite_lookup_proof`] | Standalone POC AIR demonstrating the `p3-batch-stark` LogUp integration pattern. Useful as a teaching example. |
| [`bench_suite`] | Multi-shape, multi-activity benches at TEST_PEARL and PROD profiles. All `#[ignore]`'d. |
| [`chips::stark_row`] | Monotonic STARK_ROW_IDX increment. |
| [`chips::range_table`] | Generic `RangeTableChip<COL, MIN, MAX>` with URange8/13, IRange7P1/8 instantiations. |
| [`chips::i8u8`] | i8 ↔ u8 sign-conversion table. |
| [`chips::input`] | `NOISE_PACKED_PREP` unpacking + `NOISED_PACKED = polyval(MAT) + polyval(NOISE)` integrity. |
| [`chips::control`] | `CONTROL_PREP` selector-bit unpacking + MAT_ID limb decomposition. |
| [`chips::blake3`] | Pearl-port BLAKE3 chip: scalar reference (`compress`), per-round AIR primitives (`round_ops`), AIR composition (`round_air`), and top-level chip (`chip::Blake3Chip` with selector-gated 8-row hash dispatch). |
| [`chips::matmul`] | `MatmulCumsumChip` — cross-row cumsum-update over TILE_H × TILE_D × TILE_H tiles. |
| [`chips::jackpot`] | `JackpotChip` — 16-slot rotate-XOR-13 with one-hot routing. |

## Stack choices

Goldilocks base field + Tip5 sponge for FRI + `p3-uni-stark` /
`p3-batch-stark` for the proving pipeline. See
[`2026-05-13_DESIGN.md`](docs/2026-05-13_DESIGN.md) for per-slot rationale and
[`2026-05-14_ENGINEERING_REPORT.md`](docs/2026-05-14_ENGINEERING_REPORT.md) for the
post-Phase-14b architectural review.

Plonky3 dependencies (`https://github.com/Plonky3/Plonky3`):

- `p3-air` — AIR trait
- `p3-batch-stark` — LogUp-enforced batched-AIR prover
- `p3-blake3-air` — upstream BLAKE3 sub-AIR (used by `chips::blake3` for cross-checks)
- `p3-challenger`, `p3-commit`, `p3-dft`, `p3-fri`, `p3-merkle-tree`,
  `p3-symmetric` — STARK config plumbing
- `p3-field`, `p3-goldilocks` — field arithmetic and base field
- `p3-lookup` — LogUp / interaction-builder trait
- `p3-matrix`, `p3-uni-stark` — trace + prover

Tip5: not upstream. Recursive certificate proving uses Nockchain's in-repo
[`nockchain_math::tip5`](../nockchain-math/src/tip5/) 5-round
`permute_5round` variant as the FRI sponge; the canonical 7-round
`permute` remains outside this recursive certificate path.

## Licensing

The crate is dual-licensed under `LICENSE-APACHE` and `LICENSE-MIT`
at the workspace root, **except** for the modules listed in
[`LICENSE-PEARL`](LICENSE-PEARL) — those are Pearl-source ports
(`Pearl zk-pow ...`) carrying a top-of-file ISC notice
and are governed by the Pearl ISC license terms reproduced in
that file (Copyright (c) 2025-2026 Pearl Research Labs;
Copyright (c) 2015-2016 The Decred developers). See also
[`../ai-pow/LICENSE-PEARL`](../ai-pow/LICENSE-PEARL) for the
`ai-pow`-side derived-file enumeration.

## Security parameters

- **`CircuitConfig::PROD`**: `log_blowup = 4`, `num_queries = 15`,
  `pow_bits = 1` → 62 bits unconditional Johnson FRI soundness.
  This is the 2026-05-21 anchored-between production floor.
- **`CircuitConfig::TEST_PEARL`**: `log_blowup = 2`, `num_queries = 16`
  → 32 bits Johnson soundness. For fast test round-trips only;
  not production-grade.

## Tests

```sh
cargo test -p ai-pow-zk --lib
```

Runs 272 unit tests in ~4 min including the LogUp proptests.
13 benches are `#[ignore]`'d by default — run individually:

```sh
cargo test -p ai-pow-zk --release --lib bench_suite::tests::bench_prod_8k_baseline -- \
    --ignored --nocapture
```

The 7 KAT tests in `chips/blake3/compress.rs` cross-check our
Pearl-port scalar BLAKE3 against `blake3::Hasher::new_keyed` to
anchor merge-mining compat.

## Where this fits in the `ai-pow` flow

`ai-pow-zk` is downstream of `ai-pow`'s nonce-bound attempt context. The
`ai-pow` bridge constructs a `CompositeTrace` and
`CompositePublicInputs` from verifier-derived attempt data, proves the
Layer-0 composite STARK, and wraps that proof with the canonical recursive
certificate. The recursive certificate plus statement metadata is what the
Hoon-compatible noun encoder serializes for block submission.

The `composite_prove` / `composite_verify` APIs are Layer-0 primitives. They
are useful for circuit tests and for the recursive-certificate builder, but the
raw Layer-0 proof is not the production block artifact. Block, wire, and Hoon
boundaries must consume the recursive certificate and run the full-matmul
statement precheck. That precheck derives canonical seeds from the same
nonce-keyed chunk commitments bound by the proof as `HASH_A` / `HASH_B`, and
it intentionally fails closed for multi-tile shapes until the recursive proof
binds the intended full-matmul work unit.

The production attempt boundary is intentionally minimal-reuse. Changing the
opaque AI-PoW nonce must force fresh transcript-derived commitments, noise,
noised matrix strips, tile states, jackpot preimages, and proof witness data.
Cache-friendly attempt reuse is a vulnerability, not a desired trait or
optimization target; only immutable non-work inputs such as matrix bytes, shape
metadata, and chain-pinned parameters may be reused across attempts.
