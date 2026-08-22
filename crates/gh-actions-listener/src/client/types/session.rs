//! Opening a session, and the token that authenticates every call in it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRequest<'a> {
    pub agent: AgentReference<'a>,
    pub owner_name: &'a str,
}

#[derive(Debug, Serialize)]
pub struct AgentReference<'a> {
    pub id: i64,
    pub name: &'a str,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub session_id: String,
    /// The AES key messages are encrypted under, itself usually RSA-wrapped.
    #[serde(default)]
    pub encryption_key: Option<EncryptionKey>,
    /// Whether the wrapping uses SHA-256 rather than SHA-1.
    #[serde(default)]
    pub use_fips_encryption: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionKey {
    /// Base64, and wrapped to the runner's public key when `encrypted` is set.
    pub value: String,
    #[serde(default)]
    pub encrypted: bool,
}

/// The runner's public key goes with it, because the broker mints the key an assignment is
/// encrypted to and has to be able to wrap it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSessionRequest<'a> {
    pub agent: BrokerAgent<'a>,
    pub owner_name: &'a str,
    pub use_fips_encryption: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerAgent<'a> {
    pub id: i64,
    pub name: &'a str,
    pub version: &'static str,
    pub authorization: crate::client::types::PublicKeyRequest,
}

/// camelCase, unlike the offer that names the job, which arrives in snake_case.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquireJobRequest<'a> {
    pub job_message_id: &'a str,
    #[serde(rename = "runnerOS")]
    pub runner_os: &'a str,
    pub billing_owner_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenewJobRequest<'a> {
    pub plan_id: &'a str,
    pub job_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteJobRequest<'a> {
    pub plan_id: &'a str,
    pub job_id: &'a str,
    /// `succeeded`, `failed` or `canceled`, as the service spells them.
    pub conclusion: &'a str,
    pub billing_owner_id: &'a str,
    #[serde(rename = "stepResults")]
    pub step_results: &'a [crate::client::types::StepResult],
}

#[derive(Debug, Clone)]
pub struct Token {
    pub value: String,
    /// Seconds from issue until it expires.
    pub expires_in: u64,
}
