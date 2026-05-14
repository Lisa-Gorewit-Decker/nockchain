//! Input / range chip — bit-decomposition range checks.
//!
//! Pearl's `pearl_air.rs:62-66` uses dedicated range tables for u8, u13,
//! i7+1, i8, and i8↔u8 conversion. Plonky3 doesn't ship a generic
//! range-check primitive, so we encode the same guarantee via the
//! standard **bit-decomposition** trick: per row, one "value" column
//! plus `BITS` boolean witness columns, with constraints
//!
//! ```text
//!   for each bit:  bit · (1 − bit) = 0           (boolean check)
//!   recomposition: value = Σ_i 2^i · bit_i        (assemble u_BITS)
//! ```
//!
//! Together these gate the value column into `[0, 2^BITS)`. The trace
//! generator decomposes each i8/i7/u8 value into its bits.
//!
//! Subsequent chips (`crate::air::MatmulAir`, the future jackpot chip,
//! etc.) attach their value columns to range-check rows through column
//! offsets; this module owns the constraint encoding itself.

use core::array;
use core::borrow::Borrow;

use p3_air::{Air, AirBuilder, BaseAir, WindowAccess};
use p3_field::PrimeCharacteristicRing;
use p3_matrix::dense::RowMajorMatrix;

use crate::circuit::Val;

/// AIR for an N-bit range check.
///
/// Width = `BITS + 1` columns per row:
///
/// ```text
///   [ value | bit_0 | bit_1 | ... | bit_{BITS-1} ]
/// ```
///
/// One row per value to check. Constraints are independent per row;
/// padding rows of all-zeros pass trivially (value = 0, all bits = 0).
#[derive(Debug, Clone, Copy)]
pub struct RangeAir<const BITS: usize>;

impl<const BITS: usize> RangeAir<BITS> {
    pub const fn new() -> Self {
        Self
    }

    /// Number of trace columns.
    pub const fn width() -> usize {
        BITS + 1
    }

    /// Generate a trace whose rows decompose `values` into `BITS` bits.
    ///
    /// The trace is padded with all-zero rows up to the next power of
    /// two (Plonky3's FRI requires a power-of-two height). All-zero
    /// rows trivially satisfy the constraints.
    ///
    /// # Panics
    ///
    /// Panics if any `values[i] >= 2^BITS` since the bit-decomposition
    /// can't represent it.
    pub fn generate_trace(values: &[u64]) -> RowMajorMatrix<Val> {
        const {
            assert!(BITS > 0 && BITS <= 32, "BITS must be in 1..=32");
        }
        let n = values.len().next_power_of_two().max(1);
        let width = Self::width();
        let mut flat = Vec::with_capacity(n * width);
        for &v in values {
            assert!(v < (1u64 << BITS), "value {v} out of range for BITS={BITS}");
            flat.push(u64_to_val(v));
            for i in 0..BITS {
                let bit = (v >> i) & 1;
                flat.push(u64_to_val(bit));
            }
        }
        // Pad with all-zero rows.
        for _ in values.len()..n {
            for _ in 0..width {
                flat.push(u64_to_val(0));
            }
        }
        RowMajorMatrix::new(flat, width)
    }
}

impl<const BITS: usize> Default for RangeAir<BITS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F, const BITS: usize> BaseAir<F> for RangeAir<BITS> {
    fn width(&self) -> usize {
        Self::width()
    }
}

impl<AB: AirBuilder, const BITS: usize> Air<AB> for RangeAir<BITS> {
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let row = main.current_slice();
        let value: <AB as AirBuilder>::Var = *row.first().expect("RangeAir row missing value");
        let bits: [<AB as AirBuilder>::Var; BITS] =
            array::from_fn(|i| *row.get(i + 1).expect("RangeAir row missing bit column"));
        // Boolean check per bit.
        builder.assert_bools(bits);
        // Recomposition: value = Σ 2^i · bit_i.
        // `AB::F` may not be `Copy` (it can be a packed type), so clone
        // each accumulator iteration.
        let mut sum = <AB::Expr as PrimeCharacteristicRing>::ZERO;
        let two = <AB::F as PrimeCharacteristicRing>::TWO;
        let mut pow = <AB::F as PrimeCharacteristicRing>::ONE;
        for &b in bits.iter() {
            sum = sum + b * pow.clone();
            pow = pow * two.clone();
        }
        builder.assert_eq(value, sum);
    }
}

#[inline]
fn u64_to_val(v: u64) -> Val {
    use p3_field::integers::QuotientMap;
    <Val as QuotientMap<u64>>::from_int(v)
}

// `Borrow` is not used here because we work with raw row slices instead
// of a typed `RangeCols` struct; the slice-direct path stays simple.
#[allow(dead_code)]
fn _borrow_keeper(_: &dyn Borrow<()>) {}

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

    #[test]
    fn width_is_bits_plus_one() {
        assert_eq!(<RangeAir<8> as BaseAir<Val>>::width(&RangeAir::<8>), 9);
        assert_eq!(<RangeAir<13> as BaseAir<Val>>::width(&RangeAir::<13>), 14);
        assert_eq!(<RangeAir<1> as BaseAir<Val>>::width(&RangeAir::<1>), 2);
    }

    #[test]
    fn trace_has_power_of_two_rows() {
        let trace = RangeAir::<8>::generate_trace(&[5u64, 17, 42]);
        // 3 values → next power of two is 4.
        let height = trace.values.len() / trace.width;
        assert_eq!(height, 4);
        assert_eq!(trace.width, 9);
    }

    #[test]
    fn trace_decomposition_recovers_value() {
        // Pick a value, hand-decompose, confirm the trace matches.
        let v: u64 = 0b1010_1100; // 172
        let trace = RangeAir::<8>::generate_trace(&[v]);
        let width = trace.width;
        let row = &trace.values[0..width];
        assert_eq!(row[0].as_canonical_u64(), v);
        // bit_0..bit_7 in LSB-first order.
        for i in 0..8 {
            let expected = (v >> i) & 1;
            assert_eq!(row[i + 1].as_canonical_u64(), expected);
        }
    }

    #[test]
    #[should_panic(expected = "out of range for BITS=8")]
    fn trace_panics_on_out_of_range_value() {
        let _ = RangeAir::<8>::generate_trace(&[256]);
    }

    #[test]
    fn padding_rows_are_all_zero() {
        // 3 values → padded to 4 rows. Rows 0..3 carry the values; row 3 is padding.
        let trace = RangeAir::<8>::generate_trace(&[7, 11, 13]);
        let w = trace.width;
        let height = trace.values.len() / w;
        assert_eq!(height, 4);
        let row3 = &trace.values[3 * w..4 * w];
        for &v in row3 {
            assert_eq!(v.as_canonical_u64(), 0);
        }
    }

    /// Prove + verify a valid trace end-to-end through the full
    /// AiPowStarkConfig stack. This exercises Tip5Perm + MerkleTreeMmcs +
    /// FRI together with the RangeAir constraints.
    #[test]
    fn prove_and_verify_valid_u8_trace() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = RangeAir::<8>::new();
        let trace = RangeAir::<8>::generate_trace(&[0u64, 1, 7, 42, 128, 255, 17, 99]);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("valid trace must verify");
    }

    /// Tamper one bit column entry so the bool constraint fails. The
    /// verifier must reject.
    #[test]
    fn verify_rejects_tampered_bool_bit() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = RangeAir::<8>::new();
        let mut trace = RangeAir::<8>::generate_trace(&[5u64, 10, 15, 20]);
        // Replace bit_0 of row 0 (column 1) with 2 (not in {0, 1}).
        trace.values[1] = u64_to_val(2);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "tampered bool bit must reject; got {r:?}");
    }

    /// Tamper the value column so recomposition fails (bits don't sum
    /// to value). The verifier must reject.
    #[test]
    fn verify_rejects_tampered_recomposition() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = RangeAir::<8>::new();
        let mut trace = RangeAir::<8>::generate_trace(&[5u64, 10, 15, 20]);
        // Replace value of row 0 (column 0) with 100; bits still decode to 5.
        trace.values[0] = u64_to_val(100);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "value/bits mismatch must reject; got {r:?}");
    }

    /// All four boundary u8 values must verify.
    #[test]
    fn prove_and_verify_u8_boundaries() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = RangeAir::<8>::new();
        let trace = RangeAir::<8>::generate_trace(&[0, 1, 127, 128, 254, 255, 0, 0]);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("u8 boundary trace must verify");
    }

    /// 13-bit range test (Pearl's u13 table analog).
    #[test]
    fn prove_and_verify_u13_trace() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = RangeAir::<13>::new();
        let trace = RangeAir::<13>::generate_trace(&[0u64, 1, 8191, 1024, 4096, 2048, 100, 200]);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("u13 trace must verify");
    }

    /// A 1-bit range test ensures the smallest meaningful witness round-trips.
    #[test]
    fn prove_and_verify_u1_trace() {
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = RangeAir::<1>::new();
        let trace = RangeAir::<1>::generate_trace(&[0, 1, 1, 0]);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("u1 trace must verify");
    }
}
