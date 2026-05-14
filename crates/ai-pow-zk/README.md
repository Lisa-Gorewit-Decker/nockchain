# `ai-pow-zk`

EXPERIMENTAL — a Plonky3 SNARK circuit for the [`ai-pow`](../ai-pow/)
tiling matmul puzzle. The role is the same as Pearl's
[`zk-pow`](../../pearl/zk-pow/): wrap the multi-MB plain proof in a
~60 KB SNARK so it can fit in a block certificate.

This crate is currently a **scaffold**: every entrypoint is a
`todo!()`, and the layout exists so callers in `ai-pow` can wire up
call sites independently of the circuit work.

## Public surface

| Item | Role |
|---|---|
| `prove(block_commitment, nonce, params, proof)` | Pearl-analog of `zk_prove_plain_proof` — produces a `ZkProof` from a plain `MatmulProof`. |
| `verify(params, public_inputs, proof)` | Pearl-analog of `ZKProof::verify` — checks the SNARK against public inputs. |
| `PublicInputs` | The chain-pinned public values: `params_tag`, `h_a`, `h_b`, `comm_M`, `(found_i, found_j)`, `found_leaf`. |
| `Witness` | The private side: tile strips of A, B, per-stripe `TileState`s, noise factors. |
| `MatmulAir` | The Plonky3 AIR struct encoding Pearl §4.5 tile-loop constraints. |
| `CircuitConfig` | FRI rate, PoW grinding bits, query count. |

## Where this fits in the `ai-pow` flow

`ai-pow`'s `mine` produces a plain [`MatmulProof`](../ai-pow/src/proof.rs)
containing strip data + per-row Merkle paths. At the same point Pearl
invokes `zk_prove_plain_proof` (see
[`pearl/zk-pow/src/api/prove.rs:18`](../../pearl/zk-pow/src/api/prove.rs#L18)),
`ai-pow` calls into this crate via the `zk` feature flag.

## Stack

Goldilocks base field + Tip5 sponge for FRI + `p3-blake3-air` for the
BLAKE3 sub-circuit. See [`DESIGN.md`](DESIGN.md) for the per-slot
rationale, trace column layout, public-input encoding, witness
shapes, constraint-count estimate, and parameter choices.

Pulled in from upstream Plonky3 (`https://github.com/Plonky3/Plonky3`):

- `p3-air` — AIR trait
- `p3-blake3-air` — BLAKE3 keyed-hash sub-circuit
- `p3-challenger`, `p3-commit`, `p3-dft`, `p3-fri`, `p3-merkle-tree`,
  `p3-symmetric` — STARK config plumbing
- `p3-field` — field arithmetic
- `p3-goldilocks` — base field
- `p3-tip5` — FRI sponge
- `p3-matrix`, `p3-uni-stark` — trace + prover

## Status

Pre-implementation. Scaffold ships:

- Public entry points (`prove`, `verify`, `ZkParams`, `PublicInputs`,
  `Witness`, `ZkProof`).
- AIR struct shapes (`MatmulAir`, `Blake3SubAir`) over Goldilocks.
- `CircuitConfig` with `TEST` and `PROD` defaults.
- Wired into `ai-pow` at the Pearl-analog `zk_prove_plain_proof`
  call site under the `zk` feature flag.
- `DESIGN.md` with the logical plan + parameter choices.

Every body is currently `todo!()` — the circuit logic lands in
follow-up commits.
