use nockvm::jets::list::util::flop;
use nockvm::jets::JetErr;
use nockvm::mem::Arena;
use nockvm::noun::{Noun, NounAllocator, D, T};
use noun_serde::NounEncode;

pub fn dyck<A: NounAllocator>(stack: &mut A, t: Noun, arena: &Arena) -> Result<Noun, JetErr> {
    let vec = dyck_recursive(stack, t, D(0), arena)?;
    flop(stack, vec, arena)
}

fn dyck_recursive<A: NounAllocator>(stack: &mut A, t: Noun, vec: Noun, arena: &Arena) -> Result<Noun, JetErr> {
    if t.is_atom() {
        Ok(vec)
    } else {
        let t_cell = t.as_cell()?;
        let vec_inner = T(stack, &[D(0), vec]);
        let dyck_inner = dyck_recursive(stack, t_cell.head_with_arena(arena), vec_inner, arena)?;
        let vec_outer = T(stack, &[D(1), dyck_inner]);
        dyck_recursive(stack, t_cell.tail_with_arena(arena), vec_outer, arena)
    }
}

pub fn leaf_sequence<A: NounAllocator>(stack: &mut A, t: Noun, arena: &Arena) -> Result<Noun, JetErr> {
    let mut leaf: Vec<u64> = Vec::<u64>::new();
    do_leaf_sequence(t, &mut leaf, arena)?;
    let res = leaf.to_noun(stack);
    Ok(res)
}

pub fn do_leaf_sequence(noun: Noun, vec: &mut Vec<u64>, arena: &Arena) -> Result<(), JetErr> {
    if noun.is_atom() {
        vec.push(noun.as_atom()?.as_u64()?);
        Ok(())
    } else {
        let cell = noun.as_cell()?;
        do_leaf_sequence(cell.head_with_arena(arena), vec, arena)?;
        do_leaf_sequence(cell.tail_with_arena(arena), vec, arena)?;
        Ok(())
    }
}
