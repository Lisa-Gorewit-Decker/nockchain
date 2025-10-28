use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use std::collections::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};

pub fn buc_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:      impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Cen).ignore_then(buccen(spec.clone())),
        just(Token::Wut).ignore_then(bucwut(spec.clone())),
        just(Token::Pat).ignore_then(bucpat(spec.clone())),
        just(Token::Col).ignore_then(buccol(spec.clone())),
        just(Token::Lus).ignore_then(buclus(spec.clone())),
        just(Token::Ket).ignore_then(bucket(spec.clone())),
        just(Token::Sig).ignore_then(bucsig(hoon.clone(), spec.clone())),
    ))
}

pub fn buc_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Sig).ignore_then(bucsig_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Pat).ignore_then(bucpat_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Cen).ignore_then(buccen_wide(spec_wide.clone())),
        just(Token::Wut).ignore_then(bucwut_wide(spec_wide.clone())),
    ))
}

pub fn buccab_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cab)
        .ignore_then(hoon_wide.clone())
        .map(|h| Hoon::KetCol(Box::new(Spec::BucCab(h))))
}

pub fn bucket<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(spec.clone())
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucKet(Box::new(p), Box::new(q)))))
}

pub fn buclus<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(select! { Token::Name(s) => s.to_string() })
    .then_ignore(gap())
    .then(spec.clone())
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucLus(p, Box::new(q)))))
}

pub fn bucwut<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
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

pub fn bucwut_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
        spec_wide.clone()
        .separated_by(just(Token::Ace))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Pal), just(Token::Par))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Hoon::KetCol(Box::new(
                        Spec::BucWut(Box::new(first.clone()),
                                      rest.to_vec())
            ))
        })
}

pub fn bucsig_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(spec_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(h, s)| Hoon::KetCol(Box::new(
                                Spec::BucSig(h, Box::new(s))
                                )))
}

pub fn bucpat<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(spec.clone())
    .map(|(p, q)| {
            Hoon::KetCol(Box::new(Spec::BucPat(
                                        Box::new(p),
                                        Box::new(q))))
        })
}

pub fn buccol<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
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

pub fn bucsig<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(spec.clone())
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucSig(p, Box::new(q)))))
}

pub fn buccen<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
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

pub fn buccen_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .separated_by(just(Token::Ace))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
                Hoon::KetCol(Box::new(Spec::BucCen(
                            Box::new(first.clone()), rest.to_vec())))
        })
}

pub fn bucpat_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(spec_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucPat(Box::new(p),
                                                    Box::new(q)))))
}
