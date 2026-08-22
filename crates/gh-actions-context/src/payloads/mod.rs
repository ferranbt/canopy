//! The payloads GitHub delivers describing what actually happened.

pub mod common;
pub mod events;

use serde::{Deserialize, Serialize};

pub use common::{Author, Branch, Commit, Extra, Label, Repository, User};
pub use events::{
    Comment, Issue, IssueCommentEvent, IssuesEvent, PullRequest, PullRequestEvent, Push, Release,
    ReleaseEvent, Schedule, WorkflowDispatch, WorkflowRun, WorkflowRunEvent,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Payload {
    Push(Box<Push>),
    PullRequest(Box<PullRequestEvent>),
    PullRequestTarget(Box<PullRequestEvent>),
    Issues(Box<IssuesEvent>),
    IssueComment(Box<IssueCommentEvent>),
    Release(Box<ReleaseEvent>),
    WorkflowDispatch(Box<WorkflowDispatch>),
    WorkflowRun(Box<WorkflowRunEvent>),
    Schedule(Box<Schedule>),
    Other(serde_json::Value),
}

impl Default for Payload {
    fn default() -> Self {
        Self::Other(serde_json::Value::Object(serde_json::Map::new()))
    }
}

/// Because it is untagged, the macro Derive would take the first
/// entry that matches, but because all the entries have the values as defaults
/// the first entry would always match.
impl<'de> Deserialize<'de> for Payload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::Other(serde_json::Value::deserialize(deserializer)?))
    }
}

impl Payload {
    pub fn from_value(name: &str, payload: serde_json::Value) -> Result<Self, serde_json::Error> {
        Ok(match name {
            "push" => Self::Push(serde_json::from_value(payload)?),
            "pull_request" => Self::PullRequest(serde_json::from_value(payload)?),
            "pull_request_target" => Self::PullRequestTarget(serde_json::from_value(payload)?),
            "issues" => Self::Issues(serde_json::from_value(payload)?),
            "issue_comment" => Self::IssueComment(serde_json::from_value(payload)?),
            "release" => Self::Release(serde_json::from_value(payload)?),
            "workflow_dispatch" => Self::WorkflowDispatch(serde_json::from_value(payload)?),
            "workflow_run" => Self::WorkflowRun(serde_json::from_value(payload)?),
            "schedule" => Self::Schedule(serde_json::from_value(payload)?),
            _ => Self::Other(payload),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str, text: &str) -> Payload {
        Payload::from_value(name, serde_json::from_str(text).expect("json")).expect("payload")
    }

    #[test]
    fn a_pull_request_exposes_what_expressions_reach_for() {
        let payload = read(
            "pull_request",
            r#"{
                "action": "opened",
                "number": 7,
                "pull_request": {
                    "number": 7,
                    "title": "Add a thing",
                    "draft": false,
                    "head": {"ref": "feature", "sha": "def456"},
                    "base": {"ref": "main"},
                    "labels": [{"name": "enhancement"}]
                }
            }"#,
        );

        let Payload::PullRequest(pull) = &payload else {
            panic!("expected a pull request");
        };
        assert_eq!(pull.pull_request.head.r#ref, "feature");
        assert_eq!(pull.pull_request.base.r#ref, "main");
        assert_eq!(pull.pull_request.labels[0].name, "enhancement");
    }
}
