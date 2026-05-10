//! Mamba-style SSM (selective state-space) forward for the Qwen 3.6 27B
//! hybrid block (Phase 2.13).
//!
//! Conceptual structure (per layer):
//! ```text
//! x_conv[t, c]  = causal_conv1d(x[..t+1, c], kernel=ssm_conv1d[:, c])  // depthwise
//! α[t, v]       = sigmoid_lut(rescale( x_conv @ W_α [t, v] ))           // per V-head gate
//! β[t, v]       = sigmoid_lut(rescale( x_conv @ W_β [t, v] ))           // per V-head gate
//! decay[t, v]   = saturate_i8( (α[t, v] * ssm_a[v]) >> 7 )
//! update[t, v]  = saturate_i8( (β[t, v] * ssm_dt[v]) >> 7 )
//! s[v, d]       = saturate( rescale(s[v, d] * decay[t, v], decay_scale)
//!                         + rescale(update[t, v] * x_conv[t, c(v,d)], update_scale) )
//! y[t, v, :]    = rmsnorm(s[v, :], ssm_norm_gamma) → requantize
//! out[t, :]     = y[t, :] @ ssm_out  → requantize
//! ```
//!
//! `c(v, d) = (v * head_dim + d) % hidden` is a deterministic, total
//! mapping from per-V-head channel `(v, d)` to a `hidden`-width column of
//! `x_conv`. It avoids needing an explicit per-V-head V-projection
//! tensor while keeping the algorithm well-defined when `hidden` does not
//! divide `num_v_heads * head_dim`.
//!
//! Determinism rules:
//! - Reductions are row-major, ascending index.
//! - State `s` lives in i8 between tokens — bounded growth via the same
//!   pattern DeltaNet uses.
//! - `α`, `β` come from a committed sigmoid LUT (an [`ActivationLut`]) —
//!   no `expf` on the consensus path.
//! - The conv mask zeros taps past the prefix bound (i.e. `t - k < 0`
//!   contributes zero to the convolution).
//! - All rescales use [`crate::quant::rescale_and_requantize`] — banker's
//!   rounding, `#[inline(never)]` guard.
//!
//! This is a *self-consistent* INT8 SSM, not a bit-exact match of any
//! particular bf16 reference. The architecture passes the same tensor
//! parameters as the underlying GGUF model (so a calibrated quantizer can
//! pick scales that recover the model's behavior); the verifier replays
//! exactly the integer arithmetic specified above.

use thiserror::Error;

use crate::activation_lut::ActivationLut;
use crate::deltanet::DeltaNetScales;
use crate::matmul_int8::{matmul_int8, matmul_int8_requant, requantize_vec, MatmulError};
use crate::quant::{rescale_and_requantize, saturate_i8, Scale};
use crate::rmsnorm::{rmsnorm, RmsNormError};

/// Borrowed inputs to one [`ssm_forward`] call. Every slice is i8 in the
/// crate's standard column-major (or flat) layout. The caller owns the
/// underlying storage; this struct only keeps references for the duration
/// of the forward.
#[derive(Debug, Clone, Copy)]
pub struct SsmOpts<'a> {
    /// Per-V-head state-transition diagonal, length `num_v_heads`.
    pub ssm_a: &'a [i8],
    /// Decay-gate projection, `(hidden, num_v_heads)` col-major.
    pub ssm_alpha: &'a [i8],
    /// Update-gate projection, `(hidden, num_v_heads)` col-major.
    pub ssm_beta: &'a [i8],
    /// 1D causal conv kernel, `(kernel_size, hidden)` row-major:
    /// `ssm_conv1d[k * hidden + c]` is tap `k` for channel `c`. Tap `k=0`
    /// applies to the *current* token; tap `k=1` to the previous; etc.
    pub ssm_conv1d: &'a [i8],
    /// Per-V-head time-step bias, length `num_v_heads`. Multiplies `β` to
    /// produce the discrete update gate.
    pub ssm_dt: &'a [i8],
    /// Pre-output RMSNorm gamma, length `head_dim`.
    pub ssm_norm_gamma: &'a [i8],
    /// Integer eps_q for the SSM RMSNorm. Default [`crate::rmsnorm::DEFAULT_EPS_Q`].
    pub ssm_norm_eps_q: i64,
    /// Rescale RMSNorm i32 output back to i8 before output projection.
    pub ssm_norm_post_scale: Scale,
    /// Output projection, `(num_v_heads * head_dim, hidden)` col-major.
    pub ssm_out: &'a [i8],
    /// Number of V heads (= per-V-head state matrices).
    pub num_v_heads: u32,
    /// Per-V-head state width.
    pub head_dim: u32,
    /// Causal-conv kernel size.
    pub kernel_size: u32,
    /// Repurposed [`DeltaNetScales`]:
    /// - `q` → conv1d output requantize.
    /// - `alpha_logit` → α-projection requantize (pre sigmoid).
    /// - `beta_logit` → β-projection requantize (pre sigmoid).
    /// - `decay` → state-decay term scale (`s * decay_factor → i8`).
    /// - `update` → state-update term scale (`x_conv * update_factor → i8`).
    /// - `proj` → output projection requantize.
    /// - `k`, `v`, `u`, `o` → reserved (must be present, but not used).
    pub scales: DeltaNetScales,
    /// LUT used for both α and β sigmoids.
    pub sigmoid_lut: &'a ActivationLut,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SsmError {
    #[error("input length must equal m * hidden")]
    BadInputLen,
    #[error("output length must equal m * hidden")]
    BadOutputLen,
    #[error("ssm_a length must equal num_v_heads")]
    BadSsmALen,
    #[error("ssm_alpha length must equal hidden * num_v_heads")]
    BadSsmAlphaLen,
    #[error("ssm_beta length must equal hidden * num_v_heads")]
    BadSsmBetaLen,
    #[error("ssm_conv1d length must equal kernel_size * hidden")]
    BadSsmConvLen,
    #[error("ssm_dt length must equal num_v_heads")]
    BadSsmDtLen,
    #[error("ssm_norm_gamma length must equal head_dim")]
    BadSsmNormGammaLen,
    #[error("ssm_out length must equal (num_v_heads * head_dim) * hidden")]
    BadSsmOutLen,
    #[error("dimensions must be > 0")]
    ZeroDim,
    #[error("matmul: {0}")]
    Matmul(#[from] MatmulError),
    #[error("rmsnorm: {0}")]
    Rms(#[from] RmsNormError),
}

/// Full SSM forward over `m` tokens.
///
/// `input` is `(m, hidden)` row-major i8. `output` is the same shape. The
/// internal state matrices are zero-initialized at the start of every call,
/// so the function is "stateless" from the caller's perspective.
pub fn ssm_forward(
    input: &[i8],
    hidden: u32,
    m: u32,
    opts: SsmOpts,
    output: &mut [i8],
) -> Result<(), SsmError> {
    let num_v = opts.num_v_heads;
    let hd = opts.head_dim;
    let k_size = opts.kernel_size;

    if hidden == 0 || num_v == 0 || hd == 0 || k_size == 0 || m == 0 {
        return Err(SsmError::ZeroDim);
    }
    let mu = m as usize;
    let hu = hidden as usize;
    let nv = num_v as usize;
    let hdu = hd as usize;
    let ksz = k_size as usize;

    if input.len() != mu * hu {
        return Err(SsmError::BadInputLen);
    }
    if output.len() != mu * hu {
        return Err(SsmError::BadOutputLen);
    }
    if opts.ssm_a.len() != nv {
        return Err(SsmError::BadSsmALen);
    }
    if opts.ssm_alpha.len() != hu * nv {
        return Err(SsmError::BadSsmAlphaLen);
    }
    if opts.ssm_beta.len() != hu * nv {
        return Err(SsmError::BadSsmBetaLen);
    }
    if opts.ssm_conv1d.len() != ksz * hu {
        return Err(SsmError::BadSsmConvLen);
    }
    if opts.ssm_dt.len() != nv {
        return Err(SsmError::BadSsmDtLen);
    }
    if opts.ssm_norm_gamma.len() != hdu {
        return Err(SsmError::BadSsmNormGammaLen);
    }
    if opts.ssm_out.len() != nv * hdu * hu {
        return Err(SsmError::BadSsmOutLen);
    }

    // Step 1: depthwise causal 1D conv. Tap `k=0` is the current token,
    // higher taps are progressively older. Past-prefix taps contribute 0.
    //
    // x_conv[t, c] = rescale(Σ_{k=0..ksz} ssm_conv1d[k, c] * x[t-k, c],
    //                        ssm_scales.q).
    let mut x_conv = vec![0i8; mu * hu];
    for t in 0..mu {
        for c in 0..hu {
            let mut acc: i32 = 0;
            for k in 0..ksz {
                if t < k {
                    break; // past-prefix taps zero — masked.
                }
                let in_v = input[(t - k) * hu + c] as i32;
                let kw = opts.ssm_conv1d[k * hu + c] as i32;
                acc = acc.wrapping_add(in_v.wrapping_mul(kw));
            }
            x_conv[t * hu + c] = rescale_and_requantize(acc, opts.scales.q);
        }
    }

    // Step 2: α and β projections, both followed by the sigmoid LUT.
    let mut alpha_acc = vec![0i32; mu * nv];
    matmul_int8(&x_conv, opts.ssm_alpha, m, hidden, num_v, &mut alpha_acc)?;
    let mut alpha_i8 = vec![0i8; mu * nv];
    requantize_vec(&alpha_acc, opts.scales.alpha_logit, &mut alpha_i8)?;
    opts.sigmoid_lut.apply(&mut alpha_i8);
    drop(alpha_acc);

    let mut beta_acc = vec![0i32; mu * nv];
    matmul_int8(&x_conv, opts.ssm_beta, m, hidden, num_v, &mut beta_acc)?;
    let mut beta_i8 = vec![0i8; mu * nv];
    requantize_vec(&beta_acc, opts.scales.beta_logit, &mut beta_i8)?;
    opts.sigmoid_lut.apply(&mut beta_i8);
    drop(beta_acc);

    // Step 3: per-V-head state recurrence.
    //
    // state[v * hdu + d] is the per-V-head, per-channel state. We allocate
    // one flat (nv * hdu) buffer and zero-initialize.
    let mut state = vec![0i8; nv * hdu];

    // y_concat[t, v, d] is the post-recurrence state value, before SSM
    // norm and output projection. Layout: row-major (m, nv * hdu).
    let mut y_concat = vec![0i8; mu * nv * hdu];

    for t in 0..mu {
        for v in 0..nv {
            // Decay & update factors are scalar per (token, V-head).
            // They're products of two i8 values (sigmoid in [0, 127] times
            // ssm_a/ssm_dt in [-128, 127]) → fits in i32, then downscaled
            // by 7 bits to bound back to i8.
            let alpha_v = alpha_i8[t * nv + v] as i32;
            let beta_v = beta_i8[t * nv + v] as i32;
            let a_v = opts.ssm_a[v] as i32;
            let dt_v = opts.ssm_dt[v] as i32;
            let decay_factor = saturate_i8(((alpha_v.wrapping_mul(a_v)) >> 7) as i64) as i32;
            let update_factor = saturate_i8(((beta_v.wrapping_mul(dt_v)) >> 7) as i64) as i32;

            for d in 0..hdu {
                let s_off = v * hdu + d;
                let s_old = state[s_off] as i32;
                let c = (v * hdu + d) % hu; // channel mapping (v, d) → x_conv column
                let xv = x_conv[t * hu + c] as i32;

                let decay_term = s_old.wrapping_mul(decay_factor);
                let update_term = update_factor.wrapping_mul(xv);
                let decay_i8 = rescale_and_requantize(decay_term, opts.scales.decay) as i32;
                let update_i8 = rescale_and_requantize(update_term, opts.scales.update) as i32;
                let s_new = decay_i8.wrapping_add(update_i8);
                let s_new_i8 = saturate_i8(s_new as i64);
                state[s_off] = s_new_i8;
                y_concat[t * nv * hdu + v * hdu + d] = s_new_i8;
            }
        }
    }
    drop(alpha_i8);
    drop(beta_i8);
    drop(x_conv);
    drop(state);

    // Step 4: per-(token, V-head) RMSNorm with `ssm_norm_gamma`.
    let mut acc = vec![0i32; hdu];
    for t in 0..mu {
        for v in 0..nv {
            let off = t * nv * hdu + v * hdu;
            rmsnorm(
                &y_concat[off..off + hdu],
                opts.ssm_norm_gamma,
                &mut acc,
                opts.ssm_norm_eps_q,
            )?;
            for (d, &a) in acc.iter().enumerate() {
                y_concat[off + d] = rescale_and_requantize(a, opts.ssm_norm_post_scale);
            }
        }
    }

    // Step 5: output projection. (m, nv*hdu) @ ssm_out → (m, hidden) i8.
    matmul_int8_requant(
        &y_concat,
        opts.ssm_out,
        m,
        (nv * hdu) as u32,
        hidden,
        opts.scales.proj,
        output,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_lut::{ActivationKind, ActivationLut};
    use crate::quant::SCALE_DENOM_LOG2;
    use crate::rmsnorm::DEFAULT_EPS_Q;

    fn small_scale() -> Scale {
        Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap()
    }

    fn small_scales() -> DeltaNetScales {
        let s = small_scale();
        DeltaNetScales {
            q: s,
            k: s,
            v: s,
            alpha_logit: s,
            beta_logit: s,
            u: s,
            decay: s,
            update: s,
            o: s,
            proj: s,
        }
    }

    fn lcg_bytes(len: usize, seed: u64) -> Vec<i8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.wrapping_shr(56) as i8
            })
            .collect()
    }

    fn const_lut(value: i8) -> ActivationLut {
        ActivationLut::from_bytes(ActivationKind::SiLU, &[value as u8; 256]).unwrap()
    }

    fn hard_sigmoid_lut() -> ActivationLut {
        let mut bytes = [0u8; 256];
        for (i, b) in bytes.iter_mut().enumerate() {
            let x = (i as i32) - 128;
            let v = (64 + x / 2).clamp(0, 127);
            *b = v as u8;
        }
        ActivationLut::from_bytes(ActivationKind::SiLU, &bytes).unwrap()
    }

    struct WeightsBag {
        ssm_a: Vec<i8>,
        ssm_alpha: Vec<i8>,
        ssm_beta: Vec<i8>,
        ssm_conv1d: Vec<i8>,
        ssm_dt: Vec<i8>,
        ssm_norm_gamma: Vec<i8>,
        ssm_out: Vec<i8>,
    }

    fn build_bag(hidden: u32, num_v: u32, head_dim: u32, kernel: u32, seed: u64) -> WeightsBag {
        let hu = hidden as usize;
        let nv = num_v as usize;
        let hdu = head_dim as usize;
        let ksz = kernel as usize;
        WeightsBag {
            ssm_a: lcg_bytes(nv, seed),
            ssm_alpha: lcg_bytes(hu * nv, seed.wrapping_add(1)),
            ssm_beta: lcg_bytes(hu * nv, seed.wrapping_add(2)),
            ssm_conv1d: lcg_bytes(ksz * hu, seed.wrapping_add(3)),
            ssm_dt: lcg_bytes(nv, seed.wrapping_add(4)),
            ssm_norm_gamma: lcg_bytes(hdu, seed.wrapping_add(5)),
            ssm_out: lcg_bytes(nv * hdu * hu, seed.wrapping_add(6)),
        }
    }

    fn make_opts<'a>(
        bag: &'a WeightsBag,
        num_v: u32,
        head_dim: u32,
        kernel: u32,
        sigmoid_lut: &'a ActivationLut,
    ) -> SsmOpts<'a> {
        SsmOpts {
            ssm_a: &bag.ssm_a,
            ssm_alpha: &bag.ssm_alpha,
            ssm_beta: &bag.ssm_beta,
            ssm_conv1d: &bag.ssm_conv1d,
            ssm_dt: &bag.ssm_dt,
            ssm_norm_gamma: &bag.ssm_norm_gamma,
            ssm_norm_eps_q: DEFAULT_EPS_Q,
            ssm_norm_post_scale: small_scale(),
            ssm_out: &bag.ssm_out,
            num_v_heads: num_v,
            head_dim,
            kernel_size: kernel,
            scales: small_scales(),
            sigmoid_lut,
        }
    }

    #[test]
    fn zero_input_yields_zero_output() {
        let hidden = 8u32;
        let num_v = 2u32;
        let hd = 4u32;
        let k = 3u32;
        let m = 4u32;
        let bag = build_bag(hidden, num_v, hd, k, 0xabcd);
        let lut = hard_sigmoid_lut();
        let opts = make_opts(&bag, num_v, hd, k, &lut);
        let input = vec![0i8; (m * hidden) as usize];
        let mut output = vec![1i8; (m * hidden) as usize];
        ssm_forward(&input, hidden, m, opts, &mut output).unwrap();
        // Zero input → zero conv → zero α/β logits, but sigmoid(0) ≠ 0,
        // so α,β > 0. With xv = 0 the update term vanishes; with s = 0
        // initially the decay term also vanishes; state stays zero, and
        // y → norm → output projection of zeros = zero.
        for &v in &output {
            assert_eq!(v, 0, "zero input must produce zero output (got {output:?})");
        }
    }

    #[test]
    fn determinism_two_calls_match() {
        let hidden = 8u32;
        let num_v = 2u32;
        let hd = 4u32;
        let k = 3u32;
        let m = 4u32;
        let bag = build_bag(hidden, num_v, hd, k, 0xfeed);
        let lut = hard_sigmoid_lut();
        let opts = make_opts(&bag, num_v, hd, k, &lut);
        let input = lcg_bytes((m * hidden) as usize, 0xbeef);
        let mut a = vec![0i8; (m * hidden) as usize];
        let mut b = vec![0i8; (m * hidden) as usize];
        ssm_forward(&input, hidden, m, opts, &mut a).unwrap();
        ssm_forward(&input, hidden, m, opts, &mut b).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn beta_zero_state_never_updates() {
        // β = 0 → update term zero. With state initialized to zero, the
        // decay term is also zero → state stays zero → output is zero.
        let hidden = 4u32;
        let num_v = 1u32;
        let hd = 2u32;
        let k = 2u32;
        let m = 3u32;
        let bag = build_bag(hidden, num_v, hd, k, 0x1111);
        let zero_lut = const_lut(0);
        let opts = make_opts(&bag, num_v, hd, k, &zero_lut);
        let input = lcg_bytes((m * hidden) as usize, 0x2222);
        let mut output = vec![1i8; (m * hidden) as usize];
        ssm_forward(&input, hidden, m, opts, &mut output).unwrap();
        for &v in &output {
            assert_eq!(v, 0, "β = 0 → state stays zero → output zero");
        }
    }

    #[test]
    fn fresh_state_each_call() {
        // Two separate calls must produce identical outputs (state must not
        // leak across calls via a `static` or thread-local).
        let hidden = 4u32;
        let num_v = 1u32;
        let hd = 2u32;
        let k = 2u32;
        let m = 3u32;
        let bag = build_bag(hidden, num_v, hd, k, 0x3333);
        let lut = hard_sigmoid_lut();
        let opts = make_opts(&bag, num_v, hd, k, &lut);
        let input = lcg_bytes((m * hidden) as usize, 0x4444);
        let mut a = vec![0i8; (m * hidden) as usize];
        let mut b = vec![0i8; (m * hidden) as usize];
        ssm_forward(&input, hidden, m, opts, &mut a).unwrap();
        // Run something else in between.
        let other = lcg_bytes((m * hidden) as usize, 0x5555);
        let mut out_other = vec![0i8; (m * hidden) as usize];
        ssm_forward(&other, hidden, m, opts, &mut out_other).unwrap();
        // Re-run the original.
        ssm_forward(&input, hidden, m, opts, &mut b).unwrap();
        assert_eq!(a, b, "state must not leak across calls");
    }

    #[test]
    fn conv_kernel_masks_past_prefix() {
        // Two SSM calls: one with `m=1` and one with `m=2` must produce
        // identical outputs at position 0 (the conv mask zeros taps past
        // the prefix bound, so position 0 sees only its own token).
        let hidden = 4u32;
        let num_v = 1u32;
        let hd = 2u32;
        let k = 3u32; // kernel covers t-2, t-1, t
        let bag = build_bag(hidden, num_v, hd, k, 0x6666);
        let lut = hard_sigmoid_lut();
        let opts = make_opts(&bag, num_v, hd, k, &lut);
        let input1 = lcg_bytes(hidden as usize, 0x7777);
        let mut input2 = vec![0i8; (2 * hidden) as usize];
        input2[..(hidden as usize)].copy_from_slice(&input1);
        // Position-1 input is anything; position 0 must be `input1`.
        let pad = lcg_bytes(hidden as usize, 0x8888);
        input2[(hidden as usize)..].copy_from_slice(&pad);
        let mut out1 = vec![0i8; hidden as usize];
        let mut out2 = vec![0i8; (2 * hidden) as usize];
        ssm_forward(&input1, hidden, 1, opts, &mut out1).unwrap();
        ssm_forward(&input2, hidden, 2, opts, &mut out2).unwrap();
        assert_eq!(
            &out1[..],
            &out2[..hidden as usize],
            "row 0 must match across m=1 and m=2 calls (causal mask + fresh state)",
        );
    }

    #[test]
    fn length_mismatch_rejected() {
        let hidden = 4u32;
        let num_v = 1u32;
        let hd = 2u32;
        let k = 2u32;
        let m = 2u32;
        let bag = build_bag(hidden, num_v, hd, k, 0x9999);
        let lut = hard_sigmoid_lut();
        let opts = make_opts(&bag, num_v, hd, k, &lut);

        let bad = vec![0i8; 7];
        let mut out = vec![0i8; (m * hidden) as usize];
        assert_eq!(
            ssm_forward(&bad, hidden, m, opts, &mut out).err(),
            Some(SsmError::BadInputLen),
        );

        let input = vec![0i8; (m * hidden) as usize];
        let mut bad_out = vec![0i8; 5];
        assert_eq!(
            ssm_forward(&input, hidden, m, opts, &mut bad_out).err(),
            Some(SsmError::BadOutputLen),
        );

        assert_eq!(
            ssm_forward(&input, 0, m, opts, &mut out).err(),
            Some(SsmError::ZeroDim),
        );
        assert_eq!(
            ssm_forward(&input, hidden, 0, opts, &mut out).err(),
            Some(SsmError::ZeroDim),
        );
    }

    #[test]
    fn weight_shape_mismatch_rejected() {
        let hidden = 4u32;
        let num_v = 1u32;
        let hd = 2u32;
        let k = 2u32;
        let m = 2u32;
        let bag = build_bag(hidden, num_v, hd, k, 0xaaaa);
        let lut = hard_sigmoid_lut();
        let input = vec![0i8; (m * hidden) as usize];
        let mut out = vec![0i8; (m * hidden) as usize];

        // ssm_a wrong length.
        let bad_a = vec![0i8; 2]; // should be num_v=1
        let mut o = make_opts(&bag, num_v, hd, k, &lut);
        o.ssm_a = &bad_a;
        assert_eq!(
            ssm_forward(&input, hidden, m, o, &mut out).err(),
            Some(SsmError::BadSsmALen),
        );

        // ssm_conv1d wrong length.
        let bad_conv = vec![0i8; 7];
        let mut o = make_opts(&bag, num_v, hd, k, &lut);
        o.ssm_conv1d = &bad_conv;
        assert_eq!(
            ssm_forward(&input, hidden, m, o, &mut out).err(),
            Some(SsmError::BadSsmConvLen),
        );

        // ssm_norm_gamma wrong length.
        let bad_norm = vec![0i8; 5];
        let mut o = make_opts(&bag, num_v, hd, k, &lut);
        o.ssm_norm_gamma = &bad_norm;
        assert_eq!(
            ssm_forward(&input, hidden, m, o, &mut out).err(),
            Some(SsmError::BadSsmNormGammaLen),
        );

        // ssm_out wrong length.
        let bad_out_w = vec![0i8; 7];
        let mut o = make_opts(&bag, num_v, hd, k, &lut);
        o.ssm_out = &bad_out_w;
        assert_eq!(
            ssm_forward(&input, hidden, m, o, &mut out).err(),
            Some(SsmError::BadSsmOutLen),
        );
    }
}
