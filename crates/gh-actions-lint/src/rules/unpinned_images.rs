use gh_actions_spec::{Container, Uses, Workflow};

use crate::rules::normal_jobs;
use crate::{Contexts, Diagnostic, Rule};

/// Checks that an image is asked for by digest, since a tag is whatever the registry it came
/// from says it is today.
pub struct UnpinnedImages;

impl Rule for UnpinnedImages {
    fn name(&self) -> &'static str {
        "unpinned-images"
    }

    fn check(&self, workflow: &Workflow, _contexts: &Contexts) -> Vec<Diagnostic> {
        let mut findings = Vec::new();
        let mut report = |location: String, image: &str| {
            findings.push(Diagnostic::warning(
                self.name(),
                location,
                format!(
                    "`{image}` is whatever the registry has under that tag when the job runs; \
                     ask for a digest instead"
                ),
            ));
        };

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
                let image = match container {
                    Container::Image(image) => image,
                    Container::Settings(settings) => &settings.image,
                };
                if pinned(image) {
                    continue;
                }

                report(format!("{location}.image"), image);
            }

            for (position, step) in job.steps.iter().flatten().enumerate() {
                let Some(Uses::Image(image)) = &step.uses else {
                    continue;
                };
                if pinned(image) {
                    continue;
                }

                report(crate::rules::step_location(id, position, "uses"), image);
            }
        }

        findings
    }
}

fn pinned(image: &str) -> bool {
    image.contains("@sha256:")
}

#[cfg(test)]
mod tests {
    use crate::tests::lint_source;

    #[test]
    fn a_tag_is_reported_wherever_the_image_is_asked_for() {
        let findings = lint_source(
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    container: debian:bookworm-slim
    services:
      cache:
        image: redis:7-alpine
    steps:
      - uses: docker://alpine:3.20
",
        );

        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].rule, "unpinned-images");
        assert_eq!(findings[0].location, "jobs.build.container.image");
        assert_eq!(findings[1].location, "jobs.build.services.cache.image");
        assert_eq!(findings[2].location, "jobs.build.steps[0].uses");
    }

    #[test]
    fn a_digest_is_fine() {
        let findings = lint_source(
            r"
name: Build
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    container:
      image: debian@sha256:0d0e2e2c3e2b7e0ef6a2f8e6b6a0e4cd2b7b3c1a9d8e7f6a5b4c3d2e1f0a9b8c
    steps:
      - run: echo one
",
        );

        assert!(findings.is_empty());
    }
}
