//! In-crate, native-faithful **5-round Tip5** reference for the
//! ai-pow-zk recursive proving construction (per maintainer
//! 2026-05-20).
//!
//! This is the bit-for-bit twin of `nockchain_math::tip5::permute_5round`
//! (the two cannot share code — they live in separate, excluded Cargo
//! workspaces). Faithfulness is *closed* by the committed golden KAT
//! fixture `tip5_5round_golden_kat.txt`: `nockchain-math` proves the
//! fixture matches its live `permute_5round`; here `tests::*` proves
//! these embedded constants and this `permute` match that *same
//! committed fixture* bit-for-bit. Cross-workspace soundness loop
//! closed without a code dependency.
//!
//! **DIVERGENCE FROM CANONICAL NOCKCHAIN TIP5:** Nockchain's
//! canonical hash (`nockchain_math::tip5::permute`) uses 7 rounds
//! as a defensive cushion above the Tip5 paper's spec (N=5; IACR
//! ePrint 2023/107 §2.4). This ai-pow-zk-specific 5-round variant
//! trades that 2-round cushion for proof-size reduction in the
//! in-circuit Tip5 AIR (~−25-30% AIR width). The cryptanalysis
//! (IACR ePrint 2024/1900 "Opening the Blackbox": practical
//! 3-round attacks) supports 5 rounds as paper-spec-secure with
//! no extra margin. Maintainer-approved divergence per goal
//! 2026-05-20.
//!
//! All arithmetic is plain canonical-domain Goldilocks (`mod p`),
//! exactly as `belt::{bmul,badd,bpow}` (which use `reduce`/`reduce_159`,
//! *not* Montgomery) and `permute`'s `r_cons = (RC·2^64) mod p`.

/// Goldilocks prime `p = 2^64 − 2^32 + 1` (`belt::PRIME`).
pub const P_GOLDILOCKS: u64 = 0xffff_ffff_0000_0001;
/// Tip5 state width.
pub const STATE_SIZE: usize = 16;
/// **5-round Tip5 (paper-spec IACR ePrint 2023/107 §2.4 N=5).**
/// ai-pow-zk-specific divergence from canonical Nockchain
/// `nockchain_math::tip5::NUM_ROUNDS=7`; bit-for-bit twin of
/// `nockchain_math::tip5::NUM_ROUNDS_5ROUND=5`.
pub const NUM_ROUNDS: usize = 5;
/// Split-and-lookup S-box lanes (the rest use the x^7 power map).
pub const NUM_SPLIT_AND_LOOKUP: usize = 4;

const P: u128 = P_GOLDILOCKS as u128;
/// `R = 2^64`, the Montgomery factor applied to round constants in
/// `nockchain_math::tip5::permute` (`(RC as u128 * R) % PRIME_128`).
const R: u128 = 1u128 << 64;

/// The split-and-lookup table `L` (`nockchain_math::tip5::LOOKUP_TABLE`).
/// C2.0 proves `LOOKUP_TABLE[b] == ((b+1)^3 − 1) mod 257 ∀ b`.
pub const LOOKUP_TABLE: [u8; 256] = [
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

/// **First 5·16 = 80** round constants from
/// `nockchain_math::tip5::ROUND_CONSTANTS` (raw; row-major).
/// The 5-round Tip5 variant uses ONLY these 80 entries; the
/// remaining 32 entries from the canonical 7-round array are
/// intentionally absent here.
pub const ROUND_CONSTANTS: [u64; NUM_ROUNDS * STATE_SIZE] = [
    1332676891236936200, 16607633045354064669, 12746538998793080786, 15240351333789289931,
    10333439796058208418, 986873372968378050, 153505017314310505, 703086547770691416,
    8522628845961587962, 1727254290898686320, 199492491401196126, 2969174933639985366,
    1607536590362293391, 16971515075282501568, 15401316942841283351, 14178982151025681389,
    2916963588744282587, 5474267501391258599, 5350367839445462659, 7436373192934779388,
    12563531800071493891, 12265318129758141428, 6524649031155262053, 1388069597090660214,
    3049665785814990091, 5225141380721656276, 10399487208361035835, 6576713996114457203,
    12913805829885867278, 10299910245954679423, 12980779960345402499, 593670858850716490,
    12184128243723146967, 1315341360419235257, 9107195871057030023, 4354141752578294067,
    8824457881527486794, 14811586928506712910, 7768837314956434138, 2807636171572954860,
    9487703495117094125, 13452575580428891895, 14689488045617615844, 16144091782672017853,
    15471922440568867245, 17295382518415944107, 15054306047726632486, 5708955503115886019,
    9596017237020520842, 16520851172964236909, 8513472793890943175, 8503326067026609602,
    9402483918549940854, 8614816312698982446, 7744830563717871780, 14419404818700162041,
    8090742384565069824, 15547662568163517559, 17314710073626307254, 10008393716631058961,
    14480243402290327574, 13569194973291808551, 10573516815088946209, 15120483436559336219,
    3515151310595301563, 1095382462248757907, 5323307938514209350, 14204542692543834582,
    12448773944668684656, 13967843398310696452, 14838288394107326806, 13718313940616442191,
    15032565440414177483, 13769903572116157488, 17074377440395071208, 16931086385239297738,
    8723550055169003617, 590842605971518043, 16642348030861036090, 10708719298241282592,
];

/// First row of the circulant MDS matrix
/// (`nockchain_math::tip5::MDS_MATRIX_I64[0]`). Row `i` is this row
/// rotated right by `i`: `M[i][j] = MDS_FIRST_ROW[(j + 16 − i) % 16]`.
pub const MDS_FIRST_ROW: [u64; STATE_SIZE] = [
    61402, 17845, 26798, 59689, 12021, 40901, 41351, 27521, 56951, 12034, 53865, 43244, 7454,
    33823, 28750, 1108,
];

/// The full circulant MDS matrix derived from [`MDS_FIRST_ROW`].
pub fn mds_matrix() -> [[u64; STATE_SIZE]; STATE_SIZE] {
    let mut m = [[0u64; STATE_SIZE]; STATE_SIZE];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = MDS_FIRST_ROW[(j + STATE_SIZE - i) % STATE_SIZE];
        }
    }
    m
}

/// `((RC as u128) * 2^64) % p` — the per-(round,lane) constant the AIR
/// embeds (exactly `permute`'s `r_cons`).
pub const fn rc_precomp(rc_raw: u64) -> u64 {
    (((rc_raw as u128) * R) % P) as u64
}

#[inline]
fn badd(a: u64, b: u64) -> u64 {
    (((a as u128) + (b as u128)) % P) as u64
}

#[inline]
fn bmul(a: u64, b: u64) -> u64 {
    (((a as u128) * (b as u128)) % P) as u64
}

#[inline]
fn bpow7(a: u64) -> u64 {
    let a2 = bmul(a, a);
    let a3 = bmul(a2, a);
    bmul(bmul(a3, a3), a) // a^7 = a^3 · a^3 · a
}

fn sbox_layer(state: &[u64; STATE_SIZE]) -> [u64; STATE_SIZE] {
    let mut res = [0u64; STATE_SIZE];
    for i in 0..NUM_SPLIT_AND_LOOKUP {
        let mut bytes = state[i].to_le_bytes();
        for byte in &mut bytes {
            *byte = LOOKUP_TABLE[*byte as usize];
        }
        res[i] = u64::from_le_bytes(bytes);
    }
    for j in NUM_SPLIT_AND_LOOKUP..STATE_SIZE {
        res[j] = bpow7(state[j]);
    }
    res
}

fn linear_layer(state: &[u64; STATE_SIZE]) -> [u64; STATE_SIZE] {
    let m = mds_matrix();
    let mut result = [0u64; STATE_SIZE];
    for i in 0..STATE_SIZE {
        let mut acc = 0u64;
        for j in 0..STATE_SIZE {
            acc = badd(acc, bmul(m[i][j], state[j]));
        }
        result[i] = acc;
    }
    result
}

/// The **5-round** Tip5 permutation — bit-for-bit
/// `nockchain_math::tip5::permute_5round` (the ai-pow-zk-specific
/// paper-spec variant; NOT the canonical Nockchain 7-round
/// `permute`).
pub fn permute(sponge: &mut [u64; STATE_SIZE]) {
    for i in 0..NUM_ROUNDS {
        let a = sbox_layer(sponge);
        let b = linear_layer(&a);
        for j in 0..STATE_SIZE {
            let rc = rc_precomp(ROUND_CONSTANTS[i * STATE_SIZE + j]);
            sponge[j] = badd(rc, b[j]);
        }
    }
}
