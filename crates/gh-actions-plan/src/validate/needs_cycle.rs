//! Jobs that need each other in a circle can never start.

use std::collections::{BTreeSet, HashMap};

use gh_actions_spec::Workflow;

use crate::Diagnostic;
use crate::validate::needs_exist::needs_of;

pub const NAME: &str = "needs-cycle";

pub(crate) fn check(workflow: &Workflow) -> Vec<Diagnostic> {
    let mut visited: HashMap<&str, State> = HashMap::new();
    // Each cycle is reported once, under the job that closes it.
    let mut reported = BTreeSet::new();
    let mut findings = Vec::new();

    for id in workflow.jobs.keys() {
        let mut stack = Vec::new();
        walk(
            id,
            workflow,
            &mut visited,
            &mut stack,
            &mut reported,
            &mut findings,
            NAME,
        );
    }

    findings
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Being explored; meeting it again is a cycle.
    Open,
    Done,
}

fn walk<'a>(
    id: &'a str,
    workflow: &'a Workflow,
    visited: &mut HashMap<&'a str, State>,
    stack: &mut Vec<&'a str>,
    reported: &mut BTreeSet<String>,
    findings: &mut Vec<Diagnostic>,
    rule: &'static str,
) {
    match visited.get(id) {
        Some(State::Done) => return,
        Some(State::Open) => {
            let start = stack.iter().position(|entry| *entry == id).unwrap_or(0);
            let mut loop_path: Vec<&str> = stack[start..].to_vec();
            loop_path.push(id);

            // Name the cycle by its members so the same loop is not reported per entry.
            let mut members: Vec<&str> = loop_path.clone();
            members.sort_unstable();
            members.dedup();
            if reported.insert(members.join(",")) {
                findings.push(Diagnostic::error(
                    rule,
                    format!("jobs.{id}.needs"),
                    format!("these jobs need each other: {}", loop_path.join(" -> ")),
                ));
            }
            return;
        }
        None => {}
    }

    let Some((key, job)) = workflow.jobs.get_key_value(id) else {
        return;
    };
    visited.insert(key, State::Open);
    stack.push(key);

    for need in needs_of(job) {
        walk(need, workflow, visited, stack, reported, findings, rule);
    }

    stack.pop();
    visited.insert(key, State::Done);
}

#[cfg(test)]
mod tests {
    use crate::validate::tests::check_source;

    #[test]
    fn a_loop_is_refused() {
        let findings = check_source(
            r"
on: push
jobs:
  a:
    needs: b
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
  b:
    needs: a
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
",
        );

        let cycles: Vec<_> = findings
            .iter()
            .filter(|finding| finding.rule == "needs-cycle")
            .collect();
        assert_eq!(cycles.len(), 1, "one cycle, reported once: {findings:?}");
        assert!(cycles[0].message.contains("need each other"));
    }

    #[test]
    fn a_chain_is_fine() {
        let findings = check_source(
            r"
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
  b:
    needs: a
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
  c:
    needs: [a, b]
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
",
        );

        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }
}
