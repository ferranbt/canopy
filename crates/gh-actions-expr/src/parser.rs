//! Builds an [`Expr`] tree from a token stream.

use crate::ast::{BinaryOp, Expr};
use crate::error::ParseError;
use crate::lexer::{Token, TokenKind, tokenize};

pub fn parse(source: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.parse_or()?;

    match parser.peek() {
        Some(token) => Err(ParseError::TrailingInput(token.at)),
        None => Ok(expr),
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|token| &token.kind)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<(), ParseError> {
        match self.next() {
            Some(token) if token.kind == *expected => Ok(()),
            Some(token) => Err(ParseError::UnexpectedToken(token.kind.describe(), token.at)),
            None => Err(ParseError::UnexpectedEnd),
        }
    }

    fn eat(&mut self, expected: &TokenKind) -> bool {
        if self.peek_kind() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// `or := and ( '||' and )*`
    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.eat(&TokenKind::Or) {
            let right = self.parse_and()?;
            left = Expr::Binary(BinaryOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `and := equality ( '&&' equality )*`
    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while self.eat(&TokenKind::And) {
            let right = self.parse_equality()?;
            left = Expr::Binary(BinaryOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `equality := relational ( ('=='|'!=') relational )*`
    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_relational()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Eq) => BinaryOp::Eq,
                Some(TokenKind::NotEq) => BinaryOp::NotEq,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.parse_relational()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    /// `relational := unary ( ('<'|'<='|'>'|'>=') unary )*`
    fn parse_relational(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Lt) => BinaryOp::Lt,
                Some(TokenKind::LtEq) => BinaryOp::LtEq,
                Some(TokenKind::Gt) => BinaryOp::Gt,
                Some(TokenKind::GtEq) => BinaryOp::GtEq,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    /// `unary := '!' unary | postfix`
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat(&TokenKind::Bang) {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_postfix()
    }

    /// `postfix := primary ( '.' ident | '.' '*' | '[' or ']' )*`
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.eat(&TokenKind::Dot) {
                match self.next() {
                    Some(Token {
                        kind: TokenKind::Ident(name),
                        ..
                    }) => expr = Expr::Property(Box::new(expr), name),
                    Some(Token {
                        kind: TokenKind::Star,
                        ..
                    }) => expr = Expr::Star(Box::new(expr)),
                    Some(token) => {
                        return Err(ParseError::UnexpectedToken(token.kind.describe(), token.at));
                    }
                    None => return Err(ParseError::UnexpectedEnd),
                }
            } else if self.eat(&TokenKind::LBracket) {
                // A bare `*` inside brackets is the same filter as `.*`.
                if self.eat(&TokenKind::Star) {
                    self.expect(&TokenKind::RBracket)?;
                    expr = Expr::Star(Box::new(expr));
                } else {
                    let index = self.parse_or()?;
                    self.expect(&TokenKind::RBracket)?;
                    expr = Expr::Index(Box::new(expr), Box::new(index));
                }
            } else {
                return Ok(expr);
            }
        }
    }

    /// `primary := literal | ident | call | '(' or ')'`
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let token = self.next().ok_or(ParseError::UnexpectedEnd)?;

        match token.kind {
            TokenKind::Number(n) => Ok(Expr::Number(n)),
            TokenKind::String(s) => Ok(Expr::String(s)),
            TokenKind::LParen => {
                let inner = self.parse_or()?;
                self.expect(&TokenKind::RParen)?;
                Ok(inner)
            }
            TokenKind::Ident(name) => match name.to_ascii_lowercase().as_str() {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "null" => Ok(Expr::Null),
                // An identifier followed by `(` is a function call, otherwise a context.
                _ if self.peek_kind() == Some(&TokenKind::LParen) => {
                    self.pos += 1;
                    Ok(Expr::Call(name, self.parse_args()?))
                }
                _ => Ok(Expr::Context(name)),
            },
            _ => Err(ParseError::UnexpectedToken(token.kind.describe(), token.at)),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();

        if self.eat(&TokenKind::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_or()?);
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            self.expect(&TokenKind::RParen)?;
            return Ok(args);
        }
    }
}
