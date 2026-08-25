//! The one implementation there is so far: artifacts as files in a directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::artifacts_server::{Artifact, Artifacts};

#[derive(Debug)]
pub struct LocalArtifacts {
    root: PathBuf,
    known: Mutex<BTreeMap<String, Artifact>>,
    next_id: Mutex<i64>,
}

impl LocalArtifacts {
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;

        Ok(Self {
            root,
            known: Mutex::new(BTreeMap::new()),
            next_id: Mutex::new(1),
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(sanitize(name))
    }
}

impl Artifacts for LocalArtifacts {
    fn create(&self, name: &str) -> Artifact {
        let mut next = self.next_id.lock().expect("id lock");
        *next += 1;

        let artifact = Artifact {
            name: name.to_owned(),
            size: 0,
            id: *next,
        };
        self.known
            .lock()
            .expect("artifact lock")
            .insert(name.to_owned(), artifact.clone());
        artifact
    }

    fn finalize(&self, name: &str, size: u64) -> Option<Artifact> {
        let mut known = self.known.lock().expect("artifact lock");
        let artifact = known.get_mut(name)?;
        artifact.size = size;
        Some(artifact.clone())
    }

    fn get(&self, name: &str) -> Option<Artifact> {
        self.known.lock().expect("artifact lock").get(name).cloned()
    }

    fn list(&self, name: Option<&str>) -> Vec<Artifact> {
        self.known
            .lock()
            .expect("artifact lock")
            .values()
            .filter(|artifact| name.is_none_or(|wanted| wanted == artifact.name))
            .cloned()
            .collect()
    }

    fn store(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::write(self.path(name), bytes)
    }

    fn delete(&self, name: &str) -> Option<Artifact> {
        let artifact = self.known.lock().expect("artifact lock").remove(name)?;
        let _ = std::fs::remove_file(self.path(name));

        Some(artifact)
    }

    fn load(&self, name: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.path(name))
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_as_an_artifact_store() {
        let root = std::env::temp_dir().join("local-artifacts-conformance");
        let _ = std::fs::remove_dir_all(&root);

        crate::artifacts_server::conformance(Box::new(
            LocalArtifacts::open(root).expect("store opens"),
        ));
    }
}
