use gh_actions_spec::{Env, Scalar};

use crate::rules::normal_jobs;
use crate::{Diagnostic, Rule, RuleInput};

const ALLOWED: &str = "ACTIONS_ALLOW_UNSECURE_COMMANDS";

/// Checks that nothing turns the workflow commands back on that GitHub took away, since a
/// step that prints `::set-env::` can then set anything a later step runs with.
pub struct InsecureCommands;

impl Rule for InsecureCommands {
    fn name(&self) -> &'static str {
        "insecure-commands"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut findings = Vec::new();
        let mut report = |location: String| {
            findings.push(Diagnostic::warning(
                self.name(),
                location,
                format!(
                    "`{ALLOWED}` brings back `::set-env::` and `::add-path::`, \
                     which let anything a step prints change what later steps run with"
                ),
            ));
        };

        if turned_on(workflow.env.as_ref()) {
            report(format!("env.{ALLOWED}"));
        }

        for (id, job) in normal_jobs(workflow) {
            if turned_on(job.env.as_ref()) {
                report(format!("jobs.{id}.env.{ALLOWED}"));
            }

            for (position, step) in job.steps.iter().flatten().enumerate() {
                if turned_on(step.env.as_ref()) {
                    report(crate::rules::step_location(
                        id,
                        position,
                        &format!("env.{ALLOWED}"),
                    ));
                }
            }
        }

        findings
    }
}

fn turned_on(env: Option<&Env>) -> bool {
    let Some(value) = env.and_then(|env| env.get(ALLOWED)) else {
        return false;
    };

    match value {
        Scalar::Bool(said) => *said,
        Scalar::String(said) => said != "false",
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::InsecureCommands;
    use crate::tests::findings_of;

    #[test]
    fn turning_the_commands_back_on_is_reported_wherever_it_is_done() {
        let findings = findings_of(
            &InsecureCommands,
            r"
name: Build
on: push
env:
  ACTIONS_ALLOW_UNSECURE_COMMANDS: true
jobs:
  build:
    runs-on: ubuntu-latest
    env:
      ACTIONS_ALLOW_UNSECURE_COMMANDS: 'true'
    steps:
      - run: echo one
        env:
          ACTIONS_ALLOW_UNSECURE_COMMANDS: true
",
        );

        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].location, "env.ACTIONS_ALLOW_UNSECURE_COMMANDS");
        assert_eq!(
            findings[2].location,
            "jobs.build.steps[0].env.ACTIONS_ALLOW_UNSECURE_COMMANDS"
        );
    }

    #[test]
    fn leaving_them_off_is_fine() {
        let findings = findings_of(
            &InsecureCommands,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    env:
      ACTIONS_ALLOW_UNSECURE_COMMANDS: false
    steps:
      - run: echo one
",
        );

        assert!(findings.is_empty());
    }
}
