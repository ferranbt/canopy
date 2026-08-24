//! What the runner talks to: the listener's server, with the one job there is to hand over.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use gh_actions_listener::client::types::{JobEnded, Lines, Record};
use gh_actions_listener::server::{Asking, JobServer, Jobs, Message, Plan};
use tracing::debug;

/// Where the runner is told the service is.
///
/// Port 80: it rebuilds the url it was given as `{scheme}://{host}`, so whatever port it
/// was told about is lost before it asks for anything.
pub const BASE: &str = "http://127.0.0.1/canopy/tests/";

pub enum Update {
    Records(Vec<Record>),
    /// A step's whole log, uploaded once it is over rather than as it goes.
    Log {
        step: String,
        text: String,
    },
    /// How the job went and what it came out with, said once it is over.
    Ended {
        result: String,
        outputs: BTreeMap<String, String>,
    },
}

#[derive(Clone, Default)]
pub struct Service {
    /// The one job there is to hand over, taken by the runner that asks for it.
    job: Arc<Mutex<Option<String>>>,
    /// Where what the runner says goes, while it is still saying it.
    updates: Arc<Mutex<Option<Sender<Update>>>>,
}

impl Service {
    pub fn hand_over(&self, job: String, updates: Sender<Update>) {
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

                let app = JobServer::new(serving, BASE).routes();
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

impl Jobs for Service {
    fn take(&self, _asking: &Asking) -> Option<Message> {
        let taken = self.job.lock().expect("the job").take()?;
        debug!("handing the job over");

        Some(Message { id: 1, body: taken })
    }

    fn records(&self, _plan: &Plan, _timeline: &str, records: Vec<Record>) {
        self.send(Update::Records(records));
    }

    /// Nothing: what a step says as it goes is the same as what it uploads when it stops,
    /// and the upload is the whole of it.
    fn printed(&self, _plan: &Plan, _timeline: &str, _lines: Lines) {}

    fn log(&self, _plan: &Plan, _log: i64, step: &str, text: String) {
        self.send(Update::Log {
            step: step.to_owned(),
            text,
        });
    }

    fn ended(&self, _plan: &Plan, ended: JobEnded) {
        let outputs: BTreeMap<String, String> = ended
            .outputs
            .into_iter()
            .map(|(name, said)| (name, said.value))
            .collect();

        debug!(
            result = ended.result,
            ?outputs,
            "the runner says how it went"
        );
        self.send(Update::Ended {
            result: ended.result,
            outputs,
        });
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
