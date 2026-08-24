//! The side of the protocol a runner talks to.
//!
//! What a runner insists on hearing before it will take a job is answered here; what it
//! then says about the job is handed to whoever is running it. What any of it is called —
//! the pool, the session, the token a job is held with — belongs to whoever serves it, so
//! all of it is asked for rather than assumed.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};

use crate::client::types::{
    ActionDownload, ActionDownloads, ActionReference, ActionReferences, Agent, AgentAuthorization,
    ConnectionData, Envelope, Granted, JobEnded, JobRequest, Lines, Many, Pool, Record,
    ResourceLocation, Session, TaskLog, Tenant,
};

/// Which run a call is about, which every call a job makes carries.
#[derive(Debug, Clone)]
pub struct Plan {
    pub scope: String,
    pub hub: String,
    pub id: String,
}

/// A job to hand over, under the number it goes out as.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub body: String,
}

/// Who is asking for work: one runner of one pool, under the session it registered.
#[derive(Debug, Clone)]
pub struct Asking {
    pub pool: i64,
    pub session: String,
    /// The last message it was given, so it is not given that one again.
    pub last: Option<i64>,
}

/// What a runner is given, and what it says it did.
pub trait Jobs: Send + Sync + 'static {
    fn take(&self, asking: &Asking) -> Option<Message>;

    /// Said once it has the job in hand, and is done with the message that carried it.
    fn read(&self, _pool: i64, _message: i64) {}

    fn records(&self, plan: &Plan, timeline: &str, records: Vec<Record>);

    fn printed(&self, plan: &Plan, timeline: &str, lines: Lines);

    /// What a step printed, uploaded whole once it has stopped printing.
    fn log(&self, plan: &Plan, log: i64, step: &str, text: String);

    fn ended(&self, plan: &Plan, ended: JobEnded);

    fn credential(&self) -> String {
        "canopy".to_owned()
    }

    fn pool(&self) -> Pool {
        Pool {
            id: 1,
            name: "Default".to_owned(),
            is_hosted: false,
            pool_type: "automation".to_owned(),
        }
    }

    fn agent(&self, at: &str) -> Agent {
        Agent {
            id: 1,
            authorization: AgentAuthorization {
                client_id: "00000000-0000-0000-0000-000000000004".to_owned(),
                authorization_url: format!("{at}_apis/oauth2/token"),
            },
        }
    }

    fn session(&self) -> Session {
        Session {
            session_id: "00000000-0000-0000-0000-000000000005".to_owned(),
            encryption_key: None,
            use_fips_encryption: false,
        }
    }

    /// What a runner holds a job with, which it will not start one without.
    fn holding(&self, request: i64) -> JobRequest {
        JobRequest {
            request_id: request,
            lock_token: "00000000-0000-0000-0000-000000000014".to_owned(),
            locked_until: (chrono::Utc::now() + chrono::Duration::hours(1))
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string(),
            result: None,
        }
    }

    /// Where an action comes from, which is GitHub itself unless said otherwise.
    fn download(&self, action: &ActionReference) -> ActionDownload {
        let source = format!(
            "https://codeload.github.com/{}/tar.gz/{}",
            action.name_with_owner, action.r#ref
        );

        ActionDownload {
            name_with_owner: action.name_with_owner.clone(),
            r#ref: action.r#ref.clone(),
            resolved_name_with_owner: action.name_with_owner.clone(),
            resolved_sha: action.r#ref.clone(),
            tarball_url: source.clone(),
            zipball_url: source,
        }
    }
}

pub struct JobServer<J: Jobs> {
    jobs: J,
    base: Arc<str>,
    /// Whose log is whose: a runner asks for one per step, named after the step.
    logs: Mutex<Vec<String>>,
}

impl<J: Jobs> JobServer<J> {
    pub fn new(jobs: J, base: impl Into<Arc<str>>) -> Self {
        Self {
            jobs,
            base: base.into(),
            logs: Mutex::new(Vec::new()),
        }
    }

    pub fn routes(self) -> Router {
        let pools = "/_apis/distributedtask/pools";
        let plans = "/{scope}/_apis/distributedtask/hubs/{hub}/plans/{plan}";
        // Where the runner was told the service is, which everything but registering itself
        // hangs off: it is told a url and asks under it, whatever is in front of the path.
        let under = self
            .base
            .split_once("://")
            .and_then(|(_, rest)| rest.split_once('/'))
            .map(|(_, path)| format!("/{}", path.trim_end_matches('/')))
            .filter(|path| path.len() > 1);
        let state = Arc::new(self);

        let served = Router::new()
            .route("/_apis/connectionData", get(connection::<J>))
            .route("/_apis/", any(locations))
            .route("/_apis/oauth2/token", post(granted::<J>))
            .route(pools, get(pools_of::<J>))
            .route(
                &format!("{pools}/{{pool}}/agents"),
                get(registered_already).post(registering::<J>),
            )
            .route(&format!("{pools}/{{pool}}/sessions"), post(session::<J>))
            .route(
                &format!("{pools}/{{pool}}/sessions/{{session}}"),
                delete(closed),
            )
            .route(&format!("{pools}/{{pool}}/messages"), get(message::<J>))
            .route(
                &format!("{pools}/{{pool}}/messages/{{message}}"),
                delete(read::<J>),
            )
            .route(
                &format!("{pools}/{{pool}}/jobrequests/{{request}}"),
                any(holding::<J>),
            )
            .route(&format!("{plans}/timelines/{{timeline}}"), any(said::<J>))
            .route(
                &format!("{plans}/timelines/{{timeline}}/records"),
                any(said::<J>),
            )
            .route(&format!("{plans}/logs"), post(opened::<J>))
            .route(&format!("{plans}/logs/{{log}}"), post(uploaded::<J>))
            .route(&format!("{plans}/events"), post(ended::<J>))
            .route(&format!("{plans}/actiondownloadinfo"), post(downloads::<J>));

        // Registering is the one call a runner makes before it is told where anything is,
        // and it makes that one against the host itself.
        let router = match under {
            None => served,
            Some(under) => Router::new().nest(&under, served),
        };

        router
            .route("/api/v3/actions/runner-registration", post(registered::<J>))
            .fallback(any(unknown))
            .with_state(state)
    }
}

type Serving<J> = State<Arc<JobServer<J>>>;

/// What a poll for work says about who is asking.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Polled {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    last_message_id: Option<i64>,
}

/// The parts of the path a call under a plan is addressed by.
#[derive(serde::Deserialize)]
struct Under {
    scope: String,
    hub: String,
    plan: String,
    timeline: String,
}

impl Under {
    fn split(self) -> (Plan, String) {
        let plan = Plan {
            scope: self.scope,
            hub: self.hub,
            id: self.plan,
        };

        (plan, self.timeline)
    }
}

async fn registered<J: Jobs>(State(server): Serving<J>) -> impl IntoResponse {
    axum::Json(Tenant {
        url: server.base.trim_end_matches('/').to_owned(),
        token: server.jobs.credential(),
        token_schema: "OAuthAccessToken".to_owned(),
    })
}

async fn connection<J: Jobs>(State(server): Serving<J>) -> impl IntoResponse {
    axum::Json(ConnectionData::at(&server.base))
}

async fn locations() -> impl IntoResponse {
    axum::Json(Many::of(known()))
}

async fn granted<J: Jobs>(State(server): Serving<J>) -> impl IntoResponse {
    axum::Json(Granted {
        access_token: server.jobs.credential(),
        token_type: "bearer".to_owned(),
        expires_in: 3600,
    })
}

async fn pools_of<J: Jobs>(State(server): Serving<J>) -> impl IntoResponse {
    axum::Json(Many::of(vec![server.jobs.pool()]))
}

async fn registered_already() -> impl IntoResponse {
    axum::Json(Many::of(Vec::<Agent>::new()))
}

async fn registering<J: Jobs>(State(server): Serving<J>) -> impl IntoResponse {
    axum::Json(server.jobs.agent(&server.base))
}

async fn session<J: Jobs>(State(server): Serving<J>) -> impl IntoResponse {
    axum::Json(server.jobs.session())
}

/// A poll answered with a body reads as a message, so having nothing has to be a status.
async fn message<J: Jobs>(
    State(server): Serving<J>,
    Path(pool): Path<i64>,
    Query(asked): Query<Polled>,
) -> Response {
    let asking = Asking {
        pool,
        session: asked.session_id,
        last: asked.last_message_id,
    };

    match server.jobs.take(&asking) {
        None => StatusCode::NO_CONTENT.into_response(),
        Some(message) => axum::Json(Envelope {
            message_id: Some(message.id),
            message_type: "PipelineAgentJobRequest".to_owned(),
            body: message.body,
            iv: None,
        })
        .into_response(),
    }
}

/// A runner letting go of the session it registered, which it does on its way out.
async fn closed() -> impl IntoResponse {
    StatusCode::OK
}

async fn read<J: Jobs>(
    State(server): Serving<J>,
    Path((pool, message)): Path<(i64, i64)>,
) -> impl IntoResponse {
    server.jobs.read(pool, message);
    StatusCode::OK
}

async fn holding<J: Jobs>(
    State(server): Serving<J>,
    Path((_, request)): Path<(i64, i64)>,
) -> impl IntoResponse {
    axum::Json(server.jobs.holding(request))
}

async fn said<J: Jobs>(
    State(server): Serving<J>,
    Path(under): Path<Under>,
    body: String,
) -> impl IntoResponse {
    let (plan, timeline) = under.split();
    tracing::trace!(bytes = body.len(), "the runner says what happened");

    // The same route carries both what happened and what was printed while it happened.
    if let Ok(lines) = serde_json::from_str::<Lines>(&body)
        && !lines.value.is_empty()
    {
        server.jobs.printed(&plan, &timeline, lines);
        return axum::Json(Many::of(Vec::<Record>::new()));
    }

    match serde_json::from_str::<Many<Record>>(&body) {
        Ok(update) => server.jobs.records(&plan, &timeline, update.value),
        Err(err) => tracing::warn!(%err, "a timeline this service cannot read"),
    }

    axum::Json(Many::of(Vec::<Record>::new()))
}

async fn opened<J: Jobs>(
    State(server): Serving<J>,
    axum::Json(asked): axum::Json<TaskLog>,
) -> impl IntoResponse {
    let mut logs = server.logs.lock().expect("the logs");
    logs.push(asked.path.clone());

    axum::Json(kept(logs.len() as i64, &asked.path))
}

async fn uploaded<J: Jobs>(
    State(server): Serving<J>,
    Path((scope, hub, plan, log)): Path<(String, String, String, i64)>,
    body: String,
) -> impl IntoResponse {
    let path = server
        .logs
        .lock()
        .expect("the logs")
        .get(log as usize - 1)
        .cloned()
        .unwrap_or_default();

    if !body.is_empty() {
        let step = path.rsplit(['\\', '/']).next().unwrap_or_default();
        server
            .jobs
            .log(&Plan { scope, hub, id: plan }, log, step, body);
    }

    axum::Json(kept(log, &path))
}

async fn ended<J: Jobs>(
    State(server): Serving<J>,
    Path((scope, hub, plan)): Path<(String, String, String)>,
    axum::Json(ended): axum::Json<JobEnded>,
) -> impl IntoResponse {
    server.jobs.ended(&Plan { scope, hub, id: plan }, ended);
    StatusCode::OK
}

async fn downloads<J: Jobs>(
    State(server): Serving<J>,
    axum::Json(asked): axum::Json<ActionReferences>,
) -> impl IntoResponse {
    let actions = asked
        .actions
        .iter()
        .map(|action| (action.key(), server.jobs.download(action)))
        .collect();

    axum::Json(ActionDownloads { actions })
}

async fn unknown(method: axum::http::Method, uri: axum::http::Uri) -> impl IntoResponse {
    tracing::warn!(%method, %uri, "asked for something this service does not answer");
    StatusCode::OK
}

fn kept(id: i64, path: &str) -> TaskLog {
    TaskLog {
        id,
        path: path.to_owned(),
        line_count: 0,
        created_on: "2020-01-01T00:00:00Z".to_owned(),
        last_changed_on: "2020-01-01T00:00:00Z".to_owned(),
    }
}

/// Where each call lives. A runner carries these ids, asks for the ones it wants, and calls
/// nothing it was not given a route for.
fn known() -> Vec<ResourceLocation> {
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
