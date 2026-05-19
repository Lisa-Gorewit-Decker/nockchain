//! Tip5 circuit-prover table (C2.3 / M-S4).
//!
//! Faithful mechanical mirror of `batch_stark_prover/poseidon1.rs`,
//! reduced to the **single deployed Goldilocks D=1 width-16 rate-10
//! 7-round Tip5** configuration and the **existing**
//! [`p3_tip5_circuit_air::Tip5CircuitAir`] wrapper (which composes the
//! degree-2-proven [`p3_tip5_circuit_air::Tip5PermLookupAir`] —
//! `tip5_l` LogUp bus + algebraic constraints + verifier-fixed 256-row
//! L-table, reused verbatim — with the `WitnessChecks` cross-table
//! CTL).
//!
//! Non-1:1 deviations vs. the poseidon1 mirror (and exactly why):
//! * **Single Goldilocks variant.** Poseidon1's
//!   `Poseidon1AirWrapperInner` enum + `transmute` machinery exists
//!   only to multiplex BabyBear/KoalaBear/Goldilocks × D × width. Tip5
//!   is Goldilocks-only, D=1-only ⇒ the faithful reduction is one AIR
//!   type (the `RecomposeProver`/`RecomposeAir` single-AIR
//!   `TableProver` shape, which *is* poseidon1.rs's pattern minus the
//!   variant fan-out — same trait set: `TableProver`,
//!   `NpoPreprocessor`, `NpoAirBuilder`, `Register*ForExt`,
//!   `register_*_table`).
//! * **Wrapped AIR is `Tip5CircuitAir`, not a `p3-*-circuit-air`
//!   permutation AIR.** Per spec: do not re-derive the lookup; reuse
//!   the validated `tip5_l` bus + L-table.
//! * **Trace shape.** `generate_lookup_trace` emits 256 L-table rows ++
//!   P perm rows ++ padding (the validated Tip5 layout), not
//!   one-row-per-op. The `D>1` / merkle / mmcs code paths are deleted
//!   (Tip5 has none).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use hashbrown::HashMap;
use p3_batch_stark::{StarkGenericConfig, Val};
use p3_circuit::ops::{NonPrimitivePreprocessedMap, NpoTypeId, Tip5Config, Tip5Trace};
use p3_circuit::tables::Traces;
use p3_circuit::{CircuitError, PreprocessedColumns};
use p3_field::extension::BinomialExtensionField;
use p3_field::{Algebra, ExtensionField, PrimeCharacteristicRing, PrimeField64};
use p3_goldilocks::Goldilocks;
use p3_matrix::Matrix;
use p3_tip5_circuit_air::{
    TABLE_ROWS, TIP5_CIRCUIT_PREP_WIDTH, TIP5_CTL_PREP_COLS, TIP5_RATE, TIP5_WIDTH, Tip5CircuitAir,
    Tip5CircuitRow, build_tip5_circuit_preprocessed, generate_tip5_circuit_main,
    tip5_inputs_from_rows,
};
use p3_uni_stark::{SymbolicExpression, SymbolicExpressionExt};
use p3_util::log2_ceil_usize;

use super::dynamic_air::{
    BatchAir, BatchTableInstance, DynamicAirEntry, TableProver, transmute_traces,
};
use super::{NonPrimitiveTableEntry, TablePacking};
use crate::common::{CircuitTableAir, NpoAirBuilder, NpoPreprocessor};
use crate::config::StarkField;
use crate::constraint_profile::ConstraintProfile;
use crate::impl_table_prover_batch_instances_from_base;

impl<SC> BatchAir<SC> for Tip5CircuitAir<Val<SC>>
where
    SC: StarkGenericConfig + Send + Sync,
    Val<SC>: StarkField,
    SymbolicExpressionExt<Val<SC>, SC::Challenge>:
        Algebra<SymbolicExpression<Val<SC>>> + Algebra<SC::Challenge>,
{
}

/// Per-op preprocessed CTL row width registered by the Tip5 NPO
/// executor (`in_ctl[16] | in_idx[16] | out_idx[10] | out_ctl[10]`).
const TIP5_OP_CTL_WIDTH: usize = TIP5_CTL_PREP_COLS; // 52

/// Table prover for the Tip5 NPO. Single Goldilocks D=1 config.
#[derive(Clone)]
pub struct Tip5Prover {
    config: Tip5Config,
}

unsafe impl Send for Tip5Prover {}
unsafe impl Sync for Tip5Prover {}

impl Tip5Prover {
    pub const fn new(config: Tip5Config, _profile: ConstraintProfile) -> Self {
        Self { config }
    }

    pub(crate) fn tip5_op_type(&self) -> NpoTypeId {
        NpoTypeId::tip5_perm(self.config)
    }

    /// Build the batched table instance from base-field Tip5 traces.
    ///
    /// Mirrors `Poseidon1Prover::batch_instance_base_impl`: regenerate
    /// preprocessed + trace from the captured circuit rows. The
    /// committed-preprocessed override (`air_with_committed_preprocessed`)
    /// then replaces the preprocessed with the `Tip5Preprocessor`
    /// output so the debug lookup check / verifier binding agree.
    fn batch_instance_base<SC>(
        &self,
        _config: &SC,
        packing: &TablePacking,
        traces: &Traces<Val<SC>>,
    ) -> Option<BatchTableInstance<SC>>
    where
        SC: StarkGenericConfig + 'static + Send + Sync,
        Val<SC>: StarkField,
        SymbolicExpressionExt<Val<SC>, SC::Challenge>:
            Algebra<SymbolicExpression<Val<SC>>> + Algebra<SC::Challenge>,
    {
        let op_type = self.tip5_op_type();
        let t = traces.non_primitive_trace::<Tip5Trace<Val<SC>>>(&op_type)?;
        let rows = t.total_rows();
        if rows == 0 {
            return None;
        }

        let min_height = packing.min_trace_height();

        // 1. Main trace via the *unmodified* validated generator.
        let inputs = tip5_inputs_from_rows(&t.operations);
        let (main_g, l_prep_g) = generate_tip5_circuit_main(&inputs);
        let height = main_g.height();
        // Goldilocks-only circuit ⇒ Val<SC> == Goldilocks. Go through
        // the field's canonical-u64 API (no transmute) to copy.
        let width = main_g.width();
        let mut main_vals = Val::<SC>::zero_vec(height * width);
        for (dst, src) in main_vals.iter_mut().zip(main_g.values.iter()) {
            *dst = Val::<SC>::from_u64(Goldilocks::as_canonical_u64(src));
        }
        let matrix = p3_matrix::dense::RowMajorMatrix::new(main_vals, width);

        // 2. Full preprocessed (L-table ++ per-row CTL) at the same
        //    height. (`idx_scale = 1`: base-field circuit, D == 1.)
        let prep =
            build_tip5_circuit_preprocessed::<Val<SC>>(&l_prep_g, &t.operations, height, 1);

        let air = Tip5CircuitAir::<Val<SC>>::new_with_preprocessed(prep, min_height);

        Some(BatchTableInstance {
            op_type,
            air: DynamicAirEntry::new(Box::new(air)),
            trace: matrix,
            public_values: Vec::new(),
            rows,
            lanes: 1,
        })
    }
}

impl<SC> TableProver<SC> for Tip5Prover
where
    SC: StarkGenericConfig + 'static + Send + Sync,
    Val<SC>: StarkField,
    SymbolicExpressionExt<Val<SC>, SC::Challenge>:
        Algebra<SymbolicExpression<Val<SC>>> + Algebra<SC::Challenge>,
{
    fn op_type(&self) -> NpoTypeId {
        self.tip5_op_type()
    }

    impl_table_prover_batch_instances_from_base!(batch_instance_base);

    fn batch_air_from_table_entry(
        &self,
        _config: &SC,
        _degree: usize,
        _circuit_extension_degree: u32,
        _table_entry: &NonPrimitiveTableEntry<SC>,
    ) -> Result<DynamicAirEntry<SC>, String> {
        // Interaction structure (tip5_l bus + 16 input sends + 10
        // output receives, kind-gated) is preprocessed-value
        // independent; the committed preprocessed binding is carried
        // in `common.preprocessed`. Mirror `RecomposeProver`: rebuild
        // with empty preprocessed.
        let air = Tip5CircuitAir::<Val<SC>>::new_with_preprocessed(Vec::new(), 1);
        Ok(DynamicAirEntry::new(Box::new(air)))
    }

    fn air_with_committed_preprocessed(
        &self,
        committed_prep: Vec<Val<SC>>,
        min_height: usize,
        _lanes: usize,
        _circuit_extension_degree: u32,
    ) -> Option<DynamicAirEntry<SC>> {
        let air = Tip5CircuitAir::<Val<SC>>::new_with_preprocessed(committed_prep, min_height);
        Some(DynamicAirEntry::new(Box::new(air)))
    }
}

/// Stateless plugin used for Tip5 preprocessing.
///
/// Faithful mirror of `Poseidon1Preprocessor` / `RecomposePreprocessor`:
/// resolves the per-op `out_ctl` flags to the witness read-count /
/// duplicate multiplicity from `ext_reads` / `dup_npo_outputs`, then
/// **assembles the full circuit preprocessed matrix** (verifier-fixed
/// 256-row L-table ++ per-perm-row CTL ++ power-of-two padding) so the
/// AIR's `preprocessed_trace()` and the committed binding agree.
#[derive(Clone, Default)]
pub struct Tip5Preprocessor;

fn tip5_preprocess_for_prover<F, ExtF, const D: usize>(
    preprocessed: &mut PreprocessedColumns<ExtF, D>,
) -> Result<NonPrimitivePreprocessedMap<F>, CircuitError>
where
    F: StarkField + PrimeField64,
    ExtF: ExtensionField<F>,
{
    let neg_one = F::NEG_ONE;
    let mut out: NonPrimitivePreprocessedMap<F> = HashMap::new();

    for (op_type, prep) in preprocessed.non_primitive.iter() {
        let op_str = op_type.as_str();
        if !op_str.starts_with("tip5_perm/") {
            continue;
        }

        let prep_base: Vec<F> = prep
            .iter()
            .map(|v| v.as_base().ok_or(CircuitError::InvalidPreprocessedValues))
            .collect::<Result<Vec<_>, CircuitError>>()?;

        if !prep_base.len().is_multiple_of(TIP5_OP_CTL_WIDTH) {
            return Err(CircuitError::InvalidPreprocessedValues);
        }
        let num_ops = prep_base.len() / TIP5_OP_CTL_WIDTH;

        // CTL column offsets within one op's 52-col block.
        let in_ctl_off = 0;
        let in_idx_off = in_ctl_off + TIP5_WIDTH;
        let out_idx_off = in_idx_off + TIP5_WIDTH;
        let out_ctl_off = out_idx_off + TIP5_RATE;

        let dup_wids = preprocessed.dup_npo_outputs.get(op_type);

        // Resolve out_ctl → +n_reads (first creator) / -1 (duplicate).
        let mut resolved = prep_base.clone();
        for row in 0..num_ops {
            let base = row * TIP5_OP_CTL_WIDTH;
            for j in 0..TIP5_RATE {
                let ctl = resolved[base + out_ctl_off + j];
                if ctl != F::ZERO {
                    let idx = resolved[base + out_idx_off + j];
                    let out_wid = F::as_canonical_u64(&idx) as usize / D;
                    let is_dup = dup_wids
                        .and_then(|d| d.get(out_wid).copied())
                        .unwrap_or(false);
                    resolved[base + out_ctl_off + j] = if is_dup {
                        neg_one
                    } else {
                        let n = preprocessed.ext_reads.get(out_wid).copied().unwrap_or(0);
                        F::from_u32(n)
                    };
                }
            }
        }

        // Reconstruct circuit rows and assemble the full preprocessed
        // matrix exactly as the prover's `batch_instance_base` does.
        let rows: Vec<Tip5CircuitRow<F>> = (0..num_ops)
            .map(|row| {
                let base = row * TIP5_OP_CTL_WIDTH;
                Tip5CircuitRow {
                    new_start: false, // unused by preprocessed assembly
                    input_values: Vec::new(),
                    in_ctl: (0..TIP5_WIDTH)
                        .map(|i| resolved[base + in_ctl_off + i] != F::ZERO)
                        .collect(),
                    input_indices: (0..TIP5_WIDTH)
                        .map(|i| F::as_canonical_u64(&resolved[base + in_idx_off + i]) as u32)
                        .collect(),
                    // `out_ctl` here already carries the resolved
                    // signed multiplicity; pass it through via the
                    // index/flag channel below.
                    out_ctl: (0..TIP5_RATE).map(|_| false).collect(),
                    output_indices: (0..TIP5_RATE)
                        .map(|i| F::as_canonical_u64(&resolved[base + out_idx_off + i]) as u32)
                        .collect(),
                }
            })
            .collect();

        // Build the L-table preprocessed (value-independent) via the
        // validated generator with zero inputs of the right count, so
        // heights line up.
        let zero_inputs = alloc::vec![[0u64; TIP5_WIDTH]; num_ops];
        let (main_g, l_prep_g) = generate_tip5_circuit_main(&zero_inputs);
        let height = main_g.height();

        // `idx_scale = 1`, NOT `D`. The `input_indices` / `output_
        // indices` reconstructed just above are read from `resolved`
        // (== the circuit-emitted `prep_base`), which the circuit's
        // `generate_preprocessed_columns::<D>()` has **already**
        // D-scaled — that is precisely why `out_wid = idx / D` (above)
        // recovers the witness id, identical to the validated
        // `poseidon1_preprocess_for_prover` / `recompose` paths, which
        // likewise edit the already-scaled `prep_base` in place and
        // never re-scale. Re-applying `D` here would *double-scale*
        // every CTL index (a no-op at D=1 — why this stayed latent —
        // but a `× D` corruption of the `WitnessChecks` bus at D≥2,
        // surfacing as the C2.4 Layer-0 `["0","0"]` net-mult mismatch).
        let mut full =
            build_tip5_circuit_preprocessed::<F>(&l_prep_g, &rows, height, 1);

        // Overwrite the per-perm-row `out_ctl` columns with the
        // resolved signed multiplicities (the assembler wrote raw
        // booleans / zeros for them since we passed `out_ctl=false`).
        let width = TIP5_CIRCUIT_PREP_WIDTH;
        let l_w = width - TIP5_CTL_PREP_COLS;
        let ctl_out_ctl = TIP5_WIDTH + TIP5_WIDTH + TIP5_RATE;
        for (row, op_row) in (0..num_ops).map(|r| (r, r)) {
            let dst = (TABLE_ROWS + row) * width + l_w + ctl_out_ctl;
            let src = op_row * TIP5_OP_CTL_WIDTH + out_ctl_off;
            for j in 0..TIP5_RATE {
                full[dst + j] = resolved[src + j];
            }
        }

        out.insert(op_type.clone(), full);
    }

    Ok(out)
}

impl NpoPreprocessor<Goldilocks> for Tip5Preprocessor {
    fn preprocess(
        &self,
        _circuit: &dyn core::any::Any,
        preprocessed: &mut dyn core::any::Any,
    ) -> Result<NonPrimitivePreprocessedMap<Goldilocks>, CircuitError> {
        if let Some(prep) = preprocessed.downcast_mut::<PreprocessedColumns<Goldilocks, 1>>() {
            return tip5_preprocess_for_prover::<Goldilocks, Goldilocks, 1>(prep);
        }
        if let Some(prep) = preprocessed
            .downcast_mut::<PreprocessedColumns<BinomialExtensionField<Goldilocks, 2>, 2>>()
        {
            return tip5_preprocess_for_prover::<
                Goldilocks,
                BinomialExtensionField<Goldilocks, 2>,
                2,
            >(prep);
        }
        Ok(NonPrimitivePreprocessedMap::new())
    }
}

/// Type-erased Tip5 preprocessor.
pub fn tip5_preprocessor<F>() -> Box<dyn NpoPreprocessor<F>>
where
    F: StarkField + PrimeField64,
    Tip5Preprocessor: NpoPreprocessor<F>,
{
    Box::new(Tip5Preprocessor)
}

/// Tip5 NPO AIR builder (D=1; the deployed base-field Tip5).
#[derive(Clone, Default)]
pub struct Tip5AirBuilder<const D: usize>;

impl<SC> NpoAirBuilder<SC, 1> for Tip5AirBuilder<1>
where
    SC: StarkGenericConfig + 'static + Send + Sync,
    Val<SC>: StarkField,
    SymbolicExpressionExt<Val<SC>, SC::Challenge>:
        Algebra<SymbolicExpression<Val<SC>>> + Algebra<SC::Challenge>,
{
    fn try_build(
        &self,
        op_type: &NpoTypeId,
        prep_base: &[Val<SC>],
        min_height: usize,
        _lanes: usize,
        _constraint_profile: ConstraintProfile,
    ) -> Option<(CircuitTableAir<SC, 1>, usize)> {
        let suffix = op_type.as_str().strip_prefix("tip5_perm/")?;
        let _config = Tip5Config::from_variant_name(suffix)?;

        let air = Tip5CircuitAir::<Val<SC>>::new_with_preprocessed(prep_base.to_vec(), min_height);

        let num_rows = if prep_base.is_empty() {
            0
        } else {
            prep_base.len() / TIP5_CIRCUIT_PREP_WIDTH
        };
        let padded = num_rows
            .max(1)
            .next_power_of_two()
            .max(min_height.next_power_of_two());
        let degree = log2_ceil_usize(padded);

        Some((
            CircuitTableAir::Dynamic(DynamicAirEntry::new(Box::new(air))),
            degree,
        ))
    }
}

impl<SC> NpoAirBuilder<SC, 2> for Tip5AirBuilder<2>
where
    SC: StarkGenericConfig + 'static + Send + Sync,
    Val<SC>: StarkField,
    SymbolicExpressionExt<Val<SC>, SC::Challenge>:
        Algebra<SymbolicExpression<Val<SC>>> + Algebra<SC::Challenge>,
{
    /// Faithful mirror of the D=1 `try_build`. Tip5 is Goldilocks
    /// base-field only; the `Tip5CircuitAir` (`tip5_l` LogUp bus + x⁷
    /// algebraic constraints + the `WitnessChecks` `[idx, value]` CTL)
    /// operates over base `Val<SC>` columns and its interaction
    /// structure is **circuit-extension-degree independent** — unlike
    /// Poseidon1's `Bus1`/`Bus5` witness-dimension variants, the Tip5
    /// AIR pushes the *same* `WitnessChecks` interactions for any
    /// circuit `D`. The D=2 witness-index scaling is carried entirely
    /// by the committed preprocessed produced by the (already-present)
    /// `Tip5Preprocessor` `PreprocessedColumns<BinomialExtensionField<
    /// Goldilocks, 2>, 2>` arm (`out_wid = idx / D`), which *replaces*
    /// the prover's regenerated preprocessed (`air_with_committed_
    /// preprocessed`). Hence the AIR built here is byte-identical to
    /// the D=1 case; only the `CircuitTableAir<SC, 2>` wrapper differs.
    /// No constraint / bus / single-row design is altered.
    fn try_build(
        &self,
        op_type: &NpoTypeId,
        prep_base: &[Val<SC>],
        min_height: usize,
        _lanes: usize,
        _constraint_profile: ConstraintProfile,
    ) -> Option<(CircuitTableAir<SC, 2>, usize)> {
        let suffix = op_type.as_str().strip_prefix("tip5_perm/")?;
        let _config = Tip5Config::from_variant_name(suffix)?;

        let air = Tip5CircuitAir::<Val<SC>>::new_with_preprocessed(prep_base.to_vec(), min_height);

        let num_rows = if prep_base.is_empty() {
            0
        } else {
            prep_base.len() / TIP5_CIRCUIT_PREP_WIDTH
        };
        let padded = num_rows
            .max(1)
            .next_power_of_two()
            .max(min_height.next_power_of_two());
        let degree = log2_ceil_usize(padded);

        Some((
            CircuitTableAir::Dynamic(DynamicAirEntry::new(Box::new(air))),
            degree,
        ))
    }
}

/// Tip5 AIR builders (D=1 — the only deployed Tip5).
pub fn tip5_air_builders<SC, const D: usize>() -> Vec<Box<dyn NpoAirBuilder<SC, D>>>
where
    SC: StarkGenericConfig + 'static + Send + Sync,
    Val<SC>: StarkField,
    SymbolicExpressionExt<Val<SC>, SC::Challenge>:
        Algebra<SymbolicExpression<Val<SC>>> + Algebra<SC::Challenge>,
    Tip5AirBuilder<D>: NpoAirBuilder<SC, D>,
{
    alloc::vec![Box::new(Tip5AirBuilder)]
}

/// The Tip5 verifier AIR (preprocessed-independent interaction
/// structure), mirroring `poseidon1_verifier_air_from_config`.
pub fn tip5_verifier_air_from_config<SC>(_config: Tip5Config) -> Tip5CircuitAir<Val<SC>>
where
    SC: StarkGenericConfig,
    Val<SC>: StarkField,
{
    Tip5CircuitAir::<Val<SC>>::new_with_preprocessed(Vec::new(), 1)
}
