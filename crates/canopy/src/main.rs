use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use eyre::{Result, bail};
use local_runner::{Config, Local};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "canopy", about = "run a GitHub Actions workflow locally")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a workflow.
    Run {
        workflow: PathBuf,
        /// Run only this job, and the jobs it needs.
        #[arg(short, long)]
        job: Option<String>,
        /// Event name reported as github.event_name.
        #[arg(short, long, default_value = "push")]
        event: String,
        /// Directory the steps run in, defaulting to the current one.
        #[arg(short = 'C', long)]
        workspace: Option<PathBuf>,
        /// Print the planned jobs and exit.
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Report what happens as JSON lines, one event per line.
        #[arg(long)]
        json: bool,
    },
    /// Check a workflow and report what is wrong, without running it.
    Lint {
        workflow: PathBuf,
        #[arg(short = 'C', long)]
        workspace: Option<PathBuf>,
    },
    /// Run the language server, speaking LSP on stdin and stdout.
    Lsp {
        /// Taken for the clients that pass it; stdio is the only transport there is.
        #[arg(long)]
        stdio: bool,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    match Args::parse().command {
        Command::Lsp { .. } => {
            canopy_lsp::run();
            Ok(())
        }
        Command::Lint {
            workflow,
            workspace,
        } => lint(&workflow, workspace),
        Command::Run {
            workflow,
            job,
            event,
            workspace,
            dry_run,
            json,
        } => execute(&workflow, job, &event, workspace, dry_run, json),
    }
}

fn lint(workflow: &Path, workspace: Option<PathBuf>) -> Result<()> {
    let local = start(workspace, "push")?;
    let (document, workflow) = local.read(workflow)?;

    let mut findings = gh_actions_plan::validate::check(&workflow);
    findings.extend(gh_actions_lint::check(&document, &workflow));
    for finding in &findings {
        info!("{finding}");
    }

    if gh_actions_lint::has_errors(&findings) {
        bail!("{} problem(s) found", findings.len());
    }

    info!(problems = findings.len(), "checked");
    Ok(())
}

fn execute(
    workflow: &Path,
    job: Option<String>,
    event: &str,
    workspace: Option<PathBuf>,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let local = start(workspace, event)?;
    let (workflow, mut plan) = local.plan(workflow)?;

    if let Some(id) = &job {
        plan = plan.select(id)?;
    }

    if dry_run {
        for job in &plan.jobs {
            info!(needs = %job.needs.join(", "), "{}", job.label);
        }
        return Ok(());
    }

    info!(artifacts = %local.services().root().display(), "services listening");

    let mut out: Box<dyn gh_actions_runner::Reporter> = if json {
        Box::new(gh_actions_runner::Json::new(std::io::stdout()))
    } else {
        Box::new(gh_actions_runner::Terminal)
    };
    let summary = local.run(&workflow, &plan, out.as_mut())?;
    local.clean();

    for (label, outcome) in &summary.jobs {
        info!(outcome = outcome.name(), "{label}");
    }
    if summary.jobs.is_empty() {
        bail!("no jobs ran");
    }
    if !summary.succeeded() {
        bail!("the run failed");
    }

    Ok(())
}

fn start(workspace: Option<PathBuf>, event: &str) -> Result<Local> {
    let workspace = match workspace {
        Some(workspace) => workspace,
        None => std::env::current_dir()?,
    };

    Ok(Local::start(Config::for_workspace(workspace).event(event))?)
}
