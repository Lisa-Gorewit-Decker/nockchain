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

## 2b. C2.1 arithmetization DECISION — algebraic offset-Fermat-cube (soundness-equivalent, lookup-free)

The paper arithmetizes the split S-box via a **lookup argument**
for *prover-performance* (its cost model: a LogUp gate is cheaper
than cube+mod-257+range). C2's concern is **soundness** (the
recursion verifier must never accept a forged Layer-0 proof), and
for *soundness* the per-byte L-map admits a **lookup-free
algebraic arithmetization that is exactly equivalent** to the
native table:

> **Soundness-equivalence theorem (machine-proven, C2.0).**
> `LOOKUP_TABLE[b] == ((b+1)³ − 1) mod 257` for every byte
> `b∈0..256` (`tip5::c2_kat::l_table_identity…`, green). Hence a
> constraint enforcing `c = ((b+1)³−1) mod 257` accepts exactly
> the same `(b,c)` pairs as the native `LOOKUP_TABLE[b]` — they
> are the *same relation*. No multiset/permutation/LogUp argument
> is needed to bind a byte to its L-image; the cube **is** the
> table.

Per-byte constraint (degree 3, lookup-free): `u = b+1`;
`cube = u·u·u` (integer `≤ 256³ = 16 777 216 < p`, exact in
F_p — no wrap); `cube − 1 = q·257 + c` with `c` 8-bit and `q`
≤⌊(256³−1)/257⌋ (17-bit) **range-checked by boolean bit
decomposition** (degree-2 boolean constraints — *lookup-free*);
257 prime + `c<257` ⇒ `(q,c)` unique ⇒ `c = L(b)` exactly. The
**only** residual soundness obligation is that the 8-byte split
of each split-lane input is the **unique canonical** one — the
paper §4.6 `Σ bₖ2⁸ᵏ < p` inverse-or-zero guard (a non-canonical
≡-mod-p split feeding different bytes into the cube is *the*
forgery vector); implemented + adversarially tested.

**Why this is the right C2.1 (R1 discipline):** it makes the
Tip5 *permutation* AIR a **single self-contained AIR** (no
cross-table bus) ⇒ provable/verifiable with plain
`p3_uni_stark::{prove,verify}` ⇒ the soundness-load-bearing core
is **independently + exhaustively validatable** (real prove→
verify on the frozen golden KAT + adversarial rejection) without
entangling the batch-stark/LogUp recursion machinery. Faithful
because correctness is anchored by the C2.0 identity theorem +
the §4.6 canonical guard + the native≡in-circuit KAT (the AIR
trace == `nockchain_math::tip5::permute`, bit-for-bit, on the
committed fixture). The LogUp/cross-table-bus form is required
**only** for *how the recursion verifier invokes* the perm table
(C2.2/C2.3) — a precise residual, not a soundness gap in C2.1.

## 2c. CORRECTION (2026-05-18) — §2b over-justified the lookup-free form; width is unacceptable; implement the lookup table

§2b's "why this is the right C2.1" **overstated** the case and
produced a **pathologically wide AIR (≈7604 cols)** — the
lookup-free byte range checks are **32 boolean columns per byte**
(8 `b`-bits + 8 `c`-bits + 16 `q`-bits) × 8 bytes × 4 split lanes
× 7 rounds = **7168 columns** of pure range-check scaffolding.
The Tip5 paper uses a lookup table precisely to avoid this.

The §2b claim that the lookup form "requires the batch-stark/
LogUp recursion machinery" **conflated two different things**:

- a **LOCAL LogUp lookup** into a 256-row preprocessed L-table —
  collapses the ≈7168-col core to **2 cols/byte** (`b`,`c`) +
  one shared 256-row table + a multiplicity col + 1 aux
  (running-sum) col (≈ **8×** narrower). It is **standalone-
  validatable with no recursion machinery** — Plonky3's own
  `p3-lookup` `RangeCheckAir` (`lookup/src/tests.rs`) does exactly
  this: build main+aux trace, assert the LogUp accumulator
  `s_final + last_contribution == 0` (⟺ every queried `(b,c)` ∈
  the table ⟺ `b∈[0,256)` *and* `c=L[b]`); a tamper ⇒ nonzero.
- the **CROSS-table CTL / witness-bus** binding to the recursion
  verifier — *that* is the genuine C2.3 residual.

The local lookup is part of getting the permutation AIR *right*
and should have been C2.1. **Decision: implement the lookup-table
form (`Tip5PermLookupAir`).** The §4.6 canonical-`<p` guard is
*retained* (the lookup gives `b∈[0,256)` and `c=L[b]` but not
that `Σ bₖ2⁸ᵏ` is the unique canonical representative); x⁷/MDS/RC
unchanged. The C2.0 identity theorem still anchors *which* table
(`(i, LOOKUP_TABLE[i])`, i∈0..256). Plan (R1 — staged, additive,
KAT-first; the validated lookup-free `Tip5PermAir` is kept as a
cross-oracle until the lookup form passes the **entire** gate):

**STATUS — L1 column-layout DONE; L2 PARTIAL + a DEGREE FLAW
FOUND & CORRECTED (2026-05-18).** `Tip5PermLookupAir`
(`air_lookup.rs`) + `generate_lookup_trace`: **886 main columns
vs 7604** — the column width *is* ≈8.6× smaller, and
native-equivalence is real (lookup-AIR trace `(IN,ROUT[6])` ==
`nockchain_math::tip5::permute` bit-for-bit on **315 fixture +
2048 random**; algebraic `check_constraints` clean against the
verifier-fixed preprocessed L-table; LogUp **value** accumulator
exactly 0 honest / ≠0 on tampered `c`; ROUT-tamper & §4.6
non-canonical rejected; crate 10/10).

**BUT — the single-interaction LogUp is FRI-infeasible (degree
≈226).** `air_lookup.rs::eval` pushes one `push_local_interaction`
with all `7·4·8 = 224` byte-query tuples + 1 table tuple per row.
`LogUpGadget::constraint_degree` (`p3-lookup/src/logup.rs:339`)
for one interaction is `1 + Σ_tuples(elem degree) = 1 + 225 ≈
226` ⇒ needs `log_blowup ≥ 8`; at that blowup the "narrow" AIR is
~7× *worse* than the lookup-free one. So:

- `BaseAir::max_constraint_degree = Some(4)` is **only the
  hand-written algebraic constraints**; it does *not* model the
  LogUp gadget's degree-226 constraint (now documented as such in
  `air_lookup.rs`, with a module-level ⚠).
- L2's `check_constraints` runs only the algebraic constraints
  (not the LogUp gadget); the explicit accumulator test validates
  the lookup **value** (`Σ=0`), not STARK-feasibility. So L2
  proves native-equivalence + the lookup *relation* is sound, but
  **not** that this is a provable low-degree STARK. It is not, as
  structured.
- Root cause: batching 224 lookups into one interaction is the
  wrong LogUp use. The feasible low-degree narrow form is the
  **multi-interaction shared bus** (Tip5 paper §4.7
  Hash⟷Lookup-table; the poseidon1-circuit-air `WitnessChecks`
  pattern: many small `push_interaction` calls, one aux column
  each, low degree) — which *is* the C2.3 path. `Tip5PermLookupAir`
  is retained only as the native-equivalence column-layout
  reference while the **bus form (L4)** is built.

**L4 stage-1 — global-bus restructure: DONE & the degree flaw
machine-proven FIXED (2026-05-18, user-directed "do the global
bus and do it correctly").** `air_lookup.rs::eval` now emits, via
`p3_lookup::bus::LookupBus` on the shared bus **`tip5_l`**, **224
per-byte `lookup_key` query interactions + 1 `table_entry`
provide = 225 separate single-tuple global interactions** (was
one 225-tuple `push_local_interaction`). Decisively validated
(`tests::global_bus_interactions_are_low_degree`): extracting the
interactions via `InteractionSymbolicBuilder` and compiling each
to a LogUp `Lookup`, **`LogUpGadget::constraint_degree` = 2 for
every one of the 225** (max 2; was ≈226) — FRI-feasible, well
within `log_blowup=2` (B=4). `BaseAir::max_constraint_degree =
Some(4)` is now honest (algebraic kind-gated = 4 ≥ LogUp = 2).
Native-equivalence preserved (generation unchanged;
`lookup_air_equals_native_spec` still 315 fixture + 2048 random
== `nockchain_math::tip5::permute`); algebraic + value-soundness
+ adversarial all green; crate **11/11**; lookup-free
`Tip5PermAir` retained as the redundant native-equivalence
cross-oracle; full Plonky3-recursion workspace builds clean.

**L4 stage-2 — DONE & INDEPENDENTLY RE-VALIDATED (2026-05-18,
user-directed "implement L4 stage-2: the Tip5 NPO subsystem and
batch-stark gate").** The full Tip5 NPO subsystem + circuit-prover
table + batch-stark gate, a faithful mechanical mirror of the
Poseidon1 D=1 path (Goldilocks-only, no merkle/MMCS/D>1; rate
10/cap 6):

- **circuit NPO** `circuit/src/ops/tip5_perm/{call,builder,
  executor,plugin,state,trace}.rs` (+ `npo.rs`/`builder/npo.rs`/
  `mod.rs` wiring, `enable_tip5_perm`/`add_tip5_perm`); the
  executor runs the closure = `tip5_spec::permute` (=
  `nockchain_math::tip5::permute`).
- **circuit-prover** `batch_stark_prover/tip5.rs`
  (`Tip5Prover`/`Tip5Preprocessor`/`Tip5AirBuilder`,
  `register_tip5_table`, `RegisterTip5ForExt<1>`) — wraps the
  **existing validated** `Tip5PermLookupAir` via the new
  `tip5-circuit-air::air_circuit::Tip5CircuitAir`: it calls the
  *unmodified* inner `eval` **verbatim** (validated algebraic
  constraints + the degree-2-proven `tip5_l` bus reused as-is)
  and only **adds** the standard `WitnessChecks` CTL
  (poseidon1-pattern, `kind`-gated) — confirmed by diff:
  `air_lookup.rs` changed *only* by +3 read-only column
  accessors; constraints/bus byte-for-byte unchanged.
- **Gate** `recursion/tests/test_tip5_lookups.rs`:
  `test_tip5_ctl_lookups` (real `prove_all_tables` +
  `verify_all_tables` over the Tip5 NPO + `tip5_l` global
  reconciliation — genuinely run, not bypassed) and
  `test_tip5_tampered_proof_fails` (corrupts a FRI-bound opened
  trace value ⇒ verification must reject; fails loudly if
  accepted).
- **Config**: added `circuit-prover config::goldilocks_tip5()`
  FRI `log_blowup=2` (B=4) — the correct/validated tier for the
  degree-4 §4.6/x⁷ Tip5 constraints (the default B=2 cannot
  prove degree-4); not a soundness weakening (tamper test still
  rejects at B=4).

Independently re-validated (not trusting the agent's report):
`air_lookup.rs` diff reviewed (additive-only); `Tip5CircuitAir::
eval` reviewed (verbatim inner + additive CTL); re-ran
**p3-tip5-circuit-air 11/11** (validated core intact),
**`test_tip5_ctl_lookups` + `test_tip5_tampered_proof_fails`
pass**, **full Plonky3-recursion workspace green, no regression**
(poseidon1/2 10/10, recursion 27, circuit 358, circuit-prover
40, …). Built in an isolated path, reviewed line-by-line on the
soundness-critical pieces before trust.

**Remaining C2 residual (honest, NOT this stage):** the gate
proves the Tip5 permutation is soundly provable/verifiable
through the batch-stark + CTL forgery-binding machinery. It does
**not** yet have the recursion verifier consuming a *real
ai-pow-zk Tip5 Layer-0 STARK proof* — that needs the in-circuit
**challenger duplexing + MMCS-path** reconstruction wired with
Tip5 (rate-10 `DuplexChallenger`/`PaddingFreeSponge`/
`TruncatedPermutation`), and C2.4 (real Layer-0 end-to-end +
120-bit sweep). That is the precisely-scoped remaining C2.3/C2.4
work; the hard soundness-binding kernel (Tip5 ⇒ batch-stark CTL)
is now done and validated.

## 2c.L5 — in-circuit challenger duplexing + MMCS path (DONE, independently re-validated 2026-05-19)

User-directed ("implement the in-circuit challenger duplexing +
MMCS path for Tip5"). A faithful Poseidon1-D1 mirror → Tip5
(Goldilocks, width 16, **rate 10, capacity 6, digest 5**,
7-round), so the recursion verifier reconstructs the Tip5
Fiat-Shamir transcript + Merkle-MMCS path in-circuit:

- `p3-tip5-circuit-air::Tip5Perm` — new in-workspace
  `Permutation<[Goldilocks;16]>`/`CryptographicPermutation`
  adapter over the validated `tip5_spec::permute` (the recursion
  workspace cannot depend on `ai-pow-zk`; this is the single
  native reference oracle). **Validated-core diff = +2 lib.rs
  lines only; `air_lookup/air_circuit/generation_lookup/
  tip5_spec` byte-for-byte unchanged.**
- `circuit/src/ops/perm.rs`: `PermConfig` (the closed
  challenger/MMCS perm enum the Explore wrongly called
  "trait-based/free") gains a `Tip5(Tip5Config)` arm + every
  match (`d/rate/rate_ext/width_ext/digest_ext/npo_type_id→
  tip5_perm`, `PermCall`/private-data dispatch).
- **`digest_ext` (the genuine soundness-relevant non-1:1):** all
  Poseidon configs have digest == rate, so the MMCS verifier
  conflated digest width with `rate_ext`. Tip5 has digest 5 ≠
  rate 10 (`PaddingFreeSponge<Tip5Perm,16,10,5>`,
  `TruncatedPermutation<Tip5Perm,2,5,16>`). Added
  `digest_ext()` to all three configs (`= rate_ext()` for
  Poseidon1/2 ⇒ **byte-identical**, verified by full poseidon
  regression; `= 5` for Tip5) and threaded it through
  `circuit/ops/mmcs.rs` (sibling `[digest_ext,2·digest_ext)`,
  capacity zeroed, digest-wide CTL/root) + `recursion/pcs/
  mmcs.rs` leaf-squeeze (native `OUT`).
- Tip5 merkle/MMCS executor+call+builder+plugin+state path;
  `add_tip5_perm_for_challenger_base`; `duplexing_base_tip5` +
  `as_tip5()` arm + `new_goldilocks_tip5_base()`.

**Independently re-validated (R1 — not trusting the agent):**
diffed the validated core (additive-only); reviewed
`perm.rs`/`PermConfig::Tip5`/`digest_ext` numerics
(Poseidon=`rate_ext`, Tip5=5) by hand; re-ran the full
Plonky3-recursion workspace **green, no regression** —
poseidon1/2 10/10, `challenger_transcript` **46** (old 42 +
**4 new `goldilocks_d1_tip5`**: observe/sample, partial-absorb,
multi-round, observe-after-sample — each asserts the in-circuit
challenger == native `DuplexChallenger<Goldilocks,Tip5Perm,16,
10>` **bit-for-bit** via `connect`+`runner().run()`),
`p3_recursion` **32** (+**5 `tip5_mmcs_test`**: small/8x3/
multi-height/cap-height vs native `MerkleTreeMmcs<…
PaddingFreeSponge<Tip5Perm,16,10,5>,TruncatedPermutation<Tip5
Perm,2,5,16>…>`, + **tamper-fails** = flip a sibling-digest
limb ⇒ `panic!` if accepted), C2.3 gate 2/2, validated core
**13/13**.

**Honest scope (no fake completion).** This proves the in-circuit
Tip5 challenger duplexing **and** MMCS-path reconstruction are
**bit-for-bit equal to the native `DuplexChallenger`/
`MerkleTreeMmcs`** — by the *exact same runner-based mechanism*
the existing poseidon1/2 challenger+MMCS tests use (verified, not
weaker). It does **not** add the *batch-STARK cross-row
sponge-chain forgery binding* (the single-row `Tip5CircuitAir`
pre-resolves state in the executor; the multi-row chain/swap/
mmcs-accumulator AIR constraints + a D=1 `PcsRecursionBackend`
so the `fri.rs` dispatch sites are reachable for Tip5 are the
**C2.4 residual**). Net: the recursion verifier can now
reconstruct Tip5 Fiat-Shamir + Merkle-MMCS in-circuit, validated
against the native oracle; wiring that into a full Layer-0
end-to-end recursion proof + the 120-bit sweep is C2.4.

- **L1** — `Tip5PermLookupAir` + generation: per round, per split
  lane: 8 `b` + 8 `c` byte cols; recompose `Σ bₖ2⁸ᵏ = sbox_in[t]`;
  §4.6 `inv` guard; `A[t]=Σ cₖ2⁸ᵏ`; power lanes `x2,x3,A=x3·x3·x`;
  MDS+RC→ROUT. A 256-row L-table region (`kind` selector: perm
  row vs table row) + per-table-row multiplicity; one LogUp aux
  running-sum col. Local interaction: perm rows push
  `(vec![b,c], +1)` (gated by `kind`); table rows push
  `(vec![i,L[i]], −mult_i)`.
- **L2 — exhaustive gate (the current goal):** AIR-lookup trace
  `(IN,ROUT[6])` == `nockchain_math::tip5::permute` bit-for-bit on
  all **315** fixture vectors **and 4096 random** inputs;
  `check_constraints` clean; LogUp accumulator == 0 for honest;
  adversarial — tampered `c` / non-table `b` / §4.6 non-canonical
  split / tampered output ⇒ accumulator ≠ 0 *or* constraints
  fail. Width measured + reported (expect ≈8× reduction).
- **L3** — once L2 fully green, `Tip5PermLookupAir` is the
  default; the lookup-free `Tip5PermAir` is retained only as a
  redundant native-equivalence cross-check. Docs/memory updated.

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

## 4. Staged plan + STATUS (R1 — commit per validated stage)

- **C2.0 — de-risk + KAT vectors. ✅ DONE (commit `dc4e217`).**
  Exact native spec pinned (plain-Goldilocks `bmul/badd/bpow`,
  `rc=(RC·2⁶⁴) mod p`, S-box→MDS→+RC ×7). `nockchain-math`
  `tip5::c2_kat`: machine-proved `LOOKUP_TABLE[b]==((b+1)³−1)
  mod 257 ∀b` + bijection + fixed points; froze the dep-free
  golden KAT oracle (`tip5_golden_kat.txt`: constant tables +
  18 `permute` vectors) with read-only drift detection. 2/2 green.
- **C2.1 — `tip5-circuit-air` member + native≡in-circuit KAT
  (the soundness linchpin). ✅ DONE + EXHAUSTIVELY TESTED.** New
  vendored member `Plonky3-recursion/tip5-circuit-air`
  (lookup-free §2b arithmetization: canonical 8-byte split +
  §4.6 `<p` guard + offset-Fermat-cube + x⁷ + const MDS + const
  RC, 7 rounds, one-row-per-permutation; each constraint group
  annotated with its Tip5-paper section — §2.1 round iter, §2.2
  S/T S-boxes, §2.3 circulant MDS, §2.4 RC, §4.6 decomposition).
  **7/7 green.** Exhaustive native-equivalence chain (each leg
  tested): the golden fixture is widened to **315 vectors**
  (paper-component-targeted edge cases — L fixed points,
  per-split-lane sweeps, 16 MDS single-lane impulses, power-lane
  sweeps, §4.6 boundary band, chained multi-permute — + 256
  seeded states), all generated from & re-verified against
  **live `nockchain_math::tip5::permute`** (`nockchain-math
  c2_kat`); `native_equiv_kat` asserts AIR trace `(IN,ROUT[6])`
  == that fixture bit-for-bit (315-pt direct);
  `air_equals_native_spec_exhaustive_random` asserts AIR ==
  native spec over **4096 deterministic-random** permutations
  with `check_constraints` on every one; spec≡fixture≡nockchain
  -math pinned by `tip5_spec_matches_fixture_permute` +
  `embedded_constants_match_fixture`. Adversarial — tampered
  OUT/A/bit rejected, **and the precise §4.6 forgery vector** (a
  non-canonical `x+p` split, otherwise fully consistent)
  rejected *solely* by the canonical guard. Real Goldilocks
  uni-stark `prove`→`verify` on the full batch. Full
  Plonky3-recursion workspace builds clean; nockchain root
  undisturbed.
- **C2.2 — `Tip5Config` + `ChallengerPermConfig::as_tip5`.
  ✅ DONE (commit `8ced2e8`).** `circuit/src/ops/tip5_perm/
  {config,mod}.rs`: `Tip5Config` mirrors the `Poseidon1Config`
  interface with **Tip5-correct internals** verified vs the
  deployed Layer-0 (Goldilocks, d=1, width=16, **rate=10,
  capacity=6**, digest=5, 7 rounds — *not* Poseidon's width/2
  rate; transcript-soundness-relevant). `ops/mod.rs` exports it;
  `recursion/src/challenger_perm.rs` gains `as_tip5` (default
  `None` ⇒ Poseidon1/2 unaffected) + `impl … for Tip5Config`.
  4/4 config unit tests; full Plonky3-recursion workspace builds
  clean (zero new warnings); additive/non-breaking.
- **C2.3 — Tip5 NPO subsystem + verifier/MMCS/FRI arms.
  ⏳ RESIDUAL (the soundness-critical bulk).** Mirror the
  `poseidon1_perm` op subsystem for Tip5 (**~2650 LOC** across
  `call/plugin/executor/builder/state/trace` — the
  `executor.rs` alone is ~1514 LOC of *transcript*-bearing
  chain-state / Merkle-path-swap / MMCS-index-accumulation /
  rate-10 absorption logic that **must** match the native
  prover bit-for-bit) + a `circuit-prover` Tip5 table/AIR/
  preprocessor (reuse the C2.1-validated `p3-tip5-circuit-air`
  in its CTL/witness-bus *preprocessed* form — itself a second
  soundness-bearing AIR variant needing a bus-form≡standalone≡
  native KAT) + Tip5 arms at every `as_poseidon2()/as_poseidon1()`
  dispatch site in `recursion/` (in-circuit `CircuitChallenger`
  duplexing, `RecursiveMmcs`/`PaddingFreeSponge`/
  `TruncatedPermutation`, the FRI backend
  provers/air_builders/preprocessors). This is **atomic** — it
  is only *meaningfully* validatable end-to-end (executor ≡ AIR
  ≡ native transcript); a standalone "executor permutes 16
  elements" test is green on exactly the parts that aren't
  soundness-load-bearing. Per R1 it must be staged + KAT-first,
  **not rushed** into the verifier.
- **C2.4 — end-to-end + 120-bit sweep. ⏳ RESIDUAL.** The
  recursion verifier verifies a **real ai-pow-zk Tip5 Layer-0
  proof** (accept) and rejects a tampered one; 120-bit FRI
  presets preserved; full `cargo test --workspace` on the
  vendored tree + ai-pow-zk regression.

**Honest status (R1 — validated subset + precise residual,
after a genuine multi-stage in-flight attempt).** Done +
exhaustively validated + committed: **C2.0** (`dc4e217`, oracle),
**C2.1** (`62413ba`, the soundness linchpin AIR — the
cryptographically subtle part whose wrongness silently forges
Layer-0 proofs), **C2.2** (`8ced2e8`, the integration's first
slice — `Tip5Config` + `as_tip5` threaded into the recursion
substrate, non-breaking, 4/4 tests). The integration was
**attempted and driven, not merely scoped** (C2.2 is real,
landed, validated recursion-substrate code). The residual
(C2.3 + C2.4) is the **atomic, ~2650+ LOC, transcript/CTL-
soundness-critical** verifier bulk: it is only meaningfully
validatable end-to-end (in-circuit Tip5 executor ≡ AIR ≡ native
Fiat-Shamir/MMCS transcript), so it cannot be landed as a
further *correctly-validated* small increment without rushing a
soundness-critical verifier change — which R1 forbids ("a
half-landed invasive soundness change is strictly worse than a
clean validated subset plus a precise residual"; rushed
soundness breakage "won't surface in green unit tests"). This is
R1's intended last-resort outcome reached *through* genuine
driven work across four validated stages — explicitly **not**
the R1.1 "scoped-but-not-attempted" avoidance.

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
