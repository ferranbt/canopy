use gh_actions_spec::{Permissions, PermissionsAll};

use crate::rules::normal_jobs;
use crate::{Diagnostic, Rule, RuleInput};

/// Checks what the token a job runs with is allowed to do: everything it is given is
/// something an action it runs can do with it.
pub struct ExcessivePermissions;

impl Rule for ExcessivePermissions {
    fn name(&self) -> &'static str {
        "excessive-permissions"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut findings = Vec::new();

        match &workflow.permissions {
            Some(Permissions::All(PermissionsAll::WriteAll)) => findings.push(Diagnostic::warning(
                self.name(),
                "permissions",
                "`write-all` gives every job here a token that can write anything in the \
                 repository; grant the scopes each job needs instead",
            )),
            None => findings.push(Diagnostic::warning(
                self.name(),
                "jobs",
                "nothing here says what the token may do, so every job runs with whatever \
                 the repository allows by default; start from `permissions: {}`",
            )),
            _ => {}
        }

        for (id, job) in normal_jobs(workflow) {
            if matches!(
                job.permissions,
                Some(Permissions::All(PermissionsAll::WriteAll))
            ) {
                findings.push(Diagnostic::warning(
                    self.name(),
                    format!("jobs.{id}.permissions"),
                    "`write-all` gives this job a token that can write anything in the \
                     repository; grant the scopes it needs instead",
                ));
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::ExcessivePermissions;
    use crate::tests::findings_of;

    #[test]
    fn saying_nothing_is_reported() {
        let findings = findings_of(
            &ExcessivePermissions,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo one
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "excessive-permissions");
        assert_eq!(findings[0].location, "jobs");
    }

    #[test]
    fn writing_anything_is_reported_at_either_level() {
        let findings = findings_of(
            &ExcessivePermissions,
            r"
name: Build
on: push
permissions: write-all
jobs:
  build:
    runs-on: ubuntu-latest
    permissions: write-all
    steps:
      - run: echo one
",
        );

        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].location, "permissions");
        assert_eq!(findings[1].location, "jobs.build.permissions");
    }

    #[test]
    fn starting_from_nothing_and_granting_what_is_needed_is_fine() {
        let findings = findings_of(
            &ExcessivePermissions,
            r"
name: Build
on: push
permissions: {}
jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - run: echo one
",
        );

        assert!(findings.is_empty());
    }
}
