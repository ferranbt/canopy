//! The encodings the service wraps a job's JSON in, undone.
//!
//! Contexts arrive as `{"t": 2, "d": [{"k": .., "v": ..}]}` for a dictionary and
//! `{"t": 1, "a": [..]}` for an array. Steps arrive in a second encoding of their own:
//! `{"type": 0, "lit": ..}` for a literal, `{"type": 1, "seq": [..]}` for a sequence and
//! `{"type": 2, "map": [{"Key": .., "Value": ..}]}` for a mapping. An expression uses
//! `{"type": 3, "expr": ..}` and is put back into the `${{ .. }}` form the runner evaluates.
//! Both say what they are in a key of their own, so both can be undone before anything is
//! given a type.

use serde_json::{Map, Value};

/// A null here always means the service had nothing to send rather than that it sent
/// nothing on purpose, so dropping them is what lets every field simply have a default.
pub fn normalize(value: Value) -> Value {
    let Value::Object(fields) = value else {
        if let Value::Array(items) = value {
            return Value::Array(items.into_iter().map(normalize).collect());
        }
        return value;
    };

    let fields: Map<String, Value> = fields
        .into_iter()
        .map(|(key, value)| (key, normalize(value)))
        .filter(|(_, value)| !value.is_null())
        .collect();

    if let Some(kind) = fields.get("t").and_then(Value::as_u64) {
        return match kind {
            1 => items(fields, "a"),
            _ => entries(fields, "d", "k", "v"),
        };
    }

    if let Some(kind) = fields.get("type").and_then(Value::as_u64) {
        return match kind {
            0 => fields.get("lit").cloned().unwrap_or(Value::Null),
            1 => items(fields, "seq"),
            2 => entries(fields, "map", "Key", "Value"),
            3 => expression(&fields),
            _ => Value::Object(fields),
        };
    }

    Value::Object(fields)
}

fn expression(fields: &Map<String, Value>) -> Value {
    fields
        .get("expr")
        .and_then(Value::as_str)
        .map(|source| Value::String(format!("${{{{ {source} }}}}")))
        .unwrap_or(Value::Null)
}

fn items(mut fields: Map<String, Value>, under: &str) -> Value {
    match fields.remove(under) {
        Some(items @ Value::Array(_)) => items,
        _ => Value::Array(Vec::new()),
    }
}

fn entries(mut fields: Map<String, Value>, under: &str, key: &str, value: &str) -> Value {
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
                let name = entry.remove(key)?.as_str()?.to_owned();
                Some((name, entry.remove(value).unwrap_or(Value::Null)))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_encodings_come_out_as_the_json_they_stand_for() {
        let value = normalize(serde_json::json!({
            "contextData": {
                "github": {"t": 2, "d": [
                    {"k": "run_id", "v": "32465023908"},
                    {"k": "event", "v": {"t": 2, "d": [
                        {"k": "commits", "v": {"t": 1, "a": ["first", "second"]}}
                    ]}}
                ]},
                "matrix": null
            },
            "steps": [{
                "type": "action",
                "displayNameToken": {"type": 0, "file": 1, "line": 13, "lit": "Checkout"},
                "inputs": {"type": 2, "map": [
                    {"Key": {"type": 0, "lit": "go-version"}, "Value": {"type": 0, "lit": "1.25"}}
                ]},
                "timeoutInMinutes": null
            }]
        }));

        assert_eq!(
            value,
            serde_json::json!({
                "contextData": {
                    "github": {
                        "run_id": "32465023908",
                        "event": {"commits": ["first", "second"]}
                    }
                },
                "steps": [{
                    "type": "action",
                    "displayNameToken": "Checkout",
                    "inputs": {"go-version": "1.25"}
                }]
            })
        );
    }

    #[test]
    fn an_expression_token_returns_to_its_embedded_form() {
        let value = normalize(serde_json::json!({
            "steps": [{
                "type": "action",
                "displayNameToken": {
                    "type": 0,
                    "file": 1,
                    "line": 13,
                    "lit": "Checkout"
                },
                "inputs": {"type": 2, "map": [
                    {
                        "Key": {"type": 0, "lit": "submodules"},
                        "Value": {"type": 0, "lit": "true"}
                    },
                    {
                        "Key": {"type": 0, "lit": "token"},
                        "Value": {
                            "type": 3,
                            "file": 1,
                            "line": 16,
                            "expr": "secrets.GH_TOKEN"
                        }
                    }
                ]}
            }]
        }));

        assert_eq!(
            value,
            serde_json::json!({
                "steps": [{
                    "type": "action",
                    "displayNameToken": "Checkout",
                    "inputs": {
                        "submodules": "true",
                        "token": "${{ secrets.GH_TOKEN }}"
                    }
                }]
            })
        );
    }
}
