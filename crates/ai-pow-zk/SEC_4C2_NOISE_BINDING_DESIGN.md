# §4.C.2 / Phase-A3 — store ↔ committed-plain-strip noise-derivation binding (design)

> **PROGRESS 2026-05-17:** A3.0/A3.1/A3.2a/**A3.2b all DONE &
> validated** — the §4.C.2 **noise tie is closed** (store
> `NOISE_UNPACK` forced to `noise_ref` of the C1-public seed via
> InputChip + the CRIT-1 `NOISE_PACKED_PREP` pin; `ai-pow-zk
> --lib` 351/0/22, `ai-pow --features zk` all-binaries 0-failed,
> MED-3 roundtrip green through the split store). **Maintainer
> decision (hybrid):** the remaining **plain tie (B1)** ships as
> **c-mset interim now → c-exact scoped as the zero-gap
> completion** (§7 staged plan; §3′ c-exact/c-mset). c-mset is a
> new M-S1-pattern LogUp bus (store `MAT_UNPACK` ⊆ committed-
> plain windows) — **soundness-critical + invasive ⇒ staged,
> KAT-first, not rushed (R1)**; it is itself ≈M-S1-magnitude
> (M-S1's one bus took a long staged arc). c-exact (position-
> exact, zero-gap) is the documented Phase-A3 residual after
> c-mset. Soundness meanwhile: CRIT-1 + §4.D + §6 + M-S1 + A2 +
> the A3.2b noise pin hold; §4.C.2 with A3.2b is already
> *strictly stronger than pre-A3* (store noise = public-seed
> Pearl noise, not prover-chosen) and **not a forgery hole**.
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
  - **c-mset.0** off-circuit/KAT de-risk (the M-S1 coverage-net
    / P-B.2.0 discipline): against the real bridge geometry,
    the strip-opening's committed-plain byte-window multiset ⊇
    the store `MAT_UNPACK` window multiset (the property the
    bus enforces) — no AIR change.
  - **c-mset.1** add `BUS_PLAIN`: strip-opening leaf rows
    *publish* committed-plain 8-byte windows; store rows
    *query* `MAT_UNPACK`. AIR `push_interaction` + bus const.
  - **c-mset.2** `populate_lookup_freq` accounting for the new
    bus (producer freq); honest balance.
  - **c-mset.3** Route-A + debug-assertions-ON + adversarial: a
    store `MAT_UNPACK` ∉ the committed-plain multiset rejects;
    full `ai-pow-zk --lib` + `ai-pow --features zk`. (The
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
