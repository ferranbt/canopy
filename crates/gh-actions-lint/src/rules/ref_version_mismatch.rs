use gh_actions_spec::Uses;

use crate::rules::normal_jobs;
use crate::{Diagnostic, Rule, RuleInput};

/// Checks that an action pinned to a commit says which version that commit is, since the
/// commit alone tells a reader nothing about what they are about to update.
pub struct RefVersionMismatch;

impl Rule for RefVersionMismatch {
    fn name(&self) -> &'static str {
        "ref-version-mismatch"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let mut findings = Vec::new();

        for (id, job) in normal_jobs(input.workflow) {
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
                if !commit(reference) {
                    continue;
                }

                let location = crate::rules::step_location(id, position, "uses");
                if version(input, &location).is_some() {
                    continue;
                }

                findings.push(Diagnostic::warning(
                    self.name(),
                    location,
                    format!(
                        "`{owner}/{repo}` is pinned to a commit that nothing here names; \
                         a `# v1.2.3` beside it is what says which version this is"
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

/// The version a comment beside the pin claims it is.
fn version(input: &RuleInput, location: &str) -> Option<String> {
    let node = input.document.locate(location)?;
    let comment = input.document.trailing_comment(node.span.start.line)?;

    comment
        .text
        .split_whitespace()
        .find(|word| {
            let said = word.trim_start_matches('v');
            !said.is_empty() && said.starts_with(|letter: char| letter.is_ascii_digit())
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::RefVersionMismatch;
    use crate::tests::findings_of;

    fn ours(yaml: &str) -> Vec<String> {
        findings_of(&RefVersionMismatch, yaml)
            .into_iter()
            .map(|finding| finding.location)
            .collect()
    }

    #[test]
    fn a_commit_that_names_no_version_is_reported() {
        let found = ours(
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683
",
        );

        assert_eq!(found, ["jobs.build.steps[0].uses"]);
    }

    #[test]
    fn one_that_names_it_is_fine() {
        let found = ours(
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
",
        );

        assert!(found.is_empty());
    }

    #[test]
    fn a_tag_is_left_to_the_rule_about_tags() {
        let found = ours(
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
",
        );

        assert!(found.is_empty());
    }
}
