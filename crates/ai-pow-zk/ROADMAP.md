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
| **M9** | landed | `lib::{prove, verify}` — entry points wired through real Plonky3 pipeline | 9 |
| **M9.1** | landed | `composite_air::MatmulTileAir<STRIPE>` — M6 + M7 composed with `x = c_out` sign-extension linkage and single-slot state chain | 14 |
| **M10** | landed | `lib::{prove, verify}` now use the composite AIR and pass `public_inputs.to_field_elements()` through Plonky3's public-values channel (Fiat-Shamir absorption) | +3 |
| **M11** | landed | `tests/prod_bench.rs` — `CircuitConfig::PROD` round-trip + tamper detection under 120-bit provable soundness. `#[ignore]`d by default; measured proof size 136 KB at the smallest test shape | 2 |
| **M12** | deferred | Recursion / compression. Plonky3 doesn't ship a one-step compressor today; see "M12 — recursion strategy" below for the current plan |  |
| **M9.2** | future | Slot-routing for `step mod 16` (replaces current single-slot regime); selector-gated padding so `num_stripes` can be non-power-of-two; relax `noise_rank` from the const-generic 2 to `{4, 8, …}` via dispatch |  |
| **M10.1** | future | BLAKE3 binding in-circuit — compose `Blake3SubAir` alongside `MatmulTileAir` so `keyed_hash(M_final) = found_leaf` etc. are constraint-bound, not just Fiat-Shamir-bound |  |

**Total tests today:** 111 (109 unit + 2 ignored PROD bench).

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

## M9 — `prove` / `verify` entry points (landed)

`prove(block_commitment, nonce, params, public_inputs, witness)
→ ZkProof` and the inverse `verify`. Validates inputs, builds a
composite-AIR trace from `witness.a_rows[0..k]` / `witness.b_cols[0..k]`,
calls `p3_uni_stark::prove`, serialises the proof via
`bincode::serde::encode_to_vec`. `verify` is the inverse.

Originally landed against `MatmulCellAir<2>` (M6 only). After M9.1 +
M10 the AIR is the full composite `MatmulTileAir<2>` and PIs are
threaded through.

**Current API scope:**

- Only `noise_rank = 2` is supported (the const-generic `STRIPE`).
  M9.2 will dispatch on `noise_rank ∈ {4, 8, …}`.
- `k / noise_rank` must be a power of two (composite state chain
  requires power-of-two trace height).
- `block_commitment` and `nonce` are accepted but not bound; they're
  inputs the caller used upstream to derive the public-input hashes,
  which *are* bound through the Fiat-Shamir transcript.

## M9.1 — Composite tile AIR (landed)

[`composite_air::MatmulTileAir<STRIPE>`] — folds M6 (matmul cell)
and M7 (state rotate-XOR) into one AIR with cross-chip linkages:

- `M7.x = M6.c_out` via two's-complement sign extension:
  `c_out = x − 2^32 · x_bits[31]`. A single field equation covers
  both positive (bit 31 = 0 → `c_out = x`) and negative
  (`c_out_field − x_u32 ≡ −2^32 mod p`) cases. Pins the accumulator
  to fit in i32; Pearl §4.8 keeps it well inside that.
- Single-slot state chain across rows: `next.m_in = cur.m_out`,
  first-row `m_in = 0`. All writes hit "slot 0" of Pearl's 16-slot
  state. Full `step mod 16` slot routing is M9.2.
- `num_stripes` must be a power of two (no padding-vs-chain conflict).
  Selector-gated padding is M9.2.

Trace width = `(2 + 2·STRIPE) + state_chip::WIDTH` = `M6 + 67`.
Constraint degree = 2 throughout, so `log_blowup = 1` works in the
TEST profile. 14 tests including positive-only / mixed-sign /
strictly-negative accumulators, all four boundary checks, and
tampered-cell rejection at each layer.

## M10 — Public-input threading (landed)

`lib::prove` / `lib::verify` now call
`public_inputs.to_field_elements()` and pass the 42-element vector
to `p3_uni_stark::{prove, verify}` as the `&pis` argument. The
composite AIR declares `num_public_values = NUM_PUBLIC_INPUTS = 42`
so Plonky3 absorbs the PIs into the Fiat-Shamir challenger.

**Binding strength:** the AIR does not *constrain* trace values to
match specific public inputs (e.g., "the final tile state equals
`public_inputs.found_leaf`"). Plonky3's challenger absorption is
sufficient for the tamper-rejection property: a proof produced
under one set of PIs will not verify under a different set, because
the FRI query points (derived from the challenger state) differ.
3 new tests:

- `verify_rejects_mismatched_public_inputs` — change one byte in
  `h_a`, verifier rejects.
- `verify_rejects_changed_tile_indices` — change `found_i`, verifier
  rejects.
- `proof_bytes_differ_when_public_inputs_change` — prover-side
  counterpart: changing a PI changes the proof bytes, so the
  transcript is genuinely flowing through.

Stronger AIR-level binding (e.g. `assert_eq(trace.final_m, public_inputs.found_leaf)`)
is the M10.1 follow-on, after M9.2 routes the slot logic.

## M11 — PROD benchmarks (landed)

`tests/prod_bench.rs` — two `#[ignore]`d tests:

- `prod_profile_round_trip`: build a composite trace at TEST shape
  (k=16, r=2, 8 stripes), prove and verify under `CircuitConfig::PROD`
  (`log_blowup=3, num_queries=80, pow_bits=0` → 120 bits provable),
  measure timing and proof size.
- `prod_profile_rejects_tampered_witness`: same shape, mutate
  `c_out` row 0, confirm rejection at 120-bit security.

Run with:

```sh
cargo test -p ai-pow-zk --test prod_bench --release -- --ignored --nocapture
```

Baseline numbers at the smallest valid shape (k=16, r=2, 8 stripes):

| Metric | Value |
|---|---|
| Prove time (release) | ~14 ms |
| Verify time (release) | ~31 ms |
| Proof size (bincode) | ~136 KB |

The proof-size baseline is dominated by the FRI opening proofs at 80
queries × log_blowup=3. Pearl's ~60 KB target is achieved after
Plonky2 recursion; Plonky3 doesn't ship that compressor yet (see
M12).

## M12 — Recursion / compression (deferred)

Plonky3 does not ship a one-step compressor today. The two options:

1. **Wait for upstream.** Plonky3 has open WIP on a "logUp-style"
   batched verifier AIR; once that lands, our 136 KB uni-stark proof
   should fold into a smaller verifier-AIR proof. We track upstream
   via the pinned git revision in `Cargo.toml`.
2. **Ad-hoc SNARK-over-STARK.** Build a verifier-AIR by hand. Substantial
   engineering effort; deferred until upstream's path is clearer.

The honest current answer: at M11's measured 136 KB this proof is
already block-fittable for a 1 MiB block budget, so recursion is not
on the critical path for v1. We revisit when shapes scale up to
GEMMA / QWEN FFN sizes (M11.1) and proof size grows.

## Future milestones

- **M9.2.** Multi-slot state routing (step mod 16) and selector-gated
  padding so `num_stripes` doesn't have to be a power of two. Also
  dispatch `STRIPE ∈ {2, 4, 8}` on `params.noise_rank`.
- **M10.1.** BLAKE3 binding in-circuit — compose `Blake3SubAir`
  alongside `MatmulTileAir` so the chain `kappa → s_B → s_A →
  pow_key → found_leaf` (plus per-chunk `h_a` / `h_b`) runs *inside*
  the SNARK, with constraint-level binding `final_m = found_leaf`.
  Difficulty check `found_leaf ≤ 2^(256 − b) · r · t^2` stays
  outside the AIR.
- **M11.1.** Full-shape benchmarks at miner-realistic matmul shapes
  ([`GEMMA_4_31B_FFN`](../ai-pow/src/params.rs),
  [`QWEN_3_6_27B_FFN`](../ai-pow/src/params.rs)). Likely needs the
  M9.2 multi-slot routing first.
