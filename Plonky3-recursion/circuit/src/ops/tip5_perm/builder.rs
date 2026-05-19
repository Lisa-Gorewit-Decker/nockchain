//! Inherent `CircuitBuilder` method for adding Tip5 permutation rows
//! (C2.3). Mirrors `add_poseidon1_perm_base` (the D=1 base path).

use alloc::vec;
use alloc::vec::Vec;

use p3_field::Field;

use crate::CircuitBuilderError;
use crate::builder::{CircuitBuilder, NonPrimitiveOpParams};
use crate::ops::NpoTypeId;
use crate::ops::tip5_perm::call::Tip5PermCall;
use crate::types::{ExprId, NonPrimitiveOpId};

impl<F: Field> CircuitBuilder<F> {
    /// Add a Tip5 perm row (one 7-round Tip5 permutation), D=1 base
    /// field. Mirrors `add_poseidon1_perm_base`.
    ///
    /// Returns `(op_id, outputs)` where `outputs` is `[Option<ExprId>;
    /// 16]`:
    /// - `outputs[0..10]`: present if `out_ctl[i]` is true
    ///   (CTL-verified, rate elements)
    /// - `outputs[10..16]`: present if `return_all_outputs` is true
    ///   (capacity, not CTL-verified)
    pub fn add_tip5_perm(
        &mut self,
        call: &Tip5PermCall,
    ) -> Result<(NonPrimitiveOpId, [Option<ExprId>; 16]), CircuitBuilderError> {
        let op_type = NpoTypeId::tip5_perm(call.config);
        self.ensure_op_enabled(&op_type)?;

        if call.config.d() != 1 {
            return Err(CircuitBuilderError::Tip5ConfigMismatch {
                expected: "D=1 configuration".into(),
                got: alloc::format!("D={} configuration", call.config.d()),
            });
        }

        let input_exprs: [Vec<ExprId>; 16] = call
            .inputs
            .map(|opt| opt.map_or_else(Vec::new, |v| vec![v]));

        let output_labels: [Option<&'static str>; 16] = core::array::from_fn(|i| match i {
            0..10 if call.out_ctl[i] => Some("tip5_perm_out"),
            10..16 if call.return_all_outputs => Some("tip5_perm_out_capacity"),
            _ => None,
        });

        let (op_id, _call_expr_id, outputs) = self.push_non_primitive_op_with_outputs(
            op_type,
            input_exprs.into(),
            output_labels.into(),
            Some(NonPrimitiveOpParams::Tip5Perm {
                new_start: call.new_start,
            }),
            "tip5_perm",
        );

        let outputs: [Option<ExprId>; 16] = outputs
            .try_into()
            .expect("push_non_primitive_op_with_outputs must return exactly 16 outputs");
        Ok((op_id, outputs))
    }
}
