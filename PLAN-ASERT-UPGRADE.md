# Plan: ASERT Difficulty Adjustment Upgrade

## Background

### Current System (Bitcoin-style epoch DAA)
The current difficulty adjustment is a faithful port of Bitcoin's `pow.cpp` algorithm:
- Difficulty retargets every **2,016 blocks** (one "epoch")
- Within an epoch, all blocks share the **same target**
- At epoch boundaries: `new_target = old_target * actual_duration / target_duration`
- Duration is capped to 1/4x - 4x the target epoch duration (1,209,600s = 14 days)
- Timestamps use median-of-11-blocks (MTP) to resist time warp attacks

### Proposed System (ASERT / aserti3-2d)
ASERT (Absolutely Scheduled Exponentially Rising Targets) adjusts difficulty **every block** using an exponential moving average anchored to a fixed reference block:

```
next_target = anchor_target * 2^((time_delta - ideal_block_time * (height_delta + 1)) / halflife)
```

Where:
- `anchor_target` = target of the anchor block (last block under old rules)
- `time_delta` = current_timestamp - anchor_parent_timestamp
- `height_delta` = current_height - anchor_height
- `ideal_block_time` = 600 seconds (10 minutes)
- `halflife` = 172,800 seconds (2 days)

The exponential is computed via fixed-point integer arithmetic using a cubic polynomial approximation:
- `factor = A*x + B*x^2 + C*x^3 + 2^47` (then shift right by 48)
- Coefficients: A=195766423245049, B=971821376, C=5127
- Fixed-point radix: 2^16 = 65536
- The exponent is decomposed into integer `shifts` (bit shifts) and a fractional part (polynomial input)

### Why ASERT
1. **Eliminates epoch oscillations** - No more periodic difficulty swings at 2016-block boundaries
2. **Per-block adjustment** - Responds to hashrate changes within blocks, not every ~2 weeks
3. **Exponential convergence** - Mathematically smooth; for every 2 days ahead of schedule, difficulty doubles
4. **Proven in production** - Running on Bitcoin Cash mainnet since November 2020

---

## Implementation Plan

### Phase 1: New Constants and Parameters

#### 1.1 Add `halflife` to `blockchain-constants` (Hoon)

**File:** `hoon/common/tx-engine-0.hoon` (lines 36-79)

Add `halflife=@` to the `blockchain-constants` type and its bunt (default) value:
- Default: `^~(:(mul 2 24 60 60))` = 172,800 seconds
- This field sits alongside `target-epoch-duration` which remains for backward compatibility during the transition

Also add derived constants used by the ASERT algorithm:
- `ideal-block-time=600` (already implicitly `target-epoch-duration / blocks-per-epoch`, but make explicit)
- `radix=^~((bex 16))` = 65,536 (16-bit fixed-point precision)
- Polynomial coefficients as named constants:
  - `asert-coeff-a=195.766.423.245.049`
  - `asert-coeff-b=971.821.376`
  - `asert-coeff-c=5.127`

#### 1.2 Add `halflife` to `BlockchainConstants` (Rust)

**File:** `crates/nockchain/src/setup.rs` (lines 157-180)

- Add `halflife: Seconds` field to the `BlockchainConstants` struct
- Add `DEFAULT_HALFLIFE: u64 = 172800` constant
- Add the field to `new()`, `to_blockchain_constants_v0_fields()`, and `NounEncode` impl
- Add a `with_halflife()` builder method for fakenet configuration
- Update tests

#### 1.3 Add activation height constant

**File:** `hoon/apps/dumbnet/lib/consensus.hoon`

Add a constant like `asert-activation-height` (TBD value) which marks the block height at which ASERT takes effect. Below this height, the old epoch-based DAA is used.

---

### Phase 2: Anchor Block Storage

#### 2.1 Add anchor block data to consensus state

**File:** `hoon/apps/dumbnet/lib/types.hoon`

The consensus state needs to store the anchor block's parameters once the activation height is reached. Add to the consensus state (new version):
- `asert-anchor=(unit [target=bignum:bn parent-timestamp=@ height=page-number])`

The anchor block is the **last block mined under the old DAA rules** (i.e., the block at `asert-activation-height - 1`). Its:
- `target` = the target of that anchor block
- `parent-timestamp` = the timestamp of the anchor block's parent (or MTP of its parent, matching current timestamp validation)
- `height` = the anchor block's height

#### 2.2 Set anchor during block acceptance

**File:** `hoon/apps/dumbnet/lib/consensus.hoon` (`++accept-page`, lines 229-300)

When accepting a block at height `asert-activation-height - 1`, record its data as the anchor. Once set, the anchor is immutable. After sufficient chain depth, the anchor parameters can be hard-coded.

---

### Phase 3: Core ASERT Algorithm Implementation

#### 3.1 Implement `compute-asert-target` in Hoon

**File:** `hoon/apps/dumbnet/lib/consensus.hoon` (new arm, near lines 150-194)

Implement the ASERT computation using Hoon's arbitrary-precision integer arithmetic:

```
++  compute-asert-target
  |=  $:  anchor-target=@
          anchor-parent-time=@
          anchor-height=page-number:t
          eval-timestamp=@
          eval-height=page-number:t
      ==
  ^-  bignum:bignum:t
```

Algorithm steps:
1. Compute `time_delta = eval_timestamp - anchor_parent_time`
2. Compute `height_delta = eval_height - anchor_height`
3. Compute signed exponent: `exponent = ((time_delta - ideal_block_time * (height_delta + 1)) * radix) / halflife`
   - Note: this can be negative (blocks ahead of schedule), requiring signed arithmetic. Hoon atoms are unsigned, so we need to track the sign separately or use a signed-integer pattern.
4. Decompose exponent into integer `shifts` and fractional `remainder`:
   - `shifts = exponent / radix` (arithmetic right shift by 16 bits, preserving sign)
   - `remainder = exponent % radix` (always non-negative, in range [0, 65535])
5. Compute polynomial approximation of `2^(remainder/radix)`:
   - `factor = radix + ((A*remainder + B*remainder^2 + C*remainder^3 + 2^47) >> 48)`
6. Compute `next_target = anchor_target * factor`
7. Apply integer shifts:
   - If `shifts >= 0`: `next_target <<= shifts`
   - If `shifts < 0`: `next_target >>= |shifts|`
8. Remove fixed-point scaling: `next_target >>= 16`
9. Clamp: `next_target = min(next_target, max_target)` and `next_target = max(next_target, 1)`
10. Return as bignum

**Critical implementation note on signed arithmetic in Hoon:**
Hoon atoms are unsigned. The exponent `(time_delta - ideal_block_time * (height_delta + 1))` can be negative (when blocks are ahead of schedule). Implementation options:
- Track sign as a separate `?` flag and the magnitude as `@`
- Use a cell `[sign=? magnitude=@]` for the signed exponent
- Carefully handle the floor-division and modular remainder to ensure correct behavior for negative exponents

#### 3.2 Unit tests for ASERT computation

Validate against known test vectors from the BCH specification. Key test cases:
- Steady-state: blocks arriving exactly on schedule should keep the same target
- Hashrate halving: blocks taking 2x as long should double the target after one halflife
- Hashrate doubling: blocks taking 0.5x as long should halve the target after one halflife
- Edge cases: very large/small timestamps, maximum target clamping, minimum target (1)
- Negative exponent handling (blocks arriving faster than expected)

---

### Phase 4: Integrate ASERT into Target Computation Flow

#### 4.1 Modify `targets.c` update logic in `++accept-page`

**File:** `hoon/apps/dumbnet/lib/consensus.hoon` (lines 283-295)

Currently, targets are only updated at epoch boundaries:
```hoon
=.  targets.c
  ?:  =(+(~(epoch-counter get:page:t pag)) blocks-per-epoch:t)
    ::  last block of an epoch means update to target
    ...
  ::  target remains the same throughout an epoch
  ...
```

Change to:
```
=.  targets.c
  ?:  (lth ~(height get:page:t pag) asert-activation-height)
    ::  PRE-ASERT: old epoch-based logic (unchanged)
    ...existing code...
  ::  POST-ASERT: compute target for every block
  %-  ~(put z-by targets.c)
  :-  ~(digest get:page:t pag)
  (compute-asert-target anchor-target anchor-parent-time anchor-height ...)
```

After ASERT activation, **every block** gets a freshly computed target stored in `targets.c`. The epoch-based lookup logic is bypassed.

#### 4.2 Modify target validation in `++validate-page-without-txs`

**File:** `hoon/apps/dumbnet/lib/consensus.hoon` (lines 364-366)

Currently:
```hoon
?.  =(~(target get:page:t pag) (~(got z-by targets.c) ~(parent get:page:t pag)))
  [%.n %page-target-invalid]
```

This checks that a block's target matches the target stored at its parent. This logic **still works** for ASERT because after accepting the parent block, we store the ASERT-computed target for the next block at the parent's digest key. No change needed to this line, but the *value* stored at `targets.c[parent_digest]` will now be the ASERT-computed per-block target instead of the epoch-wide target.

#### 4.3 Handle `epoch-counter` field

**File:** `hoon/apps/dumbnet/lib/consensus.hoon` (lines 331-342) and `hoon/common/tx-engine-1.hoon` (lines 56-58)

After ASERT activation, the `epoch-counter` field in the block header becomes vestigial. Options:
- **Option A (Recommended):** Continue incrementing `epoch-counter` modulo `blocks-per-epoch` for backward compatibility. It no longer influences difficulty but is still validated for structural consistency.
- **Option B:** Fix `epoch-counter` at 0 for all post-ASERT blocks (requires updating validation to skip the epoch counter check post-activation).

Recommendation: **Option A** - minimal disruption, no consensus-critical field semantics change, and the field is already part of the block hash commitment.

#### 4.4 Remove epoch-start bookkeeping for post-ASERT blocks

**File:** `hoon/apps/dumbnet/lib/consensus.hoon` (lines 272-280)

The `epoch-start.c` map tracks epoch boundaries for the old DAA's duration computation. After ASERT activation, `compute-epoch-duration` is no longer called. The `epoch-start.c` bookkeeping can be skipped for post-ASERT blocks to avoid accumulating dead state:

```hoon
=.  epoch-start.c
  ?:  (gte ~(height get:page:t pag) asert-activation-height)
    epoch-start.c  ::  no-op for post-ASERT blocks
  ::  ...existing epoch tracking logic...
```

---

### Phase 5: Mining Integration

#### 5.1 Update miner candidate block generation

**File:** `hoon/apps/dumbnet/lib/miner.hoon` (`++heard-new-block`, lines 229-287)

The miner already retrieves the target from `targets.c`:
```hoon
(~(got z-by targets.c) u.heaviest-block.c)
```

Because Phase 4 ensures `targets.c` is populated correctly for every block (not just epoch boundaries), **this line requires no change**. The miner automatically picks up the ASERT-computed target.

However, verify that `new-candidate` in `tx-engine-1.hoon` correctly passes through the per-block target. Currently it does (line 44: `target-bn=bignum:bn`).

#### 5.2 No changes needed to Rust mining code

**File:** `crates/nockchain/src/mining.rs`

The Rust mining code receives the target as a noun from the Hoon kernel and passes it through. No changes needed.

---

### Phase 6: Consensus State Migration

#### 6.1 New kernel state version

**File:** `hoon/apps/dumbnet/lib/types.hoon`

Add a new `kernel-state-N` variant (e.g., `kernel-state-7`) with the updated `consensus-state` that includes the `asert-anchor` field. Add a migration from `kernel-state-6` to `kernel-state-7` in the state upgrade path.

#### 6.2 State upgrade handler

**File:** `hoon/apps/dumbnet/inner.hoon` (or wherever state upgrades are handled)

The migration function should:
- Copy all existing state
- Initialize `asert-anchor` to `~` (null)
- The anchor will be set dynamically when the activation height is reached

---

### Phase 7: Blockchain Constants Wire Format

#### 7.1 Update Hoon constants type

**File:** `hoon/common/tx-engine-0.hoon`

The `blockchain-constants` type needs the new fields. Since `tx-engine-1.hoon` wraps `tx-engine-0.hoon` and adds its own fields, the new ASERT fields should be added in `tx-engine-1.hoon`'s wrapper layer (similar to how `v1-phase` and `bythos-phase` were added).

Alternatively, if the constants are already at their final canonical values and don't need to be poked from Rust, they can be defined as computed constants in the `tx-engine.hoon` door (like `quarter-ted` is today).

**Recommendation:** Define the ASERT polynomial coefficients and `ideal-block-time` as computed constants in `tx-engine.hoon`, since they are truly fixed. Only `halflife` needs to be in `blockchain-constants` (for fakenet flexibility).

#### 7.2 Update Rust `NounEncode`

**File:** `crates/nockchain/src/setup.rs`

If `halflife` is added to `blockchain-constants`, update the `to_noun` serialization to include it in the correct position. This must match the Hoon type layout exactly.

---

### Phase 8: Testing

#### 8.1 Hoon unit tests

Add tests for the `compute-asert-target` arm:
- Known test vectors from BCH spec
- Boundary conditions at activation height
- Signed arithmetic edge cases
- Verify polynomial approximation accuracy

#### 8.2 Integration / simulation tests

- Test the transition from epoch-based to ASERT at the activation boundary
- Verify blocks mined under old rules validate correctly
- Verify the first block under ASERT rules gets the correct target
- Simulate hashrate swings and verify target convergence

#### 8.3 Regression tests

- Verify all existing consensus tests still pass
- Verify checkpoint validation still works
- Verify fakenet operation with ASERT

---

## Summary of Files Changed

| File | Changes |
|------|---------|
| `hoon/apps/dumbnet/lib/consensus.hoon` | Core ASERT algorithm, activation gating, target computation flow, anchor block recording |
| `hoon/common/tx-engine-0.hoon` | `blockchain-constants` type: add `halflife` field |
| `hoon/common/tx-engine.hoon` | ASERT derived constants (coefficients, radix, ideal-block-time) |
| `hoon/common/tx-engine-1.hoon` | Possibly add `halflife` to v1 constants wrapper |
| `hoon/apps/dumbnet/lib/types.hoon` | New state version with `asert-anchor` in consensus state |
| `hoon/apps/dumbnet/inner.hoon` | State migration handler |
| `crates/nockchain/src/setup.rs` | Rust `BlockchainConstants`: add `halflife`, update encoding |
| `hoon/apps/dumbnet/lib/miner.hoon` | No changes expected (inherits correct target from `targets.c`) |
| `crates/nockchain/src/mining.rs` | No changes expected |

## Key Design Decisions

1. **Activation by height** - Use a fixed block height for ASERT activation (clean, deterministic, no MTP ambiguity)
2. **Dynamic anchor** - Anchor block is recorded at activation time rather than hard-coded (allows the activation height to be chosen without knowing the anchor block's hash/target in advance)
3. **Signed arithmetic in Hoon** - Use `[sign=? magnitude=@]` pairs since Hoon atoms are unsigned
4. **Epoch counter preserved** - Continue incrementing for structural compatibility
5. **Halflife configurable** - Via `blockchain-constants` for fakenet testing (mainnet: 172800, testnet: 3600)
6. **No nBits compact format** - Nockchain uses full bignum targets (not Bitcoin's compact nBits), so the ASERT implementation operates directly on full-precision integers, avoiding the nBits precision loss present in Bitcoin Cash's implementation

## Risk Assessment

- **Consensus-critical change** - Any bug in the ASERT computation would cause chain splits. Extensive testing with known vectors is essential.
- **Signed arithmetic** - The most error-prone part of the Hoon implementation. Must handle floor division and modular arithmetic correctly for negative exponents.
- **State migration** - Adding the anchor to consensus state requires a state version bump. Existing nodes will need to upgrade.
- **Activation coordination** - All nodes must agree on the activation height. A phased rollout with sufficient lead time is recommended.
