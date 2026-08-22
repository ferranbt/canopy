//! The tree a document parses into.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub value: Value,
    /// The source it covers, quotes and all.
    pub span: Span,
}

/// A YAML value, resolved by the core schema.
///
/// `on`, `yes` and `no` are strings here, not booleans. YAML 1.1 read them as booleans and
/// that is why `on:` is such a famous nuisance; GitHub does not, and neither do we.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// An empty value, `~`, or `null`.
    Null,
    /// In any case, e.g. `True`.
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Everything else, quoted or not.
    String(String),
    Sequence(Vec<Node>),
    Mapping(Mapping),
}

/// Pairs, in the order they were written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mapping {
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Carries its own span, so a finding can point at the key rather than the pair.
    pub key: Node,
    pub value: Node,
}

impl Mapping {
    pub(crate) fn push(&mut self, key: Node, value: Node) {
        self.entries.push(Entry { key, value });
    }

    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entry(key).map(|entry| &entry.value)
    }

    /// For when the key's own position is what matters.
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.key.as_str() == Some(key))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Node {
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_mapping(&self) -> Option<&Mapping> {
        match &self.value {
            Value::Mapping(mapping) => Some(mapping),
            _ => None,
        }
    }

    pub fn as_sequence(&self) -> Option<&[Node]> {
        match &self.value {
            Value::Sequence(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.value, Value::Null)
    }

    pub fn get(&self, key: &str) -> Option<&Node> {
        self.as_mapping()?.get(key)
    }

    pub fn index(&self, position: usize) -> Option<&Node> {
        self.as_sequence()?.get(position)
    }
}
