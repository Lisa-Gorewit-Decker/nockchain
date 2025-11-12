use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use std::collections::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};

pub fn ket_runes_tall<'src>(
    hoon:      impl ParserExt<'src, Hoon>,
    spec:      impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    choice((
            just("|").ignore_then(ketbar(hoon.clone())),
            just(".").ignore_then(ketdot(hoon.clone())),
            just("-").ignore_then(kethep(hoon.clone(), spec.clone())),
            just("+").ignore_then(ketlus(hoon.clone())),
            just("&").ignore_then(ketpam(hoon.clone())),
            just('~').ignore_then(ketsig(hoon.clone())),
            just("=").ignore_then(kettis(hoon.clone())),
            just('?').ignore_then(ketwut(hoon.clone())),
    ))
}

pub fn ket_runes_wide<'src>(
    hoon_wide: impl ParserExt<'src, Hoon>,
    spec_wide: impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    choice((
        just('~').ignore_then(ketsig_wide(hoon_wide.clone())),
        just("+").ignore_then(ketlus_wide(hoon_wide.clone())),
        just(".").ignore_then(ketdot_wide(hoon_wide.clone())),
        just("-").ignore_then(kethep_wide(hoon_wide.clone(), spec_wide.clone())),
        just("|").ignore_then(ketbar_wide(hoon_wide.clone())),
        just("&").ignore_then(ketpam_wide(hoon_wide.clone())),
        just("=").ignore_then(kettis_wide(hoon_wide.clone())),
        just('?').ignore_then(ketwut_wide(hoon_wide.clone())),
    ))
}

pub fn ketdot<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::KetDot(Box::new(p), Box::new(q)))
}

pub fn ketdot_wide<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
    .then_ignore(just(" "))
    .then(hoon_wide.clone())
    .delimited_by(just('('), just(')'))
    .map(|(p, q)| Hoon::KetDot(Box::new(p), Box::new(q)))
}

pub fn ketbar<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::KetBar(Box::new(p)))
}

pub fn ketbar_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
    .delimited_by(just('('), just(')'))
    .map(|p| Hoon::KetBar(Box::new(p)))
}

pub fn ketpam<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::KetPam(Box::new(p)))
}

pub fn ketpam_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
    .delimited_by(just('('), just(')'))
    .map(|p| Hoon::KetPam(Box::new(p)))
}

pub fn ketsig<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::KetSig(Box::new(p)))
}

pub fn ketwut<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::KetWut(Box::new(p)))
}

pub fn ketwut_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
    .delimited_by(just('('), just(')'))
    .map(|p| Hoon::KetWut(Box::new(p)))
}

pub fn kettis<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::KetTis(Box::new(p), Box::new(q)))
}

pub fn kettis_wide<'src>(
    hoon_wide:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
    .then_ignore(just(" "))
    .then(hoon_wide.clone())
    .delimited_by(just('('), just(')'))
    .map(|(p, q)| Hoon::KetTis(Box::new(p), Box::new(q)))
}

pub fn kethep<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>

{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(s, h)| {
        Hoon::KetHep(Box::new(s), Box::new(h))
    })
}

pub fn kethep_wide<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>

{
    spec.clone()
    .then_ignore(just(" "))
    .then(hoon.clone())
    .delimited_by(just('('), just(')'))
    .map(|(s, h)| {
        Hoon::KetHep(Box::new(s), Box::new(h))})
}

pub fn ketsig_wide<'src>(
    hoon:      impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>

{
    hoon.clone()
    .delimited_by(just('('), just(')'))
    .map(|h| Hoon::KetSig(Box::new(h)))
}

pub fn ketlus<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::KetLus(Box::new(p), Box::new(q)))
}

pub fn ketlus_wide<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    hoon_wide.clone()
    .then_ignore(just(" "))
    .then(hoon_wide.clone())
    .delimited_by(just('('), just(')'))
    .map(|(p, q)| Hoon::KetLus(Box::new(p), Box::new(q)))
}


pub fn kettar_irregular<'src>(
    // hoon_wide:   impl ParserExt<'src, Hoon>,
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just('*')
        .ignore_then(spec_wide.clone())
        .map(|s| Hoon::KetTar(Box::new(s)))
}

pub fn kethep_irregular<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just("`")
        .ignore_then(spec_wide.clone())
        .then_ignore(just("`"))
        .then(hoon_wide.clone())
        .map(|(s, w)| Hoon::KetHep(Box::new(s), Box::new(w)))
}

pub fn ketcol_irregular<'src>(
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    just(",")
        .ignore_then(spec_wide.clone())
        .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn kettar<'src>(
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    one_spec_closed_tall(spec.clone())
    .map(|s| Hoon::KetTar(Box::new(s)))
}

pub fn kettar_wide<'src>(
    spec_wide:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    one_spec_closed_wide(spec_wide.clone())
    .map(|s| Hoon::KetTar(Box::new(s)))
}

pub fn ketcol<'src>(
    spec:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    one_spec_closed_tall(spec.clone())
    .map(|s| Hoon::KetCol(Box::new(s)))
}

pub fn ketcol_wide<'src>(
    spec_wide:        impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    one_spec_closed_wide(spec_wide.clone())
    .map(|s| Hoon::KetCol(Box::new(s)))
}