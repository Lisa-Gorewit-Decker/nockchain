use crate::lexer::tokens::Token;
use crate::ast::hoon::*;
use crate::utils::*;
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use std::collections::*;

pub fn sig_runes_tall<'tokens, 'src: 'tokens, I>(
    hoon:      impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Cen).ignore_then(sigcen(hoon.clone())),
        just(Token::Fas).ignore_then(sigfas(hoon.clone())),
        just(Token::Cab).ignore_then(sigcab(hoon.clone())),
        just(Token::Lus).ignore_then(siglus(hoon.clone())),
        just(Token::Zap).ignore_then(sigzap(hoon.clone())),
        just(Token::Bar).ignore_then(sigbar(hoon.clone())),
        just(Token::Gar).ignore_then(siggar(hoon.clone())),
        just(Token::Gal).ignore_then(siggal(hoon.clone())),
        just(Token::Pam).ignore_then(sigpam(hoon.clone())),
        just(Token::Wut).ignore_then(sigwut(hoon.clone())),
    ))
}

pub fn sig_runes_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Gar).ignore_then(siggar_wide(hoon_wide.clone())),
        just(Token::Lus).ignore_then(siglus_wide(hoon_wide.clone())),
        just(Token::Cab).ignore_then(sigcab_wide(hoon_wide.clone())),
        just(Token::Bar).ignore_then(sigbar_wide(hoon_wide.clone())),
        just(Token::Pam).ignore_then(sigpam_wide(hoon_wide.clone())),
    ))
}


pub fn sigpam<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(just(Token::Gar)
            .repeated()
            .at_most(3)
            .count()
            .then_ignore(gap())
            .or_not()
        )
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|((maybe_p, q), r)| {
        let p = maybe_p.unwrap_or(0);
        Hoon::SigPam(p as u64, Box::new(q), Box::new(r))
    })
}

pub fn sigwut<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(just(Token::Gar)
            .repeated()
            .at_most(3)
            .count()
            .then_ignore(gap())
            .or_not()
        )
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(((maybe_p, q), r), s)| {
        let p = maybe_p.unwrap_or(0);
        Hoon::SigWut(p as u64, Box::new(q), Box::new(r), Box::new(s))
    })
}

pub fn sigpam_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Gar)
    .repeated()
    .at_most(3)
    .count()
    .then_ignore(just(Token::Ace))
    .or_not()
    .then(hoon_wide.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((maybe_p, q), r)| {
        let p = maybe_p.unwrap_or(0);
        Hoon::SigPam(p as u64, Box::new(q), Box::new(r))
    })
}

pub fn sigzap<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::SigZap(Box::new(p), Box::new(q)))
}

pub fn sigbar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::SigBar(Box::new(p), Box::new(q)))
}

pub fn sigbar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_wide.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::SigBar(Box::new(p), Box::new(q)))
}

pub fn siglus<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
            //  the hoon parser accepts an optional first arg here
    gap()  //  but its never used anywhere, and idk what is...
    .ignore_then(hoon.clone())
    .map(|p| Hoon::SigLus(0, Box::new(p)))
}

pub fn siglus_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
                        //  the hoon parser accepts an optional first arg here
    hoon_wide.clone()   //  but its never used anywhere, and idk what is...
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|p| Hoon::SigLus(0, Box::new(p)))
}

pub fn sigcab<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::SigCab(Box::new(p), Box::new(q)))
}

pub fn sigcab_wide<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Hoon::SigCab(Box::new(p), Box::new(q)))
}

pub fn sigcen<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(jet_signature())
    .then_ignore(gap())
    .then(hoon.clone())
    .then_ignore(gap())
    .then(jet_hooks(hoon.clone()))
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(((p, q), r), s)| Hoon::SigCen(p, Box::new(q), r, Box::new(s)))
}

pub fn sigfas<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(jet_signature())
    .then_ignore(gap())
    .then(hoon.clone())
    .map(|(p, q)| Hoon::SigFas(p, Box::new(q)))
}


pub fn siggar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    term()
    .then(just(Token::Dot)
            .ignore_then(hoon_wide.clone())
            .or_not()
            )
    .then_ignore(just(Token::Ace))
    .then(hoon_wide.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|((term, maybe_hoon), q)|  {
        match maybe_hoon {
            None =>{
                Hoon::SigGar(TermOrPair::Term(term), Box::new(q))
            }
            Some(h) => {
                Hoon::SigGar(TermOrPair::Pair((term, Box::new(h))), Box::new(q))
            }
        }
    })
}

pub fn siggar<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(term())
    .then(just(Token::Dot)
            .ignore_then(hoon_wide.clone())
            .or_not())
    .then_ignore(gap())
    .then(hoon_wide.clone())
    .map(|((term, maybe_hoon), q)|  {
        match maybe_hoon {
            None =>{
                Hoon::SigGar(TermOrPair::Term(term), Box::new(q))
            }
            Some(h) => {
                Hoon::SigGar(TermOrPair::Pair((term, Box::new(h))), Box::new(q))
            }
        }
    })
}

pub fn siggal<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(term())
    .then(just(Token::Dot)
            .ignore_then(hoon_wide.clone())
            .or_not())
    .then_ignore(gap())
    .then(hoon_wide.clone())
    .map(|((term, maybe_hoon), q)|  {
        match maybe_hoon {
            None =>{
                Hoon::SigGal(TermOrPair::Term(term), Box::new(q))
            }
            Some(h) => {
                Hoon::SigGal(TermOrPair::Pair((term, Box::new(h))), Box::new(q))
            }
        }
    })
}