//! Stand-ins for the GitHub-hosted services actions expect to find.

pub mod artifacts_server;
pub mod cache_server;
pub mod local;
pub mod store;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use artifacts_server::ArtifactServer;
use cache_server::CacheServer;
use local::LocalArtifacts;
use store::LocalCache;

pub struct Services {
    address: std::net::SocketAddr,
    root: PathBuf,
    /// Dropping it is what shuts the runtime down.
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Services {
    /// The rest of the runner is synchronous, so the server gets a thread of its own.
    pub fn start(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        let artifacts = LocalArtifacts::open(root.join("artifacts"))?;
        let caches = LocalCache::open(root.join("caches"))?;
        let (ready, wait) = mpsc::channel();
        let (shutdown, stop) = tokio::sync::oneshot::channel();

        let thread = std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = ready.send(Err(err));
                    return;
                }
            };

            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
                    Ok(listener) => listener,
                    Err(err) => {
                        let _ = ready.send(Err(err));
                        return;
                    }
                };
                let address = match listener.local_addr() {
                    Ok(address) => address,
                    Err(err) => {
                        let _ = ready.send(Err(err));
                        return;
                    }
                };

                // Known before the routes are built, since both hand out absolute URLs.
                let base_url: Arc<str> = format!("http://{address}").into();
                let app = ArtifactServer::new(artifacts, base_url.clone())
                    .routes()
                    .merge(CacheServer::new(caches, base_url).routes());

                let _ = ready.send(Ok(address));
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = stop.await;
                    })
                    .await;
            });
        });

        let address = wait
            .recv()
            .map_err(|_| std::io::Error::other("services thread died"))??;

        Ok(Self {
            address,
            root,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    pub fn env(&self) -> BTreeMap<String, String> {
        let base = format!("http://{}/", self.address);

        BTreeMap::from([
            ("ACTIONS_RUNTIME_TOKEN".to_owned(), runtime_token()),
            ("ACTIONS_RESULTS_URL".to_owned(), base.clone()),
            ("ACTIONS_RUNTIME_URL".to_owned(), base.clone()),
            ("ACTIONS_CACHE_URL".to_owned(), base),
        ])
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        drop(self.shutdown.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The client never checks the signature, but it does read ids out of `scp`.
fn runtime_token() -> String {
    #[derive(serde::Serialize)]
    struct Claims {
        scp: String,
        iss: String,
    }

    let claims = Claims {
        scp: format!(
            "Actions.Results:{}:{}",
            "00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002"
        ),
        iss: "local-actions-services".to_owned(),
    };

    // Signed with a fixed secret, since nothing on either side ever verifies it.
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(b"local-actions-services"),
    )
    .expect("claims are serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_carries_the_ids_the_client_reads() {
        let token = runtime_token();
        let payload = token.split('.').nth(1).expect("three parts");

        let decoded = decode_base64url(payload);
        let text = String::from_utf8(decoded).expect("utf8 claims");
        let scope = text
            .split('"')
            .find(|part| part.starts_with("Actions.Results:"))
            .expect("an Actions.Results scope");
        let parts: Vec<&str> = scope.split(':').collect();

        assert_eq!(parts.len(), 3, "run and job ids must both be present");
        assert!(!parts[1].is_empty() && !parts[2].is_empty());
    }

    #[test]
    fn services_start_and_report_their_env() {
        let directory = std::env::temp_dir().join("local-actions-services-test");
        let services = Services::start(&directory).expect("services start");
        let env = services.env();

        assert!(env["ACTIONS_RESULTS_URL"].starts_with("http://127.0.0.1:"));
        assert_eq!(env["ACTIONS_RESULTS_URL"], env["ACTIONS_CACHE_URL"]);
        assert!(env["ACTIONS_RUNTIME_TOKEN"].split('.').count() == 3);
    }

    fn decode_base64url(text: &str) -> Vec<u8> {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(text)
            .expect("payload is base64url")
    }
}
