//! Plugin system for Kotlin compiler plugins.
//!
//! Plugins are declared in `[plugins]` using the same `{ maven, version }` syntax
//! as dependencies. Any Maven-published compiler plugin JAR can be used without
//! needing a built-in descriptor.

use std::path::PathBuf;

use konvoy_config::lockfile::{Lockfile, PluginLock};
use konvoy_config::manifest::Manifest;
use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

use crate::error::EngineError;

/// A fully resolved plugin artifact ready for download.
#[derive(Debug, Clone)]
pub struct ResolvedPluginArtifact {
    /// The plugin name (key in `[plugins]`).
    pub plugin_name: String,
    /// Parsed Maven coordinate for the compiler plugin JAR.
    pub maven_coord: MavenCoordinate,
    /// Full download URL.
    pub url: String,
    /// Local cache path where this artifact is stored.
    pub cache_path: PathBuf,
}

/// Result of ensuring a single plugin artifact is available.
#[derive(Debug, Clone)]
pub struct PluginArtifactResult {
    /// The plugin name (key in `[plugins]`).
    pub plugin_name: String,
    /// Path to the artifact on disk.
    pub path: PathBuf,
    /// Hex-encoded SHA-256 hash.
    pub sha256: String,
    /// Full download URL.
    pub url: String,
    /// Whether the artifact was freshly downloaded.
    pub freshly_downloaded: bool,
    /// Maven coordinate in `groupId:artifactId` format (for lockfile).
    pub maven: String,
    /// Resolved version (for lockfile).
    pub version: String,
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Return the Maven cache root directory (`~/.konvoy/cache/maven`).
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
fn maven_cache_root() -> Result<PathBuf, EngineError> {
    Ok(konvoy_util::fs::konvoy_home()?.join("cache").join("maven"))
}

/// Resolve all plugin artifacts required by the manifest.
///
/// For each plugin declared in `manifest.plugins`, this resolves the `{kotlin}`
/// placeholder in the version, builds a Maven coordinate, and computes the
/// download URL and cache path.
///
/// Compiler plugin JARs are platform-independent, so no `target` parameter is needed.
///
/// # Errors
/// Returns an error if a plugin is missing `maven` or `version`, if the maven
/// coordinate is malformed, or if the cache root cannot be determined.
pub fn resolve_plugin_artifacts(
    manifest: &Manifest,
) -> Result<Vec<ResolvedPluginArtifact>, EngineError> {
    let cache_root = maven_cache_root()?;
    let kotlin_version = &manifest.toolchain.kotlin;
    let mut artifacts = Vec::new();

    for (plugin_name, spec) in &manifest.plugins {
        let maven = spec
            .maven
            .as_ref()
            .ok_or_else(|| EngineError::InvalidPluginConfig {
                name: plugin_name.clone(),
                reason: "missing `maven` coordinate".to_owned(),
            })?;
        let version_template =
            spec.version
                .as_ref()
                .ok_or_else(|| EngineError::InvalidPluginConfig {
                    name: plugin_name.clone(),
                    reason: "missing `version`".to_owned(),
                })?;

        // Resolve {kotlin} placeholder.
        let resolved_version = version_template.replace("{kotlin}", kotlin_version);

        let (group_id, artifact_id) =
            maven
                .split_once(':')
                .ok_or_else(|| EngineError::InvalidPluginConfig {
                    name: plugin_name.clone(),
                    reason: format!("invalid maven coordinate `{maven}`"),
                })?;

        let coord = MavenCoordinate::new(group_id, artifact_id, &resolved_version);
        let url = coord.to_url(MAVEN_CENTRAL);
        let cache_path = coord.cache_path(&cache_root);

        artifacts.push(ResolvedPluginArtifact {
            plugin_name: plugin_name.clone(),
            maven_coord: coord,
            url,
            cache_path,
        });
    }

    Ok(artifacts)
}

// ---------------------------------------------------------------------------
// Download / verification
// ---------------------------------------------------------------------------

/// Look up the expected SHA-256 for a plugin from the lockfile.
fn find_lockfile_hash<'a>(lockfile: &'a Lockfile, plugin_name: &str) -> Option<&'a str> {
    lockfile
        .plugins
        .iter()
        .find(|p| p.name == plugin_name)
        .map(|p| p.sha256.as_str())
}

/// Map a `UtilError::Download` to `EngineError::PluginDownload`.
fn map_download_err(name: &str, e: konvoy_util::error::UtilError) -> EngineError {
    match e {
        konvoy_util::error::UtilError::Download { message } => EngineError::PluginDownload {
            name: name.to_owned(),
            message,
        },
        konvoy_util::error::UtilError::ArtifactHashMismatch {
            expected, actual, ..
        } => EngineError::PluginHashMismatch {
            name: name.to_owned(),
            expected,
            actual,
        },
        other => EngineError::Util(other),
    }
}

/// Ensure all plugin artifacts are downloaded, hash-verified, and return results.
///
/// In `--locked` mode, every artifact must already have a hash in the lockfile.
///
/// # Errors
/// Returns an error if an artifact is missing from the lockfile in locked mode,
/// if a download fails, or if a hash does not match.
pub fn ensure_plugin_artifacts(
    artifacts: &[ResolvedPluginArtifact],
    lockfile: &Lockfile,
    locked: bool,
) -> Result<Vec<PluginArtifactResult>, EngineError> {
    let mut results = Vec::with_capacity(artifacts.len());

    for artifact in artifacts {
        let expected_hash = find_lockfile_hash(lockfile, &artifact.plugin_name);

        // In --locked mode, the hash must be present in the lockfile.
        if locked && expected_hash.is_none() {
            return Err(EngineError::LockfileUpdateRequired);
        }

        let util_result = konvoy_util::artifact::ensure_artifact(
            &artifact.url,
            &artifact.cache_path,
            expected_hash,
            &artifact.plugin_name,
            &artifact.maven_coord.version,
        )
        .map_err(|e| map_download_err(&artifact.plugin_name, e))?;

        // Reconstruct the groupId:artifactId for the lockfile.
        let maven = format!(
            "{}:{}",
            artifact.maven_coord.group_id, artifact.maven_coord.artifact_id
        );

        results.push(PluginArtifactResult {
            plugin_name: artifact.plugin_name.clone(),
            path: util_result.path,
            sha256: util_result.sha256,
            url: artifact.url.clone(),
            freshly_downloaded: util_result.freshly_downloaded,
            maven,
            version: artifact.maven_coord.version.clone(),
        });
    }

    Ok(results)
}

/// Build `PluginLock` entries from download results.
pub fn build_plugin_locks(results: &[PluginArtifactResult]) -> Vec<PluginLock> {
    results
        .iter()
        .map(|r| PluginLock {
            name: r.plugin_name.clone(),
            maven: r.maven.clone(),
            version: r.version.clone(),
            sha256: r.sha256.clone(),
            url: r.url.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn resolve_plugin_artifacts_basic() {
        let manifest = make_manifest_with_plugin(
            "kotlin-serialization",
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin",
            "2.1.0",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();

        assert_eq!(artifacts.len(), 1);
        let a = &artifacts[0];
        assert_eq!(a.plugin_name, "kotlin-serialization");
        assert!(
            a.url.contains("kotlin-serialization-compiler-plugin"),
            "url was: {}",
            a.url
        );
        assert!(
            a.url.contains("2.1.0"),
            "url should contain version: {}",
            a.url
        );
        assert_eq!(a.maven_coord.packaging, "jar");
    }

    #[test]
    fn resolve_plugin_artifacts_kotlin_placeholder() {
        let manifest = make_manifest_with_plugin(
            "kotlin-serialization",
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin",
            "{kotlin}",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();

        assert_eq!(artifacts.len(), 1);
        let a = &artifacts[0];
        // {kotlin} should resolve to the toolchain version "2.1.0".
        assert_eq!(a.maven_coord.version, "2.1.0");
        assert!(
            a.url.contains("2.1.0"),
            "url should contain resolved version: {}",
            a.url
        );
    }

    #[test]
    fn resolve_plugin_artifacts_no_target_needed() {
        // Compiler plugin JARs are platform-independent.
        // The function takes no `target` parameter — just verify it works.
        let manifest = make_manifest_with_plugin(
            "kotlin-serialization",
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin",
            "{kotlin}",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert_eq!(artifacts.len(), 1);
        // Cache path should be under cache/maven.
        let path_str = artifacts[0].cache_path.display().to_string();
        assert!(
            path_str.contains("cache/maven"),
            "path should be under cache/maven: {path_str}"
        );
    }

    #[test]
    fn resolve_plugin_artifacts_missing_maven() {
        let mut plugins = BTreeMap::new();
        plugins.insert(
            "bad-plugin".to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: None,
                version: Some("1.0.0".to_owned()),
            },
        );
        let manifest = Manifest {
            package: default_package(),
            toolchain: default_toolchain(),
            dependencies: BTreeMap::new(),
            plugins,
        };
        let result = resolve_plugin_artifacts(&manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing `maven`"), "error was: {err}");
    }

    #[test]
    fn resolve_plugin_artifacts_missing_version() {
        let mut plugins = BTreeMap::new();
        plugins.insert(
            "bad-plugin".to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: Some("org.example:plugin".to_owned()),
                version: None,
            },
        );
        let manifest = Manifest {
            package: default_package(),
            toolchain: default_toolchain(),
            dependencies: BTreeMap::new(),
            plugins,
        };
        let result = resolve_plugin_artifacts(&manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing `version`"), "error was: {err}");
    }

    #[test]
    fn find_lockfile_hash_present() {
        let lockfile = Lockfile {
            plugins: vec![PluginLock {
                name: "kotlin-serialization".to_owned(),
                maven: "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin".to_owned(),
                version: "2.1.0".to_owned(),
                sha256: "abc123".to_owned(),
                url: "https://example.com/plugin.jar".to_owned(),
            }],
            ..Lockfile::default()
        };

        let hash = find_lockfile_hash(&lockfile, "kotlin-serialization");
        assert_eq!(hash, Some("abc123"));
    }

    #[test]
    fn find_lockfile_hash_absent() {
        let lockfile = Lockfile::default();
        let hash = find_lockfile_hash(&lockfile, "kotlin-serialization");
        assert!(hash.is_none());
    }

    #[test]
    fn build_plugin_locks_from_results() {
        let results = vec![PluginArtifactResult {
            plugin_name: "kotlin-serialization".to_owned(),
            path: PathBuf::from("/cache/plugin.jar"),
            sha256: "abc123".to_owned(),
            url: "https://example.com/plugin.jar".to_owned(),
            freshly_downloaded: true,
            maven: "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin".to_owned(),
            version: "2.1.0".to_owned(),
        }];

        let locks = build_plugin_locks(&results);
        assert_eq!(locks.len(), 1);
        let lock = &locks[0];
        assert_eq!(lock.name, "kotlin-serialization");
        assert_eq!(
            lock.maven,
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
        );
        assert_eq!(lock.version, "2.1.0");
        assert_eq!(lock.sha256, "abc123");
        assert_eq!(lock.url, "https://example.com/plugin.jar");
    }

    #[test]
    fn cache_path_uses_maven_layout() {
        let manifest = make_manifest_with_plugin(
            "kotlin-serialization",
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin",
            "2.1.0",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();

        let path_str = artifacts[0].cache_path.display().to_string();
        assert!(
            path_str.contains("cache/maven"),
            "path should be under cache/maven: {path_str}"
        );
        assert!(
            path_str.contains("org/jetbrains/kotlin"),
            "path should have group path: {path_str}"
        );
    }

    #[test]
    fn resolve_plugin_artifacts_empty_plugins() {
        let manifest = Manifest {
            package: default_package(),
            toolchain: default_toolchain(),
            dependencies: BTreeMap::new(),
            plugins: BTreeMap::new(),
        };
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert!(artifacts.is_empty());
    }

    #[test]
    fn resolve_plugin_artifacts_multiple_plugins() {
        let mut plugins = BTreeMap::new();
        plugins.insert(
            "kotlin-serialization".to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: Some("org.jetbrains.kotlin:kotlin-serialization-compiler-plugin".to_owned()),
                version: Some("{kotlin}".to_owned()),
            },
        );
        plugins.insert(
            "kotlin-allopen".to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: Some("org.jetbrains.kotlin:kotlin-allopen-compiler-plugin".to_owned()),
                version: Some("2.1.0".to_owned()),
            },
        );
        let manifest = Manifest {
            package: default_package(),
            toolchain: default_toolchain(),
            dependencies: BTreeMap::new(),
            plugins,
        };
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert_eq!(artifacts.len(), 2);
        // BTreeMap is sorted, so kotlin-allopen comes before kotlin-serialization.
        assert_eq!(artifacts[0].plugin_name, "kotlin-allopen");
        assert_eq!(artifacts[1].plugin_name, "kotlin-serialization");
    }

    #[test]
    fn build_plugin_locks_empty_results() {
        let results: Vec<PluginArtifactResult> = Vec::new();
        let locks = build_plugin_locks(&results);
        assert!(locks.is_empty());
    }

    #[test]
    fn build_plugin_locks_multiple_results() {
        let results = vec![
            PluginArtifactResult {
                plugin_name: "kotlin-serialization".to_owned(),
                path: PathBuf::from("/cache/serialization.jar"),
                sha256: "hash1".to_owned(),
                url: "https://example.com/serialization.jar".to_owned(),
                freshly_downloaded: true,
                maven: "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin".to_owned(),
                version: "2.1.0".to_owned(),
            },
            PluginArtifactResult {
                plugin_name: "kotlin-allopen".to_owned(),
                path: PathBuf::from("/cache/allopen.jar"),
                sha256: "hash2".to_owned(),
                url: "https://example.com/allopen.jar".to_owned(),
                freshly_downloaded: false,
                maven: "org.jetbrains.kotlin:kotlin-allopen-compiler-plugin".to_owned(),
                version: "2.1.0".to_owned(),
            },
        ];
        let locks = build_plugin_locks(&results);
        assert_eq!(locks.len(), 2);
        assert_eq!(locks[0].name, "kotlin-serialization");
        assert_eq!(locks[0].sha256, "hash1");
        assert_eq!(locks[1].name, "kotlin-allopen");
        assert_eq!(locks[1].sha256, "hash2");
    }

    #[test]
    fn resolve_plugin_artifacts_invalid_maven_coordinate() {
        let mut plugins = BTreeMap::new();
        plugins.insert(
            "bad-plugin".to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: Some("nocolon".to_owned()),
                version: Some("1.0.0".to_owned()),
            },
        );
        let manifest = Manifest {
            package: default_package(),
            toolchain: default_toolchain(),
            dependencies: BTreeMap::new(),
            plugins,
        };
        let result = resolve_plugin_artifacts(&manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn find_lockfile_hash_multiple_plugins_finds_correct_one() {
        let lockfile = Lockfile {
            plugins: vec![
                PluginLock {
                    name: "kotlin-serialization".to_owned(),
                    maven: "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin".to_owned(),
                    version: "2.1.0".to_owned(),
                    sha256: "hash-ser".to_owned(),
                    url: "https://example.com/serialization.jar".to_owned(),
                },
                PluginLock {
                    name: "kotlin-allopen".to_owned(),
                    maven: "org.jetbrains.kotlin:kotlin-allopen-compiler-plugin".to_owned(),
                    version: "2.1.0".to_owned(),
                    sha256: "hash-open".to_owned(),
                    url: "https://example.com/allopen.jar".to_owned(),
                },
            ],
            ..Lockfile::default()
        };
        assert_eq!(
            find_lockfile_hash(&lockfile, "kotlin-serialization"),
            Some("hash-ser")
        );
        assert_eq!(
            find_lockfile_hash(&lockfile, "kotlin-allopen"),
            Some("hash-open")
        );
        assert!(find_lockfile_hash(&lockfile, "nonexistent").is_none());
    }

    #[test]
    fn resolve_plugin_url_is_maven_central() {
        let manifest = make_manifest_with_plugin(
            "kotlin-serialization",
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin",
            "2.1.0",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert!(
            artifacts[0]
                .url
                .starts_with("https://repo1.maven.org/maven2/"),
            "url should point to Maven Central: {}",
            artifacts[0].url
        );
    }

    #[test]
    fn resolve_plugin_artifacts_long_maven_coordinate() {
        let manifest = make_manifest_with_plugin(
            "my-very-long-plugin-name",
            "com.very.long.organization.group.name:an-extremely-long-artifact-name-for-testing",
            "1.2.3",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0]
            .url
            .contains("an-extremely-long-artifact-name-for-testing"));
        assert_eq!(artifacts[0].maven_coord.version, "1.2.3");
    }

    #[test]
    fn resolve_plugin_artifacts_kotlin_placeholder_mid_string() {
        // {kotlin} can appear within a larger version string, e.g. "1.0-{kotlin}-beta".
        let manifest =
            make_manifest_with_plugin("my-plugin", "com.example:my-plugin", "1.0-{kotlin}-beta");
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].maven_coord.version, "1.0-2.1.0-beta");
    }

    #[test]
    fn resolve_plugin_artifacts_multiple_kotlin_placeholders() {
        // Multiple {kotlin} occurrences should all be replaced.
        let manifest =
            make_manifest_with_plugin("my-plugin", "com.example:my-plugin", "{kotlin}-{kotlin}");
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert_eq!(artifacts[0].maven_coord.version, "2.1.0-2.1.0");
    }

    #[test]
    fn resolve_plugin_artifacts_version_no_placeholder() {
        // A version without {kotlin} should be used as-is.
        let manifest = make_manifest_with_plugin("my-plugin", "com.example:my-plugin", "3.5.0");
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert_eq!(artifacts[0].maven_coord.version, "3.5.0");
    }

    #[test]
    fn resolve_plugin_url_contains_jar_extension() {
        // Plugin artifacts should be JARs (default packaging).
        let manifest = make_manifest_with_plugin(
            "kotlin-serialization",
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin",
            "2.1.0",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        assert!(
            artifacts[0].url.ends_with(".jar"),
            "plugin URL should end with .jar: {}",
            artifacts[0].url
        );
    }

    #[test]
    fn build_plugin_locks_preserves_ordering() {
        let results = vec![
            PluginArtifactResult {
                plugin_name: "z-plugin".to_owned(),
                path: PathBuf::from("/cache/z.jar"),
                sha256: "hash-z".to_owned(),
                url: "https://example.com/z.jar".to_owned(),
                freshly_downloaded: true,
                maven: "com.example:z-plugin".to_owned(),
                version: "1.0.0".to_owned(),
            },
            PluginArtifactResult {
                plugin_name: "a-plugin".to_owned(),
                path: PathBuf::from("/cache/a.jar"),
                sha256: "hash-a".to_owned(),
                url: "https://example.com/a.jar".to_owned(),
                freshly_downloaded: false,
                maven: "com.example:a-plugin".to_owned(),
                version: "2.0.0".to_owned(),
            },
        ];
        let locks = build_plugin_locks(&results);
        assert_eq!(locks.len(), 2);
        // Order must match input order, not sorted.
        assert_eq!(locks[0].name, "z-plugin");
        assert_eq!(locks[1].name, "a-plugin");
    }

    #[test]
    fn map_download_err_maps_download_error() {
        let err = konvoy_util::error::UtilError::Download {
            message: "connection refused".to_owned(),
        };
        let engine_err = map_download_err("my-plugin", err);
        let msg = engine_err.to_string();
        assert!(
            msg.contains("my-plugin"),
            "error should mention plugin name: {msg}"
        );
        assert!(
            msg.contains("connection refused"),
            "error should mention cause: {msg}"
        );
    }

    #[test]
    fn map_download_err_maps_hash_mismatch() {
        let err = konvoy_util::error::UtilError::ArtifactHashMismatch {
            path: "/cache/my-plugin.jar".to_owned(),
            expected: "aaa".to_owned(),
            actual: "bbb".to_owned(),
        };
        let engine_err = map_download_err("my-plugin", err);
        let msg = engine_err.to_string();
        assert!(
            msg.contains("hash mismatch"),
            "error should mention hash mismatch: {msg}"
        );
        assert!(msg.contains("aaa"), "error should mention expected: {msg}");
        assert!(msg.contains("bbb"), "error should mention actual: {msg}");
    }

    #[test]
    fn map_download_err_passes_through_other_errors() {
        let err = konvoy_util::error::UtilError::NoHomeDir;
        let engine_err = map_download_err("my-plugin", err);
        let msg = engine_err.to_string();
        assert!(
            msg.contains("home directory"),
            "other errors should pass through: {msg}"
        );
    }

    #[test]
    fn resolve_plugin_artifacts_with_both_plugins_and_deps() {
        // Manifest has both plugins and dependencies — plugins should resolve independently.
        let mut plugins = BTreeMap::new();
        plugins.insert(
            "kotlin-serialization".to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: Some("org.jetbrains.kotlin:kotlin-serialization-compiler-plugin".to_owned()),
                version: Some("{kotlin}".to_owned()),
            },
        );
        let mut dependencies = BTreeMap::new();
        dependencies.insert(
            "kotlinx-coroutines".to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: Some("org.jetbrains.kotlinx:kotlinx-coroutines-core".to_owned()),
                version: Some("1.8.0".to_owned()),
            },
        );
        let manifest = Manifest {
            package: default_package(),
            toolchain: default_toolchain(),
            dependencies,
            plugins,
        };
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        // Only plugin artifacts, not dependency artifacts.
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].plugin_name, "kotlin-serialization");
    }

    #[test]
    fn find_lockfile_hash_ignores_dependency_entries() {
        // Lockfile has deps but no plugins — plugin lookup should return None.
        let mut lockfile = Lockfile::default();
        lockfile
            .dependencies
            .push(konvoy_config::lockfile::DependencyLock {
                name: "kotlin-serialization".to_owned(),
                source: konvoy_config::lockfile::DepSource::Path {
                    path: "../serial".to_owned(),
                },
                source_hash: "abcdef".to_owned(),
            });
        // Same name as a dep but should not be found as a plugin.
        let hash = find_lockfile_hash(&lockfile, "kotlin-serialization");
        assert!(
            hash.is_none(),
            "plugin lookup should not match dependency entries"
        );
    }

    // -- Helpers ---------------------------------------------------------------

    fn default_package() -> konvoy_config::manifest::Package {
        konvoy_config::manifest::Package {
            name: "test-project".to_owned(),
            kind: konvoy_config::manifest::PackageKind::Bin,
            version: None,
            entrypoint: "src/main.kt".to_owned(),
        }
    }

    fn default_toolchain() -> konvoy_config::manifest::Toolchain {
        konvoy_config::manifest::Toolchain {
            kotlin: "2.1.0".to_owned(),
            detekt: None,
        }
    }

    fn make_manifest_with_plugin(name: &str, maven: &str, version: &str) -> Manifest {
        let mut plugins = BTreeMap::new();
        plugins.insert(
            name.to_owned(),
            konvoy_config::manifest::DependencySpec {
                path: None,
                maven: Some(maven.to_owned()),
                version: Some(version.to_owned()),
            },
        );
        Manifest {
            package: default_package(),
            toolchain: default_toolchain(),
            dependencies: BTreeMap::new(),
            plugins,
        }
    }
}
