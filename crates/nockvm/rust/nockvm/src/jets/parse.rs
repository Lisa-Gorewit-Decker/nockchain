use either::{Left, Right};

/** Parsing jets
 */
use crate::interpreter::Context;
use crate::jets::bits::util::met;
use crate::jets::math::util::{gte_b, lte_b, lth_b};
use crate::jets::util::{kick, slam, slot, slot_with_arena, BAIL_FAIL};
use crate::jets::Result;
use crate::noun::{Cell, Noun, D, T};

crate::gdb!();

//
//  Text conversion
//
pub fn jet_trip(context: &mut Context, subject: Noun) -> Result {
    let arena = &*context.arena;
    let sam = slot_with_arena(subject, 6, arena)?.as_atom()?;
    let chars = met(3, sam);
    if chars == 0 {
        return Ok(D(0));
    };

    let bytes = &sam.as_ne_bytes()[0..chars];

    let mut result = D(0);
    let mut dest = &mut result as *mut Noun;

    for byte in bytes {
        unsafe {
            let (it, it_mem) = Cell::new_raw_mut(&mut context.stack);

            // safe because a byte can't overflow a direct atom
            (*it_mem).head = D((*byte) as u64);

            *dest = it.as_noun();
            dest = &mut (*it_mem).tail as *mut Noun;
        }
    }
    unsafe { *dest = D(0) };
    Ok(result)
}

//
//  Tracing
//

pub fn jet_last(context: &mut Context, subject: Noun) -> Result {
    let arena = &*context.arena;
    let sam = slot(subject, 6)?;
    let zyc = slot(sam, 2)?;
    let naz = slot(sam, 3)?;

    util::last(zyc, naz, &arena)
}

//
//  Combinators
//

pub fn jet_bend(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let sam = slot(subject, 6)?;
    let vex = slot(sam, 2)?.as_cell()?;
    let sab = slot(sam, 3)?;
    let van = slot(subject, 7)?;
    let raq = slot(van, 6)?;

    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        return Ok(vex.as_noun());
    }

    let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
    let puq_vex = uq_vex.head_with_arena(&arena);
    let quq_vex = uq_vex.tail_with_arena(&arena);

    let yit = slam(context, sab, quq_vex)?.as_cell()?;
    let p_yit = yit.head_with_arena(&arena);
    let q_yit = yit.tail_with_arena(&arena);

    let yur = util::last(p_vex, p_yit, &arena)?;

    if unsafe { q_yit.raw_equals(&D(0)) } {
        Ok(T(&mut context.stack, &[yur, q_vex]))
    } else {
        let uq_yit = q_yit.as_cell()?.tail_with_arena(&arena).as_cell()?;
        let puq_yit = uq_yit.head_with_arena(&arena);
        let quq_yit = uq_yit.tail_with_arena(&arena);

        let arg = T(&mut context.stack, &[puq_vex, puq_yit]);
        let vux = slam(context, raq, arg)?;

        if unsafe { vux.raw_equals(&D(0)) } {
            Ok(T(&mut context.stack, &[yur, q_vex]))
        } else {
            let q_vux = vux.as_cell()?.tail_with_arena(&arena);
            Ok(T(&mut context.stack, &[yur, D(0), q_vux, quq_yit]))
        }
    }
}

pub fn jet_comp(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let sam = slot(subject, 6)?;
    let vex = slot(sam, 2)?.as_cell()?;
    let sab = slot(sam, 3)?;
    let van = slot(subject, 7)?;
    let raq = slot(van, 6)?;

    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        return Ok(vex.as_noun());
    }

    let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
    let puq_vex = uq_vex.head_with_arena(&arena);
    let quq_vex = uq_vex.tail_with_arena(&arena);

    let yit = slam(context, sab, quq_vex)?.as_cell()?;
    let p_yit = yit.head_with_arena(&arena);
    let q_yit = yit.tail_with_arena(&arena);

    let yur = util::last(p_vex, p_yit, &arena)?;

    if unsafe { q_yit.raw_equals(&D(0)) } {
        Ok(T(&mut context.stack, &[yur, D(0)]))
    } else {
        let uq_yit = q_yit.as_cell()?.tail_with_arena(&arena).as_cell()?;
        let puq_yit = uq_yit.head_with_arena(&arena);
        let quq_yit = uq_yit.tail_with_arena(&arena);

        let arg = T(&mut context.stack, &[puq_vex, puq_yit]);
        let vux = slam(context, raq, arg)?;
        Ok(T(&mut context.stack, &[yur, D(0), vux, quq_yit]))
    }
}

pub fn jet_glue(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let sam = slot(subject, 6)?;
    let vex = slot(sam, 2)?.as_cell()?;
    let sab = slot(sam, 3)?;
    let van = slot(subject, 7)?;
    let bus = slot(van, 6)?;

    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        return Ok(vex.as_noun());
    }

    let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
    let puq_vex = uq_vex.head_with_arena(&arena);
    let quq_vex = uq_vex.tail_with_arena(&arena);

    let yit = slam(context, bus, quq_vex)?.as_cell()?;
    let p_yit = yit.head_with_arena(&arena);
    let q_yit = yit.tail_with_arena(&arena);

    let yur = util::last(p_vex, p_yit, &arena)?;

    if unsafe { q_yit.raw_equals(&D(0)) } {
        Ok(T(&mut context.stack, &[yur, D(0)]))
    } else {
        let uq_yit = q_yit.as_cell()?.tail_with_arena(&arena).as_cell()?;
        let quq_yit = uq_yit.tail_with_arena(&arena);

        let wam = slam(context, sab, quq_yit)?.as_cell()?;
        let p_wam = wam.head_with_arena(&arena);
        let q_wam = wam.tail_with_arena(&arena);

        let goy = util::last(yur, p_wam, &arena)?;

        if unsafe { q_wam.raw_equals(&D(0)) } {
            Ok(T(&mut context.stack, &[goy, D(0)]))
        } else {
            let uq_wam = q_wam.as_cell()?.tail_with_arena(&arena).as_cell()?;
            let puq_wam = uq_wam.head_with_arena(&arena);
            let quq_wam = uq_wam.tail_with_arena(&arena);

            let puq_arg = T(&mut context.stack, &[puq_vex, puq_wam]);
            Ok(T(&mut context.stack, &[goy, D(0x0), puq_arg, quq_wam]))
        }
    }
}

pub fn jet_pfix(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let sam = slot(subject, 6)?;
    let vex = slot(sam, 2)?.as_cell()?;
    let sab = slot(sam, 3)?;

    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        return Ok(vex.as_noun());
    }

    let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
    let quq_vex = uq_vex.tail_with_arena(&arena);

    let yit = slam(context, sab, quq_vex)?.as_cell()?;

    let p_yit = yit.head_with_arena(&arena);
    let q_yit = yit.tail_with_arena(&arena);

    //  XX: Why don't we just return yit? When would p_vex ever be the later of the two?
    let arg = util::last(p_vex, p_yit, &arena)?;
    Ok(T(&mut context.stack, &[arg, q_yit]))
}

pub fn jet_plug(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let vex = slot(subject, 12)?.as_cell()?;
    let sab = slot(subject, 13)?;
    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        Ok(vex.as_noun())
    } else {
        let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
        let puq_vex = uq_vex.head_with_arena(&arena);
        let quq_vex = uq_vex.tail_with_arena(&arena);

        let yit = slam(context, sab, quq_vex)?.as_cell()?;
        let p_yit = yit.head_with_arena(&arena);
        let q_yit = yit.tail_with_arena(&arena);

        let yur = util::last(p_vex, p_yit, &arena)?;

        if unsafe { q_yit.raw_equals(&D(0)) } {
            Ok(T(&mut context.stack, &[yur, D(0)]))
        } else {
            let uq_yit = q_yit.as_cell()?.tail_with_arena(&arena).as_cell()?;
            let puq_yit = uq_yit.head_with_arena(&arena);
            let quq_yit = uq_yit.tail_with_arena(&arena);

            let inner = T(&mut context.stack, &[puq_vex, puq_yit]);
            Ok(T(&mut context.stack, &[yur, D(0), inner, quq_yit]))
        }
    }
}

pub fn jet_pose(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let vex = slot(subject, 12)?.as_cell()?;
    let sab = slot(subject, 13)?;

    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { !q_vex.raw_equals(&D(0)) } {
        return Ok(vex.as_noun());
    }

    let roq = kick(context, sab, D(2))?.as_cell()?;
    let yur = util::last(p_vex, roq.head_with_arena(&arena), &arena)?;
    Ok(T(&mut context.stack, &[yur, roq.tail_with_arena(&arena)]))
}

pub fn jet_sfix(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let sam = slot(subject, 6)?;
    let vex = slot(sam, 2)?.as_cell()?;
    let sab = slot(sam, 3)?;

    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        return Ok(vex.as_noun());
    }

    let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
    let puq_vex = uq_vex.head_with_arena(&arena);
    let quq_vex = uq_vex.tail_with_arena(&arena);

    let yit = slam(context, sab, quq_vex)?.as_cell()?;

    let p_yit = yit.head_with_arena(&arena);
    let q_yit = yit.tail_with_arena(&arena);
    let yur = util::last(p_vex, p_yit, &arena)?;

    if unsafe { q_yit.raw_equals(&D(0)) } {
        Ok(T(&mut context.stack, &[yur, D(0)]))
    } else {
        let uq_yit = q_yit.as_cell()?.tail_with_arena(&arena).as_cell()?;
        let quq_yit = uq_yit.tail_with_arena(&arena);

        Ok(T(&mut context.stack, &[yur, D(0), puq_vex, quq_yit]))
    }
}

//
//  Rule Builders
//

pub fn jet_cold(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?;
    let van = slot(subject, 7)?;
    let cus = slot(van, 12)?;
    let sef = slot(van, 13)?;

    let vex = slam(context, sef, tub)?.as_cell()?;
    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        Ok(vex.as_noun())
    } else {
        let quq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?.tail_with_arena(&arena);

        Ok(T(&mut context.stack, &[p_vex, D(0), cus, quq_vex]))
    }
}

pub fn jet_cook(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?;
    let van = slot(subject, 7)?;
    let poq = slot(van, 12)?;
    let sef = slot(van, 13)?;

    let vex = slam(context, sef, tub)?.as_cell()?;
    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        Ok(vex.as_noun())
    } else {
        let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
        let puq_vex = uq_vex.head_with_arena(&arena);
        let quq_vex = uq_vex.tail_with_arena(&arena);

        let wag = slam(context, poq, puq_vex)?;
        Ok(T(&mut context.stack, &[p_vex, D(0), wag, quq_vex]))
    }
}

pub fn jet_easy(context: &mut Context, subject: Noun) -> Result {
    let arena = &*context.arena;
    let tub = slot(subject, 6)?;
    let van = slot(subject, 7)?;
    let huf = slot(van, 6)?;

    Ok(T(
        &mut context.stack,
        &[tub.as_cell()?.head_with_arena(&arena), D(0), huf, tub],
    ))
}

pub fn jet_here(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?;
    let van = slot(subject, 7)?;
    let hez = slot(van, 12)?;
    let sef = slot(van, 13)?;

    let p_tub = tub.as_cell()?.head_with_arena(&arena);

    let vex = slam(context, sef, tub)?.as_cell()?;
    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    // XX fixes Vere's jet mismatch with Hoon 139.
    if unsafe { q_vex.raw_equals(&D(0)) } {
        return Ok(vex.as_noun());
    }

    let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
    let puq_vex = uq_vex.head_with_arena(&arena);
    let quq_vex = uq_vex.tail_with_arena(&arena);
    let pquq_vex = quq_vex.as_cell()?.head_with_arena(&arena);

    let inner_gud = T(&mut context.stack, &[p_tub, pquq_vex]);
    let gud = T(&mut context.stack, &[inner_gud, puq_vex]);
    let wag = slam(context, hez, gud)?;

    Ok(T(&mut context.stack, &[p_vex, D(0), wag, quq_vex]))
}

pub fn jet_just(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?;
    let van = slot(subject, 7)?;
    let daf = slot(van, 6)?;

    let p_tub = tub.as_cell()?.head_with_arena(&arena);
    let q_tub = tub.as_cell()?.tail_with_arena(&arena);

    if unsafe { q_tub.raw_equals(&D(0)) || !daf.raw_equals(&q_tub.as_cell()?.head_with_arena(&arena)) } {
        util::fail(context, p_tub)
    } else {
        util::next(context, tub, &arena)
    }
}

pub fn jet_mask(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?;
    let van = slot(subject, 7)?;
    let mut bud = slot(van, 6)?;

    let p_tub = tub.as_cell()?.head_with_arena(&arena);
    let q_tub = tub.as_cell()?.tail_with_arena(&arena);

    if unsafe { q_tub.raw_equals(&D(0)) } {
        return util::fail(context, p_tub);
    }

    let iq_tub = q_tub.as_cell()?.head_with_arena(&arena);
    while unsafe { !bud.raw_equals(&D(0)) } {
        let cell = bud.as_cell()?;
        if unsafe { cell.head_with_arena(&arena).raw_equals(&iq_tub) } {
            return util::next(context, tub, &arena);
        }
        bud = cell.tail_with_arena(&arena);
    }
    util::fail(context, p_tub)
}

pub fn jet_shim(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?.as_cell()?;
    let van = slot(subject, 7)?;
    let zep = slot(van, 6)?.as_cell()?;

    let p_tub = tub.head_with_arena(&arena);
    let q_tub = tub.tail_with_arena(&arena);

    if unsafe { q_tub.raw_equals(&D(0)) } {
        util::fail(context, p_tub)
    } else {
        let p_zep = zep.head_with_arena(&arena);
        let q_zep = zep.tail_with_arena(&arena);
        let iq_tub = q_tub.as_cell()?.head_with_arena(&arena);

        if let (Some(p_zep_d), Some(q_zep_d), Some(iq_tub_d)) =
            (p_zep.direct(), q_zep.direct(), iq_tub.direct())
        {
            if (iq_tub_d.data() >= p_zep_d.data()) && (iq_tub_d.data() <= q_zep_d.data()) {
                util::next(context, tub.as_noun(), &arena)
            } else {
                util::fail(context, p_tub)
            }
        } else {
            Err(BAIL_FAIL)
        }
    }
}

pub fn jet_stag(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?;
    let van = slot(subject, 7)?;
    let gob = slot(van, 12)?;
    let sef = slot(van, 13)?;

    let vex = slam(context, sef, tub)?.as_cell()?;
    let p_vex = vex.head_with_arena(&arena);
    let q_vex = vex.tail_with_arena(&arena);

    if unsafe { q_vex.raw_equals(&D(0)) } {
        Ok(vex.as_noun())
    } else {
        let uq_vex = q_vex.as_cell()?.tail_with_arena(&arena).as_cell()?;
        let puq_vex = uq_vex.head_with_arena(&arena);
        let quq_vex = uq_vex.tail_with_arena(&arena);

        let wag = T(&mut context.stack, &[gob, puq_vex]);
        Ok(T(&mut context.stack, &[p_vex, D(0), wag, quq_vex]))
    }
}

pub fn jet_stew(context: &mut Context, subject: Noun) -> Result {
    let arena = std::sync::Arc::clone(&context.arena);
    let tub = slot(subject, 6)?.as_cell()?;
    let con = slot(subject, 7)?;
    let mut hel = slot(con, 2)?;

    let p_tub = tub.head_with_arena(&arena);
    let q_tub = tub.tail_with_arena(&arena);
    if unsafe { q_tub.raw_equals(&D(0)) } {
        return util::fail(context, p_tub);
    }

    let iq_tub = q_tub.as_cell()?.head_with_arena(&arena).as_atom()?;
    if !iq_tub.is_direct() {
        // Character cannot be encoded using 8 bytes = computibilty error
        return Err(BAIL_FAIL);
    }

    loop {
        if unsafe { hel.raw_equals(&D(0)) } {
            return util::fail(context, p_tub);
        } else {
            let n_hel = slot(hel, 2)?.as_cell()?;
            let l_hel = slot(hel, 6)?;
            let r_hel = slot(hel, 7)?;
            let pn_hel = n_hel.head_with_arena(&arena);
            let qn_hel = n_hel.tail_with_arena(&arena);

            let bit = match pn_hel.as_either_atom_cell() {
                Left(atom) => match atom.as_either() {
                    Left(direct) => iq_tub.as_direct()?.data() == direct.data(),
                    Right(_) => {
                        // Character cannot be encoded using 8 bytes = computibilty error
                        return Err(BAIL_FAIL);
                    }
                },
                Right(cell) => {
                    let hpn_hel = cell.head_with_arena(&arena).as_atom()?;
                    let tpn_hel = cell.tail_with_arena(&arena).as_atom()?;

                    match (hpn_hel.as_either(), tpn_hel.as_either()) {
                        (Left(_), Left(_)) => {
                            gte_b(&mut context.stack, iq_tub, hpn_hel)
                                && lte_b(&mut context.stack, iq_tub, tpn_hel)
                        }
                        _ => {
                            // XX: Fixes jet mismatch in Vere
                            // Character cannot be encoded using 8 bytes = computibilty error
                            return Err(BAIL_FAIL);
                        }
                    }
                }
            };

            if bit {
                return slam(context, qn_hel, tub.as_noun());
            } else {
                let wor = match pn_hel.as_either_atom_cell() {
                    Left(atom) => atom,
                    Right(cell) => cell.head_with_arena(&arena).as_atom()?,
                };

                if lth_b(&mut context.stack, iq_tub, wor) {
                    hel = l_hel;
                } else {
                    hel = r_hel;
                }
            }
        }
    }
}

// +$  edge  [p=hair q=(unit [p=* q=nail])]
#[derive(Copy, Clone)]
struct StirPair {
    pub har: Noun, // p.edge
    pub res: Noun, // p.u.q.edge
}

pub fn jet_stir(context: &mut Context, subject: Noun) -> Result {
    unsafe {
        context.with_stack_frame(0, |context| {
            let arena = std::sync::Arc::clone(&context.arena);
            let mut tub = slot(subject, 6)?;
            let van = slot(subject, 7)?;
            let rud = slot(van, 12)?;
            let raq = slot(van, 26)?;
            let fel = slot(van, 27)?;

            // initial accumulator (deconstructed)
            let mut p_wag: Noun;
            let mut puq_wag: Noun;
            let quq_wag: Noun;

            // push incremental, succesful [fel] parse results onto stack
            {
                let vex = slam(context, fel, tub)?.as_cell()?;
                let mut p_vex = vex.head_with_arena(&arena);
                let mut q_vex = vex.tail_with_arena(&arena);
                while !q_vex.raw_equals(&D(0)) {
                    let puq_vex = slot(q_vex, 6)?;
                    let quq_vex = slot(q_vex, 7)?;

                    *(context.stack.push::<StirPair>()) = StirPair {
                        har: p_vex,
                        res: puq_vex,
                    };

                    tub = quq_vex;

                    let vex = slam(context, fel, tub)?.as_cell()?;
                    p_vex = vex.head_with_arena(&arena);
                    q_vex = vex.tail_with_arena(&arena);
                }

                p_wag = p_vex;
                puq_wag = rud;
                quq_wag = tub;
            }

            // unwind the stack, folding parse results into [wag] by way of [raq]
            while !context.stack.stack_is_empty() {
                let par_u = *(context.stack.top::<StirPair>());
                p_wag = util::last(par_u.har, p_wag, &arena)?;
                let sam = T(&mut context.stack, &[par_u.res, puq_wag]);
                puq_wag = slam(context, raq, sam)?;
                context.stack.pop::<StirPair>();
            }

            let res = T(&mut context.stack, &[p_wag, D(0), puq_wag, quq_wag]);
            Ok(res)
        })
    }
}

pub mod util {
    use std::cmp::Ordering;

    use crate::interpreter::{inc, Context};
    use crate::jets::Result;
    use crate::mem::Arena;
    use crate::noun::{Noun, D, T};

    pub fn last(zyc: Noun, naz: Noun, arena: &Arena) -> Result {
        let zyl = zyc.as_cell()?;
        let nal = naz.as_cell()?;

        let p_zyc = zyl.head_with_arena(arena).as_direct()?.data();
        let q_zyc = zyl.tail_with_arena(arena).as_direct()?.data();
        let p_naz = nal.head_with_arena(arena).as_direct()?.data();
        let q_naz = nal.tail_with_arena(arena).as_direct()?.data();

        match p_zyc.cmp(&p_naz) {
            Ordering::Equal => {
                if q_zyc > q_naz {
                    Ok(zyc)
                } else {
                    Ok(naz)
                }
            }
            Ordering::Greater => Ok(zyc),
            Ordering::Less => Ok(naz),
        }
    }

    // Passing Noun and doing Cell check inside next is best to keep jet semantics in sync w/ Hoon.
    pub fn next(context: &mut Context, tub: Noun, arena: &Arena) -> Result {
        let p_tub = tub.as_cell()?.head_with_arena(arena);
        let q_tub = tub.as_cell()?.tail_with_arena(arena);

        if unsafe { q_tub.raw_equals(&D(0)) } {
            return fail(context, p_tub);
        }

        let iq_tub = q_tub.as_cell()?.head_with_arena(arena);
        let tq_tub = q_tub.as_cell()?.tail_with_arena(arena);

        let zac = lust(context, iq_tub, p_tub, arena)?;
        Ok(T(&mut context.stack, &[zac, D(0), iq_tub, zac, tq_tub]))
    }

    // Passing Noun and doing Cell check inside next is best to keep jet semantics in sync w/ Hoon.
    pub fn lust(context: &mut Context, weq: Noun, naz: Noun, arena: &Arena) -> Result {
        let p_naz = naz.as_cell()?.head_with_arena(arena).as_atom()?;
        let q_naz = naz.as_cell()?.tail_with_arena(arena).as_atom()?;

        if unsafe { weq.raw_equals(&D(10)) } {
            let arg = inc(&mut context.stack, p_naz).as_noun();
            Ok(T(&mut context.stack, &[arg, D(1)]))
        } else {
            let arg = inc(&mut context.stack, q_naz).as_noun();
            Ok(T(&mut context.stack, &[p_naz.as_noun(), arg]))
        }
    }

    pub fn fail(context: &mut Context, hair: Noun) -> Result {
        Ok(T(&mut context.stack, &[hair, D(0)]))
    }
}

#[cfg(test)]
mod tests {
    use ibig::ubig;

    use super::*;
    use crate::jets::util::test::*;
    use crate::noun::{D, T};
    use crate::serialization::cue;

    //  XX: need unit tests for:
    //      +last
    //      +bend
    //      +comp
    //      +glue
    //      +pfix
    //      +pose
    //      +sfix
    //      +here
    //      +just
    //      +mask
    //      +stag

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_easy() {
        let c = &mut init_context();

        // ((easy 'a') [[1 1] "abc"])
        //  [[1 1] "abc"]
        let sam_jam = A(&mut c.stack, &ubig!(3205468216717221061))
            .as_atom()
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        let sam = cue(&mut c.stack, sam_jam);
        //  [p=[p=1 q=1] q=[~ [p='a' q=[p=[p=1 q=1] q="abc"]]]]
        let ans_jam = A(&mut c.stack, &ubig!(1720922644868600060465749189))
            .as_atom()
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        let ans = cue(&mut c.stack, ans_jam).unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        let ctx = T(&mut c.stack, &[D(0), D(97), D(0)]);
        assert_jet_door(
            c,
            jet_easy,
            sam.unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            }),
            ctx,
            ans,
        );

        // ((easy %foo) [[1 1] "abc"])
        //  [[1 1] "abc"]
        let sam_jam = A(&mut c.stack, &ubig!(3205468216717221061))
            .as_atom()
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        let sam = cue(&mut c.stack, sam_jam).unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        //  [p=[p=1 q=1] q=[~ [p=%foo q=[p=[p=1 q=1] q="abc"]]]]
        let ans_jam = A(&mut c.stack, &ubig!(3609036366588910247778413036281029))
            .as_atom()
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        let ans = cue(&mut c.stack, ans_jam).unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        let ctx = T(&mut c.stack, &[D(0), D(0x6f6f66), D(0)]);
        assert_jet_door(c, jet_easy, sam, ctx, ans);
    }
}
