> _Created **2026-06-04** after the terminal-backend size checkpoint and
> the prover-side side-review follow-up._

# Prover-Side Security Audit Checkpoint

## Scope

This checkpoint audits the current prover-side evidence against the
2026-06 side review at
`/Users/loganallen/Documents/Reports/ai-pow-prover-side-review-2026-06.txt.md`.
It covers the active split-view co-location concern, the recursive
certificate API boundary, public-input shape documentation, and the
terminal-compression cryptographic assumptions that remain live.

This is not final audit signoff for the whole terminal-compression goal.
It is a tracked checkpoint of what is currently proved by code, tests, and
literature references.

## Literature Anchors

- STARK/FRI setting: Ben-Sasson, Bentov, Horesh, Riabzev,
  "Scalable, transparent, and post-quantum secure computational
  integrity", IACR ePrint 2018/046,
  <https://eprint.iacr.org/2018/046.pdf>.
- FRI proximity testing: Ben-Sasson, Bentov, Horesh, Riabzev,
  "Fast Reed-Solomon Interactive Oracle Proofs of Proximity", ICALP
  2018, <https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ICALP.2018.14>.
- DEEP-FRI soundness context: Ben-Sasson, Goldberg, Kopparty, Saraf,
  "DEEP-FRI: Sampling outside the box improves soundness",
  <https://arxiv.org/abs/1903.12243>.
- Recursive hash choice: Szepieniec, Lemmens, Sauer, Threadbare, Al
  Kindi, "The Tip5 Hash Function for Recursive STARKs", IACR ePrint
  2023/107, <https://eprint.iacr.org/2023/107.pdf>.
- Lookup-argument context: logarithmic-derivative lookup / LogUp lineage,
  including "Improving logarithmic derivative lookups using GKR",
  IACR ePrint 2023/1284, and the indexed-small-table follow-on
  <https://eprint.iacr.org/2025/946>.
- Fiat-Shamir risk model: non-interactive transcripts are treated in the
  random-oracle / Fiat-Shamir model, so statement, commitments, profile,
  and public values must be transcript-bound before challenges. See
  Unruh, "Post-Quantum Security of Fiat-Shamir",
  <https://eprint.iacr.org/2017/398.pdf>.

## Side-Review Findings

### P0: 64-byte co-location split-view gap

Status: **closed in current code, with strengthened cheap coverage in this
checkpoint.**

Current code evidence:

- `crates/ai-pow-zk/src/composite_layout.rs` pins
  `MAT_UNPACK_LEN = 64`, `MAT_UNPACK_WIN = 64`,
  `UINT8_DATA_LEN = 64`, `UINT8_DATA_WIN = 64`,
  `NOISE_UNPACK_WIN = 64`, `NOISED_PACKED_LEN = 16`, and
  `MAT_FREQ_LEN = 8`.
- `crates/ai-pow-zk/src/composite_full_air_with_lookups.rs::bus_emit::i8u8`
  emits paired `MAT_UNPACK[i]` / `UINT8_DATA[i]` lookups for
  `i in 0..MAT_UNPACK_WIN.min(UINT8_DATA_WIN)`, i.e. all 64 bytes.
- `bus_emit::noised_packed` publishes all eight sub-slice table keys,
  emits all matmul-side A/B sub-slice queries, and emits all eight
  BLAKE3-side self-queries when `IS_MSG_MAT` is live.
- `CompositeTrace::place_leaf_chunk` writes the full co-located 64-byte
  block into `MAT_UNPACK`, `UINT8_DATA`, `NOISE_UNPACK`, and all 16
  `NOISED_PACKED` cells on the round-0 BLAKE3 row.
- `CompositeTrace::populate_lookup_freq` counts all 64 i8/u8 pairs and
  all eight `noised_packed` self-queries.

Regression evidence:

- `non_first_subslice_split_view_rejected_by_i8u8_logup` proves that
  tampering bytes in later sub-slices rejects under the STARK verifier.
- `non_first_subslice_mat_freq_drop_rejected_by_logup` proves that
  dropping a later sub-slice's `MAT_FREQ` rejects.
- `full_msg_mat_row_populates_i8u8_freq_for_all_64_bytes` now checks
  all 64 byte positions cheaply.
- `full_msg_mat_row_populates_mat_freq_for_all_8_subslices` now checks
  all eight sub-slice frequencies cheaply.

This matches the lookup-argument requirement: every consumer of the
co-located row is tied to the same committed byte stream, and the
multiplicity columns cannot silently omit the later sub-slices.

### P2: Risky recursive-certificate helper boundary

Status: **closed in current code.**

The formerly dangerous shape was a public helper named like a canonical
certificate builder while accepting caller-supplied Layer-0 proof/program
parts. Current code exposes
`ChainVerifiedCompositeProof` and
`prove_recursive_certificate_from_chain_verified_composite_proof`.
Construction of `ChainVerifiedCompositeProof` is `unsafe` and documented
as requiring prior chain-statement verification. Production bridge call
sites construct it only after deriving/verifying the canonical statement,
program, public inputs, target, selected work unit, commitments, nonce,
and production/full-work boundary.

### P3: Stale public-input shape documentation

Status: **closed in current code/docs searched in this checkpoint.**

Current documentation describes `CompositePublicInputs` as 60 field
elements:

`cumsum(4) + jackpot(16) + hash_a(8) + hash_b(8) + job_key(8) +
commitment_hash(8) + hash_jackpot(8)`.

The stale "20 field elements" claim was not found in active
`ai-pow-zk`, `ai-pow`, `ai-pow-miner`, or `zk-pow-miner` sources/docs.

## Terminal-Compression Audit Notes

The production terminal profile is now pure-query 60-bit:
`log_blowup=4`, `num_queries=15`, `query_pow_bits=0`. This matches the
maintainer instruction not to rely on proof-of-work bits for the
terminal proof's soundness floor.

The current terminal-backend candidate clears the size target in the
latest measured production profile:

- production compact recursive certificate: `87,103` bytes (`85.1 KiB`)
- FRI-native residual-zero+recompose+value-bridge candidate:
  `99,647` bytes (`97.3 KiB`)

The remaining audit work is not size measurement; it is transcript and
commitment review of the terminal backend after the combined
selected+lookup commitment:

- verify every public value and profile value is bound before challenge
  sampling;
- verify no prover-dependent polynomial is introduced after the
  challenge that is not committed beforehand;
- verify all merged selected/lookup openings are dimension-checked and
  domain-checked;
- verify terminal FRI query count stays 15 and `query_pow_bits` stays 0
  in every production path;
- repeat performance measurement after any major constraint or
  parameter change.

2026-06-04 follow-up: the merged residual-zero+recompose+value-bridge
verifier test now directly covers the selected+lookup commitment and merged
opening-shape items above. It mutates the combined selected+lookup
commitment and expects prelude commitment rejection; mutates lookup and
value-bridge quotient profiles and expects verifier recomputation rejection;
and truncates/malforms lookup IO and value-bridge quotient openings and
expects dimension-check rejection. The focused test and full
`p3-recursion` crate suite passed before commit
`32c6c2ba Harden terminal value bridge verifier tests`.

2026-06-04 follow-up: the optimized Tip5 lookup trace profile now carries a
Tip5 digest of the verifier-fixed preprocessed L-table rows. The backend
relation digest and lookup-trace FRI transcripts absorb that digest, so a
profile-compatible table substitution cannot share the same terminal statement
or challenge stream. Focused native/parallel tests cover profile construction,
trace construction, support-bridge verification, AIR algebra verification, and
the merged value-bridge checkpoint before commit.

## Commands Run For This Checkpoint

```text
cargo test -p ai-pow-zk full_msg_mat_row_populates -- --nocapture
cargo test -p ai-pow-zk non_first_subslice -- --nocapture
cargo test -p ai-pow-zk
```

All passed on 2026-06-04. The full crate run reported 397 passed, 23
ignored, 0 failed in the library tests, plus the WHIR prototype smoke
test passed.
