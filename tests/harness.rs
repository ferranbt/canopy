use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gh_actions_plan::Plan;
use gh_actions_runner::report::{Event, Reporter};
use gh_actions_services::Services;
use gh_actions_spec::Workflow;
use local_runner::{Config, Local};

#[derive(Debug, Default)]
pub struct Outcome {
    pub events: Vec<Event>,
}

impl Reporter for Outcome {
    fn report(&mut self, event: Event) {
        // How a runner goes about running a step is its own business: only the groups a
        // workflow asks for, which are named in brackets, are the run saying something.
        if let Event::Progress { text } = &event
            && !text.starts_with('[')
        {
            return;
        }

        // What a composite action is made of is canopy's own telling: the runner GitHub
        // ships keeps an action's inner steps to itself.
        let nested = match &event {
            Event::StepStarted { depth, .. } | Event::StepFinished { depth, .. } => *depth > 0,
            _ => false,
        };
        if nested {
            return;
        }

        self.events.push(event);
    }
}

impl Outcome {
    pub fn output(&self) -> Vec<&str> {
        self.events
            .iter()
            .filter_map(|event| match event {
                Event::StepOutput { line, .. } => Some(line.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Where this run happened, under the name every run knows that place by.
    pub fn rewrite(&mut self, path: &Path, name: &str) {
        let path = path.display().to_string();

        for event in &mut self.events {
            let said = match event {
                Event::StepOutput { line, .. } => line,
                Event::Message { text, .. } => text,
                _ => continue,
            };

            *said = said.replace(&path, name);
        }
    }

    pub fn matches(&self, other: &Self) -> Result<(), String> {
        for (at, (mine, theirs)) in self.events.iter().zip(&other.events).enumerate() {
            if mine != theirs {
                return Err(format!(
                    "event {at} differs\n  expected {}\n  got      {}",
                    one(mine),
                    one(theirs)
                ));
            }
        }

        let (mine, theirs) = (self.events.len(), other.events.len());
        match mine.cmp(&theirs) {
            std::cmp::Ordering::Equal => Ok(()),
            std::cmp::Ordering::Less => Err(format!(
                "{} event(s) too many, from {}",
                theirs - mine,
                one(&other.events[mine])
            )),
            std::cmp::Ordering::Greater => Err(format!(
                "{} event(s) missing, from {}",
                mine - theirs,
                one(&self.events[theirs])
            )),
        }
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let recorded = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
        let events =
            serde_json::from_str(&recorded).map_err(|err| format!("{}: {err}", path.display()))?;

        Ok(Self { events })
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let recorded = serde_json::to_string_pretty(&self.events).map_err(|err| err.to_string())?;
        std::fs::write(path, recorded).map_err(|err| format!("{}: {err}", path.display()))
    }
}

fn one(event: &Event) -> String {
    serde_json::to_string(event).unwrap_or_default()
}

/// A case to run, with what it did the last time the runner GitHub ships ran it.
pub struct TargetFile {
    pub path: PathBuf,
    pub outcome: Option<Outcome>,
}

impl TargetFile {
    pub fn name(&self) -> String {
        self.path
            .strip_prefix(testdata())
            .unwrap_or(&self.path)
            .to_string_lossy()
            .to_string()
    }

    pub fn record(&self, outcome: &Outcome) -> Result<(), String> {
        outcome.write(&expected(&self.path))
    }
}

fn expected(case: &Path) -> PathBuf {
    case.with_file_name(format!(
        "{}_output.json",
        case.file_stem().unwrap_or_default().to_string_lossy()
    ))
}

pub struct Case {
    pub name: String,
    pub artifacts: PathBuf,
    pub workspace: PathBuf,
    pub temp: PathBuf,
    pub workflow: Workflow,
    pub plan: Plan,
    pub service_env: BTreeMap<String, String>,
}

pub struct Harness {
    artifacts: PathBuf,
    services: Services,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new("")
    }
}

impl Harness {
    pub fn new(prefix: &str) -> Self {
        let at = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let artifacts = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts")
            .join(format!("{prefix}{at}"));

        Self {
            services: Services::start(artifacts.join("services")).expect("the services start"),
            artifacts,
        }
    }

    pub fn get_test_files(&self, target: Option<&str>) -> Vec<TargetFile> {
        let testdata = testdata();
        let target = target.filter(|name| !name.is_empty());

        let mut cases: Vec<PathBuf> = glob::glob(&format!("{}/**/*.yml", testdata.display()))
            .expect("a pattern testdata can be found with")
            .filter_map(Result::ok)
            .collect();

        cases.retain(|path| {
            target.is_none_or(|wanted| {
                path.strip_prefix(&testdata).is_ok_and(|case| {
                    let case = case.to_string_lossy();
                    case.ends_with(wanted) || case.starts_with(wanted)
                })
            })
        });
        cases.sort();

        cases
            .into_iter()
            .map(|path| TargetFile {
                outcome: Outcome::read(&expected(&path)).ok(),
                path,
            })
            .collect()
    }

    pub fn run(
        &self,
        file: &Path,
        run: impl FnOnce(&Case) -> Result<Outcome, String>,
    ) -> Result<Outcome, String> {
        let case = self.prepare(file)?;
        let mut outcome = run(&case)?;

        // Under the names a run knows them by, so what one runner did reads the same as
        // what another did somewhere else.
        outcome.rewrite(&case.temp, "$RUNNER_TEMP");
        outcome.rewrite(&case.workspace, "$GITHUB_WORKSPACE");

        outcome.write(&case.artifacts.join("events.json"))?;
        well_formed(&outcome)?;

        Ok(outcome)
    }

    fn prepare(&self, file: &Path) -> Result<Case, String> {
        let testdata = testdata();
        let name = file
            .strip_prefix(&testdata)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();

        let artifacts = self.artifacts.join(name.trim_end_matches(".yml"));
        let workspace = artifacts.join("workspace");
        std::fs::create_dir_all(&workspace)
            .map_err(|err| format!("cannot make a workspace: {err}"))?;

        let actions = testdata.parent().unwrap_or(&testdata).join("actions");
        copy(&actions, &workspace.join("actions"))
            .map_err(|err| format!("cannot copy the actions: {err}"))?;

        let temp = artifacts.join("temp");
        let planner = Local::start(Config {
            temp: temp.clone(),
            ..Config::for_workspace(&workspace)
        })
        .map_err(|err| format!("cannot start: {err}"))?;

        let (workflow, plan) = planner
            .plan(file)
            .map_err(|err| format!("cannot plan: {err}"))?;

        Ok(Case {
            name,
            artifacts,
            workspace,
            temp,
            workflow,
            plan,
            service_env: self.services.env(),
        })
    }
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn well_formed(outcome: &Outcome) -> Result<(), String> {
    let mut jobs = 0usize;
    let mut open: Vec<&str> = Vec::new();

    for event in &outcome.events {
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
            false => {
                std::fs::copy(entry.path(), &target)?;
            }
        }
    }

    Ok(())
}
