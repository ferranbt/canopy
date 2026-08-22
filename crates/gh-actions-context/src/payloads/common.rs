use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

pub fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

pub type Extra = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct User {
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub id: i64,
    #[serde(default, rename = "type")]
    pub r#type: String,
    #[serde(default)]
    pub site_admin: bool,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Repository {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub owner: User,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub author: Author,
    /// Paths the commit added.
    #[serde(default, deserialize_with = "null_as_default")]
    pub added: Vec<String>,
    /// Paths it removed.
    #[serde(default, deserialize_with = "null_as_default")]
    pub removed: Vec<String>,
    /// Paths it changed.
    #[serde(default, deserialize_with = "null_as_default")]
    pub modified: Vec<String>,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Author {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Label {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(flatten)]
    pub other: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "ref")]
    pub r#ref: String,
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub repo: Option<Repository>,
    #[serde(flatten)]
    pub other: Extra,
}
