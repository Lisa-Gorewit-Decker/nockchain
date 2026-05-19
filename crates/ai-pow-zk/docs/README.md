> _Index created **2026-05-19** when the ai-pow-zk design/audit/report documents were organized into `crates/ai-pow-zk/docs/`. Each document carries its own `created · last updated` header line._

# ai-pow-zk — documentation index

Design, soundness-audit, and engineering documents produced across the
ai-pow-zk sessions. The crate `README.md` stays at the crate root (Rust
convention); everything else lives here. Dates are git-derived
(`created` = first commit that added the file, `last updated` = most
recent commit that touched it).

## Status · roadmap · cross-cutting

| Document | Created | Last updated | What it is |
|---|---|---|---|
| [`PRODUCTION_ROADMAP.md`](PRODUCTION_ROADMAP.md) | 2026-05-17 | 2026-05-19 | Authoritative milestone roadmap (C1–C4, P-A/B/C, M-S5/M-S5b). |
| [`ROADMAP.md`](ROADMAP.md) | 2026-05-13 | 2026-05-14 | Earlier roadmap (superseded by `PRODUCTION_ROADMAP.md`). |
| [`ENGINEERING_REPORT.md`](ENGINEERING_REPORT.md) | 2026-05-14 | 2026-05-14 | Engineering rationale / the "why". |
| [`ZKP_SECURITY_REPORT.md`](ZKP_SECURITY_REPORT.md) | 2026-05-15 | 2026-05-19 | Soundness/security report (CRIT/HIGH findings + closure). |
| [`GAP_AUDIT.md`](GAP_AUDIT.md) | 2026-05-15 | 2026-05-19 | Gap inventory + closure tracking. |
| [`DESIGN.md`](DESIGN.md) | 2026-05-13 | 2026-05-13 | Base AIR / per-slot design. |
| [`PROFILING.md`](PROFILING.md) | 2026-05-15 | 2026-05-15 | Profiling (samply / peak-RSS) methodology. |

## Recursion substrate — C1–C3 / M-S3–M-S5

| Document | Created | Last updated | What it is |
|---|---|---|---|
| [`C1_RECURSION_VENDOR_DESIGN.md`](C1_RECURSION_VENDOR_DESIGN.md) | 2026-05-18 | 2026-05-18 | C1/M-S3 — vendoring + rev-aligning `Plonky3-recursion`. |
| [`C2_TIP5_CIRCUIT_AIR_DESIGN.md`](C2_TIP5_CIRCUIT_AIR_DESIGN.md) | 2026-05-18 | 2026-05-19 | C2/M-S4 — Tip5 circuit AIR + challenger/MMCS; C2.0–C2.4 + R-a. |
| [`C2_TIP5_AIR_DEGREE_WIDTH_TRADEOFF.md`](C2_TIP5_AIR_DEGREE_WIDTH_TRADEOFF.md) | 2026-05-18 | 2026-05-18 | Degree-4 vs width tradeoff for the Tip5 AIR. |
| [`C3_OUTER_CERT_DESIGN.md`](C3_OUTER_CERT_DESIGN.md) | 2026-05-19 | 2026-05-19 | C3/M-S5 — outer recursive cert; DT-1→DT-4, the ≥120-bit re-scope (§13.2/§14/§15). |

## Soundness bindings & AIR

| Document | Created | Last updated | What it is |
|---|---|---|---|
| [`CANONICAL_PROGRAM_DESIGN.md`](CANONICAL_PROGRAM_DESIGN.md) | 2026-05-17 | 2026-05-18 | Phase A-CR — params-pure `canonical_program` / CRIT-1. |
| [`SEC_4C2_NOISE_BINDING_DESIGN.md`](SEC_4C2_NOISE_BINDING_DESIGN.md) | 2026-05-17 | 2026-05-18 | §4.C.2 noise-binding (zero-gap on the 16∣r path). |
| [`M52_MATRIX_BINDING.md`](M52_MATRIX_BINDING.md) | 2026-05-14 | 2026-05-14 | M52 BLAKE3 chunk-Merkle matrix binding. |
| [`HIGH2_2_DESIGN.md`](HIGH2_2_DESIGN.md) | 2026-05-15 | 2026-05-17 | HIGH-2.2 — honest matmul→fold→C4-hash chain. |
| [`P_B2_STRIP_OPENING_DESIGN.md`](P_B2_STRIP_OPENING_DESIGN.md) | 2026-05-17 | 2026-05-17 | P-B.2.2 in-circuit strip-opening (reuses the C3 binding). |
| [`BLAKE3_CHIP_ROUND_GATE_BUG.md`](BLAKE3_CHIP_ROUND_GATE_BUG.md) | 2026-05-15 | 2026-05-15 | BLAKE3 chip round-gate bug writeup. |

## M10.1c / G3 recursion-aggregation lineage

| Document | Created | Last updated | What it is |
|---|---|---|---|
| [`M10_1C_DESIGN.md`](M10_1C_DESIGN.md) | 2026-05-14 | 2026-05-14 | M10.1c composite design. |
| [`M10_1C_PROGRESS.md`](M10_1C_PROGRESS.md) | 2026-05-14 | 2026-05-14 | M10.1c phase-by-phase progress. |
| [`G3_RECURSION_AGGREGATION.md`](G3_RECURSION_AGGREGATION.md) | 2026-05-17 | 2026-05-17 | G3 recursion-aggregation design. |
| [`G3_RECURSION_AUDIT.md`](G3_RECURSION_AUDIT.md) | 2026-05-17 | 2026-05-17 | G3 recursion audit. |
| [`M_S2_G3AB_DESIGN.md`](M_S2_G3AB_DESIGN.md) | 2026-05-17 | 2026-05-17 | M-S2 G3-A/B design. |
| [`M_S2_PEARL_EVALUATION.md`](M_S2_PEARL_EVALUATION.md) | 2026-05-17 | 2026-05-17 | Pearl 3-layer recursion evaluation (origin of the ≤65 KB target). |

## Pearl model fidelity · Phase-B · vLLM

| Document | Created | Last updated | What it is |
|---|---|---|---|
| [`PHASE_B_DESIGN.md`](PHASE_B_DESIGN.md) | 2026-05-18 | 2026-05-18 | Phase-B Pearl byte-equivalence & correctness. |
| [`PEARL_FP8_SCOPING.md`](PEARL_FP8_SCOPING.md) | 2026-05-18 | 2026-05-18 | Pearl FP8 scoping. |
| [`PEARL_VLLM_CPU_FORK_DESIGN.md`](PEARL_VLLM_CPU_FORK_DESIGN.md) | 2026-05-18 | 2026-05-18 | vLLM-CPU fork design (Phase-D real forward). |
