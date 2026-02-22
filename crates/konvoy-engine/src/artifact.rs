//! Content-addressed artifact store for build outputs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cache::CacheKey;
use crate::error::EngineError;

/// Metadata stored alongside a cached artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildMetadata {
    /// Kotlin/Native target (e.g. "linux_x64").
    pub target: String,
    /// Build profile ("debug" or "release").
    pub profile: String,
    /// konanc version used for the build.
    pub konanc_version: String,
    /// ISO 8601 timestamp of when the build was produced.
    pub built_at: String,
}

/// Content-addressed store for compiled artifacts under `.konvoy/cache/`.
#[derive(Debug)]
pub struct ArtifactStore {
    cache_root: PathBuf,
}

impl ArtifactStore {
    /// Create a new artifact store rooted at `<project_root>/.konvoy/cache/`.
    pub fn new(project_root: &Path) -> Self {
        Self {
            cache_root: project_root.join(".konvoy").join("cache"),
        }
    }

    /// Return the cache directory path for a given key.
    pub fn cache_path(&self, key: &CacheKey) -> PathBuf {
        self.cache_root.join(key.as_hex())
    }

    /// Check whether a cache entry exists for the given key.
    pub fn has(&self, key: &CacheKey) -> bool {
        self.cache_path(key).is_dir()
    }

    /// Store an artifact and its metadata in the cache.
    ///
    /// The cache is immutable: if an entry already exists for this key,
    /// the store is a no-op and returns `Ok(())`.
    ///
    /// # Errors
    /// Returns an error if the cache directory cannot be created or the
    /// artifact cannot be copied.
    pub fn store(
        &self,
        key: &CacheKey,
        artifact: &Path,
        metadata: &BuildMetadata,
    ) -> Result<(), EngineError> {
        let entry_dir = self.cache_path(key);

        // Immutable cache: never overwrite.
        if entry_dir.is_dir() {
            return Ok(());
        }

        konvoy_util::fs::ensure_dir(&entry_dir)?;

        // Copy the artifact into the cache directory.
        let Some(file_name) = artifact.file_name() else {
            return Err(EngineError::Io {
                path: artifact.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "artifact path has no file name",
                ),
            });
        };
        let cached_artifact = entry_dir.join(file_name);
        std::fs::copy(artifact, &cached_artifact).map_err(|source| EngineError::Io {
            path: cached_artifact.display().to_string(),
            source,
        })?;

        // Write metadata alongside the artifact.
        let metadata_path = entry_dir.join("metadata.toml");
        let metadata_toml =
            toml::to_string_pretty(metadata).map_err(|e| EngineError::Metadata {
                message: e.to_string(),
            })?;
        std::fs::write(&metadata_path, metadata_toml).map_err(|source| EngineError::Io {
            path: metadata_path.display().to_string(),
            source,
        })?;

        Ok(())
    }

    /// Materialize a cached artifact to the given destination path.
    ///
    /// Prefers hard linking for disk efficiency, falls back to copy if linking
    /// fails (e.g. cross-filesystem).
    ///
    /// # Errors
    /// Returns an error if the cache entry does not exist or the artifact
    /// cannot be materialized.
    pub fn materialize(
        &self,
        key: &CacheKey,
        artifact_name: &str,
        dest: &Path,
    ) -> Result<(), EngineError> {
        let entry_dir = self.cache_path(key);
        let cached_artifact = entry_dir.join(artifact_name);

        if !cached_artifact.exists() {
            return Err(EngineError::Io {
                path: cached_artifact.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "cached artifact not found",
                ),
            });
        }

        konvoy_util::fs::materialize(&cached_artifact, dest)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;

    fn test_metadata() -> BuildMetadata {
        BuildMetadata {
            target: "linux_x64".to_owned(),
            profile: "debug".to_owned(),
            konanc_version: "2.1.0".to_owned(),
            built_at: "2026-02-21T00:00:00Z".to_owned(),
        }
    }

    fn test_key() -> CacheKey {
        // Use CacheInputs to compute a real key.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.kt"), "fun main() {}").unwrap();

        let inputs = crate::cache::CacheInputs {
            manifest_content: "[package]\nname = \"test\"".to_owned(),
            lockfile_content: "".to_owned(),
            konanc_version: "2.1.0".to_owned(),
            konanc_fingerprint: "abc123".to_owned(),
            target: "linux_x64".to_owned(),
            profile: "debug".to_owned(),
            source_dir: tmp.path().to_path_buf(),
            source_glob: "**/*.kt".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
        };
        CacheKey::compute(&inputs).unwrap()
    }

    #[test]
    fn store_and_has() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(tmp.path());
        let key = test_key();

        // Create a fake artifact.
        let artifact = tmp.path().join("my-app");
        fs::write(&artifact, b"binary content").unwrap();

        assert!(!store.has(&key));
        store.store(&key, &artifact, &test_metadata()).unwrap();
        assert!(store.has(&key));
    }

    #[test]
    fn store_writes_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(tmp.path());
        let key = test_key();

        let artifact = tmp.path().join("my-app");
        fs::write(&artifact, b"binary").unwrap();

        store.store(&key, &artifact, &test_metadata()).unwrap();

        let metadata_path = store.cache_path(&key).join("metadata.toml");
        assert!(metadata_path.exists());
        let content = fs::read_to_string(metadata_path).unwrap();
        assert!(content.contains("linux_x64"));
        assert!(content.contains("2.1.0"));
    }

    #[test]
    fn store_is_immutable() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(tmp.path());
        let key = test_key();

        let artifact = tmp.path().join("my-app");
        fs::write(&artifact, b"original").unwrap();
        store.store(&key, &artifact, &test_metadata()).unwrap();

        // Overwrite artifact content and store again — cache should not change.
        fs::write(&artifact, b"modified").unwrap();
        store.store(&key, &artifact, &test_metadata()).unwrap();

        let cached = store.cache_path(&key).join("my-app");
        let content = fs::read(&cached).unwrap();
        assert_eq!(content, b"original");
    }

    #[test]
    fn materialize_creates_output() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(tmp.path());
        let key = test_key();

        let artifact = tmp.path().join("my-app");
        fs::write(&artifact, b"binary content").unwrap();
        store.store(&key, &artifact, &test_metadata()).unwrap();

        let dest = tmp.path().join("output").join("my-app");
        store.materialize(&key, "my-app", &dest).unwrap();

        assert!(dest.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"binary content");
    }

    #[test]
    fn materialize_missing_entry_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(tmp.path());
        let key = test_key();

        let dest = tmp.path().join("output").join("my-app");
        let result = store.materialize(&key, "my-app", &dest);
        assert!(result.is_err());
    }

    #[test]
    fn cache_path_includes_key() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(tmp.path());
        let key = test_key();

        let path = store.cache_path(&key);
        assert!(path.display().to_string().contains(key.as_hex()));
    }
}
