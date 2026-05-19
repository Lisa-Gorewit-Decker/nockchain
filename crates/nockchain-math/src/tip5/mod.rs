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

fn linear_layer(state: &[u64; 16]) -> [u64; 16] {
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
            assert!(want < 256, "L({b})={want} not a byte — table type is [u8;256]");
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
    /// per the authoritative Tip5 paper (IACR ePrint 2023/107,
    /// `2023-107.pdf`):
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
            core::array::from_fn(|i| if i < 4 { 0x00FF_00FF_00FF_00FF } else { p_minus_1 - i as u64 }),
        ];
        // §2.2 L fixed points (0, 255) on the 4 split lanes.
        v.push(core::array::from_fn(|i| if i < 4 { 0 } else { 1 }));
        v.push(core::array::from_fn(|i| {
            if i < 4 { 0xFFFF_FFFF_FFFF_FFFF % p } else { i as u64 }
        }));
        // §2.2: each split lane individually = p−1 / 0xFF-bytes.
        for split in 0..4 {
            v.push(core::array::from_fn(|i| if i == split { p_minus_1 } else { 0 }));
            v.push(core::array::from_fn(|i| if i == split { 0x00FF_00FF_00FF_00FF } else { 7 }));
        }
        // §2.2 single-byte sweep on split lane 0 (each of 8 byte slots).
        for byte in 0..8u64 {
            v.push(core::array::from_fn(|i| if i == 0 { 0xA5u64 << (8 * byte) } else { 3 }));
        }
        // §2.3 MDS: 16 single-lane impulses (every circulant column).
        for lane in 0..16 {
            v.push(core::array::from_fn(|i| u64::from(i == lane)));
        }
        // §2.2 power-map lane sweep: each non-split lane large, rest 0.
        for lane in 4..16 {
            v.push(core::array::from_fn(|i| if i == lane { p_minus_1 } else { 0 }));
        }
        // 256 seeded-xorshift states for broad coverage.
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..256 {
            v.push(core::array::from_fn(|_| xs(&mut seed)));
        }
        v
    }

    fn join(xs: impl IntoIterator<Item = u64>) -> String {
        xs.into_iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ")
    }

    fn build_fixture() -> String {
        let mut out = String::new();
        out.push_str("# tip5 golden KAT v2 — generated from nockchain_math::tip5::permute (7-round)\n");
        out.push_str(
            "# soundness oracle for C2 tip5-circuit-air; constraints per Tip5 paper \
             (IACR ePrint 2023/107): §2.2 L-table = ((b+1)^3-1) mod 257 (verified), \
             §2.3 circulant MDS, §2.4 round constants, §4.6 canonical decomposition\n",
        );
        out.push_str(&format!("P {}\n", P));
        out.push_str(&format!("LOOKUP {}\n", join(LOOKUP_TABLE.iter().map(|&b| b as u64))));
        out.push_str(&format!("ROUND_CONSTANTS {}\n", join(ROUND_CONSTANTS.iter().copied())));
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
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ai-pow-zk/tests/fixtures"
        );
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
}
