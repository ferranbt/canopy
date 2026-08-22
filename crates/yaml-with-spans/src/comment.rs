use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// What follows the `#`, trimmed.
    pub text: String,
    /// From the `#` to the end of the line.
    pub span: Span,
    /// Alone on its line, rather than trailing a value.
    pub own_line: bool,
}

impl Comment {
    pub fn line(&self) -> u32 {
        self.span.start.line
    }
}
