//! Which contexts an expression reads, and by what path.

use crate::ast::Expr;

/// One step of a path into a context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// A property, or an index that is a string literal: `.name`, `['name']`.
    Name(String),
    /// A step this cannot name: a numeric index, or one worked out while the run happens.
    Unknown,
    Star,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// e.g. `github` or `needs`.
    pub context: String,
    /// Outermost first.
    pub path: Vec<Segment>,
}

impl Reference {
    /// The first thing read from the context, when the expression names it.
    pub fn first(&self) -> Option<&str> {
        match self.path.first() {
            Some(Segment::Name(name)) => Some(name),
            _ => None,
        }
    }

    /// The path, as far as it is named, stopping at the first step that is not.
    pub fn named(&self) -> Vec<&str> {
        self.path
            .iter()
            .map_while(|segment| match segment {
                Segment::Name(name) => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }
}

impl Expr {
    pub fn references(&self) -> Vec<Reference> {
        let mut found = Vec::new();
        collect(self, &mut found);
        found
    }
}

/// Each chain of accesses counts as one reference.
fn collect(expr: &Expr, found: &mut Vec<Reference>) {
    match expr {
        Expr::Context(_) | Expr::Property(..) | Expr::Index(..) | Expr::Star(_) => {
            let mut path = Vec::new();
            if let Some(context) = unwind(expr, &mut path, found) {
                path.reverse();
                found.push(Reference { context, path });
            }
        }
        Expr::Not(inner) => collect(inner, found),
        Expr::Binary(_, left, right) => {
            collect(left, found);
            collect(right, found);
        }
        Expr::Call(_, args) => {
            for arg in args {
                collect(arg, found);
            }
        }
        _ => {}
    }
}

/// Follows a chain of accesses to the context at its root, collecting the path on the way.
///
/// Returns nothing when the chain is rooted at something else, e.g. `fromJSON(x).name`.
fn unwind(expr: &Expr, path: &mut Vec<Segment>, found: &mut Vec<Reference>) -> Option<String> {
    match expr {
        Expr::Context(name) => Some(name.clone()),
        Expr::Property(target, name) => {
            path.push(Segment::Name(name.clone()));
            unwind(target, path, found)
        }
        Expr::Index(target, index) => {
            path.push(match index.as_ref() {
                Expr::String(name) => Segment::Name(name.clone()),
                other => {
                    collect(other, found);
                    Segment::Unknown
                }
            });
            unwind(target, path, found)
        }
        Expr::Star(target) => {
            path.push(Segment::Star);
            unwind(target, path, found)
        }
        other => {
            collect(other, found);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn references(source: &str) -> Vec<Reference> {
        Expr::parse(source).expect("expression parses").references()
    }

    #[test]
    fn a_path_is_kept_to_its_full_depth() {
        let found = references("needs.build.outputs.version");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].context, "needs");
        assert_eq!(found[0].named(), ["build", "outputs", "version"]);
    }

    #[test]
    fn a_bare_context_has_no_path() {
        let found = references("matrix");

        assert_eq!(found[0].context, "matrix");
        assert!(found[0].path.is_empty());
        assert_eq!(found[0].first(), None);
    }

    #[test]
    fn an_index_written_as_a_string_names_what_it_reads() {
        let found = references("needs['build'].outputs");

        assert_eq!(found[0].first(), Some("build"));
        assert_eq!(found[0].named(), ["build", "outputs"]);
    }

    #[test]
    fn an_index_that_is_an_expression_is_unknown_and_reports_itself() {
        let found = references("needs[matrix.job].outputs");

        // The chain, with a hole where the index is, and what the index itself reads.
        let chain = found
            .iter()
            .find(|reference| reference.context == "needs")
            .expect("the chain is reported");
        assert_eq!(
            chain.path,
            [Segment::Unknown, Segment::Name("outputs".to_owned())]
        );
        assert_eq!(chain.first(), None);
        assert!(chain.named().is_empty());

        let index = found
            .iter()
            .find(|reference| reference.context == "matrix")
            .expect("the index is reported too");
        assert_eq!(index.named(), ["job"]);
    }

    #[test]
    fn a_star_filter_is_a_step_of_its_own() {
        let found = references("github.event.commits.*.message");

        assert_eq!(
            found[0].path,
            [
                Segment::Name("event".to_owned()),
                Segment::Name("commits".to_owned()),
                Segment::Star,
                Segment::Name("message".to_owned()),
            ]
        );
        // Naming stops at the filter, which is as far as a rule can follow it.
        assert_eq!(found[0].named(), ["event", "commits"]);
    }

    #[test]
    fn references_are_found_inside_calls_and_operators() {
        let found = references("contains(steps.a.outputs.x, matrix.os) && !failure()");
        let contexts: Vec<&str> = found.iter().map(|r| r.context.as_str()).collect();

        assert_eq!(contexts, ["steps", "matrix"]);
    }

    #[test]
    fn a_chain_rooted_in_a_call_reads_no_context_of_its_own() {
        let found = references("fromJSON(steps.meta.outputs.json).name");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].context, "steps");
        assert_eq!(found[0].named(), ["meta", "outputs", "json"]);
    }
}
