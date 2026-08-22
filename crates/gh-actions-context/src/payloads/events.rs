use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::payloads::common::{
    Author, Branch, Commit, Extra, Label, Repository, User, null_as_default,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Push {
    #[serde(default, rename = "ref")]
    pub r#ref: String,
    #[serde(default)]
    pub before: String,
    #[serde(default)]
    pub after: String,
    #[serde(default)]
    pub created: bool,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub forced: bool,
    #[serde(default, deserialize_with = "null_as_default")]
    pub commits: Vec<Commit>,
    #[serde(default)]
    pub head_commit: Option<Commit>,
    #[serde(default)]
    pub pusher: Author,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PullRequestEvent {
    /// e.g. `opened`, `synchronize`, `closed`.
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub pull_request: PullRequest,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PullRequest {
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub user: User,
    #[serde(default)]
    pub head: Branch,
    #[serde(default)]
    pub base: Branch,
    #[serde(default, deserialize_with = "null_as_default")]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub html_url: String,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IssuesEvent {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub issue: Issue,
    #[serde(default)]
    pub label: Option<Label>,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub user: User,
    #[serde(default, deserialize_with = "null_as_default")]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IssueCommentEvent {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub issue: Issue,
    #[serde(default)]
    pub comment: Comment,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub user: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseEvent {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub release: Release,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub target_commitish: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub author: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDispatch {
    #[serde(default, deserialize_with = "null_as_default")]
    pub inputs: BTreeMap<String, serde_json::Value>,
    #[serde(default, rename = "ref")]
    pub r#ref: String,
    #[serde(default)]
    pub workflow: String,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunEvent {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub workflow_run: WorkflowRun,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub event: String,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub schedule: String,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}
