use gh_actions_context::Conclusion;
use serde::{Deserialize, Serialize};

/// The reason why a job was never run
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PassedOver {
    /// Its own `if`, or a job it needed, said no.
    Skipped,
    /// Its matrix was called off by a sibling failing.
    Cancelled,
}

impl PassedOver {
    pub fn name(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stream {
    #[serde(rename = "stdout")]
    Out,
    #[serde(rename = "stderr")]
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Debug,
    Notice,
    Warning,
    Error,
}

// An event being reported by the runner
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    JobStarted {
        id: String,
        label: String,
    },
    JobPassedOver {
        label: String,
        reason: PassedOver,
    },
    JobFinished {
        id: String,
        label: String,
        conclusion: Conclusion,
    },
    JobOutputs {
        id: String,
        outputs: std::collections::BTreeMap<String, String>,
    },
    StepStarted {
        index: usize,
        name: String,
        depth: usize,
    },
    #[serde(rename = "output")]
    StepOutput {
        stream: Stream,
        line: String,
    },
    StepFinished {
        index: usize,
        name: String,
        depth: usize,
        conclusion: Conclusion,
        code: Option<i32>,
    },
    Progress {
        text: String,
    },
    Message {
        level: Level,
        text: String,
    },
}

pub trait Reporter {
    fn report(&mut self, event: Event);
}

#[derive(Debug, Default)]
pub struct Terminal;

impl Reporter for Terminal {
    fn report(&mut self, event: Event) {
        match event {
            Event::JobStarted { label, .. } => println!("--- {label}"),
            Event::JobPassedOver { label, reason } => {
                println!("--- {label} ({})", reason.name());
            }
            Event::JobFinished { .. } | Event::JobOutputs { .. } => {}
            Event::StepStarted { name, depth, .. } => {
                println!("{}==> {name}", indent(depth));
            }
            Event::StepOutput { stream, line } => match stream {
                Stream::Out => println!("{line}"),
                Stream::Err => eprintln!("{line}"),
            },
            Event::StepFinished {
                name,
                depth,
                conclusion,
                code,
                ..
            } => {
                if conclusion != Conclusion::Failure {
                    return;
                }
                let indent = indent(depth);
                match code {
                    Some(code) => {
                        eprintln!("{indent}    step {name:?} failed with exit code {code}");
                    }
                    None => eprintln!("{indent}    step {name:?} was killed by a signal"),
                }
            }
            Event::Progress { text } => println!("    {text}"),
            Event::Message { level, text } => match level {
                // Only shown when the runner was asked for them, as GitHub does it.
                Level::Debug => {
                    if std::env::var("RUNNER_DEBUG").is_ok() {
                        println!("    debug: {text}");
                    }
                }
                Level::Notice => println!("    notice: {text}"),
                Level::Warning => println!("    warning: {text}"),
                Level::Error => println!("    error: {text}"),
            },
        }
    }
}

pub struct Json<W> {
    writer: W,
}

impl<W: std::io::Write> Json<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: std::io::Write> Reporter for Json<W> {
    fn report(&mut self, event: Event) {
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = writeln!(self.writer, "{line}");
        }
    }
}

/// Keeps every event, for a test to look through afterwards.
#[derive(Debug, Default)]
pub struct Collected {
    pub events: Vec<Event>,
}

impl Reporter for Collected {
    fn report(&mut self, event: Event) {
        self.events.push(event);
    }
}

impl Collected {
    pub fn output(&self) -> Vec<&str> {
        self.events
            .iter()
            .filter_map(|event| match event {
                Event::StepOutput { line, .. } => Some(line.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn messages_by_level(&self, level: Level) -> Vec<&str> {
        self.events
            .iter()
            .filter_map(|event| match event {
                Event::Message { level: at, text } if *at == level => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn steps(&self) -> Vec<&str> {
        self.events
            .iter()
            .filter_map(|event| match event {
                Event::StepStarted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collector_keeps_what_it_was_told_in_order() {
        let mut reporter = Collected::default();

        reporter.report(Event::StepStarted {
            index: 0,
            name: "first".to_owned(),
            depth: 0,
        });
        reporter.report(Event::StepOutput {
            stream: Stream::Out,
            line: "hello".to_owned(),
        });
        reporter.report(Event::StepStarted {
            index: 1,
            name: "second".to_owned(),
            depth: 0,
        });

        assert_eq!(reporter.steps(), ["first", "second"]);
        assert_eq!(reporter.output(), ["hello"]);
    }

    #[test]
    fn a_nested_step_is_written_further_in() {
        assert_eq!(indent(0), "");
        assert_eq!(indent(1), "  ");
        assert_eq!(indent(2), "    ");
    }

    #[test]
    fn an_event_survives_the_trip_down_a_pipe() {
        let events = [
            Event::StepStarted {
                index: 2,
                name: "Checkout".to_owned(),
                depth: 0,
            },
            Event::StepOutput {
                stream: Stream::Err,
                line: "not going well".to_owned(),
            },
            Event::StepFinished {
                index: 2,
                name: "Checkout".to_owned(),
                depth: 0,
                conclusion: Conclusion::Failure,
                code: Some(3),
            },
        ];

        for event in events {
            let written = serde_json::to_string(&event).expect("writes");
            let read: Event = serde_json::from_str(&written).expect("reads");
            assert_eq!(read, event, "in {written}");
        }
    }
}
