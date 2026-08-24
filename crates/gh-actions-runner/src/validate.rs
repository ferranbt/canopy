use crate::steps::{Phase, PlannedStep};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub step: String,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Unexpected {
        given: String,
        declared: Vec<String>,
    },
    MissingRequired {
        name: String,
    },
    Deprecated {
        name: String,
        message: String,
    },
}

impl Finding {
    pub fn message(&self) -> String {
        match &self.problem {
            Problem::Unexpected { given, declared } => {
                let valid = if declared.is_empty() {
                    "it declares none".to_owned()
                } else {
                    format!("valid inputs are {}", declared.join(", "))
                };
                format!("{}: unexpected input {given:?}, {valid}", self.step)
            }
            Problem::MissingRequired { name } => {
                format!("{}: required input {name:?} was not supplied", self.step)
            }
            Problem::Deprecated { name, message } => {
                format!("{}: input {name:?} is deprecated: {message}", self.step)
            }
        }
    }
}

/// What GitHub says when a step passes an action something it never declared, word for word,
/// and only that: it says nothing about a `required` input that was left out.
pub fn unexpected(
    step: &gh_actions_spec::Step,
    action: &gh_actions_spec::Action,
) -> Option<String> {
    let declared = action.inputs.clone().unwrap_or_default();
    let given: Vec<String> = step
        .with
        .iter()
        .flatten()
        .map(|(name, _)| name.clone())
        .filter(|name| !declared.contains_key(name))
        .collect();

    if given.is_empty() {
        return None;
    }

    let quoted = |names: &[String]| {
        names
            .iter()
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let valid: Vec<String> = declared.keys().cloned().collect();

    Some(format!(
        "Unexpected input(s) {}, valid inputs are [{}]",
        quoted(&given),
        quoted(&valid)
    ))
}

/// Nothing here fails a run: GitHub warns about unexpected inputs and does not enforce
/// `required` at all, leaving that to the action itself.
pub fn inputs(steps: &[PlannedStep]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for planned in steps {
        // Hooks share their step's inputs, so checking the main step is enough.
        if planned.phase != Phase::Main {
            continue;
        }
        let Some(resolved) = &planned.action else {
            continue;
        };

        let name = planned
            .step
            .name
            .clone()
            .or_else(|| planned.step.uses.as_ref().map(ToString::to_string))
            .unwrap_or_else(|| format!("step {}", planned.position + 1));
        let declared = resolved.action.inputs.clone().unwrap_or_default();
        let given = planned.step.with.clone().unwrap_or_default();

        for supplied in given.keys() {
            if !declared.contains_key(supplied) {
                findings.push(Finding {
                    step: name.clone(),
                    problem: Problem::Unexpected {
                        given: supplied.clone(),
                        declared: declared.keys().cloned().collect(),
                    },
                });
            }
        }

        for (input, declaration) in &declared {
            if let Some(message) = &declaration.deprecation_message
                && given.contains_key(input)
            {
                findings.push(Finding {
                    step: name.clone(),
                    problem: Problem::Deprecated {
                        name: input.clone(),
                        message: message.clone(),
                    },
                });
            }

            let missing = declaration.required.unwrap_or(false)
                && declaration.default.is_none()
                && !given.contains_key(input);
            if missing {
                findings.push(Finding {
                    step: name.clone(),
                    problem: Problem::MissingRequired {
                        name: input.clone(),
                    },
                });
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gh_actions_spec::{Action, ActionInput, NodeRuns, Runs, Scalar, Step, Uses};

    use super::*;
    use crate::actions::ResolvedAction;

    fn step(with: &[(&str, &str)], declared: Vec<(&str, ActionInput)>) -> PlannedStep {
        let action = Action {
            name: "greet".to_owned(),
            description: None,
            author: None,
            inputs: Some(
                declared
                    .into_iter()
                    .map(|(name, input)| (name.to_owned(), input))
                    .collect(),
            ),
            outputs: None,
            runs: Runs::Node20(NodeRuns {
                main: "index.js".to_owned(),
                ..NodeRuns::default()
            }),
            branding: None,
        };

        PlannedStep {
            step: Step {
                name: Some("greet".to_owned()),
                uses: Some(Uses::Local("./greet".into())),
                with: Some(
                    with.iter()
                        .map(|(key, value)| (key.to_string(), Scalar::String(value.to_string())))
                        .collect::<BTreeMap<_, _>>(),
                ),
                ..Step::default()
            },
            position: 0,
            phase: Phase::Main,
            action: Some(ResolvedAction {
                action,
                path: "./greet".into(),
            }),
            script: None,
            condition: None,
        }
    }

    fn declared(required: bool, default: Option<&str>) -> ActionInput {
        ActionInput {
            required: Some(required),
            default: default.map(|value| Scalar::String(value.to_owned())),
            ..ActionInput::default()
        }
    }

    #[test]
    fn an_undeclared_input_is_reported() {
        let findings = inputs(&[step(
            &[("who-to-gret", "world")],
            vec![("who-to-greet", declared(false, None))],
        )]);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].message().contains("unexpected input"));
        assert!(findings[0].message().contains("who-to-greet"));
    }

    #[test]
    fn a_declared_input_passes() {
        let findings = inputs(&[step(
            &[("who-to-greet", "world")],
            vec![("who-to-greet", declared(false, None))],
        )]);

        assert!(findings.is_empty());
    }

    #[test]
    fn a_required_input_with_a_default_is_not_missing() {
        let findings = inputs(&[step(&[], vec![("who", declared(true, Some("world")))])]);

        assert!(findings.is_empty());
    }

    #[test]
    fn a_required_input_without_one_is_reported() {
        let findings = inputs(&[step(&[], vec![("token", declared(true, None))])]);

        assert_eq!(
            findings[0].problem,
            Problem::MissingRequired {
                name: "token".to_owned()
            }
        );
    }

    #[test]
    fn a_deprecated_input_is_only_reported_when_used() {
        let deprecated = ActionInput {
            deprecation_message: Some("use `token` instead".to_owned()),
            ..ActionInput::default()
        };

        assert!(inputs(&[step(&[], vec![("old", deprecated.clone())])]).is_empty());
        assert_eq!(
            inputs(&[step(&[("old", "x")], vec![("old", deprecated)])]).len(),
            1
        );
    }
}
