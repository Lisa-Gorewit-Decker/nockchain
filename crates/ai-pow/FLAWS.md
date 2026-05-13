# Flaws in `crates/ai-pow` vs. the Pearl Whitepaper and the cuPOW paper

Evaluated against:
- `Pearl_Whitepaper.pdf` (repo root) — concrete INT8 L1 PoUW design.
- `2504.09971v4.pdf` (repo root) — Komargodski & Weinstein, "Proofs of Useful Work from Arbitrary Matrix Multiplication" (the **cuPOW** scheme, esp. Algorithm 6.4 and §6.5).

Citations below reference both papers and the crate's source files.

---

## Load-bearing flaws (break the PoUW property)

### F1. `A, B` are **not** miner-chosen — they are derived from `(block_commitment ‖ nonce)`

- Pearl §3, §4.1, §4.3 (Alg. 2): the prover commits to miner-supplied matrices `A` and `B` (e.g., LLM weights and activations). `H_A` and `H_B` are BLAKE3 Merkle commitments over row-major `A` / column-major `B`; the noise seeds `s_A, s_B` are derived from these commitments.
- cuPOW §6 / §2 (overview): the prover picks `A, B` "at her own discretion / application" — the whole point of usefulness is that the prover's *real* workload becomes the puzzle.
- Current code (`crates/ai-pow/src/matmul.rs:67-89`): `A` and `B` are expanded from `state = block_commitment ‖ nonce` via BLAKE3-XOF. They are pseudo-random throwaways. The matrix product `A · B` is computed tile-by-tile but never returned, never recoverable, never useful.

**Consequence:** the "two-for-one" / "useful work" claim of cuPOW and Pearl does not hold for this crate. Mining produces no AI artifacts.

**Scope of fix:** large — requires changes to the block format, mempool, and RPC to ship miner-supplied `A, B` and their commitments. **Documented here; out of scope for the in-crate cleanup. Tracked separately.**

### F2. Noise is **full-rank**, not low-rank `E = E_L · E_R`

- Pearl §3.1.1, §4.4 (Alg. 3): `E := E_L · E_R` where `E_L ∈ [-32, 31]^{m×r}` (int6) and `E_R ∈ {-1, 0, 1}^{r×k}` is a **choice matrix** — every column has exactly one `+1` and one `-1` at uniformly random distinct positions. Same structure for `F = F_L · F_R`. Low rank `r` is essential for both:
  - **Efficiency:** the peel step `A · F + E · (B + F)` costs `O(n² r)` per term (low-rank product), making it asymptotically negligible vs. the `O(n³)` matmul.
  - **Hardness:** cuPOW Assumption 6.4 conjectures `Ω(n^{ω_r+1}/r)` for computing the full r×r tile transcript of two random rank-r matrices. Without rank-r structure, the conjecture is vacuous.
- Current code (`crates/ai-pow/src/prng.rs:36-46`, `matmul.rs:22-43`): `E ∈ i8^{m×k}` and `F ∈ i8^{k×n}` are drawn **full-rank** as uniform `i8` in `[-128, 127]`. The `noise_rank` field on `MatmulParams` exists and is mixed into `params_tag` but **never** influences noise generation.

**Consequence:** hardness reduces to plain random-oracle hashing; the cuPOW conjecture provides no security separation over hashcash.

**Scope of fix:** contained — rewrite `BlockNoise` generation and add per-row/per-col `e_row(i)`, `f_col(j)` accessors that synthesize from the low-rank factors. The downstream matmul loop is unchanged.

### F3. Tile state is **not** step-bound — it's a one-shot BLAKE3 of the final partial sum

- Pearl §4.5 (Alg. 4): per-tile state `M ∈ {0,1}^{512}` (sixteen `int32`s) is updated **at every k-axis accumulation step**. At step ℓ < ⌊k/r⌋, the prover computes the int32 XOR of all entries in the current accumulator tile `C_blk`, then folds it in with rotation:
  ```
  M[ℓ mod 16] ← (M[ℓ mod 16] ≪ 13) ⊕ X
  ```
  Final hardness check is `BLAKE3(M, key=s_A) ≤ 2^(256-b) · r · t_m · t_n`.
- Current code (`crates/ai-pow/src/tile_hash.rs:11-18`, `matmul.rs:257-269`): `dot_axby` computes the full int32 dot product in one pass, and `tile_state_hash` then hashes the final `t × t` partial-sum block once. There is no iterative state; intermediate accumulator values along the k-axis are never observed.

**Consequence:** a miner who can predict or shortcut to the *final* int32 dot product (via FMM, sparsity, or any other trick) skips all of the matmul work without changing `M`. The protocol cannot tell the difference between "ran the canonical algorithm" and "guessed the output."

**Scope of fix:** contained — change `compute_tile` into a step-by-step accumulator over `⌊k/r⌋` r-stripes, and replace `tile_state_hash` with a `TileAccumulator` that owns the 512-bit `M` state and folds each step.

---

## Smaller deltas

### F4. Difficulty target is not shape-aware

- Pearl §4.5: target is `2^(256-b) · r · t_m · t_n` where `b` is the logarithmic difficulty.
- Current code (`crates/ai-pow/src/tile_hash.rs:33-43`): a flat 256-bit big-endian comparison `H ≤ target`. The target doesn't track tile shape, so it can't be compared across different `(r, t_m, t_n)`.

**Fix:** add a `difficulty_bits: u32` field; build the target at hardness-check time from `(b, r, t_m, t_n)`.

### F5. Input ranges are wider than Pearl

- Pearl §4.1 (mining config) / §4.2: `A, B ∈ [-64, 64]`, `E_L ∈ [-32, 31]` (int6). This keeps `(A+E)·(B+F)` bounded by `(64+32)² = 9216` per multiply, so `k ≤ 2^16` fits int32.
- Current code (`crates/ai-pow/src/prng.rs:14-22`, `params.rs:104-107`): `A, B, E, F` all uniform i8 in `[-128, 127]`. To keep accumulator in int32 the crate has to enforce `k < 2^15`, which is tighter than Pearl.

**Fix:** mask expanded `A, B` rows/cols into `[-64, 64]`; `E_L` into `[-32, 31]`; `E_R` via the choice-matrix procedure. Relax the `k` cap to `2^16`.

### F6. `lambda` is decorative

- `MatmulParams.lambda` (`params.rs:19`) is mixed into `params_tag` and otherwise never read. `spot_checks` is the parameter actually doing security work.

**Fix:** delete `lambda`.

### F7. `noise_rank` will be wired up by F2

Will become a real parameter once F2 is implemented.

### F8. No `A, B` commitments

Downstream of F1. Pearl §4.3 / §4.6 require BLAKE3 Merkle roots over rows of `A` (row-major) and columns of `B` (column-major), with block-opening proofs revealing only the relevant strips. Cannot be implemented in this crate until F1 lands.

### F9. No zk-SNARK block opening

Pearl §4.7: Plonky2-based zkSNARK compresses block opening from MB to ~60 KB and hides miner inputs. Current `lib.rs` doc-string is explicit that this is out of scope ("No SNARK / STARK is used"). Noted, not "fixed."

---

## Plan of attack

| Flaw | Status | Reason / location |
|---|---|---|
| F1 (miner-chosen A,B) | **Fixed** | `prover::mine(block, nonce, a, b, params, opts)` — caller supplies `A, B`. `BlockContext::build` validates shape + range and runs the Pearl commitment chain. |
| F2 (low-rank noise) | **Fixed** | `prng.rs`, `matmul.rs::BlockNoise` |
| F3 (iterative tile state) | **Fixed** | `matmul.rs::TileState::fold`, called from `compute_tile` |
| F4 (shape-aware target) | **Fixed** | `tile_hash.rs::difficulty_target`, threshold = `2^(256-b) · r · t^2` |
| F5 (Pearl input ranges) | **Fixed** | `prng.rs`: A,B ∈ [-64,63], E_L,F_R ∈ [-32,31]; verifier range-checks every opened strip |
| F6 (delete `lambda`) | **Fixed** | Removed from `MatmulParams`; `difficulty_bits` is the only difficulty parameter |
| F7 (wire `noise_rank`) | **Fixed** | Now load-bearing: rank of E/F, accumulator stripe width, threshold weight |
| F8 (commitments to A,B) | **Fixed** | Per Pearl §4.3 Alg. 2: `κ = derive_key("kappa", block ‖ params_tag)`, `H_A = MerkleRoot({a_row_leaf_hash(A_i, κ)})`, `H_B = MerkleRoot({b_col_leaf_hash(B_j, κ)})`, `s_B = derive_key("s_b", κ ‖ H_B)`, `s_A = derive_key("s_a", s_B ‖ H_A)`. Proof carries strip openings with Merkle paths to `H_A`/`H_B`; verifier authenticates each strip before reconstructing the tile state. |
| F9 (zk-SNARK) | Open | Pearl §4.7 — Plonky2 block-opening proof; separate work track |

### Pearl protocol compliance after this commit

The crate now implements Pearl Whitepaper §3 + §4 end-to-end:

1. **Commitment hash** (§4.3 Alg. 2) — `commitment_key`, `a_row_leaf_hash`, `b_col_leaf_hash` build the row/column Merkle commitments; `noise_seed_b`/`noise_seed_a` chain the seeds.
2. **Matmul framework** (§4.2) — `Matrices::build` produces `A' = A + E` and `B' = B + F` from caller `A, B` plus the noise factors.
3. **Noise generation** (§4.4 Alg. 3) — `BlockNoise::expand` keyed independently by `s_A` (for `E`) and `s_B` (for `F`), with `E_L, F_R ∈ [-32, 31]` and `E_R, F_L` choice matrices.
4. **Tiled matmul with accumulated hash check** (§4.5 Alg. 4) — `compute_tile` walks `⌊k/r⌋` stripes, folding each accumulator XOR into the 512-bit `M` state with `(M[step mod 16] ≪ 13) ⊕ X`.
5. **Block opening proof** (§4.6) — `MatmulProof` carries `H_A`, `H_B`, the found tile, and σ Fiat-Shamir spot-check tiles. Each `TileOpening` includes the row strips, column strips, the per-strip Merkle paths to `H_A`/`H_B`, and the tile-state Merkle path to `comm_m`.

The only remaining gap to Pearl is **F9: the Plonky2 zkSNARK block-opening proof**, which compresses the multi-MB strip openings to a ~60 KB proof and hides miner inputs.

#### Pearl deviations worth flagging

- **Per-nonce search**: Pearl proper has no nonce — each `(block, A, B)` is one attempt. This crate keeps a Bitcoin-style nonce loop via `pow_key = derive_key("pow-key", s_A ‖ nonce)` so the matmul + M-state computation amortizes across many cheap retries. The Pearl hardness analysis is unaffected because the keyed-BLAKE3 output is uniform per `pow_key` regardless of how `pow_key` is derived.
- **Input range**: A, B sampled as int7 `[-64, 63]` instead of Pearl's `[-64, 64]` (129 values), to avoid rejection sampling in the test synthesizer. The verifier accepts the full Pearl range `[-64, 64]`; only the test synthesizer is narrower.
- **Square tiles only**: `tile_m = tile_n = tile`. Pearl supports rectangular tiles via a 3-D arithmetic progression (§4.8); this can be added when needed.

### Verifying the fixes
- `cargo test -p ai-pow` — 97 tests pass (52 unit + 19 adversarial + 13 end-to-end + 5 block-noise + 5 LLM-shape + 3 soundness).
- Adversarial coverage for F1+F8 includes:
  - `reject_tampered_h_a` / `reject_tampered_h_b` — verifier rejects when the proof's matrix-commitment roots are wrong.
  - `reject_tampered_a_row_bytes` / `reject_tampered_b_col_bytes` — flipping any byte of an opened strip breaks the leaf hash and fails its Merkle path.
  - `reject_tampered_a_row_path` / `reject_tampered_b_col_path` — direct path tampering rejected.
  - `reject_a_row_out_of_range` / `reject_b_col_out_of_range` — values outside `[-64, 64]` rejected.
  - `reject_wrong_a_strip_length` — wrong-sized strip rejected.
  - `rejects_wrong_input_shape` / `rejects_out_of_range_input` — prover-side input validation.
- End-to-end coverage:
  - `different_a_yields_different_proof` — `H_A` changes when `A` changes, confirming the commitment chain binds to inputs.
  - `mine_block_preserves_per_nonce_diversity` — `H_A`/`H_B` stay fixed across nonces (block-level), only the per-nonce parts vary.
  - `block_level_commitments_are_block_scoped` — `H_A`/`H_B` differ across different `block_commitment` values for the same `(A, B)` because `κ` depends on the block.
