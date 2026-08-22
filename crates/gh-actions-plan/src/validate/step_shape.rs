//! A step runs a command or uses an action, never both and never neither.

use gh_actions_spec::Workflow;

use crate::Diagnostic;
use crate::validate::normal_jobs;

pub const NAME: &str = "step-shape";

pub(crate) fn check(workflow: &Workflow) -> Vec<Diagnostic> {
    let mut findings = Vec::new();

    for (id, job) in normal_jobs(workflow) {
        for (position, step) in job.steps.iter().flatten().enumerate() {
            let location = format!("jobs.{id}.steps[{position}]");

            match (&step.run, &step.uses) {
                (Some(_), Some(uses)) => findings.push(Diagnostic::error(
                    NAME,
                    location,
                    format!("has both `run` and `uses: {uses}`; a step is one or the other"),
                )),
                (None, None) => findings.push(Diagnostic::error(
                    NAME,
                    location,
                    "has neither `run` nor `uses`, so there is nothing to do",
                )),
                _ => {}
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use crate::validate::tests::check_source;

    #[test]
    fn a_step_with_both_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
        uses: actions/checkout@v4
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "step-shape");
        assert!(findings[0].message.contains("both"));
    }

    #[test]
    fn a_step_with_neither_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - name: does nothing
",
        );

        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("neither"));
    }

    #[test]
    fn ordinary_steps_pass() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
      - uses: actions/checkout@v4
",
        );

        assert!(findings.is_empty());
    }
}
