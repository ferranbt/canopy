use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use gh_actions_context::{Conclusion, RunContext};
use gh_actions_listener::client::types::{Lines, Record};
use gh_actions_plan::PlannedJob;
use gh_actions_runner::report::{Event, Level, Reporter, Stream};
use gh_actions_spec::Workflow;

use crate::message;
use crate::service::{self, Listening, Service, Update};

const IMAGE: &str = "gh-runner";
const CONTAINER: &str = "gh-runner";
/// Where the runner looks for the repository, which is one place for every case in turn.
const WORKSPACE: &str = "/home/runner/_work/canopy/canopy";

pub struct Job<'a> {
    pub workflow: &'a Workflow,
    pub planned: &'a PlannedJob,
    pub context: &'a RunContext,
    pub services: &'a BTreeMap<String, String>,
}

pub struct GhRunner {
    service: Service,
    agent: Agent,
    /// How many jobs this runner has been given, since no two may look alike to it.
    jobs: AtomicU64,
    /// The one workspace the container has, which each case is copied into in turn.
    workspace: PathBuf,
    _listening: Listening,
}

impl GhRunner {
    pub fn new() -> Result<Self, String> {
        let service = Service::default();
        let listening = service.start()?;
        let workspace = std::env::temp_dir().join("canopy-gh-runner");

        std::fs::create_dir_all(&workspace).map_err(|err| format!("cannot make a mount: {err}"))?;
        let agent = start(&workspace)?;

        Ok(Self {
            service,
            agent,
            jobs: AtomicU64::new(0),
            workspace,
            _listening: listening,
        })
    }

    /// The case's files, where the one container can see them.
    ///
    /// Once per case rather than once per job: what a job leaves behind is what the next
    /// one finds, the same as a run on one machine.
    pub fn place(&self, case: &std::path::Path) -> Result<(), String> {
        let held = std::fs::read_dir(&self.workspace)
            .map_err(|err| format!("cannot read a mount: {err}"))?;

        // What is in the mount changes, never the mount itself: the container is holding it
        // open, and a directory put back in its place is not the one it is looking at.
        for entry in held.flatten() {
            let _ = match entry.path().is_dir() {
                true => std::fs::remove_dir_all(entry.path()),
                false => std::fs::remove_file(entry.path()),
            };
        }

        let copied = Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", case.display()))
            .arg(&self.workspace)
            .status()
            .map_err(|err| format!("cannot copy the workspace: {err}"))?;

        match copied.success() {
            true => Ok(()),
            false => Err("cannot copy the workspace".to_owned()),
        }
    }

    pub fn run(&self, job: Job<'_>, out: &mut dyn Reporter) -> Result<(), String> {
        let nth = self.jobs.fetch_add(1, Ordering::Relaxed) + 1;
        let message = message::encode(
            job.workflow,
            job.planned,
            job.context,
            job.services,
            service::BASE,
            nth,
        );
        let (updates, arriving) = channel();
        self.service.hand_over(message, updates);

        let mut reported = Reported::new(WORKSPACE);
        while !reported.done() {
            match arriving.recv_timeout(Duration::from_millis(100)) {
                Ok(update) => reported.take(update, out),
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if self.agent.finished()? {
                        return Err("the runner stopped".to_owned());
                    }
                }
            }
        }

        match reported.steps.is_empty() {
            true => Err("the runner ran no step".to_owned()),
            false => Ok(()),
        }
    }
}

struct Reported {
    workspace: String,
    job: Option<String>,
    steps: Vec<String>,
    finished: Vec<String>,
    waiting: HashMap<String, Vec<Event>>,
    codes: HashMap<String, i32>,
    /// The steps that have said something, which is what makes an upload worth reading.
    said: Vec<String>,
    /// The steps that ran out of time, which came back with no code of their own.
    killed: Vec<String>,
    /// The step that is over, kept open until nothing more can arrive for it.
    ending: Option<(usize, Record)>,
    echoing: bool,
}

impl Reported {
    fn new(workspace: &str) -> Self {
        Self {
            workspace: workspace.to_owned(),
            job: None,
            steps: Vec::new(),
            finished: Vec::new(),
            waiting: HashMap::new(),
            codes: HashMap::new(),
            said: Vec::new(),
            killed: Vec::new(),
            ending: None,
            echoing: false,
        }
    }

    fn done(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|job| self.finished.contains(job))
    }

    fn take(&mut self, update: Update, out: &mut dyn Reporter) {
        match update {
            Update::Records(records) => self.timeline(records, out),
            Update::Printed(lines) => self.printed(lines, out),
            // Only where nothing was said as it went: the upload is the same output, and
            // the two together would say everything twice.
            Update::Log { step, text } if !self.said.contains(&step) => {
                let lines = Lines {
                    value: text.lines().map(without_timestamp).collect(),
                    step_id: step,
                };
                self.printed(lines, out);
            }
            Update::Log { .. } => {}
        }
    }

    fn timeline(&mut self, mut records: Vec<Record>, out: &mut dyn Reporter) {
        records.sort_by_key(|record| record.order);

        let job = records.iter().find(|record| !record.is_step()).cloned();
        if let Some(job) = &job
            && self.job.is_none()
        {
            self.job = Some(job.id.clone());
            self.report(
                Event::JobStarted {
                    id: job.name.clone(),
                    label: job.name.clone(),
                },
                out,
            );
        }

        for record in records.iter().filter(|record| record.is_step()) {
            self.step(record, out);
        }

        if let Some(job) = job
            && job.finished()
            && !self.finished.contains(&job.id)
        {
            self.ended(out);
            self.finished.push(job.id.clone());
            self.report(
                Event::JobFinished {
                    id: job.name.clone(),
                    label: job.name.clone(),
                    conclusion: conclusion(job.result.as_deref()),
                },
                out,
            );
        }
    }

    fn step(&mut self, record: &Record, out: &mut dyn Reporter) {
        // A runner writes its whole timeline down before it starts, so a record only says a
        // step is under way once it stops being pending.
        let under_way = record.finished() || record.state == "inProgress";
        let Some(name) = named(record).filter(|_| under_way) else {
            return;
        };

        let index = match self.steps.iter().position(|id| *id == record.id) {
            Some(at) => at,
            None => {
                self.ended(out);
                self.steps.push(record.id.clone());
                let index = self.steps.len() - 1;
                self.report(
                    Event::StepStarted {
                        index,
                        name: name.clone(),
                        depth: 0,
                    },
                    out,
                );

                for event in self.waiting.remove(&record.id).unwrap_or_default() {
                    self.report(event, out);
                }
                index
            }
        };

        // Held rather than reported: a runner uploads what a step said after it has already
        // written down that the step is over, so the step is not closed until something
        // else happens.
        if record.finished() && !self.finished.contains(&record.id) {
            self.finished.push(record.id.clone());
            self.ending = Some((index, record.clone()));
        }
    }

    /// Closes the step that was waiting to be closed, now that nothing more can be said of it.
    fn ended(&mut self, out: &mut dyn Reporter) {
        let Some((index, record)) = self.ending.take() else {
            return;
        };

        let conclusion = conclusion(record.result.as_deref());
        // Only a code worth complaining about is said out loud, so a step that came back at
        // all and did not complain came back with nothing to say. One that was killed never
        // came back, and one that was skipped never went.
        let killed = self.killed.contains(&record.id);
        let code = match (self.codes.get(&record.id), conclusion) {
            (Some(code), _) => Some(*code),
            (None, Conclusion::Skipped) => None,
            (None, _) if killed => None,
            (None, _) => Some(0),
        };

        self.report(
            Event::StepFinished {
                index,
                name: named(&record).unwrap_or_else(|| record.name.clone()),
                depth: 0,
                conclusion,
                code,
            },
            out,
        );
    }

    fn printed(&mut self, lines: Lines, out: &mut dyn Reporter) {
        if self.job.as_ref().is_some_and(|job| *job == lines.step_id) {
            return;
        }

        let mut said = Vec::new();
        for line in &lines.value {
            let line = clean(line, &self.workspace);
            if CHATTER.iter().any(|chatter| line.starts_with(chatter)) {
                continue;
            }
            if line.starts_with("##[group]Run ") {
                self.echoing = true;
            }
            if self.echoing {
                self.echoing = line != "##[endgroup]";
                continue;
            }

            if line.contains("has timed out after") {
                self.killed.push(lines.step_id.clone());
            }

            match reported(&line) {
                Said::Event(event) => said.push(event),
                Said::Code(code) => {
                    self.codes.insert(lines.step_id.clone(), code);
                }
                Said::Nothing => {}
            }
        }

        let ending = self
            .ending
            .as_ref()
            .is_some_and(|(_, record)| record.id == lines.step_id);

        if !said.is_empty() {
            self.said.push(lines.step_id.clone());
        }

        if self.steps.contains(&lines.step_id) && (ending || !self.finished.contains(&lines.step_id))
        {
            for event in said {
                self.report(event, out);
            }
            return;
        }

        self.waiting.entry(lines.step_id).or_default().extend(said);
    }

    fn report(&mut self, event: Event, out: &mut dyn Reporter) {
        out.report(event);
    }
}

enum Said {
    Event(Event),
    Code(i32),
    Nothing,
}

/// What a runner says about getting ready rather than about the step it is running. It
/// puts these against a step's log even though no step printed them, so they are the one
/// thing a step's output has to be read past.
const CHATTER: [&str; 8] = [
    "Post job cleanup.",
    "Prepare all required actions",
    "Prepare workflow directory",
    "Complete job name:",
    "Cleaning up orphan processes",
    "Current runner version:",
    "Runner name:",
    "Machine name:",
];

/// What a record is a step of ours by, and what to call it.
///
/// The steps we handed over carry the ids we gave them. A runner adds its own: `Set up job`
/// and `Complete job`, which canopy has no counterpart for and which are dropped, and the
/// `post` hook of every action that has one, which canopy does run and names the other way
/// round.
fn named(record: &Record) -> Option<String> {
    if record.id.starts_with(message::STEPS) {
        return Some(record.name.clone());
    }

    for (hook, phase) in [("Pre ", "pre"), ("Post ", "post")] {
        if let Some(name) = record.name.strip_prefix(hook) {
            return Some(format!("{name} ({phase})"));
        }
    }

    None
}

/// A line of an uploaded log, which is stamped with when it was printed.
fn without_timestamp(line: &str) -> String {
    match line.split_once(' ') {
        Some((stamp, rest)) if stamp.ends_with('Z') && stamp.contains('T') => rest.to_owned(),
        _ => line.to_owned(),
    }
}

fn reported(line: &str) -> Said {
    if let Some(code) = line
        .strip_prefix("##[error]Process completed with exit code ")
        .and_then(|code| code.trim_end_matches('.').parse().ok())
    {
        return Said::Code(code);
    }

    for (marker, level) in [
        ("##[debug]", Level::Debug),
        ("##[notice]", Level::Notice),
        ("##[warning]", Level::Warning),
        ("##[error]", Level::Error),
    ] {
        if let Some(text) = line.strip_prefix(marker) {
            return Said::Event(Event::Message {
                level,
                text: text.to_owned(),
            });
        }
    }

    if let Some(name) = line.strip_prefix("##[group]") {
        return Said::Event(Event::Progress {
            text: format!("[{name}]"),
        });
    }
    if line == "##[endgroup]" {
        return Said::Nothing;
    }

    Said::Event(Event::StepOutput {
        stream: Stream::Out,
        line: line.to_owned(),
    })
}

/// The one runner the suite is run on, which takes job after job until it is stopped.
struct Agent {
    child: Mutex<std::process::Child>,
}

impl Agent {
    fn finished(&self) -> Result<bool, String> {
        self.child
            .lock()
            .expect("the runner")
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|err| format!("cannot wait for the runner: {err}"))
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", CONTAINER])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let _ = self.child.lock().expect("the runner").wait();
    }
}

fn start(workspace: &std::path::Path) -> Result<Agent, String> {
    let child = Command::new("docker")
        .args(["run", "--rm", "--network", "host"])
        .args(["--name", CONTAINER])
        .arg("--volume")
        .arg(format!("{}:{WORKSPACE}", workspace.display()))
        .arg("--entrypoint")
        .arg("/bin/bash")
        .arg(IMAGE)
        .arg("-c")
        .arg(format!(
            "./config.sh --unattended --url {} --token canopy \
             --name canopy --labels canopy --work _work >/dev/null && ./run.sh",
            service::BASE
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("cannot run docker: {err}"))?;

    Ok(Agent {
        child: Mutex::new(child),
    })
}

fn clean(line: &str, workspace: &str) -> String {
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

    let plain = plain
        .replace(workspace, "$GITHUB_WORKSPACE")
        .replace("/home/runner/_work/_temp", "$RUNNER_TEMP")
        .replace("/home/runner/_work", "$RUNNER_WORK");

    ids(plain.trim_end())
}

fn ids(line: &str) -> String {
    const SHAPE: [usize; 5] = [8, 4, 4, 4, 12];

    line.split(' ')
        .map(|word| {
            let id = word.rsplit('_').next().unwrap_or(word);
            let parts: Vec<&str> = id.split('-').collect();

            let shaped = parts.len() == SHAPE.len()
                && parts.iter().enumerate().all(|(at, part)| {
                    part.len() == SHAPE[at] && part.chars().all(|it| it.is_ascii_hexdigit())
                });

            match shaped {
                true => word.replace(id, "<id>"),
                false => word.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn conclusion(result: Option<&str>) -> Conclusion {
    match result {
        Some("succeeded") => Conclusion::Success,
        Some("skipped") => Conclusion::Skipped,
        _ => Conclusion::Failure,
    }
}
