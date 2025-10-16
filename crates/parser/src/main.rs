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
    just(Token::Tis)
        .ignore_then(just(Token::Pal))
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

fn wutpam_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutPamWide)
       .ignore_then(hoon_wide.clone()
            .separated_by(just(Token::Ace))
            .at_least(1)
            .collect::<Vec<_>>())
        .then_ignore(just(Token::Par))
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

fn wutbar_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutBarWide)
       .ignore_then(hoon.clone()
                    .separated_by(just(Token::Ace))
                    .at_least(1)
                    .collect::<Vec<_>>())
        .then_ignore(just(Token::Par))
        .map(|hoons| Hoon::WutBar(hoons))
}

fn wuthep_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutHep)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser()) //  handle non-wing cases here
        .then_ignore(gap_parser())
        .then(spec.clone()
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

fn concatanate_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
      .then_ignore(just(Token::Ket))
      .then(hoon_wide.clone())
      .map(|(p, q)| Hoon::Pair(Box::new(p), Box::new(q)))
}

fn wutket_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutKet)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser()) //  handle non-wing cases here
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::WutKet(p, Box::new(q), Box::new(r)))
}

fn wutket_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutKetWide)
        .ignore_then(wing_parser()) //  handle non-wing cases here
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|((p, q), r)| Hoon::WutKet(p, Box::new(q), Box::new(r)))
}

fn wutsig_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutSig)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser()) //  handle non-wing cases here
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::WutSig(p, Box::new(q), Box::new(r)))
}

fn wutsig_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::WutSigWide)
        .ignore_then(wing_parser()) //  handle non-wing cases here
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|((p, q), r)| Hoon::WutSig(p, Box::new(q), Box::new(r)))
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

// fn coltar_irregular_parser<'tokens, 'src: 'tokens, I>(
//     wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
// ) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
// where
//     I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
// {
//     wide.clone()
//         .separated_by(just(Token::Ace))
//         .at_least(1)
//         .collect::<Vec<_>>()
//         .delimited_by(just(Token::Sel), just(Token::Ser))
//         .map(|hoons| Hoon::ColTar(hoons))
// }

fn buccab_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cab)
        .ignore_then(hoon_wide.clone())
        .map(|h| Hoon::KetCol(Box::new(Spec::BucCab(h))))
}

fn buccab_spec_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cab)
        .ignore_then(hoon_wide.clone())
        .map(|h| Spec::BucCab(h))
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

fn bucwut_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucWut)
        .ignore_then(gap_parser())
        .ignore_then(spec_wide.clone()
              .separated_by(gap_parser())
              .at_least(1)
              .collect::<Vec<_>>()
            )
        .then_ignore(gap_parser())
        .then_ignore(just(Token::TisTis))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Hoon::KetCol(Box::new(
                        Spec::BucWut(Box::new(first.clone()),
                                      rest.to_vec())
            ))
        })
}

fn bucwut_spec_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucWut)
        .ignore_then(gap_parser())
        .ignore_then(spec_wide.clone()
              .separated_by(gap_parser())
              .at_least(1)
              .collect::<Vec<_>>()
            )
        .then_ignore(gap_parser())
        .then_ignore(just(Token::TisTis))
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

fn zapdot_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ZapDot)   // TODO: this needs to enable/disable tracing, think about this when we are working on the compiler?
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
}

fn ketdot_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetDotWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::KetDot(Box::new(p), Box::new(q)))
}

fn barcol_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarColWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::BarCol(Box::new(p), Box::new(q)))
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
                        '+' | '>' => axis = peg(3, axis).expect("peg failed: lark_wing_parser"),
                        '-' | '<' => axis = peg(2, axis).expect("peg failed: lark_wing_parser"),
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
}

fn list_term_hoon_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Vec<(Term, Hoon)>, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {Token::Name(n) => n.to_string()}
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .repeated()
        .at_least(1)
        .collect::<Vec<(Term, Hoon)>>()
}

fn jet_hooks_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Vec<(Term, Hoon)>, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Sig).to(Vec::new())
        .or(
            just(Token::TisTis)
            .ignore_then(just(Token::Gap))
            .ignore_then(just(Token::Cen)
            .ignore_then(
                list_term_hoon_parser(hoon.clone()))
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

fn list_wing_hoon_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Vec<(WingType, Hoon)>, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let pair = wing_parser()
                .then_ignore(just(Token::Ace))
                .then(hoon.clone());

    pair
        .separated_by(just(Token::Com).then(just(Token::Ace)))
        .at_least(1)
        .collect::<Vec<_>>()
}

fn list_hoon_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Vec<(Hoon)>, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
        .separated_by(just(Token::Ace))
        .at_least(1)
        .collect::<Vec<(Hoon)>>()
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
                .then_ignore(gap_parser());

    pair.repeated().at_least(1).collect::<Vec<(WingType, Hoon)>>()
}

fn cenhep_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::CenHep)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::CenHep(Box::new(p), Box::new(q)))
}

fn cenlus_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::CenLus)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::CenLus(Box::new(p), Box::new(q), Box::new(r)))
}

fn cenhep_spec_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::CenHep)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(spec.clone())
        .map(|(p, q)| Spec::Make(p, vec![q]))
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

fn tisbar_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::TisBar)
        .ignore_then(gap_parser())
        .ignore_then(spec.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::TisBar(Box::new(p), Box::new(q)))
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

fn variable_name_type_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Skin, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let not_named = just(Token::Tis)  // =/  =foo
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

    choice((not_named, named, just_type)).boxed()
}

fn tisfas_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisFas)
        .ignore_then(gap_parser())
        .ignore_then(variable_name_type_parser(spec_wide.clone()))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::TisFas(p, Box::new(q), Box::new(r)))
}

fn tisket_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisKet)
        .ignore_then(gap_parser())
        .ignore_then(variable_name_type_parser(spec_wide.clone()))
        .then_ignore(gap_parser())
        .then(wing_parser())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(((p, q), r), s)| Hoon::TisKet(p, q, Box::new(r), Box::new(s)))
}

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

fn cord_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {Token::Cord(s) => Hoon::Sand("%t".to_string(), Noun::Atom(s.to_string()))}
}

fn term_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, String, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cen)
      .ignore_then(select! {Token::Name(s) => format!("%{}", s) })
}

fn siggar_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigGarWide)
        .ignore_then(term_parser())
        .then(just(Token::Dot)
             .ignore_then(hoon_wide.clone())
             .or_not()
              )
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|((term, maybe_hoon), q)|  {
            match maybe_hoon {
                None =>{
                    Hoon::SigGar(TermOrPair::Term(term), Box::new(q))
                }
                Some(h) => {
                    Hoon::SigGar(TermOrPair::Pair((term, Box::new(h))), Box::new(q))
                }
            }
        })
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
        .map(|((p, q), r)| Hoon::WutDot(Box::new(p), Box::new(r), Box::new(q)))
}

fn wuttis_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutTisWide)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(wing_parser())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::WutTis(Box::new(p), q))
}

fn wutgar_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutGarWide)
        .ignore_then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::WutGar(Box::new(p), Box::new(q)))
}

fn wutdot_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutDotWide)
        .ignore_then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|((p, q), r)| Hoon::WutDot(Box::new(p), Box::new(q), Box::new(r)))
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

fn wutcol_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutColWide)
        .ignore_then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|((p, q), r)| Hoon::WutCol(Box::new(p), Box::new(q), Box::new(r)))
}

fn bartar_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarTar)
        .ignore_then(gap_parser())
        .ignore_then(spec.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| Hoon::BarTar(Box::new(s), Box::new(h)))
}

fn bartis_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarTis)
        .ignore_then(gap_parser())
        .ignore_then(spec.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| Hoon::BarTis(Box::new(s), Box::new(h)))
}

fn bartis_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarTisWide)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(s, h)| Hoon::BarTis(Box::new(s), Box::new(h)))
}

fn barbuc_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarBuc)
        .ignore_then(gap_parser())
        .ignore_then(list_spec_parser())
        .then_ignore(gap_parser())
        .then(spec.clone())
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

fn function_call_parser<'tokens, 'src: 'tokens, I>(
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
                .collect::<Vec<_>>()
            )
    .then_ignore(just(Token::Par))
    .map(|(func, args)| Hoon::CenCol(Box::new(func), args))
}

fn kethep_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetHep)
        .ignore_then(gap_parser())
        .ignore_then(spec.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| {
            Hoon::KetHep(Box::new(s), Box::new(h))
        })
}

fn tisgar_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::TisGarWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::TisGar(Box::new(p), Box::new(q)))
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

fn barwut_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarWutWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|h| Hoon::BarWut(Box::new(h)))
}

fn bucsig_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucSigWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(spec_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(h, s)| Hoon::KetCol(Box::new(
                                    Spec::BucSig(h, Box::new(s))
                                )))
}

fn bucsig_spec_wide_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucSigWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(spec_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(h, s)| Spec::BucSig(h, Box::new(s)))
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
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucHepWide)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(spec_wide.clone())
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


fn ketlus_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetLus)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::KetLus(Box::new(p), Box::new(q)))
}

fn ketlus_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetLusWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::KetLus(Box::new(p), Box::new(q)))
}

fn bucpat_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucPatWide)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(spec_wide.clone())
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
        .ignore_then(just(Token::Buc).to("%$").or(select! { Token::Name(s) => s }))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(name, hoon)| (name.to_string(), hoon))
}

fn lusbuc_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, (String, Hoon), extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::LusBuc)
        .ignore_then(gap_parser())
        .ignore_then(select! { Token::Name(s) => s })
        .then_ignore(gap_parser())
        .then(spec.clone())
        .map(|(name, spec)| (name.to_string(),
                             Hoon::KetCol(Box::new(Spec::Name(name.to_string(),
                                                    Box::new(spec))))))
}

fn chapters_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
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
              .or(lusbuc_parser(hoon.clone(), spec.clone()))
              .then_ignore(gap_parser())
              .repeated().at_least(1).collect::<Vec<_>>()
            );

    chapter.repeated().at_least(1).collect::<Vec<_>>()
}

fn list_syntax_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigSel).to(true).or(just(Token::Sel).to(false))   //  ~[  or  [
        .then(hoon_wide.clone()
                    .separated_by(just(Token::Ace))
                    .at_least(1)
                    .collect::<Vec<_>>()
                    )
        .then(just(Token::SigSer).to(true).or(just(Token::Ser).to(false)))  //  ]~ or ]
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

fn tic_cell_construction_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Tic)
        .ignore_then(hoon_wide.clone())
        .map(|h| Hoon::Pair(Box::new(Hoon::Rock("%n".to_string(),
                                                     Noun::Atom("0".to_string()))),
                                 Box::new(h)))
}

fn kettar_irregular_parser<'tokens, 'src: 'tokens, I>(
    // hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Tar)
        .ignore_then(spec_wide.clone())
        .map(|s| Hoon::KetTar(Box::new(s)))
}

fn kethep_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Tic)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Tic))
        .then(hoon_wide.clone())
        .map(|(s, w)| Hoon::KetHep(Box::new(s), Box::new(w)))
}

fn centis_parser<'tokens, 'src: 'tokens, I>(
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
                    .collect::<Vec<_>>();

    just(Token::CenTis)
        .ignore_then(gap_parser())
        .ignore_then(wing_parser())
        .then_ignore(gap_parser())
        .then(list_wing_hoon_tall_parser(hoon.clone()))
        .then_ignore(just(Token::TisTis))
        .map(|(name, list)| Hoon::CenTis(name, list))
}

fn censig_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::CenSigIrregular)
        .ignore_then(wing_parser())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(list_hoon_wide_parser(hoon_wide.clone()))
        .then_ignore(just(Token::Par))
        .map(|((w, h), list)| Hoon::CenSig(w, Box::new(h), list))
}

fn centis_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    wing_parser()
        .then_ignore(just(Token::Pal))
        .then(list_wing_hoon_wide_parser(hoon_wide.clone()))
        .then_ignore(just(Token::Par))
        .map(|(name, list)| Hoon::CenTis(name, list))
}

fn core_tail_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, HashMap<Term, Tome>, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
        chapters_parser(hoon.clone(), spec.clone())
        .then_ignore(just(Token::HepHep))
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

fn barpat_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarPat)
        .ignore_then(gap_parser())
        .ignore_then(core_tail_parser(hoon.clone(), spec.clone()))
        .map(|map_term_tome| Hoon::BarPat(None, map_term_tome))
        .boxed()
}

fn barcab_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let aliases_parser =     //   +*  foo  1
                just(Token::LusTar)
                    .ignore_then(gap_parser())
                    .ignore_then(list_term_hoon_parser(hoon.clone()));

    just(Token::BarCab)
        .ignore_then(gap_parser())
        .ignore_then(spec.clone())
        .then_ignore(gap_parser())
        .then(aliases_parser.or_not().map(|x| x.unwrap_or(vec![])))
        .then_ignore(gap_parser())
        .then(core_tail_parser(hoon.clone(), spec.clone()))
        .map(|((spec, alas), map_term_tome)| Hoon::BarCab(Box::new(spec), alas, map_term_tome))
        .boxed()
}

fn barcen_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarCen)
        .ignore_then(gap_parser())
        .ignore_then(core_tail_parser(hoon.clone(), spec.clone()))
        .map(|map_term_tome| Hoon::BarCen(None, map_term_tome))
        .boxed()
}

fn parenthesis_spec_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
        .then(
            just(Token::Ace)
            .ignore_then(spec_wide.clone())
                // .separated_by(just(Token::Ace))
                // .at_least(1)
                .repeated()
                .collect::<Vec<_>>()
                .or_not()
                .map(|specs| specs.unwrap_or_default())  // ← Option<Vec> → Vec
        )
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(name, specs)| Spec::Make(name, specs))
}

fn reference_spec_parser<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    wing_parser()    //  review this, is the second part optional?
        .separated_by(just(Token::Col))
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|wings: Vec<WingType>| {
                    let (first, rest) = wings.split_first().unwrap();
                    Spec::Like(first.to_vec(), rest.to_vec())
                })
}

fn spec_wide_parser_fn<'tokens, 'src: 'tokens, I>(
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        bucpat_spec_wide_parser(spec_wide.clone()),  // $%(foo bar)
        bucsig_spec_wide_parser(spec_wide.clone(), hoon_wide.clone()),
        buchep_wide_parser(spec_wide.clone()),       // $-(foo bar)
        buclus_wide_parser(spec_wide.clone()),       // $+(foo bar)
        buccab_spec_irregular_parser(hoon_wide.clone()),   //  _p
        buctis_irregular_parser(spec_wide.clone()),  // foo=bar, =bar,  =foo=bar
        buccol_irregular_parser(spec_wide.clone()),  // [foo=bar foo=bar]
        reference_spec_parser(spec_wide.clone()),    // foo or foo:bar
        bucwut_irregular_parser(spec_wide.clone()),  // ?(foo bar)
        parenthesis_spec_parser(hoon_wide.clone(),
                                spec_wide.clone()),  // (foo bar)
        just(Token::Ket).to(Spec::Base(BaseType::Cell)),
        just(Token::Wut).to(Spec::Base(BaseType::Flag)),
        just(Token::Sig).to(Spec::Base(BaseType::Null)),
        just(Token::Tar).to(Spec::Base(BaseType::Noun)),
        just(Token::CenBar).to(Spec::Leaf("%f".to_string(), "1".to_string())),
        just(Token::CenPam).to(Spec::Leaf("%f".to_string(), "0".to_string())),
        aura_spec_parser(), //  @foo
        spec_term_parser(), // %$, %foo, %123
    )).labelled("spec-wide").boxed()
}

fn hoon_wide_parser_fn<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let parsers = vec![
        ketsig_wide_parser(hoon_wide.clone()).boxed(),
        tisgar_wide_parser(hoon_wide.clone()).boxed(),
        barwut_wide_parser(hoon_wide.clone()).boxed(),
        bucsig_wide_parser(hoon_wide.clone(), spec_wide.clone()).boxed(),
        bucpat_wide_parser(hoon_wide.clone(), spec_wide.clone()).boxed(),
        barcol_wide_parser(hoon_wide.clone()).boxed(),
        bartis_wide_parser(hoon_wide.clone(), spec_wide.clone()).boxed(),
        buccab_irregular_parser(hoon_wide.clone()).boxed(),              //  _p
        kettis_irregular_parser(hoon_wide.clone()).boxed(),              //  p=q
        centis_irregular_parser(hoon_wide.clone()).boxed(),              //  a(b c, d e, f g)
        censig_irregular_parser(hoon_wide.clone()).boxed(),              //  ~(a b c)
        wing_or_wing_hoon_parser(hoon_wide.clone()).boxed(),             //  p,  p+q,  p/q
        dottis_irregular_parser(hoon_wide.clone()).boxed(),              //  =(p q)
        list_syntax_parser(hoon_wide.clone()).boxed(),                   // [p ... pn], ~[foo], [foo]~
        kettar_irregular_parser(spec_wide.clone()).boxed(),              //  *foo
        kethep_irregular_parser(hoon_wide.clone(),
                                spec_wide.clone()).boxed(),              //  `p`q
        tic_cell_construction_parser(hoon_wide.clone()).boxed(),         //  `a
        wutzap_irregular_parser(hoon_wide.clone()).boxed(),              //  !p
        wutbar_wide_parser(hoon_wide.clone()).boxed(),
        wutbar_irregular_parser(hoon_wide.clone()).boxed(),              //  |(p q)
        wutsig_wide_parser(hoon_wide.clone()).boxed(),
        wuttis_wide_parser(hoon_wide.clone(), spec_wide.clone()).boxed(),
        wutgar_wide_parser(hoon_wide.clone()).boxed(),
        wutdot_wide_parser(hoon_wide.clone()).boxed(),
        wutcol_wide_parser(hoon_wide.clone()).boxed(),
        wutket_wide_parser(hoon_wide.clone()).boxed(),
        wutpam_wide_parser(hoon_wide.clone()).boxed(),
        siggar_wide_parser(hoon_wide.clone()).boxed(),
        function_call_parser(hoon_wide.clone()).boxed(),      //  (a b)
        increment_parser(hoon_wide.clone()).boxed(),          //  +(a)
        ketlus_wide_parser(hoon_wide.clone()).boxed(),
        ketdot_wide_parser(hoon_wide.clone()).boxed(),
        aura_hoon_parser().boxed(),
        tape_parser().boxed(),
        const_parser().boxed(),
        cord_parser().boxed(),
        name_parser().boxed(),
        number_parser().boxed(),
        select! { Token::Hex(h) => Hoon::Sand("%ux".to_string(), Noun::Atom(h.to_string())) }.boxed(),
        just(Token::CenBar).to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just(Token::CenPam).to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::Pam).to(Hoon::Sand("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::Bar).to(Hoon::Sand("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just(Token::Tar).to(Hoon::Base(BaseType::Noun)).boxed(),
        just(Token::Wut).to(Hoon::Base(BaseType::Flag)).boxed(),
        just(Token::Sig).to(Hoon::Bust(BaseType::Null)).boxed(),
        just(Token::ZapZap).to(Hoon::ZapZap).boxed(),
        just(Token::Lus).to(Hoon::CenTis(vec![Limb::Axis(3)], Vec::new())).boxed(),
    ];

    let concat = choice(parsers).labelled("hoon-wide")  //  a^b
        .separated_by(just(Token::Ket))
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|parts| {
            parts.into_iter().reduce(|acc, next| {
                Hoon::Pair(Box::new(acc), Box::new(next))
            }).unwrap()
        });

    concat                                  //  a:b
        .separated_by(just(Token::Col))
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|parts| {
            parts.into_iter().reduce(|acc, next| {
                Hoon::TisGal(Box::new(acc), Box::new(next))
            }).unwrap()
        })
        .boxed()
}

fn hoon_tall_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec_wide: impl Parser<'tokens, I, Spec, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let parsers = vec![
            barcen_parser(hoon.clone(), spec.clone()).boxed(),
            barpat_parser(hoon.clone(), spec.clone()).boxed(),
            bartis_parser(hoon.clone(), spec.clone()).boxed(),
            barcab_parser(hoon.clone(), spec.clone()).boxed(),
            bartar_parser(hoon.clone(), spec.clone()).boxed(),
            barhep_parser(hoon.clone()).boxed(),
            barcol_parser(hoon.clone()).boxed(),
            barbuc_parser(hoon.clone(), spec.clone()).boxed(),
            bucwut_parser(spec.clone()).boxed(),
            tisbar_parser(hoon.clone(), spec.clone()).boxed(),
            tiscol_parser(hoon.clone()).boxed(),
            tisgal_parser(hoon.clone()).boxed(),
            tisgar_parser(hoon.clone()).boxed(),
            tiswut_parser(hoon.clone()).boxed(),
            tisfas_parser(hoon.clone(), spec_wide.clone()).boxed(),
            tisket_parser(hoon.clone(), spec_wide.clone()).boxed(),
            kethep_parser(hoon.clone(), spec.clone()).boxed(),
            ketlus_parser(hoon.clone()).boxed(),
            wutcol_parser(hoon.clone()).boxed(),
            wutdot_parser(hoon.clone()).boxed(),
            wutgar_parser(hoon.clone()).boxed(),
            wutgal_parser(hoon.clone()).boxed(),
            wutpam_parser(hoon.clone()).boxed(),
            wutbar_parser(hoon.clone()).boxed(),
            wuthep_parser(hoon.clone(), spec.clone()).boxed(),
            wutsig_parser(hoon.clone()).boxed(),
            wutket_parser(hoon.clone()).boxed(),
            tisdot_parser(hoon.clone()).boxed(),
            tislus_parser(hoon.clone()).boxed(),
            sigcen_parser(hoon.clone()).boxed(),
            sigfas_parser(hoon.clone()).boxed(),
            sigcab_parser(hoon.clone()).boxed(),
            cencab_parser(hoon.clone()).boxed(),
            cenlus_parser(hoon.clone()).boxed(),
            cenhep_parser(hoon.clone()).boxed(),
            centis_parser(hoon.clone()).boxed(),
            zapdot_parser(hoon.clone()).boxed(),
        ];

    choice(parsers).labelled("hoon-tall")
}

fn parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    recursive(|hoon| {
        let mut spec_wide_handle = Recursive::declare();
        let mut hoon_wide_handle = Recursive::declare();

        let spec = recursive(|spec| {
            choice((
                buccen_parser(spec.clone()),
                bucwut_spec_parser(spec.clone()),
                cenhep_spec_parser(hoon.clone(), spec.clone()),
                spec_wide_handle.clone(),
            ))
            .boxed()
        }).labelled("spec");

        let spec_wide_body = recursive(|spec_wide_self| {
            spec_wide_parser_fn(spec_wide_self.clone(), hoon_wide_handle.clone())
                .boxed()
        });

        let hoon_wide_body = hoon_wide_parser_fn(
            hoon_wide_handle.clone(),
            spec_wide_handle.clone(),
        );

        spec_wide_handle.define(spec_wide_body);
        let spec_wide = spec_wide_handle.clone();
        hoon_wide_handle.define(hoon_wide_body);
        let hoon_wide = hoon_wide_handle.clone();

        gap_parser().or_not()
         .ignore_then(choice((
            hoon_tall_parser(hoon.clone(), spec.clone(), spec_wide),
            hoon_wide
        )))
        .then_ignore(gap_parser().or_not())
        .boxed()
    })
    .boxed()
}

fn main() {
    let source = fs::read_to_string("./src/hoon.hoon").expect("Failed to read file");
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