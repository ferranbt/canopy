//! The contexts the service sends with a job.

use std::collections::BTreeMap;

use gh_actions_context::Payload;
use serde::{Deserialize, Deserializer, Serialize};

/// Unknown fields are refused rather than ignored: this protocol is undocumented, and a
/// field appearing that this does not know about is the first sign of having drifted.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JobContext {
    pub github: GithubContext,
    pub job: JobDetails,
    pub strategy: StrategyContext,
    pub inputs: BTreeMap<String, String>,
    /// Already resolved to this job's combination by the service, and empty without one.
    pub matrix: BTreeMap<String, serde_json::Value>,
    pub needs: BTreeMap<String, NeedsResult>,
    pub vars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct GithubContext {
    pub event: serde_json::Value,
    pub event_name: String,
    pub repository: String,
    pub repository_owner: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub sha: String,
    pub actor: String,
    #[serde(deserialize_with = "number")]
    pub run_id: u64,
    #[serde(deserialize_with = "number")]
    pub run_number: u64,
    pub workflow: String,
    /// Anything else the service sent, which varies by event.
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct JobDetails {
    pub check_run_id: u64,
    pub workflow_ref: String,
    pub workflow_sha: String,
    pub workflow_repository: String,
    pub workflow_file_path: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct StrategyContext {
    #[serde(rename = "fail-fast")]
    pub fail_fast: bool,
    #[serde(rename = "job-index")]
    pub job_index: u64,
    #[serde(rename = "job-total")]
    pub job_total: u64,
    #[serde(rename = "max-parallel")]
    pub max_parallel: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct NeedsResult {
    pub result: String,
    pub outputs: BTreeMap<String, String>,
}

impl GithubContext {
    /// The name arrives beside the payload rather than in it, so nothing can type it until
    /// both are in hand. A payload this does not model is kept as it came, so an expression
    /// reading `github.event` finds the same thing either way.
    pub fn event(&self) -> Payload {
        Payload::from_value(&self.event_name, self.event.clone()).unwrap_or_else(|err| {
            tracing::warn!(%err, event = %self.event_name, "keeping the event untyped");
            Payload::Other(self.event.clone())
        })
    }

    /// The owner, which the service leaves out of some events but never out of the name.
    pub fn owner(&self) -> String {
        match self.repository_owner.is_empty() {
            true => self
                .repository
                .split('/')
                .next()
                .unwrap_or_default()
                .to_owned(),
            false => self.repository_owner.clone(),
        }
    }
}

fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_default())
}
