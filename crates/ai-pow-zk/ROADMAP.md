# `ai-pow-zk` — implementation roadmap

The circuit is being built incrementally as a sequence of milestones.
Each milestone is a self-contained module with its own end-to-end
prove+verify tests through the real Plonky3 FRI stack (TEST profile).

## Status snapshot

| Milestone | Status | Module(s) | Tests |
|---|---|---|---|
| **M1** | landed | `circuit::Tip5Perm` adapter (in-repo 7-round Tip5 → Plonky3 `Permutation` + `CryptographicPermutation` traits, scalar + packed SIMD lanes) | (in M1+M2 = 15) |
| **M2** | landed | `circuit::AiPowStarkConfig` factory + `CircuitConfig::{PROD, TEST}` | |
| **M3** | landed | `public::PublicInputs` ↔ `Vec<Goldilocks>` codec (42 elements) | 11 |
| **M4** | landed | `witness::Witness` ↔ `Vec<Goldilocks>` codec | 14 |
| **M5** | landed | `input_chip::RangeAir<BITS>` — bit-decomposition range check | 11 |
| **M6** | landed | `matmul_chip::MatmulCellAir<STRIPE>` — per-stripe INT8 dot-product accumulator | 12 |
| **M7** | landed | `state_chip::StateChipAir` — Pearl §4.5 rotate-XOR-13 update | 14 |
| **M8** | landed | `blake3_air` — upstream `p3-blake3-air` wired through `AiPowStarkConfig` | 6 |
| **M9** | landed (MVP) | `lib::{prove, verify}` — MVP entry points around `MatmulCellAir<2>` | 9 |
| **M9.1** | next | Compose M5–M7 into one composite AIR. Single-slot regime first; full 16-slot mod-16 routing after. | |
| **M10** | next | Wire BLAKE3 sub-AIR (M8) into the composite AIR. Bind public-input hashes (`h_a`, `h_b`, `comm_m`, `found_leaf`). | |
| **M11** | next | Production-shape benchmarks. Switch to `CircuitConfig::PROD`; measure proof size + prover time at miner-realistic matmul shapes (`GEMMA_4_31B_FFN`, `QWEN_3_6_27B_FFN`). | |
| **M12** | future | Recursion ladder. Plonky3 doesn't ship a one-step compressor today; track upstream + decide between proof-of-proof-style aggregation and ad-hoc SNARK-over-STARK. | |

**Total tests today:** 92 unit tests pass across nine modules.

## M1 — `Tip5Perm` Plonky3 adapter (landed)

Wrap [`nockchain_math::tip5::Tip5`](../nockchain-math/src/tip5/) (7-
round variant, state=16, rate=10, capacity=6, digest=5) so it
satisfies Plonky3's `Permutation` + `CryptographicPermutation` traits
for `[Goldilocks; 16]`. Also impls the same traits for the SIMD-
packed lane types (`PackedGoldilocksNeon` on aarch64; AVX2 / AVX-512
gated by `cfg`), as required by `MerkleTreeMmcs`'s `P` parameter and
`DuplexChallenger`'s `GrindingChallenger` bound.

## M2 — `AiPowStarkConfig` factory (landed)

Type-alias the full STARK stack:

```text
ValMmcs        = MerkleTreeMmcs<Goldilocks::Packing, _, Tip5Sponge, Tip5Compress, 2, 5>
Challenge      = BinomialExtensionField<Goldilocks, 2>
ChallengeMmcs  = ExtensionMmcs<Goldilocks, Challenge, ValMmcs>
Challenger     = DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>
Dft            = Radix2DitParallel<Goldilocks>
Pcs            = TwoAdicFriPcs<Goldilocks, Dft, ValMmcs, ChallengeMmcs>
AiPowStarkConfig = StarkConfig<Pcs, Challenge, Challenger>
```

`build_stark_config(&ZkParams, &CircuitConfig) -> AiPowStarkConfig`
emits a config with the chosen `log_blowup`, `num_queries`,
`pow_bits`. `PROD` targets 120 bits of provable FRI soundness with
`log_blowup=3, num_queries=80, pow_bits=0`. `TEST` is for fast
round-trips: `log_blowup=1, num_queries=8, pow_bits=0`.

## M3 — Public-input codec (landed)

`PublicInputs { params_tag, h_a, h_b, comm_m, found_i, found_j, found_leaf }`
encoded as 42 Goldilocks elements: four 32-byte hashes split into
8 × u32 LE = 32 elements, plus 2 × u32 for tile coords, plus
`found_leaf` = 8 = total 42 (pinned by `public::NUM_PUBLIC_INPUTS`).
Round-trip codec with structured `DecodeError`.

## M4 — Witness codec (landed)

Private side. Encodes `a_rows, b_cols, e_l, e_r_pos, f_r, f_l_pos,
tile_states` as a flat `Vec<Goldilocks>` whose total length is a
function of `ZkParams` (`Witness::field_element_count`). i8 cells
use `v as u8 as u64` (range-checked back via M5); u32 cells go
direct; i32 cells use `v as u32 as u64`.

## M5 — Input / range chip (landed)

`RangeAir<BITS>` — per-row `[ value | bit_0 | … | bit_{BITS-1} ]`
with constraints `bit · (1 − bit) = 0` and
`value = Σ 2^i · bit_i`. Trace pads to next power-of-two with all-
zero rows. Plonky3 has no built-in range primitive; this is the
standard bit-decomposition trick. Used by all downstream chips to
gate their input cells into the declared types.

## M6 — Matmul cell chip (landed)

`MatmulCellAir<STRIPE>` — for a single `(i, j)` tile coord, walks
`⌈k / STRIPE⌉` stripes. Per row:

```text
  c_out = c_in + Σ_{l=0..STRIPE} a[l] · b[l]
```

Plus first-row `c_in = 0` and transition `next.c_in = current.c_out`.
Width = `2 + 2·STRIPE`. Composing `t × t` of these covers a full
output tile; that composition is M9.1.

Goldilocks signed-int encoding pitfall fixed: `QuotientMap<u64>::from_int`
bit-reinterprets, mapping `-5` to `2^32 − 6` rather than `p − 5`. We
use `QuotientMap<i64>::from_int` everywhere we encode signed
integers. Caught only by real prove+verify cycles — pure data tests
saw the wrong canonical form too.

## M7 — Tile-state rotate-XOR chip (landed)

`StateChipAir` — encodes the Pearl §4.5 inner update primitive:

```text
  m_out = rotate_left_13(m_in) XOR x
```

with `m_in`, `x`, `m_out` as 32-bit words. Per row (width = 67):

```text
  [ m_in | x | m_out | m_in_bits[32] | x_bits[32] ]
```

Constraints: all 64 bit columns boolean; recomposition of `m_in` and
`x`; rotate-XOR equation `m_out = Σ 2^i · (m_in_bit_{(i-13) mod 32} ⊕
x_bit_i)` with `a ⊕ b = a + b − 2·a·b` on booleans. Max constraint
degree 2 — fits `log_blowup=1` in the `TEST` profile.

The slot-selection logic (which of 16 state slots this row updates)
lives at the composition layer in M9.1.

## M8 — BLAKE3 sub-AIR integration (landed)

Wraps upstream `p3_blake3_air::Blake3Air` and pins down that it
plugs into our `AiPowStarkConfig` without modification. End-to-end
prove + verify tested with 1- and 2-hash traces; the ~10k-column
trace exercises non-trivial FRI folding through the Goldilocks +
Tip5 stack.

Wiring up the keyed-mode chaining (kappa → s_B → s_A → pow_key →
found_leaf, plus per-chunk h_a / h_b) is M10.

## M9 — `prove` / `verify` entry points (landed, MVP scope)

`prove(block_commitment, nonce, params, public_inputs, witness)
→ ZkProof` and the inverse `verify`. Validates inputs, builds a
`MatmulCellAir<2>` trace from `witness.a_rows[0..k]` /
`witness.b_cols[0..k]`, calls `p3_uni_stark::prove`, serializes the
proof via `bincode::serde::encode_to_vec`. `verify` is the inverse.

**MVP-scope limitations** (made explicit in the crate docstring):

- Only `noise_rank = 2` is supported (the const-generic `STRIPE`).
  M9.1 will dispatch on `noise_rank ∈ {2, 4, 8}`.
- `block_commitment`, `nonce`, and the hash-shaped public inputs
  are validated for shape but not bound into the proof.
- One tile cell is proved (the `(0, 0)` cell); the full
  `t × t × (k / r)` walk is M9.1.

## M9.1 — Composite AIR (next)

Fold M5 (range) + M6 (matmul) + M7 (state) into one composite
`MatmulAir` with cross-chip linkages:

- `M7.x = M6.c_out` (per stripe step)
- `M7.m_in_{slot} chain` across steps that hit the same slot
- Padding-row handling that respects both the M6 carry and the M7
  rotate-XOR (likely via a `pad_flag` selector column).

This is the protocol-realistic AIR. Public-input binding is M10.

## M10 — BLAKE3 binding (next)

Compose `Blake3SubAir` into the composite AIR so the chain
`kappa → s_B → s_A → pow_key → found_leaf` (plus `h_a` / `h_b`
chunks) runs *inside* the SNARK, anchoring the public-input hashes
to the witness. Difficulty check `found_leaf ≤ 2^(256 − b) · r · t^2`
stays outside the AIR.

## M11 — Production benchmarks (next)

Switch to `CircuitConfig::PROD` and measure prover time + proof
size at miner-realistic matmul shapes
([`GEMMA_4_31B_FFN`](../ai-pow/src/params.rs),
[`QWEN_3_6_27B_FFN`](../ai-pow/src/params.rs)). Target: ≤ 60 KB
proof, comparable to Pearl's Plonky2-based STARK after recursion.

## M12 — Recursion (future)

Plonky3 doesn't ship a one-step compressor today. Track upstream
and decide between proof-of-proof-style aggregation and ad-hoc
SNARK-over-STARK once the protocol-realistic uni-stark is landed.
