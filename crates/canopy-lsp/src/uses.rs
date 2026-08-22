//! Finding the actions a workflow takes from GitHub, and where they are written.
//!
//! Read from the document, since what an editor needs is the place rather than the value.
//! Only repository actions: a `./action` or a `docker://image` has nothing to pin or date.

use std::str::FromStr;

use gh_actions_spec::Uses;
use yaml_with_spans::{Document, Node, Span};

pub struct Reference {
    pub owner: String,
    pub repo: String,
    pub at: String,
    pub span: Span,
    /// Just the part after the `@`, which is what pinning replaces.
    pub at_span: Span,
}

/// Jobs may use a whole workflow and steps may use an action; both are pinned the same way,
/// so both are collected.
pub fn references(document: &Document, source: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    let Some(jobs) = document.root.get("jobs").and_then(Node::as_mapping) else {
        return found;
    };

    for entry in jobs.entries() {
        let job = &entry.value;
        if let Some(node) = job.get("uses") {
            found.extend(reference(node, source));
        }

        let steps = job.get("steps").and_then(Node::as_sequence).unwrap_or(&[]);
        for step in steps {
            if let Some(node) = step.get("uses") {
                found.extend(reference(node, source));
            }
        }
    }

    found
}

fn reference(node: &Node, source: &str) -> Option<Reference> {
    let Ok(Uses::Remote {
        owner,
        repo,
        reference,
        ..
    }) = Uses::from_str(node.as_str()?)
    else {
        return None;
    };

    Some(Reference {
        owner,
        repo,
        at: reference,
        span: node.span,
        at_span: after_at(node, source)?,
    })
}

fn after_at(node: &Node, source: &str) -> Option<Span> {
    // A `uses:` is always written on one line, and the arithmetic below assumes it.
    if node.span.start.line != node.span.end.line {
        return None;
    }

    let written = node.span.of(source);
    // The span covers the quotes when there are any; the reference is inside them.
    let quoted = written.starts_with(['\'', '"']) && written.len() > 1;
    let inner = if quoted {
        &written[1..written.len() - 1]
    } else {
        written
    };
    let opening = usize::from(quoted);

    let at = inner.rfind('@')? + 1;
    let mut start = node.span.start;
    start.offset += opening + at;
    start.column += (opening + at) as u32;

    let mut end = start;
    end.offset += inner.len() - at;
    end.column += (inner.len() - at) as u32;

    Some(Span::new(start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(source: &str) -> Vec<Reference> {
        let document = Document::parse(source).expect("a document");
        references(&document, source)
    }

    #[test]
    fn finds_what_the_jobs_and_their_steps_use() {
        let found = find(concat!(
            "jobs:\n",
            "  build:\n",
            "    steps:\n",
            "      - uses: actions/checkout@v4\n",
            "      - run: echo hi\n",
            "      - uses: actions/setup-node@v4.1.0\n",
            "  ship:\n",
            "    uses: acme/workflows/.github/workflows/ship.yml@main\n",
        ));

        let named: Vec<_> = found.iter().map(|found| found.at.as_str()).collect();
        assert_eq!(named, ["v4", "v4.1.0", "main"]);
        assert_eq!(found[0].owner, "actions");
        assert_eq!(found[0].repo, "checkout");
    }

    #[test]
    fn what_cannot_be_pinned_is_left_alone() {
        let found = find(concat!(
            "jobs:\n",
            "  build:\n",
            "    steps:\n",
            "      - uses: ./.github/actions/greet\n",
            "      - uses: docker://alpine:3.20\n",
        ));

        assert!(found.is_empty());
    }

    #[test]
    fn points_at_the_reference_and_nothing_else() {
        let source = "jobs:\n  a:\n    steps:\n      - uses: actions/checkout@v4\n";
        let found = find(source);

        assert_eq!(found[0].at_span.of(source), "v4");
        assert_eq!(found[0].span.of(source), "actions/checkout@v4");
    }

    #[test]
    fn quoting_does_not_move_the_reference() {
        let source = "jobs:\n  a:\n    steps:\n      - uses: 'actions/checkout@v4'\n";
        let found = find(source);

        assert_eq!(found[0].at_span.of(source), "v4");
        assert_eq!(found[0].at_span.start.line, 3);
    }

    #[test]
    fn an_action_in_a_subdirectory_still_points_at_its_reference() {
        let source = "jobs:\n  a:\n    steps:\n      - uses: github/codeql-action/init@v3\n";
        let found = find(source);

        assert_eq!(found[0].at_span.of(source), "v3");
        assert_eq!(found[0].repo, "codeql-action");
    }

    #[test]
    fn a_reference_that_is_already_a_commit_is_found_too() {
        let sha = "9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0";
        let source = format!("jobs:\n  a:\n    steps:\n      - uses: actions/checkout@{sha}\n");
        let found = find(&source);

        assert_eq!(found[0].at_span.of(&source), sha);
    }
}
