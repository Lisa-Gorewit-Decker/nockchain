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
| **M10.1a** | landed | Found-leaf binding via exposed `m_final` public value + verifier-side `BLAKE3-keyed(m_final, pow_key) == found_leaf` check. AIR enforces `trace.last_row.m_out == pis[PI_M_FINAL_IDX]`; hash check happens in `verify` outside the SNARK. Closes Attack #1 (fake jackpot) at full cryptographic strength. | +17 |
| **M10.1b** | landed | **In-circuit** Pearl-compat BLAKE3 keyed-mode found-leaf binding. Vendored `p3-blake3-air` into `src/blake3_chip/` and patched the trace generator to populate `flags` (so the chip computes real keyed BLAKE3, byte-equivalent to `blake3::Hasher::new_keyed` — required for Pearl ↔ Nockchain merge-mining). `Blake3FoundLeafAir` wraps that chip and pins row-0 `(message, key, output)` to public values. `lib::prove` produces a *second* proof in the envelope; `lib::verify` runs both proofs through the same `AiPowStarkConfig`. The M10.1a out-of-circuit hash check stays as a fast-path. | +14 (6 found_leaf_air + 7 blake3_chip KAT + 1 lib tamper) |
| **M9.2** | future | Slot-routing for `step mod 16` (replaces current single-slot regime); selector-gated padding so `num_stripes` can be non-power-of-two; relax `noise_rank` from the const-generic 2 to `{4, 8, …}` via dispatch |  |
| **M10.1c** | future | Per-row in-circuit BLAKE3 + chunk-Merkle so the witness's `a_rows` / `b_cols` are constraint-bound to the chain-pinned `h_a` / `h_b`. Closes Attacks #2/#3 (matrix substitution), upgrading the SNARK from a hash-only PoW certificate to a true PoUW certificate. Reuses the M10.1b vendored chip but needs per-row keyed hashes + Merkle path AIR. |  |

**Total tests today:** 152 (133 unit + 7 BLAKE3 KAT + 9 binding + 3 ignored PROD bench).

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

## M10.1a — Found-leaf binding (landed)

Closes the headline security gap from M10's Fiat-Shamir-only binding:
without M10.1a, an adversary can claim *any* `found_leaf` value
(including one that trivially passes the difficulty check) and the
SNARK accepts it. With M10.1a, `found_leaf` must be the BLAKE3-keyed
hash of the witness's terminal tile-state value under the chain-
derived `pow_key`.

Two pieces:

1. **AIR side.** [`composite_air::MatmulTileAir`] declares
   `num_public_values = NUM_PUBLIC_INPUTS + 1 = 43`. The extra slot
   `PI_M_FINAL_IDX` carries `m_final`. A new last-row constraint
   `when_last_row().assert_eq(m_out, pis[PI_M_FINAL_IDX])` forces the
   trace's terminal `m_out` to match what the prover claims.

2. **Verifier side.** [`binding::derive_pow_key`] re-runs Pearl's
   `kappa → s_B → s_A → pow_key` chain over
   `(block_commitment, nonce, params_tag, h_a, h_b)` — matches
   `ai_pow::fiat_shamir` byte-for-byte. [`binding::compute_found_leaf`]
   then BLAKE3-keyed-hashes the single-slot `M_bytes` under that
   `pow_key`. `verify` rejects with `VerifyError::FoundLeafMismatch`
   if it differs from `public_inputs.found_leaf`.

The hash itself runs in plain Rust (out-of-circuit). The *binding* is
cryptographic: changing `m_final` breaks the AIR's last-row check;
changing `found_leaf` breaks the verifier's hash check; changing
anything `pow_key` depends on (`block_commitment`, `nonce`, `h_a`,
`h_b`, `params_tag`) changes `pow_key` and so changes the expected
leaf. Replay across blocks fails on `pow_key` divergence.

Doesn't close: matrix substitution attacks (Attacks #2/#3 from the
security writeup). Those are M10.1b.

## M10.1b — In-circuit Pearl-compat keyed BLAKE3 (landed)

Move the M10.1a out-of-circuit hash check INTO the SNARK while
preserving Pearl byte-compat (so Pearl ↔ Nockchain merge-mining
holds — miners can share matmul work between the two protocols).

**Constraint:** the in-circuit hash function must produce
byte-equivalent output to `blake3::Hasher::new_keyed(...)` for the
single-block keyed root case. Diverging the hash would break merge-
mining.

**Two-piece implementation:**

1. **`src/blake3_chip/`** — vendored fork of `p3-blake3-air`
   (Plonky3 @ af65376, MIT / Apache-2.0). Upstream's AIR already
   references `local.flags` correctly as `initial_row_3[3]`, but its
   trace generator never populates the column (leaves all-zero) AND
   hard-codes `state[3][3] = 0` in the scalar mirror — silently
   computing BLAKE3-compression-with-flags-0 instead of real keyed-
   mode BLAKE3. The vendored copy patches both:
     * `row.flags` is now written from a `flags: u32` parameter.
     * `state[3][3]` is initialised from `flags` (not `0`).
     * New `Blake3HashCall` struct + `generate_trace_for_calls` API
       expose `(counter, block_len, flags)` per row (upstream ties
       them to row index / `num_rows`).
     * Renamed `Blake3Air` → `Blake3KeyedAir` so the fork boundary
       is obvious at call sites.

   KAT tests (`tests/blake3_chip_kat.rs`, 7 tests) confirm
   byte-equivalence: the chip's scalar reference output matches the
   `blake3` crate's `Hasher::new_keyed` for all-zero / random /
   Pearl-tile-state inputs, and ai-pow-zk's M10.1a out-of-circuit
   `binding::compute_found_leaf` produces the same hash.

2. **`src/found_leaf_air.rs`** — `Blake3FoundLeafAir` wraps the
   vendored chip and adds public-input binding constraints on row 0:
   message[0] = `m_final` PI, message[1..16] = 0, chaining values =
   `pow_key` PIs, outputs = `found_leaf` PIs, plus pinned constants
   `counter = 0`, `block_len = 64`, `flags = 0x1B`. 17 public values
   total (`m_final | pow_key | found_leaf`).

   6 dedicated tests cover honest verify, tampered PIs at each slot,
   and the diagnostic case where a prover tries `flags = 0` (the
   upstream-broken value) — must reject.

**Integration in `lib::{prove, verify}`:**

`ZkProof` envelope grows to carry both the composite tile proof and
the BLAKE3 hash proof. `prove` builds both traces and produces both
proofs. `verify` runs the fast M10.1a out-of-circuit hash check
(cheap, plain BLAKE3 in Rust), then unpacks and verifies both
proofs through the same `AiPowStarkConfig`. Cross-proof consistency
comes from sharing `m_final` between the two PI vectors (verifier
builds both PI vectors from the same envelope `m_final`).

**Cost:** the BLAKE3 chip is ~10k columns wide, so the hash proof
is heavier than the composite proof. PROD-profile measurements
(release, smallest test shape):

| Proof | Prove | Verify | Bytes |
|---|---|---|---|
| Composite (M9.1 + M10.1a) | ~9 ms | ~23 ms | ~136 KB |
| Hash leg (M10.1b) | ~84 ms | ~364 ms | ~3.6 MB |
| **Combined** | **~93 ms** | **~387 ms** | **~3.7 MB** |

The hash leg dominates. M11.1 (full-shape benchmarks) will need to
explore whether folding the hash into the composite AIR (one wider
trace instead of two proofs) is cheaper.

## Future milestones

- **M9.2.** Multi-slot state routing (step mod 16) and selector-gated
  padding so `num_stripes` doesn't have to be a power of two. Also
  dispatch `STRIPE ∈ {2, 4, 8}` on `params.noise_rank`.
- **M11.1.** Full-shape benchmarks at miner-realistic matmul shapes
  ([`GEMMA_4_31B_FFN`](../ai-pow/src/params.rs),
  [`QWEN_3_6_27B_FFN`](../ai-pow/src/params.rs)). Likely needs the
  M9.2 multi-slot routing first.
