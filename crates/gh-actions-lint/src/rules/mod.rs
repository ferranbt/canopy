pub mod anonymous_definition;
pub mod context_availability;
pub mod duplicate_step_ids;
pub mod excessive_permissions;
pub mod hardcoded_container_credentials;
pub mod insecure_commands;
pub mod job_outputs;
pub mod overprovisioned_secrets;
pub mod ref_version_mismatch;
pub mod secrets_inherit;
pub mod template_injection;
pub mod undocumented_permissions;
pub mod unpinned_images;
pub mod unpinned_uses;
pub mod unsound_expressions;

use crate::Rule;

pub fn all() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(duplicate_step_ids::DuplicateStepIds),
        Box::new(context_availability::ContextAvailability),
        Box::new(job_outputs::JobOutputs),
        Box::new(anonymous_definition::AnonymousDefinition),
        Box::new(secrets_inherit::SecretsInherit),
        Box::new(insecure_commands::InsecureCommands),
        Box::new(hardcoded_container_credentials::HardcodedContainerCredentials),
        Box::new(unpinned_uses::UnpinnedUses),
        Box::new(unpinned_images::UnpinnedImages),
        Box::new(overprovisioned_secrets::OverprovisionedSecrets),
        Box::new(unsound_expressions::UnsoundContains),
        Box::new(unsound_expressions::UnsoundTernary),
        Box::new(template_injection::TemplateInjection),
        Box::new(excessive_permissions::ExcessivePermissions),
        Box::new(undocumented_permissions::UndocumentedPermissions),
        Box::new(ref_version_mismatch::RefVersionMismatch),
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

/// Every expression a workflow writes, with where it was written
pub(crate) fn templated(
    workflow: &gh_actions_spec::Workflow,
) -> Vec<(String, Vec<gh_actions_expr::Expr>)> {
    let mut written = Vec::new();
    let mut values = |location: String, of: &gh_actions_spec::Scalar| {
        if let gh_actions_spec::Scalar::String(source) = of {
            let (parsed, _) = gh_actions_expr::template::expressions(source);
            if !parsed.is_empty() {
                written.push((location, parsed));
            }
        }
    };

    for (name, value) in workflow.env.iter().flatten() {
        values(format!("env.{name}"), value);
    }

    for (id, job) in normal_jobs(workflow) {
        for (name, value) in job.env.iter().flatten() {
            values(format!("jobs.{id}.env.{name}"), value);
        }

        for (position, step) in job.steps.iter().flatten().enumerate() {
            for (name, value) in step.env.iter().flatten() {
                values(step_location(id, position, &format!("env.{name}")), value);
            }
            for (name, value) in step.with.iter().flatten() {
                values(step_location(id, position, &format!("with.{name}")), value);
            }
            for (field, source) in [("run", &step.run), ("name", &step.name)] {
                if let Some(source) = source {
                    values(
                        step_location(id, position, field),
                        &gh_actions_spec::Scalar::String(source.clone()),
                    );
                }
            }
        }
    }

    for (location, source) in conditions(workflow) {
        let (parsed, _) = gh_actions_expr::template::condition(&source);
        if !parsed.is_empty() {
            written.push((location, parsed));
        }
    }

    written
}

fn conditions(workflow: &gh_actions_spec::Workflow) -> Vec<(String, String)> {
    let mut written = Vec::new();

    for (id, job) in normal_jobs(workflow) {
        if let Some(source) = &job.r#if {
            written.push((format!("jobs.{id}.if"), source.clone()));
        }

        for (position, step) in job.steps.iter().flatten().enumerate() {
            if let Some(source) = &step.r#if {
                written.push((step_location(id, position, "if"), source.clone()));
            }
        }
    }

    written
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
