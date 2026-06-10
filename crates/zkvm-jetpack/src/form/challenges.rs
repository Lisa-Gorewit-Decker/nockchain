use nockvm::jets::util::BAIL_FAIL;
use nockvm::jets::JetErr;
use nockvm::noun::{Noun, NounSpace};

use crate::form::belt::Belt;
use crate::form::felt::{fadd_, finv_, fmul_, Felt};
use crate::form::math::gen_trace::build_tree_data;

pub const NUM_EXT_CHALS: u32 = 42;
pub const NUM_MEGA_EXT_CHALS: u32 = 36;

fn init_ext_chals_from_belts(chals: &[Belt]) -> Result<crate::jets::table_utils::ExtChals, JetErr> {
    let mut felts = Vec::<Felt>::with_capacity(14);
    for trip in chals.chunks(3) {
        felts.push(Felt::try_from(trip).map_err(|_| BAIL_FAIL)?);
    }
    Ok(crate::jets::table_utils::ExtChals {
        a: felts[0],
        b: felts[1],
        c: felts[2],
        _d: felts[3],
        _e: felts[4],
        _f: felts[5],
        _g: felts[6],
        _p: felts[7],
        _q: felts[8],
        _r: felts[9],
        _s: felts[10],
        _t: felts[11],
        _u: felts[12],
        alf: felts[13],
    })
}

pub fn augment_challenges(
    chals: &mut Vec<Belt>,
    subject: Noun,
    space: &NounSpace,
) -> Result<(), JetErr> {
    let ext_chals = init_ext_chals_from_belts(&chals[0..NUM_EXT_CHALS as usize])?;
    let inv_alf = finv_(&ext_chals.alf);
    let subject_tree = build_tree_data(subject, &ext_chals.alf, space)?;
    let input_ifp = fadd_(
        &fmul_(&ext_chals.a, &subject_tree.size),
        &fadd_(
            &fmul_(&ext_chals.b, &subject_tree.dyck),
            &fmul_(&ext_chals.c, &subject_tree.leaf),
        ),
    );

    chals.push(inv_alf[0]);
    chals.push(inv_alf[1]);
    chals.push(inv_alf[2]);
    chals.push(input_ifp[0]);
    chals.push(input_ifp[1]);
    chals.push(input_ifp[2]);
    Ok(())
}
