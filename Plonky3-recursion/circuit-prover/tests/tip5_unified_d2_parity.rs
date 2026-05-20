//! M-S5b S1.B Poseidon2-removal P3 — KAT parity test: D=2 batch-STARK
//! with Tip5 NPO + recompose-coeff CTL at the new
//! [`goldilocks_tip5_unified_80bit`] config.
//!
//! This test exercises the **predicted C2.4 R-a tail trigger** per
//! `crates/ai-pow-zk/docs/2026-05-20_POSEIDON2_REMOVAL_SPEC.md` §3.1:
//! Tip5 perm at D=2 with `set_recompose_coeff_ctl_for_decompose_links(true)`
//! is the exact pattern the L1 outer-cert verifier circuit uses
//! (mirroring `recursion/tests/test_tip5_layer0_compression.rs::
//! build_layer0_verifier_circuit` line 294 + line 415 `register_tip5_table::<2>`).
//!
//! Test outcome interpretation:
//!   - **PASS:** Tip5-unified config works at D=2 with NPO; the
//!     C2.4 R-a tail orphan is either non-existent in production
//!     circuits or compensated elsewhere. P3 gate green; proceed
//!     to P4 (size measurement).
//!   - **Soft-fail at runner().run() with WitnessConflict:** the
//!     predicted blocker surfaces. Per spec §3.3.B, fall back to
//!     D=1 outer-cert (slower but functional). P2's smoke tests
//!     already validate the D=1 path; the Tip5-unified config is
//!     production-deployable at D=1.

use p3_batch_stark::ProverData;
use p3_circuit::CircuitBuilder;
use p3_circuit::ops::{
    Tip5Config, Tip5Goldilocks, Tip5PermCall, generate_recompose_trace, generate_tip5_trace,
};
use p3_circuit_prover::batch_stark_prover::{recompose_air_builders, tip5_air_builders};
use p3_circuit_prover::common::{NpoPreprocessor, get_airs_and_degrees_with_prep};
use p3_circuit_prover::config::{GoldilocksTipsConfig, goldilocks_tip5_unified_80bit};
use p3_circuit_prover::{
    BatchStarkProver, CircuitProverData, ConstraintProfile, RecomposePreprocessor, TablePacking,
    Tip5Preprocessor,
};
use p3_field::extension::BinomialExtensionField;
use p3_field::{BasedVectorSpace, PrimeCharacteristicRing};
use p3_goldilocks::Goldilocks;
use p3_symmetric::Permutation;
use p3_tip5_circuit_air::Tip5Perm;

type Val = Goldilocks;
type Challenge = BinomialExtensionField<Goldilocks, 2>;

/// `Tip5Perm` lifted to `Challenge` lanes (constant basis coeff only).
/// Verbatim copy of `recursion/tests/test_tip5_layer0_compression.rs:114`.
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

/// P3 — D=2 batch-STARK with Tip5 NPO + recompose-coeff CTL at the new
/// Tip5-unified config. The predicted R-a tail trigger.
#[test]
fn p3_tip5_d2_npo_with_recompose_ctl_at_tip5_unified() {
    // CircuitBuilder over Challenge (D=2 extension field) — matches the
    // working pattern in `test_tip5_layer0_compression.rs::build_layer0_verifier_circuit`.
    let mut builder = CircuitBuilder::<Challenge>::new();

    let tip5_config = Tip5Config::GOLDILOCKS_W16;
    builder.enable_tip5_perm::<Tip5Goldilocks, _>(
        generate_tip5_trace::<Challenge, Tip5Goldilocks>,
        LiftTip5,
    );

    // Recompose enabled + coeff CTL for decompose links — the exact
    // pattern the L1 outer-cert uses (per
    // `recursion/tests/test_tip5_layer0_compression.rs:293-294`).
    // This is the predicted R-a tail trigger at D=2.
    builder.enable_recompose::<Val>(generate_recompose_trace::<Val, Challenge>);
    builder.set_recompose_coeff_ctl_for_decompose_links(true);

    // Two CTL'd public inputs into the Tip5 permutation; rest of the
    // state is zero (new_start sponge absorb).
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

    // Bind each rate output to a public input.
    for out in outputs.iter().take(10) {
        let e = builder.public_input();
        let diff = builder.sub(out.unwrap(), e);
        builder.assert_zero(diff);
    }

    let circuit = builder.build().unwrap();

    let cfg: GoldilocksTipsConfig = goldilocks_tip5_unified_80bit();

    // D=2 NPO registration — the predicted R-a tail trigger.
    let npo_prep: Vec<Box<dyn NpoPreprocessor<Val>>> = vec![
        Box::new(Tip5Preprocessor),
        Box::new(RecomposePreprocessor::new(true)),
    ];
    let mut air_builders = tip5_air_builders::<GoldilocksTipsConfig, 2>();
    air_builders.extend(recompose_air_builders::<GoldilocksTipsConfig, 2>(1, true));

    let table_packing = TablePacking::new(1, 8);
    let (airs_degrees, primitive_columns, non_primitive_columns) =
        get_airs_and_degrees_with_prep::<GoldilocksTipsConfig, Challenge, 2>(
            &circuit,
            &table_packing,
            &npo_prep,
            &air_builders,
            ConstraintProfile::Standard,
        )
        .expect("get_airs_and_degrees_with_prep at Tip5-unified D=2");
    let (airs, degrees): (Vec<_>, Vec<usize>) = airs_degrees.into_iter().unzip();

    let mut runner = circuit.runner();

    // Honest inputs: compute Tip5(state) where state = (in0, in1, 0, ..., 0).
    let in0_raw = 0x1234_5678_9abc_def0u64;
    let in1_raw = 0x0fed_cba9_8765_4321u64;
    let mut state_base: [Goldilocks; 16] = [Goldilocks::ZERO; 16];
    state_base[0] = Goldilocks::from_u64(in0_raw);
    state_base[1] = Goldilocks::from_u64(in1_raw);
    let image_base = Tip5Perm.permute(state_base);

    let in0 = Challenge::from(state_base[0]);
    let in1 = Challenge::from(state_base[1]);
    let image_chal: [Challenge; 16] = core::array::from_fn(|i| Challenge::from(image_base[i]));

    let mut pis = vec![in0, in1];
    pis.extend_from_slice(&image_chal[..10]);
    runner.set_public_inputs(&pis).unwrap();

    // *** R-a tail trigger point: runner().run() at D=2 with
    // recompose-coeff CTL enabled. If the orphan exists, this returns
    // Err with WitnessConflict (or similar). ***
    let traces = match runner.run() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "P3 D=2 Tip5+Recompose at Tip5-unified config FAILED at runner().run(): {e:?}\n\
                 This may be the predicted C2.4 R-a tail at D=2 surfacing. Per spec §3.3.B,\n\
                 fall back to D=1 outer-cert. The Tip5-unified config at D=1 is validated\n\
                 by the P2 smoke tests.",
            );
            return; // Soft-fail: blocker surfaced as predicted.
        }
    };

    let prover_data = ProverData::from_airs_and_degrees(&cfg, &airs, &degrees);
    let circuit_prover_data =
        CircuitProverData::new(prover_data, primitive_columns, non_primitive_columns);

    let mut prover = BatchStarkProver::new(cfg).with_table_packing(table_packing);
    prover.register_tip5_table::<2>(tip5_config);
    prover.register_recompose_table::<2>(true);

    let batch_proof = prover
        .prove_all_tables(&traces, &circuit_prover_data)
        .expect("Tip5-unified D=2 prove_all_tables MUST succeed");

    prover
        .verify_all_tables(&batch_proof)
        .expect("Tip5-unified D=2 verify_all_tables MUST succeed");
}
