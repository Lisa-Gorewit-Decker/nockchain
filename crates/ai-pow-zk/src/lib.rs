//! Plonky3 SNARK circuit for the `ai-pow` tiling matmul puzzle.
//!
//! Mirrors Pearl's `zk-pow` role: where Pearl uses Plonky2 to compress
//! its multi-MB `PlainProof` into a ~60 KB `ZKProof` (see
//! `pearl/zk-pow/src/api/prove.rs::zk_prove_plain_proof`), this crate
//! uses Plonky3 over Goldilocks + Tip5 + FRI to do the equivalent for
//! the `ai-pow` plain proof.
//!
//! ## Architectural note
//!
//! `ai-pow-zk` is intentionally **standalone** — it does **not** depend
//! on `ai-pow`. The proving crate (`ai-pow`) is the consumer; making
//! `ai-pow-zk` depend back on it would introduce a circular workspace
//! dep. The caller in `ai-pow` constructs [`ZkParams`], [`Witness`], and
//! [`PublicInputs`] from its own types ([`ai_pow::params::MatmulParams`],
//! [`ai_pow::proof::MatmulProof`]) at the call site.
//!
//! ## Scope of the current [`prove`] / [`verify`] entries
//!
//! The current entrypoints prove a **single tile cell** end-to-end
//! through the [`circuit::AiPowStarkConfig`] STARK stack:
//!
//!   * The AIR is [`composite_air::MatmulTileAir<2>`] (M9.1) — fuses
//!     [`matmul_chip::MatmulCellAir`] (M6, per-stripe `r`-wide INT8
//!     dot-product accumulator) with [`state_chip::StateChipAir`]
//!     (M7, Pearl §4.5 rotate-XOR-13 state update). Cross-chip
//!     linkage `x = c_out` is enforced via two's-complement sign
//!     extension; a single-slot state chain carries across rows.
//!   * The trace is generated from `witness.a_rows[0..k]` and
//!     `witness.b_cols[0..k]`, i.e. the first tile-row of `A'` and the
//!     first tile-column of `B'`.
//!   * The 42-element [`PublicInputs`] vector is passed to
//!     `p3_uni_stark::prove` as the public-values channel (M10).
//!     Plonky3 absorbs them into the Fiat-Shamir challenger; a
//!     verifier with a different `PublicInputs` will derive different
//!     query points and reject.
//!   * **M10.1a (out-of-circuit fast path)**: the trace's final
//!     tile-state value `m_final` is exposed as a public-value-bound
//!     element. The verifier recomputes `pow_key` via
//!     [`binding::derive_pow_key`] and `BLAKE3-keyed(m_final_bytes,
//!     pow_key)` via [`binding::compute_found_leaf`] in plain Rust;
//!     mismatch → reject before unpacking the heavy hash proof.
//!   * **M10.1b (in-circuit)**: the same hash relation is also proved
//!     inside the SNARK. `prove` emits a *second* proof in the
//!     envelope — a [`found_leaf_air::Blake3FoundLeafAir`] trace that
//!     constrains BLAKE3 compression on Pearl's `(message = [m_final,
//!     0, …, 0], key = pow_key, counter = 0, block_len = 64, flags =
//!     0x1B)` to produce `public_inputs.found_leaf`. Uses the vendored
//!     [`blake3_chip`] fork of `p3-blake3-air` (upstream's chip
//!     silently bypasses `flags`, which would diverge from `ai_pow ::
//!     matmul::TileState::keyed_hash` and break Pearl ↔ Nockchain
//!     merge-mining). Cross-proof consistency: `m_final` appears in
//!     both PI vectors so the same value flows through both proofs.
//!
//! ## Scope NOT yet bound
//!
//!   * **M10.1c: `h_a` / `h_b` matrix bindings.** The witness's
//!     `a_rows` / `b_cols` are not yet tied to the chain-pinned
//!     chunk-Merkle roots `h_a` / `h_b`. An adversary still has the
//!     freedom to pick any `(a, b)` and run the matmul on them —
//!     post-M10.1b they have to do *some* matmul work to find a
//!     passing leaf, but not necessarily on the *useful* matrices
//!     the chain expects. Closing this gap reuses the M10.1b
//!     vendored chip per-row plus chunk-Merkle path constraints.
//!   * **M9.2: multi-slot state routing.** The Pearl §4.5
//!     `step mod 16` slot rotation collapses to single-slot in the
//!     current AIR — only slot 0 of `M` evolves; slots 1..16 stay
//!     zero. `found_leaf` here hashes `[m_final, 0, …, 0]` (16 × i32);
//!     a full Pearl miner would have non-zero values in every slot.
//!     Today, `k / noise_rank` must be a power of two.
//!
//! See [`ROADMAP.md`](https://github.com/nockchain/nockchain/blob/master/crates/ai-pow-zk/ROADMAP.md)
//! for the full milestone list and what's next.

pub mod air;
pub mod binding;
pub mod blake3_air;
pub mod blake3_chip;
pub mod circuit;
pub mod composite_air;
pub mod found_leaf_air;
pub mod input_chip;
pub mod matmul_chip;
pub mod params;
pub mod public;
pub mod state_chip;
pub mod witness;

use bincode::config::standard as bincode_standard;
use p3_field::integers::QuotientMap;
pub use p3_goldilocks::Goldilocks as Val;
use p3_uni_stark::Proof as UniStarkProof;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use crate::air::MatmulAir;
pub use crate::circuit::{AiPowStarkConfig, CircuitConfig};
pub use crate::params::ZkParams;
pub use crate::public::PublicInputs;
pub use crate::witness::Witness;

/// Bincode-serialised Plonky3 STARK proof + `m_final` envelope.
///
/// The composite tile AIR exposes the trace's terminal tile-state
/// value `m_final` (single-slot M9.1 regime — see M9.2 for the
/// 16-slot widening) as a public-value-bound element. The prover
/// transmits both the proof bytes and `m_final` inside this
/// envelope. The verifier:
///
///   1. Recomputes `pow_key` from `(block_commitment, nonce,
///      public_inputs)` via the [`binding`] helpers.
///   2. Checks `BLAKE3_keyed(m_final_bytes, pow_key) ==
///      public_inputs.found_leaf`. Rejects if not.
///   3. Forwards `m_final` into Plonky3's public-values channel so
///      the AIR enforces `trace.last_row.m_out == m_final` (see
///      [`composite_air::PI_M_FINAL_IDX`]).
///
/// Wire format is internal to this crate — consumers persist the
/// `Vec<u8>` verbatim and round-trip it through [`verify`] only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZkProof(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ZkProofEnvelope {
    /// Bincode-serialised composite tile proof (matmul + state + linkage).
    proof_bytes: Vec<u8>,
    /// Claimed final tile-state value (single-slot M9.1 regime).
    m_final: u32,
    /// Bincode-serialised M10.1b BLAKE3 found-leaf proof (in-circuit
    /// keyed-mode hash of `[m_final, 0, …, 0]` under `pow_key`).
    hash_proof_bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum ProveError {
    /// The supplied `Witness` is malformed or inconsistent with `ZkParams`.
    #[error("witness shape mismatch: {0}")]
    Witness(String),
    /// Public inputs are malformed (wrong length / out of range).
    #[error("invalid public inputs: {0}")]
    PublicInputs(String),
    /// `ZkParams::validate` failed.
    #[error("invalid params: {0}")]
    Params(String),
    /// Bincode failed to serialize the Plonky3 proof.
    #[error("proof serialization failed: {0}")]
    Serialize(String),
    /// Parameter shape outside the MVP range supported by [`prove`].
    /// The current MVP only supports `noise_rank = 2`; relaxing this is
    /// pending — see the [`crate`]-level docstring.
    #[error("unsupported params: {0}")]
    Unsupported(String),
    /// The prover-side M10.1a sanity check failed: the trace's
    /// computed `m_final`, hashed under the derived `pow_key`, does
    /// not match `public_inputs.found_leaf`. Means the caller asked
    /// to prove a `found_leaf` value that the witness does not
    /// actually produce.
    #[error(
        "found_leaf binding mismatch: BLAKE3(m_final, pow_key) != public_inputs.found_leaf \
         (caller asked to prove an inconsistent (witness, found_leaf) pair)"
    )]
    FoundLeafMismatch,
}

#[derive(Debug, Error)]
pub enum VerifyError {
    /// The proof bytes are not a well-formed Plonky3 STARK proof.
    #[error("malformed proof: {0}")]
    Malformed(String),
    /// The Plonky3 verifier rejected the proof.
    #[error("plonky3 rejected proof: {0}")]
    Rejected(String),
    /// Public inputs do not pass shape / range validation.
    #[error("invalid public inputs: {0}")]
    PublicInputs(String),
    /// `ZkParams::validate` failed.
    #[error("invalid params: {0}")]
    Params(String),
    /// Parameter shape outside the MVP range supported by [`verify`].
    #[error("unsupported params: {0}")]
    Unsupported(String),
    /// M10.1a found-leaf binding failed: the proof's `m_final`,
    /// hashed under the derived `pow_key`, does not equal the
    /// public-input `found_leaf`. Strong cryptographic rejection —
    /// signed by the BLAKE3 keyed-hash relation, not by the FRI
    /// transcript.
    #[error("found_leaf binding rejected: BLAKE3(m_final, pow_key) != public_inputs.found_leaf")]
    FoundLeafMismatch,
}

/// Build a Plonky3 STARK that attests to the existence of a [`Witness`]
/// producing [`PublicInputs`] for the given `(block_commitment, nonce,
/// params)`.
///
/// **Current scope.** The AIR is the M9.1
/// [`composite_air::MatmulTileAir<2>`] — it proves both the per-stripe
/// INT8 dot-product accumulator (M6) and the rotate-XOR-13 tile-state
/// update (M7) for one `(0, 0)` tile cell, with the `x = c_out` two's-
/// complement linkage and the single-slot state chain. The 42-element
/// [`PublicInputs`] are passed through to Plonky3 as the public-values
/// channel — they're absorbed into the Fiat-Shamir challenger so any
/// mismatch at verify-time rejects the proof, even though the AIR does
/// not yet constrain trace values to specific public inputs (that
/// stronger binding is M9.2+).
///
/// `block_commitment` and `nonce` are not yet bound; they're inputs
/// the caller used upstream to derive the public-input hashes, which
/// *are* bound. Keeping them in the signature for forward compat with
/// future Pearl-equivalent bindings.
pub fn prove(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &ZkParams,
    public_inputs: &PublicInputs,
    witness: &Witness,
) -> Result<ZkProof, ProveError> {
    params.validate().map_err(ProveError::Params)?;
    validate_public_inputs(public_inputs).map_err(ProveError::PublicInputs)?;
    validate_witness_shape(witness, params).map_err(ProveError::Witness)?;
    require_mvp_params(params).map_err(ProveError::Unsupported)?;

    let k = params.k as usize;
    let a = &witness.a_rows[0..k];
    let b = &witness.b_cols[0..k];

    // M10.1a: compute the trace's deterministic terminal state, derive
    // pow_key the same way `ai_pow::fiat_shamir::pow_key_for_nonce`
    // does, and hash the single-slot M to get the *expected*
    // found_leaf. Bail early with a clear error if the caller asked
    // us to prove an inconsistent (witness, found_leaf) pair.
    let m_final = composite_air::MatmulTileAir::<2>::reference_final_state(a, b);
    let pow_key = binding::derive_pow_key(block_commitment, nonce, public_inputs);
    let expected_leaf = binding::compute_found_leaf(m_final, &pow_key);
    if expected_leaf != public_inputs.found_leaf {
        return Err(ProveError::FoundLeafMismatch);
    }

    let mut pis = public_inputs.to_field_elements();
    pis.push(<Val as QuotientMap<u32>>::from_int(m_final));

    let cfg = circuit::build_stark_config(params, &CircuitConfig::TEST);
    let air = composite_air::MatmulTileAir::<2>::new();
    let trace = composite_air::MatmulTileAir::<2>::generate_trace(a, b);
    let proof = p3_uni_stark::prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &pis);
    let proof_bytes = bincode::serde::encode_to_vec(&proof, bincode_standard())
        .map_err(|e| ProveError::Serialize(e.to_string()))?;

    // M10.1b: in-circuit BLAKE3 keyed-mode found-leaf binding.
    // Build a one-call Pearl-compat trace that hashes
    // `[m_final, 0, …, 0]` under `pow_key` and yields `found_leaf`,
    // then prove it through the same `AiPowStarkConfig` with the
    // 17-element found-leaf public-values vector.
    let hash_call = build_found_leaf_call(m_final, &pow_key);
    let hash_pis =
        found_leaf_air::build_public_values::<Val>(m_final, &pow_key, &public_inputs.found_leaf);
    let hash_air = found_leaf_air::Blake3FoundLeafAir::new();
    let hash_trace = blake3_chip::generate_trace_for_calls::<Val>(
        &[hash_call],
        CircuitConfig::TEST.log_blowup as usize,
    );
    let hash_proof =
        p3_uni_stark::prove::<AiPowStarkConfig, _>(&cfg, &hash_air, hash_trace, &hash_pis);
    let hash_proof_bytes = bincode::serde::encode_to_vec(&hash_proof, bincode_standard())
        .map_err(|e| ProveError::Serialize(e.to_string()))?;

    let envelope = ZkProofEnvelope {
        proof_bytes,
        m_final,
        hash_proof_bytes,
    };
    let envelope_bytes = bincode::serde::encode_to_vec(&envelope, bincode_standard())
        .map_err(|e| ProveError::Serialize(e.to_string()))?;
    Ok(ZkProof(envelope_bytes))
}

/// Build the `Blake3HashCall` an honest miner produces for the
/// found-leaf computation: single-block keyed root hash of
/// `[m_final, 0, …, 0]` (16 × i32 LE) under `pow_key`.
fn build_found_leaf_call(m_final: u32, pow_key: &[u8; 32]) -> blake3_chip::Blake3HashCall {
    // BLAKE3 flag bits for a single-block keyed root hash.
    const CHUNK_START: u32 = 1 << 0;
    const CHUNK_END: u32 = 1 << 1;
    const ROOT: u32 = 1 << 3;
    const KEYED_HASH: u32 = 1 << 4;

    let mut message = [0u32; 16];
    message[0] = m_final;
    let mut key = [0u32; 8];
    for i in 0..8 {
        let mut b = [0u8; 4];
        b.copy_from_slice(&pow_key[i * 4..(i + 1) * 4]);
        key[i] = u32::from_le_bytes(b);
    }
    blake3_chip::Blake3HashCall {
        message,
        key,
        counter: 0,
        block_len: 64,
        flags: CHUNK_START | CHUNK_END | ROOT | KEYED_HASH,
    }
}

/// Verify a [`ZkProof`] against a set of [`PublicInputs`] extracted
/// from the chain. Mirrors Pearl's `ZKProof::verify`.
///
/// The verifier reconstructs the same AIR + STARK config as the
/// prover, decodes the bincode-serialised Plonky3 proof, and calls
/// `p3_uni_stark::verify` with `public_inputs.to_field_elements()`.
/// If the verifier's public inputs differ from those that went into
/// `prove`, the Fiat-Shamir challenger absorbs different values and
/// rejection is automatic.
pub fn verify(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &ZkParams,
    public_inputs: &PublicInputs,
    proof: &ZkProof,
) -> Result<(), VerifyError> {
    params.validate().map_err(VerifyError::Params)?;
    validate_public_inputs(public_inputs).map_err(VerifyError::PublicInputs)?;
    require_mvp_params(params).map_err(VerifyError::Unsupported)?;

    let (envelope, _used): (ZkProofEnvelope, usize) =
        bincode::serde::decode_from_slice(&proof.0, bincode_standard())
            .map_err(|e| VerifyError::Malformed(e.to_string()))?;

    // M10.1a fast-path: hash check in plain Rust before unpacking the
    // (large, ~10k-column) BLAKE3 proof. M10.1b's in-circuit proof
    // below ALSO enforces this cryptographically, but the out-of-
    // circuit check is faster to fail and gives a clearer error.
    let pow_key = binding::derive_pow_key(block_commitment, nonce, public_inputs);
    let expected_leaf = binding::compute_found_leaf(envelope.m_final, &pow_key);
    if expected_leaf != public_inputs.found_leaf {
        return Err(VerifyError::FoundLeafMismatch);
    }

    let (decoded, _used): (UniStarkProof<AiPowStarkConfig>, usize) =
        bincode::serde::decode_from_slice(&envelope.proof_bytes, bincode_standard())
            .map_err(|e| VerifyError::Malformed(e.to_string()))?;

    let mut pis = public_inputs.to_field_elements();
    pis.push(<Val as QuotientMap<u32>>::from_int(envelope.m_final));

    let cfg = circuit::build_stark_config(params, &CircuitConfig::TEST);
    let air = composite_air::MatmulTileAir::<2>::new();
    p3_uni_stark::verify::<AiPowStarkConfig, _>(&cfg, &air, &decoded, &pis)
        .map_err(|e| VerifyError::Rejected(format!("composite: {e:?}")))?;

    // M10.1b: verify the in-circuit BLAKE3 found-leaf proof. The
    // verifier supplies all three pieces of public data (m_final from
    // the envelope, pow_key derived above, found_leaf from PIs); the
    // hash AIR constrains the trace's row-0 inputs/outputs to those
    // values AND proves real BLAKE3 keyed-mode compression through
    // them. Sharing `m_final` between the two proofs' public values
    // links them.
    let hash_pis = found_leaf_air::build_public_values::<Val>(
        envelope.m_final, &pow_key, &public_inputs.found_leaf,
    );
    let (hash_decoded, _used): (UniStarkProof<AiPowStarkConfig>, usize) =
        bincode::serde::decode_from_slice(&envelope.hash_proof_bytes, bincode_standard())
            .map_err(|e| VerifyError::Malformed(e.to_string()))?;
    let hash_air = found_leaf_air::Blake3FoundLeafAir::new();
    p3_uni_stark::verify::<AiPowStarkConfig, _>(&cfg, &hash_air, &hash_decoded, &hash_pis)
        .map_err(|e| VerifyError::Rejected(format!("hash: {e:?}")))?;

    Ok(())
}

/// Validate the public inputs without requiring a roundtrip through
/// `to_field_elements` / `from_field_elements`.
fn validate_public_inputs(pi: &PublicInputs) -> Result<(), String> {
    if pi.params_tag.len() != 32 {
        return Err(format!(
            "params_tag must be 32 bytes, got {}",
            pi.params_tag.len()
        ));
    }
    Ok(())
}

/// Check the witness's per-field lengths match `params`.
fn validate_witness_shape(w: &Witness, p: &ZkParams) -> Result<(), String> {
    let expected = Witness::expected_lengths(p);
    if w.a_rows.len() != expected.a_rows {
        return Err(format!(
            "a_rows length {} != expected {}",
            w.a_rows.len(),
            expected.a_rows
        ));
    }
    if w.b_cols.len() != expected.b_cols {
        return Err(format!(
            "b_cols length {} != expected {}",
            w.b_cols.len(),
            expected.b_cols
        ));
    }
    if w.a_rows.len() < p.k as usize {
        return Err(format!(
            "a_rows has {} entries; need at least k={}",
            w.a_rows.len(),
            p.k
        ));
    }
    if w.b_cols.len() < p.k as usize {
        return Err(format!(
            "b_cols has {} entries; need at least k={}",
            w.b_cols.len(),
            p.k
        ));
    }
    Ok(())
}

/// Pin the MVP support window.
///
/// The composite AIR is parameterised by a const-generic `STRIPE`,
/// which we currently fix at `2`. The state chain inside
/// [`composite_air::MatmulTileAir`] additionally requires
/// `num_stripes = k / noise_rank` to be a power of two (so the trace
/// height is power-of-two without padding rows that conflict with
/// the rotate-XOR transition). Supporting other `noise_rank` values
/// and non-power-of-two stripe counts is M9.2+.
fn require_mvp_params(p: &ZkParams) -> Result<(), String> {
    if p.noise_rank != 2 {
        return Err(format!(
            "MVP entry supports noise_rank = 2 only (got {})",
            p.noise_rank
        ));
    }
    let num_stripes = (p.k / p.noise_rank) as usize;
    if num_stripes == 0 || !num_stripes.is_power_of_two() {
        return Err(format!(
            "MVP entry requires k / noise_rank to be a power of two (got k={} r={} → {} stripes)",
            p.k, p.noise_rank, num_stripes
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mvp_params() -> ZkParams {
        ZkParams {
            m: 8,
            k: 16,
            n: 8,
            noise_rank: 2,
            tile: 2,
            difficulty_bits: 0,
        }
    }

    fn mvp_witness(p: &ZkParams) -> Witness {
        // Deterministic, reproducible filler — the MVP entry only
        // exercises `a_rows[0..k]` and `b_cols[0..k]`.
        let lens = Witness::expected_lengths(p);
        let i8s = |n: usize, salt: u8| -> Vec<i8> {
            (0..n)
                .map(|i| ((i as u32).wrapping_mul(salt as u32) as i32 % 256 - 128) as i8)
                .collect()
        };
        let pairs = |n: usize, salt: u32| -> Vec<(u32, u32)> {
            (0..n)
                .map(|i| {
                    let r = (i as u32).wrapping_mul(salt);
                    (r % p.noise_rank, (r + 1) % p.noise_rank)
                })
                .collect()
        };
        Witness {
            a_rows: i8s(lens.a_rows, 3),
            b_cols: i8s(lens.b_cols, 5),
            e_l: i8s(lens.e_l, 7),
            e_r_pos: pairs(lens.e_r_pos, 11),
            f_r: i8s(lens.f_r, 13),
            f_l_pos: pairs(lens.f_l_pos, 17),
            tile_states: (0..lens.tile_states)
                .map(|s| {
                    let mut row = [0i32; 16];
                    for j in 0..16 {
                        row[j] = ((s as i32 + 1) * (j as i32 + 1)) ^ 0x12345678;
                    }
                    row
                })
                .collect(),
        }
    }

    /// Build the public inputs for a test by mining honestly: compute
    /// the trace's `m_final` for the witness, derive `pow_key` from
    /// `(block_commit, nonce, params_tag, h_a, h_b)`, and set
    /// `found_leaf` to the resulting BLAKE3-keyed hash. This is what
    /// an honest miner produces and what `prove` / `verify`'s M10.1a
    /// binding check expects to hold.
    fn mvp_public_inputs(
        p: &ZkParams,
        w: &Witness,
        block_commit: &[u8],
        nonce: &[u8],
    ) -> PublicInputs {
        let mut pi = PublicInputs {
            params_tag: [7u8; 32],
            h_a: [11u8; 32],
            h_b: [13u8; 32],
            comm_m: [17u8; 32],
            found_i: 0,
            found_j: 0,
            found_leaf: [0u8; 32], // placeholder, overwritten below
        };
        let k = p.k as usize;
        let m_final = composite_air::MatmulTileAir::<2>::reference_final_state(
            &w.a_rows[..k],
            &w.b_cols[..k],
        );
        let pow_key = binding::derive_pow_key(block_commit, nonce, &pi);
        pi.found_leaf = binding::compute_found_leaf(m_final, &pow_key);
        pi
    }

    const BC: &[u8] = b"block-commit";
    const NONCE: &[u8] = b"nonce";

    #[test]
    fn prove_then_verify_round_trips() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");
        verify(BC, NONCE, &p, &pi, &proof).expect("verify must succeed");
    }

    #[test]
    fn proof_bytes_are_nonempty() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");
        assert!(!proof.0.is_empty(), "ZkProof must carry bytes");
    }

    #[test]
    fn proof_is_deterministic_per_inputs() {
        // The Tip5 challenger seeds from the same permutation each call,
        // and we don't randomize the trace. So back-to-back proves over
        // the same witness produce identical bytes — useful for fixture
        // tests downstream.
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof_a = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");
        let proof_b = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");
        assert_eq!(proof_a.0, proof_b.0);
    }

    /// M10 binding check: a proof produced under `pi_a` must NOT verify
    /// under a different `pi_b`. Plonky3 absorbs the public values into
    /// the Fiat-Shamir challenger; differing inputs change the FRI
    /// query points and the verifier rejects.
    #[test]
    fn verify_rejects_mismatched_public_inputs() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi_a = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi_a, &w).expect("prove must succeed");

        // Change one byte in one of the hash fields. This also
        // perturbs `pow_key` (h_a flows into the chain), so the
        // M10.1a hash check rejects too — either failure mode is fine.
        let mut pi_b = pi_a.clone();
        pi_b.h_a[0] ^= 0xFF;

        let r = verify(BC, NONCE, &p, &pi_b, &proof);
        assert!(r.is_err(), "verifier must reject mismatched PIs; got {r:?}");
    }

    /// Changing the public `found_i` / `found_j` indices must also
    /// invalidate a proof — full coverage across the 42-element PI
    /// vector.
    #[test]
    fn verify_rejects_changed_tile_indices() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi_a = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi_a, &w).expect("prove must succeed");

        let mut pi_b = pi_a.clone();
        pi_b.found_i = 1;

        let r = verify(BC, NONCE, &p, &pi_b, &proof);
        assert!(
            r.is_err(),
            "verifier must reject changed found_i; got {r:?}"
        );
    }

    /// The proof BYTES change when public inputs change. This is the
    /// prover-side counterpart of the verifier-side mismatch test —
    /// it confirms that PIs are actually flowing into the prover's
    /// Fiat-Shamir transcript.
    #[test]
    fn proof_bytes_differ_when_public_inputs_change() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        // Two consistent (pi, m_final) pairs by changing `params_tag`,
        // which flows through `pow_key` to `found_leaf` so both prove
        // calls succeed but their public-values vectors differ.
        let mut pi_a = mvp_public_inputs(&p, &w, BC, NONCE);
        pi_a.params_tag = [1u8; 32];
        let pow_a = binding::derive_pow_key(BC, NONCE, &pi_a);
        let m_final = composite_air::MatmulTileAir::<2>::reference_final_state(
            &w.a_rows[..p.k as usize],
            &w.b_cols[..p.k as usize],
        );
        pi_a.found_leaf = binding::compute_found_leaf(m_final, &pow_a);

        let mut pi_b = pi_a.clone();
        pi_b.params_tag = [2u8; 32];
        let pow_b = binding::derive_pow_key(BC, NONCE, &pi_b);
        pi_b.found_leaf = binding::compute_found_leaf(m_final, &pow_b);

        let proof_a = prove(BC, NONCE, &p, &pi_a, &w).expect("prove must succeed");
        let proof_b = prove(BC, NONCE, &p, &pi_b, &w).expect("prove must succeed");
        assert_ne!(proof_a.0, proof_b.0);
    }

    #[test]
    fn prove_rejects_invalid_params() {
        let mut p = mvp_params();
        let w_good = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w_good, BC, NONCE);
        p.tile = 0; // breaks tile divisibility
        let w = Witness {
            a_rows: vec![],
            b_cols: vec![],
            e_l: vec![],
            e_r_pos: vec![],
            f_r: vec![],
            f_l_pos: vec![],
            tile_states: vec![],
        };
        let r = prove(BC, NONCE, &p, &pi, &w);
        assert!(matches!(r, Err(ProveError::Params(_))), "got {r:?}");
    }

    #[test]
    fn prove_rejects_witness_shape_mismatch() {
        let p = mvp_params();
        let mut w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        w.a_rows.pop(); // length now wrong
        let r = prove(BC, NONCE, &p, &pi, &w);
        assert!(matches!(r, Err(ProveError::Witness(_))), "got {r:?}");
    }

    #[test]
    fn prove_rejects_unsupported_noise_rank() {
        let mut p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        p.noise_rank = 4; // outside MVP window
        let r = prove(BC, NONCE, &p, &pi, &w);
        assert!(matches!(r, Err(ProveError::Unsupported(_))), "got {r:?}");
    }

    #[test]
    fn verify_rejects_malformed_bytes() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        // 8 bytes of garbage — not a valid bincode-serialized envelope.
        let bad = ZkProof(vec![0xFFu8; 8]);
        let r = verify(BC, NONCE, &p, &pi, &bad);
        assert!(matches!(r, Err(VerifyError::Malformed(_))), "got {r:?}");
    }

    #[test]
    fn verify_rejects_tampered_proof() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let mut proof = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");
        // Flip a byte somewhere inside the bincode envelope. The
        // perturbation can hit the proof bytes (→ Plonky3 verifier
        // or bincode decode error), `m_final` (→ FoundLeafMismatch),
        // or the envelope framing (→ Malformed). Any non-Ok suffices.
        let idx = proof.0.len() / 2;
        proof.0[idx] ^= 0xFF;
        let r = verify(BC, NONCE, &p, &pi, &proof);
        assert!(r.is_err(), "tampered proof must not verify; got {r:?}");
    }

    #[test]
    fn verify_rejects_unsupported_noise_rank() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");
        // Now verify under different params that fail the MVP gate.
        let p_bad = ZkParams { noise_rank: 4, ..p };
        let r = verify(BC, NONCE, &p_bad, &pi, &proof);
        assert!(matches!(r, Err(VerifyError::Unsupported(_))), "got {r:?}");
    }

    // =====================================================================
    //  M10.1a found_leaf binding tests
    // =====================================================================

    /// Cooked-leaf attack: produce a proof, then verify with a PI that
    /// has a *fake* `found_leaf` (e.g. zeros, simulating an adversary
    /// claiming an easy-to-pass jackpot). The verifier must reject
    /// with the explicit `FoundLeafMismatch` variant — the
    /// cryptographic hash check rejects, independent of the FRI
    /// transcript.
    #[test]
    fn verify_rejects_cooked_found_leaf() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi_honest = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi_honest, &w).expect("prove must succeed");

        let mut pi_cooked = pi_honest.clone();
        pi_cooked.found_leaf = [0u8; 32]; // pretend the leaf is all zeros

        let r = verify(BC, NONCE, &p, &pi_cooked, &proof);
        assert!(
            matches!(r, Err(VerifyError::FoundLeafMismatch)),
            "cooked-leaf attack must reject with FoundLeafMismatch; got {r:?}"
        );
    }

    /// Prover-side sanity check: the caller asks us to prove a witness
    /// + public_inputs pair where `found_leaf` doesn't match what the
    /// witness actually produces. `prove` rejects up-front, saving
    /// the user a slow FRI run that would never verify anyway.
    #[test]
    fn prove_rejects_inconsistent_found_leaf() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let mut pi = mvp_public_inputs(&p, &w, BC, NONCE);
        pi.found_leaf[0] ^= 0xFF;
        let r = prove(BC, NONCE, &p, &pi, &w);
        assert!(
            matches!(r, Err(ProveError::FoundLeafMismatch)),
            "prove must reject mismatched (witness, found_leaf); got {r:?}"
        );
    }

    /// Tamper with `m_final` in the envelope without touching the
    /// inner Plonky3 proof. The verifier's hash check fails first
    /// (m_final no longer hashes to found_leaf), so we get
    /// `FoundLeafMismatch` — even though the inner proof bytes were
    /// never touched.
    #[test]
    fn verify_rejects_tampered_m_final_via_hash_check() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");

        // Decode, tamper m_final, re-encode.
        let (mut envelope, _): (ZkProofEnvelope, usize) =
            bincode::serde::decode_from_slice(&proof.0, bincode_standard()).unwrap();
        envelope.m_final = envelope.m_final.wrapping_add(1);
        let tampered_bytes = bincode::serde::encode_to_vec(&envelope, bincode_standard()).unwrap();

        let r = verify(BC, NONCE, &p, &pi, &ZkProof(tampered_bytes));
        assert!(
            matches!(r, Err(VerifyError::FoundLeafMismatch)),
            "tampered m_final must be caught by the hash check; got {r:?}"
        );
    }

    /// If an adversary tampers with `m_final` AND adjusts `found_leaf`
    /// to match the tampered hash, the verifier-side hash check now
    /// passes — but the AIR's last-row constraint fails because the
    /// trace's actual `m_out` no longer equals the public m_final
    /// slot. So Plonky3 rejects at the FRI / constraint layer.
    #[test]
    fn verify_rejects_tampered_m_final_via_air_constraint() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi_honest = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi_honest, &w).expect("prove must succeed");

        let (mut envelope, _): (ZkProofEnvelope, usize) =
            bincode::serde::decode_from_slice(&proof.0, bincode_standard()).unwrap();
        envelope.m_final = envelope.m_final.wrapping_add(1);
        let tampered_bytes = bincode::serde::encode_to_vec(&envelope, bincode_standard()).unwrap();

        // Adjust the public found_leaf so the hash check would pass.
        let pow_key = binding::derive_pow_key(BC, NONCE, &pi_honest);
        let mut pi_match = pi_honest.clone();
        pi_match.found_leaf = binding::compute_found_leaf(envelope.m_final, &pow_key);

        let r = verify(BC, NONCE, &p, &pi_match, &ZkProof(tampered_bytes));
        assert!(
            r.is_err(),
            "AIR / FRI must catch m_final mismatch even when hash check passes; got {r:?}"
        );
        // Specifically should NOT be FoundLeafMismatch (that's what
        // the adjusted PI was meant to dodge); should be Rejected
        // (Plonky3 verifier) or Malformed (bincode).
        assert!(
            !matches!(r, Err(VerifyError::FoundLeafMismatch)),
            "hash check should pass; AIR should reject. Got {r:?}"
        );
    }

    /// Honest-flow self-consistency: the (pi, m_final, witness) trio
    /// produced by `mvp_public_inputs` matches what the verifier
    /// computes. Catches accidental drift between the test fixture
    /// and the binding-helper implementations.
    #[test]
    fn mvp_public_inputs_are_self_consistent() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let pow_key = binding::derive_pow_key(BC, NONCE, &pi);
        let m_final = composite_air::MatmulTileAir::<2>::reference_final_state(
            &w.a_rows[..p.k as usize],
            &w.b_cols[..p.k as usize],
        );
        assert_eq!(
            pi.found_leaf,
            binding::compute_found_leaf(m_final, &pow_key)
        );
    }

    /// Pearl-style domain separation: the same witness mined with a
    /// different `block_commitment` produces a different `pow_key`
    /// (per `ai_pow::fiat_shamir`), which produces a different
    /// `found_leaf`. So a proof valid for one block doesn't verify
    /// for another block's PIs.
    #[test]
    fn verify_rejects_different_block_commitment() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, b"block-1", NONCE);
        let proof = prove(b"block-1", NONCE, &p, &pi, &w).expect("prove must succeed");

        // Replay against a different block_commit. pow_key shifts,
        // so the hash check rejects.
        let r = verify(b"block-2", NONCE, &p, &pi, &proof);
        assert!(
            matches!(r, Err(VerifyError::FoundLeafMismatch)),
            "replay across blocks must reject; got {r:?}"
        );
    }

    /// M10.1b in-circuit binding: tamper with the hash proof in the
    /// envelope while leaving the composite proof + M10.1a out-of-
    /// circuit hash check intact. The out-of-circuit path passes (it
    /// doesn't read `hash_proof_bytes`), so rejection must come from
    /// the in-circuit Blake3FoundLeafAir verifier — proves M10.1b's
    /// hash proof is actually being exercised, not silently skipped.
    #[test]
    fn verify_rejects_tampered_hash_proof_bytes() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, NONCE);
        let proof = prove(BC, NONCE, &p, &pi, &w).expect("prove must succeed");

        let (mut envelope, _): (ZkProofEnvelope, usize) =
            bincode::serde::decode_from_slice(&proof.0, bincode_standard()).unwrap();
        // Flip a byte in the middle of the hash proof. The composite
        // proof + m_final stay valid; only the hash leg is broken.
        let idx = envelope.hash_proof_bytes.len() / 2;
        envelope.hash_proof_bytes[idx] ^= 0xFF;
        let tampered_bytes = bincode::serde::encode_to_vec(&envelope, bincode_standard()).unwrap();

        let r = verify(BC, NONCE, &p, &pi, &ZkProof(tampered_bytes));
        assert!(
            r.is_err(),
            "tampered hash proof must reject through the M10.1b in-circuit binding; got {r:?}"
        );
    }

    /// Same as above but for `nonce` — nonces flow into `pow_key`
    /// (Pearl's per-nonce key derivation).
    #[test]
    fn verify_rejects_different_nonce() {
        let p = mvp_params();
        let w = mvp_witness(&p);
        let pi = mvp_public_inputs(&p, &w, BC, b"nonce-A");
        let proof = prove(BC, b"nonce-A", &p, &pi, &w).expect("prove must succeed");

        let r = verify(BC, b"nonce-B", &p, &pi, &proof);
        assert!(
            matches!(r, Err(VerifyError::FoundLeafMismatch)),
            "verifier with different nonce must reject; got {r:?}"
        );
    }
}
