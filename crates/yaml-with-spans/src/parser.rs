//! Reading the source into a tree.
//!
//! A hand-written recursive descent parser over the bytes, tracking the column so block
//! structure can be read from the indentation. It covers the YAML that workflow files are
//! written in rather than all of YAML 1.2: anchors, aliases and tags are refused outright,
//! which is no loss, because GitHub refuses them too.

use crate::comment::Comment;
use crate::error::Error;
use crate::node::{Mapping, Node, Value};
use crate::span::{Position, Span};

/// How a block scalar treats the newlines at its end.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Chomp {
    /// `-`, keep none of them.
    Strip,
    /// The default, keep one.
    Clip,
    /// `+`, keep all of them.
    Keep,
}

#[derive(Clone, Copy)]
struct State {
    offset: usize,
    line: u32,
    column: u32,
    comments: usize,
}

pub(crate) struct Parser<'a> {
    source: &'a str,
    offset: usize,
    line: u32,
    column: u32,
    comments: Vec<Comment>,
}

pub(crate) fn parse(source: &str) -> Result<(Node, Vec<Comment>), Error> {
    let mut parser = Parser {
        source,
        offset: 0,
        line: 0,
        column: 0,
        comments: Vec::new(),
    };

    parser.skip_trivia();
    if parser.looking_at("---") {
        parser.skip_line();
        parser.skip_trivia();
    }

    let root = if parser.eof() {
        parser.null_here()
    } else {
        parser.parse_block_node(0)?
    };

    parser.skip_trivia();
    if !parser.eof() && !parser.looking_at("...") {
        return Err(parser.error("unexpected content after the end of the document"));
    }

    Ok((root, parser.comments))
}

impl<'a> Parser<'a> {
    // --- reading bytes ---------------------------------------------------

    fn eof(&self) -> bool {
        self.offset >= self.source.len()
    }

    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.offset).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.source.as_bytes().get(self.offset + ahead).copied()
    }

    fn looking_at(&self, text: &str) -> bool {
        self.source[self.offset..].starts_with(text)
    }

    fn position(&self) -> Position {
        Position {
            offset: self.offset,
            line: self.line,
            column: self.column,
        }
    }

    /// Keeps the line and column right as it goes.
    fn bump(&mut self) {
        let Some(byte) = self.peek() else { return };
        self.offset += 1;
        if byte == b'\n' {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += 1;
        }
    }

    /// A whole character, so collecting text never splits UTF-8.
    fn bump_char(&mut self) -> Option<char> {
        let ch = self.source[self.offset..].chars().next()?;
        for _ in 0..ch.len_utf8() {
            self.bump();
        }
        Some(ch)
    }

    fn save(&self) -> State {
        State {
            offset: self.offset,
            line: self.line,
            column: self.column,
            comments: self.comments.len(),
        }
    }

    /// Forgets anything read since, comments included.
    fn restore(&mut self, state: State) {
        self.offset = state.offset;
        self.line = state.line;
        self.column = state.column;
        self.comments.truncate(state.comments);
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error {
            message: message.into(),
            position: self.position(),
        }
    }

    /// For a key written without a value.
    fn null_at(&self, at: Position) -> Node {
        Node {
            value: Value::Null,
            span: Span::new(at, at),
        }
    }

    fn null_here(&self) -> Node {
        self.null_at(self.position())
    }

    // --- whitespace and comments -----------------------------------------

    /// Never a line ending.
    fn skip_inline_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.bump();
        }
    }

    fn skip_line(&mut self) {
        while !self.eof() && self.peek() != Some(b'\n') {
            self.bump();
        }
        self.bump();
    }

    /// Whether only whitespace stands between here and the start of the line.
    fn only_space_before(&self) -> bool {
        let start = self.offset - self.column as usize;
        self.source[start..self.offset]
            .bytes()
            .all(|byte| byte == b' ' || byte == b'\t')
    }

    /// Leaves the cursor on the line ending that follows the comment.
    fn take_comment(&mut self, own_line: bool) {
        let start = self.position();
        let mut end = start;
        while !self.eof() && self.peek() != Some(b'\n') {
            self.bump();
            if !matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
                end = self.position();
            }
        }

        let span = Span::new(start, end);
        let text = span
            .of(self.source)
            .trim_start_matches('#')
            .trim()
            .to_owned();
        self.comments.push(Comment {
            text,
            span,
            own_line,
        });
    }

    /// Whitespace, blank lines and comments alike.
    fn skip_trivia(&mut self) {
        loop {
            let own_line = self.only_space_before();
            self.skip_inline_space();
            match self.peek() {
                Some(b'#') => self.take_comment(own_line),
                Some(b'\n') => self.bump(),
                _ => return,
            }
        }
    }

    fn at_line_end(&self) -> bool {
        matches!(self.peek(), None | Some(b'\n') | Some(b'#'))
    }

    /// A `-` starting an entry, rather than one inside a scalar like `-3`.
    fn at_sequence_entry(&self) -> bool {
        self.peek() == Some(b'-') && matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n'))
    }

    // --- block structure --------------------------------------------------

    /// `parent` is the indentation of the block this node belongs to, which is what decides
    /// whether a following line continues a plain scalar or starts something new.
    fn parse_block_node(&mut self, parent: u32) -> Result<Node, Error> {
        let indent = self.column;

        if self.at_sequence_entry() {
            return self.parse_block_sequence(indent);
        }
        if let Some(key) = self.try_key()? {
            return self.parse_block_mapping(indent, key);
        }

        self.parse_value(parent)
    }

    fn parse_block_mapping(&mut self, indent: u32, first_key: Node) -> Result<Node, Error> {
        let start = first_key.span.start;
        let mut mapping = Mapping::default();
        let mut key = first_key;
        let mut end;

        loop {
            let value = self.parse_key_value(indent)?;
            end = value.span.end.max(key.span.end);
            mapping.push(key, value);

            self.skip_trivia();
            if self.eof() || self.column != indent || self.at_sequence_entry() {
                break;
            }
            match self.try_key()? {
                Some(next) => key = next,
                None => break,
            }
        }

        Ok(Node {
            value: Value::Mapping(mapping),
            span: Span::new(start, end),
        })
    }

    /// Which may be on the same line as the key, or on the lines below it.
    fn parse_key_value(&mut self, indent: u32) -> Result<Node, Error> {
        let after_colon = self.position();
        self.skip_inline_space();

        if !self.at_line_end() {
            return self.parse_value(indent);
        }

        self.skip_trivia();
        if self.eof() {
            return Ok(self.null_at(after_colon));
        }
        // A sequence may sit at its key's own indentation, which nothing else may do.
        if self.column == indent && self.at_sequence_entry() {
            return self.parse_block_sequence(indent);
        }
        if self.column > indent {
            return self.parse_block_node(indent);
        }

        Ok(self.null_at(after_colon))
    }

    fn parse_block_sequence(&mut self, indent: u32) -> Result<Node, Error> {
        let start = self.position();
        let mut items = Vec::new();
        let mut end;

        loop {
            self.bump();
            let after_dash = self.position();
            self.skip_inline_space();

            let item = if self.at_line_end() {
                self.skip_trivia();
                if !self.eof() && self.column > indent {
                    self.parse_block_node(indent)?
                } else {
                    self.null_at(after_dash)
                }
            } else {
                self.parse_block_node(indent)?
            };
            end = item.span.end.max(after_dash);
            items.push(item);

            self.skip_trivia();
            if self.eof() || self.column != indent || !self.at_sequence_entry() {
                break;
            }
        }

        Ok(Node {
            value: Value::Sequence(items),
            span: Span::new(start, end),
        })
    }

    /// Nothing at all when what follows is not a key.
    fn try_key(&mut self) -> Result<Option<Node>, Error> {
        let state = self.save();
        let key = match self.peek() {
            Some(b'"') => self.parse_double_quoted()?,
            Some(b'\'') => self.parse_single_quoted()?,
            // A complex key is not something a workflow ever needs.
            Some(b'[' | b'{' | b'?') => return Ok(None),
            _ => match self.take_plain_key() {
                Some(key) => key,
                None => {
                    self.restore(state);
                    return Ok(None);
                }
            },
        };

        self.skip_inline_space();
        if self.peek() == Some(b':')
            && matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.bump();
            return Ok(Some(key));
        }

        self.restore(state);
        Ok(None)
    }

    /// Stops at the `:` that ends it.
    fn take_plain_key(&mut self) -> Option<Node> {
        let start = self.position();
        let mut end = start;
        let mut after_space = false;

        loop {
            match self.peek() {
                None | Some(b'\n') => return None,
                Some(b':')
                    if matches!(self.peek_at(1), None | Some(b' ' | b'\t' | b'\n' | b'\r')) =>
                {
                    break;
                }
                Some(b'#') if after_space => return None,
                _ => {
                    let ch = self.bump_char()?;
                    after_space = ch.is_whitespace();
                    if !after_space {
                        end = self.position();
                    }
                }
            }
        }

        let span = Span::new(start, end);
        Some(Node {
            value: resolve_plain(span.of(self.source)),
            span,
        })
    }

    // --- values -----------------------------------------------------------

    fn parse_value(&mut self, parent: u32) -> Result<Node, Error> {
        match self.peek() {
            Some(b'[') => self.parse_flow_sequence(),
            Some(b'{') => self.parse_flow_mapping(),
            Some(b'|' | b'>') => self.parse_block_scalar(parent),
            Some(b'\'') => self.parse_single_quoted(),
            Some(b'"') => self.parse_double_quoted(),
            Some(b'&' | b'*') => Err(self.error(
                "anchors and aliases are not supported, and GitHub Actions does not accept them either",
            )),
            _ => Ok(self.parse_plain(parent)),
        }
    }

    /// May fold onto the lines below it.
    fn parse_plain(&mut self, parent: u32) -> Node {
        let start = self.position();
        let (mut text, mut end) = self.take_plain_line();

        loop {
            let state = self.save();
            let before = self.line;
            self.skip_trivia();

            // Only a line indented past the block this scalar belongs to continues it.
            if self.eof() || self.column <= parent {
                self.restore(state);
                break;
            }

            // Blank lines between are a paragraph break rather than a fold.
            if self.line > before + 1 {
                text.push('\n');
            } else {
                text.push(' ');
            }

            let (more, stop) = self.take_plain_line();
            text.push_str(&more);
            end = stop;
        }

        let span = Span::new(start, end);
        Node {
            value: resolve_plain(&text),
            span,
        }
    }

    /// Stops before a comment, and trims what trails.
    fn take_plain_line(&mut self) -> (String, Position) {
        let mut text = String::new();
        let mut end = self.position();
        let mut after_space = false;

        loop {
            match self.peek() {
                None | Some(b'\n') => break,
                Some(b'#') if after_space => break,
                _ => {
                    let Some(ch) = self.bump_char() else { break };
                    text.push(ch);
                    after_space = ch.is_whitespace();
                    if !after_space {
                        end = self.position();
                    }
                }
            }
        }

        let trimmed = text.trim_end().to_owned();
        (trimmed, end)
    }

    fn parse_single_quoted(&mut self) -> Result<Node, Error> {
        let start = self.position();
        self.bump();
        let mut text = String::new();

        loop {
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b'\'') => {
                    self.bump();
                    // Two quotes in a row are one quote in the value.
                    if self.peek() == Some(b'\'') {
                        text.push('\'');
                        self.bump();
                    } else {
                        break;
                    }
                }
                Some(b'\n') => {
                    self.bump();
                    self.skip_inline_space();
                    text.push(' ');
                }
                _ => {
                    if let Some(ch) = self.bump_char() {
                        text.push(ch);
                    }
                }
            }
        }

        Ok(Node {
            value: Value::String(text),
            span: Span::new(start, self.position()),
        })
    }

    fn parse_double_quoted(&mut self) -> Result<Node, Error> {
        let start = self.position();
        self.bump();
        let mut text = String::new();

        loop {
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.bump();
                    break;
                }
                Some(b'\\') => {
                    self.bump();
                    text.push(self.escape()?);
                }
                Some(b'\n') => {
                    self.bump();
                    self.skip_inline_space();
                    text.push(' ');
                }
                _ => {
                    if let Some(ch) = self.bump_char() {
                        text.push(ch);
                    }
                }
            }
        }

        Ok(Node {
            value: Value::String(text),
            span: Span::new(start, self.position()),
        })
    }

    fn escape(&mut self) -> Result<char, Error> {
        let Some(byte) = self.peek() else {
            return Err(self.error("unterminated escape"));
        };
        self.bump();

        Ok(match byte {
            b'n' => '\n',
            b't' => '\t',
            b'r' => '\r',
            b'0' => '\0',
            b'a' => '\u{7}',
            b'b' => '\u{8}',
            b'v' => '\u{b}',
            b'f' => '\u{c}',
            b'e' => '\u{1b}',
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b' ' => ' ',
            b'x' => self.escaped_code(2)?,
            b'u' => self.escaped_code(4)?,
            b'U' => self.escaped_code(8)?,
            other => return Err(self.error(format!("unknown escape `\\{}`", other as char))),
        })
    }

    fn escaped_code(&mut self, digits: usize) -> Result<char, Error> {
        let start = self.offset;
        for _ in 0..digits {
            if !self.peek().is_some_and(|byte| byte.is_ascii_hexdigit()) {
                return Err(self.error("expected a hexadecimal escape"));
            }
            self.bump();
        }

        u32::from_str_radix(&self.source[start..self.offset], 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| self.error("escape is not a character"))
    }

    /// The body is every line indented past the parent.
    fn parse_block_scalar(&mut self, parent: u32) -> Result<Node, Error> {
        let start = self.position();
        let folded = self.peek() == Some(b'>');
        self.bump();

        let mut chomp = Chomp::Clip;
        let mut indent = None;
        loop {
            match self.peek() {
                Some(b'-') => chomp = Chomp::Strip,
                Some(b'+') => chomp = Chomp::Keep,
                Some(digit @ b'1'..=b'9') => indent = Some(parent + u32::from(digit - b'0')),
                _ => break,
            }
            self.bump();
        }

        self.skip_inline_space();
        if self.peek() == Some(b'#') {
            self.take_comment(false);
        }
        if !self.at_line_end() {
            return Err(self.error("unexpected text after a block scalar header"));
        }
        self.bump();

        let mut lines: Vec<Option<String>> = Vec::new();
        let mut end = self.position();
        let mut closed = true;

        while !self.eof() {
            let (content_column, blank) = self.measure_line();

            if blank {
                match indent {
                    // Whitespace reaching past the indentation is content, not a blank line.
                    Some(found) if content_column > found => {}
                    _ => {
                        lines.push(None);
                        self.skip_line();
                        continue;
                    }
                }
            } else {
                // The first line with content sets the indentation, unless it was given.
                let found = *indent.get_or_insert(content_column);
                if content_column < found {
                    break;
                }
            }

            for _ in 0..indent.unwrap_or(content_column) {
                if !matches!(self.peek(), Some(b' ' | b'\t')) {
                    break;
                }
                self.bump();
            }

            let mut text = String::new();
            while !self.eof() && self.peek() != Some(b'\n') {
                if let Some(ch) = self.bump_char() {
                    text.push(ch);
                }
            }
            end = self.position();
            lines.push(Some(text.trim_end_matches('\r').to_owned()));

            // A block that runs into the end of the file has no closing line break, and so
            // nothing for chomping to keep.
            closed = self.peek() == Some(b'\n');
            self.bump();
        }

        Ok(Node {
            value: Value::String(assemble(&lines, folded, chomp, closed)),
            span: Span::new(start, end),
        })
    }

    /// Where the content starts and whether there is any, without moving the cursor.
    fn measure_line(&self) -> (u32, bool) {
        let bytes = self.source.as_bytes();
        let mut at = self.offset;
        let mut column = self.column;

        while matches!(bytes.get(at), Some(b' ' | b'\t')) {
            at += 1;
            column += 1;
        }

        (column, matches!(bytes.get(at), None | Some(b'\n' | b'\r')))
    }

    // --- flow collections --------------------------------------------------

    fn parse_flow_sequence(&mut self) -> Result<Node, Error> {
        let start = self.position();
        self.bump();
        let mut items = Vec::new();

        loop {
            self.skip_trivia();
            match self.peek() {
                None => return Err(self.error("unterminated `[`")),
                Some(b']') => break,
                _ => {}
            }

            items.push(self.parse_flow_node()?);
            self.skip_trivia();
            match self.peek() {
                Some(b',') => self.bump(),
                Some(b']') => break,
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
        self.bump();

        Ok(Node {
            value: Value::Sequence(items),
            span: Span::new(start, self.position()),
        })
    }

    fn parse_flow_mapping(&mut self) -> Result<Node, Error> {
        let start = self.position();
        self.bump();
        let mut mapping = Mapping::default();

        loop {
            self.skip_trivia();
            match self.peek() {
                None => return Err(self.error("unterminated `{`")),
                Some(b'}') => break,
                _ => {}
            }

            let key = self.parse_flow_node()?;
            self.skip_trivia();
            if self.peek() != Some(b':') {
                return Err(self.error("expected `:` after a key"));
            }
            self.bump();

            self.skip_trivia();
            let value = if matches!(self.peek(), Some(b',' | b'}')) {
                self.null_here()
            } else {
                self.parse_flow_node()?
            };
            mapping.push(key, value);

            self.skip_trivia();
            match self.peek() {
                Some(b',') => self.bump(),
                Some(b'}') => break,
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
        self.bump();

        Ok(Node {
            value: Value::Mapping(mapping),
            span: Span::new(start, self.position()),
        })
    }

    fn parse_flow_node(&mut self) -> Result<Node, Error> {
        match self.peek() {
            Some(b'[') => self.parse_flow_sequence(),
            Some(b'{') => self.parse_flow_mapping(),
            Some(b'\'') => self.parse_single_quoted(),
            Some(b'"') => self.parse_double_quoted(),
            _ => Ok(self.parse_flow_plain()),
        }
    }

    /// Inside `[]` or `{}`, where the punctuation around it is what ends it.
    fn parse_flow_plain(&mut self) -> Node {
        let start = self.position();
        let mut end = start;
        let mut after_space = false;

        loop {
            match self.peek() {
                None | Some(b',' | b']' | b'}' | b'\n') => break,
                Some(b'#') if after_space => break,
                Some(b':')
                    if matches!(
                        self.peek_at(1),
                        None | Some(b' ' | b'\t' | b'\n' | b',' | b']' | b'}')
                    ) =>
                {
                    break;
                }
                _ => {
                    let Some(ch) = self.bump_char() else { break };
                    after_space = ch.is_whitespace();
                    if !after_space {
                        end = self.position();
                    }
                }
            }
        }

        let span = Span::new(start, end);
        Node {
            value: resolve_plain(span.of(self.source)),
            span,
        }
    }
}

/// Folds the lines and trims the end as the chomping asks.
fn assemble(lines: &[Option<String>], folded: bool, chomp: Chomp, closed: bool) -> String {
    let last_content = lines.iter().rposition(|line| line.is_some());
    let Some(last_content) = last_content else {
        // Nothing but blank lines: only `+` keeps them.
        return match chomp {
            Chomp::Keep => "\n".repeat(lines.len()),
            _ => String::new(),
        };
    };

    let mut body = String::new();
    for (position, line) in lines[..=last_content].iter().enumerate() {
        let Some(text) = line else {
            body.push('\n');
            continue;
        };

        if position > 0 {
            let previous = lines[position - 1].as_deref();
            let indented = text.starts_with(' ') || previous.is_some_and(|p| p.starts_with(' '));
            match previous {
                // Folding joins onto the line before, unless either is indented further,
                // in which case it stays as it was written.
                Some(_) if folded && !indented => body.push(' '),
                Some(_) => body.push('\n'),
                // A blank line ends a paragraph, and has already contributed its newline.
                None if folded => {}
                None => body.push('\n'),
            }
        }
        body.push_str(text);
    }

    if !closed {
        return body;
    }

    match chomp {
        Chomp::Strip => body,
        Chomp::Clip => {
            body.push('\n');
            body
        }
        Chomp::Keep => {
            let trailing = lines.len() - last_content - 1;
            body.push('\n');
            body.push_str(&"\n".repeat(trailing));
            body
        }
    }
}

/// By the YAML 1.2 core schema, so `on` and `yes` stay strings.
fn resolve_plain(text: &str) -> Value {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => {
            return Value::Float(f64::INFINITY);
        }
        "-.inf" | "-.Inf" | "-.INF" => return Value::Float(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return Value::Float(f64::NAN),
        _ => {}
    }

    if let Some(number) = parse_int(text) {
        return Value::Int(number);
    }
    // Rust parses `inf` and `nan` as numbers where YAML would not, so only text that looks
    // like a number is offered to it at all.
    if looks_numeric(text)
        && let Ok(number) = text.parse::<f64>()
    {
        return Value::Float(number);
    }

    Value::String(text.to_owned())
}

/// In any of the bases the core schema allows.
fn parse_int(text: &str) -> Option<i64> {
    if let Some(hex) = text.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok();
    }
    if let Some(octal) = text.strip_prefix("0o") {
        return i64::from_str_radix(octal, 8).ok();
    }
    text.parse().ok()
}

fn looks_numeric(text: &str) -> bool {
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);

    body.starts_with(|ch: char| ch.is_ascii_digit() || ch == '.')
        && body
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
}
