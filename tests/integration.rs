#[path = "gh-runner/mod.rs"]
mod gh_runner;

use clap::{Parser, ValueEnum};
use eyre::{Result, bail};
use gh_runner::runner::{GhRunner, Job};
use harness::{Case, Harness, Outcome};
use local_runner::{Config, Local};

#[derive(Parser)]
#[command(about = "Runs the corpus on canopy, and holds it to what the runner GitHub ships did")]
struct Cli {
    #[arg(long)]
    test: Option<String>,
    #[arg(long, value_enum, default_value_t = Runner::CnpGhRunner)]
    runner: Runner,
    #[arg(long)]
    validate: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Runner {
    CnpGhRunner,
    OfficialGhRunner,
    LocalRunner,
}

impl Runner {
    fn called(self) -> &'static str {
        match self {
            Self::CnpGhRunner => "_cnp",
            Self::OfficialGhRunner => "",
            Self::LocalRunner => "_loc",
        }
    }
}

fn on_this_machine(case: &Case) -> Result<Outcome, String> {
    let context = local_runner::checkout::context(&case.workspace, "push", &case.temp, false);
    let options = gh_actions_runner::Options {
        workspace: case.workspace.clone(),
        temp: case.temp.clone(),
        cache: gh_actions_runner::actions::cache_directory(),
        service_env: case.service_env.clone(),
        masks: Vec::new(),
    };

    let mut machine = gh_actions_runner::HostMachine::new(vec![
        options.workspace.clone(),
        options.temp.clone(),
        options.cache.clone(),
    ]);

    let mut outcome = Outcome::default();
    gh_actions_runner::run(
        &case.workflow,
        &case.plan,
        &context,
        &options,
        &mut machine,
        &mut outcome,
    )
    .map_err(|err| format!("cannot run: {err}"))?;

    Ok(outcome)
}

fn in_containers(case: &Case) -> Result<Outcome, String> {
    let local = Local::start(Config {
        temp: case.temp.clone(),
        // A store of its own: one kept between runs would answer for work that never
        // happened.
        services: case.artifacts.join("services"),
        ..Config::for_workspace(&case.workspace)
    })
    .map_err(|err| format!("cannot start: {err}"))?;

    let mut outcome = Outcome::default();
    local
        .run(&case.workflow, &case.plan, &mut outcome)
        .map_err(|err| format!("cannot run: {err}"))?;

    Ok(outcome)
}

/// A job at a time, since one is all the runner GitHub ships is ever handed, told what the
/// ones it waited on came to.
fn using_gh_runner(case: &Case) -> Result<Outcome, String> {
    let gh = GhRunner::new()?;
    gh.place(&case.workspace)?;

    let context = local_runner::checkout::context(&case.workspace, "push", &case.temp, false);
    let mut outcome = Outcome::default();
    let mut results = std::collections::BTreeMap::new();

    for planned in &case.plan.jobs {
        let mut context = context.clone();
        context.needs = planned
            .needs
            .iter()
            .filter_map(|id| Some((id.clone(), results.get(id).cloned()?)))
            .collect();

        let came = gh.run(
            Job {
                workflow: &case.workflow,
                planned,
                context: &context,
                services: &case.service_env,
            },
            &mut outcome,
        )?;

        results.insert(planned.id.clone(), came);
    }

    Ok(outcome)
}

fn main() -> Result<()> {
    tracing();

    let cli = Cli::parse();
    if cli.validate && cli.runner == Runner::OfficialGhRunner {
        bail!("`--validate` is for a canopy runner: there is nothing to hold this one to");
    }

    let harness = Harness::new(cli.runner.called());
    let files = harness.get_test_files(cli.test.as_deref());

    let mut failures = Vec::new();
    for file in &files {
        let name = file.name();

        let checked = (|| -> Result<(), String> {
            let outcome = harness.run(&file.path, |case| match cli.runner {
                Runner::CnpGhRunner => on_this_machine(case),
                Runner::LocalRunner => in_containers(case),
                Runner::OfficialGhRunner => using_gh_runner(case),
            })?;
            file.copy(&outcome, cli.runner.called())?;

            if cli.runner == Runner::OfficialGhRunner {
                return Ok(());
            }

            if cli.validate {
                let theirs = harness.run(&file.path, using_gh_runner)?;
                file.copy(&theirs, Runner::OfficialGhRunner.called())?;

                return theirs.matches(&outcome);
            }

            let recorded = file.outcome.as_ref().ok_or("nothing recorded to match")?;
            recorded.matches(&outcome)
        })();

        match checked {
            Ok(()) => println!("ok    {name}"),
            Err(reason) => {
                println!("FAIL  {name}: {reason}");
                failures.push(format!("{name}: {reason}"));
            }
        }
    }

    if files.is_empty() {
        bail!("no workflows to run");
    }
    if !failures.is_empty() {
        bail!("\n{}", failures.join("\n"));
    }

    Ok(())
}

fn tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
}
