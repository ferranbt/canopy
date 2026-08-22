//! Types shared between workflows, jobs and steps.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

/// Keeps a key written as null distinct from one left out.
pub(crate) fn present_or_null<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A field written as one value or as a list of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    pub fn as_slice(&self) -> &[T] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
}

/// Written unquoted or not, and always used as a string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    String(String),
    Bool(bool),
    Int(i64),
    Float(f64),
}

pub type Env = BTreeMap<String, Scalar>;

/// A literal, or an `${{ }}` expression settled at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expr<T> {
    Value(T),
    Expression(String),
}

/// A blanket grant, or a table of scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Permissions {
    All(PermissionsAll),
    Scopes(BTreeMap<String, PermissionLevel>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionsAll {
    ReadAll,
    WriteAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionLevel {
    Read,
    Write,
    None,
}

/// A bare group name, or a group with a cancellation policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Concurrency {
    Group(String),
    Settings(ConcurrencySettings),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ConcurrencySettings {
    /// Only one run of a group is active at a time.
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_in_progress: Option<Expr<bool>>,
}

/// What every `run` step in scope falls back to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunDefaults>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RunDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

/// What `with:` and a reusable workflow's `inputs:` hold.
pub type With = BTreeMap<String, Scalar>;
