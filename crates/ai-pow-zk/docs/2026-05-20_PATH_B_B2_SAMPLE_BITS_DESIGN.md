# Path B Stage B2 — `sample_bits` waste-bit elimination — design analysis

**Date:** 2026-05-20
**Status:** Design complete. Implementation attempt in progress.

## Problem statement

`recursion/src/challenger/circuit.rs:419-423`:

```rust
fn sample_bits(...) {
    let bf_bits = BF::bits();  // 64 for Goldilocks
    let base_sample = self.sample(circuit);
    let bits = circuit.decompose_to_bits::<BF>(base_sample, bf_bits)?;
    Ok(bits[..num_bits].to_vec())
}
```

`decompose_to_bits` bool-checks + sum-reconstructs all 64 bits.
Caller throws away (64 − num_bits) of them. Profiled cost:
1,920 bool_checks + 1,920 mul_adds across ~30 calls = the
single dominant Alu contributor (58% of profiled ops).

## Design space (8 alternatives considered)

### Design 1 — Modular reconstruction with single `high_witness` ★ RECOMMENDED

```rust
fn decompose_to_low_bits_with_high_witness<BF>(x, num_bits) -> Vec<bits> {
    // Hint: num_bits boolean low_bits + 1 unconstrained high_witness.
    // Assert: Σ low_bits[i] · 2^i + 2^num_bits · high_witness = x.
    // Return: low_bits.
}
```

**Saving:** per call, (bf_bits − num_bits) bool_checks +
(bf_bits − num_bits − 1) mul_adds.
- Goldilocks (bf_bits=64), num_bits=10: 54+53 = 107 ops saved.
- Profiled: ~30 calls × ~107 = **~3,210 Alu ops saved**.
- L1: ~2-3% cell reduction (1620 ops / 8 lanes × 181 cols ≈ 36 KB).
- L2 cascade: ~5% per Phase-0 amplification factor.

**Soundness:**
- `base_sample` is challenger-bound (prover can't lie).
- `low_bits` are bool-checked.
- Modular reconstruction enforces `Σ low · 2^i ≡ base_sample
  (mod 2^num_bits)`.
- `high_witness` is unconstrained but unused by downstream
  consumers (FRI query indices + PoW bit checks use only
  the low bits).
- Equivalent to existing for the FRI verifier.

**Risk:** medium — new primitive in soundness-critical
CircuitBuilder. KAT-first mandatory.

### Design 2 — Goldilocks 32-bit-limb specialization

Goldilocks `p = 2^64 − 2^32 + 1` ≈ 2^64; natural 32-bit-limb
representation. **But:** the modular constraint in Design 1 is
already optimal. A 32-bit-limb hint adds NO ops over Design 1.

**Verdict:** SKIP. Goldilocks structure doesn't reduce op count
beyond Design 1.

### Design 3 — Lazy bool-check via range proof

Sample single field witness; prove `0 ≤ low_compact < 2^num_bits`
via LogUp range table.

**Problem:** Table for num_bits=20 = 2^20 = 1M rows. Chunked
range checks bring bool_checks back via byte boundaries.

**Verdict:** SKIP. Not a clean win.

### Design 4 — Batched sample_bits across queries

Sample fewer field elements; distribute bits.

**Problem:** Breaks FRI query independence required for
soundness.

**Verdict:** SKIP. Unsound.

### Design 5 — `mul_add` deduplication for zero bits

When `b=0`, `acc + b·pow2 = acc`. But verifier doesn't know
prover-side values.

**Verdict:** SKIP. Not applicable in-circuit.

### Design 6 — Packed high_witness

Identical to Design 1 framed differently.

### Design 7 — Bool-check lane packing

Already happening via `alu_lanes=8`. The 1920 bool_checks
pack into ~240 Alu rows. Reducing bool_check count
proportionally reduces rows.

**Verdict:** Already optimized; works WITH Design 1, not
instead of.

### Design 8 — Goldilocks split-table specialization

`x = lo + hi · 2^32` for `lo, hi ∈ [0, 2^32)`. Hint two
32-bit witnesses + bool-check via byte-range table.

**Analysis:** For num_bits ≤ 32, only `lo` matters; `hi`
becomes the unconstrained high_witness. Reduces to Design 1.

For num_bits ≤ 16 (common for FRI): hint 16 bool-checked
bits + 1 high_witness covering bits 16..64. Identical to
Design 1.

**Verdict:** SKIP. Same as Design 1 in practice.

## Production call sites (3, all candidates for the swap)

| Site | Call | num_bits source |
|---|---|---|
| `recursion/src/pcs/fri/targets.rs:795` | `challenger.sample_bits(circuit, log_max_height)` | runtime (FRI query index) |
| `recursion/src/pcs/fri/targets.rs:1185` | `challenger.sample_bits(circuit, log_max_height)` | runtime (FRI query, alt variant) |
| `recursion/src/challenger/circuit.rs:440` | `sample_bits(circuit, witness_bits)` | runtime (PoW witness check) |

All three would benefit from the swap. No compile-time
specialization possible (num_bits is runtime).

## Implementation plan

1. **Add a `LowBitsPlusHighHint` variant** alongside the
   existing `BinaryDecompositionHint` in
   `circuit/src/builder/hints.rs` (or equivalent).
   - Takes input `x: Vec<F>` + parameter `num_bits`.
   - Emits `num_bits` boolean witnesses + 1 high_witness.

2. **Add `CircuitBuilder::decompose_to_low_bits_with_high_witness`**
   in `circuit/src/builder/circuit_builder.rs`.
   - Hint: `LowBitsPlusHighHint::new(num_bits)`.
   - Bool-check + sum-reconstruct only the low bits.
   - Assert modular: `Σ low_bits · 2^i + 2^num_bits ·
     high_witness = x`.

3. **Update `RecursiveChallenger::sample_bits`** to call the
   new primitive instead of `decompose_to_bits(bf_bits)`.

4. **KAT tests** in
   `circuit/src/builder/circuit_builder.rs` `#[cfg(test)]`:
   - **Test 1 (ACCEPT):** valid decomposition verifies on
     known values.
   - **Test 2 (TAMPER):** corrupting a low_bit causes
     verification to reject.
   - **Test 3 (TAMPER):** corrupting the high_witness in a
     way that violates modular constraint rejects.
   - **Test 4 (cross-check):** for values `< 2^num_bits`,
     the new primitive produces the same low_bits as the
     existing `decompose_to_bits`.

5. **Full regression** at every layer:
   - `cargo test -p p3-circuit --release`
   - `cargo test -p p3-recursion --release --tests`
   - `cargo test -p p3-circuit-prover --release --tests`
   - Stage 4+5 (Tip5-throughout L2-over-L1 ACCEPT +
     tamper-REJECT + size measurement)

6. **Stage 5 re-measure** with the swap landed; verify
   predicted ~2% L1 + ~5% L2 reduction.

## Honest residuals after implementation

- **L2 cascade prediction is ~5% based on Phase-0
  amplification factor.** Actual will be ±50% of prediction.
- **Other reductions in B1 (Family A.1, D) remain residual.**
- **`opening_proof` 85% slice opacity** unchanged — this
  reduction targets Alu cells which contribute mostly to
  `opened_values` (13.1% of L1), not `opening_proof`.

## Cross-references

- B0 inventory: `2026-05-20_PATH_B_STAGE_0_COLUMN_INVENTORY.md`
- B1 reduction map: `2026-05-20_PATH_B_STAGE_1_REDUCTION_MAP.md`
- Phase B status: `2026-05-20_PATH_B_STATUS.md`
