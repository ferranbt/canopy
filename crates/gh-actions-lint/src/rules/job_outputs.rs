use gh_actions_expr::{Expr, template};
use gh_actions_spec::Workflow;

use crate::rules::normal_jobs;
use crate::{Contexts, Diagnostic, Rule};

/// Checks that job outputs read steps the job actually has.
///
/// A misspelled step id here is silent on GitHub: the output resolves to an empty string
/// and whatever needed it quietly gets nothing.
pub struct JobOutputs;

impl Rule for JobOutputs {
    fn name(&self) -> &'static str {
        "job-outputs"
    }

    fn check(&self, workflow: &Workflow, contexts: &Contexts) -> Vec<Diagnostic> {
        let mut findings = Vec::new();

        for (id, job) in normal_jobs(workflow) {
            // The same `steps` an expression here would see, not a second reading of it.
            let Some(built) = contexts.get(id) else {
                continue;
            };
            let ids: Vec<&str> = built.context.steps.keys().map(String::as_str).collect();

            for (name, value) in job.outputs.iter().flatten() {
                let (parsed, _) = template::expressions(value);

                for reference in parsed.iter().flat_map(Expr::references) {
                    if reference.context != "steps" {
                        continue;
                    }
                    let Some(wanted) = reference.first() else {
                        continue;
                    };
                    if ids.contains(&wanted) {
                        continue;
                    }

                    findings.push(Diagnostic::warning(
                        self.name(),
                        format!("jobs.{id}.outputs.{name}"),
                        format!(
                            "reads `steps.{wanted}`, but no step in this job has that id{}",
                            no_step(wanted, &ids)
                        ),
                    ));
                }
            }
        }

        findings
    }
}

/// Why no step matched: the closest id, or that there were none to match.
fn no_step(wanted: &str, ids: &[&str]) -> String {
    if ids.is_empty() {
        return "; no step in it has an id at all".to_owned();
    }
    crate::rules::suggest(wanted, ids)
}

#[cfg(test)]
mod tests {
    use crate::tests::lint_source;

    #[test]
    fn an_output_reading_an_unknown_step_is_refused() {
        let findings = lint_source(
            r"
name: Test
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.metadata.outputs.version }}
    steps:
      - id: meta
        run: echo hello
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "job-outputs");
        assert!(findings[0].message.contains("did you mean \"meta\""));
    }

    #[test]
    fn an_output_reading_a_real_step_passes() {
        let findings = lint_source(
            r"
name: Test
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.meta.outputs.version }}
    steps:
      - id: meta
        run: echo hello
",
        );

        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }

    #[test]
    fn a_job_whose_steps_have_no_ids_says_so() {
        let findings = lint_source(
            r"
name: Test
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.meta.outputs.version }}
    steps:
      - run: echo hello
",
        );

        assert!(findings[0].message.contains("no step in it has an id"));
    }
}
