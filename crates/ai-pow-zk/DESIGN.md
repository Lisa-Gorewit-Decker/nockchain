# `ai-pow-zk` — circuit design

A Plonky3 SNARK proving the `ai-pow` tiling-matmul puzzle. Logically
equivalent to Pearl's Plonky2 STARK (`pearl/zk-pow/`); not byte-
identical. Where Pearl pins a specific protocol layout (Pearl
chip / control-bit encoding / recursion ladder), we re-derive the
same logical guarantees over a slightly different Plonky3 stack —
Goldilocks base field, Tip5 sponge for FRI, the upstream
`p3-blake3-air` for the BLAKE3 sub-circuit.

This document is the **logical plan** + **parameter choices**, in
service of the actual circuit landing in follow-up commits. Every
`todo!()` in `src/` corresponds to a section below.

## 1. Goal

Given an `ai-pow` plain `MatmulProof` (as produced by
`ai_pow::prover::mine`), output a fixed-size Plonky3 STARK proof
attesting that:

1. The miner committed to matrices `A` (m × k) and `B` (k × n)
   whose chunk-Merkle roots are the public inputs `h_a` and `h_b`.
2. Per Pearl §4.3, the noise seeds `s_A`, `s_B` were derived from
   `kappa = BLAKE3(block_commitment || params_tag)` and `(h_a, h_b)`
   through the canonical unkeyed-BLAKE3 chain.
3. The noise factors `E_L`, `E_R`, `F_L`, `F_R` are the canonical
   outputs of the Pearl PRNG seeded by `(s_A, s_B)` (Pearl §4.4).
4. The `(found_i, found_j)` tile satisfies the Pearl §4.5 iterative
   tile-state evolution under the row/column strips of `A + E` and
   `B + F`, producing a 16-slot `M` state.
5. `found_leaf = BLAKE3-keyed(M_bytes, pow_key)` where
   `pow_key = derive_key("pow-key", s_A || nonce)` and the public
   input `found_leaf` equals what the chain pinned.
6. `found_leaf ≤ 2^(256 − b) · r · t^2` (Pearl §4.5 hardness rule).

The proof is verifiable in milliseconds with kilobyte-scale wire
size. The plain `MatmulProof` carrying multi-MB strip openings is
replaced by this SNARK as the block certificate.

## 2. Stack choices

| Slot | Choice | Rationale |
|---|---|---|
| Base field | Goldilocks (`p3-goldilocks`) | Plonky3-native, 64-bit prime (`2^64 − 2^32 + 1`). Matches Pearl's choice. Lifts cleanly over a 32-bit boundary, which is convenient for the int32 accumulator entries and the BLAKE3 sub-AIR's 32-bit operations. |
| Challenge extension | `BinomialExtensionField<Goldilocks, 2>` (degree 2) | ~128 bits per FRI challenge. Standard pairing for Goldilocks STARKs. Higher-degree extensions buy more security per query but cost commit time. |
| FRI hash | **Nockchain Tip5** (`nockchain_math::tip5`) — 7-round variant | ZK-friendly arithmetic-hash sponge over Goldilocks. The 7-round parameter count is Nockchain's modification of the published Tip5 paper. Plonky3 upstream does *not* ship a `p3-tip5` crate; we wire in the in-repo implementation at `crates/nockchain-math/src/tip5/` directly via a `Tip5Perm` adapter (`src/circuit.rs`). State = 16 Goldilocks elements; rate 10, capacity 6, digest 5 elements. |
| Merkle MMCS | `MerkleTreeMmcs<Val, Tip5Perm, _>` | Standard Plonky3 mixed-matrix commitment. `PaddingFreeSponge<Tip5Perm, 16, 10, 5>` for leaves; `TruncatedPermutation<Tip5Perm, 2, 5, 16>` for the 2-to-1 compress step. |
| PCS | `TwoAdicFriPcs<…>` | Univariate FRI, the path `p3-uni-stark` expects. |
| Challenger | `DuplexChallenger<Val, _, _, _>` with Tip5 | Fiat-Shamir over the same permutation as the MMCS hashes for code reuse. |
| BLAKE3 sub-AIR | `p3-blake3-air` | Avoid reinventing the BLAKE3 wheel; the upstream AIR proves one compression-function call per fixed window of trace rows. |

### Why not Plonky2 + recursion (Pearl's stack)?

Pearl uses Plonky2 with Goldilocks + Poseidon + a 3-tier recursion
ladder to compress STARK proofs down to ~60 KB. We're starting on
Plonky3 because:

- Plonky3 is the active development line for AIR-based proving over
  Goldilocks; new field arithmetic and `p3-fri` optimizations land
  there.
- Plonky3 lets us reuse the upstream `p3-blake3-air` directly. Pearl
  has its own hand-written BLAKE3 chip in `blake_program.rs` (115
  lines plus the `blake3_chip` constraints — substantial).
- Recursion is deferred to v2. Pre-recursion a raw uni-STARK over
  this AIR will be ~150–300 KB at production matmul shapes; we'll
  layer in compression later.

## 3. Logical trace layout

We mirror Pearl's four-chip interleaving: each row carries columns
from four sub-AIRs side-by-side, and preprocessed control bits pick
which sub-AIR is "live" on that row. Padding rows are no-ops.

```
+-----------------------------------------------------------+
| Row index                                                 |
+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+-----------+
|  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |           |
|  | INPUT       | BLAKE3            | MATMUL    | JACKPOT   |
|  | (i7/i6/i8   | (one row per      | (one row  | (XOR fold |
|  |  range +    | BLAKE3 round)     | per       | + rotate- |
|  |  strip      |                   | r-stripe) | 13-XOR    |
|  |  unpack)    |                   |           | update)   |
|  |             |                   |           |           |
+--+-------------+-------------------+-----------+-----------+
   ^             ^                   ^           ^
   |             |                   |           |
   selector_input  selector_blake3   selector_matmul selector_jackpot
   (preprocessed control bits)
```

The four chips:

1. **Input chip.** Range-checks `a_rows` (i7 → `[-64, 63]`),
   `b_cols` (i7), `e_l` / `f_r` (i6 → `[-32, 31]`) values read from
   the witness, and unpacks the i8 `A' = A + E`, `B' = B + F` values
   the matmul chip consumes. Uses five range tables: `u8`, `u13`,
   `i7+1`, `i8`, and the `i8 ↔ u8` reinterpret table (mirrors
   Pearl's range tables in `pearl_air.rs:62-66`).

2. **BLAKE3 chip.** Wraps `p3-blake3-air`. Active for the ~7 keyed-
   hash calls per attempt:
   - `kappa = BLAKE3(block_commitment ‖ params_tag)`
   - `s_B   = BLAKE3(kappa ‖ h_b)`
   - `s_A   = BLAKE3(s_B   ‖ h_a)`
   - `h_a  = BLAKE3-keyed(pad(a_bytes), key=kappa)` (chunk-Merkle
     root; one compression call per 1024-byte chunk plus the
     parent-CV calls).
   - `h_b  = BLAKE3-keyed(pad(b_bytes), key=kappa)` (same shape).
   - `pow_key = derive_key("pow-key", s_A ‖ nonce)`.
   - `found_leaf = BLAKE3-keyed(M_bytes, key=pow_key)`.

3. **Matmul chip.** For tile `(found_i, found_j)`, walks `⌊k/r⌋`
   r-stripes. Per stripe row, enforces:

   ```
   C_blk[step+1] = C_blk[step] + Σ_{l=lo..hi} A'[i, l] · B'[l, j]
   ```

   for every `(i, j)` in the tile's `t × t` shape. `A'` and `B'`
   columns are populated from the witness via the Input chip's
   unpack rows.

4. **Jackpot chip.** Once per stripe boundary:

   ```
   X = ⊕_{e ∈ C_blk} e          (u32 XOR, via 4-byte decomposition)
   slot = step mod 16
   M_new[slot] = rotate_left_13(M_old[slot]) ⊕ X
   M_new[other slots] = M_old[other slots]
   ```

   After the last stripe, hand `M` to the BLAKE3 chip as the input
   to the `found_leaf` hash.

## 4. Public inputs

Same set as Pearl's `PublicProofParams` plus our shape-aware target.
Encoded as Goldilocks field elements (u32 → 1 Goldilocks each;
hashes laid out as 8 × u32 since BLAKE3 outputs 32 bytes = 8 u32s
LE).

| Public input | Size (Goldilocks elements) | Notes |
|---|---|---|
| `params_tag` | 8 | hash of `MatmulParams` (`ai_pow::prover::params_tag`); already binds `difficulty_bits` and all shape fields, so `b` does not need a separate slot. |
| `h_a` | 8 | matrix-A chunk-Merkle root |
| `h_b` | 8 | matrix-B chunk-Merkle root |
| `comm_m` | 8 | tile-state Merkle root |
| `found_i`, `found_j` | 2 | tile coordinates |
| `found_leaf` | 8 | `BLAKE3-keyed(M, pow_key)` |
| **Total** | **42** | pinned in `public::NUM_PUBLIC_INPUTS` |

The verifier binds these via:

1. AIR constraints that the trace's "public-input rows" carry these
   exact values.
2. Hardness check: outside the AIR (in `verify`), confirm
   `found_leaf <= 2^(256 − b) · r · t^2` interpreted as little-endian
   U256.

## 5. Witness columns

Private side, not seen by the verifier. Pulled from
`MatmulProof.found.{a_rows, b_cols}` + the prover's reconstructed
`BlockNoise` at the call boundary in `ai-pow`.

| Witness | Shape | Source |
|---|---|---|
| `a_rows` | `tile × k` i8 in [-64, 64] | `MatmulProof.found.a_rows` |
| `b_cols` | `tile × k` i8 in [-64, 64] | `MatmulProof.found.b_cols` |
| `e_l` | `m × r` i6 in [-32, 31] | re-derived from `s_A` via Pearl PRNG (witness or recomputed inside the BLAKE3 chip) |
| `e_r_pos` | `k × 2` u32, distinct, in `[0, r-1]` | same |
| `f_r` | `n × r` i6 in [-32, 31] | re-derived from `s_B` |
| `f_l_pos` | `k × 2` u32, distinct, in `[0, r-1]` | same |
| `m_states` | `(k/r + 1) × 16` i32 | per-stripe `M`-state evolution; recorded by the prover during the in-circuit replay |
| `c_blk_states` | `(k/r + 1) × t^2` i32 | per-stripe running accumulator; same |

Witness layout is "wide" — every witness lives in its own column
slice rather than a packed row format. This trades trace area for
simpler constraint encoding. Pearl uses a similar wide layout.

## 6. Constraint counts (rough estimate)

| Region | Rows | Comment |
|---|---|---|
| BLAKE3 keyed-call rows | `~7 × C_blake3` where `C_blake3 ≈ 1024 / round_per_row` | Tip the upstream sub-AIR; ~1024 rows per compression at the default row width. |
| Per-stripe matmul rows | `(k/r) × t^2` | Each accumulator entry gets one stripe row. |
| Per-stripe jackpot rows | `k/r` | One row per stripe boundary. |
| Range-table preprocessed | `2^13` | Fixed once. |
| **Total trace rows (PROD)** | ~`2^18 – 2^19` | Padded up to next power of two for FRI. |

At `m = n = 4096, k = 4096, r = 64, t = 128` (PROD) the dominant
cost is the BLAKE3 region for `h_a` / `h_b`, each unfolding to
`4096 × 4096 / 1024 = 16384` chunks per matrix. We may push those
two BLAKE3s **outside** the SNARK in v1 (re-prove only the seed
chain + jackpot hash + matmul + jackpot fold), at the cost of
giving the verifier 64 bytes of additional public input per matrix
to bind. **TBD.**

## 7. Parameters

The `CircuitConfig` knobs live in `src/circuit.rs`.

| Parameter | `TEST` | `PROD` | Rationale |
|---|---|---|---|
| `log_blowup` | 1 | **3** | FRI blowup factor in log-2 terms (i.e. rate `1 / 2^log_blowup`). `PROD = 3` means rate `1/8`: the committed evaluation domain is 8× the trace length. Larger blowup → more provable soundness per query, at the cost of a larger committed trace. |
| `num_queries` | 8 | **80** | At `log_blowup = 3` and `pow_bits = 0`, **provable** soundness is `num_queries · log_blowup / 2 = 80 · 3 / 2 = 120` bits. |
| `pow_bits` | **0** | **0** | We intentionally do **not** use FRI proof-of-work grinding. Each query has to carry its share of the soundness budget on its own. Keeps the prover simpler and the security analysis a clean function of `(log_blowup, num_queries)`. |

### Security model

We use the **provable** FRI soundness bound, not the list-decoding /
capacity-approaching conjecture:

> A FRI proof at log-blowup `r` and `q` queries gives at least
> `r · q / 2` bits of soundness against a malicious prover, in the
> standard unique-decoding regime. No additional cryptographic
> conjecture is assumed.

At `PROD = (log_blowup = 3, num_queries = 80, pow_bits = 0)` that
gives **120 bits of provable soundness**.

This is the conservative choice. The conjectured bound at the same
parameters would be `r · q = 240` bits — but that relies on the
list-decoding-capacity conjecture, which we don't depend on here.
`pow_bits = 0` keeps grinding out of the picture entirely; with
grinding enabled it would add `pow_bits` bits additively to the
above formula.

## 8. What's not in v1

- **Recursion.** Pearl uses three Plonky2 recursion layers to
  compress proofs to ~60 KB. We ship a raw uni-STARK at ~150–300 KB
  first. Recursion comes in a follow-up commit once the AIR
  stabilizes.
- **zkSNARK privacy.** Pearl's circuit is zero-knowledge so miner
  inputs `A, B` stay private. uni-STARK is *not* zero-knowledge by
  default; the strips remain in the witness. Adding ZK via a
  blinding round happens in a separate phase.
- **`h_a` / `h_b` inside the circuit.** As noted in §6, these may
  ship outside the circuit in v1 if the BLAKE3-chip cost is the
  dominant trace area. The decision will be made empirically once
  the trace generator is real.
- **GPU prover.** Plonky3 has GPU support (`p3-cuda-fri`, etc.) but
  we're targeting CPU first.

## 9. Integration with `ai-pow`

In `ai-pow/src/prover.rs::mine_with_context`, right where Pearl
invokes `zk_prove_plain_proof` (see
`pearl/zk-pow/src/api/prove.rs:18`), an `#[cfg(feature = "zk")]`
block builds a `ZkParams` from `MatmulParams` and (once
implemented) calls `ai_pow_zk::prove(...)` with the public/private
split extracted from the plain proof. The call shape is already
commented in at the call site — see
`crates/ai-pow/src/prover.rs::mine_with_context`'s `#[cfg(feature
= "zk")]` arm.

The plain proof is still returned alongside; the caller decides
whether to ship the plain witness or the SNARK as the block
certificate (the chain only commits to one).

## 10. References

- `pearl/zk-pow/src/api/prove.rs::zk_prove_plain_proof` — Pearl's
  prove entry. The shape of `ai_pow_zk::prove` matches.
- `pearl/zk-pow/src/circuit/pearl_air.rs` — the four-chip
  composition we mirror.
- `pearl/zk-pow/src/circuit/blake_program.rs` — Pearl's BLAKE3
  AIR. We use `p3-blake3-air` instead.
- `pearl/zk-pow/src/circuit/pearl_stark.rs::PearlStark` — the STARK
  struct binding everything. Our analog lives in
  `src/circuit.rs::build_stark_config`.
- [Plonky3 README](https://github.com/Plonky3/Plonky3) — current
  sub-crate inventory.
- [Tip5 paper](https://eprint.iacr.org/2023/107) — sponge design
  rationale.
