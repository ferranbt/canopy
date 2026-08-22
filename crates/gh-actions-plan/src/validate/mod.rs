//! Checks that decide whether a workflow can run at all.

pub mod expression_syntax;
pub mod needs_cycle;
pub mod needs_exist;
pub mod step_shape;

use gh_actions_spec::Workflow;

use crate::Diagnostic;

pub fn check(workflow: &Workflow) -> Vec<Diagnostic> {
    // Shape first: a step that is neither a command nor an action makes anything said about
    // the expressions inside it beside the point.
    let mut findings = step_shape::check(workflow);
    findings.extend(needs_exist::check(workflow));
    findings.extend(needs_cycle::check(workflow));
    findings.extend(expression_syntax::check(workflow));

    findings
}

pub(crate) fn normal_jobs(
    workflow: &Workflow,
) -> impl Iterator<Item = (&String, &gh_actions_spec::NormalJob)> {
    workflow.jobs.iter().filter_map(|(id, job)| match job {
        gh_actions_spec::Job::Normal(normal) => Some((id, normal.as_ref())),
        gh_actions_spec::Job::Reusable(_) => None,
    })
}

pub(crate) fn step_location(job: &str, position: usize, field: &str) -> String {
    format!("jobs.{job}.steps[{position}].{field}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn check_source(yaml: &str) -> Vec<Diagnostic> {
        let workflow: Workflow = yaml_with_spans::from_str(yaml).expect("workflow parses");
        check(&workflow)
    }
}
