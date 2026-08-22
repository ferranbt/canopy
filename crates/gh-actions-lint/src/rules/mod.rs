pub mod context_availability;
pub mod duplicate_step_ids;
pub mod job_outputs;

use crate::Rule;

pub fn all() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(duplicate_step_ids::DuplicateStepIds),
        Box::new(context_availability::ContextAvailability),
        Box::new(job_outputs::JobOutputs),
    ]
}

/// The jobs of a workflow that run steps, since the rules mostly ignore the others.
pub(crate) fn normal_jobs(
    workflow: &gh_actions_spec::Workflow,
) -> impl Iterator<Item = (&String, &gh_actions_spec::NormalJob)> {
    workflow.jobs.iter().filter_map(|(id, job)| match job {
        gh_actions_spec::Job::Normal(normal) => Some((id, normal.as_ref())),
        gh_actions_spec::Job::Reusable(_) => None,
    })
}

pub(crate) fn step_location(job: &str, position: usize, field: &str) -> String {
    format!("jobs.{job}.steps[{position}].{field}")
}

/// Reads as the tail of a sentence, and is empty when nothing is close enough to suggest.
pub(crate) fn suggest(wanted: &str, known: &[&str]) -> String {
    let close = known
        .iter()
        .find(|name| name.starts_with(wanted) || wanted.starts_with(**name));

    match close {
        Some(name) => format!("; did you mean {name:?}?"),
        None => String::new(),
    }
}
