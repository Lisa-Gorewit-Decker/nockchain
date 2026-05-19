//! Trace generation for [`crate::Tip5PermLookupAir`].
//!
//! Builds the single main trace (`256` verifier-table rows ++ `P`
//! permutation rows ++ inert padding) and the matching preprocessed
//! L-table, witnessing the real 7-round Tip5 from
//! [`crate::tip5_spec`]. `(IN, ROUT[6])` of every perm row equals
//! `nockchain_math::tip5::permute` bit-for-bit (asserted by the
//! `air_lookup::tests` native-equivalence gate).

use alloc::vec;
use alloc::vec::Vec;

use p3_goldilocks::Goldilocks;
use p3_matrix::dense::RowMajorMatrix;

use crate::air_lookup::{
    NBYTES, NS, PREP_WIDTH, TABLE_ROWS, a_col, rout_col, tip5_lookup_air_width,
};
use crate::tip5_spec::{
    LOOKUP_TABLE, NUM_ROUNDS, P_GOLDILOCKS, ROUND_CONSTANTS, STATE_SIZE, mds_matrix, rc_precomp,
};

const P: u128 = P_GOLDILOCKS as u128;

#[inline]
fn modpow(mut base: u128, mut exp: u128) -> u128 {
    base %= P;
    let mut acc = 1u128;
    while exp > 0 {
        if exp & 1 == 1 {
            acc = acc * base % P;
        }
        base = base * base % P;
        exp >>= 1;
    }
    acc
}
#[inline]
fn finv(a: u128) -> u128 {
    let a = a % P;
    if a == 0 { 0 } else { modpow(a, P - 2) }
}
#[inline]
fn fmul(a: u64, b: u64) -> u64 {
    ((a as u128) * (b as u128) % P) as u64
}
#[inline]
fn fadd(a: u64, b: u64) -> u64 {
    (((a as u128) + (b as u128)) % P) as u64
}

// flat main-trace column indices (mirror air_lookup.rs)
const C_KIND: usize = 0;
const C_TMULT: usize = 1;
const C_IN: usize = 2;
const RB0: usize = C_IN + STATE_SIZE;
const SPLIT_BC: usize = NS * 2 * NBYTES;
const PWR: usize = STATE_SIZE - NS;
const ROUND_GROUP: usize = SPLIT_BC + NS + PWR + PWR + STATE_SIZE + STATE_SIZE;
#[inline]
fn rb(r: usize) -> usize {
    RB0 + r * ROUND_GROUP
}
#[inline]
fn b_col(r: usize, t: usize, k: usize) -> usize {
    rb(r) + t * (2 * NBYTES) + k
}
#[inline]
fn c_col(r: usize, t: usize, k: usize) -> usize {
    rb(r) + t * (2 * NBYTES) + NBYTES + k
}
#[inline]
fn inv_col(r: usize, t: usize) -> usize {
    rb(r) + SPLIT_BC + t
}
#[inline]
fn x2_col(r: usize, j: usize) -> usize {
    rb(r) + SPLIT_BC + NS + (j - NS)
}
#[inline]
fn x3_col(r: usize, j: usize) -> usize {
    rb(r) + SPLIT_BC + NS + PWR + (j - NS)
}

/// Generate `(main_trace, preprocessed_flat)` for `inputs`.
///
/// `preprocessed_flat` is row-major `PREP_WIDTH`-wide, height = main
/// height: `[IS_TABLE, TIN, TOUT]`, the verifier-fixed
/// `(i, LOOKUP_TABLE[i])` on the first `TABLE_ROWS` rows.
pub fn generate_lookup_trace(
    inputs: &[[u64; STATE_SIZE]],
) -> (RowMajorMatrix<Goldilocks>, Vec<Goldilocks>) {
    let width = tip5_lookup_air_width();
    let p = inputs.len();
    let height = (TABLE_ROWS + p).max(1).next_power_of_two();

    let mut main = vec![Goldilocks::new(0); height * width];
    let mut prep = vec![Goldilocks::new(0); height * PREP_WIDTH];
    let mds = mds_matrix();

    // ---- preprocessed verifier-fixed L-table on rows [0, 256) ----
    for (i, tbl) in LOOKUP_TABLE.iter().enumerate() {
        prep[i * PREP_WIDTH] = Goldilocks::new(1); // IS_TABLE
        prep[i * PREP_WIDTH + 1] = Goldilocks::new(i as u64); // TIN
        prep[i * PREP_WIDTH + 2] = Goldilocks::new(*tbl as u64); // TOUT
    }

    // Global multiplicity of each byte-key i, filled exactly during
    // the witnessing pass below (`mult[b] += 1` per queried byte).
    let mut mult = [0u64; TABLE_ROWS];

    let set = |m: &mut Vec<Goldilocks>, base: usize, col: usize, v: u64| {
        m[base + col] = Goldilocks::new(v % P_GOLDILOCKS);
    };

    // ---- permutation rows [256, 256+p) ----
    for (pi, inp) in inputs.iter().enumerate() {
        let row = TABLE_ROWS + pi;
        let base = row * width;
        set(&mut main, base, C_KIND, 1);
        for lane in 0..STATE_SIZE {
            set(&mut main, base, C_IN + lane, inp[lane]);
        }
        let mut state = *inp;
        for r in 0..NUM_ROUNDS {
            let sbox_in = state;
            let mut a = [0u64; STATE_SIZE];
            for t in 0..NS {
                let bytes = sbox_in[t].to_le_bytes();
                let mut cb = [0u8; NBYTES];
                for k in 0..NBYTES {
                    let b = bytes[k];
                    let c = LOOKUP_TABLE[b as usize];
                    mult[b as usize] += 1;
                    cb[k] = c;
                    set(&mut main, base, b_col(r, t, k), b as u64);
                    set(&mut main, base, c_col(r, t, k), c as u64);
                }
                a[t] = u64::from_le_bytes(cb);
                let high = (sbox_in[t] >> 32) & 0xffff_ffff;
                let g = ((high as u128) + P - ((1u128 << 32) - 1)) % P;
                set(&mut main, base, inv_col(r, t), finv(g) as u64);
            }
            for j in NS..STATE_SIZE {
                let x = sbox_in[j];
                let x2 = fmul(x, x);
                let x3 = fmul(x2, x);
                a[j] = fmul(fmul(x3, x3), x);
                set(&mut main, base, x2_col(r, j), x2);
                set(&mut main, base, x3_col(r, j), x3);
            }
            for i in 0..STATE_SIZE {
                set(&mut main, base, a_col(r, i), a[i]);
            }
            for i in 0..STATE_SIZE {
                let mut acc = 0u64;
                for (j, &aj) in a.iter().enumerate() {
                    acc = fadd(acc, fmul(mds[i][j], aj));
                }
                let out = fadd(acc, rc_precomp(ROUND_CONSTANTS[r * STATE_SIZE + i]));
                set(&mut main, base, rout_col(r, i), out);
                state[i] = out;
            }
        }
    }

    // ---- table-row multiplicities (rows [0,256)) ----
    for (i, &m) in mult.iter().enumerate() {
        let base = i * width;
        // KIND already 0; TMULT = how many perm-row byte queries hit i
        set(&mut main, base, C_TMULT, m % P_GOLDILOCKS);
    }

    (RowMajorMatrix::new(main, width), prep)
}
