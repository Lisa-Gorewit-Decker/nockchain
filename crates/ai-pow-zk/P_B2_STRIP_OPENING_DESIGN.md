# P-B.2 — Pearl §4.6 Strip-Opening Matrix-Hash (implementation design)

> **Status:** **P-B.2.0 ✅ LANDED 2026-05-17**
> (`crates/ai-pow-zk/src/blake3_tree.rs`). Decisions D1–D4
> locked (all recommended). **D1 finding — the "latent
> pre-existing fidelity gap" hypothesis is DISPROVEN by
> P-B.2.0's exhaustive KAT:** `place_matrix_hash`'s bottom-up
> *pair-adjacent / promote-odd* parent reduction is
> **structurally identical to BLAKE3's largest-pow2-left tree
> for every chunk count** (pow2 *and* non-pow2, verified
> in-circuit 1..=31 + 100, and walker-vs-`blake3::Hasher`
> through 1000). ⇒ **D1-A's "realign `place_matrix_hash`" is a
> no-op**, and **P-B.2.1 collapses into P-B.2.0** (it *is* the
> equivalence verification). Remaining: D2/D3/D4 as locked. The
> body's D1 §4 / §8 plan are corrected accordingly below.
> **D1=A** (true tree confirmed already in place — no realign) ·
> **D2=A** (whole 1024-B chunks) · **D3=A** (verifier-recompute
> schedule via CRIT-1/MED-3, no new column) · **D4=A** (staged,
> KAT-first — KAT done).
> **Goal:** make the Pearl-faithful PROD path genuinely
> *one-tile-one-STARK* by removing the only remaining
> one-STARK blocker that P-B's measurement isolated — the
> in-circuit **full-matrix** chunk-Merkle re-hash.
> **Predecessors (done, committed):** P-A (`676d707`, Pearl §4.8
> envelope + universal `k(h+w)≤2²²` bound), P-B (`2e91b21`,
> params-driven sizing + the go/no-go finding). M-S1 (`3feae98`,
> `noised_packed` store binding the *swept* strips).
> **Authoritative context:** `M_S2_PEARL_EVALUATION.md`
> (Pearl §4.6/§4.7/§4.8), `HIGH2_2_DESIGN.md` §4.C.4-G3 P-B.

---

## 0. The problem, precisely

P-B's `expected_layer0_rows` decomposition + the
`prod_matrix_hash_is_the_scale_blocker` test established: the
bridge's `prove_and_verify_tiled` calls `place_matrix_hash_a`
/ `place_matrix_hash_b` over the **entire** `ctx.a` / `ctx.b`
(`m·k` / `k·n` bytes). That re-derives, *in-circuit*, the full
BLAKE3 keyed chunk-Merkle (`crates/ai-pow/src/commit.rs::
matrix_commitment` = `blake3::Hasher::new_keyed(κ).update(
pad_to_chunk_boundary(M)).finalize()`):

```
mhash rows ≈ 136 · ⌈|M|/1024⌉      (16 keyed compressions ×8 rows
                                    per 1024-B chunk + parent layer)
PROD (4096²):  mhash_a + mhash_b ≈ 4.46M rows  >  2²² (=4.19M)
GEMMA / QWEN:  ≈ 14–18M rows
```

i.e. the matrix-hash **alone** exceeds Pearl's one-STARK bound,
while the §6(b) matmul sweep (the PoUW truth P-A bounds) fits
comfortably (PROD ≈ 2²¹). **Pearl never does this.** Pearl
whitepaper §4.6 ("Block Opening Proof"): the commitment
`H_A/H_B` is fixed; a block opening reveals only the **rows of
A / columns of B used by the opened tile**, their leaf indices,
and the **Merkle authentication paths** that recompute
`H_A/H_B`. P-B.2 is the faithful instantiation of §4.6 in
`ai-pow-zk`.

---

## 1. Goal & success criterion

Replace the in-circuit *full-matrix* hash with an in-circuit
**authenticated strip opening**:

- `HASH_A`/`HASH_B` remain the commitment — a **public input**
  the bridge already supplies (`ctx.h_a_chunk` / `ctx.h_b_chunk`,
  computed once off-circuit by `commit::matrix_commitment`,
  checked `pis.hash_a == bytes_to_words_le(ctx.h_a_chunk)`). The
  in-AIR binding `IS_HASH_A·(CV_OUT − PI_HASH_A) = 0`
  (`composite_full_air.rs:482`) is **unchanged** — only the row
  that *produces* that `CV_OUT` changes from "root of the full
  tree" to "root recomputed from the tile's leaf chunks + the
  authentication path".
- The circuit hashes **only the 1024-B BLAKE3 chunks covering
  the attested tile's `t` rows of A (row-major) and `t` columns
  of B (col-major)**, then folds them up the BLAKE3 chunk-Merkle
  using prover-supplied **sibling chaining values** for the
  authentication path, and asserts the recomputed root equals
  `PI_HASH_A` / `PI_HASH_B`.

**Success criterion (the go condition):** for every params set
in the Pearl §4.8 envelope (P-A), the *total* Layer-0 budget
(`Layer0RowBudget` with `mhash_*` replaced by the strip-opening
cost) is `≤ PEARL_TRACE_BOUND = 2²²` — i.e. the whole PROD proof
(matrix-opening + §6(b) sweep + fold + jackpot) fits **one**
STARK. Plus: **honest-equivalence** — for honest inputs the
strip-opened root is bit-identical to the full-matrix
`matrix_commitment` root (no commitment-format change; old
proofs' PI values are unchanged).

### 1.1 Cost (why this works)

PROD tile: `t = 128` rows of A, each `k = 4096` B ⇒ A-strips
`= t·k = 512 KiB = 512` chunks; B-strips identical.

```
leaf hashing:   512 chunks · 16 · 8            ≈ 65 536 rows  (per matrix)
auth path:      (≈511 in-subtree + ≈log₂(16384/512) boundary)
                ·8 rows                        ≈   4 200 rows  (per matrix)
P-B.2 mhash_a + mhash_b                        ≈ 140 000 rows
  vs. today's 4 460 000  →  ≈ 32× reduction
PROD total (opening + sweep ≈1.18M + fixed)    ≈ next_pow2 ≈ 2²¹  ≤ 2²² ✓
```

GEMMA/QWEN: B has `k·n` bytes but the *opened* B-strips are
only `t` columns × `k` ⇒ same `≈ t·k` per matrix, independent
of `m,n`. **The opening cost is `O(t·k)` — independent of the
matrix size** — which is the entire point of §4.6.

---

## 2. Invariants (must not violate)

1. **Commitment format unchanged.** `HASH_A`/`HASH_B` are still
   `commit::matrix_commitment` (BLAKE3 keyed chunk-Merkle of the
   padded row-major A / col-major B). The PI *values* are
   identical to today; only the in-circuit *derivation* changes.
   Honest-equivalence is a hard gate.
2. **Binding mechanism unchanged.** Keep
   `IS_HASH_A·(CV_OUT−PI_HASH_A)=0` /
   `IS_HASH_B·(CV_OUT−PI_HASH_B)=0`. The soundness model — "the
   verifier supplies the committed root as a PI; the circuit
   proves a path to it" — is the *same* as today's "circuit
   recomputes the root, binds to PI"; the prover just can't
   supply leaves/siblings that hash to `PI` unless they are the
   real ones (collision-resistance of BLAKE3).
3. **Tree-shape fidelity.** The in-circuit path MUST fold to the
   *exact* root that `blake3::Hasher` (the off-circuit
   commitment) produces — i.e. it must follow BLAKE3's real
   internal chunk-tree shape. (See §4 / DECISION D1 — this is
   the central correctness risk.)
4. **Verifier-fixed opening location.** *Which* chunks and
   siblings are opened is a pure function of
   `(params, tile_i, tile_j)` and MUST be verifier-fixed
   (CRIT-1 class, the §6(a)/MED-3 discipline) — a prover cannot
   choose to open a different region.
5. **Soundness changes live in the Pinned AIR + bridge only.**
   Unit `CompositeFullAir` and the ~300 constraint tests stay
   green; the C3/M52 derivation tests are updated, not broken.
6. **debug-assertions-OFF hazard** + **LogUp coupling**
   (M-S1 lessons): the C3 chain touches `IS_MSG_MAT`/`i8u8`/
   `urange8`/`noised_packed` buses — a leaf-hashing change is
   bus-coupled; gate every step with
   `RUSTFLAGS="-Ctarget-cpu=native -C debug-assertions=on"` and
   a full Route-A run, not just the unit AIR.

---

## 3. The current binding (what we are replacing)

`crates/ai-pow-zk/src/composite_trace.rs::place_matrix_hash`
(called via `place_matrix_hash_a`(sel idx 4) /
`place_matrix_hash_b`(idx 5)):

1. `padded = pad_to_chunk_boundary(matrix_bytes)`;
   `num_chunks = padded.len()/1024`.
2. **Chunk layer:** for each chunk `c`, 16 keyed BLAKE3
   compressions (`place_blake3_hash_with_selectors`, 8 rows
   each), `F_CHUNK_START` on block 0, `F_CHUNK_END` on block 15,
   counter tweak `= c`; produces `chunk_cv[c]`.
3. **Parent layer:** `while chunk_cvs.len() > 1 { pair (left,
   right) → keyed `F_PARENT` compression (8 rows); promote an
   unpaired trailing CV }`. The final survivor is the root;
   `F_ROOT` + the `IS_HASH_A`/`IS_HASH_B` selector are set on
   the **root** compression's finalize row → that row's
   `CV_OUT` is bound to `PI_HASH_A`/`PI_HASH_B`.

The §6(b) sweep separately consumes the **noised** strips
(`mats.a_prime_row` / `b_prime_col`), bound to a declared store
by **M-S1**; the tie noised-store ↔ plain-committed-A is
**§4.C.2** (noise derivation). P-B.2 is **orthogonal to
§4.C.2**: it changes how *plain* A/B is committed (full hash →
strip+path) so it *scales*; it does not itself bind `a_prime`
to `A`. (Composition: P-B.2 ⟹ plain strips ∈ `HASH_A`; §4.C.2 ⟹
store = noise(plain strips); M-S1 ⟹ sweep ∈ store. All three
needed for "sweep is on the committed matrix"; P-B.2 makes the
first one scale.)

---

## 4. BLAKE3 chunk-tree shape — D1 (RESOLVED by P-B.2.0)

> **⚠️ RESOLVED 2026-05-17.** This section originally framed the
> tree shape as "the central correctness risk" and hypothesised
> a *latent pre-existing fidelity gap* in `place_matrix_hash`
> for non-power-of-two chunk counts. **P-B.2.0's exhaustive KAT
> disproved that hypothesis.** `place_matrix_hash`'s bottom-up
> pair-adjacent/promote-odd reduction **is** BLAKE3's
> largest-pow2-left tree — they coincide for **all** chunk
> counts, not just powers of two (a structural identity:
> bottom-up left-complete pairing ≡ top-down maximal-perfect-
> left split). Verified in-circuit for `1..=31` and `100`
> chunks, and walker-vs-`blake3::Hasher` through `1000`, all
> bit-identical (`blake3_tree.rs` tests
> `place_matrix_hash_equals_true_tree_and_blake3_all_counts`,
> `…_large_nonpow2`, `merkle_root_matches_blake3_keyed_all_chunk_counts`).
> ⇒ **There is NO gap; `place_matrix_hash` needs no realignment;
> D1-A's realign step is a no-op and P-B.2.1 = P-B.2.0's
> equivalence verification (already done).** The analysis below
> is retained for the historical rationale; "risk" → "resolved
> property".

`commit::matrix_commitment` uses **real `blake3::Hasher`**,
whose internal tree is **not** a naïve left-leaning pairwise
reduction for non-power-of-two chunk counts: BLAKE3 splits at
the **largest power of two number of chunks < total** (left
subtree), recursively. `place_matrix_hash`'s parent layer is a
*simple pairwise-with-trailing-promotion* `while` loop. These
**coincide only when `num_chunks` is a power of two**
(and for ≤ 1 chunk).

Implications:

- For the **honest full-matrix** path, the existing
  `place_matrix_hash` has presumably been tested only for
  power-of-two chunk counts (M52 KATs). Under Pearl §4.8,
  `64 | k`; PROD `k = 4096` ⇒ each A row = `4096 B = 4` chunks,
  `num_chunks_A = (m/?)…` — *not necessarily* a power of two
  (e.g. `m·k/1024` for `m=4096,k=4096` = 16384 = 2¹⁴ ✓, but
  GEMMA `m·k/1024 = 4096·5376/1024 = 21504` — **not** a power of
  two). So `place_matrix_hash` may *already* disagree with
  `blake3::Hasher` for GEMMA/QWEN-shaped inputs — a **latent
  pre-existing fidelity gap**, independent of P-B.2.
- P-B.2's authentication path MUST fold to the **same root the
  off-circuit commitment uses**. So P-B.2 forces the question:
  which tree is canonical?

**DECISION D1 (surfaced).**
- **D1-A — true BLAKE3 tree.** Implement the authentication
  path against BLAKE3's *real* chunk-tree (largest-pow2-left
  recursion), and **realign `place_matrix_hash`** (the honest
  full-matrix path, still used for sub-envelope tests) to the
  same true tree. Removes the latent fidelity gap;
  honest-equivalence is exact for *all* chunk counts. More work
  (a correct BLAKE3 tree walker + retire the pairwise loop),
  but it is the only way the in-circuit root provably equals
  `commit::matrix_commitment` in general.
- **D1-B — pin to a power-of-two chunk count.** Extend Pearl
  §4.8 (P-A `validate_prod_envelope`) with `⌈m·k/1024⌉` and
  `⌈k·n/1024⌉` **powers of two** (achievable by zero-chunk
  padding the commitment to a pow2 leaf count — change
  `commit::matrix_commitment` + Pearl-side to pad to pow2
  chunks). Then `place_matrix_hash`'s pairwise loop *is* the
  BLAKE3 tree, and the auth path is a clean balanced-tree path.
  Smaller circuit change, but **changes the commitment format**
  (breaks invariant 1 / Pearl byte-compat unless Pearl also
  pads) — likely unacceptable.
- **D1-C — leaf-CV commitment (depart from BLAKE3-internal).**
  Define `HASH_A` as a *separate* balanced Merkle over per-chunk
  CVs with our own domain separation (like the existing
  `commit.rs` `CTX_*` 32-B-leaf tree), decoupled from
  `blake3::Hasher`'s internal tree. Clean path math, but it is a
  **new commitment** (not Pearl's `H_A`), losing
  Pearl-faithfulness for the commitment itself.

**Recommendation: D1-A.** It is the only option that keeps the
Pearl/Pearl-byte-faithful commitment *and* is correct for
arbitrary (GEMMA/QWEN) shapes, and it fixes a latent
pre-existing bug. The extra work is a bounded, well-specified
tree walker.

---

## 5. Design (assuming D1-A)

### 5.1 Opened-strip set (DECISION D2 — leaf granularity)

Row-major A: tile `(i,·)` uses rows `[i·t, i·t+t)`, each `k`
bytes ⇒ a **contiguous** byte span `[i·t·k, (i·t+t)·k)` of the
padded A. Col-major B: tile `(·,j)` uses the contiguous span
`[j·t·k, (j·t+t)·k)` of the padded col-major B. Contiguity is
the key property — a contiguous byte span ⇒ a contiguous
**chunk range** `[c0, c1)` ⇒ a small set of maximal BLAKE3
subtrees + `O(log)` boundary siblings.

- **D2-A (recommended): whole-chunk opening.** Open every
  1024-B chunk that overlaps the tile's span (`c0 =
  ⌊i·t·k/1024⌋ … c1 = ⌈(i·t+t)·k/1024⌉`). If `k` is not a
  multiple of 1024, boundary chunks also contain bytes of
  *adjacent* tiles' rows — irrelevant to the SNARK (the circuit
  hashes whole chunks; the extra bytes are witness, not
  revealed — privacy is the §4.7 zkSNARK's job, already in
  scope). Count `≈ ⌈t·k/1024⌉` (+≤1 boundary).
- **D2-B: require chunk-aligned rows.** Extend §4.8 with
  `1024 | k` so each row = whole chunks, `c0/c1` exact, no
  boundary sharing. Tighter but narrows the envelope (PROD
  `k=4096` ✓, but GEMMA `k=5376` is `1024·5.25` ✗).

**Recommendation: D2-A** (general; `O(t·k)` regardless of
alignment; the §4.8 envelope already guarantees `t·k/1024` is
modest).

### 5.2 In-circuit construction (`place_matrix_strip_opening`)

A new `CompositeTrace::place_matrix_strip_opening(row_start,
strip_bytes, kappa, leaf_range, auth_path, selector_idx)`:

1. **Leaf layer (binds the revealed bytes).** For each chunk
   `c ∈ [c0, c1)`: 16 keyed BLAKE3 compressions exactly as
   `place_matrix_hash`'s chunk layer (same `F_CHUNK_*`, same
   counter tweak `= c`) over the *witness* `strip_bytes` ⇒
   `leaf_cv[c]`. This is what binds the actual strip bytes the
   §6(b)/M-S1 path consumes to the commitment. Rows
   `≈ (c1−c0)·16·8`.
2. **Authentication fold (D1-A true BLAKE3 tree).** Walk the
   BLAKE3 chunk-tree (largest-pow2-left recursion) over
   `num_chunks = ⌈|M_padded|/1024⌉`. For each internal node on
   the path from `[c0,c1)` to the root: if both children are
   inside the opened range, compute the `F_PARENT` keyed
   compression of the two recomputed CVs (8 rows); if one child
   subtree is *outside* the opened range, take that child's CV
   from the prover-supplied **`auth_path`** witness and combine.
   `F_ROOT` + `IS_HASH_A`/`IS_HASH_B` on the final (root)
   compression's finalize row ⇒ the **existing**
   `IS_HASH_A·(CV_OUT−PI)=0` binds the recomputed root to the
   committed PI. Rows `≈ (#path parents)·8`.
3. Returns `(next_row, recomputed_root)`.

`auth_path` = the ordered list of sibling CVs (`[u32;8]` each)
for the off-range subtrees, **plus** their (level, side)
position — all *witness*, but their layout (count, order,
which level/side) is **verifier-fixed** from
`(num_chunks, c0, c1)` (§5.3). The *values* are
prover-supplied; they are bound because a wrong sibling ⇒ wrong
root ≠ PI ⇒ reject (collision resistance).

### 5.3 Verifier-fixed opening schedule (DECISION D3)

`(c0, c1, num_chunks)` and the BLAKE3-tree authentication
structure (which levels need a sibling, on which side) are a
**pure deterministic function of `(params, tile_i, tile_j)`**
— all public / MED-3-derived. The opening *schedule* must be
CRIT-1-class verifier-fixed so a malicious prover cannot open a
*different* region (e.g. a cheaper or all-zero strip) and still
pass.

- **D3-A (recommended):** the schedule is recomputed by the
  verifier from public params (exactly as MED-3 recomputes
  `target`/`tile_ij` and §6(a) recomputes the fold schedule).
  The selector/`STARK_ROW_IDX` layout for the opening block is
  pinned via the existing CRIT-1 program-pin (the leaf/parent
  rows carry pinned `CONTROL_PREP` like every other row); the
  prover only supplies *values* (strip bytes, sibling CVs)
  bound by the root==PI check. **No new pinned column** — the
  opening occupies program rows whose CONTROL_PREP is
  determined by the schedule, same discipline as the
  matrix-hash today.
- **D3-B:** add a dedicated pinned `OPEN_SCHED` column. Only if
  D3-A's CONTROL_PREP encoding proves insufficient (it should
  not — the matrix-hash today is already a pinned-program
  block; P-B.2 just makes it shorter and tile-indexed).

**Recommendation: D3-A** (reuses the proven CRIT-1/MED-3
discipline; zero new preprocessed width).

### 5.4 Soundness argument

A malicious prover wants the §6(b) fold of a *cheaper* matmul
to verify. With P-B.2:

- The §6(b) sweep inputs are M-S1-bound to a declared
  `noised_packed` store.
- §4.C.2 (separate residual) ties that store to
  `noise(plain strips)`.
- **P-B.2 ties the plain strips to `HASH_A`/`HASH_B`:** the
  leaf layer hashes the witness strip bytes; the auth fold
  forces them to be the chunks at the verifier-fixed
  `[c0,c1)` of the BLAKE3-tree whose root is the committed PI.
  Forging requires a BLAKE3 collision.
- `(tile_i, tile_j)` is the real solved tile (§4.E / MED-3);
  the schedule is verifier-fixed (§5.3) so the opened region is
  exactly that tile's rows/cols.

⇒ P-B.2 closes the *scalability* of the plain-side commitment
**without weakening** the C3 soundness model (same root==PI
binding, same BLAKE3 collision resistance). It does **not**
close §4.C.2 (still the documented final tie).

### 5.5 New attack surface (and how it's closed)

| Attack | Closed by |
|---|---|
| Open a cheaper/zero region instead of the tile's rows | §5.3 verifier-fixed `(c0,c1)` from `(params,tile_i,tile_j)` (CRIT-1) |
| Supply forged sibling CVs to hit `PI` | Root recomputed in-circuit; `≠ PI` ⇒ reject (BLAKE3 CR) |
| Supply strip bytes ≠ the ones the sweep uses | M-S1 `noised_packed` store binds the *swept* strips; §4.C.2 ties store↔plain — P-B.2 binds *plain*↔commitment; the chain composes (the residual is §4.C.2, unchanged) |
| Wrong tile index | §4.E / MED-3 (unchanged) |
| Tree-shape mismatch ⇒ a "valid" path to a different root | D1-A: in-circuit tree == `blake3::Hasher` tree, proven by honest-equivalence KAT |

---

## 6. Test plan (exhaustive, per the goal)

All under the fast loop + a `-C debug-assertions=on` pass +
full Route-A (`composite_*_pinned_logup`, the LogUp/
debug-assertions hazard surface).

1. **Honest-equivalence KAT (the gate).** For a sweep of shapes
   incl. **non-power-of-two chunk counts** (GEMMA/QWEN-like):
   `place_matrix_strip_opening`'s recomputed root **==**
   `commit::matrix_commitment(full matrix)` **==**
   `blake3::Hasher::new_keyed` — bit-identical. Also assert the
   *realigned* `place_matrix_hash` (D1-A) matches
   `blake3::Hasher` for the same non-pow2 shapes (closes the
   latent gap).
2. **Cost regression.** `expected_layer0_rows` updated:
   `mhash_*` term becomes `O(t·k)`; assert PROD/GEMMA/QWEN now
   `fits_one_stark()` (the P-B `prod_matrix_hash_is_the_scale_
   blocker` test flips to **fits**, becomes
   `prod_strip_opening_fits_one_stark`).
3. **Route-A end-to-end.** `high2_2_*` + `crit1_*` + the bridge
   MED-3 roundtrip with the strip-opening replacing
   `place_matrix_hash_*`; `pis.hash_a == ctx.h_a_chunk`
   unchanged (commitment format invariant).
4. **Adversarial (must reject):** (a) tampered strip byte ⇒
   root ≠ PI; (b) forged sibling CV ⇒ reject; (c) opened range
   shifted off the attested tile ⇒ pinned-schedule mismatch
   (CRIT-1 reject); (d) zeroed opening selectors (crit1_*
   discipline).
5. **Sub-envelope non-regression:** TEST_SMALL etc. — the
   bridge swap keeps `ai-pow --features zk` green
   (commitment/PI unchanged ⇒ bit-identical PIs; trace shrinks).

---

## 7. Risks

- **D1-A tree walker fidelity** is the dominant risk: an
  off-by-one in BLAKE3's largest-pow2-left recursion ⇒ wrong
  root for some shape ⇒ honest-equivalence fails. Mitigation:
  KAT against `blake3::Hasher` over many shapes incl.
  non-pow2/odd chunk counts *first*, before wiring the AIR.
- **C3 bus coupling** (M-S1 lesson): the leaf layer drives
  `IS_MSG_MAT`/`i8u8`/`urange8`/`noised_packed`; a shorter leaf
  set changes those buses' populated rows — unit-AIR green ≠
  Route-A green. Must validate via Route-A + debug-assertions-ON
  at each step.
- **Pre-existing latent gap exposure.** D1-A *fixes*
  `place_matrix_hash` for non-pow2 chunk counts; if any shipped
  test depended on the (incorrect) pairwise root for a non-pow2
  shape it will change — audit M52 KATs (they should be pow2,
  unaffected; if not, the KAT was wrong and is corrected).
- **Invasiveness:** touches the C3/HASH_A chain, the M52
  chunk-Merkle, the bridge, and the cost estimator — a
  multi-step landing, not a one-shot.

---

## 8. Staged landing plan

Unlike M-S1, P-B.2 is **stageable** (honest-equivalence is a
pure off-circuit property checkable before any AIR change):

- **P-B.2.0 — ✅ DONE 2026-05-17** (`blake3_tree.rs`). Pure
  off-circuit module: `left_len`, `chunk_cv`/`parent_cv`
  (replicate `place_matrix_hash`'s primitive via the KAT'd
  `blake3_compress`), `merkle_root` (== `blake3::Hasher`),
  `open_strip`/`verify_strip_opening` (the §4.6 primitive
  P-B.2.2 mirrors). KAT bit-identical to `blake3::Hasher` for
  chunk counts `{1..17, 31, 32, 33, 100, 1000}`; strip-opening
  recomputes the root for every contiguous range; adversarial
  tamper rejects. *No circuit change.* **Plus: subsumes
  P-B.2.1** — proved in-circuit `place_matrix_hash` already ==
  the true tree == `blake3::Hasher` for all counts `1..=31`,
  `100`.
- **P-B.2.1 — ✅ SUBSUMED by P-B.2.0 (no-op).** The latent-gap
  hypothesis was disproven (§4): `place_matrix_hash`'s pairwise
  loop *is* BLAKE3's tree for all chunk counts, so no realign
  is needed. P-B.2.0's
  `place_matrix_hash_equals_true_tree_and_blake3_all_counts` /
  `…_large_nonpow2` *are* the equivalence gate. Net: the
  honest full-matrix root was already exact for all shapes.
- **P-B.2.2 — `place_matrix_strip_opening`.** Leaf layer over
  the opened chunk range + auth fold using the walker +
  witness siblings; `IS_HASH_A/B` on the recomputed-root row.
  Unit + Route-A + adversarial.
- **P-B.2.3 — verifier-fixed schedule (D3-A)** from
  `(params,tile_i,tile_j)`; extend `extract_program`/the
  pin-rebuild; crit1_* still reject forged schedules.
- **P-B.2.4 — bridge swap.** `prove_and_verify_tiled` calls
  `place_matrix_strip_opening` (tile strips + off-circuit
  `commit::matrix_commitment` for the PI) instead of
  `place_matrix_hash_a/b`; `expected_layer0_rows` updated;
  `fits_one_stark()` flips true for PROD. Full
  `ai-pow --features zk` green + the cost-regression test.

Each stage: commit with the `Co-Authored-By: Claude Opus 4.7
(1M context) <noreply@anthropic.com>` trailer; push only when
asked; `Plonky3-recursion/` stays untracked.

After P-B.2 the only remaining §4.C tie is **§4.C.2**
(store ↔ plain-strip via in-circuit noise derivation) — then
PROD matmul-truth is genuinely one-tile-one-STARK and
Pearl-§4.6-faithful end to end (modulo §4.C.2 + the M-S5/P-C
vertical certificate).

---

## 9. Decisions for the maintainer

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **D1** | BLAKE3 chunk-tree fidelity | A: implement BLAKE3's true tree + realign `place_matrix_hash` · B: pad commitment to pow2 chunks (changes commitment) · C: separate CV-Merkle (new, non-Pearl commitment) | **A** — only option keeping the Pearl/byte-faithful commitment correct for all shapes; fixes a latent gap |
| **D2** | Leaf granularity | A: whole 1024-B chunks covering the tile span (general, `O(t·k)`) · B: require `1024\|k` (tighter, narrows envelope) | **A** |
| **D3** | Opening-schedule pin | A: verifier-recompute from public `(params,tile_i,tile_j)` via the existing CRIT-1/CONTROL_PREP discipline (no new column) · B: dedicated pinned `OPEN_SCHED` column | **A** |
| **D4** | Landing | Staged P-B.2.0→.4 with the off-circuit honest-equivalence KAT as the pre-AIR gate · vs one-shot | **Staged** (KAT first; each stage Route-A+debug-assertions-ON) |

**ALL LOCKED 2026-05-17 (maintainer-approved): D1=A · D2=A ·
D3=A · D4=A.** No option branches remain open; the body assumes
this path. D1=A additionally fixes a **pre-existing latent
fidelity gap** in `place_matrix_hash` for non-power-of-two
chunk counts (independently worth doing). Implementation
proceeds per §8 (staged P-B.2.0→.4, KAT-first).

---

## 10. Cross-references

- Pearl §4.6 (Block Opening Proof), §4.3 (A row-major / B
  col-major → small openings), §4.7 (zkSNARK hides revealed
  strips): `M_S2_PEARL_EVALUATION.md` §1.
- Current binding: `composite_trace.rs::place_matrix_hash`
  (chunk+parent layers), `composite_full_air.rs:482`
  (`IS_HASH_A·(CV_OUT−PI)=0`), `composite_public.rs`
  (`PI_HASH_A/B`), `commit.rs::matrix_commitment` /
  `pad_to_chunk_boundary`.
- P-B finding & cost model: `HIGH2_2_DESIGN.md` §4.C.4-G3 P-B;
  `zk_bridge.rs::expected_layer0_rows` / `Layer0RowBudget`.
- Composition residual: §4.C.2 (store ↔ plain via noise
  derivation) — `ai_pow_zk_crypto_gaps` memory; **independent
  of P-B.2**.
- M-S1 (swept-strip↔store binding, done): commit `3feae98`.
