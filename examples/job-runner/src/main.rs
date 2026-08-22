//! Runs one job on this machine, from a file describing it.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use gh_actions_context::{Conclusion, Runner};
use gh_actions_listener::JobMessage;
use gh_actions_plan::PlannedJob;
use gh_actions_report::{Event, Json, Reporter, Terminal};
use gh_actions_runner::{At, Error, HostMachine, Options};
use gh_actions_spec::{NormalJob, OneOrMany, RunsOn};

#[derive(Parser)]
struct Args {
    job: PathBuf,
    #[arg(long, default_value = "_work")]
    work: PathBuf,
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Err(err) = std::fs::create_dir_all(&args.work) {
        eprintln!("error: cannot make {}: {err}", args.work.display());
        return ExitCode::FAILURE;
    }

    let job = match std::fs::read_to_string(&args.job) {
        Ok(raw) => match serde_json::from_str::<JobMessage>(&raw) {
            Ok(job) => job,
            Err(err) => {
                eprintln!("error: {} is not a job: {err}", args.job.display());
                return ExitCode::FAILURE;
            }
        },
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", args.job.display());
            return ExitCode::FAILURE;
        }
    };

    let mut json = Json::new(std::io::stdout());
    let mut terminal = Terminal;
    let out: &mut dyn Reporter = match args.json {
        true => &mut json,
        false => &mut terminal,
    };

    let workspace = Workspace::under(&args.work, name_of(&job));
    let ran = run(&job, &workspace, out);
    let conclusion = match &ran {
        Ok(conclusion) => *conclusion,
        Err(_) => Conclusion::Failure,
    };

    out.report(Event::JobFinished {
        id: job.job_id.clone(),
        label: job.job_display_name.clone(),
        conclusion,
    });

    match ran {
        Ok(Conclusion::Failure) => ExitCode::FAILURE,
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn name_of(job: &JobMessage) -> &str {
    job.context_data
        .github
        .repository
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace")
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub directory: PathBuf,
    pub temp: PathBuf,
    pub cache: PathBuf,
}

impl Workspace {
    pub fn under(work: impl Into<PathBuf>, name: &str) -> Self {
        let work = work.into();
        let work = work.canonicalize().unwrap_or(work);

        Self {
            directory: work.join(name),
            temp: work.join("_temp"),
            cache: gh_actions_runner::actions::cache_directory(),
        }
    }
}

pub fn run(
    job: &JobMessage,
    workspace: &Workspace,
    out: &mut dyn Reporter,
) -> Result<Conclusion, Error> {
    std::fs::create_dir_all(&workspace.directory).at(&workspace.directory)?;

    let steps = job
        .to_steps()
        .map_err(|err| Error::Unsupported(format!("cannot read the steps of this job: {err}")))?;

    let mut context = job.to_run_context();
    context.github.workspace = workspace.directory.display().to_string();
    context.runner = Runner::host(&workspace.temp);

    let options = Options {
        workspace: workspace.directory.clone(),
        temp: workspace.temp.clone(),
        cache: workspace.cache.clone(),
        service_env: job.env(),
        masks: job.secrets(),
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

    gh_actions_runner::run_steps(&planned, &context, &options, &mut HostMachine, out)
}
