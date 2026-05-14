# `ai-pow-zk`

EXPERIMENTAL — a Plonky3 SNARK circuit for the
[`ai-pow`](../ai-pow/) tiling matmul puzzle. The role is the same as
Pearl's [`zk-pow`](../../pearl/zk-pow/): wrap the multi-MB plain proof
in a compact SNARK so it can fit in a block certificate.

**Status:** M1–M11 + M10.1a + M10.1b landed (see
[`ROADMAP.md`](ROADMAP.md)). The entry points [`prove`] / [`verify`]
are wired into a real end-to-end Plonky3 pipeline that produces
*two* proofs in one envelope:

1. **Composite tile proof** (M9.1) — matmul accumulator (M6) +
   Pearl §4.5 rotate-XOR state update (M7) with `x = c_out` sign-
   extension linkage and a single-slot state chain.
2. **In-circuit BLAKE3 keyed-hash proof** (M10.1b) — proves
   `BLAKE3-keyed([m_final, 0, …, 0], pow_key) == found_leaf` in-
   circuit via a Pearl-compat vendored fork of `p3-blake3-air`
   (`src/blake3_chip/`). Byte-equivalent to
   `blake3::Hasher::new_keyed(...)`, so Pearl ↔ Nockchain
   merge-mining holds.

Public inputs are threaded through Plonky3's public-values channel
(M10) and a separate 17-element vector binds the hash proof's
inputs/outputs. The M10.1a out-of-circuit hash check stays as a
fast-path before unpacking the heavy hash proof.

PROD-profile bench (120-bit provable soundness) at the smallest
test shape: composite ≈ 9 ms prove / 23 ms verify / 136 KB; hash
leg ≈ 84 ms prove / 364 ms verify / 3.6 MB; combined ≈ 93 ms / 387
ms / 3.7 MB.

## What works today

```rust
use ai_pow_zk::{prove, verify, ZkParams, PublicInputs, Witness};

let p   = ZkParams { m: 8, k: 16, n: 8, noise_rank: 2, tile: 2, difficulty_bits: 0 };
let pi  = PublicInputs { /* … */ };
let w   = Witness   { /* … */ };

let proof = prove(b"block-commit", b"nonce", &p, &pi, &w)?;
verify(b"block-commit", b"nonce", &p, &pi, &proof)?;
```

`prove` builds a [`composite_air::MatmulTileAir<2>`] trace from the
first tile row of `A'` (`witness.a_rows[0..k]`) and first tile column
of `B'` (`witness.b_cols[0..k]`), runs the full Plonky3 STARK
pipeline through the `AiPowStarkConfig` (Goldilocks + Tip5 sponge +
FRI), and serializes via bincode. The proof attests that:

1. The per-stripe r-wide INT8 dot-product accumulator is computed
   correctly (M6).
2. The rolling 32-bit tile-state value evolves by Pearl §4.5's
   `m_out = rotate_left_13(m_in) XOR x` rule (M7).
3. The matmul-output / state-input linkage `x = c_out` holds across
   sign (two's-complement sign extension; positive *and* negative
   accumulators).
4. The state chain carries across rows (`next.m_in = cur.m_out`,
   first-row `m_in = 0`).
5. The 42-element [`PublicInputs`] vector that goes into Plonky3's
   public-values channel must match at verify time. Any tampered
   byte in the public inputs causes a Fiat-Shamir mismatch and
   rejection.
6. **M10.1a (out-of-circuit fast path)**: the trace's terminal
   tile-state value `m_final` is exposed as a public-value-bound
   element. The verifier recomputes `pow_key` from
   `(block_commitment, nonce, h_a, h_b, params_tag)` via Pearl's
   commitment chain and rejects if `BLAKE3-keyed(m_final, pow_key)`
   doesn't equal `public_inputs.found_leaf` — a cheap plain-Rust
   check that fails before unpacking the heavy hash proof.
7. **M10.1b (in-circuit)**: the same `BLAKE3-keyed(m_final, pow_key)
   = found_leaf` relation is also proved *inside* the SNARK via the
   vendored `Blake3FoundLeafAir`. Both proofs must verify and share
   the same `m_final` value across their public-value vectors.

**Still unbound (M10.1c future):** the witness matrices `a_rows` /
`b_cols` aren't tied to `h_a` / `h_b`. An adversary can run the
matmul on different matrices and still pass M10.1b as long as the
resulting leaf is below the difficulty target. The work is bound to
*some* matmul of the prover's choosing, not specifically the chain-
pinned one. M10.1c closes this with per-row in-circuit BLAKE3 +
chunk-Merkle path verification (reuses the M10.1b vendored chip).

**API constraints (MVP):** `noise_rank` must be `2` and
`k / noise_rank` must be a power of two.

## Module map

| Module | Role |
|---|---|
| [`circuit`] | Plonky3 `StarkConfig` factory. Pins the cryptographic stack — Goldilocks base field, `BinomialExtensionField<Goldilocks, 2>` for FRI challenges, `MerkleTreeMmcs` over the in-repo Tip5 sponge, `TwoAdicFriPcs`, `DuplexChallenger`. `CircuitConfig::PROD` targets 120 bits of **provable** FRI soundness (`80 queries · log_blowup 3 / 2 = 120`) — we do **not** rely on the list-decoding / capacity-approaching conjecture. `TEST` profile uses `log_blowup=1, num_queries=8, pow_bits=0` for fast round-trips. |
| [`params`] | `ZkParams` mirror of `MatmulParams` (keeps this crate standalone — no back-dep on `ai-pow`). |
| [`public`] | `PublicInputs` ↔ `Vec<Goldilocks>` codec. 42 elements: 4 × 8 hashes + 2 × `u32` (tile coords) + 8 (`found_leaf`). |
| [`witness`] | Private `Witness` ↔ `Vec<Goldilocks>` codec for `a_rows`, `b_cols`, `e_l`, `e_r_pos`, `f_r`, `f_l_pos`, `tile_states`. |
| [`input_chip`] | **M5.** `RangeAir<BITS>` — bit-decomposition range checks for u8 / u13 / i7 / i8 / i32 witness types. Plonky3 has no built-in range primitive; we use the standard boolean-bits + recomposition trick. |
| [`matmul_chip`] | **M6.** `MatmulCellAir<STRIPE>` — per-stripe `r`-wide INT8 dot-product accumulator for one `(i, j)` tile cell. Width `2 + 2·STRIPE`. Per-row constraint `c_out = c_in + Σ a·b` plus first-row `c_in = 0` and transition carry. |
| [`state_chip`] | **M7.** `StateChipAir` — Pearl §4.5 rotate-XOR-13 state update primitive: `m_out = rotate_left_13(m_in) XOR x`. Each 32-bit word bit-decomposed; XOR via the boolean identity `a ⊕ b = a + b − 2ab`. Width 67 per row. |
| [`blake3_air`] | **M8.** Wraps upstream `p3-blake3-air` and integrates it with `AiPowStarkConfig`. Exercised end-to-end with ~10k-column traces. |
| [`blake3_chip`] | **M10.1b.** Vendored fork of `p3-blake3-air` patched to populate the `flags` column (upstream leaves it all-zero, which silently disables BLAKE3 keyed mode). Byte-equivalent to `blake3::Hasher::new_keyed(...)` — see [`tests/blake3_chip_kat.rs`](tests/blake3_chip_kat.rs). |
| [`composite_air`] | **M9.1.** `MatmulTileAir<STRIPE>` — composes M6 + M7 with `x = c_out` sign-extension linkage and a single-slot state chain. The AIR `lib::prove` / `lib::verify` use for the composite proof. |
| [`found_leaf_air`] | **M10.1b.** `Blake3FoundLeafAir` wraps `blake3_chip` and adds public-input bindings for `(m_final, pow_key, found_leaf)` plus pinned constants `(counter, block_len, flags) = (0, 64, 0x1B)`. The hash AIR `lib::prove` / `lib::verify` use. |
| `lib.rs` | **M9 + M10 + M10.1a/b.** Public `prove` / `verify` entries, threading [`PublicInputs`] through Plonky3's public-values channel and producing both proofs in one envelope. |
| `tests/prod_bench.rs` | **M11.** `#[ignore]`d round-trip under `CircuitConfig::PROD` (120-bit provable soundness). Measures proof size + timing. |
| `air.rs` | Stub for the eventual full-protocol `MatmulAir` (BLAKE3 + multi-slot routing — M9.2 / M10.1). |

## Stack choices

Goldilocks base field + Tip5 sponge for FRI + `p3-blake3-air` for the
BLAKE3 sub-circuit. See [`DESIGN.md`](DESIGN.md) for the per-slot
rationale, trace column layout, public-input encoding, witness
shapes, and parameter choices.

Pulled in from upstream Plonky3 (`https://github.com/Plonky3/Plonky3`):

- `p3-air` — AIR trait
- `p3-blake3-air` — BLAKE3 sub-AIR
- `p3-challenger`, `p3-commit`, `p3-dft`, `p3-fri`, `p3-merkle-tree`,
  `p3-symmetric` — STARK config plumbing
- `p3-field`, `p3-goldilocks` — field arithmetic and base field
- `p3-matrix`, `p3-uni-stark` — trace + prover

Tip5: not upstream. We re-use Nockchain's in-repo
[`nockchain_math::tip5`](../nockchain-math/src/tip5/) (7-round
variant, `STATE_SIZE=16`, `RATE=10`, `CAPACITY=6`, `DIGEST=5`) via a
`Tip5Perm` adapter in [`src/circuit.rs`](src/circuit.rs).

## Security parameters

| Profile | `log_blowup` | `num_queries` | `pow_bits` | Provable soundness |
|---|---|---|---|---|
| `TEST` | 1 | 8 | 0 | ≤ 4 bits (fast round-trip only) |
| `PROD` | 3 | 80 | 0 | **120 bits** (`80 · 3 / 2`) |

The soundness math uses the **unique-decoding** regime (provable):
`num_queries · log_blowup / 2` bits. We do **not** rely on the
list-decoding / capacity-approaching conjecture for FRI soundness.

## Tests

```sh
cargo test -p ai-pow-zk
```

**133 unit tests + 7 integration KAT pass** across twelve modules,
plus **3 ignored** PROD bench tests:

| Module | # tests |
|---|---|
| `circuit` (M1 + M2) | 15 |
| lib.rs entry-point tests (M9 + M10 + M10.1a/b) | 20 |
| `witness` (M4) | 14 |
| `state_chip` (M7) | 14 |
| `composite_air` (M9.1 + M10.1a AIR side) | 15 |
| `matmul_chip` (M6) | 12 |
| `public` (M3) | 11 |
| `input_chip` (M5) | 11 |
| `binding` (M10.1a helpers) | 9 |
| `found_leaf_air` (M10.1b in-circuit binding) | 6 |
| `blake3_air` (M8) | 6 |
| `tests/blake3_chip_kat.rs` (M10.1b KAT vs `blake3` crate) | 7 |
| `tests/prod_bench.rs` (M11, ignored) | 3 |

To run the PROD bench (slow):

```sh
cargo test -p ai-pow-zk --test prod_bench --release -- --ignored --nocapture
```

Each chip's tests include:
- shape pinning (column widths, padding, OOB panics)
- end-to-end prove + verify through the real FRI stack on the `TEST` profile
- tamper detection — flipping a single trace cell or proof byte
  must cause `verify` to reject

## Where this fits in the `ai-pow` flow

`ai-pow`'s `mine` produces a plain
[`MatmulProof`](../ai-pow/src/proof.rs) containing strip data +
per-row Merkle paths. At the same point Pearl invokes
`zk_prove_plain_proof` (see
[`pearl/zk-pow/src/api/prove.rs:18`](../../pearl/zk-pow/src/api/prove.rs#L18)),
`ai-pow` calls into this crate via the `zk` feature flag.

[`circuit`]: src/circuit.rs
[`params`]: src/params.rs
[`public`]: src/public.rs
[`witness`]: src/witness.rs
[`input_chip`]: src/input_chip.rs
[`matmul_chip`]: src/matmul_chip.rs
[`state_chip`]: src/state_chip.rs
[`blake3_air`]: src/blake3_air.rs
[`composite_air`]: src/composite_air.rs
[`composite_air::MatmulTileAir<2>`]: src/composite_air.rs
[`binding`]: src/binding.rs
[`blake3_chip`]: src/blake3_chip/
[`found_leaf_air`]: src/found_leaf_air.rs
[`prove`]: src/lib.rs
[`verify`]: src/lib.rs
