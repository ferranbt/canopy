use std::path::PathBuf;

use gh_actions_payloads::Payload;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Github {
    pub action: String,
    pub action_path: Option<PathBuf>,
    pub action_ref: String,
    pub action_repository: String,
    pub action_status: String,
    pub actor: String,
    pub actor_id: String,
    pub api_url: String,
    pub base_ref: String,
    pub event: Payload,
    pub event_name: String,
    pub event_path: String,
    pub graphql_url: String,
    pub head_ref: String,
    pub job: String,
    pub r#ref: String,
    pub ref_name: String,
    pub ref_protected: bool,
    pub ref_type: String,
    pub repository: String,
    pub repository_id: String,
    pub repository_owner: String,
    pub repository_owner_id: String,
    #[serde(rename = "repositoryUrl")]
    pub repository_url: String,
    pub retention_days: u64,
    pub run_attempt: u64,
    pub run_id: u64,
    pub run_number: u64,
    pub secret_source: String,
    pub server_url: String,
    pub sha: String,
    pub token: String,
    pub triggering_actor: String,
    pub workflow: String,
    pub workflow_ref: String,
    pub workflow_sha: String,
    pub workspace: String,
    #[serde(rename = "env")]
    pub env_file: String,
    #[serde(rename = "path")]
    pub path_file: String,
    #[serde(rename = "step_summary")]
    pub step_summary_file: String,
    #[serde(skip)]
    pub output_file: String,
}
