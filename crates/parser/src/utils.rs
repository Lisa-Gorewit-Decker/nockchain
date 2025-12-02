use std::collections::*;
use crate::ast::hoon::*;
use std::sync::Arc;
use std::path::PathBuf;
use std::collections::HashMap;
use chumsky::{
    input::{Stream, ValueInput, Input, StrInput},
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
            let literal = if a == "%da" {
                "~2000.1.1".to_string()
            } else {
                "0".to_string()
            };
            Hoon::Sand(a, Noun::Atom(literal))
        }
        BaseType::Noun => {
            let rock0 = Box::new(Hoon::Rock("%$".to_string(), Noun::Atom("0".to_string())));
            let rock1 = Box::new(Hoon::Rock("%$".to_string(), Noun::Atom("1".to_string())));
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
            let rock0 = Box::new(Hoon::Rock("%$".to_string(), Noun::Atom("0".to_string())));
            let rock0_clone = rock0.clone();
            let rock1_clone = rock0.clone();
            Hoon::KetLus(Box::new(Hoon::DotTis(rock0, rock0_clone)), rock1_clone)
        }
        BaseType::Null => Hoon::Rock("%$".to_string(), Noun::Atom("0".to_string())),
        BaseType::Void => Hoon::ZapZap,
    }
}

pub fn function(
    fun: Spec,
    arg: Spec,
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<Term, Spec>,
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
    arms: HashMap<Term, Spec>,
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<Term, Spec>,
    bug: &Vec<Spot>,
    nut: &Option<Note>,
    def: &Option<Hoon>,
) -> Hoon {

    let map: HashMap<Term, Hoon> = arms.into_iter()
        .map(|(term, spec)|
                 (term, example(&spec, dom, hay, cox, &vec![], &None, &None)))
        .collect();
    let brcn = Hoon::BarCen(
        None,
        HashMap::from([("%$".to_string(), map)]),
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
                cox: HashMap<Term, Spec>,
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
                cox: HashMap<Term, Spec>,
                bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    match spec {
        Spec::Base(b) => {
            match b {
                BaseType::Void => Hoon::Rock("%n".to_string(), Noun::Atom("0".to_string())),
                _ => basal(b),
            }
        }
        Spec::BucBuc(s, map) => {
            let mut new_cox = cox;
            new_cox.extend(map);
            new_cox.insert("%$".to_string(), *s.clone());
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
        Spec::BucCab(_) => Hoon::Rock("%n".to_string(), Noun::Atom("0".to_string())),
        Spec::BucCol(spec, specs) => spore_buccol_recursion(*spec, specs, dom, hay, cox, bug, nut, def),
        Spec::BucCen(spec, specs) => spore_buccen_recursion(*spec, specs, dom, hay, cox, bug, nut, def),
        Spec::BucHep(spec, specs) => Hoon::Rock("%n".to_string(), Noun::Atom("0".to_string())),
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
         => Hoon::Rock("%n".to_string(), Noun::Atom("0".to_string())),
    }
}

pub fn spore_buccol_recursion(spec: Spec,
                list_spec: Vec<Spec>,
                dom: u64,
                hay: WingType,
                cox: HashMap<Term, Spec>,
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
                cox: HashMap<Term, Spec>,
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
                cox: HashMap<Term, Spec>,
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
    cox: &HashMap<Term, Spec>,
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
                            Box::new(Hoon::Limb("%$".to_string())),
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
    arms: HashMap<Term, Spec>,
    mod_: &Spec,
    dom: u64,
    hay: &WingType,
    cox: &HashMap<Term, Spec>,
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
                cox: &HashMap<Term, Spec>,
                bug: &Vec<Spot>,
                nut: &Option<Note>,
                def: &Option<Hoon>) -> Hoon {
    match bas {
        BaseType::Atom(a) => {
            let cnls = Hoon::CenLus(Box::new(Hoon::Limb("%ruth".to_string())),
                                    Box::new(Hoon::Sand("%ta".to_string(), Noun::Atom(a))),
                                    Box::new(Hoon::Axis(axe)));

            let example_res = Box::new(example(mod_, dom, hay, cox, bug, nut, def));

            let wtpt_limb = Limb::Axis(axe);
            let wtpt_wing: Vec<Limb> = vec![wtpt_limb];
            let wtpt = Hoon::WutPat(wtpt_wing, Box::new(Hoon::Axis(axe)), Box::new(Hoon::ZapZap));

            let zppt_limb = Limb::Parent(0, Some("%ruth".to_string()));
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
            let rock = Box::new(Hoon::Rock("%f".to_string(), Noun::Atom("&".to_string())));
            let dtts = Box::new(Hoon::DotTis(
                                    Box::new(Hoon::Rock("%$".to_string(), Noun::Atom("&".to_string()))),
                                    Box::new(Hoon::Axis(axe))
                                ));
            let wtgr = Box::new(Hoon::WutGar(
                            Box::new(Hoon::DotTis(
                                Box::new(Hoon::Rock("%$".to_string(), Noun::Atom("|".to_string()))),
                                Box::new(Hoon::Axis(axe))
                            )),
                            Box::new(Hoon::Rock("%f".to_string(), Noun::Atom("|".to_string()))))
                        );
            Hoon::WutCol(dtts, rock, wtgr)
        },
        BaseType::Null => {
            let rock = Box::new(Hoon::Rock("%n".to_string(), Noun::Atom("~".to_string())));
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
    cox: &HashMap<Term, Spec>,
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
            cox: &HashMap<Term, Spec>,
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
                cox: &HashMap<Term, Spec>,
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
                                          Box::new(Hoon::Rock("%$".to_string(), Noun::Atom(q.clone()))))),
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
            let map: HashMap<Term, Hoon> = q.into_iter()
                .map(|(term, spec)| (term.clone(), relative(axe, spec, new_dom, hay, cox, bug, nut, def)))
                .collect();
            Hoon::BarKet(
                Box::new(relative(axe, &*p, new_dom, hay, cox, bug, nut, def)),
                HashMap::from([("%$".to_string(), map)]),
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

//  TODO: accept args by ref?
pub fn factory(mod_: Spec,
                dom: u64,
                hay: WingType,
                cox: HashMap<Term, Spec>,
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
                            let sand = Hoon::Sand("tD".to_string(), Noun::Atom(a.to_string()));
                            Hoon::Pair(Box::new(sand), Box::new(tail))
                        }
                        Woof::Hoon(p) => {
                            let a = Hoon::Pair(
                                        Box::new(Hoon::KetTis(
                                                Skin::Term("a".to_string()),
                                                Box::new(Hoon::KetLus(
                                                                Box::new(Hoon::Limb("%$".to_string())),
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
                                        Box::new(Hoon::Sand("tD".to_string(), Noun::Atom("0".to_string()))),
                                    )),
                                    Box::new(Hoon::KetTis(
                                        Skin::Term("t".to_string()),
                                        Box::new(Hoon::Limb("%$".to_string())),
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
                    let wrapped_pairs: Vec<(Term, Hoon)> = tome
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
                m.insert("%$".to_string(), *p);
                m
            };
            let map_term_tome = {
                let mut m = HashMap::new();
                m.insert("%$".to_string(), map_term_hoon);
                m
            };
            Hoon::BarCen(None, map_term_tome)
        }

        Hoon::BarKet(p, arms) => {
            let mut map = arms.clone();
            if let Some(zil) = arms.get(&"%$".to_string()) {
                let updated = {
                    let mut inner = zil.clone();
                    inner.insert("%$".to_string(), *p.clone());
                    inner
                };
                map.insert("%$".to_string(), updated);
            } else {
                let mut inner = HashMap::new();
                inner.insert("%$".to_string(), *p.clone());
                map.insert("%$".to_string(), inner);
            }
            Hoon::TisGal(
                Box::new(Hoon::Limb("%$".to_string())),
                Box::new(Hoon::BarCen(None, map)),
            )
        }

        Hoon::BarHep(p) => Hoon::TisGal(Box::new(Hoon::Limb("%$".to_string())), Box::new(Hoon::BarDot(Box::new(*p)))),

        Hoon::BarSig(spec, q) => Hoon::KetBar(Box::new(Hoon::BarTis(spec.clone(), q.clone()))),

        Hoon::BarTar(spec, q) => {
            let map_term_hoon = {
                let mut m = HashMap::new();
                m.insert("%$".to_string(), *q);
                m
            };
            let map_term_tome = {
                let mut m = HashMap::new();
                m.insert("%$".to_string(), map_term_hoon);
                m
            };
            Hoon::TisLus(Box::new(Hoon::KetTar(spec)),
                        Box::new(Hoon::BarPat(None, map_term_tome)))
        }

        Hoon::BarTis(spec, q) => {
            let map_term_hoon = {
                let mut m = HashMap::new();
                m.insert("%$".to_string(), *q);
                m
            };
            let map_term_tome = {
                let mut m = HashMap::new();
                m.insert("%$".to_string(), map_term_hoon);
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
                [] => Hoon::Rock("n".to_string(), Noun::Atom("0".to_string())),
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
            Hoon::CenSig(vec![Limb::Term("%$".to_string())], p, hoons)
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
            TermOrPair::Term(Term::from("mean")),
            Box::new(Hoon::BarDot(p)),
        ),

        Hoon::SigCen(chum, p, tyre, q) => {
            let clsg_vec = {
                let mut nob = vec![];
                let mut r = tyre;
                while !r.is_empty() {
                    let (p_i, q_i) = r.remove(0);
                    nob.push(Hoon::Pair(
                        Box::new(Hoon::Rock("$".to_string(), Noun::Atom(p_i))),
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
            TermOrPair::Pair("live".to_string(), Box::new(Hoon::Rock("$".to_string(), Noun::Atom(term)))),
            q
        ),

        Hoon::SigLus(a, q) => Hoon::SigGar(
            TermOrPair::Pair("memo".to_string(), Box::new(Hoon::Rock("$".to_string(), Noun::Atom(a.to_string())))),
            q
        ),

        Hoon::SigPam(a, p, q) => Hoon::SigGar(
            TermOrPair::Pair(
                "slog".to_string(),
                Box::new(Hoon::Pair(
                    Box::new(Hoon::Sand("$".to_string(), Noun::Atom(a.to_string()))),
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
                                        m.insert("%$".to_string(), wtsg);
                                        m
                                    };
                                    let map_term_tome = {
                                        let mut m = HashMap::new();
                                        m.insert("%$".to_string(), map_term_hoon);
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
            let zoy = Hoon::Rock("ta".to_string(), Noun::Atom("$".to_string()));
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
                [] => Hoon::Rock("f".to_string(), Noun::Atom("1".to_string())),
                [head, tail @ ..] => {
                    let recurse = open(Hoon::WutBar(tail.to_vec()));
                    Hoon::WutCol(
                        Box::new(head.clone()),
                        Box::new(Hoon::Rock("f".to_string(), Noun::Atom("0".to_string()))),
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
        let WutTis = Hoon::WutTis(
            Box::new(Spec::Base(BaseType::Atom("%$".to_string()))),
            p,
        );
        Hoon::WutCol(
            Box::new(WutTis),
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
            [] => Hoon::Rock("f".to_string(), Noun::Atom("0".to_string())),
            [head, tail @ ..] => {
                let recurse = open(Hoon::WutPam(tail.to_vec()));
                Hoon::WutCol(
                    Box::new(head.clone()),
                    Box::new(recurse),
                    Box::new(Hoon::Rock("f".to_string(), Noun::Atom("1".to_string()))),
                )
            }
        }
    },

    Hoon::Xray(manx) => {
        let open_mane = match &manx.g.n {
            Mane::Tag(s) => Hoon::Rock("tas".to_string(), Noun::Atom(s.clone())),
            Mane::TagSpace(a, b) => {
                let left = Hoon::Rock("tas".to_string(), Noun::Atom(a.clone()));
                let right = Hoon::Rock("tas".to_string(), Noun::Atom(b.clone()));
                Hoon::Pair(Box::new(left), Box::new(right))
            }
        };

        let clsg_items: Vec<Hoon> = manx.g.a
            .iter()
            .map(|(mane, beers)| {
                let n_hoon = match &mane {
                    Mane::Tag(s) => Hoon::Rock("tas".to_string(), Noun::Atom(s.clone())),
                    Mane::TagSpace(a, b) => {
                        let left = Hoon::Rock("tas".to_string(), Noun::Atom(a.clone()));
                        let right = Hoon::Rock("tas".to_string(), Noun::Atom(b.clone()));
                        Hoon::Pair(Box::new(left), Box::new(right))
                    }
                };
                let woofs: Vec<Woof> = beers
                    .iter()
                    .map(|b| match b {
                        Beer::Char(cord) => Woof::Atom(cord.clone()),
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
            Box::new(Spec::Base(BaseType::Atom("%$".to_string()))),
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
            Box::new(Hoon::Rock("f".to_string(), Noun::Atom("1".to_string()))),
            Box::new(Hoon::Rock("f".to_string(), Noun::Atom("0".to_string()))),
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
            Noun::Atom(term)
        }
        Chum::StdKel(term, u) => {
            Noun::Cell(
                Box::new(Noun::Atom(term)),
                Box::new(Noun::Atom(u.to_string())),
            )
        }
        Chum::VenProKel(t1, t2, u) => {
            Noun::Cell(
                Box::new(Noun::Atom(t1)),
                Box::new(Noun::Cell(
                    Box::new(Noun::Atom(t2)),
                    Box::new(Noun::Atom(u.to_string())),
                )),
            )
        }
        Chum::VenProVerKel(t1, t2, u1, u2) => {
            Noun::Cell(
                Box::new(Noun::Atom(t1)),
                Box::new(Noun::Cell(
                    Box::new(Noun::Atom(t2)),
                    Box::new(Noun::Cell(
                        Box::new(Noun::Atom(u1.to_string())),
                        Box::new(Noun::Atom(u2.to_string())),
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
                Noun::Atom(a) => Some(Skin::Leaf(t.to_string(), a.to_string())),
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

pub fn feck(gen: Hoon) -> Option<String> {
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

pub fn name_ax(gen: Hoon) ->  Option<Term> {
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

pub fn autoname(mod_spec: Spec) -> Option<Term> {  //  ++autoname:ax
    match mod_spec {
        Spec::Base(base) => match base {
            BaseType::Atom(aura) => {
                if aura == "%$" {    //  how empty terms will be represented here in rust land?...
                    Some("%atom".to_string())
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

// pub fn autoname(mod_spec: Spec) -> Option<Term> {  //  ++autoname:ax

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

pub fn gap<'src>(
) -> impl Parser<'src, &'src str, (), Err<'src>>
{
    regex(r"(?:(?:\s{2,}\n*|\n+|(?:\s*)?::[^\n\r]*(?:\r?\n)?))+")
    .ignored()
    .labelled("Gap")
}

pub fn list_term_hoon<'src>(
    hoon: impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<(Term, Hoon)>, Err<'src>>
{
    symbol()
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .repeated()
    .at_least(1)
    .collect::<Vec<(Term, Hoon)>>()
}

pub fn list_names_wide<'src>(
) -> impl Parser<'src, &'src str, Vec<Term>, Err<'src>>
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
            .to("%$".to_string())
            .or(symbol());

    let com =   //  ,
        just(",")
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
            just("+").ignore_then(regex(r"[0-9]+"))
                .map(|n: &str| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(num)}
                );

    let pam_number =   //  &10
            just("&").ignore_then((regex(r"[0-9]+")))
                .map(|n: &str| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(left_child(num))
                });

    let bar_number =  //  |10
           just("|").ignore_then(regex(r"[0-9]+"))
                .map(|n: &str| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(right_child(num))
                });

    let dot =  //  .
            just('.').to(Limb::Axis(1));

    let lus =  //  +
        just("+").to(Limb::Axis(3));

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

pub fn float<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    let floats =
            just('-').or_not()
            .then(decimal_without_dots())
            .then(choice((
                    just('.')
                        .ignore_then(leading_zero_decimal_without_dots())
                        .map(|frac| format!(".{}", frac)),
                    empty().to("".to_string()))))
            .then(choice((
                    just('e')
                        .ignore_then(just('-').or_not())
                        .then(decimal_without_dots())
                        .map(|(maybe_hep, expo)| format!("e{}", expo)),
                    empty().to("".to_string()))))
            .map(|(((_maybe_hep, p), mant), expo)| format!("{}{}{}", p, mant, expo));

    let royl_rn
        = choice((
            floats,  ///  1.10 or 1e10
                  just('-').or_not()   //  -inf or inf
                    .then(just("inf"))
                    .map(|(_maybe_hep, inf)| inf.to_string()),
                  just("nan").map(str::to_owned),  //  nan
                )).boxed();

    let rh = just("~~").ignore_then(royl_rn.clone());
    let rq = just("~~~").ignore_then(royl_rn.clone());
    let rd = just('~').ignore_then(royl_rn.clone());
    let rs = royl_rn;

    choice((
        rh.map(|s| ("%rh".to_string(), s)),
        rq.map(|s| ("%rh".to_string(), s)),
        rd.map(|s| ("%rd".to_string(), s)),
        rs.map(|s| ("%rs".to_string(), s)),
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
) -> impl Parser<'src, &'src str, HashMap<Term, Tome>, Err<'src>> {
    let luslus = just("++")
            .ignore_then(gap())
            .ignore_then(just('$').to("%$".to_string()).or(symbol()))
            .then_ignore(gap())
            .then(hoon.clone())
            .map(|(name, hoon)| (name, hoon));

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
      .ignore_then(symbol().map(|s| format!("%{}", s)))
}

pub fn jet_hooks<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<(Term, Hoon)>, Err<'src>>
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
                        .collect::<Vec<(Term, Hoon)>>()
                        )
            .then_ignore(gap())
            .then_ignore(just("=="))
        )
}

pub fn jet_signature<'src>(
) -> impl Parser<'src, &'src str, Chum, Err<'src>>
{
    let lef = just("%")  //  %k
                .ignore_then(symbol().map(Chum::Lef));

    let stdkel = just("%")  //  %k.138
                .ignore_then(symbol())
                .then_ignore(just('.'))
                .then(decimal_number())
                .map(|(s, n)| Chum::StdKel(s, n.parse().expect("failed to parse version number")));
    // TODO: add other cases here
    choice((
        stdkel,
        lef
    ))
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
    //
    //  "foo"
    //
    let wide_tape =
        choice((
            // non-control 32-256, excluding DEL, {,  ", \
            //
            regex(r#"[\x20-\x21\x23-\x5B\x5D-\x7A\x7C\x7E\x80-\xFF]"#)
                .map(|s: &str| Woof::Atom(s.to_string())),
            //
            //  escaped \, ", {, hex
            //
            just("\\")
                .ignore_then(
                    choice((just("\\"),
                            just("\""),
                            just("{"),
                            regex(r"[0-9a-f]{2}"))))
                .map(|s: &str| Woof::Atom(s.to_string())),
            //
            //  {hoon}
            //
            sump.clone(),
        )).repeated()
        .collect::<Vec<_>>()
        .delimited_by(just("\""), just("\""));

    //  """
    //  foo
    //  """
    let tall_tape =
            choice((
                //
                // non-control excluding DEL, {,  \
                //
                regex(r#"[\x20-\x7A\x7C\x7E\x80-\xFF]"#)
                .map(|s: &str| Woof::Atom(s.to_string())),
                //
                //  escaped \, {, hex
                //
                just("\\")
                    .ignore_then(
                        choice((just("\\"),
                                just("{"),
                                regex(r"[0-9a-f]{2}"))))
                    .map(|s: &str| Woof::Atom(s.to_string())),
            //
            // linebreak
            //
                newline()
                .ignore_then(just("\"\"\"").not())
                .to(Woof::Atom('\n'.to_string())),
            //
            //  {hoon}
            //
                sump,
            )).repeated()
            .at_least(1)
            .collect::<Vec<_>>()
            .delimited_by(just("\"\"\"").ignore_then(newline()),
                          newline().then_ignore(just("\"\"\"")));

    choice((wide_tape, tall_tape))
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
        choice((just('$').to("%$".to_string()),
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

pub fn constant<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    let buc =      // %$
        just('$')
        .to(("%tas".to_string(), "%$".to_string()));

    let cord =      // %'foo'
        cord()
        .map(|s| ("%t".to_string(), s));

    let coin =      // %123, %~m5, etc.
        nuck();

    let no =
        just('|')
        .to(("%f".to_string(), "1".to_string()));

    let yes =
        just('&')
        .to(("%f".to_string(), "0".to_string()));

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
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let gon = just("\\")
                .ignore_then(gap())
                .ignore_then(just("/"));

    let char_in_cord =
    regex(r"(?:(?:\\(?:\\|'|[0-9A-Fa-f]{2}))|[^\x00-\x1F\x7F'\\])+");

    let single_quoted =  char_in_cord
                        .separated_by(gon)
                        .at_least(1)
                        .collect::<Vec<_>>()
                        .delimited_by(just("\'"), just("\'"))
                        .map(|chars| chars.concat());
    choice((
        single_quoted,
        // triple_quoted,
    ))
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

///  Alphanumeric with hyphens
///      Start with a lowercase letter
///      Followed by zero or more: lowercase letter, digit, or hyphen
///
pub fn symbol<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>> {
    regex(r"[a-z](?:[a-zA-Z0-9-])*").map(str::to_owned)
    .labelled("Term")
}

pub fn bitcoin_address<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>> {
    regex(r"0c[13][a-km-zA-HJ-NP-Z1-9]{25,34}").map(str::to_owned)
    .labelled("Bitcoin_Address")
}

pub fn urx<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>> {
    regex(r"(?:[0-9a-z._-]|~[0-9a-fA-F]+\.)+").map(str::to_owned)
}
pub fn binary_number<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first = regex(r"0b(?:0|1[01]{0,3})");

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
                parts.join(".").to_string()
            }
        })
        .labelled("Binary")
}

pub fn hexadecimal_number<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first = regex(r"0x(0|[1-9a-fA-F][0-9a-fA-F]{0,3})");

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"[0-9a-fA-F]{4}"))
            .repeated()
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

pub fn ipv6_address<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let tod = regex(r"(0|[1-9a-f][0-9a-fA-F]{0,3})");

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

pub fn base32_number<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first = regex(r"0v(0|[1-9a-v][0-9av]{0,4})");

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"0v([0-9a-v]{0,5})"))
            .repeated()
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
        .labelled("Base32")
}

pub fn base64_number<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let first = regex(r"0w(0|[1-9a-zA-Z-~][0-9a-zA-Z-~]{0,4})");

    let rest = just('.').ignore_then(gap().or_not())
            .ignore_then(regex(r"0v([0-9a-zA-Z-~]{0,5})"))
            .repeated()
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

fn poof(pax: Vec<String>) -> Vec<Hoon> {
    pax.iter()
        .map(|a| { Hoon::Sand(
            "%ta".to_string(),
            Noun::Atom(a.to_string()),
        )})
        .collect()
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
    wer: &PathBuf,
) -> Option<Vec<Hoon>> {

    let wer_strings: Vec<String> = wer
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();

    let wom: Vec<Hoon> = poof(wer_strings);

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

pub fn nuck<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    choice((
        symbol().map(|s| ("tas".to_string(), s)),
        number(),
        just('.').ignore_then(perd()),
        just('~').ignore_then(
            choice((
                twid(),
                empty().to(("%n".to_string(), "0".to_string())),
            ))),
    )).boxed()
}

pub fn perd<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    zust()
}

pub fn zust<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    choice((
        ipv4_address().map(|s| ("if".to_string(), s)),
        ipv6_address().map(|s| ("is".to_string(), s)),
        float().map(|(p, q)| (p, q)),
        just("y").to(("%f".to_string(), "0".to_string())),
        just("n").to(("%f".to_string(), "1".to_string())),
        just('~')
            .ignore_then(phonemic_name_unscrambled())
            .map(|s| ("q".to_string(), s)),
    ))
}

pub fn path<'src>(
    hoon_wide: impl ParserExt<'src, Hoon>,
    wer: PathBuf,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    let wer = Arc::new(wer);

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
                just('$').to(Hoon::Sand("tas".to_string(), Noun::Atom("%$".to_string()))),
                cord().map(|s| Hoon::Sand("t".to_string(), Noun::Atom(s))),
                nuck().map(|(p, q)| Hoon::Sand(p, Noun::Atom(q))),
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
                        b.insert(0, Some(Hoon::Sand("tas".to_string(), Noun::Atom("%$".to_string()))));
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
        let wer2 = wer.clone();
        just("/")
        .ignore_then(poor.try_map(move |(pre, pof), span| {
            match posh(pre, pof, &wer2) {
                Some(list) => Ok(Hoon::ColSig(list)),
                None => Err(Rich::custom(span, "error parsing path")),
            }
        }))
    };

    let cen_fas = {
        let wer2 = wer.clone();
        porc.try_map(move |(a, b), span| {
            match posh(Some(vec![None]), Some((a, b)), &wer2) {
                Some(list) => Ok(Hoon::ColSig(list)),
                None => Err(Rich::custom(span, "error parsing path")),
            }
        })
    };

    let multi_cen = {
        let wer2 = wer.clone();
        just("%").repeated().count().try_map(move |n, span| {
            match posh(Some(vec![None]), Some((n, vec![])), &wer2) {
                Some(list) => Ok(Hoon::ColSig(list)),
                None => Err(Rich::custom(span, "error parsing path")),
            }
        })
    };

    let cen_path = just("%").ignore_then(choice((cen_fas, multi_cen)));

    choice((
        rood,       //  /foo/%/foo
        cen_path,   //  %/foo  and  %%
    )).labelled("Path")
}

pub fn number<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    let ud_number = decimal_number()
                    .map(|s|
                        ("ud".to_string(), s));

    let ux_number = hexadecimal_number()
                    .map(|s|
                        ("ux".to_string(), s));

    let uc_number = bitcoin_address()
                    .map(|s|
                        ("uc".to_string(), s));

    let ub_number = binary_number()
                    .map(|s|
                        ("ub".to_string(), s));

    let uv_number = base32_number()
                    .map(|s|
                        ("uv".to_string(), s));

    let uw_number = base64_number()
                    .map(|s|
                        ("uw".to_string(), s));

    let ui_number =
        regex(r"0i[0-9]+")
        .map(|s: &str| {
            ("ui".to_string(), s.to_string())
        });

    let signed_number = //  signed: -num and --num
        just('-').ignore_then(just('-').or_not())
        .ignore_then(
            choice((
                decimal_number().map(|s| ("sd".to_string(), s)),
                hexadecimal_number().map(|s| ("sx".to_string(), s)),
                binary_number().map(|s| ("sb".to_string(), s)),
                bitcoin_address().map(|s| ("sc".to_string(), s)),
                base32_number().map(|s| ("sv".to_string(), s)),
                base64_number().map(|s| ("sw".to_string(), s)),
                regex(r"0i[0-9]+").map(|s: &str| ("si".to_string(), s.to_string())),
            ))
        );

    choice((
        signed_number,
        ub_number,
        uc_number,
        ui_number,
        ux_number,
        uv_number,
        uw_number,
        ud_number,
    ))
}

pub fn rap_bits(bloq: usize, list: &[u64]) -> u64 {
    let mut acc: u128 = 0;
    let mut pos = 0usize;

    for &atom in list {
        let bit_width = 64 - atom.leading_zeros() as usize;
        let block_size = 1 << bloq;
        let step = (bit_width + block_size - 1) / block_size * block_size;

        acc |= (atom as u128) << pos;
        pos += step;
    }

    acc as u64
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
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let year = decimal_without_dots()
            .then(just('-')
                .to(false)
                .or_not()
                .map(|opt| opt.unwrap_or(true)))
            .map(|(a, b)| (b, a)); // (bool, year_str)
    let month =  just('.')
                .ignore_then(regex(r"(1[0-2]|[1-9])")
                .map(str::to_owned)); // month_str
    let day =  just('.')
                .ignore_then(regex(r"([1-9][0-9]*)")
                .map(str::to_owned)); // day_str
    let hour_min_secs_fractions =
             just("..")
                .ignore_then(leading_zero_decimal_without_dots()
                            .then_ignore(just("."))
                            .then(leading_zero_decimal_without_dots())
                            .then_ignore(just("."))
                            .then(leading_zero_decimal_without_dots()))
                .then(just("..")
                    .ignore_then(
                        regex(r"[0-9a-fA-F]{4}")
                        .separated_by(just("."))
                        .at_least(1)
                        .collect::<Vec<_>>()
                    )
                    .or_not()
                    .map(|opt| opt.unwrap_or(vec![]))
                )
                .or_not()
                .map(|opt| {
                    opt.unwrap_or((
                        (("0".to_string(), "0".to_string()), "0".to_string()),
                        Vec::new(),
                    ))
                });

    year
    .then(month)
    .then(day)
    .then(hour_min_secs_fractions)
    .to("foo".to_string())
}

pub fn relative_date<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    one_of("dhms")
        .ignore_then(decimal_without_dots())
        .separated_by(just('.'))
        .at_least(1)
        .then(just("..")
            .ignore_then(
                regex(r"[0-9a-fA-F]{4}")
                .separated_by(just('.'))
                .at_least(1)
                .collect::<Vec<_>>()
            )
            .or_not()
            .map(|opt| opt.unwrap_or(vec![]))
        )
    .to("foo".to_string())
}

/// Prefix syllables (Hoon `sis`), 256 entries × 3 bytes.
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

/// You must fill this with the actual 256 suffix syllables.
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
    let a = [a[0], a[1], a[2]];
    for i in 0u8..=255 {
        if SIS[i as usize] == a {
            return Some(i);
        }
    }
    None
}

/// Linear suffix search (Hoon ++ind)
pub fn ind(a: &[u8]) -> Option<u8> {
    if a.len() != 3 {
        return None;
    }
    let a = [a[0], a[1], a[2]];
    for i in 0u8..=255 {
        if DEX[i as usize] == a {
            return Some(i);
        }
    }
    None
}

pub fn phonemic_name<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let tip = regex(r"[a-z]{3}")
        .try_map(|s: &str, span| {
            match ins(s.as_bytes()) {
                Some(i) => Ok(i),
                None => Err(Rich::custom(span, format!("invalid prefix syllable '{s}'"))),
            }
        });
   let tiq = regex(r"[a-z]{3}")
        .try_map(|s: &str, span| {
            match ind(s.as_bytes()) {
                Some(i) => Ok(i),
                None => Err(Rich::custom(span, format!("invalid suffix syllable '{s}'"))),
            }
        });

    let tep = regex(r"[a-z]{3}")
        .try_map(|s: &str, span| {
            if s == "doz" {
                return Err(Rich::custom(span, "suffix 'doz' is forbidden"));
            }
            match ind(s.as_bytes()) {
                Some(i) => Ok(i),
                None => Err(Rich::custom(span, format!("invalid suffix syllable '{s}'"))),
            }
        });

    let hef = tip.then(tiq.clone()); // check if atom is not zero for hef
    let hif = hef.clone();
    let huf =  hef.clone()
                .then(just('-')
                    .ignore_then(hif.clone())
                    .repeated()
                    .at_most(3));
    let hyf =  hif.clone()
                .separated_by(just('-'))
                .exactly(4);
    let other = huf
                .then(just("--").ignore_then(gap().or_not())
                        .ignore_then(hyf));
    let planet_moon = hef
                    .then(
                        just('-')
                        .ignore_then(hif.clone())
                        .repeated()
                        .at_least(1)
                        .at_most(4)
                    );
    let star = tep.then(tiq.clone());

    let galaxy = tiq.clone();

    choice((other.ignored(),
            planet_moon.ignored(),
            star.ignored(),
            galaxy.ignored(),
        )).to("foo".to_string())
}

pub fn phonemic_name_unscrambled<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let tip = regex(r"[a-z]{3}")   //  duplicated logic!
        .try_map(|s: &str, span| {
            match ins(s.as_bytes()) {
                Some(i) => Ok(i),
                None => Err(Rich::custom(span, format!("invalid prefix syllable '{s}'"))),
            }
        });
   let tiq = regex(r"[a-z]{3}")
        .try_map(|s: &str, span| {
            match ind(s.as_bytes()) {
                Some(i) => Ok(i),
                None => Err(Rich::custom(span, format!("invalid suffix syllable '{s}'"))),
            }
        });

    let tep = regex(r"[a-z]{3}")
        .try_map(|s: &str, span| {
            if s == "doz" {
                return Err(Rich::custom(span, "suffix 'doz' is forbidden"));
            }
            match ind(s.as_bytes()) {
                Some(i) => Ok(i),
                None => Err(Rich::custom(span, format!("invalid suffix syllable '{s}'"))),
            }
        });

    let hef = tip.then(tiq.clone()); // check if atom is not zero for hef
    let hif = hef.clone();
    let huf =  hef.clone()
                .then(just('-')
                    .ignore_then(hif.clone())
                    .repeated()
                    .at_most(3));
    let hyf =  hif.clone()
                .separated_by(just('-'))
                .exactly(4);
    let other = huf
                .then(just("--").ignore_then(gap().or_not())
                        .ignore_then(hyf));
    let planet_moon = hef
                    .then(
                        just('-')
                        .ignore_then(hif.clone())
                        .repeated()
                        .at_least(1)
                        .at_most(4)
                    );
    let star = tep.then(tiq.clone());

    hif.clone()
    .then(tiq)
    .then(just('-')
            .ignore_then(hif)
            .repeated()
            .at_least(1)
    ).to("foo".to_string())
    // ).to(Hoon::Sand("q".to_string(), Noun::Atom("foo".to_string())))
}

pub fn twid<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    choice((
        just('0')
            .ignore_then(regex(r"[0-9a-v]+").map(str::to_owned))
            .map(|s| ("%$".to_string(), s)),
        crub(),
    ))
}

pub fn crub<'src>(
) -> impl Parser<'src, &'src str, (String, String), Err<'src>>
{
    choice((
            absolute_date().to(("%da".to_string(), "foo".to_string())),
            relative_date().to(("%dr".to_string(), "foo".to_string())),
            phonemic_name().to(("%p".to_string(), "foo".to_string())),
            just('.')
                .ignore_then(regex(r"[0-9a-z._~\-]+").map(str::to_owned))
                .map(|s| ("%ta".to_string(), s)),
            just('~')
                .ignore_then(urx())
                .map(|s| ("%t".to_string(), s)),
            just('-')
                .ignore_then(urx())
                .map(|s| ("%c".to_string(), s)),
    ))
}

//  +rump: name/hoon or name+hoon
//
pub fn constant_separator_hoon<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    choice((
        just('$').to(Hoon::Rock("%tas".to_string(), Noun::Atom("0".to_string()))),
        symbol().map(|s| Hoon::Rock("%tas".to_string(), Noun::Atom(s))),
        number().map(|(p, q)| Hoon::Rock(p, Noun::Atom(q))),
        just('&').to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))),
        just('|').to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))),
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
            Box::new(Hoon::Sand(a, Noun::Atom("0".to_string()))),
            Box::new(Hoon::KetLus(Box::new(Hoon::Sand("%$".to_string(), Noun::Atom("0".to_string()))), Box::new(b))),
        )})
}

pub fn tic_cell_construction<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
        .map(|h| Hoon::Pair(Box::new(Hoon::Rock("%n".to_string(),
                                                    Noun::Atom("0".to_string()))),
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
