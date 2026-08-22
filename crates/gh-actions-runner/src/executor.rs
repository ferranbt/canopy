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
use crate::report::{Event, Reporter, Stream};

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
    },
    Node {
        entrypoint: PathBuf,
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
            Self::Script { shell, script } => {
                let (program, mut args) = match shell.as_str() {
                    // `-e` is what the runner passes, so a failing line fails the step.
                    "bash" => ("bash", vec!["-e".to_owned()]),
                    "sh" => ("sh", Vec::new()),
                    "python" => ("python3", Vec::new()),
                    other => return Err(Error::Unsupported(format!("`shell: {other}`"))),
                };
                args.push(script.display().to_string());
                Ok((program.to_owned(), args))
            }
            Self::Node { entrypoint } => {
                Ok(("node".to_owned(), vec![entrypoint.display().to_string()]))
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
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub timeout: Option<Duration>,
    pub masks: Vec<String>,
}

impl ExecRequest {
    pub fn new(exec: Exec, cwd: impl Into<PathBuf>) -> Self {
        Self {
            exec,
            env: BTreeMap::new(),
            cwd: cwd.into(),
            timeout: None,
            masks: Vec::new(),
        }
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

pub trait Machine {
    fn start(&mut self, job: &PlannedJob, out: &mut dyn Reporter) -> Result<(), Error>;

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

        run_until(command, request.timeout, &request.masks, out)
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
            Exec::Script { .. } => self.script(request, out),
            Exec::Node { .. } => self.node(request, out),
            Exec::Container { .. } => self.container(request, out),
        }
    }
}

#[derive(Debug, Default)]
pub struct HostMachine;

impl Machine for HostMachine {
    fn start(&mut self, _job: &PlannedJob, _out: &mut dyn Reporter) -> Result<(), Error> {
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

pub fn run_streaming(command: Command, out: &mut dyn Reporter) -> Result<ExecResult, Error> {
    run_until(command, None, &[], out)
}

/// Killing reaches the program that was started — for a container that is the `docker exec`
/// rather than what it is running, which the container being torn down deals with.
pub fn run_until(
    mut command: Command,
    timeout: Option<Duration>,
    masks: &[String],
    out: &mut dyn Reporter,
) -> Result<ExecResult, Error> {
    let program = command.get_program().to_os_string();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .at(&program)?;
    let mut commands = Vec::new();
    let mut masks = masks.to_vec();

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
            Ok((Stream::Out, line)) => handle_line(&line, &mut commands, &mut masks, out),
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
        let seconds = timeout.unwrap_or_default().as_secs();
        out.report(Event::Progress {
            text: format!("timed out after {seconds}s"),
        });
    }

    Ok(ExecResult {
        status: ExecStatus {
            success: status.success() && !timed_out,
            code: status.code(),
        },
        commands,
    })
}

fn pump(
    source: Option<impl std::io::Read + Send + 'static>,
    stream: Stream,
    lines: std::sync::mpsc::Sender<(Stream, String)>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(source) = source else { return };
        for line in BufReader::new(source).lines().map_while(Result::ok) {
            if lines.send((stream, line)).is_err() {
                return;
            }
        }
    })
}

fn handle_line(
    line: &str,
    commands: &mut Vec<WorkflowCommand>,
    masks: &mut Vec<String>,
    out: &mut dyn Reporter,
) {
    let Some(command) = WorkflowCommand::parse(line) else {
        out.report(Event::StepOutput {
            stream: Stream::Out,
            line: hide(line, masks),
        });
        return;
    };

    if let WorkflowCommand::AddMask(secret) = &command
        && !secret.is_empty()
    {
        masks.push(secret.clone());
    }
    if let Some(event) = command.to_event() {
        out.report(match event {
            Event::Message { level, text } => Event::Message {
                level,
                text: hide(&text, masks),
            },
            other => other,
        });
    }
    commands.push(command);
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
