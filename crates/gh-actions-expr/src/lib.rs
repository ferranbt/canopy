//! Parser and evaluator for the GitHub Actions expression language.

pub mod ast;
pub mod error;
pub mod eval;
pub mod functions;
pub mod lexer;
pub mod parser;
pub mod references;
pub mod template;
pub mod value;

use std::fmt;

pub use ast::{BinaryOp, Expr};
pub use error::{EvalError, ParseError};
pub use eval::{Context, Status, eval};
pub use references::{Reference, Segment};
pub use template::{interpolate, interpolate_value};
pub use value::{Value, to_value};

/// Either stage of expression handling failing.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Parse(ParseError),
    Eval(EvalError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "{err}"),
            Self::Eval(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ParseError> for Error {
    fn from(err: ParseError) -> Self {
        Self::Parse(err)
    }
}

impl From<EvalError> for Error {
    fn from(err: EvalError) -> Self {
        Self::Eval(err)
    }
}

impl Expr {
    /// What goes inside `${{ }}`, without the wrapper.
    pub fn parse(source: &str) -> Result<Self, ParseError> {
        parser::parse(source)
    }

    /// Parses an `if` condition, which may or may not be wrapped in `${{ }}`.
    pub fn parse_condition(source: &str) -> Result<Self, ParseError> {
        parser::parse(unwrap_condition(source))
    }

    pub fn eval(&self, context: &Context) -> Result<Value, EvalError> {
        eval::eval(self, context)
    }
}

/// Whether the step or job this condition belongs to should run.
pub fn eval_condition(source: &str, context: &Context) -> Result<bool, Error> {
    let expr = Expr::parse_condition(source)?;
    Ok(expr.eval(context)?.truthy())
}

/// Strips a single surrounding `${{ }}`, which `if` accepts but does not require.
fn unwrap_condition(source: &str) -> &str {
    let trimmed = source.trim();
    trimmed
        .strip_prefix("${{")
        .and_then(|rest| rest.strip_suffix("}}"))
        // Only a single wrapper counts; `${{ a }} ${{ b }}` is not a condition.
        .filter(|inner| !inner.contains("${{"))
        .unwrap_or(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Roughly the context a real run provides.
    fn context() -> Context {
        let github = Value::Object(BTreeMap::from([
            ("event_name".into(), Value::string("push")),
            ("ref_name".into(), Value::string("main")),
            ("actor".into(), Value::string("Octocat")),
            (
                "event".into(),
                Value::Object(BTreeMap::from([(
                    "commits".into(),
                    Value::Array(vec![
                        Value::Object(BTreeMap::from([("message".into(), Value::string("first"))])),
                        Value::Object(BTreeMap::from([(
                            "message".into(),
                            Value::string("second"),
                        )])),
                    ]),
                )])),
            ),
        ]));
        let matrix = Value::Object(BTreeMap::from([
            ("os".into(), Value::string("ubuntu-latest")),
            ("coverage".into(), Value::Bool(true)),
        ]));
        let inputs = Value::Object(BTreeMap::from([("dry-run".into(), Value::Bool(false))]));

        Context::new()
            .with("github", github)
            .with("matrix", matrix)
            .with("inputs", inputs)
    }

    fn truthy(source: &str) -> bool {
        eval_condition(source, &context()).expect("condition evaluates")
    }

    fn value(source: &str) -> Value {
        Expr::parse(source)
            .expect("expression parses")
            .eval(&context())
            .expect("expression evaluates")
    }

    #[test]
    fn literals_and_contexts() {
        assert_eq!(value("true"), Value::Bool(true));
        assert_eq!(value("null"), Value::Null);
        assert_eq!(value("-1.5e2"), Value::Number(-150.0));
        assert_eq!(value("0xff"), Value::Number(255.0));
        assert_eq!(value("'it''s here'"), Value::string("it's here"));
        assert_eq!(value("github.ref_name"), Value::string("main"));
        assert_eq!(value("matrix['os']"), Value::string("ubuntu-latest"));
        assert_eq!(value("github.missing.deeper"), Value::Null);
    }

    #[test]
    fn conditions_with_and_without_wrapper() {
        assert!(truthy("${{ github.event_name == 'push' }}"));
        assert!(truthy("github.event_name == 'push'"));
        assert!(!truthy("${{ github.event_name == 'pull_request' }}"));
    }

    #[test]
    fn string_comparison_ignores_case() {
        assert!(truthy("github.actor == 'octocat'"));
        assert!(truthy("startsWith(github.actor, 'OCTO')"));
        assert!(truthy("contains('Hello World', 'hello')"));
    }

    #[test]
    fn loose_equality_coerces_across_types() {
        assert!(truthy("1 == '1'"));
        assert!(truthy("null == 0"));
        assert!(truthy("true == 1"));
        assert!(truthy("'' == 0"));
        assert!(!truthy("'abc' == 0"));
    }

    #[test]
    fn logical_operators_return_operands() {
        assert_eq!(value("'' || 'fallback'"), Value::string("fallback"));
        assert_eq!(value("'set' || 'fallback'"), Value::string("set"));
        assert_eq!(value("null && 'unused'"), Value::Null);
        assert!(truthy("!inputs.dry-run && matrix.coverage"));
    }

    #[test]
    fn precedence_matches_the_language() {
        assert!(truthy("false && false || true"));
        assert!(!truthy("false && (false || true)"));
        assert!(truthy("!false == true"));
        assert!(truthy("1 < 2 == true"));
    }

    #[test]
    fn star_filter_collects_properties() {
        assert_eq!(
            value("github.event.commits.*.message"),
            Value::Array(vec![Value::string("first"), Value::string("second")])
        );
        assert!(truthy("contains(github.event.commits.*.message, 'second')"));
    }

    #[test]
    fn builtin_functions() {
        assert_eq!(value("format('{0}-{1}', 'a', 2)"), Value::string("a-2"));
        assert_eq!(value("format('{{literal}}')"), Value::string("{literal}"));
        assert_eq!(
            value("join(github.event.commits.*.message, ', ')"),
            Value::string("first, second")
        );
        assert_eq!(value("fromJSON('[1,2]')[1]"), Value::Number(2.0));
        assert_eq!(value("fromJSON('{\"a\":true}').a"), Value::Bool(true));
        assert_eq!(
            value("toJSON(matrix.os)"),
            Value::string("\"ubuntu-latest\"")
        );
    }

    #[test]
    fn status_functions_follow_the_context() {
        let failed = context().with_status(Status::Failure);
        assert!(eval_condition("always()", &failed).unwrap());
        assert!(eval_condition("failure()", &failed).unwrap());
        assert!(!eval_condition("success()", &failed).unwrap());
        assert!(eval_condition("success()", &context()).unwrap());
    }

    #[test]
    fn errors_are_reported() {
        assert!(matches!(
            Expr::parse("github. =="),
            Err(ParseError::UnexpectedToken(..))
        ));
        assert!(matches!(
            Expr::parse("'unclosed"),
            Err(ParseError::UnterminatedString(_))
        ));
        assert!(matches!(
            Expr::parse("1 2"),
            Err(ParseError::TrailingInput(_))
        ));
        assert!(matches!(
            eval_condition("nope.field", &context()),
            Err(Error::Eval(EvalError::UnknownContext(_)))
        ));
        assert!(matches!(
            eval_condition("hashFiles('**/Cargo.lock')", &context()),
            Err(Error::Eval(EvalError::Unsupported("hashFiles")))
        ));
    }
}
