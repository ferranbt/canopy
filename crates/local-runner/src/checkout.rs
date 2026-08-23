//! Derive the RunContext from the current git directory

use std::path::{Path, PathBuf};
use std::process::Command;

use gh_actions_context::{
    Author, Commit, Github, Payload, Push, Repository, RunContext, Runner, User,
};

pub fn context(workspace: &Path, event_name: &str, temp: &Path, debug: bool) -> RunContext {
    let git = Git::new(workspace.to_path_buf());
    let branch = git.branch().unwrap_or_default();
    let repository = git
        .repository()
        .unwrap_or_else(|| "local/workspace".to_owned());

    let owner = repository.split('/').next().unwrap_or_default().to_owned();
    let actor = git.actor().unwrap_or_else(|| "canopy".to_owned());
    let sha = git.sha().unwrap_or_default();

    let event = match event_name {
        "push" => {
            let commit = Commit {
                id: sha.clone(),
                message: git.log("%B").unwrap_or_default(),
                author: Author {
                    name: git.log("%an").unwrap_or_default(),
                    email: git.log("%ae").unwrap_or_default(),
                    username: Some(actor.clone()),
                    ..Author::default()
                },
                ..Commit::default()
            };

            Payload::Push(Box::new(Push {
                r#ref: format!("refs/heads/{branch}"),
                after: sha.clone(),
                commits: vec![commit.clone()],
                head_commit: Some(commit),
                pusher: Author {
                    name: actor.clone(),
                    ..Author::default()
                },
                repository: Repository {
                    name: repository.split('/').nth(1).unwrap_or_default().to_owned(),
                    full_name: repository.clone(),
                    default_branch: branch.clone(),
                    owner: User {
                        login: owner.clone(),
                        ..User::default()
                    },
                    ..Repository::default()
                },
                sender: User {
                    login: actor.clone(),
                    ..User::default()
                },
                ..Push::default()
            }))
        }
        _ => Payload::default(),
    };

    RunContext {
        github: Github {
            actor,
            api_url: "https://api.github.com".to_owned(),
            event,
            event_name: event_name.to_owned(),
            graphql_url: "https://api.github.com/graphql".to_owned(),
            r#ref: format!("refs/heads/{branch}"),
            ref_name: branch,
            ref_type: "branch".to_owned(),
            repository_owner: owner,
            repository,
            retention_days: 0,
            run_attempt: 1,
            run_id: 1,
            run_number: 1,
            server_url: "https://github.com".to_owned(),
            sha,
            workspace: workspace.display().to_string(),
            ..Github::default()
        },
        runner: Runner {
            debug,
            ..Runner::host(temp)
        },
        ..RunContext::default()
    }
}

struct Git {
    workspace: PathBuf,
}

impl Git {
    fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }

    fn git(&self, args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.workspace.as_path())
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn repository(&self) -> Option<String> {
        let url = self.git(&["remote", "get-url", "origin"])?;

        let path = url
            .rsplit_once(':')
            .map_or(url.as_str(), |(_, path)| path)
            .trim_end_matches(".git");

        let mut parts = path.rsplit('/');
        let repo = parts.next()?;
        let owner = parts.next()?;
        Some(format!("{owner}/{repo}"))
    }

    fn sha(&self) -> Option<String> {
        self.git(&["rev-parse", "HEAD"])
    }

    fn branch(&self) -> Option<String> {
        self.git(&["rev-parse", "--abbrev-ref", "HEAD"])
    }

    fn actor(&self) -> Option<String> {
        self.git(&["config", "user.name"])
    }

    fn log(&self, format: &str) -> Option<String> {
        self.git(&["log", "-1", &format!("--pretty={format}")])
    }
}
