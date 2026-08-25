//! The `action.yml` metadata file that defines a single action.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{Env, Scalar};
use crate::step::Step;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Action {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// In the order they were declared
    pub inputs: Option<indexmap::IndexMap<String, ActionInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<BTreeMap<String, ActionOutput>>,
    pub runs: Runs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branding: Option<Branding>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Scalar>,
    #[serde(
        default,
        rename = "deprecationMessage",
        skip_serializing_if = "Option::is_none"
    )]
    pub deprecation_message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Composite actions only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// The `runs` block, keyed by the `using` discriminator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "using")]
pub enum Runs {
    #[serde(rename = "composite")]
    Composite(CompositeRuns),
    #[serde(rename = "node16")]
    Node16(NodeRuns),
    #[serde(rename = "node20")]
    Node20(NodeRuns),
    #[serde(rename = "node24")]
    Node24(NodeRuns),
    #[serde(rename = "docker")]
    Docker(DockerRuns),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompositeRuns {
    /// A `run` step here needs an explicit `shell`.
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct NodeRuns {
    pub main: String,
    /// Runs at the start of the job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_if: Option<String>,
    /// Runs at the end of the job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_if: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DockerRuns {
    /// `Dockerfile` to build, or a `docker://` image to pull.
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Env>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branding {
    /// From the Feather set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}
