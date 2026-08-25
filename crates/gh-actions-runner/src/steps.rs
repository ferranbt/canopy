//! Expanding a job's steps into what actually runs, hooks included.

use std::path::Path;

use gh_actions_spec::{Runs, Step, Uses};

use crate::actions::{self, ResolvedAction};
use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Pre,
    Main,
    Post,
}

impl Phase {
    pub fn script(self, runs: &Runs) -> Option<String> {
        match runs {
            Runs::Node16(node) | Runs::Node20(node) | Runs::Node24(node) => match self {
                Self::Pre => node.pre.clone(),
                Self::Main => Some(node.main.clone()),
                Self::Post => node.post.clone(),
            },
            Runs::Docker(docker) => match self {
                Self::Pre => docker.pre_entrypoint.clone(),
                // The image is what a container action's main phase runs, not a script.
                Self::Main => None,
                Self::Post => docker.post_entrypoint.clone(),
            },
            Runs::Composite(_) => None,
        }
    }

    /// A composite action is torn down after the job rather than after itself, because the
    /// actions it uses are the ones with something to clean up and theirs run at the end.
    fn planned(self, runs: &Runs) -> bool {
        match (self, runs) {
            (Self::Main, _) => true,
            (Self::Post, Runs::Composite(composite)) => {
                composite.steps.iter().any(|step| step.uses.is_some())
            }
            _ => self.script(runs).is_some(),
        }
    }

    fn condition(self, runs: &Runs) -> Option<String> {
        // Hooks exist to set up and clean up, so they run whatever happened.
        let always = || Some("always()".to_owned());
        let node = match runs {
            Runs::Node16(node) | Runs::Node20(node) | Runs::Node24(node) => node,
            Runs::Docker(_) => return (self != Self::Main).then(always).flatten(),
            Runs::Composite(_) => return (self == Self::Post).then(always).flatten(),
        };

        match self {
            Self::Main => None,
            Self::Pre => node.pre_if.clone().or_else(always),
            Self::Post => node.post_if.clone().or_else(always),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlannedStep {
    pub step: Step,
    pub position: usize,
    pub phase: Phase,
    pub action: Option<ResolvedAction>,
    pub script: Option<String>,
    pub condition: Option<String>,
}

impl PlannedStep {
    pub fn is_hook(&self) -> bool {
        self.phase != Phase::Main
    }
}

/// Post steps come out in reverse, so the action set up last is torn down first.
pub fn plan(
    steps: &[Step],
    workspace: &Path,
    cache: &Path,
    nested: bool,
) -> Result<Vec<PlannedStep>, Error> {
    let mut pre = Vec::new();
    let mut main = Vec::new();
    let mut post = Vec::new();

    for (position, step) in steps.iter().enumerate() {
        let Some(uses) = &step.uses else {
            main.push(PlannedStep {
                step: step.clone(),
                position,
                phase: Phase::Main,
                action: None,
                script: None,
                condition: None,
            });
            continue;
        };

        // A bare image has no metadata, so there is nothing to resolve or hook into.
        if matches!(uses, Uses::Image(_)) {
            main.push(PlannedStep {
                step: step.clone(),
                position,
                phase: Phase::Main,
                action: None,
                script: None,
                condition: None,
            });
            continue;
        }

        // A local action is looked for again when the step runs, which is where GitHub
        // reports one that is not there: the job starts, and only that step fails.
        let action = match actions::resolve(uses, workspace, cache, nested) {
            Ok(action) => action,
            Err(_) if matches!(uses, Uses::Local(_)) => {
                main.push(PlannedStep {
                    step: step.clone(),
                    position,
                    phase: Phase::Main,
                    action: None,
                    script: None,
                    condition: None,
                });
                continue;
            }
            Err(err) => return Err(err),
        };
        for phase in [Phase::Pre, Phase::Main, Phase::Post] {
            let script = phase.script(&action.action.runs);
            if !phase.planned(&action.action.runs) {
                continue;
            }
            // An action in the repository being run has no `pre` hook: there is nowhere to
            // run it before the repository is there, and GitHub passes it over as well.
            if phase == Phase::Pre && matches!(uses, Uses::Local(_)) {
                continue;
            }

            let planned = PlannedStep {
                step: step.clone(),
                position,
                phase,
                action: Some(action.clone()),
                script: script.filter(|_| phase != Phase::Main),
                condition: phase.condition(&action.action.runs),
            };

            match phase {
                Phase::Pre => pre.push(planned),
                Phase::Main => main.push(planned),
                Phase::Post => post.push(planned),
            }
        }
    }

    post.reverse();
    pre.extend(main);
    pre.extend(post);
    Ok(pre)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gh_actions_spec::{Action, NodeRuns};

    use super::*;

    fn action(pre: Option<&str>, post: Option<&str>) -> Action {
        Action {
            name: "test".to_owned(),
            description: None,
            author: None,
            inputs: None,
            outputs: None,
            runs: Runs::Node20(NodeRuns {
                main: "index.js".to_owned(),
                pre: pre.map(str::to_owned),
                post: post.map(str::to_owned),
                pre_if: None,
                post_if: None,
            }),
            branding: None,
        }
    }

    #[test]
    fn a_phase_finds_the_script_it_runs() {
        let with_hooks = action(Some("setup.js"), Some("cleanup.js"));

        assert_eq!(
            Phase::Pre.script(&with_hooks.runs),
            Some("setup.js".to_owned())
        );
        assert_eq!(
            Phase::Main.script(&with_hooks.runs),
            Some("index.js".to_owned())
        );
        assert_eq!(
            Phase::Post.script(&with_hooks.runs),
            Some("cleanup.js".to_owned())
        );
    }

    #[test]
    fn an_action_without_hooks_declares_none() {
        let plain = action(None, None);

        assert!(Phase::Pre.script(&plain.runs).is_none());
        assert!(Phase::Post.script(&plain.runs).is_none());
    }

    fn composite(name: &str, inner: &str) -> PathBuf {
        let workspace = std::env::temp_dir().join(format!("canopy-steps-{name}"));
        let action = workspace.join("actions/outer");
        std::fs::create_dir_all(&action).expect("somewhere to put an action");
        std::fs::write(
            action.join("action.yml"),
            format!("name: Outer\nruns:\n  using: composite\n  steps:\n{inner}"),
        )
        .expect("an action to find");

        workspace
    }

    fn phases(workspace: &Path) -> Vec<Phase> {
        let steps = crate::testing::steps_of(
            r"
      - uses: ./actions/outer
",
        );

        plan(&steps, workspace, &std::env::temp_dir(), false)
            .expect("the action is there")
            .into_iter()
            .map(|planned| planned.phase)
            .collect()
    }

    #[test]
    fn a_composite_that_uses_actions_is_torn_down_after_the_job() {
        let uses_one = composite(
            "uses-one",
            "    - uses: ./actions/greet\n    - shell: bash\n      run: echo hi\n",
        );

        assert_eq!(
            phases(&uses_one),
            [Phase::Main, Phase::Post],
            "the post steps of what it used run at the end of the job, in a post step of its own"
        );
    }

    #[test]
    fn one_that_only_runs_scripts_has_nothing_to_tear_down() {
        let scripts_only = composite("scripts-only", "    - shell: bash\n      run: echo hi\n");

        assert_eq!(phases(&scripts_only), [Phase::Main]);
    }

    #[test]
    fn hooks_run_whatever_happened_unless_told_otherwise() {
        let hooked = action(Some("setup.js"), Some("cleanup.js"));

        assert_eq!(
            Phase::Post.condition(&hooked.runs),
            Some("always()".to_owned())
        );
        assert_eq!(Phase::Main.condition(&hooked.runs), None);
    }
}
