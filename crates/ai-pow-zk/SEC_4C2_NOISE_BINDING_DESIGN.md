# §4.C.2 / Phase-A3 — store ↔ committed-plain-strip noise-derivation binding (design)

> **Status:** DESIGN (2026-05-17). The last open §4.C soundness
> tie and Phase-A3 of `PRODUCTION_ROADMAP.md`. Milestone-scale
> (comparable to M-S1) ⇒ design + **staged, KAT-first** landing
> (the P-B.2.0 discipline that paid off).
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

## 3. B2 — the noise-derivation sub-AIR (the milestone core)

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
