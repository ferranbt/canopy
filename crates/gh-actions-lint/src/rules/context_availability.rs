use std::collections::{BTreeMap, BTreeSet};

use gh_actions_expr::{Expr, Reference, Value, template};
use gh_actions_plan::contexts::JobContext;
use gh_actions_spec::Workflow;

use crate::rules::normal_jobs;
use crate::{Contexts, Diagnostic, Rule};

/// Checks that expressions only reach for contexts that are there.
///
/// Only the cases that can be decided from the workflow alone are reported; anything that
/// depends on how the run was started is left alone.
pub struct ContextAvailability;

impl Rule for ContextAvailability {
    fn name(&self) -> &'static str {
        "context-availability"
    }

    fn check(&self, workflow: &Workflow, contexts: &Contexts) -> Vec<Diagnostic> {
        let mut findings = Vec::new();

        for (id, job) in normal_jobs(workflow) {
            let Some(built) = contexts.get(id) else {
                continue;
            };
            let seen = Seen::of(built);
            // A job-level condition is evaluated before the job exists, so it sees less.
            if let Some(condition) = &job.r#if {
                let (parsed, _) = template::condition(condition);
                for reference in parsed.iter().flat_map(Expr::references) {
                    if let Some(problem) = job_level(&reference, &seen) {
                        findings.push(Diagnostic::warning(
                            self.name(),
                            format!("jobs.{id}.if"),
                            problem,
                        ));
                    }
                }
            }

            for (position, step) in job.steps.iter().flatten().enumerate() {
                let sources = [
                    step.r#if.as_ref().map(|value| ("if", value.clone())),
                    step.run.as_ref().map(|value| ("run", value.clone())),
                ];

                for (field, source) in sources.into_iter().flatten() {
                    let (parsed, _) = if field == "if" {
                        template::condition(&source)
                    } else {
                        template::expressions(&source)
                    };

                    for reference in parsed.iter().flat_map(Expr::references) {
                        if let Some(problem) = seen.problem(&reference) {
                            findings.push(Diagnostic::warning(
                                self.name(),
                                crate::rules::step_location(id, position, field),
                                problem,
                            ));
                        }
                    }
                }
            }
        }

        findings
    }
}

/// What a job can see, ready to answer whether a reference resolves in it.
struct Seen {
    contexts: BTreeMap<String, Value>,
    closed: BTreeSet<String>,
}

impl Seen {
    fn of(built: &JobContext) -> Self {
        Self {
            contexts: built.context.to_expr_context().contexts,
            closed: built.closed.clone(),
        }
    }

    /// What is wrong with a reference, if anything.
    fn problem(&self, reference: &Reference) -> Option<String> {
        let name = reference.context.as_str();
        let Some(context) = self.contexts.get(name) else {
            return Some(format!("`{name}` is not a context that exists"));
        };

        // Anything else is filled in as the run goes: a missing key means nothing.
        if !self.closed.contains(name) {
            return None;
        }
        if *context == Value::Null {
            return Some(format!(
                "`{name}` is null here, so `{name}.*` reads nothing"
            ));
        }

        let field = reference.first()?;
        let Value::Object(fields) = context else {
            return None;
        };
        // Property lookup ignores case, and so does this.
        if fields.keys().any(|known| known.eq_ignore_ascii_case(field)) {
            return None;
        }

        let known: Vec<&str> = fields.keys().map(String::as_str).collect();
        Some(format!(
            "reads `{name}.{field}`, but `{name}` here has no `{field}`{}",
            crate::rules::suggest(field, &known)
        ))
    }
}

/// A job-level `if` is decided before the job exists, so some contexts are there but hold
/// nothing yet — a question about where the expression is written, not about what exists.
fn job_level(reference: &Reference, seen: &Seen) -> Option<String> {
    match reference.context.as_str() {
        "steps" => Some("`steps` is not available in a job-level `if`: no step has run yet".into()),
        "env" => Some("`env` is not available in a job-level `if`".into()),
        "job" => Some("`job` is not available in a job-level `if`".into()),
        _ => seen.problem(reference),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::lint_source;

    #[test]
    fn steps_are_not_available_in_a_job_condition() {
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: steps.meta.outputs.version == '1'
    steps:
      - run: echo hello
",
        );

        assert!(findings.iter().any(|finding| {
            finding.rule == "context-availability" && finding.message.contains("`steps`")
        }));
    }

    #[test]
    fn a_need_read_through_brackets_is_checked_too() {
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ needs['setup'].outputs.version }}
",
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("`needs` here has no `setup`")),
            "unexpected: {findings:?}"
        );
    }

    #[test]
    fn a_misspelled_context_field_is_caught() {
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.repositroy }} ${{ runner.oss }}
",
        );

        let messages: Vec<&str> = findings.iter().map(|f| f.message.as_str()).collect();
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`github` here has no `repositroy`")),
            "unexpected: {messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("did you mean \"os\"")),
            "unexpected: {messages:?}"
        );
    }

    #[test]
    fn every_field_the_context_carries_passes() {
        // `token` is never filled in locally and `action_path` only inside a composite
        // action, but both are fields of the context and workflows read them constantly.
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.token }} ${{ github.action_path }} ${{ job.status }}
",
        );

        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }

    #[test]
    fn a_context_the_workflow_fills_in_is_not_checked_field_by_field() {
        // Nothing here knows what a step wrote to `$GITHUB_ENV`, so an unknown key is not
        // evidence of anything.
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ env.SOMETHING_A_STEP_SET }} ${{ vars.ANY_VARIABLE }}
",
        );

        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }

    #[test]
    fn matrix_without_a_strategy_is_refused() {
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ matrix.os }}
",
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("`matrix` is null here"))
        );
    }

    #[test]
    fn matrix_with_a_strategy_passes() {
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        os: [ubuntu-latest]
    steps:
      - run: echo ${{ matrix.os }}
",
        );

        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }

    #[test]
    fn reading_a_job_this_one_does_not_need_is_refused() {
        let findings = lint_source(
            r"
on: push
jobs:
  setup:
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ needs.setup.outputs.version }}
",
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("`needs` here has no"))
        );
    }

    #[test]
    fn a_context_that_does_not_exist_is_refused() {
        let findings = lint_source(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ enviroment.name }}
",
        );

        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("is not a context"))
        );
    }
}
