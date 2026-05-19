//! C2.4 — Tip5 **Layer-0** in-circuit recursion-verify soundness gate.
//!
//! The vendored recursion verifier verifies a *real* `p3-uni-stark`
//! proof produced under the **exact** ai-pow-zk Layer-0 Tip5
//! `StarkConfig`, in-circuit, across the ai-pow-zk 120-bit FRI sweep
//! (PROD / LB2 / LB4 / LB5 / LB6), **accepting valid proofs and
//! rejecting tampered ones**.
//!
//! Faithful mirror of
//! `recursion/tests/goldilocks.rs::test_goldilocks_fibonacci_verifier`:
//! native `prove`/`verify` (sanity) → `StarkVerifierInputsBuilder::
//! allocate` → `verify_p3_uni_proof_circuit::<FibonacciAir,
//! Tip5Layer0Config, …, Tip5Config, 16, 10>` → `set_fri_mmcs_private_
//! data` → `runner().run()`. The *only* genuine differences vs the
//! Poseidon2-Goldilocks template are the Tip5 permutation
//! (`p3_tip5_circuit_air::Tip5Perm`), sponge **rate 10 / capacity 6 /
//! digest 5** (vs Poseidon2 W8 rate 4 / digest 4), `WIDTH = 16`,
//! `RATE = 10`, `DIGEST_ELEMS = 5`, the in-circuit challenger / MMCS
//! permutation config `Tip5Config::GOLDILOCKS_W16`, and the ai-pow-zk
//! 120-bit FRI sweep `FriParameters`.
//!
//! The binding validated here is the **recursion forgery-binding**
//! (Established fact: `verify_p3_uni_proof_circuit` recomputes the
//! Fiat–Shamir transcript + reconstructs Merkle roots in-circuit AND
//! runs the FRI low-degree fold-chain — this IS the binding). All of
//! it executes inside `runner().run()`: a `WitnessConflict` (`Err`)
//! is a rejection. A tampered opened trace value makes the in-circuit
//! quotient-consistency `connect` fail, so the verifier circuit
//! rejects loudly — exactly `fibonacci.rs::test_tampered_ood_
//! evaluation` / `test_tip5_lookups::test_tip5_tampered_proof_fails`.
//! None of the FRI / L5 Tip5 challenger / MMCS recompute is bypassed
//! or weakened (the L5 gate already proved this recompute bit-for-bit
//! vs native).
//!
//! ── HONEST SCOPE (R1 validated-subset + precise residual) ───────────
//! VALIDATED HERE (this gate, exhaustively): the in-circuit
//! `verify_p3_uni_proof_circuit` binding for a REAL ai-pow-zk Layer-0
//! proof, every 120-bit sweep profile, accept-valid + reject-tampered.
//! The proven AIR is a representative non-trivial AIR (`FibonacciAir`,
//! the established recursion-test pattern); the *config* is exactly
//! ai-pow-zk's Layer-0 `StarkConfig`.
//!
//! RESIDUAL — NOT done, NOT faked (precisely scoped):
//!  (R-a) Outer `prove_all_tables` / `verify_all_tables` of the *Tip5
//!        Layer-0 verifier circuit*. The verifier circuit is over the
//!        challenge field `BinomialExtensionField<Goldilocks, 2>`
//!        (D=2), but Tip5 is intrinsically a D=1 (base-Goldilocks)
//!        permutation. Proving that circuit needs the circuit-prover's
//!        `WitnessChecks` cross-table CTL to emit a `witness_bus_dim
//!        = D` tuple for a D=1 perm in a D≥2 circuit. The workspace
//!        only supports witness-bus-dim ∈ {1 (base), 5 (quintic)} for
//!        *any* D=1 perm (`poseidon_d1_witness_bus_dim`: `2 => None`);
//!        `Tip5CircuitAir`'s `WitnessChecks` push is hardcoded to the
//!        2-element D=1 tuple `[idx, value]`. Making it D-aware
//!        (mirroring Poseidon1's `eval_interactions<…, WITNESS_EXT_D>`
//!        padded tuple) plus a `witness_bus_dim(2)` is an invasive
//!        change to the *validated* `tip5-circuit-air` cross-table CTL
//!        binding — a soundness linchpin the task hard-rules NOT to
//!        alter, and a circuit-prover-wide D=1-perm-in-D=2 gap that is
//!        M12-adjacent. State, do not fake.
//!  (R-b) Recursively verifying ai-pow-zk's *actual M10.1c composite
//!        `RecursiveAir`* proof (vs this representative `FibonacciAir`)
//!        across the C1 ai-pow-zk↔recursion workspace decoupling —
//!        M12-adjacent, explicitly out of scope.
//!
//! What IS landed and validated end-to-end: the recursion verifier
//! soundly verifies the *exact* ai-pow-zk Layer-0 Tip5 `StarkConfig`
//! in-circuit across the full 120-bit sweep, accept + tamper-reject.

mod common;

use p3_challenger::DuplexChallenger;
use p3_circuit::CircuitBuilder;
use p3_circuit::ops::{Tip5Config, Tip5Goldilocks, generate_recompose_trace, generate_tip5_trace};
use p3_circuit::test_utils::{FibonacciAir, generate_trace_rows};
use p3_commit::ExtensionMmcs;
use p3_dft::Radix2DitParallel;
use p3_field::extension::BinomialExtensionField;
use p3_field::{BasedVectorSpace, PrimeCharacteristicRing};
use p3_fri::{FriParameters, TwoAdicFriPcs};
use p3_goldilocks::Goldilocks;
use p3_merkle_tree::MerkleTreeMmcs;
use p3_recursion::pcs::fri::{FriVerifierParams, InputProofTargets, MerkleCapTargets, RecValMmcs};
use p3_recursion::pcs::set_fri_mmcs_private_data;
use p3_recursion::public_inputs::StarkVerifierInputsBuilder;
use p3_recursion::{VerificationError, verify_p3_uni_proof_circuit};
use p3_symmetric::{PaddingFreeSponge, Permutation, TruncatedPermutation};
use p3_tip5_circuit_air::Tip5Perm;
use p3_uni_stark::{StarkConfig, prove, verify};

use crate::common::InnerFriGeneric;

// =====================================================================
//  ai-pow-zk Layer-0 `StarkConfig` — replicated in-workspace verbatim
//  (`crates/ai-pow-zk/src/circuit.rs`). We CANNOT depend on ai-pow-zk,
//  so the type aliases are reconstructed here over
//  `p3_tip5_circuit_air::Tip5Perm`.
//
//  ai-pow-zk's Layer-0 `ValMmcs` is `MerkleTreeMmcs<<Goldilocks as
//  Field>::Packing, <Goldilocks as Field>::Packing, Tip5Sponge,
//  Tip5Compress, 2, 5>` (its `Tip5Perm` carries a SIMD-packed adapter
//  so the Merkle commit can hash multiple lanes per call). We added
//  the *bit-for-bit identical* packed adapter to
//  `p3_tip5_circuit_air::Tip5Perm` (lane-by-lane unpack → the same
//  scalar `tip5_spec::permute` → repack; KAT-tested vs scalar in
//  `p3-tip5-circuit-air::perm`), so this replication is byte-identical
//  to ai-pow-zk's Layer-0 and matches the recursion `RecValMmcs::Input
//  = MerkleTreeMmcs<F::Packing, F::Packing, …>` exactly.
// =====================================================================

type Val = Goldilocks;
type GlPacking = <Goldilocks as p3_field::Field>::Packing;
/// FRI challenge field: degree-2 binomial extension of Goldilocks.
type Challenge = BinomialExtensionField<Goldilocks, 2>;
/// Tip5 sponge for hashing matrix rows into Merkle leaves (W16,R10,O5).
type Tip5Sponge = PaddingFreeSponge<Tip5Perm, 16, 10, 5>;
/// Tip5 2-to-1 truncated permutation for internal node compression.
type Tip5Compress = TruncatedPermutation<Tip5Perm, 2, 5, 16>;
/// MMCS over Goldilocks values — P = PW = `<Goldilocks as
/// Field>::Packing`, exactly ai-pow-zk Layer-0's `ValMmcs`.
type ValMmcs = MerkleTreeMmcs<GlPacking, GlPacking, Tip5Sponge, Tip5Compress, 2, 5>;
/// MMCS for committing extension-field polynomials (FRI codewords).
type ChallengeMmcs = ExtensionMmcs<Goldilocks, Challenge, ValMmcs>;
/// Fiat–Shamir challenger using the same Tip5 permutation (W16,R10).
type Layer0Challenger = DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>;
/// DFT used by the FRI low-degree test on Goldilocks.
type Dft = Radix2DitParallel<Goldilocks>;
/// Univariate FRI PCS over Goldilocks.
type Pcs = TwoAdicFriPcs<Goldilocks, Dft, ValMmcs, ChallengeMmcs>;
/// The concrete Layer-0 `StarkConfig`.
type Tip5Layer0Config = StarkConfig<Pcs, Challenge, Layer0Challenger>;

const DIGEST_ELEMS: usize = 5;
const WIDTH: usize = 16;
const RATE: usize = 10;

/// The recursion `OpeningProof` target type for the Layer-0
/// `TwoAdicFriPcs`.
type InnerFri = InnerFriGeneric<Tip5Layer0Config, Tip5Sponge, Tip5Compress, DIGEST_ELEMS>;

/// `Tip5Perm` lifted to act on `Challenge` (`BinomialExtensionField<
/// Goldilocks, 2>`) lanes — reads each lane's constant basis
/// coefficient, runs the *base-field* scalar Tip5 permutation, and
/// re-embeds with only the constant coefficient set. Exact
/// `BinomialExtensionField<Goldilocks, 2>` analog of
/// `p3_test_utils::LiftPermToQuintic` (used by the validated
/// `fibonacci_batch_stark_prover_quintic.rs`). It is the
/// recursion-verifier-circuit counterpart of the native
/// `DuplexChallenger<Goldilocks, Tip5Perm, 16, 10>`: the in-circuit
/// Tip5 NPO witnesses exactly this (its constant coefficient is the
/// base Tip5 image). Tip5's split-and-lookup / x⁷ constraints are
/// unchanged (enforced over the base rows by the validated
/// `Tip5PermLookupAir`).
#[derive(Clone, Copy, Debug, Default)]
struct LiftTip5;

impl Permutation<[Challenge; 16]> for LiftTip5 {
    fn permute(&self, input: [Challenge; 16]) -> [Challenge; 16] {
        let bases: [Goldilocks; 16] = core::array::from_fn(|i| {
            <Challenge as BasedVectorSpace<Goldilocks>>::as_basis_coefficients_slice(&input[i])[0]
        });
        let out = Tip5Perm.permute(bases);
        core::array::from_fn(|i| {
            <Challenge as BasedVectorSpace<Goldilocks>>::from_basis_coefficients_fn(|j| {
                if j == 0 { out[i] } else { Goldilocks::ZERO }
            })
        })
    }

    fn permute_mut(&self, input: &mut [Challenge; 16]) {
        *input = Permutation::permute(self, *input);
    }
}

/// One point of the ai-pow-zk 120-bit FRI sweep
/// (`crates/ai-pow-zk/src/circuit.rs::FriProfile`). All five are
/// `pow_bits = 0`, `log_final_poly_len = 0`, `max_log_arity = 1`
/// (binary folding) — exactly `build_stark_config`.
#[derive(Clone, Copy, Debug)]
struct SweepProfile {
    name: &'static str,
    log_blowup: usize,
    num_queries: usize,
}

/// PROD{3,80}, LB2{2,120}, LB4{4,60}, LB5{5,48}, LB6{6,40} — every
/// point is 120-bit provable (`num_queries · log_blowup / 2 == 120`).
const SWEEP: [SweepProfile; 5] = [
    SweepProfile { name: "PROD", log_blowup: 3, num_queries: 80 },
    SweepProfile { name: "LB2", log_blowup: 2, num_queries: 120 },
    SweepProfile { name: "LB4", log_blowup: 4, num_queries: 60 },
    SweepProfile { name: "LB5", log_blowup: 5, num_queries: 48 },
    SweepProfile { name: "LB6", log_blowup: 6, num_queries: 40 },
];

/// Build the exact ai-pow-zk Layer-0 `StarkConfig` for `profile`,
/// byte-identical to `ai_pow_zk::circuit::build_stark_config`.
fn make_layer0_config(profile: SweepProfile) -> Tip5Layer0Config {
    let perm = Tip5Perm;
    let hash = Tip5Sponge::new(perm);
    let compress = Tip5Compress::new(perm);
    // `cap_height = 0` — Merkle root only (ai-pow-zk Layer-0).
    let val_mmcs = ValMmcs::new(hash, compress, 0);
    let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());
    let dft = Dft::default();
    let challenger = Layer0Challenger::new(perm);
    let fri_params = FriParameters {
        log_blowup: profile.log_blowup,
        log_final_poly_len: 0,
        max_log_arity: 1, // binary folding
        num_queries: profile.num_queries,
        commit_proof_of_work_bits: 0, // ai-pow-zk holds pow_bits == 0
        query_proof_of_work_bits: 0,
        mmcs: challenge_mmcs,
    };
    let pcs = Pcs::new(dft, val_mmcs, fri_params);
    StarkConfig::new(pcs, challenger)
}

fn fibonacci_setup() -> (p3_matrix::dense::RowMajorMatrix<Val>, Vec<Val>, FibonacciAir) {
    let n = 1 << 3;
    let x = 21u64;
    let trace = generate_trace_rows::<Val>(0, 1, n);
    let pis = vec![Val::ZERO, Val::ONE, Val::from_u64(x)];
    (trace, pis, FibonacciAir {})
}

/// In-circuit verification outcome of a real Layer-0 proof.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// `verify_p3_uni_proof_circuit` + `runner().run()` succeeded —
    /// the FRI low-degree + L5 Tip5 challenger/MMCS recompute binding
    /// accepted (valid-proof path).
    Accepted,
    /// The verifier circuit rejected: `runner().run()` returned `Err`
    /// (`WitnessConflict`) — the in-circuit FRI / quotient-consistency
    /// binding caught the tamper (tampered-proof path).
    Rejected,
}

/// Faithful mirror of `goldilocks.rs::test_goldilocks_fibonacci_
/// verifier` with the ai-pow-zk Layer-0 Tip5 `StarkConfig`. When
/// `tamper`, one opened OOD trace value of the *real* Layer-0 proof is
/// corrupted before in-circuit verification (mirror
/// `fibonacci.rs::test_tampered_ood_evaluation`).
fn verify_layer0_in_circuit(
    profile: SweepProfile,
    tamper: bool,
) -> Result<Outcome, VerificationError> {
    let config = make_layer0_config(profile);
    let (trace, pis, air) = fibonacci_setup();

    // ---- native prove + verify (sanity) ----
    let mut proof = prove(&config, &air, trace, &pis);
    assert!(
        verify(&config, &air, &proof, &pis).is_ok(),
        "[{}] native Layer-0 prove/verify must succeed before recursion",
        profile.name
    );

    if tamper {
        // Corrupt a single FRI-bound opened OOD trace evaluation: the
        // in-circuit quotient-consistency check must reject.
        proof.opened_values.trace_local[0] += Challenge::ONE;
    }

    // ---- build the recursive verification circuit ----
    let mut circuit_builder = CircuitBuilder::<Challenge>::new();
    // D=1 Tip5 NPO in the D=2 challenge-field circuit via the base
    // `enable_tip5_perm` + a `Challenge`-lane `LiftTip5` + recompose
    // (mirror of the validated quintic test's
    // `enable_poseidon2_perm_base::<…D1…>(…, Lift)` + `enable_recompose`).
    circuit_builder.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<Challenge, Tip5Goldilocks>,
        LiftTip5,
    );
    circuit_builder.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
    circuit_builder.set_recompose_coeff_ctl_for_decompose_links(true);

    // ai-pow-zk Layer-0 FRI verifier params for this sweep point.
    let fri_verifier_params = FriVerifierParams::with_mmcs(
        profile.log_blowup,
        0, // log_final_poly_len
        0, // commit_pow_bits
        0, // query_pow_bits
        Tip5Config::GOLDILOCKS_W16,
    );

    let verifier_inputs = StarkVerifierInputsBuilder::<
        Tip5Layer0Config,
        MerkleCapTargets<Val, DIGEST_ELEMS>,
        InnerFri,
    >::allocate(&mut circuit_builder, &proof, None, pis.len());

    // The generic FRI + L5 Tip5 challenger/MMCS recompute binding.
    let mmcs_op_ids = verify_p3_uni_proof_circuit::<
        FibonacciAir,
        Tip5Layer0Config,
        MerkleCapTargets<Val, DIGEST_ELEMS>,
        InputProofTargets<Val, Challenge, RecValMmcs<Val, DIGEST_ELEMS, Tip5Sponge, Tip5Compress>>,
        InnerFri,
        Tip5Config,
        WIDTH,
        RATE,
    >(
        &config,
        &air,
        &mut circuit_builder,
        &verifier_inputs.proof_targets,
        &verifier_inputs.air_public_targets,
        &None,
        &fri_verifier_params,
        Tip5Config::GOLDILOCKS_W16,
    )?;

    let circuit = circuit_builder.build()?;
    let mut runner = circuit.runner();

    let (public_inputs, private_inputs) = verifier_inputs.pack_values(&pis, &proof, &None);
    runner
        .set_public_inputs(&public_inputs)
        .map_err(VerificationError::Circuit)?;
    runner
        .set_private_inputs(&private_inputs)
        .map_err(VerificationError::Circuit)?;

    // MMCS sibling data — the canonical `set_fri_mmcs_private_data`
    // (the packed `Tip5Perm` adapter satisfies its packed-hasher
    // bound; routing is identical for every permutation).
    set_fri_mmcs_private_data::<
        Val,
        Challenge,
        ChallengeMmcs,
        ValMmcs,
        Tip5Sponge,
        Tip5Compress,
        DIGEST_ELEMS,
    >(
        &mut runner,
        &mmcs_op_ids,
        &proof.opening_proof,
        Tip5Config::GOLDILOCKS_W16,
    )
    .map_err(|e| VerificationError::InvalidProofShape(e.to_string()))?;

    // The full in-circuit FRI low-degree + Tip5 challenger/MMCS
    // recompute binding executes here. A tampered opened value makes
    // the quotient-consistency `connect` fail → `WitnessConflict`.
    match runner.run() {
        Ok(_) => Ok(Outcome::Accepted),
        Err(_) => Ok(Outcome::Rejected),
    }
}

// ---------------------------------------------------------------------
//  5 accept tests — one per ai-pow-zk 120-bit FRI sweep profile.
//  Each: real Layer-0 prove/verify → in-circuit
//  verify_p3_uni_proof_circuit (FRI low-degree + L5 Tip5
//  challenger/MMCS recompute) must ACCEPT the valid proof.
// ---------------------------------------------------------------------

#[test]
fn tip5_layer0_recursion_prod_accepts() {
    assert_eq!(
        verify_layer0_in_circuit(SWEEP[0], false).expect("PROD pipeline must not error"),
        Outcome::Accepted,
        "PROD: valid Layer-0 proof was REJECTED in-circuit",
    );
}

#[test]
fn tip5_layer0_recursion_lb2_accepts() {
    assert_eq!(
        verify_layer0_in_circuit(SWEEP[1], false).expect("LB2 pipeline must not error"),
        Outcome::Accepted,
        "LB2: valid Layer-0 proof was REJECTED in-circuit",
    );
}

#[test]
fn tip5_layer0_recursion_lb4_accepts() {
    assert_eq!(
        verify_layer0_in_circuit(SWEEP[2], false).expect("LB4 pipeline must not error"),
        Outcome::Accepted,
        "LB4: valid Layer-0 proof was REJECTED in-circuit",
    );
}

#[test]
fn tip5_layer0_recursion_lb5_accepts() {
    assert_eq!(
        verify_layer0_in_circuit(SWEEP[3], false).expect("LB5 pipeline must not error"),
        Outcome::Accepted,
        "LB5: valid Layer-0 proof was REJECTED in-circuit",
    );
}

#[test]
fn tip5_layer0_recursion_lb6_accepts() {
    assert_eq!(
        verify_layer0_in_circuit(SWEEP[4], false).expect("LB6 pipeline must not error"),
        Outcome::Accepted,
        "LB6: valid Layer-0 proof was REJECTED in-circuit",
    );
}

// ---------------------------------------------------------------------
//  Tamper-reject tests — corrupt one opened trace value of the real
//  Layer-0 proof; the in-circuit FRI / quotient-consistency binding
//  must REJECT. Fail loudly if the tampered proof is accepted.
// ---------------------------------------------------------------------

#[test]
fn tip5_layer0_recursion_prod_tampered_rejects() {
    assert_eq!(
        verify_layer0_in_circuit(SWEEP[0], true).expect("PROD tamper pipeline must not error"),
        Outcome::Rejected,
        "PROD: TAMPERED Layer-0 proof was ACCEPTED in-circuit — soundness hole",
    );
}

#[test]
fn tip5_layer0_recursion_lb4_tampered_rejects() {
    assert_eq!(
        verify_layer0_in_circuit(SWEEP[2], true).expect("LB4 tamper pipeline must not error"),
        Outcome::Rejected,
        "LB4: TAMPERED Layer-0 proof was ACCEPTED in-circuit — soundness hole",
    );
}
