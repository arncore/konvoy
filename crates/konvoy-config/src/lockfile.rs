use serde::{Deserialize, Serialize};
use std::path::Path;

/// The `konvoy.lock` lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Lockfile {
    #[serde(default)]
    pub toolchain: Option<ToolchainLock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainLock {
    pub konanc_version: String,
}

impl Lockfile {
    /// Read and parse a `konvoy.lock` from the given path.
    /// Returns a default lockfile if the file does not exist.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or contains invalid TOML.
    pub fn from_path(path: &Path) -> Result<Self, LockfileError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path).map_err(|e| LockfileError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
        let lockfile: Lockfile = toml::from_str(&content).map_err(|e| LockfileError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(lockfile)
    }

    /// Create a lockfile with a pinned toolchain version.
    pub fn with_toolchain(version: &str) -> Self {
        Self {
            toolchain: Some(ToolchainLock {
                konanc_version: version.to_owned(),
            }),
        }
    }

    /// Write the lockfile to disk as human-readable TOML.
    ///
    /// # Errors
    /// Returns an error if serialization fails or the file cannot be written.
    pub fn write_to(&self, path: &Path) -> Result<(), LockfileError> {
        let content =
            toml::to_string_pretty(self).map_err(|e| LockfileError::Serialize { source: e })?;
        std::fs::write(path, content).map_err(|e| LockfileError::Write {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LockfileError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid konvoy.lock at {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("cannot serialize lockfile: {source}")]
    Serialize { source: toml::ser::Error },
    #[error("cannot write {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_valid_lockfile() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "1.9.22"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        let toolchain = lockfile
            .toolchain
            .as_ref()
            .unwrap_or_else(|| panic!("missing toolchain"));
        assert_eq!(toolchain.konanc_version, "1.9.22");
    }

    #[test]
    fn default_when_absent() {
        let dir = tempdir();
        let path = dir.join("nonexistent.lock");
        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(lockfile.toolchain.is_none());
    }

    #[test]
    fn write_to_disk() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        let lockfile = Lockfile::with_toolchain("2.0.0");
        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));

        let content = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(content.contains("2.0.0"), "content was: {content}");
    }

    #[test]
    fn round_trip() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");

        let original = Lockfile::with_toolchain("1.9.22");
        original.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let reparsed = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(original, reparsed);
    }

    #[test]
    fn with_toolchain_creates_lockfile() {
        let lockfile = Lockfile::with_toolchain("1.9.22");
        let toolchain = lockfile
            .toolchain
            .as_ref()
            .unwrap_or_else(|| panic!("missing toolchain"));
        assert_eq!(toolchain.konanc_version, "1.9.22");
    }

    /// Create a unique temporary directory for each test invocation.
    fn tempdir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("konvoy-test-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }
}
