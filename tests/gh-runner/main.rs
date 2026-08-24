mod message;
mod runner;
mod service;

use clap::Parser;
use eyre::{Result, bail};
use harness::{Harness, Outcome};
use runner::GhRunner;

#[derive(Parser)]
#[command(about = "Runs the corpus on the runner GitHub ships, and records what it did")]
struct Cli {
    #[arg(long)]
    test: Option<String>,
}

fn main() -> Result<()> {
    tracing();

    let harness = Harness::new("gh-");
    let files = harness.get_test_files(Cli::parse().test.as_deref());
    let gh = GhRunner::new().map_err(eyre::Report::msg)?;

    let mut failures = Vec::new();
    for file in &files {
        let name = file.name();

        let ran = harness.run(&file.path, |case| {
            gh.place(&case.workspace)?;

            let context =
                local_runner::checkout::context(&case.workspace, "push", &case.temp, false);
            let mut outcome = Outcome::default();

            for planned in &case.plan.jobs {
                gh.run(
                    runner::Job {
                        workflow: &case.workflow,
                        planned,
                        context: &context,
                        services: &case.service_env,
                    },
                    &mut outcome,
                )?;
            }

            Ok(outcome)
        });

        match ran.and_then(|outcome| file.record(&outcome)) {
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
