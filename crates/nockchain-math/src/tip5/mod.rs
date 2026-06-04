pub mod hash;

use arrayref::array_ref;

use crate::belt::*;

pub const DIGEST_LENGTH: usize = 5;
pub const STATE_SIZE: usize = 16;
pub const NUM_SPLIT_AND_LOOKUP: usize = 4;
pub const LOG2_STATE_SIZE: usize = 4;
pub const CAPACITY: usize = 6;
pub const RATE: usize = 10;
pub const NUM_ROUNDS: usize = 7;
pub const R: u128 = 18446744073709551616;
pub const R2: u64 = 0xfffffffe00000001;
pub const R_MOD_P: u64 = 4294967295;
pub const RP: u128 = 0xffffffff000000010000000000000000;
pub const P: u64 = 0xffffffff00000001;

const LOOKUP_TABLE: [u8; 256] = [
    0, 7, 26, 63, 124, 215, 85, 254, 214, 228, 45, 185, 140, 173, 33, 240, 29, 177, 176, 32, 8,
    110, 87, 202, 204, 99, 150, 106, 230, 14, 235, 128, 213, 239, 212, 138, 23, 130, 208, 6, 44,
    71, 93, 116, 146, 189, 251, 81, 199, 97, 38, 28, 73, 179, 95, 84, 152, 48, 35, 119, 49, 88,
    242, 3, 148, 169, 72, 120, 62, 161, 166, 83, 175, 191, 137, 19, 100, 129, 112, 55, 221, 102,
    218, 61, 151, 237, 68, 164, 17, 147, 46, 234, 203, 216, 22, 141, 65, 57, 123, 12, 244, 54, 219,
    231, 96, 77, 180, 154, 5, 253, 133, 165, 98, 195, 205, 134, 245, 30, 9, 188, 59, 142, 186, 197,
    181, 144, 92, 31, 224, 163, 111, 74, 58, 69, 113, 196, 67, 246, 225, 10, 121, 50, 60, 157, 90,
    122, 2, 250, 101, 75, 178, 159, 24, 36, 201, 11, 243, 132, 198, 190, 114, 233, 39, 52, 21, 209,
    108, 238, 91, 187, 18, 104, 194, 37, 153, 34, 200, 143, 126, 155, 236, 118, 64, 80, 172, 89,
    94, 193, 135, 183, 86, 107, 252, 13, 167, 206, 136, 220, 207, 103, 171, 160, 76, 182, 227, 217,
    158, 56, 174, 4, 66, 109, 139, 162, 184, 211, 249, 47, 125, 232, 117, 43, 16, 42, 127, 20, 241,
    25, 149, 105, 156, 51, 53, 168, 145, 247, 223, 79, 78, 226, 15, 222, 82, 115, 70, 210, 27, 41,
    1, 170, 40, 131, 192, 229, 248, 255,
];

const ROUND_CONSTANTS: [u64; NUM_ROUNDS * STATE_SIZE] = [
    // 1st round constants
    1332676891236936200, 16607633045354064669, 12746538998793080786, 15240351333789289931,
    10333439796058208418, 986873372968378050, 153505017314310505, 703086547770691416,
    8522628845961587962, 1727254290898686320, 199492491401196126, 2969174933639985366,
    1607536590362293391, 16971515075282501568, 15401316942841283351, 14178982151025681389,
    // 2nd round constants
    2916963588744282587, 5474267501391258599, 5350367839445462659, 7436373192934779388,
    12563531800071493891, 12265318129758141428, 6524649031155262053, 1388069597090660214,
    3049665785814990091, 5225141380721656276, 10399487208361035835, 6576713996114457203,
    12913805829885867278, 10299910245954679423, 12980779960345402499, 593670858850716490,
    // 3rd round constants
    12184128243723146967, 1315341360419235257, 9107195871057030023, 4354141752578294067,
    8824457881527486794, 14811586928506712910, 7768837314956434138, 2807636171572954860,
    9487703495117094125, 13452575580428891895, 14689488045617615844, 16144091782672017853,
    15471922440568867245, 17295382518415944107, 15054306047726632486, 5708955503115886019,
    // 4th round constants
    9596017237020520842, 16520851172964236909, 8513472793890943175, 8503326067026609602,
    9402483918549940854, 8614816312698982446, 7744830563717871780, 14419404818700162041,
    8090742384565069824, 15547662568163517559, 17314710073626307254, 10008393716631058961,
    14480243402290327574, 13569194973291808551, 10573516815088946209, 15120483436559336219,
    // 5th round constants
    3515151310595301563, 1095382462248757907, 5323307938514209350, 14204542692543834582,
    12448773944668684656, 13967843398310696452, 14838288394107326806, 13718313940616442191,
    15032565440414177483, 13769903572116157488, 17074377440395071208, 16931086385239297738,
    8723550055169003617, 590842605971518043, 16642348030861036090, 10708719298241282592,
    // 6th round constants
    12766914315707517909, 11780889552403245587, 113183285481780712, 9019899125655375514,
    3300264967390964820, 12802381622653377935, 891063765000023873, 15939045541699412539,
    3240223189948727743, 4087221142360949772, 10980466041788253952, 18199914337033135244,
    7168108392363190150, 16860278046098150740, 13088202265571714855, 4712275036097525581,
    // 7th round constants
    16338034078141228133, 1455012125527134274, 5024057780895012002, 9289161311673217186,
    9401110072402537104, 11919498251456187748, 4173156070774045271, 15647643457869530627,
    15642078237964257476, 1405048341078324037, 3059193199283698832, 1605012781983592984,
    7134876918849821827, 5796994175286958720, 7251651436095127661, 4565856221886323991,
];

// The full circulant MDS matrix. Since the 2026-05-21 cyclomul
// optimisation, `linear_layer` uses `MDS_FIRST_COLUMN_I64` (column 0)
// as the cyclic-convolution kernel; the full matrix is retained only
// as the `linear_layer_cyclomul_matches_naive` differential-test
// oracle, hence `#[cfg(test)]`.
#[cfg(test)]
const MDS_MATRIX_I64: [[i64; STATE_SIZE]; STATE_SIZE] = [
    [
        61402, 17845, 26798, 59689, 12021, 40901, 41351, 27521, 56951, 12034, 53865, 43244, 7454,
        33823, 28750, 1108,
    ],
    [
        1108, 61402, 17845, 26798, 59689, 12021, 40901, 41351, 27521, 56951, 12034, 53865, 43244,
        7454, 33823, 28750,
    ],
    [
        28750, 1108, 61402, 17845, 26798, 59689, 12021, 40901, 41351, 27521, 56951, 12034, 53865,
        43244, 7454, 33823,
    ],
    [
        33823, 28750, 1108, 61402, 17845, 26798, 59689, 12021, 40901, 41351, 27521, 56951, 12034,
        53865, 43244, 7454,
    ],
    [
        7454, 33823, 28750, 1108, 61402, 17845, 26798, 59689, 12021, 40901, 41351, 27521, 56951,
        12034, 53865, 43244,
    ],
    [
        43244, 7454, 33823, 28750, 1108, 61402, 17845, 26798, 59689, 12021, 40901, 41351, 27521,
        56951, 12034, 53865,
    ],
    [
        53865, 43244, 7454, 33823, 28750, 1108, 61402, 17845, 26798, 59689, 12021, 40901, 41351,
        27521, 56951, 12034,
    ],
    [
        12034, 53865, 43244, 7454, 33823, 28750, 1108, 61402, 17845, 26798, 59689, 12021, 40901,
        41351, 27521, 56951,
    ],
    [
        56951, 12034, 53865, 43244, 7454, 33823, 28750, 1108, 61402, 17845, 26798, 59689, 12021,
        40901, 41351, 27521,
    ],
    [
        27521, 56951, 12034, 53865, 43244, 7454, 33823, 28750, 1108, 61402, 17845, 26798, 59689,
        12021, 40901, 41351,
    ],
    [
        41351, 27521, 56951, 12034, 53865, 43244, 7454, 33823, 28750, 1108, 61402, 17845, 26798,
        59689, 12021, 40901,
    ],
    [
        40901, 41351, 27521, 56951, 12034, 53865, 43244, 7454, 33823, 28750, 1108, 61402, 17845,
        26798, 59689, 12021,
    ],
    [
        12021, 40901, 41351, 27521, 56951, 12034, 53865, 43244, 7454, 33823, 28750, 1108, 61402,
        17845, 26798, 59689,
    ],
    [
        59689, 12021, 40901, 41351, 27521, 56951, 12034, 53865, 43244, 7454, 33823, 28750, 1108,
        61402, 17845, 26798,
    ],
    [
        26798, 59689, 12021, 40901, 41351, 27521, 56951, 12034, 53865, 43244, 7454, 33823, 28750,
        1108, 61402, 17845,
    ],
    [
        17845, 26798, 59689, 12021, 40901, 41351, 27521, 56951, 12034, 53865, 43244, 7454, 33823,
        28750, 1108, 61402,
    ],
];

/// First **column** of the circulant [`MDS_MATRIX_I64`] — i.e.
/// `MDS_FIRST_COLUMN_I64[i] = MDS_MATRIX_I64[i][0]`.
///
/// Used by [`mds_cyclomul`] as the kernel of a cyclic convolution.
/// For any circulant matrix `C` (every row a rotation of the first
/// row), the matrix-vector product `C · v` equals the cyclic
/// convolution `first_column(C) ⋆ v`. This is the standard
/// linear-algebra identity that makes the Karatsuba-based
/// `mds_cyclomul` a bit-for-bit-equivalent, ~4×-fewer-multiplication
/// replacement for the naive O(n²) [`linear_layer`].
const MDS_FIRST_COLUMN_I64: [i64; STATE_SIZE] = [
    61402, 1108, 28750, 33823, 7454, 43244, 53865, 12034, 56951, 27521, 41351, 40901, 12021, 59689,
    26798, 17845,
];

pub fn permute(sponge: &mut [u64; 16]) {
    for i in 0..NUM_ROUNDS {
        let a = sbox_layer(array_ref![sponge, 0, STATE_SIZE]);
        let b = linear_layer(&a);

        for j in 0..STATE_SIZE {
            let r_cons = (((ROUND_CONSTANTS[i * STATE_SIZE + j] as u128) * R) % PRIME_128) as u64;
            sponge[j] = badd(r_cons, b[j]);
        }
    }
}

/// Round count for the **paper-spec 5-round Tip5 variant** (Tip5
/// paper IACR ePrint 2023/107 §2.4 N=5). Distinct from the
/// canonical Nockchain [`NUM_ROUNDS`] (= 7), which is the deployed
/// margin per the cryptanalysis (IACR 2024/1900 "Opening the
/// Blackbox": practical 3-round attacks; 5-round paper-spec is
/// secure; 7-round = Nockchain's defensive cushion).
///
/// This variant exists **specifically for the ai-pow-zk recursive
/// proving construction** (per maintainer 2026-05-20). It is NOT
/// the canonical Nockchain Tip5 — all other Nockchain crates
/// continue to use [`permute`] (7 rounds).
pub const NUM_ROUNDS_5ROUND: usize = 5;

/// **5-round Tip5 variant for ai-pow-zk recursive certificate
/// proving** — identical to [`permute`] (same MDS, same LOOKUP_TABLE,
/// same RC schedule, same S-box) but iterates only 5 rounds using
/// `ROUND_CONSTANTS[0..5*STATE_SIZE]` (i.e., the FIRST 80 of the 112
/// round constants).
///
/// Tip5 paper IACR ePrint 2023/107 §2.4 specifies N=5 as the
/// secure round count; the cryptanalysis (IACR ePrint 2024/1900)
/// shows practical 3-round attacks and 5-round security. The
/// canonical Nockchain [`permute`] uses 7 rounds as a defensive
/// 2-round cushion above the paper spec; this 5-round variant
/// matches the paper-spec for the ai-pow-zk recursive proving
/// construction where the cushion is traded for proof-size
/// reduction (~−25-30% in the in-circuit Tip5 AIR width).
///
/// **Do not use this for canonical Nockchain hashing.** It is
/// specifically an ai-pow-zk recursive-certificate-only variant; all
/// other Nockchain crates must continue to use the 7-round
/// [`permute`].
pub fn permute_5round(sponge: &mut [u64; 16]) {
    for i in 0..NUM_ROUNDS_5ROUND {
        let a = sbox_layer(array_ref![sponge, 0, STATE_SIZE]);
        let b = linear_layer(&a);

        for j in 0..STATE_SIZE {
            let r_cons = (((ROUND_CONSTANTS[i * STATE_SIZE + j] as u128) * R) % PRIME_128) as u64;
            sponge[j] = badd(r_cons, b[j]);
        }
    }
}

fn sbox_layer(state: &[u64; STATE_SIZE]) -> [u64; STATE_SIZE] {
    let mut res: [u64; STATE_SIZE] = [0; STATE_SIZE];

    for i in 0..NUM_SPLIT_AND_LOOKUP {
        let mut bytes = state[i].to_le_bytes();
        for i in 0..8 {
            bytes[i] = LOOKUP_TABLE[bytes[i] as usize];
        }
        res[i] = u64::from_le_bytes(bytes);
    }

    for j in NUM_SPLIT_AND_LOOKUP..STATE_SIZE {
        res[j] = bpow(state[j], 7);
    }
    res
}

/// Tip5 linear layer — the circulant MDS matrix-vector product
/// `MDS_MATRIX_I64 · state`, reduced mod the Goldilocks prime.
///
/// **2026-05-21 optimisation.** Computed as a **cyclic convolution
/// of [`MDS_FIRST_COLUMN_I64`] with `state`** via Karatsuba
/// polynomial multiplication in `Z[x]/(x¹⁶−1)` (see [`mds_cyclomul`]).
/// This replaces the naive O(n²) = 256-multiply loop with ~64
/// i64-multiplications — the MDS layer is ~¾ of the per-permutation
/// cost, so this is a ~2-3× speedup on `permute` / `permute_5round`,
/// which directly accelerates the ai-pow-zk inner STARK's Tip5-MMCS
/// trace commitment (the dominant cost of block proving — see
/// `crates/ai-pow-zk/docs/2026-05-21_E2E_LATENCY_AND_SWEEP_MEASUREMENTS.md`).
///
/// **Bit-for-bit identical** to the prior naive implementation
/// (preserved as `tests::linear_layer_naive`): a circulant
/// matrix-vector product *equals* the cyclic convolution of the
/// matrix's first column with the vector, a standard linear-algebra
/// identity. Gated by the `linear_layer_cyclomul_matches_naive`
/// differential test below AND the binding
/// `c2_kat::golden_kat_frozen_matches_live_permute` gate (the
/// frozen-oracle KAT — `permute`'s output is unchanged).
fn linear_layer(state: &[u64; 16]) -> [u64; 16] {
    mds_cyclomul(state)
}

// =====================================================================
//  Cyclic-convolution MDS via Karatsuba.
//
//  The MDS matrix is circulant. For a circulant matrix C built from
//  first row r, the matrix-vector product C·v equals the cyclic
//  convolution of v with first_column(C). Cyclic convolution mod
//  (x¹⁶−1) is polynomial multiplication mod (x¹⁶−1), computed via CRT:
//  (x¹⁶−1) = (x⁸−1)(x⁸+1); (x⁸−1) = (x⁴−1)(x⁴+1); (x⁸+1) factors
//  over Z[i]. Karatsuba multiplication at each level. ~64
//  i64-multiplications vs ~256 field-multiplications naive.
//
//  Verbatim mirror of the validated implementation in the
//  Plonky3-recursion `p3-tip5-circuit-air` crate's `tip5_spec.rs`
//  (`mds_cyclomul`), itself gated there by a differential test +
//  the C2 Tip5 AIR native-equivalence KAT. The differential test
//  `tests::linear_layer_cyclomul_matches_naive` re-establishes the
//  bit-identity locally against the naive reference.
// =====================================================================

#[derive(Copy, Clone, Default, PartialEq, Eq)]
struct Complex([i64; 2]);

#[inline(always)]
fn cadd(a: Complex, b: Complex) -> Complex {
    Complex([a.0[0] + b.0[0], a.0[1] + b.0[1]])
}

#[inline(always)]
fn csub3(a: Complex, b: Complex, c: Complex) -> Complex {
    Complex([a.0[0] - b.0[0] - c.0[0], a.0[1] - b.0[1] - c.0[1]])
}

#[inline(always)]
fn cmul(f: Complex, g: Complex) -> Complex {
    let a = f.0[0] * g.0[0];
    let b = f.0[1] * g.0[1];
    let c = (f.0[0] + f.0[1]) * (g.0[0] + g.0[1]);
    Complex([a - b, c - a - b])
}

#[inline(always)]
fn cpoly_add<const N: usize>(f: &[Complex; N], g: &[Complex; N]) -> [Complex; N] {
    let mut res = [Complex([0, 0]); N];
    for i in 0..N {
        res[i] = cadd(f[i], g[i]);
    }
    res
}

#[inline(always)]
fn cpoly_sub3<const N: usize>(
    f: &[Complex; N],
    g: &[Complex; N],
    h: &[Complex; N],
) -> [Complex; N] {
    let mut res = [Complex([0, 0]); N];
    for i in 0..N {
        res[i] = csub3(f[i], g[i], h[i]);
    }
    res
}

#[inline(always)]
fn zpoly_add<const N: usize>(f: &[i64; N], g: &[i64; N]) -> [i64; N] {
    let mut res = [0i64; N];
    for i in 0..N {
        res[i] = f[i] + g[i];
    }
    res
}

#[inline(always)]
fn zpoly_sub<const N: usize>(f: &[i64; N], g: &[i64; N]) -> [i64; N] {
    let mut res = [0i64; N];
    for i in 0..N {
        res[i] = f[i] - g[i];
    }
    res
}

#[inline(always)]
fn zpoly_sub3<const N: usize>(f: &[i64; N], g: &[i64; N], h: &[i64; N]) -> [i64; N] {
    let mut res = [0i64; N];
    for i in 0..N {
        res[i] = f[i] - g[i] - h[i];
    }
    res
}

#[inline(always)]
fn integer_karatsuba_1(f: &[i64; 2], g: &[i64; 2]) -> [i64; 3] {
    let a = f[0] * g[0];
    let c = f[1] * g[1];
    let b = ((f[0] + f[1]) * (g[0] + g[1])) - a - c;
    [a, b, c]
}

#[inline(always)]
fn integer_karatsuba_3(f: &[i64; 4], g: &[i64; 4]) -> [i64; 7] {
    let a0 = [f[2], f[3]];
    let a1 = [f[0], f[1]];
    let b0 = [g[2], g[3]];
    let b1 = [g[0], g[1]];

    let m0 = integer_karatsuba_1(&a0, &b0);
    let m2 = integer_karatsuba_1(&a1, &b1);
    let m1 = zpoly_sub3(
        &integer_karatsuba_1(&zpoly_add(&a0, &a1), &zpoly_add(&b0, &b1)),
        &m0,
        &m2,
    );
    [m2[0], m2[1], m2[2] + m1[0], m1[1], m1[2] + m0[0], m0[1], m0[2]]
}

#[inline(always)]
fn complex_karatsuba_1(f: &[Complex; 2], g: &[Complex; 2]) -> [Complex; 3] {
    let a = cmul(f[0], g[0]);
    let c = cmul(f[1], g[1]);
    let b = csub3(cmul(cadd(f[0], f[1]), cadd(g[0], g[1])), a, c);
    [a, b, c]
}

#[inline(always)]
fn complex_karatsuba_3(f: &[Complex; 4], g: &[Complex; 4]) -> [Complex; 7] {
    let a0 = [f[2], f[3]];
    let a1 = [f[0], f[1]];
    let b0 = [g[2], g[3]];
    let b1 = [g[0], g[1]];

    let m0 = complex_karatsuba_1(&a0, &b0);
    let m2 = complex_karatsuba_1(&a1, &b1);
    let mid = complex_karatsuba_1(&cpoly_add(&a0, &a1), &cpoly_add(&b0, &b1));
    let m1 = cpoly_sub3(&mid, &m0, &m2);
    [m2[0], m2[1], cadd(m2[2], m1[0]), m1[1], cadd(m1[2], m0[0]), m0[1], m0[2]]
}

/// `f·g` in `Z[x] / (x⁴ + 1)`.
#[inline(always)]
fn poly_mul_mod_x4_plus_1(f: &[i64; 4], g: &[i64; 4]) -> [i64; 4] {
    let prod = integer_karatsuba_3(f, g);
    [prod[0] - prod[4], prod[1] - prod[5], prod[2] - prod[6], prod[3]]
}

/// `f·g` in `Z[x] / (x⁴ − 1)`.
#[inline(always)]
fn poly_mul_mod_x4_minus_1(f: &[i64; 4], g: &[i64; 4]) -> [i64; 4] {
    let prod = integer_karatsuba_3(f, g);
    [prod[0] + prod[4], prod[1] + prod[5], prod[2] + prod[6], prod[3]]
}

/// `f·g` in `Z[x] / (x⁸ − 1)` via CRT `(x⁸−1) = (x⁴−1)(x⁴+1)`.
#[inline(always)]
fn poly_mul_mod_x8_minus_1(f: &[i64; 8], g: &[i64; 8]) -> [i64; 8] {
    let f0 = [f[0], f[1], f[2], f[3]];
    let f1 = [f[4], f[5], f[6], f[7]];
    let g0 = [g[0], g[1], g[2], g[3]];
    let g1 = [g[4], g[5], g[6], g[7]];

    let p0 = poly_mul_mod_x4_plus_1(&zpoly_sub(&f0, &f1), &zpoly_sub(&g0, &g1));
    let p1 = poly_mul_mod_x4_minus_1(&zpoly_add(&f0, &f1), &zpoly_add(&g0, &g1));
    [
        (p0[0] + p1[0]) >> 1,
        (p0[1] + p1[1]) >> 1,
        (p0[2] + p1[2]) >> 1,
        (p0[3] + p1[3]) >> 1,
        (-p0[0] + p1[0]) >> 1,
        (-p0[1] + p1[1]) >> 1,
        (-p0[2] + p1[2]) >> 1,
        (-p0[3] + p1[3]) >> 1,
    ]
}

/// `f·g` in `Z[x] / (x⁸ + 1)` — requires the Gaussian integers
/// `Z[i]` because `(x⁸+1) = (x⁴+i)(x⁴−i)`.
#[inline(always)]
fn poly_mul_mod_x8_plus_1(f: &[i64; 8], g: &[i64; 8]) -> [i64; 8] {
    let cf = [
        Complex([f[0], -f[4]]),
        Complex([f[1], -f[5]]),
        Complex([f[2], -f[6]]),
        Complex([f[3], -f[7]]),
    ];
    let cg = [
        Complex([g[0], -g[4]]),
        Complex([g[1], -g[5]]),
        Complex([g[2], -g[6]]),
        Complex([g[3], -g[7]]),
    ];
    let p = complex_karatsuba_3(&cf, &cg);
    [
        p[0].0[0] + p[4].0[1],
        p[1].0[0] + p[5].0[1],
        p[2].0[0] + p[6].0[1],
        p[3].0[0],
        p[4].0[0] - p[0].0[1],
        p[5].0[0] - p[1].0[1],
        p[6].0[0] - p[2].0[1],
        -p[3].0[1],
    ]
}

/// `f·g` in `Z[x] / (x¹⁶ − 1)` via CRT `(x¹⁶−1) = (x⁸−1)(x⁸+1)`.
#[inline(always)]
fn poly_mul_mod_x16_minus_1(f: &[i64; 16], g: &[i64; 16]) -> [i64; 16] {
    let f0 = [f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7]];
    let f1 = [f[8], f[9], f[10], f[11], f[12], f[13], f[14], f[15]];
    let g0 = [g[0], g[1], g[2], g[3], g[4], g[5], g[6], g[7]];
    let g1 = [g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]];

    let p0 = poly_mul_mod_x8_minus_1(&zpoly_add(&f0, &f1), &zpoly_add(&g0, &g1));
    let p1 = poly_mul_mod_x8_plus_1(&zpoly_sub(&f0, &f1), &zpoly_sub(&g0, &g1));
    [
        (p0[0] + p1[0]) >> 1,
        (p0[1] + p1[1]) >> 1,
        (p0[2] + p1[2]) >> 1,
        (p0[3] + p1[3]) >> 1,
        (p0[4] + p1[4]) >> 1,
        (p0[5] + p1[5]) >> 1,
        (p0[6] + p1[6]) >> 1,
        (p0[7] + p1[7]) >> 1,
        (p0[0] - p1[0]) >> 1,
        (p0[1] - p1[1]) >> 1,
        (p0[2] - p1[2]) >> 1,
        (p0[3] - p1[3]) >> 1,
        (p0[4] - p1[4]) >> 1,
        (p0[5] - p1[5]) >> 1,
        (p0[6] - p1[6]) >> 1,
        (p0[7] - p1[7]) >> 1,
    ]
}

const LO_MASK_U64: u64 = 0x0000_0000_ffff_ffff;

/// Reduce a (possibly negative) i128 to canonical Goldilocks
/// `[0, P)`. `rem_euclid` handles the sign; the cyclic-convolution
/// results are mathematically non-negative for non-negative inputs
/// but the Karatsuba decomposition produces signed intermediates.
#[inline(always)]
fn reduce_i128(n: i128) -> u64 {
    n.rem_euclid(P as i128) as u64
}

/// Circulant-MDS matrix-vector product `MDS_MATRIX_I64 · state` mod
/// `P`, computed as the cyclic convolution `MDS_FIRST_COLUMN ⋆ state`.
/// Bit-for-bit identical to the naive O(n²) implementation for any
/// `state ∈ [0, P)¹⁶`.
fn mds_cyclomul(state: &[u64; STATE_SIZE]) -> [u64; STATE_SIZE] {
    // Split each state element into hi/lo 32-bit halves so the
    // integer convolutions stay well inside i64.
    let hi: [i64; STATE_SIZE] = core::array::from_fn(|i| (state[i] >> 32) as i64);
    let lo: [i64; STATE_SIZE] = core::array::from_fn(|i| (state[i] & LO_MASK_U64) as i64);

    let hi_res = poly_mul_mod_x16_minus_1(&MDS_FIRST_COLUMN_I64, &hi);
    let lo_res = poly_mul_mod_x16_minus_1(&MDS_FIRST_COLUMN_I64, &lo);

    core::array::from_fn(|i| reduce_i128(((hi_res[i] as i128) << 32) + (lo_res[i] as i128)))
}

/// C2.0 — the Tip5 soundness oracle, frozen.
///
/// This module is the *normative bit-for-bit reference* the C2
/// `tip5-circuit-air` (in the separate, vendored Plonky3-recursion
/// workspace) must reproduce exactly, or the recursion verifier would
/// accept forged Layer-0 proofs. It does two soundness-critical things:
///
///  1. **L-table identity** — proves `LOOKUP_TABLE[b] == ((b+1)^3 - 1)
///     mod 257` for every byte `b` (the paper's split-and-lookup map L,
///     ePrint 2023/107 §2.2), that L is a bijection on `0..256`, and the
///     paper's fixed points (0, 255). This anchors *which* 256-row table
///     the AIR's lookup argument must embed.
///  2. **Golden KAT freeze** — emits a versioned, dependency-free text
///     fixture (`crates/ai-pow-zk/tests/fixtures/tip5_golden_kat.txt`)
///     holding the private constant tables (`LOOKUP_TABLE`,
///     `ROUND_CONSTANTS`, the AIR's precomputed `rc[i][j] = (RC·2^64) mod
///     p`, the circulant MDS first row) and `(input,output)` vectors of
///     `permute` over edge cases + seeded pseudo-random states. Both this
///     crate and `tip5-circuit-air` pin to this *same committed file*:
///     here we assert it still matches live `permute`/consts (so it can
///     never silently drift from the oracle); there the AIR is asserted
///     bit-identical to it. That closes the cross-workspace soundness
///     loop without `tip5-circuit-air` depending on `nockchain-math`.
///
/// Set `REGEN_TIP5_KAT=1` to (re)write the fixture; otherwise the test
/// is read-only and *fails on any drift* between the committed fixture
/// and live `permute`/constants.
#[cfg(test)]
mod c2_kat {
    use super::*;

    /// Goldilocks prime, as the oracle uses it (`belt::PRIME`).
    const P: u128 = 18446744069414584321;
    /// 2^64, the Montgomery factor applied to round constants in `permute`.
    const R_U128: u128 = 1u128 << 64;

    /// The paper's L: identifying {0..255} ⊂ F_257, x ↦ (x+1)^3 − 1.
    fn l_map(b: u16) -> u16 {
        let x = (b as u32 + 1) % 257;
        ((((x * x % 257) * x % 257) + 257 - 1) % 257) as u16
    }

    /// **2026-05-21 Path-1 differential gate.** The cyclic-convolution
    /// `mds_cyclomul` (now the body of `linear_layer`) must produce
    /// byte-identical field output to the naive O(n²) MDS
    /// matrix-vector product for every state — a circulant matvec
    /// *is* the cyclic convolution of the matrix's first column with
    /// the vector. Edge cases + 256 deterministic-seeded random
    /// states. Together with `golden_kat_frozen_matches_live_permute`
    /// (the frozen-oracle KAT, which exercises the full `permute`
    /// using the new `linear_layer`), this closes the bit-identity
    /// proof for the 2026-05-21 MDS optimisation.
    #[test]
    fn linear_layer_cyclomul_matches_naive() {
        // Naive O(n²) reference — the pre-2026-05-21 `linear_layer`
        // body, preserved verbatim here as the differential oracle.
        fn linear_layer_naive(state: &[u64; 16]) -> [u64; 16] {
            let mut result = [0u64; 16];
            for i in 0..16 {
                for j in 0..16 {
                    let matrix_element = MDS_MATRIX_I64[i][j] as u64;
                    let product = bmul(matrix_element, state[j]);
                    result[i] = badd(result[i], product);
                }
            }
            result
        }

        const PRIME: u64 = 0xffff_ffff_0000_0001;

        // Edge: all zeros.
        let zero = [0u64; 16];
        assert_eq!(linear_layer_naive(&zero), mds_cyclomul(&zero));

        // Edge: all P-1.
        let max = [PRIME - 1; 16];
        assert_eq!(linear_layer_naive(&max), mds_cyclomul(&max));

        // Edge: single non-zero per slot.
        for k in 0..16 {
            let mut s = [0u64; 16];
            s[k] = PRIME - 1;
            assert_eq!(
                linear_layer_naive(&s),
                mds_cyclomul(&s),
                "single-nonzero slot {k}"
            );
        }

        // Deterministic-seeded random states (LCG; no rand dep).
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut lcg = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed
        };
        for case in 0..256 {
            let state: [u64; 16] = core::array::from_fn(|_| lcg() % PRIME);
            assert_eq!(
                linear_layer_naive(&state),
                mds_cyclomul(&state),
                "random case {case}"
            );
        }
    }

    #[test]
    fn l_table_identity_bijection_fixed_points() {
        // 1. LOOKUP_TABLE[b] == ((b+1)^3 - 1) mod 257, ∀ b∈0..256.
        for b in 0u16..256 {
            let got = LOOKUP_TABLE[b as usize] as u16;
            let want = l_map(b);
            assert_eq!(
                got, want,
                "L-table mismatch at b={b}: table={got}, (x+1)^3-1 mod257={want}"
            );
            assert!(
                want < 256,
                "L({b})={want} not a byte — table type is [u8;256]"
            );
        }
        // 2. Bijection on bytes: all 256 values distinct (a permutation).
        let mut seen = [false; 256];
        for &v in LOOKUP_TABLE.iter() {
            assert!(!seen[v as usize], "LOOKUP_TABLE not a bijection: dup {v}");
            seen[v as usize] = true;
        }
        // 3. Paper's fixed points representable as bytes (0 and 255;
        //    256≡−1 mod 257 is the non-byte third fixed point).
        assert_eq!(LOOKUP_TABLE[0], 0, "L(0) must be 0");
        assert_eq!(LOOKUP_TABLE[255], 255, "L(255) must be 255");
    }

    /// `((RC as u128) * 2^64) % p` — the per-(round,lane) constant the
    /// AIR embeds (exactly `permute`'s `r_cons`).
    fn rc_precomp(rc_raw: u64) -> u64 {
        (((rc_raw as u128) * R_U128) % P) as u64
    }

    /// Deterministic xorshift64* — frozen so vectors never change.
    fn xs(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *state = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D)) % (P as u64)
    }

    /// The frozen set of permutation inputs — broad, deterministic, and
    /// targeted at each constraint component of the in-circuit Tip5 AIR
    /// per the authoritative Tip5 paper (IACR ePrint 2023/107):
    ///
    /// * **§2.2 split-and-lookup S-box** (`S`, the 4 split lanes; the
    ///   `L`-map `(x+1)^3−1 mod 257` with fixed points 0, 255): all-zero
    ///   / all-`0xFF` split bytes, single-byte sweeps, every split lane
    ///   set to `p−1`.
    /// * **§2.2 power map** (`T: x ↦ x^7`, the 12 non-split lanes):
    ///   small, large, and boundary lane values.
    /// * **§2.3 linear (circulant MDS) layer**: 16 single-lane impulses
    ///   (one lane `= 1`, rest `0`) so every MDS column is exercised.
    /// * **§2.4 round constants**: the all-zero input isolates the RC
    ///   addition schedule across all 7 rounds.
    /// * **§4.6 correct decomposition mod p**: the canonical boundary
    ///   band (`p−1`, `p−2`, values straddling the 2^32 limb split).
    /// * **§2.1 round iteration / coupling**: chained multi-permute
    ///   vectors (added in `build_fixture`).
    ///
    /// Plus 256 seeded-xorshift states for broad coverage. Every vector
    /// is run through `nockchain_math::tip5::permute` and frozen; the
    /// `golden_kat_frozen_matches_live_permute` test re-verifies the
    /// committed fixture against live `permute`, so the in-circuit AIR
    /// (which asserts trace == this fixture) is exhaustively tested
    /// against *this* implementation over the whole set.
    fn golden_inputs() -> Vec<[u64; 16]> {
        let p = P as u64;
        let p_minus_1 = p - 1; // 0xffffffff00000000
        let mut v: Vec<[u64; 16]> = vec![
            // §2.4: zero input isolates the round-constant schedule.
            [0u64; 16],
            [1u64; 16],
            core::array::from_fn(|i| i as u64),
            // §4.6: canonical boundary band.
            [p_minus_1; 16],
            [p_minus_1 - 1; 16],
            core::array::from_fn(|i| if i % 2 == 0 { 0 } else { p_minus_1 }),
            core::array::from_fn(|i| (1u64 << 32).wrapping_sub(1).wrapping_add(i as u64)),
            core::array::from_fn(|i| (1u64 << 32).wrapping_add((i as u64) << 8)),
            // §2.2 split-and-lookup: byte-patterned distinct per-byte L.
            {
                let mut s = [0u64; 16];
                s[0] = 0x0001_0203_0405_0607;
                s[1] = 0x08090A0B0C0D0E0F;
                s[2] = 0xF0E0_D0C0_B0A0_9080;
                s[3] = 0x7FFF_FFFF_FFFF_FFFE;
                for (j, lane) in s.iter_mut().enumerate().skip(4) {
                    *lane = (j as u64) * 0x1111_1111;
                }
                s
            },
            // §2.2 power map near boundaries.
            core::array::from_fn(|i| {
                if i < 4 {
                    0x00FF_00FF_00FF_00FF
                } else {
                    p_minus_1 - i as u64
                }
            }),
        ];
        // §2.2 L fixed points (0, 255) on the 4 split lanes.
        v.push(core::array::from_fn(|i| if i < 4 { 0 } else { 1 }));
        v.push(core::array::from_fn(|i| {
            if i < 4 {
                0xFFFF_FFFF_FFFF_FFFF % p
            } else {
                i as u64
            }
        }));
        // §2.2: each split lane individually = p−1 / 0xFF-bytes.
        for split in 0..4 {
            v.push(core::array::from_fn(
                |i| if i == split { p_minus_1 } else { 0 },
            ));
            v.push(core::array::from_fn(|i| {
                if i == split {
                    0x00FF_00FF_00FF_00FF
                } else {
                    7
                }
            }));
        }
        // §2.2 single-byte sweep on split lane 0 (each of 8 byte slots).
        for byte in 0..8u64 {
            v.push(core::array::from_fn(|i| {
                if i == 0 {
                    0xA5u64 << (8 * byte)
                } else {
                    3
                }
            }));
        }
        // §2.3 MDS: 16 single-lane impulses (every circulant column).
        for lane in 0..16 {
            v.push(core::array::from_fn(|i| u64::from(i == lane)));
        }
        // §2.2 power-map lane sweep: each non-split lane large, rest 0.
        for lane in 4..16 {
            v.push(core::array::from_fn(
                |i| if i == lane { p_minus_1 } else { 0 },
            ));
        }
        // 256 seeded-xorshift states for broad coverage.
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..256 {
            v.push(core::array::from_fn(|_| xs(&mut seed)));
        }
        v
    }

    fn join(xs: impl IntoIterator<Item = u64>) -> String {
        xs.into_iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn build_fixture() -> String {
        let mut out = String::new();
        out.push_str(
            "# tip5 golden KAT v2 — generated from nockchain_math::tip5::permute (7-round)\n",
        );
        out.push_str(
            "# soundness oracle for C2 tip5-circuit-air; constraints per Tip5 paper \
             (IACR ePrint 2023/107): §2.2 L-table = ((b+1)^3-1) mod 257 (verified), \
             §2.3 circulant MDS, §2.4 round constants, §4.6 canonical decomposition\n",
        );
        out.push_str(&format!("P {}\n", P));
        out.push_str(&format!(
            "LOOKUP {}\n",
            join(LOOKUP_TABLE.iter().map(|&b| b as u64))
        ));
        out.push_str(&format!(
            "ROUND_CONSTANTS {}\n",
            join(ROUND_CONSTANTS.iter().copied())
        ));
        out.push_str(&format!(
            "RC_PRECOMP {}\n",
            join(ROUND_CONSTANTS.iter().map(|&rc| rc_precomp(rc)))
        ));
        out.push_str(&format!(
            "MDS_ROW0 {}\n",
            join(MDS_MATRIX_I64[0].iter().map(|&m| m as u64))
        ));
        let inputs = golden_inputs();
        // include a double-permute vector to catch round-coupling errors
        let mut vectors: Vec<([u64; 16], [u64; 16])> = Vec::new();
        for inp in &inputs {
            let mut s = *inp;
            permute(&mut s);
            vectors.push((*inp, s));
        }
        // §2.1 round iteration / coupling: chained multi-permute KATs
        // (IN = k-times-permuted, OUT = (k+1)-times) on several seeds.
        for &seed_idx in &[2usize, 0, 8] {
            let mut s = inputs[seed_idx];
            for _ in 0..3 {
                permute(&mut s);
            }
            let prev = s;
            permute(&mut s);
            vectors.push((prev, s));
        }
        out.push_str(&format!("NVEC {}\n", vectors.len()));
        for (inp, outp) in &vectors {
            out.push_str(&format!("IN {}\n", join(inp.iter().copied())));
            out.push_str(&format!("OUT {}\n", join(outp.iter().copied())));
        }
        out
    }

    #[test]
    fn golden_kat_frozen_matches_live_permute() {
        // sanity: rc_precomp matches permute's exact r_cons formula
        for i in 0..NUM_ROUNDS {
            for j in 0..STATE_SIZE {
                let rc = ROUND_CONSTANTS[i * STATE_SIZE + j];
                let expect = (((rc as u128) * R) % PRIME_128) as u64;
                assert_eq!(rc_precomp(rc), expect, "rc_precomp != permute r_cons");
            }
        }
        let expected = build_fixture();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../ai-pow-zk/tests/fixtures");
        let path = format!("{dir}/tip5_golden_kat.txt");
        let regen = std::env::var("REGEN_TIP5_KAT").is_ok();
        let on_disk = std::fs::read_to_string(&path);
        if regen || on_disk.is_err() {
            std::fs::create_dir_all(dir).expect("create fixtures dir");
            std::fs::write(&path, &expected).expect("write golden KAT fixture");
            eprintln!("C2.0: wrote golden KAT fixture -> {path}");
        }
        let committed = std::fs::read_to_string(&path).expect("read golden KAT fixture");
        assert_eq!(
            committed, expected,
            "tip5 golden KAT fixture drifted from live permute/constants — \
             the C2 soundness oracle changed; re-validate tip5-circuit-air \
             and run with REGEN_TIP5_KAT=1 only if the change is intended"
        );
    }

    // ===== 5-ROUND VARIANT (ai-pow-zk-specific; 2026-05-20) =====
    //
    // Parallel c2_kat-style validation for the 5-round Tip5 variant
    // [`permute_5round`]. Generates a SEPARATE fixture file
    // `tip5_5round_golden_kat.txt` cross-anchored with the in-workspace
    // `p3-tip5-circuit-air::tip5_spec` 5-round twin.
    //
    // The 7-round and 5-round fixtures coexist:
    //  - `tip5_golden_kat.txt` — canonical Nockchain 7-round, anchored
    //    to `permute` (used by all non-ai-pow-zk Nockchain code).
    //  - `tip5_5round_golden_kat.txt` — ai-pow-zk's 5-round variant,
    //    anchored to `permute_5round` (used by the recursive proving
    //    construction only).
    //
    // Both fixtures share identical input vectors (golden_inputs).

    fn build_fixture_5round() -> String {
        let mut out = String::new();
        out.push_str(
            "# tip5 golden KAT (5-round variant) — generated from \
             nockchain_math::tip5::permute_5round (paper-spec N=5; \
             IACR ePrint 2023/107 §2.4)\n",
        );
        out.push_str(
            "# DIVERGENCE NOTICE: this is the AI-POW-ZK-SPECIFIC \
             5-round variant, NOT the canonical Nockchain 7-round Tip5 \
             (see tip5_golden_kat.txt). Used only by the ai-pow-zk \
             recursive proving construction per maintainer 2026-05-20.\n",
        );
        out.push_str(&format!("P {P}\n"));
        out.push_str(&format!(
            "LOOKUP {}\n",
            join(LOOKUP_TABLE.iter().map(|&b| b as u64))
        ));
        // Only the first 5*STATE_SIZE = 80 round constants are used by
        // permute_5round; emit them explicitly so the fixture stands
        // alone (a 5-round consumer doesn't need to know about the
        // remaining 32 unused entries).
        let used = &ROUND_CONSTANTS[..NUM_ROUNDS_5ROUND * STATE_SIZE];
        out.push_str(&format!("ROUND_CONSTANTS {}\n", join(used.iter().copied())));
        out.push_str(&format!(
            "RC_PRECOMP {}\n",
            join(used.iter().map(|&rc| rc_precomp(rc)))
        ));
        out.push_str(&format!(
            "MDS_ROW0 {}\n",
            join(MDS_MATRIX_I64[0].iter().map(|&m| m as u64))
        ));
        let inputs = golden_inputs();
        let mut vectors: Vec<([u64; 16], [u64; 16])> = Vec::new();
        for inp in &inputs {
            let mut s = *inp;
            permute_5round(&mut s);
            vectors.push((*inp, s));
        }
        // §2.1 round iteration / coupling: chained multi-permute KATs.
        for &seed_idx in &[2usize, 0, 8] {
            let mut s = inputs[seed_idx];
            for _ in 0..3 {
                permute_5round(&mut s);
            }
            let prev = s;
            permute_5round(&mut s);
            vectors.push((prev, s));
        }
        out.push_str(&format!("NVEC {}\n", vectors.len()));
        for (inp, outp) in &vectors {
            out.push_str(&format!("IN {}\n", join(inp.iter().copied())));
            out.push_str(&format!("OUT {}\n", join(outp.iter().copied())));
        }
        out
    }

    #[test]
    fn golden_kat_5round_frozen_matches_live_permute_5round() {
        // sanity: rc_precomp matches permute_5round's exact r_cons formula
        // (only the first 5*STATE_SIZE = 80 constants are used).
        for i in 0..NUM_ROUNDS_5ROUND {
            for j in 0..STATE_SIZE {
                let rc = ROUND_CONSTANTS[i * STATE_SIZE + j];
                let expect = (((rc as u128) * R) % PRIME_128) as u64;
                assert_eq!(
                    rc_precomp(rc),
                    expect,
                    "5-round: rc_precomp != permute_5round r_cons"
                );
            }
        }
        let expected = build_fixture_5round();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../ai-pow-zk/tests/fixtures");
        let path = format!("{dir}/tip5_5round_golden_kat.txt");
        let regen = std::env::var("REGEN_TIP5_KAT_5ROUND").is_ok();
        let on_disk = std::fs::read_to_string(&path);
        if regen || on_disk.is_err() {
            std::fs::create_dir_all(dir).expect("create fixtures dir");
            std::fs::write(&path, &expected).expect("write 5-round golden KAT fixture");
            eprintln!("5-round KAT: wrote fixture -> {path}");
        }
        let committed = std::fs::read_to_string(&path).expect("read 5-round golden KAT fixture");
        assert_eq!(
            committed, expected,
            "5-round tip5 golden KAT fixture drifted from live permute_5round/constants — \
             the ai-pow-zk-specific 5-round Tip5 oracle changed; re-validate \
             tip5-circuit-air 5-round AIR and run with REGEN_TIP5_KAT_5ROUND=1 only if \
             the change is intended"
        );
    }
}
