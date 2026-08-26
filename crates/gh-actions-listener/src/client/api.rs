//! One method per call the runner makes, in the order a runner makes them.

use reqwest::Method;
use serde::Serialize;
use tracing::debug;

use gh_actions_context::Runner;

use crate::auth::Credentials;
use crate::client::types::*;
use crate::client::{Client, decode, shown};
use crate::error::Error;

impl Client {
    /// A form rather than JSON, and answered with a secret, so only its outcome is logged.
    pub fn token(&self, credentials: &Credentials) -> Result<Token, Error> {
        let signed = crate::client::assertion::sign(credentials)?;
        debug!(url = %credentials.token_url, "→ POST (token)");

        let response = self
            .http
            .post(&credentials.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                (
                    "client_assertion_type",
                    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                ),
                ("client_assertion", &signed),
            ])
            .send()?;

        let status = response.status();
        let body = response.text()?;
        debug!(status = status.as_u16(), "← token");

        if !status.is_success() {
            return Err(Error::Status {
                status: status.as_u16(),
                body,
            });
        }

        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|err| Error::Protocol(format!("{err}")))?;
        let value = parsed["access_token"]
            .as_str()
            .ok_or_else(|| Error::Protocol(format!("no access_token, in: {}", shown(&body))))?;

        Ok(Token {
            value: value.to_owned(),
            expires_in: parsed["expires_in"].as_u64().unwrap_or(3000),
        })
    }

    pub fn tenant(&self, url: &str) -> Result<Tenant, Error> {
        let body = TenantRequest {
            url,
            runner_event: "register",
        };

        self.call(Method::POST, &registration_endpoint(url), Some(&body))
    }

    /// The pool a runner joins, which for a repository is always the default one.
    pub fn default_pool(&self, service: &str) -> Result<i64, Error> {
        let url = format!(
            "{}/_apis/distributedtask/pools?poolName=Default&api-version=6.0-preview.1",
            service.trim_end_matches('/')
        );

        let pools: Pools = self.call(Method::GET, &url, NO_BODY)?;
        pools
            .value
            .first()
            .map(|pool| pool.id)
            .ok_or_else(|| Error::Protocol("there is no Default pool to join".to_owned()))
    }

    pub fn create_agent(
        &self,
        service: &str,
        pool: i64,
        agent: &AgentRequest,
    ) -> Result<Agent, Error> {
        let url = format!(
            "{}/_apis/distributedtask/pools/{pool}/agents?api-version=6.0-preview.2",
            service.trim_end_matches('/')
        );

        self.call(Method::POST, &url, Some(agent))
    }

    pub fn agent(&self, credentials: &Credentials) -> Result<serde_json::Value, Error> {
        let url = format!(
            "{}/_apis/distributedtask/pools/{}/agents/{}?api-version=6.0-preview.2",
            credentials.service_url.trim_end_matches('/'),
            credentials.pool_id,
            credentials.agent_id,
        );

        self.call(Method::GET, &url, NO_BODY)
    }

    pub fn create_session(&self, credentials: &Credentials, name: &str) -> Result<Session, Error> {
        let url = format!(
            "{}/_apis/distributedtask/pools/{}/sessions?api-version=5.1-preview",
            credentials.service_url.trim_end_matches('/'),
            credentials.pool_id,
        );
        let body = SessionRequest {
            agent: AgentReference {
                id: credentials.agent_id,
                name,
            },
            owner_name: name,
        };

        self.call(Method::POST, &url, Some(&body))
    }

    pub fn delete_session(&self, credentials: &Credentials, session: &str) -> Result<(), Error> {
        let url = format!(
            "{}/_apis/distributedtask/pools/{}/sessions/{session}?api-version=5.1-preview",
            credentials.service_url.trim_end_matches('/'),
            credentials.pool_id,
        );

        self.nothing(Method::DELETE, &url)
    }

    /// Long-polls, so it may time out with nothing.
    pub fn messages(
        &self,
        credentials: &Credentials,
        session: &str,
        last: Option<i64>,
    ) -> Result<Option<Envelope>, Error> {
        let mut url = format!(
            "{}/_apis/distributedtask/pools/{}/messages?api-version=5.1-preview&sessionId={session}&runnerVersion={RUNNER_VERSION}",
            credentials.service_url.trim_end_matches('/'),
            credentials.pool_id,
        );
        if let Some(last) = last {
            url.push_str(&format!("&lastMessageId={last}"));
        }

        self.maybe(&url)
    }

    /// The broker keeps its own session, which is what a job is assigned against.
    pub fn create_broker_session(
        &self,
        broker: &str,
        credentials: &Credentials,
        name: &str,
    ) -> Result<Session, Error> {
        let url = format!("{}/session", broker.trim_end_matches('/'));
        let body = BrokerSessionRequest {
            agent: BrokerAgent {
                id: credentials.agent_id,
                name,
                version: RUNNER_VERSION,
                authorization: PublicKeyRequest {
                    public_key: crate::client::crypto::public_key(&credentials.private_key)?,
                },
            },
            owner_name: name,
            use_fips_encryption: false,
        };

        self.call(Method::POST, &url, Some(&body))
    }

    pub fn broker_message(
        &self,
        broker: &str,
        session: &str,
        os: &str,
        arch: &str,
    ) -> Result<Option<Envelope>, Error> {
        let url = format!(
            "{}/message?sessionId={session}&status=Online&runnerVersion={RUNNER_VERSION}&os={os}&architecture={arch}&disableUpdate=true",
            broker.trim_end_matches('/'),
        );

        self.maybe(&url)
    }

    pub fn acknowledge(
        &self,
        credentials: &Credentials,
        session: &str,
        message: i64,
    ) -> Result<(), Error> {
        let url = format!(
            "{}/_apis/distributedtask/pools/{}/messages/{message}?api-version=5.1-preview&sessionId={session}",
            credentials.service_url.trim_end_matches('/'),
            credentials.pool_id,
        );

        self.nothing(Method::DELETE, &url)
    }

    /// A job is leased, and the service hands it to somebody else once the lease lapses,
    /// so anything that takes a while has to keep asking.
    pub fn renew_job(&self, run_service_url: &str, job: &JobMessage) -> Result<(), Error> {
        let url = format!("{}/renewjob", run_service_url.trim_end_matches('/'));
        let body = RenewJobRequest {
            plan_id: &job.plan.plan_id,
            job_id: &job.job_id,
        };
        let auth = job
            .service_token()
            .map(|token| format!("Bearer {token}"))
            .or_else(|| self.auth.clone());

        self.send(self.request_as(Method::POST, &url, Some(&body), auth))?;
        Ok(())
    }

    pub fn complete_job(
        &self,
        offer: &JobOffer,
        job: &JobMessage,
        outcome: Outcome,
        steps: &[StepResult],
    ) -> Result<(), Error> {
        let url = format!(
            "{}/completejob",
            offer.run_service_url.trim_end_matches('/')
        );
        let body = CompleteJobRequest {
            plan_id: &job.plan.plan_id,
            job_id: &job.job_id,
            conclusion: outcome.name(),
            billing_owner_id: &offer.billing_owner_id,
            step_results: steps,
        };

        // The job's own token, since the runner's is not what closes a job.
        let auth = job
            .service_token()
            .map(|token| format!("Bearer {token}"))
            .or_else(|| self.auth.clone());

        self.send(self.request_as(Method::POST, &url, Some(&body), auth))?;
        Ok(())
    }

    pub fn acquire_job(&self, offer: &JobOffer) -> Result<JobMessage, Error> {
        let url = format!("{}/acquirejob", offer.run_service_url.trim_end_matches('/'));
        let body = AcquireJobRequest {
            job_message_id: &offer.runner_request_id,
            runner_os: Runner::host_os(),
            billing_owner_id: &offer.billing_owner_id,
        };

        let answered = self.send(self.request(Method::POST, &url, Some(&body)))?;
        JobMessage::decode(&answered)
    }
}

const NO_BODY: Option<&()> = None;

/// Where registration tokens are traded in, which moves for GitHub Enterprise Server.
fn registration_endpoint(url: &str) -> String {
    let host = url
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default();

    match host {
        "github.com" | "www.github.com" => {
            "https://api.github.com/actions/runner-registration".to_owned()
        }
        other => format!("https://{other}/api/v3/actions/runner-registration"),
    }
}

impl Client {
    pub fn update_steps(
        &self,
        job: &JobMessage,
        results_url: &str,
        change_order: u64,
        steps: &[StepResult],
    ) -> Result<(), Error> {
        let body = StepsUpdateRequest {
            workflow_run_backend_id: &job.plan.plan_id,
            workflow_job_run_backend_id: &job.job_id,
            change_order,
            steps,
        };

        self.results(job, results_url, WORKFLOW_STEPS_UPDATE, &body)?;
        Ok(())
    }

    /// Uploads what a step printed, in three parts: ask where, put it there, say it landed.
    ///
    /// `step` names one step, or the whole job's log when it is absent.
    pub fn upload_log(
        &self,
        job: &JobMessage,
        results_url: &str,
        step: Option<&str>,
        log: &str,
    ) -> Result<(), Error> {
        let asking = SignedLogUrlRequest {
            workflow_run_backend_id: &job.plan.plan_id,
            workflow_job_run_backend_id: &job.job_id,
            step_backend_id: step,
        };
        let endpoint = match step {
            Some(_) => STEP_LOGS_URL,
            None => JOB_LOGS_URL,
        };

        let answered = self.results(job, results_url, endpoint, &asking)?;
        let signed: SignedLogUrl = decode(&answered)?;
        self.put_append_blob(&signed, log)?;

        let landed = LogsMetadata {
            workflow_run_backend_id: &job.plan.plan_id,
            workflow_job_run_backend_id: &job.job_id,
            step_backend_id: step,
            uploaded_at: timestamp(std::time::SystemTime::now()),
            line_count: log.lines().count() as u64,
        };
        let endpoint = match step {
            Some(_) => STEP_LOGS_METADATA,
            None => JOB_LOGS_METADATA,
        };

        self.results(job, results_url, endpoint, &landed)?;
        Ok(())
    }

    fn put_append_blob(&self, signed: &SignedLogUrl, log: &str) -> Result<(), Error> {
        debug!(blob = %signed.blob_storage_type, "→ creating the log blob");

        // Azure refuses a blob that does not say what kind it is, whatever the service
        // called its storage.
        let create = self
            .http
            .put(&signed.logs_url)
            .header("x-ms-blob-type", "AppendBlob")
            .header("Content-Type", "text/plain")
            .header("Content-Length", "0");

        self.send(create)?;

        let separator = if signed.logs_url.contains('?') {
            "&"
        } else {
            "?"
        };
        let append = format!("{}{separator}comp=appendblock&seal=true", signed.logs_url);
        self.send(
            self.http
                .put(&append)
                .header("Content-Type", "text/plain")
                .body(log.to_owned()),
        )?;

        Ok(())
    }

    fn results(
        &self,
        job: &JobMessage,
        results_url: &str,
        endpoint: &str,
        body: &impl Serialize,
    ) -> Result<String, Error> {
        let url = format!(
            "{}{endpoint}",
            results_url.trim_end_matches('/').to_owned() + "/"
        );
        let auth = job
            .service_token()
            .map(|token| format!("Bearer {token}"))
            .or_else(|| self.auth.clone());

        self.send(self.request_as(Method::POST, &url, Some(body), auth))
    }
}

const STEP_LOGS_URL: &str = "twirp/results.services.receiver.Receiver/GetStepLogsSignedBlobURL";
const STEP_LOGS_METADATA: &str = "twirp/results.services.receiver.Receiver/CreateStepLogsMetadata";
const JOB_LOGS_URL: &str = "twirp/results.services.receiver.Receiver/GetJobLogsSignedBlobURL";
const JOB_LOGS_METADATA: &str = "twirp/results.services.receiver.Receiver/CreateJobLogsMetadata";
const WORKFLOW_STEPS_UPDATE: &str =
    "twirp/github.actions.results.api.v1.WorkflowStepUpdateService/WorkflowStepsUpdate";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enterprise_registers_against_its_own_host() {
        assert_eq!(
            registration_endpoint("https://github.com/octocat/hello"),
            "https://api.github.com/actions/runner-registration"
        );
        assert_eq!(
            registration_endpoint("https://ghe.example.com/octocat/hello"),
            "https://ghe.example.com/api/v3/actions/runner-registration"
        );
    }
}
