/** Sorting jets
 */
use crate::interpreter::Context;
use crate::jets;
use crate::jets::util::slot_with_arena;
use crate::noun::Noun;

crate::gdb!();

pub fn jet_dor(context: &mut Context, subject: Noun) -> jets::Result {
    let arena = &*context.arena;
    let sam = slot_with_arena(subject, 6, arena)?;
    let a = slot_with_arena(sam, 2, arena)?;
    let b = slot_with_arena(sam, 3, arena)?;

    Ok(util::dor(&mut context.stack, arena, a, b))
}

pub fn jet_gor(context: &mut Context, subject: Noun) -> jets::Result {
    let arena = &*context.arena;
    let sam = slot_with_arena(subject, 6, arena)?;
    let a = slot_with_arena(sam, 2, arena)?;
    let b = slot_with_arena(sam, 3, arena)?;

    Ok(util::gor(&mut context.stack, arena, a, b))
}

pub fn jet_mor(context: &mut Context, subject: Noun) -> jets::Result {
    let arena = &*context.arena;
    let sam = slot_with_arena(subject, 6, arena)?;
    let a = slot_with_arena(sam, 2, arena)?;
    let b = slot_with_arena(sam, 3, arena)?;

    Ok(util::mor(&mut context.stack, arena, a, b))
}

pub mod util {
    use std::cmp::Ordering;

    use either::{Left, Right};

    use crate::jets::math::util::lth;
    use crate::jets::util::slot_with_arena;
    use crate::mem::{Arena, NockStack};
    use crate::mug::mug;
    use crate::noun::{Noun, NO, YES};

    pub fn dor(stack: &mut NockStack, arena: &Arena, a: Noun, b: Noun) -> Noun {
        if unsafe { a.raw_equals(&b) } {
            YES
        } else {
            match (a.as_either_atom_cell(), b.as_either_atom_cell()) {
                (Left(atom_a), Left(atom_b)) => lth(stack, arena, atom_a, atom_b),
                (Left(_), Right(_)) => YES,
                (Right(_), Left(_)) => NO,
                (Right(cell_a), Right(cell_b)) => {
                    let a_head = match slot_with_arena(cell_a.as_noun(), 2, arena) {
                        Ok(n) => n,
                        Err(_) => return NO,
                    };
                    let b_head = slot_with_arena(cell_b.as_noun(), 2, arena).unwrap_or_else(|err| {
                        panic!(
                            "Panicked with {err:?} at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        )
                    });
                    let a_tail = slot_with_arena(cell_a.as_noun(), 3, arena).unwrap_or_else(|err| {
                        panic!(
                            "Panicked with {err:?} at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        )
                    });
                    let b_tail = slot_with_arena(cell_b.as_noun(), 3, arena).unwrap_or_else(|err| {
                        panic!(
                            "Panicked with {err:?} at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        )
                    });
                    if unsafe { a_head.raw_equals(&b_head) } {
                        dor(stack, arena, a_tail, b_tail)
                    } else {
                        dor(stack, arena, a_head, b_head)
                    }
                }
            }
        }
    }

    pub fn gor(stack: &mut NockStack, arena: &Arena, a: Noun, b: Noun) -> Noun {
        let c = mug(stack, a);
        let d = mug(stack, b);

        match c.data().cmp(&d.data()) {
            Ordering::Greater => NO,
            Ordering::Less => YES,
            Ordering::Equal => dor(stack, arena, a, b),
        }
    }

    pub fn mor(stack: &mut NockStack, arena: &Arena, a: Noun, b: Noun) -> Noun {
        let c = mug(stack, a);
        let d = mug(stack, b);

        let e = mug(stack, c.as_noun());
        let f = mug(stack, d.as_noun());

        match e.data().cmp(&f.data()) {
            Ordering::Greater => NO,
            Ordering::Less => YES,
            Ordering::Equal => dor(stack, arena, a, b),
        }
    }
}

#[cfg(test)]
mod tests {
    use ibig::ubig;

    use super::*;
    use crate::jets::util::test::{assert_jet, init_context, A};
    use crate::noun::{D, NO, T, YES};

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_dor() {
        let c = &mut init_context();

        let sam = T(&mut c.stack, &[D(1), D(1)]);
        assert_jet(c, jet_dor, sam, YES);

        let a = A(&mut c.stack, &ubig!(_0x3fffffffffffffff));
        let sam = T(&mut c.stack, &[a, D(1)]);
        assert_jet(c, jet_dor, sam, NO);

        let a = A(&mut c.stack, &ubig!(_0x3fffffffffffffff));
        let sam = T(&mut c.stack, &[a, a]);
        assert_jet(c, jet_dor, sam, YES);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_gor() {
        let c = &mut init_context();

        let sam = T(&mut c.stack, &[D(1), D(1)]);
        assert_jet(c, jet_gor, sam, YES);

        let a = A(&mut c.stack, &ubig!(_0x3fffffffffffffff));
        let sam = T(&mut c.stack, &[a, a]);
        assert_jet(c, jet_gor, sam, YES);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_mor() {
        let c = &mut init_context();

        let sam = T(&mut c.stack, &[D(1), D(1)]);
        assert_jet(c, jet_mor, sam, YES);

        let a = A(&mut c.stack, &ubig!(_0x3fffffffffffffff));
        let sam = T(&mut c.stack, &[a, a]);
        assert_jet(c, jet_mor, sam, YES);
    }
}
