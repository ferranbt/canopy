//! The `runner` context: the machine the job is executing on.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Runner {
    pub arch: String,
    #[serde(serialize_with = "debug_flag", deserialize_with = "read_debug_flag")]
    pub debug: bool,
    pub environment: String,
    pub name: String,
    pub os: String,
    pub temp: String,
    pub tool_cache: String,
}

impl Runner {
    pub fn host(temp: &Path) -> Self {
        Self {
            arch: Self::host_arch().to_owned(),
            debug: false,
            environment: "self-hosted".to_owned(),
            name: "canopy".to_owned(),
            os: Self::host_os().to_owned(),
            temp: temp.display().to_string(),
            tool_cache: temp.join("tools").display().to_string(),
        }
    }

    pub fn host_os() -> &'static str {
        match std::env::consts::OS {
            "linux" => "Linux",
            "macos" => "macOS",
            "windows" => "Windows",
            other => other,
        }
    }

    pub fn host_arch() -> &'static str {
        match std::env::consts::ARCH {
            "x86_64" => "X64",
            "aarch64" => "ARM64",
            "x86" => "X86",
            other => other,
        }
    }
}

fn debug_flag<S: Serializer>(debug: &bool, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(if *debug { "1" } else { "" })
}

fn read_debug_flag<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(String::deserialize(deserializer)? == "1")
}
