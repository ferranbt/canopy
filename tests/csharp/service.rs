//! What the runner talks to.
//!
//! Not the real API: what the runner never looks at is canned, and what it does look at is
//! answered from the types the client already models. It keeps what the runner reports, so
//! whoever ran the job can read back what happened.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use gh_actions_listener::client::types::{
    ActionDownload, ActionDownloads, ActionReference, ActionReferences, Agent, AgentAuthorization,
    ConnectionData, Envelope, Lines, Pool, Record, ResourceLocation, Session,
};
use tracing::{debug, trace, warn};

/// Where the runner is told the service is.
///
/// Port 80: it rebuilds the url it was given as `{scheme}://{host}`, so whatever port it
/// was told about is lost before it asks for anything.
pub const BASE: &str = "http://127.0.0.1/canopy/tests/";

/// The token everything is done with, which nothing here ever checks.
const TOKEN: &str = "canopy";

const SESSION: &str = "00000000-0000-0000-0000-000000000005";

pub enum Update {
    Records(Vec<Record>),
    Printed(Lines),
    /// A step's whole log, uploaded once it is over rather than as it goes.
    Log {
        step: String,
        text: String,
    },
    /// What the job came out with, which the jobs after it are given.
    Outputs(BTreeMap<String, String>),
}

#[derive(Clone, Default)]
pub struct Service {
    /// The one job there is to hand over, taken by the runner that asks for it.
    job: Arc<Mutex<Option<serde_json::Value>>>,
    /// Whose log is whose: a runner asks for one per step, named after the step.
    logs: Arc<Mutex<BTreeMap<i64, String>>>,
    /// Where what the runner says goes, while it is still saying it.
    updates: Arc<Mutex<Option<Sender<Update>>>>,
}

impl Service {
    pub fn hand_over(&self, job: serde_json::Value, updates: Sender<Update>) {
        *self.job.lock().expect("the job") = Some(job);
        *self.updates.lock().expect("the updates") = Some(updates);

        debug!("a job is ready to be picked up");
    }

    fn send(&self, update: Update) {
        if let Some(updates) = self.updates.lock().expect("the updates").as_ref() {
            let _ = updates.send(update);
        }
    }

    /// Serves until the handle it returns is dropped.
    pub fn start(&self) -> Result<Listening, String> {
        let serving = self.clone();
        let (ready, wait) = std::sync::mpsc::channel();
        let (stop, stopping) = tokio::sync::oneshot::channel();

        let thread = std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = ready.send(Err(err.to_string()));
                    return;
                }
            };

            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::bind("127.0.0.1:80").await {
                    Ok(listener) => listener,
                    Err(err) => {
                        let _ = ready.send(Err(format!("port 80: {err}")));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));

                let app = Router::new().fallback(any(answer)).with_state(serving);
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = stopping.await;
                    })
                    .await;
            });
        });

        wait.recv().map_err(|_| "the service died".to_owned())??;
        Ok(Listening {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

/// Stops the service when it goes.
pub struct Listening {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A batch of records, in the envelope the runner sends them in.
#[derive(serde::Deserialize)]
struct Records {
    value: Vec<Record>,
}

/// What a runner says when a job is over, of which only what it came out with is of use.
#[derive(serde::Deserialize)]
struct Ended {
    outputs: BTreeMap<String, Said>,
}

#[derive(serde::Deserialize)]
struct Said {
    value: String,
}

async fn answer(State(service): State<Service>, method: Method, uri: Uri, body: Bytes) -> Response {
    let path = uri.path().to_owned();
    let body = String::from_utf8_lossy(&body).to_string();
    trace!(%method, %uri, bytes = body.len(), "asked");

    // What it reports: the timeline it writes a run down on, and what a step printed.
    if path.contains("/timelines") {
        if let Ok(update) = serde_json::from_str::<Records>(&body) {
            service.send(Update::Records(update.value));
        }
        if let Ok(lines) = serde_json::from_str::<Lines>(&body)
            && !lines.value.is_empty()
        {
            service.send(Update::Printed(lines));
        }

        return axum::Json(serde_json::json!({ "count": 0, "value": [] })).into_response();
    }

    // What a step printed, uploaded whole once it is over rather than line by line.
    if let Some((_, number)) = path.split_once("/logs/")
        && let Ok(id) = number.trim_end_matches('/').parse::<i64>()
    {
        let kept = service.logs.lock().expect("the logs").get(&id).cloned();
        let Some(kept) = kept else {
            return axum::Json(serde_json::json!({})).into_response();
        };

        if !body.is_empty() {
            service.send(Update::Log {
                step: step_of(&kept),
                text: body,
            });
        }

        return axum::Json(log(id, &kept)).into_response();
    }

    // Somewhere to put it, which the runner names after the step it belongs to and then
    // insists on being told back, both now and every time it uploads to it.
    if path.ends_with("/logs") {
        let asked: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let kept = asked
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();

        let mut logs = service.logs.lock().expect("the logs");
        let id = logs.len() as i64 + 1;
        logs.insert(id, kept.clone());

        return axum::Json(log(id, &kept)).into_response();
    }

    // Where the actions a job uses come from, which it resolves before it runs any of them.
    if path.ends_with("/actiondownloadinfo") {
        let asked: ActionReferences = serde_json::from_str(&body).unwrap_or_default();
        let downloads = ActionDownloads {
            actions: asked.actions.iter().map(download).collect(),
        };

        return axum::Json(downloads).into_response();
    }

    // How a job ended, which is where what it came out with is said.
    if path.ends_with("/events") {
        if let Ok(ended) = serde_json::from_str::<Ended>(&body) {
            let outputs: BTreeMap<String, String> = ended
                .outputs
                .into_iter()
                .map(|(name, said)| (name, said.value))
                .collect();

            debug!(?outputs, "the runner says what the job came out with");
            service.send(Update::Outputs(outputs));
        }

        return axum::Json(serde_json::json!({})).into_response();
    }

    // A poll answered with a body reads as a message, so having nothing has to be a status.
    // There is only ever the one job: the runner that takes it runs it and stops.
    if path.ends_with("/messages") && method == Method::GET {
        return match service.job.lock().expect("the job").take() {
            None => StatusCode::NO_CONTENT.into_response(),
            Some(job) => {
                debug!("handing the job over");
                axum::Json(Envelope {
                    message_id: Some(1),
                    message_type: "PipelineAgentJobRequest".to_owned(),
                    body: job.to_string(),
                    iv: None,
                })
                .into_response()
            }
        };
    }

    axum::Json(canned(&path, &method)).into_response()
}

/// Which one of something a call is about, which is the last thing its path names.
fn asked_about(path: &str) -> Option<i64> {
    path.rsplit('/').next()?.parse().ok()
}

/// A log as a runner reads one back, which it will not do without the path it named.
fn log(id: i64, path: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "path": path,
        "lineCount": 0,
        "createdOn": "2020-01-01T00:00:00Z",
        "lastChangedOn": "2020-01-01T00:00:00Z",
    })
}

/// Whose log it is, which is what the runner named it after.
fn step_of(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Where one action comes from: GitHub itself, which is where canopy fetches it too.
fn download(action: &ActionReference) -> (String, ActionDownload) {
    let source = format!(
        "https://codeload.github.com/{}/tar.gz/{}",
        action.name_with_owner, action.r#ref
    );

    (
        action.key(),
        ActionDownload {
            name_with_owner: action.name_with_owner.clone(),
            r#ref: action.r#ref.clone(),
            resolved_name_with_owner: action.name_with_owner.clone(),
            resolved_sha: action.r#ref.clone(),
            tarball_url: source.clone(),
            zipball_url: source,
        },
    )
}

/// What a runner is told, grown one answer at a time from whatever it refused to go on
/// without.
fn canned(path: &str, method: &Method) -> serde_json::Value {
    match path {
        // What it trades its registration token for: where to talk, and what with.
        "/api/v3/actions/runner-registration" => serde_json::json!({
            "url": BASE.trim_end_matches('/'),
            "token": TOKEN,
            // Which credential to talk with, which a client is told out of band.
            "token_schema": "OAuthAccessToken",
        }),
        // Where everything else is. It looks itself up in here before talking to anything.
        path if path.ends_with("/_apis/connectionData") => json(ConnectionData::at(BASE)),
        // What lives where. It calls nothing whose location is not in here, and says which
        // one it wanted when it is missing.
        path if path.ends_with("/_apis/") => many(locations()),
        // One pool, which is the group the runner puts itself in.
        path if path.ends_with("/distributedtask/pools") => many(vec![Pool {
            id: 1,
            name: "Default".to_owned(),
            is_hosted: false,
            pool_type: "automation".to_owned(),
        }]),
        // Whether this runner is registered already, which it never is.
        path if path.ends_with("/agents") && method == Method::GET => many(Vec::<Pool>::new()),
        // Registering: it is told which agent it now is, and where to go for a token.
        path if path.ends_with("/agents") => json(Agent {
            id: 1,
            authorization: AgentAuthorization {
                client_id: "00000000-0000-0000-0000-000000000004".to_owned(),
                authorization_url: format!("{BASE}_apis/oauth2/token"),
            },
        }),
        path if path.ends_with("/oauth2/token") => serde_json::json!({
            "access_token": TOKEN,
            "token_type": "bearer",
            "expires_in": 3600,
        }),
        // A session, which is what it polls for work under.
        path if path.ends_with("/sessions") => json(Session {
            session_id: SESSION.to_owned(),
            encryption_key: None,
            use_fips_encryption: false,
        }),
        // Keeping the job: a runner will not start one it cannot hold on to, and it holds
        // on to the request it was given rather than to whichever one it is told about.
        path if path.contains("/jobrequests") => serde_json::json!({
            "requestId": asked_about(path).unwrap_or(1),
            "lockToken": "00000000-0000-0000-0000-000000000014",
            "lockedUntil": (chrono::Utc::now() + chrono::Duration::hours(1))
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string(),
        }),
        // Nothing, which is an answer a runner has taken so far but is worth hearing about:
        // a call that matters and is answered with nothing is a runner that stops.
        _ => {
            warn!(%method, path, "asked for something this service does not know");
            serde_json::json!({})
        }
    }
}

/// The locations a runner needs between registering and finishing a job, which is fewer
/// than a real service offers.
fn locations() -> Vec<ResourceLocation> {
    let pools = "_apis/{area}/pools/{poolId}";
    let plans = "{scopeIdentifier}/_apis/{area}/hubs/{hubName}/plans/{planId}";

    vec![
        ResourceLocation::new(
            "a8c47e17-4d56-4a56-92bb-de7ea7dc65be",
            "pools",
            "_apis/{area}/{resource}/{poolId}",
        ),
        ResourceLocation::new(
            "e298ef32-5878-4cab-993c-043836571f42",
            "agents",
            &format!("{pools}/{{resource}}/{{agentId}}"),
        ),
        ResourceLocation::new(
            "134e239e-2df3-4794-a6f6-24f1f19ec8dc",
            "sessions",
            &format!("{pools}/{{resource}}/{{sessionId}}"),
        ),
        ResourceLocation::new(
            "c3a054f6-7a8a-49c0-944e-3a8e5d7adfd7",
            "messages",
            &format!("{pools}/{{resource}}/{{messageId}}"),
        ),
        ResourceLocation::new(
            "fc825784-c92a-4299-9221-998a02d1b54f",
            "jobrequests",
            &format!("{pools}/{{resource}}/{{requestId}}"),
        ),
        ResourceLocation::new(
            "27d7f831-88c1-4719-8ca1-6a061dad90eb",
            "actiondownloadinfo",
            &format!("{plans}/{{resource}}"),
        ),
        // What a job ended up with, which a runner only says once it is told the plan is
        // real enough to say it to.
        ResourceLocation::new(
            "557624af-b29e-4c20-8ab0-0399d2204f3f",
            "events",
            &format!("{plans}/{{resource}}"),
        ),
        ResourceLocation::new(
            "858983e4-19bd-4c5e-864c-507b59b58b12",
            "records",
            &format!("{plans}/timelines/{{timelineId}}/{{resource}}"),
        ),
        ResourceLocation::new(
            "46f5667d-263a-4684-91b1-dff7fdcf64e2",
            "logs",
            &format!("{plans}/{{resource}}/{{logId}}"),
        ),
        ResourceLocation::new(
            "8893bc5b-35b2-4be7-83cb-99e683551db4",
            "timelines",
            &format!("{plans}/{{resource}}/{{timelineId}}"),
        ),
    ]
}

fn json<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_default()
}

fn many<T: serde::Serialize>(value: Vec<T>) -> serde_json::Value {
    serde_json::json!({ "count": value.len(), "value": value })
}
