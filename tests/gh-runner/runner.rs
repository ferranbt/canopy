use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;

use gh_actions_context::{Conclusion, RunContext};
use gh_actions_listener::client::types::Record;
use gh_actions_plan::PlannedJob;
use gh_actions_runner::report::{Event, Reporter, Stream};
use gh_actions_spec::Workflow;

use crate::message;
use crate::service::{self, Listening, Service, Update};

const IMAGE: &str = "gh-runner";
const CONTAINER: &str = "gh-runner";
const WORK: &str = "/home/runner/_work";
const WORKSPACE: &str = "/home/runner/_work/canopy/canopy";

pub struct Job<'a> {
    pub workflow: &'a Workflow,
    pub planned: &'a PlannedJob,
    pub context: &'a RunContext,
    pub services: &'a BTreeMap<String, String>,
}

pub struct GhRunner {
    service: Service,
    /// No two jobs may look alike to a runner.
    jobs: AtomicU64,
    workspace: PathBuf,
    _listening: Listening,
}

impl GhRunner {
    pub fn new() -> Result<Self, String> {
        let service = Service::default();
        let listening = service.start()?;
        let workspace = PathBuf::from(WORKSPACE);

        std::fs::create_dir_all(&workspace).map_err(|err| format!("cannot make a mount: {err}"))?;

        Ok(Self {
            service,
            jobs: AtomicU64::new(0),
            workspace,
            _listening: listening,
        })
    }

    /// Once per case rather than once per job: what a job leaves behind is what the next
    /// one finds, the same as a run on one machine.
    pub fn place(&self, case: &std::path::Path) -> Result<(), String> {
        for entry in std::fs::read_dir(&self.workspace)
            .map_err(|err| format!("cannot read a mount: {err}"))?
            .flatten()
        {
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

    /// A container per job: a runner takes one job and stops, which is the only thing it
    /// does without being asked to wait for work.
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
        self.service.hand_over(message.to_string(), updates);

        let mut agent = start()?;
        agent.wait()?;

        // Read once the runner is gone: what it says is only true of the job when the job
        // is over, so there is nothing to piece together.
        report(arriving.try_iter().collect(), &job.planned.id, out)
    }
}

fn report(updates: Vec<Update>, id: &str, out: &mut dyn Reporter) -> Result<(), String> {
    let mut written: BTreeMap<String, Record> = BTreeMap::new();
    let mut uploads: HashMap<String, String> = HashMap::new();
    let mut came = None;

    for update in updates {
        match update {
            Update::Records(records) => {
                for record in records {
                    written
                        .entry(record.id.clone())
                        .or_insert_with(|| Record {
                            id: record.id.clone(),
                            ..Record::default()
                        })
                        .update(record);
                }
            }
            Update::Log { step, text } => {
                uploads.insert(step, text);
            }
            Update::Ended { result, outputs } => came = Some((result, outputs)),
        }
    }

    let mut records: Vec<Record> = written.into_values().collect();
    records.sort_by_key(|record| record.order);

    let job = records.iter().find(|record| !record.is_step());
    let label = job.map(|job| job.name.clone()).unwrap_or_default();

    out.report(Event::JobStarted {
        id: id.to_owned(),
        label: label.clone(),
    });

    let mut ran = 0;
    for record in records.iter().filter(|record| record.is_step()) {
        let Some(name) = named(record).filter(|_| record.finished()) else {
            continue;
        };

        let index = ran;
        ran += 1;
        out.report(Event::StepStarted {
            index,
            name: name.clone(),
            depth: 0,
        });

        let said = uploads.remove(&record.id).unwrap_or_default();
        said.lines()
            .map(|line| clean(&without_timestamp(line)))
            .filter(|line| !CHATTER.iter().any(|chatter| line.starts_with(chatter)))
            .for_each(|line| {
                out.report(Event::StepOutput {
                    stream: Stream::Out,
                    line,
                });
            });

        out.report(Event::StepFinished {
            index,
            name,
            depth: 0,
            conclusion: conclusion(record.result.as_deref()),
            code: None,
        });
    }

    // A job that ran in a container of its own never has its record written down as over:
    // the timeline is left saying the runner was taking the container down, and what the
    // job came to is only in what it said at the end.
    let over = match job.filter(|job| job.finished()) {
        Some(job) => job.result.clone(),
        None => came.as_ref().map(|(result, _)| result.clone()),
    };
    let ended = conclusion(over.as_deref());
    out.report(Event::JobFinished {
        id: id.to_owned(),
        label,
        conclusion: ended,
    });

    if let Some((_, outputs)) = came.filter(|(_, outputs)| !outputs.is_empty()) {
        out.report(Event::JobOutputs {
            id: id.to_owned(),
            outputs,
        });
    }

    // A job that failed before its first step is a job that ran; one that says it went well
    // without running anything is a job the runner never picked up.
    match (ran, ended) {
        (0, Conclusion::Success) => Err("the runner ran no step".to_owned()),
        _ => Ok(()),
    }
}

/// What a runner puts against a step's log although no step printed it.
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

fn named(record: &Record) -> Option<String> {
    let hook = record.name.starts_with("Pre ") || record.name.starts_with("Post ");

    (record.id.starts_with(message::STEPS) || hook).then(|| record.name.clone())
}

fn without_timestamp(line: &str) -> String {
    match line.split_once(' ') {
        Some((stamp, rest)) if stamp.ends_with('Z') && stamp.contains('T') => rest.to_owned(),
        _ => line.to_owned(),
    }
}

struct Agent {
    child: std::process::Child,
}

impl Agent {
    fn wait(&mut self) -> Result<(), String> {
        self.child
            .wait()
            .map(|_| ())
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

        let _ = self.child.wait();
    }
}

fn start() -> Result<Agent, String> {
    let child = Command::new("docker")
        .args(["run", "--rm", "--network", "host"])
        .args(["--name", CONTAINER])
        // At the same path inside as out, since a runner starting a container of its own
        // hands the daemon the paths it sees, and the daemon looks for them out here.
        .args(["--volume", &format!("{WORK}:{WORK}")])
        .args(["--volume", "/var/run/docker.sock:/var/run/docker.sock"])
        .arg("--entrypoint")
        .arg("/bin/bash")
        .arg(IMAGE)
        .arg("-c")
        .arg(format!(
            "./config.sh --unattended --url {} --token canopy \
             --name canopy --labels canopy --work _work >/dev/null && ./run.sh --once",
            service::BASE
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("cannot run docker: {err}"))?;

    Ok(Agent { child })
}

fn clean(line: &str) -> String {
    line.replace(WORKSPACE, "$GITHUB_WORKSPACE")
        .replace("/home/runner/_work/_temp", "$RUNNER_TEMP")
        .replace("/home/runner/_work", "$RUNNER_WORK")
}

fn conclusion(result: Option<&str>) -> Conclusion {
    match result {
        Some("succeeded") => Conclusion::Success,
        Some("skipped") => Conclusion::Skipped,
        _ => Conclusion::Failure,
    }
}
