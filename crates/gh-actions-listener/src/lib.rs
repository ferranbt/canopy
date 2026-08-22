//! The protocol a self-hosted runner speaks to GitHub.

pub mod auth;
pub mod client;
pub mod context;
pub mod error;
pub mod listener;
pub mod progress;
pub mod register;

pub use auth::Credentials;
pub use client::types::{
    Envelope, GithubContext, JobContext, JobMessage, NeedsResult, Outcome, Variable,
};
pub use client::{Client, RUNNER_VERSION, Session, Token};
pub use error::Error;
pub use listener::{Listener, Worker};
pub use progress::Progress;
pub use register::{Registration, register};
