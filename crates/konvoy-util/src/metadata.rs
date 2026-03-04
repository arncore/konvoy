//! Format-agnostic artifact metadata abstraction.
//!
//! Provides a common representation for artifact metadata regardless of
//! the source format (Gradle Module Metadata `.module` JSON, POM XML, or
//! future formats like uklib). The resolver talks to this abstraction,
//! never to format-specific parsers directly.

use crate::error::UtilError;
use crate::module_metadata::{fetch_module_metadata, parse_module_metadata};
use crate::pom::{fetch_pom, parse_pom, pom_to_metadata};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Format-agnostic artifact metadata — what the resolver needs from any
/// metadata source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactMetadata {
    /// Transitive compile-scope dependencies (base names, no target suffix).
    pub dependencies: Vec<MetadataDep>,
    /// All klib files published for this variant (main klib + any cinterop klibs).
    pub files: Vec<MetadataFile>,
}

/// A single transitive dependency extracted from metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataDep {
    /// Maven group identifier.
    pub group_id: String,
    /// Maven artifact identifier (base name, e.g. `"atomicfu"` not `"atomicfu-linuxx64"`).
    pub artifact_id: String,
    /// Exact version string.
    pub version: String,
}

/// A file published as part of a Maven artifact variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataFile {
    /// Human-readable filename, e.g. `"atomicfu-cinterop-interop.klib"`.
    pub name: String,
    /// Relative URL within the Maven directory (filename portion for download).
    pub url: String,
    /// SHA-256 hash of the file, if provided by the metadata source.
    pub sha256: Option<String>,
}

// ---------------------------------------------------------------------------
// Provider chain
// ---------------------------------------------------------------------------

/// Fetch artifact metadata using the best available source.
///
/// Tries Gradle Module Metadata first (`.module` JSON), falls back to POM XML
/// if the `.module` file does not exist (404).
///
/// The `maven_suffix` parameter (e.g. `"linuxx64"`) is needed for POM fallback
/// to strip target suffixes from dependency artifact IDs.
///
/// # Errors
///
/// Returns an error if both `.module` and POM fetching/parsing fail.
pub fn fetch_artifact_metadata(
    group_id: &str,
    artifact_id: &str,
    version: &str,
    maven_suffix: &str,
) -> Result<ArtifactMetadata, UtilError> {
    // Try Gradle Module Metadata first.
    match fetch_module_metadata(group_id, artifact_id, version) {
        Ok(Some(json)) => {
            return parse_module_metadata(&json);
        }
        Ok(None) => {
            // 404 — fall through to POM.
        }
        Err(_) => {
            // Network error fetching .module — fall through to POM.
        }
    }

    // Fall back to POM.
    let pom_xml = fetch_pom(group_id, artifact_id, version)?;
    let pom = parse_pom(&pom_xml, Some(group_id), Some(version))?;
    Ok(pom_to_metadata(&pom, maven_suffix))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn artifact_metadata_default_construction() {
        let metadata = ArtifactMetadata {
            dependencies: vec![MetadataDep {
                group_id: "org.example".to_owned(),
                artifact_id: "lib".to_owned(),
                version: "1.0".to_owned(),
            }],
            files: vec![MetadataFile {
                name: "lib.klib".to_owned(),
                url: "lib-1.0.klib".to_owned(),
                sha256: Some("abc123".to_owned()),
            }],
        };
        assert_eq!(metadata.dependencies.len(), 1);
        assert_eq!(metadata.files.len(), 1);
    }

    #[test]
    fn metadata_dep_fields() {
        let dep = MetadataDep {
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "atomicfu".to_owned(),
            version: "0.23.1".to_owned(),
        };
        assert_eq!(dep.group_id, "org.jetbrains.kotlinx");
        assert_eq!(dep.artifact_id, "atomicfu");
        assert_eq!(dep.version, "0.23.1");
    }

    #[test]
    fn metadata_file_without_sha256() {
        let file = MetadataFile {
            name: "lib.klib".to_owned(),
            url: "lib-1.0.klib".to_owned(),
            sha256: None,
        };
        assert!(file.sha256.is_none());
    }

    #[test]
    fn fetch_artifact_metadata_nonexistent_fails() {
        // Both .module and POM will fail for a non-existent artifact.
        let result = fetch_artifact_metadata(
            "com.nonexistent.fake",
            "no-such-artifact",
            "0.0.0",
            "linuxx64",
        );
        assert!(result.is_err());
    }

    #[test]
    fn artifact_metadata_empty_deps_and_files() {
        let metadata = ArtifactMetadata {
            dependencies: Vec::new(),
            files: Vec::new(),
        };
        assert!(metadata.dependencies.is_empty());
        assert!(metadata.files.is_empty());
    }

    #[test]
    fn artifact_metadata_equality() {
        let a = ArtifactMetadata {
            dependencies: vec![MetadataDep {
                group_id: "org.example".to_owned(),
                artifact_id: "lib".to_owned(),
                version: "1.0".to_owned(),
            }],
            files: vec![MetadataFile {
                name: "lib.klib".to_owned(),
                url: "lib-1.0.klib".to_owned(),
                sha256: Some("abc123".to_owned()),
            }],
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn artifact_metadata_inequality_different_deps() {
        let a = ArtifactMetadata {
            dependencies: vec![MetadataDep {
                group_id: "org.a".to_owned(),
                artifact_id: "lib".to_owned(),
                version: "1.0".to_owned(),
            }],
            files: Vec::new(),
        };
        let b = ArtifactMetadata {
            dependencies: vec![MetadataDep {
                group_id: "org.b".to_owned(),
                artifact_id: "lib".to_owned(),
                version: "1.0".to_owned(),
            }],
            files: Vec::new(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn artifact_metadata_with_multiple_files() {
        let metadata = ArtifactMetadata {
            dependencies: Vec::new(),
            files: vec![
                MetadataFile {
                    name: "lib.klib".to_owned(),
                    url: "lib-1.0.klib".to_owned(),
                    sha256: Some("abc".to_owned()),
                },
                MetadataFile {
                    name: "lib-cinterop-native.klib".to_owned(),
                    url: "lib-1.0-cinterop-native.klib".to_owned(),
                    sha256: Some("def".to_owned()),
                },
            ],
        };
        assert_eq!(metadata.files.len(), 2);
        assert!(metadata.files.iter().any(|f| f.name.contains("cinterop")));
    }

    #[test]
    fn metadata_dep_equality() {
        let a = MetadataDep {
            group_id: "org.example".to_owned(),
            artifact_id: "lib".to_owned(),
            version: "1.0".to_owned(),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn metadata_file_equality() {
        let a = MetadataFile {
            name: "lib.klib".to_owned(),
            url: "lib-1.0.klib".to_owned(),
            sha256: Some("abc".to_owned()),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn metadata_file_inequality_different_sha256() {
        let a = MetadataFile {
            name: "lib.klib".to_owned(),
            url: "lib-1.0.klib".to_owned(),
            sha256: Some("abc".to_owned()),
        };
        let b = MetadataFile {
            name: "lib.klib".to_owned(),
            url: "lib-1.0.klib".to_owned(),
            sha256: Some("def".to_owned()),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn metadata_file_sha256_none_vs_some_not_equal() {
        let a = MetadataFile {
            name: "lib.klib".to_owned(),
            url: "lib-1.0.klib".to_owned(),
            sha256: None,
        };
        let b = MetadataFile {
            name: "lib.klib".to_owned(),
            url: "lib-1.0.klib".to_owned(),
            sha256: Some("abc".to_owned()),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn artifact_metadata_inequality_different_files() {
        let a = ArtifactMetadata {
            dependencies: Vec::new(),
            files: vec![MetadataFile {
                name: "lib.klib".to_owned(),
                url: "lib-1.0.klib".to_owned(),
                sha256: Some("abc".to_owned()),
            }],
        };
        let b = ArtifactMetadata {
            dependencies: Vec::new(),
            files: Vec::new(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn metadata_dep_inequality() {
        let a = MetadataDep {
            group_id: "org.example".to_owned(),
            artifact_id: "lib".to_owned(),
            version: "1.0".to_owned(),
        };
        let b = MetadataDep {
            group_id: "org.example".to_owned(),
            artifact_id: "lib".to_owned(),
            version: "2.0".to_owned(),
        };
        assert_ne!(a, b);
    }
}
