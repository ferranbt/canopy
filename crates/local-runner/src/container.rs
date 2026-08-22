//! Running a job's steps inside a container.
//!
//! The same shape `act` uses: one container per job, started before its first step and
//! removed after its last, with every step a `docker exec` into it. One container rather
//! than one per step, because a service a step starts has to still be running for the next.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use gh_actions_plan::PlannedJob;
use gh_actions_runner::report::{Event, Level, Reporter};
use gh_actions_runner::{At, Error, ExecRequest, ExecResult, Machine, run_until};
use gh_actions_spec::{Container, ContainerSettings, OneOrMany, RunsOn, Scalar};

pub type Images = BTreeMap<String, String>;

/// What `act` uses: they carry the bash, git and node actions assume is already there.
pub fn default_images() -> Images {
    let ubuntu = "catthehacker/ubuntu:act-latest";

    [
        "ubuntu-latest",
        "ubuntu-24.04",
        "ubuntu-22.04",
        "ubuntu-20.04",
    ]
    .into_iter()
    .map(|label| (label.to_owned(), ubuntu.to_owned()))
    .collect()
}

pub struct Containers {
    images: Images,
    mounts: Vec<PathBuf>,
    current: Option<String>,
    services: Vec<String>,
}

impl Containers {
    pub fn new(images: Images, mounts: Vec<PathBuf>) -> Self {
        Self {
            images,
            mounts,
            current: None,
            services: Vec::new(),
        }
    }

    fn image_for(&self, job: &PlannedJob) -> Option<String> {
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

    /// They share this machine's network, so each is reached at its workflow name, as
    /// GitHub promises. Nothing waits for one to be ready; a step that needs one should.
    fn start_services(
        &mut self,
        job: &PlannedJob,
        out: &mut dyn Reporter,
    ) -> Result<Vec<String>, Error> {
        let mut names = Vec::new();
        let Some(services) = &job.spec.services else {
            return Ok(names);
        };

        for (label, container) in services {
            let image = image_of(container);
            out.report(Event::Progress {
                text: format!("starting service {label} ({image})"),
            });

            let mut command = Command::new("docker");
            command.args(["run", "--rm", "--detach", "--network", "host"]);
            if let Some(settings) = settings_of(container) {
                if settings.ports.is_some() {
                    // Sharing the network puts the port where the container left it.
                    out.report(Event::Message {
                        level: Level::Warning,
                        text: format!("`ports:` on service {label} does nothing here"),
                    });
                }
                apply(&mut command, settings);
            }
            command.arg(image);

            let output = command.output().at("docker")?;
            if !output.status.success() {
                return Err(Error::Plan(format!(
                    "cannot start service {label} ({image}): {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }

            self.services
                .push(String::from_utf8_lossy(&output.stdout).trim().to_owned());
            names.push(label.clone());
        }

        Ok(names)
    }

    fn remove_current(&mut self) -> Result<(), Error> {
        for id in self
            .current
            .take()
            .into_iter()
            .chain(self.services.drain(..))
        {
            let _ = Command::new("docker").args(["rm", "--force", &id]).output();
        }

        Ok(())
    }
}

impl Machine for Containers {
    fn start(&mut self, job: &PlannedJob, out: &mut dyn Reporter) -> Result<(), Error> {
        // Better than quietly running a `macos-latest` job on whatever this is.
        let image = self.image_for(job).ok_or_else(|| {
            Error::Unsupported(format!(
                "no image for the `runs-on` of job {:?}; images are known for: {}",
                job.id,
                self.images.keys().cloned().collect::<Vec<_>>().join(", ")
            ))
        })?;

        let services = self.start_services(job, out)?;

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
        for label in &services {
            command.arg("--add-host").arg(format!("{label}:127.0.0.1"));
        }
        // What the job asked for goes on last, so it can override any of the above.
        if let Some(settings) = container_settings(job) {
            apply(&mut command, settings);
        }
        command.arg(&image).args(["-f", "/dev/null"]);

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

    /// Every kind runs the same way here: inside the container this job was given.
    ///
    /// A container action therefore runs through the daemon this one was handed, which is
    /// what the socket is mounted for.
    fn run(
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

        run_until(command, request.timeout, &request.masks, out)
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.remove_current()
    }
}

impl Drop for Containers {
    fn drop(&mut self) {
        let _ = self.remove_current();
    }
}

fn image_of(container: &Container) -> &str {
    match container {
        Container::Image(image) => image,
        Container::Settings(settings) => &settings.image,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn job(labels: &[&str]) -> PlannedJob {
        PlannedJob {
            id: "build".to_owned(),
            label: "build".to_owned(),
            matrix: BTreeMap::new(),
            needs: Vec::new(),
            spec: gh_actions_spec::NormalJob {
                runs_on: Some(RunsOn::Labels(OneOrMany::Many(
                    labels.iter().map(|label| (*label).to_owned()).collect(),
                ))),
                ..gh_actions_spec::NormalJob::default()
            },
        }
    }

    fn containers() -> Containers {
        Containers::new(default_images(), Vec::new())
    }

    #[test]
    fn a_known_label_picks_its_image() {
        let image = containers().image_for(&job(&["ubuntu-latest"]));

        assert_eq!(image.as_deref(), Some("catthehacker/ubuntu:act-latest"));
    }

    #[test]
    fn a_label_with_no_image_is_refused_rather_than_run_here() {
        let mut containers = containers();

        let error = containers
            .start(
                &job(&["macos-latest"]),
                &mut gh_actions_runner::Collected::default(),
            )
            .expect_err("should refuse");
        assert!(format!("{error}").contains("no image"));
    }

    #[test]
    fn extra_labels_do_not_stop_a_known_one_matching() {
        let image = containers().image_for(&job(&["self-hosted", "ubuntu-latest"]));

        assert!(image.is_some());
    }
}
