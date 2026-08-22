mod aws;

use std::path::PathBuf;

use clap::Parser;
use gh_actions_listener::{Credentials, Error, JobMessage, Listener, Outcome, Progress, Worker};
use gh_actions_report::{Event, Reporter};
use tracing_subscriber::EnvFilter;

use crate::aws::{Aws, Log};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "credentials.json")]
    credentials: PathBuf,
    #[command(flatten)]
    at: Fargate,
}

#[derive(Parser, Clone)]
pub struct Fargate {
    #[arg(long)]
    pub cluster: String,
    #[arg(long)]
    pub task_definition: String,
    #[arg(long, default_value = "job-runner")]
    pub container: String,
    #[arg(long)]
    pub bucket: String,
    #[arg(long)]
    pub log_group: String,
    #[arg(long, value_delimiter = ',')]
    pub subnets: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    pub security_groups: Vec<String>,
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
    let credentials = Credentials::read(&args.credentials)?;
    let aws = Aws::new().map_err(Error::Protocol)?;

    let mut listener = Listener::connect(credentials, Tasks { at: args.at, aws })?;
    let session = listener.open_session()?;
    let listened = listener.listen(&session);
    listener.close_session(&session.session_id)?;
    listened
}

struct Tasks {
    at: Fargate,
    aws: Aws,
}

impl Worker for Tasks {
    fn run(&mut self, job: &JobMessage, progress: &mut Progress) -> Result<Outcome, Error> {
        match self.dispatch(job, progress) {
            Ok(outcome) => Ok(outcome),
            Err(err) => {
                tracing::warn!(%err, "the job could not be run on fargate");
                Ok(Outcome::Failed)
            }
        }
    }
}

impl Tasks {
    fn dispatch(&self, job: &JobMessage, progress: &mut Progress) -> Result<Outcome, String> {
        let payload =
            serde_json::to_string(job).map_err(|err| format!("cannot write the job out: {err}"))?;

        let key = format!("jobs/{}.json", job.job_id);
        let uri = self.aws.put_job(&self.at, &key, &payload)?;
        let task = self.aws.run_task(&self.at, &uri)?;
        let stream = format!(
            "{}/{}/{}",
            self.at.container,
            self.at.container,
            task.rsplit('/').next().unwrap_or_default()
        );
        tracing::info!(%task, %stream, "started a task");

        self.follow(&stream, &task, progress)
    }

    fn follow(&self, stream: &str, task: &str, progress: &mut Progress) -> Result<Outcome, String> {
        let mut outcome = None;

        for message in self.aws.lines(&self.at, stream, task) {
            let line = match message? {
                Log::Line(line) => line,
                Log::Stopped => break,
            };
            let Ok(event) = serde_json::from_str::<Event>(&line) else {
                tracing::debug!(%line, "the task said something that is not an event");
                continue;
            };

            if let Event::JobFinished { conclusion, .. } = &event {
                outcome = Some(match conclusion {
                    gh_actions_context::Conclusion::Failure => Outcome::Failed,
                    _ => Outcome::Succeeded,
                });
            }
            progress.report(event);
        }

        Ok(outcome.unwrap_or(Outcome::Failed))
    }
}
