use std::fmt;

#[derive(Debug)]
pub enum Error {
    Http(String),
    Status { status: u16, body: String },
    Crypto(String),
    Protocol(String),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(message) => write!(f, "cannot reach the service: {message}"),
            Self::Status { status, body } => write!(f, "service answered {status}: {body}"),
            Self::Crypto(message) => write!(f, "{message}"),
            Self::Protocol(message) => write!(f, "unexpected response: {message}"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Protocol(err.to_string())
    }
}
