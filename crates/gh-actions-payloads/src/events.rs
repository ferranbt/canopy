use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{Author, Branch, Commit, Extra, Label, Repository, User, null_as_default};

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
    /// Where the branch was branched from, on the push that created it.
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub compare: String,
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
    pub merged_at: Option<String>,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    #[serde(default)]
    pub mergeable: Option<bool>,
    #[serde(default)]
    pub user: User,
    #[serde(default)]
    pub head: Branch,
    #[serde(default)]
    pub base: Branch,
    #[serde(default, deserialize_with = "null_as_default")]
    pub labels: Vec<Label>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub assignees: Vec<User>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub requested_reviewers: Vec<User>,
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
    #[serde(default, deserialize_with = "null_as_default")]
    pub assignees: Vec<User>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub state_reason: Option<String>,
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
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub author_association: String,
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
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub author: User,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub assets: Vec<serde_json::Value>,
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
pub struct WorkflowJobEvent {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub workflow_job: WorkflowJob,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowJob {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub run_id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub workflow_name: Option<String>,
    #[serde(default)]
    pub head_branch: Option<String>,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub runner_name: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub labels: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub steps: Vec<serde_json::Value>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CheckRunEvent {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub check_run: CheckRun,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CheckRun {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub details_url: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LabelEvent {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub label: Label,
    #[serde(default)]
    pub changes: Option<serde_json::Value>,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
    #[serde(flatten)]
    pub other: Extra,
}

/// A branch or a tag having come into being. What takes one away is `delete`, which is the
/// same payload without the `master_branch`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Create {
    #[serde(default, rename = "ref")]
    pub r#ref: String,
    /// `branch` or `tag`.
    #[serde(default)]
    pub ref_type: String,
    #[serde(default)]
    pub master_branch: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub pusher_type: String,
    #[serde(default)]
    pub repository: Repository,
    #[serde(default)]
    pub sender: User,
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
