//! Curated library index for Maven dependency resolution.
//!
//! Each library is defined by a TOML descriptor file compiled into the binary.
//! The engine resolves short names (e.g. `"kotlinx-coroutines"`) to Maven
//! coordinate templates, substituting `{target}` and `{version}` placeholders
//! at resolution time.

use serde::Deserialize;

use konvoy_targets::Target;
use konvoy_util::maven::MavenCoordinate;

use crate::error::EngineError;

/// A library descriptor loaded from a compiled-in TOML file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryDescriptor {
    /// Library short name (e.g. `"kotlinx-coroutines"`).
    pub name: String,
    /// Maven coordinate template with `{target}` and `{version}` placeholders.
    /// Example: `"org.jetbrains.kotlinx:kotlinx-coroutines-core-{target}:{version}:klib"`
    pub maven: String,
}

// ---------------------------------------------------------------------------
// Descriptor loading
// ---------------------------------------------------------------------------

const KOTLINX_COROUTINES: &str = include_str!("../../../libraries/kotlinx-coroutines.toml");
const KOTLINX_DATETIME: &str = include_str!("../../../libraries/kotlinx-datetime.toml");
const KOTLINX_IO: &str = include_str!("../../../libraries/kotlinx-io.toml");
const KOTLINX_ATOMICFU: &str = include_str!("../../../libraries/kotlinx-atomicfu.toml");

/// Load all built-in library descriptors.
///
/// # Errors
/// Returns an error if any embedded descriptor fails to parse.
pub fn load_descriptors() -> Result<Vec<LibraryDescriptor>, EngineError> {
    let sources = [
        ("kotlinx-coroutines.toml", KOTLINX_COROUTINES),
        ("kotlinx-datetime.toml", KOTLINX_DATETIME),
        ("kotlinx-io.toml", KOTLINX_IO),
        ("kotlinx-atomicfu.toml", KOTLINX_ATOMICFU),
    ];
    let mut descriptors = Vec::with_capacity(sources.len());

    for (filename, content) in sources {
        let descriptor: LibraryDescriptor =
            toml::from_str(content).map_err(|e| EngineError::InvalidLibraryDescriptor {
                name: filename.to_owned(),
                reason: e.to_string(),
            })?;
        descriptors.push(descriptor);
    }

    Ok(descriptors)
}

/// Look up a library by short name.
///
/// # Errors
/// Returns an error if the embedded descriptors fail to parse.
pub fn lookup(name: &str) -> Result<Option<LibraryDescriptor>, EngineError> {
    let descriptors = load_descriptors()?;
    Ok(descriptors.into_iter().find(|d| d.name == name))
}

/// Return a comma-separated list of available library names (for error messages).
///
/// # Errors
/// Returns an error if the embedded descriptors fail to parse.
pub fn available_library_names() -> Result<String, EngineError> {
    let descriptors = load_descriptors()?;
    Ok(descriptors
        .iter()
        .map(|d| d.name.as_str())
        .collect::<Vec<_>>()
        .join(", "))
}

// ---------------------------------------------------------------------------
// Coordinate resolution
// ---------------------------------------------------------------------------

/// Resolve a library descriptor into a concrete `MavenCoordinate` for a given
/// version and target.
///
/// Substitutes `{version}` and `{target}` placeholders in the maven template
/// string, then parses the result into a `MavenCoordinate`.
///
/// # Errors
/// Returns an error if the substituted coordinate string cannot be parsed.
pub fn resolve_coordinate(
    descriptor: &LibraryDescriptor,
    version: &str,
    target: &Target,
) -> Result<MavenCoordinate, EngineError> {
    let target_suffix = target.to_maven_suffix();
    let coord_str = descriptor
        .maven
        .replace("{version}", version)
        .replace("{target}", &target_suffix);
    MavenCoordinate::parse(&coord_str).map_err(|e| EngineError::InvalidLibraryDescriptor {
        name: descriptor.name.clone(),
        reason: format!("invalid maven coordinate after substitution: {e}"),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::str::FromStr;

    use konvoy_util::maven::MAVEN_CENTRAL;

    use super::*;

    #[test]
    fn load_descriptors_succeeds() {
        let descriptors = load_descriptors().unwrap();
        assert!(
            descriptors.len() >= 4,
            "expected at least 4 descriptors, got {}",
            descriptors.len()
        );
        // Verify each descriptor has non-empty fields.
        for d in &descriptors {
            assert!(!d.name.is_empty(), "descriptor name should not be empty");
            assert!(!d.maven.is_empty(), "descriptor maven should not be empty");
        }
    }

    #[test]
    fn lookup_known_library() {
        let result = lookup("kotlinx-coroutines").unwrap();
        assert!(result.is_some(), "expected Some for kotlinx-coroutines");
        let descriptor = result.unwrap();
        assert_eq!(descriptor.name, "kotlinx-coroutines");
        assert!(
            descriptor.maven.contains("{target}"),
            "maven template should contain {{target}} placeholder"
        );
        assert!(
            descriptor.maven.contains("{version}"),
            "maven template should contain {{version}} placeholder"
        );
    }

    #[test]
    fn lookup_unknown_library() {
        let result = lookup("nonexistent-lib").unwrap();
        assert!(result.is_none(), "expected None for nonexistent-lib");
    }

    #[test]
    fn resolve_coordinate_substitutes_placeholders() {
        let descriptor = lookup("kotlinx-coroutines").unwrap().unwrap();
        let target = Target::from_str("linux_x64").unwrap();
        let coord = resolve_coordinate(&descriptor, "1.8.0", &target).unwrap();

        assert_eq!(coord.group_id, "org.jetbrains.kotlinx");
        assert_eq!(coord.artifact_id, "kotlinx-coroutines-core-linuxx64");
        assert_eq!(coord.version, "1.8.0");
        assert_eq!(coord.packaging, "klib");
    }

    #[test]
    fn resolve_coordinate_macos_target() {
        let descriptor = lookup("kotlinx-coroutines").unwrap().unwrap();
        let target = Target::from_str("macos_arm64").unwrap();
        let coord = resolve_coordinate(&descriptor, "1.8.0", &target).unwrap();

        assert!(
            coord.artifact_id.contains("macosarm64"),
            "artifact_id should contain macosarm64, got: {}",
            coord.artifact_id
        );
    }

    #[test]
    fn resolve_coordinate_generates_valid_url() {
        let descriptor = lookup("kotlinx-coroutines").unwrap().unwrap();
        let target = Target::from_str("linux_x64").unwrap();
        let coord = resolve_coordinate(&descriptor, "1.8.0", &target).unwrap();
        let url = coord.to_url(MAVEN_CENTRAL);

        assert!(
            url.starts_with("https://repo1.maven.org/maven2/org/jetbrains/kotlinx/"),
            "url should start with Maven Central prefix, got: {url}"
        );
    }

    #[test]
    fn available_library_names_format() {
        let names = available_library_names().unwrap();
        assert!(
            names.contains("kotlinx-coroutines"),
            "available names should contain kotlinx-coroutines, got: {names}"
        );
        assert!(
            names.contains("kotlinx-datetime"),
            "available names should contain kotlinx-datetime, got: {names}"
        );
        assert!(
            names.contains("kotlinx-io"),
            "available names should contain kotlinx-io, got: {names}"
        );
        assert!(
            names.contains("kotlinx-atomicfu"),
            "available names should contain kotlinx-atomicfu, got: {names}"
        );
    }

    #[test]
    fn descriptor_rejects_unknown_fields() {
        let toml_str = r#"
name = "test-lib"
maven = "org.example:test:{version}:klib"
unknown_field = "should fail"
"#;
        let result: Result<LibraryDescriptor, _> = toml::from_str(toml_str);
        assert!(
            result.is_err(),
            "deny_unknown_fields should reject extra fields"
        );
    }
}
