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
use crate::maven::MAVEN_CENTRAL;

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
    loop {
        let Some(start) = result.find("${") else {
            break;
        };
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
    let group_path = group_id.replace('.', "/");
    format!("{MAVEN_CENTRAL}/{group_path}/{artifact_id}/{version}/{artifact_id}-{version}.pom")
}

/// Fetch a POM file from Maven Central and return its contents as a string.
///
/// Uses `ureq` with a 30-second connect timeout and 60-second global timeout.
///
/// # Errors
///
/// Returns `UtilError::Download` if the HTTP request fails or the response
/// body cannot be read.
pub fn fetch_pom(group_id: &str, artifact_id: &str, version: &str) -> Result<String, UtilError> {
    let url = pom_url(group_id, artifact_id, version);

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .build(),
    );

    let response = agent.get(&url).call().map_err(|e| UtilError::Download {
        message: format!("failed to fetch POM from {url}: {e}"),
    })?;

    let body = response
        .into_body()
        .read_to_string()
        .map_err(|e| UtilError::Download {
            message: format!("failed to read POM response body from {url}: {e}"),
        })?;

    Ok(body)
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
}
