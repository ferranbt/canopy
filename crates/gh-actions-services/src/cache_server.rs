//! Server implementtion of the cache service (`actions/cache`)

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub version: String,
    pub id: i64,
    pub committed: bool,
}

pub trait Cache: Send + Sync + 'static {
    fn reserve(&self, key: &str, version: &str) -> Option<i64>;

    fn commit(&self, id: i64) -> bool;

    fn lookup(&self, keys: &[String], version: &str) -> Option<CacheEntry>;

    fn write(&self, id: i64, offset: u64, bytes: &[u8]) -> std::io::Result<()>;

    fn read(&self, id: i64) -> std::io::Result<Vec<u8>>;
}

pub struct CacheServer<C: Cache> {
    cache: C,
    base_url: Arc<str>,
}

impl<C: Cache> CacheServer<C> {
    pub fn new(cache: C, base_url: impl Into<Arc<str>>) -> Self {
        Self {
            cache,
            base_url: base_url.into(),
        }
    }

    pub fn routes(self) -> Router {
        let state = Arc::new(self);

        Router::new()
            .route("/_apis/artifactcache/cache", get(lookup::<C>))
            .route("/_apis/artifactcache/caches", post(reserve::<C>))
            .route(
                "/_apis/artifactcache/caches/{id}",
                post(commit::<C>).patch(upload::<C>),
            )
            .route("/cache/{id}", get(download::<C>))
            .with_state(state)
    }
}

type Server<C> = State<Arc<CacheServer<C>>>;

async fn lookup<C: Cache>(
    State(server): Server<C>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let keys: Vec<String> = query
        .get("keys")
        .map(|keys| keys.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let version = query.get("version").cloned().unwrap_or_default();

    match server.cache.lookup(&keys, &version) {
        // Answer 204 on a miss
        None => StatusCode::NO_CONTENT.into_response(),
        Some(entry) => axum::Json(json!({
            "cacheKey": entry.key,
            "scope": "local",
            "archiveLocation": format!("{}/cache/{}", server.base_url, entry.id),
        }))
        .into_response(),
    }
}

async fn reserve<C: Cache>(
    State(server): Server<C>,
    axum::Json(request): axum::Json<Value>,
) -> impl IntoResponse {
    let key = request["key"].as_str().unwrap_or_default();
    let version = request["version"].as_str().unwrap_or_default();

    match server.cache.reserve(key, version) {
        Some(id) => axum::Json(json!({ "cacheId": id })).into_response(),
        // The client treats a conflict as "already cached", which is not an error.
        None => (
            StatusCode::CONFLICT,
            axum::Json(json!({ "message": "already exists" })),
        )
            .into_response(),
    }
}

async fn upload<C: Cache>(
    State(server): Server<C>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let start = headers
        .get("content-range")
        .and_then(|range| range.to_str().ok())
        .and_then(parse_range_start)
        .unwrap_or(0);

    if let Err(err) = server.cache.write(id, start, &body) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn commit<C: Cache>(State(server): Server<C>, Path(id): Path<i64>) -> impl IntoResponse {
    if server.cache.commit(id) {
        return StatusCode::NO_CONTENT.into_response();
    }
    (StatusCode::NOT_FOUND, "no such cache entry").into_response()
}

async fn download<C: Cache>(State(server): Server<C>, Path(id): Path<i64>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/octet-stream")],
        server.cache.read(id).unwrap_or_default(),
    )
}

/// The first offset of a `bytes start-end/total` header.
fn parse_range_start(range: &str) -> Option<u64> {
    range
        .trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// What any [`Cache`] has to do, so every implementation is held to one test.
#[cfg(test)]
pub fn conformance(cache: Box<dyn Cache>) {
    let id = cache
        .reserve("deps-abc", "v1")
        .expect("a fresh key reserves");
    cache.write(id, 0, b"hello").unwrap();
    cache.write(id, 5, b" world").unwrap();

    assert!(
        cache.lookup(&["deps-abc".to_owned()], "v1").is_none(),
        "an uncommitted entry is not a hit"
    );
    assert!(cache.commit(id));
    assert!(
        !cache.commit(id + 1000),
        "committing what is not there fails"
    );

    let hit = cache
        .lookup(&["deps-abc".to_owned()], "v1")
        .expect("a committed entry is a hit");
    assert_eq!(hit.id, id);
    assert_eq!(
        cache.read(id).unwrap(),
        b"hello world",
        "ranges land in order"
    );

    assert!(
        cache.lookup(&["deps-abc".to_owned()], "v2").is_none(),
        "a different version never matches"
    );
    assert!(
        cache.reserve("deps-abc", "v1").is_none(),
        "a key that is already cached cannot be reserved again"
    );

    let second = cache.reserve("deps-xyz", "v1").expect("another key");
    cache.commit(second);

    let keys = ["deps-xyz".to_owned(), "deps-".to_owned()];
    assert_eq!(
        cache.lookup(&keys, "v1").expect("a hit").key,
        "deps-xyz",
        "the first key matches exactly before any prefix is tried"
    );

    let keys = ["deps-nothing".to_owned(), "deps-".to_owned()];
    assert!(
        cache
            .lookup(&keys, "v1")
            .expect("a prefix hit")
            .key
            .starts_with("deps-"),
        "the keys after the first match by prefix"
    );
}
