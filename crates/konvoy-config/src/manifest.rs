use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The `konvoy.toml` project manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub package: Package,
    pub toolchain: Toolchain,
    #[serde(default, skip_serializing_if = "Codegen::is_empty")]
    pub codegen: Codegen,
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

/// Code generation tools configured for this project.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Codegen {
    /// OpenAPI code generation using Fabrikt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openapi: Option<OpenApiCodegen>,
}

impl Codegen {
    /// Return `true` when no code generators are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.openapi.is_none()
    }
}

/// OpenAPI code generation configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenApiCodegen {
    /// Fabrikt version to use.
    pub version: String,
    /// Project-relative path to the OpenAPI spec file.
    pub spec: String,
    /// Kotlin package name for generated sources.
    pub base_package: String,
    /// Additional project-relative directories to hash for cache invalidation,
    /// beyond the main `spec` file.
    ///
    /// Optional — defaults to empty, in which case only the `spec` file is
    /// tracked. Fabrikt resolves `$ref`'d sibling files internally but never
    /// reports which files it read, so Konvoy cannot discover them. When the spec
    /// splits across multiple files, list the directories holding them here: any
    /// change to any file under them regenerates sources. This is fully
    /// user-defined — Konvoy never assumes a directory. It deliberately
    /// over-approximates (a change to an unrelated file in a listed directory
    /// also regenerates) rather than re-parsing the spec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_spec_dirs: Vec<String>,
}

impl DependencySpec {
    /// Returns `true` if this is a Maven dependency (has both `maven` and `version` set).
    pub fn is_maven(&self) -> bool {
        self.as_maven_coord().is_some()
    }

    /// Return `(maven_coord, version)` if this spec is a complete Maven dependency.
    pub fn as_maven_coord(&self) -> Option<(&str, &str)> {
        match (&self.maven, &self.version) {
            (Some(maven), Some(version)) => Some((maven.as_str(), version.as_str())),
            _ => None,
        }
    }
}

fn default_entrypoint() -> String {
    "src/main.kt".to_owned()
}

/// Validate a package/project name, returning the failure reason on error.
///
/// A valid name is non-empty, starts with a letter or underscore, and contains
/// only ASCII alphanumeric characters, hyphens, or underscores.
///
/// # Errors
///
/// Returns a human-readable reason string if the name is invalid.
pub fn validate_name(name: &str) -> Result<(), String> {
    match name.chars().next() {
        None => return Err("name must not be empty".to_owned()),
        Some(first) if !first.is_ascii_alphabetic() && first != '_' => {
            return Err(format!(
                "must start with a letter or underscore, found '{first}'"
            ));
        }
        _ => {}
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '-' && *c != '_')
    {
        return Err(format!(
            "contains invalid character '{bad}' — only ASCII letters, digits, hyphens, and underscores are allowed"
        ));
    }
    Ok(())
}

/// Check whether a package name is valid (convenience wrapper around [`validate_name`]).
pub fn is_valid_name(name: &str) -> bool {
    validate_name(name).is_ok()
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
    path: &str,
) -> Result<(), ManifestError> {
    for (name, spec) in plugins {
        let err = |reason: String| ManifestError::InvalidPluginConfig {
            path: path.to_owned(),
            name: name.clone(),
            reason,
        };

        if spec.path.is_some() {
            return Err(err(
                "plugins must use `maven` coordinates, not `path`".to_owned()
            ));
        }
        if spec.maven.is_none() {
            return Err(err(
                "plugin must have `maven` set to a `groupId:artifactId` coordinate".to_owned(),
            ));
        }
        if spec.version.is_none() {
            return Err(err("plugin must have `version` set".to_owned()));
        }
        if spec.version.as_ref().is_some_and(|v| v.trim().is_empty()) {
            return Err(err(
                "plugin `version` must not be empty or whitespace".to_owned()
            ));
        }
        if let Some(ref maven) = spec.maven {
            if !is_valid_maven_coordinate(maven) {
                return Err(err(format!(
                    "invalid maven coordinate `{maven}` — expected `groupId:artifactId`"
                )));
            }
        }
    }
    Ok(())
}

/// Ensure a configured path stays inside the project tree: relative and free of
/// `..` traversal. `label` names the field for the error message.
fn check_project_relative(value: &str, label: &str) -> Result<(), String> {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return Err(format!(
            "{label} must be a relative path inside the project"
        ));
    }
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "{label} must be a relative path inside the project (must not contain `..`)"
        ));
    }
    Ok(())
}

fn validate_codegen(codegen: &Codegen, path: &str) -> Result<(), ManifestError> {
    let Some(openapi) = &codegen.openapi else {
        return Ok(());
    };

    let err = |reason: String| ManifestError::InvalidCodegenConfig {
        path: path.to_owned(),
        name: "openapi".to_owned(),
        reason,
    };

    // Codegen fields are already normalized (trimmed) in `Manifest::from_str`
    // before validation runs, so we read them as-is here.
    let version = openapi.version.as_str();
    if version.is_empty() {
        return Err(err("version must not be empty".to_owned()));
    }
    // Konvoy always passes Fabrikt's `--serialization-library` flag, which only
    // exists in Fabrikt 18.0.0 and newer. Reject older or unparseable pins up
    // front with an actionable message rather than letting the JAR fail
    // cryptically at generation time.
    match fabrikt_major_version(version) {
        Some(major) if major >= MIN_FABRIKT_MAJOR => {}
        Some(_) => {
            return Err(err(format!(
                "Fabrikt version `{version}` is not supported — \
                 Konvoy requires Fabrikt {MIN_FABRIKT_MAJOR}.0.0 or newer"
            )));
        }
        None => {
            return Err(err(format!(
                "version `{version}` is not a valid Fabrikt version — \
                 expected a semver like `20.0.0`"
            )));
        }
    }

    let spec = openapi.spec.as_str();
    if spec.is_empty() {
        return Err(err("spec must not be empty".to_owned()));
    }
    // Reject absolute paths and parent-directory traversal so the spec genuinely
    // stays inside the project (the error messages promise this) and so the cache
    // key never depends on files outside the project tree.
    if let Err(reason) = check_project_relative(spec, "spec") {
        return Err(err(reason));
    }
    // Extension match is case-insensitive: Fabrikt reads the file by content, not
    // extension, so a spec named `API.YAML` is valid and should not be rejected.
    let spec_ext = spec.to_ascii_lowercase();
    if !(spec_ext.ends_with(".yaml") || spec_ext.ends_with(".yml") || spec_ext.ends_with(".json")) {
        return Err(err(
            "spec must point to an OpenAPI .yaml, .yml, or .json file".to_owned(),
        ));
    }

    let base_package = openapi.base_package.as_str();
    if base_package.is_empty() {
        return Err(err("base_package must not be empty".to_owned()));
    }
    if !konvoy_util::naming::is_valid_package_name(base_package) {
        return Err(err(format!(
            "base_package `{base_package}` is not a valid package name — \
             use dot-separated identifiers like `com.example.api`"
        )));
    }

    // Each extra spec directory feeds the codegen cache key, so it must also stay
    // inside the project tree (same rules as `spec`).
    for dir in &openapi.extra_spec_dirs {
        if dir.is_empty() {
            return Err(err("extra_spec_dirs entries must not be empty".to_owned()));
        }
        if let Err(reason) = check_project_relative(dir, "extra_spec_dirs entry") {
            return Err(err(reason));
        }
    }

    Ok(())
}

/// Minimum Fabrikt major version Konvoy supports (the version that introduced
/// the `--serialization-library` flag Konvoy always passes).
const MIN_FABRIKT_MAJOR: u64 = 18;

/// Parse the leading major-version number from a Fabrikt version string.
///
/// Returns `None` when the string does not start with a numeric major
/// component (e.g. `"latest"` or an empty major).
fn fabrikt_major_version(version: &str) -> Option<u64> {
    version.split(['.', '-', '+']).next()?.parse::<u64>().ok()
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
    validate_plugins(&manifest.plugins, path)?;
    validate_codegen(&manifest.codegen, path)?;
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
        let mut manifest: Manifest = toml::from_str(content).map_err(|e| ManifestError::Parse {
            path: path.to_owned(),
            source: e,
        })?;
        // Normalize codegen fields (trim whitespace) once, here, so the stored
        // values are what the rest of the pipeline uses — e.g. the Maven
        // coordinate built from `version` — and so `validate_codegen` can read
        // them as-is. Without this, version = " 20.0.0 " would pass validation
        // but the untrimmed string would flow into the Maven coordinate and 404
        // at download.
        if let Some(openapi) = manifest.codegen.openapi.as_mut() {
            openapi.version = openapi.version.trim().to_owned();
            openapi.spec = openapi.spec.trim().to_owned();
            openapi.base_package = openapi.base_package.trim().to_owned();
            for dir in &mut openapi.extra_spec_dirs {
                *dir = dir.trim().to_owned();
            }
        }
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
    #[error("invalid codegen `{name}` in {path}: {reason}")]
    InvalidCodegenConfig {
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
    fn parse_manifest_with_openapi_codegen() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api""#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let openapi = manifest
            .codegen
            .openapi
            .as_ref()
            .unwrap_or_else(|| panic!("missing openapi codegen config"));
        assert_eq!(openapi.version, "20.0.0");
        assert_eq!(openapi.spec, "specs/api.yaml");
        assert_eq!(openapi.base_package, "com.example.api");
        assert!(!manifest.codegen.is_empty());
    }

    #[test]
    fn parse_manifest_without_codegen() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert!(manifest.codegen.openapi.is_none());
        assert!(manifest.codegen.is_empty());
    }

    #[test]
    fn reject_unknown_codegen_tool() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.grpc]
version = "1.0.0"
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err(), "unknown codegen tools should be rejected");
    }

    #[test]
    fn reject_openapi_codegen_empty_fields() {
        for (field, toml) in [
            (
                "version",
                format!(
                    r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = ""
spec = "specs/api.yaml"
base_package = "com.example.api""#
                ),
            ),
            (
                "spec",
                format!(
                    r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = ""
base_package = "com.example.api""#
                ),
            ),
            (
                "base_package",
                format!(
                    r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = """#
                ),
            ),
        ] {
            let result = Manifest::from_str(&toml, "konvoy.toml");
            assert!(result.is_err(), "{field} should be rejected");
            let err = result.unwrap_err().to_string();
            assert!(err.contains(field), "error was: {err}");
        }
    }

    #[test]
    fn reject_openapi_codegen_absolute_spec_path() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "/tmp/api.yaml"
base_package = "com.example.api""#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("relative"), "error was: {err}");
    }

    #[test]
    fn reject_openapi_codegen_unsupported_spec_extension() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.txt"
base_package = "com.example.api""#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains(".yaml"), "error was: {err}");
    }

    #[test]
    fn accept_openapi_codegen_spec_extensions() {
        // .yaml/.yml/.json are all accepted, case-insensitively.
        for spec in [
            "specs/api.yaml",
            "specs/api.yml",
            "specs/api.json",
            "specs/api.YAML",
            "specs/api.JSON",
        ] {
            let toml = format!(
                r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "{spec}"
base_package = "com.example.api""#
            );
            let result = Manifest::from_str(&toml, "konvoy.toml");
            assert!(result.is_ok(), "`{spec}` should be accepted: {result:?}");
        }
    }

    #[test]
    fn openapi_codegen_fields_are_trimmed() {
        // Surrounding whitespace is stripped at parse time so the stored values
        // (used downstream for the Maven coordinate, paths, etc.) are clean.
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = " 20.0.0 "
spec = " specs/api.yaml "
base_package = " com.example.api ""#
        );
        let openapi = Manifest::from_str(&toml, "konvoy.toml")
            .unwrap()
            .codegen
            .openapi
            .unwrap_or_else(|| panic!("missing openapi codegen config"));
        assert_eq!(openapi.version, "20.0.0");
        assert_eq!(openapi.spec, "specs/api.yaml");
        assert_eq!(openapi.base_package, "com.example.api");
    }

    #[test]
    fn reject_openapi_codegen_spec_parent_traversal() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "../outside/api.yaml"
base_package = "com.example.api""#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains(".."), "error was: {err}");
    }

    #[test]
    fn reject_openapi_codegen_version_below_floor() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "17.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api""#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("18"), "error was: {err}");
    }

    #[test]
    fn reject_openapi_codegen_non_numeric_version() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "latest"
spec = "specs/api.yaml"
base_package = "com.example.api""#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("valid Fabrikt version"), "error was: {err}");
    }

    #[test]
    fn reject_openapi_codegen_invalid_base_package() {
        for bad in ["com..example", "com.123abc", "-com.example", "com.exa mple"] {
            let toml = format!(
                r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "{bad}""#
            );
            let result = Manifest::from_str(&toml, "konvoy.toml");
            assert!(result.is_err(), "`{bad}` should be rejected");
            let err = result.unwrap_err().to_string();
            assert!(err.contains("package name"), "error was: {err}");
        }
    }

    #[test]
    fn accept_openapi_codegen_underscore_base_package() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example._internal.api2""#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(
            manifest
                .codegen
                .openapi
                .as_ref()
                .map(|o| o.base_package.as_str()),
            Some("com.example._internal.api2")
        );
    }

    #[test]
    fn round_trip_with_openapi_codegen() {
        let toml = format!(
            r#"
[package]
name = "with-codegen"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.json"
base_package = "com.example.api""#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        assert!(serialized.contains("[codegen.openapi]"));
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn parse_manifest_with_openapi_extra_spec_dirs() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
extra_spec_dirs = ["specs", "shared/models"]
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let openapi = manifest
            .codegen
            .openapi
            .as_ref()
            .unwrap_or_else(|| panic!("missing openapi codegen config"));
        assert_eq!(openapi.extra_spec_dirs, vec!["specs", "shared/models"]);
    }

    #[test]
    fn openapi_extra_spec_dirs_optional_defaults_empty() {
        // extra_spec_dirs is optional: omitting it defaults to empty (only the
        // primary `spec` is tracked), and an empty list is not serialized back.
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert!(manifest
            .codegen
            .openapi
            .as_ref()
            .map(|o| o.extra_spec_dirs.is_empty())
            .unwrap_or(false));
        // Empty extra_spec_dirs is omitted on serialize (skip_serializing_if).
        let serialized = manifest.to_toml().unwrap();
        assert!(
            !serialized.contains("extra_spec_dirs"),
            "empty extra_spec_dirs must not be serialized: {serialized}"
        );
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(manifest, reparsed);
    }

    #[test]
    fn openapi_extra_spec_dirs_entries_are_trimmed() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
extra_spec_dirs = ["  specs  "]
"#
        );
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        assert_eq!(
            manifest
                .codegen
                .openapi
                .as_ref()
                .map(|o| &o.extra_spec_dirs),
            Some(&vec!["specs".to_owned()])
        );
    }

    #[test]
    fn reject_openapi_extra_spec_dir_absolute() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
extra_spec_dirs = ["/etc"]
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("relative"), "error was: {err}");
    }

    #[test]
    fn reject_openapi_extra_spec_dir_parent_traversal() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
extra_spec_dirs = ["../outside"]
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains(".."), "error was: {err}");
    }

    #[test]
    fn reject_openapi_extra_spec_dir_empty() {
        let toml = format!(
            r#"
[package]
name = "my-app"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
extra_spec_dirs = ["specs", ""]
"#
        );
        let result = Manifest::from_str(&toml, "konvoy.toml");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("must not be empty"), "error was: {err}");
    }

    #[test]
    fn round_trip_with_openapi_extra_spec_dirs() {
        let toml = format!(
            r#"
[package]
name = "with-codegen"
{TOOLCHAIN}
[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
extra_spec_dirs = ["specs", "shared"]
"#
        );
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap();
        let serialized = original.to_toml().unwrap();
        assert!(serialized.contains("extra_spec_dirs"));
        let reparsed = Manifest::from_str(&serialized, "konvoy.toml").unwrap();
        assert_eq!(original, reparsed);
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

    #[test]
    fn is_maven_true_when_both_set() {
        let spec = DependencySpec {
            path: None,
            maven: Some("org.example:lib".to_owned()),
            version: Some("1.0.0".to_owned()),
        };
        assert!(spec.is_maven());
    }

    #[test]
    fn is_maven_false_when_maven_only() {
        let spec = DependencySpec {
            path: None,
            maven: Some("org.example:lib".to_owned()),
            version: None,
        };
        assert!(!spec.is_maven());
    }

    #[test]
    fn is_maven_false_when_version_only() {
        let spec = DependencySpec {
            path: None,
            maven: None,
            version: Some("1.0.0".to_owned()),
        };
        assert!(!spec.is_maven());
    }

    #[test]
    fn is_maven_false_for_path_dep() {
        let spec = DependencySpec {
            path: Some("../lib".to_owned()),
            maven: None,
            version: None,
        };
        assert!(!spec.is_maven());
    }

    #[test]
    fn as_maven_coord_returns_pair_when_complete() {
        let spec = DependencySpec {
            path: None,
            maven: Some("org.example:lib".to_owned()),
            version: Some("1.0.0".to_owned()),
        };
        assert_eq!(spec.as_maven_coord(), Some(("org.example:lib", "1.0.0")));
    }

    #[test]
    fn as_maven_coord_none_when_partial() {
        let maven_only = DependencySpec {
            path: None,
            maven: Some("org.example:lib".to_owned()),
            version: None,
        };
        let version_only = DependencySpec {
            path: None,
            maven: None,
            version: Some("1.0.0".to_owned()),
        };
        let neither = DependencySpec {
            path: None,
            maven: None,
            version: None,
        };
        assert_eq!(maven_only.as_maven_coord(), None);
        assert_eq!(version_only.as_maven_coord(), None);
        assert_eq!(neither.as_maven_coord(), None);
    }
}
