use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::collections::*;

pub fn zap_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:      impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Col).ignore_then(zapcol(hoon.clone())),
        just(Token::Dot).ignore_then(zapdot(hoon.clone())),
        just(Token::Com).ignore_then(zapcom(hoon.clone())),
        just(Token::Mic).ignore_then(zapmic(hoon.clone())),
        just(Token::Gar).ignore_then(zapgar(hoon.clone())),
        just(Token::Gal).ignore_then(zapgal(hoon.clone(), spec.clone())),
        just(Token::Pat).ignore_then(zappat(hoon.clone())),
        just(Token::Tis).ignore_then(zaptis(hoon.clone())),
        just(Token::Wut).ignore_then(zapwut(hoon.clone())),
        just(Token::Zap).to(Hoon::ZapZap),
    ))
}

pub fn zap_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Col).ignore_then(zapcol_wide(hoon_wide.clone())),
        just(Token::Dot).ignore_then(zapdot_wide(hoon_wide.clone())),
        just(Token::Com).ignore_then(zapcom_wide(hoon_wide.clone())),
        just(Token::Mic).ignore_then(zapmic_wide(hoon_wide.clone())),
        just(Token::Gar).ignore_then(zapgar_wide(hoon_wide.clone())),
        just(Token::Gal).ignore_then(zapgal_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Pat).ignore_then(zappat_wide(hoon_wide.clone())),
        just(Token::Tis).ignore_then(zaptis_wide(hoon_wide.clone())),
        just(Token::Wut).ignore_then(zapwut_wide(hoon_wide.clone())),
        just(Token::Zap).to(Hoon::ZapZap),
    ))
}

pub fn zapcom<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_tall(hoon.clone())
    .map(|(p, q)| Hoon::ZapCom(Box::new(p), Box::new(q)))
}

pub fn zapcom_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::ZapCom(Box::new(p), Box::new(q)))
}

pub fn zappat<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist()
        .separated_by(just(Token::Com))
        .at_least(1)
        .collect::<Vec<_>>()
    .then(two_hoons_tall(hoon.clone()))
    .map(|(list, (p, q))| Hoon::ZapPat(list, Box::new(p), Box::new(q)))
}

pub fn zappat_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist()
        .separated_by(just(Token::Com))
        .at_least(1)
        .collect::<Vec<_>>()
    .then_ignore(just(Token::Ace))
    .then(two_hoons_wide(hoon_wide.clone()))
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(list, (p, q))| Hoon::ZapPat(list, Box::new(p), Box::new(q)))
}

pub fn zapmic<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_tall(hoon.clone())
    .map(|(p, q)| Hoon::ZapMic(Box::new(p), Box::new(q)))
}

pub fn zapmic_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::ZapMic(Box::new(p), Box::new(q)))
}

pub fn zapdot<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()  // TODO: this needs to disable tracing..
    .ignore_then(hoon.clone())
}

pub fn zapdot_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()   // TODO: this needs to disable tracing..
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn zaptis<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|h| Hoon::ZapTis(Box::new(h)))
}

pub fn zaptis_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|h| Hoon::ZapTis(Box::new(h)))
}

pub fn zapgar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|h| Hoon::ZapGar(Box::new(h)))
}

pub fn zapgar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|h| Hoon::ZapGar(Box::new(h)))
}

pub fn zapgal<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::ZapGal(Box::new(p), Box::new(q)))
}

pub fn zapgal_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::ZapGal(Box::new(p), Box::new(q)))
}

pub fn zapcol<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()   // TODO: this needs to enable tracing...
    .ignore_then(hoon.clone())
}

pub fn zapcol_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()   // TODO: this needs to enable tracing..
    .delimited_by(just(Token::Pal), just(Token::Par))
}

pub fn zapwut<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(select! { Token::Number(n) => ZpwtArg::Atom(n.to_string()) }
                .or(
                    select! { Token::Number(n) => n.to_string() }
                    .then(select! { Token::Number(n) => n.to_string() })
                    .delimited_by(just(Token::Sel), just(Token::Ser))
                    .map(|(s1, s2)| ZpwtArg::Pair(s1, s2))
                ).map(|p| p)
            )
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::ZapWut(p, Box::new(q)))
}

pub fn zapwut_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Number(n) => ZpwtArg::Atom(n.to_string()) }
                .or(
                    select! { Token::Number(n) => n.to_string() }
                    .then(select! { Token::Number(n) => n.to_string() })
                    .delimited_by(just(Token::Sel), just(Token::Ser))
                    .map(|(s1, s2)| ZpwtArg::Pair(s1, s2))
                ).map(|p| p)
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::ZapWut(p, Box::new(q)))
}