use nockchain_math::belt::Belt;
use nockchain_math::mary::MarySlice;
use nockchain_math::poly::{BPolyVec, FPolyVec};
use nockchain_math::tip5::hash::{hash_10, hash_varlen};
use nockchain_math::tip5::{DIGEST_LENGTH, RATE};

#[derive(Clone, Debug)]
pub struct MerkData {
    pub leaf: [u64; DIGEST_LENGTH],
    pub axis: u64,
    pub root: [u64; DIGEST_LENGTH],
    pub path: Vec<[u64; DIGEST_LENGTH]>,
}

fn belts_to_u64s(values: &[Belt]) -> Vec<u64> {
    values.iter().map(|value| value.0).collect()
}

fn felt_to_u64s(values: &[nockchain_math::felt::Felt]) -> Vec<u64> {
    values
        .iter()
        .flat_map(|value| value.0.map(|belt| belt.0))
        .collect()
}

fn hash_u64(value: u64) -> [u64; DIGEST_LENGTH] {
    hash_varlen(&mut vec![Belt(1), Belt(value)])
}

fn hash_ten_cell(input: [u64; RATE]) -> [u64; DIGEST_LENGTH] {
    let mut belts = input.into_iter().map(Belt).collect::<Vec<_>>();
    hash_10(&mut belts)
}

fn hash_belts_list(input: &[u64]) -> [u64; DIGEST_LENGTH] {
    let mut belts = input.iter().copied().map(Belt).collect::<Vec<_>>();
    hash_varlen(&mut belts)
}

fn hash_mary(mary: MarySlice<'_>) -> [u64; DIGEST_LENGTH] {
    let step_hash = hash_u64(mary.step as u64);
    let len_hash = hash_u64(mary.len as u64);
    let dat_hash = hash_belts_list(mary.dat);

    let mut len_dat_ten_cell = [0u64; RATE];
    len_dat_ten_cell[..DIGEST_LENGTH].copy_from_slice(&len_hash);
    len_dat_ten_cell[DIGEST_LENGTH..].copy_from_slice(&dat_hash);
    let len_dat_hash = hash_ten_cell(len_dat_ten_cell);

    let mut step_len_ten_cell = [0u64; RATE];
    step_len_ten_cell[..DIGEST_LENGTH].copy_from_slice(&step_hash);
    step_len_ten_cell[DIGEST_LENGTH..].copy_from_slice(&len_dat_hash);
    hash_ten_cell(step_len_ten_cell)
}

pub fn hash_bpoly_leaf(bpoly: &BPolyVec) -> [u64; DIGEST_LENGTH] {
    let belts = belts_to_u64s(bpoly.as_slice());
    let mary = MarySlice {
        step: 1,
        len: bpoly.0.len() as u32,
        dat: &belts,
    };
    hash_mary(mary)
}

pub fn hash_fpoly_leaf(fpoly: &FPolyVec) -> [u64; DIGEST_LENGTH] {
    let belts = felt_to_u64s(fpoly.as_slice());
    let mary = MarySlice {
        step: 3,
        len: fpoly.0.len() as u32,
        dat: &belts,
    };
    hash_mary(mary)
}

pub fn verify_merk_proof(
    leaf: [u64; DIGEST_LENGTH],
    axis: u64,
    root: [u64; DIGEST_LENGTH],
    path: &[[u64; DIGEST_LENGTH]],
) -> bool {
    if axis == 0 {
        return false;
    }

    let mut current = leaf;
    let mut axis = axis;
    let mut iter = path.iter();
    loop {
        if axis == 1 {
            return iter.next().is_none() && current == root;
        }

        let sibling = match iter.next() {
            Some(sibling) => *sibling,
            None => return false,
        };
        let mut input = [0u64; RATE];
        if axis.is_multiple_of(2) {
            input[..DIGEST_LENGTH].copy_from_slice(&current);
            input[DIGEST_LENGTH..].copy_from_slice(&sibling);
            axis /= 2;
        } else {
            input[..DIGEST_LENGTH].copy_from_slice(&sibling);
            input[DIGEST_LENGTH..].copy_from_slice(&current);
            axis = (axis - 1) / 2;
        }
        current = hash_ten_cell(input);
    }
}
