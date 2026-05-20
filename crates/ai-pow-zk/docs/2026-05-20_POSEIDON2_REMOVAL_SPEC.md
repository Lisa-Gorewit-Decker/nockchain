> _Created **2026-05-20** · last updated **2026-05-20**._

# Poseidon2 removal spec — Tip5-unified outer-cert (M-S5b S1.B sub-stage 3.2.0)

> **Status (R1, honest).** SPEC + STAGED PLAN. **No code edit
> by this document.** Implementation spec for the hash-
> unification finding of
> `2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md` § 3.2.0:
> remove `Poseidon2Goldilocks<8>` from the L1/L2 outer-cert
> recursion stack and replace with Tip5 (matching the inner
> ai-pow-zk STARK's choice). The phased plan below decomposes
> the work into 8 stages (P0–P7) with KAT-first de-risk +
> per-stage exit gates + an explicit prerequisite for the
> C2.4 R-a tail at D=2 (`#127` / M12-tracked).
>
> **Goal.** Architectural unification → Tip5 everywhere
> (analogous to Pearl's BLAKE3-throughout). Eliminates the
> dual-hash defect identified in the routes audit § 3.2.0;
> saves ~8–12 KB opened-values + a structural max-constraint-
> degree drop from 7 (Poseidon2 x⁷ S-box) to 2 (Tip5 lookup-
> table post-L4); the degree drop further shrinks the
> quotient polynomial by ~10–15 KB. **Total predicted L2
> savings: ~18–27 KB** (40 KB → ~20–25 KB structural floor).
>
> **Critical prerequisite (R1 invariant).** The C2.4 R-a tail
> at D=2 (Tip5 recompose-coeff producer multiplicity
> imbalance; single orphan at wid 11468, per
> `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13 + CSA S0
> `2026-05-20_CONSTRAINT_INVENTORY.md` § 4.3) **must be
> resolved before P3 of this spec lands**. The outer-cert
> currently uses D=2 batch-stark; switching to Tip5 surfaces
> the R-a tail. **This is the load-bearing dependency** —
> attempting P3 without R-a tail resolution will cause
> WitnessChecks multiset imbalance and `WitnessConflict`
> rejection.
>
> **Soundness invariant.** Tip5 7-round retained throughout
> (audit-baseline per routes audit § 3.2.0b). No round
> reduction in this spec; the moderately-aggressive 5-round-
> at-outer-cert variant is documented as a follow-on, not in
> this spec's scope. Soundness chain MIN: 82 unconditional
> (FRI binds per S(−1)); AIR-side ≥98 bits per CSA — both
> sides ≥80 with margin preserved.

---

## 0. Glossary + scope

### 0.1 Glossary

- **Outer-cert L1, L2**: the recursion certificates that
  verify the inner Tip5-L0 STARK + each other; the
  `goldilocks_tip5_120bit*` Plonky3-recursion configs.
- **Inner STARK**: the ai-pow-zk production AIR (mineable
  unit); uses Tip5 7-round per `nockchain_math::tip5::permute`.
- **`Poseidon2Goldilocks<8>`**: Plonky3's Poseidon2 permutation
  instantiated at Goldilocks field × width 8. The hash being
  removed.
- **`Tip5Perm`**: the Tip5 7-round permutation as a Plonky3
  `CryptographicPermutation<[Goldilocks; 16]>` (defined at
  `crates/ai-pow-zk/src/circuit.rs:264-345`).
- **`Tip5Sponge`** / **`Tip5Compress`**: PaddingFreeSponge +
  TruncatedPermutation wrappers around Tip5Perm
  (`crates/ai-pow-zk/src/circuit.rs:176, 180`).
- **C2.4 R-a tail**: the Tip5 D=2 recompose-coeff producer
  multiplicity imbalance, single orphan at wid 11468; M12 /
  `#127` deferred per CSA S0 + C4 § 8.
- **D**: the batch-stark extension-field degree (D=1 base,
  D=2 quadratic extension, etc.). The outer-cert uses D=2.

### 0.2 Scope: what this spec does

- ✅ Specifies how to remove `Poseidon2Goldilocks<8>` from
  the L1/L2 outer-cert recursion stack.
- ✅ Specifies how to add a new `goldilocks_tip5_unified()`
  builder using Tip5 throughout (alongside the existing
  Poseidon2-based builders; additive, non-destructive).
- ✅ Specifies the staged P0–P7 implementation with KAT-first
  de-risk + per-stage exit gates.
- ✅ Specifies the test plan (parity tests + tamper tests +
  size measurements).

### 0.3 Scope: what this spec does NOT do

- ❌ Does NOT specify how to fix the C2.4 R-a tail at D=2 —
  that is M12 / `#127` work; this spec consumes its
  resolution as a prerequisite.
- ❌ Does NOT specify Tip5 round reduction (5-round-at-outer-
  cert variant from routes audit § 3.2.0b) — separate
  follow-on if size pressure justifies.
- ❌ Does NOT specify other paths from the routes audit
  (Path A SNARK wrap, Path D2 direct Plonky2 vendoring,
  Path F STIR/WHIR) — those are separate specs.
- ❌ Does NOT touch the inner ai-pow-zk STARK (fenced
  linchpin: C2.1 keystone byte-identical against `259cab2`).
- ❌ Does NOT touch the BabyBear / KoalaBear configs in
  `Plonky3-recursion/circuit-prover/src/config.rs` (they
  remain Poseidon2-based; this spec is Goldilocks-only).

### 0.4 R1 hard invariants

- **Fenced-linchpin byte-identical** against `259cab2`:
  C2.1 Tip5 perm AIR (`Plonky3-recursion/tip5-circuit-air/src/air.rs`,
  `air_lookup.rs`, `air_circuit.rs`); DT-4 duplex binding
  executor (`Plonky3-recursion/circuit/src/ops/tip5_perm/executor.rs`);
  C2.4-R-a infrastructure; recursion-verifier core
  (`Plonky3-recursion/recursion/src/verifier/**`); backend
  (`Plonky3-recursion/recursion/src/backend/fri.rs`).
- **Additive new config** — the new Tip5-unified builder
  is added *alongside* the existing Poseidon2-based
  `goldilocks_tip5_120bit()`; the latter remains compilable
  and validated for backward-compat. The production-config
  flip happens only at P5 (after parity is proven).
- **No soundness trade** — Tip5 7-round throughout
  (audit-baseline). No round reduction in this spec.
- **CRIT-1-style staged validation** — every stage commits
  with KAT acceptance + tamper-reject pair; full regression
  green per stage; never a half-landed change.

---

## 1. Current state inventory (what we're replacing)

### 1.1 Files using `Poseidon2Goldilocks<8>` at outer-cert

| File | Lines | Role |
|---|---|---|
| `Plonky3-recursion/circuit-prover/src/config.rs` | 21, 206, 226, 271, 316, 346 | `goldilocks()` + `goldilocks_tip5()` + `goldilocks_tip5_120bit()` + `goldilocks_tip5_120bit_higharity()` builders + `GoldilocksConfig` type alias |
| `Plonky3-recursion/circuit-prover/src/batch_stark_prover.rs` | 16, 58–59, 690–738, 902, 1509–1515 | `Poseidon2Prover` / `Poseidon2ProverD2` / `Poseidon2AirBuilder` registrations; `Poseidon2Config` type id |
| `Plonky3-recursion/poseidon2-circuit-air/src/{air,columns,public_types}.rs` | (entire crate) | The Poseidon2 perm AIR sub-circuit |
| `Plonky3-recursion/recursion/src/backend/fri.rs` | (Poseidon2Preprocessor / poseidon2_preprocessor) | FRI backend's Poseidon2 prover registration |

### 1.2 What stays (Plonky3-recursion's Poseidon2 keeps for other consumers)

- `Plonky3-recursion/poseidon2-circuit-air/` — the AIR
  itself stays vendored (Path G upstream-routing label per
  CSA S2 § 2.3). Other downstream users of Plonky3-recursion
  may use Poseidon2.
- `Plonky3-recursion/circuit-prover/src/config.rs:140-208`
  — `baby_bear()`, `koala_bear()`, `goldilocks()` (the
  non-Tip5 variants) keep using Poseidon2 since they're
  not on the M-S5 chain.

### 1.3 Current outer-cert config (the target of removal)

`Plonky3-recursion/circuit-prover/src/config.rs:268-294` —
`goldilocks_tip5_120bit()`:

```rust
pub fn goldilocks_tip5_120bit() -> GoldilocksConfig {
    use rand::SeedableRng;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
    let perm = p3_goldilocks::Poseidon2Goldilocks::<8>::new_from_rng_128(&mut rng);  // ← REMOVE
    let hash = PaddingFreeSponge::<_, 8, 4, 4>::new(perm.clone());                    // ← REPLACE with Tip5Sponge
    let compress = TruncatedPermutation::<_, COMPRESS_ARITY, 4, 8>::new(perm.clone());// ← REPLACE with Tip5Compress
    let val_mmcs = MerkleTreeMmcs::new(hash, compress, 3);
    // ... FRI params + challenger using `perm` ...
}
```

### 1.4 Inner STARK config (the template we'll mirror)

`crates/ai-pow-zk/src/circuit.rs:176-250` — provides the
exact Tip5-as-Plonky3-hash wrapper code we need to lift up
to the outer-cert:

```rust
pub type Tip5Sponge = PaddingFreeSponge<Tip5Perm, 16, 10, 5>;
pub type Tip5Compress = TruncatedPermutation<Tip5Perm, 2, 5, 16>;
pub type ValMmcs = MerkleTreeMmcs<
    /* P */ <Goldilocks as Field>::Packing,
    /* PW */ <Goldilocks as Field>::Packing,
    /* H */ Tip5Sponge,
    /* C */ Tip5Compress,
    /* N (arity) */ 2,
    /* DIGEST_ELEMS */ 5,
>;
pub type Challenger = DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>;
```

The outer-cert needs an analogous wrapping but compatible
with the recursion verifier's expectations. Width 16 (vs the
current Poseidon2 width 8) is the key dimension change.

---

## 2. Architectural target

### 2.1 Target outer-cert config

After P5 (production flip), the outer-cert builder becomes
something like:

```rust
pub fn goldilocks_tip5_unified_80bit() -> GoldilocksTipsConfig {
    let perm = Tip5Perm;
    let hash = Tip5Sponge::new(perm);
    let compress = Tip5Compress::new(perm);
    let val_mmcs = MerkleTreeMmcs::new(hash, compress, 3);
    let challenge_mmcs = ExtensionMmcs::new(val_mmcs.clone());
    let dft = Radix2DitParallel::default();
    let fri_params = FriParameters {
        log_blowup: 2,
        log_final_poly_len: 0,
        max_log_arity: 1,
        num_queries: 42,
        commit_proof_of_work_bits: 1,
        query_proof_of_work_bits: 1,
        mmcs: challenge_mmcs,
    };
    let pcs = TwoAdicFriPcs::new(dft, val_mmcs, fri_params);
    let challenger = DuplexChallenger::new(perm);
    StarkConfig::new(pcs, challenger)
}
```

And the type alias:

```rust
pub type GoldilocksTipsConfig = Config<
    Goldilocks,
    Tip5Perm,
    Tip5Perm,
    16,  // HASH_PERM_WIDTH (vs Poseidon2's 8)
    16,  // COMPRESS_PERM_WIDTH
    10,  // RATE (Tip5 sponge rate; vs Poseidon2's 4)
    5,   // OUT (Tip5 digest length; vs Poseidon2's 4)
    5,   // COMPRESS_CHUNK (Tip5 truncated-perm output; vs 4)
    2,   // CHALLENGE_DEGREE (unchanged)
>;
```

### 2.2 Recursion-verifier-circuit impact

The verifier circuit's challenger + MMCS subcircuits change:
- Was: Poseidon2-W8 perm AIR (~180 main cols, max degree 7).
- Becomes: Tip5 perm AIR (~9392 cols at 7 rounds, max
  degree 2 post-L4 lookup-table) — but this is already
  in the circuit at the Tip5 verifier-circuit position
  (verifying inner L0's Tip5 paths). The change *consolidates*
  two Tip5-verifier-circuit instances into one shared
  instance (the duplexing challenger AND the MMCS path
  hashing both use the same Tip5 perm AIR).

**Important quantification.** Width-16 Tip5 has more
columns than width-8 Poseidon2, but **degree 2 vs degree 7**
dominates the quotient-polynomial cost. The combined effect:
- More columns from Tip5 (potentially +5–10 KB);
- BUT eliminates the entire separate Poseidon2 perm AIR
  (-8 to -12 KB);
- AND drops max constraint degree from 7 to 2 ⇒ quotient
  polynomial shrinks (-10 to -15 KB).
- Net predicted savings: **~13–17 KB** ⇒ L2 floor 40 KB → 23–27 KB.

### 2.3 Soundness check

- Inner: 7-round Tip5 → ≥128 collision resistance + extra
  margin (Nockchain's choice; routes audit § 3.2.0b).
- Outer-cert: 7-round Tip5 → same ≥128 collision resistance.
- FRI: ≥82 unconditional Johnson per S(−1).
- Chain MIN: 82 unconditional bits — unchanged from LANDED.

**No soundness change**; the swap is hash-mechanism only.

---

## 3. Prerequisite — C2.4 R-a tail at D=2

### 3.1 The blocker

Per `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13.2 + CSA
inventory § 4.3:

> "Tip5 D=2 verifier circuit has orphaned ±1 on
> correctly-3-wide tuples because Tip5 input decompose
> produces coefficients via non-Hint witnesses (not
> registered in `hint_output_wids`), while recompose-coeff
> only provides multiplicity for Hint-derived coeffs.
> Scoped outside the T task (verifier soundness-linchpin
> not altered; M12 follow-on)."

The orphan is at wid 11468; single location; D=2 specific.
The Poseidon2 path doesn't have this issue because its
recompose-coeff dependency chain differs.

### 3.2 Why this blocks P3

P3 is the parity-test phase — proves that the new Tip5-
unified outer-cert produces equivalent proofs to the
Poseidon2-based outer-cert. If the C2.4 R-a tail orphan
exists, the WitnessChecks multiset for the new outer-cert
will be unbalanced → `WitnessConflict` at `runner().run()` →
parity test fails.

### 3.3 The fix path

Two options:

**Option 3.3.A — Resolve R-a tail at D=2** (M12 / `#127`
work; preferred):
- File: `Plonky3-recursion/tip5-circuit-air/src/air_circuit.rs:326-357`.
- Add producer multiplicity gating for non-Hint-derived
  coefficient witnesses at the Tip5 input-decompose step.
- Re-validate against D=1 (which is already byte-identical
  per `632cb8c`) and D=2 (new test case).
- Estimated effort: 3–5 days (single-orphan fix; well-
  scoped per the C3 design doc).

**Option 3.3.B — Use D=1 for outer-cert** (workaround;
acceptable if 3.3.A blocks):
- The outer-cert currently uses D=2 batch-stark for
  performance. Switching to D=1 would sidestep the R-a tail
  issue entirely.
- Cost: prover wall ~2× slower per outer layer (D=1 has
  smaller arithmetic at the cost of more base-field
  operations per quotient-polynomial work).
- Not preferred because it hurts prover performance, but
  unblocks this spec if M12 R-a tail fix slips.

**Recommendation:** pursue 3.3.A as the primary path; keep
3.3.B as a fallback if M12 timeline shifts past the M-S5b
critical path.

---

## 4. Staged plan P0–P7

### 4.0 Stage table

| Stage | What it commits | Invasive to linchpin? | Estimated effort | Prereq |
|---|---|---|---|---|
| **P0** | C2.4 R-a tail D=2 fix (or D=1 fallback decision) | Yes (Tip5-air `air_circuit.rs`) | 3–5 days | — |
| **P1** | New `GoldilocksTipsConfig` type alias + skeleton builder (compiles, doesn't run) | No (additive new code) | 1 day | P0 |
| **P2** | Builder `goldilocks_tip5_unified_80bit()` + `Tip5Sponge` / `Tip5Compress` shared between inner and outer | No (additive) | 2 days | P1 |
| **P3** | KAT parity test — prove L1/L2 verify-roundtrip at the new config matches the Poseidon2-based config byte-faithfully (modulo hash output difference) | No (test-only) | 3–4 days | P0, P2 |
| **P4** | Size measurement — actual L2 size at Tip5-unified; verify ~18–27 KB savings (target 40 KB → 23–27 KB structural floor) | No (measurement-only) | 1 day | P3 |
| **P5** | Production-config flip (CRIT-1-style staged: rename existing builder to `_legacy_poseidon2`, make `goldilocks_tip5_120bit()` an alias for the new unified builder) | Yes (config flip; staged validation) | 2 days | P3, P4 |
| **P6** | Remove legacy Poseidon2-based outer-cert builder + Poseidon2 prover registrations at the M-S5 chain (Plonky3-recursion `goldilocks*` Tip5 builders) | Yes (deletion; reversible) | 1 day | P5 |
| **P7** | Acceptance gate: full regression + tamper-reject pairs + production-size validation | No | 1–2 days | P6 |

**Total estimated effort:** 14–19 days (~3 weeks of focused
R1-disciplined work), assuming the C2.4 R-a tail fix takes
the upper end of 5 days.

### 4.1 P0 — C2.4 R-a tail D=2 fix (prerequisite)

**Goal.** Resolve the single-orphan recompose-coeff producer
multiplicity imbalance at D=2 so Tip5 can be used in the
outer-cert verifier.

**Files affected:**
- `Plonky3-recursion/tip5-circuit-air/src/air_circuit.rs:326-357`
  (the WitnessChecks CTL input-send / output-receive).
- Potentially `Plonky3-recursion/circuit-prover/src/air/recompose_air.rs`
  (the producer side) — needs analysis.

**Methodology:** per C3 § 13 R-a R1 protocol:
- DT-1 design doc first (root-cause analysis).
- KAT-first: build a minimal D=2 Tip5 perm test that
  surfaces the orphan in isolation; verify multiplicity
  balance pre-fix is `≠ 0` and post-fix is `== 0`.
- Implement the fix in `air_circuit.rs` (likely: register
  non-Hint-derived coefficient witnesses with the producer
  multiplicity at the input-decompose step).
- Validate: D=1 stays byte-identical to `632cb8c`; D=2 new
  KAT passes; full `p3-tip5-circuit-air` test suite green.

**Exit gate:**
- D=2 Tip5 perm-chain `WitnessChecks` multiset balance:
  net 0 (no orphan).
- D=1 byte-identical to `632cb8c` (regression check).
- New tamper test: synthetic D=2 perm-chain with one
  tampered coeff → `WitnessConflict` at `runner().run()`.

**R1 note.** This is a soundness-critical invasive edit
(touches the fenced C2.1-adjacent area). Per R1, staged
validation + design-doc-first. Reuse the C3 DT-1/DT-2/DT-3
diagnosis methodology from the C3 outer-cert design doc.

### 4.2 P1 — `GoldilocksTipsConfig` type alias (additive)

**Goal.** Add the new type alias + skeleton; don't replace
anything yet.

**Files affected:**
- `Plonky3-recursion/circuit-prover/src/config.rs` (additive
  at end of file).

**Changes (additive):**

```rust
// New: type alias for the Tip5-unified Goldilocks STARK config.
// Width 16 (Tip5 native; vs Poseidon2's width 8).
pub type GoldilocksTipsConfig = Config<
    Goldilocks,
    Tip5Perm,              // PermHash (was: Poseidon2Goldilocks<8>)
    Tip5Perm,              // PermCompress (was: Poseidon2Goldilocks<8>)
    16,                    // HASH_PERM_WIDTH (was: 8)
    16,                    // COMPRESS_PERM_WIDTH (was: 8)
    10,                    // RATE (was: 4)
    5,                     // OUT (was: 4)
    5,                     // COMPRESS_CHUNK (was: 4)
    2,                     // CHALLENGE_DEGREE (unchanged)
>;
```

Plus import for `Tip5Perm` (re-export from
`crates/ai-pow-zk/src/circuit.rs` OR vendor a copy of the
Tip5 wrapper code into `Plonky3-recursion/circuit-prover/src/tip5_wrapper.rs`
since `Plonky3-recursion` cannot depend on `ai-pow-zk`).

**The Tip5 wrapper question.** The current `Tip5Perm`
implementation lives in `crates/ai-pow-zk/src/circuit.rs` and
the inner uses it. But `Plonky3-recursion` is a separate
vendored workspace and cannot depend on `ai-pow-zk` (which
depends on `Plonky3-recursion`, creating a cycle).

**Resolution:** vendor a copy of the Tip5 wrapper code into
`Plonky3-recursion/circuit-prover/src/tip5_wrapper.rs`:
- `Tip5Perm` struct + `Permutation<[Goldilocks; 16]>` impl
  (mirrors `circuit.rs:264-313`).
- `Tip5Sponge` + `Tip5Compress` type aliases (mirrors
  `circuit.rs:176, 180`).
- Re-import `nockchain_math::tip5::permute` (this is
  already a dependency for the C2.1 Tip5 perm AIR via
  `Plonky3-recursion/tip5-circuit-air/src/tip5_spec.rs`).

KAT-anchor the new wrapper against `nockchain_math::tip5::permute`
on a fixture vector (same one C2.1 uses) to verify
byte-equivalence.

**Exit gate:**
- File compiles (skeleton — no actual config-builder yet).
- New tamper test: assert `Tip5Perm` wrapper at outer-cert
  produces same output as inner `Tip5Perm` wrapper on a
  known input.

### 4.3 P2 — `goldilocks_tip5_unified_80bit()` builder

**Goal.** Add the new Tip5-unified config builder; runs
end-to-end on a toy proof.

**Files affected:**
- `Plonky3-recursion/circuit-prover/src/config.rs` (additive).

**Changes (additive):**

```rust
/// **ADDITIVE (M-S5b S1.B sub-stage)** — Goldilocks Tip5
/// **unified** outer-cert config: Tip5-everywhere (no
/// Poseidon2). Same ≥80-bit unconditional Johnson-radius FRI
/// soundness as `goldilocks_tip5_120bit()` (log_blowup=2,
/// num_queries=42, pow=1+1); replaces Poseidon2-W8 with Tip5
/// for MMCS + Fiat-Shamir challenger.
///
/// Predicted L2 floor: ~23–27 KB (vs the dual-hash 40 KB at
/// the Poseidon2-based config). Eliminates the Poseidon2
/// perm AIR sub-circuit (-8 to -12 KB) + drops max
/// constraint degree from 7 to 2, shrinking the quotient
/// polynomial (-10 to -15 KB).
///
/// **Prerequisite:** C2.4 R-a tail at D=2 must be resolved
/// (P0 of `2026-05-20_POSEIDON2_REMOVAL_SPEC.md`).
#[inline]
pub fn goldilocks_tip5_unified_80bit() -> GoldilocksTipsConfig {
    let perm = Tip5Perm;
    let hash = Tip5Sponge::new(perm);
    let compress = Tip5Compress::new(perm);
    let val_mmcs = MerkleTreeMmcs::new(hash, compress, 3);
    let challenge_mmcs = ExtensionMmcs::new(val_mmcs.clone());
    let dft = Radix2DitParallel::default();
    let fri_params = FriParameters {
        log_blowup: 2,
        log_final_poly_len: 0,
        max_log_arity: 1,
        num_queries: 42,
        commit_proof_of_work_bits: 1,
        query_proof_of_work_bits: 1,
        mmcs: challenge_mmcs,
    };
    let pcs = TwoAdicFriPcs::new(dft, val_mmcs, fri_params);
    let challenger = DuplexChallenger::new(perm);
    StarkConfig::new(pcs, challenger)
}
```

Plus a high-arity sibling
`goldilocks_tip5_unified_80bit_higharity()` for the C3 size
lever (analogous to the existing
`goldilocks_tip5_120bit_higharity()`).

**Exit gate:**
- New builder compiles.
- Toy STARK roundtrip (Fibonacci AIR at TEST_PEARL): prove
  + verify accepts under the new config.
- Tamper-reject: any tampered cell rejects.

### 4.4 P3 — KAT parity test (the de-risk gate)

**Goal.** Prove that the new Tip5-unified outer-cert produces
verifiable proofs that compose into the recursion chain
correctly — i.e., L1 + L2 build using the new config and
verify.

**Files affected:**
- `Plonky3-recursion/recursion/tests/test_tip5_layer0_recursion.rs`
  (additive new test).
- `Plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs`
  (additive new test mirroring `c3_stage_a/b` but using
  `goldilocks_tip5_unified_80bit()`).

**Changes:**

```rust
/// CSA M-S5b S1.B P3 — KAT parity test:
/// L1 outer-cert builds + verifies using the Tip5-unified
/// config (goldilocks_tip5_unified_80bit) byte-faithfully
/// equivalent to the Poseidon2-based config
/// (goldilocks_tip5_120bit), modulo the hash-output
/// difference (which is intentional).
#[test]
fn c3_stage_a_l1_tip5_unified_kat() {
    // Build inner Tip5-L0 proof (unchanged from existing).
    // Build L1 outer-cert at goldilocks_tip5_unified_80bit().
    // Verify L1 accepts honest input + rejects tampered.
    // Compare proof byte-size vs goldilocks_tip5_120bit() variant.
}

#[test]
fn c3_stage_b_l2_over_tip5_unified_kat() {
    // L2-over-L1 at the new config; same accept + tamper.
}

#[test]
fn c3_stage_c_sweep_tip5_unified() {
    // Full 5-inner-profile sweep at the new config.
}
```

**Exit gate:**
- Toy + production-class L1 + L2 build + verify at the new
  config: all green.
- Tamper-reject pairs for each new test: all green.
- Per-test: WitnessChecks multiset net 0 (R-a tail fix
  confirmed in this stage).

### 4.5 P4 — Size measurement (the empirical validation)

**Goal.** Confirm the predicted ~18–27 KB savings empirically.

**Files affected:**
- `Plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs`
  (additive measurement; non-CI by default; manually
  invocable per the existing pattern).

**Changes:**

```rust
/// CSA M-S5b S1.B P4 — actual L1/L2 size measurement at
/// Tip5-unified config. Validates predicted ~18–27 KB
/// floor savings vs Poseidon2-based baseline.
#[test]
#[ignore = "M-S5b S1.B P4 — heavy size measurement"]
fn measure_l2_size_tip5_unified_vs_poseidon2() {
    let l2_poseidon2 = build_l2_at(goldilocks_tip5_120bit());
    let l2_tip5_unified = build_l2_at(goldilocks_tip5_unified_80bit());

    eprintln!("L2 size (Poseidon2-based): {}", l2_poseidon2.serialized_size());
    eprintln!("L2 size (Tip5-unified): {}", l2_tip5_unified.serialized_size());
    eprintln!("Delta: {}", l2_poseidon2.serialized_size() - l2_tip5_unified.serialized_size());

    // Assert the savings are at least 15 KB (lower bound of predicted 18-27 KB).
    assert!(
        l2_poseidon2.serialized_size() - l2_tip5_unified.serialized_size() >= 15_000,
        "Tip5-unified should save at least 15 KB vs Poseidon2-based"
    );
}
```

**Exit gate:**
- Measurement script reports L2 size delta ≥ 15 KB
  (target: ~18–27 KB savings).
- If delta is below 15 KB → halt P5; investigate.
- Update `2026-05-19_PROOF_SIZE_RECALIBRATION_MEASUREMENTS.md`
  with the new measurement.

### 4.6 P5 — Production-config flip (CRIT-1-style staged)

**Goal.** Make the Tip5-unified config the production default;
preserve the Poseidon2-based config under a `_legacy_` name
for rollback.

**Files affected:**
- `Plonky3-recursion/circuit-prover/src/config.rs`.
- All callers of `goldilocks_tip5_120bit()` (grep across
  repo).

**Changes (staged):**

P5.1 — Rename existing:
```rust
pub fn goldilocks_tip5_120bit_legacy_poseidon2() -> GoldilocksConfig {
    // ... old Poseidon2-based body ...
}
```

P5.2 — Make `goldilocks_tip5_120bit()` an alias for the new:
```rust
/// **Production-default after M-S5b S1.B P5 flip (2026-05-2X).**
/// Now aliases the Tip5-unified config; legacy Poseidon2-based
/// builder retained as `goldilocks_tip5_120bit_legacy_poseidon2`
/// for rollback.
#[inline]
pub fn goldilocks_tip5_120bit() -> GoldilocksTipsConfig {
    goldilocks_tip5_unified_80bit()
}
```

Wait — the return-type change breaks ABI. Callers expecting
`GoldilocksConfig` (the Poseidon2-based type) will break.

**Correct staging:** keep the *function name* but change
its body + return type in one atomic commit:
```rust
pub fn goldilocks_tip5_120bit() -> GoldilocksTipsConfig {
    // Tip5-unified body (was Poseidon2-based).
}
```

Plus update every caller's local variable type (mechanical;
the IDE-style refactor).

**Exit gate (P5):**
- Full Plonky3-recursion test suite green at the new config.
- Full `c3_stage_a/b/c` regression green at the new config
  (the existing tests now test the Tip5-unified path).
- `cargo test -p ai-pow-zk --lib --release` regression green.
- Production-config flip verifiable via a CI bench: L1/L2
  size at the new default matches P4 measurement (within
  1% noise).

### 4.7 P6 — Remove legacy Poseidon2 Goldilocks outer-cert builders

**Goal.** Delete the now-unused
`goldilocks_tip5_120bit_legacy_poseidon2()` + related
imports.

**Files affected:**
- `Plonky3-recursion/circuit-prover/src/config.rs`:
  - Remove `goldilocks_tip5_legacy_poseidon2()`
  - Remove `goldilocks_tip5_120bit_higharity_legacy_poseidon2()`
  - Remove `Poseidon2Goldilocks` import (line 21).
- `Plonky3-recursion/circuit-prover/src/batch_stark_prover.rs`:
  - Inspect: are `Poseidon2ProverD2` registrations still used
    at Goldilocks? (Probably yes, for other code paths.)
  - Conservative: keep Plonky3-recursion's general-purpose
    Poseidon2 prover registrations; only remove the
    Goldilocks Tip5-context Poseidon2-specific binding.

**Exit gate:**
- File compiles; no broken imports.
- Full test suite green.
- Git diff shows only deletions (no new code).

**R1 note.** This is the irreversible cleanup. Only land it
after P5 has been in production for a validated period
(e.g., one full release cycle of dogfooding). If P5 surfaces
any issue, roll back via the preserved legacy builder before
P6 lands.

### 4.8 P7 — Acceptance gate

**Goal.** Final regression + acceptance criteria for the
hash-unification milestone.

**Validation:**
1. **Full regression green** at the production default
   (Tip5-unified):
   - `cargo test -p ai-pow-zk --lib --release` (~390 tests).
   - `cargo test -p ai-pow --features zk --release` (~115 tests).
   - `cargo test -p p3-recursion --release` (full).
   - `cargo test -p p3-tip5-circuit-air --release` (full).
   - `c3_stage_a/b/c` + `s3ii_l3_over_l2` (manual-invocation).

2. **Tamper-reject pairs green** for every new test added in
   P3/P4/P5.

3. **L1/L2 size measurement** at Tip5-unified:
   - L1 actual: target ≤ 850 KB (vs current 961 KB) = ~110 KB
     savings.
   - L2 actual: target ≤ 530 KB (vs current 618 KB) = ~90 KB
     savings.

   The savings amplify across L1/L2 because the structural
   floor reduction applies to both layers. Total ~200 KB
   savings across the chain (more than the per-layer 18–27 KB
   structural-floor savings, because the smaller L1 also
   shrinks L2's verifier-circuit work that proves L1
   verifies).

4. **CSA per-AIR re-validation:**
   - Tip5 perm AIR appears at outer-cert now; CSA inventory
     update needed (Tip5 AIR is reused at 2 layers in the
     verifier circuit, not 1).
   - Per-AIR bits: Tip5 verifier-circuit was 109 (per CSA
     §3.2); now appears twice (inner + outer) → still ≥80
     each.
   - Quotient polynomial max degree drops from 12 to 4 ⇒
     opens for the quotient column shrink (already captured
     in §2.2).

5. **Cross-ref updates:**
   - `2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md`:
     flip §3.2.0 status from "design" to "LANDED with
     measurements".
   - `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md`:
     update §3 staged plan with the hash-unification as
     a sub-stage of S1.B.
   - `2026-05-19_C4_AUDIT_READINESS.md` §11: add this
     spec doc to the reference map.
   - `crates/ai-pow-zk/README.md`: update Open lines of
     work with the Poseidon2 removal verdict.
   - `2026-05-20_CONSTRAINT_INVENTORY.md` §4.1
     (Poseidon2 perm AIR row): mark "removed from outer-cert
     M-S5 chain" with cross-link to this spec doc.

**Exit gate (P7):**
- All 4 validation items green.
- Cross-refs updated.
- This spec doc's status flipped to LANDED.

---

## 5. Test plan summary

### 5.1 New tests landed by phase

| Phase | New tests | File |
|---|---|---|
| P0 | C2.4 R-a tail D=2 fix tests (KAT + tamper) | `Plonky3-recursion/tip5-circuit-air/src/air_circuit.rs` |
| P1 | Tip5Perm-wrapper-at-outer-cert parity test (vs inner Tip5Perm) | `Plonky3-recursion/circuit-prover/src/tip5_wrapper.rs` |
| P2 | Toy STARK accept + tamper-reject at new config | `Plonky3-recursion/circuit-prover/src/config.rs` tests |
| P3 | `c3_stage_a/b/c_tip5_unified_kat` + tamper-reject pairs | `Plonky3-recursion/recursion/tests/test_tip5_layer0_compression.rs` |
| P4 | L2-size-measurement parity (Tip5-unified vs Poseidon2) | `test_tip5_layer0_compression.rs` (manual `#[ignore]`) |
| P5 | Production-default smoke (Tip5-unified is now the production-config; existing tests now test the new path) | (existing tests; no new tests) |
| P6 | Legacy-removed regression (no broken imports / callers) | (existing tests) |
| P7 | Full regression + cross-ref doc updates | (existing tests + doc updates) |

**Total new tests:** ~10 (per-phase deterministic) + 1 manual
size-measurement.

### 5.2 Test-coverage matrix (mirrors CSA §4.1 mechanisms)

| Tamper variant | Mechanism | Phase introduced |
|---|---|---|
| Tip5 perm tampered cell at outer-cert | M1 | P2 |
| L1 proof tampered field element | M4 (WitnessConflict) | P3 |
| L2 proof tampered field element | M4 | P3 |
| Recompose-coeff D=2 producer multiplicity tampered | M4 | P0 (the R-a tail fix) |
| MMCS Merkle path tampered at outer-cert | M5 | P3 |
| FRI query schedule tampered | M1 | P3 (inherited from existing test) |

### 5.3 Backward-compat invariants

- The existing Poseidon2-based config (`goldilocks_tip5_120bit_legacy_poseidon2`)
  remains callable in P5 (renamed but functional).
- Removed in P6 — only after the new default has been validated
  in production.

---

## 6. R1 considerations

### 6.1 Soundness-critical staging

This work is on the soundness-critical path (changes the
verifier circuit's hash function). Per R1:
- KAT-first de-risk: P3 parity test gates P5 production flip.
- Validated subset + precise residual: if any phase hits a
  wall, the prior phases commit (R1 fallback).
- No fake completion: each phase's exit gate is concrete
  (size measurements + tamper-rejects + regression).

### 6.2 Fenced-linchpin invariants

- ✅ C2.1 Tip5 perm AIR (`tip5-circuit-air/src/air*.rs`):
  byte-identical against `259cab2` throughout this spec.
  Only P0 touches `air_circuit.rs` and only at the R-a tail
  fix (single location, line 326–357).
- ✅ DT-4 duplex binding (`Plonky3-recursion/circuit/src/ops/tip5_perm/executor.rs`):
  byte-identical.
- ✅ FRI backend (`Plonky3-recursion/recursion/src/backend/fri.rs`):
  byte-identical.
- ✅ Recursion verifier (`Plonky3-recursion/recursion/src/verifier/**`):
  byte-identical.

The changes are *all in the config layer* + the new Tip5
wrapper file + minimal at the R-a tail fix.

### 6.3 R1.1 anti-avoidance

Per R1.1, once this spec is approved + the C2.4 R-a tail fix
(P0) is unblocked, the implementation must be *attempted and
driven*. Per CSA's S0 / M-S5b's S(−1) precedent, the staged
phases land in disciplined increments with per-stage commits.

---

## 7. Honest residuals (R1)

What this spec does NOT close, deferred:

1. **C2.4 R-a tail at D=2 (M12 / `#127`)** is the
   prerequisite. This spec consumes its resolution but
   does not implement the fix.

2. **Round reduction at outer-cert (5-round Tip5 variant)**
   per routes audit § 3.2.0b — separate follow-on if size
   pressure justifies.

3. **STIR/WHIR PCS swap (Path F)** per routes audit § 3.6 —
   composes with this spec but is its own track.

4. **Path A (SNARK wrap) + Path D2 (direct Plonky2
   vendoring)** — sibling paths from the routes audit;
   independent of this spec.

5. **CSA re-validation post-flip**: the constraint inventory
   §4.1 needs updating to reflect Tip5 at outer-cert
   (replaces Poseidon2 row). Doc-only follow-on at P7.

---

## 8. Cross-references

- **Parent recommendation:**
  `2026-05-20_PROOF_SIZE_REDUCTION_ROUTES_AUDIT.md` § 3.2.0
  (hash unification) + § 9 (Path H recommendation).
- **M-S5b parent doc:**
  `2026-05-19_M_S5B_TERMINAL_COMPRESSION_DESIGN.md` § 3.2
  (S1 spec).
- **Prerequisite tracking:**
  `2026-05-19_C3_OUTER_CERT_DESIGN.md` § 13 (C2.4 R-a tail
  D=2 orphan); CSA inventory § 4.3
  (`2026-05-20_CONSTRAINT_INVENTORY.md`).
- **Soundness floors:**
  `2026-05-20_M_S5B_SOUNDNESS_ANALYSIS.md` (FRI side ≥82);
  `2026-05-20_CSA_S7_AUDIT_SIGNOFF.md` (AIR side ≥98).
- **Inner Tip5 wrapper template:**
  `crates/ai-pow-zk/src/circuit.rs:176-345`.
- **Current outer-cert config:**
  `Plonky3-recursion/circuit-prover/src/config.rs:268-334`.
- **Poseidon2 prover registrations:**
  `Plonky3-recursion/circuit-prover/src/batch_stark_prover.rs:690-738, 902, 1509`.
- **C4 audit-readiness:**
  `2026-05-19_C4_AUDIT_READINESS.md` § 8 (where the M12
  prerequisite is tracked) + § 11 (reference doc map).
- **R1 / R1.1:** `~/.claude/CLAUDE.md`.

---

## 9. R1 honest verdict

**Spec complete; ready for implementation.** The phased plan
P0–P7 is sequenced with concrete exit gates per phase,
explicit prerequisites (C2.4 R-a tail at D=2 = P0), and
backward-compat invariants (legacy Poseidon2-based builder
retained through P5; removed only at P6 after validation).

**Validated subset (this commit):** the architectural spec +
phased plan + test plan + R1 invariants.

**Precise residual (R1):** implementation of P0 through P7
— each phase its own commit per R1 staged-validation
discipline. P0 is the load-bearing prerequisite (C2.4 R-a
tail fix); P1–P7 are sequenced after.

**No fake completion.** This doc specs the work; it does not
claim it is implemented. The Poseidon2 is still in the
production outer-cert configuration as of this commit.
