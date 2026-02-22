use serde::{Deserialize, Serialize};
use std::path::Path;

/// The `konvoy.lock` lockfile.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Lockfile {
    #[serde(default)]
    pub toolchain: Option<ToolchainLock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainLock {
    pub konanc_version: String,
}

impl Lockfile {
    /// Read and parse a `konvoy.lock` from the given path.
    /// Returns a default lockfile if the file does not exist.
    pub fn from_path(path: &Path) -> Result<Self, LockfileError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| LockfileError::Read { path: path.display().to_string(), source: e })?;
        let lockfile: Lockfile = toml::from_str(&content)
            .map_err(|e| LockfileError::Parse { path: path.display().to_string(), source: e })?;
        Ok(lockfile)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LockfileError {
    #[error("cannot read {path}: {source}")]
    Read { path: String, source: std::io::Error },
    #[error("invalid konvoy.lock at {path}: {source}")]
    Parse { path: String, source: toml::de::Error },
}
