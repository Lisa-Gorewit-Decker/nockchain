# C2 / M-S4 — tip5-circuit-air + Tip5 challenger/MMCS (recursion verifies our Layer-0 proofs)

> **Status:** DESIGN+IMPL (2026-05-18). Roadmap Phase C, blocked
> by C1 (done — `c2c51fb`). The single most soundness-critical
> circuit-construction milestone remaining: make the vendored
> Plonky3-recursion verifier able to **soundly verify
> ai-pow-zk's Tip5-based Layer-0 STARK proofs**. Governed by R1
> — staged, de-risk-first, native≡in-circuit KAT is the soundness
> linchpin, no fake completion, escalate only a genuine
> soundness-decision blocker.

## 1. What must be true

ai-pow-zk's Layer-0 `AiPowStarkConfig` (`crates/ai-pow-zk/src/
circuit.rs`) uses **Nockchain Tip5** (`nockchain_math::tip5`,
7-round, STATE=16, RATE=10, CAP=6, DIGEST=5) for **both** the
`DuplexChallenger` (Fiat-Shamir) **and** the `MerkleTreeMmcs`
(`PaddingFreeSponge` + `TruncatedPermutation`). The vendored
recursion verifier's permutation is a **closed Poseidon1/2
abstraction** (`recursion/src/challenger_perm.rs`:
`trait ChallengerPermConfig` with `as_poseidon1/as_poseidon2`;
first-party `poseidon{1,2}-circuit-air` members). To verify our
proofs the recursion circuit must re-run Tip5-Fiat-Shamir +
Tip5-MMCS-path + the FRI verifier **in-circuit**, which requires
a faithful in-circuit **Tip5 permutation AIR** and a Tip5 arm
threaded through the verifier.

## 1b. DECISION (2026-05-18) — build from the native 7-round spec, KAT-anchored

Exhaustive archaeology of **both** `~/Dev/nockchain` and
`~/Dev/nada` (all branches, full history) established: **no Tip5
*constraint/AIR* Hoon ever existed** — every `tip5*.hoon` in
history is either a *test-vector* or the *implementation/spec*
(`prv/lib/tip5.hoon` etc.); the legacy zkVM jet-circuit AIR
tables exist only for arithmetic jets (`table/jet/{add,mul,…}`),
never `table/jet/tip5`. Additionally the legacy Hoon Tip5 impl
is **5-round** (`num-rounds 5`) while the *deployed* ai-pow-zk
Layer-0 Tip5 (`nockchain_math::tip5`) is **7-round**
(`NUM_ROUNDS=7`). User decision: **do not port a legacy AIR;
build the Tip5 permutation AIR from scratch, faithful to the
7-round `nockchain_math::tip5::permute`, with a native≡in-circuit
KAT as the soundness anchor.** The **construction reference is
the authoritative Tip5 paper** (ePrint 2023/107, `2023-107.pdf`,
added to the repo this session — §2), which normatively specifies
the arithmetization; the **single bit-for-bit oracle is
`nockchain_math::tip5::permute`** (7-round, the exact Goldilocks
semantics the deployed Layer-0 proofs use; the paper's N=5 is the
original — Nockchain deploys 7). No legacy Hoon is used. A
divergence ⇒ the recursion verifier accepts forged proofs
(catastrophic) — hence KAT-first.

## 2. Authoritative spec (Tip5 paper, ePrint 2023/107) + exact native semantics

**Authoritative source:** `2023-107.pdf` (IACR ePrint 2023/107,
*"The Tip5 Hash Function for Recursive STARKs"*, Szepieniec et
al.) — the normative design **and** arithmetization (Tip5 is
arithmetization-oriented; the paper specifies the AET/AIR at
length, §4). Read in full this session.

**SOUNDNESS CORRECTION (supersedes a prior framing).** The
split-and-lookup S-box is **NOT a "degree-3 algebraic constraint
with no lookup argument"** — that earlier framing (and the
matching memory line) was **wrong per the authoritative paper**.
Paper §1, §1.2, §4.1, §4.6, §4.7, §5.7: the `L`-map is a 256-entry
table whose polynomial representation over F_p has **maximal /
degree-256** form; the paper *explicitly rejects* the algebraic
encoding and arithmetizes the split S-box via a **lookup
argument** (logarithmic-derivative / Bézout, §4.1). The
`(x+1)³−1 mod 257` is only the *table-generation rule in F₂₅₇*,
**not** a low-degree F_p constraint. The canonical Tip5 STARK
(Triton VM, paper §4.7) is a 3-table system — Hash, Cascade
(16→8-bit, a **prover-perf optimization only**, §4.2/§4.3, *not*
a soundness requirement), Lookup (256 rows). Paper §4.3 sanctions
a **direct narrow 8-bit lookup** into the 256-row table as
equally sound (the Cascade is skippable for an 8-bit map). ⇒ our
faithful arithmetization (§3) uses the §4.3 direct 8-bit LogUp.

**Exact native semantics (the bit-for-bit soundness oracle —
`crates/nockchain-math/src/tip5/mod.rs` + `belt.rs`):**

- Field: Goldilocks `p = 0xffffffff00000001 = 2⁶⁴−2³²+1`
  (`PRIME`/`PRIME_128`). `bmul(a,b)=a·b mod p`,
  `badd(a,b)=a+b mod p`, `bpow(a,7)=a⁷ mod p` — **standard
  (canonical-domain) Goldilocks ops** (`reduce`=`reduce_159`,
  *not* Montgomery; `montiply`/`montify` exist but are **not**
  used by `permute`/`sbox_layer`/`linear_layer`). ⇒ the AIR is
  plain-Goldilocks; **no Montgomery arithmetic in the AIR.**
- `permute`: **7 rounds** (`NUM_ROUNDS=7`; the *paper* uses N=5 —
  Nockchain's deployed Layer-0 variant is 7-round; the native
  impl, not the paper's N, is canonical for the deployed proofs).
  Per round `i∈0..7`: `a = sbox_layer(state)`;
  `b = linear_layer(a)`; `state'[j] = badd(rc[i][j], b[j])`.
  **Order: S-box → MDS → +RC.**
- `rc[i][j] = ((ROUND_CONSTANTS[i·16+j] as u128)·2⁶⁴) mod p` —
  a **compile-time-constant** per (round,lane); the AIR embeds
  the 7·16 precomputed `rc` constants (NOT the raw
  `ROUND_CONSTANTS`).
- `sbox_layer`: lanes `0..4` (split-and-lookup):
  `bytes = state[t].to_le_bytes()` (literal LE 8-byte split of
  the u64); `bytes[k] = LOOKUP_TABLE[bytes[k]]` ∀k∈0..8;
  `a[t] = u64::from_le_bytes(bytes)` (= `Σ_{k} c_k·2⁸ᵏ`). Lanes
  `4..16` (power map): `a[j] = bpow(state[j],7) = state[j]⁷ mod p`.
- `linear_layer` (MDS): `b[i] = Σ_{j=0}^{15} (MDS_MATRIX_I64[i][j]
  as u64)·a[j] mod p` — a **constant** circulant 16×16 matvec
  (entries are small positive ints, first row
  `[61402,17845,…,1108]`), degree-1 in `a`.
- `LOOKUP_TABLE[256]` (in `mod.rs`) is the soundness-canonical
  L-table; C2.0 verifies `LOOKUP_TABLE[b] == ((b+1)³−1) mod 257`
  ∀b∈0..256 (the paper's `L`; the *anchor* that our 256-row AIR
  lookup table is the right one).

The AIR must reproduce this exact integer function
`[u64;16]→[u64;16]`; representation labels ("Montgomery") are
irrelevant — only the function is. A divergence ⇒ the recursion
verifier accepts forged Layer-0 proofs (catastrophic) ⇒ the
native≡in-circuit KAT is the linchpin (R1).

## 3. Faithful arithmetization & approach (paper §4.3/§4.6-anchored)

Mirror `poseidon2-circuit-air` (Plonky3-recursion member;
`air.rs`/`columns.rs`/`public_types.rs`, `p3_lookup`
`InteractionBuilder` for cross-table lookups + a companion
preprocessed table). Per Tip5 round, AIR columns/constraints:

- **Split-and-lookup lanes (4):** 8 byte-columns `b₀..b₇` per
  lane (32 total) + their looked-up images `c₀..c₇`.
  Constraints: (i) **canonical decomposition** —
  `Σ_k b_k·2⁸ᵏ == state[t]` *and* the **paper §4.6
  inverse-or-zero `< p` constraint** so the byte split is the
  *unique canonical* one (a non-canonical split is the core
  forgery vector — soundness-critical); (ii) recompose
  `a[t] == Σ_k c_k·2⁸ᵏ`; (iii) each `(b_k,c_k)` bound to the
  256-row **L Lookup Table** via a §4.1 logarithmic-derivative
  lookup (the `InteractionBuilder` LogUp — same machinery
  `poseidon2-circuit-air` already uses). The 256-row table is a
  preprocessed `(x, LOOKUP_TABLE[x])` column pair (= the §4.3
  narrow lookup; range-checks the bytes implicitly).
- **Power-map lanes (12):** intermediate columns for `x²,x³,x⁶`
  → `a[j]==state[j]⁷` staged to constraint-degree ≤ 3.
- **MDS:** `b[i] == Σ_j M[i][j]·a[j]` (constant matrix, deg-1).
- **RC:** `state'[j] == b[j] + rc[i][j]` (constant, deg-1).
- 7 round-blocks; an input row and an output row exposed for the
  recursion glue (cross-table lookup, like poseidon2-circuit-air).

Wiring (mirrors the closed Poseidon1/2 abstraction):

- **New member `tip5-circuit-air`** (`Plonky3-recursion/
  tip5-circuit-air/`) — the above AIR + its preprocessed L-table.
- **`circuit` member `ops`**: `Tip5Config` next to
  `Poseidon1Config`/`Poseidon2Config`.
- **`recursion/src/challenger_perm.rs`**:
  `ChallengerPermConfig::as_tip5(&self)->Option<&Tip5Config>`
  + `impl ChallengerPermConfig for Tip5Config`.
- **Verifier/MMCS/FRI**: every `as_poseidon2()/as_poseidon1()`
  match site in `recursion/` (in-circuit `CircuitChallenger`,
  `RecursiveMmcs`/`PaddingFreeSponge`/`TruncatedPermutation`,
  FRI verifier) gains a Tip5 branch using the new AIR.

## 4. Staged plan (R1 — commit per validated stage)

- **C2.0 — de-risk + KAT vectors.** Extract the exact native
  Tip5 spec (Montgomery domain, the `MDS_MATRIX_I64`/RC
  tabulation, the cube-map ↔ `LOOKUP_TABLE` identity) and emit
  golden permutation KAT vectors from `nockchain_math::tip5::
  permute` (the soundness oracle). Confirm `(x+1)³−1 mod 257`
  reproduces `LOOKUP_TABLE` (the low-degree-equivalence anchor).
- **C2.1 — `tip5-circuit-air` member + native≡in-circuit KAT
  (the soundness linchpin).** Implement the Tip5 permutation
  AIR; KAT: the AIR trace's output state == `nockchain_math::
  tip5::permute` bit-for-bit on the C2.0 vectors + random
  states; the AIR's constraints are satisfied iff the
  permutation is correct (a tampered round/byte/MDS cell ⇒
  unsatisfied — adversarial). Independently buildable +
  validatable; lands first.
- **C2.2 — `Tip5Config` + `ChallengerPermConfig::as_tip5`.**
  Wire the config; unit-test the trait arm.
- **C2.3 — verifier/MMCS/FRI Tip5 branches.** Thread Tip5
  through the in-circuit challenger + RecursiveMmcs +
  FRI-verifier; the recursion substrate's own suite still green.
- **C2.4 — end-to-end + 120-bit sweep.** The recursion verifier
  verifies a **real ai-pow-zk Tip5 Layer-0 proof** (accept) and
  rejects a tampered one; the 120-bit FRI presets
  (`circuit.rs`) preserved. Full `cargo test --workspace` on the
  vendored tree + ai-pow-zk regression.

C2.1 carries the native-equivalence soundness proof and is
independent of the wiring → it lands first (R1: validate the
part whose correctness is soundness-load-bearing before the
integration).

## 5. Exit gate

A new vendored `tip5-circuit-air` member whose AIR is
KAT-proven bit-identical to `nockchain_math::tip5::permute`;
Tip5 threaded through the recursion challenger/MMCS/FRI; the
recursion verifier **accepts a real ai-pow-zk Tip5 Layer-0
proof and rejects a tampered one**; vendored-tree full tests +
ai-pow-zk regression green; the 120-bit FRI sweep preserved.

## 6. Soundness-escalation criterion (the goal's hard blocker)

Escalate for a decision ONLY on a genuine, characterized
soundness wall, e.g.: the native `nockchain_math::tip5` and the
legacy Hoon AIR disagree on the permutation (which is the
protocol-canonical one for the *deployed* Layer-0 proofs?), or
the recursion verifier's perm-abstraction cannot admit a
degree-7 Tip5 without a soundness-relevant change to the FRI/
challenger reconstruction. Per R1: attempt the minimal correct
bridge first; escalate only with the exact delta + why it can't
be safely resolved — never a vague stop. Difficulty/size is NOT
a blocker; it is a reason to stage.

## 7. Cross-references

- `2023-107.pdf` (IACR ePrint 2023/107 — **authoritative Tip5
  paper**, the normative construction/arithmetization reference,
  §4 AET/AIR);
  `crates/nockchain-math/src/tip5/mod.rs` + `belt.rs` (the
  bit-for-bit native oracle: `permute`/`sbox_layer`/
  `linear_layer`, `LOOKUP_TABLE`, `ROUND_CONSTANTS`,
  `MDS_MATRIX_I64`, plain-Goldilocks `bmul/badd/bpow`).
- `crates/ai-pow-zk/src/circuit.rs` (`AiPowStarkConfig`,
  `Tip5Perm`/`Tip5Compress`/`DuplexChallenger`, 120-bit
  presets).
- `Plonky3-recursion/poseidon2-circuit-air/` (AIR template),
  `recursion/src/challenger_perm.rs` (the closed abstraction to
  extend); `C1_RECURSION_VENDOR_DESIGN.md`,
  `c1_recursion_substrate` memory.
