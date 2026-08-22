pub mod rules;
mod skip;

use std::collections::BTreeMap;

use gh_actions_plan::contexts::{self, JobContext};
use gh_actions_spec::Workflow;
use yaml_with_spans::Document;

pub use gh_actions_plan::{Diagnostic, Severity, has_errors};

/// What each job of the workflow can see, keyed by job id.
pub type Contexts = BTreeMap<String, JobContext>;

pub trait Rule {
    /// Name reported with every finding.
    fn name(&self) -> &'static str;

    /// Everything this rule objects to, against contexts that arrive already built.
    fn check(&self, workflow: &Workflow, contexts: &Contexts) -> Vec<Diagnostic>;
}

/// Runs every rule, in the order they are registered.
pub fn lint(workflow: &Workflow) -> Vec<Diagnostic> {
    let contexts = contexts::for_workflow(workflow);

    rules::all()
        .iter()
        .flat_map(|rule| rule.check(workflow, &contexts))
        .collect()
}

/// Runs every rule and drops what the workflow's own comments ask to pass over.
pub fn check(document: &Document, workflow: &Workflow) -> Vec<Diagnostic> {
    lint(workflow)
        .into_iter()
        .filter(|finding| !skip::ignored(document, finding))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn lint_source(yaml: &str) -> Vec<Diagnostic> {
        let workflow: Workflow = yaml_with_spans::from_str(yaml).expect("workflow parses");
        lint(&workflow)
    }

    fn check_source(yaml: &str) -> Vec<Diagnostic> {
        let document = Document::parse(yaml).expect("document parses");
        let workflow: Workflow =
            yaml_with_spans::from_node(&document.root).expect("workflow parses");
        check(&document, &workflow)
    }

    /// A workflow with one problem per job, so a directive's reach can be seen.
    const NOISY: &str = r"
on: push
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
on: push
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
