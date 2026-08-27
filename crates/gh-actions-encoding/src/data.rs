//! What a job is told about the run it belongs to: dictionaries and arrays are wrapped in a
//! type of their own, and everything else is sent as the JSON it already is.

use serde::de::DeserializeOwned;
use serde::{Deserializer, Serialize, Serializer};
use serde_json::Value;

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

/// The same, for a field the service sends `null` for when there is none.
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
        if let Value::Array(of) = value {
            return Value::Array(of.into_iter().map(read).collect());
        }
        return value;
    };

    match fields.get("t").and_then(Value::as_u64) {
        Some(1) => items(fields, "a", read),
        // 2 and 5, which differ only in whether a key keeps the case it was written in.
        Some(_) => entries(fields, "d", "k", "v", read),
        None => Value::Object(fields.into_iter().map(|(at, it)| (at, read(it))).collect()),
    }
}

pub fn written(value: Value) -> Value {
    match value {
        Value::Array(of) => {
            let items: Vec<Value> = of.into_iter().map(written).collect();
            serde_json::json!({ "t": 1, "a": items })
        }
        Value::Object(fields) => {
            let of: Vec<Value> = fields
                .into_iter()
                .map(|(key, value)| serde_json::json!({ "k": key, "v": written(value) }))
                .collect();
            serde_json::json!({ "t": 2, "d": of })
        }
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
    #[serde(default)]
    struct Contexts {
        #[serde(with = "crate::data")]
        strategy: Strategy,
        #[serde(with = "crate::data")]
        github: Github,
        #[serde(with = "crate::data")]
        inputs: BTreeMap<String, String>,
    }

    #[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
    #[serde(default)]
    struct Strategy {
        #[serde(rename = "fail-fast")]
        fail_fast: bool,
        #[serde(rename = "job-index")]
        job_index: u64,
    }

    #[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
    #[serde(default)]
    struct Github {
        repository: String,
        event: serde_json::Value,
    }

    fn contexts(raw: &str) -> Contexts {
        serde_json::from_str(raw).expect("the contexts read")
    }

    #[test]
    fn a_dictionary_is_the_pairs_it_was_sent_as() {
        let read = contexts(
            r#"{
                "strategy": {"t": 2, "d": [
                    {"k": "fail-fast", "v": true},
                    {"k": "job-index", "v": 0}
                ]},
                "github": {"t": 2, "d": [{"k": "repository", "v": "ferranbt/canopy"}]},
                "inputs": {"t": 2, "d": [{"k": "label", "v": "canopy-probe-1"}]}
            }"#,
        );

        assert!(read.strategy.fail_fast, "a scalar stands for itself");
        assert_eq!(read.strategy.job_index, 0);
        assert_eq!(read.github.repository, "ferranbt/canopy");
        assert_eq!(read.inputs["label"], "canopy-probe-1");
    }

    #[test]
    fn what_is_nested_is_undone_too() {
        let read = contexts(
            r#"{
                "github": {"t": 2, "d": [
                    {"k": "event", "v": {"t": 2, "d": [
                        {"k": "commits", "v": {"t": 1, "a": ["first", "second"]}},
                        {"k": "private", "v": false}
                    ]}}
                ]},
                "strategy": {"t": 2, "d": []},
                "inputs": null
            }"#,
        );

        assert_eq!(
            read.github.event,
            serde_json::json!({"commits": ["first", "second"], "private": false})
        );
        assert!(read.inputs.is_empty(), "and a null is nothing to say");
    }

    #[test]
    fn the_contexts_are_written_back_in_the_shape_they_came_in() {
        let read = contexts(
            r#"{
                "strategy": {"t": 2, "d": [{"k": "fail-fast", "v": true}]},
                "github": {"t": 2, "d": [{"k": "repository", "v": "ferranbt/canopy"}]},
                "inputs": {"t": 2, "d": [{"k": "label", "v": "canopy-probe-1"}]}
            }"#,
        );

        let written = serde_json::to_value(&read).expect("the contexts write");
        assert_eq!(
            written["inputs"],
            serde_json::json!({"t": 2, "d": [{"k": "label", "v": "canopy-probe-1"}]})
        );
        assert_eq!(
            written["strategy"]["d"][0],
            serde_json::json!({"k": "fail-fast", "v": true})
        );
        assert_eq!(
            serde_json::from_value::<Contexts>(written).expect("and read back"),
            read
        );
    }
}
