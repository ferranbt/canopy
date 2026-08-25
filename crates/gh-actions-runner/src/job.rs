//! Executes a plan, one job and one step at a time.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gh_actions_context::{Conclusion, JobResult, RunContext, step_result};
use gh_actions_expr::{Context, interpolate, interpolate_value};
use gh_actions_spec::{Action, Defaults, RunDefaults, Runs, Scalar, Step, Uses, Workflow};

use gh_actions_context::Payload;
use gh_actions_plan::{Plan, PlannedJob, scalar_value};

use crate::actions::{self, ResolvedAction};
use crate::commands::{self, Command};
use crate::error::{At, Error};
use crate::executor::{Exec, ExecRequest, ExecResult, Image, Machine, Started};
use crate::report::{Event, Level, PassedOver, Reporter};
use crate::steps::{self, Phase, PlannedStep};
use crate::validate;

#[derive(Debug, Clone)]
pub struct Options {
    pub workspace: PathBuf,
    pub temp: PathBuf,
    pub cache: PathBuf,
    pub service_env: BTreeMap<String, String>,
    pub masks: Vec<String>,
}

fn workflow_name(workflow: &Workflow) -> String {
    workflow
        .name
        .clone()
        .unwrap_or_else(|| "workflow".to_owned())
}

fn write_event_file(temp: &Path, event: &Payload) -> Result<PathBuf, Error> {
    let directory = temp.join("_github_workflow");
    fs::create_dir_all(&directory).at(&directory)?;

    let path = directory.join("event.json");
    let event = serde_json::to_value(event).map(by_name).unwrap_or_default();
    let payload = serde_json::to_string_pretty(&event).unwrap_or_else(|_| "{}".to_owned());
    fs::write(&path, payload).at(&path)?;
    Ok(path)
}

fn by_name(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(fields) => {
            let mut named: Vec<(String, serde_json::Value)> = fields.into_iter().collect();
            named.sort_by(|(one, _), (other, _)| one.cmp(other));

            serde_json::Value::Object(
                named
                    .into_iter()
                    .map(|(name, value)| (name, by_name(value)))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(by_name).collect())
        }
        scalar => scalar,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub jobs: Vec<(String, Conclusion)>,
}

impl Summary {
    pub fn succeeded(&self) -> bool {
        self.jobs
            .iter()
            .all(|(_, outcome)| *outcome != Conclusion::Failure)
    }
}

pub fn run(
    workflow: &Workflow,
    plan: &Plan,
    run_context: &RunContext,
    options: &Options,
    machine: &mut dyn Machine,
    out: &mut dyn Reporter,
) -> Result<Summary, Error> {
    let mut summary = Summary::default();
    let mut results: BTreeMap<String, JobResult> = BTreeMap::new();

    fs::create_dir_all(&options.temp).at(&options.temp)?;

    let mut cancelled: BTreeSet<String> = BTreeSet::new();

    for job in &plan.jobs {
        if cancelled.contains(&job.id) {
            out.report(Event::JobPassedOver {
                label: job.label.clone(),
                reason: PassedOver::Cancelled,
            });
            results.insert(job.id.clone(), JobResult::default());
            summary.jobs.push((job.label.clone(), Conclusion::Skipped));
            continue;
        }

        let outcome = run_job(
            workflow,
            job,
            run_context,
            options,
            machine,
            out,
            &mut results,
        )?;
        summary.jobs.push((job.label.clone(), outcome.conclusion));

        // Only the rest of this job's matrix; the workflow carries on.
        if outcome.conclusion == Conclusion::Failure && outcome.fail_fast {
            cancelled.insert(job.id.clone());
        }
    }

    Ok(summary)
}

pub fn run_steps(
    job: &PlannedJob,
    run: &RunContext,
    options: &Options,
    machine: &mut dyn Machine,
    out: &mut dyn Reporter,
) -> Result<Conclusion, Error> {
    fs::create_dir_all(&options.temp).at(&options.temp)?;

    let deadline = Instant::now() + minutes(job_timeout(&job.spec, &run.to_expr_context())?);
    let defaults = job
        .spec
        .defaults
        .as_ref()
        .and_then(|defaults| defaults.run.clone())
        .unwrap_or_default();

    let steps = job.spec.steps.clone().unwrap_or_default();
    let planned = steps::plan(&steps, &options.workspace, &options.cache, false)?;

    let started = machine.start(job, out)?;
    let outcome = run_prepared(
        job,
        &planned,
        options,
        run.clone(),
        defaults,
        deadline,
        started == Started::Missing,
        machine,
        out,
    );
    let cleaned = machine.finish();

    let (conclusion, _) = outcome?;
    cleaned?;
    Ok(conclusion)
}

/// The images the steps of a job are run from, which are pulled rather than built. An
/// action built from a `Dockerfile` is not one: that is built when the step gets there.
fn wanted(planned: &[PlannedStep]) -> BTreeSet<String> {
    let mut images = BTreeSet::new();

    for step in planned {
        if let Some(Uses::Image(image)) = &step.step.uses {
            images.insert(image.clone());
        }
        if let Some(resolved) = &step.action
            && let Runs::Docker(docker) = &resolved.action.runs
            && let Some(image) = docker.image.strip_prefix("docker://")
        {
            images.insert(image.to_owned());
        }
    }

    images
}

struct JobOutcome {
    conclusion: Conclusion,
    fail_fast: bool,
}

fn run_job(
    workflow: &Workflow,
    job: &PlannedJob,
    run_context: &RunContext,
    options: &Options,
    machine: &mut dyn Machine,
    out: &mut dyn Reporter,
    results: &mut BTreeMap<String, JobResult>,
) -> Result<JobOutcome, Error> {
    let mut run = run_context.clone();
    run.github.workflow = workflow_name(workflow);

    // Written before any step runs, since a step may read the payload from disk.
    run.github.event_path = write_event_file(&options.temp, &run.github.event)?
        .display()
        .to_string();

    let needs: BTreeMap<String, JobResult> = job
        .needs
        .iter()
        .map(|id| (id.clone(), results.get(id).cloned().unwrap_or_default()))
        .collect();
    let needs_ok = needs
        .values()
        .all(|result| result.conclusion == Conclusion::Success);

    run.github.job = job.id.clone();
    run.matrix = (!job.matrix.is_empty()).then(|| job.matrix.clone());
    run.needs = needs;
    run.env = merged_env(workflow, job);
    run.job.status = if needs_ok {
        Conclusion::Success
    } else {
        Conclusion::Failure
    };

    let context = run.to_expr_context();
    let allowed_to_fail = flag(&job.spec.continue_on_error, false, &context)?;
    let fail_fast = match &job.spec.strategy {
        Some(strategy) => flag(&strategy.fail_fast, true, &context)?,
        None => true,
    };

    if !should_run(job.spec.r#if.as_deref(), &context, needs_ok)? {
        out.report(Event::JobPassedOver {
            label: job.label.clone(),
            reason: PassedOver::Skipped,
        });
        results.insert(job.id.clone(), JobResult::default());
        return Ok(JobOutcome {
            conclusion: Conclusion::Skipped,
            fail_fast,
        });
    }
    run.job.status = Conclusion::Success;

    out.report(Event::JobStarted {
        id: job.id.clone(),
        label: job.label.clone(),
    });

    let job_deadline = Instant::now() + minutes(job_timeout(&job.spec, &run.to_expr_context())?);

    // Resolved before the machine is asked for, so a job that names an action nobody has
    // is over before anything was started for it.
    let steps = job.spec.steps.clone().unwrap_or_default();
    let planned = match steps::plan(&steps, &options.workspace, &options.cache, false) {
        Ok(planned) => planned,
        Err(err) => return Ok(abandoned(job, &[], err, fail_fast, out, results)),
    };

    let started = match machine.start(job, out) {
        Ok(started) => started,
        Err(err) => return Ok(abandoned(job, &planned, err, fail_fast, out, results)),
    };

    let defaults = run_defaults(workflow, job);
    let outcome = run_prepared(
        job,
        &planned,
        options,
        run,
        defaults,
        job_deadline,
        started == Started::Missing,
        machine,
        out,
    );

    // Collected rather than raised, so a machine that will not go away cannot hide why the
    // job stopped.
    let cleaned = machine.finish();

    let (conclusion, outputs) = match outcome {
        Ok(outcome) => outcome,
        Err(err) => return Ok(abandoned(job, &[], err, fail_fast, out, results)),
    };
    cleaned?;

    // A job that failed says so however it was allowed to: what `continue-on-error` buys
    // it is that the run carries on, not that it did something else.
    let forgiven = conclusion == Conclusion::Failure && allowed_to_fail;
    out.report(Event::JobFinished {
        id: job.id.clone(),
        label: job.label.clone(),
        conclusion,
    });
    if !outputs.is_empty() {
        out.report(Event::JobOutputs {
            id: job.id.clone(),
            outputs: outputs.clone(),
        });
    }

    let counted = match forgiven {
        true => Conclusion::Success,
        false => conclusion,
    };

    results.insert(
        job.id.clone(),
        JobResult {
            conclusion: counted,
            outputs,
        },
    );
    Ok(JobOutcome {
        conclusion: counted,
        fail_fast,
    })
}

/// A job nothing could be run for: the run carries on without it, the way GitHub leaves the
/// rest of a workflow alone when one job cannot be set up.
fn abandoned(
    job: &PlannedJob,
    planned: &[PlannedStep],
    err: Error,
    fail_fast: bool,
    out: &mut dyn Reporter,
    results: &mut BTreeMap<String, JobResult>,
) -> JobOutcome {
    out.report(Event::Message {
        level: Level::Error,
        text: err.to_string(),
    });

    for (index, step) in planned.iter().filter(|step| !step.is_hook()).enumerate() {
        let name = step.step.name.clone().unwrap_or_default();
        out.report(Event::StepStarted {
            index,
            name: name.clone(),
            depth: 0,
        });
        out.report(Event::StepFinished {
            index,
            name,
            depth: 0,
            conclusion: Conclusion::Skipped,
            code: None,
        });
    }

    out.report(Event::JobFinished {
        id: job.id.clone(),
        label: job.label.clone(),
        conclusion: Conclusion::Failure,
    });
    results.insert(
        job.id.clone(),
        JobResult {
            conclusion: Conclusion::Failure,
            outputs: BTreeMap::new(),
        },
    );

    JobOutcome {
        conclusion: Conclusion::Failure,
        fail_fast,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_prepared(
    job: &PlannedJob,
    planned: &[PlannedStep],
    options: &Options,
    run: RunContext,
    defaults: RunDefaults,
    job_deadline: Instant,
    failing: bool,
    machine: &mut dyn Machine,
    out: &mut dyn Reporter,
) -> Result<(Conclusion, BTreeMap<String, String>), Error> {
    let mut runner = JobRunner {
        options,
        machine,
        out,
        run,
        path_entries: Vec::new(),
        state: BTreeMap::new(),
        saved: BTreeMap::new(),
        deferred: BTreeMap::new(),
        counter: 0,
        job_deadline,
        step_deadline: None,
        step_name: String::new(),
        defaults,
        masks: options.masks.clone(),
    };

    // What a step passed that its action never declared is said when the step runs, the way
    // GitHub says it, so it is not said twice.
    for finding in validate::inputs(planned) {
        if matches!(finding.problem, validate::Problem::Unexpected { .. }) {
            continue;
        }
        runner.out.report(Event::Message {
            level: Level::Warning,
            text: finding.message(),
        });
    }

    // An image a step will want is got before the first of them runs, and a job that cannot
    // have one is failing before it starts: what is left of it is passed over, bar the steps
    // that run whatever happened.
    let mut missing = failing;
    for image in wanted(planned) {
        if !runner.fetch(&image)? {
            runner.out.report(Event::Message {
                level: Level::Error,
                text: format!("cannot pull {image}"),
            });
            missing = true;
        }
    }

    let (failed, _) = runner.run_steps(planned, 0, missing)?;
    let conclusion = if failed {
        Conclusion::Failure
    } else {
        Conclusion::Success
    };

    Ok((conclusion, runner.job_outputs(job)?))
}

/// The post steps of the actions a composite action used, and what they need to see.
struct Deferred {
    steps: Vec<PlannedStep>,
    inputs: BTreeMap<String, String>,
    path: PathBuf,
    state: BTreeMap<usize, BTreeMap<String, String>>,
    theirs: BTreeMap<usize, Deferred>,
}

struct JobRunner<'a> {
    options: &'a Options,
    machine: &'a mut dyn Machine,
    out: &'a mut dyn Reporter,
    run: RunContext,
    path_entries: Vec<String>,
    state: BTreeMap<usize, BTreeMap<String, String>>,
    saved: BTreeMap<String, String>,
    /// What a composite action left for its own post step to run, by the position of the
    /// step that used it.
    deferred: BTreeMap<usize, Deferred>,
    counter: usize,
    job_deadline: Instant,
    step_deadline: Option<Instant>,
    step_name: String,
    defaults: RunDefaults,
    masks: Vec<String>,
}

impl JobRunner<'_> {
    /// Whether anything failed, and what the first thing to do so came back with.
    fn run_steps(
        &mut self,
        steps: &[PlannedStep],
        depth: usize,
        failing: bool,
    ) -> Result<(bool, Option<i32>), Error> {
        let mut failed = failing;
        let mut code = None;
        // Counted as they run rather than as they were written: a hook is a step of its own
        // by the time it runs, and comes after everything the job did before it.
        let mut index = 0;

        for planned in steps {
            let step = &planned.step;
            self.run.job.status = conclusion_of(failed);

            // A post hook only runs if the step it belongs to did, and sees the state it
            // saved. A pre hook comes before any of that, so it has neither to wait for.
            self.saved.clear();
            if planned.phase == Phase::Post {
                let Some(state) = self.state.get(&planned.position).cloned() else {
                    continue;
                };
                self.saved = state;
            }

            let context = self.run.to_expr_context();
            let condition = planned.condition.as_deref().or(step.r#if.as_deref());
            let name = step_name(planned, &context)?;

            // A step whose condition says no is still a step of the job, and is reported as
            // one that was skipped rather than left out of what happened.
            if !should_run(condition, &context, !failed)? {
                // A step that was passed over is still one the job had, and what follows can
                // read how it went.
                if !planned.is_hook()
                    && let Some(id) = &step.id
                {
                    self.run.steps.insert(
                        id.clone(),
                        step_result(Conclusion::Skipped, Conclusion::Skipped, &BTreeMap::new()),
                    );
                }

                self.out.report(Event::StepStarted {
                    index,
                    name: name.clone(),
                    depth,
                });
                self.out.report(Event::StepFinished {
                    index,
                    name,
                    depth,
                    conclusion: Conclusion::Skipped,
                    code: None,
                });
                index += 1;

                continue;
            }

            self.out.report(Event::StepStarted {
                index,
                name: name.clone(),
                depth,
            });

            let outcome = self.execute(planned, &context, depth)?;
            let forgiven = continues_on_error(step, &context)?;

            if !planned.is_hook() {
                if let Some(id) = &step.id {
                    self.run.steps.insert(
                        id.clone(),
                        step_result(
                            conclusion_of(!outcome.succeeded),
                            conclusion_of(!outcome.succeeded && !forgiven),
                            &outcome.outputs,
                        ),
                    );
                }
                self.state.insert(planned.position, outcome.state.clone());
            }

            // Whatever it was forgiven, a step that failed is what the action it belongs to
            // came back with.
            if !outcome.succeeded {
                code = code.or(outcome.code);
                failed |= !forgiven;
            }
            self.out.report(Event::StepFinished {
                index,
                name,
                depth,
                conclusion: conclusion_of(!outcome.succeeded && !forgiven),
                code: outcome.code,
            });
            index += 1;
        }

        Ok((failed, code))
    }

    /// A step gets the sooner of its own limit and the job's: a job that has run out of
    /// time cannot be rescued by a step that was given longer.
    fn fetch(&mut self, image: &str) -> Result<bool, Error> {
        let request = ExecRequest::new(
            Exec::Fetch {
                image: image.to_owned(),
            },
            &self.options.workspace,
        );

        Ok(self.exec(&request)?.status.success)
    }

    fn exec(&mut self, request: &ExecRequest) -> Result<ExecResult, Error> {
        let deadline = match self.step_deadline {
            Some(step) => step.min(self.job_deadline),
            None => self.job_deadline,
        };
        let left = deadline.saturating_duration_since(Instant::now());
        let request = request
            .clone()
            .name(self.step_name.clone())
            .timeout(Some(left))
            .masks(self.masks.clone());

        self.machine.exec(&request, self.out)
    }

    fn execute(
        &mut self,
        planned: &PlannedStep,
        context: &Context,
        depth: usize,
    ) -> Result<StepOutcome, Error> {
        let step = &planned.step;
        self.step_deadline = step_timeout(step, context)?.map(|limit| Instant::now() + limit);
        self.step_name = step.name.clone().unwrap_or_default();

        if let Some(hook) = passed_over(planned) {
            self.out.report(Event::Message {
                level: Level::Warning,
                text: hook,
            });
        }

        // A composite action has no script to hook into: what it left is the post steps of
        // the actions it used.
        if planned.phase == Phase::Post
            && let Some(resolved) = &planned.action
            && matches!(resolved.action.runs, Runs::Composite(_))
        {
            return self.run_deferred(planned.position, depth);
        }

        if let (Some(hook), Some(resolved)) = (&planned.script, &planned.action) {
            let inputs = self.inputs_for(Some(&resolved.action), step, context)?;

            return match resolved.action.runs.clone() {
                // A container action hooks by running its image with another entrypoint.
                Runs::Docker(runs) => self.run_container(
                    ContainerRun {
                        image: Self::container_image(&runs.image, resolved),
                        entrypoint: Some(hook),
                        args: &[],
                        inputs: &inputs,
                        env: runs.env.as_ref(),
                    },
                    step,
                    context,
                ),
                _ => self.run_node(resolved, hook, &inputs, step, context),
            };
        }

        match (&step.run, &step.uses) {
            (Some(script), _) => self.run_script(step, script, context),
            (None, Some(uses)) => self.run_action(step, uses, context, depth, planned.position),
            (None, None) => Err(Error::Plan(
                "a step needs either `run:` or `uses:`".to_owned(),
            )),
        }
    }

    fn run_script(
        &mut self,
        step: &Step,
        script: &str,
        context: &Context,
    ) -> Result<StepOutcome, Error> {
        let script = interpolate(script, context)?;
        let asked = step.shell.clone().or_else(|| self.defaults.shell.clone());
        let named = asked.is_some();
        let shell = asked.unwrap_or_else(|| "bash".to_owned());
        let files = self.step_files()?;
        let cwd = self.script_directory(step, context)?;

        if !cwd.is_dir() {
            let (program, _) = Exec::Script {
                shell: shell.clone(),
                script: files.script.clone(),
                named,
            }
            .to_command(&BTreeMap::new())?;

            return Ok(self.refused(Error::Refused(format!(
                "An error occurred trying to start process '{}' with working directory '{}'. \
                 No such file or directory",
                found(&program),
                cwd.display()
            ))));
        }

        let request = ExecRequest::new(
            Exec::Script {
                shell,
                script: files.script.clone(),
                named,
            },
            cwd,
        )
        .envs(self.step_env(&files))
        .envs(interpolated_env(step.env.as_ref(), context)?);

        self.write_file(&files.script, &script)?;
        let status = self.exec(&request)?;
        self.collect(&files, &status)
    }

    fn run_action(
        &mut self,
        step: &Step,
        uses: &Uses,
        context: &Context,
        depth: usize,
        position: usize,
    ) -> Result<StepOutcome, Error> {
        if let Uses::Image(image) = uses {
            let image = image.clone();
            let inputs = self.inputs_for(None, step, context)?;
            return self.run_container(
                ContainerRun {
                    image: Image::Registry(image),
                    entrypoint: None,
                    args: &[],
                    inputs: &inputs,
                    env: None,
                },
                step,
                context,
            );
        }

        // A step of a composite action keeps the path it was written with, which is where
        // GitHub leaves `github.action_path` for one.
        let inside = depth > 0;
        let resolved =
            match actions::resolve(uses, &self.options.workspace, &self.options.cache, inside) {
                Ok(resolved) => resolved,
                Err(err) => return Ok(self.refused(err)),
            };

        if let Some(text) = validate::unexpected(step, &resolved.action) {
            self.out.report(Event::Message {
                level: Level::Warning,
                text,
            });
        }

        let inputs = self.inputs_for(Some(&resolved.action), step, context)?;

        match resolved.action.runs.clone() {
            Runs::Composite(runs) => {
                self.run_composite(&resolved, &runs.steps, &inputs, depth, position)
            }
            Runs::Node16(runs) | Runs::Node20(runs) | Runs::Node24(runs) => {
                self.run_node(&resolved, &runs.main, &inputs, step, context)
            }
            Runs::Docker(runs) => {
                let image = Self::container_image(&runs.image, &resolved);
                let args = runs.args.unwrap_or_default();
                self.run_container(
                    ContainerRun {
                        image,
                        entrypoint: runs.entrypoint.as_deref(),
                        args: &args,
                        inputs: &inputs,
                        env: runs.env.as_ref(),
                    },
                    step,
                    context,
                )
            }
        }
    }

    /// The post step of a composite action: what the actions it used left to be done.
    fn run_deferred(&mut self, position: usize, depth: usize) -> Result<StepOutcome, Error> {
        let Some(deferred) = self.deferred.remove(&position) else {
            return Ok(StepOutcome {
                succeeded: true,
                code: Some(0),
                outputs: BTreeMap::new(),
                state: BTreeMap::new(),
            });
        };

        let caller_inputs = std::mem::replace(&mut self.run.inputs, deferred.inputs);
        let caller_steps = std::mem::take(&mut self.run.steps);
        let caller_path = self.run.github.action_path.replace(deferred.path);
        let caller_state = std::mem::replace(&mut self.state, deferred.state);
        let caller_deferred = std::mem::replace(&mut self.deferred, deferred.theirs);

        let ran = self.run_steps(&deferred.steps, depth + 1, false);

        self.run.inputs = caller_inputs;
        self.run.steps = caller_steps;
        self.run.github.action_path = caller_path;
        self.state = caller_state;
        self.deferred = caller_deferred;

        let (failed, code) = ran?;
        Ok(StepOutcome {
            succeeded: !failed,
            code: code.or(Some(i32::from(failed))),
            outputs: BTreeMap::new(),
            state: BTreeMap::new(),
        })
    }

    /// A step the runner would not start: it says why, and only that step fails.
    fn refused(&mut self, err: Error) -> StepOutcome {
        self.out.report(Event::Message {
            level: Level::Error,
            text: err.to_string(),
        });

        StepOutcome {
            succeeded: false,
            code: Some(0),
            outputs: BTreeMap::new(),
            state: BTreeMap::new(),
        }
    }

    fn run_composite(
        &mut self,
        resolved: &ResolvedAction,
        steps: &[Step],
        inputs: &BTreeMap<String, String>,
        depth: usize,
        position: usize,
    ) -> Result<StepOutcome, Error> {
        let caller_inputs = std::mem::replace(&mut self.run.inputs, inputs.clone());
        let caller_steps = std::mem::take(&mut self.run.steps);
        let caller_path = self.run.github.action_path.replace(resolved.path.clone());
        let caller_state = std::mem::take(&mut self.state);
        let caller_deferred = std::mem::take(&mut self.deferred);

        // An action this one uses is cleaned up at the end of the job rather than the end of
        // this step, which is what the post step of a composite action is for.
        let planned = steps::plan(steps, &self.options.workspace, &self.options.cache, true)?;
        let (later, now): (Vec<_>, Vec<_>) = planned
            .into_iter()
            .partition(|step| step.phase == Phase::Post);
        let (failed, code) = self.run_steps(&now, depth + 1, false)?;

        self.run.job.status = conclusion_of(failed);
        let context = self.run.to_expr_context();
        let mut outputs = BTreeMap::new();
        for (name, output) in resolved.action.outputs.iter().flatten() {
            if let Some(value) = &output.value {
                outputs.insert(name.clone(), interpolate(value, &context)?);
            }
        }

        let theirs = std::mem::replace(&mut self.deferred, caller_deferred);
        if !later.is_empty() {
            self.deferred.insert(
                position,
                Deferred {
                    steps: later,
                    inputs: inputs.clone(),
                    path: resolved.path.clone(),
                    state: self.state.clone(),
                    theirs,
                },
            );
        }

        self.run.inputs = caller_inputs;
        self.run.steps = caller_steps;
        self.run.github.action_path = caller_path;
        self.state = caller_state;

        // What the step it gave up in came back with, since that is what the action came to.
        Ok(StepOutcome {
            succeeded: !failed,
            code: code.or(Some(i32::from(failed))),
            outputs,
            state: BTreeMap::new(),
        })
    }

    fn run_node(
        &mut self,
        resolved: &ResolvedAction,
        main: &str,
        inputs: &BTreeMap<String, String>,
        step: &Step,
        context: &Context,
    ) -> Result<StepOutcome, Error> {
        let files = self.step_files()?;
        let cwd = self.working_directory(step, context)?;

        let request = ExecRequest::new(
            Exec::Node {
                entrypoint: resolved.path.join(main),
            },
            cwd,
        )
        // No `GITHUB_ACTION_PATH`: where an action lives is told to a composite action,
        // which has steps of its own to point at it, and to nothing else.
        .envs(self.step_env(&files))
        .envs(
            inputs
                .iter()
                .map(|(key, value)| (input_variable(key), value.clone())),
        );

        let status = self.exec(&request)?;
        self.collect(&files, &status)
    }

    fn container_image(image: &str, resolved: &ResolvedAction) -> Image {
        match image.strip_prefix("docker://") {
            Some(image) => Image::Registry(image.to_owned()),
            None => Image::Dockerfile {
                path: resolved.path.join(image),
                context: resolved.path.clone(),
            },
        }
    }

    fn run_container(
        &mut self,
        container: ContainerRun<'_>,
        step: &Step,
        context: &Context,
    ) -> Result<StepOutcome, Error> {
        let ContainerRun {
            image,
            entrypoint,
            args,
            inputs,
            env: container_env,
        } = container;

        // `args` and the action's own `env` are written against the action's inputs, not the job's.
        let mut inner = self.run.clone();
        inner.inputs = inputs.clone();
        let inner = inner.to_expr_context();
        let files = self.step_files()?;

        // The files a step exchanges state through are mounted at `/github/file_commands`,
        // so they are named from there rather than by where they sit on the host.
        let mut container_vars = self.base_env(&files, |path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            format!("/{INSIDE}/{name}")
        });
        container_vars.insert(
            "GITHUB_WORKSPACE".to_owned(),
            "/github/workspace".to_owned(),
        );
        container_vars.extend(interpolated_env(step.env.as_ref(), context)?);
        container_vars.extend(interpolated_env(container_env, &inner)?);
        for (key, value) in inputs {
            container_vars.insert(input_variable(key), value.clone());
        }

        let mut interpolated = Vec::new();
        for arg in args {
            interpolated.push(interpolate(arg, &inner)?);
        }

        let request = ExecRequest::new(
            Exec::Container {
                image,
                entrypoint: entrypoint.map(str::to_owned),
                args: interpolated,
                mounts: vec![
                    (
                        self.options.workspace.clone(),
                        PathBuf::from("/github/workspace"),
                    ),
                    (
                        self.options.temp.join(COMMANDS),
                        PathBuf::from(format!("/{INSIDE}")),
                    ),
                    (
                        self.options.temp.clone(),
                        PathBuf::from("/github/runner_temp"),
                    ),
                ],
                workdir: PathBuf::from("/github/workspace"),
            },
            &self.options.workspace,
        )
        .envs(container_vars);

        let status = self.exec(&request)?;
        self.collect(&files, &status)
    }

    fn inputs_for(
        &self,
        action: Option<&Action>,
        step: &Step,
        context: &Context,
    ) -> Result<BTreeMap<String, String>, Error> {
        let mut inputs = BTreeMap::new();

        let declared = action.and_then(|action| action.inputs.as_ref());
        for (name, input) in declared.into_iter().flatten() {
            if let Some(default) = &input.default {
                inputs.insert(name.clone(), interpolate(&scalar_string(default), context)?);
            }
        }
        for (name, value) in step.with.iter().flatten() {
            inputs.insert(name.clone(), interpolate(&scalar_string(value), context)?);
        }

        Ok(inputs)
    }

    fn base_env(
        &self,
        files: &StepFiles,
        at: impl Fn(&Path) -> String,
    ) -> BTreeMap<String, String> {
        let mut env = self.run.to_env();
        env.extend(self.options.service_env.clone());
        env.extend(self.run.env.clone());

        for (key, path) in [
            ("GITHUB_ENV", &files.env),
            ("GITHUB_OUTPUT", &files.output),
            ("GITHUB_PATH", &files.path),
            ("GITHUB_STEP_SUMMARY", &files.summary),
            ("GITHUB_STATE", &files.state),
        ] {
            env.insert(key.to_owned(), at(path));
        }

        for (name, value) in &self.saved {
            env.insert(commands::state_variable(name), value.clone());
        }

        env
    }

    fn step_env(&self, files: &StepFiles) -> BTreeMap<String, String> {
        let mut env = self.base_env(files, |path| path.display().to_string());

        if !self.path_entries.is_empty() {
            let existing = std::env::var("PATH").unwrap_or_default();
            env.insert(
                "PATH".to_owned(),
                format!("{}:{existing}", self.path_entries.join(":")),
            );
        }
        env
    }

    fn collect(&mut self, files: &StepFiles, result: &ExecResult) -> Result<StepOutcome, Error> {
        let mut wrong = false;
        let mut read = |name: &str, contents: &str, out: &mut dyn Reporter| {
            match parse_env_file(contents) {
                Ok(entries) => entries,
                Err(why) => {
                    // Word for word what GitHub says, since a step is failed for it.
                    for text in [
                        format!("Unable to process file command '{name}' successfully."),
                        why,
                    ] {
                        out.report(Event::Message {
                            level: Level::Error,
                            text,
                        });
                    }
                    wrong = true;
                    BTreeMap::new()
                }
            }
        };

        let from_env = read("env", &self.read_file(&files.env)?, self.out);
        let mut outputs = read("output", &self.read_file(&files.output)?, self.out);
        let mut state = read("state", &self.read_file(&files.state)?, self.out);

        self.run.env.extend(from_env);
        self.path_entries
            .extend(read_lines(&self.read_file(&files.path)?));

        for command in &result.commands {
            match command {
                Command::SetOutput { name, value } => {
                    outputs.insert(name.clone(), value.clone());
                }
                Command::SaveState { name, value } => {
                    state.insert(name.clone(), value.clone());
                }
                Command::AddPath(path) => self.path_entries.push(path.clone()),
                Command::SetEnv { name, value } => {
                    self.run.env.insert(name.clone(), value.clone());
                }
                Command::AddMask(secret) if !secret.is_empty() => {
                    self.masks.push(secret.clone());
                }
                _ => {}
            }
        }

        Ok(StepOutcome {
            succeeded: result.status.success && !wrong,
            code: result.status.code,
            outputs,
            state,
        })
    }

    fn write_file(&self, path: &Path, contents: &str) -> Result<(), Error> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).at(parent)?;
        }
        fs::write(path, contents).at(path)
    }

    fn read_file(&self, path: &Path) -> Result<String, Error> {
        fs::read_to_string(path).at(path)
    }

    fn step_files(&mut self) -> Result<StepFiles, Error> {
        self.counter += 1;
        let files = StepFiles::at(&self.options.temp, self.counter);

        for file in files.all() {
            self.write_file(file, "")?;
        }
        Ok(files)
    }

    fn working_directory(&self, step: &Step, context: &Context) -> Result<PathBuf, Error> {
        let Some(directory) = &step.working_directory else {
            return Ok(self.options.workspace.clone());
        };
        Ok(self
            .options
            .workspace
            .join(interpolate(directory, context)?))
    }

    fn script_directory(&self, step: &Step, context: &Context) -> Result<PathBuf, Error> {
        let Some(directory) = step
            .working_directory
            .as_ref()
            .or(self.defaults.working_directory.as_ref())
        else {
            return Ok(self.options.workspace.clone());
        };
        Ok(self
            .options
            .workspace
            .join(interpolate(directory, context)?))
    }

    fn job_outputs(&self, job: &PlannedJob) -> Result<BTreeMap<String, String>, Error> {
        let context = self.run.to_expr_context();
        let mut outputs = BTreeMap::new();

        for (key, template) in job.spec.outputs.iter().flatten() {
            outputs.insert(
                key.clone(),
                interpolate_value(template, &context)?.to_display_string(),
            );
        }
        Ok(outputs)
    }
}

struct ContainerRun<'a> {
    image: Image,
    entrypoint: Option<&'a str>,
    args: &'a [String],
    inputs: &'a BTreeMap<String, String>,
    env: Option<&'a BTreeMap<String, Scalar>>,
}

struct StepOutcome {
    succeeded: bool,
    code: Option<i32>,
    outputs: BTreeMap<String, String>,
    state: BTreeMap<String, String>,
}

fn found(program: &str) -> String {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|directory| Path::new(directory).join(program))
        .find(|path| path.is_file())
        .map_or_else(|| program.to_owned(), |path| path.display().to_string())
}

fn conclusion_of(failed: bool) -> Conclusion {
    if failed {
        Conclusion::Failure
    } else {
        Conclusion::Success
    }
}

fn input_variable(name: &str) -> String {
    format!("INPUT_{}", name.to_uppercase().replace(' ', "_"))
}

/// Where the files a step exchanges state through are kept, and where a container reaches
/// them from.
const COMMANDS: &str = "_runner_file_commands";
const INSIDE: &str = "github/file_commands";

struct StepFiles {
    script: PathBuf,
    env: PathBuf,
    output: PathBuf,
    path: PathBuf,
    summary: PathBuf,
    state: PathBuf,
}

impl StepFiles {
    fn at(temp: &Path, position: usize) -> Self {
        let commands = temp.join(COMMANDS);

        Self {
            script: temp.join(format!("step-{position}.sh")),
            env: commands.join(format!("step-{position}.env")),
            output: commands.join(format!("step-{position}.output")),
            path: commands.join(format!("step-{position}.path")),
            summary: commands.join(format!("step-{position}.summary")),
            state: commands.join(format!("step-{position}.state")),
        }
    }

    fn all(&self) -> [&PathBuf; 5] {
        [
            &self.env,
            &self.output,
            &self.path,
            &self.summary,
            &self.state,
        ]
    }
}

fn should_run(condition: Option<&str>, context: &Context, default: bool) -> Result<bool, Error> {
    match condition {
        None => Ok(default),
        Some(condition) => Ok(gh_actions_expr::eval_condition(condition, context)?),
    }
}

fn run_defaults(workflow: &Workflow, job: &PlannedJob) -> RunDefaults {
    let of = |defaults: &Option<Defaults>| defaults.as_ref().and_then(|it| it.run.clone());
    let job = of(&job.spec.defaults);
    let workflow = of(&workflow.defaults);
    let pick = |get: fn(&RunDefaults) -> &Option<String>| {
        job.as_ref()
            .and_then(|it| get(it).clone())
            .or_else(|| workflow.as_ref().and_then(|it| get(it).clone()))
    };

    RunDefaults {
        shell: pick(|it| &it.shell),
        working_directory: pick(|it| &it.working_directory),
    }
}

fn minutes(count: u64) -> Duration {
    Duration::from_secs(count * 60)
}

fn job_timeout(job: &gh_actions_spec::NormalJob, context: &Context) -> Result<u64, Error> {
    match &job.timeout_minutes {
        None => Ok(360),
        Some(value) => number(value, context),
    }
}

fn step_timeout(step: &Step, context: &Context) -> Result<Option<Duration>, Error> {
    match &step.timeout_minutes {
        None => Ok(None),
        Some(value) => Ok(Some(minutes(number(value, context)?))),
    }
}

fn number(value: &gh_actions_spec::Expr<u64>, context: &Context) -> Result<u64, Error> {
    match value {
        gh_actions_spec::Expr::Value(number) => Ok(*number),
        gh_actions_spec::Expr::Expression(source) => {
            let text = interpolate_value(source, context)?.to_string();
            text.trim().parse().map_err(|_| {
                Error::Plan(format!("`{source}` is not a number of minutes: {text:?}"))
            })
        }
    }
}

fn flag(
    value: &Option<gh_actions_spec::Expr<bool>>,
    default: bool,
    context: &Context,
) -> Result<bool, Error> {
    match value {
        None => Ok(default),
        Some(gh_actions_spec::Expr::Value(value)) => Ok(*value),
        Some(gh_actions_spec::Expr::Expression(source)) => {
            Ok(interpolate_value(source, context)?.truthy())
        }
    }
}

fn continues_on_error(step: &Step, context: &Context) -> Result<bool, Error> {
    flag(&step.continue_on_error, false, context)
}

/// The `pre` hook that will not run, said the way GitHub says it, since an action that
/// counts on one is worth telling about rather than quietly leaving out.
fn passed_over(planned: &PlannedStep) -> Option<String> {
    let (Some(Uses::Local(path)), Some(resolved)) = (&planned.step.uses, &planned.action) else {
        return None;
    };
    Phase::Pre.script(&resolved.action.runs)?;

    Some(format!(
        "`pre` execution is not supported for local action from '{}'",
        path.display().to_string().trim_start_matches("./")
    ))
}

fn step_name(planned: &PlannedStep, context: &Context) -> Result<String, Error> {
    let step = &planned.step;
    let base = match (&step.name, &step.uses, &step.run) {
        (Some(name), _, _) => interpolate(name, context)?,
        (None, Some(uses), _) => uses.to_string(),
        (None, None, Some(run)) => run.lines().next().unwrap_or_default().trim().to_owned(),
        (None, None, None) => format!("step {}", planned.position + 1),
    };

    Ok(match planned.phase {
        Phase::Main => base,
        Phase::Pre => format!("{base} (pre)"),
        Phase::Post => format!("{base} (post)"),
    })
}

fn merged_env(workflow: &Workflow, job: &PlannedJob) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();

    for source in [workflow.env.as_ref(), job.spec.env.as_ref()]
        .into_iter()
        .flatten()
    {
        for (key, value) in source {
            env.insert(key.clone(), scalar_string(value));
        }
    }
    env
}

fn interpolated_env(
    env: Option<&BTreeMap<String, Scalar>>,
    context: &Context,
) -> Result<BTreeMap<String, String>, Error> {
    let mut resolved = BTreeMap::new();

    for (key, value) in env.into_iter().flatten() {
        resolved.insert(key.clone(), interpolate(&scalar_string(value), context)?);
    }
    Ok(resolved)
}

fn scalar_string(scalar: &Scalar) -> String {
    scalar_value(scalar).to_display_string()
}

/// A file written wrong is taken as nothing at all, and says which line it gave up on.
fn parse_env_file(contents: &str) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    let mut lines = contents.lines();

    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }

        if let Some((key, delimiter)) = line.split_once("<<") {
            let mut value = Vec::new();
            let mut closed = false;
            for line in lines.by_ref() {
                if line == delimiter {
                    closed = true;
                    break;
                }
                value.push(line);
            }
            if !closed {
                return Err(format!(
                    "Invalid value. Matching delimiter not found '{delimiter}'"
                ));
            }
            entries.insert(key.trim().to_owned(), value.join("\n"));
        } else if let Some((key, value)) = line.split_once('=') {
            entries.insert(key.trim().to_owned(), value.to_owned());
        } else {
            return Err(format!("Invalid format '{line}'"));
        }
    }

    Ok(entries)
}

fn read_lines(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Collected;

    #[test]
    fn input_names_become_environment_variables() {
        assert_eq!(input_variable("who-to-greet"), "INPUT_WHO-TO-GREET");
        assert_eq!(input_variable("who to greet"), "INPUT_WHO_TO_GREET");
        assert_eq!(input_variable("path"), "INPUT_PATH");
    }

    #[derive(Default)]
    struct Recorder {
        started: usize,
        finished: usize,
        ran: Vec<String>,
        code: Option<i32>,
        missing: bool,
    }

    impl Machine for Recorder {
        fn start(&mut self, _job: &PlannedJob, _out: &mut dyn Reporter) -> Result<Started, Error> {
            self.started += 1;

            Ok(match self.missing {
                true => Started::Missing,
                false => Started::Ready,
            })
        }

        fn exec(
            &mut self,
            request: &ExecRequest,
            _out: &mut dyn Reporter,
        ) -> Result<ExecResult, Error> {
            self.ran.push(request.name.clone());

            Ok(ExecResult {
                status: crate::executor::ExecStatus {
                    success: self.code.is_none(),
                    code: self.code.or(Some(0)),
                },
                commands: Vec::new(),
            })
        }

        fn finish(&mut self) -> Result<(), Error> {
            self.finished += 1;
            Ok(())
        }
    }

    fn ran(machine: &mut Recorder, steps: &str) -> Summary {
        let (workflow, plan) = crate::testing::workflow_of(steps);

        run(
            &workflow,
            &plan,
            &context(),
            &options(),
            machine,
            &mut Collected::default(),
        )
        .expect("the run itself goes through")
    }

    fn options() -> Options {
        let temp = std::env::temp_dir().join(format!("canopy-test-{}", std::process::id()));
        Options {
            workspace: temp.clone(),
            temp,
            cache: std::env::temp_dir().join("canopy-test-cache"),
            service_env: BTreeMap::new(),
            masks: Vec::new(),
        }
    }

    fn context() -> RunContext {
        let mut run = RunContext::default();
        run.github.event_name = "push".to_owned();
        run
    }

    #[test]
    fn a_job_that_fails_still_has_its_machine_cleaned_up() {
        let mut machine = Recorder {
            code: Some(1),
            ..Recorder::default()
        };

        let summary = ran(
            &mut machine,
            r"
      - run: echo hi
",
        );

        assert_eq!(
            summary.jobs,
            vec![("build".to_owned(), Conclusion::Failure)],
            "the job it could not run is the job that failed, and the run carries on"
        );
        assert_eq!(machine.started, 1);
        assert_eq!(
            machine.finished, 1,
            "a machine that was started has to be cleaned up, however the job ended"
        );
    }

    #[test]
    fn a_job_that_succeeds_cleans_up_once() {
        let mut machine = Recorder::default();

        ran(
            &mut machine,
            r"
      - run: echo hi
",
        );

        assert_eq!((machine.started, machine.finished), (1, 1));
    }

    #[test]
    fn a_forgiven_failure_counts_as_success_to_what_follows() {
        let mut machine = Recorder {
            code: Some(1),
            ..Recorder::default()
        };

        ran(
            &mut machine,
            r"
      - id: forgiven
        name: forgiven
        continue-on-error: true
        run: echo hi
      - name: conclusion
        if: steps.forgiven.conclusion == 'success'
        run: echo hi
      - name: outcome
        if: always() && steps.forgiven.outcome == 'failure'
        run: echo hi
",
        );

        assert_eq!(
            machine.ran,
            ["forgiven", "conclusion", "outcome"],
            "what the step did is a failure and what it counts as is a success, and the steps \
             after it can read both"
        );
    }

    #[test]
    fn a_step_that_was_passed_over_is_still_in_the_context() {
        let mut machine = Recorder::default();

        ran(
            &mut machine,
            r"
      - id: passed_over
        name: passed over
        if: github.event_name == 'schedule'
        run: echo hi
      - name: after
        if: steps.passed_over.outcome == 'skipped'
        run: echo hi
",
        );

        assert_eq!(
            machine.ran,
            ["after"],
            "a step nothing ran is remembered as one that was skipped"
        );
    }

    #[test]
    fn a_machine_missing_what_the_job_asked_for_still_runs_its_steps() {
        let mut machine = Recorder {
            missing: true,
            ..Recorder::default()
        };

        let summary = ran(
            &mut machine,
            r"
      - name: ordinary
        run: echo hi
      - name: whatever happened
        if: always()
        run: echo hi
",
        );

        assert_eq!(
            machine.ran,
            ["whatever happened"],
            "the job is failing before its first step, so only what runs anyway does"
        );
        assert_eq!(
            summary.jobs,
            vec![("build".to_owned(), Conclusion::Failure)]
        );
    }

    #[test]
    fn reads_both_env_file_forms() {
        let parsed = parse_env_file("SIMPLE=value\nNOTES<<EOF\nline one\nline two\nEOF\n")
            .expect("both forms are read");
        assert_eq!(parsed["SIMPLE"], "value");
        assert_eq!(parsed["NOTES"], "line one\nline two");
    }

    #[test]
    fn a_file_written_wrong_says_which_line_it_gave_up_on() {
        assert_eq!(
            parse_env_file("NEVER_CLOSED<<EOF\nline one\n"),
            Err("Invalid value. Matching delimiter not found 'EOF'".to_owned())
        );
        assert_eq!(
            parse_env_file("just some words\n"),
            Err("Invalid format 'just some words'".to_owned())
        );
    }
}
