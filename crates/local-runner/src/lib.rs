//! The local flavour: the runner, plus stand-ins for the services GitHub would provide.

pub mod checkout;
pub mod container;

use std::path::{Path, PathBuf};

use gh_actions_context::Payload;
use gh_actions_context::RunContext;
use gh_actions_plan::Plan;
use gh_actions_runner::{At, Error, Options, Summary, actions, report::Reporter};

use crate::container::{Containers, Images, default_images};

use gh_actions_services::Services;
use gh_actions_spec::{Uses, Workflow};
use yaml_with_spans::Document;

#[derive(Debug, Clone)]
pub struct Config {
    pub workspace: PathBuf,
    pub temp: PathBuf,
    pub cache: PathBuf,
    /// Outlives the run: a cache that went with it would never be read.
    pub services: PathBuf,
    pub event_name: String,
    /// Empty runs every job on this machine, with whatever it happens to have.
    pub images: Images,
}

impl Config {
    pub fn for_workspace(workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        // Docker will not mount a relative path.
        let workspace = workspace.canonicalize().unwrap_or(workspace);
        let id = std::process::id();
        let cache = actions::cache_directory();
        let services = cache
            .parent()
            .unwrap_or(&cache)
            .join("services")
            .to_path_buf();

        Self {
            workspace,
            temp: std::env::temp_dir().join(format!("canopy-{id}")),
            cache,
            services,
            event_name: "push".to_owned(),
            images: default_images(),
        }
    }

    pub fn event(mut self, name: impl Into<String>) -> Self {
        self.event_name = name.into();
        self
    }
}

pub struct Local {
    run: RunContext,
    config: Config,
    services: Services,
}

impl Local {
    pub fn start(config: Config) -> Result<Self, Error> {
        let services = Services::start(&config.services).at(&config.services)?;
        let run = checkout::context(&config.workspace, &config.event_name, &config.temp);

        Ok(Self {
            config,
            services,
            run,
        })
    }

    pub fn event(&self) -> &Payload {
        &self.run.github.event
    }

    pub fn context(&self) -> &RunContext {
        &self.run
    }

    pub fn read(&self, workflow: &Path) -> Result<(Document, Workflow), Error> {
        let source = std::fs::read_to_string(workflow).at(workflow)?;
        let document = Document::parse(&source)?;
        let workflow = yaml_with_spans::from_node(&document.root)?;

        Ok((document, workflow))
    }

    pub fn plan(&self, workflow: &Path) -> Result<(Workflow, Plan), Error> {
        let (_, workflow) = self.read(workflow)?;
        let mut plan = gh_actions_plan::plan(&workflow)?;

        // The workspace is already where checkout would have put the files, and it wants
        // a `github.token` a local run has not got. act passes over it too.
        for job in &mut plan.jobs {
            for step in job.spec.steps.iter_mut().flatten() {
                if matches!(&step.uses, Some(Uses::Remote { owner, repo, .. })
                    if owner == "actions" && repo == "checkout")
                {
                    step.name = step
                        .name
                        .clone()
                        .or_else(|| Some("actions/checkout".to_owned()));
                    step.uses = None;
                    step.run = Some("echo 'the workspace is already here'".to_owned());
                }
            }
        }

        Ok((workflow, plan))
    }

    pub fn run(
        &self,
        workflow: &Workflow,
        plan: &Plan,
        out: &mut dyn Reporter,
    ) -> Result<Summary, Error> {
        let options = self.options();

        let mut machine = Containers::new(
            self.config.images.clone(),
            vec![
                self.config.workspace.clone(),
                self.config.temp.clone(),
                self.config.cache.clone(),
            ],
        );

        gh_actions_runner::run(workflow, plan, &self.run, &options, &mut machine, out)
    }

    pub fn options(&self) -> Options {
        Options {
            workspace: self.config.workspace.clone(),
            temp: self.config.temp.clone(),
            cache: self.config.cache.clone(),
            service_env: self.services.env(),
            // Nothing here holds a secret worth hiding: a local run has none to give.
            masks: Vec::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    /// Leaves the action cache, which is worth keeping.
    pub fn clean(&self) {
        let _ = std::fs::remove_dir_all(&self.config.temp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_run_points_actions_at_its_own_services() {
        let directory = std::env::temp_dir().join("local-runner-test");
        let local = Local::start(Config::for_workspace(&directory)).expect("services start");
        let options = local.options();

        assert!(options.service_env["ACTIONS_RESULTS_URL"].starts_with("http://127.0.0.1:"));
        assert!(options.service_env.contains_key("ACTIONS_CACHE_URL"));
        assert_eq!(local.context().github.event_name, "push");
        local.clean();
    }

    #[test]
    fn planning_a_workflow_reads_it_from_disk() {
        let directory = std::env::temp_dir().join("local-runner-plan");
        std::fs::create_dir_all(&directory).expect("directory");
        let path = directory.join("workflow.yml");
        std::fs::write(
            &path,
            "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo built\n",
        )
        .expect("write workflow");

        let local = Local::start(Config::for_workspace(&directory)).expect("services start");
        let (_, plan) = local.plan(&path).expect("workflow plans");

        assert_eq!(plan.jobs.len(), 1);
        assert_eq!(plan.jobs[0].id, "build");
        local.clean();
    }
}
