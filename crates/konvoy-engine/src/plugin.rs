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
pub(crate) fn maven_cache_root() -> Result<PathBuf, EngineError> {
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
            crate::common::split_maven_coordinate(maven).map_err(|_| {
                EngineError::InvalidPluginConfig {
                    name: plugin_name.clone(),
                    reason: format!("invalid maven coordinate `{maven}`"),
                }
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
    crate::error::map_artifact_download_err(
        name,
        e,
        |name, message| EngineError::PluginDownload { name, message },
        |name, expected, actual| EngineError::PluginHashMismatch {
            name,
            expected,
            actual,
        },
    )
}

/// Ensure all plugin artifacts are downloaded, hash-verified, and return results.
///
/// Each artifact is gated through the shared `--locked` / `--offline` policy:
/// under `--locked` a pinned-but-absent plugin is downloaded (only a missing pin
/// is drift); under `--offline` an absent plugin is a hard error.
///
/// # Errors
/// Returns [`EngineError::LockfileUpdateRequired`] when a plugin has no lockfile
/// pin under `--locked`, [`EngineError::PluginOffline`] when a pinned plugin is
/// absent under `--offline`, or a download/hash error otherwise.
pub fn ensure_plugin_artifacts(
    artifacts: &[ResolvedPluginArtifact],
    lockfile: &Lockfile,
    resolver: crate::common::ArtifactResolver<'_>,
    lockfiles: crate::common::LockfileManager,
) -> Result<Vec<PluginArtifactResult>, EngineError> {
    use rayon::prelude::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};

    // Stat each artifact's cache path once; the gate precheck, the label list,
    // and the bar alignment below all share this snapshot — consistent
    // decisions and no repeated I/O.
    let present: Vec<bool> = artifacts.iter().map(|a| a.cache_path.exists()).collect();

    // Fail fast before spawning any downloads: ask the command resolver to
    // resolve each plugin artifact against the current lockfile/cache state.
    for (artifact, is_present) in artifacts.iter().zip(present.iter().copied()) {
        let has_pin = find_lockfile_hash(lockfile, &artifact.plugin_name).is_some();
        crate::common::resolve_managed_artifact(lockfiles, resolver, has_pin, is_present, || {
            EngineError::PluginOffline {
                name: artifact.plugin_name.clone(),
            }
        })?;
    }

    if artifacts.is_empty() {
        return Ok(Vec::new());
    }

    // Only allocate bars for artifacts that actually need a network fetch.
    // Cached items (cache_path already exists) re-verify their hash via a
    // hidden bar so the user sees no UI flash when nothing's being
    // downloaded.
    let download_labels: Vec<String> = artifacts
        .iter()
        .zip(present.iter().copied())
        .filter(|&(_, is_present)| !is_present)
        .map(|(a, _)| format!("{} {}", a.plugin_name, a.maven_coord.version))
        .collect();
    let any_downloads = !download_labels.is_empty();
    let bars: Vec<konvoy_util::progress::DownloadBar> = if any_downloads {
        konvoy_util::progress::pre_allocate_bars(download_labels).1
    } else {
        Vec::new()
    };
    let mut bar_iter = bars.iter();
    let aligned_bars: Vec<Option<&konvoy_util::progress::DownloadBar>> = present
        .iter()
        .map(|&is_present| if is_present { None } else { bar_iter.next() })
        .collect();

    let results: Vec<Result<PluginArtifactResult, EngineError>> = artifacts
        .par_iter()
        .zip(aligned_bars.par_iter())
        .map(|(artifact, maybe_bar)| {
            let expected_hash = find_lockfile_hash(lockfile, &artifact.plugin_name);
            let util_result = resolver
                .fetch_artifact(
                    &artifact.url,
                    &artifact.cache_path,
                    expected_hash,
                    &artifact.plugin_name,
                    *maybe_bar,
                )
                .map_err(|e| map_download_err(&artifact.plugin_name, e))?;

            Ok(PluginArtifactResult {
                plugin_name: artifact.plugin_name.clone(),
                path: util_result.path,
                sha256: util_result.sha256,
                url: artifact.url.clone(),
                freshly_downloaded: util_result.freshly_downloaded,
                maven: artifact.maven_coord.group_artifact(),
                version: artifact.maven_coord.version.clone(),
            })
        })
        .collect();

    if any_downloads {
        eprintln!();
    }

    results.into_iter().collect()
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
            codegen: Default::default(),
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
            codegen: Default::default(),
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
            codegen: Default::default(),
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
            codegen: Default::default(),
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
            codegen: Default::default(),
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
            codegen: Default::default(),
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
            codegen: Default::default(),
            dependencies: BTreeMap::new(),
            plugins,
        }
    }

    #[test]
    fn ensure_plugin_artifacts_locked_mode_requires_hash() {
        // In --locked mode, a plugin without a lockfile hash should fail.
        let manifest = make_manifest_with_plugin(
            "kotlin-serialization",
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin",
            "2.1.0",
        );
        let artifacts = resolve_plugin_artifacts(&manifest).unwrap();
        let lockfile = Lockfile::default(); // no plugin hashes

        let result = ensure_plugin_artifacts(
            &artifacts,
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(false)),
            crate::common::LockfileManager::new(true),
        );
        assert!(
            result.is_err(),
            "expected error in locked mode without hash"
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("lockfile is out of date"), "error was: {err}");
    }

    #[test]
    fn ensure_plugin_artifacts_empty_input_returns_empty() {
        let lockfile = Lockfile::default();
        let result = ensure_plugin_artifacts(
            &[],
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(false)),
            crate::common::LockfileManager::new(false),
        )
        .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn ensure_plugin_artifacts_offline_absent_errors() {
        // --offline + a pinned-but-absent plugin: hard error (PluginOffline),
        // and the unreachable URL is never contacted (the precheck fires first).
        let tmp = tempfile::tempdir().unwrap();
        let lockfile = Lockfile {
            plugins: vec![PluginLock {
                name: "ser".to_owned(),
                maven: "org.example:ser".to_owned(),
                version: "1.0.0".to_owned(),
                sha256: "0".repeat(64),
                url: "http://example.com".to_owned(),
            }],
            ..Lockfile::default()
        };
        let artifacts = vec![ResolvedPluginArtifact {
            plugin_name: "ser".to_owned(),
            maven_coord: MavenCoordinate::new("org.example", "ser", "1.0.0"),
            url: "http://127.0.0.1:1/ser.jar".to_owned(),
            cache_path: tmp.path().join("ser.jar"), // does not exist
        }];

        let result = ensure_plugin_artifacts(
            &artifacts,
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(true)),
            crate::common::LockfileManager::new(false),
        );
        match result {
            Err(EngineError::PluginOffline { name }) => assert_eq!(name, "ser"),
            other => panic!("expected PluginOffline, got {other:?}"),
        }
    }

    #[test]
    fn ensure_plugin_artifacts_offline_present_ok() {
        // --offline + a present, hash-matching plugin: re-verify the cached copy
        // with no network, returning it successfully.
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("plugin.jar");
        let content = b"present plugin content";
        std::fs::write(&cache_path, content).unwrap();
        let expected_hash = konvoy_util::hash::sha256_bytes(content);

        let lockfile = Lockfile {
            plugins: vec![PluginLock {
                name: "present".to_owned(),
                maven: "org.example:present".to_owned(),
                version: "1.0.0".to_owned(),
                sha256: expected_hash.clone(),
                url: "http://example.com".to_owned(),
            }],
            ..Lockfile::default()
        };
        let artifacts = vec![ResolvedPluginArtifact {
            plugin_name: "present".to_owned(),
            maven_coord: MavenCoordinate::new("org.example", "present", "1.0.0"),
            url: "http://127.0.0.1:1/present.jar".to_owned(), // unused
            cache_path: cache_path.clone(),
        }];

        let result = ensure_plugin_artifacts(
            &artifacts,
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(true)),
            crate::common::LockfileManager::new(false),
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].sha256, expected_hash);
        assert!(!result[0].freshly_downloaded);
    }

    #[test]
    fn ensure_plugin_artifacts_locked_pinned_absent_proceeds_to_download() {
        // --locked + a pinned-but-absent plugin must PROCEED to download (the
        // unified behavior), not fail-fast. The URL is unreachable, so we expect
        // a download error — NOT LockfileUpdateRequired (which would mean the
        // gate wrongly treated a pinned artifact as drift).
        let tmp = tempfile::tempdir().unwrap();
        let lockfile = Lockfile {
            plugins: vec![PluginLock {
                name: "pinned".to_owned(),
                maven: "org.example:pinned".to_owned(),
                version: "1.0.0".to_owned(),
                sha256: "0".repeat(64),
                url: "http://example.com".to_owned(),
            }],
            ..Lockfile::default()
        };
        let artifacts = vec![ResolvedPluginArtifact {
            plugin_name: "pinned".to_owned(),
            maven_coord: MavenCoordinate::new("org.example", "pinned", "1.0.0"),
            url: "http://127.0.0.1:1/pinned.jar".to_owned(),
            cache_path: tmp.path().join("pinned.jar"), // absent → must download
        }];

        let result = ensure_plugin_artifacts(
            &artifacts,
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(false)),
            crate::common::LockfileManager::new(true),
        );
        assert!(
            result.is_err(),
            "expected a download error, got: {result:?}"
        );
        assert!(
            !matches!(result, Err(EngineError::LockfileUpdateRequired)),
            "a pinned plugin under --locked must download, not report drift"
        );
    }

    #[test]
    fn ensure_plugin_artifacts_locked_failfast_before_downloads() {
        // Two artifacts, only one has a hash in the lockfile. Both URLs point
        // at an unreachable address. If the precheck fires first (as intended),
        // we get LockfileUpdateRequired — not a download failure — proving the
        // parallel downloads never start.
        let tmp = tempfile::tempdir().unwrap();
        let lockfile = Lockfile {
            plugins: vec![PluginLock {
                name: "has-lock".to_owned(),
                maven: "org.example:lib1".to_owned(),
                version: "1.0.0".to_owned(),
                sha256: "0".repeat(64),
                url: "http://example.com".to_owned(),
            }],
            ..Lockfile::default()
        };
        let artifacts = vec![
            ResolvedPluginArtifact {
                plugin_name: "has-lock".to_owned(),
                maven_coord: MavenCoordinate::new("org.example", "lib1", "1.0.0"),
                url: "http://127.0.0.1:1/lib1.jar".to_owned(),
                cache_path: tmp.path().join("lib1.jar"),
            },
            ResolvedPluginArtifact {
                plugin_name: "no-lock".to_owned(),
                maven_coord: MavenCoordinate::new("org.example", "lib2", "1.0.0"),
                url: "http://127.0.0.1:1/lib2.jar".to_owned(),
                cache_path: tmp.path().join("lib2.jar"),
            },
        ];

        let result = ensure_plugin_artifacts(
            &artifacts,
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(false)),
            crate::common::LockfileManager::new(true),
        );
        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn ensure_plugin_artifacts_reuses_existing_cached_file() {
        // Pre-seed a "downloaded" plugin JAR with a known hash matching the
        // lockfile entry. The function should return it without attempting any
        // network I/O, exercising the par_iter happy path.
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("plugin.jar");
        let content = b"plugin content";
        std::fs::write(&cache_path, content).unwrap();
        let expected_hash = konvoy_util::hash::sha256_bytes(content);

        let lockfile = Lockfile {
            plugins: vec![PluginLock {
                name: "test-plugin".to_owned(),
                maven: "org.example:plugin".to_owned(),
                version: "1.0.0".to_owned(),
                sha256: expected_hash.clone(),
                url: "http://example.com".to_owned(),
            }],
            ..Lockfile::default()
        };
        let artifacts = vec![ResolvedPluginArtifact {
            plugin_name: "test-plugin".to_owned(),
            maven_coord: MavenCoordinate::new("org.example", "plugin", "1.0.0"),
            url: "http://127.0.0.1:1/plugin.jar".to_owned(), // unused
            cache_path: cache_path.clone(),
        }];

        let result = ensure_plugin_artifacts(
            &artifacts,
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(false)),
            crate::common::LockfileManager::new(false),
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        let r = &result[0];
        assert_eq!(r.plugin_name, "test-plugin");
        assert_eq!(r.sha256, expected_hash);
        assert!(!r.freshly_downloaded);
        assert_eq!(r.path, cache_path);
        assert_eq!(r.maven, "org.example:plugin");
        assert_eq!(r.version, "1.0.0");
    }

    #[test]
    fn ensure_plugin_artifacts_unlocked_download_failure_maps_to_engine_error() {
        // Unlocked mode + unreachable URL + missing cache file → download fails.
        // Exercises the map_download_err path in the par_iter closure.
        let tmp = tempfile::tempdir().unwrap();
        let artifact = ResolvedPluginArtifact {
            plugin_name: "unreachable-plugin".to_owned(),
            maven_coord: MavenCoordinate::new("org.example", "lib", "1.0.0"),
            url: "http://127.0.0.1:1/lib.jar".to_owned(),
            cache_path: tmp.path().join("lib.jar"),
        };
        let lockfile = Lockfile::default();
        let result = ensure_plugin_artifacts(
            &[artifact],
            &lockfile,
            crate::common::ArtifactResolver::new(&konvoy_util::net::NetworkClient::new(false)),
            crate::common::LockfileManager::new(false),
        );
        assert!(result.is_err());
    }
}
