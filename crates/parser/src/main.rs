use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};

use std::fs;
use std::collections::HashMap;
use std::time::Instant;
use std::io::Write;
use std::path::PathBuf;

use logos::Logos;
use clap::{Parser as ClapParser, command, arg};

use parser::lexer::tokens::Token;
use parser::ast::hoon::*;
use parser::utils::*;
use parser::runes::*;

fn tape<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Tape(s) => Hoon::Knit(s.to_string()) }
}

fn aura_hoon<'tokens, 'src: 'tokens, I>(
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

fn aura_spec<'tokens, 'src: 'tokens, I>(
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

fn dottar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::DotTar)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(s, h)| Hoon::DotTar(Box::new(s), Box::new(h)))
}

fn dottar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::DotTarWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::DotTar(Box::new(p), Box::new(q)))
}

fn path<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Fas)
        .to(Hoon::ColSig(vec![]))
}

fn dotwut_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::DotWutWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|p| Hoon::DotWut(Box::new(p)))
}


fn dottis_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
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

fn concatanate<'tokens, 'src: 'tokens, I>(
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

fn wing<'tokens, 'src: 'tokens, I>(
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

fn tell<'tokens, 'src: 'tokens, I>(
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

fn spec_term<'tokens, 'src: 'tokens, I>(
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

fn constant<'tokens, 'src: 'tokens, I>(
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

fn bucbar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucBar)
        .ignore_then(gap())
        .ignore_then(spec.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Spec::BucBar(Box::new(p), q))
}

fn buccab_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cab)
        .ignore_then(hoon_wide.clone())
        .map(|h| Hoon::KetCol(Box::new(Spec::BucCab(h))))
}

fn buccab_spec_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cab)
        .ignore_then(hoon_wide.clone())
        .map(|h| Spec::BucCab(h))
}

fn bucmic_spec_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Com)
        .ignore_then(hoon_wide.clone())
        .map(|h| Spec::BucMic(h))
}

fn bucket<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucKet)
        .ignore_then(gap())
        .ignore_then(spec.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucKet(Box::new(p), Box::new(q)))))
}

fn bucket_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucKet)
        .ignore_then(gap())
        .ignore_then(spec.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Spec::BucKet(Box::new(p), Box::new(q)))
}

fn buclus<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Buc, Token::Lus])
        .ignore_then(gap())
        .ignore_then(select! { Token::Name(s) => s.to_string() })
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucLus(p, Box::new(q)))))
}

fn buclus_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Buc, Token::Lus])
        .ignore_then(gap())
        .ignore_then(select! { Token::Name(s) => s.to_string() })
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Spec::BucLus(p, Box::new(q)))
}

fn bucwut_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Wut, Token::Pal])
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

fn bucwut<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucWut)
        .ignore_then(gap())
        .ignore_then(spec_wide.clone()
              .separated_by(gap())
              .at_least(1)
              .collect::<Vec<_>>()
            )
        .then_ignore(gap())
        .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Hoon::KetCol(Box::new(
                        Spec::BucWut(Box::new(first.clone()),
                                      rest.to_vec())
            ))
        })
}

fn bucwut_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucWut)
        .ignore_then(spec_wide.clone()
              .separated_by(just(Token::Ace))
              .at_least(1)
              .collect::<Vec<_>>()
            )
        .then_ignore(just(Token::Par))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Hoon::KetCol(Box::new(
                        Spec::BucWut(Box::new(first.clone()),
                                      rest.to_vec())
            ))
        })
}

fn buctis_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucTis)
        .ignore_then(gap())
        .ignore_then(select! { Token::Name(n) => n.to_string() })
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(name, s)| { Spec::BucTis(Skin::Term(name), Box::new(s))})
}

fn bucwut_spec<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucWut)
        .ignore_then(gap())
        .ignore_then(spec_wide.clone()
              .separated_by(gap())
              .at_least(1)
              .collect::<Vec<_>>()
            )
        .then_ignore(gap())
        .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Spec::BucWut(Box::new(first.clone()), rest.to_vec())
        })
}

fn bucwut_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucWutWide)
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

fn buctis_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
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
                                // None => Err(Cheap::new(span).into()),
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

fn buccol_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
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


fn zapdot<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ZapDot)   // TODO: this needs to disable tracing..
        .ignore_then(gap())
        .ignore_then(hoon.clone())
}

fn zapgar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ZapGar)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .map(|h| Hoon::ZapGar(Box::new(h)))
}

fn zapgar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ZapGarWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|h| Hoon::ZapGar(Box::new(h)))
}

fn zapcol<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ZapCol)   // TODO: this needs to enable tracing...
        .ignore_then(gap())
        .ignore_then(hoon.clone())
}

fn ketdot<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetDot)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::KetDot(Box::new(p), Box::new(q)))
}

fn ketdot_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
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

fn ketbar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetBar)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .map(|p| Hoon::KetBar(Box::new(p)))
}

fn ketwut<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetWut)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .map(|p| Hoon::KetWut(Box::new(p)))
}

fn kettis<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetTis)
        .then_ignore(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::KetTis(Box::new(p), Box::new(q)))
}

fn jet_hooks<'tokens, 'src: 'tokens, I>(
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

fn collus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Col).ignore_then(just(Token::Lus))
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::ColLus(Box::new(p), Box::new(q), Box::new(r)))
}

fn colhep<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Col).ignore_then(just(Token::Hep))
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::ColHep(Box::new(p), Box::new(q)))
}

fn colcab<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ColCab)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::ColCab(Box::new(p), Box::new(q)))
}

fn colcab_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ColCabWide)
        .ignore_then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::ColCab(Box::new(p), Box::new(q)))
}

fn colket<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ColKet)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(((p, q), s), r)|
                    Hoon::ColKet(Box::new(p),
                                 Box::new(q),
                                 Box::new(s),
                                 Box::new(r)))
}

fn colket_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ColKetWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(((p, q), s), r)|
                    Hoon::CenKet(Box::new(p),
                                 Box::new(q),
                                 Box::new(s),
                                 Box::new(r)))
}

fn cenlus_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Cen, Token::Lus])
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|((p, q), r)| Spec::Make(p, vec![q, r]))
}

fn cenhep_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Cen, Token::Hep])
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Spec::Make(p, vec![q]))
}

fn jet_signature<'tokens, 'src: 'tokens, I>(
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

fn sigpam<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigPam)
        .ignore_then(gap())
        .ignore_then(just(Token::Gar)
                .repeated()
                .at_most(3)
                .count()
                .then_ignore(gap())
                .or_not()
            )
        .then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|((maybe_p, q), r)| {
            let p = maybe_p.unwrap_or(0);
            Hoon::SigPam(p as u64, Box::new(q), Box::new(r))
        })
}

fn sigwut<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigWut)
        .ignore_then(gap())
        .ignore_then(just(Token::Gar)
                .repeated()
                .at_most(3)
                .count()
                .then_ignore(gap())
                .or_not()
            )
        .then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(((maybe_p, q), r), s)| {
            let p = maybe_p.unwrap_or(0);
            Hoon::SigWut(p as u64, Box::new(q), Box::new(r), Box::new(s))
        })
}

fn sigpam_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigPamWide)
        .ignore_then(just(Token::Gar)
                .repeated()
                .at_most(3)
                .count()
                .then_ignore(just(Token::Ace))
                .or_not()
            )
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|((maybe_p, q), r)| {
            let p = maybe_p.unwrap_or(0);
            Hoon::SigPam(p as u64, Box::new(q), Box::new(r))
        })
}

fn sigzap<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigZap)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::SigZap(Box::new(p), Box::new(q)))
}

fn sigbar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigBar)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::SigBar(Box::new(p), Box::new(q)))
}

fn sigbar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigBarWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::SigBar(Box::new(p), Box::new(q)))
}

fn siglus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigLus)             //  the hoon parser accepts an optional first arg
        .ignore_then(gap())  //  here, but its never used anywhere, and idk what is...
        .ignore_then(hoon.clone())
        .map(|p| Hoon::SigLus(0, Box::new(p)))
}

fn siglus_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigLusWide)         //  the hoon parser accepts an optional first arg
        .ignore_then(hoon_wide.clone())   //  here, but its never used anywhere, and idk what is...
        .then_ignore(just(Token::Par))
        .map(|p| Hoon::SigLus(0, Box::new(p)))
}

fn sigcab<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigCab)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::SigCab(Box::new(p), Box::new(q)))
}

fn sigcab_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigCabWide)
        .ignore_then(hoon.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|(p, q)| Hoon::SigCab(Box::new(p), Box::new(q)))
}

fn sigcen<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigCen)
        .ignore_then(gap())
        .ignore_then(jet_signature())
        .then_ignore(gap())
        .then(hoon.clone())
        .then_ignore(gap())
        .then(jet_hooks(hoon.clone()))
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(((p, q), r), s)| Hoon::SigCen(p, Box::new(q), r, Box::new(s)))
}

fn sigfas<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigFas)
        .ignore_then(gap())
        .ignore_then(jet_signature())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::SigFas(p, Box::new(q)))
}

fn cord<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {Token::Cord(s) => Hoon::Sand("%t".to_string(), Noun::Atom(s.to_string()))}
}

fn term<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, String, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cen)
      .ignore_then(select! {Token::Name(s) => format!("%{}", s) })
}

fn siggar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigGarWide)
        .ignore_then(term())
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

fn siggar<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigGar)
        .ignore_then(gap())
        .ignore_then(term())
        .then(just(Token::Dot)
             .ignore_then(hoon_wide.clone())
             .or_not())
        .then_ignore(gap())
        .then(hoon_wide.clone())
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

fn siggal<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::SigGal)
        .ignore_then(gap())
        .ignore_then(term())
        .then(just(Token::Dot)
             .ignore_then(hoon_wide.clone())
             .or_not())
        .then_ignore(gap())
        .then(hoon_wide.clone())
        .map(|((term, maybe_hoon), q)|  {
            match maybe_hoon {
                None =>{
                    Hoon::SigGal(TermOrPair::Term(term), Box::new(q))
                }
                Some(h) => {
                    Hoon::SigGal(TermOrPair::Pair((term, Box::new(h))), Box::new(q))
                }
            }
        })
}

fn increment<'tokens, 'src: 'tokens, I>(
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

fn micsig<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::MicSig)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(list_hoon_tall(hoon.clone()))
        .then_ignore(gap())
        .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|(func, args)| Hoon::MicSig(Box::new(func), args))
}

fn micsig_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::MicSigWide)
        .ignore_then(hoon.clone())
        .then(
            just(Token::Ace)
                .ignore_then(hoon.clone())
                .repeated()
                .collect::<Vec<_>>()
            )
    .then_ignore(just(Token::Par))
    .map(|(func, args)| Hoon::MicSig(Box::new(func), args))
}

fn micmic_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::MicMicWide)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
    .map(|(s, h)| Hoon::MicMic(Box::new(s), Box::new(h)))
}

fn micfas_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::MicFasWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Par))
    .map(|(h)| Hoon::MicFas(Box::new(h)))
}

fn function_call<'tokens, 'src: 'tokens, I>(
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

fn kethep<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetHep)
        .ignore_then(gap())
        .ignore_then(spec.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(s, h)| {
            Hoon::KetHep(Box::new(s), Box::new(h))
        })
}

fn kethep_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetHepWide)
        .ignore_then(spec.clone())
        .then_ignore(just(Token::Ace))
        .then(hoon.clone())
        .then_ignore(just(Token::Par))
        .map(|(s, h)| {
            Hoon::KetHep(Box::new(s), Box::new(h))})
}

fn ketsig_wide<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetSigWide).
        ignore_then(hoon.clone()).
        then_ignore(just(Token::Par)).
        map(|h| Hoon::KetSig(Box::new(h)))
}

fn bucsig_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
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

fn bucsig_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucSigWide)
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(spec_wide.clone())
        .then_ignore(just(Token::Par))
        .map(|(h, s)| Spec::BucSig(h, Box::new(s)))
}

fn number<'tokens, 'src: 'tokens, I>(
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
fn constant_separator_hoon<'tokens, 'src: 'tokens, I>(
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

fn buclus_wide<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
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

fn buchep_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
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

fn bucpat<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucPat)
        .ignore_then(gap())
        .ignore_then(spec.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| {
                Hoon::KetCol(Box::new(Spec::BucPat(
                                         Box::new(p),
                                         Box::new(q))))
            })
}

fn bucpat_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucPat)
        .ignore_then(gap())
        .ignore_then(spec.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Spec::BucPat(Box::new(p), Box::new(q)))
}

fn buccol<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Buc).ignore_then(just(Token::Col))
        .ignore_then(gap())
        .ignore_then(spec.clone()
                    .separated_by(gap())
                    .at_least(1)
                    .collect::<Vec<_>>()
            )
        .then_ignore(gap())
            .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|specs| {
                let (first, rest) = specs.split_first().unwrap();
                Hoon::KetCol(Box::new(Spec::BucCol(
                                Box::new(first.clone()), rest.to_vec())))
            })
}

fn buccol_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Buc).ignore_then(just(Token::Col))
        .ignore_then(gap())
        .ignore_then(spec.clone()
                    .separated_by(gap())
                    .at_least(1)
                    .collect::<Vec<_>>()
            )
        .then_ignore(gap())
            .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|specs| {
                let (first, rest) = specs.split_first().unwrap();
                Spec::BucCol(Box::new(first.clone()), rest.to_vec())
            })
}

fn bucsig<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucSig)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucSig(p, Box::new(q)))))
}

fn bucsig_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucSig)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(spec.clone())
        .map(|(p, q)| Spec::BucSig(p, Box::new(q)))
}

fn buccen<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucCen)
        .ignore_then(gap())
        .ignore_then(spec.clone()
                    .separated_by(gap())
                    .at_least(1)
                    .collect::<Vec<_>>()
            )
        .then_ignore(gap())
            .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|specs| {
                let (first, rest) = specs.split_first().unwrap();
                Hoon::KetCol(Box::new(Spec::BucCen(
                                Box::new(first.clone()), rest.to_vec())))
            })
}

fn buccen_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucCenWide)
        .ignore_then(spec_wide.clone()
                    .separated_by(just(Token::Ace))
                    .at_least(1)
                    .collect::<Vec<_>>()
            )
        .then_ignore(just(Token::Par))
        .map(|specs| {
                let (first, rest) = specs.split_first().unwrap();
                Hoon::KetCol(Box::new(Spec::BucCen(
                                Box::new(first.clone()), rest.to_vec())))
            })
}

fn buccen_wide_spec<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucCenWide)
        .ignore_then(spec_wide.clone()
                    .separated_by(just(Token::Ace))
                    .at_least(1)
                    .collect::<Vec<_>>()
            )
        .then_ignore(just(Token::Par))
        .map(|specs| {
                let (first, rest) = specs.split_first().unwrap();
                Spec::BucCen(
                                Box::new(first.clone()), rest.to_vec())
            })
}

fn buccen_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BucCen)
        .ignore_then(gap())
        .ignore_then(spec.clone()
                    .separated_by(gap())
                    .at_least(1)
                    .collect::<Vec<_>>()
            )
        .then_ignore(gap())
            .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|specs| {
                let (first, rest) = specs.split_first().unwrap();
                Spec::BucCen(Box::new(first.clone()), rest.to_vec())
            })
}

fn bucpat_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
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

fn ketlus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::KetLus)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::KetLus(Box::new(p), Box::new(q)))
}

fn ketlus_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
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

fn bucpat_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
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

fn list_syntax<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
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

fn tic_cell_construction<'tokens, 'src: 'tokens, I>(
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

fn kettar_irregular<'tokens, 'src: 'tokens, I>(
    // hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Tar)
        .ignore_then(spec_wide.clone())
        .map(|s| Hoon::KetTar(Box::new(s)))
}

fn kethep_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Tic)
        .ignore_then(spec_wide.clone())
        .then_ignore(just(Token::Tic))
        .then(hoon_wide.clone())
        .map(|(s, w)| Hoon::KetHep(Box::new(s), Box::new(w)))
}

fn list_hoon_tall<'tokens, 'src: 'tokens, I>(
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

fn coltar<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ColTar)
        .ignore_then(gap())
        .ignore_then(list_hoon_tall(hoon.clone()))
        .then_ignore(gap())
        .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|list| Hoon::ColTar(list))
}

fn colsig<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ColSig)
        .ignore_then(gap())
        .ignore_then(list_hoon_tall(hoon.clone()))
        .then_ignore(gap())
        .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|list| Hoon::ColSig(list))
}

fn miccol<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::MicCol)
        .ignore_then(gap())
        .ignore_then(hoon.clone())
        .then_ignore(gap())
        .then(list_hoon_tall(hoon.clone()))
        .then_ignore(gap())
        .then_ignore(just([Token::Tis, Token::Tis]))
        .map(|(p, list)| Hoon::MicCol(Box::new(p), list))
}

fn miccol_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Col)
        .ignore_then(just(Token::Pal))
        .ignore_then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(list_hoon_wide(hoon_wide.clone()))
        .then_ignore(just(Token::Par))
        .map(|(p, list)| Hoon::MicCol(Box::new(p), list))
}

fn ketcol_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Com)
        .ignore_then(spec_wide.clone())
        .map(|p| Hoon::KetCol(Box::new(p)))
}

fn parenthesis_spec<'tokens, 'src: 'tokens, I>(
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

fn reference_spec<'tokens, 'src: 'tokens, I>(
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

fn spec_parser<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        buccen_spec(spec.clone()),
        bucbar(hoon.clone(), spec.clone()),
        bucpat_spec(spec.clone()).boxed(),
        bucwut_spec(spec.clone()),
        buctis_spec(spec.clone()),
        buclus_spec(spec.clone()),
        bucket_spec(spec.clone()),
        buccol_spec(spec.clone()),
        bucsig_spec(hoon.clone(), spec.clone()).boxed(),
        cenhep_spec(hoon.clone(), spec.clone()),
        cenlus_spec(hoon.clone(), spec.clone()),
        spec_wide.clone(),
    )).boxed()
}

fn spec_wide_parser<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let parsers = vec![
        bucpat_spec_wide(spec_wide.clone()).boxed(),  // $%(foo bar)
        bucsig_spec_wide(spec_wide.clone(),
                                hoon_wide.clone()).boxed(),
        buchep_wide(spec_wide.clone()).boxed(),       // $-(foo bar)
        buclus_wide(spec_wide.clone()).boxed(),       // $+(foo bar)
        bucwut_spec_wide(spec_wide.clone()).boxed(),
        buccen_wide_spec(spec_wide.clone()).boxed(),
        buccab_spec_irregular(hoon_wide.clone()).boxed(),  //  _p
        bucmic_spec_irregular(hoon_wide.clone()).boxed(),  //  ,p
        buctis_irregular(spec_wide.clone()).boxed(),  // foo=bar, =bar,  =foo=bar
        buccol_irregular(spec_wide.clone()).boxed(),  // [foo=bar foo=bar]
        reference_spec(spec_wide.clone()).boxed(),    // foo or foo:bar
        bucwut_irregular(spec_wide.clone()).boxed(),  // ?(foo bar)
        parenthesis_spec(hoon_wide.clone(),
                                spec_wide.clone()).boxed(),  // (foo bar)
        just(Token::Ket).to(Spec::Base(BaseType::Cell)).boxed(),
        just(Token::Wut).to(Spec::Base(BaseType::Flag)).boxed(),
        just(Token::Sig).to(Spec::Base(BaseType::Null)).boxed(),
        just(Token::Tar).to(Spec::Base(BaseType::Noun)).boxed(),
        just([Token::Cen, Token::Sig]).to(Spec::Leaf("%n".to_string(), "0".to_string())).boxed(),
        just([Token::Cen, Token::Bar]).to(Spec::Leaf("%f".to_string(), "1".to_string())).boxed(),
        just([Token::Cen, Token::Pam]).to(Spec::Leaf("%f".to_string(), "0".to_string())).boxed(),
        aura_spec().boxed(), //  @foo
        spec_term().boxed(), // %$, %foo, %123
    ];

    choice(parsers).labelled("spec-wide")
}

fn hoon_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let parsers = vec![
        just(Token::Bar)
                .ignore_then(bar_runes_wide(hoon_wide.clone(),
                                             spec_wide.clone())).boxed(),
        just(Token::Tis)
                .ignore_then(tis_runes_wide(hoon_wide.clone(),
                                             spec_wide.clone())).boxed(),
        just(Token::Wut)
                .ignore_then(wut_runes_wide(hoon_wide.clone(),
                                             spec_wide.clone())).boxed(),
        just(Token::Cen)
                .ignore_then(
                        cen_runes_wide(hoon_wide.clone()),
                        ).boxed(),
        ketsig_wide(hoon_wide.clone()).boxed(),
        bucsig_wide(hoon_wide.clone(), spec_wide.clone()).boxed(),
        bucpat_wide(hoon_wide.clone(), spec_wide.clone()).boxed(),
        buccen_wide(spec_wide.clone()).boxed(),
        bucwut_wide(spec_wide.clone()).boxed(),
        colket_wide(hoon_wide.clone()).boxed(),
        dotwut_wide(hoon_wide.clone()).boxed(),
        micsig_wide(hoon_wide.clone()).boxed(),
        siggar_wide(hoon_wide.clone()).boxed(),
        siglus_wide(hoon_wide.clone()).boxed(),
        sigcab_wide(hoon_wide.clone()).boxed(),
        sigbar_wide(hoon_wide.clone()).boxed(),
        sigpam_wide(hoon_wide.clone()).boxed(),
        ketlus_wide(hoon_wide.clone()).boxed(),
        ketdot_wide(hoon_wide.clone()).boxed(),
        kethep_wide(hoon_wide.clone(), spec_wide.clone()).boxed(),
        colcab_wide(hoon_wide.clone()).boxed(),
        dottar_wide(hoon_wide.clone()).boxed(),
        micmic_wide(hoon_wide.clone(), spec_wide.clone()).boxed(),
        micfas_wide(hoon_wide.clone()).boxed(),
        zapgar_wide(hoon_wide.clone()).boxed(),
        tape().boxed(),
        path(hoon_wide.clone()).boxed(),
        buccab_irregular(hoon_wide.clone()).boxed(),              //  _p
        miccol_irregular(hoon_wide.clone()).boxed(),              //  :(a b .. z)
        censig_irregular(hoon_wide.clone()).boxed(),              //  ~(a b c)
        constant_separator_hoon(hoon_wide.clone()).boxed(),           //  const+hoon,  const/hoon
        dottis_irregular(hoon_wide.clone()).boxed(),              //  =(p q)
        list_syntax(hoon_wide.clone()).boxed(),                   // [p ... pn], ~[foo], [foo]~
        kettar_irregular(spec_wide.clone()).boxed(),              //  *foo
        kethep_irregular(hoon_wide.clone(),
                                spec_wide.clone()).boxed(),              //  `p`q
        tic_cell_construction(hoon_wide.clone()).boxed(),         //  `a
        wutzap_irregular(hoon_wide.clone()).boxed(),              //  !p
        wutbar_irregular(hoon_wide.clone()).boxed(),              //  |(p q)
        wutpam_irregular(hoon_wide.clone()).boxed(),              //  &(p q)
        increment(hoon_wide.clone()).boxed(),          //  +(a) or .+(a)
        ketcol_irregular(spec_wide.clone()).boxed(),   //  ,p
        centis_irregular(hoon_wide.clone()).boxed(),   //  a(b c, d e, f g)
        tell(hoon_wide.clone()).boxed(),  // <foo>
        wing().boxed(),
        function_call(hoon_wide.clone()).boxed(),      //  (a b)
        aura_hoon().boxed(),
        constant().boxed(),
        cord().boxed(),
        number().boxed(),
        select! { Token::Date(d) => Hoon::Sand("%da".to_string(), Noun::Atom(d.to_string()))}.boxed(),
        just(Token::Yes).to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::No).to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just([Token::Cen, Token::Bar]).to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just([Token::Cen, Token::Pam]).to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::Pam).to(Hoon::Sand("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::Bar).to(Hoon::Sand("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just(Token::Tar).to(Hoon::Base(BaseType::Noun)).boxed(),
        just(Token::Wut).to(Hoon::Base(BaseType::Flag)).boxed(),
        just(Token::Sig).to(Hoon::Bust(BaseType::Null)).boxed(),
        just(Token::ZapZap).to(Hoon::ZapZap).boxed(),
    ];

    choice(parsers).labelled("hoon-wide")
        .then(just(Token::Tis).or(just(Token::Col)).or(just(Token::Ket))
                .then(hoon_wide.clone())
                .or_not())
        .map(|(p, maybe_separator)|  {
            match maybe_separator  {
                Some((Token::Tis, q)) => Hoon::KetTis(Box::new(p), Box::new(q)),
                Some((Token::Col, q)) => Hoon::TisGal(Box::new(p), Box::new(q)),
                Some((Token::Ket, q)) => Hoon::Pair(Box::new(p), Box::new(q)),
                _ => p,
            }
        })
}

fn hoon_tall_parser<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     let parsers = vec![
            just(Token::Bar)
                .ignore_then(
                    choice((
                        bar_runes_tall(hoon.clone(),
                                                spec.clone()),
                        // bar_runes_wide(hoon_wide.clone(),
                        //         spec_wide.clone()),
                    ))).boxed(),
            just(Token::Tis)
                .ignore_then(
                    choice((
                        tis_runes_tall(hoon.clone(),
                                       spec.clone(),
                                       spec_wide.clone()),
                        // tis_runes_wide(hoon_wide.clone(),
                                            //  spec_wide.clone()),
                    ))).boxed(),
            just(Token::Wut)
                .ignore_then(
                    choice((
                        wut_runes_tall(hoon.clone(),
                                       hoon_wide.clone(),
                                       spec.clone(),
                                       spec_wide.clone()),
                        // wut_runes_wide(hoon_wide.clone(),
                                            //  spec_wide.clone()),
                        ))).boxed(),
            just(Token::Cen)
                .ignore_then(
                        cen_runes_tall(hoon.clone()),
                        ).boxed(),
            bucwut(spec.clone()).boxed(),
            buccen(spec.clone()).boxed(),
            bucpat(spec.clone()).boxed(),
            buccol(spec.clone()).boxed(),
            buclus(spec.clone()).boxed(),
            bucket(spec.clone()).boxed(),
            dottar(hoon.clone()).boxed(),
            bucsig(hoon.clone(), spec.clone()).boxed(),
            ketbar(hoon.clone()).boxed(),
            kethep(hoon.clone(), spec.clone()).boxed(),
            ketwut(hoon.clone()).boxed(),
            ketlus(hoon.clone()).boxed(),
            kettis(hoon.clone()).boxed(),
            ketdot(hoon.clone()).boxed(),
            sigcen(hoon.clone()).boxed(),
            sigfas(hoon.clone()).boxed(),
            sigcab(hoon.clone()).boxed(),
            siglus(hoon.clone()).boxed(),
            sigzap(hoon.clone()).boxed(),
            sigbar(hoon.clone()).boxed(),
            siggar(hoon.clone()).boxed(),
            siggal(hoon.clone()).boxed(),
            sigpam(hoon.clone()).boxed(),
            sigwut(hoon.clone()).boxed(),
            colket(hoon.clone()).boxed(),
            colcab(hoon.clone()).boxed(),
            collus(hoon.clone()).boxed(),
            colhep(hoon.clone()).boxed(),
            coltar(hoon.clone()).boxed(),
            colsig(hoon.clone()).boxed(),
            miccol(hoon.clone()).boxed(),
            micsig(hoon.clone()).boxed(),
            zapdot(hoon.clone()).boxed(),
            zapcol(hoon.clone()).boxed(),
            zapgar(hoon.clone()).boxed(),
    ];
    choice((parsers))
    .labelled("hoon-tall")
}

fn parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let hoon = recursive(|hoon| {
        let mut spec_wide_handle = Recursive::declare();
        let mut hoon_wide_handle = Recursive::declare();

        let spec = recursive(|spec| {
            spec_parser(spec.clone(),
                                spec_wide_handle.clone(),
                                hoon.clone())
        }).labelled("spec");

        let spec_wide_body = recursive(|spec_wide_self| {
            spec_wide_parser(spec_wide_self.clone(), hoon_wide_handle.clone())
        });

        let hoon_wide_body = hoon_wide_parser(
            hoon_wide_handle.clone(),
            spec_wide_handle.clone(),
        );

        spec_wide_handle.define(spec_wide_body);
        hoon_wide_handle.define(hoon_wide_body);

        let spec_wide = spec_wide_handle.clone();
        let hoon_wide = hoon_wide_handle.clone();

        choice((
            hoon_tall_parser(hoon.clone(), hoon_wide.clone(), spec.clone(), spec_wide),
            hoon_wide
        ))
        .boxed()
    });

    hoon.padded_by(gap().or_not())
}

#[derive(ClapParser, Debug)]
#[command(author, version, about = "Parses a Hoon source file")]
struct Cli {
    /// Path to the input .hoon file
    #[arg(value_name = "FILE", help = "Input Hoon source file")]
    input: PathBuf,
}

fn main() {
    let cli = Cli::parse();

    let source = fs::read_to_string(&cli.input).unwrap_or_else(|err| {
        eprintln!("Error reading file '{}': {}", cli.input.display(), err);
        std::process::exit(1);
    });

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

    //  print tokens
    // for (tok, span) in token_iter.clone() {
    //     println!("{:?} {:?}", tok, span);
    // }

    let token_stream = Stream::from_iter(token_iter)
        // Tell chumsky to split the (Token, SimpleSpan) stream into its parts so that it can handle the spans for us
        // This involves giving chumsky an 'end of input' span: we just use a zero-width span at the end of the string
        .map((0..source.len()).into(), |(t, s): (_, _)| (t, s));

    match parser().parse(token_stream).into_result() {
        Ok(res) => {
           let took = start.elapsed();
           let json = serde_json::to_string_pretty(&res)
            .expect("serialisation failed");

            let out_path = std::path::PathBuf::from("out.json");

            std::fs::write(&out_path, json + "\n")
                .unwrap_or_else(|e| {
                    eprintln!("Failed to write '{}': {}", out_path.display(), e);
                    std::process::exit(1);
                });

            println!("Result written to {}!", out_path.display());
            println!("took: {:?}", took);
        }
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
