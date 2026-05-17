# HIGH-2.2 §6(b)-G3 — Proof Recursion & Aggregation: Detailed Design Spec

> **Status (2026-05-17): DESIGN.** This is the authoritative,
> implementation-ready spec for the recursion/aggregation layer
> (the "M12" workstream) that lets the §6(b) useful-work binding
> scale to true PROD. It expands `HIGH2_2_DESIGN.md §4.C.4-G3`.
> §6(b) is already **closed in-circuit for every single-Layer-0
> params set** (G1+G2, commits `010ccd3`/`604f974`); G3 removes the
> "fits one STARK" restriction with **zero probabilistic gap**.
> No code is changed by this document.
>
> Audience: someone implementing G3a/G3b/G3c. Assumes familiarity
> with `composite_proof.rs` (Route-A `composite_prove_pinned_logup`),
> CRIT-1 (`composite_full_air.rs`), the §6(b) carry registers
> (`CompositeTrace::place_useful_work_chain`), and Plonky3
> `p3-batch-stark`.

---

## Table of contents

0. Scope, terminology, what "clean" means here
1. High-level conceptual explanation (the mental model)
2. Background: precisely what a Layer-0 *segment proof* is
3. The recursion primitive: verifying a STARK inside a STARK
4. The carry vector `Γ` and the uniform claim
5. The aggregation tree
6. Extending CRIT-1 across the tree: `PROGRAM_ROOT(params)`
7. Fiat–Shamir & domain separation across layers
8. Soundness: theorem, proof sketch, error budget
9. Determinism, transparency, public-coin, no trusted setup
10. Concrete API / types and mapping to existing code
11. Edge cases & correctness details
12. Phasing: G3a / G3b / G3c with acceptance criteria
13. Relationship to Pearl; what changes vs the G4 interim
14. Open parameters to pin at implementation
15. Cross-references

---

## 0. Scope, terminology, what "clean" means here

**Scope.** Recursion + aggregation for the *PoUW Layer-0 STARK*
of `ai-pow-zk` (`CompositeFullAirWithLookupsPinned`, proved via
`p3-batch-stark`). The goal: prove a computation `T` that is far
larger than one economical STARK (true PROD ≈ several·2²⁰ rows)
by splitting `T` into bounded **segments**, proving each, and
**aggregating** the segment proofs into a single succinct proof
the chain verifies — such that the aggregate has *identical*
soundness to having proved the monolithic `T` directly.

**Terminology.**

- **Layer-0 / segment proof** — a `p3-batch-stark` `BatchProof`
  over the composite AIR for one `S`-row segment.
- **Recursion** — verifying a STARK proof *inside* another STARK
  (the verifier is arithmetized as an AIR).
- **Aggregation** — combining many proofs into one via a tree of
  recursion steps.
- **`Γ` (carry)** — the cross-segment hand-off register vector
  (§4 below).
- **Claim** — the public statement an aggregate proof attests:
  "segments covering row-span `[a,b]` were all valid, carry
  chained, against the canonical per-segment programs".
- **Root proof** — the single proof at the top of the aggregation
  tree; the only artifact the chain-level verifier checks.

**What "clean" means here (the design objectives).**

1. **One AIR, parameterized** — the composite AIR is *not* forked
   per segment; only boundary predicates are read from a small
   verifier-fixed descriptor (G3a). The single-segment case is a
   bit-identical specialization (`N = 1`), so nothing regresses.
2. **One self-similar recursion AIR** — the same recursion
   circuit verifies a *segment proof* or *another recursion
   proof* (a `kind` tag selects the verifying key). This is what
   makes it true recursion (a fixed circuit verifying proofs of
   itself) and gives unbounded aggregation depth.
3. **No new trust** — segmentation, per-segment programs, and the
   aggregation shape are **pure deterministic functions of the
   chain-pinned `params`** (the CRIT-1 / MED-3 discipline already
   in the codebase). The recursion adds *zero* trusted setup
   (FRI is transparent) and *zero* probabilistic gap (unlike the
   G4 spot-check interim).
4. **Soundness is a one-line induction** — carry-chain equality +
   per-segment CRIT-1 + count/order pinning ⇒ the concatenation
   *is* the monolithic trace.

---

## 1. High-level conceptual explanation (the mental model)

### 1.1 The problem in one paragraph

A STARK proves "I executed this bounded computation correctly."
"Bounded" because the prover must materialize the whole execution
trace and commit to it; cost grows ~linearly in trace area
(`height × width`) and the FRI commitment dominates. PROD's
useful-work computation for one tile is ≈ 2²⁰ rows (the §6(b)
matmul→StripeXor sweep) and the matrix commitment another ≈ 2²⁰
— together far past what one STARK proves economically. We cannot
shrink the computation (it *is* the useful work — a real LLM-FFN
matmul). So we must **split it and stitch the pieces with
proofs**.

### 1.2 The mental model: an assembly line with sealed batons

Picture the giant computation `T` as a conveyor belt of `S`-row
**segments** `T_0, T_1, …, T_{N-1}`. Between consecutive segments
a small **baton** `Γ` is passed: the few register values that
carry state across the cut (the matmul accumulator `CUMSUM`, the
StripeXor per-stripe register `SX_XR`, the fold state, the
running matrix-hash). Each worker (a Layer-0 STARK) proves
*locally*: "given the baton I received (`Γ_in`), I ran my `S`
rows by the rules, and here is the baton I pass on (`Γ_out`)."

A worker's proof says nothing about the other workers. To trust
the *whole* line we need three things checked:

- **every** worker's local proof is valid,
- worker `k` passed the *exact* baton worker `k+1` received
  (`Γ_out(k) == Γ_in(k+1)`),
- the line has exactly the right number of workers, in order,
  each doing *their* assigned step (no skipping the expensive
  matmul segment and splicing in a cheap one).

Doing those checks "by hand" (the chain re-verifying `N` proofs +
`N` equalities) defeats the purpose. **Recursion** is: build a
*new* small circuit — the **recursion node** — whose job *is*
"verify a Layer-0 proof and check one baton link," and prove
*that* with a STARK. Now a single recursion proof attests "this
segment was valid and its baton matches its neighbour." A
**recursion node can also verify another recursion node's
proof** — so we fold pairs up a **binary tree**: leaves verify
segment proofs, internal nodes verify two child recursion proofs
and check the batons stitch across the *spans* they cover, and
the **root** proves "the entire line `[0, N-1]` is valid, the
first baton was the zero state, and the last baton produced the
final `HASH_JACKPOT`." The chain verifies **one** root proof.

### 1.3 Why this is *clean* and not a kludge

- The thing being recursively verified is *uniform*: every
  segment uses the same composite AIR (only boundary predicates
  swap, and those are public). So one recursion circuit handles
  every segment and — because we make the aggregate proof have
  the same external shape — every tree level. One circuit,
  arbitrary depth: textbook recursion.
- The "stitch" is just **field-element equality of `Γ`**, a
  vector of ≈90 Goldilocks elements. No re-execution, no
  re-hashing of the matmul. The recursion node's only real work
  is *verifying a STARK* (FRI + Merkle + one constraint
  evaluation) — and that work is **independent of the segment's
  size** (a 2²⁰-row segment and an 2¹³-row segment have
  similarly-sized proofs and identical verifier work).
- Soundness reduces to an induction over the baton chain plus the
  existing CRIT-1 program-pinning, applied per segment. No new
  cryptographic assumption; FRI stays transparent.

### 1.4 One-sentence summary

> Split the giant useful-work trace into fixed-size segments that
> pass a tiny verifier-checked baton; prove each segment with the
> existing Layer-0 STARK; then prove-the-proofs in a binary tree
> of one self-similar recursion circuit, so the chain checks a
> single root proof that is, by induction, equivalent to having
> proved the monolith.

The rest of this document makes every word of that sentence
precise.

---

## 2. Background: precisely what a Layer-0 *segment proof* is

A segment proof is exactly today's Route-A artifact, with the
boundary predicates parameterized (G3a). Concretely:

- **AIR:** `CompositeFullAirWithLookupsPinned` (CRIT-1
  program-pin + `noised_packed`/range/i8u8/cv LogUp + §4.D/§6(b)
  keystones), width `TOTAL_TRACE_WIDTH`, height `S =
  MAX_SEGMENT_ROWS` (a fixed power of two ≥ `MIN_STARK_LEN`).
- **Proof system:** `p3-batch-stark::prove_batch` over
  `AiPowStarkConfig` (Goldilocks base field, degree-2 extension
  for FRI, `DuplexChallenger<Goldilocks, Tip5>` transcript,
  FRI ~120-bit soundness — see memory `ai_pow_zk_fri_sweep`).
- **Preprocessed (CRIT-1) trace:** the canonical program for
  *this segment*, `program_k = canonical(params, k)` — the 5
  `PROGRAM_COLS` (`CONTROL_PREP` etc., now incl. the §6(a)/G2
  fold-schedule + 6-bit stripe index) restricted to the segment's
  row range, rebuilt witness-free by the verifier from trusted
  shape (`extract_program` discipline; §6 below extends this
  across the tree).
- **Public inputs (the segment's external statement):**

  ```text
  SegmentPI = {
    seg_index      : u32          // k  (pinned; orders the segment)
    n_segments     : u32          // N  (pinned from params)
    role           : {First, Mid, Final}    // derived from k vs N
    gamma_in       : Γ            // baton received (= 0 iff First)
    gamma_out      : Γ            // baton passed on
    // Block-anchor PIs (existing C1/C3/C4 set, unchanged):
    job_key, commitment_hash, hash_a, hash_b, hash_jackpot,
    cumsum, jackpot               // (cumsum/jackpot are last-row PIs;
                                  //  HASH_JACKPOT meaningful only on Final)
  }
  ```

- **Boundary predicates (the only AIR change vs today — G3a):**
  per-row constraints are byte-identical; the StripeXor / matmul
  / FoldChip `when_first_row` predicates read `gamma_in` instead
  of the literal `0`; the last row exposes `gamma_out`; the §4.D
  + §6(b) keystones and the C2 difficulty binding are gated to
  fire **only when `role == Final`**. `N = 1` ⇒ `role = First ∧
  Final`, `gamma_in = 0` ⇒ **bit-identical to the current
  single-STARK path** (this is the regression-safety invariant
  and G3a's acceptance test).

`Γ` is defined in §4.

---

## 3. The recursion primitive: verifying a STARK inside a STARK

### 3.1 What the `p3-batch-stark` verifier actually does

`verify_batch(config, airs, proof, public_values, common)` (see
`composite_verify_pinned_logup`) performs, for a `BatchProof`:

1. **Transcript replay.** Instantiate `DuplexChallenger<Goldilocks,
   Tip5>`; absorb the preprocessed (program) commitment, the main
   trace commitment, and the public-input vector; squeeze the
   LogUp/permutation challenges; absorb the permutation
   commitment; squeeze the constraint-combination challenge
   `α`; absorb the quotient commitment; squeeze the
   out-of-domain (OOD) point `ζ`; squeeze the FRI folding
   challenges and query indices.
2. **LogUp check.** The permutation argument's cumulative sum is
   zero (the bus balances) — a field check on opened values.
3. **Quotient/constraint identity at `ζ`.** Evaluate the AIR's
   combined constraint polynomial `C(ζ)` from the opened
   trace/preprocessed/permutation values and check
   `C(ζ) == Z_H(ζ)·Q(ζ)` (vanishing-poly relation), with the
   boundary predicates parameterized by `SegmentPI`.
4. **FRI low-degree test.** Verify the FRI proof: each folding
   round's consistency + the final-polynomial degree + the query
   openings are consistent with the folded values.
5. **Merkle openings.** For every queried index, verify Merkle
   authentication paths into the trace / preprocessed / quotient
   / permutation oracles (Tip5 2-to-1 compressions).

### 3.2 Arithmetizing it: the `RecursionNode` AIR

The recursion node is an AIR `R` whose execution trace *is the
run of the verifier of step 3.1* on a child proof. Each
sub-task maps to constraints:

| verifier sub-task | arithmetization | dominant cost |
|---|---|---|
| FS transcript | in-circuit Tip5 sponge (absorb/squeeze) | cheap (Tip5 is algebraic, ~few cols/perm) |
| LogUp / quotient identity at `ζ` | field arithmetic; **embeds one symbolic evaluation of the composite AIR's constraint set** at `ζ` | moderate, `O(#constraints)`, **once per verified proof** (not per row) |
| FRI folding | field linear combinations + index bit-decomp | moderate |
| Merkle path checks | in-circuit Tip5 compressions, `#queries × tree_depth` | **the bulk** of the gate count |

Key structural facts that make this *clean*:

- **Tip5 was chosen for recursion.** The whole transcript +
  Merkle layer is Tip5; Tip5 is an algebraic permutation with a
  compact AIR (this is precisely why the project uses
  `DuplexChallenger<Goldilocks, Tip5>` rather than a byte hash).
  The recursion cost is dominated by a *fixed* number of Tip5
  permutations ≈ `#FRI_queries × (tree_depth + folding_rounds)`
  — **independent of the verified segment's height** (a 2²⁰-row
  segment commits a taller tree, ≈ +7 Merkle levels vs 2¹³;
  linear-in-`log`, negligible).
- **One constraint-evaluator, parameterized.** Because every
  segment uses the *same* `CompositeFullAirWithLookupsPinned`,
  `R` embeds exactly one symbolic evaluator of that AIR's
  constraints, parameterized by the child's `SegmentPI`
  (`gamma_in`, `role`, `seg_index`). This is the only place the
  composite AIR's algebraic complexity enters the recursion, and
  it enters **once**, at the single OOD point `ζ` — *not*
  per-row. This is the crux of why recursion is cheap relative
  to the segment.
- **Self-similar.** `R` verifies a proof against a *verifying
  key* (preprocessed commitment + AIR shape descriptor). A pinned
  `kind ∈ {Segment, Agg}` public bit selects which VK to use: the
  composite-AIR VK for a segment child, or `R`'s *own* VK for an
  aggregate child. A circuit that can verify proofs of itself ⇒
  unbounded recursion depth ⇒ aggregation trees of any `N`.

### 3.3 Cost model (order-of-magnitude, to size G3c)

Let `q` = `#FRI_queries` (≈ 84–100 for 120-bit soundness, see
`ai_pow_zk_fri_sweep`), `d` = Merkle tree depth (= `log2 S` +
oracle count ≈ 20–25 at PROD `S`), `f` = FRI folding rounds
(≈ `log2 S`). A recursion node ≈

```
  cost(R) ≈ q·(d + f) Tip5 permutations
          + 1 composite-AIR constraint evaluation at ζ
          + O(q·f) field ops
```

This is **constant in the segment trace area** (only `log`-terms
in `S`). Therefore: a recursion node is *far* smaller than the
2²⁰-row segment it verifies, and itself fits comfortably in one
Layer-0 STARK (`MIN_STARK_LEN`-class). Aggregation is `O(N)`
recursion-node proofs over a binary tree of depth `⌈log2 N⌉`
(PROD `N` ≈ tens → depth ≈ 5–6). The aggregate-prover wall-clock
is dominated by the `N` *segment* proofs, not the recursion
(recursion is the cheap glue) — which is exactly why this scales.

---

## 4. The carry vector `Γ` and the uniform claim

### 4.1 `Γ` — exactly the cross-segment-stateful registers

A register must be in `Γ` iff its value on a segment's first row
depends on the previous segment (i.e. it has a `when_first_row`
boundary predicate or a cross-row recurrence that a cut splits):

```text
Γ = {
  cumsum    : [i32; 4]            // matmul accumulator (sub-block-major
                                  //   single threaded chain; §6(b) GATE-2)
  sx_xr     : [i32; STRIPE_MAX=64]// StripeXor per-stripe XOR register
  fold_state: [u32; 16]           // FoldChip M (Pearl §4.5 JACKPOT_SIZE)
  mhash_a   : MerkleRunState      // chunk-Merkle running CV+node stack of
  mhash_b   : MerkleRunState      //   the C3 hash of A / B
  row_idx   : u64                 // STARK_ROW_IDX continuity (monotone)
}
```

≈ `4 + 64 + 16 + |MerkleRunState|·2 + 1` ≈ **90–130 Goldilocks
elements**. `MerkleRunState` = (current CV `[u32;8]`, the
log-depth node stack of partial subtree roots) — its exact width
is fixed by `place_matrix_hash_*`'s chunk-Merkle and is a small
constant (≈ 8 + 8·log2(maxchunks)).

`Γ` is small, *flat*, and field-encoded — equality is `≈130`
`assert_eq`s in the recursion node. Nothing about the matmul is
re-derived.

### 4.2 The uniform `AggClaim` (what every proof in the tree
attests)

Both a leaf recursion proof and an internal one expose the same
public statement, so internal nodes treat children uniformly:

```text
AggClaim = {
  span_lo     : u32              // first segment index covered
  span_hi     : u32              // last  segment index covered
  gamma_lo    : Γ                // baton entering segment span_lo
  gamma_hi    : Γ                // baton leaving  segment span_hi
  program_root: Digest           // CRIT-1-across-tree (see §6)
  block_anchor: Digest           // H(job_key ‖ commitment_hash ‖
                                  //   hash_a ‖ hash_b)  — ties the whole
                                  //   span to ONE block (C1/C3)
  n_segments  : u32              // N (pinned from params; constant up tree)
  final_seen  : bool             // does this span include segment N-1?
  hash_jackpot: [u32;8]          // defined iff final_seen (the PoW digest)
}
```

A **segment**'s `SegmentPI` is the leaf-level instance of this
(`span_lo = span_hi = seg_index`, `gamma_lo = gamma_in`,
`gamma_hi = gamma_out`, `block_anchor` = hash of its C1/C3 PIs,
`final_seen = (role == Final)`).

---

## 5. The aggregation tree

### 5.1 The single recursion AIR `R`

`R` takes **one or two child proofs** + their claimed
`AggClaim`s + a pinned `kind` per child, and emits a merged
`AggClaim`. One AIR, three behaviours selected by verifier-fixed
public flags (no AIR fork):

#### 5.2 Leaf behaviour — `kind = Segment` (verify one segment)

`R` with a single `Segment` child:

1. **Verify** the segment `BatchProof` (the §3.2 sub-circuit)
   against the composite-AIR verifying key and the child's
   `SegmentPI`.
2. **Bind the program:** check `commit(program_segindex)` is the
   `seg_index`-th leaf under `program_root` (Merkle membership;
   §6). This is CRIT-1 *for this segment*.
3. **Emit** `AggClaim{ span=[k,k], gamma_lo=gamma_in,
   gamma_hi=gamma_out, program_root, block_anchor=H(C1‖C3),
   n_segments, final_seen=(role==Final),
   hash_jackpot=(if Final) }`.

#### 5.3 Internal behaviour — two children (each `Segment` *or*
`Agg`)

`R` with children `L, Rt` (the `kind` bit per child selects
"verify against composite-AIR VK" vs "verify against `R`'s own
VK"):

1. **Verify** both child proofs (§3.2 sub-circuit, VK chosen by
   `kind`).
2. **Adjacency:** `L.span_hi + 1 == Rt.span_lo`.
3. **Carry stitch:** `L.gamma_hi == Rt.gamma_lo`  *(the baton
   link — ≈130 field equalities)*.
4. **Consistency:** `L.program_root == Rt.program_root`,
   `L.block_anchor == Rt.block_anchor`,
   `L.n_segments == Rt.n_segments`,
   `¬(L.final_seen ∧ Rt.final_seen)`,
   `Rt.final_seen ⇒ Rt.span_hi == n_segments-1`.
5. **Emit** `AggClaim{ span=[L.span_lo, Rt.span_hi],
   gamma_lo=L.gamma_lo, gamma_hi=Rt.gamma_hi, program_root,
   block_anchor, n_segments,
   final_seen=L.final_seen∨Rt.final_seen,
   hash_jackpot = (Rt.final_seen ? Rt.hash_jackpot
                                 : L.hash_jackpot if L.final_seen) }`.

#### 5.4 Root behaviour — the chain's single check

The chain-level verifier (in `ai_pow::zk_bridge`, MED-3
discipline) receives the **root** `R`-proof + its `AggClaim` and,
*recomputing everything from chain-pinned `params`* (never the
proof), checks:

- the root `R`-proof verifies (one STARK verify, off-chain-cheap);
- `span == [0, n_segments-1]` and `n_segments ==
  num_segments(params)`;
- `gamma_lo == Γ_ZERO` (the canonical initial baton: `cumsum=0`,
  `sx_xr=0`, `fold_state=0`, empty Merkle run-state,
  `row_idx=0`);
- `program_root == PROGRAM_ROOT(params)` (§6 — verifier-recomputed);
- `block_anchor == H(job_key ‖ commitment_hash ‖ hash_a ‖
  hash_b)` against the values the *plain* `BlockContext` exposes
  (the existing `prove_and_verify` cross-checks, now at the
  aggregate boundary);
- `final_seen == true`;
- **C2 / MED-3:** `hash_jackpot ≤ difficulty_target(params)`
  (`composite_verify_pow*` discipline, unchanged — difficulty
  stays the Pearl-Layer-0-faithful external check, on the *root*
  digest).

If all hold, the block's PoUW is accepted. **One proof, one
verify.**

#### 5.5 `N = 1` specialization (today's path is a strict subset)

For every non-PROD params set `N = 1`. The tree degenerates to a
single leaf node with `span=[0,0]`, `gamma_lo=gamma_hi=Γ_ZERO`
(`role=First∧Final`), `program_root` = the 1-leaf root. With
G3a's single-segment default the *segment* proof is
bit-identical to today's `composite_prove_pinned_logup_sx(..,
sx_bound)`. **G3 must preserve this exactly** — it is the primary
non-regression invariant (acceptance: every existing
`routea_*`/`high2_*`/`crit1_*`/e2e test passes unchanged with
`N=1`, and the leaf recursion node over a single segment yields
the same accept/reject as direct verification).

#### 5.6 Unbalanced `N`, padding

`N` is rarely a power of two. Use a **left-leaning binary tree**
(any deterministic shape derivable from `N`): the recursion AIR
already handles "child is Segment vs Agg" uniformly, so an
unbalanced tree needs no special node. The shape is a pure
function of `N` (hence of `params`) so the verifier knows it; the
adjacency/`final_seen` checks (§5.3) make any wrong shape
unprovable. No "identity proof" padding is required (avoid it —
padding nodes are extra soundness surface); an odd node simply
carries up one level unverified-again-free as a `kind=Agg` child
of its grandparent (standard Huffman-free left-leaning fold).

### 5.7 Why this is genuine recursion (not just batching)

Because `R` verifies *either* a composite-AIR proof *or* an
`R`-proof (selected by the pinned `kind`, choosing the VK), and
emits the same `AggClaim` shape it consumes, `R` is a circuit
that verifies proofs of itself. The aggregation tree can be
arbitrarily deep with a **fixed-size** recursion circuit — the
defining property of recursive proof composition (cf.
Plonky2/Plonky3 recursion, Halo-style accumulation). Depth is
`⌈log2 N⌉`; nothing in `R` grows with `N`.

---

## 6. Extending CRIT-1 across the tree: `PROGRAM_ROOT(params)`

CRIT-1 today: the verifier rebuilds the canonical program
witness-free from trusted shape and the proof is checked against
its preprocessed commitment, so a malicious prover cannot pick
the selector schedule. Across segments this must become: *every*
segment was proved against the *correct per-segment* canonical
program, and segments cannot be reordered/duplicated/dropped or
swapped for a cheaper program.

**Construction (verifier-recomputable, MED-3 discipline).**

1. `canonical(params, k)` — the `S`-row preprocessed program for
   segment `k` — is a pure deterministic function of `params`
   (the global schedule restricted to `[k·S,(k+1)·S)`; the §6(a)
   `CONTROL_PREP` selector/fold/stripe pack + the §6(a)-pattern
   6-bit stripe index + the matmul/`AB_ID` schedule, all already
   params-derived).
2. `c_k = commit(canonical(params, k))` — its Tip5 Merkle
   commitment (the same commitment `ProverData::from_airs_and_
   degrees` produces verifier-side today).
3. `PROGRAM_ROOT(params) = MerkleRoot({c_0, …, c_{N-1}})` over a
   fixed-arity Tip5 tree, `N = num_segments(params)`.

`PROGRAM_ROOT(params)` is computed **once, off-circuit, by the
verifier** from chain-pinned `params` (exactly like
`difficulty_target(params)` / `tile_ij` — never prover-supplied).
- A **leaf** recursion node proves: "segment proof verifies
  against preprocessed commitment `c`, and `c` is the
  `seg_index`-th leaf on a Merkle path to `program_root`"
  (`seg_index` pinned in the segment's `SegmentPI`/program).
- **Internal** nodes propagate a single `program_root` (checked
  equal across children).
- **Root** check: `program_root == PROGRAM_ROOT(params)`.

This binds, with no in-circuit program recomputation: the
correct program *per index*, the index → no reorder/duplicate,
the count `N` → no drop (a dropped segment leaves a span gap
caught by §5.3 adjacency + the root `span==[0,N-1]`), and the
*set* → no cheaper-program swap (its `c` is not the pinned leaf).
CRIT-1's guarantee is thus preserved verbatim, per segment,
across the whole tree.

---

## 7. Fiat–Shamir & domain separation across layers

Each proof has its **own self-contained transcript**
(`DuplexChallenger<Goldilocks, Tip5>`): a segment's challenges
derive from *its* commitments + *its* `SegmentPI` (which includes
`seg_index`, `gamma_in`, `gamma_out`, `n_segments`, `role`); a
recursion node's challenges derive from *its* commitments + *its*
`AggClaim` + the child proofs it absorbs. Requirements:

- **Domain separation tag.** Absorb a constant
  `LAYER_TAG ∈ {Segment, Recursion}` (and the AIR identifier)
  into every transcript at initialization, so a segment proof can
  never be reinterpreted as a recursion proof or vice-versa.
- **Public-input binding.** All of `SegmentPI` / `AggClaim`
  (esp. `gamma_*`, `seg_index`, `span_*`, `program_root`,
  `block_anchor`, `n_segments`) are absorbed *before* any
  challenge is squeezed — so the statement is bound; an adversary
  cannot grind a transcript that proves a *different* baton/span.
- **Child-proof binding.** A recursion node absorbs each child's
  proof transcript-commitment + claimed `AggClaim` before
  squeezing its own challenges (the child proof and its asserted
  claim are jointly bound).
- **Soundness of FS as applied** is inherited from the existing
  analysis (`ZKP_SECURITY_REPORT.md` "Fiat-Shamir soundness":
  upstream `p3-uni-stark`/`p3-batch-stark` observe commitments +
  PIs into the duplex sponge before drawing challenges). G3 adds
  *only* the layer tag + the `Γ`/span/program-root fields to the
  absorbed PI set; it introduces no new FS construction.

---

## 8. Soundness: theorem, proof sketch, error budget

### 8.1 Theorem (informal)

> Let `params` be chain-pinned, `N = num_segments(params)`,
> `T_0…T_{N-1}` the canonical `S`-row segmentation of the
> monolithic composite trace `T`. If the chain-level root check
> (§5.4) accepts a root `R`-proof, then — except with probability
> ≤ `ε_total` (§8.3) — there exists a single trace `T*` of height
> `N·S` such that `T*` satisfies the full composite AIR
> (`CompositeFullAirWithLookupsPinned`) + the §4.D/§6(b)
> keystones + C2 against `difficulty_target(params)`, with the
> CRIT-1 preprocessed program `canonical(params)` and the
> block-anchored C1/C3/C4 PIs. I.e. acceptance is equivalent to
> acceptance of a single monolithic §6(b) proof — **no
> probabilistic gap** (contrast G4).

### 8.2 Proof sketch (induction over the carry chain)

By **knowledge soundness of the STARK** (FRI + FS, ~120-bit),
each verified object yields an extractable satisfying witness:

- *Leaf:* a valid segment proof for `SegmentPI` ⇒ a satisfying
  `S`-row trace `T_k` for `CompositeFullAirWithLookupsPinned`
  with `firstrow.register == gamma_in`, `lastrow.register ==
  gamma_out`, against `canonical(params,k)` (the §6 Merkle
  binding forces the *correct* program for `k`).
- *Inductive step:* an internal `R`-proof ⇒ both children
  extract to valid spans, `L.gamma_hi == Rt.gamma_lo` (the
  stitch), adjacent spans, equal `program_root`/`block_anchor`/
  `n_segments`. Concatenate the extracted traces: the boundary
  predicate of `Rt`'s first segment reads `gamma_in =
  L.gamma_hi = ` the actual last-row registers of `L`'s last
  segment — so the join row satisfies the *same* cross-row
  recurrence the monolith would (matmul `nxt==compute_row(cur)`,
  StripeXor passthrough/active, fold) because those constraints
  are local (cur,nxt) and were proved on both sides with the
  shared boundary value. Hence the concatenation is itself a
  satisfying trace for the merged span.
- *Root:* `span==[0,N-1]`, `gamma_lo==Γ_ZERO` (canonical
  start), `program_root==PROGRAM_ROOT(params)`,
  `n_segments==num_segments(params)`, `final_seen`,
  `block_anchor` matches the block ⇒ the fully-concatenated `T*`
  is a satisfying monolithic trace; `Final`'s keystones (§4.D
  `JACKPOT_MSG==FOLD_STATE==M`, §6(b) `FOLD_XSTEP==SX_XR[stripe]`)
  held on its last row, and C2 binds the root `hash_jackpot ≤
  difficulty_target(params)`. ∎

The new (G3-specific) soundness obligations and how the proof
discharges each:

| obligation | discharged by |
|---|---|
| segments not reordered/duplicated | `seg_index`-pinned Merkle leaf under `program_root` + §5.3 adjacency + root `span==[0,N-1]` |
| no dropped (e.g. matmul) segment | span coverage `[0,N-1]` + `N` pinned from params; a gap is a non-adjacency |
| no cheaper-program swap | `c_k` must be the pinned `program_root` leaf for `k` |
| carry forgery | `gamma_*` are FS-bound STARK public IO, equality-checked by `R` |
| mixed-block splice | `block_anchor` equal across all nodes + root-checked vs the block's C1/C3 |
| layer confusion (segment↔recursion) | `LAYER_TAG` domain separation (§7) |

### 8.3 Error budget

Let `ε_stark` be one STARK's knowledge-soundness error
(FRI ~`2⁻¹²⁰` at the configured query count, see
`ai_pow_zk_fri_sweep`; PROD held at LB=3). The tree has `N`
segment proofs + `≤ N-1` recursion nodes ⇒ `≤ 2N` verified
objects. By a union bound:

```
  ε_total ≤ 2N · ε_stark + ε_FS
```

For PROD `N` ≈ a few tens (`rows(params)/S`), `2N ≲ 2⁷`, so
`ε_total ≲ 2⁻¹¹³` — negligible, and *additively* (not
multiplicatively) degrading in depth (each recursion verifies its
child *exactly*, in-circuit; depth does not compound the FRI
error — it only adds one `ε_stark` per node). This is the
standard recursive-STARK error accounting and is why aggregation
depth is "free" soundness-wise. (Tighten the constant at
implementation by setting `S` and the FRI query count from the
target `ε_total` and the chosen `N` range.)

---

## 9. Determinism, transparency, public-coin, no trusted setup

- **No trusted setup.** Everything is FRI (transparent); the
  recursion adds only more FRI/Tip5 — still transparent. There is
  no SRS, no toxic waste, at any tree level.
- **Public-coin / deterministic verification.** `S`,
  `num_segments(params)`, the tree shape, `canonical(params,k)`,
  `PROGRAM_ROOT(params)`, `Γ_ZERO`, `difficulty_target(params)`
  are *pure functions of chain-pinned `params`*. The chain
  verifier recomputes all of them; **nothing soundness-bearing
  is taken from the prover** (MED-3/CRIT-1 discipline, extended).
- **Succinct chain check.** The chain verifies exactly one root
  `R`-proof — a single STARK verification whose cost is
  independent of `N` and of the PROD trace size (it is the cost
  of verifying one fixed-size recursion proof).

---

## 10. Concrete API / types and mapping to existing code

Rust-level sketch (names indicative; lives in a new
`ai_pow_zk::recursion` module + `ai_pow::zk_bridge` glue):

```rust
// ── carry ───────────────────────────────────────────────────
pub struct Gamma {
    pub cumsum:    [i32; 4],
    pub sx_xr:     [i32; STRIPE_MAX],     // 64
    pub fold_state:[u32; JACKPOT_SIZE],   // 16
    pub mhash_a:   MerkleRunState,
    pub mhash_b:   MerkleRunState,
    pub row_idx:   u64,
}
impl Gamma { pub const ZERO: Gamma = /* all-0 / empty */; }

// ── segment (G3a/G3b; M12-independent) ──────────────────────
pub struct SegmentRole { pub is_first: bool, pub is_final: bool }
pub struct SegmentPI {
    pub seg_index: u32, pub n_segments: u32, pub role: SegmentRole,
    pub gamma_in: Gamma, pub gamma_out: Gamma,
    pub c1c3c4: CompositePublicInputs,    // existing
}
/// G3a: parameterized boundary predicates. `prog` =
/// canonical(params, seg_index). Single-segment default
/// (n=1, role=First∧Final, gamma_in=ZERO) == today's path.
pub fn prove_segment(cfg, trace, prog, pi: &SegmentPI)
    -> SegmentProof;                       // == composite_prove_pinned_logup_sx
pub fn verify_segment(cfg, prog, &SegmentProof, &SegmentPI)
    -> Result<(), _>;                      // == composite_verify_pinned_logup_sx

// ── schedule (G3b; M12-independent, params-pure) ────────────
pub fn num_segments(params) -> u32;
pub fn canonical_segment_program(params, k: u32) -> Program;
pub fn program_root(params) -> Digest;     // verifier-recomputed
pub fn segmentation_plan(params)           // row-ranges, roles
    -> Vec<(Range<usize>, SegmentRole)>;

// ── recursion / aggregation (G3c; the M12 circuit) ──────────
pub struct AggClaim { /* §4.2 */ }
pub enum  ChildKind { Segment, Agg }
pub struct RecursionNodeAir { /* the §3.2 verifier-in-AIR */ }
pub fn agg_leaf (cfg, &SegmentProof, &SegmentPI)        -> AggProof;
pub fn agg_node (cfg, left:(&AggProof,&AggClaim, ChildKind),
                      right:(&AggProof,&AggClaim,ChildKind)) -> AggProof;
pub fn aggregate(cfg, segs: &[(SegmentProof, SegmentPI)]) -> AggProof; // builds the tree
/// chain-level (ai_pow::zk_bridge, MED-3 discipline):
pub fn verify_pow_aggregate(params, target, &AggProof, &AggClaim)
    -> Result<(), PowVerifyError>;
```

**Mapping to today.** `prove_segment`/`verify_segment` are
exactly the existing `composite_{prove,verify}_pinned_logup_sx`
with `SegmentPI`-parameterized boundary predicates;
`verify_pow_aggregate` is the aggregate analogue of
`prove_and_verify_for_block` (re-derives `target`, `program_root`,
`Γ_ZERO`, `num_segments` from chain-pinned `params`; the existing
C1/C3/C4 `BlockContext` cross-checks move to the root). `N=1` ⇒
`aggregate` is a single `agg_leaf` whose inner segment proof is
byte-identical to today's Route-A artifact.

---

## 11. Edge cases & correctness details

- **Last segment shorter than `S`.** Pad to `S` with the existing
  all-zero/passthrough rows (every chip is vacuous on baseline
  rows; `CUMSUM`/`SX_XR`/`FOLD_STATE` pass through via the
  existing `fill_*_passthrough`). The §4.D/§6(b) keystones fire
  on the row where `M`/`HASH_JACKPOT` actually land; place the
  fold + jackpot block so they land in the final segment before
  its padding — identical to today's `place_fold_chain` +
  `place_jackpot_hash_block` tail. No new mechanism.
- **Matrix-hash spanning many segments.** `place_matrix_hash_*`'s
  chunk-Merkle becomes a streaming computation whose running
  CV/node-stack is `Γ.mhash_{a,b}`. The final root
  (`HASH_A`/`HASH_B`) is produced when the last hash chunk is
  consumed (some segment `< N-1`); thereafter it is a constant
  carried in `Γ` (or, equivalently, exposed as the
  `block_anchor`) and root-checked vs the block. C3's
  `BLAKE3_MSG↔UINT8_DATA` etc. are per-row (in-segment),
  unaffected.
- **Fold/jackpot entirely in the final segment (common).** Then
  `Γ.fold_state` is `0` until the final segment and the fold is
  local — the general carry handles the rare case where the fold
  chain itself straddles a boundary with no special code.
- **`N = 1`.** §5.5 — strict subset of today; the regression
  invariant.
- **Aggregation tree shape.** A pure function of `N` (hence
  `params`); the verifier knows it; wrong shapes are unprovable
  via adjacency + `span==[0,N-1]` + `n_segments` (§5.3/§5.4). No
  identity-padding nodes (keep the soundness surface minimal).
- **`Γ` field encoding.** `cumsum`/`sx_xr` carry **signed** i32
  in the same `QuotientMap<i64>` encoding the matmul `CUMSUM` and
  the §6(b) `SX_IN` already use (the signed-IN fix, commit
  `c63fbc1`) — the recursion's `Γ` equality must use that exact
  encoding, not a u32 reinterpretation, or it silently fails for
  negative accumulators (the bug-class already root-caused once).

---

## 12. Phasing: G3a / G3b / G3c with acceptance criteria

**G3a — boundary-predicate parameterization (M12-independent;
implementable & exhaustively testable now).**
- *Do:* add `SegmentPI`/`SegmentRole`/`Gamma`; make the StripeXor
  / matmul / FoldChip `when_first_row` predicates read
  `gamma_in`, expose `gamma_out` on the last row, gate
  §4.D/§6(b)/C2 by `role.is_final`. Default `SegmentPI`
  (`n=1, First∧Final, gamma_in=ZERO`).
- *Accept:* (i) **zero regression** — all
  `routea_*`/`crit1_*`/`high2_*`/composite + `ai-pow --features
  zk` e2e pass unchanged with the default `SegmentPI`
  (bit-identical proofs to today); (ii) a **2-segment split
  test**: take a small TEST_SMALL useful-work chain, cut it at an
  arbitrary row, hand-thread `Γ`, `prove_segment` both halves,
  assert `seg0.gamma_out == seg1.gamma_in` and that the final
  `HASH_JACKPOT` equals the single-segment one; (iii) the
  split-point may fall mid-sub-block-run / mid-chunk / mid-fold —
  all must verify (carry transparency, debug-assertions ON).

**G3b — segmentation schedule + per-segment program
(M12-independent; params-pure).**
- *Do:* `num_segments`, `segmentation_plan`,
  `canonical_segment_program`, `program_root`; pin `seg_index`
  into the segment program (extend the §6(a) `CONTROL_PREP`
  pattern with a segment-index field, *or* a dedicated 1-col
  pinned `SEGMENT_IDX` — decide per width/cost at impl).
- *Accept:* `program_root(params)` is deterministic &
  verifier-recomputable; a tampered/reordered per-segment program
  fails Merkle membership; `N=1` ⇒ `program_root` = the existing
  single program's commitment (continuity).

**G3c — the M12 recursion verifier + aggregation (recursion-stack
dependent; the heavy part).**
- *Do:* `RecursionNodeAir` (§3.2 verifier-in-AIR over
  `p3-batch-stark`), the self-similar `kind`-selected VK,
  `agg_leaf`/`agg_node`/`aggregate`, `verify_pow_aggregate`,
  `LAYER_TAG` domain separation.
- *Accept:* end-to-end PROD-shaped (or scaled-down multi-segment)
  block proves+verifies through the tree; the §8.2 obligations
  each have a red-team test (forged carry, dropped/reordered
  segment, swapped program, mixed-block splice, layer-confusion
  — **all rejected**); `N=1` aggregate ≡ today's single proof
  (accept/reject parity); recursion-node cost is constant in
  segment size (measure).

G3a+G3b deliver a **multi-segment-capable Layer-0** with the full
soundness *interface* in place, independent of the recursion
stack; G3c is purely the recursion circuit.

---

## 13. Relationship to Pearl; what changes vs the G4 interim

Pearl's protocol is *already* Layer-0 + recursion/aggregation
(Pearl proves bounded work and aggregates; matmul-truth at scale
rests on the §4.8 spot-check protocol — probabilistic, external).
G3 is the faithful instantiation of that architecture for
`ai-pow-zk`, with one upgrade: because each segment carries the
*full* §6(b) in-circuit binding (`X_STEP ← ⊕CUMSUM ← committed
A/B`) and the recursion stitches it deterministically, the
aggregate proves the matmul **with zero probabilistic gap** —
*strictly stronger* than Pearl's spot-checks.

- **Until G3c lands:** true-PROD takes the legacy path, the §6(b)
  keystone gated off via the verifier-set `sx_bound`; matmul-truth
  for PROD is carried by **G4** (the Pearl §4.8 spot-check
  externality + C3 commitment) — *parity with Pearl*, a documented
  scoped externality analogous to MED-3, **not** a forgery hole
  (CRIT-1 + §4.D keystone + §6(a) + §6(b)-for-single-Layer-0 hold
  unconditionally).
- **When G3c lands:** the spot-check externality for *matmul
  truth* is **removed** (G3 supersedes it); difficulty (MED-3)
  stays Pearl-Layer-0-faithfully external.
- **Orthogonal:** G3 preserves the §4.C `noised_packed` LogUp
  *per segment*; it does **not** fix the deep
  tile↔committed-store §4.C-non-vaciety residual (#108) — that is
  independent.

---

## 14. Open parameters to pin at implementation

| parameter | how to choose |
|---|---|
| `S = MAX_SEGMENT_ROWS` | largest power-of-two `≥ MIN_STARK_LEN` whose segment proof is economical AND keeps `N` (hence tree depth + `ε_total`) acceptable; profile via `scripts/profile_f1.sh`. |
| FRI query count per layer | from target `ε_total` (§8.3) given the chosen `N` range; reuse the `ai_pow_zk_fri_sweep` methodology (PROD held LB=3). |
| `seg_index` pin mechanism | extend §6(a) `CONTROL_PREP` pack (cheap, no width) **vs** a dedicated pinned `SEGMENT_IDX` col (clearer) — measure. |
| `MerkleRunState` width | fixed by `place_matrix_hash_*` chunk-Merkle; compute exactly from PROD matrix dims. |
| tree shape | left-leaning deterministic from `N` (default); revisit only if recursion-node cost asymmetry matters. |
| recursion field | same Goldilocks + Tip5 (self-similar, no field switch); confirm `p3-batch-stark` recursion-friendliness at impl. |

---

## 15. Cross-references

- `HIGH2_2_DESIGN.md` §4.C.4-G (G1–G4 overview), §4.C.4-G3
  (summary that points here), §4.C.10 (Route-A), §6(a)
  (`CONTROL_PREP` schedule pin — the pattern G3b extends),
  "Remaining soundness scope".
- `ZKP_SECURITY_REPORT.md` — CRIT-1 (program pin G6 extends),
  MED-3 (verifier-recomputation discipline G3 reuses),
  Fiat-Shamir soundness (§7 inherits).
- `composite_proof.rs` — `composite_prove_pinned_logup_sx` /
  `composite_verify_pinned_logup_sx` (= `prove_segment` /
  `verify_segment`), `extract_program` (= per-segment
  `canonical`), `logup_common_for` (verifier-side commitment
  rebuild — the §6 building block).
- `composite_full_air.rs` — `CompositeFullAirPinned`/`WithLookups`
  (the AIR G3a parameterizes), the §4.D/§6(b) keystones.
- `composite_trace.rs` — `place_useful_work_chain` (the swept
  body G3a segments), `fill_*_passthrough` (the padding G3 reuses).
- Task **#108** tracks G3 (+ the orthogonal §4.C residual).
- Memory: `ai_pow_zk_crypto_gaps` (G1+G2 done, G3 scoped),
  `ai_pow_zk_fri_sweep` (FRI soundness budget).
```
