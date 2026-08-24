//! Workflow commands: the `::name key=value::message` lines steps print to talk back.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    SaveState { name: String, value: String },
    SetOutput { name: String, value: String },
    AddPath(String),
    SetEnv { name: String, value: String },
    AddMask(String),
    Message { level: Level, text: String },
    Group(String),
    EndGroup,
    Stop(String),
    Echo(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Notice,
    Warning,
    Error,
}

impl Command {
    pub fn parse(line: &str) -> Option<Self> {
        let rest = line.trim_start().strip_prefix("::")?;
        let (head, message) = rest.split_once("::")?;
        let (name, properties) = match head.split_once(' ') {
            Some((name, properties)) => (name, parse_properties(properties)),
            None => (head, BTreeMap::new()),
        };

        let message = unescape_message(message);
        let named = |key: &str| properties.get(key).cloned().unwrap_or_default();

        Some(match name {
            "save-state" => Self::SaveState {
                name: named("name"),
                value: message,
            },
            "set-output" => Self::SetOutput {
                name: named("name"),
                value: message,
            },
            "add-path" => Self::AddPath(message),
            "set-env" => Self::SetEnv {
                name: named("name"),
                value: message,
            },
            "add-mask" => Self::AddMask(message),
            "stop-commands" => Self::Stop(message),
            "echo" => Self::Echo(message == "on"),
            "debug" => Self::Message {
                level: Level::Debug,
                text: message,
            },
            "warning" => Self::Message {
                level: Level::Warning,
                text: message,
            },
            "error" => Self::Message {
                level: Level::Error,
                text: message,
            },
            "group" => Self::Group(message),
            "endgroup" => Self::EndGroup,
            // Anything else is a command the runner GitHub ships has no name for, `::notice::`
            // among them, and what it does with those is print them.
            _ => return None,
        })
    }

    pub fn to_event(&self) -> Option<crate::report::Event> {
        use crate::report::Event;

        match self {
            // A level raised over nothing is raised over nothing, and is left where it was.
            Self::Message { text, .. } if text.is_empty() => None,
            Self::Message { level, text } => Some(Event::Message {
                level: match level {
                    Level::Debug => crate::report::Level::Debug,
                    Level::Notice => crate::report::Level::Notice,
                    Level::Warning => crate::report::Level::Warning,
                    Level::Error => crate::report::Level::Error,
                },
                text: text.clone(),
            }),
            Self::Group(name) => Some(Event::Progress {
                text: format!("[{name}]"),
            }),
            _ => None,
        }
    }
}

fn parse_properties(properties: &str) -> BTreeMap<String, String> {
    properties
        .split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), unescape_property(value)))
        .collect()
}

fn unescape_message(text: &str) -> String {
    text.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%25", "%")
}

fn unescape_property(text: &str) -> String {
    unescape_message(&text.replace("%3A", ":").replace("%2C", ","))
}

pub fn state_variable(name: &str) -> String {
    format!("STATE_{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_command_with_properties() {
        assert_eq!(
            Command::parse("::save-state name=CACHE_KEY::deps-Linux-v1"),
            Some(Command::SaveState {
                name: "CACHE_KEY".to_owned(),
                value: "deps-Linux-v1".to_owned(),
            })
        );
    }

    #[test]
    fn reads_a_command_without_properties() {
        assert_eq!(
            Command::parse("::add-mask::http://127.0.0.1:44295/cache/2"),
            Some(Command::AddMask(
                "http://127.0.0.1:44295/cache/2".to_owned()
            ))
        );
        assert_eq!(Command::parse("::endgroup::"), Some(Command::EndGroup));
    }

    #[test]
    fn undoes_the_escaping() {
        let parsed = Command::parse("::error file=a%3Ab.js,line=1::first%0Asecond%25");
        assert_eq!(
            parsed,
            Some(Command::Message {
                level: Level::Error,
                text: "first\nsecond%".to_owned(),
            })
        );
    }

    #[test]
    fn ordinary_output_is_left_alone() {
        assert!(Command::parse("building the project").is_none());
        assert!(Command::parse("a line mentioning :: in passing").is_none());
        assert!(Command::parse("::unknown-command::whatever").is_none());
    }

    #[test]
    fn state_is_handed_back_the_way_actions_read_it() {
        assert_eq!(state_variable("CACHE_KEY"), "STATE_CACHE_KEY");
    }
}
