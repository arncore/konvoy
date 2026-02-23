use serde::{Deserialize, Serialize};
use std::path::Path;

/// The `konvoy.lock` lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Lockfile {
    #[serde(default)]
    pub toolchain: Option<ToolchainLock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyLock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainLock {
    pub konanc_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub konanc_tarball_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jre_tarball_sha256: Option<String>,
}

/// A locked dependency entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyLock {
    pub name: String,
    #[serde(flatten)]
    pub source: DepSource,
    pub source_hash: String,
}

/// The resolved source of a dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "source_type")]
pub enum DepSource {
    Path { path: String },
    // Future: Git { url: String, commit: String },
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
                konanc_tarball_sha256: None,
                jre_tarball_sha256: None,
            }),
            dependencies: Vec::new(),
        }
    }

    /// Create a lockfile with a pinned toolchain version and tarball SHA-256s.
    pub fn with_managed_toolchain(
        version: &str,
        konanc_sha256: Option<&str>,
        jre_sha256: Option<&str>,
    ) -> Self {
        Self {
            toolchain: Some(ToolchainLock {
                konanc_version: version.to_owned(),
                konanc_tarball_sha256: konanc_sha256.map(str::to_owned),
                jre_tarball_sha256: jre_sha256.map(str::to_owned),
            }),
            dependencies: Vec::new(),
        }
    }

    /// Write the lockfile to disk as human-readable TOML.
    ///
    /// Uses atomic write (write-to-temp-then-rename) to prevent partial writes
    /// from corrupting the lockfile.
    ///
    /// # Errors
    /// Returns an error if serialization fails or the file cannot be written.
    pub fn write_to(&self, path: &Path) -> Result<(), LockfileError> {
        let content =
            toml::to_string_pretty(self).map_err(|e| LockfileError::Serialize { source: e })?;
        let tmp_path = path.with_extension("lock.tmp");
        std::fs::write(&tmp_path, &content).map_err(|e| LockfileError::Write {
            path: tmp_path.display().to_string(),
            source: e,
        })?;
        std::fs::rename(&tmp_path, path).map_err(|e| LockfileError::Write {
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
    fn write_to_disk_no_temp_file_remains() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        let tmp_path = path.with_extension("lock.tmp");
        let lockfile = Lockfile::with_toolchain("2.0.0");
        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));

        assert!(path.exists(), "lockfile should exist after write");
        assert!(
            !tmp_path.exists(),
            "temp file should not exist after successful write"
        );
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
        assert!(toolchain.konanc_tarball_sha256.is_none());
        assert!(toolchain.jre_tarball_sha256.is_none());
    }

    #[test]
    fn with_managed_toolchain_includes_sha256() {
        let lockfile = Lockfile::with_managed_toolchain("2.1.0", Some("abc123"), Some("def456"));
        let toolchain = lockfile
            .toolchain
            .as_ref()
            .unwrap_or_else(|| panic!("missing toolchain"));
        assert_eq!(toolchain.konanc_version, "2.1.0");
        assert_eq!(toolchain.konanc_tarball_sha256.as_deref(), Some("abc123"));
        assert_eq!(toolchain.jre_tarball_sha256.as_deref(), Some("def456"));
    }

    #[test]
    fn sha256_round_trip() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        let original =
            Lockfile::with_managed_toolchain("2.1.0", Some("deadbeef"), Some("cafebabe"));
        original.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let reparsed = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(original, reparsed);
    }

    #[test]
    fn sha256_skipped_when_absent() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        let toolchain = lockfile
            .toolchain
            .as_ref()
            .unwrap_or_else(|| panic!("missing toolchain"));
        assert!(toolchain.konanc_tarball_sha256.is_none());
        assert!(toolchain.jre_tarball_sha256.is_none());
    }

    #[test]
    fn backward_compat_old_tarball_sha256_field() {
        // Old lockfiles may have `tarball_sha256` — they should still parse
        // (the field is simply ignored since it's been renamed).
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"
tarball_sha256 = "oldvalue"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        // Should parse without error (unknown fields are ignored by default in TOML).
        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        let toolchain = lockfile
            .toolchain
            .as_ref()
            .unwrap_or_else(|| panic!("missing toolchain"));
        assert_eq!(toolchain.konanc_version, "2.1.0");
    }

    #[test]
    fn round_trip_with_dependencies() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-utils".to_owned(),
            source: DepSource::Path {
                path: "../my-utils".to_owned(),
            },
            source_hash: "abcdef1234".to_owned(),
        });
        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let reparsed = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile, reparsed);
        assert_eq!(reparsed.dependencies.len(), 1);
        let dep = reparsed
            .dependencies
            .first()
            .unwrap_or_else(|| panic!("missing dep"));
        assert_eq!(dep.name, "my-utils");
        assert_eq!(dep.source_hash, "abcdef1234");
    }

    #[test]
    fn backward_compat_no_deps() {
        let dir = tempdir();
        let path = dir.join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(lockfile.dependencies.is_empty());
    }

    #[test]
    fn empty_deps_omitted_in_toml() {
        let lockfile = Lockfile::with_toolchain("2.1.0");
        let content = toml::to_string_pretty(&lockfile).unwrap_or_else(|e| panic!("{e}"));
        assert!(!content.contains("dependencies"), "content was: {content}");
    }

    /// Create a unique temporary directory for each test invocation.
    fn tempdir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("konvoy-test-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            #[allow(clippy::unwrap_used)]
            fn lockfile_round_trip(
                version in "[0-9]{1,2}\\.[0-9]{1,2}\\.[0-9]{1,2}",
                konanc_sha in "[a-f0-9]{64}",
                jre_sha in "[a-f0-9]{64}",
            ) {
                let dir = tempdir();
                let path = dir.join("konvoy.lock");
                let original = Lockfile::with_managed_toolchain(
                    &version,
                    Some(&konanc_sha),
                    Some(&jre_sha),
                );
                original.write_to(&path).unwrap();
                let reparsed = Lockfile::from_path(&path).unwrap();
                prop_assert_eq!(original, reparsed);
            }

            #[test]
            #[allow(clippy::unwrap_used)]
            fn lockfile_with_deps_round_trip(
                version in "[0-9]{1,2}\\.[0-9]{1,2}\\.[0-9]{1,2}",
                dep_name in "[a-zA-Z][a-zA-Z0-9_-]{0,20}",
                dep_path in "\\.\\./[a-zA-Z][a-zA-Z0-9_-]{0,15}",
                source_hash in "[a-f0-9]{16,64}",
            ) {
                let dir = tempdir();
                let path = dir.join("konvoy.lock");
                let mut lockfile = Lockfile::with_toolchain(&version);
                lockfile.dependencies.push(DependencyLock {
                    name: dep_name,
                    source: DepSource::Path { path: dep_path },
                    source_hash,
                });
                lockfile.write_to(&path).unwrap();
                let reparsed = Lockfile::from_path(&path).unwrap();
                prop_assert_eq!(lockfile, reparsed);
            }
        }
    }
}
