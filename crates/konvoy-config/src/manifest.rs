use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The `konvoy.toml` project manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub package: Package,
    pub toolchain: Toolchain,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, DependencySpec>,
}

/// Toolchain specification declaring the Kotlin/Native version and optional tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    /// Kotlin/Native version, e.g. "2.1.0".
    pub kotlin: String,
    /// Detekt version, e.g. "1.23.7". When set, enables `konvoy lint`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detekt: Option<String>,
}

/// Whether this package produces an executable or a library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageKind {
    /// A native executable (default).
    #[default]
    Bin,
    /// A Kotlin/Native library (`.klib`).
    Lib,
}

/// Package metadata from the `[package]` section of `konvoy.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    #[serde(default)]
    pub kind: PackageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default = "default_entrypoint")]
    pub entrypoint: String,
}

/// Specification for a single dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencySpec {
    /// Path to the dependency project, relative to this manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Maven dependency version requirement (e.g. "1.8.0").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Maven coordinate in `groupId:artifactId` format (e.g. "org.jetbrains.kotlinx:kotlinx-coroutines-core").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maven: Option<String>,
}

fn default_entrypoint() -> String {
    "src/main.kt".to_owned()
}

/// Check whether a package name is valid: non-empty, starts with a letter or underscore,
/// and contains only ASCII alphanumeric characters, hyphens, or underscores.
fn is_valid_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Check whether an entrypoint path ends with `.kt`.
fn is_valid_entrypoint(entrypoint: &str) -> bool {
    entrypoint.ends_with(".kt")
}

/// Return `true` if `coord` is a valid `groupId:artifactId` Maven coordinate.
fn is_valid_maven_coordinate(coord: &str) -> bool {
    match coord.split_once(':') {
        Some((group, artifact)) => {
            !group.is_empty() && !artifact.is_empty() && !artifact.contains(':')
        }
        None => false,
    }
}

/// Validate plugin entries: must use Maven coordinates with a version, not path dependencies.
fn validate_plugins(
    plugins: &BTreeMap<String, DependencySpec>,
    kotlin_version: &str,
    path: &str,
) -> Result<(), ManifestError> {
    // Suppress unused-variable warning — `kotlin_version` is accepted for future
    // template-expansion validation but not needed by the current checks.
    let _ = kotlin_version;
    for (name, spec) in plugins {
        // Plugins must use Maven coordinates, not path dependencies.
        if spec.path.is_some() {
            return Err(ManifestError::InvalidPluginConfig {
                path: path.to_owned(),
                name: name.clone(),
                reason: "plugins must use `maven` coordinates, not `path`".to_owned(),
            });
        }
        if spec.maven.is_none() {
            return Err(ManifestError::InvalidPluginConfig {
                path: path.to_owned(),
                name: name.clone(),
                reason: "plugin must have `maven` set to a `groupId:artifactId` coordinate"
                    .to_owned(),
            });
        }
        if spec.version.is_none() {
            return Err(ManifestError::InvalidPluginConfig {
                path: path.to_owned(),
                name: name.clone(),
                reason: "plugin must have `version` set".to_owned(),
            });
        }
        if spec.version.as_ref().is_some_and(|v| v.trim().is_empty()) {
            return Err(ManifestError::InvalidPluginConfig {
                path: path.to_owned(),
                name: name.clone(),
                reason: "plugin `version` must not be empty or whitespace".to_owned(),
            });
        }
        // Validate maven coordinate format (reuse same colon-check as deps).
        if let Some(ref maven) = spec.maven {
            if !is_valid_maven_coordinate(maven) {
                return Err(ManifestError::InvalidPluginConfig {
                    path: path.to_owned(),
                    name: name.clone(),
                    reason: format!(
                        "invalid maven coordinate `{maven}` — expected `groupId:artifactId`"
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Validate dependency entries: names, source types, Maven coordinates, and versions.
fn validate_dependencies(
    dependencies: &BTreeMap<String, DependencySpec>,
    package_name: &str,
    path: &str,
) -> Result<(), ManifestError> {
    for (name, spec) in dependencies {
        if !is_valid_name(name) {
            return Err(ManifestError::DependencyInvalidName {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        if name == package_name {
            return Err(ManifestError::DependencySelfReference {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        // maven + path is an error — pick one source type.
        if spec.maven.is_some() && spec.path.is_some() {
            return Err(ManifestError::DependencyMavenWithPath {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        // version without maven is an error — needs a coordinate.
        if spec.version.is_some() && spec.maven.is_none() {
            return Err(ManifestError::DependencyVersionWithoutMaven {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        // maven without version is an error — needs a pinned version.
        if spec.maven.is_some() && spec.version.is_none() {
            return Err(ManifestError::DependencyMavenWithoutVersion {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        // Empty or whitespace-only version is an error.
        if spec.version.as_ref().is_some_and(|v| v.trim().is_empty()) {
            return Err(ManifestError::DependencyEmptyVersion {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        // Validate maven coordinate format: exactly one colon, non-empty parts.
        if let Some(ref maven) = spec.maven {
            if !is_valid_maven_coordinate(maven) {
                return Err(ManifestError::DependencyInvalidMaven {
                    path: path.to_owned(),
                    name: name.clone(),
                    maven: maven.clone(),
                });
            }
        }
        // No source at all — need path or maven+version.
        if spec.path.is_none() && spec.maven.is_none() && spec.version.is_none() {
            return Err(ManifestError::DependencyNoSource {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
    }
    Ok(())
}

/// Validate a parsed manifest and return validation errors.
fn validate(manifest: &Manifest, path: &str) -> Result<(), ManifestError> {
    if manifest.package.name.is_empty() {
        return Err(ManifestError::EmptyName {
            path: path.to_owned(),
        });
    }
    if !is_valid_name(&manifest.package.name) {
        return Err(ManifestError::InvalidName {
            path: path.to_owned(),
            name: manifest.package.name.clone(),
        });
    }
    // Only validate entrypoint for binary projects.
    if manifest.package.kind == PackageKind::Bin
        && !is_valid_entrypoint(&manifest.package.entrypoint)
    {
        return Err(ManifestError::InvalidEntrypoint {
            path: path.to_owned(),
            entrypoint: manifest.package.entrypoint.clone(),
        });
    }
    if manifest.toolchain.kotlin.is_empty() {
        return Err(ManifestError::InvalidToolchain {
            path: path.to_owned(),
            message: "kotlin version must not be empty".to_owned(),
        });
    }
    if manifest
        .toolchain
        .detekt
        .as_ref()
        .is_some_and(String::is_empty)
    {
        return Err(ManifestError::InvalidToolchain {
            path: path.to_owned(),
            message: "detekt version must not be empty".to_owned(),
        });
    }
    validate_plugins(&manifest.plugins, &manifest.toolchain.kotlin, path)?;
    validate_dependencies(&manifest.dependencies, &manifest.package.name, path)?;
    Ok(())
}

impl Manifest {
    /// Read and parse a `konvoy.toml` from the given path.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read, contains invalid TOML,
    /// has unknown keys, or fails validation (empty name, invalid characters,
    /// invalid entrypoint).
    pub fn from_path(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path).map_err(|e| ManifestError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
        Self::from_str(&content, &path.display().to_string())
    }

    /// Parse a manifest from a TOML string.
    ///
    /// # Errors
    /// Returns an error if the string contains invalid TOML, has unknown keys,
    /// or fails validation.
    pub fn from_str(content: &str, path: &str) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(content).map_err(|e| ManifestError::Parse {
            path: path.to_owned(),
            source: e,
        })?;
        validate(&manifest, path)?;
        Ok(manifest)
    }

    /// Serialize the manifest to a TOML string.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub fn to_toml(&self) -> Result<String, ManifestError> {
        toml::to_string_pretty(self).map_err(|e| ManifestError::Serialize { source: e })
    }
}

/// Errors produced when reading, parsing, or validating a `konvoy.toml` manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid konvoy.toml at {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("cannot serialize manifest: {source}")]
    Serialize { source: toml::ser::Error },
    #[error("package name must not be empty in {path}")]
    EmptyName { path: String },
    #[error("package name `{name}` contains invalid characters in {path} (only alphanumeric, hyphen, underscore allowed)")]
    InvalidName { path: String, name: String },
    #[error("entrypoint `{entrypoint}` must end with .kt in {path}")]
    InvalidEntrypoint { path: String, entrypoint: String },
    #[error("invalid [toolchain] in {path}: {message}")]
    InvalidToolchain { path: String, message: String },
    #[error("dependency `{name}` has no source (set `path` or `maven` + `version`) in {path}")]
    DependencyNoSource { path: String, name: String },
    #[error("dependency `{name}` has both `maven` and `path` set in {path} — use exactly one")]
    DependencyMavenWithPath { path: String, name: String },
    #[error(
        "dependency `{name}` has `maven` without `version` in {path} — add `version = \"X.Y.Z\"`"
    )]
    DependencyMavenWithoutVersion { path: String, name: String },
    #[error("dependency `{name}` has `version` without `maven` coordinate in {path} — add `maven = \"groupId:artifactId\"`")]
    DependencyVersionWithoutMaven { path: String, name: String },
    #[error("dependency `{name}` version must not be empty or whitespace in {path}")]
    DependencyEmptyVersion { path: String, name: String },
    #[error("dependency `{name}` has invalid maven coordinate `{maven}` in {path} — expected format `groupId:artifactId` (exactly one colon)")]
    DependencyInvalidMaven {
        path: String,
        name: String,
        maven: String,
    },
    #[error("dependency name `{name}` contains invalid characters in {path}")]
    DependencyInvalidName { path: String, name: String },
    #[error("dependency `{name}` references itself in {path}")]
    DependencySelfReference { path: String, name: String },
    #[error("invalid plugin `{name}` in {path}: {reason}")]
    InvalidPluginConfig {
        path: String,
        name: String,
        reason: String,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const TOOLCHAIN: &str = "\n[toolchain]\nkotlin = \"2.1.0\"\n";

    #[test]
    fn parse_valid_manifest() {
        let toml = format!(
            r#"
[package]
name = "my-project"
entrypoint = "src/main.kt"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml");
        assert!(manifest.is_ok());
        let manifest = manifest.unwrap();
        assert_eq!(manifest.package.name, "my-project");
        assert_eq!(manifest.package.entrypoint, "src/main.kt");
        assert_eq!(manifest.toolchain.kotlin, "2.1.0");
    }

    #[test]
    fn parse_minimal_manifest() {
        let toml = format!(
            r#"
[package]
name = "minimal"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml");
        assert!(manifest.is_ok());
        let manifest = manifest.unwrap();
        assert_eq!(manifest.package.name, "minimal");
        assert_eq!(manifest.package.entrypoint, "src/main.kt");
    }

    #[test]
    fn reject_missing_toolchain() {
        let toml = r#"
[package]
name = "no-toolchain"
"#;
        let result = Manifest::from_str(toml, "konvoy.toml");
        assert!(result.is_err());
    }

    #[test]
    fn reject_empty_kotlin_version() {
        let toml = r#"
[package]
name = "empty-ver"

[toolchain]
kotlin = ""
"#;
        let result = Manifest::from_str(toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"), "error was: {err}");
    }

    #[test]
    fn reject_missing_package() {
        let toml = r#"
[other]
key = "value"
"#;
        let result = Manifest::from_str(toml, "konvoy.toml");
        assert!(result.is_err());
    }

    #[test]
    fn reject_empty_name() {
        let toml = format!(
            r#"
[package]
name = ""
{TOOLCHAIN}"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty"), "error was: {err}");
    }

    #[test]
    fn reject_invalid_name_chars() {
        let toml = format!(
            r#"
[package]
name = "my project!"
{TOOLCHAIN}"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid characters"), "error was: {err}");
    }

    #[test]
    fn reject_invalid_entrypoint() {
        let toml = format!(
            r#"
[package]
name = "my-project"
entrypoint = "src/main.java"
{TOOLCHAIN}"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains(".kt"), "error was: {err}");
    }

    #[test]
    fn reject_unknown_keys() {
        let toml = format!(
            r#"
[package]
name = "my-project"
unknown_field = true
{TOOLCHAIN}"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
    }

    #[test]
    fn round_trip() {
        let toml = format!(
            r#"
[package]
name = "round-trip"
entrypoint = "src/app.kt"
{TOOLCHAIN}"#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn valid_name_chars() {
        assert!(is_valid_name("hello"));
        assert!(is_valid_name("hello-world"));
        assert!(is_valid_name("hello_world"));
        assert!(is_valid_name("Hello123"));
        assert!(is_valid_name("_leading_underscore"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("hello world"));
        assert!(!is_valid_name("hello!"));
        assert!(!is_valid_name("hello.world"));
        assert!(!is_valid_name("1abc"));
        assert!(!is_valid_name("-leading-hyphen"));
    }

    #[test]
    fn parse_lib_manifest() {
        let toml = format!(
            r#"
[package]
name = "my-lib"
kind = "lib"
version = "0.1.0"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.package.kind, PackageKind::Lib);
        assert_eq!(manifest.package.version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn default_kind_is_bin() {
        let toml = format!(
            r#"
[package]
name = "implicit-bin"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.package.kind, PackageKind::Bin);
    }

    #[test]
    fn lib_skips_entrypoint_validation() {
        let toml = format!(
            r#"
[package]
name = "my-lib"
kind = "lib"
entrypoint = "not-a-kt-file"
{TOOLCHAIN}"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_dependencies() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
my-utils = {{ path = "../my-utils" }}
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.dependencies.len(), 1);
        let dep = manifest
            .dependencies
            .get("my-utils")
            .unwrap_or_else(|| panic!("missing dep"));
        assert_eq!(dep.path.as_deref(), Some("../my-utils"));
    }

    #[test]
    fn reject_dependency_without_source() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{}}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no source"), "error was: {err}");
    }

    #[test]
    fn reject_self_referencing_dependency() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
my-app = {{ path = "." }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("itself"), "error was: {err}");
    }

    #[test]
    fn reject_invalid_dependency_name() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
"bad dep!" = {{ path = "../bad" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid characters"), "error was: {err}");
    }

    #[test]
    fn round_trip_with_deps() {
        let toml = format!(
            r#"
[package]
name = "with-deps"
{TOOLCHAIN}
[dependencies]
utils = {{ path = "../utils" }}
"#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn no_deps_omitted_in_toml() {
        let toml = format!(
            r#"
[package]
name = "no-deps"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = manifest.to_toml().unwrap();
        assert!(
            !serialized.contains("[dependencies]"),
            "serialized was: {serialized}"
        );
    }

    #[test]
    fn parse_manifest_with_detekt() {
        let toml = r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
detekt = "1.23.7"
"#;
        let manifest = Manifest::from_str(toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.toolchain.detekt.as_deref(), Some("1.23.7"));
    }

    #[test]
    fn parse_manifest_without_detekt() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert!(manifest.toolchain.detekt.is_none());
    }

    #[test]
    fn parse_toolchain_detekt_default_omitted() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert!(manifest.toolchain.detekt.is_none());
        assert_eq!(manifest.toolchain.kotlin, "2.1.0");
    }

    #[test]
    fn reject_lint_section() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[lint]
detekt = "1.23.7"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err(), "expected [lint] section to be rejected");
    }

    #[test]
    fn round_trip_with_detekt() {
        let toml = r#"
[package]
name = "with-detekt"

[toolchain]
kotlin = "2.1.0"
detekt = "1.23.7"
"#;
        let original = Manifest::from_str(toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn round_trip_toolchain_without_detekt() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}"#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        assert!(
            !serialized.contains("detekt"),
            "serialized should not contain detekt: {serialized}"
        );
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn no_detekt_omitted_in_toml() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = manifest.to_toml().unwrap();
        assert!(
            !serialized.contains("detekt"),
            "serialized was: {serialized}"
        );
    }

    #[test]
    fn reject_empty_detekt_version() {
        let toml = r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
detekt = ""
"#;
        let result = Manifest::from_str(toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("detekt version must not be empty"),
            "error was: {err}"
        );
    }

    #[test]
    fn toolchain_rejects_unknown_fields() {
        let toml = r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
unknown = true
"#;
        let result = Manifest::from_str(toml, "konvoy.toml");
        assert!(result.is_err());
    }

    #[test]
    fn parse_manifest_with_plugins() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "{{kotlin}}"
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.plugins.len(), 1);
        let plugin = manifest
            .plugins
            .get("kotlin-serialization")
            .unwrap_or_else(|| panic!("missing plugin"));
        assert_eq!(
            plugin.maven.as_deref(),
            Some("org.jetbrains.kotlin:kotlin-serialization-compiler-plugin")
        );
        assert_eq!(plugin.version.as_deref(), Some("{kotlin}"));
    }

    #[test]
    fn parse_manifest_without_plugins() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert!(manifest.plugins.is_empty());
    }

    #[test]
    fn reject_plugin_without_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("version"), "error was: {err}");
    }

    #[test]
    fn reject_plugin_without_maven() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
version = "1.8.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("maven"), "error was: {err}");
    }

    #[test]
    fn reject_plugin_with_path() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my-plugin]
path = "../plugin"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("plugins must use `maven`"), "error was: {err}");
    }

    #[test]
    fn reject_plugin_invalid_maven_coordinate() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.bad-plugin]
maven = "nocolon"
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn plugin_accepts_kotlin_placeholder_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "{{kotlin}}"
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let plugin = manifest
            .plugins
            .get("kotlin-serialization")
            .unwrap_or_else(|| panic!("missing plugin"));
        assert_eq!(plugin.version.as_deref(), Some("{kotlin}"));
    }

    #[test]
    fn reject_plugin_maven_too_many_colons() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.bad-plugin]
maven = "a:b:c"
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn reject_plugin_maven_leading_colon() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.bad-plugin]
maven = ":artifact"
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn reject_plugin_maven_trailing_colon() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.bad-plugin]
maven = "group:"
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn empty_plugins_table_parses() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins]
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert!(manifest.plugins.is_empty());
    }

    #[test]
    fn multiple_plugins_parse() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins]
kotlin-serialization = {{ maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{{kotlin}}" }}
kotlin-allopen = {{ maven = "org.jetbrains.kotlin:kotlin-allopen-compiler-plugin", version = "{{kotlin}}" }}
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.plugins.len(), 2);
        assert!(manifest.plugins.contains_key("kotlin-serialization"));
        assert!(manifest.plugins.contains_key("kotlin-allopen"));
    }

    #[test]
    fn plugin_name_with_underscores() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my_custom_plugin]
maven = "com.example:my-plugin"
version = "1.0.0"
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert!(manifest.plugins.contains_key("my_custom_plugin"));
    }

    #[test]
    fn plugins_without_dependencies_section() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "{{kotlin}}"
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.plugins.len(), 1);
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn plugin_error_message_is_actionable() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.bad-plugin]
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Error should mention what field is needed.
        assert!(
            err.contains("maven") && err.contains("groupId:artifactId"),
            "error should be actionable and mention expected format: {err}"
        );
    }

    #[test]
    fn plugin_path_error_message_is_actionable() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my-plugin]
path = "../plugin"
maven = "com.example:plugin"
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("plugins must use `maven`") && err.contains("not `path`"),
            "error should explain what to do: {err}"
        );
    }

    #[test]
    fn plugin_version_error_message_is_actionable() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my-plugin]
maven = "com.example:plugin"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("version"),
            "error should mention missing version: {err}"
        );
    }

    #[test]
    fn reject_unknown_plugin_fields() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "1.8.0"
unknown_field = true
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
    }

    #[test]
    fn round_trip_with_plugins() {
        let toml = format!(
            r#"
[package]
name = "with-plugins"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "1.8.0"
"#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn no_plugins_omitted_in_toml() {
        let toml = format!(
            r#"
[package]
name = "no-plugins"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = manifest.to_toml().unwrap();
        assert!(
            !serialized.contains("[plugins]"),
            "serialized was: {serialized}"
        );
    }

    #[test]
    fn parse_maven_dependency() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
kotlinx-coroutines = {{ maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }}
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(manifest.dependencies.len(), 1);
        let dep = manifest
            .dependencies
            .get("kotlinx-coroutines")
            .unwrap_or_else(|| panic!("missing dep"));
        assert_eq!(dep.version.as_deref(), Some("1.8.0"));
        assert_eq!(
            dep.maven.as_deref(),
            Some("org.jetbrains.kotlinx:kotlinx-coroutines-core")
        );
        assert!(dep.path.is_none());
    }

    #[test]
    fn reject_dependency_maven_with_path() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{ maven = "com.example:lib", version = "1.0", path = "../x" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("maven") && err.contains("path"),
            "error was: {err}"
        );
    }

    #[test]
    fn reject_dependency_empty_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{ maven = "com.example:lib", version = "" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty or whitespace"),
            "error should mention empty/whitespace: {err}"
        );
    }

    #[test]
    fn reject_dependency_whitespace_only_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{ maven = "com.example:lib", version = "  " }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty or whitespace"),
            "error should mention empty/whitespace: {err}"
        );
    }

    #[test]
    fn round_trip_with_maven_dep() {
        let toml = format!(
            r#"
[package]
name = "with-maven"
{TOOLCHAIN}
[dependencies]
kotlinx-coroutines = {{ maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }}
"#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        let serialized = original.to_toml().unwrap_or_else(|e| panic!("{e}"));
        let reparsed =
            Manifest::from_str(&serialized, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(original, reparsed);
    }

    #[test]
    fn mixed_path_and_maven_deps() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
my-local-lib = {{ path = "../my-lib" }}
kotlinx-coroutines = {{ maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }}
kotlinx-datetime = {{ maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }}
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(manifest.dependencies.len(), 3);
        let local = manifest
            .dependencies
            .get("my-local-lib")
            .unwrap_or_else(|| panic!("missing local dep"));
        assert_eq!(local.path.as_deref(), Some("../my-lib"));
        assert!(local.version.is_none());
        assert!(local.maven.is_none());
        let coroutines = manifest
            .dependencies
            .get("kotlinx-coroutines")
            .unwrap_or_else(|| panic!("missing coroutines dep"));
        assert_eq!(coroutines.version.as_deref(), Some("1.8.0"));
        assert_eq!(
            coroutines.maven.as_deref(),
            Some("org.jetbrains.kotlinx:kotlinx-coroutines-core")
        );
        assert!(coroutines.path.is_none());
        let datetime = manifest
            .dependencies
            .get("kotlinx-datetime")
            .unwrap_or_else(|| panic!("missing datetime dep"));
        assert_eq!(datetime.version.as_deref(), Some("0.6.0"));
        assert_eq!(
            datetime.maven.as_deref(),
            Some("org.jetbrains.kotlinx:kotlinx-datetime")
        );
        assert!(datetime.path.is_none());
    }

    #[test]
    fn reject_version_without_maven() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
some-lib = {{ version = "1.0.0" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("version") && err.contains("maven"),
            "error was: {err}"
        );
        assert!(
            err.contains("add `maven = \"groupId:artifactId\"`"),
            "error should be actionable: {err}"
        );
    }

    #[test]
    fn reject_maven_without_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
some-lib = {{ maven = "com.example:lib" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("maven") && err.contains("version"),
            "error was: {err}"
        );
    }

    #[test]
    fn reject_maven_no_colon() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{ maven = "nocolon", version = "1.0" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn reject_maven_multiple_colons() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{ maven = "a:b:c", version = "1.0" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn reject_maven_leading_colon() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{ maven = ":artifact", version = "1.0" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn reject_maven_trailing_colon() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{ maven = "group:", version = "1.0" }}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn reject_dependency_no_source_updated_message() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
bad-dep = {{}}
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no source"), "error was: {err}");
        assert!(err.contains("maven"), "error should mention maven: {err}");
    }

    #[test]
    fn reject_plugin_empty_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my-plugin]
maven = "com.example:plugin"
version = ""
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty or whitespace"),
            "error should mention empty/whitespace: {err}"
        );
    }

    #[test]
    fn reject_plugin_whitespace_only_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my-plugin]
maven = "com.example:plugin"
version = "   "
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty or whitespace"),
            "error should mention empty/whitespace: {err}"
        );
    }

    #[test]
    fn reject_plugin_with_both_path_and_maven() {
        // A plugin with both path and maven should be rejected (path check comes first).
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my-plugin]
path = "../plugin"
maven = "com.example:plugin"
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("plugins must use `maven`"), "error was: {err}");
    }

    #[test]
    fn plugins_and_dependencies_both_present() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "{{kotlin}}"

[dependencies]
kotlinx-coroutines = {{ maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }}
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.plugins.len(), 1);
        assert_eq!(manifest.dependencies.len(), 1);
        assert!(manifest.plugins.contains_key("kotlin-serialization"));
        assert!(manifest.dependencies.contains_key("kotlinx-coroutines"));
    }

    #[test]
    fn plugins_only_no_dependencies() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.kotlin-serialization]
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "{{kotlin}}"
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(manifest.plugins.len(), 1);
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn round_trip_with_both_plugins_and_deps() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[dependencies]
kotlinx-coroutines = {{ maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }}

[plugins]
kotlin-serialization = {{ maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{{kotlin}}" }}
"#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
        assert_eq!(reparsed.plugins.len(), 1);
        assert_eq!(reparsed.dependencies.len(), 1);
    }

    #[test]
    fn plugin_maven_coordinate_with_dots_in_group() {
        // Maven coordinates with deeply nested groups should be valid.
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.my-plugin]
maven = "com.very.deep.package.name:my-artifact"
version = "1.0.0"
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let plugin = manifest.plugins.get("my-plugin").unwrap();
        assert_eq!(
            plugin.maven.as_deref(),
            Some("com.very.deep.package.name:my-artifact")
        );
    }

    #[test]
    fn reject_plugin_maven_only_colon() {
        // A maven coordinate that is just ":" should be rejected.
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[plugins.bad-plugin]
maven = ":"
version = "1.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            #[allow(clippy::unwrap_used)]
            fn valid_name_parses(name in "[a-zA-Z][a-zA-Z0-9_-]{0,30}") {
                let toml_str = format!(
                    "[package]\nname = \"{name}\"\n{TOOLCHAIN}"
                );
                let result = Manifest::from_str(&toml_str, "test.toml");
                prop_assert!(result.is_ok(), "expected valid name to parse: {}", name);
            }

            #[test]
            #[allow(clippy::unwrap_used)]
            fn invalid_name_chars_rejected(
                prefix in "[a-zA-Z][a-zA-Z0-9_-]{0,10}",
                bad_char in "[^a-zA-Z0-9_\\-\"\\\\\\n\\r\\t]",
                suffix in "[a-zA-Z0-9_-]{0,10}"
            ) {
                let name = format!("{prefix}{bad_char}{suffix}");
                let toml_str = format!(
                    "[package]\nname = \"{name}\"\n{TOOLCHAIN}"
                );
                let result = Manifest::from_str(&toml_str, "test.toml");
                prop_assert!(result.is_err(), "expected invalid name to be rejected: {}", name);
            }

            #[test]
            #[allow(clippy::unwrap_used)]
            fn entrypoint_validation_never_panics(entry in "[a-zA-Z0-9_./ ]{1,50}") {
                let toml_str = format!(
                    "[package]\nname = \"test-proj\"\nentrypoint = \"{entry}\"\n{TOOLCHAIN}"
                );
                let _ = Manifest::from_str(&toml_str, "test.toml");
            }

            #[test]
            #[allow(clippy::unwrap_used)]
            fn manifest_round_trip(
                name in "[a-zA-Z][a-zA-Z0-9_-]{0,20}",
                version in "[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}",
                kotlin_ver in "[0-9]{1,2}\\.[0-9]{1,2}\\.[0-9]{1,2}",
                entrypoint in "src/[a-zA-Z][a-zA-Z0-9_]{0,15}\\.kt",
            ) {
                let toml_str = format!(
                    "[package]\nname = \"{name}\"\nversion = \"{version}\"\nentrypoint = \"{entrypoint}\"\n\n[toolchain]\nkotlin = \"{kotlin_ver}\"\n"
                );
                let original = Manifest::from_str(&toml_str, "test.toml").unwrap();
                let serialized = original.to_toml().unwrap();
                let reparsed = Manifest::from_str(&serialized, "test.toml").unwrap();
                prop_assert_eq!(original, reparsed);
            }
        }
    }
}
