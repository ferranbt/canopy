use gh_actions_expr::{BinaryOp, Expr};

use crate::rules::templated;
use crate::{Diagnostic, Rule, RuleInput};

/// Checks for `contains()` against a list written as one string, which matches anything the
/// string happens to hold rather than the values it was meant to list.
pub struct UnsoundContains;

impl Rule for UnsoundContains {
    fn name(&self) -> &'static str {
        "unsound-contains"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut findings = Vec::new();

        for (location, parsed) in templated(workflow) {
            for expr in parsed.iter().flat_map(walk) {
                let Expr::Call(name, arguments) = expr else {
                    continue;
                };
                if !name.eq_ignore_ascii_case("contains") || arguments.len() != 2 {
                    continue;
                }
                let (Expr::String(listed), looked) = (&arguments[0], &arguments[1]) else {
                    continue;
                };
                if !listed.contains([' ', ',']) || matches!(looked, Expr::String(_)) {
                    continue;
                }

                findings.push(Diagnostic::warning(
                    self.name(),
                    location.clone(),
                    format!(
                        "{listed:?} is one string, so this holds for anything inside it, \
                         `mai` included; use `fromJSON` with a list instead"
                    ),
                ));
            }
        }

        findings
    }
}

/// Checks for `a && b || c` where `b` is a value that is itself false, which is `c` however
/// `a` turns out.
pub struct UnsoundTernary;

impl Rule for UnsoundTernary {
    fn name(&self) -> &'static str {
        "unsound-ternary"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut findings = Vec::new();

        for (location, parsed) in templated(workflow) {
            for expr in parsed.iter().flat_map(walk) {
                let Expr::Binary(BinaryOp::Or, left, _) = expr else {
                    continue;
                };
                let Expr::Binary(BinaryOp::And, _, wanted) = left.as_ref() else {
                    continue;
                };
                if !falsy(wanted) {
                    continue;
                }

                findings.push(Diagnostic::warning(
                    self.name(),
                    location.clone(),
                    "the middle of this `&&`/`||` is false in itself, so the whole reads as \
                     the last part whatever the condition says",
                ));
            }
        }

        findings
    }
}

fn falsy(expr: &Expr) -> bool {
    match expr {
        Expr::String(said) => said.is_empty(),
        Expr::Bool(said) => !said,
        Expr::Number(said) => *said == 0.0,
        Expr::Null => true,
        _ => false,
    }
}

fn walk(expr: &Expr) -> Vec<&Expr> {
    let mut of = vec![expr];

    let inside: Vec<&Expr> = match expr {
        Expr::Property(on, _) | Expr::Star(on) | Expr::Not(on) => vec![on],
        Expr::Index(on, by) => vec![on, by],
        Expr::Binary(_, left, right) => vec![left, right],
        Expr::Call(_, arguments) => arguments.iter().collect(),
        _ => Vec::new(),
    };
    of.extend(inside.into_iter().flat_map(walk));

    of
}

#[cfg(test)]
mod tests {
    use super::{UnsoundContains, UnsoundTernary};
    use crate::tests::findings_of;

    #[test]
    fn a_list_written_as_one_string_is_reported() {
        let findings = findings_of(
            &UnsoundContains,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: contains('refs/heads/main refs/heads/develop', github.ref)
    steps:
      - run: ./deploy
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "unsound-contains");
        assert_eq!(findings[0].location, "jobs.build.if");
    }

    #[test]
    fn a_real_list_and_an_ordinary_substring_check_are_fine() {
        let findings = findings_of(
            &UnsoundContains,
            r#"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: contains(fromJSON('["refs/heads/main"]'), github.ref) && contains(github.ref, 'main')
    steps:
      - run: ./deploy
"#,
        );

        assert!(findings.is_empty());
    }

    #[test]
    fn a_ternary_whose_middle_is_false_in_itself_is_reported() {
        let findings = findings_of(
            &UnsoundTernary,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: ./deploy
        env:
          VALUE: ${{ github.ref_protected && '' || 'fallback' }}
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "unsound-ternary");
    }

    #[test]
    fn one_whose_middle_stands_up_is_fine() {
        let findings = findings_of(
            &UnsoundTernary,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: ./deploy
        env:
          VALUE: ${{ github.ref_protected && 'yes' || 'no' }}
",
        );

        assert!(findings.is_empty());
    }
}
