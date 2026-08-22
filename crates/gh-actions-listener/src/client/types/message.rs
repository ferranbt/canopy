//! The messages a poll answers with, and the job one of them carries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::client::types::{JobContext, PipelineStep, normalize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    /// Sent back on the next poll. Absent on a message that is not queued work.
    #[serde(default)]
    pub message_id: Option<i64>,
    pub message_type: String,
    /// Encrypted when the session has a key.
    pub body: String,
    #[serde(default)]
    pub iv: Option<String>,
}

impl Envelope {
    pub fn is_job(&self) -> bool {
        self.message_type
            .eq_ignore_ascii_case("PipelineAgentJobRequest")
    }

    /// The broker does not send the job itself, only where to collect it from.
    pub fn job_offer(&self) -> Option<JobOffer> {
        if !self.message_type.eq_ignore_ascii_case("RunnerJobRequest") {
            return None;
        }

        serde_json::from_str(&self.body).ok()
    }

    /// The service hands a runner over to the broker rather than answering polls itself,
    /// and says so once, in a message with no id of its own.
    pub fn broker_url(&self) -> Option<String> {
        if !self.message_type.eq_ignore_ascii_case("BrokerMigration") {
            return None;
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Migration {
            broker_base_url: String,
        }

        serde_json::from_str::<Migration>(&self.body)
            .ok()
            .map(|migration| migration.broker_base_url)
    }

    /// A third-party runner cannot swap out binaries it does not have, so the caller
    /// decides what to do: ignore it and hope, or stop.
    pub fn is_update(&self) -> bool {
        self.message_type
            .eq_ignore_ascii_case("AgentRefreshMessage")
    }
}

/// A job to run, already expanded: the matrix is resolved, `needs` outputs are filled in
/// and the step list is flat. That happened server side, so a listener needs no planner.
///
/// Unknown fields are refused rather than ignored, so a protocol that has moved on says so
/// the first time rather than by quietly running the wrong thing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct JobMessage {
    pub message_type: String,
    pub job_id: String,
    pub job_display_name: String,
    pub job_name: String,
    /// The request this came in on, which has to be renewed while the job runs.
    pub request_id: i64,
    pub billing_owner_id: String,
    pub plan: Plan,
    pub timeline: Timeline,
    /// Compiled by the service, so not the shape the workflow file spelled them in.
    pub steps: Vec<PipelineStep>,
    pub context_data: JobContext,
    pub variables: BTreeMap<String, Variable>,
    pub resources: serde_json::Value,
    pub mask: serde_json::Value,
    pub defaults: serde_json::Value,
    pub environment_variables: serde_json::Value,
    pub file_table: serde_json::Value,
    pub job_container: serde_json::Value,
    pub job_service_containers: serde_json::Value,
    pub job_outputs: serde_json::Value,
    pub locked_until: serde_json::Value,
    pub snapshot: serde_json::Value,
}

impl JobMessage {
    /// Reads a job as the service sends it, encodings and all.
    pub fn decode(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_value(normalize(serde_json::from_str(raw)?))
    }

    pub fn env(&self) -> BTreeMap<String, String> {
        self.variables
            .iter()
            .map(|(name, variable)| (name.clone(), variable.value.clone()))
            .collect()
    }

    pub fn feed_url(&self) -> Option<String> {
        Some(self.endpoint_data("FeedStreamUrl")?.to_owned())
    }

    pub fn run_service_url(&self) -> Option<String> {
        Some(
            self.resources
                .get("endpoints")?
                .as_array()?
                .first()?
                .get("url")?
                .as_str()?
                .to_owned(),
        )
    }

    pub fn results_url(&self) -> Option<String> {
        Some(self.endpoint_data("ResultsServiceUrl")?.to_owned())
    }

    fn endpoint_data(&self, name: &str) -> Option<&str> {
        self.resources
            .get("endpoints")?
            .as_array()?
            .iter()
            .find_map(|endpoint| endpoint.get("data")?.get(name)?.as_str())
    }

    /// The token this job was issued, which is what job-scoped calls are made with.
    ///
    /// Not the same as the one actions use: that one is a variable, this one comes with
    /// the endpoints the job was given.
    pub fn service_token(&self) -> Option<&str> {
        self.resources
            .get("endpoints")?
            .as_array()?
            .iter()
            .find_map(|endpoint| {
                endpoint
                    .get("authorization")?
                    .get("parameters")?
                    .get("AccessToken")?
                    .as_str()
            })
    }

    /// Values that must never reach the output, which the service marks as it sends them.
    pub fn secrets(&self) -> Vec<String> {
        self.variables
            .values()
            .filter(|variable| variable.is_secret && !variable.value.is_empty())
            .map(|variable| variable.value.clone())
            .collect()
    }

    pub fn runtime_token(&self) -> Option<&str> {
        self.variables
            .get("system.github.token")
            .or_else(|| self.variables.get("ACTIONS_RUNTIME_TOKEN"))
            .map(|variable| variable.value.as_str())
    }
}

/// The run service speaks snake_case, unlike the rest of the protocol.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobOffer {
    pub run_service_url: String,
    pub runner_request_id: String,
    #[serde(default)]
    pub billing_owner_id: String,
    /// Whether the message still has to be acknowledged against the old endpoint.
    #[serde(default)]
    pub should_acknowledge: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Variable {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub is_secret: bool,
}

/// The plan a job belongs to, which scopes where results are reported.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    #[serde(default)]
    pub plan_id: String,
    #[serde(default)]
    pub scope_identifier: String,
    #[serde(default)]
    pub plan_type: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    #[serde(default)]
    pub id: String,
}

/// How a job ended, in the words the service expects back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Outcome {
    #[default]
    Succeeded,
    Failed,
    Cancelled,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "canceled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn being_sent_to_the_broker_says_where_it_is() {
        let envelope: Envelope = serde_json::from_str(
            r#"{"messageType":"BrokerMigration","body":"{\"brokerBaseUrl\":\"https://broker.example\"}"}"#,
        )
        .expect("envelope decodes");

        assert_eq!(envelope.message_id, None, "it is not queued work");
        assert_eq!(
            envelope.broker_url().as_deref(),
            Some("https://broker.example")
        );
    }
}
