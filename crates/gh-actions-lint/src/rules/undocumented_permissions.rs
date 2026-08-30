use gh_actions_spec::{PermissionLevel, Permissions};

use crate::rules::normal_jobs;
use crate::{Diagnostic, Rule, RuleInput};

/// Checks that a granted permission says why it was granted, since the reason is what tells
/// a later reader whether it is still needed.
pub struct UndocumentedPermissions;

impl Rule for UndocumentedPermissions {
    fn name(&self) -> &'static str {
        "undocumented-permissions"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        let mut granted = Vec::new();

        collect(&mut granted, "permissions", workflow.permissions.as_ref());
        for (id, job) in normal_jobs(workflow) {
            collect(
                &mut granted,
                &format!("jobs.{id}.permissions"),
                job.permissions.as_ref(),
            );
        }

        granted
            .into_iter()
            .filter(|location| !explained(input, location))
            .map(|location| {
                Diagnostic::warning(
                    self.name(),
                    location,
                    "nothing here says what this is granted for; a comment is what tells the \
                     next reader whether it is still needed",
                )
            })
            .collect()
    }
}

fn collect(granted: &mut Vec<String>, at: &str, permissions: Option<&Permissions>) {
    let Some(Permissions::Scopes(scopes)) = permissions else {
        return;
    };

    let named = scopes
        .iter()
        .filter(|(_, level)| **level != PermissionLevel::None)
        .map(|(scope, _)| format!("{at}.{scope}"));

    granted.extend(named);
}

fn explained(input: &RuleInput, location: &str) -> bool {
    let Some(node) = input.document.locate(location) else {
        return true;
    };
    let line = node.span.start.line;

    input.document.trailing_comment(line).is_some()
        || input.document.comments_above(line).next().is_some()
}

#[cfg(test)]
mod tests {
    use super::UndocumentedPermissions;
    use crate::tests::findings_of;

    #[test]
    fn a_permission_granted_without_a_reason_is_reported() {
        let findings = findings_of(
            &UndocumentedPermissions,
            r"
name: Build
on: push
permissions:
  contents: write
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo one
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].location, "permissions.contents");
    }

    #[test]
    fn one_with_a_reason_beside_it_or_above_it_is_fine() {
        let findings = findings_of(
            &UndocumentedPermissions,
            r"
name: Build
on: push
permissions:
  contents: write # so the release step can push a tag
  # the linter reads what it comments on
  packages: read
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo one
",
        );

        assert!(findings.is_empty());
    }

    #[test]
    fn what_is_granted_nothing_needs_no_reason() {
        let findings = findings_of(
            &UndocumentedPermissions,
            r"
name: Build
on: push
permissions:
  contents: none
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo one
",
        );

        assert!(findings.is_empty());
    }
}
