use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use std::collections::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};

pub fn ket_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:      impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
            just(Token::Bar).ignore_then(ketbar(hoon.clone())),
            just(Token::Hep).ignore_then(kethep(hoon.clone(), spec.clone())),
            just(Token::Wut).ignore_then(ketwut(hoon.clone())),
            just(Token::Lus).ignore_then(ketlus(hoon.clone())),
            just(Token::Tis).ignore_then(kettis(hoon.clone())),
            just(Token::Dot).ignore_then(ketdot(hoon.clone())),
    ))
}

pub fn ket_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Sig).ignore_then(ketsig_wide(hoon_wide.clone())),
        just(Token::Lus).ignore_then(ketlus_wide(hoon_wide.clone())),
        just(Token::Dot).ignore_then(ketdot_wide(hoon_wide.clone())),
        just(Token::Hep).ignore_then(kethep_wide(hoon_wide.clone(), spec_wide.clone())),
    ))
}

pub fn ketdot<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::KetDot(Box::new(p), Box::new(q)))
}

pub fn ketdot_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::KetDot(Box::new(p), Box::new(q)))
}

pub fn ketbar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::KetBar(Box::new(p)))
}

pub fn ketwut<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::KetWut(Box::new(p)))
}

pub fn kettis<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::KetTis(Box::new(p), Box::new(q)))
}


pub fn kethep<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(s, h)| {
        Hoon::KetHep(Box::new(s), Box::new(h))
    })
}

pub fn kethep_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(s, h)| {
        Hoon::KetHep(Box::new(s), Box::new(h))})
}

pub fn ketsig_wide<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|h| Hoon::KetSig(Box::new(h)))
}

pub fn ketlus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::KetLus(Box::new(p), Box::new(q)))
}

pub fn ketlus_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::KetLus(Box::new(p), Box::new(q)))
}


pub fn kettar_irregular<'tokens, 'src: 'tokens, I>(
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

pub fn kethep_irregular<'tokens, 'src: 'tokens, I>(
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

pub fn ketcol_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Com)
        .ignore_then(spec_wide.clone())
        .map(|p| Hoon::KetCol(Box::new(p)))
}