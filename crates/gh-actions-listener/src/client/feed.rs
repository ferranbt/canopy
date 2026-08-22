//! The live log feed: a websocket that lines are pushed down as they are printed.
//!
//! Every batch says which step it belongs to and where in that step's output it starts, so
//! the service can put them in order however they arrive.

use std::net::TcpStream;

use serde::Serialize;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::error::Error;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedLines<'a> {
    count: usize,
    value: &'a [String],
    step_id: &'a str,
    /// Where these sit in the step's output, counting from one.
    start_line: u64,
}

pub struct Feed {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    /// The step whose output is being sent, and how much of it has gone already.
    step: String,
    sent: u64,
}

impl Feed {
    pub fn connect(url: &str, token: &str) -> Result<Self, Error> {
        let mut request = url
            .into_client_request()
            .map_err(|err| Error::Http(format!("the feed url is not one: {err}")))?;

        let bearer = format!("Bearer {token}")
            .parse()
            .map_err(|err| Error::Http(format!("the job token is not a header: {err}")))?;
        request.headers_mut().insert("Authorization", bearer);
        request.headers_mut().insert(
            "User-Agent",
            "canopy".parse().expect("a name is a header value"),
        );

        let (socket, _) = tungstenite::connect(request)
            .map_err(|err| Error::Http(format!("cannot open the log feed: {err}")))?;

        Ok(Self {
            socket,
            step: String::new(),
            sent: 0,
        })
    }

    pub fn step(&mut self, id: &str) {
        self.step = id.to_owned();
        self.sent = 0;
    }

    pub fn lines(&mut self, lines: &[String]) -> Result<(), Error> {
        if lines.is_empty() || self.step.is_empty() {
            return Ok(());
        }

        let batch = FeedLines {
            count: lines.len(),
            value: lines,
            step_id: &self.step,
            start_line: self.sent + 1,
        };
        let message = serde_json::to_string(&batch)?;

        self.socket
            .send(Message::Text(message))
            .map_err(|err| Error::Http(format!("cannot send to the log feed: {err}")))?;
        self.sent += lines.len() as u64;

        Ok(())
    }

    pub fn close(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }
}
