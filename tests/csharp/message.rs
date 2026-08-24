//! A planned job, in the shape the runner is handed one.
//!
//! What a service does when it compiles a workflow: the steps as references and tokens,
//! the contexts in the encoding they travel in, and the endpoints pointing back at
//! whoever is serving it.

use std::collections::BTreeMap;

use gh_actions_context::RunContext;
use gh_actions_plan::PlannedJob;
use gh_actions_spec::{Defaults, Expr, RunDefaults, Scalar, Step, Uses, Workflow};

/// A planned job as the runner expects to be handed one.
///
/// Straight to what goes over the wire: the two encodings the service wraps things in are
/// applied where they belong, since plain JSON does not say which of them a map wants.
pub fn encode(
    workflow: &Workflow,
    job: &PlannedJob,
    run: &RunContext,
    needs: &BTreeMap<String, serde_json::Value>,
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
                // A group of its own, so a step is told apart from the job whatever it
                // was called, and never all zeroes, which is the empty guid.
                "id": format!("{STEPS}{nth:04}-{:012}", at + 1),
                "name": step.id.clone().unwrap_or_else(|| format!("__step_{at}")),
                "contextName": step.id,
                "displayNameToken": interpolated(&called(step, at)),
                "reference": reference,
                "condition": step.r#if.clone().unwrap_or_else(|| "success()".to_owned()),
                "continueOnError": flag(step.continue_on_error.as_ref()),
                "timeoutInMinutes": flag(step.timeout_minutes.as_ref()),
                "inputs": mapping(&inputs),
                // The nearest `env` wins, so what the workflow and the job set is
                // folded in underneath the step's own.
                "environment": mapping(&env_of(workflow, job, step, services)),
            })
        })
        .collect();

    // The contexts as an expression sees them, which is what the runner is handed.
    let mut contexts = serde_json::to_value(run).unwrap_or_default();
    // Which job of which workflow is only known once one is picked to run.
    if let Some(github) = contexts.get_mut("github").and_then(|it| it.as_object_mut()) {
        github.insert("job".to_owned(), job.id.clone().into());
        github.insert(
            "workflow".to_owned(),
            workflow.name.clone().unwrap_or_default().into(),
        );
    }
    // What the jobs this one waited for came out with, which the service keeps rather than
    // any one runner.
    if !needs.is_empty()
        && let Ok(needs) = serde_json::to_value(needs)
    {
        contexts["needs"] = needs;
    }

    // Which combination of a matrix this job is, which is settled by planning it rather
    // than by anything the run as a whole knows.
    if !job.matrix.is_empty()
        && let Ok(matrix) = serde_json::to_value(&job.matrix)
    {
        contexts["matrix"] = matrix;
    }

    let context_data: serde_json::Map<String, serde_json::Value> = contexts
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.clone(), context(value)))
        .collect();

    serde_json::json!({
        "messageType": "PipelineAgentJobRequest",
        "plan": {
            "scopeIdentifier": "00000000-0000-0000-0000-000000000010",
            "planId": id(0x11, nth),
            "planType": "Build",
            // Without one a runner keeps to itself what a job ended up with, which is what
            // the jobs after it are given.
            "version": 12,
        },
        "timeline": { "id": id(0x12, nth) },
        "jobId": id(0x13, nth),
        "jobDisplayName": job.label,
        "jobName": job.id,
        // What the job is to come out with, which a runner works out at the end from what
        // its steps left behind.
        "jobOutputs": mapping(&job.spec.outputs.clone().unwrap_or_default()),
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

/// A token shaped the way the runner insists on, which is a JWT it can read the life of.
fn token() -> String {
    #[derive(serde::Serialize)]
    struct Claims {
        nbf: u64,
        exp: u64,
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
        },
        &jsonwebtoken::EncodingKey::from_secret(b"canopy"),
    )
    .expect("the claims are serialisable")
}

/// What a `uses:` step points at, however it was written.
fn reference(uses: &Uses) -> serde_json::Value {
    match uses {
        // In the repository the job belongs to: what it is called is the path to it,
        // which the runner reads from `path` rather than from the name.
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

/// The inputs an action was given, as strings, since that is all a token carries.
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

/// A value that may carry expressions, in the encoding steps arrive in.
///
/// The runner evaluates expressions, but only where it is told there are any: a literal is
/// taken as it is, `${{ }}` and all. What the service does is compile an interpolated
/// string into a `format` call, which is what this does too.
fn interpolated(text: &str) -> serde_json::Value {
    let Some((before, rest)) = text.split_once("${{") else {
        return literal(text.to_owned());
    };

    let mut shape = quoted(before);
    let mut arguments = Vec::new();
    let mut rest = rest;

    loop {
        let Some((expression, after)) = rest.split_once("}}") else {
            // Nothing closes it, so there is nothing to compile.
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

/// Text inside an expression's single quotes, where a quote is written twice.
fn quoted(text: &str) -> String {
    text.replace('\'', "''")
}

/// A literal, in the encoding steps arrive in.
fn literal(value: impl Into<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "type": 0, "lit": value.into() })
}

/// What a step said, when it said it plainly.
///
/// An expression either way: these are evaluated rather than read, and a literal token is
/// refused with `Unexpected value 'true'` however it is spelled.
fn flag<T: ToString>(value: Option<&Expr<T>>) -> serde_json::Value {
    match value {
        Some(Expr::Value(value)) => serde_json::json!({ "type": 3, "expr": value.to_string() }),
        Some(Expr::Expression(source)) => serde_json::json!({ "type": 3, "expr": source }),
        None => serde_json::Value::Null,
    }
}

/// A mapping, in the encoding steps arrive in.
fn mapping(values: &BTreeMap<String, String>) -> serde_json::Value {
    let pairs: Vec<serde_json::Value> = values
        .iter()
        .map(|(key, value)| {
            serde_json::json!({ "Key": literal(key.clone()), "Value": interpolated(value) })
        })
        .collect();

    serde_json::json!({ "type": 2, "map": pairs })
}

/// A value, in the other encoding: the one the contexts arrive in.
fn context(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::json!({ "t": 1, "a": items.iter().map(context).collect::<Vec<_>>() })
        }
        serde_json::Value::Object(fields) => {
            let entries: Vec<serde_json::Value> = fields
                .iter()
                .map(|(key, value)| serde_json::json!({ "k": key, "v": context(value) }))
                .collect();

            serde_json::json!({ "t": 2, "d": entries })
        }
        scalar => scalar.clone(),
    }
}

/// What the ids of the steps this hands over start with.
pub const STEPS: &str = "00000000-0000-0001-";

/// One of the ids a job is known by, which no two jobs on one runner may share.
fn id(kind: u16, nth: u64) -> String {
    format!("00000000-0000-{kind:04x}-{nth:04}-000000000000")
}
