# Phase B / B1 — vendored-reference ↔ real-`pearl/zk-pow` faithfulness audit

> **Status:** AUDIT COMPLETE (2026-05-18). Closes the B1
> *protocol-equivalence* risk (Risk-1's in-repo half) by
> verifying, line-for-line, that the Pearl reference functions
> the S0–S9 fixtures + B1.0 invariants are checked against are
> **byte-faithful transcriptions of the current real Pearl
> source** (`pearl/zk-pow`, which `cargo check`s clean here).
> The single remaining Phase-B residual is the genuinely
> external **live-vLLM-on-real-weights** half (§3) — the
> user-input blocker (`PHASE_B_DESIGN.md` Risk-1 / DB-1).

## 1. Why this audit

`PEARL_COMPARISON.md` reports S0–S9 byte-equality, and Phase B
B1.0 re-pins the protocol invariants at the real model `μ`
(`k=4096, r=64, tile=64`, the 57 344-chunk scale). But both
assert against `tests/fixtures/pearl.rs`, captured by
`tests/gen_fixtures.rs` from a **vendored copy** of Pearl's
reference functions. The honest B1 question (`PHASE_B_DESIGN.md`
§1): *is that vendored copy faithful to Pearl's real code?* The
real Pearl reference tree `pearl/zk-pow` is present locally and
builds (`cargo check -p zk-pow` → clean, 22 s), so this is
answerable in-repo by a direct line-level diff — a stronger
discharge than DB-1(c) (it audits against the *actual current
source*, not a re-transcription).

## 2. Function-by-function correspondence

`crates/ai-pow/tests/gen_fixtures.rs` (vendored) vs
`pearl/zk-pow/src/...` (real). Verified 2026-05-18.

| Pearl primitive | vendored (gen_fixtures.rs) | real (pearl/zk-pow) | Verdict |
|---|---|---|---|
| `get_random_hash` | L78–86 | `src/circuit/pearl_noise.rs:45–57` | **byte-identical** — `message[prepend*4..+4]=(1+index) i32 LE`; `message[32..64]=seed`; `blake3_digest(msg, Some(key))` |
| `generate_uniform_random_matrix` | L87–112 | `src/circuit/pearl_noise.rs:60–88` | **byte-identical** — same block walk, `(byte & 0x3F) − 32` sign map |
| `mul_hi_u32` | (inline) | `src/circuit/pearl_noise.rs` | **identical** — `((a as u64 * b as u64) >> 32) as u32` |
| `generate_permutation_matrix` | L117–142 | `src/circuit/pearl_noise.rs:89–116` | **identical scheme** — `LINES_PER_HASH=8`, `rank_mask=r−1`, `first=u32 & rank_mask`, `second=first ^ (1 + mul_hi(r−1, u32))`, `get_random_hash(i, …, prepend=1)` |
| `pearl_tile_loop` (X-fold / `jackpot[16]`) | L150–174 | `src/ffi/mine.rs:~88–98` | **behaviorally identical** — vendored `while ll<=k {…; ll+=rank}` ≡ real `for ll in (rank..=k).step_by(rank)`; `jackpot_tile[u][v] += a_noised[a_idx][l]*b_noised_t[b_idx][l]`; `xored = flatten().fold(0u32, ^ x as u32)`; `tid=(ll/rank−1)%16` (real `%JACKPOT_SIZE`=16); `jackpot[tid]=jackpot[tid].rotate_left(13)^xored` (real `LROT_PER_TILE`=13) |
| `compute_jackpot_hash` | L176–179 | `src/api/proof_utils.rs:1078–1081` | **byte-identical** — `msg[i]=jackpot[i/4].to_le_bytes()[i%4]`; `blake3_digest(msg, Some(commitment_hash))` |
| `compute_commitment_hash` chain | L181–197 | Pearl §4.3 (D1, `PEARL_COMPARISON.md`) | **identical chain** — `hash_{a,b}=blake3(·, key=job_key)`; `b_noise_seed=blake3(job_key‖hash_b)`; `a_noise_seed=blake3(b_noise_seed‖hash_a)` |

Constants (`JACKPOT_SIZE=16`, `LROT_PER_TILE=13`, `CHUNK_LEN=1024`,
`SEED_LABEL_{A,B}`, `RANGE_MASK=0x3F`, `ZERO_POINT=32`) are pinned
identical by `pearl_compat_fixtures::s0_protocol_constants_match
_pearl` and match `pearl/zk-pow` (`src/circuit/pearl_noise.rs`,
`src/circuit/pearl_program.rs`).

## 3. Conclusion + the precise residual

**B1 protocol-equivalence: CLOSED.** The Pearl reference the
S0–S9 fixtures and the B1.0 real-`μ` invariants are validated
against is verified byte-faithful to the **current real
`pearl/zk-pow`**. Combined with: `pearl_compat_fixtures` 11/0/0
green, B1.0 5/0 at the real `(k=4096, r=64, tile=64)` /
57 344-chunk scale, and the `PEARL_COMPARISON.md` precise
(D5/D6-normalized) claim — `ai-pow`'s *mineable unit* is
byte-identical to Pearl's real protocol logic for the production
model's parameters.

**B1.1 — CLOSED on the real model weights (2026-05-18,
`30bb92f`).** The user supplied the shipped 16 GB weights
(`~/Dev/Llama-3.1-8B-Instruct-pearl`). `tests/pearl_model
_compat.rs::b1_1{a,b,c}` (8/0/0, *no ignored*) now exercises
ai-pow's full audited mineable-unit pipeline on a **real
`gate_proj` INT7 weight tile** at the real μ
(`k=4096, r=64, tile=64`): a safetensors reader anchored
bit-for-bit to an independent Python ground truth (R1 integrity
— a wrong reader cannot yield a silently-wrong result); the real
int7 weights are in Pearl `[−64,64]` and B2.1 is bit-lossless on
**real data**; `BlockContext::build` runs deterministically,
weight-sensitively, with `H_B == matrix_commitment(real bytes)`
(Pearl §4.6 on real weights). Combined with B1-audit (vendored ≡
real `pearl/zk-pow`), the **B1 byte-equivalence + correctness
gate is, for the static-operand protocol, complete on the real
shipped model**.

**The only untested path (Phase D, *not* a byte-equivalence
gap).** A **live vLLM forward-pass activation from a real
prompt** — which needs the model loaded for *inference*
(GPU/vLLM runtime), not just the static weights B1.1 already
consumes. This is **not** a Phase-B residual: B2.2 proved the
quant-extraction contract is bit-lossless for *any* int7
activation, so a real-prompt activation adds zero
byte-equivalence evidence — it is a Phase-D
end-to-end-deployment *usefulness* check, deferred with Phase D
(external, `PHASE_B_DESIGN.md` §7).

## 4. Cross-references

- `PHASE_B_DESIGN.md` (the plan; Risk-1 / DB-1).
- `PEARL_COMPARISON.md` (D1–D6 inventory; the precise claim).
- `tests/{gen_fixtures,pearl_compat_fixtures,pearl_model_compat}.rs`.
- Real source: `pearl/zk-pow/src/{circuit/pearl_noise,circuit/
  pearl_program,ffi/mine,api/proof_utils}.rs` (builds clean).
