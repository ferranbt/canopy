//! The containers a job is run with: the one its steps run in, where it asked for one, and
//! the services that run alongside it.
//!
//! Nothing here decides which a job gets. A machine does that, and asks for what it wants.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use gh_actions_plan::PlannedJob;
use gh_actions_spec::{Container, ContainerSettings, OneOrMany, RunsOn, Scalar};

use crate::error::{At, Error};
use crate::executor::{ExecRequest, ExecResult, Started, run_until};
use crate::report::{Event, Level, Reporter};

/// The image a `runs-on` label is run in, for a machine that has to pick one.
pub type Images = BTreeMap<String, String>;

#[derive(Debug, Default)]
pub struct Containers {
    images: Images,
    mounts: Vec<PathBuf>,
    current: Option<String>,
    services: Vec<String>,
    /// Where a program turned out to be inside the container of this job.
    found: BTreeMap<String, String>,
}

impl Containers {
    pub fn new(images: Images, mounts: Vec<PathBuf>) -> Self {
        Self {
            images,
            mounts,
            current: None,
            services: Vec::new(),
            found: BTreeMap::new(),
        }
    }

    /// Whether a container of its own is running for this job.
    pub fn running(&self) -> bool {
        self.current.is_some()
    }

    /// The image a job would be run in: what it named, or what its `runs-on` maps to.
    pub fn image_for(&self, job: &PlannedJob) -> Option<String> {
        // A job that names a `container:` gets it whatever its `runs-on` says.
        if let Some(image) = container_image(job) {
            return Some(image.to_owned());
        }

        let labels = match job.spec.runs_on.as_ref()? {
            RunsOn::Labels(labels) => labels.as_slice(),
            RunsOn::Group(group) => group.labels.as_ref().map(OneOrMany::as_slice)?,
        };

        labels
            .iter()
            .find_map(|label| self.images.get(label).cloned())
    }

    /// What a machine says when a job asks for a label it has no image for.
    pub fn no_image_for(&self, job: &PlannedJob) -> Error {
        Error::Unsupported(format!(
            "no image for the `runs-on` of job {:?}; images are known for: {}",
            job.id,
            self.images.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    }

    /// They share this machine's network, so each is reached at its workflow name, as
    /// GitHub promises. Nothing waits for one to be ready; a step that needs one should.
    ///
    /// One that will not start does not stop the job: its steps run, and the job is failing
    /// before the first of them, which is what GitHub does with it.
    pub fn start_services(
        &mut self,
        job: &PlannedJob,
        out: &mut dyn Reporter,
    ) -> Result<(Vec<String>, Started), Error> {
        let mut names = Vec::new();
        let mut started = Started::Ready;
        let Some(services) = &job.spec.services else {
            return Ok((names, started));
        };

        for (label, container) in services {
            let image = image_of(container);
            out.report(Event::Progress {
                text: format!("starting service {label} ({image})"),
            });

            let mut command = Command::new("docker");
            command.args(["run", "--rm", "--detach", "--network", "host"]);
            if let Some(settings) = settings_of(container) {
                // Sharing the network already puts a service where it says it is, so a
                // port asked for under the number it listens on is where it will be. One
                // asked for under another number is not, and nothing here can move it.
                for moved in remapped(settings.ports.as_deref().unwrap_or_default()) {
                    out.report(Event::Message {
                        level: Level::Warning,
                        text: format!("`{moved}` on service {label} cannot be honoured here"),
                    });
                }
                apply(&mut command, settings);
            }
            command.arg(image);

            let output = command.output().at("docker")?;
            if !output.status.success() {
                out.report(Event::Message {
                    level: Level::Error,
                    text: format!(
                        "cannot start service {label} ({image}): {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                });
                started = Started::Missing;
                continue;
            }

            let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            self.services.push(id.clone());

            // A name belongs to a service that is there to answer to it: one that has already
            // stopped is not reached under it, the way it would not be on a network of its own.
            if !alive(&id) {
                out.report(Event::Message {
                    level: Level::Warning,
                    text: format!("service {label} ({image}) stopped as soon as it was started"),
                });
                continue;
            }

            names.push(label.clone());
        }

        Ok((names, started))
    }

    /// One container per job, started before its first step and removed after its last, with
    /// every step a `docker exec` into it. One rather than one per step, because a service a
    /// step starts has to still be running for the next.
    pub fn start_job(
        &mut self,
        job: &PlannedJob,
        image: &str,
        services: &[String],
        out: &mut dyn Reporter,
    ) -> Result<(), Error> {
        out.report(Event::Progress {
            text: format!("starting {image}"),
        });

        let mut command = Command::new("docker");
        command.args(["run", "--rm", "--detach", "--network", "host"]);
        command.args(["--entrypoint", "tail"]);

        for mount in &self.mounts {
            // docker would create a missing one as root, which the runner cannot write.
            std::fs::create_dir_all(mount).at(mount)?;
            // At the path it already has, so both sides agree on every path.
            command
                .arg("--volume")
                .arg(format!("{0}:{0}", mount.display()));
        }
        // Container actions run docker themselves, so they need to reach the daemon.
        if std::path::Path::new("/var/run/docker.sock").exists() {
            command
                .arg("--volume")
                .arg("/var/run/docker.sock:/var/run/docker.sock");
        }
        for label in services {
            command.arg("--add-host").arg(format!("{label}:127.0.0.1"));
        }
        // What the job asked for goes on last, so it can override any of the above.
        if let Some(settings) = container_settings(job) {
            apply(&mut command, settings);
        }
        command.arg(image).args(["-f", "/dev/null"]);

        let output = command.output().at("docker")?;
        if !output.status.success() {
            return Err(Error::Plan(format!(
                "cannot start {image}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        self.current = Some(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        Ok(())
    }

    /// A step run inside the container this job was given. A container action therefore runs
    /// through the daemon this one was handed, which is what the socket is mounted for.
    pub fn exec(
        &mut self,
        program: &str,
        args: &[String],
        request: &ExecRequest,
        out: &mut dyn Reporter,
    ) -> Result<ExecResult, Error> {
        let id = self
            .current
            .as_ref()
            .ok_or_else(|| Error::Plan("no container is running for this job".to_owned()))?;

        let mut command = Command::new("docker");
        command.args(["exec", "--workdir", &request.cwd.display().to_string()]);
        for (key, value) in &request.env {
            command.arg("--env").arg(format!("{key}={value}"));
        }
        command.arg(id).arg(program).args(args);

        run_until(command, request, out)
    }

    /// Looked for inside the container the steps run in, since that is the only place it
    /// could be found, and remembered so a job asks the once.
    pub fn found(&mut self, program: &str) -> String {
        if let Some(found) = self.found.get(program) {
            return found.clone();
        }

        let Some(id) = self.current.clone() else {
            return program.to_owned();
        };
        let looked = Command::new("docker")
            .args(["exec", &id, "sh", "-c"])
            .arg(format!("command -v {program}"))
            .output();

        let found = looked
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|found| !found.is_empty())
            .unwrap_or_else(|| program.to_owned());

        self.found.insert(program.to_owned(), found.clone());
        found
    }

    /// Everything this job was given, taken away again.
    pub fn remove(&mut self) -> Result<(), Error> {
        for id in self
            .current
            .take()
            .into_iter()
            .chain(self.services.drain(..))
        {
            let _ = Command::new("docker").args(["rm", "--force", &id]).output();
        }
        self.found.clear();

        Ok(())
    }
}

impl Drop for Containers {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

fn alive(id: &str) -> bool {
    let looked = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Running}}", id])
        .output();

    looked.is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "true")
}

fn image_of(container: &Container) -> &str {
    match container {
        Container::Image(image) => image,
        Container::Settings(settings) => &settings.image,
    }
}

/// The ports a service asked to be reachable at, where that is not where it listens.
fn remapped(ports: &[Scalar]) -> Vec<String> {
    ports
        .iter()
        .map(scalar)
        .filter(|port| match port.split_once(':') {
            Some((outside, inside)) => outside != inside,
            None => false,
        })
        .collect()
}

fn settings_of(container: &Container) -> Option<&ContainerSettings> {
    match container {
        Container::Settings(settings) => Some(settings),
        Container::Image(_) => None,
    }
}

fn container_image(job: &PlannedJob) -> Option<&str> {
    job.spec.container.as_ref().map(image_of)
}

fn container_settings(job: &PlannedJob) -> Option<&ContainerSettings> {
    job.spec.container.as_ref().and_then(settings_of)
}

fn apply(command: &mut Command, settings: &ContainerSettings) {
    for (name, value) in settings.env.iter().flatten() {
        command
            .arg("--env")
            .arg(format!("{name}={}", scalar(value)));
    }
    for volume in settings.volumes.iter().flatten() {
        command.arg("--volume").arg(volume);
    }
    if let Some(options) = &settings.options {
        command.args(options.split_whitespace());
    }
}

fn scalar(value: &Scalar) -> String {
    match value {
        Scalar::String(text) => text.clone(),
        Scalar::Bool(value) => value.to_string(),
        Scalar::Int(value) => value.to_string(),
        Scalar::Float(value) => value.to_string(),
    }
}
