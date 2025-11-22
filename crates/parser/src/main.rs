use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::{
    input::{Stream, ValueInput, StrInput},
    prelude::*,
};

use std::fs;
use std::collections::HashMap;
use std::time::Instant;
use std::io::Write;
use std::path::PathBuf;

use clap::{Parser as ClapParser, command, arg};

use parser::ast::hoon::*;
use parser::utils::*;
use parser::runes::*;

macro_rules! rune_branch_pair {
    ($token:expr, $tall:expr, $wide:expr) => {
        just($token)
            .ignore_then(choice(($tall, $wide)))
            .boxed()
    };
}

macro_rules! rune_branch {
    ($token:expr, $form:expr) => {
        just($token)
            .ignore_then($form)
            .boxed()
    };
}

fn spec_parser<'src>(
    spec:        impl ParserExt<'src, Spec>,
    spec_wide:   impl ParserExt<'src, Spec>,
    hoon:        impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>> + Clone
{
    choice((
        rune_branch!(
            "$",
            buc_spec_tall(hoon.clone(), spec.clone())
        ),
        rune_branch!(
            "%",
            cen_spec_tall(hoon.clone(), spec.clone())
        ),
        spec_wide.clone(),
    )).boxed()
}

fn spec_wide_parser<'src>(
    spec_wide:   impl ParserExt<'src, Spec>,
    hoon_wide:   impl ParserExt<'src, Hoon>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>> + Clone
{
    let parsers = vec![
        just('$').ignore_then(buc_spec_wide(hoon_wide.clone(), spec_wide.clone())).boxed(),
        buccab_spec_irregular(hoon_wide.clone()).boxed(),  //  _p
        bucmic_spec_irregular(hoon_wide.clone()).boxed(),  //  ,p
        buctis_irregular(spec_wide.clone()).boxed(),  // foo=bar, =bar,  =foo=bar
        buccol_irregular(spec_wide.clone()).boxed(),  // [foo=bar foo=bar]
        reference_spec(spec_wide.clone()).boxed(),    // foo or foo:bar
        bucwut_irregular(spec_wide.clone()).boxed(),  // ?(foo bar)
        parenthesis_spec(hoon_wide.clone(),
                                spec_wide.clone()).boxed(),  // (foo bar)
        loop_spec().boxed(),
        just('^').to(Spec::Base(BaseType::Cell)).boxed(),
        just('?').to(Spec::Base(BaseType::Flag)).boxed(),
        just('~').to(Spec::Base(BaseType::Null)).boxed(),
        just('*').to(Spec::Base(BaseType::Noun)).boxed(),
        just("!!").to(Spec::Base(BaseType::Void)).boxed(),
        just("%~").to(Spec::Leaf("%n".to_string(), "0".to_string())).boxed(),
        just("%|").to(Spec::Leaf("%f".to_string(), "1".to_string())).boxed(),
        just("%&").to(Spec::Leaf("%f".to_string(), "0".to_string())).boxed(),
        aura_spec().boxed(), //  @foo
        spec_term().boxed(), // %$, %foo, %123
    ];

    choice(parsers).boxed().labelled("spec-wide")
}

fn hoon_wide_parser<'src>(
    hoon_wide:   impl ParserExt<'src, Hoon>,
    spec_wide:   impl ParserExt<'src, Spec>,
    wer: PathBuf,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>> + Clone
{
    let parsers = vec![
        rune_branch!(
            '|',
            bar_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        just('=').ignore_then(
            choice((
                    tis_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                    dottis_irregular(hoon_wide.clone()), //  =(p q)
                    kettis_irregular(spec_wide.clone()).boxed(),  // =bar
                ))).boxed(),

        just('?').ignore_then(
            choice((
                wut_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                just('?').to(Hoon::Base(BaseType::Flag)).boxed(),
            ))
        ).boxed(),

        just('%').ignore_then(
        choice((
            cen_runes_wide(hoon_wide.clone()),
            just(".y").to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))),
            just(".n").to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))),
            just('|').to(Hoon::Rock("%f".to_string(), Noun::Atom("1".to_string()))),
            just('&').to(Hoon::Rock("%f".to_string(), Noun::Atom("0".to_string()))),
            nuck(true),
        ))).boxed(),

        just(':').ignore_then(
            choice((
                    col_runes_wide(hoon_wide.clone()),
                    miccol_irregular(hoon_wide.clone()).boxed(), //  :(a b .. z)
                ))).boxed(),

        just('~')
            .ignore_then(
            choice((
                    sig_runes_wide(hoon_wide.clone()),
                    censig_irregular(hoon_wide.clone()),  //  ~(a b c)
                    twid(),
                ))).boxed(),

        rune_branch!(
            '$',
            buc_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch!(
            '^',
            ket_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch!(
            '!',
            zap_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch!(
            ';',
            mic_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        just('.').ignore_then(
            choice((
                dot_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                float_sand(),
                just('y').to(Hoon::Sand("%f".to_string(), Noun::Atom("0".to_string()))),
                just('n').to(Hoon::Sand("%f".to_string(), Noun::Atom("1".to_string()))),
            ))).boxed(),

        just('`')
            .ignore_then(
                choice((
                    tic_aura(hoon_wide.clone()),                     //  `@p`q
                    kethep_irregular(hoon_wide.clone(),
                                    spec_wide.clone()).boxed(),       //  `p`q
                    ketlus_irregular(hoon_wide.clone()),              // `+p`q
                    tic_cell_construction(hoon_wide.clone()).boxed(), //  `a
                ))).boxed(),

        aura_hoon().boxed(),
        tape().boxed(),
        buccab_irregular(hoon_wide.clone()).boxed(),              //  _p
        constant_separator_hoon(hoon_wide.clone()).boxed(),       //  const+hoon,  const/hoon
        list_syntax(hoon_wide.clone()).boxed(),                   // [p ... pn], ~[foo], [foo]~
        kettar_irregular(spec_wide.clone()).boxed(),              //  *foo
        wutzap_irregular(hoon_wide.clone()).boxed(),              //  !p
        wutbar_irregular(hoon_wide.clone()).boxed(),              //  |(p q)
        wutpam_irregular(hoon_wide.clone()).boxed(),              //  &(p q)
        increment(hoon_wide.clone()).boxed(),          //  +(a) or .+(a)
        ketcol_irregular(spec_wide.clone()).boxed(),   //  ,p
        centis_irregular(hoon_wide.clone()).boxed(),   //  a(b c, d e, f g)
        tell(hoon_wide.clone()).boxed(),  // <foo>
        number_sand().boxed(),
        wing().boxed(),
        function_call(hoon_wide.clone()).boxed(),      //  (a b)
        constant().boxed(),
        cord().map(|s| Hoon::Sand("%t".to_string(), Noun::Atom(s))).boxed(),
        just('~').to(Hoon::Bust(BaseType::Null)).boxed(),
        path(hoon_wide.clone(), wer).boxed(),
        just('&').to(Hoon::Sand("%f".to_string(), Noun::Atom("0".to_string()))).boxed(),
        just('|').to(Hoon::Sand("%f".to_string(), Noun::Atom("1".to_string()))).boxed(),
        just('*').to(Hoon::Base(BaseType::Noun)).boxed(),
    ];

    choice(parsers).boxed().labelled("hoon-wide")
        .then(just('=').or(just(':')).or(just('^'))
                .then(hoon_wide.clone())
                .or_not())
        .map(|(p, maybe_separator)|  {
            match maybe_separator  {
                Some(('=', q)) => Hoon::KetTis(Box::new(p), Box::new(q)),
                Some((':', q)) => Hoon::TisGal(Box::new(p), Box::new(q)),
                Some(('^', q)) => Hoon::Pair(Box::new(p), Box::new(q)),
                _ => p,
            }
        })
}

fn hoon_parser<'src>(
    hoon: impl ParserExt<'src, Hoon>,
    hoon_wide: impl ParserExt<'src, Hoon>,
    spec: impl ParserExt<'src, Spec>,
    spec_wide: impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>>
{
    let parsers = vec![
        rune_branch_pair!(
            '|',
            bar_runes_tall(hoon.clone(), spec.clone()),
            bar_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch_pair!(
            '=',
            tis_runes_tall(hoon.clone(), spec.clone(), spec_wide.clone()),
            tis_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch_pair!(
            '?',
            wut_runes_tall(
                hoon.clone(),
                hoon_wide.clone(),
                spec.clone(),
                spec_wide.clone()
            ),
            wut_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch_pair!(
            '%',
            cen_runes_tall(hoon.clone()),
            cen_runes_wide(hoon_wide.clone())
        ),

        rune_branch_pair!(
            ':',
            col_runes_tall(hoon.clone()),
            col_runes_wide(hoon_wide.clone())
        ),

        rune_branch_pair!(
            '~',
            sig_runes_tall(hoon.clone()),
            sig_runes_wide(hoon_wide.clone())
        ),

        rune_branch_pair!(
            '$',
            buc_runes_tall(hoon.clone(), spec.clone()),
            buc_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch_pair!(
            '^',
            ket_runes_tall(hoon.clone(), spec.clone()),
            ket_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch_pair!(
            '!',
            zap_runes_tall(hoon.clone(), spec.clone()),
            zap_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch_pair!(
            ';',
            mic_runes_tall(hoon.clone(), spec.clone()),
            mic_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        rune_branch_pair!(
            '.',
            dot_runes_tall(hoon.clone(), spec.clone()),
            dot_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),
    ];

    choice(parsers)
        .labelled("hoon-tall")
        .boxed()
}

pub fn parser<'src>(
    // bug: bool,
    wer: PathBuf,
)
-> impl Parser<'src, &'src str, Hoon, Err<'src>> {
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
            wer.clone(),
        );

        spec_wide_handle.define(spec_wide_body);
        hoon_wide_handle.define(hoon_wide_body);

        let spec_wide = spec_wide_handle.clone();
        let hoon_wide = hoon_wide_handle.clone();

        choice((
            hoon_parser(hoon.clone(), hoon_wide.clone(), spec.clone(), spec_wide),
            hoon_wide
        )).labelled("Hoon")
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

    match parser(cli.input).parse(source.as_str()).into_result() {
        Ok(res) => {
            let took = start.elapsed();
            let json = serde_json::to_string_pretty(&res).expect("serialisation failed");
            let out_path = std::path::PathBuf::from("out.json");
            std::fs::write(&out_path, json + "\n").unwrap_or_else(|e| {
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
                    // .with_message(err.to_string())
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