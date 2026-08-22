//! Errors surfaced while planning or running a workflow.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Error {
    Io { at: PathBuf, err: std::io::Error },
    Yaml(yaml_with_spans::Error),
    Expr(gh_actions_expr::Error),
    Invalid(gh_actions_plan::Diagnostic),
    Plan(String),
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { at, err } => write!(f, "{}: {err}", at.display()),
            Self::Yaml(err) => write!(f, "invalid workflow: {err}"),
            Self::Expr(err) => write!(f, "expression error: {err}"),
            Self::Invalid(finding) => write!(
                f,
                "{} [{}] {}",
                finding.location, finding.rule, finding.message
            ),
            Self::Plan(msg) => write!(f, "cannot plan workflow: {msg}"),
            Self::Unsupported(what) => write!(f, "not supported yet: {what}"),
        }
    }
}

impl std::error::Error for Error {}

pub trait At<T> {
    fn at(self, at: impl AsRef<Path>) -> Result<T, Error>;
}

impl<T> At<T> for Result<T, std::io::Error> {
    fn at(self, at: impl AsRef<Path>) -> Result<T, Error> {
        self.map_err(|err| Error::Io {
            at: at.as_ref().to_path_buf(),
            err,
        })
    }
}

impl From<yaml_with_spans::Error> for Error {
    fn from(err: yaml_with_spans::Error) -> Self {
        Self::Yaml(err)
    }
}

impl From<gh_actions_expr::Error> for Error {
    fn from(err: gh_actions_expr::Error) -> Self {
        Self::Expr(err)
    }
}

impl From<gh_actions_expr::ParseError> for Error {
    fn from(err: gh_actions_expr::ParseError) -> Self {
        Self::Expr(gh_actions_expr::Error::Parse(err))
    }
}

impl From<gh_actions_expr::EvalError> for Error {
    fn from(err: gh_actions_expr::EvalError) -> Self {
        Self::Expr(gh_actions_expr::Error::Eval(err))
    }
}

impl From<gh_actions_plan::Error> for Error {
    fn from(err: gh_actions_plan::Error) -> Self {
        match err {
            gh_actions_plan::Error::Invalid(finding) => Self::Invalid(finding),
            gh_actions_plan::Error::Plan(message) => Self::Plan(message),
            gh_actions_plan::Error::Unsupported(what) => Self::Unsupported(what),
        }
    }
}
