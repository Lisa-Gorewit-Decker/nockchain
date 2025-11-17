use std::collections::*;
use crate::ast::hoon::*;
use std::sync::Arc;
use std::path::PathBuf;
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

//  build default sample
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
        Spec::Gist(help, spec) => spore_recursion(*spec, dom, hay, cox, bug, nut, def),
        Spec::Leaf(term, atom) => Hoon::Rock(term, Noun::Atom(atom)),
        Spec::Loop(term) => {
            let maybe_spec = cox.get(&term);
            match maybe_spec {
                Some(spec) => spore_recursion(spec.clone(), dom, hay, cox, bug, nut, def),
                None => Hoon::ZapZap,  //  we probably need to return None here...
            }
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
            // Hoon::KetTis(skin, Box::new(tail))
            Hoon::KetTis(Box::new(Hoon::ZapZap), Box::new(tail)) //  TODO use skin here

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

//  +analyse:basic
pub fn basic(bas: BaseType,
                axe: u64,
                spec: Spec,
                dom: u64,
                hay: WingType,
                cox: HashMap<Term, Spec>,
                mut bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    match bas {
        BaseType::Atom(a) => {
            let cnls = Hoon::CenLus(Box::new(Hoon::Limb("%ruth".to_string())),
                                    Box::new(Hoon::Sand("%ta".to_string(), Noun::Atom(a))),
                                    Box::new(Hoon::Axis(axe)));

            let example_res = Box::new(Hoon::ZapZap);
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
            let example_res = Box::new(Hoon::ZapZap);
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

//  +analyse:relative
// pub fn relative(axe: u64,
//                 spec: Spec,
//                 dom: u64,
//                 hay: WingType,
//                 cox: HashMap<Term, Spec>,
//                 mut bug: Vec<Spot>,
//                 nut: Option<Note>,
//                 def: Option<Hoon>) -> Hoon {
//     match spec {
//         Spec::Base => 
//         _ => Hoon::ZapZap
//     }
// }

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

//  make a normalizing gate (mold)
pub fn factory(spec: Spec,
                dom: u64,
                hay: WingType,
                cox: HashMap<Term, Spec>,
                mut bug: Vec<Spot>,
                nut: Option<Note>,
                def: Option<Hoon>) -> Hoon {
    match spec {
        Spec::Dbug(spot, spec) => {
            bug.insert(0, spot);
            factory(*spec, dom, hay, cox, bug, nut, def)
        }
        Spec::BucSig(hoon, spec) => {
            let spec_clone = spec.clone();
            let spec_clone2 = spec.clone();
            factory(*spec_clone, dom, hay, cox, bug, nut, Some(Hoon::KetHep(spec_clone2, Box::new(hoon))))
        }
        _ => {
            match (def.clone(), spec.clone()) {
                (Some(_d), Spec::BucMic(h)) => home(h, hay, dom),
                (Some(_d), Spec::Like(wing, vec_wing)) => home(unreel(wing, vec_wing), hay, dom),
                (Some(_d), Spec::Loop(term)) => home(Hoon::Limb(term), hay, dom),
                (Some(_d), Spec::Make(h, s)) => home(unfold(h, s), hay, dom),
                _ => {
                    let spore_res = spore(spec.clone(),
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
                    // let relative_res = relative(6, spec, descent_axis, hay, cox, bug, nut, def);
                    let relative_res = Hoon::ZapZap;
                    let tail = Hoon::TisLus(Box::new(relative_res),
                                            Box::new(tislus));

                    Hoon::BarCol(ketsig, Box::new(tail))
                }
            }
        }
    }
}

pub fn open(gen: Hoon) -> Hoon {  // desugarer
    match gen {
        Hoon::Axis(a) =>  Hoon::CenTis(vec![Limb::Axis(a)] , Vec::new()),
        Hoon::Base(b) =>  factory(Spec::Base(b), 1, Vec::new(), HashMap::new(), Vec::new(), None, None),
        Hoon::Dbug(_p, q) => *q,
        _  =>  gen
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

        Hoon::Note(n, hoon) => {
            match n {
                Note::Help(h) => {
                    let maybe_skin = flay(*hoon);
                    match maybe_skin {
                        Some(s) => Some(Skin::Help(h.to_string(), Box::new(s))),
                        None => None,
                    }
                }
                _ => None,
            }
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

        Hoon::KetTisSkin(skin, h) => {
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
            // let desugared = open(gen.clone());
            // if desugared == gen {
                None
            // } else {
                // flay(desugared)
            // }
        }

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
        Spec::Gist(_, q) => autoname(*q),
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
                Some(u) => Box::new(Hoon::KetTisSkin(Skin::Term(u), q)),
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

pub fn peg(a: u64, b: u64) -> Result<u64, &'static str> {  // this is broken...
    if a == 0 || b == 0 {
        return Err("a and b must be non-zero");
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
    .separated_by(just(" "))
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

pub fn list_wing_hoon_wide<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<(WingType, Hoon)>, Err<'src>>
{
    let pair = winglist()
                .then_ignore(just(" "))
                .then(hoon.clone());

    pair
        .separated_by(just(",").then(just(" ")))
        .at_least(1)
        .collect::<Vec<_>>()
}

pub fn list_hoon_wide<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Vec<Hoon>, Err<'src>>
{
    hoon_wide.clone()
    .separated_by(just(" "))
    .at_least(1)
    .collect::<Vec<Hoon>>()
}

pub fn list_spec_closed_wide<'src>(
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Vec<Spec>, Err<'src>>
{
    spec_wide.clone()
    .separated_by(just(" "))
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
                let what = "".to_string();
                let tome: Tome = (what, arms_map);
                map_term_tome.insert(key, tome);
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

pub fn tape<'src>(
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    regex(r#""[^"]*""#).map(|s: &str| { // fix this
        Hoon::Knit(s.to_string())
    })
}

pub fn aura_hoon<'src>(
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just("@")
    .ignore_then(regex(r"[a-zA-Z]+")
                .map(str::to_owned)
                .or_not())
    .map(|maybe_name| {
        let name = maybe_name.unwrap_or("~.".to_string());
        Hoon::Base(BaseType::Atom(name))
    })
}

pub fn aura_spec<'src>(
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    just("@")
    .ignore_then(regex(r"[a-zA-Z]+")
                .map(str::to_owned)
                .or_not())
    .map(|maybe_name| {
        let name = maybe_name.unwrap_or("~.".to_string());
        Spec::Base(BaseType::Atom(name))
    })
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

pub fn spec_term<'src>(
) -> impl Parser<'src, &'src str, Spec, Err<'src>>
{
    let buc =      // %$
        just("%")
        .ignore_then(just('$'))
        .map(|_| Spec::Leaf("%tas".to_string(), "%$".to_string()));

    let number =      // %123
        just("%")
        .ignore_then(decimal_number())
        .map(|n| Spec::Leaf("%ud".to_string(), n));

    let name =      // %foo
        just("%")
        .ignore_then(symbol())
        .map(|s| Spec::Leaf("%tas".to_string(), s));

    let cord =      // %'foo'
        just("%")
        .ignore_then(cord())
        .map(|s| Spec::Leaf("%t".to_string(), s));

    let yes =
        just("%.y").to(Spec::Leaf("%f".to_string(), "0".to_string()));

    let no =
        just("%.n").to(Spec::Leaf("%f".to_string(), "1".to_string()));

    choice((
        buc,
        number,
        name,
        cord,
        yes,
        no,
    ))
}

pub fn constant<'src>(
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    let buc_const =      // %$
        just("%$")
        .map(|_|
            Hoon::Rock("%tas".to_string(), Noun::Atom("%$".to_string()))
        );

    let number_const =      // %123
        just("%")
        .ignore_then(number_constant());

    let name_const =      // %foo
        just("%")
        .ignore_then(symbol())
        .map(|s| Hoon::Rock("%tas".to_string(), Noun::Atom(s.to_string())));

    let cord_const =      // %'foo'
        just("%")
        .ignore_then(cord())
        .map(|n| Hoon::Rock("%t".to_string(), Noun::Atom(n)));

    choice((
        buc_const,
        number_const,
        name_const,
        cord_const,
    ))
}

pub fn cord<'src>(
) -> impl Parser<'src, &'src str, String, Err<'src>>
{
    let gon = just("\\")
                .ignore_then(gap())
                .ignore_then(just("/"));

    let char_in_cord =
    regex(r"(?:(?:\\(?:\\|'|[0-9A-Fa-f]{2}))|[^\x00-\x1F\x7F'\\])+");

    let single_quoted =
                            char_in_cord
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
            just(" ")
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
        .labelled("binary")
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
        .labelled("hexadecimal")
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

pub fn number_constant<'src>(
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    let ud_number = decimal_number().map(|s|
                        Hoon::Rock("ud".to_string(), Noun::Atom(s))
                    );

    let ux_number = hexadecimal_number().map(|s|
                        Hoon::Rock("ux".to_string(), Noun::Atom(s))
                 );

    let ub_number = binary_number().map(|s|
                        Hoon::Rock("ub".to_string(), Noun::Atom(s))
                    );

    let sd_number = //  signed: -num and --num
        just('-').ignore_then(just('-').or_not())
        .ignore_then(
            choice((
                decimal_number().map(|s| Hoon::Rock("sd".to_string(), Noun::Atom(s))),
                hexadecimal_number().map(|s| Hoon::Rock("sx".to_string(), Noun::Atom(s))),
                binary_number().map(|s| Hoon::Rock("sb".to_string(), Noun::Atom(s))),
            ))
        );

    let unicode =
        regex(r"~-~?[0-9a-fA-F]+\.?|~-[a-zA-Z]|~\[(?:~-[a-zA-Z0-9]+(?:\s+)?)+\]")
        .map(|s: &str| {
            Hoon::Rock("c".to_string(), Noun::Atom(s.to_string()))
        });

    let ui_number =
        regex(r"0i[0-9]+").map(|s: &str| {
            Hoon::Rock("ui".to_string(), Noun::Atom(s.to_string()))
        });

    choice((
        sd_number,
        ux_number,
        ui_number,
        ub_number,
        unicode,
        ud_number,
    ))
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

pub fn path<'src>(
    hoon_wide: impl ParserExt<'src, Hoon>,
    wer: PathBuf,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    let wer = Arc::new(wer);

    let hasp = choice((
                hoon_wide.clone().delimited_by(just("["), just("]")),
                hoon_wide.clone()
                    .separated_by(just(" "))
                    .at_least(1)
                    .collect::<Vec<_>>()
                    .delimited_by(just("("), just(")"))
                    .map(|list| {
                        let (first, rest) = list.split_first().unwrap();
                        Hoon::CenCol(Box::new(first.clone()), rest.to_vec())
                    }),
                just('$').to(Hoon::Sand("tas".to_string(), Noun::Atom("%$".to_string()))),
                cord().map(|s| Hoon::Sand("t".to_string(), Noun::Atom(s))),
                symbol().map(|s| Hoon::Sand("ta".to_string(), Noun::Atom(s))),  // add the other cases here...
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
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    let ud_number = decimal_number().map(|s|
                        Hoon::Sand("ud".to_string(), Noun::Atom(s))
                    );

    let ux_number = hexadecimal_number().map(|s|
                        Hoon::Sand("ux".to_string(), Noun::Atom(s))
                 );

    let ub_number = binary_number().map(|s|
                        Hoon::Sand("ub".to_string(), Noun::Atom(s))
                    );

    let sd_number = //  signed: -num and --num
        just('-').ignore_then(just('-').or_not())
        .ignore_then(
            choice((
                decimal_number().map(|s| Hoon::Sand("sd".to_string(), Noun::Atom(s))),
                hexadecimal_number().map(|s| Hoon::Sand("sx".to_string(), Noun::Atom(s))),
                binary_number().map(|s| Hoon::Sand("sb".to_string(), Noun::Atom(s))),
            ))
        );

    let unicode =
        regex(r"~-~?[0-9a-fA-F]+\.?|~-[a-zA-Z]|~\[(?:~-[a-zA-Z0-9]+(?:\s+)?)+\]")
        .map(|s: &str| {
            Hoon::Sand("c".to_string(), Noun::Atom(s.to_string()))
        });

    let ui_number =
        regex(r"0i[0-9]+").map(|s: &str| {
            Hoon::Sand("ui".to_string(), Noun::Atom(s.to_string()))
        });

    choice((
        sd_number,
        ux_number,
        ui_number,
        ub_number,
        unicode,
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

//  +rump: name/hoon or name+hoon
//
pub fn constant_separator_hoon<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just('$').to(Hoon::Rock("%tas".to_string(), Noun::Atom("0".to_string())))
        .or(symbol().map(|s| Hoon::Rock("%tas".to_string(), Noun::Atom(s))))
        .or(decimal_number().map(|n| Hoon::Rock("%ud".to_string(), Noun::Atom(n))))
        .or(just("&").to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))))
        .or(just("|").to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))))
    .then(just("+").or(just("/"))
            .ignore_then(hoon.clone()))
    .map(|(rock, hoon)| Hoon::Pair(Box::new(rock), Box::new(hoon)))
}

pub fn tic_cell_construction<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just("`")
        .ignore_then(hoon_wide.clone())
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
            just(" ")
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
    .then_ignore(just(" "))
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
    .then_ignore(just(" "))
    .then(hoon_wide.clone())
    .then_ignore(just(" "))
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
    .then_ignore(just(" "))
    .then(spec_wide.clone())
    .delimited_by(just('('), just(')'))
}

pub fn hoon_spec_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
    spec_wide:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, (Hoon, Spec), Err<'src>>
{
    hoon_wide.clone()
    .then_ignore(just(" "))
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
    .then_ignore(just(" "))
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
    .then_ignore(just(" "))
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
