use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::fs;
use std::collections::HashMap;
use std::time::Instant;
use logos::Logos;

pub mod tokens;
pub mod utils;
pub mod hoon;

use self::tokens::Token;
use self::hoon::*;
use self::utils::*;

fn gap_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, (), extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Gap)
        .repeated()
        .ignored()
}


fn tape_parser<'tokens, 'src: 'tokens, I>(
    // hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Tape(s) => Hoon::Knit(s.to_string()) }
}

fn aura_hoon_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Aura(str) => str }
        .map(|s| Hoon::Base(BaseType::Atom(s.to_string())))
}

fn aura_spec_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Aura(str) => str }
    .map(|s| Spec::Base(BaseType::Atom(s.to_string())))
}

fn wutzap_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Zap)
    .ignore_then(hoon_wide.clone())
    .map(|h| Hoon::WutZap(Box::new(h)))
}

fn kettis_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {
        Token::Name(str) => {
            let wing = vec![Limb::Term(str.to_string())];
            Hoon::Wing(wing)   //  TODO convert $hoon into $skin
        }
    }
    .then_ignore(just(Token::Tis))
    .then(hoon_wide.clone())
    .map(|(p, q)| Hoon::KetTis(Box::new(p), Box::new(q)))
}

fn dottis_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::DotTisIrregular)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::DotTis(Box::new(p), Box::new(q)))
}

fn wutbar_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutBarIrregular)
       .ignore_then(hoon_wide.clone()
            .separated_by(just(Token::Ace))
            .at_least(1)
            .collect::<Vec<_>>()
        )
        .then_ignore(just(Token::Par))
        .map(|hoons| Hoon::WutBar(hoons))
}

fn wutpam_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutPam)
       .ignore_then(gap_parser())
       .ignore_then(hoon.clone()
            .separated_by(gap_parser())
            .at_least(1)
            .collect::<Vec<_>>()
            )
        .then_ignore(gap_parser())
        .then_ignore(just(Token::TisTis))
        .map(|hoons| Hoon::WutPam(hoons))
}

fn wutbar_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutBar)
       .ignore_then(gap_parser())
       .ignore_then(hoon.clone()
            .separated_by(gap_parser())
            .at_least(1)
            .collect::<Vec<_>>()
            )
        .then_ignore(gap_parser())
        .then_ignore(just(Token::TisTis))
        .map(|hoons| Hoon::WutBar(hoons))
}

fn wuthep_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutHep)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser()) //  handle non-wing cases here
        .then_ignore(gap_parser())
        .then(spec_parser(hoon.clone())
                .then_ignore(gap_parser())
                .then(hoon.clone())
                .then_ignore(gap_parser())
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>()
        )
        .then_ignore(gap_parser())
        .then_ignore(just(Token::TisTis))
        .map(|(w, spec_hoon)| Hoon::WutHep(w, spec_hoon))
}

fn name_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {
        Token::Name(name_str) => {
            let wing = vec![Limb::Term(name_str.to_string())];
            Hoon::Wing(wing)
        }
    }.labelled("Name")
}

fn spec_term_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let buc_parser =      // %$
        just(Token::Cen)
        .ignore_then(just(Token::Buc))
        .map(|_| Spec::Leaf("%tas".to_string(), "%$".to_string()));

    let number_parser =      // %123
        just(Token::Cen)
        .ignore_then(select! { Token::Number(n) => n })
        .map(|n| Spec::Leaf("%ud".to_string(), n.to_string()));


    let name_parser =      // %foo
        just(Token::Cen)
        .ignore_then(select! { Token::Name(s) => s })
        .map(|s| Spec::Leaf("%tas".to_string(), s.to_string()));

    choice((
        buc_parser,
        number_parser,
        name_parser
    ))
}

fn const_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let buc_const_parser =      // %$
        just(Token::Cen)
        .ignore_then(just(Token::Buc))
        .map(|_|
            Hoon::Rock("%tas".to_string(), Noun::Atom("%$".to_string()))
        );

    let number_const_parser =      // %123
        just(Token::Cen)
        .ignore_then(select! { Token::Number(n) => n })
        .map(|n| Hoon::Rock("%ud".to_string(), Noun::Atom(n.to_string())));


    let name_const_parser =      // %foo
        just(Token::Cen)
        .ignore_then(select! { Token::Name(s) => s })
        .map(|s| Hoon::Rock("%tas".to_string(), Noun::Atom(s.to_string())));

    choice((
        buc_const_parser,
        number_const_parser,
        name_const_parser
    ))
}

fn coltar_irregular_parser<'tokens, 'src: 'tokens, I>(
    wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    wide.clone()
        .separated_by(just(Token::Ace))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Sel), just(Token::Ser))
        .map(|hoons| Hoon::ColTar(hoons))
}

fn bucwut_irregular_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucWutIrregular)
        .ignore_then(spec_wide.clone()
              .separated_by(just(Token::Ace))
              .at_least(1)
              .collect::<Vec<_>>()
            )
        .then_ignore(just(Token::Par))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Spec::BucWut(Box::new(first.clone()), rest.to_vec())
        })
}

fn buctis_irregular_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Name(n) => n }  //  foo=bar
        .then_ignore(just(Token::Tis))
        .then(spec_wide.clone())
        .map(|(n, s)| Spec::BucTis(Skin::Term(n.to_string()), Box::new(s)))
        .or(
            just(Token::Tis)
            .ignore_then(select! { Token::Name(n) => n }  // =foo=bar
                            .then_ignore(just(Token::Tis))
                            .then(spec_wide.clone())
                            .map(|(name, spec)| (Some(name), spec))
                        .or(spec_wide.clone()
                            .map(|spec| (None, spec)))      //   =bar
                        .try_map(|(name, spec), span| {
                            let auto = autoname(spec.clone());
                            match auto {
                                None => Err(Rich::custom(span, "cannot autoname")),
                                Some(auto_term) => {
                                    let term = match name {
                                        None => auto_term.to_string(),
                                        Some(n) => {
                                            let new_name = format!("{}-{}", n, auto_term);
                                            new_name
                                        }
                                    };
                                    Ok(Spec::BucTis(Skin::Term(term), Box::new(spec.clone())))
                                }
                            }
                        })
                    )
        )
}

fn buccol_irregular_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
        .separated_by(just(Token::Ace))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Sel), just(Token::Ser))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Spec::BucCol(Box::new(first.clone()), rest.to_vec())
        })
}

fn tisdot_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisDot)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::TisDot(p, Box::new(q), Box::new(r)))
}

fn tislus_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::TisLus)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::TisLus(Box::new(p), Box::new(q)))
}

fn tisgal_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::TisGal)
        .then_ignore(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::TisGal(Box::new(p), Box::new(q)))
}

fn tisgar_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::TisGar)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::TisGar(Box::new(p), Box::new(q)))
}

fn wing_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, WingType, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let name_parser =      //  Name or $
        just(Token::Buc)
            .map(|_| "%$".to_string())
            .or(select! { Token::Name(name) => name.to_string() });

    let com_parser =   //  ,
        just(Token::Com)
        .map(|_| Limb::Axis(0));

    let ket_name_parser =   //  ^^name or name
        just(Token::Ket)
            .repeated()
            .count()
            .then(name_parser)
            .map(|(cnt, name)| {
                if cnt == 0 {
                    return Limb::Term(name);
                } else {
                    return Limb::Parent(cnt as u64, Some(name));
                }
            });

    let lus_number_parser =   //  +10
            just(Token::Lus)
                .ignore_then(select! {Token::Number(n) => n.to_string()})
                .map(|n| {
                    let num = n.parse::<u64>().expect("Invalid number: lus_number_parser");
                    Limb::Axis(num)}
                );

    let pam_number_parser =   //  &10
            just(Token::Pam)
                .ignore_then(select! {Token::Number(n) => n.to_string()})
                .map(|n| {
                    let num = n.parse::<u64>().expect("Invalid number: pam_number_parser");
                    Limb::Axis(left_child(num))
                });

    let bar_number_parser =  //  |10
            just(Token::Bar)
                .ignore_then(select! {Token::Number(n) => n.to_string()})
                .map(|n| {
                    let num = n.parse::<u64>().expect("Invalid number: bar_number_parser");
                    Limb::Axis(right_child(num))
                });

    let dot_parser =  //  .
            just(Token::Dot)
                .map(|_| Limb::Axis(1));

    let lus_parser =  //  +
        just(Token::Lus)
            .map(|_| Limb::Axis(3));

    let hep_parser =  //  -
        just(Token::Hep)
            .map(|_| Limb::Axis(3));

    let lark_parser =   //    +>-<  notation
            select! { Token::LarkExpression(str) => {
                let mut axis = 1;
                for c in str.chars() {
                    match c {
                        '+' | '>' => axis = peg(axis, 3).expect("peg failed: lark_wing_parser"),
                        '-' | '<' => axis = peg(axis, 2).expect("peg failed: lark_wing_parser"),
                        _ => axis = 1,
                    }
                }
                Limb::Axis(right_child(axis))
            }};

    com_parser
        .or(ket_name_parser)
        .or(lus_number_parser)
        .or(pam_number_parser)
        .or(bar_number_parser)
        .or(lark_parser)
        .or(dot_parser)
        .or(lus_parser)
        .or(hep_parser)
        .separated_by(just(Token::Dot))
        .at_least(1)
        .collect::<Vec<_>>()
        .labelled("Wing")
        // .map(|limbs| Hoon::Wing(limbs))
}


// fn wing_parser<'tokens, 'src: 'tokens, I>(
// ) -> impl Parser<'tokens, I, WingType, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
// where
//     I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
// {
//     // Helper: parse a decimal number token into u64 (fails if invalid)
//     let number_u64 = select! {
//         Token::Number(n) => n.parse::<u64>().ok()
//     }
//     .labelled("valid number")
//     .try_map(|num_opt, span| {
//         num_opt.ok_or(Rich::custom(span, "invalid number"))
//     });

//     // Limb parsers
//     let com = just(Token::Com).to(Limb::Axis(0));

//     let dot = just(Token::Dot).to(Limb::Axis(1));

//     let lus = just(Token::Lus).to(Limb::Axis(3));
//     let hep = just(Token::Hep).to(Limb::Axis(3));

//     let name_or_dollar = just(Token::Buc).to("%$".to_string())
//         .or(select! { Token::Name(name) => name.to_string() });

//     let ket_name = just(Token::Ket)
//         .repeated()
//             .count()                    // ← returns usize
//         .then(name_or_dollar)
//         .map(|(kets, name)| {
//             let cnt = kets as u64;
//             if cnt == 0 {
//                 Limb::Term(name)
//             } else {
//                 Limb::Parent(cnt, Some(name))
//             }
//         });

//     let prefixed_number = |prefix: Token<'src>, f: fn(u64) -> u64| {
//         just(prefix)
//             .ignore_then(number_u64.clone())
//             .map(move |n| Limb::Axis(f(n)))
//     };

//     let lus_num = prefixed_number(Token::Lus, |n| n);
//     let pam_num = prefixed_number(Token::Pam, left_child);
//     let bar_num = prefixed_number(Token::Bar, right_child);

//     let lark = select! {
//         Token::LarkExpression(s) => {
//             let mut axis = 1u64;
//             for c in s.chars() {
//                 axis = match c {
//                     '+' | '>' => peg(axis, 3).unwrap_or(1),
//                     '-' | '<' => peg(axis, 2).unwrap_or(1),
//                     _ => 1,
//                 };
//             }
//             Limb::Axis(right_child(axis))
//         }
//     };

//     // Combine all limb alternatives
//     let limb = com
//         .or(ket_name)
//         .or(lus_num)
//         .or(pam_num)
//         .or(bar_num)
//         .or(lark)
//         .or(dot)
//         .or(lus)
//         .or(hep);

//     // A wing is one or more limbs separated by dots
//     limb
//         .separated_by(just(Token::Dot))
//         .at_least(1)
//         .collect::<Vec<_>>()
//         .labelled("Wing")
// }

fn jet_hooks_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Vec<(Term, Hoon)>, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
   let pair =   just(Token::Cen)
                .ignore_then(select! {Token::Name(n) => n.to_string()})
                .then_ignore(gap_parser())
                .then(hoon.clone())
                .then_ignore(gap_parser())
                .map(|(name, h)| (name, h));

    just(Token::Sig).to(Vec::new())
        .or(
            just(Token::TisTis)
            .ignore_then(just(Token::Gap))
            .ignore_then(pair
                        .repeated()
                        .at_least(1)
                         .collect::<Vec<(Term, Hoon)>>()
                        )
            .then_ignore(just(Token::TisTis))
        )
}

fn list_spec_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Vec<Term>, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{

   let wid =  select! { Token::Name(s) => s.to_string() }
                .separated_by(just(Token::Ace))
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Sel), just(Token::Ser));

   wid
    // pair.repeated().at_least(1).collect::<Vec<(WingType, Hoon)>>()
}

fn list_wing_hoon_tall_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Vec<(WingType, Hoon)>, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
   let pair = wing_parser()
                .then_ignore(gap_parser())
                .then(hoon.clone())
                .then_ignore(gap_parser())
                .map(|(name, h)| (name, h));

    pair.repeated().at_least(1).collect::<Vec<(WingType, Hoon)>>()
}

fn cencab_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::CenCab)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser())
        .then_ignore(gap_parser())
        .then(list_wing_hoon_tall_parser(hoon.clone()))
        .then_ignore(just(Token::TisTis))
        .map(|(p, q)| Hoon::CenCab(p, q))
}

fn tiscol_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::TisCol)
        .then_ignore(gap_parser())
        .ignore_then(list_wing_hoon_tall_parser(hoon.clone()))
        .then_ignore(just(Token::TisTis))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::TisCol(p, Box::new(q)))
}

// fn tismic_parser<'tokens, 'src: 'tokens, I>(
//     hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
// ) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
// where
//     I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
// {
//        just(Token::TisMic)
//         .ignore_then(gap_parser())
//         .ignore_then(spec_parser(hoon.clone()))
//         .then_ignore(gap_parser())
//         .then(hoon.clone())
//         .then_ignore(gap_parser())
//         .then(hoon.clone())
//         .map(|((p, q), r)| Hoon::TisMic(Box::new(p), Box::new(r), Box::new(q)))
// }

// fn named_noun_parser<'tokens, 'src: 'tokens, I>(
//     hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
// ) -> impl Parser<'tokens, I, Skin, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
// where
//     I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
// {

//     let not_name =   just(Token::Tis)
//                     .ignore_then(spec_wide_parser())     //      =/  =foo
//                     .filter_map(|s, _| {
//                         utils::autoname(&s).map(|n| {
//                             Skin::Name(
//                                 n,
//                                 Box::new(Skin::Spec(
//                                     Box::new(s),
//                                     Box::new(Skin::Base(BaseType::Noun)),
//                                 )),
//                             )
//                         })
//                     })
//                     // .map(|s| {
//                     //     let t = utils::autoname(s);
//                     //     match t {
//                     //         None => fail,
//                     //         Some(n) => {
//                     //           let skin =
//                     //                     Skin::Spec(Box::new(s),
//                     //                         Box::new(Skin::Base(BaseType::Noun)));
//                     //           Skin::Name(n, Box::new(skin))
//                     //         }
//                     //     }
//                     // )
// }

// fn tisfas_parser<'tokens, 'src: 'tokens, I>(
//     hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
// ) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
// where
//     I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
// {
//        just(Token::TisFas)
//         .ignore_then(gap_parser())
//         .ignore_then(skin_parser())
//         .then_ignore(gap_parser())
//         .then(hoon.clone())
//         .then_ignore(gap_parser())
//         .then(hoon.clone())
//         .map(|((p, q), r)| Hoon::TisFas(p, Box::new(q), Box::new(r)))
// }

fn jet_signature_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Chum, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let lef_parser = just(Token::Cen)  //  %k
                .ignore_then(select!
                    { Token::Name(s) => Chum::Lef(s.to_string())}
                );

    let stdkel_parser = just(Token::Cen)  //  %k.138
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

    stdkel_parser
    .or(lef_parser)
}

fn sigcab_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigCab)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::SigCab(Box::new(p), Box::new(q)))
}

fn sigcen_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigCen)
        .ignore_then(gap_parser())
        .ignore_then(jet_signature_parser())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(jet_hooks_parser(hoon.clone()))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(((p, q), r), s)| Hoon::SigCen(p, Box::new(q), r, Box::new(s)))
}

fn sigfas_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigFas)
        .ignore_then(gap_parser())
        .ignore_then(jet_signature_parser())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::SigFas(p, Box::new(q)))
}

fn tiswut_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisWut)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(((p, q), r), s)| Hoon::TisWut(p, Box::new(q), Box::new(r), Box::new(s)))
}

fn wutgar_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutGar)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .map(|(p, q)| Hoon::WutGar(Box::new(p), Box::new(q)))
}


fn wutdot_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutDot)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::WutCol(Box::new(p), Box::new(r), Box::new(q)))
}

fn wutgal_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutGal)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::WutGal(Box::new(p), Box::new(q)))
}

fn wutcol_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutCol)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::WutCol(Box::new(p), Box::new(q), Box::new(r)))
}

fn bartis_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarTis)
        .ignore_then(gap_parser())
        .ignore_then(spec_parser(hoon.clone()))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| Hoon::BarTis(Box::new(s), Box::new(h)))
}

fn barbuc_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarBuc)
        .ignore_then(gap_parser())
        .ignore_then(list_spec_parser())
        .then_ignore(gap_parser())
        .then(spec_parser(hoon.clone()))
        .map(|(list, h)| Hoon::BarBuc(list, Box::new(h)))
}

fn barcol_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarCol)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| Hoon::BarCol(Box::new(s), Box::new(h)))
}

fn barhep_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarHep)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .map(|h| Hoon::BarHep(Box::new(h)))
}

fn increment_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Increment)
        .ignore_then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|h| Hoon::DotLus(Box::new(h)))
}

fn gatecall_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Pal)
        .ignore_then(hoon.clone())
        .then(
            just(Token::Ace)
                .ignore_then(hoon.clone())
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
    .map(|(func, args)| Hoon::CenCol(Box::new(func), args))
    .then_ignore(just(Token::Par))
}

fn kethep_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetHep)
        .ignore_then(gap_parser())
        .ignore_then(spec_parser(hoon.clone()))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| {
            Hoon::KetHep(Box::new(s), Box::new(h))
        })
}

fn ketsig_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetSigWide).
        ignore_then(hoon.clone()).
        then_ignore(just(Token::Par)).
        map(|h| Hoon::KetSig(Box::new(h)))
}

fn zapzap_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ZapZap).map(|_| Hoon::ZapZap)
}

fn number_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {
        Token::Number(num_str) => {
            Hoon::Sand("ud".to_string(), Noun::Atom(num_str.to_string()))
        }
    }.labelled("Number")
}

fn wing_or_wing_hoon_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    // wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    wing_parser()
    .then(just(Token::Lus).or(just(Token::Fas)).ignore_then(hoon.clone()).or_not())
        .try_map(|(wing, maybe_hoon), span| {
            match maybe_hoon {
                Some(hoon) => {
                    if let [Limb::Term(t)] = wing.as_slice() {
                        Ok(Hoon::Pair(
                            Box::new(Hoon::Rock("%tas".to_string(), Noun::Atom(t.clone()))),
                            Box::new(hoon),
                        ))
                    } else {
                        // it will be discarded if backtracking succeeds
                        Err(Rich::custom(span, "invalid wing shape"))
                    }
                }
                None => Ok(Hoon::Wing(wing)),
            }
        })
}

fn buclus_wide_parser<'tokens, 'src: 'tokens, I>(
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucLusWide)
        .ignore_then(select! { Token::Name(s) => s.to_string() }
                    .then_ignore(just(Token::Ace))
                    .then(spec.clone())
        )
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Spec::BucLus(p, Box::new(q)))
}

fn buchep_wide_parser<'tokens, 'src: 'tokens, I>(
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucHepWide)
        .ignore_then(spec.clone()
                    .then_ignore(just(Token::Ace))
                    .then(spec.clone())
        )
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Spec::BucHep(Box::new(p), Box::new(q)))
}

fn buccen_parser<'tokens, 'src: 'tokens, I>(
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucCen)
        .ignore_then(gap_parser())
        .ignore_then(spec.clone()
                    .separated_by(gap_parser())
                    .at_least(1)
                    .collect::<Vec<_>>()
            )
        .then_ignore(gap_parser())
            .then_ignore(just(Token::TisTis))
        .map(|specs| {
                let (first, rest) = specs.split_first().unwrap();
                Spec::BucCen(Box::new(first.clone()), rest.to_vec())
            })
}

fn bucpat_spec_wide_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucPatWide)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(spec_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Spec::BucPat(Box::new(p), Box::new(q)))
}

fn bucpat_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucPatWide)
        .ignore_then(spec_wide_parser(hoon_wide.clone()))
        .then_ignore(just(Token::Ace))
        .then(spec_wide_parser(hoon_wide.clone()))
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucPat(Box::new(p),
                                                        Box::new(q)))))
}

fn luslus_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, (String, Hoon), extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::LusLus)
        .ignore_then(gap_parser())
        .ignore_then(select! { Token::Name(s) => s })
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(name, hoon)| (name.to_string(), hoon))
}

fn lusbuc_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, (String, Hoon), extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::LusBuc)
        .ignore_then(gap_parser())
        .ignore_then(select! { Token::Name(s) => s })
        .then_ignore(gap_parser())
        .then(spec_parser(hoon.clone()))
        .map(|(name, spec)| (name.to_string(),
                             Hoon::KetCol(Box::new(Spec::Name(name.to_string(),
                                                    Box::new(spec))))))
}

fn chapters_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Vec<(Option<String>, Vec<(String, Hoon)>)>, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let optional_chapter_label = just(Token::LusBar)
        .then_ignore(gap_parser())
        .then(just(Token::Cen))
        .ignore_then(select! { Token::Name(s) => s.to_string() })
        .then_ignore(gap_parser())
        .or_not();

    let chapter = optional_chapter_label
        .then(luslus_parser(hoon.clone())
              .or(lusbuc_parser(hoon.clone()))
              .then_ignore(gap_parser())
              .repeated().at_least(1).collect::<Vec<_>>()
            );

    chapter.repeated().at_least(1).collect::<Vec<_>>()
}

fn kethep_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Tic)
        .ignore_then(spec_wide_parser(hoon_wide.clone()))
        .then_ignore(just(Token::Tic))
        .then(hoon_wide.clone())
        .map(|(s, w)| Hoon::KetHep(Box::new(s), Box::new(w)))
}

fn centis_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let wing_hoon = wing_parser()
                    .then_ignore(just(Token::Ace))
                    .then(hoon.clone());

    let list_wing = wing_hoon
                    .separated_by(just(Token::Com).then(just(Token::Ace)))
                    .at_least(1)
                    .collect::<Vec<_>>()
                    .then_ignore(just(Token::Par));

    select! { Token::CenTisIrregular(n) => {
                vec![Limb::Term(n.to_string())]
              }
            }
    .then(list_wing)
    .map(|(name, list)| Hoon::CenTis(name, list))
}

fn barcen_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarCen)
        .ignore_then(gap_parser())
        .ignore_then(chapters_parser(hoon.clone()))
        .then_ignore(just(Token::HepHep))
        .map(|chapters_vec: Vec<(Option<String>, Vec<(String, Hoon)>)>| {
            let mut tome_map = HashMap::new();
            for (opt_label, arms_vec) in chapters_vec {
                let mut arms_map = HashMap::new();
                for (name, hoon) in arms_vec {
                    arms_map.insert(name, hoon);
                }
                let key = opt_label.unwrap_or_else(|| "$".to_string());
                let what = "".to_string();
                let tome: Tome = (what, arms_map);
                tome_map.insert(key, tome);
            }
            Hoon::BarCen(None, tome_map)
        })
        .boxed()
}

// fn parenthesis_spec_parser<'tokens, 'src: 'tokens, I>(
//     hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
//     // spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
// ) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
// where
//     I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
// {
//     hoon_wide
//         .then_ignore(just(Token::Ace))
//         .then(spec_wide.clone()
//               .separated_by(just(Token::Ace))
//               .at_least(1)
//               .collect::<Vec<_>>()
//         )
//         .delimited_by(just(Token::Pal), just(Token::Par))
//         .map(|(name, specs)| Spec::Make(name, specs))
// }

fn reference_spec_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    wing_parser()
        .separated_by(just(Token::Col))
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|wings: Vec<WingType>| {
                    let (first, rest) = wings.split_first().unwrap();
                    Spec::Like(first.to_vec(), rest.to_vec())
                })
}

fn spec_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    recursive(|spec_wide| {
        choice((
            bucpat_spec_wide_parser(spec_wide.clone()),  // $%(foo bar)
            buchep_wide_parser(spec_wide.clone()),       // $-(foo bar)
            buclus_wide_parser(spec_wide.clone()),       // $+(foo bar)
            buccol_irregular_parser(spec_wide.clone()),  // [foo=bar foo=bar]
            buctis_irregular_parser(spec_wide.clone()),  // foo=bar, =bar,  =foo=bar
            reference_spec_parser(spec_wide.clone()),    // foo or foo:bar
            bucwut_irregular_parser(spec_wide.clone()),  // ?(foo bar)
            // parenthesis_spec_parser(spec_wide.clone()),  // (foo bar)
            just(Token::Wut).to(Spec::Base(BaseType::Flag)),
            just(Token::Sig).to(Spec::Base(BaseType::Null)),
            just(Token::Tar).to(Spec::Base(BaseType::Noun)),
            just(Token::CenBar).to(Spec::Leaf("%f".to_string(), "1".to_string())),
            just(Token::CenPam).to(Spec::Leaf("%f".to_string(), "0".to_string())),
            aura_spec_parser(), //  @foo
            spec_term_parser(), // %$, %foo, %123
        )).boxed()
    })
}

fn spec_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    recursive(|spec| {
        choice((
            buccen_parser(spec.clone()),
            spec_wide_parser(hoon.clone()),
        )).boxed()
    })
}

fn hoon_tall_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        barcen_parser(hoon.clone()),
        bartis_parser(hoon.clone()),
        barhep_parser(hoon.clone()),
        barcol_parser(hoon.clone()),
        barbuc_parser(hoon.clone()),
        // tismic_parser(hoon.clone()),
        // tisfas_parser(hoon.clone()),
        tiscol_parser(hoon.clone()),
        tisgal_parser(hoon.clone()),
        tisgar_parser(hoon.clone()),
        tiswut_parser(hoon.clone()),
        kethep_parser(hoon.clone()),
        wutcol_parser(hoon.clone()),
        wutdot_parser(hoon.clone()),
        wutgar_parser(hoon.clone()),
        wutgal_parser(hoon.clone()),
        wutpam_parser(hoon.clone()),
        wutbar_parser(hoon.clone()),
        wuthep_parser(hoon.clone()),
        tisdot_parser(hoon.clone()),
        tislus_parser(hoon.clone()),
        sigcen_parser(hoon.clone()),
        sigfas_parser(hoon.clone()),
        sigcab_parser(hoon.clone()),
        cencab_parser(hoon.clone()),
        zapzap_parser(),
    ))
}

fn hoon_wide_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    recursive(|hoon_wide| {
        choice((
            ketsig_wide_parser(hoon_wide.clone()),            //  ^~(p)
            bucpat_wide_parser(hoon_wide.clone()),
            kettis_irregular_parser(hoon_wide.clone()),       //  p=q
            wing_or_wing_hoon_parser(hoon_wide.clone()),      // p,  p+q,  p/q
            dottis_irregular_parser(hoon_wide.clone()),       //  =(p q)
            coltar_irregular_parser(hoon_wide.clone()),       // [p ... pn]
            kethep_irregular_parser(hoon_wide.clone()),       //  `p`q
            wutzap_irregular_parser(hoon_wide.clone()),       //  !p
            wutbar_irregular_parser(hoon_wide.clone()),       //  |(p q)
            // tisgal_irregular_parser(hoon_wide.clone()),       //  p:q
            centis_irregular_parser(hoon_wide.clone()),       //  a(b c, d e, f g)
            gatecall_parser(hoon_wide.clone()),               //  (a b c d e)
            increment_parser(hoon_wide.clone()),              //  +(a)
            aura_hoon_parser(),
            tape_parser(),
            const_parser(),
            name_parser(),
            number_parser(),
            just(Token::CenBar).to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))),
            just(Token::CenPam).to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))),
            just(Token::Wut).to(Hoon::Base(BaseType::Flag)),
            just(Token::Sig).to(Hoon::Bust(BaseType::Null)),
            just(Token::Lus).to(Hoon::CenTis(vec![Limb::Axis(3)], Vec::new())),
        )).boxed()
    })
}

fn hoon_wide_wrapper_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide_parser().labelled("hoon-wide")
        .separated_by(just(Token::Col))   //  irregular tisgal    a:b
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|hoons: Vec<Hoon>| {
                    let (first, rest) = hoons.split_first().unwrap();
                    rest.into_iter().fold(first.clone(), |acc, next| {
                        Hoon::TisGal(Box::new(acc.clone()), Box::new(next.clone()))
                    })
                })
}

fn parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    recursive(|hoon| {
        choice((
            hoon_tall_parser(hoon.clone()).labelled("hoon-tall"),
            hoon_wide_wrapper_parser(),
        ))
        .boxed()
    })
    .padded_by(gap_parser())
    .boxed()
}

fn main() {
    let source = fs::read_to_string("./src/test2.hoon").expect("Failed to read file");
    let start = Instant::now();

    let token_iter = Token::lexer(&source)
        .spanned()
        // Convert logos errors into tokens. We want parsing to be recoverable and not fail at the lexing stage, so
        // we have a dedicated `Token::Error` variant that represents a token error that was previously encountered
        .map(|(tok, span)| match tok {
            // Turn the `Range<usize>` spans logos gives us into chumsky's `SimpleSpan` via `Into`, because it's easier
            // to work with
            Ok(tok) => (tok, span.into()),
            Err(()) => (Token::LexerError, span.into()),
        });

    let token_stream = Stream::from_iter(token_iter)
        // Tell chumsky to split the (Token, SimpleSpan) stream into its parts so that it can handle the spans for us
        // This involves giving chumsky an 'end of input' span: we just use a zero-width span at the end of the string
        .map((0..source.len()).into(), |(t, s): (_, _)| (t, s));

    match parser().parse(token_stream).into_result() {
        Ok(res) => {
            let took = start.elapsed();
            println!("{:?}", res);
            println!("took: {:?}", took);
        },
        Err(errs) => {
            for err in errs {
                Report::build(ReportKind::Error, ((), err.span().into_range()))
                    .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                    .with_code(3)
                    .with_message(err.to_string())
                    .with_label(
                        Label::new(((), err.span().into_range()))
                            .with_message(err.reason().to_string())
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint(Source::from(source.clone()))
                    .unwrap();
            }
        }
    };
}