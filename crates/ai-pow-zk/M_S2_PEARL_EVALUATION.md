# M-S2 (G3a/G3b) — Evaluation Against Pearl's Implementation & Paper

> **Status:** EVALUATION (2026-05-17). Requested by maintainer:
> "evaluate the proposed M-S2 design against Pearl's
> implementation and paper."
> **Verdict up front:** the M-S2/G3 *carry-vector segmentation +
> recursive aggregation* architecture is **NOT how Pearl works**.
> Pearl deliberately **caps its PoW parameters so one opened tile
> always fits a single STARK**, and uses recursion only for
> *vertical* proof-compression. This is a material finding: it
> means M-S2/G3 is solving a scaling problem Pearl *designed
> away*, and several premises in the G3 doc family
> (`G3_RECURSION_AGGREGATION.md`, `HIGH2_2_DESIGN.md` §4.C.4-G3,
> the "G4 = Pearl §4.8 spot-check" framing) are inaccurate and
> need correction. A decision is surfaced at the end.

## Sources

- **Academic paper:** Komargodski & Weinstein, *Proofs of
  Useful Work from Arbitrary Matrix Multiplication*
  (arXiv:2504.09971v4) — `2504.09971v4.pdf`. Read: abstract,
  §1–2 (PoUW model), §6 (Encode/Decode instantiations), Remarks
  2.1–2.3, App. B (Poisson process).
- **Pearl whitepaper:** *Pearl // whitepaper* (Pearl Research
  Labs) — `Pearl_Whitepaper.pdf`. Read: §3 (PoUW Overview),
  §3.1.1–3.1.5, §4.1–4.8 (implementation, **§4.8 Supported PoW
  Parameters**), §5.1 (block structure).
- **Pearl implementation:** `pearl/zk-pow/src/circuit/*`
  (Plonky2/Starky), esp. `pearl_program.rs:36-57`
  (`expected_num_rows`/`degree_bits`), `pearl_circuit.rs`
  (3-layer recursion), `pearl_trace.rs`,
  `chip/jackpot/constraints.rs:85-87`; plus a full-codebase
  segmentation/carry/aggregation sweep (Explore agent,
  2026-05-17).

---

## 1. What Pearl actually does (faithful summary)

### 1.1 The mineable unit is ONE tile — tiles are independent

Paper Remark 2.1: instead of hashing the whole transcript, "hash
each of the `(n/r)³` intermediate matrices **separately**, and
use each one of them as a potential proof for solving the
puzzle." Whitepaper §3.1.3: the miner keeps per-tile state
`M_{i,j} ∈ {0,1}^512`; after the k-sweep, tile `(i,j)` is
*opened* iff `BLAKE3(seed, M_{i,j}) < 2^{256-b}`. §3.1.4:
randomness extraction ⇒ "each block product has an independent
and fair chance." The matmul yields `(n/t)²·(k/r)` **independent
lottery tickets**; there is **no sequential dependency between
tiles** and no global "transcript carry."

### 1.2 The SNARK proves ONE opened tile, only on a win

Paper Remark 2.3: the SNARK is generated "only when the (rare)
event that a valid π is found … the cost of this computation is
amortized." Whitepaper §3.1.5 / §4.7: the zkSNARK attests *"there
exist strips consistent with the commitments to A and B such that
their product yields an accumulated digest `M_{i,j}` whose BLAKE3
hash equals a published `h`."* The proof statement is **a single
opened tile's** strip-product → rotate-XOR-13 fold → keyed BLAKE3
— exactly the ai-pow-zk §6(b) sweep+fold for **one** tile.

### 1.3 Parameters are CAPPED so one tile = one STARK

Whitepaper **§4.8 "Supported PoW Parameters"** (this is the
section the ai-pow-zk docs miscite as a "spot-check protocol" —
it is not):

- `m, n ≤ 2²⁴`
- `16r ≤ k ≤ 4r²`, **`k ≤ 2¹⁶`**, `64 | k`
- `r ∈ {2⁵, 2⁶, …, 2¹⁰}`
- tile shapes = bounded 3-D arithmetic progressions
- `h·w ≥ 32`
- **`k(h+w) ≤ 2²²`** — *the verifier restricts this.*

These caps bound the per-tile proof trace. Pearl's impl
(`pearl_program.rs:54-56`):
`degree_bits = expected_num_rows().next_power_of_two().max(MIN_STARK_LEN).ilog2()`
with `expected_num_rows ≈ num_tiles·(k/r)·instr_per_r`. The §4.8
caps keep this within a **single** (large, padded ≤ ~2²²)
Plonky2 STARK. Pearl never segments because it **forbids
parameters that would overflow one STARK**.

### 1.4 Recursion is VERTICAL compression, not aggregation

Whitepaper §4.7: Pearl uses **Plonky2** (hash-based, hand-AIR,
preprocessed columns for the agreed noise) and *"3-layered
recursion, resulting in a final proof size below 60KB"* — the
recursion's stated purpose is *"first prove the claim as fast as
possible … then recursively compress the proof aggressively."*
Pearl impl confirms (Explore sweep + `pearl_circuit.rs`): Layer-0
= one STARK over one trace; Layer-1 = a Plonky2 circuit that
*verifies that single STARK*; Layer-2 = a ZK wrapper. **One**
`add_virtual_stark_proof_with_pis(...)` — a single inner proof.
No `CrossTableLookup`, no multi-segment verifier, no carry
equality, no per-segment program root. §5.1: the on-chain
certificate is ≤ 65KB (the compression target).

### 1.5 No carry vector; no spot-check; PIs are endpoints only

Pearl's STARK PIs are endpoints only: `JOB_KEY`,
`COMMITMENT_HASH`, `HASH_A`, `HASH_B`, `HASH_JACKPOT`
(`pearl_layout.rs` `pearl_public`). State is initialized fresh
at row 0 (`jackpot/constraints.rs:85-87`:
`constraint_first_row(jackpot_msg[i])` zeroes `M`). There is **no
`Γ`-analogue, no cross-proof state hand-off, and no probabilistic
spot-check** anywhere in Pearl. The §4.6 "Block Opening Proof"
reveals Merkle strips for **the one opened tile** (the plaintext,
pre-zkSNARK opening) — *not* a sample of many tiles. Pearl's
matmul-truth for the opened tile is **zero-gap** (the SNARK
proves the full per-tile computation), achieved by capping
parameters, not by sampling.

---

## 2. M-S2/G3 vs Pearl — point by point

| Aspect | M-S2 / G3 design | Pearl (paper + impl) | Assessment |
|---|---|---|---|
| Mineable unit | one tile's fold (✓ already in ai-pow-zk) | one tile's fold | **Aligned** |
| SNARK-on-win, amortized | ✓ (zk_bridge on solve) | ✓ Remark 2.3 / §4.7 | **Aligned** |
| Scale strategy | **horizontal segmentation** of one tile's k-sweep into `N` segments + carry `Γ` + recursive aggregation tree | **cap `k ≤ 2¹⁶`, `k(h+w) ≤ 2²²`** so one tile = one STARK; **no segmentation** | **Divergent — core finding** |
| Carry vector `Γ` | ~85-elem cross-segment baton, flat PIs, chained in G3c | **none** (no cross-proof state) | **No Pearl precedent** |
| Recursion role | aggregate *many* segment proofs (horizontal) + carry stitch | *compress one* proof (vertical), <60KB | **Different kind of recursion** |
| `program_root` across segments | new CRIT-1-across-tree obligation | **none** (single program) | **No Pearl precedent** |
| Probabilistic gap | "zero gap, strictly stronger than Pearl spot-checks" | **already zero-gap** (no spot-check exists); gap is `ε_FRI` only | **Premise wrong** (see §3) |
| Proof system | p3-batch-stark (Plonky3), Tip5 | Plonky2, hash-based, Goldilocks | Orthogonal (both hash-STARK) |

### 2.1 The central finding

**M-S2/G3 introduces a sequential-segmentation + carry +
recursive-aggregation architecture that Pearl does not have and
does not need.** Pearl's answer to "the per-tile k-sweep is big"
is *bound the parameters* (`k ≤ 2¹⁶`, `k(h+w) ≤ 2²²`,
`r ≤ 2¹⁰`) so the single-tile proof always fits one (large,
padded) STARK, then *vertically* compress for a succinct
on-chain certificate. The hard, soundness-critical machinery
G3 invents — `Γ`, the carry stitch, `PROGRAM_ROOT`-across-tree,
segment adjacency/count/order, the G3c recursion verifier — has
**no analogue in Pearl** and exists only to support an
architecture Pearl deliberately avoided.

### 2.2 Is ai-pow-zk's PROD really past one STARK?

The G3 docs justify segmentation with *"true PROD: `k/r = 64`,
sweep ≈ 2²⁰ rows ≫ one Layer-0."* Under Pearl's own caps:
`r ≤ 2¹⁰`, `k ≤ 2¹⁶` ⇒ `k/r ≥ 16` and `≤ 4r`. With `r = 2¹⁰`,
`k = 2¹⁶`, `k/r = 64` — i.e. ai-pow-zk's "true PROD" sits **at
Pearl's parameter ceiling**, and Pearl proves *exactly that* in
**one** STARK (trace ≤ ~2²², the `k(h+w) ≤ 2²²` cap). Pearl's
Plonky2 Layer-0 routinely handles `~2²²`-row traces; the binding
constraint is **prover economics for a single big trace**
(memory/time), which Pearl *accepts* and addresses with vertical
recursion for the *certificate*, not by segmenting the witness.

⇒ The premise that PROD *requires* horizontal segmentation is
**not established against Pearl**. The real open question is
narrower and economic: *can ai-pow-zk's Plonky3 Layer-0 prove a
single ~2²⁰–2²² trace within mining economics?* If yes (Pearl's
bet), **G3 is unnecessary** — adopt §4. If ai-pow-zk's per-row
width makes a 2²² single trace economically infeasible where
Pearl's is not, G3 (or a different segmentation) is justified —
but that case must be **measured**, not assumed (it currently is
assumed: `HIGH2_2_DESIGN.md` §4.C.4-G3 has no prover-cost
measurement for the single-big-trace alternative).

---

## 3. Inaccuracies in the current doc family (must correct)

These statements, repeated across `G3_RECURSION_AGGREGATION.md`
§13, `M_S2_G3AB_DESIGN.md` §0/§13, `HIGH2_2_DESIGN.md`
§4.C.4-G3 / §13, and the M-S1-era `GAP_AUDIT.md` /
`ZKP_SECURITY_REPORT.md` "G4" framing, are **contradicted** by
Pearl's paper + impl:

1. *"Pearl's protocol is already Layer-0 + recursion/aggregation
   (Pearl proves bounded work and **aggregates**)."* — **False.**
   Pearl proves the *whole opened tile* (not "bounded work" that
   is then aggregated) in one STARK; its recursion is vertical
   compression, not aggregation of segments.
2. *"G3 is the faithful instantiation of Pearl's architecture,
   with one upgrade (zero gap, strictly stronger than Pearl's
   spot-checks)."* — **Doubly wrong.** (a) G3's
   carry-segmentation is *not* Pearl's architecture; (b) Pearl
   has **no spot-checks for matmul truth** — its per-tile SNARK
   is already zero-gap (modulo `ε_FRI`). There is nothing for G3
   to be "strictly stronger than" here.
3. *"G4 = the Pearl §4.8 spot-check externality (`MatmulProof.spot`),
   parity with Pearl, authoritative for PROD until G3c."* —
   **Conflates three distinct things** (verified in code):
   - **ai-pow's** `MatmulProof.spot: Vec<TileOpening>` +
     `params.spot_checks` (`proof.rs:54`, `params.rs:33` — 8
     test / **80 PROD**) is *ai-pow's own* plain (non-SNARK)
     probabilistic light-verification: reveal `spot_checks`
     **random** tile openings, verifier recomputes them. It is
     real and is the actual G4 interim mechanism — but it is
     **ai-pow's design, not Pearl's**.
   - Pearl whitepaper **§4.8 = "Supported PoW Parameters"**
     (parameter caps), *not* a spot-check.
   - Pearl whitepaper **§4.6** reveals the **single opened
     tile's** Merkle strips (one tile, before the §4.7 zkSNARK
     subsumes it) — *not* `N` random tiles.
   ⇒ The G4 framing is correct that ai-pow *has* a spot-check
   interim, but wrong to call it "Pearl §4.8" or "parity with
   Pearl." Pearl's per-tile matmul-truth is **zero-gap within
   capped params**; ai-pow's `spot_checks` interim is
   *probabilistic and weaker than Pearl*, not parity. *Action:*
   re-attribute G4 to ai-pow's own `MatmulProof.spot`, drop the
   "Pearl §4.8 / parity-with-Pearl" claim, and state plainly
   that the Pearl-faithful target is zero-gap-per-tile (capped
   params), of which ai-pow's spot-check is a *weaker* stand-in.
4. *"matmul-truth at scale rests on the §4.8 spot-check protocol
   — probabilistic, external."* — **Not Pearl.** Pearl's
   matmul-truth is the zero-gap per-tile SNARK within capped
   params; "external" in Pearl is only the *difficulty* check
   (MED-3-analogue) and the *plaintext §4.6 opening* before the
   zkSNARK subsumes it (§4.7).

None of these change ai-pow-zk's *current* soundness (CRIT-1 +
§4.D + §6 + M-S1 hold regardless); they change the **roadmap
rationale** and the **G4 framing** — which is exactly what
M-S2/G3 is predicated on. They should be corrected before more
G3 work is sequenced.

---

## 4. The Pearl-faithful alternative to M-S2/G3

If ai-pow-zk matches Pearl's actual design, the Track-A
"PROD-scale" milestone becomes **far smaller and lower-risk**
than G3a/G3b/G3c:

- **P-A. Adopt Pearl's §4.8 parameter caps as the ai-pow-zk PROD
  envelope.** `k ≤ 2¹⁶`, `k(h+w) ≤ 2²²`, `r ∈ {2⁵..2¹⁰}`,
  `64 | k`, tile shapes as bounded 3-D APs. Verifier-enforced
  (params are already public/MED-3-derived). This *defines away*
  the overflow: every legal PROD puzzle is one Layer-0 trace.
- **P-B. Raise the Layer-0 trace ceiling** from
  `MIN_STARK_LEN = 2¹³` to a PROD ceiling (≤ ~2²², Pearl's
  bound), exactly as Pearl's `degree_bits` grows. p3-uni/batch-
  stark already supports arbitrary `2^k`. **Measure** the
  single-big-trace prover cost at the cap (the missing datum in
  §2.2) — this is the real go/no-go.
- **P-C. Vertical recursion for the certificate only** (the
  M-S3/M-S4/M-S5 substrate, but used to *compress one* proof to
  a ≤65KB on-chain cert — Pearl §4.7 / §5.1 — not to aggregate
  segments). This still needs the vendored recursion + Tip5/
  Poseidon2 work (P0–P6) but **drops the entire `Γ` / carry
  stitch / `PROGRAM_ROOT`-across-tree / adjacency surface**
  (the G3c bespoke glue the audit `G3_RECURSION_AUDIT.md` flags
  as the riskiest, unaudited part).
- **G4 stays** as the documented interim for *anything outside
  the caps* — but reframed correctly: it is *parameter
  restriction*, not a "Pearl spot-check."

**Trade-off.** M-S2/G3 buys matmul-truth for **arbitrarily
large per-tile `k`** (beyond Pearl's `k ≤ 2¹⁶`). If ai-pow-zk
genuinely targets loads past Pearl's envelope, G3's segmentation
is the way and the design stands (with §3's framing corrected).
If ai-pow-zk targets *parity with Pearl* (the stated G4 north
star), the Pearl-faithful path P-A/P-B/P-C is **strictly
simpler, lower soundness-risk, and is what the paper+impl
actually do.**

---

## 5. Decision surfaced to the maintainer

The pivotal question is **which scaling architecture Track-A
should pursue**, now that we know Pearl does *not* segment:

- **(α) Proceed with M-S2/G3 as designed** (carry-vector
  segmentation + recursive aggregation). Justified only if
  ai-pow-zk targets per-tile `k` **beyond** Pearl's `2¹⁶` cap;
  accept the larger soundness surface (Γ stitch / PROGRAM_ROOT /
  adjacency / G3c bespoke glue). Correct §3's Pearl-relationship
  framing but keep the milestone.
- **(β) Pivot to the Pearl-faithful path** (P-A param caps +
  P-B raise Layer-0 ceiling + measure + P-C vertical-recursion
  certificate). Smaller, lower-risk, matches paper+impl; caps
  ai-pow-zk's per-tile `k` at Pearl's `2¹⁶`. Replace
  M-S2/G3a-b/G3c with P-A/P-B/P-C in the roadmap.
- **(γ) Hybrid / staged:** do P-A+P-B now (immediate, cheap,
  unblocks PROD within Pearl's envelope), and keep G3 **designed
  but deferred** as the "beyond-Pearl-envelope" future option,
  pursued only if a concrete load past `k = 2¹⁶` is required.

Regardless of α/β/γ, the **§3 doc-family corrections should be
applied** (they are factual fixes, independent of the
architecture choice).

Recommendation: **(γ)** — P-A/P-B are low-risk, directly
PROD-unblocking, and Pearl-faithful; G3's heavy machinery should
not be built until a need beyond Pearl's parameter envelope is
demonstrated. But this is a product/scope call (how ambitious is
ai-pow-zk's target vs. Pearl parity), which is the maintainer's.

---

## 6. Cross-references

- Pearl whitepaper §3.1.3–3.1.5 (per-tile open), **§4.8
  (Supported PoW Parameters — the real §4.8)**, §4.7 (Plonky2 +
  vertical 3-layer recursion <60KB), §5.1 (≤65KB certificate).
- arXiv:2504.09971v4 Remarks 2.1 (per-block independence), 2.3
  (SNARK only on win, amortized).
- Pearl impl: `pearl/zk-pow/src/circuit/pearl_program.rs:36-57`,
  `pearl_circuit.rs` (single inner STARK proof, vertical
  recursion), `chip/jackpot/constraints.rs:85-87`.
- Affected ai-pow-zk docs to correct (§3):
  `G3_RECURSION_AGGREGATION.md` §13, `M_S2_G3AB_DESIGN.md`
  §0/§13, `HIGH2_2_DESIGN.md` §4.C.4-G3/§7/§13, `GAP_AUDIT.md`
  + `ZKP_SECURITY_REPORT.md` (the "G4 = Pearl §4.8 spot-check"
  lines), and the `ai_pow_zk_crypto_gaps` memory.
- M-S1 (done, independent): commit `3feae98`.
