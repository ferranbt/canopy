//! The payloads GitHub delivers describing what actually happened.
//!
//! `record-webhooks`, behind the `record` feature, is what the fixtures under `fixtures/`
//! were recorded with.

pub mod common;
pub mod events;

use serde::{Deserialize, Serialize};

pub use common::{Author, Branch, Commit, Extra, Label, Repository, User};
pub use events::{
    CheckRun, CheckRunEvent, Comment, Create, Issue, IssueCommentEvent, IssuesEvent, LabelEvent,
    PullRequest, PullRequestEvent, Push, Release, ReleaseEvent, Schedule, WorkflowDispatch,
    WorkflowJob, WorkflowJobEvent, WorkflowRun, WorkflowRunEvent,
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
    WorkflowJob(Box<WorkflowJobEvent>),
    CheckRun(Box<CheckRunEvent>),
    Label(Box<LabelEvent>),
    Create(Box<Create>),
    Delete(Box<Create>),
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
            "workflow_job" => Self::WorkflowJob(serde_json::from_value(payload)?),
            "check_run" => Self::CheckRun(serde_json::from_value(payload)?),
            "label" => Self::Label(serde_json::from_value(payload)?),
            "create" => Self::Create(serde_json::from_value(payload)?),
            "delete" => Self::Delete(serde_json::from_value(payload)?),
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

    #[test]
    fn test_fixtures_webhooks() {
        let delivered = fixtures();
        for (file, payload) in &delivered {
            let event = file.split('.').next().expect("an event");
            assert!(
                !matches!(payload, Payload::Other(_)) || !modelled(event),
                "{file} is modelled but was read as anything at all"
            );
        }

        for wanted in ["push", "pull_request", "issues", "issue_comment", "release"] {
            assert!(
                delivered.iter().any(|(file, _)| file.starts_with(wanted)),
                "{wanted} recorded"
            );
        }

        let pushes: Vec<_> = delivered
            .iter()
            .filter_map(|(_, payload)| match payload {
                Payload::Push(push) => Some(push),
                _ => None,
            })
            .collect();
        assert!(
            pushes
                .iter()
                .all(|push| push.r#ref.starts_with("refs/") && !push.after.is_empty())
        );
        assert!(
            pushes.iter().any(|push| !push.commits.is_empty()),
            "one of them carried a commit"
        );

        let Payload::PullRequest(pull) = recorded("pull_request.opened") else {
            panic!("a pull request");
        };
        assert_eq!(pull.action, "opened");
        assert!(pull.pull_request.number > 0);
        assert!(!pull.pull_request.head.r#ref.is_empty());
        assert!(!pull.pull_request.base.r#ref.is_empty());

        let Payload::Issues(issue) = recorded("issues.labeled") else {
            panic!("an issue");
        };
        assert_eq!(issue.action, "labeled");
        assert!(issue.issue.number > 0);
        assert!(issue.label.is_some(), "the label it was given");
        assert!(issue.issue.html_url.contains("/issues/"));
        assert!(!issue.sender.login.is_empty());

        let Payload::Release(release) = recorded("release.published") else {
            panic!("a release");
        };
        assert!(!release.release.tag_name.is_empty());
        assert!(release.release.html_url.contains("/releases/"));

        let Payload::Create(create) = recorded("create") else {
            panic!("a branch or a tag");
        };
        assert!(matches!(create.ref_type.as_str(), "branch" | "tag"));
        assert!(!create.master_branch.is_empty());

        let Payload::Label(label) = recorded("label.created") else {
            panic!("a label");
        };
        assert_eq!(label.action, "created");
        assert!(!label.label.name.is_empty());

        let Payload::WorkflowJob(job) = recorded("workflow_job.queued") else {
            panic!("a job");
        };
        assert_eq!(job.action, "queued");
        assert!(job.workflow_job.run_id > 0);
        assert!(!job.workflow_job.head_sha.is_empty());

        let Payload::CheckRun(check) = recorded("check_run.created") else {
            panic!("a check run");
        };
        assert!(!check.check_run.name.is_empty());
        assert!(!check.check_run.head_sha.is_empty());

        let Payload::WorkflowRun(run) = recorded("workflow_run.requested") else {
            panic!("a run");
        };
        assert_eq!(run.action, "requested");
        assert!(run.workflow_run.id > 0);
        assert!(!run.workflow_run.head_branch.is_empty());
    }

    fn fixtures_directory() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/webhooks")
    }

    fn recorded(name: &str) -> Payload {
        let path = fixtures_directory().join(format!("{name}.json"));
        let text = std::fs::read_to_string(path).expect("the payload reads");

        read(name.split('.').next().expect("an event"), &text)
    }

    fn fixtures() -> Vec<(String, Payload)> {
        let mut delivered: Vec<(String, Payload)> = std::fs::read_dir(fixtures_directory())
            .expect("the fixtures are there")
            .flatten()
            .filter_map(|entry| {
                let file = entry.file_name().to_string_lossy().into_owned();
                let name = file.strip_suffix(".json")?.to_owned();

                Some((name.clone(), recorded(&name)))
            })
            .collect();
        delivered.sort_by(|one, two| one.0.cmp(&two.0));

        delivered
    }

    fn modelled(event: &str) -> bool {
        !matches!(
            Payload::from_value(event, serde_json::json!({})),
            Ok(Payload::Other(_))
        )
    }
}
