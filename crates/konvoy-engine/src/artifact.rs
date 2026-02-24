//! Content-addressed artifact store for build outputs.

use std::path::{Path, PathBuf};
use std::process::Command;

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
    /// Epoch seconds timestamp of when the build was produced (e.g. "1708646400s-since-epoch").
    pub built_at: String,
}

/// Content-addressed store for compiled artifacts under `.konvoy/cache/`.
#[derive(Debug)]
pub struct ArtifactStore {
    cache_root: PathBuf,
}

impl ArtifactStore {
    /// Create a new artifact store.
    ///
    /// If the project is inside a git worktree, the cache is shared with the
    /// main worktree at `<main-root>/.konvoy/cache/`. Otherwise, it lives at
    /// `<project_root>/.konvoy/cache/`.
    pub fn new(project_root: &Path) -> Self {
        Self {
            cache_root: resolve_cache_root(project_root),
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

/// Check whether any component of `path` is a symlink.
///
/// Walks from the root toward the leaf, checking each prefix with
/// `symlink_metadata`. Returns `true` as soon as a symlink is found.
fn path_contains_symlink(path: &Path) -> bool {
    let mut accumulated = PathBuf::new();
    for component in path.components() {
        accumulated.push(component);
        if let Ok(meta) = std::fs::symlink_metadata(&accumulated) {
            if meta.file_type().is_symlink() {
                return true;
            }
        }
    }
    false
}

/// Resolve the cache root directory, sharing cache across git worktrees.
///
/// In a git worktree, `git rev-parse --git-common-dir` returns the `.git`
/// directory of the main worktree. We use its parent as the shared cache root.
/// For normal repos or non-git projects, falls back to the project root.
///
/// If the resolved shared cache path contains symlinks, a warning is emitted
/// and the local fallback path is used instead (to avoid symlink-based
/// redirection of cache reads/writes).
fn resolve_cache_root(project_root: &Path) -> PathBuf {
    let fallback = project_root.join(".konvoy").join("cache");

    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(project_root)
        .output();

    let Ok(output) = output else {
        return fallback;
    };

    if !output.status.success() {
        return fallback;
    }

    let common_dir = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    // Resolve to absolute path.
    let common_path = if Path::new(&common_dir).is_absolute() {
        PathBuf::from(&common_dir)
    } else {
        project_root.join(&common_dir)
    };

    // Canonicalize and get the parent (main worktree root).
    let Some(main_root) = common_path
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    else {
        return fallback;
    };

    // If we're already in the main worktree, no need to share.
    if let (Ok(canonical_main), Ok(canonical_project)) =
        (main_root.canonicalize(), project_root.canonicalize())
    {
        if canonical_main == canonical_project {
            return fallback;
        }
    }

    let shared_cache = main_root.join(".konvoy").join("cache");

    // Guard against symlink-based redirection of the shared cache path.
    // A symlink in the path could redirect cache I/O to an attacker-controlled
    // directory, so we fall back to the local project cache instead.
    if path_contains_symlink(&shared_cache) {
        eprintln!(
            "warning: symlink detected in shared cache path '{}'; falling back to local cache",
            shared_cache.display()
        );
        return fallback;
    }

    shared_cache
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
            dependency_hashes: Vec::new(),
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

    #[test]
    fn path_without_symlinks_returns_false() {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize to resolve any OS-level symlinks (e.g. macOS /tmp -> /private/tmp).
        let canonical = tmp.path().canonicalize().unwrap();
        let nested = canonical.join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();
        assert!(!path_contains_symlink(&nested));
    }

    #[cfg(unix)]
    #[test]
    fn path_with_symlink_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().join("real");
        fs::create_dir_all(&real_dir).unwrap();

        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real_dir, &link).unwrap();

        assert!(path_contains_symlink(&link));

        // Also check a path that goes through the symlink.
        let nested = link.join("child");
        fs::create_dir_all(&nested).unwrap();
        assert!(path_contains_symlink(&nested));
    }

    #[test]
    fn concurrent_store_same_key() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ArtifactStore::new(tmp.path()));
        let key = Arc::new(test_key());

        // Create a fake artifact for each thread to store.
        let artifact = tmp.path().join("my-app");
        fs::write(&artifact, b"binary content").unwrap();
        let artifact = Arc::new(artifact);

        let num_threads = 8;
        let barrier = Arc::new(Barrier::new(num_threads));
        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let store = Arc::clone(&store);
                let key = Arc::clone(&key);
                let artifact = Arc::clone(&artifact);
                let meta = test_metadata();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    // Synchronize all threads to maximize contention.
                    barrier.wait();
                    store.store(&key, &artifact, &meta)
                })
            })
            .collect();

        for handle in handles {
            let result = handle.join().unwrap();
            // Every thread should succeed (first wins, rest see it exists).
            assert!(result.is_ok(), "store failed: {result:?}");
        }

        // The artifact should exist exactly once in the cache.
        assert!(store.has(&key));
        let cached = store.cache_path(&key).join("my-app");
        assert_eq!(fs::read(&cached).unwrap(), b"binary content");
    }

    #[test]
    fn concurrent_materialize_same_key() {
        use std::sync::Arc;
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ArtifactStore::new(tmp.path()));
        let key = Arc::new(test_key());

        // Store an artifact first.
        let artifact = tmp.path().join("my-app");
        fs::write(&artifact, b"binary content").unwrap();
        store.store(&key, &artifact, &test_metadata()).unwrap();

        let num_threads = 8;
        let output_dir = Arc::new(tmp.path().join("outputs"));

        let handles: Vec<_> = (0..num_threads)
            .map(|i| {
                let store = Arc::clone(&store);
                let key = Arc::clone(&key);
                let output_dir = Arc::clone(&output_dir);
                thread::spawn(move || {
                    let dest = output_dir.join(format!("thread-{i}")).join("my-app");
                    store.materialize(&key, "my-app", &dest)?;
                    let content = fs::read(&dest).map_err(|source| EngineError::Io {
                        path: dest.display().to_string(),
                        source,
                    })?;
                    assert_eq!(content, b"binary content");
                    Ok::<(), EngineError>(())
                })
            })
            .collect();

        for handle in handles {
            let result = handle.join().unwrap();
            assert!(result.is_ok(), "materialize failed: {result:?}");
        }
    }

    #[test]
    fn concurrent_materialize_same_destination() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ArtifactStore::new(tmp.path()));
        let key = Arc::new(test_key());

        // Store an artifact in the cache.
        let artifact = tmp.path().join("my-app");
        fs::write(&artifact, b"binary content").unwrap();
        store.store(&key, &artifact, &test_metadata()).unwrap();

        // Materialize uses hard links, so concurrent materialize+remove to the
        // same destination can cause `fs::copy` to truncate the shared inode
        // (corrupting the cache). To test concurrent writes to a single
        // destination safely, each thread copies from a per-thread snapshot of
        // the cached artifact.
        let num_threads = 8;
        let barrier = Arc::new(Barrier::new(num_threads));
        let dest = Arc::new(tmp.path().join("output").join("my-app"));

        // Ensure parent directory exists.
        let output_dir = tmp.path().join("output");
        fs::create_dir_all(&output_dir).unwrap();

        // Create per-thread source copies (independent inodes).
        let sources: Vec<_> = (0..num_threads)
            .map(|i| {
                let src = tmp.path().join(format!("source-{i}"));
                fs::copy(store.cache_path(&key).join("my-app"), &src).unwrap();
                Arc::new(src)
            })
            .collect();

        let handles: Vec<_> = sources
            .into_iter()
            .map(|src| {
                let dest = Arc::clone(&dest);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    // Synchronize all threads to maximize contention.
                    barrier.wait();
                    // Simulate materialize: remove + copy to the same dest.
                    let _ = std::fs::remove_file(&*dest);
                    std::fs::copy(&*src, &*dest)
                })
            })
            .collect();

        for handle in handles {
            // All threads should complete without panicking.
            let _result = handle.join().unwrap();
        }

        // The final file should exist with correct content regardless of
        // which thread won the race.
        assert!(
            dest.exists(),
            "destination should exist after concurrent writes"
        );
        assert_eq!(fs::read(&*dest).unwrap(), b"binary content");
    }

    #[test]
    fn resolve_cache_root_non_git_uses_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let root = resolve_cache_root(tmp.path());
        assert_eq!(root, tmp.path().join(".konvoy").join("cache"));
    }

    #[test]
    fn resolve_cache_root_normal_repo_uses_project_root() {
        let tmp = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let root = resolve_cache_root(tmp.path());
        assert_eq!(root, tmp.path().join(".konvoy").join("cache"));
    }

    #[test]
    fn resolve_cache_root_worktree_uses_main_root() {
        let tmp = tempfile::tempdir().unwrap();
        let main_dir = tmp.path().join("main");
        fs::create_dir_all(&main_dir).unwrap();

        // Init a repo in main_dir with an initial commit.
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&main_dir)
            .output()
            .unwrap();
        for cmd in &[
            vec!["config", "user.email", "t@t.com"],
            vec!["config", "user.name", "T"],
            vec!["commit", "-q", "--allow-empty", "-m", "init"],
        ] {
            Command::new("git")
                .args(cmd)
                .current_dir(&main_dir)
                .output()
                .unwrap();
        }

        // Create a worktree.
        let wt_dir = tmp.path().join("wt");
        Command::new("git")
            .args([
                "worktree",
                "add",
                "-q",
                wt_dir.display().to_string().as_str(),
                "-b",
                "wt",
            ])
            .current_dir(&main_dir)
            .output()
            .unwrap();

        let root = resolve_cache_root(&wt_dir);
        let expected = main_dir
            .canonicalize()
            .unwrap()
            .join(".konvoy")
            .join("cache");
        assert_eq!(root, expected);
    }
}
