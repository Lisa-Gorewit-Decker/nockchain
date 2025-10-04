use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};
use logos::Logos;
use std::fs;
use std::collections::HashMap;
use std::time::Instant;

pub mod tokens;
pub mod hoon;

use self::tokens::Token;
use self::hoon::*;

fn gap_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, (), extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Gap)
        .repeated()
        .ignored()
}

fn aura_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Aura(str) => str }
        .map(|s| Hoon::Base(BaseTyp::Atom { aura: s.to_string() }))
}

fn wutzap_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Zap)
    .ignore_then(hoon.clone())
    .map(|h| Hoon::WutZap(Box::new(h)))
}


fn dottis_irregular_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::DotTisIrregular)
    .ignore_then(hoon.clone())
    .then_ignore(just(Token::Ace))
    .then(hoon.clone())
    .then_ignore(just(Token::Par))
    .map(|(p, q)| Hoon::DotTis(Box::new(p), Box::new(q)))
}

fn alias_parser<'tokens, 'src: 'tokens, I>(
    spec: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Name(n) => n }
        .then_ignore(just(Token::Tis))
        .then(spec.clone())
        .map(|(n, s)| Hoon::BucTis(n.to_string(), Box::new(s)))
}

fn spec_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
    spec: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     choice((
        alias_parser(spec.clone()),  //  foo=bar
        aura_parser()
    )).boxed()
}

fn tisdot_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisDot)
        .ignore_then(gap_parser())
        .ignore_then(name_parser())  //  this should be Wing parser, not just name
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::TisDot(Box::new(p), Box::new(q), Box::new(r)))
}

fn tiscol_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let pair = name_parser()
                .ignore_then(gap_parser())
                .then(hoon.clone())
                .then_ignore(gap_parser())
                .map(|(name, h)| (name, Box::new(h)));

    let pairs = pair.repeated().at_least(1).collect::<Vec<_>>();

    just(Token::TisCol)
        .then_ignore(gap_parser())
        .ignore_then(pairs)
        .then_ignore(just(Token::TisTis))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(p, q)| Hoon::TisCol(p, Box::new(q)))
}

fn tismic_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisMic)
        .ignore_then(gap_parser())
        .ignore_then(spec_parser(hoon.clone()))  //  do all specs parses here?
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::TisMic(Box::new(p), Box::new(r), Box::new(q)))
}

fn tisfas_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisFas)
        .ignore_then(gap_parser())
        .ignore_then(name_parser().or(alias_parser(hoon.clone())))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::TisFas(Box::new(p), Box::new(q), Box::new(r)))
}

fn tiswut_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
       just(Token::TisWut)
        .ignore_then(gap_parser())
        .ignore_then(name_parser())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(((p, q), r), s)| Hoon::TisWut(Box::new(p), Box::new(q), Box::new(r), Box::new(s)))
}

fn wutgar_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutGar)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .map(|(p, q)| Hoon::WutGar(Box::new(p), Box::new(q)))
}


fn wutdot_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutDot)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::WutCol(Box::new(p), Box::new(r), Box::new(q)))
}

fn wutcol_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     just(Token::WutCol)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|((p, q), r)| Hoon::WutCol(Box::new(p), Box::new(q), Box::new(r)))
}

fn spec_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     recursive(|spec| {
        choice((
            spec_wide_parser(hoon.clone(), spec.clone()),
        )).boxed()
    })
}

fn bartis_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarTis)
        .ignore_then(gap_parser())
        .ignore_then(spec_parser(hoon.clone()))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| Hoon::BarTis(Box::new(s), Box::new(h)))
}

fn barhep_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarHep)
        .ignore_then(gap_parser())
        .ignore_then(hoon.clone())
        .map(|h| Hoon::BarHep(Box::new(h)))
}

fn name_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {
        Token::Name(name_str) => {
            let wing = vec![Limb::Term(name_str.to_string())];
            Hoon::Wing(wing)
        }
    }.labelled("Name")
}

fn gatecall_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Pal)
        .ignore_then(hoon.clone())
        .then(
            just(Token::Ace)
                .ignore_then(hoon.clone())
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
    .map(|(func, args)| Hoon::CenCol(Box::new(func), args))
    .then_ignore(just(Token::Par))
}

fn kethep_parser<'tokens, 'src: 'tokens, I>(
        hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
    ) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
    where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
    {
    just(Token::KetHep)
        .ignore_then(gap_parser())
        .ignore_then(spec_parser(hoon.clone()))
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(s, h)| Hoon::KetHep(Box::new(s), Box::new(h)))
}

fn ketsig_wide_parser<'tokens, 'src: 'tokens, I>(
        hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
    ) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
    where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
    {
    just(Token::KetSigWide).
        ignore_then(hoon.clone()).
        then_ignore(just(Token::Par)).
        map(|h| Hoon::KetSig(Box::new(h)))
}

fn zapzap_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::ZapZap).map(|_| Hoon::ZapZap)
}

fn number_parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {
        Token::Number(num_str) => {
            Hoon::Sand("ud".to_string(), Noun::Atom(num_str.to_string()))
        }
    }.labelled("Number")
}
use chumsky::prelude::*;

fn luslus_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, (String, Hoon), extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::LusLus)
        .ignore_then(gap_parser())
        .ignore_then(select! { Token::Name(s) => s })
        .then_ignore(gap_parser())
        .then(hoon.clone())
        .map(|(name, hoon)| (name.to_string(), hoon))
}


fn chapters_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Vec<(Option<String>, Vec<(String, Hoon)>)>, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let optional_chapter_label = just(Token::LusBar)
        .ignore_then(select! { Token::Name(s) => s })
        .then_ignore(gap_parser())
        .or_not()
        .map(|opt: Option<&str>| opt.map(|s| s.to_string()));

    let chapter = optional_chapter_label
        .then(luslus_parser(hoon.clone())
        .then_ignore(gap_parser())
        .repeated().at_least(1).collect::<Vec<_>>());

    chapter.repeated().at_least(1).collect::<Vec<_>>()
}

fn barcen_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::BarCen)
        .ignore_then(gap_parser())
        .ignore_then(chapters_parser(hoon.clone()))
        .then_ignore(just(Token::HepHep))
        .map(|chapters_vec: Vec<(Option<String>, Vec<(String, Hoon)>)>| {
            let mut tome_map = HashMap::new();
            for (opt_label, arms_vec) in chapters_vec {
                let mut arms_map = HashMap::new();
                for (name, hoon) in arms_vec {
                    arms_map.insert(name, hoon);
                }
                let key = opt_label.unwrap_or_else(|| "$".to_string());
                let what = "".to_string();
                let tome: Tome = (what, arms_map);
                tome_map.insert(key, Box::new(tome));
            }
            Hoon::BarCen(None, tome_map)
        })
        .boxed()
}

fn hoon_parser<'tokens, 'src: 'tokens, I>(
    hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        barcen_parser(hoon.clone()),
        bartis_parser(hoon.clone()),
        barhep_parser(hoon.clone()),
        kethep_parser(hoon.clone()),
        wutcol_parser(hoon.clone()),
        wutdot_parser(hoon.clone()),
        wutgar_parser(hoon.clone()),
        tisdot_parser(hoon.clone()),
        tismic_parser(hoon.clone()),
        tisfas_parser(hoon.clone()),
        tiscol_parser(hoon.clone()),
        tiswut_parser(hoon.clone()),
        zapzap_parser(),
    )).boxed()
}

fn hoon_wide_parser<'tokens, 'src: 'tokens, I>(
        hoon: impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>> + Clone + 'tokens
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
            ketsig_wide_parser(hoon.clone()),
            gatecall_parser(hoon.clone()),
            dottis_irregular_parser(hoon.clone()),
            wutzap_irregular_parser(hoon.clone()),
            name_parser(),
            number_parser(),
        )).boxed()
}

fn parser<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Hoon, extra::Err<Rich<'tokens, Token<'src>>>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    recursive(|hoon| {
        choice((
            hoon_parser(hoon.clone()),
            hoon_wide_parser(hoon.clone()),
        )).boxed()
    })
}

fn main() {
    let source = fs::read_to_string("./src/test1.hoon").expect("Failed to read file");
    let start = Instant::now();

    let token_iter = Token::lexer(&source)
        .spanned()
        // Convert logos errors into tokens. We want parsing to be recoverable and not fail at the lexing stage, so
        // we have a dedicated `Token::Error` variant that represents a token error that was previously encountered
        .map(|(tok, span)| match tok {
            // Turn the `Range<usize>` spans logos gives us into chumsky's `SimpleSpan` via `Into`, because it's easier
            // to work with
            Ok(tok) => (tok, span.into()),
            Err(()) => (Token::LexerError, span.into()),
        });

    let token_stream = Stream::from_iter(token_iter)
        // Tell chumsky to split the (Token, SimpleSpan) stream into its parts so that it can handle the spans for us
        // This involves giving chumsky an 'end of input' span: we just use a zero-width span at the end of the string
        .map((0..source.len()).into(), |(t, s): (_, _)| (t, s));

    match parser().parse(token_stream).into_result() {
        Ok(res) => {
            let took = start.elapsed();
            println!("{:?}", res);
            println!("took: {:?}", took);
        },
        Err(errs) => {
            for err in errs {
                Report::build(ReportKind::Error, ((), err.span().into_range()))
                    .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                    .with_code(3)
                    .with_message(err.to_string())
                    .with_label(
                        Label::new(((), err.span().into_range()))
                            .with_message(err.reason().to_string())
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint(Source::from(source.clone()))
                    .unwrap();
            }
        }
    };
}