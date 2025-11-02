use std::collections::*;
use crate::ast::hoon::*;
use crate::lexer::tokens::Token;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};

pub type Err<'tokens, 'src> = extra::Err<Rich<'tokens, Token<'src>>,>;

use chumsky::input::Input;          // <-- bring the trait into scope

pub trait ParserExt<'tokens, 'src: 'tokens, I, O>:
    Parser<'tokens, I, O, Err<'tokens, 'src>> + Clone + 'tokens
where
    I: Input<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
}
impl<'tokens, 'src: 'tokens, I, O, P> ParserExt<'tokens, 'src, I, O> for P
where
    P: Parser<'tokens, I, O, Err<'tokens, 'src>> + Clone + 'tokens,
    I: Input<'tokens, Token = Token<'src>, Span = SimpleSpan>,
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

///  Parses one or more Gaps
///
///   One or more because when the lexer gets rids of comments
///   it will generate multiple Gap Tokens for what is
///   gramaticaly one.
///
pub fn gap<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, (), Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Gap)
    .repeated()
    .at_least(1)
    .ignored()
}

pub fn list_term_hoon<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Vec<(Term, Hoon)>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {Token::Name(n) => n.to_string()}
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .repeated()
    .at_least(1)
    .collect::<Vec<(Term, Hoon)>>()
}

pub fn list_names_wide<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Vec<Term>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
   select! { Token::Name(s) => s.to_string() }
    .separated_by(just(Token::Ace))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Sel), just(Token::Ser))
}

pub fn winglist<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, WingType, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let name =      //  Name or $
        just(Token::Buc)
            .map(|_| "%$".to_string())
            .or(select! { Token::Name(name) => name.to_string() });

    let com =   //  ,
        just(Token::Com)
        .map(|_| Limb::Axis(0));

    let ket_name =   //  ^^name or name
        just(Token::Ket)
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
            just(Token::Lus)
                .ignore_then(select! {Token::Number(n) => n.to_string()})
                .map(|n| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(num)}
                );

    let pam_number =   //  &10
            just(Token::Pam)
                .ignore_then(select! {Token::Number(n) => n.to_string()})
                .map(|n| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(left_child(num))
                });

    let bar_number =  //  |10
            just(Token::Bar)
                .ignore_then(select! {Token::Number(n) => n.to_string()})
                .map(|n| {
                    let num = n.parse::<u64>().unwrap();
                    Limb::Axis(right_child(num))
                });

    let dot =  //  .
            just(Token::Dot)
                .map(|_| Limb::Axis(1));

    let lus =  //  +
        just(Token::Lus)
            .map(|_| Limb::Axis(3));

    let hep =  //  -
        just(Token::Hep)
            .map(|_| Limb::Axis(3));

    let lark =   //    +>-<  notation
            select! { Token::LarkExpression(str) => {
                let mut axis = 1;
                for c in str.chars() {
                    match c {
                        '+' | '>' => axis = peg(axis, 3).unwrap(),
                        '-' | '<' => axis = peg(axis, 2).unwrap(),
                        _ => axis = 1,
                    }
                }
                Limb::Axis(axis)
            }}.labelled("Lark Expression");

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
    )).separated_by(just(Token::Dot))
        .at_least(1)
        .collect::<Vec<_>>()
        .labelled("Wing")
}



pub fn variable_name_and_type<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Skin, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let not_named = just(Token::Tis)  // =/  =foo
        .ignore_then(spec_wide.clone())
        .try_map(|spec, span| {
            let auto = autoname(spec.clone());
             match auto {
                        // None => Err(Cheap::new(span).into()),
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

     let named = select! { Token::Name(s) => s.to_string() }    //  =/  a=foo  ,  =/  a
        .then_ignore(just(Token::Fas).or(just(Token::Tis)))
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

pub fn list_wing_hoon_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Vec<(WingType, Hoon)>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let pair = winglist()
                .then_ignore(just(Token::Ace))
                .then(hoon.clone());

    pair
        .separated_by(just(Token::Com).then(just(Token::Ace)))
        .at_least(1)
        .collect::<Vec<_>>()
}

pub fn list_hoon_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Vec<Hoon>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .separated_by(just(Token::Ace))
    .at_least(1)
    .collect::<Vec<Hoon>>()
}

pub fn list_spec_closed_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Vec<Spec>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .separated_by(just(Token::Ace))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn list_spec_closed_tall<'tokens, 'src: 'tokens, I>(
    spec:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Vec<Spec>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone()
                .separated_by(gap())
                .at_least(1)
                .collect::<Vec<_>>()
        )
    .then_ignore(gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
}

pub fn list_wing_hoon_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Vec<(WingType, Hoon)>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
   let pair = winglist()
                .then_ignore(gap())
                .then(hoon.clone())
                .then_ignore(gap());

    pair.repeated().at_least(1).collect::<Vec<(WingType, Hoon)>>()
}

pub fn tiki_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Tiki, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let with_name = select! { Token::Name(term) => term.to_string() }
        .then_ignore(just(Token::Tis))
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

pub fn tiki_tall<'tokens, 'src: 'tokens, I>(
    hoon_tall: impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Tiki, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let with_name = select! { Token::Name(term) => term.to_string() }
        .then_ignore(just(Token::Tis))
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
            just([Token::Ket, Token::Tis]).then(gap()).or_not()
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
pub fn chapters<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec> + Clone + 'tokens
) -> impl Parser<'tokens, I, HashMap<Term, Tome>, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{

    let luslus = just([Token::Lus, Token::Lus])
            .ignore_then(gap())
            .ignore_then(just(Token::Buc).to("%$").or(select! { Token::Name(s) => s }))
            .then_ignore(gap())
            .then(hoon.clone())
            .map(|(name, hoon)| (name.to_string(), hoon));

    let lusbuc =  just([Token::Lus, Token::Buc])
            .ignore_then(gap())
            .ignore_then(select! { Token::Name(s) => s })
            .then_ignore(gap())
            .then(spec.clone())
            .map(|(name, spec)| (name.to_string(),
                                Hoon::KetCol(Box::new(Spec::Name(name.to_string(),
                                                        Box::new(spec))))));

    let optional_chapter_label =
        just([Token::Lus, Token::Bar])
        .then_ignore(gap())
        .then(just(Token::Cen))
        .ignore_then(select! { Token::Name(s) => s.to_string() })
        .then_ignore(gap())
        .or_not();

    let chapter = optional_chapter_label
                    .then(luslus.or(lusbuc)
                          .then_ignore(gap())
                          .repeated().at_least(1).collect::<Vec<_>>());

    chapter.repeated().at_least(1).collect::<Vec<_>>()
        .then_ignore(just([Token::Hep, Token::Hep]))
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

pub fn list_hoon_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Vec<Hoon>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .separated_by(gap())
    .at_least(1)
    .collect::<Vec<_>>()
}

pub fn term<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, String, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cen)
      .ignore_then(select! {Token::Name(s) => format!("%{}", s) })
}

pub fn jet_hooks<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Vec<(Term, Hoon)>, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Sig).to(Vec::new())
        .or(
            just([Token::Tis, Token::Tis])
            .ignore_then(just(Token::Gap))
            .ignore_then(just(Token::Cen)
                        .ignore_then(select! {Token::Name(n) => format!("%{}", n)})
                        .then_ignore(gap())
                        .then(hoon.clone())
                        .separated_by(gap())
                        .at_least(1)
                        .collect::<Vec<(Term, Hoon)>>()
                        )
            .then_ignore(gap())
            .then_ignore(just([Token::Tis, Token::Tis]))
        )
}

pub fn jet_signature<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Chum, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let lef = just(Token::Cen)  //  %k
                .ignore_then(select!
                    { Token::Name(s) => Chum::Lef(s.to_string())}
                );

    let stdkel = just(Token::Cen)  //  %k.138
                .ignore_then(select!
                    { Token::Name(s) => s.to_string() }
                )
                .then_ignore(just(Token::Dot))
                .then(select! {
                    Token::Number(n) => {
                        n.chars()
                            .filter(|c| c.is_digit(10))
                            .collect::<String>()
                            .parse::<u64>()
                            .ok()
                    }
                })
                .map(|(s, n)| Chum::StdKel(s, n.unwrap_or(0)));

    stdkel
    .or(lef)
}

pub fn tape<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Tape(s) => Hoon::Knit(s.to_string()) }
}

pub fn aura_hoon<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Pat)
    .ignore_then(
        select! { Token::Name(s) => s.to_string() }.or_not()
    )
    .map(|maybe_name| {
        let name = maybe_name.unwrap_or("~.".to_string());
        Hoon::Base(BaseType::Atom(name))
    })
}

pub fn aura_spec<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Pat)
    .ignore_then(
        select! { Token::Name(s) => s.to_string() }.or_not()
    )
    .map(|maybe_name| {
        let name = maybe_name.unwrap_or("~.".to_string());
        Spec::Base(BaseType::Atom(name))
    })
}

pub fn path<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Fas)
        .to(Hoon::ColSig(vec![]))
}

pub fn concatanate<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
      .then_ignore(just(Token::Ket))
      .then(hoon_wide.clone())
      .map(|(p, q)| Hoon::Pair(Box::new(p), Box::new(q)))
}

pub fn wing<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
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

pub fn tell<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Gal)
        .ignore_then(list_hoon_wide(hoon_wide.clone()))
        .then_ignore(just(Token::Gar))
        .map(|list| Hoon::Tell(list))
}

pub fn spec_term<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let buc =      // %$
        just(Token::Cen)
        .ignore_then(just(Token::Buc))
        .map(|_| Spec::Leaf("%tas".to_string(), "%$".to_string()));

    let number =      // %123
        just(Token::Cen)
        .ignore_then(select! { Token::Number(n) => n })
        .map(|n| Spec::Leaf("%ud".to_string(), n.to_string()));

    let name =      // %foo
        just(Token::Cen)
        .ignore_then(select! { Token::Name(s) => s })
        .map(|s| Spec::Leaf("%tas".to_string(), s.to_string()));

    let cord =      // %'foo'
        just(Token::Cen)
        .ignore_then(select! { Token::Cord(s) => s })
        .map(|s| Spec::Leaf("%t".to_string(), s.to_string()));

    let yes =      // %.y
        just(Token::Yes).to(Spec::Leaf("%f".to_string(), "0".to_string()));

    let no =      // %.n
        just(Token::No).to(Spec::Leaf("%f".to_string(), "1".to_string()));

    choice((
        buc,
        number,
        name,
        cord,
        yes,
        no,
    ))
}

pub fn constant<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let buc_const =      // %$
        just(Token::Cen)
        .ignore_then(just(Token::Buc))
        .map(|_|
            Hoon::Rock("%tas".to_string(), Noun::Atom("%$".to_string()))
        );

    let number_const =      // %123
        just(Token::Cen)
        .ignore_then(select! { Token::Number(n) => n })
        .map(|n| Hoon::Rock("%ud".to_string(), Noun::Atom(n.to_string())));


    let name_const =      // %foo
        just(Token::Cen)
        .ignore_then(select! { Token::Name(s) => s })
        .map(|s| Hoon::Rock("%tas".to_string(), Noun::Atom(s.to_string())));

    let cord_const =      // %'foo'
        just(Token::Cen)
        .ignore_then(select! { Token::Cord(n) => n })
        .map(|n| Hoon::Rock("%t".to_string(), Noun::Atom(n.to_string())));

    choice((
        buc_const,
        number_const,
        name_const,
        cord_const,
    ))
}

pub fn cord<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {Token::Cord(s) => Hoon::Sand("%t".to_string(), Noun::Atom(s.to_string()))}
}

pub fn increment<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Dot).or_not()
        .ignore_then(just(Token::Lus))
        .ignore_then(just(Token::Pal))
        .ignore_then(
            hoon_wide.clone()
        )
        // .then_ignore(just(Token::Ace).not())
        .then_ignore(just(Token::Par))
        .map(|h| Hoon::DotLus(Box::new(h)))
}

pub fn function_call<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Pal)
        .ignore_then(hoon.clone())
        .then(
            just(Token::Ace)
                .ignore_then(hoon.clone())
                .repeated()
                .collect::<Vec<_>>()
            )
    .then_ignore(just(Token::Par))
    .map(|(func, args)| Hoon::CenCol(Box::new(func), args))
}

pub fn number<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let decimal = select! {
        Token::Number(num_str) => {
            Hoon::Sand("ud".to_string(), Noun::Atom(num_str.to_string()))
        }
    };

    let signed = select! {
        Token::SignedNumber(num_str) => {
            Hoon::Sand("sd".to_string(), Noun::Atom(num_str.to_string()))
        }
    };

    let hexadecimal = select! {
        Token::HexNumber(num_str) => {
            Hoon::Sand("ux".to_string(), Noun::Atom(num_str.to_string()))
        }
    };

    let binary = select! {
        Token::BinaryNumber(num_str) => {
            Hoon::Sand("ub".to_string(), Noun::Atom(num_str.to_string()))
        }
    };

    let unicode = select! {
        Token::Unicode(num_str) => {
            Hoon::Sand("c".to_string(), Noun::Atom(num_str.to_string()))
        }
    };

    decimal
    .or(signed)
    .or(hexadecimal)
    .or(binary)
    .or(unicode)
    .labelled("Number")
}

//  +rump: name/hoon or name+hoon
//
pub fn constant_separator_hoon<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Buc).to(Hoon::Rock("%tas".to_string(), Noun::Atom("%$".to_string())))
        .or(select! { Token::Name(s) => Hoon::Rock("%tas".to_string(), Noun::Atom(s.to_string())) })
        .or(select! { Token::Number(n) => Hoon::Rock("%ud".to_string(), Noun::Atom(n.to_string())) })
        .or(just(Token::Pam).to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))))
        .or(just(Token::Bar).to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))))
        .then(just(Token::Lus).or(just(Token::Fas))
              .ignore_then(hoon.clone()))
        .map(|(rock, hoon)| Hoon::Pair(Box::new(rock), Box::new(hoon)))
}

pub fn list_syntax<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Sig, Token::Sel]).to(true).or(just(Token::Sel).to(false))   //  ~[  or  [
        .then(hoon_wide.clone()
                .separated_by(just(Token::Ace))
                .at_least(1)
                .collect::<Vec<_>>()
            )
        .then(just([Token::Ser, Token::Sig]).to(true).or(just(Token::Ser).to(false)))  //  ]~ or ]
        .map(|((start, list), end)| {
                if start {
                    if end {
                       return Hoon::ColSig(vec![Hoon::ColSig(list)]);
                    } {
                        return Hoon::ColSig(list);
                    }
                } else {
                   if end {
                       return Hoon::ColSig(vec![Hoon::ColTar(list)]);
                    } {
                        return Hoon::ColTar(list);
                    }
                }
            })
}

pub fn tic_cell_construction<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Tic)
        .ignore_then(hoon_wide.clone())
        .map(|h| Hoon::Pair(Box::new(Hoon::Rock("%n".to_string(),
                                                     Noun::Atom("0".to_string()))),
                                 Box::new(h)))
}

pub fn parenthesis_spec<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
        .then(
            just(Token::Ace)
            .ignore_then(spec_wide.clone())
                .repeated()
                .collect::<Vec<_>>()
                .or_not()
                .map(|specs| specs.unwrap_or_default())
        )
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(name, specs)| Spec::Make(name, specs))
}

pub fn reference_spec<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {
        Token::Buc => (),
        Token::Com => (),
        Token::Ket => (),
        Token::Name(_) => (),
    }
    .rewind()
    .ignore_then(
        winglist()
            .separated_by(just(Token::Col))
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|wings: Vec<WingType>| {
                        let (first, rest) = wings.split_first().unwrap();
                        Spec::Like(first.to_vec(), rest.to_vec())
                    })
        )
}

pub fn two_hoons_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, (Hoon, Hoon), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
}

pub fn two_hoons_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, (Hoon, Hoon), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
}

pub fn three_hoons_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, ((Hoon, Hoon), Hoon), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
}

pub fn three_hoons_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, ((Hoon, Hoon), Hoon), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
}

pub fn two_specs_tall<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (Spec, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(spec.clone())
}

pub fn two_specs_closed_tall<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (Spec, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_tall(spec.clone())
    .then_ignore(gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
}

pub fn two_specs_closed_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (Spec, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(spec_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn hoon_spec_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (Hoon, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(spec_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn hoon_spec_tall<'tokens, 'src: 'tokens, I>(
    hoon:           impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:           impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (Hoon, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(spec.clone())
}

pub fn spec_hoon_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (Spec, Hoon), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
}

pub fn spec_hoon_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (Spec, Hoon), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
}

pub fn name_spec_tall<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (String, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(select! { Token::Name(n) => n.to_string() })
    .then_ignore(gap())
    .then(spec.clone())
}

pub fn name_spec_closed_tall<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (String, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(select! { Token::Name(n) => n.to_string() })
    .then_ignore(gap())
    .then(spec.clone())
    .then_ignore(just([Token::Tis, Token::Tis]))
}

pub fn name_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, (String, Spec), Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Name(n) => n.to_string() }
    .then_ignore(just(Token::Ace))
    .then(spec_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn one_hoon_closed_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn one_hoon_closed_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .delimited_by(just(Token::Tis), just(Token::Tis))
}

pub fn one_spec_closed_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn one_spec_closed_tall<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .delimited_by(just(Token::Tis), just(Token::Tis))
}