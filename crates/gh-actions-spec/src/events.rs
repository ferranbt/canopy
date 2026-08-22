//! The `on` key: the events that trigger a workflow.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::Scalar;

/// In any of the three forms GitHub accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum On {
    /// e.g. `on: push`.
    Event(String),
    /// e.g. `on: [push, pull_request]`.
    Events(Vec<String>),
    /// e.g. `on: { push: { branches: [main] } }`.
    Map(Box<Events>),
}

impl Default for On {
    fn default() -> Self {
        Self::Map(Box::default())
    }
}

/// The events whose configuration has a shape of its own are typed out; the rest are not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Events {
    #[serde(
        default,
        deserialize_with = "crate::common::present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub push: Option<Option<RefFilters>>,
    #[serde(
        default,
        deserialize_with = "crate::common::present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub pull_request: Option<Option<RefFilters>>,
    /// Like `pull_request`, but in the base repository's context, with its secrets.
    #[serde(
        default,
        deserialize_with = "crate::common::present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub pull_request_target: Option<Option<RefFilters>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<Schedule>>,
    #[serde(
        default,
        deserialize_with = "crate::common::present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub workflow_dispatch: Option<Option<WorkflowDispatch>>,
    /// Its presence is what makes the workflow reusable.
    #[serde(
        default,
        deserialize_with = "crate::common::present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub workflow_call: Option<Option<WorkflowCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run: Option<WorkflowRun>,
    /// Every event not named above, keyed by name.
    #[serde(flatten)]
    pub other: BTreeMap<String, Option<RefFilters>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RefFilters {
    /// e.g. `opened` or `synchronize`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branches: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branches_ignore: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags_ignore: Option<Vec<String>>,
    /// A changed file has to match one of these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
    /// A change to these never triggers the workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths_ignore: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// POSIX cron, in UTC.
    pub cron: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDispatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<BTreeMap<String, DispatchInput>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DispatchInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Scalar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<InputType>,
    /// The values allowed when the type is `choice`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    Boolean,
    Choice,
    Environment,
    Number,
    String,
}

/// A reusable workflow's interface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<BTreeMap<String, CallInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<BTreeMap<String, CallSecret>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<BTreeMap<String, CallOutput>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CallInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Scalar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<InputType>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSecret {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Usually reads a job output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WorkflowRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflows: Option<Vec<String>>,
    /// e.g. `completed` or `requested`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branches: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branches_ignore: Option<Vec<String>>,
}
