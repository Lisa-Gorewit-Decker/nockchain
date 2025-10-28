use logos::Logos;
use std::fmt;

#[derive(Logos, Debug, PartialEq, Clone)]
pub enum Token<'a> {
    LexerError,

    #[token("%.y")]
    Yes,
    #[token("%.n")]
    No,

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

    #[regex(r"\s{2,}(?:\n+)?|\n+")]
    #[regex(r"(?:\s*)?::[^\n\r]*(?:\r?\n)?")]  // comments
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