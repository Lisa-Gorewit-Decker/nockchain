# `ai-pow-zk`

EXPERIMENTAL — a Plonky3 SNARK circuit for the
[`ai-pow`](../ai-pow/) tiling matmul puzzle. The role is the same as
Pearl's [`zk-pow`](../../pearl/zk-pow/): wrap the multi-MB plain proof
in a compact SNARK so it can fit in a block certificate.

**Status:** M1–M9 landed (see [`ROADMAP.md`](ROADMAP.md)). The entry
points [`prove`] / [`verify`] are wired into a real end-to-end
Plonky3 pipeline today, with an MVP-scope AIR (single tile-cell
matmul). The four building-block chips for the full Pearl protocol
all exist and are exercised individually; composing them into one
AIR is the next milestone.

## What works today

```rust
use ai_pow_zk::{prove, verify, ZkParams, PublicInputs, Witness};

let p   = ZkParams { m: 8, k: 16, n: 8, noise_rank: 2, tile: 2, difficulty_bits: 0 };
let pi  = PublicInputs { /* … */ };
let w   = Witness   { /* … */ };

let proof = prove(b"block-commit", b"nonce", &p, &pi, &w)?;
verify(b"block-commit", b"nonce", &p, &pi, &proof)?;
```

The MVP `prove` builds a `MatmulCellAir<2>` trace from the first tile
row of `A'` (`witness.a_rows[0..k]`) and first tile column of `B'`
(`witness.b_cols[0..k]`) and runs the full Plonky3 STARK pipeline
through the `AiPowStarkConfig` (Goldilocks + Tip5 sponge + FRI). The
proof attests that the dot-product accumulator was computed
correctly under the AIR transition semantics.

**Not yet bound into the proof:** `block_commitment`, `nonce`, and
the public-input hashes (`h_a`, `h_b`, `comm_m`, `found_leaf`). The
chips needed for that binding exist (M5/M7/M8); the cross-chip
composition into one composite AIR is M9.1.

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
| `lib.rs` | **M9.** Public `prove` / `verify` entries. MVP-scope; see above. |
| `air.rs` | Composite `MatmulAir` (currently a stub). The four chips above will fold into this for the full Pearl protocol. |

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

**92 unit tests pass** across nine modules:

| Module | # tests |
|---|---|
| `circuit` (M1 + M2) | 15 |
| `witness` (M4) | 14 |
| `state_chip` (M7) | 14 |
| `matmul_chip` (M6) | 12 |
| `public` (M3) | 11 |
| `input_chip` (M5) | 11 |
| lib.rs entry-point tests (M9) | 9 |
| `blake3_air` (M8) | 6 |

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
[`prove`]: src/lib.rs
[`verify`]: src/lib.rs
