//! Evaluates an [`Expr`] against a set of contexts.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::ast::{BinaryOp, Expr};
use crate::error::EvalError;
use crate::functions;
use crate::value::Value;

/// The contexts and job status an expression is evaluated against.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    /// By name, e.g. `github`, `env`, `matrix`, `steps`.
    pub contexts: BTreeMap<String, Value>,
    /// Backs `success()`, `failure()` and `cancelled()`.
    pub status: Status,
    /// What `hashFiles()` reads its patterns against.
    pub workspace: Option<PathBuf>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    /// One context per field of the object; anything else has none.
    pub fn from_value(value: Value, status: Status) -> Self {
        let contexts = match value {
            Value::Object(fields) => fields,
            _ => BTreeMap::new(),
        };
        Self {
            contexts,
            status,
            workspace: None,
        }
    }

    pub fn with(mut self, name: impl Into<String>, value: Value) -> Self {
        self.contexts.insert(name.into(), value);
        self
    }

    pub fn with_status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }

    pub fn with_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }
}

/// How the job has gone so far, as the status functions see it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Success,
    Failure,
    Cancelled,
}

impl Status {
    pub fn is_success(self) -> bool {
        self == Self::Success
    }

    pub fn is_failure(self) -> bool {
        self == Self::Failure
    }

    pub fn is_cancelled(self) -> bool {
        self == Self::Cancelled
    }
}

pub fn eval(expr: &Expr, context: &Context) -> Result<Value, EvalError> {
    match expr {
        Expr::Null => Ok(Value::Null),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::Number(n) => Ok(Value::Number(*n)),
        Expr::String(s) => Ok(Value::String(s.clone())),
        Expr::Context(name) => context
            .contexts
            .get(name)
            .cloned()
            .ok_or_else(|| EvalError::UnknownContext(name.clone())),
        Expr::Property(target, name) => Ok(property(&eval(target, context)?, name)),
        Expr::Index(target, index) => {
            let target = eval(target, context)?;
            let index = eval(index, context)?;
            Ok(self::index(&target, &index))
        }
        Expr::Star(target) => Ok(star(&eval(target, context)?)),
        Expr::Not(inner) => Ok(Value::Bool(!eval(inner, context)?.truthy())),
        Expr::Binary(op, left, right) => binary(*op, left, right, context),
        Expr::Call(name, args) => {
            let args: Vec<Value> = args
                .iter()
                .map(|arg| eval(arg, context))
                .collect::<Result<_, _>>()?;
            functions::call(name, &args, context)
        }
    }
}

/// On an array this filters, collecting the property of every element.
fn property(target: &Value, name: &str) -> Value {
    match target {
        Value::Object(fields) => fields
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map_or(Value::Null, |(_, value)| value.clone()),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| property(item, name))
                .filter(|value| *value != Value::Null)
                .collect(),
        ),
        _ => Value::Null,
    }
}

/// Numeric on arrays, name on objects.
fn index(target: &Value, index: &Value) -> Value {
    match target {
        Value::Array(items) => {
            let n = index.to_number();
            if n.is_nan() || n < 0.0 || n.fract() != 0.0 {
                return Value::Null;
            }
            items.get(n as usize).cloned().unwrap_or(Value::Null)
        }
        Value::Object(_) => property(target, &index.to_display_string()),
        _ => Value::Null,
    }
}

/// Every element of an array, or every value of an object.
fn star(target: &Value) -> Value {
    match target {
        Value::Array(items) => Value::Array(items.clone()),
        Value::Object(fields) => Value::Array(fields.values().cloned().collect()),
        _ => Value::Array(Vec::new()),
    }
}

fn binary(op: BinaryOp, left: &Expr, right: &Expr, context: &Context) -> Result<Value, EvalError> {
    let lhs = eval(left, context)?;

    // These two return one of their operands, not a bool.
    match op {
        BinaryOp::And if !lhs.truthy() => return Ok(lhs),
        BinaryOp::And => return eval(right, context),
        BinaryOp::Or if lhs.truthy() => return Ok(lhs),
        BinaryOp::Or => return eval(right, context),
        _ => {}
    }

    let rhs = eval(right, context)?;
    let result = match op {
        BinaryOp::Eq => lhs.loose_eq(&rhs),
        BinaryOp::NotEq => !lhs.loose_eq(&rhs),
        BinaryOp::Lt => matches!(lhs.loose_cmp(&rhs), Some(std::cmp::Ordering::Less)),
        BinaryOp::LtEq => matches!(
            lhs.loose_cmp(&rhs),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        BinaryOp::Gt => matches!(lhs.loose_cmp(&rhs), Some(std::cmp::Ordering::Greater)),
        BinaryOp::GtEq => matches!(
            lhs.loose_cmp(&rhs),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
        BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
    };

    Ok(Value::Bool(result))
}
