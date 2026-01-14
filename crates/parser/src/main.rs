use clap::{Parser as ClapParser, command, arg};

use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::{
    input::{Stream, ValueInput, StrInput},
    prelude::*,
};

use std::fs;
use std::rc::Rc;
use std::collections::HashMap;
use std::time::Instant;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::cell::Cell;
use bytes::Bytes;
use nockapp::noun::slab::NounSlab;
use nockvm::noun::{D, T};
use nockvm_macros::tas;
use nockvm::noun::Atom;
use ibig::ubig;
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
    hoon:        impl ParserExt<'src, Hoon>,
    hoon_wide:        impl ParserExt<'src, Hoon>,
    spec:        impl ParserExt<'src, Spec>,
    spec_wide:   impl ParserExt<'src, Spec>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>> + Clone
{
    choice((
        rune_branch_pair!(
            "$",
            buc_spec_tall(hoon.clone(), spec.clone()),
            buc_spec_wide(hoon_wide.clone(),
                                        spec_wide.clone())
        ),
        rune_branch_pair!(
            "%",
            cen_spec_tall(hoon.clone(), spec.clone()),
            cen_spec_wide(hoon_wide.clone(), spec_wide.clone())
        ),
        spec_wide.clone(),
    ))
    .boxed()
}

fn spec_wide_parser<'src>(
    spec_wide:   impl ParserExt<'src, Spec>,
    hoon_wide:   impl ParserExt<'src, Hoon>,
    linemap: Arc<LineMap>,
) -> impl Parser<'src, &'src str, Spec, Err<'src>> + Clone
{
    let parsers = vec![
        just('$')
            .ignore_then(buc_spec_wide(hoon_wide.clone(),
                                        spec_wide.clone())).boxed(),
        buccab_spec_irregular(hoon_wide.clone()).boxed(),    //  _p
        bucmic_spec_irregular(hoon_wide.clone()).boxed(),    //  ,p0
        buctis_irregular(spec_wide.clone()).boxed(),         // foo=bar, =bar,  =foo=bar
        buccol_irregular(spec_wide.clone()).boxed(),         // [foo=bar foo=bar]
        reference_spec(spec_wide.clone()).boxed(),           // foo or foo:bar
        bucwut_irregular_spec(spec_wide.clone()).boxed(),    // ?(foo bar)
        parenthesis_spec(hoon_wide.clone(),
                                spec_wide.clone()).boxed(),  // (foo bar)
        constant(linemap)
        .try_map(|coin, span| {                             //  %foo
            match coin {
                Coin::Dime(p, q) => {
                    Ok(Spec::Leaf(p, q))
                }
                _ =>  Err(Rich::custom(span, "invalid spec constant")),
            }
        }).boxed(),
        aura_spec().boxed(),                                 //  @foo
        loop_spec().boxed(),                                 //  /foo
        just('^').to(Spec::Base(BaseType::Cell)).boxed(),
        just('?').to(Spec::Base(BaseType::Flag)).boxed(),
        just('~').to(Spec::Base(BaseType::Null)).boxed(),
        just('*').to(Spec::Base(BaseType::NounExpr)).boxed(),
        just("!!").to(Spec::Base(BaseType::Void)).boxed(),
    ];

    choice(parsers).boxed()
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
enum WideOp {
    KetTis,
    TisGal,
    Pair,
}

fn hoon_wide_parser<'src>(
    hoon:        impl ParserExt<'src, Hoon>,
    hoon_wide:   impl ParserExt<'src, Hoon>,
    spec_wide:   impl ParserExt<'src, Spec>,
    hoon_wide_with_trace: impl ParserExt<'src, Hoon>,
    hoon_wide_no_trace:   impl ParserExt<'src, Hoon>,
    wer: Path,
    linemap: Arc<LineMap>,
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
                bucwut_irregular(spec_wide.clone()).boxed(),   // ?(foo bar)
                just('?').to(Hoon::Base(BaseType::Flag)).boxed(),
            ))
        ).boxed(),

        just('%')
        .ignore_then(
        choice((
            cen_runes_wide(hoon_wide.clone()),
            just('|').to(Hoon::Rock("f".to_string(), NounExpr::ParsedAtom(ParsedAtom::Small(1)))),
            just('&').to(Hoon::Rock("f".to_string(), NounExpr::ParsedAtom(ParsedAtom::Small(0)))),
            nuck().map(|coin| jock(true, &coin)),
        ))).boxed(),

        just(':').ignore_then(
            choice((
                    col_runes_wide(hoon_wide.clone()),
                    miccol_irregular(hoon_wide.clone()).boxed(),     //  :(a b .. z)
                ))).boxed(),

        just('~')
            .ignore_then(
            choice((
                    sig_runes_wide(hoon_wide.clone()),
                    censig_irregular(hoon_wide.clone()),              //  ~(a b c)
                    twid().map(|coin| jock(false, &coin)),
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
            zap_runes_wide(hoon_wide.clone(),
                            spec_wide.clone(),
                            hoon_wide_with_trace.clone(),
                            hoon_wide_no_trace.clone())
        ),

        rune_branch!(
            ';',
            mic_runes_wide(hoon_wide.clone(), spec_wide.clone())
        ),

        just('.').ignore_then(
            choice((
                dot_runes_wide(hoon_wide.clone(), spec_wide.clone()),
                perd().map(|coin| jock(false, &coin)),
            ))).boxed(),

        just('`')
            .ignore_then(
                choice((
                    tic_aura(hoon_wide.clone()),                              //  `@p`q
                    kethep_irregular(hoon_wide.clone(),
                                    spec_wide.clone()).boxed(),               //  `p`q
                    ketlus_irregular(hoon_wide.clone()),                      // `+p`q
                    tic_cell_construction(hoon_wide.clone()).boxed(),         //  `a
                ))).boxed(),

        function_call(hoon_wide.clone()).boxed(),                             //  (a b)
        centis_irregular(hoon_wide.clone()).boxed(),                          //  a(b c, d e, f g)
        aura_hoon().boxed(),
        buccab_irregular(hoon_wide.clone()).boxed(),                          //  _p
        constant_separator_hoon(hoon_wide.clone()).boxed(),                   //  const+hoon,  const/hoon
        list_syntax(hoon.clone(), hoon_wide.clone()).boxed(),                 // [p ... pn], ~[foo], [foo]~
        kettar_irregular(spec_wide.clone()).boxed(),                          //  *foo
        wutzap_irregular(hoon_wide.clone()).boxed(),                          //  !p
        wutbar_irregular(hoon_wide.clone()).boxed(),                          //  |(p q)
        wutpam_irregular(hoon_wide.clone()).boxed(),                          //  &(p q)
        increment(hoon_wide.clone()).boxed(),                                 //  +(a) or .+(a)
        ketcol_irregular(spec_wide.clone()).boxed(),                          //  ,p
        tell(hoon_wide.clone()).boxed(),                                      // <foo> render as tape
        yell_parser(hoon_wide.clone()).boxed(),                               // >foo< render as tank
        number().map(|(p, q)| Hoon::Sand(p, NounExpr::ParsedAtom(q))).boxed(),//  111.111, 0x1111, etc.
        wing().boxed(),                                                       //   foo, foo.bar, etc.
        constant(linemap.clone()).map(|coin| jock(true, &coin)).boxed(),      //  %foo
        cord(linemap.clone())
            .map(|s| Hoon::Sand("t".to_string(), NounExpr::ParsedAtom(s))).boxed(), //  'foo'
        path(hoon_wide.clone(), wer, linemap.clone()).boxed(),                      //  /a/b/c
        tape(hoon_wide.clone(), linemap).boxed(),                                   //  "foo"
        just('~').to(Hoon::Bust(BaseType::Null)).boxed(),
        just('&').to(Hoon::Sand("f".to_string(), NounExpr::ParsedAtom(ParsedAtom::Small(0)))).boxed(),
        just('|').to(Hoon::Sand("f".to_string(), NounExpr::ParsedAtom(ParsedAtom::Small(1)))).boxed(),
        just('*').to(Hoon::Base(BaseType::NounExpr)).boxed(),
    ];

    choice(parsers).boxed()
    .then(choice((just('=').to(WideOp::KetTis),
            just(':').to(WideOp::TisGal),
            just('^').to(WideOp::Pair)))
        .then(hoon_wide.clone())
        .or_not())
    .try_map(|(p, maybe_separator), span| {
            match maybe_separator  {
                Some((WideOp::KetTis, q)) => {
                    let maybe_skin = flay(p);
                    match maybe_skin {
                        None => Err(Rich::custom(span, "invalid p in p=q")),
                        Some(s) => Ok(Hoon::KetTis(s, Box::new(q))),
                    }
                },
                Some((WideOp::TisGal, q)) => Ok(Hoon::TisGal(Box::new(p), Box::new(q))),
                Some((WideOp::Pair, q)) => Ok(Hoon::Pair(Box::new(p), Box::new(q))),
                None => Ok(p),
            }
        })
}

pub fn hoon_parser<'src>(
    hoon: impl ParserExt<'src, Hoon>,
    hoon_wide: impl ParserExt<'src, Hoon>,
    spec: impl ParserExt<'src, Spec>,
    spec_wide: impl ParserExt<'src, Spec>,
    hoon_with_trace: impl ParserExt<'src, Hoon>,
    hoon_no_trace: impl ParserExt<'src, Hoon>,
    hoon_wide_with_trace: impl ParserExt<'src, Hoon>,
    hoon_wide_no_trace: impl ParserExt<'src, Hoon>,
    wer: Path,
    linemap: Arc<LineMap>,
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
                zap_runes_tall(hoon.clone(), spec.clone(), hoon_with_trace.clone(), hoon_no_trace.clone()),
                zap_runes_wide(hoon_wide.clone(),
                                spec_wide.clone(),
                                hoon_wide_with_trace.clone(),
                                hoon_wide_no_trace.clone())
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

            just('/')  // skip imports...
                .ignore_then(fas_runes_tall(hoon.clone(),
                                            hoon_wide.clone(),
                                            wer.clone(),
                                            linemap.clone()))
                                            .boxed(),

            hoon_wide.clone().boxed(),

            noun_tall(hoon.clone()).boxed(),
        ];

    choice(parsers)
}

pub fn parser<'src>(
    wer: Path,
    bug: bool,
    linemap: Arc<LineMap>,
) -> impl Parser<'src, &'src str, Hoon, Err<'src>> {

    let mut hoon                = Recursive::declare();
    let mut hoon_wide           = Recursive::declare();
    let mut spec                = Recursive::declare();
    let mut spec_wide           = Recursive::declare();

    let mut hoon_no_trace       = Recursive::declare();
    let mut hoon_wide_no_trace  = Recursive::declare();
    let mut spec_no_trace       = Recursive::declare();
    let mut spec_wide_no_trace  = Recursive::declare();

    let spec_body = spec_parser(hoon.clone(),
                                hoon_wide.clone(),
                                spec.clone(),
                                spec_wide.clone())
                                .map_with(wrap_spec_with_trace(wer.clone(), linemap.clone()))
                                .labelled("Spec")
                                .boxed();

    spec.define(spec_body);

    let spec_wide_body =
            spec_wide_parser(spec_wide.clone(),
                             hoon_wide.clone(),
                             linemap.clone())
                            .map_with(wrap_spec_with_trace(wer.clone(), linemap.clone()))
                            .labelled("Spec Wide")
                            .boxed();

    spec_wide.define(spec_wide_body);

    let hoon_wide_body = hoon_wide_parser(
                                hoon.clone(),
                                hoon_wide.clone(),
                                spec_wide.clone(),
                                hoon_wide.clone(),
                                hoon_wide_no_trace.clone(),
                                wer.clone(),
                                linemap.clone(),
                            )
                            .map_with(wrap_hoon_with_trace(wer.clone(), linemap.clone()))
                            .labelled("Hoon Wide")
                            .boxed();

    hoon_wide.define(hoon_wide_body);

    let hoon_body =
            hoon_parser(hoon.clone(),
                        hoon_wide.clone(),
                        spec.clone(),
                        spec_wide.clone(),
                        hoon.clone(),
                        hoon_no_trace.clone(),
                        hoon_wide.clone(),
                        hoon_wide_no_trace.clone(),
                        wer.clone(),
                        linemap.clone(),
                        )
                        .map_with(wrap_hoon_with_trace(wer.clone(), linemap.clone()))
                        .labelled("Hoon")
                        .boxed();

    hoon.define(hoon_body);

    let hoon_no_trace_body =
            hoon_parser(hoon_no_trace.clone(),
                        hoon_wide_no_trace.clone(),
                        spec_no_trace.clone(),
                        spec_wide_no_trace.clone(),
                        hoon.clone(),
                        hoon_no_trace.clone(),
                        hoon_wide.clone(),
                        hoon_wide_no_trace.clone(),
                        wer.clone(),
                        linemap.clone())
                        .labelled("Hoon")
                        .boxed();

    hoon_no_trace.define(hoon_no_trace_body);

    let hoon_wide_no_trace_body
                    = hoon_wide_parser(
                                        hoon_no_trace.clone(),
                                        hoon_wide_no_trace.clone(),
                                        spec_wide_no_trace.clone(),
                                        hoon_wide.clone(),
                                        hoon_wide_no_trace.clone(),
                                        wer.clone(),
                                        linemap.clone(),
                                    )
                                    .labelled("Hoon Wide")
                                    .boxed();

    hoon_wide_no_trace.define(hoon_wide_no_trace_body);

    let spec_body_no_trace = spec_parser(hoon_no_trace.clone(),
                                         hoon_wide_no_trace.clone(),
                                         spec_no_trace.clone(),
                                         spec_wide_no_trace.clone())
                                        .labelled("Spec")
                                        .boxed();

    spec_no_trace.define(spec_body_no_trace);

    let spec_wide_no_trace_body =
            spec_wide_parser(spec_wide_no_trace.clone(),
                             hoon_wide_no_trace.clone(),
                            linemap)
                            .labelled("Spec Wide")
                             .boxed();

    spec_wide_no_trace.define(spec_wide_no_trace_body);

    let hoon = if bug { hoon } else { hoon_no_trace };

    hoon
    .separated_by(gap())
    .at_least(1)
    .collect::<Vec<Hoon>>()
    .map(|hoons| Hoon::TisSig(hoons))
    .delimited_by(gap().or_not(), gap().or_not())
    .boxed()
}

#[cfg(not(feature = "bazel_build"))]
pub static HOON138JAM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test/parsed-hoon138.jam"
));

#[derive(ClapParser, Debug)]
struct Cli {
    /// input file or directory (required unless --test)
    #[arg(value_name = "PATH", required = false)]
    input: Option<PathBuf>,

    /// disable debug traces
    #[arg(long = "no-dbug", short = 'b')]
    no_dbug: bool,

    /// write JAM instead of JSON
    #[arg(long = "jam")]
    jam: bool,

    /// output file (defaults to stdout)
    #[arg(long = "out", short = 'o', value_name = "PATH")]
    out: Option<PathBuf>,

    /// run hardcoded hoon-138 test
    #[arg(long = "test")]
    test: bool,
}

fn run_test() {
    let source_path = PathBuf::from("../hoonc/hoon/hoon-138.hoon");

    let source = fs::read_to_string(&source_path).unwrap();
    let linemap = Arc::new(LineMap::new(&source));

    let wer = vec![
        "hoonc".to_string(),
        "hoon".to_string(),
        "hoon-138".to_string(),
        "hoon".to_string(),
    ];

    let start = Instant::now();

    match parser(wer, false, linemap)
        .parse(source.as_str())
        .into_result()
    {
        Ok(res) => {
            let end = start.elapsed();

            let mut slab = NounSlab::new();
            let parsed_hoon = hoon_to_noun(&mut slab, &res);
            let jammed = Bytes::from(HOON138JAM);
            let cued = slab.cue_into(jammed).unwrap();

            diff_and_report(&cued, &parsed_hoon);

            println!("test parsing took: {:?}", end);
        }
        Err(errs) => {
             for err in errs {
                let span = err.span().into_range();
                let file_id = source_path.to_string_lossy().to_string();

                Report::build(ReportKind::Error, (file_id.clone(), span.clone()))
                    .with_config(
                        ariadne::Config::new()
                            .with_index_type(ariadne::IndexType::Byte),
                    )
                    .with_label(
                        Label::new((file_id.clone(), span))
                            .with_message(err.reason().to_string())
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint((file_id.clone(), Source::from(source.clone())))
                    .unwrap();
            }
        }
    };
}

fn run_parser(source_path: &PathBuf, jam: bool, dbug: bool, out: Option<PathBuf>) {

    let source = fs::read_to_string(source_path).unwrap_or_else(|err| {
        eprintln!("Error reading file '{}': {}", source_path.display(), err);
        std::process::exit(1);
    });

    let start = Instant::now();

    let wer: Vec<String> = source_path
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();

    let linemap = Arc::new(LineMap::new(&source));

    match parser(wer, dbug, linemap)
        .parse(source.as_str())
        .into_result()
    {
        Ok(res) => {
            let took = start.elapsed();

            let mut slab = NounSlab::new();
            let start2 = Instant::now();
            let parsed_hoon = hoon_to_noun(&mut slab, &res);
            let took2 = start2.elapsed();

            if jam {
                slab.set_root(parsed_hoon);
                let jammed = slab.jam();

                match &out {
                    Some(out) if out.is_dir() => {
                        let out_file = out.join(
                            source_path.file_name().unwrap()
                        );
                        fs::write(out_file, &jammed).unwrap();
                    }
                    Some(out) => fs::write(out, &jammed).unwrap(),
                    None => std::io::stdout().write_all(&jammed).unwrap(),
                }
            } else {
                let json = serde_json::to_string_pretty(&res)
                    .expect("AST JSON serialization failed");

                match &out {
                    None => {
                        println!("{json}");
                    }
                    Some(out) if out.is_dir() => {
                        let mut out_file = out.join(
                            source_path
                                .file_name()
                                .expect("input has no filename"),
                        );
                        out_file.set_extension("json");
                        fs::write(&out_file, json).unwrap_or_else(|e| {
                            eprintln!("Failed to write '{}': {}", out_file.display(), e);
                            std::process::exit(1);
                        });
                    }
                    Some(out) => {
                        fs::write(out, json).unwrap_or_else(|e| {
                            eprintln!("Failed to write '{}': {}", out.display(), e);
                            std::process::exit(1);
                        });
                    }
                }
            }

            println!(
                "parsed file {}, took {:?}, noun creation time {:?}",
                source_path.display(),
                took,
                took2
            );
        }

        Err(errs) => {
            for err in errs {
                let span = err.span().into_range();
                let file_id = source_path.to_string_lossy().to_string();

                Report::build(ReportKind::Error, (file_id.clone(), span.clone()))
                    .with_config(
                        ariadne::Config::new()
                            .with_index_type(ariadne::IndexType::Byte),
                    )
                    .with_label(
                        Label::new((file_id.clone(), span))
                            .with_message(err.reason().to_string())
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint((file_id.clone(), Source::from(source.clone())))
                    .unwrap();
            }
        }
    };
}

fn main() {
    let cli = Cli::parse();

    if cli.test {
        run_test();
        return;
    }

    let input = cli.input.clone().unwrap_or_else(|| {
        eprintln!("Input file or directory is required unless --test");
        std::process::exit(2);
    });

    let inputs = collect_inputs(&input);

    let start = Instant::now();

    for source_path in inputs {
        run_parser(&source_path, cli.jam, !cli.no_dbug, cli.out.clone());
    }

    println!("total running time {:?} ", start.elapsed());
}