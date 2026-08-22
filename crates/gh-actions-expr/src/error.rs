//! Errors produced while parsing or evaluating an expression.

use std::fmt;

/// A failure to turn expression source into an AST.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    UnexpectedChar(char, usize),
    UnterminatedString(usize),
    InvalidNumber(String, usize),
    UnexpectedToken(String, usize),
    UnexpectedEnd,
    TrailingInput(usize),
    UnterminatedTemplate(usize),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedChar(c, at) => write!(f, "unexpected character {c:?} at {at}"),
            Self::UnterminatedString(at) => write!(f, "unterminated string starting at {at}"),
            Self::InvalidNumber(s, at) => write!(f, "invalid number {s:?} at {at}"),
            Self::UnexpectedToken(t, at) => write!(f, "unexpected token {t:?} at {at}"),
            Self::UnexpectedEnd => write!(f, "unexpected end of expression"),
            Self::TrailingInput(at) => write!(f, "unexpected trailing input at {at}"),
            Self::UnterminatedTemplate(at) => {
                write!(f, "unterminated `${{{{` starting at offset {at}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// A failure to evaluate an AST against a context.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    UnknownContext(String),
    UnknownFunction(String),
    WrongArity {
        function: String,
        expected: String,
        got: usize,
    },
    /// A function that needs runtime support this crate does not provide yet.
    Unsupported(&'static str),
    InvalidJson(String),
    InvalidFormat(String),
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownContext(name) => write!(f, "unknown context {name:?}"),
            Self::UnknownFunction(name) => write!(f, "unknown function {name:?}"),
            Self::WrongArity {
                function,
                expected,
                got,
            } => write!(f, "{function} expects {expected} arguments, got {got}"),
            Self::Unsupported(name) => write!(f, "{name} is not supported yet"),
            Self::InvalidJson(msg) => write!(f, "invalid JSON: {msg}"),
            Self::InvalidFormat(msg) => write!(f, "invalid format string: {msg}"),
        }
    }
}

impl std::error::Error for EvalError {}
