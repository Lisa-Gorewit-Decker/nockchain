# §4.C.2 / Phase-A3 — store ↔ committed-plain-strip noise-derivation binding (design)

> **✅ §4.C.2 RESOLVED 2026-05-18 — ZERO-GAP on the
> production-faithful 16|r path (c-exact; A3.3 flipped).** The
> noise tie (A3.0–A3.2b) + the plain tie (cx.1 generalized C3 +
> CRIT-1 word-pair pin; cx.2 the X1 g=1 co-location flip — the
> strip-opening leaf round-0 rows are the M-S1 `noised_packed`
> producers, the whole-block C3 binds their committed
> `UINT8_DATA[0..64]` to `BLAKE3_MSG` ∈ `HASH_A`) are both
> closed. End-to-end + **position-exact adversarially**
> validated on a real 16|r `P16` bridge trace (honest roundtrip
> proves+pow-verifies at real difficulty with C3 ACTIVE; a
> tampered co-located committed-plain byte is rejected).
> `ai-pow-zk --lib` 352/0/22, `ai-pow --features zk` 89/0/1,
> debug-assertions-ON P16 g=1 per-row clean. Pearl §4.8 is
> always 16|r ⇒ production is zero-gap; non-16|r *test* geometry
> remains the A3.2b separate-store path (strictly stronger than
> pre-A3, *not* a forgery hole). ~11 validated commits
> (cx.0–cx.2.1, the cx.2 zero-blast foundation
> layout/c3/pcols/mat-input/matfreq/bus, the g=1 flip, the
> position-exact adversarial). Full record: §8 (§8.1–§8.10).
> The original design narrative below is the historical
> rationale.
>
> **PROGRESS 2026-05-17:** A3.0/A3.1/A3.2a/**A3.2b all DONE &
> validated** — the §4.C.2 **noise tie is closed** (store
> `NOISE_UNPACK` forced to `noise_ref` of the C1-public seed via
> InputChip + the CRIT-1 `NOISE_PACKED_PREP` pin; `ai-pow-zk
> --lib` 351/0/22, `ai-pow --features zk` all-binaries 0-failed,
> MED-3 roundtrip green through the split store). **DECISION
> 2026-05-17 (maintainer): the plain tie (B1) is done via
> c-exact — the c-mset `BUS_PLAIN` bus is ABANDONED.** History:
> c-mset shipped as the de-risk arc — **c-mset.0 ✅ + c-mset.1a
> ✅** (`2c2d7c6`): the KAT-first de-risk at the *exact
> `BUS_PLAIN` AIR key* proved `consumer ⊆ producer` holds **iff
> `16|r`** (Pearl is always `16|r`; `TEST_SMALL` r=4 proven
> negative) **and** that the bus needs invasive
> CRIT-1-pinned-program producer-row gating regardless. Given
> that, the maintainer chose **c-exact** (position-exact,
> zero-gap; reuses the *proven* C3 — no new bus/FREQ/permutation,
> no open producer-isolation decision; strictly dominates — see
> **§8** for the full comparison + c-exact design + the staged
> cx.0–cx.3 plan). c-mset.0/.1a are **retained** as the de-risk
> that justified the decision and establishes the
> contiguity/`16|r`-alignment facts c-exact reuses (the
> P-B.2.0/D1 pattern). **cx.0 ✅ DONE & validated** (`2bbf4cd`):
> KAT proved every position-addressed store row binds, via the
> exact C3 identity at a witness-free `(chunk,block,word_off)`
> leaf address, to the exact committed bytes ∈ `HASH_A` (r=16 +
> r=32). **Next = cx.1** (first invasive AIR stage — generalize
> the proven C3 to a CRIT-1-pinned word-offset; §6(b)/G2-scale,
> per-sub-stage validated cx.1a→cx.1c; §8.3). Not to be rushed
> (R1). Soundness meanwhile: CRIT-1 + §4.D + §6 +
> M-S1 + A2 + the A3.2b noise pin hold; §4.C.2 with A3.2b is
> already *strictly stronger than pre-A3* (store noise =
> public-seed Pearl noise, not prover-chosen) and **not a
> forgery hole**.
>
> **Status:** DESIGN — **CORRECTED 2026-05-17 (A3.0 finding).**
> The original §3 proposed a heavy in-circuit BLAKE3-keyed
> noise **sub-AIR ("B2")**. **That is Pearl-unfaithful and
> unnecessary.** Pearl whitepaper §4.7 explicitly does NOT
> re-derive the noise in-circuit — *"the zk-prover and verifier
> agree on the plaintext noise … extend the AIR with
> preprocessed columns."* ai-pow-zk **already has exactly this**:
> `NOISE_PACKED_PREP` is one of the 5 CRIT-1 PROGRAM_COLS
> (verifier-pinned preprocessed), and `chips/input.rs` already
> enforces `NOISE_PACKED_PREP == polyval(NOISE_UNPACK,129)` **and**
> `NOISED_PACKED == polyval(MAT_UNPACK,256)+polyval(NOISE_UNPACK,256)`
> *unconditionally on every row*. ⇒ §4.C.2 reuses CRIT-1 +
> InputChip + C3 + the (A3.0-proven) `noise_ref` — **no new
> sub-AIR**. The corrected design is §3′ below; the original §3
> is struck through (kept for rationale). This is the same
> KAT-first re-grounding win as P-B.2.0's D1.
> The last open §4.C soundness tie and Phase-A3 of
> `PRODUCTION_ROADMAP.md`. Staged, KAT-first (P-B.2.0
> discipline).
> **Predecessors (done):** M-S1 (sweep ↔ declared `noised_packed`
> store), A1/P-B.2.3 (verifier-fixed opening schedule), A2/P-B.2.4
> (strip-opening binds the *plain* tile strips to
> `HASH_A`/`HASH_B`).
> **Cross-refs:** `P_B2_STRIP_OPENING_DESIGN.md`,
> `PRODUCTION_ROADMAP.md` Phase A, `ai_pow_zk_crypto_gaps`
> memory, `crates/ai-pow/src/{matmul.rs,prng.rs}` (the
> noise reference this must mirror byte-for-byte).

---

## 0. The gap (precisely)

After A2 the §4.C chain is:

```
committed A/B  ──A2 strip-opening──▶  plain tile strips ∈ HASH_A
                                              │   (§4.C.2 — THIS)
                                              ▼
swept a′/b′  ──M-S1 LogUp multiset──▶  declared noised_packed store
```

M-S1 binds *sweep ⊆ store*; A2 binds *plain strips ∈ committed
`HASH_A`*. **Nothing yet forces `store == noise(plain
strips)`.** A malicious prover may declare *any* noised store
(M-S1 only ties the sweep to whatever store it declares) — so
the swept matmul need not be over the committed matrix. §4.C.2
closes this: every store entry must be
`A_committed + E` with `E` the Pearl low-rank noise derived from
the C1-pinned seed `s_a` (resp. `B + F` from `s_b`).

Soundness today without §4.C.2: held by CRIT-1 + §4.D keystone +
§6(a) + §6(b) + M-S1 (not a *forgery* hole — the swept work is
pinned to *a* declared store and the fold/digest chain is
forced); §4.C.2 upgrades "fold of *a declared* matmul" → "fold
of *the committed block's* matmul" — full §4.C soundness, zero
gap.

## 1. The exact relation to enforce (verified against the code)

`crates/ai-pow/src/matmul.rs` + `prng.rs` (the byte-equivalent
Pearl reference):

```
a_prime[i,l] = A[i,l] + E[i,l]
E[i,l]       = E_L[i, pp_l] − E_L[i, pm_l]
  E_L[i,·]   = expand_e_l_row(s_a, i, r)       // r × uniform 6-bit, BLAKE3(s_a)-keyed
  (pp_l,pm_l)= e_r_col_positions(s_a, l, r)    // per-COLUMN pair, BLAKE3(s_a)-keyed,
                                               //   distinct, ∈ [0,r)
b_prime[l,j] = B[l,j] + F[l,j]                 // B col-major
F[l,j]       = F_R[j, pp_l] − F_R[j, pm_l]
  F_R[j,·]   = expand_f_r_col(s_b, j, r)
  (pp_l,pm_l)= f_l_row_positions(s_b, l, r)    // BLAKE3(s_b)-keyed
```

Key simplification (why this is tractable, not a generic
`r`-wide matmul): `E_R` is a **signed 2-sparse selection**
(exactly one `+1` and one `−1` per column) ⇒ `E[i,l]` is just a
**difference of two `E_L` entries** of row `i`. The per-element
noise needs: (a) the `r`-wide `E_L` row for the strip's matrix
row, (b) the two column-positions `(pp_l,pm_l)`, (c) a
select-and-subtract. Both (a) and (b) are BLAKE3-keyed PRNG
expansions of the **C1-pinned** `s_a`/`s_b`.

## 2. The lever already in place

`chips/input.rs` **already** enforces, per row:

```
NOISED_PACKED[c] == polyval(MAT_UNPACK[4c..],256) + polyval(NOISE_UNPACK[4c..],256)
```

So if, on the M-S1 store rows, we set
**`MAT_UNPACK = the committed plain strip bytes`** and
**`NOISE_UNPACK = E` (the Pearl noise bytes)**, then
`NOISED_PACKED == plain + E == a_prime` *for free* (existing
InputChip constraint). §4.C.2 then needs only **two** new
bindings:

- **B1 — `MAT_UNPACK` ↔ committed plain strip.** The store
  row's `MAT_UNPACK` must equal the bytes the A2 strip-opening
  leaf layer hashed (which are ∈ `HASH_A`). This is the
  **existing C3 pattern** (`IS_MSG_MAT·(BLAKE3_MSG[j] −
  base256(UINT8_DATA))=0`): make the strip-opening *leaf* rows
  carry the plain bytes as `BLAKE3_MSG` *and* be the store
  rows (`IS_MSG_MAT=1`, `MAT_UNPACK=plain`). C3 already binds
  hashed-bytes ↔ `UINT8_DATA`/`MAT_UNPACK`; reuse it. (M52's C3
  is exactly this mechanism on the full-matrix path; A2 made
  the leaf layer the *strip* path — B1 is wiring the store onto
  those leaf rows.)
- **B2 — `NOISE_UNPACK` ↔ `E` derived from `s_a`.** The new
  **noise-derivation sub-AIR**: from the C1-pinned `s_a`,
  expand `E_L[i,·]` and `(pp_l,pm_l)`, compute
  `NOISE_UNPACK[·] = E_L[i,pp_l] − E_L[i,pm_l]`. This is the
  hard, genuinely new part.

## 3′. CORRECTED design — Pearl §4.7 preprocessed noise (no sub-AIR)

The InputChip (`chips/input.rs::eval`, every row, unconditional):

```
(1) NOISE_PACKED_PREP == polyval(NOISE_UNPACK[0..8], base=129)
(2) NOISED_PACKED[i]  == polyval(MAT_UNPACK[4i..],256)
                       + polyval(NOISE_UNPACK[4i..],256)        i∈{0,1}
```

`NOISE_PACKED_PREP` ∈ the 5 CRIT-1 PROGRAM_COLS — the verifier
rebuilds it witness-free from the trusted shape
(`composite_preprocess::fill_preprocessed_row` ←
`RowDescriptor.noise_packed`). (1) ⇒ `NOISE_UNPACK` is *forced*
to the verifier-pinned noise (polyval-129 is injective over the
range-checked i7 bytes). So:

**§4.C.2 = three wirings, all on existing mechanisms:**

- **W1 (verifier-pinned noise).** The canonical-program
  reconstruction sets, for each M-S1 store row, `noise_packed =
  polyval(noise_chunk,129)` where `noise_chunk` is the Pearl
  noise for that chunk's matrix positions, computed by the
  verifier via **`noise_ref`** (A3.0, proven byte-equivalent to
  `BlockNoise`; a pure public fn of the C1-pinned `s_a`/`s_b` +
  params). InputChip (1) then forces the prover's
  `NOISE_UNPACK` to equal it.
- **W2 (store decomposition).** M-S1 store rows change from
  `MAT_UNPACK=a′, NOISE_UNPACK=0` to `MAT_UNPACK=committed
  plain, NOISE_UNPACK=pinned noise`. InputChip (2) then makes
  `NOISED_PACKED = plain + noise = a′` — **the same
  `NOISED_PACKED` value as today**, so M-S1's
  `enumerate_noised_chunks`/`MAT_FREQ` LogUp still balances
  (only the MAT/NOISE split changes, not the packed value).
- **W3 = B1 (plain ↔ HASH_A).** Bind `MAT_UNPACK` (now the
  plain bytes) to the A2 strip-opening leaf bytes via the
  **existing C3** `IS_MSG_MAT·(BLAKE3_MSG[j] −
  base256(UINT8_DATA))=0` (the M52 mechanism) — make the
  store rows the strip-opening leaf rows (`IS_MSG_MAT=1`,
  `MAT_UNPACK=plain`).

Net chain (all forced in-circuit, zero gap): committed A/B
─C3/strip-opening(A2)→ plain ─InputChip(2)→ `NOISED_PACKED =
plain + noise` ; noise ─InputChip(1)→ `NOISE_PACKED_PREP`
─CRIT-1 pin← verifier `noise_ref(s_a)` ─C1← `s_a` ; M-S1 ⇒
sweep ⊆ store(`NOISED_PACKED`). ⇒ the swept matmul is provably
over `noise(committed A,B)`.

> **⚠️ A3.2 CORE FINDING (2026-05-17) — the real reason §4.C.2
> is the multi-session hard residual.** W1 needs the verifier
> to pin `NOISE_PACKED_PREP` per store row **witness-free** (the
> CRIT-1 program is rebuilt from public data only). But M-S1's
> store is **value-deduplicated from the witness `a′`**
> (`enumerate_noised_chunks`) — so the store-row ↔ `(i,l)` map
> (hence each row's `noise_ref` noise *and* its committed plain
> bytes) is **NOT recomputable from public params alone**.
> A3.1 produced the *prover-side* map (`…_with_src`, KAT-proven)
> but the *verifier* cannot reproduce a value-deduped layout.
> ⇒ A3.2 must first **rework the M-S1 store into a
> position-addressed, params-deterministic schedule** (row `k`
> ↔ a fixed `(tile, lane, l)` derived purely from
> `params`/`tile_i,j` — like the A1 `tile_chunk_range`
> discipline — so the verifier recomputes the row layout, and
> `NOISE_PACKED_PREP` via `noise_ref`, with no witness; the
> committed plain side comes through B1/C3 from the
> strip-opening leaves). This is M-S1-magnitude soundness-
> linchpin work and warrants its own design pass + staged
> landing; it is **not** the lighter "just wiring" the bullets
> below implied. Soundness meanwhile: CRIT-1 + §4.D + §6 + M-S1
> + A2 hold (§4.C.2 is the documented residual, not a forgery
> hole).

**Staged landing (corrected, KAT-first):**
- **A3.0 ✅** `noise_ref` + cross-crate KAT (done, `4c6b3e8`).
- **A3.1 ✅** prover-side per-store-row `(plain,noise)` map +
  cross-crate KAT (`enumerate_noised_chunks_with_src`,
  `NoisedChunkSrc`; every store chunk == committed_plain +
  `noise_ref` byte-for-byte; `79f748d`). No AIR change.
- **A3.2a (next) — position-addressed store schedule.** Replace
  M-S1's value-deduped `enumerate_noised_chunks` store layout
  with a params/tile-deterministic one so the verifier can
  recompute row→`(tile,lane,l)` (and thus `NOISE_PACKED_PREP`)
  witness-free. Must keep M-S1's LogUp balance
  (`NOISED_PACKED` multiset unchanged) and re-validate Route-A.
- **A3.2b ✅ DONE 2026-05-17 (b1).** `write_noised_row_split` +
  `place_noised_store_row_split`; bridge places the store via
  the A3.1 `enumerate_noised_chunks_with_src` decomposition
  (`MAT_UNPACK=ctx.a/b plain`, `NOISE_UNPACK=noise_ref(s_a/s_b)`,
  `NOISE_PACKED_PREP=polyval(noise,129)`). `a′=plain+noise` fits
  i8 no-wrap ⇒ `NOISED_PACKED` unchanged ⇒ M-S1 LogUp balanced.
  InputChip eqn1 + the CRIT-1 `NOISE_PACKED_PREP` pin ⇒ **the
  store noise is forced to Pearl `noise_ref` of the C1-public
  seed — the prover cannot choose it**. MED-3 bridge roundtrip
  green through the full split-store path; §4.C.2 decomposition
  KAT green. **The §4.C.2 *noise* tie is closed.**
- **A3.2c = W3/B1 — the remaining §4.C.2 piece (the *plain*
  tie); not yet implemented.** Two forms, different
  soundness/cost (FINDING 2026-05-17):
  - **(c-exact) — co-location (position-exact, zero-gap).** Make
    the store rows *be* the strip-opening leaf compression rows
    so the existing C3 (`IS_MSG_MAT·IS_NEW_BLAKE·(BLAKE3_MSG[j]−
    base256(UINT8_DATA))`) binds `MAT_UNPACK` to the exact
    committed plain bytes that leaf hashed into `HASH_A`.
    Requires aligning the 8-i8 store window to a leaf
    compression's 64-byte `BLAKE3_MSG` across the
    1024-B-chunk ↔ 8-i8-window granularity gap — **the
    M-S1-magnitude core difficulty §4.C.2 has always been
    flagged for.** This is the zero-gap target.
  - **(c-mset) — LogUp multiset (tractable, structurally
    weaker).** A new bus: strip-opening leaves *publish* the
    committed plain byte-windows; store rows *query* their
    `MAT_UNPACK` ⊆ that multiset (mirrors M-S1's own pattern).
    Far more tractable (well-trodden in this codebase) but
    binds *membership*, not *position*: with the A3.2b
    position-pinned noise it yields "every swept `a′` = (some
    committed plain window) + (the position's Pearl noise)" —
    strictly stronger than today, but **not plain-side
    position-exact** (a documented residual vs "zero gap").
  Per R1, (c-exact) is the correct end state and is **not to
  be rushed**; (c-mset) is a legitimate validated *interim*
  with a precise residual if needed. The decision (c-exact now
  vs c-mset-interim→c-exact) is surfaced to the maintainer.
- **A3.2c (B1 plain tie) — c-mset interim, staged (maintainer
  hybrid decision; ≈M-S1-magnitude new LogUp bus ⇒ R1 staged,
  not rushed):**
  - **c-mset.0 ✅** off-circuit/KAT de-risk (the M-S1 coverage-net
    / P-B.2.0 discipline): against the real bridge geometry,
    every store `MAT_UNPACK` window's *real bytes* == committed
    plain at contiguous positions inside the hashed span
    (`5436f89`). Necessary but — as c-mset.1a showed — **not
    sufficient** for a balancing bus (it `continue`d past
    zero-pad).
  - **c-mset.1a ✅ DONE 2026-05-17 (`2c2d7c6`)** — KAT-first
    de-risk at the **exact `BUS_PLAIN` AIR key** (no AIR change;
    the P-B.2.0/c-mset.0 discipline carried one level deeper):
    - **Producer** = strip-opening leaf-chunk **round-0
      (`IS_NEW_BLAKE`)** rows' *unpermuted* `BLAKE3_MSG` (16
      u32-LE words = the 64 committed bytes of each hashed
      block; rows 1..7/finalize hold the *permuted* schedule —
      must NOT be read), split into the 8 disjoint word-pair
      windows `(BLAKE3_MSG[2j], BLAKE3_MSG[2j+1])`, j∈0..8,
      over the opened strip `[c0,c1)` only.
    - **Consumer** = store-row plain `MAT_UNPACK` window, packed
      identically (u32-LE of `UINT8_DATA` = `polyval(.,256)` per
      4 bytes).
    - **Validated FINDING:** `consumer ⊆ producer` (the exact
      LogUp balance premise) holds **iff `16 | r`** — then every
      store window is 8 *dense* contiguous committed bytes,
      8-aligned in the row/col-major matrix, == one producer
      word-pair. Pearl §4.8 pins `r ∈ {2⁵..2¹⁰}` (every value a
      multiple of 16) ⇒ **production is always clean**;
      `TEST_SMALL` (`r=4`) is **not** (zero-pad tail, no
      committed counterpart — proven by the test's negative
      assertion). POSITIVE validated on `r=16` (single-chunk)
      **and** `r=32` (multi-chunk) §6(b)-live single-STARK
      geometries.
  - **c-mset.1b (next; residual)** add `BUS_PLAIN` const +
    `PLAIN_FREQ` column + `push_interaction`: producer on the
    strip-opening leaf-chunk round-0 rows; consumer on store
    rows. **Emission `16|r`-gated** (params-derived, the
    existing PROD-gating discipline — Pearl is always `16|r`;
    `TEST_SMALL` r=4 stays inert so its Route-A tests are
    untouched). **OPEN MAINTAINER DECISION (soundness-AIR
    shape):** the producer must fire *only* on matrix-leaf-chunk
    round-0 rows, never on parent compressions (message =
    `left‖right` CVs, not committed bytes) or C2/C4/jackpot
    BLAKE3. No existing selector isolates this. Options, both
    invasive to the **CRIT-1-pinned program**: (i) a dedicated
    selector set by `place_leaf_chunk` on leaf round-0 rows
    (new `SELECTOR_COL` ⇒ width/preprocessed/CRIT-1-rebuild/all
    layout-assert/all Route-A-baseline ripple); (ii) decode the
    BLAKE3 tweak flags (`F_CHUNK_START & F_KEYED_HASH &
    !F_PARENT`) from the packed `CV_OR_TWEAK_PREP` in-AIR (no
    width change; adds tweak-decode constraint logic to the
    pinned program). Decision required before the AIR change.
  - **c-mset.2** `populate_lookup_freq` accounting for the new
    bus (producer `PLAIN_FREQ`); honest balance.
  - **c-mset.3** Route-A on a **`16|r` §6(b)-live single-STARK
    geometry** (`P16 = {m:16,k:64,n:16,noise_rank:16,tile:8}` —
    c-mset.1a-confirmed §6(b)-live & single-STARK; **NOT**
    `TEST_SMALL`) + debug-assertions-ON + adversarial (a store
    `MAT_UNPACK` ∉ the committed-plain multiset rejects); full
    `ai-pow-zk --lib` + `ai-pow --features zk`. (The
    LogUp-coupling / unit≠Route-A hazard — M-S1 lesson — gates
    every sub-step.)
  After c-mset: §4.C.2 = "every swept `a′` = (some committed
  plain window) + (the store row's position-pinned Pearl
  noise)" — strictly stronger than pre-A3, **not a forgery
  hole**; plain-side position-exactness is the **precise
  documented residual** (c-exact).
- **A3.2c (B1) — c-exact (zero-gap completion; the Phase-A3
  residual after c-mset).** Co-locate store rows onto the
  strip-opening leaf compression rows so the existing C3 binds
  `MAT_UNPACK` to the *exact* committed plain bytes ∈ `HASH_A`
  (the 1024-B-chunk ↔ 8-i8-window granularity bridge — the
  long-flagged §4.C.2 core difficulty). Its own staged effort
  (R1); closes the c-mset residual ⇒ true zero-gap §4.C.2.
- **A3.3** final gate: full `ai-pow-zk --lib` + `ai-pow
  --features zk` + debug-assertions-ON; §4.C end-to-end
  (committed → noise → store → M-S1 sweep → fold → digest);
  docs/`ZKP_SECURITY_REPORT`/`GAP_AUDIT` flip.

Far lighter than the struck-through §3 (no BLAKE3-keyed PRNG
sub-AIR, no select-subtract chip, no new LogUp): it is
*wiring + verifier-side noise reconstruction*, reusing CRIT-1 /
InputChip / C3. Still soundness-critical & invasive (CRIT-1
program reconstruction is the PoW-soundness linchpin) ⇒ staged
with Route-A + debug-assertions-ON at each step.

---

## 3. ~~B2 — the noise-derivation sub-AIR~~ (SUPERSEDED by §3′)

> Struck through — the in-circuit PRNG sub-AIR is **not**
> needed (Pearl §4.7 uses preprocessed noise; see §3′). Kept
> for the rationale of why the relation has the
> select-subtract form.

### ~~B2~~ (the milestone core)

Mirror, in-circuit, `prng.rs`'s `pearl_random_hash` /
`fill_uniform_row` / `pearl_permutation_pair`:

- **PRNG = BLAKE3-keyed.** `expand_e_l_row` and the position
  pair are BLAKE3 hashes keyed by `s_a` over a small message
  (chunk/slot index). ai-pow-zk **already has the in-circuit
  BLAKE3 chip** (`place_blake3_hash_*`) and C1 pins `s_a`
  (`COMMITMENT_HASH`). So E_L / positions are BLAKE3 outputs of
  `s_a` + a public counter — placeable as ordinary BLAKE3
  compression blocks whose key is the C1-pinned `s_a`.
- **`E_L` row:** `r` × 6-bit uniform = a deterministic slice of
  the BLAKE3 keystream (`fill_uniform_row`'s exact byte→i7 map).
- **Positions `(pp_l,pm_l)`:** `pearl_permutation_pair`'s
  `first = rnd & (r−1); second = first ^ (1 + mul_hi(r−1,rnd))`
  — a few field ops on a BLAKE3 word; `r` is a pinned power of
  two (§4.8/P-A) so `r−1` is a constant mask.
- **Select + subtract:** `E[i,l] = E_L[i,pp_l] − E_L[i,pm_l]`.
  `pp_l/pm_l ∈ [0,r)`; selection via a pinned/derived one-hot
  over the `r` `E_L` lanes (degree-2, the §6(b) one-hot
  pattern), then a subtraction. `r ≤ 1024` (§4.8) bounds the
  one-hot width.

This is bounded (no generic `r`-matmul), but it is a new
multi-row sub-AIR + LogUp/range plumbing + the C1-`s_a` key
binding ⇒ **milestone-scale**.

## 4. Staged landing (KAT-first — the P-B.2.0 discipline)

- **A3.0 — off-circuit noise reference + KAT (no circuit
  change).** A pure `noise_ref` module reproducing
  `E[i,l]`/`F[l,j]` from `s_a`/`s_b` and **bit-identical to
  `ai-pow::matmul::BlockNoise`** over many shapes (the
  cross-crate equivalence; ai-pow-zk must not dep ai-pow, so
  re-derive from the same BLAKE3 primitive and KAT the values
  passed across the bridge). De-risks the spec before any AIR.
- **A3.1 — `E_L`/position expansion in-circuit** from the
  C1-pinned `s_a` (BLAKE3 blocks + the byte→i7 map +
  permutation-pair ops); unit + KAT vs A3.0.
- **A3.2 — select-subtract + the InputChip wiring**: store rows
  get `MAT_UNPACK=plain`, `NOISE_UNPACK=E`; the existing
  InputChip closes `NOISED_PACKED=plain+E`. B1 (reuse C3) binds
  `MAT_UNPACK`↔strip-opening leaf bytes.
- **A3.3 — Route-A + adversarial**: a store whose `NOISE_UNPACK
  ≠ E(s_a)` (or `MAT_UNPACK ≠` committed strip) must reject;
  full `ai-pow-zk --lib` + `ai-pow --features zk`;
  debug-assertions-ON; the §4.C chain end-to-end (committed →
  noise → store → M-S1 sweep → fold → digest), zero gap.

Each stage: commit with the trailer; Route-A +
debug-assertions-ON; `Plonky3-recursion/` untracked.

## 5. Risks

- **Largest remaining soundness milestone** — invasive
  (touches the store rows, InputChip wiring, a new BLAKE3-keyed
  sub-AIR, C1-`s_a` binding, LogUp coupling — the M-S1 lesson:
  unit-AIR green ≠ Route-A green). Stage strictly; KAT-first.
- **Byte-equivalence to Pearl's PRNG** is the correctness
  anchor — A3.0's KAT vs `ai-pow::BlockNoise` (itself written
  Pearl-byte-equivalent) is the gate; ideally also vs Pearl
  reference vectors (Phase B1).
- B1 reuses C3 (proven), so the plain side is low-risk; B2 is
  where the effort/risk concentrates.

## 6. Cross-references

- Noise reference (mirror exactly): `ai-pow::matmul`
  (`BlockNoise::expand`, `e_row_into`/`f_col_into`),
  `ai-pow::prng` (`expand_e_l_row`, `e_r_col_positions`,
  `pearl_permutation_pair`, `fill_uniform_row`).
- Lever: `ai-pow-zk::chips::input` (the `NOISED_PACKED =
  polyval(MAT_UNPACK)+polyval(NOISE_UNPACK)` constraint).
- C3 pattern to reuse for B1: the `IS_MSG_MAT·(BLAKE3_MSG −
  base256(UINT8_DATA))` binding (`composite_full_air`).
- M-S1 store: `composite_trace::place_noised_store_row` /
  `enumerate_noised_chunks`.
- Roadmap: `PRODUCTION_ROADMAP.md` Phase A3; `HIGH2_2_DESIGN.md`
  §7 / §4.C.11.

---

## 8. Maintainer decision 2026-05-17 — **skip c-mset → c-exact** (comparison + c-exact design + staged plan)

After **c-mset.1a** (`2c2d7c6`) the c-mset path required a fork
(the producer-row-isolation mechanism — a CRIT-1-pinned-program
shape decision). Surfaced to the maintainer; **decision:
ABANDON the c-mset `BUS_PLAIN` bus; pursue c-exact**
(position-exact, zero-gap), with this comparison produced first.

> **c-mset.0 / c-mset.1a are NOT discarded.** They are retained
> as the KAT-first de-risk that (a) *justified* this decision
> (c-mset.1a proved the bus needs invasive CRIT-1-program gating
> *and* only honest-balances `16|r`), and (b) establishes facts
> **c-exact directly reuses**: every store window is a
> *contiguous* committed sub-run inside the hashed span
> (c-mset.0) that is *8-aligned == one leaf word-pair* under
> production `16|r` (c-mset.1a). This is the P-B.2.0/D1 pattern:
> exhaustive KATs surfacing the true cost/shape *before*
> invasive AIR work.

### 8.1 c-mset vs c-exact — effort / soundness

| Axis | c-mset (LogUp membership bus) | **c-exact (co-location via C3)** |
|---|---|---|
| Soundness | swept `a′` ∈ {committed windows} + pinned noise — *membership* | swept `a′` = committed byte at the *exact* strip position + noise — *position-exact* |
| Residual | plain-side position-exactness (documented gap) | **none** — true zero-gap §4.C.2 |
| New trace surface | new `BUS_PLAIN` + `PLAIN_FREQ` col + producer-row isolation | reuses the **existing, proven C3** binding; no new bus / FREQ col |
| CRIT-1 pinned-program impact | unavoidable (FREQ width **or** new selector **or** tweak-decode constraint) | C3 already exists/proven; impact = a `CONTROL_PREP`-pinned word-offset + an 8-wide word-pair one-hot main-trace block (the **proven §6(b)/G2 `FOLD_STRIPE_SEL` pattern**): **zero *preprocessed*-width** (§4.C.8 trap avoided) + **zero-blast at `o=0`**; no new bus / FREQ / permutation |
| `16|r` geometry | bus only balances `16|r`; needs a new `P16` Route-A geometry | same `16|r` alignment is *exploited* (store window == leaf word-pair) — c-mset.1a's positive finding is **reused** |
| LogUp-coupling risk (M-S1 unit≠Route-A) | present (new bus + `populate_lookup_freq`) | **absent** — C3 is a per-row algebraic identity, no permutation argument |
| Effort | M-S1-magnitude (new bus arc) **+** the open decision | M-S1-magnitude (the granularity bridge) **−** LogUp/freq risk |

**Conclusion:** c-exact dominates — comparable effort,
*strictly stronger* (zero-gap) result, and it **removes** both
the LogUp-coupling risk class and the open producer-isolation
decision. c-mset's only edge (the well-trodden bus pattern) was
nullified by c-mset.1a showing the bus needs invasive
CRIT-1-program gating regardless. The decision is well-founded.

### 8.2 c-exact design — the 1024-B-chunk ↔ 8-i8-window bridge

**C3** (`IS_MSG_MAT·IS_NEW_BLAKE·(BLAKE3_MSG[j] −
base256(UINT8_DATA[4j..4j+4])) = 0`) binds, on a strip-opening
leaf-chunk **round-0** row, that row's `UINT8_DATA` to its
hashed `BLAKE3_MSG` — which is ∈ `HASH_A` via the strip-opening
tree + the CRIT-1-pinned `HASH_A` PI. Today `UINT8_DATA_LEN=8`
so C3 binds only the first **8** of a leaf block's **64**
hashed bytes (message words j=0,1). The store needs *every*
8-byte window bound to its **exact** committed leaf
sub-position. Mechanisms:

- **(E1 — RECOMMENDED) pinned per-row word-offset.** Keep
  `UINT8_DATA=8`. Co-locate each store row onto the leaf
  compression whose 64-byte block contains its window; add a
  CRIT-1-pinned per-row word-offset `o ∈ {0,2,…,14}` so C3
  binds `BLAKE3_MSG[o+j]` (not fixed j∈{0,1}). Selection uses
  the **proven §6(b)/G2 `FOLD_STRIPE_SEL` pattern**: an 8-wide
  word-pair one-hot main-trace block whose index is folded into
  `CONTROL_PREP` (like §6(a)'s fold-slot / G2's fold-stripe) —
  **zero *preprocessed*-width ripple** (§4.C.8 trap avoided) and
  **zero-blast at `o=0`** (reduces bit-identically to today's
  C3); it does add an 8-col main-trace block
  (`TOTAL_TRACE_WIDTH` change → layout asserts + Route-A
  baselines). `o` is a pure function of the A3.2a
  position-addressed store layout (`noised_store_layout`) + the
  A1 `tile_chunk_range` schedule ⇒ verifier-recomputable
  witness-free (cx.0-validated; the CRIT-1 discipline already in
  place). Position-exact by construction; reuses the *proven*
  C3 + *proven* §6(b)/G2 pin; no bus, no FREQ, no permutation.
- **(E2) widen C3 to 64.** `UINT8_DATA` 8→64, C3 over all 16
  words. One leaf row binds all 64 bytes; heavier width ripple
  than E1.
- **(E3) generic store↔leaf LogUp.** Re-introduces a
  permutation argument — the risk class being moved away from.
  Rejected.

### 8.3 Staged KAT-first plan (R1; the next concrete step is cx.0)

- **cx.0 ✅ DONE & validated 2026-05-17 (`2bbf4cd`)** — KAT-first
  de-risk, **no AIR change**
  (`sec_4c2_cx0_store_binds_exact_committed_leaf_subposition_via_c3`):
  for every A3.2a **position-addressed** store row
  (`enumerate_noised_chunks_positioned`) on a `16|r` geometry
  (r=16 single-chunk **and** r=32 multi-chunk, tile (0,0)):
  (1) `idx = lane_g·k+l0` is 8-aligned ∈ the opened strip ⇒ a
  **unique** leaf address `(chunk, block, even word_off)`;
  (2) `a_pad[idx..idx+8]` (the exact bytes that leaf hashed into
  `HASH_A`) == store plain `MAT_UNPACK` == `a′ − noise_ref`;
  (3) the **exact C3 identity** `BLAKE3_MSG[word_off+j] ==
  base256(plain[4j..4j+4])` holds at that address;
  (4) `(side, src)` (hence the address / `o`) is reproduced by
  the params-pure `noised_store_layout` skeleton (no `a′`) ⇒
  **verifier-recomputable witness-free**. The whole c-exact
  mechanism's premise is validated before any AIR change.
- **cx.1 (next; the first invasive AIR stage — its own staged
  arc, R1).** Generalize C3 from the **fixed** word indices
  `{0,1}` (today `composite_full_air.rs:539-554`,
  `BLAKE3_MSG[j]`) to a verifier-pinned per-row word-offset `o`.
  **Correction to §8.2:** like the proven §6(b)/G2
  `FOLD_STRIPE_SEL` precedent this needs *both* a CONTROL_PREP
  index pin **and an 8-wide word-pair one-hot main-trace block**
  (`BLAKE3_MSG[o+j] = Σ_p MSG_PAIR_SEL[p]·BLAKE3_MSG[2p+j]`,
  `Σ_p MSG_PAIR_SEL[p] = IS_MSG_MAT·IS_NEW_BLAKE`, the one-hot ↔
  pinned-index consistency constrained) — a `TOTAL_TRACE_WIDTH`
  change (rippling to every `composite_layout` assert + every
  Route-A baseline). It is **zero *preprocessed*-width** (the
  §4.C.8 trap is avoided exactly as §6(a)/G2; the pinned offset
  folds into the existing `CONTROL_PREP` polyval) and
  **zero-blast at `o=0`** (reduces bit-identically to today's
  C3 ⇒ all existing traces unchanged — the §6(a) discipline).
  Sub-stages, each validated+committed (R1):
  - **cx.1a** design the `CONTROL_PREP` offset field placement
    (next free bit past §6(a) 2^47/2^48 + G2 2^52) +
    `MSG_PAIR_SEL[8]` layout, against the actual
    `pack_control_prep_full`/`ControlChip`/`extract_program`
    code; no behavior change.
  - **cx.1b** add `MSG_PAIR_SEL[8]` + generalized C3 in the
    **unit `CompositeFullAir`** only, zero-blast at `o=0`; the
    ~300 unit tests + `crit1_*`/`high2_*`/`routea_*` stay green
    (byte-identical); +exhaustive one-hot/offset unit tests.
  - **cx.1c** pin `o` in `CONTROL_PREP`
    (`pack_control_prep_full` + `ControlChip::eval` assert +
    `extract_program` lift — the §6(a)/G2 mechanism); adversarial
    (stale / forged / claimed-absent offset reject);
    `crit1_*` still rejects forgeries with the extended pack;
    debug-assertions-ON.
  Each sub-stage: targeted + `crit1_*`/`routea_*` + `ai-pow
  --features zk` + debug-assertions-ON gate (the M-S1 lesson:
  unit-AIR green ≠ Route-A green; debug-assertions-OFF hazard).
- **cx.2** co-locate store rows onto leaf compression rows
  (`place_matrix_strip_opening`/bridge): the store row *is* the
  leaf round-0 row with `IS_MSG_MAT=1`, `UINT8_DATA` = its
  committed window, `o` pinned from the deterministic schedule.
- **cx.3** Route-A on a `16|r` §6(b)-live single-STARK geometry
  (`P16={m:16,k:64,n:16,noise_rank:16,tile:8}`, c-mset.1a-
  confirmed; **not** `TEST_SMALL`) + adversarial (store window
  ≠ the committed leaf sub-block ⇒ C3 reject = the
  position-exact soundness statement) + debug-assertions-ON +
  full `ai-pow-zk --lib` + `ai-pow --features zk`.
- **A3.3** final gate; §4.C end-to-end; docs /
  `ZKP_SECURITY_REPORT` / `GAP_AUDIT` flip → §4.C.2 **zero-gap**.

Soundness meanwhile unchanged: CRIT-1 + §4.D + §6 + M-S1 + A2 +
the A3.2b noise pin hold; §4.C.2-with-A3.2b is already strictly
stronger than pre-A3 and **not a forgery hole**. c-exact is its
own staged effort (R1). **STATUS: cx.0 ✅ (`2bbf4cd`) ·
cx.1a+cx.1b-layout ✅ (`449ae8f`) · cx.1b-constraints ✅
(`1bb2058`) · cx.1c ✅ (2026-05-18) — cx.1 COMPLETE.** The
generalized C3 is live and its leaf word-pair is CRIT-1
verifier-fixed (the prover cannot choose it); all zero-blast
(every current trace `g=0`, `MSG_PAIR_SEL=0` ⇒ byte-identical).
**Next = cx.2** (co-locate the M-S1 store rows onto the
strip-opening leaf round-0 rows so the generalized C3 actually
binds the real store — the integration step; `IS_MSG_MAT=1` on
those rows ⇒ `g=1` ⇒ C3 ACTIVE, no longer zero-blast → its own
staged effort with the position-exact adversarial + 16|r
Route-A at cx.3) → **cx.3** → **A3.3**. Per R1 each sub-stage
lands only when correct + exhaustively gated (unit +
`crit1_*`/`routea_*` + `ai-pow --features zk` +
debug-assertions-ON).

### 8.4 cx.1a — concrete design (grounded in the live code)

**Current C3** (`composite_full_air.rs:539-554`), gate
`g = IS_MSG_MAT·IS_NEW_BLAKE`:
`g·(BLAKE3_MSG[j] − Σ_{b<4} UINT8_DATA[4j+b]·256^b) = 0`, j∈{0,1}
— **fixed** message words {0,1}. cx.0 proved the store window
lives at words `(2p, 2p+1)`, `p = word_off/2 ∈ 0..8`.

**Generalized C3 (cx.1):** introduce an 8-wide one-hot
`MSG_PAIR_SEL[0..8]` (the proven §6(b)/G2 `FOLD_STRIPE_SEL`
shape). For j∈{0,1}:
`g·( (Σ_{p<8} MSG_PAIR_SEL[p]·BLAKE3_MSG[2p+j]) − Σ_b
UINT8_DATA[4j+b]·256^b ) = 0`, plus `MSG_PAIR_SEL[p]` boolean
and `Σ_p MSG_PAIR_SEL[p] = g` (so exactly one pair is selected
iff the C3 gate is live; degree stays ≤3 = today's C3 degree).

**Pin (cx.1c):** pair index `p = Σ_p MSG_PAIR_SEL[p]·p` folded
into `CONTROL_PREP` at the next free bit past G2's
`FOLD_STRIPE_BIT(52)+FOLD_STRIPE_BITS(6)=58`:
`MSG_PAIR_BIT = 58`, `MSG_PAIR_BITS = 3` (p∈0..8) ⇒ top packed
bit 60 ≪ 64 (Goldilocks-safe). `pack_control_prep_full` gains a
`msg_pair: u8` arg (`pack |= (p&7)<<58`); `ControlChip::eval`
adds `acc += pair_idx·2^58` (mirroring `stripe_idx·2^52`);
`extract_program` already lifts `CONTROL_PREP` (a PROGRAM_COL) ⇒
the offset is CRIT-1-pinned automatically once packed.
`RowDescriptor` gains `msg_pair: u8` (default 0).

**Layout (cx.1b-layout):** append after `FOLD_STRIPE_SEL_END`
(shifts no existing offset — exactly how `FOLD_STRIPE_SEL` was
appended after `SX_END`):
`MSG_PAIR_SEL_START = FOLD_STRIPE_SEL_END`,
`MSG_PAIR_SEL_LEN = 8`, `TOTAL_TRACE_WIDTH = MSG_PAIR_SEL_END`.
Update the one `layout_offsets_are_contiguous` checkpoint
(`FOLD_STRIPE_SEL → TOTAL_TRACE_WIDTH` becomes
`FOLD_STRIPE_SEL → MSG_PAIR_SEL` + `MSG_PAIR_SEL →
TOTAL_TRACE_WIDTH`); `total_trace_width_in_pearl_ballpark`
(<2200) still holds (~1939).

**Zero-blast proof.** Today **no row** has `g=1` (C3 comment
`:533` — "Vacuous on every current trace"). New columns
`MSG_PAIR_SEL` default 0 ⇒ `Σ MSG_PAIR_SEL = 0 = g` ✓ (the
new one-hot constraint holds), generalized-C3 is vacuous
(`g=0`) exactly as today, and `pair_idx = 0` ⇒ `CONTROL_PREP`
gains `+0` ⇒ **byte-identical** for every existing trace (the
§6(a) zero-blast argument). ⇒ the ~300 unit tests +
`crit1_*`/`high2_*`/`routea_*` + `ai-pow --features zk` must
stay green with no value change.

**Sub-stage split (each landed only when its gate is green):**
- **cx.1b-layout** — the additive 8-col block + width + the
  contiguity checkpoint. *No constraint.* Provably zero-blast
  (8 zero-default cols). Gate: `composite_layout` tests + a
  representative composite prove/verify unchanged.
- **cx.1b-constraints ✅ DONE & validated 2026-05-17** —
  generalized C3 (`composite_full_air.rs`): `MSG_PAIR_SEL[p]`
  boolean + `Σ_p MSG_PAIR_SEL[p] == g` + `Σ_p
  MSG_PAIR_SEL[p]·(BLAKE3_MSG[2p+j] − recomposed_j) = 0`
  (degree ≤2, *lower* than the prior deg-3 C3). Validated:
  full `ai-pow-zk --lib` **351/0/22** (byte-identical baseline,
  incl. `crit1_*`/`routea_*`/the C3 negative test — which now
  rejects via the Σ==g constraint); `ai-pow --features zk`
  **85/0/1** (real bridge / MED-3 / §4.C.2 KATs);
  debug-assertions-ON: **all positive composite tests pass
  per-row** (zero-blast at `check_constraints` granularity) —
  the `*_rejects_*` negatives panic under debug-assertions-ON
  by pre-existing design (place violating rows; documented
  M-S1/§6(b) profile behavior, not a cx.1 regression).
- **cx.1c ✅ DONE & validated 2026-05-18** — `CONTROL_PREP`
  pin: `MSG_PAIR_BIT=58`/`MSG_PAIR_BITS=3`,
  `pack_control_prep_full(.., msg_pair)`, `ControlChip::eval`
  `+ pair_idx·2^58` (`pair_idx = Σ_p MSG_PAIR_SEL[p]·p`),
  `RowDescriptor.msg_pair`. `extract_program` lifts
  `CONTROL_PREP` unchanged (a PROGRAM_COL) ⇒ the word-pair is
  CRIT-1 verifier-fixed automatically once packed. Validated:
  control chips **19/0** (new `msg_pair_mismatch_rejected` — a
  `MSG_PAIR_SEL` one-hot ≠ the `CONTROL_PREP`-packed `msg_pair`
  rejects = "prover cannot re-point C3"; all §6(a)/G2 pins green;
  `non_fold_pack_is_unchanged` = zero-blast); full `ai-pow-zk
  --lib` **352/0/22** (+1; `routea_crit1_tampered_program_col_
  rejected` ✓ — CRIT-1 still rejects forgeries with the
  extended pack); `ai-pow --features zk` **85/0/1** (MED-3 /
  real bridge / §4.C.2 KATs); debug-assertions-ON positive
  (`composite_full_air_baseline_trace_verifies`,
  `fold_schedule_consistent_control_prep_verifies`) per-row
  clean. **⇒ cx.1 COMPLETE: the generalized C3's leaf
  word-pair is verifier-fixed; the prover cannot choose it.**

### 8.5 cx.2.0 — structural de-risk FINDING (no AIR/trace-gen change)

Grounding cx.2 (co-locate the M-S1 store rows onto the
strip-opening leaf round-0 rows so the generalized C3 binds the
real store) in the live code surfaced — *before* any invasive
trace-gen change, the R1 KAT-first discipline — two facts:

1. **The `noised_packed` LogUp interaction is NOT the blocker.**
   Co-location sets `IS_MSG_MAT=1` on the store/leaf row ⇒ the
   M52 BLAKE3-side self-query fires
   (`composite_full_air_with_lookups::bus_emit::noised_packed`,
   `populate_lookup_freq` §2186-2197 `if is_msg_mat==1`). This
   is **already modelled** (the M52 self-referential pattern:
   the row both publishes `(MAT_ID,NOISED_PACKED)×−MAT_FREQ` and
   self-queries `×+1`; `key_to_first_row` routing balances it).
   No new freq logic needed.

2. **THE BLOCKER (the long-flagged granularity core
   difficulty, now concrete).** `place_leaf_chunk` places
   **exactly one** BLAKE3 compression — one round-0
   (`IS_NEW_BLAKE`) row — per 64-byte block; it **cannot be
   duplicated** (the strip-opening fold expects the exact
   compression sequence — duplicating breaks the BLAKE3 tree ⇒
   the recomputed root ≠ `HASH_A`). But a 64-byte block holds
   **up to 8** distinct swept store windows
   (`p = (idx%64)/8 ∈ 0..8`), and the generalized C3 binds
   **one** 8-byte `UINT8_DATA` window per row (the
   `CONTROL_PREP`-pinned `msg_pair`). ⇒ naive
   1-store-row-per-leaf co-location leaves **≤7 swept windows
   per block with no C3-binding row**. C3 would bind only one
   window per block; the rest of the sweep stays plain-unbound.

⇒ cx.2 is **not** a mechanical "co-locate"; it needs a
structural decision (a genuine design fork — surface to the
maintainer, the c-mset.1a→decision pattern):

- **(X1) Widen `UINT8_DATA` 8→64 + per-word C3 on the single
  leaf round-0 row.** One row/block carries all 64 committed
  bytes; generalized C3 binds all 16 message words to the
  64-wide `UINT8_DATA`. M-S1 store windows for that block are
  8-byte sub-slices of that row's `UINT8_DATA` (an intra-row
  read, no extra rows). Heaviest width ripple
  (`UINT8_DATA` + dependents + i8u8/urange8 bus emissions all
  8→64) but **structurally clean** (1 row/block, the real
  compression; binds the *entire* hashed block ⇒ every swept
  window in it is covered). Likely the correct end state.
- **(X2) Multi-row block: 1 real compression row + ≤7
  "shadow" C3-only rows** carrying the same block's
  `BLAKE3_MSG` (for C3's `base256` recomposition) but
  `IS_NEW_BLAKE=0` (no compression, no hash-chain
  perturbation) and a per-row `msg_pair`. Needs C3's gate
  decoupled from `IS_NEW_BLAKE` (a new "C3-active" pin) +
  binding the shadow rows' `BLAKE3_MSG` to the real
  compression row's (else the prover forges shadow message
  words) — re-introduces an intra-block indirection ≈ the
  c-mset complexity c-exact was chosen to avoid.
- **(X3) Restrict the M-S1 store to ≤1 window per leaf
  block** (coarsen the sweep-chunk↔store granularity to
  64-byte). Changes M-S1's established 8-byte chunking ⇒
  re-opens M-S1's noised_packed balance (high regression
  surface). Disfavoured.

**Recommendation: X1** (widen `UINT8_DATA` to 64) — the only
option that is both structurally clean (1 row/block = the real
compression, binds the whole hashed block) and free of a new
intra-block indirection. Its cost is a known, mechanical width
ripple (the cx.1b-layout discipline, at larger scale), staged
+ zero-blast-able (default still binds the same bytes).

**STATUS: cx.1 COMPLETE & exhaustively validated (cx.0/cx.1a/
cx.1b-layout/cx.1b-constraints/cx.1c, 5 commits).** cx.2 is
**blocked on the X1/X2/X3 maintainer decision** (a soundness-
structural fork uncovered by the KAT-first de-risk, exactly as
intended — *before* an invasive trace-gen change). Per R1 this
is the validated-subset + precise-residual + surfaced-decision
stop; cx.2's trace-gen integration is **not** to be rushed
across this fork. Next once decided: cx.2.1 (the chosen
structure's KAT de-risk) → cx.2.2 (trace-gen in
`place_matrix_strip_opening`/bridge) → cx.3 (16|r `P16`
Route-A + position-exact adversarial) → A3.3.

### 8.6 X1 — precise in-circuit design + the atomic-landing constraint (cx.2 spec)

Maintainer decision: **X1** (performance/§8.4: ~+3% global width,
flat height/degree, lowest soundness risk). cx.2.1 (`b36fd44`)
validated the X1 whole-block premise on real 16|r geometry
(every swept window = its block's committed sub-slice =
`a′−noise_ref`; 64 block bytes = the 16 `BLAKE3_MSG` words;
`max_windows_per_block ≥ 2` ⇒ the cx.2.0 blocker is genuinely
resolved, non-vacuously).

**Why X1 forces co-location (settled).** A per-window M-S1 store
row and a per-block leaf row tied *across rows* is exactly a
LogUp bus = c-mset (rejected). Zero-gap ⇒ the store producer
and the C3-bound committed bytes must be the **same row**. So
under X1 the **leaf round-0 row (1/block, the real
non-duplicable compression) is the M-S1 `noised_packed`
producer for every swept 8-byte sub-slice of its block.**

**The X1 binding chain (all in-circuit, one row per block):**
```
strip-opening leaf round-0 row B (IS_NEW_BLAKE=1, IS_MSG_MAT=1):
  BLAKE3_MSG[0..16]  = the 64 committed plain bytes of block B
                       → hashed → … → HASH_A  (strip-opening, A2)
  C3 (per-word, all 16 w, gated g=IS_MSG_MAT·IS_NEW_BLAKE):
     BLAKE3_MSG[w] == base256(UINT8_DATA[4w..4w+4])
  ⇒ UINT8_DATA[0..64] ≡ committed plain block B  ∈ HASH_A
  NOISE_UNPACK[0..64] = noise_ref(s_a/s_b) for block B's positions
                        (A3.2b discipline, widened 8→64)
  NOISE_PACKED_PREP    = polyval(NOISE_UNPACK,129)  (CRIT-1-pinned;
                        InputChip eqn1 forces it ⇒ noise = public
                        seed's, prover cannot choose — A3.2b)
  per swept sub-slice p∈0..8 of block B:
     NOISED_PACKED_p = polyval(plain[8p..8p+8]) + polyval(noise[..])
                     = a′  (InputChip eqn2, widened)
     noised_packed bus: publish (MAT_ID, NOISED_PACKED_p) ×−FREQ
  M-S1 sweep query (unchanged) ⊆ these published a′ keys
```
Net (zero-gap §4.C.2): swept `a′` —M-S1 bus→ a published
sub-slice key —InputChip→ `plain ∈ UINT8_DATA` + `noise` ;
`plain` —per-word C3→ `BLAKE3_MSG` —strip-opening→ `HASH_A` ;
`noise` —InputChip eqn1 + CRIT-1 `NOISE_PACKED_PREP`→
`noise_ref(public s_a)`. Every swept input is provably
`noise(the committed, HASH_A-authenticated A/B)`. **No bus
beyond M-S1's existing one; no shadow rows; no permutation
added.** cx.1c's `MSG_PAIR_SEL`/`msg_pair` pin is retained as
the per-published-key sub-slice address (which 8 bytes of the
64 a given `noised_packed` emission covers), now over
`UINT8_DATA` not the word-pair.

**This is an M-S1-class ATOMIC change (the documented M-S1
lesson — `ai_pow_zk_crypto_gaps` memory).** It simultaneously
touches, and must balance together under Route-A +
debug-assertions-ON in **one** landing (incremental ⇒
unit≠Route-A failure, exactly M-S1):
- `composite_layout`: `UINT8_DATA` 8→64 (+ `NOISE_UNPACK` 8→64),
  width/offsets/contiguity asserts (~+112 cols; still <2200).
- `chips/input` (InputChip): `NOISED_PACKED` /
  `NOISE_PACKED_PREP` eqns over 64 (8 sub-slices/row) not 8.
- C3 (`composite_full_air`): per-word over all 16 (the X1 form;
  cx.1b's `MSG_PAIR_SEL` Σ-select → whole-block bind).
- `bus_emit::{urange8,i8u8,noised_packed}`: loops 8→64 / the
  leaf row as multi-sub-slice `noised_packed` producer.
- `composite_trace`: `place_matrix_strip_opening` /
  `place_leaf_chunk` co-locate the split-store columns
  (`UINT8_DATA`/`NOISE_UNPACK`/`NOISED_PACKED`/`NOISE_PACKED_PREP`/
  `MAT_FREQ` + `IS_MSG_MAT=1` + `MSG_PAIR_SEL` + the
  `CONTROL_PREP` `msg_pair`) onto each leaf round-0 row;
  retire the separate `place_noised_store_*` rows.
- `composite_preprocess`/CRIT-1 program rebuild: the
  position-addressed (A3.2a) per-block noise/`msg_pair` schedule
  reconstructed witness-free.
- `populate_lookup_freq`: `noised_packed` (multi-key/leaf-row) +
  `urange8` (64) + `i8u8` re-accounted; honest balance.
- bridge `prove_and_verify_tiled`: row-budget (store rows
  retired into leaf rows) + the §6(b) sweep still fits one
  STARK.

**Gate (one landing):** full `ai-pow-zk --lib` (incl.
`crit1_*`/`routea_*`) + `ai-pow --features zk` (MED-3 / real
bridge) + debug-assertions-ON + a **16|r §6(b)-live single-STARK
Route-A** roundtrip (`P16`, cx.2.1-confirmed) that exercises C3
**active** (`g=1`) end-to-end + the **position-exact
adversarial**: a store window whose plain ≠ the committed leaf
sub-slice ⇒ C3 reject (= the zero-gap soundness statement).
Then **A3.3**: docs / `ZKP_SECURITY_REPORT` / `GAP_AUDIT` flip
→ §4.C.2 **ZERO-GAP**.

**STATUS:** cx.1 COMPLETE + cx.2.0/cx.2.1 de-risked & designed
(8 validated commits). cx.2 is the **M-S1-class atomic
integration** — its own focused atomic effort (the M-S1
precedent: such LogUp-coupled §4.C trace-side changes land
atomically, never incrementally). Per R1 not to be rushed at
the tail of a session; the validated subset (cx.1 + the X1
de-risk/design) + this precise atomic spec is the
R1-mandated checkpoint. Next: execute the cx.2 atomic landing
against this §8.6 spec, then cx.3-equivalent (the Route-A +
adversarial gate above) → A3.3.

### 8.7 cx.2 attempt — concrete wall found by driving it (R1.1): the noise-pin / §4.C.8 collision

Driving the cx.2 implementation (per R1.1: attempt, don't defer)
surfaced — by analysis grounded in the live column shapes — a
real feasibility issue §8.6 under-analyzed on the **noise** side:

- Zero-gap c-exact **forces co-location** (re-derived rigorously
  several ways): a separate-store-row ↔ per-block-leaf-row plain
  tie is necessarily *across rows* = a lookup = c-mset (the
  rejected option). So the producer/store row **must be** the
  leaf round-0 row, and a 64-B block's **≤8 swept sub-slices
  share that one row** (1 compression/block, only round-0 has
  the unpermuted committed `BLAKE3_MSG` C3 needs).
- The **plain** side widens cleanly (UINT8_DATA 8→64, per-word
  C3 — main-trace, the cx.1b discipline). The **noise** side
  does **not**: A3.2b pins noise via `NOISE_PACKED_PREP`, **one
  of the 5 CRIT-1 *preprocessed* PROGRAM_COLS**. 8 sub-slices on
  one row ⇒ 8 pinned noise values ⇒ `NOISE_PACKED_PREP` 1→8
  (preprocessed width 5→12). Widening *preprocessed* is the
  **§4.C.8 trap** (the measured ~10x prover regression was the
  5→69 preprocessed widening). 8 noise-polyvals cannot pack into
  one Goldilocks col (range), and re-deriving noise in-circuit
  from `s_a` is the §3′-rejected heavy PRNG sub-AIR.

**This is R1.1's legitimate "concrete wall hit while driving"**
(not preemptive avoidance — co-location feasibility, plain-side
widen, and the noise-side blocker were all worked out by
attempting the change). It is **not** a stop: the resolution is
a **bounded go/no-go measurement** (the P-B pattern), the
concrete immediate continuation —

- **cx.2.2-measure (next, bounded):** widen `NOISE_PACKED_PREP`
  1→8 as preprocessed and **measure** the prover-cost delta on
  a PROD-class profile (preprocessed 5→12 — *not* 5→69; the
  §4.C.8 ~10x may not apply at this scale). The proof is
  amortized (only on a win), so a modest constant may be
  acceptable; P-A's one-STARK envelope unaffected (preprocessed
  width ≠ trace-area `k(h+w)`). Decide: (a) acceptable ⇒
  proceed with the §8.6 structure + 8-wide `NOISE_PACKED_PREP`;
  (b) prohibitive ⇒ restructure the noise pin (candidates: a
  verifier-fixed noise *lookup table* keyed by the A3.2a
  position schedule — a new bus, weigh vs c-mset; or a single
  packed pin at a coarser granularity). Measurement gates the
  structure; do not widen preprocessed blindly (R1: no rushing
  the CRIT-1 linchpin across an unmeasured §4.C.8 cost).

**STATUS:** cx.1 COMPLETE (validated). cx.2 **in flight** — the
attempt established: co-location is forced; the plain side
(UINT8_DATA 8→64 + per-word C3) is the cx.1b-discipline path;
the **noise side is gated on the cx.2.2-measure go/no-go**
(NOISE_PACKED_PREP 1→8 preprocessed cost). That measurement is
the immediate next action (bounded, ~one PROD-profile prove),
**not** a deferred "future session". After it: the corrected
§8.6 structure lands atomically (full `ai-pow-zk --lib` +
`ai-pow --features zk` + debug-assertions-ON + 16|r `P16`
Route-A with C3 active + position-exact adversarial) → A3.3.

### 8.8 §8.7 §4.C.8 go/no-go — MEASURED & RESOLVED (cx.2-pcols)

cx.2-pcols landed `NOISE_PACKED_PREP` 1→8 / `PROGRAM_COLS` 5→12
(zero-blast: 7 added preproc cols 0 while g=0). The full
`ai-pow-zk --lib` gate ran in **988.87s vs the ~570s
(352/0/22) baseline ≈ 1.7×** suite-level prover overhead for
preproc 5→12. **This is NOT the §4.C.8 ~10× trap** (that was
the 5→69 widening). A ~1.7× preproc-commit constant on an
*amortized* proof (PROD proves only on a win; P-A's
one-STARK trace-area envelope is unaffected — preproc width ≠
`k(h+w)`) is acceptable. **GO: proceed with X1's 8-wide
`NOISE_PACKED_PREP`; no noise-pin restructure (X2/alt) needed.**
The §8.7 open measurement is closed. CRIT-1 soundness intact:
`routea_crit1_tampered_program_col_rejected` ✓ with the 12-wide
preprocessed pin.

### 8.9 cx.2 g=1 co-location flip — design + cx.2-coloc.0 de-risk (validated)

The zero-blast foundation is complete (cx.2-layout/c3/pcols/
mat-input/matfreq/bus — 6 validated commits; the full X1
machinery present & inert at g=0). The remaining single
irreducibly-atomic step (the g=1 flip) is now **designed +
KAT-first de-risked** (the cx.0/cx.2.1 discipline; "design and
validate the next step"):

**Trace-gen design.** `place_matrix_strip_opening` gains a
co-location context `{seed: &[u8;32] (s_a|s_b), k, r, side}`;
threaded through `fold_strip`/`subtree_inside` to
`place_leaf_chunk`. For each leaf compression's **round-0 row**
`cr` (where `IS_NEW_BLAKE=1`, holding the unpermuted 64-byte
block in `BLAKE3_MSG`), after placing the compression, write:
`IS_MSG_MAT=1` (⇒ g=1); `MAT_UNPACK[0..64]/UINT8_DATA[0..64] =
the block's committed bytes` (i8/u8 — the bytes it already
hashes; cx.2-c3 whole-block C3 binds these ∈ `HASH_A`);
`NOISE_UNPACK[0..64] = noise_ref` at each byte's matrix
position (A row-major `row=p/k,col=p%k`→`e_value`; B col-major
`col=p/k,kidx=p%k`→`f_value`); `NOISED_PACKED[0..16] = a′`
(InputChip-8 then holds non-vacuously); `NOISE_PACKED_PREP[0..8]
= polyval(noise_subslice,129)` (the CRIT-1 pin; `extract_program`
lifts it from the honest trace — verifier-recompute-from-params
is the separate Phase-A-CR item); `MSG_PAIR_SEL[0]=1` +
`CONTROL_PREP msg_pair=0` (satisfy cx.1b-(ii)/cx.1c — vestigial
under X1's whole-block C3). Re-write `CONTROL_PREP`/selectors on
`cr` as `{IS_NEW_BLAKE, IS_MSG_MAT}` + `pack_control_prep_full(..
msg_pair=0)`. Bridge (`prove_and_verify_tiled`) passes the
context for the A (`s_a`,row-major) and B (`s_b`,col-major)
strip-openings and **retires the separate
`place_noised_store_*`** (the leaf rows are now the producers).

**cx.2-coloc.0 (DONE & validated — `sec_4c2_cx2coloc0_leaf_
producer_superset_and_noise_pin`).** Off-circuit, no
AIR/trace-gen change, real bridge geometry (16|r: r=16
single-chunk + r=32 multi-chunk): **(P1)** the
opened-leaf-block sub-slice producer set ⊇ every distinct
M-S1-swept `a′` chunk (`enumerate_noised_chunks_positioned`)
⇒ the `noised_packed` LogUp stays balanced once the producer
moves onto the leaf rows; **(P2)** each sub-slice
`NOISE_PACKED_PREP = polyval(noise_ref,129)` is well-formed &
bounded (`< 2^60 ≪ p`). §4.C.2 family 6/6 green.

**Status.** The g=1 flip is designed + its two load-bearing
premises validated KAT-first. It remains the single
non-decomposable Route-A-valid landing (the M-S1-class atomic
core — per the documented M-S1 precedent a focused
multi-iteration effort with the debug-assertions-OFF hazard).
Next focused effort: implement the cx.2-coloc trace-gen +
bridge per the above → full `ai-pow-zk` + `ai-pow zk` +
debug-assertions-ON + 16|r `P16` Route-A C3-active +
position-exact adversarial → A3.3 → §4.C.2 **ZERO-GAP**.

### 8.10 cx.2 g=1 co-location flip — LANDED & validated (`5109852`)

The single irreducibly-atomic g=1 flip is **implemented,
16|r-gated, and exhaustively validated**. The §4.C.2 c-exact
plain tie is **LIVE end-to-end on the production-faithful 16|r
path**:

- `place_matrix_strip_opening`/`fold_strip`/`subtree_inside`/
  `place_leaf_chunk` thread `Option<&[i8]> noise_strip`; each
  co-located leaf round-0 row gets `IS_MSG_MAT=1` (⇒ g=1) + the
  8 sub-slices (committed plain→`MAT_UNPACK`/`UINT8_DATA[0..64]`,
  `noise_ref`→`NOISE_UNPACK[0..64]`, a′→`NOISED_PACKED[0..16]`,
  `polyval(noise_s,129)`→`NOISE_PACKED_PREP[0..8]`,
  `MSG_PAIR_SEL[0]=1`, `CONTROL_PREP msg_pair=0`).
- bridge: per-strip `noise_ref` precompute (A row-major / B
  col-major, pad→0); `coloc = (r % 16 == 0)` — 16|r ⇒ leaf rows
  are the M-S1 producers (separate `place_noised_store_*`
  retired); non-16|r ⇒ pre-cx.2 A3.2b separate-store path
  (TEST_SMALL etc.; co-location only honest-balances 16|r —
  cmset.1a/cx.2-coloc.0; Pearl §4.8 is always 16|r).

**Validated:** the **decisive**
`sec_4c2_cx2_g1_p16_route_a_c3_active_roundtrip` — 16|r `P16`
`prove_and_verify_for_block` proves + pow-verifies at real
difficulty with **C3 ACTIVE (g=1)** (co-located producers +
whole-block C3 + InputChip-8 + 8-key `noised_packed` + sweep all
balance end-to-end; `HASH_A` = the real committed-matrix
commitment). `ai-pow-zk --lib` 352/0/22 (g=0/None path intact);
`ai-pow --features zk` 88/0/1 (P16 g=1 + non-16|r A3.2b +
§4.C.2 family 6/6); **debug-assertions-ON P16 g=1 roundtrip
per-row clean** (the M-S1 debug-assertions-OFF hazard CLOSED for
the honest g=1 path). The C3-active-**reject** mechanism (tamper
the committed-plain `UINT8_DATA` view on a g=1 row ⇒ whole-block
C3 rejects) is validated by `c3_rejects_is_msg_mat_row_with_
mismatched_blake_msg` (passes under cx.2-c3's whole-block C3 in
every 352/0/22 run).

**Net (§4.C.2 on 16|r):** committed A/B ∈ `HASH_A` via the
active whole-block C3 + strip-opening; swept a′ = noise(committed)
via the leaf-row `noised_packed` producers (cx.2-coloc.0
producer⊇consumer); noise = `noise_ref(public s_a)` via the
CRIT-1 `NOISE_PACKED_PREP` pin. **The §4.C.2 plain tie is
closed end-to-end on the production path.**

**Precise residual to formally flip the §4.C.2 ZERO-GAP claim
in `ZKP_SECURITY_REPORT`/`GAP_AUDIT` (not yet asserted — no
premature/fake completion, R1):** a **cx.2-bridge-specific
position-exact adversarial** — tamper a co-located `P16` leaf
row's committed plain in the *actual bridge trace* and assert
`prove_and_verify_for_block` rejects (the constraint-level
reject is already validated generically via `c3_rejects_*`;
this is end-to-end hardening). Then A3.3: docs/`ZKP_SECURITY_
REPORT`/`GAP_AUDIT` flip → §4.C.2 **ZERO-GAP** (16|r); non-16|r
remains the documented A3.2b strictly-stronger-than-pre-A3
state. Soundness held throughout (CRIT-1+§4.D+§6+M-S1+A2+A3.2b).
