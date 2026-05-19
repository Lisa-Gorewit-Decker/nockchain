//! Tip5 permutation executor (C2.3).
//!
//! Faithful mechanical mirror of `Poseidon1PermExecutor`'s **D=1
//! non-merkle base path only** (`execute_base` / `build_base_trace_row`
//! / `init_chain_state` + the compact-D1 preprocessed registration).
//! All merkle / MMCS / D>1 / ext-field code is removed: Tip5 is
//! Goldilocks D=1, width 16, rate 10, sponge/challenger only.
//!
//! The permutation itself is the closure registered via
//! `enable_tip5_perm`, which runs `nockchain_math::tip5::permute`
//! bit-for-bit (its in-crate twin `p3_tip5_circuit_air::tip5_spec::
//! permute`), so the witness is exactly the deployed Tip5.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use p3_field::Field;

use crate::CircuitError;
use crate::ops::tip5_perm::config::{Tip5Config, Tip5PermConfigData, Tip5PermExec};
use crate::ops::tip5_perm::state::Tip5ExecutionState;
use crate::ops::tip5_perm::trace::Tip5CircuitRow;
use crate::ops::{ExecutionContext, NonPrimitiveExecutor, NpoTypeId, PreprocessedWriter};
use crate::types::WitnessId;

/// Runtime executor for a single Tip5 permutation row (D=1, non-merkle).
#[derive(Debug, Clone)]
pub(crate) struct Tip5PermExecutor {
    /// Operation type identifier for config/state lookups.
    op_type: NpoTypeId,
    /// Tip5 parameters (always D=1 width 16 rate 10).
    config: Tip5Config,
    /// When true, this row starts a fresh sponge chain instead of
    /// continuing from the previous permutation output.
    pub(crate) new_start: bool,
}

impl Tip5PermExecutor {
    pub const fn new(op_type: NpoTypeId, config: Tip5Config, new_start: bool) -> Self {
        Self {
            op_type,
            config,
            new_start,
        }
    }

    #[inline]
    const fn limb_ctl_enabled(slot: &[WitnessId]) -> bool {
        !slot.is_empty()
    }

    /// Build the initial permutation state vector.
    ///
    /// - New chain: zero vector of `width` (16) elements.
    /// - Continuation: copies the previous full-state output
    ///   (sponge overwrite mode — full width incl. capacity, matching
    ///   `PaddingFreeSponge`). Mirrors the Poseidon1 D=1 normal path.
    fn init_chain_state<F: Field>(
        &self,
        last_output: Option<&[F]>,
        ctx: &ExecutionContext<'_, F>,
    ) -> Result<Vec<F>, CircuitError> {
        let width = self.config.width();
        let mut resolved = F::zero_vec(width);

        if self.new_start {
            return Ok(resolved);
        }

        let prev = last_output.ok_or_else(|| CircuitError::Tip5ChainMissingPreviousState {
            operation_index: ctx.operation_id(),
        })?;
        let n = width.min(prev.len());
        resolved[..n].copy_from_slice(&prev[..n]);
        Ok(resolved)
    }

    /// Previous permutation output from chain state, if any.
    fn get_chain_output<'a, F: Field + 'static>(
        &self,
        ctx: &'a ExecutionContext<'_, F>,
    ) -> Option<&'a Vec<F>> {
        ctx.get_op_state::<Tip5ExecutionState<F>>(&self.op_type)
            .and_then(|s| s.last_output.as_ref())
    }

    /// Construct the circuit trace row (D=1): one CTL flag + witness
    /// index per physical input slot (`WIDTH`=16) and per rate output
    /// slot (`RATE`=10). Mirrors `build_base_trace_row`.
    fn build_trace_row<F: Field>(
        &self,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        input_values: &[F],
    ) -> Tip5CircuitRow<F> {
        let width = self.config.width();
        let rate = self.config.rate();
        let mut in_ctl = vec![false; width];
        let mut input_indices = vec![0u32; width];
        for i in 0..width {
            if let Some(inp) = inputs.get(i)
                && let [wid] = inp.as_slice()
            {
                in_ctl[i] = true;
                input_indices[i] = wid.0;
            }
        }

        let mut out_ctl = vec![false; rate];
        let mut output_indices = vec![0u32; rate];
        for i in 0..rate {
            if let Some(out_slot) = outputs.get(i)
                && let [wid] = out_slot.as_slice()
            {
                out_ctl[i] = true;
                output_indices[i] = wid.0;
            }
        }

        Tip5CircuitRow {
            new_start: self.new_start,
            input_values: input_values.to_vec(),
            in_ctl,
            input_indices,
            out_ctl,
            output_indices,
        }
    }

    /// Store the permutation output in chain state and append the
    /// trace row. Mirrors `update_chain_state` (normal mode only).
    fn update_chain_state<F: Field + 'static>(
        &self,
        ctx: &mut ExecutionContext<'_, F>,
        output: Vec<F>,
        row: Tip5CircuitRow<F>,
    ) {
        let state = ctx.get_op_state_mut::<Tip5ExecutionState<F>>(&self.op_type);
        state.last_output = Some(output);
        state.rows.push(row);
    }

    /// Write permutation output values to witness slots. Mirrors
    /// `write_outputs`.
    fn write_outputs<F: Field>(
        &self,
        outputs: &[Vec<WitnessId>],
        output_values: &[F],
        ctx: &mut ExecutionContext<'_, F>,
    ) -> Result<(), CircuitError> {
        for (out_slot, &val) in outputs.iter().zip(output_values) {
            match out_slot.as_slice() {
                [] => {}
                [wid] => ctx.set_witness(*wid, val)?,
                _ => {
                    return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                        op: self.op_type.clone(),
                        expected: "0 or 1 witness per output limb".into(),
                        got: out_slot.len(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Execute the D=1 Tip5 permutation. Mirrors `execute_base`:
    /// validate the 16-input / 10-or-16-output layout, resolve witness
    /// values (chain carry then CTL overwrite), run the permutation,
    /// record a trace row, write outputs.
    fn execute_inner<F: Field + Send + Sync + 'static>(
        &self,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        ctx: &mut ExecutionContext<'_, F>,
        exec: &dyn Fn(&[F]) -> Vec<F>,
    ) -> Result<(), CircuitError> {
        let width = self.config.width();
        let rate = self.config.rate();

        if inputs.len() != width {
            return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                op: self.op_type.clone(),
                expected: alloc::format!("{width} input vectors for Tip5 (d=1)"),
                got: inputs.len(),
            });
        }
        for (i, inp) in inputs.iter().enumerate() {
            if inp.len() > 1 {
                return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                    op: self.op_type.clone(),
                    expected: alloc::format!("0 or 1 witness per input element {i}"),
                    got: inp.len(),
                });
            }
        }
        if outputs.len() != rate && outputs.len() != width {
            return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                op: self.op_type.clone(),
                expected: alloc::format!("{rate} or {width} output vectors for Tip5 (d=1)"),
                got: outputs.len(),
            });
        }

        // Initialize from previous chain output (or zeros for
        // new_start), then overwrite with CTL-exposed witnesses.
        let chain_output = self.get_chain_output(ctx);
        let mut resolved_inputs =
            self.init_chain_state(chain_output.map(|v| v.as_slice()), ctx)?;
        for (slot, inp) in resolved_inputs.iter_mut().zip(inputs) {
            if let [wid] = inp.as_slice() {
                *slot = ctx.get_witness(*wid)?;
            }
        }

        let output = exec(&resolved_inputs);
        let row = self.build_trace_row(inputs, outputs, &resolved_inputs);

        self.write_outputs(outputs, &output, ctx)?;
        self.update_chain_state(ctx, output, row);
        Ok(())
    }

    /// Emit the per-op preprocessed CTL columns for the Tip5 circuit
    /// table: `in_ctl[16] | in_idx[16] | out_idx[10] | out_ctl[10]`.
    ///
    /// Input slots that are CTL-exposed are registered as
    /// `WitnessChecks` **reads** (so the bus reader count / `ext_reads`
    /// is incremented, exactly like Poseidon1 input limbs); output
    /// slots are registered as **output indices** (creators whose
    /// `out_ctl` multiplicity the `Tip5Preprocessor` later resolves
    /// from `ext_reads`, exactly like Poseidon1 / Recompose).
    fn preprocess_ctl<F: Field>(
        &self,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        preprocessed: &mut dyn PreprocessedWriter<F>,
    ) -> Result<(), CircuitError> {
        let width = self.config.width();
        let rate = self.config.rate();

        // in_ctl[16]
        for inp in inputs.iter().take(width) {
            preprocessed.register_non_primitive_preprocessed_no_read(
                &self.op_type,
                &[F::from_bool(Self::limb_ctl_enabled(inp))],
            );
        }
        // in_idx[16] — CTL'd inputs are bus readers; empty slots get 0.
        for inp in inputs.iter().take(width) {
            if inp.is_empty() {
                preprocessed
                    .register_non_primitive_preprocessed_no_read(&self.op_type, &[F::ZERO]);
            } else if let [_] = inp.as_slice() {
                preprocessed.register_non_primitive_witness_reads(&self.op_type, inp)?;
            } else {
                return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                    op: self.op_type.clone(),
                    expected: "0 or 1 witness per input limb".into(),
                    got: inp.len(),
                });
            }
        }
        // out_idx[10] — outputs are creators on the WitnessChecks bus.
        for out in outputs.iter().take(rate) {
            if out.is_empty() {
                preprocessed
                    .register_non_primitive_preprocessed_no_read(&self.op_type, &[F::ZERO]);
            } else if let [_] = out.as_slice() {
                preprocessed.register_non_primitive_output_index(&self.op_type, out);
            } else {
                return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                    op: self.op_type.clone(),
                    expected: "0 or 1 witness per output limb".into(),
                    got: out.len(),
                });
            }
        }
        // out_ctl[10] — raw flag; preprocessor rewrites to n_reads / -1.
        for out in outputs.iter().take(rate) {
            preprocessed.register_non_primitive_preprocessed_no_read(
                &self.op_type,
                &[F::from_bool(Self::limb_ctl_enabled(out))],
            );
        }
        Ok(())
    }
}

impl<F: Field + Send + Sync + 'static> NonPrimitiveExecutor<F> for Tip5PermExecutor {
    fn execute(
        &self,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        ctx: &mut ExecutionContext<'_, F>,
    ) -> Result<(), CircuitError> {
        let exec: Tip5PermExec<F> = ctx
            .get_config(&self.op_type)?
            .downcast_ref::<Tip5PermConfigData<F>>()
            .map(|cfg| cfg.exec.clone())
            .ok_or_else(|| CircuitError::InvalidNonPrimitiveOpConfiguration {
                op: self.op_type.clone(),
            })?;
        self.execute_inner(inputs, outputs, ctx, exec.as_ref())
    }

    fn op_type(&self) -> &NpoTypeId {
        &self.op_type
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn num_exposed_outputs(&self) -> Option<usize> {
        Some(self.config.rate())
    }

    fn preprocess(
        &self,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        preprocessed: &mut dyn PreprocessedWriter<F>,
    ) -> Result<(), CircuitError> {
        self.preprocess_ctl(inputs, outputs, preprocessed)
    }

    fn boxed(&self) -> Box<dyn NonPrimitiveExecutor<F>> {
        Box::new(self.clone())
    }
}
