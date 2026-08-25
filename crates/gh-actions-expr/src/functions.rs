//! The built-in functions of the expression language.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::error::EvalError;
use crate::eval::Context;
use crate::value::Value;

/// The arguments are already evaluated.
pub fn call(name: &str, args: &[Value], context: &Context) -> Result<Value, EvalError> {
    match name.to_ascii_lowercase().as_str() {
        "contains" => arity(name, args, 2).map(|()| contains(&args[0], &args[1])),
        "startswith" => arity(name, args, 2).map(|()| starts_with(&args[0], &args[1])),
        "endswith" => arity(name, args, 2).map(|()| ends_with(&args[0], &args[1])),
        "format" => format(name, args),
        "join" => join(name, args),
        "tojson" => arity(name, args, 1).map(|()| to_json(&args[0])),
        "fromjson" => arity(name, args, 1).and_then(|()| from_json(&args[0])),
        "hashfiles" => hash_files(args, context),
        "success" => arity(name, args, 0).map(|()| Value::Bool(context.status.is_success())),
        "failure" => arity(name, args, 0).map(|()| Value::Bool(context.status.is_failure())),
        "cancelled" => arity(name, args, 0).map(|()| Value::Bool(context.status.is_cancelled())),
        "always" => arity(name, args, 0).map(|()| Value::Bool(true)),
        _ => Err(EvalError::UnknownFunction(name.to_owned())),
    }
}

fn arity(name: &str, args: &[Value], expected: usize) -> Result<(), EvalError> {
    if args.len() == expected {
        return Ok(());
    }
    Err(EvalError::WrongArity {
        function: name.to_owned(),
        expected: expected.to_string(),
        got: args.len(),
    })
}

/// Substring on strings, loose membership on arrays.
fn contains(haystack: &Value, needle: &Value) -> Value {
    match haystack {
        Value::Array(items) => Value::Bool(items.iter().any(|item| item.loose_eq(needle))),
        _ => {
            let haystack = haystack.to_display_string().to_lowercase();
            let needle = needle.to_display_string().to_lowercase();
            Value::Bool(haystack.contains(&needle))
        }
    }
}

/// Case-insensitive.
fn starts_with(value: &Value, prefix: &Value) -> Value {
    let value = value.to_display_string().to_lowercase();
    let prefix = prefix.to_display_string().to_lowercase();
    Value::Bool(value.starts_with(&prefix))
}

/// Case-insensitive.
fn ends_with(value: &Value, suffix: &Value) -> Value {
    let value = value.to_display_string().to_lowercase();
    let suffix = suffix.to_display_string().to_lowercase();
    Value::Bool(value.ends_with(&suffix))
}

/// Replaces `{N}` placeholders with the remaining arguments; `{{` and `}}` are literal braces.
fn format(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    let Some((template, values)) = args.split_first() else {
        return Err(EvalError::WrongArity {
            function: name.to_owned(),
            expected: "at least 1".into(),
            got: 0,
        });
    };

    let template = template.to_display_string();
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                out.push('{');
                i += 2;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                out.push('}');
                i += 2;
            }
            '{' => {
                let start = i + 1;
                let end = chars[start..]
                    .iter()
                    .position(|c| *c == '}')
                    .map(|offset| start + offset)
                    .ok_or_else(|| EvalError::InvalidFormat("unclosed placeholder".to_owned()))?;
                let digits: String = chars[start..end].iter().collect();
                let index = digits.parse::<usize>().map_err(|_| {
                    EvalError::InvalidFormat(format!("bad placeholder {{{digits}}}"))
                })?;
                let value = values.get(index).ok_or_else(|| {
                    EvalError::InvalidFormat(format!("no argument for placeholder {{{index}}}"))
                })?;
                out.push_str(&value.to_display_string());
                i = end + 1;
            }
            '}' => return Err(EvalError::InvalidFormat("unmatched '}'".to_owned())),
            c => {
                out.push(c);
                i += 1;
            }
        }
    }

    Ok(Value::String(out))
}

/// The separator defaults to a comma.
fn join(name: &str, args: &[Value]) -> Result<Value, EvalError> {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongArity {
            function: name.to_owned(),
            expected: "1 or 2".into(),
            got: args.len(),
        });
    }

    let separator = args
        .get(1)
        .map_or_else(|| ",".to_owned(), Value::to_display_string);

    match &args[0] {
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(Value::to_display_string).collect();
            Ok(Value::String(parts.join(&separator)))
        }
        // A non-array joins to itself.
        other => Ok(Value::String(other.to_display_string())),
    }
}

/// Pretty-printed, as the runner writes it.
fn hash_files(args: &[Value], context: &Context) -> Result<Value, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongArity {
            function: "hashFiles".to_owned(),
            expected: "at least 1".to_owned(),
            got: 0,
        });
    }

    let Some(workspace) = &context.workspace else {
        return Err(EvalError::Unsupported("hashFiles"));
    };

    // Taken in the order they were asked for, since that is the order they are hashed in: a
    // pattern adds what it matches, and one written with a `!` takes back what it does.
    let mut matched: Vec<PathBuf> = Vec::new();
    for pattern in args {
        let asked = pattern.to_display_string();
        let (taking_back, pattern) = match asked.strip_prefix('!') {
            Some(rest) => (true, rest.to_owned()),
            None => (false, asked),
        };

        // Everything under a directory, which is what a pattern ending there asks for.
        let pattern = match pattern.strip_suffix("**") {
            Some(above) => format!("{above}**/*"),
            None => pattern,
        };
        let against = workspace.join(pattern);
        let found = glob::glob(&against.to_string_lossy())
            .map_err(|err| EvalError::InvalidPattern(err.to_string()))?;

        let mut found: Vec<PathBuf> = found.flatten().filter(|path| path.is_file()).collect();
        found.sort();

        if taking_back {
            matched.retain(|path| !found.contains(path));
            continue;
        }
        for path in found {
            if !matched.contains(&path) {
                matched.push(path);
            }
        }
    }

    if matched.is_empty() {
        return Ok(Value::String(String::new()));
    }

    let mut whole = Sha256::new();
    for path in matched {
        let bytes = std::fs::read(&path)
            .map_err(|err| EvalError::InvalidPattern(format!("{}: {err}", path.display())))?;
        whole.update(Sha256::digest(bytes));
    }

    let digest = whole
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    Ok(Value::String(digest))
}

fn to_json(value: &Value) -> Value {
    let json = serde_json::Value::from(value);
    Value::String(serde_json::to_string_pretty(&json).unwrap_or_else(|_| "null".to_owned()))
}

fn from_json(value: &Value) -> Result<Value, EvalError> {
    let source = value.to_display_string();
    serde_json::from_str::<serde_json::Value>(&source)
        .map(Into::into)
        .map_err(|err| EvalError::InvalidJson(err.to_string()))
}
