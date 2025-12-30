use std::collections::*;
use crate::ast::hoon::*;
use std::cmp;
use std::sync::Arc;
use std::str::FromStr;
use std::hash::{Hash, Hasher, DefaultHasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::cell::Cell;
use std::collections::HashMap;
use num_bigint::BigUint;
use num_traits::{One, Num, FromPrimitive, ToPrimitive};
use num_traits::identities::Zero;
use sha2::{Sha256, Digest};
use bitvec::prelude::*;
use bitvec::vec::BitVec;
use bitvec::slice::BitSlice;
use bitvec::order::Lsb0;
use chumsky::{
    span::Span,
    input::{Stream, ValueInput, Input, StrInput},
    input::MapExtra,
    prelude::*,
};

pub type Err<'src> = extra::Full<Rich<'src, char>, (), ()>;

pub trait ParserExt<'src, O>:
    Parser<'src, &'src str, O, Err<'src>> + Clone + 'src
{
}

impl<'src, O, P> ParserExt<'src, O> for P
where
    P: Parser<'src, &'src str, O, Err<'src>> + Clone + 'src,
{
}

pub fn basal(bas: BaseType) -> Hoon {
    match bas {
        BaseType::Atom(a) => {
            let literal = if a == "da" {
                Atom::Small(year(
                    true,
                    2000,
                    1,
                    1,
                    0,
                    0,
                    0,
                    &Vec::new()
                ))
            } else {
                decimal_to_atom("0".to_string())
            };
            Hoon::Sand(a, Noun::Atom(literal))
        }
        BaseType::Noun => {
            let rock0 = Box::new(Hoon::Rock("$".to_string(), Noun::Atom(Atom::Small(0))));
            let rock1 = Box::new(Hoon::Rock("$".to_string(), Noun::Atom(Atom::Small(1))));
            let rock0_clone = rock0.clone();
            let rock0_clone2 = rock0.clone();
            Hoon::KetLus(Box::new(Hoon::DotTar(rock0, Box::new(Hoon::Pair(rock0_clone, rock1)))), rock0_clone2)
        }
        BaseType::Cell => {
            let noun = Box::new(basal(BaseType::Noun));
            let noun_clone = noun.clone();
            Hoon::Pair(noun, noun_clone)
        }
        BaseType::Flag => {
            let rock0 = Box::new(Hoon::Rock("$".to_string(), Noun::Atom(Atom::Small(0))));
            let rock0_clone = rock0.clone();
            let rock1_clone = rock0.clone();
            Hoon::KetLus(Box::new(Hoon::DotTis(rock0, rock0_clone)), rock1_clone)
        }
        BaseType::Null => Hoon::Rock("$".to_string(), Noun::Atom(Atom::Small(0))),
        BaseType::Void => Hoon::ZapZap,
    }
}

pub fn function(
    fun: Spec,
    arg: Spec,
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<String, Spec>,
    bug: &Vec<Spot>,
    nut: &Option<Note>,
    def: &Option<Hoon>,
) -> Hoon {
    Hoon::TisGar(
        Box::new(Hoon::Pair(
                    Box::new(example(&fun.clone(), dom, hay, cox, &vec![], &None, &None)),
                    Box::new(example(&arg.clone(), dom, hay, cox, &vec![], &None, &None)))),
        Box::new(Hoon::KetBar(Box::new(Hoon::BarCol(Box::new(Hoon::Axis(2)),
                                        Box::new(Hoon::Axis(15)))))),
    )
}

pub fn interface(
    variance: Vair,
    payload: Spec,
    arms: HashMap<String, Spec>,
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<String, Spec>,
    bug: &Vec<Spot>,
    nut: &Option<Note>,
    def: &Option<Hoon>,
) -> Hoon {

    let map: HashMap<String, Hoon> = arms.into_iter()
        .map(|(term, spec)|
                 (term, example(&spec, dom, hay, cox, &vec![], &None, &None)))
        .collect();
    let brcn = Hoon::BarCen(
        None,
        HashMap::from([("$".to_string(), map)]),
    );

    let example_res = example(&payload, dom, hay, cox, &vec![], &None, &None);
    let tsgr = Hoon::TisGar(Box::new(example_res), Box::new(brcn));
    match variance {
        Vair::Gold => tsgr,
        Vair::Lead => Hoon::KetWut(Box::new(tsgr)),
        Vair::Zinc => Hoon::KetPam(Box::new(tsgr)),
        Vair::Iron => Hoon::KetBar(Box::new(tsgr)),
    }
}

// TODO: accept args by ref?
pub fn spore(spec: Spec,
                dom: u64,
                hay: WingType,
                cox: HashMap<String, Spec>,
                bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    let subject = match def {
        Some(d) => d,
        None => spore_recursion(spec, dom, hay, cox, bug, nut, def),
    };
    let ketlus_tail = home(subject, Vec::new(), dom);
    Hoon::KetLus(Box::new(Hoon::Bust(BaseType::Noun)), Box::new(ketlus_tail))
}

pub fn spore_recursion(spec: Spec,
                dom: u64,
                hay: WingType,
                cox: HashMap<String, Spec>,
                bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    match spec {
        Spec::Base(b) => {
            match b {
                BaseType::Void => Hoon::Rock("n".to_string(), Noun::Atom(Atom::Small(0))),
                _ => basal(b),
            }
        }
        Spec::BucBuc(s, map) => {
            let mut new_cox = cox;
            new_cox.extend(map);
            new_cox.insert("$".to_string(), *s.clone());
            spore_recursion(*s, dom, hay, new_cox, bug, nut, def)
        }
        Spec::Dbug(spot, spec) => {
            let tail = spore_recursion(*spec, dom, hay, cox, bug, nut, def);
            Hoon::Dbug(spot, Box::new(tail))
        }
        Spec::Leaf(term, atom) => Hoon::Rock(term, Noun::Atom(atom)),
        Spec::Loop(term) => {
            let spec = cox.get(&term).expect("Spec-Loop: Name not found");
            spore_recursion(spec.clone(), dom, hay, cox, bug, nut, def)
        }
        Spec::Like(wing, wings) => {
            let p = unreel(wing, wings);
            spore_recursion(Spec::BucMic(p), dom, hay, cox, bug, nut, def)
        }
        Spec::Made(_, q) => spore_recursion(*q, dom, hay, cox, bug, nut, def),
        Spec::Make(hoon, specs) => {
            let p = unfold(hoon, specs);
            spore_recursion(Spec::BucMic(p), dom, hay, cox, bug, nut, def)
        }
        Spec::Name(term, spec) => spore_recursion(*spec, dom, hay, cox, bug, nut, def),
        Spec::Over(wing, spec) => spore_recursion(*spec, dom, wing, cox, bug, nut, def),
        Spec::BucBar(spec, hoon) => spore_recursion(*spec, dom, hay, cox, bug, nut, def),
        Spec::BucCab(_) => Hoon::Rock("n".to_string(), Noun::Atom(Atom::Small(0))),
        Spec::BucCol(spec, specs) => spore_buccol_recursion(*spec, specs, dom, hay, cox, bug, nut, def),
        Spec::BucCen(spec, specs) => spore_buccen_recursion(*spec, specs, dom, hay, cox, bug, nut, def),
        Spec::BucHep(spec, specs) => Hoon::Rock("n".to_string(), Noun::Atom(Atom::Small(0))),
        Spec::BucGal(p_spec, q_spec) => spore_recursion(*q_spec, dom, hay, cox, bug, nut, def),
        Spec::BucGar(p_spec, q_spec) => spore_recursion(*q_spec, dom, hay, cox, bug, nut, def),
        Spec::BucKet(p_spec, q_spec) => spore_recursion(*q_spec, dom, hay, cox, bug, nut, def),
        Spec::BucLus(stud, spec) => {
           let tail = spore_recursion(*spec, dom, hay, cox, bug, nut, def);
           Hoon::Note(Note::Know(stud), Box::new(tail))
        }
        Spec::BucMic(hoon) => Hoon::TisGal(Box::new(Hoon::Axis(6)), Box::new(hoon)),
        Spec::BucPam(spec, hoon) => spore_recursion(*spec, dom, hay, cox, bug, nut, def),
        Spec::BucSig(hoon, spec) => Hoon::KetHep(spec, Box::new(hoon)),
        Spec::BucTis(skin, spec) => {
            let tail = spore_recursion(*spec, dom, hay, cox, bug, nut, def);
            Hoon::KetTis(skin, Box::new(tail))
        }
        Spec::BucPat(p_spec, q_spec) => spore_recursion(*p_spec, dom, hay, cox, bug, nut, def),
        Spec::BucWut(spec, specs) => spore_bucwut_recursion(*spec, specs, dom, hay, cox, bug, nut, def),
        Spec::BucDot(..) | Spec::BucFas(..) | Spec::BucTic(..) | Spec::BucZap(..)
         => Hoon::Rock("n".to_string(), Noun::Atom(Atom::Small(0))),
    }
}

pub fn spore_buccol_recursion(spec: Spec,
                list_spec: Vec<Spec>,
                dom: u64,
                hay: WingType,
                cox: HashMap<String, Spec>,
                bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    if list_spec.is_empty() {
        spore_recursion(spec, dom, hay, cox, bug, nut, def)
    } else {
        let head = spore_recursion(spec,
                                    dom.clone(),
                                    hay.clone(),
                                    cox.clone(),
                                    bug.clone(),
                                    nut.clone(),
                                    def.clone());
        let tail = spore_buccol_recursion(list_spec.first().unwrap().clone(),
                                         list_spec[1..].to_vec(),
                                         dom,
                                         hay,
                                         cox,
                                         bug,
                                         nut,
                                         def);
        Hoon::Pair(Box::new(head), Box::new(tail))
    }
}

pub fn spore_bucwut_recursion(spec: Spec,
                list_spec: Vec<Spec>,
                dom: u64,
                hay: WingType,
                cox: HashMap<String, Spec>,
                bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    if list_spec.is_empty() {
        spore_recursion(spec, dom, hay, cox, bug, nut, def)
    } else {
        spore_bucwut_recursion(list_spec.first().unwrap().clone(),
                               list_spec[1..].to_vec(),
                                dom,
                                hay,
                                cox,
                                bug,
                                nut,
                                def)
    }
}

pub fn spore_buccen_recursion(spec: Spec,
                list_spec: Vec<Spec>,
                dom: u64,
                hay: WingType,
                cox: HashMap<String, Spec>,
                bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    if list_spec.is_empty() {
        spore_recursion(spec, dom, hay, cox, bug, nut, def)
    } else {
        spore_buccen_recursion(list_spec.first().unwrap().clone(),
                               list_spec[1..].to_vec(),
                                dom,
                                hay,
                                cox,
                                bug,
                                nut,
                                def)
    }
}

pub fn example(
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<String, Spec>,
    bug: &Vec<Spot>,
    nut: &Option<Note>,
    def: &Option<Hoon>,
) -> Hoon {
    match mod_ {
        Spec::Base(b) => {
            decorate(basal(b.clone()), bug.clone(), nut.clone())
        }
        Spec::Dbug(spot, inner) => {
            let mut bug = bug.clone();
            bug.push(spot.clone());
            example(&inner, dom, hay, cox, &bug, nut, def)
        }
        Spec::Leaf(term, atom) => {
            decorate(Hoon::Rock(term.clone(), Noun::Atom(atom.clone())), bug.clone(), nut.clone())
        }
        Spec::Like(wing, list) => {
            example(&Spec::BucMic(unreel(wing.clone(), list.clone())),
                dom, wing, cox, bug, nut, def)
        }
        Spec::Loop(term) => {
            Hoon::Limb(term.clone())
        }
        Spec::Made((t, list), inner) => {
            let pieces = list
                .iter()
                .map(|s| vec![Limb::Term(s.to_string())])
                .collect();
            example(&inner, dom, hay, cox, bug,
                    &Some(Note::Made(t.to_string(), Some(pieces))), def)
        }
        Spec::Make(head, tail) => {
            example(&Spec::BucMic(unfold(head.clone(), tail.clone())), dom, hay, cox, bug, nut, def)
        }
        Spec::Name(term, inner) => {
            example(&inner, dom, hay, cox, bug, &Some(Note::Made(term.to_string(), None)), def)
        }
        Spec::Over(wing, inner) => {
            example(&inner, dom, wing, cox, bug, nut, def)
        }
        Spec::BucCab(p) => {
            decorate(home(p.clone(), hay.clone(), dom.clone()), bug.clone(), nut.clone())
        }
        Spec::BucCol(head, tail) => {
           let mut result = example(head, dom, hay, cox, &vec![], &None, &None);

            for x in tail.iter().rev() {
                let next = example(&x, dom, hay, cox, &vec![], &None, &None);
                result = Hoon::Pair(Box::new(next), Box::new(result));
            }

            decorate(result, bug.clone(), nut.clone())
        }
        Spec::BucHep(p, q) => {
            let function_res = function(*p.clone(), *q.clone(), mod_, dom, hay, cox, &vec![], &None, &None);
            decorate(
                function_res,
                bug.clone(),
                nut.clone())
        }
        Spec::BucMic(inner) => {
            let tsgl = Hoon::TisGal(
                            Box::new(Hoon::Limb("$".to_string())),
                            Box::new(inner.clone()));
            decorate(home(tsgl, hay.clone(), dom.clone()), bug.clone(), nut.clone())
        }
        Spec::BucSig(inner, list) => {
            Hoon::KetLus(
                Box::new(example(&list, dom, hay, cox, bug, nut, def)),
                Box::new(home(inner.clone(), hay.clone(), dom.clone()))
            )
        }
        Spec::BucLus(stud, inner) => {
            decorate(
                Hoon::Note(
                    Note::Know(stud.clone()),
                    Box::new(example(&inner.clone(), dom, hay, cox, bug, nut, def)),
                ),
                bug.clone(),
                nut.clone())
        }
        Spec::BucTis(skin, inner) => {
            decorate(
                Hoon::KetTis(
                    skin.clone(),
                    Box::new(example(&inner.clone(), dom, hay, cox, bug, nut, def)),
                ),
                bug.clone(),
                nut.clone())
        }
        Spec::BucDot(inner, map) => vair_case(Vair::Gold, *inner.clone(), map.clone(), mod_, dom, hay, cox, bug, nut, def),
        Spec::BucFas(inner, map) => vair_case(Vair::Iron, *inner.clone(), map.clone(), mod_, dom, hay, cox, bug, nut, def),
        Spec::BucZap(inner, map) => vair_case(Vair::Lead, *inner.clone(), map.clone(), mod_, dom, hay, cox, bug, nut, def),
        Spec::BucTic(inner, map) => vair_case(Vair::Zinc, *inner.clone(), map.clone(), mod_, dom, hay, cox, bug, nut, def),
        _ => {
            let spore_result = spore(mod_.clone(),
                                          dom.clone(),
                                          hay.clone(),
                                          cox.clone(),
                                          bug.clone(),
                                          nut.clone(),
                                          def.clone());
            let dom = peg(dom, 3).expect("example +peg failed");
            let relative_result = relative(2, mod_, dom, hay, cox, bug, nut, def);
            Hoon::TisLus(Box::new(spore_result), Box::new(relative_result))
        }
    }
}

// used in +example
fn vair_case(
    vair: Vair,
    payload: Spec,
    arms: HashMap<String, Spec>,
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<String, Spec>,
    bug: &Vec<Spot>,
    nut: &Option<Note>,
    def: &Option<Hoon>,
) -> Hoon {
    let hoon = interface(vair, payload, arms, mod_, dom, hay, cox, bug, nut, def);
    decorate(home(hoon, hay.clone(), dom.clone()), bug.clone(), nut.clone())
}

pub fn basic(bas: BaseType,
                axe: u64,
                mod_: &Spec,
                dom: u64,
                hay: &WingType,
                cox: &HashMap<String, Spec>,
                bug: &Vec<Spot>,
                nut: &Option<Note>,
                def: &Option<Hoon>) -> Hoon {
    match bas {
        BaseType::Atom(a) => {
            let cnls = Hoon::CenLus(Box::new(Hoon::Limb("ruth".to_string())),
                                    Box::new(Hoon::Sand("ta".to_string(), Noun::Atom(string_to_atom(a)))),
                                    Box::new(Hoon::Axis(axe)));

            let example_res = Box::new(example(mod_, dom, hay, cox, bug, nut, def));

            let wtpt_limb = Limb::Axis(axe);
            let wtpt_wing: Vec<Limb> = vec![wtpt_limb];
            let wtpt = Hoon::WutPat(wtpt_wing, Box::new(Hoon::Axis(axe)), Box::new(Hoon::ZapZap));

            let zppt_limb = Limb::Parent(0, Some("ruth".to_string()));
            let zppt_wing: Vec<Limb> = vec![zppt_limb];
            let zppt_list_wing: Vec<Vec<Limb>> = vec![zppt_wing];
            let zppt = Hoon::ZapPat(zppt_list_wing, Box::new(cnls), Box::new(wtpt));

            Hoon::KetLus(example_res, Box::new(zppt))
        }
        BaseType::Cell => {
            let example_res = Box::new(example(mod_, dom, hay, cox, bug, nut, def));
            let wing = Limb::Axis(axe);
            let wing: Vec<Limb> = vec![wing];
            let mut p = wing.clone();
            p.insert(0, Limb::Axis(2));
            let mut q = wing.clone();
            q.insert(0, Limb::Axis(3));
            let pair = Hoon::Pair(Box::new(Hoon::Wing(p)), Box::new(Hoon::Wing(q)));

            Hoon::KetLus(example_res, Box::new(pair))
        }
        BaseType::Flag => {
            let rock = Box::new(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(0))));
            let dtts = Box::new(Hoon::DotTis(
                                    Box::new(Hoon::Rock("$".to_string(), Noun::Atom(Atom::Small(0)))),
                                    Box::new(Hoon::Axis(axe))
                                ));
            let wtgr = Box::new(Hoon::WutGar(
                            Box::new(Hoon::DotTis(
                                Box::new(Hoon::Rock("$".to_string(), Noun::Atom(Atom::Small(1)))),
                                Box::new(Hoon::Axis(axe))
                            )),
                            Box::new(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(1))))
                        ));
            Hoon::WutCol(dtts, rock, wtgr)
        },
        BaseType::Null => {
            let rock = Box::new(Hoon::Rock("n".to_string(), Noun::Atom(Atom::Small(0))));
            let dtts = Box::new(Hoon::DotTis(
                                    Box::new(Hoon::Bust(BaseType::Noun)),
                                    Box::new(Hoon::Axis(axe))
                                ));
            Hoon::WutGar(dtts, rock)
        }
        BaseType::Noun => Hoon::Axis(axe),
        BaseType::Void => Hoon::ZapZap,
    }
}

pub fn switch(
    one: Spec,
    mut rep: Vec<Spec>,
    axe: u64,
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<String, Spec>,
    bug: &Vec<Spot>,
    nut: &Option<Note>,
    def: &Option<Hoon>,
) -> Hoon {
    if rep.is_empty() {
        return relative(axe, &one, dom, hay, cox, &vec![], &None, &None);
    }

    let mut iter = rep.into_iter();
    let i_rep = iter.next().unwrap();
    let t_rep: Vec<Spec> = iter.collect();

    let fin = switch(i_rep.clone(), t_rep, axe, mod_, dom, hay, cox, bug, nut, def);

    let example_res = example(&one.clone(), dom, hay, cox, &vec![], &None, &None);

    let fits = Hoon::Fits(
        Box::new(Hoon::TisGal(
            Box::new(Hoon::Axis(2)),
            Box::new(example_res),
        )),
        vec![Limb::Axis(peg(axe, 2).expect("+switch, peg failed!"))],
    );

    let relative_result = relative(axe, &one, dom, hay, cox, &vec![], &None, &None);

    Hoon::WutCol(Box::new(fits), Box::new(relative_result), Box::new(fin))
}

pub fn choice_(one: Spec,
            mut rep: Vec<Spec>,
            axe: u64,
            mod_: &Spec,
            dom: u64,
            hay: &WingType,
            cox: &HashMap<String, Spec>,
            bug: &Vec<Spot>,
            nut: &Option<Note>,
            def: &Option<Hoon>,
) -> Hoon {
    if rep.is_empty() {
        return relative(axe, &one, dom, hay, cox, &vec![], &None, &None);
    }

    let mut iter = rep.into_iter();
    let i_rep = iter.next().unwrap();
    let t_rep: Vec<Spec> = iter.collect();

    let example_res = example(&one.clone(), dom, hay, cox, &vec![], &None, &None);

    let fits = Hoon::Fits(
        Box::new(example_res),
        vec![Limb::Axis(axe)],
    );

    let relative_result =
            relative(axe,
                        &one.clone(),
                        dom,
                        hay,
                        cox,
                        &vec![],
                        &None,
                        &None);
    let tail = choice_(i_rep.clone(), t_rep, axe, mod_, dom, hay, cox, bug, nut, def);

    Hoon::WutCol(Box::new(fits), Box::new(relative_result), Box::new(tail))
}

pub fn relative(axe: u64,
                mod_: &Spec,
                dom: u64,
                hay: &WingType,
                cox: &HashMap<String, Spec>,
                bug: &Vec<Spot>,
                nut: &Option<Note>,
                def: &Option<Hoon>,
) -> Hoon {
    match &mod_ {
        Spec::Base(p) => decorate(basic(p.clone(), axe, mod_, dom, hay, cox, &vec![], &None, &None), bug.clone(), nut.clone()),
        Spec::Dbug(p, q) => {
            let mut bug = bug.clone();
            bug.push(p.clone());
            relative(axe, &*q, dom, hay, cox, &bug, nut, def)
        },
        Spec::Leaf(p, q) => {
            decorate(
                Hoon::WutGar(
                    Box::new(Hoon::DotTis(Box::new(Hoon::Axis(axe)),
                                          Box::new(Hoon::Rock("$".to_string(), Noun::Atom(q.clone()))))),
                    Box::new(Hoon::Rock(p.clone(), Noun::Atom(q.clone())))
                ),
                bug.clone(),
                nut.clone(),
            )
        }
        Spec::Make(p, q) => relative(axe, &Spec::BucMic(unfold(p.clone(), q.clone())), dom, hay, cox, bug, nut, def),
        Spec::Like(p, q) => relative(axe, &Spec::BucMic(unreel(p.clone(), q.clone())), dom, hay, cox, bug, nut, def),
        Spec::Loop(p) => decorate(
            Hoon::CenHep(Box::new(Hoon::Limb(p.clone())), Box::new(Hoon::Axis(axe))),
            bug.clone(),
            nut.clone(),
        ),
        Spec::Name(p, q) => relative(axe, &*q, dom, hay, cox, bug, &Some(Note::Made(p.clone(), None)), def),
        Spec::Made((term, list), q) => {
            let pieces = list
                        .iter()
                        .map(|s| vec![Limb::Term(s.to_string())])
                        .collect();
            let nut = Some(Note::Made(term.clone(), Some(pieces)));
            relative(axe, &*q, dom, hay, cox, bug, &nut, def)
        }
        Spec::Over(p, q) => relative(axe, &*q, dom, p, cox, bug, nut, def),
        Spec::BucBuc(p, q) => {
            let new_dom = peg(3, dom).expect("+relative-bucbuc-peg-failed");
            let map: HashMap<String, Hoon> = q.into_iter()
                .map(|(term, spec)| (term.clone(), relative(axe, spec, new_dom, hay, cox, bug, nut, def)))
                .collect();
            Hoon::BarKet(
                Box::new(relative(axe, &*p, new_dom, hay, cox, bug, nut, def)),
                HashMap::from([("$".to_string(), map)]),
            )
        }
        Spec::BucPam(p, q) => Hoon::TisLus(
            Box::new(relative(axe, &*p, dom, hay, cox, bug, nut, def)),
            Box::new(Hoon::TisLus(
                Box::new(Hoon::TisGar(Box::new(Hoon::Axis(3)), Box::new(q.clone()))),
                Box::new(Hoon::TisLus(
                    Box::new(Hoon::CenHep(Box::new(Hoon::Axis(2)), Box::new(Hoon::Axis(6)))),
                    Box::new(Hoon::WutGar(
                        Box::new(Hoon::WutBar(vec![
                            Hoon::DotTis(Box::new(Hoon::Axis(14)), Box::new(Hoon::Axis(2))),
                            Hoon::DotTis(
                                Box::new(Hoon::Axis(2)),
                                Box::new(Hoon::CenHep(Box::new(Hoon::Axis(6)), Box::new(Hoon::Axis(2))))
                            )
                        ])),
                        Box::new(Hoon::Axis(2))
                    ))
                ))
            ))
        ),
        Spec::BucBar(p, q) => Hoon::TisLus(
            Box::new(relative(axe, &*p, dom, hay, cox, bug, nut, def)),
            Box::new(Hoon::WutGar(
                Box::new(Hoon::CenHep(Box::new(Hoon::TisGar(Box::new(Hoon::Axis(3)), Box::new(q.clone()))), Box::new(Hoon::Axis(2)))),
                Box::new(Hoon::Axis(2))
            ))
        ),
        Spec::BucCab(p) => decorate(home(p.clone(), hay.clone(), dom.clone()), bug.clone(), nut.clone()),
        Spec::BucCen(p, t) => decorate(switch(*p.clone(), t.clone(), axe, mod_, dom, hay, cox, bug, nut, def),
                                        bug.clone(),
                                        nut.clone()),
        Spec::BucCol(p, q) => {
            let mut result: Option<Hoon> = None;
            let mut current_axe = axe;

            let first = relative(
                peg(current_axe, 2).expect("+relative-buccol-peg-failed"),
                &*p,
                dom,
                hay,
                cox,
                bug,
                nut,
                def,
            );

            result = Some(first);
            current_axe = peg(current_axe, 3).expect("+relative-buccol-peg-failed");

            for spec in q {
                let hoon = relative(
                    peg(current_axe, 2).expect("+relative-buccol-peg-failed"),
                    spec,
                    dom,
                    hay,
                    cox,
                    bug,
                    nut,
                    def,
                );

                result = Some(Hoon::Pair(
                    Box::new(result.unwrap()),
                    Box::new(hoon),
                ));

                current_axe = peg(current_axe, 3).expect("+relative-buccol-peg-failed");
            }

            decorate(result.unwrap(), bug.clone(), nut.clone())
        }
        Spec::BucGal(p, q) => Hoon::TisLus(
            Box::new(relative(axe, &*q, dom, hay, cox, &vec![], &None, &None)),
            Box::new(Hoon::WutGal(
                Box::new(Hoon::WutTis(
                    Box::new(Spec::Over(vec![Limb::Axis(3)], p.clone())),
                    vec![Limb::Axis(4)]
                )),
                Box::new(Hoon::Axis(2))
            ))
        ),
        Spec::BucGar(p, q) => Hoon::TisLus(
            Box::new(relative(axe, &*q, dom, hay, cox, &vec![], &None, &None)),
            Box::new(Hoon::WutGar(
                Box::new(Hoon::WutTis(
                    Box::new(Spec::Over(vec![Limb::Axis(3)], p.clone())),
                    vec![Limb::Axis(4)],
                )),
                Box::new(Hoon::Axis(2))
            ))
        ),
        Spec::BucHep(p, q) => {
            let function_res = function(*p.clone(), *q.clone(), mod_, dom, hay, cox, &vec![], &None, &None);
            decorate(
                match def {
                    Some(d) => Hoon::KetLus(Box::new(function_res),
                                            Box::new(d.clone())),
                    None => function_res
                },
                bug.clone(),
                nut.clone(),
            )
        }
        Spec::BucKet(p, q) => decorate(
            Hoon::WutCol(
                Box::new(Hoon::DotWut(Box::new(Hoon::Axis(peg(axe, 2).expect("bucket-peg-failed"))))),
                Box::new(relative(axe, &*p, dom, hay, cox, &vec![], &None, &None)),
                Box::new(relative(axe, &*q, dom, hay, cox, &vec![], &None, &None))
            ),
            bug.clone(),
            nut.clone(),
        ),
        Spec::BucMic(p) => decorate(
            Hoon::CenCol(
                Box::new(home(p.clone(), hay.clone(), dom.clone())),
                vec![Hoon::Axis(axe)],
            ),
            bug.clone(),
            nut.clone(),
        ),
        Spec::BucSig(p, q) => relative(axe, &*q, dom, hay, cox, bug, nut, &Some(Hoon::KetHep(q.clone(), Box::new(p.clone())))),
        Spec::BucWut(p, t) => decorate(choice_(*p.clone(), t.clone(), axe, mod_, dom, hay, cox, bug, nut, def), bug.clone(), nut.clone()),
        Spec::BucTis(p, q) => Hoon::KetTis(p.clone(), Box::new(relative(axe, &*q, dom, hay, cox, bug, nut, def))),
        Spec::BucPat(p, q) => decorate(
            Hoon::WutCol(
                Box::new(Hoon::DotWut(Box::new(Hoon::Axis(axe)))),
                Box::new(relative(axe, &*q, dom, hay, cox, &vec![], &None, &None)),
                Box::new(relative(axe, &*p, dom, hay, cox, &vec![], &None, &None)),
            ),
            bug.clone(),
            nut.clone(),
        ),
        Spec::BucLus(p, q) => Hoon::Note(Note::Know(p.clone()),
                                        Box::new(relative(axe, &*q, dom, hay, cox, bug, nut, def))),
        Spec::BucDot(p, q) => {
            let x = interface(Vair::Gold, *p.clone(), q.clone(), mod_, dom, hay, cox, bug, nut, def);
            let y = home(x, hay.clone(), dom.clone());
            decorate(y, bug.clone(), nut.clone())
        }

        Spec::BucFas(p, q) => {
            let x = interface(Vair::Iron, *p.clone(), q.clone(), mod_, dom, hay, cox, bug, nut, def);
            let y = home(x, hay.clone(), dom.clone());
            decorate(y, bug.clone(), nut.clone())
        }

        Spec::BucZap(p, q) => {
            let x = interface(Vair::Lead, *p.clone(), q.clone(), mod_, dom, hay, cox, bug, nut, def);
            let y = home(x, hay.clone(), dom.clone());
            decorate(y, bug.clone(), nut.clone())
        }

        Spec::BucTic(p, q) => {
            let x = interface(Vair::Zinc, *p.clone(), q.clone(), mod_, dom, hay, cox, bug, nut, def);
            let y = home(x, hay.clone(), dom.clone());
            decorate(y, bug.clone(), nut.clone())
        }
    }
}

pub fn home(gen: Hoon,
            mut hay: WingType,
            dom: u64) -> Hoon {

    let wing = if  1 != dom {
        hay
    } else {
        hay.push(Limb::Axis(dom));
        hay
    };

    if wing.is_empty() {
        gen
    } else {
        Hoon::TisGar(Box::new(Hoon::Wing(wing)), Box::new(gen))
    }
}

pub fn unreel(one: WingType, res: Vec<WingType>) -> Hoon {
    if res.is_empty() {
        Hoon::Wing(one)
    } else {
        match res.first() {
            Some(first) => {
                let wing_tail = unreel(first.clone(), res[1..].to_vec());
                Hoon::TisGal(Box::new(Hoon::Wing(one)), Box::new(wing_tail))
            }
            None => Hoon::Wing(one),
        }
    }
}


pub fn unfold(fun: Hoon, arg: Vec<Spec>) -> Hoon {
    let cencol_tail: Vec<Hoon> = arg.iter().map(|spec| Hoon::KetCol(Box::new(spec.clone()))).collect();
    Hoon::CenCol(Box::new(fun), cencol_tail)
}

pub fn factory(mod_: Spec,
                dom: u64,
                hay: WingType,
                cox: HashMap<String, Spec>,
                bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    match mod_ {
        Spec::Dbug(spot, spec) => {
            let mut bug = bug.clone();
            bug.insert(0, spot);
            factory(*spec, dom, hay, cox, bug, nut, def)
        }
        Spec::BucSig(hoon, spec) => {
            let spec_clone = spec.clone();
            let spec_clone2 = spec.clone();
            factory(*spec_clone, dom, hay, cox, bug, nut, Some(Hoon::KetHep(spec_clone2, Box::new(hoon))))
        }
        _ => {
            match (def.clone(), mod_.clone()) {
                (Some(_), Spec::BucMic(h)) => decorate(home(h, hay, dom), bug, nut),
                (Some(_), Spec::Like(wing, vec_wing)) => decorate(home(unreel(wing, vec_wing), hay, dom), bug, nut),
                (Some(_), Spec::Loop(term)) => decorate(home(Hoon::Limb(term), hay, dom), bug, nut),
                (Some(_), Spec::Make(h, s)) => decorate(home(unfold(h, s), hay, dom), bug, nut),
                _ => {
                    let spore_res = spore(mod_.clone(),
                                          dom.clone(),
                                          hay.clone(),
                                          cox.clone(),
                                          bug.clone(),
                                          nut.clone(),
                                          def.clone());

                    let ketsig = Box::new(Hoon::KetSig(Box::new(spore_res)));

                    let descent_axis = peg(7, dom).expect("factory-peg-failed");
                    let tislus =  Hoon::TisLus(Box::new(Hoon::DotTis(Box::new(Hoon::Axis(14)),
                                                            Box::new(Hoon::Axis(2)))),
                                               Box::new(Hoon::Axis(6)));
                    let relative_res = relative(6, &mod_, descent_axis, &hay, &cox, &bug, &nut, &def);
                    let tail = Hoon::TisLus(Box::new(relative_res),
                                            Box::new(tislus));

                    Hoon::BarCol(ketsig, Box::new(tail))
                }
            }
        }
    }
}

pub fn open(gen: Hoon) -> Hoon {
    match gen {
        Hoon::Axis(a) => Hoon::CenTis(vec![Limb::Axis(a)], Vec::new()),
        Hoon::Base(b) => factory(Spec::Base(b), 1, Vec::new(), HashMap::new(), Vec::new(), None, None),
        Hoon::Bust(b) => example(
            &Spec::Base(b), 1, &WingType::default(), &HashMap::new(), &Vec::new(), &None, &None,
        ),
        Hoon::Dbug(_, q) => *q,
        Hoon::Eror(s) => panic!("{}", s),
        Hoon::Knit(woofs) => {
            let ktts = Hoon::KetTis(Skin::Term("v".to_string()), Box::new(Hoon::Axis(1)));

            fn knit_loop(woofs: Vec<Woof>) -> Hoon {
                if woofs.is_empty() {
                    Hoon::Bust(BaseType::Null)
                } else {
                    let head = &woofs[0];
                    let tail = knit_loop(woofs[1..].to_vec());
                    match head {
                        Woof::Atom(a) => {
                            let sand = Hoon::Sand("tD".to_string(), Noun::Atom(a.clone()));
                            Hoon::Pair(Box::new(sand), Box::new(tail))
                        }
                        Woof::Hoon(p) => {
                            let a = Hoon::Pair(
                                        Box::new(Hoon::KetTis(
                                                Skin::Term("a".to_string()),
                                                Box::new(Hoon::KetLus(
                                                                Box::new(Hoon::Limb("$".to_string())),
                                                                Box::new(Hoon::TisGar(
                                                                        Box::new(Hoon::Limb("v".to_string())),
                                                                        Box::new(p.clone())))
                                                            )))),
                                        Box::new(Hoon::KetTis(Skin::Term("a".to_string()), Box::new(tail)))
                                    );
                            let b = Hoon::BarHep(
                                Box::new(
                                    Hoon::WutPat(
                                        vec![Limb::Term("a".to_string())],
                                        Box::new(Hoon::Limb("b".to_string())),
                                        Box::new(Hoon::Pair(
                                            Box::new(Hoon::TisGal(Box::new(Hoon::Axis(2)),
                                                                  Box::new(Hoon::Limb("a".to_string()))
                                                                )),
                                            Box::new(Hoon::CenTis(vec![Limb::Term("$".to_string())],
                                                                vec![(vec![Limb::Term("a".to_string())],
                                                                        Hoon::TisGal(Box::new(Hoon::Axis(3)),
                                                                            Box::new(Hoon::Limb("a".to_string())),
                                                                        ))])
                                                    ))
                                        )
                                    )
                                ));

                            Hoon::TisLus(
                                Box::new(a),
                                Box::new(b),
                            )

                        }
                    }
                }
            }

            let ktls =
                Hoon::KetLus(
                    Box::new(
                        Hoon::BarHep(Box::new(
                            Hoon::WutCol(
                                Box::new(Hoon::Bust(BaseType::Flag)),
                                Box::new(Hoon::Bust(BaseType::Null)),
                                Box::new(Hoon::Pair(
                                    Box::new(Hoon::KetTis(
                                        Skin::Term("i".to_string()),
                                        Box::new(Hoon::Sand("tD".to_string(),
                                                            Noun::Atom(Atom::Small(0)))),
                                    )),
                                    Box::new(Hoon::KetTis(
                                        Skin::Term("t".to_string()),
                                        Box::new(Hoon::Limb("$".to_string())),
                                    )),
                                )),
                            )
                        ))),
                    Box::new(knit_loop(woofs))
                );

            let brhp = Hoon::BarHep(Box::new(ktls));

            Hoon::TisGar(
                Box::new(ktts),
                Box::new(brhp),
            )
        }
        Hoon::Leaf(term, atom) => factory(Spec::Leaf(term, atom), 1, Vec::new(), HashMap::new(), Vec::new(), None, None),
        Hoon::Limb(term) => Hoon::CenTis(vec![Limb::Term(term)], Vec::new()),
        Hoon::Wing(wing) => Hoon::CenTis(wing, Vec::new()),
        Hoon::Note(_, q) => *q,

        Hoon::Tell(hoons) => {
            let zpgr = Hoon::ZapGar(Box::new(Hoon::ColTar(hoons)));
            Hoon::CenCol(
                Box::new(Hoon::Limb("noah".to_string())),
                vec![zpgr],
            )
        }

        Hoon::Yell(hoons) => {
            let zpgr = Hoon::ZapGar(Box::new(Hoon::ColTar(hoons)));
            Hoon::CenCol(
                Box::new(Hoon::Limb("cain".to_string())),
                vec![zpgr],
            )
        }

        Hoon::BarBuc(sample, body) => {
            if sample.is_empty() {
                panic!("empty sample in BarBuc");
            }

            let tar = Spec::Base(BaseType::Noun);
            let bcsg = Spec::BucSig(
                Hoon::Base(BaseType::Noun),
                Box::new(Spec::BucHep(
                    Box::new(tar.clone()),
                    Box::new(tar),
                )),
            );

            let transformed: Vec<Spec> = sample
                .iter()
                .map(|term| Spec::BucTis(Skin::Term(term.clone()), Box::new(bcsg.clone())))
                .collect();

            let (first, rest) = transformed.split_first().unwrap();

            Hoon::BarTar(
                Box::new(Spec::BucCol(
                    Box::new(first.clone()),
                    rest.to_vec(),
                )),
                Box::new(Hoon::KetCol(Box::new(*body))),
            )
        }

        Hoon::BarCab(spec, alas, arms) => {
            let transformed_arms = arms
                .into_iter()
                .map(|(term, tome)| {
                    let wrapped_pairs: Vec<(String, Hoon)> = tome
                            .into_iter()
                            .map(|(face, expr)| {
                                let wrapped_expr = alas.iter().rev().fold(expr, |body, (alas_face, alas_init)| {
                                    Hoon::TisTar(
                                        (alas_face.clone(), None),
                                        Box::new(alas_init.clone()),
                                        Box::new(body),
                                    )
                                });
                                (face, wrapped_expr)
                            })
                            .collect();

                    let tome_map: HashMap<_, _> = wrapped_pairs.into_iter().collect();

                    (term, tome_map)
                })
                .collect();

            Hoon::TisLus(
                Box::new(Hoon::KetTar(spec)),
                Box::new(Hoon::BarCen(None, transformed_arms)),
            )
        }

        Hoon::BarCol(p, q) => Hoon::TisLus(p, Box::new(Hoon::BarDot(q))),

        Hoon::BarDot(p) => {
            let map_term_hoon = {
                let mut m = HashMap::new();
                m.insert("$".to_string(), *p);
                m
            };
            let map_term_tome = {
                let mut m = HashMap::new();
                m.insert("$".to_string(), map_term_hoon);
                m
            };
            Hoon::BarCen(None, map_term_tome)
        }

        Hoon::BarKet(p, arms) => {
            let mut map = arms.clone();
            if let Some(zil) = arms.get(&"$".to_string()) {
                let updated = {
                    let mut inner = zil.clone();
                    inner.insert("$".to_string(), *p.clone());
                    inner
                };
                map.insert("$".to_string(), updated);
            } else {
                let mut inner = HashMap::new();
                inner.insert("$".to_string(), *p.clone());
                map.insert("$".to_string(), inner);
            }
            Hoon::TisGal(
                Box::new(Hoon::Limb("$".to_string())),
                Box::new(Hoon::BarCen(None, map)),
            )
        }

        Hoon::BarHep(p) => Hoon::TisGal(Box::new(Hoon::Limb("$".to_string())), Box::new(Hoon::BarDot(Box::new(*p)))),

        Hoon::BarSig(spec, q) => Hoon::KetBar(Box::new(Hoon::BarTis(spec.clone(), q.clone()))),

        Hoon::BarTar(spec, q) => {
            let map_term_hoon = {
                let mut m = HashMap::new();
                m.insert("$".to_string(), *q);
                m
            };
            let map_term_tome = {
                let mut m = HashMap::new();
                m.insert("$".to_string(), map_term_hoon);
                m
            };
            Hoon::TisLus(Box::new(Hoon::KetTar(spec)),
                        Box::new(Hoon::BarPat(None, map_term_tome)))
        }

        Hoon::BarTis(spec, q) => {
            let map_term_hoon = {
                let mut m = HashMap::new();
                m.insert("$".to_string(), *q);
                m
            };
            let map_term_tome = {
                let mut m = HashMap::new();
                m.insert("$".to_string(), map_term_hoon);
                m
            };
            Hoon::BarCab(spec, vec![], map_term_tome)
        }

        Hoon::BarWut(p) => Hoon::KetWut(Box::new(Hoon::BarDot(p))),

        Hoon::ColKet(p, q, r, s) => {
               Hoon::Pair(
                    p,
                    Box::new(Hoon::Pair(
                        q,
                        Box::new(Hoon::Pair(
                            r,
                            s
                        ))
                    ))
                )
            }

        Hoon::ColCab(p, q) => Hoon::Pair(q, p),

        Hoon::ColHep(p, q) => Hoon::Pair(p, q),

        Hoon::ColLus(p, q, r) => {
            Hoon::Pair(
                    p,
                    Box::new(
                        Hoon::Pair(
                            q,
                            r,
                        )
                    )
                )
        }

        Hoon::ColSig(hoons) => {
            match hoons.as_slice() {
                [] => Hoon::Rock("n".to_string(), Noun::Atom(Atom::Small(0))),
                [h] => h.clone(),
                [h, tail @ ..] => {
                    let rest = open(Hoon::ColSig(tail.to_vec()));
                    Hoon::Pair(Box::new(h.clone()), Box::new(rest))
                }
            }
        }

        Hoon::ColTar(hoons) => {
            match hoons.as_slice() {
                [] => Hoon::ZapZap,
                [h] => h.clone(),
                [h, tail @ ..] => {
                    let rest = open(Hoon::ColTar(tail.to_vec()));
                    Hoon::Pair(Box::new(h.clone()), Box::new(rest))
                }
            }
        }
        Hoon::KetTar(spec) => Hoon::KetSig(
                                    Box::new(example(&spec, 1, &Vec::new(), &HashMap::new(), &Vec::new(), &None, &None))),

        Hoon::CenCab(wing, pairs) => {
            Hoon::KetLus(Box::new(Hoon::Wing(wing.clone())),
                        Box::new(Hoon::CenTis(wing, pairs)))
        }

        Hoon::CenDot(p, q) => Hoon::CenCol(q, vec![*p]),

        Hoon::CenKet(p, q, r, s) => Hoon::CenCol(p, vec![*q, *r, *s]),

        Hoon::CenLus(p, q, r) => Hoon::CenCol(p, vec![*q, *r]),

        Hoon::CenHep(p, q) => Hoon::CenCol(p, vec![*q]),

        Hoon::CenCol(p, hoons) => {
            Hoon::CenSig(vec![Limb::Term("$".to_string())], p, hoons)
        }

        Hoon::CenSig(wing, p, hoons) => {
            fn compile_r_gen_rec(r_gen: &[Hoon], axe: u64) -> Vec<(Vec<Limb>, Hoon)> {
                match r_gen.split_first() {
                    None => vec![],
                    Some((hoon, rest)) => {
                        let (wing_axe, next_axe) = if rest.is_empty() {
                            (axe, 0)
                        } else {
                            (peg(axe, 2).expect("+open: peg failed"), peg(axe, 3).expect("+open: peg failed"))
                        };

                        let wing = vec![
                            Limb::Parent(0, None),
                            Limb::Axis(wing_axe),
                        ];

                        let mut out = vec![(wing, hoon.clone())];
                        if !rest.is_empty() {
                            out.extend(compile_r_gen_rec(rest, next_axe));
                        }
                        out
                    }
                }
            }
            let list = compile_r_gen_rec(&hoons, 6);
            Hoon::CenTar(wing, p, list)
        }

        Hoon::CenTar(mut wing, p, pairs) => {
            if pairs.is_empty() {
               return Hoon::TisGar(p, Box::new(Hoon::Wing(wing)));
            }
            wing.extend(vec![Limb::Axis(2)]);
            let wrapped = pairs
                    .into_iter()
                    .map(|(p, q)| (p, Hoon::TisGar(Box::new(Hoon::Axis(3)), Box::new(q))))
                    .collect();
            Hoon::TisLus(p,
                    Box::new(
                        Hoon::CenTis(wing, wrapped)
                    ))
        }

        Hoon::KetDot(p, q) => Hoon::KetLus(Box::new(Hoon::CenCol(p, vec![*q.clone()])), q),

        Hoon::KetHep(spec, q) => {
            let example_res =
                example(&spec, 1, &Vec::new(), &HashMap::new(), &Vec::new(), &None, &None);
            Hoon::KetLus(Box::new(example_res), q)
        }

        Hoon::KetTis(skin, p) => grip(skin, *p, vec![]),


        Hoon::SigBar(p, q) => {
            let fek = {
                let fek = feck(*p.clone());
                match fek {
                    Some(s) => Hoon::Rock("tas".to_string(), Noun::Atom(s)),
                    None => {
                        Hoon::BarDot(Box::new(Hoon::CenCol(
                            Box::new(Hoon::Limb("cain".to_string())),
                            vec![Hoon::ZapGar(Box::new(Hoon::TisGal(
                                              Box::new(Hoon::Axis(3)), p)))])))
                    }
                }
            };
            let hint = TermOrPair::Pair("mean".to_string(), Box::new(fek));
            Hoon::SigGar(hint, q)
        }

        Hoon::SigCab(p, q) => Hoon::SigGar(
            TermOrPair::Term("mean".to_string()),
            Box::new(Hoon::BarDot(p)),
        ),

        Hoon::SigCen(chum, p, tyre, q) => {
            let clsg_vec = {
                let mut nob = vec![];
                let mut r = tyre;
                while !r.is_empty() {
                    let (p_i, q_i) = r.remove(0);
                    nob.push(Hoon::Pair(
                        Box::new(Hoon::Rock("$".to_string(), Noun::Atom(string_to_atom(p_i)))),
                        Box::new(Hoon::ZapTis(Box::new(q_i))),
                    ));
                }
                nob
            };
            let clls =
                Hoon::ColLus(
                    Box::new(Hoon::Rock("$".to_string(), chum_to_noun(chum))),
                    Box::new(Hoon::ZapTis(q.clone())),
                    Box::new(Hoon::ColSig(clsg_vec)),
                );
            Hoon::SigGal(
                TermOrPair::Pair("fast".to_string(), Box::new(clls)),
                q,
            )
        }

        Hoon::SigFas(chum, q) => Hoon::SigCen(chum, Box::new(Hoon::Axis(7)), vec![], q),

        Hoon::SigGal(term_or_pair, q) => Hoon::TisGal(Box::new(Hoon::SigGar(term_or_pair, Box::new( Hoon::Axis(1)))), q),

        Hoon::SigBuc(term, q) => Hoon::SigGar(
            TermOrPair::Pair("live".to_string(), Box::new(Hoon::Rock("$".to_string(), Noun::Atom(string_to_atom(term))))),
            q
        ),

        Hoon::SigLus(a, q) => Hoon::SigGar(
            TermOrPair::Pair("memo".to_string(),
                            Box::new(Hoon::Rock("$".to_string(),
                                                Noun::Atom(Atom::Small(a.into()))))),
            q
        ),

        Hoon::SigPam(a, p, q) => Hoon::SigGar(
            TermOrPair::Pair(
                "slog".to_string(),
                Box::new(Hoon::Pair(
                    Box::new(Hoon::Sand("$".to_string(), Noun::Atom(Atom::Small(a.into())))),
                    Box::new(Hoon::CenCol(
                    Box::new(Hoon::Limb("cain".to_string())),
                    vec![Hoon::ZapGar(p)]))))
            ),
            q,
        ),

        Hoon::SigTis(p, q) => Hoon::SigGar(
            TermOrPair::Pair("germ".to_string(), p),
            q,
        ),

        Hoon::SigWut(a, p, q, r) => {
            let wtdt = Hoon::WutDot(p, Box::new(Hoon::Bust(BaseType::Null)), Box::new(Hoon::Pair(Box::new(Hoon::Bust(BaseType::Null)),
                                                                                                 Box::new(*q))));
            let sgpm = Hoon::SigPam(a, Box::new(Hoon::Axis(5)), Box::new(Hoon::TisGar(Box::new(Hoon::Axis(3)), r.clone())));
            let wtsg = Hoon::WutSig(vec![Limb::Axis(2)], Box::new(Hoon::TisGar(Box::new(Hoon::Axis(3)), r)), Box::new(sgpm));
            Hoon::TisLus(
                Box::new(wtdt),
                Box::new(wtsg),
                )
        }

        Hoon::MicTis(marl) => {
            fn loop_marl(marl: Marl) -> Hoon {
                match marl.split_first() {
                    None => Hoon::Bust(BaseType::Null),
                    Some((head, tail)) => match head {
                        Tuna::Manx(m) => {
                            Hoon::Pair(Box::new(Hoon::Xray(m.clone())), Box::new(loop_marl(tail.to_vec())))
                        },
                        Tuna::TunaTail(TunaTail::Manx(m)) => Hoon::Pair(Box::new(m.clone()), Box::new(loop_marl(tail.to_vec()))),
                        Tuna::TunaTail(TunaTail::Tape(t)) => Hoon::Pair(Box::new(Hoon::MicFas(Box::new(t.clone()))),
                                                        Box::new(loop_marl(tail.to_vec()))),
                        Tuna::TunaTail(TunaTail::Call(h)) => Hoon::CenCol(Box::new(h.clone()), vec![loop_marl(tail.to_vec())]),
                        Tuna::TunaTail(TunaTail::Marl(sub)) => {
                            let tsbr = Box::new(Hoon::TisBar(
                                Box::new(Spec::Base(BaseType::Cell)),
                                Box::new(Hoon::BarPat(None, {
                                    let sug = vec![Limb::Axis(12)];
                                    let wtsg = Hoon::WutSig(sug.clone(),
                                                            Box::new(Hoon::CenTis(sug.clone(), vec![(vec![Limb::Axis(1)], Hoon::Axis(13))])),
                                                            Box::new(Hoon::CenTis(sug.clone(),
                                                                vec![(vec![Limb::Axis(3)],
                                                                    Hoon::CenTis(vec![Limb::Term("$".to_string())],
                                                                        vec![(sug, Hoon::Axis(25))]
                                                                    ))]))
                                                        );
                                    let map_term_hoon = {
                                        let mut m = HashMap::new();
                                        m.insert("$".to_string(), wtsg);
                                        m
                                    };
                                    let map_term_tome = {
                                        let mut m = HashMap::new();
                                        m.insert("$".to_string(), map_term_hoon);
                                        m
                                    };
                                    map_term_tome
                                }))),
                            );
                            Hoon::CenDot(Box::new(Hoon::Pair(Box::new(sub.clone()),
                                                        Box::new(loop_marl(tail.to_vec())))), tsbr)
                        }
                    }
                }
            }
            loop_marl(marl)
        }

        Hoon::MicCol(p, hoons) => {
            match hoons.as_slice() {
                [] => Hoon::ZapZap,
                [h] => h.clone(),
                [h, tail @ ..] => {
                    let yex = hoons;
                    fn loop_yex(yex: &[Hoon]) -> Hoon {
                        match yex {
                            [] => panic!("empty yex"),
                            [h] => Hoon::TisGal(Box::new(Hoon::Axis(3)), Box::new(h.clone())),
                            [h, t @ ..] => Hoon::CenCol(
                                Box::new(Hoon::Axis(2)),
                                vec![Hoon::TisGar(Box::new(Hoon::Axis(3)), Box::new(h.clone())),
                                loop_yex(t)]),
                            _ => panic!("miccol error"),
                        }
                    }
                    Hoon::TisLus(p, Box::new(loop_yex(&yex)))
                }
            }
        }

        Hoon::MicFas(p) => {
            let zoy = Hoon::Rock("ta".to_string(), Noun::Atom(Atom::Small(0)));
            Hoon::ColSig(vec![Hoon::Pair(
                                Box::new(zoy.clone()),
                                 Box::new(Hoon::ColSig(vec![Hoon::Pair(
                                        Box::new(zoy.clone()),
                                        p.clone())])))])
        }

        Hoon::MicGal(spec, q, r, s) => {
            let ktcl_p = Hoon::KetCol(spec.clone());
            let cnhp = Hoon::CenHep(q, Box::new(ktcl_p));
            let brts = Hoon::BarTis(spec, Box::new(Hoon::TisGar(Box::new(Hoon::Axis(3)), s)));
            Hoon::CenLus(
                Box::new(cnhp),
                r,
                Box::new(brts)
            )
        }

        Hoon::MicSig(p, q) => {
            fn loop_tail(p: Box<Hoon>, q: Vec<Hoon>) -> Hoon {
                match q.as_slice() {
                    [] => {
                        panic!("open-mcsg")
                    }
                    [first, rest @ ..] => {
                        if rest.is_empty() {
                            return Hoon::TisGar(Box::new(Hoon::Limb("v".to_string())), Box::new(first.clone()));
                        }
                        let a_bind = Hoon::KetTis(Skin::Term("a".to_string()),
                                                    Box::new(loop_tail(p.clone(), rest.to_vec())));

                        let b_expr = Hoon::TisGar(
                            Box::new(Hoon::Limb("v".to_string())),
                            Box::new(first.clone()),
                        );
                        let b_bind =
                            Hoon::KetTis(
                            Skin::Term("b".to_string()),
                                Box::new(Hoon::TisGar(Box::new(Hoon::Limb("v".to_string())), Box::new(first.clone())))
                            );

                        let wing_c = vec![
                            Limb::Parent(0, None),
                            Limb::Axis(6),
                        ];
                        let c_expr = Hoon::TisGal(
                            Box::new(Hoon::Wing(wing_c)),
                            Box::new(Hoon::Limb("b".to_string())),
                        );
                        let c_bind =
                            Hoon::KetTis(
                                Skin::Term("c".to_string()),
                                Box::new(Hoon::TisGal(
                                            Box::new(Hoon::Wing(vec![Limb::Parent(0, None), Limb::Axis(6)])),
                                            Box::new(Hoon::Limb("b".to_string())))));

                        let tsgr_v_p = Hoon::TisGar(
                            Box::new(Hoon::Limb("v".to_string())),
                            p.clone(),
                        );
                        let cncl_b_c = Hoon::CenCol(
                            Box::new(Hoon::Limb("b".to_string())),
                            vec![Hoon::Limb("c".to_string())],
                        );
                        let cnts_wing = vec![
                            Limb::Parent(0, None),
                            Limb::Axis(6),
                        ];
                        let cnts = Hoon::CenTis(
                            vec![Limb::Term("a".to_string())],
                            vec![(cnts_wing, Hoon::Limb("c".to_string()))],
                        );
                        let cnls = Hoon::CenLus(
                            Box::new(tsgr_v_p),
                            Box::new(cncl_b_c),
                            Box::new(cnts),
                        );

                        Hoon::TisLus(
                            Box::new(a_bind),
                            Box::new(Hoon::TisLus(
                                Box::new(b_bind),
                                Box::new(Hoon::TisLus(
                                    Box::new(c_bind),
                                    Box::new(Hoon::BarDot(
                                        Box::new(cnls),
                                    ))
                                ))
                            ))
                        )
                    }
                }
            };

            let tail = loop_tail(p, q);

            Hoon::TisGar(
                Box::new(Hoon::KetTis(Skin::Term("$".to_string()), Box::new(Hoon::Axis(1)))),
                Box::new(tail),
            )
        },

        Hoon::MicMic(spec, q) => Hoon::CenHep(
            Box::new(factory(*spec, 1, Vec::new(), HashMap::new(), Vec::new(), None, None)),
            q,
        ),

        Hoon::TisBar(spec, q) => Hoon::TisLus(Box::new(Hoon::KetTar(spec)), q),

        Hoon::TisTar((term, spec_opt), p, q) => {
            let inner = match spec_opt {
                None => *p,
                Some(spec_box) => Hoon::KetHep(spec_box, p),
            };
            let mut m = HashMap::new();
            m.insert(term, Some(inner));
            Hoon::TisGal(q, Box::new(Hoon::Tune(TermOrTune::Tune((m, vec![])))))
        }

        Hoon::TisCol(pairs, q) => {
            let wing = vec![Limb::Term("$".to_string())];
            Hoon::TisGar(Box::new(Hoon::CenCab(wing, pairs)), q)
        }

        Hoon::TisFas(skin, p, q) => Hoon::TisLus(Box::new(Hoon::KetTis(skin, p)), q),

        Hoon::TisMic(skin, p, q) => Hoon::TisFas(skin, q, p),

        Hoon::TisDot(wing, p, q) => Hoon::TisGar(Box::new(Hoon::CenCab(vec![Limb::Axis(1)], vec![(wing, *p)])), q),

        Hoon::TisWut(wing, p, q, r) => {
            let wtcl = Hoon::WutCol(p, q, Box::new(Hoon::Wing(wing.clone())));
            Hoon::TisDot(wing, Box::new(wtcl), r)
        }

        Hoon::TisGal(p, q) => Hoon::TisGar(q, p),

        Hoon::TisHep(p, q) => Hoon::TisLus(q, p),

        Hoon::TisKet(skin, wing, p, q) => {
            let wuy = weld(wing.clone(), vec![Limb::Term("v".to_string())]);
            let v_bind =
                Hoon::KetTis(Skin::Term("v".to_string()),  Box::new(Hoon::Axis(1)));
            let a_bind =
                Hoon::KetTis(Skin::Term("a".to_string()),
                        Box::new(Hoon::TisGar(
                            Box::new(Hoon::Limb("v".to_string())), p.clone())));
            let tsdt =
                Box::new(Hoon::TisDot(
                    wuy.clone(),
                    Box::new(Hoon::TisGal(
                        Box::new(Hoon::Axis(3)),
                        Box::new(Hoon::Limb("a".to_string())),
                    )),
                    Box::new(Hoon::TisGar(
                        Box::new(Hoon::Pair(
                            Box::new(Hoon::KetTis(
                                Skin::Over(vec![Limb::Term("v".to_string())],  Box::new(skin)),
                                Box::new(Hoon::TisGal(
                                    Box::new(Hoon::Axis(2)),
                                    Box::new(Hoon::Limb("a".to_string())),
                                )))),
                            Box::new(Hoon::Limb("v".to_string())),
                        )),
                        q
                    )),
                ));
            Hoon::TisGar(
                Box::new(v_bind),
                Box::new(Hoon::TisLus(
                    Box::new(a_bind),
                    tsdt,
                )))
        }

        Hoon::TisLus(p, q) => Hoon::TisGar(Box::new(
                                            Hoon::Pair(p,
                                                        Box::new(Hoon::Axis(1)))),
                                                    q),

        Hoon::TisSig(hoons) => {
            match hoons.as_slice() {
                [] => Hoon::Axis(1),
                [h] => h.clone(),
                [h, tail @ ..] => {
                    let rest = open(Hoon::TisSig(tail.to_vec()));
                    Hoon::TisGar(Box::new(h.clone()), Box::new(rest))
                }
            }
        }
        Hoon::WutBar(p) => {
            match p.as_slice() {
                [] => Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(1))),
                [head, tail @ ..] => {
                    let recurse = open(Hoon::WutBar(tail.to_vec()));
                    Hoon::WutCol(
                        Box::new(head.clone()),
                        Box::new(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(0)))),
                        Box::new(recurse),
                    )
                }
            }
        },

    Hoon::WutDot(p, q, r) => {
        Hoon::WutCol(
            Box::new(*p),
            r,
            q,
        )
    },

    Hoon::WutGal(p, q) => {
        Hoon::WutCol(
            Box::new(*p),
            Box::new(Hoon::ZapZap),
            q,
        )
    },

    Hoon::WutGar(p, q) => {
        Hoon::WutCol(
            Box::new(*p),
            q,
            Box::new(Hoon::ZapZap),
        )
    },

    Hoon::WutKet(p, q, r) => {
        let wuttis = Hoon::WutTis(
            Box::new(Spec::Base(BaseType::Atom("$".to_string()))),
            p,
        );
        Hoon::WutCol(
            Box::new(wuttis),
            r,
            q,
        )
    },

    Hoon::WutHep(p, q) => {
        match q.as_slice() {
            [] => {
                Hoon::Lost(Box::new(Hoon::Wing(p)))
            }
            [(spec, head), tail @ ..] => {
                let wtts = Hoon::WutTis(Box::new(spec.clone()), p.clone());
                let recurse = open(Hoon::WutHep(p.clone(), tail.to_vec()));
                Hoon::WutCol(
                    Box::new(wtts),
                    Box::new(head.clone()),
                    Box::new(recurse),
                )
            }
        }
    },

    Hoon::WutLus(p, q, r) => {
        let mut new_r = r.clone();
        new_r.push((Spec::Base(BaseType::Noun), *q));
        Hoon::WutHep(p, new_r)
    },

    Hoon::WutPam(p) => {
        match p.as_slice() {
            [] => Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(0))),
            [head, tail @ ..] => {
                let recurse = open(Hoon::WutPam(tail.to_vec()));
                Hoon::WutCol(
                    Box::new(head.clone()),
                    Box::new(recurse),
                    Box::new(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(1)))),
                )
            }
        }
    },

    Hoon::Xray(manx) => {
        let open_mane = match &manx.g.n {
            Mane::Tag(s) => Hoon::Rock("tas".to_string(), Noun::Atom(string_to_atom(s.clone()))),
            Mane::TagSpace(a, b) => {
                let left = Hoon::Rock("tas".to_string(), Noun::Atom(string_to_atom(a.clone())));
                let right = Hoon::Rock("tas".to_string(), Noun::Atom(string_to_atom(b.clone())));
                Hoon::Pair(Box::new(left), Box::new(right))
            }
        };

        let clsg_items: Vec<Hoon> = manx.g.a
            .iter()
            .map(|(mane, beers)| {
                let n_hoon = match &mane {
                    Mane::Tag(s) => Hoon::Rock("tas".to_string(), Noun::Atom(string_to_atom(s.clone()))),
                    Mane::TagSpace(a, b) => {
                        let left = Hoon::Rock("tas".to_string(), Noun::Atom(string_to_atom(a.clone())));
                        let right = Hoon::Rock("tas".to_string(), Noun::Atom(string_to_atom(b.clone())));
                        Hoon::Pair(Box::new(left), Box::new(right))
                    }
                };
                let woofs: Vec<Woof> = beers
                    .iter()
                    .map(|b| match b {
                        Beer::Char(cord) => Woof::Atom(string_to_atom(cord.clone())),
                        Beer::Hoon(hoon) => Woof::Hoon(hoon.clone()),
                    })
                    .collect();

                Hoon::Pair(
                    Box::new(n_hoon),
                    Box::new(Hoon::Knit(woofs)),
                )
            })
            .collect();

        let clsg = Hoon::ColSig(clsg_items);
        let head = Hoon::Pair(Box::new(open_mane), Box::new(clsg));
        let tail = Hoon::MicTis(manx.c);

        Hoon::Pair(Box::new(head), Box::new(tail))
    },

    Hoon::WutPat(p, q, r) => {
        let wtts = Hoon::WutTis(
            Box::new(Spec::Base(BaseType::Atom("$".to_string()))),
            p,
        );
        Hoon::WutCol(
            Box::new(wtts),
            q,
            r,
        )
    },

    Hoon::WutSig(p, q, r) => {
        let wtts = Hoon::WutTis(
            Box::new(Spec::Base(BaseType::Null)),
            p,
        );
        Hoon::WutCol(
            Box::new(wtts),
            q,
            r,
        )
    },

    Hoon::WutTis(spec, q) => {
        let example_res = example(&spec, 1, &Vec::new(), &HashMap::new(), &Vec::new(), &None, &None);
        Hoon::Fits(
            Box::new(example_res),
            q,
        )
    },

    Hoon::WutZap(p) => {
        Hoon::WutCol(
            p,
            Box::new(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(1)))),
            Box::new(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(0)))),
        )
    },

    Hoon::ZapGar(p) => {
        let limb_onan = Hoon::Limb("onan".to_string());
        let limb_abel = Hoon::Limb("abel".to_string());
        let bcmc = Spec::BucMic(limb_abel);
        let kttr = Hoon::KetTar(Box::new(bcmc));
        let zpmc = Hoon::ZapMic(Box::new(kttr), p);

        Hoon::CenCol(Box::new(limb_onan), vec![zpmc])
    },

    Hoon::ZapWut(arg, q) => {
        const HOON_VERSION: u64 = 138;  // hardcoded...

        let version_ok = match &arg {
            ZpwtArg::Atom(s) => {
                s.parse::<u64>().map_or(false, |v| HOON_VERSION <= v)
            }
            ZpwtArg::Pair(min_s, max_s) => {
                match (min_s.parse::<u64>(), max_s.parse::<u64>()) {
                    (Ok(min), Ok(max)) => min <= HOON_VERSION && HOON_VERSION <= max,
                    _ => false,
                }
            }
        };

        if version_ok {
            *q
        } else {
            panic!("hoon-version")
        }
    },

        _ => gen,
    }
}

pub fn chum_to_noun(chum: Chum) -> Noun {
    match chum {
        Chum::Lef(term) => {
            Noun::Atom(string_to_atom(term))
        }
        Chum::StdKel(term, u) => {
            Noun::Cell(
                Box::new(Noun::Atom(string_to_atom(term))),
                Box::new(Noun::Atom(u)),
            )
        }
        Chum::VenProKel(t1, t2, u) => {
            Noun::Cell(
                Box::new(Noun::Atom(string_to_atom(t1))),
                Box::new(Noun::Cell(
                    Box::new(Noun::Atom(string_to_atom(t2))),
                    Box::new(Noun::Atom(u)),
                )),
            )
        }
        Chum::VenProVerKel(t1, t2, u1, u2) => {
            Noun::Cell(
                Box::new(Noun::Atom(string_to_atom(t1))),
                Box::new(Noun::Cell(
                    Box::new(Noun::Atom(string_to_atom(t2))),
                    Box::new(Noun::Cell(
                        Box::new(Noun::Atom(u1)),
                        Box::new(Noun::Atom(u2)),
                    )),
                )),
            )
        }
    }
}

pub fn flay(gen: Hoon) -> Option<Skin> {
    match gen {
        Hoon::Pair(p, q) => {
            let maybe_p = flay(*p);
            let maybe_q = flay(*q);
            match (maybe_p, maybe_q) {
                (Some(p), Some(q)) => Some(Skin::Cell(Box::new(p), Box::new(q))),
                _ => None,
            }
        }

        Hoon::Base(b) => Some(Skin::Base(b.clone())),

        Hoon::Rock(t, n) => {
            match n {
                Noun::Atom(a) => Some(Skin::Leaf(t.to_string(), a)),
                Noun::Cell(_, _) => None,
            }
        }

        Hoon::CenTis(w, l) => {
            match (w, l) {
                (v, l) if l.is_empty() => match v.as_slice() {
                    [Limb::Term(t)] => Some(Skin::Term((*t).to_string())),
                    _ => None,
                },
                _ => None,
            }
        }

        Hoon::TisGar(p, q) => {
            let maybe_wing = reek(*p);
            match maybe_wing {
                Some(w) => {
                    let skin = flay(*q);
                    match skin {
                        None => None,
                        Some(s) => Some(Skin::Over(w, Box::new(s))),
                    }
                }
                None => None,
            }
        }

        Hoon::Limb(t) => {
            Some(Skin::Term(t.to_string()))
        }

        Hoon::Wing(w) => {
            match w.as_slice() {
                [Limb::Term(t)] => Some(Skin::Term(t.clone())),
                _ => {
                    fn recur(w: &[Limb]) -> Option<Skin> {
                        match w {
                            [] => Some(Skin::Wash(0)),
                            [Limb::Parent(0, None), rest @ ..] => recur(rest),
                            _ => None,
                        }
                    }
                    recur(w.as_slice())
                }
            }
        }

        Hoon::KetTar(s) => {
            Some(Skin::Spec(s.clone(), Box::new(Skin::Base(BaseType::Noun))))
        }

        Hoon::KetTis(skin, h) => {
            let maybe_skin = flay(*h);
            match maybe_skin {
                Some(s) => {
                    match s {
                        Skin::Term(t) => Some(Skin::Name(t, Box::new(skin.clone()))),
                        Skin::Name(ref t, ref b) // Borrow t and b
                            if matches!(**b, Skin::Base(BaseType::Noun)) => {
                            Some(Skin::Name(t.clone(), Box::new(s))) // Clone t if needed
                        },
                        _ => None,
                    }
                }
                None => None,
            }
        }

        _ => {
            let desugared = open(gen.clone());
            if desugared == gen {
                None
            } else {
                flay(desugared)
            }
        }

    }
}

pub fn feck(gen: Hoon) -> Option<Atom> {
    match gen {
        Hoon::Sand(term, noun) => {
            if term == "tas" {
                match noun {
                    Noun::Atom(s) => Some(s),
                    Noun::Cell(_, _) => None,
                }
            } else {
                None
            }
        }

        Hoon::Dbug(_spot, expr) => feck(*expr),

        _ => None,
    }
}

pub fn grip(skin: Skin, gen: Hoon, rel: WingType) -> Hoon {
    match skin {
        Skin::Term(term) => {
            Hoon::TisGal(
                Box::new(Hoon::Tune(TermOrTune::Term(term))),
                Box::new(gen),
            )
        }

        Skin::Base(base) => {
            if base == BaseType::Noun {
                gen
            } else {
                Hoon::KetHep(
                    Box::new(Spec::Base(base)),
                    Box::new(gen),
                )
            }
        }

        Skin::Cell(car_skin, cdr_skin) => {
            let haf = half(gen.clone());
            match haf {
                None => {
                    let car_gen = Hoon::Axis(4);
                    let cdr_gen = Hoon::Axis(5);
                    let pair = Hoon::Pair(
                        Box::new(grip(*car_skin, car_gen, rel.clone())),
                        Box::new(grip(*cdr_skin, cdr_gen, rel.clone())),
                    );
                    Hoon::TisLus(Box::new(gen), Box::new(pair))
                }
                Some((p, q)) => {
                    Hoon::Pair(
                        Box::new(grip(*car_skin, p, rel.clone())),
                        Box::new(grip(*cdr_skin, q, rel.clone())),
                    )
                }
            }
        }

        Skin::Dbug(spot, inner_skin) => {
            Hoon::Dbug(
                spot,
                Box::new(grip(*inner_skin, gen, rel)),
            )
        }

        Skin::Leaf(aura, atom) => {
            Hoon::KetHep(
                Box::new(Spec::Leaf(aura, atom)),
                Box::new(gen),
            )
        }

        Skin::Name(term, inner_skin) => {
            Hoon::TisGal(
                Box::new(Hoon::Tune(TermOrTune::Term(term))),
                Box::new(grip(*inner_skin, gen, rel)),
            )
        }

        Skin::Over(mut wing, inner_skin) => {
            wing.extend(rel);
            grip(*inner_skin, gen, wing)
        }

        Skin::Spec(spec, inner_skin) => {
            let check_skin = if rel.is_empty() {
                spec
            } else {
                Box::new(Spec::Over(rel.clone(), spec))
            };

            let inner = grip(*inner_skin, gen, rel);

            Hoon::KetHep(
                check_skin,
                Box::new(inner),
            )
        }

        Skin::Wash(depth) => {
            let wing: WingType = (0..depth)
                    .map(|_| Limb::Parent(0, None))
                    .collect();
            Hoon::TisGal(
                Box::new(Hoon::Wing(wing)),
                Box::new(gen),
            )
        }
    }
}

pub fn half(gen: Hoon) -> Option<(Hoon, Hoon)> {
    match gen {
         Hoon::Pair(car, cdr) => {
            Some((*car, *cdr))
        }

        Hoon::Dbug(_spot, expr) => {
            half(*expr)
        }

        Hoon::ColCab(car, cdr) => {
            Some((*cdr, *car))
        }

        Hoon::ColHep(car, cdr) => {
            Some((*car, *cdr))
        }

        Hoon::ColKet(a, b, c, d) => {
            let tail = Hoon::ColLus(b, c, d);
            Some((*a, tail))
        }

        Hoon::ColSig(mut items) => {
            if items.is_empty() {
                None
            } else {
                let head = items.remove(0);
                Some((head, Hoon::ColSig(items)))
            }
        }

        Hoon::ColTar(mut items) => {
            if items.is_empty() {
                None
            } else if items.len() == 1 {
                half(items.remove(0))
            } else {
                let head = items.remove(0);
                let tail = Hoon::ColTar(items);
                Some((head, tail))
            }
        }

        _ => None,
    }
}

pub fn reek(gen: Hoon) -> Option<WingType> {
    match gen {
        Hoon::Pair(p, _q) => {
            match *p {
                Hoon::Axis(a) => Some(vec![Limb::Axis(a)]),
                _ => None,
            }
        }
        Hoon::Limb(t) => Some(vec![Limb::Term(t.clone())]),
        Hoon::Wing(w) => Some(w.to_vec()),
        Hoon::Dbug(_s, h) => reek(*h),
        _ => None
    }
}

pub fn name_ax(gen: Hoon) ->  Option<String> {
    match gen {
        Hoon::Wing(p) => {
            if p.is_empty() {
                None
            } else if let Some(i) = p.first() {
                match i {
                    Limb::Axis(_) => None,
                    Limb::Term(q) =>  Some(q.to_string()),
                    Limb::Parent(_, q) => q.clone(),
                }
            } else {
                None
            }
        }
        Hoon::Limb(p) => Some(p),
        Hoon::Dbug(_, q) => name_ax(*q),
        Hoon::TisGal(p, q) => name_ax(Hoon::TisGar(q, p)),
        Hoon::TisGar(_, q) => name_ax(*q),
        _ => None
    }
}

pub fn autoname(mod_spec: Spec) -> Option<String> {  //  ++autoname:ax
    match mod_spec {
        Spec::Base(base) => match base {
            BaseType::Atom(aura) => {
                if aura == "$" {    //  how empty terms will be represented here in rust land?...
                    Some("atom".to_string())
                } else {
                    Some(aura)
                }
            }
            _ => None,
        },
        Spec::Dbug(_, q) => autoname(*q),
        Spec::Leaf(p, _) => Some(p),
        Spec::Loop(p) => Some(p),
        Spec::Like(wing, _list_wing) => {
            if wing.is_empty() {
                None
            } else if let Some(i) = wing.first() {
                match i {
                    Limb::Axis(_) => None,
                    Limb::Term(q) =>  Some(q.to_string()),
                    Limb::Parent(_, q) => q.clone(),
                }
            } else {
                None
            }
        },
        Spec::Make(p, _) => name_ax(p),
        Spec::Made(_, q) => autoname(*q),
        Spec::Name(_, q) => autoname(*q),
        Spec::Over(_, q) => autoname(*q),
        Spec::BucBuc(p, _) => autoname(*p),
        Spec::BucBar(p, _) => autoname(*p),
        Spec::BucCab(p) => name_ax(p),
        Spec::BucCol(i, _) => autoname(*i),
        Spec::BucCen(i, _) => autoname(*i),
        Spec::BucDot(_, _) => None,
        Spec::BucGal(_, q) => autoname(*q),
        Spec::BucGar(_, q) => autoname(*q),
        Spec::BucHep(p, _) => autoname(*p),
        Spec::BucKet(_, q) => autoname(*q),
        Spec::BucLus(_, q) => autoname(*q),
        Spec::BucFas(_, _) => None,
        Spec::BucMic(p) => name_ax(p),
        Spec::BucPam(p, _) => autoname(*p),
        Spec::BucSig(_, q) => autoname(*q),
        Spec::BucTic(_, _) => None,
        Spec::BucTis(_, q) => autoname(*q),
        Spec::BucPat(_, q) => autoname(*q),
        Spec::BucWut(i, _) => autoname(*i),
        Spec::BucZap(_, _) => None,
    }
}

pub fn decorate(gen: Hoon, bug: Vec<Spot>, nut: Option<Note>) -> Hoon {
    let mut out = gen;

    for spot in bug.into_iter().rev() {
        out = Hoon::Dbug(spot, Box::new(out));
    }

    match nut {
        None => out,
        Some(note) => Hoon::Note(note, Box::new(out)),
    }
}

pub fn blue(tik: Tiki, gen: Hoon) -> Hoon {
    match tik {
        Tiki::Hoon((None, h)) => gen,
        _ =>  Hoon::TisGar(Box::new(Hoon::Axis(3)), Box::new(gen)),
    }
}

pub fn teal(tik: Tiki, mod_: Spec) -> Spec {
    match tik {
        Tiki::Wing((_, _)) => mod_,
        Tiki::Hoon((_, _)) => Spec::Over(vec![Limb::Axis(3)], Box::new(mod_)),
    }
}

pub fn tele(tik: Tiki, syn: Skin) -> Skin {
    match tik {
        Tiki::Wing((_, _)) => syn,
        Tiki::Hoon((_, _)) => Skin::Over(vec![Limb::Axis(3)], Box::new(syn)),
    }
}

pub fn gray(tik: Tiki, gen: Hoon) -> Hoon {
    match tik {
        Tiki::Wing((p, q)) => {
            match p {
                None => gen,
                Some(u) => Hoon::TisTar((u, None),
                                        Box::new(Hoon::Wing(q)),
                                        Box::new(gen)),
            }
        }
        Tiki::Hoon((p, q)) => {
            let arg = match p {
                None => q,
                Some(u) => Box::new(Hoon::KetTis(Skin::Term(u), q)),
            };
            Hoon::TisLus(arg, Box::new(gen))
        }
    }
}

pub fn puce(tik: Tiki) -> WingType {
    match tik {
        Tiki::Wing((p, q)) => match p {
            None => q,
            Some(u) => vec![Limb::Term(u)],
        },
        Tiki::Hoon((_, _)) => vec![Limb::Axis(2)],
    }
}

pub fn wthp(tik: Tiki, opt: Vec<(Spec, Hoon)>) -> Hoon {
    let mapped = opt.into_iter()
                .map(|(a, b)| (a, blue(tik.clone(), b)))
                .collect::<Vec<(Spec, Hoon)>>();
    gray(tik.clone(), Hoon::WutHep(puce(tik.clone()), mapped))
}

pub fn wtkt(tik: Tiki, sic: Hoon, non: Hoon) -> Hoon {
    gray(tik.clone(), Hoon::WutKet(puce(tik.clone()),
              Box::new(blue(tik.clone(), sic)),
              Box::new(blue(tik.clone(), non))))
}

pub fn wtls(tik: Tiki, gen: Hoon, opt: Vec<(Spec, Hoon)>) -> Hoon {
    let mapped = opt.into_iter()
                .map(|(a, b)| (a, blue(tik.clone(), b)))
                .collect::<Vec<(Spec, Hoon)>>();
    gray(tik.clone(), Hoon::WutLus(puce(tik.clone()), Box::new(blue(tik.clone(), gen)), mapped))
}

pub fn wtpt(tik: Tiki, sic: Hoon, non: Hoon) -> Hoon {
    gray(tik.clone(), Hoon::WutPat(puce(tik.clone()),
                            Box::new(blue(tik.clone(), sic)),
                            Box::new(blue(tik.clone(), non))))
}

pub fn wtsg(tik: Tiki, sic: Hoon, non: Hoon) -> Hoon {
    gray(tik.clone(), Hoon::WutSig(puce(tik.clone()),
                            Box::new(blue(tik.clone(), sic)),
                            Box::new(blue(tik.clone(), non))))
}

pub fn wthx(tik: Tiki, syn: Skin) -> Hoon {
    gray(tik.clone(), Hoon::WutHax(tele(tik.clone(), syn), puce(tik.clone())))
}

pub fn wtts(tik: Tiki, mod_: Spec) -> Hoon {
    gray(tik.clone(), Hoon::WutTis(Box::new(teal(tik.clone(), mod_)), puce(tik.clone())))
}

pub fn right_child(n: u64) -> u64 {
    if n == 0 {
        1
    } else {
        (2 * right_child(n - 1)) + 1
    }
}

pub fn left_child(n: u64) -> u64 {
    if n == 0 {
        0
    } else {
        2 * (left_child(n - 1) + 1)
    }
}

pub fn peg(a: u64, b: u64) -> Result<u64, &'static str> {
    if a == 0 || b == 0 {
        return Err("peg: a and b must be non-zero");
    }

    let k = b.ilog2();
    let offset = b & ((1u64 << k) - 1);
    Ok((a << k) + offset)
}

// pub fn autoname(mod_spec: Spec) -> Option<String> {  //  ++autoname:ax

// }

pub fn digit<'src>(
) -> impl Parser<'src, &'src str, char, Err<'src>>
{
    one_of("0123456789")
}

pub fn non_zero_digit<'src>(
) -> impl Parser<'src, &'src str, char, Err<'src>>
{
    one_of("0123456789")
}

pub fn lowercase<'src>(
) -> impl Parser<'src, &'src str, char, Err<'src>>
{
    one_of("abcdefghijklmnopqrstuvwxyz")
}

pub fn uppercase<'src>(
) -> impl Parser<'src, &'src str, char, Err<'src>>
{
    one_of("ABCDEFGHIJKLMNOPQRSTUVWXYZ")
}

// non-control ASCII (32-255, excluding 127/DEL)
fn non_control_char<'src>(
) -> impl Parser<'src, &'src str, char, Err<'src>> {
    any().filter(|c: &char| {
        let code = *c as u32;
        (code >= 0x20 && code < 0x7F) || code >= 0x80
    }).labelled("Non-Control Character")
}

fn gah<'src>() -> impl Parser<'src, &'src str, (), Err<'src>> {
    choice((
        just(' ').ignored(),
        newline(),
    ))
    .labelled("Space or NewLine")

}

// comments
fn vul<'src>() -> impl Parser<'src, &'src str, (), Err<'src>> {
    just("::")
        .ignore_then(non_control_char().repeated())
        .ignore_then(newline())
        .ignored()
        .labelled("Comments")
}

fn gaq<'src>() -> impl Parser<'src, &'src str, (), Err<'src>> {
    choice((
        newline().ignored(),

        gah().ignore_then(
                choice((gah().ignored(),
                        vul(),
                        ))
        ),

        vul(),
    ))
    .ignored()
}

pub fn gap<'src>(
) -> impl Parser<'src, &'src str, (), Err<'src>>
{
    gaq()
    .then_ignore(
        choice((
            vul(),
            gah().ignored(),
        ))
        .repeated()
        .or_not()
    )
    .ignored()
    .labelled("Gap")
}

pub fn list_term_hoon<'src>(
    hoon: impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<(String, Hoon)>, Err<'src>>
{
    symbol()
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .repeated()
    .at_least(1)
    .collect::<Vec<(String, Hoon)>>()
}

pub fn list_names_wide<'src>(
) -> impl Parser<'src, &'src str, Vec<String>, Err<'src>>
{
    symbol()
    .separated_by(just(' '))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just("["), just("]"))
}

pub fn winglist<'src>(
) -> impl Parser<'src, &'src str, WingType, Err<'src>>
{
    let name =      //  Name or $
        just('$')
            .to("$".to_string())
            .or(symbol());

    let com =   //  ,
        just(',')
        .to(Limb::Axis(0));

    let ket_name =   //  ^^name or name
        just('^')
            .repeated()
            .count()
            .then(name)
            .map(|(cnt, name)| {
                if cnt == 0 {
                    return Limb::Term(name);
                } else {
                    return Limb::Parent(cnt as u64, Some(name));
                }
            });

    let lus_number =   //  +10
            just('+').ignore_then(regex(r"[0-9]+"))
                .map(|n: &str| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(num)}
                );

    let pam_number =   //  &10
            just('&').ignore_then((regex(r"[0-9]+")))
                .map(|n: &str| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(left_child(num))
                });

    let bar_number =  //  |10
           just('|').ignore_then(regex(r"[0-9]+"))
                .map(|n: &str| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(right_child(num))
                });

    let dot =  //  .
            just('.').to(Limb::Axis(1));

    let lus =  //  +
        just('+').to(Limb::Axis(3));

    let hep =  //  -
        just('-').to(Limb::Axis(2));

    let lark =   //    +>-<  notation
            regex(r"[+-][<>](?:[+-][<>])*[+-]?")
            .map(|s: &str| {
                let str = s.to_string();
                let mut axis = 1;
                for c in str.chars() {
                    match c {
                        '+' | '>' => axis = peg(axis, 3).unwrap(),
                        '-' | '<' => axis = peg(axis, 2).unwrap(),
                        _ => axis = 1,
                    }
                }
                Limb::Axis(axis)
            }).labelled("Lark Expression");

    choice((
        com,
        ket_name,
        lus_number,
        pam_number,
        bar_number,
        lark,
        dot,
        lus,
        hep,
    )).separated_by(just('.'))
        .at_least(1)
        .collect::<Vec<_>>()
        .labelled("Wing")
}

pub fn variable_name_and_type<'src>(
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Skin, Err<'src>>
{
    let not_named = just('=')  // =/  =foo
        .ignore_then(spec_wide.clone())
        .try_map(|spec, span| {
            let auto = autoname(spec.clone());
             match auto {
                        None => Err(Rich::custom(span, "cannot autoname")),
                        Some(term) => {
                            Ok(Skin::Name(
                              term,
                                Box::new(Skin::Spec(
                                    Box::new(spec),
                                    Box::new(Skin::Base(BaseType::Noun)),
                                )),
                            ))
                        }
                    }
        });

     let named = symbol()    //  =/  a=foo  ,  =/  a
        .then_ignore(just('/').or(just('=')))
        .then(
            spec_wide.clone()
                .or_not() // handle foo or foo=bar
        )
        .map(|(term, maybe_spec)|
            match maybe_spec {
                None => Skin::Term(term),
                Some(spec) => Skin::Name(
                    term,
                    Box::new(Skin::Spec(
                        Box::new(spec),
                        Box::new(Skin::Base(BaseType::Noun)),
                    )),
                ),
        });

    let just_type = spec_wide.clone() // =/  type
        .map(|s| Skin::Spec(Box::new(s), Box::new(Skin::Base(BaseType::Noun))));

    choice((not_named, named, just_type))
}

// ++  si                                                  ::  signed integer
pub fn syn_si(a: u128) -> bool {
    end_u128(0, 1, a) == 0
}

pub fn abs_si(a: u128) -> u128 {
    let rsh_res = rsh_u128(0, 1, a);
    let end_res = end_u128(0, 1, a.clone());
    end_res + rsh_res
}

pub fn old_si(a: u128) -> (bool, u128) {
    (syn_si(a), abs_si(a))
}
pub fn new_si(sign: bool, mag: u128) -> u128 {
    if mag == 0 { 0 } else if sign { mag << 1 } else { (mag << 1) - 1 }
}
fn sun_si(a: u128) -> u128 {
    a << 1
}

pub fn sum_si(a: u128, b: u128) -> u128 {
    let (c_sign, c_mag) = old_si(a);
    let (d_sign, d_mag) = old_si(b);
    match (c_sign, d_sign) {
        (false, false) => new_si(false, c_mag.wrapping_add(d_mag)),
        (false, true) => {
            if c_mag >= d_mag { new_si(false, c_mag - d_mag) }
            else { new_si(true, d_mag - c_mag) }
        },
        (true, false) => {
            if c_mag >= d_mag { new_si(true, c_mag - d_mag) }
            else { new_si(false, d_mag - c_mag) }
        },
        (true, true) => new_si(true, c_mag.wrapping_add(d_mag)),
    }
}

pub fn dif_si(a: u128, b: u128) -> u128 {
    let (b_sign, b_mag) = old_si(b);
    let neg_b = new_si(!b_sign, b_mag);
    sum_si(a, neg_b)
}

// swr: swap rounding direction for negative numbers
pub fn swr(r: char) -> char {
    match r {
        'd' => 'u',
        'u' => 'd',
        _ => r,
    }
}

// fli: flip sign of BinaryFloat
pub fn fli(a: BinaryFloat) -> BinaryFloat {
    match a {
        BinaryFloat::Finite { sign, exp, mant } => BinaryFloat::Finite { sign: !sign, exp, mant },
        BinaryFloat::Infinity { sign } => BinaryFloat::Infinity { sign: !sign },
        BinaryFloat::NaN => BinaryFloat::NaN,
    }
}

// zer: zero float node
pub fn zer() -> BinaryFloat {
    BinaryFloat::Finite {
        sign: false,
        exp: 0, // si-encoding of 0 is 0
        mant: BigUint::from(0u8),
    }
}

fn rau(e: u128, a: BigUint, t: bool, p: u128, v: u128, w: u128, r: char, d: char) -> BinaryFloat {
    let mode = match r {
        'z' | 'd' => LugMode::Floor,
        'a' | 'u' => LugMode::Ceiling,
        'n'       => LugMode::Nearest,
        _         => LugMode::Nearest,
    };

    lug(mode, e, a, t, p, v, w, r, d)
}

pub fn cmp_si(a: u128, b: u128) -> u128 {
    if a == b {
        0
    } else if syn_si(a) {
        if syn_si(b) {
            if a > b { 2 } else { 1 }
        } else {
            2
        }
    } else if syn_si(b) {
        1
    } else {
        if a  >  b { 1 } else { 2 }
    }
}

#[derive(Debug, Clone, Copy)]
enum LugMode {
    Floor,     // %fl
    Ceiling,   // %ce
    Smaller,   // %sm
    Larger,    // %lg
    Nearest,   // %ne  (ties to even)
    NearestAway,
    NearestTowards,
}

fn sub_or_panic(mut a: u128, b: u128) -> u128 {
    a = a.checked_sub(b).expect("subtraction underflow");
    a
}

fn sub_or_panic_big(a: &BigUint, b: &BigUint) -> BigUint {
    if a < b {
        panic!("subtraction underflow");
    }
    a - b
}

fn prc(p: u128) -> u128 {
    assert!(p > 1, "precision should be >= 2");
    p
}

fn lug(mode: LugMode, mut e: u128, mut a: BigUint, s: bool, p: u128, v: u128, w: u128, r: char, d: char) -> BinaryFloat {

    use BinaryFloat::*;
    use LugMode::*;

    if a == BigUint::zero() { panic!("lug: mantissa zero"); }

    let m = met(0, &Atom::Big(a.clone())) as u128;
    let prc_res = prc(p);
    assert!(s | (m > prc_res), "lug: stick bit is false or precision is invalid");

    let max_p = if m > prc_res {
        sub_or_panic(m as u128, prc_res)
    } else {
        0
    };

    let max_q = {
        let abs_arg = if d == 'i' {
            0
        } else if cmp_si(e, v) == 1  {
            dif_si(v, e)
        } else {
            0
        };
        abs_si(abs_arg)
    };

    let q = max_p.max(max_q);

    let b = end_big(0, q as usize, &a).to_u128().expect("value too large for u128");

    a = rsh(0, q as usize, &Atom::Big(a)).to_biguint();

    e = sum_si(e, sun_si(q));

    if a == BigUint::zero() {
        assert!(d != 'i', "lug: d == %i");
        return match mode {
            Floor | Smaller => Finite { sign: true, exp: 0, mant: BigUint::zero() },
            Ceiling | Larger => {
                Finite { sign: true, exp: v, mant: BigUint::one() }
            },
            Nearest | NearestTowards => {
                let half = bex(q.saturating_sub(1));
                if s {
                    if b <= half {
                       return Finite { sign: true, exp: 0, mant: BigUint::zero() };
                    }
                    return Finite { sign: true, exp: v, mant: BigUint::one() };
                }
                if b < half {
                    return Finite { sign: true, exp: 0, mant: BigUint::zero() };
                }
                return Finite { sign: true, exp: v, mant: BigUint::one() };
            },
            NearestAway => {
                let half = bex(q.saturating_sub(1));
                if b < half {
                    return Finite { sign: true, exp: 0, mant: BigUint::zero() };
                }
                return Finite { sign: true, exp: v, mant: BigUint::one() };
            }
        };
    }

    (e, a) = xpd(e, a, d, p, v);

    match mode {
        Floor => { /* no change */ }
        Larger => a = a + BigUint::one(),
        Smaller => {
            if b == 0 && s {
                if e == v && d != 'i' {
                    a = sub_or_panic_big(&a, &BigUint::one());
                } else {
                    let y = sub_or_panic_big(&(a.clone() * BigUint::from(2 as u128)), &BigUint::one());
                    if met_big(0, &y) as u128 <= prc_res {
                        a = y;
                        e = dif_si(e, 2);
                    } else {
                        a = sub_or_panic_big(&a, &BigUint::one());
                    }
                }
            }
        },
        Ceiling => { if !(b == 0 && !s) { a = a + BigUint::one(); } },
        Nearest => {
            if b != 0 {
                let y = bex(sub_or_panic(q, 1));
                if b == y && s {
                    if dis_big(&a, &BigUint::one()) != BigUint::zero() {
                        a = a + BigUint::one();
                    }
                } else if b < y {
                } else {
                    a = a + BigUint::one();
                }
            }
        }
        NearestAway => {
            if b != 0 {
                let y = bex(sub_or_panic(q, 1));
                if !(b < y) {
                    a = a + BigUint::one();
                }
            }
        }
        NearestTowards => {
            if b != 0 {
                let y = bex(sub_or_panic(q, 1));
                if b == y {
                    if !s {
                        a = a + BigUint::one();
                    }
                }
                if !(b < y) {
                    a = a + BigUint::one();
                }
            }
        }
    };

    (e, a) = if (met_big(0, &a.clone()) as u128) != (prc_res + 1) {
        (e, a)
    } else {
        a = rsh(0, 1, &Atom::Big(a)).to_u128().expect("lug: cast failled").into();
        e = sum_si(e, 2);
        (e, a)
    };

    if a == BigUint::zero() {
        return Finite { sign: true, exp: 0, mant: BigUint::zero() };
    }

    let res = if d == 'i' {
        Finite { sign: true, exp: e, mant: BigUint::from(a) }
    } else if cmp_si(emx(v, w), e) == 1 {
        Infinity { sign: true }
    } else {
        Finite { sign: true, exp: e, mant: BigUint::from(a) }
    };

    if !(d == 'f') {
        return res;
    }

    match res {
        Finite { sign, exp, ref mant } => {
            if  met_big(0, &mant.clone()) as u128 == prc(p) {
               return Finite { sign: true, exp: 0, mant: BigUint::zero() };
            }
            res
        },
        _ => res,
    }
}

fn emx(v: u128, w: u128) -> u128 {
    sum_si(v, sun_si(w))
}

fn rou(e: u128, a: BigUint, p: u128, v: u128, w: u128, r: char, d: char) -> BinaryFloat {
    rau(e, a, true, p, v, w, r, d)
}

pub fn binaryfloat_mul_internal(a_e: u128, a_a: BigUint, b_e: u128, b_a: BigUint, p: u128, v: u128, w: u128, r: char, d: char) -> BinaryFloat {
    let e = sum_si(a_e, b_e);
    let a = a_a * b_a;
    rou(e, a, p, v, w, r, d)
}


pub fn binaryfloat_div_internal(
    a_e: u128,
    a_a: BigUint,
    b_e: u128,
    b_a: BigUint,
    p: u128,
    v_min: u128,
    w: u128,
    r: char,
    d: char,
) -> BinaryFloat {
    let ma = met_big(0, &a_a) as u128;
    let mb = met_big(0, &b_a) as u128;

    let rhs = sun_si(mb + prc(p) + 1);
    let v = dif_si(sun_si(ma), rhs);

    let (a_e_shifted, a_a_shifted) = if syn_si(v) {
        (a_e, a_a)
    } else {
        let shift = abs_si(v) as usize;
        let new_e = sum_si(v, a_e);
        let new_a = lsh(0, shift, &Atom::Big(a_a.clone())).to_biguint();
        (new_e, new_a)
    };

    let j = dif_si(a_e_shifted, b_e);
    let (quot, rem) = dvr_big(&a_a_shifted, &b_a);

    rau(j, quot, rem.is_zero(), p, v_min, w, r, d)
}

fn dvr_big(a: &BigUint, b: &BigUint) -> (BigUint, BigUint) {
    let quot = a / b;
    let rem  = a % b;
    (quot, rem)
}

pub fn bex(a: u128) -> u128 {
    if a == 0 {
        1
    } else {
        assert!(a < 128, "bex: exponent too large for u128");
        1u128 << a
    }
}

fn xpd(e: u128, a: BigUint, d: char, p: u128, v: u128) -> (u128, BigUint) {
    let ma = met_big(0, &a.clone()) as u128;

    if ma >= prc(p) {
        return (e, a);
    }

    let shift = if d == 'i' {
        sub_or_panic(prc(p), ma as u128)
    } else {
        let w = dif_si(e, v);
        let q = if syn_si(w) { abs_si(w) } else { 0 };
        let needed = sub_or_panic(prc(p), ma as u128);
        q.min(needed)
    };

    let e_new = dif_si(e, sun_si(shift));
    let a_new = lsh_big(0, shift as usize, &a);

    (e_new, a_new)
}

pub fn binaryfloat_mul(a: BinaryFloat, b: BinaryFloat, p: u128, v: u128, w: u128, mut r: char, d: char) -> BinaryFloat {
    use BinaryFloat::*;

    if matches!(a, NaN) || matches!(b, NaN) {
        return NaN;
    }

    if let Infinity { sign: sa } = a {
        if let Infinity { sign: sb } = b {
            return Infinity { sign: sa == sb };
        }

        let b_mant = if let Finite { ref mant, .. } = b { mant.clone() } else { BigUint::zero() };
        if b_mant == BigUint::zero() {
            return NaN;
        }
        return Infinity { sign: sa == b.sign() };
    }

    if let Infinity { sign: sb } = b {
        let a_mant = if let Finite { ref mant, .. } = a { mant.clone() } else { BigUint::zero() };
        if a_mant == BigUint::zero() {
            return NaN;
        }
        return Infinity { sign: a.sign() == sb };
    }

    let (sa, ea, ma) = if let Finite { sign, exp, mant } = a { (sign, exp, mant) } else { (false, 0, BigUint::zero()) };
    let (sb, eb, mb) = if let Finite { sign, exp, mant } = b { (sign, exp, mant) } else { (false, 0, BigUint::zero()) };

    if ma == BigUint::zero() || mb == BigUint::zero() {
        return Finite {
            sign: sa == sb, // =(s.a s.b)
            exp: 0, // zer = [e=--0 a=0]
            mant:  BigUint::zero(),
        };
    }

    if ma == BigUint::zero() || mb == BigUint::zero() {
        return binaryfloat_mul_internal(ea, ma, eb, mb, p, v, w, r, d);
    }
    r = swr(r);
    fli(binaryfloat_mul_internal(ea, ma, eb, mb, p, v, w, r, d))
}

pub fn binaryfloat_div(a: BinaryFloat, b: BinaryFloat, p: u128, v: u128, w: u128, mut r: char, d: char) -> BinaryFloat {
    use BinaryFloat::*;

    if matches!(a, NaN) || matches!(b, NaN) {
        return NaN;
    }

    if let Infinity { sign: sa } = a {
        if let Infinity { sign: sb } = b {
            return NaN;
        }
        return Infinity { sign: sa == b.sign() };
    }

    if let Infinity { sign: sb } = b {
        return Finite {
            sign: a.sign() == sb,
            exp: 0,     // zer = [e=--0 a=0]
            mant: BigUint::zero(),
        };
    }

    let (sa, ea, ma) = if let Finite { sign, exp, mant } = a { (sign, exp, mant) } else { (false, 0, BigUint::zero()) };
    let (sb, eb, mb) = if let Finite { sign, exp, mant } = b { (sign, exp, mant) } else { (false, 0, BigUint::zero()) };

    if ma == BigUint::zero() {
        if mb == BigUint::zero() {
            return NaN;
        }
        return Finite {
            sign: sa == sb,
            exp: 0,
            mant: BigUint::zero(),
        };
    }

    if mb == BigUint::zero() {
        return Infinity { sign: sa == sb };
    }

    if sa == sb {
        return binaryfloat_div_internal(ea, ma, eb, mb, p, v, w, r, d);
    }
    r = swr(r);
    fli(binaryfloat_div_internal(ea, ma, eb, mb, p, v, w, r, d))
}

pub fn pow(base: u128, exp: u128) -> BigUint {
    if exp == 0 {
        return BigUint::from(1u8);
    }

    let mut result = BigUint::from(1u8);
    let mut base = BigUint::from(base);
    let mut exp = exp;

    while exp > 0 {
        if exp & 1 == 1 {
            result *= &base;
        }
        base *= base.clone();
        exp >>= 1;
    }

    result
}

pub fn fil(a: u32, b: u32, c: u128) -> Atom {
    if b == 0 {
        return Atom::Small(0);
    }

    let bloq_bits = 1u32 << a; // 2^a bits per block
    let mask = if bloq_bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bloq_bits) - 1
    };
    let c_masked = c & mask;

    if bloq_bits as u64 * b as u64 <= 128 && c_masked != 0 {
        let mut result = 0u128;
        for i in 0..b {
            let shift = (b - 1 - i) as u32 * bloq_bits;
            if shift >= 128 { break; }
            result |= c_masked << shift;
        }
        Atom::Small(result)
    } else {
        let c_big = BigUint::from(c_masked);
        let mut result = BigUint::from(0u8);
        for i in 0..b {
            let shift = (b - 1 - i) as usize * bloq_bits as usize;
            result |= &c_big << shift;
        }
        Atom::Big(result)
    }
}

pub fn bif(a: BinaryFloat, w: u128, p: u128, b: u128, r: char) -> Atom {
    match a {
        BinaryFloat::Infinity { sign } => {
            let fill_val = fil(0, w as u32, 1);
            let q = lsh(0, p as usize, &fill_val);
            if sign {
                q
            } else {
                let q_u128 = q.to_u128()
                            .expect("float bigger than 128 bits");
                Atom::Small(q_u128.wrapping_add(bex(w + p)))
            }
        }

        BinaryFloat::NaN => {
            let fill_val = fil(0, (w + 1) as u32, 1);
            let shift = sub_or_panic(p, 1) as usize;
            if shift >= 128 { panic!("bif: shift too large"); }
            lsh(0, shift, &fill_val)
        }

        BinaryFloat::Finite { sign, exp: e, mant: a_a } => {
            if a_a.is_zero() {
                return if sign {
                    Atom::Small(0)
                } else {
                    Atom::Small(bex(w + p))
                };
            }

            let ma = met_big(0, &a_a) as u128;

            if ma != p + 1 {
                assert!(e == dif_si(dif_si(2, b), sun_si(p)), "bif: subnormal exponent != me");
                assert!(ma < p + 1, "bif: subnormal mantissa too large");

                let a_small = if a_a.bits() > 128 {
                    panic!("bif: mantissa too large for Small");
                } else {
                    a_a.to_u128().unwrap()
                };

                return if sign {
                    Atom::Small(a_small)
                } else {
                    Atom::Small(a_small.wrapping_add(bex(w + p)))
                };
            }

            let diff = dif_si(e, dif_si(dif_si(2, b), sun_si(p)));
            let q = sum_si(diff, 2);

            let abs_q = abs_si(q);
            let shifted = (abs_q as u128) << p;
            let a_small = if a_a.bits() > 128 {
                panic!("bif: mantissa too large");
            } else {
                a_a.to_u128().unwrap()
            };
            let low_p = a_small & ((1u128 << p) - 1);
            let r = shifted.wrapping_add(low_p);

            if sign {
                Atom::Small(r)
            } else {
                Atom::Small(r.wrapping_add(bex(w + p)))
            }
        }
    }
}

pub fn grd_fl(a: DecimalFloat, b: u128, p: u128, w: u128, mut r: char) -> BinaryFloat {

    //  +pa:ff arm will set these configs before calling +grd:fl
    let v = dif_si(dif_si(2, b), sun_si(p));
    let p = p + 1;
    let w = bex(w) - 3;
    let d = 'd';

    match a {
        DecimalFloat::NaN => BinaryFloat::NaN,
        DecimalFloat::Infinity { sign } => BinaryFloat::Infinity { sign },
        DecimalFloat::Finite { sign, exp: e, mant } => {
            r = 'n';
            let q = abs_si(e);
            let pow5 = pow(5, q);

            let left = BinaryFloat::Finite {
                sign,
                exp: 0,
                mant: BigUint::from(mant),
            };
            if syn_si(e) {
                let right = BinaryFloat::Finite {
                    sign: true,
                    exp: e,
                    mant: pow5,
                };
                binaryfloat_mul(left, right, p, v, w, r, d)
            } else {
                let divisor = BinaryFloat::Finite {
                    sign: true,
                    exp: sun_si(q),
                    mant: pow5,
                };
                binaryfloat_div(left.clone(), divisor.clone(), p, v, w, r, d)
            }
        }
    }
}

//  rylh -> grd:rh -> grd:ff -> grd:fl
pub fn rylh(a: DecimalFloat) -> Atom {
    let w = 5;
    let p = 10;
    let b = 30; // --15
    let r = 'z';
    let grd_res = grd_fl(a, b,  p, w, r);
    bif(grd_res, w, p, b, r)
}

pub fn rylq(a: DecimalFloat) -> Atom {
    let w = 15;
    let p = 112;
    let b = 32766; // --16.383
    let r = 'z';
    let grd_res = grd_fl(a, b,  p, w, r);
    bif(grd_res, w, p, b, r)
}

pub fn ryld(a: DecimalFloat) -> Atom {
    let w = 11;
    let p = 52;
    let b = 2046; // --1.023
    let r = 'z';
    let grd_res = grd_fl(a, b,  p, w, r);
    bif(grd_res, w, p, b, r)
}

pub fn ryls(a: DecimalFloat) -> Atom {
    let w = 8;
    let p = 23;
    let b = 254; // --1.023
    let r = 'z';
    let grd_res = grd_fl(a, b,  p, w, r);
    bif(grd_res, w, p, b, r)
}

pub fn float<'src>(
) -> impl Parser<'src, &'src str, (String, Atom), Err<'src>>
{
    let floats =
            just('-').or_not()
            .then(decimal_without_dots())
            .then(choice((
                    just('.')
                        .ignore_then(leading_zero_decimal_without_dots())
                        .map(|frac| {
                            (frac.len(),
                            frac.parse::<BigUint>().expect("float: invalid fraction"))
                        }),
                    empty().to((0, BigUint::zero())))))
            .then(choice((
                    just('e')
                        .ignore_then(just('-').or_not())
                        .then(decimal_without_dots())
                        .map(|(maybe_hep, expo)| {
                            (!maybe_hep.is_some(), expo.parse::<u128>().expect("float: invalid exponent"))
                        }),
                    empty().to((true, 0)))))
            .map(|(((maybe_hep, p), (len_mant, mant)), (sign_expo, expo))| {
                let term1 = new_si(sign_expo, expo);
                let term2 = sun_si(len_mant as u128);
                let h = dif_si(term1, term2);
                let po = BigUint::from(10u32).pow(len_mant.try_into().unwrap());
                let integer_part = p.parse::<BigUint>().expect("float: invalid decimal");
                let a = integer_part * po + mant;
                DecimalFloat::Finite { sign: !maybe_hep.is_some(), exp: h, mant: a }
            });

    let inf =  just('-').or_not()   //  -inf or inf
                    .then(just("inf"))
                    .map(|(maybe_hep, inf)| DecimalFloat::Infinity{ sign: !maybe_hep.is_some() })
                    .boxed();

    let nan = just("nan")
                .to(DecimalFloat::NaN)
                .boxed();  //  nan

    let royl_rn
             =  choice((
                  floats,  ///  1.10 or 1e10
                  inf,
                  nan,
                )).boxed();

    let rh = just("~~").ignore_then(royl_rn.clone());
    let rq = just("~~~").ignore_then(royl_rn.clone());
    let rd = just('~').ignore_then(royl_rn.clone());
    let rs = royl_rn;

    choice((
        rh.map(|dn| ("rh".to_string(), rylh(dn))),
        rq.map(|dn| ("rq".to_string(), rylq(dn))),
        rd.map(|dn| ("rd".to_string(), ryld(dn))),
        rs.map(|dn| ("rs".to_string(), ryls(dn))),
    )).labelled("Float")
}

pub fn list_wing_hoon_wide<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<(WingType, Hoon)>, Err<'src>>
{
    let pair = winglist()
                .then_ignore(just(' '))
                .then(hoon.clone());

    pair
        .separated_by(just(",").then(just(' ')))
        .at_least(1)
        .collect::<Vec<_>>()
}

pub fn list_hoon_wide<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<Hoon>, Err<'src>>
{
    hoon_wide.clone()
    .separated_by(just(' '))
    .at_least(1)
    .collect::<Vec<Hoon>>()
}

pub fn list_spec_closed_wide<'src>(
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Vec<Spec>, Err<'src>>
{
    spec_wide.clone()
    .separated_by(just(' '))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just('('), just(')'))
}

pub fn list_spec_closed_tall<'src>(
    spec:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Vec<Spec>, Err<'src>>
{
    gap()
    .ignore_then(spec.clone()
                .separated_by(gap())
                .at_least(1)
                .collect::<Vec<_>>()
        )
    .then_ignore(gap())
    .then_ignore(just("=="))
}

pub fn list_wing_hoon_tall<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<(WingType, Hoon)>, Err<'src>>
{
   let pair = winglist()
                .then_ignore(gap())
                .then(hoon.clone())
                .then_ignore(gap());

    pair.repeated().at_least(1).collect::<Vec<(WingType, Hoon)>>()
}

pub fn tiki_wide<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Tiki, Err<'src>>
{
    let with_name = symbol()
        .then_ignore(just('='))
        .then(
            winglist()
                .map(|w| {
                    Box::new(move |t: String| Tiki::Wing((Some(t), w)))
                        as Box<dyn FnOnce(String) -> Tiki>
                })
                .or(hoon_wide.clone()
                    .map(|h| {
                        Box::new(move |t: String| Tiki::Hoon((Some(t), Box::new(h))))
                         as Box<dyn FnOnce(String) -> Tiki>
                }))
        )
        .map(|(t, f)| f(t));

    let no_name = winglist()
        .map(|w| Tiki::Wing((None, w)))
        .or(hoon_wide.clone().map(|h| Tiki::Hoon((None, Box::new(h)))));

    with_name.or(no_name)
}

pub fn tiki_tall<'src>(
    hoon_tall: impl ParserExt<'src, Hoon>,
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Tiki, Err<'src>>
{
    let with_name = symbol()
        .then_ignore(just('='))
        .then(
            winglist()
                .map(|w| {
                    Box::new(move |t: String| Tiki::Wing((Some(t), w)))
                        as Box<dyn FnOnce(String) -> Tiki>
                })
                .or(hoon_tall.clone()
                    .map(|h| {
                        Box::new(move |t: String| Tiki::Hoon((Some(t), Box::new(h))))
                         as Box<dyn FnOnce(String) -> Tiki>
                }))
        )
        .map(|(t, f)| f(t));

    tiki_wide(hoon_wide.clone())    //  the hoon parser has ^= case here but
        .or(
            just("^=").then(gap()).or_not()
            .ignore_then(with_name)
        )
        .or(
            hoon_tall.clone().map(|h| Tiki::Hoon((None, Box::new(h))))
        )
}

///  Parses arms of a Core (grouped by chapters).
///     chapters can be unamed or named with +$
///     arms can be named with ++ or +$
///
pub fn chapters<'src>(
    hoon: impl ParserExt<'src, Hoon>,
    spec: impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, HashMap<String, Tome>, Err<'src>> {
    let luslus = just("++")
            .ignore_then(gap())
            .ignore_then(just('$').to("$".to_string()).or(symbol()))
            .then_ignore(gap())
            .then(hoon.clone())
            .map(|(name, hoon)| {
                // println!("parsing {}", name);
                (name, hoon)
            });

    let lusbuc =  just("+$")
            .ignore_then(gap())
            .ignore_then(symbol())
            .then_ignore(gap())
            .then(spec.clone())
            .map(|(name, spec)| (name.clone(),
                                Hoon::KetCol(Box::new(Spec::Name(name.clone(),
                                                        Box::new(spec))))));

    let optional_chapter_label =
        just("+|")
        .then_ignore(gap())
        .then(just("%"))
        .ignore_then(symbol())
        .then_ignore(gap())
        .or_not();

    let chapter = optional_chapter_label
                    .then(luslus.or(lusbuc)
                          .then_ignore(gap())
                          .repeated().at_least(1).collect::<Vec<_>>());

    chapter.repeated().at_least(1).collect::<Vec<_>>()
        .then_ignore(just("--"))
        .map(|chapters_vec: Vec<(Option<String>, Vec<(String, Hoon)>)>| {
            let mut map_term_tome = HashMap::new();
            for (opt_label, arms_vec) in chapters_vec {
                let mut arms_map = HashMap::new();
                for (name, hoon) in arms_vec {
                    arms_map.insert(name, hoon);
                }
                let key = opt_label.unwrap_or_else(|| "$".to_string());
                map_term_tome.insert(key, arms_map);
            }
            map_term_tome
        })
}

pub fn list_hoon_tall<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<Hoon>, Err<'src>>
{
    hoon.clone()
    .separated_by(gap())
    .at_least(1)
    .collect::<Vec<_>>()
}

pub fn term<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    just("%")
      .ignore_then(symbol())
}

pub fn jet_hooks<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<(String, Hoon)>, Err<'src>>
{
    just('~').to(Vec::new())
        .or(
            just("==")
            .ignore_then(gap())
            .ignore_then(just("%")
                        .ignore_then(symbol().map(|s| format!("%{}", s)))
                        .then_ignore(gap())
                        .then(hoon.clone())
                        .separated_by(gap())
                        .at_least(1)
                        .collect::<Vec<(String, Hoon)>>()
                        )
            .then_ignore(gap())
            .then_ignore(just("=="))
        )
}

pub fn jet_signature<'src>(
) -> impl Parser<'src, &'src str, Chum, Err<'src>>
{
    let lef = symbol().map(Chum::Lef); //  %k

    let stdkel = symbol()              //  %k.138
                .then_ignore(just('.'))
                .then(decimal_number())
                .map(|(s, n)| Chum::StdKel(s, decimal_to_atom(n)));

    let venprokel =
                symbol()  //  %k:foo.138
                .then_ignore(just(':'))
                .then(symbol())
                .then_ignore(just('.'))
                .then(decimal_number())
                .map(|((s1, s2), n)| Chum::VenProKel(s1, s2, decimal_to_atom(n)));

    let venproverkel =  //  %k:foo:bar..138
                symbol()
                .then_ignore(just(':'))
                .then(symbol())
                .then_ignore(just(".."))
                .then(decimal_number())
                .map(|((s1, s2), n)| Chum::VenProKel(s1, s2, decimal_to_atom(n)));

    just("%")
    .ignore_then(
        choice((
            venproverkel,
            venprokel,
            stdkel,
            lef
        )))
}

//  +lute
//
pub fn noun_tall<'src>(
    hoon:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon // can this wrongly match something?
    .separated_by(gap())
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just('[').ignore_then(gap()),
                gap().ignore_then(just(']')))
    .map(|h| Hoon::ColTar(h))
}

pub fn newline<'src>(
) -> impl Parser<'src, &'src str, (), Err<'src>>
{
    just('\n').labelled("Newline").ignored()
}

pub fn soil<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<Woof>, Err<'src>>
{
    let sump = hoon_wide
                .separated_by(just(' '))
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(just('{'), just('}'))
                .map(|h| Woof::Hoon(Hoon::ColTar(h))).boxed();

     // non-control 32-256, excluding DEL, {,  ", \
    let wide_char = any().filter(|c: &char| {
        let x = *c as u32;
        x >= 0x20 && x <= 0x7E && *c != '{' && *c != '"' && *c != '\\'
            || x >= 0x80 && x <= 0xFF
    });

    //
    //  "foo"
    //
    let wide_tape =
        choice((
            //
            //  escaped \, ", {, hex
            //
            just("\\")
                .ignore_then(
                    choice((
                            just("\\").to('\\'),
                            just("\"").to('\"'),
                            just("{").to('{'),
                            // \HH hex escape
                            any().filter(|c: &char| c.is_ascii_hexdigit())
                                .then(any().filter(|c: &char| c.is_ascii_hexdigit()))
                                .map(|(a, b)| {
                                    let hx = format!("{}{}", a, b);
                                    let byte = u8::from_str_radix(&hx, 16).unwrap();
                                    byte as char
                                }),
                            ))
                )
                .map(|c: char| Woof::Atom(Atom::Small(c as u128))),
            //
            //  {hoon}
            //
            sump.clone(),
            ///
            wide_char
                .map(|c| Woof::Atom(Atom::Small(c as u128))),
        )).repeated()
        .collect::<Vec<Woof>>()
        .delimited_by(just("\""), just("\""))
        .labelled("Tape");

    // non-control 32-256, excluding DEL, {,  \
    let tall_char = any().filter(|c: &char| {
        let x = *c as u32;
        x >= 0x20 && x <= 0x7E && *c != '{' && *c != '\\'
            || x >= 0x80 && x <= 0xFF
    });

    //  """
    //  foo
    //  """
    let tall_tape =
            choice((
                //
                //  escaped \, {, hex
                //
                just("\\")
                .ignore_then(
                    choice((just("\\").to('\\'),
                            just("{").to('{'),
                            // \HH hex escape
                            any().filter(|c: &char| c.is_ascii_hexdigit())
                                .then(any().filter(|c: &char| c.is_ascii_hexdigit()))
                                .map(|(a, b)| {
                                    let hx = format!("{}{}", a, b);
                                    let byte = u8::from_str_radix(&hx, 16).unwrap();
                                    byte as char
                                })
                            ))
                )
                .map(|c: char| Woof::Atom(Atom::Small(c as u128))),
            //
            // linebreak
            //
                newline()
                .ignore_then(just("\"\"\"").not())
                .to(Woof::Atom(Atom::Small('\n' as u128))),
            //
            //  {hoon}
            //
                sump,
            //
                tall_char
                .map(|c| Woof::Atom(Atom::Small(c as u128))),
            )).repeated()
            .at_least(1)
            .collect::<Vec<Woof>>()
            .delimited_by(just("\"\"\"").ignore_then(newline()),
                          newline().then_ignore(just("\"\"\"")))
            .labelled("Tape");

    choice((tall_tape, wide_tape))
}

pub fn tape<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    soil(hoon_wide.clone())
    .separated_by(just('.').ignore_then(gap().or_not()))
    .at_least(1)
    .collect::<Vec<_>>()
    .map(|s: Vec<Vec<Woof>>| {
        let wof: Vec<Woof> = s.into_iter().flatten().collect();
        Hoon::Knit(wof)
    }).labelled("Tape")
}

pub fn aura_text<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    just("@")
    .ignore_then(regex(r"[a-zA-Z]+")
                .map(str::to_owned)
                .or_not())
    .map(|maybe_name| {
        maybe_name.unwrap_or("~.".to_string())
    })
}

pub fn aura_hoon<'src>(
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    aura_text()
    .map(|s| Hoon::Base(BaseType::Atom(s)))
}

pub fn loop_spec<'src>(
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    just('/')
    .ignore_then(
        choice((just('$').to("$".to_string()),
            symbol(),
        )))
    .map(|s| Spec::Loop(s))
}

pub fn aura_spec<'src>(
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    aura_text()
    .map(|s| Spec::Base(BaseType::Atom(s)))
}

pub fn concatanate<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
      .then_ignore(just('^'))
      .then(hoon_wide.clone())
      .map(|(p, q)| Hoon::Pair(Box::new(p), Box::new(q)))
}

pub fn wing<'src>(
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    winglist()
    .map(|list: WingType| {
        match list.first() {
            Some(Limb::Axis(0))
                | Some(Limb::Term(_))
                | Some(Limb::Parent(_, _)) => {
                Hoon::Wing(list)
            }
            _ => Hoon::CenTis(list, vec![])
        }
    })
}

pub fn tell<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just("<")
        .ignore_then(list_hoon_wide(hoon_wide.clone()))
        .then_ignore(just(">"))
        .map(|list| Hoon::Tell(list))
}


pub fn yell<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just(">")
        .ignore_then(list_hoon_wide(hoon_wide.clone()))
        .then_ignore(just("<"))
        .map(|list| Hoon::Yell(list))
}

pub fn constant<'src>(
) -> impl Parser<'src, &'src str, Coin, Err<'src>>
{
    let buc =      // %$
        just('$')
        .to(Coin::Dime("tas".to_string(), Atom::Small(0)));

    let cord =      // %'foo'
        cord()
        .map(|s| Coin::Dime("t".to_string(), s));

    let coin =      // %123, %~m5, etc.
        nuck();

    let no =
        just('|')
        .to(Coin::Dime("f".to_string(), Atom::Small(1)));

    let yes =
        just('&')
        .to(Coin::Dime("f".to_string(), Atom::Small(0)));

    just('%')
    .ignore_then(
        choice((
            buc,
            yes,
            no,
            cord,
            coin,
        )))
}

pub fn cord<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>>
{
    //  \\, \' and \AA were A is a hex digit
    let escape = just('\\')
        .ignore_then(
            choice((
                just('\\').to('\\'),
                just('\'').to('\''),
                // \HH hex escape
                any().filter(|c: &char| c.is_ascii_hexdigit())
                    .then(any().filter(|c: &char| c.is_ascii_hexdigit()))
                    .map(|(a, b)| {
                        let hx = format!("{}{}", a, b);
                        let byte = u8::from_str_radix(&hx, 16).unwrap();
                        byte as char
                    }),
            ))
        );

    //  chars from 32-256 (excluding DEL, ', \)
    let raw_char = any().filter(|c: &char| {
        let x = *c as u32;

        (0x20..=0x7E).contains(&x)
            && x != 0x27   // '
            && x != 0x5C   // '\'
        ||
        (0x80..=0xFF).contains(&x)
    });

    let gon = just("\\")  // multiline separator
            .ignore_then(gap())
            .ignore_then(just("/"))
            .ignored()
            .labelled("Cord Multiline Separator");

    let char_in_singled_quoted = choice((escape,
                                         raw_char,
                                        )).labelled("Cord Character");

    let single_quoted =  char_in_singled_quoted.then_ignore(gon.or_not())
                        .repeated()
                        .collect::<Vec<char>>()
                        .delimited_by(just("'"), just("'"))
                        .map(cord_chars_to_atom);

    let triple_quoted = non_control_char()
                        .repeated()
                        .collect::<Vec<char>>()
                        .delimited_by(just("'''").ignore_then(gap()),
                                    newline().ignore_then(just("'''")))
                        .map(cord_chars_to_atom);

    choice((
        triple_quoted,
        single_quoted,
    )).labelled("Cord")
}

pub fn increment<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just('.').or_not()
        .ignore_then(just("+"))
        .ignore_then(just('('))
        .ignore_then(
            hoon_wide.clone()
        )
        .then_ignore(just(')'))
        .map(|h| Hoon::DotLus(Box::new(h)))
}

pub fn function_call<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just('(')
        .ignore_then(hoon.clone())
        .then(
            just(' ')
                .ignore_then(hoon.clone())
                .repeated()
                .collect::<Vec<_>>()
            )
    .then_ignore(just(')'))
    .map(|(func, args)| Hoon::CenCol(Box::new(func), args))
}

const YEAR_OFFSET: u64 = 292_277_024_400;

fn yelp(yer: u64) -> bool {
    (yer % 4 == 0) && ((yer % 100 != 0) || (yer % 400 == 0))
}

// Constants from ++yo
const CETY: u64 = 36_524;   // days in 100 years (non-leap century)
const DAY: u64 = 86_400;    // seconds/day
const ERA: u64 = 146_097;   // days in 400 years
const HOR: u64 = 3_600;     // seconds/hour
const MIT: u64 = 60;        // seconds/minute
const MOH: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]; // normal
const MOY: [u64; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]; // leap

// ++yawn: days since "Jesus" (proleptic Gregorian)
fn yawn(mut yer: u64, mut mot: u64, mut day: u64) -> u64 {
    // => .(mot (dec mot), day (dec day))
    mot = mot.saturating_sub(1);
    day = day.saturating_sub(1);

    let cah = if yelp(yer) { &MOY } else { &MOH };
    for i in 0..mot as usize {
        day += cah[i];
    }

    loop {
        if yer % 4 != 0 {
            if yer == 0 { break; }
            yer -= 1;
            day += if yelp(yer) { 366 } else { 365 };
            continue;
        }
        if yer % 100 != 0 {
            if yer < 4 { break; }
            yer -= 4;
            day += if yelp(yer) { 1_461 } else { 1_460 };
            continue;
        }
        if yer % 400 != 0 {
            if yer < 100 { break; }
            yer -= 100;
            day += if yelp(yer) { 36_525 } else { 36_524 };
            continue;
        }
        // divisible by 400
        day += (yer / 400) * (1 + 4 * CETY); // 1 + 4*36524 = 146097 = ERA
        break;
    }
    day
}

pub fn apply_sign(a: bool, b: Atom) -> Atom {
    match b {
        Atom::Small(n) => {
            let out = if a {
                2 * n
            } else if n == 0 {
                0
            } else {
                2 * (n - 1) + 1
            };
            Atom::Small(out)
        }
        Atom::Big(n) => {
            let out = if a {
                &n << 1
            } else if n.is_zero() {
                num_bigint::BigUint::from(0u32)
            } else {
                ((&n - 1u32) << 1) + 1u32
            };
            Atom::Big(out)
        }
    }
}

//  @ta, @tas to @
pub fn string_to_atom(s: String) -> Atom {
    let vec_u128: Vec<u128> = s.chars().map(|c| c as u128).collect();

    rap(3, &vec_u128)
}

//  @ud to @
pub fn decimal_to_atom(s: String) -> Atom {
    Atom::Small(s.parse::<u128>().expect("decimal_to_atom failed"))
}

//  @ux to @
pub fn hex_to_atom(s: String) -> Atom {
    let clean = s.strip_prefix("0x").unwrap_or(&s);

    if clean.len() <= 32 {
        if let Ok(n) = u128::from_str_radix(clean, 16) {
            return Atom::Small(n);
        }
    }

    let big = BigUint::parse_bytes(clean.as_bytes(), 16)
        .expect("invalid hex in big atom");

    Atom::Big(big)
}

//  @ub to @
pub fn binary_to_atom(s: String) -> Atom {
    Atom::Small(u128::from_str_radix(&s, 2).expect("binary_to_atom failed"))
}

//  @t to @
pub fn cord_chars_to_atom(chars: Vec<char>) -> Atom {

    let mut atom = BigUint::zero();
    let mut power = BigUint::from(1u32);
    let base = BigUint::from(256u32);

    for &c in &chars {
        let byte = BigUint::from(c as u32 & 0xFF);
        atom += &byte * &power;
        power *= &base;
    }

    Atom::Big(atom)

}

const ALPH64: &str =
    "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-~";

//  @uw to @
pub fn base64_to_atom(s: String) -> Atom {
    let mut n: u128 = 0;

    for ch in s.chars() {
        let v = match ALPH64.find(ch) {
            Some(i) => i as u128,
            None => panic!("invalid digit '{ch}' in base64"),
        };

        n = n
            .checked_mul(64)
            .expect("value exceeds u128 range (mul)");

        n = n
            .checked_add(v)
            .expect("value exceeds u128 range (add)");
    }

    Atom::Small(n)
}

const ALPH32: &str = "0123456789abcdefghijklmnopqrstuv";

//  @uv to @
pub fn base32_to_atom(s: String) -> Atom {
    let mut n: u128 = 0;

    for ch in s.chars() {
        let v = match ALPH32.find(ch) {
            Some(i) => i as u128,
            None => panic!("invalid digit '{ch}' in base32"),
        };

        n = n
            .checked_mul(32)
            .expect("value exceeds u128 range (mul)");

        n = n
            .checked_add(v)
            .expect("value exceeds u128 range (add)");
    }

    Atom::Small(n)
}

///  Alphanumeric with hyphens
///      Start with a lowercase letter
///      Followed by zero or more: lowercase letter, digit, or hyphen
///
pub fn symbol<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>> {
    regex(r"[a-z](?:[a-zA-Z0-9-])*").map(str::to_owned)
    .labelled("Term")
}

const BTC_BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn build_yek() -> [u8; 256] {
    let mut yek = [0xFFu8; 256];
    for (i, ch) in BTC_BASE58.chars().enumerate() {
        let idx = ch as u8 as usize;
        if idx < 256 {
            yek[idx] = i as u8;
        }
    }
    yek
}

fn cha_fa(yek: &[u8; 256], ch: char) -> Option<u8> {
    let idx = ch as u32;
    if idx > 255 { return None; }
    let val = yek[idx as usize];
    if val == 0xFF { None } else { Some(val) }
}

fn bass_58(digits: &[u8]) -> BigUint {
    digits.iter().fold(BigUint::from(0u32), |acc, &d| {
        &acc * 58u32 + d as u32
    })
}

fn tok(a: &Atom) -> Atom {
    let b = pad(&a.to_biguint());

    let swapped = swp(3, a);

    let padded = lsh(3, b, &swapped);

    let len = b + met(3, a);

    let hashed = shay(len as u64, &padded.to_biguint());

    let double_hashed = &Atom::Big(shay(32, &hashed));
    let truncated = end(3, 4, double_hashed);

    let n = net(5, &truncated);
    n
}

pub fn shay(len: u64, ruz: &BigUint) -> BigUint {
    let len = len as usize;

    let ruz_bytes = ruz.to_bytes_le();
    let msg_len = ruz_bytes.len();

    let mut msg = vec![0u8; len];

    if len == 0 {
    } else if msg_len >= len {
        msg.copy_from_slice(&ruz_bytes[..len]);
    } else {
        msg[..msg_len].copy_from_slice(&ruz_bytes);
    }

    let mut hasher = Sha256::new();
    hasher.update(&msg);
    let digest = hasher.finalize();

    BigUint::from_bytes_le(&digest)
}

fn swp(bloq: usize, b: &Atom) -> Atom {
    let blocks = rip(bloq, b);
    let rev = flop(&blocks);
    rep(bloq, None, &rev)
}

fn rip(bloq: usize, b: &Atom) -> Vec<Atom> {
    if b.is_zero() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut cur = b.clone();

    while !cur.is_zero() {
        out.push(end(bloq, 1, &cur));
        cur = rsh(bloq, 1, &cur);
    }

    out
}

pub fn den_fa(a: &Atom) -> Option<Atom> {
    let b = rsh(3, 4, a);

    if tok(&b) == end(3, 4, a) {
        Some(b)
    } else {
        None
    }
}

fn sit(a: usize, b: &Atom) -> Atom {
    end(a, 1, b)
}

//  flip byte endianness
fn net(a: usize, b: &Atom)
-> Atom {
    let b = sit(a, b);

    if a <= 3 {
        return b;
    }

    let c: usize = a - 1;

    let hi_bit = cut(c, 0, 1, &b);
    let hi = net(c, &hi_bit);

    let lo_bit = cut(c, 1, 1, &b);
    let lo = net(c, &lo_bit);

    let res = con_atoms(lsh(c, 1, &hi), lo);
    // println!("net a {} b {:?} res {:?}", a, b, res);
    res
}

fn met_big(bloq: u32, atom: &BigUint) -> u32 {
    let bits = 1u32 << bloq; // bloq_bits
    if atom.is_zero() {
        return 1;
    }
    let atom_bits = atom.bits() as u32;
    (atom_bits + bits - 1) / bits
}

/// pad(a): number of zero bytes needed to pad `a` to 21 bytes
fn pad(a: &BigUint) -> usize {
    let b = met(3, &Atom::Big(a.clone()));
    if b >= 21 {
        0
    } else {
        21 - b as usize
    }
}

// +fim
pub fn base58_to_atom(s: String) -> Option<Atom> {
    let yek = build_yek();

    let digits: Vec<u8> = s.chars()
        .map(|ch| cha_fa(&yek, ch))
        .collect::<Option<_>>()?;

    let a = Atom::Big(bass_58(&digits));
    den_fa(&a)
}

pub fn bitcoin_address<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>> {
    just("0c")
    .ignore_then(
        regex(r"[0-9a-zA-Z]+").map(str::to_owned))
    .labelled("Bitcoin_Address")
}


pub fn urs<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>> {
    regex(r"[0-9a-z._~\-]+").map(|s: &str| string_to_atom(s.to_string()))
}

pub fn urt<'src>(
) -> impl Parser<'src, &'src str, &'src str, Err<'src>> {
    regex(r"[0-9a-z.~\-]+")
    // .map(str::to_owned)
}

fn wick(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('~') => out.push('~'),           // ~~ -> ~
                Some('-') => out.push('_'),           // ~- -> _
                Some(_) | None => return None,        // invalid escape
            }
        } else {
            // Only allow valid @ta characters: [a-z0-9._-]
            // (original regex ensures this, but let’s be safe)
            if c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '.'
                || c == '_'
                || c == '-'
            {
                out.push(c);
            } else {
                return None; // invalid char in atom
            }
        }
    }

    Some(out)
}

pub fn urx<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>> {
    let hex_escape =
        any().filter(|c: &char| c.is_ascii_hexdigit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .delimited_by(just('~'), just('.'))
        .map(|hex_str: String| {
                let big = BigUint::from_str_radix(&hex_str, 16).unwrap_or_default();
                let value_32 = big.iter_u32_digits().next().unwrap_or(0); // low 32 bits

                let tuft_result = tuft(&Atom::Small(value_32 as u128));

                match tuft_result {
                    Atom::Small(n) => n,
                    Atom::Big(_) => panic!("tuft overflow"),
                }
            });

    let special = choice((
        just("~~").to(b'~' as u128),
        just("~.").to(b'.' as u128),
        just('.').to(b' ' as u128),
    ));

    let ascii = any().filter(|c: &char| {
        c.is_ascii_digit() || c.is_ascii_lowercase() || *c == '-' || *c == '_'
    })
    .map(|c| c as u128);

    let token = choice((
        hex_escape,
        special,
        ascii,
    ));

    token
    .repeated()
    .at_least(1)
    .collect::<Vec<u128>>()
    .map(|chars: Vec<u128>| rap(3, &chars))
}

fn atom_shl(a: &Atom, bits: usize) -> Atom {
    if bits == 0 { return a.clone(); }
    match a {
        Atom::Small(n) => {
            if bits >= 128 { Atom::from_biguint(BigUint::from(*n) << bits) }
            else { Atom::Small(n << bits) }
        }
        Atom::Big(b) => Atom::from_biguint(b << bits),
    }
}

fn atom_shr(atom: &Atom, bits: usize) -> Atom {
    if bits == 0 {
        return atom.clone();
    }
    match atom {
        Atom::Small(n) => {
            if bits >= 128 {
                Atom::Small(0)
            } else {
                Atom::Small(n >> bits)
            }
        }
        Atom::Big(b) => {
            Atom::from_biguint(b >> bits)
        }
    }
}

fn atom_mask_low_bits(atom: &Atom, bits: usize) -> Atom {
    if bits == 0 {
        return Atom::Small(0);
    }
    match atom {
        Atom::Small(n) => {
            if bits >= 128 {
                Atom::Small(*n)
            } else {
                let mask = (1u128 << bits) - 1;
                Atom::Small(*n & mask)
            }
        }
        Atom::Big(b) => {
            if bits <= 128 {
                let mask: u128 = (1u128 << bits) - 1;
                let mut limbs = b.iter_u64_digits();
                let lo = limbs.next().unwrap_or(0);
                let hi = limbs.skip(1).next().unwrap_or(0);
                let low_u128 = ((hi as u128) << 64) | (lo as u128);
                Atom::Small(low_u128 & mask)
            } else {
                let mask = (BigUint::one() << bits) - BigUint::one();
                Atom::from_biguint(b & &mask)
            }
        }
    }
}

// tuft: Atom (codepoint) -> Atom (UTF-8 bytes, @t)
pub fn tuft(cp_atom: &Atom) -> Atom {
    let cp = match cp_atom {
        Atom::Small(n) => *n as u32,
        Atom::Big(b) => {
            b.to_u32().unwrap_or(0)
        }
    };

    let (bytes, len): ([u8; 4], usize) = if cp <= 0x7F {
        ([cp as u8, 0, 0, 0], 1)
    } else if cp <= 0x7FF {
        ([
            (0xC0 | ((cp >> 6) & 0x1F)) as u8,
            (0x80 | (cp & 0x3F)) as u8,
            0, 0,
        ], 2)
    } else if cp <= 0xFFFF {
        ([
            (0xE0 | ((cp >> 12) & 0x0F)) as u8,
            (0x80 | ((cp >> 6) & 0x3F)) as u8,
            (0x80 | (cp & 0x3F)) as u8,
            0,
        ], 3)
    } else {
        // 4-byte case: use low 21 bits (like Hoon's cut [18 3], [12 6], [6 6], [0 6])
        // Hoon does not validate range — just encodes 32-bit value's low 21 bits
        ([
            (0xF0 | ((cp >> 18) & 0x07)) as u8,
            (0x80 | ((cp >> 12) & 0x3F)) as u8,
            (0x80 | ((cp >> 6) & 0x3F)) as u8,
            (0x80 | (cp & 0x3F)) as u8,
        ], 4)
    };

    // Pack little-endian: bytes[0] at bits 0–7, bytes[1] at 8–15, etc.
    let mut acc: u128 = 0;
    for i in 0..len {
        acc |= (bytes[i] as u128) << (i * 8);
    }

    Atom::Small(acc)
}

// --- Extract low byte as u8 ---
fn atom_to_u8(atom: &Atom) -> u8 {
    match end(3, 1, atom) {
        Atom::Small(n) => n as u8,
        Atom::Big(_) => 0,
    }
}

// --- UTF-8 continuation byte check ---
fn is_continuation(b: u8) -> bool {
    b & 0xC0 == 0x80
}

// --- teff: UTF-8 leading byte → length (1–4) ---
fn teff(atom: &Atom) -> usize {
    let b = atom_to_u8(atom);
    if b == 0 {
        return 0;
    }
    if b <= 0x7F { 1 }
    else if b <= 0xDF { 2 }
    else if b <= 0xEF { 3 }
    else if b <= 0xF4 { 4 }
    else { 1 } // invalid → skip 1 byte
}

// --- Decode one UTF-8 codepoint ---
fn decode_one_utf8(atom: &Atom, len: usize) -> u32 {
    match len {
        1 => atom_to_u8(atom) as u32,
        2 => {
            let b0 = atom_to_u8(atom);
            let b1 = atom_to_u8(&rsh(3, 1, atom));
            if !is_continuation(b1) { return 0xFFFD; }
            let cp = ((b0 & 0x1F) as u32) << 6 | (b1 & 0x3F) as u32;
            if cp < 0x80 { 0xFFFD } else { cp }
        }
        3 => {
            let b0 = atom_to_u8(atom);
            let b1 = atom_to_u8(&rsh(3, 1, atom));
            let b2 = atom_to_u8(&rsh(3, 2, atom));
            if !is_continuation(b1) || !is_continuation(b2) { return 0xFFFD; }
            let cp = ((b0 & 0x0F) as u32) << 12 | ((b1 & 0x3F) as u32) << 6 | (b2 & 0x3F) as u32;
            if cp < 0x800 || (0xD800..=0xDFFF).contains(&cp) { 0xFFFD } else { cp }
        }
        4 => {
            let b0 = atom_to_u8(atom);
            let b1 = atom_to_u8(&rsh(3, 1, atom));
            let b2 = atom_to_u8(&rsh(3, 2, atom));
            let b3 = atom_to_u8(&rsh(3, 3, atom));
            if !is_continuation(b1) || !is_continuation(b2) || !is_continuation(b3) { return 0xFFFD; }
            let cp = ((b0 & 0x07) as u32) << 18 | ((b1 & 0x3F) as u32) << 12
                   | ((b2 & 0x3F) as u32) << 6  | (b3 & 0x3F) as u32;
            if !(0x1_0000..=0x10_FFFF).contains(&cp) { 0xFFFD } else { cp }
        }
        _ => 0xFFFD,
    }
}

// @t (UTF-8 atom) -> @c (UTF-32 packed atom)
pub fn taft(atom: &Atom) -> Atom {
    let mut codepoints = Vec::new();
    let mut current = atom.clone();

    loop {
        let len = teff(&current);
        if len == 0 {
            break;
        }
        let cp = decode_one_utf8(&current, len);
        codepoints.push(cp);
        current = rsh(3, len, &current); // shift by `len` bytes
    }

    // Pack into @c: each u32 in 32-bit lane, LSB-first (rap 5)
    if codepoints.is_empty() {
        Atom::Small(0)
    } else if codepoints.len() <= 4 {
        let mut acc: u128 = 0;
        for (i, &cp) in codepoints.iter().enumerate() {
            acc |= (cp as u128) << (i * 32);
        }
        Atom::Small(acc)
    } else {
        let mut acc = BigUint::zero();
        for (i, &cp) in codepoints.iter().enumerate() {
            acc |= BigUint::from(cp) << (i * 32);
        }
        Atom::from_biguint(acc)
    }
}

pub fn binary_number<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first = just("0b")
                .ignore_then(regex(r"(?:0|1[01]{0,3})"));

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"[01]{4}"));

    first
        .then(rest.repeated().collect::<Vec<_>>())
        .map(|(first, mut rest)| {
            if rest.is_empty() {
                first.to_string()
            } else {
                let mut parts = vec![first];
                parts.append(&mut rest);
                parts.join("").to_string()
            }
        })
        .labelled("Binary")
}

pub fn hexadecimal_number<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first =
        just("0x")
        .ignore_then(regex(r"0|[1-9a-fA-F][0-9a-fA-F]{0,3}"));

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"[0-9a-fA-F]{4}"))
            .repeated()
            .collect::<Vec<_>>();

    first
    .then(rest)
        .map(|(first, mut rest)| {
            if rest.is_empty() {
                first.to_string()
            } else {
                let mut parts = vec![first];
                parts.append(&mut rest);
                parts.join("").to_string()
            }
        })
        .labelled("Hexadecimal")
}

pub fn ipv4_address<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first = regex(r"(0|[1-9][0-9]{0,2})");

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"([0-9]{0,3})").clone())
            .repeated()
            .exactly(3)
            .collect::<Vec<_>>();

    first.then(rest)
        .map(|(first, mut rest)| {
            if rest.is_empty() {
                first.to_string()
            } else {
                let mut parts = vec![first];
                parts.append(&mut rest);
                parts.join(".").to_string()
            }
        })
        .labelled("Ipv4-Address")
}

pub fn ipv4_to_atom(s: String) ->  Option<Atom>{
    let addr = s
        .parse::<std::net::Ipv4Addr>().ok()?;

    let ip_num = u32::from_be_bytes(addr.octets());

    Some(Atom::Small(ip_num.into()))
}

pub fn ipv6_address<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let tod = regex(r"0|[1-9a-f][0-9a-fA-F]{0,3}");

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(tod.clone())
            .repeated()
            .exactly(7)
            .collect::<Vec<_>>();

    tod.then(rest)
        .map(|(first, mut rest)| {
            if rest.is_empty() {
                first.to_string()
            } else {
                let mut parts = vec![first];
                parts.append(&mut rest);
                parts.join(".").to_string()
            }
        })
        .labelled("Ipv6-Address")
}

pub fn ipv6_to_atom(s: String) -> Option<Atom> {
    let addr = s.parse::<std::net::Ipv6Addr>().ok()?;
    let num = u128::from_be_bytes(addr.octets());
    Some(Atom::Small(num))
}

pub fn base32_number<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>>
{
    let first = just("0v")
                .ignore_then(regex(r"0|[1-9a-v][0-9a-v]{0,4}"));

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"[0-9a-v]{0,5}"))
            .repeated()
            .collect::<Vec<_>>();

    first.then(rest)
        .map(|(first, mut rest)| {
            if rest.is_empty() {
                base32_to_atom(first.to_string())
            } else {
                let mut parts = vec![first];
                parts.append(&mut rest);
                base32_to_atom(parts.join(""))
            }
        })
        .labelled("Base32")
}

pub fn base64_number<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>>
{
    let first = just("0w")
                .ignore_then(regex(r"0|[1-9a-zA-Z-~][0-9a-zA-Z-~]{0,4}"));

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"[0-9a-zA-Z-~]{0,5}"))
            .repeated()
            .collect::<Vec<_>>();

    first.then(rest)
        .map(|(first, mut rest)| {
            if rest.is_empty() {
                base64_to_atom(first.to_string())
            } else {
                let mut parts = vec![first];
                parts.append(&mut rest);
                base64_to_atom(parts.join(""))
            }
        })
        .labelled("Base64")
}

pub fn decimal_number<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first =
        regex(r"(0|[1-9][0-9]{0,2})")
        .map(str::to_owned);

    let rest = just('.')
                .ignore_then(gap().or_not())
                .ignore_then(regex(r"[0-9]{3}").map(str::to_owned))
                .repeated()
                .collect::<Vec<String>>();

    first.then(rest)
        .map(|(first_digits, rest_digits)| {
            let all_digits: String = std::iter::once(first_digits)
                .chain(rest_digits.into_iter())
                .collect();
            all_digits
            // all_digits
            //     .into_iter()
            //     .map(|c| c.to_digit(10).expect("invalid digit") as u64)
            //     .try_fold(0u64, |acc, digit| {
            //         acc.checked_mul(10).and_then(|v| v.checked_add(digit))
            //     })
            //     .expect("number overflow")
        })
        .labelled("Decimal Number")
}

pub fn weld<T: Clone>(a: impl AsRef<[T]>, b: impl AsRef<[T]>) -> Vec<T> {
    let a = a.as_ref();
    let b = b.as_ref();
    let mut v = Vec::with_capacity(a.len() + b.len());
    v.extend_from_slice(a);
    v.extend_from_slice(b);
    v
}

pub fn scag<T: Clone>(n: usize, list: impl AsRef<[T]>) -> Vec<T> {
    list.as_ref().iter().take(n).cloned().collect()
}

pub fn slag<T: Clone>(n: usize, list: impl AsRef<[T]>) -> Vec<T> {
    list.as_ref().iter().skip(n).cloned().collect()
}

pub fn flop<T: Clone>(list: impl AsRef<[T]>) -> Vec<T> {
    let mut v = list.as_ref().to_vec();
    v.reverse();
    v
}

fn poof(pax: Path) -> Vec<Hoon> {
    pax.iter()
        .map(|a| { Hoon::Sand(
            "ta".to_string(),
            Noun::Atom(string_to_atom(a.clone())),
        )})
        .collect()
}

// used to create dbug traces
#[derive(Clone)]
pub struct LineMap {
    starts: Vec<usize>,
}

impl LineMap {
    #[inline]
    pub fn new(src: &str) -> Self {
        let mut starts = Vec::with_capacity(128);
        starts.push(0);

        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }

        Self { starts }
    }

    #[inline(always)]
    fn line_col(&self, byte: usize) -> (u64, u64) {
        let line = match self.starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        };

        (
            (line + 1) as u64,
            (byte - self.starts[line] + 1) as u64,
        )
    }

    #[inline(always)]
    pub fn pint(&self, span: std::ops::Range<usize>) -> Pint {
        Pint {
            p: self.line_col(span.start),
            q: self.line_col(span.end),
        }
    }
}

fn poon(
    pag: &[Hoon],
    goo: &[Option<Hoon>],
) -> Option<Vec<Hoon>> {
    if goo.is_empty() {
        return Some(vec![]);
    }

    let (goo_hd, goo_tl) = goo.split_first().unwrap();

    let head = match goo_hd {
        Some(x) => x.clone(),
        None => {
            let (pag_hd, _) = pag.split_first()?;
            pag_hd.clone()
        }
    };

    let pag_tl = if pag.is_empty() {
        &[]
    } else {
        &pag[1..]
    };

    let mut rest = poon(pag_tl, goo_tl)?;

    let mut out = Vec::with_capacity(rest.len() + 1);
    out.push(head);
    out.append(&mut rest);

    Some(out)
}

pub fn posh(
    pre: Option<Vec<Option<Hoon>>>,           // (unit tyke)
    pof: Option<(usize, Vec<Option<Hoon>>)>,  // (unit [p=@ud q=tyke])
    wer: Path,
) -> Option<Vec<Hoon>> {

    let wom: Vec<Hoon> = poof(wer);

    let yez = if pre.is_none() {
        Some(wom.clone())
    } else {
        let pre_val = pre.as_ref().unwrap();

        let moz = poon(&wom, pre_val)?;

        if let Some(_) = pof {
            let n  = pre_val.len();
            let sl = slag(n, &wom.clone());
            Some(weld(&moz, &sl))
        } else {
            Some(moz)
        }
    }?;

    if pof.is_none() {
        return Some(yez);
    }

    let (p, q) = pof.unwrap();

    let zey = flop(&yez.clone());

    let moz = scag(p, &zey);
    let gul = slag(p, &zey);

    let zom = poon(&flop(&moz.clone()), &q);

    match zom {
        None => None,
        Some(z) => Some(weld(&flop(&gul), z))
    }
}

pub fn nusk<'src>(
) -> impl Parser<'src, &'src str, Coin, Err<'src>>
{
    urt()
    .try_map(|s, span| {
           wick(s).ok_or_else(||
                Rich::custom(span, format!("invalid knot escape in '{}'", s))
            )
        })
        .try_map(|unescaped: String, span| {
            let parsed = nuck().parse(&unescaped);
            match parsed.into_result() {
                Ok(output) => Ok(output),
                Err(_errors) => {
                    Err(Rich::custom(span, "nuck parse failed"))
                }
            }
        })
}

pub fn jock(rad: bool, lot: &Coin) -> Hoon {
    match lot {
        Coin::Dime(tag, atom) => {
            if rad {
                Hoon::Rock(tag.clone(), Noun::Atom(atom.clone()))
            } else {
                Hoon::Sand(tag.clone(), Noun::Atom(atom.clone()))
            }
        }

        Coin::Blob(noun) => {
            if rad {
                Hoon::Rock("$".to_string(), noun.clone())
            } else {
                match noun {
                    Noun::Atom(atom) => Hoon::Sand("$".to_string(), Noun::Atom(atom.clone())),
                    Noun::Cell(head, tail) => {
                        Hoon::Pair(
                            Box::new(jock(rad, &Coin::Blob(*head.clone()))),
                            Box::new(jock(rad, &Coin::Blob(*tail.clone()))),
                        )
                    }
                }
            }
        }

        Coin::Many(coins) => {
            Hoon::ColTar(coins.iter().map(|c| jock(rad, c)).collect())
        }
    }
}

pub fn nuck<'src>(
) -> impl Parser<'src, &'src str, Coin, Err<'src>>
{
    choice((
        symbol().map(|s| Coin::Dime("tas".to_string(), string_to_atom(s))),
        number().map(|(p, q)| Coin::Dime(p, q)),
        just('.').ignore_then(perd()),
        just('~').ignore_then(
            choice((
                twid(),
                empty().to(Coin::Dime("n".to_string(), Atom::Small(0))),
            ))),
    )).boxed()
}

pub fn perd<'src>(
) -> impl Parser<'src, &'src str, Coin, Err<'src>>
{
    choice((
        zust(),
        nusk()
            .separated_by(just('_'))
            .at_least(1)
            .collect::<Vec<_>>()
            .delimited_by(just('_'), just("__"))
            .map(|t| Coin::Many(t))
    ))
}

pub fn zust<'src>(
) -> impl Parser<'src, &'src str, Coin, Err<'src>>
{
    choice((
        ipv4_address().try_map(|s, span| {
            let maybe_ipv4 = ipv4_to_atom(s);
            match maybe_ipv4 {
                None => Err(Rich::custom(span, "invalid ipv4")),
                Some(atom) => Ok(Coin::Dime("if".to_string(), atom)),
            }
        }),
        ipv6_address().try_map(|s, span| {
            let maybe_ipv6 = ipv6_to_atom(s);
            match maybe_ipv6 {
                None => Err(Rich::custom(span, "invalid ipv6")),
                Some(atom) => Ok(Coin::Dime("is".to_string(), atom)),
            }
        }),
        float().map(|(p, q)| Coin::Dime(p, q)),
        just("y").to(Coin::Dime("f".to_string(), Atom::Small(0))),
        just("n").to(Coin::Dime("f".to_string(), Atom::Small(1))),
        just('~')
            .ignore_then(phonemic_name_unscrambled())
            .map(|s| Coin::Dime("q".to_string(), s)),
    ))
}

pub fn path<'src>(
    hoon_wide: impl ParserExt<'src, Hoon>,
    wer: Path,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    let wer1 = wer.clone();
    let wer2 = wer.clone();
    let wer3 = wer.clone();
    let wer4 = wer.clone();

    let hasp = choice((
                hoon_wide.clone().delimited_by(just('['), just(']')),
                hoon_wide.clone()
                    .separated_by(just(' '))
                    .at_least(1)
                    .collect::<Vec<_>>()
                    .delimited_by(just('('), just(')'))
                    .map(|list| {
                        let (first, rest) = list.split_first().unwrap();
                        Hoon::CenCol(Box::new(first.clone()), rest.to_vec())
                    }),
                just('$').to(Hoon::Sand("tas".to_string(), Noun::Atom(Atom::Small(0)))),
                cord().map(|s| Hoon::Sand("t".to_string(), Noun::Atom(s))),
                nuck().map(|coin| jock(false, &coin)), // TODO: call rent here..
            ));

    let gasp = choice((
                    just('=')
                        .to(None)
                        .repeated()
                        .collect::<Vec<Option<Hoon>>>()
                    .then(hasp.map(|h| vec![Some(h)]))
                        .then(
                            just('=')
                                .to(None)
                                .repeated()
                                .collect::<Vec<Option<Hoon>>>()
                        )
                        .map(|((mut a, b), c)| {
                            a.extend(b);
                            a.extend(c);
                            a
                        }),
                    just('=')
                        .to(None)
                        .repeated()
                        .at_least(1)
                        .collect::<Vec<Option<Hoon>>>(),
                    ));

    let limp =  just("/").repeated().count()
                .then(gasp)
                .map(|(a, mut b)| {
                    for _ in 0..a {
                        b.insert(0, Some(Hoon::Sand("tas".to_string(), Noun::Atom(Atom::Small(0)))));
                    }
                    b
                });

    let gash = limp
            .separated_by(just("/"))
            .collect::<Vec<Vec<Option<Hoon>>>>()
            .map(|a| a.into_iter().flatten().collect::<Vec<_>>())
            .boxed();

    let porc = just("%").repeated().count()     //  usize
                .then(just("/").ignore_then(gash.clone())); // Vec<Option<Hoon>>

    let poor = gash.clone()
                .map(|pre| Some(pre))
                    .then(just("%")
                            .ignore_then(porc.clone())
                            .or_not());

    let rood = {
        just("/")
        .ignore_then(poor.try_map(move |(pre, pof), span| {
            match posh(pre, pof, wer1.clone()) {
                Some(list) => Ok(Hoon::ColSig(list)),
                None => Err(Rich::custom(span, "error parsing path")),
            }
        }))
    };

    let cen_fas = {
        porc.try_map(move |(a, b), span| {
            match posh(Some(vec![None]), Some((a, b)), wer2.clone()) {
                Some(list) => Ok(Hoon::ColSig(list)),
                None => Err(Rich::custom(span, "error parsing path")),
            }
        })
    };

    let multi_cen = {
        just("%").repeated().count().try_map(move |n, span| {
            match posh(Some(vec![None]), Some((n, vec![])), wer3.clone()) {
                Some(list) => Ok(Hoon::ColSig(list)),
                None => Err(Rich::custom(span, "error parsing path")),
            }
        })
    };

    let cen_path = just("%").ignore_then(choice((cen_fas, multi_cen)));

    choice((
        rood.boxed(),       //  /foo/%/foo
        cen_path.boxed(),   //  %/foo  and  %%
    )).labelled("Path")
}

pub fn number<'src>(
) -> impl Parser<'src, &'src str, (String, Atom), Err<'src>>
{
    let ud_number = decimal_number()
                    .map(|s|
                        ("ud".to_string(), decimal_to_atom(s)));

    let ux_number = hexadecimal_number()
                    .map(|s|
                        ("ux".to_string(), hex_to_atom(s)));

    let uc_number = bitcoin_address()
                    .try_map(|s, span| {
                        let maybe_base58 = base58_to_atom(s);
                        match maybe_base58 {
                            None => Err(Rich::custom(span, "Invalid BTC address.")),
                            Some(atom) => Ok(("uc".to_string(), atom))
                        }
                    });

    let ub_number = binary_number()
                    .map(|s|
                        ("ub".to_string(), binary_to_atom(s)));

    let uv_number = base32_number()
                    .map(|a|
                        ("uv".to_string(), a));

    let uw_number = base64_number()
                    .map(|a|
                        ("uw".to_string(), a));

    let ui_number =
    just("0i")
        .ignore_then(regex(r"[0-9]+"))
        .map(|s: &str| {
            ("ui".to_string(), decimal_to_atom(s.to_string()))
        });

    let negative = choice((
                hexadecimal_number().map(|s| ("sx".to_string(), hex_to_atom(s))),
                binary_number().map(|s| ("sb".to_string(), binary_to_atom(s))),
                bitcoin_address()
                    .try_map(|s, span| {
                        let maybe_base58 = base58_to_atom(s);
                        match maybe_base58 {
                            None => Err(Rich::custom(span, "Invalid BTC address.")),
                            Some(atom) => Ok(("uc".to_string(), atom))
                        }
                    }),
                base32_number().map(|a| ("sv".to_string(), a)),
                base64_number().map(|a| ("sw".to_string(), a)),
                just("0i").ignore_then(regex(r"[0-9]+"))
                    .map(|s: &str| ("si".to_string(), decimal_to_atom(s.to_string()))),
                decimal_number().map(|s| ("sd".to_string(), decimal_to_atom(s))),
            )).boxed();

    let signed_number = // signed: -num and --num
        just('-')
        .ignore_then(
            just('-')
            .ignore_then(negative.clone().map(|(p, q)| (p, apply_sign(true, q))))
            .or(negative.map(|(p, q)| (p, apply_sign(false, q)))));

    choice((
        signed_number,
        ub_number,
        uc_number,
        ui_number,
        ux_number,
        uv_number,
        uw_number,
        ud_number,
    )).labelled("Number")
}

// decimal without leading 0 and without dots.
//
pub fn decimal_without_dots<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    regex(r"(0|[1-9][0-9]*)").map(str::to_owned)
}

// decimal with leading 0 and without dots.
//
pub fn leading_zero_decimal_without_dots<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    regex(r"([0-9]*)").map(str::to_owned)
}

pub fn absolute_date<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>>
{
    let era_year = decimal_without_dots()
        .then(
            just('-')
                .to(false)
                .or_not()
                .map(|opt| opt.unwrap_or(true)),
        )
        .try_map(|(year_str, era), span| {
            let year: u64 = year_str.parse()
                .map_err(|_| Rich::custom(span, "invalid year number"))?;

            if year == 0 {
                return Err(Rich::custom(span, "year must be ≥ 1"));
            }

            Ok((era, year))
        });
        let month = just('.')
            .ignore_then(regex(r"[0-9]+"))
            .try_map(|s: &str, span| {
                let m: u64 = s.parse().map_err(|_| Rich::custom(span, "invalid month"))?;
                if (1..=12).contains(&m) {
                    Ok(m)
                } else {
                    Err(Rich::custom(span, "month out of range (1–12)"))
                }
            });
        let day = just('.')
            .ignore_then(regex(r"[0-9]+"))
            .try_map(|s: &str, span| {
                let d: u64 = s.parse().map_err(|_| Rich::custom(span, "invalid day"))?;
                if (1..=31).contains(&d) {
                    Ok(d)
                } else {
                    Err(Rich::custom(span, "day out of range (1–31)"))
                }
            });
    let hour_min_secs_fractions =
        just("..")
            .ignore_then(
                leading_zero_decimal_without_dots()
                    .try_map(|h_str, span| {
                        let h: u64 = h_str
                                        .parse::<u64>()
                                        .map_err(|_| Rich::custom(span, "invalid hour"))?;
                        if h < 24 { Ok(h) } else { Err(Rich::custom(span, "hour out of range (0–23)")) }
                    })
                    .then_ignore(just("."))
                    .then(
                        leading_zero_decimal_without_dots()
                        .try_map(|m_str, span| {
                            let m: u64 = m_str
                                .parse::<u64>()
                                .map_err(|_| Rich::custom(span, "invalid minute"))?;
                            if m < 60 {
                                Ok(m)
                            } else {
                                Err(Rich::custom(span, "minute out of range (0–59)"))
                            }
                        }))
                    .then_ignore(just("."))
                    .then(leading_zero_decimal_without_dots()
                          .try_map(|s_str, span| {
                                let s: u64 = s_str
                                    .parse::<u64>()
                                    .map_err(|_| Rich::custom(span, "invalid second"))?;
                                if s < 60 {
                                    Ok(s)
                                } else {
                                    Err(Rich::custom(span, "second out of range (0–59)"))
                                }
                            })))
            .then(
                just("..")
                    .ignore_then(
                        regex(r"[0-9a-fA-F]+")
                            .separated_by(just("."))
                            .at_least(1)
                            .collect::<Vec<&str>>(),
                    )
                    .or_not()
                    .map(|opt| opt.unwrap_or_default()),
            )
            .try_map(|(((h, m), s), frags), span| {
                let mut fractions = Vec::new();

                for f in frags {
                        let val = u16::from_str_radix(f, 16)
                            .map_err(|_| Rich::custom(span, "invalid fraction digits"))?;
                        fractions.push(val);
                    }

                Ok((h, m, s, fractions))
            })
            .or_not()
            .map(|opt| opt.unwrap_or((0, 0, 0, Vec::new())));

    era_year
    .then(month)
    .then(day)
    .then(hour_min_secs_fractions)
    .map(|((((era, y), m), d), (hour, min, sec, f))| {
        Atom::Small(year(era,
                        y,
                        m,
                        d,
                        hour,
                        min,
                        sec,
                        &f
                    ))
    })
}

fn unit_value_pair<'src>(
)-> impl Parser<'src, &'src str, (char, u64), Err<'src>> {
    one_of("dhms")
        .then(
            decimal_without_dots()
            .try_map(|s, span| {
                s.parse::<u64>().map_err(|_| Rich::custom(span, "Invalid Number"))
            })
        )
}

pub fn relative_date<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>> {
    let time_part = unit_value_pair()
        .separated_by(just('.'))
        .at_least(1)
        .collect::<Vec<(char, u64)>>();

    let hex_part = just("..")
        .ignore_then(
            any().filter(|c: &char| c.is_ascii_hexdigit())
                .repeated()
                .exactly(4)
                .collect::<String>()
                .map(|s| u16::from_str_radix(&s, 16).unwrap_or(0))
                .separated_by(just('.'))
                .at_least(1)
                .collect::<Vec<u16>>()
        )
        .or_not()
        .map(|v| v.unwrap_or_default());

    time_part
        .then(hex_part)
        .map(|(pairs, hex_vec):  (Vec<(char, u64)>, Vec<u16>)| {
            let mut days = 0u64;
            let mut hours = 0u64;
            let mut minutes = 0u64;
            let mut seconds = 0u64;

            for (unit, value) in pairs {
                match unit {
                    'd' => days += value,
                    'h' => hours += value,
                    'm' => minutes += value,
                    's' => seconds += value,
                    _ => {},
                }
            }

            Atom::Small(yule(days, hours, minutes, seconds, &hex_vec))
        })
}

// ++year: date -> @da
pub fn year(a: bool, y: u64, m: u64, d: u64, h: u64, min: u64, s: u64, f: &[u16]) -> u128 {
    let yer = if a {
        YEAR_OFFSET + y
    } else {
        // (sub 292.277.024.400 (dec y))
        YEAR_OFFSET - (y - 1)
    };

    let day_count = yawn(yer, m, d);

    yule(day_count, h, min, s, f)
}

pub fn is_leap_year(year: i32) -> bool {
    // Gregorian calendar proleptic
    (year % 4 == 0) && (year % 100 != 0 || year % 400 == 0)
}

pub fn yule(d: u64, h: u64, m: u64, s: u64, f: &[u16]) -> u128 {
    let sec = d * DAY + h * HOR + m * MIT + s;

    let mut fac: u64 = 0;
    let mut muc = 4i32; // starts at 4
    for &val in f.iter().take(4) {
        muc -= 1; // decrement *before* shift
        fac += (val as u64) << (muc as u32 * 16);
    }

    ((sec as u128) << 64) | (fac as u128)
}

fn bloq_bits(bloq: u32) -> u32 {
    if bloq >= 7 {
        panic!("bloq must be < 7 (max 64-bit chunks for u128)");
    }
    1 << bloq
}

pub fn met(bloq: usize, atom: &Atom) -> usize {
    let bits_per_block: usize = 1usize << bloq;

    match atom {
        Atom::Small(n) => {
            if *n == 0 {
                1
            } else {
                let atom_bits: usize = 128 - n.leading_zeros() as usize;
                (atom_bits + bits_per_block - 1) / bits_per_block
            }
        }
        Atom::Big(b) => {
            if b.is_zero() {
                1
            } else {
                let atom_bits: usize = b.bits() as usize;
                (atom_bits + bits_per_block - 1) / bits_per_block
            }
        }
    }
}

// fn met(bloq: u32, atom: u128) -> u32 {
//     let bits = bloq_bits(bloq);
//     if atom == 0 {
//         return 1;
//     }
//     let atom_bits = (128 - atom.leading_zeros()) as u32;
//     (atom_bits + bits - 1) / bits
// }

/// rep: assemble list of Atoms into one Atom using bite spec
///
/// - `bloq`: block size exponent (e.g. 3 → 8-bit blocks)
/// - `step_opt`: number of bloqs to take from each atom; if `None`, defaults to 1 (per Hoon ?^(a a [a *step]))
/// - `list`: slice of Atoms (representing Hoon `(list @)`)
///
/// Semantics:
///   result = Σ_i ( (atom_i & mask) << (i * chunk_bits) )
///   where mask = (1 << chunk_bits) - 1
pub fn rep(bloq: usize, step_opt: Option<usize>, list: &[Atom]) -> Atom {
    let step = step_opt.unwrap_or(1); // default step = 1

    let bloq_size = 1usize << bloq;        // 2^bloq
    let chunk_bits = step * bloq_size;     // bits per item

    if list.is_empty() || chunk_bits == 0 {
        return Atom::Small(0);
    }

    let mut result = BigUint::from(0u32);

    for (i, atom) in list.iter().enumerate() {
        let atom_bu = atom.to_biguint();

        let truncated = if chunk_bits < 128 {
            let mask = (1u128 << chunk_bits) - 1;
            let mask_bu = BigUint::from(mask);
            atom_bu & mask_bu
        } else {
            if atom_bu.bits() as usize <= chunk_bits {
                atom_bu
            } else {
                let mask = ( BigUint::from(1u32) << chunk_bits) - 1u8;
                &atom_bu & mask
            }
        };

        let shifted = if i == 0 {
            truncated
        } else {
            truncated << (i * chunk_bits)
        };

        result += shifted;
    }

    Atom::Big(result)
}

pub fn rap(bloq: usize, chunks: &[u128]) -> Atom {
    if chunks.is_empty() {
        return Atom::Small(0);
    }

    let bits_per_bloq = bloq_bits(bloq as u32) as u64;
    let mut result = BigUint::zero();
    let mut shift = 0u64; // total bits shifted so far

    for &chunk in chunks {
        let width_bloqs = met(bloq, &Atom::Small(chunk)) as u64;
        let width_bits = width_bloqs * bits_per_bloq;

        // Validate that chunk fits in its declared bloq width
        let mask = if width_bits >= 128 {
            u128::MAX
        } else {
            (1u128 << width_bits) - 1
        };
        if chunk & !mask != 0 {
            panic!("atom {:#x} too large for bloq {}", chunk, bloq);
        }

        // Shift left and add
        let chunk_big = BigUint::from(chunk);
        result |= chunk_big << shift;

        shift += width_bits;

        // Early exit if we already know we need BigUint
        if shift > 128 {
            // continue — we'll return Big at the end
        }
    }

    // Now decide which variant to return
    if shift <= 128 {
        let value = result.to_u128().expect("logic error: shift <=128 but not u128");
        Atom::Small(value)
    } else {
        Atom::Big(result)
    }
}

/// Extract `run` bloqs starting at bloq `start`, where each bloq is `2^bloq` bits.
pub fn cut(bloq: usize, start: usize, run: usize, atom: &Atom) -> Atom {
    if run == 0 {
        return Atom::Small(0);
    }

    let bloq_bits = match 1usize.checked_shl(bloq as u32) {
        Some(b) => b,
        None => return Atom::Small(0),
    };

    let bit_start = match start.checked_mul(bloq_bits) {
        Some(s) => s,
        None => return Atom::Small(0),
    };

    let bit_len = match run.checked_mul(bloq_bits) {
        Some(l) => l,
        None => return Atom::Small(0),
    };

    let src_bits = match atom {
        Atom::Small(0) => 0,
        Atom::Small(n) => (128 - n.leading_zeros()) as usize,
        Atom::Big(b) => b.bits() as usize,
    };

    if bit_start >= src_bits {
        return Atom::Small(0);
    }

    let bit_len = cmp::min(bit_len, src_bits - bit_start);
    if bit_len == 0 {
        return Atom::Small(0);
    }

    let shifted = match atom {
        Atom::Small(n) => {
            if bit_start >= 128 {
                Atom::Small(0)
            } else {
                Atom::Small(n >> bit_start)
            }
        }
        Atom::Big(b) => {
            if bit_start == 0 {
                atom.clone()
            } else {
                Atom::from_biguint(b >> bit_start)
            }
        }
    };

    match &shifted {
        Atom::Small(n) => {
            if bit_len >= 128 {
                shifted
            } else {
                let mask = (1u128 << bit_len) - 1;
                Atom::Small(*n & mask)
            }
        }
        Atom::Big(b) => {  // b: &BigUint
            if bit_len <= 128 {
                // Extract low 128 bits manually (portable)
                let low_u128 = {
                    // Convert to u128, but preserve low bits even if truncated
                    // u128::try_from returns Err for >u128::MAX, but we want modulo 2^128
                    // So: take first 2 u64 limbs
                    let mut limbs = b.iter_u64_digits();
                    let lo = limbs.next().unwrap_or(0);
                    let hi = limbs.next().unwrap_or(0);
                    ((hi as u128) << 64) | (lo as u128)
                };
                let mask = if bit_len == 128 {
                    u128::MAX
                } else {
                    (1u128 << bit_len) - 1
                };
                Atom::Small(low_u128 & mask)
            } else {
                // Big mask: (1 << bit_len) - 1
                let mask = (BigUint::one() << bit_len) - BigUint::one();
                // Use & with references to avoid move
                let masked = b & &mask; // &BigUint & &BigUint → BigUint
                Atom::from_biguint(masked)
            }
        }
    }
}

pub fn lsh(bloq: usize, step: usize, atom: &Atom) -> Atom {
    let bits = match step.checked_mul(1usize << bloq) {
        Some(b) => b,
        None => return Atom::Small(0),
    };
    atom_shl(atom, bits)
}

pub fn rsh(bloq: usize, step: usize, atom: &Atom) -> Atom {
    let bits = match step.checked_mul(1usize << bloq) {
        Some(b) => b,
        None => return Atom::Small(0),
    };
    atom_shr(atom, bits)
}

fn lsh_u128(bloq: usize, step: usize, atom: u128) -> u128 {
    let bits = step.checked_mul(1 << bloq).unwrap_or(128);
    if bits >= 128 { 0 } else { atom << bits }
}

fn rsh_u128(bloq: usize, step: usize, atom: u128) -> u128 {
    let bits = step.checked_mul(1 << bloq).unwrap_or(128);
    if bits >= 128 { 0 } else { atom >> bits }
}

fn lsh_big(bloq: usize, step: usize, atom: &BigUint) -> BigUint {
    let bits = step.checked_mul(1 << bloq).unwrap_or(usize::MAX);
    if bits == 0 { atom.clone() } else { atom << bits }
}

fn rsh_big(bloq: usize, step: usize, atom: &BigUint) -> BigUint {
    let bits = step.checked_mul(1 << bloq).unwrap_or(usize::MAX);
    if bits == 0 { atom.clone() } else { atom >> bits }
}

fn end(bloq: usize, step: usize, atom: &Atom) -> Atom {
    let total_bits = match step.checked_mul(1usize << bloq) {
        Some(b) => b,
        None => return Atom::Small(0),
    };
    atom_mask_low_bits(atom, total_bits)
}

fn end_big(bloq: usize, step: usize, atom: &BigUint) -> BigUint {
    let total_bits = match step.checked_mul(1usize << bloq) {
        Some(b) => b as u128,
        None => return BigUint::zero(),
    };
    if total_bits == 0 {
        return BigUint::zero();
    }
    let mask = (BigUint::one() << total_bits) - BigUint::one();
    atom & &mask
}

fn end_u128(bloq: usize, step: usize, atom: u128) -> u128 {
    let total_bits = match step.checked_mul(1usize << bloq) {
        Some(b) => b as u128,
        None => return 0,
    };
    if total_bits >= 128 {
        atom
    } else {
        let mask = (1u128 << total_bits) - 1;
        atom & mask
    }
}

pub const SIS: [[u8; 3]; 256] = [
    *b"doz", *b"mar", *b"bin", *b"wan", *b"sam", *b"lit", *b"sig", *b"hid",
    *b"fid", *b"lis", *b"sog", *b"dir", *b"wac", *b"sab", *b"wis", *b"sib",
    *b"rig", *b"sol", *b"dop", *b"mod", *b"fog", *b"lid", *b"hop", *b"dar",
    *b"dor", *b"lor", *b"hod", *b"fol", *b"rin", *b"tog", *b"sil", *b"mir",
    *b"hol", *b"pas", *b"lac", *b"rov", *b"liv", *b"dal", *b"sat", *b"lib",
    *b"tab", *b"han", *b"tic", *b"pid", *b"tor", *b"bol", *b"fos", *b"dot",
    *b"los", *b"dil", *b"for", *b"pil", *b"ram", *b"tir", *b"win", *b"tad",
    *b"bic", *b"dif", *b"roc", *b"wid", *b"bis", *b"das", *b"mid", *b"lop",
    *b"ril", *b"nar", *b"dap", *b"mol", *b"san", *b"loc", *b"nov", *b"sit",
    *b"nid", *b"tip", *b"sic", *b"rop", *b"wit", *b"nat", *b"pan", *b"min",
    *b"rit", *b"pod", *b"mot", *b"tam", *b"tol", *b"sav", *b"pos", *b"nap",
    *b"nop", *b"som", *b"fin", *b"fon", *b"ban", *b"mor", *b"wor", *b"sip",
    *b"ron", *b"nor", *b"bot", *b"wic", *b"soc", *b"wat", *b"dol", *b"mag",
    *b"pic", *b"dav", *b"bid", *b"bal", *b"tim", *b"tas", *b"mal", *b"lig",
    *b"siv", *b"tag", *b"pad", *b"sal", *b"div", *b"dac", *b"tan", *b"sid",
    *b"fab", *b"tar", *b"mon", *b"ran", *b"nis", *b"wol", *b"mis", *b"pal",
    *b"las", *b"dis", *b"map", *b"rab", *b"tob", *b"rol", *b"lat", *b"lon",
    *b"nod", *b"nav", *b"fig", *b"nom", *b"nib", *b"pag", *b"sop", *b"ral",
    *b"bil", *b"had", *b"doc", *b"rid", *b"moc", *b"pac", *b"rav", *b"rip",
    *b"fal", *b"tod", *b"til", *b"tin", *b"hap", *b"mic", *b"fan", *b"pat",
    *b"tac", *b"lab", *b"mog", *b"sim", *b"son", *b"pin", *b"lom", *b"ric",
    *b"tap", *b"fir", *b"has", *b"bos", *b"bat", *b"poc", *b"hac", *b"tid",
    *b"hav", *b"sap", *b"lin", *b"dib", *b"hos", *b"dab", *b"bit", *b"bar",
    *b"rac", *b"par", *b"lod", *b"dos", *b"bor", *b"toc", *b"hil", *b"mac",
    *b"tom", *b"dig", *b"fil", *b"fas", *b"mit", *b"hob", *b"har", *b"mig",
    *b"hin", *b"rad", *b"mas", *b"hal", *b"rag", *b"lag", *b"fad", *b"top",
    *b"mop", *b"hab", *b"nil", *b"nos", *b"mil", *b"fop", *b"fam", *b"dat",
    *b"nol", *b"din", *b"hat", *b"nac", *b"ris", *b"fot", *b"rib", *b"hoc",
    *b"nim", *b"lar", *b"fit", *b"wal", *b"rap", *b"sar", *b"nal", *b"mos",
    *b"lan", *b"don", *b"dan", *b"lad", *b"dov", *b"riv", *b"bac", *b"pol",
    *b"lap", *b"tal", *b"pit", *b"nam", *b"bon", *b"ros", *b"ton", *b"fod",
    *b"pon", *b"sov", *b"noc", *b"sor", *b"lav", *b"mat", *b"mip", *b"fip",
];

pub const DEX: [[u8; 3]; 256] = [
    *b"zod", *b"nec", *b"bud", *b"wes", *b"sev", *b"per", *b"sut", *b"let", *b"ful", *b"pen", *b"syt", *b"dur", *b"wep", *b"ser", *b"wyl", *b"sun", 
    *b"ryp", *b"syx", *b"dyr", *b"nup", *b"heb", *b"peg", *b"lup", *b"dep", *b"dys", *b"put", *b"lug", *b"hec", *b"ryt", *b"tyv", *b"syd", *b"nex", 
    *b"lun", *b"mep", *b"lut", *b"sep", *b"pes", *b"del", *b"sul", *b"ped", *b"tem", *b"led", *b"tul", *b"met", *b"wen", *b"byn", *b"hex", *b"feb", 
    *b"pyl", *b"dul", *b"het", *b"mev", *b"rut", *b"tyl", *b"wyd", *b"tep", *b"bes", *b"dex", *b"sef", *b"wyc", *b"bur", *b"der", *b"nep", *b"pur", 
    *b"rys", *b"reb", *b"den", *b"nut", *b"sub", *b"pet", *b"rul", *b"syn", *b"reg", *b"tyd", *b"sup", *b"sem", *b"wyn", *b"rec", *b"meg", *b"net", 
    *b"sec", *b"mul", *b"nym", *b"tev", *b"web", *b"sum", *b"mut", *b"nyx", *b"rex", *b"teb", *b"fus", *b"hep", *b"ben", *b"mus", *b"wyx", *b"sym", 
    *b"sel", *b"ruc", *b"dec", *b"wex", *b"syr", *b"wet", *b"dyl", *b"myn", *b"mes", *b"det", *b"bet", *b"bel", *b"tux", *b"tug", *b"myr", *b"pel", 
    *b"syp", *b"ter", *b"meb", *b"set", *b"dut", *b"deg", *b"tex", *b"sur", *b"fel", *b"tud", *b"nux", *b"rux", *b"ren", *b"wyt", *b"nub", *b"med", 
    *b"lyt", *b"dus", *b"neb", *b"rum", *b"tyn", *b"seg", *b"lyx", *b"pun", *b"res", *b"red", *b"fun", *b"rev", *b"ref", *b"mec", *b"ted", *b"rus", 
    *b"bex", *b"leb", *b"dux", *b"ryn", *b"num", *b"pyx", *b"ryg", *b"ryx", *b"fep", *b"tyr", *b"tus", *b"tyc", *b"leg", *b"nem", *b"fer", *b"mer", 
    *b"ten", *b"lus", *b"nus", *b"syl", *b"tec", *b"mex", *b"pub", *b"rym", *b"tuc", *b"fyl", *b"lep", *b"deb", *b"ber", *b"mug", *b"hut", *b"tun", 
    *b"byl", *b"sud", *b"pem", *b"dev", *b"lur", *b"def", *b"bus", *b"bep", *b"run", *b"mel", *b"pex", *b"dyt", *b"byt", *b"typ", *b"lev", *b"myl", 
    *b"wed", *b"duc", *b"fur", *b"fex", *b"nul", *b"luc", *b"len", *b"ner", *b"lex", *b"rup", *b"ned", *b"lec", *b"ryd", *b"lyd", *b"fen", *b"wel", 
    *b"nyd", *b"hus", *b"rel", *b"rud", *b"nes", *b"hes", *b"fet", *b"des", *b"ret", *b"dun", *b"ler", *b"nyr", *b"seb", *b"hul", *b"ryl", *b"lud", 
    *b"rem", *b"lys", *b"fyn", *b"wer", *b"ryc", *b"sug", *b"nys", *b"nyl", *b"lyn", *b"dyn", *b"dem", *b"lux", *b"fed", *b"sed", *b"bec", *b"mun", 
    *b"lyr", *b"tes", *b"mud", *b"nyt", *b"byr", *b"sen", *b"weg", *b"fyr", *b"mur", *b"tel", *b"rep", *b"teg", *b"pec", *b"nel", *b"nev", *b"fes"
];

/// Fetch prefix syllable (Hoon ++tos)
pub fn tos(i: u8) -> &'static [u8; 3] {
    &SIS[i as usize]
}

/// Fetch suffix syllable (Hoon ++tod)
pub fn tod(i: u8) -> &'static [u8; 3] {
    &DEX[i as usize]
}

/// Linear prefix search (Hoon ++ins)
pub fn ins(a: &[u8]) -> Option<u8> {
    if a.len() != 3 {
        return None;
    }

    let key = [a[0], a[1], a[2]];

    for (i, entry) in SIS.iter().enumerate() {
        if *entry == key {
            return Some(i as u8);
        }
    }

    None
}

/// Linear suffix search (Hoon ++ind)
pub fn ind(a: &[u8]) -> Option<u8> {
    if a.len() != 3 {
        return None;
    }

    let key = [a[0], a[1], a[2]];

    for (i, entry) in DEX.iter().enumerate() {
        if *entry == key {
            return Some(i as u8);
        }
    }

    None
}

// +tip:ab
pub fn tip<'src>(
) -> impl Parser<'src, &'src str, u8, Err<'src>>
{
    regex(r"[a-z]{3}")
        .try_map(|s: &str, span| {
            match ins(s.as_bytes()) {
                Some(i) => Ok(i),
                None => Err(Rich::custom(span, format!("invalid prefix syllable '{s}'"))),
            }
        }).labelled("Phonetic Prefix")
}

// +tiq:ab
pub fn tiq<'src>(
) -> impl Parser<'src, &'src str, u8, Err<'src>>
{
    regex(r"[a-z]{3}")
    .try_map(|s: &str, span| {
        match ind(s.as_bytes()) {
            Some(i) => Ok(i),
            None => Err(Rich::custom(span, format!("invalid suffix syllable '{s}'"))),
        }
    }).labelled("Phonetic Suffix")
}

// +hif:ab
pub fn hif<'src>(
) -> impl Parser<'src, &'src str, u16, Err<'src>>
{
    tip()
    .then(tiq())
    .try_map(|(p, q), span| {
        Ok((p as u16) * 256 + (q as u16))
    })
}

pub fn phonemic_name<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>>
{
    let tep =  regex(r"[a-z]{3}")
            .try_map(|s: &str, span| {
                if s == "doz" {
                    return Err(Rich::custom(span, "prefix 'doz' is forbidden"));
                }
                match ins(s.as_bytes()) {
                    Some(i) => Ok(i),
                    None => Err(Rich::custom(span, format!("invalid prefix syllable '{s}'"))),
                }
            }).labelled("Phonetic Prefix");
    let hef = tip()
                .then(tiq())
                .try_map(|(p, q), span| {
                    let val = (p as u16) * 256 + (q as u16);
                    if val == 0 {
                        Err(Rich::custom(span, format!("phonetic is zero")))
                    } else {
                        Ok(val)
                    }
                }).boxed();
    let huf =  hef.clone() // u16
                .then(just('-')
                    .ignore_then(hif())  // u16
                    .repeated()
                    .at_most(3)
                    .collect::<Vec<_>>())
                    .map(|(first, rest)| {
                        std::iter::once(first).chain(rest).collect::<Vec<_>>()
                    })
                    .map(|hefs: Vec<u16>| {
                        let mut acc = BigUint::from(0u32);
                        for &digit in &hefs {
                            acc = (acc << 16) + BigUint::from(digit);
                        }
                        acc
                    });
    let hyf =  hif()
                .separated_by(just('-'))
                .exactly(4)
                .collect::<Vec<_>>()
                .map(|hefs: Vec<u16>| {
                    let mut acc = BigUint::from(0u32);
                    for &digit in &hefs {
                        acc = (acc << 16) + BigUint::from(digit);
                    }
                    acc
                });
    let other = huf
                .then(just("--").ignore_then(gap().or_not())
                        .ignore_then(hyf)
                        .repeated()
                        .at_least(1)
                        .collect::<Vec<_>>())
                .map(|(first, rest)| {
                    std::iter::once(first).chain(rest).collect::<Vec<_>>()
                })
                .map(|hefs: Vec<BigUint>| {
                    let acc = hefs
                                .iter()
                                .fold(BigUint::from(0u32), |acc, d| (acc << 64) + d);
                    Atom::Big(fynd_big(&acc))
                });
    let planet_moon = hef
                    .then(
                        just('-')
                        .ignore_then(hif())
                        .repeated()
                        .at_least(1)
                        .at_most(3)
                        .collect::<Vec<_>>())
                    .map(|(first, rest)| {
                        std::iter::once(first).chain(rest).collect::<Vec<_>>()
                    })
                    .map(|hefs: Vec<u16>| {
                        let mut acc = BigUint::from_u32(0).unwrap();
                        for &digit in &hefs {
                            acc = (acc << 16) + BigUint::from_u32(digit as u32).unwrap();
                        }
                        Atom::Big(fynd_big(&acc))
                    });
    let star = tep
                .then(tiq())
                .try_map(|(p, q), span| {
                    let x = (p as u16) * 256 + (q as u16);
                    Ok(Atom::Small(x as u128))
                });
    let galaxy = tiq().map(|p| Atom::Small(p.into()));

    choice((
            other.labelled("Long Phonemic"),
            planet_moon.labelled("Planet or Moon"),
            star.labelled("Star"),
            galaxy.labelled("Galaxy"),
        ))
}

pub fn phonemic_name_unscrambled<'src>(
) -> impl Parser<'src, &'src str, Atom, Err<'src>>
{
    hif().or(tiq().map(|i| i as u16))
    .then(just('-')
            .ignore_then(gap().or_not())
            .ignore_then(hif())
            .repeated()
            .collect::<Vec<_>>())
    .map(|(first, rest)| {
        std::iter::once(first)
            .chain(rest)
            .map(Atom::from)
            .collect::<Vec<Atom>>()
    })
    .map(|mut hifs| {
        hifs.reverse();
        rep(4, None, &hifs)
    })
}

fn dis_big(x: &BigUint, mask: &BigUint) -> BigUint {
    x & mask
}

fn dis(x: u64, mask: u64) -> u64 {
    x & mask
}

fn con(hi: u64, lo: u64) -> u64 {
    hi | lo
}

fn con_atoms(hi: Atom, lo: Atom) -> Atom {
    match (hi, lo) {
        (Atom::Small(a), Atom::Small(b)) => Atom::Small(a | b),
        (a, b) => {
            let x = a.to_biguint();
            let y = b.to_biguint();
            Atom::from_biguint(x | y)
        }
    }
}

const RAKU: [u32; 4] = [
    0xb76d_5eed,
    0xee28_1300,
    0x85bc_ae01,
    0x4b38_7af7,
];
#[inline]
fn rol32(x: u32, r: u32) -> u32 {
    x.rotate_left(r)
}

#[inline]
fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

fn muk(seed: u32, len: u32, key: u64) -> u32 {
    let c1: u32 = 0xcc9e_2d51;
    let c2: u32 = 0x1b87_3593;

    // EXACTLY len bytes, little-endian
    let mut data = vec![0u8; len as usize];
    let mut k = key;
    for i in 0..len as usize {
        data[i] = (k & 0xff) as u8;
        k >>= 8;
    }

    let nblocks = (len / 4) as usize; // intentionally off-by-one
    let mut h1 = seed;

    let mut blocks = Vec::new();
    for i in 0..nblocks {
        let mut v = 0u32;
        for j in 0..4 {
            let idx = i * 4 + j;
            if idx < data.len() {
                v |= (data[idx] as u32) << (8 * j);
            }
        }
        blocks.push(v);
    }

    let mut i = nblocks;
    while i > 0 {
        let mut k1 = blocks[nblocks - i];
        k1 = k1.wrapping_mul(c1);
        k1 = k1.rotate_left(15);
        k1 = k1.wrapping_mul(c2);

        h1 ^= k1;
        h1 = h1.rotate_left(13);
        h1 = h1.wrapping_mul(5).wrapping_add(0xe654_6b64);
        i -= 1;
    }

    let tail = &data[(nblocks * 4)..];
    let mut k1 = 0u32;

    match len & 3 {
        3 => {
            k1 ^= (tail[2] as u32) << 16;
            k1 ^= (tail[1] as u32) << 8;
            k1 ^= tail[0] as u32;
            k1 = k1.wrapping_mul(c1);
            k1 = k1.rotate_left(15);
            k1 = k1.wrapping_mul(c2);
            h1 ^= k1;
        }
        2 => {
            k1 ^= (tail[1] as u32) << 8;
            k1 ^= tail[0] as u32;
            k1 = k1.wrapping_mul(c1);
            k1 = k1.rotate_left(15);
            k1 = k1.wrapping_mul(c2);
            h1 ^= k1;
        }
        1 => {
            k1 ^= tail[0] as u32;
            k1 = k1.wrapping_mul(c1);
            k1 = k1.rotate_left(15);
            k1 = k1.wrapping_mul(c2);
            h1 ^= k1;
        }
        _ => {}
    }

    h1 ^= len;
    fmix32(h1)
}

fn eff(j: u64, r: u64) -> u64 {
    let seed = RAKU[(j as usize) & 3];
    muk(seed, 2, r) as u64
}

fn fen(
    r: u64,
    a: u64,
    b: u64,
    m: u64,
) -> u64 {
    let mut j = r;

    let (ahh, ale) = if r % 2 == 0 {
        (m % a, m / a)
    } else {
        (m / a, m % a)
    };

    let (mut ell, mut arr) = if ale == a {
        (ahh, ale)
    } else {
        (ale, ahh)
    };

    while j >= 1 {
        let f = eff(j - 1, ell);

        let tmp = if j % 2 != 0 {
            (arr + a - (f % a)) % a
        } else {
            (arr + b - (f % b)) % b
        };

        j -= 1;
        arr = ell;
        ell = tmp;
    }

    &arr * a + ell
}

fn feen(r: u64, a: u64, b: u64, k: u64, m: u64) -> u64 {
    let c = fen(r, a, b, m);
    if c < k.into() {
        c
    } else {
        fen(r, a, b, c)
    }
}

/* === tail === */
fn tail(m: u64) -> u64 {
    feen(
        4,
        0xffff,
        0x1_0000,
        0xffff * 0x1_0000,
        m,
    )
}

fn fynd_big(cry: &BigUint) -> BigUint {
    let one_16 = BigUint::from(0x1_0000u32);
    let max_32 = BigUint::from(0xffff_ffffu32);
    let one_32 = BigUint::from(0x1_0000_0000u64);
    let max_64 = BigUint::from(u64::MAX);

    // 17–32 bits: [0x1_0000 .. 0xffff_ffff]
    if cry >= &one_16 && cry <= &max_32 {
        let x = cry.to_u64().unwrap();
        return BigUint::from(fynd_u64(x));
    }

    // 33–64 bits: [0x1_0000_0000 .. 0xffff_ffff_ffff_ffff]
    if cry >= &one_32 && cry <= &max_64 {
        let lo = cry & &max_32;        // low 32 bits
        let hi = cry - &lo;            // high bits (≥32)
        let lo_f = BigUint::from(fynd_u64(lo.to_u64().unwrap()));
        return hi + lo_f;
    }

    // All other values: passthrough (including >64-bit)
    cry.clone()
}

/* === fynd === */
pub fn fynd_u64(cry: u64) -> u64 {
    // 17–32 bits
    if cry >= 0x1_0000 && cry <= 0xffff_ffff {
        return 0x1_0000 + tail(cry - 0x1_0000);
    }

    // 33–64 bits
    if cry >= 0x1_0000_0000 && cry <= 0xffff_ffff_ffff_ffff {
        let lo = dis(cry, 0xffff_ffff);
        let hi = dis(cry, 0xffff_ffff_0000_0000);
        return con(hi, fynd_u64(lo));
    }

    cry
}

pub fn twid<'src>(
) -> impl Parser<'src, &'src str, Coin, Err<'src>>
{
    choice((
        just('0')
            .ignore_then(regex(r"[0-9a-v]+"))
             .try_map(|s: &str, span| {
                    let atom = base32_to_atom(s.to_string());
                    cue_simple(atom)
                    .map(Coin::Blob)
                    .map_err(|e| Rich::custom(span, format!("Failed to +cue: {}", e)))
                }),
        crub(),
    ))
}

pub fn cue_simple(buffer: Atom) -> Result<Noun, Box<dyn std::error::Error>> {
    let bits = atom_to_bits(&buffer);
    let mut backrefs = HashMap::new();
    let (noun, _) = cue_inner(&bits, 0, &mut backrefs)?;
    Ok(noun)
}

fn noun_hash(noun: &Noun) -> u64 {
    let mut hasher = DefaultHasher::new();
    noun.hash(&mut hasher);
    hasher.finish()
}

pub fn jam_simple(noun: Noun) -> Atom {
    let mut bits = Vec::new();
    let mut backrefs = HashMap::new(); // Noun → bit_offset
    let mut stack = vec![noun];

    while let Some(current) = stack.pop() {
        if let Some(&offset) = backrefs.get(&current) {
            let use_backref = match &current {
                Noun::Atom(atom) => {
                    let atom_bits = mat_bits(atom).len();
                    let offset_bits = mat_bits(&offset_to_atom(offset)).len();
                    offset_bits < atom_bits
                }
                Noun::Cell(_, _) => true,
            };

            if use_backref {
                bits.push(true);  // 1
                bits.push(true);  // 1
                bits.extend(mat_bits(&offset_to_atom(offset)));
                continue;
            }
        }

        let offset = bits.len();
        backrefs.insert(current.clone(), offset);

        match current {
            Noun::Atom(atom) => {
                bits.push(false);
                bits.extend(mat_bits(&atom));
            }
            Noun::Cell(head, tail) => {
                bits.push(true);
                bits.push(false);
                stack.push(*tail);
                stack.push(*head);
            }
        }
    }

    bits_to_atom(&bits)
}

fn offset_to_atom(offset: usize) -> Atom {
    if offset <= u128::MAX as usize {
        Atom::Small(offset as u128)
    } else {
        Atom::Big(BigUint::from(offset))
    }
}

fn mat_bits(atom: &Atom) -> Vec<bool> {
    let n = atom_bit_len(atom); // = met0(atom): number of bits needed to represent the atom

    let mut bits = Vec::new();

    if n == 0 {
        bits.push(true);
        return bits;
    }

    let k = usize_bit_len(n); // met0(n)

    bits.extend(std::iter::repeat(false).take(k));

    bits.push(true);

    if k > 1 {
        let offset = n - (1usize << (k - 1)); // same as n & ((1 << (k-1)) - 1)
        for i in 0..(k - 1) {
            bits.push((offset >> i) & 1 == 1);
        }
    }

    for i in 0..n {
        bits.push(atom_get_bit(atom, i as u64));
    }

    bits
}

fn usize_bit_len(x: usize) -> usize {
    if x == 0 { 1 } else { (usize::BITS - x.leading_zeros()) as usize }
}

fn atom_bit_len(atom: &Atom) -> usize {
    match atom {
        Atom::Small(0) => 0,
        Atom::Small(x) => (128 - x.leading_zeros() as usize),
        Atom::Big(x) => x.bits() as usize,
    }
}

fn atom_get_bit(atom: &Atom, i: u64) -> bool {
    match atom {
        Atom::Small(x) => i < 128 && ((x >> i) & 1 == 1),
        Atom::Big(x) => {
            let byte_index = (i / 8) as usize;
            let bit_index = (i % 8) as u8;
            let bytes = x.to_bytes_le();
            if byte_index < bytes.len() {
                let byte = bytes[byte_index];
                (byte >> bit_index) & 1 == 1
            } else {
                false
            }
        }
    }
}

fn bits_to_atom(bits: &[bool]) -> Atom {
    if bits.is_empty() {
        return Atom::Small(0);
    }

    // Count bits
    let len = bits.len();

    if len <= 128 {
        let mut val: u128 = 0;
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                val |= 1u128 << i;
            }
        }
        Atom::Small(val)
    } else {
        let mut big = BigUint::from(0u32);
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                big += BigUint::from(1u32) << i;
            }
        }
        Atom::Big(big)
    }
}

#[derive(Debug)]
enum ParseAction {
    Start(u64),           // start parsing noun at cursor
    CellHeadDone(u64, Box<Noun>), // head done, now parse tail at given cursor
    FinishCell(Box<Noun>, Box<Noun>),
    StoreBackref(u64),
}
fn rub_backref(bits: &[bool], cursor: &mut usize) -> Result<u64, Box<dyn std::error::Error>> {
    let size = get_size(bits, cursor)?;
    if size == 0 {
        return Ok(0);
    }
    if size > 64 {
        return Err("backref offset too large (>64 bits)".into());
    }
    if *cursor + size as usize > bits.len() {
        return Err("not enough bits for backref".into());
    }

    let mut val: u64 = 0;
    for i in 0..size {
        if bits[*cursor + i as usize] {
            val |= 1u64 << i;
        }
    }
    *cursor += size as usize;
    Ok(val)
}

fn rub_atom(bits: &[bool], cursor: &mut usize) -> Result<Atom, Box<dyn std::error::Error>> {
    let size = get_size(bits, cursor)?;

    if size == 0 {
        return Ok(Atom::Small(0));
    }

    if *cursor + size as usize > bits.len() {
        return Err("not enough bits for rub atom payload".into());
    }

    // Read `size` bits, LSB-first → value = sum bit_i * 2^i
    if size <= 128 {
        let mut val: u128 = 0;
        for i in 0..size {
            if bits[*cursor + i as usize] {
                val |= 1u128 << i;
            }
        }
        *cursor += size as usize;
        Ok(Atom::Small(val))
    } else {
        // Use BigUint
        let mut big = BigUint::from(0u32);
        for i in 0..size {
            if bits[*cursor + i as usize] {
                big += BigUint::from(1u32) << i;
            }
        }
        *cursor += size as usize;
        Ok(Atom::Big(big))
    }
}

fn get_size(bits: &[bool], cursor: &mut usize) -> Result<u64, &'static str> {
    let start = *cursor;
    // Count leading zeros
    while *cursor < bits.len() && !bits[*cursor] {
        *cursor += 1;
    }

    if *cursor >= bits.len() {
        return Err("unexpected EOF in rub size prefix");
    }

    let c = (*cursor - start) as u32; // number of leading zeros
    *cursor += 1; // consume the '1'

    if c == 0 {
        Ok(0)
    } else {
        // Read c-1 bits
        if *cursor + (c - 1) as usize > bits.len() {
            return Err("not enough bits for rub size field");
        }

        let mut x = 0u64;
        for i in 0..(c - 1) {
            if bits[*cursor + i as usize] {
                x |= 1u64 << i; // LSB-first: first bit = 2^0
            }
        }
        *cursor += (c - 1) as usize;

        let size = (1u64 << (c - 1)) + x;
        Ok(size)
    }
}

fn atom_to_bits(atom: &Atom) -> Vec<bool> {
    match atom {
        Atom::Small(x) => {
            let mut bits = Vec::with_capacity(128);
            for i in 0..128 {
                bits.push((x >> i) & 1 == 1);
            }
            // Trim trailing zeros beyond highest set bit? Not needed — cue stops when done.
            bits
        }
        Atom::Big(x) => {
            // Convert to little-endian bytes, then bits
            let bytes = x.to_bytes_le();
            let mut bits = Vec::new();
            for &byte in &bytes {
                for i in 0..8 {
                    bits.push((byte >> i) & 1 == 1);
                }
            }
            // Pad to next multiple of 8? Not necessary.
            bits
        }
    }
}

fn cue_inner( // rename
    bits: &[bool],
    cursor: usize,
    backrefs: &mut HashMap<u64, Noun>,
) -> Result<(Noun, usize), Box<dyn std::error::Error>> {
    if cursor >= bits.len() {
        return Err("unexpected EOF".into());
    }

    let tag0 = bits[cursor];
    if !tag0 {
        let mut cur = cursor + 1;
        let atom = rub_atom(bits, &mut cur)?;
        let noun = Noun::Atom(atom);
        backrefs.insert(cursor as u64, noun.clone());
        Ok((noun, cur))
    } else {
        if cursor + 1 >= bits.len() {
            return Err("unexpected EOF after tag 1".into());
        }
        let tag1 = bits[cursor + 1];
        if !tag1 {
            let mut cur = cursor + 2;
            let (head, next) = cue_inner(bits, cur, backrefs)?;
            cur = next;
            let (tail, next2) = cue_inner(bits, cur, backrefs)?;
            cur = next2;
            let noun = Noun::Cell(Box::new(head), Box::new(tail));
            backrefs.insert(cursor as u64, noun.clone());
            Ok((noun, cur))
        } else {
            let mut cur = cursor + 2;
            let offset = rub_backref(bits, &mut cur)?;

            let noun = backrefs
                .get(&(offset))
                .cloned()
                .ok_or_else(|| format!("backref to {} not found", offset))?;
            Ok((noun, cur))
        }
    }
}

pub fn crub<'src>(
) -> impl Parser<'src, &'src str, Coin, Err<'src>>
{
    choice((
            absolute_date().map(|d| Coin::Dime("da".to_string(), d)),
            relative_date().map(|d| Coin::Dime("dr".to_string(), d)),
            phonemic_name().map(|p| Coin::Dime("p".to_string(), p)),
            just('.')
                .ignore_then(urs())
                .map(|atom| Coin::Dime("ta".to_string(), atom)),
            just('~')
                .ignore_then(urx())
                .map(|atom| Coin::Dime("t".to_string(), atom)),
            just('-')
                .ignore_then(urx())
                .map(|atom| Coin::Dime("c".to_string(), taft(&atom))),
    ))
}

//  +rump: name/hoon or name+hoon
//
pub fn constant_separator_hoon<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    choice((
        just('$').to(Hoon::Rock("tas".to_string(), Noun::Atom(Atom::Small(0)))),
        symbol().map(|s| Hoon::Rock("tas".to_string(), Noun::Atom(string_to_atom(s)))),
        number().map(|(p, q)| Hoon::Rock(p, Noun::Atom(q))),
        just('&').to(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(0)))),
        just('|').to(Hoon::Rock("f".to_string(), Noun::Atom(Atom::Small(1)))),
        just('~').to(Hoon::Bust(BaseType::Null)),
    ))
    .then(just('+').or(just('/'))
            .ignore_then(hoon.clone()))
    .map(|(p, hoon)| Hoon::Pair(Box::new(p), Box::new(hoon)))
}

//  `@p`q
//
pub fn tic_aura<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    aura_text()
    .then_ignore(just("`"))
    .then(hoon_wide.clone())
    .map(|(a, b)| {
        Hoon::KetLus(
            Box::new(Hoon::Sand(a, Noun::Atom(Atom::Small(0)))),
            Box::new(Hoon::KetLus(Box::new(Hoon::Sand("$".to_string(), Noun::Atom(Atom::Small(0)))), Box::new(b))),
        )})
}

pub fn tic_cell_construction<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
        .map(|h| Hoon::Pair(Box::new(Hoon::Rock("n".to_string(),
                                                    Noun::Atom(Atom::Small(0)))),
                                 Box::new(h)))
}

pub fn parenthesis_spec<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    hoon_wide.clone()
        .then(
            just(' ')
            .ignore_then(spec_wide.clone())
                .repeated()
                .collect::<Vec<_>>()
                .or_not()
                .map(|specs| specs.unwrap_or_default())
        )
    .delimited_by(just('('), just(')'))
    .map(|(name, specs)| Spec::Make(name, specs))
}

pub fn reference_spec<'src>(
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    regex(r"([a-z][a-zA-Z0-9]*)|[\$\^\,]").to(())
    .rewind()
    .ignore_then(
            winglist()
            .separated_by(just(':'))
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|wings: Vec<WingType>| {
                        let (first, rest) = wings.split_first().unwrap();
                        Spec::Like(first.clone(), rest.to_vec())
                    })
        )
}

pub fn two_hoons_tall<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, (Hoon, Hoon), Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
}

pub fn two_hoons_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, (Hoon, Hoon), Err<'src>>
{
    hoon_wide.clone()
    .then_ignore(just(' '))
    .then(hoon_wide.clone())
}

pub fn three_hoons_tall<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, ((Hoon, Hoon), Hoon), Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
}

pub fn three_hoons_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, ((Hoon, Hoon), Hoon), Err<'src>>
{
    hoon_wide.clone()
    .then_ignore(just(' '))
    .then(hoon_wide.clone())
    .then_ignore(just(' '))
    .then(hoon_wide.clone())
}

pub fn two_specs_tall<'src>(
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Spec, Spec), Err<'src>>
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(spec.clone())
}

pub fn two_specs_closed_tall<'src>(
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Spec, Spec), Err<'src>>
{
    two_specs_tall(spec.clone())
    .then_ignore(gap())
    .then_ignore(just("=="))
}

pub fn two_specs_closed_wide<'src>(
    spec_wide:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Spec, Spec), Err<'src>>
{
    spec_wide.clone()
    .then_ignore(just(' '))
    .then(spec_wide.clone())
    .delimited_by(just('('), just(')'))
}

pub fn hoon_spec_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
    spec_wide:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Hoon, Spec), Err<'src>>
{
    hoon_wide.clone()
    .then_ignore(just(' '))
    .then(spec_wide.clone())
    .delimited_by(just('('), just(')'))
}

pub fn hoon_spec_tall<'src>(
    hoon:           impl ParserExt<'src, Hoon>,
    spec:           impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Hoon, Spec), Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(spec.clone())
}

pub fn spec_hoon_tall<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Spec, Hoon), Err<'src>>
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
}

pub fn spec_hoon_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
    spec_wide:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Spec, Hoon), Err<'src>>
{
    spec_wide.clone()
    .then_ignore(just(' '))
    .then(hoon_wide.clone())
}

pub fn name_spec_tall<'src>(
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src,  &'src str, (String, Spec), Err<'src>>
{
    gap()
    .ignore_then(symbol())
    .then_ignore(gap())
    .then(spec.clone())
}

pub fn name_spec_closed_tall<'src>(
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (String, Spec), Err<'src>>
{
    gap()
    .ignore_then(symbol())
    .then_ignore(gap())
    .then(spec.clone())
    .then_ignore(just("=="))
}

pub fn name_spec_wide<'src>(
    spec_wide:        impl ParserExt<'src, Spec> + Clone,
) -> impl Parser<'src, &'src str, (String, Spec), Err<'src>>
{
    symbol()
    .then_ignore(just(' '))
    .then(spec_wide.clone())
    .delimited_by(just('('), just(')'))
}

pub fn one_hoon_closed_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
    .delimited_by(just('('), just(')'))
}

pub fn one_hoon_closed_tall<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .delimited_by(just('='), just('='))
}

pub fn one_spec_closed_wide<'src>(
    spec_wide:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    spec_wide.clone()
    .delimited_by(just('('), just(')'))
}

pub fn one_spec_closed_tall<'src>(
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .delimited_by(just('='), just('='))
}

pub fn wrap_hoon_with_trace(
    wer: Path,
    linemap: Arc<LineMap>,
) -> impl for<'src> Fn(
        Hoon,
        &mut MapExtra<'src, '_, &'src str, Err<'src>>,
    ) -> Hoon
    + Clone
{
    move |node, e| {
        Hoon::Dbug(
            chumsky_spot_to_hoon_spot((e.span().start(), e.span().end()), &wer, &linemap),
            Box::new(node),
        )
    }
}

pub fn wrap_spec_with_trace(
    wer: Path,
    linemap: Arc<LineMap>,
) -> impl for<'src> Fn(
        Spec,
        &mut MapExtra<'src, '_, &'src str, Err<'src>>,
    ) -> Spec
    + Clone
{
    move |node, e| {
        Spec::Dbug(
            chumsky_spot_to_hoon_spot((e.span().start(), e.span().end()), &wer, &linemap),
            Box::new(node),
        )
    }
}

fn chumsky_spot_to_hoon_spot(
    span: (usize, usize),
    wer: &Path,
    linemap: &Arc<LineMap>,
) -> Spot {
    let (start, end) = span;

    let (sl, sc) = linemap.line_col(start);
    let (el, ec) = linemap.line_col(end);

    Spot {
        p: wer.clone(),
        q: Pint {
            p: (sl as u64, sc as u64),
            q: (el as u64, ec as u64),
        },
    }
}
