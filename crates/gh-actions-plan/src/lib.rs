//! Turns a workflow into jobs to run, and decides which of them may run next.

pub mod contexts;
mod diagnostic;
pub mod validate;

use std::collections::{BTreeMap, HashSet};
use std::fmt;

pub use diagnostic::{Diagnostic, Severity, has_errors};

use gh_actions_context::{Conclusion, JobResult};
use gh_actions_expr::Value;
use gh_actions_spec::{Job, Matrix, MatrixValue, NormalJob, Scalar, Workflow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Invalid(Diagnostic),
    Plan(String),
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Without the severity, which being an error already says.
            Self::Invalid(finding) => write!(
                f,
                "{} [{}] {}",
                finding.location, finding.rule, finding.message
            ),
            Self::Plan(message) => write!(f, "cannot plan workflow: {message}"),
            Self::Unsupported(what) => write!(f, "not supported yet: {what}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Plan {
    pub jobs: Vec<PlannedJob>,
}

#[derive(Debug, Clone)]
pub struct PlannedJob {
    pub id: String,
    pub label: String,
    pub matrix: BTreeMap<String, Value>,
    pub needs: Vec<String>,
    pub spec: NormalJob,
}

impl Plan {
    pub fn select(&self, id: &str) -> Result<Self, Error> {
        if !self.jobs.iter().any(|job| job.id == id) {
            return Err(Error::Plan(format!("no job {id:?} in this workflow")));
        }

        // Walk backwards so a job is always seen before the jobs it needs.
        let mut wanted: HashSet<&str> = HashSet::from([id]);
        for job in self.jobs.iter().rev() {
            if wanted.contains(job.id.as_str()) {
                wanted.extend(job.needs.iter().map(String::as_str));
            }
        }

        Ok(Self {
            jobs: self
                .jobs
                .iter()
                .filter(|job| wanted.contains(job.id.as_str()))
                .cloned()
                .collect(),
        })
    }

    /// Returns every job that is ready, not just the first, so a caller that wants to run
    /// them at the same time can.
    pub fn ready<'a>(&'a self, finished: &BTreeMap<String, JobResult>) -> Vec<&'a PlannedJob> {
        self.jobs
            .iter()
            .filter(|job| !finished.contains_key(&job.id))
            .filter(|job| job.needs.iter().all(|need| finished.contains_key(need)))
            .collect()
    }

    pub fn needs_satisfied(job: &PlannedJob, finished: &BTreeMap<String, JobResult>) -> bool {
        job.needs.iter().all(|need| {
            finished
                .get(need)
                .is_some_and(|result| result.conclusion == Conclusion::Success)
        })
    }
}

/// Validation runs first and is the only thing that decides whether a workflow can run, so
/// everything below may take a sound workflow for granted: `needs` names a job that is
/// there, the graph has no loop, every step is a step. Nothing re-checks that.
pub fn plan(workflow: &Workflow) -> Result<Plan, Error> {
    if let Some(fatal) = validate::check(workflow)
        .into_iter()
        .find(|finding| finding.severity == Severity::Error)
    {
        return Err(Error::Invalid(fatal));
    }

    let mut jobs = BTreeMap::new();

    for (id, job) in &workflow.jobs {
        match job {
            Job::Normal(normal) => {
                jobs.insert(id.clone(), (**normal).clone());
            }
            Job::Reusable(_) => {
                return Err(Error::Unsupported(format!(
                    "job {id:?} calls a reusable workflow with `uses:`"
                )));
            }
        }
    }

    let order = topological_order(&jobs);
    let mut planned = Vec::new();

    for id in order {
        let spec = &jobs[&id];
        let needs = needs_of(spec);

        for matrix in expand_matrix(spec)? {
            planned.push(PlannedJob {
                id: id.clone(),
                label: label_for(&id, &matrix),
                matrix,
                needs: needs.clone(),
                spec: spec.clone(),
            });
        }
    }

    Ok(Plan { jobs: planned })
}

fn needs_of(job: &NormalJob) -> Vec<String> {
    job.needs
        .as_ref()
        .map(|needs| needs.as_slice().to_vec())
        .unwrap_or_default()
}

/// Takes the graph as sound, because [`plan`] has already had it validated: a `needs` that
/// names nothing, or a loop, is refused before anything gets here. The grey marking below
/// is only what makes the walk terminate, not a check.
fn topological_order(jobs: &BTreeMap<String, NormalJob>) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut order = Vec::new();

    // Depth first, marking a job on the way in so the walk cannot go round for ever.
    fn visit<'a>(
        id: &'a str,
        jobs: &'a BTreeMap<String, NormalJob>,
        seen: &mut HashSet<&'a str>,
        order: &mut Vec<String>,
    ) {
        if !seen.insert(id) {
            return;
        }

        for need in needs_of(&jobs[id]) {
            if let Some((key, _)) = jobs.get_key_value(&need) {
                visit(key, jobs, seen, order);
            }
        }

        order.push(id.to_owned());
    }

    for id in jobs.keys() {
        visit(id, jobs, &mut seen, &mut order);
    }

    order
}

/// A job without a matrix yields one empty combination.
fn expand_matrix(job: &NormalJob) -> Result<Vec<BTreeMap<String, Value>>, Error> {
    let Some(strategy) = &job.strategy else {
        return Ok(vec![BTreeMap::new()]);
    };
    let Some(matrix) = &strategy.matrix else {
        return Ok(vec![BTreeMap::new()]);
    };
    let literal = match matrix {
        Matrix::Literal(literal) => literal,
        Matrix::Expression(_) => {
            return Err(Error::Unsupported(
                "a `matrix` built from an expression".to_owned(),
            ));
        }
    };

    // Every axis multiplies the combinations built so far.
    let mut combinations: Vec<BTreeMap<String, Value>> = vec![BTreeMap::new()];
    for (axis, values) in &literal.axes {
        let mut expanded = Vec::with_capacity(combinations.len() * values.len());
        for combination in &combinations {
            for value in values {
                let mut next = combination.clone();
                next.insert(axis.clone(), matrix_value(value));
                expanded.push(next);
            }
        }
        combinations = expanded;
    }

    if let Some(excludes) = &literal.exclude {
        let excludes: Vec<BTreeMap<String, Value>> = excludes.iter().map(convert_map).collect();
        combinations.retain(|combination| {
            !excludes
                .iter()
                .any(|exclude| matches_partially(combination, exclude))
        });
    }

    if let Some(includes) = &literal.include {
        for include in includes.iter().map(convert_map) {
            apply_include(&mut combinations, &literal.axes.keys().collect(), &include);
        }
    }

    Ok(combinations)
}

fn apply_include(
    combinations: &mut Vec<BTreeMap<String, Value>>,
    axes: &HashSet<&String>,
    include: &BTreeMap<String, Value>,
) {
    // Only keys that are axes decide which combinations an include applies to.
    let selector: BTreeMap<String, Value> = include
        .iter()
        .filter(|(key, _)| axes.contains(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    let mut applied = false;
    for combination in combinations.iter_mut() {
        if matches_partially(combination, &selector) {
            for (key, value) in include {
                combination.insert(key.clone(), value.clone());
            }
            applied = true;
        }
    }

    // An include that matches nothing becomes a combination of its own.
    if !applied {
        combinations.push(include.clone());
    }
}

fn matches_partially(
    combination: &BTreeMap<String, Value>,
    filter: &BTreeMap<String, Value>,
) -> bool {
    filter
        .iter()
        .all(|(key, value)| combination.get(key).is_some_and(|found| found == value))
}

fn convert_map(map: &BTreeMap<String, MatrixValue>) -> BTreeMap<String, Value> {
    map.iter()
        .map(|(key, value)| (key.clone(), matrix_value(value)))
        .collect()
}

fn matrix_value(value: &MatrixValue) -> Value {
    match value {
        MatrixValue::Scalar(scalar) => scalar_value(scalar),
        MatrixValue::List(items) => Value::Array(items.iter().map(matrix_value).collect()),
        MatrixValue::Map(fields) => Value::Object(convert_map(fields)),
    }
}

pub fn scalar_value(scalar: &Scalar) -> Value {
    match scalar {
        Scalar::String(s) => Value::String(s.clone()),
        Scalar::Bool(b) => Value::Bool(*b),
        Scalar::Int(n) => Value::Number(*n as f64),
        Scalar::Float(n) => Value::Number(*n),
    }
}

fn label_for(id: &str, matrix: &BTreeMap<String, Value>) -> String {
    if matrix.is_empty() {
        return id.to_owned();
    }
    let values: Vec<String> = matrix.values().map(Value::to_display_string).collect();
    format!("{id} ({})", values.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(yaml: &str) -> Plan {
        let workflow: Workflow = yaml_with_spans::from_str(yaml).expect("workflow parses");
        plan(&workflow).expect("workflow plans")
    }

    #[test]
    fn orders_jobs_by_needs() {
        let plan = plan_of(
            r"
on: push
jobs:
  deploy:
    needs: [build]
    runs-on: ubuntu-latest
    steps: []
  build:
    needs: lint
    runs-on: ubuntu-latest
    steps: []
  lint:
    runs-on: ubuntu-latest
    steps: []
",
        );
        let ids: Vec<&str> = plan.jobs.iter().map(|job| job.id.as_str()).collect();
        assert_eq!(ids, ["lint", "build", "deploy"]);
    }

    #[test]
    fn rejects_cycles_and_missing_jobs() {
        let cyclic: Workflow = yaml_with_spans::from_str(
            r"
on: push
jobs:
  a:
    needs: b
    runs-on: ubuntu-latest
  b:
    needs: a
    runs-on: ubuntu-latest
",
        )
        .unwrap();
        // Refused by validation, which says where and under which rule — the planner no
        // longer works that out for itself.
        let Err(Error::Invalid(finding)) = plan(&cyclic) else {
            panic!("a cycle should be refused");
        };
        assert_eq!(finding.rule, "needs-cycle");

        let missing: Workflow = yaml_with_spans::from_str(
            r"
on: push
jobs:
  a:
    needs: ghost
    runs-on: ubuntu-latest
",
        )
        .unwrap();
        let Err(Error::Invalid(finding)) = plan(&missing) else {
            panic!("a missing job should be refused");
        };
        assert_eq!(finding.rule, "needs-exist");
        assert_eq!(finding.location, "jobs.a.needs");
    }

    #[test]
    fn expands_matrix_with_include_and_exclude() {
        let plan = plan_of(
            r"
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
        rust: [stable, nightly]
        exclude:
          - os: macos-latest
            rust: nightly
        include:
          - os: ubuntu-latest
            rust: stable
            coverage: true
    steps: []
",
        );

        assert_eq!(plan.jobs.len(), 3);
        let labels: Vec<&str> = plan.jobs.iter().map(|job| job.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "test (true, ubuntu-latest, stable)",
                "test (ubuntu-latest, nightly)",
                "test (macos-latest, stable)",
            ]
        );

        // The include only extends the combination it selects.
        let covered = plan
            .jobs
            .iter()
            .filter(|job| job.matrix.contains_key("coverage"))
            .count();
        assert_eq!(covered, 1);
    }

    #[test]
    fn include_without_a_match_adds_a_combination() {
        let plan = plan_of(
            r"
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        os: [ubuntu-latest]
        include:
          - os: windows-latest
    steps: []
",
        );
        assert_eq!(plan.jobs.len(), 2);
    }

    #[test]
    fn a_job_without_a_matrix_runs_once() {
        let plan = plan_of(
            r"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps: []
",
        );
        assert_eq!(plan.jobs.len(), 1);
        assert_eq!(plan.jobs[0].label, "build");
        assert!(plan.jobs[0].matrix.is_empty());
    }
}
