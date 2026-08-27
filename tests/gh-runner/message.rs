//! A planned job, in the shape the runner is handed one.

use std::collections::BTreeMap;

use gh_actions_context::RunContext;
use gh_actions_encoding::data;
use gh_actions_plan::PlannedJob;
use gh_actions_spec::{Container, Defaults, Expr, RunDefaults, Scalar, Step, Uses, Workflow};

pub fn encode(
    workflow: &Workflow,
    job: &PlannedJob,
    run: &RunContext,
    services: &BTreeMap<String, String>,
    base: &str,
    nth: u64,
) -> serde_json::Value {
    let steps: Vec<serde_json::Value> = job
        .spec
        .steps
        .iter()
        .flatten()
        .enumerate()
        .map(|(at, step)| {
            let (reference, mut inputs) = match (&step.run, &step.uses) {
                (Some(script), _) => (
                    serde_json::json!({ "type": "script" }),
                    BTreeMap::from([("script".to_owned(), script.clone())]),
                ),
                (None, Some(uses)) => (reference(uses), with(step)),
                (None, None) => (serde_json::json!({ "type": "script" }), BTreeMap::new()),
            };
            // The nearest `defaults.run` wins, which a service resolves before a runner
            // ever sees the step.
            let defaults = [&job.spec.defaults, &workflow.defaults];
            if let Some(shell) = step
                .shell
                .clone()
                .or_else(|| nearest(&defaults, |run| &run.shell))
            {
                inputs.insert("shell".to_owned(), shell);
            }
            if let Some(directory) = step
                .working_directory
                .clone()
                .or_else(|| nearest(&defaults, |run| &run.working_directory))
            {
                inputs.insert("workingDirectory".to_owned(), directory);
            }

            serde_json::json!({
                "type": "action",
                // Never all zeroes, which is the empty guid, and a group of its own so a
                // step is told apart from the job.
                "id": format!("{STEPS}{nth:04}-{:012}", at + 1),
                "name": step.id.clone().unwrap_or_else(|| format!("__step_{at}")),
                "contextName": step.id,
                "displayNameToken": interpolated(&called(step, at)),
                "reference": reference,
                "condition": step.r#if.clone().unwrap_or_else(|| "success()".to_owned()),
                "continueOnError": flag(step.continue_on_error.as_ref()),
                "timeoutInMinutes": flag(step.timeout_minutes.as_ref()),
                "inputs": mapping(&inputs),
                "environment": mapping(&env_of(workflow, job, step, services)),
            })
        })
        .collect();

    let mut contexts = serde_json::to_value(run).unwrap_or_default();
    if let Some(github) = contexts.get_mut("github").and_then(|it| it.as_object_mut()) {
        github.insert("job".to_owned(), job.id.clone().into());
        github.insert(
            "workflow".to_owned(),
            workflow.name.clone().unwrap_or_default().into(),
        );

        // What a run is numbered is said in words: a runner passes this context on to the
        // steps as it was given it, and a number reaches them as nothing at all.
        for value in github.values_mut() {
            if let Some(number) = value.as_u64() {
                *value = number.to_string().into();
            }
        }
    }
    if !job.matrix.is_empty()
        && let Ok(matrix) = serde_json::to_value(&job.matrix)
    {
        contexts["matrix"] = matrix;
    }

    let context_data: serde_json::Map<String, serde_json::Value> = contexts
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.clone(), data::written(value.clone())))
        .collect();

    serde_json::json!({
        "messageType": "PipelineAgentJobRequest",
        "plan": {
            "scopeIdentifier": "00000000-0000-0000-0000-000000000010",
            "planId": id(0x11, nth),
            "planType": "Build",
            // Without one a runner keeps to itself what a job ended up with.
            "version": 12,
        },
        "timeline": { "id": id(0x12, nth) },
        "jobId": id(0x13, nth),
        "jobDisplayName": job.label,
        "jobName": job.id,
        "jobOutputs": mapping(&job.spec.outputs.clone().unwrap_or_default()),
        "jobContainer": job.spec.container.as_ref().map(container),
        "jobServiceContainers": job.spec.services.as_ref().map(|services| {
            let alongside: Vec<serde_json::Value> = services
                .iter()
                .map(|(label, service)| {
                    serde_json::json!({ "Key": literal(label.clone()), "Value": container(service) })
                })
                .collect();

            serde_json::json!({ "type": 2, "map": alongside })
        }),
        "requestId": nth,
        "steps": steps,
        "contextData": context_data,
        "variables": {},
        "resources": { "endpoints": [{
            "name": "SystemVssConnection",
            "url": base,
            "authorization": { "scheme": "OAuth", "parameters": { "AccessToken": token() } },
            "data": {},
        }]},
        "maskHints": [],
        "fileTable": [],
    })
}

/// Handed to the actions as `ACTIONS_RUNTIME_TOKEN`: one that cannot read a run and a job
/// out of the claims will not upload anything.
fn token() -> String {
    #[derive(serde::Serialize)]
    struct Claims {
        nbf: u64,
        exp: u64,
        scp: String,
        iss: String,
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();

    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &Claims {
            nbf: now,
            exp: now + 3600,
            scp: format!(
                "Actions.Results:{}:{}",
                "00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002"
            ),
            iss: "local-actions-services".to_owned(),
        },
        &jsonwebtoken::EncodingKey::from_secret(b"canopy"),
    )
    .expect("the claims are serialisable")
}

fn reference(uses: &Uses) -> serde_json::Value {
    match uses {
        // A local action is named by `path` rather than by its name.
        Uses::Local(path) => serde_json::json!({
            "type": "repository",
            "name": "",
            "ref": "",
            "path": path.display().to_string().trim_start_matches("./"),
            "repositoryType": "self",
        }),
        Uses::Remote {
            owner,
            repo,
            subdir,
            reference,
        } => serde_json::json!({
            "type": "repository",
            "name": format!("{owner}/{repo}"),
            "ref": reference,
            "path": subdir,
            "repositoryType": "GitHub",
        }),
        Uses::Image(image) => serde_json::json!({
            "type": "containerRegistry",
            "image": image,
        }),
    }
}

fn container(container: &Container) -> serde_json::Value {
    let settings = match container {
        Container::Image(image) => return literal(image.clone()),
        Container::Settings(settings) => settings,
    };

    let mut of = vec![
        serde_json::json!({ "Key": literal("image".to_owned()), "Value": literal(settings.image.clone()) }),
    ];
    if let Some(options) = &settings.options {
        of.push(
            serde_json::json!({ "Key": literal("options".to_owned()), "Value": literal(options.clone()) }),
        );
    }
    if let Some(env) = &settings.env {
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(name, value)| (name.clone(), scalar(value)))
            .collect();

        of.push(serde_json::json!({ "Key": literal("env".to_owned()), "Value": mapping(&env) }));
    }
    if let Some(ports) = &settings.ports {
        let ports: Vec<serde_json::Value> =
            ports.iter().map(|port| literal(scalar(port))).collect();
        of.push(
            serde_json::json!({ "Key": literal("ports".to_owned()), "Value": { "type": 1, "seq": ports } }),
        );
    }
    if let Some(volumes) = &settings.volumes {
        let volumes: Vec<serde_json::Value> = volumes
            .iter()
            .map(|volume| literal(volume.clone()))
            .collect();
        of.push(
            serde_json::json!({ "Key": literal("volumes".to_owned()), "Value": { "type": 1, "seq": volumes } }),
        );
    }

    serde_json::json!({ "type": 2, "map": of })
}

/// What a step is called when it was not given a name, which a service works out before a
/// runner ever sees it.
fn called(step: &Step, at: usize) -> String {
    match (&step.name, &step.uses, &step.run) {
        (Some(name), _, _) => name.clone(),
        (None, Some(uses), _) => uses.to_string(),
        (None, None, Some(run)) => run.lines().next().unwrap_or_default().trim().to_owned(),
        (None, None, None) => format!("step {}", at + 1),
    }
}

fn nearest(
    defaults: &[&Option<Defaults>],
    of: impl Fn(&RunDefaults) -> &Option<String>,
) -> Option<String> {
    defaults
        .iter()
        .filter_map(|defaults| defaults.as_ref()?.run.as_ref())
        .find_map(|run| of(run).clone())
}

fn with(step: &Step) -> BTreeMap<String, String> {
    step.with
        .iter()
        .flatten()
        .map(|(name, value)| (name.clone(), scalar(value)))
        .collect()
}

fn env_of(
    workflow: &Workflow,
    job: &PlannedJob,
    step: &Step,
    services: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env = services.clone();
    env.extend(
        [&workflow.env, &job.spec.env, &step.env]
            .into_iter()
            .flatten()
            .flatten()
            .map(|(name, value)| (name.clone(), scalar(value))),
    );

    env
}

fn scalar(value: &Scalar) -> String {
    match value {
        Scalar::String(text) => text.clone(),
        Scalar::Bool(yes) => yes.to_string(),
        Scalar::Int(number) => number.to_string(),
        Scalar::Float(number) => number.to_string(),
    }
}

/// A runner evaluates expressions only where it is told there are any: a literal is taken
/// as it is, `${{ }}` and all, so an interpolated string is compiled into a `format` call.
fn interpolated(text: &str) -> serde_json::Value {
    let Some((before, rest)) = text.split_once("${{") else {
        return literal(text.to_owned());
    };

    let mut shape = quoted(before);
    let mut arguments = Vec::new();
    let mut rest = rest;

    loop {
        let Some((expression, after)) = rest.split_once("}}") else {
            return literal(text.to_owned());
        };
        shape.push_str(&format!("{{{}}}", arguments.len()));
        arguments.push(expression.trim().to_owned());

        match after.split_once("${{") {
            Some((between, next)) => {
                shape.push_str(&quoted(between));
                rest = next;
            }
            None => {
                shape.push_str(&quoted(after));
                break;
            }
        }
    }

    let call = format!("format('{shape}', {})", arguments.join(", "));
    serde_json::json!({ "type": 3, "expr": call })
}

fn quoted(text: &str) -> String {
    text.replace('\'', "''")
}

fn literal(value: impl Into<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "type": 0, "lit": value.into() })
}

/// An expression either way: a literal token is refused with `Unexpected value 'true'`
/// however it is spelled.
fn flag<T: ToString>(value: Option<&Expr<T>>) -> serde_json::Value {
    match value {
        Some(Expr::Value(value)) => serde_json::json!({ "type": 3, "expr": value.to_string() }),
        Some(Expr::Expression(source)) => serde_json::json!({ "type": 3, "expr": source }),
        None => serde_json::Value::Null,
    }
}

fn mapping(values: &BTreeMap<String, String>) -> serde_json::Value {
    let pairs: Vec<serde_json::Value> = values
        .iter()
        .map(|(key, value)| {
            serde_json::json!({ "Key": literal(key.clone()), "Value": interpolated(value) })
        })
        .collect();

    serde_json::json!({ "type": 2, "map": pairs })
}

pub const STEPS: &str = "00000000-0000-0001-";

fn id(kind: u16, nth: u64) -> String {
    format!("00000000-0000-{kind:04x}-{nth:04}-000000000000")
}
