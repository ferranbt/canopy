//! The parsed shape of an expression.

/// A node of the expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// A top-level context reference, e.g. `github` in `github.sha`.
    Context(String),
    /// Property access, e.g. `.sha`.
    Property(Box<Expr>, String),
    /// Index access, e.g. `['os']` or `[0]`.
    Index(Box<Expr>, Box<Expr>),
    /// The `*` filter, collecting every element or value of the target.
    Star(Box<Expr>),
    Not(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// The binary operators of the language, in precedence groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// Compares loosely, with type coercion.
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    /// Returns one of its operands rather than a bool.
    And,
    /// Returns one of its operands rather than a bool.
    Or,
}
