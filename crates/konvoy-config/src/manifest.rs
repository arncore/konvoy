use serde::{Deserialize, Serialize};
use std::path::Path;

/// The `konvoy.toml` project manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub package: Package,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    #[serde(default = "default_entrypoint")]
    pub entrypoint: String,
}

fn default_entrypoint() -> String {
    "src/main.kt".to_owned()
}

impl Manifest {
    /// Read and parse a `konvoy.toml` from the given path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or contains invalid TOML.
    pub fn from_path(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path).map_err(|e| ManifestError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
        let manifest: Manifest = toml::from_str(&content).map_err(|e| ManifestError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(manifest)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid konvoy.toml at {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
}
