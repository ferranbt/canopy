//! The `uses:` value of a step, decoded into what it points at.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The value is decoded when the workflow is read, because `uses:` never contains an
/// expression: GitHub resolves it before any context exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Uses {
    /// `./path`, inside the repository.
    Local(PathBuf),
    /// `owner/repo[/subdir]@ref`.
    Remote {
        owner: String,
        repo: String,
        /// Set when the action is not at the repository root.
        subdir: Option<String>,
        reference: String,
    },
    /// `docker://image:tag`.
    Image(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsesError {
    MissingReference(String),
    NotAReference(String),
}

impl fmt::Display for UsesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingReference(uses) => write!(f, "`uses: {uses}` is missing its `@ref`"),
            Self::NotAReference(uses) => {
                write!(
                    f,
                    "`uses: {uses}` is not `owner/repo@ref`, `./path` or `docker://image`"
                )
            }
        }
    }
}

impl std::error::Error for UsesError {}

impl FromStr for Uses {
    type Err = UsesError;

    fn from_str(uses: &str) -> Result<Self, Self::Err> {
        if let Some(image) = uses.strip_prefix("docker://") {
            return Ok(Self::Image(image.to_owned()));
        }
        if uses.starts_with("./") || uses.starts_with("../") {
            return Ok(Self::Local(PathBuf::from(uses)));
        }

        let (path, reference) = uses
            .split_once('@')
            .ok_or_else(|| UsesError::MissingReference(uses.to_owned()))?;
        let mut parts = path.splitn(3, '/');
        let (Some(owner), Some(repo)) = (parts.next(), parts.next()) else {
            return Err(UsesError::NotAReference(uses.to_owned()));
        };
        if owner.is_empty() || repo.is_empty() {
            return Err(UsesError::NotAReference(uses.to_owned()));
        }

        Ok(Self::Remote {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            subdir: parts.next().map(str::to_owned),
            reference: reference.to_owned(),
        })
    }
}

impl fmt::Display for Uses {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(path) => write!(f, "{}", path.display()),
            Self::Image(image) => write!(f, "docker://{image}"),
            Self::Remote {
                owner,
                repo,
                subdir,
                reference,
            } => {
                write!(f, "{owner}/{repo}")?;
                if let Some(subdir) = subdir {
                    write!(f, "/{subdir}")?;
                }
                write!(f, "@{reference}")
            }
        }
    }
}

impl<'de> Deserialize<'de> for Uses {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

impl Serialize for Uses {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_every_form() {
        assert_eq!(
            "./.github/actions/greet".parse::<Uses>().unwrap(),
            Uses::Local(PathBuf::from("./.github/actions/greet"))
        );
        assert_eq!(
            "docker://alpine:3.19".parse::<Uses>().unwrap(),
            Uses::Image("alpine:3.19".to_owned())
        );
        assert_eq!(
            "actions/checkout@v4".parse::<Uses>().unwrap(),
            Uses::Remote {
                owner: "actions".to_owned(),
                repo: "checkout".to_owned(),
                subdir: None,
                reference: "v4".to_owned(),
            }
        );
        assert_eq!(
            "owner/repo/sub/dir@main".parse::<Uses>().unwrap(),
            Uses::Remote {
                owner: "owner".to_owned(),
                repo: "repo".to_owned(),
                subdir: Some("sub/dir".to_owned()),
                reference: "main".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_what_it_cannot_decode() {
        assert!("actions/checkout".parse::<Uses>().is_err());
        assert!("nonsense".parse::<Uses>().is_err());
    }

    #[test]
    fn round_trips_through_its_text_form() {
        for original in [
            "./actions/greet",
            "docker://alpine:3.19",
            "actions/checkout@v4",
            "owner/repo/sub/dir@main",
        ] {
            assert_eq!(original.parse::<Uses>().unwrap().to_string(), original);
        }
    }
}
