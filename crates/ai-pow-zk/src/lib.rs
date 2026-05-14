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
//! The current entrypoints prove a **single tile-cell matmul** end-to-
//! end through the [`circuit::AiPowStarkConfig`] STARK stack:
//!
//!   * The AIR is [`matmul_chip::MatmulCellAir<R>`] (M6) — encodes the
//!     per-stripe `r`-wide INT8 dot-product accumulator for one `(i, j)`
//!     cell.
//!   * The trace is generated from `witness.a_rows[0..k]` and
//!     `witness.b_cols[0..k]`, i.e. the first tile-row of `A'` and the
//!     first tile-column of `B'`.
//!   * `block_commitment`, `nonce`, and the bulk of `public_inputs` are
//!     **not yet bound into the proof**. The proof attests that "*some*
//!     INT8 dot-product accumulator was computed correctly for those
//!     rows", not that the inputs match Pearl's `h_a`, `h_b`, `comm_m`,
//!     or `found_leaf`.
//!
//! What the building-block chips already exist for, but are **not**
//! composed into the prove/verify entries yet:
//!
//!   * [`input_chip::RangeAir`] (M5) — range checks for the i8 / u13 /
//!     i32 witness types.
//!   * [`state_chip::StateChipAir`] (M7) — Pearl §4.5 rotate-XOR-13
//!     state update.
//!   * [`blake3_air`] (M8) — keyed BLAKE3 sub-AIR for the public-input
//!     boundary.
//!
//! Composing the four chips into one AIR (with cross-chip linkages
//! `x = c_out`, `keyed_hash(M) = found_leaf`, etc.) is the next
//! milestone. The current entry points exist so callers in `ai-pow`
//! can wire the call sites now and the implementation fills in
//! independently.

pub mod air;
pub mod blake3_air;
pub mod circuit;
pub mod input_chip;
pub mod matmul_chip;
pub mod params;
pub mod public;
pub mod state_chip;
pub mod witness;

use bincode::config::standard as bincode_standard;
pub use p3_goldilocks::Goldilocks as Val;
use p3_uni_stark::Proof as UniStarkProof;
use thiserror::Error;

pub use crate::air::MatmulAir;
pub use crate::circuit::{AiPowStarkConfig, CircuitConfig};
pub use crate::params::ZkParams;
pub use crate::public::PublicInputs;
pub use crate::witness::Witness;

/// Bincode-serialized Plonky3 STARK proof. Wire format is internal to
/// this crate — consumers persist the `Vec<u8>` verbatim and round-trip
/// it through [`verify`] only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZkProof(pub Vec<u8>);

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
}

/// Build a Plonky3 STARK that attests to the existence of a [`Witness`]
/// producing [`PublicInputs`] for the given `(block_commitment, nonce,
/// params)`.
///
/// **Current scope.** See the [`crate`]-level docs. The proof attests
/// only that the dot-product accumulator for the `(0, 0)` tile cell of
/// `witness.a_rows` / `witness.b_cols` is computed correctly under the
/// AIR transition semantics.
pub fn prove(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &ZkParams,
    public_inputs: &PublicInputs,
    witness: &Witness,
) -> Result<ZkProof, ProveError> {
    let _ = (block_commitment, nonce);

    params.validate().map_err(ProveError::Params)?;
    validate_public_inputs(public_inputs).map_err(ProveError::PublicInputs)?;
    validate_witness_shape(witness, params).map_err(ProveError::Witness)?;
    require_mvp_params(params).map_err(ProveError::Unsupported)?;

    let k = params.k as usize;
    let a = &witness.a_rows[0..k];
    let b = &witness.b_cols[0..k];
    let cfg = circuit::build_stark_config(params, &CircuitConfig::TEST);
    let air = matmul_chip::MatmulCellAir::<2>::new();
    let trace = matmul_chip::MatmulCellAir::<2>::generate_trace(a, b);
    let proof = p3_uni_stark::prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
    let bytes = bincode::serde::encode_to_vec(&proof, bincode_standard())
        .map_err(|e| ProveError::Serialize(e.to_string()))?;
    Ok(ZkProof(bytes))
}

/// Verify a [`ZkProof`] against a set of [`PublicInputs`] extracted
/// from the chain. Mirrors Pearl's `ZKProof::verify`.
pub fn verify(
    block_commitment: &[u8],
    nonce: &[u8],
    params: &ZkParams,
    public_inputs: &PublicInputs,
    proof: &ZkProof,
) -> Result<(), VerifyError> {
    let _ = (block_commitment, nonce);

    params.validate().map_err(VerifyError::Params)?;
    validate_public_inputs(public_inputs).map_err(VerifyError::PublicInputs)?;
    require_mvp_params(params).map_err(VerifyError::Unsupported)?;

    let (decoded, _used): (UniStarkProof<AiPowStarkConfig>, usize) =
        bincode::serde::decode_from_slice(&proof.0, bincode_standard())
            .map_err(|e| VerifyError::Malformed(e.to_string()))?;

    let cfg = circuit::build_stark_config(params, &CircuitConfig::TEST);
    let air = matmul_chip::MatmulCellAir::<2>::new();
    p3_uni_stark::verify::<AiPowStarkConfig, _>(&cfg, &air, &decoded, &[])
        .map_err(|e| VerifyError::Rejected(format!("{e:?}")))
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
/// The matmul-cell AIR is parameterised by a const-generic `STRIPE`,
/// which we currently fix at `2`. Supporting `noise_rank ∈ {4, 8, …}`
/// is a follow-on; today, `params.noise_rank` must equal `2`.
fn require_mvp_params(p: &ZkParams) -> Result<(), String> {
    if p.noise_rank != 2 {
        return Err(format!(
            "MVP entry supports noise_rank = 2 only (got {}); compose other \
             STRIPE values via the chip directly until M9.1",
            p.noise_rank
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

    fn mvp_public_inputs() -> PublicInputs {
        PublicInputs {
            params_tag: [7u8; 32],
            h_a: [11u8; 32],
            h_b: [13u8; 32],
            comm_m: [17u8; 32],
            found_i: 0,
            found_j: 0,
            found_leaf: [19u8; 32],
        }
    }

    #[test]
    fn prove_then_verify_round_trips() {
        let p = mvp_params();
        let pi = mvp_public_inputs();
        let w = mvp_witness(&p);
        let proof = prove(b"block-commit", b"nonce", &p, &pi, &w).expect("prove must succeed");
        verify(b"block-commit", b"nonce", &p, &pi, &proof).expect("verify must succeed");
    }

    #[test]
    fn proof_bytes_are_nonempty() {
        let p = mvp_params();
        let pi = mvp_public_inputs();
        let w = mvp_witness(&p);
        let proof = prove(b"x", b"y", &p, &pi, &w).expect("prove must succeed");
        assert!(!proof.0.is_empty(), "ZkProof must carry bytes");
    }

    #[test]
    fn proof_is_deterministic_per_inputs() {
        // The Tip5 challenger seeds from the same permutation each call,
        // and we don't randomize the trace. So back-to-back proves over
        // the same witness produce identical bytes — useful for fixture
        // tests downstream.
        let p = mvp_params();
        let pi = mvp_public_inputs();
        let w = mvp_witness(&p);
        let proof_a = prove(b"x", b"y", &p, &pi, &w).expect("prove must succeed");
        let proof_b = prove(b"x", b"y", &p, &pi, &w).expect("prove must succeed");
        assert_eq!(proof_a.0, proof_b.0);
    }

    #[test]
    fn prove_rejects_invalid_params() {
        let mut p = mvp_params();
        p.tile = 0; // breaks tile divisibility
        let pi = mvp_public_inputs();
        let w = Witness {
            a_rows: vec![],
            b_cols: vec![],
            e_l: vec![],
            e_r_pos: vec![],
            f_r: vec![],
            f_l_pos: vec![],
            tile_states: vec![],
        };
        let r = prove(b"x", b"y", &p, &pi, &w);
        assert!(matches!(r, Err(ProveError::Params(_))), "got {r:?}");
    }

    #[test]
    fn prove_rejects_witness_shape_mismatch() {
        let p = mvp_params();
        let pi = mvp_public_inputs();
        let mut w = mvp_witness(&p);
        w.a_rows.pop(); // length now wrong
        let r = prove(b"x", b"y", &p, &pi, &w);
        assert!(matches!(r, Err(ProveError::Witness(_))), "got {r:?}");
    }

    #[test]
    fn prove_rejects_unsupported_noise_rank() {
        let mut p = mvp_params();
        p.noise_rank = 4; // outside MVP window
        let pi = mvp_public_inputs();
        let w = mvp_witness(&p);
        let r = prove(b"x", b"y", &p, &pi, &w);
        assert!(matches!(r, Err(ProveError::Unsupported(_))), "got {r:?}");
    }

    #[test]
    fn verify_rejects_malformed_bytes() {
        let p = mvp_params();
        let pi = mvp_public_inputs();
        // 8 bytes of garbage — not a valid bincode-serialized Proof.
        let bad = ZkProof(vec![0xFFu8; 8]);
        let r = verify(b"x", b"y", &p, &pi, &bad);
        assert!(matches!(r, Err(VerifyError::Malformed(_))), "got {r:?}");
    }

    #[test]
    fn verify_rejects_tampered_proof() {
        let p = mvp_params();
        let pi = mvp_public_inputs();
        let w = mvp_witness(&p);
        let mut proof = prove(b"x", b"y", &p, &pi, &w).expect("prove must succeed");
        // Flip a byte in the middle of the bincode blob. Likely either
        // deserialization fails (Malformed) or the Plonky3 verifier
        // rejects (Rejected). Either is fine — we just want non-Ok.
        let idx = proof.0.len() / 2;
        proof.0[idx] ^= 0xFF;
        let r = verify(b"x", b"y", &p, &pi, &proof);
        assert!(r.is_err(), "tampered proof must not verify; got {r:?}");
    }

    #[test]
    fn verify_rejects_unsupported_noise_rank() {
        let p = mvp_params();
        let pi = mvp_public_inputs();
        let w = mvp_witness(&p);
        let proof = prove(b"x", b"y", &p, &pi, &w).expect("prove must succeed");
        // Now verify under different params that fail the MVP gate.
        let p_bad = ZkParams { noise_rank: 4, ..p };
        let r = verify(b"x", b"y", &p_bad, &pi, &proof);
        assert!(matches!(r, Err(VerifyError::Unsupported(_))), "got {r:?}");
    }
}
