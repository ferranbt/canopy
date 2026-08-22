//! The embedded form of the language: `${{ }}` inside an ordinary string.

use crate::Error;
use crate::ast::Expr;
use crate::error::ParseError;
use crate::eval::Context;
use crate::value::Value;

pub fn interpolate(source: &str, context: &Context) -> Result<String, Error> {
    let mut out = String::new();
    let mut rest = source;

    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        let inner_start = start + 3;
        let end = find_close(&rest[inner_start..]).ok_or(ParseError::UnterminatedTemplate(
            source.len() - rest.len() + start,
        ))?;

        let expr = Expr::parse(&rest[inner_start..inner_start + end])?;
        out.push_str(&expr.eval(context)?.to_display_string());
        rest = &rest[inner_start + end + 2..];
    }

    out.push_str(rest);
    Ok(out)
}

/// Evaluates a string that is a single `${{ }}`, keeping the value's type; otherwise interpolates.
pub fn interpolate_value(source: &str, context: &Context) -> Result<Value, Error> {
    let trimmed = source.trim();

    if let Some(inner) = trimmed.strip_prefix("${{")
        && let Some(end) = find_close(inner)
        && end == inner.len() - 2
    {
        return Ok(Expr::parse(&inner[..end])?.eval(context)?);
    }

    Ok(Value::String(interpolate(source, context)?))
}

/// Every `${{ }}` in a string, parsed; a failure in one does not hide the others.
pub fn expressions(source: &str) -> (Vec<Expr>, Vec<ParseError>) {
    let mut parsed = Vec::new();
    let mut failures = Vec::new();
    let mut rest = source;

    while let Some(start) = rest.find("${{") {
        let inner = &rest[start + 3..];
        let Some(end) = find_close(inner) else {
            // Against the whole string, not against what is left of it.
            failures.push(ParseError::UnterminatedTemplate(
                source.len() - rest.len() + start,
            ));
            break;
        };

        match Expr::parse(&inner[..end]) {
            Ok(expr) => parsed.push(expr),
            Err(err) => failures.push(err),
        }
        rest = &inner[end + 2..];
    }

    (parsed, failures)
}

/// The `${{ }}` is optional around a condition.
pub fn condition(source: &str) -> (Vec<Expr>, Vec<ParseError>) {
    if source.contains("${{") {
        return expressions(source);
    }

    match Expr::parse_condition(source) {
        Ok(expr) => (vec![expr], Vec::new()),
        Err(err) => (Vec::new(), vec![err]),
    }
}

/// Finds the `}}` that closes an expression, ignoring one inside a string literal.
///
/// In bytes, so it can index the string it searched.
fn find_close(source: &str) -> Option<usize> {
    let mut in_string = false;
    let mut chars = source.char_indices().peekable();

    while let Some((at, character)) = chars.next() {
        match character {
            '\'' => in_string = !in_string,
            '}' if !in_string && chars.peek().map(|(_, next)| *next) == Some('}') => {
                return Some(at);
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn context() -> Context {
        Context::new().with(
            "matrix",
            Value::Object(BTreeMap::from([
                ("os".into(), Value::String("ubuntu-latest".into())),
                ("jobs".into(), Value::Number(4.0)),
            ])),
        )
    }

    #[test]
    fn a_multi_byte_character_does_not_shift_the_closing_braces() {
        let rendered = interpolate("x ${{ 'ü' }} y", &context());
        assert_eq!(rendered.unwrap(), "x ü y");
    }

    #[test]
    fn finds_every_expression_in_a_string() {
        let (parsed, failures) = expressions("a ${{ github.sha }} b ${{ matrix.os }}");

        assert_eq!(parsed.len(), 2);
        assert!(failures.is_empty());
    }

    #[test]
    fn one_bad_expression_does_not_hide_the_others() {
        let (parsed, failures) = expressions("${{ github.sha }} and ${{ == }}");

        assert_eq!(parsed.len(), 1);
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn an_unterminated_expression_is_reported_where_it_starts() {
        let source = "${{ github.sha }} and ${{ oops";
        let (_, failures) = expressions(source);

        // The offset is into the whole string: the expression before it does not move it.
        assert_eq!(
            failures.first(),
            Some(&ParseError::UnterminatedTemplate(22))
        );
        assert!(source[22..].starts_with("${{"));
    }

    #[test]
    fn a_condition_needs_no_wrapper() {
        let (parsed, failures) = condition("github.event_name == 'push'");

        assert_eq!(parsed.len(), 1);
        assert!(failures.is_empty());
    }

    #[test]
    fn substitutes_expressions() {
        let rendered = interpolate(
            "cargo test -j ${{ matrix.jobs }} on ${{ matrix.os }}",
            &context(),
        );
        assert_eq!(rendered.unwrap(), "cargo test -j 4 on ubuntu-latest");
    }

    #[test]
    fn leaves_plain_text_alone() {
        assert_eq!(interpolate("cargo test", &context()).unwrap(), "cargo test");
    }

    #[test]
    fn ignores_braces_inside_string_literals() {
        let rendered = interpolate("${{ format('{0}}}', matrix.os) }}", &context());
        assert_eq!(rendered.unwrap(), "ubuntu-latest}");
    }

    #[test]
    fn a_lone_expression_keeps_its_type() {
        let value = interpolate_value("${{ matrix.jobs }}", &context()).unwrap();
        assert_eq!(value, Value::Number(4.0));

        let mixed = interpolate_value("n=${{ matrix.jobs }}", &context()).unwrap();
        assert_eq!(mixed, Value::String("n=4".into()));
    }

    #[test]
    fn reports_an_unterminated_expression() {
        assert!(matches!(
            interpolate("${{ matrix.os", &context()),
            Err(Error::Parse(ParseError::UnterminatedTemplate(0)))
        ));
    }
}
