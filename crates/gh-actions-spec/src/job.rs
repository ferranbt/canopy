//! The `jobs` key: units of work that run on a runner or call another workflow.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{Concurrency, Defaults, Env, Expr, OneOrMany, Permissions, Scalar, With};
use crate::step::Step;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Job {
    /// Tried first, since only this one has a `uses` and it is required.
    Reusable(Box<ReusableJob>),
    Normal(Box<NormalJob>),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct NormalJob {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<OneOrMany<String>>,
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub r#if: Option<String>,
    /// e.g. `ubuntu-latest`, or a set of labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_on: Option<RunsOn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<Concurrency>,
    /// What jobs that `needs` this one can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Env>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Defaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<Step>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<Strategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Container>,
    /// Keyed by the hostname each answers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub services: Option<BTreeMap<String, Container>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<Expr<u64>>,
    /// Whether failing here still lets the workflow succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_on_error: Option<Expr<bool>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ReusableJob {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<OneOrMany<String>>,
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub r#if: Option<String>,
    /// `owner/repo/.github/workflows/file.yml@ref`, or `./path`.
    pub uses: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<With>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Secrets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<Strategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<Concurrency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunsOn {
    Labels(OneOrMany<String>),
    Group(RunnerGroup),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<OneOrMany<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Environment {
    Name(String),
    Detailed(EnvironmentDetails),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentDetails {
    /// Carries its own secrets and protection rules.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Secrets {
    /// Passes every secret of the calling workflow.
    Inherit(SecretsInherit),
    Explicit(BTreeMap<String, Scalar>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretsInherit {
    Inherit,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Strategy {
    /// Each combination it expands to is one job run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<Matrix>,
    /// Whether the first failure calls off the rest of the matrix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_fast: Option<Expr<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel: Option<Expr<u64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Matrix {
    /// e.g. `${{ fromJSON(...) }}`.
    Expression(String),
    Literal(MatrixLiteral),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MatrixLiteral {
    /// Extra combinations, or extra values for combinations already there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<BTreeMap<String, MatrixValue>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<BTreeMap<String, MatrixValue>>>,
    /// Expanded as a cartesian product.
    #[serde(flatten)]
    pub axes: BTreeMap<String, Vec<MatrixValue>>,
}

/// A nested map is addressable as `matrix.key.sub`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MatrixValue {
    Scalar(Scalar),
    List(Vec<MatrixValue>),
    Map(BTreeMap<String, MatrixValue>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Container {
    Image(String),
    Settings(ContainerSettings),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContainerSettings {
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<ContainerCredentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Env>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<Scalar>>,
    /// As `source:destination`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<String>>,
    /// Passed to `docker create` as they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerCredentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}
