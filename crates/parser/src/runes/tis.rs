use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::collections::*;

pub fn tis_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:      impl ParserExt<'tokens, 'src, I, Spec>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Bar).ignore_then(tisbar(hoon.clone(), spec.clone())),
        just(Token::Dot).ignore_then(tisdot(hoon.clone())),
        just(Token::Wut).ignore_then(tiswut(hoon.clone())),
        just(Token::Ket).ignore_then(tisket(hoon.clone(), spec_wide.clone())),
        just(Token::Col).ignore_then(tiscol(hoon.clone())),
        just(Token::Fas).ignore_then(tisfas(hoon.clone(), spec_wide.clone())),
        just(Token::Mic).ignore_then(tismic(hoon.clone(), spec_wide.clone())),
        just(Token::Gal).ignore_then(tisgal(hoon.clone())),
        just(Token::Gar).ignore_then(tisgar(hoon.clone())),
        just(Token::Hep).ignore_then(tishep(hoon.clone())),
        just(Token::Tar).ignore_then(tistar(hoon.clone(), spec_wide.clone())),
        just(Token::Com).ignore_then(tiscom(hoon.clone())),
        just(Token::Lus).ignore_then(tislus(hoon.clone())),
        just(Token::Sig).ignore_then(tissig(hoon.clone())),
    ))
}

pub fn tis_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Bar).ignore_then(tisbar_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Dot).ignore_then(tisdot_wide(hoon_wide.clone())),
        just(Token::Wut).ignore_then(tiswut_wide(hoon_wide.clone())),
        just(Token::Ket).ignore_then(tisket_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Col).ignore_then(tiscol_wide(hoon_wide.clone())),
        just(Token::Fas).ignore_then(tisfas_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Mic).ignore_then(tismic_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Gal).ignore_then(tisgal_wide(hoon_wide.clone())),
        just(Token::Gar).ignore_then(tisgar_wide(hoon_wide.clone())),
        just(Token::Hep).ignore_then(tishep_wide(hoon_wide.clone())),
        just(Token::Tar).ignore_then(tistar_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Com).ignore_then(tiscom_wide(hoon_wide.clone())),
        just(Token::Lus).ignore_then(tislus_wide(hoon_wide.clone())),
        just(Token::Sig).ignore_then(tissig_wide(hoon_wide.clone())),
    ))
}

pub fn tiswut_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(((p, q), r), s)| Hoon::TisWut(p, Box::new(q), Box::new(r), Box::new(s)))
}

pub fn tiswut<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(winglist())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(((p, q), r), s)| Hoon::TisWut(p, Box::new(q), Box::new(r), Box::new(s)))
}

pub fn tisgar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::TisGar(Box::new(p), Box::new(q)))
}

pub fn tisgal_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::TisGal(Box::new(p), Box::new(q)))
}

pub fn tishep_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::TisHep(Box::new(p), Box::new(q)))
}

pub fn tiscom_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::TisCom(Box::new(p), Box::new(q)))
}

pub fn tislus_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::TisLus(Box::new(p), Box::new(q)))
}

pub fn tisket<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(variable_name_and_type(spec_wide.clone()))
    .then_ignore(gap())
    .then(winglist())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(((p, q), r), s)| Hoon::TisKet(p, q, Box::new(r), Box::new(s)))
}

pub fn tisket_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    variable_name_and_type(spec_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(winglist())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(((p, q), r), s)| Hoon::TisKet(p, q, Box::new(r), Box::new(s)))
}

pub fn tisfas<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(variable_name_and_type(spec_wide.clone()))
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| Hoon::TisFas(p, Box::new(q), Box::new(r)))
}

pub fn tisfas_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    variable_name_and_type(spec_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| Hoon::TisFas(p, Box::new(r), Box::new(q)))
}

pub fn tismic_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    variable_name_and_type(spec_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| Hoon::TisMic(p, Box::new(r), Box::new(q)))
}

pub fn tismic<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(variable_name_and_type(spec_wide.clone()))
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| Hoon::TisMic(p, Box::new(r), Box::new(q)))
}

pub fn tiscol<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(list_wing_hoon_tall(hoon.clone()))
    .then_ignore(just([Token::Tis, Token::Tis]))
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::TisCol(p, Box::new(q)))
}

pub fn tiscol_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    list_wing_hoon_wide(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::TisCol(p, Box::new(q)))
}

pub fn tisbar<'tokens, 'src: 'tokens, I>(
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
    .map(|(p, q)| Hoon::TisBar(Box::new(p), Box::new(q)))
}

pub fn tisbar_wide<'tokens, 'src: 'tokens, I>(
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
    .map(|(p, q)| Hoon::TisBar(Box::new(p), Box::new(q)))
}

pub fn tisgar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::TisGar(Box::new(p), Box::new(q)))
}

pub fn tistar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(select! { Token::AlphaNumeric(n) => n.to_string() } )
    .then(just(Token::Tis)
            .ignore_then(spec_wide.clone())
            .map(|s| Box::new(s))
            .or_not()
        )
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(((term, maybe_spec), q), r)| {
            Hoon::TisTar((term, maybe_spec), Box::new(q), Box::new(r))
    })
}

pub fn tistar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::AlphaNumeric(n) => n.to_string() }
    .then(just(Token::Tis)
            .ignore_then(spec_wide.clone())
            .map(|s| Box::new(s))
            .or_not()
        )
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(((term, maybe_spec), q), r)| {
            Hoon::TisTar((term, maybe_spec), Box::new(q), Box::new(r))
    })
}

pub fn tisdot<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(winglist())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((p, q), r)| Hoon::TisDot(p, Box::new(q), Box::new(r)))
}

pub fn tisdot_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    winglist()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| Hoon::TisDot(p, Box::new(q), Box::new(r)))
}

pub fn tiscom<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_tall(hoon.clone())
    .map(|(p, q)| Hoon::TisCom(Box::new(p), Box::new(q)))
}

pub fn tislus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_tall(hoon.clone())
    .map(|(p, q)| Hoon::TisLus(Box::new(p), Box::new(q)))
}

pub fn tisgal<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::TisGal(Box::new(p), Box::new(q)))
}

pub fn tishep<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::TisHep(Box::new(p), Box::new(q)))
}

pub fn tissig<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(
        hoon.clone()
        .separated_by(gap())
        .at_least(2)
        .collect::<Vec<_>>())
    .then_ignore(gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|list| Hoon::TisSig(list))
}

pub fn tissig_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .separated_by(just(Token::Ace))
    .at_least(2)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|list| Hoon::TisSig(list))
}