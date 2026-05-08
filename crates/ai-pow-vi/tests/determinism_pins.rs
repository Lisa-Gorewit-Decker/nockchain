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
use ai_pow_vi::determinism::{hash_canonical, ARCH_TAG};
use ai_pow_vi::quant::{rescale_and_requantize, Scale, SCALE_DENOM_LOG2};
use ai_pow_vi::rmsnorm::{isqrt_floor, rmsnorm, DEFAULT_EPS_Q};

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
