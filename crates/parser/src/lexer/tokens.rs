use logos::Logos;
use std::fmt;

#[derive(Logos, Debug, PartialEq, Clone)]
pub enum Token<'a> {
    LexerError,

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
    #[token("$<")]
    BucGal,
    #[token("$>")]
    BucGar,
    #[token("$-")]
    BucHep,
    #[token("$^")]
    BucKet,
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
    #[token("$%(")]
    BucCenWide,

    #[token("%.y")]
    Yes,
    #[token("%.n")]
    No,

    #[token(":_")]
    ColCab,
    #[token(":^")]
    ColKet,
    #[token(":*")]
    ColTar,
    #[token(":~")]
    ColSig,
    #[token(":_(")]
    ColCabWide,
    #[token(":^(")]
    ColKetWide,
    // #[regex(r"::[^\n\r]*(?:\r?\n)?", logos::skip)]  matched as Gap
    // ColCol,

    #[token(".*")]
    DotTar,
    #[token(".=")]
    DotTis,
    #[token(".?")]
    DotWut,
    #[token(".?(")]
    DotWutWide,
    #[token(".*(")]
    DotTarWide,

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
    #[token(";~", priority = 2)]
    MicSig,
    #[token(";*")]
    MicTar,
    #[token(";=")]
    MicTis,
    #[token(";~(", priority = 3)]
    MicSigWide,
    #[token(";;(")]
    MicMicWide,
    #[token(";/(")]
    MicFasWide,

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
    #[token("~+(")]
    SigLusWide,
    #[token("~_(")]
    SigCabWide,
    #[token("~|(")]
    SigBarWide,
    #[token("~&(")]
    SigPamWide,

    #[token("!,")]
    ZapCom,
    #[token("!>")]
    ZapGar,
    #[token("!<")]
    ZapGal,
    #[token("!;")]
    ZapMic,
    #[token("!@")]
    ZapPat,
    #[token("!:")]
    ZapCol,
    #[token("!.")]
    ZapDot,
    #[token("!!")]
    ZapZap,
    #[token("!>(")]
    ZapGarWide,

    #[regex(r"~\d{4}\.\d{1,2}\.\d{1,2}(?:\.\.\d+\.\d+\.\d+\.\.[0-9a-f]+)?", |lex| lex.slice(), priority = 20)]
    Date(&'a str),

    #[regex(r"[+-][<>](?:[+-][<>])*[+-]?", |lex| lex.slice())]
    LarkExpression(&'a str),  //  +>- expression, with 2 or more chars,
                              //  single chars will be matched by another rule

    #[regex(r#""[^"]*""#, |lex| &lex.slice()[1..lex.slice().len() - 1])]
    Tape(&'a str),

    #[regex(r"'(?:[^'\\\n]|\\[ -~]|\\\n)*'", |lex| {
        &lex.slice()[1..lex.slice().len() - 1]
    })]
    Cord(&'a str),

    // triple cords: '''multi\nline'''   //TODO
    // CordLong(&'a str),

    #[token("--")]
    HepHep,

    #[token("]~")]
    SigSer,
    #[token("~[")]
    SigSel,

    #[regex(r"\s{2,}(?:\n+)?|\n+")]
    #[regex(r"(?:\s*)?::[^\n\r]*(?:\r?\n)?")]
    Gap,

    #[regex(r" ")]
    Ace,

    #[token("%")]
    Cen,
    #[token(">")]
    Gar,
    #[token("<")]
    Gal,
    #[token("@")]
    Pat,
    #[token("(")]
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
    #[token("!")]
    Zap,
    #[token(";")]
    Mic,
    #[token("?")]
    Wut,
    #[token("_")]
    Cab,

    #[regex(r"0b[01]{1,4}(?:\.[\t\n\r ]*[01]{4})*", |lex| lex.slice(), priority = 1)]
    BinaryNumber(&'a str),

    #[regex(r"0x[0-9a-fA-F]{1,4}(?:\.[\t\n\r ]*[0-9a-fA-F]{4})*", |lex| lex.slice(), priority = 4)]
    HexNumber(&'a str),

    #[regex(r"-{1,2}[0-9]{1,3}(?:\.(?: *\n+ *| {2,})?[0-9]{3})*", priority = 3)]
    SignedNumber(&'a str),

    #[regex(r"[0-9]{1,3}(?:\.[0-9]{3})*", callback = |lex| lex.slice(), priority = 1)]
    Number(&'a str),

    #[regex(r"~-~?[0-9a-fA-F]+\.?|~-[a-zA-Z]|~\[(?:~-[a-zA-Z0-9]+(?:\s+)?)+\]", |lex| lex.slice())]
    Unicode(&'a str),

    // #[regex(r"@[a-zA-Z0-9]*", |lex| lex.slice(), priority = 1)]
    // Aura(&'a str),

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