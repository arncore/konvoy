//! Gradle Module Metadata parser for Kotlin/Native artifacts.
//!
//! Parses `.module` JSON files published alongside Maven artifacts. These files
//! contain richer metadata than POM files, including cinterop klib artifacts
//! that are not discoverable via POM.
//!
//! See: <https://docs.gradle.org/current/userguide/publishing_gradle_module_metadata.html>

use serde::Deserialize;

use crate::error::UtilError;
use crate::maven::MAVEN_CENTRAL;
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
    let group_path = group_id.replace('.', "/");
    format!("{MAVEN_CENTRAL}/{group_path}/{artifact_id}/{version}/{artifact_id}-{version}.module")
}

/// Fetch a Gradle Module Metadata file from Maven Central.
///
/// Returns `Ok(Some(json))` if the file exists, `Ok(None)` if the server
/// returns a 404 (meaning the artifact does not publish `.module` files),
/// or an error for other HTTP failures.
///
/// # Errors
///
/// Returns `UtilError::Download` if the HTTP request fails with a non-404 error
/// or the response body cannot be read.
pub fn fetch_module_metadata(
    group_id: &str,
    artifact_id: &str,
    version: &str,
) -> Result<Option<String>, UtilError> {
    let url = module_metadata_url(group_id, artifact_id, version);

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .build(),
    );

    match agent.get(&url).call() {
        Ok(response) => {
            let body = response
                .into_body()
                .read_to_string()
                .map_err(|e| UtilError::Download {
                    message: format!(
                        "failed to read module metadata response body from {url}: {e}"
                    ),
                })?;
            Ok(Some(body))
        }
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(UtilError::Download {
            message: format!("failed to fetch module metadata from {url}: {e}"),
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
    fn fetch_module_metadata_nonexistent_returns_none() {
        // Use a non-existent artifact to test 404 handling.
        let result = fetch_module_metadata("com.nonexistent.fake", "no-such-artifact", "0.0.0");
        // This may either return None (404) or Err (connection refused).
        // Both are acceptable for a non-existent artifact — we just verify no panic.
        match result {
            Ok(None) => {} // Expected: 404
            Err(_) => {}   // Also acceptable: network error
            Ok(Some(_)) => panic!("should not find a nonexistent artifact"),
        }
    }
}
