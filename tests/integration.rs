use std::path::{Path, PathBuf};

use gh_actions_runner::report::{Collected, Event};
use local_runner::{Config, Local};

#[test]
fn run_integration_tests() {
    let testdata = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
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

    // A run of its own each time, so what a case left behind is still there to look at.
    let run = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join(chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
    println!("artifacts under {}", run.display());

    let mut failures = Vec::new();
    for path in &files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let source = std::fs::read_to_string(path).expect("workflow is readable");

        let artifacts = run.join(path.file_stem().unwrap_or_default());
        let workspace = artifacts.join("workspace");
        std::fs::create_dir_all(&workspace).expect("the artifacts directory is made");
        // Copy the actions folder to the workspace so that they can
        // have a relative access to it (TODO: improve)
        copy(&testdata.join("actions"), &workspace.join("actions")).expect("the actions copy");

        let config = Config {
            temp: artifacts.join("temp"),
            ..Config::for_workspace(&workspace)
        };

        match check(path, &source, &config, &artifacts) {
            Ok(()) => println!("ok    {name}"),
            Err(reason) => {
                println!("FAIL  {name}: {reason}");
                failures.push(format!("{name}: {reason} ({})", artifacts.display()));
            }
        }
    }

    assert!(!files.is_empty(), "no workflows to run");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Runs one workflow and compares what happened to what it says should have.
fn check(path: &Path, source: &str, config: &Config, artifacts: &Path) -> Result<(), String> {
    let expect = directive(source, "expect");
    let local = Local::start(config.clone()).map_err(|err| format!("cannot start: {err}"))?;

    let planned = local.plan(path);
    if expect == Some("invalid") {
        return match planned {
            Ok(_) => Err("expected the workflow to be refused".to_owned()),
            Err(err) => {
                let refusal = err.to_string();
                write(artifacts.join("log.txt"), refusal.clone());

                match directive(source, "refuses") {
                    Some(rule) if !refusal.contains(rule) => {
                        Err(format!("refused, but not for {rule}: {refusal}"))
                    }
                    _ => Ok(()),
                }
            }
        };
    }

    let should_succeed = expect != Some("failure");
    let (workflow, plan) = planned.map_err(|err| format!("cannot plan: {err}"))?;
    let mut out = Collected::default();
    let summary = local.run(&workflow, &plan, &mut out);
    // Kept whatever happened, since a run that went wrong is the one worth reading.
    write(artifacts.join("log.txt"), out.output().join("\n"));
    write(
        artifacts.join("events.json"),
        serde_json::to_string_pretty(&out.events).unwrap_or_default(),
    );

    let summary = summary.map_err(|err| format!("cannot run: {err}"))?;
    well_formed(&out)?;

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

fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;

    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());

        match entry.file_type()?.is_dir() {
            true => copy(&entry.path(), &target)?,
            // Which keeps the mode, so what was executable still is.
            false => {
                std::fs::copy(entry.path(), &target)?;
            }
        }
    }

    Ok(())
}

fn write(path: PathBuf, contents: String) {
    if let Err(err) = std::fs::write(&path, contents) {
        println!("cannot write {}: {err}", path.display());
    }
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
