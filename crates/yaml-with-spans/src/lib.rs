//! A YAML reader that remembers where everything was, and what was said in the comments.
//!
//! Every other YAML library throws both away: a value tree is all a program usually needs.
//! Tools that talk back to a person need more — an editor cannot underline a value it cannot
//! locate, a file cannot be rewritten in place without byte offsets, and a directive written
//! in a comment is invisible to a reader that drops comments.
//!
//! It covers the YAML that configuration files are written in, not all of YAML 1.2. Anchors,
//! aliases and tags are refused rather than half-supported.

pub mod comment;
mod de;
mod error;
pub mod node;
mod parser;
mod path;
pub mod span;

pub use comment::Comment;
pub use de::{from_node, from_str};
pub use error::Error;
pub use node::{Entry, Mapping, Node, Value};
pub use span::{Position, Span};

/// A parsed document: its values, and the comments written around them.
#[derive(Debug, Clone)]
pub struct Document {
    pub root: Node,
    /// In the order they appear.
    pub comments: Vec<Comment>,
}

impl Document {
    pub fn parse(source: &str) -> Result<Self, Error> {
        let (root, comments) = parser::parse(source)?;
        Ok(Self { root, comments })
    }

    /// The node a path names, as in `jobs.build.steps[2].if`.
    pub fn at(&self, path: &str) -> Option<&Node> {
        path::lookup(&self.root, path)
    }

    /// The nearest node a path names, shortening it until something is found.
    ///
    /// A tool often has something to say about what is *not* written — a field left out, an
    /// index past the end — and that has no node of its own. Falling back to the innermost
    /// part of the path that does exist puts the remark beside what it is about.
    pub fn locate(&self, path: &str) -> Option<&Node> {
        let mut path = path;

        loop {
            if let Some(node) = self.at(path) {
                return Some(node);
            }
            path = path.rsplit_once('.')?.0;
        }
    }

    pub fn trailing_comment(&self, line: u32) -> Option<&Comment> {
        self.comments
            .iter()
            .find(|comment| comment.line() == line && !comment.own_line)
    }

    /// The comments written on their own lines directly above a line, nearest first.
    ///
    /// A blank line breaks the run, so a comment about the file as a whole is not mistaken
    /// for one about whatever happens to follow it.
    pub fn comments_above(&self, line: u32) -> impl Iterator<Item = &Comment> {
        let mut wanted = line;

        std::iter::from_fn(move || {
            let above = wanted.checked_sub(1)?;
            let comment = self
                .comments
                .iter()
                .find(|comment| comment.line() == above && comment.own_line)?;
            wanted = above;
            Some(comment)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Document {
        Document::parse(source).expect("a document")
    }

    #[test]
    fn reads_a_nested_mapping() {
        let document =
            parse("name: Example\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n");

        assert_eq!(document.at("name").unwrap().as_str(), Some("Example"));
        // `on` is a string, not the boolean YAML 1.1 would have made of it.
        assert_eq!(document.at("on").unwrap().as_str(), Some("push"));
        assert_eq!(
            document.at("jobs.build.runs-on").unwrap().as_str(),
            Some("ubuntu-latest")
        );
    }

    #[test]
    fn reads_a_sequence_of_compact_mappings() {
        let document =
            parse("steps:\n  - run: echo one\n  - name: two\n    uses: actions/checkout@v4\n");

        let steps = document.at("steps").unwrap().as_sequence().unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(
            document.at("steps[0].run").unwrap().as_str(),
            Some("echo one")
        );
        assert_eq!(
            document.at("steps[1].uses").unwrap().as_str(),
            Some("actions/checkout@v4")
        );
    }

    #[test]
    fn a_sequence_may_sit_at_its_keys_indentation() {
        let document = parse("jobs:\n  build:\n    steps:\n    - run: one\n    - run: two\n");

        assert_eq!(
            document.at("jobs.build.steps[1].run").unwrap().as_str(),
            Some("two")
        );
    }

    #[test]
    fn every_node_knows_where_it_is() {
        let source = "jobs:\n  build:\n    steps:\n      - run: echo hi\n";
        let document = parse(source);

        let run = document.at("jobs.build.steps[0].run").unwrap();
        assert_eq!(run.span.start.line, 3);
        assert_eq!(run.span.of(source), "echo hi");
    }

    #[test]
    fn a_key_knows_where_it_is_too() {
        let source = "jobs:\n  build:\n    needs: gone\n";
        let document = parse(source);

        let entry = document
            .at("jobs.build")
            .unwrap()
            .as_mapping()
            .unwrap()
            .entry("needs")
            .unwrap();
        assert_eq!(entry.key.span.start.line, 2);
        assert_eq!(entry.key.span.start.column, 4);
    }

    #[test]
    fn reads_a_literal_block() {
        let document = parse("run: |\n  one\n  two\nnext: after\n");

        assert_eq!(document.at("run").unwrap().as_str(), Some("one\ntwo\n"));
        assert_eq!(document.at("next").unwrap().as_str(), Some("after"));
    }

    #[test]
    fn a_block_keeps_the_indentation_inside_it() {
        let document = parse("run: |\n  if true; then\n    echo hi\n  fi\n");

        assert_eq!(
            document.at("run").unwrap().as_str(),
            Some("if true; then\n  echo hi\nfi\n")
        );
    }

    #[test]
    fn chomping_decides_the_trailing_newline() {
        assert_eq!(parse("a: |-\n  x\n").at("a").unwrap().as_str(), Some("x"));
        assert_eq!(parse("a: |\n  x\n").at("a").unwrap().as_str(), Some("x\n"));
        assert_eq!(
            parse("a: |+\n  x\n\n\n").at("a").unwrap().as_str(),
            Some("x\n\n\n")
        );
    }

    #[test]
    fn a_folded_block_joins_its_lines() {
        let document = parse("a: >\n  one\n  two\n\n  three\n");

        assert_eq!(document.at("a").unwrap().as_str(), Some("one two\nthree\n"));
    }

    #[test]
    fn a_plain_scalar_folds_onto_the_lines_below_it() {
        let document =
            parse("if: github.event_name == 'push' &&\n    github.ref == 'main'\nrun: x\n");

        assert_eq!(
            document.at("if").unwrap().as_str(),
            Some("github.event_name == 'push' && github.ref == 'main'")
        );
        assert_eq!(document.at("run").unwrap().as_str(), Some("x"));
    }

    #[test]
    fn reads_flow_collections() {
        let document = parse("needs: [one, two]\nwith: { a: 1, b: two }\n");

        assert_eq!(document.at("needs[1]").unwrap().as_str(), Some("two"));
        assert_eq!(document.at("with.a").unwrap().value, Value::Int(1));
        assert_eq!(document.at("with.b").unwrap().as_str(), Some("two"));
    }

    #[test]
    fn quoting_keeps_a_value_a_string() {
        let document = parse("a: 'true'\nb: \"1.0\"\nc: true\nd: 1.0\ne: 12\n");

        assert_eq!(document.at("a").unwrap().as_str(), Some("true"));
        assert_eq!(document.at("b").unwrap().as_str(), Some("1.0"));
        assert_eq!(document.at("c").unwrap().value, Value::Bool(true));
        assert_eq!(document.at("d").unwrap().value, Value::Float(1.0));
        assert_eq!(document.at("e").unwrap().value, Value::Int(12));
    }

    #[test]
    fn a_version_is_not_a_number() {
        let document = parse("version: 1.2.3\nimage: ubuntu-24.04\n");

        assert_eq!(document.at("version").unwrap().as_str(), Some("1.2.3"));
        assert_eq!(document.at("image").unwrap().as_str(), Some("ubuntu-24.04"));
    }

    #[test]
    fn a_key_without_a_value_is_null() {
        let document = parse("on:\n  push:\n  workflow_dispatch:\n");

        assert!(document.at("on.push").unwrap().is_null());
        assert!(document.at("on.workflow_dispatch").unwrap().is_null());
    }

    #[test]
    fn escapes_are_read() {
        let document = parse(r#"a: "one\ntwo\t\"quoted\"!""#);

        assert_eq!(
            document.at("a").unwrap().as_str(),
            Some("one\ntwo\t\"quoted\"!")
        );
    }

    #[test]
    fn keeps_the_comments() {
        let source = "# about the file\n\njobs:\n  build:\n    uses: a/b@v4 # canopy:ignore\n";
        let document = parse(source);

        let uses = document.at("jobs.build.uses").unwrap();
        let trailing = document.trailing_comment(uses.span.start.line).unwrap();
        assert_eq!(trailing.text, "canopy:ignore");
        assert!(!trailing.own_line);

        assert_eq!(document.comments.len(), 2);
        assert!(document.comments[0].own_line);
    }

    #[test]
    fn a_comment_is_not_taken_for_a_value() {
        let document = parse("run: echo hi # says hello\nnext: x\n");

        assert_eq!(document.at("run").unwrap().as_str(), Some("echo hi"));
        // Only a `#` after a space starts a comment, so a URL fragment survives.
        assert_eq!(
            parse("url: http://x/y#z\n").at("url").unwrap().as_str(),
            Some("http://x/y#z")
        );
    }

    #[test]
    fn finds_the_comments_written_above_a_line() {
        let source = "# far above\n\n# nearest\n# also near\nrun: x\n";
        let document = parse(source);

        let above: Vec<_> = document
            .comments_above(4)
            .map(|comment| comment.text.as_str())
            .collect();
        assert_eq!(above, ["also near", "nearest"]);
    }

    #[test]
    fn refuses_what_it_will_not_pretend_to_read() {
        let error = Document::parse("a: &anchor x\nb: *anchor\n").unwrap_err();

        assert!(error.message.contains("anchors"), "{}", error.message);
        assert_eq!(error.position.line, 0);
    }

    #[test]
    fn reports_where_it_gave_up() {
        let error = Document::parse("a: \"unterminated\n").unwrap_err();

        assert!(error.to_string().starts_with("2:"), "{error}");
    }
}
