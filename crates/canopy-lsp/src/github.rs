//! Asking GitHub what a `uses:` really points at.
//!
//! `git ls-remote` answers which commit a tag is at, and which tags are at a commit,
//! without a token or a rate limit; only the publish date needs the API, which allows
//! sixty requests an hour unauthenticated. Answers are kept for the life of the server.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// How long to wait on git before giving up. The API client keeps its own.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
enum Answer {
    Found(String),
    Missing,
}

pub struct Refs {
    api: octocrab::Octocrab,
    known: Mutex<HashMap<String, Answer>>,
}

impl Refs {
    pub fn new() -> Self {
        let token = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN"));
        // A token lifts the rate limit from sixty an hour to five thousand; without one the
        // client still works, so a missing token is no reason to refuse to start.
        let api = match token {
            Ok(token) => octocrab::Octocrab::builder()
                .personal_token(token)
                .build()
                .unwrap_or_default(),
            Err(_) => octocrab::Octocrab::default(),
        };

        Self {
            api,
            known: Mutex::new(HashMap::new()),
        }
    }

    pub async fn commit(&self, owner: &str, repo: &str, reference: &str) -> Option<String> {
        let key = format!("commit:{owner}/{repo}@{reference}");
        if let Some(known) = self.recall(&key) {
            return known;
        }

        let listing = self.ls_remote(owner, repo, &[], &[reference]).await?;
        // An annotated tag names itself first and the commit it points at second, and it is
        // the commit that a workflow actually runs.
        let found = listing
            .lines()
            .find(|line| line.ends_with("^{}"))
            .or_else(|| listing.lines().next())
            .and_then(|line| line.split_whitespace().next())
            .map(str::to_owned);

        self.remember(key, found.clone());
        found
    }

    pub async fn tag(&self, owner: &str, repo: &str, commit: &str) -> Option<String> {
        let key = format!("tag:{owner}/{repo}@{commit}");
        if let Some(known) = self.recall(&key) {
            return known;
        }

        let listing = self.ls_remote(owner, repo, &["--tags"], &[]).await?;
        // Several tags may stand at one commit — `v4` and `v4.2.1` usually both do — and the
        // longer one says more about what is pinned.
        let found = listing
            .lines()
            .filter(|line| line.starts_with(commit))
            .filter_map(|line| line.split_whitespace().nth(1))
            .filter_map(|name| name.strip_prefix("refs/tags/"))
            .map(|name| name.trim_end_matches("^{}").to_owned())
            .max_by_key(String::len);

        self.remember(key, found.clone());
        found
    }

    pub async fn published(&self, owner: &str, repo: &str, reference: &str) -> Option<String> {
        let key = format!("published:{owner}/{repo}@{reference}");
        if let Some(known) = self.recall(&key) {
            return known;
        }

        // A release carries a date GitHub set itself, which an upstream repository cannot
        // backdate, so it is asked for first. A branch, or a tag cut without a release, has
        // no such date and falls back to the commit.
        let release = self
            .api
            .repos(owner, repo)
            .releases()
            .get_by_tag(reference)
            .await
            .ok()
            .and_then(|release| release.published_at.or(release.created_at));

        let found = match release {
            Some(at) => Some(day(at)),
            None => self
                .api
                .commits(owner, repo)
                .get(reference)
                .await
                .ok()
                .and_then(|commit| {
                    let commit = commit.commit;
                    commit
                        .committer
                        .or(commit.author)
                        .and_then(|author| author.date)
                })
                .map(day),
        };

        self.remember(key, found.clone());
        found
    }

    /// Options go before the repository and patterns after it. Git does not complain when
    /// they are the wrong way round — it reads `--tags` as a pattern matching no ref, and
    /// succeeds with nothing to say.
    async fn ls_remote(
        &self,
        owner: &str,
        repo: &str,
        options: &[&str],
        patterns: &[&str],
    ) -> Option<String> {
        let url = format!("https://github.com/{owner}/{repo}");
        let mut command = tokio::process::Command::new("git");
        command
            .arg("ls-remote")
            .args(options)
            .arg(&url)
            .args(patterns);
        // Nothing here can answer a prompt, so a private repository must fail rather than
        // leave the editor waiting on a password.
        command.env("GIT_TERMINAL_PROMPT", "0");

        let output = tokio::time::timeout(TIMEOUT, command.output())
            .await
            .ok()?
            .ok()?;

        // A repository that is not there is an answer, and not worth asking again.
        if !output.status.success() {
            return Some(String::new());
        }

        String::from_utf8(output.stdout).ok()
    }

    fn recall(&self, key: &str) -> Option<Option<String>> {
        match self.known.lock().unwrap().get(key)? {
            Answer::Found(value) => Some(Some(value.clone())),
            Answer::Missing => Some(None),
        }
    }

    fn remember(&self, key: String, answer: Option<String>) {
        let answer = match answer {
            Some(value) => Answer::Found(value),
            None => Answer::Missing,
        };

        self.known.lock().unwrap().insert(key, answer);
    }
}

/// The day out of a timestamp, since the time of day says nothing useful here.
fn day(at: chrono::DateTime<chrono::Utc>) -> String {
    at.date_naive().to_string()
}

/// Whether a reference is a commit written out in full, which cannot move.
pub fn is_commit(reference: &str) -> bool {
    reference.len() == 40 && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_sha_is_a_commit_and_nothing_else_is() {
        assert!(is_commit("9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0"));
        assert!(!is_commit("v4"));
        assert!(!is_commit("main"));
        // Too short to be a commit, and GitHub forbids branches shaped like one anyway.
        assert!(!is_commit("9c091bb"));
        assert!(!is_commit("9c091bb21b7c1c1d1991bb908d89e4e9dddfe3eZ"));
    }
}
