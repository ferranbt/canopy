//! The top level of a workflow file under `.github/workflows`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{Concurrency, Defaults, Env, Permissions};
use crate::events::On;
use crate::job::Job;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Workflow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Names the individual run, as the run list shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_name: Option<String>,
    pub on: On,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Env>,
    /// Applied to every `run` step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Defaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<Concurrency>,
    #[serde(default)]
    pub jobs: BTreeMap<String, Job>,
}
