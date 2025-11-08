use logos::Logos;
use std::fmt;

fn parse_multiline_string<'a>(lex: &mut logos::Lexer<'a, Token<'a>>) -> Option<&'a str> {
    let remainder = lex.remainder();
    let mut end = 0;
    for line in remainder.lines() {
        end += line.len() + 1; // +1 for '\n'
        if line.trim() == "'''" {
            let slice = &remainder[..end - line.len() - 1]; // exclude closing line
            lex.bump(end); // advance lexer past closing marker
            return Some(slice);
        }
    }
    None
}

#[derive(Logos, Debug, PartialEq, Clone)]
pub enum Token<'a> {
    LexerError,

    // Absolute date: ~2025.11.04 or ~2025.11.04..14.30.00..1f2e
    #[regex(r"~\d{4}\.\d{1,2}\.\d{1,2}(?:\.\.\d+\.\d+\.\d+\.\.[0-9a-f]+)?",
            |lex| lex.slice(),
            priority = 4)]
    DateAbsolute(&'a str),

    // Relative date: ~d5, ~h12.m30, ~s1.h2.d3.m4
    #[regex(r"~[dhms]\d+(?:\.[dhms]\d+)*",
            |lex| lex.slice(),
            priority = 4)]
    DateRelative(&'a str),

    #[regex(r"[+-][<>](?:[+-][<>])*[+-]?", |lex| lex.slice())]
    LarkExpression(&'a str),  //  +>- expression, with 2 or more chars,
                              //  single chars will be matched by another rule

    #[regex(r#""[^"]*""#, |lex| &lex.slice()[1..lex.slice().len() - 1])]
    Tape(&'a str),

    #[regex(r"~-~?[0-9a-fA-F]+\.?|~-[a-zA-Z]|~\[(?:~-[a-zA-Z0-9]+(?:\s+)?)+\]", |lex| lex.slice())]
    Unicode(&'a str),

    #[regex(r"[a-zA-Z0-9]+", |lex| lex.slice())]
    AlphaNumeric(&'a str),

    #[regex(r"0i[0-9]+", |lex| lex.slice())]
    UiNumber(&'a str),

    #[regex(r"deletethis", |lex| lex.slice())]
    Number(&'a str),

    #[token("%.y")]
    Yes,
    #[token("%.n")]
    No,

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
    #[token("'")]
    Soq,

    #[regex(r"'(.*)'", |lex| {
        &lex.slice()[1..lex.slice().len() - 1]
    })]
    Cord(&'a str),
    #[regex(r"/(.*)\\", |lex| {
        &lex.slice()[1..lex.slice().len() - 1]
    })]
    CordContinuation(&'a str),
    #[regex(r"'(.*)\\", |lex| {
        &lex.slice()[1..lex.slice().len() - 1]
    })]
    CordOpened(&'a str),
    #[regex(r"/(.*)'", |lex| {
        &lex.slice()[1..lex.slice().len() - 1]
    })]
    CordClosed(&'a str),

    TripleCord(String) = {
        #[regex(r"(?s)'''\r?\n((?:.*\r?\n)*?)(\r?\n)?'''", |lex| {
            let caps = lex.captures();
            let mut body = caps.get(1).map(|m| m.as_str()).unwrap_or("");

            if let Some(nl) = body.find('\n') {
                let line = &body[..nl];
                if line.trim_start().starts_with("::") || line.trim().is_empty() {
                    body = body[nl + 1..].trim_start_matches(['\r', '\n']);
                }
            } else if body.trim_start().starts_with("::") || body.trim().is_empty() {
                body = "";
            }

            body.trim_end().to_string()
        })]
        => TripleCord(<>)
    };

    #[regex(r"\s{2,}(?:\n+)?|\n+")]
    #[regex(r"(?:\s*)?::[^\n\r]*(?:\r?\n)?")]  // comments
    Gap,

    #[regex(r" ")]
    Ace,
}

impl<'a> fmt::Display for Token<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let debug_str = format!("{:?}", self);
        let token_name = debug_str.split('(').next().unwrap_or(&debug_str);
        write!(f, "{}", token_name)
    }
}