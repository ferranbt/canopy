use gh_actions_spec::Uses;

use crate::rules::normal_jobs;
use crate::{Diagnostic, Rule, RuleInput};

/// Checks that an action is asked for by commit, since a tag or a branch is whatever the
/// repository it came from says it is today.
pub struct UnpinnedUses;

impl Rule for UnpinnedUses {
    fn name(&self) -> &'static str {
        "unpinned-uses"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut findings = Vec::new();

        for (id, job) in normal_jobs(workflow) {
            for (position, step) in job.steps.iter().flatten().enumerate() {
                let Some(Uses::Remote {
                    owner,
                    repo,
                    reference,
                    ..
                }) = &step.uses
                else {
                    continue;
                };
                if commit(reference) {
                    continue;
                }

                findings.push(Diagnostic::warning(
                    self.name(),
                    crate::rules::step_location(id, position, "uses"),
                    format!(
                        "`{owner}/{repo}@{reference}` is whatever that name points at when the \
                         step runs; ask for a commit instead"
                    ),
                ));
            }
        }

        findings
    }
}

fn commit(reference: &str) -> bool {
    reference.len() == 40 && reference.chars().all(|letter| letter.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::UnpinnedUses;
    use crate::tests::findings_of;

    #[test]
    fn a_tag_or_a_branch_is_reported() {
        let findings = findings_of(
            &UnpinnedUses,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@main
",
        );

        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].rule, "unpinned-uses");
        assert_eq!(findings[0].location, "jobs.build.steps[0].uses");
    }

    #[test]
    fn a_commit_and_an_action_of_our_own_are_fine() {
        let findings = findings_of(
            &UnpinnedUses,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683
      - uses: ./actions/build
",
        );

        assert!(findings.is_empty());
    }
}
