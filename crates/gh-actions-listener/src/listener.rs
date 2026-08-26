//! The poll loop: ask for work, decrypt it, hand it over, say what happened.

use std::time::{Duration, Instant};

use base64::Engine as _;
use gh_actions_context::Runner;
use tracing::{debug, error, info, warn};

use crate::auth::Credentials;
use crate::client::crypto;
use crate::client::types::{Envelope, JobMessage, Outcome};
use crate::client::{Client, Session, Token};
use crate::error::Error;
use crate::progress::Progress;

pub use crate::client::RUNNER_VERSION;

/// The protocol knows how to be handed a job, how to say what is happening while it runs,
/// and how to say what became of it. What running one means is the worker's business.
pub trait Worker {
    fn run(&mut self, job: &JobMessage, progress: &mut Progress) -> Result<Outcome, Error>;
}

const SESSION_ATTEMPTS: u32 = 30;

const SESSION_RETRY: Duration = Duration::from_secs(10);

pub struct Listener<W: Worker> {
    credentials: Credentials,
    worker: W,
    client: Client,
    token: Token,
    issued: Instant,
    broker: Option<String>,
    /// The session the broker keeps, which is not the one the service opened.
    broker_session: Option<Session>,
}

impl<W: Worker> Listener<W> {
    pub fn connect(credentials: Credentials, worker: W) -> Result<Self, Error> {
        let mut client = Client::new()?;
        let token = client.token(&credentials)?;
        client.bearer(&token.value);

        info!(
            agent = credentials.agent_id,
            pool = credentials.pool_id,
            service = %credentials.service_url,
            "authenticated"
        );

        Ok(Self {
            credentials,
            worker,
            client,
            token,
            issued: Instant::now(),
            broker: None,
            broker_session: None,
        })
    }

    pub fn agent(&self) -> Result<serde_json::Value, Error> {
        self.client.agent(&self.credentials)
    }

    /// A runner may only have one at a time, and one left behind by a run that was killed
    /// takes a little while to be given up, so a conflict is waited out rather than raised.
    pub fn open_session(&mut self) -> Result<Session, Error> {
        let name = self.agent_name();
        let mut attempt = 0;

        loop {
            match self.client.create_session(&self.credentials, name) {
                Err(Error::Status { status: 409, .. }) if attempt < SESSION_ATTEMPTS => {
                    attempt += 1;
                    warn!(
                        attempt,
                        "another session is still open; waiting for it to lapse"
                    );
                    std::thread::sleep(SESSION_RETRY);
                }
                Err(err) => return Err(err),
                Ok(session) => {
                    info!(
                        session = %session.session_id,
                        encrypted = session.encryption_key.is_some(),
                        "session opened"
                    );
                    return Ok(session);
                }
            }
        }
    }

    /// Older credentials predate the name being kept, hence the fallback.
    fn agent_name(&self) -> &str {
        match self.credentials.agent_name.as_str() {
            "" => "canopy",
            name => name,
        }
    }

    pub fn close_session(&mut self, session_id: &str) -> Result<(), Error> {
        self.client.delete_session(&self.credentials, session_id)
    }

    /// One message at a time, acknowledged only once the job is done, so a crash leaves the
    /// job to be handed out again rather than silently dropped.
    pub fn listen(&mut self, session: &Session) -> Result<(), Error> {
        let key = self.session_key(session)?;
        let mut last_message: Option<i64> = None;

        loop {
            let Some(envelope) = self.poll(session, last_message)? else {
                continue;
            };

            // Being sent to the broker is not work; the next poll goes there instead.
            if let Some(broker) = envelope.broker_url() {
                info!(%broker, "handed over to the broker");
                let opened = self.client.create_broker_session(
                    &broker,
                    &self.credentials,
                    self.agent_name(),
                );
                match opened {
                    Ok(session) => {
                        info!(session = %session.session_id, "broker session opened");
                        self.broker_session = Some(session);
                    }
                    Err(err) => warn!(%err, "the broker kept no session of its own"),
                }
                self.broker = Some(broker);
                continue;
            }
            last_message = envelope.message_id;

            // The old protocol sends the job; the broker sends where to collect it from.
            let job = match envelope.job_offer() {
                // A job that cannot be collected is one job lost, not a reason to stop
                // listening: the next one may be perfectly fine.
                Some(offer) => match self.client.acquire_job(&offer) {
                    Ok(job) => Some((job, Some(offer))),
                    Err(err) => {
                        error!(%err, request = %offer.runner_request_id, "cannot collect the job");
                        None
                    }
                },
                None if envelope.is_job() => {
                    Some((self.decode_job(&envelope, key.as_deref())?, None))
                }
                None => {
                    debug!(
                        kind = %envelope.message_type,
                        body = %envelope.body,
                        "a message that is not a job"
                    );
                    None
                }
            };

            if let Some((job, offer)) = job {
                info!(job = %job.job_display_name, steps = job.steps.len(), "running a job");

                let mut progress = Progress::open(&job)?;
                let outcome = self.worker.run(&job, &mut progress)?;
                progress.finish(offer.as_ref(), outcome);

                info!(job = %job.job_display_name, outcome = outcome.name(), "job finished");
            }

            if let Some(message) = envelope.message_id {
                self.client
                    .acknowledge(&self.credentials, &session.session_id, message)?;
            }
        }
    }

    fn session_key(&self, session: &Session) -> Result<Option<Vec<u8>>, Error> {
        let Some(key) = &session.encryption_key else {
            return Ok(None);
        };

        let raw = base64::engine::general_purpose::STANDARD
            .decode(&key.value)
            .map_err(|err| Error::Crypto(format!("session key is not base64: {err}")))?;
        if !key.encrypted {
            return Ok(Some(raw));
        }

        let private = crypto::read_key(&self.credentials.private_key)?;
        Ok(Some(crypto::unwrap_session_key(
            &private,
            &raw,
            session.use_fips_encryption,
        )?))
    }

    fn poll(&mut self, session: &Session, last: Option<i64>) -> Result<Option<Envelope>, Error> {
        self.refresh_token_if_stale()?;

        match &self.broker {
            Some(broker) => {
                let id = match &self.broker_session {
                    Some(broker_session) => &broker_session.session_id,
                    None => &session.session_id,
                };
                self.client
                    .broker_message(broker, id, Runner::host_os(), Runner::host_arch())
            }
            None => self
                .client
                .messages(&self.credentials, &session.session_id, last),
        }
    }

    fn decode_job(&self, envelope: &Envelope, key: Option<&[u8]>) -> Result<JobMessage, Error> {
        let Some(key) = key else {
            return JobMessage::decode(&envelope.body);
        };

        let iv = envelope
            .iv
            .as_deref()
            .ok_or_else(|| Error::Protocol("an encrypted message with no iv".to_owned()))?;
        let iv = base64::engine::general_purpose::STANDARD
            .decode(iv)
            .map_err(|err| Error::Crypto(format!("iv is not base64: {err}")))?;
        let body = base64::engine::general_purpose::STANDARD
            .decode(&envelope.body)
            .map_err(|err| Error::Crypto(format!("body is not base64: {err}")))?;

        JobMessage::decode(&crypto::decrypt_message(key, &iv, &body)?)
    }

    fn refresh_token_if_stale(&mut self) -> Result<(), Error> {
        let lifetime = Duration::from_secs(self.token.expires_in);
        if self.issued.elapsed() + Duration::from_secs(300) < lifetime {
            return Ok(());
        }

        self.token = self.client.token(&self.credentials)?;
        self.client.bearer(&self.token.value);
        self.issued = Instant::now();
        Ok(())
    }
}
