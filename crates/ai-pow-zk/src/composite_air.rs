//! Composite tile AIR (M9.1) — matmul cell × state rotate-XOR.
//!
//! Folds [`matmul_chip::MatmulCellAir<STRIPE>`] (M6) and
//! [`state_chip::StateChipAir`] (M7) into a single AIR with the
//! cross-chip linkages spelled out as constraints. This is the
//! protocol-realistic primitive that proves, for one `(i, j)` tile
//! cell:
//!
//!   1. The per-stripe `r`-wide INT8 dot product is computed correctly.
//!   2. The rolling tile-state value evolves by Pearl §4.5's rotate-XOR
//!      update where the per-stripe XOR-fold `x` is bound to the matmul
//!      accumulator `c_out` via two's-complement sign extension.
//!   3. The state chain carries across rows: `next.m_in = cur.m_out`,
//!      with the chain seeded at zero on the first row.
//!
//! ## Trace layout
//!
//! One row per stripe step. Height = `num_stripes`. **`num_stripes`
//! must be a power of two** — the state chain's rotate-XOR constraint
//! does not have a natural all-zero padding row, so the MVP composite
//! requires zero padding. (See `M9.2` follow-on in `ROADMAP.md` for
//! selector-gated padding.)
//!
//! Width = `(2 + 2·STRIPE) + state_chip::WIDTH` = `M6_WIDTH + 67`.
//!
//! ```text
//!   col 0..2          : c_in, c_out                       (M6)
//!   col 2..2+r        : a[s*r .. (s+1)*r]                 (M6)
//!   col 2+r..2+2r     : b[s*r .. (s+1)*r]                 (M6)
//!   col 2+2r          : m_in                              (M7)
//!   col 2+2r+1        : x                                 (M7)
//!   col 2+2r+2        : m_out                             (M7)
//!   col 2+2r+3..      : m_in_bits[32], x_bits[32]         (M7)
//! ```
//!
//! ## Constraints
//!
//! Per row:
//! ```text
//!   c_out = c_in + Σ_{l=0..r} a[l] · b[l]                 (M6 dot product)
//!   m_in, x, m_out, all bits boolean                       (M7 booleans)
//!   m_in  = Σ_{i=0..32} 2^i · m_in_bits[i]                 (M7 recomp)
//!   x     = Σ_{i=0..32} 2^i · x_bits[i]                    (M7 recomp)
//!   m_out = Σ_{i=0..32} 2^i · (m_in_bit_{(i-13) mod 32}
//!                              ⊕ x_bit_i)                  (M7 rotate-XOR)
//!   c_out = x - 2^32 · x_bits[31]                          (LINKAGE: sign-ext)
//! ```
//!
//! Boundary rows:
//! ```text
//!   first row:  c_in = 0,  m_in = 0
//!   transition: next.c_in = cur.c_out
//!               next.m_in = cur.m_out
//! ```
//!
//! The linkage `c_out = x - 2^32 · x_bits[31]` is the two's-complement
//! sign-extension rule: if `c_out` is in `[0, 2^31)`, `x_bits[31] = 0`
//! and `c_out = x` directly; if `c_out_signed ∈ [-2^31, 0)`, then
//! `c_out_field = p + c_out_signed`, `x_u32 = 2^32 + c_out_signed`,
//! and `c_out_field - x_u32 = p - 2^32 ≡ -2^32 (mod p)`, so
//! `c_out = x - 2^32` when `x_bits[31] = 1`. One field equation covers
//! both cases.
//!
//! This pins the matmul accumulator to fit in the i32 range
//! `[-2^31, 2^31)`. Pearl §4.8 keeps the accumulator well inside that.

use p3_air::{Air, AirBuilder, BaseAir, WindowAccess};
use p3_field::PrimeCharacteristicRing;
use p3_matrix::dense::RowMajorMatrix;

use crate::circuit::Val;
use crate::state_chip;

/// Composite AIR proving the M6 + M7 stack for one tile cell.
#[derive(Debug, Clone, Copy, Default)]
pub struct MatmulTileAir<const STRIPE: usize>;

impl<const STRIPE: usize> MatmulTileAir<STRIPE> {
    pub const fn new() -> Self {
        Self
    }

    /// Number of M6 columns (`c_in | c_out | a | b`).
    pub const fn m6_width() -> usize {
        2 + 2 * STRIPE
    }

    /// First column of the M7 (state) block in the composite row.
    pub const fn state_start() -> usize {
        Self::m6_width()
    }

    /// Total trace width.
    pub const fn width() -> usize {
        Self::m6_width() + state_chip::WIDTH
    }

    /// Build a composite trace from a row of `A'` and a column of `B'`.
    ///
    /// # Panics
    ///
    /// - `a.len() != b.len()`
    /// - `STRIPE` does not divide `a.len()`
    /// - `num_stripes = a.len() / STRIPE` is not a power of two
    /// - any intermediate accumulator exceeds the i32 range
    pub fn generate_trace(a: &[i8], b: &[i8]) -> RowMajorMatrix<Val> {
        assert_eq!(a.len(), b.len(), "a and b must be the same length");
        assert!(STRIPE > 0, "STRIPE must be > 0");
        assert_eq!(a.len() % STRIPE, 0, "STRIPE must divide a.len()");

        let num_stripes = a.len() / STRIPE;
        assert!(
            num_stripes.is_power_of_two() && num_stripes > 0,
            "composite tile AIR requires num_stripes to be a power of two; got {num_stripes}"
        );

        let width = Self::width();
        let mut flat: Vec<Val> = Vec::with_capacity(num_stripes * width);

        let mut c_signed: i64 = 0;
        let mut m_u32: u32 = 0;
        for s in 0..num_stripes {
            let lo = s * STRIPE;
            let hi = lo + STRIPE;

            // M6 update.
            let c_in_signed = c_signed;
            let mut sum: i64 = 0;
            for l in lo..hi {
                sum += (a[l] as i64) * (b[l] as i64);
            }
            let c_out_signed = c_in_signed + sum;
            assert!(
                c_out_signed >= i32::MIN as i64 && c_out_signed <= i32::MAX as i64,
                "accumulator out of i32 range at stripe {s}: {c_out_signed}",
            );
            c_signed = c_out_signed;

            // M7 update under linkage `x = c_out as i32 as u32`.
            let x_u32 = (c_out_signed as i32) as u32;
            let m_in_u32 = m_u32;
            let m_out_u32 = m_in_u32.rotate_left(state_chip::ROT as u32) ^ x_u32;
            m_u32 = m_out_u32;

            // -------- M6 cells --------
            flat.push(i64_to_val(c_in_signed));
            flat.push(i64_to_val(c_out_signed));
            for l in lo..hi {
                flat.push(i64_to_val(a[l] as i64));
            }
            for l in lo..hi {
                flat.push(i64_to_val(b[l] as i64));
            }
            // -------- M7 cells --------
            flat.push(u32_to_val(m_in_u32));
            flat.push(u32_to_val(x_u32));
            flat.push(u32_to_val(m_out_u32));
            for i in 0..state_chip::W {
                flat.push(bit_to_val((m_in_u32 >> i) & 1));
            }
            for i in 0..state_chip::W {
                flat.push(bit_to_val((x_u32 >> i) & 1));
            }
        }

        RowMajorMatrix::new(flat, width)
    }

    /// Reference scalar computation: final tile-state `M[0]` for the
    /// single-slot chain `m ← rotate_left_13(m) ⊕ x_s`. Used by tests
    /// to predict what the trace should produce.
    pub fn reference_final_state(a: &[i8], b: &[i8]) -> u32 {
        assert_eq!(a.len(), b.len());
        let num_stripes = a.len() / STRIPE;
        let mut c: i64 = 0;
        let mut m: u32 = 0;
        for s in 0..num_stripes {
            let lo = s * STRIPE;
            for l in lo..lo + STRIPE {
                c += (a[l] as i64) * (b[l] as i64);
            }
            let x = (c as i32) as u32;
            m = m.rotate_left(state_chip::ROT as u32) ^ x;
        }
        m
    }
}

/// Public-values index where the last-row `m_out` is exposed.
///
/// The composite AIR's public-values vector is laid out as
/// `[public_inputs(42 elements) | m_final_slot_0]`. The trace's
/// last row's `m_out` column is constrained to equal this slot — see
/// the M10.1a constraint in [`Air::eval`] below. This is what binds
/// the SNARK's computed final tile state to a verifier-checkable
/// value; the verifier then computes
/// `BLAKE3_keyed(m_final_bytes, pow_key)` out-of-band and matches
/// against `public_inputs.found_leaf`.
pub const PI_M_FINAL_IDX: usize = crate::public::NUM_PUBLIC_INPUTS;

/// Total length of the public-values vector this AIR consumes.
///
/// `42` for the Pearl public inputs (M10) + `1` for the single-slot
/// `m_final` exposure (M10.1a). Once M9.2 routes the 16-slot state
/// the second block widens from 1 → 16.
pub const NUM_AIR_PUBLIC_VALUES: usize = crate::public::NUM_PUBLIC_INPUTS + 1;

impl<F, const STRIPE: usize> BaseAir<F> for MatmulTileAir<STRIPE> {
    fn width(&self) -> usize {
        Self::width()
    }

    /// Number of Goldilocks elements in the public-values channel.
    ///
    /// `NUM_PUBLIC_INPUTS (= 42)` for the Pearl public inputs the
    /// caller wants the SNARK to commit to (M10) plus `1` for the
    /// `m_final` exposure that the M10.1a verifier-side hash check
    /// reads back (see [`PI_M_FINAL_IDX`]).
    fn num_public_values(&self) -> usize {
        NUM_AIR_PUBLIC_VALUES
    }

    /// Max degree of any constraint, used to pin FRI's
    /// `quotient_degree_bound`. Constraints:
    ///   - per-row matmul `c_out = c_in + Σ a·b` → degree 2
    ///   - first-row / transition guards (degree 1) on degree-1
    ///     expressions → degree 2
    ///   - bit booleans `b·(1-b) = 0` → degree 2
    ///   - rotate-XOR `m_out = Σ … (1 − 2ab)` → degree 2
    /// Max overall = 2.
    fn max_constraint_degree(&self) -> Option<usize> {
        Some(2)
    }
}

impl<AB: AirBuilder, const STRIPE: usize> Air<AB> for MatmulTileAir<STRIPE> {
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let cur = main.current_slice();
        let nxt = main.next_slice();

        let state_start = Self::state_start();

        // =========================================================
        //  M6: per-row matmul accumulator
        // =========================================================
        let c_in: AB::Var = cur[0];
        let c_out: AB::Var = cur[1];
        let mut dot: AB::Expr = c_in.into();
        for l in 0..STRIPE {
            let a_l: AB::Var = cur[2 + l];
            let b_l: AB::Var = cur[2 + STRIPE + l];
            dot = dot + a_l * b_l;
        }
        builder.assert_eq(c_out, dot);
        builder.when_first_row().assert_zero(c_in);
        let nxt_c_in: AB::Var = nxt[0];
        builder.when_transition().assert_eq(c_out, nxt_c_in);

        // =========================================================
        //  M7: rotate-XOR state update
        // =========================================================
        let m_in: AB::Var = cur[state_start];
        let x: AB::Var = cur[state_start + 1];
        let m_out: AB::Var = cur[state_start + 2];

        // Snapshot bit columns.
        let mut m_in_bits: [AB::Var; state_chip::W] = [m_in; state_chip::W];
        let mut x_bits: [AB::Var; state_chip::W] = [m_in; state_chip::W];
        for i in 0..state_chip::W {
            m_in_bits[i] = cur[state_start + 3 + i];
            x_bits[i] = cur[state_start + 3 + state_chip::W + i];
        }
        builder.assert_bools(m_in_bits);
        builder.assert_bools(x_bits);

        // Precompute the field constants `2^i` for `i = 0..=W`.
        // We need pows[32] for the sign-extension linkage.
        let two = <AB::F as PrimeCharacteristicRing>::TWO;
        let mut pows: Vec<AB::F> = Vec::with_capacity(state_chip::W + 1);
        let mut cur_pow = <AB::F as PrimeCharacteristicRing>::ONE;
        for _ in 0..=state_chip::W {
            pows.push(cur_pow.clone());
            cur_pow = cur_pow * two.clone();
        }

        // m_in recomposition.
        {
            let mut sum: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
            for i in 0..state_chip::W {
                sum = sum + m_in_bits[i] * pows[i].clone();
            }
            builder.assert_eq(m_in, sum);
        }
        // x recomposition.
        {
            let mut sum: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
            for i in 0..state_chip::W {
                sum = sum + x_bits[i] * pows[i].clone();
            }
            builder.assert_eq(x, sum);
        }
        // m_out rotate-XOR equation.
        {
            let mut sum: AB::Expr = <AB::Expr as PrimeCharacteristicRing>::ZERO;
            for i in 0..state_chip::W {
                let src = (i + state_chip::W - state_chip::ROT) % state_chip::W;
                let a: AB::Var = m_in_bits[src];
                let b: AB::Var = x_bits[i];
                let xor_expr: AB::Expr =
                    AB::Expr::from(a) + AB::Expr::from(b) - AB::Expr::from(a) * b * two.clone();
                sum = sum + xor_expr * pows[i].clone();
            }
            builder.assert_eq(m_out, sum);
        }

        // =========================================================
        //  LINKAGE: c_out = x - 2^32 · x_bits[31]
        // =========================================================
        // Sign-extension from u32 `x` to signed field `c_out`. See the
        // module docstring for the two-case proof.
        {
            let two_pow_32 = pows[state_chip::W].clone();
            let rhs: AB::Expr =
                AB::Expr::from(x) - AB::Expr::from(x_bits[state_chip::W - 1]) * two_pow_32;
            builder.assert_eq(c_out, rhs);
        }

        // =========================================================
        //  State chain across rows
        // =========================================================
        builder.when_first_row().assert_zero(m_in);
        let nxt_m_in: AB::Var = nxt[state_start];
        builder.when_transition().assert_eq(m_out, nxt_m_in);

        // =========================================================
        //  M10.1a: bind last row's m_out to public M_final slot
        // =========================================================
        // The trace's terminal tile-state value (single-slot regime)
        // is forced to equal the public-values entry at
        // `PI_M_FINAL_IDX`. Combined with the verifier-side hash check
        // `BLAKE3_keyed(m_final, pow_key) == public_inputs.found_leaf`
        // this gives Pearl-style found_leaf binding without in-circuit
        // BLAKE3. See `crate::lib::verify` for the hash side.
        let m_final_pi = builder.public_values()[PI_M_FINAL_IDX];
        builder.when_last_row().assert_eq(m_out, m_final_pi);
    }
}

#[inline]
fn i64_to_val(v: i64) -> Val {
    use p3_field::integers::QuotientMap;
    <Val as QuotientMap<i64>>::from_int(v)
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

    /// `NUM_AIR_PUBLIC_VALUES` placeholder public values, with
    /// `pis[PI_M_FINAL_IDX]` set to the test trace's expected final
    /// state so the M10.1a last-row constraint
    /// `trace.last_row.m_out == pis[PI_M_FINAL_IDX]` is satisfied.
    /// The other 42 PI slots can be anything (they're Fiat-Shamir-
    /// only at this AIR level; the lib.rs integration tests check the
    /// hash-binding semantics).
    fn test_public_values(a: &[i8], b: &[i8]) -> Vec<Val> {
        let mut pis = vec![Val::default(); NUM_AIR_PUBLIC_VALUES];
        let m_final = MatmulTileAir::<2>::reference_final_state(a, b);
        use p3_field::integers::QuotientMap;
        pis[PI_M_FINAL_IDX] = <Val as QuotientMap<u32>>::from_int(m_final);
        pis
    }

    /// Same shape as `test_public_values` but parameterised by STRIPE.
    fn test_public_values_stripe8(a: &[i8], b: &[i8]) -> Vec<Val> {
        let mut pis = vec![Val::default(); NUM_AIR_PUBLIC_VALUES];
        let m_final = MatmulTileAir::<8>::reference_final_state(a, b);
        use p3_field::integers::QuotientMap;
        pis[PI_M_FINAL_IDX] = <Val as QuotientMap<u32>>::from_int(m_final);
        pis
    }

    #[test]
    fn width_is_m6_plus_state() {
        assert_eq!(
            <MatmulTileAir<2> as BaseAir<Val>>::width(&MatmulTileAir::<2>),
            6 + 67
        );
        assert_eq!(
            <MatmulTileAir<4> as BaseAir<Val>>::width(&MatmulTileAir::<4>),
            10 + 67
        );
        assert_eq!(
            <MatmulTileAir<8> as BaseAir<Val>>::width(&MatmulTileAir::<8>),
            18 + 67
        );
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn trace_panics_on_non_pow2_stripes() {
        // 3 stripes of size 2 → 6 elements, not a power of two.
        let _ = MatmulTileAir::<2>::generate_trace(&[1; 6], &[1; 6]);
    }

    #[test]
    #[should_panic(expected = "STRIPE must divide")]
    fn trace_panics_on_misaligned_length() {
        let _ = MatmulTileAir::<4>::generate_trace(&[1; 5], &[1; 5]);
    }

    #[test]
    fn trace_initial_seeds_are_zero() {
        let trace = MatmulTileAir::<2>::generate_trace(&[1, 2, 3, 4], &[5, 6, 7, 8]);
        // c_in row 0 = 0
        assert_eq!(trace.values[0].as_canonical_u64(), 0);
        // m_in row 0 = 0
        assert_eq!(
            trace.values[MatmulTileAir::<2>::state_start()].as_canonical_u64(),
            0
        );
    }

    #[test]
    fn trace_final_state_matches_reference() {
        // 2 stripes (=power of 2), positive values.
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        let want = MatmulTileAir::<2>::reference_final_state(&a, &b);
        // Final m_out is at the last row, m_out column.
        let w = trace.width;
        let h = trace.values.len() / w;
        let m_out_col = MatmulTileAir::<2>::state_start() + 2;
        let got = trace.values[(h - 1) * w + m_out_col].as_canonical_u64();
        assert_eq!(got as u32, want);
    }

    #[test]
    fn prove_and_verify_simple_positive() {
        let a: [i8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
        let b: [i8; 8] = [9, 10, 11, 12, 13, 14, 15, 16];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        let pis = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        verify(&cfg, &air, &proof, &pis).expect("composite trace must verify");
    }

    #[test]
    fn prove_and_verify_mixed_sign() {
        // Mixed signs exercise the sign-extension linkage.
        let a: [i8; 8] = [1, -2, 3, -4, 5, -6, 7, -8];
        let b: [i8; 8] = [-1, 2, -3, 4, -5, 6, -7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        let pis = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        verify(&cfg, &air, &proof, &pis).expect("mixed-sign composite trace must verify");
    }

    #[test]
    fn prove_and_verify_negative_accumulator() {
        // Drive c_out strictly negative across multiple stripes.
        let a: [i8; 8] = [-64, -64, -64, -64, -64, -64, -64, -64];
        let b: [i8; 8] = [64, 64, 64, 64, 64, 64, 64, 64];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        let pis = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        verify(&cfg, &air, &proof, &pis).expect("negative-accumulator composite trace must verify");
    }

    #[test]
    fn verify_rejects_tampered_dot_product() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let mut trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        // Corrupt c_out of row 0.
        trace.values[1] = i64_to_val(999);
        let pis = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        let r = verify(&cfg, &air, &proof, &pis);
        assert!(r.is_err(), "tampered c_out must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_tampered_state_chain() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let mut trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        let w = trace.width;
        // Row 1's m_in (state_start col, row 1) — break the chain
        // transition without disturbing M6 (we leave c_in/c_out alone).
        let m_in_col = MatmulTileAir::<2>::state_start();
        trace.values[w + m_in_col] = u32_to_val(0xDEAD_BEEF);
        let pis = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        let r = verify(&cfg, &air, &proof, &pis);
        assert!(r.is_err(), "broken state chain must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_tampered_linkage() {
        // Mutate the x column to break `x = c_out` while leaving the
        // matmul side coherent. The rotate-XOR equation will also fail
        // because m_out was built from a different x, but the linkage
        // failure alone suffices.
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let mut trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        let x_col = MatmulTileAir::<2>::state_start() + 1;
        trace.values[x_col] = u32_to_val(0x1234_5678);
        let pis = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        let r = verify(&cfg, &air, &proof, &pis);
        assert!(r.is_err(), "tampered x linkage must reject; got {r:?}");
    }

    #[test]
    fn verify_rejects_nonzero_initial_m_in() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let mut trace = MatmulTileAir::<2>::generate_trace(&a, &b);
        // Replace m_in row 0 with a non-zero value, plus its bit
        // decomposition (so M7 recomposition stays consistent) — only
        // the "first row m_in = 0" boundary should fail.
        let m_in_col = MatmulTileAir::<2>::state_start();
        let m_in_bits_col = MatmulTileAir::<2>::state_start() + 3;
        trace.values[m_in_col] = u32_to_val(1);
        trace.values[m_in_bits_col] = u32_to_val(1); // bit 0
                                                     // The transition `next.m_in = cur.m_out` will also fail because
                                                     // row 1's m_in was computed assuming m_in_0 = 0 — that's still
                                                     // a valid first-row violation; we just need any rejection.
        let pis = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        let r = verify(&cfg, &air, &proof, &pis);
        assert!(r.is_err(), "nonzero initial m_in must reject; got {r:?}");
    }

    #[test]
    fn stripe_8_round_trip() {
        // 4 stripes of width 8 → k = 32 over 4 rows (power of 2).
        let a: Vec<i8> = (0..32).map(|i| ((i * 3) % 64) as i8 - 32).collect();
        let b: Vec<i8> = (0..32).map(|i| ((i * 5) % 64) as i8 - 32).collect();
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<8>::new();
        let trace = MatmulTileAir::<8>::generate_trace(&a, &b);
        let pis = test_public_values_stripe8(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis);
        verify(&cfg, &air, &proof, &pis).expect("STRIPE=8 composite trace must verify");
    }

    /// M10.1a binding: tampering with the `m_final` slot of the public
    /// values without tampering with the trace must reject — the AIR's
    /// `when_last_row().assert_eq(m_out, pis[PI_M_FINAL_IDX])`
    /// constraint catches it.
    #[test]
    fn verify_rejects_tampered_m_final_public_value() {
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        let cfg = build_stark_config(&test_zk_params(), &CircuitConfig::TEST);
        let air = MatmulTileAir::<2>::new();
        let trace = MatmulTileAir::<2>::generate_trace(&a, &b);

        let pis_honest = test_public_values(&a, &b);
        let proof = prove(&cfg, &air, trace, &pis_honest);

        let mut pis_tampered = pis_honest.clone();
        use p3_field::integers::QuotientMap;
        pis_tampered[PI_M_FINAL_IDX] = <Val as QuotientMap<u32>>::from_int(0xDEAD_BEEF);

        let r = verify(&cfg, &air, &proof, &pis_tampered);
        assert!(
            r.is_err(),
            "tampered m_final public value must be rejected by the AIR; got {r:?}"
        );
    }

    #[test]
    fn reference_final_state_matches_pearl_semantics() {
        // The reference must match a hand-traced rotate-XOR over the
        // sequence of c_out values, which is Pearl §4.5's single-slot
        // semantics.
        let a: [i8; 4] = [1, 2, 3, 4];
        let b: [i8; 4] = [5, 6, 7, 8];
        // stripe 0: c = 1·5 + 2·6 = 17; x = 17. m = 0.rot13 ^ 17 = 17.
        // stripe 1: c = 17 + 3·7 + 4·8 = 70; x = 70. m = 17.rot13 ^ 70.
        let want = (17u32.rotate_left(13)) ^ 70u32;
        assert_eq!(MatmulTileAir::<2>::reference_final_state(&a, &b), want);
    }
}
