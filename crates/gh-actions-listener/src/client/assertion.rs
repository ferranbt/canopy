//! The assertion a runner signs to prove who it is, in place of a client secret.

use serde::Serialize;

use crate::auth::Credentials;
use crate::error::Error;

/// The service refuses one that lives longer than five minutes, and it counts from `nbf`,
/// so backdating for clock skew comes out of the same five minutes rather than adding to it.
const ASSERTION_LIFETIME: u64 = 300;

/// How far an assertion is backdated, so a clock that is a little fast is still accepted.
const BACKDATE: u64 = 60;

#[derive(Debug, Serialize)]
struct Assertion {
    iss: String,
    sub: String,
    aud: String,
    /// Unique, so an assertion cannot be replayed.
    jti: String,
    nbf: u64,
    exp: u64,
}

fn claims(credentials: &Credentials, now: u64) -> Assertion {
    Assertion {
        iss: credentials.client_id.clone(),
        sub: credentials.client_id.clone(),
        aud: credentials.token_url.clone(),
        jti: uuid::Uuid::new_v4().to_string(),
        nbf: now.saturating_sub(BACKDATE),
        exp: now.saturating_sub(BACKDATE) + ASSERTION_LIFETIME,
    }
}

pub fn sign(credentials: &Credentials) -> Result<String, Error> {
    let assertion = claims(credentials, seconds_since_epoch());

    let key = jsonwebtoken::EncodingKey::from_rsa_pem(credentials.private_key.as_bytes())
        .map_err(|err| Error::Crypto(format!("cannot use the runner key to sign: {err}")))?;

    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &assertion,
        &key,
    )
    .map_err(|err| Error::Crypto(format!("cannot sign the assertion: {err}")))
}

fn seconds_since_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_assertion_lives_no_longer_than_the_service_allows() {
        let credentials = Credentials {
            service_url: "https://example.invalid".to_owned(),
            pool_id: 1,
            agent_id: 2,
            agent_name: "canopy".to_owned(),
            client_id: "client".to_owned(),
            token_url: "https://example.invalid/token".to_owned(),
            private_key: "not a key".to_owned(),
        };
        let assertion = claims(&credentials, 1_000_000);

        // Counted from `nbf`, which is backdated: the service rejects anything longer.
        assert!(
            assertion.exp - assertion.nbf <= 300,
            "{} seconds is too long",
            assertion.exp - assertion.nbf
        );
    }
}
