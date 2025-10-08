use crate::hoon::{Hoon, Term, Spec, Limb, BaseType};

pub fn name_ax(gen: Hoon) ->  Option<Term> {  //  ++name:ax
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
        return Err("a and b must be non-zero");
    }

    let k = b.ilog2(); // floor(log2(b))
    let offset = b & ((1u64 << k) - 1); // lower k bits of b
    Ok((a << k) + offset)
}