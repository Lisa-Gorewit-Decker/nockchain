//! End-to-end Tip5 NPO → circuit-prover batch-STARK gate (C2.3 / M-S4).
//!
//! Mirrors `test_lookups.rs::test_poseidon2_ctl_lookups` for the
//! deployed Goldilocks D=1 width-16 rate-10 7-round Tip5: build a
//! circuit with CTL'd Tip5 inputs/outputs, run a *real*
//! `prove_all_tables` (which executes the `tip5_l` LogUp global
//! reconciliation inside the wrapped, validated `Tip5PermLookupAir`
//! plus the `WitnessChecks` cross-table CTL), then a *real*
//! `verify_all_tables`. Plus an adversarial negative test: a tampered
//! proof field must make verification fail.
//!
//! Uses `config::goldilocks_tip5()` (FRI `log_blowup = 2`, tier B = 4)
//! — the exact FRI tier the `p3-tip5-circuit-air` `Tip5PermLookupAir`
//! is validated sound at (its degree-4 §4.6 / x⁷ constraints need
//! B ≥ 4; the default `goldilocks()` B = 2 config cannot prove them).

use p3_batch_stark::ProverData;
use p3_circuit::CircuitBuilder;
use p3_circuit::ops::{Tip5Config, Tip5Goldilocks, Tip5PermCall, generate_tip5_trace};
use p3_circuit_prover::batch_stark_prover::tip5_air_builders;
use p3_circuit_prover::common::{NpoPreprocessor, get_airs_and_degrees_with_prep};
use p3_circuit_prover::config::{self, GoldilocksTipsConfig};
use p3_circuit_prover::{
    BatchStarkProver, CircuitProverData, ConstraintProfile, TablePacking, Tip5Preprocessor,
};
use p3_field::PrimeCharacteristicRing;
use p3_field::extension::BinomialExtensionField;
use p3_goldilocks::Goldilocks;
use p3_symmetric::Permutation;
use p3_tip5_circuit_air::{STATE_SIZE, permute};

type F = Goldilocks;

/// `Permutation<[Goldilocks;16]>` adapter over the in-crate,
/// KAT-anchored, bit-for-bit twin of `nockchain_math::tip5::permute`.
#[derive(Clone)]
struct Tip5Perm;

impl Permutation<[F; STATE_SIZE]> for Tip5Perm {
    fn permute(&self, input: [F; STATE_SIZE]) -> [F; STATE_SIZE] {
        let mut s: [u64; STATE_SIZE] =
            core::array::from_fn(|i| <F as p3_field::PrimeField64>::as_canonical_u64(&input[i]));
        permute(&mut s);
        core::array::from_fn(|i| F::from_u64(s[i]))
    }
}

fn tip5_native(input: &[F; STATE_SIZE]) -> [F; STATE_SIZE] {
    Tip5Perm.permute(*input)
}

/// Build the Tip5 CTL circuit + run a real `prove_all_tables`,
/// returning the proof and the prover (for verification).
fn build_and_prove() -> (
    p3_circuit_prover::BatchStarkProof<GoldilocksTipsConfig>,
    BatchStarkProver<GoldilocksTipsConfig>,
) {
    let mut builder: CircuitBuilder<F> = CircuitBuilder::new();
    let tip5_config = Tip5Config::GOLDILOCKS_W16;
    builder.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<F, Tip5Goldilocks>,
        Tip5Perm,
    );

    // Two CTL'd public inputs into the Tip5 permutation; the rest of
    // the 16-wide state is zero (new_start sponge absorb).
    let input0 = builder.public_input();
    let input1 = builder.public_input();
    let mut perm_inputs: [Option<_>; 16] = [None; 16];
    perm_inputs[0] = Some(input0);
    perm_inputs[1] = Some(input1);

    let (_op_id, outputs) = builder
        .add_tip5_perm(&Tip5PermCall {
            config: tip5_config,
            new_start: true,
            inputs: perm_inputs,
            out_ctl: [true; 10],
            return_all_outputs: false,
        })
        .unwrap();

    // Bind every rate output to a public input via assert_zero so the
    // WitnessChecks producer/consumer multiset is non-trivial and the
    // `tip5_l` reconciliation must actually hold over real values.
    for out in outputs.iter().take(10) {
        let e = builder.public_input();
        let diff = builder.sub(out.unwrap(), e);
        builder.assert_zero(diff);
    }

    let circuit = builder.build().unwrap();
    // FRI tier B=4 (log_blowup=2) — required for the validated
    // degree-4 Tip5 lookup-AIR constraints.
    let cfg = config::goldilocks_tip5();

    let npo_prep: Vec<Box<dyn NpoPreprocessor<F>>> = vec![Box::new(Tip5Preprocessor)];
    let air_builders = tip5_air_builders::<GoldilocksTipsConfig, 1>();
    let (airs_degrees, primitive_columns, non_primitive_columns) =
        get_airs_and_degrees_with_prep::<GoldilocksTipsConfig, F, 1>(
            &circuit,
            &TablePacking::default(),
            &npo_prep,
            &air_builders,
            ConstraintProfile::Standard,
        )
        .unwrap();
    let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();

    let mut runner = circuit.runner();

    // Concrete input state and its native Tip5 image.
    let in0 = F::from_u64(0x1234_5678_9abc_def0);
    let in1 = F::from_u64(0x0fed_cba9_8765_4321);
    let mut state = [F::ZERO; STATE_SIZE];
    state[0] = in0;
    state[1] = in1;
    let image = tip5_native(&state);

    let mut pis = vec![in0, in1];
    pis.extend_from_slice(&image[..10]);
    runner.set_public_inputs(&pis).unwrap();
    let traces = runner.run().unwrap();

    let prover_data = ProverData::from_airs_and_degrees(&cfg, &airs, &degrees);
    let circuit_prover_data =
        CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

    let mut prover = BatchStarkProver::new(cfg);
    prover.register_tip5_table::<1>(tip5_config);

    let proof = prover
        .prove_all_tables(&traces, &circuit_prover_data)
        .unwrap();
    assert_eq!(proof.ext_degree, 1);
    assert!(proof.w_binomial.is_none());
    (proof, prover)
}

#[test]
fn test_tip5_ctl_lookups() {
    let (proof, prover) = build_and_prove();
    prover
        .verify_all_tables(&proof)
        .expect("Tip5 CTL lookup verification should succeed");
}

#[test]
fn test_tip5_tampered_proof_fails() {
    let (mut proof, prover) = build_and_prove();

    // Sanity: the untampered proof verifies.
    prover
        .verify_all_tables(&proof)
        .expect("baseline Tip5 proof must verify before tampering");

    // Corrupt a FRI-bound opened value: the Tip5 table (instance
    // index 3 = Const,Public,Alu,Tip5) opened trace evaluation at the
    // out-of-domain point. Any single mutated opening must break the
    // batched FRI / constraint binding ⇒ `verify_all_tables` must
    // reject (Err) or panic — it must NOT silently accept.
    const TIP5_INSTANCE: usize = 3;
    let inst = &mut proof.proof.opened_values.instances[TIP5_INSTANCE];
    assert!(
        !inst.base_opened_values.trace_local.is_empty(),
        "expected Tip5 opened trace values"
    );
    // `GoldilocksTipsConfig`'s challenge field is `BinomialExtensionField<Goldilocks, 2>`.
    inst.base_opened_values.trace_local[0] += BinomialExtensionField::<Goldilocks, 2>::ONE;

    let tampered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prover.verify_all_tables(&proof)
    }));
    match tampered {
        Ok(Ok(())) => panic!("tampered Tip5 proof was accepted by verify_all_tables"),
        Ok(Err(_)) | Err(_) => { /* rejected (Err) or panicked — both are correct */ }
    }
}
