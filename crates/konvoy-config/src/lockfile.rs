use serde::{Deserialize, Serialize};
use std::path::Path;

/// The `konvoy.lock` lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    #[serde(default)]
    pub toolchain: Option<ToolchainLock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyLock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginLock>,
}

/// A locked plugin artifact entry in the lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLock {
    /// Human-readable plugin name (e.g. `"kotlin-serialization"`).
    pub name: String,
    /// Maven coordinate in `groupId:artifactId` format.
    pub maven: String,
    /// Resolved version (e.g. `"2.1.0"`, never `"{kotlin}"`).
    pub version: String,
    /// Hex-encoded SHA-256 hash of the artifact file.
    pub sha256: String,
    /// Download URL used to fetch this artifact.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolchainLock {
    pub konanc_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub konanc_tarball_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jre_tarball_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detekt_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detekt_jar_sha256: Option<String>,
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
    Path {
        path: String,
    },
    Maven {
        version: String,
        /// The canonical `groupId:artifactId` coordinate (no template placeholders).
        maven: String,
        targets: std::collections::BTreeMap<String, String>,
        /// Names of dependencies that pulled this one in transitively.
        /// Empty for direct deps declared in `konvoy.toml`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required_by: Vec<String>,
        /// Maven classifier for non-primary artifacts (e.g. `"cinterop-interop"`).
        ///
        /// When set, the download URL includes the classifier in the filename:
        /// `{artifact}-{target}-{version}-{classifier}.klib`.
        /// Most dependencies do not have a classifier.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        classifier: Option<String>,
    },
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
                detekt_version: None,
                detekt_jar_sha256: None,
            }),
            dependencies: Vec::new(),
            plugins: Vec::new(),
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
                detekt_version: None,
                detekt_jar_sha256: None,
            }),
            dependencies: Vec::new(),
            plugins: Vec::new(),
        }
    }

    /// Write the lockfile to disk as human-readable TOML.
    ///
    /// Uses an atomic write-to-temp-then-rename pattern so that readers never
    /// observe a partially-written lockfile.
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_valid_lockfile() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "1.9.22"
"#,
        )
        .unwrap();

        let lockfile = Lockfile::from_path(&path).unwrap();
        let toolchain = lockfile
            .toolchain
            .as_ref()
            .unwrap_or_else(|| panic!("missing toolchain"));
        assert_eq!(toolchain.konanc_version, "1.9.22");
    }

    #[test]
    fn default_when_absent() {
        let dir = make_test_dir();
        let path = dir.path().join("nonexistent.lock");
        let lockfile = Lockfile::from_path(&path).unwrap();
        assert!(lockfile.toolchain.is_none());
    }

    #[test]
    fn write_to_disk() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let lockfile = Lockfile::with_toolchain("2.0.0");
        lockfile.write_to(&path).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("2.0.0"), "content was: {content}");
    }

    #[test]
    fn round_trip() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");

        let original = Lockfile::with_toolchain("1.9.22");
        original.write_to(&path).unwrap();
        let reparsed = Lockfile::from_path(&path).unwrap();
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
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let original =
            Lockfile::with_managed_toolchain("2.1.0", Some("deadbeef"), Some("cafebabe"));
        original.write_to(&path).unwrap();
        let reparsed = Lockfile::from_path(&path).unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn sha256_skipped_when_absent() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"
"#,
        )
        .unwrap();

        let lockfile = Lockfile::from_path(&path).unwrap();
        let toolchain = lockfile
            .toolchain
            .as_ref()
            .unwrap_or_else(|| panic!("missing toolchain"));
        assert!(toolchain.konanc_tarball_sha256.is_none());
        assert!(toolchain.jre_tarball_sha256.is_none());
    }

    #[test]
    fn unknown_toolchain_field_rejected() {
        // Unknown fields in [toolchain] must be rejected so that typos
        // (e.g. the old `tarball_sha256` name) are caught immediately.
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"
tarball_sha256 = "oldvalue"
"#,
        )
        .unwrap();

        let err = Lockfile::from_path(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field"),
            "expected 'unknown field' in error, got: {msg}"
        );
    }

    #[test]
    fn round_trip_with_dependencies() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-utils".to_owned(),
            source: DepSource::Path {
                path: "../my-utils".to_owned(),
            },
            source_hash: "abcdef1234".to_owned(),
        });
        lockfile.write_to(&path).unwrap();
        let reparsed = Lockfile::from_path(&path).unwrap();
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
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"
"#,
        )
        .unwrap();

        let lockfile = Lockfile::from_path(&path).unwrap();
        assert!(lockfile.dependencies.is_empty());
    }

    #[test]
    fn unknown_top_level_field_rejected() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
bogus_field = "oops"

[toolchain]
konanc_version = "2.1.0"
"#,
        )
        .unwrap();

        let err = Lockfile::from_path(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field"),
            "expected 'unknown field' in error, got: {msg}"
        );
    }

    #[test]
    fn atomic_write_no_temp_file_after_success() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let tmp_path = path.with_extension("lock.tmp");
        let lockfile = Lockfile::with_toolchain("2.0.0");
        lockfile.write_to(&path).unwrap();

        assert!(path.exists(), "lockfile should exist after write");
        assert!(
            !tmp_path.exists(),
            "temp file should not exist after successful write"
        );
    }

    #[test]
    fn round_trip_with_plugins() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.plugins.push(PluginLock {
            name: "kotlin-serialization".to_owned(),
            maven: "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin".to_owned(),
            version: "2.1.0".to_owned(),
            sha256: "abc123def456".to_owned(),
            url: "https://repo1.maven.org/maven2/org/jetbrains/kotlin/kotlin-serialization-compiler-plugin/2.1.0/kotlin-serialization-compiler-plugin-2.1.0.jar".to_owned(),
        });
        lockfile.write_to(&path).unwrap();
        let reparsed = Lockfile::from_path(&path).unwrap();
        assert_eq!(lockfile, reparsed);
        assert_eq!(reparsed.plugins.len(), 1);
    }

    #[test]
    fn backward_compat_no_plugins() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"
"#,
        )
        .unwrap();

        let lockfile = Lockfile::from_path(&path).unwrap();
        assert!(lockfile.plugins.is_empty());
    }

    #[test]
    fn empty_plugins_omitted_in_toml() {
        let lockfile = Lockfile::with_toolchain("2.1.0");
        let content = toml::to_string_pretty(&lockfile).unwrap();
        assert!(!content.contains("plugins"), "content was: {content}");
    }

    #[test]
    fn empty_deps_omitted_in_toml() {
        let lockfile = Lockfile::with_toolchain("2.1.0");
        let content = toml::to_string_pretty(&lockfile).unwrap();
        assert!(!content.contains("dependencies"), "content was: {content}");
    }

    /// Create a unique temporary directory that is auto-cleaned on drop.
    fn make_test_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn round_trip_with_maven_dep() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "aabbccdd".to_owned());
        targets.insert("macos_arm64".to_owned(), "11223344".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "kotlinx-coroutines".to_owned(),
            source: DepSource::Maven {
                version: "1.8.0".to_owned(),
                maven: "org.jetbrains.kotlinx:kotlinx-coroutines-core".to_owned(),
                targets,
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "maven-hash-1234".to_owned(),
        });
        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let reparsed = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile, reparsed);
        assert_eq!(reparsed.dependencies.len(), 1);
        let dep = reparsed
            .dependencies
            .first()
            .unwrap_or_else(|| panic!("missing dep"));
        assert_eq!(dep.name, "kotlinx-coroutines");
        match &dep.source {
            DepSource::Maven {
                version,
                maven,
                targets,
                required_by,
                classifier,
            } => {
                assert_eq!(version, "1.8.0");
                assert_eq!(maven, "org.jetbrains.kotlinx:kotlinx-coroutines-core");
                assert_eq!(targets.len(), 2);
                assert_eq!(targets.get("linux_x64").unwrap(), "aabbccdd");
                assert_eq!(targets.get("macos_arm64").unwrap(), "11223344");
                assert!(required_by.is_empty());
                assert!(classifier.is_none());
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    #[test]
    fn maven_dep_serialization_format() {
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "deadbeef".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "kotlinx-datetime".to_owned(),
            source: DepSource::Maven {
                version: "0.6.0".to_owned(),
                maven: "org.jetbrains.kotlinx:kotlinx-datetime".to_owned(),
                targets,
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "hash-5678".to_owned(),
        });
        let content = toml::to_string_pretty(&lockfile).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            content.contains("source_type = \"maven\""),
            "content was: {content}"
        );
        assert!(
            content.contains("version = \"0.6.0\""),
            "content was: {content}"
        );
        assert!(
            content.contains("maven = \"org.jetbrains.kotlinx:kotlinx-datetime\""),
            "content was: {content}"
        );
        assert!(
            !content.contains("required_by"),
            "required_by should be omitted for direct deps, content was: {content}"
        );
        assert!(
            content.contains("[dependencies.targets]")
                || content.contains("targets") && content.contains("linux_x64"),
            "content was: {content}"
        );
    }

    #[test]
    fn backward_compat_path_deps_still_work() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"

[[dependencies]]
name = "my-utils"
source_type = "path"
path = "../my-utils"
source_hash = "abcdef1234"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile.dependencies.len(), 1);
        let dep = lockfile
            .dependencies
            .first()
            .unwrap_or_else(|| panic!("missing dep"));
        assert_eq!(dep.name, "my-utils");
        match &dep.source {
            DepSource::Path { path } => {
                assert_eq!(path, "../my-utils");
            }
            other => panic!("expected Path source, got: {other:?}"),
        }
    }

    #[test]
    fn round_trip_with_required_by() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "789xyz".to_owned());
        targets.insert("macos_arm64".to_owned(), "uvw012".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets,
                required_by: vec!["kotlinx-coroutines".to_owned()],
                classifier: None,
            },
            source_hash: "transitive-hash".to_owned(),
        });
        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let reparsed = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile, reparsed);
        let dep = reparsed
            .dependencies
            .first()
            .unwrap_or_else(|| panic!("missing dep"));
        match &dep.source {
            DepSource::Maven {
                version,
                maven,
                required_by,
                ..
            } => {
                assert_eq!(version, "0.23.1");
                assert_eq!(maven, "org.jetbrains.kotlinx:atomicfu");
                assert_eq!(required_by, &["kotlinx-coroutines"]);
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    #[test]
    fn required_by_omitted_for_direct_deps() {
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "aabb".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "kotlinx-coroutines".to_owned(),
            source: DepSource::Maven {
                version: "1.8.0".to_owned(),
                maven: "org.jetbrains.kotlinx:kotlinx-coroutines-core".to_owned(),
                targets,
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "direct-hash".to_owned(),
        });
        let content = toml::to_string_pretty(&lockfile).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            !content.contains("required_by"),
            "required_by should not appear for direct deps, content was: {content}"
        );
    }

    #[test]
    fn required_by_present_for_transitive_deps() {
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "ccdd".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets,
                required_by: vec!["kotlinx-coroutines".to_owned()],
                classifier: None,
            },
            source_hash: "transitive-hash".to_owned(),
        });
        let content = toml::to_string_pretty(&lockfile).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            content.contains("required_by"),
            "required_by should appear for transitive deps, content was: {content}"
        );
        assert!(
            content.contains("kotlinx-coroutines"),
            "required_by should contain parent dep name, content was: {content}"
        );
    }

    #[test]
    fn maven_field_is_plain_group_artifact() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "eeff".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "kotlinx-coroutines".to_owned(),
            source: DepSource::Maven {
                version: "1.8.0".to_owned(),
                maven: "org.jetbrains.kotlinx:kotlinx-coroutines-core".to_owned(),
                targets,
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "some-hash".to_owned(),
        });
        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let content = fs::read_to_string(&path).unwrap();
        // The maven field should not contain template placeholders.
        assert!(
            !content.contains("{target}"),
            "maven field should not contain {{target}}, content was: {content}"
        );
        assert!(
            !content.contains("{version}"),
            "maven field should not contain {{version}}, content was: {content}"
        );
        // It should be plain groupId:artifactId.
        assert!(
            content.contains("maven = \"org.jetbrains.kotlinx:kotlinx-coroutines-core\""),
            "maven field should be plain groupId:artifactId, content was: {content}"
        );
    }

    #[test]
    fn round_trip_with_classifier() {
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "cinterop-hash".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu-cinterop-interop".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets,
                required_by: vec!["atomicfu".to_owned()],
                classifier: Some("cinterop-interop".to_owned()),
            },
            source_hash: "classifier-hash".to_owned(),
        });
        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let reparsed = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile, reparsed);
        let dep = reparsed
            .dependencies
            .first()
            .unwrap_or_else(|| panic!("missing dep"));
        match &dep.source {
            DepSource::Maven { classifier, .. } => {
                assert_eq!(classifier.as_deref(), Some("cinterop-interop"));
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    #[test]
    fn classifier_omitted_in_toml_when_none() {
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "hash".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets,
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "some-hash".to_owned(),
        });
        let content = toml::to_string_pretty(&lockfile).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            !content.contains("classifier"),
            "classifier should not appear when None, content was: {content}"
        );
    }

    #[test]
    fn classifier_present_in_toml_when_set() {
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = std::collections::BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "hash".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu-cinterop-interop".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets,
                required_by: vec!["atomicfu".to_owned()],
                classifier: Some("cinterop-interop".to_owned()),
            },
            source_hash: "hash".to_owned(),
        });
        let content = toml::to_string_pretty(&lockfile).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            content.contains("classifier = \"cinterop-interop\""),
            "classifier should appear when Some, content was: {content}"
        );
    }

    #[test]
    fn backward_compat_no_classifier_field() {
        // Old lockfiles without a classifier field should parse correctly.
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"

[[dependencies]]
name = "atomicfu"
source_type = "maven"
version = "0.23.1"
maven = "org.jetbrains.kotlinx:atomicfu"
source_hash = "abcdef"

[dependencies.targets]
linux_x64 = "hash123"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile.dependencies.len(), 1);
        match &lockfile.dependencies.first().unwrap().source {
            DepSource::Maven { classifier, .. } => {
                assert!(classifier.is_none(), "classifier should default to None");
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    #[test]
    fn round_trip_mixed_deps_with_and_without_classifier() {
        // A lockfile with both a regular Maven dep (no classifier) and a
        // cinterop dep (with classifier) should round-trip correctly.
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");

        // Main klib (no classifier).
        let mut targets1 = std::collections::BTreeMap::new();
        targets1.insert("linux_x64".to_owned(), "main-hash".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets: targets1,
                required_by: vec!["kotlinx-coroutines".to_owned()],
                classifier: None,
            },
            source_hash: "main-source-hash".to_owned(),
        });

        // Cinterop klib (with classifier).
        let mut targets2 = std::collections::BTreeMap::new();
        targets2.insert("linux_x64".to_owned(), "cinterop-hash".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu-cinterop-interop".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets: targets2,
                required_by: vec!["atomicfu".to_owned()],
                classifier: Some("cinterop-interop".to_owned()),
            },
            source_hash: "cinterop-source-hash".to_owned(),
        });

        lockfile.write_to(&path).unwrap_or_else(|e| panic!("{e}"));
        let reparsed = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile, reparsed);
        assert_eq!(reparsed.dependencies.len(), 2);

        // Verify the first dep has no classifier.
        match &reparsed.dependencies.first().unwrap().source {
            DepSource::Maven { classifier, .. } => {
                assert!(classifier.is_none());
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
        // Verify the second dep has a classifier.
        match &reparsed.dependencies.get(1).unwrap().source {
            DepSource::Maven { classifier, .. } => {
                assert_eq!(classifier.as_deref(), Some("cinterop-interop"));
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    #[test]
    fn backward_compat_old_lockfile_without_classifier_or_required_by() {
        // An old lockfile that has neither classifier nor required_by should
        // parse correctly with both defaulting to their zero values.
        let dir = make_test_dir();
        let path = dir.path().join("konvoy.lock");
        fs::write(
            &path,
            r#"
[toolchain]
konanc_version = "2.1.0"

[[dependencies]]
name = "kotlinx-coroutines"
source_type = "maven"
version = "1.8.0"
maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core"
source_hash = "old-hash"

[dependencies.targets]
linux_x64 = "aabb"
macos_arm64 = "ccdd"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let lockfile = Lockfile::from_path(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(lockfile.dependencies.len(), 1);
        match &lockfile.dependencies.first().unwrap().source {
            DepSource::Maven {
                classifier,
                required_by,
                targets,
                ..
            } => {
                assert!(classifier.is_none());
                assert!(required_by.is_empty());
                assert_eq!(targets.len(), 2);
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
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
                let dir = make_test_dir();
                let path = dir.path().join("konvoy.lock");
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
                let dir = make_test_dir();
                let path = dir.path().join("konvoy.lock");
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
