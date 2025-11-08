use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::collections::*;

pub fn mic_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:      impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Col).ignore_then(miccol(hoon.clone())),
        just(Token::Fas).ignore_then(micfas(hoon.clone())),
        just(Token::Gal).ignore_then(micgal(hoon.clone(), spec.clone())),
        just(Token::Sig).ignore_then(micsig(hoon.clone())),
        just(Token::Mic).ignore_then(micmic(hoon.clone(), spec.clone())),
    ))
}

pub fn mic_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Col).ignore_then(miccol_wide(hoon_wide.clone())),
        just(Token::Fas).ignore_then(micfas_wide(hoon_wide.clone())),
        just(Token::Gal).ignore_then(micgal_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Sig).ignore_then(micsig_wide(hoon_wide.clone())),
        just(Token::Mic).ignore_then(micmic_wide(hoon_wide.clone(), spec_wide.clone())),
    ))
}

pub fn micsig<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(list_hoon_tall(hoon.clone()))
    .then_ignore(gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|(func, args)| Hoon::MicSig(Box::new(func), args))
}

pub fn micsig_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .then(
        just(Token::Ace)
            .ignore_then(hoon.clone())
            .repeated()
            .collect::<Vec<_>>()
    )
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(func, args)| Hoon::MicSig(Box::new(func), args))
}

pub fn micmic_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(s, h)| Hoon::MicMic(Box::new(s), Box::new(h)))
}

pub fn micgal<'tokens, 'src: 'tokens, I>(
    hoon:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then(three_hoons_tall(hoon.clone()))
    .map(|(p, ((q, r), s))| Hoon::MicGal(Box::new(p), Box::new(q), Box::new(r), Box::new(s)))
}

pub fn micgal_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(three_hoons_wide(hoon_wide.clone()))
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, ((q, r), s))| Hoon::MicGal(Box::new(p), Box::new(q), Box::new(r), Box::new(s)))
}

pub fn micmic<'tokens, 'src: 'tokens, I>(
    hoon:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec.clone()
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(s, h)| Hoon::MicMic(Box::new(s), Box::new(h)))
}

pub fn micfas<'tokens, 'src: 'tokens, I>(
    hoon:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|(h)| Hoon::MicFas(Box::new(h)))
}

pub fn micfas_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(h)| Hoon::MicFas(Box::new(h)))
}

pub fn miccol<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(list_hoon_tall(hoon.clone()))
    .then_ignore(gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|(p, list)| Hoon::MicCol(Box::new(p), list))
}

pub fn miccol_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(list_hoon_wide(hoon_wide.clone()))
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, list)| Hoon::MicCol(Box::new(p), list))
}

pub fn miccol_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(list_hoon_wide(hoon_wide.clone()))
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, list)| Hoon::MicCol(Box::new(p), list))
}
