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
        just(Token::Cab).ignore_then(buccab(hoon.clone())),
        just(Token::Gar).ignore_then(bucgal(spec.clone())),
        just(Token::Gal).ignore_then(bucgar(spec.clone())),
        just(Token::Bar).ignore_then(bucbar(hoon.clone(), spec.clone())),
        just(Token::Pam).ignore_then(bucpam(hoon.clone(), spec.clone())),
        just(Token::Hep).ignore_then(buchep(spec.clone())),
        just(Token::Mic).ignore_then(bucmic(hoon.clone())),
        just(Token::Tis).ignore_then(buctis(spec.clone())),
        just(Token::Wut).ignore_then(bucwut(spec.clone())),
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
        just(Token::Cen).ignore_then(buccen_wide(spec_wide.clone())),
        just(Token::Wut).ignore_then(bucwut_wide(spec_wide.clone())),
        just(Token::Pat).ignore_then(bucpat_wide(spec_wide.clone())),
        just(Token::Col).ignore_then(buccol_wide(spec_wide.clone())),
        just(Token::Lus).ignore_then(buclus_wide(spec_wide.clone())),
        just(Token::Ket).ignore_then(bucket_wide(spec_wide.clone())),
        just(Token::Sig).ignore_then(bucsig_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Cab).ignore_then(buccab_wide(hoon_wide.clone())),
        just(Token::Gal).ignore_then(bucgal_wide(spec_wide.clone())),
        just(Token::Gar).ignore_then(bucgar_wide(spec_wide.clone())),
        just(Token::Bar).ignore_then(bucbar_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Pam).ignore_then(bucpam_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Hep).ignore_then(buchep_wide(spec_wide.clone())),
        just(Token::Mic).ignore_then(bucmic_wide(hoon_wide.clone())),
        just(Token::Tis).ignore_then(buctis_wide(spec_wide.clone())),
        just(Token::Wut).ignore_then(bucwut_wide(spec_wide.clone())),
    ))
}

pub fn buc_spec_tall<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
    // spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Cen).ignore_then(buccen_spec(spec.clone())),
        just(Token::Bar).ignore_then(bucbar_spec(hoon.clone(), spec.clone())),
        just(Token::Pat).ignore_then(bucpat_spec(spec.clone())),
        just(Token::Wut).ignore_then(bucwut_spec(spec.clone())),
        just(Token::Tis).ignore_then(buctis_spec(spec.clone())),
        just(Token::Lus).ignore_then(buclus_spec(spec.clone())),
        just(Token::Ket).ignore_then(bucket_spec(spec.clone())),
        just(Token::Col).ignore_then(buccol_spec(spec.clone())),
        just(Token::Sig).ignore_then(bucsig_spec(hoon.clone(), spec.clone())),
        just(Token::Mic).ignore_then(bucmic_spec(hoon.clone())),
        just(Token::Pam).ignore_then(bucpam_spec(hoon.clone(), spec.clone())),
        just(Token::Cab).ignore_then(buccab_spec(hoon.clone())),
        just(Token::Gal).ignore_then(bucgal_spec(spec.clone())),
        just(Token::Gar).ignore_then(bucgar_spec(spec.clone())),
    )).boxed()
}

pub fn buc_spec_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
    // spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + Clone + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    choice((
        just(Token::Pat).ignore_then(bucpat_spec_wide(spec_wide.clone())),
        just(Token::Sig).ignore_then(bucsig_spec_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Hep).ignore_then(buchep_spec_wide(spec_wide.clone())),
        just(Token::Lus).ignore_then(buclus_spec_wide(spec_wide.clone())),
        just(Token::Cen).ignore_then(buccen_spec_wide(spec_wide.clone())),
        just(Token::Wut).ignore_then(bucwut_spec_wide(spec_wide.clone())),
        just(Token::Mic).ignore_then(bucmic_spec_wide(hoon_wide.clone())),
        just(Token::Bar).ignore_then(bucbar_spec_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Pam).ignore_then(bucpam_spec_wide(hoon_wide.clone(), spec_wide.clone())),
        just(Token::Cab).ignore_then(buccab_spec_wide(hoon_wide.clone())),
        just(Token::Tis).ignore_then(buctis_spec_wide(spec_wide.clone())),
        just(Token::Col).ignore_then(buccol_spec_wide(spec_wide.clone())),
        just(Token::Gal).ignore_then(bucgal_spec_wide(spec_wide.clone())),
        just(Token::Gar).ignore_then(bucgar_spec_wide(spec_wide.clone())),
        just(Token::Ket).ignore_then(bucket_spec_wide(spec_wide.clone())),
    )).boxed()
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
    two_specs_tall(spec.clone())
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucKet(Box::new(p), Box::new(q)))))
}

pub fn bucket_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucket_spec_wide(spec_wide.clone())
    .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn bucket_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_closed_wide(spec_wide.clone())
    .map(|(p, q)| Spec::BucKet(Box::new(p), Box::new(q)))
}

pub fn bucpam<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucpam_spec(hoon.clone(), spec.clone())
    .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn bucpam_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_hoon_tall(hoon.clone(), spec.clone())
    .map(|(p, q)| Spec::BucPam(Box::new(p), q))
}

pub fn buclus<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    name_spec_tall(spec.clone())
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucLus(p, Box::new(q)))))
}

pub fn buclus_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    name_spec_wide(spec_wide.clone())
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
        list_spec_closed_wide(spec_wide.clone())
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
    hoon_spec_wide(hoon_wide.clone(), spec_wide.clone())
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
    two_specs_tall(spec.clone())
    .map(|(p, q)| {
            Hoon::KetCol(Box::new(Spec::BucPat(
                                        Box::new(p),
                                        Box::new(q))))
        })
}

pub fn buccab<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    buccab_spec(hoon.clone())
    .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn buccab_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|p| Spec::BucCab(p))
}

pub fn buccab_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    buccab_spec_wide(hoon_wide.clone())
    .map(|s| Hoon::KetCol(Box::new(s)))
}

pub fn buccab_spec_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    one_hoon_closed_wide(hoon_wide.clone())
    .map(|p| Spec::BucCab(p))
}

pub fn bucgar<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucgar_spec(spec.clone())
    .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn bucgar_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_tall(spec.clone())
    .map(|(p, q)| {
        Spec::BucGar(Box::new(p), Box::new(q))
    })
}

pub fn bucgar_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucgar_spec_wide(spec_wide.clone())
    .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn bucgar_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_closed_wide(spec_wide.clone())
    .map(|(p, q)| Spec::BucGar(Box::new(p), Box::new(q)))
}

pub fn bucgal<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucgal_spec(spec.clone())
    .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn bucgal_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_tall(spec.clone())
    .map(|(p, q)| {
         Spec::BucGal(Box::new(p), Box::new(q))
    })
}

pub fn bucgal_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucgal_spec_wide(spec_wide.clone())
    .map(|p| Hoon::KetCol(Box::new(p)))
}

pub fn bucgal_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_closed_wide(spec_wide.clone())
    .map(|(p, q)| Spec::BucGal(Box::new(p), Box::new(q)))
}

pub fn buchep<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_tall(spec.clone())
    .map(|(p, q)| {
        Hoon::KetCol(Box::new(
                Spec::BucHep(Box::new(p), Box::new(q))
        ))
    })
}

pub fn buchep_wide<'tokens, 'src: 'tokens, I>(
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_closed_wide(spec_wide.clone())
        .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucHep(Box::new(p), Box::new(q)))))
}

pub fn bucpam_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide: impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide: impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_hoon_wide(hoon_wide.clone(), spec_wide.clone())
        .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucPam(Box::new(p), q))))
}

pub fn buccol<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    list_spec_closed_tall(spec.clone())
    .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Hoon::KetCol(Box::new(Spec::BucCol(
                            Box::new(first.clone()), rest.to_vec())))
        })
}

pub fn buccol_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    buccol_spec_wide(spec_wide.clone())
    .map(|s| { Hoon::KetCol(Box::new(s))})
}

pub fn buccol_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    list_spec_closed_wide(spec_wide.clone())
    .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Spec::BucCol(Box::new(first.clone()), rest.to_vec())
        })
}

pub fn bucsig<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_spec_tall(hoon.clone(), spec.clone())
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucSig(p, Box::new(q)))))
}

pub fn buccen<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    buccen_spec(spec.clone())
    .map(|s| Hoon::KetCol(Box::new(s)))
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

pub fn buccab_spec_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Cab)
        .ignore_then(hoon_wide.clone())
        .map(|h| Spec::BucCab(h))
}

pub fn bucbar<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucbar_spec(hoon.clone(), spec.clone())
    .map(|s| Hoon::KetCol(Box::new(s)))
}

pub fn bucbar_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_hoon_wide(hoon_wide.clone(), spec_wide.clone())
    .map(|(p, q)| Hoon::KetCol(Box::new(Spec::BucBar(Box::new(p), q))))
}

pub fn bucbar_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_hoon_tall(hoon.clone(), spec.clone())
    .map(|(p, q)| Spec::BucBar(Box::new(p), q))
}

pub fn bucmic<'tokens, 'src: 'tokens, I>(
    hoon:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon.clone()
    .map(|h| Hoon::KetCol(Box::new(Spec::BucMic(h))))
}

pub fn bucmic_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    bucmic(hoon_wide.clone())
}

pub fn bucmic_spec<'tokens, 'src: 'tokens, I>(
    hoon:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    gap()
    .ignore_then(hoon.clone())
    .map(|h| Spec::BucMic(h))
}

pub fn bucmic_spec_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    one_hoon_closed_wide(hoon_wide.clone())
    .map(|h| Spec::BucMic(h))
}

pub fn bucmic_spec_irregular<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just(Token::Com)
        .ignore_then(hoon_wide.clone())
        .map(|h| Spec::BucMic(h))
}

pub fn bucket_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_tall(spec.clone())
    .map(|(p, q)| Spec::BucKet(Box::new(p), Box::new(q)))
}

pub fn buclus_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    name_spec_tall(spec.clone())
    .map(|(p, q)| Spec::BucLus(p, Box::new(q)))
}

pub fn bucwut_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    just([Token::Wut, Token::Pal])
        .ignore_then(spec_wide.clone()
              .separated_by(just(Token::Ace))
              .at_least(1)
              .collect::<Vec<_>>()
            )
        .then_ignore(just(Token::Par))
        .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Spec::BucWut(Box::new(first.clone()), rest.to_vec())
        })
}

pub fn buctis<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    name_spec_tall(spec.clone())
    .map(|(name, s)| {
        Hoon::KetCol(Box::new(
            Spec::BucTis(Skin::Term(name), Box::new(s))
        ))
    })
}

pub fn buctis_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Hoon, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    buctis_spec_wide(spec_wide.clone())
    .map(|s| {
        Hoon::KetCol(Box::new(s))
    })
}

pub fn buctis_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    name_spec_wide(spec_wide.clone())
    .map(|(name, s)| {
            Spec::BucTis(Skin::Term(name), Box::new(s))
    })
}

pub fn buctis_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    name_spec_tall(spec.clone())
    .map(|(name, s)| { Spec::BucTis(Skin::Term(name), Box::new(s))})
}

pub fn bucwut_spec<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
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
        Spec::BucWut(Box::new(first.clone()), rest.to_vec())
    })
}

pub fn bucwut_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
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
            Spec::BucWut(Box::new(first.clone()), rest.to_vec())
        })
}

pub fn buctis_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Name(n) => n }  //  foo=bar
        .then_ignore(just(Token::Tis))
        .then(spec_wide.clone())
        .map(|(n, s)| Spec::BucTis(Skin::Term(n.to_string()), Box::new(s)))
        .or(
            just(Token::Tis)
            .ignore_then(select! { Token::Name(n) => n }  // =foo=bar
                            .then_ignore(just(Token::Tis))
                            .then(spec_wide.clone())
                            .map(|(name, spec)| (Some(name), spec))
                        .or(spec_wide.clone()
                            .map(|spec| (None, spec)))      //   =bar
                        .try_map(|(name, spec), span| {
                            let auto = autoname(spec.clone());
                            match auto {
                                // None => Err(Cheap::new(span).into()),
                                None => Err(Rich::custom(span, "cannot autoname")),
                                Some(auto_term) => {
                                    let term = match name {
                                        None => auto_term.to_string(),
                                        Some(n) => {
                                            let new_name = format!("{}-{}", n, auto_term);
                                            new_name
                                        }
                                    };
                                    Ok(Spec::BucTis(Skin::Term(term), Box::new(spec.clone())))
                                }
                            }
                        })
                    )
        )
}

pub fn buccol_irregular<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>> + 'tokens
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_wide.clone()
    .separated_by(just(Token::Ace))
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Sel), just(Token::Ser))
    .map(|specs| {
        let (first, rest) = specs.split_first().unwrap();
        Spec::BucCol(Box::new(first.clone()), rest.to_vec())
    })
}

pub fn bucsig_spec_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_spec_wide( hoon_wide.clone(), spec_wide.clone())
    .map(|(h, s)| Spec::BucSig(h, Box::new(s)))
}

pub fn buclus_spec_wide<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! { Token::Name(s) => s.to_string() }
    .then_ignore(just(Token::Ace))
    .then(spec.clone())
    .delimited_by(just(Token::Pal), just(Token::Par))
    .map(|(p, q)| Spec::BucLus(p, Box::new(q)))
}

pub fn buchep_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_closed_wide(spec_wide.clone())
    .map(|(p, q)| Spec::BucHep(Box::new(p), Box::new(q)))
}

pub fn bucpat_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_tall(spec.clone())
    .map(|(p, q)| Spec::BucPat(Box::new(p), Box::new(q)))
}

pub fn buccol_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
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
            Spec::BucCol(Box::new(first.clone()), rest.to_vec())
        })
}

pub fn bucsig_spec<'tokens, 'src: 'tokens, I>(
    hoon:        impl ParserExt<'tokens, 'src, I, Hoon>,
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    hoon_spec_tall(hoon.clone(), spec.clone())
    .map(|(p, q)| Spec::BucSig(p, Box::new(q)))
}

pub fn buccen_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
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
            Spec::BucCen(
                            Box::new(first.clone()), rest.to_vec())
        })
}

pub fn buccen_spec<'tokens, 'src: 'tokens, I>(
    spec:        impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    list_spec_closed_tall(spec.clone())
    .map(|specs| {
            let (first, rest) = specs.split_first().unwrap();
            Spec::BucCen(Box::new(first.clone()), rest.to_vec())
        })
}

pub fn bucpat_spec_wide<'tokens, 'src: 'tokens, I>(
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    two_specs_closed_wide(spec_wide.clone())
    .map(|(p, q)| Spec::BucPat(Box::new(p), Box::new(q)))
}

pub fn bucbar_spec_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_hoon_wide(hoon_wide.clone(), spec_wide.clone())
    .map(|(p, q)| Spec::BucBar(Box::new(p), q))
}

pub fn bucpam_spec_wide<'tokens, 'src: 'tokens, I>(
    hoon_wide:   impl ParserExt<'tokens, 'src, I, Hoon>,
    spec_wide:   impl ParserExt<'tokens, 'src, I, Spec>,
) -> impl Parser<'tokens, I, Spec, Err<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    spec_hoon_wide(hoon_wide.clone(), spec_wide.clone())
    .map(|(p, q)| Spec::BucPam(Box::new(p), q))
}