//! Resolves what a step's `uses:` points at and loads its `action.yml`.

use std::path::{Path, PathBuf};
use std::process::Command;

use gh_actions_spec::{Action, Uses};

use crate::error::{At, Error};

#[derive(Debug, Clone)]
pub struct ResolvedAction {
    pub action: Action,
    pub path: PathBuf,
}

pub fn resolve(
    reference: &Uses,
    workspace: &Path,
    cache: &Path,
    nested: bool,
) -> Result<ResolvedAction, Error> {
    let path = match reference {
        // Without the `./` a local reference is written with, which is how the path reads
        // everywhere it is handed on, `github.action_path` included. Inside a composite
        // action it is kept, which is where GitHub leaves it.
        Uses::Local(path) => {
            let written = match nested {
                true => path.as_path(),
                false => path.strip_prefix("./").unwrap_or(path),
            };
            let path = workspace.join(written);
            if !["action.yml", "action.yaml", "Dockerfile"]
                .iter()
                .any(|name| path.join(name).is_file())
            {
                return Err(Error::Refused(format!(
                    "Can't find 'action.yml', 'action.yaml' or 'Dockerfile' under '{}'. \
                     Did you forget to run actions/checkout before running your local action?",
                    path.display()
                )));
            }

            path
        }
        Uses::Remote {
            owner,
            repo,
            subdir,
            reference,
        } => {
            let checkout = fetch(owner, repo, reference, cache)?;
            match subdir {
                Some(subdir) => checkout.join(subdir),
                None => checkout,
            }
        }
        Uses::Image(image) => {
            return Err(Error::Plan(format!(
                "`docker://{image}` has no action.yml to load"
            )));
        }
    };

    Ok(ResolvedAction {
        action: load(&path)?,
        path,
    })
}

pub fn load(directory: &Path) -> Result<Action, Error> {
    for name in ["action.yml", "action.yaml"] {
        let candidate = directory.join(name);
        if candidate.is_file() {
            let source = std::fs::read_to_string(&candidate).at(&candidate)?;
            return Ok(yaml_with_spans::from_str(&source)?);
        }
    }

    Err(Error::Plan(format!(
        "no action.yml in {}",
        directory.display()
    )))
}

fn fetch(owner: &str, repo: &str, reference: &str, cache: &Path) -> Result<PathBuf, Error> {
    let destination = cache.join(owner).join(repo).join(reference);

    if destination.join(".git").is_dir() {
        return Ok(destination);
    }
    let parent = destination.parent().unwrap_or(cache);
    std::fs::create_dir_all(parent).at(parent)?;

    let url = format!("https://github.com/{owner}/{repo}");
    tracing::info!(%owner, %repo, reference, "fetching an action");

    // A ref that names a branch or tag clones directly; a commit SHA needs a checkout afterwards.
    let shallow = git(&[
        "clone",
        "--depth",
        "1",
        "--branch",
        reference,
        "--quiet",
        &url,
        &destination.display().to_string(),
    ]);

    if shallow.is_err() {
        let _ = std::fs::remove_dir_all(&destination);
        git(&["clone", "--quiet", &url, &destination.display().to_string()])?;
        git(&[
            "-C",
            &destination.display().to_string(),
            "checkout",
            "--quiet",
            reference,
        ])?;
    }

    Ok(destination)
}

fn git(args: &[&str]) -> Result<(), Error> {
    let output = Command::new("git").args(args).output().at("git")?;

    if output.status.success() {
        return Ok(());
    }
    Err(Error::Plan(format!(
        "git {} failed: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

pub fn cache_directory() -> PathBuf {
    std::env::var("CANOPY_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
            PathBuf::from(home).join(".cache/canopy/actions")
        })
}
