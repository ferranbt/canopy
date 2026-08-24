//! Where the steps of a job run.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use gh_actions_plan::PlannedJob;

use crate::commands::Command as WorkflowCommand;
use crate::error::{At, Error};
use crate::report::{Event, Level, Reporter, Stream};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Image {
    Registry(String),
    Dockerfile { path: PathBuf, context: PathBuf },
}

impl Image {
    pub fn tag(&self) -> String {
        match self {
            Self::Registry(image) => image.clone(),
            // Named after where it was built from, so the same action reuses its image.
            Self::Dockerfile { path, .. } => {
                let mut hasher = DefaultHasher::new();
                path.hash(&mut hasher);
                format!("canopy-action:{:x}", hasher.finish())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Exec {
    Script {
        shell: String,
        script: PathBuf,
        /// Whether the workflow asked for that shell or was given it for asking for none,
        /// which is the difference between `bash -e` and the stricter shell GitHub runs a
        /// step in when it names one.
        named: bool,
    },
    Node {
        entrypoint: PathBuf,
    },
    /// An image a step will want, got before any of them runs.
    Fetch {
        image: String,
    },
    Container {
        image: Image,
        entrypoint: Option<String>,
        args: Vec<String>,
        mounts: Vec<(PathBuf, PathBuf)>,
        workdir: PathBuf,
    },
}

impl Exec {
    pub fn to_command(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<(String, Vec<String>), Error> {
        match self {
            Self::Script {
                shell,
                script,
                named,
            } => {
                let switches = |switches: &[&str]| -> Vec<String> {
                    switches.iter().map(|it| (*it).to_owned()).collect()
                };
                // `-e` is what the runner passes, so a failing line fails the step. A shell
                // a step asked for by name is given more than that: bash is run without the
                // files it would read and with a failing half of a pipe failing the whole.
                let (program, mut args) = match (shell.as_str(), named) {
                    ("bash", true) => (
                        "bash",
                        switches(&["--noprofile", "--norc", "-e", "-o", "pipefail"]),
                    ),
                    ("bash", false) => ("bash", switches(&["-e"])),
                    ("sh", _) => ("sh", switches(&["-e"])),
                    ("python", _) => ("python3", Vec::new()),
                    (other, _) => return Err(Error::Unsupported(format!("`shell: {other}`"))),
                };
                args.push(script.display().to_string());
                Ok((program.to_owned(), args))
            }
            // TODO: run it on the node major the action asks for, which is what a runner
            // does from the copies it carries.
            Self::Node { entrypoint } => {
                Ok(("node".to_owned(), vec![entrypoint.display().to_string()]))
            }
            Self::Fetch { image } => {
                Ok(("docker".to_owned(), vec!["pull".to_owned(), image.clone()]))
            }
            Self::Container {
                image,
                entrypoint,
                args,
                mounts,
                workdir,
            } => {
                let mut line = vec![
                    "run".to_owned(),
                    "--rm".to_owned(),
                    "--workdir".to_owned(),
                    workdir.display().to_string(),
                ];
                for (host, inside) in mounts {
                    line.push("-v".to_owned());
                    line.push(format!("{}:{}", host.display(), inside.display()));
                }
                for (key, value) in env {
                    line.push("-e".to_owned());
                    line.push(format!("{key}={value}"));
                }
                if let Some(entrypoint) = entrypoint {
                    line.push("--entrypoint".to_owned());
                    line.push(entrypoint.clone());
                }
                line.push(image.tag());
                line.extend(args.iter().cloned());

                Ok(("docker".to_owned(), line))
            }
        }
    }

    pub fn build(&self) -> Option<(String, Vec<String>)> {
        let Self::Container {
            image: image @ Image::Dockerfile { path, context },
            ..
        } = self
        else {
            return None;
        };

        Some((
            "docker".to_owned(),
            vec![
                "build".to_owned(),
                "-q".to_owned(),
                "-t".to_owned(),
                image.tag(),
                "-f".to_owned(),
                path.display().to_string(),
                context.display().to_string(),
            ],
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub exec: Exec,
    pub name: String,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub timeout: Option<Duration>,
    pub masks: Vec<String>,
}

impl ExecRequest {
    pub fn new(exec: Exec, cwd: impl Into<PathBuf>) -> Self {
        Self {
            exec,
            name: String::new(),
            env: BTreeMap::new(),
            cwd: cwd.into(),
            timeout: None,
            masks: Vec::new(),
        }
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn masks(mut self, masks: Vec<String>) -> Self {
        self.masks = masks;
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn envs(mut self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env.extend(env);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecStatus {
    pub success: bool,
    pub code: Option<i32>,
}

impl Default for ExecStatus {
    fn default() -> Self {
        Self {
            success: true,
            code: Some(0),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExecResult {
    pub status: ExecStatus,
    pub commands: Vec<WorkflowCommand>,
}

/// How a machine came up for the job it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    Ready,
    /// The machine is there but something the job asked for is not, so its steps run and the
    /// job is failing before the first of them.
    Missing,
}

pub trait Machine {
    fn start(&mut self, job: &PlannedJob, out: &mut dyn Reporter) -> Result<Started, Error>;

    fn finish(&mut self) -> Result<(), Error>;

    fn run(
        &mut self,
        program: &str,
        args: &[String],
        request: &ExecRequest,
        out: &mut dyn Reporter,
    ) -> Result<ExecResult, Error> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(&request.env)
            .current_dir(&request.cwd);

        run_until(command, request, out)
    }

    fn script(
        &mut self,
        request: &ExecRequest,
        out: &mut dyn Reporter,
    ) -> Result<ExecResult, Error> {
        let (program, args) = request.exec.to_command(&request.env)?;
        self.run(&program, &args, request, out)
    }

    fn node(&mut self, request: &ExecRequest, out: &mut dyn Reporter) -> Result<ExecResult, Error> {
        let (program, args) = request.exec.to_command(&request.env)?;
        self.run(&program, &args, request, out)
    }

    fn container(
        &mut self,
        request: &ExecRequest,
        out: &mut dyn Reporter,
    ) -> Result<ExecResult, Error> {
        if let Some((program, args)) = request.exec.build() {
            out.report(Event::Progress {
                text: "[Building docker image]".to_owned(),
            });

            let built = self.run(&program, &args, request, out)?;
            if !built.status.success {
                return Ok(built);
            }
        }

        let (program, args) = request.exec.to_command(&request.env)?;
        self.run(&program, &args, request, out)
    }

    fn exec(&mut self, request: &ExecRequest, out: &mut dyn Reporter) -> Result<ExecResult, Error> {
        match &request.exec {
            Exec::Script { .. } | Exec::Fetch { .. } => self.script(request, out),
            Exec::Node { .. } => self.node(request, out),
            Exec::Container { .. } => self.container(request, out),
        }
    }
}

#[derive(Debug, Default)]
pub struct HostMachine;

impl Machine for HostMachine {
    fn start(&mut self, _job: &PlannedJob, _out: &mut dyn Reporter) -> Result<Started, Error> {
        Ok(Started::Ready)
    }

    fn finish(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

/// Killing reaches the program that was started — for a container that is the `docker exec`
/// rather than what it is running, which the container being torn down deals with.
pub fn run_until(
    mut command: Command,
    request: &ExecRequest,
    out: &mut dyn Reporter,
) -> Result<ExecResult, Error> {
    let program = command.get_program().to_os_string();
    // From the request rather than from what was spawned, since a machine that runs steps
    // somewhere else hands the environment over its own way.
    let switches = Switches::of(&request.env);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .at(&program)?;
    let timeout = request.timeout;
    let mut commands = Vec::new();
    let mut masks = request.masks.clone();
    let mut refused = false;
    let mut listening = Listening::default();

    let (lines, reader) = std::sync::mpsc::channel();
    let output = pump(child.stdout.take(), Stream::Out, lines.clone());
    let errors = pump(child.stderr.take(), Stream::Err, lines);

    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let mut timed_out = false;
    loop {
        let left = match deadline {
            None => Duration::from_secs(3600),
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
        };

        match reader.recv_timeout(left) {
            Ok((Stream::Out, line)) => {
                refused |= handle_line(
                    &line,
                    &switches,
                    &mut listening,
                    &mut commands,
                    &mut masks,
                    out,
                );
            }
            Ok((Stream::Err, line)) => out.report(Event::StepOutput {
                stream: Stream::Err,
                line: hide(&line, &masks),
            }),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    let _ = child.kill();
                    timed_out = true;
                    break;
                }
            }
        }
    }

    let status = child.wait().at(&program)?;
    let _ = output.join();
    let _ = errors.join();

    if timed_out {
        // Word for word what GitHub says, since a workflow may be reading the log for it.
        // In whole minutes, which is the only unit the field is given in, and what was left
        // of one is what a step was allowed.
        let minutes = timeout.unwrap_or_default().as_secs().div_ceil(60);
        out.report(Event::Message {
            level: Level::Error,
            text: format!(
                "The action '{}' has timed out after {minutes} minutes.",
                request.name
            ),
        });
    }

    Ok(ExecResult {
        status: ExecStatus {
            success: status.success() && !timed_out && !refused,
            code: status.code(),
        },
        commands,
    })
}

/// What a step may ask the runner for, which it says by setting it in its own environment.
struct Switches {
    /// `::debug::` is only shown when the run was asked for it.
    debug: bool,
    /// The commands GitHub took away, which a step has to opt back into by name.
    unsecure: bool,
}

impl Switches {
    fn of(env: &BTreeMap<String, String>) -> Self {
        let set = |name: &str| env.get(name).is_some_and(|value| !value.is_empty());

        Self {
            debug: set("RUNNER_DEBUG"),
            unsecure: set("ACTIONS_ALLOW_UNSECURE_COMMANDS"),
        }
    }
}

fn pump(
    source: Option<impl std::io::Read + Send + 'static>,
    stream: Stream,
    lines: std::sync::mpsc::Sender<(Stream, String)>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(source) = source else { return };
        let mut reader = BufReader::new(source);
        let mut read = Vec::new();

        loop {
            read.clear();
            // Read as the bytes they are: a step that prints something that is not text has
            // still printed it, and what it said is not thrown away for that.
            match reader.read_until(b'\n', &mut read) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            while let Some(b'\n' | b'\r') = read.last() {
                read.pop();
            }

            let line = String::from_utf8_lossy(&read).into_owned();
            if lines.send((stream, line)).is_err() {
                return;
            }
        }
    })
}

/// What a step has asked the runner to make of the lines that follow.
#[derive(Debug, Default)]
struct Listening {
    /// The token a step said would put the runner back to listening for commands.
    stopped: Option<String>,
    /// Whether a command is written to the log as well as acted on.
    echo: bool,
}

/// Whether the step is to fail for having asked for a command it may not have.
fn handle_line(
    line: &str,
    switches: &Switches,
    listening: &mut Listening,
    commands: &mut Vec<WorkflowCommand>,
    masks: &mut Vec<String>,
    out: &mut dyn Reporter,
) -> bool {
    let printed = |out: &mut dyn Reporter, masks: &Vec<String>| {
        out.report(Event::StepOutput {
            stream: Stream::Out,
            line: hide(line, masks),
        });
    };

    // Until the token comes back, everything a step says is only what it said.
    if let Some(token) = &listening.stopped {
        if line.trim() == format!("::{token}::") {
            listening.stopped = None;
        }
        printed(out, masks);
        return false;
    }

    let Some(command) = WorkflowCommand::parse(line) else {
        printed(out, masks);
        return false;
    };

    let taken_away = match &command {
        WorkflowCommand::AddPath(_) => Some("add-path"),
        WorkflowCommand::SetEnv { .. } => Some("set-env"),
        _ => None,
    };
    if let Some(name) = taken_away.filter(|_| !switches.unsecure) {
        for text in refusal(line.trim(), name) {
            out.report(Event::Message {
                level: Level::Error,
                text,
            });
        }
        return true;
    }

    if let WorkflowCommand::Stop(token) = &command {
        // Kept out of the log from here on, so nothing a step prints can pass for the token
        // that would have the runner listening again.
        masks.push(token.clone());
        listening.stopped = Some(token.clone());
        printed(out, masks);
        return false;
    }

    if let WorkflowCommand::Echo(on) = &command {
        if listening.echo {
            printed(out, masks);
        }
        listening.echo = *on;
        return false;
    }

    if let WorkflowCommand::AddMask(secret) = &command
        && !secret.is_empty()
    {
        masks.push(secret.clone());
    }
    if let Some(event) = command.to_event() {
        let hidden = match event {
            Event::Message { level, text } => Event::Message {
                level,
                text: hide(&text, masks),
            },
            other => other,
        };

        let quiet = matches!(
            hidden,
            Event::Message {
                level: Level::Debug,
                ..
            }
        ) && !switches.debug;
        if !quiet {
            out.report(hidden);
        }
    }
    commands.push(command);

    false
}

/// What GitHub says when a step asks for a command it took away, word for word, since a
/// workflow may well be reading the log for it.
fn refusal(line: &str, name: &str) -> [String; 2] {
    [
        format!("Unable to process command '{line}' successfully."),
        format!(
            "The `{name}` command is disabled. Please upgrade to using Environment Files or opt \
             into unsecure command execution by setting the `ACTIONS_ALLOW_UNSECURE_COMMANDS` \
             environment variable to `true`. For more information see: \
             https://github.blog/changelog/2020-10-01-github-actions-deprecating-set-env-and-add-path-commands/"
        ),
    ]
}

/// Only ever catches an accident: a step that means to print a secret can take it apart
/// first. The same best effort GitHub makes, and worth about as much.
fn hide(line: &str, masks: &[String]) -> String {
    let mut line = line.to_owned();
    for secret in masks {
        if !secret.is_empty() {
            line = line.replace(secret.as_str(), "***");
        }
    }

    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Collected;

    fn script(name: &str, body: &str) -> ExecRequest {
        let path = std::env::temp_dir().join(format!("canopy-exec-{name}"));
        std::fs::write(&path, body).expect("the script is written");

        ExecRequest::new(
            Exec::Script {
                shell: "sh".to_owned(),
                script: path,
                named: true,
            },
            std::env::temp_dir(),
        )
    }

    #[test]
    fn the_host_runs_a_command_and_reports_how_it_went() {
        let mut machine = HostMachine;
        let request = script("how-it-went", "echo hello; exit 3");

        let result = machine
            .exec(&request, &mut Collected::default())
            .expect("runs");

        assert!(!result.status.success);
        assert_eq!(result.status.code, Some(3));
    }

    #[test]
    fn masked_values_are_hidden_wherever_they_appear() {
        let masks = vec!["hunter2".to_owned(), "s3cret".to_owned()];

        assert_eq!(hide("token=hunter2", &masks), "token=***");
        assert_eq!(hide("a hunter2 b hunter2", &masks), "a *** b ***");
        assert_eq!(hide("both hunter2 and s3cret", &masks), "both *** and ***");
        assert_eq!(hide("nothing to hide", &masks), "nothing to hide");
    }

    #[test]
    fn an_empty_mask_would_hide_everything_so_it_is_refused() {
        assert_eq!(hide("untouched", &["".to_owned()]), "untouched");
    }

    #[test]
    fn a_mask_hides_what_the_rest_of_the_step_prints() {
        let mut machine = HostMachine;
        let request = script(
            "add-mask",
            "echo '::add-mask::hunter2'; echo 'password is hunter2'",
        );

        let result = machine
            .exec(&request, &mut Collected::default())
            .expect("runs");

        assert!(
            result
                .commands
                .contains(&WorkflowCommand::AddMask("hunter2".to_owned()))
        );
    }

    #[test]
    fn a_step_that_was_told_a_secret_hides_it_from_the_start() {
        let mut machine = HostMachine;
        let request =
            script("told-a-secret", "echo 'leaking hunter2'").masks(vec!["hunter2".to_owned()]);

        machine
            .exec(&request, &mut Collected::default())
            .expect("runs");
    }

    #[test]
    fn workflow_commands_are_kept_and_ordinary_output_is_not() {
        let mut machine = HostMachine;
        let request = script("commands", "echo plain; echo '::set-output name=x::1'");

        let result = machine
            .exec(&request, &mut Collected::default())
            .expect("runs");

        assert_eq!(result.commands.len(), 1);
        assert_eq!(
            result.commands[0],
            WorkflowCommand::SetOutput {
                name: "x".to_owned(),
                value: "1".to_owned(),
            }
        );
    }
}
