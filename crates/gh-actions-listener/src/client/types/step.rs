//! A step as the service compiled it, which is not how the workflow file spelled it.

use std::collections::BTreeMap;

use gh_actions_encoding::token;
use serde::{Deserialize, Serialize};

/// Unknown fields are refused rather than ignored: this protocol is undocumented, and a
/// field appearing that this does not know about is the first sign of having drifted.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PipelineStep {
    /// Always `action`; a step is one whatever it runs.
    pub r#type: String,
    /// The id the service knows this step by, which is what results are reported against.
    pub id: String,
    pub name: String,
    #[serde(alias = "displayNameToken", with = "token")]
    pub display_name: String,
    pub reference: StepReference,
    /// What later steps read this one's outputs through, i.e. its `id`.
    pub context_name: Option<String>,
    /// Always present, and `success()` when the step declared no `if`.
    pub condition: Option<String>,
    #[serde(with = "token::maybe")]
    pub continue_on_error: Option<bool>,
    #[serde(with = "token::maybe")]
    pub timeout_in_minutes: Option<u64>,
    /// A script step keeps its script here; an action keeps what it was given.
    #[serde(with = "token")]
    pub inputs: BTreeMap<String, String>,
    #[serde(alias = "environment", with = "token")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum StepReference {
    /// A `run:` step, whose script is an input rather than part of the reference.
    #[default]
    Script,
    #[serde(rename_all = "camelCase")]
    Repository {
        name: String,
        #[serde(rename = "ref")]
        r#ref: String,
        #[serde(default)]
        path: Option<String>,
        /// `GitHub`, or the local repository the workflow itself came from.
        repository_type: String,
    },
    #[serde(rename_all = "camelCase")]
    ContainerRegistry { image: String },
}
