//! Registering a runner: what is asked for, and what comes back.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct TenantRequest<'a> {
    pub url: &'a str,
    pub runner_event: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tenant {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct Pools {
    pub value: Vec<Pool>,
}

#[derive(Debug, Deserialize)]
pub struct Pool {
    pub id: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequest {
    pub name: String,
    pub version: &'static str,
    pub os_description: &'static str,
    pub max_parallelism: u32,
    pub ephemeral: bool,
    /// A third-party runner has no binaries to swap out, so it asks not to be updated.
    pub disable_update: bool,
    pub provisioning_state: &'static str,
    pub labels: Vec<Label>,
    pub authorization: PublicKeyRequest,
}

#[derive(Debug, Serialize)]
pub struct Label {
    pub name: String,
    /// `system` for the ones every runner has, `user` for the rest.
    pub r#type: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKeyRequest {
    pub public_key: PublicKey,
}

/// Base64 of big-endian bytes, which is how the service spells a key.
#[derive(Debug, Serialize)]
pub struct PublicKey {
    pub exponent: String,
    pub modulus: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub id: i64,
    pub authorization: AgentAuthorization,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAuthorization {
    pub client_id: String,
    pub authorization_url: String,
}
