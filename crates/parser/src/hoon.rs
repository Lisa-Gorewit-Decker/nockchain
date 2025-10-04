
use std::fmt;
use std::collections::*;

#[derive(Debug, Clone)]
pub enum Noun {
    Atom(String),
    Cell(Box<Noun>, Box<Noun>),
}

impl Noun {
    pub fn atom(n: String) -> Self {
        Noun::Atom(n)
    }
    pub fn cell(a: Noun, b: Noun) -> Self {
        Noun::Cell(Box::new(a), Box::new(b))
    }
}

pub type What = String;
pub type Term = String;
pub type Tome = (What, HashMap<Term, Hoon>);

#[derive(Debug, Clone)]
pub enum Limb {
    Term(String),
    Axis(u64),
    Parent(u64, Option<String>),  // ^foo  ->   (1, %foo)   ->  matches second foo in the subject
}

#[derive(Debug)]
pub enum Hoon {
    ZapZap,
    Base(BaseTyp),
    Sand(Term, Noun),
    Wing(Vec<Limb>),

    CenCol(Box<Hoon>, Vec<Hoon>),
    DotTis(Box<Hoon>, Box<Hoon>),

    BarCen(Option<()>, HashMap<Term, Box<Tome>>),
    BarTis(Box<Hoon>, Box<Hoon>),
    BarHep(Box<Hoon>),

    BucTis(Term, Box<Hoon>),

    KetSig(Box<Hoon>),
    KetHep(Box<Hoon>, Box<Hoon>),

    TisDot(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisMic(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisFas(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisWut(Box<Hoon>, Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisCol(Vec<((), Box<Hoon>)>, Box<Hoon>),

    WutCol(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    WutDot(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    WutGar(Box<Hoon>, Box<Hoon>),
    WutZap(Box<Hoon>),

}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseTyp {
    Noun,
    Cell,
    Flag,
    Null,
    Void,
    Atom { aura: String },
}

// impl fmt::Display for BaseTyp {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         match self {
//             BaseTyp::Noun => write!(f, "%noun"),
//             BaseTyp::Cell => write!(f, "%cell"),
//             BaseTyp::Flag => write!(f, "%flag"),
//             BaseTyp::Null => write!(f, "%null"),
//             BaseTyp::Void => write!(f, "%void"),
//             BaseTyp::Atom { aura: n } => write!(f, "@{}", n),
//         }
//     }
// }

// impl fmt::Display for Limb {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         match self {
//             Limb::Term(s) => write!(f, "{}", s),
//             Limb::Axis(n) => write!(f, "{}", n),
//             Limb::Parent(n, Some(s)) => write!(f, "({}, {})", n, s),
//             Limb::Parent(n, None) => write!(f, "({}, None)", n),
//         }
//     }
// }
// impl fmt::Display for Noun {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         match self {
//             Noun::Atom(s) => write!(f, "{}", s),
//             Noun::Cell(a, b) => write!(f, "[{} {}]", a, b),
//         }
//     }
// }

// impl<'a> fmt::Display for Hoon {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         match self {
//             Hoon::ZapZap => write!(f, "!!"),
//             Hoon::Sand(_term, value) => {
//                 write!(f, "{}", value)
//             },
//             Hoon::KetSig(h) => {
//                 write!(f, "^~  {}", h)
//             }
//             Hoon::Wing(limbs) => {
//                 let parts: Vec<String> = limbs.iter().map(|l| l.to_string()).collect();
//                 write!(f, "Wing({})", parts.join(", "))
//             }
//             Hoon::CenCol(boxed, vec) => {
//                 let vec_str = vec
//                     .iter()
//                     .map(|h| h.to_string())
//                     .collect::<Vec<_>>()
//                     .join("  ");
//                 write!(f, "%:  {}  {})", boxed, vec_str)
//             }
//             Hoon::BarCen(opt, map) => {
//                 if opt.is_some() {
//                     write!(f, "|%\n")?;
//                 } else {
//                     write!(f, "|%\n")?;
//                 }
//                 let entries: Vec<String> = map
//                     .iter()
//                     .map(|(k, tome)| {
//                         let (_what, arms_map) = &**tome;
//                         let arms: Vec<String> = arms_map
//                             .iter()
//                             .map(|(arm_name, hoon)| format!("++  {}\n  {}\n", arm_name, hoon))
//                             .collect();
//                         format!("+| {}\n{}--", k, arms.join(""))
//                     })
//                     .collect();
//                 write!(f, "{}", entries.join(", "))
//             }
//             Hoon::Base(s) => {
//                 write!(f, "%{}\n", s)
//             }
//             Hoon::BarTis(args, body) => {
//                 write!(f, "|=  {}\n  {}", args, body)
//             }
//             Hoon::BucTis(term, spec) => {
//                 write!(f, "$=  {}\n  {}", term, spec)
//             }
//         }
//     }
// }
