use std::collections::BTreeMap;

use gh_actions_spec::Workflow;

use crate::rules::normal_jobs;
use crate::{Contexts, Diagnostic, Rule};

/// Checks that step ids are unique within their job.
pub struct DuplicateStepIds;

impl Rule for DuplicateStepIds {
    fn name(&self) -> &'static str {
        "duplicate-step-ids"
    }

    fn check(&self, workflow: &Workflow, _contexts: &Contexts) -> Vec<Diagnostic> {
        let mut findings = Vec::new();

        for (id, job) in normal_jobs(workflow) {
            let mut seen: BTreeMap<&str, usize> = BTreeMap::new();

            for (position, step) in job.steps.iter().flatten().enumerate() {
                let Some(step_id) = &step.id else {
                    continue;
                };

                if let Some(first) = seen.get(step_id.as_str()) {
                    findings.push(Diagnostic::warning(
                        self.name(),
                        crate::rules::step_location(id, position, "id"),
                        format!(
                            "id {step_id:?} is already used by step {first}; \
                             `steps.{step_id}` would only ever see one of them"
                        ),
                    ));
                    continue;
                }
                seen.insert(step_id, position);
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::lint_source;

    #[test]
    fn a_repeated_id_is_refused() {
        let findings = lint_source(
            r"
name: Test
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - id: meta
        run: echo one
      - id: meta
        run: echo two
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "duplicate-step-ids");
        assert_eq!(findings[0].location, "jobs.build.steps[1].id");
    }

    #[test]
    fn the_same_id_in_another_job_is_fine() {
        let findings = lint_source(
            r"
name: Test
on: push
jobs:
  one:
    runs-on: ubuntu-latest
    steps:
      - id: meta
        run: echo one
  two:
    runs-on: ubuntu-latest
    steps:
      - id: meta
        run: echo two
",
        );

        assert!(findings.is_empty());
    }
}
