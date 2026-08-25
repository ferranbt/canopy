use std::collections::BTreeMap;

use gh_actions_context::{Conclusion, JobResult, RunContext};
use gh_actions_expr::Value;

use gh_actions_spec::{Scalar, Step, Uses, With};

use crate::client::types::{JobContext, JobMessage, PipelineStep, StepReference};

impl JobContext {
    pub fn to_run_context(&self) -> RunContext {
        let mut run = RunContext::default();

        run.github.actor = self.github.actor.clone();
        run.github.event = self.github.event();
        run.github.event_name = self.github.event_name.clone();
        run.github.r#ref = self.github.r#ref.clone();
        run.github.repository = self.github.repository.clone();
        run.github.repository_owner = self.github.owner();
        run.github.run_id = self.github.run_id;
        run.github.run_number = self.github.run_number;
        run.github.sha = self.github.sha.clone();
        run.github.workflow = self.github.workflow.clone();

        run.inputs = self.inputs.clone();
        run.vars = self.vars.clone();
        run.matrix = (!self.matrix.is_empty()).then(|| {
            self.matrix
                .iter()
                .map(|(key, value)| (key.clone(), Value::from(value.clone())))
                .collect()
        });
        run.needs = self
            .needs
            .iter()
            .map(|(id, result)| {
                let result = JobResult {
                    conclusion: Conclusion::from_name(&result.result),
                    outputs: result.outputs.clone(),
                };
                (id.clone(), result)
            })
            .collect();

        run
    }
}

impl JobMessage {
    pub fn to_run_context(&self) -> RunContext {
        let mut run = self.context_data.to_run_context();
        run.github.job = self.job_name.clone();
        run.github.token = self.runtime_token().unwrap_or_default().to_owned();
        // Nothing says where the workspace is: that is the runner's own business.
        run
    }
}

impl PipelineStep {
    /// The service compiles a step down before sending it: a `run:` becomes a script whose
    /// command is an input, and a `uses:` becomes a repository reference with the `with:`
    /// block alongside. This puts it back together.
    pub fn to_step(&self) -> Result<Step, gh_actions_spec::UsesError> {
        let mut step = Step {
            id: self.context_name.clone(),
            name: Some(self.display_name.clone()).filter(|name| !name.is_empty()),
            // Every step arrives with one, and `success()` is what having none compiles to.
            r#if: self.condition.clone(),
            env: (!self.env.is_empty()).then(|| scalars(&self.env).into_iter().collect()),
            ..Step::default()
        };

        match &self.reference {
            StepReference::Script => {
                step.run = self.inputs.get("script").cloned();
                step.shell = self.inputs.get("shell").cloned();
                step.working_directory = self.inputs.get("workingDirectory").cloned();
            }
            StepReference::Repository {
                name, r#ref, path, ..
            } => {
                let uses = match path.as_deref().filter(|path| !path.is_empty()) {
                    Some(path) => format!("{name}/{path}@{}", r#ref),
                    None => format!("{name}@{}", r#ref),
                };
                step.uses = Some(uses.parse::<Uses>()?);
                step.with = with(&self.inputs);
            }
            StepReference::ContainerRegistry { image } => {
                step.uses = Some(Uses::Image(image.clone()));
                step.with = with(&self.inputs);
            }
        }

        Ok(step)
    }
}

impl JobMessage {
    pub fn to_steps(&self) -> Result<Vec<Step>, gh_actions_spec::UsesError> {
        self.steps.iter().map(PipelineStep::to_step).collect()
    }
}

fn scalars(values: &BTreeMap<String, String>) -> With {
    values
        .iter()
        .map(|(name, value)| (name.clone(), Scalar::String(value.clone())))
        .collect()
}

/// The inputs an action was given, minus the ones that are not inputs at all.
fn with(inputs: &BTreeMap<String, String>) -> Option<With> {
    let given: BTreeMap<String, String> = inputs
        .iter()
        .filter(|(name, _)| name.as_str() != "script")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();

    (!given.is_empty()).then(|| scalars(&given))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::types::StepReference;

    fn job() -> JobMessage {
        JobMessage::decode(include_str!("../fixtures/acquired-job.json")).expect("the job decodes")
    }

    #[test]
    fn every_field_of_a_real_job_is_understood() {
        let job = job();

        assert_eq!(job.job_display_name, "test");
        assert_eq!(job.steps.len(), 6);
        assert_eq!(job.runtime_token(), Some("redacted"));
    }

    #[test]
    fn a_job_can_be_written_out_and_read_back() {
        let written = serde_json::to_string(&job()).expect("the job writes");
        let read = serde_json::from_str::<JobMessage>(&written).expect("and reads back");

        assert_eq!(read.steps.len(), 6);
        assert_eq!(read.steps[1].display_name, "Set up Go");
        assert_eq!(read.context_data.github.run_id, 32_465_023_908);
        assert_eq!(read.to_run_context().github.event_name, "workflow_dispatch");
    }

    #[test]
    fn the_contexts_arrive_encoded_and_come_out_as_values() {
        let context = job().to_run_context();

        assert_eq!(context.github.repository, "ferranbt/test-ci");
        assert_eq!(context.github.repository_owner, "ferranbt");
        assert_eq!(context.github.r#ref, "refs/heads/main");
        assert_eq!(context.github.run_number, 9);
        assert!(context.matrix.is_none(), "this job has no matrix");
    }

    #[test]
    fn the_event_that_started_the_run_survives() {
        let context = job().to_run_context();
        let event = serde_json::to_value(&context.github.event).expect("the event writes");

        assert_eq!(context.github.event_name, "workflow_dispatch");
        assert_eq!(event["repository"]["full_name"], "ferranbt/test-ci");
    }

    #[test]
    fn a_script_step_carries_its_script_and_an_action_carries_its_inputs() {
        let job = job();

        let StepReference::Repository { name, r#ref, .. } = &job.steps[1].reference else {
            panic!("expected an action, got {:?}", job.steps[1].reference);
        };
        assert_eq!((name.as_str(), r#ref.as_str()), ("actions/setup-go", "v5"));
        assert_eq!(job.steps[1].inputs["go-version"], "1.25");

        assert!(matches!(job.steps[3].reference, StepReference::Script));
        assert_eq!(job.steps[3].inputs["script"], "go vet ./...");
    }

    #[test]
    fn a_compiled_step_goes_back_to_what_a_workflow_would_have_written() {
        let steps = job()
            .to_steps()
            .expect("every step is a reference this understands");

        assert_eq!(steps[1].name.as_deref(), Some("Set up Go"));
        assert_eq!(
            steps[1].uses.as_ref().map(ToString::to_string).as_deref(),
            Some("actions/setup-go@v5")
        );
        assert_eq!(
            steps[1].with.as_ref().expect("inputs")["go-version"],
            Scalar::String("1.25".to_owned())
        );

        assert_eq!(steps[3].run.as_deref(), Some("go vet ./..."));
        assert!(steps[3].uses.is_none());
    }
}
