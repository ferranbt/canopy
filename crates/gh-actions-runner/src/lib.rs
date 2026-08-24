//! Runs a planned job: resolves its actions, executes its steps, collects its outputs.

pub mod actions;
pub mod commands;
pub mod error;
pub mod executor;
pub mod job;

pub mod steps;
pub mod validate;

pub use error::{At, Error};

pub use executor::{
    Exec, ExecRequest, ExecResult, ExecStatus, HostMachine, Image, Machine, Started, run_until,
};
pub use gh_actions_report as report;
pub use job::{Options, Summary, run, run_steps};
pub use report::{Collected, Event, Json, Reporter, Terminal};
