//! Server implementation of the arficacts service

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{post, put};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub size: u64,
    pub id: i64,
}

pub trait Artifacts: Send + Sync + 'static {
    fn create(&self, name: &str) -> Artifact;

    fn finalize(&self, name: &str, size: u64) -> Option<Artifact>;

    fn get(&self, name: &str) -> Option<Artifact>;

    fn list(&self, name: Option<&str>) -> Vec<Artifact>;

    fn store(&self, name: &str, bytes: &[u8]) -> std::io::Result<()>;

    fn load(&self, name: &str) -> std::io::Result<Vec<u8>>;
}

pub struct ArtifactServer<A: Artifacts> {
    artifacts: A,
    base_url: Arc<str>,
    /// The protocol might send the artifacts as multiple blobs of data.
    /// Store them in memory before handing them over to Arifacts::store
    staged: Mutex<BTreeMap<String, BTreeMap<String, Vec<u8>>>>,
}

impl<A: Artifacts> ArtifactServer<A> {
    pub fn new(artifacts: A, base_url: impl Into<Arc<str>>) -> Self {
        Self {
            artifacts,
            base_url: base_url.into(),
            staged: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn routes(self) -> Router {
        let state = Arc::new(self);

        Router::new()
            .route(
                "/twirp/github.actions.results.api.v1.ArtifactService/CreateArtifact",
                post(create_artifact::<A>),
            )
            .route(
                "/twirp/github.actions.results.api.v1.ArtifactService/FinalizeArtifact",
                post(finalize_artifact::<A>),
            )
            .route(
                "/twirp/github.actions.results.api.v1.ArtifactService/ListArtifacts",
                post(list_artifacts::<A>),
            )
            .route(
                "/twirp/github.actions.results.api.v1.ArtifactService/GetSignedArtifactURL",
                post(signed_url::<A>),
            )
            .route(
                "/blob/{name}",
                put(upload_blob::<A>).get(download_blob::<A>),
            )
            .with_state(state)
    }

    fn stage(&self, artifact: &str, block: &str, bytes: Vec<u8>) {
        self.staged
            .lock()
            .expect("staged blocks")
            .entry(artifact.to_owned())
            .or_default()
            .insert(block.to_owned(), bytes);
    }

    fn assemble(&self, artifact: &str, order: &[String]) -> Vec<u8> {
        let Some(blocks) = self.staged.lock().expect("staged blocks").remove(artifact) else {
            return Vec::new();
        };

        order
            .iter()
            .filter_map(|id| blocks.get(id).cloned())
            .flatten()
            .collect()
    }
}

type Server<A> = State<Arc<ArtifactServer<A>>>;

async fn create_artifact<A: Artifacts>(
    State(server): Server<A>,
    axum::Json(request): axum::Json<Value>,
) -> impl IntoResponse {
    let name = request["name"].as_str().unwrap_or("artifact").to_owned();
    server.artifacts.create(&name);

    axum::Json(json!({
        "ok": true,
        "signedUploadUrl": format!("{}/blob/{name}?sig=local", server.base_url),
    }))
}

async fn finalize_artifact<A: Artifacts>(
    State(server): Server<A>,
    axum::Json(request): axum::Json<Value>,
) -> impl IntoResponse {
    let name = request["name"].as_str().unwrap_or_default();

    match server.artifacts.finalize(name, size_of(&request["size"])) {
        Some(artifact) => {
            axum::Json(json!({ "ok": true, "artifactId": artifact.id.to_string() })).into_response()
        }
        None => twirp_error(StatusCode::NOT_FOUND, "not_found", "no such artifact"),
    }
}

async fn list_artifacts<A: Artifacts>(
    State(server): Server<A>,
    axum::Json(request): axum::Json<Value>,
) -> impl IntoResponse {
    let run = request["workflowRunBackendId"].as_str().unwrap_or_default();
    let job = request["workflowJobRunBackendId"]
        .as_str()
        .unwrap_or_default();
    let named = asked(&request, ["nameFilter", "name_filter"]);
    let only = asked(&request, ["idFilter", "id_filter"]);

    let artifacts: Vec<Value> = server
        .artifacts
        .list(named.as_deref())
        .into_iter()
        .filter(|artifact| {
            only.as_ref()
                .is_none_or(|id| *id == artifact.id.to_string())
        })
        .map(|artifact| {
            json!({
                "workflowRunBackendId": run,
                "workflowJobRunBackendId": job,
                "databaseId": artifact.id.to_string(),
                "name": artifact.name,
                "size": artifact.size.to_string(),
            })
        })
        .collect();

    axum::Json(json!({ "artifacts": artifacts }))
}

fn asked(request: &Value, names: [&str; 2]) -> Option<String> {
    names.into_iter().find_map(|key| {
        request[key]
            .as_str()
            .map(str::to_owned)
            .or_else(|| request[key]["value"].as_str().map(str::to_owned))
            .or_else(|| request[key].as_i64().map(|id| id.to_string()))
    })
}

async fn signed_url<A: Artifacts>(
    State(server): Server<A>,
    axum::Json(request): axum::Json<Value>,
) -> impl IntoResponse {
    let name = request["name"].as_str().unwrap_or_default();

    match server.artifacts.get(name) {
        Some(artifact) => axum::Json(json!({
            "signedUrl": format!("{}/blob/{}?sig=local", server.base_url, artifact.name),
        }))
        .into_response(),
        None => twirp_error(StatusCode::NOT_FOUND, "not_found", "no such artifact"),
    }
}

async fn upload_blob<A: Artifacts>(
    State(server): Server<A>,
    Path(name): Path<String>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    match query.get("comp").map(String::as_str) {
        // One block of a larger upload, kept until the block list arrives.
        Some("block") => {
            let block = query.get("blockid").cloned().unwrap_or_default();
            server.stage(&name, &block, body.to_vec());
        }
        // The list naming the blocks, in order, that make up the finished blob.
        Some("blocklist") => {
            let order = block_ids(&String::from_utf8_lossy(&body));
            let assembled = server.assemble(&name, &order);
            if let Err(err) = server.artifacts.store(&name, &assembled) {
                return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
            }
        }
        // A small blob sent in a single request.
        _ if headers.contains_key("x-ms-blob-type") || !body.is_empty() => {
            if let Err(err) = server.artifacts.store(&name, &body) {
                return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
            }
        }
        _ => {}
    }

    StatusCode::CREATED.into_response()
}

async fn download_blob<A: Artifacts>(
    State(server): Server<A>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match server.artifacts.load(&name) {
        Ok(bytes) => (StatusCode::OK, [("content-type", "application/zip")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// proto3 JSON sends one as a string once it no longer fits in 32 bits.
fn size_of(value: &Value) -> u64 {
    value
        .as_str()
        .and_then(|size| size.parse().ok())
        .or_else(|| value.as_u64())
        .unwrap_or_default()
}

fn block_ids(xml: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = xml;

    // Entries look like `<Latest>base64</Latest>`, and may also be Committed or Uncommitted.
    while let Some(start) = rest.find('>') {
        let Some(end) = rest[start + 1..].find('<') else {
            break;
        };
        let candidate = rest[start + 1..start + 1 + end].trim();
        if !candidate.is_empty() {
            ids.push(candidate.to_owned());
        }
        rest = &rest[start + 1 + end + 1..];
    }

    ids
}

fn twirp_error(status: StatusCode, code: &str, message: &str) -> axum::response::Response {
    (status, axum::Json(json!({ "code": code, "msg": message }))).into_response()
}

#[cfg(test)]
pub fn conformance(artifacts: Box<dyn Artifacts>) {
    let created = artifacts.create("build-output");
    artifacts
        .store("build-output", b"a zip, in spirit")
        .unwrap();

    assert_eq!(
        artifacts.finalize("build-output", 16).map(|found| found.id),
        Some(created.id),
        "finalizing returns the artifact that was created"
    );

    let stored = artifacts.get("build-output").expect("it was created");
    assert_eq!(stored.size, 16, "the finalized size is kept");
    assert_eq!(artifacts.load("build-output").unwrap(), b"a zip, in spirit");

    artifacts.create("notes");
    assert_eq!(artifacts.list(None).len(), 2, "both are listed");
    assert_eq!(artifacts.list(Some("notes")).len(), 1, "listing filters");
    assert!(artifacts.list(Some("missing")).is_empty());

    assert!(artifacts.get("missing").is_none());
    assert!(
        artifacts.finalize("missing", 1).is_none(),
        "finalizing something that was never created reports nothing"
    );
}
