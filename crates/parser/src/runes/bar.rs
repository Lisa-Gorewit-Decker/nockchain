use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::collections::*;

pub fn bar_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:      impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Cen).ignore_then(barcen(hoon.clone(), spec.clone())),
        just(Token::Dot).ignore_then(bardot(hoon.clone())),
        just(Token::Tar).ignore_then(bartar(hoon.clone(), spec.clone())),
        just(Token::Cab).ignore_then(barcab(hoon.clone(), spec.clone())),
        just(Token::Pat).ignore_then(barpat(hoon.clone(), spec.clone())),
        just(Token::Tis).ignore_then(bartis(hoon.clone(), spec.clone())),
        just(Token::Sig).ignore_then(barsig(hoon.clone(), spec.clone())),
        just(Token::Hep).ignore_then(barhep(hoon.clone())),
        just(Token::Ket).ignore_then(barket(hoon.clone(), spec.clone())),
        just(Token::Col).ignore_then(barcol(hoon.clone())),
        just(Token::Buc).ignore_then(barbuc(hoon.clone(), spec.clone())),
    ))
}

pub fn bar_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Dot).ignore_then(bardot_wide(hoon_wide.clone())),
        just(Token::Tar).ignore_then(bartar_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Tis).ignore_then(bartis_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Sig).ignore_then(barsig_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Hep).ignore_then(barhep_wide(hoon_wide.clone())),
        just(Token::Col).ignore_then(barcol_wide(hoon_wide.clone())),
        just(Token::Wut).ignore_then(barwut_wide(hoon_wide.clone())),
    ))
}

fn barcen<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(chapters(hoon.clone(), spec.clone()))
    .map(|map_term_tome| Hoon::BarCen(None, map_term_tome))
}

fn bardot<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Hoon::BarDot(Box::new(p)))
}

fn bardot_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|p| Hoon::BarDot(Box::new(p)))
}

pub fn bartar<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(s, h)| Hoon::BarTar(Box::new(s), Box::new(h)))
}

pub fn bartar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(s, h)| Hoon::BarTar(Box::new(s), Box::new(h)))
}

pub fn barsig<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(s, h)| Hoon::BarSig(Box::new(s), Box::new(h)))
}

pub fn barsig_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(s, h)| Hoon::BarSig(Box::new(s), Box::new(h)))
}

pub fn bartis<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(s, h)| Hoon::BarTis(Box::new(s), Box::new(h)))
}

fn bartis_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(s, h)| Hoon::BarTis(Box::new(s), Box::new(h)))
}

pub fn barbuc<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(list_spec())
    .then_ignore(gap())
    .then(spec.clone())
    .map(|(list, h)| Hoon::BarBuc(list, Box::new(h)))
}

pub fn barcol<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(s, h)| Hoon::BarCol(Box::new(s), Box::new(h)))
}

pub fn barhep<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|h| Hoon::BarHep(Box::new(h)))
}

pub fn barhep_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|h| Hoon::BarHep(Box::new(h)))
}

pub fn barwut_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|h| Hoon::BarWut(Box::new(h)))
}

pub fn barket<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(chapters(hoon.clone(), spec.clone()))
    .map(|(h, map_term_tome)| Hoon::BarKet(Box::new(h), map_term_tome))
}

pub fn barpat<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(chapters(hoon.clone(), spec.clone()))
    .map(|map_term_tome| Hoon::BarPat(None, map_term_tome))
}

pub fn barcab<'tokens, 'src: 'tokens, I>(
    hoon: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec: impl ParserExt<'tokens, 'src, I, Spec> ,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let aliases =     //   +*  foo  1
                just([Token::Lus, Token::Tar])
                    .ignore_then(gap())
                    .ignore_then(list_term_hoon(hoon.clone()));

    gap()
    .ignore_then(spec.clone())
    .then_ignore(gap())
    .then(aliases.then_ignore(gap()).or_not().map(|x| x.unwrap_or(vec![])))
    .then(chapters(hoon.clone(), spec.clone()))
    .map(|((spec, alas), map_term_tome)| Hoon::BarCab(Box::new(spec), alas, map_term_tome))
}

pub fn barcol_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::BarCol(Box::new(p), Box::new(q)))
}
