//! Cross-architecture determinism pins.
//!
//! Each implemented op runs a canonical test vector and asserts the BLAKE3
//! of its output against a hard-coded `expected_hash`. CI runs this on
//! both x86_64 and aarch64; if either platform diverges, this test fails
//! with `ARCH_TAG` named in the diagnostic.
//!
//! When you intentionally change an op's output (e.g. tightening
//! quantization, adjusting the integer reciprocal-sqrt convergence
//! criterion) you must update the corresponding `expected` constant below.
//! Updating without thought silently breaks consensus across already-
//! deployed nodes; the test is structured to make the change explicit and
//! reviewable.

use ai_pow_vi::activation_lut::{ActivationKind, ActivationLut};
use ai_pow_vi::activations::{ActivationLayout, ActivationLog};
use ai_pow_vi::attention::{attention_forward, AttentionScales, AttentionWeights};
use ai_pow_vi::comm_w::compute_comm_w;
use ai_pow_vi::deltanet::{deltanet_forward, DeltaNetScales, DeltaNetWeights};
use ai_pow_vi::determinism::{hash_canonical, ARCH_TAG};
use ai_pow_vi::ffn::{ffn_forward, FfnScales, FfnWeights};
use ai_pow_vi::layer::{forward_layer, LayerContext, LayerWeights, NormSpec};
use ai_pow_vi::layernorm::layernorm;
use ai_pow_vi::matmul_int8::matmul_int8;
use ai_pow_vi::model::{Model, ModelDims};
use ai_pow_vi::prompt::synth_prompt;
use ai_pow_vi::prover::{mine_vi, ProverOptions};
use ai_pow_vi::quant::{rescale_and_requantize, Scale, SCALE_DENOM_LOG2};
use ai_pow_vi::rmsnorm::{isqrt_floor, rmsnorm, DEFAULT_EPS_Q};
use ai_pow_vi::rope::{rope_apply, RopeTables, FRACT_BITS as ROPE_FRACT_BITS};
use ai_pow_vi::softmax::{softmax_int, ExpLut};
use ai_pow_vi::ssm::{ssm_forward, SsmOpts};
use ai_pow_vi::verifier::{verify_vi, VerifierMode};

fn canonical_input_i8(len: usize, seed: u64) -> Vec<i8> {
    // Deterministic linear-congruential stream over u64; map to i8.
    // Not a real RNG; just stable bytes that exercise the dynamic range.
    let mut s = seed;
    (0..len)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s.wrapping_shr(56) as i8)
        })
        .collect()
}

#[test]
fn pin_rescale_and_requantize() {
    // 1024 i32 accumulators × scale = 0.5 (banker's rounding sweep).
    let scale = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 1)).unwrap();
    let mut bytes = Vec::with_capacity(1024);
    for i in -512i32..512 {
        bytes.push(rescale_and_requantize(i, scale) as u8);
    }
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x25, 0xda, 0x36, 0x69, 0x70, 0x21, 0x75, 0x21, 0xe1, 0x70, 0x4a, 0x01, 0x8f, 0x43, 0xd7,
        0x17, 0x64, 0x06, 0xff, 0x41, 0x89, 0x49, 0x88, 0x43, 0x0c, 0x97, 0xdf, 0xd9, 0x3e, 0x21,
        0x10, 0x75,
    ];
    assert_eq!(
        actual, expected,
        "{ARCH_TAG} rescale_and_requantize divergence"
    );
}

#[test]
fn pin_activation_lut_identity_commit() {
    let lut = ActivationLut::identity();
    let actual = lut.commit();
    let expected: [u8; 32] = [
        0xec, 0xd3, 0xdf, 0x65, 0x29, 0xfa, 0xc0, 0x31, 0xdd, 0xe6, 0x54, 0xf0, 0xb5, 0x6c, 0xa1,
        0x5e, 0xd4, 0x7b, 0x6f, 0x0f, 0xf7, 0x7c, 0x71, 0xaf, 0xce, 0x08, 0xd9, 0xfb, 0x72, 0xa2,
        0xbb, 0x41,
    ];
    assert_eq!(
        actual, expected,
        "{ARCH_TAG} identity LUT commit divergence"
    );
}

#[test]
fn pin_activation_lut_gelu_shape_commit() {
    // A *handcoded* "GeLU-shaped" LUT: linear ramp with a squash near zero.
    // Not the real GeLU — purely to exercise the commit pipeline with a
    // non-trivial table. The pin must move iff the bytes change.
    let mut bytes = [0u8; 256];
    for (i, slot) in bytes.iter_mut().enumerate() {
        let x = i as i32 - 128;
        let squash = if x.abs() < 4 { 0 } else { x };
        *slot = (squash.clamp(-128, 127)) as u8;
    }
    let lut = ActivationLut::from_bytes(ActivationKind::GeLU, &bytes).unwrap();
    let actual = lut.commit();
    let expected: [u8; 32] = [
        0xb3, 0x6c, 0x08, 0xff, 0x69, 0xe8, 0x8a, 0xb2, 0xc6, 0x01, 0xd0, 0x47, 0x6f, 0xa5, 0x31,
        0x3b, 0xa3, 0x79, 0x20, 0x1c, 0x92, 0x01, 0x33, 0x09, 0xd3, 0xab, 0x29, 0x69, 0x4e, 0x28,
        0xed, 0x1f,
    ];
    assert_eq!(
        actual, expected,
        "{ARCH_TAG} handcoded GeLU LUT commit divergence"
    );
}

#[test]
fn pin_isqrt_floor_canonical_sweep() {
    // Sweep representative magnitudes; serialize the i64 results LE.
    let mut bytes: Vec<u8> = Vec::with_capacity(64 * 8);
    for k in 0..64u64 {
        let y = (k * k * 17 + k * 9876543 + k.wrapping_mul(987654321)) as i64;
        bytes.extend_from_slice(&isqrt_floor(y).to_le_bytes());
    }
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x4b, 0x84, 0xf6, 0xbc, 0x0a, 0x51, 0x05, 0x30, 0x6f, 0x23, 0x87, 0x08, 0xee, 0x98, 0x34,
        0x58, 0x67, 0x26, 0xd1, 0xf7, 0x07, 0x77, 0xbd, 0xf8, 0x1c, 0xa9, 0x16, 0xf2, 0xea, 0x31,
        0x57, 0x40,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} isqrt_floor divergence");
}

#[test]
fn pin_rmsnorm_canonical_output() {
    let hidden = 64;
    let input = canonical_input_i8(hidden, 0xaaaa_bbbb_cccc_ddde);
    let gamma = canonical_input_i8(hidden, 0x1111_2222_3333_4444);
    let mut output = vec![0i32; hidden];
    rmsnorm(&input, &gamma, &mut output, DEFAULT_EPS_Q).unwrap();
    let mut bytes: Vec<u8> = Vec::with_capacity(hidden * 4);
    for v in &output {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0xe1, 0xbe, 0x32, 0x20, 0x00, 0x3c, 0x82, 0x52, 0x25, 0x2e, 0x9f, 0xc8, 0xa6, 0x6c, 0x0f,
        0xa4, 0xd7, 0x08, 0xaa, 0x5d, 0x04, 0x6a, 0xb5, 0x28, 0xc8, 0x20, 0xa1, 0xc6, 0x34, 0xe8,
        0x9d, 0x67,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} rmsnorm divergence");
}

#[test]
fn pin_softmax_canonical_output() {
    // Build a hand-coded "decay" LUT (not real exp; just sharp enough to
    // exercise the integer normalize). 16-position score array; result is
    // 16 i8 probabilities. Pin both the LUT commit and the softmax output.
    let mut table = [0i32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let v = if i < 16 {
            (1i32 << 16).wrapping_shr(i as u32)
        } else {
            0
        };
        *slot = v;
    }
    let lut = ExpLut { table };
    let lut_commit = lut.commit();
    let pinned_lut_commit: [u8; 32] = [
        0xf3, 0x0a, 0xd8, 0xb0, 0xbf, 0xd6, 0x94, 0x75, 0xce, 0x12, 0x98, 0x96, 0xcf, 0xaf, 0x31,
        0x52, 0x0b, 0xfe, 0x8e, 0x08, 0x88, 0xaa, 0x2d, 0x83, 0xaa, 0x48, 0xd4, 0x15, 0x52, 0x22,
        0x26, 0x9f,
    ];
    assert_eq!(
        lut_commit, pinned_lut_commit,
        "{ARCH_TAG} ExpLut commit divergence"
    );

    let scores: Vec<i32> = (0..16).map(|i| (i * 3) % 17 - 6).collect();
    let mut out = vec![0i8; scores.len()];
    softmax_int(&scores, &lut, &mut out).unwrap();
    let bytes: Vec<u8> = out.iter().map(|&v| v as u8).collect();
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0xc3, 0x84, 0x25, 0x6f, 0x33, 0x5b, 0xd4, 0x04, 0xfc, 0x9d, 0x9a, 0xe8, 0xaa, 0x9b, 0x95,
        0xf7, 0xe3, 0xce, 0x9b, 0xe3, 0x9b, 0xc4, 0x8e, 0x5c, 0xfa, 0x98, 0xf8, 0x6f, 0xa5, 0xf1,
        0x3f, 0xb0,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} softmax_int divergence");
}

#[test]
fn pin_rope_canonical_output() {
    // Hand-built tables: 4 positions × 4 pair-indices. Position 0 is the
    // identity rotation; positions 1..3 use distinct fixed angles.
    let mut tables = RopeTables::zeros(4, 4);
    for j in 0..4 {
        // Position 0 (identity).
        tables.cos[j] = 1 << ROPE_FRACT_BITS;
        tables.sin[j] = 0;
        // Position 1: 30° (cos≈14189, sin≈8192 in 2^14 fixed-point).
        tables.cos[4 + j] = 14189;
        tables.sin[4 + j] = 8192;
        // Position 2: -45° (cos≈11585, sin≈-11585).
        tables.cos[8 + j] = 11585;
        tables.sin[8 + j] = -11585;
        // Position 3: 90° (cos=0, sin=2^14).
        tables.cos[12 + j] = 0;
        tables.sin[12 + j] = 1 << ROPE_FRACT_BITS;
    }
    let tables_commit = tables.commit();
    let pinned_tables_commit: [u8; 32] = [
        0xc0, 0xe7, 0x42, 0xab, 0xae, 0x43, 0x72, 0x17, 0x90, 0xf2, 0x6a, 0x9b, 0xad, 0x13, 0xaa,
        0xa0, 0x7a, 0x06, 0x3b, 0x1c, 0x23, 0x13, 0x3e, 0xfa, 0xc1, 0x5f, 0x48, 0x2a, 0xdc, 0x6c,
        0x80, 0x35,
    ];
    assert_eq!(
        tables_commit, pinned_tables_commit,
        "{ARCH_TAG} RopeTables commit divergence"
    );

    // Apply RoPE at each position to the same input vector; concatenate
    // results and pin their hash.
    let seed: Vec<i8> = (0..8i8).map(|v| (v * 13 - 50) as i8).collect();
    let mut all = Vec::with_capacity(8 * 4);
    for pos in 0..4 {
        let mut x = seed.clone();
        rope_apply(&mut x, pos, &tables).unwrap();
        all.extend_from_slice(&x.iter().map(|&v| v as u8).collect::<Vec<_>>());
    }
    let actual = hash_canonical(&all);
    let expected: [u8; 32] = [
        0x64, 0xf9, 0xe6, 0x6f, 0x58, 0xdb, 0x2c, 0xb9, 0xd6, 0xf2, 0x5e, 0x5c, 0x38, 0xc9, 0xbb,
        0x8c, 0x28, 0x41, 0x44, 0x4c, 0x02, 0xee, 0x7e, 0xa4, 0x5c, 0xa8, 0xff, 0xa8, 0xe5, 0xae,
        0x3d, 0xca,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} rope_apply divergence");
}

#[test]
fn pin_matmul_int8_canonical_output() {
    // (4, 8) × (8, 4) → 16 i32 outputs. Inputs from the same canonical
    // LCG used elsewhere; that gives a stable byte stream that exercises
    // both signs and full i8 range.
    let m = 4u32;
    let k = 8u32;
    let n = 4u32;
    let a = canonical_input_i8((m * k) as usize, 0xfeed_beef_cafe_babe);
    let b = canonical_input_i8((k * n) as usize, 0x0123_4567_89ab_cdef);
    let mut out = vec![0i32; (m * n) as usize];
    matmul_int8(&a, &b, m, k, n, &mut out).unwrap();
    let mut bytes: Vec<u8> = Vec::with_capacity(out.len() * 4);
    for v in &out {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0xa7, 0x06, 0xdd, 0x54, 0xac, 0x98, 0x38, 0x3c, 0xa8, 0x73, 0x18, 0xe7, 0xf9, 0x4f, 0xe6,
        0x24, 0x4c, 0x2c, 0xbc, 0x3b, 0x46, 0x08, 0x87, 0x7c, 0x1c, 0xe1, 0x26, 0x83, 0x32, 0x25,
        0x02, 0x6c,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} matmul_int8 divergence");
}

#[test]
fn pin_layernorm_canonical_output() {
    let hidden = 64;
    let input = canonical_input_i8(hidden, 0x9999_aaaa_bbbb_cccc);
    let gamma = canonical_input_i8(hidden, 0x1357_2468_acef_bd13);
    let beta = canonical_input_i8(hidden, 0x4242_4242_4242_4242);
    let mut output = vec![0i32; hidden];
    layernorm(&input, &gamma, &beta, &mut output, DEFAULT_EPS_Q).unwrap();
    let mut bytes: Vec<u8> = Vec::with_capacity(hidden * 4);
    for v in &output {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x13, 0xcf, 0x8e, 0x09, 0xd1, 0xb4, 0xbd, 0x04, 0x1f, 0x6f, 0x15, 0x37, 0x72, 0x35, 0xed,
        0x70, 0xa3, 0x1a, 0x93, 0x58, 0x37, 0x49, 0x75, 0xe6, 0xe2, 0x8e, 0x08, 0xe7, 0x12, 0x5c,
        0x6e, 0x11,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} layernorm divergence");
}

#[test]
fn pin_ffn_canonical_output() {
    // Small SwiGLU: m=2, hidden=8, intermediate=16. Inputs and weights from
    // the LCG; identity activation LUT (so we test the matmul + multiply
    // composition, not the activation curve — that has its own pin).
    let hidden = 8u32;
    let intermediate = 16u32;
    let m = 2u32;
    let input = canonical_input_i8((m * hidden) as usize, 0xfeedface_deadbeefu64);
    let w_gate = canonical_input_i8((hidden * intermediate) as usize, 0xa1a1_b2b2_c3c3_d4d4u64);
    let w_up = canonical_input_i8((hidden * intermediate) as usize, 0xe5e5_f6f6_0707_1818u64);
    let w_down = canonical_input_i8((intermediate * hidden) as usize, 0x2929_3a3a_4b4b_5c5cu64);
    let weights = FfnWeights {
        hidden,
        intermediate,
        w_gate,
        w_up,
        w_down,
    };
    let activation = ActivationLut::identity();
    let scales = FfnScales {
        gate: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
        up: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
        mid: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        down: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
    };
    let mut output = vec![0i8; (m * hidden) as usize];
    ffn_forward(&input, &weights, &activation, scales, m, &mut output).unwrap();
    let bytes: Vec<u8> = output.iter().map(|&v| v as u8).collect();
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x29, 0x5c, 0x41, 0xea, 0xe0, 0x13, 0x48, 0x2f, 0x54, 0xe4, 0x9b, 0xaa, 0x2c, 0x0c, 0xaf,
        0x1f, 0x9f, 0x2e, 0x4f, 0xd8, 0x79, 0x2e, 0x0e, 0x0e, 0xd2, 0x3c, 0xe0, 0xd6, 0x84, 0xaf,
        0x26, 0x92,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} ffn_forward divergence");
}

#[test]
fn pin_attention_canonical_output() {
    // Fixture: m=4, hidden=8, num_q_heads=2, num_kv_heads=1, head_dim=4.
    // Identity RoPE (exercises composition, not rotation — rope has its own pin).
    // Uniform-test ExpLut (softmax has its own pin).
    let hidden = 8u32;
    let num_q = 2u32;
    let num_kv = 1u32;
    let hd = 4u32;
    let m = 4u32;
    let hu = hidden as usize;
    let qu = (num_q * hd) as usize;
    let kvu = (num_kv * hd) as usize;
    let w_q = canonical_input_i8(hu * qu, 0xaa11_bb22_cc33_dd44);
    let w_k = canonical_input_i8(hu * kvu, 0xee55_ff66_0077_1188);
    let w_v = canonical_input_i8(hu * kvu, 0x2299_33aa_44bb_55cc);
    let w_o = canonical_input_i8(qu * hu, 0x66dd_77ee_88ff_990a);
    let weights = AttentionWeights {
        hidden,
        num_q_heads: num_q,
        num_kv_heads: num_kv,
        head_dim: hd,
        w_q,
        w_k,
        w_v,
        w_o,
    };
    let scales = AttentionScales {
        q: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        k: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        v: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        score: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        attn_out: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        o: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
    };
    let rope_tables = RopeTables::identity(m, hd / 2);
    let lut = ExpLut::uniform_test();
    let input = canonical_input_i8((m * hidden) as usize, 0xfeed_beef_cafe_babe);
    let mut output = vec![0i8; (m * hidden) as usize];
    attention_forward(&input, &weights, scales, &rope_tables, &lut, m, &mut output).unwrap();
    let bytes: Vec<u8> = output.iter().map(|&v| v as u8).collect();
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0xf3, 0x50, 0x7d, 0x54, 0xe2, 0x72, 0xb0, 0xf4, 0xe4, 0x1f, 0xb2, 0xe6, 0xf2, 0x63, 0x79,
        0x8a, 0x0f, 0xca, 0x77, 0x1d, 0x83, 0x5e, 0xc0, 0x40, 0x53, 0x86, 0x61, 0x81, 0x58, 0xcd,
        0x95, 0x10,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} attention_forward divergence");
}

#[test]
fn pin_deltanet_canonical_output() {
    // Fixture: m=3, hidden=4, num_qk=2, num_v=4, head_dim_qk=2, head_dim_v=2.
    // 2 V heads per QK head (GQA: v_to_qk(v) = v * num_qk / num_v).
    // Per-head state is 2*2 = 4 i8; total state is 4*4 = 16 i8.
    let hidden = 4u32;
    let num_qk = 2u32;
    let num_v = 4u32;
    let hd_qk = 2u32;
    let hd_v = 2u32;
    let m = 3u32;

    let weights = DeltaNetWeights {
        hidden,
        num_qk_heads: num_qk,
        num_v_heads: num_v,
        head_dim_qk: hd_qk,
        head_dim_v: hd_v,
        w_q: canonical_input_i8((hidden * num_qk * hd_qk) as usize, 0xa1a1_b2b2_c3c3_d4d4u64),
        w_k: canonical_input_i8((hidden * num_qk * hd_qk) as usize, 0xe5e5_f6f6_0707_1818u64),
        w_v: canonical_input_i8((hidden * num_v * hd_v) as usize, 0x2929_3a3a_4b4b_5c5cu64),
        w_alpha: canonical_input_i8((hidden * num_qk) as usize, 0x6d6d_7e7e_8f8f_90a0u64),
        w_beta: canonical_input_i8((hidden * num_qk) as usize, 0xb1b1_c2c2_d3d3_e4e4u64),
        w_o: canonical_input_i8((num_v * hd_v * hidden) as usize, 0xf5f5_0606_1717_2828u64),
    };

    // Hand-coded "hard sigmoid"-shape LUT: clamp(64 + x/2, 0, 127). Pinned so
    // the deltanet pin stays stable.
    let mut sigmoid_bytes = [0u8; 256];
    for (i, b) in sigmoid_bytes.iter_mut().enumerate() {
        let x = (i as i32) - 128;
        let v = (64 + x / 2).clamp(0, 127);
        *b = v as u8;
    }
    let sigmoid_lut = ActivationLut::from_bytes(ActivationKind::SiLU, &sigmoid_bytes).unwrap();
    let lut_commit = sigmoid_lut.commit();
    let pinned_lut_commit: [u8; 32] = [
        0xc7, 0x67, 0xe2, 0x7a, 0xff, 0x30, 0x0f, 0x14, 0x74, 0x79, 0x34, 0xd9, 0xf7, 0xb0, 0x13,
        0x4c, 0xa7, 0xdc, 0xb2, 0x97, 0xa0, 0xa6, 0xb0, 0x03, 0x4a, 0x87, 0xb6, 0xb4, 0x18, 0x67,
        0xf2, 0x3b,
    ];
    assert_eq!(
        lut_commit, pinned_lut_commit,
        "{ARCH_TAG} hard-sigmoid LUT commit divergence"
    );

    let scales = DeltaNetScales {
        q: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        k: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        v: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        alpha_logit: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        beta_logit: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        u: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        decay: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 6)).unwrap(),
        update: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        o: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        proj: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
    };

    let input = canonical_input_i8((m * hidden) as usize, 0x9999_aaaa_bbbb_ccccu64);
    let mut output = vec![0i8; (m * hidden) as usize];
    deltanet_forward(&input, &weights, scales, &sigmoid_lut, m, &mut output).unwrap();
    let bytes: Vec<u8> = output.iter().map(|&v| v as u8).collect();
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0xc8, 0xdd, 0xa5, 0x45, 0xba, 0x29, 0xfa, 0xaf, 0x9c, 0x5a, 0x7b, 0xf1, 0xbe, 0xba, 0xc2,
        0x2f, 0xb2, 0x72, 0x30, 0xf7, 0x24, 0xa9, 0x7c, 0x6e, 0xd8, 0xd7, 0x8d, 0xa9, 0x47, 0x5f,
        0xa8, 0x93,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} deltanet_forward divergence");
}

#[test]
fn pin_ssm_canonical_output() {
    // Phase 2.13 Mamba SSM forward determinism pin.
    // Fixture: m=3, hidden=4, num_v=2, head_dim=2, kernel=3.
    // Per-V-head state is 2 i8; total state is 4 i8.
    let hidden = 4u32;
    let num_v = 2u32;
    let hd = 2u32;
    let kernel = 3u32;
    let m = 3u32;

    let nv = num_v as usize;
    let hu = hidden as usize;
    let hdu = hd as usize;
    let ksz = kernel as usize;

    let ssm_a = canonical_input_i8(nv, 0xa1a1_b2b2_c3c3_d4d4u64);
    let ssm_alpha = canonical_input_i8(hu * nv, 0xe5e5_f6f6_0707_1818u64);
    let ssm_beta = canonical_input_i8(hu * nv, 0x2929_3a3a_4b4b_5c5cu64);
    let ssm_conv1d = canonical_input_i8(ksz * hu, 0x6d6d_7e7e_8f8f_90a0u64);
    let ssm_dt = canonical_input_i8(nv, 0xb1b1_c2c2_d3d3_e4e4u64);
    let ssm_norm_gamma = canonical_input_i8(hdu, 0xf5f5_0606_1717_2828u64);
    let ssm_out = canonical_input_i8(nv * hdu * hu, 0x3939_4a4a_5b5b_6c6cu64);

    // Hard-sigmoid LUT (same shape as the deltanet pin uses, kept locally
    // here so this test is self-contained).
    let mut sigmoid_bytes = [0u8; 256];
    for (i, b) in sigmoid_bytes.iter_mut().enumerate() {
        let x = (i as i32) - 128;
        let v = (64 + x / 2).clamp(0, 127);
        *b = v as u8;
    }
    let sigmoid_lut = ActivationLut::from_bytes(ActivationKind::SiLU, &sigmoid_bytes).unwrap();

    let scales = DeltaNetScales {
        q: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        k: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        v: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        alpha_logit: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        beta_logit: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        u: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        decay: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        update: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        o: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        proj: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
    };

    let opts = SsmOpts {
        ssm_a: &ssm_a,
        ssm_alpha: &ssm_alpha,
        ssm_beta: &ssm_beta,
        ssm_conv1d: &ssm_conv1d,
        ssm_dt: &ssm_dt,
        ssm_norm_gamma: &ssm_norm_gamma,
        ssm_norm_eps_q: DEFAULT_EPS_Q,
        ssm_norm_post_scale: Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap(),
        ssm_out: &ssm_out,
        num_v_heads: num_v,
        head_dim: hd,
        kernel_size: kernel,
        scales,
        sigmoid_lut: &sigmoid_lut,
    };

    let input = canonical_input_i8((m * hidden) as usize, 0x9999_aaaa_bbbb_ccccu64);
    let mut output = vec![0i8; (m * hidden) as usize];
    ssm_forward(&input, hidden, m, opts, &mut output).unwrap();
    let bytes: Vec<u8> = output.iter().map(|&v| v as u8).collect();
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x39, 0xcb, 0x09, 0x20, 0x5d, 0xab, 0xea, 0xaf, 0x01, 0xb5, 0x77, 0xbd, 0xcc, 0x7a, 0xec,
        0xee, 0x86, 0xde, 0xd4, 0xee, 0xb7, 0xac, 0x9c, 0xd7, 0xe4, 0x8f, 0xd0, 0x1a, 0xa4, 0x53,
        0x1c, 0x56,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} ssm_forward divergence");
}

#[test]
fn pin_activation_log_canonical_root() {
    // Three-layer log over a small (seq_len=4, hidden=8) tensor with tile=2
    // (so 2*4 = 8 tiles per layer, padded to 8 leaves — already power of 2).
    // Concatenate the three layer roots and pin their hash.
    let layout = ActivationLayout {
        seq_len: 4,
        hidden: 8,
        tile: 2,
    };
    let mut log = ActivationLog::new(layout).unwrap();
    let t0 = canonical_input_i8((4 * 8) as usize, 0xa1a1_b2b2_c3c3_d4d4u64);
    let t1 = canonical_input_i8((4 * 8) as usize, 0xe5e5_f6f6_0707_1818u64);
    let t2 = canonical_input_i8((4 * 8) as usize, 0x2929_3a3a_4b4b_5c5cu64);
    log.record_layer(0, &t0).unwrap();
    log.record_layer(1, &t1).unwrap();
    log.record_layer(2, &t2).unwrap();
    let mut bytes: Vec<u8> = Vec::with_capacity(3 * 32);
    for r in &log.layer_roots {
        bytes.extend_from_slice(r);
    }
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x43, 0x8f, 0x9c, 0x00, 0x25, 0x61, 0x7b, 0xfa, 0x28, 0xcb, 0xde, 0x85, 0x0a, 0xf8, 0x5d,
        0x45, 0xe7, 0x71, 0x98, 0x5e, 0x2f, 0xf0, 0x18, 0xd6, 0x6c, 0xe4, 0x4e, 0x53, 0x38, 0x26,
        0x10, 0xb8,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} activation_log divergence");
}

#[test]
fn pin_attention_layer_canonical_output() {
    use ai_pow_vi::rmsnorm::DEFAULT_EPS_Q;
    use ai_pow_vi::rope::RopeTables;
    use ai_pow_vi::softmax::ExpLut;

    let hidden = 4u32;
    let num_q = 1u32;
    let num_kv = 1u32;
    let hd = 2u32;
    let m = 2u32;
    let hu = hidden as usize;

    let small = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap();
    let layer = LayerWeights::Attention {
        norm1: NormSpec::RmsNorm {
            gamma: canonical_input_i8(hu, 0xa1a1_b2b2_c3c3_d4d4),
            eps_q: DEFAULT_EPS_Q,
            post_scale: small,
        },
        attn: AttentionWeights {
            hidden,
            num_q_heads: num_q,
            num_kv_heads: num_kv,
            head_dim: hd,
            w_q: canonical_input_i8(hu * (num_q * hd) as usize, 0xe5e5_f6f6_0707_1818),
            w_k: canonical_input_i8(hu * (num_kv * hd) as usize, 0x2929_3a3a_4b4b_5c5c),
            w_v: canonical_input_i8(hu * (num_kv * hd) as usize, 0x6d6d_7e7e_8f8f_90a0),
            w_o: canonical_input_i8((num_q * hd) as usize * hu, 0xb1b1_c2c2_d3d3_e4e4),
        },
        attn_scales: AttentionScales {
            q: small,
            k: small,
            v: small,
            score: small,
            attn_out: small,
            o: small,
        },
        norm2: NormSpec::RmsNorm {
            gamma: canonical_input_i8(hu, 0xf5f5_0606_1717_2828),
            eps_q: DEFAULT_EPS_Q,
            post_scale: small,
        },
        ffn: FfnWeights {
            hidden,
            intermediate: hidden * 2,
            w_gate: canonical_input_i8(hu * (hu * 2), 0x3939_4a4a_5b5b_6c6c),
            w_up: canonical_input_i8(hu * (hu * 2), 0x7d7d_8e8e_9f9f_a0a0),
            w_down: canonical_input_i8((hu * 2) * hu, 0xb1b1_c2c2_d3d3_e4e4),
        },
        ffn_scales: FfnScales {
            gate: small,
            up: small,
            mid: small,
            down: small,
        },
    };

    let rope_tables = RopeTables::identity(m, hd / 2);
    let softmax_lut = ExpLut::uniform_test();
    let sigmoid_lut = ActivationLut::identity();
    let ffn_act = ActivationLut::identity();
    let ctx = LayerContext {
        rope_tables: &rope_tables,
        softmax_lut: &softmax_lut,
        sigmoid_lut: &sigmoid_lut,
        ffn_activation: &ffn_act,
    };
    let input = canonical_input_i8((m * hidden) as usize, 0x1234_5678_9abc_def0);
    let mut output = vec![0i8; (m * hidden) as usize];
    forward_layer(&input, &layer, &ctx, m, &mut output).unwrap();
    let bytes: Vec<u8> = output.iter().map(|&v| v as u8).collect();
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x1c, 0x00, 0xd4, 0x2f, 0x10, 0xcb, 0x80, 0xc8, 0x45, 0x7f, 0x64, 0x95, 0x83, 0x35, 0x50,
        0x61, 0x0e, 0x6b, 0xb8, 0x27, 0x37, 0xf1, 0x07, 0x65, 0x07, 0x9f, 0xe6, 0x0a, 0xec, 0x66,
        0x9a, 0xf3,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} attention_layer divergence");
}

#[test]
fn pin_synth_prompt_canonical() {
    // Generate a 32-token prompt with a fixed (block_commitment, model_id);
    // pin the byte hash of the resulting `Vec<Token>` (LE-encoded u32).
    let block = b"ai-pow-vi pin block-commitment v1";
    let model_id = [0xa5u8; 32];
    let reserved = vec![0u32, 1u32, 2u32]; // first three reserved.
    let prompt = synth_prompt(block, &model_id, 32, 256, &reserved).unwrap();
    let mut bytes = Vec::with_capacity(prompt.len() * 4);
    for t in &prompt {
        bytes.extend_from_slice(&t.to_le_bytes());
    }
    let actual = hash_canonical(&bytes);
    let expected: [u8; 32] = [
        0x28, 0x8d, 0xc2, 0x20, 0xa3, 0x58, 0x9d, 0x3b, 0xca, 0x85, 0x1e, 0x26, 0xea, 0x08, 0xc5,
        0x5e, 0x48, 0x40, 0x28, 0xcb, 0x94, 0x01, 0xea, 0x2d, 0x96, 0x9c, 0x9b, 0xfc, 0x15, 0x00,
        0x9f, 0x26,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} synth_prompt divergence");
}

#[test]
fn pin_comm_w_canonical_model() {
    // Tiny canonical Attention-flavored model. Tests that:
    // 1. comm_W is stable across runs (cross-arch).
    // 2. The full byte-pipeline (canonical_weight_bytes → tile-Merkle →
    //    derive_key) gives a fixed 32-byte output for fixed inputs.
    let hidden = 4u32;
    let hu = hidden as usize;
    let small = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap();
    let model = Model {
        dims: ModelDims {
            vocab: 8,
            hidden,
            seq_len: 4,
            activation_tile: 2,
        },
        arch_tag: [0u8; 16],
        feature_flags: 0,
        embed: canonical_input_i8(8 * hu, 0xa1a1_b2b2_c3c3_d4d4),
        layers: vec![LayerWeights::Attention {
            norm1: NormSpec::RmsNorm {
                gamma: canonical_input_i8(hu, 0xe5e5_f6f6_0707_1818),
                eps_q: 1,
                post_scale: small,
            },
            attn: AttentionWeights {
                hidden,
                num_q_heads: 1,
                num_kv_heads: 1,
                head_dim: 2,
                w_q: canonical_input_i8(hu * 2, 0x2929_3a3a_4b4b_5c5c),
                w_k: canonical_input_i8(hu * 2, 0x6d6d_7e7e_8f8f_90a0),
                w_v: canonical_input_i8(hu * 2, 0xb1b1_c2c2_d3d3_e4e4),
                w_o: canonical_input_i8(2 * hu, 0xf5f5_0606_1717_2828),
            },
            attn_scales: AttentionScales {
                q: small,
                k: small,
                v: small,
                score: small,
                attn_out: small,
                o: small,
            },
            norm2: NormSpec::RmsNorm {
                gamma: canonical_input_i8(hu, 0x3939_4a4a_5b5b_6c6c),
                eps_q: 1,
                post_scale: small,
            },
            ffn: FfnWeights {
                hidden,
                intermediate: hidden * 2,
                w_gate: canonical_input_i8(hu * (hu * 2), 0x7d7d_8e8e_9f9f_a0a0),
                w_up: canonical_input_i8(hu * (hu * 2), 0xb1b1_c2c2_d3d3_e4e4),
                w_down: canonical_input_i8((hu * 2) * hu, 0xf5f5_0606_1717_2828),
            },
            ffn_scales: FfnScales {
                gate: small,
                up: small,
                mid: small,
                down: small,
            },
        }],
        final_norm: Some(NormSpec::RmsNorm {
            gamma: canonical_input_i8(hu, 0x9999_aaaa_bbbb_cccc),
            eps_q: 1,
            post_scale: small,
        }),
        rope_tables: RopeTables::identity(4, 1),
        softmax_lut: ExpLut::uniform_test(),
        sigmoid_lut: ActivationLut::identity(),
        ffn_activation: ActivationLut::identity(),
    };
    let actual = compute_comm_w(&model);
    // Phase 2.10 manifest v2: comm_W now incorporates arch_tag and
    // feature_flags (both zero here). Pin refreshed accordingly.
    let expected: [u8; 32] = [
        0x91, 0x50, 0x48, 0xc3, 0xa6, 0x27, 0x0f, 0xce, 0x26, 0x09, 0x78, 0xa8, 0xf7, 0x3d, 0x99,
        0xf9, 0xbd, 0x00, 0x76, 0xf2, 0xbe, 0xa9, 0x9f, 0x7e, 0xc2, 0x01, 0xd8, 0x1a, 0x57, 0xe0,
        0x32, 0x43,
    ];
    assert_eq!(actual, expected, "{ARCH_TAG} comm_w divergence");
}

#[test]
fn pin_vi_proof_round_trip_canonical() {
    // Phase 3 end-to-end: build a small model, run mine_vi to a Some(proof)
    // with an easy target, then verify it via FullReplica. This pin
    // verifies that the prover/verifier are byte-stable across architectures
    // — any drift in any underlying op (matmul, rmsnorm, ffn, attention,
    // tile_state_hash, merkle_root) would either change the encoded proof
    // bytes or fail verify, both of which trip this test.
    let hidden = 4u32;
    let hu = hidden as usize;
    let intermediate = 8u32;
    let iu = intermediate as usize;
    let seq_len = 4u32;

    let small = Scale::from_num(1 << (SCALE_DENOM_LOG2 - 4)).unwrap();
    let model = Model {
        dims: ModelDims {
            vocab: 16,
            hidden,
            seq_len,
            activation_tile: 2,
        },
        arch_tag: [0u8; 16],
        feature_flags: 0,
        embed: canonical_input_i8(16 * hu, 0xfeed_beef_cafe_babe),
        layers: vec![LayerWeights::Attention {
            norm1: NormSpec::RmsNorm {
                gamma: canonical_input_i8(hu, 0x1234_5678_9abc_def0),
                eps_q: 1,
                post_scale: small,
            },
            attn: AttentionWeights {
                hidden,
                num_q_heads: 1,
                num_kv_heads: 1,
                head_dim: 2,
                w_q: canonical_input_i8(hu * 2, 0xa1a1_b2b2_c3c3_d4d4),
                w_k: canonical_input_i8(hu * 2, 0xe5e5_f6f6_0707_1818),
                w_v: canonical_input_i8(hu * 2, 0x2929_3a3a_4b4b_5c5c),
                w_o: canonical_input_i8(2 * hu, 0x6d6d_7e7e_8f8f_90a0),
            },
            attn_scales: AttentionScales {
                q: small,
                k: small,
                v: small,
                score: small,
                attn_out: small,
                o: small,
            },
            norm2: NormSpec::RmsNorm {
                gamma: canonical_input_i8(hu, 0xb1b1_c2c2_d3d3_e4e4),
                eps_q: 1,
                post_scale: small,
            },
            ffn: FfnWeights {
                hidden,
                intermediate,
                w_gate: canonical_input_i8(hu * iu, 0xf5f5_0606_1717_2828),
                w_up: canonical_input_i8(hu * iu, 0x3939_4a4a_5b5b_6c6c),
                w_down: canonical_input_i8(iu * hu, 0x7d7d_8e8e_9f9f_a0a0),
            },
            ffn_scales: FfnScales {
                gate: small,
                up: small,
                mid: small,
                down: small,
            },
        }],
        final_norm: None,
        rope_tables: RopeTables::identity(seq_len, 1),
        softmax_lut: ExpLut::uniform_test(),
        sigmoid_lut: ActivationLut::identity(),
        ffn_activation: ActivationLut::identity(),
    };
    let model_id = compute_comm_w(&model);
    let ctx = LayerContext {
        rope_tables: &model.rope_tables,
        softmax_lut: &model.softmax_lut,
        sigmoid_lut: &model.sigmoid_lut,
        ffn_activation: &model.ffn_activation,
    };
    let target = [0xffu8; 32];
    let opts = ProverOptions {
        target_layer: 0,
        sigma: 3,
        tile: 2,
    };
    let proof = mine_vi(
        &model, &model_id, &ctx, b"pin-block", b"pin-nonce", &target, opts,
    )
    .unwrap()
    .expect("expected Some on max target");

    // Pin the encoded proof bytes' hash. This is the strongest possible
    // determinism check: any byte of any commitment, any opening, any path
    // changes here.
    let bytes = proof.encode();
    let actual = hash_canonical(&bytes);
    // Phase 2.10: ViProof embeds the model's comm_W as `model_id`, so
    // any change in comm_W cascades into a new encoded-proof hash.
    let expected: [u8; 32] = [
        0xd2, 0x1f, 0xd9, 0xb9, 0xe6, 0xc9, 0x38, 0xe5, 0x6f, 0xce, 0x04, 0xc1, 0x01, 0x5c, 0x76,
        0x28, 0xcb, 0xa7, 0x57, 0xbe, 0x64, 0x2b, 0x4e, 0xc8, 0xeb, 0x5f, 0x37, 0x7e, 0x7a, 0x8b,
        0x07, 0xee,
    ];
    assert_eq!(
        actual, expected,
        "{ARCH_TAG} ViProof encoded bytes divergence"
    );

    // And the proof must verify against itself.
    verify_vi(
        &model,
        &ctx,
        b"pin-block",
        b"pin-nonce",
        &target,
        &proof,
        opts,
        VerifierMode::FullReplica,
    )
    .expect("self-verification must succeed");
}
