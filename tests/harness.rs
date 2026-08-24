use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gh_actions_context::Payload;
use gh_actions_plan::Plan;
use gh_actions_runner::report::{Event, Reporter};
use gh_actions_services::Services;
use gh_actions_spec::Workflow;
use local_runner::{Config, Local};

/// What running one planned job came to: what happened, and what each step printed.
#[derive(Debug, Default)]
pub struct Outcome {
    pub events: Vec<Event>,
    /// One log per step, in the order the steps ran.
    pub logs: Vec<Printed>,
}

/// What one step printed, kept apart from the next one's.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Printed {
    pub step: String,
    pub lines: Vec<String>,
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

        // What the node an action runs on says about itself, which is about the node and
        // not the action: a runner carries its own, and whatever is here is another.
        if let Event::StepOutput { line, .. } = &event
            && (line.contains("DeprecationWarning:") || line.starts_with("(Use `node"))
        {
            return;
        }

        // How a container was built and started is the runner's own business again, and no
        // two builds tell it the same way: what a step is, is what it ran, not what docker
        // said while it was got ready.
        if let Event::StepOutput { line, .. } = &event
            && got_ready(line)
        {
            return;
        }

        // What a composite action is made of is canopy's own telling: the runner GitHub
        // ships keeps an action's inner steps to itself. What they print is the step's all
        // the same, so only the boundaries go.
        let nested = match &event {
            Event::StepStarted { depth, .. } | Event::StepFinished { depth, .. } => *depth > 0,
            _ => false,
        };
        if nested {
            return;
        }

        match event {
            // Under the step that printed it, which is the one still open. What a line
            // ends in is a runner's, not a step's: one of them uploads what it was given
            // and the other hands it over as it read it.
            Event::StepOutput { line, .. } => {
                if let Some(printed) = self.logs.last_mut() {
                    printed.lines.push(line.trim_end().to_owned());
                }
            }
            Event::StepStarted { name, .. } => {
                self.logs.push(Printed {
                    step: name.clone(),
                    lines: Vec::new(),
                });
                self.events.push(Event::StepStarted {
                    index: self.logs.len() - 1,
                    name,
                    depth: 0,
                });
            }
            event => self.events.push(event),
        }
    }
}

impl Outcome {
    /// What is only true of this run, under the name every run knows it by: where it
    /// happened, and which commit of what it happened on.
    pub fn rewrite(&mut self, from: &str, name: &str) {
        let path = from.to_owned();

        for said in self.said() {
            *said = said.replace(&path, name);
        }
    }

    /// What no two runs agree on and neither is wrong about: the ids given to the files a
    /// run makes as it goes, and how many bytes something it packed came to.
    pub fn settle(&mut self) {
        for said in self.said() {
            *said = settled(said);

            if let Some((before, rest)) = said.split_once(" B)")
                && let Some((head, bytes)) = before.rsplit_once('(')
                && bytes.chars().all(|it| it.is_ascii_digit())
            {
                *said = format!("{head}(<bytes>){rest}");
            }
        }
    }

    fn said(&mut self) -> impl Iterator<Item = &mut String> {
        let messages = self.events.iter_mut().filter_map(|event| match event {
            Event::Message { text, .. } => Some(text),
            _ => None,
        });

        messages.chain(self.logs.iter_mut().flat_map(|printed| &mut printed.lines))
    }

    /// The same run, or the first thing the two do not agree on: what happened first, and
    /// then what was said while it happened.
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
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Less => {
                return Err(format!(
                    "{} event(s) too many, from {}",
                    theirs - mine,
                    one(&other.events[mine])
                ));
            }
            std::cmp::Ordering::Greater => {
                return Err(format!(
                    "{} event(s) missing, from {}",
                    mine - theirs,
                    one(&self.events[theirs])
                ));
            }
        }

        for (mine, theirs) in self.logs.iter().zip(&other.logs) {
            if mine != theirs {
                return Err(format!(
                    "step {:?} printed something else\n  expected {}\n  got      {}",
                    mine.step,
                    mine.lines.join(" ⏎ "),
                    theirs.lines.join(" ⏎ ")
                ));
            }
        }

        Ok(())
    }

    /// Read back from where it was written down: what happened, and a log for each step.
    pub fn read(at: &Path) -> Result<Self, String> {
        let happened = at.join("steps.json");

        let recorded = std::fs::read_to_string(&happened).map_err(|err| err.to_string())?;
        let events = serde_json::from_str(&recorded)
            .map_err(|err| format!("{}: {err}", happened.display()))?;

        let mut outcome = Self {
            events,
            ..Self::default()
        };

        for (of, step) in named(&outcome.events) {
            let text = std::fs::read_to_string(log(at, "", of, &step)).unwrap_or_default();
            outcome.logs.push(Printed {
                step,
                lines: text.lines().map(str::to_owned).collect(),
            });
        }

        Ok(outcome)
    }

    /// Written down under a name of its own, so what one runner did sits beside what
    /// another did rather than over it.
    pub fn write(&self, at: &Path, called: &str) -> Result<(), String> {
        std::fs::create_dir_all(at).map_err(|err| format!("{}: {err}", at.display()))?;
        clear(at, called);

        let happened = at.join(format!("{called}steps.json"));
        let recorded = serde_json::to_string_pretty(&self.events).map_err(|err| err.to_string())?;
        std::fs::write(&happened, recorded)
            .map_err(|err| format!("{}: {err}", happened.display()))?;

        for (of, printed) in self.logs.iter().enumerate() {
            if printed.lines.is_empty() {
                continue;
            }

            let path = log(at, called, of, &printed.step);
            let said = printed.lines.join("\n");

            std::fs::write(&path, said).map_err(|err| format!("{}: {err}", path.display()))?;
        }

        Ok(())
    }
}

/// What a run left here last time, so a step that no longer prints leaves nothing behind.
fn clear(at: &Path, called: &str) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let theirs = called.is_empty() && name.starts_with("cnp_");

        if name.starts_with(called) && !theirs {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Which step is which, in the order they ran.
fn named(events: &[Event]) -> Vec<(usize, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::StepStarted { index, name, .. } => Some((*index, name.clone())),
            _ => None,
        })
        .collect()
}

/// Where one step's log is kept, named after the step so a folder reads as the run did.
fn log(at: &Path, called: &str, of: usize, step: &str) -> PathBuf {
    let sanitised: String = step
        .chars()
        .map(|letter| match letter.is_alphanumeric() {
            true => letter.to_ascii_lowercase(),
            false => '-',
        })
        .collect();

    let step = sanitised.trim_matches('-').replace("--", "-");
    at.join(format!("{called}{:02}-{step}.log", of + 1))
}

/// What a runner says while it gets a container ready: the file it builds from, the
/// commands it runs, what the build says as it goes, and the image it comes out with.
fn got_ready(line: &str) -> bool {
    let building = line.starts_with('#')
        && line[1..].starts_with(|it: char| it.is_ascii_digit())
        && !line.starts_with("##[");

    line.is_empty()
        || building
        || line.starts_with("Dockerfile for action:")
        || line.starts_with("##[command]/usr/bin/docker")
        || line.strip_prefix("sha256:").is_some_and(hexadecimal)
}

/// By the piece: what one run and the next say differently is a word of its own, part of
/// a path, or the tail of a name.
fn settled(line: &str) -> String {
    line.split(' ')
        .map(|word| word.split('/').map(piece).collect::<Vec<_>>().join("/"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An id given to something a run made, a digest of what it packed, or the port whatever
/// served it was told to listen on: none of them is the same twice.
fn piece(part: &str) -> String {
    const SHAPE: [usize; 5] = [8, 4, 4, 4, 12];
    const DIGEST: usize = 64;

    let id = part.rsplit('_').next().unwrap_or(part);
    let shape: Vec<&str> = id.split('-').collect();
    let shaped = shape.len() == SHAPE.len()
        && shape
            .iter()
            .enumerate()
            .all(|(at, piece)| piece.len() == SHAPE[at] && hexadecimal(piece));

    if shaped {
        return part.replace(id, "<id>");
    }

    if part.len() == DIGEST && hexadecimal(part) {
        return "<digest>".to_owned();
    }

    if let Some((host, port)) = part.rsplit_once(':')
        && host.ends_with("127.0.0.1")
        && !port.is_empty()
        && port.chars().all(|it| it.is_ascii_digit())
    {
        return format!("{host}:<port>");
    }

    part.to_owned()
}

fn hexadecimal(word: &str) -> bool {
    word.chars().all(|it| it.is_ascii_hexdigit())
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
        outcome.write(&expected(&self.path), "")
    }

    /// What canopy did, beside what the runner GitHub ships did, so the two can be read
    /// against each other rather than one difference at a time. Under a name of its own,
    /// since only what the runner did is kept.
    pub fn ours(&self, outcome: &Outcome) -> Result<(), String> {
        outcome.write(&expected(&self.path), "cnp_")
    }
}

fn expected(case: &Path) -> PathBuf {
    outputs(case)
}

/// Where a case's outcome is kept: a folder of its own, named after the case.
fn outputs(case: &Path) -> PathBuf {
    let testdata = testdata();
    let named = case
        .strip_prefix(&testdata)
        .unwrap_or(case)
        .with_extension("");

    testdata.with_file_name("testdata_outputs").join(named)
}

pub struct Case {
    pub name: String,
    pub artifacts: PathBuf,
    pub workspace: PathBuf,
    pub temp: PathBuf,
    /// The commit and branch the run is on, which is whatever the repository is on today,
    /// and what that commit was made for.
    pub sha: String,
    pub branch: String,
    pub said: String,
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
        // what another did somewhere else, on another commit of another branch.
        outcome.rewrite(&case.temp.display().to_string(), "$RUNNER_TEMP");
        outcome.rewrite(&case.workspace.display().to_string(), "$GITHUB_WORKSPACE");
        outcome.rewrite(&case.sha, "$GITHUB_SHA");
        outcome.rewrite(&case.branch, "$GITHUB_REF_NAME");
        outcome.rewrite(&case.said, "$GITHUB_COMMIT_MESSAGE");
        outcome.settle();

        outcome.write(&case.artifacts.join("outcome"), "")?;
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
            sha: planner.context().github.sha.clone(),
            branch: planner.context().github.ref_name.clone(),
            said: committed(&planner.context().github.event),
            workflow,
            plan,
            service_env: self.services.env(),
        })
    }
}

/// What the commit a run is on was made for, which is a new thing every commit and says
/// nothing about the run.
fn committed(event: &Payload) -> String {
    let Payload::Push(push) = event else {
        return String::new();
    };

    push.head_commit
        .as_ref()
        .map(|commit| commit.message.clone())
        .unwrap_or_default()
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
