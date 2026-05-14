// Patched trace generator for the vendored Blake3 AIR.
//
// Vendored from Plonky3/Plonky3 @ af65376 (`blake3-air/src/generation.rs`).
// Patches:
//   * `generate_trace_rows_for_perm` now takes `(counter, block_len,
//     flags)` and writes all three into the trace row (upstream
//     hard-codes `flags = 0`, which prevents the chip from computing
//     BLAKE3 keyed-mode hashes).
//   * The `state[3]` initialization uses `flags` instead of `0` so
//     the constraint and the trace agree.
//   * A `generate_trace_for_calls` entry point takes a `&[Blake3HashCall]`
//     so callers can specify all per-row parameters. Pads to the next
//     power of two with a default no-op hash so a single user-supplied
//     call still produces a power-of-two-height trace.
//   * The upstream random-input `generate_trace_rows` helper is dropped
//     (Pearl-compat hashes always need concrete inputs).

use core::array;

use p3_air::utils::u32_to_bits_le;
use p3_field::{PrimeCharacteristicRing, PrimeField64};
use p3_matrix::dense::RowMajorMatrix;

use super::columns::{Blake3Cols, Blake3State, FullRound, NUM_BLAKE3_COLS};
use super::constants::{permute, IV};

/// One BLAKE3 compression-function call. The chip-level AIR proves
/// `BLAKE3-compression(state_init(key, counter, block_len, flags),
/// message) = outputs` for each call.
///
/// For a single-block keyed-mode root hash (the M10.1b found_leaf
/// case), the parameters are:
///
/// ```text
///   message    = 64-byte input split into 16 u32 LE words
///   key        = 32-byte BLAKE3 key split into 8 u32 LE words
///   counter    = 0
///   block_len  = actual message byte length (e.g. 64)
///   flags      = CHUNK_START | CHUNK_END | ROOT | KEYED_HASH = 0x1B
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Blake3HashCall {
    pub message: [u32; 16],
    pub key: [u32; 8],
    pub counter: u64,
    pub block_len: u32,
    pub flags: u32,
}

impl Blake3HashCall {
    /// Build a no-op default that satisfies the AIR constraints (all
    /// zero inputs). Used for padding rows when the caller asks for
    /// fewer hashes than the next power-of-two trace height.
    pub const fn zero() -> Self {
        Self {
            message: [0; 16],
            key: [0; 8],
            counter: 0,
            block_len: 0,
            flags: 0,
        }
    }
}

impl Default for Blake3HashCall {
    fn default() -> Self {
        Self::zero()
    }
}

/// Build a Blake3 keyed-mode trace from a slice of per-row hash calls.
///
/// `calls.len()` is padded up to the next power of two; padding rows
/// run `Blake3HashCall::zero()` so the AIR constraints are satisfied
/// trivially.
///
/// `extra_capacity_bits` reserves leading capacity in the values vec
/// to accommodate the FRI DFT (mirrors upstream's behaviour).
pub fn generate_trace_for_calls<F: PrimeField64>(
    calls: &[Blake3HashCall],
    extra_capacity_bits: usize,
) -> RowMajorMatrix<F> {
    let num_rows = calls.len().next_power_of_two().max(1);
    let trace_length = num_rows * NUM_BLAKE3_COLS;

    let mut long_trace = F::zero_vec(trace_length << extra_capacity_bits);
    long_trace.truncate(trace_length);

    let mut trace = RowMajorMatrix::new(long_trace, NUM_BLAKE3_COLS);
    let (prefix, rows, suffix) = unsafe { trace.values.align_to_mut::<Blake3Cols<F>>() };
    assert!(prefix.is_empty(), "Alignment should match");
    assert!(suffix.is_empty(), "Alignment should match");
    assert_eq!(rows.len(), num_rows);

    for (row_idx, row) in rows.iter_mut().enumerate() {
        let call = calls.get(row_idx).copied().unwrap_or_default();
        generate_trace_rows_for_perm(row, call);
    }
    trace
}

/// One row = one full BLAKE3 compression. Patched from upstream to
/// accept `(counter, block_len, flags)` explicitly and to write all
/// three into the trace's per-bit columns. The state[3] init now
/// uses `flags` rather than the upstream-hard-coded `0`.
pub(super) fn generate_trace_rows_for_perm<F: PrimeField64>(
    row: &mut Blake3Cols<F>,
    call: Blake3HashCall,
) {
    let Blake3HashCall {
        message,
        key,
        counter,
        block_len,
        flags,
    } = call;

    // M_vec: 16 message words (block input).
    row.inputs = array::from_fn(|i| u32_to_bits_le(message[i]));

    // Chaining values: 8 u32s, laid out as [4 × row0_state, 4 × row1_state].
    // For keyed mode these are the key split into 8 LE u32s.
    row.chaining_values = array::from_fn(|i| array::from_fn(|j| u32_to_bits_le(key[4 * i + j])));

    row.counter_low = u32_to_bits_le(counter as u32);
    row.counter_hi = u32_to_bits_le((counter >> 32) as u32);
    row.block_len = u32_to_bits_le(block_len);
    // ── Patch: populate `flags` (upstream leaves this as all-zero).
    row.flags = u32_to_bits_le(flags);

    row.initial_row0 =
        array::from_fn(|i| [F::from_u16(key[i] as u16), F::from_u16((key[i] >> 16) as u16)]);
    row.initial_row2 = array::from_fn(|i| [F::from_u16(IV[i][0]), F::from_u16(IV[i][1])]);

    // Scalar mirror of the AIR state. Used to compute the round
    // outputs we save back into the trace.
    let mut m_vec: [u32; 16] = message;
    let mut state = [
        [key[0], key[1], key[2], key[3]],
        [key[4], key[5], key[6], key[7]],
        [
            (IV[0][0] as u32) + ((IV[0][1] as u32) << 16),
            (IV[1][0] as u32) + ((IV[1][1] as u32) << 16),
            (IV[2][0] as u32) + ((IV[2][1] as u32) << 16),
            (IV[3][0] as u32) + ((IV[3][1] as u32) << 16),
        ],
        [
            counter as u32,
            (counter >> 32) as u32,
            block_len,
            // ── Patch: use `flags`, not the upstream-hard-coded 0.
            flags,
        ],
    ];

    generate_trace_row_for_round(&mut row.full_rounds[0], &mut state, &m_vec); // round 1
    permute(&mut m_vec);
    generate_trace_row_for_round(&mut row.full_rounds[1], &mut state, &m_vec); // round 2
    permute(&mut m_vec);
    generate_trace_row_for_round(&mut row.full_rounds[2], &mut state, &m_vec); // round 3
    permute(&mut m_vec);
    generate_trace_row_for_round(&mut row.full_rounds[3], &mut state, &m_vec); // round 4
    permute(&mut m_vec);
    generate_trace_row_for_round(&mut row.full_rounds[4], &mut state, &m_vec); // round 5
    permute(&mut m_vec);
    generate_trace_row_for_round(&mut row.full_rounds[5], &mut state, &m_vec); // round 6
    permute(&mut m_vec);
    generate_trace_row_for_round(&mut row.full_rounds[6], &mut state, &m_vec); // round 7

    // Compression finalisation: the outputs are the standard
    // `state ^ chaining_in` XORs upstream computes.
    row.final_round_helpers = array::from_fn(|i| u32_to_bits_le(state[2][i]));
    row.outputs[0] = array::from_fn(|i| u32_to_bits_le(state[0][i] ^ state[2][i]));
    row.outputs[1] = array::from_fn(|i| u32_to_bits_le(state[1][i] ^ state[3][i]));
    // The XOR with `input[16 + i]` upstream is the original chaining
    // value (= key[i] in keyed mode). state[0..4] of the chaining
    // input for keyed-mode compression is `key`.
    row.outputs[2] = array::from_fn(|i| u32_to_bits_le(state[2][i] ^ key[i]));
    row.outputs[3] = array::from_fn(|i| u32_to_bits_le(state[3][i] ^ key[4 + i]));
}

fn generate_trace_row_for_round<F: PrimeField64>(
    round_data: &mut FullRound<F>,
    state: &mut [[u32; 4]; 4],
    m_vec: &[u32; 16],
) {
    // We populate the round_data as we iterate through and compute the permutation following the reference implementation.

    // We start by performing the first half of the four column quarter round functions.
    (0..4).for_each(|i| {
        (state[0][i], state[1][i], state[2][i], state[3][i]) = verifiable_half_round(
            state[0][i],
            state[1][i],
            state[2][i],
            state[3][i],
            m_vec[2 * i],
            false,
        );
    });

    // After the first four operations we need to save a copy of the state into the trace.
    save_state_to_trace(&mut round_data.state_prime, state);

    // Next we do the second half of the four column quarter round functions.
    (0..4).for_each(|i| {
        (state[0][i], state[1][i], state[2][i], state[3][i]) = verifiable_half_round(
            state[0][i],
            state[1][i],
            state[2][i],
            state[3][i],
            m_vec[2 * i + 1],
            true,
        );
    });

    // Again we save another copy of the state.
    save_state_to_trace(&mut round_data.state_middle, state);

    // We repeat with the diagonals quarter round function.

    // Do the first half of the four diagonal quarter round functions.
    (0..4).for_each(|i| {
        (
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
        ) = verifiable_half_round(
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
            m_vec[8 + 2 * i],
            false,
        );
    });

    // Save a copy of the state to the trace.
    save_state_to_trace(&mut round_data.state_middle_prime, state);

    // Do the second half of the four diagonal quarter round functions.
    (0..4).for_each(|i| {
        (
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
        ) = verifiable_half_round(
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
            m_vec[9 + 2 * i],
            true,
        );
    });

    // Save a copy of the state to the trace.
    save_state_to_trace(&mut round_data.state_output, state);
}

/// Perform half of a quarter round on the given elements.
///
/// The boolean flag, indicates whether this is the first (false) or second (true) half round.
const fn verifiable_half_round(
    mut a: u32,
    mut b: u32,
    mut c: u32,
    mut d: u32,
    m: u32,
    flag: bool,
) -> (u32, u32, u32, u32) {
    let (rot_1, rot_2) = if flag { (8, 7) } else { (16, 12) };

    // The first summation:
    a = a.wrapping_add(b);
    a = a.wrapping_add(m);

    // The first xor:
    d = (d ^ a).rotate_right(rot_1);

    // The second summation:
    c = c.wrapping_add(d);

    // The second xor:
    b = (b ^ c).rotate_right(rot_2);

    (a, b, c, d)
}

fn save_state_to_trace<R: PrimeCharacteristicRing>(
    trace: &mut Blake3State<R>,
    state: &[[u32; 4]; 4],
) {
    trace.row0 = array::from_fn(|i| {
        [R::from_u16(state[0][i] as u16), R::from_u16((state[0][i] >> 16) as u16)]
    });
    trace.row1 = array::from_fn(|i| u32_to_bits_le(state[1][i]));
    trace.row2 = array::from_fn(|i| {
        [R::from_u16(state[2][i] as u16), R::from_u16((state[2][i] >> 16) as u16)]
    });
    trace.row3 = array::from_fn(|i| u32_to_bits_le(state[3][i]));
}

/// Reference scalar BLAKE3 compression output (8 u32s = 32 bytes).
///
/// Used by tests to anchor what the AIR proves against the upstream
/// `blake3` crate's keyed-hash result for known inputs.
pub fn reference_compression_output(call: Blake3HashCall) -> [u32; 8] {
    let mut m_vec: [u32; 16] = call.message;
    let mut state = [
        [call.key[0], call.key[1], call.key[2], call.key[3]],
        [call.key[4], call.key[5], call.key[6], call.key[7]],
        [
            (IV[0][0] as u32) + ((IV[0][1] as u32) << 16),
            (IV[1][0] as u32) + ((IV[1][1] as u32) << 16),
            (IV[2][0] as u32) + ((IV[2][1] as u32) << 16),
            (IV[3][0] as u32) + ((IV[3][1] as u32) << 16),
        ],
        [call.counter as u32, (call.counter >> 32) as u32, call.block_len, call.flags],
    ];

    for round in 0..7 {
        scalar_round(&mut state, &m_vec);
        if round < 6 {
            permute(&mut m_vec);
        }
    }

    // Same finalisation XORs the AIR computes (`outputs[0]` and
    // `outputs[1]` are the first 8 u32s of the compression output).
    let mut out = [0u32; 8];
    for i in 0..4 {
        out[i] = state[0][i] ^ state[2][i];
        out[4 + i] = state[1][i] ^ state[3][i];
    }
    out
}

fn scalar_round(state: &mut [[u32; 4]; 4], m_vec: &[u32; 16]) {
    for i in 0..4 {
        (state[0][i], state[1][i], state[2][i], state[3][i]) = verifiable_half_round(
            state[0][i],
            state[1][i],
            state[2][i],
            state[3][i],
            m_vec[2 * i],
            false,
        );
    }
    for i in 0..4 {
        (state[0][i], state[1][i], state[2][i], state[3][i]) = verifiable_half_round(
            state[0][i],
            state[1][i],
            state[2][i],
            state[3][i],
            m_vec[2 * i + 1],
            true,
        );
    }
    for i in 0..4 {
        (
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
        ) = verifiable_half_round(
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
            m_vec[8 + 2 * i],
            false,
        );
    }
    for i in 0..4 {
        (
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
        ) = verifiable_half_round(
            state[0][i],
            state[1][(i + 1) % 4],
            state[2][(i + 2) % 4],
            state[3][(i + 3) % 4],
            m_vec[9 + 2 * i],
            true,
        );
    }
}
