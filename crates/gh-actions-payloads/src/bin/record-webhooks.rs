use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use clap::Parser;
use octocrab::Octocrab;
use serde_json::{Value, json};
use tracing::info;

const NOWHERE: &str = "https://canopy.invalid/webhooks";

const SETTLES: Duration = Duration::from_secs(20);

type Whatever = Box<dyn std::error::Error>;

#[derive(Parser)]
#[command(about = "Provokes webhook events on a test repository and keeps what GitHub delivered")]
struct Cli {
    #[arg(long, help = "The repository to write to, as `owner/repo`")]
    repo: String,
    #[arg(long, default_value = "crates/gh-actions-payloads/fixtures/webhooks")]
    out: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Whatever> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let token = std::env::var("CANOPY_PROBE_TOKEN")?;
    let (owner, repo) = cli
        .repo
        .split_once('/')
        .ok_or("the repository is an owner and a repo")?;

    let github = Octocrab::builder().personal_token(token).build()?;
    let at = |rest: String| format!("/repos/{owner}/{repo}{rest}");

    let hook = hook(&github, &at).await?;
    info!(hook, repo = %cli.repo, "recording");

    let since = Utc::now();
    provoke(&github, &at).await?;

    info!(?SETTLES, "waiting for what that set off");
    tokio::time::sleep(SETTLES).await;

    let delivered = delivered(&github, &at, hook, since).await?;
    let kept = write(&cli.out, delivered)?;

    info!(kept, at = %cli.out.display(), "recorded");
    Ok(())
}

async fn hook(github: &Octocrab, at: &impl Fn(String) -> String) -> Result<u64, Whatever> {
    let hooks: Value = github.get(at("/hooks".to_owned()), None::<&()>).await?;
    let standing = hooks
        .as_array()
        .into_iter()
        .flatten()
        .find(|hook| hook.pointer("/config/url").and_then(Value::as_str) == Some(NOWHERE));

    if let Some(hook) = standing {
        return Ok(hook["id"].as_u64().ok_or("a hook with no id")?);
    }

    let made: Value = github
        .post(
            at("/hooks".to_owned()),
            Some(&json!({
                "name": "web",
                "active": true,
                "events": ["*"],
                "config": { "url": NOWHERE, "content_type": "json", "insecure_ssl": "0" },
            })),
        )
        .await?;

    Ok(made["id"].as_u64().ok_or("a hook with no id")?)
}

async fn provoke(github: &Octocrab, at: &impl Fn(String) -> String) -> Result<(), Whatever> {
    let about: Value = github.get(at(String::new()), None::<&()>).await?;
    let base = about["default_branch"]
        .as_str()
        .unwrap_or("main")
        .to_owned();
    let head: Value = github
        .get(at(format!("/git/ref/heads/{base}")), None::<&()>)
        .await?;
    let sha = head["object"]["sha"].as_str().ok_or("no sha")?.to_owned();
    let named = format!("canopy-{}", Utc::now().format("%Y%m%d-%H%M%S"));

    info!(%named, "a label");
    let _: Value = github
        .post(
            at("/labels".to_owned()),
            Some(&json!({ "name": &named, "color": "5319e7", "description": "recording" })),
        )
        .await?;

    info!("an issue, labelled, commented on, closed and opened again");
    let issue: Value = github
        .post(
            at("/issues".to_owned()),
            Some(&json!({ "title": format!("Recorded by {named}"), "body": "By the recorder." })),
        )
        .await?;
    let number = issue["number"].as_u64().unwrap_or_default();
    let _: Value = github
        .post(
            at(format!("/issues/{number}/labels")),
            Some(&json!({ "labels": [&named] })),
        )
        .await?;
    let comment: Value = github
        .post(
            at(format!("/issues/{number}/comments")),
            Some(&json!({ "body": "Said by the recorder." })),
        )
        .await?;
    let comment = comment["id"].as_u64().unwrap_or_default();
    let _: Value = github
        .patch(
            at(format!("/issues/comments/{comment}")),
            Some(&json!({ "body": "Said again." })),
        )
        .await?;
    github
        ._delete(at(format!("/issues/comments/{comment}")), None::<&()>)
        .await?;
    for state in ["closed", "open"] {
        let _: Value = github
            .patch(
                at(format!("/issues/{number}")),
                Some(&json!({ "state": state })),
            )
            .await?;
    }

    info!("a branch, a commit on it, and a pull request");
    let _: Value = github
        .post(
            at("/git/refs".to_owned()),
            Some(&json!({ "ref": format!("refs/heads/{named}"), "sha": &sha })),
        )
        .await?;
    let written = base64::engine::general_purpose::STANDARD.encode("So there is a push.\n");
    let _: Value = github
        .put(
            at(format!("/contents/{named}.md")),
            Some(&json!({
                "message": "What the recorder wrote",
                "content": written,
                "branch": &named,
            })),
        )
        .await?;
    let pull: Value = github
        .post(
            at("/pulls".to_owned()),
            Some(&json!({
                "title": format!("Recorded by {named}"),
                "head": &named,
                "base": &base,
                "body": "By the recorder.",
            })),
        )
        .await?;
    let opened = pull["number"].as_u64().unwrap_or_default();
    let _: Value = github
        .patch(
            at(format!("/pulls/{opened}")),
            Some(&json!({ "state": "closed" })),
        )
        .await?;

    info!("a release, and a dispatch");
    let _: Value = github
        .post(
            at("/releases".to_owned()),
            Some(&json!({
                "tag_name": &named,
                "name": format!("Recorded by {named}"),
                "target_commitish": &sha,
            })),
        )
        .await?;
    github
        ._post(
            at("/dispatches".to_owned()),
            Some(
                &json!({ "event_type": "canopy-webhooks", "client_payload": { "recorded": true } }),
            ),
        )
        .await?;

    Ok(())
}

async fn delivered(
    github: &Octocrab,
    at: &impl Fn(String) -> String,
    hook: u64,
    since: DateTime<Utc>,
) -> Result<BTreeMap<String, Value>, Whatever> {
    let listed: Value = github
        .get(
            at(format!("/hooks/{hook}/deliveries?per_page=100")),
            None::<&()>,
        )
        .await?;

    let mut kept = BTreeMap::new();
    for delivery in listed.as_array().into_iter().flatten().rev() {
        let sent = delivery["delivered_at"].as_str().unwrap_or_default();
        if sent.parse::<DateTime<Utc>>().is_ok_and(|sent| sent < since) {
            continue;
        }

        let id = delivery["id"].as_u64().unwrap_or_default();
        let whole: Value = github
            .get(at(format!("/hooks/{hook}/deliveries/{id}")), None::<&()>)
            .await?;
        let Some(payload) = whole.pointer("/request/payload") else {
            continue;
        };

        let event = delivery["event"].as_str().unwrap_or("unknown");
        let named = match payload["action"].as_str() {
            Some(action) => format!("{event}.{action}"),
            None => event.to_owned(),
        };

        let again = (2..).map(|nth| format!("{named}.{nth}"));
        let named = std::iter::once(named.clone())
            .chain(again)
            .find(|named| !kept.contains_key(named))
            .expect("a name of its own");

        kept.insert(named, payload.clone());
    }

    Ok(kept)
}

fn scrubbed(value: &Value) -> Value {
    match value {
        Value::String(said) if said.contains('@') && said.contains('.') && !said.contains(' ') => {
            Value::String("someone@example.com".to_owned())
        }
        Value::Array(items) => Value::Array(items.iter().map(scrubbed).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), scrubbed(value)))
                .collect(),
        ),
        value => value.clone(),
    }
}

fn write(at: &PathBuf, payloads: BTreeMap<String, Value>) -> Result<usize, Whatever> {
    std::fs::create_dir_all(at)?;

    for (named, payload) in &payloads {
        let path = at.join(format!("{named}.json"));
        let text = serde_json::to_string_pretty(&scrubbed(payload))?;
        std::fs::write(&path, text + "\n")?;
    }

    Ok(payloads.len())
}
