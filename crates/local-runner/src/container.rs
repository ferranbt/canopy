//! Running a job's steps inside a container, whether it asked for one or not.
//!
//! Nothing a workflow says is ever run on this machine: a job that named no `container:` is
//! given the image its `runs-on` maps to, and every step is a `docker exec` into it.

use std::path::PathBuf;

use gh_actions_plan::PlannedJob;
use gh_actions_runner::report::Reporter;
use gh_actions_runner::{Containers, Error, ExecRequest, ExecResult, Machine, Started};

pub use gh_actions_runner::Images;

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

pub struct InContainers {
    containers: Containers,
}

impl InContainers {
    pub fn new(images: Images, mounts: Vec<PathBuf>) -> Self {
        Self {
            containers: Containers::new(images, mounts),
        }
    }
}

impl Machine for InContainers {
    fn start(&mut self, job: &PlannedJob, out: &mut dyn Reporter) -> Result<Started, Error> {
        let (services, started) = self.containers.start_services(job, out)?;

        let image = self
            .containers
            .image_for(job)
            .ok_or_else(|| self.containers.no_image_for(job))?;

        self.containers.start_job(job, &image, &services, out)?;
        Ok(started)
    }

    fn run(
        &mut self,
        program: &str,
        args: &[String],
        request: &ExecRequest,
        out: &mut dyn Reporter,
    ) -> Result<ExecResult, Error> {
        self.containers.exec(program, args, request, out)
    }

    fn found(&mut self, program: &str) -> String {
        self.containers.found(program)
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.containers.remove()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh_actions_spec::{OneOrMany, RunsOn};
    use std::collections::BTreeMap;

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

    fn local() -> InContainers {
        InContainers::new(default_images(), Vec::new())
    }

    #[test]
    fn a_known_label_picks_its_image() {
        let image = local().containers.image_for(&job(&["ubuntu-latest"]));

        assert_eq!(image.as_deref(), Some("catthehacker/ubuntu:act-latest"));
    }

    #[test]
    fn a_label_with_no_image_is_refused_rather_than_run_here() {
        let mut local = local();

        let error = local
            .start(
                &job(&["macos-latest"]),
                &mut gh_actions_runner::Collected::default(),
            )
            .expect_err("should refuse");
        assert!(format!("{error}").contains("no image"));
    }

    #[test]
    fn extra_labels_do_not_stop_a_known_one_matching() {
        let image = local()
            .containers
            .image_for(&job(&["self-hosted", "ubuntu-latest"]));

        assert!(image.is_some());
    }
}
