//! Data model of the GitHub Actions workflow file format.

pub mod action;
pub mod common;
pub mod events;
pub mod job;
pub mod step;
pub mod uses;
pub mod workflow;

pub use action::{
    Action, ActionInput, ActionOutput, Branding, CompositeRuns, DockerRuns, NodeRuns, Runs,
};
pub use common::{
    Concurrency, ConcurrencySettings, Defaults, Env, Expr, OneOrMany, PermissionLevel, Permissions,
    PermissionsAll, RunDefaults, Scalar, With,
};
pub use events::{
    CallInput, CallOutput, CallSecret, DispatchInput, Events, InputType, On, RefFilters, Schedule,
    WorkflowCall, WorkflowDispatch, WorkflowRun,
};
pub use job::{
    Container, ContainerCredentials, ContainerSettings, Environment, EnvironmentDetails, Job,
    Matrix, MatrixLiteral, MatrixValue, NormalJob, ReusableJob, RunnerGroup, RunsOn, Secrets,
    SecretsInherit, Strategy,
};
pub use step::Step;
pub use uses::{Uses, UsesError};
pub use workflow::Workflow;

#[cfg(test)]
mod tests {
    use super::Workflow;

    #[test]
    fn parses_fixtures() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
        let mut parsed = 0;

        for entry in std::fs::read_dir(dir).expect("fixtures directory") {
            let path = entry.expect("fixture entry").path();
            if path
                .extension()
                .is_none_or(|ext| ext != "yml" && ext != "yaml")
            {
                continue;
            }

            let yaml = std::fs::read_to_string(&path).expect("read fixture");
            yaml_with_spans::from_str::<Workflow>(&yaml)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            parsed += 1;
        }

        assert!(parsed > 0, "no fixtures found in {dir}");
    }
}
