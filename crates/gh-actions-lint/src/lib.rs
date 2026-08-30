pub mod rules;
mod skip;

use std::collections::BTreeMap;

use gh_actions_plan::contexts::{self, JobContext};
use gh_actions_spec::Workflow;
use yaml_with_spans::Document;

pub use gh_actions_plan::{Diagnostic, Severity, has_errors};

/// What each job of the workflow can see, keyed by job id.
pub type Contexts = BTreeMap<String, JobContext>;

pub struct RuleInput<'a> {
    pub workflow: &'a Workflow,
    pub document: &'a Document,
    pub contexts: Contexts,
}

impl<'a> RuleInput<'a> {
    pub fn new(document: &'a Document, workflow: &'a Workflow) -> Self {
        Self {
            contexts: contexts::for_workflow(workflow),
            workflow,
            document,
        }
    }
}

pub trait Rule {
    /// Name reported with every finding.
    fn name(&self) -> &'static str;

    /// Everything this rule objects to.
    fn check(&self, input: &RuleInput) -> Vec<Diagnostic>;
}

/// Runs every rule, in the order they are registered.
pub fn lint(input: &RuleInput) -> Vec<Diagnostic> {
    rules::all()
        .iter()
        .flat_map(|rule| rule.check(input))
        .collect()
}

/// Runs every rule and drops what the workflow's own comments ask to pass over.
pub fn check(document: &Document, workflow: &Workflow) -> Vec<Diagnostic> {
    lint(&RuleInput::new(document, workflow))
        .into_iter()
        .filter(|finding| !skip::ignored(document, finding))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn lint_source(yaml: &str) -> Vec<Diagnostic> {
        let document = Document::parse(yaml).expect("document parses");
        let workflow: Workflow =
            yaml_with_spans::from_node(&document.root).expect("workflow parses");

        lint(&RuleInput::new(&document, &workflow))
    }

    pub(crate) fn findings_of(rule: &dyn Rule, yaml: &str) -> Vec<Diagnostic> {
        let document = Document::parse(yaml).expect("document parses");
        let workflow: Workflow =
            yaml_with_spans::from_node(&document.root).expect("workflow parses");

        rule.check(&RuleInput::new(&document, &workflow))
    }

    fn check_source(yaml: &str) -> Vec<Diagnostic> {
        let document = Document::parse(yaml).expect("document parses");
        let workflow: Workflow =
            yaml_with_spans::from_node(&document.root).expect("workflow parses");
        check(&document, &workflow)
    }

    /// A workflow with one problem per job, so a directive's reach can be seen. It is
    /// otherwise sound, so what it says about a directive is not lost among other findings.
    const NOISY: &str = r"
name: Test
on: push
permissions: {}
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.nothing.outputs.version }}
    steps:
      - run: echo hi
  ship:
    runs-on: ubuntu-latest
    steps:
      - id: twice
        run: echo one
      - id: twice
        run: echo two
";

    #[test]
    fn without_a_directive_everything_is_reported() {
        let findings = check_source(NOISY);

        assert_eq!(findings.len(), 2, "{findings:?}");
    }

    #[test]
    fn a_trailing_directive_silences_its_own_line() {
        let findings = check_source(&NOISY.replace(
            "      - id: twice\n        run: echo two",
            "      - id: twice # canopy:ignore\n        run: echo two",
        ));

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "job-outputs");
    }

    #[test]
    fn a_directive_on_the_line_above_reaches_the_line_below() {
        let findings = check_source(&NOISY.replace(
            "      version: ${{ steps.nothing.outputs.version }}",
            "      # canopy:ignore\n      version: ${{ steps.nothing.outputs.version }}",
        ));

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "duplicate-step-ids");
    }

    #[test]
    fn a_directive_naming_another_rule_silences_nothing_here() {
        let findings = check_source(&NOISY.replace(
            "      version: ${{ steps.nothing.outputs.version }}",
            "      version: ${{ steps.nothing.outputs.version }} # canopy:ignore step-shape",
        ));

        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn a_sound_workflow_has_nothing_to_say_about_it() {
        let findings = lint_source(
            r"
name: Test
on: push
permissions: {}
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.meta.outputs.version }}
    steps:
      - id: meta
        run: echo version=1 >> $GITHUB_OUTPUT
  ship:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ needs.build.outputs.version }}
",
        );

        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }

    #[test]
    fn findings_read_as_one_line() {
        let finding = Diagnostic::warning("some-rule", "jobs.build", "something looks wrong");

        assert_eq!(
            finding.to_string(),
            "warning: jobs.build [some-rule] something looks wrong"
        );
    }
}
