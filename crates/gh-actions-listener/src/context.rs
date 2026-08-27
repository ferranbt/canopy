use std::collections::BTreeMap;

use gh_actions_context::{Conclusion, JobResult, RunContext, Strategy};
use gh_actions_expr::{Value, interpolate};

use gh_actions_spec::{Expr, Scalar, Step, Uses, With};

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
        run.strategy = Strategy {
            fail_fast: self.strategy.fail_fast,
            job_index: self.strategy.job_index,
            job_total: self.strategy.job_total,
            max_parallel: self.strategy.max_parallel,
        };
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
        // The service sends workflow secrets as secret variables. The two internal access
        // tokens have dedicated contexts and, like the official runner, are not exposed under
        // `secrets` as well.
        run.secrets = self
            .variables
            .iter()
            .filter(|(name, variable)| {
                variable.is_secret
                    && !name.eq_ignore_ascii_case("system.accesstoken")
                    && !name.eq_ignore_ascii_case("system.github.token")
            })
            .map(|(name, variable)| (name.clone(), variable.value.clone()))
            .collect();
        extend_environment(&mut run, &self.environment_variables);
        // Nothing says where the workspace is: that is the runner's own business.
        run
    }
}

/// Workflow and job env arrive as a list of mappings, in precedence order. Each mapping is
/// evaluated against the contexts established before it, then made visible to later mappings.
fn extend_environment(run: &mut RunContext, environments: &serde_json::Value) {
    let Some(environments) = environments.as_array() else {
        return;
    };

    for environment in environments {
        let Some(environment) = environment.as_object() else {
            continue;
        };
        let context = run.to_expr_context();
        let resolved = environment.iter().filter_map(|(name, value)| {
            let source = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Bool(value) => value.to_string(),
                serde_json::Value::Number(value) => value.to_string(),
                _ => {
                    tracing::warn!(%name, "cannot read a job environment variable");
                    return None;
                }
            };
            let value = interpolate(&source, &context).unwrap_or_else(|err| {
                tracing::warn!(%err, %name, "cannot evaluate a job environment variable");
                source
            });
            Some((name.clone(), value))
        });
        run.env.extend(resolved);
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
            // Already decided by the service, so what arrives is the answer rather than the
            // expression a workflow may have written.
            continue_on_error: self.continue_on_error.map(Expr::Value),
            timeout_minutes: self.timeout_in_minutes.map(Expr::Value),
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

    /// The probe workflow as a real run hands it over, with its secrets taken out: every
    /// step of it is something the service has an encoding of its own for.
    #[test]
    fn test_fixtures_probe() {
        let job = JobMessage::decode(include_str!("../fixtures/probe-job.json"))
            .expect("the job decodes");

        assert_eq!(job.job_display_name, "probe");
        assert_eq!(job.steps.len(), 11);
        assert!(job.secrets().contains(&"the-probe-secret".to_owned()));
        assert_eq!(job.secrets().len(), 3, "and both tokens");

        let steps = &job.steps;
        assert_eq!(steps[9].display_name, "A failure that is forgiven");
        assert_eq!(steps[9].continue_on_error, Some(true));
        assert_eq!(steps[0].continue_on_error, None, "it declared none");
        assert_eq!(steps[10].condition.as_deref(), Some("success() && (false)"));
        assert_eq!(
            steps[3].display_name,
            "A name with \"quotes\", a comma, and é🌲"
        );
        assert_eq!(
            steps[0].env.get("FROM_SECRET").map(String::as_str),
            Some("${{ secrets.CANOPY_TEST_SECRET }}")
        );
        assert!(steps[0].inputs["script"].starts_with("test -n \"$FROM_SECRET\""));

        // What the service decided about a step is what the runner is asked to do.
        let run = job.to_steps().expect("the steps read");
        assert_eq!(run[9].continue_on_error, Some(Expr::Value(true)));
        assert_eq!(run[0].continue_on_error, None);
        assert_eq!(run[10].r#if.as_deref(), Some("success() && (false)"));

        // Written back in the shape it came in, so what goes out is what the service sends.
        let written = serde_json::to_string(&job).expect("the job writes");
        let read = JobMessage::decode(&written).expect("and reads back");
        assert_eq!(read.steps.len(), job.steps.len());
        assert_eq!(read.steps[9].continue_on_error, Some(true));
        assert_eq!(read.steps[0].env, job.steps[0].env);
        assert_eq!(read.context_data.github.repository, "ferranbt/canopy");
        assert!(read.context_data.strategy.fail_fast);

        let context = job.to_run_context();
        assert_eq!(context.github.repository, "ferranbt/canopy");
        assert_eq!(context.github.event_name, "workflow_dispatch");
        assert_eq!(context.github.run_id, 32_960_554_847);
        assert_eq!(
            context.inputs.get("label").map(String::as_str),
            Some("canopy-probe-575695")
        );
        assert!(context.strategy.fail_fast, "it is not turned off");
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
    fn only_workflow_secrets_enter_the_secrets_context() {
        let context = job().to_run_context();

        assert_eq!(context.secrets["github_token"], "redacted");
        assert!(
            !context.secrets.contains_key("system.github.token"),
            "the runner's internal token has its own github.token context"
        );
    }

    #[test]
    fn an_expression_input_reads_a_secret_variable() {
        let job = JobMessage::decode(
            r#"{
                "variables": {"GH_TOKEN": {"value": "a-secret", "isSecret": true}},
                "steps": [{"inputs": {"type": 2, "map": [
                    {"Key": {"type": 0, "lit": "token"},
                     "Value": {"type": 3, "expr": "secrets.GH_TOKEN"}}
                ]}}]
            }"#,
        )
        .expect("the expression token decodes");

        assert_eq!(job.steps[0].inputs["token"], "${{ secrets.GH_TOKEN }}");
        assert_eq!(
            gh_actions_expr::interpolate(
                &job.steps[0].inputs["token"],
                &job.to_run_context().to_expr_context()
            )
            .expect("the secret expression evaluates"),
            "a-secret"
        );
    }

    #[test]
    fn job_environment_reads_a_secret_variable() {
        let job = JobMessage::decode(
            r#"{
                "variables": {"TOKEN": {
                    "value": "a-secret",
                    "isSecret": true
                }},
                "environmentVariables": [{"type": 2, "map": [
                    {"Key": {"type": 0, "lit": "TOKEN"},
                     "Value": {"type": 3, "expr": "secrets.TOKEN"}}
                ]}]
            }"#,
        )
        .expect("the job environment decodes");

        let context = job.to_run_context();
        assert_eq!(
            context.env.get("TOKEN").map(String::as_str),
            Some("a-secret")
        );
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
