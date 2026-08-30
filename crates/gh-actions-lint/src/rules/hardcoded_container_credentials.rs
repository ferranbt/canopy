use gh_actions_spec::{Container, ContainerSettings};

use crate::rules::normal_jobs;
use crate::{Diagnostic, Rule, RuleInput};

/// Checks that the password a registry is reached with comes from a secret, since a workflow
/// file is readable by anyone who can read the repository.
pub struct HardcodedContainerCredentials;

impl Rule for HardcodedContainerCredentials {
    fn name(&self) -> &'static str {
        "hardcoded-container-credentials"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut findings = Vec::new();

        for (id, job) in normal_jobs(workflow) {
            let alongside = job.services.iter().flatten();
            let containers = job
                .container
                .iter()
                .map(|container| (format!("jobs.{id}.container"), container))
                .chain(
                    alongside
                        .map(|(label, service)| (format!("jobs.{id}.services.{label}"), service)),
                );

            for (location, container) in containers {
                let Container::Settings(settings) = container else {
                    continue;
                };
                if !written_down(settings) {
                    continue;
                }

                findings.push(Diagnostic::warning(
                    self.name(),
                    format!("{location}.credentials.password"),
                    "this password is written into the workflow, where anyone who can read \
                     the repository can read it; take it from `secrets` instead",
                ));
            }
        }

        findings
    }
}

fn written_down(settings: &ContainerSettings) -> bool {
    let Some(password) = settings
        .credentials
        .as_ref()
        .and_then(|credentials| credentials.password.as_deref())
    else {
        return false;
    };

    !password.contains("${{")
}

#[cfg(test)]
mod tests {
    use super::HardcodedContainerCredentials;
    use crate::tests::findings_of;

    #[test]
    fn a_password_written_into_the_workflow_is_reported() {
        let findings = findings_of(
            &HardcodedContainerCredentials,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    container:
      image: ghcr.io/canopy/build@sha256:0d0e2e2c3e2b7e0ef6a2f8e6b6a0e4cd2b7b3c1a9d8e7f6a5b4c3d2e1f0a9b8c
      credentials:
        username: canopy
        password: hunter2
    steps:
      - run: echo one
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "hardcoded-container-credentials");
        assert_eq!(
            findings[0].location,
            "jobs.build.container.credentials.password"
        );
    }

    #[test]
    fn a_service_is_looked_at_too() {
        let findings = findings_of(
            &HardcodedContainerCredentials,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    services:
      registry:
        image: ghcr.io/canopy/registry@sha256:0d0e2e2c3e2b7e0ef6a2f8e6b6a0e4cd2b7b3c1a9d8e7f6a5b4c3d2e1f0a9b8c
        credentials:
          username: canopy
          password: hunter2
    steps:
      - run: echo one
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].location,
            "jobs.build.services.registry.credentials.password"
        );
    }

    #[test]
    fn one_taken_from_a_secret_is_fine() {
        let findings = findings_of(
            &HardcodedContainerCredentials,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    container:
      image: ghcr.io/canopy/build@sha256:0d0e2e2c3e2b7e0ef6a2f8e6b6a0e4cd2b7b3c1a9d8e7f6a5b4c3d2e1f0a9b8c
      credentials:
        username: canopy
        password: ${{ secrets.REGISTRY_PASSWORD }}
    steps:
      - run: echo one
",
        );

        assert!(findings.is_empty());
    }
}
