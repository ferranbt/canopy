use clap::{Parser, ValueEnum};
use eyre::{Result, bail};
use harness::{Case, Harness, Outcome};
use local_runner::{Config, Local};

#[derive(Parser)]
#[command(about = "Runs the corpus on canopy, and holds it to what the runner GitHub ships did")]
struct Cli {
    /// A case or a group of them, by either end of its path under testdata.
    #[arg(long)]
    test: Option<String>,
    #[arg(long, value_enum, default_value_t = Runner::GhActionsRunner)]
    runner: Runner,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Runner {
    GhActionsRunner,
    LocalRunner,
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

    let mut outcome = Outcome::default();
    gh_actions_runner::run(
        &case.workflow,
        &case.plan,
        &context,
        &options,
        &mut gh_actions_runner::HostMachine,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let harness = Harness::new("cnp-");
    let files = harness.get_test_files(cli.test.as_deref());

    let mut failures = Vec::new();
    for file in &files {
        let name = file.name();

        let ran = harness.run(&file.path, |case| match cli.runner {
            Runner::GhActionsRunner => on_this_machine(case),
            Runner::LocalRunner => in_containers(case),
        });

        let checked = ran.and_then(|outcome| {
            file.ours(&outcome)?;

            let recorded = file.outcome.as_ref().ok_or("nothing recorded to match")?;
            recorded.matches(&outcome)
        });

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
