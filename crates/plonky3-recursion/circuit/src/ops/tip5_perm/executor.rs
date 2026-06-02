//! Tip5 permutation executor (C2.3).
//!
//! Faithful mechanical mirror of `Poseidon1PermExecutor`'s **D=1
//! path**, both the non-merkle base path (`execute_base` /
//! `build_base_trace_row` / `init_chain_state` + the compact-D1
//! preprocessed registration) **and** the merkle / MMCS path
//! (`init_chain_state` rate-only carry, `fill_sibling_data` into the
//! capacity portion, `apply_merkle_swap` by the direction bit,
//! `resolve_private_data` / `resolve_mmcs_bit`). The only genuine
//! differences vs the Poseidon1 mirror are the Tip5 numbers
//! (rate 10, capacity 6, width 16, digest 5, Goldilocks-only) and
//! that the Tip5 wrapper AIR is the validated single-row
//! `Tip5CircuitAir` — so the executor resolves the full 16-element
//! state (chain ⊕ sibling ⊕ swap) and feeds it to the permutation,
//! exactly as the Poseidon1 D=1 executor does; the `IN`/`ROUT` rate
//! slots are CTL-bound by the `WitnessChecks` bus and the in-circuit
//! MMCS root is `connect`-bound to the claimed root.
//!
//! The permutation itself is the closure registered via
//! `enable_tip5_perm`, which runs `nockchain_math::tip5::permute`
//! bit-for-bit (its in-crate twin `p3_tip5_circuit_air::tip5_spec::
//! permute`), so the witness is exactly the deployed Tip5 and the
//! in-circuit MMCS matches native
//! `MerkleTreeMmcs<Goldilocks, _, PaddingFreeSponge<Tip5Perm,16,10,5>,
//! TruncatedPermutation<Tip5Perm,2,5,16>, …>` bit-for-bit.

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use p3_field::Field;

use crate::CircuitError;
use crate::ops::tip5_perm::config::{Tip5Config, Tip5PermConfigData, Tip5PermExec};
use crate::ops::tip5_perm::state::{Tip5ExecutionState, Tip5PermPrivateData};
use crate::ops::tip5_perm::trace::Tip5CircuitRow;
use crate::ops::{ExecutionContext, NonPrimitiveExecutor, NpoTypeId, PreprocessedWriter};
use crate::types::WitnessId;

/// Runtime executor for a single Tip5 permutation row.
///
/// Handles the D=1 non-merkle base path (sponge / challenger) and the
/// D=1 merkle / MMCS path (sibling-into-capacity + direction-bit
/// swap), exactly like `Poseidon1PermExecutor` for its D=1 modes.
#[derive(Debug, Clone)]
pub(crate) struct Tip5PermExecutor {
    /// Operation type identifier for config/state lookups.
    op_type: NpoTypeId,
    /// Tip5 parameters (always D=1 width 16 rate 10).
    config: Tip5Config,
    /// When true, this row starts a fresh sponge / Merkle chain
    /// instead of continuing from the previous permutation output.
    pub(crate) new_start: bool,
    /// When true, the executor arranges inputs for Merkle-path
    /// verification and conditionally swaps the rate halves based on
    /// the direction bit (mirrors Poseidon1's merkle path).
    pub(crate) merkle_path: bool,
}

impl Tip5PermExecutor {
    pub const fn new(
        op_type: NpoTypeId,
        config: Tip5Config,
        new_start: bool,
        merkle_path: bool,
    ) -> Self {
        Self {
            op_type,
            config,
            new_start,
            merkle_path,
        }
    }

    #[inline]
    const fn limb_ctl_enabled(slot: &[WitnessId]) -> bool {
        !slot.is_empty()
    }

    /// Whether any of the first `rate` output slots is CTL-exposed on
    /// the `WitnessChecks` bus (a single wid). Exactly the predicate
    /// `build_trace_row` uses to set `out_ctl[i]` / `preprocess_ctl`
    /// uses for `out_idx`. Used by `execute_mmcs` to detect the
    /// chained-only / leaf-compress merkle perms (no CTL output) whose
    /// trace row must carry the **pre-merkle-swap** bus state
    /// (2026-05-19_C3_OUTER_CERT_DESIGN.md §13).
    #[inline]
    fn has_ctl_output(&self, outputs: &[Vec<WitnessId>]) -> bool {
        outputs
            .iter()
            .take(self.config.rate())
            .any(|o| Self::limb_ctl_enabled(o))
    }

    /// Build the initial permutation state vector. Mirrors
    /// `Poseidon1PermExecutor::init_chain_state`.
    ///
    /// - New chain: zero vector of `width` (16) elements.
    /// - Continuation, non-merkle: copies the previous full-state
    ///   output (sponge overwrite mode — full width incl. capacity,
    ///   matching `PaddingFreeSponge`).
    /// - Continuation, merkle: copies only the previous rate portion
    ///   forward (the running digest), exactly like Poseidon1's
    ///   merkle branch.
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

        if self.merkle_path {
            // Merkle / MMCS compress: only the running *digest*
            // (`digest_ext` = 5) is carried forward into the first 5
            // state slots — exactly native
            // `TruncatedPermutation<Tip5Perm,2,5,16>`, which builds a
            // fresh `[d0(0..5), d1(5..10), 0(10..16)]` state per
            // compress (capacity zero, not chained). Mirrors the
            // Poseidon1 D=1 merkle branch, where `digest_ext ==
            // rate_ext`, so this is the faithful generalisation.
            let n = self.config.digest_ext().min(prev.len());
            resolved[..n].copy_from_slice(&prev[..n]);
        } else {
            let n = width.min(prev.len());
            resolved[..n].copy_from_slice(&prev[..n]);
        }
        Ok(resolved)
    }

    /// Copy the sibling digest into the second digest slot of the
    /// state. Faithful generalisation of
    /// `Poseidon1PermExecutor::fill_sibling_data` to `digest ≠ rate`:
    /// native `TruncatedPermutation<Tip5Perm,2,5,16>` places the
    /// sibling 5-element digest at `[digest, 2·digest)` = `[5, 10)`
    /// (Poseidon's `[rate, width)` is the same slice because there
    /// `digest == rate` and `2·rate == width`).
    fn fill_sibling_data<F: Field>(&self, state: &mut [F], private: Option<&[F]>) {
        if let Some(private) = private
            && self.merkle_path
        {
            let digest_ext = self.config.digest_ext();
            let n = private.len().min(digest_ext);
            state[digest_ext..digest_ext + n].copy_from_slice(&private[..n]);
        }
    }

    /// Swap the two digest halves of the state in-place when in Merkle
    /// mode and the direction bit is set. Faithful generalisation of
    /// `Poseidon1PermExecutor::apply_merkle_swap` to `digest ≠ rate`.
    ///
    /// native `TruncatedPermutation<Tip5Perm,2,5,16>` places the two
    /// 5-element digests in state slots `[0,5)` and `[5,10)`;
    /// direction bit = 1 (right child) means the running digest is
    /// the *second* operand, so the executor swaps `[0,digest)` ↔
    /// `[digest,2·digest)` before the permutation — identical to the
    /// Poseidon1 D=1 merkle swap (there `digest == rate`).
    fn apply_merkle_swap<F: Field>(&self, state: &mut [F], mmcs_bit: bool) {
        if self.merkle_path && mmcs_bit {
            let digest_ext = self.config.digest_ext();
            for i in 0..digest_ext {
                state.swap(i, digest_ext + i);
            }
        }
    }

    /// Overwrite state elements with witness values from CTL-exposed
    /// input slots. Mirrors `Poseidon1PermExecutor::apply_witness_values`.
    fn apply_witness_values<F: Field>(
        &self,
        state: &mut [F],
        inputs: &[Vec<WitnessId>],
        ctx: &ExecutionContext<'_, F>,
    ) -> Result<(), CircuitError> {
        for (slot, inp) in state[..self.config.width()].iter_mut().zip(inputs) {
            if let [wid] = inp.as_slice() {
                *slot = ctx.get_witness(*wid)?;
            }
        }
        Ok(())
    }

    /// Extract Merkle sibling data from the operation's private
    /// payload. Mirrors `Poseidon1PermExecutor::resolve_private_data`.
    fn resolve_private_data<'a, F: Field + 'static>(
        &self,
        ctx: &'a ExecutionContext<'_, F>,
    ) -> Result<Option<&'a [F]>, CircuitError> {
        let Ok(private_data) = ctx.get_private_data() else {
            return Ok(None);
        };
        let Some(data) = private_data.downcast_ref::<Tip5PermPrivateData<F>>() else {
            return Ok(None);
        };
        if !self.merkle_path {
            return Err(CircuitError::IncorrectNonPrimitiveOpPrivateData {
                op: self.op_type.clone(),
                operation_index: ctx.operation_id(),
                expected: "no private data (only Merkle mode accepts private data)".to_string(),
                got: "private data provided for non-Merkle operation".to_string(),
            });
        }
        Ok(Some(data.sibling.as_slice()))
    }

    /// Read the MMCS direction bit from the witness table. Mirrors
    /// `Poseidon1PermExecutor::resolve_mmcs_bit` (the value must be
    /// boolean; missing in merkle mode is an error).
    fn resolve_mmcs_bit<F: Field>(
        &self,
        inputs: &[Vec<WitnessId>],
        ctx: &ExecutionContext<'_, F>,
    ) -> Result<bool, CircuitError> {
        let width_ext = self.config.width_ext();
        if let Some(&wid) = inputs[width_ext + 1].first() {
            let val = ctx.get_witness(wid)?;
            match val {
                v if v == F::ZERO => Ok(false),
                v if v == F::ONE => Ok(true),
                v => Err(CircuitError::IncorrectNonPrimitiveOpPrivateData {
                    op: self.op_type.clone(),
                    operation_index: ctx.operation_id(),
                    expected: "boolean mmcs_bit (0 or 1)".into(),
                    got: alloc::format!("{v:?}"),
                }),
            }
        } else if self.merkle_path {
            Err(CircuitError::IncorrectNonPrimitiveOpPrivateData {
                op: self.op_type.clone(),
                operation_index: ctx.operation_id(),
                expected: "mmcs_bit must be provided when merkle_path=true".into(),
                got: "missing mmcs_bit".into(),
            })
        } else {
            Ok(false)
        }
    }

    /// Previous permutation output from chain state, if any. Mirrors
    /// `Poseidon1PermExecutor::get_chain_output` (separate normal /
    /// merkle slots).
    fn get_chain_output<'a, F: Field + 'static>(
        &self,
        ctx: &'a ExecutionContext<'_, F>,
    ) -> Option<&'a Vec<F>> {
        ctx.get_op_state::<Tip5ExecutionState<F>>(&self.op_type)
            .and_then(|s| {
                if self.merkle_path {
                    s.last_output_merkle.as_ref()
                } else {
                    s.last_output_normal.as_ref()
                }
            })
    }

    /// Construct the circuit trace row (D=1): one CTL flag + witness
    /// index per physical input slot (`WIDTH`=16) and per rate output
    /// slot (`RATE`=10). Mirrors `build_base_trace_row` — the
    /// single-row Tip5 AIR consumes the fully resolved `input_values`
    /// (post chain ⊕ sibling ⊕ swap) and the CTL flags only; merkle /
    /// mmcs_bit are executor-internal (no AIR columns), exactly like
    /// the Poseidon1 D=1 compact path feeds a resolved IN.
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
    /// trace row. Mirrors `Poseidon1PermExecutor::update_chain_state`
    /// (writes to the merkle or normal slot per this executor's mode).
    fn update_chain_state<F: Field + 'static>(
        &self,
        ctx: &mut ExecutionContext<'_, F>,
        output: Vec<F>,
        row: Tip5CircuitRow<F>,
    ) {
        let state = ctx.get_op_state_mut::<Tip5ExecutionState<F>>(&self.op_type);
        if self.merkle_path {
            state.last_output_merkle = Some(output);
        } else {
            state.last_output_normal = Some(output);
        }
        state.rows.push(row);
    }

    /// Write permutation output values to witness slots. Mirrors
    /// `Poseidon1PermExecutor::write_outputs`.
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

    /// Execute the D=1 Tip5 permutation, non-merkle base path. Mirrors
    /// `Poseidon1PermExecutor::execute_base`: validate the 16-input /
    /// 10-or-16-output layout (or the `width_ext+2` layout with empty
    /// MMCS tail, exactly like Poseidon1), resolve witness values
    /// (chain carry then CTL overwrite), run the permutation, record a
    /// trace row, write outputs.
    fn execute_base<F: Field + Send + Sync + 'static>(
        &self,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        ctx: &mut ExecutionContext<'_, F>,
        exec: &dyn Fn(&[F]) -> Vec<F>,
    ) -> Result<(), CircuitError> {
        let width = self.config.width();
        let width_ext = self.config.width_ext();
        let rate = self.config.rate();

        let limbs: &[Vec<WitnessId>] = match inputs.len() {
            n if n == width => inputs,
            n if n == width_ext + 2 => {
                for (i, slot) in inputs[width_ext..].iter().enumerate() {
                    if !slot.is_empty() {
                        return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                            op: self.op_type.clone(),
                            expected: alloc::format!(
                                "empty mmcs slots for Tip5 D=1 non-Merkle (tail slot {i})"
                            ),
                            got: slot.len(),
                        });
                    }
                }
                &inputs[..width]
            }
            got => {
                return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                    op: self.op_type.clone(),
                    expected: alloc::format!(
                        "{width} or {} input vectors for Tip5 (d=1)",
                        width_ext + 2
                    ),
                    got,
                });
            }
        };

        for (i, inp) in limbs.iter().enumerate() {
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
        for (slot, inp) in resolved_inputs.iter_mut().zip(limbs) {
            if let [wid] = inp.as_slice() {
                *slot = ctx.get_witness(*wid)?;
            }
        }

        let output = exec(&resolved_inputs);
        let row = self.build_trace_row(limbs, outputs, &resolved_inputs);

        self.write_outputs(outputs, &output, ctx)?;
        self.update_chain_state(ctx, output, row);
        Ok(())
    }

    /// Validate the merkle / MMCS-path input layout. Mirrors
    /// `Poseidon1PermExecutor::validate_ext_inputs` reduced to D=1:
    /// exactly `width_ext + 2` slots (16 limb slots + mmcs_index_sum +
    /// mmcs_bit), each with 0 or 1 witness.
    fn validate_mmcs_inputs(&self, inputs: &[Vec<WitnessId>]) -> Result<(), CircuitError> {
        let width_ext = self.config.width_ext();
        let expected_inputs = width_ext + 2;
        if inputs.len() != expected_inputs {
            return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                op: self.op_type.clone(),
                expected: alloc::format!("{expected_inputs} input vectors"),
                got: inputs.len(),
            });
        }
        for limb_inputs in inputs[..width_ext].iter() {
            if limb_inputs.len() > 1 {
                return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                    op: self.op_type.clone(),
                    expected: "0 or 1 witness per input limb".to_string(),
                    got: limb_inputs.len(),
                });
            }
        }
        if inputs[width_ext].len() > 1 {
            return Err(CircuitError::IncorrectNonPrimitiveOpPrivateDataSize {
                op: self.op_type.clone(),
                expected: "0 or 1 element for mmcs_index_sum".to_string(),
                got: inputs[width_ext].len(),
            });
        }
        if inputs[width_ext + 1].len() > 1 {
            return Err(CircuitError::IncorrectNonPrimitiveOpPrivateDataSize {
                op: self.op_type.clone(),
                expected: "0 or 1 element for mmcs_bit".to_string(),
                got: inputs[width_ext + 1].len(),
            });
        }
        Ok(())
    }

    /// Validate the merkle / MMCS-path output layout: `rate` (10) or
    /// `width` (16) output vectors. Mirrors `validate_ext_outputs`.
    fn validate_mmcs_outputs(&self, outputs: &[Vec<WitnessId>]) -> Result<(), CircuitError> {
        let rate = self.config.rate();
        let width = self.config.width();
        if outputs.len() != rate && outputs.len() != width {
            return Err(CircuitError::NonPrimitiveOpLayoutMismatch {
                op: self.op_type.clone(),
                expected: alloc::format!("{rate} or {width} output vectors"),
                got: outputs.len(),
            });
        }
        Ok(())
    }

    /// Execute the D=1 Tip5 permutation, merkle / MMCS path. Faithful
    /// mirror of the `Poseidon1PermExecutor::execute` D=1 merkle
    /// branch:
    /// 1. start from zeros (new chain) or the previous *rate* output,
    /// 2. place sibling limbs in the capacity portion,
    /// 3. overwrite with any CTL-exposed witness values,
    /// 4. swap the rate halves when the direction bit is set,
    /// 5. run the permutation and record the result.
    fn execute_mmcs<F: Field + Send + Sync + 'static>(
        &self,
        inputs: &[Vec<WitnessId>],
        outputs: &[Vec<WitnessId>],
        ctx: &mut ExecutionContext<'_, F>,
        exec: &dyn Fn(&[F]) -> Vec<F>,
    ) -> Result<(), CircuitError> {
        self.validate_mmcs_inputs(inputs)?;
        self.validate_mmcs_outputs(outputs)?;

        let private_inputs = self.resolve_private_data(ctx)?;
        let mmcs_bit = self.resolve_mmcs_bit(inputs, ctx)?;
        let chain_output = self.get_chain_output(ctx);

        let mut state = self.init_chain_state(chain_output.map(|v| v.as_slice()), ctx)?;
        self.fill_sibling_data(&mut state, private_inputs);
        self.apply_witness_values(&mut state, inputs, ctx)?;

        // 2026-05-19_C3_OUTER_CERT_DESIGN.md §13 (DT-4, debug-confirmed) — fix the
        // merkle-swap slot↔idx desync that orphaned the D≥2 outer-cert
        // `WitnessChecks` global-sum. `apply_merkle_swap` exchanges the
        // digest halves `[0,digest)`↔`[digest,2·digest)` by the runtime
        // `mmcs_bit`, but `build_trace_row` / `preprocess_ctl` record
        // `input_indices` from the **static pre-swap** slot order. With
        // the post-swap state in the trace row, slot `i`'s recorded
        // `input_indices[i] = wid.0` (pre-swap) is paired with
        // `input_values[i]` (post-swap, = the *sibling* value at slot
        // `i` after a `mmcs_bit==1` swap). For a chained-only /
        // leaf-compress merkle perm (no CTL output: it feeds only the
        // running digest, not the witness bus) that perm's INPUT_LIMB
        // bus tuple then becomes `(idx = wid, value = sibling)` — a
        // value that wid does not hold — so it cannot cancel the
        // upstream perm-A OUTPUT_LIMB producer `(idx = wid, value =
        // get_witness(wid))`, leaving the observed lone +1 at
        // `idx = wid·D`. Recording the **pre-swap** `bus_state` makes
        // perm-B's INPUT_LIMB carry `get_witness(inputs[i])` (= perm-A's
        // challenger/MMCS-bound output), so the multiset cancels
        // *because* the duplex `connect` + verbatim
        // `Tip5PermLookupAir` x⁷/`tip5_l` constraints + the
        // challenger/MMCS recompute bind it — net-0 as a *consequence
        // of the binding*, NO multiplicity changed. `exec` /
        // `write_outputs` / `update_chain_state` (the Merkle-root
        // binding, untouched in `mmcs.rs`) all keep using the
        // **post-swap** state, so the native compression / root chain
        // is bit-for-bit unchanged. Only the desync class
        // (`!has_ctl_output`: leaf/chained merkle perms) takes the
        // pre-swap row; CTL-output perms keep the post-swap row exactly
        // as before (their output is the bus value, no input-side
        // desync).
        let bus_state = state.clone();
        self.apply_merkle_swap(&mut state, mmcs_bit);

        let output = exec(&state);
        let trace_state = if self.has_ctl_output(outputs) {
            &state
        } else {
            &bus_state
        };
        let row = self.build_trace_row(inputs, outputs, trace_state);
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
    /// from `ext_reads`, exactly like Poseidon1 / Recompose). Only the
    /// first `width` (16) input slots and `rate` (10) output slots are
    /// registered; any trailing `mmcs_index_sum` / `mmcs_bit` slots
    /// from the merkle layout are not CTL columns of the single-row
    /// Tip5 AIR (the executor consumes the resolved state instead).
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

        // Non-merkle base path: `width` limbs (or `width_ext+2` with
        // empty MMCS tail). Merkle path: full `width_ext+2` layout
        // with sibling private data + direction-bit swap.
        if self.merkle_path {
            self.execute_mmcs(inputs, outputs, ctx, exec.as_ref())
        } else {
            self.execute_base(inputs, outputs, ctx, exec.as_ref())
        }
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
