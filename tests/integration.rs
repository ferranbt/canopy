use clap::Parser;
use eyre::{Result, bail};
use harness::{Harness, Outcome};
use local_runner::{Config, Local};

#[derive(Parser)]
#[command(about = "Runs the corpus on canopy, and holds it to what the runner GitHub ships did")]
struct Cli {
    /// A case or a group of them, by either end of its path under testdata.
    #[arg(long)]
    test: Option<String>,
}

fn main() -> Result<()> {
    let harness = Harness::new("cnp-");
    let files = harness.get_test_files(Cli::parse().test.as_deref());

    let mut failures = Vec::new();
    for file in &files {
        let name = file.name();

        let ran = harness.run(&file.path, |case| {
            let local = Local::start(Config {
                temp: case.temp.clone(),
                ..Config::for_workspace(&case.workspace)
            })
            .map_err(|err| format!("cannot start: {err}"))?;

            let mut outcome = Outcome::default();
            local
                .run(&case.workflow, &case.plan, &mut outcome)
                .map_err(|err| format!("cannot run: {err}"))?;

            Ok(outcome)
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
