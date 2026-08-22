//! Finding a node by the path a tool names it with.
//!
//! Tools describe a place in a workflow as `jobs.build.steps[2].if`, which is what a person
//! reading a report wants to see. This turns that back into the node it names.

use crate::node::Node;

pub(crate) fn lookup<'a>(root: &'a Node, path: &str) -> Option<&'a Node> {
    let mut node = root;

    for segment in path.split('.').filter(|segment| !segment.is_empty()) {
        let (key, indexes) = split_indexes(segment);
        if !key.is_empty() {
            node = node.get(key)?;
        }
        for index in indexes {
            node = node.index(index?)?;
        }
    }

    Some(node)
}

/// Splits `steps[2]` into its key and the indexes that follow it.
fn split_indexes(segment: &str) -> (&str, impl Iterator<Item = Option<usize>> + '_) {
    let key = segment.split('[').next().unwrap_or(segment);
    let rest = &segment[key.len()..];

    let indexes = rest
        .split('[')
        .filter(|part| !part.is_empty())
        .map(|part| part.strip_suffix(']')?.parse().ok());

    (key, indexes)
}
