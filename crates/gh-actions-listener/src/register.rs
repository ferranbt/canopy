//! Registering a runner: turning a registration token into credentials that last.
//!
//! Three calls:
//!
//! 1. `POST api.github.com/actions/runner-registration` with the registration token, for
//!    the Actions service url and a bearer token to it.
//! 2. `GET {service}/_apis/distributedtask/pools?poolName=Default`, for the pool id.
//! 3. `POST {service}/_apis/distributedtask/pools/{pool}/agents` with the public half of a
//!    key generated here, for the agent id and the OAuth endpoint it authenticates at.
//!
//! The private half stays in the credentials and signs the assertion that buys an access
//! token, so the registration token is needed once and can be short-lived.

use base64::Engine as _;
use gh_actions_context::Runner;
use rsa::RsaPrivateKey;
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts as _;
use tracing::info;

use crate::auth::Credentials;
use crate::client::Client;
use crate::client::types::{AgentRequest, Label, PublicKey, PublicKeyRequest, RUNNER_VERSION};
use crate::error::Error;

#[derive(Debug, Clone)]
pub struct Registration {
    /// The url of the repo that the runner joins, e.g. `https://github.com/owner/repo`.
    pub url: String,
    /// Short-lived and single-use.
    pub token: String,
    pub name: String,
    /// Custom labels for the runner on top of the ones every runner has.
    pub labels: Vec<String>,
}

pub fn register(registration: &Registration) -> Result<Credentials, Error> {
    let mut client = Client::new()?;
    client.registration(&registration.token);

    let tenant = client.tenant(&registration.url)?;
    client.bearer(&tenant.token);
    info!(service = %tenant.url, "registering");

    let pool = client.default_pool(&tenant.url)?;
    let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048)
        .map_err(|err| Error::Crypto(format!("cannot generate a runner key: {err}")))?;
    let agent = client.create_agent(&tenant.url, pool, &agent_request(registration, &key))?;
    info!(agent = agent.id, pool, "registered");

    let private_key = key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .map_err(|err| Error::Crypto(format!("cannot write the runner key out: {err}")))?
        .to_string();

    Ok(Credentials {
        service_url: tenant.url,
        pool_id: pool,
        agent_id: agent.id,
        agent_name: registration.name.clone(),
        client_id: agent.authorization.client_id,
        token_url: agent.authorization.authorization_url,
        private_key,
    })
}

fn agent_request(registration: &Registration, key: &RsaPrivateKey) -> AgentRequest {
    let number = |value: &rsa::BigUint| {
        base64::engine::general_purpose::STANDARD.encode(value.to_bytes_be())
    };

    AgentRequest {
        name: registration.name.clone(),
        version: RUNNER_VERSION,
        os_description: Runner::host_os(),
        max_parallelism: 1,
        ephemeral: false,
        disable_update: true,
        provisioning_state: "Provisioned",
        labels: labels(registration),
        authorization: PublicKeyRequest {
            public_key: PublicKey {
                exponent: number(key.e()),
                modulus: number(key.n()),
            },
        },
    }
}

fn labels(registration: &Registration) -> Vec<Label> {
    let system = ["self-hosted", Runner::host_os(), Runner::host_arch()];

    system
        .iter()
        .map(|name| Label {
            name: (*name).to_owned(),
            r#type: "system",
        })
        .chain(
            registration
                .labels
                .iter()
                .filter(|name| !system.contains(&name.as_str()))
                .map(|name| Label {
                    name: name.clone(),
                    r#type: "user",
                }),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runner_offers_the_system_labels_and_its_own_without_repeating_any() {
        let offered = labels(&Registration {
            url: "https://github.com/octocat/hello".to_owned(),
            token: "AAAA".to_owned(),
            name: "canopy".to_owned(),
            labels: vec!["gpu".to_owned(), "self-hosted".to_owned()],
        });
        let names: Vec<&str> = offered.iter().map(|label| label.name.as_str()).collect();

        assert_eq!(
            names,
            ["self-hosted", Runner::host_os(), Runner::host_arch(), "gpu"]
        );
        assert_eq!(offered[0].r#type, "system");
        assert_eq!(offered[3].r#type, "user");
    }
}
