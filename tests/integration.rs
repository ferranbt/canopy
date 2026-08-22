use std::path::{Path, PathBuf};

use gh_actions_runner::report::{Collected, Event};
use local_runner::{Config, Local};

#[test]
fn run_integration_tests() {
    let testdata = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testdata");
    let target = std::env::var("TARGET_FILE")
        .ok()
        .filter(|name| !name.is_empty());

    let mut files: Vec<PathBuf> = std::fs::read_dir(&testdata)
        .expect("testdata exists")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "yml"))
        .filter(|path| {
            target
                .as_deref()
                .is_none_or(|name| path.file_name().is_some_and(|file| file == name))
        })
        .collect();
    files.sort();

    let config = Config::for_workspace(&testdata);

    let mut failures = Vec::new();
    for path in &files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let source = std::fs::read_to_string(path).expect("workflow is readable");

        match check(path, &source, &config) {
            Ok(()) => println!("ok    {name}"),
            Err(reason) => {
                println!("FAIL  {name}: {reason}");
                failures.push(format!("{name}: {reason}"));
            }
        }
    }

    assert!(!files.is_empty(), "no workflows to run");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Runs one workflow and compares what happened to what it says should have.
fn check(path: &Path, source: &str, config: &Config) -> Result<(), String> {
    let expect = directive(source, "expect");
    let local = Local::start(config.clone()).map_err(|err| format!("cannot start: {err}"))?;

    let planned = local.plan(path);
    if expect == Some("invalid") {
        return match planned {
            Ok(_) => Err("expected the workflow to be refused".to_owned()),
            Err(err) => match directive(source, "refuses") {
                Some(rule) if !err.to_string().contains(rule) => {
                    Err(format!("refused, but not for {rule}: {err}"))
                }
                _ => Ok(()),
            },
        };
    }

    let should_succeed = expect != Some("failure");
    let (workflow, plan) = planned.map_err(|err| format!("cannot plan: {err}"))?;
    let mut out = Collected::default();
    let summary = local
        .run(&workflow, &plan, &mut out)
        .map_err(|err| format!("cannot run: {err}"))?;
    local.clean();

    well_formed(&out).map_err(|reason| format!("{reason}\n{}", out.output().join("\n")))?;

    if summary.jobs.is_empty() {
        return Err("no jobs ran".to_owned());
    }
    if summary.succeeded() != should_succeed {
        let outcomes: Vec<String> = summary
            .jobs
            .iter()
            .map(|(label, outcome)| format!("{label}={}", outcome.name()))
            .collect();

        return Err(format!(
            "expected the run to {}, got {}",
            if should_succeed { "succeed" } else { "fail" },
            outcomes.join(" ")
        ));
    }

    Ok(())
}

/// Steps nest, so they have to come out as a stack whose depth agrees with how deep it is.
fn well_formed(out: &Collected) -> Result<(), String> {
    let mut jobs = 0usize;
    let mut open: Vec<&str> = Vec::new();

    for event in &out.events {
        match event {
            Event::JobStarted { .. } => jobs += 1,
            Event::JobFinished { label, .. } => {
                if jobs == 0 {
                    return Err(format!("job {label:?} finished without starting"));
                }
                if let Some(name) = open.last() {
                    return Err(format!("job {label:?} finished inside step {name:?}"));
                }
                jobs -= 1;
            }
            Event::StepStarted { name, depth, .. } => {
                if *depth != open.len() {
                    return Err(format!(
                        "step {name:?} says depth {depth} but {} step(s) are open",
                        open.len()
                    ));
                }
                open.push(name);
            }
            Event::StepFinished { name, depth, .. } => match open.pop() {
                Some(started) if started == name && *depth == open.len() => {}
                Some(started) => {
                    return Err(format!("step {name:?} finished, but {started:?} was open"));
                }
                None => return Err(format!("step {name:?} finished without starting")),
            },
            _ => {}
        }
    }

    if jobs != 0 {
        return Err(format!("{jobs} job(s) never finished"));
    }
    if let Some(name) = open.last() {
        return Err(format!("step {name:?} never finished"));
    }
    Ok(())
}

/// Reads a `# name: value` directive from the comments a workflow opens with.
fn directive<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    source
        .lines()
        .take_while(|line| line.starts_with('#'))
        .filter_map(|line| line.trim_start_matches('#').trim().split_once(':'))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim())
}
