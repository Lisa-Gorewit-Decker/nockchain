//! SwiGLU feed-forward block.
//!
//! ```text
//! gate = activation_lut( requantize( x @ W_gate, gate_scale ) )
//! up   = requantize( x @ W_up, up_scale )
//! mid  = elementwise_mul( gate, up, mid_scale )
//! out  = requantize( mid @ W_down, down_scale )
//! ```
//!
//! Same flow as the standard SwiGLU / GeGLU forward used in Gemma and Qwen,
//! with the activation function replaced by a committed
//! [`crate::activation_lut::ActivationLut`].
//!
//! This module is the per-attempt critical path of the verifiable-inference
//! puzzle: the gate / up matmul `(m=seq_len, k=hidden, n=intermediate)`
//! lines up exactly with `MatmulParams::GEMMA_4_31B_FFN` /
//! `MatmulParams::QWEN_3_6_27B_FFN` from `ai-pow`.

use thiserror::Error;

use crate::activation_lut::ActivationLut;
use crate::matmul_int8::{matmul_int8, requantize_vec, MatmulError};
use crate::quant::{rescale_and_requantize, Scale};

/// Weights for one SwiGLU FFN block. All tensors are INT8; the per-tensor
/// scales travel separately so that requantization happens at well-defined
/// points (after each matmul, after the elementwise multiply).
///
/// `w_gate`, `w_up` are shape `(hidden, intermediate)` column-major.
/// `w_down` is shape `(intermediate, hidden)` column-major. (Same layout
/// convention as [`crate::matmul_int8::matmul_int8`].)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfnWeights {
    pub hidden: u32,
    pub intermediate: u32,
    pub w_gate: Vec<i8>,
    pub w_up: Vec<i8>,
    pub w_down: Vec<i8>,
}

/// Per-tensor quantization scales for one SwiGLU step. Each scale rescales
/// the i32 accumulator coming out of its matmul (or out of the elementwise
/// multiply, in `mid_scale`'s case) back to i8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfnScales {
    pub gate: Scale,
    pub up: Scale,
    pub mid: Scale,
    pub down: Scale,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FfnError {
    #[error("input length must equal m * hidden")]
    BadInputLen,
    #[error("output length must equal m * hidden")]
    BadOutputLen,
    #[error("w_gate length must equal hidden * intermediate")]
    BadGateLen,
    #[error("w_up length must equal hidden * intermediate")]
    BadUpLen,
    #[error("w_down length must equal intermediate * hidden")]
    BadDownLen,
    #[error("matmul: {0}")]
    Matmul(#[from] MatmulError),
}

/// Elementwise multiply of two i8 vectors with a rescale-and-requantize.
/// `out[i] = round(a[i] * b[i] * scale) → i8 (saturated)`.
///
/// The intermediate `a[i] * b[i]` is at most `128*128 = 16384`, fits in
/// `i32`; after multiplying by `scale.num` (i32) we widen to `i64` for the
/// final round-half-to-even shift inside `rescale_and_requantize`.
pub fn elementwise_mul_i8(
    a: &[i8],
    b: &[i8],
    scale: Scale,
    out: &mut [i8],
) -> Result<(), FfnError> {
    if a.len() != b.len() || a.len() != out.len() {
        return Err(FfnError::BadInputLen);
    }
    for ((aa, bb), oo) in a.iter().zip(b.iter()).zip(out.iter_mut()) {
        let prod = (*aa as i32).wrapping_mul(*bb as i32);
        *oo = rescale_and_requantize(prod, scale);
    }
    Ok(())
}

/// SwiGLU forward.
///
/// `input` is `(m, hidden)` row-major i8; `output` is the same shape.
///
/// Allocates three intermediate tensors of size `m * intermediate`
/// internally — at LLM scale this is `4096 * 21504 = 88 MB` per
/// intermediate, comfortable on any modern miner. A streaming variant
/// that processes one row at a time can be added later if memory
/// pressure is an issue.
pub fn ffn_forward(
    input: &[i8],
    weights: &FfnWeights,
    activation: &ActivationLut,
    scales: FfnScales,
    m: u32,
    output: &mut [i8],
) -> Result<(), FfnError> {
    let hidden = weights.hidden;
    let intermediate = weights.intermediate;
    let mu = m as usize;
    let hu = hidden as usize;
    let iu = intermediate as usize;

    if input.len() != mu * hu {
        return Err(FfnError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(FfnError::BadOutputLen);
    }
    if weights.w_gate.len() != hu * iu {
        return Err(FfnError::BadGateLen);
    }
    if weights.w_up.len() != hu * iu {
        return Err(FfnError::BadUpLen);
    }
    if weights.w_down.len() != iu * hu {
        return Err(FfnError::BadDownLen);
    }

    // gate_acc = input @ W_gate  →  (m, intermediate) i32.
    let mut gate_acc = vec![0i32; mu * iu];
    matmul_int8(
        input, &weights.w_gate, m, hidden, intermediate, &mut gate_acc,
    )?;
    // gate = activation_lut(requantize(gate_acc, gate_scale)).
    let mut gate_q = vec![0i8; mu * iu];
    requantize_vec(&gate_acc, scales.gate, &mut gate_q)?;
    activation.apply(&mut gate_q);
    // free gate_acc
    drop(gate_acc);

    // up = requantize(input @ W_up, up_scale).
    let mut up_acc = vec![0i32; mu * iu];
    matmul_int8(input, &weights.w_up, m, hidden, intermediate, &mut up_acc)?;
    let mut up_q = vec![0i8; mu * iu];
    requantize_vec(&up_acc, scales.up, &mut up_q)?;
    drop(up_acc);

    // mid = elementwise_mul(gate, up, mid_scale).
    let mut mid_q = vec![0i8; mu * iu];
    elementwise_mul_i8(&gate_q, &up_q, scales.mid, &mut mid_q)?;
    drop(gate_q);
    drop(up_q);

    // down_acc = mid @ W_down  →  (m, hidden) i32.
    let mut down_acc = vec![0i32; mu * hu];
    matmul_int8(
        &mid_q, &weights.w_down, m, intermediate, hidden, &mut down_acc,
    )?;
    drop(mid_q);
    // output = requantize(down_acc, down_scale).
    requantize_vec(&down_acc, scales.down, output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_lut::{ActivationKind, ActivationLut};
    use crate::quant::SCALE_DENOM_LOG2;

    fn unit_scale() -> Scale {
        Scale::from_num(1 << SCALE_DENOM_LOG2).unwrap()
    }

    fn ffn_scales(s: Scale) -> FfnScales {
        FfnScales {
            gate: s,
            up: s,
            mid: s,
            down: s,
        }
    }

    #[test]
    fn elementwise_mul_zero_is_zero() {
        let a = vec![0i8; 5];
        let b = vec![1i8, 2, 3, 4, 5];
        let mut out = vec![0i8; 5];
        elementwise_mul_i8(&a, &b, unit_scale(), &mut out).unwrap();
        for &v in &out {
            assert_eq!(v, 0);
        }
    }

    #[test]
    fn elementwise_mul_unit_scale_just_clamps() {
        let a = vec![5i8, -10, 100];
        let b = vec![3i8, -2, 4];
        let mut out = vec![0i8; 3];
        elementwise_mul_i8(&a, &b, unit_scale(), &mut out).unwrap();
        assert_eq!(out[0], 15); // 5*3
        assert_eq!(out[1], 20); // -10 * -2
        assert_eq!(out[2], 127); // 100*4 = 400, saturates to 127
    }

    #[test]
    fn elementwise_mul_quarter_scale() {
        // scale = 1/4 (num = 2^13). 100 * 4 = 400; 400/4 = 100.
        let s = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 2)).unwrap();
        let a = vec![100i8];
        let b = vec![4i8];
        let mut out = vec![0i8];
        elementwise_mul_i8(&a, &b, s, &mut out).unwrap();
        assert_eq!(out[0], 100);
    }

    #[test]
    fn elementwise_mul_rejects_len_mismatch() {
        let mut out = vec![0i8; 3];
        assert_eq!(
            elementwise_mul_i8(&[1, 2, 3], &[1, 2], unit_scale(), &mut out).err(),
            Some(FfnError::BadInputLen),
        );
    }

    fn build_test_ffn(hidden: u32, intermediate: u32, seed: u64) -> FfnWeights {
        let mut s = seed;
        let mut step = || -> i8 {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            s.wrapping_shr(56) as i8
        };
        let h = hidden as usize;
        let i = intermediate as usize;
        let w_gate: Vec<i8> = (0..h * i).map(|_| step()).collect();
        let w_up: Vec<i8> = (0..h * i).map(|_| step()).collect();
        let w_down: Vec<i8> = (0..i * h).map(|_| step()).collect();
        FfnWeights {
            hidden,
            intermediate,
            w_gate,
            w_up,
            w_down,
        }
    }

    #[test]
    fn ffn_forward_zero_input_is_zero() {
        let hidden = 8u32;
        let intermediate = 16u32;
        let m = 2u32;
        let weights = build_test_ffn(hidden, intermediate, 0xabcdef);
        let activation = ActivationLut::identity();
        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![1i8; (m * hidden) as usize]; // pre-fill to confirm overwrite
        ffn_forward(
            &input,
            &weights,
            &activation,
            ffn_scales(unit_scale()),
            m,
            &mut output,
        )
        .unwrap();
        for &v in &output {
            assert_eq!(v, 0, "zero input → zero output");
        }
    }

    #[test]
    fn ffn_forward_validates_lengths() {
        let hidden = 8u32;
        let intermediate = 16u32;
        let m = 2u32;
        let weights = build_test_ffn(hidden, intermediate, 0x123);
        let activation = ActivationLut::identity();
        let input = vec![0i8; 7]; // wrong size
        let mut output = vec![0i8; (m * hidden) as usize];
        assert_eq!(
            ffn_forward(
                &input,
                &weights,
                &activation,
                ffn_scales(unit_scale()),
                m,
                &mut output,
            )
            .err(),
            Some(FfnError::BadInputLen),
        );

        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![0i8; 5];
        assert_eq!(
            ffn_forward(
                &input,
                &weights,
                &activation,
                ffn_scales(unit_scale()),
                m,
                &mut output,
            )
            .err(),
            Some(FfnError::BadOutputLen),
        );
    }

    #[test]
    fn ffn_forward_determinism() {
        let hidden = 8u32;
        let intermediate = 16u32;
        let m = 2u32;
        let weights = build_test_ffn(hidden, intermediate, 0xdeadbeef);
        let activation =
            ActivationLut::from_bytes(ActivationKind::SiLU, &(0u8..=255u8).collect::<Vec<_>>())
                .unwrap();
        let input: Vec<i8> = (0..(m * hidden) as i32)
            .map(|i| (i * 7 - 50) as i8)
            .collect();
        let mut a = vec![0i8; (m * hidden) as usize];
        let mut b = vec![0i8; (m * hidden) as usize];
        let scales = FfnScales {
            gate: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
            up: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
            mid: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
            down: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
        };
        ffn_forward(&input, &weights, &activation, scales, m, &mut a).unwrap();
        ffn_forward(&input, &weights, &activation, scales, m, &mut b).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn ffn_with_zeroed_gate_yields_zero_output() {
        // If W_gate is all zeros, the gate matmul yields all zeros. The
        // identity LUT then maps zero → zero. The elementwise multiply
        // with the up tensor is zero. Final down matmul is zero. So output
        // is zero regardless of W_up / W_down.
        let hidden = 8u32;
        let intermediate = 16u32;
        let m = 2u32;
        let mut weights = build_test_ffn(hidden, intermediate, 0xfeed);
        weights.w_gate = vec![0i8; (hidden * intermediate) as usize];
        let activation = ActivationLut::identity();
        let input: Vec<i8> = (0..(m * hidden) as i32)
            .map(|i| (i * 3 - 7) as i8)
            .collect();
        let mut output = vec![0i8; (m * hidden) as usize];
        ffn_forward(
            &input,
            &weights,
            &activation,
            ffn_scales(unit_scale()),
            m,
            &mut output,
        )
        .unwrap();
        for &v in &output {
            assert_eq!(v, 0);
        }
    }
}
