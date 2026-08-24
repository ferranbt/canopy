mod message;
mod runner;
mod service;

use std::collections::BTreeMap;

use clap::Parser;
use eyre::{Result, bail};
use gh_actions_context::Conclusion;
use gh_actions_plan::PlannedJob;
use gh_actions_runner::report::{Event, PassedOver, Reporter};
use harness::{Harness, Outcome};
use runner::GhRunner;

#[derive(Parser)]
#[command(about = "Runs the corpus on the runner GitHub ships, and records what it did")]
struct Cli {
    /// A case or a group of them, by either end of its path under testdata.
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
            // Which job runs at all is the service's to decide, never the runner's: it is
            // only ever handed jobs that are meant to run.
            let mut done: BTreeMap<String, Conclusion> = BTreeMap::new();

            for planned in &case.plan.jobs {
                if let Some(reason) = passed_over(planned, &done) {
                    outcome.report(Event::JobPassedOver {
                        label: planned.label.clone(),
                        reason,
                    });
                    // Without losing what the job is already known to have done: one
                    // combination failing is what calls the rest of them off.
                    done.entry(planned.id.clone()).or_insert(Conclusion::Skipped);

                    continue;
                }

                gh.run(
                    runner::Job {
                        workflow: &case.workflow,
                        planned,
                        context: &context,
                        needs: &needed(planned, &done, &outcome),
                        services: &case.service_env,
                    },
                    &mut outcome,
                )?;

                done.insert(planned.id.clone(), ended(planned, &outcome, &planned.label));
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

/// Why a job is not run at all, if it is not to be.
///
/// A job whose `needs` failed never starts, and one combination of a matrix failing calls
/// off the rest of it, which is what `fail-fast` means and what it defaults to.
fn passed_over(job: &PlannedJob, done: &BTreeMap<String, Conclusion>) -> Option<PassedOver> {
    let fail_fast = job
        .spec
        .strategy
        .as_ref()
        .and_then(|strategy| said(&strategy.fail_fast))
        .unwrap_or(true);

    if fail_fast && done.get(&job.id) == Some(&Conclusion::Failure) {
        return Some(PassedOver::Cancelled);
    }

    // A job that says when it runs decides for itself, and what it usually says is that it
    // runs whatever happened. Anything else waits for what it needs to have gone well.
    if job.spec.r#if.is_some() {
        return None;
    }

    let wanted = job
        .needs
        .iter()
        .all(|need| done.get(need) == Some(&Conclusion::Success));

    (!wanted).then_some(PassedOver::Skipped)
}

/// What the jobs this one waited for came to, as the `needs` context has it: how each of
/// them went, and what it came out with.
fn needed(
    job: &PlannedJob,
    done: &BTreeMap<String, Conclusion>,
    outcome: &Outcome,
) -> BTreeMap<String, serde_json::Value> {
    job.needs
        .iter()
        .map(|need| {
            let outputs = outcome
                .events
                .iter()
                .rev()
                .find_map(|event| match event {
                    Event::JobOutputs { id, outputs } if id == need => Some(outputs.clone()),
                    _ => None,
                })
                .unwrap_or_default();

            let result = done.get(need).copied().unwrap_or(Conclusion::Skipped);
            (
                need.clone(),
                serde_json::json!({ "result": result.name(), "outputs": outputs }),
            )
        })
        .collect()
}

/// A flag as it was written down, where it was written down as a flag rather than as
/// something to work out from the run.
fn said(flag: &Option<gh_actions_spec::Expr<bool>>) -> Option<bool> {
    match flag {
        Some(gh_actions_spec::Expr::Value(said)) => Some(*said),
        _ => None,
    }
}

/// How a job ended, which is what it was last seen saying about itself.
///
/// A job allowed to fail counts as one that did not: what it says it did is one thing, and
/// what the jobs after it are told is another.
fn ended(job: &PlannedJob, outcome: &Outcome, label: &str) -> Conclusion {
    if said(&job.spec.continue_on_error).unwrap_or(false) {
        return Conclusion::Success;
    }

    outcome
        .events
        .iter()
        .rev()
        .find_map(|event| match event {
            Event::JobFinished {
                label: ended,
                conclusion,
                ..
            } if ended == label => Some(*conclusion),
            _ => None,
        })
        .unwrap_or(Conclusion::Failure)
}

/// What the run says about itself, which `RUST_LOG` asks for: `RUST_LOG=csharp=debug` for
/// what the runner is told, `trace` for every call it makes.
fn tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
}
