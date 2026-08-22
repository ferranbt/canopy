//! Runtime values and the coercion rules the language applies to them.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A value produced by evaluating an expression.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            Self::Array(_) | Self::Object(_) => true,
        }
    }

    pub fn to_number(&self) -> f64 {
        match self {
            Self::Null => 0.0,
            Self::Bool(true) => 1.0,
            Self::Bool(false) => 0.0,
            Self::Number(n) => *n,
            Self::String(s) => parse_number(s),
            Self::Array(_) | Self::Object(_) => f64::NAN,
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => format_number(*n),
            Self::String(s) => s.clone(),
            Self::Array(_) => "Array".into(),
            Self::Object(_) => "Object".into(),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    /// Loose equality: strings compare case-insensitively, mismatched types compare as numbers.
    pub fn loose_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::String(a), Self::String(b)) => a.eq_ignore_ascii_case(b),
            (Self::Null, Self::Null) => true,
            (Self::Array(a), Self::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.loose_eq(y))
            }
            (Self::Object(a), Self::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).is_some_and(|other| v.loose_eq(other)))
            }
            // Anything else falls back to a numeric comparison, so NaN is never equal.
            _ => {
                let (a, b) = (self.to_number(), other.to_number());
                !a.is_nan() && !b.is_nan() && a == b
            }
        }
    }

    /// Ordering comparison: two strings compare case-insensitively, everything else numerically.
    pub fn loose_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Self::String(a), Self::String(b)) => {
                Some(a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
            }
            _ => self.to_number().partial_cmp(&other.to_number()),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

pub fn to_value<T: Serialize + ?Sized>(value: &T) -> Result<Value, serde_json::Error> {
    serde_json::to_value(value).map(Value::from)
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(serde_json::Value::deserialize(deserializer)?.into())
    }
}

impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(b) => serializer.serialize_bool(*b),
            // NaN and the infinities have no JSON spelling.
            Self::Number(n) if !n.is_finite() => serializer.serialize_unit(),
            Self::Number(n) => serializer.serialize_f64(*n),
            Self::String(s) => serializer.serialize_str(s),
            Self::Array(items) => serializer.collect_seq(items),
            Self::Object(fields) => serializer.collect_map(fields),
        }
    }
}

impl From<serde_json::Value> for Value {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => Self::Number(n.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(s) => Self::String(s),
            serde_json::Value::Array(items) => {
                Self::Array(items.into_iter().map(Into::into).collect())
            }
            serde_json::Value::Object(fields) => Self::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key, value.into()))
                    .collect(),
            ),
        }
    }
}

impl From<&Value> for serde_json::Value {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(*b),
            Value::Number(n) => serde_json::Number::from_f64(*n).map_or(Self::Null, Self::Number),
            Value::String(s) => Self::String(s.clone()),
            Value::Array(items) => Self::Array(items.iter().map(Into::into).collect()),
            Value::Object(fields) => Self::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.into()))
                    .collect(),
            ),
        }
    }
}

/// Parses a string as a number, accepting hex literals and treating blank as zero.
fn parse_number(s: &str) -> f64 {
    let trimmed = s.trim();

    if trimmed.is_empty() {
        return 0.0;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).map_or(f64::NAN, |n| n as f64);
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// Renders a number without a trailing `.0`, matching how the runner prints them.
fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    n.to_string()
}
