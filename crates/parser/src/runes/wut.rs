use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::collections::*;

pub fn wut_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:      impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:      impl ParserExt<'tokens, 'src, I, Spec>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Sig).ignore_then(wutsig(hoon.clone(), hoon_wide.clone())),
        just(Token::Dot).ignore_then(wutdot(hoon.clone())),
        just(Token::Col).ignore_then(wutcol(hoon.clone())),
        just(Token::Bar).ignore_then(wutbar(hoon.clone())),
        just(Token::Gar).ignore_then(wutgar(hoon.clone())),
        just(Token::Gal).ignore_then(wutgal(hoon.clone())),
        just(Token::Ket).ignore_then(wutket(hoon.clone())),
        just(Token::Pam).ignore_then(wutpam(hoon.clone())),
        just(Token::Pat).ignore_then(wutpat(hoon.clone(), hoon_wide.clone())),
        just(Token::Tis).ignore_then(wuttis(hoon.clone(), hoon_wide.clone(), spec.clone())),
        just(Token::Lus).ignore_then(wutlus(hoon.clone(), hoon_wide.clone(), spec.clone())),
        just(Token::Hep).ignore_then(wuthep(hoon.clone(), hoon_wide.clone(), spec.clone())),
        just(Token::Zap).ignore_then(wutzap(hoon.clone())), 
         // add wuthax here..
    ))
}

pub fn wut_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Sig).ignore_then(wutsig_wide(hoon_wide.clone())),
        just(Token::Dot).ignore_then(wutdot_wide(hoon_wide.clone())),
        just(Token::Col).ignore_then(wutcol_wide(hoon_wide.clone())),
        just(Token::Bar).ignore_then(wutbar_wide(hoon_wide.clone())),
        just(Token::Gar).ignore_then(wutgar_wide(hoon_wide.clone())),
        just(Token::Gal).ignore_then(wutgal_wide(hoon_wide.clone())),
        just(Token::Ket).ignore_then(wutket_wide(hoon_wide.clone())),
        just(Token::Pam).ignore_then(wutpam_wide(hoon_wide.clone())),
        just(Token::Pat).ignore_then(wutpat_wide(hoon_wide.clone())),
        just(Token::Tis).ignore_then(wuttis_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Lus).ignore_then(wutlus_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Hep).ignore_then(wuthep_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Zap).ignore_then(wutzap_wide(hoon_wide.clone())),
    ))
}

pub fn wutket<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(winglist()) //  handle non-wing cases here
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| Hoon::WutKet(p, Box::new(q), Box::new(r)))
}

pub fn wutket_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist() //  handle non-wing cases here
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| Hoon::WutKet(p, Box::new(q), Box::new(r)))
}

pub fn wutpat<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(tiki_tall(hoon.clone(),
                                    hoon_wide.clone()))
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| wtpt(p, q, r))
}

pub fn wutpat_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    tiki_wide(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| wtpt(p, q, r))
}

pub fn wutzap<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::WutZap(Box::new(p)))
}

pub fn wutzap_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|p| Hoon::WutZap(Box::new(p)))
}

pub fn wutcol<'tokens, 'src: 'tokens, I>(
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
    .map(|((p, q), r)| Hoon::WutCol(Box::new(p), Box::new(q), Box::new(r)))
}

pub fn wutcol_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| Hoon::WutCol(Box::new(p), Box::new(q), Box::new(r)))
}

pub fn wutgal<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_tall(hoon.clone())
    .map(|(p, q)| Hoon::WutGal(Box::new(p), Box::new(q)))
}

pub fn wutgal_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::WutGal(Box::new(p), Box::new(q)))
}

pub fn wutdot_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| Hoon::WutDot(Box::new(p), Box::new(q), Box::new(r)))
}

pub fn wutgar_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::WutGar(Box::new(p), Box::new(q)))
}

// pub fn wuthax<'tokens, 'src: 'tokens, I>(
//     hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
//     hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
// ) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
// where
//     I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
// {
//     gap()
//     .ignore_then(hoon.clone())
//     .then_ignore(gap())
//     .then(tiki_tall(hoon.clone(), hoon_wide.clone()))
//     .map(|(p, q)| WutHax(q, p))
// }

pub fn wuttis<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(tiki_tall(hoon.clone(), hoon_wide.clone()))
    .map(|(p, q)| wtts(q, p))
}

pub fn wuttis_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(tiki_wide(hoon_wide.clone()))
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| wtts(q, p))
}

pub fn wutdot<'tokens, 'src: 'tokens, I>(
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
    .map(|((p, q), r)| Hoon::WutDot(Box::new(p), Box::new(r), Box::new(q)))
}

pub fn wutgar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::WutGar(Box::new(p), Box::new(q)))
}

pub fn wuthep<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(tiki_tall(hoon.clone(), hoon_wide.clone()))
    .then_ignore(gap())
    .then(spec.clone()
            .then_ignore(gap())
            .then(hoon.clone())
            .then_ignore(gap())
            .repeated()
            .at_least(1)
            .collect::<Vec<_>>()
    )
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|(t, list)| wthp(t, list))
}

pub fn wuthep_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    tiki_wide(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(spec_wide.clone()
        .then_ignore(just(Token::Ace))
        .then(hoon_wide.clone())
        .separated_by(just(Token::Com).then(just(Token::Ace)))
        .at_least(1)
        .collect::<Vec<_>>())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| wthp(p, q))
}

pub fn wutlus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(tiki_tall(hoon.clone(),
                                    hoon_wide.clone()))
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(spec.clone()
            .then_ignore(gap())
            .then(hoon.clone())
            .then_ignore(gap())
            .repeated()
            .at_least(1)
            .collect::<Vec<_>>()
    )
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|((t, h), list)| wtls(t, h, list))
}

pub fn wutlus_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    tiki_wide(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(spec_wide.clone()
            .then_ignore(just(Token::Ace))
            .then(hoon_wide.clone())
            .separated_by(just(Token::Com).then(just(Token::Ace)))
            .at_least(1)
            .collect::<Vec<_>>()
        )
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((t, h), list)| wtls(t, h, list))
}

pub fn wutbar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .separated_by(just(Token::Ace))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|hoons| Hoon::WutBar(hoons))
}

pub fn wutbar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .separated_by(gap())
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(gap(), gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|hoons| Hoon::WutBar(hoons))
}

pub fn wutsig<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(tiki_tall(hoon.clone(), hoon_wide.clone()))
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| wtsg(p, q, r))
}

pub fn wutsig_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    tiki_wide(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| wtsg(p, q, r))
}

pub fn wutpam<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .separated_by(gap())
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(gap(), gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|hoons| Hoon::WutPam(hoons))
}

pub fn wutpam_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .separated_by(just(Token::Ace))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|hoons| Hoon::WutPam(hoons))
}

pub fn wutpam_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Pam)
    .ignore_then(hoon_wide.clone()
            .separated_by(just(Token::Ace))
            .at_least(1)
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Pal), just(Token::Par)))
    .map(|hoons| Hoon::WutPam(hoons))
}

pub fn wutbar_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Bar)
    .ignore_then(
        hoon_wide.clone()
        .separated_by(just(Token::Ace))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Pal), just(Token::Par)))
    .map(|hoons| Hoon::WutBar(hoons))
}

pub fn wutzap_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Zap)
    .ignore_then(hoon_wide.clone())
    .map(|h| Hoon::WutZap(Box::new(h)))
}
