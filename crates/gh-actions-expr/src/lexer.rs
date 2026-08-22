//! Turns expression source into a token stream.

use crate::error::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte offset where the token starts.
    pub at: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// Keywords are not distinguished, e.g. `github`, `contains`, `true`.
    Ident(String),
    Number(f64),
    /// Single-quoted, with `''` escapes resolved.
    String(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Star,
    Bang,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

impl TokenKind {
    /// How it is spelled, for an error message to quote.
    pub fn describe(&self) -> String {
        match self {
            Self::Ident(name) => name.clone(),
            Self::Number(n) => n.to_string(),
            Self::String(s) => format!("'{s}'"),
            Self::LParen => "(".into(),
            Self::RParen => ")".into(),
            Self::LBracket => "[".into(),
            Self::RBracket => "]".into(),
            Self::Comma => ",".into(),
            Self::Dot => ".".into(),
            Self::Star => "*".into(),
            Self::Bang => "!".into(),
            Self::Eq => "==".into(),
            Self::NotEq => "!=".into(),
            Self::Lt => "<".into(),
            Self::LtEq => "<=".into(),
            Self::Gt => ">".into(),
            Self::GtEq => ">=".into(),
            Self::And => "&&".into(),
            Self::Or => "||".into(),
        }
    }
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, ParseError> {
    let bytes: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        let at = i;

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        let kind = match c {
            '(' => {
                i += 1;
                TokenKind::LParen
            }
            ')' => {
                i += 1;
                TokenKind::RParen
            }
            '[' => {
                i += 1;
                TokenKind::LBracket
            }
            ']' => {
                i += 1;
                TokenKind::RBracket
            }
            ',' => {
                i += 1;
                TokenKind::Comma
            }
            '*' => {
                i += 1;
                TokenKind::Star
            }
            '.' if !bytes.get(i + 1).is_some_and(char::is_ascii_digit) => {
                i += 1;
                TokenKind::Dot
            }
            '=' if bytes.get(i + 1) == Some(&'=') => {
                i += 2;
                TokenKind::Eq
            }
            '!' if bytes.get(i + 1) == Some(&'=') => {
                i += 2;
                TokenKind::NotEq
            }
            '!' => {
                i += 1;
                TokenKind::Bang
            }
            '<' if bytes.get(i + 1) == Some(&'=') => {
                i += 2;
                TokenKind::LtEq
            }
            '<' => {
                i += 1;
                TokenKind::Lt
            }
            '>' if bytes.get(i + 1) == Some(&'=') => {
                i += 2;
                TokenKind::GtEq
            }
            '>' => {
                i += 1;
                TokenKind::Gt
            }
            '&' if bytes.get(i + 1) == Some(&'&') => {
                i += 2;
                TokenKind::And
            }
            '|' if bytes.get(i + 1) == Some(&'|') => {
                i += 2;
                TokenKind::Or
            }
            '\'' => lex_string(&bytes, &mut i)?,
            // A `-` only starts a number here, as the language has no subtraction.
            c if c.is_ascii_digit() || c == '.' || (c == '-' && starts_number(&bytes, i)) => {
                lex_number(&bytes, &mut i)?
            }
            c if is_ident_start(c) => lex_ident(&bytes, &mut i),
            _ => return Err(ParseError::UnexpectedChar(c, at)),
        };

        tokens.push(Token { kind, at });
    }

    Ok(tokens)
}

/// Whether a `-` at this position introduces a negative numeric literal.
fn starts_number(bytes: &[char], i: usize) -> bool {
    bytes
        .get(i + 1)
        .is_some_and(|c| c.is_ascii_digit() || *c == '.')
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Hyphens are legal inside a property name.
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Where `''` stands for a literal quote.
fn lex_string(bytes: &[char], i: &mut usize) -> Result<TokenKind, ParseError> {
    let start = *i;
    *i += 1;
    let mut out = String::new();

    loop {
        match bytes.get(*i) {
            None => return Err(ParseError::UnterminatedString(start)),
            Some('\'') if bytes.get(*i + 1) == Some(&'\'') => {
                out.push('\'');
                *i += 2;
            }
            Some('\'') => {
                *i += 1;
                return Ok(TokenKind::String(out));
            }
            Some(c) => {
                out.push(*c);
                *i += 1;
            }
        }
    }
}

fn lex_number(bytes: &[char], i: &mut usize) -> Result<TokenKind, ParseError> {
    let start = *i;

    if bytes[*i] == '-' {
        *i += 1;
    }

    if bytes.get(*i) == Some(&'0') && matches!(bytes.get(*i + 1), Some('x') | Some('X')) {
        *i += 2;
        let digits_start = *i;
        while bytes.get(*i).is_some_and(char::is_ascii_hexdigit) {
            *i += 1;
        }
        let raw: String = bytes[start..*i].iter().collect();
        let digits: String = bytes[digits_start..*i].iter().collect();
        let value = u64::from_str_radix(&digits, 16)
            .map_err(|_| ParseError::InvalidNumber(raw.clone(), start))?;
        let signed = if bytes[start] == '-' { -1.0 } else { 1.0 };
        return Ok(TokenKind::Number(signed * value as f64));
    }

    while bytes.get(*i).is_some_and(char::is_ascii_digit) {
        *i += 1;
    }
    if bytes.get(*i) == Some(&'.') {
        *i += 1;
        while bytes.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
    }
    if matches!(bytes.get(*i), Some('e') | Some('E')) {
        *i += 1;
        if matches!(bytes.get(*i), Some('+') | Some('-')) {
            *i += 1;
        }
        while bytes.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
    }

    let raw: String = bytes[start..*i].iter().collect();
    raw.parse::<f64>()
        .map(TokenKind::Number)
        .map_err(|_| ParseError::InvalidNumber(raw, start))
}

fn lex_ident(bytes: &[char], i: &mut usize) -> TokenKind {
    let start = *i;
    while bytes.get(*i).copied().is_some_and(is_ident_continue) {
        *i += 1;
    }
    TokenKind::Ident(bytes[start..*i].iter().collect())
}
