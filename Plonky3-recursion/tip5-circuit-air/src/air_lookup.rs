//! Lookup-table Tip5 permutation AIR (the narrow, paper-faithful
//! form — Tip5 paper IACR ePrint 2023/107 §4.7: a Hash table that
//! *looks up* the split-and-lookup S-box `L` in a 256-row Lookup
//! table via a LogUp argument).
//!
//! This **replaces the ≈7168-column boolean range-check core** of
//! the lookup-free [`crate::Tip5PermAir`] with **2 columns per byte**
//! (`b`,`c`) + one verifier-fixed 256-row preprocessed L-table + a
//! per-table-row multiplicity + a local LogUp interaction. The
//! algebraic constraints (canonical recomposition + §4.6 `<p` guard
//! + x⁷ + circulant MDS + round constants) are the *same, validated*
//! logic as `Tip5PermAir`; only the split-S-box encoding changes
//! (bits+cube ⇒ byte+image+LogUp). It is standalone-validatable with
//! **no recursion machinery** (mirroring Plonky3 `p3-lookup`'s own
//! `RangeCheckAir`): the LogUp soundness is the running-sum
//! accumulator returning to zero.
//!
//! Layout — a single main trace of `H = next_pow2(256 + P)` rows:
//! * rows `[0,256)`  = **table rows** (`KIND=0`); preprocessed
//!   `(TIN,TOUT)=(i, LOOKUP_TABLE[i])`, main `TMULT = #queries of i`.
//! * rows `[256,256+P)` = **permutation rows** (`KIND=1`): one full
//!   7-round Tip5 evaluation each.
//! * rows `[256+P,H)` = inert padding (`KIND=0, TMULT=0`).
//!
//! Soundness: every per-byte `(b,c)` on a perm row is a LogUp query
//! into the preprocessed table; the accumulator is zero iff every
//! `(b,c)` equals a genuine `(i, LOOKUP_TABLE[i])` row ⇒ `b∈[0,256)`
//! **and** `c = LOOKUP_TABLE[b]` (the C2.0 identity anchors that the
//! table is the paper's `L`). A tampered `c`, an out-of-table `b`,
//! or a non-canonical §4.6 split ⇒ accumulator ≠ 0 / constraints
//! fail (adversarially tested).

use p3_air::{Air, AirBuilder, BaseAir, WindowAccess};
use p3_field::PrimeCharacteristicRing;
use p3_lookup::builder::InteractionBuilder;

use crate::tip5_spec::{
    LOOKUP_TABLE, NUM_ROUNDS, NUM_SPLIT_AND_LOOKUP, ROUND_CONSTANTS, STATE_SIZE, mds_matrix,
    rc_precomp,
};

pub(crate) const NS: usize = NUM_SPLIT_AND_LOOKUP; // 4 split lanes
pub(crate) const NBYTES: usize = 8; // bytes per split lane
/// Number of L-table rows (every byte value 0..256).
pub const TABLE_ROWS: usize = 256;

// ---- main-trace flat layout ----
// [ KIND | TMULT | IN[16] | round_0 .. round_6 ]   per round group:
//   split: NS*(2*NBYTES) (b then c)  +  INV[NS]
//   power: X2[12] X3[12]  +  A[16]  +  ROUT[16]
const SPLIT_BC: usize = NS * 2 * NBYTES; // 64
const PWR: usize = STATE_SIZE - NS; // 12
pub(crate) const ROUND_GROUP: usize = SPLIT_BC + NS + PWR + PWR + STATE_SIZE + STATE_SIZE; // 124

const C_KIND: usize = 0;
const C_TMULT: usize = 1;
const C_IN: usize = 2; // IN[0..16]
const RB0: usize = C_IN + STATE_SIZE; // first round group base = 18

#[inline]
const fn rb(r: usize) -> usize {
    RB0 + r * ROUND_GROUP
}
#[inline]
const fn b_col(r: usize, t: usize, k: usize) -> usize {
    rb(r) + t * (2 * NBYTES) + k
}
#[inline]
const fn c_col(r: usize, t: usize, k: usize) -> usize {
    rb(r) + t * (2 * NBYTES) + NBYTES + k
}
#[inline]
const fn inv_col(r: usize, t: usize) -> usize {
    rb(r) + SPLIT_BC + t
}
#[inline]
const fn x2_col(r: usize, j: usize) -> usize {
    rb(r) + SPLIT_BC + NS + (j - NS)
}
#[inline]
const fn x3_col(r: usize, j: usize) -> usize {
    rb(r) + SPLIT_BC + NS + PWR + (j - NS)
}
#[inline]
pub(crate) const fn a_col(r: usize, i: usize) -> usize {
    rb(r) + SPLIT_BC + NS + 2 * PWR + i
}
#[inline]
pub(crate) const fn rout_col(r: usize, i: usize) -> usize {
    rb(r) + SPLIT_BC + NS + 2 * PWR + STATE_SIZE + i
}
#[inline]
const fn sbox_in_col(r: usize, lane: usize) -> usize {
    if r == 0 {
        C_IN + lane
    } else {
        rout_col(r - 1, lane)
    }
}

/// Total main-trace width (≈8× narrower than the lookup-free AIR).
pub const fn tip5_lookup_air_width() -> usize {
    RB0 + NUM_ROUNDS * ROUND_GROUP
}

// ---- preprocessed (verifier-fixed) L-table: [IS_TABLE | TIN | TOUT] ----
const P_IS_TABLE: usize = 0;
const P_TIN: usize = 1;
const P_TOUT: usize = 2;
pub const PREP_WIDTH: usize = 3;

/// The lookup-table Tip5 permutation AIR. Carries the verifier-fixed
/// preprocessed L-table (flat row-major, `PREP_WIDTH` cols, height =
/// the main trace height) — the poseidon-circuit-air pattern, so the
/// `(i, LOOKUP_TABLE[i])` rows are *not* prover-controlled.
#[derive(Debug, Default, Clone)]
pub struct Tip5PermLookupAir<F> {
    preprocessed: alloc::vec::Vec<F>,
}

impl<F> Tip5PermLookupAir<F> {
    pub const fn new(preprocessed: alloc::vec::Vec<F>) -> Self {
        Self { preprocessed }
    }
}

impl<F: p3_field::Field> BaseAir<F> for Tip5PermLookupAir<F> {
    fn width(&self) -> usize {
        tip5_lookup_air_width()
    }
    fn preprocessed_width(&self) -> usize {
        PREP_WIDTH
    }
    fn preprocessed_trace(&self) -> Option<p3_matrix::dense::RowMajorMatrix<F>> {
        Some(p3_matrix::dense::RowMajorMatrix::new(
            self.preprocessed.clone(),
            PREP_WIDTH,
        ))
    }
    fn max_constraint_degree(&self) -> Option<usize> {
        // kind-gated guard / x⁷ closer reach degree 4 (same FRI tier
        // as the degree-3 lookup-free AIR: log_blowup=2, B=4).
        Some(4)
    }
}

impl<AB: AirBuilder + InteractionBuilder> Air<AB> for Tip5PermLookupAir<AB::F>
where
    AB::F: p3_field::Field,
{
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let prep = builder.preprocessed().clone();
        let local = main.current_slice();
        let pre = prep.current_slice();

        let fe = |v: u64| -> AB::Expr { AB::Expr::from(AB::F::from_u64(v)) };
        let var = |c: usize| -> AB::Expr { local[c].into() };
        let pvar = |c: usize| -> AB::Expr { pre[c].into() };
        let pow8 = |k: usize| -> AB::Expr { fe(1u64 << (8 * k)) };

        let kind = var(C_KIND);
        // KIND is boolean.
        builder.assert_zero(kind.clone() * (kind.clone() - AB::Expr::ONE));

        let mds = mds_matrix();
        let two32_m1 = fe((1u64 << 32) - 1);

        // ---- LogUp local interaction (paper §4.7): perm rows QUERY
        //      each (b,c); the preprocessed table PROVIDES (i,L[i]). ----
        let mut tuples: alloc::vec::Vec<(alloc::vec::Vec<AB::Expr>, AB::Expr)> =
            alloc::vec::Vec::with_capacity(NS * NBYTES * NUM_ROUNDS + 1);
        for r in 0..NUM_ROUNDS {
            for t in 0..NS {
                for k in 0..NBYTES {
                    tuples.push((
                        alloc::vec![var(b_col(r, t, k)), var(c_col(r, t, k))],
                        kind.clone(), // query multiplicity = 1 on perm rows, 0 else
                    ));
                }
            }
        }
        // table side: provide (TIN,TOUT) with -TMULT (0 on perm/pad).
        tuples.push((
            alloc::vec![pvar(P_TIN), pvar(P_TOUT)],
            -(var(C_TMULT) * pvar(P_IS_TABLE)),
        ));
        builder.push_local_interaction(tuples);

        // ---- algebraic constraints, gated by KIND (perm rows only;
        //      table/pad rows have KIND=0 ⇒ trivially satisfied) ----
        for r in 0..NUM_ROUNDS {
            for t in 0..NS {
                let mut recompose_b = AB::Expr::ZERO;
                let mut recompose_c = AB::Expr::ZERO;
                let mut low = AB::Expr::ZERO;
                let mut high = AB::Expr::ZERO;
                for k in 0..NBYTES {
                    let bk = var(b_col(r, t, k));
                    let ck = var(c_col(r, t, k));
                    recompose_b = recompose_b + bk.clone() * pow8(k);
                    recompose_c = recompose_c + ck * pow8(k);
                    if k < 4 {
                        low = low + bk * pow8(k);
                    } else {
                        high = high + bk * pow8(k - 4);
                    }
                }
                // canonical 8-byte decomposition of the S-box input
                builder.assert_zero(
                    kind.clone() * (recompose_b - var(sbox_in_col(r, t))),
                );
                // A[t] = recomposed looked-up bytes
                builder.assert_zero(kind.clone() * (var(a_col(r, t)) - recompose_c));
                // §4.6 canonical (<p) guard: H = 2^32−1 ⇒ L = 0.
                let g = high - two32_m1.clone();
                let inv = var(inv_col(r, t));
                let prod = g.clone() * inv;
                builder.assert_zero(kind.clone() * g * (prod.clone() - AB::Expr::ONE));
                builder.assert_zero(kind.clone() * (AB::Expr::ONE - prod) * low);
            }

            for j in NS..STATE_SIZE {
                let x = var(sbox_in_col(r, j));
                let x2 = var(x2_col(r, j));
                let x3 = var(x3_col(r, j));
                builder.assert_zero(kind.clone() * (x2.clone() - x.clone() * x.clone()));
                builder.assert_zero(kind.clone() * (x3.clone() - x2 * x.clone()));
                builder.assert_zero(
                    kind.clone() * (var(a_col(r, j)) - x3.clone() * x3 * x),
                );
            }

            for i in 0..STATE_SIZE {
                let mut acc = AB::Expr::ZERO;
                for j in 0..STATE_SIZE {
                    acc = acc + fe(mds[i][j]) * var(a_col(r, j));
                }
                let rc = fe(rc_precomp(ROUND_CONSTANTS[r * STATE_SIZE + i]));
                builder.assert_zero(kind.clone() * (var(rout_col(r, i)) - acc - rc));
            }
        }
        let _ = LOOKUP_TABLE; // table content lives in the preprocessed trace
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::vec::Vec;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::{fs, vec};

    use p3_air::check_constraints;
    use p3_field::{Field, PrimeCharacteristicRing, PrimeField64};
    use p3_goldilocks::Goldilocks;
    use p3_matrix::Matrix;
    use p3_test_utils::goldilocks_params::Challenge;

    use super::*;
    use crate::generation_lookup::generate_lookup_trace;
    use crate::tip5_spec::permute;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/ai-pow-zk/tests/fixtures/tip5_golden_kat.txt"
    );

    fn fixture_vectors() -> Vec<([u64; STATE_SIZE], [u64; STATE_SIZE])> {
        let text = fs::read_to_string(FIXTURE).expect("C2.0 fixture present");
        let nums = |l: &str| -> Vec<u64> {
            l.split_whitespace().skip(1).map(|t| t.parse().unwrap()).collect()
        };
        let mut out = Vec::new();
        let mut pend: Option<[u64; STATE_SIZE]> = None;
        for line in text.lines() {
            if line.starts_with("IN ") {
                let v = nums(line);
                let mut a = [0u64; STATE_SIZE];
                a.copy_from_slice(&v);
                pend = Some(a);
            } else if line.starts_with("OUT ") {
                let v = nums(line);
                let mut a = [0u64; STATE_SIZE];
                a.copy_from_slice(&v);
                out.push((pend.take().unwrap(), a));
            }
        }
        out
    }

    fn xs(s: &mut u64) -> u64 {
        let mut x = *s;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *s = x;
        x.wrapping_mul(0x2545F4914F6CDD1D) % crate::tip5_spec::P_GOLDILOCKS
    }

    /// β = 257 makes `combine(b,c) = 257·b + c` injective on bytes
    /// (`b,c < 256`); α huge avoids poles. For a valid lookup the
    /// query multiset equals the table multiset ⇒ the running sum is
    /// **exactly** 0; any byte tamper breaks injective-matching ⇒ ≠ 0.
    fn logup_accumulator(
        main: &p3_matrix::dense::RowMajorMatrix<Goldilocks>,
        prep: &[Goldilocks],
    ) -> Challenge {
        let w = main.width();
        let h = main.height();
        let beta = Challenge::from_u64(257);
        let alpha = Challenge::from_u64(0x1_0000_0001);
        let comb = |x: u64, y: u64| beta * Challenge::from_u64(x) + Challenge::from_u64(y);
        let g2u = |g: Goldilocks| g.as_canonical_u64();
        let mut acc = Challenge::ZERO;
        for row in 0..h {
            let b = row * w;
            let kind = g2u(main.values[b + C_KIND]);
            if kind == 1 {
                for r in 0..NUM_ROUNDS {
                    for t in 0..NS {
                        for k in 0..NBYTES {
                            let bb = g2u(main.values[b + b_col(r, t, k)]);
                            let cc = g2u(main.values[b + c_col(r, t, k)]);
                            acc += (alpha - comb(bb, cc)).inverse();
                        }
                    }
                }
            }
            // table side: -(TMULT * IS_TABLE) / (α - combine(TIN,TOUT))
            let pb = row * PREP_WIDTH;
            let is_t = g2u(prep[pb + P_IS_TABLE]);
            let tmult = g2u(main.values[b + C_TMULT]);
            if is_t == 1 && tmult != 0 {
                let tin = g2u(prep[pb + P_TIN]);
                let tout = g2u(prep[pb + P_TOUT]);
                acc -= Challenge::from_u64(tmult) * (alpha - comb(tin, tout)).inverse();
            }
        }
        acc
    }

    /// Width: the lookup AIR must be dramatically narrower than the
    /// lookup-free one (the whole point of this work).
    #[test]
    fn width_is_dramatically_narrower() {
        let lookup = tip5_lookup_air_width();
        let free = crate::tip5_perm_air_width();
        std::eprintln!(
            "tip5 AIR width: lookup-table={lookup} vs lookup-free={free} \
             ({:.1}× narrower)",
            free as f64 / lookup as f64
        );
        assert!(lookup < free / 5, "expected ≥5× narrower, got {lookup} vs {free}");
    }

    /// Native-equivalence: lookup-AIR trace ROUT[6] == nockchain-math
    /// `permute`, on all 315 fixture vectors **and** 2048 random.
    #[test]
    fn lookup_air_equals_native_spec() {
        let fv = fixture_vectors();
        assert!(fv.len() >= 300);
        let mut inputs: Vec<[u64; STATE_SIZE]> = fv.iter().map(|(i, _)| *i).collect();
        let mut seed = 0xC0FFEE_1234_5678u64;
        for _ in 0..2048 {
            inputs.push(core::array::from_fn(|_| xs(&mut seed)));
        }
        let (main, prep) = generate_lookup_trace(&inputs);
        let w = main.width();

        for (pi, inp) in inputs.iter().enumerate() {
            let row = TABLE_ROWS + pi;
            let bse = row * w;
            let mut exp = *inp;
            permute(&mut exp);
            for lane in 0..STATE_SIZE {
                assert_eq!(main.values[bse + C_IN + lane], Goldilocks::from_u64(inp[lane]));
                assert_eq!(
                    main.values[bse + rout_col(NUM_ROUNDS - 1, lane)],
                    Goldilocks::from_u64(exp[lane]),
                    "lookup-AIR != nockchain-math permute, perm {pi} lane {lane}"
                );
            }
        }
        // fixture OUT must match too (== nockchain-math, by C2.0)
        for (pi, (_, out)) in fv.iter().enumerate() {
            let bse = (TABLE_ROWS + pi) * w;
            for lane in 0..STATE_SIZE {
                assert_eq!(
                    main.values[bse + rout_col(NUM_ROUNDS - 1, lane)],
                    Goldilocks::from_u64(out[lane])
                );
            }
        }

        // algebraic constraints satisfied (kind-gated; preprocessed table)
        let air = Tip5PermLookupAir::new(prep.clone());
        check_constraints(&air, &main, &[]);
        // LogUp soundness: honest accumulator is exactly zero.
        assert_eq!(
            logup_accumulator(&main, &prep),
            Challenge::ZERO,
            "honest LogUp accumulator must be 0"
        );
    }

    fn panics(f: impl FnOnce()) -> bool {
        catch_unwind(AssertUnwindSafe(f)).is_err()
    }

    /// Adversarial: tampered S-box image / output / non-canonical
    /// split are all rejected (LogUp accumulator ≠ 0 or constraints
    /// fail) — the lookup binding + §4.6 guard hold.
    #[test]
    fn lookup_air_adversarial() {
        let inputs: Vec<[u64; STATE_SIZE]> =
            vec![core::array::from_fn(|i| (i as u64) * 0x1111 + 1); 1];
        let air_of = |p: &[Goldilocks]| Tip5PermLookupAir::new(p.to_vec());
        let (good, gp) = generate_lookup_trace(&inputs);
        check_constraints(&air_of(&gp), &good, &[]);
        assert_eq!(logup_accumulator(&good, &gp), Challenge::ZERO);
        let w = good.width();
        let prow = TABLE_ROWS; // first perm row

        // (a) tamper an S-box image byte c → LogUp accumulator ≠ 0
        let mut t = good.clone();
        let cc = prow * w + c_col(0, 0, 0);
        t.values[cc] += Goldilocks::ONE;
        assert_ne!(
            logup_accumulator(&t, &gp),
            Challenge::ZERO,
            "tampered S-box image accepted by the lookup"
        );

        // (b) tamper the permutation output ROUT[6] → constraints fail
        let mut t2 = good.clone();
        t2.values[prow * w + rout_col(NUM_ROUNDS - 1, 3)] += Goldilocks::ONE;
        assert!(
            panics(|| check_constraints(&air_of(&gp), &t2, &[])),
            "tampered ROUT accepted"
        );

        // (c) §4.6 non-canonical split: rewrite lane-0 round-0 bytes to
        //     the alias x+p (H=2^32−1, L≠0) keeping recompose ≡ — only
        //     the kind-gated canonical guard must fire.
        let mut t3 = good.clone();
        // input lane0 chosen = small; alias bytes of (v + p):
        let v0 = inputs[0][0];
        let alias = (v0 as u128 + crate::tip5_spec::P_GOLDILOCKS as u128) as u64;
        if (alias as u128) < (1u128 << 64) {
            let ab = alias.to_le_bytes();
            for k in 0..NBYTES {
                t3.values[prow * w + b_col(0, 0, k)] = Goldilocks::from_u64(ab[k] as u64);
            }
            assert!(
                panics(|| check_constraints(&air_of(&gp), &t3, &[])),
                "non-canonical §4.6 split accepted"
            );
        }
    }
}
