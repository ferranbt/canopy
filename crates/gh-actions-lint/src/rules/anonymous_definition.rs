use gh_actions_spec::Workflow;

use crate::{Contexts, Diagnostic, Rule};

/// Checks that a workflow says what it is called, since what it is called is what every run
/// of it is listed under.
pub struct AnonymousDefinition;

impl Rule for AnonymousDefinition {
    fn name(&self) -> &'static str {
        "anonymous-definition"
    }

    fn check(&self, workflow: &Workflow, _contexts: &Contexts) -> Vec<Diagnostic> {
        if workflow.name.is_some() {
            return Vec::new();
        }

        vec![Diagnostic::warning(
            self.name(),
            "name",
            "this workflow has no `name`, so runs of it are listed under its file path",
        )]
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::lint_source;

    #[test]
    fn a_workflow_without_a_name_is_reported() {
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo one
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "anonymous-definition");
    }

    #[test]
    fn one_with_a_name_is_left_alone() {
        let findings = lint_source(
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo one
",
        );

        assert!(findings.is_empty());
    }
}
