//! Where something is in the source.

/// Carries both the byte offset and the line and column, because the two callers want
/// different things: rewriting a file wants offsets, an editor wants lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Position {
    /// Bytes from the start of the source.
    pub offset: usize,
    /// Counted from zero, as an editor counts it.
    pub line: u32,
    /// Bytes from the start of the line.
    pub column: u32,
}

/// The half-open range something covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: Position,
    /// One past the last byte.
    pub end: Position,
}

impl Span {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    /// The source it covers.
    pub fn of<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start.offset..self.end.offset]
    }
}
