//! Report to GitHub how a job is going. It reports both
//! the status of each job and the logs associated.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use gh_actions_context::Conclusion;
use gh_actions_report::{Event, Reporter};
use tracing::{debug, warn};

use crate::client::types::{JobMessage, JobOffer, Outcome, StepResult, timestamp};
use crate::client::{Client, Feed};
use crate::error::Error;

pub struct Progress {
    client: Client,
    job: JobMessage,
    feed: Option<Feed>,
    results_url: Option<String>,
    /// Each step by its place in the job, so a `post` hook joins the step it belongs to.
    steps: BTreeMap<usize, Step>,
    /// Orders updates that arrive out of order.
    change_order: u64,
    lease: Option<Lease>,
}

struct Lease {
    running: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// How often to ask for longer, against a lease measured in minutes.
const RENEW_EVERY: Duration = Duration::from_secs(60);

struct Step {
    id: String,
    name: String,
    outcome: Outcome,
    started_at: SystemTime,
    finished_at: Option<SystemTime>,
    log: String,
}

impl Progress {
    pub fn open(job: &JobMessage) -> Result<Self, Error> {
        let feed = job
            .feed_url()
            .zip(job.service_token())
            .and_then(|(url, token)| match Feed::connect(&url, token) {
                Ok(feed) => Some(feed),
                Err(err) => {
                    warn!(%err, "no live log feed for this job");
                    None
                }
            });

        Ok(Self {
            client: Client::new()?,
            lease: job.run_service_url().map(|url| Lease::keep(job, url)),
            job: job.clone(),
            feed,
            results_url: job.results_url(),
            steps: BTreeMap::new(),
            change_order: 0,
        })
    }

    fn step_started(&mut self, position: usize) {
        let Some(step) = self.job.steps.get(position) else {
            return;
        };
        let (id, name) = (step.id.clone(), step.display_name.clone());

        if let Some(feed) = &mut self.feed {
            feed.step(&id);
        }
        // A `post` hook runs under the position of the step it belongs to, and is that
        // step carrying on rather than one the service has ever heard of.
        self.steps.entry(position).or_insert_with(|| Step {
            id,
            name,
            outcome: Outcome::Succeeded,
            started_at: SystemTime::now(),
            finished_at: None,
            log: String::new(),
        });

        self.push();
    }

    fn line(&mut self, line: &str) {
        let Some(step) = self.steps.values_mut().next_back() else {
            return;
        };
        step.log.push_str(line);
        step.log.push('\n');

        if let Some(feed) = &mut self.feed
            && let Err(err) = feed.lines(std::slice::from_ref(&line.to_owned()))
        {
            debug!(%err, "the log feed dropped a line");
            self.feed = None;
        }
    }

    fn step_finished(&mut self, position: usize, outcome: Outcome) {
        if let Some(step) = self.steps.get_mut(&position) {
            step.finished_at = Some(SystemTime::now());
            if outcome == Outcome::Failed {
                step.outcome = outcome;
            }
        }

        self.push();
    }

    pub(crate) fn finish(&mut self, offer: Option<&JobOffer>, outcome: Outcome) {
        if let Some(lease) = self.lease.take() {
            lease.drop();
        }
        if let Some(feed) = &mut self.feed {
            feed.close();
        }

        let results = self.results();
        if let Some(results_url) = self.results_url.clone() {
            for step in self.steps.values().filter(|step| !step.log.is_empty()) {
                if let Err(err) =
                    self.client
                        .upload_log(&self.job, &results_url, Some(&step.id), &step.log)
                {
                    warn!(%err, step = %step.name, "cannot upload the log of a step");
                }
            }

            let whole: String = self.steps.values().map(|step| step.log.as_str()).collect();
            if !whole.is_empty()
                && let Err(err) = self
                    .client
                    .upload_log(&self.job, &results_url, None, &whole)
            {
                warn!(%err, "cannot upload the log of the job");
            }
        }

        // Until this lands the service still counts the runner as busy with the job.
        if let Some(offer) = offer
            && let Err(err) = self
                .client
                .complete_job(offer, &self.job, outcome, &results)
        {
            warn!(%err, "cannot report what became of the job");
        }
    }

    /// A step with no finish is reported as still running, which is what [`Self::results`]
    /// says for it, so the UI shows it before the job is over.
    fn push(&mut self) {
        let Some(results_url) = self.results_url.clone() else {
            return;
        };
        let steps = self.results();

        self.change_order += 1;
        if let Err(err) =
            self.client
                .update_steps(&self.job, &results_url, self.change_order, &steps)
        {
            warn!(%err, "cannot say where the steps have got to");
        }
    }

    fn results(&self) -> Vec<StepResult> {
        self.steps
            .values()
            .enumerate()
            .map(|(at, step)| StepResult {
                external_id: step.id.clone(),
                number: at as u32 + 1,
                name: step.name.clone(),
                status: match step.finished_at {
                    Some(_) => "completed",
                    None => "inProgress",
                },
                conclusion: step.finished_at.map(|_| step.outcome.name()),
                started_at: Some(timestamp(step.started_at)),
                completed_at: step.finished_at.map(timestamp),
            })
            .collect()
    }
}

impl Reporter for Progress {
    fn report(&mut self, event: Event) {
        for line in gh_actions_report::github_log(&event) {
            self.line(&line);
        }

        match event {
            Event::StepStarted { index, .. } => self.step_started(index),
            Event::StepFinished {
                index, conclusion, ..
            } => {
                let outcome = match conclusion {
                    Conclusion::Failure => Outcome::Failed,
                    _ => Outcome::Succeeded,
                };
                self.step_finished(index, outcome);
            }
            // The rest is about the job, which the listener already knows about.
            _ => {}
        }
    }
}

impl Lease {
    fn keep(job: &JobMessage, run_service_url: String) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let (job, alive) = (job.clone(), running.clone());

        let thread = std::thread::spawn(move || {
            let Ok(client) = Client::new() else {
                return;
            };

            while alive.load(Ordering::Relaxed) {
                // Split so the job is not held for a whole minute after it ends.
                for _ in 0..RENEW_EVERY.as_secs() {
                    if !alive.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }

                match client.renew_job(&run_service_url, &job) {
                    Ok(()) => debug!("asked for longer to run the job"),
                    Err(err) => warn!(%err, "cannot ask for longer to run the job"),
                }
            }
        });

        Self { running, thread }
    }

    fn drop(self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.thread.join();
    }
}
