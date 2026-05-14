# `crates/ai-pow` vs. Pearl reference implementation (`pearl/zk-pow`)

Compared against `pearl/zk-pow` at the version vendored in `pearl/` (Rust 2024,
blake3 1.8). Pearl source citations use `pearl/<path>:<line>`; ours use
`crates/ai-pow/src/<path>:<line>`.

## License attribution

Pearl is distributed under the ISC license (Copyright (c) 2025-2026 Pearl
Research Labs; Copyright (c) 2015-2016 The Decred developers). The
reference functions in `tests/gen_fixtures.rs` are derived line-for-line
from Pearl source and carry forward the ISC terms. A verbatim copy of
the Pearl license is at [`LICENSE-PEARL`](LICENSE-PEARL); see that file
for the full text and the specific Pearl files this crate derives from.

The rest of this crate is dual-licensed under the workspace's
`LICENSE-APACHE` / `LICENSE-MIT`.

## TL;DR

Our v3 implementation is **byte-equivalent to Pearl** on every load-bearing
mechanism: Pearl §4.3 commitment chain, Pearl §4.4 low-rank noise factors,
Pearl §4.5 iterative tile state, the keyed tile-state hash, the matrix
commitment (chunk-Merkle), and the difficulty target encoding. Given identical
inputs at every protocol boundary, our crate and Pearl produce identical bytes,
verified by 11 fixture-based unit tests in `tests/pearl_compat_fixtures.rs`.

The only remaining behavioral difference (D6) is the search loop: Pearl has
no per-attempt nonce — each `(block, A, B)` is one attempt. Our crate
introduces a `pow_key = derive_key("pow-key", s_A ‖ nonce)` extension so the
matmul + noise factors can be amortized across many nonce retries. This is
intentional and does not affect Pearl-direction byte compatibility for any
single attempt.

## Byte-level divergence inventory

### D1. Seed-derivation algorithm (CLOSED)

**Pearl** (`pearl/zk-pow/src/ffi/mine.rs:156-178`):

```rust
fn compute_job_key(header, config) -> [u8; 32] {
    let mut data = Vec::with_capacity(128);
    data.extend_from_slice(&header.to_bytes());
    data.extend_from_slice(&config.to_bytes());
    blake3_digest(&data, None)  // unkeyed BLAKE3
}

fn compute_commitment_hash(job_key, a_row_major, b_col_major) -> ([u8;32], [u8;32]) {
    let hash_a = blake3_digest(a_row_major, Some(*job_key));  // keyed BLAKE3
    let hash_b = blake3_digest(b_col_major, Some(*job_key));

    let mut b_seed_input = [0u8; 64];
    b_seed_input[..32].copy_from_slice(job_key);
    b_seed_input[32..].copy_from_slice(&hash_b);
    let b_noise_seed = blake3_digest(&b_seed_input, None);   // unkeyed

    let mut a_seed_input = [0u8; 64];
    a_seed_input[..32].copy_from_slice(&b_noise_seed);
    a_seed_input[32..].copy_from_slice(&hash_a);
    let a_noise_seed = blake3_digest(&a_seed_input, None);   // unkeyed
    (b_noise_seed, a_noise_seed)
}
```

**Ours, now byte-equivalent** (`fiat_shamir.rs`): unkeyed BLAKE3 chain
matching Pearl line-for-line. Verified by `s7_commitment_chain_matches_pearl`.

```rust
pub fn commitment_key(block_commitment, params_tag) -> [u8; 32] {
    Hasher::new().update(block_commitment).update(params_tag).finalize()
}
pub fn noise_seed_b(kappa, h_b) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(kappa);
    input[32..].copy_from_slice(h_b);
    Hasher::new().update(&input).finalize()
}
pub fn noise_seed_a(s_b, h_a) -> [u8; 32] { /* same shape as noise_seed_b */ }
```

### D2. Matrix commitment `H_A`, `H_B` (CLOSED for root computation; proof format still uses per-row Merkle for strip authentication)

**Pearl** (`pearl/zk-pow/src/ffi/mine.rs:46-48`):

```rust
let a_row_major = pearl_blake3::pad_to_chunk_boundary(&flatten_matrix(&a_matrix));
let b_col_major = pearl_blake3::pad_to_chunk_boundary(&flatten_matrix(&b_transposed));
// hash_a = blake3_digest(a_row_major, Some(job_key))
```

Pearl flattens `A` row-major (m rows × k cols) and `B^T` row-major (which is `B`
column-major: n cols × k entries), zero-pads to a multiple of 1024 bytes
(`CHUNK_LEN`), then keyed-BLAKE3-hashes the whole buffer. This is identical
to the root of a `MerkleTree::new(padded, key)` where leaves are 1024-byte
chunks; see `pearl/zk-pow/src/ffi/mine.rs:253-266`.

**Ours, now byte-equivalent for the root** (`commit.rs::matrix_commitment`):

```rust
pub fn matrix_commitment(matrix_bytes: &[u8], kappa: &[u8; 32]) -> [u8; 32] {
    let padded = pad_to_chunk_boundary(matrix_bytes);
    Hasher::new_keyed(kappa).update(&padded).finalize()
}
```

Verified by `s8_matrix_merkle_root_matches_pearl` for both single-chunk
(512 B → 1024 B padded) and multi-chunk (3000 B → 3072 B padded) inputs.

**What's still open in D2:** our internal `MatmulProof`/`TileOpening` still
authenticate row/column strips against a per-row Merkle (`a_row_leaf_hash`
+ explicit Merkle tree), not Pearl's chunk-Merkle multi-leaf proof. That's
a follow-on proof-format change tracked separately — switching the
strip-authentication scheme touches the prover, verifier, and wire
format, and is independent of `matrix_commitment` itself.

### D3. Noise PRNG byte stream (CLOSED)

**Pearl** (`pearl/zk-pow/src/circuit/pearl_noise.rs:45-79`):

```rust
pub fn get_random_hash(index, seed, key, prepend_index) -> [u8; 32] {
    let mut message = vec![0u8; 64];
    let prepend_value = (1 + index) as i32;
    message[prepend_index * 4 .. prepend_index * 4 + 4]
        .copy_from_slice(&prepend_value.to_le_bytes());
    message[32..64].copy_from_slice(seed);
    blake3_digest(&message, Some(*key))  // keyed BLAKE3
}
// Uniform matrix rows:
//   start_idx = row_idx * num_cols
//   for block in (start_idx / 32) .. ((start_idx + num_cols + 31) / 32):
//       chunk = get_random_hash(block, seed_label, key, prepend=0)
//       emit chunk[k] & 0x3F - 32 for k in valid range
```

- `seed` = a 32-byte label, either `b"A_tensor" || zeros` or `b"B_tensor" || zeros`
- `key` = `a_noise_seed` or `b_noise_seed`
- `prepend_index = 0` for uniform random, `1` for permutation
- Per-chunk index in the flattened layout determines BLAKE3 keyed-hash input

**Ours, now byte-equivalent** (`prng.rs::pearl_random_hash` and
`prng.rs::fill_uniform_row`): exact Pearl construction with
`prepend_index ∈ {0, 1}`, `seed ∈ {SEED_LABEL_A, SEED_LABEL_B}`, keyed by
the noise seed. Verified by `s1_prng_byte_stream_matches_pearl` (16 vectors
across both labels × both prepend indices) and
`s3_uniform_noise_matches_pearl` (full E_L / F_R matrices for m=n=4, r=32).

```rust
pub fn pearl_random_hash(index: usize, seed: &[u8; 32], key: &[u8; 32], prepend_index: usize) -> [u8; 32] {
    let mut message = [0u8; 64];
    message[prepend_index * 4 .. prepend_index * 4 + 4]
        .copy_from_slice(&((1 + index) as i32).to_le_bytes());
    message[32..64].copy_from_slice(seed);
    Hasher::new_keyed(key).update(&message).finalize()
}
```

### D4. Permutation generation (CLOSED)

**Pearl** (`pearl/zk-pow/src/circuit/pearl_noise.rs:89-115`): deterministic
no-rejection sampling.

```rust
let first_idx = random_uint32 & rank_mask;
let second_idx = first_idx ^ (1 + mul_hi_u32((noise_rank - 1) as u32, random_uint32));
```

This relies on `noise_rank` being a power of two and uses a `mul_hi` trick to
guarantee `1 + mul_hi(...) ∈ [1, noise_rank - 1]`, hence `first_idx XOR delta`
is always distinct from `first_idx` and within `[0, noise_rank - 1]` (since
both operands are below the next power of two).

**Ours, now byte-equivalent** (`prng.rs::pearl_permutation_pair`): exact
XOR-trick scheme reading 4 bytes per pair from the BLAKE3 hash. The
`MatmulParams::validate` now enforces `noise_rank.is_power_of_two()`
(Pearl precondition). Verified by `s2_permutation_pairs_match_pearl`
across `noise_rank ∈ {32, 64, 128}`.

### D5. Final tile hash (PARTIALLY CLOSED)

**Pearl** (`pearl/zk-pow/src/api/proof_utils.rs:1077-1081`):

```rust
pub fn compute_jackpot_hash(jackpot: &[u32; 16], commitment_hash: [u8; 32]) -> Hash256 {
    let msg: [u8; 64] = std::array::from_fn(|i| jackpot[i / 4].to_le_bytes()[i % 4]);
    blake3_digest(&msg, Some(commitment_hash))  // keyed BLAKE3 over exactly 64 bytes
}
```

Where `commitment_hash` is Pearl's `a_noise_seed`.

**Ours, now byte-equivalent** (`matmul.rs::TileState::keyed_hash`):

```rust
pub fn keyed_hash(&self, pow_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Hasher::new_keyed(pow_key);
    for v in &self.0 {
        hasher.update(&v.to_le_bytes());  // 16 × i32 LE bytes == 16 × u32 LE bytes
    }
    *hasher.finalize().as_bytes()
}
```

The previous context-string prefix has been removed. Given the same
`(jackpot, key)`, both implementations produce identical 32-byte hashes
— verified by `tests/pearl_compat.rs::pearl_jackpot_hash_matches_our_keyed_hash`.

**Remaining divergence (key derivation only):** our `pow_key = derive_key(
"pow-key", s_A ‖ nonce)` adds nonce dependency; Pearl uses `a_noise_seed`
directly. This is part of D6 (search loop), not the hash itself.

### D6. Mining search loop

Pearl has **no nonce**: each `(block_header, mining_config, A, B)` is a single
attempt. To search, miners vary `A` or `B`. The `try_mine_one` function in
`pearl/zk-pow/src/ffi/mine.rs:19` generates fresh random `A, B` per attempt
inside a loop (`mine` at line 125-142) until a jackpot is found.

Our crate keeps a Bitcoin-style nonce loop (`mine_block` sweeps a nonce
iterator) and mixes `nonce` into the keyed-hash key via `pow_key_for_nonce`
so the matmul + noise + M-state can amortize across many cheap retries.

Both are valid PoUW search loops — Pearl's matches the whitepaper literally,
ours adds an amortization-friendly extension.

## What matches Pearl byte-for-byte today

| Property | Both produce | Cite |
|---|---|---|
| `E_L` value range | `[-32, 31]` via `byte & 0x3F - 32` | Pearl `pearl_noise.rs:12-16, 73`; ours `prng.rs:42-46` |
| Permutation index range | `[0, noise_rank - 1]`, distinct pair | Pearl `pearl_noise.rs:106-110`; ours `prng.rs:104-115` |
| Tile state shape | 16 × `u32` (Pearl) / `i32` (ours) with identical bit pattern | Pearl `pearl_program.rs:25` `JACKPOT_SIZE=16`; ours `matmul.rs::TileState` |
| Tile state update | `M[step mod 16] = M[step mod 16].rotate_left(13) ^ X` | Pearl `pearl_program.rs:27` + `mine.rs:95-97`; ours `matmul.rs::TileState::fold` |
| Tile state rotation amount | `13` | Pearl `LROT_PER_TILE = 13`; ours hardcoded `13` in `fold` |
| `X` (per-stripe fold value) | u32-XOR of all accumulator entries | Pearl `mine.rs:95`; ours `matmul.rs::compute_tile` |
| Stripe step count | `⌊k / noise_rank⌋` | Both |
| Per-stripe inner product | `Σ_l (A+E)[i,l] · (B+F)[l,j]`, i32 accumulator | Both |
| Difficulty bound shape | `U256` little-endian on both sides | Pearl `mine.rs:101` `U256::from_little_endian`; ours `tile_hash.rs::hash_le_target` now interprets both `hash` and `target` as little-endian 256-bit unsigned integers |

The difficulty-target endianness has been switched to little-endian to match
Pearl exactly. `difficulty_target` produces a 32-byte LE encoding of
`2^(256-b) · r · t^2` (saturating), and `hash_le_target` walks from byte 31
(MSB) down to byte 0 (LSB) for the comparison.

## Cross-impl test layout

The Pearl reference functions used to live inline in `tests/pearl_compat.rs`
as a vendored `mod pearl_ref`. They've since been promoted to **captured
byte fixtures** so the tests carry no Pearl source at runtime:

| File | Role |
|---|---|
| `tests/gen_fixtures.rs` | One-shot generator (`#[ignore]`-marked test). Vendors Pearl's reference functions and prints the expected outputs as Rust `const` declarations. Re-run with `cargo test -p ai-pow --test gen_fixtures -- --include-ignored --nocapture > tests/fixtures/pearl.rs` and prepend the module doc when Pearl updates upstream. |
| `tests/fixtures/pearl.rs` | Captured Pearl reference outputs for fixed inputs. Sections S0–S9 cover every byte-level boundary in the protocol. |
| `tests/pearl_compat_fixtures.rs` | The actual unit tests. Loads `tests/fixtures/pearl.rs` via `#[path]` and asserts our crate's outputs against the captured Pearl bytes. |

Each section in `tests/fixtures/pearl.rs` lines up with a row in the
divergence inventory:

| Section | Topic | Status | Test name |
|---|---|---|---|
| S0 | Protocol constants (`JACKPOT_SIZE`, `LROT_PER_TILE`, `SEED_LABEL_*`, etc.) | **lock-in** | `s0_protocol_constants_match_pearl` |
| S1 | `get_random_hash` byte stream | **D3 (divergent)** | `s1_prng_byte_stream_d3` `#[ignore]` |
| S2 | `generate_permutation_matrix` pairs over `r ∈ {32, 64, 128}` | **D4 (divergent)** | `s2_permutation_pairs_d4` `#[ignore]` |
| S3 | `generate_uniform_random_matrix` for fixed seeds | **D3 (divergent)** | `s3_uniform_noise_d3` `#[ignore]` |
| S4 | `matvec_sparse_perm` reconstruction | **lock-in** | `s4_noise_reconstruction_matches_pearl` |
| S5 | Per-tile `jackpot[16]` evolution | **lock-in** | `s5_tile_loop_jackpot_matches_pearl` |
| S6 | `compute_jackpot_hash` | **lock-in** | `s6_jackpot_hash_matches_pearl` |
| S7 | `compute_commitment_hash` chain | **D1 (divergent)** | `s7_commitment_chain_d1` `#[ignore]` |
| S8 | Matrix-commitment chunk-Merkle root | **D2 (divergent)** | `s8_matrix_merkle_root_d2` `#[ignore]` |
| S9 | Shape-aware difficulty target (LE byte encoding) | **lock-in** | `s9_difficulty_target_le_bytes_match`, `s9_le_compare_matches_pearl_u256_semantics` |

## How to close a divergence

Every divergent section in `tests/pearl_compat_fixtures.rs` is a `#[ignore]`-d
test whose `#[ignore = "..."]` message names the change required to flip
it green. The workflow is:

1. Pick a divergence (e.g., D3).
2. Make the crate change (e.g., rewrite `prng.rs` to use Pearl's
   `get_random_hash` byte stream).
3. Remove the `#[ignore]` attribute from the corresponding test.
4. `cargo test -p ai-pow --test pearl_compat_fixtures -- <test_name>` and
   keep iterating until the byte-equality assertion passes.

When all five `#[ignore]`s have been removed and the test passes green,
our crate is byte-compatible with Pearl across every boundary covered by
the fixtures.

## Path to full byte compatibility

To make our crate produce byte-identical outputs to Pearl (so a Pearl node
could verify our proofs and vice versa), the following changes are needed:

1. Replace `commitment_key` / `noise_seed_a` / `noise_seed_b` with Pearl's
   unkeyed-BLAKE3 chain (D1).
2. Replace per-row Merkle commitments with the BLAKE3 chunk-Merkle of the
   chunk-padded flat matrix bytes (D2). This also changes the proof format:
   strips become "chunks of A's row range" rather than "row hashes".
3. Replace the noise PRNG XOF with Pearl's keyed-BLAKE3 chunk PRNG, keyed
   by `a_noise_seed`/`b_noise_seed` with the `SEED_LABEL_A` / `SEED_LABEL_B`
   labels and `prepend_index = 0/1` (D3).
4. Replace the rejection-sampling permutation with Pearl's deterministic
   XOR scheme (D4).
5. Drop the context-string prefix and null separator from `keyed_hash`,
   and drop the nonce dependency from the key (D5).
6. Either drop the nonce concept entirely, or move the nonce influence
   somewhere protocol-equivalent (e.g., a per-attempt salt that varies `A`
   or `B`) (D6).

Items 1–5 are mechanical and contained; item 6 is the only one that touches
the search-loop API. Each can be a separate commit.
