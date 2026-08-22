//! Every call this makes to GitHub, and the types they are made of.
//!
//! All of them go through one request method, so what was sent and what came back can be
//! seen by turning on `debug` logging rather than by adding prints.

pub mod api;
pub mod assertion;
pub mod crypto;
pub mod feed;
pub mod types;

use std::time::Duration;

use reqwest::Method;
use reqwest::blocking::RequestBuilder;
use serde::Serialize;
use tracing::{debug, trace};

use crate::error::Error;

pub use feed::Feed;
pub use types::*;

const SHOWN: usize = 600;

pub struct Client {
    http: reqwest::blocking::Client,
    auth: Option<String>,
}

impl Client {
    pub fn new() -> Result<Self, Error> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;

        Ok(Self { http, auth: None })
    }

    pub fn bearer(&mut self, token: &str) {
        self.auth = Some(format!("Bearer {token}"));
    }

    pub fn registration(&mut self, token: &str) {
        self.auth = Some(format!("RemoteAuth {token}"));
    }

    fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        url: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T, Error> {
        decode(&self.send(self.request(method, url, body))?)
    }

    fn maybe<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<Option<T>, Error> {
        let answered = self.send(self.request(Method::GET, url, NOTHING))?;
        if answered.trim().is_empty() {
            return Ok(None);
        }

        decode(&answered).map(Some)
    }

    fn nothing(&self, method: Method, url: &str) -> Result<(), Error> {
        self.send(self.request(method, url, NOTHING))?;
        Ok(())
    }

    fn request(&self, method: Method, url: &str, body: Option<&impl Serialize>) -> RequestBuilder {
        self.request_as(method, url, body, self.auth.clone())
    }

    fn request_as(
        &self,
        method: Method,
        url: &str,
        body: Option<&impl Serialize>,
        auth: Option<String>,
    ) -> RequestBuilder {
        let mut request = self
            .http
            .request(method.clone(), url)
            .header("Accept", "application/json;api-version=5.1-preview")
            .header("User-Agent", "canopy");

        if let Some(auth) = &auth {
            request = request.header("Authorization", auth);
        }

        debug!(%method, %url, "→");
        match body {
            Some(body) => {
                trace!(body = %serde_json::to_string(body).unwrap_or_default(), "→ sent");
                request.json(body)
            }
            None => request,
        }
    }

    fn send(&self, request: RequestBuilder) -> Result<String, Error> {
        let response = request.send()?;
        let status = response.status();
        let body = response.text()?;
        debug!(status = status.as_u16(), bytes = body.len(), "←");
        trace!(%body, "← answered");

        if !status.is_success() {
            return Err(Error::Status {
                status: status.as_u16(),
                body,
            });
        }
        Ok(body)
    }
}

const NOTHING: Option<&()> = None;

/// The protocol is undocumented, so a response that does not fit is the thing worth seeing.
fn decode<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, Error> {
    serde_json::from_str(body).map_err(|err| Error::Protocol(format!("{err}, in: {}", shown(body))))
}

fn shown(body: &str) -> String {
    body.chars().take(SHOWN).collect()
}
