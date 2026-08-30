use gh_actions_spec::{Job, Secrets, Workflow};

use crate::{Contexts, Diagnostic, Rule};

/// Checks that a called workflow is handed the secrets it needs rather than all of them.
pub struct SecretsInherit;

impl Rule for SecretsInherit {
    fn name(&self) -> &'static str {
        "secrets-inherit"
    }

    fn check(&self, workflow: &Workflow, _contexts: &Contexts) -> Vec<Diagnostic> {
        workflow
            .jobs
            .iter()
            .filter_map(|(id, job)| match job {
                Job::Reusable(called) => Some((id, called)),
                Job::Normal(_) => None,
            })
            .filter(|(_, called)| matches!(called.secrets, Some(Secrets::Inherit(_))))
            .map(|(id, called)| {
                Diagnostic::warning(
                    self.name(),
                    format!("jobs.{id}.secrets"),
                    format!(
                        "`secrets: inherit` hands {:?} every secret this workflow can see; \
                         name the ones it needs instead",
                        called.uses
                    ),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::lint_source;

    #[test]
    fn inheriting_every_secret_is_reported() {
        let findings = lint_source(
            r"
name: Call
on: push
jobs:
  called:
    uses: ./.github/workflows/other.yml
    secrets: inherit
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "secrets-inherit");
        assert_eq!(findings[0].location, "jobs.called.secrets");
    }

    #[test]
    fn naming_the_secrets_is_fine() {
        let findings = lint_source(
            r"
name: Call
on: push
jobs:
  called:
    uses: ./.github/workflows/other.yml
    secrets:
      token: ${{ secrets.PUBLISH_TOKEN }}
",
        );

        assert!(findings.is_empty());
    }
}
