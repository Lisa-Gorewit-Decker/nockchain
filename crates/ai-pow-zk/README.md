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

## Dependencies

Pinned to a git revision of [Plonky3](https://github.com/Plonky3/Plonky3)
in `Cargo.toml` because the upstream publishes irregularly to
crates.io. Currently pulls in the minimal subset for AIR + uni-STARK
over BabyBear (`p3-air`, `p3-baby-bear`, `p3-field`, `p3-matrix`,
`p3-uni-stark`). Add the FRI / Poseidon2 / Merkle / commit / challenger
sub-crates when wiring real proving:

- `p3-challenger`
- `p3-commit`
- `p3-dft`
- `p3-fri`
- `p3-merkle-tree`
- `p3-poseidon2`
- `p3-symmetric`

## Status

Pre-implementation. Filing this scaffold so the API shape and dependency
boundaries can land in a separate commit from the circuit logic itself.
