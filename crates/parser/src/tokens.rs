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
    #[token("!")]
    WutZapIrregular,

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
    #[token("=-")]
    TisHep,
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
    #[token("=~")]
    TisSig,
    #[token("=*")]
    TisTar,
    #[token("=?")]
    TisWut,

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
    #[token("$:")]
    BucCol,
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

    #[token(":-")]
    ColHep,
    #[token(":_")]
    ColCab,
    #[token(":+")]
    ColLus,
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
    #[token(".+")]
    DotLus,
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
    #[token("!=")]
    ZapTis,
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

    #[token("=(")]
    DotTisIrregular,
    #[token(":(")]
    MicColIrregular,
    #[token("~(")]
    CenSigIrregular,
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_-]*\(", priority = 1)]
    CenTisIrregular,

    #[regex("'[ -~]*'")]
    Cord,
    #[regex("\"[ -~]*\"")]
    Tape,

    #[token("==")]
    TisTis,
    #[token("--")]
    HepHep,

    #[token("~[")]
    ListOpen,
    #[token("+(")]
    Increment,

    #[regex("\\s{2,}|\\n+")]
    Gap,
    #[regex(r" ")]
    Ace,

    #[token("(")]  //  we are using this for gate calls
    Pal,
    #[token(")")]
    Par,
    #[token("+")]
    Lus,
    #[token("[")]
    Sel,
    #[token("]")]
    Ser,
    #[token("=")]
    Tis,
    #[token(":")]
    Col,
    #[token(",")]
    Con,

    #[regex(r"[0-9]{1,3}(?:\.(?: *\n+ *| {2,})?[0-9]{3})*", |lex| lex.slice())]
    Number(&'a str),

    #[regex(r"@[a-zA-Z0-9]*", |lex| lex.slice())]
    Aura(&'a str),

    #[regex(r"%[a-zA-Z0-9]+", |lex| lex.slice())]
    Term(&'a str),

    #[regex(r"[a-zA-Z$][a-zA-Z0-9-$]*", |lex| lex.slice())]
    Name(&'a str),
}

impl<'a> fmt::Display for Token<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Name(s) => write!(f, "Name({})", s),
            Token::LusLus => write!(f, "LusLus"),
            Token::LusBar => write!(f, "LusBar"),
            Token::ZapZap => write!(f, "ZapZap"),
            Token::Gap => write!(f, "Gap"),
            Token::BarCen => write!(f, "BarCen"),
            Token::HepHep => write!(f, "HepHep"),
            _ =>  write!(f, ""),
            // Add other variants as needed
        }
    }
}