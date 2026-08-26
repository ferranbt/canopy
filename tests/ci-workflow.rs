//! Canopy against the real GitHub: it registers itself as a runner, asks for a workflow to
//! be dispatched at it, runs the job it is handed, and then reads back what GitHub kept.
//!
//! Everything it checks is something the listener encodes on the way out or decodes on the
//! way in, which is where a runner of one's own goes wrong quietly.

use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

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

/// Long enough for GitHub to hand the job over, and for the job to run.
const WAIT: Duration = Duration::from_secs(600);

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

        Ok(Self {
            github: Octocrab::builder()
                .personal_token(token.to_owned())
                .build()?,
            on: Runtime::new()?,
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }

    /// Short-lived and single-use: it buys the credentials a runner keeps.
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

    fn remove_runner(&self, named: &str) -> Result<()> {
        let runners = self.on.block_on(
            self.github
                .actions()
                .list_repo_self_hosted_runners(&self.owner, &self.repo)
                .send(),
        )?;

        let Some(runner) = runners.items.into_iter().find(|it| it.name == named) else {
            return Ok(());
        };

        self.on.block_on(self.github.actions().delete_repo_runner(
            &self.owner,
            &self.repo,
            runner.id,
        ))?;

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
        let outcome = self.work_through(job, progress);
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

    let repository = asked_for("GITHUB_REPOSITORY")?;
    let branch = std::env::var("GITHUB_REF_NAME").unwrap_or_else(|_| "main".to_owned());
    let named = format!(
        "canopy-probe-{}",
        std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| std::process::id().to_string())
    );

    let github = Github::new(&repository, &asked_for("CANOPY_PROBE_TOKEN")?)?;
    let credentials = gh_actions_listener::register(&Registration {
        url: format!("https://github.com/{repository}"),
        token: github.registration_token()?,
        name: named.clone(),
        labels: vec![named.clone()],
    })?;
    println!("registered {named}");

    let (told, heard) = channel();
    let probe = Probe {
        work: PathBuf::from("_work").join(&named),
        told,
    };
    std::thread::spawn(move || {
        if let Err(err) = listen(credentials, probe) {
            tracing::error!(%err, "the listener gave up");
        }
    });

    // The job queues until a runner with the label turns up, so dispatching once the runner
    // is registered is enough; it is picked up as soon as the session is open.
    github.dispatch(PROBE, &branch, &named)?;
    println!("dispatched {PROBE} at {named}");

    let came = heard.recv_timeout(WAIT);
    let deregistered = github.remove_runner(&named);

    let (job, outcome) = came.map_err(|_| eyre!("no job was handed over within {WAIT:?}"))?;
    deregistered?;

    println!(
        "ok    {} came to {} over {} step(s)",
        job.job_display_name,
        outcome.name(),
        job.steps.len()
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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
}
