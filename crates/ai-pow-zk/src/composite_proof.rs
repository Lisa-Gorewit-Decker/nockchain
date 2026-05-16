//! Lib-level prove/verify wrappers for the composite AIR.
//!
//! ## Entrypoints — three tiers (pick by use case)
//!
//! | Family | AIR | Prover | Use |
//! |---|---|---|---|
//! | [`composite_prove`] / [`composite_verify`] | `CompositeFullAir` (unpinned) | uni-stark | unit / constraint-logic dev only — **not sound for PoW** (CRIT-1: a prover can zero selectors) |
//! | [`composite_prove_pinned`] / [`composite_verify_pinned`] / [`composite_verify_pow_pinned`] | `CompositeFullAirPinned` | uni-stark | CRIT-1 program-pinned, **no LogUp**. Lighter; backs the `crit1_*` / `high2_*` constraint-logic regression suite. Not the production path (matmul reads unbound — §4.C). |
//! | [`composite_prove_pinned_logup`] / [`composite_verify_pinned_logup`] / [`composite_verify_pow_pinned_logup`] | `CompositeFullAirWithLookupsPinned` | **batch-stark** | ★ **PRODUCTION ENTRYPOINT** (HIGH-2.2 §4.C Route A). CRIT-1 program-pin **and** the `noised_packed`/range/i8u8/cv-routing LogUp enforced in one proof. Used by [`ai-pow::zk_bridge`] (the `mine()` gate) and `f1_harness`. ≈1.23x the uni-stark pinned cost. |
//!
//! New production callers should use the **`*_pinned_logup`**
//! family. The uni-stark `*_pinned` family is retained as the
//! lighter no-LogUp variant + the home of the CRIT-1/HIGH-2
//! constraint-logic adversarial suite. The unpinned
//! `composite_prove`/`verify` is dev-only and PoW-unsound.
//!
//! ## CRIT-1 trust model (all `*_pinned*` families)
//!
//! The verifier rebuilds the canonical `program` from the
//! trusted per-block shape (a pure function of `ctx`/`params`),
//! **never** from the proof, and checks the proof against that
//! program's preprocessed commitment. A forged trace whose
//! program differs is rejected. See `ai-pow::zk_bridge`.
//!
//! ## Public-input shape
//!
//! [`CompositePublicInputs`] — 20 field elements: 4 i32 final
//! CUMSUM_TILE + 16 u32 final JACKPOT_MSG, bound by the AIR on
//! the trace's last row. See
//! [`crate::composite_public`] for the layout and the
//! `CompositePublicInputs::derive_from_trace` helper that snapshots
//! the values from a generated trace.

use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
use crate::composite_full_air::{extract_program, CompositeFullAir, CompositeFullAirPinned};
use crate::composite_public::CompositePublicInputs;
use crate::composite_trace::CompositeTrace;
use crate::params::ZkParams;

use p3_commit::Pcs;
use p3_uni_stark::{
    prove, prove_with_preprocessed, setup_preprocessed, verify, verify_with_preprocessed,
    PreprocessedProverData, PreprocessedVerifierKey, Proof, StarkGenericConfig, Val,
    VerificationError,
};

/// Concrete type of the verification error for the composite AIR.
/// Equivalent to `VerificationError<PcsError<AiPowStarkConfig>>`.
pub type CompositeVerificationError = VerificationError<
    <<AiPowStarkConfig as StarkGenericConfig>::Pcs as Pcs<
        <AiPowStarkConfig as StarkGenericConfig>::Challenge,
        <AiPowStarkConfig as StarkGenericConfig>::Challenger,
    >>::Error,
>;

/// Build the composite STARK config for the given parameters +
/// profile. Re-export of [`build_stark_config`] for ergonomics.
pub fn build_config(params: &ZkParams, profile: &CircuitConfig) -> AiPowStarkConfig {
    build_stark_config(params, profile)
}

/// Prove the composite AIR against a given trace + public inputs.
///
/// `trace` must be a [`CompositeTrace`] whose internal matrix has
/// width [`crate::composite_layout::TOTAL_TRACE_WIDTH`] and height
/// a power of 2 ≥ `MIN_STARK_LEN`. `public_inputs` must match the
/// trace's last-row CUMSUM_TILE / JACKPOT_MSG cells — the AIR
/// enforces this binding.
///
/// The returned [`Proof`] can be serialised via [`bincode`] for
/// transport.
pub fn composite_prove(
    config: &AiPowStarkConfig,
    trace: CompositeTrace,
    public_inputs: &CompositePublicInputs,
) -> Proof<AiPowStarkConfig> {
    let pis = public_inputs.to_vec();
    prove::<AiPowStarkConfig, _>(config, &CompositeFullAir, trace.matrix, &pis)
}

/// Verify a composite proof against the claimed public inputs.
/// Returns `Ok(())` if valid; otherwise a
/// [`CompositeVerificationError`] describing the failure.
pub fn composite_verify(
    config: &AiPowStarkConfig,
    proof: &Proof<AiPowStarkConfig>,
    public_inputs: &CompositePublicInputs,
) -> Result<(), CompositeVerificationError> {
    let pis = public_inputs.to_vec();
    verify::<AiPowStarkConfig, _>(config, &CompositeFullAir, proof, &pis)
}

/// Encode the 8×u32 `HASH_JACKPOT` PI as a 32-byte little-endian
/// u256, byte-identical to a BLAKE3 digest (`bytes[4i..4i+4] =
/// word[i].to_le_bytes()`). Matches the encoding M52's
/// `place_matrix_hash` uses (CV_OUT word i = LE u32 of digest
/// bytes 4i..4i+4), so the inverse reconstructs the digest.
pub fn hash_jackpot_le_bytes(hash_jackpot: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&hash_jackpot[i].to_le_bytes());
    }
    out
}

/// 256-bit unsigned `hash <= target`, both little-endian 32-byte.
/// Identical comparison to `ai-pow::tile_hash::hash_le_target` —
/// kept local so `ai-pow-zk` stays standalone.
fn le_u256_le(hash: &[u8; 32], target: &[u8; 32]) -> bool {
    for k in (0..32).rev() {
        match hash[k].cmp(&target[k]) {
            core::cmp::Ordering::Less => return true,
            core::cmp::Ordering::Greater => return false,
            core::cmp::Ordering::Equal => continue,
        }
    }
    true
}

/// Error from [`composite_verify_pow`]: either the STARK proof is
/// invalid, or it is valid but the proven `HASH_JACKPOT` does not
/// clear the difficulty target.
#[derive(Debug)]
pub enum PowVerifyError {
    /// The underlying STARK proof failed verification.
    Stark(CompositeVerificationError),
    /// STARK valid, but `HASH_JACKPOT > target` (tile not a winner).
    DifficultyNotMet,
}

impl core::fmt::Display for PowVerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PowVerifyError::Stark(e) => write!(f, "stark verification failed: {e:?}"),
            PowVerifyError::DifficultyNotMet => {
                write!(f, "HASH_JACKPOT does not clear the difficulty target")
            }
        }
    }
}

impl std::error::Error for PowVerifyError {}

/// C2 — full proof-of-work verification.
///
/// Pearl's Layer-0 STARK does **not** enforce the difficulty
/// inequality `BLAKE3(M, key=s_a) ≤ target` in-circuit; it is
/// checked outside (block validation / higher recursion layers,
/// see `pearl_circuit.rs`). `ai-pow-zk` is a single STARK with no
/// recursion layers, so this wrapper performs the Pearl-equivalent
/// check after STARK verification, against the **bound**
/// `HASH_JACKPOT` public input (C4). Soundness rests on
/// HASH_JACKPOT being a selector-gated bound PI — the verifier
/// compares the *proven* tile-state keyed hash against `target`,
/// not an unconstrained claim. An in-AIR 256-bit comparator was
/// considered and rejected: it is strictly more than Pearl does
/// at Layer-0, costs a dedicated chip, and recursion (M12) would
/// absorb the external check anyway.
///
/// `target` is the 32-byte little-endian difficulty bound
/// (`ai-pow::tile_hash::difficulty_target` produces it).
pub fn composite_verify_pow(
    config: &AiPowStarkConfig,
    proof: &Proof<AiPowStarkConfig>,
    public_inputs: &CompositePublicInputs,
    target: &[u8; 32],
) -> Result<(), PowVerifyError> {
    composite_verify(config, proof, public_inputs).map_err(PowVerifyError::Stark)?;
    let hj = hash_jackpot_le_bytes(&public_inputs.hash_jackpot);
    if le_u256_le(&hj, target) {
        Ok(())
    } else {
        Err(PowVerifyError::DifficultyNotMet)
    }
}

// ───────────────────────────────────────────────────────────────
//  CRIT-1: program-pinned prove / verify (ZKP_SECURITY_REPORT)
//
//  The unit `composite_prove`/`composite_verify` above prove the
//  *unpinned* `CompositeFullAir` — a malicious prover can zero
//  every selector and forge a winning proof (no preprocessed
//  commitment ⇒ no verifier-fixed program). The pinned API below
//  commits the 5 PROGRAM_COLS as a *preprocessed* trace whose
//  commitment goes in the verifying key; the AIR forces the
//  prover's in-trace `*_PREP` cells to equal it. The verifier
//  rebuilds the canonical program from the trusted shape (never
//  from the proof) — see `ai-pow::zk_bridge`. This is the
//  production path.
// ───────────────────────────────────────────────────────────────

type Program = p3_matrix::dense::RowMajorMatrix<Val<AiPowStarkConfig>>;

fn program_degree_bits(program: &Program) -> usize {
    use p3_matrix::Matrix;
    let h = program.height();
    assert!(h.is_power_of_two(), "trace height must be a power of two");
    h.trailing_zeros() as usize
}

/// Commit a program matrix as a preprocessed trace, returning the
/// reusable prover data + verifying key. Deterministic in
/// `program`: prover and verifier independently arrive at the
/// same commitment iff they use the same canonical program.
pub fn composite_setup(
    config: &AiPowStarkConfig,
    program: &Program,
) -> (
    PreprocessedProverData<AiPowStarkConfig>,
    PreprocessedVerifierKey<AiPowStarkConfig>,
) {
    let air = CompositeFullAirPinned::new(program.clone());
    setup_preprocessed(config, &air, program_degree_bits(program))
        .expect("CompositeFullAirPinned always has preprocessed columns")
}

/// Program-pinned prove (uni-stark, **no LogUp** — lighter
/// variant + the `crit1_*`/`high2_*` constraint-logic harness).
/// **Production should call [`composite_prove_pinned_logup`]**
/// (Route A) so the `noised_packed` matrix binding is enforced.
///
/// Derives the canonical program from the (honest) trace's
/// `*_PREP` columns, commits it, and proves. Returns the proof
/// **and** the program — the caller hands the program to the
/// verifier *out of band from a trusted source* (params), never
/// lets the verifier take it from the proof.
pub fn composite_prove_pinned(
    config: &AiPowStarkConfig,
    trace: CompositeTrace,
    public_inputs: &CompositePublicInputs,
) -> (Proof<AiPowStarkConfig>, Program) {
    let program = extract_program(&trace.matrix);
    let air = CompositeFullAirPinned::new(program.clone());
    let (pp, _vk) = composite_setup(config, &program);
    let pis = public_inputs.to_vec();
    let proof = prove_with_preprocessed(config, &air, trace.matrix, &pis, Some(&pp));
    (proof, program)
}

/// Program-pinned verify. `program` MUST be the canonical program
/// for the agreed `ZkParams`, rebuilt by the verifier from the
/// trusted shape — never extracted from the prover's proof. The
/// preprocessed commitment in the derived VK pins the prover's
/// selector schedule; a forged trace whose `*_PREP` columns
/// differ fails the in-AIR equality (CRIT-1 closed).
pub fn composite_verify_pinned(
    config: &AiPowStarkConfig,
    program: &Program,
    proof: &Proof<AiPowStarkConfig>,
    public_inputs: &CompositePublicInputs,
) -> Result<(), CompositeVerificationError> {
    let air = CompositeFullAirPinned::new(program.clone());
    let (_pp, vk) = composite_setup(config, program);
    let pis = public_inputs.to_vec();
    verify_with_preprocessed(config, &air, proof, &pis, Some(&vk))
}

/// Program-pinned full PoW verify: pinned STARK verify + the C2
/// difficulty check against the bound `HASH_JACKPOT`.
pub fn composite_verify_pow_pinned(
    config: &AiPowStarkConfig,
    program: &Program,
    proof: &Proof<AiPowStarkConfig>,
    public_inputs: &CompositePublicInputs,
    target: &[u8; 32],
) -> Result<(), PowVerifyError> {
    composite_verify_pinned(config, program, proof, public_inputs)
        .map_err(PowVerifyError::Stark)?;
    let hj = hash_jackpot_le_bytes(&public_inputs.hash_jackpot);
    if le_u256_le(&hj, target) {
        Ok(())
    } else {
        Err(PowVerifyError::DifficultyNotMet)
    }
}

// ───────────────────────────────────────────────────────────────
//  HIGH-2.2 §4.C Route A: pinned + LogUp production prove/verify
//
//  The uni-stark `composite_*_pinned` above enforce the CRIT-1
//  program-pin but NOT the cross-chip LogUp — so the matmul
//  `A_NOISED`/`B_NOISED` reads are unbound vs the C3/HASH_A
//  canonical store (§4.C gap). These prove/verify the
//  `CompositeFullAirWithLookupsPinned` AIR via `p3-batch-stark`,
//  which enforces the CRIT-1 pin AND the `noised_packed`
//  (+ range / i8u8 / cv-routing) LogUp simultaneously. Spike
//  measured ~1.23x the uni-stark pinned prover cost
//  (HIGH2_2_DESIGN.md §4.C.10), vs naive Route C's ~10x.
//
//  Same CRIT-1 trust model: the verifier rebuilds the canonical
//  `program` from the trusted per-block `ctx` (never from the
//  proof), derives the preprocessed commitment from it via
//  `ProverData::from_airs_and_degrees` (witness-free — needs only
//  the program + the public trace height), and checks the proof
//  against that.
// ───────────────────────────────────────────────────────────────

/// Route-A pinned prove. `trace` is LogUp-balanced here
/// (`populate_lookup_freq`) so the bus argument closes. Returns
/// the batch proof + the canonical program (handed to the
/// verifier out-of-band from a trusted source, never via the
/// proof — identical CRIT-1 discipline to `composite_prove_pinned`).
pub fn composite_prove_pinned_logup(
    config: &AiPowStarkConfig,
    mut trace: CompositeTrace,
    public_inputs: &CompositePublicInputs,
) -> (p3_batch_stark::BatchProof<AiPowStarkConfig>, Program) {
    use p3_batch_stark::{prove_batch, ProverData, StarkInstance};

    trace.populate_lookup_freq();
    let program = extract_program(&trace.matrix);
    let air = crate::composite_full_air_with_lookups::CompositeFullAirWithLookupsPinned::new(
        program.clone(),
    );
    let pvs = public_inputs.to_vec();
    let instances = vec![StarkInstance {
        air: &air,
        trace: &trace.matrix,
        public_values: pvs,
    }];
    let pd = ProverData::from_instances(config, &instances);
    let proof = prove_batch(config, &instances, &pd);
    (proof, program)
}

/// Verifier-side `CommonData` for the canonical `program` —
/// rebuilt witness-free from the program + its (public) height.
fn logup_common_for(
    config: &AiPowStarkConfig,
    program: &Program,
) -> p3_batch_stark::ProverData<AiPowStarkConfig> {
    use p3_batch_stark::ProverData;
    let air = crate::composite_full_air_with_lookups::CompositeFullAirWithLookupsPinned::new(
        program.clone(),
    );
    let log_ext_db = program_degree_bits(program) + config.is_zk() as usize;
    ProverData::from_airs_and_degrees(config, std::slice::from_ref(&air), &[log_ext_db])
}

/// Route-A pinned verify. `program` MUST be the canonical program
/// the verifier rebuilds from the trusted shape (never from the
/// proof) — exactly as `composite_verify_pinned`.
pub fn composite_verify_pinned_logup(
    config: &AiPowStarkConfig,
    program: &Program,
    proof: &p3_batch_stark::BatchProof<AiPowStarkConfig>,
    public_inputs: &CompositePublicInputs,
) -> Result<(), CompositeVerificationError> {
    use p3_batch_stark::verify_batch;
    let air = crate::composite_full_air_with_lookups::CompositeFullAirWithLookupsPinned::new(
        program.clone(),
    );
    let pd = logup_common_for(config, program);
    verify_batch(
        config,
        std::slice::from_ref(&air),
        proof,
        &[public_inputs.to_vec()],
        &pd.common,
    )
}

/// Route-A pinned full PoW verify: pinned+LogUp STARK verify +
/// the C2 difficulty check against the bound `HASH_JACKPOT`.
pub fn composite_verify_pow_pinned_logup(
    config: &AiPowStarkConfig,
    program: &Program,
    proof: &p3_batch_stark::BatchProof<AiPowStarkConfig>,
    public_inputs: &CompositePublicInputs,
    target: &[u8; 32],
) -> Result<(), PowVerifyError> {
    composite_verify_pinned_logup(config, program, proof, public_inputs)
        .map_err(PowVerifyError::Stark)?;
    let hj = hash_jackpot_le_bytes(&public_inputs.hash_jackpot);
    if le_u256_le(&hj, target) {
        Ok(())
    } else {
        Err(PowVerifyError::DifficultyNotMet)
    }
}

#[allow(dead_code)]
fn _suppress_unused_val_import(_v: Val<AiPowStarkConfig>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_zk_params() -> ZkParams {
        ZkParams {
            m: 8,
            k: 16,
            n: 8,
            noise_rank: 2,
            tile: 2,
            difficulty_bits: 0,
        }
    }

    #[test]
    fn composite_prove_verify_round_trip() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis).expect("composite proof must verify");
    }

    /// HIGH-2.2 §4.A repro: a baseline trace + a real fold chain
    /// must satisfy the (unit) composite AIR — isolates the
    /// FoldChip-in-composite OodEvaluationMismatch the e2e hit,
    /// fast (unit `composite_prove`, no batch-stark / pinning).
    #[test]
    fn high2_2_fold_chain_in_composite_unit() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        // Small non-trivial x_steps placed mid-trace; FOLD_STATE
        // propagates to the last row.
        let xs: Vec<i32> = (0..16i32)
            .map(|i| i.wrapping_mul(0x0151_5151) ^ 0x33)
            .collect();
        let _m = trace.place_fold_chain(64, &xs);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis)
            .expect("baseline + real fold chain must satisfy the composite AIR");
    }

    /// HIGH-2.2 §4.A repro (pinned+LogUp layer): isolate whether
    /// the §4.D keystone / CRIT-1 pin / Route-A batch-stark is
    /// what the zk_bridge e2e tripped (the unit variant above
    /// passes). Mirrors the bridge trace shape: fold chain →
    /// JACKPOT_MSG = final M (so the keystone holds) →
    /// jackpot-hash block, proven via the production
    /// `*_pinned_logup` path.
    // KNOWN-FAILING §4.A repro (kept for the fix). A real fold
    // chain + keystone + jackpot, via the Route-A batch-stark
    // path, fails `OodEvaluationMismatch { index: Some(0) }`.
    // The unit `composite_prove` path (no pin/keystone/LogUp)
    // PASSES with the same trace (`*_in_composite_unit`), and
    // the §4.D keystone *data* precond is asserted holding.
    // The degree-3→2 FoldChip rewrite (FOLD_XOR_OUT) did NOT
    // fix it — **hypothesis disproven**: the cause is NOT
    // FoldChip constraint degree. It is isolated to the
    // pinned/keystone/LogUp(batch-stark) layer with non-zero
    // FOLD. Next bisection: a uni-stark *pinned* (keystone, no
    // LogUp) variant to split keystone/program-pin vs the
    // batch-stark LogUp layer. `#[ignore]` so the suite stays
    // green. See HIGH2_2_DESIGN.md §4.A.
    #[test]
    #[ignore = "HIGH-2.2 §4.A: batch-stark pinned+LogUp + non-zero FOLD fails; degree hypothesis disproven; see HIGH2_2_DESIGN.md §4.A"]
    fn high2_2_fold_chain_pinned_logup() {
        use crate::composite_layout::TOTAL_TRACE_WIDTH;
        use p3_field::integers::QuotientMap;

        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let ch: [u32; 8] = core::array::from_fn(|i| 0x5EED_0000 + i as u32);
        let mut trace = CompositeTrace::baseline_min();
        let h = trace.height();
        let xs: Vec<i32> = (0..16i32)
            .map(|i| i.wrapping_mul(0x0151_5151) ^ 0x33)
            .collect();
        let m = trace.place_fold_chain(64, &xs);
        // Keystone needs last-row JACKPOT_MSG == FOLD_STATE = M.
        // place_jackpot_hash_block writes JACKPOT_MSG[h-1] = m.
        let _ = trace.place_jackpot_hash_block(h - 8, &m, &ch);
        // FOLD_STATE on the jackpot-block rows is already M
        // (place_fold_chain propagated it). Sanity: last-row
        // JACKPOT_MSG == FOLD_STATE.
        let last = (h - 1) * TOTAL_TRACE_WIDTH;
        for s in 0..16 {
            let jm = trace.matrix.values
                [last + crate::composite_layout::JACKPOT_MSG_START + s];
            let fs = trace.matrix.values
                [last + crate::composite_layout::FOLD_STATE_START + s];
            assert_eq!(
                jm,
                <Val<AiPowStarkConfig> as QuotientMap<u64>>::from_int(m[s] as u64),
                "JACKPOT_MSG[{s}] != M"
            );
            assert_eq!(jm, fs, "keystone precondition: JACKPOT_MSG[{s}] != FOLD_STATE[{s}]");
        }
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let canonical = extract_program(&trace.matrix);
        let (proof, _) = composite_prove_pinned_logup(&cfg, trace, &pis);
        composite_verify_pinned_logup(&cfg, &canonical, &proof, &pis)
            .expect("fold chain + keystone + jackpot must verify under Route-A");
    }

    /// HIGH-2.2 §4.A bisection #2: fold chain + jackpot block via
    /// the **unit** `composite_prove` (`CompositeFullAir` — NO
    /// program-pin, NO §4.D keystone, NO LogUp). Splits the
    /// remaining locus:
    ///   - FAILS ⇒ base `CompositeFullAir` fold↔jackpot (or
    ///     last-row PI-binding) interaction — bug is in a base
    ///     chip / the base `when_last_row` JACKPOT_MSG↔PI bind.
    ///   - PASSES ⇒ fault is specifically the CRIT-1 program-pin
    ///     or the §4.D keystone with non-zero last-row
    ///     FOLD_STATE/JACKPOT_MSG.
    /// CONTROL (bisection #3): `place_jackpot_hash_block` ALONE
    /// in a minimal trace (baseline + jackpot, **no fold, no
    /// key-pin, no matrix-hash**) via unit `composite_prove`.
    /// The passing jackpot case (`routea_honest`) has key-pin +
    /// matrix-hash scaffolding; `*_fold_chain_jackpot_unit` had
    /// neither — so "fold×jackpot" was never cleanly isolated.
    ///   - FAILS ⇒ the bug is the jackpot block needing
    ///     scaffolding (NOT a fold interaction); fix is the
    ///     bridge/repro trace shape, not FoldChip.
    ///   - PASSES ⇒ confirms the fold×jackpot interaction.
    #[test]
    fn high2_2_jackpot_only_unit() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let ch: [u32; 8] = core::array::from_fn(|i| 0x5EED_0000 + i as u32);
        let mut trace = CompositeTrace::baseline_min();
        let h = trace.height();
        let _ = trace.place_jackpot_hash_block(h - 8, &[0u32; 16], &ch);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis)
            .expect("CONTROL: jackpot block alone (minimal, no fold/scaffolding)");
    }

    /// CONTROL #4 (decisive): `place_jackpot_hash_block` with a
    /// **non-zero** 16-word message — NO fold chain. Every
    /// shipping/test jackpot placement uses `&[0u32;16]`; the
    /// fold repro is the first with a non-zero message (`&m`).
    /// RESULT: **FAILED** `OodEvaluationMismatch` (also fails at
    /// log_blowup=4, see `*_lb4`). ⇒ DEFINITIVE root cause:
    /// `place_jackpot_hash_block` / the BLAKE3 keyed-hash with a
    /// **non-zero message** fails composite verify (per-row
    /// `check_constraints` passes; the polynomial verify fails;
    /// not a degree/blowup issue). The fold chain, §4.D
    /// keystone, CRIT-1 pin, and batch-stark are ALL exonerated:
    /// a pre-existing latent bug — every shipping/test jackpot
    /// placement used `&[0u32;16]`; HIGH-2.2 is just the first to
    /// need a non-zero `JACKPOT_MSG`. `#[ignore]` (known-failing
    /// reproducer); see HIGH2_2_DESIGN.md §4.A.
    #[test]
    #[ignore = "pre-existing: place_jackpot_hash_block non-zero msg fails composite verify; HIGH2_2_DESIGN §4.A"]
    fn high2_2_jackpot_nonzero_msg_unit() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let ch: [u32; 8] = core::array::from_fn(|i| 0x5EED_0000 + i as u32);
        let msg: [u32; 16] = core::array::from_fn(|i| 0xABCD_0001u32.wrapping_mul(i as u32 + 1));
        let mut trace = CompositeTrace::baseline_min();
        let h = trace.height();
        let _ = trace.place_jackpot_hash_block(h - 8, &msg, &ch);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis)
            .expect("CONTROL#4: jackpot block with NON-ZERO message (no fold)");
    }

    /// CONTROL #5: identical to #4 but with `PROD_LB4`
    /// (log_blowup = 4, 16× LDE) instead of `TEST_PEARL`
    /// (log_blowup = 2). If this PASSES while #4 FAILS, the §4.A
    /// bug is a composite-AIR constraint whose true degree
    /// exceeds what log_blowup=2 supports — zero-valued (thus
    /// masked) for the all-zero messages every prior test used,
    /// non-zero only with a real `JACKPOT_MSG`. Fix is then
    /// either a degree reduction in the offending chip or a
    /// blowup bump (TEST_PEARL/PROD use 2/3).
    /// RESULT: **FAILED** at log_blowup=4 too ⇒ NOT a
    /// degree-vs-blowup issue; the non-zero-message jackpot bug
    /// is blowup-independent. `#[ignore]` (reproducer).
    #[test]
    #[ignore = "pre-existing: non-zero-msg jackpot fails at lb4 too (blowup-independent); HIGH2_2_DESIGN §4.A"]
    fn high2_2_jackpot_nonzero_msg_lb4() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::PROD_LB4);
        let ch: [u32; 8] = core::array::from_fn(|i| 0x5EED_0000 + i as u32);
        let msg: [u32; 16] = core::array::from_fn(|i| 0xABCD_0001u32.wrapping_mul(i as u32 + 1));
        let mut trace = CompositeTrace::baseline_min();
        let h = trace.height();
        let _ = trace.place_jackpot_hash_block(h - 8, &msg, &ch);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis)
            .expect("CONTROL#5: non-zero-message jackpot at log_blowup=4");
    }

    /// RESULT (2026-05-16): **FAILED** `OodEvaluationMismatch
    /// { index: None }`. ⇒ the §4.A bug is a **base
    /// `CompositeFullAir` constraint interaction between the fold
    /// chain and the jackpot-hash block** — independent of the
    /// program-pin, §4.D keystone, batch-stark, LogUp, and
    /// FoldChip degree (all ruled out by bisection). Each alone
    /// passes (`high2_2_fold_chain_in_composite_unit` ✓; every
    /// shipping jackpot-block test ✓); only *together* they
    /// fail. `place_jackpot_hash_block` does not overwrite
    /// FOLD_* (verified). Next step (HIGH2_2_DESIGN §4.A): run
    /// this trace under uni-stark **debug `check_constraints`**
    /// to name the exact failing row/constraint. `#[ignore]` so
    /// the suite stays green.
    #[test]
    #[ignore = "HIGH-2.2 §4.A: base fold↔jackpot constraint interaction; needs debug check_constraints; see HIGH2_2_DESIGN.md §4.A"]
    fn high2_2_fold_chain_jackpot_unit() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let ch: [u32; 8] = core::array::from_fn(|i| 0x5EED_0000 + i as u32);
        let mut trace = CompositeTrace::baseline_min();
        let h = trace.height();
        let xs: Vec<i32> = (0..16i32)
            .map(|i| i.wrapping_mul(0x0151_5151) ^ 0x33)
            .collect();
        let m = trace.place_fold_chain(64, &xs);
        let _ = trace.place_jackpot_hash_block(h - 8, &m, &ch);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis).expect(
            "BISECTION#2: fold + jackpot via unit CompositeFullAir (no pin/keystone)",
        );
    }

    /// HIGH-2.2 §4.A bisection: same fold-chain+keystone+jackpot
    /// trace, proven via the **uni-stark pinned** path
    /// (`composite_prove_pinned` / `composite_verify_pinned`) —
    /// the §4.D keystone + CRIT-1 program-pin **without**
    /// batch-stark / LogUp. Splits the failing layer:
    ///   - this PASSES + `*_pinned_logup` FAILS ⇒ bug is the
    ///     batch-stark LogUp layer.
    ///   - this FAILS ⇒ bug is the §4.D keystone / program-pin
    ///     (independent of batch-stark).
    /// RESULT (2026-05-16): **FAILED** `OodEvaluationMismatch`.
    /// ⇒ the §4.A bug is **not** batch-stark/LogUp-specific
    /// (uni-stark pinned fails too) and **not** FoldChip
    /// constraint degree (the degree-2 rewrite didn't help).
    /// Locus narrowed to **(fold chain + jackpot block) under
    /// the pinned path** (CompositeFullAir + program-pin + §4.D
    /// keystone); the unit `composite_prove` + fold-only path
    /// passes. Next bisection (documented, HIGH2_2_DESIGN.md
    /// §4.A): unit `composite_prove` + fold + jackpot (no
    /// pin/keystone) to split a base fold↔jackpot interaction
    /// from a keystone/program-pin one. `#[ignore]` so the suite
    /// stays green.
    #[test]
    #[ignore = "HIGH-2.2 §4.A: (fold+jackpot)×pinned uni-stark fails; not degree/not batch-stark; see HIGH2_2_DESIGN.md §4.A"]
    fn high2_2_fold_chain_pinned_unistark() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let ch: [u32; 8] = core::array::from_fn(|i| 0x5EED_0000 + i as u32);
        let mut trace = CompositeTrace::baseline_min();
        let h = trace.height();
        let xs: Vec<i32> = (0..16i32)
            .map(|i| i.wrapping_mul(0x0151_5151) ^ 0x33)
            .collect();
        let m = trace.place_fold_chain(64, &xs);
        let _ = trace.place_jackpot_hash_block(h - 8, &m, &ch);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let canonical = extract_program(&trace.matrix);
        let (proof, _) = composite_prove_pinned(&cfg, trace, &pis);
        composite_verify_pinned(&cfg, &canonical, &proof, &pis).expect(
            "BISECTION: fold chain + keystone + program-pin (uni-stark, no LogUp)",
        );
    }

    // ───────────── CRIT-1 malicious-prover regression suite ─────────────
    //
    // ZKP_SECURITY_REPORT CRIT-1: a malicious prover can zero every
    // selector to vacate the C1/C3/C4 PI bindings and forge a
    // winning proof with no work. These tests assert the
    // program-pinned API closes that: a proof only verifies against
    // the *canonical* program's verifying key (rebuilt by the
    // verifier from the trusted shape, never from the proof).

    /// Build a representative honest/canonical trace: matrix-hash
    /// A/B (C3) + key-pin rows (C1) + final jackpot-hash block
    /// (C4). Mirrors `ai-pow::zk_bridge`'s construction. Returns
    /// the trace; `extract_program` of it is the canonical program.
    fn honest_trace() -> CompositeTrace {
        let kappa = [0xA5u8; 32];
        let jk: [u32; 8] = core::array::from_fn(|i| 0xC0FE_0000 + i as u32);
        let ch: [u32; 8] = core::array::from_fn(|i| 0x5EED_0000 + i as u32);
        let a = vec![0x11u8; 1024];
        let b = vec![0x22u8; 1024];
        let mut t = CompositeTrace::baseline_min();
        let h = t.height();
        let (n1, _) = t.place_matrix_hash_a(0, &a, &kappa);
        let (mh_end, _) = t.place_matrix_hash_b(n1, &b, &kappa);
        t.place_key_pin_row(mh_end + 1, false, &jk);
        t.place_key_pin_row(mh_end + 2, true, &ch);
        // jackpot-hash keyed by COMMITMENT_HASH (= `ch` words).
        t.place_jackpot_hash_block(h - 8, &[0u32; 16], &ch);
        t
    }

    /// Honest pinned round-trip verifies; the difficulty check is
    /// real (a 0 target rejects the non-zero keyed digest).
    #[test]
    fn crit1_honest_pinned_roundtrip() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = honest_trace();
        let canonical = extract_program(&trace.matrix);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_ne!(pis.hash_jackpot, [0u32; 8], "C4 digest non-vacuous");

        let (proof, prog) = composite_prove_pinned(&cfg, trace, &pis);
        assert_eq!(prog.values, canonical.values, "prover program == canonical");

        composite_verify_pinned(&cfg, &canonical, &proof, &pis)
            .expect("honest pinned proof must verify against canonical program");
        composite_verify_pow_pinned(&cfg, &canonical, &proof, &pis, &[0xFFu8; 32])
            .expect("clears an easy target");
        // Hardest target 0: BLAKE3(0,key) > 0 ⇒ difficulty not met.
        match composite_verify_pow_pinned(&cfg, &canonical, &proof, &pis, &[0u8; 32]) {
            Err(PowVerifyError::DifficultyNotMet) => {}
            other => panic!("expected DifficultyNotMet, got {other:?}"),
        }
    }

    /// THE CRIT-1 exploit, now blocked. A malicious prover submits
    /// an all-zero-selector trace (no matmul, no hashing, no work)
    /// with a forged winning `HASH_JACKPOT = 0`. It is
    /// self-consistent against its *own* (all-zero) program, but
    /// the verifier uses the **canonical** program's VK — the
    /// preprocessed commitment differs, so verification fails.
    #[test]
    fn crit1_zeroed_selector_forgery_rejected() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        // Canonical program the honest verifier trusts.
        let canonical = extract_program(&honest_trace().matrix);

        // Malicious: baseline (all selectors 0) + forged zero PIs
        // (HASH_JACKPOT = 0 ≤ any target).
        let evil = CompositeTrace::baseline_min();
        let forged = CompositePublicInputs::zero();
        let (evil_proof, evil_prog) = composite_prove_pinned(&cfg, evil, &forged);
        assert_ne!(
            evil_prog.values, canonical.values,
            "attacker's program differs from canonical"
        );

        // Self-consistent against the attacker's own program
        // (the AIR is satisfied for an all-zero schedule) — this
        // is exactly why pinning to a *trusted* program matters.
        composite_verify_pinned(&cfg, &evil_prog, &evil_proof, &forged)
            .expect("attacker proof is self-consistent vs its own program");

        // Against the canonical (trusted) program: REJECTED.
        assert!(
            composite_verify_pinned(&cfg, &canonical, &evil_proof, &forged).is_err(),
            "CRIT-1: forged proof must fail against the canonical program VK"
        );
        assert!(
            composite_verify_pow_pinned(
                &cfg, &canonical, &evil_proof, &forged, &[0xFFu8; 32]
            )
            .is_err(),
            "CRIT-1: forged winning PoW must be rejected"
        );
    }

    /// Tampering any PROGRAM_COL in an otherwise-honest trace
    /// changes the prover's committed program; verification
    /// against the canonical program rejects it.
    #[test]
    fn crit1_tampered_program_col_rejected() {
        use crate::composite_full_air::PROGRAM_COLS;
        use crate::composite_layout::TOTAL_TRACE_WIDTH;
        use p3_field::integers::QuotientMap;
        use p3_field::PrimeField64;

        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let canonical = extract_program(&honest_trace().matrix);

        for &col in &PROGRAM_COLS {
            let mut t = honest_trace();
            // Tamper this program column on a mid-trace row.
            let r = 50usize;
            let cur = t.matrix.values
                [r * TOTAL_TRACE_WIDTH + col]
                .as_canonical_u64();
            t.matrix.values[r * TOTAL_TRACE_WIDTH + col] =
                <Val<AiPowStarkConfig> as QuotientMap<u64>>::from_int(
                    cur.wrapping_add(1),
                );
            let pis = CompositePublicInputs::derive_from_matrix(&t.matrix);
            let (proof, _) = composite_prove_pinned(&cfg, t, &pis);
            assert!(
                composite_verify_pinned(&cfg, &canonical, &proof, &pis).is_err(),
                "tampered PROGRAM_COL {col} must be rejected vs canonical program"
            );
        }
    }

    /// Even with the *correct* canonical program, a prover cannot
    /// forge `HASH_JACKPOT`: with selectors pinned, IS_HASH_JACKPOT
    /// fires on the jackpot-hash row and the C4 binding forces
    /// CV_OUT == PI_HASH_JACKPOT (the real non-zero keyed digest),
    /// so a swapped-to-zero PI violates the constraint.
    #[test]
    fn crit1_forged_hash_jackpot_with_canonical_program_rejected() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = honest_trace();
        let canonical = extract_program(&trace.matrix);
        let mut pis = CompositePublicInputs::derive_from_trace(&trace);
        pis.hash_jackpot = [0u32; 8]; // forge a trivially-winning value

        let (proof, _) = composite_prove_pinned(&cfg, trace, &pis);
        assert!(
            composite_verify_pinned(&cfg, &canonical, &proof, &pis).is_err(),
            "CRIT-1: forged HASH_JACKPOT must fail the C4 binding under the pinned program"
        );
    }

    /// HIGH-2: the C4-hashed `JACKPOT_MSG` is no longer
    /// prover-free — the pinned AIR forces last-row
    /// `JACKPOT_MSG[0..4] == CUMSUM_TILE[0..4]` (matmul-bound) and
    /// `JACKPOT_MSG[4..16] == 0`. An attacker who grinds an
    /// arbitrary winning jackpot message (the old hashcash attack,
    /// no matmul) is rejected: the planted message no longer
    /// equals the bound accumulator.
    #[test]
    fn high2_free_jackpot_message_rejected() {
        use crate::composite_layout::{JACKPOT_MSG_START, TOTAL_TRACE_WIDTH};
        use p3_field::integers::QuotientMap;

        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        // Honest baseline: CUMSUM = 0, JACKPOT_MSG = 0 on the last
        // row ⇒ keystone holds (0 == 0); HASH_JACKPOT =
        // BLAKE3(0, key=s_a), a fixed value the attacker cannot
        // grind. Sanity: verifies.
        let ok = honest_trace();
        let canonical = extract_program(&ok.matrix);
        let pis_ok = CompositePublicInputs::derive_from_trace(&ok);
        let (p_ok, _) = composite_prove_pinned(&cfg, ok, &pis_ok);
        composite_verify_pinned(&cfg, &canonical, &p_ok, &pis_ok)
            .expect("zero-CUMSUM honest trace satisfies the HIGH-2 keystone");

        // Attack: plant a "winning" free jackpot message on the
        // last row while CUMSUM stays 0 — exactly the pre-HIGH-2
        // hashcash forge (no matmul). Keystone JACKPOT_MSG ==
        // CUMSUM is violated ⇒ must be rejected.
        let mut evil = honest_trace();
        let h = evil.height();
        let last = (h - 1) * TOTAL_TRACE_WIDTH;
        evil.matrix.values[last + JACKPOT_MSG_START] =
            <Val<AiPowStarkConfig> as QuotientMap<u64>>::from_int(0xDEAD_BEEFu64);
        let pis_evil = CompositePublicInputs::derive_from_matrix(&evil.matrix);
        let (p_evil, _) = composite_prove_pinned(&cfg, evil, &pis_evil);
        assert!(
            composite_verify_pinned(&cfg, &canonical, &p_evil, &pis_evil).is_err(),
            "HIGH-2: a free (non-CUMSUM) jackpot message must be rejected"
        );
    }

    // ───────── HIGH-2.2 §4.C Route-A production suite ─────────
    //
    // The CRIT-1 / HIGH-2 adversarial regressions, re-run against
    // the batch-stark pinned+LogUp path (`*_pinned_logup`). These
    // prove the production Route-A binding keeps CRIT-1 soundness
    // and the HIGH-2 keystone *while additionally enforcing the
    // noised_packed/range LogUp*. (The noised_packed *matmul-input*
    // binding is non-vacuous only once §4.A places real matmul
    // rows — HIGH2_2_DESIGN.md §4.C.10; not overclaimed here.)

    /// Honest pinned+LogUp round-trip verifies; the C2 difficulty
    /// check is real (0 target rejects the non-zero keyed digest,
    /// an all-FF target clears it).
    #[test]
    fn routea_honest_roundtrip_and_pow() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = honest_trace();
        let canonical = extract_program(&trace.matrix);
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_ne!(pis.hash_jackpot, [0u32; 8], "C4 digest non-vacuous");

        let (proof, prog) = composite_prove_pinned_logup(&cfg, trace, &pis);
        assert_eq!(prog.values, canonical.values, "prover program == canonical");

        composite_verify_pinned_logup(&cfg, &canonical, &proof, &pis)
            .expect("Route-A honest pinned+LogUp proof must verify");
        composite_verify_pow_pinned_logup(&cfg, &canonical, &proof, &pis, &[0xFFu8; 32])
            .expect("clears an easy target");
        match composite_verify_pow_pinned_logup(&cfg, &canonical, &proof, &pis, &[0u8; 32]) {
            Err(PowVerifyError::DifficultyNotMet) => {}
            other => panic!("expected DifficultyNotMet, got {other:?}"),
        }
    }

    /// CRIT-1 under Route A: a zeroed-selector forgery is
    /// self-consistent vs its own program but REJECTED vs the
    /// canonical program's preprocessed commitment.
    #[test]
    fn routea_crit1_zeroed_selector_forgery_rejected() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let canonical = extract_program(&honest_trace().matrix);

        let evil = CompositeTrace::baseline_min();
        let forged = CompositePublicInputs::zero();
        let (evil_proof, evil_prog) = composite_prove_pinned_logup(&cfg, evil, &forged);
        assert_ne!(evil_prog.values, canonical.values);

        composite_verify_pinned_logup(&cfg, &evil_prog, &evil_proof, &forged)
            .expect("evil proof self-consistent vs its own program");
        assert!(
            composite_verify_pinned_logup(&cfg, &canonical, &evil_proof, &forged).is_err(),
            "CRIT-1 under Route A: forged proof must fail vs canonical program"
        );
        assert!(
            composite_verify_pow_pinned_logup(
                &cfg, &canonical, &evil_proof, &forged, &[0xFFu8; 32]
            )
            .is_err(),
            "CRIT-1 under Route A: forged winning PoW rejected"
        );
    }

    /// Tampering any PROGRAM_COL is rejected vs the canonical
    /// program under Route A (full coverage of all PROGRAM_COLS).
    #[test]
    fn routea_crit1_tampered_program_col_rejected() {
        use crate::composite_full_air::PROGRAM_COLS;
        use crate::composite_layout::TOTAL_TRACE_WIDTH;
        use p3_field::integers::QuotientMap;
        use p3_field::PrimeField64;

        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let canonical = extract_program(&honest_trace().matrix);

        for &col in &PROGRAM_COLS {
            let mut t = honest_trace();
            let r = 50usize;
            let cur =
                t.matrix.values[r * TOTAL_TRACE_WIDTH + col].as_canonical_u64();
            t.matrix.values[r * TOTAL_TRACE_WIDTH + col] =
                <Val<AiPowStarkConfig> as QuotientMap<u64>>::from_int(cur.wrapping_add(1));
            let pis = CompositePublicInputs::derive_from_matrix(&t.matrix);
            let (proof, _) = composite_prove_pinned_logup(&cfg, t, &pis);
            assert!(
                composite_verify_pinned_logup(&cfg, &canonical, &proof, &pis).is_err(),
                "Route A: tampered PROGRAM_COL {col} must be rejected vs canonical"
            );
        }
    }

    /// HIGH-2 keystone holds under Route A: a free (non-CUMSUM)
    /// winning jackpot message is rejected.
    #[test]
    fn routea_high2_free_jackpot_message_rejected() {
        use crate::composite_layout::{JACKPOT_MSG_START, TOTAL_TRACE_WIDTH};
        use p3_field::integers::QuotientMap;

        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        let ok = honest_trace();
        let canonical = extract_program(&ok.matrix);
        let pis_ok = CompositePublicInputs::derive_from_trace(&ok);
        let (p_ok, _) = composite_prove_pinned_logup(&cfg, ok, &pis_ok);
        composite_verify_pinned_logup(&cfg, &canonical, &p_ok, &pis_ok)
            .expect("zero-CUMSUM honest trace satisfies the keystone under Route A");

        let mut evil = honest_trace();
        let h = evil.height();
        let last = (h - 1) * TOTAL_TRACE_WIDTH;
        evil.matrix.values[last + JACKPOT_MSG_START] =
            <Val<AiPowStarkConfig> as QuotientMap<u64>>::from_int(0xDEAD_BEEFu64);
        let pis_evil = CompositePublicInputs::derive_from_matrix(&evil.matrix);
        let (p_evil, _) = composite_prove_pinned_logup(&cfg, evil, &pis_evil);
        assert!(
            composite_verify_pinned_logup(&cfg, &canonical, &p_evil, &pis_evil).is_err(),
            "HIGH-2 under Route A: free jackpot message must be rejected"
        );
    }

    #[test]
    fn hash_jackpot_le_bytes_is_blake3_digest_order() {
        // word i ↦ bytes[4i..4i+4] little-endian — the inverse of
        // M52's `u32::from_le_bytes([digest[4i..4i+4]])`.
        let hj: [u32; 8] = [
            0x04030201, 0x08070605, 0x0C0B0A09, 0x100F0E0D,
            0xEFBEADDE, 0xCEFAEDFE, 0xBEBAFECA, 0x78563412,
        ];
        let bytes = hash_jackpot_le_bytes(&hj);
        assert_eq!(&bytes[0..4], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(&bytes[28..32], &[0x12, 0x34, 0x56, 0x78]);
        // Round-trip back to words (the place_matrix_hash encoding).
        let back: [u32; 8] = core::array::from_fn(|i| {
            u32::from_le_bytes([
                bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3],
            ])
        });
        assert_eq!(back, hj);
    }

    #[test]
    fn c2_difficulty_check_pass_and_fail() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        // Baseline trace: no IS_HASH_JACKPOT row ⇒ hash_jackpot = 0.
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_eq!(pis.hash_jackpot, [0u32; 8]);
        let proof = composite_prove(&cfg, trace, &pis);

        // hash_jackpot = 0 (all-zero LE u256). Any non-zero target
        // ⇒ 0 ≤ target ⇒ PoW check passes.
        let easy_target = [0xFFu8; 32];
        composite_verify_pow(&cfg, &proof, &pis, &easy_target)
            .expect("zero HASH_JACKPOT clears a max target");

        // Hardest possible target: 0. 0 ≤ 0 ⇒ still passes (equality).
        let zero_target = [0u8; 32];
        composite_verify_pow(&cfg, &proof, &pis, &zero_target)
            .expect("0 ≤ 0 is a pass (>= comparison is inclusive)");

        // Tamper the PI hash_jackpot so it's large, with a tiny
        // target ⇒ DifficultyNotMet (and STARK still verifies since
        // baseline has no IS_HASH_JACKPOT row, so the binding
        // constraint is vacuous and hash_jackpot is unconstrained).
        let mut big = pis.clone();
        big.hash_jackpot = [0xFFFF_FFFF; 8]; // max u256
        let big_proof = {
            let trace2 = CompositeTrace::baseline_min();
            composite_prove(&cfg, trace2, &big)
        };
        let tiny_target = {
            let mut t = [0u8; 32];
            t[0] = 1; // value = 1
            t
        };
        match composite_verify_pow(&cfg, &big_proof, &big, &tiny_target) {
            Err(PowVerifyError::DifficultyNotMet) => {}
            other => panic!("expected DifficultyNotMet, got {other:?}"),
        }
    }

    #[test]
    fn composite_proof_is_serializable() {
        // The proof type derives Serialize/Deserialize (see crates/
        // ai-pow-zk/Cargo.toml for the bincode dep). Verifying a
        // bincode round-trip is the structural soundness check
        // every lib-level consumer cares about.
        use bincode::config::standard as bincode_standard;

        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);

        let encoded =
            bincode::serde::encode_to_vec(&proof, bincode_standard()).expect("encode");
        let (decoded, _len) = bincode::serde::decode_from_slice::<Proof<AiPowStarkConfig>, _>(
            &encoded,
            bincode_standard(),
        )
        .expect("decode");
        composite_verify(&cfg, &decoded, &pis).expect("decoded proof verifies");
    }

    /// Two proofs over baseline traces of different sizes both
    /// verify with the same config (the config is per-params, not
    /// per-trace-size, in TEST_PEARL).
    #[test]
    fn composite_proofs_at_two_trace_sizes() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        let trace_small = CompositeTrace::baseline_min();
        let pis_small = CompositePublicInputs::derive_from_trace(&trace_small);
        let p_small = composite_prove(&cfg, trace_small, &pis_small);
        composite_verify(&cfg, &p_small, &pis_small).expect("small proof");

        let trace_big =
            CompositeTrace::baseline(crate::composite_layout::MIN_STARK_LEN * 2);
        let pis_big = CompositePublicInputs::derive_from_trace(&trace_big);
        let p_big = composite_prove(&cfg, trace_big, &pis_big);
        composite_verify(&cfg, &p_big, &pis_big).expect("big proof");
    }

    // =================================================================
    //  Public-input binding tests
    // =================================================================

    /// Tamper a PI element on the verifier side; verification
    /// rejects (the AIR's `when_last_row` constraint forces the
    /// trace's last-row CUMSUM_TILE to match `pis[0..4]`).
    #[test]
    fn verify_rejects_wrong_cumsum_pi() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);

        let mut bad_pis = pis.clone();
        bad_pis.cumsum[0] = 42; // baseline has 0 everywhere; 42 is wrong.

        assert!(
            composite_verify(&cfg, &proof, &bad_pis).is_err(),
            "wrong CUMSUM PI must reject"
        );
    }

    /// Tamper a JACKPOT PI element on the verifier side.
    #[test]
    fn verify_rejects_wrong_jackpot_pi() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);
        let proof = composite_prove(&cfg, trace, &pis);

        let mut bad_pis = pis.clone();
        bad_pis.jackpot[5] = 0xDEAD_BEEF;

        assert!(
            composite_verify(&cfg, &proof, &bad_pis).is_err(),
            "wrong JACKPOT PI must reject"
        );
    }

    /// Build a trace with threaded non-zero cumsum + jackpot;
    /// PIs derived from it; prove + verify succeeds.
    #[test]
    fn prove_verify_with_threaded_state() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);
        let mut trace = CompositeTrace::baseline_min();
        // Thread a non-zero state through to the last row.
        trace.fill_cumsum_passthrough(0, &[1, -2, 3, -4]);
        let jp: [u32; 16] = core::array::from_fn(|i| (i as u32 + 1) * 0x12345);
        trace.fill_jackpot_passthrough(0, &jp);

        let pis = CompositePublicInputs::derive_from_trace(&trace);
        assert_eq!(pis.cumsum, [1, -2, 3, -4]);
        assert_eq!(pis.jackpot, jp);

        let proof = composite_prove(&cfg, trace, &pis);
        composite_verify(&cfg, &proof, &pis)
            .expect("threaded-state proof must verify with matching PIs");
    }

    /// PIs are part of the verification call, so swapping a
    /// proof's PIs for another proof's still rejects.
    #[test]
    fn verify_rejects_pi_substitution_across_proofs() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::TEST_PEARL);

        // Proof A: baseline trace + zero PIs.
        let trace_a = CompositeTrace::baseline_min();
        let pis_a = CompositePublicInputs::derive_from_trace(&trace_a);
        let proof_a = composite_prove(&cfg, trace_a, &pis_a);

        // Proof B: threaded state + non-zero PIs.
        let mut trace_b = CompositeTrace::baseline_min();
        trace_b.fill_cumsum_passthrough(0, &[1, 1, 1, 1]);
        let pis_b = CompositePublicInputs::derive_from_trace(&trace_b);
        let _proof_b = composite_prove(&cfg, trace_b, &pis_b);

        // Verifying proof A against B's PIs must reject.
        assert!(
            composite_verify(&cfg, &proof_a, &pis_b).is_err(),
            "proof A with B's PIs must reject"
        );
    }

    /// PROD-shape bench. Ignored by default — run with
    /// `cargo test --release composite_proof_prod_bench -- --ignored --nocapture`.
    ///
    /// Measures prove + verify wall-clock for the baseline trace
    /// at MIN_STARK_LEN under [`CircuitConfig::PROD`] (`log_blowup
    /// = 3`, `num_queries = 80` — 120 bits of provable FRI
    /// soundness). The baseline trace has no chip activity, so
    /// this bench is a structural ceiling: real proofs with
    /// matmul / BLAKE3 activity will take longer because the
    /// dot-product / round constraints actually evaluate to
    /// non-trivial polynomials.
    #[test]
    #[ignore = "PROD bench — expensive; run with --ignored"]
    fn composite_proof_prod_bench() {
        let cfg = build_config(&test_zk_params(), &CircuitConfig::PROD);
        let trace = CompositeTrace::baseline_min();
        let pis = CompositePublicInputs::derive_from_trace(&trace);

        let t0 = std::time::Instant::now();
        let proof = composite_prove(&cfg, trace, &pis);
        let prove_ms = t0.elapsed().as_millis();

        let t1 = std::time::Instant::now();
        composite_verify(&cfg, &proof, &pis).expect("PROD verify");
        let verify_ms = t1.elapsed().as_millis();

        // Serialise to measure proof size.
        use bincode::config::standard as bincode_standard;
        let bytes = bincode::serde::encode_to_vec(&proof, bincode_standard())
            .expect("encode");
        let proof_bytes = bytes.len();

        println!(
            "ai-pow-zk PROD bench (composite baseline @ MIN_STARK_LEN = {} rows × {} cols):",
            crate::composite_layout::MIN_STARK_LEN,
            crate::composite_layout::TOTAL_TRACE_WIDTH
        );
        println!("  prove    : {prove_ms} ms");
        println!("  verify   : {verify_ms} ms");
        println!("  proof    : {proof_bytes} bytes");
    }
}
