//! Canopy against the real GitHub: it registers itself as a runner, asks for a workflow to
//! be dispatched at it, runs the job it is handed, and then reads back what GitHub kept.
//!
//! Everything it checks is something the listener encodes on the way out or decodes on the
//! way in, which is where a runner of one's own goes wrong quietly.

use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use chrono::{DateTime, Utc};
use eyre::{Result, eyre};
use gh_actions_context::{Conclusion, Runner};
use gh_actions_listener::{
    Credentials, Error, JobMessage, Listener, Outcome, Progress, Registration, Worker,
};
use gh_actions_plan::PlannedJob;
use gh_actions_runner::report::Terminal;
use gh_actions_runner::{HostMachine, Options};
use gh_actions_spec::{NormalJob, OneOrMany, RunsOn};
use octocrab::Octocrab;
use serde_json::json;
use tokio::runtime::Runtime;

const PROBE: &str = "probe-canopy.yml";

/// What every runner a probe registers is named after, so a leftover one is known for one.
const PROBES: &str = "canopy-probe-";

/// Long enough for GitHub to hand the job over, and for the job to run.
const WAIT: Duration = Duration::from_secs(600);

/// How often GitHub is asked where the run has got to.
const BETWEEN: Duration = Duration::from_secs(10);

/// The few calls the probe makes to GitHub, sync because everything around them is.
struct Github {
    github: Octocrab,
    on: Runtime,
    owner: String,
    repo: String,
}

impl Github {
    fn new(repository: &str, token: &str) -> Result<Self> {
        let (owner, repo) = repository
            .split_once('/')
            .ok_or_else(|| eyre!("{repository:?} is not an owner and a repo"))?;

        let on = Runtime::new()?;
        let github =
            on.block_on(async { Octocrab::builder().personal_token(token.to_owned()).build() })?;

        Ok(Self {
            github,
            on,
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }

    fn registration_token(&self) -> Result<String> {
        let minted = self.on.block_on(
            self.github
                .actions()
                .create_repo_runner_registration_token(&self.owner, &self.repo),
        )?;

        Ok(minted.token)
    }

    fn dispatch(&self, workflow: &str, branch: &str, label: &str) -> Result<()> {
        self.on.block_on(
            self.github
                .actions()
                .create_workflow_dispatch(&self.owner, &self.repo, workflow, branch)
                .inputs(json!({ "label": label }))
                .send(),
        )?;

        Ok(())
    }

    fn wait_for_run(&self, workflow: &str, since: DateTime<Utc>) -> Result<String> {
        let mut waited = Duration::ZERO;

        loop {
            let runs = self.on.block_on(
                self.github
                    .workflows(&self.owner, &self.repo)
                    .list_runs(workflow)
                    .event("workflow_dispatch")
                    .per_page(20)
                    .send(),
            )?;

            let ours = runs
                .items
                .into_iter()
                .filter(|run| run.created_at >= since)
                .min_by_key(|run| run.created_at);

            match ours {
                Some(run) if run.status == "completed" => {
                    return Ok(run.conclusion.unwrap_or_else(|| run.status.clone()));
                }
                Some(run) => tracing::info!(status = %run.status, url = %run.html_url, "the run"),
                None => tracing::info!("the dispatch has not turned into a run yet"),
            }

            if waited >= WAIT {
                return Err(eyre!("the run never finished"));
            }
            std::thread::sleep(BETWEEN);
            waited += BETWEEN;
        }
    }

    fn remove_runners(&self, named: &str) -> Result<()> {
        let runners = self.on.block_on(
            self.github
                .actions()
                .list_repo_self_hosted_runners(&self.owner, &self.repo)
                .send(),
        )?;

        let leftover = runners.items.into_iter().filter(|it| {
            it.name == named || (it.name.starts_with(PROBES) && it.status != "online")
        });

        for runner in leftover {
            tracing::info!(runner = %runner.name, "removing");
            self.on.block_on(self.github.actions().delete_repo_runner(
                &self.owner,
                &self.repo,
                runner.id,
            ))?;
        }

        Ok(())
    }
}

/// Runs the job it is given the way `canopy` would, and tells whoever is waiting what it was
/// handed and what became of it.
struct Probe {
    work: PathBuf,
    told: Sender<(Box<JobMessage>, Outcome)>,
}

impl Worker for Probe {
    fn run(&mut self, job: &JobMessage, progress: &mut Progress) -> Result<Outcome, Error> {
        tracing::info!(
            job = %job.job_display_name,
            steps = job.steps.len(),
            secrets = job.secrets().len(),
            "the probe was handed a job"
        );

        let outcome = self.work_through(job, progress);
        tracing::info!(outcome = outcome.name(), "the probe ran the job");
        let _ = self.told.send((Box::new(job.clone()), outcome));

        Ok(outcome)
    }
}

impl Probe {
    fn work_through(&self, job: &JobMessage, progress: &mut Progress) -> Outcome {
        let options = Options {
            workspace: self.work.join("workspace"),
            temp: self.work.join("temp"),
            cache: gh_actions_runner::actions::cache_directory(),
            service_env: job.env(),
            masks: job.secrets(),
        };
        let _ = std::fs::create_dir_all(&options.workspace);

        let mut context = job.to_run_context();
        context.runner = Runner::host(&options.temp);
        context.github.workspace = options.workspace.display().to_string();

        let Ok(steps) = job.to_steps() else {
            return Outcome::Failed;
        };
        let planned = PlannedJob {
            id: job.job_id.clone(),
            label: job.job_display_name.clone(),
            needs: Vec::new(),
            matrix: Default::default(),
            spec: NormalJob {
                runs_on: Some(RunsOn::Labels(OneOrMany::One("self-hosted".to_owned()))),
                steps: Some(steps),
                ..NormalJob::default()
            },
        };

        let mut machine = HostMachine::new(vec![
            options.workspace.clone(),
            options.temp.clone(),
            options.cache.clone(),
        ]);

        match gh_actions_runner::run_steps(
            &planned,
            &context,
            &options,
            &mut machine,
            &mut Tee(Terminal::default(), progress),
        ) {
            Ok(Conclusion::Failure) | Err(_) => Outcome::Failed,
            Ok(_) => Outcome::Succeeded,
        }
    }
}

struct Tee<'a>(Terminal, &'a mut Progress);

impl gh_actions_runner::report::Reporter for Tee<'_> {
    fn report(&mut self, event: gh_actions_runner::report::Event) {
        self.0.report(event.clone());
        self.1.report(event);
    }
}

fn listen(credentials: Credentials, probe: Probe) -> Result<(), Error> {
    let mut listener = Listener::connect(credentials, probe)?;
    let session = listener.open_session()?;
    let listened = listener.listen(&session);
    let _ = listener.close_session(&session.session_id);

    listened
}

fn main() -> Result<()> {
    tracing();

    let _ = rustls::crypto::ring::default_provider().install_default();

    let repository = asked_for("GITHUB_REPOSITORY")?;
    let branch = asked_for("PROBE_REF")
        .or_else(|_| asked_for("GITHUB_REF_NAME"))
        .unwrap_or_else(|_| "main".to_owned());
    let named = format!(
        "{PROBES}{}",
        std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| std::process::id().to_string())
    );

    let github = Github::new(&repository, &asked_for("CANOPY_PROBE_TOKEN")?)?;
    let credentials = gh_actions_listener::register(&Registration {
        url: format!("https://github.com/{repository}"),
        token: github.registration_token()?,
        name: named.clone(),
        labels: vec![named.clone()],
    })?;
    tracing::info!(runner = %named, "registered");

    let (told, heard) = channel();
    let probe = Probe {
        // Absolute, since a step runs in the workspace and would look for its script under
        // that rather than under where the runner wrote it.
        work: std::env::current_dir()?.join("_work").join(&named),
        told,
    };
    std::thread::spawn(move || {
        if let Err(err) = listen(credentials, probe) {
            tracing::error!(%err, "the listener gave up");
        }
    });

    // The job queues until a runner with the label turns up, so dispatching once the runner
    // is registered is enough; it is picked up as soon as the session is open.
    let since = chrono::Utc::now();
    github.dispatch(PROBE, &branch, &named)?;
    tracing::info!(workflow = PROBE, at = %named, "dispatched, waiting to be handed the job");

    let came = heard.recv_timeout(WAIT);
    let (job, outcome) = came.map_err(|_| eyre!("no job was handed over within {WAIT:?}"))?;

    // Not over when the steps are: the listener is still uploading what they said.
    let conclusion = github.wait_for_run(PROBE, since);
    github.remove_runners(&named)?;

    tracing::info!(
        job = %job.job_display_name,
        outcome = outcome.name(),
        steps = job.steps.len(),
        github = %conclusion?,
        "the probe is done"
    );
    Ok(())
}

fn asked_for(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| eyre!("{name} is not set"))
}

fn tracing() {
    let log_level = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .with_target(false)
        .init();
}
