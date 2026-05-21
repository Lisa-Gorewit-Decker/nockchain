# `ai-pow-zk`

EXPERIMENTAL — a Plonky3 SNARK circuit for the
[`ai-pow`](../ai-pow/) tiling matmul puzzle. The role is the same as
Pearl's `zk-pow`: wrap the multi-MB plain proof
in a compact SNARK so it can fit in a block certificate.

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

The **only** hash function in the live SNARK proving path is:

- **Tip5** (Nockchain's 7-round variant; KAT-anchored to
  `nockchain_math::tip5::permute`, the in-tree bit-for-bit twin
  also exposed via `p3-tip5-circuit-air::Tip5Perm`). Used at
  **every** layer:
  - **Inner ai-pow-zk STARK** — `Tip5Perm` (width 16, rate 10,
    digest 5) is the MMCS hash + Fiat-Shamir challenger
    permutation (`crates/ai-pow-zk/src/circuit.rs:186, 203`).
    KAT'd against the C2.1 Tip5 perm AIR (the soundness linchpin;
    `Plonky3-recursion/tip5-circuit-air/src/air_lookup.rs`).
  - **Outer-cert L1/L2 (recursion verifier circuit)** — `Tip5Perm`
    everywhere via `config::goldilocks_tip5_80bit()` (post-2026-05-20
    M-S5b S1.B P5 flip; was Poseidon2-Goldilocks<8>).
    `Plonky3-recursion/circuit-prover/src/config.rs::goldilocks_tip5_80bit`.

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
- **Nockchain choice**: **N=7 rounds** — 2 rounds above the
  paper's spec, providing additional margin against future
  cryptanalysis. C2.1 keystone byte-identical against `259cab2`.
- **Third-party cryptanalysis**: "Opening the Blackbox" (Liu,
  Koschatko, Grassi, Yan, Chen, Banik, Meier, **IACR ePrint
  2024/1900**). Practical SFS collision on 3-round Tip5 at 2^41.2;
  full collision on 3-round at 2^121.1. No attack reaches 4-round
  Tip5 or above.
- **Nockchain safety margin**: **4 rounds above broken** (7 − 3),
  twice the Tip5 paper's post-OtB 2-round margin (5 − 3).
- **Sponge collision security**: `min(capacity/2, output)` =
  `min(6×64/2, 5×64)` = `min(192, 320)` = **192 bits**. Well
  above the ≥80-bit floor.

#### What's NOT in the SNARK proving stack

- **No Poseidon2 (any variant: W8, W16, W24, Fused).** The
  outer-cert flipped to Tip5 in P5 of M-S5b S1.B (2026-05-20)
  per maintainer directive "I'm not willing to use Poseidon2".
  Poseidon2 remains in:
  - `Plonky3-recursion/circuit-prover/src/config.rs::goldilocks()`
    — the GENERAL-PURPOSE Goldilocks STARK config; NOT used by
    ai-pow-zk's recursive proving. Kept for non-recursive
    test cases.
  - `Plonky3-recursion/circuit-prover/src/batch_stark_prover/poseidon2.rs`
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
  arithmetization-oriented hashes. Tip5 is the sole choice.
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

- **Provable bound**: **≥80 bits unconditional at the Johnson
  radius**, anchored on Ben-Sasson, Carmon, Habock, Kopparty,
  Saraf, "On Proximity Gaps for Reed–Solomon Codes" (**IACR
  ePrint 2025/2055**, Nov 2025) Theorem 1.5 + §1.3.2.
- **Formula**: `unconditional_bits ≈ log_blowup · num_queries +
  commit_proof_of_work_bits + query_proof_of_work_bits`.
- **Per-layer (LANDED FRI configurations)**:
  - Inner Tip5-L0 PROD: `lb=3, nq=30, pow=0+0` ⇒ **90 bits**
    unconditional (configurable; LB2/LB4/LB5/LB6 variants all
    ≥80; per `crates/ai-pow-zk/src/circuit.rs:90-142`).
  - Outer-cert L1/L2 (`goldilocks_tip5_80bit`): **production FRI
    parameters as of 2026-05-20 (post-Phase-0)** stack every
    soundness-neutral compression lever — `lb=4, nq=20, mla=3,
    lfp=2, cap=3, pow=1+1, d=5` ⇒ **82 bits** unconditional
    Johnson (`Plonky3-recursion/circuit-prover/src/config.rs`).
    Pre-2026-05-20 baseline (`lb=2 nq=42 mla=1 lfp=0 cap=0`) was
    85 bits, ~1011 KB L1.
    **Measured at production-faithful params (Stage 5
    post-5-round-Tip5 commit `88bb526`, 9.5 min):** L1 = 402.94 KB
    (**~−60% vs pre-2026-05-20 baseline ~1011 KB**),
    L2 = 438.79 KB, **L2/L1 = 1.089×**. Trade-off: `lb=4` ⇒ 16×
    LDE (vs prior 4×) ⇒ ~4× prover memory; 5-round Tip5 dropped
    prover time ~57% (22 min → 9.5 min). **The ai-pow-zk-specific
    5-round Tip5 (paper-spec per IACR 2023/107 §2.4 N=5; canonical
    Nockchain 7-round `permute` UNCHANGED) was the single biggest
    proof-size lever — bigger than Tier B (−46%), Phase 0
    (additional −1%), and Path B B2 (~0%) combined.**
    **For ≤65 KB:** still requires Path A (SNARK wrap); in-substrate
    post-quantum floor is now ~403 KB (vs ≤65 KB target = ~6.2×
    over). Path A is the only known path to the target.
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
  - **Combined chain MIN**: **82 bits unconditional at the
    outer cert** (FRI binds at Tier B; AIR + LogUp have ≥16-bit
    margin to FRI).

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
  or other primes. (These exist in `Plonky3-recursion/circuit-prover/src/config.rs`
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
`Plonky3-recursion/`:

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
| **M-S5b / P-C2** — ≤65 KB terminal compression of the M-S5 cert | [`2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`](docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md) | S(−1) FRI soundness analysis LANDED ([`2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md`](docs/2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md)). S1 routes audit LANDED ([`2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`](docs/2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md)). **S1.B Poseidon2 removal LANDED 2026-05-20 (P0–P7)** ([`2026-05-20_POSEIDON2_REMOVAL_SPEC.md`](docs/2026-05-20_POSEIDON2_REMOVAL_SPEC.md)) — outer-cert flipped from Poseidon2-Goldilocks<8> to Tip5 (one hash family throughout, analogous to Pearl's BLAKE3-throughout). **S1.B size-reduction investigation LANDED 2026-05-20** ([`2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md`](docs/2026-05-20_RECURSIVE_PROOF_SIZE_INVESTIGATION.md)): full per-lever empirical sweep at ≥80-bit Johnson; **Tier B production flip LANDED** — outer-cert FRI moved from `lb=2 nq=42` (~1011 KB, 85 bits) to `lb=4 nq=20` (~548 KB, 82 bits) for **−46% L1** at +2-bit margin / paper-faithful digest=5; trade-off is 16× LDE (4× prover memory). Tier C (~470 KB Pareto floor) deferred — requires digest=4 paper-divergence. Empirical L2 measurement at Tier B pending. In-substrate floor ~470 KB; ≤65 KB target still requires Path A (outermost SNARK wrap) per routes-audit recommendation. |
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

```rust
use ai_pow_zk::{
    composite_prove, composite_verify,
    CircuitConfig, CompositePublicInputs, CompositeTrace, ZkParams,
    composite_proof::build_config,
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
let proof = composite_prove(&config, trace, &pis);
composite_verify(&config, &proof, &pis)?;
```

`composite_prove` builds a [`composite_full_air::CompositeFullAir`]
trace from the per-row column layout in [`composite_layout`], runs
the full Plonky3 STARK pipeline through [`circuit::AiPowStarkConfig`]
(Goldilocks + Tip5 sponge + FRI), and serializes via bincode.

For LogUp-enforced cross-chip lookups (the cryptographically
complete form), wrap with `prove_batch` / `verify_batch` from
`p3-batch-stark` against [`composite_full_air_with_lookups::CompositeFullAirWithLookups`].
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
   canonical matrix store (`noised_packed` bus). Merge-mining
   byte-equivalence anchor.
7. **BLAKE3 CV routing** across non-adjacent rows (`cv_routing` bus).
8. The trace's last-row CUMSUM_TILE and JACKPOT_MSG match the
   claimed `CompositePublicInputs`.

## What's still unbound

(See the "Open lines of work" table above for the doc-pointer
form. This subsection is the in-narrative description.)

- **`h_a` / `h_b` matrix bindings.** The witness's matrix entries
  aren't yet tied to chain-pinned chunk-Merkle roots. An adversary
  can still pick any `(a, b)` and run the matmul on them. Multi-
  week deferred work — task #52.
- **Final CV_OUT in public inputs.** The composite trace doesn't
  yet thread "current CV" forward to the last row. Add when
  downstream protocols need the final hash output.
- **Recursion compression (M-S5b / #131).** A vertical-recursion
  cert lands at ~618 KB L2 at the new ≥80-Johnson bar (Stage B
  measured); the ≤65 KB target remains deferred to M-S5b — see
  [`docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`](docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md)
  for the path tree (Path B verifier-AIR floor-attack now primary;
  stacked recursion confirmed-dead).

## Module map

| Module | Role |
|---|---|
| [`circuit`] | Plonky3 `StarkConfig` factory. Pins Goldilocks + Tip5 sponge + FRI parameters per profile. `CircuitConfig::PROD` targets 120-bit provable FRI soundness (`80 queries · log_blowup 3 / 2`). |
| [`params`] | `ZkParams` mirror of `MatmulParams` (keeps this crate standalone — no back-dep on `ai-pow`). |
| [`composite_layout`] | The 1378-column composite trace layout (Pearl byte-equivalent). All per-chip column blocks anchored here. |
| [`composite_full_air`] | `CompositeFullAir` — top-level AIR over `TOTAL_TRACE_WIDTH` cols. Calls all 10 chip evals via per-chip `eval_composite` methods. Public-input binding on the trace's last row. |
| [`composite_full_air_with_lookups`] | `CompositeFullAirWithLookups` — same AIR + 7 LogUp bus emissions in a `bus_emit::*` submodule. Used with `p3-batch-stark`'s `prove_batch` / `verify_batch`. |
| [`composite_trace`] | `CompositeTrace` — composite-trace builder. `place_blake3_hash`, `place_matmul_step`, `place_jackpot_step`, `fill_*_passthrough`, `populate_lookup_freq`. |
| [`composite_public`] | `CompositePublicInputs` — typed 20-element PI vector (4 i32 cumsum + 16 u32 jackpot). `derive_from_trace` snapshots the trace's last row. |
| [`composite_proof`] | Lib-level `composite_prove` / `composite_verify` wrappers around `p3-uni-stark`. |
| [`composite_lookups`] | Lookup-bus design + multiplicity calculus. Names the 7 LogUp buses (`urange8`, `urange13`, `irange7p1`, `irange8`, `i8u8`, `noised_packed`, `cv_routing`). |
| [`composite_preprocess`] | Preprocessed-trace generation (CONTROL_PREP / NOISE_PACKED_PREP / CV_OR_TWEAK_PREP / AB_ID_PREP / STARK_ROW_IDX). |
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

Tip5: not upstream. We re-use Nockchain's in-repo
[`nockchain_math::tip5`](../nockchain-math/src/tip5/) (7-round
parameter set) as the FRI sponge.

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

- **`CircuitConfig::PROD`**: `log_blowup = 3`, `num_queries = 80` →
  `80 · 3 / 2 = 120` bits provable FRI soundness. Bench: ~50 s
  prove / ~140 ms verify / ~900 KB baseline / ~1.65 MB with
  activity at `MIN_STARK_LEN = 8192` rows × 1378 cols.
- **`CircuitConfig::TEST_PEARL`**: `log_blowup = 2`, `num_queries = 16`
  → ~12 bits provable soundness. For fast test round-trips only;
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

`ai-pow-zk` is downstream of `ai-pow`'s `MatmulProof` / plain
proof. The plan is for `ai-pow` to construct a `CompositeTrace`
from a verified plain proof + place the corresponding instructions
into specific rows, then call `composite_prove` to produce the
compact SNARK that gets transmitted in the block certificate.

The `composite_prove` / `composite_verify` API is the boundary;
neither crate sees the other's types past `ZkParams` +
`CompositeTrace` + `CompositePublicInputs`.

Today the integration is one-directional — `ai-pow-zk` exposes the
API; `ai-pow` hasn't yet been updated to actually call it. The
`prover.rs` in `ai-pow` has a stub comment marking the call site.
