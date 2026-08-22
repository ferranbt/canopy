use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub service_url: String,
    pub pool_id: i64,
    pub agent_id: i64,
    #[serde(default)]
    pub agent_name: String,
    pub client_id: String,
    pub token_url: String,
    pub private_key: String,
}

impl Credentials {
    pub fn read(path: &Path) -> Result<Self, Error> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn write(&self, path: &Path) -> Result<(), Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
