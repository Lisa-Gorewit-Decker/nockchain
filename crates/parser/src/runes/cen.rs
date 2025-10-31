use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::collections::*;

pub fn cen_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Ket).ignore_then(cenket(hoon.clone())),
        just(Token::Hep).ignore_then(cenhep(hoon.clone())),
        just(Token::Dot).ignore_then(cendot(hoon.clone())),
        just(Token::Sig).ignore_then(censig(hoon.clone())),
        just(Token::Lus).ignore_then(cenlus(hoon.clone())),
        just(Token::Cab).ignore_then(cencab(hoon.clone())),
        just(Token::Tis).ignore_then(centis(hoon.clone())),
    ))
}

pub fn cen_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Ket).ignore_then(cenket_wide(hoon_wide.clone())),
        just(Token::Tar).ignore_then(centar_wide(hoon_wide.clone())),
        just(Token::Tis).ignore_then(centis_wide(hoon_wide.clone())),
    ))
}

pub fn cen_spec_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
    // spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Hep).ignore_then(cenhep_spec(hoon.clone(), spec.clone())),
        just(Token::Lus).ignore_then(cenlus_spec(hoon.clone(), spec.clone())),
    ))
}

pub fn cenket<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(((p, q), s), r)|
                Hoon::CenKet(Box::new(p),
                                Box::new(q),
                                Box::new(s),
                                Box::new(r)))
}

pub fn cenket_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(((p, q), s), r)|
                Hoon::CenKet(Box::new(p),
                                Box::new(q),
                                Box::new(s),
                                Box::new(r)))
}

pub fn cenhep<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::CenHep(Box::new(p), Box::new(q)))
}

pub fn cendot<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::CenDot(Box::new(p), Box::new(q)))
}

pub fn cenlus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| Hoon::CenLus(Box::new(p), Box::new(q), Box::new(r)))
}

pub fn cencab<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(winglist())
    .then_ignore(gap())
    .then(list_wing_hoon_tall(hoon.clone()))
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|(p, q)| Hoon::CenCab(p, q))
}

pub fn centar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(list_wing_hoon_wide(hoon_wide.clone()))
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), list)| Hoon::CenTar(p, Box::new(q), list))
}

pub fn centis<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(winglist())
    .then_ignore(gap())
    .then(list_wing_hoon_tall(hoon.clone()))
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|(name, list)| Hoon::CenTis(name, list))
}

pub fn centis_wide<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist()
    .then_ignore(just(Token::Ace))
    .then(list_wing_hoon_wide(hoon.clone()))
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(name, list)| Hoon::CenTis(name, list))
}

pub fn censig<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(winglist())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| Hoon::CenSig(p, Box::new(q), vec![r]))
}

pub fn censig_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Sig)
    .ignore_then(
        winglist()
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .then_ignore(just(Token::Ace))
        .then(list_hoon_wide(hoon_wide.clone()))
        .delimited_by(just(Token::Pal), just(Token::Par))
    )
    .map(|((w, h), list)| Hoon::CenSig(w, Box::new(h), list))
}

pub fn centis_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist()
    .then(list_wing_hoon_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par)))
    .map(|(name, list)| Hoon::CenTis(name, list))
}

pub fn cenlus_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(spec.clone())
    .then_ignore(gap())
    .then(spec.clone())
    .map(|((p, q), r)| Spec::Make(p, vec![q, r]))
}

pub fn cenhep_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(spec.clone())
    .map(|(p, q)| Spec::Make(p, vec![q]))
}