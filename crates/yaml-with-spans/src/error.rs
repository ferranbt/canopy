//! What comes back when the source is not YAML we can read.

use std::fmt;

use crate::span::Position;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    /// Where it gave up.
    pub position: Position,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            self.position.line + 1,
            self.position.column + 1,
            self.message
        )
    }
}

impl std::error::Error for Error {}
