//! What the service compiled a workflow into: a tree of tokens, each saying what kind of
//! thing it is and, for a literal, keeping its value under a key named after that kind.

use serde::de::DeserializeOwned;
use serde::{Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::{entries, items, read_with, write_with};

pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    read_with(deserializer, read)
}

pub fn serialize<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    write_with(value, serializer, written)
}

/// The same, for a field the service sends `null` for when a workflow left it out.
pub mod maybe {
    use super::{DeserializeOwned, Deserializer, Serialize, Serializer, Value};
    use serde::Deserialize;

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned + Default,
    {
        match Value::deserialize(deserializer)? {
            Value::Null => Ok(None),
            value => crate::of(super::read(value)).map(Some),
        }
    }

    pub fn serialize<S, T>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        match value {
            Some(value) => super::serialize(value, serializer),
            None => serializer.serialize_none(),
        }
    }
}

pub(crate) fn read(value: Value) -> Value {
    let Value::Object(fields) = value else {
        if let Value::Array(items) = value {
            return Value::Array(items.into_iter().map(read).collect());
        }
        return value;
    };

    match fields.get("type").and_then(Value::as_u64) {
        Some(1) => items(fields, "seq", read),
        Some(2) => entries(fields, "map", "Key", "Value", read),
        Some(3) => expression(&fields),
        Some(_) => literal(fields),
        None => Value::Object(fields.into_iter().map(|(at, it)| (at, read(it))).collect()),
    }
}

fn written(value: Value) -> Value {
    match value {
        Value::String(said) => match source(&said) {
            Some(source) => serde_json::json!({ "type": 3, "expr": source }),
            None => serde_json::json!({ "type": 0, "lit": said }),
        },
        Value::Bool(value) => serde_json::json!({ "type": 5, "bool": value }),
        Value::Number(value) => serde_json::json!({ "type": 6, "num": value }),
        Value::Array(of) => {
            let seq: Vec<Value> = of.into_iter().map(written).collect();
            serde_json::json!({ "type": 1, "seq": seq })
        }
        Value::Object(fields) => {
            let map: Vec<Value> = fields
                .into_iter()
                .map(|(key, value)| {
                    serde_json::json!({
                        "Key": { "type": 0, "lit": key },
                        "Value": written(value),
                    })
                })
                .collect();
            serde_json::json!({ "type": 2, "map": map })
        }
        Value::Null => Value::Null,
    }
}

/// A literal keeps its value under a key named after the kind of literal it is.
fn literal(mut fields: Map<String, Value>) -> Value {
    match ["lit", "bool", "num"]
        .into_iter()
        .find_map(|kind| fields.remove(kind))
    {
        Some(value) => value,
        None => Value::Object(fields),
    }
}

fn expression(fields: &Map<String, Value>) -> Value {
    fields
        .get("expr")
        .and_then(Value::as_str)
        .map(|source| Value::String(format!("${{{{ {source} }}}}")))
        .unwrap_or(Value::Null)
}

/// What an expression stands for, where a string is one and nothing besides.
fn source(said: &str) -> Option<&str> {
    let inner = said.strip_prefix("${{")?.strip_suffix("}}")?;

    (!inner.contains("${{")).then(|| inner.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
    struct Step {
        #[serde(rename = "displayNameToken", with = "crate::token")]
        name: String,
        #[serde(default, with = "crate::token::maybe")]
        continue_on_error: Option<bool>,
        #[serde(default, with = "crate::token::maybe")]
        timeout_in_minutes: Option<u64>,
        #[serde(with = "crate::token")]
        inputs: BTreeMap<String, String>,
    }

    fn step(raw: &str) -> Step {
        serde_json::from_str(raw).expect("a step reads")
    }

    #[test]
    fn a_literal_is_known_by_the_kind_of_literal_it_is() {
        let read = step(
            r#"{
                "displayNameToken": {"type": 0, "file": 1, "line": 85, "lit": "A step"},
                "continue_on_error": {"type": 5, "line": 86, "col": 28, "bool": true},
                "timeout_in_minutes": {"type": 6, "num": 5},
                "inputs": {"type": 2, "map": [
                    {"Key": {"type": 0, "lit": "script"}, "Value": {"type": 0, "lit": "exit 3"}}
                ]}
            }"#,
        );

        assert_eq!(read.name, "A step");
        assert_eq!(read.continue_on_error, Some(true));
        assert_eq!(read.timeout_in_minutes, Some(5));
        assert_eq!(read.inputs["script"], "exit 3");
    }

    #[test]
    fn what_a_workflow_left_out_is_left_out() {
        let read = step(
            r#"{
                "displayNameToken": {"type": 0, "lit": "A step"},
                "continue_on_error": null,
                "timeout_in_minutes": null,
                "inputs": null
            }"#,
        );

        assert_eq!(read.continue_on_error, None);
        assert_eq!(read.timeout_in_minutes, None);
        assert!(read.inputs.is_empty(), "and a null map is an empty one");
    }

    #[test]
    fn an_expression_comes_back_as_the_form_a_workflow_writes() {
        let read = step(
            r#"{
                "displayNameToken": {"type": 0, "lit": "A step"},
                "inputs": {"type": 2, "map": [
                    {"Key": {"type": 0, "lit": "script"},
                     "Value": {"type": 3, "line": 30, "expr": "secrets.TOKEN"}}
                ]}
            }"#,
        );

        assert_eq!(read.inputs["script"], "${{ secrets.TOKEN }}");
    }

    #[test]
    fn a_step_is_written_back_in_the_shape_it_came_in() {
        let read = step(
            r#"{
                "displayNameToken": {"type": 0, "lit": "A step"},
                "continue_on_error": {"type": 5, "bool": true},
                "inputs": {"type": 2, "map": [
                    {"Key": {"type": 0, "lit": "script"},
                     "Value": {"type": 3, "expr": "secrets.TOKEN"}}
                ]}
            }"#,
        );

        let written = serde_json::to_value(&read).expect("a step writes");
        assert_eq!(
            written["displayNameToken"],
            serde_json::json!({"type": 0, "lit": "A step"})
        );
        assert_eq!(
            written["continue_on_error"],
            serde_json::json!({"type": 5, "bool": true})
        );
        assert_eq!(
            written["inputs"],
            serde_json::json!({"type": 2, "map": [{
                "Key": {"type": 0, "lit": "script"},
                "Value": {"type": 3, "expr": "secrets.TOKEN"},
            }]})
        );
        assert_eq!(
            serde_json::from_value::<Step>(written).expect("and reads back"),
            read
        );
    }
}
