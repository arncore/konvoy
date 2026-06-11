//! Gradle Module Metadata parser for Kotlin/Native artifacts.
//!
//! Parses `.module` JSON files published alongside Maven artifacts. These files
//! contain richer metadata than POM files, including cinterop klib artifacts
//! that are not discoverable via POM.
//!
//! See: <https://docs.gradle.org/current/userguide/publishing_gradle_module_metadata.html>

use serde::Deserialize;

use crate::error::UtilError;
use crate::metadata::{ArtifactMetadata, MetadataDep, MetadataFile};

// ---------------------------------------------------------------------------
// Serde structs — only the fields we need
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GradleModule {
    #[serde(default)]
    variants: Vec<GradleVariant>,
}

#[derive(Debug, Deserialize)]
struct GradleVariant {
    name: String,
    #[serde(default)]
    dependencies: Option<Vec<GradleDep>>,
    #[serde(default)]
    files: Option<Vec<GradleFile>>,
}

#[derive(Debug, Deserialize)]
struct GradleDep {
    group: String,
    module: String,
    #[serde(default)]
    version: Option<GradleVersion>,
}

#[derive(Debug, Deserialize)]
struct GradleVersion {
    #[serde(default)]
    requires: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GradleFile {
    name: String,
    url: String,
    #[serde(default)]
    sha256: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a Gradle Module Metadata JSON string into [`ArtifactMetadata`].
///
/// Looks for a variant whose name ends with `ApiElements-published`, which
/// contains the compile-scope dependencies and all published klib files
/// (including cinterop klibs).
///
/// # Errors
///
/// Returns `UtilError::ModuleMetadataParse` if the JSON is malformed or
/// no suitable variant is found.
pub fn parse_module_metadata(json: &str) -> Result<ArtifactMetadata, UtilError> {
    let module: GradleModule =
        serde_json::from_str(json).map_err(|e| UtilError::ModuleMetadataParse {
            reason: e.to_string(),
        })?;

    // Find the variant ending in "ApiElements-published".
    let variant = module
        .variants
        .iter()
        .find(|v| v.name.ends_with("ApiElements-published"))
        .ok_or_else(|| UtilError::ModuleMetadataParse {
            reason: "no variant ending with 'ApiElements-published' found".to_owned(),
        })?;

    let dependencies = variant
        .dependencies
        .as_ref()
        .map(|deps| {
            deps.iter()
                .filter_map(|d| {
                    let version = d.version.as_ref().and_then(|v| v.requires.clone())?;
                    Some(MetadataDep {
                        group_id: d.group.clone(),
                        artifact_id: d.module.clone(),
                        version,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let files = variant
        .files
        .as_ref()
        .map(|fs| {
            fs.iter()
                .map(|f| MetadataFile {
                    name: f.name.clone(),
                    url: f.url.clone(),
                    sha256: f.sha256.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ArtifactMetadata {
        dependencies,
        files,
    })
}

/// Build the Maven Central URL for a Gradle Module Metadata file.
///
/// The URL pattern is:
/// `{MAVEN_CENTRAL}/{group_path}/{artifact_id}/{version}/{artifact_id}-{version}.module`
pub fn module_metadata_url(group_id: &str, artifact_id: &str, version: &str) -> String {
    crate::maven::maven_artifact_url(group_id, artifact_id, version, "module")
}

/// Build the on-disk cache path for a Gradle Module Metadata file.
///
/// Layout matches the POM cache scheme:
/// `~/.konvoy/cache/pom/<group_path>/<artifact_id>-<version>.module`.
/// (Same root directory; a different extension keeps POM and module entries
/// from colliding.) Delegates to [`crate::pom::metadata_cache_path`] so both
/// formats share validation and layout-versioning logic.
fn module_cache_path(
    group_id: &str,
    artifact_id: &str,
    version: &str,
) -> Result<std::path::PathBuf, UtilError> {
    crate::pom::metadata_cache_path(group_id, artifact_id, version, "module")
}

/// Build the 404-sentinel path that records "this artifact has no .module file".
///
/// Appends `.404` to the body filename. Fails loudly if the body path has no
/// filename component — the caller (this module) always constructs the body
/// via [`module_cache_path`], which guarantees a filename, so this is an
/// invariant check rather than a user-facing error.
fn module_404_sentinel_path(body: &std::path::Path) -> Result<std::path::PathBuf, UtilError> {
    let file_name = body
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| UtilError::Io {
            path: body.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cache path has no filename",
            ),
        })?;
    let mut name = file_name.to_owned();
    name.push_str(".404");
    Ok(body.with_file_name(name))
}

/// Fetch a Gradle Module Metadata file from Maven Central.
///
/// Returns `Ok(Some(json))` if the file exists, `Ok(None)` if the server
/// returns a 404 (meaning the artifact does not publish `.module` files),
/// or an error for other HTTP failures.
///
/// Uses a disk cache at `~/.konvoy/cache/pom/<group_path>/<artifact_id>-<version>.module`.
/// 404 responses are recorded by writing an empty sentinel file at
/// `<path>.404`, so re-runs do not hit the network for known-missing
/// artifacts. Cache writes are best-effort: failures are logged but the
/// fetched content is still returned.
///
/// # Errors
///
/// Returns `UtilError::Download` if the HTTP request fails with a non-404
/// error or the response body cannot be read. Returns `UtilError::InvalidVersion`
/// if any of `group_id`/`artifact_id`/`version` fail validation.
pub fn fetch_module_metadata(
    net: &crate::net::NetworkClient,
    group_id: &str,
    artifact_id: &str,
    version: &str,
) -> Result<Option<String>, UtilError> {
    // 1. Try the disk cache. Check the body file first, then the 404 sentinel.
    // Body file wins over the 404 sentinel if both exist. This recovers
    // gracefully from the case where Maven Central publishes a previously
    // missing artifact between two `konvoy update` runs. No network is
    // involved here, so cached metadata resolves offline too.
    let cache_path = module_cache_path(group_id, artifact_id, version)?;
    if cache_path.exists() {
        if let Ok(body) = std::fs::read_to_string(&cache_path) {
            return Ok(Some(body));
        }
        // Cache read failed (truncated, permissions, etc.); fall through to
        // fetch from network rather than block builds.
    }
    let sentinel_path = module_404_sentinel_path(&cache_path)?;
    if sentinel_path.exists() {
        return Ok(None);
    }

    // 2. Cache miss — fetch from Maven Central.
    let url = module_metadata_url(group_id, artifact_id, version);

    match net.get(&url, 60) {
        Ok(response) => {
            let body = response
                .into_body()
                .read_to_string()
                .map_err(|e| UtilError::Download {
                    message: format!(
                        "failed to read module metadata response body from {url}: {e}"
                    ),
                })?;

            if let Err(e) = crate::pom::write_cache_atomic(&cache_path, body.as_bytes()) {
                eprintln!(
                    "warning: failed to cache module metadata at {}: {e}",
                    cache_path.display()
                );
            }

            Ok(Some(body))
        }
        Err(crate::net::RequestError::Status { code: 404, .. }) => {
            // Persist the 404 so we don't re-hit the network on every run.
            if let Err(e) = crate::pom::write_cache_atomic(&sentinel_path, &[]) {
                eprintln!(
                    "warning: failed to cache module 404 sentinel at {}: {e}",
                    sentinel_path.display()
                );
            }
            Ok(None)
        }
        Err(crate::net::RequestError::Offline) => Err(UtilError::Offline { url }),
        Err(crate::net::RequestError::Status { message, .. })
        | Err(crate::net::RequestError::Transport { message }) => Err(UtilError::Download {
            message: format!("failed to fetch module metadata from {url}: {message}"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// An online client for tests exercising fetch paths.
    fn online() -> crate::net::NetworkClient {
        crate::net::NetworkClient::new(false)
    }

    /// Real-world `.module` JSON for kotlinx-coroutines-core-linuxx64 (trimmed).
    const COROUTINES_MODULE: &str = r#"{
  "formatVersion": "1.1",
  "component": {
    "group": "org.jetbrains.kotlinx",
    "module": "kotlinx-coroutines-core-linuxx64",
    "version": "1.8.0"
  },
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "attributes": {
        "artifactType": "org.jetbrains.kotlin.klib",
        "org.gradle.category": "library",
        "org.gradle.usage": "kotlin-api",
        "org.jetbrains.kotlin.native.target": "linux_x64",
        "org.jetbrains.kotlin.platform.type": "native"
      },
      "dependencies": [
        {
          "group": "org.jetbrains.kotlinx",
          "module": "atomicfu",
          "version": { "requires": "0.23.1" }
        },
        {
          "group": "org.jetbrains.kotlin",
          "module": "kotlin-stdlib",
          "version": { "requires": "1.9.21" }
        }
      ],
      "files": [
        {
          "name": "kotlinx-coroutines-core.klib",
          "url": "kotlinx-coroutines-core-linuxx64-1.8.0.klib",
          "size": 828734,
          "sha256": "3c84b014dde1626094b47ad1fb95a3e01886846c33bda7526e6ba9979088305a"
        }
      ]
    }
  ]
}"#;

    /// `.module` JSON for atomicfu-linuxx64 with cinterop artifact.
    const ATOMICFU_MODULE: &str = r#"{
  "formatVersion": "1.1",
  "component": {
    "group": "org.jetbrains.kotlinx",
    "module": "atomicfu-linuxx64",
    "version": "0.23.1"
  },
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "dependencies": [
        {
          "group": "org.jetbrains.kotlin",
          "module": "kotlin-stdlib",
          "version": { "requires": "1.9.21" }
        }
      ],
      "files": [
        {
          "name": "atomicfu.klib",
          "url": "atomicfu-linuxx64-0.23.1.klib",
          "sha256": "9cc63f4e87d66dad000111222333444555666777888999aaabbbcccdddeeefff"
        },
        {
          "name": "atomicfu-cinterop-interop.klib",
          "url": "atomicfu-linuxx64-0.23.1-cinterop-interop.klib",
          "sha256": "c401e39db4e7c6a1000111222333444555666777888999aaabbbcccdddeeefff"
        }
      ]
    }
  ]
}"#;

    #[test]
    fn parse_coroutines_module_extracts_deps_and_files() {
        let metadata = parse_module_metadata(COROUTINES_MODULE).unwrap();

        assert_eq!(metadata.dependencies.len(), 2);

        let dep0 = metadata.dependencies.first().unwrap();
        assert_eq!(dep0.group_id, "org.jetbrains.kotlinx");
        assert_eq!(dep0.artifact_id, "atomicfu");
        assert_eq!(dep0.version, "0.23.1");

        let dep1 = metadata.dependencies.get(1).unwrap();
        assert_eq!(dep1.group_id, "org.jetbrains.kotlin");
        assert_eq!(dep1.artifact_id, "kotlin-stdlib");
        assert_eq!(dep1.version, "1.9.21");

        assert_eq!(metadata.files.len(), 1);
        let file0 = metadata.files.first().unwrap();
        assert_eq!(file0.name, "kotlinx-coroutines-core.klib");
        assert_eq!(file0.url, "kotlinx-coroutines-core-linuxx64-1.8.0.klib");
        assert_eq!(
            file0.sha256.as_deref(),
            Some("3c84b014dde1626094b47ad1fb95a3e01886846c33bda7526e6ba9979088305a")
        );
    }

    #[test]
    fn parse_atomicfu_module_extracts_cinterop_files() {
        let metadata = parse_module_metadata(ATOMICFU_MODULE).unwrap();

        assert_eq!(metadata.files.len(), 2);

        let main_klib = metadata.files.first().unwrap();
        assert_eq!(main_klib.name, "atomicfu.klib");

        let cinterop_klib = metadata.files.get(1).unwrap();
        assert_eq!(cinterop_klib.name, "atomicfu-cinterop-interop.klib");
        assert_eq!(
            cinterop_klib.url,
            "atomicfu-linuxx64-0.23.1-cinterop-interop.klib"
        );
        assert!(cinterop_klib.sha256.is_some());
    }

    #[test]
    fn parse_module_no_cinterop_returns_single_file() {
        let metadata = parse_module_metadata(COROUTINES_MODULE).unwrap();
        // coroutines has no cinterop artifacts.
        assert_eq!(metadata.files.len(), 1);
        assert!(!metadata.files.iter().any(|f| f.name.contains("cinterop")));
    }

    #[test]
    fn parse_module_missing_api_elements_variant_errors() {
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "sourcesElements",
      "files": []
    }
  ]
}"#;
        let err = parse_module_metadata(json).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ApiElements-published"), "error was: {msg}");
    }

    #[test]
    fn parse_module_malformed_json_errors() {
        let err = parse_module_metadata("this is not json").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cannot parse module metadata"),
            "error was: {msg}"
        );
    }

    #[test]
    fn parse_module_empty_variants_errors() {
        let json = r#"{ "formatVersion": "1.1", "variants": [] }"#;
        let err = parse_module_metadata(json).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ApiElements-published"), "error was: {msg}");
    }

    #[test]
    fn parse_module_deps_without_version_are_skipped() {
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "dependencies": [
        {
          "group": "org.example",
          "module": "no-version"
        },
        {
          "group": "org.example",
          "module": "with-version",
          "version": { "requires": "1.0" }
        }
      ],
      "files": []
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        assert_eq!(metadata.dependencies.len(), 1);
        assert_eq!(
            metadata.dependencies.first().unwrap().artifact_id,
            "with-version"
        );
    }

    #[test]
    fn module_metadata_url_format() {
        let url = module_metadata_url(
            "org.jetbrains.kotlinx",
            "kotlinx-coroutines-core-linuxx64",
            "1.8.0",
        );
        assert_eq!(
            url,
            "https://repo1.maven.org/maven2/org/jetbrains/kotlinx/kotlinx-coroutines-core-linuxx64/1.8.0/kotlinx-coroutines-core-linuxx64-1.8.0.module"
        );
    }

    #[test]
    fn module_metadata_url_single_segment_group() {
        let url = module_metadata_url("com", "mylib", "1.0.0");
        assert_eq!(
            url,
            "https://repo1.maven.org/maven2/com/mylib/1.0.0/mylib-1.0.0.module"
        );
    }

    #[test]
    fn parse_module_variant_with_null_files_returns_empty_files() {
        // A variant can have `files: null` (serialized as missing).
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "dependencies": [],
      "files": null
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        assert!(metadata.files.is_empty());
        assert!(metadata.dependencies.is_empty());
    }

    #[test]
    fn parse_module_variant_with_no_deps_key_returns_empty_deps() {
        // A variant may omit the `dependencies` key entirely.
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "files": [
        {
          "name": "lib.klib",
          "url": "lib-1.0.klib"
        }
      ]
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        assert!(metadata.dependencies.is_empty());
        assert_eq!(metadata.files.len(), 1);
    }

    #[test]
    fn parse_module_file_without_sha256() {
        // Files may omit sha256 — it should be `None`.
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "files": [
        {
          "name": "lib.klib",
          "url": "lib-1.0.klib"
        }
      ]
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        let file = metadata.files.first().unwrap();
        assert!(file.sha256.is_none());
    }

    #[test]
    fn parse_module_multiple_cinterop_files() {
        // An artifact may publish multiple cinterop klibs alongside the main klib.
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "files": [
        { "name": "lib.klib", "url": "lib-linuxx64-1.0.klib", "sha256": "aaa" },
        { "name": "lib-cinterop-foo.klib", "url": "lib-linuxx64-1.0-cinterop-foo.klib", "sha256": "bbb" },
        { "name": "lib-cinterop-bar.klib", "url": "lib-linuxx64-1.0-cinterop-bar.klib", "sha256": "ccc" }
      ]
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        assert_eq!(metadata.files.len(), 3);
        let cinterop_files: Vec<_> = metadata
            .files
            .iter()
            .filter(|f| f.name.contains("cinterop"))
            .collect();
        assert_eq!(cinterop_files.len(), 2);
    }

    #[test]
    fn parse_module_picks_first_matching_api_elements_variant() {
        // If there are multiple variants ending in ApiElements-published
        // (unusual but possible), the first one is used.
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "dependencies": [
        { "group": "org.example", "module": "first", "version": { "requires": "1.0" } }
      ],
      "files": []
    },
    {
      "name": "macosArm64ApiElements-published",
      "dependencies": [
        { "group": "org.example", "module": "second", "version": { "requires": "2.0" } }
      ],
      "files": []
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        assert_eq!(metadata.dependencies.len(), 1);
        assert_eq!(metadata.dependencies.first().unwrap().artifact_id, "first");
    }

    #[test]
    fn module_cache_path_uses_module_extension() {
        let path = module_cache_path("com.example", "lib", "1.0.0").unwrap();
        let suffix: std::path::PathBuf = ["cache", "pom", "com", "example", "lib-1.0.0.module"]
            .iter()
            .collect();
        assert!(
            path.ends_with(&suffix),
            "expected {} to end with {}",
            path.display(),
            suffix.display()
        );
    }

    #[test]
    fn module_cache_path_rejects_path_traversal() {
        assert!(module_cache_path("..", "lib", "1.0.0").is_err());
        assert!(module_cache_path("com.ex", "..", "1.0.0").is_err());
        assert!(module_cache_path("com.ex", "lib", "..").is_err());
    }

    #[test]
    fn module_404_sentinel_path_appends_suffix() {
        let body = std::path::PathBuf::from("/tmp/cache/pom/com/example/lib-1.0.0.module");
        let sentinel = module_404_sentinel_path(&body).unwrap();
        assert_eq!(
            sentinel,
            std::path::PathBuf::from("/tmp/cache/pom/com/example/lib-1.0.0.module.404")
        );
    }

    #[test]
    fn module_404_sentinel_path_rejects_path_without_filename() {
        // An empty path has no file_name — the invariant check should fire.
        let body = std::path::PathBuf::from("");
        let result = module_404_sentinel_path(&body);
        assert!(result.is_err());
    }

    #[test]
    fn fetch_module_metadata_rejects_path_traversal_in_group_id() {
        let result = fetch_module_metadata(&online(), "../etc/passwd", "lib", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn fetch_module_metadata_rejects_invalid_version() {
        let err = fetch_module_metadata(&online(), "com.example", "lib", "1.0/0").unwrap_err();
        assert!(
            matches!(err, UtilError::InvalidVersion { .. }),
            "expected InvalidVersion, got: {err:?}"
        );
    }

    #[test]
    fn fetch_module_metadata_nonexistent_returns_none() {
        // Use a non-existent artifact to test 404 handling.
        let result = fetch_module_metadata(
            &online(),
            "com.nonexistent.fake",
            "no-such-artifact",
            "0.0.0",
        );
        // This may either return None (404) or Err (connection refused).
        // Both are acceptable for a non-existent artifact — we just verify no panic.
        match result {
            Ok(None) => {} // Expected: 404
            Err(_) => {}   // Also acceptable: network error
            Ok(Some(_)) => panic!("should not find a nonexistent artifact"),
        }
    }

    #[test]
    fn parse_module_empty_json_object_errors() {
        // A JSON object with no `variants` key at all should still parse
        // (serde default gives empty vec), but fail because no matching variant.
        let json = r#"{}"#;
        let err = parse_module_metadata(json).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ApiElements-published"), "error was: {msg}");
    }

    #[test]
    fn parse_module_dep_with_version_key_but_no_requires_is_skipped() {
        // A dependency with a `version` object but no `requires` field inside
        // it should be skipped (the filter_map returns None).
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "dependencies": [
        {
          "group": "org.example",
          "module": "preferred-only",
          "version": { "prefers": "1.0" }
        },
        {
          "group": "org.example",
          "module": "has-requires",
          "version": { "requires": "2.0" }
        }
      ],
      "files": []
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        // Only the dep with `requires` should be included.
        assert_eq!(metadata.dependencies.len(), 1);
        assert_eq!(
            metadata.dependencies.first().unwrap().artifact_id,
            "has-requires"
        );
    }

    #[test]
    fn parse_module_dep_with_null_version_requires_is_skipped() {
        // A dependency where `version.requires` is explicitly null should be
        // skipped, not panic.
        let json = r#"{
  "formatVersion": "1.1",
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "dependencies": [
        {
          "group": "org.example",
          "module": "null-requires",
          "version": { "requires": null }
        }
      ],
      "files": []
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        assert!(
            metadata.dependencies.is_empty(),
            "dep with null requires should be skipped"
        );
    }

    #[test]
    fn parse_module_extra_fields_are_ignored() {
        // Unknown fields in the JSON (e.g. `capabilities`, `attributes`)
        // should be silently ignored, not cause parse errors.
        let json = r#"{
  "formatVersion": "1.1",
  "createdBy": { "gradle": { "version": "8.2" } },
  "variants": [
    {
      "name": "linuxX64ApiElements-published",
      "attributes": { "org.gradle.usage": "kotlin-api" },
      "capabilities": [{ "group": "org.example", "name": "lib", "version": "1.0" }],
      "dependencies": [
        {
          "group": "org.example",
          "module": "dep",
          "version": { "requires": "1.0" },
          "excludes": [{ "group": "*", "module": "excluded" }]
        }
      ],
      "files": [
        {
          "name": "lib.klib",
          "url": "lib-1.0.klib",
          "size": 12345,
          "sha512": "ignored-hash",
          "md5": "also-ignored"
        }
      ]
    }
  ]
}"#;
        let metadata = parse_module_metadata(json).unwrap();
        assert_eq!(metadata.dependencies.len(), 1);
        assert_eq!(metadata.files.len(), 1);
        // sha256 is None because it was not present (sha512/md5 are different fields).
        assert!(metadata.files.first().unwrap().sha256.is_none());
    }

    // -----------------------------------------------------------------
    // fetch_module_metadata — disk-cache behavior (HOME-override pattern)
    // -----------------------------------------------------------------

    /// Run `f` with HOME pointing at a tempdir, serialized against other
    /// HOME-mutating tests. Restores HOME before assertions can panic.
    fn with_fake_home<F, R>(f: F) -> R
    where
        F: FnOnce(&std::path::Path) -> R,
    {
        let _guard = crate::test_util::ENV_LOCK.lock().unwrap();

        let saved_home = std::env::var("HOME").ok();
        let saved_profile = std::env::var("USERPROFILE").ok();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("USERPROFILE");

        // Catch any panic so we always restore the env.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(tmp.path())));

        if let Some(v) = saved_home {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(v) = saved_profile {
            std::env::set_var("USERPROFILE", v);
        }

        match result {
            Ok(r) => r,
            Err(e) => std::panic::resume_unwind(e),
        }
    }

    #[test]
    fn fetch_module_metadata_cache_hit_body_returns_some_without_network() {
        // Pre-populate the body file; the function must return `Some(body)`
        // without any HTTP I/O.
        with_fake_home(|_home| {
            let group = "com.example";
            let artifact = "cached-mod";
            let version = "1.0.0";
            let body_path = module_cache_path(group, artifact, version).unwrap();
            std::fs::create_dir_all(body_path.parent().unwrap()).unwrap();
            let expected = r#"{"variants":[{"name":"x"}]}"#;
            std::fs::write(&body_path, expected).unwrap();

            let result = fetch_module_metadata(&online(), group, artifact, version).unwrap();
            assert_eq!(
                result.as_deref(),
                Some(expected),
                "cache hit must return Some(body)"
            );
        });
    }

    #[test]
    fn fetch_module_metadata_cache_hit_404_sentinel_returns_none() {
        // Pre-populate ONLY the 404 sentinel; the function must return None
        // without any HTTP I/O.
        with_fake_home(|_home| {
            let group = "com.example";
            let artifact = "no-module";
            let version = "1.0.0";
            let body_path = module_cache_path(group, artifact, version).unwrap();
            let sentinel = module_404_sentinel_path(&body_path).unwrap();
            std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
            std::fs::write(&sentinel, b"").unwrap();

            let result = fetch_module_metadata(&online(), group, artifact, version).unwrap();
            assert!(
                result.is_none(),
                "sentinel hit must return None (no .module published)"
            );
        });
    }

    #[test]
    fn fetch_module_metadata_body_wins_over_404_sentinel() {
        // If both the body and the sentinel exist (Maven Central started
        // publishing a previously-missing artifact), the body must win.
        // This is the recovery contract documented at the call site.
        with_fake_home(|_home| {
            let group = "com.example";
            let artifact = "now-published";
            let version = "1.0.0";
            let body_path = module_cache_path(group, artifact, version).unwrap();
            let sentinel = module_404_sentinel_path(&body_path).unwrap();
            std::fs::create_dir_all(body_path.parent().unwrap()).unwrap();
            let expected = r#"{"variants":[{"name":"y"}]}"#;
            std::fs::write(&body_path, expected).unwrap();
            std::fs::write(&sentinel, b"").unwrap();

            let result = fetch_module_metadata(&online(), group, artifact, version).unwrap();
            assert_eq!(
                result.as_deref(),
                Some(expected),
                "body file must take precedence over stale 404 sentinel"
            );
        });
    }

    #[test]
    fn fetch_module_metadata_offline_cached_ok_uncached_refuses() {
        // Offline policy: a cached body or 404 sentinel resolves from disk;
        // an uncached coordinate is refused at the wire (UtilError::Offline),
        // NOT silently treated as "no .module published" (Ok(None)).
        with_fake_home(|_home| {
            let offline = crate::net::NetworkClient::new(true);

            let body_path = module_cache_path("com.example", "cached", "1.0.0").unwrap();
            std::fs::create_dir_all(body_path.parent().unwrap()).unwrap();
            std::fs::write(&body_path, r#"{"variants":[]}"#).unwrap();

            let cached = fetch_module_metadata(&offline, "com.example", "cached", "1.0.0");
            assert_eq!(cached.unwrap().as_deref(), Some(r#"{"variants":[]}"#));

            let uncached = fetch_module_metadata(&offline, "com.example", "uncached", "1.0.0");
            assert!(
                matches!(uncached, Err(UtilError::Offline { .. })),
                "uncached .module under offline must refuse, got: {uncached:?}"
            );
        });
    }
}
