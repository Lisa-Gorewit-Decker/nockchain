//! Matmul chip — per-stripe `r`-wide INT8 dot-product accumulator.
//!
//! Encodes the inner-loop arithmetic of Pearl §4.5 tile multiplication
//! for a single `(i, j)` tile position. The full `MatmulAir` will
//! compose `tile * tile` instances of this chip; here we lay out one
//! cell, exercise it end-to-end, and verify the transition semantics.
//!
//! ## Trace layout
//!
//! One row per stripe step. Height `= ⌈k / r⌉` (padded to the next
//! power of two). Width `= 2 + 2 * STRIPE`.
//!
//! ```text
//!   col 0          : c_in   — running accumulator entering this stripe
//!   col 1          : c_out  — running accumulator leaving this stripe
//!   col 2 .. 2+r   : a[s*r .. s*r + r] for this stripe
//!   col 2+r .. 2+2r: b[s*r .. s*r + r] for this stripe
//! ```
//!
//! ## Constraints
//!
//! ```text
//!   per row:     c_out = c_in + Σ_{l=0..r} a[l] · b[l]
//!   first row:   c_in = 0
//!   transition:  c_in_{next} = c_out_{current}
//! ```
//!
//! All arithmetic is in Goldilocks; the prover's Rust-side i32
//! accumulator wraps via `v as u32 as u64` for negatives. As long as
//! the human value stays inside `[-2^31, 2^31)` (Pearl §4.8 keeps it
//! well inside i32 by design), the field-level equation holds iff the
//! integer-level equation holds.

use p3_air::{Air, AirBuilder, BaseAir, WindowAccess};
use p3_matrix::dense::RowMajorMatrix;

use crate::circuit::Val;

/// AIR for a single (i, j) tile cell's stripe accumulator.
#[derive(Debug, Clone, Copy)]
pub struct MatmulCellAir<const STRIPE: usize>;

impl<const STRIPE: usize> MatmulCellAir<STRIPE> {
    pub const fn new() -> Self {
        Self
    }

    /// Number of trace columns (`c_in | c_out | a × r | b × r`).
    pub const fn width() -> usize {
        2 + 2 * STRIPE
    }

    /// Build the trace for a single `(i, j)` cell given a row of `A'`
    /// and a column of `B'` (both length `k`, must be a multiple of
    /// `STRIPE`). Per-row entries are written in the column order
    /// documented at the top of this module.
    ///
    /// # Panics
    ///
    /// Panics if `a.len() != b.len()` or if `STRIPE` doesn't divide
    /// `a.len()`.
    pub fn generate_trace(a: &[i8], b: &[i8]) -> RowMajorMatrix<Val> {
        assert_eq!(a.len(), b.len(), "a and b must be the same length");
        assert!(STRIPE > 0, "STRIPE must be > 0");
        assert_eq!(a.len() % STRIPE, 0, "STRIPE must divide a.len()");

        let num_stripes = a.len() / STRIPE;
        let n = num_stripes.next_power_of_two().max(1);
        let width = Self::width();
        let mut flat: Vec<Val> = Vec::with_capacity(n * width);

        let mut c: i64 = 0;
        for s in 0..num_stripes {
            let c_in = c;
            let lo = s * STRIPE;
            let hi = lo + STRIPE;
            let mut stripe_sum: i64 = 0;
            for l in lo..hi {
                stripe_sum += (a[l] as i64) * (b[l] as i64);
            }
            let c_out = c_in + stripe_sum;

            flat.push(i64_to_val(c_in));
            flat.push(i64_to_val(c_out));
            for l in lo..hi {
                flat.push(i64_to_val(a[l] as i64));
            }
            for l in lo..hi {
                flat.push(i64_to_val(b[l] as i64));
            }

            c = c_out;
        }

        // Padding: keep the carry consistent. Each padded row reads
        // c_in from the previous c_out and emits the same c_out with
        // an all-zero stripe (so the dot product is zero and c_in =
        // c_out trivially).
        for _ in num_stripes..n {
            flat.push(i64_to_val(c));
            flat.push(i64_to_val(c));
            for _ in 0..(2 * STRIPE) {
                flat.push(i64_to_val(0));
            }
        }

        RowMajorMatrix::new(flat, width)
    }
}

impl<const STRIPE: usize> Default for MatmulCellAir<STRIPE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F, const STRIPE: usize> BaseAir<F> for MatmulCellAir<STRIPE> {
    fn width(&self) -> usize {
        Self::width()
    }
}

impl<AB: AirBuilder, const STRIPE: usize> Air<AB> for MatmulCellAir<STRIPE> {
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let cur = main.current_slice();
        let nxt = main.next_slice();

        let c_in: AB::Var = cur[0];
        let c_out: AB::Var = cur[1];

        // Per-row dot product: c_out = c_in + Σ a[i] · b[i]
        let mut acc: AB::Expr = c_in.into();
        for l in 0..STRIPE {
            let a_l: AB::Var = cur[2 + l];
            let b_l: AB::Var = cur[2 + STRIPE + l];
            acc = acc + a_l * b_l;
        }
        builder.assert_eq(c_out, acc);

        // First row: c_in starts at 0.
        builder.when_first_row().assert_zero(c_in);

        // Transition: next.c_in = current.c_out.
        let nxt_c_in: AB::Var = nxt[0];
        builder.when_transition().assert_eq(c_out, nxt_c_in);
    }
}

#[inline]
fn i64_to_val(v: i64) -> Val {
    use p3_field::integers::QuotientMap;
    // Map signed integers into Goldilocks: nonnegative → `Self::new(v as u64)`;
    // negative → `Self::new(P + v)`. (`QuotientMap<u64>::from_int` would just
    // bit-reinterpret, mapping `-5` to `2^32 − 6` rather than `p − 5`.)
    <Val as QuotientMap<i64>>::from_int(v)
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

    #[test]
    fn width_is_two_plus_two_stripe() {
        assert_eq!(
            <MatmulCellAir<2> as BaseAir<Val>>::width(&MatmulCellAir::<2>),
            6
        );
        assert_eq!(
            <MatmulCellAir<8> as BaseAir<Val>>::width(&MatmulCellAir::<8>),
            18
        );
        assert_eq!(
            <MatmulCellAir<1> as BaseAir<Val>>::width(&MatmulCellAir::<1>),
            4
        );
    }

    #[test]
    fn trace_initial_c_in_is_zero() {
        let trace = MatmulCellAir::<2>::generate_trace(&[1, 2, 3, 4], &[5, 6, 7, 8]);
        // Row 0, col 0 = c_in initial = 0.
        assert_eq!(trace.values[0].as_canonical_u64(), 0);
    }

    #[test]
    fn trace_dot_product_matches_naive() {
        // 2 stripes of size 2 each: stripe 0 = (1,2)·(5,6) = 17;
        // stripe 1 = (3,4)·(7,8) = 53. Cumulative: c_out_0=17, c_out_1=70.
        let trace = MatmulCellAir::<2>::generate_trace(&[1, 2, 3, 4], &[5, 6, 7, 8]);
        let w = trace.width;
        // Row 0: c_out at col 1.
        assert_eq!(trace.values[1].as_canonical_u64(), 17);
        // Row 1: c_in at col 0 = 17 (carry), c_out at col 1 = 70.
        assert_eq!(trace.values[w].as_canonical_u64(), 17);
        assert_eq!(trace.values[w + 1].as_canonical_u64(), 70);
    }

    #[test]
    fn trace_padding_carries_accumulator() {
        // 3 stripes → padded to 4 rows. Padding row 3 must have
        // c_in = c_out = final accumulator.
        let trace = MatmulCellAir::<2>::generate_trace(&[1, 1, 1, 1, 1, 1], &[1, 1, 1, 1, 1, 1]);
        let w = trace.width;
        let height = trace.values.len() / w;
        assert_eq!(height, 4);
        // Final c_out from row 2 is 2 + 2 + 2 = 6.
        assert_eq!(trace.values[2 * w + 1].as_canonical_u64(), 6);
        // Row 3 (padding): c_in = c_out = 6.
        assert_eq!(trace.values[3 * w].as_canonical_u64(), 6);
        assert_eq!(trace.values[3 * w + 1].as_canonical_u64(), 6);
    }

    #[test]
    #[should_panic(expected = "STRIPE must divide")]
    fn trace_panics_on_misaligned_length() {
        let _ = MatmulCellAir::<4>::generate_trace(&[1; 5], &[1; 5]);
    }

    #[test]
    fn prove_and_verify_simple_matmul_cell() {
        // 4 stripes of size 2 = k=8.
        let a: [i8; 8] = [1, -2, 3, -4, 5, -6, 7, -8];
        let b: [i8; 8] = [-1, 2, -3, 4, -5, 6, -7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulCellAir::<2>::new();
        let trace = MatmulCellAir::<2>::generate_trace(&a, &b);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("valid matmul-cell trace must verify");
    }

    #[test]
    fn prove_and_verify_negative_accumulator() {
        // Values chosen to keep |c_out| under ~10^5; Goldilocks
        // handles the negative-as-(p-|v|) representation.
        let a: [i8; 8] = [-64; 8];
        let b: [i8; 8] = [64; 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulCellAir::<2>::new();
        let trace = MatmulCellAir::<2>::generate_trace(&a, &b);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("negative accumulator trace must verify");
    }

    #[test]
    fn verify_rejects_tampered_dot_product() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulCellAir::<2>::new();
        let mut trace = MatmulCellAir::<2>::generate_trace(&a, &b);
        // Replace c_out of row 0 with a wrong value.
        trace.values[1] = i64_to_val(999);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "tampered c_out must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_tampered_carry() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulCellAir::<2>::new();
        let mut trace = MatmulCellAir::<2>::generate_trace(&a, &b);
        // Force row 1's c_in to disagree with row 0's c_out: row 0 c_out = 17.
        // Set row 1 c_in (offset = width + 0) to 99.
        let w = trace.width;
        trace.values[w] = i64_to_val(99);
        // Also fix row 1's c_out so the row-local equation still holds —
        // then the only failing constraint is the transition carry.
        trace.values[w + 1] = i64_to_val(99 + 3 * 7 + 4 * 8);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "broken carry must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_nonzero_initial_c_in() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulCellAir::<2>::new();
        let mut trace = MatmulCellAir::<2>::generate_trace(&a, &b);
        // Replace row 0's c_in (= 0) with 7; also bump c_out by 7 so
        // the per-row equation still holds — only the first-row
        // constraint should fail.
        trace.values[0] = i64_to_val(7);
        trace.values[1] = i64_to_val(7 + 1 * 5 + 2 * 6);
        // And carry through the carry chain so transition holds.
        let w = trace.width;
        trace.values[w] = trace.values[1];
        trace.values[w + 1] = i64_to_val(trace.values[1].as_canonical_u64() as i64 + 3 * 7 + 4 * 8);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "nonzero initial c_in must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_tampered_a_value() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulCellAir::<2>::new();
        let mut trace = MatmulCellAir::<2>::generate_trace(&a, &b);
        // Replace a[0] of row 0 (col 2) with a wrong value, leaving c_out unchanged.
        trace.values[2] = i64_to_val(99);
        let proof = prove(&cfg, &air, trace, &[]);
        let r = verify(&cfg, &air, &proof, &[]);
        assert!(r.is_err(), "tampered a-value must reject; got {r:?}");
    }

    #[test]
    fn larger_stripe_round_trip() {
        let a: Vec<i8> = (0..32).map(|i| ((i * 3) % 64) as i8 - 32).collect();
        let b: Vec<i8> = (0..32).map(|i| ((i * 5) % 64) as i8 - 32).collect();
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulCellAir::<8>::new();
        let trace = MatmulCellAir::<8>::generate_trace(&a, &b);
        let proof = prove(&cfg, &air, trace, &[]);
        verify(&cfg, &air, &proof, &[]).expect("STRIPE=8 trace must verify");
    }
}
