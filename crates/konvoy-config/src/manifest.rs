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
}

/// Toolchain specification declaring the Kotlin/Native version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    /// Kotlin/Native version, e.g. "2.1.0".
    pub kotlin: String,
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
    // Future: git, tag, branch, rev fields for git dependencies.
}

fn default_entrypoint() -> String {
    "src/main.kt".to_owned()
}

/// Check whether a package name is valid: non-empty, only alphanumeric, hyphen, or underscore.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Check whether an entrypoint path ends with `.kt`.
fn is_valid_entrypoint(entrypoint: &str) -> bool {
    entrypoint.ends_with(".kt")
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
    // Validate dependencies.
    for (name, spec) in &manifest.dependencies {
        if !is_valid_name(name) {
            return Err(ManifestError::DependencyInvalidName {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        if name == &manifest.package.name {
            return Err(ManifestError::DependencySelfReference {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
        if spec.path.is_none() {
            return Err(ManifestError::DependencyNoSource {
                path: path.to_owned(),
                name: name.clone(),
            });
        }
    }
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
    #[error("dependency `{name}` has no source (set `path`) in {path}")]
    DependencyNoSource { path: String, name: String },
    #[error("dependency name `{name}` contains invalid characters in {path}")]
    DependencyInvalidName { path: String, name: String },
    #[error("dependency `{name}` references itself in {path}")]
    DependencySelfReference { path: String, name: String },
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
        let manifest = manifest.unwrap_or_else(|e| panic!("{e}"));
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
        let manifest = manifest.unwrap_or_else(|e| panic!("{e}"));
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
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        let serialized = original.to_toml().unwrap_or_else(|e| panic!("{e}"));
        let reparsed =
            Manifest::from_str(&serialized, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(original, reparsed);
    }

    #[test]
    fn valid_name_chars() {
        assert!(is_valid_name("hello"));
        assert!(is_valid_name("hello-world"));
        assert!(is_valid_name("hello_world"));
        assert!(is_valid_name("Hello123"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("hello world"));
        assert!(!is_valid_name("hello!"));
        assert!(!is_valid_name("hello.world"));
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
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
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
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
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
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
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
        let original = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        let serialized = original.to_toml().unwrap_or_else(|e| panic!("{e}"));
        let reparsed =
            Manifest::from_str(&serialized, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
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
        let manifest = Manifest::from_str(&toml, "konvoy.toml").unwrap_or_else(|e| panic!("{e}"));
        let serialized = manifest.to_toml().unwrap_or_else(|e| panic!("{e}"));
        assert!(
            !serialized.contains("[dependencies]"),
            "serialized was: {serialized}"
        );
    }

    proptest! {
        #[test]
        fn package_name_validation_never_panics(name in "\\PC*") {
            // Just call from_str - it should return Ok or Err, never panic
            let _ = Manifest::from_str(
                &format!(
                    "[package]\nname = \"{}\"\n[toolchain]\nkotlin = \"2.1.0\"",
                    name.replace('\\', "\\\\").replace('"', "\\\"")
                ),
                "test.toml",
            );
        }

        #[test]
        fn is_valid_name_never_panics(name in "\\PC*") {
            let _ = is_valid_name(&name);
        }

        #[test]
        fn is_valid_entrypoint_never_panics(ep in "\\PC*") {
            let _ = is_valid_entrypoint(&ep);
        }
    }
}
