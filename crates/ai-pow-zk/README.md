# `ai-pow-zk`

EXPERIMENTAL — a Plonky3 SNARK circuit for the
[`ai-pow`](../ai-pow/) tiling matmul puzzle. The role is the same as
Pearl's [`zk-pow`](../../pearl/zk-pow/): wrap the multi-MB plain proof
in a compact SNARK so it can fit in a block certificate.

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
| **M-S5b / P-C2** — ≤65 KB terminal compression of the M-S5 cert | [`2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`](docs/2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md) | Design + KAT-first de-risk plan landed; S1 (Path B verifier-AIR reduction map) is the next deliverable. Empirical post-recalibration L2 = ~618 KB → ~9.5× over 65 KB. |
| **C4 / M-S6** — independent crypto audit | [`2026-05-19_C4_AUDIT_READINESS.md`](docs/2026-05-19_C4_AUDIT_READINESS.md) | Readiness package landed (threat model + soundness-claim index + KAT/adversarial catalogue + known residuals). Team in-house audit walk is the next deliverable; people other than us will also audit. |
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
(`pearl/zk-pow/src/circuit/...`) carrying a top-of-file ISC notice
and are governed by the Pearl ISC license terms reproduced in
that file (Copyright (c) 2025-2026 Pearl Research Labs;
Copyright (c) 2015-2016 The Decred developers). The Pearl
upstream itself is vendored read-only under
[`../../pearl/`](../../pearl/) for line-level cross-reference;
see also [`../ai-pow/LICENSE-PEARL`](../ai-pow/LICENSE-PEARL)
for the ai-pow-side derived-file enumeration.

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
