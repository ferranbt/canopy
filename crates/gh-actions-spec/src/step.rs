//! The `steps` key: the individual units of work inside a job.

use serde::{Deserialize, Serialize};

use crate::common::{Env, Expr, With};
use crate::uses::Uses;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Step {
    /// What later steps read this one's outputs through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub r#if: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses: Option<Uses>,
    /// Mutually exclusive with `uses`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<With>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Env>,
    /// e.g. `bash`, `pwsh` or `python`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Whether failing here still lets the job succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_on_error: Option<Expr<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<Expr<u64>>,
}

impl Step {
    pub fn command(&self) -> Option<&str> {
        self.run.as_deref()
    }

    pub fn action(&self) -> Option<&Uses> {
        self.uses.as_ref()
    }
}
