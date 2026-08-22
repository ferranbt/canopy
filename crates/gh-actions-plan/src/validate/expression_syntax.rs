//! Every `${{ }}` in the workflow has to parse.

use gh_actions_expr::template;
use gh_actions_spec::Workflow;

use crate::Diagnostic;
use crate::validate::normal_jobs;

pub const NAME: &str = "expression-syntax";

/// Worth catching early: GitHub only reports these when the run reaches the step, so a
/// typo in the last step of the last job costs a whole run to find.
pub(crate) fn check(workflow: &Workflow) -> Vec<Diagnostic> {
    let mut findings = Vec::new();
    let mut report = |location: String, source: &str, is_condition: bool| {
        let (_, failures) = if is_condition {
            template::condition(source)
        } else {
            template::expressions(source)
        };

        for failure in failures {
            findings.push(Diagnostic::error(
                NAME,
                location.clone(),
                failure.to_string(),
            ));
        }
    };

    if let Some(name) = &workflow.run_name {
        report("run-name".to_owned(), name, false);
    }

    for (id, job) in normal_jobs(workflow) {
        if let Some(condition) = &job.r#if {
            report(format!("jobs.{id}.if"), condition, true);
        }
        if let Some(name) = &job.name {
            report(format!("jobs.{id}.name"), name, false);
        }
        for (key, value) in job.outputs.iter().flatten() {
            report(format!("jobs.{id}.outputs.{key}"), value, false);
        }

        for (position, step) in job.steps.iter().flatten().enumerate() {
            let at = |field: &str| crate::validate::step_location(id, position, field);

            if let Some(condition) = &step.r#if {
                report(at("if"), condition, true);
            }
            if let Some(name) = &step.name {
                report(at("name"), name, false);
            }
            if let Some(script) = &step.run {
                report(at("run"), script, false);
            }
            for (key, value) in step.with.iter().flatten() {
                report(at(&format!("with.{key}")), &scalar(value), false);
            }
            for (key, value) in step.env.iter().flatten() {
                report(at(&format!("env.{key}")), &scalar(value), false);
            }
        }
    }

    findings
}

fn scalar(value: &gh_actions_spec::Scalar) -> String {
    match value {
        gh_actions_spec::Scalar::String(text) => text.clone(),
        gh_actions_spec::Scalar::Bool(value) => value.to_string(),
        gh_actions_spec::Scalar::Int(value) => value.to_string(),
        gh_actions_spec::Scalar::Float(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::tests::check_source;

    #[test]
    fn a_malformed_expression_in_run_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github. == }}
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "expression-syntax");
        assert_eq!(findings[0].location, "jobs.build.steps[0].run");
    }

    #[test]
    fn a_malformed_condition_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: github.event_name ==
    steps:
      - run: echo hello
",
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.location == "jobs.build.if")
        );
    }

    #[test]
    fn an_unterminated_expression_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.sha
",
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("unterminated"))
        );
    }

    #[test]
    fn sound_expressions_pass() {
        let findings = check_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: github.event_name == 'push'
    steps:
      - run: echo ${{ github.sha }} ${{ format('{0}', github.ref) }}
",
        );

        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }
}
