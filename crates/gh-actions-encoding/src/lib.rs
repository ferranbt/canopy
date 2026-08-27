//! The two encodings the service wraps a job's JSON in, undone as serde reads a field and
//! put back as serde writes it.
//!
//! Contexts are [`data`]: `{"t": 2, "d": [{"k": .., "v": ..}]}` for a dictionary and
//! `{"t": 1, "a": [..]}` for an array, with anything else sent as the JSON it already is.
//! What the service compiled a workflow into is a [`token`]: `{"type": 0, "lit": ..}` for a
//! string, `{"type": 5, "bool": ..}` for a boolean, `{"type": 6, "num": ..}` for a number,
//! `{"type": 1, "seq": [..]}` for a sequence, `{"type": 2, "map": [{"Key": .., "Value": ..}]}`
//! for a mapping, and `{"type": 3, "expr": ..}` for an expression, which is put back into
//! the `${{ .. }}` form the runner evaluates.
//!
//! A field says which it arrived in, and serde does the rest:
//!
//! ```ignore
//! #[serde(alias = "displayNameToken", with = "gh_actions_encoding::token")]
//! pub display_name: String,
//! #[serde(default, with = "gh_actions_encoding::token::maybe")]
//! pub continue_on_error: Option<bool>,
//! #[serde(with = "gh_actions_encoding::data")]
//! pub strategy: StrategyContext,
//! ```
//!
//! A token also says where in the workflow it was written. Nothing here reads that, so it is
//! dropped on the way in and not invented on the way out.

pub mod data;
pub mod token;

use serde::de::DeserializeOwned;
use serde::ser::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A null is the service having nothing to send rather than sending nothing on purpose, so
/// a field it says nothing about is left at whatever it would have been.
pub(crate) fn of<T, E>(value: Value) -> Result<T, E>
where
    T: DeserializeOwned + Default,
    E: serde::de::Error,
{
    match value {
        Value::Null => Ok(T::default()),
        value => T::deserialize(value).map_err(E::custom),
    }
}

pub(crate) fn items(mut fields: Map<String, Value>, at: &str, read: fn(Value) -> Value) -> Value {
    match fields.remove(at) {
        Some(Value::Array(items)) => Value::Array(items.into_iter().map(read).collect()),
        _ => Value::Array(Vec::new()),
    }
}

/// A dictionary sent as the pairs it is made of, in the order they were written.
pub(crate) fn entries(
    mut fields: Map<String, Value>,
    under: &str,
    key: &str,
    value: &str,
    read: fn(Value) -> Value,
) -> Value {
    let Some(Value::Array(entries)) = fields.remove(under) else {
        return Value::Object(Map::new());
    };

    Value::Object(
        entries
            .into_iter()
            .filter_map(|entry| {
                let Value::Object(mut entry) = entry else {
                    return None;
                };
                let name = match read(entry.remove(key)?) {
                    Value::String(name) => name,
                    named => named.to_string(),
                };

                Some((name, read(entry.remove(value).unwrap_or(Value::Null))))
            })
            .collect(),
    )
}

/// Reads a field's own JSON, whichever encoding it is in, and hands it to the type.
pub(crate) fn read_with<'de, D, T>(deserializer: D, read: fn(Value) -> Value) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    of(read(Value::deserialize(deserializer)?))
}

pub(crate) fn write_with<S, T>(
    value: &T,
    serializer: S,
    written: fn(Value) -> Value,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: Serialize,
{
    let value = serde_json::to_value(value).map_err(S::Error::custom)?;
    written(value).serialize(serializer)
}
