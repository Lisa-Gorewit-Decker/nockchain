use logos::Logos;
use std::fmt;

#[derive(Logos, Debug, PartialEq, Clone)]
pub enum Token<'a> {
    LexerError,

    #[token("|$")]
    BarBuc,
    #[token("|_")]
    BarCab,
    #[token("|:")]
    BarCol,
    #[token("|%")]
    BarCen,
    #[token("|.")]
    BarDot,
    #[token("|^")]
    BarKet,
    #[token("|-")]
    BarHep,
    #[token("|~")]
    BarSig,
    #[token("|*")]
    BarTar,
    #[token("|=")]
    BarTis,
    #[token("|@")]
    BarPat,
    #[token("|?")]
    BarWut,
    #[token("|?(")]
    BarWutWide,
    #[token("|$(")]
    BarBucWide,
    #[token("|:(")]
    BarColWide,
    #[token("|.(")]
    BarDotWide,
    #[token("|-(")]
    BarHepWide,
    #[token("|~(")]
    BarSigWide,
    #[token("|*(")]
    BarTarWide,
    #[token("|=(")]
    BarTisWide,

    #[token("^|")]
    KetBar,
    #[token("^:")]
    KetCol,
    #[token("^.")]
    KetDot,
    #[token("^-")]
    KetHep,
    #[token("^+")]
    KetLus,
    #[token("^&")]
    KetPam,
    #[token("^~")]
    KetSig,
    #[token("^*")]
    KetTar,
    #[token("^=")]
    KetTis,
    #[token("^?")]
    KetWut,
    #[token("^|(")]
    KetBarWide,
    #[token("^:(")]
    KetColWide,
    #[token("^.(")]
    KetDotWide,
    #[token("^-(")]
    KetHepWide,
    #[token("^+(")]
    KetLusWide,
    #[token("^&(")]
    KetPamWide,
    #[token("^~(")]
    KetSigWide,
    #[token("^*(")]
    KetTarWide,
    #[token("^=(")]
    KetTisWide,
    #[token("^?(")]
    KetWutWide,

    #[token("?|")]
    WutBar,
    #[token("?-")]
    WutHep,
    #[token("?:")]
    WutCol,
    #[token("?.")]
    WutDot,
    #[token("?^")]
    WutKet,
    #[token("?<")]
    WutGal,
    #[token("?>")]
    WutGar,
    #[token("?+")]
    WutLus,
    #[token("?&")]
    WutPam,
    #[token("?@")]
    WutPat,
    #[token("?~")]
    WutSig,
    #[token("?=")]
    WutTis,
    #[token("?!")]
    WutZap,
    #[token("?-(")]
    WutHepWide,
    #[token("?|(")]
    WutBarWide,
    #[token("?:(")]
    WutColWide,
    #[token("?.(")]
    WutDotWide,
    #[token("?^(")]
    WutKetWide,
    #[token("?<(")]
    WutGalWide,
    #[token("?>(")]
    WutGarWide,
    #[token("?+(")]
    WutLusWide,
    #[token("?&(")]
    WutPamWide,
    #[token("?@(")]
    WutPatWide,
    #[token("?~(")]
    WutSigWide,
    #[token("?=(")]
    WutTisWide,
    #[token("?!(")]
    WutZapWide,

    #[token("|(")]
    WutBarIrregular,
    #[token("&(")]
    WutPamIrregular,

    #[token("=>")]
    TisGar,
    #[token("=|")]
    TisBar,
    #[token("=:")]
    TisCol,
    #[token("=,")]
    TisCom,
    #[token("=.")]
    TisDot,
    // #[token("=-")]
    // TisHep,
    #[token("=^")]
    TisKet,
    #[token("=<")]
    TisGal,
    #[token("=+")]
    TisLus,
    #[token("=;")]
    TisMic,
    #[token("=/")]
    TisFas,
    // #[token("=~")]
    // TisSig,
    // #[token("=*")]
    // TisTar,
    // #[token("=?")]
    // TisWut,
    #[token("=>(")]
    TisGarWide,

    #[token("%_")]
    CenCab,
    #[token("%:")]
    CenCol,
    #[token("%.")]
    CenDot,
    #[token("%-")]
    CenHep,
    #[token("%^")]
    CenKet,
    #[token("%+")]
    CenLus,
    #[token("%~")]
    CenSig,
    #[token("%*")]
    CenTar,
    #[token("%=")]
    CenTis,

    #[token("+|")]
    LusBar,
    #[token("+$")]
    LusBuc,
    #[token("++")]
    LusLus,
    #[token("+*")]
    LusTar,

    #[token("$|")]
    BucBar,
    #[token("$_")]
    BucCab,
    #[token("$%")]
    BucCen,
    // #[token("$:")]
    // BucCol,
    #[token("$<")]
    BucGal,
    #[token("$>")]
    BucGar,
    #[token("$-")]
    BucHep,
    #[token("$^")]
    BucKet,
    #[token("$+")]
    BucLus,
    #[token("$&")]
    BucPam,
    #[token("$~")]
    BucSig,
    #[token("$@")]
    BucPat,
    #[token("$=")]
    BucTis,
    #[token("$?")]
    BucWut,
    #[token("$?(")]
    BucWutWide,
    #[token("$@(")]
    BucPatWide,
    #[token("$-(")]
    BucHepWide,
    #[token("$+(")]
    BucLusWide,
    #[token("$~(")]
    BucSigWide,

    #[token(":-")]
    ColHep,
    #[token(":_")]
    ColCab,
    // #[token(":+")]
    // ColLus,
    #[token(":^")]
    ColKet,
    #[token(":*")]
    ColTar,
    #[token(":~")]
    ColSig,
    #[regex(r"::[^\n\r]*(?:\r?\n)?", logos::skip)]
    ColCol,

    #[token(".^")]
    DotKet,
    // #[token(".+")]
    // DotLus,
    #[token(".*")]
    DotTar,
    #[token(".=")]
    DotTis,
    #[token(".?")]
    DotWut,

    #[token(";:")]
    MicCol,
    #[token(";<")]
    MicGal,
    #[token(";+")]
    MicLus,
    #[token(";;")]
    MicMic,
    #[token(";/")]
    MicFas,
    #[token(";~")]
    MicSig,
    #[token(";*")]
    MicTar,
    #[token(";=")]
    MicTis,

    #[token("~>(")]
    SigGarWide,
    #[token("~>")]
    SigGar,
    #[token("~|")]
    SigBar,
    #[token("~$")]
    SigBuc,
    #[token("~_")]
    SigCab,
    #[token("~%")]
    SigCen,
    #[token("~<")]
    SigGal,
    #[token("~+")]
    SigLus,
    #[token("~/")]
    SigFas,
    #[token("~&")]
    SigPam,
    #[token("~=")]
    SigTis,
    #[token("~?")]
    SigWut,
    #[token("~!")]
    SigZap,

    #[token("!,")]
    ZapCom,
    #[token("!>")]
    ZapGar,
    #[token("!<")]
    ZapGal,
    #[token("!;")]
    ZapMic,
    // #[token("!=")]     this conflicts with  WutZapIrregular ->  !  ->  !=(1 2)
    // ZapTis,
    #[token("!?")]
    ZapWut,
    #[token("!@")]
    ZapPat,
    #[token("!:")]
    ZapCol,
    #[token("!.")]
    ZapDot,
    #[token("!!")]
    ZapZap,

    #[token("?(")]
    BucWutIrregular,
    // #[token(":(")]
    // MicColIrregular,
    #[token("~(")]
    CenSigIrregular,
    // #[regex(r"[a-zA-Z_$][a-zA-Z0-9]*\(", |lex| {  //  foo(           some invalid cases will match in here...
    //     let slice = lex.slice();
    //     slice.strip_suffix('(').unwrap_or(slice)
    // })]
    // CenTisIrregular(&'a str),

    #[regex(r"[+-][<>](?:[+-][<>])*[+-]?", |lex| lex.slice())]
    LarkExpression(&'a str),  //  +>- expression, with 2 or more chars,
                              //  single chars will be matched by another rule

    #[regex(r#""[^"]*""#, |lex| &lex.slice()[1..lex.slice().len() - 1])]
    Tape(&'a str),

    #[regex(r#"'[^']*'"#, |lex| &lex.slice()[1..lex.slice().len() - 1])]
    Cord(&'a str),

    #[token("==")]
    TisTis,
    #[token("--")]
    HepHep,

    #[token("%|")]
    CenBar,
    #[token("%&")]
    CenPam,

    #[token("]~")]
    SigSer,
    #[token("~[")]
    SigSel,
    #[token("+(")]
    Increment,

    #[regex("\\s{2,}|\\n+")]
    Gap,
    #[regex(r" ")]
    Ace,

    #[token("%")]
    Cen,
    #[token(">")]  //  we are using this for gate calls
    Gar,
    #[token("<")]
    Gal,
    #[token("(")]  //  we are using this for gate calls
    Pal,
    #[token(")")]
    Par,
    #[token("+")]
    Lus,
    #[token("-")]
    Hep,
    #[token("[")]
    Sel,
    #[token("]")]
    Ser,
    #[token("~")]
    Sig,
    #[token("`")]
    Tic,
    #[token("=")]
    Tis,
    #[token(":")]
    Col,
    #[token(",")]
    Com,
    #[token("^")]
    Ket,
    #[token("|")]
    Bar,
    #[token("/")]
    Fas,
    #[token("\\")]
    Bas,
    #[token("&")]
    Pam,
    #[token("*")]
    Tar,
    #[token(".")]
    Dot,
    #[token("$")]
    Buc,
    #[token("!")]  //  WutZapIrregular/ZapTis
    Zap,
    #[token("?")]
    Wut,
    #[token("_")]
    Cab,

    #[regex(r"[0-9]{1,3}(?:\.(?: *\n+ *| {2,})?[0-9]{3})*", |lex| lex.slice())]
    Number(&'a str),

    #[regex(r"@[a-zA-Z0-9]*", |lex| lex.slice())]
    Aura(&'a str),

    #[regex(r"0x[0-9a-fA-F]+(\.[0-9a-fA-F]+)?")]
    Hex(&'a str),

    #[regex(r"[a-zA-Z][a-zA-Z0-9-]*", |lex| lex.slice())]
    Name(&'a str),
}
impl<'a> fmt::Display for Token<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let debug_str = format!("{:?}", self);
        let token_name = debug_str.split('(').next().unwrap_or(&debug_str);
        write!(f, "{}", token_name)
    }
}