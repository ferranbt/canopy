use gh_actions_expr::Expr;

use crate::rules::templated;
use crate::{Diagnostic, Rule, RuleInput};

/// Checks that a secret is asked for by name, since reaching for the whole context hands
/// over every secret the workflow can see.
pub struct OverprovisionedSecrets;

impl Rule for OverprovisionedSecrets {
    fn name(&self) -> &'static str {
        "overprovisioned-secrets"
    }

    fn check(&self, input: &RuleInput) -> Vec<Diagnostic> {
        let workflow = input.workflow;
        templated(workflow)
            .into_iter()
            .filter(|(_, parsed)| {
                parsed
                    .iter()
                    .flat_map(Expr::references)
                    .any(|reference| reference.context == "secrets" && reference.path.is_empty())
            })
            .map(|(location, _)| {
                Diagnostic::warning(
                    self.name(),
                    location,
                    "this reaches for the whole `secrets` context, which hands over every \
                     secret the workflow can see; name the ones it needs instead",
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::OverprovisionedSecrets;
    use crate::tests::findings_of;

    #[test]
    fn handing_over_every_secret_is_reported() {
        let findings = findings_of(
            &OverprovisionedSecrets,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: ./deploy
        env:
          EVERYTHING: ${{ toJSON(secrets) }}
",
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "overprovisioned-secrets");
        assert_eq!(findings[0].location, "jobs.build.steps[0].env.EVERYTHING");
    }

    #[test]
    fn naming_one_is_fine() {
        let findings = findings_of(
            &OverprovisionedSecrets,
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: ./deploy
        env:
          TOKEN: ${{ secrets.DEPLOY_TOKEN }}
",
        );

        assert!(findings.is_empty());
    }
}
