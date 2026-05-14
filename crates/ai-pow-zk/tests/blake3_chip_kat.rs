//! Known-answer tests for the vendored Pearl-compat BLAKE3 chip (M10.1b).
//!
//! Confirms that:
//!
//!   1. The reference scalar `reference_compression_output` matches what
//!      the `blake3` crate produces for the *same* key + 64-byte input
//!      under keyed-hash mode. This anchors the byte-compat claim — if
//!      this test passes, an honest miner using `blake3::Hasher::new_keyed`
//!      will produce the *same* `found_leaf` value the SNARK constrains
//!      the verifier to recompute, so Pearl ↔ Nockchain merge-mining is
//!      preserved.
//!   2. A round-trip prove+verify through the [`AiPowStarkConfig`] STARK
//!      stack succeeds on a one-call trace — proves that the AIR's
//!      constraints are satisfied when `flags`, `counter`, and
//!      `block_len` are populated correctly (upstream omits all three
//!      from the generator).
//!   3. Tampering with the trace cells rejects.

use ai_pow_zk::blake3_chip::{
    generate_trace_for_calls, reference_compression_output, Blake3HashCall, Blake3KeyedAir,
};
use ai_pow_zk::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
use ai_pow_zk::params::ZkParams;
use ai_pow_zk::Val;
use p3_uni_stark::{prove, verify};

fn test_zk_params() -> ZkParams {
    ZkParams {
        m: 8,
        k: 16,
        n: 8,
        noise_rank: 2,
        tile: 2,
        difficulty_bits: 0,
    }
}

/// BLAKE3 flag bits (from the spec).
const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const ROOT: u32 = 1 << 3;
const KEYED_HASH: u32 = 1 << 4;

/// Build the call parameters an honest miner would use to compute a
/// single-block (≤ 64 bytes) keyed root hash.
fn keyed_root_call(message_bytes: &[u8; 64], key: &[u8; 32]) -> Blake3HashCall {
    let mut message = [0u32; 16];
    for i in 0..16 {
        let mut chunk = [0u8; 4];
        chunk.copy_from_slice(&message_bytes[i * 4..(i + 1) * 4]);
        message[i] = u32::from_le_bytes(chunk);
    }
    let mut key_u32 = [0u32; 8];
    for i in 0..8 {
        let mut chunk = [0u8; 4];
        chunk.copy_from_slice(&key[i * 4..(i + 1) * 4]);
        key_u32[i] = u32::from_le_bytes(chunk);
    }
    Blake3HashCall {
        message,
        key: key_u32,
        counter: 0,
        block_len: 64,
        flags: CHUNK_START | CHUNK_END | ROOT | KEYED_HASH,
    }
}

/// Reference output → 32 bytes (LE concat of 8 u32s).
fn output_bytes(out: [u32; 8]) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for i in 0..8 {
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&out[i].to_le_bytes());
    }
    bytes
}

#[test]
fn reference_matches_blake3_crate_keyed_hash_all_zero_message() {
    // 64 bytes of zeros, key = [0u8; 32].
    let message_bytes = [0u8; 64];
    let key = [0u8; 32];

    let call = keyed_root_call(&message_bytes, &key);
    let chip_output = output_bytes(reference_compression_output(call));

    // Reference computation via `blake3` crate's keyed hash.
    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(&message_bytes);
    let blake3_output: [u8; 32] = *hasher.finalize().as_bytes();

    assert_eq!(
        chip_output, blake3_output,
        "Vendored chip's keyed-mode output must match blake3 crate"
    );
}

#[test]
fn reference_matches_blake3_crate_keyed_hash_random_message() {
    let message_bytes: [u8; 64] = std::array::from_fn(|i| ((i as u32 * 31 + 7) & 0xff) as u8);
    let key: [u8; 32] = std::array::from_fn(|i| ((i as u32 * 53 + 11) & 0xff) as u8);

    let call = keyed_root_call(&message_bytes, &key);
    let chip_output = output_bytes(reference_compression_output(call));

    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(&message_bytes);
    let blake3_output: [u8; 32] = *hasher.finalize().as_bytes();

    assert_eq!(chip_output, blake3_output);
}

#[test]
fn reference_matches_blake3_crate_pearl_tile_state_shape() {
    // Pearl's `TileState::keyed_hash` shape: 16 × i32 LE, single-slot
    // means slot 0 = m_final, slots 1..16 = 0. This is the exact input
    // M10.1a's `compute_found_leaf` already hashes out-of-band.
    let m_final: u32 = 0x1234_5678;
    let pow_key = [0xAAu8; 32];

    let mut message_bytes = [0u8; 64];
    message_bytes[..4].copy_from_slice(&(m_final as i32).to_le_bytes());
    let call = keyed_root_call(&message_bytes, &pow_key);
    let chip_output = output_bytes(reference_compression_output(call));

    // Match against ai-pow-zk's M10.1a out-of-circuit binding helper.
    let expected = ai_pow_zk::binding::compute_found_leaf(m_final, &pow_key);
    assert_eq!(
        chip_output, expected,
        "vendored chip's reference output must agree with the M10.1a out-of-circuit helper \
         (both compute BLAKE3-keyed of the same Pearl-style M_bytes)"
    );
}

#[test]
fn prove_and_verify_one_keyed_hash_through_ai_pow_config() {
    let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
    let air = Blake3KeyedAir::default();

    let message_bytes: [u8; 64] = std::array::from_fn(|i| (i as u8).wrapping_mul(7));
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(13));
    let call = keyed_root_call(&message_bytes, &key);

    let calls = vec![call];
    let trace = generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);
    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
    verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
        .expect("Pearl-compat keyed-hash trace must verify");
}

#[test]
fn prove_and_verify_two_keyed_hashes() {
    // Distinct (message, key) pairs — neither row is a no-op.
    let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
    let air = Blake3KeyedAir::default();

    let m1: [u8; 64] = std::array::from_fn(|i| ((i + 1) as u8).wrapping_mul(11));
    let k1: [u8; 32] = std::array::from_fn(|i| ((i + 1) as u8).wrapping_mul(17));
    let m2: [u8; 64] = std::array::from_fn(|i| ((i + 1) as u8).wrapping_mul(23));
    let k2: [u8; 32] = std::array::from_fn(|i| ((i + 1) as u8).wrapping_mul(31));

    let calls = vec![keyed_root_call(&m1, &k1), keyed_root_call(&m2, &k2)];
    let trace = generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);
    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
    verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[])
        .expect("Two-keyed-hash trace must verify");
}

#[test]
fn padding_row_satisfies_constraints() {
    // One real call + one padding row (`Blake3HashCall::zero()`) →
    // trace height 2. The padding row's all-zero call must satisfy
    // the AIR (otherwise the round-trip below fails on the FRI side).
    let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
    let air = Blake3KeyedAir::default();

    let m: [u8; 64] = [3u8; 64];
    let k: [u8; 32] = [5u8; 32];
    let calls = vec![keyed_root_call(&m, &k)];
    let trace = generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);

    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
    verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]).expect("Padded trace must verify");
}

#[test]
fn verify_rejects_tampered_input_bit() {
    let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
    let air = Blake3KeyedAir::default();

    let m: [u8; 64] = [2u8; 64];
    let k: [u8; 32] = [4u8; 32];
    let calls = vec![keyed_root_call(&m, &k)];
    let mut trace =
        generate_trace_for_calls::<Val>(&calls, CircuitConfig::TEST.log_blowup as usize);

    // Flip cell 0 to a non-boolean value. Cell 0 is an input bit.
    use p3_field::integers::QuotientMap;
    trace.values[0] = <Val as QuotientMap<u32>>::from_int(2);

    let proof = prove::<AiPowStarkConfig, _>(&cfg, &air, trace, &[]);
    let r = verify::<AiPowStarkConfig, _>(&cfg, &air, &proof, &[]);
    assert!(r.is_err(), "tampered trace must reject; got {r:?}");
}
