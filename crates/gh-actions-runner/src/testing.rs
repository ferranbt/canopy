use gh_actions_plan::Plan;
use gh_actions_spec::{Step, Workflow};

pub fn workflow_of(steps: &str) -> (Workflow, Plan) {
    let source =
        format!("on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n{steps}");
    let workflow: Workflow = yaml_with_spans::from_str(&source).expect("a workflow");
    let plan = gh_actions_plan::plan(&workflow).expect("a workflow that plans");

    (workflow, plan)
}

pub fn steps_of(steps: &str) -> Vec<Step> {
    let (_, plan) = workflow_of(steps);

    plan.jobs
        .into_iter()
        .flat_map(|job| job.spec.steps.unwrap_or_default())
        .collect()
}
