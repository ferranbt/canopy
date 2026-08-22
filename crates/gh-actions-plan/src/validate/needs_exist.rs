//! A job can only need a job that exists.

use gh_actions_spec::{Job, Workflow};

use crate::Diagnostic;

pub const NAME: &str = "needs-exist";

pub(crate) fn check(workflow: &Workflow) -> Vec<Diagnostic> {
    let mut findings = Vec::new();

    for (id, job) in &workflow.jobs {
        for need in needs_of(job) {
            if workflow.jobs.contains_key(need) {
                continue;
            }

            findings.push(Diagnostic::error(
                NAME,
                format!("jobs.{id}.needs"),
                format!("needs {need:?}, which is not a job in this workflow"),
            ));
        }

        if needs_of(job).iter().any(|need| need == id) {
            findings.push(Diagnostic::error(
                NAME,
                format!("jobs.{id}.needs"),
                "needs itself, so it can never start".to_owned(),
            ));
        }
    }

    findings
}

pub(crate) fn needs_of(job: &Job) -> &[String] {
    let needs = match job {
        Job::Normal(normal) => normal.needs.as_ref(),
        Job::Reusable(reusable) => reusable.needs.as_ref(),
    };

    needs
        .map(gh_actions_spec::OneOrMany::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use crate::validate::tests::check_source;

    #[test]
    fn a_missing_job_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    needs: [setup]
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "needs-exist");
        assert!(findings[0].message.contains("setup"));
    }

    #[test]
    fn a_job_needing_itself_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
",
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("itself"))
        );
    }

    #[test]
    fn a_job_that_exists_passes() {
        let findings = check_source(
            r"
on: push
jobs:
  setup:
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
  build:
    needs: setup
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
",
        );

        assert!(findings.is_empty());
    }
}
