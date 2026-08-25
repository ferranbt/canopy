use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use criterion::{Criterion, criterion_group, criterion_main};

use gh_actions_context::RunContext;
use gh_actions_expr::{Context, eval_condition, interpolate};
use gh_actions_plan::{Plan, PlannedJob};
use gh_actions_runner::commands::Command;
use gh_actions_runner::report::{Event, Reporter};
use gh_actions_runner::{Error, ExecRequest, ExecResult, Machine, Options, Started};
use gh_actions_spec::Workflow;

struct Pretend;

impl Machine for Pretend {
    fn start(&mut self, _job: &PlannedJob, _out: &mut dyn Reporter) -> Result<Started, Error> {
        Ok(Started::Ready)
    }

    fn run(
        &mut self,
        _program: &str,
        _args: &[String],
        _request: &ExecRequest,
        _out: &mut dyn Reporter,
    ) -> Result<ExecResult, Error> {
        Ok(ExecResult::default())
    }

    fn found(&mut self, program: &str) -> String {
        program.to_owned()
    }

    fn finish(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

struct Quiet;

impl Reporter for Quiet {
    fn report(&mut self, _event: Event) {}
}

const FIXTURES: [&str; 4] = ["tiny", "steps", "matrix", "expressions"];

fn source(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("workflows")
        .join(format!("{name}.yml"));

    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn read(name: &str) -> (Workflow, Plan) {
    let workflow: Workflow = yaml_with_spans::from_str(&source(name)).expect("a workflow");
    let plan = gh_actions_plan::plan(&workflow).expect("a workflow that plans");

    (workflow, plan)
}

fn directory(name: &str) -> PathBuf {
    let at = std::env::temp_dir()
        .join(format!("canopy-bench-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).expect("a directory to run in");

    at
}

fn parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");

    for name in FIXTURES {
        let source = source(name);
        group.bench_function(name, |b| {
            b.iter(|| yaml_with_spans::from_str::<Workflow>(&source).expect("a workflow"))
        });
    }
}

fn plan(c: &mut Criterion) {
    let mut group = c.benchmark_group("plan");

    for name in FIXTURES {
        let (workflow, _) = read(name);
        group.bench_function(name, |b| {
            b.iter(|| gh_actions_plan::plan(&workflow).expect("a workflow that plans"))
        });
    }
}

fn job(c: &mut Criterion) {
    let mut group = c.benchmark_group("job");

    for name in FIXTURES {
        let (workflow, plan) = read(name);
        let workspace = directory(name);
        let options = Options {
            temp: workspace.join("temp"),
            cache: workspace.join("cache"),
            workspace,
            service_env: BTreeMap::new(),
            masks: Vec::new(),
        };
        let context = RunContext::default();

        group.bench_function(name, |b| {
            b.iter(|| {
                gh_actions_runner::run(
                    &workflow,
                    &plan,
                    &context,
                    &options,
                    &mut Pretend,
                    &mut Quiet,
                )
                .expect("a workflow that runs")
            })
        });
    }
}

const EXPRESSIONS: [(&str, &str); 5] = [
    ("nothing", "nothing to interpolate at all"),
    ("context", "${{ github.event_name }}"),
    (
        "format",
        "${{ format('{0}-{1}', runner.os, github.run_number) }}",
    ),
    ("json", "${{ fromJSON('[\"one\", \"two\"]')[1] }}"),
    (
        "several calls",
        "${{ contains('one two three', 'two') && startsWith(github.workflow, 'a') }}",
    ),
];

const CONDITIONS: [(&str, &str); 3] = [
    ("one call", "success()"),
    (
        "several parts",
        "success() && github.event_name == 'push' && !cancelled()",
    ),
    (
        "reading a step",
        "steps.named.conclusion == 'success' || failure()",
    ),
];

fn expr(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr");
    let context: Context = RunContext::default().to_expr_context();

    for (name, source) in EXPRESSIONS {
        group.bench_function(format!("interpolate {name}"), |b| {
            b.iter(|| interpolate(source, &context).expect("an expression that evaluates"))
        });
    }

    for (name, source) in CONDITIONS {
        group.bench_function(format!("condition {name}"), |b| {
            b.iter(|| eval_condition(source, &context).expect("a condition that evaluates"))
        });
    }
}

const PRINTED: [&str; 6] = [
    "::set-output name=where::somewhere",
    "::add-mask::a secret",
    "::group::A group of lines",
    "::error file=main.rs,line=12::something went wrong",
    "::debug::a line only a debug run sees",
    "a line that is not a command at all",
];

fn commands(c: &mut Criterion) {
    let mut group = c.benchmark_group("commands");

    group.bench_function("a block of output", |b| {
        b.iter(|| {
            PRINTED
                .iter()
                .filter_map(|line| Command::parse(line))
                .count()
        })
    });
}

criterion_group!(benches, parse, plan, job, expr, commands);
criterion_main!(benches);
