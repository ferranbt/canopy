//! The context each job of a workflow would see, worked out from the workflow alone.

use std::collections::{BTreeMap, BTreeSet};

use gh_actions_context::{Conclusion, JobResult, RunContext, step_result};
use gh_actions_expr::Value;
use gh_actions_spec::{Job, Matrix, NormalJob, Workflow};

#[derive(Debug, Clone, Default)]
pub struct JobContext {
    pub context: RunContext,
    /// Contexts where a key that is not there is a mistake, rather than one that arrives
    /// later. Never `env`, `vars`, `secrets` or `inputs`, which are filled from outside.
    pub closed: BTreeSet<String>,
}

const ALWAYS_CLOSED: &[&str] = &["github", "runner", "job", "strategy", "needs", "steps"];

pub fn for_workflow(workflow: &Workflow) -> BTreeMap<String, JobContext> {
    workflow
        .jobs
        .iter()
        .map(|(id, job)| {
            let built = match job {
                Job::Normal(normal) => for_job(workflow, normal),
                // Runs no steps of its own, but still declares what it needs.
                Job::Reusable(reusable) => {
                    let mut built = JobContext::default();
                    built.context.needs = needs_of(reusable.needs.as_ref());
                    built.closed = ALWAYS_CLOSED
                        .iter()
                        .map(|name| (*name).to_owned())
                        .collect();
                    built
                }
            };
            (id.clone(), built)
        })
        .collect()
}

fn for_job(workflow: &Workflow, job: &NormalJob) -> JobContext {
    let mut context = RunContext::default();
    let mut closed: BTreeSet<String> = ALWAYS_CLOSED
        .iter()
        .map(|name| (*name).to_owned())
        .collect();

    context.github.workflow = workflow.name.clone().unwrap_or_default();
    context.needs = needs_of(job.needs.as_ref());

    // A step is addressable once it has an id, and nothing at run time gives it one.
    context.steps = job
        .steps
        .iter()
        .flatten()
        .filter_map(|step| step.id.as_ref())
        .map(|id| {
            (
                id.clone(),
                step_result(Conclusion::Success, &BTreeMap::new()),
            )
        })
        .collect();

    match job
        .strategy
        .as_ref()
        .and_then(|strategy| strategy.matrix.as_ref())
    {
        Some(Matrix::Literal(literal)) => {
            // `include:` may introduce variables of its own, on top of the axes.
            let mut keys: BTreeSet<&String> = literal.axes.keys().collect();
            for entry in literal.include.iter().flatten() {
                keys.extend(entry.keys());
            }
            context.matrix = Some(
                keys.into_iter()
                    .map(|key| (key.clone(), Value::Null))
                    .collect(),
            );
            closed.insert("matrix".to_owned());
        }
        Some(Matrix::Expression(_)) => context.matrix = Some(BTreeMap::new()),
        None => {
            closed.insert("matrix".to_owned());
        }
    }

    JobContext { context, closed }
}

fn needs_of(needs: Option<&gh_actions_spec::OneOrMany<String>>) -> BTreeMap<String, JobResult> {
    needs
        .map(gh_actions_spec::OneOrMany::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|id| (id.clone(), JobResult::default()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_sees_what_it_needs_the_steps_it_has_and_its_matrix() {
        let workflow: Workflow = yaml_with_spans::from_str(
            r"
on: push
jobs:
  setup:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
  build:
    runs-on: ubuntu-latest
    needs: setup
    strategy:
      matrix:
        os: [ubuntu-latest]
    steps:
      - id: meta
        run: echo hi
      - run: echo no id
",
        )
        .expect("workflow parses");

        let built = for_workflow(&workflow);
        let build = &built["build"];

        assert_eq!(build.context.needs.keys().collect::<Vec<_>>(), ["setup"]);
        assert_eq!(build.context.steps.keys().collect::<Vec<_>>(), ["meta"]);
        assert_eq!(
            build
                .context
                .matrix
                .as_ref()
                .expect("a matrix")
                .keys()
                .collect::<Vec<_>>(),
            ["os"]
        );
        assert!(build.closed.contains("matrix"));
        assert!(!build.closed.contains("env"));
    }
}
