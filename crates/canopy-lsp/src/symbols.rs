//! The jobs and steps of a workflow, as an editor wants to list them.
//!
//! Read from the document, not the typed workflow: a `BTreeMap` of jobs would list them
//! alphabetically rather than as written, and `deny_unknown_fields` would empty the outline
//! on the first typo. The jobs are the ones written, so a matrix is one symbol, not several.

use yaml_with_spans::{Document, Node, Span};

pub struct Job<'a> {
    pub id: &'a str,
    /// Just the `id:` key, for putting the cursor on.
    pub key: Span,
    pub whole: Span,
    pub node: &'a Node,
}

impl<'a> Job<'a> {
    pub fn name(&self) -> Option<&'a str> {
        self.node.get("name")?.as_str()
    }

    pub fn steps(&self) -> Vec<Step<'a>> {
        self.node
            .get("steps")
            .and_then(Node::as_sequence)
            .unwrap_or(&[])
            .iter()
            .map(|node| Step { node })
            .collect()
    }
}

pub struct Step<'a> {
    pub node: &'a Node,
}

impl<'a> Step<'a> {
    /// The same order the runner uses when it announces a step, so a step reads the same in
    /// the outline as it does in the log.
    pub fn name(&self) -> &'a str {
        if let Some(name) = self.node.get("name").and_then(Node::as_str) {
            return name;
        }
        if let Some(uses) = self.node.get("uses").and_then(Node::as_str) {
            return uses;
        }
        self.node
            .get("run")
            .and_then(Node::as_str)
            .map_or("step", |script| {
                script.lines().next().unwrap_or("step").trim()
            })
    }

    pub fn span(&self) -> Span {
        self.node.span
    }
}

pub fn jobs(document: &Document) -> Vec<Job<'_>> {
    let Some(jobs) = document.root.get("jobs").and_then(Node::as_mapping) else {
        return Vec::new();
    };

    jobs.entries()
        .iter()
        .filter_map(|entry| {
            let id = entry.key.as_str()?;
            Some(Job {
                id,
                key: entry.key.span,
                // From the key to the end of the body, which is the block as a whole.
                whole: Span::new(entry.key.span.start, entry.value.span.end),
                node: &entry.value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = r"name: Example
on: push

jobs:
  build:
    name: Build it
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Compile
        run: cargo build
      - run: |
          cargo test
          echo done
  ship:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - run: echo ship
";

    fn parse() -> Document {
        Document::parse(WORKFLOW).expect("a document")
    }

    #[test]
    fn finds_every_job_in_the_order_they_are_written() {
        let document = parse();
        let found = jobs(&document);

        let ids: Vec<&str> = found.iter().map(|job| job.id).collect();
        assert_eq!(ids, ["build", "ship"]);
        assert_eq!(found[0].name(), Some("Build it"));
        assert_eq!(found[1].name(), None);
    }

    #[test]
    fn a_job_points_at_its_key_and_covers_its_body() {
        let document = parse();
        let build = &jobs(&document)[0];

        assert_eq!(build.key.of(WORKFLOW), "build");
        assert_eq!(build.key.start.line, 4);
        // The block runs to the end of its last step, not just the key.
        assert!(build.whole.end.line >= 13, "{:?}", build.whole);
    }

    #[test]
    fn a_step_is_called_what_it_was_named_or_what_it_does() {
        let document = parse();
        let steps = jobs(&document)[0].steps();

        assert_eq!(steps.len(), 3);
        // Named, then the action it uses, then the first line of its script.
        assert_eq!(steps[0].name(), "actions/checkout@v4");
        assert_eq!(steps[1].name(), "Compile");
        assert_eq!(steps[2].name(), "cargo test");
    }

    #[test]
    fn a_workflow_with_no_jobs_has_no_symbols() {
        let document = Document::parse("on: push\n").expect("a document");

        assert!(jobs(&document).is_empty());
    }
}
