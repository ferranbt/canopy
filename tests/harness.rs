use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gh_actions_context::Conclusion;
use gh_actions_plan::Plan;
use gh_actions_runner::report::{Event, Reporter, Stream};
use gh_actions_services::Services;
use gh_actions_spec::Workflow;
use local_runner::{Config, Local};

#[derive(Debug, Default)]
pub struct Outcome {
    pub events: Vec<Event>,
    pub logs: Vec<Printed>,
    inside: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Printed {
    pub step: String,
    pub lines: Vec<String>,
}

impl Reporter for Outcome {
    fn report(&mut self, event: Event) {
        // Only the groups a workflow asked for, which a runner names in brackets.
        if let Event::Progress { text } = &event
            && !text.starts_with('[')
        {
            return;
        }

        // What the node an action runs on says about itself, not what the action said.
        if let Event::StepOutput { line, .. } = &event
            && (line.contains("DeprecationWarning:") || line.starts_with("(Use `node"))
        {
            return;
        }

        if let Event::StepOutput { line, .. } = &event
            && got_ready(line)
        {
            return;
        }

        // A message of several lines reaches the log of the runner GitHub ships as the first
        // of them under the level it was raised at, and the rest as what the step printed.
        if let Event::Message { level, text } = &event
            && let Some((first, rest)) = text.split_once('\n')
        {
            let (level, rest) = (*level, rest.to_owned());
            self.report(Event::Message {
                level,
                text: first.to_owned(),
            });
            for line in rest.lines() {
                self.report(Event::StepOutput {
                    stream: Stream::Out,
                    line: line.to_owned(),
                });
            }
            return;
        }

        // What a runner says between steps goes to the job's own log rather than a step's,
        // and the runner GitHub ships keeps that to itself.
        let between = matches!(&event, Event::Message { .. } | Event::StepOutput { .. });
        if between && !self.inside {
            return;
        }

        // The runner GitHub ships keeps a composite action's inner steps to itself, so only
        // their boundaries go: what they printed is the step's all the same.
        let nested = match &event {
            Event::StepStarted { depth, .. } | Event::StepFinished { depth, .. } => *depth > 0,
            _ => false,
        };
        if nested {
            return;
        }

        match event {
            Event::StepOutput { line, .. } => {
                if let Some(printed) = self.logs.last_mut() {
                    printed.lines.push(line.trim_end().to_owned());
                }
            }
            // What a step that failed came back with is only in the log of the runner GitHub
            // ships when a shell was what ran, so it is not something to hold either to.
            Event::StepFinished {
                index,
                name,
                depth,
                conclusion: Conclusion::Failure,
                ..
            } => {
                self.inside = false;
                self.events.push(Event::StepFinished {
                    index,
                    name,
                    depth,
                    conclusion: Conclusion::Failure,
                    code: None,
                });
            }
            Event::StepStarted { name, .. } => {
                self.inside = true;
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
            event => {
                self.inside &= !matches!(event, Event::StepFinished { .. });
                self.events.push(event);
            }
        }
    }
}

impl Outcome {
    pub fn rewrite(&mut self, from: &str, name: &str) {
        let path = from.to_owned();

        for said in self.said() {
            *said = said.replace(&path, name);
        }
    }

    /// What no two runs agree on and neither is wrong about.
    pub fn settle(&mut self) {
        for said in self.said() {
            *said = settled(&plain(said));

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

fn named(events: &[Event]) -> Vec<(usize, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::StepStarted { index, name, .. } => Some((*index, name.clone())),
            _ => None,
        })
        .collect()
}

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

/// What a runner says while it builds and starts a container, which no two runs say alike.
fn got_ready(line: &str) -> bool {
    let building = line.starts_with('#')
        && line[1..].starts_with(|it: char| it.is_ascii_digit())
        && !line.starts_with("##[");

    line.is_empty()
        || building
        || line.starts_with("Dockerfile for action:")
        || line.starts_with("Getting action download info")
        || line.starts_with("Download action repository")
        || line.starts_with("##[command]/usr/bin/docker")
        || line.strip_prefix("sha256:").is_some_and(hexadecimal)
}

// Remove color from the text
fn plain(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(start) = rest.find('\u{1b}') {
        plain.push_str(&rest[..start]);
        rest = match rest[start..].find('m') {
            Some(end) => &rest[start + end + 1..],
            None => "",
        };
    }
    plain.push_str(rest);

    plain
}

fn settled(line: &str) -> String {
    line.split(' ')
        .map(|word| word.split('/').map(piece).collect::<Vec<_>>().join("/"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn piece(part: &str) -> String {
    const DIGEST: usize = 64;

    // The script a `run:` step is handed is named by whichever runner wrote it.
    if let Some((name, rest)) = part.split_once(".sh")
        && (shaped_like_an_id(name) || name.starts_with("step-"))
    {
        return format!("<script>.sh{rest}");
    }

    let id = part.rsplit('_').next().unwrap_or(part);
    if shaped_like_an_id(id) {
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

fn shaped_like_an_id(word: &str) -> bool {
    const SHAPE: [usize; 5] = [8, 4, 4, 4, 12];

    let shape: Vec<&str> = word.split('-').collect();

    shape.len() == SHAPE.len()
        && shape
            .iter()
            .enumerate()
            .all(|(at, piece)| piece.len() == SHAPE[at] && hexadecimal(piece))
}

fn hexadecimal(word: &str) -> bool {
    word.chars().all(|it| it.is_ascii_hexdigit())
}

fn one(event: &Event) -> String {
    serde_json::to_string(event).unwrap_or_default()
}

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

    /// Beside what the runner GitHub ships did, under a name of its own: only what the
    /// runner did is kept.
    pub fn ours(&self, outcome: &Outcome) -> Result<(), String> {
        outcome.write(&expected(&self.path), "cnp_")
    }
}

fn expected(case: &Path) -> PathBuf {
    outputs(case)
}

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
    pub sha: String,
    pub branch: String,
    pub workflow: Workflow,
    pub plan: Plan,
    pub service_env: BTreeMap<String, String>,
    _services: Services,
}

pub struct Harness {
    artifacts: PathBuf,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new("")
    }
}

impl Harness {
    pub fn new(prefix: &str) -> Self {
        // Who committed and what they called it is the machine's to say, and a recording
        // made on one machine is read on another.
        unsafe {
            std::env::set_var("GITHUB_REF_NAME", "canopy-branch");
            std::env::set_var("GITHUB_ACTOR", "canopy");
            std::env::set_var("GITHUB_COMMIT_AUTHOR", "canopy");
            std::env::set_var("GITHUB_COMMIT_EMAIL", "canopy@example.com");
            std::env::set_var("GITHUB_COMMIT_MESSAGE", "a commit");
        }

        let at = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let artifacts = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts")
            .join(format!("{prefix}{at}"));

        Self { artifacts }
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

        outcome.rewrite(&case.temp.display().to_string(), "$RUNNER_TEMP");
        outcome.rewrite(&case.workspace.display().to_string(), "$GITHUB_WORKSPACE");
        outcome.rewrite(&case.sha, "$GITHUB_SHA");
        outcome.rewrite(&case.branch, "$GITHUB_REF_NAME");
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

        let services = Services::start(artifacts.join("services"))
            .map_err(|err| format!("cannot start the services: {err}"))?;

        Ok(Case {
            name,
            artifacts,
            workspace,
            temp,
            sha: planner.context().github.sha.clone(),
            branch: planner.context().github.ref_name.clone(),
            workflow,
            plan,
            service_env: services.env(),
            _services: services,
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
