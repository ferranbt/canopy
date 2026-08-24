pub mod github;
pub mod payloads;
pub mod runner;

use std::collections::BTreeMap;

use gh_actions_expr::{Context, Status, Value, to_value};
use serde::{Deserialize, Serialize};

pub use github::Github;
pub use payloads::{
    Author, Branch, Comment, Commit, Extra, Issue, IssueCommentEvent, IssuesEvent, Label, Payload,
    PullRequest, PullRequestEvent, Push, Release, ReleaseEvent, Repository, Schedule, User,
    WorkflowDispatch, WorkflowRun, WorkflowRunEvent,
};
pub use runner::Runner;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Conclusion {
    #[default]
    Success,
    Failure,
    Skipped,
}

impl Conclusion {
    pub fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Skipped => "skipped",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "failure" => Self::Failure,
            "skipped" | "cancelled" => Self::Skipped,
            _ => Self::Success,
        }
    }

    pub fn status(self) -> Status {
        match self {
            Self::Success | Self::Skipped => Status::Success,
            Self::Failure => Status::Failure,
        }
    }
}

impl<'de> Deserialize<'de> for Conclusion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Self::from_name(&name))
    }
}

impl Serialize for Conclusion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct JobResult {
    #[serde(rename = "result")]
    pub conclusion: Conclusion,
    pub outputs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Job {
    pub status: Conclusion,
    pub container: BTreeMap<String, String>,
    pub services: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Strategy {
    #[serde(rename = "fail-fast")]
    pub fail_fast: bool,
    #[serde(rename = "job-index")]
    pub job_index: u64,
    #[serde(rename = "job-total")]
    pub job_total: u64,
    #[serde(rename = "max-parallel")]
    pub max_parallel: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct StepContext {
    /// How the step ended, before `continue-on-error` is applied.
    pub outcome: Conclusion,
    /// How the step is reported, after it.
    pub conclusion: Conclusion,
    pub outputs: BTreeMap<String, String>,
}

/// Everything a step's expressions can see.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RunContext {
    pub github: Github,
    pub runner: Runner,
    pub env: BTreeMap<String, String>,
    pub matrix: Option<BTreeMap<String, Value>>,
    pub needs: BTreeMap<String, JobResult>,
    pub steps: BTreeMap<String, Value>,
    pub inputs: BTreeMap<String, String>,
    pub job: Job,
    pub strategy: Strategy,
    pub vars: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, String>,
}

impl RunContext {
    pub fn to_expr_context(&self) -> Context {
        let value = to_value(self).expect("a run context is always serialisable");
        Context::from_value(value, self.job.status.status()).with_workspace(&self.github.workspace)
    }

    pub fn to_env(&self) -> BTreeMap<String, String> {
        let (github, runner) = (&self.github, &self.runner);
        let mut vars = BTreeMap::new();

        vars.insert("CI".to_owned(), "true".to_owned());
        vars.insert("GITHUB_ACTIONS".to_owned(), "true".to_owned());

        let mut set = |key: &str, value: String| {
            vars.insert(key.to_owned(), value);
        };

        set("GITHUB_ACTION", github.action.clone());
        // Only where there is one to give: outside a composite action there is no action
        // path, and a step that looks for one is meant to find nothing rather than empty.
        if let Some(path) = &github.action_path {
            set("GITHUB_ACTION_PATH", path.display().to_string());
        }
        set("GITHUB_ACTION_REF", github.action_ref.clone());
        set("GITHUB_ACTION_REPOSITORY", github.action_repository.clone());
        set("GITHUB_ACTOR", github.actor.clone());
        set("GITHUB_ACTOR_ID", github.actor_id.clone());
        set("GITHUB_API_URL", github.api_url.clone());
        set("GITHUB_BASE_REF", github.base_ref.clone());
        set("GITHUB_ENV", github.env_file.clone());
        set("GITHUB_EVENT_NAME", github.event_name.clone());
        set("GITHUB_EVENT_PATH", github.event_path.clone());
        set("GITHUB_GRAPHQL_URL", github.graphql_url.clone());
        set("GITHUB_HEAD_REF", github.head_ref.clone());
        set("GITHUB_JOB", github.job.clone());
        set("GITHUB_OUTPUT", github.output_file.clone());
        set("GITHUB_PATH", github.path_file.clone());
        set("GITHUB_REF", github.r#ref.clone());
        set("GITHUB_REF_NAME", github.ref_name.clone());
        set("GITHUB_REF_PROTECTED", github.ref_protected.to_string());
        set("GITHUB_REF_TYPE", github.ref_type.clone());
        set("GITHUB_REPOSITORY", github.repository.clone());
        set("GITHUB_REPOSITORY_ID", github.repository_id.clone());
        set("GITHUB_REPOSITORY_OWNER", github.repository_owner.clone());
        set(
            "GITHUB_REPOSITORY_OWNER_ID",
            github.repository_owner_id.clone(),
        );
        set("GITHUB_RETENTION_DAYS", github.retention_days.to_string());
        set("GITHUB_RUN_ATTEMPT", github.run_attempt.to_string());
        set("GITHUB_RUN_ID", github.run_id.to_string());
        set("GITHUB_RUN_NUMBER", github.run_number.to_string());
        set("GITHUB_SERVER_URL", github.server_url.clone());
        set("GITHUB_SHA", github.sha.clone());
        set("GITHUB_STEP_SUMMARY", github.step_summary_file.clone());
        set("GITHUB_TRIGGERING_ACTOR", github.triggering_actor.clone());
        set("GITHUB_WORKFLOW", github.workflow.clone());
        set("GITHUB_WORKFLOW_REF", github.workflow_ref.clone());
        set("GITHUB_WORKFLOW_SHA", github.workflow_sha.clone());
        set("GITHUB_WORKSPACE", github.workspace.clone());
        set("RUNNER_ARCH", runner.arch.clone());
        set("RUNNER_ENVIRONMENT", runner.environment.clone());
        set("RUNNER_NAME", runner.name.clone());
        set("RUNNER_OS", runner.os.clone());
        set("RUNNER_TEMP", runner.temp.clone());
        set("RUNNER_TOOL_CACHE", runner.tool_cache.clone());

        // Only set when debug logging is enabled, and then to `1` rather than `true`.
        if runner.debug {
            vars.insert("RUNNER_DEBUG".to_owned(), "1".to_owned());
        }

        vars
    }
}

pub fn step_result(
    outcome: Conclusion,
    conclusion: Conclusion,
    outputs: &BTreeMap<String, String>,
) -> Value {
    let step = StepContext {
        outcome,
        conclusion,
        outputs: outputs.clone(),
    };
    to_value(&step).expect("a step context is always serialisable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_the_contexts_expressions_expect() {
        let mut run = RunContext::default();
        run.github.job = "build".to_owned();
        run.github.repository = "octocat/hello".to_owned();
        run.github.repository_owner = "octocat".to_owned();
        run.github.event_name = "push".to_owned();
        run.env.insert("GREETING".to_owned(), "hello".to_owned());
        run.needs.insert(
            "setup".to_owned(),
            JobResult {
                conclusion: Conclusion::Success,
                outputs: BTreeMap::from([("version".to_owned(), "1.2.3".to_owned())]),
            },
        );

        let context = run.to_expr_context();
        for source in [
            "github.repository == 'octocat/hello'",
            "github.repository_owner == 'octocat'",
            "github.job == 'build'",
            "env.GREETING == 'hello'",
            "needs.setup.outputs.version == '1.2.3'",
            "needs.setup.result == 'success'",
            "job.status == 'success'",
            "runner.debug == ''",
            "strategy['job-index'] == 0",
            "success()",
        ] {
            assert!(
                gh_actions_expr::eval_condition(source, &context).unwrap(),
                "{source} should hold"
            );
        }
    }
}
