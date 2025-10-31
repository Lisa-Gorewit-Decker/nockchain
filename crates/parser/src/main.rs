use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::{
    input::{Stream, ValueInput},
    prelude::*,
};

use std::fs;
use std::collections::HashMap;
use std::time::Instant;
use std::io::Write;
use std::path::PathBuf;

use logos::Logos;
use clap::{Parser as ClapParser, command, arg};

use parser::lexer::tokens::Token;
use parser::ast::hoon::*;
use parser::utils::*;
use parser::runes::*;

fn spec_parser<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Buc).ignore_then(buc_spec_tall(hoon.clone(), spec.clone())),
        just(Token::Cen).ignore_then(cen_spec_tall(hoon.clone(), spec.clone())),
        spec_wide.clone(),
    )).boxed()
}

fn spec_wide_parser<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let parsers = vec![
        just(Token::Buc).ignore_then(buc_spec_wide(hoon_wide.clone(), spec_wide.clone())).boxed(),
        buccab_spec_irregular(hoon_wide.clone()).boxed(),  //  _p
        bucmic_spec_irregular(hoon_wide.clone()).boxed(),  //  ,p
        buctis_irregular(spec_wide.clone()).boxed(),  // foo=bar, =bar,  =foo=bar
        buccol_irregular(spec_wide.clone()).boxed(),  // [foo=bar foo=bar]
        reference_spec(spec_wide.clone()).boxed(),    // foo or foo:bar
        bucwut_irregular(spec_wide.clone()).boxed(),  // ?(foo bar)
        parenthesis_spec(hoon_wide.clone(),
                                spec_wide.clone()).boxed(),  // (foo bar)
        just(Token::Ket).to(Spec::Base(BaseType::Cell)).boxed(),
        just(Token::Wut).to(Spec::Base(BaseType::Flag)).boxed(),
        just(Token::Sig).to(Spec::Base(BaseType::Null)).boxed(),
        just(Token::Tar).to(Spec::Base(BaseType::Noun)).boxed(),
        just([Token::Cen, Token::Sig]).to(Spec::Leaf("%n".to_string(), "0".to_string())).boxed(),
        just([Token::Cen, Token::Bar]).to(Spec::Leaf("%f".to_string(), "1".to_string())).boxed(),
        just([Token::Cen, Token::Pam]).to(Spec::Leaf("%f".to_string(), "0".to_string())).boxed(),
        aura_spec().boxed(), //  @foo
        spec_term().boxed(), // %$, %foo, %123
    ];

    choice(parsers).boxed().labelled("spec-wide")
}

fn hoon_wide_parser<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let parsers = vec![
        just(Token::Bar)
                .ignore_then(bar_runes_wide(hoon_wide.clone(),
                                             spec_wide.clone())).boxed(),
        just(Token::Tis)
                .ignore_then(tis_runes_wide(hoon_wide.clone(),
                                             spec_wide.clone())).boxed(),
        just(Token::Wut)
                .ignore_then(wut_runes_wide(hoon_wide.clone(),
                                             spec_wide.clone())).boxed(),
        just(Token::Cen)
                .ignore_then(
                        cen_runes_wide(hoon_wide.clone()),
                        ).boxed(),
        just(Token::Col)
                .ignore_then(
                        col_runes_wide(hoon_wide.clone()),
                        ).boxed(),
        just(Token::Sig)
                .ignore_then(
                        sig_runes_wide(hoon_wide.clone()),
                        ).boxed(),
        just(Token::Buc)
                .ignore_then(
                        buc_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                        ).boxed(),
        just(Token::Ket)
                .ignore_then(
                        ket_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                        ).boxed(),
        just(Token::Zap)
                .ignore_then(
                        zap_runes_wide(hoon_wide.clone()),
                        ).boxed(),
        just(Token::Mic)
                .ignore_then(
                        mic_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                        ).boxed(),
        just(Token::Dot)
                .ignore_then(
                        dot_runes_wide(hoon_wide.clone()),
                        ).boxed(),
        tape().boxed(),
        path(hoon_wide.clone()).boxed(),
        buccab_irregular(hoon_wide.clone()).boxed(),              //  _p
        miccol_irregular(hoon_wide.clone()).boxed(),              //  :(a b .. z)
        censig_irregular(hoon_wide.clone()).boxed(),              //  ~(a b c)
        constant_separator_hoon(hoon_wide.clone()).boxed(),           //  const+hoon,  const/hoon
        dottis_irregular(hoon_wide.clone()).boxed(),              //  =(p q)
        list_syntax(hoon_wide.clone()).boxed(),                   // [p ... pn], ~[foo], [foo]~
        kettar_irregular(spec_wide.clone()).boxed(),              //  *foo
        kethep_irregular(hoon_wide.clone(),
                                spec_wide.clone()).boxed(),              //  `p`q
        tic_cell_construction(hoon_wide.clone()).boxed(),         //  `a
        wutzap_irregular(hoon_wide.clone()).boxed(),              //  !p
        wutbar_irregular(hoon_wide.clone()).boxed(),              //  |(p q)
        wutpam_irregular(hoon_wide.clone()).boxed(),              //  &(p q)
        increment(hoon_wide.clone()).boxed(),          //  +(a) or .+(a)
        ketcol_irregular(spec_wide.clone()).boxed(),   //  ,p
        centis_irregular(hoon_wide.clone()).boxed(),   //  a(b c, d e, f g)
        tell(hoon_wide.clone()).boxed(),  // <foo>
        wing().boxed(),
        function_call(hoon_wide.clone()).boxed(),      //  (a b)
        aura_hoon().boxed(),
        constant().boxed(),
        cord().boxed(),
        number().boxed(),
        select! { Token::Date(d) => Hoon::Sand("%da".to_string(), Noun::Atom(d.to_string()))}.boxed(),
        just(Token::Yes).to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::No).to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just([Token::Cen, Token::Bar]).to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just([Token::Cen, Token::Pam]).to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::Pam).to(Hoon::Sand("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just(Token::Bar).to(Hoon::Sand("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just(Token::Tar).to(Hoon::Base(BaseType::Noun)).boxed(),
        just(Token::Wut).to(Hoon::Base(BaseType::Flag)).boxed(),
        just(Token::Sig).to(Hoon::Bust(BaseType::Null)).boxed(),
    ];

    choice(parsers).boxed().labelled("hoon-wide")
        .then(just(Token::Tis).or(just(Token::Col)).or(just(Token::Ket))
                .then(hoon_wide.clone())
                .or_not())
        .map(|(p, maybe_separator)|  {
            match maybe_separator  {
                Some((Token::Tis, q)) => Hoon::KetTis(Box::new(p), Box::new(q)),
                Some((Token::Col, q)) => Hoon::TisGal(Box::new(p), Box::new(q)),
                Some((Token::Ket, q)) => Hoon::Pair(Box::new(p), Box::new(q)),
                _ => p,
            }
        })
}

fn hoon_tall_parser<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
     let parsers = vec![
    // choice((
            just(Token::Bar)
                .ignore_then(
                    choice((
                        bar_runes_tall(hoon.clone(),
                                                spec.clone()),
                        // bar_runes_wide(hoon_wide.clone(),
                                // spec_wide.clone()),
                    ))).boxed(),
            just(Token::Tis)
                .ignore_then(
                    choice((
                        tis_runes_tall(hoon.clone(),
                                       spec.clone(),
                                       spec_wide.clone()),
                        // tis_runes_wide(hoon_wide.clone(),
                                            //  spec_wide.clone()),
                    ))).boxed(),
            just(Token::Wut)
                .ignore_then(
                    choice((
                        wut_runes_tall(hoon.clone(),
                                       hoon_wide.clone(),
                                       spec.clone(),
                                       spec_wide.clone()),
                        // wut_runes_wide(hoon_wide.clone(),
                                            //  spec_wide.clone()),
                        ))).boxed(),
            just(Token::Cen)
                .ignore_then(choice((
                        cen_runes_tall(hoon.clone()),
                        // cen_runes_wide(hoon_wide.clone()),
                ))
                        ).boxed(),
            just(Token::Col)
                .ignore_then(choice((
                        col_runes_tall(hoon.clone()),
                        // col_runes_wide(hoon_wide.clone()),
                ))
                 ).boxed(),
            just(Token::Sig)
                .ignore_then(choice((
                        sig_runes_tall(hoon.clone()),
                        // sig_runes_wide(hoon_wide.clone()),
                ))).boxed(),
            just(Token::Buc)
                .ignore_then(choice((
                        buc_runes_tall(hoon.clone(), spec.clone()),
                        // buc_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                ))).boxed(),
            just(Token::Ket)
                .ignore_then(choice((
                        ket_runes_tall(hoon.clone(), spec.clone()),
                        // ket_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                ))).boxed(),
            just(Token::Zap)
                .ignore_then(choice((
                        zap_runes_tall(hoon.clone()),
                        // ket_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                ))).boxed(),
            just(Token::Mic)
                .ignore_then(
                        mic_runes_tall(hoon.clone()),
                        ).boxed(),
            just(Token::Dot)
                .ignore_then(
                        dot_runes_tall(hoon.clone()),
                        ).boxed(),
    // )).box
    ];
    choice((parsers)).boxed()
    .labelled("hoon-tall")
}

fn parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let hoon = recursive(|hoon| {
        let mut spec_wide_handle = Recursive::declare();
        let mut hoon_wide_handle = Recursive::declare();

        let spec = recursive(|spec| {
            spec_parser(spec.clone(),
                                spec_wide_handle.clone(),
                                hoon.clone())
        }).labelled("spec");

        let spec_wide_body = recursive(|spec_wide_self| {
            spec_wide_parser(spec_wide_self.clone(), hoon_wide_handle.clone())
        });

        let hoon_wide_body = hoon_wide_parser(
            hoon_wide_handle.clone(),
            spec_wide_handle.clone(),
        );

        spec_wide_handle.define(spec_wide_body);
        hoon_wide_handle.define(hoon_wide_body);

        let spec_wide = spec_wide_handle.clone();
        let hoon_wide = hoon_wide_handle.clone();

        choice((
            hoon_tall_parser(hoon.clone(), hoon_wide.clone(), spec.clone(), spec_wide),
            hoon_wide
        ))
        .boxed()
    });

    hoon.padded_by(gap().or_not()).boxed()
}

#[derive(ClapParser, Debug)]
#[command(author, version, about = "Parses a Hoon source file")]
struct Cli {
    /// Path to the input .hoon file
    #[arg(value_name = "FILE", help = "Input Hoon source file")]
    input: PathBuf,
}

fn main() {
    let cli = Cli::parse();

    let source = fs::read_to_string(&cli.input).unwrap_or_else(|err| {
        eprintln!("Error reading file '{}': {}", cli.input.display(), err);
        std::process::exit(1);
    });

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

    //  print tokens
    // for (tok, span) in token_iter.clone() {
    //     println!("{:?} {:?}", tok, span);
    // }

    let token_stream = Stream::from_iter(token_iter)
        // Tell chumsky to split the (Token, SimpleSpan) stream into its parts so that it can handle the spans for us
        // This involves giving chumsky an 'end of input' span: we just use a zero-width span at the end of the string
        .map((0..source.len()).into(), |(t, s): (_, _)| (t, s));

    match parser().parse(token_stream).into_result() {
        Ok(res) => {
           let took = start.elapsed();
           let json = serde_json::to_string_pretty(&res)
            .expect("serialisation failed");

            let out_path = std::path::PathBuf::from("out.json");

            std::fs::write(&out_path, json + "\n")
                .unwrap_or_else(|e| {
                    eprintln!("Failed to write '{}': {}", out_path.display(), e);
                    std::process::exit(1);
                });

            println!("Result written to {}!", out_path.display());
            println!("took: {:?}", took);
        }
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
