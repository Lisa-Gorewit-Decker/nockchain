//! Tile-state rotate-XOR chip — Pearl §4.5 inner update.
//!
//! Each row encodes one application of Pearl's rolling state update:
//!
//! ```text
//!   m_out = rotate_left_13(m_in) XOR x
//! ```
//!
//! where `m_in`, `x`, `m_out` are all 32-bit values reinterpreted as
//! Goldilocks field elements (via the same i32→u32→u64 path Pearl
//! uses). The slot-selection logic — i.e., "which of the 16 state
//! slots does this row update?" — lives at the composition layer
//! (M8). This chip just *is* the rotate-XOR primitive, validated end-
//! to-end through the FRI stack so the higher layers can compose it
//! with confidence.
//!
//! ## Trace layout
//!
//! Width = `3 + 64` = 67 columns per row.
//!
//! ```text
//!   col 0          : m_in           — 32-bit input as a field element
//!   col 1          : x              — 32-bit XOR-fold value
//!   col 2          : m_out          — claimed output of rotate_left_13(m_in) ^ x
//!   col 3 .. 35    : m_in_bits[32]  — LSB-first bit decomposition of m_in
//!   col 35 .. 67   : x_bits[32]     — LSB-first bit decomposition of x
//! ```
//!
//! Padding rows are all-zero; rotate(0) XOR 0 = 0, so they satisfy
//! every constraint trivially.
//!
//! ## Constraints
//!
//! For each row:
//!
//! 1. **Booleans.** Each of the 64 bit columns satisfies `b · (1 − b) = 0`.
//! 2. **Recomposition of `m_in`.** `m_in = Σ_{i=0..32} 2^i · m_in_bits[i]`.
//! 3. **Recomposition of `x`.** `x = Σ_{i=0..32} 2^i · x_bits[i]`.
//! 4. **Rotate-XOR equation.** For each output bit position `i`,
//!    `m_out_bit_i = m_in_bit_{(i − 13) mod 32} XOR x_bit_i`, and
//!    `m_out = Σ_{i=0..32} 2^i · m_out_bit_i`. Encoded as one
//!    aggregate constraint:
//!    ```text
//!    m_out = Σ_i 2^i · (m_in_bit_{src(i)} + x_bit_i − 2 · m_in_bit_{src(i)} · x_bit_i)
//!    ```
//!    where `src(i) = (i − 13).rem_euclid(32)`.
//!
//! The XOR identity `a ⊕ b = a + b − 2·a·b` holds for booleans, so the
//! aggregate constraint is degree-2 in trace columns — compatible
//! with `log_blowup = 1` in the TEST profile.

use p3_air::{Air, AirBuilder, BaseAir, WindowAccess};
use p3_matrix::dense::RowMajorMatrix;

use crate::circuit::Val;

/// Number of bits per word.
pub const W: usize = 32;

/// Pearl §4.5 rotate amount.
pub const ROT: usize = 13;

/// Column index of `m_in` in the row.
pub const COL_M_IN: usize = 0;
/// Column index of `x` in the row.
pub const COL_X: usize = 1;
/// Column index of `m_out` in the row.
pub const COL_M_OUT: usize = 2;
/// First column of `m_in`'s bit decomposition.
pub const COL_M_IN_BITS: usize = 3;
/// First column of `x`'s bit decomposition.
pub const COL_X_BITS: usize = COL_M_IN_BITS + W;
/// Total trace width.
pub const WIDTH: usize = COL_X_BITS + W;

/// AIR for the rotate-XOR-13 state update primitive.
#[derive(Debug, Clone, Copy, Default)]
pub struct StateChipAir;

impl StateChipAir {
    pub const fn new() -> Self {
        Self
    }

    /// Build a trace from a sequence of `(m_in, x)` pairs. Each pair
    /// is laid out in one row; the row's `m_out` column is filled in
    /// with the reference rotate-XOR result.
    ///
    /// The trace is padded with all-zero rows to the next power of
    /// two (FRI requires power-of-two height). Plain-zero rows
    /// satisfy every constraint trivially.
    pub fn generate_trace(ops: &[(i32, i32)]) -> RowMajorMatrix<Val> {
        let n = ops.len().next_power_of_two().max(1);
        let mut flat: Vec<Val> = Vec::with_capacity(n * WIDTH);

        for &(m_in, x) in ops {
            let m_in_u = m_in as u32;
            let x_u = x as u32;
            let m_out_u = m_in_u.rotate_left(ROT as u32) ^ x_u;

            flat.push(u32_to_val(m_in_u));
            flat.push(u32_to_val(x_u));
            flat.push(u32_to_val(m_out_u));
            for i in 0..W {
                flat.push(bit_to_val((m_in_u >> i) & 1));
            }
            for i in 0..W {
                flat.push(bit_to_val((x_u >> i) & 1));
            }
        }
        for _ in ops.len()..n {
            for _ in 0..WIDTH {
                flat.push(Val::default());
            }
        }
        RowMajorMatrix::new(flat, WIDTH)
    }

    /// For one `(m_in, x)` pair, return the reference output value
    /// Pearl computes. Used by tests and (eventually) the witness
    /// builder.
    #[inline]
    pub fn reference(m_in: i32, x: i32) -> i32 {
        ((m_in as u32).rotate_left(ROT as u32) ^ x as u32) as i32
    }
}

impl<F> BaseAir<F> for StateChipAir {
    fn width(&self) -> usize {
        WIDTH
    }
}

impl<AB: AirBuilder> Air<AB> for StateChipAir {
    fn eval(&self, builder: &mut AB) {
        use p3_field::PrimeCharacteristicRing;

        let main = builder.main();
        let row = main.current_slice();

        let m_in: AB::Var = row[COL_M_IN];
        let x: AB::Var = row[COL_X];
        let m_out: AB::Var = row[COL_M_OUT];

        // Snapshot the 64 bit columns into typed arrays so we can index
        // them cleanly.
        let mut m_in_bits: [AB::Var; W] = [m_in; W];
        let mut x_bits: [AB::Var; W] = [m_in; W];
        for i in 0..W {
            m_in_bits[i] = row[COL_M_IN_BITS + i];
            x_bits[i] = row[COL_X_BITS + i];
        }

        // 1. Booleans.
        builder.assert_bools(m_in_bits);
        builder.assert_bools(x_bits);

        // Precompute the field constants 2^i. `AB::F` may not be Copy
        // (packed types), so clone each step.
        let two = <AB::F as PrimeCharacteristicRing>::TWO;
        let mut pows: Vec<AB::F> = Vec::with_capacity(W);
        let mut cur_pow = <AB::F as PrimeCharacteristicRing>::ONE;
        for _ in 0..W {
            pows.push(cur_pow.clone());
            cur_pow = cur_pow * two.clone();
        }

        // 2. m_in = Σ 2^i · m_in_bits[i].
        {
            let mut sum: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
            for i in 0..W {
                sum = sum + m_in_bits[i] * pows[i].clone();
            }
            builder.assert_eq(m_in, sum);
        }

        // 3. x = Σ 2^i · x_bits[i].
        {
            let mut sum: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
            for i in 0..W {
                sum = sum + x_bits[i] * pows[i].clone();
            }
            builder.assert_eq(x, sum);
        }

        // 4. m_out = Σ 2^i · (m_in_bit_src + x_bit_i − 2 · m_in_bit_src · x_bit_i)
        //    where src(i) = (i − ROT) mod W.
        {
            let mut sum: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
            for i in 0..W {
                let src = (i + W - ROT) % W;
                let a: AB::Var = m_in_bits[src];
                let b: AB::Var = x_bits[i];
                // XOR via booleans: a + b − 2·a·b.
                let xor_expr: AB::Expr =
                    AB::Expr::from(a) + AB::Expr::from(b) - AB::Expr::from(a) * b * two.clone();
                sum = sum + xor_expr * pows[i].clone();
            }
            builder.assert_eq(m_out, sum);
        }
    }
}

#[inline]
fn u32_to_val(v: u32) -> Val {
    use p3_field::integers::QuotientMap;
    <Val as QuotientMap<u32>>::from_int(v)
}

#[inline]
fn bit_to_val(b: u32) -> Val {
    use p3_field::integers::QuotientMap;
    debug_assert!(b <= 1);
    <Val as QuotientMap<u32>>::from_int(b)
}

#[cfg(test)]
mod tests {
    use p3_field::PrimeField64;
    use p3_uni_stark::{prove, verify};

    use super::*;
    use crate::circuit::{build_stark_config, CircuitConfig};
    use crate::params::ZkParams;

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

    /// Mirrors `ai_pow::matmul::TileState::fold` semantics for one slot.
    fn pearl_rotate_xor(m_in: i32, x: i32) -> i32 {
        let rotated = (m_in as u32).rotate_left(ROT as u32) as i32;
        rotated ^ x
    }

    #[test]
    fn width_is_pinned() {
        assert_eq!(WIDTH, 67);
        assert_eq!(<StateChipAir as BaseAir<Val>>::width(&StateChipAir), 67);
    }

    #[test]
    fn reference_matches_pearl_for_all_powers_of_two() {
        // rotate_left(13) is bit-position arithmetic — exercise the
        // top of the word and the wrap.
        for shift in 0..32 {
            let m_in = 1i32.wrapping_shl(shift);
            let x = 0i32;
            assert_eq!(
                StateChipAir::reference(m_in, x),
                pearl_rotate_xor(m_in, x),
                "shift={shift}",
            );
        }
    }

    #[test]
    fn reference_matches_pearl_with_x_xor() {
        // Force x to be the same bit pattern as the rotated input —
        // result must be zero.
        let m_in: i32 = 0x12345678u32 as i32;
        let x: i32 = (m_in as u32).rotate_left(ROT as u32) as i32;
        assert_eq!(StateChipAir::reference(m_in, x), 0);
    }

    #[test]
    fn trace_layout_matches_spec() {
        let ops = [(0x1u32 as i32, 0i32)];
        let trace = StateChipAir::generate_trace(&ops);
        // 1 row → padded to 1 (already power of two).
        let h = trace.values.len() / WIDTH;
        assert_eq!(h, 1);
        // m_in canonical = 1.
        assert_eq!(trace.values[COL_M_IN].as_canonical_u64(), 1);
        // m_in_bits[0] = 1, m_in_bits[1..] = 0.
        assert_eq!(trace.values[COL_M_IN_BITS].as_canonical_u64(), 1);
        for i in 1..W {
            assert_eq!(trace.values[COL_M_IN_BITS + i].as_canonical_u64(), 0);
        }
        // rotate_left(13) of bit-0 sets bit-13 in output.
        assert_eq!(trace.values[COL_M_OUT].as_canonical_u64(), 1u64 << 13,);
    }

    #[test]
    fn prove_and_verify_single_rotate_only() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let trace = StateChipAir::generate_trace(&[(0x12345678u32 as i32, 0)]);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("rotate-only trace must verify");
    }

    #[test]
    fn prove_and_verify_xor_only() {
        // m_in = 0 → rotate(0) = 0 → m_out = x.
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let trace = StateChipAir::generate_trace(&[(0, 0x0F0F_F0F0u32 as i32)]);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("xor-only trace must verify");
    }

    #[test]
    fn prove_and_verify_sequence() {
        // 8 operations exercising different bit patterns.
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let ops: Vec<(i32, i32)> = (0..8)
            .map(|i| {
                let m = 0x12345678u32.wrapping_mul((i as u32).wrapping_add(1));
                let x = 0xAABBCCDDu32.wrapping_add(i as u32 * 17);
                (m as i32, x as i32)
            })
            .collect();
        let trace = StateChipAir::generate_trace(&ops);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("sequence trace must verify");
    }

    #[test]
    fn prove_and_verify_negative_values() {
        // Negative i32 round-trips through u32 reinterpret.
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let trace = StateChipAir::generate_trace(&[(-1i32, -1i32), (i32::MIN, i32::MAX), (-7, 11)]);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("negative-value trace must verify");
    }

    #[test]
    fn prove_and_verify_pearl_fold_chain() {
        // Simulate Pearl's TileState::fold over a sequence of 16 steps
        // hitting slot 0 (steps 0, 16, 32, …) — i.e., chain rotate-XORs.
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let xs: [i32; 8] = [
            0x1111_1111u32 as i32, 0x2222_2222u32 as i32, 0x3333_3333u32 as i32,
            0x4444_4444u32 as i32, 0x5555_5555u32 as i32, 0x6666_6666u32 as i32,
            0x7777_7777u32 as i32, 0x8888_8888u32 as i32,
        ];
        let mut m: i32 = 0;
        let ops: Vec<(i32, i32)> = xs
            .iter()
            .map(|&x| {
                let entry = (m, x);
                m = pearl_rotate_xor(m, x);
                entry
            })
            .collect();
        let trace = StateChipAir::generate_trace(&ops);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("Pearl fold chain must verify");
    }

    #[test]
    fn padding_rows_are_zero() {
        // 3 ops → padded to 4.
        let ops = [(1i32, 2i32), (-3i32, 4i32), (5i32, -6i32)];
        let trace = StateChipAir::generate_trace(&ops);
        let h = trace.values.len() / WIDTH;
        assert_eq!(h, 4);
        let row3 = &trace.values[3 * WIDTH..4 * WIDTH];
        for &v in row3 {
            assert_eq!(v.as_canonical_u64(), 0);
        }
    }

    #[test]
    fn verify_rejects_tampered_m_out() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let mut trace = StateChipAir::generate_trace(&[(0x12345678u32 as i32, 0)]);
        // Replace m_out with a wrong value (rotate-XOR equation must fail).
        trace.values[COL_M_OUT] = u32_to_val(0xDEADBEEF);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "tampered m_out must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_non_boolean_bit() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let mut trace = StateChipAir::generate_trace(&[(1i32, 0i32)]);
        // Set m_in_bits[1] to 2 (non-boolean).
        trace.values[COL_M_IN_BITS + 1] = u32_to_val(2);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "non-boolean bit must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_inconsistent_m_in_bits() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        // m_in = 5 → bits [1, 0, 1, 0, …]. Tamper with bits to read as 7.
        let mut trace = StateChipAir::generate_trace(&[(5i32, 0i32)]);
        // Flip m_in_bits[1] from 0 to 1, leaving m_in=5: recomposition
        // breaks (bits sum to 7 ≠ 5).
        trace.values[COL_M_IN_BITS + 1] = u32_to_val(1);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "inconsistent m_in bits must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_inconsistent_x_bits() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = StateChipAir::new();
        let mut trace = StateChipAir::generate_trace(&[(0i32, 5i32)]);
        // Flip x_bits[1] from 0 to 1, leaving x = 5: recomposition breaks.
        trace.values[COL_X_BITS + 1] = u32_to_val(1);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "inconsistent x bits must reject; got {r:?}");
    }
}
