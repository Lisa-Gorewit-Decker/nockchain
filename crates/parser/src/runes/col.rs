use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use std::collections::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};

pub fn col_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Ket).ignore_then(colket(hoon.clone())),
        just(Token::Cab).ignore_then(colcab(hoon.clone())),
        just(Token::Lus).ignore_then(collus(hoon.clone())),
        just(Token::Hep).ignore_then(colhep(hoon.clone())),
        just(Token::Tar).ignore_then(coltar(hoon.clone())),
        just(Token::Sig).ignore_then(colsig(hoon.clone())),
    ))
}

pub fn col_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Ket).ignore_then(colket_wide(hoon_wide.clone())),
        just(Token::Cab).ignore_then(colcab_wide(hoon_wide.clone())),
        just(Token::Lus).ignore_then(collus_wide(hoon_wide.clone())),
        just(Token::Hep).ignore_then(colhep_wide(hoon_wide.clone())),
        just(Token::Tar).ignore_then(coltar_wide(hoon_wide.clone())),
        just(Token::Sig).ignore_then(colsig_wide(hoon_wide.clone())),
    ))

}

pub fn collus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    three_hoons_tall(hoon.clone())
    .map(|((p, q), r)| Hoon::ColLus(Box::new(p), Box::new(q), Box::new(r)))
}

pub fn collus_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    three_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((p, q), r)| Hoon::ColLus(Box::new(p), Box::new(q), Box::new(r)))
}

pub fn colhep<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_tall(hoon.clone())
    .map(|(p, q)| Hoon::ColHep(Box::new(p), Box::new(q)))
}

pub fn colhep_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::ColHep(Box::new(p), Box::new(q)))
}

pub fn colcab<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::ColCab(Box::new(p), Box::new(q)))
}


pub fn colcab_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_hoons_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::ColCab(Box::new(p), Box::new(q)))
}

pub fn colket<'tokens, 'src: 'tokens, I>(
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
                Hoon::ColKet(Box::new(p),
                             Box::new(q),
                             Box::new(s),
                             Box::new(r)))
}

pub fn colket_wide<'tokens, 'src: 'tokens, I>(
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
                Hoon::ColKet(Box::new(p),
                                Box::new(q),
                                Box::new(s),
                                Box::new(r)))
}

pub fn coltar<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(list_hoon_tall(hoon.clone()))
    .then_ignore(gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|list| Hoon::ColTar(list))
}

pub fn coltar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    list_hoon_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|list| Hoon::ColTar(list))
}

pub fn colsig<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(list_hoon_tall(hoon.clone()))
    .then_ignore(gap())
    .then_ignore(just([Token::Tis, Token::Tis]))
    .map(|list| Hoon::ColSig(list))
}

pub fn colsig_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    list_hoon_wide(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|list| Hoon::ColSig(list))
}


pub fn list_syntax<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Sig, Token::Sel]).to(true).or(just(Token::Sel).to(false))   //  ~[  or  [
    .then(hoon_wide.clone()
            .separated_by(just(Token::Ace))
            .at_least(1)
            .collect::<Vec<_>>()
    )
    .then(just([Token::Ser, Token::Sig]).to(true).or(just(Token::Ser).to(false)))  //  ]~ or ]
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