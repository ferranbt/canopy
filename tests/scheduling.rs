use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gh_actions_context::Conclusion;
use gh_actions_runner::report::Collected;
use gh_actions_runner::{HostMachine, Summary};
use local_runner::{Config, Local};

fn workflows() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scheduling")
}

fn directive(source: &str, name: &str) -> Vec<String> {
    let said = format!("{name}: ");

    source
        .lines()
        .map_while(|line| line.strip_prefix('#'))
        .filter_map(|line| line.trim_start().strip_prefix(&said))
        .map(|line| line.trim().to_owned())
        .collect()
}

fn ran(path: &Path) -> Summary {
    let at = std::env::temp_dir().join(format!(
        "canopy-scheduling-{}-{}",
        std::process::id(),
        path.file_stem().expect("a name").to_string_lossy()
    ));
    let workspace = harness::workspace(&at).expect("somewhere to run");

    let local = Local::start(Config {
        temp: at.join("temp"),
        services: at.join("services"),
        ..Config::for_workspace(&workspace)
    })
    .expect("a local run");
    let (workflow, plan) = local.plan(path).expect("a workflow that plans");
    let options = local.options();

    let mut machine = HostMachine::new(vec![
        options.workspace.clone(),
        options.temp.clone(),
        options.cache.clone(),
    ]);

    gh_actions_runner::run(
        &workflow,
        &plan,
        local.context(),
        &options,
        &mut machine,
        &mut Collected::default(),
    )
    .expect("the workflow runs")
}

#[test]
fn every_workflow_is_scheduled_the_way_it_says() {
    for entry in std::fs::read_dir(workflows())
        .expect("the workflows are there")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_none_or(|kind| kind != "yml") {
            continue;
        }

        let name = path
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let source = std::fs::read_to_string(&path).expect("the workflow reads");

        let wanted: BTreeMap<String, Conclusion> = directive(&source, "expect")
            .iter()
            .filter_map(|said| said.split_once(" = "))
            .map(|(label, outcome)| {
                (
                    label.trim().to_owned(),
                    Conclusion::from_name(outcome.trim()),
                )
            })
            .collect();
        assert!(!wanted.is_empty(), "{name} says nothing to expect");

        let summary = ran(&path);
        let mut came: BTreeMap<String, Conclusion> = summary.jobs.iter().cloned().collect();
        came.insert(
            "run".to_owned(),
            match summary.succeeded() {
                true => Conclusion::Success,
                false => Conclusion::Failure,
            },
        );

        assert_eq!(came, wanted, "{name} was scheduled otherwise");
    }
}
