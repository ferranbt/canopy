//! Telling the results service what happened, which is what the web UI shows.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    pub external_id: String,
    pub number: u32,
    pub name: String,
    /// `pending`, `inProgress` or `completed`.
    pub status: &'static str,
    /// `succeeded`, `failed` or `skipped`; absent while the step is still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SignedLogUrlRequest<'a> {
    /// The plan, which is what the results service calls a run.
    pub workflow_run_backend_id: &'a str,
    pub workflow_job_run_backend_id: &'a str,
    /// Absent when the log is the job's rather than one step's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_backend_id: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedLogUrl {
    pub logs_url: String,
    #[serde(default)]
    pub blob_storage_type: String,
    #[serde(default)]
    pub soft_size_limit: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct LogsMetadata<'a> {
    pub workflow_run_backend_id: &'a str,
    pub workflow_job_run_backend_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_backend_id: Option<&'a str>,
    pub uploaded_at: String,
    pub line_count: u64,
}

#[derive(Debug, Serialize)]
pub struct StepsUpdateRequest<'a> {
    pub workflow_run_backend_id: &'a str,
    pub workflow_job_run_backend_id: &'a str,
    /// Orders updates that arrive out of order.
    pub change_order: u64,
    pub steps: &'a [StepResult],
}

/// The moment, spelled the way the service spells one.
pub fn timestamp(at: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at)
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}
