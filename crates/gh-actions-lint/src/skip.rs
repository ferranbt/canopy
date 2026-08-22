//! Silencing a rule inside the workflow.
//!
//! ```yaml
//! - run: echo hi
//!   if: ${{ github.event_name }}   # canopy:ignore
//!
//!   # canopy:ignore needs-exist expression-syntax
//! - uses: ./local
//! ```
//!

use yaml_with_spans::Document;

use crate::Diagnostic;

/// What a skip directive starts with.
const DIRECTIVE: &str = "canopy:ignore";

/// Whether the workflow itself asks for this finding to be passed over.
pub fn ignored(document: &Document, finding: &Diagnostic) -> bool {
    let Some(node) = document.locate(&finding.location) else {
        return false;
    };
    let line = node.span.start.line;

    let trailing = document.trailing_comment(line);
    let above = document.comments_above(line);

    trailing
        .into_iter()
        .chain(above)
        .any(|comment| silences(&comment.text, finding.rule))
}

/// Whether a comment silences a rule.
fn silences(comment: &str, rule: &str) -> bool {
    let Some((_, rest)) = comment.split_once(DIRECTIVE) else {
        return false;
    };

    let mut named = rest
        .split([' ', ',', '\t'])
        .filter(|name| !name.is_empty())
        .peekable();

    // Naming nothing silences every rule; naming some silences only those.
    named.peek().is_none() || named.any(|name| name == rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_directive_silences_anything() {
        assert!(silences("canopy:ignore", "needs-exist"));
        assert!(silences("canopy:ignore", "step-shape"));
    }

    #[test]
    fn a_named_directive_silences_only_what_it_names() {
        assert!(silences("canopy:ignore needs-exist", "needs-exist"));
        assert!(!silences("canopy:ignore needs-exist", "step-shape"));
        assert!(silences(
            "canopy:ignore needs-exist, step-shape",
            "step-shape"
        ));
    }

    #[test]
    fn a_directive_may_follow_something_else_in_the_comment() {
        assert!(silences(
            "fixed in #123 canopy:ignore needs-exist",
            "needs-exist"
        ));
    }

    #[test]
    fn an_ordinary_comment_silences_nothing() {
        assert!(!silences("this needs to be fixed", "needs-exist"));
        assert!(!silences("ignore this", "needs-exist"));
    }
}
