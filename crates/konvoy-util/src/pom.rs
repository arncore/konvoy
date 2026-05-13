//! Minimal POM parser for extracting compile-scope transitive dependencies.
//!
//! Handles:
//! - `<dependencies>` extraction with scope filtering (compile only)
//! - Inline `<parent>` inheritance for `groupId` and `version`
//! - Caller-supplied fallback values for `groupId` and `version` (from `konvoy.toml`)
//! - Property interpolation for `${project.version}` and `${project.groupId}`
//!
//! Rejects with actionable errors:
//! - Version ranges (e.g. `[1.0,2.0)`)
//! - Property placeholders beyond `${project.version}` / `${project.groupId}`

use crate::error::UtilError;
use crate::metadata::{ArtifactMetadata, MetadataDep};

/// A parsed Maven POM, containing identity and compile-scope dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pom {
    /// Maven group identifier, e.g. `"org.jetbrains.kotlinx"`.
    pub group_id: String,
    /// Maven artifact identifier, e.g. `"kotlinx-coroutines-core"`.
    pub artifact_id: String,
    /// Artifact version, e.g. `"1.9.0"`.
    pub version: String,
    /// Compile-scope (non-optional) dependencies.
    pub dependencies: Vec<PomDependency>,
}

/// A single compile-scope dependency extracted from a POM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PomDependency {
    /// Maven group identifier.
    pub group_id: String,
    /// Maven artifact identifier.
    pub artifact_id: String,
    /// Exact version string.
    pub version: String,
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

/// Get the text content of the first direct child element with the given tag name.
fn child_text<'a>(node: &roxmltree::Node<'a, 'a>, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)
        .and_then(|n| n.text())
        .map(|t| t.trim().to_owned())
}

/// Return `true` if `version` looks like a Maven version range.
///
/// Maven version ranges use brackets/parentheses and commas, e.g.
/// `[1.0,2.0)`, `(,1.5]`, `[1.0,)`. Exact versions never contain these.
fn is_version_range(version: &str) -> bool {
    let trimmed = version.trim();
    (trimmed.starts_with('[') || trimmed.starts_with('('))
        && (trimmed.ends_with(']') || trimmed.ends_with(')'))
}

/// Interpolate `${project.version}` and `${project.groupId}` in `value`.
///
/// Any other `${...}` placeholder is rejected with an actionable error.
fn interpolate_property(
    value: &str,
    project_version: &str,
    project_group_id: &str,
) -> Result<String, UtilError> {
    let mut result = value.to_owned();

    // Process all ${...} placeholders.
    while let Some(start) = result.find("${") {
        let Some(rel_end) = result.get(start..).and_then(|s| s.find('}')) else {
            break;
        };
        let end = start + rel_end + 1;
        let Some(placeholder) = result.get(start..end) else {
            break;
        };
        let placeholder_owned = placeholder.to_owned();

        let Some(prop_name) = placeholder_owned
            .strip_prefix("${")
            .and_then(|s| s.strip_suffix('}'))
        else {
            break;
        };

        let replacement = match prop_name {
            "project.version" => project_version,
            "project.groupId" => project_group_id,
            _ => {
                return Err(UtilError::PomUnsupportedProperty {
                    property: placeholder_owned,
                });
            }
        };

        result = format!(
            "{}{}{}",
            result.get(..start).unwrap_or_default(),
            replacement,
            result.get(end..).unwrap_or_default()
        );
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Internal POM identity (used during parsing before full Pom construction)
// ---------------------------------------------------------------------------

/// Identity fields extracted from a `<project>` or `<parent>` element.
struct PomIdentity {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
}

/// Extract identity fields from a `<project>` root node.
fn extract_identity(root: &roxmltree::Node<'_, '_>) -> PomIdentity {
    PomIdentity {
        group_id: child_text(root, "groupId"),
        artifact_id: child_text(root, "artifactId"),
        version: child_text(root, "version"),
    }
}

/// Extract identity from a `<parent>` child element, if present.
fn extract_parent_identity(root: &roxmltree::Node<'_, '_>) -> Option<PomIdentity> {
    let parent_node = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "parent")?;

    Some(PomIdentity {
        group_id: child_text(&parent_node, "groupId"),
        artifact_id: child_text(&parent_node, "artifactId"),
        version: child_text(&parent_node, "version"),
    })
}

// ---------------------------------------------------------------------------
// Dependency extraction
// ---------------------------------------------------------------------------

/// Extract compile-scope, non-optional dependencies from the `<dependencies>` block.
fn extract_dependencies(
    root: &roxmltree::Node<'_, '_>,
    project_version: &str,
    project_group_id: &str,
) -> Result<Vec<PomDependency>, UtilError> {
    let mut deps = Vec::new();

    // Find the <dependencies> element (direct child of <project>).
    let Some(deps_node) = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "dependencies")
    else {
        return Ok(deps);
    };

    for dep_node in deps_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "dependency")
    {
        // Skip optional dependencies.
        if child_text(&dep_node, "optional")
            .as_deref()
            .is_some_and(|v| v == "true")
        {
            continue;
        }

        // Scope filtering: only "compile" (the default when absent).
        let scope = child_text(&dep_node, "scope");
        match scope.as_deref() {
            None | Some("compile") | Some("") => { /* include */ }
            Some(_) => continue, // test, provided, runtime, system
        }

        let group = child_text(&dep_node, "groupId").ok_or_else(|| UtilError::PomParse {
            reason: "dependency is missing <groupId>".to_owned(),
        })?;

        let artifact = child_text(&dep_node, "artifactId").ok_or_else(|| UtilError::PomParse {
            reason: "dependency is missing <artifactId>".to_owned(),
        })?;

        let raw_version = child_text(&dep_node, "version").unwrap_or_default();

        // Interpolate properties.
        let group = interpolate_property(&group, project_version, project_group_id)?;
        let artifact = interpolate_property(&artifact, project_version, project_group_id)?;
        let version = interpolate_property(&raw_version, project_version, project_group_id)?;

        // Reject version ranges.
        if is_version_range(&version) {
            return Err(UtilError::PomUnsupportedVersionRange {
                group,
                artifact,
                range: version,
            });
        }

        deps.push(PomDependency {
            group_id: group,
            artifact_id: artifact,
            version,
        });
    }

    Ok(deps)
}

// ---------------------------------------------------------------------------
// Public API — parsing
// ---------------------------------------------------------------------------

/// Parse a POM XML string into a [`Pom`].
///
/// Resolution order for `groupId` and `version`:
/// 1. The POM's own `<groupId>` / `<version>` elements
/// 2. The POM's inline `<parent>` element (no external fetch)
/// 3. Caller-supplied fallback values (`known_group_id` / `known_version`)
///
/// The caller-supplied values typically come from the Maven coordinate in
/// `konvoy.toml` and the pinned version in `konvoy.lock`, so no parent POM
/// fetch is ever required.
///
/// Property interpolation is limited to `${project.version}` and
/// `${project.groupId}`. Dependencies are filtered to compile-scope only;
/// optional dependencies are skipped entirely.
///
/// # Errors
///
/// Returns an error if:
/// - The XML is malformed or missing required elements (`artifactId`, etc.)
/// - `groupId` or `version` cannot be resolved from any source
/// - A dependency uses a version range (e.g. `[1.0,2.0)`)
/// - A property placeholder other than `${project.version}` or
///   `${project.groupId}` is encountered
pub fn parse_pom(
    xml: &str,
    known_group_id: Option<&str>,
    known_version: Option<&str>,
) -> Result<Pom, UtilError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| UtilError::PomParse {
        reason: e.to_string(),
    })?;

    let root = doc.root_element();

    let identity = extract_identity(&root);
    let parent_identity = extract_parent_identity(&root);

    // Resolve groupId: project > inline parent > caller-supplied > error.
    let group_id = identity
        .group_id
        .or_else(|| parent_identity.as_ref().and_then(|p| p.group_id.clone()))
        .or_else(|| known_group_id.map(ToOwned::to_owned))
        .ok_or_else(|| UtilError::PomParse {
            reason: "POM is missing <groupId> and has no <parent> to inherit from — check the maven coordinate in konvoy.toml".to_owned(),
        })?;

    let artifact_id = identity.artifact_id.ok_or_else(|| UtilError::PomParse {
        reason: "POM is missing <artifactId>".to_owned(),
    })?;

    // Resolve version: project > inline parent > caller-supplied > error.
    let version = identity
        .version
        .or_else(|| parent_identity.as_ref().and_then(|p| p.version.clone()))
        .or_else(|| known_version.map(ToOwned::to_owned))
        .ok_or_else(|| UtilError::PomParse {
            reason: "POM is missing <version> and has no <parent> to inherit from — check the version in konvoy.toml".to_owned(),
        })?;

    let dependencies = extract_dependencies(&root, &version, &group_id)?;

    Ok(Pom {
        group_id,
        artifact_id,
        version,
        dependencies,
    })
}

// ---------------------------------------------------------------------------
// Public API — POM URL and fetching
// ---------------------------------------------------------------------------

/// Build the Maven Central URL for a POM file.
///
/// The URL pattern is:
/// `https://repo1.maven.org/maven2/{group_path}/{artifact_id}/{version}/{artifact_id}-{version}.pom`
///
/// where `group_path` replaces dots in `group_id` with `/`.
pub fn pom_url(group_id: &str, artifact_id: &str, version: &str) -> String {
    crate::maven::maven_artifact_url(group_id, artifact_id, version, "pom")
}

/// Build the on-disk cache path for a Maven artifact metadata file.
///
/// Layout matches the contract in CLAUDE.md:
/// `~/.konvoy/cache/pom/<group_path>/<artifact_id>-<version>.<extension>`.
///
/// `group_id` is split on `.` so that `org.jetbrains.kotlinx` becomes
/// nested directories (matching Maven Central's own layout). The
/// `extension` is `"pom"` for POM files or `"module"` for Gradle Module
/// Metadata files; both share the same cache root so a single layout-version
/// bump suffices for either format.
///
/// # Errors
/// Returns an error if the home directory cannot be located, or any
/// identifier/version component fails validation (path-traversal guard).
pub(crate) fn metadata_cache_path(
    group_id: &str,
    artifact_id: &str,
    version: &str,
    extension: &str,
) -> Result<std::path::PathBuf, UtilError> {
    crate::artifact::validate_identifier(group_id)?;
    crate::artifact::validate_identifier(artifact_id)?;
    // `validate_identifier` is strictly stronger than `validate_version`
    // (same charset plus `..` rejection), so a single call suffices.
    crate::artifact::validate_identifier(version)?;

    let mut path = crate::fs::konvoy_home()?.join("cache").join("pom");
    for segment in group_id.split('.') {
        // `validate_identifier` already rejects `..`, but be defensive against
        // empty segments produced by leading/trailing dots.
        if segment.is_empty() {
            return Err(UtilError::InvalidVersion {
                version: group_id.to_owned(),
            });
        }
        path.push(segment);
    }
    path.push(format!("{artifact_id}-{version}.{extension}"));
    Ok(path)
}

/// Build the on-disk cache path for a POM file.
///
/// Thin wrapper over [`metadata_cache_path`] with `extension = "pom"`.
fn pom_cache_path(
    group_id: &str,
    artifact_id: &str,
    version: &str,
) -> Result<std::path::PathBuf, UtilError> {
    metadata_cache_path(group_id, artifact_id, version, "pom")
}

/// Atomically write `contents` to `dest` via a temp file in the same directory.
///
/// Mirrors the pattern in `ensure_artifact`: write to a `.tmp-<pid>` sibling
/// and rename into place. If another writer wins the race, the rename may fail
/// but the destination exists with valid contents, which is what we want.
///
/// Returns `Ok(())` on success. On failure to create the parent directory or
/// to write the temp file, returns the underlying I/O error. A failed rename
/// when `dest` already exists is treated as success (another writer placed it).
///
/// # Errors
/// Returns the underlying I/O error if the parent directory cannot be created
/// or the temp file cannot be written. A rename failure where `dest` already
/// exists is not treated as an error.
pub(crate) fn write_cache_atomic(dest: &std::path::Path, contents: &[u8]) -> Result<(), UtilError> {
    let parent = dest.parent().ok_or_else(|| UtilError::Io {
        path: dest.display().to_string(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "cache path has no parent"),
    })?;

    std::fs::create_dir_all(parent).map_err(|source| UtilError::Io {
        path: parent.display().to_string(),
        source,
    })?;

    let pid = std::process::id();
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| UtilError::Io {
            path: dest.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cache path has no filename",
            ),
        })?;
    let tmp_path = parent.join(format!(".tmp-{pid}-{file_name}"));

    std::fs::write(&tmp_path, contents).map_err(|source| UtilError::Io {
        path: tmp_path.display().to_string(),
        source,
    })?;

    match std::fs::rename(&tmp_path, dest) {
        Ok(()) => Ok(()),
        Err(_) if dest.exists() => {
            // Another writer beat us — clean up our temp file and proceed.
            // POSIX rename(2) is atomic, so if dest now exists another writer completed an atomic place — safe to read.
            // NTFS MoveFileEx is similarly atomic for same-volume renames.
            let _ = std::fs::remove_file(&tmp_path);
            Ok(())
        }
        Err(source) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(UtilError::Io {
                path: dest.display().to_string(),
                source,
            })
        }
    }
}

/// Fetch a POM file from Maven Central and return its contents as a string.
///
/// Uses a disk cache at `~/.konvoy/cache/pom/<group_path>/<artifact_id>-<version>.pom`.
/// Cache hits avoid the HTTP round trip entirely. Cache misses fetch from
/// Maven Central, then write the contents atomically into the cache (write
/// failures are non-fatal — the function still returns the fetched content).
///
/// Uses `ureq` with a 60-second global timeout.
///
/// # Errors
///
/// Returns `UtilError::Download` if the HTTP request fails or the response
/// body cannot be read. Returns `UtilError::InvalidVersion` if any of
/// `group_id`/`artifact_id`/`version` fail validation.
pub fn fetch_pom(group_id: &str, artifact_id: &str, version: &str) -> Result<String, UtilError> {
    // 1. Try the on-disk cache.
    let cache_path = pom_cache_path(group_id, artifact_id, version)?;
    if cache_path.exists() {
        if let Ok(body) = std::fs::read_to_string(&cache_path) {
            return Ok(body);
        }
        // Cache read failed (truncated, permissions, etc.); fall through to
        // fetch from network so a degraded cache never blocks builds.
    }

    // 2. Cache miss — fetch from Maven Central.
    let url = pom_url(group_id, artifact_id, version);

    let agent = crate::download::http_agent(60);

    let response = agent.get(&url).call().map_err(|e| UtilError::Download {
        message: format!("failed to fetch POM from {url}: {e}"),
    })?;

    let body = response
        .into_body()
        .read_to_string()
        .map_err(|e| UtilError::Download {
            message: format!("failed to read POM response body from {url}: {e}"),
        })?;

    // 3. Best-effort cache write. A failure here must not block the caller.
    if let Err(e) = write_cache_atomic(&cache_path, body.as_bytes()) {
        eprintln!(
            "warning: failed to cache POM at {}: {e}",
            cache_path.display()
        );
    }

    Ok(body)
}

// ---------------------------------------------------------------------------
// Public API — POM to metadata adapter
// ---------------------------------------------------------------------------

/// Strip a known Maven target suffix from an artifact ID.
///
/// Per-target POMs reference dependencies with target-suffixed artifact IDs
/// (e.g. `atomicfu-macosarm64`). We strip that suffix to get the base
/// artifact ID (`atomicfu`).
pub fn strip_target_suffix(artifact_id: &str, maven_suffix: &str) -> String {
    let suffix = format!("-{maven_suffix}");
    if let Some(base) = artifact_id.strip_suffix(&suffix) {
        base.to_owned()
    } else {
        artifact_id.to_owned()
    }
}

/// Convert a parsed [`Pom`] into [`ArtifactMetadata`].
///
/// Strips target suffixes from dependency artifact IDs using `maven_suffix`
/// (e.g. `"linuxx64"`) and returns an empty `files` list because POM files
/// do not contain information about cinterop or other additional artifacts.
pub fn pom_to_metadata(pom: &Pom, maven_suffix: &str) -> ArtifactMetadata {
    let dependencies = pom
        .dependencies
        .iter()
        .map(|d| MetadataDep {
            group_id: d.group_id.clone(),
            artifact_id: strip_target_suffix(&d.artifact_id, maven_suffix),
            version: d.version.clone(),
        })
        .collect();

    ArtifactMetadata {
        dependencies,
        files: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A trimmed-down POM based on kotlinx-coroutines-core-macosarm64.
    const COROUTINES_POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.jetbrains.kotlinx</groupId>
  <artifactId>kotlinx-coroutines-core-macosarm64</artifactId>
  <version>1.9.0</version>
  <dependencies>
    <dependency>
      <groupId>org.jetbrains.kotlinx</groupId>
      <artifactId>atomicfu-macosarm64</artifactId>
      <version>0.25.0</version>
      <scope>compile</scope>
    </dependency>
    <dependency>
      <groupId>org.jetbrains.kotlin</groupId>
      <artifactId>kotlin-stdlib</artifactId>
      <version>2.0.21</version>
      <scope>compile</scope>
    </dependency>
    <dependency>
      <groupId>org.jetbrains.kotlinx</groupId>
      <artifactId>kotlinx-coroutines-test</artifactId>
      <version>1.9.0</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
</project>"#;

    #[test]
    fn parse_real_pom_extracts_compile_deps() {
        let pom = parse_pom(COROUTINES_POM, None, None).unwrap();
        assert_eq!(pom.group_id, "org.jetbrains.kotlinx");
        assert_eq!(pom.artifact_id, "kotlinx-coroutines-core-macosarm64");
        assert_eq!(pom.version, "1.9.0");
        assert_eq!(pom.dependencies.len(), 2);

        let dep0 = pom.dependencies.first().unwrap();
        assert_eq!(dep0.group_id, "org.jetbrains.kotlinx");
        assert_eq!(dep0.artifact_id, "atomicfu-macosarm64");
        assert_eq!(dep0.version, "0.25.0");

        let dep1 = pom.dependencies.get(1).unwrap();
        assert_eq!(dep1.group_id, "org.jetbrains.kotlin");
        assert_eq!(dep1.artifact_id, "kotlin-stdlib");
        assert_eq!(dep1.version, "2.0.21");
    }

    #[test]
    fn parse_pom_skips_test_scope() {
        let pom = parse_pom(COROUTINES_POM, None, None).unwrap();
        // The test-scoped dependency should not be present.
        assert!(!pom
            .dependencies
            .iter()
            .any(|d| d.artifact_id == "kotlinx-coroutines-test"));
    }

    #[test]
    fn inherits_from_inline_parent() {
        // The inline <parent> is used when the POM itself lacks groupId/version.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>2.0.0</version>
  </parent>
  <artifactId>child</artifactId>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert_eq!(pom.group_id, "com.example");
        assert_eq!(pom.version, "2.0.0");
        assert_eq!(pom.artifact_id, "child");
    }

    #[test]
    fn caller_supplied_fallback_for_group_id() {
        // POM has no groupId and no parent — caller supplies it.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <artifactId>orphan</artifactId>
  <version>1.0.0</version>
</project>"#;

        let pom = parse_pom(xml, Some("com.caller"), None).unwrap();
        assert_eq!(pom.group_id, "com.caller");
    }

    #[test]
    fn caller_supplied_fallback_for_version() {
        // POM has no version and no parent — caller supplies it.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>no-ver</artifactId>
</project>"#;

        let pom = parse_pom(xml, None, Some("9.9.9")).unwrap();
        assert_eq!(pom.version, "9.9.9");
    }

    #[test]
    fn pom_value_takes_precedence_over_caller_fallback() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.pom</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
</project>"#;

        let pom = parse_pom(xml, Some("com.caller"), Some("9.9.9")).unwrap();
        assert_eq!(pom.group_id, "com.pom");
        assert_eq!(pom.version, "1.0.0");
    }

    #[test]
    fn inline_parent_takes_precedence_over_caller_fallback() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <parent>
    <groupId>com.parent</groupId>
    <artifactId>parent</artifactId>
    <version>3.0.0</version>
  </parent>
  <artifactId>child</artifactId>
</project>"#;

        let pom = parse_pom(xml, Some("com.caller"), Some("9.9.9")).unwrap();
        assert_eq!(pom.group_id, "com.parent");
        assert_eq!(pom.version, "3.0.0");
    }

    #[test]
    fn reject_version_range() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>org.other</groupId>
      <artifactId>ranged</artifactId>
      <version>[1.0,2.0)</version>
    </dependency>
  </dependencies>
</project>"#;

        let err = parse_pom(xml, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("version range"), "error was: {msg}");
        assert!(msg.contains("[1.0,2.0)"), "error was: {msg}");
        assert!(msg.contains("org.other:ranged"), "error was: {msg}");
    }

    #[test]
    fn skip_optional_dependencies() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>required</artifactId>
      <version>1.0.0</version>
    </dependency>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>opt-dep</artifactId>
      <version>2.0.0</version>
      <optional>true</optional>
    </dependency>
  </dependencies>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert_eq!(pom.dependencies.len(), 1);
        assert_eq!(pom.dependencies.first().unwrap().artifact_id, "required");
    }

    #[test]
    fn skip_provided_and_runtime_scopes() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>compile-dep</artifactId>
      <version>1.0.0</version>
    </dependency>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>provided-dep</artifactId>
      <version>1.0.0</version>
      <scope>provided</scope>
    </dependency>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>runtime-dep</artifactId>
      <version>1.0.0</version>
      <scope>runtime</scope>
    </dependency>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>system-dep</artifactId>
      <version>1.0.0</version>
      <scope>system</scope>
    </dependency>
  </dependencies>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert_eq!(pom.dependencies.len(), 1);
        assert_eq!(pom.dependencies.first().unwrap().artifact_id, "compile-dep");
    }

    #[test]
    fn property_interpolation_project_version() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <version>5.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>sibling</artifactId>
      <version>${project.version}</version>
    </dependency>
  </dependencies>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert_eq!(pom.dependencies.first().unwrap().version, "5.0.0");
    }

    #[test]
    fn property_interpolation_project_group_id() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example.group</groupId>
  <artifactId>my-lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>${project.groupId}</groupId>
      <artifactId>sibling</artifactId>
      <version>1.0.0</version>
    </dependency>
  </dependencies>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert_eq!(
            pom.dependencies.first().unwrap().group_id,
            "com.example.group"
        );
    }

    #[test]
    fn reject_unsupported_property() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>other</artifactId>
      <version>${some.custom.version}</version>
    </dependency>
  </dependencies>
</project>"#;

        let err = parse_pom(xml, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unsupported property"), "error was: {msg}");
        assert!(msg.contains("${some.custom.version}"), "error was: {msg}");
    }

    #[test]
    fn pom_url_format() {
        let url = pom_url("org.jetbrains.kotlinx", "kotlinx-coroutines-core", "1.9.0");
        assert_eq!(
            url,
            "https://repo1.maven.org/maven2/org/jetbrains/kotlinx/kotlinx-coroutines-core/1.9.0/kotlinx-coroutines-core-1.9.0.pom"
        );
    }

    #[test]
    fn pom_url_single_segment_group() {
        let url = pom_url("com", "mylib", "1.0.0");
        assert_eq!(
            url,
            "https://repo1.maven.org/maven2/com/mylib/1.0.0/mylib-1.0.0.pom"
        );
    }

    #[test]
    fn parse_pom_empty_dependencies() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>no-deps</artifactId>
  <version>1.0.0</version>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert!(pom.dependencies.is_empty());
        assert_eq!(pom.artifact_id, "no-deps");
    }

    #[test]
    fn parse_pom_empty_dependencies_element() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>empty-deps</artifactId>
  <version>1.0.0</version>
  <dependencies/>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert!(pom.dependencies.is_empty());
    }

    #[test]
    fn parse_pom_missing_artifact_id_errors() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <version>1.0.0</version>
</project>"#;

        let err = parse_pom(xml, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing <artifactId>"), "error was: {msg}");
    }

    #[test]
    fn missing_group_id_no_fallback_errors() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <artifactId>orphan</artifactId>
  <version>1.0.0</version>
</project>"#;

        let err = parse_pom(xml, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing <groupId>"), "error was: {msg}");
        assert!(
            msg.contains("konvoy.toml"),
            "error should be actionable: {msg}"
        );
    }

    #[test]
    fn missing_version_no_fallback_errors() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>no-ver</artifactId>
</project>"#;

        let err = parse_pom(xml, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing <version>"), "error was: {msg}");
        assert!(
            msg.contains("konvoy.toml"),
            "error should be actionable: {msg}"
        );
    }

    #[test]
    fn parse_pom_malformed_xml_errors() {
        let xml = "this is not xml at all";
        let err = parse_pom(xml, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot parse POM"), "error was: {msg}");
    }

    #[test]
    fn parse_pom_default_scope_is_compile() {
        // A dependency with no <scope> element defaults to compile.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.other</groupId>
      <artifactId>no-scope</artifactId>
      <version>2.0.0</version>
    </dependency>
  </dependencies>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert_eq!(pom.dependencies.len(), 1);
        assert_eq!(pom.dependencies.first().unwrap().artifact_id, "no-scope");
    }

    #[test]
    fn version_range_detection() {
        assert!(is_version_range("[1.0,2.0)"));
        assert!(is_version_range("(,1.5]"));
        assert!(is_version_range("[1.0,)"));
        assert!(is_version_range("[1.0]"));
        assert!(!is_version_range("1.0.0"));
        assert!(!is_version_range("2.0.0-RC1"));
        assert!(!is_version_range(""));
    }

    #[test]
    fn interpolation_replaces_both_properties() {
        let result =
            interpolate_property("${project.groupId}:${project.version}", "1.0", "com.ex").unwrap();
        assert_eq!(result, "com.ex:1.0");
    }

    #[test]
    fn interpolation_no_placeholders_returns_unchanged() {
        let result = interpolate_property("plain-value", "1.0", "com.ex").unwrap();
        assert_eq!(result, "plain-value");
    }

    #[test]
    fn fetch_pom_invalid_coordinates_errors() {
        // Use a non-routable address to ensure fast failure.
        // We can't easily test fetch_pom against real Maven Central in unit tests,
        // but we can verify it returns an error for unreachable hosts by using
        // a coordinate that doesn't exist.
        let result = fetch_pom("com.nonexistent.fake", "no-such-artifact", "0.0.0");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("download failed"), "error was: {msg}");
    }

    #[test]
    fn fetch_pom_rejects_path_traversal_in_group_id() {
        // Path-traversal in the group must be caught before any HTTP work or
        // cache-path construction.
        let result = fetch_pom("../etc/passwd", "lib", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn fetch_pom_rejects_invalid_version() {
        let err = fetch_pom("com.example", "lib", "1.0/0").unwrap_err();
        assert!(
            matches!(err, UtilError::InvalidVersion { .. }),
            "expected InvalidVersion, got: {err:?}"
        );
    }

    #[test]
    fn pom_cache_path_layout_matches_claude_md_contract() {
        // Verify the layout suffix without depending on HOME (env mutation in
        // tests races with other tests in the crate). The suffix is the
        // load-bearing part — the prefix is `konvoy_home()` which is tested
        // independently in fs.rs.
        let path =
            pom_cache_path("org.jetbrains.kotlinx", "kotlinx-coroutines-core", "1.9.0").unwrap();
        let suffix: std::path::PathBuf = [
            "cache",
            "pom",
            "org",
            "jetbrains",
            "kotlinx",
            "kotlinx-coroutines-core-1.9.0.pom",
        ]
        .iter()
        .collect();
        assert!(
            path.ends_with(&suffix),
            "expected path to end with {} — got {}",
            suffix.display(),
            path.display()
        );
    }

    #[test]
    fn pom_cache_path_rejects_invalid_identifiers() {
        assert!(pom_cache_path("..", "lib", "1.0.0").is_err());
        assert!(pom_cache_path("com.ex", "..", "1.0.0").is_err());
        assert!(pom_cache_path("com.ex", "lib", "..").is_err());
    }

    #[test]
    fn write_cache_atomic_creates_parent_and_writes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("nested").join("dir").join("file.pom");

        write_cache_atomic(&dest, b"contents").unwrap();

        let written = std::fs::read(&dest).unwrap();
        assert_eq!(written, b"contents");
    }

    #[test]
    fn write_cache_atomic_overwrites_existing_file() {
        // Overwriting is fine — Unix rename replaces the target atomically.
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("file.pom");
        std::fs::write(&dest, b"old").unwrap();

        write_cache_atomic(&dest, b"new").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
    }

    #[test]
    fn parse_pom_dependency_missing_version_inherits_empty() {
        // When a dependency has no <version>, it gets an empty string.
        // In practice the caller would resolve it via dependencyManagement
        // (out of scope), but we shouldn't crash.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.other</groupId>
      <artifactId>managed-dep</artifactId>
    </dependency>
  </dependencies>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        assert_eq!(pom.dependencies.len(), 1);
        assert_eq!(pom.dependencies.first().unwrap().version, "");
    }

    #[test]
    fn child_overrides_inline_parent_version() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0.0</version>
  </parent>
  <artifactId>child</artifactId>
  <version>2.0.0</version>
</project>"#;

        let pom = parse_pom(xml, None, None).unwrap();
        // Child's own version takes precedence over parent.
        assert_eq!(pom.version, "2.0.0");
    }

    #[test]
    fn reject_version_range_parentheses() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>org.other</groupId>
      <artifactId>ranged</artifactId>
      <version>(1.0,2.0)</version>
    </dependency>
  </dependencies>
</project>"#;

        let err = parse_pom(xml, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("version range"), "error was: {msg}");
    }

    #[test]
    fn strip_target_suffix_removes_suffix() {
        assert_eq!(
            strip_target_suffix("atomicfu-macosarm64", "macosarm64"),
            "atomicfu"
        );
        assert_eq!(
            strip_target_suffix("kotlinx-coroutines-core-linuxx64", "linuxx64"),
            "kotlinx-coroutines-core"
        );
    }

    #[test]
    fn strip_target_suffix_no_suffix() {
        assert_eq!(strip_target_suffix("atomicfu", "macosarm64"), "atomicfu");
    }

    #[test]
    fn strip_target_suffix_empty_artifact_id() {
        // An empty artifact ID should remain empty.
        assert_eq!(strip_target_suffix("", "linuxx64"), "");
    }

    #[test]
    fn strip_target_suffix_suffix_in_middle_not_stripped() {
        // If the target suffix appears in the middle (not at the end), it
        // should NOT be stripped.
        assert_eq!(
            strip_target_suffix("linuxx64-special-lib", "linuxx64"),
            "linuxx64-special-lib"
        );
    }

    #[test]
    fn pom_to_metadata_strips_suffixes_and_returns_empty_files() {
        let pom = parse_pom(COROUTINES_POM, None, None).unwrap();
        let metadata = pom_to_metadata(&pom, "macosarm64");

        assert_eq!(metadata.dependencies.len(), 2);

        let dep0 = metadata.dependencies.first().unwrap();
        assert_eq!(dep0.group_id, "org.jetbrains.kotlinx");
        // "atomicfu-macosarm64" should be stripped to "atomicfu".
        assert_eq!(dep0.artifact_id, "atomicfu");
        assert_eq!(dep0.version, "0.25.0");

        let dep1 = metadata.dependencies.get(1).unwrap();
        assert_eq!(dep1.artifact_id, "kotlin-stdlib");

        // POM adapter returns no files (POM doesn't know about cinterop klibs).
        assert!(metadata.files.is_empty());
    }

    #[test]
    fn pom_to_metadata_empty_deps() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>no-deps</artifactId>
  <version>1.0.0</version>
</project>"#;
        let pom = parse_pom(xml, None, None).unwrap();
        let metadata = pom_to_metadata(&pom, "linuxx64");
        assert!(metadata.dependencies.is_empty());
        assert!(metadata.files.is_empty());
    }

    #[test]
    fn pom_to_metadata_strips_different_target_suffixes() {
        // Verify that different target suffixes are stripped correctly.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>lib-macosarm64</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.other</groupId>
      <artifactId>dep-macosarm64</artifactId>
      <version>2.0.0</version>
    </dependency>
    <dependency>
      <groupId>com.other</groupId>
      <artifactId>no-suffix</artifactId>
      <version>3.0.0</version>
    </dependency>
  </dependencies>
</project>"#;
        let pom = parse_pom(xml, None, None).unwrap();
        let metadata = pom_to_metadata(&pom, "macosarm64");

        assert_eq!(metadata.dependencies.len(), 2);
        // "dep-macosarm64" should be stripped to "dep".
        assert_eq!(metadata.dependencies.first().unwrap().artifact_id, "dep");
        // "no-suffix" stays unchanged.
        assert_eq!(
            metadata.dependencies.get(1).unwrap().artifact_id,
            "no-suffix"
        );
    }

    #[test]
    fn pom_to_metadata_always_returns_empty_files() {
        // Regardless of the POM content, the files list should always be empty
        // because POM files do not contain information about cinterop klibs.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>big-lib-linuxx64</artifactId>
  <version>5.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.a</groupId>
      <artifactId>dep1-linuxx64</artifactId>
      <version>1.0</version>
    </dependency>
    <dependency>
      <groupId>com.b</groupId>
      <artifactId>dep2-linuxx64</artifactId>
      <version>2.0</version>
    </dependency>
    <dependency>
      <groupId>com.c</groupId>
      <artifactId>dep3-linuxx64</artifactId>
      <version>3.0</version>
    </dependency>
  </dependencies>
</project>"#;
        let pom = parse_pom(xml, None, None).unwrap();
        let metadata = pom_to_metadata(&pom, "linuxx64");

        assert_eq!(metadata.dependencies.len(), 3);
        assert!(
            metadata.files.is_empty(),
            "POM adapter must never produce files"
        );
    }

    #[test]
    fn pom_to_metadata_preserves_group_and_version() {
        // Verify that group_id and version are passed through unchanged.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>org.jetbrains.kotlinx</groupId>
  <artifactId>atomicfu-linuxx64</artifactId>
  <version>0.23.1</version>
  <dependencies>
    <dependency>
      <groupId>org.jetbrains.kotlin</groupId>
      <artifactId>kotlin-native-prebuilt-linuxx64</artifactId>
      <version>1.9.21</version>
    </dependency>
  </dependencies>
</project>"#;
        let pom = parse_pom(xml, None, None).unwrap();
        let metadata = pom_to_metadata(&pom, "linuxx64");

        let dep = metadata.dependencies.first().unwrap();
        assert_eq!(dep.group_id, "org.jetbrains.kotlin");
        assert_eq!(dep.artifact_id, "kotlin-native-prebuilt");
        assert_eq!(dep.version, "1.9.21");
    }

    // -----------------------------------------------------------------
    // metadata_cache_path validation branches (one test per validation site)
    // -----------------------------------------------------------------

    #[test]
    fn metadata_cache_path_rejects_invalid_group_id() {
        // First validation site: group_id.
        let err = metadata_cache_path("../etc", "lib", "1.0.0", "pom").unwrap_err();
        assert!(
            matches!(err, UtilError::InvalidVersion { .. }),
            "expected InvalidVersion for bad group_id, got: {err:?}"
        );
    }

    #[test]
    fn metadata_cache_path_rejects_invalid_artifact_id() {
        // Second validation site: artifact_id.
        let err = metadata_cache_path("com.example", "..", "1.0.0", "pom").unwrap_err();
        assert!(
            matches!(err, UtilError::InvalidVersion { .. }),
            "expected InvalidVersion for bad artifact_id, got: {err:?}"
        );
    }

    #[test]
    fn metadata_cache_path_rejects_invalid_version() {
        // Third validation site: version.
        let err = metadata_cache_path("com.example", "lib", "..", "pom").unwrap_err();
        assert!(
            matches!(err, UtilError::InvalidVersion { .. }),
            "expected InvalidVersion for bad version, got: {err:?}"
        );
    }

    #[test]
    fn metadata_cache_path_rejects_empty_dot_segment() {
        // A leading dot makes `split('.')` produce an empty segment even
        // though `validate_identifier` (which only rejects ".." and bad
        // chars) accepts the input. That's specifically what the in-loop
        // `if segment.is_empty()` guard exists for.
        let err = metadata_cache_path(".com", "lib", "1.0.0", "pom").unwrap_err();
        assert!(
            matches!(err, UtilError::InvalidVersion { .. }),
            "expected InvalidVersion for empty segment, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------
    // write_cache_atomic — error branches
    // -----------------------------------------------------------------

    #[test]
    fn write_cache_atomic_errors_when_path_has_no_filename() {
        // `Path::file_name()` returns None when the path ends in `..`. The
        // parent and the create_dir_all succeed (parent() returns the
        // containing dir, which exists), so the no-filename branch is the
        // first failure point.
        let tmp = tempfile::tempdir().unwrap();
        let bad_path = tmp.path().join("..");
        let result = write_cache_atomic(&bad_path, b"contents");
        assert!(result.is_err(), "path ending in `..` has no filename");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(err, UtilError::Io { .. }),
            "expected Io error, got: {msg}"
        );
        assert!(
            msg.contains("no filename") || msg.contains("InvalidInput"),
            "error should mention the no-filename invariant, got: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_cache_atomic_errors_when_path_has_no_parent() {
        // The root directory `/` has no parent — `Path::parent()` returns None.
        // This exercises the early `ok_or_else` branch.
        let result = write_cache_atomic(std::path::Path::new("/"), b"contents");
        assert!(result.is_err(), "writing to / must fail with a typed error");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(err, UtilError::Io { .. }),
            "expected Io error, got: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_cache_atomic_errors_on_unwritable_parent() {
        // Reproduces the "rename failed, dest does not exist" branch:
        // make the parent read-only so the temp-file write fails. (Running
        // as root would bypass this; CI normally doesn't.)
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("ro");
        std::fs::create_dir(&parent).unwrap();
        let mut perms = std::fs::metadata(&parent).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        let original = perms.mode();
        perms.set_mode(0o500); // r-x -- no writes
        std::fs::set_permissions(&parent, perms).unwrap();

        let dest = parent.join("file.pom");
        let result = write_cache_atomic(&dest, b"contents");

        // Restore permissions so the tempdir cleans up.
        let mut restored = std::fs::metadata(&parent).unwrap().permissions();
        restored.set_mode(original);
        let _ = std::fs::set_permissions(&parent, restored);

        // On most filesystems the temp-file write fails with EACCES; on
        // an unusual setup it could succeed, so we only assert when the
        // error path engages.
        if let Err(err) = result {
            assert!(
                matches!(err, UtilError::Io { .. }),
                "expected Io error, got: {err}"
            );
        }
    }

    // -----------------------------------------------------------------
    // fetch_pom — cache hit and cache-write failure paths
    // -----------------------------------------------------------------

    #[test]
    fn fetch_pom_cache_hit_returns_disk_contents_without_network() {
        // Verify the cache-hit early-return path: by pre-populating the
        // on-disk cache, `fetch_pom` must read it back without trying any
        // HTTP I/O. Uses a HOME override so we point at a temp dir.
        let _guard = crate::test_util::ENV_LOCK.lock().unwrap();

        let saved_home = std::env::var("HOME").ok();
        let saved_profile = std::env::var("USERPROFILE").ok();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        // Clear USERPROFILE so the unix path is used everywhere.
        std::env::remove_var("USERPROFILE");

        // Construct the cache path that `fetch_pom` will look at, and
        // pre-populate it.
        let group = "com.example";
        let artifact = "cached-lib";
        let version = "1.2.3";
        let cache_path = pom_cache_path(group, artifact, version).unwrap();
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        let expected_body = "<project><artifactId>cached-lib</artifactId></project>";
        std::fs::write(&cache_path, expected_body).unwrap();

        let result = fetch_pom(group, artifact, version);

        // Restore HOME/USERPROFILE before asserting so a panic leaves the
        // environment clean for other tests.
        if let Some(v) = saved_home {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(v) = saved_profile {
            std::env::set_var("USERPROFILE", v);
        }

        let body = result.unwrap();
        assert_eq!(body, expected_body, "cache hit must return the disk bytes");
    }
}
