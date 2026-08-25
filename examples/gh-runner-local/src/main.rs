use std::path::PathBuf;

use clap::Parser;
use gh_actions_context::{Conclusion, Runner};
use gh_actions_listener::{Credentials, Error, JobMessage, Listener, Outcome, Progress, Worker};
use gh_actions_plan::PlannedJob;
use gh_actions_runner::report::{Event, Reporter, Terminal};
use gh_actions_runner::{HostMachine, Options};
use gh_actions_spec::{NormalJob, OneOrMany, RunsOn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "credentials.json")]
    credentials: PathBuf,
    #[arg(long, default_value = "_work")]
    workspace: PathBuf,
}

fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .with_target(false)
        .init();

    let args = Args::parse();
    std::fs::create_dir_all(&args.workspace)?;
    let credentials = Credentials::read(&args.credentials)?;

    let mut listener = Listener::connect(credentials, Host::new(args.workspace))?;

    match listener.agent() {
        Ok(agent) => tracing::info!(%agent, "the service says"),
        Err(err) => tracing::warn!(%err, "cannot read the runner record"),
    }

    let remembered = args.credentials.with_extension("session");
    if let Ok(stale) = std::fs::read_to_string(&remembered) {
        match listener.close_session(stale.trim()) {
            Ok(()) => tracing::info!(session = stale.trim(), "closed a stale session"),
            Err(err) => tracing::debug!(%err, "no stale session to close"),
        }
    }

    let session = listener.open_session()?;
    std::fs::write(&remembered, &session.session_id)?;

    let listened = listener.listen(&session);
    listener.close_session(&session.session_id)?;
    let _ = std::fs::remove_file(&remembered);
    listened
}

struct Host {
    work: PathBuf,
}

struct Tee<'a>(Terminal, &'a mut Progress);

impl Reporter for Tee<'_> {
    fn report(&mut self, event: Event) {
        self.0.report(event.clone());
        self.1.report(event);
    }
}

impl Host {
    fn new(work: PathBuf) -> Self {
        Self {
            work: work.canonicalize().unwrap_or(work),
        }
    }

    fn options(&self, job: &JobMessage) -> Options {
        let repository = job.context_data.github.repository.clone();
        let name = repository.rsplit('/').next().unwrap_or("workspace");

        let options = Options {
            workspace: self.work.join(name),
            temp: self.work.join("_temp"),
            cache: gh_actions_runner::actions::cache_directory(),
            service_env: job.env(),
            masks: job.secrets(),
        };

        let _ = std::fs::create_dir_all(&options.workspace);
        options
    }
}

impl Worker for Host {
    fn run(&mut self, job: &JobMessage, progress: &mut Progress) -> Result<Outcome, Error> {
        let options = self.options(job);
        let mut context = job.to_run_context();
        context.runner = Runner::host(&options.temp);
        context.github.workspace = options.workspace.display().to_string();

        let steps = match job.to_steps() {
            Ok(steps) => steps,
            Err(err) => {
                tracing::warn!(%err, "cannot read the steps of this job");
                return Ok(Outcome::Failed);
            }
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

        let conclusion = gh_actions_runner::run_steps(
            &planned,
            &context,
            &options,
            &mut machine,
            &mut Tee(Terminal::default(), progress),
        );

        Ok(match conclusion {
            Ok(Conclusion::Failure) => Outcome::Failed,
            Err(err) => {
                tracing::warn!(%err, "the job could not be run");
                Outcome::Failed
            }
            Ok(_) => Outcome::Succeeded,
        })
    }
}
