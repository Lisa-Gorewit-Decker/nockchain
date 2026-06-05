# AI-PoW Proof Sizes, Soundness, And Assumptions

Date: 2026-06-05
Status: current measurement and cryptographic-assumption checkpoint.

## Scope

This note answers four concrete questions for the current AI-PoW proving stack:

1. How large is the regular proof of the AI-PoW puzzle?
2. How large is the recursive proof of that proof?
3. How sound is each layer?
4. Which cryptographic assumptions are taken at each step, and are they cited?

Here "regular proof" means the Layer-0 `CompositeFullAirWithLookupsPinned`
batch-STARK produced by `composite_prove_pinned_logup`. It does not mean the
legacy/plain `MatmulProof`, which is a miner diagnostic and pre-ZKP target-hit
object, not the production block artifact. Production block/wire integration is
the structured recursive certificate.

## Current Sizes

| Artifact | Current production role | Last measured size | Source |
|---|---|---:|---|
| Layer-0 composite proof | Regular STARK proof of the AI-PoW puzzle statement; consumed by recursion; diagnostic/intermediate, not persisted by consensus | `303,896` bytes / `296.8 KiB` | `2026-05-29_AI_ZKP_NOUN_WIRE_SPEC.md`, `prod_recursion_measure 15` |
| Historical L1 recursive certificate | Batch-STARK proof that an L1 verifier circuit accepted the Layer-0 proof; superseded for the terminal path | `205,446` bytes / `200.6 KiB` fixed-int bincode (`231,235` bytes / `225.8 KiB` legacy postcard) | same `prod_recursion_measure 15` run |
| Active terminal recursive certificate | Current production terminal proof of the recursive-verifier relation | `99,289` bytes / `97.0 KiB` | `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`, production checkpoint |

The active production answer is therefore:

- regular Layer-0 proof: **296.8 KiB** if materialized;
- active recursive proof/certificate: **97.0 KiB**;
- older non-terminal recursive certificate: **200.6 KiB**, retained only as
  historical context.

## Soundness Summary

| Layer | Parameters | Soundness claim | PoW counted? |
|---|---|---:|---|
| Layer-0 composite STARK | `log_blowup=4`, `num_queries=15`, `pow_bits=1` in `CircuitConfig::PROD` | 60 pure FRI-query bits, 62 bits under the code's Johnson accounting including the two one-bit PoW hooks | No PoW is needed to reach 60 bits; the two bits are extra margin |
| Historical L1 batch-STARK recursive certificate | `log_blowup=4`, `num_queries=9`, `query_pow_bits=24`, `cap_height=5` | 60 bits under mixed query/PoW Johnson accounting | Yes; this is why it is not the final terminal profile |
| Active terminal recursive certificate | `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`, `max_log_arity=3`, `log_final_poly_len=0` | 60 pure FRI-query bits, conditionally on the selected Plonky3 FRI theorem/assumption and terminal theorem | No |
| End-to-end accepted recursive certificate | L0 proof accepted inside terminal recursive-verifier relation | At most the minimum of the L0 and terminal layers: **60 bits** | No terminal PoW; L0 already has 60 pure-query bits |

The recursive certificate does not make the underlying Layer-0 statement more
sound. It replaces the large Layer-0 proof object with a smaller proof that the
recursive verifier accepted that Layer-0 proof. A successful forgery must either
forge the Layer-0 STARK statement, forge the terminal proof that the verifier
accepted it, or break one of the transcript/commitment assumptions that bind
the two.

## Logic Flow

```text
AI-PoW attempt data
  |
  | native puzzle checks derive nonce-bound kappa, matrix commitments,
  | noised matmul values, cumsum, jackpot message, and jackpot hash
  v
Layer-0 composite STARK
  proves:
    - canonical program is pinned, not prover-selected;
    - public inputs are bound:
      cumsum, jackpot, HASH_A, HASH_B, JOB_KEY, COMMITMENT_HASH,
      HASH_JACKPOT;
    - BLAKE3 matrix/jackpot hash AIR rows match the public commitments;
    - noised matrix/matmul/range/i8-u8/cv routing lookups are globally
      consistent through LogUp;
    - FRI openings prove low-degree trace/quotient consistency.
  |
  | recursive verifier circuit runs the Layer-0 verifier
  v
Active terminal recursive certificate
  proves:
    - the verifier circuit was executed with the committed Layer-0 proof,
      public inputs, relation profile, and production parameters;
    - primitive verifier-circuit rows satisfy the sparse-R1CS row-product
      argument;
    - supported Tip5/recompose NPO rows satisfy the merged
      residual-zero/recompose/value-bridge proof;
    - 5-round Tip5 lookup trace, byte-table LogUp, and selected-vs-trace
      NPO-IO LogUp are consistent;
    - terminal FRI openings are valid under the canonical 15-query,
      zero-query-PoW profile.
  |
  v
Nockchain block/wire artifact: structured recursive certificate
```

## Assumptions By Step

### 1. Native AI-PoW Attempt And Public Statement

Cryptographic assumptions:

- BLAKE3 behaves as a collision-resistant hash and keyed hash/MAC for matrix
  commitments and jackpot hashing.
- The nonce/ticket attempt state is unique: changing the nonce, ticket, matrix,
  noise, target, or public commitments changes the derived statement before
  proof construction.
- The verifier recomputes the public statement instead of trusting prover-supplied
  metadata.

Citations and anchors:

- BLAKE3 specification: O'Connor, Aumasson, Neves, Wilcox-O'Hearn,
  "BLAKE3: one function, fast everywhere",
  <https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf>.
- Current statement-binding docs:
  `2026-05-31_AI_POW_ONE_MATMUL_ONE_ATTEMPT_AUDIT.md` and
  `crates/ai-pow/src/zk_bridge.rs`.

### 2. Layer-0 Composite STARK

Cryptographic assumptions:

- The Plonky3 STARK reduction is sound for the committed AIR, public inputs,
  LogUp buses, and quotient identities.
- FRI proximity/opening verification is sound for the production Goldilocks
  rate and 15 transcript-derived queries.
- Fiat-Shamir challenges are modeled as random-oracle challenges derived after
  all relevant statement data and commitments are bound.
- The Tip5 Merkle/MMCS commitment is binding/collision-resistant for the
  committed trace, quotient, and lookup columns.
- LogUp rational-sum identities are sound except for standard
  Schwartz-Zippel/denominator-pole failure probabilities over the extension
  challenge field.

Implementation anchors:

- `crates/ai-pow-zk/src/composite_proof.rs` documents the production Layer-0
  family: `composite_prove_pinned_logup` /
  `composite_verify_pow_pinned_logup`.
- `crates/ai-pow-zk/src/circuit.rs::CircuitConfig::PROD` sets
  `log_blowup=4`, `num_queries=15`, and `pow_bits=1`.
- `crates/ai-pow-zk/README.md` records the current Layer-0 soundness policy:
  60 pure query bits, 62 under the code's Johnson accounting with the two
  one-bit PoW hooks.

Citations:

- STARKs: Ben-Sasson, Bentov, Horesh, Riabzev, "Scalable, transparent, and
  post-quantum secure computational integrity", IACR ePrint 2018/046,
  <https://eprint.iacr.org/2018/046.pdf>.
- FRI: Ben-Sasson, Bentov, Horesh, Riabzev, "Fast Reed-Solomon Interactive
  Oracle Proofs of Proximity", ICALP 2018,
  <https://doi.org/10.4230/LIPIcs.ICALP.2018.14>.
- DEEP-FRI context: Ben-Sasson, Goldberg, Kopparty, Saraf, IACR ePrint
  2019/336, <https://eprint.iacr.org/2019/336>.
- Fiat-Shamir for FRI and batched FRI: Block, Garreta, Katz, Thaler, Tiwari,
  Zajac, IACR ePrint 2023/1071, <https://eprint.iacr.org/2023/1071>.
- LogUp/logarithmic-derivative lookups: Haboeck, IACR ePrint 2022/1530,
  <https://eprint.iacr.org/2022/1530>.
- Tip5: Szepieniec, Lemmens, Sauer, Threadbare, Al Kindi, "The Tip5 Hash
  Function for Recursive STARKs", IACR ePrint 2023/107,
  <https://eprint.iacr.org/2023/107.pdf>.
- Reed-Solomon proximity-gap policy anchor used by current repo docs:
  Ben-Sasson, Carmon, Haboeck, Kopparty, Saraf, "On Proximity Gaps for
  Reed-Solomon Codes", IACR ePrint 2025/2055,
  <https://eprint.iacr.org/2025/2055>.

### 3. Recursive Verifier Circuit

Cryptographic assumptions:

- The recursive verifier circuit faithfully implements the native Layer-0
  verifier transcript, commitment observations, FRI query derivation, Merkle
  path checks, LogUp checks, and public-input binding.
- The in-circuit 5-round Tip5 operations match the native 5-round Tip5
  permutation used by Layer-0 challenger/MMCS commitments.
- The recursive statement binds the same Layer-0 public-input vector and
  relation/profile metadata that the native verifier would use.

Implementation anchors:

- `crates/ai-pow-zk/src/recursion.rs::recurse_composite_to_l1` defines the
  pipeline: prove Layer 0, build the L1 verifier circuit, verify Layer 0
  in-circuit, and prove the verifier circuit.
- `crates/plonky3-recursion/recursion/src/pcs/fri/params.rs` requires the safe
  `with_mmcs` constructor for production FRI verification; the arithmetic-only
  path is explicitly unsafe/test-only.

Citations:

- Plonky3 recursion model: <https://plonky3.github.io/Plonky3-recursion/introduction.html>.
- Tip5 paper as above.
- FRI and Fiat-Shamir-for-FRI references as above.

### 4. Active Terminal Recursive Certificate

Cryptographic assumptions:

- The terminal certificate binding digest commits to protocol id, production
  proof kind, public-values digest, proof-body digest, relation digest, and
  proof parameters before backend challenges are derived.
- Primitive verifier-circuit rows are soundly reduced by the sparse-R1CS
  row-product/sumcheck argument.
- Supported Tip5/recompose NPO rows are soundly covered by the merged
  residual-zero/recompose/value-bridge proof and integrated Tip5 AIR/LogUp /
  selected-vs-trace NPO-IO proof.
- The terminal FRI/PCS proof is sound under the active pure-query production
  profile.
- Terminal `TerminalCompressedFriProof` is soundness-neutral serialization: it
  restores a normal Plonky3 FRI proof and does not serialize query indices.

Implementation anchors:

- `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md` records the active
  production terminal tuple:
  `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`,
  `max_log_arity=3`, `log_final_poly_len=0`.
- The same spec records the production checkpoint:
  `99,289` bytes / `97.0 KiB`, integrated NPO compact FRI payload
  `80,885` bytes / `79.0 KiB`, `total_prove=23.688s`,
  `total_verify=62.6ms`.
- `crates/plonky3-recursion/recursion/src/terminal.rs` defines
  `TerminalProductionProof`, `TerminalProductionNpoPolynomialProof`,
  `prove_terminal_production_goldilocks`, and
  `verify_terminal_production_goldilocks`.

Citations:

- Spartan and Aurora give the row-product/sumcheck/R1CS design lineage:
  Setty, "Spartan: Efficient and general-purpose zkSNARKs without trusted
  setup", CRYPTO 2020 / IACR ePrint 2019/550,
  <https://eprint.iacr.org/2019/550>; and Ben-Sasson, Chiesa, Riabzev,
  Spooner, Virza, Ward, "Aurora: Transparent Succinct Arguments for R1CS",
  IACR ePrint 2018/828, <https://eprint.iacr.org/2018/828.pdf>.
- FRI, Fiat-Shamir-for-FRI, LogUp, and Tip5 citations are the same as the
  Layer-0 section because the terminal proof uses the same families of
  assumptions.

## What Is Not Being Assumed

- No trusted setup.
- No KZG, pairing-friendly curve, Groth16, or Plonkish SNARK wrapper.
- No Plonky2 proof system in production. Pearl/Plonky2 code was read only as a
  design reference for safe FRI path compression, and the native terminal
  compressor is implemented in the vendored Plonky3-recursion stack.
- No terminal query proof-of-work bits are counted toward the active terminal
  certificate's 60-bit production floor.
- No zero-knowledge claim for the active terminal certificate. It is compact,
  but selected FRI openings may reveal evaluations of witness-derived columns.

## Clear End-To-End Claim

For the current production path, the block-facing artifact is a structured
recursive terminal certificate of **99,289 bytes / 97.0 KiB**. It proves that
the recursive verifier accepted the Layer-0 AI-PoW composite STARK statement.
The materialized Layer-0 proof is **303,896 bytes / 296.8 KiB**, but it is an
intermediate diagnostic artifact rather than the consensus wire object.

The end-to-end soundness floor is **60 bits**, with the following reduction:

1. If the AI-PoW computation/public statement is false, a valid Layer-0 proof
   requires breaking the Layer-0 STARK/FRI/LogUp/Tip5/BLAKE3 assumptions or
   exploiting a bug in the AIR/public-input binding.
2. If the Layer-0 verifier would reject, a valid terminal certificate requires
   breaking the terminal row-product/NPO/FRI/Tip5 binding assumptions or
   exploiting a bug in the recursive-verifier relation.
3. The certificate binds public values, relation/profile metadata, commitments,
   and production parameters before challenge derivation, so there is no
   intended grinding surface over public values, profiles, roots, or query
   indices.

The weakest active production soundness term is the terminal profile's 60 pure
FRI-query bits. The older 200.6 KiB recursive certificate also had a 60-bit
claim, but it got there through mixed query/PoW accounting and is superseded by
the active pure-query terminal profile.
